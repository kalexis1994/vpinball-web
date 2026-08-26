//! WebGPU renderer.
//!
//! Replaces the original's `src/renderer/` (DirectX 11/12, OpenGL, bgfx, VR).
//! None of that gets ported: we target a single backend, WebGPU, with no VR,
//! no anaglyph and no editor mode.

pub mod bake;
pub mod camera;
pub mod dynamic;
pub mod env;
pub mod flashers;
pub mod lights;
pub mod pass;
pub mod pipeline;
pub mod post;
pub mod scene;
pub mod segments;
pub mod table_renderer;

#[cfg(not(target_arch = "wasm32"))]
pub mod offscreen;

pub use camera::Camera;
pub use dynamic::{DynamicParts, MAX_BALLS};
pub use env::EnvMap;
pub use flashers::Flashers;
pub use lights::Lights;
pub use pipeline::TablePipeline;
pub use post::Post;
pub use scene::{Batch, GpuScene, GpuVertex, SceneStats};
pub use table_renderer::TableRenderer;

/// Graphics context: device, queue and surface, already configured.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// What the offscreen passes can draw into on this device, decided once.
    /// See [`crate::post::hdr_format`].
    pub hdr_format: wgpu::TextureFormat,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// Kept so another surface can be made later. See [`GpuContext::attach`].
    ///
    /// A surface belongs to the instance that made it, and configuring one with
    /// a device from a different instance is not a thing wgpu supports. Letting
    /// the instance fall out of `new` would mean the only canvas this context
    /// can ever draw to is the one it was born with.
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
}

#[derive(Debug)]
pub enum GpuInitError {
    NoAdapter(String),
    NoDevice(String),
    Surface(String),
}

impl std::fmt::Display for GpuInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAdapter(e) => write!(f, "no WebGPU adapter found: {e}"),
            Self::NoDevice(e) => write!(f, "could not create the device: {e}"),
            Self::Surface(e) => write!(f, "could not create the surface: {e}"),
        }
    }
}

impl std::error::Error for GpuInitError {}

