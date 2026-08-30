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
//! The **sound card**: a second 6502 with its own ROM, on a card of its own,
//! in both the shapes Gottlieb built it. See [`sound`].
//!
//! The **Sound & Speech board**, a third card again — a 6502, a 6532, two
//! converters and a Votrax speech chip — which about forty of these games
//! carry. See [`speech`] and [`votrax`].
//!
//! A set whose sound card this port does not know comes through with no card
//! at all, which costs the sound and cannot stall the game: the CPU board
//! writes a command to a latch and never waits for an answer.
//!
//! The alphanumeric displays of **System 80B** — two rows of twenty
//! sixteen-segment characters with a Rockwell 10939 in front of each, where
//! the older boards have BCD latches. See [`alpha`]. A set is recognised as an
//! 80B by the one thing that changed underneath: one 8 KB system ROM where the
//! 80A has two of 4 KB.
//!
//! Not here: the **80B sound boards**, which are a different card again in
//! three generations — two more processors with an AY-8913 or a YM2151 — so an
//! 80B set plays silently unless it carries one of the older cards.
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
//! # One trap worth reading before anything else
//!
//! Gottlieb wired its switch matrix with the rows and columns the other way
//! round from everybody else, so a switch number does **not** decompose into
//! the strobe and return you would expect. `sys80.vbs` says the start button
//! is 47 and the first coin is 17, and both are on the same return line. The
//! conversion is in [`io`], and getting it wrong gives you a machine that
//! boots, counts and lights up, and that nobody can put a coin in.
//!
//! Address lines 14 and 15 are not connected, so the whole map repeats four
//! times through the 64 KB. That is not a curiosity to be tidied away: the
//! **stack lives at `$0100`**, which is RIOT 2's RAM, and the CPU's vectors
//! are fetched from `$FFFA`–`$FFFF`, which with A14 and A15 missing is the top
//! of the U3 ROM. A map that does not fold cannot boot.

pub mod alpha;
pub mod board;
pub mod display;
pub mod games;
pub mod io;
pub mod sound;
pub mod speech;
pub mod votrax;

pub use board::{Board, Generation, System80};
pub use sound::SoundBoard;
pub use speech::SpeechBoard;
