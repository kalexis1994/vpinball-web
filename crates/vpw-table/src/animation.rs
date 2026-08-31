//! The pieces that move.
//!
//! The renderer **bakes** the table's transforms at load time: it takes the
//! three thousand pieces into world space once and never touches them again. It
//! is the right decision for a table that does not change once loaded, and the
//! wrong one for the ten pieces that do.
//!
//! This module is the list of those ten. For each one it keeps:
//!
//! - the mesh in its **local** frame, unbaked;
//! - which physics piece its state comes from;
//! - and the transform split in three, which is what makes animating it cheap.
//!
//! # Why the transform is split in three
//!
//! A flipper is placed with `translate · rotate(angle) · scale`. The angle is the
//! only thing that changes, and it is in the **middle** of the product: it
//! cannot be multiplied in front or behind, the product has to be redone.
//! Keeping `base` (what comes before) and `local` (what comes after) separately
//! leaves the animation at `base · f(state) · local`, which is two matrix
//! products per frame per piece.
//!
//! # How the pieces are paired up with the physics
//!
//! Neither by name nor by position: by **order**. `physics::build` walks the
//! file's gameitems exactly once, pushing shapes as it goes, so the nth
//! `Shape::Flipper` is the nth flipper in the file — of those that passed the
//! physics filter, which is not the same as the drawing one. A disabled flipper
//! produces no shape but may still have a visible mesh.
//!
//! That is why each kind filters the gameitems with **the same predicate the
//! physics uses** before pairing them up, and only then decides whether it is
//! also drawn.

use crate::geometry::{Mesh, MeshKind};
use std::f32::consts::PI;
use vpin::vpx::VPX;
use vpin::vpx::gameitem::GameItemEnum;
use vpw_math::{Mat4, Vec3};
use vpw_physics::engine::{Engine, Shape};

/// Where a moving piece's state comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Animation {
    /// Turns around its vertical axis: the flipper.
    Flipper { shape: usize },
    /// Turns around its horizontal axis: the gate.
    Gate { shape: usize },
    /// Same, but it goes all the way round: the spinner's plate.
    Spinner { shape: usize },
    /// Sinks in `z` when the ball goes over it.
    Trigger { trigger: usize },
    /// A primitive the script moves: a flipper bat, a drop target, a toy.
    ///
    /// Unlike everything else here its pose does not come from the physics —
    /// the engine has never heard of it. It comes from the nine placement
    /// numbers the script writes on the part, so the matrix is supplied from
    /// outside and [`AnimatedPart::animated`] leaves it alone.
    ///
    /// `index` is the part's place in `vpx.gameitems`, which is what a host
    /// needs to find the item the script is writing to.
    Primitive { index: usize },
    /// A part that never moves, and whose only animation is whether it is
    /// drawn at all.
    ///
    /// Sounds like a contradiction and is not: a slingshot's arm on F-14 is
    /// three rubbers stacked in the same place, and the "animation" is the
    /// script showing one and hiding the other two, frame by frame, off a
    /// timer. Nothing about the geometry changes; what changes is which of the
    /// three is visible. A part like this still has to be **out** of the static
    /// scene, because a baked mesh is drawn whatever the script says.
    Fixed,
    /// The bumper's ring, which drops sharply and comes back on its own.
    BumperRing { shape: usize },
    /// The head of the plunger — tip, ring and shoulder — which slides along
    /// `y` with the plunger's position.
    PlungerHead { shape: usize },
    /// The plunger's shaft, which does **not** slide: its far end is pinned to
    /// the back of the barrel and it stretches (`plunger.cpp:509-518`, and the
    /// long explanation in [`crate::plunger`]).
    PlungerShaft {
        shape: usize,
        /// Where the head ends, relative to the tip, in table units.
        shoulder_y: f32,
        /// Where the back of the rod is pinned, in table coordinates. The
        /// original calls it `rody`.
        back_y: f32,
    },
}

/// A piece that is drawn with a matrix of its own.
#[derive(Debug, Clone)]
pub struct AnimatedPart {
    /// The mesh, with its vertices in the local frame and unbaked.
    pub mesh: Mesh,
    /// What goes **before** the animated part.
    pub base: Mat4,
    /// What goes **after**.
    pub local: Mat4,
    pub anim: Animation,
}

