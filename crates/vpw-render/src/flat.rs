//! The flat engine: the table photographed once and played as pictures.
//!
//! For a machine whose GPU cannot afford the table — six hundred thousand
//! triangles through a physically-based shader, twice with the reflection
//! probe — this trades all of it for a handful of textured quads, the way the
//! nineties pinball games did, and leans on one physical fact to stay honest:
//! **light is additive**. Photograph the table dark, photograph it with one
//! lamp lit, and the difference of the two photographs is that lamp's light —
//! its halo, its glow through the plastics, its share of the baked lightmap,
//! everything the real renderer would have drawn for it. Composite
//! `base + Σ level·layer` in HDR, before the tone map, and the arithmetic is
//! the scene's own.
//!
//! What stays live, in real 3D, is everything that moves: the balls, the
//! flippers and the rest of the [`DynamicParts`], the flashers (one of them is
//! the DMD), and the head's score display. A few thousand triangles against
//! the table's fraction of a million. They draw *into* the photograph's own
//! depth — the bake keeps it — so a ball still disappears behind a post that
//! is now only a picture of a post.
//!
//! The bake itself is resumable: a few photographs per frame while the real
//! renderer keeps playing, and the switch happens when the last lamp is done.
//! Nothing here reads back to the CPU — each lamp's screen rectangle comes
//! from projecting its own falloff sphere, and the photographs go straight
//! into an atlas.

use crate::dynamic::DynamicParts;
use crate::flashers::Flashers;
use crate::lights::{Gi, Lights};
use crate::pipeline::TablePipeline;
use crate::post::Post;
use crate::scene::{GpuScene, GpuVertex};
use vpw_math::{Mat4, Vec3};

/// A photographed lamp: where its light lands on screen, and where in the
/// atlas the photograph went.
struct Layer {
    light: usize,
    /// Clip-space rect: x0, y0 (bottom-left) to x1, y1 (top-right).
    rect: [f32; 4],
    /// The atlas slot as uv: top-left, then bottom-right.
    uv: [f32; 4],
    page: u32,
    /// The slot in atlas pixels: x, y, width, height.
    slot: [u32; 4],
}

/// What one layer instance carries to the vertex stage. Mirrors `LayerIn`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuLayer {
    rect: [f32; 4],
    uv: [f32; 4],
    meta: [f32; 2],
}

/// Mirrors `Slot` in `flat.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuSlot {
    origin: [f32; 4],
    step: [f32; 4],
}

/// Everything that exists once the base photograph is taken.
struct Baked {
    base: wgpu::TextureView,
    /// Depth in an `R32Float` colour texture. See the shader module note.
    depth: wgpu::TextureView,
    /// A real depth buffer for the keep-depth pass itself to sort with.
    scratch_depth: wgpu::TextureView,
    /// One view per page, to photograph into.
    pages: Vec<wgpu::TextureView>,
    live_bind: wgpu::BindGroup,
    blit_bind: wgpu::BindGroup,
    diff_bind: wgpu::BindGroup,
    layers: Vec<Layer>,
    instances: wgpu::Buffer,
    size: (u32, u32),
}

enum State {
    /// Nothing baked, or what was baked no longer matches the world.
    Invalid,
    /// The base is photographed; lamps from `next` on still need theirs.
    Baking {
        next: usize,
    },
    Ready,
}

pub struct Flat {
    live_layout: wgpu::BindGroupLayout,
    bake_layout: wgpu::BindGroupLayout,
    base_pipeline: wgpu::RenderPipeline,
    layer_pipeline: wgpu::RenderPipeline,
    blit_pipeline: wgpu::RenderPipeline,
    diff_pipeline: wgpu::RenderPipeline,
    /// The static scene's geometry alone, for the depth the photograph keeps.
    depth_only: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    slot_uniform: wgpu::Buffer,
    state: State,
    baked: Option<Baked>,
}

/// The batches the photograph excludes and the live pass draws instead: the
/// head's score display, whose texture changes every frame.
fn is_live(b: &crate::scene::Batch) -> bool {
    b.image == vpw_table::backbox::DISPLAY_IMAGE
}

