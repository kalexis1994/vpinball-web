//! The buffers and passes that run after the table is drawn.
//!
//! The table no longer goes straight to the screen. It goes into a
//! floating-point buffer, and between that buffer and the screen sit the two
//! passes that make a playfield look lit rather than merely coloured:
//!
//! - **Transmitted light.** Before anything else is drawn, every light on the
//!   table is rendered on its own into a small buffer and blurred. The material
//!   shader then samples it under any translucent surface, which is the glow you
//!   see coming *up through* an insert's plastic and spilling onto the ones
//!   beside it. `Renderer::DrawBulbLightBuffer`, `Renderer.cpp:1484`.
//!
//! - **Bloom.** After the table is drawn, everything brighter than the
//!   threshold is cut out of the picture, blurred very wide, and added back.
//!   `Renderer::UpdateBloom`, `Renderer.cpp:2043`.
//!
//! Both blur into buffers a quarter the size in each direction, which is what
//! the original uses: a blur this wide has nothing in it that a quarter
//! resolution cannot hold, and it costs a sixteenth as much.
//!
//! The original runs both through the *same* buffer, one after the other, and
//! leaves a comment where it has to remember that (`Renderer.cpp:1978`). We keep
//! them apart. At a quarter resolution the second buffer is about a megabyte,
//! which is a cheap price for not having to think about it.

use crate::pipeline::DEPTH_FORMAT;

/// What the table is drawn into, when the device will draw into it.
///
/// Sixteen-bit float per channel, and the whole point is that it has no
/// ceiling: a lit insert comes out of the material shader at four or ten, and
/// clipping it at one — which is all an eight-bit buffer can do — is exactly
/// what the bloom exists to catch.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// What to draw into where the good one is not available.
///
/// Everything still works and the picture is still right side up; what is lost
/// is the headroom. A highlight that wanted to come out at four arrives at one,
/// so the bloom has less to find and a bright insert flattens instead of
/// blooming. Better than a black canvas, which is the alternative.
const FALLBACK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Picks the format the passes will use on this device.
///
/// On WebGPU the half-float target is guaranteed. On WebGL2 it is an extension
/// —`EXT_color_buffer_half_float`— that most desktops have and some older
/// phones do not, and asking for it where it is missing does not degrade: the
/// texture simply fails to create and nothing is ever drawn.
pub fn hdr_format(adapter: &wgpu::Adapter) -> wgpu::TextureFormat {
    let can_draw_into = adapter
        .get_texture_format_features(HDR_FORMAT)
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT);
    if can_draw_into {
        HDR_FORMAT
    } else {
        log::warn!(
            "this device cannot draw into {HDR_FORMAT:?}; falling back to {FALLBACK_FORMAT:?},              so bright highlights will clip instead of blooming"
        );
        FALLBACK_FORMAT
    }
}

/// How much smaller the blur buffers are, in each direction.
const DOWNSCALE: u32 = 4;

/// How much smaller the reflection probe is. See [`Post::targets`].
const REFLECTION_DOWNSCALE: u32 = 2;

/// A texture that is only ever drawn into and sampled from.
struct Target {
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl Target {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        label: &str,
        width: u32,
        height: u32,
    ) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Target {
            view: texture.create_view(&Default::default()),
            width: width.max(1),
            height: height.max(1),
        }
    }
}

/// The uniform every post pass reads.
///
/// `texel` and `params` together are the original's `w_h_height` and
/// `bloom_dither_colorgrade`, which it packs the same way — one over the source
/// size, then the bloom strength (`Renderer.cpp:2071`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    texel: [f32; 4],
    params: [f32; 4],
}

/// Which uniform a pass reads: one describing the full-resolution buffer, one
/// describing the small ones.
#[derive(Clone, Copy)]
enum Scale {
    Full,
    Small,
}

