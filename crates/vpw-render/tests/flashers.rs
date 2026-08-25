//! The flashers, rendered for real.
//!
//! The same discipline as `dynamic.rs`: a tiny scene drawn offscreen, and
//! every question asked as a **difference** between two renders — with the
//! flasher and without, on and off — never as absolute brightness. A shader
//! that fails to compile, a blend with a factor the wrong way round or a
//! matrix that puts the polygon under the floor all build fine and all draw
//! nothing, and only the pixels can tell.
//!
//! One shared software adapter behind a mutex, for the reason given in
//! `dynamic.rs`: several at once take the process down.

use std::sync::{Mutex, MutexGuard, OnceLock};
use vpin::vpx::gameitem::dragpoint::DragPoint;
use vpin::vpx::gameitem::flasher::{Flasher as VpxFlasher, RenderMode};
use vpin::vpx::gameitem::ramp_image_alignment::RampImageAlignment;
use vpw_math::{Mat4, Vec3};
use vpw_render::Camera;
use vpw_render::offscreen::Offscreen;
use vpw_table::geometry::{Bounds, Lighting, Mesh, MeshKind, Scene, Vertex};

const W: u32 = 128;
const H: u32 = 128;

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

/// A table that is nothing but a floor.
fn floor_scene() -> Scene {
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
        material: String::new(),
        visible: true,
        kind: MeshKind::Playfield,
    };
    Scene {
        meshes: vec![floor],
        materials: Vec::new(),
        images: Vec::new(),
        playfield: Bounds {
            min: Vec3::new(-half, -half, 0.0),
            max: Vec3::new(half, half, 0.0),
        },
        playfield_image: String::new(),
        playfield_material: String::new(),
        lighting: Lighting {
            lights: [Vec3::new(0.0, 0.0, 800.0), Vec3::new(0.0, 0.0, 800.0)],
            emission: [1.0, 1.0, 1.0],
            ambient: [0.1, 0.1, 0.1],
            range: 3000.0,
            env_scale: 1.0,
            exposure: 1.0,
            bloom_strength: 0.0,
            reflection_strength: 0.0,
        },
        lights: Vec::new(),
        flashers: Vec::new(),
        physics: vpw_table::geometry::TablePhysics {
            slope_deg: 6.0,
            gravity: 1.76,
            default_scatter_deg: 0.0,
            difficulty: 0.0,
        },
    }
}

/// A rectangular flasher from (x0, y0) to (x1, y1), fifty units up, with no
/// picture: the flat colour path.
fn rectangle(x0: f32, y0: f32, x1: f32, y1: f32) -> VpxFlasher {
    let corner = |x: f32, y: f32| DragPoint {
        x,
        y,
        z: 0.0,
        smooth: false,
        ..DragPoint::default()
    };
    VpxFlasher {
        name: "F".into(),
        height: 50.0,
        alpha: 100,
        modulate_vs_add: 0.9,
        is_visible: true,
        filter_amount: 100,
        color: vpin::vpx::color::Color::rgb(255, 40, 40),
        image_alignment: RampImageAlignment::Wrap,
        drag_points: vec![
            corner(x0, y0),
            corner(x0, y1),
            corner(x1, y1),
            corner(x1, y0),
        ],
        ..VpxFlasher::default()
    }
}

