//! The pipeline built from a small polyglot fixture matches `fixtures/pipeline/expected.toml` exactly (every
//! step, every link, the groups and the stats), and the real-repo checks (`STUDIO_REAL_REPOS`, ignored).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use studio_graph::EvidenceTier;
use studio_sources::pipeline::crates;
use studio_sources::{
    build_pipeline, discover_tools, pipeline_job, LinkKind, Packages, Pipeline, SourceEvent, SourceHub, SourceKind,
    StepKind, Tools,
};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pipeline")
}

/// Every file under `root` (skipping `.git`, `node_modules` and `target*` folders), project-relative, sorted.
fn project_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !(e.file_type().is_dir()
                && e.depth() > 0
                && (name == ".git" || name == "node_modules" || name.starts_with("target")))
        })
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.path().strip_prefix(root).ok().map(Path::to_path_buf))
        .collect();
    files.sort();
    files
}

fn manifests(root: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
    files.iter().filter(|f| f.file_name().is_some_and(|n| n == "Cargo.toml")).map(|f| root.join(f)).collect()
}

/// The fixture's inputs, with packages read from the manifests (no cargo).
fn fixture_inputs() -> (PathBuf, Packages, Tools, Vec<PathBuf>) {
    let root = fixture_root();
    let files = project_files(&root);
    let packages = studio_sources::packages::read_manifests(&manifests(&root, &files));
    let tools = discover_tools(&root, &packages, &files);
    (root, packages, tools, files)
}

fn step_ref(p: &Pipeline, i: usize) -> String {
    let s = &p.steps[i];
    format!("{:?} {} ({}:{})", s.kind, s.title, s.path.display(), s.line)
}

fn tier(t: EvidenceTier) -> &'static str {
    t.label()
}

/// One line per step, link and group, and the stats, in the pipeline's own order.
fn describe(p: &Pipeline) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let steps = (0..p.steps.len()).map(|i| step_ref(p, i)).collect();
    let links = p
        .links
        .iter()
        .map(|l| {
            format!(
                "{} -> {} | {:?} {} {:?} n={} | if={} | cross={} | label={}",
                step_ref(p, l.from),
                step_ref(p, l.to),
                l.kind,
                tier(l.provenance.tier),
                l.provenance.basis,
                l.provenance.candidates,
                l.conditional.as_deref().unwrap_or(""),
                l.crossing.as_ref().map(|(a, b)| format!("{a}>{b}")).unwrap_or_default(),
                l.label
            )
        })
        .collect();
    let groups = p
        .groups
        .iter()
        .map(|g| {
            let flows: Vec<String> = g
                .flows
                .iter()
                .map(|&fi| {
                    let f = &p.flows[fi];
                    let sizes: Vec<String> = f.layers.iter().map(|l| l.len().to_string()).collect();
                    format!("{} [{}] links={} truncated={}", f.name, sizes.join(","), f.links.len(), f.truncated)
                })
                .collect();
            format!("{:?} {} expanded={} : {}", g.kind, g.name, g.expanded_by_default, flows.join(" ; "))
        })
        .collect();
    let s = &p.stats;
    let stats = vec![
        format!("routes={}", s.routes),
        format!("route_tags={}", s.route_tags),
        format!("contract_tags={}", s.contract_tags),
        format!("entry_points={}", s.entry_points),
        format!("flows={}", s.flows),
        format!("steps={}", s.steps),
        format!("links_by_tier={:?}", s.links_by_tier),
        format!("crossings={}", s.crossings),
        format!("handler_tags_agree={}", s.handler_tags_agree),
        format!("handler_tags_disagree={}", s.handler_tags_disagree),
    ];
    (steps, links, groups, stats)
}

fn list(value: &toml::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("expected.toml has no `{key}` list"))
        .iter()
        .map(|v| v.as_str().expect("strings only").to_string())
        .collect()
}

/// Exact equality both ways, with the missing and unexpected lines named.
fn assert_same(what: &str, got: &[String], expected: &[String]) {
    let missing: Vec<&String> = expected.iter().filter(|e| !got.contains(e)).collect();
    let extra: Vec<&String> = got.iter().filter(|g| !expected.contains(g)).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{what}: recall misses {missing:#?}\nprecision fails on {extra:#?}"
    );
    assert_eq!(got, expected, "{what}: same lines, different order");
}

