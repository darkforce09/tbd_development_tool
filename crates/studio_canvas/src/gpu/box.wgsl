// Rounded rects in world space: folder frames, cycle boxes, and cards seen from far away.

struct Uniforms {
    screen_size: vec2f,
    pan: vec2f,
    zoom: f32,
    time: f32,
    kind_mask: u32,
    flags: u32,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

struct VertexOutput {
    @builtin(position) clip_pos: vec4f,
    @location(0) local: vec2f,
    @location(1) half_size: vec2f,
    @location(2) radius: f32,
    @location(3) border_width: f32,
    @location(4) fill: vec4f,
    @location(5) border: vec4f,
};

fn hidden() -> VertexOutput {
    var out: VertexOutput;
    out.clip_pos = vec4f(2.0, 2.0, 2.0, 1.0);
    return out;
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_idx: u32,
    @location(0) min_world: vec2f,
    @location(1) size_world: vec2f,
    @location(2) fill: vec4f,
    @location(3) border: vec4f,
    @location(4) radius_world: f32,
    @location(5) border_world: f32,
) -> VertexOutput {
    let z = u.zoom;
    let smin = min_world * z + u.pan;
    let size = size_world * z;
    // A box without a border that is under half a pixel on screen is skipped.
    if (border_world <= 0.0 && (size.x < 0.5 || size.y < 0.5)) {
        return hidden();
    }
    if (smin.x + size.x < -2.0 || smin.y + size.y < -2.0 || smin.x > u.screen_size.x + 2.0 || smin.y > u.screen_size.y + 2.0) {
        return hidden();
    }
    var border_width = 0.0;
    if (border_world > 0.0 && border.a > 0.0) {
        border_width = clamp(border_world * z, 1.0, 2.5);
    }
    let corners = array<vec2f, 6>(
        vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0),
        vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0),
    );
    let pad = 1.0;
    let p = smin - vec2f(pad) + corners[vertex_idx % 6u] * (size + vec2f(2.0 * pad));
    let half_size = size * 0.5;

    var out: VertexOutput;
    out.clip_pos = vec4f(p.x / u.screen_size.x * 2.0 - 1.0, 1.0 - p.y / u.screen_size.y * 2.0, 0.0, 1.0);
    out.local = p - (smin + half_size);
    out.half_size = half_size;
    out.radius = min(radius_world * z, min(half_size.x, half_size.y));
    out.border_width = border_width;
    out.fill = fill;
    out.border = border;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    let q = abs(in.local) - in.half_size + vec2f(in.radius);
    let d = length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - in.radius;
    let outer = clamp(0.5 - d, 0.0, 1.0);
    let inner = clamp(0.5 - (d + in.border_width), 0.0, 1.0);
    let ring = outer - inner;
    let alpha = in.fill.a * inner + in.border.a * ring;
    if (alpha <= 0.001) {
        discard;
    }
    let rgb = in.fill.rgb * in.fill.a * inner + in.border.rgb * in.border.a * ring;
    return vec4f(rgb, alpha);
}
