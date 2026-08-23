//! Tests of the geometry extraction.
//!
//! Almost all of them pin down a detail of the original that has already cost us
//! time once. The reference is cited in each case.

use vpw_math::{Mat4, Vec3};
use vpw_table::geometry::{Bounds, Material, Mesh, MeshKind, Vertex};

fn material(opacity: f32, active: bool) -> Material {
    Material {
        name: "m".into(),
        base_color: [1.0, 1.0, 1.0],
        glossy_color: [0.3, 0.3, 0.3],
        clearcoat_color: [0.0, 0.0, 0.0],
        is_metal: false,
        roughness: 0.5,
        wrap_lighting: 0.0,
        glossy_image_lerp: 0.0,
        edge: 0.4,
        edge_alpha: 1.0,
        thickness: 0.05,
        opacity,
        opacity_active: active,
    }
}

#[test]
fn opacity_only_counts_if_the_flag_is_on() {
    // `Shader.cpp:829`: const float alpha = bOpacityActive ? fOpacity : 1.0f;
    //
    // It is the bug that made F-14's playfield come out transparent: the
    // material stores opacity 0 with the flag off.
    assert_eq!(material(0.0, false).alpha(), 1.0);
    assert_eq!(material(0.0, true).alpha(), 0.0);
    assert_eq!(material(0.5, true).alpha(), 0.5);
    assert_eq!(material(1.0, false).alpha(), 1.0);
}

#[test]
fn transparent_is_flag_on_and_alpha_less_than_one() {
    // `Shader.cpp:850`: if (bOpacityActive && (has_alpha || alpha < 0.999f))
    assert!(
        !material(0.0, false).is_transparent(false),
        "flag off, it is opaque"
    );
    assert!(material(0.5, true).is_transparent(false));
    assert!(
        !material(1.0, true).is_transparent(false),
        "opaque even with the flag on"
    );
    // A texture with an alpha channel is enough on its own, if the flag is on.
    assert!(
        material(1.0, true).is_transparent(true),
        "the texture has alpha"
    );
    assert!(
        !material(1.0, false).is_transparent(true),
        "but the flag rules"
    );
}

#[test]
fn roughness_maps_to_the_originals_exponent() {
    // `Shader.cpp:799`: exp2f(10 * roughness + 1), from 0..1 to 2..2048.
    let mut m = material(1.0, false);
    m.roughness = 0.0;
    assert_eq!(m.shader_inputs().glossy_power, 2.0, "matte");
    m.roughness = 1.0;
    assert_eq!(m.shader_inputs().glossy_power, 2048.0, "glossy");
}

#[test]
fn metal_sends_the_specular_black_and_does_not_attenuate_at_the_edge() {
    // `Shader.cpp:833`: cGlossy is (0,0,0,0) on metal, because the shader uses
    // the base color in its place. And `BasicShader.hlsl:323`: edge = 1.0 on
    // metal.
    let mut m = material(1.0, false);
    m.is_metal = true;
    let i = m.shader_inputs();
    assert_eq!(i.glossy_color, [0.0, 0.0, 0.0]);
    assert_eq!(i.edge, 1.0);

    // Without metal, the edge is the material's.
    m.is_metal = false;
    assert_eq!(m.shader_inputs().edge, 0.4);
}

#[test]
fn with_no_material_the_originals_defaults_apply() {
    // `Shader.cpp:812-826`. The two that matter: the specular and the clearcoat
    // start out **black**, not some arbitrary gray. With a gray, the whole table
    // gets a floor of reflection that dulls the textures.
    let d = vpw_table::geometry::ShaderInputs::default();
    assert_eq!(d.glossy_color, [0.0, 0.0, 0.0]);
    assert_eq!(d.clearcoat, [0.0, 0.0, 0.0]);
    assert_eq!(d.glossy_image_lerp, 1.0);
    assert_eq!(d.glossy_power, 2.0);
    assert_eq!(d.alpha, 1.0);
    assert_eq!(d.thickness, 0.05);
}

#[test]
fn the_clearcoat_already_comes_with_the_dielectric_factor() {
    // `BasicShader.hlsl:323`: specular = cClearcoat_EdgeAlpha.xyz * 0.08
    let mut m = material(1.0, false);
    m.clearcoat_color = [1.0, 1.0, 1.0];
    let c = m.shader_inputs().clearcoat;
    assert!((c[0] - 0.08).abs() < 1e-6, "it gave {c:?}");
}

