//! Works the coin door of a System 80 and prints what the display says.
//!
//! ```text
//! cargo run -p vpw-gts80 --example coindoor -- <rom directory> [trough...]
//! ```
//!
//! The one thing a machine will not tell you from the outside is *why* it is
//! ignoring you. The three coin chutes are set separately, a board with an
//! empty memory does not agree with itself about what they cost, and the test
//! button walks a menu that changes both. This puts a coin in each chute in
//! turn and reads the glass after every one, which settles it in a few
//! seconds.

use std::path::PathBuf;

use vpw_gts80::Board;

const RATE: u32 = 48_000;
/// `sys80.vbs:23`: the test button and the three chutes, in order.
const TEST: i32 = 7;
const CHUTES: [i32; 3] = [17, 27, 37];
const START: i32 = 47;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("a rom directory"));
    let trough: Vec<i32> = args.filter_map(|a| a.parse().ok()).collect();

    let mut images = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("the directory will not open") {
        let path = entry.expect("a directory entry").path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        images.push((name, std::fs::read(&path).expect("a file will not open")));
    }
    let set = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let roms = vpw_gts80::games::detect(&set, &images).expect("not a System 80 set");

    // With no trough given, go and find it: a machine will not start a game
    // without a ball where it can see one, and which switch that is depends on
    // the table rather than the board.
    if trough.is_empty() {
        println!("looking for the trough switch");
        for candidate in 0..80 {
            let mut board = Board::from_roms(&roms, RATE);
            board.run_seconds(2.5);
            board.mem.io.set_switch(candidate, true);
            board.run_seconds(0.5);
            for _ in 0..3 {
                board.mem.io.set_switch(CHUTES[0], true);
                board.run_seconds(0.15);
                board.mem.io.set_switch(CHUTES[0], false);
                board.run_seconds(0.3);
            }
            board.mem.io.set_switch(START, true);
            board.run_seconds(0.3);
            board.mem.io.set_switch(START, false);
            let mut on = false;
            for _ in 0..20 {
                board.run_seconds(0.1);
                on |= board.mem.io.solenoid_on(10);
            }
            if on {
                let (_, bottom) = board.text();
                println!(
                    "  switch {candidate:2} starts a game: [{}]",
                    bottom.trim_end()
                );
            }
        }
        return;
    }

    let mut board = Board::from_roms(&roms, RATE);

    let show = |board: &Board, what: &str| {
        let (top, bottom) = board.text();
        println!(
            "{what:<14} [{}] [{}]  relays {:02b}",
            top.trim_end(),
            bottom.trim_end(),
            board.mem.io.lamps[0] & 3
        );
    };
    let coils = |board: &Board| {
        let s = board.mem.io.solenoids();
        (0..11)
            .filter(|i| s & (1 << i) != 0)
            .map(|i| (i + 1).to_string())
            .collect::<Vec<_>>()
            .join(",")
    };

    board.run_seconds(3.0);
    for &switch in &trough {
        board.mem.io.set_switch(switch, true);
    }
    board.run_seconds(1.0);
    show(&board, "attract");

    // Each chute in turn, three coins apiece. A board with nothing in its
    // memory charges differently on each, and one of the three is usually a
    // coin a credit.
    for chute in CHUTES {
        for n in 1..=3 {
            board.mem.io.set_switch(chute, true);
            board.run_seconds(0.2);
            board.mem.io.set_switch(chute, false);
            board.run_seconds(0.4);
            show(&board, &format!("chute {chute}, {n}"));
        }
    }

    board.mem.io.set_switch(START, true);
    board.run_seconds(0.3);
    board.mem.io.set_switch(START, false);
    for i in 0..8 {
        board.run_seconds(0.5);
        let fired = coils(&board);
        show(&board, &format!("start +{:.1}s", f64::from(i) * 0.5));
        if !fired.is_empty() {
            println!("               coils {fired}");
        }
    }
    println!(
        "game on: {}   tilt: {}",
        board.mem.io.solenoid_on(10),
        board.mem.io.solenoid_on(11)
    );

    // And the test button, in case the machine wants setting up before it will
    // take anybody's money.
    for n in 1..=4 {
        board.mem.io.set_switch(TEST, true);
        board.run_seconds(0.2);
        board.mem.io.set_switch(TEST, false);
        board.run_seconds(0.6);
        show(&board, &format!("test {n}"));
    }
}
