//! Coding agents' session logs (`~/.claude/projects`): which sessions worked in this project,
//! the files they read and changed, their plans and their tasks. Everything kept is a path, an
//! id, a slug, a branch name, a time or a byte offset into a log; text (prompts, file contents,
//! commands, patches, plans, task subjects) stays in the logs and is read at show time only.
//! Facts from session logs are Observed: they say what an agent did, not what the code is.

mod cache;
mod discover;
mod index;
mod parse;
pub mod show;

pub use show::{match_touch, plan_text, task_steps, touch_needle, Needle, PlanText, TaskStep, TouchMatch};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::exec::CancelToken;
use crate::hub::{JobContext, JobError, SourceEvent};
use crate::store::Store;

/// The empty state when nothing maps to the project.
pub const NO_SESSIONS: &str = "No agent sessions for this project";

/// Every session that worked in the project's roots, with what it touched.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentIndex {
    /// The repository root first, then linked worktrees (canonical).
    pub roots: Vec<PathBuf>,
    /// The log files that belong to the project, oldest first.
    pub logs: Vec<LogFile>,
    /// Sessions, oldest first.
    pub sessions: Vec<Session>,
    /// Files read and changed, by session, then time.
    pub touches: Vec<Touch>,
    /// Every folder under `projects`, whether or not it belongs to the project.
    pub folders: Vec<FolderStat>,
    pub stats: IndexStats,
}

/// One session log file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFile {
    pub path: PathBuf,
    /// A subagent's log (`<session-id>/subagents/agent-*.jsonl`).
    pub subagent: bool,
    /// Index into [`AgentIndex::sessions`].
    pub session: Option<usize>,
}

/// One session: its main log and its subagents' logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    /// The session's generated name, also its plan file's name; shown with the start time.
    pub slug: Option<String>,
    /// Index into [`AgentIndex::roots`]: the longest root its first `cwd` inside a root is in.
    pub root: usize,
    pub branches: Vec<String>,
    /// First and last line time, unix ms.
    pub started: i64,
    pub ended: i64,
    /// Indexes into [`AgentIndex::logs`].
    pub logs: Vec<usize>,
    pub plan: Option<PlanRef>,
    /// TaskCreate calls in order, with their TaskUpdate states.
    pub tasks: Vec<TaskRef>,
    pub touch_count: usize,
}

/// Where a session's plan is: the plan file and the ExitPlanMode line that showed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRef {
    /// `planFilePath`, else `~/.claude/plans/<slug>.md` if it existed when indexed. May have
    /// been deleted since: checked at show time.
    pub path: Option<PathBuf>,
    /// The ExitPlanMode line (the latest one); `len == 0` when the plan was found by its file
    /// only and no line shows it.
    pub log: usize,
    pub offset: u64,
    pub len: u32,
    pub time: i64,
}

/// One task a session created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRef {
    pub id: String,
    /// The TaskCreate line.
    pub log: usize,
    pub offset: u64,
    pub len: u32,
    pub created: i64,
    /// States TaskUpdate set, oldest first (pending until the first).
    pub states: Vec<(i64, TaskState)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskState {
    Pending,
    InProgress,
    Completed,
    Deleted,
    Other,
}

impl TaskState {
    pub(crate) fn from_status(status: &str) -> Self {
        match status {
            "pending" => TaskState::Pending,
            "in_progress" => TaskState::InProgress,
            "completed" => TaskState::Completed,
            "deleted" => TaskState::Deleted,
            _ => TaskState::Other,
        }
    }

    pub(crate) fn code(self) -> u8 {
        self as u8
    }

    pub(crate) fn from_code(code: u8) -> Option<Self> {
        [TaskState::Pending, TaskState::InProgress, TaskState::Completed, TaskState::Deleted, TaskState::Other]
            .get(code as usize)
            .copied()
    }

    pub fn label(self) -> &'static str {
        match self {
            TaskState::Pending => "pending",
            TaskState::InProgress => "in progress",
            TaskState::Completed => "completed",
            TaskState::Deleted => "deleted",
            TaskState::Other => "other",
        }
    }
}

/// A file a session read or changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Touch {
    /// Index into [`AgentIndex::sessions`].
    pub session: usize,
    /// Canonical, absolute, inside one of the roots.
    pub path: PathBuf,
    pub kind: TouchKind,
    /// Unix ms of the tool call.
    pub time: i64,
    /// The tool call's line: log index, byte offset, length.
    pub log: usize,
    pub offset: u64,
    pub len: u32,
    /// The tool result's line, if it was written.
    pub result: Option<(usize, u64, u32)>,
    /// Read `offset` / `limit` (0 when not given).
    pub read: Option<(u32, u32)>,
    /// Line numbers of the change, from `structuredPatch` or `bashEditDiff` (main logs; empty
    /// when the log has none, then ranges are found at show time).
    pub hunks: Vec<HunkRange>,
    pub subagent: bool,
    /// Index into the session's tasks: the one in progress when the call was made.
    pub task: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkRange {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TouchKind {
    Read,
    Edit,
    Write,
    /// A file a Bash command changed, as the log's `bashEditDiff` records it. The command
    /// itself is never shown.
    BashEdit,
}

impl TouchKind {
    pub fn label(self) -> &'static str {
        match self {
            TouchKind::Read => "read",
            TouchKind::Edit => "edit",
            TouchKind::Write => "write",
            TouchKind::BashEdit => "bash edit",
        }
    }
}

