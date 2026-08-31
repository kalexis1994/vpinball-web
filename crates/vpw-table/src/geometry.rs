//! Table geometry: pulling something drawable out of the `.vpx`.
//!
//! # Visual Pinball's coordinate system
//!
//! The playfield is the `z = 0` plane, with the normal pointing at `+Z`
//! (`pintable.cpp:3242-3257`). `x` grows to the right and `y` grows
//! **downwards**, that is, towards the player: the top of the table is small
//! `y`. Units are VPU (Visual Pinball units); a typical table measures about
//! 950 x 2150.
//!
//! # A trap when porting the matrices
//!
//! Visual Pinball uses the **row-vector** convention (`v * M`), so its products
//! read left to right in the order they are applied. `glam` uses column vectors
//! (`M * v`). When porting you have to **reverse the order of the products**,
//! not transcribe them.

use vpw_math::{Mat4, Vec3};

/// A vertex exactly as the `.vpx` stores it: position, normal and texture
/// coordinate. That is 32 bytes, the same layout as the original's
/// `Vertex3D_NoTex2`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// A mesh in local space, along with the transform that takes it to the world.
///
/// The transform is kept apart instead of being applied right away: the pieces
/// the script animates need it alive, and for the static ones the renderer
/// decides whether it is worth baking (see [`Mesh::baked`]).
#[derive(Debug, Clone)]
pub struct Mesh {
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub transform: Mat4,
    /// Texture name, if it has one. Resolved against `Scene::images`.
    pub image: String,
    /// Material name, if it has one. Resolved against `Scene::materials`.
    pub material: String,
    pub visible: bool,
    /// Whether this part is the room the table stands in rather than the
    /// machine itself.
    ///
    /// Set only on a table that models one — a cabinet, a backdrop, a whole
    /// room — and only for the parts of it that stand up at head height. It is
    /// scenery built for one camera: the author's cabinet view, from inside
    /// it. From any view that looks *at* the machine you are outside it, and
    /// what you see is the underside of its lid hanging over the playfield and
    /// across the backglass.
    ///
    /// So the views that look at the machine leave it out, and the one that
    /// stands you inside it draws it. That is the same arrangement the head
    /// itself already has, and for the same reason: what belongs in a picture
    /// depends on where the picture is taken from.
    pub scenery: bool,
    /// Whether the texture stops at its edges instead of tiling.
    ///
    /// The original chooses this per part rather than once for the scene, and
    /// on a ramp it reads the part's own image alignment to do it
    /// (`ramp.cpp:895`): a ramp whose image is *wrapped along* it clamps, and
    /// one whose image is tiled by world coordinates repeats. The difference
    /// is a ramp's artwork spilling out past its own edges — The Sopranos'
    /// apron is a two-triangle ramp with the apron printed on it, and
    /// repeating it laid a second, mirrored apron across the cabinet beside
    /// the real one.
    pub clamp: bool,
    pub kind: MeshKind,
    /// Set when the part is drawn by **adding** it to what is already there
    /// rather than by lighting it. See [`Additive`].
    pub additive: Option<Additive>,
    /// Which of two coplanar transparent parts is drawn **over** the other:
    /// the primitive's **depth bias**.
    ///
    /// Not a depth-buffer offset, whatever the name suggests. The original
    /// uses it as a *sort key* for the transparent pass — the draws are
    /// ordered by `depthBias - center.z` (`RenderDevice.cpp:2708`) — and it is
    /// how a table says "this goes on top" about two surfaces at the same
    /// height. More negative means later, which means over.
    ///
    /// A baked table leans on it entirely: its overlay and its plastics are
    /// coplanar with the playfield by construction, so `BM_Overlay` at −1 and
    /// `BM_plastics1` at −100 are the only thing keeping them in order.
    pub depth_bias: f32,
    /// How much of the scene's light this part refuses, 0..1 — the primitive's
    /// **BlendDisableLighting** (`m_disableLightingTop`).
    ///
    /// At 1 the surface is drawn as its own texture and the light loop is
    /// skipped: the original lerps the lit result towards the raw diffuse by
    /// this (`Material.fxh:144`). A table sets it on anything whose lighting
    /// is already painted into the picture — a lit backglass, an insert's
    /// "on" artwork, and every mesh of a baked table, whose whole point is
    /// that the light is in the texture. Lighting those a second time is how
    /// a baked table comes out white.
    pub disable_lighting: f32,
}

/// A part that is light rather than a thing: it is *added* to the picture,
/// unlit, and writes no depth (`primitive.cpp:1166`).
///
/// A `.vpx` marks it with the primitive's **Additive Blend** flag, which is a
/// switch any table may throw and not a convention of any one tool. What throws
/// it in bulk is a bake: the Virtual Lighting Mod takes a table into Blender,
/// bakes it, and hands back the machine as a few `BM_*` meshes plus one `LM_*`
/// copy of the geometry **per lamp**, holding that lamp's light and nothing
/// else. Circus is ninety-six of them.
///
/// Drawing one as ordinary geometry is not a small error: a lightmap is black
/// everywhere its lamp does not reach, so it buries what it was meant to
/// brighten.
#[derive(Debug, Clone, PartialEq)]
pub struct Additive {
    /// The colour the texture is multiplied by, from the primitive's `Color`.
    ///
    /// A layer whose colour is black adds nothing and the original skips it
    /// outright (`primitive.cpp:1088`).
    pub color: [f32; 3],
    /// How much of it to add, 0..1, from the primitive's `Alpha`.
    pub alpha: f32,
    /// The lamp this layer belongs to, if it belongs to one.
    ///
    /// This is what makes a bake *animate*: the layer is added in proportion
    /// to how bright that lamp is right now against how bright it is at full
    /// power (`primitive.cpp:1078-1085`). Without it a bake is a photograph
    /// with every lamp lit at once.
    pub light: Option<String>,
}

