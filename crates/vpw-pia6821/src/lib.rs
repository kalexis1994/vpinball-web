//! Motorola MC6821 PIA (Peripheral Interface Adapter).
//!
//! It is all of the I/O on a Williams System 11: switches, lamps, solenoids,
//! displays and the handshake with the sound board. A System 11 machine carries
//! **six** of them, with every interrupt line wired together to the 6808's IRQ
//! (`s11.c:121`, `s11.c:832-837` in PinMAME).
//!
//! Each PIA has two independent halves (A and B) and four addressable registers
//! selected by two select lines:
//!
//! | RS1 | RS0 | Register |
//! |-----|-----|----------|
//! | 0 | 0 | Data A, or direction A depending on bit 2 of CRA |
//! | 0 | 1 | Control A |
//! | 1 | 0 | Data B, or direction B depending on bit 2 of CRB |
//! | 1 | 1 | Control B |
//!
//! # The asymmetry between A and B
//!
//! The two halves are not the same, and it matters: the **CA2 handshake fires
//! on a read** of port A, while the **CB2 one fires on a write** to port B.
//! System 11 uses CA2 precisely as the sound handshake and CB2 to enable
//! special solenoids.
//!
//! # Pin model
//!
//! The pins are modelled as the value driven by the chip on the output bits and
//! as the external level on the input ones. On a real 6821, reading port A
//! returns the pin voltage even on the bits programmed as outputs, so external
//! hardware pulling on an output would read itself back; that is not modelled.
//! No System 11 board relies on it.
//!
//! # Usage
//!
//! ```
//! use vpw_pia6821::Pia;
//!
//! let mut pia = Pia::new();
//!
//! // Port B all output: CRB bit 2 at 0 points at the direction register.
//! pia.write(3, 0x00);
//! pia.write(2, 0xFF);
//! // And now bit 2 at 1 points it at the data register.
//! pia.write(3, 0x04);
//! pia.write(2, 0b1010_1010);
//!
//! assert_eq!(pia.port_b_output(), 0b1010_1010);
//! ```

mod port;

pub use port::C2Mode;
use port::{Port, Side};

/// One MC6821.
#[derive(Debug, Clone)]
pub struct Pia {
    a: Port,
    b: Port,
}

impl Default for Pia {
    fn default() -> Self {
        Self::new()
    }
}

impl Pia {
    pub fn new() -> Self {
        Self {
            a: Port::new(Side::A),
            b: Port::new(Side::B),
        }
    }

    /// Reset: every register back to zero.
    pub fn reset(&mut self) {
        self.a.reset();
        self.b.reset();
    }

    // --- Access from the CPU --------------------------------------------------

    /// Reads one of the four registers. `select` is the RS1:RS0 lines (0-3);
    /// the higher bits are ignored, which is what the chip does.
    pub fn read(&mut self, select: u8) -> u8 {
        match select & 0x03 {
            0 => {
                if self.a.data_register_selected() {
                    self.a.read_data()
                } else {
                    self.a.ddr
                }
            }
            1 => self.a.control_for_read(),
            2 => {
                if self.b.data_register_selected() {
                    self.b.read_data()
                } else {
                    self.b.ddr
                }
            }
            _ => self.b.control_for_read(),
        }
    }

    /// Writes one of the four registers.
    pub fn write(&mut self, select: u8, value: u8) {
        match select & 0x03 {
            0 => {
                if self.a.data_register_selected() {
                    self.a.write_data(value);
                } else {
                    self.a.ddr = value;
                }
            }
            1 => self.a.write_control(value),
            2 => {
                if self.b.data_register_selected() {
                    self.b.write_data(value);
                } else {
                    self.b.ddr = value;
                }
            }
            _ => self.b.write_control(value),
        }
    }

    // --- Data pins --------------------------------------------------------

    /// The level the outside world puts on the port A pins.
    pub fn set_port_a_input(&mut self, value: u8) {
        self.a.input = value;
    }

