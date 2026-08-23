//! One of the PIA's two halves. A and B are almost identical.

/// Which of the two ports this is. It matters because the 6821 is not
/// symmetric: the CA2 handshake fires on a **read** of port A, and the CB2 one
/// on a **write** to port B.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    A,
    B,
}

/// What the C2 line (CA2 / CB2) does, according to bits 5-3 of the control
/// register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum C2Mode {
    /// Input. The interrupt flag is set by the active transition.
    Input {
        /// `true` = rising edge, `false` = falling edge.
        rising_edge: bool,
        /// Whether the active transition raises an interrupt.
        irq_enabled: bool,
    },
    /// Output in handshake mode: goes down on a port access and comes back up
    /// on the next active transition of C1.
    Handshake,
    /// Output in pulse mode: a one-E-cycle low pulse on a port access.
    Pulse,
    /// Manual output: the line follows bit 3 of the control register.
    Manual,
}

#[derive(Debug, Clone)]
pub(crate) struct Port {
    side: Side,

    /// Data direction register. 1 = output, 0 = input.
    pub(crate) ddr: u8,
    /// Output register (ORA / ORB).
    pub(crate) output: u8,
    /// Control register, bits 0-5. Bits 6 and 7 are the flags and live apart
    /// because they are read-only.
    pub(crate) control: u8,

    /// C1's interrupt flag (bit 7 of the control register).
    pub(crate) irq1: bool,
    /// C2's interrupt flag (bit 6). Only set if C2 is an input.
    pub(crate) irq2: bool,

    /// The levels the outside world puts on the pins.
    pub(crate) input: u8,
    /// Current level of the C1 line, to detect transitions.
    c1_level: bool,
    /// Current level of the C2 line when it is an input.
    c2_level: bool,
    /// The level driven on the C2 line when it is an output.
    pub(crate) c2_output: bool,
    /// There was a pulse on C2 since the last time it was polled.
    pub(crate) c2_pulsed: bool,
}

impl Port {
    pub(crate) fn new(side: Side) -> Self {
        Self {
            side,
            ddr: 0,
            output: 0,
            control: 0,
            irq1: false,
            irq2: false,
            input: 0,
            c1_level: false,
            c2_level: false,
            c2_output: false,
            c2_pulsed: false,
        }
    }

    pub(crate) fn reset(&mut self) {
        let side = self.side;
        *self = Self::new(side);
    }

    // --- Control register decoding ------------------------------

    /// Bit 0: the active transition of C1 raises an interrupt.
    fn c1_irq_enabled(&self) -> bool {
        self.control & 0x01 != 0
    }

    /// Bit 1: `true` = rising edge.
    fn c1_rising_edge(&self) -> bool {
        self.control & 0x02 != 0
    }

    /// Bit 2: `true` = the port access sees the data register;
    /// `false` = it sees the direction register.
    pub(crate) fn data_register_selected(&self) -> bool {
        self.control & 0x04 != 0
    }

    /// Bits 5-3.
    pub(crate) fn c2_mode(&self) -> C2Mode {
        if self.control & 0x20 == 0 {
            C2Mode::Input {
                rising_edge: self.control & 0x10 != 0,
                irq_enabled: self.control & 0x08 != 0,
            }
        } else if self.control & 0x10 != 0 {
            C2Mode::Manual
        } else if self.control & 0x08 != 0 {
            C2Mode::Pulse
        } else {
            C2Mode::Handshake
        }
    }

    /// The control register as the CPU reads it: bits 0-5 plus the two
    /// interrupt flags on top.
    ///
    /// Bit 6 (C2's flag) reads zero while C2 is configured as an output, even
    /// if the flag was set earlier. That is what the datasheet says, and it
    /// has to be masked explicitly: if the software sets the flag with C2 as
    /// an input and then reconfigures it as an output, the flag stays stored
    /// but stops being visible.
    pub(crate) fn control_for_read(&self) -> u8 {
        let c2_is_input = matches!(self.c2_mode(), C2Mode::Input { .. });
        let irq2_visible = self.irq2 && c2_is_input;
        (self.control & 0x3F) | (u8::from(self.irq1) << 7) | (u8::from(irq2_visible) << 6)
    }

    /// The interrupt line the PIA presents to the CPU.
    pub(crate) fn irq(&self) -> bool {
        let from_c1 = self.irq1 && self.c1_irq_enabled();
        let from_c2 = match self.c2_mode() {
            // As an output, C2 raises no interrupts.
            C2Mode::Input { irq_enabled, .. } => self.irq2 && irq_enabled,
            _ => false,
        };
        from_c1 || from_c2
    }

    // --- Access from the CPU --------------------------------------------------

    /// Read of the data port. Clears both interrupt flags.
    pub(crate) fn read_data(&mut self) -> u8 {
        self.irq1 = false;
        self.irq2 = false;

        // The CA2 handshake fires on a read of port A.
        if self.side == Side::A {
            self.trigger_c2_output();
        }

        self.pin_levels()
    }

    /// Write to the output register.
    pub(crate) fn write_data(&mut self, value: u8) {
        self.output = value;

        // The CB2 one, on a write to port B.
        if self.side == Side::B {
            self.trigger_c2_output();
        }
    }

    /// Write to the control register. Bits 6 and 7 are read-only.
    pub(crate) fn write_control(&mut self, value: u8) {
        self.control = value & 0x3F;

        // In manual mode the line follows bit 3 right away.
        if self.c2_mode() == C2Mode::Manual {
            self.c2_output = self.control & 0x08 != 0;
        }
    }

    /// What is read on the pins: the output register on the bits configured as
    /// outputs, the external level on the input ones.
    pub(crate) fn pin_levels(&self) -> u8 {
        (self.output & self.ddr) | (self.input & !self.ddr)
    }

    /// What the chip is driving outwards.
    pub(crate) fn driven(&self) -> u8 {
        self.output & self.ddr
    }

    // --- Control lines ----------------------------------------------------

    /// Changes C1's level and sets the flag if the transition is the active one.
    pub(crate) fn set_c1(&mut self, level: bool) {
        let active = if self.c1_rising_edge() {
            !self.c1_level && level
        } else {
            self.c1_level && !level
        };
        self.c1_level = level;

        if active {
            self.irq1 = true;
            // In handshake, C1's active transition returns C2 to a high level.
            if self.c2_mode() == C2Mode::Handshake {
                self.c2_output = true;
            }
        }
    }

    /// Changes C2's level. Only has an effect if C2 is an input.
    pub(crate) fn set_c2(&mut self, level: bool) {
        let C2Mode::Input { rising_edge, .. } = self.c2_mode() else {
            return;
        };

        let active = if rising_edge {
            !self.c2_level && level
        } else {
            self.c2_level && !level
        };
        self.c2_level = level;

        if active {
            self.irq2 = true;
        }
    }

    /// Handshake and pulse on a port access.
    fn trigger_c2_output(&mut self) {
        match self.c2_mode() {
            C2Mode::Handshake => self.c2_output = false,
            C2Mode::Pulse => {
                // The pulse lasts a single E cycle, shorter than our
                // instruction granularity, so we expose it as an event
                // instead of as a level.
                self.c2_pulsed = true;
            }
            _ => {}
        }
    }
}
