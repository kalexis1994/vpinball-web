//! The pipeline that draws the table.
//!
//! The original picks one of dozens of ubershader "techniques" per draw call
//! (`Shader.cpp`, `BasicShader.hlsl`). Here there are two fixed pipelines
//! —opaque and transparent— that share module and layout, chosen when the scene
//! is built and not per frame.

use crate::scene::{GpuFrame, GpuVertex};
use bytemuck::Zeroable;
use vpw_math::Mat4;
use wgpu::util::DeviceExt;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub struct TablePipeline {
    pub opaque: wgpu::RenderPipeline,
    pub blended: wgpu::RenderPipeline,
    /// The same two, for the pieces that carry a model matrix. They differ from
    /// the static ones only in the vertex stage and in having a third bind
    /// group; everything downstream — the material, the lighting, the blending
    /// — is shared.
    pub dynamic_opaque: wgpu::RenderPipeline,
    pub dynamic_blended: wgpu::RenderPipeline,
    pub frame_layout: wgpu::BindGroupLayout,
    /// The camera on its own, without the environment or the transmitted-light
    /// buffer. The light halos want only this, and binding them the full frame
    /// group would put the transmitted-light texture in scope during the very
    /// pass that renders into it.
    pub light_frame_layout: wgpu::BindGroupLayout,
    pub light_frame_bind_group: wgpu::BindGroup,
    pub material_layout: wgpu::BindGroupLayout,
    pub model_layout: wgpu::BindGroupLayout,
    pub frame_buffer: wgpu::Buffer,
    /// The same, for the pass that draws the reflection probe: a camera flipped
    /// through the playfield and a clip plane to go with it.
    pub mirror_buffer: wgpu::Buffer,
    pub mirror_bind_group: wgpu::BindGroup,
    /// One black pixel, bound wherever a probe would be but must not be.
    blank: wgpu::TextureView,
    pub frame_bind_group: wgpu::BindGroup,
    pub envmap: crate::env::EnvMap,
    pub mip_levels: u32,
    pub env_height: u32,
}

