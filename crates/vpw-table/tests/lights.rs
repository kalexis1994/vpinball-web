//! Tests of the light model.

use vpin::vpx::gameitem::light::{Fader as VpxFader, Light as VpxLight};
use vpw_table::light::{BLINKING, Fader, Lamp, build};

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
fn a_blinking_lamp_is_carried_as_lit_and_flagged() {
    // State 2 is "blinking". It is kept at full brightness with the flag beside
    // it, because what the lamp actually shows is the pattern's business — one
    // is only where the pattern starts.
    let l = build(&light(2.0, 1.0), 0.0).expect("it has to be drawn");
    assert!(l.intensity > 0.0);
    assert_eq!(l.state, 1.0);
    assert!(l.blinking, "the pattern is what makes it a blinker");
}

#[test]
fn a_lamp_the_author_hid_is_not_drawn() {
    // `VSBL`. The original gates the whole lightmap on it (`light.cpp:700`), and
    // tables use it for lamps that exist only for the script to hold state on.
    // Drawing them puts coloured discs on the playfield that no real game has.
    let mut hidden = light(1.0, 1.0);
    hidden.visible = Some(false);
    assert!(build(&hidden, 0.0).is_none());

    // Absent means shown: the chunk is from 10.8 and the original's constructor
    // defaults it to true (`light.h:94`).
    let mut old = light(1.0, 1.0);
    old.visible = None;
    assert!(build(&old, 0.0).is_some());
}

#[test]
fn a_pre_10_8_lamp_that_starts_lit_is_lit() {
    // Before 10.8 the state was an integer in `STAT`; `STTF` did not exist, so
    // `state` is absent and `state_u32` carries it. Reading the float alone and
    // falling back to zero turns off every lamp such a file declares as lit.
    let mut l = light(0.0, 1.0);
    l.state = None;
    l.state_u32 = 1;
    assert_eq!(build(&l, 0.0).expect("kept").state, 1.0);
}

#[test]
fn a_bulb_floats_its_halo_and_a_classic_light_does_not() {
    // `light.cpp:490-494`: a bulb's halo mesh goes at the surface plus its halo
    // height, and only then the 0.1 that keeps it off the surface's depth. A GI
    // light left on the playfield lies flat under every ramp that crosses it and
    // is occluded by them, instead of hanging above the lot.
    let mut bulb = light(1.0, 1.0);
    bulb.is_bulb_light = true;
    bulb.bulb_halo_height = 28.0;
    let l = build(&bulb, 40.0).unwrap();
    assert!(
        l.vertices.iter().all(|v| (v[2] - 68.1).abs() < 1e-4),
        "the halo should float at 40 + 28 + 0.1"
    );

    // A classic insert is artwork on the surface and stays there, halo height or
    // no halo height — the original adds it only for a bulb.
    let mut classic = light(1.0, 1.0);
    classic.bulb_halo_height = 28.0;
    let l = build(&classic, 40.0).unwrap();
    assert!(l.vertices.iter().all(|v| (v[2] - 40.1).abs() < 1e-4));
}

#[test]
fn the_falloff_is_centred_at_the_lamp_s_own_height() {
    // `GetCurrentHeight()` is `m_initSurfaceHeight + m_d.m_height`
    // (`light.h:143`), and it is a different height from the one the outline
    // sits at: `HGHT` lifts the *source* of the light without moving the disc it
    // is painted on, so a raised lamp washes wider instead of spotting tight.
    let mut raised = light(1.0, 1.0);
    raised.height = Some(30.0);
    let l = build(&raised, 40.0).unwrap();
    assert!((l.center.z - 70.0).abs() < 1e-4, "got {}", l.center.z);
    // And the outline is still on the surface.
    assert!(l.vertices.iter().all(|v| (v[2] - 40.1).abs() < 1e-4));
}

#[test]
fn what_a_lamp_transmits_comes_from_the_file() {
    // `TRMS` gates the transmitted-light buffer: zero keeps the lamp out of it
    // altogether (`light.cpp:600`) and anything else scales it (`:801`).
    let mut l = light(1.0, 1.0);
    l.transmission_scale = 0.0;
    assert_eq!(build(&l, 0.0).unwrap().transmission_scale, 0.0);
    l.transmission_scale = 0.75;
    assert_eq!(build(&l, 0.0).unwrap().transmission_scale, 0.75);
}

