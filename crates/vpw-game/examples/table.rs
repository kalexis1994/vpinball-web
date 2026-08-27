//! Runs a real table with its ROM, headless, and says what the machine is doing.
//!
//!     cargo run --release -p vpw-game --example table -- table.vpx roms/ [seconds]
//!
//! The browser is a bad place to find out why a table is quiet: there is no
//! way to look inside it. This drives the same code the browser drives — the
//! script, the physics, the board, the frame timers — and then prints the two
//! things that say what state the machine is in: what the display says, and
//! which switches the table is reporting.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use vpw_game::controller::{RomDir, RomSource};
use vpw_game::{Game, LibraryDir, Resources, ScriptLibrary};

const SCRIPTS: &str = "../../../vpinball/scripts";

/// Two channels of sixteen-bit samples, in the only container everything reads.
fn write_wav(path: &str, samples: &[u8], rate: u32) {
    let mut out = Vec::with_capacity(samples.len() + 44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    out.extend_from_slice(samples);
    let _ = std::fs::write(path, out);
}

fn shape_kind(s: &vpw_physics::engine::Shape) -> &'static str {
    use vpw_physics::engine::Shape as S;
    match s {
        S::Line(_) => "Line",
        S::LineZ(_) => "LineZ",
        S::Line3D(_) => "Line3D",
        S::Circle(_) => "Circle",
        S::Plane(_) => "Plane",
        S::Point(_) => "Point",
        S::Poly(_) => "Poly",
        S::Triangle(_) => "Triangle",
        S::Slingshot(_) => "Slingshot",
        S::Bumper(_) => "Bumper",
        S::Gate(_) => "Gate",
        S::Spinner(_) => "Spinner",
        S::Kicker(_) => "Kicker",
        S::Flipper(_) => "Flipper",
        S::Plunger(_) => "Plunger",
    }
}

/// The bake's candidate rule, spelled out here because this example must not
/// pull the renderer in: bulbs whose reach is field-scale. Keep in step with
/// `vpw_render::bake::field_scale_candidates`.
fn vpw_render_candidates(scene: &vpw_table::geometry::Scene) -> Vec<String> {
    scene
        .lights
        .iter()
        .filter(|l| l.is_bulb && l.falloff_radius >= 120.0 && l.intensity > 0.0)
        .map(|l| l.name.clone())
        .collect()
}

/// Logging without a logger crate: for a diagnostic tool, stderr is the place.
fn stderr_logger() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            eprintln!("[{}] {}", record.level(), record.args());
        }
        fn flush(&self) {}
    }
    static LOGGER: Stderr = Stderr;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}

