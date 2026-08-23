//! Which ROM images a game needs, and how they sit in the map.
//!
//! Knowing how to emulate a System 11 is not enough to run one. A ROM set is a
//! zip of images with names like `f14_u26.l1`, and where each goes in the 64 KB
//! map differs by family: some games put a 16 KB image at `$4000` and a 32 KB
//! one at `$8000`, others mirror a small image to cover a gap, others use a
//! single 64 KB image. Get it wrong and the CPU runs garbage.
//!
//! The table comes from PinMAME's `s11games.c` and `degames.c`. System 9,
//! System 11 and Data East share one memory map — `s11.c` has a single one —
//! so all three families run on this emulator and all three are listed here.
//!
//! It used to live next to the tests, which was the wrong place: a caller who
//! wants to boot a game needs it as much as a test does.

use crate::DisplayConfig;

/// The manifest, as a tab-separated table. Generated from PinMAME's driver
/// files; see the comments at the top of it.
const MANIFEST: &str = include_str!("../data/rom_sets.tsv");

/// How a game's images are laid out in the CPU's 64 KB map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// 16 KB at `$4000` and 32 KB at `$8000`. The commonest System 11 set.
    Split16At4000,
    /// The same, with the small image mirrored to cover the gap at `$6000`.
    Mirrored8At4000,
    /// 32 KB at `$0000` and 32 KB at `$8000`.
    Split32At0000,
    /// One 32 KB image at `$8000`.
    Single32At8000,
    /// One 64 KB image covering the whole map.
    Single64,
}

impl Layout {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "16400+32-8000" => Layout::Split16At4000,
            "8x4000+32-8000" => Layout::Mirrored8At4000,
            "32-0000+32-8000" => Layout::Split32At0000,
            "32-8000" => Layout::Single32At8000,
            "64-0000" => Layout::Single64,
            _ => return None,
        })
    }

    /// Where each image goes, given the one or two the set has.
    ///
    /// Returns `None` when the layout needs a second image and the set does not
    /// have one, which means the manifest and the zip disagree.
    pub fn place<'a>(
        self,
        first: &'a [u8],
        second: Option<&'a [u8]>,
    ) -> Option<Vec<(usize, &'a [u8])>> {
        Some(match self {
            Layout::Split16At4000 => vec![(0x4000, first), (0x8000, second?)],
            Layout::Mirrored8At4000 => {
                vec![(0x4000, first), (0x6000, first), (0x8000, second?)]
            }
            Layout::Split32At0000 => vec![(0x0000, first), (0x8000, second?)],
            Layout::Single32At8000 => vec![(0x8000, first)],
            Layout::Single64 => vec![(0x0000, first)],
        })
    }
}

use crate::bulb::Drive;

/// One entry of the manifest.
#[derive(Debug, Clone, Copy)]
pub struct Game {
    /// The set's name, as PinMAME and every table's `cGameName` spell it.
    pub set: &'static str,
    /// `S11A`, `S11B`, `DE`, and so on. Informational.
    pub family: &'static str,
    pub display: DisplayConfig,
    pub layout: Layout,
    /// The file names of the one or two main-CPU images inside the zip.
    pub image1: &'static str,
    pub image2: Option<&'static str>,
}

/// The solenoid outputs that drive a bulb rather than a coil, for a set.
///
/// A System 11 board has sixteen solenoid drivers and a game is free to hang
/// whatever it likes off them. Most are coils, but the general illumination
/// strings and the big flashers are bulbs, and the difference matters: the game
/// modulates a bulb — chopping the GI to dim it, ramping a flasher to fade it —
/// where it only ever pulses a coil. A bulb read as a coil reports the
/// modulation as flapping.
///
/// The original keeps this per game from `s11.c:895` onwards, matching on the
/// ROM set name with `strncasecmp`, which is what the prefixes here are. Sets
/// not listed have nothing but coils as far as we know, which is the safe way
/// round: a bulb wrongly called a coil flickers, a coil wrongly called a bulb
/// stops firing.
pub fn bulb_outputs(set: &str) -> &'static [(u8, Drive)] {
    const GI: Drive = Drive::GENERAL_ILLUMINATION;
    const FLASH: Drive = Drive::FLASHER;
    let set = set.to_ascii_lowercase();
    // F-14 Tomcat. `s11.c:985`. Its playfield and backbox GI are 10 and 11;
    // solenoid 14 is the A/C mux relay, a real relay, and the table has the
    // playfield lighting hanging off it.
    if set.starts_with("f14") {
        return &[(9, FLASH), (10, GI), (11, GI), (15, FLASH)];
    }
    // Big Guns. `s11.c:930`.
    if set.starts_with("bguns") {
        return &[(9, GI), (10, GI), (11, GI)];
    }
    // Bad Cats and Bugs Bunny both put the two GI strings on 10 and 11.
    // `s11.c:919` and `s11.c:925`.
    if set.starts_with("bcats") || set.starts_with("bbnny") {
        return &[(10, GI), (11, GI)];
    }
    &[]
}

