//! The pieces that move, on the GPU.
//!
//! The static scene bakes its transform once and never touches it again
//! ([`crate::scene`]). That is right for the three thousand parts of a table
//! that stand still and wrong for the dozen that do not: a flipper, a gate, a
//! spinner, a trigger, a bumper's ring, the plunger and the balls.
//!
//! Those go here. Each one keeps its vertices in a local frame, gets its own
//! model matrix in a uniform of its own, and gets its own draw call. Twelve
//! draw calls against a table's twenty-six does not move the needle, and it is
//! the only way a flipper can rotate without re-uploading its vertices.
//!
//! # How it is arranged
//!
//! One vertex buffer and one index buffer for **all** the moving pieces
//! together, same as the static scene: a part is a range of indices, and the
//! buffers are bound once for the whole pass. What changes per draw is group 1
//! (the material) and group 2 (the matrix).
//!
//! Balls are a special case: they all share one range of indices —the same
//! mesh, uploaded once— and differ only in their matrix. Slots for them are
//! reserved up front and hidden while unused, so that starting a multiball does
//! not allocate a buffer in the middle of a frame.

use crate::pipeline::TablePipeline;
use crate::scene::{GpuVertex, table_sampler, white_texture};
use vpw_math::{Mat4, Vec3};
use vpw_table::animation::AnimatedPart;
use vpw_table::geometry::{Material, Mesh, Scene};
use wgpu::util::DeviceExt;

/// How many balls can be on the table at once.
///
/// The most crowded multiball in existence does not reach a dozen. Reserving
/// eight and hiding the unused ones costs eight matrices worth of memory and
/// saves allocating anything while the ball is in play.
pub const MAX_BALLS: usize = 8;

/// What the vertex stage needs of each piece. Mirrors `struct Model` in
/// `dynamic_vs.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuModel {
    model: [[f32; 4]; 4],
    /// Inverse transpose of the above, for the normals.
    normal: [[f32; 4]; 4],
}

impl GpuModel {
    fn new(m: Mat4) -> Self {
        Self {
            model: m.to_cols_array_2d(),
            normal: m.inverse().transpose().to_cols_array_2d(),
        }
    }
}

/// What a piece that is **light** needs remembering between frames.
///
/// See [`vpw_table::geometry::Additive`]. The layer is added in proportion to
/// how bright its lamp is right now against how bright that lamp is at full
/// power, so its material is rewritten whenever the lamp moves — and only
/// then.
struct LightLayer {
    /// The uniform holding the material, to write the new brightness into.
    uniform: wgpu::Buffer,
    /// The lamp's name as the file gives it, until [`DynamicParts::link_lights`]
    /// turns it into an index.
    name: Option<String>,
    color: [f32; 3],
    /// The layer's own alpha, before the lamp is taken into account.
    alpha: f32,
    /// What was last written, so a still frame writes nothing.
    written: f32,
}

/// One moving piece: a range of indices, a material and a matrix.
struct Part {
    first_index: u32,
    index_count: u32,
    material: wgpu::BindGroup,
    model_buffer: wgpu::Buffer,
    model: wgpu::BindGroup,
    transparent: bool,
    /// Hidden pieces are skipped. Only the reserved ball slots use it.
    visible: bool,
    /// What was last written to `model_buffer`.
    ///
    /// Most of a table's moving parts stand still most of the time: measured on
    /// F-14, the median number that move between one frame and the next is
    /// **zero**, out of seventy-three. Writing them all anyway was four
    /// thousand pointless buffer writes a second, each one a trip out of wasm
    /// and into the GPU process. The lights already skip an unchanged write
    /// (`Lights::set_state`); this is the same thing for the parts.
    written: Mat4,
    /// Set when this piece is light rather than a thing. See [`Layer`].
    layer: Option<LightLayer>,
}