/// Whether a primitive is one of a baked table's lightmaps.
///
/// Asked by what reads a table's *shape* rather than what draws it: a lightmap
/// is a copy of geometry that is already there, holding one lamp's light, so
/// it has no place in the collision, the bounds or a bake of our own. It
/// counts the table twice.
///
/// What draws them asks [`Mesh::additive`] instead — they are drawn, and the
/// file's own Additive Blend flag is what says so.
pub fn is_lightmap(name: &str, image: &str) -> bool {
    name.starts_with("LM_") && image.starts_with("VLM.")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshKind {
    /// The playfield plane, generated by us.
    Playfield,
    /// A primitive with its own mesh stored in the file.
    Primitive,
    /// A wall, generated from its control points.
    Wall,
    /// A rubber: a tube extruded along its path.
    Rubber,
    /// A ramp, either flat or wire.
    Ramp,
    /// One of the original's builtin meshes: bumper, target, gate, spinner.
    Builtin,
    /// The head of the machine, standing behind the playfield.
    ///
    /// Not in the file and not one of the table's own parts: it is built from
    /// the cabinet's proportions so a camera has something true to frame and
    /// the score has somewhere to sit. See [`crate::backbox`].
    Backbox,
}

impl Mesh {
    /// Triangle count.
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// Returns the vertices already taken to world space.
    ///
    /// Normals go through the inverse transpose, which is the only correct
    /// thing to do when the scale is non-uniform — and on tables it almost
    /// never is.
    pub fn baked(&self) -> Vec<Vertex> {
        let normal_matrix = self.transform.inverse().transpose();
        self.vertices
            .iter()
            .map(|v| {
                let p = self.transform.transform_point3(Vec3::from_array(v.pos));
                let n = (normal_matrix * Vec3::from_array(v.normal).extend(0.0))
                    .truncate()
                    .normalize_or_zero();
                Vertex {
                    pos: p.to_array(),
                    normal: n.to_array(),
                    uv: v.uv,
                }
            })
            .collect()
    }

    /// Bounding box in world space.
    pub fn bounds(&self) -> Option<Bounds> {
        let mut it = self
            .vertices
            .iter()
            .map(|v| self.transform.transform_point3(Vec3::from_array(v.pos)));
        let first = it.next()?;
        let (mut min, mut max) = (first, first);
        for p in it {
            min = min.min(p);
            max = max.max(p);
        }
        Some(Bounds { min, max })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }
}

/// A material, with the same fields `Shader::SetMaterial` consumes
/// (`renderer/Shader.cpp:790`). The values are stored **as they come**: the
/// conversion into what the shader expects happens in a single place
/// ([`Material::shader_inputs`]), which is the port of that function.
#[derive(Debug, Clone)]
pub struct Material {
    pub name: String,
    /// Diffuse color, linear, 0..1.
    pub base_color: [f32; 3],
    /// Color of the specular layer.
    pub glossy_color: [f32; 3],
    /// Color of the clearcoat layer (the second layer, on top of the specular).
    pub clearcoat_color: [f32; 3],
    pub is_metal: bool,
    /// Roughness **with Visual Pinball's semantics**: 0 is matte and 1 is
    /// glossy, the other way around from standard PBR. It is stored as it
    /// comes; the exponent is computed in `shader_inputs`.
    pub roughness: f32,
    /// How much the light wraps around towards the dark side (0..1).
    pub wrap_lighting: f32,
    /// How much the texture tints the specular layer (0 = not at all, 1 =
    /// completely).
    pub glossy_image_lerp: f32,
    /// Reflectance at the edges, for the Fresnel term.
    pub edge: f32,
    /// How much the edge of a translucent piece becomes opaque.
    pub edge_alpha: f32,
    /// Apparent thickness, for geometric opacity.
    pub thickness: f32,
    pub opacity: f32,
    /// Whether opacity is actually applied.
    ///
    /// This is a detail that is expensive to ignore: many materials store
    /// `opacity = 0` with the flag **off**, and there the opacity is not used.
    /// The original resolves it in one line (`Shader.cpp:829`):
    /// `const float alpha = bOpacityActive ? fOpacity : 1.0f;`. Taking the
    /// opacity without looking at the flag, F-14's playfield comes out with
    /// zero alpha: it is not drawn white, it is drawn **transparent**.
    pub opacity_active: bool,
}

/// What actually goes into the shader, already resolved.
///
/// Port of `Shader::SetMaterial` (`renderer/Shader.cpp:790-855`) plus the first
/// few lines of `ps_main` / `ps_main_texture` (`BasicShader.hlsl:320-323` and
/// `371-374`).
#[derive(Debug, Clone, Copy)]
pub struct ShaderInputs {
    pub base_color: [f32; 3],
    pub alpha: f32,
    /// Specular color, **without** the 0.08 factor: the shader applies that one
    /// because on textured pieces it depends on the texel.
    pub glossy_color: [f32; 3],
    pub glossy_image_lerp: f32,
    /// Clearcoat layer, already multiplied by 0.08.
    pub clearcoat: [f32; 3],
    /// Specular exponent, already mapped from 0..1 to 2..2048.
    pub glossy_power: f32,
    pub wrap_lighting: f32,
    pub edge: f32,
    pub edge_alpha: f32,
    pub thickness: f32,
    pub is_metal: bool,
}

impl Default for ShaderInputs {
    /// The values the original uses when a piece has **no material**
    /// (`Shader.cpp:812-826`). Watch out for two of them: the specular and the
    /// clearcoat start out **black**, not some arbitrary gray, and the image
    /// lerp starts at one.
    fn default() -> Self {
        Self {
            base_color: [0.5, 0.5, 0.5],
            alpha: 1.0,
            glossy_color: [0.0, 0.0, 0.0],
            glossy_image_lerp: 1.0,
            clearcoat: [0.0, 0.0, 0.0],
            glossy_power: 2.0, // exp2(10*0 + 1)
            wrap_lighting: 0.0,
            edge: 1.0,
            edge_alpha: 1.0,
            thickness: 0.05,
            is_metal: false,
        }
    }
}

impl Material {
    /// The alpha that goes into the shader (`Shader.cpp:829`).
    pub fn alpha(&self) -> f32 {
        if self.opacity_active {
            self.opacity
        } else {
            1.0
        }
    }

    /// Whether it has to be drawn in the blended pass (`Shader.cpp:850`).
    ///
    /// `texture_has_alpha` is the original's `has_alpha`: a texture with an
    /// alpha channel is enough to send the piece to the transparent pass even
    /// if the material's opacity is one.
    pub fn is_transparent(&self, texture_has_alpha: bool) -> bool {
        self.opacity_active && (texture_has_alpha || self.alpha() < 0.999)
    }

    /// Resolves the material into what the shader consumes.
    pub fn shader_inputs(&self) -> ShaderInputs {
        ShaderInputs {
            base_color: self.base_color,
            alpha: self.alpha(),
            // On metal the specular comes from the base color, so the field of
            // its own is sent in black (`Shader.cpp:833`).
            glossy_color: if self.is_metal {
                [0.0; 3]
            } else {
                self.glossy_color
            },
            glossy_image_lerp: self.glossy_image_lerp,
            clearcoat: self.clearcoat_color.map(|c| c * 0.08),
            // From 0..1 to 2..2048 (`Shader.cpp:799`).
            glossy_power: (10.0 * self.roughness + 1.0).exp2(),
            wrap_lighting: self.wrap_lighting,
            // Metal does not attenuate by Fresnel at the edge
            // (`BasicShader.hlsl:323`).
            edge: if self.is_metal { 1.0 } else { self.edge },
            edge_alpha: self.edge_alpha,
            thickness: self.thickness,
            is_metal: self.is_metal,
        }
    }
}

/// A table texture.
///
/// The `.vpx` stores images in two ways: most of them as PNG or JPG as-is, and
/// some — the ones that were BMPs back in the day — as raw pixels compressed
/// with LZW. The latter arrive here already decoded.
#[derive(Debug, Clone)]
pub struct Image {
    pub name: String,
    /// PNG/JPG bytes, if it came in a known compressed format.
    pub encoded: Option<Vec<u8>>,
    /// Ready-to-use RGBA pixels, if it came as a raw BMP.
    pub rgba: Option<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// Whether the source format can carry an alpha channel. Together with the
    /// material, this decides whether the piece goes to the blended pass
    /// (`Shader.cpp:850`).
    pub has_alpha: bool,
    /// Below what alpha a texel is thrown away instead of drawn, or negative
    /// where the table did not ask for it.
    ///
    /// This is **not** transparency, and it is not decided by the material. A
    /// sword cut out of its background is a texture with an alpha channel on a
    /// piece whose material is plain opaque plastic, and the original draws it
    /// in the opaque pass with the see-through texels *discarded*:
    ///
    /// ```hlsl
    /// clip(pixel.a <= alphaTestValue ? -1 : 1);
    /// ```
    ///
    /// Without that, the cut-out background is drawn as whatever colour it
    /// happens to carry, and a piece of artwork comes out as a flat rectangle
    /// — which looks exactly like a texture that failed to load.
    ///
    /// The table stores it out of 255 and the shader wants it out of one, so it
    /// is scaled on the way in, the same as `Texture.cpp:937`.
    pub alpha_test: f32,
    /// Whether its pixels change while the table runs.
    ///
    /// Almost nothing a table carries does: an image comes out of the file
    /// once and is the same for the whole session. The machine's score display
    /// is the exception — it is redrawn every time the segments change — and
    /// saying so here is what gets it a texture the renderer is allowed to
    /// write to. See [`crate::backbox::DISPLAY_IMAGE`].
    pub redrawn: bool,
}

impl Image {
    /// Whether the image has pixels in either of the two forms.
    pub fn has_data(&self) -> bool {
        self.encoded.is_some() || self.rgba.is_some()
    }
}

/// The table's scene lights.
///
/// Visual Pinball does not light with the script's lights: for the material it
/// uses **two fixed point lights** whose position derives from the table size
/// (`Renderer.cpp:1055-1060`), plus an ambient term and an environment map. The
/// `.vpx` lights are something else — those are the game's lamps, and they go
/// in a separate pass.
#[derive(Debug, Clone, Copy)]
pub struct Lighting {
    /// The two lights, in VPU.
    pub lights: [Vec3; 2],
    /// Emission color of the lights, already scaled by `light_emission_scale`.
    pub emission: [f32; 3],
    /// Ambient color.
    pub ambient: [f32; 3],
    /// Range of the lights, for the attenuation.
    pub range: f32,
    /// How much the environment map contributes.
    pub env_scale: f32,
    /// The table's own day/night — `m_globalEmissionScale`, already multiplied
    /// into the three terms above. Kept on its own as well so a player-side
    /// day/night override (`Renderer.cpp:377`, `Mode::User`) can divide it
    /// back out: the original's user mode *replaces* this value, and replacing
    /// a factor that is already baked in means knowing what it was.
    pub global: f32,
    /// Scene exposure, which feeds into tone mapping.
    pub exposure: f32,
    /// How strongly the bloom is added back, from the table's own field.
    ///
    /// Worth reading rather than assuming, and the assumption was expensive:
    /// the default is 1.8 and a table that wants its lights crisp sets its own,
    /// so a renderer that hardcodes the default turns every lit insert on such
    /// a table into a soft blown-out blob. A modern table's lamps run to an
    /// intensity of two hundred, which is bright by design and is meant to be
    /// bright *there*, not smeared over its neighbours.
    pub bloom_strength: f32,
    /// How strongly the playfield mirrors what stands on it.
    ///
    /// Visual Pinball starts every part's own strength from this table-wide
    /// number and lets a part override it (`gamedata.rs:834`). We use the one
    /// number for the whole table, which the shader then applies only to
    /// surfaces facing the probe — so it reaches the playfield and the flat
    /// tops of things standing on it, and not the walls.
    ///
    /// The table also carries `ReflectElementsOnPlayfield`, and it is tempting
    /// to read that as an on-off switch. It is not one any more: it is a tag
    /// from before 10.8, when reflection was a single hardcoded pass, and
    /// modern Visual Pinball ignores it — its script property "logs a
    /// deprecation error and always returns true". F-14 has it set to false
    /// and a strength of a quarter, so honouring it would turn the reflections
    /// off on exactly the tables old enough to have an opinion.
    pub reflection_strength: f32,
}

/// The camera the table's author set up, as the file stores it.
///
/// Every `.vpx` carries one per view mode; this is the desktop one, which is
/// the mode a player at a keyboard is in. Visual Pinball computes its camera
/// from these numbers and a fit, and a table's author tunes them until the
/// machine looks the way they want it to — so a port that invents its own
/// framing is not showing the table its author made.
///
/// Angles are degrees, offsets are VPU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuthoredView {
    /// How far above the playfield the eye sits. `BG_INCLINATION`.
    pub inclination: f32,
    /// Vertical field of view. `BG_FOV`.
    pub fov: f32,
    /// The backward skew legacy tables were authored with. `BG_LAYBACK`.
    ///
    /// Read and reported; not applied. It is a shear in the projection rather
    /// than a camera move, and the tables that use more than a degree of it
    /// are rare.
    pub layback: f32,
    /// Where the view is nudged to, from the fit. `BG_OFFSET_X/Y/Z`.
    pub offset: Vec3,
}

