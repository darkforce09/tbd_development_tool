//! GPU drawing of the canvas scene (see `crate::scene`). Scene buffers are uploaded when the
//! scene's revision changes; every other frame only the camera uniforms are written.

use std::ops::Range;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use egui::PaintCallbackInfo;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use wgpu::util::DeviceExt;

use crate::scene::{BoxInstance, CanvasScene, PinInstance, WireInstance};

/// Vertices drawn per curve: 32 segments, two triangles each.
pub const VERTICES_PER_CURVE: u32 = 32 * 6;
/// Vertices drawn per straight segment, box or pin: one quad.
pub const VERTICES_PER_QUAD: u32 = 6;

/// Camera and switches shared by every canvas shader (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct SceneUniforms {
    /// Window size in points (the viewport covers the whole window).
    pub screen_size: [f32; 2],
    /// Screen position of world (0, 0), in points.
    pub pan: [f32; 2],
    pub zoom: f32,
    /// Seconds, for pulses along highlighted wires.
    pub time: f32,
    /// Bit n set: wires of kind n (`EdgeKind as u32`) are shown.
    pub kind_mask: u32,
    /// Bit 0: dim every wire that is not highlighted (an active flow is shown).
    pub flags: u32,
}

/// Cards drawn as rects from far away, with the revision they were built at.
#[derive(Debug, Clone)]
pub struct CardLayer {
    pub revision: u64,
    pub boxes: Arc<Vec<BoxInstance>>,
}

/// What one canvas frame draws on the GPU. Shared by the frame's paint callbacks.
#[derive(Debug)]
pub struct CanvasFrame {
    /// Unique per frame, so the per-frame highlight layer is uploaded once.
    pub id: u64,
    pub uniforms: SceneUniforms,
    pub scene: Arc<CanvasScene>,
    /// Ranges of `scene.segments` in visible tiles.
    pub wire_ranges: Vec<Range<u32>>,
    /// Ranges of `scene.curves` in visible tiles.
    pub curve_ranges: Vec<Range<u32>>,
    pub show_wires: bool,
    /// Highlighted wires: straight segments first, then curves from `overlay_curves_start`.
    pub overlay: Vec<WireInstance>,
    pub overlay_curves_start: u32,
    /// Folder frames come from the GPU (far zoom); otherwise egui paints them.
    pub folders: bool,
    pub gates: bool,
    pub cards: Option<CardLayer>,
}

/// Which part of the frame a paint callback draws. Separate callbacks keep the draw order
/// interleaved with egui's own shapes (folder labels, cards).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasLayer {
    Folders,
    CycleBoxes,
    Wires,
    Gates,
    Cards,
}

/// One paint callback of a canvas frame.
pub struct CanvasPaint {
    pub layer: CanvasLayer,
    pub frame: Arc<CanvasFrame>,
}

impl CanvasPaint {
    pub fn into_paint_callback(self, rect: egui::Rect) -> egui::PaintCallback {
        egui_wgpu::Callback::new_paint_callback(rect, self)
    }
}

/// A vertex buffer holding `len` instances.
struct Uploaded {
    buffer: wgpu::Buffer,
    len: u32,
}

fn upload<T: Pod>(device: &wgpu::Device, label: &str, data: &[T]) -> Option<Uploaded> {
    if data.is_empty() {
        return None;
    }
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(data),
        usage: wgpu::BufferUsages::VERTEX,
    });
    Some(Uploaded { buffer, len: data.len() as u32 })
}

/// Pipelines and buffers of the canvas, kept in egui's callback resources.
pub struct CanvasGpu {
    wire_pipeline: wgpu::RenderPipeline,
    box_pipeline: wgpu::RenderPipeline,
    pin_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Instances per segment buffer: segments are split so no buffer exceeds the device limit.
    chunk_len: u32,
    scene_revision: u64,
    segments: Vec<Uploaded>,
    curves: Option<Uploaded>,
    boxes: Option<Uploaded>,
    gates: Option<Uploaded>,
    cards_revision: u64,
    cards: Option<Uploaded>,
    overlay_frame: u64,
    overlay: Option<wgpu::Buffer>,
    overlay_capacity: u64,
}

