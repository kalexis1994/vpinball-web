//! The 6502, exercised one behaviour at a time.
//!
//! Each test names a fact about the chip rather than a function of ours, and
//! several of them are facts that are easy to get wrong in exactly one
//! direction — the zero page wrapping, the `JMP` bug, what `RTS` adds back.
//! Those are the ones a ROM finds within a few thousand instructions and the
//! ones nothing else would catch.

use vpw_m6502::{Cpu, FlatMemory, flags, vectors};

/// A CPU with a program at `$0200` and the reset vector pointing at it.
fn boot(program: &[u8]) -> (Cpu, FlatMemory) {
    let mut mem = FlatMemory::new();
    mem.load(0x0200, program);
    mem.bytes[vectors::RESET as usize] = 0x00;
    mem.bytes[vectors::RESET as usize + 1] = 0x02;
    let mut cpu = Cpu::new();
    cpu.reset(&mut mem);
    (cpu, mem)
}

#[test]
fn reset_takes_the_vector_and_leaves_the_stack_where_a_rom_expects_it() {
    // Reset runs the same microcode as an interrupt: it decrements the stack
    // pointer three times without writing, so a chip powering up at $00 ends
    // at $FD. Every ROM of the era is written against that.
    let (cpu, _) = boot(&[0xEA]);
    assert_eq!(cpu.pc, 0x0200);
    assert_eq!(cpu.s, 0xFD);
    assert!(cpu.p & flags::IRQ_DISABLE != 0, "interrupts start off");
}

#[test]
fn a_load_sets_zero_and_negative_and_nothing_else() {
    let (mut cpu, mut mem) = boot(&[0xA9, 0x00, 0xA9, 0x80, 0xA9, 0x01]);
    cpu.step(&mut mem);
    assert!(cpu.p & flags::ZERO != 0);
    cpu.step(&mut mem);
    assert!(cpu.p & flags::NEGATIVE != 0);
    assert!(cpu.p & flags::ZERO == 0);
    cpu.step(&mut mem);
    assert_eq!(cpu.p & (flags::ZERO | flags::NEGATIVE), 0);
}

#[test]
fn the_zero_page_index_wraps_inside_the_page() {
    // `LDA $FF,X` with X = 2 reads $01, not $0101. A ROM that indexes a
    // zero-page table off the end is relying on it.
    let (mut cpu, mut mem) = boot(&[0xA2, 0x02, 0xB5, 0xFF]);
    mem.bytes[0x0001] = 0x42;
    mem.bytes[0x0101] = 0x99;
    cpu.step(&mut mem);
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x42);
}

#[test]
fn an_indexed_read_across_a_page_costs_one_more_and_a_write_always_does() {
    // The chip fetches from the wrong page and fixes the high byte, so a read
    // pays only when it was wrong. A write pays every time, because the
    // fix-up happens whether it was needed or not.
    let (mut cpu, mut mem) = boot(&[0xA2, 0x01, 0xBD, 0xFF, 0x02]);
    cpu.step(&mut mem);
    assert_eq!(cpu.step(&mut mem), 5, "LDA $02FF,X with X=1 crosses");

    let (mut cpu, mut mem) = boot(&[0xA2, 0x01, 0xBD, 0x00, 0x03]);
    cpu.step(&mut mem);
    assert_eq!(cpu.step(&mut mem), 4, "and does not when it stays");

    let (mut cpu, mut mem) = boot(&[0xA2, 0x01, 0x9D, 0x00, 0x03]);
    cpu.step(&mut mem);
    assert_eq!(cpu.step(&mut mem), 5, "a write pays even inside the page");
}

#[test]
fn jmp_indirect_keeps_its_hardware_bug() {
    // `JMP ($03FF)` reads the low byte from $03FF and the high byte from
    // $0300 — the same page, not the next one. It is a defect and it is what
    // a ROM assembled in 1985 was tested against.
    let (mut cpu, mut mem) = boot(&[0x6C, 0xFF, 0x03]);
    mem.bytes[0x03FF] = 0x34;
    mem.bytes[0x0400] = 0xFF; // what a correct chip would use
    mem.bytes[0x0300] = 0x12; // what this one uses
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x1234);
}

#[test]
fn a_call_returns_to_the_instruction_after_it() {
    // `JSR` pushes the address of its own last byte and `RTS` adds one back.
    // Either half alone is off by one and every call returns into itself.
    let (mut cpu, mut mem) = boot(&[0x20, 0x00, 0x03]);
    mem.load(0x0300, &[0x60]);
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0300);
    assert_eq!(cpu.s, 0xFB, "two bytes pushed");
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0203);
}

