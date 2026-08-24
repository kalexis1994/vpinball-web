//! Boots the real sound firmware and says how far it gets.
//!
//!     cargo run --release -p vpw-at91 --example boot -- lotr.zip [seconds]
//!
//! The two things worth watching are the **remap** and the **samples**. The
//! board boots out of the BIOS, builds its real program in the RAM at
//! `$300000`, and asks the bus interface to swap the two: reaching that is the
//! proof that the processor executed a few hundred thousand instructions
//! correctly, because the copy is a loop and the remap is the instruction
//! after it. Producing samples is the proof that it is doing its job.

use std::collections::BTreeMap;
use std::io::Read;

use vpw_arm7::{Bus, Cpu};
use vpw_at91::Sound;

/// Where the processor was, what it was about to run, and how it stood: enough
/// to read a trail backwards and see what jumped where it should not have.
type Step = (u32, u32, u32);

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: boot <rom.zip> [seconds]");
    let seconds: f64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let roms = read_zip(&path);
    for (name, data) in &roms {
        println!("  {name:<20} {:>9} bytes", data.len());
    }

    let mut board = Sound::new();
    let want = |part: &str| -> &[u8] {
        roms.iter()
            .find(|(n, _)| n.to_ascii_lowercase().contains(part))
            .map(|(_, d)| d.as_slice())
            .unwrap_or_else(|| panic!("no image matching {part:?}"))
    };
    board.load_bios(want("bios.u8"));
    board.load_u7(want("u7"));
    for (i, part) in ["u17", "u21", "u36", "u37"].iter().enumerate() {
        board.load_samples(i, want(part));
    }

    let mut cpu = Cpu::new();
    cpu.reset();
    println!();
    println!("starting at {:08x}", cpu.r[15]);

    let target = (seconds * f64::from(vpw_at91::board::CLOCK_HZ)) as u64;
    let mut cycles = 0u64;
    let mut remapped_at = None;
    let mut pcs = std::collections::BTreeSet::new();
    let mut unmapped: BTreeMap<u32, u32> = BTreeMap::new();
    let mut instructions = 0u64;
    let mut hot_pc: BTreeMap<u32, u64> = BTreeMap::new();
    let mut delay_calls: Vec<(u64, u32)> = Vec::new();
    let mut poll_pc: BTreeMap<u32, u64> = BTreeMap::new();
    // The last few addresses, so that a jump into nowhere can be traced back
    // to the instruction that made it.
    let mut trail = std::collections::VecDeque::with_capacity(33);
    let mut derailed: Option<(u64, Vec<Step>)> = None;
    let mut polled: BTreeMap<u32, u64> = BTreeMap::new();
    let mut irqs_taken = 0u64;
    let mut irq_asserted = 0u64;
    let mut was_irq = false;
    let mut entries = 0u64;
    let mut was_mode = cpu.mode();

    // A command, once the board has settled. Silence is what a sound board
    // produces when nobody has asked it for anything, so a peak of zero proves
    // nothing until something has been asked for.
    let ask_at = target / 3;
    let mut asked = false;
    let mut peak = 0i32;
    let mut samples = 0u64;

    while cycles < target {
        if !asked && cycles > ask_at {
            board.send(0x03);
            asked = true;
            println!("sent a sound command at {cycles} cycles");
        }
        let before = cpu.r[15];
        let was = board.reads[0] + board.reads[2];
        let n = cpu.step(&mut board);
        if board.reads[0] + board.reads[2] != was {
            *poll_pc.entry(before).or_insert(0) += 1;
        }
        cycles += u64::from(n);
        instructions += 1;
        if derailed.is_none() {
            if trail.len() == 32 {
                trail.pop_front();
            }
            let pc = cpu.r[15];
            let word = board.read32(pc);
            trail.push_back((pc, word, cpu.cpsr));
            // The words just past the vector table are nowhere any program
            // means to be; reaching them is a jump into nothing. The vectors
            // themselves are excluded because taking one is the normal way in.
            // A word of zero is `andeq r0, r0, r0`: legal, harmless, and never
            // what a compiler emits. Executing one means the processor is in
            // data, which is unambiguous in a way an address range is not.
            if board.remapped && word == 0 {
                derailed = Some((cycles, trail.iter().copied().collect()));
            }
        }
        instructions += 0;
        // The entry to the delay routine: r1 is the count it was asked for.
        if cpu.r[15] == 0x29ec {
            delay_calls.push((cycles, cpu.r[1]));
        }
        if cycles > target / 2 {
            *hot_pc.entry(cpu.r[15]).or_insert(0) += 1;
            if let Some(addr) = board.last_read.take() {
                *polled.entry(addr).or_insert(0) += 1;
            }
        }
        pcs.insert(cpu.r[15] & !0xfff);
        if cpu.mode() != was_mode {
            if cpu.mode() == vpw_arm7::Mode::Irq {
                entries += 1;
            }
            was_mode = cpu.mode();
        }
        let now = board.tick(n);
        if now && !was_irq {
            irq_asserted += 1;
        }
        was_irq = now;
        // Taking it is visible as the processor arriving at the vector.
        if now && cpu.cpsr & (1 << 7) == 0 {
            irqs_taken += 1;
        }
        cpu.irq = now;
        if let Some((addr, _)) = board.last_unmapped.take() {
            *unmapped.entry(addr).or_insert(0) += 1;
        }
        for (l, r) in board.take_audio() {
            samples += 1;
            peak = peak.max(i32::from(l).abs()).max(i32::from(r).abs());
        }
        if board.remapped && remapped_at.is_none() {
            remapped_at = Some(cycles);
            println!("remapped after {cycles} cycles ({instructions} instructions)");
        }
    }

    println!();
    println!("after {seconds}s of board time:");
    println!("  instructions   {instructions}");
    println!("  pc             {:08x}", cpu.r[15]);
    println!("  pages visited  {}", pcs.len());
    println!("  remapped       {}", board.remapped);
    println!("  samples made   {samples}");
    println!("  loudest sample {peak} of 32767");
    println!(
        "  command reads  {} bytes, {} words",
        board.reads[0], board.reads[2]
    );
    println!("delay routine entered {} times:", delay_calls.len());
    for (at, count) in delay_calls.iter().take(12) {
        println!(
            "    at {:>12} cycles ({:>6.3}s) counting {count} (~{:.3}s of spinning)",
            at,
            *at as f64 / f64::from(vpw_at91::board::CLOCK_HZ),
            f64::from(*count) / 2.0 * 5.0 / f64::from(vpw_at91::board::CLOCK_HZ),
        );
    }
    match &derailed {
        Some((at, trail)) => {
            println!(
                "ran off the rails at {:.4}s; the last 32 instructions:",
                *at as f64 / f64::from(vpw_at91::board::CLOCK_HZ)
            );
            for (pc, word, cpsr) in trail {
                println!("    {pc:08x}  {word:08x}   cpsr {cpsr:08x}");
            }
        }
        None => println!("never left its program"),
    }
    top("who reads the command port", &poll_pc, 4);
    if let Some((&a, _)) = poll_pc.iter().max_by_key(|&(_, c)| *c) {
        dump(&mut board, a.saturating_sub(32), 20);
    }
    top("busiest instructions (second half)", &hot_pc, 14);
    let busiest = hot_pc.iter().max_by_key(|&(_, c)| *c).map(|(&a, _)| a);
    if let Some(a) = busiest {
        dump(&mut board, a.saturating_sub(24), 12);
    }
    dump(&mut board, 0, 16);
    top("busiest reads (second half)", &polled, 14);
    println!(
        "  sample reads   {} bytes, {} words",
        board.reads[1], board.reads[3]
    );
    println!(
        "  timers         {:?}",
        board
            .timers
            .iter()
            .map(|t| (t.running, t.rc))
            .collect::<Vec<_>>()
    );
    println!("  irq enabled    {:08x}", board.aic.enabled);
    println!("  irq asserted   {irq_asserted} times");
    println!("  irq unmasked   {irqs_taken} of those");
    println!("  entered irq    {entries} times");
    println!(
        "  timer0 mode    {:08x}  counter {}",
        board.timers[0].mode, board.timers[0].counter
    );
    println!("  timer0 mask    {:08x}", board.timers[0].irq_mask);
    println!("  aic pending    {:08x}", board.aic.pending);
    println!(
        "  aic vectors    {:08x} {:08x}",
        board.aic.vectors[4], board.aic.spurious_vector
    );
    println!("  cpsr           {:08x}", cpu.cpsr);
    if unmapped.is_empty() {
        println!("  every address it touched is on the map");
    } else {
        println!("  addresses off the map:");
        for (addr, n) in unmapped.iter().take(8) {
            println!("    {addr:08x}  {n}");
        }
    }
}

/// The words around an address, for reading by eye.
fn dump(board: &mut Sound, from: u32, n: u32) {
    println!("words at {from:08x}:");
    for i in 0..n {
        let a = from + i * 4;
        println!("    {a:08x}  {:08x}", board.read32(a));
    }
}

fn top(what: &str, map: &BTreeMap<u32, u64>, n: usize) {
    let mut v: Vec<_> = map.iter().map(|(&a, &c)| (a, c)).collect();
    v.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
    println!("{what}:");
    for (addr, count) in v.into_iter().take(n) {
        println!("    {addr:08x}  {count:>10}");
    }
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
