//! The items that use the original's builtin meshes.
//!
//! Bumpers, targets, gates and spinners generate no geometry: the original
//! ships their shape in `src/meshes/*.h` and each item places it with a
//! transform of its own. Those transforms are here, one per item.
//!
//! The meshes live in [`crate::meshes`], converted into a binary blob.

use crate::geometry::{Mesh, MeshKind, Vertex};
use crate::meshes;
use vpin::vpx::gameitem::GameItemEnum;
use vpin::vpx::gameitem::hittarget::TargetType;
use vpw_math::{Mat4, Vec3};

/// Height of the surface an item sits on, **at that item's own position**.
///
/// Many items are not on the playfield but on a wall or a ramp, and the
/// original resolves the height by looking the surface up by name
/// (`PinTable::GetSurfaceHeight`, `pintable.cpp:5161`). Without this,
/// everything on a raised plastic shows up sunk into the floor.
///
/// The position is not decoration. A wall has one height and the answer does
/// not depend on where you ask, but **a ramp climbs**, and the original
/// forwards the question to [`ramp_surface_height`] precisely because "how high
/// is the ramp" only has an answer at a point. F-14 makes the difference
/// visible: its popper `sw24a` sits on `Ramp4`, which runs from 150 down to
/// 112, and the kicker is up at the 150 end. Answering with the far end put the
/// ball it creates inside the ramp's own floor, where the next step shoved it
/// off the ramp entirely and it ended up rolling into a corner of the top arch
/// and jamming there.
pub fn surface_height(items: &[GameItemEnum], name: &str, x: f32, y: f32) -> f32 {
    if name.is_empty() {
        return 0.0;
    }
    items
        .iter()
        .find_map(|i| match i {
            GameItemEnum::Wall(w) if w.name.eq_ignore_ascii_case(name) => Some(w.height_top),
            GameItemEnum::Ramp(r) if r.name.eq_ignore_ascii_case(name) => {
                Some(ramp_surface_height(r, x, y))
            }
            _ => None,
        })
        .unwrap_or(0.0)
}

/// How high a ramp is over a given point (`Ramp::GetSurfaceHeight`,
/// `ramp.cpp:494`).
///
/// It finds the closest point on the ramp's central curve, measures how far
/// along the curve that is, and interpolates the climb by that fraction. The
/// interpolation is by **arc length**, not by index: a curve is sampled more
/// finely where it bends, so counting vertices would make the tight parts climb
/// faster than the straight ones.
///
/// Returns zero when the point is not over the ramp at all, which is what the
/// original returns and is honest: an item that names a ramp it does not stand
/// on is on the playfield.
pub fn ramp_surface_height(ramp: &vpin::vpx::gameitem::ramp::Ramp, x: f32, y: f32) -> f32 {
    // The original leaves the accuracy at its default here, which for a ramp
    // that is not see-through resolves to the finest the code allows
    // (`ramp.h:157`, then `4 * 10^((10-10)/1.5)` = 4). The curve is only used
    // to measure distance along the path, so finer is strictly closer to the
    // real one.
    let curve = crate::dragpoint::expand(
        &crate::dragpoint::from_vpin(&ramp.drag_points),
        false,
        crate::dragpoint::ACCURACY,
    );
    if curve.len() < 2 {
        return 0.0;
    }

    let Some((seg, at)) = closest_point_on_polyline(&curve, x, y) else {
        return 0.0; // not over this ramp
    };

    let mut total = 0.0f32;
    let mut start = 0.0f32;
    for i in 1..curve.len() {
        let len = (curve[i].pos.truncate() - curve[i - 1].pos.truncate()).length();
        if i <= seg {
            start += len;
        }
        total += len;
    }
    // How far the point sits between the two nearest vertices. Matters mostly
    // on the straight stretches, where one segment can be long.
    start += (at - curve[seg].pos.truncate()).length();

    if total <= 0.0 {
        return ramp.height_bottom;
    }
    curve[seg].pos.z + (start / total) * (ramp.height_top - ramp.height_bottom) + ramp.height_bottom
}

