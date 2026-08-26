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
    let radius = 25.0;
    let m = Mat4::from_translation(Vec3::new(x, y, radius))
        * Mat4::from_scale(Vec3::splat(radius))
        * Mat4::from_quat(Quat::IDENTITY);
    gpu.dynamic
        .as_mut()
        .unwrap()
        .set_ball_transform(&gpu.queue, 0, Some(m));

    let b = scene.playfield;
    let mut camera = vpw_render::Camera::framing(b.min, b.max, w as f32 / h as f32);
    camera.inclination = 55.0;
    camera.distance *= 0.55;
    let pixels = gpu.render(&uploaded, &camera);
    image::save_buffer(&out, &pixels, w, h, image::ColorType::Rgba8).unwrap();
    eprintln!("saved {out}");
}
