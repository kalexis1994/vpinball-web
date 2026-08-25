//! The flashers on the GPU.
//!
//! A pipeline of their own and a place of their own in the pass, after the
//! rest of the table: a flasher is drawn blended over what is already there
//! and writes no depth (`flasher.cpp:1300`), so nothing after it could be
//! occluded by it anyway. What it *tests* depth against is the whole scene,
//! which is what keeps a strobe under a ramp from shining through the ramp.
//!
//! # What changes, and how often
//!
//! The outline is uploaded once. What a script writes — on, off, dimmer,
//! colour, which picture, where — is a block of five vectors per flasher, and
//! the block is rewritten only when the numbers in it move. Measured the same
//! way as the moving parts: on a table with forty-seven flashers the typical
//! frame changes none of them, and the frame that fires a strobe changes one.
//!
//! # The two blends
//!
//! An additive flasher goes through the reverse-subtract blend the bulb light
//! uses (`flasher.cpp:1303-1304`, and `fs_bulb` in `light.wgsl` for the
//! algebra); a painted one is a plain alpha blend. They are two pipelines
//! rather than a switch in the shader because the blend is fixed-function.
//!
//! # The display
//!
//! A flasher in DMD mode shows the machine's dot matrix. The frame is one
//! texel per dot, and the shader draws the dots the way the original's legacy
//! renderer does (`fs_dmd.sc`); see [`Flashers::set_dmd`] for how the frame
//! gets here.

use crate::scene::{table_sampler, upload_texture, white_texture};
use std::collections::HashMap;
use vpw_table::flasher::{Flasher, RenderMode, State};
use vpw_table::geometry::Scene;
use wgpu::util::DeviceExt;

/// Mirrors `struct FlasherData` in `flasher.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuFlasher {
    model: [[f32; 4]; 4],
    color: [f32; 4],
    tests: [f32; 4],
    blend: [f32; 4],
    res: [f32; 4],
}

/// A corner of the outline: position on the table and texel.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuFlasherVertex {
    pos: [f32; 3],
    uv: [f32; 2],
}

/// A picture a flasher can show, by the name the table uses for it.
struct Picture {
    view: wgpu::TextureView,
    /// The alpha below which a texel is thrown away, from the image
    /// (`Texture.h:175`, `m_alphaTestValue`), or negative for never.
    alpha_test: f32,
}

/// One uploaded flasher.
struct Entry {
    shape: Flasher,
    /// What the script last said.
    state: State,
    /// The lightmap's contribution to the alpha: the bound lamp's level over
    /// its full level (`flasher.cpp:1171-1177`), or one when there is no
    /// lamp. Separate from the state because it comes from a different part
    /// of the table.
    light_scale: f32,
    first_index: u32,
    index_count: u32,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Exactly what is in the uniform, so an unchanged frame writes nothing.
    data: GpuFlasher,
    /// The picture names the bind group was built with, so a script that
    /// swaps `ImageA` gets a new bind group and one that rewrites the same
    /// name does not.
    bound: (String, String),
}

/// The machine's dot matrix as a texture, one texel per dot.
struct DmdTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

pub struct Flashers {
    /// Painted: alpha blend.
    alpha: wgpu::RenderPipeline,
    /// Additive: the modulating reverse-subtract blend.
    add: wgpu::RenderPipeline,
    /// The display, painted and additive (`EnableAlphaBlend`,
    /// `RenderDevice.cpp:2497-2505`, as `flasher.cpp:1332` calls it).
    dmd_alpha: wgpu::RenderPipeline,
    dmd_add: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// For the display alone: clamped, where the table's sampler repeats.
    ///
    /// The dot shader jitters its samples up to a texel and a half past the
    /// fragment, and at the panel's edge that reaches beyond the frame. With
    /// a repeating sampler it lands on the *far* column of dots, and the
    /// right-hand edge of the display shows a sliver of whatever is lit on
    /// the left — found by a test that lit half a panel and saw the dark half
    /// change at its outer edge.
    clamp: wgpu::Sampler,
    /// Bound wherever a picture is missing: the shader always samples both.
    white: wgpu::TextureView,
    /// The pictures the table's flashers name, by lower-cased name — the
    /// table resolves images case-insensitively (`pintable.cpp:4232`).
    pictures: HashMap<String, Picture>,
    dmd: DmdTexture,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
    entries: Vec<Entry>,
    /// The flashers' names, in the same order, so a host can find the one
    /// the script is writing to.
    pub names: Vec<String>,
    /// Counts the display frames, for the dot shader's moving dither.
    frame: u32,
}

