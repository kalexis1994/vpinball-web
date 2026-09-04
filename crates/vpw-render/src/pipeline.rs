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
    /// The opaque pipeline with back faces culled, for the batches the
    /// original culls. See `Batch::culled`.
    pub opaque_culled: wgpu::RenderPipeline,
    pub blended: wgpu::RenderPipeline,
    /// The same two, for the pieces that carry a model matrix. They differ from
    /// the static ones only in the vertex stage and in having a third bind
    /// group; everything downstream — the material, the lighting, the blending
    /// — is shared.
    pub dynamic_opaque: wgpu::RenderPipeline,
    pub dynamic_blended: wgpu::RenderPipeline,
    /// For the pieces that are **light** rather than things: added to what is
    /// already drawn, writing no depth and culling nothing, exactly as the
    /// original does for a primitive with Additive Blend
    /// (`primitive.cpp:1166`). See [`vpw_table::geometry::Additive`].
    pub dynamic_additive: wgpu::RenderPipeline,
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
    /// The baked GI lightmap array, or a single black layer until a bake
    /// lands, and how many layers of it are live.
    gi_lightmap: wgpu::TextureView,
    gi_layers: u32,
    /// The playfield's picture, for the ball's planar reflection.
    field_picture: wgpu::TextureView,
    /// The same, for the pass that draws the reflection probe: a camera flipped
    /// through the playfield and a clip plane to go with it.
    pub mirror_buffer: wgpu::Buffer,
    pub mirror_bind_group: wgpu::BindGroup,
    /// One black pixel, bound wherever a probe would be but must not be.
    blank: wgpu::TextureView,
    /// The probes the frame groups were last built with, kept so they can
    /// be built again when the environment changes underneath them: the
    /// environment is the table's (`Renderer.cpp:208`), and a bind group
    /// holds the views it was made from.
    transmission: wgpu::TextureView,
    transmission_sampler: wgpu::Sampler,
    reflection: wgpu::TextureView,
    pub frame_bind_group: wgpu::BindGroup,
    pub envmap: crate::env::EnvMap,
    pub mip_levels: u32,
    pub env_height: u32,
    /// What the table pass clears to: the room's tint, kept dim. What is
    /// behind a machine is the room it stands in, out of the light.
    pub clear: wgpu::Color,
    /// Draws the table's own backdrop picture over the cleared frame. See
    /// `backdrop.wgsl`.
    pub backdrop: wgpu::RenderPipeline,
    /// What that pipeline binds: the picture and its sampler.
    pub backdrop_layout: wgpu::BindGroupLayout,
}