/// The point of a polyline closest to `(x, y)`, and which segment it is on.
///
/// Port of `ClosestPointOnPolygon` for the open case (`MeshUtils.h:364`). The
/// perpendicular foot has to land **on** the segment and not out past its ends,
/// which is why a nearer infinite line can still lose to a further segment; the
/// original allows a tenth of a unit of slack at each end so a foot that lands
/// exactly on a joint is not rejected by rounding.
fn closest_point_on_polyline(
    curve: &[crate::dragpoint::Point],
    x: f32,
    y: f32,
) -> Option<(usize, vpw_math::Vec2)> {
    let mut best = f32::MAX;
    let mut found = None;

    for i in 0..curve.len() - 1 {
        let p1 = curve[i].pos.truncate();
        let p2 = curve[i + 1].pos.truncate();

        let a = p1.y - p2.y;
        let b = p2.x - p1.x;
        let c = -(a * p1.x + b * p1.y);

        let denom = (a * a + b * b).sqrt();
        if denom == 0.0 {
            continue;
        }
        let dist = (a * x + b * y + c).abs() / denom;
        if dist >= best {
            continue;
        }

        // Where the perpendicular from the point meets the line.
        let d = -b;
        let f = -(d * x + a * y);
        let det = a * a - b * d;
        let inv_det = if det != 0.0 { 1.0 / det } else { 0.0 };
        let at = vpw_math::Vec2::new((b * f - a * c) * inv_det, (c * d - a * f) * inv_det);

        if at.x >= p1.x.min(p2.x) - 0.1
            && at.x <= p1.x.max(p2.x) + 0.1
            && at.y >= p1.y.min(p2.y) - 0.1
            && at.y <= p1.y.max(p2.y) + 0.1
        {
            best = dist;
            found = Some((i, at));
        }
    }
    found
}

/// Applies a transform to a builtin mesh and returns it ready to go.
fn place(
    mesh_name: &str,
    name: String,
    transform: Mat4,
    material: String,
    image: String,
    kind: MeshKind,
) -> Option<Mesh> {
    let m = meshes::get(mesh_name)?;
    Some(Mesh {
        name,
        vertices: m.vertices,
        indices: m.indices,
        transform,
        image,
        material,
        visible: true,
        kind,
    })
}

/// Bumper: base, cap, ring and socket, each with its own material and height.
///
/// `Bumper::RenderSetup` (`bumper.cpp:224-228`) and the four copies of
/// `bumper.cpp:536-610`. The scale in the plane is the **radius**, the vertical
/// one is `heightScale`, and each part starts at a different height.
pub fn bumper(b: &vpin::vpx::gameitem::bumper::Bumper, base_z: f32) -> Vec<Mesh> {
    let matrix = |z: f32| {
        Mat4::from_translation(Vec3::new(b.center.x, b.center.y, z))
            * Mat4::from_rotation_z(b.orientation.to_radians())
            * Mat4::from_scale(Vec3::new(b.radius, b.radius, b.height_scale))
    };

    let mut meshes = Vec::new();
    if b.is_base_visible {
        meshes.extend(place(
            "bumperBase",
            format!("{} (base)", b.name),
            matrix(base_z),
            b.base_material.clone(),
            String::new(),
            MeshKind::Builtin,
        ));
    }
    if b.is_socket_visible.unwrap_or(true) {
        // The socket goes five units higher than the base.
        meshes.extend(place(
            "bumperSocket",
            format!("{} (socket)", b.name),
            matrix(base_z + 5.0),
            b.socket_material.clone(),
            String::new(),
            MeshKind::Builtin,
        ));
    }
    if b.is_ring_visible.unwrap_or(true) {
        meshes.extend(place(
            "bumperRing",
            format!("{} (ring)", b.name),
            matrix(base_z),
            b.ring_material.clone().unwrap_or_default(),
            String::new(),
            MeshKind::Builtin,
        ));
    }
    if b.is_cap_visible {
        // The cap starts right at the top: `heightScale + baseHeight`.
        meshes.extend(place(
            "bumperCap",
            format!("{} (cap)", b.name),
            matrix(base_z + b.height_scale),
            b.cap_material.clone(),
            String::new(),
            MeshKind::Builtin,
        ));
    }
    meshes
}

/// Which mesh each target type gets (`hittarget.cpp:41-95`).
fn target_mesh(t: &TargetType) -> &'static str {
    match t {
        TargetType::DropTargetBeveled => "hitTargetT2",
        TargetType::DropTargetSimple => "hitTargetT3",
        TargetType::DropTargetFlatSimple => "hitTargetT4",
        TargetType::HitTargetRound => "hitTargetRound",
        TargetType::HitTargetRectangle => "hitTargetRectangle",
        TargetType::HitFatTargetRectangle => "hitFatTargetRectangle",
        TargetType::HitFatTargetSquare => "hitFatTargetSquare",
        TargetType::HitTargetSlim => "hitTargetT1Slim",
        TargetType::HitFatTargetSlim => "hitTargetT2Slim",
    }
}

/// Where a target stands: scale on each axis, turn about Z, then position.
/// `hittarget.cpp:394`.
pub fn target_transform(t: &vpin::vpx::gameitem::hittarget::HitTarget) -> Mat4 {
    Mat4::from_translation(Vec3::new(t.position.x, t.position.y, t.position.z))
        * Mat4::from_rotation_z(t.rot_z.to_radians())
        * Mat4::from_scale(Vec3::new(t.size.x, t.size.y, t.size.z))
}

