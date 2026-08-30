//! The environment map is the table's, not the renderer's.
//!
//! `Renderer.cpp:208-210` loads the image the table names in `EIMG` and only
//! falls back to the shipped `EnvMap.webp` when there is none. On a table lit
//! by nothing else — F-14 leaves ambient and both scene lights black — that map
//! is the whole exposure, and a renderer that always uses the shipped one draws
//! every such table at a brightness the author never saw.
//!
//! Like the other renderer tests these need a GPU adapter, share one behind a
//! mutex, and skip themselves when there is none.

use std::sync::{Mutex, MutexGuard, OnceLock};
use vpw_math::{Mat4, Vec3};
use vpw_render::Camera;
use vpw_render::env::{DEFAULT_SOURCE, EnvMap};
use vpw_render::offscreen::Offscreen;
use vpw_table::geometry::{Bounds, Image, Lighting, Material, Mesh, MeshKind, Scene, Vertex};

const W: u32 = 96;
const H: u32 = 96;

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

/// A small equirectangular map of one colour, the way a `.vpx` BMP arrives:
/// already RGBA.
fn uniform_map(name: &str, value: u8) -> Image {
    let (w, h) = (16u32, 8u32);
    Image {
        name: name.into(),
        encoded: None,
        rgba: Some([value, value, value, 255].repeat((w * h) as usize)),
        width: w,
        height: h,
        has_alpha: false,
        alpha_test: -1.0,
        redrawn: false,
    }
}

/// A grey floor under nothing but the environment, which is how F-14 is lit.
fn floor_under(env_image: &str, images: Vec<Image>) -> Scene {
    let half = 500.0;
    let v = |x: f32, y: f32| Vertex {
        pos: [x, y, 0.0],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 0.0],
    };
    Scene {
        view: vpw_table::geometry::AuthoredView::default(),
        cabinet: vpw_table::geometry::AuthoredView::default(),
        built_head: true,
        meshes: vec![Mesh {
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
            disable_lighting: 0.0,
        }],
        materials: vec![Material {
            name: "floor".into(),
            base_color: [0.8, 0.8, 0.8],
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
        images,
        playfield: Bounds {
            min: Vec3::new(-half, -half, 0.0),
            max: Vec3::new(half, half, 0.0),
        },
        playfield_image: String::new(),
        playfield_material: "floor".into(),
        env_image: env_image.into(),
        ball_decal: String::new(),
        // Ambient and the two scene lights black, as on F-14: whatever the
        // floor shows came from the environment.
        lighting: Lighting {
            lights: [Vec3::new(0.0, 0.0, 800.0), Vec3::new(0.0, 0.0, 800.0)],
            emission: [0.0; 3],
            ambient: [0.0; 3],
            range: 3000.0,
            env_scale: 1.0,
            global: 1.0,
            exposure: 1.0,
            bloom_strength: 0.0,
            reflection_strength: 0.0,
        },
        lights: Vec::new(),
        flashers: Vec::new(),
        physics: vpw_table::geometry::TablePhysics {
            slope_deg: 6.0,
            gravity: 0.0,
            default_scatter_deg: 0.0,
            difficulty: 0.0,
        },
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

/// Mean brightness of the middle of the picture, which is floor whichever
/// way the camera framed it.
fn centre_brightness(gpu: &mut Offscreen, scene: &Scene) -> f64 {
    let uploaded = gpu.upload(scene);
    gpu.upload_lights(scene);
    let pixels = gpu.render(&uploaded, &top_down());
    let mut sum = 0.0;
    let mut n = 0.0;
    for y in H / 3..2 * H / 3 {
        for x in W / 3..2 * W / 3 {
            let i = ((y * W + x) * 4) as usize;
            sum += (u32::from(pixels[i]) + u32::from(pixels[i + 1]) + u32::from(pixels[i + 2]))
                as f64
                / 3.0;
            n += 1.0;
        }
    }
    sum / n
}

#[test]
fn a_table_that_names_an_environment_image_gets_it_and_one_that_does_not_gets_the_default() {
    let Some(mut held) = gpu() else { return };
    let gpu = &mut *held;
    // Named in a different case from how it is stored, as F-14 does with its
    // playfield: `GetImage` does not care (`pintable.cpp:4232`).
    let named = floor_under("SKY", vec![uniform_map("Sky", 200)]);
    let map = EnvMap::for_table(&gpu.device, &gpu.queue, &named);
    assert_eq!(map.source, "Sky");

    let unnamed = floor_under("", vec![uniform_map("Sky", 200)]);
    let map = EnvMap::for_table(&gpu.device, &gpu.queue, &unnamed);
    assert_eq!(map.source, DEFAULT_SOURCE);
}

#[test]
fn a_name_the_table_does_not_carry_or_cannot_decode_falls_back_to_the_default() {
    let Some(mut held) = gpu() else { return };
    let gpu = &mut *held;
    let missing = floor_under("Sky", Vec::new());
    let map = EnvMap::for_table(&gpu.device, &gpu.queue, &missing);
    assert_eq!(map.source, DEFAULT_SOURCE);

    let broken = Image {
        encoded: Some(b"not an image".to_vec()),
        rgba: None,
        ..uniform_map("Sky", 0)
    };
    let undecodable = floor_under("Sky", vec![broken]);
    let map = EnvMap::for_table(&gpu.device, &gpu.queue, &undecodable);
    assert_eq!(map.source, DEFAULT_SOURCE);
}

#[test]
fn loading_a_table_lights_it_by_its_own_map() {
    // The map that matters is the one the frame is drawn with, not the one
    // that got loaded: a bind group still holding the old views would pass
    // the test above and fail this one. Same floor, two tables, two maps.
    let Some(mut held) = gpu() else { return };
    let gpu = &mut *held;
    let bright = centre_brightness(gpu, &floor_under("Env", vec![uniform_map("Env", 255)]));
    let dim = centre_brightness(gpu, &floor_under("Env", vec![uniform_map("Env", 40)]));
    assert!(
        bright > 2.0 * dim && dim > 1.0,
        "under a white map the floor is {bright:.1}, under a dark one {dim:.1}"
    );
}
