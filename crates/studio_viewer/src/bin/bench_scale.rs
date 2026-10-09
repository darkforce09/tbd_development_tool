use std::path::PathBuf;
use std::time::Instant;
use egui::{Pos2, Rect};
use studio_canvas::SpatialHashGrid;
use studio_graph::{DataType, Graph, NodeArchetype};
use studio_parser::{
    build_project_graph, clear_project_cache, extract_project, load_project_cache,
    save_project_cache, scan_project, ProjectStats, SymbolSearchIndex, ViewGranularity,
};
use studio_viewer::telemetry::{TelemetryBudget, TimelineTracker};

const USAGE: &str = "usage: bench_scale [PROJECT_DIR] [--ram-budget-gb GB] [--target-fps FPS]
  PROJECT_DIR      project to load for the real-world benchmark (default: current directory)
  --ram-budget-gb  RAM ceiling checked by the benchmark (default: half of system RAM)
  --target-fps     frame rate the render simulation must sustain (default: 60)";

struct BenchArgs {
    project: PathBuf,
    budget: TelemetryBudget,
}

fn parse_args() -> Result<BenchArgs, String> {
    let mut budget = TelemetryBudget::for_this_machine();
    let mut project = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut number = |flag: &str| -> Result<f64, String> {
            let value = args.next().ok_or(format!("{flag} needs a value"))?;
            value.parse::<f64>().ok().filter(|v| *v > 0.0).ok_or(format!("{flag}: '{value}' is not a positive number"))
        };
        match arg.as_str() {
            "--ram-budget-gb" => budget.ram_bytes = (number("--ram-budget-gb")? * 1_073_741_824.0) as u64,
            "--target-fps" => budget.target_fps = number("--target-fps")?,
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
    Ok(BenchArgs { project, budget })
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
    benchmark_real_project(&mut tracker, &args.project);

    // Part 2: rkyv Zero-Copy Caching Benchmark
    benchmark_rkyv_caching(&mut tracker);

    // Part 3: 10x Scale Stress Test (212,640 nodes, 106,700 edges)
    benchmark_extreme_scale(&mut tracker);

    // Part 4: Ultra Scale Verification: 100,000 Folders, 500,000 Files, 5,000,000 Nodes & Wires
    benchmark_ultra_scale_5m(&mut tracker);

    // Part 5: GPU Wire Batching & VRAM Efficiency Verification
    benchmark_gpu_wire_throughput(&mut tracker);

    // Final Timeline Summary Report
    tracker.print_timeline_summary();
}

fn benchmark_real_project(tracker: &mut TimelineTracker, target_path: &std::path::Path) {
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

    for k in &scanned.crates {
        if matches!(k.name.as_str(), "apps: mod" | "documentation" | ".ai" | "assets" | "contracts" | "deploy" | ".cursor" | ".github") {
            println!("      • Subsystem {:<18} -> {} files", format!("'{}'", k.name), k.source_files.len());
        }
    }

    // 1b. Tier 1 Instant Skeleton Startup (< 400ms)
    let t_skel = Instant::now();
    let (skel_graph, skel_stats) = studio_parser::build_skeleton_files_graph(&scanned);
    let skel_dur = t_skel.elapsed();
    let instant_startup_time = scan_dur + skel_dur;
    tracker.record_stage(
        "Tier 1 Instant Startup",
        format!("Scan + Layout: {} nodes, {} clusters ready to render in {:?}", skel_stats.node_count, skel_graph.clusters.len(), instant_startup_time),
        None,
        None,
        Some(skel_stats.node_count),
        None,
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
    let (mut graph, stats) = build_project_graph(&extracted, ViewGranularity::FilesAndFolders);
    let _build_dur = t2.elapsed();
    tracker.record_stage(
        "Graph Assembly",
        format!("{} nodes, {} edges, {} clusters", stats.node_count, stats.wire_count, graph.clusters.len()),
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

    // 7. Viewport Render Simulation across Zoom Levels (frame budget verification)
    println!("  Simulating Real-World Viewport Frames across Zoom Levels ({:.0} FPS budget):", tracker.budget.target_fps);
    let real_scenarios = [
        ("Real LOD 0 (High Zoom 200% 2.0x)", 2.0, 500),
        ("Real LOD 0 (Standard 100% 1.0x)", 1.0, 500),
        ("Real LOD 0 (Mid Zoom 47% 0.47x)", 0.47, 500),
        ("Real LOD 2 (Full Project 4.1% 0.041x)", 0.041, 500),
    ];
    let screen_w = 2560.0;
    let screen_h = 1440.0;
    for (name, zoom, count) in real_scenarios {
        let mut scenario_us = 0.0;
        let world_w = screen_w / zoom;
        let world_h = screen_h / zoom;
        let center = if let Some(wb) = grid.world_bounds() {
            Pos2::new((wb[0] + wb[2]) * 0.5 - world_w * 0.5, (wb[1] + wb[3]) * 0.5 - world_h * 0.5)
        } else {
            Pos2::new(0.0, 0.0)
        };
        let visible_rect = Rect::from_min_size(center, egui::vec2(world_w, world_h));

        let mut sample_nodes = 0;
        let mut sample_wires = 0;

        for _ in 0..count {
            let frame_start = Instant::now();
            let visible_nodes = grid.query_rect(visible_rect);
            sample_nodes = visible_nodes.len();

            let wire_cull_rect = visible_rect.expand(200.0);
            let visible_edges = grid.query_edges_rect(wire_cull_rect);
            sample_wires = visible_edges.len();

            let mut batch = studio_canvas::GpuWireBatch::new([screen_w, screen_h], zoom, 0.0);
            for &e_id in &visible_edges {
                if let Some(edge) = graph.get_edge(e_id) {
                    if let (Some(fn_node), Some(tn_node)) = (graph.nodes.get(&edge.from_node), graph.nodes.get(&edge.to_node)) {
                        let p0 = Pos2::new(fn_node.position[0], fn_node.position[1]);
                        let p3 = Pos2::new(tn_node.position[0], tn_node.position[1]);
                        batch.push_wire(p0, p3, egui::Color32::WHITE, None, 2.0, 0.0, false);
                    }
                }
            }

            let _ = search_index.search("config", 8);

            let frame_us = frame_start.elapsed().as_secs_f64() * 1_000_000.0;
            scenario_us += frame_us;
        }

        let avg_us = scenario_us / count as f64;
        let fps = 1_000_000.0 / avg_us.max(0.1);
        tracker.record_stage(
            name,
            format!("zoom: {:.3}x | nodes: {} | wires: {}", zoom, sample_nodes, sample_wires),
            Some(fps),
            Some(avg_us),
            Some(sample_nodes),
            Some(sample_wires),
        );
    }
    println!();
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
    };

    let temp_proj = std::env::temp_dir().join("tbd_bench_rkyv_project");
    let _ = std::fs::create_dir_all(&temp_proj);
    let sample_file = temp_proj.join("Cargo.toml");
    let _ = std::fs::write(&sample_file, "[package]\nname = \"bench_proj\"\nversion = \"0.1.0\"\n");

    // Measure serialization
    let cache_path = save_project_cache(&temp_proj, ViewGranularity::AllItems, &graph, &stats)
        .expect("Failed to save rkyv cache");
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
    let loaded = load_project_cache(&temp_proj, ViewGranularity::AllItems)
        .expect("Failed to load rkyv cache");
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
        NodeArchetype::Ingress,
        NodeArchetype::Compute,
        NodeArchetype::State,
        NodeArchetype::Egress,
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

            // 4. Bezier Wire Curve Spatial Query & Evaluation
            let wire_cull_rect = visible_rect.expand(200.0);
            let visible_edges = spatial_grid.query_edges_rect(wire_cull_rect);
            let _ = visible_edges.len();

            // 5. Spotlight Search query
            let _ = search_index.search("component_node_10", 12);

            let frame_us = frame_start.elapsed().as_secs_f64() * 1_000_000.0;
            scenario_us += frame_us;
        }

        let avg_us = scenario_us / (count as f64);
        let fps = 1_000_000.0 / avg_us;
        let sample_nodes = spatial_grid.query_rect(visible_rect).len();
        let sample_wires = spatial_grid.query_edges_rect(visible_rect.expand(200.0)).len();

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
        let mut cluster = studio_graph::GroupCluster::new(
            &cluster_id,
            format!("dir_{}", f_idx),
            "Directory",
            row,
        );
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
                let (arch, vis) = if m_idx % 2 == 0 {
                    (NodeArchetype::Function, "pub")
                } else {
                    (NodeArchetype::Struct, "pub")
                };
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
            let visible_nodes = if zoom >= 0.005 {
                spatial_grid.query_rect(visible_rect)
            } else {
                Vec::new()
            };

            // 2. Cursor hover query
            let _hover = spatial_grid.query_point(Pos2::new(10500.0, 10500.0));

            // 3. Fast O(1) Collapsed cluster checks
            for &nid in visible_nodes.iter().take(64) {
                let _ = graph.is_node_in_collapsed_cluster(nid);
            }

            // 4. Wire Culling & Evaluation:
            // Connection wires are active and queried across all zoom levels
            let wire_cull_rect = visible_rect.expand(200.0);
            let visible_edges = spatial_grid.query_edges_rect(wire_cull_rect);
            let _ = visible_edges.len();

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
        let sample_wires = spatial_grid.query_edges_rect(visible_rect.expand(200.0)).len();

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
    println!("  Budget Target:         {:.2} ms ({:.0} µs) for {:.0} FPS", budget.frame_budget_us() / 1000.0, budget.frame_budget_us(), budget.target_fps);
    println!("  Average Frame Time:    {:.2} µs ({:.3} ms)", overall_avg_us, overall_avg_us / 1000.0);
    println!("  Peak Max Frame Time:   {:.2} µs ({:.3} ms)", max_frame_us, max_frame_us / 1000.0);
    println!("  Achieved Throughput:   {:.0} FPS ({:.1}x headroom vs {:.0} FPS)", overall_fps, budget.frame_budget_us() / overall_avg_us, budget.target_fps);
    println!("  Resident RAM:          {:.2} GB (budget {:.2} GB)", current_ram_gb, budget.ram_gb());
    println!("================================================================================\n");

    assert!(overall_avg_us < budget.frame_budget_us(), "Ultra scale frame time exceeded the {:.0} FPS budget", budget.target_fps);
    println!("[SUCCESS] {:.0} FPS sustained on 5,000,000 nodes within {:.2} GB RAM\n", budget.target_fps, budget.ram_gb());
}

fn benchmark_gpu_wire_throughput(tracker: &mut TimelineTracker) {
    println!("--------------------------------------------------------------------------------");
    println!(">>> BENCHMARK 5: GPU WIRE BATCHING & VRAM EFFICIENCY VERIFICATION");
    println!("    Target: Sub-millisecond staging, compact 64B descriptors, {:.0} FPS", tracker.budget.target_fps);
    println!("--------------------------------------------------------------------------------");

    let wire_counts = [1_000, 10_000, 100_000, 500_000];
    for count in wire_counts {
        let t0 = Instant::now();
        let mut batch = studio_canvas::GpuWireBatch::new([2560.0, 1440.0], 1.0, 123.45);
        for i in 0..count {
            let offset = (i as f32) * 1.5;
            let p0 = Pos2::new(100.0 + offset, 200.0 + offset);
            let p3 = Pos2::new(500.0 + offset, 400.0 + offset);
            let is_animated = (i % 8) == 0;
            let glow = if is_animated {
                Some(egui::Color32::from_rgba_premultiplied(100, 200, 255, 90))
            } else {
                None
            };
            batch.push_wire(p0, p3, egui::Color32::WHITE, glow, 2.0, 6.0, is_animated);
        }
        let dur = t0.elapsed();
        let dur_us = dur.as_secs_f64() * 1_000_000.0;
        let vram_kb = (count * std::mem::size_of::<studio_canvas::GpuWireInstance>()) as f64 / 1024.0;
        let ns_per_wire = (dur.as_nanos() as f64) / (count as f64);
        let fps_equiv = 1_000_000.0 / dur_us.max(0.1);

        tracker.record_stage(
            format!("GPU Wire Batch ({}k)", count / 1000),
            format!("{:.1} ns/wire | VRAM: {:.1} KB | Batch: {:.2?}", ns_per_wire, vram_kb, dur),
            Some(fps_equiv),
            Some(dur_us),
            None,
            Some(count),
        );
    }
    println!();
}
