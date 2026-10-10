//! The session index on a fake `~/.claude` (`tests/fixtures/agents`): every number in
//! `expected.toml`, no text in the cache, and resuming from byte offsets.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use studio_sources::agents::NO_SESSIONS;
use studio_sources::{index_sessions, index_sessions_with, AgentIndex, IndexOptions, JobError, Store, TouchKind};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/agents");

/// A copy of the fixture with its placeholders filled.
struct Setup {
    _dir: tempfile::TempDir,
    base: PathBuf,
    root: PathBuf,
    claude: PathBuf,
}

impl Setup {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let root = base.join("repo");
        for sub in ["src", "sub", "wt", "docs"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        std::fs::write(root.join("README.md"), "").unwrap();
        std::fs::create_dir_all(base.join("other")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, base.join("link")).unwrap();
        let claude = base.join("claude");
        let fill = |text: &str| {
            text.replace("{ROOT}", root.to_str().unwrap())
                .replace("{LINK}", base.join("link").to_str().unwrap())
                .replace("{OTHER}", base.join("other").to_str().unwrap())
                .replace("{PARENT}", base.to_str().unwrap())
                .replace("{CLAUDE}", claude.to_str().unwrap())
        };
        copy_filled(&Path::new(FIXTURE).join("claude"), &claude, &fill);
        Setup { _dir: dir, base, root, claude }
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![self.root.clone(), self.root.join("wt")]
    }

    fn index(&self, store: Option<&Store>) -> AgentIndex {
        index_sessions(&self.claude, &self.roots(), store, None).unwrap()
    }

    fn log(&self, rel: &str) -> PathBuf {
        self.claude.join("projects").join(rel)
    }

    fn store(&self) -> Store {
        Store::in_dir(self.base.join("store"))
    }

    fn append(&self, rel: &str, text: &str) {
        let mut f = std::fs::OpenOptions::new().append(true).open(self.log(rel)).unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }
}

fn copy_filled(from: &Path, to: &Path, fill: &dyn Fn(&str) -> String) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_filled(&entry.path(), &target, fill);
        } else {
            let text = std::fs::read_to_string(entry.path()).unwrap();
            std::fs::write(&target, fill(&text)).unwrap();
        }
    }
}

fn expected() -> toml::Value {
    let text = std::fs::read_to_string(Path::new(FIXTURE).join("expected.toml")).unwrap();
    toml::from_str(&text).unwrap()
}

fn kind_name(kind: TouchKind) -> &'static str {
    match kind {
        TouchKind::Read => "read",
        TouchKind::Edit => "edit",
        TouchKind::Write => "write",
        TouchKind::BashEdit => "bash_edit",
    }
}

/// An assistant line calling `Edit` on `path`, and its result line.
fn edit_lines(session: &str, id: &str, path: &str, time: &str) -> String {
    let call = serde_json::json!({
        "type": "assistant", "uuid": format!("{id}-call"), "timestamp": time, "sessionId": session, "cwd": "/",
        "message": {"role": "assistant", "content": [{"type": "tool_use", "id": id, "name": "Edit",
            "input": {"file_path": path, "old_string": "SECRET-PROMPT-7f3a", "new_string": "SECRET-PROMPT-7f3a"}}]}
    });
    let result = serde_json::json!({
        "type": "user", "uuid": format!("{id}-result"), "timestamp": time, "sessionId": session, "cwd": "/",
        "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": "SECRET-PROMPT-7f3a"}]},
        "toolUseResult": {"filePath": path, "originalFile": "SECRET-PROMPT-7f3a",
            "structuredPatch": [{"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 2, "lines": ["+SECRET-PROMPT-7f3a"]}]}
    });
    format!("{call}\n{result}\n")
}

