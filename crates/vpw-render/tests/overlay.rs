//! The score on a table's own backdrop.
//!
//! A table that brings a backdrop is drawn the original's way: the picture
//! over the whole window, no head, and the score in the windows the picture
//! has for it. The proof is a grey picture with one window on it: the window
//! comes out the colour of what was put in it, and the rest stays the
//! picture's grey, with the head nowhere.
//!
//! One shared software adapter behind a mutex, for the reason `dynamic.rs`
//! gives: several at once take the process down.

use std::sync::{Mutex, MutexGuard, OnceLock};
use vpw_math::{Mat4, Vec3};
use vpw_render::Camera;
use vpw_render::offscreen::Offscreen;
use vpw_render::segments::Raster;
use vpw_table::backdrop::ScoreWindows;
use vpw_table::geometry::{
    Bounds, Image, Lighting, Material, Mesh, MeshKind, Scene, TablePhysics, Vertex,
};

const W: u32 = 160;
const H: u32 = 120;
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

/// A picture of one flat grey, the size of nothing in particular.
fn grey_picture() -> Image {
    let (w, h) = (8u32, 8u32);
    Image {
        name: "bg".into(),
        encoded: None,
        rgba: Some([128u8, 128, 128, 255].repeat((w * h) as usize)),
        width: w,
        height: h,
        has_alpha: false,
        alpha_test: -1.0,
        redrawn: false,
    }
}

/// A small dark floor far below the camera's line of sight, with the backdrop
/// behind it and one score window on the backdrop. No head.
fn scene(window: [f32; 4]) -> Scene {
    let playfield = Bounds {
        min: Vec3::new(0.0, 0.0, 0.0),
        max: Vec3::new(TABLE, TABLE * 2.0, 0.0),
    };
    let quad = |x0: f32, y0: f32, x1: f32, y1: f32| -> Vec<Vertex> {
        [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
            .into_iter()
            .map(|(x, y)| Vertex {
                pos: [x, y, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
            })
            .collect()
    };
    let floor = Mesh {
        name: "floor".into(),
        vertices: quad(0.0, 0.0, TABLE, TABLE * 2.0),
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
    Scene {
        view: vpw_table::geometry::AuthoredView::default(),
        cabinet: vpw_table::geometry::AuthoredView::default(),
        built_head: false,
        meshes: vec![floor],
        physics: TablePhysics {
            slope_deg: 6.0,
            gravity: 0.0,
            default_scatter_deg: 0.0,
            difficulty: 0.0,
        },
        materials: vec![Material {
            name: "floor".into(),
            base_color: [0.02, 0.02, 0.02],
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
        images: vec![grey_picture()],
        playfield,
        playfield_image: String::new(),
        playfield_material: "floor".into(),
        lighting: Lighting {
            lights: [Vec3::new(500.0, 500.0, 800.0); 2],
            emission: [0.0; 3],
            ambient: [0.0; 3],
            range: 3000.0,
            env_scale: 0.0,
            global: 1.0,
            exposure: 1.0,
            bloom_strength: 0.0,
            reflection_strength: 0.0,
        },
        lights: Vec::new(),
        env_image: String::new(),
        ball_decal: String::new(),
        backdrop_image: "bg".into(),
        backdrop_color: [0.0; 3],
        score_windows: ScoreWindows {
            rects: vec![window],
            pick_one: false,
            over: false,
        },
        head_windows: Vec::new(),
        flashers: Vec::new(),
    }
}

/// Looking down at the floor from high above, so the floor fills the bottom
/// of the picture and the top of the picture is backdrop alone.
fn camera() -> Camera {
    let mut c = Camera::framing(
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(TABLE, TABLE * 2.0, 0.0),
        W as f32 / H as f32,
    );
    c.inclination = 80.0;
    c.distance *= 3.0;
    c
}

fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * W + x) * 4) as usize;
    [frame[i], frame[i + 1], frame[i + 2]]
}

#[test]
fn the_window_shows_what_was_put_in_it_and_the_picture_shows_round_it() {
    let Some(mut gpu) = gpu() else { return };
    // A window in the top left corner of the screen, over backdrop alone.
    let window = [0.05, 0.05, 0.4, 0.2];
    let scene = scene(window);
    let gpu_scene = gpu.upload(&scene);
    assert!(gpu.overlay.is_some(), "a backdrop brings its windows");

    // Solid red in the window.
    gpu.set_score_window(
        0,
        &Raster {
            width: 4,
            height: 4,
            rgba: [255u8, 0, 0, 255].repeat(16),
        },
    );
    let frame = gpu.render(&gpu_scene, &camera());

    let inside = pixel(
        &frame,
        ((window[0] + window[2] * 0.5) * W as f32) as u32,
        ((window[1] + window[3] * 0.5) * H as f32) as u32,
    );
    assert!(
        inside[0] > 150 && inside[1] < 60 && inside[2] < 60,
        "the window should be red, was {inside:?}"
    );
    // Just outside it, the picture's own grey.
    let beside = pixel(&frame, ((window[0] + window[2] + 0.05) * W as f32) as u32, 4);
    assert!(
        beside.iter().all(|&c| c > 60 && c < 200) && (beside[0] as i32 - beside[2] as i32).abs() < 20,
        "beside the window is the backdrop, was {beside:?}"
    );
}

#[test]
fn a_transparent_window_leaves_the_picture_alone() {
    let Some(mut gpu) = gpu() else { return };
    let window = [0.05, 0.05, 0.4, 0.2];
    let scene = scene(window);
    let gpu_scene = gpu.upload(&scene);
    // Nothing put in the window: it starts as one transparent pixel.
    let frame = gpu.render(&gpu_scene, &camera());
    let inside = pixel(&frame, (0.25 * W as f32) as u32, (0.15 * H as f32) as u32);
    assert!(
        (inside[0] as i32 - inside[2] as i32).abs() < 20 && inside[0] > 60,
        "an empty window is the picture through it, was {inside:?}"
    );
}