impl TablePipeline {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
        samples: u32,
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
        let backdrop_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vpw-backdrop-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let backdrop_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vpw-backdrop-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("backdrop.wgsl").into()),
        });
        let backdrop = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vpw-backdrop"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("vpw-backdrop-pipeline-layout"),
                    bind_group_layouts: &[Some(&backdrop_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &backdrop_module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &backdrop_module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            // No depth at all: it is behind everything by construction, and
            // the original turns the test and the write off for it.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });

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
                // The baked general illumination: one layer per group, in
                // playfield UV space.
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                // The playfield's picture, for the ball to reflect.
                env_texture(8),
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
                         depth_write: bool,
                         cull: Option<wgpu::Face>| {
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
                    // `None` for everything thin-walled — ramps, walls,
                    // rubbers, whose inside is their far side — and `Back`
                    // for the one batch class the original culls: opaque
                    // primitives, under its scene-wide `CULL_CCW`
                    // (`Renderer.cpp:927`, `primitive.cpp:1132`). D3D
                    // measures winding with y down and WebGPU with y up, so
                    // its CULL_CCW is our back-face cull with the default
                    // front face.
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };

        let frame_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vpw-frame"),
            contents: bytemuck::bytes_of(&GpuFrame::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // The shipped map, until a table brings its own (`set_envmap`).
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
        // The GI lightmap starts as black layers: a table with no bake adds
        // nothing, and the shader never has to ask. Two of them rather than
        // one, because wgpu-hal guesses a texture's view dimension from its
        // layer count — a single layer is assumed to be a plain `D2`, and
        // viewing it as the array the shader binds earns a warning at every
        // boot.
        let field_picture = blank.clone();
        let gi_lightmap = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("vpw-no-gi-bake"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 2,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        let frame_bind_group = Self::frame_bg(
            device,
            &frame_layout,
            &frame_buffer,
            &envmap,
            &blank,
            &envmap.sampler,
            &blank,
            &gi_lightmap,
            &field_picture,
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
            &gi_lightmap,
            &field_picture,
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
            transmission: blank.clone(),
            transmission_sampler: envmap.sampler.clone(),
            reflection: blank.clone(),
            blank,
            gi_lightmap,
            gi_layers: 0,
            field_picture,
            light_frame_layout,
            light_frame_bind_group,
            mip_levels: envmap.mip_levels,
            env_height: envmap.height,
            envmap,
            clear: crate::pass::CLEAR,
            opaque: make_with("vpw-opaque", &shader, &layout, None, true, None),
            opaque_culled: make_with(
                "vpw-opaque-culled",
                &shader,
                &layout,
                None,
                true,
                Some(wgpu::Face::Back),
            ),
            blended: make_with(
                "vpw-transparent",
                &shader,
                &layout,
                Some(wgpu::BlendState::ALPHA_BLENDING),
                false,
                None,
            ),
            dynamic_opaque: make_with(
                "vpw-dynamic-opaque",
                &dynamic_shader,
                &dynamic_layout,
                None,
                true,
                None,
            ),
            dynamic_blended: make_with(
                "vpw-dynamic-transparent",
                &dynamic_shader,
                &dynamic_layout,
                Some(wgpu::BlendState::ALPHA_BLENDING),
                false,
                None,
            ),
            dynamic_additive: make_with(
                "vpw-dynamic-additive",
                &dynamic_shader,
                &dynamic_layout,
                // `SRCBLEND = SRC_ALPHA`, `DESTBLEND = ONE`, `BLENDOP = ADD`
                // — the original's `EnableAlphaBlend(true)`
                // (`RenderDevice.cpp:2497`).
                //
                // The source alpha is doing real work: for a lightmap it is
                // the coverage baked into the atlas, so a lamp adds light only
                // where the bake says it reaches. With `ONE, ONE` the whole
                // atlas lands on every layer and ninety-six of them turn the
                // table white.
                Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::SrcAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::REPLACE,
                }),
                false,
                None,
            ),
            backdrop,
            backdrop_layout,
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
        self.transmission = view.clone();
        self.transmission_sampler = sampler.clone();
        self.reflection = reflection.clone();
        self.rebind(device);
    }

    /// Hands the frame the playfield's picture, for the ball to reflect.
    pub fn set_field_picture(&mut self, device: &wgpu::Device, view: wgpu::TextureView) {
        self.field_picture = view;
        self.rebind(device);
    }

    /// Hands the frame the baked GI lightmap array. See `crate::bake`.
    pub fn set_gi_lightmap(&mut self, device: &wgpu::Device, view: wgpu::TextureView, layers: u32) {
        self.gi_lightmap = view;
        self.gi_layers = layers;
        self.rebind(device);
    }

    /// Swaps in a table's environment map.
    ///
    /// The map belongs to the table, not to the pipeline (`Renderer.cpp:208`
    /// loads it from `m_table->m_envImage`), so it is set when a table is and
    /// not when the pipeline is. The frame groups hold the old views and are
    /// built again; the mip count and height the shader picks its specular
    /// level from (`set_frame`) follow the new map.
    pub fn set_envmap(&mut self, device: &wgpu::Device, envmap: crate::env::EnvMap) {
        self.mip_levels = envmap.mip_levels;
        self.env_height = envmap.height;
        // The backdrop takes the room's tint, kept far below the table: what
        // is behind a machine is the room it stands in, out of the light.
        // The mean is normalised so only the hue survives, then set to a
        // fixed dim level — a bright map should not mean a bright void.
        let peak = envmap.mean.iter().cloned().fold(1e-6f32, f32::max);
        let [r, g, b] = envmap.mean.map(|c| f64::from(c / peak * 0.006));
        self.clear = wgpu::Color { r, g, b, a: 1.0 };
        self.envmap = envmap;
        self.rebind(device);
    }

    /// Builds both frame groups again from whatever is currently set.
    fn rebind(&mut self, device: &wgpu::Device) {
        self.frame_bind_group = Self::frame_bg(
            device,
            &self.frame_layout,
            &self.frame_buffer,
            &self.envmap,
            &self.transmission,
            &self.transmission_sampler,
            &self.reflection,
            &self.gi_lightmap,
            &self.field_picture,
        );
        // The reflection pass renders *into* the probe, so its own bind group
        // must not hold it — and has no use for it either: nothing reflects
        // inside a reflection.
        self.mirror_bind_group = Self::frame_bg(
            device,
            &self.frame_layout,
            &self.mirror_buffer,
            &self.envmap,
            &self.transmission,
            &self.transmission_sampler,
            &self.blank,
            &self.gi_lightmap,
            &self.field_picture,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn frame_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buffer: &wgpu::Buffer,
        envmap: &crate::env::EnvMap,
        transmission: &wgpu::TextureView,
        transmission_sampler: &wgpu::Sampler,
        reflection: &wgpu::TextureView,
        gi_lightmap: &wgpu::TextureView,
        field_picture: &wgpu::TextureView,
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
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(gi_lightmap),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(field_picture),
                },
            ],
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_frame(
        &self,
        queue: &wgpu::Queue,
        view_proj: Mat4,
        eye: vpw_math::Vec3,
        lighting: &vpw_table::geometry::Lighting,
        size: (u32, u32),
        gi: &crate::lights::Gi,
        field: [f32; 4],
    ) {
        let mut data = GpuFrame::from_lighting(lighting);
        data.view_proj = view_proj.to_cols_array_2d();
        data.eye = [eye.x, eye.y, eye.z, 1.0];
        data.env[0] = self.mip_levels as f32;
        data.env[1] = self.env_height as f32;
        // The general illumination, brightest first. See `GpuFrame::gi`.
        let bulbs = gi.rows.len().min(crate::scene::MAX_GI_BULBS);
        data.env[3] = bulbs as f32;
        data.gi_bounce = [gi.bounce[0], gi.bounce[1], gi.bounce[2], 0.0];
        data.gi_levels = gi.levels;
        data.env[2] = self.gi_layers as f32;
        data.field = field;
        for (i, rows) in gi.rows.iter().take(bulbs).enumerate() {
            data.gi[i * 2] = rows[0];
            data.gi[i * 2 + 1] = rows[1];
        }
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
pub fn depth_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    samples: u32,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("vpw-depth"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}