impl AnimatedPart {
    /// This piece's transform with the physics' current state.
    ///
    /// A [`Animation::Primitive`] keeps the pose it was built in: the engine
    /// does not know where it is, and only whoever holds the script's state
    /// does. A host that draws those has to ask for the matrix elsewhere; one
    /// that does not gets the resting pose, which is what the piece looked like
    /// before anything moved it.
    pub fn transform(&self, engine: &Engine) -> Mat4 {
        if matches!(self.anim, Animation::Primitive { .. } | Animation::Fixed) {
            return self.mesh.transform;
        }
        self.base * self.animated(engine) * self.local
    }

    /// The middle part: the one that depends on the state.
    fn animated(&self, engine: &Engine) -> Mat4 {
        let shape = |i: usize| engine.shapes().get(i);
        match self.anim {
            // Handled in `transform`: nothing about them is in the engine.
            Animation::Primitive { .. } | Animation::Fixed => Mat4::IDENTITY,
            Animation::Flipper { shape: i } => match shape(i) {
                Some(Shape::Flipper(f)) => Mat4::from_rotation_z(f.angle_cur),
                _ => Mat4::IDENTITY,
            },
            // The physics angle grows towards the side the ball pushes the gate
            // from, and on a one-way gate that is a **negative** turn in x.
            //
            // A two-way one turns the other way: `gate.cpp:429` builds the
            // matrix as `MatrixRotateX(twoWay ? angle : -angle)`. It is not a
            // sign convention anyone would guess from the physics, and the
            // difference shows: a two-way wire's angle swings either side of
            // zero, so negating it dips the leaf **below** the playfield on
            // every hit from the side the file calls positive.
            Animation::Gate { shape: i } => match shape(i) {
                Some(Shape::Gate(g)) => {
                    Mat4::from_rotation_x(if g.two_way { g.angle } else { -g.angle })
                }
                _ => Mat4::IDENTITY,
            },
            Animation::Spinner { shape: i } => match shape(i) {
                Some(Shape::Spinner(s)) => Mat4::from_rotation_x(-s.angle),
                _ => Mat4::IDENTITY,
            },
            // `anim_offset` already comes out negative when the wire is sunk.
            Animation::Trigger { trigger } => match engine.triggers().get(trigger) {
                Some(t) => Mat4::from_translation(Vec3::new(0.0, 0.0, t.anim_offset)),
                None => Mat4::IDENTITY,
            },
            Animation::BumperRing { shape: i } => match shape(i) {
                Some(Shape::Bumper(b)) => {
                    Mat4::from_translation(Vec3::new(0.0, 0.0, b.ring_offset))
                }
                _ => Mat4::IDENTITY,
            },
            Animation::PlungerHead { shape: i } => match shape(i) {
                Some(Shape::Plunger(p)) => Mat4::from_translation(Vec3::new(0.0, p.pos, 0.0)),
                _ => Mat4::IDENTITY,
            },
            // The shaft is a unit-long cylinder: put its near end on the head's
            // shoulder and stretch it until it reaches the pinned back end.
            // Never let it go to zero or negative — a plunger pushed past its
            // own barrel would turn the mesh inside out.
            Animation::PlungerShaft {
                shape: i,
                shoulder_y,
                back_y,
            } => match shape(i) {
                Some(Shape::Plunger(p)) => {
                    let near = p.pos + shoulder_y;
                    let length = (back_y - near).max(0.01);
                    Mat4::from_translation(Vec3::new(0.0, near, 0.0))
                        * Mat4::from_scale(Vec3::new(1.0, length, 1.0))
                }
                _ => Mat4::IDENTITY,
            },
        }
    }
}

/// All the moving pieces of a table already loaded into the engine.
///
/// `engine` must have been built with `physics::build(vpx)` and its triggers
/// with `physics::triggers(vpx)`, in that order: the pairing depends on it.
pub fn animated_parts(vpx: &VPX, engine: &Engine) -> Vec<AnimatedPart> {
    let mut out = Vec::new();
    let shapes = engine.shapes();

    flippers(vpx, shapes, &mut out);
    gates(vpx, shapes, &mut out);
    spinners(vpx, shapes, &mut out);
    bumper_rings(vpx, shapes, &mut out);
    plunger_rods(vpx, shapes, &mut out);
    triggers(vpx, &mut out);
    moving_primitives(vpx, &mut out);
    moving_rubbers(vpx, &mut out);

    // Every builder leaves `mesh.transform` at whatever its own piece happens
    // to use as a resting pose, and they do not agree: a flipper's carries its
    // start angle, a trigger's is just a translation and a plunger rod's is the
    // identity, because where the rod sits is a fact about the engine and not
    // about the `.vpx`. Resolving them all here makes `mesh.transform` mean one
    // thing — where the piece is right now — which is what the renderer uses to
    // seed its matrices.
    for part in &mut out {
        part.mesh.transform = part.transform(engine);
    }

    out
}

