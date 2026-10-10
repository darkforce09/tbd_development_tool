//! One hub per open project. It runs each source's job on its own thread and hands the results to
//! the UI as [`SourceEvent`]s over one channel; the UI drains them once per frame, which costs no
//! more than swapping in what arrived. Dropping the hub cancels every job and kills the commands
//! they are running.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::exec::{CancelToken, ExecError, Runner};

/// Everything Studio reads about a project besides its source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceKind {
    /// Files and folders as on disk, sizes, ignore and attribute rules.
    Disk,
    /// Packages and how they depend on each other.
    Packages,
    /// Manifests, toolchain and tool settings.
    Settings,
    /// Commands the project defines: binaries, scripts, aliases, CI jobs.
    Tools,
    /// Branches, worktrees, commits and changes.
    Git,
    /// Coding agents' session logs.
    Agents,
    /// Tickets kept in the repository.
    Tickets,
    /// Routes, contracts and the flows through them.
    Pipeline,
}

impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Disk => "disk",
            SourceKind::Packages => "packages",
            SourceKind::Settings => "settings",
            SourceKind::Tools => "tools",
            SourceKind::Git => "git",
            SourceKind::Agents => "agent sessions",
            SourceKind::Tickets => "tickets",
            SourceKind::Pipeline => "pipeline",
        }
    }
}

/// Where a source is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceState {
    Running,
    Ready {
        elapsed: Duration,
    },
    /// The project has nothing for this source, e.g. "No git repository here". Shown as the
    /// district's empty state, never as an error.
    Unavailable(String),
    Failed(String),
}

/// What a source sends the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceEvent {
    Status {
        source: SourceKind,
        state: SourceState,
    },
    Packages(Arc<crate::packages::Packages>),
    Tools(Arc<crate::tools::Tools>),
    GitDisk(Arc<crate::disk::GitDisk>),
    GitHistory(Arc<crate::git::GitHistory>),
    Settings(Arc<crate::settings::Settings>),
    /// Bytes a top-level entry (relative to the root) takes on disk.
    FolderSize(std::path::PathBuf, u64),
    /// The project's tickets (`.ai/tickets`).
    Tickets(Arc<crate::tickets::TicketIndex>),
    /// The last commit on HEAD that touched each folder.
    PartCommits(Arc<crate::git::PartCommits>),
    /// The files one commit changed, by its full id; read when asked for.
    CommitFiles(String, Arc<Vec<crate::git::FileChange>>),
    /// Coding agents' sessions that worked in the project.
    Agents(Arc<crate::agents::AgentIndex>),
    /// Routes, senders, handlers, contracts and the flows through them.
    Pipeline(Arc<crate::pipeline::Pipeline>),
}

/// Why a job stopped without a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobError {
    /// Nothing to read here; the reason is shown as the empty state.
    Unavailable(String),
    Failed(String),
    /// The project closed.
    Cancelled,
}

impl From<ExecError> for JobError {
    fn from(e: ExecError) -> Self {
        match e {
            ExecError::Cancelled => JobError::Cancelled,
            ExecError::NotFound(program) => JobError::Unavailable(format!("{program} is not installed")),
            other => JobError::Failed(other.to_string()),
        }
    }
}

type Repaint = Arc<dyn Fn() + Send + Sync>;

/// What a running job can use: the project root, the command runner, and the way back to the UI.
pub struct JobContext {
    pub root: PathBuf,
    pub runner: Runner,
    tx: Sender<SourceEvent>,
    repaint: Repaint,
}

impl JobContext {
    /// Sends an event to the UI and asks for a repaint. Returns false once the hub is gone.
    pub fn send(&self, event: SourceEvent) -> bool {
        let sent = self.tx.send(event).is_ok();
        if sent {
            (self.repaint)();
        }
        sent
    }

    pub fn is_cancelled(&self) -> bool {
        self.runner.cancel_token().is_cancelled()
    }
}

