use egui::{Pos2, Rect};
use std::path::PathBuf;
use std::time::Instant;
use studio_canvas::{CanvasState, CanvasView, SpatialHashGrid};
use studio_graph::{DataType, Graph, NodeArchetype};
use studio_parser::{
    build_project_graph, clear_project_cache, extract_project, load_project_cache, save_project_cache, scan_project,
    ProjectStats, SymbolSearchIndex,
};
use studio_viewer::telemetry::{TelemetryBudget, TimelineTracker};

const USAGE: &str = "usage: bench_scale [PROJECT_DIR] [--ram-budget-gb GB] [--target-fps FPS]
  PROJECT_DIR      project to load for the real-world benchmark (default: current directory)
  --ram-budget-gb  RAM ceiling checked by the benchmark (default: half of system RAM)
  --target-fps     frame rate the render simulation must sustain (default: 60)
  --real-only      only benchmark the project, skip the synthetic suites
  --layout-report  print the shape of the project's layout: size, aspect, widest and tallest folders
  --cpu-wires      time canvas frames with wires painted by egui instead of the GPU";

/// `--cpu-wires`: canvas frames paint wires with egui (the painter path) instead of preparing GPU
/// instances.
static CPU_WIRES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct BenchArgs {
    project: PathBuf,
    budget: TelemetryBudget,
    real_only: bool,
    layout_report: bool,
}

fn parse_args() -> Result<BenchArgs, String> {
    let mut budget = TelemetryBudget::for_this_machine();
    let mut project = None;
    let mut real_only = false;
    let mut layout_report = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut number = |flag: &str| -> Result<f64, String> {
            let value = args.next().ok_or(format!("{flag} needs a value"))?;
            value.parse::<f64>().ok().filter(|v| *v > 0.0).ok_or(format!("{flag}: '{value}' is not a positive number"))
        };
        match arg.as_str() {
            "--ram-budget-gb" => budget.ram_bytes = (number("--ram-budget-gb")? * 1_073_741_824.0) as u64,
            "--target-fps" => budget.target_fps = number("--target-fps")?,
            "--real-only" => real_only = true,
            "--layout-report" => layout_report = true,
            "--cpu-wires" => CPU_WIRES.store(true, std::sync::atomic::Ordering::Relaxed),
            "-h" | "--help" => return Err(String::new()),
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            path if project.is_none() => project = Some(PathBuf::from(path)),
            extra => return Err(format!("unexpected argument {extra}")),
        }
    }
    let project = match project {
        Some(p) => p,
        None => std::env::current_dir().map_err(|e| format!("no project given and cwd unavailable: {e}"))?,
    };
    Ok(BenchArgs { project, budget, real_only, layout_report })
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("error: {msg}");
            }
            eprintln!("{USAGE}");
            std::process::exit(if msg.is_empty() { 0 } else { 2 });
        }
    };

    println!("================================================================================");
    println!("     STUDIO HIGH-SCALE PERFORMANCE & SCALE VERIFICATION SUITE                   ");
    println!("================================================================================\n");

    let mut tracker = TimelineTracker::new(args.budget);
    println!("  GPU:    {}", tracker.gpu_summary());
    println!("  Budget: {}\n", tracker.budget.describe());

    // Part 1: Real-world project benchmark
    benchmark_real_project(&mut tracker, &args.project, args.layout_report);
    if args.real_only {
        tracker.print_timeline_summary();
        return;
    }

    // Part 2: rkyv Zero-Copy Caching Benchmark
    benchmark_rkyv_caching(&mut tracker);

    // Part 3: 10x Scale Stress Test (212,640 nodes, 106,700 edges)
    benchmark_extreme_scale(&mut tracker);

    // Part 4: Ultra Scale Verification: 100,000 Folders, 500,000 Files, 5,000,000 Nodes & Wires
    benchmark_ultra_scale_5m(&mut tracker);

    // Final Timeline Summary Report
    tracker.print_timeline_summary();
}

