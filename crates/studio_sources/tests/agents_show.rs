//! Show-time reads on the fake `~/.claude` (`tests/fixtures/agents`): plan text, task steps,
//! edit needles and where each edit sits in a file's current text.

use std::path::{Path, PathBuf};

use studio_sources::agents::{
    match_touch, plan_text, task_steps, touch_needle, Needle, PlanText, TaskStep, TouchMatch,
};
use studio_sources::{index_sessions, AgentIndex, HunkRange, TaskState};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/agents");
const MARKER: &str = "SECRET-PROMPT-7f3a";

/// A copy of the fixture with its placeholders filled, and its index.
struct Setup {
    _dir: tempfile::TempDir,
    base: PathBuf,
    claude: PathBuf,
    index: AgentIndex,
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
        let index = index_sessions(&claude, &[root.clone(), root.join("wt")], None, None).unwrap();
        Setup { _dir: dir, base, claude, index }
    }

    fn session(&self, id: &str) -> usize {
        self.index.sessions.iter().position(|s| s.id == id).unwrap()
    }

    /// The touch made at `time` (unix ms; unique in the fixture).
    fn touch(&self, time: i64) -> usize {
        let found: Vec<usize> = (0..self.index.touches.len()).filter(|&i| self.index.touches[i].time == time).collect();
        assert_eq!(found.len(), 1, "one touch at {time}");
        found[0]
    }

    fn plan(&self, slug: &str) -> PathBuf {
        self.claude.join("plans").join(format!("{slug}.md"))
    }

    fn log(&self, rel: &str) -> PathBuf {
        self.claude.join("projects").join(rel)
    }

    /// Every file under the temp dir, to show the reads write nothing.
    fn files(&self) -> Vec<PathBuf> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    walk(&e.path(), out);
                } else {
                    out.push(e.path());
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.base, &mut out);
        out.sort();
        out
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

// Touch times (expected.toml).
const E1_HUNK: i64 = 1_790_848_860_000; // Edit src/lib.rs, structuredPatch newStart 3
const W1: i64 = 1_790_848_980_000; // Write src/new.rs
const R1: i64 = 1_790_849_040_000; // Read src/lib.rs offset 10 limit 20
const B1: i64 = 1_790_849_100_000; // Bash edit src/main.rs, bashEditDiff newStart 1
const E3_ALL: i64 = 1_790_849_280_000; // Edit src/lib.rs, replace_all
const X1_SUB: i64 = 1_790_849_370_000; // subagent Edit src/lib.rs, no hunks
const E5_REL: i64 = 1_790_849_520_000; // Edit ../docs/rel.md from cwd src, no hunks
const S2_LINK: i64 = 1_790_931_660_000; // Edit through a symlinked root, newStart 5

#[test]
fn plan_from_file() {
    let s = Setup::new();
    let path = s.plan("brave-blue-fox");
    assert_eq!(
        plan_text(&s.index, s.session("s1")),
        PlanText::File { path: path.clone(), text: std::fs::read_to_string(&path).unwrap() }
    );
    // Found by its slug only (no ExitPlanMode line).
    let PlanText::File { path, text } = plan_text(&s.index, s.session("s3")) else { panic!("s3 plan file") };
    assert_eq!(path, s.plan("calm-green-owl"));
    assert!(text.contains(&format!("{MARKER} resumed plan")));
}

#[test]
fn plan_from_log_when_the_file_is_deleted() {
    let s = Setup::new();
    let path = s.plan("brave-blue-fox");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        plan_text(&s.index, s.session("s1")),
        PlanText::FromLog { text: format!("# {MARKER} plan"), deleted: Some(path) }
    );
    // A plan line with no file at all.
    assert_eq!(plan_text(&s.index, s.session("s2")), PlanText::FromLog { text: MARKER.to_string(), deleted: None });
    // Found by file only, and the file is gone: nothing else holds it.
    let slug_plan = s.plan("calm-green-owl");
    std::fs::remove_file(&slug_plan).unwrap();
    assert_eq!(plan_text(&s.index, s.session("s3")), PlanText::Deleted(slug_plan));
}

#[test]
fn no_plan() {
    let s = Setup::new();
    assert_eq!(plan_text(&s.index, s.session("s6")), PlanText::None);
    assert_eq!(plan_text(&s.index, 99), PlanText::None);
    assert!(task_steps(&s.index, s.session("s6")).is_empty());
}

#[test]
fn task_steps_in_create_order_with_subjects() {
    let s = Setup::new();
    assert_eq!(
        task_steps(&s.index, s.session("s1")),
        vec![
            TaskStep { index: 0, subject: format!("{MARKER} task one"), state: TaskState::Completed },
            TaskStep { index: 1, subject: format!("{MARKER} task two"), state: TaskState::InProgress },
        ]
    );
}