pub struct Post {
    /// The table, at full resolution and with no ceiling.
    hdr: Target,
    pub depth: wgpu::TextureView,
    /// Where the bloom is built.
    bloom: Target,
    /// Where a separable blur puts its first half.
    scratch: Target,
    /// The lights on their own, blurred, for translucent surfaces to sample.
    transmission: Target,
    /// The table seen from under the playfield: what the playfield mirrors.
    reflection: Target,
    /// Its own depth, since it is drawn before the table and at its own size.
    reflection_depth: wgpu::TextureView,

    cut_off: wgpu::RenderPipeline,
    wide_h: wgpu::RenderPipeline,
    wide_v: wgpu::RenderPipeline,
    narrow_h: wgpu::RenderPipeline,
    narrow_v: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,

    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// One per [`Scale`], written when the size changes.
    uniforms: [wgpu::Buffer; 2],
    /// What the passes draw into on this device. See [`hdr_format`].
    format: wgpu::TextureFormat,

    /// Bind groups for the combinations the passes actually use, built once per
    /// resize rather than once per frame.
    from_hdr: Option<wgpu::BindGroup>,
    from_bloom: Option<wgpu::BindGroup>,
    from_scratch: Option<wgpu::BindGroup>,
    from_transmission: Option<wgpu::BindGroup>,
    hdr_with_bloom: Option<wgpu::BindGroup>,

    /// What the material shader binds to reach [`Post::transmission`].
    transmission_binding: Option<wgpu::BindGroup>,

    strength: f32,
    exposure: f32,
}

