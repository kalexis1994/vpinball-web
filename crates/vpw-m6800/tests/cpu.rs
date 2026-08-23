//! Tests of the MC6800 core.
//!
//! Everything asserted here comes from the Motorola specification: effects on
//! the CCR, addressing modes, push order and cycles.

use vpw_m6800::{Cpu, FlatMemory, vectors};

/// Builds a reset CPU with the program loaded at `0x0100`.
fn boot(program: &[u8]) -> (Cpu, FlatMemory) {
    let mut mem = FlatMemory::new();
    mem.load(0x0100, program);
    mem.set_vector(vectors::RESET, 0x0100);

    let mut cpu = Cpu::new();
    cpu.reset(&mut mem);
    cpu.sp = 0x00FF;
    (cpu, mem)
}

// --- Addressing modes ---------------------------------------------------------

#[test]
fn immediate_mode() {
    let (mut cpu, mut mem) = boot(&[0x86, 0x42]); // LDAA #$42
    assert_eq!(cpu.step(&mut mem), 2);
    assert_eq!(cpu.a, 0x42);
}

#[test]
fn direct_mode_reads_page_zero() {
    let (mut cpu, mut mem) = boot(&[0x96, 0x30]); // LDAA $30
    mem.bytes[0x0030] = 0x99;
    assert_eq!(cpu.step(&mut mem), 3);
    assert_eq!(cpu.a, 0x99);
}

#[test]
fn extended_mode() {
    let (mut cpu, mut mem) = boot(&[0xB6, 0x1F, 0x40]); // LDAA $1F40
    mem.bytes[0x1F40] = 0x7E;
    assert_eq!(cpu.step(&mut mem), 4);
    assert_eq!(cpu.a, 0x7E);
}

#[test]
fn the_indexed_offset_is_unsigned() {
    // With X = 0x2000 and offset 0xFF the address is 0x20FF, not 0x1FFF.
    let (mut cpu, mut mem) = boot(&[0xA6, 0xFF]); // LDAA $FF,X
    cpu.x = 0x2000;
    mem.bytes[0x20FF] = 0x5A;
    mem.bytes[0x1FFF] = 0xEE;
    assert_eq!(cpu.step(&mut mem), 5);
    assert_eq!(cpu.a, 0x5A);
}

// --- Arithmetic and flags ------------------------------------------------------

#[test]
fn add_sets_carry_and_half_carry() {
    let (mut cpu, mut mem) = boot(&[0x8B, 0x01]); // ADDA #$01
    cpu.a = 0xFF;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0x00);
    assert!(cpu.cc.c, "0xFF + 1 overflows 8 bits");
    assert!(cpu.cc.h, "carry from bit 3 into bit 4");
    assert!(cpu.cc.z);
    assert!(!cpu.cc.n);
    assert!(!cpu.cc.v, "no signed overflow: -1 + 1 = 0");
}

#[test]
fn add_sets_signed_overflow() {
    let (mut cpu, mut mem) = boot(&[0x8B, 0x01]); // ADDA #$01
    cpu.a = 0x7F;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0x80);
    assert!(cpu.cc.v, "127 + 1 leaves the signed range");
    assert!(cpu.cc.n);
    assert!(!cpu.cc.c, "there is no unsigned carry");
}

#[test]
fn sub_sets_borrow() {
    let (mut cpu, mut mem) = boot(&[0x80, 0x02]); // SUBA #$02
    cpu.a = 0x01;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0xFF);
    assert!(cpu.cc.c, "1 - 2 borrows");
    assert!(cpu.cc.n);
    assert!(!cpu.cc.v);
}

#[test]
fn sub_sets_signed_overflow() {
    let (mut cpu, mut mem) = boot(&[0x80, 0x01]); // SUBA #$01
    cpu.a = 0x80; // -128
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0x7F);
    assert!(cpu.cc.v, "-128 - 1 goes out of range");
    assert!(!cpu.cc.c);
}

