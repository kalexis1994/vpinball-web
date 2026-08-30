//! The System 80B sound cards, against firmware written here.
//!
//! No 80B set was to hand, so what a real ROM would prove — that a game plays
//! a tune — is not proved. What these prove is everything it would rest on:
//! that both processors wake up in the right place, that a command from the
//! game board arrives inverted in a latch they both read and interrupts them
//! both, that the Y-CPU's own interrupt runs at the rate its register asks
//! for, that the converter is a level multiplied by a volume, and that the
//! music chips are reachable through the handshake each generation uses.

use vpw_gts80::games;
use vpw_gts80::sound80b::{Generation, SoundCard80B};

const RATE: u32 = 48_000;

/// Where the firmware sits in each processor's memory, and what it counts in.
const ENTRY: u16 = 0xE100;
const IRQ_COUNT: u16 = 0x0010;
const NMI_COUNT: u16 = 0x0011;

/// A first-generation image: 8 KB, landing at `$E000`.
///
/// The firmware counts its interrupts and copies whatever the latch holds into
/// RAM, which is enough to see the board's wiring from outside.
fn firmware() -> Vec<u8> {
    let mut rom = vec![0xEAu8; 0x2000];
    let put = |rom: &mut Vec<u8>, at: usize, bytes: &[u8]| {
        rom[at..at + bytes.len()].copy_from_slice(bytes);
    };

    #[rustfmt::skip]
    let program: &[u8] = &[
        0x58,                   // CLI
        0x4C, 0x00, 0xE1,       // JMP $E100, forever
    ];
    put(&mut rom, 0x100, program);
    // The interrupt handlers, at $E180 and $E190.
    put(&mut rom, 0x180, &[0xEE, IRQ_COUNT as u8, 0x00, 0x40]);
    put(&mut rom, 0x190, &[0xEE, NMI_COUNT as u8, 0x00, 0x40]);

    put(&mut rom, 0x1FFA, &[0x90, 0xE1]); // NMI
    put(&mut rom, 0x1FFC, &[0x00, 0xE1]); // RESET
    put(&mut rom, 0x1FFE, &[0x80, 0xE1]); // IRQ
    rom
}

/// Lays an image out the way the board does: stacked down from the top.
fn stacked(image: &[u8]) -> Vec<u8> {
    let mut space = vec![0u8; 0x10000];
    let at = 0x10000 - image.len();
    space[at..].copy_from_slice(image);
    space
}

fn built(generation: Generation) -> SoundCard80B {
    let rom = stacked(&firmware());
    SoundCard80B::new(generation, rom.clone(), rom, RATE)
}

#[test]
fn both_processors_wake_up_where_the_vectors_point() {
    let card = built(Generation::One);
    assert_eq!(card.y_cpu.pc, ENTRY, "the music processor");
    assert_eq!(card.d_cpu.pc, ENTRY, "and the one driving the converter");
}

/// The game board drives the bus the other way up, and the card's first
/// command arrives before it is listening.
#[test]
fn a_command_is_inverted_and_the_first_one_is_dropped() {
    let mut card = built(Generation::One);
    card.run(20_000.0);
    let before = card.ram_y(IRQ_COUNT);

    // The first is thrown away whatever it is.
    card.command(0xA5);
    card.run(20_000.0);
    assert_eq!(card.ram_y(IRQ_COUNT), before, "nobody was listening yet");

    // And the second arrives inverted, in a latch both processors read.
    card.command(0xA5);
    card.run(20_000.0);
    assert!(
        card.ram_y(IRQ_COUNT) > before,
        "the music processor heard it"
    );
    assert!(card.ram_d(IRQ_COUNT) > 0, "and so did the other one");

    // `$FF` inverts to nothing, which is the board saying nothing.
    let now = card.ram_y(IRQ_COUNT);
    card.command(0x00);
    card.run(20_000.0);
    assert_eq!(card.ram_y(IRQ_COUNT), now, "all ones is silence");
}

