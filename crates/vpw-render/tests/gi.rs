//! The general illumination, measured.
//!
//! One lit bulb over a dark floor in a room with no other light: the floor
//! under it must brighten, because the bulb is a light and not only a halo.
//! The same floor with the bulb off must stay dark. See `gi_diffuse` in
//! `material.wgsl` for the departure this pins.

use std::sync::{Mutex, MutexGuard, OnceLock};
use vpw_math::{Mat4, Vec3};
use vpw_render::Camera;
use vpw_render::offscreen::Offscreen;
use vpw_table::geometry::{
    Bounds, Image, Lighting, Material, Mesh, MeshKind, Scene, TablePhysics, Vertex,
};
use vpw_table::light::{Fader, Light};

const W: u32 = 128;
const H: u32 = 128;
const TABLE: f32 = 1000.0;

static GPU: OnceLock<Option<Mutex<Offscreen>>> = OnceLock::new();

fn gpu() -> Option<MutexGuard<'static, Offscreen>> {
    let cell = GPU.get_or_init(|| match pollster::block_on(Offscreen::new(W, H)) {
        Ok(g) => Some(Mutex::new(g)),
        Err(e) => {
            eprintln!("skipped: no GPU adapter ({e})");
            None
        }
    });
    Some(
        cell.as_ref()?
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

/// A grey floor and one bulb light floating over its middle, covering most of
/// it, lit or not. With `wall`, an opaque fin stands east of the lamp,
/// between it and the east half of the floor.
fn scene_with(state: f32, name: &str, wall: bool) -> Scene {
    // The playfield convention: UV spans the field, `(0,0)` at the minimum
    // corner — the lightmap's space.
    let v = |x: f32, y: f32| Vertex {
        pos: [x, y, 0.0],
        normal: [0.0, 0.0, 1.0],
        uv: [x / TABLE, y / TABLE],
    };
    let floor = Mesh {
        name: "floor".into(),
        vertices: vec![v(0.0, 0.0), v(TABLE, 0.0), v(TABLE, TABLE), v(0.0, TABLE)],
        indices: vec![0, 1, 2, 0, 2, 3],
        transform: Mat4::IDENTITY,
        image: String::new(),
        material: "floor".into(),
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Playfield,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    };
    let c = TABLE / 2.0;
    // The shape is tiny on purpose: what this measures is the *illumination*
    // of the floor around the bulb, not the halo painted over its own shape.
    let corners = [
        [c - 5.0, c - 5.0],
        [c + 5.0, c - 5.0],
        [c + 5.0, c + 5.0],
        [c - 5.0, c + 5.0],
    ];
    let bulb = Light {
        scenery: false,
        name: name.into(),
        vertices: corners.iter().map(|p| [p[0], p[1], 30.0]).collect(),
        indices: vec![0, 1, 2, 0, 2, 3],
        uvs: Vec::new(),
        image: String::new(),
        image_mode: false,
        surface_material: "floor".into(),
        surface_image: String::new(),
        center: Vec3::new(c, c, 30.0),
        falloff_radius: 400.0,
        falloff_power: 2.0,
        intensity: 15.0,
        color: [1.0, 0.6, 0.4],
        color2: [1.0, 0.6, 0.4],
        state,
        blinking: false,
        is_bulb: true,
        transmission_scale: 0.0,
        modulate: 1.0,
        fader: Fader::None,
        fade_up: 0.2,
        fade_down: 0.2,
        blink: vec![true],
        blink_interval: 125.0,
    };

    let mut meshes = vec![floor];
    if wall {
        // A fin from the floor up past the lamp, at x = 600: everything east
        // of it is in its shadow.
        let wv = |y: f32, z: f32| Vertex {
            pos: [600.0, y, z],
            normal: [1.0, 0.0, 0.0],
            uv: [0.0, 0.0],
        };
        meshes.push(Mesh {
            name: "wall".into(),
            vertices: vec![
                wv(200.0, 0.0),
                wv(800.0, 0.0),
                wv(800.0, 60.0),
                wv(200.0, 60.0),
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            transform: Mat4::IDENTITY,
            image: String::new(),
            material: "floor".into(),
            visible: true,
            clamp: false,
            scenery: false,
            kind: MeshKind::Wall,
            additive: None,
            depth_bias: 0.0,
            disable_lighting: 0.0,
        });
    }

    Scene {
        view: vpw_table::geometry::AuthoredView::default(),
        cabinet: vpw_table::geometry::AuthoredView::default(),
        built_head: true,
        meshes,
        physics: TablePhysics {
            slope_deg: 6.0,
            gravity: 0.0,
            default_scatter_deg: 0.0,
            difficulty: 0.0,
        },
        materials: vec![Material {
            name: "floor".into(),
            base_color: [0.4, 0.4, 0.4],
            glossy_color: [0.0; 3],
            clearcoat_color: [0.0; 3],
            is_metal: false,
            roughness: 0.0,
            wrap_lighting: 0.0,
            glossy_image_lerp: 0.0,
            edge: 1.0,
            edge_alpha: 1.0,
            thickness: 0.05,
            opacity: 1.0,
            opacity_active: false,
        }],
        images: Vec::new(),
        playfield: Bounds {
            min: Vec3::new(0.0, 0.0, 0.0),
            max: Vec3::new(TABLE, TABLE, 0.0),
        },
        playfield_image: String::new(),
        playfield_material: "floor".into(),
        lighting: Lighting {
            lights: [Vec3::new(c, c, 800.0); 2],
            emission: [0.0; 3],
            ambient: [0.0; 3],
            range: 3000.0,
            env_scale: 0.0,
            global: 1.0,
            exposure: 1.0,
            bloom_strength: 0.0,
            reflection_strength: 0.0,
        },
        lights: vec![bulb],
        env_image: String::new(),
        ball_decal: String::new(),
        backdrop_image: String::new(),
        backdrop_color: [0.0; 3],
        flashers: Vec::new(),
    }
}

/// A point on the floor a third of the table from the bulb: inside its range,
/// well outside its own drawn shape.
fn floor_sample(pixels: &[u8]) -> i32 {
    let (x, y) = (W / 2 + W / 5, H / 2);
    let i = ((y * W + x) * 4) as usize;
    i32::from(pixels[i]) + i32::from(pixels[i + 1]) + i32::from(pixels[i + 2])
}

fn shoot(gpu: &mut Offscreen, scene: &Scene) -> Vec<u8> {
    let mut camera = Camera::framing(
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(TABLE, TABLE, 0.0),
        W as f32 / H as f32,
    );
    camera.inclination = 89.0;
    let uploaded = gpu.upload(scene);
    gpu.upload_lights(scene);
    gpu.set_bloom(0.0);
    gpu.render(&uploaded, &camera)
}

fn scene(state: f32) -> Scene {
    scene_with(state, "gi", false)
}

#[test]
fn a_lit_bulb_lights_the_floor_around_it_and_an_unlit_one_does_not() {
    let Some(mut gpu) = gpu() else { return };

    let dark = floor_sample(&shoot(&mut gpu, &scene(0.0)));
    let lit = floor_sample(&shoot(&mut gpu, &scene(1.0)));

    assert!(
        lit > dark + 60,
        "the bulb sheds no light on the floor: off {dark}, on {lit}"
    );
}

/// A red texel out of one bake layer, by table position.
fn layer_texel(bake: &vpw_render::bake::GiBakeSet, layer: usize, x: f32, y: f32) -> f32 {
    use vpw_render::bake::{BAKE_H, BAKE_W};
    let i = (x / TABLE * BAKE_W as f32) as u32;
    let j = (y / TABLE * BAKE_H as f32) as u32;
    let at = ((j * BAKE_W + i) * 4) as usize;
    half::f16::from_bits(bake.layers[layer][at]).to_f32()
}

/// The bake itself, with no GPU anywhere: a wall east of the lamp must put
/// the east half of the floor in shadow and leave the west half lit.
#[test]
fn the_bake_draws_the_wall_s_shadow() {
    use vpw_render::bake::{bake_gi_set, gi_groups};

    let scene = scene_with(1.0, "GI_1", true);
    let groups = gi_groups(&scene);
    assert_eq!(groups.len(), 1, "the GI-named bulb is the group");
    assert_eq!(groups[0].indices, vec![0]);
    let bake = bake_gi_set(&scene, &groups, 0);

    // Two texels equally far from the lamp, either side of the wall.
    let lit = layer_texel(&bake, 0, 350.0, 500.0);
    let shadowed = layer_texel(&bake, 0, 650.0, 500.0);
    assert!(lit > 0.0, "the open side is lit: {lit}");
    assert!(
        shadowed < lit * 0.05,
        "the wall casts no shadow: lit {lit}, behind the wall {shadowed}"
    );
}

/// The bounce, measured against its absence. The geometry is a corner a
/// mirror test would recognise: a low fence shadows the floor east of it, and
/// a tall white wall further east still sees the lamp over the fence — so a
/// texel in the fence's shadow gets nothing directly and something off the
/// white wall, which is light turning a corner.
#[test]
fn one_bounce_carries_light_into_the_shadow() {
    use vpw_render::bake::{bake_gi_set, gi_groups};

    let mut scene = scene_with(1.0, "GI_1", false);
    let fin = |name: &str, x: f32, z_top: f32| {
        let v = |y: f32, z: f32| Vertex {
            pos: [x, y, z],
            normal: [1.0, 0.0, 0.0],
            uv: [0.0, 0.0],
        };
        Mesh {
            name: name.into(),
            vertices: vec![
                v(200.0, 0.0),
                v(800.0, 0.0),
                v(800.0, z_top),
                v(200.0, z_top),
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            transform: Mat4::IDENTITY,
            image: String::new(),
            material: "wall".into(),
            visible: true,
            clamp: false,
            scenery: false,
            kind: MeshKind::Wall,
            additive: None,
            depth_bias: 0.0,
            disable_lighting: 0.0,
        }
    };
    // The fence: below the lamp at z = 30, so it shadows the floor but not
    // the wall behind it.
    scene.meshes.push(fin("fence", 600.0, 25.0));
    // The reflector, lit over the fence's top.
    scene.meshes.push(fin("reflector", 800.0, 120.0));

    let groups = gi_groups(&scene);
    let flat = bake_gi_set(&scene, &groups, 0);
    let bounced = bake_gi_set(&scene, &groups, 64);

    // In the fence's shadow, between fence and reflector.
    let (x, y) = (700.0, 500.0);
    let direct_only = layer_texel(&flat, 0, x, y);
    let with_bounce = layer_texel(&bounced, 0, x, y);
    assert_eq!(direct_only, 0.0, "the fence shadows the spot");
    assert!(
        with_bounce > 0.0,
        "the bounce adds nothing: direct {direct_only}, bounced {with_bounce}"
    );
}

/// The bounce map comes out smooth: sixteen Monte Carlo samples speckle, and
/// the blur is what turns them into light. Measured texel against neighbour
/// in the open field near the reflector, where the indirect signal is real
/// and would show the noise.
#[test]
fn the_indirect_map_is_smooth_where_the_light_is() {
    use vpw_render::bake::{bake_gi_set, gi_groups};

    let mut scene = scene_with(1.0, "GI_1", false);
    let v = |y: f32, z: f32| Vertex {
        pos: [800.0, y, z],
        normal: [1.0, 0.0, 0.0],
        uv: [0.0, 0.0],
    };
    scene.meshes.push(Mesh {
        name: "reflector".into(),
        vertices: vec![
            v(200.0, 0.0),
            v(800.0, 0.0),
            v(800.0, 120.0),
            v(200.0, 120.0),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        transform: Mat4::IDENTITY,
        image: String::new(),
        material: "wall".into(),
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Wall,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    });
    let groups = gi_groups(&scene);
    let bake = bake_gi_set(&scene, &groups, 16);

    // The direct light at these texels is identical by symmetry-ish; what
    // varies texel to texel is the gathered bounce. Neighbours a texel apart
    // must agree to well within the level itself.
    let mut worst = 0.0f32;
    for step in 0..8 {
        let y = 460.0 + step as f32 * 10.0;
        let a = layer_texel(&bake, 0, 700.0, y);
        let b = layer_texel(&bake, 0, 700.0, y + 5.0);
        if a > 0.0 {
            worst = worst.max((a - b).abs() / a.max(1e-6));
        }
    }
    assert!(
        worst < 0.35,
        "the bounce speckles: neighbours differ by {:.0}%",
        worst * 100.0
    );
}

/// Two GI strings of two colours are two groups with two layers, and each
/// lamp lands in its own.
#[test]
fn strings_of_different_colours_become_their_own_groups() {
    use vpw_render::bake::{bake_gi_set, gi_groups};

    let mut scene = scene_with(1.0, "GI_1", false);
    let mut red = scene.lights[0].clone();
    red.name = "GI_Red_1".into();
    red.color = [1.0, 0.0, 0.0];
    red.color2 = [1.0, 0.0, 0.0];
    red.center = Vec3::new(250.0, 500.0, 30.0);
    scene.lights.push(red);

    let groups = gi_groups(&scene);
    assert_eq!(groups.len(), 2, "one group per colour");
    let bake = bake_gi_set(&scene, &groups, 0);
    assert_eq!(bake.layers.len(), 2);

    // Whichever layer the red string landed in is red where the red lamp is.
    let red_layer = usize::from(groups[0].names[0] != "GI_Red_1");
    let at = |layer: usize, c: usize| {
        use vpw_render::bake::{BAKE_H, BAKE_W};
        let i = (250.0 / TABLE * BAKE_W as f32) as u32;
        let j = (500.0 / TABLE * BAKE_H as f32) as u32;
        let base = ((j * BAKE_W + i) * 4) as usize;
        half::f16::from_bits(bake.layers[layer][base + c]).to_f32()
    };
    assert!(at(red_layer, 0) > 0.0, "red where the red lamp is");
    assert_eq!(at(red_layer, 2), 0.0, "and no blue in the red layer");
}

/// Steel shows the light around it — by reflection, which is the only way a
/// metal answers. A metal wall standing on the lit field must mirror the
/// field: its reflected rays dive to the floor, and the floor's baked light
/// comes back off the steel. (A flat ambient once stood in for this and made
/// every sphere look like wax; this is the test that replaced it.)
#[test]
fn a_metal_wall_mirrors_the_baked_field() {
    let Some(mut gpu) = gpu() else { return };
    let mut scene = scene_with(1.0, "GI_1", false);

    // The floor needs a picture: the planar reflection samples the field's
    // image, and a field painted by material alone reflects the blank.
    scene.images.push(Image {
        name: "wood".into(),
        encoded: None,
        rgba: Some(vec![255u8; 8 * 8 * 4]),
        width: 8,
        height: 8,
        has_alpha: false,
        alpha_test: -1.0,
        redrawn: false,
    });
    scene.playfield_image = "wood".into();
    scene.meshes[0].image = "wood".into();

    // A steel fin mid-field, its face toward the camera, standing where the
    // lamp lights the floor in front of it.
    let v = |x: f32, z: f32| Vertex {
        pos: [x, 500.0, z],
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 0.0],
    };
    scene.materials.push(vpw_table::geometry::Material {
        name: "steel".into(),
        base_color: [0.85, 0.85, 0.88],
        glossy_color: [1.0, 1.0, 1.0],
        clearcoat_color: [0.0; 3],
        is_metal: true,
        roughness: 0.95,
        wrap_lighting: 0.0,
        glossy_image_lerp: 1.0,
        edge: 1.0,
        edge_alpha: 1.0,
        thickness: 0.05,
        opacity: 1.0,
        opacity_active: false,
    });
    scene.meshes.push(Mesh {
        name: "fin".into(),
        vertices: vec![
            v(350.0, 0.0),
            v(650.0, 0.0),
            v(650.0, 120.0),
            v(350.0, 120.0),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        transform: Mat4::IDENTITY,
        image: String::new(),
        material: "steel".into(),
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Wall,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    });

    let mut camera = Camera::framing(
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(TABLE, TABLE, 0.0),
        W as f32 / H as f32,
    );
    camera.inclination = 40.0;

    let wall_sample = |pixels: &[u8]| {
        // The fin's face lands mid-frame under this framing; take the
        // brightest pixel of a small window on it, since the exact row
        // depends on the projection.
        let mut best = 0i32;
        for y in H * 2 / 5..H * 3 / 5 {
            for x in W * 2 / 5..W * 3 / 5 {
                let i = ((y * W + x) * 4) as usize;
                best = best.max(pixels[i] as i32 + pixels[i + 1] as i32 + pixels[i + 2] as i32);
            }
        }
        best
    };

    let uploaded = gpu.upload(&scene);
    gpu.upload_lights(&scene);
    gpu.set_bloom(0.0);
    let before = wall_sample(&gpu.render(&uploaded, &camera));
    let baked = gpu.bake_gi(&scene);
    assert_eq!(baked, 1);
    let after = wall_sample(&gpu.render(&uploaded, &camera));

    assert!(
        after > before + 30,
        "the steel does not mirror the baked field: before {before}, after {after}"
    );
}

/// A table that ships its own lightmaps gets none of the departure: no
/// groups to bake, no point lights, no bounce.
#[test]
fn a_prebaked_table_switches_the_whole_departure_off() {
    use vpw_render::bake::gi_groups;

    let mut scene = scene_with(1.0, "GI_1", false);
    // The 10.8 lightmap pattern: a flasher bound to a lamp.
    let corner = |x: f32, y: f32| vpin::vpx::gameitem::dragpoint::DragPoint {
        x,
        y,
        z: 0.0,
        smooth: false,
        ..Default::default()
    };
    let lightmap = vpin::vpx::gameitem::flasher::Flasher {
        name: "LM".into(),
        is_visible: true,
        light_map: Some("GI_1".into()),
        drag_points: vec![
            corner(100.0, 100.0),
            corner(200.0, 100.0),
            corner(200.0, 200.0),
            corner(100.0, 200.0),
        ],
        ..Default::default()
    };
    scene
        .flashers
        .extend(vpw_table::flasher::build(&lightmap, scene.playfield));
    assert!(!scene.flashers.is_empty(), "the lightmap flasher builds");

    assert!(gi_groups(&scene).is_empty(), "nothing to bake");

    let Some(mut gpu) = gpu() else { return };
    let uploaded = gpu.upload(&scene);
    gpu.upload_lights(&scene);
    let gi = gpu.lights.gi_sources(32);
    assert!(gi.rows.is_empty(), "no point lights");
    assert_eq!(gi.bounce, [0.0; 3], "no bounce");
    drop(uploaded);
}

/// The whole path on the GPU: with the bake applied, the shadowed side of the
/// floor is darker than the open side; without it, the point light shines
/// straight through the wall and the two sides match.
#[test]
fn the_baked_shadow_reaches_the_picture() {
    let Some(mut gpu) = gpu() else { return };
    let scene = scene_with(1.0, "GI_1", true);

    let mut camera = Camera::framing(
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(TABLE, TABLE, 0.0),
        W as f32 / H as f32,
    );
    camera.inclination = 89.0;

    let side = |pixels: &[u8], x: u32| {
        let i = ((H / 2 * W + x) * 4) as usize;
        pixels[i] as i32 + pixels[i + 1] as i32 + pixels[i + 2] as i32
    };

    // Without the bake: the point light knows nothing of the wall.
    let uploaded = gpu.upload(&scene);
    gpu.upload_lights(&scene);
    gpu.set_bloom(0.0);
    let flat = gpu.render(&uploaded, &camera);
    let (open_flat, behind_flat) = (side(&flat, W * 2 / 5), side(&flat, W * 3 / 5));

    // With it: the shadow is in the map, and the lamp leaves the point table.
    let baked = gpu.bake_gi(&scene);
    assert_eq!(baked, 1, "one group");
    let shot = gpu.render(&uploaded, &camera);
    let (open, behind) = (side(&shot, W * 2 / 5), side(&shot, W * 3 / 5));

    assert!(
        (open_flat - behind_flat).abs() < 25,
        "without the bake the wall should not shadow: {open_flat} vs {behind_flat}"
    );
    assert!(
        behind + 40 < open,
        "the baked shadow does not show: open {open}, behind {behind}"
    );
}
