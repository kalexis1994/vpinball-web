//! Photographs the flippers at rest and raised, from above.
//!
//! ```text
//! cargo run --release -p vpw-game --example flippers -- table.vpx out-prefix [scripts-dir]
//! ```
//!
//! It exists because "the flipper does not look right" is not something a test
//! can be written for until you know what it looks like. Two pictures of the
//! same corner, one with the flippers down and one with them up, say in a
//! glance what a hundred assertions about matrices cannot.

use std::rc::Rc;

use vpw_game::{Game, LibraryDir, NoLibraries, Resources, ScriptLibrary};
use vpw_math::Vec3;

fn main() {
    let mut args = std::env::args().skip(1);
    let table = args
        .next()
        .expect("usage: flippers <table.vpx> [prefix] [scripts]");
    let prefix = args.next().unwrap_or_else(|| "flippers".into());
    let scripts = args.next();
    let roms = args.next();

    let bytes = std::fs::read(&table).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);

    let libraries: Rc<dyn ScriptLibrary> = match &scripts {
        Some(dir) => Rc::new(LibraryDir(dir.into())),
        None => Rc::new(NoLibraries),
    };
    let mut resources = Resources::new(libraries);
    if let Some(dir) = &roms {
        let source: Rc<dyn vpw_game::controller::RomSource> =
            Rc::new(vpw_game::controller::RomDir(dir.into()));
        resources = resources.with_roms(source);
    }
    let mut game = Game::load(&vpx, &mut scene, resources)
        .unwrap_or_else(|e| panic!("the table failed to load: {e}"));
    game.start().ok();

    // With a ROM, put a coin in and start a game: a table's lamps are the
    // *game's* lamps, and in attract mode almost none of them are on.
    if game.machine().is_running() {
        for _ in 0..4000 {
            game.step();
        }
        for (key, hold, after) in [("Digit5", 60, 2500), ("Digit1", 60, 3000)] {
            game.key(key, true);
            for _ in 0..hold {
                game.step();
            }
            game.key(key, false);
            for _ in 0..after {
                game.step();
            }
        }
        println!("machine says {:?}", game.machine().displays());
    }

    // VPW_NO=substring drops meshes by name before uploading, which is how you
    // find out who is drawing something you cannot account for.
    if let Ok(drop) = std::env::var("VPW_NO") {
        let drop = drop.to_lowercase();
        scene
            .meshes
            .retain(|m| !m.name.to_lowercase().contains(drop.as_str()));
    }

    let (width, height) = if std::env::var("VPW_FRAME").as_deref() == Ok("table") {
        (700u32, 1100u32)
    } else {
        (900u32, 900u32)
    };
    let mut gpu = pollster::block_on(vpw_render::offscreen::Offscreen::new(width, height))
        .expect("could not initialise wgpu");
    let gpu_scene = gpu.upload(&scene);
    gpu.upload_lights(&scene);
    println!("{} lamps", gpu.lights.names.len());

    // Frame the flippers: look straight down at the point between them, close
    // enough that the shape of a bat is legible.
    //
    // VPW_FRAME=table pulls back to the whole thing instead. The flippers are
    // what this program was written for, but it is also the only one that
    // photographs a table with its ROM running, and some of what a ROM does —
    // the score, the backbox — is nowhere near the flippers.
    // VPW_AT=x,y points it somewhere else on the table — the backbox display,
    // a saucer, whatever is in question.
    let whole = std::env::var("VPW_FRAME").as_deref() == Ok("table");
    let at = match std::env::var("VPW_AT") {
        Ok(spec) => {
            let mut n = spec.split(',').filter_map(|v| v.trim().parse::<f32>().ok());
            Vec3::new(n.next().unwrap_or(0.0), n.next().unwrap_or(0.0), 0.0)
        }
        Err(_) => flipper_centre(&game),
    };
    let camera = if whole {
        let (min, max) = gpu_scene.bounds;
        let mut c = vpw_render::Camera::framing(min, max, width as f32 / height as f32);
        c.target.y += (max.y - min.y) * 0.05;
        c
    } else {
        let mut c = vpw_render::Camera::framing(
            Vec3::new(at.x - 230.0, at.y - 230.0, at.z - 10.0),
            Vec3::new(at.x + 230.0, at.y + 230.0, at.z + 60.0),
            width as f32 / height as f32,
        );
        c.target = at;
        // Nearly straight down: the question is the shape of the bat, and a low
        // angle hides half of it behind the plastics.
        c.inclination = 78.0;
        c
    };

    for (label, down) in [("down", true), ("up", false)] {
        game.key("KeyZ", !down);
        game.key("KeyM", !down);
        // Long enough for the flipper to finish travelling: a flipper takes
        // about 40 ms to swing, and the physics runs at 1 kHz.
        for _ in 0..250 {
            game.step();
        }
        // VPW_STATIC=1 leaves the moving pieces out, so the picture shows only
        // what was baked. Anything flipper-shaped still in it is geometry that
        // should have been taken out of the scene and was not.
        if std::env::var("VPW_STATIC").is_err() {
            // The lamps, as the script has them.
            {
                let queue = gpu.queue.clone();
                for i in 0..gpu.lights.names.len() {
                    let level = game
                        .items()
                        .get(&gpu.lights.names[i])
                        .map_or(1.0, |item| item.light_level());
                    gpu.lights.set_state(&queue, i, level, 1.0);
                }
                let lit = (0..gpu.lights.names.len())
                    .filter(|&i| {
                        game.items()
                            .get(&gpu.lights.names[i])
                            .is_none_or(|it| it.light_level() > 0.0)
                    })
                    .count();
                println!("   {lit} lit");
            }
            gpu.upload_dynamic(&scene, game.parts());
        }
        // The upload only carries the pose the parts were built in. Where they
        // are *now* comes from the engine, the same way the player's own frame
        // loop does it — without this the photo always shows the rest pose,
        // whatever the physics has been up to.
        if let Some(dynamic) = gpu.dynamic.as_mut() {
            let queue = gpu.queue.clone();
            for i in 0..game.parts().len() {
                dynamic.set_part_transform(&queue, i, game.part_transform(i));
                dynamic.set_part_visible(i, game.part_visible(i));
            }
        }
        let pixels = gpu.render(&gpu_scene, &camera);
        let path = format!("{prefix}-{label}.png");
        write_png(&path, width, height, &pixels);
        println!("{path}");
        let engine = game.engine.borrow();
        for part in game.parts() {
            if !part.mesh.name.to_lowercase().contains("flip") {
                continue;
            }
            let m = part.transform(&engine);
            // The direction the bat points, taken from the matrix itself.
            let dir = m.x_axis;
            println!(
                "   {:16} points ({:+.2},{:+.2}) verts={}",
                part.mesh.name,
                dir.x,
                dir.y,
                part.mesh.vertices.len()
            );
        }
    }
}