/// The indices of the shapes of one kind, in the order the table built them.
fn indices_of(shapes: &[Shape], of_this_kind: impl Fn(&Shape) -> bool) -> Vec<usize> {
    shapes
        .iter()
        .enumerate()
        .filter(|(_, s)| of_this_kind(s))
        .map(|(i, _)| i)
        .collect()
}

fn flippers(vpx: &VPX, shapes: &[Shape], out: &mut Vec<AnimatedPart>) {
    let shape_indices = indices_of(shapes, |s| matches!(s, Shape::Flipper(_)));
    let items = vpx.gameitems.iter().filter_map(|i| match i {
        // The same filter as `physics::flipper`.
        GameItemEnum::Flipper(f) if f.is_enabled => Some(f),
        _ => None,
    });

    for (f, &shape) in items.zip(&shape_indices) {
        // `flipper::build` already builds the mesh with the corrected radii;
        // what is needed is to pull the rest angle out of the transform, which
        // is exactly what the animation replaces.
        let Some(mut mesh) =
            crate::flipper::build(f, surface_height(vpx, &f.surface, f.center.x, f.center.y))
        else {
            continue;
        };
        let base = Mat4::from_translation(Vec3::new(
            f.center.x,
            f.center.y,
            surface_height(vpx, &f.surface, f.center.x, f.center.y),
        ));
        let local = Mat4::from_scale(Vec3::new(1.0, 1.0, f.height)) * Mat4::from_rotation_z(PI);
        mesh.transform = base * Mat4::from_rotation_z(f.start_angle.to_radians()) * local;
        out.push(AnimatedPart {
            mesh,
            base,
            local,
            anim: Animation::Flipper { shape },
        });

        // The rubber ring turns with the bat, and saying so is not optional:
        // it is a second mesh, and a second mesh that nobody animates is a
        // black ring left behind at the rest angle while the bat swings out
        // from under it. Its own thickness and its own height above the
        // playfield, neither of which is the bat's (`flipper.cpp:706`).
        let Some(mut ring) =
            crate::flipper::rubber(f, surface_height(vpx, &f.surface, f.center.x, f.center.y))
        else {
            continue;
        };
        let ring_base = Mat4::from_translation(Vec3::new(
            f.center.x,
            f.center.y,
            surface_height(vpx, &f.surface, f.center.x, f.center.y)
                + f.rubber_height.unwrap_or(0.0),
        ));
        let ring_local = Mat4::from_scale(Vec3::new(1.0, 1.0, f.rubber_width.unwrap_or(0.0)))
            * Mat4::from_rotation_z(PI);
        ring.transform = ring_base * Mat4::from_rotation_z(f.start_angle.to_radians()) * ring_local;
        out.push(AnimatedPart {
            mesh: ring,
            base: ring_base,
            local: ring_local,
            anim: Animation::Flipper { shape },
        });
    }
}

/// The primitives the table itself says are not static.
///
/// A `.vpx` marks every primitive with `StaticRendering`, and the ones that
/// have it off are exactly the ones something is expected to move — the
/// original re-renders them every frame for that reason. Taking the same list
/// means a port does not have to guess which toys a script might animate, and
/// does not pay for the hundreds it will not.
///
/// It matters more than "a toy that wobbles". On F-14 the flipper you see is
/// not the flipper part at all: the script hides that one and shows a primitive
/// bat, turning it from a timer with
/// `batleft.ObjRotZ = LeftFlipper.CurrentAngle + 1`. A port that bakes those
/// primitives draws a flipper that never moves, with the hidden one poking out
/// from under it.
fn moving_primitives(vpx: &VPX, out: &mut Vec<AnimatedPart>) {
    for (index, item) in vpx.gameitems.iter().enumerate() {
        let GameItemEnum::Primitive(p) = item else {
            continue;
        };
        if p.static_rendering {
            continue;
        }
        // Being hidden at load time is not a reason to leave it out. Whether a
        // part is drawn is asked every frame, of the live object, and the
        // script is free to answer differently a moment later — that is the
        // whole point of a part the table marked dynamic.
        let Some(mesh) = crate::geometry::primitive_part(p) else {
            continue;
        };
        // The whole placement is rebuilt from the numbers each frame, so
        // neither half of it belongs in `base` or `local`.
        out.push(AnimatedPart {
            mesh,
            base: Mat4::IDENTITY,
            local: Mat4::IDENTITY,
            anim: Animation::Primitive { index },
        });
    }
}

