//! The display board, without needing its ROM.
//!
//! It is a second computer with a one-byte letterbox, and almost everything
//! that can go wrong here is about that letterbox: a command that is stored but
//! never strobed, a busy line that never clears, a reset that fires on the
//! wrong edge. None of those stop the board running. They stop it *listening*,
//! which from the outside is a machine with a blank display and no explanation.

use vpw_ws::dmd::{Dmd, HEIGHT, WIDTH};

#[test]
fn it_is_a_hundred_and_twenty_eight_by_thirty_two() {
    let d = Dmd::new();
    assert_eq!(d.frame().len(), WIDTH * HEIGHT);
    assert_eq!(WIDTH, 128);
    assert_eq!(HEIGHT, 32);
}

#[test]
fn a_command_arrives_on_the_strobe_going_up_and_not_on_the_write() {
    // `dmd32_ctrl_w` (`dedmd.c:123`) takes the command on bit 0's rising edge.
    // Storing it on the write instead would work for a board that is idle and
    // lose one for a board that is not, which is the harder half of the time.
    let mut d = Dmd::new();
    assert!(!d.busy(), "nothing has been sent");
    d.set_data(0x42);
    assert!(!d.busy(), "writing the latch is not sending it");
    d.set_ctrl(1);
    assert!(d.busy(), "the strobe is what sends it");
}

#[test]
fn the_same_strobe_held_high_does_not_send_it_twice() {
    let mut d = Dmd::new();
    d.set_data(1);
    d.set_ctrl(1);
    assert!(d.busy());
    // Reading it is what clears the line, and nothing has read it.
    d.set_ctrl(1);
    assert!(d.busy());
}

#[test]
fn reset_is_the_other_line_going_down() {
    // Bit 1, falling (`dedmd.c:129`). A board that reset on the rising edge
    // would reset when the CPU board asserts it and then run normally while it
    // is *held*, which is exactly backwards.
    let mut d = Dmd::new();
    d.set_ctrl(0x02);
    d.set_data(7);
    d.set_ctrl(0x03);
    assert!(d.busy(), "a command went in");
    d.set_ctrl(0x01); // bit 1 falls
    // The reset does not clear the letterbox; it restarts the processor. What
    // is being checked is that it is taken at all, and that it is taken here.
    assert_eq!(d.status(), 0, "a board that has just reset says nothing");
}

#[test]
fn a_board_with_no_rom_still_answers_its_lines() {
    // Every game does not have a display board, and a machine without one has
    // to keep running rather than wedge on a status byte that never comes.
    let mut d = Dmd::new();
    d.run_until(100_000);
    assert_eq!(d.status() & 0xf0, 0, "status is four bits");
}

#[test]
fn a_short_image_is_mirrored_to_fill_the_region() {
    // The same rule as the CPU board's: the region is half a megabyte whatever
    // the game carries, and the fixed half of the map is the end of it.
    let mut d = Dmd::new();
    let mut image = vec![0u8; 0x8000];
    image[0] = 0xaa;
    assert!(d.load_rom(&image).is_ok());
    assert!(d.load_rom(&[]).is_err(), "an empty image is not an image");
    assert!(
        d.load_rom(&vec![0u8; 0x10_0000]).is_err(),
        "an image bigger than the region does not fit it"
    );
}

#[test]
fn the_dots_have_four_levels_and_start_dark() {
    // Not a stencil. The panel is one bit and the picture is four levels,
    // because every row is rasterised twice from two frames a page apart —
    // see the module note. A host that treats this as on-or-off throws away
    // two thirds of what the board is saying.
    let d = Dmd::new();
    assert!(
        d.frame().iter().all(|&v| v <= 3),
        "a dot is 0 to 3 and nothing else"
    );
    assert!(
        d.frame().iter().all(|&v| v == 0),
        "an unloaded board is dark"
    );
}