#[test]
fn sbc_subtracts_the_previous_carry() {
    let (mut cpu, mut mem) = boot(&[0x82, 0x01]); // SBCA #$01
    cpu.a = 0x10;
    cpu.cc.c = true;
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x0E, "0x10 - 1 - 1");
}

#[test]
fn cmp_does_not_modify_the_accumulator() {
    let (mut cpu, mut mem) = boot(&[0x81, 0x42]); // CMPA #$42
    cpu.a = 0x42;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0x42);
    assert!(cpu.cc.z);
    assert!(!cpu.cc.c);
}

#[test]
fn bit_does_not_modify_the_accumulator() {
    let (mut cpu, mut mem) = boot(&[0x85, 0x0F]); // BITA #$0F
    cpu.a = 0xF0;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0xF0);
    assert!(cpu.cc.z, "0xF0 & 0x0F = 0");
}

// --- Shifts and rotates -------------------------------------------------------

#[test]
fn asr_preserves_the_sign() {
    let (mut cpu, mut mem) = boot(&[0x47]); // ASRA
    cpu.a = 0x80;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0xC0, "bit 7 is replicated");
    assert!(cpu.cc.n);
    assert!(!cpu.cc.c);
}

#[test]
fn lsr_leaves_n_at_zero() {
    let (mut cpu, mut mem) = boot(&[0x44]); // LSRA
    cpu.a = 0x81;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0x40);
    assert!(!cpu.cc.n, "LSR always shifts a 0 in at the top");
    assert!(cpu.cc.c, "bit 0 comes out through the carry");
    assert!(cpu.cc.v, "V = N xor C");
}

#[test]
fn rol_shifts_in_through_the_carry() {
    let (mut cpu, mut mem) = boot(&[0x49]); // ROLA
    cpu.a = 0x80;
    cpu.cc.c = true;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0x01, "the carry comes in through bit 0");
    assert!(cpu.cc.c, "bit 7 comes out through the carry");
}

#[test]
fn ror_shifts_in_through_the_carry() {
    let (mut cpu, mut mem) = boot(&[0x46]); // RORA
    cpu.a = 0x01;
    cpu.cc.c = true;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0x80);
    assert!(cpu.cc.c);
}

#[test]
fn com_always_leaves_the_carry_at_one() {
    let (mut cpu, mut mem) = boot(&[0x43]); // COMA
    cpu.a = 0x0F;
    cpu.cc.c = false;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0xF0);
    assert!(cpu.cc.c, "COM sets the carry, it is part of the spec");
}

#[test]
fn neg_of_0x80_overflows() {
    let (mut cpu, mut mem) = boot(&[0x40]); // NEGA
    cpu.a = 0x80;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0x80, "0x80 is its own negative");
    assert!(cpu.cc.v);
    assert!(cpu.cc.c);
}

#[test]
fn neg_of_zero_does_not_set_carry() {
    let (mut cpu, mut mem) = boot(&[0x40]); // NEGA
    cpu.a = 0x00;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0x00);
    assert!(!cpu.cc.c);
    assert!(cpu.cc.z);
}

#[test]
fn inc_and_dec_do_not_touch_the_carry() {
    let (mut cpu, mut mem) = boot(&[0x4C, 0x4A]); // INCA, DECA
    cpu.a = 0xFF;
    cpu.cc.c = true;
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x00);
    assert!(
        cpu.cc.c,
        "INC does not touch the carry even if the result wraps"
    );

    cpu.a = 0x80;
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x7F);
    assert!(cpu.cc.v, "DEC of 0x80 overflows");
    assert!(cpu.cc.c, "DEC does not touch the carry either");
}

// --- DAA -----------------------------------------------------------------------

#[test]
fn daa_adjusts_a_bcd_addition() {
    // 0x19 + 0x01 = 0x1A, which in BCD has to be 0x20.
    let (mut cpu, mut mem) = boot(&[0x8B, 0x01, 0x19]); // ADDA #$01 ; DAA
    cpu.a = 0x19;
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x1A);
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x20);
    assert!(!cpu.cc.c);
}

