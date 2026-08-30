//! Uploading a table to the GPU.
//!
//! # Where it departs from the original engine
//!
//! Visual Pinball keeps one object per part with its own matrix, its own
//! `MeshBuffer` and its own draw call, because the editor can mutate anything
//! at any moment. A player does not need that: once loaded, the table is
//! immutable except for the handful of parts the script animates.
//!
//! So here:
//!
//! - **The transform is baked.** Vertices are taken to world space at load
//!   time, once. There is no per-draw model matrix and no per-vertex multiply
//!   in the shader.
//! - **Everything goes into a single pair of buffers.** One vertex buffer and
//!   one index buffer for the whole table, and every batch is a range of
//!   indices. Buffers are never swapped between draws.
//! - **Batches are grouped by material and texture**, which is what actually
//!   costs something to change. The original sorts by backwards-compatibility
//!   rules — the comment at `RenderPass.cpp:80` says so in as many words:
//!   "designed to ensure backward compatibility".
//! - **Opaque and transparent are kept apart**, the opaque ones front to back
//!   so early-Z can throw away what is covered, and the transparent ones the
//!   other way round because there the order really does change the result.

use std::collections::HashMap;
use vpw_math::Vec3;
use vpw_table::geometry::{Mesh, Scene};
use wgpu::util::DeviceExt;

/// A vertex exactly as the shader consumes it: 32 bytes, same as in the `.vpx`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