/// Which solenoid throws the mux relay, and which switches fire the special
/// solenoids, for a set.
///
/// Both come from the same place in the original: the arguments of a game's
/// `INITGAMEFULL` (`s11games.c:38`). The fourth is the mux; the last six are
/// the switches wired straight to special solenoids 17 to 22, with a zero where
/// there is no switch and the CPU drives that one instead.
///
/// F-14 reads `INITGAMEFULL(f14, GEN_S11A, s11_dispS11a, 14, FLIP_SWNO(63,15),
/// 0,0,0,0, 57, 58, 0, 28, 0, 0)`: the mux is solenoid 14, the flipper buttons
/// are switches 63 and 15, switches 57 and 58 fire the two slingshots, switch
/// 28 fires the pop bumper, and solenoids 21 and 22 — the diverters — are left
/// to the CPU.
pub fn wiring(set: &str) -> Wiring {
    let set = set.to_ascii_lowercase();
    if set.starts_with("f14") {
        return Wiring {
            mux: Some(14),
            special_switches: [57, 58, 0, 28, 0, 0],
            flipper_buttons: (63, 15),
        };
    }
    Wiring::default()
}

/// How a game's board is wired up beyond the parts that are the same on every
/// System 11. See [`wiring`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Wiring {
    /// The solenoid that throws the A/C mux relay, if this game has one.
    pub mux: Option<u8>,
    /// The playfield switch that fires each of special solenoids 17 to 22
    /// directly, or 0 where the CPU drives it instead.
    pub special_switches: [u8; 6],
    /// The switches the two flipper buttons close, left then right, from the
    /// same line's `FLIP_SWNO(63,15)`. Zero where the game has none.
    ///
    /// They are read for two different things and both matter. The ROM sees
    /// them as ordinary switches, and *separately* they raise the four
    /// invented flipper solenoids that a table's script hangs the flipper
    /// sound and the flipper itself off — see [`crate::io::FIRST_FLIPPER`].
    pub flipper_buttons: (u8, u8),
}

/// Looks a game up by its set name, without regard to case.
///
/// The name is what a table's script calls `cGameName`, so this is the one
/// function that turns "the table says it is `f14_l1`" into "here is how to
/// build that machine".
pub fn find(set: &str) -> Option<Game> {
    all().find(|g| g.set.eq_ignore_ascii_case(set))
}

/// Every set the manifest knows.
pub fn all() -> impl Iterator<Item = Game> {
    MANIFEST.lines().filter_map(parse_row)
}

fn parse_row(line: &'static str) -> Option<Game> {
    if line.starts_with('#') || line.trim().is_empty() {
        return None;
    }
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 5 {
        return None;
    }
    Some(Game {
        set: f[0],
        family: f[1],
        display: match f[2] {
            "s11b" => DisplayConfig::S11B,
            _ => DisplayConfig::S11A,
        },
        layout: Layout::parse(f[3])?,
        image1: f[4],
        image2: f.get(5).copied().filter(|s| !s.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_has_the_families_it_claims() {
        let games: Vec<Game> = all().collect();
        assert!(
            games.len() > 250,
            "the manifest lists hundreds of sets, not {}",
            games.len()
        );
        for family in ["S11A", "S11B"] {
            assert!(games.iter().any(|g| g.family == family));
        }
    }

    #[test]
    fn a_table_can_be_looked_up_by_the_name_its_script_uses() {
        // `Const cGameName = "f14_l1"` is the line in F-14's script.
        let g = find("f14_l1").expect("F-14 should be in the manifest");
        assert_eq!(g.family, "S11A");
        assert_eq!(g.layout, Layout::Split16At4000);
        assert_eq!(g.image1, "f14_u26.l1");
        assert_eq!(g.image2, Some("f14_u27.l1"));
        // And the case a script writes it in does not matter.
        assert!(find("F14_L1").is_some());
        assert!(find("no_such_game").is_none());
    }

    #[test]
    fn every_row_parses() {
        // A row the parser skips is a game that silently cannot be booted, so
        // the count of rows and the count of games have to agree.
        let rows = MANIFEST
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .count();
        assert_eq!(
            rows,
            all().count(),
            "some rows in the manifest do not parse"
        );
    }

    #[test]
    fn a_layout_that_needs_two_images_says_so() {
        let one = [0u8; 4];
        assert!(Layout::Split16At4000.place(&one, None).is_none());
        assert!(Layout::Single32At8000.place(&one, None).is_some());
    }

    #[test]
    fn the_mirrored_layout_covers_the_gap() {
        let small = [1u8; 4];
        let big = [2u8; 4];
        let places = Layout::Mirrored8At4000.place(&small, Some(&big)).unwrap();
        // The small image appears twice, which is the whole point: without the
        // copy at $6000 the CPU reads nothing there.
        assert_eq!(places.len(), 3);
        assert_eq!(places[0].0, 0x4000);
        assert_eq!(places[1].0, 0x6000);
        assert_eq!(places[2].0, 0x8000);
    }
}