impl Default for AuthoredView {
    /// Visual Pinball's own defaults, for a file that stores none.
    fn default() -> Self {
        Self {
            inclination: 45.0,
            fov: 45.0,
            layback: 0.0,
            offset: Vec3::ZERO,
        }
    }
}

/// Everything drawable in a table.
#[derive(Debug, Clone)]
pub struct Scene {
    /// The camera its author set up for a desktop. See [`AuthoredView`].
    pub view: AuthoredView,
    /// And the one they set up for a cabinet: a long lens low down, looking
    /// along the table from the player's end.
    ///
    /// Worth carrying separately because it is a different picture and not a
    /// tweak of the same one — The Sopranos asks for a fifteen degree lens at
    /// twenty-two degrees with sixty of layback, against forty-five and
    /// fifty-two on the desktop — and because the tables that model a room or
    /// a cabinet around themselves model it *for this camera*. Standing in the
    /// desktop view, that scenery is in the way; standing here, you are inside
    /// it, which is what its author built.
    pub cabinet: AuthoredView,
    /// Whether the head standing behind the playfield is one we built.
    ///
    /// False when the table modelled its own — see `brings_its_own_head` — in
    /// which case there is no panel of ours for the score to sit on and the
    /// camera has nothing extra to frame. Everything downstream that assumed
    /// there is always a head of ours has to ask.
    pub built_head: bool,
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub images: Vec<Image>,
    /// Playfield extent in VPU.
    pub playfield: Bounds,
    /// Playfield texture, applied to the generated plane.
    pub playfield_image: String,
    pub playfield_material: String,
    /// The image the table lights itself with — `EIMG`, `pintable.cpp:2415`
    /// — resolved against `images` like any other texture name. Empty when
    /// the table never set one, which is when the renderer uses the map that
    /// ships with Visual Pinball (`Renderer.cpp:208-210`).
    pub env_image: String,
    /// The wear on the ball — `BLIF`, the decal every ball wears over its
    /// steel. Resolved against `images`; empty when the table never set one,
    /// which is when the renderer falls back to the scratches it makes
    /// itself ([`crate::ball::scratches`]), the same way the original falls
    /// back to the scuffed ball in its `Assets/`.
    pub ball_decal: String,
    pub lighting: Lighting,
    /// Every lamp the table has, lit or not.
    ///
    /// Which ones are on is a question for the script, and the answer changes
    /// several times a second — so the set is fixed here and the state is not.
    /// Keeping only the ones the file says are on would leave a playfield that
    /// can never light up: a table's lamps are almost all off in the file,
    /// because they are the game's lamps and the game turns them on.
    pub lights: Vec<crate::light::Light>,
    /// Every flasher, shown or not, for the same reason as the lamps: the
    /// game switches them, and a strobe saved off is the one about to fire.
    /// See [`crate::flasher`].
    pub flashers: Vec<crate::flasher::Flasher>,
    /// What the table asks the physics for. See [`TablePhysics`].
    pub physics: TablePhysics,
}