    pub fn set_port_b_input(&mut self, value: u8) {
        self.b.input = value;
    }

    /// What the chip drives outwards on port A. The bits configured as inputs
    /// come out as zero.
    pub fn port_a_output(&self) -> u8 {
        self.a.driven()
    }

    pub fn port_b_output(&self) -> u8 {
        self.b.driven()
    }

    /// Port A direction register: 1 = output.
    pub fn port_a_direction(&self) -> u8 {
        self.a.ddr
    }

    pub fn port_b_direction(&self) -> u8 {
        self.b.ddr
    }

    /// Whether an access to port A sees the data register (bit 2 of CRA) and
    /// not the direction one.
    ///
    /// Needed by anyone who has to tell "the CPU wrote data" apart from "the
    /// CPU reprogrammed the direction", because both go through the same bus
    /// address.
    pub fn port_a_data_selected(&self) -> bool {
        self.a.data_register_selected()
    }

    pub fn port_b_data_selected(&self) -> bool {
        self.b.data_register_selected()
    }

    // --- Control lines ------------------------------------------------------

    pub fn set_ca1(&mut self, level: bool) {
        self.a.set_c1(level);
    }

    pub fn set_cb1(&mut self, level: bool) {
        self.b.set_c1(level);
    }

    /// Only has an effect if CA2 is configured as an input.
    pub fn set_ca2(&mut self, level: bool) {
        self.a.set_c2(level);
    }

    pub fn set_cb2(&mut self, level: bool) {
        self.b.set_c2(level);
    }

    /// CA2's level when it is an output.
    pub fn ca2_output(&self) -> bool {
        self.a.c2_output
    }

    pub fn cb2_output(&self) -> bool {
        self.b.c2_output
    }

    /// Whether CA2 is being driven by the PIA rather than read.
    ///
    /// Anything hanging off the line only hears from it while it is an output;
    /// while it is an input the PIA is listening, not talking.
    pub fn ca2_is_output(&self) -> bool {
        !matches!(self.ca2_mode(), C2Mode::Input { .. })
    }

    pub fn cb2_is_output(&self) -> bool {
        !matches!(self.cb2_mode(), C2Mode::Input { .. })
    }

    pub fn ca2_mode(&self) -> C2Mode {
        self.a.c2_mode()
    }

    pub fn cb2_mode(&self) -> C2Mode {
        self.b.c2_mode()
    }

    /// Consumes the CA2 pulse event, if there was one.
    ///
    /// In pulse mode the line goes down for a single E cycle, shorter than an
    /// instruction, so it is exposed as an event and not as a level.
    pub fn take_ca2_pulse(&mut self) -> bool {
        std::mem::take(&mut self.a.c2_pulsed)
    }

    pub fn take_cb2_pulse(&mut self) -> bool {
        std::mem::take(&mut self.b.c2_pulsed)
    }

    // --- Interrupts -----------------------------------------------------------

    /// Interrupt line of half A.
    pub fn irq_a(&self) -> bool {
        self.a.irq()
    }

    /// Interrupt line of half B.
    pub fn irq_b(&self) -> bool {
        self.b.irq()
    }

    /// Both together. On System 11 every line of all six PIAs is wired to the
    /// same 6808 IRQ.
    pub fn irq(&self) -> bool {
        self.irq_a() || self.irq_b()
    }

    /// Raw interrupt flags of half A: `(C1, C2)`.
    ///
    /// They are bits 7 and 6 of the control register. They get set even when
    /// their interrupt is disabled, so they are not equivalent to
    /// [`Pia::irq_a`].
    pub fn irq_flags_a(&self) -> (bool, bool) {
        (self.a.irq1, self.a.irq2)
    }

    pub fn irq_flags_b(&self) -> (bool, bool) {
        (self.b.irq1, self.b.irq2)
    }
}
