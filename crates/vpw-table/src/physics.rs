//! From the table to the collision shapes.
//!
//! What is drawn and what stops the ball **are not the same thing**, and the
//! original keeps them separate on purpose: a wall's visual geometry is
//! textured quads, and its collision geometry is segments, corners and a lid. A
//! wire ramp is drawn as tubes and collides as two walls.
//!
//! This module builds the second one. Port of each piece's `PhysicSetup`.

use crate::builtin::surface_height;
use crate::dragpoint;
use vpin::vpx::VPX;
use vpin::vpx::gameitem::GameItemEnum;
use vpin::vpx::gameitem::trigger::TriggerShape;
use vpw_math::{Quat, Vec2, Vec3};
use vpw_physics::collision::Material as PhysMaterial;
use vpw_physics::constants::PHYS_SKIN;
use vpw_physics::engine::Shape;
use vpw_physics::flipper::{Flipper, FlipperParams};
use vpw_physics::parts::{Bumper, Gate, Kicker, PivotAxis, Spinner};
use vpw_physics::plunger::{Plunger, PlungerParams};
use vpw_physics::shapes::{
    HitCircle, HitLine3D, HitLineZ, HitPlane, HitPoint, HitPoly, HitTriangle, LineSeg, Slingshot,
};
use vpw_physics::trigger::{Trigger, Zone};

/// The physical properties of a piece.
///
/// Each piece can either use those of a named material or override them with
/// its own (`Surface::SetupHitObject`, `surface.cpp:360`).
fn material(
    vpx: &VPX,
    material_name: Option<&str>,
    overrides: bool,
    own: (f32, f32, f32, f32),
) -> PhysMaterial {
    if overrides {
        let (elasticity, elasticity_falloff, friction, scatter) = own;
        return PhysMaterial {
            elasticity,
            elasticity_falloff,
            friction,
            scatter: scatter.to_radians(),
        };
    }
    find_material(vpx, material_name.unwrap_or(""))
}

/// Looks up a physics material by name, case-insensitively.
fn find_material(vpx: &VPX, name: &str) -> PhysMaterial {
    if let Some(modern) = &vpx.gamedata.materials
        && let Some(m) = modern.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    {
        return PhysMaterial {
            elasticity: m.elasticity,
            elasticity_falloff: m.elasticity_falloff,
            friction: m.friction,
            scatter: m.scatter_angle.to_radians(),
        };
    }
    if let Some(old) = &vpx.gamedata.materials_physics_old
        && let Some(m) = old
            .iter()
            .zip(vpx.gamedata.materials_old.iter())
            .find(|(_, n)| n.name.eq_ignore_ascii_case(name))
            .map(|(p, _)| p)
    {
        return PhysMaterial {
            elasticity: m.elasticity,
            elasticity_falloff: m.elasticity_falloff,
            friction: m.friction,
            scatter: m.scatter_angle.to_radians(),
        };
    }
    // With no material declared, the values **the table** carries. They are not
    // constants of ours: every `.vpx` stores its own friction, elasticity and
    // scatter, and those are what define how it feels to roll across that
    // playfield (`pintable.cpp:3835`).
    PhysMaterial {
        elasticity: vpx.gamedata.elasticity,
        elasticity_falloff: vpx.gamedata.elastic_falloff,
        friction: vpx.gamedata.friction,
        scatter: vpx.gamedata.scatter.to_radians(),
    }
}

/// The collision shapes of a table, and which game item each one came from.
///
/// The second half is what makes a script's `Bumper1_Hit` possible. One item
/// becomes many shapes — a wall is a ring of segments plus a joint at every
/// corner plus a lid — so "the ball hit shape 1174" says nothing on its own.
/// `owners[i]` is the index into `vpx.gameitems` that produced shape `i`, or
/// `None` for the two the table itself owns: the playfield and the glass.
pub struct Collision {
    pub shapes: Vec<Shape>,
    pub owners: Vec<Option<usize>>,
}

impl Collision {
    /// The shapes a given game item produced.
    pub fn shapes_of(&self, item: usize) -> impl Iterator<Item = usize> + '_ {
        self.owners
            .iter()
            .enumerate()
            .filter(move |(_, o)| **o == Some(item))
            .map(|(i, _)| i)
    }
}

/// All the collision shapes of a table, with no record of where they came from.
///
/// Includes the playfield and the glass on top, which the original always tests
/// and which therefore do not go into the tree.
pub fn build(vpx: &VPX) -> Vec<Shape> {
    build_with_owners(vpx).shapes
}

/// The same, keeping track of which game item produced each shape.
pub fn build_with_owners(vpx: &VPX) -> Collision {
    let g = &vpx.gamedata;
    let mut shapes = Vec::new();

    // The playfield, with the table's material.
    let pf = find_material(vpx, &g.playfield_material);
    shapes.push(Shape::Plane(HitPlane {
        normal: Vec3::Z,
        d: 0.0,
        material: pf,
    }));

    // The glass on top, facing down, so the ball does not escape upwards on a
    // very steep ramp. The height of the top edge works as a reference.
    let height = g.glass_top_height.max(100.0);
    shapes.push(Shape::Plane(HitPlane {
        normal: -Vec3::Z,
        d: -height,
        material: PhysMaterial {
            elasticity: 0.2,
            friction: 0.3,
            ..PhysMaterial::default()
        },
    }));

    // The cabinet's four sides (`PhysicsEngine.cpp:174-178`).
    //
    // These are the outermost thing on the table and the only colliders the
    // original builds without a game item behind them — its own comment says
    // so. A table normally draws its own perimeter well inside them, which is
    // why leaving them out is invisible until the day a ball gets past that
    // perimeter: then there is nothing at all between it and the edge of the
    // world.
    //
    // The winding is the original's, vertex for vertex, because a `LineSeg` is
    // one-sided and reversing a pair would face it out of the cabinet instead
    // of into it. The top and bottom edges are the far and near ends of the
    // playfield, and take the glass height at their own end.
    let (l, r, t, b) = (g.left, g.right, g.top, g.bottom);
    let glass_top = g.glass_top_height;
    // Before Visual Pinball 10.8 the glass was horizontal and the file has no
    // second height; `pintable.cpp:2019-2022` copies the top down over it.
    let glass_bottom = g.glass_bottom_height.unwrap_or(glass_top);
    let cabinet = PhysMaterial::default();
    for (v1, v2, z_high) in [
        (Vec2::new(r, t), Vec2::new(r, b), glass_top),
        (Vec2::new(l, b), Vec2::new(l, t), glass_bottom),
        (Vec2::new(r, b), Vec2::new(l, b), glass_bottom),
        (Vec2::new(l, t), Vec2::new(r, t), glass_top),
    ] {
        shapes.push(Shape::Line(LineSeg::new(v1, v2, 0.0, z_high, cabinet)));
    }

    // The playfield, the glass and the cabinet belong to the table, not to any
    // item.
    let mut owners: Vec<Option<usize>> = vec![None; shapes.len()];

    for (index, item) in vpx.gameitems.iter().enumerate() {
        let before = shapes.len();
        match item {
            GameItemEnum::Wall(w) => wall(w, vpx, &mut shapes),
            GameItemEnum::Ramp(r) => ramp(r, vpx, &mut shapes),
            GameItemEnum::Rubber(r) => rubber(r, vpx, &mut shapes),
            GameItemEnum::Bumper(b) => bumper(b, vpx, &mut shapes),
            GameItemEnum::Kicker(k) => kicker(k, vpx, &mut shapes),
            GameItemEnum::Gate(g) => gate(g, vpx, &mut shapes),
            GameItemEnum::Spinner(sp) => spinner(sp, vpx, &mut shapes),
            GameItemEnum::Flipper(f) => flipper(f, vpx, &mut shapes),
            GameItemEnum::Plunger(p) => plunger(p, vpx, &mut shapes),
            GameItemEnum::HitTarget(t) => hit_target(t, vpx, &mut shapes),
            GameItemEnum::Primitive(p) => primitive(p, vpx, &mut shapes),
            _ => {}
        }
        owners.resize(shapes.len(), Some(index));
        debug_assert!(shapes.len() >= before);
    }

    Collision { shapes, owners }
}

