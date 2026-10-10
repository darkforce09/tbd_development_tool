//! The disk as git and the file system see it: which files git tracks, which entries it ignores
//! and by which rule, which files live in Git LFS, what changed, and how much space each
//! top-level entry takes, measured the way `du` measures it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rayon::prelude::*;
use studio_parser::classify::{classify, ClassInputs, FileClass, RULES};

use crate::exec::{Command, Runner};
use crate::hub::{JobContext, JobError, SourceEvent, SourceKind, SourceState};

/// The `.gitignore` line that ignores an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRule {
    pub source: PathBuf,
    pub line: usize,
    pub pattern: String,
}

/// Something git ignores: a folder (ending in `/`) or a file, relative to the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredEntry {
    pub path: String,
    pub rule: Option<IgnoreRule>,
}

/// Changes in the working tree, as `git status` counts them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub modified: usize,
    pub added: usize,
    pub deleted: usize,
    pub renamed: usize,
    pub untracked: usize,
    pub conflicted: usize,
}

impl StatusCounts {
    pub fn total(&self) -> usize {
        self.modified + self.added + self.deleted + self.renamed + self.untracked + self.conflicted
    }
}

/// What git says about the files on disk. Paths are relative to the root, `/` separated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitDisk {
    pub tracked: usize,
    /// Tracked files whose `filter` attribute is `lfs` (always `lfs_paths.len()`).
    pub lfs: usize,
    /// Tracked files whose `filter` attribute is `lfs`.
    pub lfs_paths: BTreeSet<String>,
    /// Tracked files marked `linguist-generated` in `.gitattributes`.
    pub generated_paths: BTreeSet<String>,
    /// Tracked files marked `linguist-vendored` in `.gitattributes`.
    pub vendored_paths: BTreeSet<String>,
    /// Tracked files marked `linguist-documentation` in `.gitattributes`.
    pub documentation_paths: BTreeSet<String>,
    /// Tracked files by class, in the table's class order; classes without files are left out.
    pub classes: Vec<ClassTotal>,
    pub ignored: Vec<IgnoredEntry>,
    pub changes: StatusCounts,
}

/// Tracked files by the attributes `check-attr` reports for them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttrPaths {
    pub lfs: BTreeSet<String>,
    pub generated: BTreeSet<String>,
    pub vendored: BTreeSet<String>,
    pub documentation: BTreeSet<String>,
}

/// Tracked files of one class: how many, what they take, and the rules that decided them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassTotal {
    pub class: FileClass,
    pub files: usize,
    /// Their lengths in the working tree, summed (a file missing there counts 0).
    pub bytes: u64,
    /// Each deciding rule (`Rule::why`) and its files, most files first, ties by name.
    pub rules: Vec<(String, usize)>,
}

/// The attributes asked of `git check-attr`, in one call.
const ATTRS: [&str; 4] = ["filter", "linguist-generated", "linguist-vendored", "linguist-documentation"];

/// Sorts `git check-attr -z` output (`path NUL attribute NUL value NUL`, repeated) into
/// [`AttrPaths`]. A linguist attribute counts when set (`set` or `true`).
pub fn parse_check_attr(bytes: &[u8]) -> AttrPaths {
    let mut out = AttrPaths::default();
    let fields: Vec<&[u8]> = bytes.split(|&b| b == 0).collect();
    for triple in fields.chunks_exact(3) {
        let [path, attr, value] = triple else { continue };
        let set = match *attr {
            b"filter" => (*value == b"lfs").then_some(&mut out.lfs),
            b"linguist-generated" => matches!(*value, b"set" | b"true").then_some(&mut out.generated),
            b"linguist-vendored" => matches!(*value, b"set" | b"true").then_some(&mut out.vendored),
            b"linguist-documentation" => matches!(*value, b"set" | b"true").then_some(&mut out.documentation),
            _ => None,
        };
        if let Some(set) = set.filter(|_| !path.is_empty()) {
            set.insert(String::from_utf8_lossy(path).into_owned());
        }
    }
    out
}