impl GpuVertex {
    pub const ATTRS: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuMaterial {
    /// rgb = base color, a = alpha already resolved by `opacity_active`
    pub base_color: [f32; 4],
    /// rgb = specular color (without the 0.08), a = specular exponent
    pub glossy: [f32; 4],
    /// rgb = clearcoat layer already multiplied by 0.08, a = edge weight
    pub clearcoat: [f32; 4],
    /// x = has texture, y = is metal, z = wrap lighting, w = edge
    pub flags: [f32; 4],
    /// x = specular image lerp, y = thickness, z = alpha test,
    /// w = emissive: the texel is the light itself, skip the light loop
    pub extra: [f32; 4],
}

impl GpuMaterial {
    /// Builds the block from what `Material::shader_inputs` resolves.
    fn from_inputs(
        i: &vpw_table::geometry::ShaderInputs,
        has_texture: bool,
        alpha_test: f32,
    ) -> Self {
        Self {
            base_color: [i.base_color[0], i.base_color[1], i.base_color[2], i.alpha],
            glossy: [
                i.glossy_color[0],
                i.glossy_color[1],
                i.glossy_color[2],
                i.glossy_power,
            ],
            clearcoat: [i.clearcoat[0], i.clearcoat[1], i.clearcoat[2], i.edge_alpha],
            flags: [
                if has_texture { 1.0 } else { 0.0 },
                if i.is_metal { 1.0 } else { 0.0 },
                i.wrap_lighting,
                i.edge,
            ],
            extra: [i.glossy_image_lerp, i.thickness, alpha_test, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuFrame {
    pub view_proj: [[f32; 4]; 4],
    pub eye: [f32; 4],
    /// rgb = ambient, a = range of the lights
    pub ambient: [f32; 4],
    pub light0: [f32; 4],
    pub light1: [f32; 4],
    /// rgb = emission, a = environment weight
    pub emission: [f32; 4],
    /// x = envmap mip levels, y = envmap height. Exposure used to live here
    /// too; it moved to the post uniform when the tone mapping did.
    pub env: [f32; 4],
    /// xy = one screen pixel in UV. The original's `w_h_height`, and what lets
    /// a fragment look itself up in the transmitted-light buffer.
    pub screen: [f32; 4],
    /// The plane below which a fragment is thrown away, as `xyz` normal and `w`
    /// distance. All zero in the pass that draws the table for real; set only
    /// while the reflection probe is being drawn, where the point is to see
    /// what stands *above* the playfield and neither the playfield itself nor
    /// what is under it.
    pub clip: [f32; 4],
    /// xyz = the normal of the surface that reflects, w = how strongly.
    /// The original's `mirrorNormal_factor` (`primitive.cpp:1230`).
    pub mirror: [f32; 4],
    /// The general illumination: up to [`MAX_GI_BULBS`] lit bulb lights, two
    /// rows each — `[x, y, z, 1/range]` and `[r·K, g·K, b·K, falloff_power]`
    /// with the colour already scaled by the level and the calibration.
    /// `env.z` says how many rows are live.
    ///
    /// A **departure**, and the one that makes a dark table look like a
    /// machine instead of a photograph of one. The original draws these bulbs
    /// as screen-space halos that mostly *modulate* what is under them
    /// (`light.cpp:826-830`, its own words: "a very crude approximation of
    /// real lighting") — and modulating a playfield that the table's own
    /// lighting leaves black produces black. A real machine's GI string is
    /// thirty bulbs pouring real light onto the wood; in a dark arcade the
    /// machine glows. So the brightest lit bulbs are also fed to the material
    /// loop as point lights, which is what they are.
    /// rgb = the GI's first bounce: the lit bulbs' flux spread over the
    /// field, in their average colour. The flat term that keeps a corner no
    /// bulb reaches from being black. See [`crate::lights::GI_BOUNCE`].
    pub gi_bounce: [f32; 4],
    /// Each baked group's live level, scaling its lightmap layer. `env.z`
    /// says how many layers are live.
    pub gi_levels: [f32; 4],
    /// The playfield in world units — `[min.x, min.y, 1/width, 1/height]` —
    /// which is how a fragment that is not the playfield finds its place in
    /// the lightmap: a ball is steel, and steel shows the light around it.
    pub field: [f32; 4],
    pub gi: [[f32; 4]; MAX_GI_BULBS * 2],
}

/// How many GI bulbs the frame carries. Thirty-two holds a real GI string
/// whole — F-14 wires twenty-seven — with room for the flashers on top; the
/// selection takes the brightest by emitted flux, so a table with more loses
/// its dimmest, not its look.
pub const MAX_GI_BULBS: usize = 32;

impl GpuFrame {
    /// The matrix that flips the world through the reflecting plane.
    ///
    /// `SetPlaneReflection`, `matrix.h:324`: the identity with `2 n nᵀ` taken
    /// off it, and the translation `-2 d n`. The original writes it out by
    /// hand; this is the same thing.
    pub fn mirror_of_plane(&self) -> [[f32; 4]; 4] {
        let n = vpw_math::Vec3::new(self.mirror[0], self.mirror[1], self.mirror[2]).normalize();
        let d = 0.0;
        let mut m = [[0.0f32; 4]; 4];
        for row in 0..3 {
            for col in 0..3 {
                let a = [n.x, n.y, n.z];
                m[row][col] = f32::from(row == col) - 2.0 * a[row] * a[col];
            }
        }
        m[3] = [-2.0 * d * n.x, -2.0 * d * n.y, -2.0 * d * n.z, 1.0];
        m
    }

    /// The table's lighting data, which does not change between frames.
    pub fn from_lighting(l: &vpw_table::geometry::Lighting) -> Self {
        Self {
            view_proj: vpw_math::Mat4::IDENTITY.to_cols_array_2d(),
            eye: [0.0; 4],
            ambient: [l.ambient[0], l.ambient[1], l.ambient[2], l.range],
            light0: [l.lights[0].x, l.lights[0].y, l.lights[0].z, 0.0],
            light1: [l.lights[1].x, l.lights[1].y, l.lights[1].z, 0.0],
            emission: [l.emission[0], l.emission[1], l.emission[2], l.env_scale],
            // z once held the exposure; that moved to the post uniform with
            // the tone mapping, and the slot is dead. w counts the GI rows.
            env: [1.0, 0.0, 0.0, 0.0],
            screen: [0.0; 4],
            clip: [0.0; 4],
            // The playfield is the plane z = 0 and it faces up.
            mirror: [0.0, 0.0, 1.0, l.reflection_strength],
            gi_bounce: [0.0; 4],
            gi_levels: [0.0; 4],
            field: [0.0; 4],
            gi: [[0.0; 4]; MAX_GI_BULBS * 2],
        }
    }
}

/// A range of indices drawn with a single state.
#[derive(Debug, Clone)]
pub struct Batch {
    pub first_index: u32,
    pub index_count: u32,
    /// Index into `GpuScene::bind_groups`.
    pub binding: usize,
    pub transparent: bool,
    /// Representative depth, for sorting.
    pub depth: f32,
    /// How many meshes were merged into this batch.
    pub merged: usize,
    /// Material and image of the batch, so it can be inspected from outside.
    pub material: String,
    pub image: String,
    /// Whether the texture really did resolve.
    pub textured: bool,
    /// Whether this is the machine's head rather than the table.
    pub backbox: bool,
    /// Whether this is the room the machine stands in rather than the machine.
    /// See [`vpw_table::geometry::Mesh::scenery`].
    pub scenery: bool,
    /// Whether the batch draws with back faces culled.
    ///
    /// The original's scene default is `CULL_CCW` (`Renderer.cpp:927`) and a
    /// primitive drawn opaque keeps it (`primitive.cpp:1132`); ramps, walls
    /// and rubbers set `CULL_NONE` because they are thin-walled and their
    /// inside *is* their far side. Primitives are most of a modern table's
    /// triangles, so this is also most of a modern table's overdraw.
    pub culled: bool,
}

/// Counts of what building the scene cost. They are there to measure, which is
/// the whole point of the port.
#[derive(Debug, Clone, Copy, Default)]
pub struct SceneStats {
    pub meshes: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub batches: usize,
    pub textures: usize,
    /// How many draw calls were left after merging.
    pub draw_calls: usize,
    /// How many the original would have made: one per visible mesh.
    pub draw_calls_naive: usize,
}

/// The whole table, ready to draw.
pub struct GpuScene {
    pub vertices: wgpu::Buffer,
    /// Textures whose pixels change while the table runs, by image name. See
    /// [`vpw_table::geometry::Image::redrawn`].
    pub redrawn: HashMap<String, wgpu::Texture>,
    pub indices: wgpu::Buffer,
    pub batches: Vec<Batch>,
    pub bind_groups: Vec<wgpu::BindGroup>,
    pub stats: SceneStats,
    pub bounds: (Vec3, Vec3),
    /// The playfield as the lightmap's frame wants it:
    /// `[min.x, min.y, 1/width, 1/height]`. See `GpuFrame::field`.
    pub field: [f32; 4],
    /// The playfield's picture, for the ball to reflect. The white fallback
    /// when the table paints its floor with a material alone.
    pub field_picture: Option<wgpu::TextureView>,
    /// A copy of the table's lighting, so we do not have to drag the CPU scene
    /// all the way to drawing time.
    pub lighting: vpw_table::geometry::Lighting,
}

/// The key that decides whether two meshes can share a draw call.
#[derive(PartialEq, Eq, Hash, Clone)]
struct BatchKey {
    // (fields below; `playfield` keeps the floor in a batch of its own, which
    // is what lets its material carry the baked-GI flag.)
    material: String,
    image: String,
    transparent: bool,
    /// Kept in the key so the head never merges into a batch with the table:
    /// a view that leaves it out has to be able to leave it out on its own.
    backbox: bool,
    scenery: bool,
    playfield: bool,
    /// See [`Batch::culled`]; culled and two-sided meshes cannot share a
    /// draw call.
    culled: bool,
    /// Whether this batch's texture stops at its edges. See
    /// [`clamped_sampler`]; two parts that disagree cannot share a draw.
    clamp: bool,
}

impl GpuScene {
    /// Uploads a scene. `layout` has to be group 1 of the pipeline.
    pub fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        scene: &Scene,
    ) -> Self {
        // An additive layer is drawn by the dynamic path, which is the only
        // one with somewhere to put a per-frame intensity. See
        // [`crate::dynamic::DynamicParts`].
        let visible: Vec<&Mesh> = scene
            .meshes
            .iter()
            .filter(|m| m.visible && m.additive.is_none())
            .collect();
        let mut redrawn: HashMap<String, wgpu::Texture> = HashMap::new();
        // One copy of each picture across every batch. See [`TextureCache`].
        let mut textures_on_card = TextureCache::new();

        // Group by material+texture before touching the GPU. Meshes that share
        // state end up contiguous in the index buffer, which is what lets us
        // draw them in one go.
        let mut groups: HashMap<BatchKey, Vec<&Mesh>> = HashMap::new();
        for m in &visible {
            let material = scene.material(&m.material);
            // The original sends the part to the blended pass if the material
            // asks for it **or** if the texture carries an alpha channel
            // (`Shader.cpp:850`).
            let with_alpha = scene.image(&m.image).is_some_and(|i| i.has_alpha);
            let transparent = material.is_some_and(|mat| mat.is_transparent(with_alpha));
            let key = BatchKey {
                material: m.material.clone(),
                image: m.image.clone(),
                transparent,
                backbox: matches!(m.kind, vpw_table::geometry::MeshKind::Backbox),
                scenery: m.scenery,
                playfield: matches!(m.kind, vpw_table::geometry::MeshKind::Playfield),
                // Opaque primitives take the original's `CULL_CCW`; anything
                // transparent draws without depth writes there and goes
                // `CULL_NONE` with it (`primitive.cpp:1132`).
                culled: !transparent && matches!(m.kind, vpw_table::geometry::MeshKind::Primitive),
                // Two parts with the same material and texture still cannot
                // share a draw if one of them wants its image to stop at the
                // edge and the other wants it to tile.
                clamp: m.clamp,
            };
            groups.entry(key).or_default().push(m);
        }

        let mut vertices: Vec<GpuVertex> = Vec::with_capacity(scene.total_vertices());
        let mut indices: Vec<u32> = Vec::with_capacity(scene.total_triangles() * 3);
        let mut batches = Vec::new();
        let mut bind_groups = Vec::new();
        let mut field_picture = None;
        let mut textures = 0usize;
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));

        let sampler = table_sampler(device);
        let clamped = clamped_sampler(device);
        let white = white_texture(device, queue);

        // Stable order: the HashMap does not guarantee it and we want two loads
        // of the same table to produce exactly the same buffers.
        let mut keys: Vec<BatchKey> = groups.keys().cloned().collect();
        keys.sort_by(|a, b| {
            (a.transparent, &a.material, &a.image).cmp(&(b.transparent, &b.material, &b.image))
        });

        for key in keys {
            let meshes = &groups[&key];
            let first_index = indices.len() as u32;
            let mut sum = Vec3::ZERO;
            let mut count = 0.0f32;

            for m in meshes {
                let base = vertices.len() as u32;
                // Here is where the baking happens: exactly once, at load time.
                for v in m.baked() {
                    let p = Vec3::from_array(v.pos);
                    min = min.min(p);
                    max = max.max(p);
                    sum += p;
                    count += 1.0;
                    vertices.push(GpuVertex {
                        pos: v.pos,
                        normal: v.normal,
                        uv: v.uv,
                    });
                }
                indices.extend(m.indices.iter().map(|i| i + base));
            }

            let index_count = indices.len() as u32 - first_index;
            if index_count == 0 {
                continue;
            }

            let slot = material_slot_cached(
                device,
                queue,
                layout,
                if key.clamp { &clamped } else { &sampler },
                &white,
                scene.material(&key.material),
                scene.image(&key.image),
                key.playfield,
                &mut textures_on_card,
            );
            if slot.textured {
                textures += 1;
            }
            if key.playfield && slot.textured {
                field_picture = Some(slot.view.clone());
            }
            // The one image whose pixels change while the table runs. Kept by
            // name, because the renderer asks for it by name too and there is
            // no other way in: the batches are grouped and reordered on the way
            // to the GPU, so an index into them means nothing afterwards.
            if let Some(tex) = slot.redrawn {
                redrawn.insert(key.image.clone(), tex);
            }
            bind_groups.push(slot.bind_group);

            batches.push(Batch {
                first_index,
                index_count,
                binding: bind_groups.len() - 1,
                transparent: key.transparent,
                depth: if count > 0.0 { sum.y / count } else { 0.0 },
                merged: meshes.len(),
                material: key.material.clone(),
                image: key.image.clone(),
                textured: slot.textured,
                backbox: key.backbox,
                scenery: key.scenery,
                culled: key.culled,
            });
        }

        // Opaque first, front to back (big y = close to the player), and
        // transparent at the end, back to front.
        batches.sort_by(|a, b| match (a.transparent, b.transparent) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            (false, false) => b.depth.total_cmp(&a.depth),
            (true, true) => a.depth.total_cmp(&b.depth),
        });

        // Order the triangles for the GPU's caches — see `crate::meshopt`.
        // Opaque batches only: in a transparent batch the triangle order is
        // the blending order and belongs to the picture. The vertex remap
        // then follows the final order of everything.
        for b in &batches {
            if !b.transparent {
                let range = b.first_index as usize..(b.first_index + b.index_count) as usize;
                crate::meshopt::optimize_order(&mut indices[range]);
            }
        }
        crate::meshopt::remap_by_first_use(&mut vertices, &mut indices);

        let stats = SceneStats {
            meshes: visible.len(),
            vertices: vertices.len(),
            triangles: indices.len() / 3,
            batches: batches.len(),
            textures,
            draw_calls: batches.len(),
            draw_calls_naive: visible.len(),
        };

        Self {
            redrawn,
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vpw-vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vpw-indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            batches,
            bind_groups,
            stats,
            bounds: (min, max),
            field: {
                let b = &scene.playfield;
                let (dx, dy) = (b.max.x - b.min.x, b.max.y - b.min.y);
                [b.min.x, b.min.y, 1.0 / dx.max(1.0), 1.0 / dy.max(1.0)]
            },
            field_picture,
            lighting: scene.lighting,
        }
    }