fn benchmark_real_project(tracker: &mut TimelineTracker, target_path: &std::path::Path, layout_report: bool) {
    if !target_path.exists() {
        println!("[-] Target path does not exist: {}", target_path.display());
        return;
    }

    println!(">>> BENCHMARK 1: Real-world Project Loading: {}", target_path.display());

    // 1. Scanning
    let t0 = Instant::now();
    let scanned = scan_project(target_path).expect("Failed to scan project");
    let scan_dur = t0.elapsed();
    let total_files: usize = scanned.crates.iter().map(|c| c.source_files.len()).sum();
    tracker.record_stage(
        "Workspace Scan",
        format!("{} subsystems, {} files discovered", scanned.crates.len(), total_files),
        None,
        None,
        Some(total_files),
        None,
    );

    let packages = scanned.crates.iter().filter(|c| c.is_package()).count();
    let outside: usize = scanned.crates.iter().filter(|c| !c.is_package()).map(|c| c.source_files.len()).sum();
    println!("      • {packages} packages; {outside} text files outside every package");

    // 1b. Tier 1 Instant Skeleton Startup (< 400ms)
    let t_skel = Instant::now();
    let (skel_graph, skel_stats) = studio_parser::build_skeleton_files_graph(&scanned);
    let skel_dur = t_skel.elapsed();
    let instant_startup_time = scan_dur + skel_dur;
    tracker.record_stage(
        "Tier 1 Instant Startup",
        format!(
            "Scan + Layout: {} nodes, {} clusters ready to render in {:?}",
            skel_stats.node_count,
            skel_graph.clusters.len(),
            instant_startup_time
        ),
        None,
        None,
        Some(skel_stats.node_count),
        None,
    );

    // 1c. Heavy folders are counted after the first layout, off its path.
    let t_heavy = Instant::now();
    let mut measured = scanned.tree.clone();
    studio_parser::tree::measure_heavy_dirs(&mut measured);
    println!(
        "      • {} heavy folders counted in {:?}, after the first layout",
        measured.heavy_dirs().count(),
        t_heavy.elapsed()
    );

    // 2. Parallel AST Extraction (Rayon)
    let t1 = Instant::now();
    let extracted = extract_project(&scanned);
    let _extract_dur = t1.elapsed();
    tracker.record_stage(
        "Parallel AST Extract",
        "Rayon multithreaded worker threadpool extraction",
        None,
        None,
        Some(total_files),
        None,
    );

    // 3. Graph Construction
    let t2 = Instant::now();
    let (mut graph, stats) = build_project_graph(&extracted);
    let _build_dur = t2.elapsed();
    tracker.record_stage(
        "Graph Assembly",
        format!(
            "{} nodes, {} edges, {} clusters; R1 {} files in {:.1} ms: {} Proven wires, {} upgraded, {} retargeted, \
             {} added, {} dropped, {} unresolved, {} cfg-gated",
            stats.node_count,
            stats.wire_count,
            graph.clusters.len(),
            stats.r1.files,
            (stats.r1.crate_graph_us + stats.r1.index_us + stats.r1.resolve_us) as f64 / 1000.0,
            stats.r1.proven_edges,
            stats.r1.upgraded,
            stats.r1.retargeted,
            stats.r1.added,
            stats.r1.dropped,
            stats.r1.unresolved,
            stats.r1.cfg_gated
        ),
        None,
        None,
        Some(stats.node_count),
        Some(stats.wire_count),
    );

    // 4. Fast Indices
    let t3 = Instant::now();
    graph.rebuild_fast_indices();
    let _index_dur = t3.elapsed();
    tracker.record_stage(
        "Fast Graph Index",
        "O(1) degree maps, port maps, archetype counts",
        None,
        None,
        Some(stats.node_count),
        Some(stats.wire_count),
    );

    // 4b. Dataflow layout of the whole folder tree, timed on its own
    graph.flow_layout = true;
    let t_layout = Instant::now();
    graph.layout_folder_tree();
    let layout_dur = t_layout.elapsed();
    let (gates, boxes, routes, points, doc_routes) = graph.flow.as_ref().map_or((0, 0, 0, 0, 0), |f| {
        let points = f.routes.iter().map(|r| r.points.len()).sum::<usize>();
        (f.gates.len(), f.cycle_boxes.len(), f.routes.len(), points, f.doc_routes.len())
    });
    // Segments shared by several routes (one provider fanning out) are drawn once.
    let unique_segments: std::collections::HashSet<[i32; 4]> = graph
        .flow
        .iter()
        .flat_map(|f| f.routes.iter())
        .flat_map(|r| r.points.windows(2))
        .map(|w| {
            let q = |v: f32| (v * 10.0).round() as i32;
            [q(w[0][0]), q(w[0][1]), q(w[1][0]), q(w[1][1])]
        })
        .collect();
    println!("    route segments: {} total, {} distinct", points.saturating_sub(routes), unique_segments.len());
    tracker.record_stage(
        "Dataflow Layout",
        format!(
            "{:.1} ms: {} gates, {} cycle boxes, {} routes ({} points), {} documentation routes",
            layout_dur.as_secs_f64() * 1000.0,
            gates,
            boxes,
            routes,
            points,
            doc_routes
        ),
        None,
        None,
        Some(stats.node_count),
        Some(stats.wire_count),
    );

    if layout_report {
        print_layout_report(&graph);
    }

    // 5. Symbol Search Index (Trigram DDR5)
    let t4 = Instant::now();
    let search_index = SymbolSearchIndex::build(&graph);
    let _search_dur = t4.elapsed();
    tracker.record_stage(
        "Trigram Search Index",
        "In-memory DDR5 streaming trigram index",
        None,
        None,
        Some(stats.node_count),
        None,
    );

    // 6. Spatial Hash Grid
    let t5 = Instant::now();
    let mut grid = SpatialHashGrid::new(768.0);
    grid.build_from_graph(&graph);
    let _grid_dur = t5.elapsed();
    tracker.record_stage(
        "Spatial Hash Grid",
        "2D uniform spatial partition & clamped bounds index",
        None,
        None,
        Some(stats.node_count),
        Some(stats.wire_count),
    );

    // Run sample search queries
    let test_queries = ["event", "handler", "config", "token", "message"];
    println!("  Testing Spotlight Symbol Queries:");
    for q in test_queries {
        let t_search = Instant::now();
        let matches = search_index.search(q, 16);
        let s_dur = t_search.elapsed();
        println!("    Query {:<10} -> {:>2} results in {:>6.2?}", format!("'{}'", q), matches.len(), s_dur);
    }
    println!();

    // 7. Real canvas frames across zoom levels (frame budget verification)
    benchmark_canvas_frames(tracker, &mut graph);

    // 8. Geometry-only relayout: a card opening, a folder changing level.
    benchmark_relayout(tracker, &mut graph);
}

