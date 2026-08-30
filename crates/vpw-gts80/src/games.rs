//! Recognising a System 80 ROM set by its shape.
//!
//! There is almost no table of games here and there does not need to be one.
//! Every System 80 set is the same three ROMs: a **2 KB game ROM** named for
//! the game's own number, and **two 4 KB system ROMs** shared by every machine
//! of the generation and named for it. Ice Fever's set is `695.cpu`,
//! `u2_80a.bin` and `u3_80a.bin`, and so is every other 80A game's but for the
//! first name. A **System 80B** is the same idea with one 8 KB system ROM
//! where those two were.
//!
//! That shape is distinctive enough to recognise on its own, which is worth
//! more than a list: a list is right about the games somebody typed in and
//! silent about the rest, and there are a hundred and fifty of these.
//!
//! # The sound cards, of which there are five
//!
//! Their images are in the set too, and the shape says which card:
//!
//! | Images | Card |
//! |--------|------|
//! | one 2 KB `-s.snd` | the 1983 piggyback ([`crate::sound`]) |
//! | one 1 KB `.snd` and `6530sy80.bin` | the 1980 sound card |
//! | two 2 KB `-s1`/`-s2.snd` | the Sound & Speech board ([`crate::speech`]) |
//! | `drom*.snd` and `yrom*.snd` | a System 80B card ([`crate::sound80b`]) |
//!
//! The last of those is the only one that names its images for the processor
//! they belong to, which is what makes it easy: `d` for the one driving the
//! converter, `y` for the one making the music.

/// The ROMs a System 80 needs, borrowed from the set.
#[derive(Debug, Clone, Copy)]
pub struct Roms<'a> {
    /// The game's own ROM. Empty on the System 80B sets that hold the whole
    /// game in the 8 KB image instead.
    pub game: &'a [u8],
    /// The system ROM: two 4 KB images on a System 80 or 80A, one of 8 KB on
    /// an 80B. Either way it lands at `$2000`.
    pub system: System<'a>,
    /// The sound card's firmware, if the set carries a card this port knows.
    pub sound: Option<Sound<'a>>,
}

/// Which generation a set is, told by the shape of its system ROM.
#[derive(Debug, Clone, Copy)]
pub enum System<'a> {
    /// System 80 and 80A: `u2_80a.bin` and `u3_80a.bin`, 4 KB each, and a
    /// score display of BCD latches.
    Bcd { u2: &'a [u8], u3: &'a [u8] },
    /// System 80B: one 8 KB image where those two were, and two rows of
    /// twenty characters instead of the latches.
    Alphanumeric(&'a [u8]),
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
    /// One of the three System 80B cards: a processor for the music and
    /// another for the converter, each with its own ROM.
    Card80B {
        generation: crate::sound80b::Generation,
        /// The music processor's images. The first lands at the top of its
        /// memory and the second, where there is one, below it.
        y: (&'a [u8], Option<&'a [u8]>),
        /// The converter processor's.
        d: &'a [u8],
    },
}

/// The System 80B games whose sound card has the FM chip rather than the two
/// square-wave ones.
///
/// The second and third cards carry ROMs of the same shape and differ only in
/// which chip is soldered on, so nothing in a set says which it is. Hence the
/// one list in this file, and it is short: five titles and their regional
/// variants, from the machines `gts80games.c` builds on `gl_mGTS80BS3`.
#[rustfmt::skip]
const FM_GAMES: [&str; 31] = [
    "amaz3afp", "amazn3fp", "amazon3a", "amazonh3",
    "badgirl2", "badgirlf", "badgirlg", "badgirls", "badgrffp", "badgrgfp", "badgrlfp",
    "bighosfp", "bighouse", "bighousf", "bighousg", "bighsffp", "bighsgfp",
    "bonebffp", "bonebgfp", "bonebsfp", "bonebstf", "bonebstg", "bonebstr",
    "hotshffp", "hotshgfp", "hotshotf", "hotshotg", "hotshots", "hotshtfp",
    "nmoves", "nmovesfp",
];

/// Whether a name looks like a sound image rather than a game one.
fn is_sound(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("snd") || n.contains("-s.")
}

