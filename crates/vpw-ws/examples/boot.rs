//! Boots a real Whitestar ROM and says how far it gets.
//!
//!     cargo run --release -p vpw-ws --example boot -- lotr.zip [seconds]
//!
//! What it is looking for is not "no crash" — a 6809 running on garbage does
//! not crash, it runs garbage. It is looking for the things a board that is
//! actually executing its game does: the ROM bank moving, the switch matrix
//! being strobed through all eight columns, the lamp columns being swept, and
//! the watchdog-adjacent habit of writing to the same few ports for ever.

use std::collections::BTreeMap;
use std::io::Read;

use vpw_ws::Whitestar;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: boot <rom.zip> [seconds]");
    let seconds: f64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2.0);

    let roms = read_zip(&path);
    println!("=== {path} ===");
    for (name, data) in &roms {
        println!("  {name:<20} {:>9} bytes", data.len());
    }
    println!();

    let set = vpw_ws::games::find("lotr").expect("lotr should be a set this knows");
    let cpu = find(&roms, set.cpu).unwrap_or_else(|| panic!("no image matching {:?}", set.cpu));
    println!(
        "loading {} ({} KB) as the CPU image",
        set.cpu,
        cpu.len() / 1024
    );

    let mut machine = Whitestar::new();
    machine.load_rom(cpu).expect("the image fits the region");
    if let Some(display) = find(&roms, set.display) {
        machine
            .load_display_rom(display)
            .expect("the display image fits");
        println!(
            "loading {} ({} KB) as the display image",
            set.display,
            display.len() / 1024
        );
    }
    // The sound board, if the zip has all five of its images.
    let sound = set.sound;
    match (
        find(&roms, sound.bios),
        find(&roms, sound.u7),
        sound.samples.map(|n| find(&roms, n)),
    ) {
        (Some(bios), Some(u7), [Some(a), Some(b), Some(c), Some(d)]) => {
            machine.load_sound(bios, u7, [a, b, c, d]);
            println!("loading the sound board's five images");
        }
        _ => println!("the sound board's images are not all here; running silent"),
    }
    println!("reset vector -> pc {:04x}", machine.cpu.pc);
    println!();

    // The dedicated byte at `$3000` is the three coin-door buttons and the
    // flippers, and there is no door interlock in it at all. The flippers come
    // from the switch matrix's column eleven rather than from this byte
    // (`se.c:648`), so there is nothing to open here — the line is left as a
    // marker of where a probe would press a button, not as a thing that does
    // anything.
    machine.board.switches.set_dedicated(0, false);
    // Balls in the trough, given as switch numbers. A machine that cannot see
    // any spends its life searching for them and lights nothing.
    if let Some(list) = std::env::args().nth(3) {
        for number in list.split(',').filter_map(|n| n.trim().parse::<u8>().ok()) {
            machine.board.switches.set(number, true);
            println!("holding switch {number} closed");
        }
    }

    let mut banks = BTreeMap::new();
    let mut columns = 0u8;
    let mut lamp_columns = 0u16;
    let mut lamp_rows = 0u8;
    let mut most_lit = 0usize;
    let mut unmapped: BTreeMap<u16, u32> = BTreeMap::new();
    let mut pcs = std::collections::BTreeSet::new();
    let mut hot: BTreeMap<u16, u64> = BTreeMap::new();
    let mut resets = 0u32;
    let entry = machine.cpu.pc;
    let (mut sound_writes, mut last_sound) = (0u32, 0u8);
    let mut sols = 0u32;
    let steps = (seconds * f64::from(vpw_ws::CPU_CLOCK_HZ)) as u64;
    let mut spent = 0u64;
    let (mut heard, mut peak) = (0u64, 0i32);
    let mut wav: Vec<u8> = Vec::new();
    let started = std::time::Instant::now();
    while spent < steps {
        spent += u64::from(machine.step());
        for (l, r) in machine.take_audio() {
            heard += 1;
            peak = peak.max(i32::from(l).abs()).max(i32::from(r).abs());
            wav.extend_from_slice(&l.to_le_bytes());
            wav.extend_from_slice(&r.to_le_bytes());
        }
        *banks.entry(machine.board.bank()).or_insert(0u32) += 1;
        pcs.insert(machine.cpu.pc);
        if machine.cpu.pc == entry {
            resets += 1;
        }
        *hot.entry(machine.cpu.pc & 0xff00).or_insert(0u64) += 1;
        columns |= machine.board.switches.column();
        lamp_columns |= machine.board.lamps.column();
        lamp_rows |= machine.board.lamps.rows();
        let lit = (1..=128u8)
            .filter(|&n| machine.board.lamps.is_lit(n))
            .count();
        most_lit = most_lit.max(lit);
        sols |= machine.board.solenoids.live();
        if machine.board.sound_latch != last_sound {
            last_sound = machine.board.sound_latch;
            sound_writes += 1;
        }
        if let Some((addr, _)) = machine.board.last_unmapped.take() {
            *unmapped.entry(addr).or_insert(0) += 1;
        }
    }

    let real = started.elapsed().as_secs_f64();
    println!(
        "after {seconds}s of board time ({real:.1}s of ours, {:.2}x real):",
        seconds / real
    );
    if let Some(s) = &machine.sound {
        let (taken, reads, enabled, pc) = s.diagnostics();
        println!("  sound board     pc {pc:08x}, aic enabled {enabled:08x}");
        println!("    fast irqs     {taken}");
        println!("    command reads {} bytes, {} words", reads[0], reads[2]);
        println!("    sample reads  {} bytes, {} words", reads[1], reads[3]);
    }
    if !wav.is_empty() {
        // The board's own rate, which is what its timer was set to.
        let rate = (heard as f64 / seconds).round() as u32;
        write_wav("sound.wav", &wav, rate);
        println!("  wrote sound.wav  {} frames at {rate} Hz", heard);
    }
    println!("  samples heard   {heard}, loudest {peak} of 32767");
    println!("  pc              {:04x}", machine.cpu.pc);
    println!("  firqs raised    {}", machine.firqs);
    println!("  cc.f (masked)   {}", machine.cpu.cc.f);
    println!("  cc.i (masked)   {}", machine.cpu.cc.i);
    println!("  pcs seen        {} distinct", pcs.len());
    println!("  back at entry   {resets} times");
    let mut top: Vec<_> = hot.into_iter().collect();
    top.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    println!("  busiest pages:");
    for (page, n) in top.into_iter().take(6) {
        println!("    {page:04x}xx  {n:>10}");
    }
    println!("  rom banks used  {:?}", banks.keys().collect::<Vec<_>>());
    println!("  switch columns  {columns:08b}");
    println!("  lamp columns    {lamp_columns:016b}");
    println!("  lamp rows ever  {lamp_rows:08b}");
    println!("  most lamps lit  {most_lit}");
    println!("  matrix now      {:?}", machine.board.lamps.matrix());
    println!("  solenoids ever  {sols:032b}");
    println!(
        "  sound latch     {:02x} (writes seen: {sound_writes})",
        machine.board.sound_latch
    );
    println!(
        "  dmd latch       {:02x}, enabled {}",
        machine.board.dmd_latch, machine.board.dmd_enabled
    );
    println!("  diagnostic led  {}", machine.board.diagnostic_led);
    if let Some(display) = &machine.dmd {
        let f = display.frame();
        let lit = f.iter().filter(|&&d| d > 0).count();
        println!("  dmd frames      {}", display.frames);
        println!("  dots lit        {lit} of {}", f.len());
        println!("  dmd status      {:02x}", machine.board.dmd_status);
        // A picture is worth more than a count.
        for y in 0..vpw_ws::dmd::HEIGHT {
            let row: String = (0..vpw_ws::dmd::WIDTH)
                .map(|x| match f[y * vpw_ws::dmd::WIDTH + x] {
                    0 => ' ',
                    1 => '.',
                    2 => '+',
                    _ => '#',
                })
                .collect();
            println!("    |{row}|");
        }
    }
    if unmapped.is_empty() {
        println!("  every address it touched is on the map");
    } else {
        println!("  addresses off the map:");
        for (addr, n) in unmapped.iter().take(10) {
            println!("    {addr:04x}  {n} times");
        }
    }
}

/// Two channels of sixteen-bit samples, in the only container everything reads.
fn write_wav(path: &str, samples: &[u8], rate: u32) {
    let mut out = Vec::with_capacity(samples.len() + 44);
    let block = 4u16;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // uncompressed
    out.extend_from_slice(&2u16.to_le_bytes()); // stereo
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    out.extend_from_slice(samples);
    let _ = std::fs::write(path, out);
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
        let mut entry = zip.by_index(i).expect("could not read an entry");
        if entry.is_dir() {
            continue;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data).expect("could not decompress");
        out.insert(entry.name().to_string(), data);
    }
    out
}
