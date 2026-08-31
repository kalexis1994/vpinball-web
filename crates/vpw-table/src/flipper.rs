//! Flippers.
//!
//! The only item whose mesh is half builtin and half computed. The original
//! starts from `flipperBase` — a cylinder with a base and a tip — and
//! **rearranges** the thirteen vertices of each circle so the radii match the
//! ones the table declares (`Flipper::GenerateBaseMesh`, `flipper.cpp:636`).
//!
//! # Why they need rearranging
//!
//! The builtin mesh has a base and a tip of arbitrary radii. Each table declares
//! its own — `BaseRadius`, `EndRadius` — and the length between centers. Scaling
//! the whole mesh would deform both circles equally; what the original does is
//! identify the thirteen vertices of each circle by their exact position and
//! re-project them onto the correct radius, correcting the angle along the way
//! so the join between base and tip comes out tangent (`ApplyFix`,
//! `flipper.cpp:604`).

use crate::geometry::{Mesh, MeshKind, Vertex};
use crate::meshes;
use std::f32::consts::PI;
use vpw_math::{Mat4, Vec2, Vec3};

// The four reference circles of the builtin mesh. They serve to recognize which
// vertex belongs to which circle, comparing by exact position — which is what
// the original does.
const VERTSBASEBOTTOM: [[f32; 3]; 13] = [
    [-0.100762, -0.0, 0.003753],
    [-0.097329, -0.026079, 0.003753],
    [-0.087263, -0.050381, 0.003753],
    [-0.07125, -0.07125, 0.003753],
    [-0.050381, -0.087263, 0.003753],
    [-0.026079, -0.097329, 0.003753],
    [-0.0, -0.100762, 0.003753],
    [0.026079, -0.097329, 0.003753],
    [0.050381, -0.087263, 0.003753],
    [0.07125, -0.07125, 0.003753],
    [0.087263, -0.050381, 0.003753],
    [0.097329, -0.026079, 0.003753],
    [0.100762, -0.0, 0.003753],
];

const VERTSBASETOP: [[f32; 3]; 13] = [
    [-0.100762, 0.0, 1.00425],
    [-0.097329, -0.026079, 1.00425],
    [-0.087263, -0.050381, 1.00425],
    [-0.07125, -0.07125, 1.00425],
    [-0.050381, -0.087263, 1.00425],
    [-0.026079, -0.097329, 1.00425],
    [-0.0, -0.100762, 1.00425],
    [0.026079, -0.097329, 1.00425],
    [0.050381, -0.087263, 1.00425],
    [0.07125, -0.07125, 1.00425],
    [0.087263, -0.050381, 1.00425],
    [0.097329, -0.026079, 1.00425],
    [0.100762, -0.0, 1.00425],
];

const VERTSTIPBOTTOM: [[f32; 3]; 13] = [
    [-0.101425, 0.786319, 0.003753],
    [-0.097969, 0.812569, 0.003753],
    [-0.087837, 0.837031, 0.003753],
    [-0.071718, 0.858037, 0.003753],
    [-0.050713, 0.874155, 0.003753],
    [-0.026251, 0.884288, 0.003753],
    [-0.0, 0.887744, 0.003753],
    [0.026251, 0.884288, 0.003753],
    [0.050713, 0.874155, 0.003753],
    [0.071718, 0.858037, 0.003753],
    [0.087837, 0.837031, 0.003753],
    [0.097969, 0.812569, 0.003753],
    [0.101425, 0.786319, 0.003753],
];

const VERTSTIPTOP: [[f32; 3]; 13] = [
    [-0.101425, 0.786319, 1.00425],
    [-0.097969, 0.812569, 1.00425],
    [-0.087837, 0.837031, 1.00425],
    [-0.071718, 0.858037, 1.00425],
    [-0.050713, 0.874155, 1.00425],
    [-0.026251, 0.884288, 1.00425],
    [-0.0, 0.887744, 1.00425],
    [0.026251, 0.884288, 1.00425],
    [0.050713, 0.874155, 1.00425],
    [0.071718, 0.858037, 1.00425],
    [0.087837, 0.837031, 1.00425],
    [0.097969, 0.812569, 1.00425],
    [0.101425, 0.786319, 1.00425],
];

