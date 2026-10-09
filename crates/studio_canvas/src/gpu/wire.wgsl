// GPU Bezier Wire Shader for Studio Canvas
// Hardware Accelerated on NVIDIA RTX 3070 via WebGPU / WGSL

struct CanvasUniforms {
    screen_size: vec2f,
    zoom: f32,
    anim_time: f32,
};

@group(0) @binding(0)
var<uniform> uniforms: CanvasUniforms;

struct VertexOutput {
    @builtin(position) clip_pos: vec4f,
    @location(0) color: vec4f,
    @location(1) glow_color: vec4f,
    @location(2) core_radius: f32,
    @location(3) glow_radius: f32,
    @location(4) dist_from_center: f32,
    @location(5) curve_t: f32,
    @location(6) flags: f32,
};

const NUM_SEGMENTS: u32 = 64u;
const NUM_SEGMENTS_F: f32 = 64.0;

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_idx: u32,
    @location(0) p0: vec2f,
    @location(1) p3: vec2f,
    @location(2) color: vec4f,
    @location(3) glow_color: vec4f,
    @location(4) params: vec4f, // params: [core_width, glow_width, anim_time, flags]
) -> VertexOutput {
    // 64 segments per curve, 6 vertices (2 triangles) per segment = 384 vertices
    let seg_idx = vertex_idx / 6u;
    let quad_v = vertex_idx % 6u;

    // Determine whether this vertex is on the start (t0) or end (t1) of the segment quad
    let is_t1 = (quad_v == 2u || quad_v == 4u || quad_v == 5u);
    // Determine whether this vertex is on the left (+1.0) or right (-1.0) normal side
    let side = select(-1.0, 1.0, (quad_v == 0u || quad_v == 2u || quad_v == 5u));

    let t = select(f32(seg_idx) / NUM_SEGMENTS_F, f32(seg_idx + 1u) / NUM_SEGMENTS_F, is_t1);

    // Compute world-space span to ensure 100% scale-invariant curve geometry across all zoom levels
    let inv_zoom = 1.0 / max(uniforms.zoom, 1e-4);
    let dx_w = abs(p3.x - p0.x) * inv_zoom;
    let dy_w = abs(p3.y - p0.y) * inv_zoom;

    // Organic curvature evaluated purely in world space
    let min_tangent_w = 40.0;
    let base_tangent_w = dx_w * 0.38 + dy_w * 0.12;
    let max_tangent_w = 320.0;
    let tangent_w = clamp(base_tangent_w, min_tangent_w, max_tangent_w);

    // Screen-space tangent scales strictly linearly with zoom: T_screen = T_w * zoom
    let tangent_len = tangent_w * uniforms.zoom;
    let c1 = p0 + vec2f(tangent_len, 0.0);
    let c2 = p3 - vec2f(tangent_len, 0.0);

    // Evaluate cubic Bezier position P(t)
    let inv_t = 1.0 - t;
    let b0 = inv_t * inv_t * inv_t;
    let b1 = 3.0 * inv_t * inv_t * t;
    let b2 = 3.0 * inv_t * t * t;
    let b3 = t * t * t;
    let P = b0 * p0 + b1 * c1 + b2 * c2 + b3 * p3;

    // Evaluate cubic Bezier tangent T(t) = P'(t)
    let db0 = 3.0 * inv_t * inv_t;
    let db1 = 6.0 * inv_t * t;
    let db2 = 3.0 * t * t;
    let T = db0 * (c1 - p0) + db1 * (c2 - c1) + db2 * (p3 - c2);

    // Screen-space 2D normal vector perpendicular to tangent
    var normal = vec2f(0.0, 1.0);
    let len_T = length(T);
    if (len_T > 1e-4) {
        let unit_T = T / len_T;
        normal = vec2f(-unit_T.y, unit_T.x);
    }

    // Widths
    let core_width = params.x;
    let glow_width = params.y;
    let max_width = max(core_width, glow_width);
    let half_width = max_width * 0.5;
    let aa_margin = 1.0; // 1-pixel guard band for sub-pixel analytical anti-aliasing
    let total_half_width = half_width + aa_margin;

    // Offset along normal in screen space
    let vertex_screen = P + normal * (side * total_half_width);

    // Transform screen points into Normalized Device Coordinates (NDC)
    let ndc_x = (vertex_screen.x / uniforms.screen_size.x) * 2.0 - 1.0;
    let ndc_y = 1.0 - (vertex_screen.y / uniforms.screen_size.y) * 2.0;

    var out: VertexOutput;
    out.clip_pos = vec4f(ndc_x, ndc_y, 0.0, 1.0);
    out.color = color;
    out.glow_color = glow_color;
    out.core_radius = core_width * 0.5;
    out.glow_radius = glow_width * 0.5;
    out.dist_from_center = side * total_half_width;
    out.curve_t = t;
    out.flags = params.w;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4f {
    let dist = abs(in.dist_from_center);

    // 1. Sub-pixel analytical smooth anti-aliased edge (C1 continuous Hermite filter)
    let d_core = dist - in.core_radius;
    let aa_width = 0.65;
    let core_cov = smoothstep(aa_width, -aa_width, d_core);

    // 2. Analytical coverage and quadratic falloff for outer glow aura
    var glow_cov = 0.0;
    if (in.glow_radius > in.core_radius) {
        let d_glow = dist - in.glow_radius;
        let edge_glow_cov = smoothstep(1.0, -1.0, d_glow);
        let falloff = saturate(1.0 - dist / in.glow_radius);
        glow_cov = edge_glow_cov * (falloff * falloff);
    }

    // 3. Animated pulse particles on active / hovered wires
    var particle_boost = 0.0;
    let flags = u32(in.flags);
    if ((flags & 2u) != 0u) { // FLAG_ANIMATED
        let speed = 0.45;
        let t1 = fract(uniforms.anim_time * speed);
        let t2 = fract(uniforms.anim_time * speed + 0.5);
        let d1 = abs(in.curve_t - t1);
        let d2 = abs(in.curve_t - t2);
        let p_dist = min(d1, d2);
        let p_envelope = smoothstep(0.04, 0.0, p_dist);
        particle_boost = p_envelope * core_cov;
    }

    // 4. Composite core, glow aura, and pulse particles
    let core_col = in.color;
    let glow_col = in.glow_color;

    // Linear color blend
    let blended_rgb = mix(glow_col.rgb, core_col.rgb, core_cov) + vec3f(particle_boost);
    let blended_alpha = max(core_col.a * core_cov, glow_col.a * glow_cov) + particle_boost * 0.7;

    if (blended_alpha <= 0.001) {
        discard;
    }

    // Premultiplied alpha output matching egui framebuffer blending
    return vec4f(blended_rgb * blended_alpha, blended_alpha);
}
