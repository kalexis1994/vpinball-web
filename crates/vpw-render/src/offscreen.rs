//! Rendering to a texture, with no window and no canvas.
//!
//! It exists so the renderer can be verified from the terminal: load a table,
//! draw it into memory and save a PNG. It is also what makes visual regression
//! tests possible, which in a renderer are worth more than any assert over data
//! structures.

use crate::camera::Camera;
use crate::dynamic::DynamicParts;
use crate::flashers::Flashers;
use crate::lights::Lights;
use crate::pipeline::TablePipeline;
use crate::post::Post;
use crate::scene::GpuScene;
use vpw_table::geometry::Scene;
use wgpu::util::DeviceExt;

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub struct Offscreen {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub pipeline: TablePipeline,
    pub lights: Lights,
    /// The strobes, and the display where a table places it with one.
    pub flashers: Flashers,
    /// The pieces that move. `None` draws only the baked geometry, which is
    /// what a plain photo of a table wants.
    pub dynamic: Option<DynamicParts>,
    width: u32,
    height: u32,
    color: wgpu::Texture,
    post: Post,
    /// Name of the adapter we ended up using.
    pub adapter: String,
    /// What the offscreen passes draw into here, and what the adapter said it
    /// could do with that format. Kept so a caller can check the choice against
    /// the device rather than against an assumption. See
    /// [`crate::post::hdr_format`].
    hdr: wgpu::TextureFormat,
    hdr_usages: wgpu::TextureUsages,
}