/// A wall's shapes (`Surface::PhysicSetup`, `surface.cpp:304`).
///
/// A wall is not a segment: it is a ring of segments, plus a vertical joint at
/// each vertex so the ball does not slip through the corner, plus points at the
/// top and bottom, plus the lid. The original builds all five things.
fn wall(w: &vpin::vpx::gameitem::wall::Wall, vpx: &VPX, out: &mut Vec<Shape>) {
    if !w.is_collidable {
        return;
    }

    let points = dragpoint::expand(
        &dragpoint::from_vpin(&w.drag_points),
        true,
        dragpoint::ACCURACY,
    );
    if points.len() < 3 {
        return;
    }

    let m = material(
        vpx,
        w.physics_material.as_deref(),
        w.overwrite_physics.unwrap_or(false),
        (
            w.elasticity,
            w.elasticity_falloff.unwrap_or(0.0),
            w.friction,
            w.scatter,
        ),
    );
    let (bottom, top) = (w.height_bottom, w.height_top);
    let n = points.len();

    // Which control points are marked as slingshot. The mark lives on the
    // **starting** point of each stretch (`Surface::AddLine`,
    // `surface.cpp:404`), so it has to be carried from the original control
    // points to the already expanded ones.
    let control: Vec<bool> = w
        .drag_points
        .iter()
        .map(|d| d.is_slingshot.unwrap_or(false))
        .collect();
    let mut control_idx = 0usize;

    for i in 0..n {
        let p1 = points[i].pos.truncate();
        let p2 = points[(i + 1) % n].pos.truncate();

        // Every time we pass through a point the author placed, the index over
        // the control points advances.
        let is_sling = if points[i].control {
            let v = control.get(control_idx).copied().unwrap_or(false);
            control_idx += 1;
            v
        } else {
            false
        };

        // The side. If the stretch is marked, it is a slingshot: a wall with a
        // solenoid that shoves the ball back.
        let seg = LineSeg::new(p1, p2, bottom, top, m);
        if is_sling {
            out.push(Shape::Slingshot(Slingshot::new(
                seg,
                w.slingshot_force,
                w.slingshot_threshold,
            )));
        } else {
            out.push(Shape::Line(seg));
        }

        // The two horizontal edges of that side (`surface.cpp:428-434`).
        //
        // They are the rim where the flat top of a wall meets its vertical
        // face, and without them that rim is a gap: a ball arriving exactly
        // along it is above the side's z range and at the very border of the
        // lid, so neither shape claims it and the ball passes through into the
        // margin outside the playfield. The lower edge only exists on a wall
        // that is raised off the playfield, since one sitting on it has no
        // underside to hit.
        if let Some(edge) =
            HitLine3D::new(Vec3::new(p1.x, p1.y, top), Vec3::new(p2.x, p2.y, top), m)
        {
            out.push(Shape::Line3D(edge));
        }
        if bottom != 0.0
            && let Some(edge) = HitLine3D::new(
                Vec3::new(p1.x, p1.y, bottom),
                Vec3::new(p2.x, p2.y, bottom),
                m,
            )
        {
            out.push(Shape::Line3D(edge));
        }

        // The vertical joint at the vertex. A circle of radius zero is exactly
        // that, and it stops the ball from slipping through the corner between
        // two segments.
        out.push(Shape::Circle(HitCircle {
            center: p1,
            radius: 0.0,
            z_low: bottom,
            z_high: top,
            material: m,
        }));

        // And the ends of the edge.
        out.push(Shape::Point(HitPoint {
            p: Vec3::new(p1.x, p1.y, top),
            material: m,
        }));
        if bottom != 0.0 {
            out.push(Shape::Point(HitPoint {
                p: Vec3::new(p1.x, p1.y, bottom),
                material: m,
            }));
        }
    }

    // The lid: without it a wall is a shell and the ball falls inside.
    let lid: Vec<Vec3> = points
        .iter()
        .map(|p| Vec3::new(p.pos.x, p.pos.y, top))
        .collect();
    if let Some(poly) = HitPoly::new(lid, m) {
        out.push(Shape::Poly(poly));
    }

    // And the floor, if the wall is solid underneath.
    if w.is_bottom_solid {
        let floor: Vec<Vec3> = points
            .iter()
            .rev()
            .map(|p| Vec3::new(p.pos.x, p.pos.y, bottom))
            .collect();
        if let Some(poly) = HitPoly::new(floor, m) {
            out.push(Shape::Poly(poly));
        }
    }
}

/// How many sides a rubber's cross-section has (`GenerateMesh(6, true)`,
/// `rubber.cpp:536`). The original's comment warns that the two flattened
/// vertices below are hard-coded to this number.
const RUBBER_SEGMENTS: usize = 6;

/// A rubber's shapes (`Rubber::PhysicSetup`, `rubber.cpp:528`).
///
/// A rubber is a **tube**, not a line. The original sweeps a hexagon of radius
/// `thickness / 2` along the spline and gives the physics one `HitTriangle` per
/// face, one `HitLine3D` per edge and a `HitPoint` per vertex — solid from
/// every direction, and standing off the centreline by half its thickness on
/// each side.
///
/// A ring of one-sided segments *on* the centreline looks similar and is not:
/// it has no thickness to hit, so a ball reaches half a rubber further in than
/// it should before anything stops it; and being one-sided, it is transparent
/// from inside its own loop, which is where a ball that got in is trying to get
/// out.
///
/// Two of the six vertices are squashed to 60% of their height
/// (`rubber.cpp:1299`) — the original calls it a hack and does it so the
/// collision shape is more of a rectangle than a smooth tube.
fn rubber(r: &vpin::vpx::gameitem::rubber::Rubber, vpx: &VPX, out: &mut Vec<Shape>) {
    if !r.is_collidable {
        return;
    }
    // The coarse curve, as the original's three collision paths do
    // (`rubber.cpp:162`, `:184`, `:247`); the mesh uses the fine one.
    let points = dragpoint::expand(
        &dragpoint::from_vpin(&r.drag_points),
        true,
        dragpoint::collision_accuracy(),
    );
    if points.len() < 3 {
        return;
    }

    let m = material(
        vpx,
        r.physics_material.as_deref(),
        r.overwrite_physics.unwrap_or(false),
        (r.elasticity, r.elasticity_falloff, r.friction, r.scatter),
    );

    let height = r.hit_height.unwrap_or(r.height);
    let half = r.thickness as f32 * 0.5;
    let rings = points.len();

    // The ring vertices, swept along the spline. The frame is carried from one
    // ring to the next rather than recomputed, so the hexagon does not spin
    // around the tube as the curve turns (`rubber.cpp:1269-1290`).
    let mut verts: Vec<Vec3> = Vec::with_capacity(rings * RUBBER_SEGMENTS);
    let mut prev_binormal = Vec3::ZERO;
    for i in 0..rings {
        let a = points[i].pos.truncate();
        let b = points[(i + 1) % rings].pos.truncate();
        let tangent = Vec3::new(b.x - a.x, b.y - a.y, 0.0);

        let (normal, binormal) = if i == 0 {
            let up = Vec3::new(b.x + a.x, b.y + a.y, height * 2.0);
            let normal = tangent.cross(up);
            (normal, tangent.cross(normal))
        } else {
            let normal = prev_binormal.cross(tangent);
            (normal, tangent.cross(normal))
        };
        let normal = normal.normalize_or_zero();
        prev_binormal = binormal.normalize_or_zero();
        let axis = tangent.normalize_or_zero();

        for j in 0..RUBBER_SEGMENTS {
            let angle = (j as f32) * std::f32::consts::TAU / RUBBER_SEGMENTS as f32;
            let mut offset = Quat::from_axis_angle(axis, angle) * normal * half;
            if j == 0 || j == 3 {
                offset.z *= 0.6;
            }
            verts.push(Vec3::new(a.x + offset.x, a.y + offset.y, height + offset.z));
        }
    }

    // The faces, and from them the triangles, the edges and the points.
    let mut edges: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for i in 0..rings {
        let next_ring = (i + 1) % rings;
        for j in 0..RUBBER_SEGMENTS {
            let next = (j + 1) % RUBBER_SEGMENTS;
            let quad = [
                i * RUBBER_SEGMENTS + j,
                i * RUBBER_SEGMENTS + next,
                next_ring * RUBBER_SEGMENTS + j,
                next_ring * RUBBER_SEGMENTS + next,
            ];
            // `rubber.cpp:1339-1344`, and then read back three at a time.
            for tri in [[quad[0], quad[1], quad[2]], [quad[3], quad[2], quad[1]]] {
                // The mesh is wound clockwise and a `HitTriangle` wants the
                // other way round, so the last two swap (`rubber.cpp:543`).
                let v = [verts[tri[0]], verts[tri[2]], verts[tri[1]]];
                if let Some(t) = HitTriangle::new(v, m) {
                    out.push(Shape::Triangle(t));
                }
                for (x, y) in [(tri[0], tri[2]), (tri[2], tri[1]), (tri[1], tri[0])] {
                    let key = (x.min(y), x.max(y));
                    if edges.insert(key)
                        && let Some(l) = HitLine3D::new(verts[x], verts[y], m)
                    {
                        out.push(Shape::Line3D(l));
                    }
                }
            }
        }
    }
    for v in verts {
        out.push(Shape::Point(HitPoint { p: v, material: m }));
    }
}

