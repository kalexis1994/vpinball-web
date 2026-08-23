//! The complete renderer on top of a surface: what the browser uses.
//!
//! It brings together the WebGPU context, the pipeline, the depth texture and
//! the uploaded scene, and draws frames. It is the equivalent of the original's
//! `Renderer` (`src/renderer/Renderer.cpp`, 3,523 lines) cut down to what a
//! player needs.

use crate::camera::Camera;
use crate::dynamic::DynamicParts;
use crate::lights::Lights;
use crate::pipeline::TablePipeline;
use crate::post::Post;
use crate::scene::{GpuScene, SceneStats};
use crate::{FrameError, GpuContext, GpuInitError};
use vpw_math::Vec3;

pub struct TableRenderer {
    gpu: GpuContext,
    pipeline: TablePipeline,
    /// The buffers and passes between the table and the screen. It owns the
    /// depth buffer too, since both are sized to the window.
    post: Post,
    scene: Option<GpuScene>,
    /// The pieces that move. `None` until a table is loaded, and also for a
    /// table with nothing that moves.
    dynamic: Option<DynamicParts>,
    lights: Lights,
    pub camera: Camera,
    /// Where the player is looking from, and what that framing was worked out
    /// against. Kept so the view survives a resize and a table reload.
    view: crate::camera::View,
    /// A display drawing that arrived before there was a texture the right
    /// size for it. Applied when the scene is next built.
    pending_display: Option<crate::segments::Raster>,
    framing: Option<(vpw_table::geometry::Bounds, vpw_table::backbox::Backbox)>,
}

impl TableRenderer {
    pub async fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, GpuInitError> {
        let gpu = GpuContext::new(target, width, height).await?;
        // What the passes draw into, decided once from what this device can
        // actually draw into. See [`crate::post::hdr_format`].
        let hdr = gpu.hdr_format;
        let post = Post::new(&gpu.device, &gpu.queue, hdr, gpu.format(), width, height);
        // The table is drawn into the floating-point buffer, not onto the
        // screen, so both it and the lights are built for that format. Only the
        // final composite targets the surface.
        let mut pipeline = TablePipeline::new(&gpu.device, &gpu.queue, hdr);
        pipeline.set_probes(
            &gpu.device,
            post.transmission_view(),
            post.sampler(),
            post.reflection_view(),
        );
        let lights = Lights::new(&gpu.device, &pipeline.light_frame_layout, hdr);
        Ok(Self {
            gpu,
            pipeline,
            post,
            scene: None,
            dynamic: None,
            lights,
            camera: Camera::default(),
            view: crate::camera::View::Front,
            pending_display: None,
            framing: None,
        })
    }

