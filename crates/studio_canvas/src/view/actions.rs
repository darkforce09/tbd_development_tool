use studio_graph::{FolderDetail, Graph, NodeArchetype};

use super::layout::calculate_file_node_size;
use super::types::{CanvasAction, CanvasState, ContextMenuAction, RenderEvents};
use crate::camera::{CameraTarget, Stop};

pub fn apply_render_events(events: RenderEvents, state: &mut CanvasState, graph: &mut Graph) {
    if let Some((cid, detail)) = events.folder_detail {
        let unloaded = graph.clusters.iter().any(|c| c.id == cid && c.is_collapsed() && c.lazy.is_some());
        if unloaded && detail != FolderDetail::Minimised {
            // The host loads the folder's contents, then shows it at that level.
            state.action_request = Some(CanvasAction::ExpandFolder(cid, detail));
        } else if !unloaded {
            graph.set_folder_detail(&cid, detail);
            state.mark_scene_dirty();
        }
    }

    if let Some(node_id) = events.file_dropdown_toggle_clicked {
        let new_bounds = if let Some(node) = graph.nodes.get_mut(&node_id) {
            node.is_dropdown_expanded = !node.is_dropdown_expanded;
            node.size = calculate_file_node_size(node);
            Some([node.position[0], node.position[1], node.position[0] + node.size[0], node.position[1] + node.size[1]])
        } else {
            None
        };
        if let Some(bounds) = new_bounds {
            if graph.tree_layout {
                // A taller card shifts its row and every folder below it.
                graph.relayout_resized(&[node_id]);
                state.mark_scene_dirty();
            } else {
                state.spatial_grid.update(node_id, bounds);
                graph.update_cluster_bounds();
                state.mark_scene_dirty();
            }
        }
    }

    if let Some(node_id) = events.file_wires_toggle_clicked {
        if let Some(node) = graph.nodes.get_mut(&node_id) {
            node.show_member_wires = !node.show_member_wires;
        }
    }

    if let Some((node_id, member_id)) = events.member_code_toggle_clicked {
        let new_bounds = if let Some(node) = graph.nodes.get_mut(&node_id) {
            node.expanded_member_id =
                if node.expanded_member_id.as_deref() == Some(&member_id) { None } else { Some(member_id) };
            node.size = calculate_file_node_size(node);
            Some([node.position[0], node.position[1], node.position[0] + node.size[0], node.position[1] + node.size[1]])
        } else {
            None
        };
        if let Some(bounds) = new_bounds {
            if graph.tree_layout {
                // A taller card shifts its row and every folder below it.
                graph.relayout_resized(&[node_id]);
                state.mark_scene_dirty();
            } else {
                state.spatial_grid.update(node_id, bounds);
                graph.update_cluster_bounds();
                state.mark_scene_dirty();
            }
        }
    }

    if let Some((node_id, member_id)) = events.member_row_clicked {
        state.selected_nodes.clear();
        state.selected_nodes.insert(node_id);
        state.action_request = Some(CanvasAction::InspectMember(node_id, member_id));
    }

    if let Some(node_id) = events.file_code_expand_clicked {
        state.action_request = Some(CanvasAction::ToggleCodeExpand(node_id));
    }

    if let Some(node_id) = events.markdown_preview_toggle_clicked {
        state.action_request = Some(CanvasAction::ToggleMarkdownPreview(node_id));
    }

    if let Some((node_id, tab_idx)) = events.node_tab_clicked {
        if let Some(node) = graph.nodes.get_mut(&node_id) {
            node.expanded_tab = tab_idx;
            node.scroll_offset_y = 0.0;
        }
    }

    if let Some(target_id) = events.port_jump_clicked {
        state.selected_nodes.clear();
        state.selected_nodes.insert(target_id);
        state.action_request = Some(CanvasAction::CenterNode(target_id));
    }

    if let Some((node_id, target_line)) = events.subnode_jump_clicked {
        if let Some(node) = graph.nodes.get_mut(&node_id) {
            let start_line = node.line_number.unwrap_or(1);
            let line_offset = target_line.saturating_sub(start_line) as f32;
            node.scroll_offset_y = (line_offset * 18.0).max(0.0);
        }
    }

    if let Some((node_id, member_id)) = events.member_fold_clicked {
        if !state.collapsed_subnodes.remove(&(node_id, member_id.clone())) {
            state.collapsed_subnodes.insert((node_id, member_id));
        }
    }

    if let Some((node_id, action)) = events.context_menu_action {
        state.context_menu = None;
        match action {
            ContextMenuAction::OpenOnDesk => {
                state.action_request = Some(CanvasAction::OpenOnDesk(node_id));
            }
            ContextMenuAction::ToggleExpand => {
                state.action_request = Some(CanvasAction::ToggleCodeExpand(node_id));
            }
            ContextMenuAction::ViewDocs => {
                if let Some(n) = graph.nodes.get_mut(&node_id) {
                    if !n.is_code_expanded {
                        state.action_request = Some(CanvasAction::ToggleCodeExpand(node_id));
                    }
                    n.expanded_tab = 1;
                    n.scroll_offset_y = 0.0;
                }
            }
            ContextMenuAction::Center => {
                state.action_request = Some(CanvasAction::CenterNode(node_id));
            }
            ContextMenuAction::ToggleWires => {
                state.action_request = Some(CanvasAction::ToggleMemberWires(node_id));
            }
            ContextMenuAction::CopyPath => {
                if let Some(node) = graph.nodes.get(&node_id) {
                    let path = node.file_path.as_deref().unwrap_or(&node.title);
                    state.status_message = Some(format!("Copied: {}", path));
                    state.copy_request = Some(path.to_string());
                }
            }
        }
    }

    if let Some(node_id) = events.code_close_clicked {
        let new_bounds = if let Some(node) = graph.nodes.get_mut(&node_id) {
            node.is_code_expanded = false;
            node.scroll_offset_y = 0.0;
            if node.archetype == NodeArchetype::File {
                node.size = calculate_file_node_size(node);
            } else {
                let desc_h = if !node.description.is_empty() { 24.0 } else { 0.0 };
                let max_ports = node.inputs.len().max(node.outputs.len()).max(1);
                node.size = [300.0, 34.0 + desc_h + (max_ports as f32 * 26.0) + 12.0];
            }
            Some([node.position[0], node.position[1], node.position[0] + node.size[0], node.position[1] + node.size[1]])
        } else {
            None
        };
        if let Some(bounds) = new_bounds {
            if graph.tree_layout {
                // A taller card shifts its row and every folder below it.
                graph.relayout_resized(&[node_id]);
                state.mark_scene_dirty();
            } else {
                state.spatial_grid.update(node_id, bounds);
                graph.update_cluster_bounds();
                state.mark_scene_dirty();
            }
        }
    }

    if let Some(action) = events.single_selected_action {
        state.action_request = Some(action);
    }

    // Handle internal canvas actions if any
    if let Some(action) = state.action_request.clone() {
        match action {
            CanvasAction::CenterNode(id) => {
                state.fly_to(CameraTarget::Node(id));
                state.action_request = None;
            }
            CanvasAction::ToggleCodeExpand(id) => {
                let new_bounds = if let Some(n) = graph.nodes.get_mut(&id) {
                    n.is_code_expanded = !n.is_code_expanded;
                    n.scroll_offset_y = 0.0;
                    if n.is_code_expanded {
                        // Lazily read from disk if source_code is None
                        if n.source_code.is_none() {
                            if let Some(ref fp) = n.file_path {
                                if let Ok(content) = std::fs::read_to_string(fp) {
                                    n.source_code = Some(content);
                                }
                            }
                        }
                        let line_count = n.source_code.as_ref().map(|s| s.lines().count()).unwrap_or(8);
                        let is_md = n.badge.as_deref() == Some("MD") || n.is_markdown_preview;
                        let card_w = if is_md { 580.0 } else { 540.0 };
                        let card_h = if is_md {
                            (line_count as f32 * 20.0 + 80.0).clamp(380.0, 720.0)
                        } else {
                            (line_count as f32 * 18.0 + 80.0).clamp(320.0, 640.0)
                        };
                        n.size = [card_w, card_h];
                    } else if n.archetype == NodeArchetype::File {
                        n.size = calculate_file_node_size(n);
                    } else {
                        let desc_h = if !n.description.is_empty() { 24.0 } else { 0.0 };
                        let max_ports = n.inputs.len().max(n.outputs.len()).max(1);
                        n.size = [300.0, 34.0 + desc_h + (max_ports as f32 * 26.0) + 12.0];
                    }
                    Some([n.position[0], n.position[1], n.position[0] + n.size[0], n.position[1] + n.size[1]])
                } else {
                    None
                };
                if let Some(bounds) = new_bounds {
                    if graph.tree_layout {
                        graph.relayout_resized(&[id]);
                        state.mark_scene_dirty();
                    } else {
                        graph.update_cluster_bounds();
                        state.mark_scene_dirty();
                        state.spatial_grid.update(id, bounds);
                    }
                }
                state.action_request = None;
            }
            CanvasAction::SetNodeTab(id, tab_idx) => {
                if let Some(n) = graph.nodes.get_mut(&id) {
                    n.expanded_tab = tab_idx;
                    n.scroll_offset_y = 0.0;
                }
                state.action_request = None;
            }
            CanvasAction::ToggleMarkdownPreview(id) => {
                if let Some(n) = graph.nodes.get_mut(&id) {
                    n.is_markdown_preview = !n.is_markdown_preview;
                }
                state.action_request = None;
            }
            CanvasAction::ToggleFileDropdown(id) => {
                let new_bounds = if let Some(n) = graph.nodes.get_mut(&id) {
                    n.is_dropdown_expanded = !n.is_dropdown_expanded;
                    n.size = calculate_file_node_size(n);
                    Some([n.position[0], n.position[1], n.position[0] + n.size[0], n.position[1] + n.size[1]])
                } else {
                    None
                };
                if let Some(bounds) = new_bounds {
                    if graph.tree_layout {
                        graph.relayout_resized(&[id]);
                        state.mark_scene_dirty();
                    } else {
                        graph.update_cluster_bounds();
                        state.mark_scene_dirty();
                        state.spatial_grid.update(id, bounds);
                    }
                }
                state.action_request = None;
            }
            CanvasAction::ToggleMemberWires(id) => {
                if let Some(n) = graph.nodes.get_mut(&id) {
                    n.show_member_wires = !n.show_member_wires;
                }
                state.action_request = None;
            }
            CanvasAction::ToggleMemberDrawer(id, mem_id) => {
                let new_bounds = if let Some(n) = graph.nodes.get_mut(&id) {
                    n.expanded_member_id =
                        if n.expanded_member_id.as_deref() == Some(&mem_id) { None } else { Some(mem_id) };
                    n.size = calculate_file_node_size(n);
                    Some([n.position[0], n.position[1], n.position[0] + n.size[0], n.position[1] + n.size[1]])
                } else {
                    None
                };
                if let Some(bounds) = new_bounds {
                    if graph.tree_layout {
                        graph.relayout_resized(&[id]);
                        state.mark_scene_dirty();
                    } else {
                        graph.update_cluster_bounds();
                        state.mark_scene_dirty();
                        state.spatial_grid.update(id, bounds);
                    }
                }
                state.action_request = None;
            }
            CanvasAction::OpenOnDesk(id) => {
                if let Some(path) = graph.nodes.get(&id).and_then(|n| n.file_path.clone()) {
                    state.desk.open(std::path::Path::new(&path), None);
                    state.desk.navigator.reveal(graph, id);
                    state.fly_to(CameraTarget::Stop(Stop::Desk));
                }
                state.action_request = None;
            }
            CanvasAction::FitGraph => {
                state.fly_to(CameraTarget::Stop(Stop::Code));
                state.action_request = None;
            }
            CanvasAction::ResetGraph => {
                state.fly_to(CameraTarget::Stop(Stop::World));
                state.action_request = None;
            }
            _ => {} // Other actions like InspectNode are handled by the parent viewer app
        }
    }
}