/// Everything that moves, ready to draw.
pub struct DynamicParts {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    parts: Vec<Part>,
    /// Where the ball slots start inside `parts`. The table's own pieces come
    /// first, so an index into `parts` is also an index into the
    /// `Vec<AnimatedPart>` it was built from.
    first_ball: usize,
}

impl DynamicParts {
    /// The layout of group 2: one matrix per piece.
    pub fn model_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vpw-model-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        })
    }

    /// Uploads the table's moving pieces plus the ball slots.
    ///
    /// `scene` is only used to resolve materials and textures by name, which is
    /// why the balls —which are in no `.vpx`— carry their own.
    pub fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &TablePipeline,
        scene: &Scene,
        animated: &[AnimatedPart],
        ball: &Mesh,
        ball_material: &Material,
    ) -> Self {
        let sampler = table_sampler(device);
        let white = white_texture(device, queue);

        let mut vertices: Vec<GpuVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut parts = Vec::with_capacity(animated.len() + MAX_BALLS);

        // A piece's vertices go in **unbaked**: the whole point is that its
        // transform is alive.
        let push_mesh = |m: &Mesh, vertices: &mut Vec<GpuVertex>, indices: &mut Vec<u32>| {
            let first_index = indices.len() as u32;
            let base = vertices.len() as u32;
            vertices.extend(m.vertices.iter().map(|v| GpuVertex {
                pos: v.pos,
                normal: v.normal,
                uv: v.uv,
            }));
            indices.extend(m.indices.iter().map(|i| i + base));
            (first_index, indices.len() as u32 - first_index)
        };

        // One copy of each picture, however many parts name it. See
        // [`crate::scene::TextureCache`].
        let mut textures = crate::scene::TextureCache::new();
        let mut make_part = |device: &wgpu::Device,
                             first_index: u32,
                             index_count: u32,
                             material: Option<&Material>,
                             image: Option<&vpw_table::geometry::Image>,
                             transform: Mat4,
                             visible: bool|
         -> (Part, wgpu::Buffer) {
            let slot = crate::scene::material_slot_cached(
                device,
                queue,
                &pipeline.material_layout,
                &sampler,
                &white,
                material,
                image,
                false,
                &mut textures,
            );
            let with_alpha = image.is_some_and(|i| i.has_alpha);
            let transparent = material.is_some_and(|m| m.is_transparent(with_alpha));

            let model_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vpw-model"),
                contents: bytemuck::bytes_of(&GpuModel::new(transform)),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let model = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vpw-model-bg"),
                layout: &pipeline.model_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: model_buffer.as_entire_binding(),
                }],
            });
            (
                Part {
                    first_index,
                    index_count,
                    written: transform,
                    material: slot.bind_group,
                    model,
                    model_buffer,
                    transparent,
                    visible,
                    layer: None,
                },
                slot.uniform,
            )
        };

        for part in animated {
            let (first_index, index_count) = push_mesh(&part.mesh, &mut vertices, &mut indices);
            let (mut part_made, uniform) = make_part(
                device,
                first_index,
                index_count,
                scene.material(&part.mesh.material),
                scene.image(&part.mesh.image),
                part.mesh.transform,
                true,
            );
            // A piece that is light rather than a thing: its material is not a
            // surface at all, and its brightness follows a lamp. The slot's
            // uniform is overwritten rather than the group rebuilt, because it
            // will be overwritten again on the next frame the lamp moves.
            if let Some(a) = &part.mesh.additive {
                queue.write_buffer(
                    &uniform,
                    0,
                    bytemuck::bytes_of(&crate::scene::additive_material(a.color, a.alpha)),
                );
                part_made.layer = Some(LightLayer {
                    uniform: uniform.clone(),
                    name: a.light.clone(),
                    color: a.color,
                    alpha: a.alpha,
                    written: f32::NAN,
                });
            }
            parts.push(part_made);
        }

        let first_ball = parts.len();
        let (ball_first, ball_count) = push_mesh(ball, &mut vertices, &mut indices);
        // The wear the ball carries: the table's own decal when it brought
        // one (`BLIF`), the made-here scratches when it did not. It rides the
        // mesh's own UVs, and the mesh turns with the physics' quaternion —
        // which is what finally makes the roll *visible*: a perfect mirror
        // looks the same from every orientation, scuffs do not.
        //
        // A table's decal is authored for the original's blending — marks in
        // the colour channels, coverage in the alpha, meant to be added over
        // its ball image — and fed to the material path raw it *becomes* the
        // ball: alpha near zero everywhere, so the steel vanishes and only
        // the scratches float. `ball_wear` folds it into the one shape the
        // shader wants: a multiplicative map, white where the steel is
        // untouched, dimmed where a mark scatters what the mirror would have
        // returned.
        let decal = soften(
            scene
                .image(&scene.ball_decal)
                .and_then(ball_wear)
                .unwrap_or_else(vpw_table::ball::scratches),
        );
        for _ in 0..MAX_BALLS {
            parts.push(
                make_part(
                    device,
                    ball_first,
                    ball_count,
                    Some(ball_material),
                    Some(&decal),
                    Mat4::IDENTITY,
                    false,
                )
                .0,
            );
        }

        // And a shadow slot behind every ball slot: a soft dark disc
        // projected onto the playfield, which is most of what keeps a ball
        // looking like it is *on* the table — above all in the flat mode,
        // where the table under it is a photograph.
        let shadow_mesh = vpw_table::ball::shadow_mesh();
        let (shadow_first, shadow_count) = push_mesh(&shadow_mesh, &mut vertices, &mut indices);
        let shadow_material = vpw_table::ball::shadow_material();
        let shadow_image = vpw_table::ball::shadow_image();
        for _ in 0..MAX_BALLS {
            parts.push(
                make_part(
                    device,
                    shadow_first,
                    shadow_count,
                    Some(&shadow_material),
                    Some(&shadow_image),
                    Mat4::IDENTITY,
                    false,
                )
                .0,
            );
        }

        Self {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vpw-dynamic-vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vpw-dynamic-indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            parts,
            first_ball,
        }
    }

    /// How many table pieces there are, not counting the ball slots.
    pub fn table_parts(&self) -> usize {
        self.first_ball
    }

    /// Moves one of the table's pieces.
    pub fn set_part_transform(&mut self, queue: &wgpu::Queue, index: usize, m: Mat4) {
        self.write(queue, index, Some(m));
    }

    /// Shows or hides one piece.
    ///
    /// A script does this constantly, and on a table with primitive flippers it
    /// is not decoration: the built-in flipper is hidden and a primitive bat is
    /// shown in its place, so drawing both gives one flipper that swings and
    /// another that does not.
    pub fn set_part_visible(&mut self, index: usize, visible: bool) {
        if let Some(part) = self.parts.get_mut(index) {
            part.visible = visible;
        }
    }

    /// Moves ball `index`, or hides it with `None`. Its shadow follows: the
    /// ball's translation dropped to the playfield, its radius read back off
    /// the matrix — the transform is `translation · scale · rotation`, so a
    /// basis vector's length is the radius.
    pub fn set_ball_transform(&mut self, queue: &wgpu::Queue, index: usize, m: Option<Mat4>) {
        if index >= MAX_BALLS {
            return;
        }
        self.write(queue, self.first_ball + index, m);
        let shadow = m.map(|m| {
            let at = m.w_axis;
            let radius = m.x_axis.truncate().length().max(1.0);
            vpw_table::ball::shadow_transform(Vec3::new(at.x, at.y, 0.0), radius)
        });
        self.write(queue, self.first_ball + MAX_BALLS + index, shadow);
    }

    fn write(&mut self, queue: &wgpu::Queue, index: usize, m: Option<Mat4>) {
        let Some(part) = self.parts.get_mut(index) else {
            return;
        };
        match m {
            Some(m) => {
                part.visible = true;
                // An exact comparison on purpose: the question is not whether
                // the piece moved enough to see, it is whether the bytes about
                // to be sent are the ones already there.
                if m != part.written {
                    part.written = m;
                    queue.write_buffer(
                        &part.model_buffer,
                        0,
                        bytemuck::bytes_of(&GpuModel::new(m)),
                    );
                }
            }
            None => part.visible = false,
        }
    }

    /// Whether anything is going to be drawn in the given pass.
    pub fn any(&self, transparent: bool) -> bool {
        self.parts
            .iter()
            .any(|p| p.visible && p.transparent == transparent && p.index_count > 0)
    }

    /// The lamp each additive layer belongs to, by the part's index.
    ///
    /// Resolved by the caller against the *table's* lamps and not the
    /// renderer's, for the reason a bake exists at all: the lamps whose light
    /// has been baked into these layers are switched **invisible** in the file
    /// — their bulbs and halos are already painted into the bake — and an
    /// invisible light is not one this renderer carries. Its state is still
    /// live in the script, which is the only place left to ask. The flashers
    /// resolve their own light-map link the same way.
    pub fn layer_lights(&self) -> impl Iterator<Item = (usize, Option<&str>)> {
        self.parts
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.layer.as_ref().map(|l| (i, l.name.as_deref())))
    }

    /// How much of layer `index` to add: 0 for a lamp that is off, 1 for one at
    /// full power, and the fraction in between while it fades.
    ///
    /// The original's `m_currentIntensity / (m_intensity * m_intensity_scale)`
    /// (`primitive.cpp:1080`). Writes nothing when the lamp has not moved,
    /// which on a table with hundreds of lamps and a handful changing is
    /// nearly every layer, every frame.
    pub fn set_layer_level(&mut self, queue: &wgpu::Queue, index: usize, level: f32) {
        let Some(part) = self.parts.get_mut(index) else {
            return;
        };
        let Some(layer) = &mut part.layer else { return };
        let alpha = layer.alpha * level.clamp(0.0, 1.0);
        if (alpha - layer.written).abs() < 1.0 / 512.0 {
            return;
        }
        layer.written = alpha;
        queue.write_buffer(
            &layer.uniform,
            0,
            bytemuck::bytes_of(&crate::scene::additive_material(layer.color, alpha)),
        );
        // A layer with nothing left to add is not drawn at all
        // (`primitive.cpp:1088`).
        part.visible = alpha > 0.0;
    }

    /// Whether anything is light rather than a thing.
    pub fn any_additive(&self) -> bool {
        self.parts.iter().any(|p| p.visible && p.layer.is_some())
    }

    /// Emits the draws for the additive pass: the pieces that are light. Group
    /// 0 and the additive pipeline have to be bound already.
    pub fn draw_additive(&self, pass: &mut wgpu::RenderPass<'_>) {
        if !self.any_additive() {
            return;
        }
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        for p in &self.parts {
            if !p.visible || p.layer.is_none() || p.index_count == 0 {
                continue;
            }
            pass.set_bind_group(1, &p.material, &[]);
            pass.set_bind_group(2, &p.model, &[]);
            pass.draw_indexed(p.first_index..p.first_index + p.index_count, 0, 0..1);
        }
    }

    /// Emits the draws for one of the two lit passes. Group 0 has to be bound
    /// already; the pipeline too.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, transparent: bool) {
        if !self.any(transparent) {
            return;
        }
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        for p in &self.parts {
            // A layer of light is drawn by `draw_additive` and nowhere else.
            if !p.visible || p.layer.is_some() || p.transparent != transparent || p.index_count == 0
            {
                continue;
            }
            pass.set_bind_group(1, &p.material, &[]);
            pass.set_bind_group(2, &p.model, &[]);
            pass.draw_indexed(p.first_index..p.first_index + p.index_count, 0, 0..1);
        }
    }
}

