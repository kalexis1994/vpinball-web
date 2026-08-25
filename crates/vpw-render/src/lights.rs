//! The lights on the GPU.
//!
//! They get their own pipeline and their own pass, after the geometry: they are
//! drawn **additively** over what is already there, without writing depth —but
//! testing it, so that an insert is not visible through a ramp that covers it.
//!
//! The original draws them with `blend_modulate_vs_add` at 0.0001, which is its
//! way of saying "pure additive without turning blending off"
//! (`light.cpp:781`).

use wgpu::util::DeviceExt;

/// What the shader needs from each light.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuLight {
    /// xyz = center, w = 1 / range
    center: [f32; 4],
    /// rgb = center color, a = intensity
    color: [f32; 4],
    /// rgb = edge color, a = falloff exponent
    color2: [f32; 4],
    /// x = how much the halo modulates rather than adds, y = how much of it
    /// reaches the transmitted-light buffer.
    blend: [f32; 4],
}

/// One uploaded light: its shape and its data block.
pub struct Light {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub index_count: u32,
    pub bind_group: wgpu::BindGroup,
    /// The uniform, kept so the state can be written to it: a lamp goes on and
    /// off many times a second and rebuilding its buffers for that would be
    /// absurd.
    uniform: wgpu::Buffer,
    /// Exactly what is in the uniform, so a change can be written without
    /// rebuilding the parts that did not move.
    data: GpuLight,
    /// The two colours as the file gives them, before the incandescent fader
    /// tints them by the filament's temperature. Kept because that tint is a
    /// *ratio* applied to the originals every frame, and folding it into
    /// `data` would compound it.
    base_color: [f32; 3],
    base_color2: [f32; 3],
    /// The lamp itself: the level it is showing, where in the blink pattern it
    /// is, how hot its filament is. See `vpw_table::light::Lamp`.
    lamp: vpw_table::light::Lamp,
    /// Whether this one blends as a bulb rather than as a flat additive disc.
    bulb: bool,
    /// `TRMS`. Zero keeps the lamp out of the transmitted-light pass entirely
    /// (`light.cpp:600`).
    transmission: f32,
    /// Whether the **file** declared this one a blinker. See
    /// [`Lights::blinks`].
    blinking: bool,
}

/// Which of the three draws a call is making. They differ in the blend, in
/// whether they test depth, and in which lights they take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    /// A flat additive disc, depth-tested: the classic insert.
    Classic,
    /// The modulating halo of a bulb light, depth-tested.
    Bulb,
    /// Into the transmitted-light buffer, no depth, bulbs only.
    Transmitted,
}

impl Light {
    /// Pushes whatever the lamp has just changed into the uniform.
    ///
    /// Two shapes of write, because the common one is tiny. A lamp that only
    /// got brighter has moved one float, and a table has hundreds of lamps; a
    /// lamp on the incandescent fader has also changed colour, since the tint
    /// is a function of its filament's temperature (`light.cpp:723-735`), and
    /// that is the two colours either side of the intensity — so the three
    /// travel together.
    fn write(&mut self, queue: &wgpu::Queue) {
        let intensity = self.lamp.level();
        let tint = self.lamp.tint();
        self.data.color[3] = intensity;
        if tint == [1.0; 3] {
            queue.write_buffer(
                &self.uniform,
                std::mem::offset_of!(GpuLight, color) as u64 + 3 * 4,
                bytemuck::bytes_of(&intensity),
            );
            return;
        }
        for (i, t) in tint.iter().enumerate() {
            self.data.color[i] = self.base_color[i] * t;
            self.data.color2[i] = self.base_color2[i] * t;
        }
        // `color` and `color2` are adjacent, so one write covers both.
        let at = std::mem::offset_of!(GpuLight, color) as u64;
        queue.write_buffer(
            &self.uniform,
            at,
            bytemuck::cast_slice(&[self.data.color, self.data.color2]),
        );
    }
}