/// The Y-CPU has no fixed timer: a register sets its own interrupt rate, and a
/// game changes it mid-tune. It fires only while the control register says so,
/// and a new rate takes effect at the next firing rather than at once — the
/// board reloads the timer when it goes off, not when the register is written.
#[test]
fn the_interrupt_runs_at_the_rate_the_register_asks_for() {
    let clock = f64::from(Generation::One.clock());

    // Nothing until it is switched on, however long we wait.
    let mut idle = built(Generation::One);
    idle.write_y(0xA000, 0xFF);
    idle.run(clock);
    assert_eq!(idle.ram_y(NMI_COUNT), 0, "not enabled yet");

    // The fastest the register goes: both nibbles at fifteen, which divides
    // the 976 Hz clock by one and one, so about a thousand a second.
    let mut fast = built(Generation::One);
    fast.write_y(0xA000, 0xFF);
    fast.write_y(0x4000, 0x01);
    fast.run(clock * 0.05);
    let quick = fast.ram_y(NMI_COUNT);
    assert!(
        quick > 20,
        "a thousand a second is more than {quick} in a twentieth"
    );

    // And the slowest: both nibbles at zero divides by sixteen twice, which is
    // under four a second.
    let mut slow = built(Generation::One);
    slow.write_y(0xA000, 0x00);
    slow.write_y(0x4000, 0x01);
    slow.run(clock * 0.05);
    let few = slow.ram_y(NMI_COUNT);
    assert!(
        few <= 1,
        "four a second is no more than one in a twentieth, not {few}"
    );
}

/// The D-CPU does one thing: it writes a level and a volume to a converter,
/// and what comes out is the two multiplied.
#[test]
fn the_converter_is_a_level_times_a_volume() {
    for (generation, at) in [
        (Generation::One, 0x4000u16),
        (Generation::Two, 0x8000),
        (Generation::Three, 0x8000),
    ] {
        let mut card = built(generation);
        card.write_d(at, 0xFF); // volume, hard over
        card.write_d(at + 1, 0xFF); // and the level with it
        let loud = (0..2_000)
            .map(|_| card.card.sample())
            .fold(0.0f32, f32::max);

        card.write_d(at, 0x00); // volume down
        let quiet = (0..2_000)
            .map(|_| card.card.sample().abs())
            .fold(0.0f32, f32::max);
        assert!(loud > quiet, "{generation:?}: the volume should matter");
    }
}

/// A first- or second-generation card reaches its two square-wave chips
/// through a handshake: a byte on a latch, then the direction pin falling with
/// the register-or-data pin saying which.
#[test]
fn the_music_chips_are_reached_through_the_handshake() {
    for (generation, latch, control) in [
        (Generation::One, 0x8000u16, 0x4000u16),
        (Generation::Two, 0x8000, 0xA000),
    ] {
        let mut card = built(generation);
        let send = |card: &mut SoundCard80B, chip_bit: u8, reg: u8, value: u8| {
            // The register number, with the select pin high.
            card.write_y(latch, reg);
            card.write_y(control, 0x04 | chip_bit | 0x10);
            card.write_y(control, chip_bit | 0x10);
            // Then the value, with it low.
            card.write_y(latch, value);
            card.write_y(control, 0x04 | chip_bit);
            card.write_y(control, chip_bit);
        };

        // Channel A of the first chip: a period, and the mixer letting it out.
        send(&mut card, 0x08, 0, 250);
        send(&mut card, 0x08, 7, 0b0011_1110);
        send(&mut card, 0x08, 8, 15);
        let first = (0..4_800)
            .map(|_| card.card.sample().abs())
            .fold(0.0f32, f32::max);
        assert!(first > 0.01, "{generation:?}: the first chip should sound");

        // And the second is the same handshake with one bit down.
        let mut card = built(generation);
        send(&mut card, 0x00, 0, 250);
        send(&mut card, 0x00, 7, 0b0011_1110);
        send(&mut card, 0x00, 8, 15);
        let second = (0..4_800)
            .map(|_| card.card.sample().abs())
            .fold(0.0f32, f32::max);
        assert!(second > 0.01, "{generation:?}: and so should the second");
    }
}

