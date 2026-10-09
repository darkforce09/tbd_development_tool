use crate::extractor::ExtractedProject;
use std::collections::HashMap;
use studio_graph::{DataType, Graph, GroupCluster, NodeArchetype, NodeId, PortId};

use super::common::truncate_str;
use super::ProjectStats;

pub fn build_modules_graph(project: &ExtractedProject) -> (Graph, ProjectStats) {
    let mut graph = Graph::new();
    let mut stats =
        ProjectStats { project_name: project.name.clone(), crate_count: project.crates.len(), ..Default::default() };

    let mut module_to_node: HashMap<String, NodeId> = HashMap::new();
    let mut pending_uses: Vec<(NodeId, PortId, String)> = Vec::new();

    let mut current_x = 100.0f32;
    let card_width = 300.0f32;
    let col_gap = 90.0f32;
    let row_gap = 50.0f32;

    for (crate_idx, krate) in project.crates.iter().enumerate() {
        let mut current_y = 100.0f32;
        let mut row_count = 0;
        let mut crate_node_ids = Vec::new();

        for file in &krate.files {
            stats.file_count += 1;

            let mod_name = file.module_name.clone();
            let total_items = file.functions.len() + file.structs.len() + file.enums.len() + file.traits.len();

            let badge = format!("CRATE: {}", krate.name.to_uppercase());
            let desc = format!(
                "{} • {} items ({} fns, {} types)",
                file.relative_path.display(),
                total_items,
                file.functions.len(),
                file.structs.len() + file.enums.len()
            );

            // Inputs: imported modules
            let mut input_defs = Vec::new();
            for u in file.uses.iter().take(4) {
                input_defs.push((truncate_str(&u.path, 24).to_string(), DataType::RustFlow));
            }

            // Outputs: exported items
            let mut output_defs = Vec::new();
            for s in file.structs.iter().filter(|s| s.visibility.is_public()).take(3) {
                output_defs.push((format!("struct {}", s.name), DataType::RustType(s.name.clone())));
            }
            for f in file.functions.iter().filter(|f| f.visibility.is_public()).take(3) {
                output_defs.push((format!("fn {}", f.name), DataType::RustFlow));
            }
            if output_defs.is_empty() {
                output_defs.push(("mod_export".to_string(), DataType::RustFlow));
            }

            let node_id = graph.add_node(
                format!("mod {}", mod_name),
                NodeArchetype::Module,
                desc,
                Some(badge),
                input_defs,
                output_defs,
                [current_x, current_y],
            );

            graph.set_node_metadata(
                node_id,
                Some(file.file_path.to_string_lossy().to_string()),
                Some(1),
                Some(krate.name.clone()),
                Some(format!("{}::{}", krate.name, mod_name)),
                None,
                Some(format!("// Module {}\npub mod {};", file.relative_path.display(), mod_name)),
                Some(krate.name.clone()),
            );
            crate_node_ids.push(node_id);

            let node_ref = &graph.nodes[&node_id];
            module_to_node.insert(mod_name.clone(), node_id);

            // Record uses
            for (idx, u) in file.uses.iter().take(4).enumerate() {
                if let Some(port) = node_ref.inputs.get(idx) {
                    let imported_mod =
                        u.path.split([':', '/', '\\', '.']).rfind(|s| !s.is_empty()).unwrap_or("").trim().to_string();
                    pending_uses.push((node_id, port.id, imported_mod));
                }
            }

            current_y += node_ref.size[1] + row_gap;
            row_count += 1;
            if row_count >= 5 {
                current_x += card_width + col_gap;
                current_y = 100.0;
                row_count = 0;
            }
        }

        if row_count > 0 {
            current_x += card_width + col_gap;
        }

        if !crate_node_ids.is_empty() {
            let mut c =
                GroupCluster::new(&krate.name, format!("crate: {}", krate.name), "Module Architecture", crate_idx);
            c.node_ids = crate_node_ids;
            graph.clusters.push(c);
        }
    }

    // Connect inter-module dependencies
    let mut step = 1;
    for (dst_node, dst_port, imported_mod) in pending_uses {
        if let Some(&src_node) = module_to_node.get(&imported_mod) {
            if src_node != dst_node {
                if let Some(out_port) = graph.nodes[&src_node].outputs.first().map(|p| p.id) {
                    graph.connect_labeled(
                        src_node,
                        out_port,
                        dst_node,
                        dst_port,
                        Some("use".to_string()),
                        Some(step),
                        None,
                    );
                    step += 1;
                }
            }
        }
    }

    graph.update_cluster_bounds();
    graph.rebuild_fast_indices();

    stats.node_count = graph.nodes.len();
    stats.wire_count = graph.edges.len();

    (graph, stats)
}
