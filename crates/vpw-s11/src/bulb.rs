//! Incandescent bulbs, as filaments rather than as switches.
//!
//! A pinball's lamps and its general illumination are not driven the way a
//! naive reading of the hardware suggests. The lamp matrix strobes one column
//! at a time, so every lamp is off for seven eighths of the time; the general
//! illumination is chopped by a triac so the game can dim it. Ask "is the drive
//! high right now" and the answer is a square wave at tens or hundreds of hertz.
//! Ask the bulb and the answer is a steady glow, because a tungsten filament
//! takes tens of milliseconds to heat and to cool and simply cannot follow.
//!
//! Reading the drive instead of the bulb is why the playfield flickers: the
//! game holds the GI on and the picture strobes with it. So this is the bulb.
//!
//! It is a port of PinMAME's `bulb.c`, which in turn follows Dulli Chandra
//! Agrawal's papers on the heating times of tungsten filament lamps. The
//! filament is a resistor whose resistance rises with temperature; current
//! heats it (Ohm), radiation cools it (Planck and Stefan-Boltzmann), and the
//! visible light that comes out is a function of the temperature it settles at.
//! Everything expensive is a lookup table computed once.
//!
//! The numbers in [`Kind`] are measured from real bulbs — see the comment at
//! the end of `bulb.c`, which carries the program used to measure them.

use std::sync::OnceLock;

/// Room temperature, in kelvin, and the coldest a filament is modelled at.
const ROOM: f64 = 293.0;

/// The hottest, a little under tungsten's melting point. `bulb.h:11`.
const MELT: usize = 3400;

/// The coolest temperature that emits any visible light at all.
const GLOW: usize = 1500;

/// How much of the radiated energy hits the filament again rather than leaving,
/// because the filament is a coil rather than a straight wire. From "The Coiling
/// Factor in the Tungsten Filament Lamps", D. C. Agrawal.
const EMISSIVITY_COIL_FACTOR: f64 = 0.6865;

/// What the base and the wires carry away by convection, as a fraction of the
/// power at the bulb's rating. Small enough to be ignored; kept because the
/// original keeps it and the point is to match it.
const BASE_WIRE_LOSS: f64 = 0.07;

/// W·m⁻²·K⁻⁴.
const STEFAN_BOLTZMANN: f64 = 5.670_374_419e-8;

/// Which bulb. The playfield ones are #44 and #47; flashers are #89 and #906.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 6.3 V, 250 mA. Playfield inserts and general illumination. 54 ms to go
    /// from a tenth to nine tenths, 36 ms to fall back.
    N44,
    /// 6.3 V, 150 mA. The other common insert lamp.
    N47,
    /// 6.3 V, 200 mA.
    N86,
    /// 13 V, 577 mA. A flasher.
    N89,
    /// 13 V, 690 mA. The big flasher.
    N906,
}

impl Kind {
    fn index(self) -> usize {
        match self {
            Kind::N44 => 0,
            Kind::N47 => 1,
            Kind::N86 => 2,
            Kind::N89 => 3,
            Kind::N906 => 4,
        }
    }

    /// The voltage the bulb is rated for, which is what it sees when its drive
    /// is on.
    pub fn volts(self) -> f32 {
        TABLES.get_or_init(Tables::build).bulbs[self.index()].rating_u as f32
    }
}

/// What is measured about a bulb, and what is precomputed from it.
struct Characteristics {
    /// Rated voltage.
    rating_u: f64,
    /// The temperature it settles at when run at its rating.
    rating_t: usize,
    /// Filament surface, m².
    surface: f64,
    /// Filament mass, kg.
    mass: f64,
    /// Resistance at room temperature, derived from the three ratings.
    r0: f64,
    /// Kelvin per second lost to radiation and convection, per temperature.
    cool: Vec<f64>,
    /// Kelvin per second gained per volt squared, per temperature.
    heat: Vec<f64>,
}