impl Flat {
    pub fn new(
        device: &wgpu::Device,
        pipeline: &TablePipeline,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vpw-flat-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("flat.wgsl").into()),
        });

        let texture = |binding, dimension, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: dimension,
                multisampled: false,
            },
            count: None,
        };
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let live_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vpw-flat-live-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(1, wgpu::TextureViewDimension::D2, float),
                // Non-filterable: the depth-keeping texture is `R32Float`,
                // and it is only ever `textureLoad`ed.
                texture(
                    2,
                    wgpu::TextureViewDimension::D2,
                    wgpu::TextureSampleType::Float { filterable: false },
                ),
                texture(3, wgpu::TextureViewDimension::D2Array, float),
            ],
        });
        let bake_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vpw-flat-bake-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(5, wgpu::TextureViewDimension::D2, float),
                texture(6, wgpu::TextureViewDimension::D2, float),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let live_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vpw-flat-live-pipeline-layout"),
            bind_group_layouts: &[Some(&live_layout)],
            immediate_size: 0,
        });
        let bake_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vpw-flat-bake-pipeline-layout"),
            bind_group_layouts: &[Some(&bake_layout)],
            immediate_size: 0,
        });

        // The base draw restores the photograph's depth through `frag_depth`,
        // so it writes depth unconditionally; the layers touch neither.
        let base_depth = wgpu::DepthStencilState {
            format: crate::pipeline::DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        };
        let layer_depth = wgpu::DepthStencilState {
            depth_write_enabled: Some(false),
            ..base_depth.clone()
        };

        let base_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vpw-flat-base"),
            layout: Some(&live_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_full"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_base"),
                targets: &[Some(format.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: Some(base_depth),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });

        let layer_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vpw-flat-layer"),
            layout: Some(&live_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_layer"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuLayer>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 16,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 32,
                            shader_location: 2,
                        },
                    ],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_layer"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Added whole: the fragment already scaled itself by the
                    // lamp's level, and a negative difference subtracts.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: Some(layer_depth),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });

        let bake = |name: &str, entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&bake_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_full"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(format.into())],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let blit_pipeline = bake("vpw-flat-blit", "fs_blit");
        let diff_pipeline = bake("vpw-flat-diff", "fs_diff");

        // The static scene's vertex stage plus a one-line fragment that
        // writes the fragment's own depth into a colour channel. Written to a
        // colour texture rather than kept as a depth texture because the live
        // pass has to read it back, and the WebGL2 backend cannot fetch from
        // a depth texture.
        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vpw-flat-depth-shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}{}{}",
                    include_str!("material.wgsl"),
                    include_str!("table_vs.wgsl"),
                    "@fragment