/// The numbers a table sets that decide how it plays.
///
/// Separate from the geometry because they are not geometry: they are three
/// floats that between them decide how fast the ball runs, and getting them
/// from the file rather than from a constant is the difference between a table
/// that plays like itself and one that plays like every other table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TablePhysics {
    /// The playfield's slope in degrees, already interpolated.
    ///
    /// A table gives a **range** and the difficulty picks a point in it:
    /// `slope = lerp(min, max, difficulty)` (`player.cpp:498`). Taking the
    /// maximum alone — which is what this did — puts a table whose author
    /// asked for six to eight and a half degrees permanently at eight and a
    /// half. The ball is too fast for the whole game and drains too easily,
    /// and nothing about it looks broken.
    pub slope_deg: f32,
    /// Gravity, in the original's units (`player.cpp:499`).
    pub gravity: f32,
    /// The scatter any part that does not set its own falls back to, in
    /// degrees. `c_hardScatter = ANGTORAD(m_defaultScatter)` (`player.cpp:197`).
    pub default_scatter_deg: f32,
    /// How hard the table is set to play, from 0 to 1. Parts scale their own
    /// scatter by it (`kicker.cpp:726`, `hitball.cpp:104`) and it is what
    /// picks the slope out of the range above.
    pub difficulty: f32,
}

impl Scene {
    pub fn total_vertices(&self) -> usize {
        self.meshes.iter().map(|m| m.vertices.len()).sum()
    }
    pub fn total_triangles(&self) -> usize {
        self.meshes.iter().map(Mesh::triangles).sum()
    }
    /// Names are resolved **case-insensitively**. That is not a liberty we
    /// took: the original uses a hash map with its own comparator and justifies
    /// it in `utils/hash.h:53`, "use case-insensitive compare because user can
    /// enter the names in lower case from the script".
    pub fn material(&self, name: &str) -> Option<&Material> {
        self.materials
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
    }
    /// Same for textures (`PinTable::GetImage`, `pintable.cpp:4232`). It is an
    /// easy detail to miss and it fails silently: F-14 declares
    /// `gamedata.image = "playfield"` and stores the image as `"Playfield"`, so
    /// comparing exactly the whole table is drawn white.
    pub fn image(&self, name: &str) -> Option<&Image> {
        self.images
            .iter()
            .find(|i| i.name.eq_ignore_ascii_case(name))
    }
    /// Removes from the static scene the meshes somebody else is going to draw.
    ///
    /// Moving pieces are drawn separately, with their own matrix. If they also
    /// stay in the baked scene, each one shows up **twice**: one nailed to its
    /// rest position and one following the physics. A ghost flipper at the
    /// starting angle is one of the most bewildering things a player can show.
    ///
    /// They go by name, which in a `.vpx` is unique per item: Visual Pinball's
    /// editor does not allow duplicates.
    pub fn remove(&mut self, names: &[String]) {
        self.meshes.retain(|m| !names.iter().any(|n| n == &m.name));
    }

    /// Corners of everything standing on the playfield, for a camera to frame.
    ///
    /// One box around the whole table is a bad question to ask a camera looking
    /// straight down: it claims something as tall as the tallest ramp stands in
    /// each corner of the playfield, and what stands there is the flat sheet.
    /// These are the boxes the meshes really occupy, so the camera can tell the
    /// difference.
    ///
    /// Strays are dropped. Tables carry parts far outside the playfield — a
    /// backglass, a DMD panel, a spare toy left at the origin — and a camera
    /// that frames those leaves the table small in a corner. The margin is
    /// generous enough for a wall sitting on the edge and mean enough to catch
    /// something in the next postcode.
    /// The corners the original's legacy camera is fitted to.
    ///
    /// Not "everything on the table", and that is the whole point. Visual
    /// Pinball builds this set from three kinds of part and no others
    /// (`ViewSetup.cpp:437`, and the `GetBoundingVertices` of each):
    ///
    /// * a **wall** contributes the *whole playfield rectangle* at its own top
    ///   and bottom heights — "hardwired to table dimensions" is the comment
    ///   in `surface.cpp:451`, and it is not a shortcut: it is what makes the
    ///   fit frame the table rather than the furniture;
    /// * a **ramp** and a **rubber** contribute their own boxes
    ///   (`ramp.cpp:293`, `rubber.cpp:305`);
    /// * a **primitive** contributes *nothing*. `primitive.cpp:622` says why
    ///   in as many words: the position was computed from a partial bounding
    ///   volume that would not include primitives, so it must never fill this
    ///   list.
    ///
    /// Everything else — bumpers, targets, gates, spinners, kickers — has no
    /// `GetBoundingVertices` at all and has never been framed on.
    ///
    /// So the height of this box is the tallest *wall, ramp or rubber*, which
    /// on any table is two or three hundred units. That is the rule that
    /// makes the front view robust without a single guess about what is
    /// scenery: a table that models a room around itself does it with
    /// primitives, and a camera that never framed on primitives never has to
    /// find that out. The Sopranos' room reaches 1215 units over the whole
    /// playfield and the original has simply never looked at it.
    /// The corners of everything the table actually contains, scenery and all.
    ///
    /// Not for framing — see [`Self::legacy_bounds`] for that, and for why a
    /// primitive must never be framed on. This is for the other question: a
    /// camera has to stand *outside* what exists, even the parts it does not
    /// frame on, or it ends up inside a room looking at the back of a wall.
    pub fn extent(&self) -> Vec<Vec3> {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        let mut any = false;
        // An additive layer is light and not shape: it is a copy of geometry
        // that is already here, and counting it would be counting the table
        // twice.
        for mesh in self
            .meshes
            .iter()
            .filter(|m| m.visible && m.additive.is_none())
        {
            if let Some(b) = mesh.bounds() {
                lo = lo.min(b.min);
                hi = hi.max(b.max);
                any = true;
            }
        }
        if any {
            crate::backbox::corners_of(lo, hi).to_vec()
        } else {
            Vec::new()
        }
    }

    pub fn legacy_bounds(&self) -> Vec<Vec3> {
        let pf = self.playfield;
        let mut out = Vec::new();
        for mesh in &self.meshes {
            if !mesh.visible {
                continue;
            }
            let Some(b) = mesh.bounds() else { continue };
            let (lo, hi) = match mesh.kind {
                // The table's own rectangle, raised to this wall's heights.
                MeshKind::Wall => (
                    Vec3::new(pf.min.x, pf.min.y, b.min.z),
                    Vec3::new(pf.max.x, pf.max.y, b.max.z),
                ),
                MeshKind::Ramp | MeshKind::Rubber => (b.min, b.max),
                _ => continue,
            };
            out.extend_from_slice(&crate::backbox::corners_of(lo, hi));
        }
        // A table with no wall, ramp or rubber at all is not a table anybody
        // has, but a camera with nothing to frame is a black screen. The sheet
        // itself is always something to look at.
        if out.is_empty() {
            out.extend_from_slice(&crate::backbox::corners_of(
                Vec3::new(pf.min.x, pf.min.y, 0.0),
                Vec3::new(pf.max.x, pf.max.y, 0.0),
            ));
        }
        out
    }

