// Canvas wires in world space. The camera (pan, zoom) is a uniform, so the instance buffer is
// uploaded once per layout and a frame only rewrites the uniforms.

struct Uniforms {
    screen_size: vec2f,
    pan: vec2f,
    zoom: f32,
    time: f32,
    // Bit n set: wires of kind n are shown.
    kind_mask: u32,
    // Bit 0: an active flow is shown, so every other wire is dimmed.
    flags: u32,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

struct VertexOutput {
    @builtin(position) clip_pos: vec4f,
    @location(0) color: vec4f,
    @location(1) glow_color: vec4f,
    @location(2) core_radius: f32,
    @location(3) glow_radius: f32,
    @location(4) dist_from_center: f32,
    @location(5) curve_t: f32,
    @location(6) @interpolate(flat) flags: u32,
};

const CURVE_SEGMENTS: u32 = 32u;
const QUIET_ALPHA: f32 = 0.72;
const FLAG_CURVE: u32 = 16u;
const FLAG_HIGHLIGHT: u32 = 32u;
const FLAG_ACTIVE: u32 = 64u;
// Thin and fade: below FADE_START wires narrow towards a 1 px hairline and fade, reaching their
// faintest at FADE_END.
const FADE_START: f32 = 0.05;
const FADE_END: f32 = 0.004;
const FADE_MIN_ALPHA: f32 = 0.3;

fn hidden() -> VertexOutput {
    var out: VertexOutput;
    out.clip_pos = vec4f(2.0, 2.0, 2.0, 1.0);
    return out;
}

fn to_clip(p: vec2f) -> vec4f {
    return vec4f(p.x / u.screen_size.x * 2.0 - 1.0, 1.0 - p.y / u.screen_size.y * 2.0, 0.0, 1.0);
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_idx: u32,
    @location(0) p0_world: vec2f,
    @location(1) p1_world: vec2f,
    @location(2) color: vec4f,
    @location(3) flags: u32,
) -> VertexOutput {
    if ((u.kind_mask & (1u << (flags & 15u))) == 0u) {
        return hidden();
    }
    let curve = (flags & FLAG_CURVE) != 0u;
    let highlight = (flags & FLAG_HIGHLIGHT) != 0u;
    let in_flow = (flags & FLAG_ACTIVE) != 0u;
    let z = u.zoom;
    let s0 = p0_world * z + u.pan;
    let s1 = p1_world * z + u.pan;

    var core_width = clamp(1.9 * z, 1.4, 3.0);
    if (highlight) {
        core_width = clamp(3.0 * z, 2.0, 5.0);
    } else if (z < 0.35) {
        core_width = clamp(1.8 * z, 1.1, 2.1);
    }
    // A wire standing for many card pairs (bits 7-9: log2 of the count) is drawn thicker.
    let weight = f32((flags >> 7u) & 7u);
    core_width = core_width * (1.0 + 0.45 * weight);
    var alpha = color.a;
    if (!highlight && !in_flow) {
        // Wires are quieter than the structure until highlighted.
        alpha = alpha * QUIET_ALPHA;
        let near = smoothstep(FADE_END, FADE_START, z);
        core_width = mix(1.0, core_width, near);
        alpha = alpha * mix(FADE_MIN_ALPHA, 1.0, near);
        if ((u.flags & 1u) != 0u) {
            alpha = alpha * (45.0 / 255.0);
        }
    }
    var glow_width = 0.0;
    var glow_alpha = 0.0;
    if (highlight) {
        glow_width = clamp(6.0 * z, 2.5, 8.0);
        glow_alpha = 90.0 / 255.0;
    } else if (in_flow) {
        glow_width = clamp(4.5 * z, 2.5, 8.0);
        glow_alpha = 40.0 / 255.0;
    }
    let half_width = max(core_width, glow_width) * 0.5 + 1.0;

    var c1 = mix(s0, s1, 1.0 / 3.0);
    var c2 = mix(s0, s1, 2.0 / 3.0);
    if (curve) {
        let span = abs(p1_world - p0_world);
        let tangent = clamp(span.x * 0.38 + span.y * 0.12, 40.0, 320.0) * z;
        c1 = s0 + vec2f(tangent, 0.0);
        c2 = s1 - vec2f(tangent, 0.0);
    }

    // Off screen, or a straight piece shorter than a quarter pixel: nothing to draw.
    let lo = min(min(s0, s1), min(c1, c2)) - vec2f(half_width);
    let hi = max(max(s0, s1), max(c1, c2)) + vec2f(half_width);
    if (hi.x < 0.0 || hi.y < 0.0 || lo.x > u.screen_size.x || lo.y > u.screen_size.y) {
        return hidden();
    }
    if (!curve && !highlight && !in_flow && distance(s0, s1) < 0.25) {
        return hidden();
    }

    let seg_idx = vertex_idx / 6u;
    let quad_v = vertex_idx % 6u;
    let is_t1 = (quad_v == 2u || quad_v == 4u || quad_v == 5u);
    let side = select(-1.0, 1.0, (quad_v == 0u || quad_v == 2u || quad_v == 5u));

    var p = s0;
    var tangent_dir = s1 - s0;
    var t = select(0.0, 1.0, is_t1);
    if (curve) {
        t = select(f32(seg_idx), f32(seg_idx + 1u), is_t1) / f32(CURVE_SEGMENTS);
        let inv_t = 1.0 - t;
        p = inv_t * inv_t * inv_t * s0 + 3.0 * inv_t * inv_t * t * c1 + 3.0 * inv_t * t * t * c2 + t * t * t * s1;
        tangent_dir = 3.0 * inv_t * inv_t * (c1 - s0) + 6.0 * inv_t * t * (c2 - c1) + 3.0 * t * t * (s1 - c2);
    } else {
        p = mix(s0, s1, t);
    }

    var normal = vec2f(0.0, 1.0);
    var along = vec2f(1.0, 0.0);
    let len_t = length(tangent_dir);
    if (len_t > 1e-4) {
        along = tangent_dir / len_t;
        normal = vec2f(-along.y, along.x);
    }
    if (!curve) {
        // Square caps, so the corners of an orthogonal route close.
        p = p + along * select(-1.0, 1.0, is_t1) * core_width * 0.5;
    }

    var out: VertexOutput;
    out.clip_pos = to_clip(p + normal * (side * half_width));
    out.color = vec4f(color.rgb, alpha);
    out.glow_color = vec4f(color.rgb, glow_alpha);
    out.core_radius = core_width * 0.5;
    out.glow_radius = glow_width * 0.5;
    out.dist_from_center = side * half_width;
    out.curve_t = t;
    out.flags = flags;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    let dist = abs(in.dist_from_center);
    let core_cov = smoothstep(0.65, -0.65, dist - in.core_radius);

    var glow_cov = 0.0;
    if (in.glow_radius > in.core_radius) {
        let edge = smoothstep(1.0, -1.0, dist - in.glow_radius);
        let falloff = saturate(1.0 - dist / in.glow_radius);
        glow_cov = edge * falloff * falloff;
    }

    // Pulses moving along highlighted and active-flow wires.
    var pulse = 0.0;
    if ((in.flags & (FLAG_HIGHLIGHT | FLAG_ACTIVE)) != 0u) {
        let t1 = fract(u.time * 0.45);
        let t2 = fract(u.time * 0.45 + 0.5);
        let d = min(abs(in.curve_t - t1), abs(in.curve_t - t2));
        pulse = smoothstep(0.04, 0.0, d) * core_cov;
    }

    let rgb = mix(in.glow_color.rgb, in.color.rgb, core_cov) + vec3f(pulse);
    let alpha = max(in.color.a * core_cov, in.glow_color.a * glow_cov) + pulse * 0.7;
    if (alpha <= 0.001) {
        discard;
    }
    return vec4f(rgb * alpha, alpha);
}
