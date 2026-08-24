//! An Atmel AT91, which is what makes the noise in a pinball machine from 2003.
//!
//! The processor is in [`vpw_arm7`]; this is everything around it. See
//! [`soc`] for the peripherals and [`board`] for the sound board they sit on.

pub mod board;
pub mod soc;

pub use board::Sound;
