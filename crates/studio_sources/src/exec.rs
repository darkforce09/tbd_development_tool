//! The only programs Studio runs to read a project, and how it runs them.
//!
//! Every command comes from a closed list of read-only `git` and `cargo` subcommands. Each runs
//! with a timeout, never more than [`MAX_CHILDREN`] at a time, and is killed when its job is
//! cancelled (the project closes). Git never takes optional locks, so a running Studio never
//! blocks the user's own git commands; cargo never installs a toolchain.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Most child processes running at once, across all sources of one project.
pub const MAX_CHILDREN: usize = 4;
/// How long a command may run unless it says otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// Git subcommands that only read.
const GIT_READ_ONLY: &[&str] = &[
    "cat-file",
    "check-attr",
    "check-ignore",
    "diff",
    "for-each-ref",
    "log",
    "ls-files",
    "rev-list",
    "rev-parse",
    "show",
    "status",
    "version",
    "worktree",
];

/// A program Studio may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Program {
    Git,
    Cargo,
}

impl Program {
    fn binary(self) -> &'static str {
        match self {
            Program::Git => "git",
            Program::Cargo => "cargo",
        }
    }
}

/// A read-only command, built only through [`Command::git`] and [`Command::cargo`].
#[derive(Debug, Clone)]
pub struct Command {
    program: Program,
    args: Vec<OsString>,
    cwd: PathBuf,
    timeout: Duration,
    /// Written to the command's standard input, e.g. paths for `git check-attr --stdin`.
    stdin: Option<Vec<u8>>,
}

impl Command {
    /// `git <args>` in `cwd`. Refused unless the subcommand only reads; `worktree` only lists.
    pub fn git<I, S>(cwd: &Path, args: I) -> Result<Self, ExecError>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
        let sub = args.first().and_then(|a| a.to_str()).unwrap_or_default();
        let lists_worktrees = sub != "worktree" || args.get(1).and_then(|a| a.to_str()) == Some("list");
        if !GIT_READ_ONLY.contains(&sub) || !lists_worktrees {
            return Err(ExecError::NotAllowed(format!("git {sub}")));
        }
        // Never take optional locks (index refresh), never page, paths as raw bytes.
        let mut full: Vec<OsString> =
            ["--no-optional-locks", "--no-pager", "-c", "core.quotepath=off"].into_iter().map(OsString::from).collect();
        full.extend(args);
        Ok(Self { program: Program::Git, args: full, cwd: cwd.to_path_buf(), timeout: DEFAULT_TIMEOUT, stdin: None })
    }

    /// `cargo metadata --no-deps` for one manifest, offline. Nothing else cargo can do is allowed
    /// here.
    pub fn cargo_metadata(manifest: &Path) -> Self {
        let mut args: Vec<OsString> =
            ["metadata", "--format-version", "1", "--offline", "--no-deps", "--manifest-path"]
                .map(OsString::from)
                .into();
        args.push(manifest.as_os_str().to_owned());
        let cwd = manifest.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
        Self { program: Program::Cargo, args, cwd, timeout: DEFAULT_TIMEOUT, stdin: None }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Feeds `input` to the command's standard input.
    pub fn with_stdin(mut self, input: Vec<u8>) -> Self {
        self.stdin = Some(input);
        self
    }

    pub fn program(&self) -> Program {
        self.program
    }

    /// The command line, for logs and error messages.
    pub fn display(&self) -> String {
        let args: Vec<String> = self.args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        format!("{} {}", self.program.binary(), args.join(" "))
    }

    fn spawn(&self) -> std::io::Result<Child> {
        let mut cmd = std::process::Command::new(self.program.binary());
        cmd.args(&self.args)
            .current_dir(&self.cwd)
            .stdin(if self.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Plain, untranslated output that never waits for a password.
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("RUSTUP_AUTO_INSTALL", "0")
            .env("CARGO_TERM_COLOR", "never");
        cmd.spawn()
    }
}

/// What a finished command printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// Not on the read-only list.
    NotAllowed(String),
    /// The program is not installed.
    NotFound(String),
    TimedOut(Duration),
    Cancelled,
    /// It ran and exited with an error; holds the code and the first lines it printed to stderr.
    Failed {
        code: Option<i32>,
        stderr: String,
    },
    Io(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::NotAllowed(cmd) => write!(f, "{cmd} is not a read-only command"),
            ExecError::NotFound(program) => write!(f, "{program} is not installed"),
            ExecError::TimedOut(after) => write!(f, "took longer than {}s", after.as_secs()),
            ExecError::Cancelled => write!(f, "cancelled"),
            ExecError::Failed { code: Some(code), stderr } => write!(f, "exited with {code}: {stderr}"),
            ExecError::Failed { code: None, stderr } => write!(f, "was killed: {stderr}"),
            ExecError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ExecError {}

/// Set once to stop every job of a project; checked between steps and while waiting on children.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Runs commands for one project: at most [`MAX_CHILDREN`] at once, each killed on timeout or
/// when the project's [`CancelToken`] is set.
#[derive(Debug, Clone)]
pub struct Runner {
    slots: Arc<(Mutex<usize>, Condvar)>,
    cancel: CancelToken,
}

impl Runner {
    pub fn new(cancel: CancelToken) -> Self {
        Self { slots: Arc::new((Mutex::new(0), Condvar::new())), cancel }
    }

    pub fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }

    /// Runs `command` to completion. A non-zero exit is an error.
    pub fn run(&self, command: &Command) -> Result<Output, ExecError> {
        let _slot = self.acquire()?;
        let mut child = command.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ExecError::NotFound(command.program.binary().to_string()),
            _ => ExecError::Io(e.to_string()),
        })?;
        // Read both pipes on their own threads so a chatty command never blocks on a full pipe.
        let read_all = |pipe: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_end(&mut bytes);
                }
                bytes
            })
        };
        if let (Some(input), Some(mut pipe)) = (command.stdin.clone(), child.stdin.take()) {
            // Written on its own thread: a command that answers as it reads would block otherwise.
            std::thread::spawn(move || {
                use std::io::Write;
                let _ = pipe.write_all(&input);
            });
        }
        let stdout = read_all(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
        let stderr = read_all(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));

        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| ExecError::Io(e.to_string()))? {
                break status;
            }
            if self.cancel.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ExecError::Cancelled);
            }
            if started.elapsed() > command.timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ExecError::TimedOut(command.timeout));
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let output = Output { stdout: stdout.join().unwrap_or_default(), stderr: stderr.join().unwrap_or_default() };
        if status.success() {
            Ok(output)
        } else {
            let text = String::from_utf8_lossy(&output.stderr);
            let stderr = text.lines().take(3).collect::<Vec<_>>().join(" / ");
            Err(ExecError::Failed { code: status.code(), stderr })
        }
    }

    /// Waits for a free child slot, giving up when cancelled.
    fn acquire(&self) -> Result<SlotGuard<'_>, ExecError> {
        let (lock, freed) = &*self.slots;
        let mut running = lock.lock().unwrap_or_else(|e| e.into_inner());
        while *running >= MAX_CHILDREN {
            if self.cancel.is_cancelled() {
                return Err(ExecError::Cancelled);
            }
            running = freed.wait_timeout(running, Duration::from_millis(50)).unwrap_or_else(|e| e.into_inner()).0;
        }
        *running += 1;
        Ok(SlotGuard(&self.slots))
    }
}

