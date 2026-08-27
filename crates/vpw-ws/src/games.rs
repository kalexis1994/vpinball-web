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

/// The five images a Stern sound board is built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sound {
    pub bios: &'static str,
    pub u7: &'static str,
    pub samples: [&'static str; 4],
}

/// Every set this knows.
///
/// A short list on purpose. The board is the same for all of them and adding
/// one is a row, but a row that has not been booted against a real image is a
/// row that claims something nobody checked.
const GAMES: &[Game] = &[
    Game {
        // South Park (Sega 1999): the other end of the Whitestar era. Same
        // main board and display board as LOTR; the sound board is not the
        // AT91 but its ancestor, the Data East BSMT2000 board (`se.c:357`
        // initialises `SNDBRD_DE2S` for this generation) — so the zip has no
        // `bios.u8`, the sound entries below never all match, and the loader
        // takes the silent path until that board exists here.
        set: "sprk_103",
        cpu: "spkcpu",
        display: "spdsp",
        sound: Sound {
            bios: "bios.u8",
            u7: "spku7",
            samples: ["spku17", "spku21", "spku36", "spku37"],
        },
        // `INITGAME(sprk,GEN_WS,se_dmd128x32,0)` — no auxiliary boards
        // (`segames.c:475`).
        boards: crate::board::Boards {
            aux_solenoids: false,
            leds: false,
        },
        // `se.c:361-362`: "sprk_103" keeps the flippers-live flag at zero —
        // PinMAME stores the address plus one so zero can mean "none", and
        // the option does that job here.
        fast_flips: Some(0x0000),
    },
    Game {
        set: "lotr",
        cpu: "lotrcpua",
        display: "lotrdspa",
        sound: Sound {
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
    },
];

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
