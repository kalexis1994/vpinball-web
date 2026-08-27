//! The lit insert, rendered for real.
//!
//! A classic light with a picture is drawn as that picture, lit, with the halo
//! folded into it (`ClassicLightShader.hlsl:52-87`) — and the only way to know
//! the picture is being sampled at all is to look. The picture here is half
//! red and half blue; a halo has no halves.
//!
//! One shared software adapter behind a mutex, for the reason `dynamic.rs`
//! gives: several at once take the process down.

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
/// The table is a 1000 by 1000 square starting at the origin, the way a real
/// table's does — the insert's coordinates are `x / width` with no offset
/// (`light.cpp:519`), so a table centred on the origin would sample the wrong
/// half of the picture.
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

/// A picture of the whole playfield: red on the left half, blue on the right.
fn picture() -> Image {
    let mut rgba = Vec::with_capacity(64 * 4);
    for x in 0..64 {
        rgba.extend_from_slice(if x < 32 {
            &[255, 0, 0, 255]
        } else {
            &[0, 0, 255, 255]
        });
    }
    Image {
        name: "art".into(),
        encoded: None,
        rgba: Some(rgba),
        width: 64,
        height: 1,
        has_alpha: false,
        alpha_test: -1.0,
        redrawn: false,
    }
}

/// A dark floor with one square insert in the middle of it. `image` names the
/// insert's picture, `surface_image` what the floor is said to show, and the
/// scene lights are all off so that whatever colour comes out is the insert's.
fn scene(image: &str, image_mode: bool, surface_image: &str, state: f32) -> Scene {
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

    let (c, half) = (TABLE / 2.0, 60.0);
    let corners = [
        [c - half, c - half],
        [c + half, c - half],
        [c + half, c + half],
        [c - half, c + half],
    ];
    let light = Light {
        name: "Insert".into(),
        vertices: corners.iter().map(|p| [p[0], p[1], 1.0]).collect(),
        indices: vec![0, 1, 2, 0, 2, 3],
        // Table space: `light.cpp:519-520`.
        uvs: corners
            .iter()
            .map(|p| [p[0] / TABLE, p[1] / TABLE])
            .collect(),
        image: image.into(),
        image_mode,
        surface_material: "floor".into(),
        surface_image: surface_image.into(),
        center: Vec3::new(c, c, 1.0),
        falloff_radius: half * 1.5,
        falloff_power: 2.0,
        intensity: 1.0,
        color: [1.0, 1.0, 1.0],
        color2: [1.0, 1.0, 1.0],
        state,
        blinking: false,
        is_bulb: false,
        transmission_scale: 0.0,
        modulate: 0.0,
        fader: Fader::None,
        fade_up: 0.2,
        fade_down: 0.2,
        blink: vec![true],
        blink_interval: 125.0,
    };

    Scene {
        meshes: vec![floor],
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
        images: vec![picture()],
        playfield: Bounds {
            min: Vec3::new(0.0, 0.0, 0.0),
            max: Vec3::new(TABLE, TABLE, 0.0),
        },
        playfield_image: String::new(),
        playfield_material: "floor".into(),
        lighting: Lighting {
            lights: [Vec3::new(c, c, 800.0), Vec3::new(c, c, 800.0)],
            emission: [0.0, 0.0, 0.0],
            ambient: [0.0, 0.0, 0.0],
            range: 3000.0,
            env_scale: 0.0,
            global: 1.0,
            exposure: 1.0,
            bloom_strength: 0.0,
            reflection_strength: 0.0,
        },
        lights: vec![light],
        env_image: String::new(),
        ball_decal: String::new(),
        flashers: Vec::new(),
    }
}

fn shoot(gpu: &mut Offscreen, scene: &Scene) -> Vec<u8> {
    let mut camera = Camera::framing(
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(TABLE, TABLE, 0.0),
        W as f32 / H as f32,
    );
    camera.inclination = 89.0;
    // The fit may have slid the target to centre the picture *at the fit's
    // own inclination*; overriding the angle afterwards keeps the slide but
    // not the composition it served. Straight above, the centre of the rig
    // is the composition.
    camera.target = Vec3::new(TABLE * 0.5, TABLE * 0.5, 0.0);
    let uploaded = gpu.upload(scene);
    gpu.upload_lights(scene);
    gpu.set_bloom(0.0);
    gpu.render(&uploaded, &camera)
}

