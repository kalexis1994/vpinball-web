//! The ball: its mesh, how it looks and where it goes.
//!
//! It is the only piece that does not come out of any `.vpx`: the ball is not a
//! table item, the game puts it there. The original ships it as a builtin mesh
//! in `src/meshes/ballMesh.h` — a subdivided icosahedron of 181 vertices and 320
//! faces, with the `uv`s already mapped for a ball texture — and a smaller
//! version, `basicBallLo`, for when it is far away or the detail is at its
//! lowest (`Renderer.cpp:246-249`).
//!
//! The medium one is used here, which is what Visual Pinball draws by default.
//! The small one does not make the cut: a player that draws one ball has no
//! reason whatsoever to turn its detail down.
//!
//! # Why the original mesh's `uv`s matter
//!
//! Because the ball **rolls**. A sphere generated on the fly is easy to write but
//! it ends up with a seam where `u` wraps from 1 back to 0, and a spinning ball
//! texture shows it no mercy. The ones in `ballMesh.h` come already sorted out.

use crate::geometry::{Material, Mesh, MeshKind};
use crate::meshes;
use vpw_math::{Mat4, Quat, Vec3};

/// The name of the mesh inside the blob, as it is called in the original.
const MESH_NAME: &str = "basicBallMid";

/// The ball's mesh: a sphere of radius one centered at the origin.
///
/// The radius goes in the transform and not in the vertices: the ball is scaled,
/// moved and rotated every frame, so its matrix is alive anyway.
pub fn mesh() -> Mesh {
    let m = meshes::get(MESH_NAME).expect("the ball's mesh has to be in the blob");
    Mesh {
        name: "Ball".into(),
        vertices: m.vertices,
        indices: m.indices,
        transform: Mat4::IDENTITY,
        image: String::new(),
        material: String::new(),
        visible: true,
        kind: MeshKind::Builtin,
    }
}

/// Where the ball goes, with its radius and its spin (`ball.cpp:445-455`).
///
/// The original composes `rot · scale · translation` with the row-vector
/// convention; in `glam`, which is column-vector, the product goes the other way
/// round.
///
/// `captured` is for balls a kicker is holding: the original draws them one
/// radius lower so they look **sunk** into the hole and not floating above it.
pub fn transform(pos: Vec3, radius: f32, orientation: Quat, captured: bool) -> Mat4 {
    let z = if captured { pos.z - radius } else { pos.z };
    Mat4::from_translation(Vec3::new(pos.x, pos.y, z))
        * Mat4::from_scale(Vec3::splat(radius))
        * Mat4::from_quat(orientation)
}

/// How a ball without a texture looks: polished steel.
///
/// The original devotes a whole shader to it (`BallShader.hlsl`), with a
/// reflection of the playfield and the six nearest lamps reflected on top. That
/// is not here yet; what is here is what makes a ball look like a ball, which is
/// that it be **metal**: that way the base color acts as the specular and all
/// the light you see on it comes from the environment map. A smooth sphere lit
/// only by the two scene lights looks like a gray plastic ball.
pub fn material() -> Material {
    Material {
        name: "vpw-ball".into(),
        base_color: [0.85, 0.85, 0.88],
        glossy_color: [1.0, 1.0, 1.0],
        clearcoat_color: [0.0; 3],
        is_metal: true,
        // Almost smooth: the steel of a pinball reflects sharply.
        roughness: 0.95,
        wrap_lighting: 0.0,
        glossy_image_lerp: 1.0,
        edge: 1.0,
        edge_alpha: 1.0,
        thickness: 0.05,
        opacity: 1.0,
        opacity_active: false,
    }
}

