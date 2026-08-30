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
//! The sound ROMs are in the set too, and which ones tells you which of the
//! two cards the machine has: one 2 KB `695-s.snd` is the piggyback, a 1 KB
//! `652.snd` beside a 1 KB `6530sy80.bin` is the older sound card. See
//! [`crate::sound`]. A set carrying neither — the forty-odd games with the
//! Sound & Speech board, which is a different card again — comes through with
//! no sound board, which is a machine that plays silently rather than not at
//! all.

/// The three ROMs a System 80 needs, borrowed from the set.
#[derive(Debug, Clone, Copy)]
pub struct Roms<'a> {
    pub game: &'a [u8],
    pub u2: &'a [u8],
    pub u3: &'a [u8],
    /// The sound card's firmware, if the set carries a card this port knows.
    pub sound: Option<Sound<'a>>,
}

/// Which sound card a set is carrying, and its ROMs.
#[derive(Debug, Clone, Copy)]
pub enum Sound<'a> {
    /// The 1983 piggyback: one 2 KB image holding code and samples both.
    Piggyback(&'a [u8]),
    /// The 1980 sound card: a kilobyte of four-bit samples, and the code in
    /// the 6530's own mask ROM beside it.
    Card { rom: &'a [u8], system: &'a [u8] },
    /// The Sound & Speech board: two 2 KB images side by side in one 4 KB
    /// window, and a Votrax between them and the cabinet.
    Speech { first: &'a [u8], second: &'a [u8] },
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
    let of_size = |size: usize, sound: bool| {
        images
            .iter()
            .find(|(name, data)| {
                let n = name.to_ascii_lowercase();
                data.len() == size && (n.contains("snd") || n.contains("-s.")) == sound
            })
            .map(|(_, data)| data.as_slice())
    };

    // Which card a set carries is written in the shape of its sound images.
    // Two of 2 KB is the Sound & Speech board, whose halves Gottlieb names
    // `s1` and `s2` — and which half is which decides what the machine says,
    // so they are taken in name order rather than in whatever order the zip
    // happened to store them. One of 2 KB is the piggyback. One of 1 KB
    // beside the 6530's own — which every set for the older card carries
    // under the same name, because every one of those cards has the same chip
    // — is the 1980 sound card.
    let mut two_k: Vec<&(String, Vec<u8>)> = images
        .iter()
        .filter(|(name, data)| {
            let n = name.to_ascii_lowercase();
            data.len() == 2048 && (n.contains("snd") || n.contains("-s."))
        })
        .collect();
    two_k.sort_by_key(|(name, _)| name.to_ascii_lowercase());

    let sound = match two_k.as_slice() {
        [first, second, ..] => Some(Sound::Speech {
            first: first.1.as_slice(),
            second: second.1.as_slice(),
        }),
        [only] => Some(Sound::Piggyback(only.1.as_slice())),
        [] => match (of_size(1024, true), named("6530", 1024)) {
            (Some(rom), Some(system)) => Some(Sound::Card { rom, system }),
            _ => None,
        },
    };

    Some(Roms {
        game: of_size(2048, false)?,
        u2,
        u3,
        sound,
    })
}