/// The one reset the display board ever gets, and where it comes from.
///
/// `dmdlatch_w` writes the data and then drives the control lines to a literal
/// `0` and a literal `1` (`se.c:725`). The zero is what clears bit 1, which
/// `dmdreset_w` had left set, and bit 1 falling is what resets this board. A
/// port that preserves bit 1 — the obvious reading, since the command lives in
/// bit 0 — sends every command perfectly and never once resets the processor.
#[test]
fn the_command_latch_drops_the_reset_line_on_its_way_past() {
    let mut d = Dmd::new();
    // `dmdreset_w` with a non-zero byte: bit 1 up, and nothing has happened.
    d.set_ctrl(0x02);
    assert_eq!(d.resets, 0, "asserting reset is not resetting");

    d.latch(0x42);
    assert_eq!(d.resets, 1, "the first command after it pulses reset");
    assert!(d.busy(), "and the command still goes in");

    // Only the first: bit 1 is down now and stays down until the CPU board
    // asserts it again.
    d.latch(0x43);
    assert_eq!(d.resets, 1);
    d.set_ctrl(0x02);
    d.latch(0x44);
    assert_eq!(d.resets, 2);
}

/// The filter, fed by hand. See `vpw_ws::dmd::Pwm`.
mod pwm {
    use vpw_ws::dmd::{FRAME_BYTES, Pwm};

    /// A plane with dot (0, 0) lit or not. Bit 7 of byte 0 is the first dot.
    fn plane(lit: bool) -> [u8; FRAME_BYTES] {
        let mut p = [0u8; FRAME_BYTES];
        if lit {
            p[0] = 0x80;
        }
        p
    }

    /// One vblank's worth: the slow plane twice, the fast one once
    /// (`dedmd.c:168-169`).
    fn vblank(pwm: &mut Pwm, slow: bool, fast: bool) {
        pwm.submit(&plane(slow), 2);
        pwm.submit(&plane(fast), 1);
        pwm.update();
    }

    #[test]
    fn a_dot_lit_in_both_frames_is_fully_on_and_in_neither_is_off() {
        let mut pwm = Pwm::new();
        for _ in 0..Pwm::TAPS {
            vblank(&mut pwm, true, true);
        }
        assert_eq!(pwm.luminance()[0], 255);
        assert_eq!(pwm.luminance()[1], 0, "its neighbour was never lit");
        for _ in 0..Pwm::TAPS {
            vblank(&mut pwm, false, false);
        }
        assert_eq!(pwm.luminance()[0], 0);
    }

    #[test]
    fn the_slow_frame_is_two_thirds_and_the_fast_one_a_third() {
        // Not because the filter knows: because the slow frame is submitted
        // twice and the fast one once, and the weights are symmetric enough
        // that the share comes out as the share of time on the glass.
        let mut slow = Pwm::new();
        let mut fast = Pwm::new();
        for _ in 0..Pwm::TAPS {
            vblank(&mut slow, true, false);
            vblank(&mut fast, false, true);
        }
        let (s, f) = (
            i32::from(slow.luminance()[0]),
            i32::from(fast.luminance()[0]),
        );
        assert!(
            (s - 170).abs() <= 12,
            "slow only should be about two thirds, was {s}"
        );
        assert!(
            (f - 85).abs() <= 12,
            "fast only should be about a third, was {f}"
        );
        assert!(s > f, "and the slow frame is the brighter of the two");
    }

    #[test]
    fn a_dot_that_flickers_every_frame_reads_as_a_steady_shade() {
        // The whole point. A game animates by moving a dot between the two
        // frames; the raw pair of bits goes 3, 0, 3, 0 and a viewer that drew
        // that would strobe. The eye sees a shade, and so does this.
        let mut pwm = Pwm::new();
        for _ in 0..Pwm::TAPS {
            vblank(&mut pwm, true, true);
            vblank(&mut pwm, false, false);
        }
        let mut seen = Vec::new();
        for i in 0..8 {
            vblank(&mut pwm, i % 2 == 0, i % 2 == 0);
            seen.push(i32::from(pwm.luminance()[0]));
        }
        let (lo, hi) = (*seen.iter().min().unwrap(), *seen.iter().max().unwrap());
        assert!(
            hi - lo < 60,
            "a flickering dot should settle to a shade, not swing between {lo} and {hi}"
        );
        assert!(
            lo > 60 && hi < 200,
            "and the shade is a middle one: {lo}..{hi}"
        );
    }
}