    pub fn occupied(&self) -> Vec<Vec3> {
        const SLACK: f32 = 50.0;
        let pf = self.playfield;
        let mut out = Vec::new();
        for mesh in &self.meshes {
            if !mesh.visible || matches!(mesh.kind, MeshKind::Backbox) {
                continue;
            }
            // Primitives are left out, and that is Visual Pinball's own rule
            // rather than a convenience. Its legacy camera fit — the one every
            // table before 10.8 was authored against, and the one whose
            // numbers are still in these files — is built from a partial
            // bounding volume that *never* includes a primitive
            // (`primitive.cpp:622`, which says so in as many words; the
            // walls, ramps and rubbers fill the second list and primitives
            // only ever fill the first).
            //
            // It is not a detail. A table that models its own cabinet or a
            // room around itself does it with primitives, and The Sopranos'
            // room reaches 1215 units up over the whole playfield: fitting a
            // camera to that walks it backwards out through the room's own
            // opening, and what you get is a photograph of the outside of a
            // box with a pinball table somewhere inside it. Visual Pinball
            // has never framed on those, and neither does this now.
            if matches!(mesh.kind, MeshKind::Primitive) {
                continue;
            }
            let Some(b) = mesh.bounds() else { continue };
            let inside = b.max.x >= pf.min.x - SLACK
                && b.min.x <= pf.max.x + SLACK
                && b.max.y >= pf.min.y - SLACK
                && b.min.y <= pf.max.y + SLACK;
            if !inside {
                continue;
            }
            // Clamped to the playfield: a part may legitimately overhang the
            // edge a little, and the camera's job is to frame the table, not to
            // be dragged outward by a bracket screwed to the side of it.
            let lo = Vec3::new(
                b.min.x.max(pf.min.x),
                b.min.y.max(pf.min.y),
                b.min.z.max(0.0),
            );
            let hi = Vec3::new(b.max.x.min(pf.max.x), b.max.y.min(pf.max.y), b.max.z);
            if lo.x > hi.x || lo.y > hi.y || lo.z > hi.z {
                continue;
            }
            out.extend_from_slice(&[
                Vec3::new(lo.x, lo.y, hi.z),
                Vec3::new(hi.x, lo.y, hi.z),
                Vec3::new(lo.x, hi.y, hi.z),
                Vec3::new(hi.x, hi.y, hi.z),
            ]);
        }
        out
    }