struct SlotGuard<'a>(&'a (Mutex<usize>, Condvar));

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        let (lock, freed) = self.0;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
        freed.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_read_only_git_commands_can_be_built() {
        let here = Path::new(".");
        assert!(Command::git(here, ["status", "--porcelain=v2"]).is_ok());
        assert!(Command::git(here, ["worktree", "list", "--porcelain"]).is_ok());
        for refused in [&["commit", "-m", "x"][..], &["worktree", "add", "x"], &["push"], &["config", "x", "y"], &[]] {
            assert!(
                matches!(Command::git(here, refused.iter().copied()), Err(ExecError::NotAllowed(_))),
                "{refused:?}"
            );
        }
        let cmd = Command::git(here, ["log"]).unwrap();
        assert!(cmd.display().starts_with("git --no-optional-locks --no-pager"), "{}", cmd.display());
        let cargo = Command::cargo_metadata(Path::new("/p/Cargo.toml")).display();
        assert!(
            cargo.contains("metadata --format-version 1 --offline --no-deps --manifest-path /p/Cargo.toml"),
            "{cargo}"
        );
    }

    #[test]
    fn git_runs_and_reports_failures() {
        let runner = Runner::new(CancelToken::default());
        let version = match runner.run(&Command::git(Path::new("."), ["version"]).unwrap()) {
            Ok(out) => out,
            Err(ExecError::NotFound(_)) => return, // no git here
            Err(e) => panic!("{e}"),
        };
        assert!(String::from_utf8_lossy(&version.stdout).starts_with("git version"));

        let outside = tempfile::tempdir().unwrap();
        let err = runner.run(&Command::git(outside.path(), ["rev-parse", "--verify", "no-such-ref"]).unwrap());
        assert!(matches!(err, Err(ExecError::Failed { code: Some(_), .. })), "{err:?}");
    }

    #[test]
    fn a_cancelled_runner_starts_nothing() {
        let cancel = CancelToken::default();
        cancel.cancel();
        let runner = Runner::new(cancel);
        // Every slot taken, so the run has to wait for one and sees the cancel instead.
        *runner.slots.0.lock().unwrap() = MAX_CHILDREN;
        assert!(matches!(runner.run(&Command::git(Path::new("."), ["version"]).unwrap()), Err(ExecError::Cancelled)));
    }

    #[test]
    fn a_command_that_runs_too_long_is_killed() {
        let runner = Runner::new(CancelToken::default());
        if runner.run(&Command::git(Path::new("."), ["version"]).unwrap()).is_err() {
            return; // no git here
        }
        // Built by hand: the public constructors only allow read-only commands.
        let slow = Command {
            program: Program::Git,
            args: ["-c", "alias.wait=!sleep 5", "wait"].map(OsString::from).into(),
            cwd: PathBuf::from("."),
            timeout: Duration::from_millis(200),
            stdin: None,
        };
        let started = Instant::now();
        assert_eq!(runner.run(&slow), Err(ExecError::TimedOut(Duration::from_millis(200))));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