#[test]
fn fixture_pipeline_matches_expected() {
    let (root, packages, tools, files) = fixture_inputs();
    let pipeline = build_pipeline(&root, &packages, &tools, &files);
    let text = std::fs::read_to_string(root.join("expected.toml")).unwrap();
    let expected: toml::Value = text.parse().unwrap();
    let (steps, links, groups, stats) = describe(&pipeline);
    if std::env::var("PIPELINE_DUMP").is_ok() {
        let dump = |name: &str, lines: &[String]| {
            println!("{name} = [");
            for l in lines {
                println!("    {:?},", l);
            }
            println!("]");
        };
        dump("steps", &steps);
        dump("links", &links);
        dump("groups", &groups);
        dump("stats", &stats);
    }
    assert_same("steps", &steps, &list(&expected, "steps"));
    assert_same("links", &links, &list(&expected, "links"));
    assert_same("groups", &groups, &list(&expected, "groups"));
    assert_same("stats", &stats, &list(&expected, "stats"));

    // Indices are consistent and every flow's links join its own steps.
    for f in &pipeline.flows {
        let in_flow: Vec<usize> = f.layers.iter().flatten().copied().collect();
        assert_eq!(f.layers[0].last(), Some(&f.entry), "the entry closes layer 0");
        assert_eq!(f.layers[0].iter().filter(|&&s| s == f.entry).count(), 1, "the entry is in layer 0 once");
        // Every usable link leaving the entry (other than a request) is part of its flow.
        for (li, l) in pipeline.links.iter().enumerate() {
            let usable = matches!(l.provenance.tier, EvidenceTier::Proven | EvidenceTier::PossibleSet);
            if l.from == f.entry && usable && l.kind != LinkKind::Requests {
                assert!(f.links.contains(&li), "{} leaves out its entry's link {li}", f.name);
            }
        }
        for &li in &f.links {
            let l = &pipeline.links[li];
            assert!(in_flow.contains(&l.from) && in_flow.contains(&l.to), "{} uses a link outside it", f.name);
        }
        assert!(pipeline.groups[f.group].flows.iter().any(|&fi| pipeline.flows[fi].entry == f.entry));
    }
    // A route with three senders keeps its entry in layer 0, last, after a sender that sorts after routes.
    let start = pipeline.flows.iter().find(|f| f.name == "POST /api/v1/things/{id}/start").unwrap();
    let layer0: Vec<String> = start.layers[0].iter().map(|&i| step_ref(&pipeline, i)).collect();
    assert_eq!(
        layer0,
        [
            "Main tool (tool/src/main.rs:5)",
            "Sender SendDone (game/Scripts/Game/ThingSender.c:6)",
            "Function resend (api/src/requests.rs:11)",
            "Route POST /api/v1/things/{id}/start (api/src/lib.rs:40)",
        ]
    );
    // A program and a function that carry a route tag keep their kind and say they also send.
    let details: Vec<(&str, &str)> = pipeline
        .steps
        .iter()
        .filter(|s| s.detail.contains("also"))
        .map(|s| (s.title.as_str(), s.detail.as_str()))
        .collect();
    assert_eq!(details, [("tool", "main.rs:5 · also a sender"), ("resend", "requests.rs:11 · also a sender")]);
    // Unresolved contract reasons are carried verbatim in the link label, and counted per tag.
    let (_, report) = studio_sources::pipeline::build_with(&root, &packages, &tools, &files, &|| false).unwrap();
    let unresolved: Vec<(String, usize)> = pipeline
        .links
        .iter()
        .filter(|l| l.kind == LinkKind::Declares && l.provenance.tier == EvidenceTier::Unresolved)
        .map(|l| (l.label.clone(), 1))
        .collect();
    assert_eq!(report.unresolved_contracts, unresolved);
    // The same input gives the same pipeline.
    let mut again = build_pipeline(&root, &packages, &tools, &files);
    again.stats.elapsed_ms = pipeline.stats.elapsed_ms;
    assert_eq!(again, pipeline, "deterministic");
}

#[test]
fn fixture_crate_graph_from_packages_matches_the_manifests() {
    let (root, packages, _, _) = fixture_inputs();
    let from_packages = crates::crate_graph(&packages);
    let from_manifests =
        studio_parser::analysis::crate_graph::CrateGraph::from_manifests(&root, &crates::manifests(&packages));
    assert_eq!(crates::differences(&from_packages, &from_manifests, &root), Vec::<String>::new());
    let tool = from_packages.crates.iter().find(|c| c.name == "tool").unwrap();
    assert_eq!(tool.deps.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["server"], "the rename is the name");
    let dep = &packages.find("tool").unwrap().dependencies[0];
    assert_eq!((dep.name.as_str(), dep.rename.as_deref()), ("api", Some("server")));
}

