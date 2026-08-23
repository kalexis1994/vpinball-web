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
}