/// `ApplyFix`, `flipper.cpp:604`.
///
/// Re-projects a vertex onto a circle of a different radius, turning it towards
/// `mid_angle` by the proportion `fix_angle_scale`. The normal goes along with
/// the same turn, keeping its length.
fn apply_fix(
    v: &mut Vertex,
    center: Vec2,
    mid_angle: f32,
    radius: f32,
    new_center: Vec2,
    fix_angle_scale: f32,
) {
    let mut v_angle = (v.pos[1] - center.y).atan2(v.pos[0] - center.x);
    let mut n_angle = v.normal[1].atan2(v.normal[0]);

    // Both angles have to have the same sign as `mid_angle`.
    if mid_angle < 0.0 {
        if v_angle > 0.0 {
            v_angle -= 2.0 * PI;
        }
        if n_angle > 0.0 {
            n_angle -= 2.0 * PI;
        }
    } else {
        if v_angle < 0.0 {
            v_angle += 2.0 * PI;
        }
        if n_angle < 0.0 {
            n_angle += 2.0 * PI;
        }
    }

    let sign = if mid_angle < 0.0 { -1.0 } else { 1.0 };
    n_angle -= (v_angle - mid_angle) * fix_angle_scale * sign;
    v_angle -= (v_angle - mid_angle) * fix_angle_scale * sign;

    let normal_len = Vec2::new(v.normal[0], v.normal[1]).length();
    v.pos[0] = v_angle.cos() * radius + new_center.x;
    v.pos[1] = v_angle.sin() * radius + new_center.y;
    v.normal[0] = n_angle.cos() * normal_len;
    v.normal[1] = n_angle.sin() * normal_len;
}

/// Whether a vertex falls exactly on one of the reference circles.
///
/// The comparison is **exact**, as in the original: the builtin mesh's vertices
/// are the same literals as these, so they match bit for bit.
fn belongs_to(v: &Vertex, circle: &[[f32; 3]; 13]) -> bool {
    circle
        .iter()
        .any(|c| v.pos[0] == c[0] && v.pos[1] == c[1] && v.pos[2] == c[2])
}

/// The bat and its rubber ring.
///
/// **Two meshes, not one.** The original builds the same base mesh twice
/// (`flipper.cpp:683-712`): once shrunk by the rubber's thickness, which is the
/// bat, and once at the full radius, which is the ring — z-scaled by
/// `rubberwidth`, lifted by `rubberheight`, and taking the second half of the
/// texture. Shrinking the bat and never adding the ring back, which is what
/// this did, leaves every flipper on every table visibly thinner than the one
/// the author drew, and with no rubber on it at all.
pub fn build(f: &vpin::vpx::gameitem::flipper::Flipper, base_z: f32) -> Option<Mesh> {
    if !f.is_visible {
        return None;
    }
    let m = meshes::get("flipperBase")?;
    let mut vertices = m.vertices.clone();

    // The length between centers. `FlipperRadiusMin` lets the global difficulty
    // shorten the flipper; with no physics yet, the maximum is used.
    let length = f.flipper_radius_max.max(0.01);
    let rubber = f.rubber_thickness.unwrap_or(0.0);
    let base_radius = f.base_radius - rubber;
    let tip_radius = f.end_radius - rubber;

    // The angle needed for the join between base and tip to come out tangent.
    // The original notes that forcing it to zero reproduces the look of the old
    // versions, with the join broken.
    let sine = ((f.base_radius - f.end_radius) / length).clamp(-1.0, 1.0);
    let fix_angle_scale = sine.asin() / (PI * 0.5);

    let base_center = Vec2::new(VERTSBASEBOTTOM[6][0], VERTSBASEBOTTOM[0][1]);
    let tip_center = Vec2::new(VERTSTIPBOTTOM[6][0], VERTSTIPBOTTOM[0][1]);

    for v in &mut vertices {
        if belongs_to(v, &VERTSBASEBOTTOM) || belongs_to(v, &VERTSBASETOP) {
            apply_fix(
                v,
                base_center,
                -PI * 0.5,
                base_radius,
                Vec2::ZERO,
                fix_angle_scale,
            );
        } else if belongs_to(v, &VERTSTIPBOTTOM) || belongs_to(v, &VERTSTIPTOP) {
            apply_fix(
                v,
                tip_center,
                PI * 0.5,
                tip_radius,
                Vec2::new(0.0, length),
                fix_angle_scale,
            );
        }
    }

    // The builtin mesh faces the wrong way; the original turns it half a turn in
    // Z and only then scales it in height (`flipper.cpp:637, 672`).
    let half_turn = Mat4::from_rotation_z(PI);
    let height = Mat4::from_scale(Vec3::new(1.0, 1.0, f.height));

    // And it places it at its center, turned by the rest angle
    // (`flipper.cpp:541-544`).
    let place = Mat4::from_translation(Vec3::new(f.center.x, f.center.y, base_z))
        * Mat4::from_rotation_z(f.start_angle.to_radians());

    Some(Mesh {
        name: f.name.clone(),
        vertices,
        indices: m.indices,
        transform: place * height * half_turn,
        image: f.image.clone().unwrap_or_default(),
        material: f.material.clone(),
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Builtin,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    })
}

