//! Which lamps switch together, asked of the machine itself.
//!
//! The GI bake (`vpw-render`'s `bake`) needs its lamps in groups that switch
//! as one, and the file does not say which those are: the wiring lives in the
//! ROM. Names and colours guess well on tables that follow convention —
//! `GI_1` red, `GI_2` red — and guess is what it is. The machine, on the
//! other hand, *demonstrates* its wiring every time it runs: lamps on one
//! relay move on the same frame, every frame, for ever.
//!
//! So this runs the game headless for a stretch of attract, samples every
//! candidate lamp's level on a beat, and groups the lamps whose on/off
//! history came out identical. A signature is worth exactly as much as the
//! stretch is long: a pair of strings that happened to move together for the
//! whole observation is one group here, and if the game later splits them the
//! bake is wrong until it is traced again. Attract mode is a good witness —
//! it is the machine showing off every light show it has.

use crate::Game;

/// How often the lamps are sampled, in table milliseconds. Faster than any
/// light show's beat, slower than the script's per-frame housekeeping.
const SAMPLE_MS: u32 = 100;

/// A lamp must reach past this level to count as on. Half, because levels are
/// dimmer positions and what wires lamps together is the switch, not the dim.
const ON: f32 = 0.5;

/// Runs the game for `seconds` of table time and answers which of the
/// candidate lamps switched together, as groups of lamp names.
///
/// The game is really stepped — the ROM boots, attract runs, the script
/// drives the lamps — so this costs real emulation time and belongs next to
/// the bake it feeds, off every thread anyone is holding.
pub fn observe_lamp_groups(
    game: &mut Game,
    candidates: &[String],
    seconds: f32,
) -> Vec<Vec<String>> {
    let steps = (seconds * 1000.0) as u32 / SAMPLE_MS;
    let mut signatures: Vec<Vec<bool>> = vec![Vec::with_capacity(steps as usize); candidates.len()];

    for _ in 0..steps {
        for _ in 0..SAMPLE_MS {
            game.step();
        }
        // The script's frame work, where `SetLamp` writes land coherently:
        // sampling between two of one frame's writes would split a string
        // that the game switches as one.
        game.game_sync();
        for (lamp, signature) in candidates.iter().zip(&mut signatures) {
            let level = game
                .items()
                .get(lamp)
                .map_or(0.0, |item| item.light_level());
            signature.push(level > ON);
        }
    }

    cluster(candidates, &signatures)
}

/// Groups the names whose signatures match exactly.
///
/// Exact, not fuzzy: the sampling lands after the frame's writes, so lamps
/// on one switch really do produce the same bits. Lamps that never came on
/// are dropped — an observation that never saw a lamp move has nothing to say
/// about its wiring, and the runtime point path keeps them honest meanwhile.
fn cluster(names: &[String], signatures: &[Vec<bool>]) -> Vec<Vec<String>> {
    use std::collections::BTreeMap;
    let mut by_signature: BTreeMap<&[bool], Vec<String>> = BTreeMap::new();
    for (name, signature) in names.iter().zip(signatures) {
        if signature.iter().any(|&on| on) {
            by_signature
                .entry(signature.as_slice())
                .or_default()
                .push(name.clone());
        }
    }
    by_signature.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::cluster;

    fn sig(bits: &[u8]) -> Vec<bool> {
        bits.iter().map(|&b| b != 0).collect()
    }

    #[test]
    fn lamps_with_one_history_are_one_group() {
        let names: Vec<String> = ["a", "b", "c", "d"].map(String::from).into();
        let signatures = vec![
            sig(&[1, 1, 0, 1]),
            sig(&[1, 1, 0, 1]),
            sig(&[0, 1, 1, 1]),
            sig(&[1, 1, 0, 1]),
        ];
        let mut groups = cluster(&names, &signatures);
        for g in &mut groups {
            g.sort();
        }
        groups.sort();
        assert_eq!(groups, vec![vec!["a", "b", "d"], vec!["c"]]);
    }

    #[test]
    fn a_lamp_that_never_lit_tells_nothing_and_joins_nothing() {
        let names: Vec<String> = ["on", "off"].map(String::from).into();
        let signatures = vec![sig(&[0, 1]), sig(&[0, 0])];
        let groups = cluster(&names, &signatures);
        assert_eq!(groups, vec![vec!["on"]]);
    }

    /// The trap the sampling design avoids, pinned so nobody reintroduces it:
    /// two lamps of one string sampled mid-write would differ by one bit and
    /// exact matching would split them. The fix is *when* to sample, not how
    /// to compare — this only documents that exact matching assumes it.
    #[test]
    fn one_stray_bit_splits_a_group_which_is_why_sampling_lands_after_the_frame() {
        let names: Vec<String> = ["a", "b"].map(String::from).into();
        let signatures = vec![sig(&[1, 1, 0]), sig(&[1, 1, 1])];
        assert_eq!(cluster(&names, &signatures).len(), 2);
    }
}