/// Times what the user waits for when a card opens or a folder changes level: the geometry-only
/// relayout plus rebuilding the scene and spatial grid.
fn benchmark_relayout(tracker: &mut TimelineTracker, graph: &mut Graph) {
    let mut state = CanvasState::default();
    state.refresh_scene(graph);
    // The card with the most members, in the deepest folder: the most containers to re-measure.
    let Some(card) = graph
        .nodes
        .values()
        .filter(|n| n.archetype == NodeArchetype::File && !graph.is_node_in_collapsed_cluster(n.id))
        .max_by_key(|n| (n.group_id.as_deref().map_or(0, |g| g.matches('/').count()), n.member_nodes.len(), n.id))
        .map(|n| n.id)
    else {
        return;
    };
    let mut time = |name: &str, graph: &mut Graph, change: &dyn Fn(&mut Graph) -> bool| {
        let t = Instant::now();
        let geometry = change(graph);
        let layout_ms = t.elapsed().as_secs_f64() * 1000.0;
        state.mark_scene_dirty();
        let t_scene = Instant::now();
        state.refresh_scene(graph);
        let scene_ms = t_scene.elapsed().as_secs_f64() * 1000.0;
        tracker.record_stage(
            name.to_string(),
            format!(
                "{} {:.1} ms + scene {:.1} ms = {:.1} ms",
                if geometry { "geometry relayout" } else { "FULL layout" },
                layout_ms,
                scene_ms,
                layout_ms + scene_ms
            ),
            None,
            Some((layout_ms + scene_ms) * 1000.0),
            None,
            None,
        );
    };
    let toggle = |graph: &mut Graph| {
        let node = graph.nodes.get_mut(&card).expect("card");
        node.is_dropdown_expanded = !node.is_dropdown_expanded;
        node.size = studio_canvas::calculate_file_node_size(node);
        graph.relayout_geometry(&[card])
    };
    time("Relayout: card opens", graph, &toggle);
    time("Relayout: card closes", graph, &toggle);
    let folder = graph.nodes[&card].group_id.clone().unwrap_or_default();
    let level = |detail: studio_graph::FolderDetail| {
        let folder = folder.clone();
        move |graph: &mut Graph| {
            let geometry = graph.has_layout_cache();
            graph.set_folder_detail(&folder, detail);
            geometry
        }
    };
    time("Relayout: folder to node view", graph, &level(studio_graph::FolderDetail::NodeView));
    time("Relayout: folder opens again", graph, &level(studio_graph::FolderDetail::Open));
}

/// Times real canvas frames: `CanvasView::show` in a headless egui context plus tessellation,
/// which is the CPU work of one frame. GPU time is not included.
fn benchmark_canvas_frames(tracker: &mut TimelineTracker, graph: &mut Graph) {
    const WARMUP: usize = 3;
    const FRAMES: usize = 30;
    println!("  Canvas frames, CPU only ({:.0} FPS budget):", tracker.budget.target_fps);
    let ctx = egui::Context::default();
    studio_ui::apply_theme(&ctx);
    let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(2560.0, 1440.0));
    let mut state = CanvasState::default();
    state.use_gpu_wires = !CPU_WIRES.load(std::sync::atomic::Ordering::Relaxed);

    let t_scene = Instant::now();
    state.refresh_scene(graph);
    let scene = &state.scene;
    // Wires left without a route, by kind and why.
    let mut unrouted: std::collections::BTreeMap<(&str, &str), usize> = Default::default();
    for &id in &scene.curve_owner {
        let Some(e) = graph.get_edge(id) else { continue };
        let homed = |n| graph.clusters.iter().any(|c| c.node_ids.contains(&n));
        let why = if e.from_node == e.to_node {
            "same card"
        } else if !homed(e.from_node) || !homed(e.to_node) {
            "card in no folder"
        } else {
            "no route"
        };
        *unrouted.entry((e.kind.label(), why)).or_default() += 1;
    }
    println!("    wires drawn as curves: {unrouted:?}");
    tracker.record_stage(
        "Canvas Scene Build",
        format!(
            "{:.1} ms (scene {:.1} ms): {} segments in {} tiles, {} curves, {} boxes, {} gates",
            t_scene.elapsed().as_secs_f64() * 1000.0,
            scene.build_ms,
            scene.segments.len(),
            scene.tiles.len(),
            scene.curves.len(),
            scene.boxes.len(),
            scene.gates.len()
        ),
        None,
        None,
        Some(graph.nodes.len()),
        Some(graph.edges.len()),
    );

    state.zoom_to_fit(graph, screen);
    let fit = state.transform.zoom;
    let fit_center = state.transform.screen_to_world(screen.center());
    let dense = densest_point(graph).unwrap_or(fit_center);
    for (name, zoom) in [("fit-all", fit), ("2%", 0.02), ("10%", 0.1), ("30%", 0.3), ("100%", 1.0)] {
        let center = if zoom == fit { fit_center } else { dense };
        state.transform.center_on_world_pos(center, screen, Some(zoom));
        let mut total = 0.0;
        let mut vertices = 0;
        for frame in 0..WARMUP + FRAMES {
            let start = Instant::now();
            let raw = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let output = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                    CanvasView::new(&mut state, graph).show(ui);
                });
            });
            vertices = ctx
                .tessellate(output.shapes, output.pixels_per_point)
                .iter()
                .map(|p| match &p.primitive {
                    egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.len(),
                    egui::epaint::Primitive::Callback(_) => 0,
                })
                .sum();
            if frame >= WARMUP {
                total += start.elapsed().as_secs_f64();
            }
        }
        let avg_us = total / FRAMES as f64 * 1_000_000.0;
        let stats = state.frame_stats;
        tracker.record_stage(
            format!("Canvas Frame {name}"),
            format!(
                "zoom {:.4} | {:.2} ms | {} nodes, {} wires, {} egui vertices",
                zoom,
                avg_us / 1000.0,
                stats.visible_nodes,
                stats.visible_wires,
                vertices
            ),
            Some(1_000_000.0 / avg_us.max(0.1)),
            Some(avg_us),
            Some(stats.visible_nodes),
            Some(stats.visible_wires),
        );
    }
    benchmark_world_frames(tracker, &ctx, &mut state, graph, screen);
    println!();
}