/// The wear a ball earns: thin scuffs over the polish, made here.
///
/// This is what makes the roll **visible**. The physics spins the ball and the
/// mesh turns with it, but a perfect mirror looks identical from every
/// orientation — everything it shows depends on the view and the normal, not
/// on which side of the sphere faces you. The original ships a scuffed decal
/// in its `Assets/` for exactly this reason, and a table can bring its own
/// (`BLIF`); this is the fallback for tables that do not.
///
/// The texture is multiplicative — white leaves the steel alone, a scuff dims
/// it — because that is what a scratch does: scatter light the mirror would
/// have returned. Deterministic on purpose, so every ball on every table
/// carries the same wear and a pixel test can rely on it.
pub fn scratches() -> crate::geometry::Image {
    const W: usize = 256;
    const H: usize = 256;
    let mut px = vec![255u8; W * H * 4];

    // A small linear congruential generator: enough randomness for scuffs,
    // no dependency, and the same picture every run.
    let mut state: u32 = 0x2b5d_11a7;
    let mut rand = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 16_777_216.0
    };

    // Each scuff is a short random walk: a starting point, a heading that
    // drifts, a depth that fades in and out along its length. Drawn wrapping
    // horizontally, because the sphere's UV seam is a meridian and a scuff
    // that stops dead at it would draw the seam on the ball.
    for _ in 0..160 {
        let mut x = rand() * W as f32;
        let mut y = rand() * H as f32;
        let mut heading = rand() * std::f32::consts::TAU;
        let len = 8.0 + rand() * 40.0;
        let depth = 10.0 + rand() * 35.0;
        let steps = len as usize;
        for i in 0..steps {
            heading += (rand() - 0.5) * 0.4;
            x += heading.cos();
            y += heading.sin();
            // In and out: a scuff is deepest in its middle.
            let along = i as f32 / steps as f32;
            let bite = depth * (1.0 - (2.0 * along - 1.0).abs());
            let xi = x.rem_euclid(W as f32) as usize % W;
            let yi = (y.max(0.0) as usize).min(H - 1);
            let at = (yi * W + xi) * 4;
            for c in 0..3 {
                px[at + c] = px[at + c].saturating_sub(bite as u8);
            }
        }
    }

    crate::geometry::Image {
        name: "vpw-ball-scratches".into(),
        encoded: None,
        rgba: Some(px),
        width: W as u32,
        height: H as u32,
        has_alpha: false,
        alpha_test: -1.0,
        redrawn: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_originals_mesh_is_in_the_blob() {
        let m = mesh();
        // `basicBallMidNumVertices = 181`, `basicBallMidNumFaces = 320*3`.
        assert_eq!(m.vertices.len(), 181);
        assert_eq!(m.indices.len(), 960);
    }

    #[test]
    fn it_is_a_sphere_of_radius_one() {
        for v in &mesh().vertices {
            let p = Vec3::from_array(v.pos);
            assert!(
                (p.length() - 1.0).abs() < 1e-3,
                "vertex off the unit radius: {p:?}"
            );
        }
    }

    #[test]
    fn a_captured_ball_is_drawn_sunk() {
        let pos = Vec3::new(100.0, 200.0, 25.0);
        let free = transform(pos, 25.0, Quat::IDENTITY, false);
        let captured = transform(pos, 25.0, Quat::IDENTITY, true);
        assert_eq!(free.w_axis.z, 25.0);
        assert_eq!(captured.w_axis.z, 0.0);
    }

    #[test]
    fn the_transform_takes_the_mesh_where_it_belongs() {
        let m = transform(Vec3::new(10.0, 20.0, 30.0), 25.0, Quat::IDENTITY, false);
        // The north pole of the unit sphere ends up one radius higher.
        let p = m.transform_point3(Vec3::Z);
        assert!((p - Vec3::new(10.0, 20.0, 55.0)).length() < 1e-3);
    }

    #[test]
    fn the_balls_material_is_metal() {
        let inputs = material().shader_inputs();
        assert!(inputs.is_metal);
        // On metal the specular comes from the base color; the field of its own
        // goes black.
        assert_eq!(inputs.glossy_color, [0.0; 3]);
        assert_eq!(inputs.alpha, 1.0);
    }

    #[test]
    fn the_scratches_are_wear_on_white() {
        let img = scratches();
        let px = img.rgba.as_ref().unwrap();
        assert_eq!(px.len(), (img.width * img.height * 4) as usize);
        // Multiplicative wear: mostly untouched steel, nothing brighter than
        // white, and enough scuffed texels that a roll is actually visible.
        let mut worn = 0usize;
        for texel in px.chunks_exact(4) {
            assert_eq!(texel[3], 255, "wear has no transparency");
            if texel[0] < 250 {
                worn += 1;
            }
        }
        let total = (img.width * img.height) as usize;
        assert!(worn > total / 200, "hardly any wear: {worn} texels");
        assert!(worn < total / 4, "more scuff than steel: {worn} texels");
    }

    #[test]
    fn the_wear_is_the_same_every_time() {
        // A pixel test elsewhere may rely on the ball's look, so the wear has
        // to come out identical run after run.
        assert_eq!(scratches().rgba, scratches().rgba);
    }
}