impl Characteristics {
    /// The filament's resistance at a temperature.
    ///
    /// Linear in temperature — `R0 (1 + 0.0045 (T - T0))` — rather than the
    /// `(T/T0)^1.215` power law an older version used. The linear one is what
    /// the bulbs were measured against, so it is the one that reproduces them.
    fn resistance(&self, t: f64) -> f64 {
        self.r0 * (1.0 + 0.0045 * (t - ROOM))
    }
}

/// Everything computed once: the two temperature-to-light tables and, for each
/// bulb, its heating and cooling curves.
struct Tables {
    /// Luminous flux against temperature, from `GLOW` kelvin upwards.
    emission: Vec<f64>,
    bulbs: Vec<Characteristics>,
}

static TABLES: OnceLock<Tables> = OnceLock::new();

impl Tables {
    fn build() -> Tables {
        // rating_u, rating_i, rating_T, surface, mass — measured, `bulb.c:79`.
        let measured: [(f64, f64, usize, f64, f64); 5] = [
            (6.3, 0.250, 2710, 2.219_161_565_4e-6, 0.311_786_646_6e-6),
            (6.3, 0.150, 2690, 1.387_813_044_7e-6, 0.140_280_115_5e-6),
            (6.3, 0.200, 2550, 2.428_632_148_2e-6, 0.319_691_504_6e-6),
            (13.0, 0.577, 2810, 8.834_759_160_2e-6, 2.067_568_963_9e-6),
            (13.0, 0.690, 2755, 11.660_997_987_0e-6, 3.155_994_017_2e-6),
        ];

        let mut bulbs: Vec<Characteristics> = measured
            .iter()
            .map(|&(rating_u, rating_i, rating_t, surface, mass)| {
                // R0 falls out of the three ratings: at the rated temperature
                // the filament must draw the rated current at the rated
                // voltage, and `resistance` says what R0 that implies.
                let mut c = Characteristics {
                    rating_u,
                    rating_t,
                    surface,
                    mass,
                    r0: 1.0,
                    cool: Vec::new(),
                    heat: Vec::new(),
                };
                c.r0 = (rating_u / rating_i) / c.resistance(rating_t as f64);
                c
            })
            .collect();

        // Visible light against filament temperature, from "Luminous radiation
        // from a black body and the mechanical equivalent of light" by W. W.
        // Coblentz and W. B. Emerson. The four terms are the visible band's
        // share of the Planck curve, fitted; the constant converts the result
        // from W·sr⁻¹·cm⁻² to lumen·sr⁻¹·m⁻².
        let emission: Vec<f64> = (GLOW..=MELT)
            .map(|k| {
                let t = k as f64;
                let p = 1.247 / (1.0 + 129.05 / t).powf(204.0)
                    + 0.0678 / (1.0 + 78.85 / t).powf(404.0)
                    + 0.0489 / (1.0 + 23.52 / t).powf(1004.0)
                    + 0.0406 / (1.0 + 13.67 / t).powf(2004.0);
                p * 68493.150685
            })
            .collect();

        for c in &mut bulbs {
            c.cool = Vec::with_capacity(MELT + 1);
            c.heat = Vec::with_capacity(MELT + 1);
        }
        for k in 0..=MELT {
            let t = k as f64;

            // Tungsten's specific heat, in J·kg⁻¹·K⁻¹, which itself depends on
            // temperature. 45.2268 is the gas constant for tungsten and 310 K
            // its Debye temperature. Agrawal again.
            let specific_heat = 3.0 * 45.2268 * (1.0 - (310.0 * 310.0) / (20.0 * t * t))
                + (2.0 * 4.554_9e-3 * t)
                + (4.0 * 5.778_74e-10 * t * t * t);

            // Tungsten's emissivity across all wavelengths, cut down by the
            // share of radiation the coil catches itself.
            let emissivity = EMISSIVITY_COIL_FACTOR * 0.000_068_9 * t.powf(1.0748);

            for c in &mut bulbs {
                let radiated =
                    -STEFAN_BOLTZMANN * c.surface * emissivity * (t.powi(4) - ROOM.powi(4));
                let conducted =
                    -BASE_WIRE_LOSS * ((t - ROOM) / c.rating_t as f64) / c.resistance(t);
                let mc = c.mass * specific_heat;
                c.cool.push((radiated + conducted) / mc);
                // P = U²/R, with the U² left out so the caller can modulate it.
                c.heat.push((1.0 / c.resistance(t)) / mc);
            }
        }

        Tables { emission, bulbs }
    }
}