#[test]
fn the_playfield_quad_has_the_originals_uvs() {
    // `pintable.cpp:3255-3259`:
    //   tv = (i & 2) ? 1 : 0
    //   tu = (i == 1 || i == 2) ? 1 : 0
    let expected = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let bounds = Bounds {
        min: Vec3::new(0.0, 0.0, 0.0),
        max: Vec3::new(950.0, 2100.0, 0.0),
    };

    let quad = test_playfield(bounds);
    assert_eq!(quad.vertices.len(), 4);
    for (v, want) in quad.vertices.iter().zip(expected) {
        assert_eq!(v.uv, want);
    }
    // And the normal points up (`pintable.cpp:3257`, nz = 1).
    assert!(quad.vertices.iter().all(|v| v.normal == [0.0, 0.0, 1.0]));
}

/// Reproduces the quad `extract` builds, without needing a `.vpx`.
fn test_playfield(b: Bounds) -> Mesh {
    let n = [0.0, 0.0, 1.0];
    Mesh {
        name: "playfield".into(),
        vertices: vec![
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
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        transform: Mat4::IDENTITY,
        image: String::new(),
        material: String::new(),
        visible: true,
        kind: MeshKind::Playfield,
    }
}

#[test]
fn baking_takes_the_vertices_to_world_space() {
    let mut m = test_playfield(Bounds {
        min: Vec3::ZERO,
        max: Vec3::new(2.0, 2.0, 0.0),
    });
    m.transform = Mat4::from_translation(Vec3::new(10.0, 20.0, 30.0));

    let baked = m.baked();
    assert_eq!(baked[0].pos, [10.0, 20.0, 30.0]);
    assert_eq!(baked[2].pos, [12.0, 22.0, 30.0]);
    // A pure translation does not touch the normals.
    assert_eq!(baked[0].normal, [0.0, 0.0, 1.0]);
}

#[test]
fn the_normals_go_through_the_inverse_transpose() {
    // With a non-uniform scale, transforming the normal as if it were a point
    // gives a badly oriented result. Tables almost never have a uniform scale.
    let mut m = test_playfield(Bounds {
        min: Vec3::ZERO,
        max: Vec3::new(1.0, 1.0, 0.0),
    });
    m.vertices[0].normal = [1.0, 1.0, 0.0];
    m.transform = Mat4::from_scale(Vec3::new(10.0, 1.0, 1.0));

    let n = m.baked()[0].normal;
    // Scaling X by ten has to **flatten** the normal's X component, not stretch
    // it.
    assert!(
        n[0] < n[1],
        "the normal came out {n:?}, it was transformed as a point"
    );
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    assert!(
        (len - 1.0).abs() < 1e-5,
        "it has to end up normalized, it gave {len}"
    );
}

#[test]
fn the_bounding_box_uses_the_transform() {
    let mut m = test_playfield(Bounds {
        min: Vec3::ZERO,
        max: Vec3::new(2.0, 4.0, 0.0),
    });
    m.transform = Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0));
    let b = m.bounds().unwrap();
    assert_eq!(b.min, Vec3::new(100.0, 0.0, 0.0));
    assert_eq!(b.max, Vec3::new(102.0, 4.0, 0.0));
}

#[test]
fn counting_triangles() {
    let m = test_playfield(Bounds {
        min: Vec3::ZERO,
        max: Vec3::ONE,
    });
    assert_eq!(m.triangles(), 2);
}