#[test]
fn a_file_with_no_blink_interval_still_blinks() {
    // `Load` seeds 125 ms before it reads `BINT` (`light.cpp:926`). Carrying the
    // zero a file without the chunk gives would step the pattern every frame,
    // which is not a blink, it is a flicker.
    let mut l = light(2.0, 1.0);
    l.blink_interval = 0;
    assert_eq!(build(&l, 0.0).unwrap().blink_interval, 125.0);
    // And an empty pattern becomes one off frame, the way the original replaces
    // it (`light.cpp:1258`), because the pattern is indexed unconditionally.
    l.blink_pattern = String::new();
    assert_eq!(build(&l, 0.0).unwrap().blink, vec![false]);
}

#[test]
fn a_lamp_ramps_up_at_the_speed_the_file_asks_for() {
    // `light.cpp:322-334`: `m_fadeSpeedUp` is intensity per millisecond, and
    // the ramp is clamped so it cannot overshoot its target. Writing the level
    // straight through instead is what makes every insert on every table snap
    // like a switch.
    let mut off = light(0.0, 10.0);
    off.fade_speed_up = 0.2;
    off.fade_speed_down = 0.1;
    off.fader = Some(VpxFader::Linear);
    let built = build(&off, 0.0).unwrap();
    assert_eq!(built.fader, Fader::Linear);
    let mut lamp = Lamp::new(&built);
    assert_eq!(lamp.level(), 0.0, "the file says off");

    lamp.update(1.0, 1.0, 10.0);
    assert!(
        (lamp.level() - 2.0).abs() < 1e-4,
        "ten milliseconds at 0.2 is 2, not {}",
        lamp.level()
    );
    // Fifty milliseconds in total is the whole way, and no further.
    lamp.update(1.0, 1.0, 100.0);
    assert!((lamp.level() - 10.0).abs() < 1e-4);

    // And down at its own speed, which is half of it here.
    lamp.update(0.0, 1.0, 10.0);
    assert!((lamp.level() - 9.0).abs() < 1e-4, "got {}", lamp.level());
}

#[test]
fn a_fader_of_none_is_still_a_switch() {
    let mut off = light(0.0, 10.0);
    off.fader = Some(VpxFader::None);
    let mut lamp = Lamp::new(&build(&off, 0.0).unwrap());
    lamp.update(1.0, 1.0, 1.0);
    assert_eq!(lamp.level(), 10.0);
}

#[test]
fn a_lamp_with_no_usable_fade_speed_still_gets_there() {
    // A deliberate departure. The original does not clamp `FASP`, so a file
    // that stores zero — or one of the infinities `FASP` is known to carry —
    // leaves the lamp frozen at whatever it was showing for the rest of the
    // game. A dead lamp is a worse answer than an instant one.
    let mut off = light(0.0, 10.0);
    off.fade_speed_up = 0.0;
    let mut lamp = Lamp::new(&build(&off, 0.0).unwrap());
    lamp.update(1.0, 1.0, 16.0);
    assert_eq!(lamp.level(), 10.0);
}

#[test]
fn a_blinking_lamp_walks_its_pattern() {
    // `light.cpp:315` reads one character of `BPAT` per frame of the pattern and
    // `light.h:311` steps the cursor every `BINT` milliseconds. Treating state 2
    // as plainly on — which is what this port did — leaves every blinking lamp
    // on a table permanently lit.
    let mut l = light(2.0, 10.0);
    l.blink_pattern = "1000".into();
    l.blink_interval = 100;
    l.fader = Some(VpxFader::None);
    let built = build(&l, 0.0).unwrap();
    let mut lamp = Lamp::new(&built);
    assert_eq!(lamp.level(), 10.0, "the pattern starts lit");

    // Not yet: the first frame of the pattern lasts a hundred milliseconds.
    lamp.update(BLINKING, 1.0, 50.0);
    assert_eq!(lamp.level(), 10.0);

    // Then three dark frames, one per interval.
    for step in 0..3 {
        lamp.update(BLINKING, 1.0, 100.0);
        assert_eq!(lamp.level(), 0.0, "frame {} of the pattern", step + 1);
    }
    // And round again.
    lamp.update(BLINKING, 1.0, 100.0);
    assert_eq!(lamp.level(), 10.0, "the pattern repeats");
}

