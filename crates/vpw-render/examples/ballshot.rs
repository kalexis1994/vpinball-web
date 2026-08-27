//! Photographs the table with a ball standing wherever you put it.
//!
//!     cargo run --release -p vpw-render --example ballshot -- table.vpx out.png [x y]
//!
//! The ball is the one part whose look cannot be judged from a table shot —
//! it exists to reflect what surrounds it, so it has to be photographed *on*
//! a field, lit. `VPW_LIGHTS` takes a lamp dump the way `shot` does, and
//! `VPW_BAKE=1` traces the GI first, which is the light the ball mirrors.
use vpw_math::{Mat4, Quat, Vec3};

fn main() {
    let mut args = std::env::args().skip(1);
    let table = args
        .next()
        .expect("usage: ballshot table.vpx out.png [x y]");
    let out = args
        .next()
        .expect("usage: ballshot table.vpx out.png [x y]");
    let x: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(480.0);
    let y: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1500.0);

    let bytes = std::fs::read(&table).unwrap();
    let vpx = vpin::vpx::from_bytes(&bytes).unwrap();
    let mut scene = vpw_table::geometry::extract(&vpx);
    if std::env::var("VPW_LIGHTS").is_ok() {
        let text = std::fs::read_to_string(std::env::var("VPW_LIGHTS").unwrap()).unwrap();
        let mut levels = std::collections::HashMap::new();
        for line in text.lines() {
            if let Some((name, level)) = line.split_once(char::from(9)) {
                levels.insert(
                    name.to_ascii_lowercase(),
                    level.parse::<f32>().unwrap_or(0.0),
                );
            }
        }
        for light in &mut scene.lights {
            if let Some(&level) = levels.get(&light.name.to_ascii_lowercase()) {
                light.state = level;
            }
        }
    }

    let (w, h) = (720u32, 1280u32);
    let mut gpu = pollster::block_on(vpw_render::offscreen::Offscreen::new(w, h)).unwrap();
    let uploaded = gpu.upload(&scene);
    gpu.upload_lights(&scene);
    gpu.upload_flashers(&scene);
    gpu.upload_dynamic(&scene, &[]);
    if std::env::var("VPW_BAKE").is_ok() {
        let n = gpu.bake_gi(&scene);
        eprintln!("baked {n} groups");
    }
    // VPW_ENV=room.hdr photographs the table inside a room: the same
    // Radiance map the player can choose in the page.
    if let Ok(path) = std::env::var("VPW_ENV") {
        let bytes = std::fs::read(&path).expect("could not read the room's map");
        assert!(gpu.set_environment_hdr(&bytes), "the room did not decode");
        eprintln!("environment: {path}");
    }
    // VPW_NOBALL=1 photographs the same framing with no ball at all, for
    // telling an artifact around the ball from the artwork under it.
    let no_ball = std::env::var("VPW_NOBALL").is_ok();
    // VPW_SPIN=degrees rolls the ball about the x axis before the photo: two
    // shots at different angles are the proof that the wear turns with the
    // ball while the reflections stay put.
    let spin: f32 = std::env::var("VPW_SPIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let radius = 25.0;
    let m = Mat4::from_translation(Vec3::new(x, y, radius))
        * Mat4::from_scale(Vec3::splat(radius))
        * Mat4::from_quat(Quat::from_rotation_x(spin.to_radians()));
    if !no_ball {
        gpu.dynamic
            .as_mut()
            .unwrap()
            .set_ball_transform(&gpu.queue, 0, Some(m));
    }

    let b = scene.playfield;
    let mut camera = vpw_render::Camera::framing(b.min, b.max, w as f32 / h as f32);
    // VPW_TOPDOWN=1 looks straight down, which maps world to screen simply
    // enough to find the ball in the pixels from a script.
    if std::env::var("VPW_TOPDOWN").is_ok() {
        camera.inclination = 89.0;
    } else {
        // VPW_INCL picks the angle; the front view a player actually holds
        // is a low one, and it is where a mirror ball is hardest to draw.
        camera.inclination = std::env::var("VPW_INCL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(55.0);
        camera.distance *= 0.55;
    }
    let pixels = gpu.render(&uploaded, &camera);
    image::save_buffer(&out, &pixels, w, h, image::ColorType::Rgba8).unwrap();
    eprintln!("saved {out}");
}
