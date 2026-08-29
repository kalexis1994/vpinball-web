//! The machine's display, rendered for real.
//!
//! A plasma panel makes its own photons. Drawn through the light loop it shows
//! at whatever the room happens to be — and on a table authored dark, that is
//! nothing: F-14 asks for a global emission of 0.08, and its score was a ghost
//! on the head. So the display's material carries an emissive flag, and the
//! proof is the hardest case: a room with **no light at all** — scene lights
//! black, ambient black, no environment — in which the segments must glow
//! anyway, because they are the light.
//!
//! One shared software adapter behind a mutex, for the reason `dynamic.rs`
//! gives: several at once take the process down.

use std::sync::{Mutex, MutexGuard, OnceLock};
use vpw_math::{Mat4, Vec3};
use vpw_render::Camera;
use vpw_render::offscreen::Offscreen;
use vpw_table::backbox::{Backbox, DISPLAY_IMAGE, DISPLAY_PIXELS};
use vpw_table::geometry::{
    Bounds, Image, Lighting, Material, Mesh, MeshKind, Scene, TablePhysics, Vertex,
};

const W: u32 = 192;
const H: u32 = 192;
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

/// The writable texture the display draws into, black to begin with, the way
/// `geometry::extract` builds it.
fn display_image() -> Image {
    Image {
        name: DISPLAY_IMAGE.into(),
        encoded: None,
        rgba: Some(vec![0; (DISPLAY_PIXELS.0 * DISPLAY_PIXELS.1 * 4) as usize]),
        width: DISPLAY_PIXELS.0,
        height: DISPLAY_PIXELS.1,
        has_alpha: false,
        alpha_test: -1.0,
        redrawn: true,
    }
}

/// A dark floor with the machine's head standing behind it, and not one
/// light anywhere.
fn scene() -> Scene {
    let v = |x: f32, y: f32| Vertex {
        pos: [x, y, 0.0],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 0.0],
    };
    let floor = Mesh {
        name: "floor".into(),
        vertices: vec![v(0.0, 0.0), v(TABLE, 0.0), v(TABLE, TABLE), v(0.0, TABLE)],
        indices: vec![0, 1, 2, 0, 2, 3],
        transform: Mat4::IDENTITY,
        image: String::new(),
        material: "floor".into(),
        visible: true,
        kind: MeshKind::Playfield,
    };
    let playfield = Bounds {
        min: Vec3::new(0.0, 0.0, 0.0),
        max: Vec3::new(TABLE, TABLE, 0.0),
    };
    let head = Backbox::for_playfield(playfield);

    Scene {
        view: vpw_table::geometry::AuthoredView::default(),
        cabinet: vpw_table::geometry::AuthoredView::default(),
        built_head: true,
        meshes: vec![floor, head.mesh(), head.display_mesh()],
        // (the head is rebuilt in `camera` from the same bounds)
        physics: TablePhysics {
            slope_deg: 6.0,
            gravity: 0.0,
            default_scatter_deg: 0.0,
            difficulty: 0.0,
        },
        materials: vec![Material {
            name: "floor".into(),
            base_color: [0.05, 0.05, 0.05],
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
        images: vec![display_image()],
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
        flashers: Vec::new(),
    }
}

/// Frames the whole machine, head included, from the front.
fn camera(scene: &Scene, head: &Backbox) -> Camera {
    let mut min = scene.playfield.min;
    let mut max = scene.playfield.max;
    for c in head.corners() {
        min = min.min(c);
        max = max.max(c);
    }
    let mut c = Camera::framing(min, max, W as f32 / H as f32);
    c.inclination = 30.0;
    c
}

fn brightest(pixels: &[u8]) -> i32 {
    pixels
        .chunks(4)
        .map(|p| i32::from(p[0]) + i32::from(p[1]) + i32::from(p[2]))
        .max()
        .unwrap_or(0)
}

#[test]
fn the_display_glows_in_a_room_with_no_light_at_all() {
    let Some(mut gpu) = gpu() else { return };
    let scene = scene();
    let head = Backbox::for_playfield(scene.playfield);
    let uploaded = gpu.upload(&scene);
    gpu.set_bloom(0.0);
    let camera = camera(&scene, &head);

    // Nothing has been written to the display yet: the whole picture is the
    // dark it should be. The bound is loose because the surrounds are grey,
    // not black; what matters is the distance to the lit case below.
    let dark = brightest(&gpu.render(&uploaded, &camera));

    // The machine says something. The raster is what `segments::draw`
    // produces in spirit: lit pixels on black.
    let mut rgba = vec![0u8; (DISPLAY_PIXELS.0 * DISPLAY_PIXELS.1 * 4) as usize];
    for px in rgba.chunks_mut(4) {
        px.copy_from_slice(&[255, 150, 30, 255]);
    }
    gpu.set_display(
        &uploaded,
        &vpw_render::segments::Raster {
            width: DISPLAY_PIXELS.0,
            height: DISPLAY_PIXELS.1,
            rgba,
        },
    );
    let lit = brightest(&gpu.render(&uploaded, &camera));

    // A panel that is its own light: bright with every lamp in the world off.
    // Through Reinhard at the emissive boost, full amber lands well past 400
    // summed; a display drawn through the light loop in this room would not
    // clear the floor's own grey.
    assert!(
        lit > dark + 200,
        "the display does not glow: dark {dark}, lit {lit}"
    );
}
