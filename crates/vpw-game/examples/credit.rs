//! Puts a coin in a table and presses start, and says what the machine did.
//!
//!     cargo run --release -p vpw-game --example credit -- table.vpx scripts roms [seconds]
//!
//! The one thing that says a board is really running a game rather than merely
//! executing: a coin has to become a credit, start has to begin a ball, and the
//! machine has to serve one out of the trough on its own.

use std::rc::Rc;

use vpw_game::controller::{RomDir, RomSource};
use vpw_game::{Game, LibraryDir, Resources, ScriptLibrary};

fn tick(game: &mut Game, t: u32) {
    game.step();
    if t % 17 == 16 {
        game.game_sync();
        game.new_frame();
    }
}

fn run(game: &mut Game, ms: u32, from: u32) -> u32 {
    for i in 0..ms {
        tick(game, from + i);
    }
    from + ms
}

fn logging() {
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
    log::set_max_level(log::LevelFilter::Warn);
}

fn main() {
    logging();
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: credit <table.vpx> <scripts> <roms>");
    let scripts = args.next().expect("a scripts directory");
    let roms = args.next().expect("a roms directory");

    let bytes = std::fs::read(&path).expect("could not read the table");
    let vpx = vpin::vpx::from_bytes(&bytes).expect("could not parse the .vpx");
    let mut scene = vpw_table::geometry::extract(&vpx);
    let libraries: Rc<dyn ScriptLibrary> = Rc::new(LibraryDir(scripts.into()));
    let source: Rc<dyn RomSource> = Rc::new(RomDir(roms.into()));
    let mut game = Game::load(
        &vpx,
        &mut scene,
        Resources::new(libraries).with_roms(source),
    )
    .expect("the table should load");
    game.start().expect("the script should start");

    println!("rom running   {}", game.machine().is_running());
    let troughs = |g: &Game| {
        (11..=14u8)
            .map(|n| {
                if g.machine().switch_closed(n) {
                    '1'
                } else {
                    '0'
                }
            })
            .collect::<String>()
    };
    println!("game name     {:?}", game.machine().game_name());

    let lamps = |g: &Game| (1..=128u8).filter(|&n| g.machine().lamp_lit(n)).count();

    let mut t = run(&mut game, 4000, 0);
    println!();
    println!("after four seconds of attract:");
    println!("  lamps lit   {}", lamps(&game));
    println!("  solenoids   {:032b}", game.machine().solenoids_active());

    // A coin, then start. `core.vbs` binds both to the number row.
    for (key, label) in [("Digit5", "coin"), ("Digit1", "start")] {
        game.key(key, true);
        t = run(&mut game, 60, t);
        game.key(key, false);
        t = run(&mut game, 1500, t);
        println!();
        println!("after {label}:");
        println!("  lamps lit   {}", lamps(&game));
        println!("  solenoids   {:032b}", game.machine().solenoids_active());
        println!("  sounds      {}", game.take_sounds().len());
        println!("  trough      {}", troughs(&game));
        println!("  coil 1      {}", game.machine().solenoid_fired(1));
    }

    // Watch a while: serving a ball is a coil pulse and a pulse is easy to miss.
    let mut ever = 0u32;
    let mut most = 0usize;
    for _ in 0..60 {
        t = run(&mut game, 100, t);
        ever |= game.machine().solenoids_active();
        most = most.max(lamps(&game));
    }
    println!();
    println!("over the next six seconds:");
    println!("  coils ever  {ever:032b}");
    println!("  most lamps  {most}");
    println!("  balls       {}", game.engine.borrow().balls.len());
    for m in game.take_messages() {
        println!("  says: {m}");
    }
}
