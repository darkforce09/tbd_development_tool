use egui::Pos2;
use studio_graph::{Edge, Graph, Node, NodeArchetype, PortDirection, PortId};

/// Calculates the size of a File node based on whether its member dropdown or code drawer is expanded.
pub fn calculate_file_node_size(node: &Node) -> [f32; 2] {
    if !node.is_dropdown_expanded {
        [220.0, 42.0]
    } else {
        let header_h = 38.0;
        let divider_h = 2.0;
        let rows_h = if node.member_nodes.is_empty() { 24.0 } else { node.member_nodes.len() as f32 * 26.0 };

        let drawer_h = if let Some(ref mem_id) = node.expanded_member_id {
            if let Some(mem) = node.member_nodes.iter().find(|m| &m.id == mem_id) {
                let lines_count = mem.source_code.lines().count().clamp(1, 40);
                lines_count as f32 * 14.0 + 18.0
            } else {
                0.0
            }
        } else {
            0.0
        };

        let total_h = (header_h + divider_h + rows_h + drawer_h + 8.0).clamp(64.0, 780.0);
        [280.0, total_h]
    }
}

/// Distance of the documentation port below a card's top edge.
pub const DOC_PORT_OFFSET_Y: f32 = 9.0;

/// Computes the world position of a port socket on a node card.
pub fn port_world_position(node: &Node, port_id: PortId) -> Option<Pos2> {
    let (idx, dir) = node.port_index(port_id)?;
    // The documentation port sits at the top of the left side, above the code inputs.
    if node.doc_port() == Some(port_id) {
        return Some(Pos2::new(node.position[0], node.position[1] + DOC_PORT_OFFSET_Y));
    }
    let y = if node.archetype == NodeArchetype::File && !node.is_code_expanded {
        if node.is_dropdown_expanded {
            if let Some((m_idx, _)) = node
                .member_nodes
                .iter()
                .enumerate()
                .find(|(_, m)| m.in_port_id == Some(port_id) || m.out_port_id == Some(port_id))
            {
                let header_h = 38.0;
                let divider_h = 2.0;
                let mut prev_h = 0.0;
                for prev in &node.member_nodes[..m_idx] {
                    let drawer_h = if node.expanded_member_id.as_deref() == Some(&prev.id) {
                        let lines_count = prev.source_code.lines().count().clamp(1, 40);
                        lines_count as f32 * 14.0 + 18.0
                    } else {
                        0.0
                    };
                    prev_h += 26.0 + drawer_h;
                }
                node.position[1] + header_h + divider_h + 4.0 + prev_h + 12.0
            } else {
                node.position[1] + 19.0
            }
        } else {
            node.position[1] + (node.size[1] * 0.5)
        }
    } else if node.is_code_expanded {
        let header_h = 32.0;
        let tab_bar_h = 28.0;
        let content_top = node.position[1] + header_h + tab_bar_h;
        let line_h = 18.0;

        let member_match =
            node.member_nodes.iter().find(|m| m.in_port_id == Some(port_id) || m.out_port_id == Some(port_id));
        let ideal_y = if let Some(m) = member_match {
            let start_line = node.line_number.unwrap_or(1);
            let line_offset = (m.line_number.saturating_sub(start_line)) as f32;
            content_top + 8.0 + (line_offset * line_h) + (line_h * 0.5) - node.scroll_offset_y
        } else {
            content_top + 8.0 + (idx as f32 * 22.0) + 9.0 - node.scroll_offset_y
        };

        let min_y = content_top + 8.0;
        let max_y = (node.position[1] + node.size[1] - 12.0).max(min_y);
        ideal_y.clamp(min_y, max_y)
    } else {
        let desc_height = if !node.description.is_empty() { 24.0 } else { 0.0 };
        let body_top = node.position[1] + 34.0 + desc_height;
        body_top + (idx as f32 * 26.0) + 14.0
    };

    let w = node.size[0];
    let x = match dir {
        PortDirection::Input => node.position[0],
        PortDirection::Output => node.position[0] + w,
    };

    Some(Pos2::new(x, y))
}

/// World points of a routed wire, with its ends moved to the edge's actual ports (a member row
/// rather than the card's file-level port). `None` when the wire has no route.
pub fn routed_wire_points(graph: &Graph, edge: &Edge) -> Option<Vec<Pos2>> {
    let mut points = Vec::new();
    routed_wire_points_into(graph, edge, &mut points).then_some(points)
}

/// [`routed_wire_points`] into a reused buffer. Returns false (and leaves `points` empty) when the
/// wire has no route.
pub fn routed_wire_points_into(graph: &Graph, edge: &Edge, points: &mut Vec<Pos2>) -> bool {
    points.clear();
    if !edge.kind.is_code_flow() {
        return false;
    }
    let Some(route) = graph.route_for(edge.from_node, edge.to_node) else { return false };
    if route.points.len() < 2 {
        return false;
    }
    points.extend(route.points.iter().map(|p| Pos2::new(p[0], p[1])));
    // A card hidden in a collapsed folder is represented by that folder's gate, where the route
    // already ends.
    let port = |node, port| {
        (!graph.is_node_in_collapsed_cluster(node))
            .then(|| graph.nodes.get(&node).and_then(|n| port_world_position(n, port)))
            .flatten()
    };
    let start = port(edge.from_node, edge.from_port);
    let end = port(edge.to_node, edge.to_port);
    if points.len() == 2 {
        // A straight route: give the ends room to step to their rows halfway along.
        let mid = (points[0].x + points[1].x) * 0.5;
        let (y0, y1) = (points[0].y, points[1].y);
        points.insert(1, Pos2::new(mid, y0));
        points.insert(2, Pos2::new(mid, y1));
    }
    let n = points.len();
    if let Some(p) = start {
        points[0] = p;
        points[1].y = p.y;
    }
    if let Some(p) = end {
        points[n - 1] = p;
        points[n - 2].y = p.y;
    }
    true
}
