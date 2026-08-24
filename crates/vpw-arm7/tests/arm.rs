//! The ARM instruction set, one class at a time.
//!
//! A processor core is the one thing where a test suite is not optional. It is
//! thousands of encodings, every one of them is silent when wrong, and the
//! symptom of a wrong flag in a subtraction is a sound board that produces
//! noise three minutes later with nothing to point at.

use vpw_arm7::{Bus, Cpu, Mode};

/// Sixty-four kilobytes, flat.
struct Ram(Vec<u8>);

impl Ram {
    fn new() -> Self {
        Self(vec![0; 0x1_0000])
    }
    /// Puts a word where the processor will fetch it.
    fn at(&mut self, addr: u32, word: u32) {
        self.0[addr as usize..addr as usize + 4].copy_from_slice(&word.to_le_bytes());
    }
}

impl Bus for Ram {
    fn read32(&mut self, addr: u32) -> u32 {
        let i = (addr & !3) as usize % self.0.len();
        u32::from_le_bytes(self.0[i..i + 4].try_into().unwrap())
    }
    fn write32(&mut self, addr: u32, value: u32) {
        let i = (addr & !3) as usize % self.0.len();
        self.0[i..i + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn read8(&mut self, addr: u32) -> u8 {
        self.0[addr as usize % self.0.len()]
    }
    fn write8(&mut self, addr: u32, value: u8) {
        let i = addr as usize % self.0.len();
        self.0[i] = value;
    }
}

/// Runs one instruction at address zero and gives back the processor.
fn run(insn: u32, setup: impl FnOnce(&mut Cpu)) -> (Cpu, Ram) {
    let mut ram = Ram::new();
    ram.at(0, insn);
    let mut cpu = Cpu::new();
    cpu.r[15] = 0;
    setup(&mut cpu);
    cpu.step(&mut ram);
    (cpu, ram)
}

const NEG: u32 = 1 << 31;
const ZERO: u32 = 1 << 30;
const CARRY: u32 = 1 << 29;
const OVER: u32 = 1 << 28;

// ------------------------------------------------------------- the basics ---

#[test]
fn r15_reads_eight_on() {
    // The pipeline, and not a detail to round off: `add r0, pc, #4` is how a
    // compiler of this era addressed a literal, and it only lands on the
    // literal if R15 reads as the instruction plus eight.
    let cpu = Cpu::new();
    assert_eq!(cpu.reg(15), 8, "in ARM state");
}

#[test]
fn a_condition_that_fails_costs_a_cycle_and_does_nothing() {
    // `movne r0, #1` with Z set.
    let (cpu, _) = run(0x13a0_0001, |c| c.cpsr |= ZERO);
    assert_eq!(cpu.r[0], 0);
    assert_eq!(cpu.r[15], 4, "and it still moved on");
}

// ------------------------------------------------------- data processing ---

#[test]
fn mov_moves() {
    // `mov r0, #0x2a`
    let (cpu, _) = run(0xe3a0_002a, |_| {});
    assert_eq!(cpu.r[0], 0x2a);
}

#[test]
fn an_immediate_is_eight_bits_rotated_by_an_even_amount() {
    // `mov r0, #0xff000000` is 0xff rotated right by 8, encoded as rotate 4.
    let (cpu, _) = run(0xe3a0_04ff, |_| {});
    assert_eq!(cpu.r[0], 0xff00_0000);
}

#[test]
fn adds_sets_the_flags_an_addition_sets() {
    // `adds r0, r1, r2` with the two halves of an overflow.
    let (cpu, _) = run(0xe091_0002, |c| {
        c.r[1] = 0x7fff_ffff;
        c.r[2] = 1;
    });
    assert_eq!(cpu.r[0], 0x8000_0000);
    assert!(cpu.cpsr & NEG != 0, "negative");
    assert!(cpu.cpsr & OVER != 0, "signed overflow");
    assert!(cpu.cpsr & CARRY == 0, "no carry out");
}

#[test]
fn the_carry_out_of_a_subtraction_is_not_borrow() {
    // The one that catches everybody: a subtraction sets C when there was
    // **no** borrow. `subs r0, r1, r2` with 5 - 3.
    let (cpu, _) = run(0xe051_0002, |c| {
        c.r[1] = 5;
        c.r[2] = 3;
    });
    assert_eq!(cpu.r[0], 2);
    assert!(cpu.cpsr & CARRY != 0, "5 - 3 borrows nothing, so C is set");

    let (cpu, _) = run(0xe051_0002, |c| {
        c.r[1] = 3;
        c.r[2] = 5;
    });
    assert_eq!(cpu.r[0], (-2i32) as u32);
    assert!(cpu.cpsr & CARRY == 0, "3 - 5 borrows, so C is clear");
}

#[test]
fn cmp_does_not_write_a_register() {
    // `cmp r1, r2`, equal.
    let (cpu, _) = run(0xe151_0002, |c| {
        c.r[0] = 0xdead;
        c.r[1] = 7;
        c.r[2] = 7;
    });
    assert_eq!(cpu.r[0], 0xdead, "rd is not a destination here");
    assert!(cpu.cpsr & ZERO != 0);
}

#[test]
fn the_shifter_carry_reaches_the_flag() {
    // `movs r0, r1, lsr #1` with the bottom bit set. Invisible until a
    // compiler uses it to get a bit into carry, which they do.
    let (cpu, _) = run(0xe1b0_00a1, |c| c.r[1] = 0b11);
    assert_eq!(cpu.r[0], 1);
    assert!(cpu.cpsr & CARRY != 0, "the bit shifted out is the carry");
}

#[test]
fn an_immediate_lsr_of_zero_means_thirty_two() {
    // `movs r0, r1, lsr #32` — encoded with the amount field at zero.
    let (cpu, _) = run(0xe1b0_0021, |c| c.r[1] = 0x8000_0000);
    assert_eq!(cpu.r[0], 0);
    assert!(cpu.cpsr & CARRY != 0, "the top bit came out");
}

#[test]
fn an_immediate_ror_of_zero_is_rrx() {
    // `movs r0, r1, rrx`: one place right, through carry.
    let (cpu, _) = run(0xe1b0_0061, |c| {
        c.r[1] = 2;
        c.cpsr |= CARRY;
    });
    assert_eq!(cpu.r[0], 0x8000_0001);
    assert!(
        cpu.cpsr & CARRY == 0,
        "and the bit that fell out is the new carry"
    );
}

// -------------------------------------------------------------- branches ---

#[test]
fn a_branch_goes_where_it_says() {
    // `b .+8` — the offset is in words and relative to R15, which is eight on.
    let (cpu, _) = run(0xea00_0000, |_| {});
    assert_eq!(cpu.r[15], 8);
}

#[test]
fn a_branch_backwards_goes_backwards() {
    // `b .-4`, offset -3 words.
    let (cpu, _) = run(0xeaff_fffd, |c| c.r[15] = 0);
    assert_eq!(cpu.r[15], 8u32.wrapping_sub(12));
}

#[test]
fn branch_and_link_leaves_the_return_address() {
    let (cpu, _) = run(0xeb00_0000, |_| {});
    assert_eq!(cpu.r[14], 4, "the instruction after this one");
    assert_eq!(cpu.r[15], 8);
}

#[test]
fn branch_and_exchange_takes_the_state_from_the_low_bit() {
    // The low bit is not part of the address, it is where to go: Thumb or ARM.
    let (cpu, _) = run(0xe12f_ff11, |c| c.r[1] = 0x100 | 1);
    assert_eq!(cpu.r[15], 0x100, "the bit is not part of the address");
    assert!(cpu.thumb(), "and it is the state");
}

// ------------------------------------------------------------- transfers ---

#[test]
fn a_word_load_and_a_word_store_agree() {
    // `str r1, [r0]` then read it back.
    let (_, mut ram) = run(0xe580_1000, |c| {
        c.r[0] = 0x200;
        c.r[1] = 0x1234_5678;
    });
    assert_eq!(ram.read32(0x200), 0x1234_5678);
}

#[test]
fn an_unaligned_word_load_rotates_rather_than_faults() {
    // This part does not fault on an unaligned word: it reads the aligned word
    // and rotates it, and compilers of the era used that deliberately.
    let mut ram = Ram::new();
    ram.at(0, 0xe590_0000); // ldr r0, [r0]
    ram.at(0x200, 0x1122_3344);
    let mut cpu = Cpu::new();
    cpu.r[15] = 0;
    cpu.r[0] = 0x201;
    cpu.step(&mut ram);
    assert_eq!(cpu.r[0], 0x4411_2233, "rotated by one byte");
}

#[test]
fn a_post_indexed_load_writes_the_base_back_before_the_load() {
    // `ldr r0, [r0], #4` loads into the register it addressed through, and the
    // load wins.
    let mut ram = Ram::new();
    ram.at(0, 0xe490_0004);
    ram.at(0x200, 0xcafe_babe);
    let mut cpu = Cpu::new();
    cpu.r[15] = 0;
    cpu.r[0] = 0x200;
    cpu.step(&mut ram);
    assert_eq!(cpu.r[0], 0xcafe_babe, "the loaded value, not the new base");
}

#[test]
fn a_byte_load_is_zero_extended_and_a_signed_one_is_not() {
    let mut ram = Ram::new();
    ram.at(0, 0xe5d0_0000); // ldrb r0, [r0]
    ram.write8(0x200, 0xff);
    let mut cpu = Cpu::new();
    cpu.r[15] = 0;
    cpu.r[0] = 0x200;
    cpu.step(&mut ram);
    assert_eq!(cpu.r[0], 0xff, "a byte load is not sign extended");

    let mut ram = Ram::new();
    ram.at(0, 0xe1d0_00d0); // ldrsb r0, [r0]
    ram.write8(0x200, 0xff);
    let mut cpu = Cpu::new();
    cpu.r[15] = 0;
    cpu.r[0] = 0x200;
    cpu.step(&mut ram);
    assert_eq!(cpu.r[0], 0xffff_ffff, "a signed one is");
}

#[test]
fn a_halfword_load_takes_two_bytes() {
    let mut ram = Ram::new();
    ram.at(0, 0xe1d0_00b0); // ldrh r0, [r0]
    ram.at(0x200, 0x0000_beef);
    let mut cpu = Cpu::new();
    cpu.r[15] = 0;
    cpu.r[0] = 0x200;
    cpu.step(&mut ram);
    assert_eq!(cpu.r[0], 0xbeef);
}

#[test]
fn the_lowest_register_goes_to_the_lowest_address() {
    // Whichever direction the block runs. `stmda r0!, {r1, r2}` writes
    // downwards from the base, and r1 still lands below r2.
    let mut ram = Ram::new();
    ram.at(0, 0xe820_0006); // stmda r0!, {r1,r2}
    let mut cpu = Cpu::new();
    cpu.r[15] = 0;
    cpu.r[0] = 0x204;
    cpu.r[1] = 0x1111_1111;
    cpu.r[2] = 0x2222_2222;
    cpu.step(&mut ram);
    // Decrement-after uses the addresses Rn-4(n-1) up to Rn, so with a base of
    // 0x204 and two registers that is 0x200 and 0x204 — and r1, the lower
    // register, still lands on the lower of the two.
    assert_eq!(ram.read32(0x200), 0x1111_1111, "r1 at the low address");
    assert_eq!(ram.read32(0x204), 0x2222_2222);
    assert_eq!(
        cpu.r[0], 0x1fc,
        "and the base ended two words below where it started"
    );
}

#[test]
fn a_block_load_and_store_round_trip() {
    let mut ram = Ram::new();
    ram.at(0, 0xe8a0_000e); // stmia r0!, {r1,r2,r3}
    ram.at(4, 0xe8b0_0070); // ldmia r0!, {r4,r5,r6}
    let mut cpu = Cpu::new();
    cpu.r[15] = 0;
    cpu.r[0] = 0x300;
    cpu.r[1] = 1;
    cpu.r[2] = 2;
    cpu.r[3] = 3;
    cpu.step(&mut ram);
    assert_eq!(cpu.r[0], 0x30c);
    cpu.r[0] = 0x300;
    cpu.step(&mut ram);
    assert_eq!((cpu.r[4], cpu.r[5], cpu.r[6]), (1, 2, 3));
}

// ----------------------------------------------------------- multiplies ---

#[test]
fn multiply_multiplies_and_accumulates() {
    // `mul r0, r1, r2`
    let (cpu, _) = run(0xe000_0291, |c| {
        c.r[1] = 7;
        c.r[2] = 6;
    });
    assert_eq!(cpu.r[0], 42);

    // `mla r0, r1, r2, r3`
    let (cpu, _) = run(0xe020_3291, |c| {
        c.r[1] = 7;
        c.r[2] = 6;
        c.r[3] = 8;
    });
    assert_eq!(cpu.r[0], 50);
}

#[test]
fn a_signed_long_multiply_keeps_its_sign() {
    // `smull r0, r1, r2, r3` with a negative operand.
    let (cpu, _) = run(0xe0c1_0392, |c| {
        c.r[2] = (-2i32) as u32;
        c.r[3] = 3;
    });
    let full = (u64::from(cpu.r[1]) << 32) | u64::from(cpu.r[0]);
    assert_eq!(full as i64, -6);
}

// ------------------------------------------------- modes and exceptions ---

#[test]
fn each_mode_has_its_own_stack_pointer() {
    let mut cpu = Cpu::new();
    cpu.set_mode(Mode::User);
    cpu.r[13] = 0x1000;
    cpu.set_mode(Mode::Irq);
    cpu.r[13] = 0x2000;
    cpu.set_mode(Mode::User);
    assert_eq!(cpu.r[13], 0x1000, "user got its own back");
    cpu.set_mode(Mode::Irq);
    assert_eq!(cpu.r[13], 0x2000);
}

#[test]
fn fiq_banks_five_more_registers_than_anything_else() {
    // The awkward one. FIQ banks r8 to r14 and every other mode banks only r13
    // and r14, so leaving FIQ has to put r8 to r12 back and leaving IRQ must
    // not touch them.
    let mut cpu = Cpu::new();
    cpu.set_mode(Mode::User);
    cpu.r[8] = 0xaaaa;
    cpu.set_mode(Mode::Fiq);
    cpu.r[8] = 0xbbbb;
    cpu.set_mode(Mode::User);
    assert_eq!(cpu.r[8], 0xaaaa, "FIQ had its own r8");

    cpu.set_mode(Mode::Irq);
    assert_eq!(cpu.r[8], 0xaaaa, "IRQ shares user's r8");
    cpu.r[8] = 0xcccc;
    cpu.set_mode(Mode::User);
    assert_eq!(cpu.r[8], 0xcccc, "and writing it there writes user's");
}

#[test]
fn an_interrupt_banks_the_return_address_and_the_status() {
    let mut ram = Ram::new();
    ram.at(0x100, 0xe1a0_0000); // nop
    let mut cpu = Cpu::new();
    cpu.r[15] = 0x100;
    cpu.cpsr &= !(1 << 7); // unmask irq
    cpu.irq = true;
    cpu.step(&mut ram);
    assert_eq!(cpu.mode(), Mode::Irq);
    assert_eq!(cpu.r[15], 0x18, "the IRQ vector");
    // Four past the instruction that had not run, so that the `SUBS pc, lr, #4`
    // every handler ends in lands back on it.
    assert_eq!(cpu.r[14], 0x104, "and the way back");
    assert!(cpu.cpsr & (1 << 7) != 0, "with interrupts masked");
}

#[test]
fn a_masked_interrupt_is_not_taken() {
    let mut ram = Ram::new();
    ram.at(0x100, 0xe1a0_0000);
    let mut cpu = Cpu::new();
    cpu.r[15] = 0x100;
    cpu.irq = true; // and I is set, from reset
    cpu.step(&mut ram);
    assert_eq!(cpu.r[15], 0x104, "it just carried on");
}

#[test]
fn movs_pc_lr_is_how_an_exception_returns() {
    // The restore of the saved status is the whole point of the S bit here.
    let mut ram = Ram::new();
    ram.at(0x18, 0xe1b0_f00e); // movs pc, lr
    let mut cpu = Cpu::new();
    cpu.r[15] = 0x100;
    cpu.cpsr = Mode::User as u32;
    cpu.irq = true;
    cpu.step(&mut ram); // takes it
    assert_eq!(cpu.mode(), Mode::Irq);
    cpu.step(&mut ram); // and returns
    assert_eq!(cpu.mode(), Mode::User, "back where it came from");
    // `MOVS pc, lr` without the subtraction lands one instruction on, which is
    // why handlers that use it are the ones that do the subtraction themselves.
    assert_eq!(cpu.r[15], 0x104);
}

#[test]
fn a_software_interrupt_is_reported_and_taken() {
    let (cpu, _) = run(0xef12_3456, |c| c.cpsr = Mode::User as u32);
    assert_eq!(cpu.swi, Some(0x12_3456), "the comment field");
    assert_eq!(cpu.mode(), Mode::Supervisor);
    assert_eq!(cpu.r[15], 0x08);
}

#[test]
fn mrs_and_msr_move_the_status_register() {
    // `mrs r0, cpsr`
    let (cpu, _) = run(0xe10f_0000, |c| c.cpsr |= CARRY);
    assert!(cpu.r[0] & CARRY != 0);

    // `msr cpsr_f, r0` — the flags byte only.
    let (cpu, _) = run(0xe128_f000, |c| {
        c.cpsr = Mode::Supervisor as u32;
        c.r[0] = NEG;
    });
    assert!(cpu.cpsr & NEG != 0);
    assert_eq!(
        cpu.mode(),
        Mode::Supervisor,
        "and the mode was not in the mask"
    );
}

#[test]
fn user_mode_cannot_change_its_own_mode_with_msr() {
    // The privilege check that keeps a program from promoting itself.
    let (cpu, _) = run(0xe129_f000, |c| {
        c.cpsr = Mode::User as u32;
        c.r[0] = Mode::Supervisor as u32 | NEG;
    });
    assert_eq!(cpu.mode(), Mode::User, "the control byte was refused");
}

/// An interrupt has to come back to the instruction it interrupted, not to the
/// one after it. Every handler on this part ends in `SUBS pc, lr, #4`, so the
/// banked link register has to be four past the instruction that has not run.
///
/// The case that catches it is the one every idle loop is made of: a branch to
/// itself. Returning four bytes late walks straight out of the program.
#[test]
fn an_interrupt_returns_to_the_instruction_it_interrupted() {
    let mut ram = Ram::new();
    // 0x18 is the IRQ vector; this handler returns at once.
    ram.at(0x18, 0xea00_0000); // b 0x20
    ram.at(0x20, 0xe25e_f004); // subs pc, lr, #4
    // The idle loop the interrupt lands in.
    ram.at(0x100, 0xeaff_fffe); // b .

    let mut cpu = Cpu::new();
    cpu.r[15] = 0x100;
    cpu.cpsr &= !(1 << 7); // interrupts allowed
    cpu.irq = true;

    cpu.step(&mut ram); // takes the interrupt
    assert_eq!(cpu.r[15], 0x18, "should be at the vector");
    assert_eq!(
        cpu.r[14], 0x104,
        "the link register is the instruction plus four"
    );
    cpu.irq = false;
    cpu.step(&mut ram); // the branch in the vector
    cpu.step(&mut ram); // subs pc, lr, #4
    assert_eq!(cpu.r[15], 0x100, "back at the branch, not past it");
}