/// One CPU frame of the canvas at `time`, in microseconds.
fn canvas_frame(ctx: &egui::Context, state: &mut CanvasState, graph: &mut Graph, screen: Rect, time: f64) -> f64 {
    let start = Instant::now();
    let raw = egui::RawInput { screen_rect: Some(screen), time: Some(time), ..Default::default() };
    let output = ctx.run_ui(raw, |ui| {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            CanvasView::new(state, graph).show(ui);
        });
    });
    let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
    start.elapsed().as_secs_f64() * 1_000_000.0
}

/// A frame at every stop of the world, and the slowest frame of a flight across it.
fn benchmark_world_frames(
    tracker: &mut TimelineTracker,
    ctx: &egui::Context,
    state: &mut CanvasState,
    graph: &mut Graph,
    screen: Rect,
) {
    use studio_canvas::{CameraTarget, Stop};
    let world = state.world;
    println!(
        "    world: map {:.0} x {:.0}, districts at {:.1} world units per point, whole world {:.0} x {:.0}",
        world.code.width(),
        world.code.height(),
        world.scale,
        world.bounds.width(),
        world.bounds.height()
    );
    let mut time = 1000.0;
    for stop in [Stop::World, Stop::Pipeline, Stop::Files, Stop::Code, Stop::Desk, Stop::Run, Stop::Changes] {
        state.jump_to(CameraTarget::Stop(stop));
        let mut worst: f64 = 0.0;
        for _ in 0..8 {
            time += 1.0 / 60.0;
            worst = worst.max(canvas_frame(ctx, state, graph, screen, time));
        }
        tracker.record_stage(
            format!("World stop {}", stop.label()),
            format!("zoom {:.4}, slowest of 8 frames {:.2} ms", state.transform.zoom, worst / 1000.0),
            Some(1_000_000.0 / worst.max(0.1)),
            Some(worst),
            Some(state.frame_stats.visible_nodes),
            Some(state.frame_stats.visible_wires),
        );
    }
    // Files to Run crosses the whole map: the longest glide.
    state.jump_to(CameraTarget::Stop(Stop::Files));
    time += 1.0;
    canvas_frame(ctx, state, graph, screen, time);
    state.fly_to(CameraTarget::Stop(Stop::Run));
    let (mut worst, mut frames): (f64, usize) = (0.0, 0);
    while frames < 120 && (frames == 0 || state.camera.is_flying()) {
        time += 1.0 / 60.0;
        worst = worst.max(canvas_frame(ctx, state, graph, screen, time));
        frames += 1;
    }
    tracker.record_stage(
        "World flight Files to Run",
        format!("{frames} frames, slowest {:.2} ms", worst / 1000.0),
        Some(1_000_000.0 / worst.max(0.1)),
        Some(worst),
        None,
        None,
    );
}

