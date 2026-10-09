use std::collections::HashMap;
use studio_graph::{DataType, Graph, GroupCluster, NodeArchetype, NodeId, PortId};
use crate::extractor::{ExtractedProject, FunctionItem};

use super::common::{clean_type_key, truncate_str, NodeLookup};
use super::ProjectStats;

pub fn build_items_graph(project: &ExtractedProject, public_only: bool) -> (Graph, ProjectStats) {
    let mut graph = Graph::new();
    let mut stats = ProjectStats {
        project_name: project.name.clone(),
        crate_count: project.crates.len(),
        ..Default::default()
    };

    let mut name_to_node: HashMap<String, NodeLookup> = HashMap::new();
    let mut pending_calls: Vec<(NodeId, PortId, String)> = Vec::new();
    let mut pending_type_wires: Vec<(NodeId, PortId, String)> = Vec::new();

    let mut current_x = 100.0f32;
    let mut col_max_width = 280.0f32;
    let col_gap = 60.0f32;
    let item_gap = 28.0f32;

    for (crate_idx, krate) in project.crates.iter().enumerate() {
        let mut crate_node_ids = Vec::new();

        for file in &krate.files {
            stats.file_count += 1;

            let file_label = file.relative_path.to_string_lossy();
            let mut current_y = 100.0f32;
            let mut items_in_col = 0;

            // 1. Structs
            for s in &file.structs {
                if public_only && !s.visibility.is_public() {
                    continue;
                }
                stats.type_count += 1;

                let badge = format!("{}STRUCT", s.visibility.badge_prefix());
                let desc = if !s.docs.is_empty() {
                    s.docs.clone()
                } else {
                    format!("{} • {} fields", file_label, s.fields.len())
                };

                let input_defs: Vec<(String, DataType)> = s
                    .fields
                    .iter()
                    .map(|f| {
                        (
                            format!("{}: {}", f.name, truncate_str(&f.type_str, 24)),
                            DataType::RustType(f.type_str.clone()),
                        )
                    })
                    .collect();

                let output_defs = vec![("Self".to_string(), DataType::RustType(s.name.clone()))];

                let node_id = graph.add_node(
                    format!("struct {}", s.name),
                    NodeArchetype::Struct,
                    desc,
                    Some(badge),
                    input_defs,
                    output_defs,
                    [current_x, current_y],
                );

                graph.set_node_metadata(
                    node_id,
                    Some(file.file_path.to_string_lossy().to_string()),
                    Some(s.line),
                    Some(krate.name.clone()),
                    Some(format!("{}::{}", krate.name, file.module_name)),
                    if !s.docs.is_empty() { Some(s.docs.clone()) } else { None },
                    Some(s.source_code.clone()),
                    Some(krate.name.clone()),
                );
                crate_node_ids.push(node_id);

                let node_ref = &graph.nodes[&node_id];
                let self_out = node_ref.outputs.first().map(|p| p.id);
                let mut type_inputs = HashMap::new();
                for (idx, field) in s.fields.iter().enumerate() {
                    if let Some(port) = node_ref.inputs.get(idx) {
                        type_inputs.insert(clean_type_key(&field.type_str), port.id);
                    }
                }

                name_to_node.insert(
                    s.name.clone(),
                    NodeLookup {
                        node_id,
                        exec_in_port: None,
                        call_out_port: None,
                        self_out_port: self_out,
                        type_inputs,
                        type_outputs: HashMap::new(),
                    },
                );

                col_max_width = col_max_width.max(node_ref.size[0]);
                current_y += node_ref.size[1] + item_gap;
                items_in_col += 1;
                if items_in_col >= 6 {
                    current_x += col_max_width + col_gap;
                    current_y = 100.0;
                    col_max_width = 280.0;
                    items_in_col = 0;
                }
            }

            // 2. Enums
            for e in &file.enums {
                if public_only && !e.visibility.is_public() {
                    continue;
                }
                stats.type_count += 1;

                let badge = format!("{}ENUM", e.visibility.badge_prefix());
                let desc = if !e.docs.is_empty() {
                    e.docs.clone()
                } else {
                    format!("{} • {} variants", file_label, e.variants.len())
                };

                let mut output_defs = vec![("Self".to_string(), DataType::RustType(e.name.clone()))];
                for v in e.variants.iter().take(4) {
                    output_defs.push((v.clone(), DataType::RustType(e.name.clone())));
                }

                let node_id = graph.add_node(
                    format!("enum {}", e.name),
                    NodeArchetype::Enum,
                    desc,
                    Some(badge),
                    vec![],
                    output_defs,
                    [current_x, current_y],
                );

                graph.set_node_metadata(
                    node_id,
                    Some(file.file_path.to_string_lossy().to_string()),
                    Some(e.line),
                    Some(krate.name.clone()),
                    Some(format!("{}::{}", krate.name, file.module_name)),
                    if !e.docs.is_empty() { Some(e.docs.clone()) } else { None },
                    Some(e.source_code.clone()),
                    Some(krate.name.clone()),
                );
                crate_node_ids.push(node_id);

                let node_ref = &graph.nodes[&node_id];
                let self_out = node_ref.outputs.first().map(|p| p.id);

                name_to_node.insert(
                    e.name.clone(),
                    NodeLookup {
                        node_id,
                        exec_in_port: None,
                        call_out_port: None,
                        self_out_port: self_out,
                        type_inputs: HashMap::new(),
                        type_outputs: HashMap::new(),
                    },
                );

                col_max_width = col_max_width.max(node_ref.size[0]);
                current_y += node_ref.size[1] + item_gap;
                items_in_col += 1;
                if items_in_col >= 6 {
                    current_x += col_max_width + col_gap;
                    current_y = 100.0;
                    col_max_width = 280.0;
                    items_in_col = 0;
                }
            }

            // 3. Traits
            for t in &file.traits {
                if public_only && !t.visibility.is_public() {
                    continue;
                }
                stats.type_count += 1;

                let badge = format!("{}TRAIT", t.visibility.badge_prefix());
                let desc = if !t.docs.is_empty() {
                    t.docs.clone()
                } else {
                    format!("{} • {} methods", file_label, t.methods.len())
                };

                let output_defs = t
                    .methods
                    .iter()
                    .take(5)
                    .map(|m| (m.clone(), DataType::RustFlow))
                    .collect();

                let node_id = graph.add_node(
                    format!("trait {}", t.name),
                    NodeArchetype::Trait,
                    desc,
                    Some(badge),
                    vec![],
                    output_defs,
                    [current_x, current_y],
                );

                graph.set_node_metadata(
                    node_id,
                    Some(file.file_path.to_string_lossy().to_string()),
                    Some(t.line),
                    Some(krate.name.clone()),
                    Some(format!("{}::{}", krate.name, file.module_name)),
                    if !t.docs.is_empty() { Some(t.docs.clone()) } else { None },
                    Some(t.source_code.clone()),
                    Some(krate.name.clone()),
                );
                crate_node_ids.push(node_id);

                let node_ref = &graph.nodes[&node_id];
                name_to_node.insert(
                    t.name.clone(),
                    NodeLookup {
                        node_id,
                        exec_in_port: None,
                        call_out_port: None,
                        self_out_port: None,
                        type_inputs: HashMap::new(),
                        type_outputs: HashMap::new(),
                    },
                );

                col_max_width = col_max_width.max(node_ref.size[0]);
                current_y += node_ref.size[1] + item_gap;
                items_in_col += 1;
                if items_in_col >= 6 {
                    current_x += col_max_width + col_gap;
                    current_y = 100.0;
                    col_max_width = 280.0;
                    items_in_col = 0;
                }
            }

            // 4. Free Functions & Impl Methods
            let all_functions: Vec<&FunctionItem> = file
                .functions
                .iter()
                .chain(file.impls.iter().flat_map(|i| i.methods.iter()))
                .collect();

            for f in all_functions {
                if public_only && !f.visibility.is_public() {
                    continue;
                }
                stats.function_count += 1;

                let badge = if f.is_async {
                    format!("{}ASYNC FN", f.visibility.badge_prefix())
                } else {
                    format!("{}FN", f.visibility.badge_prefix())
                };

                let desc = if !f.docs.is_empty() {
                    f.docs.clone()
                } else {
                    format!("{}:{}", file_label, f.line)
                };

                // Inputs: "exec" trigger + parameters
                let mut input_defs = vec![("exec".to_string(), DataType::RustFlow)];
                for p in &f.inputs {
                    input_defs.push((
                        format!("{}: {}", p.name, truncate_str(&p.type_str, 20)),
                        DataType::RustType(p.type_str.clone()),
                    ));
                }

                // Outputs: "call" trigger + return type
                let mut output_defs = vec![("call".to_string(), DataType::RustFlow)];
                if let Some(ret) = &f.output {
                    output_defs.push((
                        format!("-> {}", truncate_str(ret, 22)),
                        DataType::RustType(ret.clone()),
                    ));
                }

                let node_id = graph.add_node(
                    format!("fn {}", f.name),
                    NodeArchetype::Function,
                    desc,
                    Some(badge),
                    input_defs,
                    output_defs,
                    [current_x, current_y],
                );

                graph.set_node_metadata(
                    node_id,
                    Some(file.file_path.to_string_lossy().to_string()),
                    Some(f.line),
                    Some(krate.name.clone()),
                    Some(format!("{}::{}", krate.name, file.module_name)),
                    if !f.docs.is_empty() { Some(f.docs.clone()) } else { None },
                    Some(f.source_code.clone()),
                    Some(krate.name.clone()),
                );
                crate_node_ids.push(node_id);

                let node_ref = &graph.nodes[&node_id];
                let exec_in = node_ref.inputs.first().map(|p| p.id);
                let call_out = node_ref.outputs.first().map(|p| p.id);

                // Record parameter ports for type wiring
                for (idx, p) in f.inputs.iter().enumerate() {
                    if let Some(port) = node_ref.inputs.get(idx + 1) {
                        let clean_k = clean_type_key(&p.type_str);
                        pending_type_wires.push((node_id, port.id, clean_k));
                    }
                }

                // Record outgoing calls
                if let Some(cout) = call_out {
                    for called_fn in &f.calls {
                        pending_calls.push((node_id, cout, called_fn.clone()));
                    }
                }

                name_to_node.insert(
                    f.name.clone(),
                    NodeLookup {
                        node_id,
                        exec_in_port: exec_in,
                        call_out_port: call_out,
                        self_out_port: None,
                        type_inputs: HashMap::new(),
                        type_outputs: HashMap::new(),
                    },
                );

                col_max_width = col_max_width.max(node_ref.size[0]);
                current_y += node_ref.size[1] + item_gap;
                items_in_col += 1;
                if items_in_col >= 6 {
                    current_x += col_max_width + col_gap;
                    current_y = 100.0;
                    col_max_width = 280.0;
                    items_in_col = 0;
                }
            }

            if items_in_col > 0 {
                current_x += col_max_width + col_gap;
                col_max_width = 280.0;
            }
        }

        if !crate_node_ids.is_empty() {
            let mut c = GroupCluster::new(&krate.name, format!("crate: {}", krate.name), "Crate Subsystem", crate_idx);
            c.node_ids = crate_node_ids;
            graph.clusters.push(c);
        }
    }

    // Connect edges for function calls with step numbers (call -> exec)
    let mut call_step = 1;
    for (src_node, src_port, callee_name) in pending_calls {
        if let Some(target) = name_to_node.get(&callee_name) {
            if target.node_id != src_node {
                if let Some(exec_port) = target.exec_in_port {
                    graph.connect_labeled(
                        src_node,
                        src_port,
                        target.node_id,
                        exec_port,
                        Some("call".to_string()),
                        Some(call_step),
                        None,
                    );
                    call_step += 1;
                }
            }
        }
    }

    // Connect edges for data types (Self -> parameter)
    for (consumer_node, consumer_port, type_name) in pending_type_wires {
        if let Some(producer) = name_to_node.get(&type_name) {
            if producer.node_id != consumer_node {
                if let Some(self_port) = producer.self_out_port {
                    graph.connect_labeled(
                        producer.node_id,
                        self_port,
                        consumer_node,
                        consumer_port,
                        Some("type".to_string()),
                        None,
                        None,
                    );
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