/// How much light a filament at a temperature gives off, relative to the same
/// bulb at its rated temperature. One is "as bright as it is meant to be".
///
/// Below `GLOW` kelvin it is warm and dark: a filament emits, but not where an
/// eye can see it.
pub fn emission(kind: Kind, t: f32) -> f32 {
    let tables = TABLES.get_or_init(Tables::build);
    if t < GLOW as f32 {
        return 0.0;
    }
    let c = &tables.bulbs[kind.index()];
    let rated = tables.emission[c.rating_t - GLOW];
    let k = (t as usize).min(MELT) - GLOW;
    (tables.emission[k] / rated) as f32
}

/// Kelvin per second, at a temperature and under a voltage.
///
/// The sum of the two effects, so it is negative when the drive is off or too
/// weak to hold the filament where it is. `serial_r` is any resistance in
/// series with the bulb, which divides the voltage it actually sees; zero for
/// a bulb wired straight across the supply.
pub fn slope(kind: Kind, t: f32, volts: f32, serial_r: f32) -> f32 {
    let tables = TABLES.get_or_init(Tables::build);
    let c = &tables.bulbs[kind.index()];
    let k = (t as usize).clamp(0, MELT);
    let mut u = volts as f64;
    if serial_r != 0.0 {
        let r = c.resistance(t as f64);
        u *= r / (r + serial_r as f64);
    }
    (u * u * c.heat[k] + c.cool[k]) as f32
}

/// How a bulb is wired up.
///
/// The same #44 appears twice on a System 11 with completely different
/// behaviour, because what matters is not the bulb but what it hangs off. In
/// the lamp matrix it is run at eighteen volts through four ohms and strobed
/// for an eighth of the time, which is deliberate: overdriven at a low duty
/// cycle it settles at the same brightness a steady 6.3 volts would give, and
/// gets there faster. On the general illumination string it is run at 6.3 volts
/// of mains AC through a relay, and the relay is wired to invert — the drive
/// being high is the string being *off*.
///
/// These are the two `core_set_pwm_output_type` cases a System 11 uses,
/// `core.c:3074` and `core.c:3115`.
#[derive(Clone, Copy, Debug)]
pub struct Drive {
    pub kind: Kind,
    /// Supply voltage. For AC this is the RMS figure, so the peak is √2 times it.
    pub volts: f32,
    /// Resistance in series with the filament, which divides the voltage the
    /// filament actually sees — and does so by less as the filament heats up
    /// and its own resistance climbs.
    pub serial_r: f32,
    /// Whether the supply is mains AC rather than a DC rail.
    pub alternating: bool,
    /// Whether the drive signal is inverted on its way to the bulb, which is
    /// what a relay in the string does.
    pub reversed: bool,
}

impl Drive {
    /// The lamp matrix. `CORE_MODOUT_BULB_44_18V_DC_S11`, `s11.c:872`.
    pub const LAMP_MATRIX: Drive = Drive {
        kind: Kind::N44,
        volts: 18.0,
        serial_r: 4.3,
        alternating: false,
        reversed: false,
    };

    /// A general illumination string.
    /// `CORE_MODOUT_BULB_44_6_3V_AC_REV`, `s11.c:987`.
    pub const GENERAL_ILLUMINATION: Drive = Drive {
        kind: Kind::N44,
        volts: 6.3,
        serial_r: 0.0,
        alternating: true,
        reversed: true,
    };

    /// A flasher. `CORE_MODOUT_BULB_89_25V_DC_S11`, `s11.c:986`.
    pub const FLASHER: Drive = Drive {
        kind: Kind::N89,
        volts: 25.0,
        serial_r: 0.0,
        alternating: false,
        reversed: false,
    };
}

