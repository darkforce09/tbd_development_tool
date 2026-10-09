use bytemuck::{Pod, Zeroable};

pub const VERTICES_PER_CURVE: u32 = 384;
pub const FLAG_GLOW: u32 = 1;
pub const FLAG_ANIMATED: u32 = 2;

/// A compact 64-byte descriptor for a single Bezier wire uploaded to the GPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuWireInstance {
    pub p0: [f32; 2],
    pub p3: [f32; 2],
    pub color: [f32; 4],
    pub glow_color: [f32; 4],
    pub params: [f32; 4], // [core_width, glow_width, anim_time, flags]
}

/// Uniform data passed to the wire shader per frame.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct CanvasUniforms {
    pub screen_size: [f32; 2],
    pub zoom: f32,
    pub anim_time: f32,
}

/// Dedicated wgpu render pipeline for GPU-accelerated connection wires.
pub struct GpuWirePipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub uniform_buffer: wgpu::Buffer,
    pub uniform_bind_group: wgpu::BindGroup,
    pub instance_buffer: wgpu::Buffer,
    pub instance_capacity: usize,
}

impl GpuWirePipeline {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu_wire_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("wire.wgsl").into()),
        });

        // 1. Uniform Buffer and Bind Group
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_wire_uniforms"),
            size: std::mem::size_of::<CanvasUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gpu_wire_uniform_layout"),
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

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu_wire_uniform_bind_group"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        // 2. Initial Instance Buffer (capacity 4096 wires)
        let initial_capacity = 4096;
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_wire_instances"),
            size: (initial_capacity * std::mem::size_of::<GpuWireInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // 3. Pipeline Layout & Vertex Attributes
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu_wire_pipeline_layout"),
            bind_group_layouts: &[&uniform_bind_group_layout],
            push_constant_ranges: &[],
        });

        let vertex_attributes = [
            // @location(0) p0: vec2f
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            },
            // @location(1) p3: vec2f
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 8,
                shader_location: 1,
            },
            // @location(2) color: vec4f
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 16,
                shader_location: 2,
            },
            // @location(3) glow_color: vec4f
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 32,
                shader_location: 3,
            },
            // @location(4) params: vec4f [core_width, glow_width, anim_time, flags]
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 48,
                shader_location: 4,
            },
        ];

        let vertex_buffer_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuWireInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &vertex_attributes,
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gpu_wire_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[vertex_buffer_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        Self {
            pipeline,
            uniform_buffer,
            uniform_bind_group,
            instance_buffer,
            instance_capacity: initial_capacity,
        }
    }

    /// Resizes the instance buffer if the batch size exceeds current capacity.
    pub fn ensure_instance_capacity(&mut self, device: &wgpu::Device, required: usize) {
        if required > self.instance_capacity {
            let new_capacity = (required * 2).max(self.instance_capacity * 2);
            self.instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu_wire_instances_resized"),
                size: (new_capacity * std::mem::size_of::<GpuWireInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.instance_capacity = new_capacity;
        }
    }

    /// Uploads frame uniforms (screen dimensions, zoom, anim_time).
    pub fn update_uniforms(&self, queue: &wgpu::Queue, uniforms: &CanvasUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
    }

    /// Uploads instance data to the GPU buffer.
    pub fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[GpuWireInstance],
    ) {
        if instances.is_empty() {
            return;
        }
        self.ensure_instance_capacity(device, instances.len());
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));
    }
}
