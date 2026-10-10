use egui::{epaint::CircleShape, Color32, Painter, Pos2, Stroke, Vec2};
use studio_ui::{color_tokens::*, with_alpha};

/// Evaluates a point along a cubic Bezier curve at parameter `t` in [0, 1].
#[inline]
pub fn eval_cubic_bezier(p0: Pos2, c1: Pos2, c2: Pos2, p3: Pos2, t: f32) -> Pos2 {
    let inv_t = 1.0 - t;
    let b0 = inv_t * inv_t * inv_t;
    let b1 = 3.0 * inv_t * inv_t * t;
    let b2 = 3.0 * inv_t * t * t;
    let b3 = t * t * t;

    Pos2::new(b0 * p0.x + b1 * c1.x + b2 * c2.x + b3 * p3.x, b0 * p0.y + b1 * c1.y + b2 * c2.y + b3 * p3.y)
}

/// Computes horizontal cubic Bezier control points with 100% scale-invariant world-space curvature.
pub fn compute_bezier_control_points(p0: Pos2, p3: Pos2, zoom: f32) -> (Pos2, Pos2) {
    let inv_zoom = 1.0 / zoom.max(1e-4);
    let dx_w = (p3.x - p0.x).abs() * inv_zoom;
    let dy_w = (p3.y - p0.y).abs() * inv_zoom;

    let min_tangent_w = 40.0;
    let base_tangent_w = dx_w * 0.38 + dy_w * 0.12;
    let max_tangent_w = 320.0;
    let tangent_w = base_tangent_w.clamp(min_tangent_w, max_tangent_w);

    let tangent_len = tangent_w * zoom;
    let c1 = p0 + Vec2::new(tangent_len, 0.0);
    let c2 = p3 - Vec2::new(tangent_len, 0.0);
    (c1, c2)
}

/// Visual properties for rendering an edge wire.
pub struct WireRenderProps<'a> {
    pub p0: Pos2,
    pub p3: Pos2,
    pub color: Color32,
    pub is_active: bool,
    pub is_hovered: bool,
    pub zoom: f32,
    pub anim_time: f64,
    pub label: Option<&'a str>,
    pub step_number: Option<usize>,
}

#[inline]
fn outcode(p: Pos2, rect: egui::Rect) -> u8 {
    let mut code = 0;
    if p.x < rect.min.x {
        code |= 1;
    } else if p.x > rect.max.x {
        code |= 2;
    }
    if p.y < rect.min.y {
        code |= 4;
    } else if p.y > rect.max.y {
        code |= 8;
    }
    code
}

#[inline]
fn segment_intersects_rect(p1: Pos2, p2: Pos2, rect: egui::Rect) -> bool {
    let out1 = outcode(p1, rect);
    let out2 = outcode(p2, rect);
    if (out1 & out2) != 0 {
        return false;
    }
    true
}

