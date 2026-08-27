//! Which Whitestar games this knows, and what each one's zip holds.
//!
//! Kept apart from the board because it is a list and not a mechanism, and
//! because it is the file that grows: the board is the same silicon in every
//! one of these games and the differences are which images are in the zip.

/// A Whitestar game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Game {
    /// The set name, which is what a table's script calls `cGameName`.
    pub set: &'static str,
    /// The CPU image inside the zip. The name is matched without regard to
    /// case, and only has to be contained in the entry's.
    pub cpu: &'static str,
    /// The display board's image, for when there is a display board to run it.
    pub display: &'static str,
    /// The sound board's five images: the BIOS every one of these boards
    /// carries, the game's own small one, and four megabytes of samples.
    ///
    /// Matched the same loose way as the rest, so a set whose sample images
    /// are named for the game still finds them.
    pub sound: Sound,
    /// What is on its auxiliary connector. See [`crate::board::Boards`].
    pub boards: crate::board::Boards,
    /// Where in RAM this game keeps the flag that says the flippers are live.
    /// See [`crate::board::Board::fast_flip_addr`].
    pub fast_flips: Option<u16>,
}

/// The images a game's sound board is built from — and which board that is.
///
/// Two generations wore the same connector. Sega and early Stern shipped the
/// BSMT2000 board: a 6809 whose whole firmware is `u7`, no BIOS anywhere.
/// From LOTR on, an AT91 emulates that chip in software and carries the
/// two-megabyte `bios.u8` beside the game's images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    /// The AT91 generation. See [`vpw_at91`] by way of `crate::sound`.
    At91 {
        bios: &'static str,
        u7: &'static str,
        samples: [&'static str; 4],
    },
    /// The BSMT2000 generation. See `crate::de2s`.
    Bsmt {
        u7: &'static str,
        samples: [&'static str; 4],
    },
}

/// The sets that need saying something the zip cannot: overrides.
///
/// Most Whitestar zips are self-describing and go through [`detect`] with no
/// row at all. A row earns its place only when a game carries something no
/// zip reveals — LOTR's auxiliary LED board, say — and then it overrides the
/// detection wholesale.
const GAMES: &[Game] = &[Game {
    set: "lotr",
    cpu: "lotrcpua",
    display: "lotrdspa",
    sound: Sound::At91 {
        bios: "bios.u8",
        u7: "lotr-u7",
        samples: ["lotr-u17", "lotr-u21", "lotr-u36", "lotr-u37"],
    },
    // A 520-5068-01 and the nineteen-LED board that is this game's own
    // (`segames.c:1499`).
    boards: crate::board::Boards {
        aux_solenoids: true,
        leds: true,
    },
    // `MACHINE_INIT(se3)` gives every one of this generation the same address
    // — "It appears all systems of se3 generation are the same... :)"
    // (`se.c:292`) — but it is still a per-game fact, found by hand, so it is
    // listed per game here rather than assumed.
    fast_flips: Some(0x0004),
}];

/// Looks a game up by set name, without regard to case.
pub fn find(set: &str) -> Option<Game> {
    GAMES
        .iter()
        .copied()
        .find(|g| g.set.eq_ignore_ascii_case(set))
}

pub fn all() -> impl Iterator<Item = Game> {
    GAMES.iter().copied()
}

/// The images of a set the zip itself described, borrowed from it.
pub struct Detected<'a> {
    pub cpu: &'a [u8],
    pub display: Option<&'a [u8]>,
    pub sound: Option<DetectedSound<'a>>,
    pub boards: crate::board::Boards,
    pub fast_flips: Option<u16>,
}

/// Which sound board the zip implies, with its images.
pub enum DetectedSound<'a> {
    At91 {
        bios: &'a [u8],
        u7: &'a [u8],
        samples: [&'a [u8]; 4],
    },
    Bsmt {
        u7: &'a [u8],
        samples: [&'a [u8]; 4],
    },
}

/// Reads a Whitestar set out of the zip's own shape, no row required.
///
/// A PinMAME set is self-describing twice over. The names follow convention —
/// `cpu` in the CPU image, `dsp` in the display's, `u7` and `u17/u21/u36/u37`
/// on the sound board — and where a name says nothing, the sizes do: the CPU
/// program is the 128 KB image, the display board's is the 512 KB one, the
/// sound program 64 KB, the samples a megabyte apiece. And the one thing no
/// zip can reveal — where the game keeps its flippers-live flag — is a closed
/// book: the Whitestar era ended in 2004, and PinMAME's catalogue of those
/// addresses (`se.c:361-410`) is complete for ever. `None` when the zip does
/// not look like a Whitestar set at all.
pub fn detect<'a>(set: &str, images: &'a [(String, Vec<u8>)]) -> Option<Detected<'a>> {
    let named = |token: &str| {
        images
            .iter()
            .find(|(name, _)| name.to_ascii_lowercase().contains(token))
            .map(|(_, data)| data.as_slice())
    };
    let sized = |bytes: usize| {
        images
            .iter()
            .find(|(_, data)| data.len() == bytes)
            .map(|(_, data)| data.as_slice())
    };

    let cpu = named("cpu")
        .filter(|i| i.len() <= 0x40000)
        .or(sized(0x20000))?;
    let display = named("dsp").or(named("disp")).or(sized(0x80000));

    // The sound half. `bios.u8` is the AT91 generation's signature; without
    // it, a `u7` means the BSMT board. Samples go by their board positions.
    let bios = named("bios");
    let u7 = named("u7");
    let samples = [named("u17"), named("u21"), named("u36"), named("u37")];
    let sound = match (bios, u7, samples) {
        (Some(bios), Some(u7), [Some(a), Some(b), Some(c), Some(d)]) => Some(DetectedSound::At91 {
            bios,
            u7,
            samples: [a, b, c, d],
        }),
        (None, Some(u7), [Some(a), Some(b), Some(c), Some(d)]) => Some(DetectedSound::Bsmt {
            u7,
            samples: [a, b, c, d],
        }),
        _ => None,
    };

    Some(Detected {
        cpu,
        display,
        sound,
        // No zip reveals an auxiliary board; the games that carry one get an
        // override row above.
        boards: crate::board::Boards {
            aux_solenoids: false,
            leds: false,
        },
        fast_flips: fast_flips(set, bios.is_some()),
    })
}

