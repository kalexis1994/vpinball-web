//! The moving pieces, rendered for real.
//!
//! These tests build a tiny scene, draw it offscreen and look at the pixels.
//! That is the point: a shader that does not compile, a bind group with the
//! wrong layout or a model matrix that is transposed all produce code that
//! builds fine and an image that is wrong, and only an image catches them.
//!
//! Everything is measured as a **difference against the same scene without the
//! piece**, never as absolute brightness. A chrome ball on a pale floor comes
//! out darker than what it covers, so "did the image get brighter" answers the
//! wrong question — the first version of these tests asked it and concluded the
//! ball was not being drawn while it was.
//!
//! They need a GPU adapter. There is none in some CI containers, so they skip
//! themselves rather than fail — a llvmpipe/lavapipe software adapter is enough
//! and is what they normally run on.
//!
//! All of them share **one** adapter, behind a mutex. That is not an
//! optimisation: `cargo test` runs the file's tests on one thread each, and six
//! simultaneous software rasterisers are enough to take the process down with a
//! SIGSEGV on a machine with modest memory. One device, taken in turn.

use std::sync::{Mutex, MutexGuard, OnceLock};
use vpw_math::{Mat4, Quat, Vec3};
use vpw_render::Camera;
use vpw_render::offscreen::Offscreen;
use vpw_table::animation::{AnimatedPart, Animation};
use vpw_table::geometry::{Bounds, Lighting, Mesh, MeshKind, Scene, Vertex};

const W: u32 = 128;
const H: u32 = 128;

/// The one shared device, or `None` if this machine has no adapter.
static GPU: OnceLock<Option<Mutex<Offscreen>>> = OnceLock::new();

fn gpu() -> Option<MutexGuard<'static, Offscreen>> {
    let cell = GPU.get_or_init(|| match pollster::block_on(Offscreen::new(W, H)) {
        Ok(g) => Some(Mutex::new(g)),
        Err(e) => {
            eprintln!("skipped: no GPU adapter ({e})");
            None
        }
    });
    // A failing test poisons the mutex; letting that turn every later test into
    // a confusing poison error would hide the one real failure.
    Some(
        cell.as_ref()?
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

/// A table that is nothing but a floor, so anything drawn on top of it stands
/// out against a flat background.
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
        clamp: false,
        scenery: false,
        kind: MeshKind::Playfield,
        additive: None,
        disable_lighting: 0.0,
    };

    Scene {
        view: vpw_table::geometry::AuthoredView::default(),
        cabinet: vpw_table::geometry::AuthoredView::default(),
        built_head: true,
        meshes: vec![floor],
        materials: Vec::new(),
        images: Vec::new(),
        playfield: Bounds {
            min: Vec3::new(-half, -half, 0.0),
            max: Vec3::new(half, half, 0.0),
        },
        playfield_image: String::new(),
        playfield_material: String::new(),
        env_image: String::new(),
        ball_decal: String::new(),
        lighting: Lighting {
            lights: [Vec3::new(0.0, 0.0, 800.0), Vec3::new(0.0, 0.0, 800.0)],
            emission: [1.0, 1.0, 1.0],
            ambient: [0.1, 0.1, 0.1],
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
            gravity: 1.76,
            default_scatter_deg: 0.0,
            difficulty: 0.0,
        },
    }
}

/// Looks almost straight down at the floor.
fn top_down() -> Camera {
    let mut c = Camera::framing(
        Vec3::new(-500.0, -500.0, 0.0),
        Vec3::new(500.0, 500.0, 0.0),
        W as f32 / H as f32,
    );
    c.inclination = 89.0;
    c
}

/// The pixels where two renders differ, as `(x, y)` pairs.
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

/// Where the difference between two renders sits, on average.
fn centre_of_change(before: &[u8], after: &[u8]) -> Option<(f32, f32)> {
    let px = changed(before, after);
    if px.is_empty() {
        return None;
    }
    let n = px.len() as f32;
    let sx: f32 = px.iter().map(|p| p.0 as f32).sum();
    let sy: f32 = px.iter().map(|p| p.1 as f32).sum();
    Some((sx / n, sy / n))
}

fn ball_at(x: f32, y: f32, radius: f32) -> Mat4 {
    vpw_table::ball::transform(Vec3::new(x, y, radius), radius, Quat::IDENTITY, false)
}

#[test]
fn the_shaders_compile() {
    // `Offscreen::new` builds `TablePipeline`, which compiles both shader
    // modules — the static one and the dynamic one — and both pipeline pairs.
    // If either piece of WGSL is malformed, this is where it shows.
    let Some(gpu) = gpu() else { return };
    assert!(gpu.adapter.len() > 1, "the adapter should identify itself");
}