#[test]
fn needles_come_from_the_call_lines() {
    let s = Setup::new();
    assert_eq!(touch_needle(&s.index, s.touch(E1_HUNK)), Some(Needle::Text(format!("{MARKER} new"))));
    assert_eq!(touch_needle(&s.index, s.touch(W1)), Some(Needle::Text(MARKER.to_string())));
    assert_eq!(touch_needle(&s.index, s.touch(X1_SUB)), Some(Needle::Text(MARKER.to_string())));
    assert_eq!(touch_needle(&s.index, s.touch(R1)), None);
    let range = HunkRange { old_start: 1, old_lines: 1, new_start: 1, new_lines: 2 };
    assert_eq!(touch_needle(&s.index, s.touch(B1)), Some(Needle::Hunks(vec![(range, vec![MARKER.to_string()])])));
}

#[test]
fn found_once_is_lit() {
    let s = Setup::new();
    let file = format!("a\n{MARKER}\nb\n");
    assert_eq!(match_touch(&s.index, s.touch(X1_SUB), &file), TouchMatch::Lit(vec![2..=2]));
    // A relative path from a subfolder cwd.
    assert_eq!(match_touch(&s.index, s.touch(E5_REL), &format!("{MARKER}\n")), TouchMatch::Lit(vec![1..=1]));
    // A Write: its whole content.
    assert_eq!(match_touch(&s.index, s.touch(W1), &format!("x\n{MARKER}")), TouchMatch::Lit(vec![2..=2]));
}

#[test]
fn found_three_times_is_candidates() {
    let s = Setup::new();
    let file = format!("{MARKER}\nx\n{MARKER}\ny\n{MARKER}\n");
    assert_eq!(match_touch(&s.index, s.touch(X1_SUB), &file), TouchMatch::Candidates(vec![1..=1, 3..=3, 5..=5]));
}

#[test]
fn stale_after_the_file_changed() {
    let s = Setup::new();
    assert_eq!(match_touch(&s.index, s.touch(X1_SUB), "a\nb\n"), TouchMatch::Stale);
    assert_eq!(match_touch(&s.index, s.touch(E1_HUNK), "a\nb\n"), TouchMatch::Stale);
    assert_eq!(match_touch(&s.index, s.touch(B1), ""), TouchMatch::Stale);
}

#[test]
fn structured_patch_still_in_place() {
    let s = Setup::new();
    let file = format!("l1\nl2\n{MARKER}\nl4\n{MARKER} new\n");
    assert_eq!(match_touch(&s.index, s.touch(E1_HUNK), &file), TouchMatch::Lit(vec![3..=3]));
    // Through a symlinked root.
    let file = format!("1\n2\n3\n4\n{MARKER}\n{MARKER}\n");
    assert_eq!(match_touch(&s.index, s.touch(S2_LINK), &file), TouchMatch::Lit(vec![5..=5]));
}

#[test]
fn structured_patch_shifted_is_found_elsewhere() {
    let s = Setup::new();
    let file = format!("l1\nl2\nl3\nl4\nl5\n{MARKER}\n");
    assert_eq!(match_touch(&s.index, s.touch(E1_HUNK), &file), TouchMatch::Lit(vec![6..=6]));
    let twice = format!("l1\n{MARKER}\nl3\nl4\n{MARKER}\n");
    assert_eq!(match_touch(&s.index, s.touch(E1_HUNK), &twice), TouchMatch::Candidates(vec![2..=2, 5..=5]));
}

#[test]
fn bash_edit_hunks() {
    let s = Setup::new();
    let b1 = s.touch(B1);
    assert_eq!(match_touch(&s.index, b1, &format!("{MARKER}\nfn main() {{}}\n")), TouchMatch::Lit(vec![1..=1]));
    assert_eq!(match_touch(&s.index, b1, &format!("// moved\n{MARKER}\n")), TouchMatch::Lit(vec![2..=2]));
}

#[test]
fn replace_all_lights_every_occurrence() {
    let s = Setup::new();
    let file = format!("{MARKER}\nx\n{MARKER}\n");
    assert_eq!(match_touch(&s.index, s.touch(E3_ALL), &file), TouchMatch::Lit(vec![1..=1, 3..=3]));
    assert_eq!(match_touch(&s.index, s.touch(E3_ALL), "x\n"), TouchMatch::Stale);
}

#[test]
fn crlf_files_match() {
    let s = Setup::new();
    let file = format!("a\r\n{MARKER}\r\nb\r\n");
    assert_eq!(match_touch(&s.index, s.touch(X1_SUB), &file), TouchMatch::Lit(vec![2..=2]));
    let file = format!("l1\r\nl2\r\n{MARKER}\r\nl4\r\n");
    assert_eq!(match_touch(&s.index, s.touch(E1_HUNK), &file), TouchMatch::Lit(vec![3..=3]));
    assert_eq!(match_touch(&s.index, s.touch(B1), &format!("{MARKER}\r\n")), TouchMatch::Lit(vec![1..=1]));
}