/// The rubbers the table itself says are not static.
///
/// The same rule as [`moving_primitives`] and for the same reason: a `.vpx`
/// marks a rubber with `EnableStaticRendering`, and the original checks that
/// flag before it decides whether the rubber goes into the static buffer or is
/// redrawn every frame (`rubber.cpp:466`, `:713`). Taking the table's own
/// answer means a port does not have to guess.
///
/// It is what makes a slingshot look like one. F-14 draws the arm as three
/// rubbers in the same place — `RSling`, `RSling1`, `RSling2` — and
/// `RightSlingShot_Slingshot` shows them in turn off a timer while the plastic
/// underneath dips on its `TransZ`. All three are marked dynamic and two of
/// them start invisible, so they are built regardless of that: what the script
/// wants is the frame, and it asks for it later.
///
/// Baking them instead leaves the arm nailed to its rest pose, showing the one
/// frame that happened to be visible at load time, no matter what the script
/// does.
fn moving_rubbers(vpx: &VPX, out: &mut Vec<AnimatedPart>) {
    for item in &vpx.gameitems {
        let GameItemEnum::Rubber(r) = item else {
            continue;
        };
        if r.static_rendering {
            continue;
        }
        let Some(mesh) = crate::rubber::mesh(r) else {
            continue;
        };
        out.push(AnimatedPart {
            mesh,
            base: Mat4::IDENTITY,
            local: Mat4::IDENTITY,
            anim: Animation::Fixed,
        });
    }
}

fn gates(vpx: &VPX, shapes: &[Shape], out: &mut Vec<AnimatedPart>) {
    let shape_indices = indices_of(shapes, |s| matches!(s, Shape::Gate(_)));
    // Every gate in the file, with no filter: `physics::gate` builds a leaf for
    // each one whether it collides or not, and this pairing is by order. It
    // used to skip the non-collidable ones on both sides, which agreed with
    // itself but not with the original — and a table's decorative gate was
    // drawn nailed shut while the collidable one after it in the file borrowed
    // its angle.
    let items = vpx.gameitems.iter().filter_map(|i| match i {
        GameItemEnum::Gate(g) => Some(g),
        _ => None,
    });

    for (g, &shape) in items.zip(&shape_indices) {
        // The bracket does not move: the static scene draws it. What gets
        // animated is the wire, which is the first mesh `builtin` produces.
        let Some(mesh) =
            crate::builtin::gate(g, surface_height(vpx, &g.surface, g.center.x, g.center.y))
                .into_iter()
                .next()
        else {
            continue;
        };
        let base = Mat4::from_translation(Vec3::new(
            g.center.x,
            g.center.y,
            g.height + surface_height(vpx, &g.surface, g.center.x, g.center.y),
        )) * Mat4::from_rotation_z(g.rotation.to_radians());
        let local = Mat4::from_scale(Vec3::splat(g.length));
        out.push(AnimatedPart {
            mesh,
            base,
            local,
            anim: Animation::Gate { shape },
        });
    }
}

fn spinners(vpx: &VPX, shapes: &[Shape], out: &mut Vec<AnimatedPart>) {
    let shape_indices = indices_of(shapes, |s| matches!(s, Shape::Spinner(_)));
    // The physics builds one spinner for every one the file ships, unfiltered.
    let items = vpx.gameitems.iter().filter_map(|i| match i {
        GameItemEnum::Spinner(s) => Some(s),
        _ => None,
    });

    for (s, &shape) in items.zip(&shape_indices) {
        let Some(mesh) =
            crate::builtin::spinner(s, surface_height(vpx, &s.surface, s.center.x, s.center.y))
                .into_iter()
                .next()
        else {
            continue;
        };
        let base = Mat4::from_translation(Vec3::new(
            s.center.x,
            s.center.y,
            s.height + surface_height(vpx, &s.surface, s.center.x, s.center.y),
        )) * Mat4::from_rotation_z(s.rotation.to_radians());
        let local = Mat4::from_scale(Vec3::splat(s.length));
        out.push(AnimatedPart {
            mesh,
            base,
            local,
            anim: Animation::Spinner { shape },
        });
    }
}