    /// Box containing everything visible.
    pub fn bounds(&self) -> Bounds {
        self.meshes
            .iter()
            .filter(|m| m.visible)
            .filter_map(Mesh::bounds)
            .fold(self.playfield, Bounds::union)
    }
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

use vpin::vpx::VPX;
use vpin::vpx::color::Color;
use vpin::vpx::gameitem::GameItemEnum;
use vpin::vpx::gameitem::primitive::Primitive;

/// Builds the drawable scene of an already parsed table.
pub fn extract(vpx: &VPX) -> Scene {
    let g = &vpx.gamedata;
    let playfield = Bounds {
        min: Vec3::new(g.left, g.top, 0.0),
        max: Vec3::new(g.right, g.bottom, 0.0),
    };

    let mut meshes = Vec::new();
    let mut lights = Vec::new();
    let mut flashers = Vec::new();

    // The playfield plane is not in the file: it has to be generated. Unless
    // the table ships a primitive called `playfield_mesh`, which replaces it
    // (`primitive.h:271`, `IsPlayfield`).
    let has_own_mesh = vpx
        .gameitems
        .iter()
        .any(|i| matches!(i, GameItemEnum::Primitive(p) if is_playfield(&p.name)));
    if !has_own_mesh {
        meshes.push(playfield_quad(playfield, &g.image, &g.playfield_material));
    }

    for item in &vpx.gameitems {
        // Neither walls nor rubbers ship a mesh: they are generated from their
        // control points.
        if let GameItemEnum::Wall(w) = item {
            meshes.extend(crate::surface::build(w, playfield));
        }
        if let GameItemEnum::Rubber(r) = item {
            meshes.extend(crate::rubber::build(r));
        }
        if let GameItemEnum::Ramp(r) = item {
            meshes.extend(crate::ramp::build(r, playfield));
        }

        // The ones that use the original's builtin meshes. Each one may sit on
        // top of a wall rather than on the playfield.
        // The point matters: on a ramp the surface height moves along it.
        let surface_height =
            |s: &str, x: f32, y: f32| crate::builtin::surface_height(&vpx.gameitems, s, x, y);
        match item {
            GameItemEnum::Bumper(b) => {
                meshes.extend(crate::builtin::bumper(
                    b,
                    surface_height(&b.surface, b.center.x, b.center.y),
                ));
            }
            GameItemEnum::HitTarget(t) => meshes.extend(crate::builtin::hit_target(t)),
            GameItemEnum::Gate(g) => {
                meshes.extend(crate::builtin::gate(
                    g,
                    surface_height(&g.surface, g.center.x, g.center.y),
                ));
            }
            GameItemEnum::Spinner(s) => {
                meshes.extend(crate::builtin::spinner(
                    s,
                    surface_height(&s.surface, s.center.x, s.center.y),
                ));
            }
            GameItemEnum::Kicker(k) => {
                meshes.extend(crate::builtin::kicker(
                    k,
                    surface_height(&k.surface, k.center.x, k.center.y),
                ));
            }
            GameItemEnum::Flipper(f) => {
                let z = surface_height(&f.surface, f.center.x, f.center.y);
                meshes.extend(crate::flipper::build(f, z));
                meshes.extend(crate::flipper::rubber(f, z));
            }
            GameItemEnum::Trigger(t) => {
                meshes.extend(crate::trigger::build(
                    t,
                    surface_height(&t.surface, t.center.x, t.center.y),
                ));
            }
            GameItemEnum::Light(l) => {
                lights.extend(crate::light::build(
                    l,
                    surface_height(&l.surface, l.center.x, l.center.y),
                    &crate::light::Site::resolve(vpx, &l.surface, playfield),
                ));
            }
            // Not a mesh: a flasher is drawn by a pass of its own, blended,
            // with a state the script rewrites every frame. Baking it into the
            // static scene would draw a strobe permanently on, which is the one
            // thing a strobe never is.
            GameItemEnum::Flasher(f) => flashers.extend(crate::flasher::build(f, playfield)),
            _ => {}
        }

        if let GameItemEnum::Primitive(p) = item
            && let Some(mut mesh) = primitive_mesh(p)
        {
            // For that primitive the original **overrides** material and image
            // with the table's before drawing it (`primitive.cpp:1048-1053`).
            // Without this the table is drawn with whatever material the
            // primitive carries — often none — and the playfield comes out flat
            // and gray.
            if is_playfield(&p.name) {
                mesh.material = g.playfield_material.clone();
                mesh.image = g.image.clone();
                mesh.kind = MeshKind::Playfield;
            }
            meshes.push(mesh);
        }
    }

    // The head of the machine, which the file usually does not describe: a
    // `.vpx` stops at the playfield because Visual Pinball draws the backglass
    // from somewhere else entirely. Built from the cabinet's proportions so a
    // camera has something true to frame. See [`crate::backbox`].
    //
    // Usually, and not always — which is the whole of the check below.
    let head = crate::backbox::Backbox::for_playfield(playfield);
    let build_head = !brings_its_own_head(&meshes, playfield, &head);
    if build_head {
        // The table brought no head, so anything of its own standing up at
        // head height is the room it is standing in rather than the machine.
        // Marked here and left in: which views draw it is the renderer's
        // question, and the answer is the one that stands you inside it.
        let floor = head.bounds().min.z;
        for m in &mut meshes {
            if m.bounds().is_some_and(|b| b.max.z > floor) {
                m.scenery = true;
            }
        }
        // And its lamps, by the same line. A room's ceiling lights are up
        // there with its ceiling, and a view that leaves the ceiling out has
        // to leave them out too or they hang in the air as bare halos over a
        // playfield they are not lighting.
        for l in &mut lights {
            if l.center.z > floor {
                l.scenery = true;
            }
        }
        // And the table's own backbox lamps go out, because we just put a
        // backbox of our own where they were. A flasher standing at the very
        // back of the table, a hand's width or more above the wood, is not
        // lighting a playfield: The Sopranos has eight of them at 165 and 265
        // units across the back, which is its backglass lit from behind, and
        // with our head standing over them they read as a row of dots hanging
        // in the gap. The script keeps them — it can still switch them on and
        // read them back — they are simply not drawn.
        let far_end = playfield.min.y + (playfield.max.y - playfield.min.y) * 0.02;
        const BACKBOX_LAMP_HEIGHT: f32 = 100.0;
        flashers.retain(|f| !(f.center.y <= far_end && f.state.height >= BACKBOX_LAMP_HEIGHT));
        meshes.push(head.mesh());
        meshes.push(head.display_mesh());
    }

    // And the surface its face is textured with. Empty to start, because what
    // goes on it is what the machine is saying and the machine has not been
    // switched on yet; the renderer redraws it as the segments change.
    let mut images = images(vpx);

    // The artwork the head wears, painted rather than loaded: a `.vpx` has no
    // backglass in it, so the alternative is the blank white panel this port
    // used to stand behind every machine. See [`crate::backglass`] for what is
    // painted, and why it is painted from the table's own colours.
    let palette = palette_of(&images, &g.image);
    images.push(Image {
        name: crate::backglass::BACKGLASS_IMAGE.into(),
        encoded: None,
        rgba: Some(crate::backglass::paint(&palette)),
        width: crate::backglass::BACKGLASS_PIXELS.0,
        height: crate::backglass::BACKGLASS_PIXELS.1,
        // Opaque artwork on an opaque sheet: nothing to test, nothing to see
        // through.
        alpha_test: -1.0,
        has_alpha: false,
        redrawn: false,
    });

    images.push(Image {
        name: crate::backbox::DISPLAY_IMAGE.into(),
        encoded: None,
        rgba: Some(vec![
            0;
            (crate::backbox::DISPLAY_PIXELS.0 * crate::backbox::DISPLAY_PIXELS.1 * 4)
                as usize
        ]),
        width: crate::backbox::DISPLAY_PIXELS.0,
        height: crate::backbox::DISPLAY_PIXELS.1,
        // The score is drawn on it, not cut out of it.
        alpha_test: -1.0,
        has_alpha: true,
        redrawn: true,
    });

    Scene {
        view: AuthoredView {
            // A file that never wrote a field leaves a zero there, and zero is
            // not a camera: an inclination of zero looks along the playfield
            // edge-on and a field of view of zero has no width at all. So a
            // number that cannot be meant is replaced by the one Visual
            // Pinball would have used.
            inclination: sane(g.bg_inclination_desktop, 5.0..=85.0, 45.0),
            fov: sane(g.bg_fov_desktop, 5.0..=120.0, 45.0),
            layback: g.bg_layback_desktop,
            offset: Vec3::new(
                g.bg_offset_x_desktop,
                g.bg_offset_y_desktop,
                g.bg_offset_z_desktop,
            ),
        },
        cabinet: AuthoredView {
            inclination: sane(g.bg_inclination_fullscreen, 1.0..=85.0, 22.0),
            fov: sane(g.bg_fov_fullscreen, 5.0..=120.0, 20.0),
            layback: sane(g.bg_layback_fullscreen, 0.0..=89.0, 0.0),
            offset: Vec3::new(
                g.bg_offset_x_fullscreen,
                g.bg_offset_y_fullscreen,
                g.bg_offset_z_fullscreen,
            ),
        },
        built_head: build_head,
        meshes,
        materials: materials(vpx),
        images,
        playfield,
        playfield_image: g.image.clone(),
        playfield_material: g.playfield_material.clone(),
        // `Option` in the file because 10.01 did not write the tag; a
        // missing one means the same as an empty one.
        env_image: g.env_image.clone().unwrap_or_default(),
        ball_decal: g.ball_image_front.clone(),
        lighting: lighting(vpx),
        physics: table_physics(vpx),
        lights,
        flashers,
    }
}

/// The two scene lights, exactly as `Renderer.cpp:1055-1065` builds them.
/// The table's own physics numbers. See [`TablePhysics`].
fn table_physics(vpx: &VPX) -> TablePhysics {
    let g = &vpx.gamedata;
    // The original keeps the difficulty as a fraction and multiplies by a
    // hundred only for the script (`pintable.cpp:6373`). A file that stored a
    // percentage would put the slope far past its own maximum, so the value is
    // clamped rather than trusted.
    let difficulty = g.global_difficulty.clamp(0.0, 1.0);
    TablePhysics {
        slope_deg: g.angle_tilt_min + (g.angle_tilt_max - g.angle_tilt_min) * difficulty,
        gravity: g.gravity,
        default_scatter_deg: g.default_scatter,
        difficulty,
    }
}

fn lighting(vpx: &VPX) -> Lighting {
    let g = &vpx.gamedata;
    // The day/night scale, which multiplies the ambient, the two scene lights
    // and the environment alike (`Renderer.cpp:1037`, `:1051`, `:1063`).
    //
    // Always the table's, which is the mode the original starts in
    // (`Renderer.cpp:377`: the mode is `Mode::Table` unless the *player* has
    // ticked "override table emission scale", and there is no such setting
    // here), and `Renderer.cpp:398` then takes `m_globalEmissionScale`
    // unconditionally.
    //
    // Not gated on the file's "overwrite global day/night" flag, which is a
    // reasonable-looking guess and wrong: the original reads that flag and
    // throws the value away — `case FID(OGDN): reader.AsBool(); break;`
    // (`pintable.cpp:2574`). It is a leftover the loader still has to step over
    // to stay in sync with the stream.
    let global = g.global_emission_scale;
    let scale = g.light_emission_scale * global;
    Lighting {
        lights: [
            Vec3::new(g.right * 0.5, g.bottom / 3.0, g.light_height),
            Vec3::new(g.right * 0.5, g.bottom * 2.0 / 3.0, g.light_height),
        ],
        emission: color(&g.light0_emission).map(|c| c * scale),
        ambient: color(&g.light_ambient).map(|c| c * global),
        range: g.light_range,
        env_scale: g.env_emission_scale * global,
        global,
        // Tables older than 10.8 do not carry the field; the neutral value is 1.
        exposure: g.exposure.unwrap_or(1.0),
        bloom_strength: g.bloom_strength,
        reflection_strength: g.playfield_reflection_strength,
    }
}

/// Visual Pinball identifies the playfield **by name alone**, case-insensitively
/// (`primitive.h:271`).
fn is_playfield(name: &str) -> bool {
    name.eq_ignore_ascii_case("playfield_mesh")
}

/// The playfield plane: a quad at `z = 0` with the normal pointing up.
///
/// It comes from `pintable.cpp:3242-3260`. The UV goes from 0 to 1 across the
/// whole table.
fn playfield_quad(b: Bounds, image: &str, material: &str) -> Mesh {
    let n = [0.0, 0.0, 1.0];
    let vertices = vec![
        Vertex {
            pos: [b.min.x, b.min.y, 0.0],
            normal: n,
            uv: [0.0, 0.0],
        },
        Vertex {
            pos: [b.max.x, b.min.y, 0.0],
            normal: n,
            uv: [1.0, 0.0],
        },
        Vertex {
            pos: [b.max.x, b.max.y, 0.0],
            normal: n,
            uv: [1.0, 1.0],
        },
        Vertex {
            pos: [b.min.x, b.max.y, 0.0],
            normal: n,
            uv: [0.0, 1.0],
        },
    ];
    Mesh {
        name: "playfield".into(),
        vertices,
        indices: vec![0, 1, 2, 0, 2, 3],
        transform: Mat4::IDENTITY,
        image: image.to_string(),
        material: material.to_string(),
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Playfield,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    }
}

/// A primitive's mesh, for a caller that is going to place it itself.
///
/// Same geometry as the baked one; the difference is who owns the matrix. A
/// primitive the script animates gets its transform rebuilt every frame from
/// the numbers the script has written, so the one baked in here is only its
/// resting pose.
pub fn primitive_part(p: &Primitive) -> Option<Mesh> {
    primitive_mesh(p)
}

/// What the primitive says about being added rather than lit.
fn additive(p: &Primitive) -> Option<Additive> {
    if !p.add_blend.unwrap_or(false) {
        return None;
    }
    let c = p.color.as_ref();
    let color = c.map_or([1.0; 3], |c| {
        [
            f32::from(c.r) / 255.0,
            f32::from(c.g) / 255.0,
            f32::from(c.b) / 255.0,
        ]
    });
    Some(Additive {
        color,
        // A `.vpx` keeps it as a percentage.
        alpha: p.alpha.unwrap_or(100.0) / 100.0,
        light: p.light_map.clone().filter(|n| !n.is_empty()),
    })
}

fn primitive_mesh(p: &Primitive) -> Option<Mesh> {
    let raw = p.read_mesh().ok().flatten()?;
    let vertices = raw
        .vertices
        .iter()
        .map(|w| {
            let v = &w.vertex;
            Vertex {
                pos: [v.x, v.y, v.z],
                normal: [v.nx, v.ny, v.nz],
                uv: [v.tu, v.tv],
            }
        })
        .collect();
    let indices = raw
        .indices
        .iter()
        .flat_map(|f| [f.i0 as u32, f.i1 as u32, f.i2 as u32])
        .collect();

    Some(Mesh {
        name: p.name.clone(),
        vertices,
        indices,
        transform: primitive_transform(p),
        image: p.image.clone(),
        material: p.material.clone(),
        visible: p.is_visible,
        clamp: false,
        scenery: false,
        kind: MeshKind::Primitive,
        additive: additive(p),
        depth_bias: p.depth_bias,
        disable_lighting: p.disable_lighting_top.unwrap_or(0.0).clamp(0.0, 1.0),
    })
}

/// A primitive's transform: scale, two rotations and position.
///
/// # The order matters, and the whole thing has to be flipped
///
/// The original builds it in `primitive.cpp:372-388`, and since it uses
/// **row vectors** (`v * M`, with the translation in the fourth *row* — see
/// `matrix.h:524` and `MultiplyVector` in `matrix.h:759`), its products read
/// left to right **in the order they are applied**:
///
/// ```text
/// scale, translation, RotZ(r2), RotY(r1), RotX(r0),
///                     RotZ(r8), RotY(r7), RotX(r6), position
/// ```
///
/// `glam` uses column vectors, so the same chain is written **backwards**.
/// Reversing only part of it — for instance leaving the rotations as they were
/// — displaces and misorients any piece that uses the mesh's own translation,
/// and it looks like primitives floating far away from the table.
///
/// The per-axis rotations do match `glam`'s one to one: the three in
/// `matrix.h:300-322` are the standard counter-clockwise ones.
pub fn primitive_transform(p: &Primitive) -> Mat4 {
    let r = &p.rot_and_tra;
    let rot_x = |a: f32| Mat4::from_rotation_x(a.to_radians());
    let rot_y = |a: f32| Mat4::from_rotation_y(a.to_radians());
    let rot_z = |a: f32| Mat4::from_rotation_z(a.to_radians());

    // Indices 0..2 are the first rotation, 3..5 the mesh's own translation and
    // 6..8 the second one.
    Mat4::from_translation(Vec3::new(p.position.x, p.position.y, p.position.z))
        * rot_x(r[6])
        * rot_y(r[7])
        * rot_z(r[8])
        * rot_x(r[0])
        * rot_y(r[1])
        * rot_z(r[2])
        * Mat4::from_translation(Vec3::new(r[3], r[4], r[5]))
        * Mat4::from_scale(Vec3::new(p.size.x, p.size.y, p.size.z))
}

/// Same as [`primitive_transform`] but taking the loose fields, so the order can
/// be pinned down in a test without building a whole primitive.
pub fn primitive_transform_from_fields(position: Vec3, size: Vec3, rot_and_tra: [f32; 9]) -> Mat4 {
    let r = &rot_and_tra;
    let rot_x = |a: f32| Mat4::from_rotation_x(a.to_radians());
    let rot_y = |a: f32| Mat4::from_rotation_y(a.to_radians());
    let rot_z = |a: f32| Mat4::from_rotation_z(a.to_radians());

    Mat4::from_translation(position)
        * rot_x(r[6])
        * rot_y(r[7])
        * rot_z(r[8])
        * rot_x(r[0])
        * rot_y(r[1])
        * rot_z(r[2])
        * Mat4::from_translation(Vec3::new(r[3], r[4], r[5]))
        * Mat4::from_scale(size)
}

/// A colour from the file, the way the original converts one.
///
/// A plain divide by 255 and **no gamma decode**, which is `convertColor`
/// (`utils/color.h:22`) and is what feeds the material's base, glossy and
/// clearcoat (`Shader.cpp:830-838`), the ambient and the scene lights
/// (`Renderer.cpp:1049`, `:1062`) and a lamp's two colours
/// (`light.cpp:711-712`).
///
/// The asymmetry with textures is deliberate on the original's part and is
/// worth stating, because it looks like a bug: a *texture* is decoded, by the
/// hardware, because it is a picture of something. A material colour is not a
/// picture — it is a multiplier the table's author dialled in while looking at
/// the result, so the number that matters is the one they saw.
///
/// Decoding it as well costs about a third of the light on a table. A base
/// colour of 180 is a multiplier of 0.706 in the original and 0.456 with a
/// decode, so a playfield the original draws at 70 out of 255 comes out at 54.
/// Whether a picture is stored with **floating-point** channels.
///
/// Which decides whether its alpha counts for anything, and the original is
/// explicit that it does not: `BaseTexture::UpdateOpaque` scans the alpha of
/// an 8-bit image to find out whether it is really see-through, and skips the
/// scan entirely for a float one, with the note that "the alpha channel is
/// always opaque, only added for driver's texture format support"
/// (`Texture.cpp:883`). So a float image is opaque, full stop.
///
/// It matters because of what carries float channels: a **baked** table's
/// atlas, written as OpenEXR because a lightmap holds light and light does not
/// fit in a byte. Counting its alpha puts the bake's meshes in the see-through
/// pass, where they write no depth and paint over whatever is already
/// there — on Circus that is the instruction cards on the apron, every "WHEN
/// LIT" inside its insert, and the target numbers: thirty-nine thousand pixels
/// of a table, gone under a sheet that should have been standing behind them.
///
/// Told by the magic number rather than by decoding: OpenEXR opens with
/// `0x76 0x2f 0x31 0x01`, and Radiance's `.hdr` — the other float format a
/// table might carry — with `#?RADIANCE`.
fn floating_point(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x76, 0x2F, 0x31, 0x01]) || bytes.starts_with(b"#?RADIANCE")
}