/// Why a frame could not be produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The surface handed back no texture, not even after reconfiguring it.
    Unavailable,
    /// Validation error inside `get_current_texture`.
    Validation,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => write!(f, "the surface is not available"),
            Self::Validation => write!(f, "validation error while acquiring the frame"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Which backends the instance may try.
///
/// On the web the browser decides most of it: `Backends::all()` is WebGPU
/// with WebGL2 behind it, and wgpu takes WebGPU when `navigator.gpu` exists —
/// which is exactly the secure-context question, because an insecure origin
/// has no `navigator.gpu` at all and lands on WebGL2, which predates the
/// requirement. The renderer was built inside WebGL2's envelope on purpose
/// (`downlevel_webgl2_defaults`, no compute), so both doors open onto the
/// same room.
///
/// `VPW_FORCE_WEBGL` on the global scope pins it to GL, so the path a phone
/// on plain HTTP takes can be exercised on a desktop that would never take
/// it. The page sets it from `?gpu=gl`.
fn requested_backends() -> wgpu::Backends {
    #[cfg(target_arch = "wasm32")]
    {
        let forced = js_sys::Reflect::get(
            &js_sys::global(),
            &wasm_bindgen::JsValue::from_str("VPW_FORCE_WEBGL"),
        );
        if forced.is_ok_and(|v| v.is_truthy()) {
            return wgpu::Backends::GL;
        }
    }
    wgpu::Backends::from_env().unwrap_or(wgpu::Backends::all())
}

impl GpuContext {
    /// Initialises WebGPU against the given target (on the web, a `<canvas>`).
    pub async fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, GpuInitError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: requested_backends(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });

        let surface = instance
            .create_surface(target)
            .map_err(|e| GpuInitError::Surface(e.to_string()))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(|e| GpuInitError::NoAdapter(e.to_string()))?;

        // Which door was opened. On the web there are two — WebGPU, and
        // WebGL2 through the GL backend — and everything downstream behaves
        // identically, so this line is the only place the difference shows.
        {
            let info = adapter.get_info();
            log::info!("adapter: {} ({:?})", info.name, info.backend);
        }

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("vpw-device"),
                // We start from the downlevel limits so as not to shut the
                // door on mobile, which is exactly the case we want to improve
                // on relative to the original engine.
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await
            .map_err(|e| GpuInitError::NoDevice(e.to_string()))?;

        // Say something when the driver rejects what we built.
        //
        // Without this a shader that fails validation is completely silent from
        // in here: the pipeline is never created, nothing is drawn with it, and
        // the canvas is simply black while the rest of the table — the physics,
        // the ROM, the sound — carries on perfectly. That is a bad half hour,
        // and it cost one: a `textureSample` behind a per-fragment branch,
        // which native wgpu accepts and a browser does not.
        device.on_uncaptured_error(std::sync::Arc::new(|error| {
            log::error!("the graphics device rejected something: {error}");
        }));

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        // WebGPU does not offer an sRGB format for a canvas: the capabilities
        // come back as `Bgra8Unorm` and nothing else. The shader works in
        // linear light and expects the conversion to happen on the way out, so
        // a canvas configured as plain unorm shows every colour raw — which
        // looks like the table is in a dark room, evenly and everywhere, with
        // nothing obviously broken to point at.
        //
        // The fix is not to change the shader: it is to render through a view
        // that has the sRGB variant of the same format. The surface stays as
        // the platform wants it and the hardware does the encode.
        let view_format = format.add_srgb_suffix();
        let view_formats = if view_format == format {
            Vec::new()
        } else {
            vec![view_format]
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats,
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        Ok(Self {
            hdr_format: crate::post::hdr_format(&adapter),
            device,
            queue,
            surface,
            config,
            instance,
            adapter,
        })
    }

    /// Points the context at a different canvas.
    ///
    /// A page that leaves the game and comes back builds a **new** canvas
    /// element, and a surface is bound to the element it was made from — not to
    /// its id. Without this the renderer carries on drawing, perfectly, into a
    /// canvas that is no longer in the document: the sound plays, the controls
    /// are there, and the table is a black rectangle. That is the bug this
    /// exists for.
    ///
    /// The device, the pipelines and everything already uploaded are kept. Only
    /// the surface is rebuilt, which is what makes coming back from the menu
    /// cost nothing rather than costing a hundred-megabyte re-upload.
    pub fn attach(
        &mut self,
        target: wgpu::SurfaceTarget<'static>,
        width: u32,
        height: u32,
    ) -> Result<(), GpuInitError> {
        let surface = self
            .instance
            .create_surface(target)
            .map_err(|e| GpuInitError::Surface(e.to_string()))?;

        // The format is re-read rather than assumed: it is a property of the
        // surface, and a second canvas is not obliged to offer the same one.
        let caps = surface.get_capabilities(&self.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let view_format = format.add_srgb_suffix();

        self.config.format = format;
        self.config.view_formats = if view_format == format {
            Vec::new()
        } else {
            vec![view_format]
        };
        self.config.alpha_mode = caps.alpha_modes[0];
        self.config.width = width.max(1);
        self.config.height = height.max(1);

        surface.configure(&self.device, &self.config);
        self.surface = surface;
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.config.width == width && self.config.height == height {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// The format the pipelines have to be built for.
    ///
    /// The **view's** format, not the surface's: on a canvas those differ, and
    /// building a pipeline for the surface's while drawing into an sRGB view is
    /// how you get a validation error instead of a picture. See where the
    /// surface is configured.
    #[inline]
    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format.add_srgb_suffix()
    }

    /// A view of a frame, in the format [`Self::format`] promises.
    pub fn frame_view(&self, frame: &wgpu::SurfaceTexture) -> wgpu::TextureView {
        frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.format()),
            ..Default::default()
        })
    }

    #[inline]
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Takes the texture for the next frame.
    ///
    /// `Ok(None)` is not an error: it means this frame gets skipped (the tab
    /// is covered, or the surface took too long) and the caller retries on the
    /// next tick.
    pub fn acquire(&mut self) -> Result<Option<wgpu::SurfaceTexture>, FrameError> {
        use wgpu::CurrentSurfaceTexture as Cst;
        Ok(Some(match self.surface.get_current_texture() {
            Cst::Success(f) | Cst::Suboptimal(f) => f,
            // The surface went stale (resize, DPI change): we reconfigure it
            // and retry exactly once.
            Cst::Outdated | Cst::Lost => {
                self.surface.configure(&self.device, &self.config);
                match self.surface.get_current_texture() {
                    Cst::Success(f) | Cst::Suboptimal(f) => f,
                    _ => return Err(FrameError::Unavailable),
                }
            }
            Cst::Timeout | Cst::Occluded => return Ok(None),
            Cst::Validation => return Err(FrameError::Validation),
        }))
    }

    pub fn present(&self, frame: wgpu::SurfaceTexture) {
        self.queue.present(frame);
    }

    /// Clears the screen. It is what gets drawn while no table is loaded.
    pub fn render(&mut self) -> Result<(), FrameError> {
        let Some(frame) = self.acquire()? else {
            return Ok(());
        };

        let view = self.frame_view(&frame);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vpw-frame"),
            });

        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vpw-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(crate::pass::CLEAR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(frame);
        Ok(())
    }
}
