pub mod batch;
pub mod callback;
pub mod pipeline;

pub use batch::GpuWireBatch;
pub use callback::GpuWireCallback;
pub use pipeline::{
    CanvasUniforms, GpuWireInstance, GpuWirePipeline, FLAG_ANIMATED, FLAG_GLOW,
    VERTICES_PER_CURVE,
};

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, Pos2};

    #[test]
    fn test_gpu_wire_instance_memory_layout() {
        assert_eq!(
            std::mem::size_of::<GpuWireInstance>(),
            64,
            "GpuWireInstance must be exactly 64 bytes for optimal GPU cache alignment"
        );
        assert_eq!(
            std::mem::align_of::<GpuWireInstance>(),
            4,
            "GpuWireInstance alignment must match f32"
        );
    }

    #[test]
    fn test_gpu_canvas_uniforms_memory_layout() {
        assert_eq!(
            std::mem::size_of::<CanvasUniforms>(),
            16,
            "CanvasUniforms must be 16 bytes for WebGPU uniform alignment"
        );
    }

    #[test]
    fn test_gpu_wire_batch_push_and_flags() {
        let mut batch = GpuWireBatch::new([1920.0, 1080.0], 1.0, 42.0);
        assert!(batch.is_empty());

        let p0 = Pos2::new(10.0, 20.0);
        let p3 = Pos2::new(100.0, 200.0);
        let color = Color32::from_rgba_premultiplied(255, 128, 64, 255);
        let glow = Some(Color32::from_rgba_premultiplied(255, 128, 64, 90));

        batch.push_wire(p0, p3, color, glow, 2.5, 6.0, true);

        assert_eq!(batch.len(), 1);
        let inst = &batch.instances[0];
        assert_eq!(inst.p0, [10.0, 20.0]);
        assert_eq!(inst.p3, [100.0, 200.0]);
        assert_eq!(inst.params[0], 2.5); // core_width
        assert_eq!(inst.params[1], 6.0); // glow_width
        assert_eq!(inst.params[2], 42.0); // anim_time

        let flags = inst.params[3] as u32;
        assert_eq!(flags & FLAG_GLOW, FLAG_GLOW);
        assert_eq!(flags & FLAG_ANIMATED, FLAG_ANIMATED);
    }

    #[test]
    fn test_wgsl_shader_syntax_validation() {
        let shader_source = include_str!("wire.wgsl");
        assert!(!shader_source.is_empty());

        // Validate that naga can parse the WGSL module
        let module = wgpu::naga::front::wgsl::parse_str(shader_source);
        assert!(
            module.is_ok(),
            "wire.wgsl failed syntax validation: {:?}",
            module.err()
        );
    }
}
