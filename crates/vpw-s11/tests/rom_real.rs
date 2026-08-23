//! Boot of a real System 11 ROM.
//!
//! The ROMs are copyrighted material and do not go in the repo, so the test
//! skips itself unless it is told where the zip is:
//!
//! ```text
//! VPW_S11_ROM=/path/f14_l1.zip cargo test -p vpw-s11
//! ```

use std::collections::BTreeMap;
use std::io::Read;

use vpw_s11::{System11, pia};

/// Returns the images from the zip, or `None` if the path was not configured.
fn load_roms() -> Option<BTreeMap<String, Vec<u8>>> {
    let path = std::env::var("VPW_S11_ROM").ok()?;
    let file = std::fs::File::open(&path).expect("VPW_S11_ROM points at a file that will not open");
    let mut archive = zip::ZipArchive::new(file).expect("VPW_S11_ROM is not a valid zip");

    let mut out = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        if entry.is_file() {
            let name = entry.name().to_ascii_lowercase();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            out.insert(name, data);
        }
    }
    Some(out)
}

fn machine_with_rom(roms: &BTreeMap<String, Vec<u8>>) -> System11 {
    let pick = |needle: &str| {
        roms.iter()
            .find(|(name, _)| name.contains(needle))
            .map(|(_, data)| data.as_slice())
            .unwrap_or_else(|| panic!("the zip has no {needle} image"))
    };

    let mut machine = System11::new();
    machine
        .load_rom_48(pick("u26"), pick("u27"))
        .expect("the images fit");
    machine.reset();
    machine
}

/// Counts distinct addresses executed in `seconds` of simulated time.
fn run_and_trace(machine: &mut System11, seconds: f64, press_advance: bool) -> usize {
    let target = machine.cycle() + (seconds * 1_000_000.0) as u64;
    let mut seen = std::collections::HashSet::new();
    let mut next_pulse = machine.cycle() + 2_000_000;

    while machine.cycle() < target {
        if press_advance && machine.cycle() >= next_pulse {
            machine.board.diagnostic_advance = !machine.board.diagnostic_advance;
            next_pulse += 250_000;
        }
        machine.step();
        seen.insert(machine.cpu.pc);
    }
    seen.len()
}

#[test]
fn the_real_rom_boots_without_leaving_the_map() {
    let Some(roms) = load_roms() else {
        eprintln!("skipped: set VPW_S11_ROM to run this test");
        return;
    };
    let mut machine = machine_with_rom(&roms);

    // F-14 L-1 takes its reset vector from $FFFE, inside u27.
    assert_eq!(machine.cpu.pc, 0xC107, "reset vector");

    let seen = run_and_trace(&mut machine, 20.0, false);

    assert_eq!(
        machine.cpu.last_illegal, None,
        "the ROM executed no undefined opcodes"
    );
    assert!(!machine.cpu.is_hcf(), "the CPU did not hang");
    assert_eq!(
        machine.board.last_unmapped, None,
        "the ROM did not touch any address outside the map"
    );
    assert!(seen > 800, "the POST really ran ({seen} addresses)");
}

#[test]
fn the_real_rom_drives_the_peripherals() {
    let Some(roms) = load_roms() else {
        eprintln!("skipped: set VPW_S11_ROM to run this test");
        return;
    };
    let mut machine = machine_with_rom(&roms);
    run_and_trace(&mut machine, 20.0, false);

    // The ROM programs the port directions to match the board's real wiring.
    // That it does so by itself is a good sign that the memory map is right.
    assert_eq!(
        machine.board.pias[pia::SWITCHES].port_a_direction(),
        0x00,
        "the switch rows are inputs"
    );
    assert_eq!(
        machine.board.pias[pia::SWITCHES].port_b_direction(),
        0xFF,
        "the switch columns are outputs"
    );
    assert_eq!(
        machine.board.pias[pia::DISPLAY_SELECT].port_a_direction(),
        0x7F,
        "PA7 of PIA 2 is an input: it is the W7 jumper"
    );

    // And it is strobing the lamp matrix.
    let (_, columns) = machine.board.lamp_drive();
    assert_ne!(columns, 0, "there is a lamp column driven");
}

#[test]
fn the_diagnostic_button_opens_the_menu() {
    let Some(roms) = load_roms() else {
        eprintln!("skipped: set VPW_S11_ROM to run this test");
        return;
    };

    // A board that was just plugged in, with a blank CMOS, sits waiting for
    // the operator to press Advance. Pressing it has to get it into code that
    // does not run without the button.
    let mut idle = machine_with_rom(&roms);
    let without_button = run_and_trace(&mut idle, 20.0, false);

    let mut with_button_machine = machine_with_rom(&roms);
    let with_button = run_and_trace(&mut with_button_machine, 20.0, true);

    assert!(
        with_button > without_button * 3 / 2,
        "pressing Advance should open the diagnostic menu \
         (without button {without_button}, with button {with_button})"
    );
    assert_eq!(with_button_machine.cpu.last_illegal, None);
    assert_eq!(with_button_machine.board.last_unmapped, None);
}