fn color(c: &Color) -> [f32; 3] {
    [c.r, c.g, c.b].map(|v| f32::from(v) / 255.0)
}

fn materials(vpx: &VPX) -> Vec<Material> {
    // 10.8 tables carry `materials`; the old ones, `materials_old`.
    if let Some(modern) = &vpx.gamedata.materials {
        return modern
            .iter()
            .map(|m| {
                let [r, gg, b] = color(&m.base_color);
                Material {
                    name: m.name.clone(),
                    base_color: [r, gg, b],
                    glossy_color: color(&m.glossy_color),
                    clearcoat_color: color(&m.clearcoat_color),
                    is_metal: m.type_ == vpin::vpx::material::MaterialType::Metal,
                    roughness: m.roughness,
                    wrap_lighting: m.wrap_lighting,
                    glossy_image_lerp: m.glossy_image_lerp,
                    edge: m.edge,
                    edge_alpha: m.edge_alpha,
                    thickness: m.thickness,
                    opacity: m.opacity,
                    opacity_active: m.opacity_active,
                }
            })
            .collect();
    }
    vpx.gamedata
        .materials_old
        .iter()
        .map(|m| {
            let [r, gg, b] = color(&m.base_color);
            Material {
                name: m.name.clone(),
                base_color: [r, gg, b],
                glossy_color: color(&m.glossy_color),
                clearcoat_color: color(&m.clearcoat_color),
                is_metal: m.is_metal,
                roughness: m.roughness,
                wrap_lighting: m.wrap_lighting,
                // Old tables store it quantized and **inverted**: the original
                // reads it as `1 - v/255` and notes that the '1.0 -' is there
                // for compatibility with previous versions
                // (`pintable.cpp:2491`).
                glossy_image_lerp: 1.0 - f32::from(m.glossy_image_lerp) / 255.0,
                edge: m.edge,
                // The lowest bit is the opacity flag; the seven above it are
                // the weight of the Fresnel at the edge (`Material.h:23`).
                edge_alpha: f32::from(m.opacity_active_edge_alpha >> 1) / 127.0,
                thickness: f32::from(m.thickness) / 255.0,
                opacity: m.opacity,
                opacity_active: (m.opacity_active_edge_alpha & 1) != 0,
            }
        })
        .collect()
}