fn nul_fields(bytes: &[u8]) -> Vec<String> {
    bytes.split(|&b| b == 0).filter(|f| !f.is_empty()).map(|f| String::from_utf8_lossy(f).into_owned()).collect()
}

/// Tracked files by class, from the classification table with git's inputs (`inputs.files` holds
/// every tracked path), in the table's class order; empty classes are left out. `size` gives a
/// file's length. Classifies in parallel; the totals do not depend on the order.
pub fn class_totals(tracked: &[String], inputs: &ClassInputs, size: impl Fn(&str) -> u64 + Sync) -> Vec<ClassTotal> {
    let decided: Vec<(usize, u64)> = tracked
        .par_iter()
        .map(|rel| {
            let (_, rule) = classify(rel, inputs);
            (RULES.iter().position(|r| std::ptr::eq(r, rule)).unwrap_or(RULES.len() - 1), size(rel))
        })
        .collect();
    let mut by_rule = vec![(0usize, 0u64); RULES.len()];
    for (row, bytes) in decided {
        by_rule[row].0 += 1;
        by_rule[row].1 += bytes;
    }
    FileClass::ALL
        .iter()
        .filter_map(|&class| {
            let (mut files, mut bytes) = (0, 0);
            // Rows that read the same (the three file-name rows of config) count as one.
            let mut why: BTreeMap<String, usize> = BTreeMap::new();
            for (rule, (n, size)) in RULES.iter().zip(&by_rule).filter(|(r, (n, _))| r.class == class && *n > 0) {
                *why.entry(rule.why()).or_default() += n;
                (files, bytes) = (files + n, bytes + size);
            }
            let mut rules: Vec<(String, usize)> = why.into_iter().collect();
            rules.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            (files > 0).then_some(ClassTotal { class, files, bytes, rules })
        })
        .collect()
}

