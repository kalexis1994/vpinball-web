//! Boots a System 80 ROM set and says what the machine did.
//!
//! ```text
//! cargo run --release -p vpw-gts80 --example boot -- path/to/icefever/ [seconds]
//! ```
//!
//! This is the test that settles the 6502 and the 6532 next door. Sixteen unit
//! tests and nine cannot prove a processor right; a ROM that boots, sets its
//! interrupt going, walks all sixteen display positions and starts strobing
//! the switch matrix has exercised more of both in a second than any test
//! anybody would write by hand.

use std::collections::HashMap;

use vpw_gts80::Board;

/// The clock: 3.579545 MHz divided by four (`gts80.c:708`).
const CLOCK: u32 = 894_886;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: boot <rom-directory> [seconds]");
    let seconds: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2.0);

    let roms = read_roms(&path);
    println!("{path}");
    let mut names: Vec<&String> = roms.keys().collect();
    names.sort();
    for name in &names {
        println!("  {name:<16} {} bytes", roms[*name].len());
    }

    let (game, u2, u3) = pick(&roms);
    println!(
        "\ngame {} bytes, system {} + {} bytes",
        game.len(),
        u2.len(),
        u3.len()
    );

    let mut board = Board::new(game, u2, u3);
    println!("reset to ${:04X}", board.cpu.pc);

    // Somewhere for the ROM to find its switches: everything open is a machine
    // sitting on the floor with nobody at it.
    board.mem.io.dips = [0x00; 4];

    let clocks = (CLOCK as f64 * seconds) as u32;
    let mut seen_pc: HashMap<u16, u64> = HashMap::new();
    let mut interrupts = 0u64;
    let mut strobes = 0u64;
    let mut last_strobe = 0u8;
    let mut spent = 0u32;
    let mut in_irq = false;

    while spent < clocks {
        let asking = board.mem.irq();
        if asking && !in_irq {
            interrupts += 1;
        }
        in_irq = asking;

        let strobe = board.mem.riot[0].port_b_output();
        if strobe != last_strobe {
            strobes += 1;
            last_strobe = strobe;
        }

        *seen_pc.entry(board.cpu.pc >> 8).or_default() += 1;
        spent += board.step();

        if let Some((at, op)) = board.cpu.last_illegal.take() {
            println!("\n!! unknown opcode ${op:02X} at ${at:04X} after {spent} clocks");
            break;
        }
    }

    println!(
        "\nafter {:.2} s of machine time:",
        f64::from(spent) / f64::from(CLOCK)
    );
    println!("  interrupts   {interrupts}");
    println!("  switch strobes {strobes}");

    let lit = (1..=48).filter(|n| board.mem.io.lamp_lit(*n)).count();
    println!("  lamps lit    {lit}");
    println!(
        "  relays       game-on {}, tilt {}",
        board.mem.io.game_on(),
        board.mem.io.tilted()
    );
    println!("  solenoids    {:#011b}", board.mem.io.solenoids);
    println!(
        "  sound        last command {:#04X}",
        board.mem.io.sound_command
    );

    let written = board
        .mem
        .displays
        .digits
        .iter()
        .flatten()
        .filter(|d| **d != 0)
        .count();
    println!("  display      {written} of 48 positions lit");
    for (i, bank) in board.mem.displays.digits.iter().enumerate() {
        let text: String = bank.iter().map(|d| glyph(*d)).collect();
        println!("    bank {i}      [{text}]");
    }

    // Where the time went, which is the quickest way to see a machine stuck in
    // a loop rather than running a game.
    let mut pages: Vec<(u16, u64)> = seen_pc.into_iter().collect();
    pages.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    print!("  time in      ");
    for (page, n) in pages.iter().take(5) {
        print!("${page:02X}xx {n}  ");
    }
    println!();
}

/// A rough glyph for a set of segments, so a display can be read at a glance.
fn glyph(segments: u8) -> char {
    const DIGITS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];
    match DIGITS.iter().position(|s| *s == segments) {
        Some(d) => char::from(b'0' + d as u8),
        None if segments == 0 => ' ',
        None => '?',
    }
}

/// Picks the three ROMs out of a set by their sizes and names.
///
/// A System 80 set is a 2 KB game ROM and two 4 KB system ROMs, and the
/// system pair is shared by every game of a generation — which is why they are
/// named for the generation (`u2_80a.bin`) and the game ROM for the game's own
/// number (`695.cpu`).
fn pick(roms: &HashMap<String, Vec<u8>>) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let find = |want: &dyn Fn(&str, &[u8]) -> bool| -> Option<Vec<u8>> {
        let mut names: Vec<&String> = roms.keys().collect();
        names.sort();
        names
            .into_iter()
            .find(|n| want(&n.to_lowercase(), &roms[*n]))
            .map(|n| roms[n].clone())
    };

    let game = find(&|n: &str, b: &[u8]| b.len() == 2048 && !n.contains("snd"))
        .expect("no 2 KB game ROM in the set");
    let u2 = find(&|n: &str, b: &[u8]| b.len() == 4096 && n.contains("u2"))
        .or_else(|| find(&|_, b: &[u8]| b.len() == 4096))
        .expect("no 4 KB system ROM");
    let u3 =
        find(&|n: &str, b: &[u8]| b.len() == 4096 && n.contains("u3")).expect("no U3 system ROM");
    (game, u2, u3)
}

/// Reads a ROM set out of a directory.
///
/// A directory and not a zip: the zip reading — with the inflater a real set
/// needs — already exists in `vpw-game`, where the machine is wired up, and a
/// second copy of it here would be a second copy to keep right. Unpack the set
/// and point this at the folder.
fn read_roms(path: &str) -> HashMap<String, Vec<u8>> {
    let mut out = HashMap::new();
    let dir = std::fs::read_dir(path).expect("could not open the ROM directory");
    for entry in dir.flatten() {
        let p = entry.path();
        if p.is_file()
            && let Ok(bytes) = std::fs::read(&p)
        {
            out.insert(p.file_name().unwrap().to_string_lossy().into_owned(), bytes);
        }
    }
    out
}