#[test]
fn brk_pushes_its_own_flag_and_an_interrupt_does_not() {
    // The break bit is not a register bit — nothing can test it — and the
    // copy on the stack is the only way a handler tells `BRK` from an IRQ.
    let mut mem = FlatMemory::new();
    mem.load(0x0200, &[0x00, 0xAA]);
    mem.bytes[vectors::RESET as usize] = 0x00;
    mem.bytes[vectors::RESET as usize + 1] = 0x02;
    mem.bytes[vectors::IRQ as usize] = 0x00;
    mem.bytes[vectors::IRQ as usize + 1] = 0x04;
    let mut cpu = Cpu::new();
    cpu.reset(&mut mem);

    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0400);
    let pushed = mem.bytes[0x0100 | (usize::from(cpu.s) + 1)];
    assert!(pushed & flags::BREAK != 0, "BRK says so on the stack");
    // And the return address skips the byte after `BRK`, which is what lets a
    // handler read a code there.
    let lo = mem.bytes[0x0100 | (usize::from(cpu.s) + 2)];
    let hi = mem.bytes[0x0100 | (usize::from(cpu.s) + 3)];
    assert_eq!(u16::from_le_bytes([lo, hi]), 0x0202);

    let mut cpu = Cpu::new();
    cpu.reset(&mut mem);
    cpu.p &= !flags::IRQ_DISABLE;
    cpu.irq = true;
    cpu.step(&mut mem);
    let pushed = mem.bytes[0x0100 | (usize::from(cpu.s) + 1)];
    assert!(pushed & flags::BREAK == 0, "an interrupt does not");
}

#[test]
fn an_irq_waits_for_the_disable_flag_and_an_nmi_does_not() {
    let mut mem = FlatMemory::new();
    mem.load(0x0200, &[0xEA, 0xEA]);
    mem.bytes[vectors::RESET as usize] = 0x00;
    mem.bytes[vectors::RESET as usize + 1] = 0x02;
    mem.bytes[vectors::IRQ as usize + 1] = 0x04;
    mem.bytes[vectors::NMI as usize + 1] = 0x05;
    let mut cpu = Cpu::new();
    cpu.reset(&mut mem);

    // Reset leaves interrupts off, so the line does nothing.
    cpu.irq = true;
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0201, "the IRQ was masked");

    cpu.p &= !flags::IRQ_DISABLE;
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0400);

    // The NMI is not maskable and is edge-triggered: one request, one
    // interrupt, whatever the flag says.
    let mut cpu = Cpu::new();
    cpu.reset(&mut mem);
    cpu.nmi = true;
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0500);
    assert!(!cpu.nmi, "the edge is consumed");
}

#[test]
fn a_taken_branch_costs_one_more_and_a_page_two() {
    // `BNE` forward, not taken.
    let (mut cpu, mut mem) = boot(&[0xA9, 0x00, 0xD0, 0x10]);
    cpu.step(&mut mem);
    assert_eq!(cpu.step(&mut mem), 2);

    // Taken, inside the page.
    let (mut cpu, mut mem) = boot(&[0xA9, 0x01, 0xD0, 0x10]);
    cpu.step(&mut mem);
    assert_eq!(cpu.step(&mut mem), 3);

    // Taken, across one. The page is measured from the instruction *after*
    // the branch, not from the branch itself: at $02F0 the next instruction
    // is $02F2 and the target is $0302, so this one really crosses.
    let mut mem = FlatMemory::new();
    mem.load(0x02F0, &[0xD0, 0x10]);
    mem.bytes[vectors::RESET as usize] = 0xF0;
    mem.bytes[vectors::RESET as usize + 1] = 0x02;
    let mut cpu = Cpu::new();
    cpu.reset(&mut mem);
    cpu.p &= !flags::ZERO;
    assert_eq!(cpu.step(&mut mem), 4);
    assert_eq!(cpu.pc, 0x0302);
}

#[test]
fn a_comparison_sets_the_carry_when_the_register_is_the_larger() {
    // The carry is "no borrow", which is the opposite way round from what the
    // name suggests and the reason `SEC` precedes a subtraction.
    let (mut cpu, mut mem) = boot(&[0xA9, 0x40, 0xC9, 0x30, 0xC9, 0x50, 0xC9, 0x40]);
    cpu.step(&mut mem);
    cpu.step(&mut mem);
    assert!(cpu.p & flags::CARRY != 0, "0x40 >= 0x30");
    cpu.step(&mut mem);
    assert!(cpu.p & flags::CARRY == 0, "0x40 < 0x50");
    cpu.step(&mut mem);
    assert!(
        cpu.p & flags::CARRY != 0 && cpu.p & flags::ZERO != 0,
        "equal"
    );
}