#[test]
fn daa_propagates_the_bcd_carry() {
    // 0x99 + 0x01 = 0x9A -> 0x00 with carry.
    let (mut cpu, mut mem) = boot(&[0x8B, 0x01, 0x19]);
    cpu.a = 0x99;
    cpu.step(&mut mem);
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.cc.c, "the BCD carry comes out through the carry");
    assert!(cpu.cc.z);
}

// --- Branches ------------------------------------------------------------------

#[test]
fn branch_with_a_negative_offset() {
    // BRA -2 jumps onto itself: a closed loop.
    let (mut cpu, mut mem) = boot(&[0x20, 0xFE]);
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0100);
}

#[test]
fn a_branch_always_costs_four_cycles() {
    let (mut cpu, mut mem) = boot(&[0x26, 0x10]); // BNE +$10
    cpu.cc.z = true; // not taken
    assert_eq!(cpu.step(&mut mem), 4);
    assert_eq!(cpu.pc, 0x0102, "it was not taken");

    let (mut cpu, mut mem) = boot(&[0x26, 0x10]);
    cpu.cc.z = false; // taken
    assert_eq!(cpu.step(&mut mem), 4);
    assert_eq!(cpu.pc, 0x0112);
}

#[test]
fn signed_versus_unsigned_comparisons() {
    // 0x50 - 0xB0: unsigned 80 < 176, signed 80 > -80.
    let (mut cpu, mut mem) = boot(&[0x81, 0xB0]); // CMPA #$B0
    cpu.a = 0x50;
    cpu.step(&mut mem);

    assert!(
        cpu.cc.c,
        "unsigned: 0x50 < 0xB0, there is a borrow (BLO/BCS)"
    );
    assert!(cpu.cc.n == cpu.cc.v, "signed: 80 >= -80, so BGE is taken");
}

// --- Subroutines ---------------------------------------------------------------

#[test]
fn jsr_and_rts_make_the_round_trip() {
    // JSR $0200 ; (return) LDAA #$01
    let (mut cpu, mut mem) = boot(&[0xBD, 0x02, 0x00, 0x86, 0x01]);
    mem.load(0x0200, &[0x39]); // RTS

    assert_eq!(cpu.step(&mut mem), 9, "extended JSR is 9 cycles");
    assert_eq!(cpu.pc, 0x0200);
    assert_eq!(cpu.sp, 0x00FD, "2 bytes were left on the stack");

    assert_eq!(cpu.step(&mut mem), 5, "RTS is 5 cycles");
    assert_eq!(cpu.pc, 0x0103, "it comes back right after the JSR");
    assert_eq!(cpu.sp, 0x00FF);

    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x01);
}

#[test]
fn bsr_uses_a_relative_offset() {
    let (mut cpu, mut mem) = boot(&[0x8D, 0x10]); // BSR +$10
    assert_eq!(cpu.step(&mut mem), 8);
    assert_eq!(cpu.pc, 0x0112);
    assert_eq!(cpu.sp, 0x00FD);
}

#[test]
fn psh_and_pul_respect_the_order() {
    let (mut cpu, mut mem) = boot(&[0x36, 0x37, 0x33, 0x32]); // PSHA PSHB PULB PULA
    cpu.a = 0xAA;
    cpu.b = 0xBB;

    cpu.step(&mut mem);
    cpu.step(&mut mem);
    assert_eq!(cpu.sp, 0x00FD);

    cpu.a = 0;
    cpu.b = 0;
    cpu.step(&mut mem);
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0xAA);
    assert_eq!(cpu.b, 0xBB);
    assert_eq!(cpu.sp, 0x00FF);
}

// --- Index and stack pointer ---------------------------------------------------

#[test]
fn inx_only_affects_zero() {
    let (mut cpu, mut mem) = boot(&[0x08]); // INX
    cpu.x = 0xFFFF;
    cpu.cc.n = true;
    cpu.cc.c = true;
    cpu.cc.v = true;
    cpu.step(&mut mem);

    assert_eq!(cpu.x, 0x0000);
    assert!(cpu.cc.z);
    assert!(cpu.cc.n, "INX does not touch N");
    assert!(cpu.cc.c, "INX does not touch C");
    assert!(cpu.cc.v, "INX does not touch V");
}