    /// Emits the draws. A single buffer bind for the whole table.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_filtered(pass, |_| true);
    }

    /// The same, but drawing only the batches that pass the filter. Useful for
    /// working out who is covering what.
    pub fn draw_filtered(&self, pass: &mut wgpu::RenderPass<'_>, filter: impl Fn(&Batch) -> bool) {
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        let mut last = usize::MAX;
        for b in self.batches.iter().filter(|b| filter(b)) {
            if b.binding != last {
                pass.set_bind_group(1, &self.bind_groups[b.binding], &[]);
                last = b.binding;
            }
            pass.draw_indexed(b.first_index..b.first_index + b.index_count, 0, 0..1);
        }
    }
}

/// A resolved group-1 binding: the material block plus its texture.
pub struct MaterialSlot {
    pub bind_group: wgpu::BindGroup,
    /// The texture view the slot bound — the fallback when nothing resolved.
    /// A handle, so keeping it costs nothing; the ball's shader wants the
    /// playfield's to reflect.
    pub view: wgpu::TextureView,
    /// Whether the texture really did resolve, or we fell back to white.
    pub textured: bool,
    /// The texture itself, kept only when the image says its pixels change
    /// while the table runs. Everything else is uploaded once and never looked
    /// at again, and holding on to it would be holding a table's worth of
    /// textures for nothing.
    pub redrawn: Option<wgpu::Texture>,
    /// The uniform the bind group points at. Kept so a caller whose material
    /// changes while the table runs can rewrite it in place rather than
    /// rebuild the group — which is every additive layer, sixty times a
    /// second. See [`crate::dynamic::DynamicParts::relight`].
    pub uniform: wgpu::Buffer,
}

