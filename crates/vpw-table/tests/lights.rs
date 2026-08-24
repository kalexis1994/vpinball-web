//! Tests of the light model.

use vpin::vpx::gameitem::light::Light as VpxLight;
use vpw_table::light::build;

/// A circular light of a given radius, with `n` control points.
fn light(state: f32, intensity: f32) -> VpxLight {
    use vpin::vpx::gameitem::dragpoint::DragPoint;
    let points: Vec<DragPoint> = (0..8)
        .map(|i| {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            DragPoint {
                x: 100.0 + 20.0 * a.cos(),
                y: 100.0 + 20.0 * a.sin(),
                z: 0.0,
                smooth: false,
                ..DragPoint::default()
            }
        })
        .collect();

    VpxLight {
        name: "L".into(),
        intensity,
        state: Some(state),
        state_u32: state as u32,
        falloff_radius: 50.0,
        falloff_power: 2.0,
        drag_points: points,
        ..VpxLight::default()
    }
}

#[test]
fn a_light_that_starts_off_is_still_a_light() {
    // It used to be dropped here, on the grounds that nothing would ever turn
    // it on. Something does: in F-14 only 24 of 443 lamps start on, and the
    // other 419 are the game's — the ROM lights them as you play. Dropping them
    // at load gave a playfield that could never light up.
    let off = build(&light(0.0, 1.0), 0.0).expect("kept");
    assert_eq!(off.state, 0.0, "and it knows it is off");
    let on = build(&light(1.0, 1.0), 0.0).expect("kept");
    assert_eq!(on.state, 1.0);
    // Both at full intensity: how bright it is drawn is the state's business,
    // and the state changes several times a second.
    assert_eq!(off.intensity, on.intensity);
}

#[test]
fn a_light_with_no_intensity_is_not_drawn_either() {
    assert!(build(&light(1.0, 0.0), 0.0).is_none());
}

#[test]
fn blinking_is_treated_as_on() {
    // State 2 is "blinking". Without a game clock, it is taken as on: showing it
    // off would be just as arbitrary and it looks worse.
    let l = build(&light(2.0, 1.0), 0.0).expect("it has to be drawn");
    assert!(l.intensity > 0.0);
}

#[test]
fn the_halo_carries_the_whole_intensity() {
    // `light.cpp:798`: lightColor_intensity.w = m_currentIntensity
    //
    // The `* 0.02f` twenty-two lines above it is the **bulb mesh**, a different
    // draw of a different mesh (`light.cpp:776`), and there is no bulb mesh
    // here. Applying it to the halo divides every lamp on a table by fifty,
    // and the way that fails is not "dim": a bulb halo multiplies what is under
    // it by one plus its contribution, so at a fiftieth the multiplier is one
    // and a lit insert is pixel-for-pixel identical to an unlit one.
    //
    // This test used to assert the opposite, on a measurement of F-14 whose
    // mean luminance went from 43 to 111 with the factor removed. That
    // measurement was real and its conclusion was wrong: the bloom was pinned
    // at the engine default of 1.8 instead of the strength the table asks for,
    // so the light was being spread over the table rather than staying where
    // it was. Two bugs, and each one made the other look like the fix.
    let l = build(&light(1.0, 10.0), 0.0).unwrap();
    assert!(
        (l.intensity - 10.0).abs() < 1e-6,
        "it gave {} and 10 was expected",
        l.intensity
    );
}

#[test]
fn an_intermediate_state_is_carried_as_the_state() {
    // From 10.8 onwards the state is a float, and half a light is half as
    // bright. The halving happens where the state can still change — at draw
    // time — so what comes out of here is the state, not a dimmed intensity.
    let full = build(&light(1.0, 10.0), 0.0).unwrap();
    let half = build(&light(0.5, 10.0), 0.0).unwrap();
    assert_eq!(half.state, 0.5);
    assert_eq!(half.intensity, full.intensity);
}

#[test]
fn the_shape_sits_just_above_the_surface() {
    // The original raises it by 0.1 so it does not fight with the playfield over
    // depth (`light.cpp:515`).
    let l = build(&light(1.0, 1.0), 40.0).unwrap();
    assert!(
        l.vertices.iter().all(|v| (v[2] - 40.1).abs() < 1e-4),
        "z badly placed"
    );
    // And the halo's center goes at the surface's height, not the outline's.
    assert!((l.center.z - 40.0).abs() < 1e-4);
}

#[test]
fn the_shape_gets_triangulated() {
    let l = build(&light(1.0, 1.0), 0.0).unwrap();
    assert_eq!(l.indices.len() % 3, 0);
    assert!(!l.indices.is_empty(), "an octagon has to give triangles");
    let max = l.indices.iter().copied().max().unwrap() as usize;
    assert!(max < l.vertices.len(), "index out of range");
}
