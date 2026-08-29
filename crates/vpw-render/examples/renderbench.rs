//! Times the frame, and nothing else.
//!
//!     cargo run --release -p vpw-render --example renderbench -- table.vpx [frames]
//!
//! A rendering change has to answer to a number, and fps in a browser
//! saturates at the display's refresh. This submits a batch of frames with no
//! read-back, waits for the queue, and divides. `VPW_BAKE`, `VPW_LIGHTS` and
//! `VPW_ENV` mean what they mean in `ballshot`, so the timed frame is the
//! frame a player actually gets.

fn main() {
    let mut args = std::env::args().skip(1);
    let table = args.next().expect("usage: renderbench table.vpx [frames]");
    let frames: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);

    let bytes = std::fs::read(&table).unwrap();
    let vpx = vpin::vpx::from_bytes(&bytes).unwrap();
    let mut scene = vpw_table::geometry::extract(&vpx);
    if let Ok(path) = std::env::var("VPW_LIGHTS") {
        let text = std::fs::read_to_string(path).unwrap();
        for line in text.lines() {
            if let Some((name, level)) = line.split_once('\t') {
                for light in &mut scene.lights {
                    if light.name.eq_ignore_ascii_case(name) {
                        light.state = level.parse().unwrap_or(0.0);
                    }
                }
            }
        }
    }

    // VPW_LIT=1 turns every lamp on, which is the attract-mode worst case
    // and the frame a slow report is usually about.
    if std::env::var("VPW_LIT").is_ok() {
        for light in &mut scene.lights {
            light.state = 1.0;
        }
    }
    eprintln!(
        "{} lights in the file, {} lit",
        scene.lights.len(),
        scene.lights.iter().filter(|l| l.state > 0.0).count()
    );

    let (w, h) = (720u32, 1280u32);
    let mut gpu = pollster::block_on(vpw_render::offscreen::Offscreen::new(w, h)).unwrap();
    eprintln!("adapter: {}", gpu.adapter);
    let uploaded = gpu.upload(&scene);
    gpu.upload_lights(&scene);
    gpu.upload_flashers(&scene);
    gpu.upload_dynamic(&scene, &[]);
    if std::env::var("VPW_BAKE").is_ok() {
        gpu.bake_gi(&scene);
    }
    if let Ok(path) = std::env::var("VPW_ENV") {
        assert!(gpu.set_environment_hdr(&std::fs::read(path).unwrap()));
    }

    let b = scene.playfield;
    let camera = vpw_render::Camera::framing(b.min, b.max, w as f32 / h as f32);

    // Warm up: pipelines compile lazily on some drivers, and the first frame
    // pays for it.
    for _ in 0..10 {
        gpu.draw_only(&uploaded, &camera, |_| true);
    }
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());

    let t = std::time::Instant::now();
    for _ in 0..frames {
        gpu.draw_only(&uploaded, &camera, |_| true);
        // One flush per frame, as the browser does: letting hundreds queue up
        // measures how deep the driver's buffer is, not how long a frame takes.
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    }
    let total = t.elapsed();
    println!(
        "{frames} frames in {:.1?}  ->  {:.3} ms/frame",
        total,
        total.as_secs_f64() * 1000.0 / f64::from(frames)
    );
}