fn fs_keep_depth(in : VsOut) -> @location(0) vec4<f32> {
                         return vec4<f32>(in.clip.z, 0.0, 0.0, 1.0);
}
"
                )
                .into(),
            ),
        });
        let depth_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vpw-flat-depth-pipeline-layout"),
            bind_group_layouts: &[
                Some(&pipeline.frame_layout),
                Some(&pipeline.material_layout),
            ],
            immediate_size: 0,
        });
        let depth_only = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vpw-flat-depth"),
            layout: Some(&depth_layout),
            vertex: wgpu::VertexState {
                module: &scene_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(GpuVertex::layout())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &scene_shader,
                entry_point: Some("fs_keep_depth"),
                targets: &[Some(wgpu::TextureFormat::R32Float.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::pipeline::DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vpw-flat-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let slot_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vpw-flat-slot"),
            size: std::mem::size_of::<GpuSlot>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            live_layout,
            bake_layout,
            base_pipeline,
            layer_pipeline,
            blit_pipeline,
            diff_pipeline,
            depth_only,
            sampler,
            slot_uniform,
            state: State::Invalid,
            baked: None,
        }
    }

    /// Whatever was baked no longer matches the world: a resize, a new view,
    /// a different room, a lightmap that arrived. The next `bake_step` starts
    /// over.
    pub fn invalidate(&mut self) {
        self.state = State::Invalid;
        self.baked = None;
    }

    pub fn ready(&self) -> bool {
        matches!(self.state, State::Ready)
    }

    /// Projects a lamp's reach onto the screen. Conservative: the corners of
    /// the box around its falloff sphere, unioned with its mesh's own extent,
    /// expanded a little for the light that creeps along geometry.
    fn screen_rect(vp: Mat4, foot: &crate::lights::Footprint) -> Option<[f32; 4]> {
        let (bmin, bmax) = (
            foot.bounds.0.min(foot.center - Vec3::splat(foot.radius)),
            foot.bounds.1.max(foot.center + Vec3::splat(foot.radius)),
        );
        let mut min = [f32::MAX; 2];
        let mut max = [f32::MIN; 2];
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { bmin.x } else { bmax.x },
                if i & 2 == 0 { bmin.y } else { bmax.y },
                if i & 4 == 0 { bmin.z } else { bmax.z },
            );
            let clip = vp * corner.extend(1.0);
            if clip.w <= 0.0 {
                // A corner behind the eye has no screen position; give the
                // lamp the whole screen rather than a wrong crop.
                return None;
            }
            let x = clip.x / clip.w;
            let y = clip.y / clip.w;
            min[0] = min[0].min(x);
            min[1] = min[1].min(y);
            max[0] = max[0].max(x);
            max[1] = max[1].max(y);
        }
        // A quarter more on every side: transmitted glow on the plastics
        // above the lamp lands near it on screen but not inside it.
        let (w, h) = (max[0] - min[0], max[1] - min[1]);
        Some([
            (min[0] - w * 0.25).max(-1.0),
            (min[1] - h * 0.25).max(-1.0),
            (max[0] + w * 0.25).min(1.0),
            (max[1] + h * 0.25).min(1.0),
        ])
    }

    /// Plans every lamp's slot and allocates the textures. The atlas pages
    /// are screen-sized; small slots shelf-pack into them, and the lamps
    /// whose light spans the field — the baked-lightmap ones above all — are
    /// photographed at reduced resolution, because a floodlight is
    /// low-frequency by nature.
    fn plan(
        &mut self,
        device: &wgpu::Device,
        post: &Post,
        lights: &Lights,
        vp: Mat4,
        format: wgpu::TextureFormat,
    ) {
        let (w, h) = post.scene_size();
        let (wf, hf) = (w as f32, h as f32);

        // Slot sizes in pixels, from the projected rects.
        struct Planned {
            light: usize,
            rect: [f32; 4],
            size: (u32, u32),
        }
        let mut planned: Vec<Planned> = Vec::new();
        for i in 0..lights.len() {
            let Some(foot) = lights.footprint(i) else {
                continue;
            };
            if lights.full_level(i) <= 0.0 {
                continue;
            }
            let rect = if foot.baked {
                // Its share of the lightmap reaches the whole field.
                None
            } else {
                Self::screen_rect(vp, &foot)
            };
            let rect = rect.unwrap_or([-1.0, -1.0, 1.0, 1.0]);
            let px_w = (rect[2] - rect[0]) * 0.5 * wf;
            let px_h = (rect[3] - rect[1]) * 0.5 * hf;
            if px_w < 1.0 || px_h < 1.0 {
                continue;
            }
            // Resolution by how much screen the light covers: a floodlight is
            // soft, and a soft thing photographed at quarter size stays soft.
            let coverage = (px_w * px_h) / (wf * hf);
            let res = if foot.baked || coverage > 0.5 {
                0.25
            } else if coverage > 0.1 {
                0.5
            } else {
                1.0
            };
            planned.push(Planned {
                light: i,
                rect,
                size: (
                    ((px_w * res) as u32).clamp(1, w),
                    ((px_h * res) as u32).clamp(1, h),
                ),
            });
        }

        // Shelf-pack, tallest first, into screen-sized pages.
        planned.sort_by_key(|p| std::cmp::Reverse(p.size.1));
        const PAD: u32 = 2;
        let mut layers: Vec<Layer> = Vec::new();
        let (mut page, mut x, mut y, mut shelf) = (0u32, 0u32, 0u32, 0u32);
        for p in &planned {
            let (sw, sh) = (p.size.0.min(w - PAD), p.size.1.min(h - PAD));
            if x + sw + PAD > w {
                x = 0;
                y += shelf + PAD;
                shelf = 0;
            }
            if y + sh + PAD > h {
                page += 1;
                x = 0;
                y = 0;
                shelf = 0;
            }
            layers.push(Layer {
                light: p.light,
                rect: p.rect,
                uv: [
                    x as f32 / wf,
                    y as f32 / hf,
                    (x + sw) as f32 / wf,
                    (y + sh) as f32 / hf,
                ],
                page,
                slot: [x, y, sw, sh],
            });
            x += sw + PAD;
            shelf = shelf.max(sh);
        }
        let pages = page + 1;

        let make_target = |label: &str, format: wgpu::TextureFormat, layers: u32| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let base_tex = make_target("vpw-flat-base", format, 1);
        let depth_tex = make_target("vpw-flat-base-depth", wgpu::TextureFormat::R32Float, 1);
        let scratch_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vpw-flat-scratch-depth"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::pipeline::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        // Two layers minimum: wgpu-hal reads a single layer as a plain
        // picture rather than the array the shader binds.
        let atlas_tex = make_target("vpw-flat-atlas", format, pages.max(2));

        let base = base_tex.create_view(&Default::default());
        let depth = depth_tex.create_view(&Default::default());
        let scratch_depth = scratch_tex.create_view(&Default::default());
        let atlas = atlas_tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let pages_views = (0..pages)
            .map(|i| {
                atlas_tex.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: i,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        let live_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vpw-flat-live-bg"),
            layout: &self.live_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&base),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&depth),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&atlas),
                },
            ],
        });
        let bake_bind = |dark: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vpw-flat-bake-bg"),
                layout: &self.bake_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(post.scene_view()),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(dark),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: self.slot_uniform.as_entire_binding(),
                    },
                ],
            })
        };
        // The blit writes `base`, so its bind group must not also carry it.
        let blit_bind = bake_bind(post.scene_view());
        let diff_bind = bake_bind(&base);

        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vpw-flat-instances"),
            size: (layers.len().max(1) * std::mem::size_of::<GpuLayer>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.baked = Some(Baked {
            base,
            depth,
            scratch_depth,
            pages: pages_views,
            live_bind,
            blit_bind,
            diff_bind,
            layers,
            instances,
            size: (w, h),
        });
    }

    /// One photograph: the static table with the given lamp levels, into the
    /// scene's HDR buffer. The same passes the live renderer runs, minus
    /// everything that moves.
    #[expect(
        clippy::too_many_arguments,
        reason = "a photograph needs the whole studio"
    )]
    fn photograph(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        post: &Post,
        pipeline: &TablePipeline,
        scene: &GpuScene,
        lights: &Lights,
        levels: &[f32],
        head: bool,
        reflection: bool,
    ) {
        // The transmitted-light buffer, with exactly these lamps in it.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vpw-flat-transmitted"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: post.transmission_view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            lights.draw_flat_forced(&mut pass, &pipeline.light_frame_bind_group, levels);
        }
        post.blur_transmission(encoder);

        if reflection && scene.lighting.reflection_strength > 0.0 {
            let (color, resolve) = post.reflection_color();
            crate::pass::draw_reflection(
                encoder,
                color,
                resolve,
                post.reflection_depth(),
                pipeline,
                scene,
                None,
            );
        }

        let (color, resolve) = post.scene_color();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("vpw-flat-photo"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: resolve,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(pipeline.clear),
                    store: if resolve.is_some() {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &post.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &pipeline.frame_bind_group, &[]);
        pass.set_pipeline(&pipeline.opaque);
        scene.draw_filtered(&mut pass, |b| {
            !b.transparent && !b.culled && !is_live(b) && (head || !b.backbox)
        });
        pass.set_pipeline(&pipeline.opaque_culled);
        scene.draw_filtered(&mut pass, |b| {
            b.culled && !is_live(b) && (head || !b.backbox)
        });
        if scene.batches.iter().any(|b| b.transparent) {
            pass.set_pipeline(&pipeline.blended);
            scene.draw_filtered(&mut pass, |b| {
                b.transparent && !is_live(b) && (head || !b.backbox)
            });
        }
        lights.draw_forced(&mut pass, &pipeline.frame_bind_group, levels);
    }

    /// Advances the bake by up to `budget` lamp photographs. Returns whether
    /// the flat scene is ready to draw. The caller keeps rendering the real
    /// scene meanwhile; every submission here restores nothing — the caller's
    /// next frame rewrites the frame uniform and the lamp levels it owns.
    #[expect(
        clippy::too_many_arguments,
        reason = "the bake borrows the renderer piecewise"
    )]
    pub fn bake_step(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &TablePipeline,
        post: &Post,
        scene: &GpuScene,
        lights: &Lights,
        camera_vp: Mat4,
        eye: Vec3,
        lighting: &vpw_table::geometry::Lighting,
        head: bool,
        reflection: bool,
        budget: usize,
    ) -> bool {
        let format = post.format();
        let saved = lights.save_levels();
        let zeros = vec![0.0f32; saved.len()];
        let all_off = |queue: &wgpu::Queue| {
            for i in 0..saved.len() {
                lights.force_level(queue, i, 0.0);
            }
        };
        let restore = |queue: &wgpu::Queue| {
            for (i, l) in saved.iter().enumerate() {
                lights.force_level(queue, i, *l);
            }
        };
        let dark_gi = Gi {
            rows: Vec::new(),
            bounce: [0.0; 3],
            levels: [0.0; 4],
        };

        if matches!(self.state, State::Invalid) {
            self.plan(device, post, lights, camera_vp, format);
            let baked = self.baked.as_ref().expect("plan just built it");

            // The dark photograph, and the depth the live pass will restore.
            all_off(queue);
            pipeline.set_frame(
                queue,
                camera_vp,
                eye,
                lighting,
                baked.size,
                &dark_gi,
                scene.field,
            );
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vpw-flat-bake-base"),
            });
            self.photograph(
                &mut encoder,
                post,
                pipeline,
                scene,
                lights,
                &zeros,
                head,
                reflection,
            );
            // Into its keeping texture, then the geometry's depth.
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("vpw-flat-keep-base"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &baked.base,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&self.blit_pipeline);
                pass.set_bind_group(0, &baked.blit_bind, &[]);
                pass.draw(0..3, 0..1);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("vpw-flat-keep-depth"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &baked.depth,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // Cleared to the far plane: where no geometry
                            // lands, the live pieces must always win.
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &baked.scratch_depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_pipeline(&self.depth_only);
                pass.set_bind_group(0, &pipeline.frame_bind_group, &[]);
                scene.draw_filtered(&mut pass, |b| {
                    !b.transparent && !is_live(b) && (head || !b.backbox)
                });
            }
            queue.submit(Some(encoder.finish()));
            restore(queue);
            self.state = State::Baking { next: 0 };
            log::trace!(
                "flat bake: base photographed, {} lamps to go",
                self.baked.as_ref().map_or(0, |b| b.layers.len())
            );
        }

        let State::Baking { next } = self.state else {
            return self.ready();
        };
        let baked = self.baked.as_ref().expect("baking implies planned");
        let (w, h) = baked.size;
        let end = (next + budget.max(1)).min(baked.layers.len());
        for l in next..end {
            let layer = &baked.layers[l];
            let mut levels = zeros.clone();
            levels[layer.light] = lights.full_level(layer.light);

            all_off(queue);
            lights.force_level(queue, layer.light, levels[layer.light]);
            pipeline.set_frame(
                queue,
                camera_vp,
                eye,
                lighting,
                (w, h),
                &lights.gi_solo(layer.light),
                scene.field,
            );
            // Where on screen the photograph's rect lives, in uv, and how the
            // atlas slot maps back onto it.
            let r = layer.rect;
            let origin_uv = [(r[0] + 1.0) * 0.5, (1.0 - r[3]) * 0.5];
            let size_uv = [(r[2] - r[0]) * 0.5, (r[3] - r[1]) * 0.5];
            queue.write_buffer(
                &self.slot_uniform,
                0,
                bytemuck::bytes_of(&GpuSlot {
                    origin: [origin_uv[0], origin_uv[1], size_uv[0], size_uv[1]],
                    step: [
                        layer.slot[0] as f32,
                        layer.slot[1] as f32,
                        1.0 / layer.slot[2].max(1) as f32,
                        1.0 / layer.slot[3].max(1) as f32,
                    ],
                }),
            );

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vpw-flat-bake-lamp"),
            });
            self.photograph(
                &mut encoder,
                post,
                pipeline,
                scene,
                lights,
                &levels,
                head,
                reflection,
            );
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("vpw-flat-diff"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &baked.pages[layer.page as usize],
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // The first slot on a page clears it; after that
                            // the page holds its neighbours.
                            load: if baked.layers[..l].iter().all(|o| o.page != layer.page) {
                                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_viewport(
                    layer.slot[0] as f32,
                    layer.slot[1] as f32,
                    layer.slot[2] as f32,
                    layer.slot[3] as f32,
                    0.0,
                    1.0,
                );
                pass.set_scissor_rect(layer.slot[0], layer.slot[1], layer.slot[2], layer.slot[3]);
                pass.set_pipeline(&self.diff_pipeline);
                pass.set_bind_group(0, &baked.diff_bind, &[]);
                pass.draw(0..3, 0..1);
            }
            queue.submit(Some(encoder.finish()));
        }
        restore(queue);

        if end >= baked.layers.len() {
            self.state = State::Ready;
            log::trace!("flat bake: {} lamp sprites ready", baked.layers.len());
        } else {
            self.state = State::Baking { next: end };
        }
        self.ready()
    }

    /// The flat frame: the photograph, the lamps at their live levels, and
    /// everything that genuinely moves drawn live over it. Runs inside the
    /// caller's encoder, into the scene's HDR buffer — bloom and tone mapping
    /// pick it up from there exactly as they would a rendered frame.
    #[expect(
        clippy::too_many_arguments,
        reason = "the live pieces each bring their own state"
    )]
    pub fn draw(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        post: &Post,
        pipeline: &TablePipeline,
        scene: &GpuScene,
        dynamic: Option<&DynamicParts>,
        lights: &Lights,
        flashers: Option<&Flashers>,
        head: bool,
    ) {
        let Some(baked) = &self.baked else { return };

        // The lamps' levels this frame, as fractions of their bakes. Only the
        // lit ones are drawn.
        let mut instances: Vec<GpuLayer> = Vec::new();
        for layer in &baked.layers {
            let full = lights.full_level(layer.light);
            if full <= 0.0 {
                continue;
            }
            let scale = lights.level(layer.light) / full;
            if scale <= 0.001 {
                continue;
            }
            instances.push(GpuLayer {
                rect: layer.rect,
                uv: layer.uv,
                meta: [layer.page as f32, scale],
            });
        }
        if !instances.is_empty() {
            queue.write_buffer(&baked.instances, 0, bytemuck::cast_slice(&instances));
        }

        let (color, resolve) = post.scene_color();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("vpw-flat-frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: resolve,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(pipeline.clear),
                    store: if resolve.is_some() {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &post.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        // The photograph, colour and depth in one draw.
        pass.set_pipeline(&self.base_pipeline);
        pass.set_bind_group(0, &baked.live_bind, &[]);
        pass.draw(0..3, 0..1);

        // The lamps: one instanced draw for every lit light on the table.
        if !instances.is_empty() {
            pass.set_pipeline(&self.layer_pipeline);
            pass.set_vertex_buffer(0, baked.instances.slice(..));
            pass.draw(0..6, 0..instances.len() as u32);
        }

        // And the things that truly move, in real 3D against the
        // photograph's depth: the parts and balls, the score display, the
        // flashers. The frame uniform is live, so the ball still catches
        // the lit lamps' glints.
        pass.set_bind_group(0, &pipeline.frame_bind_group, &[]);
        if let Some(d) = dynamic.filter(|d| d.any(false)) {
            pass.set_pipeline(&pipeline.dynamic_opaque);
            d.draw(&mut pass, false);
        }
        if head && scene.batches.iter().any(|b| is_live(b) && !b.transparent) {
            pass.set_pipeline(&pipeline.opaque);
            scene.draw_filtered(&mut pass, |b| is_live(b) && !b.transparent);
        }
        if head && scene.batches.iter().any(|b| is_live(b) && b.transparent) {
            pass.set_pipeline(&pipeline.blended);
            scene.draw_filtered(&mut pass, |b| is_live(b) && b.transparent);
        }
        if let Some(d) = dynamic.filter(|d| d.any(true)) {
            pass.set_pipeline(&pipeline.dynamic_blended);
            d.draw(&mut pass, true);
        }
        if let Some(f) = flashers {
            f.draw(&mut pass, &pipeline.light_frame_bind_group);
        }
    }
}