#[test]
fn a_ball_shows_up_on_the_table() {
    let Some(mut held) = gpu() else { return };
    // Through the guard the borrow checker cannot split fields, so `gpu.queue`
    // and `gpu.dynamic` would fight. One reborrow and they are two fields again.
    let gpu = &mut *held;
    let scene = floor_scene();
    let uploaded = gpu.upload(&scene);
    let camera = top_down();

    gpu.upload_dynamic(&scene, &[]);
    let empty = gpu.render(&uploaded, &camera);

    gpu.dynamic
        .as_mut()
        .unwrap()
        .set_ball_transform(&gpu.queue, 0, Some(ball_at(0.0, 0.0, 150.0)));
    let with_ball = gpu.render(&uploaded, &camera);

    let px = changed(&empty, &with_ball);
    assert!(
        px.len() > 200,
        "the ball barely changed the image: {} pixels",
        px.len()
    );

    // And it landed in the middle, which is where it was put.
    let (cx, cy) = centre_of_change(&empty, &with_ball).unwrap();
    assert!(
        (cx - W as f32 / 2.0).abs() < 12.0 && (cy - H as f32 / 2.0).abs() < 20.0,
        "the ball came out at ({cx:.0}, {cy:.0}) instead of the centre"
    );
}

#[test]
fn a_hidden_ball_draws_nothing() {
    let Some(mut held) = gpu() else { return };
    // Through the guard the borrow checker cannot split fields, so `gpu.queue`
    // and `gpu.dynamic` would fight. One reborrow and they are two fields again.
    let gpu = &mut *held;
    let scene = floor_scene();
    let uploaded = gpu.upload(&scene);
    let camera = top_down();

    gpu.upload_dynamic(&scene, &[]);
    let hidden = gpu.render(&uploaded, &camera);

    gpu.dynamic
        .as_mut()
        .unwrap()
        .set_ball_transform(&gpu.queue, 0, Some(ball_at(0.0, 0.0, 150.0)));
    let shown = gpu.render(&uploaded, &camera);

    gpu.dynamic
        .as_mut()
        .unwrap()
        .set_ball_transform(&gpu.queue, 0, None);
    let hidden_again = gpu.render(&uploaded, &camera);

    assert!(
        !changed(&hidden, &shown).is_empty(),
        "the ball never appeared"
    );
    assert_eq!(
        hidden, hidden_again,
        "hiding a ball has to leave the image exactly as it was"
    );
}

#[test]
fn every_ball_slot_draws_on_its_own() {
    // Multiball: all the slots share one range of indices and differ only in
    // their matrix, so it is worth checking that they really are independent.
    let Some(mut held) = gpu() else { return };
    // Through the guard the borrow checker cannot split fields, so `gpu.queue`
    // and `gpu.dynamic` would fight. One reborrow and they are two fields again.
    let gpu = &mut *held;
    let scene = floor_scene();
    let uploaded = gpu.upload(&scene);
    let camera = top_down();

    gpu.upload_dynamic(&scene, &[]);
    let empty = gpu.render(&uploaded, &camera);

    for slot in 0..vpw_render::MAX_BALLS {
        gpu.dynamic.as_mut().unwrap().set_ball_transform(
            &gpu.queue,
            slot,
            Some(ball_at(0.0, 0.0, 120.0)),
        );
        let one = gpu.render(&uploaded, &camera);
        assert!(
            !changed(&empty, &one).is_empty(),
            "ball slot {slot} does not draw"
        );
        gpu.dynamic
            .as_mut()
            .unwrap()
            .set_ball_transform(&gpu.queue, slot, None);
    }
}

#[test]
fn moving_a_ball_moves_its_pixels() {
    let Some(mut held) = gpu() else { return };
    // Through the guard the borrow checker cannot split fields, so `gpu.queue`
    // and `gpu.dynamic` would fight. One reborrow and they are two fields again.
    let gpu = &mut *held;
    let scene = floor_scene();
    let uploaded = gpu.upload(&scene);
    let camera = top_down();
    gpu.upload_dynamic(&scene, &[]);
    let empty = gpu.render(&uploaded, &camera);

    let mut shot = |x: f32| {
        gpu.dynamic.as_mut().unwrap().set_ball_transform(
            &gpu.queue,
            0,
            Some(ball_at(x, 0.0, 120.0)),
        );
        let img = gpu.render(&uploaded, &camera);
        centre_of_change(&empty, &img).expect("the ball has to show up somewhere")
    };

    let left = shot(-300.0);
    let right = shot(300.0);

    // In Visual Pinball `x` grows to the right of the screen.
    assert!(
        left.0 < right.0 - 20.0,
        "the ball's matrix does not move it along x: {left:?} vs {right:?}"
    );
}