#[test]
fn tsx_and_txs_differ_by_one() {
    // The 6800 points the SP at the next free slot, so TSX adds 1 and TXS
    // subtracts 1 so that the round trip is neutral.
    let (mut cpu, mut mem) = boot(&[0x30, 0x35]); // TSX ; TXS
    cpu.sp = 0x00FF;
    cpu.step(&mut mem);
    assert_eq!(cpu.x, 0x0100);
    cpu.step(&mut mem);
    assert_eq!(cpu.sp, 0x00FF, "the round trip does not move the SP");
}

#[test]
fn cpx_does_not_touch_the_carry() {
    // A 6800 oddity: CPX affects N, Z and V but leaves the carry untouched.
    // Only the 6801 changed that.
    let (mut cpu, mut mem) = boot(&[0x8C, 0xFF, 0xFF]); // CPX #$FFFF
    cpu.x = 0x0000;
    cpu.cc.c = false;
    cpu.step(&mut mem);

    assert!(!cpu.cc.z);
    assert!(!cpu.cc.c, "CPX does not touch the carry on the 6800");
}

#[test]
fn cpx_reads_negative_and_overflow_off_the_high_bytes_only() {
    // The instruction looks like a sixteen-bit compare and only half of it is.
    // Zero comes from the whole difference, but N and V come from the **high
    // bytes alone** (`6800ops.c:1091`). The 6803 does have a proper sixteen-bit
    // compare, and it is a different opcode; a System 11 runs 6808s, which
    // `#define m6808 m6800` puts on this table.
    //
    // `$8000` against `$0001` is the case that tells them apart: as sixteen
    // bits the answer is `$7FFF`, positive, with signed overflow. As high bytes
    // it is `$80 - $00 = $80`, negative, no overflow — and a `BMI` after it
    // goes the other way.
    let (mut cpu, mut mem) = boot(&[0x8C, 0x00, 0x01]); // CPX #$0001
    cpu.x = 0x8000;
    cpu.step(&mut mem);

    assert!(cpu.cc.n, "$80 - $00 is negative");
    assert!(!cpu.cc.v, "and it does not overflow");
    assert!(!cpu.cc.z, "$8000 is not $0001");
}

#[test]
fn cpx_still_says_equal_on_the_whole_sixteen_bits() {
    // Because Z is the one flag that does come from the full difference. Two
    // values with the same high byte and different low bytes are not equal, and
    // reading Z off the high bytes would say they were.
    let (mut cpu, mut mem) = boot(&[0x8C, 0x12, 0x34, 0x8C, 0x12, 0xFF]);
    cpu.x = 0x1234;

    cpu.step(&mut mem);
    assert!(cpu.cc.z, "$1234 against $1234");

    cpu.step(&mut mem);
    assert!(!cpu.cc.z, "$1234 against $12FF: same high byte, not equal");
}

#[test]
fn lds_and_sts_move_16_bits() {
    let (mut cpu, mut mem) = boot(&[0x8E, 0x01, 0xFF, 0xBF, 0x20, 0x00]); // LDS #$01FF ; STS $2000
    assert_eq!(cpu.step(&mut mem), 3);
    assert_eq!(cpu.sp, 0x01FF);

    assert_eq!(cpu.step(&mut mem), 6);
    assert_eq!(mem.bytes[0x2000], 0x01, "big-endian");
    assert_eq!(mem.bytes[0x2001], 0xFF);
}

#[test]
fn ldx_and_stx_use_the_b_column() {
    let (mut cpu, mut mem) = boot(&[0xCE, 0x12, 0x34, 0xFF, 0x20, 0x00]); // LDX #$1234 ; STX $2000
    cpu.step(&mut mem);
    assert_eq!(cpu.x, 0x1234);

    cpu.step(&mut mem);
    assert_eq!(mem.bytes[0x2000], 0x12);
    assert_eq!(mem.bytes[0x2001], 0x34);
}

