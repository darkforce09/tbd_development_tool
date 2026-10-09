use egui::{CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Stroke, Vec2};
use studio_graph::{Graph, NodeArchetype, NodeId};
use studio_ui::{
    color_tokens::*, paint_card_frame, paint_code_card, paint_file_card, paint_pin_socket, truncate_with_ellipsis,
    with_alpha, CardFrameProps, CodeCardProps, FileCardMember, FileCardProps, PortDisplayInfo, SocketVisualState,
};

use crate::interaction::InteractionMode;

use super::layout::port_world_position;
use super::types::{archetype_color, data_type_color, CanvasState, RenderEvents};

fn build_members_ui<'a>(member_nodes: &'a [studio_graph::FileMemberNode]) -> Vec<FileCardMember<'a>> {
    member_nodes
        .iter()
        .map(|m| FileCardMember {
            id: &m.id,
            name: &m.name,
            archetype_tag: if m.visibility == "MOD"
                || m.visibility == "LNK"
                || m.visibility == "CLS"
                || m.visibility == "CODE"
                || m.visibility.starts_with('H')
            {
                &m.visibility
            } else {
                match m.archetype {
                    NodeArchetype::Function => "FN",
                    NodeArchetype::Struct => "STR",
                    NodeArchetype::Enum => "ENM",
                    NodeArchetype::Trait => "TRT",
                    NodeArchetype::Module => "MOD",
                    _ => "ITEM",
                }
            },
            archetype_color: if m.visibility == "MOD" {
                egui::Color32::from_rgb(0xf9, 0x73, 0x16)
            } else if m.visibility.starts_with('H') {
                egui::Color32::from_rgb(0x2d, 0xd4, 0xbf)
            } else {
                archetype_color(m.archetype)
            },
            visibility: &m.visibility,
            signature: &m.signature,
            line_number: m.line_number,
            source_code: &m.source_code,
        })
        .collect()
}

/// Whether a card is dimmed because it does not match the search (`query`, lowercase) or the
/// category filter.
pub fn is_dimmed(
    node: &studio_graph::Node,
    query: &str,
    categories: &std::collections::BTreeSet<NodeArchetype>,
    trace: Option<&std::collections::BTreeSet<studio_graph::NodeId>>,
) -> bool {
    if trace.is_some_and(|t| !t.contains(&node.id)) {
        return true;
    }
    let matches_search = query.is_empty()
        || node.title.to_lowercase().contains(query)
        || node.description.to_lowercase().contains(query)
        || node.crate_name.as_deref().is_some_and(|c| c.to_lowercase().contains(query));
    let matches_category = categories.is_empty() || categories.contains(&node.archetype);
    !matches_search || !matches_category
}

/// The first wire on a port, through the port index.
fn first_port_edge(graph: &Graph, node: NodeId, port: studio_graph::PortId) -> Option<&studio_graph::Edge> {
    graph.port_edges.get(&(node, port)).and_then(|edges| edges.first()).and_then(|&e| graph.get_edge(e))
}

