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
    println!("reset vector -> pc {:04x}", machine.cpu.pc);
    println!();

    // Somebody has to be at the door, or a board that boots sits waiting.
    machine.board.switches.set_dedicated(0, false);

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
    while spent < steps {
        spent += u64::from(machine.step());
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

    println!("after {seconds}s of board time:");
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