impl Post {
    /// How strongly the bloom is added back.
    ///
    /// What the original defaults `m_bloom_strength` to. Only a default: the
    /// table's own value is read and used (`vpw_table::geometry::Lighting`),
    /// and it is not the rarity this comment used to claim — the one table this
    /// was first noticed on asks for 0.3, a sixth of it.
    const DEFAULT_STRENGTH: f32 = 1.8;

    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        output_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Post {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vpw-post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()),
        });

        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
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
            label: Some("vpw-post-layout"),
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
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Only the composite reads a second texture. The passes that do
                // not bind their own source here again, so there is one layout.
                texture_entry(3),
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vpw-post-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let make = |name: &str, entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vpw-post-sampler"),
            // Clamped, because every one of these passes reads outside its own
            // edge: a blur this wide reaches nineteen texels past the border,
            // and wrapping would fetch the far side of the playfield.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniforms = std::array::from_fn(|i| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(if i == 0 {
                    "vpw-post-uniform-full"
                } else {
                    "vpw-post-uniform-small"
                }),
                size: std::mem::size_of::<Uniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });

        let mut post = Post {
            // Filled in by `resize` below, which is the one place that builds
            // targets and the bind groups that point at them.
            hdr: Target::new(device, format, "vpw-hdr", 1, 1),
            depth: crate::pipeline::depth_texture(device, 1, 1),
            bloom: Target::new(device, format, "vpw-bloom", 1, 1),
            scratch: Target::new(device, format, "vpw-blur-scratch", 1, 1),
            transmission: Target::new(device, format, "vpw-transmission", 1, 1),
            reflection: Target::new(device, format, "vpw-reflection", 1, 1),
            reflection_depth: crate::pipeline::depth_texture(device, 1, 1),
            from_hdr: None,
            from_bloom: None,
            from_scratch: None,
            from_transmission: None,
            hdr_with_bloom: None,
            transmission_binding: None,
            cut_off: make("vpw-post-cutoff", "cut_off", format),
            wide_h: make("vpw-post-wide-h", "wide_h", format),
            wide_v: make("vpw-post-wide-v", "wide_v", format),
            narrow_h: make("vpw-post-narrow-h", "narrow_h", format),
            narrow_v: make("vpw-post-narrow-v", "narrow_v", format),
            composite: make("vpw-post-composite", "composite", output_format),
            layout,
            sampler,
            uniforms,
            format,
            strength: Self::DEFAULT_STRENGTH,
            exposure: 1.0,
        };
        post.resize(device, queue, width, height);
        post
    }

    fn targets(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (
        Target,
        Target,
        Target,
        Target,
        wgpu::TextureView,
        Target,
        wgpu::TextureView,
    ) {
        let (sw, sh) = ((width / DOWNSCALE).max(1), (height / DOWNSCALE).max(1));
        // The reflection is drawn at half size in each direction. The original
        // scales it by the reflecting surface's roughness
        // (`GetRoughnessDownscale`, `RenderProbe.cpp:71`); half is what a
        // playfield's roughness lands on, and it is a whole scene pass, so the
        // quarter of the work is the difference between affording it and not.
        let (rw, rh) = (
            (width / REFLECTION_DOWNSCALE).max(1),
            (height / REFLECTION_DOWNSCALE).max(1),
        );
        (
            Target::new(device, format, "vpw-hdr", width, height),
            Target::new(device, format, "vpw-bloom", sw, sh),
            Target::new(device, format, "vpw-blur-scratch", sw, sh),
            Target::new(device, format, "vpw-transmission", sw, sh),
            crate::pipeline::depth_texture(device, width, height),
            Target::new(device, format, "vpw-reflection", rw, rh),
            crate::pipeline::depth_texture(device, rw, rh),
        )
    }

    fn bind(
        &self,
        device: &wgpu::Device,
        scale: Scale,
        source: &Target,
        overlay: &Target,
    ) -> Option<wgpu::BindGroup> {
        let (layout, uniforms, sampler) = (&self.layout, &self.uniforms, &self.sampler);
        Some(
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vpw-post-bind"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniforms[match scale {
                            Scale::Full => 0,
                            Scale::Small => 1,
                        }]
                        .as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&source.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&overlay.view),
                    },
                ],
            }),
        )
    }

    /// The buffer the table is drawn into.
    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.hdr.view
    }

    /// The sampler the transmitted-light buffer is read with: clamped, so a
    /// fragment near the edge of the screen does not fetch the far side.
    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// The buffer the lights are drawn into on their own.
    pub fn transmission_view(&self) -> &wgpu::TextureView {
        &self.transmission.view
    }

    /// The buffer the table is drawn into upside down.
    pub fn reflection_view(&self) -> &wgpu::TextureView {
        &self.reflection.view
    }

    pub fn reflection_depth(&self) -> &wgpu::TextureView {
        &self.reflection_depth
    }

    /// How bright the picture is overall, from the table's own settings.
    pub fn set_exposure(&mut self, queue: &wgpu::Queue, exposure: f32) {
        if self.exposure != exposure {
            self.exposure = exposure;
            self.write_uniforms(queue);
        }
    }

    /// How strongly the bloom is added back, where zero turns it off.
    ///
    /// The original reads this per table (`m_bloom_strength`) and also lets a
    /// player force it off entirely (`ForceBloomOff`, `Renderer.cpp:64`).
    pub fn set_strength(&mut self, queue: &wgpu::Queue, strength: f32) {
        if self.strength != strength {
            self.strength = strength;
            self.write_uniforms(queue);
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
        let (hdr, bloom, scratch, transmission, depth, reflection, reflection_depth) =
            Self::targets(device, self.format, width, height);
        (
            self.hdr,
            self.bloom,
            self.scratch,
            self.transmission,
            self.depth,
            self.reflection,
            self.reflection_depth,
        ) = (
            hdr,
            bloom,
            scratch,
            transmission,
            depth,
            reflection,
            reflection_depth,
        );
        self.write_uniforms(queue);

        let (hdr, bloom) = (&self.hdr, &self.bloom);
        let (scratch, transmission) = (&self.scratch, &self.transmission);
        self.from_hdr = self.bind(device, Scale::Full, hdr, hdr);
        self.from_bloom = self.bind(device, Scale::Small, bloom, bloom);
        self.from_scratch = self.bind(device, Scale::Small, scratch, scratch);
        self.from_transmission = self.bind(device, Scale::Small, transmission, transmission);
        self.hdr_with_bloom = self.bind(device, Scale::Full, hdr, bloom);
        self.transmission_binding = self.bind(device, Scale::Small, transmission, transmission);
    }

    fn write_uniforms(&self, queue: &wgpu::Queue) {
        // The bright pass reads the full-resolution buffer, so its four taps are
        // one full-resolution texel apart even though it is writing into a
        // small one. `Renderer.cpp:2071` passes `1/w, 1/h` of the *rendered*
        // target for exactly this reason.
        let full = Uniform {
            texel: [
                1.0 / self.hdr.width as f32,
                1.0 / self.hdr.height as f32,
                0.0,
                0.0,
            ],
            params: [self.strength, self.exposure, 1.0, 0.0],
        };
        let small = Uniform {
            texel: [
                1.0 / self.bloom.width as f32,
                1.0 / self.bloom.height as f32,
                0.0,
                0.0,
            ],
            params: [self.strength, self.exposure, 1.0, 0.0],
        };
        queue.write_buffer(&self.uniforms[0], 0, bytemuck::bytes_of(&full));
        queue.write_buffer(&self.uniforms[1], 0, bytemuck::bytes_of(&small));
    }

    /// Runs one post pass: a full-screen triangle from `bind` into `target`.
    fn quad(
        encoder: &mut wgpu::CommandEncoder,
        label: &str,
        target: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        bind: &wgpu::BindGroup,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Blurs the transmitted-light buffer in place, through the scratch buffer.
    ///
    /// Nineteen wide, which is the kernel the original asks for here
    /// (`Renderer.cpp:1510`) — narrower than the bloom's, because this is light
    /// spreading a few millimetres through a piece of plastic and not a
    /// highlight blooming across the glass.
    pub fn blur_transmission(&self, encoder: &mut wgpu::CommandEncoder) {
        Self::quad(
            encoder,
            "vpw-transmission-h",
            &self.scratch.view,
            &self.narrow_h,
            self.from_transmission.as_ref().expect("built on resize"),
        );
        Self::quad(
            encoder,
            "vpw-transmission-v",
            &self.transmission.view,
            &self.narrow_v,
            self.from_scratch.as_ref().expect("built on resize"),
        );
    }

    /// Cuts the bright part out of the drawn table and blurs it very wide.
    /// `Renderer::UpdateBloom`, `Renderer.cpp:2043`.
    pub fn build_bloom(&self, encoder: &mut wgpu::CommandEncoder) {
        Self::quad(
            encoder,
            "vpw-bloom-cutoff",
            &self.bloom.view,
            &self.cut_off,
            self.from_hdr.as_ref().expect("built on resize"),
        );
        Self::quad(
            encoder,
            "vpw-bloom-h",
            &self.scratch.view,
            &self.wide_h,
            self.from_bloom.as_ref().expect("built on resize"),
        );
        Self::quad(
            encoder,
            "vpw-bloom-v",
            &self.bloom.view,
            &self.wide_v,
            self.from_scratch.as_ref().expect("built on resize"),
        );
    }

    /// Scene plus bloom, tone mapped, onto the screen.
    pub fn present(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        Self::quad(
            encoder,
            "vpw-composite",
            target,
            &self.composite,
            self.hdr_with_bloom.as_ref().expect("built on resize"),
        );
    }

    /// Everything after the table pass, in order.
    ///
    /// Both passes run every frame whatever is on the table. Bloom is not only
    /// for lamps — the specular off the ball overshoots too — and a table with
    /// nothing lit costs three passes over a buffer a sixteenth of the screen.
    pub fn finish(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        self.build_bloom(encoder);
        self.present(encoder, target);
    }

    /// What the material shader binds to reach the transmitted-light buffer.
    pub fn transmission_binding(&self) -> &wgpu::BindGroup {
        self.transmission_binding
            .as_ref()
            .expect("resize runs in the constructor")
    }

    /// The depth format the table pass uses, so callers do not have to reach
    /// into two modules for one frame.
    pub const DEPTH: wgpu::TextureFormat = DEPTH_FORMAT;
}