#[allow(clippy::too_many_arguments)]
pub fn render_nodes_and_sockets(
    painter: &Painter,
    state: &CanvasState,
    graph: &Graph,
    visible_node_ids: &[NodeId],
    pointer_pos: Pos2,
    pointer_clicked: bool,
    interactive_rects: &mut Vec<Rect>,
    events: &mut RenderEvents,
) -> Option<(NodeId, Rect)> {
    let zoom = state.transform.zoom;
    let search_query = state.search_filter.trim().to_lowercase();
    let mut single_selected_rect = None;

    // Layer D: Node Cards
    for &node_id in visible_node_ids {
        if graph.is_node_in_collapsed_cluster(node_id) {
            continue;
        }
        let node = match graph.nodes.get(&node_id) {
            Some(n) => n,
            None => continue,
        };

        let min_world = Pos2::new(node.position[0], node.position[1]);
        let max_world = Pos2::new(node.position[0] + node.size[0], node.position[1] + node.size[1]);
        let min_screen = state.transform.world_to_screen(min_world);
        let max_screen = state.transform.world_to_screen(max_world);
        let card_rect = Rect::from_min_max(min_screen, max_screen);

        let is_selected = state.selected_nodes.contains(&node_id);
        if is_selected && state.selected_nodes.len() == 1 {
            single_selected_rect = Some((node_id, card_rect));
        }

        let is_hovered = state.hover.hovered_node == Some(node_id);

        let is_dimmed = is_dimmed(node, &search_query, &state.category_filter, state.trace_nodes.as_ref());
        let arch_color = archetype_color(node.archetype);

        if zoom < 0.03 {
            // LOD 3: Galaxy Overview (skip sub-half-pixel cards, draw micro colored rects for visible ones)
            if card_rect.width() >= 0.5 && card_rect.height() >= 0.5 {
                let fill = if is_selected {
                    CARD_BORDER_SELECTED
                } else if is_dimmed {
                    with_alpha(arch_color, 30)
                } else {
                    with_alpha(arch_color, 160)
                };
                painter.rect_filled(card_rect, CornerRadius::ZERO, fill);
            }
        } else if zoom < 0.12 {
            // LOD 2: Micro/Far Zoom
            let fill = if is_selected {
                CARD_BORDER_SELECTED
            } else if is_dimmed {
                with_alpha(arch_color, 40)
            } else {
                with_alpha(arch_color, 190)
            };
            painter.rect_filled(card_rect, CornerRadius::from(2.0), fill);
        } else if zoom < 0.35 {
            // LOD 1: Medium Zoom
            let fill = if is_hovered { CARD_BG_HOVER } else { CARD_BG };
            let border =
                if is_selected { Stroke::new(1.5, CARD_BORDER_SELECTED) } else { Stroke::new(1.0, CARD_BORDER_NORMAL) };
            painter.rect(card_rect, CornerRadius::from(4.0), fill, border, egui::StrokeKind::Middle);

            let header_h = (4.0 * zoom).max(1.5);
            let header_rect = Rect::from_min_size(card_rect.min, Vec2::new(card_rect.width(), header_h));
            painter.rect_filled(header_rect, CornerRadius::from(2.0), arch_color);

            let font_size = (11.0 * zoom).max(6.0);
            painter.text(
                card_rect.min + Vec2::new(6.0 * zoom, 8.0 * zoom),
                egui::Align2::LEFT_TOP,
                truncate_with_ellipsis(&node.title, 24),
                FontId::new(font_size, FontFamily::Proportional),
                if is_dimmed { TEXT_DIM } else { TEXT_PRIMARY },
            );

            for port in node.inputs.iter().chain(node.outputs.iter()) {
                if let Some(wpos) = port_world_position(node, port.id) {
                    let spos = state.transform.world_to_screen(wpos);
                    let port_color = data_type_color(&port.data_type);
                    painter.circle_filled(spos, (3.5 * zoom).max(1.5), port_color);
                }
            }
        } else {
            // LOD 0: Close-up Zoom
            let node_painter = if is_dimmed { painter.with_clip_rect(card_rect) } else { painter.clone() };

            if node.is_code_expanded {
                let code_text = node.source_code.as_deref().unwrap_or(&node.description);
                let start_line = node.line_number.unwrap_or(1);

                let members_ui = build_members_ui(&node.member_nodes);

                let input_infos: Vec<PortDisplayInfo> = node
                    .inputs
                    .iter()
                    .map(|p| {
                        let connected_edge = first_port_edge(graph, node_id, p.id);
                        let connected_node = connected_edge.and_then(|e| graph.nodes.get(&e.from_node));
                        PortDisplayInfo {
                            id: p.id.0,
                            name: &p.name,
                            type_name: p.data_type.display_name(),
                            type_color: data_type_color(&p.data_type),
                            connected_node_title: connected_node.map(|n| n.title.clone()),
                            connected_node_id: connected_edge.map(|e| e.from_node.0),
                        }
                    })
                    .collect();

                let output_infos: Vec<PortDisplayInfo> = node
                    .outputs
                    .iter()
                    .map(|p| {
                        let connected_edge = first_port_edge(graph, node_id, p.id);
                        let connected_node = connected_edge.and_then(|e| graph.nodes.get(&e.to_node));
                        PortDisplayInfo {
                            id: p.id.0,
                            name: &p.name,
                            type_name: p.data_type.display_name(),
                            type_color: data_type_color(&p.data_type),
                            connected_node_title: connected_node.map(|n| n.title.clone()),
                            connected_node_id: connected_edge.map(|e| e.to_node.0),
                        }
                    })
                    .collect();

                let collapsed_set: std::collections::BTreeSet<String> = state
                    .collapsed_subnodes
                    .iter()
                    .filter(|(nid, _)| *nid == node_id)
                    .map(|(_, mem_id)| mem_id.clone())
                    .collect();

                let code_layout = paint_code_card(
                    &node_painter,
                    CodeCardProps {
                        rect: card_rect,
                        title: &node.title,
                        file_path: node.file_path.as_deref(),
                        start_line,
                        code: code_text,
                        doc_comment: node.doc_comment.as_deref(),
                        is_selected,
                        zoom,
                        language_hint: node.badge.as_deref(),
                        active_tab: node.expanded_tab,
                        is_markdown_preview: node.is_markdown_preview,
                        members: &members_ui,
                        expanded_member_id: node.expanded_member_id.as_deref(),
                        collapsed_subnodes: Some(&collapsed_set),
                        inputs: &input_infos,
                        outputs: &output_infos,
                        hovered_pos: Some(pointer_pos),
                        scroll_y: node.scroll_offset_y,
                    },
                );

                interactive_rects.push(code_layout.close_button_rect);
                if let Some(toggle_r) = code_layout.preview_toggle_rect {
                    interactive_rects.push(toggle_r);
                }
                interactive_rects.extend(code_layout.tab_rects.iter().map(|(r, _)| *r));
                interactive_rects.extend(code_layout.subnode_clicks.iter().map(|(r, _)| *r));
                interactive_rects.extend(code_layout.member_fold_clicks.iter().map(|(r, _)| *r));
                interactive_rects.extend(code_layout.jump_clicks.iter().map(|(r, _)| *r));
                interactive_rects.extend(code_layout.member_code_clicks.iter().map(|(r, _)| *r));
                interactive_rects.extend(code_layout.member_row_clicks.iter().map(|(r, _)| *r));

                if pointer_clicked {
                    if code_layout.close_button_rect.contains(pointer_pos) {
                        events.code_close_clicked = Some(node_id);
                    } else if let Some(toggle_r) = code_layout.preview_toggle_rect {
                        if toggle_r.contains(pointer_pos) {
                            events.markdown_preview_toggle_clicked = Some(node_id);
                        }
                    } else {
                        let mut handled = false;
                        for (tab_rect, tab_idx) in code_layout.tab_rects {
                            if tab_rect.contains(pointer_pos) {
                                events.node_tab_clicked = Some((node_id, tab_idx));
                                handled = true;
                                break;
                            }
                        }
                        if !handled {
                            for (fold_rect, mem_id) in code_layout.member_fold_clicks {
                                if fold_rect.contains(pointer_pos) {
                                    events.member_fold_clicked = Some((node_id, mem_id));
                                    handled = true;
                                    break;
                                }
                            }
                        }
                        if !handled {
                            for (sub_rect, target_line) in code_layout.subnode_clicks {
                                if sub_rect.contains(pointer_pos) {
                                    events.subnode_jump_clicked = Some((node_id, target_line));
                                    handled = true;
                                    break;
                                }
                            }
                        }
                        if !handled {
                            for (jump_rect, target_nid) in code_layout.jump_clicks {
                                if jump_rect.contains(pointer_pos) {
                                    events.port_jump_clicked = Some(NodeId(target_nid));
                                    handled = true;
                                    break;
                                }
                            }
                        }
                        if !handled {
                            for (code_btn_rect, mem_id) in code_layout.member_code_clicks {
                                if code_btn_rect.contains(pointer_pos) {
                                    events.member_code_toggle_clicked = Some((node_id, mem_id));
                                    handled = true;
                                    break;
                                }
                            }
                        }
                        if !handled {
                            for (row_rect, mem_id) in code_layout.member_row_clicks {
                                if row_rect.contains(pointer_pos) {
                                    events.member_row_clicked = Some((node_id, mem_id));
                                    break;
                                }
                            }
                        }
                    }
                }
            } else if node.archetype == NodeArchetype::File {
                let ext = node.badge.as_deref().unwrap_or("rs");
                let step_number = node
                    .outputs
                    .iter()
                    .flat_map(|p| graph.port_edges.get(&(node_id, p.id)).into_iter().flatten())
                    .filter_map(|&e| graph.get_edge(e))
                    .find_map(|e| e.step_number);

                let members_ui = build_members_ui(&node.member_nodes);

                let file_layout = paint_file_card(
                    &node_painter,
                    FileCardProps {
                        rect: card_rect,
                        title: &node.title,
                        extension: ext,
                        item_count: node.member_nodes.len(),
                        line_count: node.line_number,
                        is_selected,
                        is_hovered,
                        is_dropdown_expanded: node.is_dropdown_expanded,
                        is_code_expanded: node.is_code_expanded,
                        show_member_wires: node.show_member_wires,
                        members: &members_ui,
                        expanded_member_id: node.expanded_member_id.as_deref(),
                        has_docs: node.doc_comment.is_some(),
                        step_number,
                        zoom,
                        hovered_pos: Some(pointer_pos),
                    },
                );

                interactive_rects.push(file_layout.dropdown_button_rect);
                interactive_rects.push(file_layout.code_expand_button_rect);
                interactive_rects.extend(file_layout.member_code_clicks.iter().map(|(r, _)| *r));
                interactive_rects.extend(file_layout.member_row_clicks.iter().map(|(r, _)| *r));

                if pointer_clicked {
                    if file_layout.dropdown_button_rect.contains(pointer_pos) {
                        events.file_dropdown_toggle_clicked = Some(node_id);
                    } else if file_layout.code_expand_button_rect.contains(pointer_pos) {
                        events.file_code_expand_clicked = Some(node_id);
                    } else {
                        let mut handled_member = false;
                        for (code_btn_rect, mem_id) in file_layout.member_code_clicks {
                            if code_btn_rect.contains(pointer_pos) {
                                events.member_code_toggle_clicked = Some((node_id, mem_id));
                                handled_member = true;
                                break;
                            }
                        }
                        if !handled_member {
                            for (row_rect, mem_id) in file_layout.member_row_clicks {
                                if row_rect.contains(pointer_pos) {
                                    events.member_row_clicked = Some((node_id, mem_id));
                                    break;
                                }
                            }
                        }
                    }
                }
            } else {
                paint_card_frame(
                    &node_painter,
                    CardFrameProps {
                        rect: card_rect,
                        title: &node.title,
                        archetype_label: node.archetype.label(),
                        archetype_color: arch_color,
                        description: &node.description,
                        badge: node.badge.as_deref(),
                        is_selected,
                        is_hovered,
                        zoom,
                    },
                );

                if !is_dimmed {
                    let card_painter = painter.with_clip_rect(card_rect);
                    let has_both = !node.inputs.is_empty() && !node.outputs.is_empty();
                    let port_font_size = (10.5 * zoom).max(3.0);
                    let char_width_approx = (6.2 * zoom).max(2.0);
                    let x_offset = 12.0 * zoom;

                    for (idx, port) in node.inputs.iter().enumerate() {
                        if let Some(wpos) = port_world_position(node, port.id) {
                            let spos = state.transform.world_to_screen(wpos);
                            let label_pos = spos + Vec2::new(x_offset, 0.0);
                            let port_color = data_type_color(&port.data_type);
                            let is_port_hovered = state.hover.hovered_port == Some((node_id, port.id));

                            let max_width = if has_both && idx < node.outputs.len() {
                                (card_rect.width() * 0.52) - 18.0 * zoom
                            } else {
                                card_rect.width() - 28.0 * zoom
                            };
                            let max_chars = ((max_width / char_width_approx) as usize).max(4);

                            card_painter.text(
                                label_pos,
                                egui::Align2::LEFT_CENTER,
                                truncate_with_ellipsis(&port.name, max_chars),
                                FontId::new(port_font_size, FontFamily::Proportional),
                                if is_port_hovered { TEXT_HIGHLIGHT } else { with_alpha(port_color, 210) },
                            );
                        }
                    }

                    for (idx, port) in node.outputs.iter().enumerate() {
                        if let Some(wpos) = port_world_position(node, port.id) {
                            let spos = state.transform.world_to_screen(wpos);
                            let label_pos = spos + Vec2::new(-x_offset, 0.0);
                            let port_color = data_type_color(&port.data_type);
                            let is_port_hovered = state.hover.hovered_port == Some((node_id, port.id));

                            let max_width = if has_both && idx < node.inputs.len() {
                                (card_rect.width() * 0.45) - 18.0 * zoom
                            } else {
                                card_rect.width() - 28.0 * zoom
                            };
                            let max_chars = ((max_width / char_width_approx) as usize).max(4);

                            card_painter.text(
                                label_pos,
                                egui::Align2::RIGHT_CENTER,
                                truncate_with_ellipsis(&port.name, max_chars),
                                FontId::new(port_font_size, FontFamily::Proportional),
                                if is_port_hovered { TEXT_HIGHLIGHT } else { with_alpha(port_color, 210) },
                            );
                        }
                    }
                }
            }
        }
    }

    // Layer E: Pin Sockets
    if zoom >= 0.35 {
        for &node_id in visible_node_ids {
            if graph.is_node_in_collapsed_cluster(node_id) {
                continue;
            }
            if let Some(node) = graph.nodes.get(&node_id) {
                for port in node.inputs.iter().chain(node.outputs.iter()) {
                    let is_member_port = node
                        .member_nodes
                        .iter()
                        .any(|m| m.in_port_id == Some(port.id) || m.out_port_id == Some(port.id));
                    if is_member_port
                        && (!node.is_dropdown_expanded
                            || (!node.show_member_wires && !state.show_subnode_wires_globally))
                    {
                        continue;
                    }
                    let is_connected = graph.is_port_connected(node_id, port.id);
                    // An unused documentation port is noise: show it only when documentation links in.
                    if port.data_type == studio_graph::DataType::Documentation && !is_connected {
                        continue;
                    }
                    if let Some(wpos) = port_world_position(node, port.id) {
                        let spos = state.transform.world_to_screen(wpos);
                        let is_hovered = state.hover.hovered_port == Some((node_id, port.id));
                        let is_snapped =
                            if let InteractionMode::Connecting { snapped_target: Some((sn_id, sp_id)), .. } =
                                state.interaction
                            {
                                sn_id == node_id && sp_id == port.id
                            } else {
                                false
                            };

                        let port_color = data_type_color(&port.data_type);

                        paint_pin_socket(
                            painter,
                            spos,
                            port_color,
                            SocketVisualState { is_hovered, is_connected, is_snapped },
                            zoom,
                        );
                    }
                }
            }
        }
    }

    single_selected_rect
}
