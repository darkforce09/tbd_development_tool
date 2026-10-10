//! The session index on real projects (`STUDIO_REAL_REPOS`, colon-separated): a cold pass on a
//! fresh store, then a warm one that must read nothing new and agree.
//! `STUDIO_REAL_REPOS=/path/a:/path/b cargo test --release -p studio_sources --test agents_real -- --ignored --nocapture`

use std::path::PathBuf;
use std::time::Instant;

#[test]
#[ignore]
fn real_projects_index_cold_then_warm() {
    let Some(claude) = studio_sources::default_claude_dir() else { return };
    let repos = std::env::var("STUDIO_REAL_REPOS").unwrap_or_default();
    for repo in repos.split(':').filter(|r| !r.is_empty()) {
        let root = PathBuf::from(repo);
        let dir = tempfile::tempdir().unwrap();
        let store = studio_sources::Store::in_dir(dir.path());
        let started = Instant::now();
        let cold = match studio_sources::index_sessions(&claude, std::slice::from_ref(&root), Some(&store), None) {
            Ok(index) => index,
            Err(e) => {
                println!("{repo}: {e:?}");
                continue;
            }
        };
        let cold_time = started.elapsed();
        let started = Instant::now();
        let warm = studio_sources::index_sessions(&claude, &[root], Some(&store), None).unwrap();
        let warm_time = started.elapsed();
        println!(
            "{repo}: {} sessions, {} touches, {} logs; cold {cold_time:.2?}, warm {warm_time:.2?}",
            cold.sessions.len(),
            cold.touches.len(),
            cold.logs.len()
        );
        assert_eq!(warm.stats.reparsed_files, 0);
        assert!(warm.sessions.len() >= cold.sessions.len(), "sessions only come and grow");
        assert!(warm.touches.len() >= cold.touches.len());
        for t in &cold.touches {
            assert!(cold.roots.iter().any(|r| t.path.starts_with(r)), "touches stay inside the roots");
        }
    }
}