    /// Redraws the machine's score display onto its head.
    ///
    /// The image comes from [`crate::segments`], which knows how a digit is
    /// shaped and nothing else — the same drawing the page puts on a floating
    /// canvas when the head is not in shot. One drawing, two destinations, so
    /// the two can never disagree about what the machine is saying.
    ///
    /// The texture is recreated when the size changes and written in place when
    /// it does not, which is almost always: the number of digits is a fact
    /// about the machine and does not move.
    pub fn set_display(&mut self, raster: &crate::segments::Raster) {
        let Some(scene) = &mut self.scene else { return };
        let Some(tex) = scene.redrawn.get(vpw_table::backbox::DISPLAY_IMAGE) else {
            return;
        };
        if tex.width() != raster.width || tex.height() != raster.height {
            // The first real drawing arrives after a one-pixel placeholder, and
            // a machine with a different display would be a different size
            // again. Rebuilding means a new bind group, which the scene owns,
            // so this is left to the upload path rather than done behind it.
            self.pending_display = Some(raster.clone());
            return;
        }
        self.gpu.queue.write_texture(
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

    /// Where the machine's head lands on screen, as `[left, top, width, height]`
    /// in fractions of the canvas.
    ///
    /// So the score can be drawn on the head instead of at a guessed offset
    /// from the top of the window. The guess was wrong the moment the window
    /// changed shape, and it has no way of being right across two views that
    /// frame completely different things.
    ///
    /// `None` when the head is not in shot — behind the camera, or simply not
    /// part of this view — which is the signal to put the score somewhere else
    /// rather than to hide it.
    pub fn backbox_screen_rect(&self) -> Option<[f32; 4]> {
        if !self.view.shows_backbox() {
            return None;
        }
        let (_, head) = self.framing?;
        let (w, h) = self.gpu.size();
        let vp = self.camera.view_projection(w as f32 / h as f32);

        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for corner in head.corners() {
            let clip = vp * corner.extend(1.0);
            // Behind the eye: there is no honest screen position for it, and
            // dividing by a negative `w` gives a confident wrong one.
            if clip.w <= 0.0 {
                return None;
            }
            // Clip space is -1..1 with `y` up; the page counts from the top.
            let x = (clip.x / clip.w + 1.0) * 0.5;
            let y = (1.0 - clip.y / clip.w) * 0.5;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        Some([min_x, min_y, max_x - min_x, max_y - min_y])
    }

    /// Where the player is looking from.
    pub fn view(&self) -> crate::camera::View {
        self.view
    }

    /// Moves to one of the named views, reframing on whatever is loaded.
    ///
    /// Cheap and idempotent: the framing is a short search over a box the
    /// renderer already has, so a caller can set the view it wants every frame
    /// without having to remember which one is current.
    pub fn set_view(&mut self, view: crate::camera::View) {
        if self.view == view {
            return;
        }
        self.view = view;
        self.reframe();
    }

    /// Puts the camera back where the current view says it goes.
    ///
    /// Called on a new table and on a resize as well as on a change of view:
    /// the framing depends on the aspect ratio, so a window that changes shape
    /// changes what fits.
    fn reframe(&mut self) {
        let Some((table, head)) = self.framing else {
            return;
        };
        let head = head.bounds();
        let (w, h) = self.gpu.size();
        self.camera = Camera::for_view(
            self.view,
            (table.min, table.max),
            (head.min, head.max),
            w as f32 / h as f32,
        );
    }

    /// Uploads a table and frames the camera on it.
    ///
    /// The framing uses the extent of the **playfield**, not that of everything
    /// in the file: plenty of tables carry stray parts far away —a backglass, a
    /// DMD panel— and framing on all of it leaves the table tiny in a corner.
    pub fn load(&mut self, scene: &vpw_table::geometry::Scene) {
        self.load_with_parts(scene, &[]);
    }

    /// The same, plus the table's moving pieces.
    ///
    /// The caller has to have taken those pieces **out** of `scene` already
    /// (`Scene::remove`), or each one gets drawn twice: once baked at its rest
    /// position and once following the physics. A ghost flipper frozen at its
    /// start angle is one of the more baffling things a player can show.
    pub fn load_with_parts(
        &mut self,
        scene: &vpw_table::geometry::Scene,
        animated: &[vpw_table::animation::AnimatedPart],
    ) {
        let gpu_scene = GpuScene::upload(
            &self.gpu.device,
            &self.gpu.queue,
            &self.pipeline.material_layout,
            scene,
        );

        let pf = scene.playfield;
        // A bit of height so that the toys and the ramps fit in. The playfield
        // is a flat rectangle in the file and a table is not: the ramps and the
        // toys stand on it, and a camera that frames the sheet cuts their tops
        // off.
        let height = (pf.max.x - pf.min.x) * 0.45;
        self.framing = Some((
            vpw_table::geometry::Bounds {
                min: Vec3::new(pf.min.x, pf.min.y, 0.0),
                max: Vec3::new(pf.max.x, pf.max.y, height),
            },
            vpw_table::backbox::Backbox::for_playfield(pf),
        ));
        self.reframe();
        self.lights.upload(&self.gpu.device, &scene.lights);
        self.dynamic = Some(DynamicParts::upload(
            &self.gpu.device,
            &self.gpu.queue,
            &self.pipeline,
            scene,
            animated,
            &vpw_table::ball::mesh(),
            &vpw_table::ball::material(),
        ));
        self.scene = Some(gpu_scene);
    }

    pub fn unload(&mut self) {
        self.scene = None;
        self.dynamic = None;
        self.lights.lights.clear();
    }

    /// The moving pieces, to move them before drawing.
    /// The lamps, to turn on and off as the game does.
    pub fn lights_mut(&mut self) -> &mut crate::lights::Lights {
        &mut self.lights
    }

    pub fn dynamic_mut(&mut self) -> Option<&mut DynamicParts> {
        self.dynamic.as_mut()
    }

    /// The queue, which is what writing a matrix needs.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.gpu.queue
    }

    /// How many lit lights the loaded table has.
    pub fn lit_lights(&self) -> usize {
        self.lights.len()
    }

    pub fn stats(&self) -> Option<SceneStats> {
        self.scene.as_ref().map(|s| s.stats)
    }

    pub fn size(&self) -> (u32, u32) {
        self.gpu.size()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 || self.gpu.size() == (width, height) {
            return;
        }
        self.gpu.resize(width, height);
        self.post
            .resize(&self.gpu.device, &self.gpu.queue, width, height);
        self.pipeline.set_probes(
            &self.gpu.device,
            self.post.transmission_view(),
            self.post.sampler(),
            self.post.reflection_view(),
        );
        // A window that changes shape changes what fits, so the view has to be
        // framed again: a table that filled a wide screen leaves the head off
        // the top of a narrow one.
        self.reframe();
    }

    /// Draws a frame. With no table loaded it only clears, which is still
    /// enough to tell that the chain down to WebGPU is alive.
    pub fn render(&mut self) -> Result<(), FrameError> {
        let (w, h) = self.gpu.size();
        let aspect = w as f32 / h as f32;

        let Some(scene) = &self.scene else {
            return self.gpu.render();
        };

        self.pipeline.set_frame(
            &self.gpu.queue,
            self.camera.view_projection(aspect),
            self.camera.eye(),
            &scene.lighting,
            (w, h),
        );
        self.post
            .set_exposure(&self.gpu.queue, scene.lighting.exposure);

        let frame = self.gpu.acquire()?;
        let Some(frame) = frame else { return Ok(()) };

        // Through the sRGB view, not the surface's own format: the shader
        // works in linear light and the hardware does the encode on the way
        // out. See `Gpu::format`.
        let view = self.gpu.frame_view(&frame);
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vpw-frame"),
            });
        // The order matters and is the original's: the lights on their own
        // first, because the table reads that buffer; then the table; then the
        // bloom, which reads the table; then all of it to the screen.
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
            crate::pass::draw_reflection(
                &mut encoder,
                self.post.reflection_view(),
                self.post.reflection_depth(),
                &self.pipeline,
                scene,
                self.dynamic.as_ref(),
            );
        }
        crate::pass::draw(
            &mut encoder,
            self.post.scene_view(),
            &self.post.depth,
            &self.pipeline,
            scene,
            self.dynamic.as_ref(),
            Some(&self.lights),
        );
        self.post.finish(&mut encoder, &view);
        self.gpu.queue.submit(Some(encoder.finish()));
        self.gpu.present(frame);
        Ok(())
    }
}