/// The sampler most of the table shares.
///
/// Repeat on both axes because that is what table textures expect: a playfield
/// image is authored to tile, and clamping it leaves a smeared border along
/// every edge. The parts that want the other answer say so — see
/// [`clamped_sampler`].
pub fn table_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    sampler(device, wgpu::AddressMode::Repeat)
}

/// The sampler for a part whose image stops at its own edges.
///
/// The original chooses per part rather than once for the scene, and on a ramp
/// it reads that part's image alignment to do it (`ramp.cpp:895`): an image
/// wrapped *along* the ramp clamps, and one tiled by world coordinates
/// repeats. It is not a nicety. The Sopranos' apron is a two-triangle ramp
/// with the apron artwork printed on it; repeating that put a second apron,
/// mirrored, across the cabinet beside the real one.
pub fn clamped_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    sampler(device, wgpu::AddressMode::ClampToEdge)
}

fn sampler(device: &wgpu::Device, mode: wgpu::AddressMode) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("vpw-sampler"),
        address_mode_u: mode,
        address_mode_v: mode,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    })
}

/// Turns a material and an image into the bind group the shader wants.
///
/// Both paths go through here — the baked static scene and the pieces that
/// move — and that is the point: a flipper has to shade exactly like the wall
/// next to it. Resolving a material is fiddly enough (the `opacity_active`
/// flag, the 0.08 on the specular, falling back to white when the texture is
/// missing) that two copies of it would drift apart within a month.
///
/// `fallback` is the 1x1 white texture: the shader always samples, so there has
/// to be something bound even when the piece has no image.
#[allow(clippy::too_many_arguments)]
/// Textures already on the card, by image name.
///
/// A table names the same picture from a great many parts, and a baked one
/// names the same *atlas* from nearly all of them. Without this, every part
/// that mentions `VLM.Nestmap1` uploads its own copy of a four-thousand-pixel
/// square: on Circus that was two hundred uploads of sixty-seven megabytes
/// each, which is nine and a half seconds of the ten the table took to load,
/// and a card full of the same picture.
pub type TextureCache = std::collections::HashMap<String, (wgpu::Texture, wgpu::TextureView)>;