/// Paints a smooth cubic Bezier wire with viewport clipping, glow on hover/active, particles, and labels.
pub fn paint_bezier_wire(painter: &Painter, props: WireRenderProps<'_>) {
    let base_color = if props.is_hovered { WIRE_HOVER } else { props.color };

    let clip_rect = painter.clip_rect().expand(60.0);

    let (c1, c2) = compute_bezier_control_points(props.p0, props.p3, props.zoom);

    // Viewport-based frustum culling on the curve bounding box
    let min_x = props.p0.x.min(props.p3.x).min(c1.x).min(c2.x);
    let max_x = props.p0.x.max(props.p3.x).max(c1.x).max(c2.x);
    let min_y = props.p0.y.min(props.p3.y).min(c1.y).min(c2.y);
    let max_y = props.p0.y.max(props.p3.y).max(c1.y).max(c2.y);
    let curve_aabb = egui::Rect::from_min_max(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y));

    if !curve_aabb.intersects(clip_rect) {
        return; // Entire curve is off-screen! 0 CPU tessellation, 0 GPU work!
    }

    // Dynamic stroke widths
    let core_width = if props.is_hovered {
        (2.8 * props.zoom).clamp(1.5, 4.5)
    } else if props.zoom < 0.35 {
        (2.0 * props.zoom).clamp(1.2, 2.5)
    } else {
        (2.0 * props.zoom).clamp(1.5, 3.5)
    };
    let core_stroke = Stroke::new(core_width, base_color);

    // Outer glow ONLY drawn when hovered or active to eliminate alpha fill-rate thrashing
    let draw_glow = props.is_hovered || props.is_active;
    let glow_stroke = if draw_glow {
        Some(Stroke::new(
            (if props.is_hovered { 6.0 } else { 4.5 } * props.zoom).clamp(2.5, 8.0),
            with_alpha(base_color, if props.is_hovered { 90 } else { 40 }),
        ))
    } else {
        None
    };

    // Smooth budgeted polyline with segment viewport clipping (eliminates 30,000px off-screen tessellation)
    let num_segments = if props.zoom < 0.35 { 12 } else { 18 };
    let mut prev_pt = props.p0;

    for i in 1..=num_segments {
        let t = i as f32 / num_segments as f32;
        let pt = eval_cubic_bezier(props.p0, c1, c2, props.p3, t);

        if segment_intersects_rect(prev_pt, pt, clip_rect) {
            if let Some(glow) = glow_stroke {
                painter.line_segment([prev_pt, pt], glow);
            }
            painter.line_segment([prev_pt, pt], core_stroke);
        }

        prev_pt = pt;
    }

    // 3. Animated pulse particles along active or hovered wires
    if (props.is_active || props.is_hovered) && props.zoom >= 0.15 {
        let speed = 0.45; // loops per second
        let num_particles = 2;

        for i in 0..num_particles {
            let offset = i as f64 / num_particles as f64;
            let t = ((props.anim_time * speed + offset) % 1.0) as f32;

            let particle_pos = eval_cubic_bezier(props.p0, c1, c2, props.p3, t);

            if clip_rect.contains(particle_pos) {
                // Particle glow aura
                painter.add(CircleShape {
                    center: particle_pos,
                    radius: (4.5 * props.zoom).clamp(2.0, 7.0),
                    fill: with_alpha(base_color, 80),
                    stroke: Stroke::NONE,
                });

                // Particle bright center
                painter.add(CircleShape {
                    center: particle_pos,
                    radius: (2.2 * props.zoom).clamp(1.0, 4.0),
                    fill: Color32::WHITE,
                    stroke: Stroke::NONE,
                });
            }
        }
    }

    // 4. CodeSee Step Number Badge & Haystack Wire Label at Midpoint
    paint_wire_badge_and_label(painter, props.p0, props.p3, props.zoom, props.step_number, props.label);
}

/// Paints CodeSee step number badge and wire label pill at the curve midpoint.
pub fn paint_wire_badge_and_label(
    painter: &Painter,
    p0: Pos2,
    p3: Pos2,
    zoom: f32,
    step_number: Option<usize>,
    label: Option<&str>,
) {
    if zoom < 0.35 || (step_number.is_none() && label.is_none()) {
        return;
    }

    let clip_rect = painter.clip_rect().expand(60.0);
    let (c1, c2) = compute_bezier_control_points(p0, p3, zoom);
    let mid = eval_cubic_bezier(p0, c1, c2, p3, 0.5);
    if !clip_rect.contains(mid) {
        return;
    }

    // Step number circular badge (CodeSee 1, 2, 3...)
    if let Some(step) = step_number {
        let badge_radius = (9.0 * zoom).clamp(6.0, 15.0);
        painter.add(CircleShape {
            center: mid,
            radius: badge_radius + 1.5,
            fill: Color32::from_black_alpha(160),
            stroke: Stroke::NONE,
        });
        painter.add(CircleShape {
            center: mid,
            radius: badge_radius,
            fill: STEP_BADGE_BG,
            stroke: Stroke::new((1.2 * zoom).max(1.0), Color32::WHITE),
        });
        painter.text(
            mid,
            egui::Align2::CENTER_CENTER,
            format!("{}", step),
            egui::FontId::new((10.0 * zoom).max(6.0), egui::FontFamily::Monospace),
            STEP_BADGE_TEXT,
        );
    }

    // Edge label pill (e.g. function call name or type)
    if let Some(label_text) = label {
        let label_offset =
            if step_number.is_some() { Vec2::new(0.0, -14.0 * zoom) } else { Vec2::new(0.0, -8.0 * zoom) };
        let label_pos = mid + label_offset;
        let font_size = (9.5 * zoom).max(5.0);

        let text_len = label_text.chars().count() as f32;
        let pill_width = (text_len * 6.0 + 10.0) * zoom;
        let pill_height = 14.0 * zoom;
        let pill_rect = egui::Rect::from_center_size(label_pos, Vec2::new(pill_width, pill_height));

        painter.rect_filled(pill_rect, egui::CornerRadius::from(3.0 * zoom), Color32::from_black_alpha(180));
        painter.text(
            label_pos,
            egui::Align2::CENTER_CENTER,
            label_text,
            egui::FontId::new(font_size, egui::FontFamily::Proportional),
            TEXT_SECONDARY,
        );
    }
}

