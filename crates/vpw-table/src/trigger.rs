//! A trigger's mesh.
//!
//! Port of `Trigger::GenerateMesh` and `Trigger::RenderSetup` (`trigger.cpp`).
//!
//! The star and button ones are solid pieces scaled by their radius. The wire
//! ones are a thin ring, and the thickness does not come in the mesh: each
//! vertex is pushed out along its **normal** by `wire_thickness`. That is what
//! makes the same ring work for a thick wire and a thin one.

use crate::geometry::{Mesh, MeshKind, Vertex};
use crate::meshes;
use vpin::vpx::gameitem::trigger::{Trigger, TriggerShape};
use vpw_math::{Mat4, Vec3};

/// How far each shape sinks when the ball goes over it
/// (`Trigger::UpdateAnimation`, `trigger.cpp:493`).
pub fn sink_depth(shape: &TriggerShape, radius: f32) -> f32 {
    match shape {
        TriggerShape::Star => radius * (1.0 / 5.0),
        TriggerShape::Button => radius * (1.0 / 10.0),
        TriggerShape::WireC => 60.0,
        TriggerShape::WireD | TriggerShape::Inder => 25.0,
        _ => 32.0,
    }
}

/// Which builtin mesh corresponds to each shape (`trigger.cpp:411`).
fn mesh_for(shape: &TriggerShape) -> Option<&'static str> {
    match shape {
        TriggerShape::WireA | TriggerShape::WireB | TriggerShape::WireC => Some("triggerSimple"),
        TriggerShape::WireD => Some("triggerDWire"),
        TriggerShape::Inder => Some("triggerInder"),
        TriggerShape::Button => Some("triggerButton"),
        TriggerShape::Star => Some("triggerStar"),
        TriggerShape::None => None,
    }
}

/// Whether the shape is a wire one, and therefore carries a thickness.
fn is_wire(shape: &TriggerShape) -> bool {
    matches!(
        shape,
        TriggerShape::WireA
            | TriggerShape::WireB
            | TriggerShape::WireC
            | TriggerShape::WireD
            | TriggerShape::Inder
    )
}

/// Builds a trigger's mesh.
///
/// The vertices are left in a **local** frame centered on the trigger, and the
/// translation to the world goes in `transform`. It is not for tidiness: the
/// trigger sinks when the ball goes over it, and that sinking is exactly a
/// translation in z. Leaving it alive, animating it means changing one number in
/// the matrix instead of regenerating the mesh every frame — which is what the
/// original does.
pub fn build(t: &Trigger, base_z: f32) -> Option<Mesh> {
    if !t.is_visible {
        return None;
    }
    let m = meshes::get(mesh_for(&t.shape)?)?;

    // Two shapes come tilted out of the box.
    //
    // The original composes `RotateX * RotateZ` and applies it as `v * M` — the
    // row-vector convention — so the vertex turns **first** in X and then in Z.
    // In glam, which is column-vector, that is written the other way round.
    let rot = match &t.shape {
        TriggerShape::WireB => {
            Mat4::from_rotation_z(t.rotation.to_radians())
                * Mat4::from_rotation_x((-23.0f32).to_radians())
        }
        TriggerShape::WireC => {
            Mat4::from_rotation_z(t.rotation.to_radians())
                * Mat4::from_rotation_x(140.0f32.to_radians())
        }
        _ => Mat4::from_rotation_z(t.rotation.to_radians()),
    };

    // The button is drawn five units higher and the C wire nineteen lower. They
    // are the original's numbers, so each mesh rests where it belongs.
    let z_offset = match &t.shape {
        TriggerShape::Button => 5.0,
        TriggerShape::WireC => -19.0,
        _ => 0.0,
    };

    // The round ones scale by their radius, on all three axes. The outline ones
    // scale by their two factors and **not** in z: the wire is as tall as it is.
    let scale = match &t.shape {
        TriggerShape::Button | TriggerShape::Star => Vec3::splat(t.radius),
        _ => Vec3::new(t.scale_x, t.scale_y, 1.0),
    };
    let thickness = if is_wire(&t.shape) {
        t.wire_thickness.unwrap_or(0.0)
    } else {
        0.0
    };

    let vertices = m
        .vertices
        .iter()
        .map(|v| {
            let p = rot.transform_point3(Vec3::from(v.pos));
            let n = rot.transform_vector3(Vec3::from(v.normal));
            // The thickness is added **after** scaling and along the already
            // rotated normal, not before: with different scales in x and y it is
            // not the same thing, and doing it beforehand would deform the
            // wire's thickness depending on the axis.
            let pos = p * scale + n * thickness;
            Vertex {
                pos: pos.into(),
                normal: n.into(),
                uv: v.uv,
            }
        })
        .collect();

    Some(Mesh {
        name: t.name.clone(),
        vertices,
        indices: m.indices,
        transform: Mat4::from_translation(Vec3::new(t.center.x, t.center.y, base_z + z_offset)),
        image: String::new(),
        material: t.material.clone(),
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Builtin,
        additive: None,
        disable_lighting: 0.0,
    })
}