/// A reasonable spot to drop a ball: right at the top, in the plunger lane if
/// that can be worked out.
pub fn ball_launch_point(vpx: &VPX) -> Vec3 {
    let g = &vpx.gamedata;
    // The plunger lane sits against the right edge; if the table declares one,
    // its position is used.
    for item in &vpx.gameitems {
        if let GameItemEnum::Plunger(p) = item {
            return Vec3::new(p.center.x, p.center.y - 100.0, 30.0);
        }
    }
    Vec3::new(g.right * 0.9, g.bottom * 0.85, 30.0)
}

/// A bumper's circle, with its kick (`Bumper::PhysicSetup`).
fn bumper(b: &vpin::vpx::gameitem::bumper::Bumper, vpx: &VPX, out: &mut Vec<Shape>) {
    if !b.is_collidable.unwrap_or(true) {
        return;
    }
    let base_z = surface_height(&vpx.gameitems, &b.surface, b.center.x, b.center.y);
    // `Bumper::PhysicSetup` (`bumper.cpp:204`) sets exactly one physical number
    // on the circle it builds, `m_scatter = ANGTORAD(m_d.m_scatter)`, and leaves
    // the rest at the collider's defaults. Reading the table's material for the
    // other three is the port's own choice and is kept; dropping `BSCT` on the
    // floor was not a choice — a bumper is where a table asks for the most
    // scatter it asks for anywhere, because a ball that came off one along a
    // repeatable line would make the same shot land the same way every time.
    let m = PhysMaterial {
        scatter: b.scatter.unwrap_or(0.0).to_radians(),
        ..find_material(vpx, "")
    };

    let circle = HitCircle {
        center: Vec2::new(b.center.x, b.center.y),
        radius: b.radius,
        z_low: base_z,
        z_high: base_z + b.height_scale,
        material: m,
    };
    let mut bumper = Bumper::new(circle, b.force, b.threshold);
    // The bumper's own `HAHE`. It decides whether the coil fires at all, not
    // only whether the script hears about it — see `Bumper::hit_event`. The
    // editor's default is on, which is also what a file too old to carry the
    // tag was saved with (`Settings_properties.inl:917`).
    bumper.hit_event = b.hit_event.unwrap_or(true);
    // How far the ring drops on a hit: whatever the table declares plus half
    // the bumper's height. The speed comes in units per millisecond.
    bumper.ring_speed = b.ring_speed;
    bumper.ring_drop = b.ring_drop_offset.unwrap_or(0.0) + b.height_scale * 0.5;
    out.push(Shape::Bumper(bumper));
}

/// A kicker's circle (`Kicker::PhysicSetup`, `kicker.cpp:152`).
///
/// Two things you would not guess:
///
/// - The cylinder goes up to `hit_height`, **not** to the radius. They are
///   different things: the radius is how much the hole takes up in the plane
///   and the height is how far up it grabs.
/// - In legacy mode the collision radius **shrinks** — to 0.6, or to 0.75 if the
///   ball falls through — because there only the kicker's inner circle fires.
///   In modern mode the full radius is used.
fn kicker(k: &vpin::vpx::gameitem::kicker::Kicker, vpx: &VPX, out: &mut Vec<Shape>) {
    // A kicker that is switched off still **exists**. The original always
    // builds the hit circle and puts the flag on it —
    // `phitcircle->m_enabled = m_d.m_enabled` (`kicker.cpp:184`) — and
    // `CreateSizedBallWithMass` asks only whether the circle is there
    // (`kicker.cpp:657`), not whether it is on.
    //
    // Skipping it entirely, which is what this did, means a switched-off
    // kicker cannot make a ball and cannot be switched on later either. That
    // is not a corner: a ball trough's release kicker is normally saved off,
    // precisely so it does not catch balls rolling past, and a table uses it
    // as nothing but a ball factory. The machine starts a game, knocks four
    // times trying to serve, and gives you nothing.
    //
    // Whether it is on is set from the file where the items are built, through
    // the same switch a script uses.
    let base_z = surface_height(&vpx.gameitems, &k.surface, k.center.x, k.center.y);
    let factor = if k.legacy_mode {
        if k.fall_through { 0.75 } else { 0.6 }
    } else {
        1.0
    };
    let height = k.hit_height.unwrap_or(40.0);

    let circle = HitCircle {
        center: Vec2::new(k.center.x, k.center.y),
        radius: k.radius * factor,
        z_low: base_z,
        z_high: base_z + height,
        material: find_material(vpx, &k.material),
    };
    out.push(Shape::Kicker(
        Kicker::new(circle, k.hit_accuracy, k.legacy_mode).with_fall_through(k.fall_through),
    ));
}