fn pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
    name: &str,
    source: &str,
    stride: usize,
    attributes: &[wgpu::VertexAttribute],
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(name),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(name),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: stride as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes,
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl CanvasGpu {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("canvas_uniforms"),
            size: std::mem::size_of::<SceneUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canvas_uniform_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("canvas_uniform_bind_group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canvas_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        use wgpu::VertexFormat as F;
        let attr = |format, offset, shader_location| wgpu::VertexAttribute { format, offset, shader_location };
        let wire_pipeline = pipeline(
            device,
            format,
            &layout,
            "canvas_wires",
            include_str!("wire.wgsl"),
            std::mem::size_of::<WireInstance>(),
            &[attr(F::Float32x2, 0, 0), attr(F::Float32x2, 8, 1), attr(F::Unorm8x4, 16, 2), attr(F::Uint32, 20, 3)],
        );
        let box_pipeline = pipeline(
            device,
            format,
            &layout,
            "canvas_boxes",
            include_str!("box.wgsl"),
            std::mem::size_of::<BoxInstance>(),
            &[
                attr(F::Float32x2, 0, 0),
                attr(F::Float32x2, 8, 1),
                attr(F::Unorm8x4, 16, 2),
                attr(F::Unorm8x4, 20, 3),
                attr(F::Float32, 24, 4),
                attr(F::Float32, 28, 5),
            ],
        );
        let pin_pipeline = pipeline(
            device,
            format,
            &layout,
            "canvas_pins",
            include_str!("pin.wgsl"),
            std::mem::size_of::<PinInstance>(),
            &[attr(F::Float32x2, 0, 0), attr(F::Unorm8x4, 8, 1), attr(F::Unorm8x4, 12, 2)],
        );
        let max_bytes = device.limits().max_buffer_size.min(1 << 28);
        let chunk_len = (max_bytes / std::mem::size_of::<WireInstance>() as u64).min(u32::MAX as u64) as u32;
        Self {
            wire_pipeline,
            box_pipeline,
            pin_pipeline,
            uniform_buffer,
            bind_group,
            chunk_len,
            scene_revision: 0,
            segments: Vec::new(),
            curves: None,
            boxes: None,
            gates: None,
            cards_revision: 0,
            cards: None,
            overlay_frame: 0,
            overlay: None,
            overlay_capacity: 0,
        }
    }

    /// Brings the GPU buffers up to date with `frame`, uploading only what changed.
    fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &CanvasFrame, window_points: [f32; 2]) {
        let uniforms = SceneUniforms { screen_size: window_points, ..frame.uniforms };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let scene = &frame.scene;
        if self.scene_revision != scene.revision {
            self.scene_revision = scene.revision;
            self.segments = scene
                .segments
                .chunks(self.chunk_len as usize)
                .filter_map(|chunk| upload(device, "canvas_segments", chunk))
                .collect();
            self.curves = upload(device, "canvas_curves", &scene.curves);
            self.boxes = upload(device, "canvas_boxes", &scene.boxes);
            self.gates = upload(device, "canvas_gates", &scene.gates);
        }
        if let Some(cards) = &frame.cards {
            if self.cards_revision != cards.revision {
                self.cards_revision = cards.revision;
                self.cards = upload(device, "canvas_cards", &cards.boxes);
            }
        }
        if self.overlay_frame != frame.id {
            self.overlay_frame = frame.id;
            let bytes: &[u8] = bytemuck::cast_slice(&frame.overlay);
            if bytes.len() as u64 > self.overlay_capacity {
                self.overlay_capacity = (bytes.len() as u64).next_power_of_two().max(4096);
                self.overlay = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("canvas_overlay"),
                    size: self.overlay_capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            if let (Some(buffer), false) = (&self.overlay, bytes.is_empty()) {
                queue.write_buffer(buffer, 0, bytes);
            }
        }
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'static>, layer: CanvasLayer, frame: &CanvasFrame) {
        pass.set_bind_group(0, &self.bind_group, &[]);
        match layer {
            CanvasLayer::Folders | CanvasLayer::CycleBoxes => {
                let Some(boxes) = &self.boxes else { return };
                let split = (frame.scene.cycle_start as u32).min(boxes.len);
                let range = if layer == CanvasLayer::Folders { 0..split } else { split..boxes.len };
                if (layer == CanvasLayer::Folders && !frame.folders) || range.is_empty() {
                    return;
                }
                pass.set_pipeline(&self.box_pipeline);
                pass.set_vertex_buffer(0, boxes.buffer.slice(..));
                pass.draw(0..VERTICES_PER_QUAD, range);
            }
            CanvasLayer::Wires => {
                if !frame.show_wires {
                    return;
                }
                pass.set_pipeline(&self.wire_pipeline);
                for range in &frame.wire_ranges {
                    // A range can span several segment buffers.
                    let mut start = range.start;
                    while start < range.end {
                        let chunk = (start / self.chunk_len) as usize;
                        let Some(buffer) = self.segments.get(chunk) else { break };
                        let base = chunk as u32 * self.chunk_len;
                        let end = range.end.min(base + buffer.len);
                        pass.set_vertex_buffer(0, buffer.buffer.slice(..));
                        pass.draw(0..VERTICES_PER_QUAD, start - base..end - base);
                        start = end;
                    }
                }
                if let Some(curves) = &self.curves {
                    pass.set_vertex_buffer(0, curves.buffer.slice(..));
                    for range in &frame.curve_ranges {
                        pass.draw(0..VERTICES_PER_CURVE, range.start.min(curves.len)..range.end.min(curves.len));
                    }
                }
                if let (Some(buffer), false) = (&self.overlay, frame.overlay.is_empty()) {
                    let total = frame.overlay.len() as u32;
                    let straight = frame.overlay_curves_start.min(total);
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    if straight > 0 {
                        pass.draw(0..VERTICES_PER_QUAD, 0..straight);
                    }
                    if total > straight {
                        pass.draw(0..VERTICES_PER_CURVE, straight..total);
                    }
                }
            }
            CanvasLayer::Gates => {
                if let (Some(gates), true) = (&self.gates, frame.gates) {
                    pass.set_pipeline(&self.pin_pipeline);
                    pass.set_vertex_buffer(0, gates.buffer.slice(..));
                    pass.draw(0..VERTICES_PER_QUAD, 0..gates.len);
                }
            }
            CanvasLayer::Cards => {
                if let (Some(cards), Some(_)) = (&self.cards, &frame.cards) {
                    pass.set_pipeline(&self.box_pipeline);
                    pass.set_vertex_buffer(0, cards.buffer.slice(..));
                    pass.draw(0..VERTICES_PER_QUAD, 0..cards.len);
                }
            }
        }
    }
}

