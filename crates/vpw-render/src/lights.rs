//! The lights on the GPU.
//!
//! They get their own pipelines and their own pass, after the geometry: they
//! are drawn **additively** over what is already there, without writing depth
//! — but testing it, so that an insert is not visible through a ramp that
//! covers it.
//!
//! The original draws them with `blend_modulate_vs_add` at 0.0001, which is its
//! way of saying "pure additive without turning blending off"
//! (`light.cpp:781`).
//!
//! # A lit insert is a texture, lit
//!
//! A classic light with a picture is not a halo. The original draws it with
//! the `light_with_texture` technique (`light.cpp:808-811`): the picture is
//! bound as `tex_light_color`, the light lies on a surface whose *material* is
//! set on the shader (`:807`), and the fragment lights the picture's texel
//! through that material before folding the halo into it. So such a light
//! needs what a piece of the table needs — a material block and a texture —
//! and it gets them the way a piece of the table does: through
//! [`crate::scene::material_slot`], into the pipeline's material layout. Two
//! lights on the same surface with the same picture share one slot; F-14 has
//! sixty inserts and one picture between them.

use std::collections::HashMap;
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
    /// reaches the transmitted-light buffer, z = image mode (the picture as it
    /// is, not lit).
    blend: [f32; 4],
}

/// One vertex of a light's mesh: where it is, and where on the insert's
/// picture it looks (`light.cpp:515-520`). A light with no picture carries
/// zeros there; the untextured techniques never read them.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuLightVertex {
    pos: [f32; 3],
    uv: [f32; 2],
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
    /// The insert's picture and the material it is lit through, when it has a
    /// picture that resolved. `None` is the original's null `offTexel`
    /// (`light.cpp:708`), which takes the halo-only technique (`:823`).
    texel: Option<wgpu::BindGroup>,
    /// Whether the light is drawn at zero intensity. See
    /// `vpw_table::light::Light::drawn_when_off`.
    drawn_when_off: bool,
}

/// Which of the four draws a call is making. They differ in the blend, in
/// whether they test depth, and in which lights they take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    /// A flat additive disc, depth-tested: the classic light with no picture.
    Classic,
    /// The classic light with a picture — the lit insert — depth-tested and
    /// added one-to-one.
    Texel,
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
    /// The lit insert: a picture lit through its surface's material with the
    /// halo folded in. See `fs_texel`.
    texel: wgpu::RenderPipeline,
    pub layout: wgpu::BindGroupLayout,
    /// What goes in the material's slot for a light that has no picture.
    ///
    /// The light's own data is at group 2 so that a lit insert can take the
    /// material shader's group 1 unchanged, and a light without a picture has
    /// nothing to put there. WebGPU lets a pipeline layout leave the slot
    /// empty, but whether a draw then has to bind *something* to it is the
    /// kind of question two implementations answer differently; an empty
    /// bind group is the answer they all accept.
    empty: wgpu::BindGroup,
    pub lights: Vec<Light>,
    /// The lamps' names, in the same order, so a caller can find the one the
    /// script is talking about.
    pub names: Vec<String>,
}