/// A gate (`Gate::PhysicSetup`, `gate.cpp:291`).
///
/// The gate itself lets things through: the ball crosses it and only makes it
/// swing. What stops it from coming back the way it came, when the gate is
/// **one-way**, is a separate rigid segment at the height of a ball. Without it
/// a one-way gate would not stop anything.
///
/// And if it has a bracket, two finite posts at the ends: the ball can hit them
/// if the frame ends up low enough.
///
/// The gate's angles come in **radians** in the `.vpx` (the spinner's come in
/// degrees; it is an asymmetry of the format, not an oversight).
fn gate(g: &vpin::vpx::gameitem::gate::Gate, vpx: &VPX, out: &mut Vec<Shape>) {
    // A gate that is not collidable still **exists**, and building nothing for
    // it was wrong twice over. `Gate::PhysicSetup` (`gate.cpp:322-333`) always
    // creates the leaf and, on a one-way gate, the blocking segment beside it;
    // the flag only ends up on the leaf, as `m_phitgate->m_enabled =
    // m_d.m_collidable`. A script that later writes `Collidable = True` — or
    // `Open = False`, which restores the leaf to that same value — has
    // something to switch back on. Skipping the build left it with nothing, and
    // left the drawing code, which pairs meshes with shapes by order, one short
    // for every gate after it.
    let base_z = surface_height(&vpx.gameitems, &g.surface, g.center.x, g.center.y);
    let (sn, cs) = g.rotation.to_radians().sin_cos();
    let half = g.length * 0.5;
    let center = Vec2::new(g.center.x, g.center.y);

    if !g.two_way {
        let tip = Vec2::new(cs * (half + PHYS_SKIN), sn * (half + PHYS_SKIN));
        out.push(Shape::Line(LineSeg::new(
            center + tip,
            center - tip,
            base_z,
            base_z + 2.0 * PHYS_SKIN,
            material(
                vpx,
                Some(&g.material),
                true,
                (g.elasticity, 0.0, g.friction, 0.0),
            ),
        )));
    }

    let mut leaf = Gate::new(
        PivotAxis {
            center,
            length: g.length,
            rotation_deg: g.rotation,
            height: g.height,
            z_low: base_z,
            angle_min: g.angle_min,
            angle_max: g.angle_max,
            damping: g.damping.unwrap_or(0.985),
        },
        g.gravity_factor.unwrap_or(0.25),
        g.two_way,
    );
    // The file's `GCOL`. It is kept beside `enabled` rather than folded into it
    // because `Open = False` puts the leaf back to *this* and not to whatever a
    // script last wrote (`gate.cpp:753`). Note that the blocking segment above
    // is left switched on either way: the original never gives it the flag at
    // build time, only `put_Collidable` and `put_Open` ever touch it. It is a
    // strange asymmetry to copy, and copying it is the point — a table that
    // ships a non-collidable one-way gate is relying on the ball still not
    // getting back through it.
    leaf.collidable = g.is_collidable;
    leaf.enabled = g.is_collidable;
    out.push(Shape::Gate(leaf));

    if g.show_bracket {
        for side in [1.0, -1.0] {
            let tip = Vec2::new(cs * half * side, sn * half * side);
            out.push(Shape::Circle(HitCircle {
                center: center + tip,
                radius: 0.01,
                z_low: base_z,
                z_high: base_z + g.height,
                material: find_material(vpx, &g.material),
            }));
        }
    }
}

/// A spinner (`Spinner::PhysicSetup`, `spinner.cpp:182`).
///
/// Unlike the gate, it does not carry a rigid segment: a spinner never stops the
/// ball, it only spins. If it has a bracket, the two posts are genuinely thick
/// — 7.5% of the length — and they start at the height of the plate, not at the
/// floor.
///
/// Its angles come in **degrees**, the other way around from the gate's.
fn spinner(sp: &vpin::vpx::gameitem::spinner::Spinner, vpx: &VPX, out: &mut Vec<Shape>) {
    let base_z = surface_height(&vpx.gameitems, &sp.surface, sp.center.x, sp.center.y);
    let center = Vec2::new(sp.center.x, sp.center.y);

    out.push(Shape::Spinner(Spinner::new(
        PivotAxis {
            center,
            length: sp.length,
            rotation_deg: sp.rotation,
            height: sp.height,
            z_low: base_z,
            angle_min: sp.angle_min.to_radians(),
            angle_max: sp.angle_max.to_radians(),
            damping: sp.damping,
        },
        sp.elasticity,
    )));

    if sp.show_bracket {
        // The extra 0.1875 leaves the posts outside the plate.
        let half = sp.length * 0.5 + sp.length * 0.1875;
        let (sn, cs) = sp.rotation.to_radians().sin_cos();
        for side in [1.0, -1.0] {
            out.push(Shape::Circle(HitCircle {
                center: center + Vec2::new(cs * half * side, sn * half * side),
                radius: sp.length * 0.075,
                z_low: base_z + sp.height,
                z_high: base_z + sp.height + 30.0,
                material: find_material(vpx, &sp.material),
            }));
        }
    }
}

/// A flipper (`Flipper::PhysicSetup`, `flipper.cpp:...`).
///
/// The length the physics uses comes from `flipper_radius_max`. The original
/// interpolates it with `flipper_radius_min` according to the table's global
/// difficulty, which is a player setting; with difficulty at zero — the default
/// — the maximum is what is left, and that is what is used here.
fn flipper(f: &vpin::vpx::gameitem::flipper::Flipper, vpx: &VPX, out: &mut Vec<Shape>) {
    if !f.is_enabled {
        return;
    }
    let base_z = surface_height(&vpx.gameitems, &f.surface, f.center.x, f.center.y);
    let d = FlipperParams::default();

    out.push(Shape::Flipper(Flipper::new(FlipperParams {
        center: Vec2::new(f.center.x, f.center.y),
        base_radius: f.base_radius,
        end_radius: f.end_radius,
        flipper_radius: f.flipper_radius_max,
        z_low: base_z,
        z_high: base_z + f.height,
        angle_start: f.start_angle.to_radians(),
        angle_end: f.end_angle.to_radians(),
        mass: f.mass,
        strength: f.strength,
        return_ratio: f.return_,
        ramp_up: f.ramp_up,
        torque_damping: f.torque_damping.unwrap_or(d.torque_damping),
        // Degrees in the file, radians everywhere here — the same conversion
        // the two angles above get. The fallback is already in radians.
        torque_damping_angle: f
            .torque_damping_angle
            .map(f32::to_radians)
            .unwrap_or(d.torque_damping_angle),
        material: PhysMaterial {
            elasticity: f.elasticity,
            elasticity_falloff: f.elasticity_falloff,
            friction: f.friction,
            scatter: f.scatter.unwrap_or(0.0).to_radians(),
        },
    })));
}

/// The zones that report back from a table (`Trigger::PhysicSetup`,
/// `trigger.cpp`).
///
/// They go apart from the collision shapes because a trigger **collides with
/// nothing**: there is no point putting it in the collision tree nor asking it
/// when it is going to touch the ball. You ask it whether the ball is inside,
/// and that is something else.
///
/// The star and button ones are circles and the `.vpx` gives them by radius. The
/// wire ones come as an outline of drag points, just like a wall, and have to be
/// expanded with the same spline.
pub fn triggers(vpx: &VPX) -> Vec<Trigger> {
    let mut out = Vec::new();
    for item in &vpx.gameitems {
        let GameItemEnum::Trigger(t) = item else {
            continue;
        };
        if !t.is_enabled {
            continue;
        }
        let base_z = surface_height(&vpx.gameitems, &t.surface, t.center.x, t.center.y);

        let (area, height) = match t.shape {
            TriggerShape::Star | TriggerShape::Button => (
                Zone::Circle {
                    center: Vec2::new(t.center.x, t.center.y),
                    radius: t.radius,
                },
                t.hit_height,
            ),
            _ => {
                let points = dragpoint::expand(
                    &dragpoint::from_vpin(&t.drag_points),
                    true,
                    dragpoint::ACCURACY,
                );
                if points.len() < 3 {
                    continue;
                }
                (
                    Zone::Polygon {
                        vertices: points.iter().map(|p| Vec2::new(p.pos.x, p.pos.y)).collect(),
                    },
                    // The outline ones are eight units lower. It comes from the
                    // original, with the comment "adjust for same hit height as
                    // circular" next to it: it compensates for the circle
                    // measuring the height in a different way.
                    (t.hit_height - 8.0).max(0.0),
                )
            }
        };

        // How far it sinks and how fast. `anim_speed` comes in units per
        // millisecond.
        out.push(
            Trigger::new(area, base_z, base_z + height)
                .with_animation(crate::trigger::sink_depth(&t.shape, t.radius), t.anim_speed),
        );
    }
    out
}