/// The sources of one open project.
pub struct SourceHub {
    root: PathBuf,
    tx: Sender<SourceEvent>,
    rx: Receiver<SourceEvent>,
    runner: Runner,
    repaint: Repaint,
}

impl SourceHub {
    /// A hub for the project at `root`; `repaint` wakes the UI when an event arrives.
    pub fn new(root: impl Into<PathBuf>, repaint: impl Fn() + Send + Sync + 'static) -> Self {
        let (tx, rx) = channel();
        Self { root: root.into(), tx, rx, runner: Runner::new(CancelToken::default()), repaint: Arc::new(repaint) }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Runs `job` for `source` on its own thread, reporting Running, then Ready, Unavailable or
    /// Failed.
    pub fn spawn<F>(&self, source: SourceKind, job: F)
    where
        F: FnOnce(&JobContext) -> Result<(), JobError> + Send + 'static,
    {
        let ctx = JobContext {
            root: self.root.clone(),
            runner: self.runner.clone(),
            tx: self.tx.clone(),
            repaint: self.repaint.clone(),
        };
        let spawned = std::thread::Builder::new().name(format!("studio-{}", source.label())).spawn(move || {
            ctx.send(SourceEvent::Status { source, state: SourceState::Running });
            let started = Instant::now();
            let state = match job(&ctx) {
                Ok(()) => SourceState::Ready { elapsed: started.elapsed() },
                Err(JobError::Unavailable(why)) => SourceState::Unavailable(why),
                Err(JobError::Failed(why)) => SourceState::Failed(why),
                Err(JobError::Cancelled) => return,
            };
            if !ctx.is_cancelled() {
                ctx.send(SourceEvent::Status { source, state });
            }
        });
        if let Err(e) = spawned {
            let state = SourceState::Failed(format!("could not start: {e}"));
            let _ = self.tx.send(SourceEvent::Status { source, state });
        }
    }

    /// Everything that arrived since the last call. Never blocks.
    pub fn drain(&self) -> Vec<SourceEvent> {
        self.rx.try_iter().collect()
    }
}

impl Drop for SourceHub {
    fn drop(&mut self) {
        // Jobs see this between steps; running commands are killed. Nothing waits for them here,
        // so closing a project never stalls the UI.
        self.runner.cancel_token().cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn wait_for(hub: &SourceHub, count: usize) -> Vec<SourceEvent> {
        let mut events = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while events.len() < count && Instant::now() < deadline {
            events.extend(hub.drain());
            std::thread::sleep(Duration::from_millis(2));
        }
        events
    }

    #[test]
    fn a_job_reports_running_then_its_outcome_and_wakes_the_ui() {
        let repaints = Arc::new(AtomicUsize::new(0));
        let counter = repaints.clone();
        let hub = SourceHub::new("/project", move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        hub.spawn(SourceKind::Git, |_| Err(JobError::Unavailable("No git repository here".into())));
        let events = wait_for(&hub, 2);
        assert_eq!(events[0], SourceEvent::Status { source: SourceKind::Git, state: SourceState::Running });
        assert_eq!(
            events[1],
            SourceEvent::Status {
                source: SourceKind::Git,
                state: SourceState::Unavailable("No git repository here".into())
            }
        );
        assert_eq!(repaints.load(Ordering::Relaxed), 2);

        hub.spawn(SourceKind::Tools, |ctx| {
            assert_eq!(ctx.root, Path::new("/project"));
            Ok(())
        });
        let events = wait_for(&hub, 2);
        assert!(matches!(events[1], SourceEvent::Status { state: SourceState::Ready { .. }, .. }));
    }

    #[test]
    fn dropping_the_hub_cancels_its_jobs() {
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        let hub = SourceHub::new("/project", || {});
        hub.spawn(SourceKind::Disk, move |ctx| {
            while !ctx.is_cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            flag.store(true, Ordering::Relaxed);
            Err(JobError::Cancelled)
        });
        assert_eq!(wait_for(&hub, 1).len(), 1, "the job started");
        drop(hub);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !stopped.load(Ordering::Relaxed) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(stopped.load(Ordering::Relaxed));
    }
}