/// The third card has an FM chip where the others have the square waves, and
/// it is reached through a port select rather than a handshake.
#[test]
fn the_third_card_has_the_fm_chip_and_the_others_do_not() {
    let three = built(Generation::Three);
    assert!(three.card.fm.is_some());
    assert!(three.card.speech.is_none());

    let one = built(Generation::One);
    assert!(one.card.fm.is_none());
    assert!(
        one.card.speech.is_some(),
        "and the first has the speech chip"
    );

    let two = built(Generation::Two);
    assert!(two.card.fm.is_none());
    assert!(two.card.speech.is_none(), "which the second dropped");
}

/// A set is recognised by the names of its sound images: Gottlieb names them
/// for the processor they belong to, which no other card does.
#[test]
fn a_set_says_which_card_it_has_and_the_list_says_which_of_the_last_two() {
    let image = |name: &str, size: usize| (name.to_string(), vec![0u8; size]);

    // A first-generation set: 8 KB images, one for each processor.
    let gen1 = [
        image("prom1.cpu", 8192),
        image("drom1.snd", 8192),
        image("yrom1.snd", 8192),
    ];
    let roms = games::detect("rock", &gen1).expect("a System 80B");
    match roms.sound {
        Some(games::Sound::Card80B { generation, .. }) => {
            assert_eq!(generation, Generation::One);
        }
        other => panic!("expected a System 80B card, got {other:?}"),
    }

    // A later one: 32 KB images. Which of the two it is cannot be seen here,
    // so the name decides.
    let later = [
        image("prom1.cpu", 8192),
        image("drom1.snd", 32768),
        image("yrom1.snd", 32768),
    ];
    let victory = games::detect("victory", &later).expect("a System 80B");
    match victory.sound {
        Some(games::Sound::Card80B { generation, .. }) => {
            assert_eq!(generation, Generation::Two, "not on the list");
        }
        other => panic!("expected a System 80B card, got {other:?}"),
    }
    let hotshots = games::detect("hotshots", &later).expect("a System 80B");
    match hotshots.sound {
        Some(games::Sound::Card80B { generation, .. }) => {
            assert_eq!(generation, Generation::Three, "on the list");
        }
        other => panic!("expected a System 80B card, got {other:?}"),
    }

    // The sound images must not be mistaken for the game's: `drom1.snd` is
    // 8 KB and so is the system ROM.
    assert!(matches!(roms.system, games::System::Alphanumeric(s) if s.len() == 8192));
}

/// The music processor's two images stack downwards, because the vectors are
/// at the top and the second image has to sit under the first.
#[test]
fn two_images_stack_down_from_the_vectors() {
    let mut high = vec![0xAAu8; 0x2000];
    high[0x1FFC] = 0x00;
    high[0x1FFD] = 0xE1;
    let low = vec![0xBBu8; 0x2000];

    let images = [
        ("prom1.cpu".to_string(), vec![0u8; 8192]),
        ("drom1a.snd".to_string(), vec![0u8; 0x2000]),
        ("yrom1a.snd".to_string(), high),
        ("yrom2a.snd".to_string(), low),
    ];
    let roms = games::detect("genesis", &images).expect("a System 80B");
    let games::Sound::Card80B { y, .. } = roms.sound.expect("a card") else {
        panic!("expected a System 80B card");
    };
    assert_eq!(y.0[0], 0xAA, "yrom1 first, and it goes at the top");
    assert_eq!(y.1.expect("a second image")[0], 0xBB, "yrom2 under it");
}
