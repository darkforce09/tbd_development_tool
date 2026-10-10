//! Prints what Studio reads about a project, with timings, without opening a window:
//! `cargo run --release -p studio_sources --bin sources_report -- <project>`.

use std::time::Instant;

fn main() {
    let Some(root) = std::env::args().nth(1).map(std::path::PathBuf::from) else {
        eprintln!("usage: sources_report <project folder>");
        std::process::exit(2);
    };
    if args_has("--sessions") {
        return sessions_section(&root, args_has("--cold"));
    }
    let started = Instant::now();
    let (graph, stats) = match studio_parser::load_rust_project(&root) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("could not read {}: {e}", root.display());
            std::process::exit(1);
        }
    };
    println!("{} — read in {:.2?}", stats.project_name, started.elapsed());
    println!("  files on disk      {}", stats.file_count);
    println!("  packages           {}", stats.crate_count);
    let tests: u64 = graph.nodes.values().map(|n| n.test_count as u64).sum();
    println!("  tests              {tests}");

    // Every part with tests, from the top: the folders the tests live in, two levels down.
    let mut parts: Vec<(String, u64, usize)> = Vec::new();
    for cluster in &graph.clusters {
        let depth_ok = cluster.depth >= 1 && cluster.depth <= 2;
        if !depth_ok {
            continue;
        }
        let inside: Vec<String> =
            std::iter::once(cluster.id.clone()).chain(graph.descendant_clusters(&cluster.id)).collect();
        let (mut files, mut tests) = (0usize, 0u64);
        for id in &inside {
            if let Some(c) = graph.clusters.iter().find(|c| &c.id == id) {
                files += c.node_ids.len();
                tests += c.node_ids.iter().filter_map(|n| graph.nodes.get(n)).map(|n| n.test_count as u64).sum::<u64>();
            }
        }
        if tests > 0 {
            parts.push((cluster.id.trim_start_matches("dir:").to_string(), tests, files));
        }
    }
    parts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("  tests by part (top 12):");
    for (part, tests, files) in parts.iter().take(12) {
        println!("    {part:<40} {tests:>6} tests in {files} files");
    }
    // Packages, through cargo where it can be asked.
    let started = Instant::now();
    let manifests: Vec<std::path::PathBuf> = graph
        .nodes
        .values()
        .filter(|n| n.title == "Cargo.toml")
        .filter_map(|n| n.file_path.as_ref().map(std::path::PathBuf::from))
        .collect();
    let runner = studio_sources::Runner::new(studio_sources::CancelToken::default());
    let packages = studio_sources::read_packages(&runner, &manifests);
    let by = |source| packages.packages.iter().filter(|p| p.source == source).count();
    println!(
        "  cargo packages     {} ({} through cargo, {} from their manifest) in {:.2?}",
        packages.packages.len(),
        by(studio_sources::PackageSource::Cargo),
        by(studio_sources::PackageSource::Manifest),
        started.elapsed()
    );
    for (workspace, why) in &packages.fallbacks {
        println!("    read from manifests: {} ({why})", workspace.display());
    }
    let count = |kind| packages.packages.iter().flat_map(|p| &p.targets).filter(|t| t.kind == kind).count();
    println!(
        "  targets            {} binaries, {} examples, {} libraries, {} test targets",
        count(studio_sources::TargetKind::Bin),
        count(studio_sources::TargetKind::Example),
        count(studio_sources::TargetKind::Lib),
        count(studio_sources::TargetKind::Test)
    );
    let edges: usize = packages.packages.iter().map(|p| p.depends_on.len()).sum();
    println!("  package links      {edges} (path and workspace dependencies)");

    // Tools.
    let started = Instant::now();
    let files: Vec<std::path::PathBuf> =
        graph.nodes.values().filter_map(|n| n.file_path.as_ref().map(std::path::PathBuf::from)).collect();
    let tools = studio_sources::discover_tools(&root.canonicalize().unwrap_or(root.clone()), &packages, &files);
    println!("  tools              found in {:.2?}", started.elapsed());
    use studio_sources::ToolKind::*;
    for kind in [Binary, Example, CargoAlias, Subcommand, NpmScript, MakeTarget, JustRecipe, Workflow, WorkflowJob] {
        let n = tools.count(kind);
        if n > 0 {
            println!("    {:<16} {n}", kind.label());
        }
    }
    for tool in tools.tools.iter().filter(|t| t.kind == Binary && !t.children.is_empty()) {
        let nested = tool.walk().len() - 1;
        println!(
            "    {} has {} subcommands ({nested} with nesting): {}",
            tool.invocation,
            tool.children.len(),
            tool.children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(" ")
        );
    }
    for package in packages
        .packages
        .iter()
        .filter(|p| p.targets.iter().filter(|t| t.kind == studio_sources::TargetKind::Bin).count() > 1)
    {
        let bins: Vec<&str> = package
            .targets
            .iter()
            .filter(|t| t.kind == studio_sources::TargetKind::Bin)
            .map(|t| t.name.as_str())
            .collect();
        println!("    {} binaries of {}: {}", bins.len(), package.name, bins.join(" "));
    }
    for workflow in tools.tools.iter().filter(|t| t.kind == Workflow) {
        println!(
            "    workflow {} ({} jobs) {}",
            workflow.name,
            workflow.children.len(),
            workflow.description.as_deref().unwrap_or("")
        );
    }

    // The disk as git sees it, and settings.
    let canonical = root.canonicalize().unwrap_or(root.clone());
    let started = Instant::now();
    match studio_sources::read_git_disk(&runner, &canonical) {
        Ok(disk) => {
            println!("  git                read in {:.2?}", started.elapsed());
            println!("    tracked files    {}", disk.tracked);
            println!("    in Git LFS       {}", disk.lfs);
            println!("    ignored entries  {}", disk.ignored.len());
            let c = disk.changes;
            println!("    changes          {} ({} modified, {} untracked)", c.total(), c.modified, c.untracked);
            if let Some(target) = disk.ignored.iter().find(|e| e.path == "target/") {
                if let Some(rule) = &target.rule {
                    println!("    target/ ignored by {}:{} {}", rule.source.display(), rule.line, rule.pattern);
                }
            }
        }
        Err(e) => println!("  git                {e:?}"),
    }
    git_history_section(&runner, &canonical);
    let json: Vec<std::path::PathBuf> =
        files.iter().filter(|f| f.extension().is_some_and(|e| e == "json")).cloned().collect();
    let started = Instant::now();
    let settings = studio_sources::read_settings(&canonical, &manifests, &json);
    println!("  settings           read in {:.2?}", started.elapsed());
    if let Some(ignore) = &settings.gitignore {
        println!("    .gitignore       {} patterns in {} groups", ignore.patterns(), ignore.groups.len());
    }
    for sheet in &settings.sheets {
        println!("    {:<16} {}", sheet.title, sheet.summary);
    }
    let mut by_folder: std::collections::BTreeMap<String, usize> = Default::default();
    for s in &settings.schemas {
        let folder = s
            .file
            .parent()
            .and_then(|p| p.strip_prefix(&canonical).ok())
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        *by_folder.entry(folder).or_default() += 1;
    }
    let (most, n) = by_folder.iter().max_by_key(|(_, n)| **n).map(|(f, n)| (f.clone(), *n)).unwrap_or_default();
    println!("    JSON Schemas     {} ({n} in {most})", settings.schemas.len());
    if args_has("--du") {
        for folder in ["target", "target-glibc236"] {
            let started = Instant::now();
            let bytes = studio_sources::du_bytes(&canonical.join(folder));
            println!("    du {folder:<14} {bytes} bytes in {:.2?}", started.elapsed());
        }
    }

    // Tickets (`.ai/tickets`), with every `shipped_at` asked of git once.
    let started = Instant::now();
    match studio_sources::read_tickets(&runner, &canonical) {
        Ok(index) => {
            println!("  tickets            read in {:.2?}", started.elapsed());
            println!("    files            {} ({} bad)", index.files, index.bad_files.len());
            for (file, why) in index.bad_files.iter().take(5) {
                println!("      {file}: {why}");
            }
            let counts: Vec<String> = index.status_counts.iter().map(|(s, n)| format!("{s} {n}")).collect();
            println!("    status           {}", counts.join(" / "));
            let shipped = |want: fn(&studio_sources::Shipped) -> bool| {
                index.tickets.iter().filter(|t| t.shipped_at.is_some() && want(&t.shipped)).count()
            };
            println!(
                "    shipped_at       {} confirmed, {} not in repo, {} not checked",
                shipped(|s| matches!(s, studio_sources::Shipped::Commit(_))),
                shipped(|s| *s == studio_sources::Shipped::NotInRepo),
                shipped(|s| *s == studio_sources::Shipped::None)
            );
            let next: Vec<&str> = studio_sources::next_tickets(&index, 5).iter().map(|t| t.id.as_str()).collect();
            println!("    next             {}", next.join(" "));
        }
        Err(e) => println!("  tickets            {e:?}"),
    }

    // `--tests <path prefix>`: tests per file under that folder, to check against other counts.
    let args: Vec<String> = std::env::args().collect();
    if let Some(prefix) = args.iter().position(|a| a == "--tests").and_then(|i| args.get(i + 1)) {
        let prefix = root.join(prefix);
        let mut rows: Vec<(String, u32)> = graph
            .nodes
            .values()
            .filter(|n| n.test_count > 0)
            .filter_map(|n| Some((n.file_path.clone()?, n.test_count)))
            .filter(|(p, _)| std::path::Path::new(p).starts_with(&prefix))
            .collect();
        rows.sort();
        for (path, tests) in rows {
            println!("{tests:>7} {path}");
        }
    }
    let about: Vec<(&str, &str)> = graph
        .clusters
        .iter()
        .filter(|c| c.depth <= 2)
        .filter_map(|c| Some((c.id.trim_start_matches("dir:"), c.about.as_deref()?)))
        .take(8)
        .collect();
    println!("  READMEs (first 8):");
    for (part, sentence) in about {
        println!("    {part:<40} {sentence}");
    }
}

