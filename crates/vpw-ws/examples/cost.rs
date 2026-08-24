//! What the sound board costs, measured against a control.
//!
//!     cargo run --release -p vpw-ws --example cost -- lotr.zip [seconds]
//!
//! The machine is built twice — once with the sound board's images and once
//! without — and each is run for the same amount of board time, interleaved,
//! with a fixed arithmetic workload before and after. The control is not
//! decoration: this phone's clock moves by a factor of several depending on
//! what else is happening, and a number taken without one says more about the
//! governor than about the code.

use std::collections::BTreeMap;
use std::io::Read;
use std::time::Instant;

use vpw_ws::Whitestar;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: cost <rom.zip> [seconds]");
    let seconds: f64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4.0);
    let roms = read_zip(&path);
    let set = vpw_ws::games::find("lotr").expect("a set this knows");

    let build = |with_sound: bool| {
        let mut machine = Whitestar::new();
        machine
            .load_rom(find(&roms, set.cpu).expect("the cpu image"))
            .expect("it fits");
        if let Some(d) = find(&roms, set.display) {
            machine.load_display_rom(d).expect("it fits");
        }
        if with_sound {
            let s = set.sound;
            if let (Some(b), Some(u7), [Some(a), Some(bb), Some(c), Some(d)]) = (
                find(&roms, s.bios),
                find(&roms, s.u7),
                s.samples.map(|n| find(&roms, n)),
            ) {
                machine.load_sound(b, u7, [a, bb, c, d]);
            }
        }
        machine.board.switches.set_dedicated(0, false);
        machine
    };

    let steps = (seconds * f64::from(vpw_ws::CPU_CLOCK_HZ)) as u64;
    let run = |machine: &mut Whitestar| {
        let mut spent = 0u64;
        let at = Instant::now();
        while spent < steps {
            spent += u64::from(machine.step());
            machine.take_audio();
        }
        at.elapsed().as_secs_f64()
    };

    println!("control before   {:.3}s", control());
    let mut quiet = build(false);
    let mut loud = build(true);
    // Interleaved, so that a clock that changes changes both.
    let (mut a, mut b) = (0.0, 0.0);
    for _ in 0..2 {
        a += run(&mut quiet);
        b += run(&mut loud);
    }
    println!("control after    {:.3}s", control());
    let total = seconds * 2.0;
    println!();
    println!("{total}s of board time each:");
    println!("  without sound  {a:.2}s  ({:.2}x real time)", total / a);
    println!("  with sound     {b:.2}s  ({:.2}x real time)", total / b);
    println!("  the board costs {:.0}% more", (b / a - 1.0) * 100.0);
}

/// A fixed amount of arithmetic, to say how fast this machine is right now.
fn control() -> f64 {
    let at = Instant::now();
    let mut acc = 0u64;
    for i in 0..40_000_000u64 {
        acc = acc.wrapping_mul(6364136223846793005).wrapping_add(i);
    }
    std::hint::black_box(acc);
    at.elapsed().as_secs_f64()
}

fn find<'a>(roms: &'a BTreeMap<String, Vec<u8>>, want: &str) -> Option<&'a [u8]> {
    roms.iter()
        .find(|(name, _)| {
            name.to_ascii_lowercase()
                .contains(&want.to_ascii_lowercase())
        })
        .map(|(_, data)| data.as_slice())
}

fn read_zip(path: &str) -> BTreeMap<String, Vec<u8>> {
    let file = std::fs::File::open(path).expect("could not open the zip");
    let mut zip = zip::ZipArchive::new(file).expect("not a zip");
    let mut out = BTreeMap::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).expect("entry");
        if entry.is_dir() {
            continue;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data).expect("decompress");
        out.insert(entry.name().to_string(), data);
    }
    out
}