/// The rubber ring that goes round the bat.
///
/// The same base mesh a second time at the **full** radius — the original
/// writes `baseRadius + m_d.m_rubberthickness`, which is where it started
/// before [`build`] shrank it (`flipper.cpp:691`) — with its own thickness,
/// its own height above the playfield, its own material, and the second half
/// of the texture so that one image can carry the bat and its rubber.
///
/// Without it every flipper on every table is visibly thinner than the one its
/// author drew, and has no rubber on it at all.
pub fn rubber(f: &vpin::vpx::gameitem::flipper::Flipper, base_z: f32) -> Option<Mesh> {
    if !f.is_visible {
        return None;
    }
    let thickness = f.rubber_thickness.unwrap_or(0.0);
    if thickness <= 0.0 {
        return None;
    }
    let m = meshes::get("flipperBase")?;
    let mut vertices = m.vertices.clone();

    let length = f.flipper_radius_max.max(0.01);
    let sine = ((f.base_radius - f.end_radius) / length).clamp(-1.0, 1.0);
    let fix_angle_scale = sine.asin() / (PI * 0.5);
    let base_center = Vec2::new(VERTSBASEBOTTOM[6][0], VERTSBASEBOTTOM[0][1]);
    let tip_center = Vec2::new(VERTSTIPBOTTOM[6][0], VERTSTIPBOTTOM[0][1]);

    for v in &mut vertices {
        if belongs_to(v, &VERTSBASEBOTTOM) || belongs_to(v, &VERTSBASETOP) {
            apply_fix(
                v,
                base_center,
                -PI * 0.5,
                f.base_radius,
                Vec2::ZERO,
                fix_angle_scale,
            );
        } else if belongs_to(v, &VERTSTIPBOTTOM) || belongs_to(v, &VERTSTIPTOP) {
            apply_fix(
                v,
                tip_center,
                PI * 0.5,
                f.end_radius,
                Vec2::new(0.0, length),
                fix_angle_scale,
            );
        }
        // `flipper.cpp:711`.
        v.uv[1] += 0.5;
    }

    let half_turn = Mat4::from_rotation_z(PI);
    let width = Mat4::from_scale(Vec3::new(1.0, 1.0, f.rubber_width.unwrap_or(0.0)));
    let place = Mat4::from_translation(Vec3::new(
        f.center.x,
        f.center.y,
        base_z + f.rubber_height.unwrap_or(0.0),
    )) * Mat4::from_rotation_z(f.start_angle.to_radians());

    Some(Mesh {
        name: format!("{} (rubber)", f.name),
        vertices,
        indices: m.indices,
        transform: place * width * half_turn,
        image: f.image.clone().unwrap_or_default(),
        material: f.rubber_material.clone(),
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Builtin,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    })
}