fn args_has(flag: &str) -> bool {
    std::env::args().any(|a| a == flag)
}

/// Branches, history, co-authors, worktrees, the per-folder last commit and one commit's files.
fn git_history_section(runner: &studio_sources::Runner, root: &std::path::Path) {
    let started = Instant::now();
    let history = match studio_sources::read_history(runner, root) {
        Ok(history) => history,
        Err(e) => return println!("  git history        {e:?}"),
    };
    println!("  git history        read in {:.2?}", started.elapsed());
    println!("    local branches   {}", history.branches.len());
    println!("    commits on HEAD  {} ({} read)", history.commit_count, history.commits.len());
    println!("    co-authored      {} commits", history.co_authored_commits);
    let mut names: Vec<(&String, &usize)> = history.co_authors.iter().collect();
    names.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (name, n) in names.iter().take(5) {
        println!("      {name:<30} {n}");
    }
    for wt in &history.worktrees {
        let (added, deleted) =
            wt.files.iter().fold((0, 0), |(a, d), f| (a + f.added.unwrap_or(0), d + f.deleted.unwrap_or(0)));
        println!(
            "    worktree         {} [{}]{} {} changes: {} files +{added} -{deleted}, {} untracked",
            wt.path.display(),
            wt.branch.as_deref().unwrap_or("detached"),
            if wt.is_main { " (main checkout)" } else { "" },
            wt.changes.total(),
            wt.files.len(),
            wt.changes.untracked
        );
    }
    let started = Instant::now();
    match studio_sources::read_part_commits(runner, root) {
        Ok(parts) => println!(
            "  part commits       {} folders from {} commits in {:.2?} (paths as recorded, renames unpaired)",
            parts.by_folder.len(),
            parts.commits_scanned,
            started.elapsed()
        ),
        Err(e) => println!("  part commits       {e:?}"),
    }
    let Some(head) = history.commits.first() else { return };
    let dir = std::env::temp_dir().join(format!("studio-sources-report-{}", std::process::id()));
    let store = studio_sources::Store::in_dir(&dir);
    let started = Instant::now();
    let cold = studio_sources::commit_files_cached(runner, root, &store, &head.id);
    let cold_time = started.elapsed();
    let started = Instant::now();
    let warm = studio_sources::commit_files_cached(runner, root, &store, &head.id);
    let warm_time = started.elapsed();
    let _ = std::fs::remove_dir_all(&dir);
    match (cold, warm) {
        (Ok(cold), Ok(warm)) => println!(
            "  commit files       HEAD {}: {} files, cold {cold_time:.2?}, warm {warm_time:.2?}{}",
            head.short,
            cold.len(),
            if cold == warm { "" } else { " (warm differs!)" }
        ),
        (cold, warm) => println!("  commit files       cold {cold:?}, warm {warm:?}"),
    }
}

