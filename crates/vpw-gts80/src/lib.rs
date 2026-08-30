//! Gottlieb's **System 80**, the board behind Ice Fever, Black Hole, Haunted
//! House and about fifty more machines built between 1980 and 1985.
//!
//! It is the smallest machine in this project by a long way. A 6502 at 894 kHz
//! and three 6532 RIOTs, and that is the whole computer: the RIOTs' ports are
//! the switch matrix, the lamps, the solenoids, the displays and the dip
//! switches, their RAM is the scratchpad, and their timer is the interrupt the
//! game runs on. Two hundred and fifty-six bytes of battery-backed RAM hold
//! the audits and the high scores.
//!
//! # What is here and what is not
//!
//! The **CPU board**: the memory map, the three RIOTs, the interrupt, the
//! switch matrix, the lamp matrix, the solenoids, the dip switches and the
//! BCD score displays. That is what makes a game run.
//!
//! Not here: the **sound board**, which is a second 6502 with its own ROM on
//! its own card. The CPU board only ever writes a command to it, so its
//! absence costs the sound and cannot stall the game — the same arrangement
//! the Whitestar's display and sound have next door.
//!
//! Not here either: the alphanumeric displays of **System 80B**, which are
//! driven by a pair of Rockwell 10939 display processors rather than by BCD
//! latches. A 80B game will run on this and show nothing; the two generations
//! differ in that one respect and share everything else.
//!
//! # The memory map (`gts80.c:505-517`)
//!
//! | From | To | What |
//! |------|-----|------|
//! | `$0000` | `$007F` | RIOT 0 RAM (U4) |
//! | `$0080` | `$00FF` | RIOT 1 RAM (U5) |
//! | `$0100` | `$017F` | RIOT 2 RAM (U6) |
//! | `$0200` | `$027F` | RIOT 0 registers |
//! | `$0280` | `$02FF` | RIOT 1 registers |
//! | `$0300` | `$037F` | RIOT 2 registers |
//! | `$1000` | `$17FF` | the game ROM |
//! | `$1800` | `$1FFF` | 256 bytes of battery RAM, mirrored eight times |
//! | `$2000` | `$2FFF` | system ROM U2 |
//! | `$3000` | `$3FFF` | system ROM U3 |
//!
//! Address lines 14 and 15 are not connected, so the whole map repeats four
//! times through the 64 KB. That is not a curiosity to be tidied away: the
//! **stack lives at `$0100`**, which is RIOT 2's RAM, and the CPU's vectors
//! are fetched from `$FFFA`–`$FFFF`, which with A14 and A15 missing is the top
//! of the U3 ROM. A map that does not fold cannot boot.

pub mod board;
pub mod display;
pub mod io;

pub use board::{Board, System80};