/// Which of the plungers is the player's.
///
/// A table may have several and only one is driven by the button. F-14 ships
/// two: the one in the shooter lane and a `KickBack`, which is the mechanism
/// that returns the ball from the left outlane and is fired by the ROM. Taking
/// the first one that shows up would give whichever the file order happens to
/// put first.
///
/// It is picked by **name**, as with the flippers and for the same reason:
/// position does not tell a plunger apart from a mechanism. If none of them says
/// so, the one closest to the bottom of the table is taken, which is where the
/// shooter lane goes.
pub fn player_plunger(vpx: &VPX, shapes: &[Shape]) -> Option<usize> {
    let names: Vec<&str> = vpx
        .gameitems
        .iter()
        .filter_map(|item| match item {
            GameItemEnum::Plunger(p) => Some(p.name.as_str()),
            _ => None,
        })
        .collect();

    let found: Vec<(usize, f32)> = shapes
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            Shape::Plunger(p) => Some((i, p.bbox().max.y)),
            _ => None,
        })
        .collect();

    if found.len() <= 1 {
        return found.first().map(|(i, _)| *i);
    }

    let by_name = found.iter().zip(&names).find(|(_, n)| {
        let n = n.to_ascii_lowercase();
        n.contains("plunger") && !n.contains("kick")
    });
    if let Some(((i, _), _)) = by_name {
        return Some(*i);
    }

    found
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| *i)
}

/// Which button a flipper responds to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// Splits the flippers between the two buttons.
///
/// In the game this is done by the table's **script**: the VBScript ties the
/// left flipper key to an object with a name of its own. Until that engine
/// exists, it has to be decided from the outside.
///
/// It is decided by **name**, not by position. That looks more fragile but it is
/// the opposite: a table has flippers the player does not drive. F-14 ships two
/// `Diverter`s, which are diverters moved by the ROM; both sit to the left of
/// the axis, so splitting by position would tie them to the left button and they
/// would jerk about on every shot. By name they stay out, which is the right
/// thing: without a script engine nobody moves them.
///
/// The `LeftFlipper` / `RightFlipper` convention is widespread enough that it
/// works almost every time. When it does not — **no** flipper says which side it
/// is on — it falls back to position, but only for the two closest to the bottom
/// of the table, which are the ones you play with. A stray diverter up top does
/// not enter that count.
///
/// The indices are the ones of the `Vec<Shape>` that [`build`] produces, so they
/// can be handed to `Engine::set_flipper_solenoid`.
pub fn assign_flippers(vpx: &VPX, shapes: &[Shape]) -> Vec<(usize, Button)> {
    // The flippers come out in `shapes` in the same order they are in the file,
    // so walking both in parallel is enough.
    let names: Vec<&str> = vpx
        .gameitems
        .iter()
        .filter_map(|item| match item {
            GameItemEnum::Flipper(f) if f.is_enabled => Some(f.name.as_str()),
            _ => None,
        })
        .collect();

    let found: Vec<(usize, Vec2)> = shapes
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            Shape::Flipper(f) => Some((i, f.center)),
            _ => None,
        })
        .collect();

    let by_name: Vec<(usize, Vec2, Option<Button>)> = found
        .iter()
        .zip(&names)
        .map(|(&(i, c), name)| {
            let n = name.to_ascii_lowercase();
            let button = if n.contains("left") || n.contains("izq") {
                Some(Button::Left)
            } else if n.contains("right") || n.contains("der") {
                Some(Button::Right)
            } else {
                None
            };
            (i, c, button)
        })
        .collect();

    if by_name.iter().any(|(_, _, b)| b.is_some()) {
        return by_name
            .into_iter()
            .filter_map(|(i, _, b)| b.map(|b| (i, b)))
            .collect();
    }

    // No name said anything. The two lowest ones are taken — in VP `y` grows
    // towards the player — and split by the table's axis.
    let axis_x = (vpx.gamedata.left + vpx.gamedata.right) * 0.5;
    let mut candidates = by_name;
    candidates.sort_by(|a, b| b.1.y.total_cmp(&a.1.y));
    candidates.truncate(2);
    candidates
        .into_iter()
        .map(|(i, c, _)| {
            let button = if c.x < axis_x {
                Button::Left
            } else {
                Button::Right
            };
            (i, button)
        })
        .collect()
}

/// The plunger (`Plunger::PhysicSetup`, `plunger.cpp:...`).
///
/// The slot it runs in goes from `center.x - width` to `center.x + width`, and
/// the travel from `center.y - stroke` — all the way forward — to `center.y`.
/// The base is offset by `height`, which on a plunger is not a height but how
/// far the seat sticks out backwards.
fn plunger(p: &vpin::vpx::gameitem::plunger::Plunger, vpx: &VPX, out: &mut Vec<Shape>) {
    let base_z = surface_height(&vpx.gameitems, &p.surface, p.center.x, p.center.y);
    out.push(Shape::Plunger(Plunger::new(PlungerParams {
        x: p.center.x - p.width,
        x2: p.center.x + p.width,
        y: p.center.y + p.height,
        z_low: base_z,
        frame_end: p.center.y - p.stroke,
        frame_start: p.center.y,
        rest_pos: p.park_position,
        speed_pull: p.speed_pull,
        speed_fire: p.speed_fire,
        mech_strength: p.mech_strength,
        scatter_velocity: p.scatter_velocity,
        momentum_xfer: p.momentum_xfer,
        auto_plunger: p.auto_plunger,
        material: find_material(vpx, &p.material),
    })));
}