impl Offscreen {
    pub async fn new(width: u32, height: u32) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::all()),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("vpw-offscreen"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;

        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vpw-color"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // The same chain the browser runs, so a photograph taken from the
        // terminal is a photograph of what a player would see. The table goes
        // into the floating-point buffer and only the composite writes `color`.
        // Offscreen is a terminal tool on a real GPU: the good format is
        // always there, and asking the adapter costs nothing.
        let hdr = crate::post::hdr_format(&adapter);
        let hdr_usages = adapter.get_texture_format_features(hdr).allowed_usages;
        // The same four-or-one the browser decides, asked of the same flags,
        // so a photograph is a photograph of what a player would get.
        let flags = adapter.get_texture_format_features(hdr).flags;
        let samples = if flags.sample_count_supported(4) {
            4
        } else {
            1
        };
        // VPW_MSAA=1 or =4 overrides, which is how the benchmark takes a
        // matched pair on the same warmed-up device.
        let samples = std::env::var("VPW_MSAA")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|s| *s == 1 || (*s == 4 && flags.sample_count_supported(4)))
            .unwrap_or(samples);
        eprintln!("offscreen MSAA {samples}x");
        let post = Post::new(&device, &queue, hdr, FORMAT, samples, width, height);
        let mut pipeline = TablePipeline::new(&device, &queue, hdr, samples);
        pipeline.set_probes(
            &device,
            post.transmission_view(),
            post.sampler(),
            post.reflection_view(),
        );
        let lights = Lights::new(&device, &pipeline, hdr, samples);
        let flashers = Flashers::new(&device, &queue, &pipeline.light_frame_layout, hdr, samples);

        Ok(Self {
            device,
            queue,
            pipeline,
            lights,
            flashers,
            dynamic: None,
            width,
            height,
            color,
            post,
            adapter: format!("{} ({:?})", info.name, info.backend),
            hdr,
            hdr_usages,
        })
    }

    /// Redraws the machine's score display, the same way the browser does.
    ///
    /// So a photograph taken from the terminal is a photograph of what a player
    /// would see, display and all.
    pub fn set_display(&mut self, scene: &GpuScene, raster: &crate::segments::Raster) {
        let Some(tex) = scene.redrawn.get(vpw_table::backbox::DISPLAY_IMAGE) else {
            return;
        };
        if tex.width() != raster.width || tex.height() != raster.height {
            return;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &raster.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(raster.width * 4),
                rows_per_image: Some(raster.height),
            },
            wgpu::Extent3d {
                width: raster.width,
                height: raster.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// What the offscreen passes draw into on this device.
    pub fn hdr_format(&self) -> wgpu::TextureFormat {
        self.hdr
    }

    /// What the adapter said it could do with a format.
    pub fn adapter_format_usages(&self, format: wgpu::TextureFormat) -> wgpu::TextureUsages {
        debug_assert_eq!(format, self.hdr, "only the chosen format is remembered");
        self.hdr_usages
    }

    /// How strongly the bloom is added back. Zero turns it off, which is what
    /// makes it possible to photograph a scene twice and measure what the pass
    /// actually contributed.
    pub fn set_bloom(&mut self, strength: f32) {
        self.post.set_strength(&self.queue, strength);
    }

    /// Traces the GI groups' lightmaps and hands them to the frame.
    ///
    /// Returns how many groups were baked; zero means the table names no GI
    /// string — or brought its own bake, which is better than ours. See
    /// `crate::bake` for what and why.
    pub fn bake_gi(&mut self, scene: &Scene) -> usize {
        let groups = crate::bake::gi_groups(scene);
        if groups.is_empty() {
            return 0;
        }
        let bake = crate::bake::bake_gi_set(scene, &groups, crate::bake::INDIRECT_SAMPLES);
        self.apply_gi_bake(
            &bake,
            &groups.iter().map(|g| g.names.clone()).collect::<Vec<_>>(),
        );
        groups.len()
    }

    /// Installs an already-traced bake — this run's, or one a cache kept.
    pub fn apply_gi_bake(&mut self, bake: &crate::bake::GiBakeSet, groups: &[Vec<String>]) {
        let mut data: Vec<u16> = Vec::with_capacity(bake.layers.len() * bake.layers[0].len());
        for layer in &bake.layers {
            data.extend_from_slice(layer);
        }
        // A spare black layer when there is only one, as in the on-screen
        // renderer: wgpu-hal guesses the view dimension from the layer count,
        // and one layer reads as `D2` rather than the array the shader binds.
        let layers = (bake.layers.len() as u32).max(2);
        data.resize(layers as usize * bake.layers[0].len(), 0);
        let texture = &self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("vpw-gi-bake"),
                size: wgpu::Extent3d {
                    width: bake.width,
                    height: bake.height,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytemuck::cast_slice(&data),
        );
        self.pipeline.set_gi_lightmap(
            &self.device,
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
            bake.layers.len() as u32,
        );
        self.lights.set_baked_groups(&self.queue, groups);
    }

    /// Uploads the table's lit lights.
    /// Puts the table in a room: a Radiance `.hdr` equirectangular map
    /// replaces whatever environment the upload installed. The photograph
    /// harnesses use it to shoot a table under the same room the player can
    /// choose.
    pub fn set_environment_hdr(&mut self, bytes: &[u8]) -> bool {
        match crate::env::EnvMap::from_hdr(&self.device, &self.queue, bytes, "room") {
            Some(map) => {
                self.pipeline.set_envmap(&self.device, map);
                true
            }
            None => false,
        }
    }

    pub fn upload_lights(&mut self, scene: &Scene) {
        self.lights
            .upload(&self.device, &self.queue, &self.pipeline, scene);
        self.post.set_exposure(&self.queue, scene.lighting.exposure);
    }

    /// Uploads the table's flashers, in the state the file leaves them.
    pub fn upload_flashers(&mut self, scene: &Scene) {
        self.flashers.upload(&self.device, &self.queue, scene);
    }

    /// Puts a frame of dots on the flashers that show the display, the same
    /// way the browser does. See `TableRenderer::set_dmd`.
    pub fn set_dmd(&mut self, dots: &[u8], width: usize, height: usize) {
        self.flashers
            .set_dmd(&self.device, &self.queue, dots, width, height);
    }

    /// Uploads the moving pieces, so the photo shows the flippers where the
    /// physics has them instead of at their rest angle.
    pub fn upload_dynamic(
        &mut self,
        scene: &Scene,
        animated: &[vpw_table::animation::AnimatedPart],
    ) {
        self.dynamic = Some(DynamicParts::upload(
            &self.device,
            &self.queue,
            &self.pipeline,
            scene,
            animated,
            &vpw_table::ball::mesh(),
            &vpw_table::ball::material(),
        ));
    }

    /// Uploads the table, environment map included: the map is the table's
    /// (`Renderer.cpp:208`), and a photograph under the wrong one is a
    /// photograph of a different table.
    pub fn upload(&mut self, scene: &Scene) -> GpuScene {
        self.pipeline.set_envmap(
            &self.device,
            crate::env::EnvMap::for_table(&self.device, &self.queue, scene),
        );
        let gpu_scene = GpuScene::upload(
            &self.device,
            &self.queue,
            &self.pipeline.material_layout,
            scene,
        );
        // The floor's picture, for the ball's planar reflection.
        if let Some(view) = gpu_scene.field_picture.clone() {
            self.pipeline.set_field_picture(&self.device, view);
        }
        gpu_scene
    }

    /// Draws a frame and returns the RGBA of the image.
    pub fn render(&self, scene: &GpuScene, camera: &Camera) -> Vec<u8> {
        self.render_filtered(scene, camera, |_| true)
    }

    /// The same, but drawing only the batches that pass the filter.
    pub fn render_filtered(
        &self,
        scene: &GpuScene,
        camera: &Camera,
        filter: impl Fn(&crate::scene::Batch) -> bool,
    ) -> Vec<u8> {
        self.draw_only(scene, camera, filter);
        self.read_back()
    }

    /// Draws and submits a frame without reading it back.
    ///
    /// For measuring: the read-back costs more than the frame and would bury
    /// any rendering change under its constant. A benchmark submits many of
    /// these, waits for the queue, and divides.
    pub fn draw_only(
        &self,
        scene: &GpuScene,
        camera: &Camera,
        filter: impl Fn(&crate::scene::Batch) -> bool,
    ) {
        let aspect = self.width as f32 / self.height as f32;
        let gi = self.lights.gi_sources(crate::scene::MAX_GI_BULBS);
        self.pipeline.set_frame(
            &self.queue,
            camera.view_projection(aspect),
            camera.eye(),
            &scene.lighting,
            (self.width, self.height),
            &gi,
            scene.field,
        );

        let view = self
            .color
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vpw-offscreen"),
            });
        crate::pass::draw_lights_only(
            &mut encoder,
            self.post.transmission_view(),
            &self.pipeline,
            &self.lights,
        );
        self.post.blur_transmission(&mut encoder);
        // The probe is a whole extra pass over the scene, so it is not drawn
        // for a table that does not mirror. The original skips it the same way,
        // by never creating the probe (`RenderProbe::REFL_NONE`).
        if scene.lighting.reflection_strength > 0.0 {
            let (color, resolve) = self.post.reflection_color();
            crate::pass::draw_reflection(
                &mut encoder,
                color,
                resolve,
                self.post.reflection_depth(),
                &self.pipeline,
                scene,
                self.dynamic.as_ref(),
            );
        }
        let (color, resolve) = self.post.scene_color();
        crate::pass::draw_full(
            &mut encoder,
            color,
            resolve,
            &self.post.depth,
            &self.pipeline,
            scene,
            self.dynamic.as_ref(),
            Some(&self.lights),
            Some(&self.flashers),
            &filter,
        );
        self.post.finish(&mut encoder, &view);
        self.queue.submit(Some(encoder.finish()));
    }

    /// Copies the texture back to the CPU. wgpu's copy demands rows aligned to
    /// 256 bytes, so the padding has to be stripped by hand.
    fn read_back(&self) -> Vec<u8> {
        let unpadded = self.width * 4;
        let padded = unpadded.div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vpw-readback"),
            size: u64::from(padded) * u64::from(self.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            self.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());

        let data = slice
            .get_mapped_range()
            .expect("could not map the readback buffer");
        let mut output = Vec::with_capacity((unpadded * self.height) as usize);
        for row in 0..self.height {
            let from = (row * padded) as usize;
            output.extend_from_slice(&data[from..from + unpadded as usize]);
        }
        drop(data);
        buffer.unmap();
        output
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