/// A stored number, or the fallback when it is outside what it could mean.
fn sane(value: f32, range: std::ops::RangeInclusive<f32>, fallback: f32) -> f32 {
    if value.is_finite() && range.contains(&value) {
        value
    } else {
        fallback
    }
}

/// Whether the table already models a head of its own.
///
/// Three things have to be true at once, and every one of them was learned by
/// getting it wrong.
///
/// **It has to be high.** Nothing that belongs on a playfield stands as high
/// as a machine's head — the tallest ramp is two or three hundred units and
/// the head starts three times higher. But *reaching* the head's floor is not
/// enough: The Sopranos has a toy at the back of its playfield topping out
/// eleven units above it, and that toy was enough to make us go without a
/// head. So the test is halfway up the head, not its underside.
///
/// **It has to be behind the playfield.** A backbox stands at the far end,
/// off the end of the table; a *room* stands over the whole thing. The
/// Sopranos has one of those too, reaching twelve hundred units over the
/// entire playfield, and height alone called it a head.
///
/// Get either wrong and the table loses its backglass and the score panel
/// standing on it, which is a great deal to give up for a pole dancer.
fn brings_its_own_head(meshes: &[Mesh], playfield: Bounds, head: &crate::backbox::Backbox) -> bool {
    let box_ = head.bounds();
    // Halfway up the head we would otherwise build.
    let high = (box_.min.z + box_.max.z) * 0.5;
    // And at or behind the far edge of the playfield, with a hair of slack for
    // a head modelled a few units onto it.
    let behind = playfield.min.y + (playfield.max.y - playfield.min.y) * 0.01;
    meshes.iter().any(|m| {
        m.visible
            && m.bounds()
                .is_some_and(|b| b.max.z > high && b.min.y <= behind)
    })
}

/// The colours of the machine, for the head's artwork.
///
/// The playfield texture first, because it is the one image a table always
/// has and the one its designer picked the whole machine's look from. If it
/// has nothing to say — and plenty of images in a table do not, being masks
/// and cut-outs — the biggest pictures the file carries are asked in turn, on
/// the reasoning that the largest thing an author bothered to store is the
/// thing they meant to be looked at.
///
/// A few tried at most: each one that is not already pixels has to be
/// decoded, and a table's textures run to four thousand pixels square. This
/// is the expensive half of painting the head, and it is paid once.
fn palette_of(images: &[Image], playfield_image: &str) -> crate::backglass::Palette {
    /// How many pictures are asked before the house colours are used.
    const TRIES: usize = 4;

    let mut candidates: Vec<&Image> = Vec::with_capacity(TRIES);
    // Names in a `.vpx` are matched without regard to case everywhere else in
    // the file, and they have to be here too: F-14 asks for "playfield" and
    // stores "Playfield". Comparing them exactly is why every machine came
    // out wearing the house colours the first time.
    if let Some(pf) = images
        .iter()
        .find(|i| i.name.eq_ignore_ascii_case(playfield_image))
    {
        candidates.push(pf);
    }
    let mut rest: Vec<&Image> = images
        .iter()
        .filter(|i| !candidates.iter().any(|c| std::ptr::eq(*c, *i)))
        .collect();
    rest.sort_by_key(|i| std::cmp::Reverse(u64::from(i.width) * u64::from(i.height)));
    candidates.extend(rest);

    for art in candidates.into_iter().take(TRIES) {
        let palette = match (&art.rgba, &art.encoded) {
            // A raw BMP: already pixels.
            (Some(rgba), _) => crate::backglass::Palette::from_art(rgba, art.width, art.height),
            (None, Some(bytes)) => match image::load_from_memory(bytes) {
                Ok(decoded) => {
                    let rgba = decoded.to_rgba8();
                    let (w, h) = (rgba.width(), rgba.height());
                    crate::backglass::Palette::from_art(&rgba, w, h)
                }
                Err(e) => {
                    log::warn!("the image \"{}\" would not decode: {e}", art.name);
                    None
                }
            },
            _ => None,
        };
        if let Some(palette) = palette {
            log::debug!(
                "the head is painted from \"{}\": {:?}",
                art.name,
                palette.colours
            );
            return palette;
        }
    }
    log::debug!("nothing in this table says what colour it is; the head keeps the house colours");
    crate::backglass::Palette::fallback()
}

fn images(vpx: &VPX) -> Vec<Image> {
    vpx.images
        .iter()
        .map(|i| Image {
            redrawn: false,
            name: i.name.clone(),
            encoded: i.jpeg.as_ref().map(|j| j.data.clone()),
            rgba: i
                .bits
                .as_ref()
                .and_then(|b| bmp_to_rgba(b, i.width, i.height)),
            width: i.width,
            height: i.height,
            // The `.vpx` BMPs always come with alpha 255; of the rest, the
            // only format among those that show up in tables which cannot
            // carry alpha is JPEG. And a float image never counts, whatever
            // its alpha channel holds — see [`floating_point`].
            has_alpha: i.bits.is_none()
                && !i.ext().eq_ignore_ascii_case("jpg")
                && !i.jpeg.as_ref().is_some_and(|j| floating_point(&j.data)),
            // `Texture.cpp:937`. A table that never set one leaves the field at
            // its own default, which is below zero and means "do not".
            alpha_test: i.alpha_test_value / 255.0,
        })
        .collect()
}

/// Decompresses a raw BMP from the `.vpx` and converts it to RGBA.
///
/// They come as 32-bit sBGRA compressed with LZW and, like every BMP, with the
/// rows running bottom to top: they have to be flipped.
fn bmp_to_rgba(bits: &vpin::vpx::image::ImageDataBits, width: u32, height: u32) -> Option<Vec<u8>> {
    let raw = vpin::vpx::lzw::from_lzw_blocks(&bits.lzw_compressed_data);
    let expected = (width as usize) * (height as usize) * 4;
    if raw.len() < expected || width == 0 || height == 0 {
        return None;
    }
    let row_bytes = width as usize * 4;
    let mut out = Vec::with_capacity(expected);
    for row in (0..height as usize).rev() {
        let from = row * row_bytes;
        for px in raw[from..from + row_bytes].as_chunks::<4>().0 {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    Some(out)
}
