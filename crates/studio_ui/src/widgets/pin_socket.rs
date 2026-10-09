use egui::{epaint::CircleShape, Color32, Painter, Pos2, Stroke};

use crate::colors::*;

/// Socket visual state for rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SocketVisualState {
    pub is_hovered: bool,
    pub is_connected: bool,
    pub is_snapped: bool,
}

/// Paint a pin socket at `center` with the given data type color, state, and zoom.
pub fn paint_pin_socket(painter: &Painter, center: Pos2, color: Color32, state: SocketVisualState, zoom: f32) {
    let base_radius = 5.5 * zoom.clamp(0.4, 2.0);
    let radius = if state.is_hovered || state.is_snapped { base_radius * 1.35 } else { base_radius };

    // Outer glow aura when hovered or snapped
    if state.is_hovered || state.is_snapped {
        let aura_radius = radius + (3.5 * zoom).clamp(2.0, 7.0);
        painter.add(CircleShape { center, radius: aura_radius, fill: with_alpha(color, 65), stroke: Stroke::NONE });
    }

    // Outer ring rim
    let ring_stroke = if state.is_hovered || state.is_snapped {
        Stroke::new((1.8 * zoom).clamp(1.2, 3.5), SOCKET_RING_HOVER)
    } else {
        Stroke::new((1.5 * zoom).clamp(1.0, 3.0), SOCKET_RING_IDLE)
    };

    let fill_color = if state.is_hovered || state.is_snapped || state.is_connected { color } else { CARD_BG };

    painter.add(CircleShape { center, radius, fill: fill_color, stroke: ring_stroke });

    // If disconnected and not hovered, draw a subtle inner accent dot if socket is large enough
    if !state.is_connected && !state.is_hovered && !state.is_snapped && radius >= 3.0 {
        painter.add(CircleShape { center, radius: radius * 0.42, fill: color, stroke: Stroke::NONE });
    }
}
