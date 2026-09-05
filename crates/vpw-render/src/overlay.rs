//! The score, drawn on the backdrop.
//!
//! When a table brings its own backdrop the head is not drawn — the original
//! draws none in desktop mode, the picture is the backglass — and the score
//! has to go somewhere on the picture instead. Where it *can* go is worked
//! out once from the table, in [`vpw_table::backdrop`]; which of those places
//! it goes is decided here, because it depends on where the table lands on
//! the screen, and that moves with every resize and every change of view.
//!
//! One textured rectangle per player, each written from a raster the same
//! way the head's panel is, with the alpha the raster carries so the paint
//! under the digits shows through as the window it is.
//!
//! Painted windows are drawn straight after the backdrop, before any part of
//! the table, so a window the playfield stands over is covered by it the way
//! the picture is. A window the table *declared* — and the strip the score
//! falls back to when nothing was painted — is drawn last and over
//! everything, which is where the original puts a declared one
//! (`textbox.cpp:317`).

use crate::segments::Raster;
use vpw_table::backdrop::{Quad, Rect};

/// A sprite's uniform: the rectangle it covers and the part of its texture
/// it shows, both as `[left, top, width, height]` fractions.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpriteUniform {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
}

/// The uniform for a sprite over `rect`, writable so the sprite can move.
pub fn sprite_uniform(device: &wgpu::Device, rect: Rect) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("vpw-sprite-uniform"),
        contents: bytemuck::bytes_of(&SpriteUniform {
            rect,
            uv: [0.0, 0.0, 1.0, 1.0],
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

/// A texture, a sampler and a sprite uniform, bound together for
/// `sprite.wgsl`.
pub fn sprite_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("vpw-sprite"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform.as_entire_binding(),
            },
        ],
    })
}

pub fn sprite_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("vpw-sprite-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

/// One player's picture and the rectangle it is currently drawn at.
struct Player {
    rect: Rect,
    texture: wgpu::Texture,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// The score windows on the backdrop, each with the picture it shows.
pub struct ScoreOverlay {
    /// Whether the windows go over the table rather than under it. See the
    /// module notes.
    pub over: bool,
    /// Every place the score could go, as the table gave them.
    candidates: Vec<Rect>,
    /// Whether they are alternatives for one window. See
    /// [`vpw_table::backdrop::ScoreWindows::pick_one`].
    pick_one: bool,
    /// The places it does go, by player; a player whose corner has no window
    /// has `None`. See [`Self::choose`].
    chosen: Vec<Option<Rect>>,
    /// A picture per player, as many as could ever be shown.
    players: Vec<Player>,
    sampler: wgpu::Sampler,
}

/// How many players a backglass has windows for.
const PLAYERS: usize = 4;

impl ScoreOverlay {
    /// Empty windows for the table's candidates. Empty is a transparent
    /// pixel: nothing shows until the machine says something.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        windows: &vpw_table::backdrop::ScoreWindows,
    ) -> Self {
        let sampler = sprite_sampler(device);
        let blank = Raster {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        };
        let candidates = windows.rects.clone();
        let chosen = Self::choice(&candidates, windows.pick_one, None);
        let slots = if windows.pick_one {
            1
        } else {
            candidates.len().min(PLAYERS)
        };
        let players = (0..slots)
            .map(|i| {
                let rect = chosen.get(i).copied().flatten().unwrap_or([0.0; 4]);
                let uniform = sprite_uniform(device, rect);
                let (texture, bind_group) =
                    Self::picture(device, queue, layout, &sampler, &uniform, &blank);
                Player {
                    rect,
                    texture,
                    uniform,
                    bind_group,
                }
            })
            .collect();
        Self {
            over: windows.over,
            candidates,
            pick_one: windows.pick_one,
            chosen,
            players,
            sampler,
        }
    }

    fn choice(candidates: &[Rect], pick_one: bool, table: Option<Quad>) -> Vec<Option<Rect>> {
        if pick_one {
            vpw_table::backdrop::pick_one(candidates, table)
                .into_iter()
                .map(Some)
                .collect()
        } else {
            vpw_table::backdrop::by_corner(candidates, table)
        }
    }

    fn picture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        uniform: &wgpu::Buffer,
        raster: &Raster,
    ) -> (wgpu::Texture, wgpu::BindGroup) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vpw-score-window"),
            size: wgpu::Extent3d {
                width: raster.width.max(1),
                height: raster.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // The raster is sRGB bytes, like every picture; the shader wants
            // linear light and the hardware does the decode.
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        write_raster(queue, &texture, raster);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = sprite_bind_group(device, layout, &view, sampler, uniform);
        (texture, bind_group)
    }

    /// Decides which windows the players get, given where the table stands
    /// on the screen.
    ///
    /// A window the table covers is no use to anybody, and a backdrop
    /// composed with the glass twice over — Spider-Man's — has eight of
    /// them, four of which are behind the playfield from any camera. So the
    /// covered ones are set aside and the rest are handed out by corner, the
    /// way a four-player glass is laid out. See
    /// [`vpw_table::backdrop::by_corner`].
    ///
    /// Called whenever the framing changes; nothing is rebuilt, only the
    /// rectangles rewritten.
    pub fn choose(&mut self, queue: &wgpu::Queue, table: Option<Quad>) {
        self.chosen = Self::choice(&self.candidates, self.pick_one, table);
        log::debug!(
            "score windows: table at {table:?}, {} candidates, chosen {:?}",
            self.candidates.len(),
            self.chosen
        );
        for (i, p) in self.players.iter_mut().enumerate() {
            let rect = self.chosen.get(i).copied().flatten().unwrap_or([0.0; 4]);
            if rect != p.rect {
                p.rect = rect;
                queue.write_buffer(
                    &p.uniform,
                    0,
                    bytemuck::bytes_of(&SpriteUniform {
                        rect,
                        uv: [0.0, 0.0, 1.0, 1.0],
                    }),
                );
            }
        }
    }

    /// How many player slots there are right now, the empty ones included.
    pub fn len(&self) -> usize {
        self.chosen.len().min(self.players.len())
    }

    pub fn is_empty(&self) -> bool {
        self.rects().iter().all(Option::is_none)
    }

    /// The windows by player: `None` for a player whose corner has none.
    pub fn rects(&self) -> Vec<Option<Rect>> {
        self.chosen[..self.len()].to_vec()
    }

    /// Puts a picture in player `i`'s window. The texture is written in place
    /// when the size has not changed, which is nearly always, and rebuilt
    /// when it has.
    pub fn set(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        i: usize,
        raster: &Raster,
    ) {
        let Some(p) = self.players.get_mut(i) else { return };
        if raster.width == 0 || raster.height == 0 {
            return;
        }
        if p.texture.width() != raster.width || p.texture.height() != raster.height {
            let (texture, bind_group) =
                Self::picture(device, queue, layout, &self.sampler, &p.uniform, raster);
            p.texture = texture;
            p.bind_group = bind_group;
        } else {
            write_raster(queue, &p.texture, raster);
        }
    }

    /// Draws every window in use. The pass has to have the sprite pipeline
    /// set.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        for (p, rect) in self.players.iter().zip(&self.chosen) {
            if rect.is_none() {
                continue;
            }
            pass.set_bind_group(0, &p.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}

fn write_raster(queue: &wgpu::Queue, texture: &wgpu::Texture, raster: &Raster) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
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