/// The eye, as a filter.
///
/// A filament reaches equilibrium but it does not sit still: strobed at 125 Hz
/// a lamp still swings by a third, and mains ripple puts a few per cent of
/// 120 Hz on top of every general illumination bulb. Nobody sees either, because
/// an eye integrates. This is where the last of the flicker goes.
///
/// Four cascaded RC low passes, from `core_eye_flicker_fusion` (`core.c:2867`),
/// with a delay of about twenty milliseconds — slow enough to fuse a strobe,
/// fast enough that a flasher still reads as a flash. The coefficient rises
/// with brightness after the Ferry-Porter law: a bright light needs a higher
/// frequency before it fuses, so a bright light is filtered less.
#[derive(Clone, Copy, Debug, Default)]
struct Eye {
    stages: [f32; 3],
    previous: f32,
    value: f32,
}

impl Eye {
    fn see(&mut self, emission: f32) -> f32 {
        // `0.07 + 0.02 log(6v + 1)`, with the log approximated by a square root
        // as the original approximates it. Landing between about 0.07 and 0.11.
        let k = 0.07 + 0.02 * 2.0 * self.value.max(0.0).sqrt();
        let rev = 1.0 - k;
        let old = self.stages;
        self.stages[0] = (k * 0.5) * (emission + self.previous) + rev * old[0];
        self.stages[1] = (k * 0.5) * (self.stages[0] + old[0]) + rev * old[1];
        self.stages[2] = (k * 0.5) * (self.stages[1] + old[1]) + rev * old[2];
        self.value = (k * 0.5) * (self.stages[2] + old[2]) + rev * self.value;
        self.previous = emission;
        self.value
    }
}

/// One bulb's filament, carried through time.
///
/// Feed it the voltage across it and how long that voltage was there, and ask
/// it how bright it is. It starts cold.
#[derive(Clone, Copy, Debug)]
pub struct Filament {
    drive: Drive,
    /// Kelvin.
    temperature: f32,
    /// Seconds since this filament started, which is what the mains phase is
    /// read off. The original counts from the last positive zero crossing of a
    /// clock shared by every output (`core.c:2955`); for a steady sixty hertz
    /// any consistent origin gives the same answer.
    elapsed: f32,
    eye: Eye,
    /// When the filament was last brought up to date.
    last: f32,
    /// The drive it has had since then.
    held: bool,
}

/// How much time may pile up unintegrated before it is worth a step anyway,
/// as a multiple of [`STEP`]. `core.c:2941`.
const LAZY_STEPS: f32 = 4.0;

/// How long a step the integrator takes. `BULB_INTEGRATION_PERIOD`, `core.c:2909`.
const STEP: f32 = 0.001;

/// The most the temperature may move in one step, which keeps the surge of
/// current into a cold filament — its resistance is a sixteenth of its hot
/// resistance — from overshooting when a millisecond is a long step.
const MAX_RISE: f32 = 1000.0;

impl Filament {
    pub fn new(drive: Drive) -> Filament {
        Filament {
            drive,
            temperature: ROOM as f32,
            elapsed: 0.0,
            eye: Eye::default(),
            last: 0.0,
            held: false,
        }
    }

    /// Brings the filament up to `now`, given that the drive has been what it
    /// was since the last call and is `on` from here.
    ///
    /// This is the shape the original uses (`core_update_pwm_output_bulb`,
    /// `core.c:2916`) and it is the reason a lamp matrix can be integrated at
    /// all: rather than stepping every bulb every millisecond, each one is only
    /// touched when its own drive changes, and the time between two changes is
    /// then run off in millisecond steps. A bulb whose drive has not moved and
    /// has not been waiting long is left alone, its time accumulating, because
    /// there is nothing to compute yet.
    pub fn integrate(&mut self, now: f32, on: bool) {
        let dt = (now - self.last).max(0.0);
        let steps = if on != self.held {
            // The drive flipped, so the time up to the flip has to be spent
            // before the new state can begin. At least one step, even for a
            // pulse shorter than a step: dropping it would lose the energy.
            (dt / STEP).floor().max(1.0)
        } else if dt >= LAZY_STEPS * STEP {
            // Nothing changed but enough has piled up to be worth spending.
            (dt / STEP).floor()
        } else {
            // Leave `last` where it is, so this time is not lost — it will be
            // spent along with whatever comes next.
            return;
        };
        self.run(self.held, dt / steps, steps as usize);
        self.last = now;
        self.held = on;
    }

