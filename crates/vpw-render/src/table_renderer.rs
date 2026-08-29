//! The complete renderer on top of a surface: what the browser uses.
//!
//! It brings together the WebGPU context, the pipeline, the depth texture and
//! the uploaded scene, and draws frames. It is the equivalent of the original's
//! `Renderer` (`src/renderer/Renderer.cpp`, 3,523 lines) cut down to what a
//! player needs.

use crate::camera::Camera;
use crate::dynamic::DynamicParts;
use crate::flashers::Flashers;
use crate::lights::Lights;
use crate::pipeline::TablePipeline;
use crate::post::Post;
use crate::scene::{GpuScene, SceneStats};
use crate::{FrameError, GpuContext, GpuInitError};
use vpw_math::Vec3;
use wgpu::util::DeviceExt;

pub struct TableRenderer {
    gpu: GpuContext,
    pipeline: TablePipeline,
    /// The buffers and passes between the table and the screen. It owns the
    /// depth buffer too, since both are sized to the window.
    post: Post,
    scene: Option<GpuScene>,
    /// The environment the table asked for, kept aside while a room of the
    /// player's choosing stands in for it. See [`Self::set_environment`].
    table_env: Option<crate::env::EnvMap>,
    /// The pieces that move. `None` until a table is loaded, and also for a
    /// table with nothing that moves.
    dynamic: Option<DynamicParts>,
    lights: Lights,
    /// The strobes and flash domes, and on a 10.8 table the display. Empty
    /// until a table is loaded, and for a table without any.
    flashers: Flashers,
    pub camera: Camera,
    /// Where the player is looking from, and what that framing was worked out
    /// against. Kept so the view survives a resize and a table reload.
    view: crate::camera::View,
    /// A display drawing that arrived before there was a texture the right
    /// size for it. Applied when the scene is next built.
    pending_display: Option<crate::segments::Raster>,
    framing: Option<(vpw_table::geometry::Bounds, vpw_table::backbox::Backbox)>,
    /// Whether the head in `framing` is one we built and therefore one the
    /// front view has to keep in shot. A table that models its own is framed
    /// on its own parts, the way its author framed it.
    built_head: bool,
    /// The camera the table's author set up, for the views that have one.
    authored: Option<vpw_table::geometry::AuthoredView>,
    /// What the original's own camera is fitted to. See
    /// [`vpw_table::geometry::Scene::legacy_bounds`].
    legacy: Vec<Vec3>,
    /// And the one for a cabinet. See [`vpw_table::geometry::Scene::cabinet`].
    cabinet: Option<vpw_table::geometry::AuthoredView>,
    /// Corners of what really stands on the playfield. See `Scene::occupied`.
    occupied: Vec<Vec3>,
    /// The player's day/night, when they have set one; the table's own
    /// otherwise. The original's `SceneLighting` in `Mode::User`
    /// (`Renderer.cpp:377-398`): the player's level *replaces* the table's
    /// `m_globalEmissionScale`. The knob exists because plenty of tables are
    /// authored dark on purpose — F-14 asks for 0.08 — and how dark a room a
    /// player wants to sit in is the player's business.
    day_night: Option<f32>,
    /// Whether the playfield reflection probe runs. See
    /// [`Self::set_reflection_enabled`].
    reflection_enabled: bool,
    /// The flat engine, when the player asked for it. Its bake follows the
    /// camera and the lighting; anything that moves either invalidates it.
    /// See [`crate::flat`].
    flat: Option<crate::flat::Flat>,
    flat_on: bool,
    /// The fraction of the surface the scene is drawn at, as last set. See
    /// [`Self::set_render_scale`] for why this is remembered.
    render_scale: f32,
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
        // Four samples where the device offers them for the HDR format, one
        // where it does not; the whole scene pass follows the answer.
        let samples = gpu.msaa_samples(hdr);
        log::trace!("MSAA {samples}x");
        let post = Post::new(
            &gpu.device,
            &gpu.queue,
            hdr,
            gpu.format(),
            samples,
            width,
            height,
        );
        // The table is drawn into the floating-point buffer, not onto the
        // screen, so both it and the lights are built for that format. Only the
        // final composite targets the surface.
        let mut pipeline = TablePipeline::new(&gpu.device, &gpu.queue, hdr, samples);
        pipeline.set_probes(
            &gpu.device,
            post.transmission_view(),
            post.sampler(),
            post.reflection_view(),
        );
        let lights = Lights::new(&gpu.device, &pipeline, hdr, samples);
        let flashers = Flashers::new(
            &gpu.device,
            &gpu.queue,
            &pipeline.light_frame_layout,
            hdr,
            samples,
        );
        Ok(Self {
            gpu,
            pipeline,
            post,
            scene: None,
            table_env: None,
            dynamic: None,
            lights,
            flashers,
            camera: Camera::default(),
            occupied: Vec::new(),
            day_night: None,
            reflection_enabled: true,
            view: crate::camera::View::Front,
            pending_display: None,
            framing: None,
            built_head: true,
            authored: None,
            legacy: Vec::new(),
            cabinet: None,
            flat: None,
            flat_on: false,
            render_scale: 1.0,
        })
    }

    /// Switches the flat engine on or off. On, the table is photographed a
    /// few lamps per frame while the real renderer keeps playing, and the
    /// frame switches to the photographs the moment the last lamp is done.
    /// See [`crate::flat`] for what that trades away.
    pub fn set_flat(&mut self, on: bool) {
        if self.flat_on == on {
            return;
        }
        self.flat_on = on;
        if on && self.flat.is_none() {
            self.flat = Some(crate::flat::Flat::new(
                &self.gpu.device,
                &self.pipeline,
                self.gpu.hdr_format,
                self.gpu.msaa_samples(self.gpu.hdr_format),
            ));
        }
        if let Some(flat) = &mut self.flat {
            flat.invalidate();
        }
    }

    /// Whether the flat engine is asked for at all — baking or drawing.
    /// The camera answers to this: a photograph has no camera to move.
    pub fn flat_enabled(&self) -> bool {
        self.flat_on
    }

    /// Whether the flat engine is drawing the frames right now — on, and
    /// with its bake complete.
    pub fn flat_active(&self) -> bool {
        self.flat_on && self.flat.as_ref().is_some_and(crate::flat::Flat::ready)
    }

    /// The world changed under the photographs; they have to be retaken.
    fn invalidate_flat(&mut self) {
        if let Some(flat) = &mut self.flat {
            flat.invalidate();
        }
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

    /// Where the playfield lands on screen, as `[left, top, width, height]`
    /// in fractions of the canvas — the companion of
    /// [`Self::backbox_screen_rect`], for whoever wants to lay furniture in
    /// the space *beside* the table: in the overhead view a wide window
    /// leaves gutters either side, and that is where the score panel fits
    /// without covering a single flipper.
    pub fn playfield_screen_rect(&self) -> Option<[f32; 4]> {
        let (table, _) = self.framing?;
        let (w, h) = self.gpu.size();
        let vp = self.camera.view_projection(w as f32 / h as f32);
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for i in 0..4 {
            let corner = Vec3::new(
                if i & 1 == 0 { table.min.x } else { table.max.x },
                if i & 2 == 0 { table.min.y } else { table.max.y },
                0.0,
            );
            let clip = vp * corner.extend(1.0);
            if clip.w <= 0.0 {
                return None;
            }
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
        // The flat world is a photograph taken from one place; there is no
        // other camera to move to, so the switch waits until the mode is off.
        if self.flat_on {
            log::trace!("the flat engine holds the camera; view change ignored");
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
        // A camera that moved makes every photograph a lie.
        self.invalidate_flat();
        if let Some(camera) = self.camera_for(self.view) {
            self.camera = camera;
        }
    }

    /// Where a given view's camera goes, without moving to it.
    ///
    /// Separate from [`Self::reframe`] because one caller wants the camera and
    /// not the consequences: a photograph for the library looks at the machine
    /// from the front for a single frame, and it must not disturb the view the
    /// player is using or throw away a flat bake that took seconds to make.
    fn camera_for(&self, view: crate::camera::View) -> Option<Camera> {
        let (table, head) = self.framing?;
        let head = head.bounds();
        let (w, h) = self.gpu.size();
        // A head we did not build is not ours to frame. It is one of the
        // table's own parts, already in `occupied` at its real extent, and
        // handing it over again as a box would push the machine away to make
        // room for a claim about its corners that is not true. Giving the
        // playfield's own box in its place adds nothing, which is right.
        let head = if self.built_head {
            (head.min, head.max)
        } else {
            (table.min, table.max)
        };
        // The front view is framed the original's way, on the parts the
        // original frames on. The overhead view is ours, and keeps the finer
        // set: looking straight down, where the tall things *actually* are is
        // worth a couple of per cent of a phone screen.
        let corners = match view {
            crate::camera::View::Front | crate::camera::View::Cabinet => &self.legacy,
            crate::camera::View::Overhead => &self.occupied,
        };
        let mut camera = Camera::for_authored_view(
            view,
            (table.min, table.max),
            head,
            w as f32 / h as f32,
            corners,
            match view {
                crate::camera::View::Cabinet => self.cabinet,
                _ => self.authored,
            },
        );
        // On a table that models its own room, the front view starts at the
        // table: the room's lid is between the eye and the playfield from
        // every angle this view can take. Tables that brought no scenery of
        // their own have nothing up there and nothing changes for them.
        if matches!(view, crate::camera::View::Front) && !self.built_head {
            camera.start_at(&self.legacy);
        }
        Some(camera)
    }

    /// Draws one frame as a photograph of the whole machine, then puts
    /// everything back.
    ///
    /// For the library's card. From the front, because a pinball machine seen
    /// from straight above is a rectangle of artwork and seen from the front
    /// is a *machine* — the head standing over the playfield with its own
    /// backglass lit, which is how anybody would recognise which one it is.
    /// With the full renderer even when the flat engine is on, because a
    /// photograph is worth one frame of the real thing and the flat world
    /// has no camera to move.
    ///
    /// The frame lands on the surface like any other and is gone on the next
    /// one, so whoever asks for this has to take the canvas straight after —
    /// which is exactly what the page does, with the loop held.
    pub fn shoot(&mut self) -> Result<(), FrameError> {
        let Some(camera) = self.camera_for(crate::camera::View::Front) else {
            return Ok(());
        };
        let (was_view, was_camera, was_flat) = (self.view, self.camera, self.flat_on);
        self.view = crate::camera::View::Front;
        self.camera = camera;
        self.flat_on = false;

        let out = self.render();

        self.view = was_view;
        self.camera = was_camera;
        self.flat_on = was_flat;
        out
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
        // The light the table asked to be seen under, or the shipped map when
        // it asked for none (`Renderer.cpp:208-210`). Per table, not per
        // pipeline: on F-14 it is the only light there is. A copy is kept, so
        // that a player who tries a room and comes back gets the table's own
        // light and not a reload.
        let table_env = crate::env::EnvMap::for_table(&self.gpu.device, &self.gpu.queue, scene);
        self.table_env = Some(table_env.clone());
        self.pipeline.set_envmap(&self.gpu.device, table_env);
        // The floor's picture, for the ball's planar reflection.
        if let Some(view) = gpu_scene.field_picture.clone() {
            self.pipeline.set_field_picture(&self.gpu.device, view);
        }

        let pf = scene.playfield;
        // The sheet itself, flat, and separately the corners of everything
        // standing on it.
        //
        // It used to be one box, the sheet raised by a guessed `0.45 * width`.
        // Two things were wrong with that and they compounded. The guess was
        // nearly twice F-14's real 235; and a box says the tallest thing on the
        // table stands in all four corners of it, which is exactly the claim a
        // camera looking straight down is most expensive to satisfy. Keeping
        // the two apart lets the camera ask where the tall things actually are,
        // and the answer for every table is "not at the edges".
        self.framing = Some((
            vpw_table::geometry::Bounds {
                min: Vec3::new(pf.min.x, pf.min.y, 0.0),
                max: Vec3::new(pf.max.x, pf.max.y, 0.0),
            },
            vpw_table::backbox::Backbox::for_playfield(pf),
        ));
        self.built_head = scene.built_head;
        self.authored = Some(scene.view);
        self.cabinet = Some(scene.cabinet);
        self.occupied = scene.occupied();
        self.legacy = scene.legacy_bounds();
        self.reframe();
        self.lights
            .upload(&self.gpu.device, &self.gpu.queue, &self.pipeline, scene);
        self.flashers
            .upload(&self.gpu.device, &self.gpu.queue, scene);
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
        self.invalidate_flat();
    }

    pub fn unload(&mut self) {
        self.scene = None;
        self.dynamic = None;
        self.lights.lights.clear();
        self.flashers = Flashers::new(
            &self.gpu.device,
            &self.gpu.queue,
            &self.pipeline.light_frame_layout,
            self.gpu.hdr_format,
            self.gpu.msaa_samples(self.gpu.hdr_format),
        );
    }

    /// The moving pieces, to move them before drawing.
    /// The lamps, to turn on and off as the game does.
    pub fn lights_mut(&mut self) -> &mut crate::lights::Lights {
        &mut self.lights
    }

    /// The flashers, to switch and fade as the script does.
    ///
    /// Handed back with the device and the queue, because setting a flasher
    /// may need both: a script that swaps a picture needs a new bind group,
    /// and everything else is a buffer write.
    pub fn flashers_mut(&mut self) -> (&mut Flashers, &wgpu::Device, &wgpu::Queue) {
        (&mut self.flashers, &self.gpu.device, &self.gpu.queue)
    }

    /// Puts a frame of the machine's dot matrix on the flashers that show it.
    ///
    /// The same frame that goes onto the head through [`Self::set_display`],
    /// by a different route: the head wants a picture with the dots already
    /// drawn, a flasher wants the dots themselves and draws them in its own
    /// shader, the way the original does (`Renderer::SetupDMDRender`,
    /// `Renderer.cpp:1420`). Nothing to do on a table with no such flasher.
    pub fn set_dmd(&mut self, dots: &[u8], width: usize, height: usize) {
        if !self.flashers.shows_dmd() {
            return;
        }
        self.flashers
            .set_dmd(&self.gpu.device, &self.gpu.queue, dots, width, height);
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

    /// Points the renderer at a different canvas, keeping everything else.
    ///
    /// See [`crate::GpuContext::attach`] for why this is needed at all. The
    /// offscreen targets are rebuilt because they are sized to the surface, and
    /// the camera is framed again because the new canvas may be a different
    /// shape; the uploaded table is untouched.
    pub fn attach(
        &mut self,
        target: wgpu::SurfaceTarget<'static>,
        width: u32,
        height: u32,
    ) -> Result<(), crate::GpuInitError> {
        self.gpu.attach(target, width, height)?;
        self.post
            .resize(&self.gpu.device, &self.gpu.queue, width, height);
        self.pipeline.set_probes(
            &self.gpu.device,
            self.post.transmission_view(),
            self.post.sampler(),
            self.post.reflection_view(),
        );
        self.reframe();
        Ok(())
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

    /// Traces the GI groups' lightmaps and hands them to the frame.
    ///
    /// Returns how many groups were baked; zero means the table names no GI
    /// string — or brought its own bake, which is better than ours. See
    /// `crate::bake` for what and why.
    pub fn bake_gi(&mut self, scene: &vpw_table::geometry::Scene) -> usize {
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
        // wgpu-hal reads a texture's shape as its intent, and one layer looks
        // like a plain picture to it rather than the array the shader binds.
        // A table whose GI is a single relay gets a spare black layer, and
        // the guess and the binding agree.
        let layers = (bake.layers.len() as u32).max(2);
        data.resize(layers as usize * bake.layers[0].len(), 0);
        let texture = &self.gpu.device.create_texture_with_data(
            &self.gpu.queue,
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
            &self.gpu.device,
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
            bake.layers.len() as u32,
        );
        self.lights.set_baked_groups(groups);
        // The lightmap changes what every photograph of the field shows.
        self.invalidate_flat();
    }

    /// Sets the player's day/night, 0 to 1, or clears it with `None`.
    ///
    /// Replaces the table's own global emission scale, exactly as the
    /// original's user mode does — which means dividing the baked factor out
    /// and multiplying the chosen one in, on the three terms the original
    /// scales by it (`Renderer.cpp:1037,1051,1063`): the scene lights, the
    /// ambient, and the environment.
    /// Puts the table in a room of the player's choosing, or back in its own.
    ///
    /// `Some(bytes)` is a Radiance `.hdr` equirectangular map — a real room,
    /// with its dim walls and its few bright lamps kept twenty times apart,
    /// which is what makes the reflections on the steel read as somewhere.
    /// `None` restores the environment the table asked for. Returns whether
    /// the map was accepted; a table that has not loaded yet has nothing to
    /// restore and nothing to light, and says no.
    pub fn set_environment(&mut self, hdr: Option<&[u8]>) -> bool {
        self.invalidate_flat();
        match hdr {
            Some(bytes) => {
                let Some(map) =
                    crate::env::EnvMap::from_hdr(&self.gpu.device, &self.gpu.queue, bytes, "room")
                else {
                    log::warn!("the room's environment map does not decode");
                    return false;
                };
                self.pipeline.set_envmap(&self.gpu.device, map);
                true
            }
            None => match self.table_env.clone() {
                Some(map) => {
                    self.pipeline.set_envmap(&self.gpu.device, map);
                    true
                }
                None => false,
            },
        }
    }

    /// Turns the playfield's reflection probe off and on.
    ///
    /// The probe is a whole second pass over the scene — on a heavy modern
    /// table that is another six hundred thousand triangles a frame — and
    /// vertex work is the one cost that shrinking the render resolution does
    /// not touch. The governor drops it on the low rungs, exactly as the
    /// original's quality settings do (`RenderProbe::REFL_NONE` is a level).
    pub fn set_reflection_enabled(&mut self, on: bool) {
        self.reflection_enabled = on;
    }

    /// Renders the scene at a fraction of the surface, the composite
    /// stretching it back. See `Post::set_scale` for why this never blinks.
    pub fn set_render_scale(&mut self, scale: f32) {
        // Only when it really changes. The governor calls this on every rung
        // it steps to, including the first one — which is the reflection
        // probe going off at the *same* resolution — and throwing the flat
        // bake away on a scale that did not move meant a slow bake made the
        // governor step, the step restarted the bake, and the bake never
        // finished.
        if (self.render_scale - scale).abs() < 1e-3 {
            return;
        }
        self.render_scale = scale;
        self.invalidate_flat();
        self.post
            .set_scale(&self.gpu.device, &self.gpu.queue, scale);
        // The probes' views were rebuilt with the targets; the material
        // pipeline holds bind groups onto them and has to be told, exactly
        // as a resize tells it.
        self.pipeline.set_probes(
            &self.gpu.device,
            self.post.transmission_view(),
            self.post.sampler(),
            self.post.reflection_view(),
        );
    }

    pub fn set_day_night(&mut self, scale: Option<f32>) {
        self.day_night = scale.map(|s| s.clamp(0.0, 1.0));
        self.invalidate_flat();
    }

    /// The table's lighting with the player's day/night applied, if any.
    fn effective_lighting(
        &self,
        lighting: &vpw_table::geometry::Lighting,
    ) -> vpw_table::geometry::Lighting {
        let Some(user) = self.day_night else {
            return *lighting;
        };
        let factor = user / lighting.global.max(1e-6);
        let mut out = *lighting;
        out.emission = out.emission.map(|c| c * factor);
        out.ambient = out.ambient.map(|c| c * factor);
        out.env_scale *= factor;
        out
    }

    /// Draws a frame. With no table loaded it only clears, which is still
    /// enough to tell that the chain down to WebGPU is alive.
    pub fn render(&mut self) -> Result<(), FrameError> {
        // The aspect comes from the surface; the pixel sizes in the frame
        // uniform come from the scene buffers, which the quality ladder may
        // have shrunk — screen-space lookups live in those pixels.
        let (sw, sh) = self.gpu.size();
        let aspect = sw as f32 / sh as f32;
        let (w, h) = self.post.scene_size();

        let Some(scene) = &self.scene else {
            return self.gpu.render();
        };

        let lighting = self.effective_lighting(&scene.lighting);

        // The flat bake, a few photographs a frame, goes first: it writes
        // frame uniforms and lamp levels of its own, and the live
        // `set_frame` below takes the frame back afterwards.
        if self.flat_on
            && let Some(flat) = &mut self.flat
            && !flat.ready()
        {
            flat.bake_step(
                &self.gpu.device,
                &self.gpu.queue,
                &self.pipeline,
                &self.post,
                scene,
                &mut self.lights,
                self.camera.view_projection(aspect),
                self.camera.eye(),
                &lighting,
                self.view.shows_backbox(),
                // Always with the playfield's mirror, whatever the governor
                // decided for the live path: the probe's cost is per
                // photograph here, not per frame, and the photographs are
                // what the player will look at for the rest of the session.
                true,
                3,
            );
        }

        // The frame's light lists, before any pass records. In the flat
        // path the bake above may have prepared a forced view; this puts the
        // live levels back.
        self.lights.prepare(&self.gpu.device, &self.gpu.queue, None);
        let gi = self.lights.gi_sources(crate::scene::MAX_GI_BULBS);
        self.pipeline.set_frame(
            &self.gpu.queue,
            self.camera.view_projection(aspect),
            self.camera.eye(),
            &lighting,
            (w, h),
            &gi,
            scene.field,
        );
        self.post
            .set_exposure(&self.gpu.queue, scene.lighting.exposure);
        // The table's own bloom, not the default. A table whose lamps run to an
        // intensity of two hundred asks for very little of it, and giving it
        // the default anyway smears every lit insert over its neighbours.
        self.post
            .set_strength(&self.gpu.queue, scene.lighting.bloom_strength);

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
        let head = self.view.shows_backbox();
        if let Some(flat) = self.flat.as_ref().filter(|f| self.flat_on && f.ready()) {
            // The flat frame: the photographs plus everything that moves.
            // The transmitted-light buffer still runs — the live pieces'
            // materials read it — but the scene and reflection passes, the
            // heavy ones, are what the photographs replaced.
            crate::pass::draw_lights_only(
                &mut encoder,
                self.post.transmission_view(),
                &self.pipeline,
                &self.lights,
            );
            self.post.blur_transmission(&mut encoder);
            flat.draw(
                &self.gpu.queue,
                &mut encoder,
                &self.post,
                &self.pipeline,
                scene,
                self.dynamic.as_ref(),
                &self.lights,
                Some(&self.flashers),
                head,
            );
            self.post.finish(&mut encoder, &view);
            self.gpu.queue.submit(Some(encoder.finish()));
            self.gpu.present(frame);
            return Ok(());
        }
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
        if self.reflection_enabled && scene.lighting.reflection_strength > 0.0 {
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
        // The head is left out of the views that do not show it. Looking
        // straight down it is a vertical panel seen edge-on — a grey stripe
        // across the top of the picture, standing where the glass would be —
        // and the whole point of that view is that the screen is the glass.
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
            move |b| head || !b.backbox,
        );
        self.post.finish(&mut encoder, &view);
        self.gpu.queue.submit(Some(encoder.finish()));
        self.gpu.present(frame);
        Ok(())
    }
}