/// A ramp's collision (`Ramp::PhysicSetup`, `ramp.cpp:538`).
///
/// A ramp is the one part of a table that a ball travels **on** rather than
/// bounces off, and it is the reason a launch lane works: F-14's shooter lane
/// is a ramp that lifts the ball from the playfield to 61 units, over a wall
/// that is 45 tall, and drops it in the saucer at the top. Leave ramps out of
/// the physics and the ball runs into that wall at full speed and comes back —
/// which is exactly what it did.
///
/// Three kinds of shape, all from the original:
///
/// - **The floor**, as two triangles per step of the path, and the same two
///   again facing down, so a ball under a ramp cannot rise through it.
/// - **The side walls**, as line segments whose top follows the climb. The
///   original subdivides a segment whose ends differ in height by more than the
///   collision skin, because a segment has one top and one bottom and a steep
///   one would otherwise reach well above and below the floor it belongs to.
/// - **The joints**: a pole at each seam. Two surfaces that meet at an angle
///   each consider a ball at the join to be past their edge, so neither catches
///   it and it falls through. This is most of the shape count and all of the
///   reason a ramp feels solid.
fn ramp(r: &vpin::vpx::gameitem::ramp::Ramp, vpx: &VPX, out: &mut Vec<Shape>) {
    use vpin::vpx::gameitem::ramp::RampType;

    if !r.is_collidable {
        return;
    }
    let Some(c) = crate::ramp::path(r, dragpoint::collision_accuracy()) else {
        return;
    };
    let n = c.height.len();
    if n < 2 {
        return;
    }
    // The **physics** material, not the visual one (`Ramp::SetupHitObject`,
    // `ramp.cpp:799-810`). `r.material` is what the ramp is painted with, and a
    // table sets it for how the wood looks; how the ball behaves on it comes
    // either from `m_szPhysicsMaterial` or from the ramp's own four numbers,
    // exactly as it does for a wall. Passing the visual one meant a ramp took
    // the elasticity of whatever chrome or plastic it was skinned in, which is
    // most visible on a habitrail: the ball rattles down it instead of running.
    //
    // A deliberate departure in one detail: the original never touches
    // elasticity falloff for a ramp — the surface's version of this function
    // sets it and the ramp's does not — so the collider keeps its default of
    // zero even when a physics material declares one. Passing zero for the
    // override branch matches that; the named-material branch takes the
    // material's, which is the shared helper's behaviour and closer to what the
    // table's author asked for than dropping the number would be.
    let material = material(
        vpx,
        r.physics_material.as_deref(),
        r.overwrite_physics.unwrap_or(false),
        (r.elasticity, 0.0, r.friction, r.scatter),
    );

    // How tall the side walls are. A flat ramp says so itself; the wire kinds
    // carry the numbers the original hard-codes for them, which are there so
    // that old tables keep playing the way they did.
    let (wall_left, wall_right) = match r.ramp_type {
        RampType::Flat => (r.left_wall_height, r.right_wall_height),
        RampType::OneWire | RampType::TwoWire => (31.0, 31.0),
        RampType::FourWire => (62.0, 62.0),
        RampType::ThreeWireLeft => (62.0, 6.0 + 12.5),
        RampType::ThreeWireRight => (6.0 + 12.5, 62.0),
    };

    let pole = |p: Vec2, z_low: f32, z_high: f32, out: &mut Vec<Shape>| {
        if z_high > z_low {
            out.push(Shape::LineZ(HitLineZ {
                xy: p,
                z_low,
                z_high,
                material,
            }));
        }
    };
    let joint = |a: Vec3, b: Vec3, out: &mut Vec<Shape>| {
        if let Some(l) = HitLine3D::new(a, b, material) {
            out.push(Shape::Line3D(l));
        }
    };

    // The two side walls.
    for (edge, wall_height) in [(&c.right, wall_right), (&c.left, wall_left)] {
        if wall_height <= 0.0 {
            continue;
        }
        for i in 0..n - 1 {
            // The left wall is walked from the far end in the original, so that
            // its segments face outwards like the right one's. Both are added
            // in each direction anyway — a ramp wall is solid from both sides,
            // unlike a table wall.
            wall_segment(
                edge[i],
                edge[i + 1],
                c.height[i],
                c.height[i + 1],
                wall_height,
                i > 0,
                &material,
                out,
            );
            wall_segment(
                edge[i + 1],
                edge[i],
                c.height[i],
                c.height[i + 1],
                wall_height,
                i < n - 2,
                &material,
                out,
            );
        }
        pole(edge[0], c.height[0], c.height[0] + wall_height, out);
        pole(
            edge[n - 1],
            c.height[n - 1],
            c.height[n - 1] + wall_height,
            out,
        );
    }

    // The floor, two triangles per step, plus the joints along their edges.
    //
    //     3 - - 4      the path runs from the bottom of this sketch to the top
    //     | \   |
    //     |   \ |
    //     2 - - 1
    //
    // The long edges are known in advance, but the seams *between* triangles
    // are not: two that lie in the same plane need nothing between them, and
    // two that do not need a line or the ball crosses the fold unsupported and
    // drops through the ramp. The original decides that pair by pair as it
    // goes (`Ramp::CheckJoint`, `ramp.cpp:752`), and so does this.
    let mut previous: Option<Vec3> = None;
    let mut floor = |v: [Vec3; 3], out: &mut Vec<Shape>| {
        let normal = (v[1] - v[0]).cross(v[2] - v[0]);
        if normal.length_squared() < 1e-12 {
            return; // degenerate: the ramp has no width here
        }
        // `ramp.cpp:756`. By the order the vertices are given in, the first two
        // are always the edge this triangle shares with the one before it.
        if previous.is_none_or(|before| before.cross(normal).length_squared() >= 1e-8) {
            joint(v[0], v[1], out);
        }
        previous = Some(normal);

        // Facing up, so the ball rides on it...
        push_triangle(v, &material, out);
        // ...and facing down, so a ball underneath cannot come up through.
        push_triangle([v[1], v[0], v[2]], &material, out);
    };

    for i in 0..n - 1 {
        let (z0, z1) = (c.height[i], c.height[i + 1]);
        let p1 = c.right[i].extend(z0);
        let p2 = c.left[i].extend(z0);
        let p3 = c.left[i + 1].extend(z1);
        let p4 = c.right[i + 1].extend(z1);

        // The near edge, once, and then the two long edges of every step.
        if i == 0 {
            joint(p2, p1, out);
        }
        joint(p2, p3, out);
        joint(p1, p4, out);

        floor([p2, p1, p3], out);
        floor([p3, p1, p4], out);
    }
    let last = n - 1;
    joint(
        c.right[last].extend(c.height[last]),
        c.left[last].extend(c.height[last]),
        out,
    );
}

/// One segment of a ramp's side wall, subdividing where it climbs too fast.
///
/// A line segment has a single top and a single bottom, so on a steep ramp one
/// spanning the whole step would stick out above and below the floor it borders
/// (`ramp.cpp:776`). Halving until the rise is within the collision skin keeps
/// the error under the ball.
#[allow(clippy::too_many_arguments)]
fn wall_segment(
    a: Vec2,
    b: Vec2,
    z_a: f32,
    z_b: f32,
    wall_height: f32,
    with_joint: bool,
    material: &PhysMaterial,
    out: &mut Vec<Shape>,
) {
    // A guard on the depth as well as the height: a path with a NaN in it would
    // otherwise recurse until the stack ran out.
    wall_segment_inner(a, b, z_a, z_b, wall_height, with_joint, material, out, 0);
}

#[allow(clippy::too_many_arguments)]
fn wall_segment_inner(
    a: Vec2,
    b: Vec2,
    z_a: f32,
    z_b: f32,
    wall_height: f32,
    with_joint: bool,
    material: &PhysMaterial,
    out: &mut Vec<Shape>,
    depth: u32,
) {
    const MAX_DEPTH: u32 = 8;
    if z_b - z_a > 2.0 * PHYS_SKIN && depth < MAX_DEPTH {
        let mid = (a + b) * 0.5;
        let z_mid = (z_a + z_b) * 0.5;
        wall_segment_inner(
            a,
            mid,
            z_a,
            z_mid,
            wall_height,
            with_joint,
            material,
            out,
            depth + 1,
        );
        wall_segment_inner(
            mid,
            b,
            z_mid,
            z_b,
            wall_height,
            true,
            material,
            out,
            depth + 1,
        );
        return;
    }

    if (a - b).length() > 1e-6 {
        out.push(Shape::Line(LineSeg::new(
            a,
            b,
            z_a,
            z_b + wall_height,
            *material,
        )));
        if with_joint {
            out.push(Shape::LineZ(HitLineZ {
                xy: a,
                z_low: z_a,
                z_high: z_b + wall_height,
                material: *material,
            }));
        }
    }
}

/// A floor triangle, skipping the degenerate ones.
///
/// A ramp whose width goes to zero at a point produces triangles with no area,
/// and a normal cannot be taken from those. The original drops them too.
/// A triangle of a ramp's floor.
///
/// `HitTriangle` and not `HitPoly`, because that is what the original builds
/// here (`ramp.cpp:656`, `:677`, `:717`, `:731`) and the two are not
/// interchangeable. `Hit3DPoly` — which `HitPoly` ports — turns a ball away as
/// soon as it is receding at more than `C_LOWNORMVEL`, a ten-thousandth
/// (`collideex.cpp:554`); a triangle allows a thousand times that,
/// `C_CONTACTVEL` (`collideex.cpp:830`). On the surface a ball spends most of
/// its time resting on, the difference is whether a whisker of upward drift
/// costs it the floor under its feet.
///
/// The two also decide "is the contact point inside" differently — barycentric
/// in three dimensions against a crossing count on the xy projection — and a
/// ramp that climbs steeply is exactly where the projection stops being a
/// faithful picture of the triangle.
fn push_triangle(v: [Vec3; 3], material: &PhysMaterial, out: &mut Vec<Shape>) {
    if let Some(t) = HitTriangle::new(v, *material) {
        out.push(Shape::Triangle(t));
    }
}

/// The plane a drop target is hit *on*, as against the mesh it is made of.
///
/// A drop target has to know the difference between being hit on the face and
/// being brushed from behind or from the side, because only the first should
/// drop it. The original solves that by building a second, much simpler shape —
/// a flat panel standing just in front of the target — and putting the hit
/// event on *that* while the mesh itself only bounces the ball.
///
/// The numbers are the original's, `hittarget.cpp:191`. The mesh is in the
/// target's own space, so it goes through the same scale, turn and translation
/// the visible mesh does.
const DROP_TARGET_HIT_PLANE: [[f32; 3]; 16] = [
    [-0.300000, 0.001737, -0.160074],
    [-0.300000, 0.001738, 0.439926],
    [0.300000, 0.001738, 0.439926],
    [0.300000, 0.001737, -0.160074],
    [-0.500000, 0.001738, 0.439926],
    [-0.500000, 0.001738, 1.789926],
    [0.500000, 0.001738, 1.789926],
    [0.500000, 0.001738, 0.439926],
    [-0.535355, 0.001738, 0.454570],
    [-0.535355, 0.001738, 1.775281],
    [-0.550000, 0.001738, 0.489926],
    [-0.550000, 0.001738, 1.739926],
    [0.535355, 0.001738, 0.454570],
    [0.535355, 0.001738, 1.775281],
    [0.550000, 0.001738, 0.489926],
    [0.550000, 0.001738, 1.739926],
];