/// [`material_slot`] with a fresh cache, for a caller that builds only one.
#[allow(clippy::too_many_arguments)]
pub fn material_slot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    fallback: &wgpu::TextureView,
    material: Option<&vpw_table::geometry::Material>,
    image: Option<&vpw_table::geometry::Image>,
    playfield: bool,
) -> MaterialSlot {
    let mut cache = TextureCache::new();
    material_slot_cached(
        device, queue, layout, sampler, fallback, material, image, playfield, &mut cache,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn material_slot_cached(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    fallback: &wgpu::TextureView,
    material: Option<&vpw_table::geometry::Material>,
    image: Option<&vpw_table::geometry::Image>,
    playfield: bool,
    cache: &mut TextureCache,
) -> MaterialSlot {
    // A picture whose pixels change while the table runs is not shared: its
    // owner rewrites it, and two parts rewriting one texture is a different
    // decision from this one.
    let uploaded = image.and_then(|i| {
        if i.redrawn {
            return upload_texture(device, queue, i);
        }
        if let Some(hit) = cache.get(&i.name) {
            return Some(hit.clone());
        }
        let made = upload_texture(device, queue, i)?;
        cache.insert(i.name.clone(), made.clone());
        Some(made)
    });
    let redrawn = match (image, uploaded.as_ref()) {
        (Some(i), Some((tex, _))) if i.redrawn => Some(tex.clone()),
        _ => None,
    };
    let (view, textured) = match uploaded {
        Some((_, v)) => (v, true),
        None => (fallback.clone(), false),
    };

    let inputs = material
        .map(vpw_table::geometry::Material::shader_inputs)
        .unwrap_or_default();
    // The alpha test belongs to the *image*, not to the material: it is what
    // the table's author set on that picture, and it is how a cut-out piece of
    // artwork gets its background thrown away rather than drawn. Only where
    // there really is a texture with an alpha channel to test — the original's
    // `pin->m_alphaTestValue >= 0.f && !pin->IsOpaque()` (`ramp.cpp:907`).
    let alpha_test = match image {
        Some(i) if textured && i.has_alpha => i.alpha_test,
        _ => -1.0,
    };
    let mut data = GpuMaterial::from_inputs(&inputs, textured, alpha_test);
    // The machine's display is a light, not a lit thing: a plasma panel makes
    // its own photons, and putting it through the light loop draws it at
    // whatever the room happens to be — on a table authored dark, invisible.
    // The original never faces this because its desktop backglass is a
    // backdrop drawn outside the scene's lighting altogether; this port's head
    // is in the scene, so the panel carries an emissive flag instead.
    if image.is_some_and(|i| i.name == vpw_table::backbox::DISPLAY_IMAGE) {
        data.extra[3] = 1.0;
    }
    // And the artwork around it is a light too, for the same reason and to a
    // gentler degree: a backglass is a translucent sheet with tubes behind it
    // (see `vpw_table::backglass`), so it glows in a dark room instead of
    // going out with it. 3.0 rather than 1.0 because the display is a plasma
    // panel and this is a lit picture — the same branch at half the gain
    // would make the head brighter than the score on it.
    if image.is_some_and(|i| i.name == vpw_table::backglass::BACKGLASS_IMAGE) {
        data.extra[3] = 3.0;
    }
    // The playfield receives the baked GI: its UVs span the field, which is
    // the lightmap's space. 2.0 in the emissive slot, because the two flags
    // are mutually exclusive and the uniform has no room left over.
    if playfield {
        data.extra[3] = 2.0;
    }
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("vpw-material"),
        contents: bytemuck::bytes_of(&data),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    MaterialSlot {
        uniform: uniform.clone(),
        view: view.clone(),
        bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vpw-material-bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        }),
        textured,
        redrawn,
    }
}

