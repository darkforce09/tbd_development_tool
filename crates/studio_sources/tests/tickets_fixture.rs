//! `read_tickets` on `tests/fixtures/tickets`, checked against its `expected.toml`.

use std::path::{Path, PathBuf};

use studio_sources::{
    next_tickets, read_tickets, tickets_job, CancelToken, JobError, Runner, Shipped, SourceEvent, SourceHub,
    SourceKind, TicketIndex,
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tickets")
}

fn expected() -> toml::Table {
    std::fs::read_to_string(fixture().join("expected.toml")).unwrap().parse().unwrap()
}

/// The fixture's `.ai` folder copied into a new temporary project.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let from = fixture();
    for entry in walkdir::WalkDir::new(from.join(".ai")) {
        let entry = entry.unwrap();
        let to = dir.path().join(entry.path().strip_prefix(&from).unwrap());
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&to).unwrap();
        } else {
            std::fs::copy(entry.path(), &to).unwrap();
        }
    }
    dir
}

/// Makes `dir` a git repository with one commit whose id is fixed: same content, author and
/// dates, no user or system config. False when git is not installed.
fn commit_fixture(dir: &Path) -> bool {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
            .output()
    };
    let Ok(init) = git(&["init", "-q", "."]) else { return false };
    assert!(init.status.success());
    std::fs::write(dir.join("README"), "fixture\n").unwrap();
    assert!(git(&["add", "README"]).unwrap().status.success());
    assert!(git(&["commit", "-q", "-m", "Ship T-1"]).unwrap().status.success());
    let head = git(&["rev-parse", "HEAD"]).unwrap();
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "35c962b8419dc5e2e44930ef9c1a9214b9c4e06f");
    true
}

fn runner() -> Runner {
    Runner::new(CancelToken::default())
}

fn strings(value: &toml::Value) -> Vec<String> {
    value.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

/// Every ticket's `shipped` as `expected.toml` writes it; tickets without `shipped_at` left out.
fn shipped(index: &TicketIndex) -> toml::Table {
    index
        .tickets
        .iter()
        .filter(|t| t.shipped_at.is_some() || t.shipped != Shipped::None)
        .map(|t| {
            let said = match &t.shipped {
                Shipped::Commit(full) => full.clone(),
                Shipped::NotInRepo => "not in repo".to_string(),
                Shipped::None => "not checked".to_string(),
            };
            (t.id.clone(), toml::Value::String(said))
        })
        .collect()
}

fn check_counts(index: &TicketIndex, expected: &toml::Table) {
    assert_eq!(index.files as i64, expected["files"].as_integer().unwrap());
    assert_eq!(index.tickets.len() as i64, expected["tickets"].as_integer().unwrap());
    let bad: Vec<String> = index.bad_files.iter().map(|(name, _)| name.clone()).collect();
    assert_eq!(bad, strings(&expected["bad_files"]));
    let counts: toml::Table =
        index.status_counts.iter().map(|(status, n)| (status.clone(), toml::Value::Integer(*n as i64))).collect();
    assert_eq!(&counts, expected["status_counts"].as_table().unwrap());
    let next: Vec<String> = next_tickets(index, usize::MAX).iter().map(|t| t.id.clone()).collect();
    assert_eq!(next, strings(&expected["next"]));
    assert_eq!(next_tickets(index, 3).len(), 3);
}

#[test]
fn the_fixture_reads_as_expected_with_git_confirming_shipped_commits() {
    let project = project();
    if !commit_fixture(project.path()) {
        return; // no git here
    }
    let expected = expected();
    let index = read_tickets(&runner(), project.path()).unwrap();
    check_counts(&index, &expected);
    assert_eq!(&shipped(&index), expected["shipped"].as_table().unwrap());

    let t1 = index.tickets.iter().find(|t| t.id == "T-1").unwrap();
    assert_eq!(t1.file, ".ai/tickets/T-1.toml");
    assert_eq!(t1.shipped_at.as_deref(), Some("35c962b"));
    assert_eq!((t1.kind.as_deref(), t1.priority, t1.order), (Some("program"), Some(1), Some(1)));
    assert_eq!((t1.spec.as_deref(), t1.plan.as_deref()), (Some("docs/specs/t1.md"), Some("docs/plans/t1.md")));
    let nested = index.tickets.iter().find(|t| t.id == "T-1.2.3.4").unwrap();
    assert_eq!((nested.parent.as_deref(), nested.priority), (Some("T-1"), None));
    let ids: Vec<&str> = index.tickets.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["T-1", "T-1.2.3.4", "T-2", "T-2.9", "T-2.10", "T-3", "T-4", "T-5", "T-6", "T-7", "T-9", "T-10"]);
    assert_eq!(index.folder, project.path().join(".ai/tickets"));
    for (name, why) in &index.bad_files {
        assert!(!why.contains('\n') && !why.contains("Broken"), "{name}: {why}");
    }
}

#[test]
fn without_git_nothing_is_checked_and_shipped_at_is_kept() {
    let project = project();
    let inside_a_repo = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(project.path())
        .output()
        .is_ok_and(|o| o.status.success());
    if inside_a_repo {
        return; // the temp folder is inside some repository; nothing to show here
    }
    let expected = expected();
    let index = read_tickets(&runner(), project.path()).unwrap();
    check_counts(&index, &expected);
    assert_eq!(&shipped(&index), expected["shipped_without_git"].as_table().unwrap());
}

#[test]
fn no_tickets_folder_says_so() {
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        read_tickets(&runner(), empty.path()),
        Err(JobError::Unavailable("No tickets here (.ai/tickets)".into()))
    );
}

#[test]
fn the_job_sends_the_index() {
    let project = project();
    let hub = SourceHub::new(project.path(), || {});
    hub.spawn(SourceKind::Tickets, tickets_job());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut index = None;
    while index.is_none() && std::time::Instant::now() < deadline {
        for event in hub.drain() {
            if let SourceEvent::Tickets(i) = event {
                index = Some(i);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(index.expect("a Tickets event").files, 14);
}

/// `STUDIO_REAL_REPOS=/a:/b cargo test -p studio_sources --test tickets_fixture -- --ignored`
#[test]
#[ignore]
fn real_repos_read_whole() {
    let Ok(repos) = std::env::var("STUDIO_REAL_REPOS") else { return };
    for repo in repos.split(':').filter(|r| !r.is_empty()) {
        let started = std::time::Instant::now();
        let index = match read_tickets(&runner(), Path::new(repo)) {
            Ok(index) => index,
            Err(JobError::Unavailable(why)) => {
                eprintln!("{repo}: {why}");
                continue;
            }
            Err(e) => panic!("{repo}: {e:?}"),
        };
        assert_eq!(index.files, index.tickets.len() + index.bad_files.len());
        assert_eq!(index.status_counts.values().sum::<usize>(), index.tickets.len());
        for t in &index.tickets {
            if let Shipped::Commit(full) = &t.shipped {
                let asked = t.shipped_at.as_deref().unwrap().to_ascii_lowercase();
                assert!(full.len() == 40 && full.starts_with(&asked), "{}: {full}", t.id);
            }
        }
        eprintln!("{repo}: {} files, {:?} in {:.2?}", index.files, index.status_counts, started.elapsed());
    }
}
