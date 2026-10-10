//! Prints what Studio reads about a project, with timings, without opening a window:
//! `cargo run --release -p studio_sources --bin sources_report -- <project>`.

use std::time::Instant;

fn main() {
    let Some(root) = std::env::args().nth(1).map(std::path::PathBuf::from) else {
        eprintln!("usage: sources_report <project folder>");
        std::process::exit(2);
    };
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