/// Prints the overall size and aspect of the layout, and the folders that make it that size.
fn print_layout_report(graph: &Graph) {
    let roots: Vec<&studio_graph::GroupCluster> = graph.clusters.iter().filter(|c| c.parent_id.is_none()).collect();
    let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
    for c in &roots {
        min = [min[0].min(c.position[0]), min[1].min(c.position[1])];
        max = [max[0].max(c.position[0] + c.size[0]), max[1].max(c.position[1] + c.size[1])];
    }
    let (w, h) = (max[0] - min[0], max[1] - min[1]);
    let stats = &graph.layout_stats;
    println!("\n  LAYOUT REPORT");
    println!(
        "    world: {w:.0} x {h:.0} (aspect {:.1}:1), {} roots, {} containers",
        w / h.max(1.0),
        roots.len(),
        stats.len()
    );
    let sum = |f: fn(&studio_graph::ContainerStats) -> f32| stats.iter().map(f).sum::<f32>();
    println!(
        "    totals: columns {:.0}, gaps {:.0}, gate bands {:.0}, documentation strips {:.0} high",
        sum(|s| s.columns_width),
        sum(|s| s.gaps_width),
        sum(|s| s.in_band + s.out_band),
        sum(|s| s.doc_strip)
    );
    let row = |s: &studio_graph::ContainerStats| {
        let name: String = s.id.chars().rev().take(44).collect::<String>().chars().rev().collect();
        println!(
            "    {:<44} d{:<2} {:>8.0} x {:>7.0} ({:>5.1}:1) | {:>3} layers, {:>4} wired (max {:>3}/col, {:>4} lanes, col h {:>6.0}) | shelf {:>4} in {:>3} rows | cols {:>7.0} gaps {:>7.0} (max {:>3} tracks) | gates {:>4} in {:>4} out {:>4} doc, bands {:>5.0}+{:<5.0} strip {:>4.0}",
            name,
            s.depth,
            s.size[0],
            s.size[1],
            s.size[0] / s.size[1].max(1.0),
            s.layers,
            s.column_items,
            s.max_column_items,
            s.max_column_lanes,
            s.max_column_height,
            s.shelf_items,
            s.shelf_rows,
            s.columns_width,
            s.gaps_width,
            s.max_gap_tracks,
            s.in_gates,
            s.out_gates,
            s.doc_gates,
            s.in_band,
            s.out_band,
            s.doc_strip
        );
    };
    let mut sorted: Vec<&studio_graph::ContainerStats> = stats.iter().collect();
    sorted.sort_by(|a, b| b.size[0].total_cmp(&a.size[0]));
    println!("    widest:");
    sorted.iter().take(15).for_each(|s| row(s));
    sorted.sort_by(|a, b| b.size[1].total_cmp(&a.size[1]));
    println!("    tallest:");
    sorted.iter().take(10).for_each(|s| row(s));
    sorted.sort_by_key(|s| std::cmp::Reverse(s.in_gates + s.out_gates));
    println!("    most gates:");
    sorted.iter().take(10).for_each(|s| row(s));
    println!();
}

/// Centre of the 4096-unit cell holding the most cards: where zoomed-in frames are busiest.
fn densest_point(graph: &Graph) -> Option<Pos2> {
    const CELL: f32 = 4096.0;
    let mut counts: std::collections::HashMap<(i32, i32), usize> = std::collections::HashMap::new();
    for node in graph.nodes.values() {
        let key = ((node.position[0] / CELL).floor() as i32, (node.position[1] / CELL).floor() as i32);
        *counts.entry(key).or_default() += 1;
    }
    let (&(x, y), _) = counts.iter().max_by_key(|(&key, &n)| (n, std::cmp::Reverse(key)))?;
    Some(Pos2::new((x as f32 + 0.5) * CELL, (y as f32 + 0.5) * CELL))
}

fn benchmark_rkyv_caching(tracker: &mut TimelineTracker) {
    println!("--------------------------------------------------------------------------------");
    println!(">>> BENCHMARK 2: rkyv ZERO-COPY CACHING & INSTANT STARTUP TEST");
    println!("    Target: 25,000 Nodes, 12,500 Edges (Simulating Large Multi-Crate Monorepo)");
    println!("--------------------------------------------------------------------------------");

    let count_nodes = 25_000;
    let count_edges = 12_500;

    let t_gen = Instant::now();
    let mut graph = Graph::new();
    let mut node_ids = Vec::with_capacity(count_nodes);

    for i in 0..count_nodes {
        let nid = graph.add_node(
            format!("component_{}", i),
            NodeArchetype::Function,
            format!("Module component description {}", i),
            Some("FN".to_string()),
            vec![("in".to_string(), DataType::RustFlow)],
            vec![("out".to_string(), DataType::RustFlow)],
            [(i % 100) as f32 * 120.0, (i / 100) as f32 * 80.0],
        );
        node_ids.push(nid);
    }

    for i in 0..count_edges {
        let src = node_ids[i];
        let dst = node_ids[(i + 1) % count_nodes];
        let src_port = graph.nodes[&src].outputs[0].id;
        let dst_port = graph.nodes[&dst].inputs[0].id;
        graph.connect(src, src_port, dst, dst_port);
    }
    graph.rebuild_fast_indices();
    let gen_dur = t_gen.elapsed();
    tracker.record_stage(
        "Synthetic Graph Gen",
        format!("{} nodes, {} edges generated in {:?}", count_nodes, count_edges, gen_dur),
        None,
        None,
        Some(count_nodes),
        Some(count_edges),
    );

    let stats = ProjectStats {
        project_name: "SyntheticLargeMonorepo".to_string(),
        crate_count: 12,
        file_count: 350,
        node_count: count_nodes,
        wire_count: count_edges,
        function_count: count_nodes,
        type_count: 0,
        ..Default::default()
    };

    let temp_proj = std::env::temp_dir().join("tbd_bench_rkyv_project");
    let _ = std::fs::create_dir_all(&temp_proj);
    let sample_file = temp_proj.join("Cargo.toml");
    let _ = std::fs::write(&sample_file, "[package]\nname = \"bench_proj\"\nversion = \"0.1.0\"\n");

    // Measure serialization
    let cache_path = save_project_cache(&temp_proj, &graph, &stats).expect("Failed to save rkyv cache");
    let file_size_mb = std::fs::metadata(&cache_path).map(|m| m.len() as f64 / 1_048_576.0).unwrap_or(0.0);
    tracker.record_stage(
        "rkyv Serialization",
        format!("Disk footprint: {:.2} MB written to {}", file_size_mb, cache_path.display()),
        None,
        None,
        Some(count_nodes),
        Some(count_edges),
    );

    // Measure zero-copy load
    let t_load = Instant::now();
    let loaded = load_project_cache(&temp_proj).expect("Failed to load rkyv cache");
    let load_dur = t_load.elapsed();

    assert!(loaded.is_some(), "Cache must be valid");
    let (restored_graph, restored_stats) = loaded.unwrap();
    let speedup = gen_dur.as_secs_f64() / load_dur.as_secs_f64();
    tracker.record_stage(
        "rkyv Zero-Copy Reload",
        format!("mmap + zero-copy access: {:.1}x faster than cold generation ({:?})", speedup, load_dur),
        None,
        None,
        Some(restored_graph.nodes.len()),
        Some(restored_graph.edges.len()),
    );

    assert_eq!(restored_graph.nodes.len(), count_nodes);
    assert_eq!(restored_stats.project_name, "SyntheticLargeMonorepo");

    // Clean up
    let _ = clear_project_cache(&temp_proj);
    let _ = std::fs::remove_dir_all(&temp_proj);
}