#[test]
fn the_fixture_gives_every_expected_number() {
    let setup = Setup::new();
    let index = setup.index(None);
    let want = expected();
    let int = |key: &str| want[key].as_integer().unwrap() as u64;
    let s = &index.stats;

    assert_eq!(index.sessions.len() as u64, int("sessions"));
    assert_eq!(s.main_logs as u64, int("main_logs"));
    assert_eq!(s.subagent_logs as u64, int("subagent_logs"));
    assert_eq!(index.logs.len() as u64, int("main_logs") + int("subagent_logs"));
    assert_eq!(s.lines, int("lines"));
    assert_eq!(s.bad_lines, int("bad_lines"));
    assert_eq!(s.duplicate_lines, int("duplicate_lines"));
    assert_eq!(s.out_of_repo_touches, int("out_of_repo_touches"));
    assert_eq!(s.out_of_repo_gone, int("out_of_repo_gone"));
    assert_eq!(s.out_of_repo_existing, int("out_of_repo_existing"));
    let top: Vec<(String, u64)> = want["out_of_repo_top"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let t = t.as_array().unwrap();
            (setup.base.join(t[0].as_str().unwrap()).to_string_lossy().into_owned(), t[1].as_integer().unwrap() as u64)
        })
        .collect();
    assert_eq!(s.out_of_repo_top, top);
    assert_eq!(s.later_cwd_logs as u64, int("later_cwd_logs"));
    assert_eq!(s.later_cwd_sessions as u64, int("later_cwd_sessions"));
    assert_eq!(index.folders.len() as u64, int("folders"));
    let mapped: Vec<String> = index
        .folders
        .iter()
        .filter(|f| f.mapped)
        .map(|f| f.folder.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let want_mapped: Vec<String> =
        want["folders_mapped"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(mapped, want_mapped);
    let unknown: BTreeMap<String, u64> = want["unknown_types"]
        .as_table()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_integer().unwrap() as u64))
        .collect();
    assert_eq!(s.unknown_types, unknown);

    let touches = want["touches"].as_table().unwrap();
    for kind in [TouchKind::Read, TouchKind::Edit, TouchKind::Write, TouchKind::BashEdit] {
        let n = index.touches.iter().filter(|t| t.kind == kind).count() as i64;
        assert_eq!(n, touches[kind_name(kind)].as_integer().unwrap(), "{kind:?} touches");
    }
    let subagent = index.touches.iter().filter(|t| t.subagent).count() as i64;
    assert_eq!(subagent, touches["subagent"].as_integer().unwrap());

    let mut per_file: BTreeMap<String, i64> = BTreeMap::new();
    for t in &index.touches {
        let rel = t.path.strip_prefix(&setup.root).expect("touches are inside the roots");
        *per_file.entry(rel.to_string_lossy().into_owned()).or_default() += 1;
    }
    let want_per_file: BTreeMap<String, i64> =
        want["per_file"].as_table().unwrap().iter().map(|(k, v)| (k.clone(), v.as_integer().unwrap())).collect();
    assert_eq!(per_file, want_per_file);

    // Sessions, in order.
    let sessions = want["session"].as_array().unwrap();
    assert_eq!(index.sessions.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), {
        sessions.iter().map(|s| s["id"].as_str().unwrap()).collect::<Vec<_>>()
    });
    for (got, want) in index.sessions.iter().zip(sessions) {
        let id = &got.id;
        assert_eq!(got.slug.as_deref(), want.get("slug").and_then(|v| v.as_str()), "{id} slug");
        assert_eq!(got.root as i64, want["root"].as_integer().unwrap(), "{id} root");
        let branches: Vec<&str> = want["branches"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(got.branches, branches, "{id} branches");
        assert_eq!(got.started, want["started"].as_integer().unwrap(), "{id} started");
        assert_eq!(got.ended, want["ended"].as_integer().unwrap(), "{id} ended");
        assert_eq!(got.logs.len() as i64, want["logs"].as_integer().unwrap(), "{id} logs");
        assert_eq!(got.touch_count as i64, want["touches"].as_integer().unwrap(), "{id} touches");
        assert!(got.logs.iter().all(|&l| index.logs[l].session == index.sessions.iter().position(|s| &s.id == id)));
        let plan = got.plan.as_ref();
        let kind = match plan {
            None => "none",
            Some(p) if p.len == 0 => "slug",
            Some(p) if p.path.is_some() => "line",
            Some(_) => "line-no-file",
        };
        assert_eq!(kind, want["plan"].as_str().unwrap(), "{id} plan");
        if let Some(file) = want.get("plan_file").and_then(|v| v.as_str()) {
            assert_eq!(plan.unwrap().path.as_ref().unwrap(), &setup.claude.join("plans").join(file), "{id} plan file");
        }
        if let Some(time) = want.get("plan_time").and_then(|v| v.as_integer()) {
            let p = plan.unwrap();
            assert_eq!(p.time, time, "{id} plan time");
            // The offset points at the ExitPlanMode line.
            let bytes = std::fs::read(&index.logs[p.log].path).unwrap();
            let line = &bytes[p.offset as usize..p.offset as usize + p.len as usize];
            assert!(line.windows(14).any(|w| w == b"\"ExitPlanMode\""), "{id} plan line");
        }
        let tasks: Vec<&str> = want["tasks"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(got.tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), tasks, "{id} tasks");
        if let Some(states) = want.get("task_states").and_then(|v| v.as_array()) {
            let got_states: Vec<(String, i64, String)> = got
                .tasks
                .iter()
                .flat_map(|t| t.states.iter().map(move |(time, s)| (t.id.clone(), *time, s.label().replace(' ', "_"))))
                .collect();
            let want_states: Vec<(String, i64, String)> = states
                .iter()
                .map(|s| {
                    let s = s.as_array().unwrap();
                    (s[0].as_str().unwrap().into(), s[1].as_integer().unwrap(), s[2].as_str().unwrap().into())
                })
                .collect();
            assert_eq!(got_states, want_states, "{id} task states");
            for t in &got.tasks {
                let bytes = std::fs::read(&index.logs[t.log].path).unwrap();
                let line = &bytes[t.offset as usize..t.offset as usize + t.len as usize];
                assert!(line.windows(12).any(|w| w == b"\"TaskCreate\""), "{id} task line");
            }
        }
    }

    // Every touch, exactly: nothing more, nothing less.
    type Row = (String, String, String, i64, String, usize, bool, Option<(u32, u32)>);
    let mut got: Vec<Row> = index
        .touches
        .iter()
        .map(|t| {
            let session = &index.sessions[t.session];
            let task = t.task.map(|i| session.tasks[i].id.clone()).unwrap_or_default();
            let rel = t.path.strip_prefix(&setup.root).unwrap().to_string_lossy().into_owned();
            (session.id.clone(), kind_name(t.kind).into(), rel, t.time, task, t.hunks.len(), t.subagent, t.read)
        })
        .collect();
    let mut want_rows: Vec<Row> = want["touch"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let read = t
                .get("read")
                .and_then(|r| r.as_array())
                .map(|r| (r[0].as_integer().unwrap() as u32, r[1].as_integer().unwrap() as u32));
            (
                t["session"].as_str().unwrap().into(),
                t["kind"].as_str().unwrap().into(),
                t["path"].as_str().unwrap().into(),
                t["time"].as_integer().unwrap(),
                t["task"].as_str().unwrap().into(),
                t["hunks"].as_integer().unwrap() as usize,
                t.get("subagent").and_then(|v| v.as_bool()).unwrap_or(false),
                read,
            )
        })
        .collect();
    got.sort();
    want_rows.sort();
    assert_eq!(got, want_rows);

    // Each touch's offsets point at its call line, and its result at the result line.
    for t in &index.touches {
        let bytes = std::fs::read(&index.logs[t.log].path).unwrap();
        let line = &bytes[t.offset as usize..t.offset as usize + t.len as usize];
        assert!(line.starts_with(b"{") && line.ends_with(b"}"), "a whole line");
        assert!(line.windows(10).any(|w| w == b"\"tool_use\""));
        if let Some((log, offset, len)) = t.result {
            let bytes = std::fs::read(&index.logs[log].path).unwrap();
            let line = &bytes[offset as usize..offset as usize + len as usize];
            assert!(line.windows(13).any(|w| w == b"\"tool_result\""));
        }
    }
}