/// A table's ball decal, folded into the wear map the shader expects.
///
/// The original blends the decal over its ball image — marks in the colour
/// channels, coverage in the alpha (`BallShader.hlsl`, scratches mode). The
/// material path here has no second layer to blend: the ball's one texture
/// multiplies its reflections. So the decal becomes that multiplier — white
/// where it covers nothing, dimmed in proportion to how much mark it carries —
/// which is what a scratch does to a mirror: scatter part of what it would
/// have returned. `None` when the image cannot be decoded, and the caller
/// falls back to the scratches made in `vpw_table::ball`.
/// Blurs the wear until it reads as haze rather than engraving.
///
/// A decal's strokes are drawn pixel-sharp, and pixel-sharp marks on a curved
/// mirror look like cracks in it. Real wear is thousands of overlapping
/// micro-scratches, and what the eye gets of it is a soft smudge. Two passes
/// of a small box blur are that smudge; the horizontal pass wraps, because
/// the sphere's UV seam is a meridian and a blur that stopped at it would
/// draw the seam on the ball.
fn soften(mut wear: vpw_table::geometry::Image) -> vpw_table::geometry::Image {
    let (w, h) = (wear.width as usize, wear.height as usize);
    let Some(px) = wear.rgba.as_mut() else {
        return wear;
    };
    let mut gray: Vec<u16> = px.as_chunks::<4>().0.iter().map(|t| t[0] as u16).collect();
    let mut pass = vec![0u16; gray.len()];
    const R: isize = 2;
    for _ in 0..2 {
        // Horizontal, wrapping.
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                for dx in -R..=R {
                    let sx = (x as isize + dx).rem_euclid(w as isize) as usize;
                    sum += gray[y * w + sx] as u32;
                }
                pass[y * w + x] = (sum / (2 * R + 1) as u32) as u16;
            }
        }
        // Vertical, clamped: the poles are not each other's neighbours.
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                for dy in -R..=R {
                    let sy = (y as isize + dy).clamp(0, h as isize - 1) as usize;
                    sum += pass[sy * w + x] as u32;
                }
                gray[y * w + x] = (sum / (2 * R + 1) as u32) as u16;
            }
        }
    }
    for (texel, g) in px.as_chunks_mut::<4>().0.iter_mut().zip(&gray) {
        let v = *g as u8;
        texel[0] = v;
        texel[1] = v;
        texel[2] = v;
    }
    wear
}

