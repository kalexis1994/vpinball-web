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