fn bumper_rings(vpx: &VPX, shapes: &[Shape], out: &mut Vec<AnimatedPart>) {
    let shape_indices = indices_of(shapes, |s| matches!(s, Shape::Bumper(_)));
    let items = vpx.gameitems.iter().filter_map(|i| match i {
        GameItemEnum::Bumper(b) if b.is_collidable.unwrap_or(true) => Some(b),
        _ => None,
    });

    for (b, &shape) in items.zip(&shape_indices) {
        if !b.is_ring_visible.unwrap_or(true) {
            continue;
        }
        let base_z = surface_height(vpx, &b.surface, b.center.x, b.center.y);
        let Some(mut mesh) = crate::meshes::get("bumperRing").map(|m| Mesh {
            name: format!("{} (ring)", b.name),
            vertices: m.vertices,
            indices: m.indices,
            transform: Mat4::IDENTITY,
            image: String::new(),
            material: b.ring_material.clone().unwrap_or_default(),
            visible: true,
            clamp: false,
            scenery: false,
            kind: MeshKind::Builtin,
            additive: None,
            depth_bias: 0.0,
            disable_lighting: 0.0,
        }) else {
            continue;
        };
        let base = Mat4::from_translation(Vec3::new(b.center.x, b.center.y, base_z));
        let local = Mat4::from_rotation_z(b.orientation.to_radians())
            * Mat4::from_scale(Vec3::new(b.radius, b.radius, b.height_scale));
        mesh.transform = base * local;
        out.push(AnimatedPart {
            mesh,
            base,
            local,
            anim: Animation::BumperRing { shape },
        });
    }
}

fn plunger_rods(vpx: &VPX, shapes: &[Shape], out: &mut Vec<AnimatedPart>) {
    let shape_indices = indices_of(shapes, |s| matches!(s, Shape::Plunger(_)));
    let items = vpx.gameitems.iter().filter_map(|i| match i {
        GameItemEnum::Plunger(p) => Some(p),
        _ => None,
    });

    for (p, &shape) in items.zip(&shape_indices) {
        if !p.is_visible {
            continue;
        }
        let base_z = surface_height(vpx, &p.surface, p.center.x, p.center.y);
        // The rod's axis sits at `zheight + width` (`plunger.cpp:541`), which
        // for a standard plunger is the height of the ball's centre — which is
        // where it has to hit it. `width` is the plunger's half-width and a
        // standard ball's radius measures the same.
        let base = Mat4::from_translation(Vec3::new(p.center.x, 0.0, base_z + p.width));

        // `rody = beginy + m_d.m_height` (`plunger.cpp:257, 517`): the back of
        // the rod, which does not move.
        let back_y = p.center.y + p.height;

        out.push(AnimatedPart {
            mesh: crate::plunger::head_mesh(&p.name, p.width, p.material.clone()),
            base,
            local: Mat4::IDENTITY,
            anim: Animation::PlungerHead { shape },
        });
        out.push(AnimatedPart {
            mesh: crate::plunger::shaft_mesh(
                &format!("{} (shaft)", p.name),
                p.width,
                p.material.clone(),
            ),
            base,
            local: Mat4::IDENTITY,
            anim: Animation::PlungerShaft {
                shape,
                shoulder_y: crate::plunger::SHOULDER_Y,
                back_y,
            },
        });
    }
}

fn triggers(vpx: &VPX, out: &mut Vec<AnimatedPart>) {
    // `physics::triggers` filters by `is_enabled`; drawing also filters by
    // `is_visible`. A disabled one produces no trigger but may still be drawn,
    // so the index is counted over the ones the physics did build.
    let mut index = 0usize;
    for item in &vpx.gameitems {
        let GameItemEnum::Trigger(t) = item else {
            continue;
        };
        if !t.is_enabled {
            continue;
        }
        let trigger = index;
        index += 1;

        let base_z = surface_height(vpx, &t.surface, t.center.x, t.center.y);
        let Some(mesh) = crate::trigger::build(t, base_z) else {
            continue;
        };
        // `trigger::build` leaves the vertices in a local frame precisely for
        // this: the sinking is a translation in the matrix and nothing else.
        let base = mesh.transform;
        out.push(AnimatedPart {
            mesh,
            base,
            local: Mat4::IDENTITY,
            anim: Animation::Trigger { trigger },
        });
    }
}

/// The height of the surface a part stands on, **where that part stands**.
/// See [`crate::builtin::surface_height`]: on a ramp the answer moves.
fn surface_height(vpx: &VPX, surface: &str, x: f32, y: f32) -> f32 {
    crate::builtin::surface_height(&vpx.gameitems, surface, x, y)
}

/// The names of the meshes that get animated.
///
/// The static scene has to **exclude** them: otherwise every moving piece is
/// drawn twice, once baked into its rest position and once following the
/// physics, and what you see is a ghost flipper nailed to the starting angle.
pub fn names(parts: &[AnimatedPart]) -> Vec<String> {
    parts.iter().map(|m| m.mesh.name.clone()).collect()
}