    /// Runs the filament for `dt` seconds with the drive signal `on` or off.
    ///
    /// `on` is the logic level, not the voltage: what the relay or the strobe
    /// driver is being told. Whether that means current through the filament is
    /// [`Drive::reversed`]'s business.
    pub fn advance(&mut self, on: bool, dt: f32) {
        let whole = (dt / STEP).floor() as usize;
        self.run(on, STEP, whole);
        let rest = dt - whole as f32 * STEP;
        if rest > 0.0 {
            self.run(on, rest, 1);
        }
        self.last += dt;
        self.held = on;
    }

    /// `count` steps of `step` seconds each, with the drive held.
    fn run(&mut self, on: bool, step: f32, count: usize) {
        let energised = on != self.drive.reversed;
        for _ in 0..count {
            self.temperature = self.temperature.clamp(ROOM as f32, MELT as f32);
            let volts = if !energised {
                0.0
            } else if self.drive.alternating {
                // The instantaneous mains voltage. The filament sees the
                // magnitude of a sine at twice mains frequency, which at 120 Hz
                // it cannot begin to follow — a real GI bulb ripples by a few
                // per cent and that is all.
                std::f32::consts::SQRT_2
                    * (std::f32::consts::TAU * 60.0 * self.elapsed).sin()
                    * self.drive.volts
            } else {
                self.drive.volts
            };
            let rise = step
                * slope(
                    self.drive.kind,
                    self.temperature,
                    volts,
                    self.drive.serial_r,
                );
            self.temperature += rise.min(MAX_RISE);
            self.eye.see(emission(self.drive.kind, self.temperature));
            self.elapsed += step;
        }
    }

    /// How bright it looks, where one is the bulb run at its rating.
    ///
    /// What an eye makes of the filament, not what the filament is doing this
    /// microsecond — see [`Eye`]. This is the number to draw with.
    pub fn brightness(&self) -> f32 {
        self.eye.value
    }

    /// What the filament is actually emitting, before the eye gets to it.
    pub fn emission(&self) -> f32 {
        emission(self.drive.kind, self.temperature)
    }

