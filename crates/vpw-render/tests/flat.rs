//! The flat engine, measured against the renderer it photographs.
//!
//! The whole promise of `crate::flat` is that a player on a weak machine sees
//! *the same picture*: the base photograph is the real render of the dark
//! table, and each lamp's sprite is the real render's own difference. So the
//! test is the promise, verbatim — photograph the table flat, render it for
//! real, and compare the pixels. Then flip a lamp and compare again, because
//! the sprites' entire job is to track the lamps live.
//!
//! Like the other renderer tests these need a GPU adapter, share one behind a
//! mutex, and skip themselves when there is none.

use std::sync::{Mutex, MutexGuard, OnceLock};
use vpw_math::{Mat4, Vec3};
use vpw_render::Camera;
use vpw_render::offscreen::Offscreen;
use vpw_table::geometry::{Bounds, Lighting, Mesh, MeshKind, Scene, Vertex};
use vpw_table::light::Light;

const W: u32 = 160;
const H: u32 = 160;

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

/// A floor with an insert lamp on it — the smallest scene where the flat
/// engine has both a base to photograph and a lamp to make a sprite of.
fn floor_with_lamp() -> Scene {
    let half = 500.0;
    let v = |x: f32, y: f32| Vertex {
        pos: [x, y, 0.0],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 0.0],
    };
    let floor = Mesh {
        name: "Floor".into(),
        vertices: vec![
            v(-half, -half),
            v(half, -half),
            v(half, half),
            v(-half, half),
        ],
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

    let lamp = 60.0;
    let light = Light {
        scenery: false,
        name: "Lamp".into(),
        vertices: vec![
            [-lamp, -lamp, 1.0],
            [lamp, -lamp, 1.0],
            [lamp, lamp, 1.0],
            [-lamp, lamp, 1.0],
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        center: Vec3::new(0.0, 0.0, 1.0),
        falloff_radius: lamp * 1.5,
        falloff_power: 2.0,
        intensity: 12.0,
        color: [1.0, 0.9, 0.6],
        color2: [1.0, 0.9, 0.6],
        state: 1.0,
        blinking: false,
        is_bulb: false,
        transmission_scale: 0.5,
        modulate: 0.0,
        // One frame against one frame: a fade would measure the fade.
        fader: vpw_table::light::Fader::None,
        fade_up: 0.2,
        fade_down: 0.2,
        blink: vec![true],
        blink_interval: 125.0,
        uvs: Vec::new(),
        image: String::new(),
        image_mode: false,
        surface_material: "floor".into(),
        surface_image: String::new(),
    };

    Scene {
        view: vpw_table::geometry::AuthoredView::default(),
        cabinet: vpw_table::geometry::AuthoredView::default(),
        built_head: true,
        meshes: vec![floor],
        physics: vpw_table::geometry::TablePhysics {
            slope_deg: 6.0,
            gravity: 0.0,
            default_scatter_deg: 0.0,
            difficulty: 0.0,
        },
        materials: vec![vpw_table::geometry::Material {
            name: "floor".into(),
            base_color: [0.3, 0.3, 0.3],
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
            min: Vec3::new(-half, -half, 0.0),
            max: Vec3::new(half, half, 0.0),
        },
        playfield_image: String::new(),
        playfield_material: "floor".into(),
        env_image: String::new(),
        ball_decal: String::new(),
        backdrop_image: String::new(),
        backdrop_color: [0.0; 3],
        lighting: Lighting {
            lights: [Vec3::new(0.0, 0.0, 800.0), Vec3::new(0.0, 0.0, 800.0)],
            emission: [0.4, 0.4, 0.4],
            ambient: [0.1, 0.1, 0.1],
            range: 3000.0,
            env_scale: 0.0,
            global: 1.0,
            exposure: 1.0,
            bloom_strength: 0.0,
            reflection_strength: 0.0,
        },
        lights: vec![light],
        flashers: Vec::new(),
    }
}

fn top_down() -> Camera {
    let mut c = Camera::framing(
        Vec3::new(-500.0, -500.0, 0.0),
        Vec3::new(500.0, 500.0, 0.0),
        W as f32 / H as f32,
    );
    c.inclination = 89.0;
    c
}

/// Mean absolute difference per channel, 0-255 scale.
fn mean_diff(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let sum: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    sum as f64 / a.len() as f64
}

fn mean(img: &[u8]) -> f64 {
    img.iter().map(|&v| u64::from(v)).sum::<u64>() as f64 / img.len() as f64
}

#[test]
fn the_flat_photograph_matches_the_render() {
    let Some(mut gpu) = gpu() else { return };
    let scene = floor_with_lamp();
    let camera = top_down();
    let uploaded = gpu.upload(&scene);
    gpu.upload_lights(&scene);
    gpu.set_bloom(0.0);

    // The real render, lamp lit — the file leaves it lit.
    let real = gpu.render(&uploaded, &camera);
    assert!(mean(&real) > 1.0, "the reference must not be black");

    // The same frame out of the photographs.
    gpu.flat_on(&uploaded, &camera);
    let flat = gpu.render(&uploaded, &camera);
    let diff = mean_diff(&real, &flat);
    assert!(
        diff < 3.0,
        "the flat frame must be the rendered frame: mean difference {diff:.2}"
    );

    // And the way back returns the real renderer, bit for bit the same
    // question it was asked before.
    gpu.flat_off();
    let back = gpu.render(&uploaded, &camera);
    let diff = mean_diff(&real, &back);
    assert!(diff < 0.5, "flat off must restore the render: {diff:.2}");
}

#[test]
fn a_lamp_switches_in_the_flat_world() {
    let Some(mut gpu) = gpu() else { return };
    let scene = floor_with_lamp();
    let camera = top_down();
    let uploaded = gpu.upload(&scene);
    gpu.upload_lights(&scene);
    gpu.set_bloom(0.0);

    // References from the real renderer: lamp on, lamp off.
    let real_on = gpu.render(&uploaded, &camera);
    gpu.lights.set_state(0, 0.0, 1.0);
    let real_off = gpu.render(&uploaded, &camera);
    assert!(
        mean_diff(&real_on, &real_off) > 0.5,
        "the lamp must matter to the reference"
    );

    // The flat world, asked the same two questions. The bake runs with the
    // lamp off, which must not matter: sprites are photographed at full
    // power regardless of where the switches stand.
    gpu.flat_on(&uploaded, &camera);
    let flat_off = gpu.render(&uploaded, &camera);
    let diff_off = mean_diff(&real_off, &flat_off);
    assert!(
        diff_off < 3.0,
        "flat with the lamp off must match the dark render: {diff_off:.2}"
    );

    gpu.lights.set_state(0, 1.0, 1.0);
    let flat_on = gpu.render(&uploaded, &camera);
    let diff_on = mean_diff(&real_on, &flat_on);
    assert!(
        diff_on < 3.0,
        "flat with the lamp on must match the lit render: {diff_on:.2}"
    );

    // Leave the world as found for the other tests.
    gpu.flat_off();
}