/// The display's size before a machine has sent a frame. The most common
/// panel there is, and the size is only a placeholder: the texture is rebuilt
/// to whatever arrives.
const DMD_PLACEHOLDER: (u32, u32) = (128, 32);

impl Flashers {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vpw-flasher-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("flasher.wgsl").into()),
        });

        let texture = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vpw-flasher-layout"),
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
                texture(1),
                texture(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vpw-flasher-pipeline-layout"),
            bind_group_layouts: &[Some(frame_layout), Some(&layout)],
            immediate_size: 0,
        });

        let attributes = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuFlasherVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2],
        })];

        let make = |name: &str, entry: &str, blend: wgpu::BlendComponent| {
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
                    // `CULL_NONE`, `flasher.cpp:1239`: a flasher stood on its
                    // edge is meant to be seen from either side.
                    cull_mode: None,
                    ..Default::default()
                },
                // Tested, never written (`ZWRITEENABLE` false,
                // `flasher.cpp:1300`, `:1335`).
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
            })
        };

        // `SRC_ALPHA`, `INVSRC_ALPHA`, `ADD` (`flasher.cpp:1302-1304`, the
        // non-additive arm).
        let painted = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        // `SRC_ALPHA`, `INVSRC_COLOR`, `REVSUBTRACT`: the additive arm of the
        // same lines, and the blend `fs_bulb` documents.
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrc,
            operation: wgpu::BlendOperation::ReverseSubtract,
        };
        // The display's additive blend is the ordinary one: `SRC_ALPHA`,
        // `ONE`, `ADD` (`RenderDevice.cpp:2502`).
        let dmd_additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };

        let dmd = Self::dmd_texture(device, DMD_PLACEHOLDER.0, DMD_PLACEHOLDER.1);

        Self {
            alpha: make("vpw-flasher", "fs_flasher", painted),
            add: make("vpw-flasher-add", "fs_flasher", additive),
            dmd_alpha: make("vpw-flasher-dmd", "fs_dmd", painted),
            dmd_add: make("vpw-flasher-dmd-add", "fs_dmd", dmd_additive),
            layout,
            sampler: table_sampler(device),
            clamp: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("vpw-flasher-dmd-sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            white: white_texture(device, queue),
            pictures: HashMap::new(),
            dmd,
            vertices: None,
            indices: None,
            entries: Vec::new(),
            names: Vec::new(),
            frame: 0,
        }
    }

    /// A texture for the dot matrix: sRGB, so the level stored in it is
    /// decoded to linear on the way out. The original applies that decode to
    /// the frame itself ("it is already applied to DMD texture",
    /// `fs_dmd.sc:151`); letting the sampler do it is the same number with
    /// less arithmetic.
    fn dmd_texture(device: &wgpu::Device, width: u32, height: u32) -> DmdTexture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vpw-flasher-dmd"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        DmdTexture {
            texture,
            view,
            width: width.max(1),
            height: height.max(1),
        }
    }

    /// Uploads a table's flashers, every one of them, shown or not.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, scene: &Scene) {
        self.entries.clear();
        self.pictures.clear();
        self.names = scene.flashers.iter().map(|f| f.name.clone()).collect();

        // The pictures the flashers name, uploaded once each. A picture a
        // script switches to later has to be one of these: the scene is not
        // kept around after upload, and holding every image of a hundred
        // megabyte table against the chance is not a trade a phone can make.
        // The original with a missing picture draws the flat colour, and so
        // does this.
        for f in &scene.flashers {
            for name in [&f.state.image_a, &f.state.image_b] {
                let key = name.to_ascii_lowercase();
                if key.is_empty() || self.pictures.contains_key(&key) {
                    continue;
                }
                let Some(image) = scene.image(name) else {
                    continue;
                };
                let Some((_, view)) = upload_texture(device, queue, image) else {
                    continue;
                };
                self.pictures.insert(
                    key,
                    Picture {
                        view,
                        // `ramp.cpp:907` tests only where the picture has an
                        // alpha channel to test; a JPEG's texels are all
                        // opaque and the test would be against nothing.
                        alpha_test: if image.has_alpha {
                            image.alpha_test
                        } else {
                            -1.0
                        },
                    },
                );
            }
        }

        let mut vertices: Vec<GpuFlasherVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        for f in &scene.flashers {
            let first_index = indices.len() as u32;
            let base = vertices.len() as u32;
            vertices.extend(f.vertices.iter().map(|v| GpuFlasherVertex {
                pos: [v.pos[0], v.pos[1], 0.0],
                uv: v.uv,
            }));
            indices.extend(f.indices.iter().map(|i| i + base));

            let state = f.state.clone();
            let data = self.compute(f, &state, 1.0);
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vpw-flasher-data"),
                contents: bytemuck::bytes_of(&data),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bound = (
                state.image_a.to_ascii_lowercase(),
                state.image_b.to_ascii_lowercase(),
            );
            let bind_group = self.bind_group(device, f.mode, &uniform, &bound);
            self.entries.push(Entry {
                shape: f.clone(),
                state,
                light_scale: 1.0,
                first_index,
                index_count: indices.len() as u32 - first_index,
                uniform,
                bind_group,
                data,
                bound,
            });
        }

        self.vertices = (!vertices.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vpw-flasher-vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        self.indices = (!indices.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vpw-flasher-indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            })
        });
    }

    /// Which pictures a flasher actually has, the way `Flasher::Render`
    /// resolves them: a flasher with only `ImageB` shows it as its one
    /// picture (`flasher.cpp:1267-1274`).
    fn resolve(&self, bound: &(String, String)) -> (Option<&Picture>, Option<&Picture>) {
        let a = self.pictures.get(&bound.0);
        let b = self.pictures.get(&bound.1);
        match (a, b) {
            (None, Some(b)) => (Some(b), None),
            other => other,
        }
    }

    fn bind_group(
        &self,
        device: &wgpu::Device,
        mode: RenderMode,
        uniform: &wgpu::Buffer,
        bound: &(String, String),
    ) -> wgpu::BindGroup {
        let (a, b) = match mode {
            // The display goes where the first picture would: `ImageA` on a
            // display flasher is the glass over it (`flasher.cpp:1330`),
            // which is not drawn here.
            RenderMode::Dmd => (&self.dmd.view, &self.white),
            _ => {
                let (a, b) = self.resolve(bound);
                (
                    a.map_or(&self.white, |p| &p.view),
                    b.map_or(&self.white, |p| &p.view),
                )
            }
        };
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vpw-flasher-bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(a),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(b),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(match mode {
                        RenderMode::Dmd => &self.clamp,
                        _ => &self.sampler,
                    }),
                },
            ],
        })
    }

    /// The uniform block for a flasher in a given state: `Flasher::Render`,
    /// `flasher.cpp:1241-1294` for a picture flasher and `:1336-1340` with
    /// `Renderer::SetupDMDRender` (`Renderer.cpp:1428-1432`) for a display.
    fn compute(&self, shape: &Flasher, s: &State, light_scale: f32) -> GpuFlasher {
        let model = shape.transform(s).to_cols_array_2d();
        let mut color = s.shader_color();
        // The lightmap scales the alpha before anything else looks at it
        // (`flasher.cpp:1169-1177`).
        color[3] *= light_scale;

        match shape.mode {
            RenderMode::Dmd => {
                // `vColor_Intensity`: the colour times the brightness, and the
                // brightness is the alpha (`Renderer.cpp:1430`, `color.w`).
                let a = color[3];
                GpuFlasher {
                    model,
                    color: [color[0] * a, color[1] * a, color[2] * a, a],
                    tests: [0.0; 4],
                    blend: [0.0, 0.0, 0.0, (self.frame % 2048) as f32],
                    // `vRes_Alpha_time`: dots across and down, and the
                    // display's opacity — which is `modulate_vs_add`, not the
                    // flasher's alpha (`flasher.cpp:1338`, the `alpha`
                    // argument).
                    res: [
                        self.dmd.width as f32,
                        self.dmd.height as f32,
                        s.modulate_vs_add,
                        0.0,
                    ],
                }
            }
            _ => {
                let bound = (
                    s.image_a.to_ascii_lowercase(),
                    s.image_b.to_ascii_lowercase(),
                );
                let (a, b) = self.resolve(&bound);
                let mode = match (a, b) {
                    (Some(_), None) => 0.0,
                    (Some(_), Some(_)) => 1.0,
                    _ => 2.0,
                };
                // The alpha test is only for a painted flasher: an additive
                // one leaves both at the "never" of minus one
                // (`flasher.cpp:1252`, `:1264`, `:1272`, `:1284`).
                let test = |p: Option<&Picture>| {
                    if s.add_blend {
                        -1.0
                    } else {
                        p.map_or(-1.0, |p| p.alpha_test)
                    }
                };
                GpuFlasher {
                    model,
                    color,
                    tests: [
                        test(a),
                        test(b),
                        s.filter as u8 as f32,
                        if s.add_blend { 1.0 } else { 0.0 },
                    ],
                    blend: [s.filter_amount / 100.0, s.clamped_modulate(), mode, 0.0],
                    res: [0.0; 4],
                }
            }
        }
    }

    /// Tells flasher `index` what the script has set.
    ///
    /// `light_scale` is the bound lamp's level over its full level, or one
    /// for a flasher with no lamp; see [`Flashers::light_map`]. Writes nothing
    /// when nothing changed, and rebuilds the bind group only when a picture
    /// was swapped.
    pub fn set_state(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        index: usize,
        state: &State,
        light_scale: f32,
    ) {
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        if entry.state == *state && entry.light_scale == light_scale {
            return;
        }
        let data = self.compute(&entry.shape, state, light_scale);
        let bound = (
            state.image_a.to_ascii_lowercase(),
            state.image_b.to_ascii_lowercase(),
        );
        let rebind = bound != entry.bound && entry.shape.mode != RenderMode::Dmd;
        let bind_group =
            rebind.then(|| self.bind_group(device, entry.shape.mode, &entry.uniform, &bound));

        let entry = &mut self.entries[index];
        entry.state = state.clone();
        entry.light_scale = light_scale;
        if let Some(bg) = bind_group {
            entry.bind_group = bg;
            entry.bound = bound;
        }
        if data != entry.data {
            entry.data = data;
            queue.write_buffer(&entry.uniform, 0, bytemuck::bytes_of(&data));
        }
    }

    /// The name of the lamp flasher `index` follows, if the file bound one
    /// (`LMAP`).
    pub fn light_map(&self, index: usize) -> Option<&str> {
        self.entries
            .get(index)
            .and_then(|e| e.shape.light_map.as_deref())
    }

    /// What flasher `index` is showing, as last set.
    pub fn state(&self, index: usize) -> Option<&State> {
        self.entries.get(index).map(|e| &e.state)
    }

    /// Whether any flasher shows the machine's display.
    pub fn shows_dmd(&self) -> bool {
        self.entries.iter().any(|e| e.shape.mode == RenderMode::Dmd)
    }

    /// Puts a frame of the machine's dot matrix on every display flasher.
    ///
    /// `dots` is one byte per dot, row-major, top row first, from zero (dark)
    /// to three (full): the four levels a PinMAME-style board makes by
    /// showing a one-bit frame for a fraction of the cycle (`vpw_ws::dmd`).
    /// Each becomes a texel; the shader draws the dot. The texture is
    /// rebuilt when a different size arrives, which is once per machine.
    pub fn set_dmd(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dots: &[u8],
        width: usize,
        height: usize,
    ) {
        if width == 0 || height == 0 || dots.len() < width * height {
            return;
        }
        let (w, h) = (width as u32, height as u32);
        if (self.dmd.width, self.dmd.height) != (w, h) {
            self.dmd = Self::dmd_texture(device, w, h);
            // The display's bind groups hold the old view.
            for i in 0..self.entries.len() {
                if self.entries[i].shape.mode != RenderMode::Dmd {
                    continue;
                }
                let bg = self.bind_group(
                    device,
                    RenderMode::Dmd,
                    &self.entries[i].uniform,
                    &self.entries[i].bound,
                );
                self.entries[i].bind_group = bg;
            }
        }

        // Level over three, out of 255, in every channel. Stored sRGB, so the
        // sampler hands the shader the linear light of a dot at that level.
        let mut rgba = Vec::with_capacity(width * height * 4);
        for &level in &dots[..width * height] {
            let v = level.min(3) * 85;
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.dmd.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        // A new frame moves the dither (`vRes_Alpha_time.w` is the frame
        // count, `Renderer.cpp:1431`). The original advances it every
        // rendered frame; here it moves with the display, which is what the
        // shimmer is for anyway — and it also carries the size into the
        // uniform when the texture was rebuilt.
        self.frame = self.frame.wrapping_add(1);
        for i in 0..self.entries.len() {
            if self.entries[i].shape.mode != RenderMode::Dmd {
                continue;
            }
            let data = self.compute(
                &self.entries[i].shape,
                &self.entries[i].state,
                self.entries[i].light_scale,
            );
            let entry = &mut self.entries[i];
            if data != entry.data {
                entry.data = data;
                queue.write_buffer(&entry.uniform, 0, bytemuck::bytes_of(&data));
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Emits the flashers, back to front.
    ///
    /// Group 0 is the camera-only frame group the lights use. The order is
    /// the original's for transparent draws: by `depthBias - z`, largest
    /// first (`RenderPass.cpp:118-121`, `RenderDevice.cpp:2708`), and stable,
    /// "since we don't want to change the order of blended draw calls between
    /// frames". A display flasher sorts ten thousand further back so it lands
    /// under every picture flasher (`flasher.cpp:1341-1343`).
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, frame: &wgpu::BindGroup) {
        let (Some(vertices), Some(indices)) = (&self.vertices, &self.indices) else {
            return;
        };
        let mut order: Vec<usize> = (0..self.entries.len())
            .filter(|&i| {
                let e = &self.entries[i];
                // `flasher.cpp:1162`, `:1179`: hidden, black or faded to
                // nothing is an early return, and a display without a frame
                // yet is too (`:1328`) — here that is a texture of dark dots,
                // which draws nothing either.
                matches!(e.shape.mode, RenderMode::Flasher | RenderMode::Dmd)
                    && e.state.is_drawn()
                    && e.data.color[3] > 0.0
            })
            .collect();
        if order.is_empty() {
            return;
        }
        order.sort_by(|&a, &b| {
            let (ea, eb) = (&self.entries[a], &self.entries[b]);
            eb.shape
                .sort_depth(&eb.state)
                .total_cmp(&ea.shape.sort_depth(&ea.state))
        });

        pass.set_bind_group(0, frame, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        let mut current: Option<&wgpu::RenderPipeline> = None;
        for i in order {
            let e = &self.entries[i];
            let pipeline = match (e.shape.mode, e.state.add_blend) {
                (RenderMode::Dmd, false) => &self.dmd_alpha,
                (RenderMode::Dmd, true) => &self.dmd_add,
                (_, false) => &self.alpha,
                (_, true) => &self.add,
            };
            if !current.is_some_and(|c| std::ptr::eq(c, pipeline)) {
                pass.set_pipeline(pipeline);
                current = Some(pipeline);
            }
            pass.set_bind_group(1, &e.bind_group, &[]);
            pass.draw_indexed(e.first_index..e.first_index + e.index_count, 0, 0..1);
        }
    }
}