/// `hittarget.cpp:212`.
const DROP_TARGET_HIT_PLANE_INDICES: [usize; 42] = [
    0, 1, 2, 2, 3, 0, 1, 4, 5, 6, 7, 2, 5, 6, 1, 2, 1, 6, 4, 8, 9, 9, 5, 4, 8, 10, 11, 11, 9, 8, 6,
    12, 7, 12, 6, 13, 12, 13, 14, 13, 15, 14,
];

/// How far in front of the target the hit plane stands, by target type.
/// `hittarget.cpp:263`.
fn hit_plane_offset(t: &vpin::vpx::gameitem::hittarget::TargetType) -> f32 {
    use vpin::vpx::gameitem::hittarget::TargetType as T;
    match t {
        T::DropTargetBeveled => 0.25,
        T::DropTargetFlatSimple => 0.13,
        _ => 0.18,
    }
}

/// Whether a target type drops out of the playfield when it is hit.
fn drops(t: &vpin::vpx::gameitem::hittarget::TargetType) -> bool {
    use vpin::vpx::gameitem::hittarget::TargetType as T;
    matches!(
        t,
        T::DropTargetBeveled | T::DropTargetSimple | T::DropTargetFlatSimple
    )
}

/// A target, drop or stand-up (`HitTarget::PhysicSetup`, `hittarget.cpp:220`).
///
/// The target's whole mesh becomes colliders: one triangle each, one line along
/// each unique edge, one point at each vertex. That is not belt and braces —
/// three triangles meeting at a corner leave a seam a ball can pass through, and
/// the edges and points seal it. It is the same construction ramps and
/// primitives use, and the reason a mesh of a hundred triangles turns into
/// several hundred shapes.
///
/// A **drop target** gets a second shape on top: the flat plane described by
/// [`DROP_TARGET_HIT_PLANE`], standing just in front of the face. Only that
/// plane reports hits, so a ball rolling past the back of the target bounces
/// off it without knocking it down. In legacy mode the plane is not built and
/// the mesh reports hits itself, which is what tables written before the plane
/// existed expect.
fn hit_target(t: &vpin::vpx::gameitem::hittarget::HitTarget, vpx: &VPX, out: &mut Vec<Shape>) {
    let Some(mesh) = crate::builtin::target_collision_mesh(t) else {
        return;
    };

    let material = if t.overwrite_physics.unwrap_or(false) {
        PhysMaterial {
            elasticity: t.elasticity,
            elasticity_falloff: t.elasticity_falloff,
            friction: t.friction,
            scatter: t.scatter.to_radians(),
        }
    } else {
        find_material(vpx, t.physics_material.as_deref().unwrap_or(""))
    };

    push_mesh(&mesh.0, &mesh.1, &material, out);

    if drops(&t.target_type) && !t.is_legacy {
        let offset = hit_plane_offset(&t.target_type);
        let transform = crate::builtin::target_transform(t);
        let verts: Vec<Vec3> = DROP_TARGET_HIT_PLANE
            .iter()
            .map(|v| transform.transform_point3(Vec3::new(v[0], v[1] + offset, v[2])))
            .collect();
        push_mesh(&verts, &DROP_TARGET_HIT_PLANE_INDICES, &material, out);
    }
}

/// Turns an indexed mesh into colliders: a triangle, its three edges and its
/// three corners, with the edges and corners shared between neighbours.
///
/// `AddHitEdge` (`hittarget.cpp:351`) keeps a set of the edges it has already
/// built so a shared one is not added twice; the same reasoning applies to the
/// vertices, which the original handles by walking the vertex list rather than
/// the index list.
fn push_mesh(verts: &[Vec3], indices: &[usize], material: &PhysMaterial, out: &mut Vec<Shape>) {
    let mut edges: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();

    for tri in indices.as_chunks::<3>().0 {
        let [i0, i1, i2] = *tri;
        let Some(&v0) = verts.get(i0) else { continue };
        let (Some(&v1), Some(&v2)) = (verts.get(i1), verts.get(i2)) else {
            continue;
        };
        // The renderer's winding; `HitTriangle` wants the other one and does
        // the swap itself.
        if let Some(t) = HitTriangle::from_render_order(v0, v1, v2, *material) {
            out.push(Shape::Triangle(t));
        }
        for (a, b) in [(i0, i1), (i1, i2), (i2, i0)] {
            let key = (a.min(b), a.max(b));
            if edges.insert(key)
                && let Some(edge) = HitLine3D::new(verts[key.0], verts[key.1], *material)
            {
                out.push(Shape::Line3D(edge));
            }
        }
    }

    for &v in verts {
        out.push(Shape::Point(HitPoint {
            p: v,
            material: *material,
        }));
    }
}

/// A primitive's mesh, as colliders (`Primitive::PhysicSetup`,
/// `primitive.cpp:178`).
///
/// On F-14 this is a refinement; on almost any table built since, it is the
/// difference between a playfield and a photograph of one. A primitive is an
/// arbitrary mesh, and once tables stopped being made out of walls and ramps it
/// became the way nearly everything solid is built — the plastics, the toys,
/// the metal guides, the habitrails.
///
/// Three flags decide whether anything is built at all, and they mean different
/// things. `is_visible` is about drawing and says nothing about collision.
/// `is_collidable` is the switch a script throws. And `is_toy` is the table
/// author declaring the part decorative: a toy gets no colliders even when it is
/// marked collidable, which is the first thing the original checks
/// (`primitive.cpp:187`).
///
/// # What is not ported
///
/// The original can **simplify** a mesh before colliding with it, running it
/// through a progressive-mesh reduction down to a vertex count the table asks
/// for (`primitive.cpp:194`). It only does so above four hundred and twenty
/// vertices and only when the author has set a reduction factor, and F-14 has
/// no primitive anywhere near that. Leaving it out means a table that asked for
/// a simpler collision mesh gets the full one: more accurate than it asked for,
/// and slower, which is the safe direction to be wrong in.
fn primitive(p: &vpin::vpx::gameitem::primitive::Primitive, vpx: &VPX, out: &mut Vec<Shape>) {
    if !p.is_collidable || p.is_toy {
        return;
    }
    let Some(mesh) = p.read_mesh().ok().flatten() else {
        return;
    };

    // A primitive called `playfield_mesh` *is* the playfield, and takes the
    // table's own physics rather than a material of its own
    // (`primitive.h:271`, `primitive.cpp:330`).
    let material = if p.name.eq_ignore_ascii_case("playfield_mesh") {
        find_material(vpx, &vpx.gamedata.playfield_material)
    } else if p.overwrite_physics.unwrap_or(false) {
        PhysMaterial {
            elasticity: p.elasticity,
            elasticity_falloff: p.elasticity_falloff,
            friction: p.friction,
            scatter: p.scatter.to_radians(),
        }
    } else {
        find_material(vpx, p.physics_material.as_deref().unwrap_or(""))
    };

    let transform = crate::geometry::primitive_transform(p);
    let verts: Vec<Vec3> = mesh
        .vertices
        .iter()
        .map(|w| transform.transform_point3(Vec3::new(w.vertex.x, w.vertex.y, w.vertex.z)))
        .collect();
    let indices: Vec<usize> = mesh
        .indices
        .iter()
        .flat_map(|f| [f.i0 as usize, f.i1 as usize, f.i2 as usize])
        .collect();

    push_mesh(&verts, &indices, &material, out);
}