fn with_flasher(f: &VpxFlasher) -> Scene {
    let mut scene = floor_scene();
    scene
        .flashers
        .extend(vpw_table::flasher::build(f, scene.playfield));
    assert_eq!(scene.flashers.len(), 1, "the flasher should build");
    scene
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

fn changed(before: &[u8], after: &[u8]) -> Vec<(u32, u32)> {
    before
        .as_chunks::<4>()
        .0
        .iter()
        .zip(after.as_chunks::<4>().0)
        .enumerate()
        .filter(|(_, (a, b))| (0..3).any(|c| a[c].abs_diff(b[c]) > 8))
        .map(|(i, _)| (i as u32 % W, i as u32 / W))
        .collect()
}

fn centre_of_change(before: &[u8], after: &[u8]) -> Option<(f32, f32)> {
    let px = changed(before, after);
    if px.is_empty() {
        return None;
    }
    let n = px.len() as f32;
    Some((
        px.iter().map(|p| p.0 as f32).sum::<f32>() / n,
        px.iter().map(|p| p.1 as f32).sum::<f32>() / n,
    ))
}

/// The floor alone, and the floor with the flasher, from the same device.
fn plain_and_lit(gpu: &mut Offscreen, f: &VpxFlasher) -> (Vec<u8>, Vec<u8>) {
    let scene = with_flasher(f);
    let uploaded = gpu.upload(&scene);
    let camera = top_down();
    gpu.upload_flashers(&floor_scene());
    let plain = gpu.render(&uploaded, &camera);
    gpu.upload_flashers(&scene);
    let lit = gpu.render(&uploaded, &camera);
    (plain, lit)
}

#[test]
fn the_shaders_compile() {
    // `Offscreen::new` builds all four flasher pipelines from `flasher.wgsl`;
    // malformed WGSL shows up here and nowhere else.
    let Some(gpu) = gpu() else { return };
    assert!(gpu.adapter.len() > 1);
}

#[test]
fn a_painted_flasher_lands_where_it_was_put() {
    let Some(mut held) = gpu() else { return };
    let gpu = &mut *held;
    // Off-centre, so the position is checked and not just the presence.
    let (plain, lit) = plain_and_lit(gpu, &rectangle(-350.0, -350.0, -150.0, -150.0));
    let px = changed(&plain, &lit);
    assert!(
        px.len() > 100,
        "the flasher barely changed the image: {} pixels",
        px.len()
    );
    let (cx, cy) = centre_of_change(&plain, &lit).unwrap();
    assert!(
        cx < W as f32 * 0.4 && cy < H as f32 * 0.4,
        "it came out at ({cx:.0}, {cy:.0}), not in the top-left quarter"
    );
    // And it is red, which is the colour it was given: the flat-colour path
    // wires `staticColor_Alpha` straight through.
    let (x, y) = (cx as u32, cy as u32);
    let p = &lit[((y * W + x) * 4) as usize..][..3];
    assert!(p[0] > p[1] + 30 && p[0] > p[2] + 30, "not red: {p:?}");
}

#[test]
fn an_additive_flasher_brightens_and_never_darkens() {
    // The reverse-subtract blend hands the hardware a negative colour and an
    // alpha of `1/m - 1`; get either sign wrong and a strobe is a dark patch.
    let Some(mut held) = gpu() else { return };
    let gpu = &mut *held;
    let mut f = rectangle(-100.0, -100.0, 100.0, 100.0);
    f.add_blend = true;
    let (plain, lit) = plain_and_lit(gpu, &f);
    let px = changed(&plain, &lit);
    assert!(px.len() > 100, "{} pixels changed", px.len());
    for (x, y) in px {
        let i = ((y * W + x) * 4) as usize;
        assert!(
            lit[i] >= plain[i],
            "the red channel went down at ({x}, {y}): {} to {}",
            plain[i],
            lit[i]
        );
    }
}

#[test]
fn a_flasher_switched_off_by_the_script_draws_nothing() {
    let Some(mut held) = gpu() else { return };
    let gpu = &mut *held;
    let f = rectangle(-100.0, -100.0, 100.0, 100.0);
    let scene = with_flasher(&f);
    let uploaded = gpu.upload(&scene);
    let camera = top_down();
    gpu.upload_flashers(&scene);
    let on = gpu.render(&uploaded, &camera);

    // What `core.vbs` does off a solenoid: `Visible = False`.
    let mut state = scene.flashers[0].state.clone();
    state.visible = false;
    gpu.flashers
        .set_state(&gpu.device, &gpu.queue, 0, &state, 1.0);
    let off = gpu.render(&uploaded, &camera);
    assert!(
        changed(&on, &off).len() > 100,
        "hiding it should change the image"
    );

    // Back on, but scaled to nothing by `IntensityScale`: the same.
    state.visible = true;
    state.intensity_scale = 0.0;
    gpu.flashers
        .set_state(&gpu.device, &gpu.queue, 0, &state, 1.0);
    let dimmed = gpu.render(&uploaded, &camera);
    assert!(
        changed(&off, &dimmed).is_empty(),
        "a flasher at zero scale is off"
    );

    // And a lightmap flasher whose lamp is off is off too
    // (`flasher.cpp:1171-1177`).
    state.intensity_scale = 1.0;
    gpu.flashers
        .set_state(&gpu.device, &gpu.queue, 0, &state, 0.0);
    let unlit = gpu.render(&uploaded, &camera);
    assert!(changed(&off, &unlit).is_empty(), "its lamp is off");
}

#[test]
fn the_script_can_move_a_flasher() {
    let Some(mut held) = gpu() else { return };
    let gpu = &mut *held;
    let f = rectangle(-100.0, -100.0, 100.0, 100.0);
    let scene = with_flasher(&f);
    let uploaded = gpu.upload(&scene);
    let camera = top_down();
    gpu.upload_flashers(&floor_scene());
    let plain = gpu.render(&uploaded, &camera);
    gpu.upload_flashers(&scene);

    let mut state = scene.flashers[0].state.clone();
    state.x -= 250.0;
    gpu.flashers
        .set_state(&gpu.device, &gpu.queue, 0, &state, 1.0);
    let moved = gpu.render(&uploaded, &camera);
    let (cx, _) = centre_of_change(&plain, &moved).expect("it is drawn");
    assert!(cx < W as f32 * 0.4, "it should have moved left: {cx:.0}");
}

#[test]
fn a_display_flasher_shows_the_dots_it_was_given() {
    let Some(mut held) = gpu() else { return };
    let gpu = &mut *held;
    let mut f = rectangle(-300.0, -75.0, 300.0, 75.0);
    f.render_mode = Some(RenderMode::DMD);
    f.color = vpin::vpx::color::Color::rgb(255, 160, 40);
    // Opaque: `modulate_vs_add` is the display's opacity (`flasher.cpp:1338`).
    f.modulate_vs_add = 1.0;
    let scene = with_flasher(&f);
    let uploaded = gpu.upload(&scene);
    let camera = top_down();
    gpu.upload_flashers(&scene);
    // No bloom: the question is where the dots are, and a lit panel with the
    // bloom on spreads a halo into the half that is supposed to stay dark.
    gpu.set_bloom(0.0);

    // No frame yet: dark dots, which draw as a dark panel. Then a frame with
    // the left half lit.
    let dark = gpu.render(&uploaded, &camera);
    let (w, h) = (128usize, 32usize);
    let mut dots = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w / 2 {
            dots[y * w + x] = 3;
        }
    }
    gpu.set_dmd(&dots, w, h);
    let lit = gpu.render(&uploaded, &camera);
    let px = changed(&dark, &lit);
    assert!(
        px.len() > 100,
        "the dots barely changed the image: {} pixels",
        px.len()
    );
    let (cx, cy) = centre_of_change(&dark, &lit).unwrap();
    assert!(
        cx < W as f32 * 0.45,
        "the lit half should be on the left: centre at {cx:.0}"
    );
    assert!(
        (cy - H as f32 / 2.0).abs() < 12.0,
        "and across the middle: {cy:.0}"
    );
    // Nothing on the right half moved: an unlit dot is dark either way.
    let rightmost = px.iter().map(|p| p.0).max().unwrap();
    assert!(
        rightmost < W * 55 / 100,
        "the dark half lit up: a pixel changed at x = {rightmost}"
    );
    // The bloom back where the other tests expect it.
    gpu.set_bloom(vpw_render::Post::DEFAULT_STRENGTH);
    // And it is the amber it was given, brighter than the floor.
    let (x, y) = (cx as u32, cy as u32);
    let p = &lit[((y * W + x) * 4) as usize..][..3];
    assert!(p[0] > p[2] + 40, "not amber: {p:?}");
}