/// One folder under `projects`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderStat {
    pub folder: PathBuf,
    /// Some session in it belongs to the project.
    pub mapped: bool,
    /// Log files in it (main and subagent).
    pub logs: usize,
    pub bytes: u64,
}

/// How the index was made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexStats {
    /// Lines and bytes in the project's logs.
    pub lines: u64,
    pub bytes: u64,
    /// Lines read with the JSON parser (tool calls and their results); the rest are skimmed.
    pub parsed_lines: u64,
    /// Line types the index does not read, with their counts.
    pub unknown_types: BTreeMap<String, u64>,
    pub out_of_repo_touches: u64,
    /// Out-of-repo touches whose folder no longer exists (e.g. removed worktrees), and those
    /// whose folder is still on disk; a relative path on a line without a `cwd` is in neither.
    pub out_of_repo_gone: u64,
    pub out_of_repo_existing: u64,
    /// The 5 folders with the most out-of-repo touches, most first (then by path), each cut
    /// to its first component below the deepest folder holding the roots' parents.
    pub out_of_repo_top: Vec<(String, u64)>,
    /// Lines whose `uuid` an earlier line already had (resume and fork copies).
    pub duplicate_lines: u64,
    pub bad_lines: u64,
    /// Files read only past where the cache stopped.
    pub resumed_files: usize,
    /// Cached files read again from the start (shrunk or replaced).
    pub reparsed_files: usize,
    /// Files read from the start because the cache did not have them.
    pub new_files: usize,
    /// Main and subagent logs, and their bytes.
    pub main_logs: usize,
    pub subagent_logs: usize,
    pub main_bytes: u64,
    pub subagent_bytes: u64,
    /// Every log file under `projects`.
    pub scanned_logs: usize,
    /// Logs and sessions in the project only because a line after the first cwd is inside a
    /// root, and the bytes scanned this run to find such lines.
    pub later_cwd_logs: usize,
    pub later_cwd_sessions: usize,
    pub later_cwd_bytes: u64,
    pub elapsed_ms: u64,
}

/// How sessions map to the project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexOptions {
    /// Also map sessions whose `cwd` is a parent folder of a root (off: such a session may
    /// have worked anywhere below it).
    pub map_parent_cwds: bool,
}

/// `$HOME/.claude`.
pub fn default_claude_dir() -> Option<PathBuf> {
    std::env::home_dir().filter(|h| !h.as_os_str().is_empty()).map(|h| h.join(".claude"))
}

/// Indexes every session under `claude_dir/projects` that worked in `roots` (the repository
/// root first, then linked worktrees). With a `store`, files already read are resumed from
/// where they stopped.
pub fn index_sessions(
    claude_dir: &Path,
    roots: &[PathBuf],
    store: Option<&Store>,
    cancel: Option<&CancelToken>,
) -> Result<AgentIndex, JobError> {
    index_sessions_with(claude_dir, roots, store, cancel, &IndexOptions::default())
}

/// [`index_sessions`] with options.
pub fn index_sessions_with(
    claude_dir: &Path,
    roots: &[PathBuf],
    store: Option<&Store>,
    cancel: Option<&CancelToken>,
    options: &IndexOptions,
) -> Result<AgentIndex, JobError> {
    index::run(claude_dir, roots, store, cancel, options)
}

/// The job for a [`crate::SourceHub`]: `roots` as for [`index_sessions`] (the hub's root when
/// empty), cached in the project's [`Store`].
pub fn agents_job(roots: Vec<PathBuf>) -> impl FnOnce(&JobContext) -> Result<(), JobError> + Send + 'static {
    move |ctx| {
        let claude_dir = default_claude_dir().ok_or_else(|| JobError::Unavailable(NO_SESSIONS.to_string()))?;
        let roots = if roots.is_empty() { vec![ctx.root.clone()] } else { roots };
        let store = Store::for_project(&ctx.root);
        let index = index_sessions(&claude_dir, &roots, Some(&store), Some(ctx.runner.cancel_token()))?;
        ctx.send(SourceEvent::Agents(Arc::new(index)));
        Ok(())
    }
}
