//! 6808 + PIA integration: the first piece of machine that really runs.
//!
//! It builds a minimal System 11-style memory map —RAM down low, a PIA at
//! `$2100`, ROM up top— and runs real machine code that configures the chip,
//! writes to a port and services an interrupt.

use vpw_m6800::{Bus, Cpu, vectors};
use vpw_pia6821::Pia;

/// Base address of PIA 0, the same one System 11 uses.
const PIA0: u16 = 0x2100;

/// Memory map: RAM everywhere except the PIA's window.
struct Machine {
    ram: Box<[u8; 0x1_0000]>,
    pia: Pia,
}

impl Machine {
    fn new() -> Self {
        Self {
            ram: Box::new([0; 0x1_0000]),
            pia: Pia::new(),
        }
    }

    fn load(&mut self, addr: u16, data: &[u8]) {
        let start = addr as usize;
        self.ram[start..start + data.len()].copy_from_slice(data);
    }

    fn set_vector(&mut self, vector: u16, target: u16) {
        let [hi, lo] = target.to_be_bytes();
        self.ram[vector as usize] = hi;
        self.ram[vector as usize + 1] = lo;
    }

    /// The PIA answers to a block of 4 addresses. Since it only decodes two
    /// lines, on real hardware it repeats all over the window; here the exact
    /// block is enough.
    fn pia_select(addr: u16) -> Option<u8> {
        (PIA0..PIA0 + 4)
            .contains(&addr)
            .then(|| (addr - PIA0) as u8)
    }
}

impl Bus for Machine {
    fn read(&mut self, addr: u16) -> u8 {
        match Self::pia_select(addr) {
            Some(select) => self.pia.read(select),
            None => self.ram[addr as usize],
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match Self::pia_select(addr) {
            Some(select) => self.pia.write(select, value),
            None => self.ram[addr as usize] = value,
        }
    }
}

/// Runs `steps` instructions keeping the PIA's IRQ wired to the CPU, which is
/// what the copper trace does on the real board.
fn run(cpu: &mut Cpu, machine: &mut Machine, steps: usize) {
    for _ in 0..steps {
        cpu.set_irq(machine.pia.irq());
        cpu.step(machine);
    }
}

#[test]
fn the_6808_configures_the_pia_and_services_its_interrupt() {
    // Main program:
    //
    //   0100: LDS  #$00FF     ; stack
    //   0103: LDAA #$00
    //   0105: STAA $2103      ; CRB = 0 -> the access to $2102 sees the direction
    //   0108: LDAA #$FF
    //   010A: STAA $2102      ; DDRB = all output
    //   010D: LDAA #$04
    //   010F: STAA $2103      ; CRB bit 2 -> the access sees the data
    //   0112: LDAA #$5A
    //   0114: STAA $2102      ; port B = $5A
    //   0117: LDAA #$05       ; data + CA1 IRQ enabled, falling edge
    //   0119: STAA $2101      ; CRA
    //   011C: CLI             ; drops the mask
    //   011D: BRA  -2         ; wait
    let main = [
        0x8E, 0x00, 0xFF, // LDS #$00FF
        0x86, 0x00, // LDAA #$00
        0xB7, 0x21, 0x03, // STAA $2103
        0x86, 0xFF, // LDAA #$FF
        0xB7, 0x21, 0x02, // STAA $2102
        0x86, 0x04, // LDAA #$04
        0xB7, 0x21, 0x03, // STAA $2103
        0x86, 0x5A, // LDAA #$5A
        0xB7, 0x21, 0x02, // STAA $2102
        0x86, 0x05, // LDAA #$05
        0xB7, 0x21, 0x01, // STAA $2101
        0x0E, // CLI
        0x20, 0xFE, // BRA -2
    ];

    // Interrupt routine:
    //
    //   0200: LDAA $2100      ; reading port A drops the PIA's flag
    //   0203: INC  $0050      ; count of interrupts serviced
    //   0206: RTI
    let isr = [
        0xB6, 0x21, 0x00, // LDAA $2100
        0x7C, 0x00, 0x50, // INC $0050
        0x3B, // RTI
    ];

    let mut machine = Machine::new();
    machine.load(0x0100, &main);
    machine.load(0x0200, &isr);
    machine.set_vector(vectors::RESET, 0x0100);
    machine.set_vector(vectors::IRQ, 0x0200);

    let mut cpu = Cpu::new();
    cpu.reset(&mut machine);

    // Run the initialisation and leave it in the wait loop.
    run(&mut cpu, &mut machine, 15);

    assert_eq!(
        cpu.last_illegal, None,
        "the program did not execute anything odd"
    );
    assert_eq!(
        machine.pia.port_b_output(),
        0x5A,
        "the machine code programmed port B through the bus"
    );
    assert_eq!(machine.pia.port_b_direction(), 0xFF);
    assert_eq!(cpu.pc, 0x011D, "it ended up in the wait loop");
    assert!(!cpu.cc.i, "the CLI ran");
    assert_eq!(machine.ram[0x0050], 0, "there have been no interrupts yet");

    // A switch closes: CA1 makes its active transition (falling edge).
    machine.pia.set_ca1(true);
    machine.pia.set_ca1(false);
    assert!(machine.pia.irq(), "the PIA asks for an interrupt");

    // The CPU takes it, runs the routine and comes back.
    run(&mut cpu, &mut machine, 4);

    assert_eq!(machine.ram[0x0050], 1, "the interrupt routine ran");
    assert!(!machine.pia.irq(), "reading port A dropped the line");
    assert_eq!(cpu.pc, 0x011D, "the RTI put it back in the loop");
    assert!(!cpu.cc.i, "and the RTI restored the mask down");

    // Without a new transition, it does not go in again.
    run(&mut cpu, &mut machine, 10);
    assert_eq!(machine.ram[0x0050], 1);

    // Another switch closure, another interrupt.
    machine.pia.set_ca1(true);
    machine.pia.set_ca1(false);
    run(&mut cpu, &mut machine, 4);
    assert_eq!(machine.ram[0x0050], 2);
}

#[test]
fn the_6808_reads_a_switch_through_port_a() {
    // Port A as an input (DDRA = 0 after the reset) and the program polls it.
    //
    //   0100: LDAA #$04
    //   0102: STAA $2101      ; CRA bit 2 -> the access sees the data
    //   0105: LDAA $2100      ; read the switches
    //   0108: STAA $0060
    //   010B: BRA -2
    let program = [
        0x86, 0x04, // LDAA #$04
        0xB7, 0x21, 0x01, // STAA $2101
        0xB6, 0x21, 0x00, // LDAA $2100
        0xB7, 0x00, 0x60, // STAA $0060
        0x20, 0xFE, // BRA -2
    ];

    let mut machine = Machine::new();
    machine.load(0x0100, &program);
    machine.set_vector(vectors::RESET, 0x0100);
    machine.pia.set_port_a_input(0b1010_0101);

    let mut cpu = Cpu::new();
    cpu.reset(&mut machine);
    run(&mut cpu, &mut machine, 4);

    assert_eq!(
        machine.ram[0x0060], 0b1010_0101,
        "the switch state made it into RAM"
    );
}