#[test]
fn the_cache_holds_no_text_from_the_logs() {
    let setup = Setup::new();
    let store = setup.store();
    setup.index(Some(&store));
    let marker = expected()["marker"].as_str().unwrap().to_string();
    let mut files = 0;
    for entry in walkdir::WalkDir::new(setup.base.join("store")) {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            files += 1;
            let bytes = std::fs::read(entry.path()).unwrap();
            for needle in [marker.as_bytes(), b"SECRET"] {
                assert!(!bytes.windows(needle.len()).any(|w| w == needle), "{} holds log text", entry.path().display());
            }
        }
    }
    assert_eq!(files, 1, "the cache was written");
}

#[test]
fn a_warm_run_reads_nothing_and_agrees() {
    let setup = Setup::new();
    let store = setup.store();
    let cold = setup.index(Some(&store));
    assert_eq!(cold.stats.new_files, 6);
    assert!(cold.stats.later_cwd_bytes > 0, "logs that start outside were scanned");
    let warm = setup.index(Some(&store));
    assert_eq!((warm.stats.new_files, warm.stats.resumed_files, warm.stats.reparsed_files), (0, 0, 0));
    assert_eq!(warm.stats.later_cwd_bytes, 0, "the scan is cached");
    assert_eq!(warm.stats.later_cwd_sessions, cold.stats.later_cwd_sessions);
    assert_eq!((&warm.sessions, &warm.touches, &warm.logs), (&cold.sessions, &cold.touches, &cold.logs));
    assert_eq!(warm.stats.duplicate_lines, cold.stats.duplicate_lines);
    assert_eq!(warm.stats.unknown_types, cold.stats.unknown_types);
}