#[test]
fn a_table_piece_is_drawn_from_its_own_matrix() {
    let Some(mut held) = gpu() else { return };
    // Through the guard the borrow checker cannot split fields, so `gpu.queue`
    // and `gpu.dynamic` would fight. One reborrow and they are two fields again.
    let gpu = &mut *held;
    let scene = floor_scene();
    let uploaded = gpu.upload(&scene);
    let camera = top_down();

    // A piece whose animation resolves to the identity, because there is no
    // engine behind it. What is being checked is the `base`/`local` path.
    let base = Mat4::from_translation(Vec3::new(-300.0, 0.0, 150.0));
    let local = Mat4::from_scale(Vec3::splat(150.0));
    let mut piece = vpw_table::ball::mesh();
    piece.name = "Piece".into();
    // `animated_parts` guarantees this invariant: `mesh.transform` is where the
    // piece is right now, and that is what seeds the renderer's matrix.
    piece.transform = base * local;
    let part = AnimatedPart {
        mesh: piece,
        base,
        local,
        anim: Animation::Flipper { shape: usize::MAX },
    };

    gpu.upload_dynamic(&scene, &[]);
    let without = gpu.render(&uploaded, &camera);

    gpu.upload_dynamic(&scene, std::slice::from_ref(&part));
    assert_eq!(gpu.dynamic.as_ref().unwrap().table_parts(), 1);
    let at_rest = gpu.render(&uploaded, &camera);
    let rest_centre = centre_of_change(&without, &at_rest)
        .expect("a piece has to be drawn at its mesh transform");

    // Now move it to the other side through its matrix.
    gpu.dynamic.as_mut().unwrap().set_part_transform(
        &gpu.queue,
        0,
        Mat4::from_translation(Vec3::new(300.0, 0.0, 150.0)) * local,
    );
    let moved = gpu.render(&uploaded, &camera);
    let moved_centre =
        centre_of_change(&without, &moved).expect("the piece disappeared when it was moved");

    assert!(
        rest_centre.0 < moved_centre.0 - 20.0,
        "the piece's matrix does not move it: {rest_centre:?} vs {moved_centre:?}"
    );
}

/// A layer of light: a copy of some geometry that is **added** to what is
/// already drawn, in proportion to how bright its lamp is.
///
/// See `vpw_table::geometry::Additive`. Drawn as ordinary geometry instead,
/// this square would be an opaque black patch on the floor rather than a
/// brightening of it — which is exactly the bug it exists to prevent.
fn light_layer(name: &str, lamp: Option<&str>) -> AnimatedPart {
    let half = 200.0;
    let v = |x: f32, y: f32| Vertex {
        pos: [x, y, 1.0],
        normal: [0.0, 0.0, 1.0],
        uv: [0.5, 0.5],
    };
    AnimatedPart {
        mesh: Mesh {
            name: name.into(),
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
            clamp: false,
            scenery: false,
            kind: MeshKind::Primitive,
            disable_lighting: 0.0,
            additive: Some(vpw_table::geometry::Additive {
                color: [1.0, 1.0, 1.0],
                alpha: 1.0,
                light: lamp.map(str::to_string),
            }),
        },
        base: Mat4::IDENTITY,
        local: Mat4::IDENTITY,
        anim: Animation::Fixed,
    }
}

#[test]
fn a_layer_of_light_adds_to_the_table_rather_than_covering_it() {
    let Some(mut held) = gpu() else { return };
    let gpu = &mut *held;
    let scene = floor_scene();
    let uploaded = gpu.upload(&scene);
    let camera = top_down();

    let parts = [light_layer("LM_test", Some("gi_001"))];
    gpu.upload_dynamic(&scene, &parts);

    // Its lamp is off, so there is nothing to add and nothing is drawn.
    gpu.dynamic
        .as_mut()
        .unwrap()
        .set_layer_level(&gpu.queue, 0, 0.0);
    let dark = gpu.render(&uploaded, &camera);

    // And on.
    gpu.dynamic
        .as_mut()
        .unwrap()
        .set_layer_level(&gpu.queue, 0, 1.0);
    let lit = gpu.render(&uploaded, &camera);

    let px = changed(&dark, &lit);
    assert!(
        px.len() > 200,
        "the layer barely changed the image: {} pixels",
        px.len()
    );

    // Added, not laid over: every pixel it touched got *brighter*. An opaque
    // black copy of the floor would fail here, and that is the whole point.
    let brighter = dark
        .as_chunks::<4>()
        .0
        .iter()
        .zip(lit.as_chunks::<4>().0)
        .filter(|(a, b)| (0..3).any(|c| b[c] > a[c]))
        .count();
    let darker = dark
        .as_chunks::<4>()
        .0
        .iter()
        .zip(lit.as_chunks::<4>().0)
        .filter(|(a, b)| (0..3).any(|c| b[c] + 8 < a[c]))
        .count();
    assert!(brighter > 200, "only {brighter} pixels got brighter");
    assert_eq!(
        darker, 0,
        "{darker} pixels got darker under a layer of light"
    );
}