fn px(pixels: &[u8], x: u32, y: u32) -> [i32; 3] {
    let i = ((y * W + x) * 4) as usize;
    [
        i32::from(pixels[i]),
        i32::from(pixels[i + 1]),
        i32::from(pixels[i + 2]),
    ]
}

/// Red minus blue: positive on the red half of the picture, negative on the
/// blue half, about zero on a white halo.
fn tint(p: [i32; 3]) -> i32 {
    p[0] - p[2]
}

/// The two halves of the insert, measured across it both ways — which way the
/// camera lays the table out on the screen is the camera's business.
fn halves(pixels: &[u8]) -> ([i32; 3], [i32; 3], [i32; 3], [i32; 3]) {
    let (cx, cy) = (W / 2, H / 2);
    (
        px(pixels, cx - 4, cy),
        px(pixels, cx + 4, cy),
        px(pixels, cx, cy - 4),
        px(pixels, cx, cy + 4),
    )
}

fn split(pixels: &[u8]) -> i32 {
    let (l, r, t, b) = halves(pixels);
    (tint(l) - tint(r)).abs().max((tint(t) - tint(b)).abs())
}

#[test]
fn an_insert_with_a_picture_shows_the_picture_and_not_a_disc() {
    let Some(mut gpu) = gpu() else { return };

    // The halo alone: white, and the same on both sides of the centre.
    let halo = shoot(&mut gpu, &scene("", false, "", 1.0));
    assert_eq!(gpu.lights.textured(), 0);
    let (l, r, t, b) = halves(&halo);
    assert!(l.iter().sum::<i32>() > 0, "the halo is drawn at all: {l:?}");
    assert!(
        split(&halo) < 30,
        "a halo has no halves: {l:?} {r:?} {t:?} {b:?}"
    );

    // With the picture, in both modes: the halo's colour is folded into the
    // picture's, and the picture has a red half and a blue half.
    for image_mode in [true, false] {
        let lit = shoot(&mut gpu, &scene("art", image_mode, "", 1.0));
        assert_eq!(gpu.lights.textured(), 1);
        let (l, r, t, b) = halves(&lit);
        assert!(
            split(&lit) > 100,
            "image mode {image_mode}: the insert should be red on one side and blue on the other, got {l:?} {r:?} {t:?} {b:?}"
        );
    }
}

#[test]
fn a_picture_the_table_does_not_have_is_the_halo_again() {
    // `light.cpp:708` resolves the name through `GetImage`, and a null texel
    // takes `light_without_texture` (`:823`). A picture that failed to load
    // must not black the insert out.
    let Some(mut gpu) = gpu() else { return };
    let halo = shoot(&mut gpu, &scene("", false, "", 1.0));
    let missing = shoot(&mut gpu, &scene("no such picture", false, "", 1.0));
    assert_eq!(gpu.lights.textured(), 0);
    assert_eq!(halves(&halo), halves(&missing));
}

#[test]
fn an_unlit_insert_with_a_picture_of_its_own_is_still_drawn() {
    // `light.cpp:713-718`: the early-out at zero intensity is for a picture
    // that is the surface's own. One of its own is drawn dark — here in image
    // mode, so "dark" is the picture as it is, and it can be seen.
    let Some(mut gpu) = gpu() else { return };
    let own = shoot(&mut gpu, &scene("art", true, "", 0.0));
    assert!(split(&own) > 100, "the picture is there with the lamp off");

    // The same picture, but it is what the floor already shows: nothing to
    // add, and the original leaves before drawing.
    let same = shoot(&mut gpu, &scene("art", false, "ART", 0.0));
    let (l, ..) = halves(&same);
    assert_eq!(l, [0, 0, 0], "not drawn: {l:?}");
}