#[test]
fn appended_lines_are_read_from_where_the_cache_stopped() {
    let setup = Setup::new();
    let store = setup.store();
    let before = setup.index(Some(&store));
    let late = setup.root.join("src/late.rs");
    setup.append("-repo-a/s1.jsonl", &edit_lines("s1", "toolu_late", late.to_str().unwrap(), "2026-10-01T10:30:00Z"));
    let after = setup.index(Some(&store));
    assert_eq!(after.stats.resumed_files, 1);
    assert_eq!(after.stats.reparsed_files, 0);
    assert_eq!(after.stats.new_files, 0);
    assert_eq!(after.touches.len(), before.touches.len() + 1);
    let new = after.touches.iter().find(|t| t.path == late).expect("the appended edit");
    assert_eq!((new.kind, new.hunks.len(), new.task.is_some()), (TouchKind::Edit, 1, true), "task 2 is in progress");
    // The same as reading everything afresh.
    let fresh = setup.index(None);
    assert_eq!((&after.sessions, &after.touches), (&fresh.sessions, &fresh.touches));
    assert_eq!(after.stats.lines, fresh.stats.lines);
}

#[test]
fn an_incomplete_last_line_waits_for_its_newline() {
    let setup = Setup::new();
    let store = setup.store();
    let before = setup.index(Some(&store));
    let late = setup.root.join("src/late.rs");
    assert!(before.touches.iter().all(|t| t.path != late));
    setup.append("-link/s2.jsonl", "\n");
    let after = setup.index(Some(&store));
    assert_eq!(after.stats.resumed_files, 1);
    let new = after.touches.iter().find(|t| t.path == late).expect("the completed line");
    assert_eq!(after.sessions[new.session].id, "s2");
    assert_eq!(new.result, None, "its result was never written");
}

