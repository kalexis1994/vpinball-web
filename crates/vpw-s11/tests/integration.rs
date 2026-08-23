//! The complete machine: the main board and the two sound boards running
//! together.
//!
//! A System 11A has **three** CPUs with different clocks, and each sound board
//! receives its commands over its own path. This checks that all of that stays
//! hooked up.

use vpw_s11::{Bus, PIA_BASES, ROM_START, System11, pia};

/// `BRA -2`: a tight loop. The encoding is the same on the 6800 and on the
/// 6809.
const LOOP: [u8; 2] = [0x20, 0xFE];

/// A minimal program for the main board.
///
/// It has to be a loop and not a field of NOPs: without it, the PC walks the
/// whole ROM, wraps around and ends up executing RAM.
fn main_rom() -> Vec<u8> {
    let mut rom = vec![0x01u8; 0xC000]; // NOP
    rom[0..2].copy_from_slice(&LOOP); // at $4000
    let reset = (0xFFFE - ROM_START) as usize;
    rom[reset] = 0x40;
    rom[reset + 1] = 0x00;
    rom
}

/// A 32 KB image for the XS board, which starts at `$C000`.
///
/// The high window sees `u21` from its offset `$4000`, so that is where the
/// loop goes.
fn xs_rom() -> Vec<u8> {
    let mut rom = vec![0x01u8; 0x8000];
    rom[0x4000..0x4002].copy_from_slice(&LOOP);
    rom[0x7FFE] = 0xC0;
    rom[0x7FFF] = 0x00;
    rom
}

/// A 32 KB image for the CS board, which starts at `$9000`.
fn cs_rom() -> Vec<u8> {
    let mut rom = vec![0x12u8; 0x8000]; // the 6809's NOP
    rom[0x1000..0x1002].copy_from_slice(&LOOP);
    rom[0x7FFE] = 0x90;
    rom[0x7FFF] = 0x00;
    rom
}

fn complete_machine() -> System11 {
    let mut machine = System11::new();
    machine.board.load_rom(ROM_START, &main_rom()).unwrap();
    assert!(machine.load_sound_roms(&xs_rom(), &[0u8; 0x8000]));
    assert!(machine.load_cs_sound_roms(&[&cs_rom()]));
    machine.reset();
    machine
}

/// Leaves a port of the given PIA as output and with the access on data.
fn prepare_port(machine: &mut System11, pia_index: usize, port_b: bool) {
    let base = PIA_BASES[pia_index];
    let (control, data) = if port_b {
        (base + 3, base + 2)
    } else {
        (base + 1, base)
    };
    machine.board.write(control, 0x00); // point at the direction
    machine.board.write(data, 0xFF); // all output
    machine.board.write(control, 0x04); // point at the data
}

// --- Timing -----------------------------------------------------------------------

#[test]
fn the_three_cpus_run_at_their_own_clock() {
    let mut machine = complete_machine();
    machine.run_cycles(2_000_000);

    let main = machine.cycle();
    let xs = machine.sound.as_ref().unwrap().cycle();
    let cs = machine.sound_cs.as_ref().unwrap().cycle();

    assert!(
        main.abs_diff(xs) < 20,
        "the XS runs at 1 MHz just like the main board: {main} against {xs}"
    );
    assert!(
        (main * 2).abs_diff(cs) < 20,
        "the CS runs at 2 MHz, that is twice as fast: {} against {cs}",
        main * 2
    );
}

#[test]
fn the_audio_comes_out_at_the_requested_rate() {
    let mut machine = complete_machine();
    machine.run_cycles(1_000_000); // one CPU second

    assert_eq!(
        machine.pending_audio(),
        48_000,
        "one second should give 48000 samples"
    );

    let audio = machine.drain_audio();
    assert_eq!(audio.len(), 48_000);
    assert_eq!(machine.pending_audio(), 0, "draining it leaves it at zero");
}