/// Reads tracked files, LFS, generated, vendored and documentation files, ignored entries with
/// their rules, tracked files by class, and changes.
pub fn read_git_disk(runner: &Runner, root: &Path) -> Result<GitDisk, JobError> {
    let git = |args: &[&str]| -> Result<Vec<u8>, JobError> {
        Ok(runner.run(&Command::git(root, args.iter().copied())?)?.stdout)
    };
    if git(&["rev-parse", "--is-inside-work-tree"]).is_err() {
        return Err(JobError::Unavailable("No git repository here".to_string()));
    }
    let listed = git(&["ls-files", "-z"])?;
    let tracked_paths = nul_fields(&listed);

    let args = ["check-attr", "--stdin", "-z"].into_iter().chain(ATTRS);
    let attrs = parse_check_attr(&runner.run(&Command::git(root, args)?.with_stdin(listed))?.stdout);

    let ignored_paths =
        nul_fields(&git(&["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z"])?);
    let mut ignored: Vec<IgnoredEntry> =
        ignored_paths.iter().map(|p| IgnoredEntry { path: p.clone(), rule: None }).collect();
    if !ignored_paths.is_empty() {
        // `-v` names the file, line and pattern; `--no-index` keeps it from consulting the index.
        let input: Vec<u8> = ignored_paths.iter().flat_map(|p| p.bytes().chain([0])).collect();
        let command = Command::git(root, ["check-ignore", "-v", "-z", "--stdin", "--no-index"])?.with_stdin(input);
        let out = runner.run(&command);
        // check-ignore exits 1 when a path is not ignored; it still prints the ones that are.
        let stdout = match out {
            Ok(o) => o.stdout,
            Err(crate::exec::ExecError::Failed { .. }) => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let at: HashMap<String, usize> = ignored_paths.into_iter().enumerate().map(|(i, p)| (p, i)).collect();
        for chunk in nul_fields(&stdout).chunks(4) {
            let [source, line, pattern, path] = chunk else { continue };
            if let Some(entry) = at.get(path).and_then(|&i| ignored.get_mut(i)) {
                entry.rule = Some(IgnoreRule {
                    source: root.join(source),
                    line: line.parse().unwrap_or(0),
                    pattern: pattern.clone(),
                });
            }
        }
    }

    let status = git(&["status", "--porcelain=v2", "-z", "--untracked-files=normal"])?;
    let changes = parse_status_counts(&status);
    let inputs = ClassInputs {
        files: tracked_paths.iter().cloned().collect(),
        ignored: ignored.iter().map(|e| e.path.clone()).collect(),
        generated: attrs.generated,
        vendored: attrs.vendored,
        documentation: attrs.documentation,
    };
    let size = |rel: &str| std::fs::symlink_metadata(root.join(rel)).map(|m| m.len()).unwrap_or(0);
    let classes = class_totals(&tracked_paths, &inputs, size);
    Ok(GitDisk {
        tracked: tracked_paths.len(),
        lfs: attrs.lfs.len(),
        lfs_paths: attrs.lfs,
        generated_paths: inputs.generated,
        vendored_paths: inputs.vendored,
        documentation_paths: inputs.documentation,
        classes,
        ignored,
        changes,
    })
}

/// Counts `git status --porcelain=v2 -z` entries.
pub fn parse_status_counts(bytes: &[u8]) -> StatusCounts {
    let mut counts = StatusCounts::default();
    let mut fields = bytes.split(|&b| b == 0).filter(|f| !f.is_empty());
    while let Some(field) = fields.next() {
        let text = String::from_utf8_lossy(field);
        let mut parts = text.splitn(3, ' ');
        let (kind, xy) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let (x, y) = (xy.chars().next().unwrap_or('.'), xy.chars().nth(1).unwrap_or('.'));
        match kind {
            "1" => {
                if x == 'A' {
                    counts.added += 1;
                } else if x == 'D' || y == 'D' {
                    counts.deleted += 1;
                } else {
                    counts.modified += 1;
                }
            }
            "2" => {
                counts.renamed += 1;
                fields.next(); // the path it was renamed from
            }
            "u" => counts.conflicted += 1,
            "?" => counts.untracked += 1,
            _ => {}
        }
    }
    counts
}

/// Bytes `path` takes on disk, as `du -s --block-size=1` counts them: allocated blocks, each hard
/// link's file once, never crossing into another file system. Elsewhere than Unix, file lengths.
pub fn du_bytes(path: &Path) -> u64 {
    let seen = Mutex::new(HashSet::new());
    match std::fs::symlink_metadata(path) {
        Ok(meta) => du(path, &meta, device(&meta), &seen),
        Err(_) => 0,
    }
}

#[cfg(unix)]
fn device(meta: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::dev(meta)
}

#[cfg(not(unix))]
fn device(_: &std::fs::Metadata) -> u64 {
    0
}

/// What one entry takes, counting a file with several hard links only the first time.
fn own_bytes(meta: &std::fs::Metadata, seen: &Mutex<HashSet<(u64, u64)>>) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !meta.is_dir()
            && meta.nlink() > 1
            && !seen.lock().unwrap_or_else(|e| e.into_inner()).insert((meta.dev(), meta.ino()))
        {
            return 0;
        }
        meta.blocks() * 512
    }
    #[cfg(not(unix))]
    {
        let _ = seen;
        meta.len()
    }
}

fn du(path: &Path, meta: &std::fs::Metadata, dev: u64, seen: &Mutex<HashSet<(u64, u64)>>) -> u64 {
    if device(meta) != dev {
        return 0;
    }
    let own = own_bytes(meta, seen);
    if !meta.is_dir() {
        return own;
    }
    let Ok(entries) = std::fs::read_dir(path) else { return own };
    let entries: Vec<(PathBuf, std::fs::Metadata)> =
        entries.flatten().filter_map(|e| Some((e.path(), std::fs::symlink_metadata(e.path()).ok()?))).collect();
    own + entries.par_iter().map(|(p, m)| du(p, m, dev, seen)).sum::<u64>()
}

