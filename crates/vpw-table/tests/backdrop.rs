//! Where the score goes on a backdrop.
//!
//! The painted windows are found by looking at the picture, which is a
//! guess, so the tests are the two ways a guess goes wrong: a window it
//! should see and does not, and a window it sees that is not there.

use vpin::vpx::VPX;
use vpin::vpx::gameitem::GameItemEnum;
use vpin::vpx::gameitem::textbox::TextBox;
use vpw_table::backdrop::{
    FALLBACK_WINDOWS, by_corner, declared, painted, pick_one, quad_of, score_windows,
};
use vpw_table::geometry::Image;

const W: u32 = 900;
const H: u32 = 500;

/// A cheap deterministic noise, so the "art" is not flat anywhere.
fn noise(x: u32, y: u32) -> [u8; 3] {
    let n = x.wrapping_mul(7919) ^ y.wrapping_mul(104_729) ^ (x * y);
    [
        (n & 0xFF) as u8,
        ((n >> 8) & 0xFF) as u8,
        ((n >> 16) & 0xFF) as u8,
    ]
}

/// A backglass: noisy art with framed, flat, teal windows painted on it,
/// each with a lighter band across its top the way a painted highlight is.
fn glass(windows: &[[u32; 4]]) -> Vec<u8> {
    let mut rgba = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            let [r, g, b] = noise(x, y);
            rgba[i..i + 3].copy_from_slice(&[r, g, b]);
            rgba[i + 3] = 255;
        }
    }
    for &[x0, y0, w, h] in windows {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let i = ((y * W + x) * 4) as usize;
                let frame = x < x0 + 2 || x >= x0 + w - 2 || y < y0 + 2 || y >= y0 + h - 2;
                let highlight = y < y0 + h / 3;
                // The highlight is the same paint, brighter: Spider-Man's
                // reads (1,139,178) in the band and (6,130,156) below it.
                let px = if frame {
                    [200, 30, 30]
                } else if highlight {
                    [8, 176, 211]
                } else {
                    [6, 130, 156]
                };
                rgba[i..i + 3].copy_from_slice(&px);
            }
        }
    }
    rgba
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.02)
}

#[test]
fn the_painted_windows_are_found_where_they_are() {
    // Four windows in the corners, as a four-player glass has them: 110 wide
    // and 30 tall on a 900 by 500 picture.
    let rects = [[40, 60, 110, 30], [750, 60, 110, 30], [40, 400, 110, 30], [750, 400, 110, 30]];
    let found = painted(&glass(&rects), W, H);
    assert_eq!(found.len(), 4, "found {found:?}");
    for r in rects {
        let want = [
            r[0] as f32 / W as f32,
            r[1] as f32 / H as f32,
            r[2] as f32 / W as f32,
            r[3] as f32 / H as f32,
        ];
        assert!(
            found.iter().any(|f| close(*f, want)),
            "no window near {want:?} in {found:?}"
        );
    }
}

#[test]
fn art_alone_has_no_windows() {
    assert!(painted(&glass(&[]), W, H).is_empty());
}

#[test]
fn a_flat_wall_is_not_a_window() {
    // A big flat area — a sky — is the wrong shape and the wrong size.
    let found = painted(&glass(&[[0, 0, 900, 250]]), W, H);
    assert!(found.is_empty(), "found {found:?}");
}

#[test]
fn the_corners_say_which_player() {
    let tl = [0.05, 0.1, 0.1, 0.05];
    let tr = [0.85, 0.1, 0.1, 0.05];
    let bl = [0.05, 0.8, 0.1, 0.05];
    let br = [0.85, 0.8, 0.1, 0.05];
    // Handed over in any order, they come back one, two, three, four.
    assert_eq!(
        by_corner(&[br, tl, bl, tr], None),
        vec![Some(tl), Some(tr), Some(bl), Some(br)]
    );
    // Two along the top are players one and two.
    assert_eq!(by_corner(&[tr, tl], None), vec![Some(tl), Some(tr)]);
    // Two down the left are players one and *three*: a window keeps its
    // corner, and the player without one goes without.
    assert_eq!(by_corner(&[bl, tl], None), vec![Some(tl), None, Some(bl)]);
    // One window anywhere is player one.
    let mid = [0.45, 0.5, 0.1, 0.05];
    assert_eq!(by_corner(&[mid], None), vec![Some(mid)]);
    // A fifth is left out: the glass has four players, and the nearer of two
    // claiming a corner wins it.
    let nearer_tl = [0.01, 0.02, 0.1, 0.05];
    assert_eq!(by_corner(&[tl, tr, bl, br, nearer_tl], None)[0], Some(nearer_tl));
}

/// The glass painted twice, the way Spider-Man's is: the inner windows are
/// behind the playfield and the outer ones are the players'.
#[test]
fn a_window_the_table_stands_on_is_passed_over() {
    let outer_l = [0.02, 0.1, 0.1, 0.05];
    let inner_l = [0.38, 0.1, 0.1, 0.05];
    let inner_r = [0.52, 0.1, 0.1, 0.05];
    let outer_r = [0.88, 0.1, 0.1, 0.05];
    let table = Some(quad_of([0.35, 0.0, 0.3, 1.0]));
    assert_eq!(
        by_corner(&[inner_l, outer_r, inner_r, outer_l], table),
        vec![Some(outer_l), Some(outer_r)]
    );
    // Half under the table is still usable; wholly under it is not.
    let half = [0.30, 0.1, 0.1, 0.05];
    assert_eq!(by_corner(&[half], table), vec![Some(half)]);
    // And when the table covers everything, the covered ones are better than
    // nothing.
    assert_eq!(
        by_corner(&[inner_l, inner_r], table),
        vec![Some(inner_l), Some(inner_r)]
    );
    // The fallback strip goes the same way: the first the table leaves alone.
    assert_eq!(pick_one(&[inner_l, outer_r], table), Some(outer_r));
    assert_eq!(pick_one(&[inner_l], table), Some(inner_l));
}