pub struct Lights {
    pub pipeline: wgpu::RenderPipeline,
    /// The same shape with no depth test and the transmission scale folded in,
    /// for the transmitted-light pass.
    flat: wgpu::RenderPipeline,
    /// A bulb light's halo, which modulates as well as adds. See `fs_bulb`.
    bulb: wgpu::RenderPipeline,
    pub layout: wgpu::BindGroupLayout,
    pub lights: Vec<Light>,
    /// The lamps' names, in the same order, so a caller can find the one the
    /// script is talking about.
    pub names: Vec<String>,
}

impl Lights {
    pub fn new(
        device: &wgpu::Device,
        frame_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vpw-light-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("light.wgsl").into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vpw-light-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vpw-light-pipeline-layout"),
            bind_group_layouts: &[Some(frame_layout), Some(&layout)],
            immediate_size: 0,
        });

        let attributes = [Some(wgpu::VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            }],
        })];

        let make = |name: &str, entry: &str, depth: Option<wgpu::DepthStencilState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &attributes,
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        // Additive: the light **adds** to what is already there.
                        // The alpha the shader returns modulates how much it adds
                        // at the edge of the halo, where the attenuation has
                        // already faded it out.
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::SrcAlpha,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent::REPLACE,
                        }),
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: depth,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        // Two of the same pipeline, differing only in the depth test.
        //
        // In the scene the halo must be occluded — an insert under a ramp is
        // not visible through it — so it tests depth without writing it. In the
        // transmitted-light pass there is no depth buffer at all and there
        // should not be: the question there is where the lamp light *is*, not
        // what can see it, and the original disables z for the same pass with
        // the same reasoning (`Renderer.cpp:1496`).
        // The bulb halo's blend, which is where a bulb light differs from a
        // classic one. See `fs_bulb` in `light.wgsl` for what the three
        // settings compute; `light.cpp:827` is where the original sets them.
        let bulb = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vpw-lights-bulb"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &attributes,
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_bulb"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrc,
                            operation: wgpu::BlendOperation::ReverseSubtract,
                        },
                        alpha: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::pipeline::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let pipeline = make(
            "vpw-lights",
            "fs_main",
            Some(wgpu::DepthStencilState {
                format: crate::pipeline::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
        );
        let flat = make("vpw-lights-flat", "fs_transmitted", None);

        Self {
            pipeline,
            bulb,
            flat,
            layout,
            lights: Vec::new(),
            names: Vec::new(),
        }
    }

    /// Uploads a table's lit lights.
    pub fn upload(&mut self, device: &wgpu::Device, lights: &[vpw_table::light::Light]) {
        self.names = lights.iter().map(|l| l.name.clone()).collect();
        self.lights = lights
            .iter()
            .map(|l| {
                // The lamp starts where the file leaves it — lit if the file
                // says lit — rather than at zero with a fade to climb out of.
                let lamp = vpw_table::light::Lamp::new(l);
                let tint = lamp.tint();
                let data = GpuLight {
                    center: [
                        l.center.x,
                        l.center.y,
                        l.center.z,
                        1.0 / l.falloff_radius.max(1.0),
                    ],
                    color: [
                        l.color[0] * tint[0],
                        l.color[1] * tint[1],
                        l.color[2] * tint[2],
                        lamp.level(),
                    ],
                    blend: [l.modulate, l.transmission_scale, 0.0, 0.0],
                    color2: [
                        l.color2[0] * tint[0],
                        l.color2[1] * tint[1],
                        l.color2[2] * tint[2],
                        l.falloff_power.max(0.1),
                    ],
                };
                let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vpw-light"),
                    contents: bytemuck::bytes_of(&data),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
                Light {
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("vpw-light-vertices"),
                        contents: bytemuck::cast_slice(&l.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("vpw-light-indices"),
                        contents: bytemuck::cast_slice(&l.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    index_count: l.indices.len() as u32,
                    bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("vpw-light-bg"),
                        layout: &self.layout,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        }],
                    }),
                    uniform,
                    data,
                    base_color: l.color,
                    base_color2: l.color2,
                    lamp,
                    bulb: l.is_bulb,
                    transmission: l.transmission_scale,
                    blinking: l.blinking,
                }
            })
            .collect();
    }

    /// Turns a lamp on, off, or part way, with no fade at all.
    ///
    /// `state` is the original's `m_inPlayState` — 0 off, 1 on,
    /// `vpw_table::light::BLINKING` for the pattern, or any level in between
    /// from a 10.8 table — and `scale` is the script's `IntensityScale`, which
    /// a table uses to dim a lamp rather than switch it.
    ///
    /// For a caller with no clock to offer. Everything drawing a table should
    /// use [`Lights::animate`] instead: the fade is not decoration, it is what
    /// separates a bulb from a switch.
    pub fn set_state(&mut self, queue: &wgpu::Queue, index: usize, state: f32, scale: f32) {
        let Some(light) = self.lights.get_mut(index) else {
            return;
        };
        if light.lamp.snap(state, scale) {
            light.write(queue);
        }
    }

    /// One frame of a lamp's life: `Light::UpdateAnimation`, `light.cpp:299`.
    ///
    /// `dt_ms` is how much **table** time the frame was worth. The fade, the
    /// blink pattern and the filament all run off it, so a fade that keeps
    /// going while the physics is stopped would drift away from the game it
    /// belongs to.
    ///
    /// Writes nothing when nothing moved, which is almost always: a table has
    /// hundreds of lamps and a handful change on any given frame.
    pub fn animate(
        &mut self,
        queue: &wgpu::Queue,
        index: usize,
        state: f32,
        scale: f32,
        dt_ms: f32,
    ) {
        let Some(light) = self.lights.get_mut(index) else {
            return;
        };
        if light.lamp.update(state, scale, dt_ms) {
            light.write(queue);
        }
    }

    /// Whether the file declared lamp `index` a blinker.
    ///
    /// For a caller whose source of lamp states cannot say so itself. The
    /// original keeps one number, `m_inPlayState`, and two is the value that
    /// means "run the pattern" (`light.cpp:315`) — but a port whose script
    /// layer stores the state as on-or-off has thrown that value away by the
    /// time it gets here, and every blinking lamp on the table sits
    /// permanently lit. This is what such a caller can fall back on.
    pub fn blinks(&self, index: usize) -> bool {
        self.lights.get(index).is_some_and(|l| l.blinking)
    }

    pub fn is_empty(&self) -> bool {
        self.lights.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lights.len()
    }

    /// Emits the lights.
    ///
    /// Two pipelines, chosen per light: a bulb blends into what is under it and
    /// a classic one is added flat on top. The original switches the same two
    /// sets of render states for the same reason (`light.cpp:817` and `:827`).
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, frame: &wgpu::BindGroup) {
        self.draw_with(pass, &self.pipeline, Which::Classic, frame);
        self.draw_with(pass, &self.bulb, Which::Bulb, frame);
    }

    /// The same, into a pass with no depth buffer.
    pub fn draw_flat(&self, pass: &mut wgpu::RenderPass<'_>, frame: &wgpu::BindGroup) {
        self.draw_with(pass, &self.flat, Which::Transmitted, frame);
    }

    fn draw_with(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        which: Which,
        frame: &wgpu::BindGroup,
    ) {
        if self.lights.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, frame, &[]);
        for l in &self.lights {
            let takes = match which {
                Which::Classic => !l.bulb,
                Which::Bulb => l.bulb,
                // The transmitted-light buffer takes bulb lights and only bulb
                // lights, and only those with something to transmit: the
                // original leaves `Light::Render` before it draws anything at
                // all otherwise (`light.cpp:600`). A classic insert is artwork
                // lit from behind and it has no business shining *through* the
                // plastics above it or onto the ball — put every light in here
                // and the whole table glows from underneath.
                Which::Transmitted => l.bulb && l.transmission > 0.0,
            };
            if !takes {
                continue;
            }
            // A lamp that is off contributes nothing but still costs a draw
            // call, and a table has hundreds of them with a handful lit.
            if l.lamp.level() <= 0.0 {
                continue;
            }
            pass.set_bind_group(1, &l.bind_group, &[]);
            pass.set_vertex_buffer(0, l.vertices.slice(..));
            pass.set_index_buffer(l.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..l.index_count, 0, 0..1);
        }
    }
}