#[test]
fn switching_a_lamp_to_blinking_restarts_its_pattern() {
    // `setInPlayState`, `light.cpp:1533`: the cursor goes back to the first
    // frame and the next step is due immediately — "Start pattern right away"
    // (`light.cpp:1543`). A lamp that picked the pattern up wherever the last
    // one left off would blink out of step with every other lamp the game
    // started at the same moment, which is what a bank of chase lights is.
    let mut l = light(1.0, 10.0);
    l.blink_pattern = "1100".into();
    l.blink_interval = 100;
    l.fader = Some(VpxFader::None);
    let built = build(&l, 0.0).unwrap();

    // Two lamps left running on plain for very different lengths of time, then
    // told to blink.
    let (mut early, mut late) = (Lamp::new(&built), Lamp::new(&built));
    early.update(1.0, 1.0, 130.0);
    late.update(1.0, 1.0, 970.0);
    early.update(BLINKING, 1.0, 0.0);
    late.update(BLINKING, 1.0, 0.0);

    let walk = |lamp: &mut Lamp| {
        (0..6)
            .map(|_| {
                let level = lamp.level();
                lamp.update(BLINKING, 1.0, 100.0);
                level
            })
            .collect::<Vec<_>>()
    };
    let (a, b) = (walk(&mut early), walk(&mut late));
    assert_eq!(a, b, "they should be in step whenever they were started");
    // Both one frame in, because the switch makes the first step due at once.
    assert_eq!(a, vec![10.0, 0.0, 0.0, 10.0, 10.0, 0.0]);
}

#[test]
fn an_incandescent_lamp_takes_its_time_and_reddens() {
    // `light.cpp:336-353` into `bulb.cpp`: a #44 filament with a mass and a
    // surface, heated by U²/R and cooled by Stefan-Boltzmann radiation. It is
    // not a curve — it is why a real bulb comes up fast, dies away slowly, and
    // goes orange on the way out.
    let mut l = light(0.0, 10.0);
    l.fader = Some(VpxFader::Incandescent);
    let built = build(&l, 0.0).unwrap();
    assert_eq!(built.fader, Fader::Incandescent);
    let mut lamp = Lamp::new(&built);
    assert_eq!(lamp.level(), 0.0);

    // A cold filament emits nothing for the first instant: it has to get past
    // 1500 K before any of it is visible.
    lamp.update(1.0, 1.0, 1.0);
    let early = lamp.level();
    assert!(early < 10.0, "a bulb does not arrive instantly");

    // Given long enough it settles at full power and stays there.
    for _ in 0..100 {
        lamp.update(1.0, 1.0, 16.0);
    }
    let lit = lamp.level();
    assert!(
        (lit - 10.0).abs() < 0.5,
        "a #44 at 6.3 V should settle at its rating, not {lit}"
    );
    // At full power the tint is the 2700 K reference it is measured against.
    let hot = lamp.tint();
    assert!(
        hot.iter().all(|c| (c - 1.0).abs() < 0.1),
        "a lamp at full power is its own colour: {hot:?}"
    );

    // Switched off it fades rather than stopping, and reddens as it goes:
    // the tint is normalised for luminance, so what changes is the balance.
    lamp.update(0.0, 1.0, 16.0);
    let cooling = lamp.level();
    assert!(
        cooling > 0.0 && cooling < lit,
        "it should be on the way down, not off: {cooling}"
    );
    let cool = lamp.tint();
    assert!(
        cool[0] > hot[0] && cool[2] < hot[2],
        "a cooling filament goes red: {cool:?} against {hot:?}"
    );
}

#[test]
fn a_lamp_starts_where_the_file_leaves_it() {
    // Not at zero with a fade to climb out of: a lamp the file declares as lit
    // is lit on the first frame, the way `RenderSetup` leaves it
    // (`light.cpp:1311`).
    let on = build(&light(1.0, 10.0), 0.0).unwrap();
    assert_eq!(Lamp::new(&on).level(), 10.0);
    let off = build(&light(0.0, 10.0), 0.0).unwrap();
    assert_eq!(Lamp::new(&off).level(), 0.0);
}

#[test]
fn the_dimmer_and_the_switch_are_two_different_numbers() {
    // `targetIntensity = m_d.m_intensity * m_d.m_intensity_scale * lightState`
    // (`light.cpp:316`). A script fading a lamp by hand writes the scale every
    // frame and leaves the switch alone.
    let mut l = light(1.0, 10.0);
    l.fader = Some(VpxFader::None);
    let mut lamp = Lamp::new(&build(&l, 0.0).unwrap());
    lamp.update(1.0, 0.25, 16.0);
    assert_eq!(lamp.level(), 2.5);
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
