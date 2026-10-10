//! What the index keeps between runs, per log file: where it stopped reading and the numbers it
//! found up to there. Paths, ids (as hashes), slugs, branch names, times, byte offsets and line
//! numbers only; never text from a prompt, a file, a command, a patch, a plan or a task.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Bumped whenever a cached struct or its meaning changes.
pub(crate) const VERSION: u32 = 2;
/// The [`crate::Store`] name.
pub(crate) const NAME: &str = "agents";
/// Bytes hashed at the start of a file to notice that it was replaced.
pub(crate) const HEAD_BYTES: u64 = 4096;

/// Every log file seen on the last run.
#[derive(Debug, Clone, Default, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub(crate) struct AgentCache {
    pub files: Vec<CachedFile>,
}

/// One log file as it was on the last run.
#[derive(Debug, Clone, Default, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub(crate) struct CachedFile {
    pub path: String,
    pub len: u64,
    pub mtime: i64,
    /// How many leading bytes `head` hashes, and their hash.
    pub head_len: u64,
    pub head: u64,
    /// The first `cwd` in the file, which decides whether it belongs to the project.
    pub first_cwd: Option<String>,
    /// Bytes read for the decision (only while no `cwd` was found).
    pub scanned_to: u64,
    /// When the first `cwd` is outside the roots: the first later `cwd` inside one, found by
    /// scanning the bytes for the roots' spellings.
    pub later_cwd: Option<String>,
    /// How far that scan went, and the spellings it looked for (a hash; a new set scans again).
    pub later_scanned_to: u64,
    pub later_key: u64,
    /// What it says, for files that belong to the project.
    pub facts: Option<FileFacts>,
}

/// What one log file says, up to its last complete line.
#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub(crate) struct FileFacts {
    /// Offset just past the last complete line read.
    pub parsed_to: u64,
    pub lines: u64,
    pub parsed_lines: u64,
    pub bad_lines: u64,
    pub unknown_types: Vec<TypeCount>,
    /// Earliest and latest line time, unix ms (`i64::MAX` / `i64::MIN` while none).
    pub started: i64,
    pub ended: i64,
    pub slug: Option<String>,
    pub branches: Vec<String>,
    pub first_cwd: Option<String>,
    /// Hashes of the lines' `uuid`s, to count lines copied by a resume or fork.
    pub uuids: Vec<u64>,
    pub uses: Vec<UseFact>,
    /// Bash calls whose result has not been read yet: it may carry a `bashEditDiff`.
    pub pending_bash: Vec<PendingBash>,
}

impl Default for FileFacts {
    fn default() -> Self {
        Self {
            parsed_to: 0,
            lines: 0,
            parsed_lines: 0,
            bad_lines: 0,
            unknown_types: Vec::new(),
            started: i64::MAX,
            ended: i64::MIN,
            slug: None,
            branches: Vec::new(),
            first_cwd: None,
            uuids: Vec::new(),
            uses: Vec::new(),
            pending_bash: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub(crate) struct TypeCount {
    pub name: String,
    pub count: u64,
}

/// What a [`UseFact`] is.
pub(crate) mod use_kind {
    pub const READ: u8 = 0;
    pub const EDIT: u8 = 1;
    pub const WRITE: u8 = 2;
    pub const BASH_EDIT: u8 = 3;
    pub const PLAN: u8 = 4;
    pub const TASK_CREATE: u8 = 5;
    pub const TASK_UPDATE: u8 = 6;
}

/// One tool call the index cares about, with where its line and its result's line are.
#[derive(Debug, Clone, Default, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub(crate) struct UseFact {
    /// Hash of the tool_use id (mixed with the file's place for a Bash edit of several files).
    pub id: u64,
    pub kind: u8,
    /// The canonical path a file tool touched, or the plan file named by the call.
    pub path: Option<String>,
    pub out_of_repo: bool,
    /// The canonical folder of an out-of-repo file.
    pub outside: Option<String>,
    pub offset: u64,
    pub len: u32,
    pub time: i64,
    /// Read `offset` / `limit` (0 when not given).
    pub read: Option<(u32, u32)>,
    pub result: Option<(u64, u32)>,
    pub error: bool,
    pub hunks: Vec<Hunk>,
    /// TaskCreate: the id its result gave; TaskUpdate: the id it names.
    pub task_id: Option<String>,
    /// TaskUpdate: the state it set (see `TaskState::code`), 255 for none.
    pub status: u8,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub(crate) struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub(crate) struct PendingBash {
    pub id: u64,
    pub offset: u64,
    pub len: u32,
    pub time: i64,
}

/// FNV-1a, 64 bits: stable across runs and builds, which the cache needs.
pub(crate) fn hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The cache key: what was indexed and for which roots.
pub(crate) fn key(claude_dir: &Path, roots: &[PathBuf], map_parent_cwds: bool) -> u64 {
    let mut text = String::new();
    text.push_str(&claude_dir.to_string_lossy());
    for root in roots {
        text.push('\0');
        text.push_str(&root.to_string_lossy());
    }
    text.push_str(if map_parent_cwds { "\0parents" } else { "\0" });
    hash(text.as_bytes())
}

/// Hash of the first `len` bytes of `path` (fewer if it is shorter now).
pub(crate) fn head_hash(path: &Path, len: u64) -> std::io::Result<u64> {
    let mut bytes = Vec::with_capacity(len as usize);
    std::fs::File::open(path)?.take(len).read_to_end(&mut bytes)?;
    if (bytes.len() as u64) < len {
        return Err(std::io::Error::other("shorter than before"));
    }
    Ok(hash(&bytes))
}

/// Modification time in nanoseconds, 0 when unknown.
pub(crate) fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}