#[test]
fn add_and_subtract_carry_and_overflow_the_way_the_chip_does() {
    // Overflow is about signs: two positives making a negative.
    let (mut cpu, mut mem) = boot(&[0x18, 0xA9, 0x7F, 0x69, 0x01]);
    cpu.step(&mut mem);
    cpu.step(&mut mem);
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x80);
    assert!(cpu.p & flags::OVERFLOW != 0);
    assert!(cpu.p & flags::NEGATIVE != 0);
    assert!(cpu.p & flags::CARRY == 0);

    // And a subtraction borrows when the carry was clear.
    let (mut cpu, mut mem) = boot(&[0x38, 0xA9, 0x05, 0xE9, 0x03]);
    cpu.step(&mut mem);
    cpu.step(&mut mem);
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x02);
    assert!(cpu.p & flags::CARRY != 0, "no borrow");
}

#[test]
fn decimal_mode_adds_the_way_a_score_is_kept() {
    // A pinball score is a string of BCD digits and every machine of this era
    // adds to it with SED/ADC/CLD. 0x28 + 0x14 is 42, not 0x3C.
    let (mut cpu, mut mem) = boot(&[0xF8, 0x18, 0xA9, 0x28, 0x69, 0x14]);
    for _ in 0..4 {
        cpu.step(&mut mem);
    }
    assert_eq!(cpu.a, 0x42);
    assert!(cpu.p & flags::CARRY == 0);

    // And carries out of the top digit.
    let (mut cpu, mut mem) = boot(&[0xF8, 0x18, 0xA9, 0x99, 0x69, 0x01]);
    for _ in 0..4 {
        cpu.step(&mut mem);
    }
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.p & flags::CARRY != 0);

    // Subtracting comes back the same way.
    let (mut cpu, mut mem) = boot(&[0xF8, 0x38, 0xA9, 0x42, 0xE9, 0x14]);
    for _ in 0..4 {
        cpu.step(&mut mem);
    }
    assert_eq!(cpu.a, 0x28);
}

#[test]
fn the_rotates_go_through_the_carry() {
    // `ROL` is a nine-bit rotate: the carry is part of the ring, which is how
    // a multi-byte shift is written.
    let (mut cpu, mut mem) = boot(&[0x38, 0xA9, 0x80, 0x2A]);
    cpu.step(&mut mem);
    cpu.step(&mut mem);
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x01, "the old carry came in at the bottom");
    assert!(cpu.p & flags::CARRY != 0, "and the top bit went out");
}

#[test]
fn bit_tests_memory_without_touching_the_accumulator() {
    // The odd one: N and V come straight off the memory byte's top two bits,
    // and only Z involves the accumulator at all.
    let (mut cpu, mut mem) = boot(&[0xA9, 0x01, 0x24, 0x50]);
    mem.bytes[0x0050] = 0xC0;
    cpu.step(&mut mem);
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x01, "untouched");
    assert!(cpu.p & flags::NEGATIVE != 0);
    assert!(cpu.p & flags::OVERFLOW != 0);
    assert!(cpu.p & flags::ZERO != 0, "0xC0 & 0x01 is nothing");
}

#[test]
fn the_stack_wraps_inside_its_own_page() {
    // Pushing with S = 0 writes $0100 and leaves S = $FF. A stack that ran
    // into $00FF would be writing over the zero page, which is where
    // everything on this machine lives.
    let (mut cpu, mut mem) = boot(&[0x48, 0x48]);
    cpu.s = 0x00;
    cpu.a = 0x42;
    cpu.step(&mut mem);
    assert_eq!(mem.bytes[0x0100], 0x42);
    assert_eq!(cpu.s, 0xFF);
    cpu.step(&mut mem);
    assert_eq!(mem.bytes[0x01FF], 0x42);
}

#[test]
fn an_unknown_opcode_is_recorded_rather_than_guessed_at() {
    // The undocumented opcodes are not implemented: a ROM assembled against
    // Rockwell's own tools has no reason to contain one, so if this fires the
    // interesting question is what put it there.
    let (mut cpu, mut mem) = boot(&[0x02]);
    let cycles = cpu.step(&mut mem);
    assert_eq!(cycles, 2);
    assert_eq!(cpu.last_illegal, Some((0x0200, 0x02)));
}