impl TablePipeline {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
    ) -> Self {
        // Two shader modules that share everything but the vertex stage. See
        // the header of `material.wgsl` for why the split is there rather than
        // a model matrix forced onto the static path.
        let make_module = |name: &str, vertex_stage: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(
                    format!("{}{}", include_str!("material.wgsl"), vertex_stage).into(),
                ),
            })
        };
        let shader = make_module("vpw-table-shader", include_str!("table_vs.wgsl"));
        let dynamic_shader = make_module("vpw-dynamic-shader", include_str!("dynamic_vs.wgsl"));

        let env_texture = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vpw-frame-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The environment goes here and not with the material: it
                // belongs to the whole scene and is bound once per frame.
                env_texture(1),
                env_texture(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // The lights, drawn on their own and blurred, for translucent
                // surfaces to add what is shining through them. It belongs to
                // the frame because it is one buffer for the whole picture.
                env_texture(4),
                // The playfield's reflection, likewise.
                env_texture(6),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let light_frame_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("vpw-light-frame-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vpw-material-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let model_layout = crate::dynamic::DynamicParts::model_layout(device);

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vpw-table-layout"),
            bind_group_layouts: &[Some(&frame_layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let dynamic_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vpw-dynamic-layout"),
            bind_group_layouts: &[
                Some(&frame_layout),
                Some(&material_layout),
                Some(&model_layout),
            ],
            immediate_size: 0,
        });

        let vertex_attrs = [Some(GpuVertex::layout())];
        let make_with = |name: &str,
                         module: &wgpu::ShaderModule,
                         pipeline_layout: &wgpu::PipelineLayout,
                         blend: Option<wgpu::BlendState>,
                         depth_write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(pipeline_layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs_main"),
                    buffers: &vertex_attrs,
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend,
                        // Color only: the destination alpha is left exactly
                        // as the clear left it. Blending still uses the alpha
                        // the shader returns —that comes from the source, not
                        // the destination— but the final image ends up opaque
                        // instead of full of holes wherever there are
                        // translucent parts.
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    // Tables are full of single-sided meshes with their
                    // winding the wrong way round; culling faces here erases
                    // half the playfield. The original does not cull by default
                    // either.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let frame_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vpw-frame"),
            contents: bytemuck::bytes_of(&GpuFrame::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let envmap = crate::env::EnvMap::default_map(device, queue)
            .expect("the default environment map has to be loadable");
        // A one-pixel black stand-in until `set_transmission` is handed the real
        // buffer, so the bind group is complete from the first frame whether or
        // not anything has been sized yet.
        let blank = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("vpw-no-transmission"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let frame_bind_group = Self::frame_bg(
            device,
            &frame_layout,
            &frame_buffer,
            &envmap,
            &blank,
            &envmap.sampler,
            &blank,
        );

        let mirror_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vpw-mirror-frame"),
            size: std::mem::size_of::<GpuFrame>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mirror_bind_group = Self::frame_bg(
            device,
            &frame_layout,
            &mirror_buffer,
            &envmap,
            &blank,
            &envmap.sampler,
            &blank,
        );

        let light_frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vpw-light-frame-bg"),
            layout: &light_frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });

        Self {
            mirror_buffer,
            mirror_bind_group,
            blank,
            light_frame_layout,
            light_frame_bind_group,
            mip_levels: envmap.mip_levels,
            env_height: envmap.height,
            envmap,
            opaque: make_with("vpw-opaque", &shader, &layout, None, true),
            blended: make_with(
                "vpw-transparent",
                &shader,
                &layout,
                Some(wgpu::BlendState::ALPHA_BLENDING),
                false,
            ),
            dynamic_opaque: make_with(
                "vpw-dynamic-opaque",
                &dynamic_shader,
                &dynamic_layout,
                None,
                true,
            ),
            dynamic_blended: make_with(
                "vpw-dynamic-transparent",
                &dynamic_shader,
                &dynamic_layout,
                Some(wgpu::BlendState::ALPHA_BLENDING),
                false,
            ),
            frame_layout,
            material_layout,
            model_layout,
            frame_buffer,
            frame_bind_group,
        }
    }

    /// Uploads the frame block: camera plus the table's lighting. Out of all
    /// of it, the only thing that changes between frames is the camera.
    /// Points the shader at the transmitted-light buffer.
    ///
    /// Separate from the constructor because the buffer is sized to the window
    /// and so is replaced whenever the window is, and a bind group holds the
    /// view it was built with.
    pub fn set_probes(
        &mut self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        reflection: &wgpu::TextureView,
    ) {
        self.frame_bind_group = Self::frame_bg(
            device,
            &self.frame_layout,
            &self.frame_buffer,
            &self.envmap,
            view,
            sampler,
            reflection,
        );
        // The reflection pass renders *into* the probe, so its own bind group
        // must not hold it — and has no use for it either: nothing reflects
        // inside a reflection.
        let _ = reflection;
        self.mirror_bind_group = Self::frame_bg(
            device,
            &self.frame_layout,
            &self.mirror_buffer,
            &self.envmap,
            view,
            sampler,
            &self.blank,
        );
    }

    fn frame_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buffer: &wgpu::Buffer,
        envmap: &crate::env::EnvMap,
        transmission: &wgpu::TextureView,
        transmission_sampler: &wgpu::Sampler,
        reflection: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vpw-frame-bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&envmap.radiance),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&envmap.irradiance),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&envmap.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(transmission),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(transmission_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(reflection),
                },
            ],
        })
    }

    pub fn set_frame(
        &self,
        queue: &wgpu::Queue,
        view_proj: Mat4,
        eye: vpw_math::Vec3,
        lighting: &vpw_table::geometry::Lighting,
        size: (u32, u32),
    ) {
        let mut data = GpuFrame::from_lighting(lighting);
        data.view_proj = view_proj.to_cols_array_2d();
        data.eye = [eye.x, eye.y, eye.z, 1.0];
        data.env[0] = self.mip_levels as f32;
        data.env[1] = self.env_height as f32;
        data.screen = [
            1.0 / size.0.max(1) as f32,
            1.0 / size.1.max(1) as f32,
            0.0,
            0.0,
        ];
        queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&data));

        // And the same frame seen through the playfield, for the reflection
        // probe. Two buffers rather than one written twice: both passes are
        // recorded into the same command encoder, and a second write would land
        // on the first pass as well.
        let mirror = Mat4::from_cols_array_2d(&data.mirror_of_plane());
        data.view_proj = (view_proj * mirror).to_cols_array_2d();
        // Everything below the playfield is thrown away. The probe is of what
        // stands *on* the table; the playfield itself, drawn into it, would
        // cover the lot.
        data.clip = [0.0, 0.0, 1.0, 0.0];
        // Nothing reflects inside a reflection.
        data.mirror[3] = 0.0;
        // The eye moves with the world, or the specular in the reflection comes
        // from the wrong side.
        let e = mirror.transform_point3(eye);
        data.eye = [e.x, e.y, e.z, 1.0];
        queue.write_buffer(&self.mirror_buffer, 0, bytemuck::bytes_of(&data));
    }
}

/// Creates the depth texture at the requested size.
pub fn depth_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("vpw-depth"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}
