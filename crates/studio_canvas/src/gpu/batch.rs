use super::callback::GpuWireCallback;
use super::pipeline::{CanvasUniforms, GpuWireInstance, FLAG_ANIMATED, FLAG_GLOW, FLAG_STRAIGHT};
use egui::{Color32, Pos2, Rect};

/// Frame collector for GPU wire instances.
#[derive(Debug, Clone)]
pub struct GpuWireBatch {
    pub instances: Vec<GpuWireInstance>,
    /// Straight segments of routed wires, drawn with one quad each.
    pub straight: Vec<GpuWireInstance>,
    pub screen_size: [f32; 2],
    pub zoom: f32,
    pub anim_time: f32,
}

impl GpuWireBatch {
    pub fn new(screen_size: [f32; 2], zoom: f32, anim_time: f32) -> Self {
        Self { instances: Vec::with_capacity(512), straight: Vec::new(), screen_size, zoom, anim_time }
    }

    /// Adds a connection wire to the GPU render batch.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn push_wire(
        &mut self,
        p0: Pos2,
        p3: Pos2,
        color: Color32,
        glow_color: Option<Color32>,
        core_width: f32,
        glow_width: f32,
        is_animated: bool,
    ) {
        let mut flags = 0u32;
        let glow_rgba = if let Some(g) = glow_color {
            flags |= FLAG_GLOW;
            [g.r() as f32 / 255.0, g.g() as f32 / 255.0, g.b() as f32 / 255.0, g.a() as f32 / 255.0]
        } else {
            [0.0, 0.0, 0.0, 0.0]
        };

        if is_animated {
            flags |= FLAG_ANIMATED;
        }

        let color_rgba =
            [color.r() as f32 / 255.0, color.g() as f32 / 255.0, color.b() as f32 / 255.0, color.a() as f32 / 255.0];

        self.instances.push(GpuWireInstance {
            p0: [p0.x, p0.y],
            p3: [p3.x, p3.y],
            color: color_rgba,
            glow_color: glow_rgba,
            params: [core_width, glow_width, self.anim_time, flags as f32],
        });
    }

    /// Adds one straight segment of a routed wire.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn push_segment(
        &mut self,
        p0: Pos2,
        p3: Pos2,
        color: Color32,
        glow_color: Option<Color32>,
        core_width: f32,
        glow_width: f32,
        is_animated: bool,
    ) {
        self.push_wire(p0, p3, color, glow_color, core_width, glow_width, is_animated);
        let mut segment = self.instances.pop().expect("just pushed");
        segment.params[3] = (segment.params[3] as u32 | FLAG_STRAIGHT) as f32;
        self.straight.push(segment);
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty() && self.straight.is_empty()
    }

    pub fn len(&self) -> usize {
        self.instances.len() + self.straight.len()
    }

    /// Constructs an `egui::PaintCallback` targeting `wgpu`.
    pub fn into_paint_callback(self, viewport_rect: Rect) -> egui::PaintCallback {
        let uniforms = CanvasUniforms { screen_size: self.screen_size, zoom: self.zoom, anim_time: self.anim_time };
        let curves = self.instances.len() as u32;
        let mut instances = self.instances;
        instances.extend(self.straight);
        let callback = GpuWireCallback { uniforms, instances, curves };
        egui_wgpu::Callback::new_paint_callback(viewport_rect, callback)
    }
}