/// PinMAME's catalogue of flippers-live flag addresses, complete.
///
/// The AT91 generation is uniform — "It appears all systems of se3
/// generation are the same" (`se.c:292`) — and the earlier boards are listed
/// game by game (`se.c:361-410`), matched by the same name prefixes PinMAME
/// matches. PinMAME stores each address plus one so zero can mean "none";
/// the `Option` does that job here. A set in neither list plays with
/// whatever its real flipper solenoids do.
fn fast_flips(set: &str, at91: bool) -> Option<u16> {
    if at91 {
        return Some(0x0004);
    }
    const CATALOG: &[(&str, u16)] = &[
        ("sprk_103", 0x0000),
        ("austin", 0x0000),
        ("monopoly", 0x00f0),
        ("twst_405", 0x014d),
        ("shrkysht", 0x0000),
        ("harl_a30", 0x0000),
        ("hirolcas", 0x0004),
        ("simpprty", 0x0004),
        ("term3", 0x0004),
        ("playboys", 0x0004),
        ("rctycn", 0x0004),
        ("apollo13", 0x0122),
        ("godzilla", 0x0000),
        ("id4", 0x0150),
        ("lostspc", 0x0000),
        ("jplstw22", 0x0000),
        ("spacejam", 0x014d),
        ("swtril43", 0x0000),
        ("vipr_102", 0x0000),
        ("xfiles", 0x0000),
        ("nfl", 0x0000),
        ("startrp", 0x0000),
        ("strikext", 0x0000),
        ("strxt_uk", 0x0000),
    ];
    let lower = set.to_ascii_lowercase();
    CATALOG
        .iter()
        .find(|(prefix, _)| lower.starts_with(prefix))
        .map(|&(_, addr)| addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(name: &str, len: usize) -> (String, Vec<u8>) {
        (name.into(), vec![0u8; len])
    }

    #[test]
    fn a_bsmt_era_zip_describes_itself() {
        // South Park's shape, with its real file names.
        let images = vec![
            image("spdspa.101", 0x80000),
            image("spkcpu.103", 0x20000),
            image("spku17.100", 0x100000),
            image("spku21.100", 0x100000),
            image("spku36.100", 0x100000),
            image("spku37.100", 0x100000),
            image("spku7.101", 0x10000),
        ];
        let d = detect("sprk_103", &images).expect("this is a Whitestar zip");
        assert_eq!(d.cpu.len(), 0x20000);
        assert!(d.display.is_some());
        assert!(matches!(d.sound, Some(DetectedSound::Bsmt { .. })));
        assert_eq!(d.fast_flips, Some(0x0000));
    }

    #[test]
    fn a_bios_makes_it_the_at91_generation() {
        let images = vec![
            image("elvscpu.500", 0x20000),
            image("elvsdspa.500", 0x80000),
            image("bios.u8", 0x200000),
            image("elvis-u7.bin", 0x10000),
            image("elvis-u17.bin", 0x100000),
            image("elvis-u21.bin", 0x100000),
            image("elvis-u36.bin", 0x100000),
            image("elvis-u37.bin", 0x100000),
        ];
        let d = detect("elv_400", &images).expect("a Whitestar zip");
        assert!(matches!(d.sound, Some(DetectedSound::At91 { .. })));
        // The whole se3 generation keeps the flag at four.
        assert_eq!(d.fast_flips, Some(0x0004));
    }

    #[test]
    fn a_zip_that_is_not_whitestar_shaped_says_so() {
        // An S11 zip: small 6808 images, nothing 128 KB.
        let images = vec![image("f14_l1.rom", 0x8000), image("f14_u26.bin", 0x4000)];
        assert!(detect("f14_l1", &images).is_none());
    }

    #[test]
    fn the_catalogue_matches_by_prefix_as_pinmame_does() {
        assert_eq!(fast_flips("term3_301", false), Some(0x0004));
        assert_eq!(fast_flips("MONOPOLY_303", false), Some(0x00f0));
        assert_eq!(fast_flips("unknown_set", false), None);
    }
}