impl CallbackTrait for CanvasPaint {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(gpu) = resources.get_mut::<CanvasGpu>() {
            let ppp = screen.pixels_per_point.max(1e-4);
            let window = [screen.size_in_pixels[0] as f32 / ppp, screen.size_in_pixels[1] as f32 / ppp];
            gpu.sync(device, queue, &self.frame, window);
        }
        Vec::new()
    }

    fn paint(&self, info: PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &CallbackResources) {
        let clip = info.clip_rect_in_pixels();
        if clip.width_px <= 0 || clip.height_px <= 0 {
            return;
        }
        // The whole window is the viewport, so shader coordinates match egui's; the scissor keeps
        // the canvas off the side panels.
        pass.set_viewport(0.0, 0.0, info.screen_size_px[0] as f32, info.screen_size_px[1] as f32, 0.0, 1.0);
        pass.set_scissor_rect(clip.left_px as u32, clip.top_px as u32, clip.width_px as u32, clip.height_px as u32);
        if let Some(gpu) = resources.get::<CanvasGpu>() {
            gpu.draw(pass, self.layer, &self.frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances_have_the_sizes_the_pipelines_expect() {
        assert_eq!(std::mem::size_of::<WireInstance>(), 24);
        assert_eq!(std::mem::size_of::<BoxInstance>(), 32);
        assert_eq!(std::mem::size_of::<PinInstance>(), 16);
        assert_eq!(std::mem::size_of::<SceneUniforms>(), 32);
    }

    #[test]
    fn shaders_parse_and_validate() {
        for (name, source) in
            [("wire", include_str!("wire.wgsl")), ("box", include_str!("box.wgsl")), ("pin", include_str!("pin.wgsl"))]
        {
            let module = wgpu::naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{name}.wgsl: {}", e.emit_to_string(source)));
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}.wgsl invalid: {e:?}"));
        }
    }
}
