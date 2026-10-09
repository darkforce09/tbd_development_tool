// Gate pins in world space: a filled circle with a ring, sized in screen pixels.

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
    @location(1) radius: f32,
    @location(2) ring_width: f32,
    @location(3) fill: vec4f,
    @location(4) ring: vec4f,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_idx: u32,
    @location(0) center_world: vec2f,
    @location(1) fill: vec4f,
    @location(2) ring: vec4f,
) -> VertexOutput {
    // Same sizes as `paint_pin_socket` at 80% of the zoom.
    let z = u.zoom * 0.8;
    let radius = 5.5 * clamp(z, 0.4, 2.0);
    let ring_width = clamp(1.5 * z, 1.0, 3.0);
    let center = center_world * u.zoom + u.pan;
    let corners = array<vec2f, 6>(
        vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(-1.0, 1.0),
        vec2f(-1.0, 1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
    );
    let extent = radius + ring_width * 0.5 + 1.0;
    let local = corners[vertex_idx % 6u] * extent;
    let p = center + local;

    var out: VertexOutput;
    out.clip_pos = vec4f(p.x / u.screen_size.x * 2.0 - 1.0, 1.0 - p.y / u.screen_size.y * 2.0, 0.0, 1.0);
    out.local = local;
    out.radius = radius;
    out.ring_width = ring_width;
    out.fill = fill;
    out.ring = ring;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    // The ring is centred on the circle's edge, like an egui stroke.
    let d = length(in.local) - in.radius;
    let outer = clamp(0.5 - (d - in.ring_width * 0.5), 0.0, 1.0);
    let inner = clamp(0.5 - (d + in.ring_width * 0.5), 0.0, 1.0);
    let ring = outer - inner;
    let alpha = in.fill.a * inner + in.ring.a * ring;
    if (alpha <= 0.001) {
        discard;
    }
    return vec4f(in.fill.rgb * in.fill.a * inner + in.ring.rgb * in.ring.a * ring, alpha);
}