/// From the front the table is a trapezoid, narrow at the far end. A strip
/// across the top corner is clear of it even when the table's bounding box
/// would say otherwise.
#[test]
fn the_table_covers_by_its_outline_not_its_box() {
    let trapezoid = Some([[0.3, 0.0], [0.7, 0.0], [0.9, 1.0], [0.1, 1.0]]);
    let top_left = [0.02, 0.03, 0.26, 0.12];
    let top_centre = [0.37, 0.03, 0.26, 0.12];
    let bottom_left = [0.02, 0.85, 0.26, 0.12];
    assert_eq!(pick_one(&[top_centre, top_left], trapezoid), Some(top_left));
    // The near end is wide, and the bottom strip is under it.
    assert_eq!(pick_one(&[bottom_left, top_left], trapezoid), Some(top_left));
}

#[test]
fn a_textbox_marked_dmd_places_the_display() {
    let mut vpx = VPX::default();
    vpx.gameitems.push(GameItemEnum::TextBox(TextBox {
        name: "Score".into(),
        is_dmd: Some(true),
        ver1: vpin::vpx::gameitem::vertex2d::Vertex2D { x: 250.0, y: 75.0 },
        ver2: vpin::vpx::gameitem::vertex2d::Vertex2D { x: 750.0, y: 150.0 },
        ..TextBox::default()
    }));
    // In the original's 1000 by 750 space (`stdafx.h:43`).
    assert_eq!(declared(&vpx), Some([0.25, 0.1, 0.5, 0.1]));
    let windows = score_windows(&vpx, &[], "bg");
    assert!(windows.over);
    assert_eq!(windows.rects, vec![[0.25, 0.1, 0.5, 0.1]]);
}

/// A backdrop that is black but for a logo in one corner — Circus — puts the
/// strip where the black is.
#[test]
fn the_strip_goes_where_the_picture_is_quiet() {
    let vpx = VPX::default();
    let mut rgba = vec![0u8; (W * H * 4) as usize];
    // A bright logo across the top left.
    for y in 0..(H / 6) {
        for x in 0..(W / 3) {
            let i = ((y * W + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[220, 200, 40, 255]);
        }
    }
    let art = Image {
        name: "bg".into(),
        encoded: None,
        rgba: Some(rgba),
        width: W,
        height: H,
        has_alpha: false,
        alpha_test: -1.0,
        redrawn: false,
    };
    let windows = score_windows(&vpx, &[art], "bg");
    let top_left = FALLBACK_WINDOWS[1];
    assert_ne!(windows.rects[0], top_left, "not across the logo");
    assert_eq!(*windows.rects.last().unwrap(), top_left, "the logo's strip comes last");
}

/// A backdrop that is the glass twice over, the second copy cropped at the
/// edge the way Spider-Man's is, and the copy's width read off it.
#[test]
fn a_repeated_backdrop_gives_up_the_width_of_one_copy() {
    use vpw_table::backdrop::{crop_left, repeat_width};
    // One copy is 560 of the 900 pixels; the rest is the start of another.
    let copy = 560u32;
    let mut rgba = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            let [r, g, b] = noise(x % copy, y);
            rgba[i..i + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
    let found = repeat_width(&rgba, W, H).expect("a repeat");
    assert!((found as i32 - copy as i32).abs() <= 8, "found {found}");
    let (one, w, h) = crop_left(&rgba, W, H, found);
    assert_eq!((w, h, one.len()), (found, H, (found * H * 4) as usize));
    // A white margin between the copies is not part of the glass.
    let mut with_margin = rgba.clone();
    for y in 0..H {
        for x in (copy - 40)..copy {
            let i = ((y * W + x) * 4) as usize;
            with_margin[i..i + 3].copy_from_slice(&[255, 255, 255]);
        }
    }
    let (_, w, _) = crop_left(&with_margin, W, H, copy);
    assert_eq!(w, copy - 40, "the margin is trimmed");
    // And still when the cut lands a few columns past it, into the next
    // copy, the way a coarse repeat search does.
    let (_, w, _) = crop_left(&with_margin, W, H, copy + 8);
    assert_eq!(w, copy - 40, "the margin is found near the cut");
    // Plain noise repeats nowhere.
    assert_eq!(repeat_width(&glass(&[]), W, H), None);
    // And a black picture lines up with itself everywhere, which is not a
    // repeat.
    assert_eq!(repeat_width(&vec![0u8; (W * H * 4) as usize], W, H), None);
}

#[test]
fn a_backdrop_with_nothing_to_say_gets_the_strip() {
    let vpx = VPX::default();
    let art = Image {
        name: "bg".into(),
        encoded: None,
        rgba: Some(glass(&[])),
        width: W,
        height: H,
        has_alpha: false,
        alpha_test: -1.0,
        redrawn: false,
    };
    let windows = score_windows(&vpx, &[art], "bg");
    assert!(windows.over, "the strip is nowhere in the picture");
    assert!(windows.pick_one, "one strip, out of several");
    let mut strips = windows.rects.clone();
    strips.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    let mut all = FALLBACK_WINDOWS.to_vec();
    all.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    assert_eq!(strips, all);
    // And no backdrop at all means no windows: the score is on the head.
    assert!(score_windows(&vpx, &[], "").rects.is_empty());
}