#[test]
fn reads_are_read_only() {
    let s = Setup::new();
    assert_eq!(match_touch(&s.index, s.touch(R1), "anything"), TouchMatch::ReadOnly(Some(10..=29)));
    let whole = s.index.touches.iter().position(|t| t.subagent && t.read.is_none() && t.path.ends_with("README.md"));
    assert_eq!(match_touch(&s.index, whole.unwrap(), ""), TouchMatch::ReadOnly(None));
}

#[test]
fn a_rewritten_log_is_unavailable() {
    let s = Setup::new();
    let changed = TouchMatch::Unavailable("log changed".to_string());
    // Every offset in s1.jsonl moves by one line.
    let main = s.log("-repo-a/s1.jsonl");
    let text = std::fs::read_to_string(&main).unwrap();
    std::fs::write(&main, format!("{{\"type\":\"summary\"}}\n{text}")).unwrap();
    for time in [E1_HUNK, W1, B1, E3_ALL, E5_REL] {
        assert_eq!(match_touch(&s.index, s.touch(time), MARKER), changed, "touch at {time}");
        assert_eq!(touch_needle(&s.index, s.touch(time)), None);
    }
    assert!(task_steps(&s.index, s.session("s1")).iter().all(|t| t.subject.is_empty()));
    std::fs::remove_file(s.plan("brave-blue-fox")).unwrap();
    assert_eq!(plan_text(&s.index, s.session("s1")), PlanText::Deleted(s.plan("brave-blue-fox")));
    // A log cut short, and one deleted.
    let sub = s.log("-repo-a/s1/subagents/agent-ax1.jsonl");
    std::fs::write(&sub, "").unwrap();
    assert_eq!(match_touch(&s.index, s.touch(X1_SUB), MARKER), changed);
    std::fs::remove_file(&sub).unwrap();
    assert_eq!(match_touch(&s.index, s.touch(X1_SUB), MARKER), TouchMatch::Unavailable("log deleted".to_string()));
    assert_eq!(match_touch(&s.index, 9999, MARKER), TouchMatch::Unavailable("no such touch".to_string()));
}

#[test]
fn show_reads_write_nothing() {
    let s = Setup::new();
    let before = s.files();
    for session in 0..s.index.sessions.len() {
        plan_text(&s.index, session);
        task_steps(&s.index, session);
    }
    for touch in 0..s.index.touches.len() {
        touch_needle(&s.index, touch);
        match_touch(&s.index, touch, MARKER);
    }
    assert_eq!(s.files(), before);
}

/// Every touch of real projects (`STUDIO_REAL_REPOS`, colon-separated) matched against its file
/// as it is now: counts per outcome and timings only, no text.
/// `STUDIO_REAL_REPOS=/path/a:/path/b cargo test --release -p studio_sources --test agents_show -- --ignored --nocapture`
#[test]
#[ignore]
fn real_projects_match_counts() {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};
    let Some(claude) = studio_sources::default_claude_dir() else { return };
    let repos = std::env::var("STUDIO_REAL_REPOS").unwrap_or_default();
    for repo in repos.split(':').filter(|r| !r.is_empty()) {
        let Ok(index) = index_sessions(&claude, &[PathBuf::from(repo)], None, None) else { continue };
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let (mut total, mut worst) = (Duration::ZERO, Duration::ZERO);
        for (i, t) in index.touches.iter().enumerate() {
            let Ok(text) = std::fs::read_to_string(&t.path) else {
                *counts.entry("file gone".to_string()).or_default() += 1;
                continue;
            };
            let started = Instant::now();
            let found = match_touch(&index, i, &text);
            let took = started.elapsed();
            total += took;
            worst = worst.max(took);
            let name = match found {
                TouchMatch::Lit(_) => "lit".to_string(),
                TouchMatch::Candidates(_) => "candidates".to_string(),
                TouchMatch::Stale => "stale".to_string(),
                TouchMatch::ReadOnly(_) => "read".to_string(),
                TouchMatch::Unavailable(why) => format!("unavailable: {why}"),
            };
            *counts.entry(name).or_default() += 1;
        }
        let started = Instant::now();
        let plans = (0..index.sessions.len()).filter(|&s| plan_text(&index, s) != PlanText::None).count();
        let steps: usize = (0..index.sessions.len()).map(|s| task_steps(&index, s).len()).sum();
        println!(
            "{repo}: {} touches {counts:?}; match total {total:.2?}, worst {worst:.2?}; {plans} plans, {steps} steps in {:.2?}",
            index.touches.len(),
            started.elapsed()
        );
    }
}