fn ball_wear(image: &vpw_table::geometry::Image) -> Option<vpw_table::geometry::Image> {
    let rgba: std::borrow::Cow<[u8]> = match (&image.rgba, &image.encoded) {
        (Some(px), _) => std::borrow::Cow::Borrowed(px),
        (None, Some(bytes)) => {
            std::borrow::Cow::Owned(image::load_from_memory(bytes).ok()?.to_rgba8().into_raw())
        }
        (None, None) => return None,
    };
    if rgba.len() < (image.width * image.height * 4) as usize {
        return None;
    }
    let px = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|texel| {
            // The mark is the colour itself. Decals of this era are additive
            // maps — black adds nothing, a bright stroke is a scratch — and
            // their alpha is noise (F-14's "Scratches" never rises above 43).
            let mark = texel[..3].iter().copied().max().unwrap_or(0) as u32;
            // Well under half strength: wear is something noticed on the
            // second look, and at full weight the ball reads as damaged
            // rather than played.
            let dim = 255 - (mark / 4) as u8;
            [dim, dim, dim, 255]
        })
        .collect();
    Some(vpw_table::geometry::Image {
        name: "vpw-ball-wear".into(),
        encoded: None,
        rgba: Some(px),
        width: image.width,
        height: image.height,
        has_alpha: false,
        alpha_test: -1.0,
        redrawn: false,
    })
}