// --- Interrupts ----------------------------------------------------------------

#[test]
fn the_irq_pushes_seven_bytes_in_order() {
    let (mut cpu, mut mem) = boot(&[0x01]); // NOP
    mem.set_vector(vectors::IRQ, 0x0300);

    cpu.a = 0xAA;
    cpu.b = 0xBB;
    cpu.x = 0x1234;
    cpu.cc.i = false;
    cpu.cc.c = true;
    let cc_bits = cpu.cc.bits();

    cpu.set_irq(true);
    assert_eq!(cpu.step(&mut mem), 12);

    assert_eq!(cpu.pc, 0x0300, "jump to the IRQ vector");
    assert!(cpu.cc.i, "the IRQ is left masked on entry");
    assert_eq!(cpu.sp, 0x00F8, "7 bytes pushed");

    // Push order: PCL, PCH, IXL, IXH, ACCA, ACCB, CCR.
    assert_eq!(mem.bytes[0x00FF], 0x00, "PCL");
    assert_eq!(mem.bytes[0x00FE], 0x01, "PCH");
    assert_eq!(mem.bytes[0x00FD], 0x34, "IXL");
    assert_eq!(mem.bytes[0x00FC], 0x12, "IXH");
    assert_eq!(mem.bytes[0x00FB], 0xAA, "ACCA");
    assert_eq!(mem.bytes[0x00FA], 0xBB, "ACCB");
    assert_eq!(mem.bytes[0x00F9], cc_bits, "CCR");
}

#[test]
fn the_mask_blocks_the_irq_but_not_the_nmi() {
    let (mut cpu, mut mem) = boot(&[0x01, 0x01]);
    mem.set_vector(vectors::IRQ, 0x0300);
    mem.set_vector(vectors::NMI, 0x0400);
    cpu.cc.i = true;

    cpu.set_irq(true);
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0101, "the IRQ was masked, the NOP ran");

    cpu.set_nmi(true);
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0400, "the NMI ignores the mask");
}

#[test]
fn the_nmi_triggers_on_the_edge() {
    let (mut cpu, mut mem) = boot(&[0x01, 0x01, 0x01]);
    mem.set_vector(vectors::NMI, 0x0400);
    mem.load(0x0400, &[0x3B]); // RTI

    cpu.set_nmi(true);
    cpu.step(&mut mem); // services the NMI
    assert_eq!(cpu.pc, 0x0400);

    cpu.step(&mut mem); // RTI
    assert_eq!(cpu.pc, 0x0100);

    // The line is still high, but without a fresh edge it does not fire again.
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0101, "the NOP ran, it did not re-enter the NMI");
}

#[test]
fn rti_restores_the_whole_context() {
    let (mut cpu, mut mem) = boot(&[0x01]);
    mem.set_vector(vectors::IRQ, 0x0300);
    mem.load(0x0300, &[0x3B]); // RTI

    cpu.a = 0x11;
    cpu.b = 0x22;
    cpu.x = 0x3344;
    cpu.cc.i = false;
    cpu.cc.v = true;

    cpu.set_irq(true);
    cpu.step(&mut mem);
    cpu.set_irq(false);

    // We dirty everything before the RTI.
    cpu.a = 0;
    cpu.b = 0;
    cpu.x = 0;

    assert_eq!(cpu.step(&mut mem), 10, "RTI is 10 cycles");
    assert_eq!(cpu.a, 0x11);
    assert_eq!(cpu.b, 0x22);
    assert_eq!(cpu.x, 0x3344);
    assert_eq!(cpu.pc, 0x0100);
    assert!(!cpu.cc.i, "the original CCR had the mask clear");
    assert!(cpu.cc.v);
    assert_eq!(cpu.sp, 0x00FF, "the stack was left as it was");
}

#[test]
fn swi_uses_its_own_vector() {
    let (mut cpu, mut mem) = boot(&[0x3F]); // SWI
    mem.set_vector(vectors::SWI, 0x0500);

    assert_eq!(cpu.step(&mut mem), 12);
    assert_eq!(cpu.pc, 0x0500);
    assert!(cpu.cc.i);
    assert_eq!(cpu.sp, 0x00F8);
}