#[test]
fn the_display_shows_the_boot_message() {
    let Some(roms) = load_roms() else {
        eprintln!("skipped: set VPW_S11_ROM to run this test");
        return;
    };
    let mut machine = machine_with_rom(&roms);
    run_and_trace(&mut machine, 10.0, false);

    // A board that was just plugged in shows "FACTORY SETTING". The O reads as
    // 0 and the S reads as 5 because in 14 segments they are the same drawing.
    assert_eq!(machine.upper_display(), "FACT0RY5ETTING");
    assert!(
        !machine.upper_display().contains('?'),
        "some glyph is not in the font: {:?}",
        machine.upper_display()
    );
}

#[test]
fn the_diagnostic_menu_shows_the_settings() {
    let Some(roms) = load_roms() else {
        eprintln!("skipped: set VPW_S11_ROM to run this test");
        return;
    };
    let mut machine = machine_with_rom(&roms);
    run_and_trace(&mut machine, 12.0, true);

    // Pressing Advance walks through the operator settings. The top one gives
    // the name and the bottom one the setting number and its value.
    let upper = machine.upper_display();
    let lower = machine.lower_display();

    assert_eq!(upper, "ATTRACT50UND5 ", "setting 48: ATTRACT SOUNDS");
    assert!(lower.contains("Ad 48"), "the setting number: {lower:?}");
    assert!(lower.trim_end().ends_with("0N"), "its value, ON: {lower:?}");

    assert!(!upper.contains('?'), "undecoded glyph on top: {upper:?}");
    assert!(
        !lower.contains('?'),
        "undecoded glyph on the bottom: {lower:?}"
    );
}

#[test]
fn the_post_tests_the_lamps_and_the_solenoids() {
    let Some(roms) = load_roms() else {
        eprintln!("skipped: set VPW_S11_ROM to run this test");
        return;
    };
    let mut machine = machine_with_rom(&roms);

    // During boot the ROM runs its test: it lights every lamp and pulses the
    // coils. It has to be watched by accumulating, because at any given
    // instant there is almost never anything active.
    let mut max_lamps = 0;
    let mut solenoids = 0u32;
    let target = machine.cycle() + 20_000_000;
    while machine.cycle() < target {
        machine.step();
        max_lamps = max_lamps.max(machine.board.lamps.lit_count());
        solenoids |= machine.board.solenoids.fired();
    }

    assert_eq!(max_lamps, 64, "the lamp test lights them all");
    assert_eq!(
        solenoids & 0xFF00,
        0xFF00,
        "the test pulses solenoids 9-16: {solenoids:#018b}"
    );

    // Once in the operator menu there is not a single one left lit.
    assert_eq!(machine.board.lamps.lit_count(), 0);
}

#[test]
fn the_real_sound_board_synthesises_effects_and_speech() {
    let Some(roms) = load_roms() else {
        eprintln!("skipped: set VPW_S11_ROM to run this test");
        return;
    };
    let pick = |needle: &str| {
        roms.iter()
            .find(|(name, _)| name.contains(needle))
            .map(|(_, d)| d.as_slice())
            .unwrap_or_else(|| panic!("{needle} is missing"))
    };

    /// Runs the board for half a second after sending it a command (or not)
    /// and returns `(DAC writes, speech bits)`.
    fn measure(u21: &[u8], u22: &[u8], command: Option<u8>) -> (u64, u64) {
        let mut board = vpw_s11::SoundBoard::new();
        assert!(board.load_roms(u21, u22));
        board.reset();
        board.run_until(200_000); // let it finish booting

        let before = (board.bus.dac_writes(), board.bus.cvsd_bits());
        if let Some(c) = command {
            board.send_command(c);
        }
        board.run_until(board.cycle() + 500_000);

        (
            board.bus.dac_writes() - before.0,
            board.bus.cvsd_bits() - before.1,
        )
    }

    let (u21, u22) = (pick("u21"), pick("u22"));

    // At rest the board touches nothing.
    let (dac, speech) = measure(u21, u22, None);
    assert_eq!(dac, 0, "without a command it should not write the DAC");
    assert_eq!(speech, 0, "nor the CVSD");

    // The low range is effects: they are synthesised through the DAC.
    let (dac, speech) = measure(u21, u22, Some(10));
    assert!(dac > 1000, "command 10 should move the DAC ({dac} writes)");
    assert!(speech < 100, "and it is not speech ({speech} bits)");

    // The high range is speech: it comes out of the CVSD and the DAC never
    // even notices.
    let (dac, speech) = measure(u21, u22, Some(191));
    assert!(speech > 5000, "command 191 should talk ({speech} bits)");
    assert_eq!(dac, 0, "the speech does not go through the DAC");
}