#[cfg(test)]
mod target_tests {
    use super::*;
    use vpin::vpx::gameitem::hittarget::{HitTarget, TargetType};

    /// A target of a given type, at the origin and at its natural size.
    fn target(target_type: TargetType) -> HitTarget {
        HitTarget {
            name: "t".into(),
            target_type,
            size: vpin::vpx::gameitem::vertex3d::Vertex3D {
                x: 32.0,
                y: 32.0,
                z: 32.0,
            },
            ..HitTarget::default()
        }
    }

    /// A mesh with no shared vertices between triangles would give three edges
    /// and three points per triangle; a real one shares nearly all of them.
    /// This is the check that the sharing happens, because without it a target
    /// costs four times what it should and pays for nothing.
    #[test]
    fn a_targets_edges_and_corners_are_shared() {
        let mut shapes = Vec::new();
        hit_target(
            &target(TargetType::HitTargetRound),
            &blank_table(),
            &mut shapes,
        );

        let count = |f: fn(&Shape) -> bool| shapes.iter().filter(|s| f(s)).count();
        let tris = count(|s| matches!(s, Shape::Triangle(_)));
        let edges = count(|s| matches!(s, Shape::Line3D(_)));
        let points = count(|s| matches!(s, Shape::Point(_)));

        assert!(tris > 0, "a target should produce triangles");
        assert!(
            edges < tris * 3,
            "{edges} edges for {tris} triangles means none are shared"
        );
        assert!(
            points < tris * 3,
            "{points} corners for {tris} triangles means none are shared"
        );
    }

    /// The hit plane, which is the whole reason a drop target knows which way
    /// round it was hit.
    ///
    /// A stand-up has no plane: every triangle of it reports. A drop target in
    /// legacy mode has none either, because tables written before the plane
    /// existed expect the mesh to report. Only a modern drop target gets one.
    #[test]
    fn only_a_modern_drop_target_gets_a_hit_plane() {
        let table = blank_table();
        let count = |t: HitTarget| {
            let mut shapes = Vec::new();
            hit_target(&t, &table, &mut shapes);
            shapes
                .iter()
                .filter(|s| matches!(s, Shape::Triangle(_)))
                .count()
        };

        let standup = count(target(TargetType::HitTargetRound));
        let drop = count(target(TargetType::DropTargetSimple));
        let mut legacy = target(TargetType::DropTargetSimple);
        legacy.is_legacy = true;
        let legacy = count(legacy);

        // The plane is 42 indices, so 14 triangles.
        assert_eq!(
            drop - legacy,
            14,
            "the drop target's extra shape is the hit plane"
        );
        assert!(standup > 0, "a stand-up still collides");
    }

    /// A target the table says is not collidable produces nothing at all.
    #[test]
    fn a_target_that_is_not_collidable_is_not_there() {
        let mut t = target(TargetType::HitTargetRound);
        t.is_collidable = false;
        let mut shapes = Vec::new();
        hit_target(&t, &blank_table(), &mut shapes);
        assert!(shapes.is_empty());
    }

    /// Every triangle a target produces has to be usable.
    ///
    /// [`HitTriangle`] refuses a sliver, and it refuses it by returning
    /// `None` rather than by producing something that answers hit tests
    /// wrongly. If a target's mesh were full of slivers this would be a target
    /// with holes in it.
    #[test]
    fn a_targets_triangles_all_have_a_direction() {
        let mut shapes = Vec::new();
        hit_target(
            &target(TargetType::DropTargetBeveled),
            &blank_table(),
            &mut shapes,
        );
        for s in &shapes {
            if let Shape::Triangle(t) = s {
                let n = t.normal.length();
                assert!(
                    (n - 1.0).abs() < 1e-3,
                    "a triangle's normal should be a unit vector, and one is {n}"
                );
            }
        }
    }

    fn blank_table() -> VPX {
        VPX::default()
    }
}

#[cfg(test)]
mod part_tests {
    use super::*;
    use vpin::vpx::gameitem::vertex2d::Vertex2D;

    fn blank_table() -> VPX {
        VPX::default()
    }

    /// A gate at the origin, one-way and with a bracket, so it makes one of
    /// each shape the builder can make.
    fn a_gate() -> vpin::vpx::gameitem::gate::Gate {
        vpin::vpx::gameitem::gate::Gate {
            name: "g".into(),
            center: Vertex2D { x: 0.0, y: 0.0 },
            length: 60.0,
            height: 40.0,
            rotation: 0.0,
            two_way: false,
            show_bracket: true,
            is_collidable: true,
            angle_min: 0.0,
            angle_max: std::f32::consts::FRAC_PI_2,
            ..vpin::vpx::gameitem::gate::Gate::default()
        }
    }

    fn built(g: &vpin::vpx::gameitem::gate::Gate) -> Vec<Shape> {
        let mut out = Vec::new();
        gate(g, &blank_table(), &mut out);
        out
    }

    /// A gate the file says is not collidable is still built.
    ///
    /// `Gate::PhysicSetup` (`gate.cpp:322-333`) creates the leaf and the
    /// blocking segment whatever the flag says and only puts the flag on the
    /// leaf. Building nothing leaves a script that later writes
    /// `Collidable = True` with nothing to switch on — and, because the drawing
    /// side pairs meshes with shapes by order, leaves every gate after it in
    /// the file borrowing the wrong one's angle.
    #[test]
    fn a_gate_that_does_not_collide_is_still_built() {
        let mut g = a_gate();
        g.is_collidable = false;
        let shapes = built(&g);

        let leaf = shapes.iter().find_map(|s| match s {
            Shape::Gate(g) => Some(g),
            _ => None,
        });
        let leaf = leaf.expect("the leaf is built either way");
        assert!(!leaf.enabled, "but it is switched off");
        assert!(!leaf.collidable, "and it remembers that the file said so");

        // The blocking segment is still there **and still on**, which is the
        // original's own asymmetry: nothing gives it the flag at build time.
        let blocking = shapes
            .iter()
            .filter(|s| matches!(s, Shape::Line(_)))
            .count();
        assert_eq!(blocking, 1, "a one-way gate keeps its blocking segment");
    }

    /// A two-way gate has no blocking segment at all: it lets the ball through
    /// from both sides, which is the whole difference between the two kinds.
    #[test]
    fn a_two_way_gate_has_no_blocking_segment() {
        let mut g = a_gate();
        g.two_way = true;
        let shapes = built(&g);
        assert!(!shapes.iter().any(|s| matches!(s, Shape::Line(_))));
        assert!(shapes.iter().any(|s| matches!(s, Shape::Gate(_))));
    }

    /// A bumper's scatter comes from the file, not from the table's default.
    ///
    /// `bumper.cpp:204` sets exactly one physical number on the circle it
    /// builds, and this is it. A bumper is where a table asks for the most
    /// scatter it asks for anywhere: without it the same shot off the same
    /// bumper lands in the same place every time.
    #[test]
    fn a_bumpers_scatter_comes_from_the_file() {
        let b = vpin::vpx::gameitem::bumper::Bumper {
            name: "b".into(),
            center: Vertex2D { x: 0.0, y: 0.0 },
            radius: 45.0,
            scatter: Some(9.0),
            hit_event: Some(false),
            ..vpin::vpx::gameitem::bumper::Bumper::default()
        };
        let mut out = Vec::new();
        bumper(&b, &blank_table(), &mut out);

        let Some(Shape::Bumper(built)) = out.first() else {
            panic!("a bumper should build a bumper");
        };
        assert!(
            (built.circle.material.scatter - 9.0f32.to_radians()).abs() < 1e-6,
            "the scatter should be the file's nine degrees in radians, it is {}",
            built.circle.material.scatter
        );
        assert!(!built.hit_event, "and `HAHE` reaches the coil");
    }
}