/// Calculates minimum distance from a query point to a cubic Bezier curve.
pub fn distance_to_bezier(point: Pos2, p0: Pos2, c1: Pos2, c2: Pos2, p3: Pos2) -> f32 {
    let samples = 20;
    let mut min_dist_sq = f32::MAX;

    let mut prev_pt = p0;
    for i in 1..=samples {
        let t = i as f32 / samples as f32;
        let curr_pt = eval_cubic_bezier(p0, c1, c2, p3, t);

        // Distance from point to line segment (prev_pt -> curr_pt)
        let v = curr_pt - prev_pt;
        let w = point - prev_pt;
        let c1_proj = w.x * v.x + w.y * v.y;
        let c2_proj = v.x * v.x + v.y * v.y;

        let closest = if c2_proj <= 1e-4 || c1_proj <= 0.0 {
            prev_pt
        } else if c1_proj >= c2_proj {
            curr_pt
        } else {
            let b = c1_proj / c2_proj;
            prev_pt + v * b
        };

        let dist_sq = (point.x - closest.x).powi(2) + (point.y - closest.y).powi(2);
        if dist_sq < min_dist_sq {
            min_dist_sq = dist_sq;
        }

        prev_pt = curr_pt;
    }

    min_dist_sq.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bezier_endpoints() {
        let p0 = Pos2::new(100.0, 100.0);
        let p3 = Pos2::new(300.0, 200.0);
        let (c1, c2) = compute_bezier_control_points(p0, p3, 1.0);

        let start = eval_cubic_bezier(p0, c1, c2, p3, 0.0);
        let end = eval_cubic_bezier(p0, c1, c2, p3, 1.0);

        assert!((start.x - p0.x).abs() < 1e-4);
        assert!((start.y - p0.y).abs() < 1e-4);
        assert!((end.x - p3.x).abs() < 1e-4);
        assert!((end.y - p3.y).abs() < 1e-4);
    }

    #[test]
    fn test_distance_to_bezier_on_curve() {
        let p0 = Pos2::new(100.0, 100.0);
        let p3 = Pos2::new(300.0, 200.0);
        let (c1, c2) = compute_bezier_control_points(p0, p3, 1.0);

        let midpoint = eval_cubic_bezier(p0, c1, c2, p3, 0.5);
        let dist = distance_to_bezier(midpoint, p0, c1, c2, p3);

        // Distance from a point strictly on the curve should be near zero
        assert!(dist < 2.0);
    }

    #[test]
    fn test_scale_invariant_bezier_shape() {
        let p0_w = Pos2::new(100.0, 100.0);
        let p3_w = Pos2::new(500.0, 300.0);

        // At zoom = 1.0:
        let (c1_1, c2_1) = compute_bezier_control_points(p0_w, p3_w, 1.0);
        let mid_1 = eval_cubic_bezier(p0_w, c1_1, c2_1, p3_w, 0.5);

        // At zoom = 0.02 (50x zoom out):
        let p0_002 = Pos2::new(p0_w.x * 0.02, p0_w.y * 0.02);
        let p3_002 = Pos2::new(p3_w.x * 0.02, p3_w.y * 0.02);
        let (c1_002, c2_002) = compute_bezier_control_points(p0_002, p3_002, 0.02);
        let mid_002 = eval_cubic_bezier(p0_002, c1_002, c2_002, p3_002, 0.5);

        // mid_002 must be EXACTLY mid_1 * 0.02 (affine scale-invariant)
        assert!((mid_002.x - mid_1.x * 0.02).abs() < 1e-4, "Curve shape X must be affine scale-invariant");
        assert!((mid_002.y - mid_1.y * 0.02).abs() < 1e-4, "Curve shape Y must be affine scale-invariant");
    }
}