/// Whether a set of images looks like a System 80, and which ROM is which.
///
/// Deliberately strict about the 80 and 80A's two system ROMs: they have to be
/// named, not merely the right size. A set with two anonymous 4 KB images could
/// be anything, and guessing which was U2 boots a machine that runs its own ROM
/// backwards — which looks like a broken CPU rather than a misread set.
///
/// The set name is used for exactly one thing: telling the second System 80B
/// sound card from the third, which nothing in the images can.
pub fn detect<'a>(set: &str, images: &'a [(String, Vec<u8>)]) -> Option<Roms<'a>> {
    let named = |token: &str, size: usize| {
        images
            .iter()
            .find(|(name, data)| data.len() == size && name.to_ascii_lowercase().contains(token))
            .map(|(_, data)| data.as_slice())
    };

    // The game and system ROMs are the images that are *not* the sound card's.
    // Gottlieb names a sound one after the game with an `-s` on it, so the two
    // sit next to each other in a listing and are told apart by that alone.
    let of_size = |size: usize| {
        images
            .iter()
            .find(|(name, data)| data.len() == size && !is_sound(name))
            .map(|(_, data)| data.as_slice())
    };

    let sound = sound_card(set, images);

    // A System 80B: one 8 KB system image, and a game ROM of 2 KB or 4 KB
    // beside it — or none at all on the sets whose whole game is in the 8 KB.
    if let Some(system) = of_size(8192) {
        return Some(Roms {
            game: of_size(2048).or_else(|| of_size(4096)).unwrap_or(&[]),
            system: System::Alphanumeric(system),
            sound,
        });
    }

    Some(Roms {
        game: of_size(2048)?,
        system: System::Bcd {
            u2: named("u2", 4096)?,
            u3: named("u3", 4096)?,
        },
        sound,
    })
}

/// Which sound card the set carries, if it is one this port knows.
fn sound_card<'a>(set: &str, images: &'a [(String, Vec<u8>)]) -> Option<Sound<'a>> {
    // A System 80B card names its images for the processor they belong to,
    // which no other card does, so it is worth asking about first.
    let by_prefix = |prefix: &str| {
        let mut found: Vec<&'a (String, Vec<u8>)> = images
            .iter()
            .filter(|(name, _)| {
                let n = name.to_ascii_lowercase();
                is_sound(&n) && n.starts_with(prefix)
            })
            .collect();
        // In name order, because `yrom1` sits above `yrom2` in the processor's
        // memory and getting them the other way round is a card that runs its
        // own firmware backwards.
        found.sort_by_key(|(name, _)| name.to_ascii_lowercase());
        found
    };

    let (y, d) = (by_prefix("yrom"), by_prefix("drom"));
    if let [d_rom, ..] = d.as_slice() {
        let generation = if d_rom.1.len() > 0x2000 {
            // The bigger images are the later cards, and only a list can say
            // which of those two this is.
            if FM_GAMES.contains(&set) {
                crate::sound80b::Generation::Three
            } else {
                crate::sound80b::Generation::Two
            }
        } else {
            crate::sound80b::Generation::One
        };
        // Amazon Hunt II has a converter processor and no music one at all,
        // which is a card with nothing in half of it rather than a broken set.
        let y = match y.as_slice() {
            [first, second, ..] => (first.1.as_slice(), Some(second.1.as_slice())),
            [first] => (first.1.as_slice(), None),
            [] => (&[][..], None),
        };
        return Some(Sound::Card80B {
            generation,
            y,
            d: d_rom.1.as_slice(),
        });
    }

    // The older cards, by the shape of what they carry. Two 2 KB images is the
    // Sound & Speech board, whose halves Gottlieb names `s1` and `s2` — and
    // which half is which decides what the machine says, so they are taken in
    // name order rather than in whatever order the zip happened to store them.
    // One of 2 KB is the piggyback. One of 1 KB beside the 6530's own — which
    // every set for the older card carries under the same name, because every
    // one of those cards has the same chip — is the 1980 sound card.
    let mut two_k: Vec<&'a (String, Vec<u8>)> = images
        .iter()
        .filter(|(name, data)| data.len() == 2048 && is_sound(name))
        .collect();
    two_k.sort_by_key(|(name, _)| name.to_ascii_lowercase());

    match two_k.as_slice() {
        [first, second, ..] => Some(Sound::Speech {
            first: first.1.as_slice(),
            second: second.1.as_slice(),
        }),
        [only] => Some(Sound::Piggyback(only.1.as_slice())),
        [] => {
            let rom = images
                .iter()
                .find(|(name, data)| data.len() == 1024 && is_sound(name))
                .map(|(_, data)| data.as_slice())?;
            let system = images
                .iter()
                .find(|(name, data)| {
                    data.len() == 1024 && name.to_ascii_lowercase().contains("6530")
                })
                .map(|(_, data)| data.as_slice())?;
            Some(Sound::Card { rom, system })
        }
    }
}