#[test]
fn the_audio_rate_can_be_changed() {
    let mut machine = complete_machine();
    machine.set_audio_rate(22_050);
    machine.run_cycles(1_000_000);

    assert_eq!(machine.pending_audio(), 22_050);
}

#[test]
fn the_audio_never_goes_out_of_range() {
    let mut machine = complete_machine();
    machine.run_cycles(500_000);

    for (i, sample) in machine.drain_audio().iter().enumerate() {
        assert!(
            (-1.0..=1.0).contains(sample),
            "sample {i} went out of range: {sample}"
        );
    }
}

// --- The command paths ---------------------------------------------------------------

#[test]
fn the_xs_board_receives_through_pia_0() {
    // The latch comes out of PIA 0's port A and the handshake out of its CA2.
    let mut machine = complete_machine();
    prepare_port(&mut machine, pia::SOUND_SOLENOIDS, false);

    machine.board.write(PIA_BASES[pia::SOUND_SOLENOIDS], 0x5A);
    machine.step();

    let xs = &mut machine.sound.as_mut().unwrap().bus.pia;
    assert_eq!(xs.port_a_direction(), 0x00, "the XS's port A is an input");
    // The control register has to be pointed at the data; otherwise reading
    // the port returns the direction register.
    xs.write(1, 0x04);
    assert_eq!(xs.read(0), 0x5A, "the sound number reached the XS board");
}

#[test]
fn the_cs_board_receives_through_pia_5() {
    // It is the other path: the CS does not listen to the same latch as the XS.
    let mut machine = complete_machine();
    prepare_port(&mut machine, pia::WIDGET, true);

    machine.board.write(PIA_BASES[pia::WIDGET] + 2, 0xA5);
    machine.step();

    // On the CS, port B is the channel to the main board.
    let cs = &mut machine.sound_cs.as_mut().unwrap().bus.pia;
    cs.write(3, 0x04);
    assert_eq!(cs.read(2), 0xA5, "the sound number reached the CS board");
}

#[test]
fn the_two_boards_listen_to_different_latches() {
    let mut machine = complete_machine();
    prepare_port(&mut machine, pia::SOUND_SOLENOIDS, false);
    prepare_port(&mut machine, pia::WIDGET, true);

    machine.board.write(PIA_BASES[pia::SOUND_SOLENOIDS], 0x11);
    machine.board.write(PIA_BASES[pia::WIDGET] + 2, 0x22);
    machine.step();

    let xs = &mut machine.sound.as_mut().unwrap().bus.pia;
    xs.write(1, 0x04);
    assert_eq!(xs.read(0), 0x11, "the XS listens to PIA 0");

    let cs = &mut machine.sound_cs.as_mut().unwrap().bus.pia;
    cs.write(3, 0x04);
    assert_eq!(cs.read(2), 0x22, "the CS listens to PIA 5");
}

// --- Partial configurations -------------------------------------------------------------

#[test]
fn the_machine_runs_without_any_sound_board() {
    let mut machine = System11::new();
    machine.board.load_rom(ROM_START, &main_rom()).unwrap();
    machine.reset();
    machine.run_cycles(100_000);

    assert!(machine.sound.is_none());
    assert!(machine.sound_cs.is_none());
    assert_eq!(machine.audio_sample(), 0.0, "silence");
    assert_eq!(
        machine.pending_audio(),
        4800,
        "but the audio clock runs all the same"
    );
}

#[test]
fn the_machine_runs_with_a_single_board() {
    let mut machine = System11::new();
    machine.board.load_rom(ROM_START, &main_rom()).unwrap();
    assert!(machine.load_cs_sound_roms(&[&cs_rom()]));
    machine.reset();
    machine.run_cycles(100_000);

    assert!(machine.sound.is_none());
    assert!(machine.sound_cs.is_some());
    assert_eq!(
        machine.sound_sample(),
        0.0,
        "the XS that isn't there makes no sound"
    );
    assert_eq!(
        machine.cpu.last_illegal, None,
        "the main board stayed in its loop"
    );
    assert_eq!(
        machine.sound_cs.as_ref().unwrap().cpu.last_illegal,
        None,
        "and so did the CS"
    );
}