/// `--sessions [--cold]`: the agent session index for the project and its linked worktrees,
/// indexed twice on one store (with `--cold`, a fresh temporary store: cold, then warm).
fn sessions_section(root: &std::path::Path, cold: bool) {
    let root = root.canonicalize().unwrap_or(root.to_path_buf());
    let runner = studio_sources::Runner::new(studio_sources::CancelToken::default());
    let mut roots = vec![root.clone()];
    if let Ok(out) =
        studio_sources::Command::git(&root, ["worktree", "list", "--porcelain"]).and_then(|c| runner.run(&c))
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                let path = std::path::Path::new(path).canonicalize().unwrap_or(path.into());
                if !roots.contains(&path) {
                    roots.push(path);
                }
            }
        }
    }
    let Some(claude) = studio_sources::default_claude_dir() else {
        return println!("  sessions           no home folder");
    };
    println!("  sessions           {}", claude.display());
    for (i, r) in roots.iter().enumerate() {
        println!("    root {i}           {}", r.display());
    }
    let temp = std::env::temp_dir().join(format!("studio-sessions-report-{}", std::process::id()));
    let store = if cold { studio_sources::Store::in_dir(&temp) } else { studio_sources::Store::for_project(&root) };
    let mut runs = Vec::new();
    for _ in 0..2 {
        let started = Instant::now();
        let result = studio_sources::index_sessions(&claude, &roots, Some(&store), None);
        runs.push((started.elapsed(), result));
    }
    let _ = std::fs::remove_dir_all(&temp);
    let first_label = if cold { "cold (fresh store)" } else { "first (project store)" };
    let read = |r: &Result<studio_sources::AgentIndex, _>| {
        r.as_ref().map_or(String::new(), |i| {
            let s = &i.stats;
            format!(
                " ({} new, {} resumed, {} reparsed files; {} MB scanned for later cwds)",
                s.new_files,
                s.resumed_files,
                s.reparsed_files,
                s.later_cwd_bytes / 1_000_000
            )
        })
    };
    println!("    {first_label:<20} {:.2?}{}", runs[0].0, read(&runs[0].1));
    println!("    warm (same store)    {:.2?}{}", runs[1].0, read(&runs[1].1));
    let index = match runs.pop().map(|r| r.1) {
        Some(Ok(index)) => index,
        Some(Err(e)) => return println!("    {e:?}"),
        None => return,
    };
    if runs[0].1.as_ref().ok().map(|i| (&i.sessions, &i.touches)) != Some((&index.sessions, &index.touches)) {
        println!("    WARNING: the warm pass differs from the first");
    }
    let s = &index.stats;
    let mapped: Vec<_> = index.folders.iter().filter(|f| f.mapped).collect();
    println!("    folders          {} scanned ({} logs), {} mapped", index.folders.len(), s.scanned_logs, mapped.len());
    for f in &mapped {
        let name = f.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        println!("      {name} ({} logs, {} MB)", f.logs, f.bytes / 1_000_000);
    }
    println!("    main logs        {} ({} MB)", s.main_logs, s.main_bytes / 1_000_000);
    println!("    subagent logs    {} ({} MB)", s.subagent_logs, s.subagent_bytes / 1_000_000);
    println!(
        "    lines            {} ({} parsed, {} bad, {} duplicate)",
        s.lines, s.parsed_lines, s.bad_lines, s.duplicate_lines
    );
    println!(
        "    sessions         {} ({} entered after their first cwd, {} main logs)",
        index.sessions.len(),
        s.later_cwd_sessions,
        s.later_cwd_logs
    );
    use studio_sources::TouchKind;
    for kind in [TouchKind::Read, TouchKind::Edit, TouchKind::Write, TouchKind::BashEdit] {
        let main = index.touches.iter().filter(|t| t.kind == kind && !t.subagent).count();
        let sub = index.touches.iter().filter(|t| t.kind == kind && t.subagent).count();
        println!("    {:<16} {main} main, {sub} subagent", format!("touches {}", kind.label()));
    }
    let with_hunks = index.touches.iter().filter(|t| !t.hunks.is_empty()).count();
    let in_task = index.touches.iter().filter(|t| t.task.is_some()).count();
    println!("    touches          {} ({with_hunks} with line numbers, {in_task} in a task)", index.touches.len());
    let gone_files: std::collections::BTreeSet<_> =
        index.touches.iter().map(|t| &t.path).filter(|p| !p.is_file()).collect();
    let in_root_gone = index.touches.iter().filter(|t| gone_files.contains(&t.path)).count();
    println!("    in repo, gone    {in_root_gone} touches on {} files no longer on disk", gone_files.len());
    println!(
        "    out of repo      {} (gone: {} — e.g. removed worktrees; still on disk: {})",
        s.out_of_repo_touches, s.out_of_repo_gone, s.out_of_repo_existing
    );
    for (folder, n) in &s.out_of_repo_top {
        println!("      {n:>6}  {folder}");
    }
    let plans: Vec<_> = index.sessions.iter().filter_map(|s| s.plan.as_ref()).collect();
    let on_disk = plans.iter().filter(|p| p.path.as_ref().is_some_and(|p| p.is_file())).count();
    let by_file_only = plans.iter().filter(|p| p.len == 0).count();
    println!(
        "    plans            {} ({on_disk} with file, {} without; {by_file_only} found by slug only)",
        plans.len(),
        plans.len() - on_disk
    );
    let tasks: usize = index.sessions.iter().map(|s| s.tasks.len()).sum();
    let states: usize = index.sessions.iter().flat_map(|s| &s.tasks).map(|t| t.states.len()).sum();
    println!("    tasks            {tasks} ({states} state changes)");
    let mut unknown: Vec<_> = s.unknown_types.iter().collect();
    unknown.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let top: Vec<String> = unknown.iter().take(10).map(|(t, n)| format!("{t} {n}")).collect();
    println!("    unknown types    {}", top.join(", "));
    println!("    index time       {} ms (warm pass, as the index measures itself)", s.elapsed_ms);
}