/// The midpoint between the two flippers nearest the drain.
///
/// Not the average of every flipper: a table's diverters and upper flippers are
/// flippers too, and averaging them all frames the middle of the playfield,
/// where there is nothing to see.
fn flipper_centre(game: &Game) -> Vec3 {
    let engine = game.engine.borrow();
    let mut centres: Vec<(f32, f32)> = engine
        .shapes()
        .iter()
        .filter_map(|s| match s {
            vpw_physics::engine::Shape::Flipper(f) => Some((f.center.x, f.center.y)),
            _ => None,
        })
        .collect();
    if centres.is_empty() {
        return Vec3::ZERO;
    }
    // The player's end of the table is the high end of y.
    centres.sort_by(|a, b| b.1.total_cmp(&a.1));
    centres.truncate(2);
    let n = centres.len() as f32;
    Vec3::new(
        centres.iter().map(|c| c.0).sum::<f32>() / n,
        centres.iter().map(|c| c.1).sum::<f32>() / n,
        0.0,
    )
}

/// A PNG, written by hand: one filter byte per row and a single stored deflate
/// block, so there is no dependency for something this program only does twice.
fn write_png(path: &str, width: u32, height: u32, rgba: &[u8]) {
    let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    for row in rgba.chunks_exact(width as usize * 4) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }

    let mut png = Vec::new();
    png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png).expect("could not write the png");
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = crc32(kind);
    crc = crc32_continue(crc, data);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// zlib with every block stored. Bigger than compressing, and it never lies.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    for (i, block) in data.chunks(0xFFFF).enumerate() {
        let last = u8::from((i + 1) * 0xFFFF >= data.len());
        out.push(last);
        out.extend_from_slice(&(block.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// A CRC over one run of bytes. `crc32_continue` inverts its input, so starting
/// it from zero is what gives the standard all-ones initial register.
fn crc32(data: &[u8]) -> u32 {
    crc32_continue(0, data)
}

fn crc32_continue(crc: u32, data: &[u8]) -> u32 {
    let mut c = crc ^ 0xFFFF_FFFF;
    for &byte in data {
        c ^= u32::from(byte);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    c ^ 0xFFFF_FFFF
}
