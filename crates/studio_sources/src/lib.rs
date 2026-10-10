//! Everything Studio reads about a project besides its source code: packages, settings, tools,
//! git, coding agents' sessions, tickets and the pipeline. Each source reads files or runs a
//! read-only command off the UI thread and reports through a [`SourceHub`]. Nothing here changes
//! the project, and every result says where it came from (docs/ROADMAP.md S1, S2).

pub mod agents;
pub mod disk;
pub mod exec;
pub mod git;
pub mod hub;
pub mod packages;
pub mod settings;
pub mod store;
pub mod tickets;
pub mod tools;

pub use agents::{
    agents_job, default_claude_dir, index_sessions, index_sessions_with, AgentIndex, FolderStat, HunkRange,
    IndexOptions, IndexStats, LogFile, PlanRef, Session, TaskRef, TaskState, Touch, TouchKind,
};
pub use disk::{disk_job, du_bytes, read_git_disk, GitDisk, IgnoreRule, IgnoredEntry, StatusCounts};
pub use exec::{CancelToken, Command, ExecError, Output, Program, Runner};
pub use git::{
    commit_files, commit_files_cached, commit_files_job, history_job, parse_numstat_z, part_commits_job, read_history,
    read_part_commits, Branch, Commit, FileChange, GitHistory, PartCommit, PartCommits, Worktree,
};
pub use hub::{JobContext, JobError, SourceEvent, SourceHub, SourceKind, SourceState};
pub use packages::{packages_job, read_packages, Package, PackageSource, Packages, Target, TargetKind};
pub use settings::{
    read_settings, settings_job, Fact, FactSheet, IgnoreFile, IgnoreGroup, IgnoreRuleLine, SchemaShape, Settings,
};
pub use store::Store;
pub use tickets::{id_order, next_tickets, read_tickets, ticket_ids_in, tickets_job, Shipped, Ticket, TicketIndex};
pub use tools::{discover_tools, tools_job, Tool, ToolKind, Tools};
