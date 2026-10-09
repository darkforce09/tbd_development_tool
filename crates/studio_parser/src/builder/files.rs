use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use studio_graph::{
    DataType, FileMemberNode, Graph, GroupCluster, NodeArchetype, NodeId, Port, PortDirection, PortId,
};
use crate::extractor::ExtractedProject;

use super::ProjectStats;

/// Builds a compact, hierarchical File/Folder architecture graph.
/// Inspired by CodeSee's clean nested containers, compact cards, and step numbers,
/// combined with CodeCanvas's spatial hierarchy and expandable inline view.
pub fn build_files_graph(project: &ExtractedProject) -> (Graph, ProjectStats) {
    let mut graph = Graph::new();
    let mut stats = ProjectStats {
        project_name: project.name.clone(),
        crate_count: project.crates.len(),
        ..Default::default()
    };

    let mut file_to_node: HashMap<String, NodeId> = HashMap::new();
    let mut file_to_in_port: HashMap<NodeId, PortId> = HashMap::new();
    let mut file_to_out_port: HashMap<NodeId, PortId> = HashMap::new();
    let mut module_to_file: HashMap<String, NodeId> = HashMap::new();
    let mut symbol_to_file: HashMap<String, NodeId> = HashMap::new();
    let mut pending_file_deps: Vec<(NodeId, String, &'static str)> = Vec::new();
    let mut member_in_ports: HashMap<String, (NodeId, PortId)> = HashMap::new();
    let mut member_out_ports: HashMap<String, (NodeId, PortId)> = HashMap::new();
    let mut pending_subnode_deps: Vec<(NodeId, PortId, String, &'static str)> = Vec::new();

    let mut current_crate_x = 100.0f32;
    let mut current_crate_y = 140.0f32;
    let mut current_shelf_height = 0.0f32;
    let max_shelf_width = 9_600.0f32;

    for (crate_idx, krate) in project.crates.iter().enumerate() {
        let crate_cluster_id = format!("crate:{}", krate.name);
        let (cluster_label, cluster_kind) = if krate.name.ends_with("_root") {
            (format!("root: {}", krate.name.trim_end_matches("_root")), "Root Directory")
        } else if krate.name.starts_with("apps:") {
            (krate.name.clone(), "App / Mod Subsystem")
        } else if krate.name.starts_with('.') {
            (format!("folder: {}", krate.name), "Config / AI Subsystem")
        } else if matches!(krate.name.as_str(), "documentation" | "assets" | "contracts" | "deploy") {
            (format!("folder: {}", krate.name), "Workspace Subsystem")
        } else if krate.name.contains(':') {
            (krate.name.clone(), "Directory Subsystem")
        } else {
            (format!("crate: {}", krate.name), "Crate Subsystem")
        };

        let mut crate_cluster = GroupCluster::new(
            &crate_cluster_id,
            cluster_label,
            cluster_kind,
            crate_idx,
        );
        crate_cluster.subtitle = Some(format!("{} files", krate.files.len()));

        let mut dir_to_files: BTreeMap<String, Vec<&crate::extractor::ExtractedFile>> = BTreeMap::new();

        for file in &krate.files {
            stats.file_count += 1;
            stats.function_count += file.functions.len() + file.impls.iter().map(|i| i.methods.len()).sum::<usize>();
            stats.type_count += file.structs.len() + file.enums.len() + file.traits.len();

            let parent_dir = file.relative_path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            dir_to_files.entry(parent_dir).or_default().push(file);
        }

        // Layout parameters
        let card_w = 220.0f32;
        let card_h = 42.0f32;
        let card_gap_x = 14.0f32;
        let card_gap_y = 10.0f32;
        let dir_gap_x = 36.0f32;
        let dir_gap_y = 40.0f32;

        let mut crate_node_ids = Vec::new();
        let mut child_clusters = Vec::new();

        let mut dir_layout_x = current_crate_x + 30.0f32;
        let mut dir_layout_y = current_crate_y + 40.0f32;
        let mut max_dir_row_height = 0.0f32;

        for (dir_path, files) in &dir_to_files {
            if files.is_empty() {
                continue;
            }

            let dir_cluster_id = format!("{}:{}", krate.name, if dir_path.is_empty() { "root" } else { dir_path });
            let dir_label = if dir_path.is_empty() {
                "root".to_string()
            } else {
                Path::new(dir_path).file_name().and_then(|n| n.to_str()).unwrap_or(dir_path).to_string()
            };

            let mut dir_cluster = GroupCluster::new(
                &dir_cluster_id,
                dir_label,
                "Directory",
                crate_idx,
            );
            dir_cluster.parent_id = Some(crate_cluster_id.clone());
            dir_cluster.subtitle = Some(format!("{} files", files.len()));

            // Smart 2-column packing: 1-4 files in 1 column, 5+ files in 2 columns
            let num_files = files.len();
            let num_cols = if num_files <= 4 { 1 } else { 2 };
            let rows_per_col = num_files.div_ceil(num_cols);

            let mut dir_node_ids = Vec::new();

            for (f_idx, file) in files.iter().enumerate() {
                let col = f_idx / rows_per_col;
                let row = f_idx % rows_per_col;

                let file_pos_x = dir_layout_x + 24.0 + (col as f32 * (card_w + card_gap_x));
                let file_pos_y = dir_layout_y + 44.0 + (row as f32 * (card_h + card_gap_y));

                let ext = file.relative_path.extension().and_then(|e| e.to_str()).unwrap_or("rs").to_lowercase();
                let file_name = file.relative_path.file_name().and_then(|f| f.to_str()).unwrap_or("file");
                let is_markdown = ext == "md" || ext == "markdown";
                let is_enforce = ext == "ens" || ext == "es" || (ext == "c" && file.structs.iter().any(|s| s.derives.contains(&"modded".to_string()) || s.source_code.contains("class ")));

                let badge = if is_markdown {
                    "MD".to_string()
                } else if is_enforce {
                    "ENS".to_string()
                } else {
                    ext.to_uppercase()
                };

                let total_items = file.functions.len() + file.structs.len() + file.enums.len() + file.traits.len();
                let desc = if is_markdown {
                    format!("{} headings, {} links", file.structs.len(), file.functions.len())
                } else if is_enforce {
                    let method_count = file.impls.iter().map(|i| i.methods.len()).sum::<usize>() + file.functions.len();
                    format!("{} classes, {} methods", file.structs.len(), method_count)
                } else {
                    format!(
                        "{} items ({} fns, {} types)",
                        total_items,
                        file.functions.len(),
                        file.structs.len() + file.enums.len()
                    )
                };

                let input_defs = vec![("in".to_string(), DataType::RustFlow)];
                let output_defs = vec![("out".to_string(), DataType::RustFlow)];

                let node_id = graph.add_node(
                    file_name,
                    NodeArchetype::File,
                    desc,
                    Some(badge),
                    input_defs,
                    output_defs,
                    [file_pos_x, file_pos_y],
                );

                // Extract file's internal nodes (functions, structs, enums, traits, methods)
                let mut member_nodes = Vec::new();

                for s in &file.structs {
                    let is_heading = s.derives.first().map(|d| d.starts_with('H')).unwrap_or(false);
                    let is_modded = s.derives.contains(&"modded".to_string());
                    let (archetype, vis, sig) = if is_heading {
                        let lvl = s.derives.first().cloned().unwrap_or_else(|| "H1".to_string());
                        (NodeArchetype::Module, lvl, s.name.clone())
                    } else if is_modded {
                        (NodeArchetype::Struct, "MOD".to_string(), format!("modded class {}", s.name))
                    } else if is_enforce || s.source_code.contains("class ") {
                        let base = s.derives.first().map(|b| format!(" : {}", b)).unwrap_or_default();
                        (NodeArchetype::Struct, "CLS".to_string(), format!("class {}{}", s.name, base))
                    } else {
                        let vis = if s.visibility.is_public() { "pub" } else { "" };
                        (NodeArchetype::Struct, vis.to_string(), format!("struct {} ({} fields)", s.name, s.fields.len()))
                    };

                    member_nodes.push(FileMemberNode::new(
                        format!("struct:{}", s.name),
                        &s.name,
                        archetype,
                        vis,
                        sig,
                        s.line,
                        &s.source_code,
                        if s.docs.is_empty() { None } else { Some(s.docs.clone()) },
                    ));
                }

                for e in &file.enums {
                    let vis = if e.visibility.is_public() { "pub" } else { "" };
                    member_nodes.push(FileMemberNode::new(
                        format!("enum:{}", e.name),
                        &e.name,
                        NodeArchetype::Enum,
                        vis,
                        format!("enum {} ({} variants)", e.name, e.variants.len()),
                        e.line,
                        &e.source_code,
                        if e.docs.is_empty() { None } else { Some(e.docs.clone()) },
                    ));
                }

                for t in &file.traits {
                    let vis = if t.visibility.is_public() { "pub" } else { "" };
                    let kind_label = if ext == "ts" || ext == "tsx" || ext == "java" || ext == "cs" || ext == "go" {
                        "interface"
                    } else {
                        "trait"
                    };
                    member_nodes.push(FileMemberNode::new(
                        format!("trait:{}", t.name),
                        &t.name,
                        NodeArchetype::Trait,
                        vis,
                        format!("{} {} ({} methods)", kind_label, t.name, t.methods.len()),
                        t.line,
                        &t.source_code,
                        if t.docs.is_empty() { None } else { Some(t.docs.clone()) },
                    ));
                }

                for f in &file.functions {
                    let is_link = f.docs.starts_with("Markdown link");
                    let is_code_block = f.name.starts_with("block:");
                    let (archetype, vis, sig) = if is_link {
                        (NodeArchetype::Function, "LNK".to_string(), format!("[{}]", f.name))
                    } else if is_code_block {
                        (NodeArchetype::Module, "CODE".to_string(), f.name.clone())
                    } else {
                        let vis = if f.visibility.is_public() { "pub" } else { "" };
                        let params_sig = f.inputs.iter().map(|p| format!("{}: {}", p.name, p.type_str)).collect::<Vec<_>>().join(", ");
                        let ret_sig = f.output.as_ref().map(|o| format!(" -> {}", o)).unwrap_or_default();
                        let prefix = if ext == "py" {
                            if f.is_async { "async def " } else { "def " }
                        } else if ext == "go" {
                            "func "
                        } else if ext == "sh" || ext == "bash" {
                            "sh "
                        } else if f.is_async {
                            "async fn "
                        } else {
                            "fn "
                        };
                        (NodeArchetype::Function, vis.to_string(), format!("{}{}({}){}", prefix, f.name, params_sig, ret_sig))
                    };

                    member_nodes.push(FileMemberNode::new(
                        format!("fn:{}", f.name),
                        &f.name,
                        archetype,
                        vis,
                        sig,
                        f.line,
                        &f.source_code,
                        if f.docs.is_empty() { None } else { Some(f.docs.clone()) },
                    ));
                }

                for imp in &file.impls {
                    for m in &imp.methods {
                        let vis = if is_enforce { "FN" } else if m.visibility.is_public() { "pub" } else { "" };
                        let params_sig = m.inputs.iter().map(|p| format!("{}: {}", p.name, p.type_str)).collect::<Vec<_>>().join(", ");
                        let ret_sig = m.output.as_ref().map(|o| format!(" -> {}", o)).unwrap_or_default();
                        let prefix = if m.is_async { "async fn " } else { "fn " };
                        member_nodes.push(FileMemberNode::new(
                            format!("method:{}::{}", imp.target_type, m.name),
                            format!("{}::{}", imp.target_type, m.name),
                            NodeArchetype::Function,
                            vis,
                            format!("{}{}({}){}", prefix, m.name, params_sig, ret_sig),
                            m.line,
                            &m.source_code,
                            if m.docs.is_empty() { None } else { Some(m.docs.clone()) },
                        ));
                    }
                }

                member_nodes.sort_by_key(|m| m.line_number);

                for m in &mut member_nodes {
                    let in_pid = PortId(graph.next_raw_id());
                    let out_pid = PortId(graph.next_raw_id());
                    m.in_port_id = Some(in_pid);
                    m.out_port_id = Some(out_pid);

                    member_in_ports.insert(m.name.clone(), (node_id, in_pid));
                    member_out_ports.insert(m.name.clone(), (node_id, out_pid));
                    member_in_ports.insert(m.id.clone(), (node_id, in_pid));
                    member_out_ports.insert(m.id.clone(), (node_id, out_pid));

                    if let Some(n) = graph.nodes.get_mut(&node_id) {
                        n.inputs.push(Port {
                            id: in_pid,
                            name: format!("{}:in", m.name),
                            data_type: DataType::RustFlow,
                            direction: PortDirection::Input,
                        });
                        n.outputs.push(Port {
                            id: out_pid,
                            name: format!("{}:out", m.name),
                            data_type: DataType::RustFlow,
                            direction: PortDirection::Output,
                        });
                    }
                }

                if let Some(n) = graph.nodes.get_mut(&node_id) {
                    n.size = [card_w, card_h];
                    n.member_nodes = member_nodes;
                }

                let docs_preview = file.functions.iter().find(|f| !f.docs.is_empty()).map(|f| f.docs.clone())
                    .or_else(|| file.structs.iter().find(|s| !s.docs.is_empty()).map(|s| s.docs.clone()));

                graph.set_node_metadata(
                    node_id,
                    Some(file.file_path.to_string_lossy().to_string()),
                    Some(1),
                    Some(krate.name.clone()),
                    Some(format!("{}::{}", krate.name, file.module_name)),
                    docs_preview,
                    None,
                    Some(dir_cluster_id.clone()),
                );

                dir_node_ids.push(node_id);
                crate_node_ids.push(node_id);

                let node_ref = &graph.nodes[&node_id];
                if let Some(in_port) = node_ref.inputs.first() {
                    file_to_in_port.insert(node_id, in_port.id);
                }
                if let Some(out_port) = node_ref.outputs.first() {
                    file_to_out_port.insert(node_id, out_port.id);
                }

                file_to_node.insert(file.relative_path.to_string_lossy().to_string(), node_id);
                file_to_node.insert(file_name.to_string(), node_id);
                if let Some(stem) = file.relative_path.file_stem().and_then(|s| s.to_str()) {
                    file_to_node.insert(stem.to_string(), node_id);
                }
                module_to_file.insert(file.module_name.clone(), node_id);
                module_to_file.insert(format!("{}::{}", krate.name, file.module_name), node_id);

                for f in &file.functions {
                    symbol_to_file.insert(f.name.clone(), node_id);
                    let caller_out = member_out_ports.get(&f.name).map(|&(_, p)| p);
                    for call in &f.calls {
                        pending_file_deps.push((node_id, call.clone(), "call"));
                        if let Some(caller_p) = caller_out {
                            pending_subnode_deps.push((node_id, caller_p, call.clone(), "call"));
                        }
                    }
                }
                for imp in &file.impls {
                    for m in &imp.methods {
                        let method_key = format!("{}::{}", imp.target_type, m.name);
                        symbol_to_file.insert(method_key.clone(), node_id);
                        let caller_out = member_out_ports.get(&method_key).map(|&(_, p)| p);
                        for call in &m.calls {
                            if let Some(caller_p) = caller_out {
                                pending_subnode_deps.push((node_id, caller_p, call.clone(), "call"));
                            }
                        }
                    }
                }
                for s in &file.structs {
                    symbol_to_file.insert(s.name.clone(), node_id);
                }
                for e in &file.enums {
                    symbol_to_file.insert(e.name.clone(), node_id);
                }
                for t in &file.traits {
                    symbol_to_file.insert(t.name.clone(), node_id);
                }

                for u in &file.uses {
                    let last_segment = u.path
                        .split([':', '/', '\\', '.'])
                        .filter(|s| !s.is_empty())
                        .last()
                        .unwrap_or("")
                        .trim()
                        .to_string();

                    if !last_segment.is_empty() {
                        pending_file_deps.push((node_id, last_segment, "use"));
                    }
                    if !u.path.is_empty() {
                        pending_file_deps.push((node_id, u.path.clone(), "use"));
                    }
                    for item in &u.items {
                        if !item.is_empty() && item != "import" && item != "#include" {
                            pending_file_deps.push((node_id, item.clone(), "use"));
                        }
                    }
                }
            }

            let dir_content_w = if num_cols == 1 {
                card_w + 48.0
            } else {
                (2.0 * card_w) + card_gap_x + 48.0
            };
            let dir_content_h = (rows_per_col as f32 * (card_h + card_gap_y)) + 68.0;

            dir_cluster.node_ids = dir_node_ids;
            dir_cluster.position = [dir_layout_x, dir_layout_y];
            dir_cluster.size = [dir_content_w, dir_content_h];

            max_dir_row_height = max_dir_row_height.max(dir_content_h);
            crate_cluster.child_cluster_ids.push(dir_cluster_id.clone());
            child_clusters.push(dir_cluster);

            dir_layout_x += dir_content_w + dir_gap_x;
            if (dir_layout_x - current_crate_x) > 960.0 {
                dir_layout_x = current_crate_x + 30.0;
                dir_layout_y += max_dir_row_height + dir_gap_y;
                max_dir_row_height = 0.0;
            }
        }

        crate_cluster.node_ids = crate_node_ids;
        let crate_total_width = (dir_layout_x - current_crate_x).max(500.0);
        let crate_total_height = (dir_layout_y + max_dir_row_height - current_crate_y + 40.0).max(300.0);
        current_shelf_height = current_shelf_height.max(crate_total_height);

        current_crate_x += crate_total_width + 80.0;
        if current_crate_x > max_shelf_width {
            current_crate_x = 100.0;
            current_crate_y += current_shelf_height + 120.0;
            current_shelf_height = 0.0;
        }

        graph.clusters.push(crate_cluster);
        graph.clusters.extend(child_clusters);
    }

    // Connect inter-file dependencies with sequential flow step numbers (CodeSee style)
    let mut step = 1;
    let mut connected_pairs: HashSet<(NodeId, NodeId)> = HashSet::new();

    for (src_file_node, target_name, label) in pending_file_deps {
        let clean = target_name
            .trim_start_matches("./")
            .trim_end_matches(".js")
            .trim_end_matches(".ts")
            .trim_end_matches(".tsx")
            .trim_end_matches(".jsx")
            .trim_end_matches(".py");
        let stem = Path::new(&clean).file_stem().and_then(|s| s.to_str()).unwrap_or(clean);

        let target_node = module_to_file.get(&target_name)
            .or_else(|| symbol_to_file.get(&target_name))
            .or_else(|| file_to_node.get(&target_name))
            .or_else(|| file_to_node.get(clean))
            .or_else(|| file_to_node.get(stem))
            .or_else(|| symbol_to_file.get(stem))
            .or_else(|| module_to_file.get(stem))
            .copied();

        if let Some(target_file_node) = target_node {
            if target_file_node != src_file_node && connected_pairs.insert((src_file_node, target_file_node)) {
                if let (Some(&out_port), Some(&in_port)) = (
                        file_to_out_port.get(&src_file_node),
                        file_to_in_port.get(&target_file_node),
                    ) {
                        graph.connect_labeled(
                            src_file_node,
                            out_port,
                            target_file_node,
                            in_port,
                            Some(label.to_string()),
                            Some(step),
                            None,
                        );
                        step += 1;
                    }
                }
            }
    }

    // Connect fine-grained sub-node dependencies between individual function/method/link member ports
    let mut connected_subnode_pairs: HashSet<(NodeId, PortId, NodeId, PortId)> = HashSet::new();
    for (src_file, caller_out_port, target_name, label) in pending_subnode_deps {
        let clean = target_name.trim_start_matches("./");
        let short_name = target_name
            .split([':', '.', '>', '-'])
            .filter(|s| !s.is_empty())
            .last()
            .unwrap_or(&target_name);

        let target = member_in_ports.get(&target_name)
            .or_else(|| member_in_ports.get(clean))
            .or_else(|| member_in_ports.get(short_name))
            .copied()
            .or_else(|| {
                // If a markdown link or call references a file directly, connect caller port to target file's in_port!
                file_to_node.get(&target_name)
                    .or_else(|| file_to_node.get(clean))
                    .or_else(|| file_to_node.get(short_name))
                    .and_then(|tid| file_to_in_port.get(tid).map(|&pid| (*tid, pid)))
            });

        if let Some((target_file, callee_in_port)) = target {
            if (src_file != target_file || caller_out_port != callee_in_port)
                && connected_subnode_pairs.insert((src_file, caller_out_port, target_file, callee_in_port))
            {
                graph.connect_labeled(
                    src_file,
                    caller_out_port,
                    target_file,
                    callee_in_port,
                    Some(label.to_string()),
                    Some(step),
                    None,
                );
                step += 1;
            }
        }
    }

    graph.update_cluster_bounds();
    graph.rebuild_fast_indices();

    stats.node_count = graph.nodes.len();
    stats.wire_count = graph.edges.len();

    (graph, stats)
}

/// Builds a lightweight initial File/Folder spatial graph directly from scanned file paths (< 30ms).
/// Used by Tier 1 progressive startup so the user can immediately view and navigate the project canvas.
pub fn build_skeleton_files_graph(project: &crate::project::RustProject) -> (Graph, ProjectStats) {
    let mut graph = Graph::new();
    let mut stats = ProjectStats {
        project_name: project.name.clone(),
        crate_count: project.crates.len(),
        ..Default::default()
    };

    let card_w = 220.0f32;
    let card_h = 42.0f32;
    let card_gap_x = 14.0f32;
    let card_gap_y = 10.0f32;
    let dir_gap_x = 36.0f32;
    let dir_gap_y = 40.0f32;

    let mut current_crate_x = 100.0f32;
    let mut current_crate_y = 140.0f32;
    let mut current_shelf_height = 0.0f32;
    let max_shelf_width = 9_600.0f32;

    for (crate_idx, krate) in project.crates.iter().enumerate() {
        let crate_cluster_id = format!("crate:{}", krate.name);
        let (cluster_label, cluster_kind) = if krate.name.ends_with("_root") {
            (format!("root: {}", krate.name.trim_end_matches("_root")), "Root Directory")
        } else if krate.name.starts_with("apps:") {
            (krate.name.clone(), "App / Mod Subsystem")
        } else if krate.name.starts_with('.') {
            (format!("folder: {}", krate.name), "Config / AI Subsystem")
        } else if matches!(krate.name.as_str(), "documentation" | "assets" | "contracts" | "deploy") {
            (format!("folder: {}", krate.name), "Workspace Subsystem")
        } else if krate.name.contains(':') {
            (krate.name.clone(), "Directory Subsystem")
        } else {
            (format!("crate: {}", krate.name), "Crate Subsystem")
        };

        let mut crate_cluster = GroupCluster::new(
            &crate_cluster_id,
            cluster_label,
            cluster_kind,
            crate_idx,
        );
        crate_cluster.subtitle = Some(format!("{} files", krate.source_files.len()));

        let mut dir_to_files: BTreeMap<String, Vec<&Path>> = BTreeMap::new();
        for file_path in &krate.source_files {
            stats.file_count += 1;
            let rel_path = file_path.strip_prefix(&krate.root_path).unwrap_or(file_path);
            let parent_dir = rel_path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            dir_to_files.entry(parent_dir).or_default().push(file_path.as_path());
        }

        let mut crate_node_ids = Vec::new();
        let mut child_clusters = Vec::new();

        let mut dir_layout_x = current_crate_x + 30.0f32;
        let mut dir_layout_y = current_crate_y + 40.0f32;
        let mut max_dir_row_height = 0.0f32;

        for (dir_path, files) in &dir_to_files {
            let dir_cluster_id = format!("{}:{}", krate.name, if dir_path.is_empty() { "root" } else { dir_path });
            let dir_label = if dir_path.is_empty() {
                "root".to_string()
            } else {
                Path::new(dir_path).file_name().and_then(|n| n.to_str()).unwrap_or(dir_path).to_string()
            };

            let mut dir_cluster = GroupCluster::new(
                &dir_cluster_id,
                dir_label,
                "Directory",
                crate_idx,
            );
            dir_cluster.parent_id = Some(crate_cluster_id.clone());
            dir_cluster.subtitle = Some(format!("{} files", files.len()));

            let num_files = files.len();
            let num_cols = if num_files <= 4 { 1 } else { 2 };
            let rows_per_col = num_files.div_ceil(num_cols);

            let mut dir_node_ids = Vec::new();

            for (f_idx, file_path) in files.iter().enumerate() {
                let col = f_idx / rows_per_col;
                let row = f_idx % rows_per_col;

                let file_pos_x = dir_layout_x + 24.0 + (col as f32 * (card_w + card_gap_x));
                let file_pos_y = dir_layout_y + 44.0 + (row as f32 * (card_h + card_gap_y));

                let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("rs").to_lowercase();
                let file_name = file_path.file_name().and_then(|f| f.to_str()).unwrap_or("file");
                let badge = match ext.as_str() {
                    "md" | "markdown" => "MD".to_string(),
                    "ens" | "es" | "c" => "ENS".to_string(),
                    "rs" => "RS".to_string(),
                    "json" => "JSON".to_string(),
                    "toml" => "TOML".to_string(),
                    "py" => "PY".to_string(),
                    "ts" | "js" => "JS".to_string(),
                    "bvh" => "BVH".to_string(),
                    "layout" => "UI".to_string(),
                    "meta" => "META".to_string(),
                    _ => ext.to_uppercase(),
                };

                let input_defs = vec![("in".to_string(), DataType::RustFlow)];
                let output_defs = vec![("out".to_string(), DataType::RustFlow)];

                let node_id = graph.add_node(
                    file_name,
                    NodeArchetype::File,
                    "Loading details...".to_string(),
                    Some(badge),
                    input_defs,
                    output_defs,
                    [file_pos_x, file_pos_y],
                );

                if let Some(n) = graph.nodes.get_mut(&node_id) {
                    n.size = [card_w, card_h];
                }

                graph.set_node_metadata(
                    node_id,
                    Some(file_path.to_string_lossy().to_string()),
                    Some(1),
                    Some(krate.name.clone()),
                    None,
                    None,
                    None,
                    Some(dir_cluster_id.clone()),
                );

                dir_node_ids.push(node_id);
                crate_node_ids.push(node_id);
            }

            let dir_content_w = if num_cols == 1 {
                card_w + 48.0
            } else {
                (2.0 * card_w) + card_gap_x + 48.0
            };
            let dir_content_h = (rows_per_col as f32 * (card_h + card_gap_y)) + 68.0;

            dir_cluster.node_ids = dir_node_ids;
            dir_cluster.position = [dir_layout_x, dir_layout_y];
            dir_cluster.size = [dir_content_w, dir_content_h];

            max_dir_row_height = max_dir_row_height.max(dir_content_h);
            crate_cluster.child_cluster_ids.push(dir_cluster_id.clone());
            child_clusters.push(dir_cluster);

            dir_layout_x += dir_content_w + dir_gap_x;
            if (dir_layout_x - current_crate_x) > 960.0 {
                dir_layout_x = current_crate_x + 30.0;
                dir_layout_y += max_dir_row_height + dir_gap_y;
                max_dir_row_height = 0.0;
            }
        }

        crate_cluster.node_ids = crate_node_ids;
        let crate_total_width = (dir_layout_x - current_crate_x).max(500.0);
        let crate_total_height = (dir_layout_y + max_dir_row_height - current_crate_y + 40.0).max(300.0);
        current_shelf_height = current_shelf_height.max(crate_total_height);

        current_crate_x += crate_total_width + 80.0;
        if current_crate_x > max_shelf_width {
            current_crate_x = 100.0;
            current_crate_y += current_shelf_height + 120.0;
            current_shelf_height = 0.0;
        }

        graph.clusters.push(crate_cluster);
        graph.clusters.extend(child_clusters);
    }

    graph.update_cluster_bounds();
    graph.rebuild_fast_indices();

    stats.node_count = graph.nodes.len();
    stats.wire_count = graph.edges.len();

    (graph, stats)
}