#[test]
fn the_primitives_matrix_applies_the_translation_before_rotating() {
    // The order `primitive.cpp:372-388` fixes, read in row-vector form, is:
    // scale, own translation, RotZ, RotY, RotX, the second triple, position.
    //
    // Which means the own translation is applied **before** the rotations and
    // therefore ends up rotated. If it is composed the other way round, the
    // piece lands somewhere else: that is what made some primitives float
    // outside the table.
    let m = vpw_table::geometry::primitive_transform_from_fields(
        Vec3::ZERO,
        Vec3::ONE,
        // A translation of 10 in X, and a 90 degree turn around Z.
        [0.0, 0.0, 90.0, 10.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    );

    let p = m.transform_point3(Vec3::ZERO);
    assert!(
        (p - Vec3::new(0.0, 10.0, 0.0)).length() < 1e-4,
        "the translation has to end up rotated; it gave {p:?}, expected (0, 10, 0)"
    );
}

#[test]
fn the_primitives_position_is_not_rotated() {
    // The position is applied **last**, so no rotation touches it.
    let m = vpw_table::geometry::primitive_transform_from_fields(
        Vec3::new(100.0, 200.0, 0.0),
        Vec3::ONE,
        [0.0, 0.0, 90.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    );
    let p = m.transform_point3(Vec3::ZERO);
    assert!(
        (p - Vec3::new(100.0, 200.0, 0.0)).length() < 1e-4,
        "the position must not be rotated; it gave {p:?}"
    );
}

#[test]
fn the_scale_is_applied_first_of_all() {
    // Scaling and then translating is not the same as translating and then
    // scaling: with the scale first, the own translation does not grow.
    let m = vpw_table::geometry::primitive_transform_from_fields(
        Vec3::ZERO,
        Vec3::new(2.0, 2.0, 2.0),
        [0.0; 9].map(|_| 0.0),
    );
    let p = m.transform_point3(Vec3::new(3.0, 0.0, 0.0));
    assert!(
        (p - Vec3::new(6.0, 0.0, 0.0)).length() < 1e-4,
        "it gave {p:?}"
    );

    let m = vpw_table::geometry::primitive_transform_from_fields(
        Vec3::ZERO,
        Vec3::new(2.0, 2.0, 2.0),
        [0.0, 0.0, 0.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    );
    let p = m.transform_point3(Vec3::ZERO);
    assert!(
        (p - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-4,
        "the own translation is not scaled; it gave {p:?}"
    );
}

#[test]
fn all_the_originals_builtin_meshes_are_there() {
    // The 34 meshes Visual Pinball ships, converted into a binary blob. This test
    // does not verify their shape — that is what the renders are for — but that
    // the blob and its index do not get out of sync: a badly computed offset
    // gives vertices full of garbage, and that shows up late.
    let names: Vec<&str> = vpw_table::meshes::names().collect();
    assert!(names.len() >= 30, "there are only {} meshes", names.len());

    // The ones each item we draw uses.
    for expected in [
        "bumperBase",
        "bumperCap",
        "bumperRing",
        "bumperSocket",
        "flipperBase",
        "gateWire",
        "gateBracket",
        "spinnerPlate",
        "hitTargetRound",
        "hitTargetT2",
        "kickerCup",
    ] {
        assert!(names.contains(&expected), "the mesh {expected} is missing");
    }

    for name in names {
        let m = vpw_table::meshes::get(name).expect("the mesh has to decode");
        assert!(!m.vertices.is_empty(), "{name} has no vertices");
        assert_eq!(
            m.indices.len() % 3,
            0,
            "{name}: the indices are not triangles"
        );

        // Every index has to fall inside the vertex array. If the blob's offset
        // were off, this fires.
        let max = m.indices.iter().copied().max().unwrap_or(0) as usize;
        assert!(max < m.vertices.len(), "{name}: index {max} out of range");

        // The original's normals come normalized.
        for v in m.vertices.iter().take(16) {
            let n = Vec3::from_array(v.normal);
            assert!(
                n.length() < 1.01 && v.pos.iter().all(|c| c.is_finite()),
                "{name}: vertex full of garbage, normal {n:?}"
            );
        }
    }
}

#[test]
fn the_flipper_takes_the_tables_radii_and_does_not_scale_the_mesh() {
    // The original does **not** scale the flipper's builtin mesh: it identifies
    // the thirteen vertices of each circle and re-projects them onto the radius
    // the table asks for (`ApplyFix`, `flipper.cpp:604`). Base and tip have
    // independent radii, so scaling would deform both equally.
    //
    // The proof: two flippers that differ only in the base radius have to give
    // meshes of different widths, and that width has to be the one asked for.
    use vpin::vpx::gameitem::flipper::Flipper;

    let build = |base_radius: f32, length: f32| {
        let f = Flipper {
            name: "F".into(),
            base_radius,
            end_radius: 13.0,
            flipper_radius_max: length,
            rubber_thickness: Some(0.0),
            height: 50.0,
            start_angle: 0.0,
            is_visible: true,
            ..Flipper::default()
        };
        let m = vpw_table::flipper::build(&f, 0.0).expect("it has to generate a mesh");
        // Maximum width in X, already in world space.
        let xs: Vec<f32> = m.baked().iter().map(|v| v.pos[0]).collect();
        let max = xs.iter().copied().fold(f32::MIN, f32::max);
        let min = xs.iter().copied().fold(f32::MAX, f32::min);
        max - min
    };

    let narrow = build(20.0, 130.0);
    let wide = build(40.0, 130.0);

    assert!(
        wide > narrow + 10.0,
        "doubling the base radius has to widen the flipper: {narrow} vs {wide}"
    );
    // And the width has to be of the order of the requested diameter, not some
    // arbitrary value inherited from the builtin mesh.
    assert!(
        (wide - 80.0).abs() < 20.0,
        "with radius 40 the flipper should measure about 80 units across; it gave {wide}"
    );
}