#[test]
fn a_shrunk_or_replaced_file_is_read_again() {
    let setup = Setup::new();
    let store = setup.store();
    setup.index(Some(&store));
    // Shrunk: s1 keeps its first eight lines.
    let s1 = setup.log("-repo-a/s1.jsonl");
    let text = std::fs::read_to_string(&s1).unwrap();
    let kept: String = text.split_inclusive('\n').take(8).collect();
    std::fs::write(&s1, kept).unwrap();
    let after = setup.index(Some(&store));
    assert_eq!((after.stats.reparsed_files, after.stats.resumed_files), (1, 0));
    let fresh = setup.index(None);
    assert_eq!((&after.sessions, &after.touches), (&fresh.sessions, &fresh.touches));
    assert_eq!(after.stats.duplicate_lines, fresh.stats.duplicate_lines);

    // Replaced by a longer file that starts differently.
    let s3 = setup.log("-repo-a/s3.jsonl");
    let other = setup.root.join("src/other.rs");
    let mut text = edit_lines("s3", "toolu_other", other.to_str().unwrap(), "2026-10-01T12:00:00Z");
    text = text.replace("\"cwd\":\"/\"", &format!("\"cwd\":\"{}\"", setup.root.display()));
    text.push_str(&std::fs::read_to_string(&s3).unwrap());
    std::fs::write(&s3, text).unwrap();
    let after = setup.index(Some(&store));
    assert_eq!((after.stats.reparsed_files, after.stats.resumed_files), (1, 0));
    let fresh = setup.index(None);
    assert_eq!((&after.sessions, &after.touches), (&fresh.sessions, &fresh.touches));
    assert!(after.touches.iter().any(|t| t.path == other));
}

#[test]
fn a_log_that_moves_in_later_maps_from_that_run_on() {
    let setup = Setup::new();
    let store = setup.store();
    let before = setup.index(Some(&store));
    assert!(!before.sessions.iter().any(|s| s.id == "s8"));
    let s8 = setup.log("-other/s8.jsonl");
    let len = std::fs::metadata(&s8).unwrap().len();
    let late = setup.root.join("src/late.rs");
    let lines = edit_lines("s8", "toolu_s8late", late.to_str().unwrap(), "2026-10-01T14:30:00Z");
    setup.append("-other/s8.jsonl", &lines.replace("\"cwd\":\"/\"", &format!("\"cwd\":\"{}\"", setup.root.display())));
    let after = setup.index(Some(&store));
    let grown = std::fs::metadata(&s8).unwrap().len();
    let scanned = after.stats.later_cwd_bytes;
    assert!(scanned > 0 && scanned <= grown - len, "only the new lines are scanned");
    let s = after.sessions.iter().position(|s| s.id == "s8").expect("s8 maps once a line is inside");
    // Read from its start: the edit made before it moved in counts too.
    let paths: Vec<&Path> = after.touches.iter().filter(|t| t.session == s).map(|t| t.path.as_path()).collect();
    assert_eq!(paths, [setup.root.join("src/lib.rs").as_path(), late.as_path()]);
    let fresh = setup.index(None);
    assert_eq!((&after.sessions, &after.touches), (&fresh.sessions, &fresh.touches));
}

#[test]
fn nothing_to_index_says_so() {
    let setup = Setup::new();
    let none = index_sessions(&setup.base.join("nowhere"), &setup.roots(), None, None);
    assert_eq!(none, Err(JobError::Unavailable(NO_SESSIONS.to_string())));
    let unrelated = index_sessions(&setup.claude, &[setup.base.join("unrelated")], None, None);
    assert_eq!(unrelated, Err(JobError::Unavailable(NO_SESSIONS.to_string())));
}

#[test]
fn parent_folder_sessions_map_only_when_asked() {
    let setup = Setup::new();
    let options = IndexOptions { map_parent_cwds: true };
    let index = index_sessions_with(&setup.claude, &setup.roots(), None, None, &options).unwrap();
    assert!(index.sessions.iter().any(|s| s.id == "s5"));
    assert!(!index.sessions.iter().any(|s| s.id == "s4"), "an unrelated folder never maps");
    assert!(index.touches.iter().any(|t| t.path == setup.root.join("x.rs")));
}