impl Lights {
    /// Builds the pipelines against the table's own layouts.
    ///
    /// The lit insert needs the material shader's frame — the environment
    /// map, for the light loop — and its material layout; the halos need only
    /// the frame uniform. The transmitted-light pass is drawn with the frame
    /// layout that leaves the transmitted-light texture *out*, because that
    /// pass is the one writing it.
    pub fn new(
        device: &wgpu::Device,
        pipeline: &crate::pipeline::TablePipeline,
        color_format: wgpu::TextureFormat,
    ) -> Self {
        // One module, the material shader with the light stages after it. See
        // the header of `light.wgsl` for why the two share a module.
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vpw-light-shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}{}",
                    include_str!("material.wgsl"),
                    include_str!("light.wgsl")
                )
                .into(),
            ),
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
        let empty_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vpw-light-empty-layout"),
            entries: &[],
        });
        let empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vpw-light-empty-bg"),
            layout: &empty_layout,
            entries: &[],
        });

        // Three pipeline layouts: the halos in the scene, the halos in the
        // transmitted-light pass, and the lit insert with its material.
        let halo_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vpw-light-pipeline-layout"),
            bind_group_layouts: &[
                Some(&pipeline.frame_layout),
                Some(&empty_layout),
                Some(&layout),
            ],
            immediate_size: 0,
        });
        let transmitted_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vpw-light-transmitted-pipeline-layout"),
            bind_group_layouts: &[
                Some(&pipeline.light_frame_layout),
                Some(&empty_layout),
                Some(&layout),
            ],
            immediate_size: 0,
        });
        let texel_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vpw-light-texel-pipeline-layout"),
            bind_group_layouts: &[
                Some(&pipeline.frame_layout),
                Some(&pipeline.material_layout),
                Some(&layout),
            ],
            immediate_size: 0,
        });

        let attributes = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuLightVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 12,
                    shader_location: 1,
                },
            ],
        })];

        // In the scene the halo must be occluded — an insert under a ramp is
        // not visible through it — so it tests depth without writing it. In the
        // transmitted-light pass there is no depth buffer at all and there
        // should not be: the question there is where the lamp light *is*, not
        // what can see it, and the original disables z for the same pass with
        // the same reasoning (`Renderer.cpp:1496`).
        let tested = || {
            Some(wgpu::DepthStencilState {
                format: crate::pipeline::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            })
        };

        let make = |name: &str,
                    pipeline_layout: &wgpu::PipelineLayout,
                    entry: &str,
                    blend: wgpu::BlendComponent,
                    depth: Option<wgpu::DepthStencilState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_light"),
                    buffers: &attributes,
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: Some(wgpu::BlendState {
                            color: blend,
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

        // Additive: the light **adds** to what is already there. The alpha the
        // shader returns modulates how much it adds at the edge of the halo,
        // where the attenuation has already faded it out.
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline_classic = make("vpw-lights", &halo_layout, "fs_classic", additive, tested());
        let flat = make(
            "vpw-lights-flat",
            &transmitted_layout,
            "fs_transmitted",
            additive,
            None,
        );
        // The bulb halo's blend, which is where a bulb light differs from a
        // classic one. See `fs_bulb` in `light.wgsl` for what the three
        // settings compute; `light.cpp:827` is where the original sets them.
        let bulb = make(
            "vpw-lights-bulb",
            &halo_layout,
            "fs_bulb",
            wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrc,
                operation: wgpu::BlendOperation::ReverseSubtract,
            },
            tested(),
        );
        // The lit insert is added whole — `SRCBLEND ONE`, `DESTBLEND ONE`
        // (`light.cpp:815-817`), with the note that "TOTAN and Flintstones
        // inserts break if alpha blending is disabled here". Its alpha is the
        // overlay's and means nothing to the blend; scaling by it, the way the
        // halo is, would fade the artwork by the picture's own alpha channel.
        let texel = make(
            "vpw-lights-texel",
            &texel_layout,
            "fs_texel",
            wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            tested(),
        );

        Self {
            pipeline: pipeline_classic,
            bulb,
            flat,
            texel,
            layout,
            empty,
            lights: Vec::new(),
            names: Vec::new(),
        }
    }

    /// Uploads a table's lights, pictures included.
    ///
    /// The scene is needed and not just its lights: an insert's picture is
    /// one of the table's images and the material it is lit through is one of
    /// the table's materials, both resolved by name the way the original's
    /// `GetImage` and `GetSurfaceMaterial` resolve them (`light.cpp:373`,
    /// `:708`).
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &crate::pipeline::TablePipeline,
        scene: &vpw_table::geometry::Scene,
    ) {
        let lights = &scene.lights;
        self.names = lights.iter().map(|l| l.name.clone()).collect();

        // The same sampler and the same fallback as the table's pieces, so an
        // insert samples its picture exactly the way the playfield under it
        // samples the same picture. The original asks for clamp addressing
        // here (`light.cpp:811`) where the table's pieces wrap; the insert's
        // coordinates are inside the table and so inside 0..1, and the one
        // texel at the border where the two differ is not worth a second
        // sampler.
        let sampler = crate::scene::table_sampler(device);
        let white = crate::scene::white_texture(device, queue);
        // One slot per (surface material, picture): the picture is uploaded
        // once, not once per insert.
        let mut slots: HashMap<(String, String), Option<wgpu::BindGroup>> = HashMap::new();

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
                    blend: [
                        l.modulate,
                        l.transmission_scale,
                        if l.image_mode { 1.0 } else { 0.0 },
                        0.0,
                    ],
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

                let texel = if l.image.is_empty() {
                    None
                } else {
                    let key = (
                        l.surface_material.to_ascii_lowercase(),
                        l.image.to_ascii_lowercase(),
                    );
                    slots
                        .entry(key)
                        .or_insert_with(|| {
                            let slot = crate::scene::material_slot(
                                device,
                                queue,
                                &pipeline.material_layout,
                                &sampler,
                                &white,
                                scene.material(&l.surface_material),
                                scene.image(&l.image),
                            );
                            // A name that is not one of the table's images is
                            // the original's null `offTexel`: the halo alone.
                            slot.textured.then_some(slot.bind_group)
                        })
                        .clone()
                };

                let vertices: Vec<GpuLightVertex> = l
                    .vertices
                    .iter()
                    .enumerate()
                    .map(|(i, &pos)| GpuLightVertex {
                        pos,
                        uv: l.uvs.get(i).copied().unwrap_or([0.0; 2]),
                    })
                    .collect();

                Light {
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("vpw-light-vertices"),
                        contents: bytemuck::cast_slice(&vertices),
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
                    // Only with a picture that resolved: a light whose picture
                    // is missing falls back to the halo, and a halo at zero
                    // intensity is nothing.
                    drawn_when_off: texel.is_some() && l.drawn_when_off(),
                    texel,
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

    /// How many of the uploaded lights are lit inserts — a picture lit through
    /// a material — rather than halos.
    pub fn textured(&self) -> usize {
        self.lights.iter().filter(|l| l.texel.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.lights.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lights.len()
    }

    /// Emits the lights into the scene.
    ///
    /// `frame` is the table's full frame bind group, environment and all: the
    /// lit insert goes through the light loop, and the light loop reads the
    /// environment map.
    ///
    /// Three pipelines, chosen per light: a bulb blends into what is under it,
    /// a classic one with a picture adds the lit picture, and one without adds
    /// its halo flat on top. The original switches the same sets of render
    /// states for the same reason (`light.cpp:810-817` and `:827`).
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, frame: &wgpu::BindGroup) {
        self.draw_with(pass, &self.pipeline, Which::Classic, frame);
        self.draw_with(pass, &self.texel, Which::Texel, frame);
        self.draw_with(pass, &self.bulb, Which::Bulb, frame);
    }

    /// The same, into a pass with no depth buffer. `frame` is the reduced
    /// frame bind group, the one without the transmitted-light texture.
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
        if which != Which::Texel {
            pass.set_bind_group(1, &self.empty, &[]);
        }
        for l in &self.lights {
            let takes = match which {
                Which::Classic => !l.bulb && l.texel.is_none(),
                Which::Texel => l.texel.is_some(),
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
            // call, and a table has hundreds of them with a handful lit — bar
            // the insert with a picture of its own, which is drawn dark
            // (`light.cpp:713-718`), and only in the scene: in the
            // transmitted-light buffer dark is nothing.
            if l.lamp.level() <= 0.0 && !(which == Which::Texel && l.drawn_when_off) {
                continue;
            }
            if let Some(texel) = &l.texel
                && which == Which::Texel
            {
                pass.set_bind_group(1, texel, &[]);
            }
            pass.set_bind_group(2, &l.bind_group, &[]);
            pass.set_vertex_buffer(0, l.vertices.slice(..));
            pass.set_index_buffer(l.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..l.index_count, 0, 0..1);
        }
    }
}
