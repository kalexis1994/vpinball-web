//! Recognising a System 80 ROM set by its shape.
//!
//! There is no table of games here and there does not need to be one. Every
//! System 80 set is the same three ROMs: a **2 KB game ROM** named for the
//! game's own number, and **two 4 KB system ROMs** shared by every machine of
//! the generation and named for it. Ice Fever's set is `695.cpu`, `u2_80a.bin`
//! and `u3_80a.bin`, and so is every other 80A game's but for the first name.
//!
//! That shape is distinctive enough to recognise on its own, which is worth
//! more than a list: a list is right about the games somebody typed in and
//! silent about the rest, and there are fifty-odd of these.
//!
//! The sound ROM is in the set too — `695-s.snd`, also 2 KB — and is left for
//! the sound board to pick up when there is one.

/// The three ROMs a System 80 needs, borrowed from the set.
#[derive(Debug, Clone, Copy)]
pub struct Roms<'a> {
    pub game: &'a [u8],
    pub u2: &'a [u8],
    pub u3: &'a [u8],
    /// The sound board's ROM, if the set carries one.
    pub sound: Option<&'a [u8]>,
}

/// Whether a set of images looks like a System 80, and which ROM is which.
///
/// Deliberately strict about the two system ROMs: they have to be named, not
/// merely the right size. A set with two anonymous 4 KB images could be
/// anything, and guessing which was U2 boots a machine that runs its own ROM
/// backwards — which looks like a broken CPU rather than a misread set.
pub fn detect(images: &[(String, Vec<u8>)]) -> Option<Roms<'_>> {
    let named = |token: &str, size: usize| {
        images
            .iter()
            .find(|(name, data)| data.len() == size && name.to_ascii_lowercase().contains(token))
            .map(|(_, data)| data.as_slice())
    };

    let u2 = named("u2", 4096)?;
    let u3 = named("u3", 4096)?;

    // The game ROM is the 2 KB image that is not the sound board's. Gottlieb
    // names the sound one after the game with an `-s` on it, so the two sit
    // next to each other in a listing and are told apart by that alone.
    let two_k = |sound: bool| {
        images
            .iter()
            .find(|(name, data)| {
                let n = name.to_ascii_lowercase();
                data.len() == 2048 && (n.contains("snd") || n.contains("-s.")) == sound
            })
            .map(|(_, data)| data.as_slice())
    };

    Some(Roms {
        game: two_k(false)?,
        u2,
        u3,
        sound: two_k(true),
    })
}
