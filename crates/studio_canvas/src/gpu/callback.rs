use super::pipeline::{CanvasUniforms, GpuWireInstance, GpuWirePipeline, VERTICES_PER_CURVE};
use egui::PaintCallbackInfo;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

/// Custom paint callback implementing `egui_wgpu::CallbackTrait`.
pub struct GpuWireCallback {
    pub uniforms: CanvasUniforms,
    pub instances: Vec<GpuWireInstance>,
}

impl CallbackTrait for GpuWireCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if self.instances.is_empty() {
            return Vec::new();
        }

        if let Some(pipeline) = callback_resources.get_mut::<GpuWirePipeline>() {
            let ppp = screen_descriptor.pixels_per_point.max(1e-4);
            let window_size_points =
                [screen_descriptor.size_in_pixels[0] as f32 / ppp, screen_descriptor.size_in_pixels[1] as f32 / ppp];
            let mut uniforms = self.uniforms;
            uniforms.screen_size = window_size_points;

            pipeline.update_uniforms(queue, &uniforms);
            pipeline.upload_instances(device, queue, &self.instances);
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        if self.instances.is_empty() {
            return;
        }

        let clip_px = info.clip_rect_in_pixels();
        if clip_px.width_px <= 0 || clip_px.height_px <= 0 {
            return;
        }

        // Establish full-window viewport for exact 1:1 match with egui screen coordinates
        render_pass.set_viewport(0.0, 0.0, info.screen_size_px[0] as f32, info.screen_size_px[1] as f32, 0.0, 1.0);

        // Set scissor rect to the canvas clip rect to prevent lines bleeding over sidebars
        render_pass.set_scissor_rect(
            clip_px.left_px as u32,
            clip_px.top_px as u32,
            clip_px.width_px as u32,
            clip_px.height_px as u32,
        );

        if let Some(pipeline) = callback_resources.get::<GpuWirePipeline>() {
            render_pass.set_pipeline(&pipeline.pipeline);
            render_pass.set_bind_group(0, &pipeline.uniform_bind_group, &[]);
            render_pass.set_vertex_buffer(0, pipeline.instance_buffer.slice(..));
            render_pass.draw(0..VERTICES_PER_CURVE, 0..self.instances.len() as u32);
        }
    }
}