#[test]
fn the_hub_job_sends_the_pipeline() {
    let (root, packages, tools, files) = fixture_inputs();
    let hub = SourceHub::new(&root, || {});
    hub.spawn(SourceKind::Pipeline, pipeline_job(packages.into(), tools.into(), files));
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut got = None;
    while got.is_none() && Instant::now() < deadline {
        for event in hub.drain() {
            if let SourceEvent::Pipeline(p) = event {
                got = Some(p);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let pipeline = got.expect("SourceEvent::Pipeline arrives");
    assert_eq!(pipeline.stats.routes, 12);
}

// ---- Real repositories (STUDIO_REAL_REPOS, colon-separated) ----

fn real_repos() -> Vec<PathBuf> {
    std::env::var("STUDIO_REAL_REPOS")
        .map(|v| v.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default()
}

fn real_inputs(root: &Path) -> (Packages, Tools, Vec<PathBuf>) {
    let files = project_files(root);
    let runner = studio_sources::Runner::new(studio_sources::CancelToken::default());
    let packages = studio_sources::read_packages(&runner, &manifests(root, &files));
    let tools = discover_tools(root, &packages, &files);
    (packages, tools, files)
}

fn find_step(p: &Pipeline, kind: StepKind, title: &str, path_end: &str) -> Option<usize> {
    p.steps.iter().position(|s| s.kind == kind && s.title == title && s.path.to_string_lossy().ends_with(path_end))
}

#[test]
#[ignore]
fn fleet_report_route_is_proven() {
    for root in real_repos() {
        let sender_file = "FleetCommands/TBD_FleetCommandExecution.c";
        if !root.join("mod/tbd-framework/Scripts/Game/TBD/API").join(sender_file).is_file() {
            continue;
        }
        let (packages, tools, files) = real_inputs(&root);
        let started = Instant::now();
        let p = build_pipeline(&root, &packages, &tools, &files);
        let elapsed = started.elapsed();
        let sender = p
            .steps
            .iter()
            .position(|s| s.kind == StepKind::Sender && s.title == "SendReport" && s.path.ends_with(sender_file))
            .expect("Sender SendReport");
        assert_eq!(p.steps[sender].line, 116);
        for (suffix, handler, line) in
            [("result", "finish_fleet_command", 81), ("executing", "start_fleet_command", 66)]
        {
            let title = format!("POST /api/v1/fleet-executor/commands/{{commandId}}/{suffix}");
            let route = p.steps.iter().position(|s| s.kind == StepKind::Route && s.title == title).expect("route");
            let request = p
                .links
                .iter()
                .find(|l| l.from == sender && l.to == route && l.kind == LinkKind::Requests)
                .expect("SendReport requests the route");
            assert_eq!(request.provenance, studio_graph::Provenance::proven(studio_graph::Basis::RouteTable));
            assert_eq!(request.crossing, Some(("Enforce".to_string(), "Rust".to_string())));
            let h = find_step(&p, StepKind::Handler, handler, "handlers/fleet_executor.rs").expect("handler");
            assert_eq!(p.steps[h].line, line);
            let serves = p
                .links
                .iter()
                .find(|l| l.from == route && l.to == h && l.kind == LinkKind::Serves)
                .expect("the route serves the handler");
            assert_eq!(serves.provenance.tier, EvidenceTier::Proven);
        }
        print_numbers(&root, &p, elapsed);
    }
}

fn print_numbers(root: &Path, p: &Pipeline, elapsed: Duration) {
    let s = &p.stats;
    let senders = p.steps.iter().filter(|x| x.kind == StepKind::Sender).count();
    println!("{} pipeline in {elapsed:.2?}", root.display());
    println!(
        "  routes {} | route tags {} (senders {senders}, handler tags {} agree / {} disagree)",
        s.routes, s.route_tags, s.handler_tags_agree, s.handler_tags_disagree
    );
    let mut declares = [0usize; 4];
    for l in p.links.iter().filter(|l| l.kind == LinkKind::Declares) {
        declares[match l.provenance.tier {
            EvidenceTier::Proven => 0,
            EvidenceTier::PossibleSet => 1,
            EvidenceTier::Observed => 2,
            EvidenceTier::Unresolved => 3,
        }] += 1;
    }
    println!("  contract tags {} (Declares by tier {declares:?})", s.contract_tags);
    println!("  entry points {} | flows {} | steps {}", s.entry_points, s.flows, s.steps);
    for g in &p.groups {
        println!("    group {:?} {}: {} flows", g.kind, g.name, g.flows.len());
    }
    println!("  links by tier {:?} | crossings {}", s.links_by_tier, s.crossings);
    for l in p.links.iter().filter(|l| l.kind == LinkKind::Requests && l.crossing.is_none()) {
        println!(
            "    same-language request {} -> {} [{}]",
            step_ref(p, l.from),
            step_ref(p, l.to),
            tier(l.provenance.tier)
        );
    }
}

#[test]
#[ignore]
fn crate_graphs_agree() {
    for root in real_repos() {
        let (packages, _, _) = real_inputs(&root);
        if packages.packages.is_empty() {
            continue;
        }
        let from_packages = crates::crate_graph(&packages);
        let from_manifests =
            studio_parser::analysis::crate_graph::CrateGraph::from_manifests(&root, &crates::manifests(&packages));
        let diff = crates::differences(&from_packages, &from_manifests, &root);
        println!("{}: {} crates, {} differences", root.display(), from_packages.crates.len(), diff.len());
        for d in &diff {
            println!("  {d}");
        }
        assert!(diff.is_empty(), "cargo metadata and the manifests disagree on {}", root.display());
    }
}