#[test]
fn wai_does_not_push_twice() {
    // WAI leaves the context on the stack and halts. The interrupt that wakes
    // it up must not push it again, or the stack gets corrupted.
    let (mut cpu, mut mem) = boot(&[0x3E]); // WAI
    mem.set_vector(vectors::IRQ, 0x0300);
    cpu.cc.i = false;

    assert_eq!(cpu.step(&mut mem), 9);
    assert!(cpu.is_waiting());
    assert_eq!(cpu.sp, 0x00F8, "WAI pushed the 7 bytes");

    // With no interrupt it stays halted.
    cpu.step(&mut mem);
    assert!(cpu.is_waiting());
    assert_eq!(cpu.sp, 0x00F8);

    cpu.set_irq(true);
    cpu.step(&mut mem);

    assert!(!cpu.is_waiting());
    assert_eq!(cpu.pc, 0x0300);
    assert_eq!(cpu.sp, 0x00F8, "the context was not pushed again");
}

// --- Complete program ------------------------------------------------------------

#[test]
fn sums_a_table_of_bytes() {
    // A real loop: it walks 5 bytes from $0200 and accumulates into B.
    //
    //        LDX  #$0200
    //        CLRB
    //        LDAA #$05
    // loop:  ADDB $00,X
    //        INX
    //        DECA
    //        BNE  loop
    //        SWI
    let program = [
        0xCE, 0x02, 0x00, // LDX #$0200
        0x5F, // CLRB
        0x86, 0x05, // LDAA #$05
        0xEB, 0x00, // ADDB $00,X
        0x08, // INX
        0x4A, // DECA
        0x26, 0xFA, // BNE -6
        0x3F, // SWI
    ];
    let (mut cpu, mut mem) = boot(&program);
    mem.load(0x0200, &[10, 20, 30, 40, 50]);
    mem.set_vector(vectors::SWI, 0x9000);

    // With a cap, so that a control-flow bug fails the test instead of hanging
    // it.
    for _ in 0..200 {
        cpu.step(&mut mem);
        if cpu.pc == 0x9000 {
            break;
        }
    }

    assert_eq!(cpu.pc, 0x9000, "the program did not finish");
    assert_eq!(cpu.b, 150, "10+20+30+40+50");
    assert_eq!(cpu.x, 0x0205);
    assert_eq!(cpu.last_illegal, None);
}

#[test]
fn does_not_run_undefined_opcodes_silently() {
    // 0x87 would be "immediate STAA", which makes no sense: there is nowhere
    // to store to. The 6800 does not define it.
    let (mut cpu, mut mem) = boot(&[0x87, 0x00]);
    cpu.step(&mut mem);
    assert_eq!(cpu.last_illegal, Some((0x0100, 0x87)));
}

#[test]
fn hcf_leaves_the_cpu_unusable() {
    // 0x9D on the MC6800 is HCF: the PC runs away and the CPU stops responding
    // to interrupts. The 6801 replaced this opcode with a direct JSR, but
    // System 11 carries a 6808.
    let (mut cpu, mut mem) = boot(&[0x9D]);
    mem.set_vector(vectors::NMI, 0x0400);

    cpu.step(&mut mem);
    assert!(cpu.is_hcf());

    let pc_before = cpu.pc;
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, pc_before.wrapping_add(1), "the PC keeps running");

    // Not even the NMI wakes it up.
    cpu.set_nmi(true);
    cpu.step(&mut mem);
    assert_ne!(cpu.pc, 0x0400, "HCF ignores interrupts");
    assert!(cpu.is_hcf());

    // Only the reset recovers it.
    cpu.reset(&mut mem);
    assert!(!cpu.is_hcf());
    assert_eq!(cpu.pc, 0x0100);
}

#[test]
fn the_other_hcf_is_0xdd() {
    let (mut cpu, mut mem) = boot(&[0xDD]);
    cpu.step(&mut mem);
    assert!(cpu.is_hcf());
}