/// Half as bright a lamp puts half as much light on the table.
#[test]
fn a_layer_follows_its_lamp() {
    let Some(mut held) = gpu() else { return };
    let gpu = &mut *held;
    let scene = floor_scene();
    let uploaded = gpu.upload(&scene);
    let camera = top_down();

    let parts = [light_layer("LM_test", Some("gi_001"))];
    gpu.upload_dynamic(&scene, &parts);

    let level = |gpu: &mut vpw_render::offscreen::Offscreen, v: f32| {
        gpu.dynamic
            .as_mut()
            .unwrap()
            .set_layer_level(&gpu.queue, 0, v);
        let img = gpu.render(&uploaded, &camera);
        // The middle of the square, well inside it.
        let i = ((H / 2 * W) + W / 2) as usize * 4;
        f32::from(img[i]) + f32::from(img[i + 1]) + f32::from(img[i + 2])
    };

    let off = level(gpu, 0.0);
    let half = level(gpu, 0.5);
    let full = level(gpu, 1.0);

    assert!(half > off, "half a lamp added nothing: {off} -> {half}");
    assert!(
        full > half,
        "a full lamp was no brighter than half: {half} -> {full}"
    );
}

/// A lightmap's alpha is its **coverage**: where the bake holds nothing for
/// that lamp, the layer must add nothing.
///
/// The pass blends with `SRC_ALPHA, ONE` and the shader returns
/// `staticColor_Alpha * texel` including the alpha, so a fully transparent
/// texel contributes nothing at all. Getting either half wrong adds the whole
/// atlas over the whole layer — and with ninety-six layers on a baked table,
/// turns it white.
#[test]
fn a_layer_adds_nothing_where_its_texture_is_transparent() {
    let Some(mut held) = gpu() else { return };
    let gpu = &mut *held;
    let mut scene = floor_scene();
    // One texel: white, and completely transparent.
    scene.images.push(vpw_table::geometry::Image {
        name: "clear".into(),
        encoded: None,
        rgba: Some(vec![255, 255, 255, 0]),
        width: 1,
        height: 1,
        has_alpha: true,
        alpha_test: -1.0,
        redrawn: false,
    });
    let uploaded = gpu.upload(&scene);
    let camera = top_down();

    let mut layer = light_layer("LM_clear", Some("gi_001"));
    layer.mesh.image = "clear".into();
    gpu.upload_dynamic(&scene, &[layer]);

    gpu.dynamic
        .as_mut()
        .unwrap()
        .set_layer_level(&gpu.queue, 0, 0.0);
    let dark = gpu.render(&uploaded, &camera);

    gpu.dynamic
        .as_mut()
        .unwrap()
        .set_layer_level(&gpu.queue, 0, 1.0);
    let lit = gpu.render(&uploaded, &camera);

    let px = changed(&dark, &lit);
    assert!(
        px.is_empty(),
        "a layer with no coverage changed {} pixels",
        px.len()
    );
}

/// `BlendDisableLighting = 1`: the part is its own texture, and the scene's
/// light does not touch it.
///
/// Every mesh of a baked table carries this — the lighting is painted into the
/// picture, which is the whole point of a bake — so lighting one again is how
/// a baked table comes out white. The test is that the *same* part under two
/// very different scene lights comes out identical.
#[test]
fn a_part_that_refuses_the_light_ignores_the_scene() {
    let Some(mut held) = gpu() else { return };
    let gpu = &mut *held;
    let camera = top_down();

    let render = |gpu: &mut vpw_render::offscreen::Offscreen, emission: f32, refuse: f32| {
        let mut scene = floor_scene();
        scene.lighting.emission = [emission, emission, emission];
        scene.lighting.ambient = [emission * 0.5, emission * 0.5, emission * 0.5];
        scene.meshes[0].disable_lighting = refuse;
        let uploaded = gpu.upload(&scene);
        gpu.upload_dynamic(&scene, &[]);
        gpu.render(&uploaded, &camera)
    };

    // Lit as usual, the scene's light is what decides how it looks.
    let lit_bright = render(gpu, 1.0, 0.0);
    let lit_dark = render(gpu, 0.0, 0.0);
    assert!(
        !changed(&lit_bright, &lit_dark).is_empty(),
        "the scene light changed nothing, so this test proves nothing"
    );

    // Refusing it, the same two scenes are the same picture.
    let free_bright = render(gpu, 1.0, 1.0);
    let free_dark = render(gpu, 0.0, 1.0);
    let px = changed(&free_bright, &free_dark);
    assert!(
        px.is_empty(),
        "a part that refuses the light still moved with it: {} pixels",
        px.len()
    );
}