#[test]
fn the_complete_machine_runs_both_cpus_together() {
    let Some(roms) = load_roms() else {
        eprintln!("skipped: set VPW_S11_ROM to run this test");
        return;
    };
    let pick = |needle: &str| {
        roms.iter()
            .find(|(name, _)| name.contains(needle))
            .map(|(_, d)| d.as_slice())
            .unwrap_or_else(|| panic!("{needle} is missing"))
    };

    let mut machine = System11::new();
    machine.load_rom_48(pick("u26"), pick("u27")).unwrap();
    assert!(machine.load_sound_roms(pick("u21"), pick("u22")));
    machine.reset();

    let sound = machine.sound.as_ref().unwrap();
    assert_eq!(sound.cpu.pc, 0xDEF6, "reset vector of the sound board");

    machine.run_cycles(10_000_000);

    let sound = machine.sound.as_ref().unwrap();
    assert_eq!(
        sound.cpu.last_illegal, None,
        "the sound ROM did not go off the rails"
    );
    assert_eq!(sound.bus.last_unmapped, None, "nor did it leave the map");
    assert!(
        machine.cycle().abs_diff(sound.cycle()) < 20,
        "both CPUs run at 1 MHz: main {}, sound {}",
        machine.cycle(),
        sound.cycle()
    );
    assert!(
        sound.bus.dac_writes() > 1000,
        "the board should be making sound ({} DAC writes)",
        sound.bus.dac_writes()
    );
}

#[test]
fn the_real_cs_board_makes_music_and_speech() {
    let Some(roms) = load_roms() else {
        eprintln!("skipped: set VPW_S11_ROM to run this test");
        return;
    };
    let pick = |needle: &str| {
        roms.iter()
            .find(|(name, _)| name.contains(needle))
            .map(|(_, d)| d.as_slice())
            .unwrap_or_else(|| panic!("{needle} is missing"))
    };

    /// Runs the board for a second after sending it a command (or not) and
    /// returns `(DAC writes, speech bits, peak of active FM operators)`.
    fn measure(u4: &[u8], u19: &[u8], command: Option<u8>) -> (u64, u64, usize) {
        let mut board = vpw_s11::CsSoundBoard::new();
        assert!(board.load_roms(&[u4, u19]));
        board.reset();
        board.run_until(1_000_000);

        let before = (board.bus.dac_writes(), board.bus.cvsd_bits());
        if let Some(c) = command {
            board.send_command(c);
        }

        let mut fm = 0;
        let target = board.cycle() + 1_000_000;
        while board.cycle() < target {
            board.step();
            fm = fm.max(board.bus.ym.active_operators());
        }

        (
            board.bus.dac_writes() - before.0,
            board.bus.cvsd_bits() - before.1,
            fm,
        )
    }

    let (u4, u19) = (pick("u4"), pick("u19"));

    // It boots clean.
    let mut board = vpw_s11::CsSoundBoard::new();
    board.load_roms(&[u4, u19]);
    board.reset();
    assert_eq!(board.cpu.pc, 0xB6B8, "reset vector of the CS board");
    board.run_until(2_000_000);
    assert_eq!(
        board.cpu.last_illegal, None,
        "the ROM executed nothing strange"
    );
    assert_eq!(board.bus.last_unmapped, None, "nor did it leave the map");

    // At rest it does nothing.
    let (dac, speech, fm) = measure(u4, u19, None);
    assert_eq!(
        (dac, speech, fm),
        (0, 0, 0),
        "without a command it should sit still"
    );

    // The 152-156 range is music: synthesised with the YM's 32 operators.
    let (_, speech, fm) = measure(u4, u19, Some(152));
    assert_eq!(fm, 32, "command 152 should use the whole FM chip");
    assert_eq!(speech, 0, "and it is not speech");

    // The 81-84 range is speech: it comes out of the CVSD without touching the
    // FM.
    let (_, speech, fm) = measure(u4, u19, Some(81));
    assert!(speech > 5000, "command 81 should talk ({speech} bits)");
    assert_eq!(fm, 0, "and it does not use the FM synthesis");
}