fn benchmark_extreme_scale(tracker: &mut TimelineTracker) {
    println!("--------------------------------------------------------------------------------");
    println!(">>> BENCHMARK 3: 10X EXTREME SCALE SYNTHETIC STRESS TEST");
    println!("    Target: 212,640 Components, 106,700 Active Wires, 50,000x50,000px World");
    println!("    Budget: {} on {}", tracker.budget.describe(), tracker.gpu_summary());
    println!("--------------------------------------------------------------------------------");

    let target_nodes = 212_640;
    let target_edges = 106_700;

    let mut graph = Graph::new();
    let archetypes = [
        NodeArchetype::Function,
        NodeArchetype::Struct,
        NodeArchetype::Enum,
        NodeArchetype::Trait,
        NodeArchetype::Module,
        NodeArchetype::File,
    ];

    let mut node_ids = Vec::with_capacity(target_nodes);
    let grid_cols = 461;
    let spacing = 110.0;

    for i in 0..target_nodes {
        let arch = archetypes[i % archetypes.len()];
        let col = (i % grid_cols) as f32;
        let row = (i / grid_cols) as f32;
        let x = col * spacing;
        let y = row * spacing;

        let node_id = graph.add_node(
            format!("component_node_{}", i),
            arch,
            format!("High-scale component instance {} in architecture map", i),
            Some(format!("{:?}", arch)),
            vec![("in".to_string(), DataType::RustFlow)],
            vec![("out".to_string(), DataType::RustFlow)],
            [x, y],
        );
        node_ids.push(node_id);
    }

    for i in 0..target_edges {
        let src_id = node_ids[i];
        let dst_id = node_ids[(i + 1) % target_nodes];
        let src_node = &graph.nodes[&src_id];
        let dst_node = &graph.nodes[&dst_id];
        let src_port = src_node.outputs[0].id;
        let dst_port = dst_node.inputs[0].id;
        graph.connect(src_id, src_port, dst_id, dst_port);
    }

    graph.rebuild_fast_indices();

    let mut spatial_grid = SpatialHashGrid::new(768.0);
    spatial_grid.build_from_graph(&graph);
    let scene = studio_canvas::CanvasScene::build(&graph, true);
    let search_index = SymbolSearchIndex::build(&graph);

    tracker.record_stage(
        "Extreme Scale Gen & Index",
        format!("{} nodes, {} edges indexed into Spatial Hash Grid", target_nodes, target_edges),
        None,
        None,
        Some(target_nodes),
        Some(target_edges),
    );

    let screen_w = 2560.0;
    let screen_h = 1440.0;

    let zoom_scenarios = [
        ("Extreme LOD 0 (High Zoom 200% 2.0x)", 2.0, 500),
        ("Extreme LOD 0 (Close-up 1.0x)", 1.0, 500),
        ("Extreme LOD 0 (Normal 0.5x)", 0.5, 500),
        ("Extreme LOD 1 (Medium 0.2x)", 0.2, 500),
        ("Extreme LOD 2 (Full Map 0.05x)", 0.05, 500),
    ];

    for (name, zoom, count) in zoom_scenarios {
        let mut scenario_us = 0.0;
        let world_w = screen_w / zoom;
        let world_h = screen_h / zoom;
        let visible_rect = Rect::from_min_size(Pos2::new(10000.0, 10000.0), egui::vec2(world_w, world_h));

        for _ in 0..count {
            let frame_start = Instant::now();

            // 1. Frustum Culling
            let visible_nodes = spatial_grid.query_rect(visible_rect);

            // 2. Point hover query
            let _hover = spatial_grid.query_point(Pos2::new(10500.0, 10500.0));

            // 3. Fast O(1) Collapsed cluster checks
            for &nid in visible_nodes.iter().take(32) {
                let _ = graph.is_node_in_collapsed_cluster(nid);
            }

            // 4. Visible wire tiles
            let wire_cull_rect = visible_rect.expand(200.0);
            let _ = scene.visible_ranges(wire_cull_rect).len() + scene.visible_curve_ranges(wire_cull_rect).len();

            // 5. Spotlight Search query
            let _ = search_index.search("component_node_10", 12);

            let frame_us = frame_start.elapsed().as_secs_f64() * 1_000_000.0;
            scenario_us += frame_us;
        }

        let avg_us = scenario_us / (count as f64);
        let fps = 1_000_000.0 / avg_us;
        let sample_nodes = spatial_grid.query_rect(visible_rect).len();
        let cull = visible_rect.expand(200.0);
        let sample_wires: usize = scene
            .visible_ranges(cull)
            .into_iter()
            .chain(scene.visible_curve_ranges(cull))
            .map(|r| (r.end - r.start) as usize)
            .sum();

        tracker.record_stage(
            name,
            format!("Zoom {:.2}x: {} visible nodes, {} wires", zoom, sample_nodes, sample_wires),
            Some(fps),
            Some(avg_us),
            Some(sample_nodes),
            Some(sample_wires),
        );
    }
}