/// The job for a [`crate::SourceHub`]: git's facts first, then the size of each top-level entry
/// of the project and of each entry git ignores, as each is measured, on 4 threads (sizes are
/// disk-bound; more would only thrash). Without git it says so at once (Disk `Unavailable`),
/// still measures, and ends `Unavailable`.
pub fn disk_job() -> impl FnOnce(&JobContext) -> Result<(), JobError> + Send {
    move |ctx| {
        let mut entries: Vec<PathBuf> =
            std::fs::read_dir(&ctx.root).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        entries.sort();
        let mut no_git = None;
        match read_git_disk(&ctx.runner, &ctx.root) {
            Ok(git) => {
                for e in &git.ignored {
                    let path = ctx.root.join(e.path.trim_end_matches('/'));
                    if !entries.contains(&path) {
                        entries.push(path);
                    }
                }
                ctx.send(SourceEvent::GitDisk(std::sync::Arc::new(git)));
            }
            Err(JobError::Unavailable(why)) => {
                ctx.send(SourceEvent::Status {
                    source: SourceKind::Disk,
                    state: SourceState::Unavailable(why.clone()),
                });
                no_git = Some(why);
            }
            Err(e) => return Err(e),
        }
        let pool =
            rayon::ThreadPoolBuilder::new().num_threads(4).build().map_err(|e| JobError::Failed(e.to_string()))?;
        for entry in entries {
            if ctx.is_cancelled() {
                return Err(JobError::Cancelled);
            }
            let bytes = pool.install(|| du_bytes(&entry));
            let rel = entry.strip_prefix(&ctx.root).unwrap_or(&entry).to_path_buf();
            ctx.send(SourceEvent::FolderSize(rel, bytes));
        }
        no_git.map_or(Ok(()), |why| Err(JobError::Unavailable(why)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::CancelToken;

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn status_counts_each_kind_of_change() {
        let out = b"1 .M N... 100644 100644 100644 aa bb src/a.rs\0\
1 A. N... 000000 100644 100644 00 cc new.rs\0\
1 .D N... 100644 100644 000000 aa aa gone.rs\0\
2 R. N... 100644 100644 100644 aa aa R100 moved.rs\0was.rs\0\
u UU N... 100644 100644 100644 100644 aa bb cc both.rs\0\
? notes.txt\0";
        let c = parse_status_counts(out);
        assert_eq!((c.modified, c.added, c.deleted, c.renamed, c.conflicted, c.untracked), (1, 1, 1, 1, 1, 1));
        assert_eq!(c.total(), 6);
    }

    #[test]
    fn check_attr_output_sorts_into_lfs_generated_and_vendored() {
        let out = b"a.png\0filter\0lfs\0a.png\0linguist-generated\0unspecified\0a.png\0linguist-vendored\0unset\0\
gen/x.rs\0filter\0unspecified\0gen/x.rs\0linguist-generated\0set\0gen/x.rs\0linguist-vendored\0false\0\
v/y.js\0filter\0unspecified\0v/y.js\0linguist-generated\0false\0v/y.js\0linguist-vendored\0true\0\
d/z.rs\0linguist-documentation\0set\0a.png\0linguist-documentation\0unspecified\0";
        let attrs = parse_check_attr(out);
        assert_eq!(attrs.lfs, BTreeSet::from(["a.png".to_string()]));
        assert_eq!(attrs.generated, BTreeSet::from(["gen/x.rs".to_string()]));
        assert_eq!(attrs.vendored, BTreeSet::from(["v/y.js".to_string()]));
        assert_eq!(attrs.documentation, BTreeSet::from(["d/z.rs".to_string()]));
        assert_eq!(parse_check_attr(b""), AttrPaths::default());
    }

    #[test]
    fn git_says_what_is_tracked_ignored_and_by_which_rule() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(r)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !git(&["init", "-q"]) {
            return; // no git here
        }
        write(r, ".gitignore", b"# build output\n/target/\n*.log\n");
        write(r, ".gitattributes", b"*.png filter=lfs diff=lfs merge=lfs -text\n");
        write(r, "src/main.rs", b"fn main() {}\n");
        write(r, "logo.png", b"not really a png");
        write(r, "target/debug/app", b"binary");
        write(r, "run.log", b"log");
        assert!(git(&["add", "."]) && git(&["commit", "-qm", "one"]));
        write(r, "src/main.rs", b"fn main() { println!(); }\n");
        write(r, "draft.txt", b"new");

        let facts = read_git_disk(&Runner::new(CancelToken::default()), r).unwrap();
        assert_eq!(facts.tracked, 4, ".gitignore, .gitattributes, src/main.rs, logo.png");
        assert_eq!(facts.lfs, 1);
        assert_eq!(facts.lfs_paths, BTreeSet::from(["logo.png".to_string()]));
        let rule = |path: &str| facts.ignored.iter().find(|e| e.path == path).and_then(|e| e.rule.clone()).unwrap();
        assert_eq!((rule("target/").line, rule("target/").pattern.as_str()), (2, "/target/"));
        assert_eq!(rule("run.log").pattern, "*.log");
        assert_eq!((facts.changes.modified, facts.changes.untracked), (1, 1));

        let outside = tempfile::tempdir().unwrap();
        // A folder with a broken .git file is outside any repository, whatever surrounds it.
        write(outside.path(), ".git", b"not a gitdir\n");
        assert!(matches!(
            read_git_disk(&Runner::new(CancelToken::default()), outside.path()),
            Err(JobError::Unavailable(_)) | Err(JobError::Failed(_))
        ));
    }

    /// One tracked file per path below, each a different length; every class is met.
    const EVERY_CLASS: [&str; 22] = [
        ".gitignore",
        ".gitattributes",
        "Cargo.toml",
        "Cargo.lock",
        "settings.yaml",
        "src/main.rs",
        "src/lib.rs",
        "tests/it.rs",
        "src/a_test.go",
        "build.rs",
        "run.sh",
        "README.md",
        "guide/setup.rs",
        "api.schema.json",
        "logo.png",
        "gen/api.rs",
        ".github/workflows/ci.yml",
        ".ai/tickets/T-1.toml",
        "vendor/lib.js",
        "blob.bin",
        "out/old.rs",
        "notes.txt",
    ];

    /// The totals for [`EVERY_CLASS`] when file `i` is `i + 1` bytes long; `out/old.rs` is code,
    /// or build output when git ignores `out/`.
    fn every_class_totals(out_ignored: bool) -> Vec<ClassTotal> {
        let bytes = |rels: &[&str]| -> u64 {
            rels.iter().map(|rel| EVERY_CLASS.iter().position(|p| p == rel).unwrap() as u64 + 1).sum()
        };
        let total = |class, rels: &[&str], rules: &[(&str, usize)]| ClassTotal {
            class,
            files: rels.len(),
            bytes: bytes(rels),
            rules: rules.iter().map(|(why, n)| (why.to_string(), *n)).collect(),
        };
        let generated = ("gitattributes: linguist-generated", 1);
        let (code, build) = if out_ignored {
            (
                total(FileClass::Code, &["src/main.rs", "src/lib.rs"], &[("extension", 2)]),
                total(FileClass::BuildOutput, &["gen/api.rs", "out/old.rs"], &[generated, ("gitignored", 1)]),
            )
        } else {
            (
                total(FileClass::Code, &["src/main.rs", "src/lib.rs", "out/old.rs"], &[("extension", 3)]),
                total(FileClass::BuildOutput, &["gen/api.rs"], &[generated]),
            )
        };
        vec![
            code,
            total(
                FileClass::Tests,
                &["tests/it.rs", "src/a_test.go"],
                &[("file name", 1), ("tool convention: Cargo", 1)],
            ),
            total(FileClass::Tooling, &["build.rs", "run.sh"], &[("extension", 1), ("tool convention: Cargo", 1)]),
            total(
                FileClass::Docs,
                &["README.md", "guide/setup.rs", "notes.txt"],
                &[("extension", 1), ("file name", 1), ("gitattributes: linguist-documentation", 1)],
            ),
            total(FileClass::Schemas, &["api.schema.json"], &[("file name", 1)]),
            total(FileClass::Assets, &["logo.png"], &[("extension", 1)]),
            total(
                FileClass::Config,
                &[".gitignore", ".gitattributes", "Cargo.toml", "Cargo.lock", "settings.yaml"],
                &[("file name", 4), ("extension", 1)],
            ),
            build,
            total(FileClass::Ci, &[".github/workflows/ci.yml"], &[("tool convention: GitHub Actions", 1)]),
            total(FileClass::Tickets, &[".ai/tickets/T-1.toml"], &[("tool convention: Studio", 1)]),
            total(FileClass::Vendored, &["vendor/lib.js"], &[("gitattributes: linguist-vendored", 1)]),
            total(FileClass::Other, &["blob.bin"], &[("fallback", 1)]),
        ]
    }

    #[test]
    fn tracked_files_are_totalled_by_class_in_table_order() {
        let set = |paths: &[&str]| paths.iter().map(|p| p.to_string()).collect::<BTreeSet<_>>();
        let tracked: Vec<String> = EVERY_CLASS.iter().map(|p| p.to_string()).collect();
        let inputs = ClassInputs {
            files: tracked.iter().cloned().collect(),
            ignored: set(&["out/"]),
            generated: set(&["gen/api.rs"]),
            vendored: set(&["vendor/lib.js"]),
            documentation: set(&["guide/setup.rs"]),
        };
        let size = |rel: &str| EVERY_CLASS.iter().position(|p| *p == rel).unwrap() as u64 + 1;
        let totals = class_totals(&tracked, &inputs, size);
        assert_eq!(totals, every_class_totals(true));
        assert_eq!(totals.iter().map(|t| t.files).sum::<usize>(), EVERY_CLASS.len());
        // A class without files is left out; the rest keep the table's order.
        let few = class_totals(&tracked[..2], &inputs, size);
        assert_eq!(few.iter().map(|t| (t.class, t.files)).collect::<Vec<_>>(), [(FileClass::Config, 2)]);
    }

    #[test]
    fn git_classes_every_tracked_file_with_its_attributes() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(r)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !git(&["init", "-q"]) {
            return; // no git here
        }
        // Each file is as long as its place in the list; the two git files are padded to theirs.
        let attrs = "gen/** linguist-generated\nvendor/** linguist-vendored\nguide/** linguist-documentation\n";
        for (i, rel) in EVERY_CLASS.iter().enumerate() {
            let text = match *rel {
                ".gitattributes" => attrs.to_string(),
                _ => "x".repeat(i + 1),
            };
            write(r, rel, text.as_bytes());
        }
        assert!(git(&["add", "."]) && git(&["commit", "-qm", "one"]));
        let facts = read_git_disk(&Runner::new(CancelToken::default()), r).unwrap();
        assert_eq!(facts.documentation_paths, BTreeSet::from(["guide/setup.rs".to_string()]));
        // `.gitattributes` holds its rules, not two bytes: correct its length in the expected totals.
        let mut want = every_class_totals(false);
        want[6].bytes += attrs.len() as u64 - 2;
        assert_eq!(facts.classes, want);
    }

    #[cfg(unix)]
    #[test]
    fn sizes_match_du_and_count_hard_links_once() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "a/one.bin", &vec![1u8; 10_000]);
        write(r, "a/b/two.bin", &vec![2u8; 70_000]);
        std::fs::hard_link(r.join("a/one.bin"), r.join("a/b/again.bin")).unwrap();
        let ours = du_bytes(&r.join("a"));
        let theirs = std::process::Command::new("du").args(["-s", "--block-size=1"]).arg(r.join("a")).output();
        if let Some(out) = theirs.ok().filter(|o| o.status.success()) {
            let du: u64 = String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap().parse().unwrap();
            assert_eq!(ours, du);
        }
        let twice = du_bytes(&r.join("a/one.bin")) + du_bytes(&r.join("a/b/again.bin"));
        assert!(ours < twice + du_bytes(&r.join("a/b/two.bin")) + 3 * 4096, "the linked file counts once");
    }
}