#[test]
fn an_invalid_sound_rom_leaves_nothing_half_done() {
    let mut machine = System11::new();
    assert!(!machine.load_cs_sound_roms(&[&[0u8; 100]]));
    assert!(machine.sound_cs.is_none());
}

#[test]
fn the_reset_clears_the_three_boards_and_the_audio() {
    let mut machine = complete_machine();
    machine.run_cycles(500_000);
    assert!(machine.pending_audio() > 0);

    machine.reset();

    assert_eq!(machine.cycle(), 0);
    assert_eq!(machine.sound.as_ref().unwrap().cycle(), 0);
    assert_eq!(machine.sound_cs.as_ref().unwrap().cycle(), 0);
    assert_eq!(machine.pending_audio(), 0);
}

// --- Input -----------------------------------------------------------------------

#[test]
fn the_switches_open_and_close_by_number() {
    let mut machine = complete_machine();

    assert!(
        machine.set_switch(63, true),
        "63 exists: it is one of the F-14's flippers"
    );
    assert!(machine.switch_closed(63));

    assert!(machine.set_switch(63, false));
    assert!(!machine.switch_closed(63));

    assert!(
        !machine.set_switch(99, true),
        "99 does not exist in the matrix"
    );
    assert!(!machine.switch_closed(99));
}

#[test]
fn a_switch_pulse_lets_the_machine_run() {
    // It is what happens with a rollover: the CPU has to get a chance to sweep
    // its column while the switch is closed.
    let mut machine = complete_machine();
    let before = machine.cycle();

    assert!(machine.pulse_switch(63, 5_000));

    assert!(
        machine.cycle() >= before + 5_000,
        "it ran while it was closed"
    );
    assert!(!machine.switch_closed(63), "and it ended up open");
}

#[test]
fn they_can_all_be_opened_at_once() {
    let mut machine = complete_machine();
    for number in [11, 25, 47, 63, 88] {
        machine.set_switch(number, true);
    }
    machine.clear_switches();
    for number in [11, 25, 47, 63, 88] {
        assert!(!machine.switch_closed(number), "{number} stayed closed");
    }
}

#[test]
fn the_diagnostic_buttons_reach_pia_2() {
    let mut machine = complete_machine();

    machine.set_diagnostic_advance(true);
    machine.set_diagnostic_up(true);
    // The board refreshes those lines when it raises the periodic interrupt.
    machine.run_cycles(2 * vpw_s11::IRQ_PERIOD_CYCLES);

    let pia2 = &machine.board.pias[pia::DISPLAY_SELECT];
    let (ca1, cb1) = pia2.irq_flags_a();
    assert!(
        ca1 || cb1 || pia2.irq_flags_b().0,
        "pressing the buttons should leave some mark on PIA 2"
    );
}

#[test]
fn the_cs_board_can_answer_the_main_one() {
    // PIA 5's port B is bidirectional: the main board leaves the sound number
    // and the CS answers over the same path.
    let mut machine = complete_machine();

    // Have the CS leave something on its port B.
    let cs = &mut machine.sound_cs.as_mut().unwrap().bus;
    cs.pia.write(3, 0x00); // point at the direction
    cs.pia.write(2, 0xFF); // all output
    cs.pia.write(3, 0x04); // point at the data
    cs.pia.write(2, 0x5A);
    // A read of the PIA propagates the answer.
    cs.read(0x4002);

    machine.step();

    // And the main board sees it as an input on its PIA 5 port B.
    let base = PIA_BASES[pia::WIDGET];
    machine.board.write(base + 3, 0x04); // CRB -> data
    assert_eq!(
        machine.board.read(base + 2),
        0x5A,
        "the CS's answer should reach PIA 5"
    );
}