    /// Kelvin, for anything that wants the colour rather than the brightness.
    pub fn temperature(&self) -> f32 {
        self.temperature
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bulb on its own rating, for the tests that are about the filament
    /// rather than about how a System 11 wires it.
    fn rated(kind: Kind) -> Drive {
        Drive {
            kind,
            volts: kind.volts(),
            serial_r: 0.0,
            alternating: false,
            reversed: false,
        }
    }

    /// The rise and fall times in the comments beside `bulb.c:79`, which are
    /// the whole reason the model is here: 54 ms and 36 ms for a #44 is longer
    /// than a frame, so a lamp strobed at 8 ms cannot follow its drive.
    #[test]
    fn a_44_takes_about_fifty_milliseconds_to_come_up() {
        let mut f = Filament::new(rated(Kind::N44));
        let (mut tenth, mut ninety) = (None, None);
        for i in 0..400 {
            f.advance(true, 0.001);
            let b = f.emission();
            if tenth.is_none() && b >= 0.1 {
                tenth = Some(i);
            }
            if ninety.is_none() && b >= 0.9 {
                ninety = Some(i);
            }
        }
        let rise = ninety.expect("it should reach nine tenths") - tenth.expect("and a tenth");
        assert!(
            (44..=64).contains(&rise),
            "a #44 rises in about 54 ms, not {rise}"
        );
    }

    #[test]
    fn and_about_thirty_six_to_go_out() {
        let mut f = Filament::new(rated(Kind::N44));
        f.advance(true, 1.0);
        let (mut ninety, mut tenth) = (None, None);
        for i in 0..400 {
            f.advance(false, 0.001);
            let b = f.emission();
            if ninety.is_none() && b <= 0.9 {
                ninety = Some(i);
            }
            if tenth.is_none() && b <= 0.1 {
                tenth = Some(i);
            }
        }
        let fall = tenth.expect("it should go out") - ninety.expect("having been bright");
        assert!(
            (28..=46).contains(&fall),
            "a #44 falls in about 36 ms, not {fall}"
        );
    }

    #[test]
    fn at_its_rating_it_settles_at_one() {
        let mut f = Filament::new(rated(Kind::N44));
        f.advance(true, 1.0);
        let b = f.brightness();
        assert!((b - 1.0).abs() < 0.05, "rated brightness is one, not {b}");
        assert!(
            (f.emission() - 1.0).abs() < 0.05,
            "and so is the filament's"
        );
    }

    #[test]
    fn a_cold_filament_is_dark() {
        let f = Filament::new(rated(Kind::N44));
        assert_eq!(f.brightness(), 0.0);
        assert_eq!(f.emission(), 0.0);
    }

    /// The big flashers are slower than the inserts, which is why a flasher
    /// pulse reads as a fade rather than a blink.
    #[test]
    fn a_906_is_slower_than_a_44() {
        let mut small = Filament::new(rated(Kind::N44));
        let mut big = Filament::new(rated(Kind::N906));
        small.advance(true, 0.03);
        big.advance(true, 0.03);
        assert!(
            small.emission() > big.emission(),
            "after 30 ms the small bulb is ahead: {} vs {}",
            small.emission(),
            big.emission()
        );
    }

    /// The point of the whole file, and the reason the matrix is overdriven.
    ///
    /// A lamp in an eight column matrix is energised for an eighth of the time.
    /// Run at its own 6.3 volts that would leave it dim; run at eighteen through
    /// four ohms, as a System 11 runs it, it reaches full brightness and holds
    /// it through the dark seven eighths.
    #[test]
    fn a_strobed_lamp_glows_steadily_and_brightly() {
        let mut f = Filament::new(Drive::LAMP_MATRIX);
        // One millisecond a column, eight columns, which is about how fast a
        // System 11 goes round. Half a second to reach equilibrium.
        let sweep = |f: &mut Filament| {
            f.advance(true, 0.001);
            f.advance(false, 0.007);
        };
        for _ in 0..64 {
            sweep(&mut f);
        }
        let (mut low, mut high) = (f32::MAX, 0.0f32);
        for _ in 0..8 {
            f.advance(true, 0.001);
            low = low.min(f.brightness());
            high = high.max(f.brightness());
            f.advance(false, 0.007);
            low = low.min(f.brightness());
            high = high.max(f.brightness());
        }
        assert!(
            low > 0.5,
            "an insert is properly lit, not merely glowing: {low}"
        );
        assert!(
            high - low < 0.02,
            "and it does not visibly strobe: {low}..{high}"
        );
    }

    /// General illumination, which is the case that made the playfield blink.
    ///
    /// The relay inverts, so the drive going *low* is the string coming on, and
    /// the supply is mains AC — the filament is riding a 120 Hz ripple on top of
    /// whatever the game does with the relay. Neither shows.
    #[test]
    fn general_illumination_does_not_flicker() {
        let mut f = Filament::new(Drive::GENERAL_ILLUMINATION);
        for _ in 0..30 {
            f.advance(false, 1.0 / 60.0);
        }
        let (mut low, mut high) = (f32::MAX, 0.0f32);
        for _ in 0..120 {
            f.advance(false, 1.0 / 600.0);
            low = low.min(f.brightness());
            high = high.max(f.brightness());
        }
        assert!(low > 0.5, "the string is lit: {low}");
        assert!(
            high - low < 0.02,
            "and the mains ripple does not show: {low}..{high}"
        );
    }

    /// And the relay really is the wrong way round: holding the drive high is
    /// how the game turns the general illumination *off*.
    #[test]
    fn the_general_illumination_relay_is_inverted() {
        let mut f = Filament::new(Drive::GENERAL_ILLUMINATION);
        f.advance(true, 1.0);
        assert_eq!(f.brightness(), 0.0, "drive high is the string switched out");
    }
}