#[test]
fn the_complete_machine_runs_its_three_cpus() {
    let Some(roms) = load_roms() else {
        eprintln!("skipped: set VPW_S11_ROM to run this test");
        return;
    };
    let pick = |needle: &str| {
        roms.iter()
            .find(|(name, _)| name.contains(needle))
            .map(|(_, d)| d.as_slice())
            .unwrap_or_else(|| panic!("{needle} is missing"))
    };

    let mut machine = System11::new();
    machine.load_rom_48(pick("u26"), pick("u27")).unwrap();
    assert!(machine.load_sound_roms(pick("u21"), pick("u22")));
    assert!(machine.load_cs_sound_roms(&[pick("u4"), pick("u19")]));
    machine.reset();

    // All three start where their reset vector says.
    assert_eq!(machine.cpu.pc, 0xC107, "main");
    assert_eq!(machine.sound.as_ref().unwrap().cpu.pc, 0xDEF6, "XS board");
    assert_eq!(
        machine.sound_cs.as_ref().unwrap().cpu.pc,
        0xB6B8,
        "CS board"
    );

    machine.run_cycles(20_000_000); // twenty simulated seconds

    // None of them goes off the rails.
    assert_eq!(machine.cpu.last_illegal, None, "main");
    assert_eq!(machine.board.last_unmapped, None, "main");
    let xs = machine.sound.as_ref().unwrap();
    assert_eq!(xs.cpu.last_illegal, None, "XS board");
    assert_eq!(xs.bus.last_unmapped, None, "XS board");
    let cs = machine.sound_cs.as_ref().unwrap();
    assert_eq!(cs.cpu.last_illegal, None, "CS board");
    assert_eq!(cs.bus.last_unmapped, None, "CS board");

    // And each one runs at its own clock: 1 MHz, 1 MHz and 2 MHz.
    assert!(machine.cycle().abs_diff(xs.cycle()) < 20);
    assert!((machine.cycle() * 2).abs_diff(cs.cycle()) < 20);

    // The audio comes out at 48 kHz: twenty seconds are 960000 samples.
    assert_eq!(machine.pending_audio(), 960_000);

    // And the machine is still showing its boot message.
    assert_eq!(machine.upper_display(), "FACT0RY5ETTING");
}

/// Boots every set in the manifest that is present in the given directory.
///
/// It skips itself unless it is told where the ROMs are:
///
/// ```text
/// VPW_ROM_DIR=/path/to/the/roms cargo test --release -p vpw-s11 -- --nocapture
/// ```
#[test]
fn the_set_manifest_boots_almost_entirely() {
    let Ok(dir) = std::env::var("VPW_ROM_DIR") else {
        eprintln!("skipped: set VPW_ROM_DIR to run this test");
        return;
    };
    let dir = std::path::Path::new(&dir);

    let mut tested = 0usize;
    let mut clean = 0usize;
    let mut problems = Vec::new();

    // The manifest is the emulator's own, not a copy kept next to the test:
    // a set that boots here has to be one a caller can actually ask for.
    for game in vpw_s11::games::all() {
        let path = dir.join(format!("{}.zip", game.set));
        if !path.exists() {
            continue;
        }

        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let Ok(mut zip) = zip::ZipArchive::new(file) else {
            continue;
        };
        let mut roms: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for i in 0..zip.len() {
            let mut e = zip.by_index(i).unwrap();
            if e.is_file() {
                let name = e.name().to_ascii_lowercase();
                let mut data = Vec::new();
                e.read_to_end(&mut data).unwrap();
                roms.insert(name, data);
            }
        }

        // A zip missing a file the manifest names is not the emulator's
        // problem, so it is skipped rather than counted against it.
        let Some(first) = roms.get(game.image1) else {
            continue;
        };
        let second = match game.image2 {
            Some(name) => match roms.get(name) {
                Some(d) => Some(d.as_slice()),
                None => continue,
            },
            None => None,
        };
        let Some(images) = game.layout.place(first, second) else {
            continue;
        };
        let set = game.set;

        let mut machine = System11::new();
        if machine.load_cpu_region(&images).is_err() {
            continue;
        }
        machine.set_display_config(game.display);
        machine.reset();
        machine.run_cycles(8_000_000);

        tested += 1;
        if machine.cpu.last_illegal.is_none()
            && machine.board.last_unmapped.is_none()
            && !machine.cpu.is_hcf()
        {
            clean += 1;
        } else {
            problems.push(set.to_string());
        }
    }

    if tested == 0 {
        eprintln!("skipped: there was no set from the manifest in {dir:?}");
        return;
    }

    let percentage = clean * 100 / tested;
    eprintln!("{clean} of {tested} sets clean ({percentage}%)");
    if !problems.is_empty() {
        eprintln!("with problems: {problems:?}");
    }
    assert!(
        percentage >= 98,
        "{percentage}% of the sets booted clean; something broke ({problems:?})"
    );
}