fn main() {
    stderr_logger();
    let mut args = std::env::args().skip(1);
    let table = args
        .next()
        .expect("usage: table <table.vpx> <roms/> [seconds]");
    let roms = args
        .next()
        .expect("usage: table <table.vpx> <roms/> [seconds]");
    let seconds: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);

    let bytes = std::fs::read(&table).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(Path::new(SCRIPTS).into()));
    let source: Rc<dyn RomSource> = Rc::new(RomDir(PathBuf::from(&roms)));
    let mut game = Game::load(
        &vpx,
        &mut scene,
        Resources::new(libraries).with_roms(source),
    )
    .unwrap_or_else(|e| panic!("the table failed to load: {e}"));
    game.start().unwrap_or_else(|e| panic!("Table1_Init: {e}"));

    // The pieces that move a ball, with where they are, so a fault a player
    // describes by position can be named. VPX's y grows toward the player, so
    // "bottom left" is small x and large y.
    if std::env::var("VPW_MECHS").is_ok() {
        use vpin::vpx::gameitem::GameItemEnum as G;
        let mut rows: Vec<(f32, f32, &str, String)> = Vec::new();
        for item in &vpx.gameitems {
            let (c, kind) = match item {
                G::Kicker(k) => (k.center, "Kicker"),
                G::Gate(g) => (g.center, "Gate"),
                G::Bumper(b) => (b.center, "Bumper"),
                G::Spinner(sp) => (sp.center, "Spinner"),
                _ => continue,
            };
            rows.push((c.x, c.y, kind, item.name().to_string()));
        }
        // Nearest the bottom-left corner first. VPX's y grows toward the
        // player, so that corner is small x and large y.
        let far = vpx.gamedata.bottom;
        rows.sort_by(|a, b| {
            let d = |r: &(f32, f32, &str, String)| r.0 * r.0 + (far - r.1) * (far - r.1);
            d(a).partial_cmp(&d(b)).unwrap()
        });
        println!("mechanisms, nearest the bottom-left corner first:");
        for (x, y, kind, name) in rows.iter().take(12) {
            println!("  {kind:<8} {name:<22} at ({x:>6.0}, {y:>6.0})");
        }
        for item in &vpx.gameitems {
            if let G::Gate(g) = item
                && (g.name == "Gate7" || std::env::var("VPW_MECHS").as_deref() == Ok("all"))
            {
                println!(
                    "  {} two_way {} collidable {} rot {} min {} max {} len {} visible {}",
                    g.name,
                    g.two_way,
                    g.is_collidable,
                    g.rotation,
                    g.angle_min,
                    g.angle_max,
                    g.length,
                    g.is_visible
                );
            }
            if let G::Kicker(k) = item
                && k.name == "sw9"
            {
                println!(
                    "  {} enabled {} type {:?} angle {} speed {} radius {} legacy {:?} fall_through {:?} hit_accuracy {:?}",
                    k.name,
                    k.is_enabled,
                    k.kicker_type,
                    k.orientation,
                    k.scatter,
                    k.radius,
                    k.legacy_mode,
                    k.fall_through,
                    k.hit_accuracy
                );
            }
        }
        println!();
    }
    // VPW_NEARBY=x,y,z prints every collision line within 80 units, which is
    // how a mystery wall gets a name.
    if let Ok(spec) = std::env::var("VPW_NEARBY") {
        let p: Vec<f32> = spec
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        if let [x, y, z] = p[..] {
            let at = vpw_math::Vec3::new(x, y, z);
            let engine = game.engine.borrow();
            for (i, s) in engine.shapes().iter().enumerate() {
                if let vpw_physics::engine::Shape::Line(l) = s {
                    let (a, b) = (l.v1, l.v2);
                    let mid = (a + b) * 0.5;
                    let at2 = vpw_math::Vec2::new(at.x, at.y);
                    if (mid - at2).length() < 60.0 && l.z_low <= z && z <= l.z_high {
                        println!(
                            "  line2d #{i}  ({:.0},{:.0}) -> ({:.0},{:.0})  z {:.0}..{:.0}  normal ({:.2},{:.2})",
                            a.x, a.y, b.x, b.y, l.z_low, l.z_high, l.normal.x, l.normal.y
                        );
                    }
                }
                if let vpw_physics::engine::Shape::Line3D(l) = s {
                    let (a, b) = l.endpoints();
                    let mid = (a + b) * 0.5;
                    if (mid - at).length() < 80.0
                        || (a - at).length() < 80.0
                        || (b - at).length() < 80.0
                    {
                        println!(
                            "  line3d #{i}  ({:.0},{:.0},{:.0}) -> ({:.0},{:.0},{:.0})",
                            a.x, a.y, a.z, b.x, b.y, b.z
                        );
                    }
                }
            }
        }
    }
    println!("board running   {}", game.machine().is_running());
    println!("game name       {:?}", game.machine().game_name());
    println!();

    // Coin, then start, the way a player does: hold the key for a moment and
    // let it go. Both go through the script, not the board, because that is
    // the path a browser takes.
    let press = |game: &mut Game, code: &str, at: u32, t: u32| {
        if t == at {
            game.key(code, true);
        }
        if t == at + 60 {
            game.key(code, false);
        }
    };

    // What the board drives, watched rather than taken: `changed_lamps` and
    // its friends are consumed by whoever asks first, and the table's own
    // script is asking. A probe that drains them is a probe that stops the
    // table working while it looks at it.
    let mut lamps_ever = [false; 128];
    let mut solenoids_ever = 0u32;
    let mut lit_now = 0usize;
    // And what the *table* has: the board's lamps only matter once the script
    // has copied them onto the pieces the renderer draws, and that is a
    // different set of numbers from the board's.
    let mut table_lit_ever = 0usize;
    let mut table_lit_now = 0usize;
    let mut table_lights = 0usize;
    let mut ever: std::collections::HashSet<String> = std::collections::HashSet::new();

    // VPW_DROP=x,y drops a ball at a playfield spot once the game is running
    // and reports where it is every quarter second, which is how a fault a
    // player describes as "it gets stuck over there" is turned into a place
    // and a shape.
    let drop: Option<(f32, f32)> = std::env::var("VPW_DROP").ok().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
    });
    let mut trail: Vec<(u32, vpw_math::Vec3, f32)> = Vec::new();
    let mut jumped = false;

    let mut heard = 0usize;
    let mut loud = 0usize;
    let mut peak = 0.0f32;
    let mut energy = 0.0f64;
    let mut wav: Vec<u8> = Vec::new();
    let mut silent_runs = Vec::new();
    let mut run_of_silence = 0usize;
    for t in 0..seconds * 1000 {
        // VPW_VOLUME_UP=<n> taps the coin door's up button n times before the
        // coin goes in, the way an operator turns a machine up. The volume is
        // the machine's own setting and it starts wherever a factory-fresh one
        // starts.
        if let Ok(v) = std::env::var("VPW_VOLUME_UP")
            && let Ok(taps) = v.parse::<u32>()
        {
            for i in 0..taps {
                press(
                    &mut game,
                    std::env::var("VPW_VOLUME_KEY")
                        .unwrap_or_else(|_| "Digit8".into())
                        .as_str(),
                    200 + i * 200,
                    t,
                );
            }
        }
        // Two seconds to settle, a coin, then start.
        press(&mut game, "Digit5", 2_000, t);
        press(&mut game, "Digit1", 4_000, t);
        // VPW_CALL="Name@ms,Name@ms" calls a script Sub at a moment, which is
        // how one half of a mechanism gets exercised without the other: a
        // VUK's release can be fired without a ball ever finding the hole.
        if let Ok(list) = std::env::var("VPW_CALL") {
            for spec in list.split(',') {
                if let Some((name, at)) = spec.trim().split_once('@')
                    && let Ok(at) = at.trim().parse::<u32>()
                    && t == at
                {
                    // `Name:arg@ms` passes one boolean or number along.
                    let (name, args) = match name.split_once(':') {
                        Some((n, a)) => {
                            let v = match a.trim() {
                                "1" | "true" => vpw_vbscript::value::Value::Bool(true),
                                "0" | "false" => vpw_vbscript::value::Value::Bool(false),
                                other => {
                                    vpw_vbscript::value::Value::Double(other.parse().unwrap_or(0.0))
                                }
                            };
                            (n, vec![v])
                        }
                        None => (name, Vec::new()),
                    };
                    println!("  calling {name} at {t} ms");
                    if let Err(e) = game.script_mut().call(name.trim(), &args) {
                        println!("  {name}: {e}");
                    }
                }
            }
        }
        // VPW_KEYS="Code@ms,Code@ms,..." presses arbitrary keys at arbitrary
        // times, held for a tenth of a second each — the way to walk a
        // machine's own service menu from here, which is the only honest way
        // to find out what a setting does.
        if let Ok(list) = std::env::var("VPW_KEYS") {
            for spec in list.split(',') {
                if let Some((code, at)) = spec.trim().split_once('@')
                    && let Ok(at) = at.trim().parse::<u32>()
                {
                    if t == at {
                        game.key(code.trim(), true);
                    }
                    if t == at + 100 {
                        game.key(code.trim(), false);
                    }
                }
            }
        }
        if let Some((x, y)) = drop {
            if t == 6_000 {
                let mut engine = game.engine.borrow_mut();
                engine.balls.clear();
                let z: f32 = std::env::var("VPW_DROP_Z")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(25.0);
                engine.add_ball(vpw_physics::ball::Ball::new(
                    vpw_math::Vec3::new(x, y, z),
                    25.0,
                ));
            }
            // Every sound the script asks for while the ball is being watched,
            // and the instant its vertical speed jumps: between them they say
            // whether a kicker took the ball and what threw it.
            if t >= 6_000 {
                for name in game.take_sounds() {
                    if name.to_ascii_lowercase().contains("kick") {
                        println!("  {:>5} ms  sound {name}", t - 6_000);
                    }
                }
                if let Some(b) = game.engine.borrow().balls.first()
                    && b.vel.z.abs() > 5.0
                    && !jumped
                {
                    jumped = true;
                    println!(
                        "  {:>5} ms  vertical jump: pos ({:.0},{:.0},{:.0}) vel ({:.1},{:.1},{:.1})",
                        t - 6_000,
                        b.pos.x,
                        b.pos.y,
                        b.pos.z,
                        b.vel.x,
                        b.vel.y,
                        b.vel.z
                    );
                }
            }
            if t >= 6_000
                && t % 250 == 0
                && let Some(b) = game.engine.borrow().balls.first()
            {
                trail.push((t - 6_000, b.pos, b.vel.length()));
            }
        }
        // With VPW_CALL in play, every ball is worth watching: the sub that
        // was called is usually one that makes or throws one.
        if std::env::var("VPW_CALL").is_ok() && (8_500..=9_200).contains(&t) && t % 50 == 0 {
            let engine = game.engine.borrow();
            if let Some(b) = engine.balls.last() {
                println!(
                    "  {t:>6} ms  last ball ({:.0},{:.0},{:.0}) vel {:?} locked {}",
                    b.pos.x, b.pos.y, b.pos.z, b.vel, b.locked
                );
            }
        }
        if std::env::var("VPW_CALL").is_ok() && t >= 6_000 && t % 250 == 0 {
            let engine = game.engine.borrow();
            if !engine.balls.is_empty() {
                let spots: Vec<String> = engine
                    .balls
                    .iter()
                    .map(|b| {
                        format!(
                            "({:.0},{:.0},{:.0} v{:.1})",
                            b.pos.x,
                            b.pos.y,
                            b.pos.z,
                            b.vel.length()
                        )
                    })
                    .collect();
                println!("  {t:>6} ms  balls: {}", spots.join(" "));
            }
        }
        game.step();
        if t % 17 == 16 {
            game.game_sync();
            game.new_frame();
        }
        if t % 16 == 0 {
            let m = game.machine();
            solenoids_ever |= m.solenoids_active();
            lit_now = 0;
            for n in 1..=128u8 {
                if m.lamp_lit(n) {
                    lamps_ever[usize::from(n) - 1] = true;
                    lit_now += 1;
                }
            }
        }
        if t % 16 == 0 {
            table_lit_now = 0;
            table_lights = 0;
            for item in game.items().iter() {
                if item.kind != vpw_game::items::Kind::Light {
                    continue;
                }
                table_lights += 1;
                if item.light_level() > 0.0 {
                    table_lit_now += 1;
                    ever.insert(item.name.to_string());
                }
            }
            table_lit_ever = ever.len();
        }
        // The mixer is where the board's audio ends up; ask it for the same
        // millisecond of sound a host would have asked for.
        let mut out = [0.0f32; 96];
        game.render_audio(&mut out);
        heard += out.len() / 2;
        for s in out {
            peak = peak.max(s.abs());
            energy += f64::from(s) * f64::from(s);
            wav.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        let quiet = out.iter().all(|s| s.abs() < 1.0e-4);
        if quiet {
            run_of_silence += 1;
        } else {
            loud += 1;
            if run_of_silence > 0 {
                silent_runs.push(run_of_silence);
                run_of_silence = 0;
            }
        }
    }

    // VPW_GREP=word prints every script line mentioning it, so a piece can be
    // traced from the file to what the table's own code does with it.
    if let Ok(word) = std::env::var("VPW_GREP") {
        let lower = word.to_ascii_lowercase();
        println!("script lines mentioning {word:?}:");
        for line in vpx.gamedata.code.string.lines() {
            if line.to_ascii_lowercase().contains(&lower) {
                println!("  {}", line.trim());
            }
        }
        println!();
    }
    // VPW_RAMP=name prints a ramp's path as the file has it and what the
    // physics built from it: the two things to compare when a ball falls
    // through one.
    if let Ok(name) = std::env::var("VPW_RAMP") {
        use vpin::vpx::gameitem::GameItemEnum as G;
        for item in &vpx.gameitems {
            let G::Ramp(r) = item else { continue };
            if !r.name.eq_ignore_ascii_case(&name) {
                continue;
            }
            println!(
                "ramp {}: type {:?}, z {:.0} -> {:.0}, width {:.0} -> {:.0}, collidable {}, \
                 physics mat {:?}, overwrite {:?}, elas {} fric {} scat {}",
                r.name,
                r.ramp_type,
                r.height_bottom,
                r.height_top,
                r.width_bottom,
                r.width_top,
                r.is_collidable,
                r.physics_material,
                r.overwrite_physics,
                r.elasticity,
                r.friction,
                r.scatter
            );
            println!("  drag points:");
            for (i, p) in r.drag_points.iter().enumerate() {
                println!(
                    "    {i:>2}: ({:>6.0}, {:>6.0}) smooth {}",
                    p.x, p.y, p.smooth
                );
            }
            match vpw_table::ramp::collision_path(r, 4.0) {
                Some(c) => {
                    println!("  collision path: {} steps", c.height.len());
                    for i in 0..c.height.len() {
                        println!(
                            "    {i:>2}: right ({:>6.0},{:>6.0}) left ({:>6.0},{:>6.0}) z {:>5.1}",
                            c.right[i].x, c.right[i].y, c.left[i].x, c.left[i].y, c.height[i]
                        );
                    }
                }
                None => println!("  collision path: NONE"),
            }
        }
        match game.items().get(&name) {
            Some(item) => {
                let engine = game.engine.borrow();
                let mut kinds = std::collections::BTreeMap::new();
                for &s in &item.shapes {
                    if let Some(sh) = engine.shapes().get(s) {
                        *kinds.entry(shape_kind(sh)).or_insert(0) += 1;
                    }
                }
                println!("  shapes owned: {kinds:?}");
            }
            None => println!("  (no item named {name} in the game)"),
        }
        println!();
    }
    if let Some((x, y)) = drop {
        // Everything whose footprint covers the drop point, so a lane a
        // player calls "a metal channel" gets its file name.
        use vpin::vpx::gameitem::GameItemEnum as G;
        println!("what the file has around ({x:.0}, {y:.0}):");
        for item in &vpx.gameitems {
            let (kind, near) = match item {
                G::Ramp(r) => {
                    let near = r
                        .drag_points
                        .iter()
                        .any(|p| (p.x - x).abs() < 80.0 && (p.y - y).abs() < 200.0);
                    if near {
                        println!(
                            "  Ramp       {:<14} type {:?} z {:.0}..{:.0} w {:.0}..{:.0} collidable {} visible {} points {}",
                            r.name,
                            r.ramp_type,
                            r.height_bottom,
                            r.height_top,
                            r.width_bottom,
                            r.width_top,
                            r.is_collidable,
                            r.is_visible,
                            r.drag_points.len()
                        );
                    }
                    continue;
                }
                G::Wall(w) => (
                    "Wall",
                    w.drag_points
                        .iter()
                        .any(|p| (p.x - x).abs() < 80.0 && (p.y - y).abs() < 200.0),
                ),
                G::Rubber(r) => (
                    "Rubber",
                    r.drag_points
                        .iter()
                        .any(|p| (p.x - x).abs() < 80.0 && (p.y - y).abs() < 200.0),
                ),
                G::Kicker(k) => (
                    "Kicker",
                    (k.center.x - x).abs() < 80.0 && (k.center.y - y).abs() < 200.0,
                ),
                G::Gate(g) => (
                    "Gate",
                    (g.center.x - x).abs() < 80.0 && (g.center.y - y).abs() < 200.0,
                ),
                G::Trigger(t) => (
                    "Trigger",
                    (t.center.x - x).abs() < 80.0 && (t.center.y - y).abs() < 200.0,
                ),
                G::Primitive(p) => (
                    "Primitive",
                    (p.position.x - x).abs() < 80.0 && (p.position.y - y).abs() < 200.0,
                ),
                _ => continue,
            };
            if near {
                println!("  {kind:<10} {}", item.name());
            }
        }
        println!();
    }
    if !trail.is_empty() {
        println!("the dropped ball, every quarter second:");
        for (ms, p, v) in &trail {
            println!(
                "  {ms:>5} ms  at ({:>6.0}, {:>6.0}, {:>4.0})  speed {v:>5.1}",
                p.x, p.y, p.z
            );
        }
        // What it ended up touching, so the shape has a name.
        if let Some(b) = game.engine.borrow().balls.first() {
            let engine = game.engine.borrow();
            let mut near: Vec<(f32, String)> = Vec::new();
            for (i, shape) in engine.shapes().iter().enumerate() {
                let Some(bb) = shape.bbox() else { continue };
                let dx = (b.pos.x - b.pos.x.clamp(bb.min.x, bb.max.x)).abs();
                let dy = (b.pos.y - b.pos.y.clamp(bb.min.y, bb.max.y)).abs();
                let d = (dx * dx + dy * dy).sqrt();
                if d < 30.0 {
                    let owner = game
                        .items()
                        .by_shape(i)
                        .map(|it| format!("{} ({:?})", it.name, it.kind))
                        .unwrap_or_else(|| "?".into());
                    near.push((d, format!("{owner} shape#{i} {}", shape_kind(shape))));
                }
            }
            near.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            println!("  resting against:");
            for (d, what) in near.iter().take(8) {
                println!("    {d:>5.1} away: {what}");
            }
        }
        println!();
    }
    println!("after {seconds}s of table time:");
    println!("  handlers fired  {}", game.handlers_fired());
    println!("  switches closed {:016x}", game.machine().switch_matrix());
    let closed: Vec<u8> = (1..=64u8)
        .filter(|&n| game.machine().switch_closed(n))
        .collect();
    println!("  which are       {closed:?}");
    println!(
        "  lamps ever lit  {} of 128, {lit_now} lit at the end",
        lamps_ever.iter().filter(|&&b| b).count()
    );
    println!(
        "  table lights    {table_lit_ever} of {table_lights} ever on, {table_lit_now} on at the end"
    );
    println!("  solenoids ever  {solenoids_ever:032b}");
    println!(
        "  solenoids now   {:032b}",
        game.machine().solenoids_active()
    );
    println!("  sound latch     {:02x}", game.machine().sound_latch());
    println!(
        "  audio           {loud} of {} ms had sound in them",
        seconds * 1000
    );
    silent_runs.sort_unstable();
    if let Some(longest) = silent_runs.last() {
        println!("  longest gap     {longest} ms");
    }
    println!("  frames rendered {heard} sample frames asked for");
    println!(
        "  loudest sample  {peak:.4} of 1.0, rms {:.4}",
        (energy / (heard.max(1) * 2) as f64).sqrt()
    );
    write_wav("table.wav", &wav, vpw_game::AUDIO_RATE);
    println!("  wrote table.wav");

    // What the game has lit, by name, for a renderer to photograph. Numbers
    // about lamps have been agreeing with each other all day while the picture
    // stayed dark, so the picture is the thing to look at.
    // VPW_DUMP_GROUPS=<file> keeps the machine running for another thirty
    // seconds and writes which field-scale lamps it switched together, one
    // group per line, tab-separated: the machine's own answer to the bake's
    // grouping question, for photographing next to the guessed one.
    if let Ok(path) = std::env::var("VPW_DUMP_GROUPS") {
        // A scene of its own: the game took the moving parts out of the
        // first one, and the candidate list wants the lights untouched.
        let fresh = vpw_table::geometry::extract(&vpx);
        let candidates = vpw_render_candidates(&fresh);
        let groups = vpw_game::grouping::observe_lamp_groups(&mut game, &candidates, 30.0);
        let mut out = String::new();
        for group in &groups {
            out.push_str(&group.join("\t"));
            out.push('\n');
        }
        std::fs::write(&path, out).expect("could not write the groups");
        println!("  wrote {path} ({} groups)", groups.len());
    }

    if let Ok(path) = std::env::var("VPW_DUMP_LIGHTS") {
        let mut out = String::new();
        for item in game.items().iter() {
            if item.kind == vpw_game::items::Kind::Light {
                out.push_str(&format!("{}\t{:.4}\n", item.name, item.light_level()));
            }
        }
        std::fs::write(&path, out).expect("could not write the light dump");
        println!("  wrote {path}");
    }

    let (dots, w, h) = game.machine().dmd();
    if !dots.is_empty() {
        println!("\nthe display says:");
        for y in 0..h {
            let row: String = (0..w)
                .map(|x| match dots[y * w + x] {
                    0 => ' ',
                    1 => '.',
                    2 => '+',
                    _ => '#',
                })
                .collect();
            println!("  |{}|", row.trim_end());
        }
    }
}
