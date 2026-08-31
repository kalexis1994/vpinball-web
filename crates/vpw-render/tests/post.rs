//! Bloom and transmitted light, measured on the pixels.
//!
//! Both passes are invisible when they work and invisible when they do not: a
//! bind group pointing at the wrong buffer, a blur reading its own output, a
//! composite that adds nothing, all of them compile and all of them produce a
//! picture that looks broadly right. So each test here renders the same scene
//! twice — once with the pass doing its work and once with it turned off — and
//! measures the difference between the two images.
//!
//! Absolute brightness is the wrong question. Bloom moves light around rather
//! than adding it, so what identifies it is *where* the picture changed: light
//! appearing in places the geometry does not cover.
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

/// A dark floor with one very bright small light in the middle of it.
///
/// Dark on purpose: bloom is a question about what happens *around* a bright
/// thing, and a bright floor leaves nowhere for the answer to show. The light
/// is a small square so that "outside the light" is a large, well-defined
/// region rather than a few pixels at the rim.
fn one_bright_light(intensity: f32, alpha: f32) -> Scene {
    let half = 500.0;
    let v = |x: f32, y: f32| Vertex {
        pos: [x, y, 0.0],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 0.0],
    };
    let mut floor = Mesh {
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
    // Slightly above the floor, so it is not fighting it for depth.
    floor.transform = Mat4::IDENTITY;

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
        intensity,
        color: [1.0, 1.0, 1.0],
        color2: [1.0, 1.0, 1.0],
        state: 1.0,
        blinking: false,
        is_bulb: false,
        transmission_scale: 0.5,
        modulate: 0.0,
        // No fade: these tests render one frame and compare it against
        // another, and a lamp that spent that frame ramping up would be
        // measuring the ramp.
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
        // Nothing here rolls a ball, so the numbers the table would give the
        // physics are the engine's own.
        physics: vpw_table::geometry::TablePhysics {
            slope_deg: 6.0,
            gravity: 0.0,
            default_scatter_deg: 0.0,
            difficulty: 0.0,
        },
        materials: vec![vpw_table::geometry::Material {
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
            opacity: alpha,
            opacity_active: alpha < 1.0,
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
        // Nothing else lighting the scene: the lamp is the only source, so
        // every difference between two renders belongs to it.
        lighting: Lighting {
            lights: [Vec3::new(0.0, 0.0, 800.0), Vec3::new(0.0, 0.0, 800.0)],
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

/// Renders the scene and hands back the pixels.
fn shoot(gpu: &mut Offscreen, scene: &Scene, strength: f32) -> Vec<u8> {
    shoot_from(gpu, scene, strength, &top_down())
}

fn shoot_from(gpu: &mut Offscreen, scene: &Scene, strength: f32, camera: &Camera) -> Vec<u8> {
    let uploaded = gpu.upload(scene);
    gpu.upload_lights(scene);
    gpu.set_bloom(strength);
    gpu.render(&uploaded, camera)
}

/// A table's own view: down the length of it and at an angle.
///
/// The angle is what these tests need. Seen from straight above, a mirror image
/// lands exactly underneath the thing that casts it and the object hides its
/// own reflection — which is not a bug, it is what a mirror does, and it is
/// why the first version of these tests measured nothing at all.
fn tilted() -> Camera {
    let mut c = Camera::framing(
        Vec3::new(-500.0, -500.0, 0.0),
        Vec3::new(500.0, 500.0, 200.0),
        W as f32 / H as f32,
    );
    c.inclination = 40.0;
    c
}

/// Average luminance over a ring around the centre — outside the lamp's own
/// geometry, inside where a wide blur reaches.
fn ring(pixels: &[u8]) -> f32 {
    let (cx, cy) = (W as f32 / 2.0, H as f32 / 2.0);
    let mut total = 0.0;
    let mut count = 0.0;
    for y in 0..H {
        for x in 0..W {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            // The lamp covers roughly the middle fifth of the frame.
            if !(24.0..56.0).contains(&d) {
                continue;
            }
            let i = ((y * W + x) * 4) as usize;
            total += f32::from(pixels[i]) + f32::from(pixels[i + 1]) + f32::from(pixels[i + 2]);
            count += 3.0;
        }
    }
    total / count
}

/// Average over the middle of the frame, where the lamp itself is.
fn middle(pixels: &[u8]) -> f32 {
    let (cx, cy) = (W as f32 / 2.0, H as f32 / 2.0);
    let mut total = 0.0;
    let mut count = 0.0;
    for y in 0..H {
        for x in 0..W {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            if d > 12.0 {
                continue;
            }
            let i = ((y * W + x) * 4) as usize;
            total += f32::from(pixels[i]) + f32::from(pixels[i + 1]) + f32::from(pixels[i + 2]);
            count += 3.0;
        }
    }
    total / count
}

/// The point of the whole pass: light outside the thing that emits it.
#[test]
fn a_bright_lamp_spills_past_its_own_edge() {
    let Some(mut gpu) = gpu() else { return };
    // Well over the threshold, which is 2.5 after tone mapping — the lamp has
    // to be genuinely blowing out for there to be anything to bloom.
    let scene = one_bright_light(30.0, 1.0);

    let without = shoot(&mut gpu, &scene, 0.0);
    let with = shoot(&mut gpu, &scene, 1.8);

    let (a, b) = (ring(&without), ring(&with));
    assert!(
        b > a + 4.0,
        "the ring around the lamp should be lit by the bloom: {a:.1} without, {b:.1} with"
    );
}

/// And it is only the bright things that spill.
///
/// A lamp below the threshold has to leave the picture alone, or bloom turns
/// into a blur over the whole table.
#[test]
fn a_dim_lamp_does_not() {
    let Some(mut gpu) = gpu() else { return };
    let scene = one_bright_light(0.4, 1.0);

    let without = shoot(&mut gpu, &scene, 0.0);
    let with = shoot(&mut gpu, &scene, 1.8);

    let (a, b) = (ring(&without), ring(&with));
    assert!(
        (b - a).abs() < 1.5,
        "a lamp under the threshold should not bloom: {a:.1} without, {b:.1} with"
    );
}

/// Bloom must not eat the thing it came from.
#[test]
fn the_lamp_itself_stays_lit() {
    let Some(mut gpu) = gpu() else { return };
    let scene = one_bright_light(30.0, 1.0);

    let without = shoot(&mut gpu, &scene, 0.0);
    let with = shoot(&mut gpu, &scene, 1.8);

    assert!(
        middle(&with) >= middle(&without) - 1.0,
        "the middle got darker: {:.1} to {:.1}",
        middle(&without),
        middle(&with)
    );
}

/// Light coming up through a translucent surface.
///
/// The floor is made partly transparent, which is what a plastic insert cover
/// is, and the lamp is put underneath it. The material shader should pick the
/// lamp up out of the transmitted-light buffer and add it — so the floor
/// *around* the lamp brightens, not just the lamp's own square.
///
/// Measured against the same scene with an opaque floor, since an opaque
/// surface is exactly the case the original skips (`BasicShader.hlsl:330`).
#[test]
fn a_translucent_surface_lets_the_lamp_through() {
    let Some(mut gpu) = gpu() else { return };

    // A bulb light, because the transmitted-light buffer takes bulb lights and
    // only bulb lights: the original leaves `Light::Render` before drawing
    // anything at all for a classic one (`light.cpp:600`). A classic insert is
    // artwork lit from behind, and light does not come *out* of the playfield
    // where one is.
    let bulb = |alpha| {
        let mut scene = one_bright_light(6.0, alpha);
        for l in &mut scene.lights {
            l.is_bulb = true;
            // What `build` would clamp a bulb's blend to: zero disables the
            // blend outright (`light.cpp:830`).
            l.modulate = 0.0001;
            l.transmission_scale = 1.0;
        }
        scene
    };

    // No bloom in either, so nothing but the transmission can account for a
    // difference in the ring.
    let opaque = shoot(&mut gpu, &bulb(1.0), 0.0);
    let clear = shoot(&mut gpu, &bulb(0.6), 0.0);

    let (a, b) = (ring(&opaque), ring(&clear));
    assert!(
        b > a + 2.0,
        "the translucent floor should glow where the lamp is under it: \
         {a:.1} opaque, {b:.1} translucent"
    );
}

/// A floating slab above the floor, for the playfield to mirror.
///
/// It is put off to one side so that "did the reflection appear" can be asked
/// of the floor *beside* it rather than under it, where the slab itself is what
/// the camera sees.
fn scene_with_something_to_mirror(reflection: f32) -> Scene {
    let mut scene = one_bright_light(6.0, 1.0);
    scene.lighting.reflection_strength = reflection;
    // Something has to light the slab or there is nothing to mirror: the
    // lamp in `one_bright_light` is a halo drawn on its own and does not
    // illuminate anything, and the scene is otherwise pitch dark on purpose.
    scene.lighting.ambient = [0.7, 0.7, 0.7];
    scene.materials.push(vpw_table::geometry::Material {
        name: "slab".into(),
        base_color: [1.0, 1.0, 1.0],
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
    });

    let (half, height) = (150.0, 120.0);
    let v = |x: f32, y: f32| Vertex {
        pos: [x, y, height],
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 0.0],
    };
    scene.meshes.push(Mesh {
        name: "Slab".into(),
        vertices: vec![
            v(-half, -half),
            v(half, -half),
            v(half, half),
            v(-half, half),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        transform: Mat4::from_translation(Vec3::new(0.0, 260.0, 0.0)),
        image: String::new(),
        material: "slab".into(),
        visible: true,
        clamp: false,
        scenery: false,
        kind: MeshKind::Primitive,
        additive: None,
        depth_bias: 0.0,
        disable_lighting: 0.0,
    });
    scene
}

/// Average luminance over the whole frame.
///
/// The reflection only ever **adds** — the original is explicit that it is not
/// mixed in by the Fresnel term — so the picture getting brighter is the whole
/// question, and asking it of the whole frame avoids having to work out where
/// on the floor a given slab's mirror image lands.
fn whole(pixels: &[u8]) -> f32 {
    let total: f32 = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2]))
        .sum();
    total / (pixels.len() as f32 / 4.0 * 3.0)
}

/// The playfield shows what stands on it.
#[test]
fn the_floor_mirrors_what_is_above_it() {
    let Some(mut gpu) = gpu() else { return };

    let camera = tilted();
    let without = shoot_from(&mut gpu, &scene_with_something_to_mirror(0.0), 0.0, &camera);
    let with = shoot_from(&mut gpu, &scene_with_something_to_mirror(1.0), 0.0, &camera);

    // The band the slab's reflection lands in, which is on the near side of it
    // — the mirror of something above the floor appears below the floor, so the
    // camera sees it beyond the object rather than under it.
    let (a, b) = (whole(&without), whole(&with));
    assert!(
        b > a + 0.5,
        "the floor should have picked up the slab: {a:.2} without, {b:.2} with"
    );
}

/// And only surfaces that face it.
///
/// The reflection is selected by `smoothstep(0.5, 0.9, dot(N, up))`, so a
/// surface square to the playfield takes all of it and one tilted past sixty
/// degrees takes none. Without that, every wall on the table would mirror the
/// playfield sideways.
#[test]
fn a_surface_turned_away_takes_no_reflection() {
    let Some(mut gpu) = gpu() else { return };

    let mut scene = scene_with_something_to_mirror(1.0);
    // Stand the floor on its edge: same geometry, same everything, normal now
    // pointing along y instead of up.
    for mesh in &mut scene.meshes {
        if mesh.name == "Floor" {
            for vertex in &mut mesh.vertices {
                vertex.normal = [0.0, 1.0, 0.0];
            }
        }
    }
    let camera = tilted();
    let turned = shoot_from(&mut gpu, &scene, 0.0, &camera);

    let mut flat = scene.clone();
    flat.lighting.reflection_strength = 0.0;
    let dark = shoot_from(&mut gpu, &flat, 0.0, &camera);

    let (a, b) = (whole(&dark), whole(&turned));
    assert!(
        (b - a).abs() < 0.2,
        "a floor facing sideways should mirror nothing: {a:.2} against {b:.2}"
    );
}

#[test]
fn the_passes_draw_into_a_format_this_device_can_draw_into() {
    // The table does not go straight to the screen: it goes into a
    // floating-point buffer, and the transmitted light and the bloom are built
    // from it. Sixteen bits a channel is guaranteed under WebGPU and is an
    // extension under WebGL2 — `EXT_color_buffer_half_float` — which most
    // desktops have and some older phones do not.
    //
    // Asking for it where it is missing does not degrade gracefully: the
    // texture fails to create and the canvas stays black. So the format is
    // chosen from the adapter, and this is the invariant that has to hold
    // whichever one it picked. It runs on whatever backend the machine offers,
    // which is the point — force `WGPU_BACKEND=gl` and it is the WebGL2 path.
    let Some(gpu) = gpu() else { return };

    let chosen = gpu.hdr_format();
    let usages = gpu.adapter_format_usages(chosen);
    assert!(
        usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
        "the passes were given {chosen:?}, which this device cannot draw into"
    );
    assert!(
        usages.contains(wgpu::TextureUsages::TEXTURE_BINDING),
        "and the composite has to be able to sample it back"
    );
}

/// The browser's quality ladder, exercised on the same seams the player
/// crosses: the scene draws at a fraction of the output, the composite
/// stretches it back, and the output never changes size — which is what
/// makes a tier change invisible. The picture has to stay the same
/// picture, only softer: same amount of light on the floor, and a lamp
/// still lit after climbing back to full size.
#[test]
fn a_scaled_render_is_the_same_picture_stretched() {
    let Some(mut gpu) = gpu() else { return };
    let scene = one_bright_light(30.0, 1.0);
    let full = shoot(&mut gpu, &scene, 0.0);

    gpu.set_render_scale(0.55);
    let scaled = shoot(&mut gpu, &scene, 0.0);
    gpu.set_render_scale(1.0);
    let back = shoot(&mut gpu, &scene, 0.0);

    assert_eq!(
        full.len(),
        scaled.len(),
        "the output must stay output-sized"
    );
    let mean = |img: &[u8]| {
        img.chunks(4)
            .map(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]))
            .sum::<u32>() as f64
            / (img.len() / 4) as f64
    };
    let a = mean(&full);
    let b = mean(&scaled);
    let c = mean(&back);
    assert!(a > 1.0, "the full-size photograph must not be black");
    assert!(
        (a - b).abs() / a < 0.2,
        "at 55% the floor holds its light: full {a:.2}, scaled {b:.2}"
    );
    assert!(
        (a - c).abs() / a < 0.02,
        "back at 100% the photograph is the first one again: {a:.2} vs {c:.2}"
    );
}