/// A target's mesh in world space, for building colliders out of.
///
/// Deliberately not gated on the target being visible. An invisible target
/// still stops the ball — the original checks visibility when it *animates* a
/// drop target and says so in a comment, because old tables rely on hidden
/// drop targets never dropping, but it never checks it when it builds the
/// colliders (`hittarget.cpp:220`).
pub fn target_collision_mesh(
    t: &vpin::vpx::gameitem::hittarget::HitTarget,
) -> Option<(Vec<Vec3>, Vec<usize>)> {
    if !t.is_collidable {
        return None;
    }
    let mesh = crate::meshes::get(target_mesh(&t.target_type))?;
    let transform = target_transform(t);
    Some((
        mesh.vertices
            .iter()
            .map(|v| transform.transform_point3(Vec3::new(v.pos[0], v.pos[1], v.pos[2])))
            .collect(),
        mesh.indices.iter().map(|&i| i as usize).collect(),
    ))
}

/// Target: per-axis scale, turn in Z and position (`hittarget.cpp:235, 277-285`).
pub fn hit_target(t: &vpin::vpx::gameitem::hittarget::HitTarget) -> Option<Mesh> {
    if !t.is_visible {
        return None;
    }
    let transform = target_transform(t);

    place(
        target_mesh(&t.target_type),
        t.name.clone(),
        transform,
        t.material.clone(),
        t.image.clone(),
        MeshKind::Builtin,
    )
}

/// Gate: the wire plus, if it is there, the bracket.
///
/// The length scales the mesh in the plane; the height raises it.
pub fn gate(g: &vpin::vpx::gameitem::gate::Gate, base_z: f32) -> Vec<Mesh> {
    if !g.is_visible {
        return Vec::new();
    }
    use vpin::vpx::gameitem::gate::GateType;

    let mesh = match g.gate_type.as_ref().unwrap_or(&GateType::WireW) {
        GateType::WireW => "gateWire",
        GateType::WireRectangle => "gateWireRectangle",
        GateType::Plate => "gatePlate",
        GateType::LongPlate => "gateLongPlate",
        // A type we do not know about: better the W wire, which is the most
        // common one, than drawing nothing at all.
        GateType::Unknown(_) => "gateWire",
    };

    let transform = Mat4::from_translation(Vec3::new(g.center.x, g.center.y, g.height + base_z))
        * Mat4::from_rotation_z(g.rotation.to_radians())
        * Mat4::from_scale(Vec3::splat(g.length));

    let mut meshes: Vec<Mesh> = place(
        mesh,
        g.name.clone(),
        transform,
        g.material.clone(),
        String::new(),
        MeshKind::Builtin,
    )
    .into_iter()
    .collect();

    if g.show_bracket {
        meshes.extend(place(
            "gateBracket",
            format!("{} (bracket)", g.name),
            transform,
            g.material.clone(),
            String::new(),
            MeshKind::Builtin,
        ));
    }
    meshes
}

/// Spinner: the plate plus, if it is there, the bracket.
pub fn spinner(s: &vpin::vpx::gameitem::spinner::Spinner, base_z: f32) -> Vec<Mesh> {
    if !s.is_visible {
        return Vec::new();
    }
    let transform = Mat4::from_translation(Vec3::new(s.center.x, s.center.y, s.height + base_z))
        * Mat4::from_rotation_z(s.rotation.to_radians())
        * Mat4::from_scale(Vec3::splat(s.length));

    let mut meshes: Vec<Mesh> = place(
        "spinnerPlate",
        s.name.clone(),
        transform,
        s.material.clone(),
        s.image.clone(),
        MeshKind::Builtin,
    )
    .into_iter()
    .collect();

    if s.show_bracket {
        meshes.extend(place(
            "spinnerBracket",
            format!("{} (bracket)", s.name),
            transform,
            s.material.clone(),
            String::new(),
            MeshKind::Builtin,
        ));
    }
    meshes
}

/// Kicker: the visible part of the hole.
pub fn kicker(k: &vpin::vpx::gameitem::kicker::Kicker, base_z: f32) -> Option<Mesh> {
    use vpin::vpx::gameitem::kicker::KickerType;

    // The invisible type draws nothing, which is what almost every table uses
    // for the kickers in the lanes.
    let mesh = match k.kicker_type {
        KickerType::Invisible => return None,
        KickerType::Hole => "kickerHole",
        KickerType::HoleSimple => "kickerSimpleHole",
        KickerType::Cup | KickerType::Cup2 => "kickerCup",
        KickerType::Williams => "kickerWilliams",
        KickerType::Gottlieb => "kickerGottlieb",
    };

    let transform = Mat4::from_translation(Vec3::new(k.center.x, k.center.y, base_z))
        * Mat4::from_rotation_z(k.orientation.to_radians())
        * Mat4::from_scale(Vec3::splat(k.radius));

    place(
        mesh,
        k.name.clone(),
        transform,
        k.material.clone(),
        String::new(),
        MeshKind::Builtin,
    )
}

/// Converts a builtin mesh to vertices, in case it needs inspecting.
pub fn vertices_of(name: &str) -> Option<Vec<Vertex>> {
    meshes::get(name).map(|m| m.vertices)
}