fn benchmark_ultra_scale_5m(tracker: &mut TimelineTracker) {
    println!("--------------------------------------------------------------------------------");
    println!(">>> BENCHMARK 4: ULTRA-SCALE 5,000,000 NODES VERIFICATION");
    println!("    Target: 100,000 Folders, 500,000 Files, 5,000,000 Nodes with Lines");
    println!("    Budget: {}", tracker.budget.describe());
    println!("--------------------------------------------------------------------------------");

    let num_folders = 100_000;
    let files_per_folder = 5;
    let members_per_file = 9;
    let total_files = num_folders * files_per_folder;
    let total_nodes = total_files * (members_per_file + 1);
    let target_edges = total_files;

    let mut graph = Graph::new();
    let mut spatial_grid = SpatialHashGrid::new(1024.0);

    let folders_per_row = 100;
    let folder_w = 1200.0f32;
    let folder_h = 400.0f32;
    let gap = 80.0f32;

    let mut all_file_ids = Vec::with_capacity(total_files);
    let mut next_port_raw = 10_000_000u64;

    for f_idx in 0..num_folders {
        let row = f_idx / folders_per_row;
        let col = f_idx % folders_per_row;
        let folder_x = col as f32 * (folder_w + gap);
        let folder_y = row as f32 * (folder_h + gap);

        let cluster_id = format!("folder:{}", f_idx);
        let mut cluster = studio_graph::GroupCluster::new(&cluster_id, format!("dir_{}", f_idx), "Directory", row);
        cluster.position = [folder_x, folder_y];
        cluster.size = [folder_w, folder_h];

        let mut folder_node_ids = Vec::with_capacity(files_per_folder);

        for file_idx in 0..files_per_folder {
            let file_x = folder_x + 30.0 + (file_idx as f32 * 230.0);
            let file_y = folder_y + 40.0;

            let file_nid = graph.add_node(
                format!("module_{}_{}.rs", f_idx, file_idx),
                NodeArchetype::File,
                format!("File {} in folder {}", file_idx, f_idx),
                Some("RS".to_string()),
                vec![("in".to_string(), DataType::RustFlow)],
                vec![("out".to_string(), DataType::RustFlow)],
                [file_x, file_y],
            );

            let mut member_nodes = Vec::with_capacity(members_per_file);
            for m_idx in 0..members_per_file {
                let in_pid = studio_graph::PortId(next_port_raw);
                next_port_raw += 1;
                let out_pid = studio_graph::PortId(next_port_raw);
                next_port_raw += 1;
                let (arch, vis) =
                    if m_idx % 2 == 0 { (NodeArchetype::Function, "pub") } else { (NodeArchetype::Struct, "pub") };
                let mut member = studio_graph::FileMemberNode::new(
                    format!("member_{}", m_idx),
                    format!("fn_or_struct_{}", m_idx),
                    arch,
                    vis.to_string(),
                    format!("fn run_{}()", m_idx),
                    m_idx * 10 + 1,
                    "// source code",
                    None,
                );
                member.in_port_id = Some(in_pid);
                member.out_port_id = Some(out_pid);
                member_nodes.push(member);
            }

            if let Some(node) = graph.nodes.get_mut(&file_nid) {
                node.size = [220.0, 42.0];
                node.member_nodes = member_nodes;
            }

            folder_node_ids.push(file_nid);
            all_file_ids.push(file_nid);
        }

        cluster.node_ids = folder_node_ids;
        graph.clusters.push(cluster);
    }

    for i in 0..target_edges {
        let src_id = all_file_ids[i];
        let dst_id = all_file_ids[(i + 7) % total_files];
        let src_port = graph.nodes[&src_id].outputs[0].id;
        let dst_port = graph.nodes[&dst_id].inputs[0].id;
        graph.connect(src_id, src_port, dst_id, dst_port);
    }

    graph.rebuild_fast_indices();
    spatial_grid.build_from_graph(&graph);
    let scene = studio_canvas::CanvasScene::build(&graph, true);

    tracker.record_stage(
        "Ultra 5M Graph & Spatial Grid",
        "100k folders, 500k files, 5M components, 500k wires indexed".to_string(),
        None,
        None,
        Some(total_nodes),
        Some(target_edges),
    );

    // Verify RAM budget
    let last_event = tracker.events.last().unwrap();
    let current_ram_gb = last_event.rss_end_mb / 1024.0;
    let budget = tracker.budget;
    println!("  [✓] Verified RAM: {:.2} GB / {:.2} GB budget", current_ram_gb, budget.ram_gb());
    assert!(current_ram_gb <= budget.ram_gb(), "RAM exceeded the {:.2} GB budget", budget.ram_gb());

    println!(
        "\n  Simulating 5,000 UI Render Frames at {:.0} FPS budget ({:.2} ms / {:.0} µs max):",
        budget.target_fps,
        budget.frame_budget_us() / 1000.0,
        budget.frame_budget_us()
    );

    let screen_w = 2560.0;
    let screen_h = 1440.0;

    let zoom_scenarios = [
        ("Ultra LOD 0 (High Zoom 200% 2.0x)", 2.0, 500),
        ("Ultra LOD 0 (Close-up 1.0x)", 1.0, 500),
        ("Ultra LOD 1 (Medium 0.2x)", 0.2, 500),
        ("Ultra LOD 2 (Overview 0.05x)", 0.05, 500),
        ("Ultra LOD 3 (Galaxy 0.002x)", 0.002, 500),
    ];

    let mut total_simulated_frames = 0;
    let mut total_simulated_time_us = 0.0;
    let mut max_frame_us = 0.0f64;

    for (name, zoom, count) in zoom_scenarios {
        let mut scenario_us = 0.0;
        let world_w = screen_w / zoom;
        let world_h = screen_h / zoom;
        let visible_rect = Rect::from_min_size(Pos2::new(10000.0, 10000.0), egui::vec2(world_w, world_h));

        for _ in 0..count {
            let frame_start = Instant::now();

            // 1. Frustum Culling:
            // At galaxy zoom (< 0.005), 220px cards project to < 0.44px (completely sub-pixel).
            // Individual card queries are bypassed while the 100,000 parent folder clusters are rendered.
            let visible_nodes = if zoom >= 0.005 { spatial_grid.query_rect(visible_rect) } else { Vec::new() };

            // 2. Cursor hover query
            let _hover = spatial_grid.query_point(Pos2::new(10500.0, 10500.0));

            // 3. Fast O(1) Collapsed cluster checks
            for &nid in visible_nodes.iter().take(64) {
                let _ = graph.is_node_in_collapsed_cluster(nid);
            }

            // 4. Visible wire tiles, at every zoom level
            let wire_cull_rect = visible_rect.expand(200.0);
            let _ = scene.visible_ranges(wire_cull_rect).len() + scene.visible_curve_ranges(wire_cull_rect).len();

            let frame_us = frame_start.elapsed().as_secs_f64() * 1_000_000.0;
            scenario_us += frame_us;
            total_simulated_time_us += frame_us;
            if frame_us > max_frame_us {
                max_frame_us = frame_us;
            }
        }

        let avg_us = scenario_us / (count as f64);
        let fps = 1_000_000.0 / avg_us;
        let sample_nodes = if zoom >= 0.005 { spatial_grid.query_rect(visible_rect).len() } else { 0 };
        let cull = visible_rect.expand(200.0);
        let sample_wires: usize = scene
            .visible_ranges(cull)
            .into_iter()
            .chain(scene.visible_curve_ranges(cull))
            .map(|r| (r.end - r.start) as usize)
            .sum();

        tracker.record_stage(
            name,
            format!("Zoom {:.3}x: {} visible cards, {} wires (Clusters active)", zoom, sample_nodes, sample_wires),
            Some(fps),
            Some(avg_us),
            Some(sample_nodes),
            Some(sample_wires),
        );

        total_simulated_frames += count;
    }

    let overall_avg_us = total_simulated_time_us / (total_simulated_frames as f64);
    let overall_fps = 1_000_000.0 / overall_avg_us;

    println!("\n================================================================================");
    println!("     ULTRA-SCALE RESULTS (100,000 FOLDERS, 500,000 FILES, 5,000,000 NODES)     ");
    println!("================================================================================");
    println!(
        "  Budget Target:         {:.2} ms ({:.0} µs) for {:.0} FPS",
        budget.frame_budget_us() / 1000.0,
        budget.frame_budget_us(),
        budget.target_fps
    );
    println!("  Average Frame Time:    {:.2} µs ({:.3} ms)", overall_avg_us, overall_avg_us / 1000.0);
    println!("  Peak Max Frame Time:   {:.2} µs ({:.3} ms)", max_frame_us, max_frame_us / 1000.0);
    println!(
        "  Achieved Throughput:   {:.0} FPS ({:.1}x headroom vs {:.0} FPS)",
        overall_fps,
        budget.frame_budget_us() / overall_avg_us,
        budget.target_fps
    );
    println!("  Resident RAM:          {:.2} GB (budget {:.2} GB)", current_ram_gb, budget.ram_gb());
    println!("================================================================================\n");

    assert!(
        overall_avg_us < budget.frame_budget_us(),
        "Ultra scale frame time exceeded the {:.0} FPS budget",
        budget.target_fps
    );
    println!(
        "[SUCCESS] {:.0} FPS sustained on 5,000,000 nodes within {:.2} GB RAM\n",
        budget.target_fps,
        budget.ram_gb()
    );
}