pub fn white_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let tex = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("vpw-white"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &[255, 255, 255, 255],
    );
    tex.create_view(&wgpu::TextureViewDescriptor::default())
}

/// The material of a layer that is added rather than lit.
///
/// Nothing about a surface applies: it is not rough, it does not reflect, it
/// has no clearcoat. All the shader needs is the colour to multiply the texture
/// by — already carrying the intensity, as the original does
/// (`primitive.cpp:1170`) — and the flag that sends it down the unshaded
/// branch.
pub fn additive_material(color: [f32; 3], alpha: f32) -> GpuMaterial {
    let mut data = GpuMaterial::from_inputs(&Default::default(), true, -1.0);
    data.base_color = [color[0] * alpha, color[1] * alpha, color[2] * alpha, alpha];
    data.extra[3] = 4.0;
    data
}

pub(crate) fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &vpw_table::geometry::Image,
) -> Option<(wgpu::Texture, wgpu::TextureView)> {
    // The ones that came in as raw BMP are already RGBA and nothing has to be
    // decoded; the rest go through the PNG/JPG decoder.
    let (w, h, pixels) = match (&image.rgba, &image.encoded) {
        (Some(rgba), _) => (
            image.width,
            image.height,
            std::borrow::Cow::Borrowed(rgba.as_slice()),
        ),
        (None, Some(bytes)) => {
            let img = image::load_from_memory(bytes).ok()?.to_rgba8();
            let (w, h) = img.dimensions();
            (w, h, std::borrow::Cow::Owned(img.into_raw()))
        }
        (None, None) => return None,
    };
    if w == 0 || h == 0 || pixels.len() < (w as usize * h as usize * 4) {
        return None;
    }
    let tex = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("vpw-texture"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            // An image whose pixels change while the table runs needs a
            // texture the renderer can write to. Almost none do; the score
            // display is the one that does.
            usage: if image.redrawn {
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST
            } else {
                wgpu::TextureUsages::TEXTURE_BINDING
            },
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &pixels,
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    Some((tex, view))
}
