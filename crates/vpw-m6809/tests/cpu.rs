//! Tests of the MC6809 core, against the Motorola specification.

use vpw_m6809::{Cpu, FlatMemory, vectors};

/// CPU reset with the program at `$0100` and a system stack at `$0200`.
fn boot(program: &[u8]) -> (Cpu, FlatMemory) {
    let mut mem = FlatMemory::new();
    mem.load(0x0100, program);
    mem.set_vector(vectors::RESET, 0x0100);

    let mut cpu = Cpu::new();
    cpu.reset(&mut mem);
    cpu.s = 0x0200;
    cpu.u = 0x0300;
    (cpu, mem)
}

// --- Registers ------------------------------------------------------------------

#[test]
fn d_is_the_a_b_pair() {
    let mut cpu = Cpu::new();
    cpu.a = 0x12;
    cpu.b = 0x34;
    assert_eq!(cpu.d(), 0x1234, "A is the high byte");

    cpu.set_d(0xABCD);
    assert_eq!(cpu.a, 0xAB);
    assert_eq!(cpu.b, 0xCD);
}

#[test]
fn reset_sets_both_masks() {
    let (cpu, _) = boot(&[0x12]);
    assert!(cpu.cc.i, "IRQ masked");
    assert!(cpu.cc.f, "FIRQ masked");
    assert_eq!(cpu.dp, 0, "the direct page starts at zero");
}

// --- Addressing modes -----------------------------------------------------------

#[test]
fn direct_mode_uses_the_dp_page() {
    // LDA $30 with DP = $12 reads from $1230, not from $0030.
    let (mut cpu, mut mem) = boot(&[0x96, 0x30]);
    cpu.dp = 0x12;
    mem.bytes[0x1230] = 0x7E;
    mem.bytes[0x0030] = 0xEE;

    assert_eq!(cpu.step(&mut mem), 4);
    assert_eq!(cpu.a, 0x7E);
}

#[test]
fn the_indexed_5_bit_offset_is_signed() {
    // LDA -1,X  ->  postbyte 0b0001_1111
    let (mut cpu, mut mem) = boot(&[0xA6, 0b0001_1111]);
    cpu.x = 0x0500;
    mem.bytes[0x04FF] = 0x42;

    assert_eq!(cpu.step(&mut mem), 5, "4 base + 1 for the short offset");
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.x, 0x0500, "the base register does not move");
}

#[test]
fn auto_increment_advances_after_use() {
    // LDA ,X+  ->  postbyte 0b1000_0000
    let (mut cpu, mut mem) = boot(&[0xA6, 0b1000_0000]);
    cpu.x = 0x0500;
    mem.bytes[0x0500] = 0x11;

    assert_eq!(cpu.step(&mut mem), 6, "4 base + 2");
    assert_eq!(cpu.a, 0x11, "it read from the old address");
    assert_eq!(cpu.x, 0x0501, "and advanced afterwards");
}

#[test]
fn auto_decrement_steps_back_before_use() {
    // LDA ,-X  ->  postbyte 0b1000_0010
    let (mut cpu, mut mem) = boot(&[0xA6, 0b1000_0010]);
    cpu.x = 0x0500;
    mem.bytes[0x04FF] = 0x22;

    assert_eq!(cpu.step(&mut mem), 6);
    assert_eq!(cpu.a, 0x22, "first it stepped back, then it read");
    assert_eq!(cpu.x, 0x04FF);
}

#[test]
fn double_auto_increment_advances_by_two() {
    // LDD ,Y++  ->  postbyte 0b1010_0001
    let (mut cpu, mut mem) = boot(&[0xEC, 0b1010_0001]);
    cpu.y = 0x0600;
    mem.load(0x0600, &[0xAB, 0xCD]);

    cpu.step(&mut mem);
    assert_eq!(cpu.d(), 0xABCD);
    assert_eq!(cpu.y, 0x0602);
}

#[test]
fn the_offset_can_come_from_an_accumulator() {
    // LDA B,X  ->  postbyte 0b1000_0101
    let (mut cpu, mut mem) = boot(&[0xA6, 0b1000_0101]);
    cpu.x = 0x0500;
    cpu.b = 0xFF; // -1 signed
    mem.bytes[0x04FF] = 0x33;

    assert_eq!(cpu.step(&mut mem), 5);
    assert_eq!(cpu.a, 0x33, "the accumulator offset is signed");
}

#[test]
fn the_16_bit_offset_from_d() {
    // LDA D,U  ->  postbyte 0b1100_1011
    let (mut cpu, mut mem) = boot(&[0xA6, 0b1100_1011]);
    cpu.u = 0x0400;
    cpu.set_d(0x0100);
    mem.bytes[0x0500] = 0x44;

    assert_eq!(cpu.step(&mut mem), 8, "4 base + 4");
    assert_eq!(cpu.a, 0x44);
}

#[test]
fn pc_relative_mode_counts_from_after_the_offset() {
    // LDA n,PCR with an 8-bit offset -> postbyte 0b1000_1100
    let (mut cpu, mut mem) = boot(&[0xA6, 0b1000_1100, 0x10]);
    // After reading the offset the PC is $0103, so the address is $0113.
    mem.bytes[0x0113] = 0x55;

    assert_eq!(cpu.step(&mut mem), 5);
    assert_eq!(cpu.a, 0x55);
}

#[test]
fn indirection_adds_one_more_hop() {
    // LDA [,X]  ->  postbyte 0b1001_0100 (bit 4 = indirect)
    let (mut cpu, mut mem) = boot(&[0xA6, 0b1001_0100]);
    cpu.x = 0x0500;
    mem.load(0x0500, &[0x06, 0x00]); // at $0500 sits the address $0600
    mem.bytes[0x0600] = 0x66;

    assert_eq!(
        cpu.step(&mut mem),
        7,
        "4 base + 0 for the mode + 3 for the indirection"
    );
    assert_eq!(cpu.a, 0x66, "it read from the final destination");
}

#[test]
fn extended_indirect_carries_the_address_in_the_code() {
    // LDA [$0600]  ->  postbyte 0b1001_1111 + address
    let (mut cpu, mut mem) = boot(&[0xA6, 0b1001_1111, 0x06, 0x00]);
    mem.load(0x0600, &[0x07, 0x00]);
    mem.bytes[0x0700] = 0x77;

    assert_eq!(cpu.step(&mut mem), 9, "4 base + 2 + 3");
    assert_eq!(cpu.a, 0x77);
}

#[test]
fn all_four_registers_can_be_a_base() {
    for (bits, name) in [(0b00, "X"), (0b01, "Y"), (0b10, "U"), (0b11, "S")] {
        let postbyte = 0b1000_0100 | (bits << 5); // ,R
        let (mut cpu, mut mem) = boot(&[0xA6, postbyte]);
        cpu.x = 0x0500;
        cpu.y = 0x0500;
        cpu.u = 0x0500;
        cpu.s = 0x0500;
        mem.bytes[0x0500] = 0x88;

        cpu.step(&mut mem);
        assert_eq!(cpu.a, 0x88, "register {name} did not work as a base");
    }
}

// --- 16-bit arithmetic -----------------------------------------------------------

#[test]
fn addd_adds_16_bits_at_a_time() {
    let (mut cpu, mut mem) = boot(&[0xC3, 0x10, 0x00]); // ADDD #$1000
    cpu.set_d(0x2345);

    assert_eq!(cpu.step(&mut mem), 4);
    assert_eq!(cpu.d(), 0x3345);
    assert!(!cpu.cc.c);
}

#[test]
fn subd_flags_a_16_bit_borrow() {
    let (mut cpu, mut mem) = boot(&[0x83, 0x00, 0x02]); // SUBD #$0002
    cpu.set_d(0x0001);

    cpu.step(&mut mem);
    assert_eq!(cpu.d(), 0xFFFF);
    assert!(cpu.cc.c, "1 - 2 borrows");
}

#[test]
fn the_16_bit_cmp_does_touch_the_carry() {
    // The 6800 left the carry untouched in CPX; the 6809 fixed it.
    let (mut cpu, mut mem) = boot(&[0x8C, 0xFF, 0xFF]); // CMPX #$FFFF
    cpu.x = 0x0000;
    cpu.cc.c = false;

    cpu.step(&mut mem);
    assert!(cpu.cc.c, "0 - 0xFFFF borrows, and the 6809 reports it");
    assert!(!cpu.cc.z);
}

#[test]
fn mul_multiplies_unsigned() {
    let (mut cpu, mut mem) = boot(&[0x3D]); // MUL
    cpu.a = 0xFF;
    cpu.b = 0xFF;

    assert_eq!(cpu.step(&mut mem), 11);
    assert_eq!(cpu.d(), 0xFE01, "255 * 255");
    assert!(!cpu.cc.z);
    assert!(!cpu.cc.c, "the carry comes out of bit 7 of the result");
}

#[test]
fn mul_leaves_the_carry_for_rounding() {
    let (mut cpu, mut mem) = boot(&[0x3D]);
    cpu.a = 0x02;
    cpu.b = 0x40; // 2 * 64 = 128 = 0x0080

    cpu.step(&mut mem);
    assert_eq!(cpu.d(), 0x0080);
    assert!(cpu.cc.c, "bit 7 is set: it is there to round to a byte");
}

#[test]
fn sex_extends_the_sign_of_b() {
    let (mut cpu, mut mem) = boot(&[0x1D, 0x1D]); // SEX ; SEX
    cpu.b = 0x80; // negative
    assert_eq!(cpu.step(&mut mem), 2);
    assert_eq!(cpu.d(), 0xFF80);
    assert!(cpu.cc.n);

    cpu.b = 0x7F; // positive
    cpu.step(&mut mem);
    assert_eq!(cpu.d(), 0x007F);
    assert!(!cpu.cc.n);
}

#[test]
fn abx_adds_b_unsigned_and_without_flags() {
    let (mut cpu, mut mem) = boot(&[0x3A]); // ABX
    cpu.x = 0x1000;
    cpu.b = 0xFF;
    cpu.cc.c = true;

    assert_eq!(cpu.step(&mut mem), 3);
    assert_eq!(
        cpu.x, 0x10FF,
        "unsigned: it adds 255, it does not subtract 1"
    );
    assert!(cpu.cc.c, "ABX does not touch the flags");
}

// --- Transfers ----------------------------------------------------------------------

#[test]
fn tfr_copies_between_16_bit_registers() {
    let (mut cpu, mut mem) = boot(&[0x1F, 0x12]); // TFR X,Y
    cpu.x = 0xBEEF;

    assert_eq!(cpu.step(&mut mem), 6);
    assert_eq!(cpu.y, 0xBEEF);
    assert_eq!(cpu.x, 0xBEEF, "the source does not change");
}

#[test]
fn exg_swaps() {
    let (mut cpu, mut mem) = boot(&[0x1E, 0x01]); // EXG D,X
    cpu.set_d(0x1111);
    cpu.x = 0x2222;

    assert_eq!(cpu.step(&mut mem), 8);
    assert_eq!(cpu.d(), 0x2222);
    assert_eq!(cpu.x, 0x1111);
}

#[test]
fn tfr_between_8_bit_registers() {
    let (mut cpu, mut mem) = boot(&[0x1F, 0x8B]); // TFR A,DP
    cpu.a = 0x40;

    cpu.step(&mut mem);
    assert_eq!(cpu.dp, 0x40);
}

// --- Stack ---------------------------------------------------------------------------

#[test]
fn the_stack_pre_decrements() {
    // The other way round from the 6800: it points at the last byte stored.
    let (mut cpu, mut mem) = boot(&[0x34, 0x02]); // PSHS A
    cpu.a = 0x5A;

    assert_eq!(cpu.step(&mut mem), 6, "5 base + 1 byte");
    assert_eq!(cpu.s, 0x01FF);
    assert_eq!(mem.bytes[0x01FF], 0x5A);
}

#[test]
fn pshs_and_puls_respect_the_order() {
    // PSHS CC,A,B,DP,X,Y,U,PC  ;  PULS everything
    let (mut cpu, mut mem) = boot(&[0x34, 0xFF, 0x35, 0xFF]);
    cpu.a = 0x11;
    cpu.b = 0x22;
    cpu.dp = 0x33;
    cpu.x = 0x4444;
    cpu.y = 0x5555;
    cpu.u = 0x6666;

    assert_eq!(cpu.step(&mut mem), 17, "5 base + 12 bytes");
    assert_eq!(cpu.s, 0x0200 - 12);

    // Dirty everything and pull it back.
    cpu.a = 0;
    cpu.b = 0;
    cpu.dp = 0;
    cpu.x = 0;
    cpu.y = 0;
    cpu.u = 0;
    cpu.step(&mut mem);

    assert_eq!(cpu.a, 0x11);
    assert_eq!(cpu.b, 0x22);
    assert_eq!(cpu.dp, 0x33);
    assert_eq!(cpu.x, 0x4444);
    assert_eq!(cpu.y, 0x5555);
    assert_eq!(cpu.u, 0x6666);
    assert_eq!(cpu.s, 0x0200);
}

#[test]
fn pshu_uses_the_other_stack() {
    let (mut cpu, mut mem) = boot(&[0x36, 0x02]); // PSHU A
    cpu.a = 0x77;

    cpu.step(&mut mem);
    assert_eq!(cpu.u, 0x02FF);
    assert_eq!(cpu.s, 0x0200, "the system stack is not touched");
    assert_eq!(mem.bytes[0x02FF], 0x77);
}

#[test]
fn bit_6_means_the_other_stack() {
    // PSHS U saves U; PSHU S saves S.
    let (mut cpu, mut mem) = boot(&[0x34, 0x40, 0x36, 0x40]);

    // PSHS U saves the value of U on the system stack.
    cpu.u = 0x0300;
    let original_u = cpu.u;
    cpu.step(&mut mem);
    assert_eq!(mem.bytes[0x01FE], 0x03, "the high byte of U");
    assert_eq!(mem.bytes[0x01FF], 0x00, "and the low one");

    // PSHU S saves the value of S on the user stack.
    cpu.step(&mut mem);
    assert_eq!(cpu.u, original_u - 2, "it wrote to the user stack");
    assert_eq!(
        mem.bytes[0x02FE], 0x01,
        "the high byte of S, which ended up at $01FE"
    );
    assert_eq!(mem.bytes[0x02FF], 0xFE);
}

#[test]
fn lea_adjusts_the_stack_without_losing_the_flags() {
    // LEAS -2,S does not touch Z; LEAX does.
    // LEAS -2,S: the 5-bit offset in two's complement is 0b11110.
    let (mut cpu, mut mem) = boot(&[0x32, 0b0111_1110, 0x30, 0b1000_0100]);
    cpu.cc.z = true;
    cpu.step(&mut mem);
    assert_eq!(cpu.s, 0x01FE);
    assert!(cpu.cc.z, "LEAS does not touch the flags");

    cpu.x = 0x0000;
    cpu.step(&mut mem); // LEAX ,X
    assert!(cpu.cc.z, "LEAX does affect Z, and here it came out zero");
}

// --- Branches -------------------------------------------------------------------------

#[test]
fn brn_never_branches() {
    // It exists so a branch can be disabled by changing a single byte.
    let (mut cpu, mut mem) = boot(&[0x21, 0x10]);
    assert_eq!(cpu.step(&mut mem), 3);
    assert_eq!(cpu.pc, 0x0102);
}

#[test]
fn long_branches_reach_the_whole_map() {
    // LBRA with a 16-bit offset.
    let (mut cpu, mut mem) = boot(&[0x16, 0x10, 0x00]);
    assert_eq!(cpu.step(&mut mem), 5);
    assert_eq!(cpu.pc, 0x1103);
}

#[test]
fn a_conditional_long_branch_costs_according_to_whether_it_is_taken() {
    // LBNE: on page $10.
    let (mut cpu, mut mem) = boot(&[0x10, 0x26, 0x00, 0x10]);
    cpu.cc.z = true; // not taken
    assert_eq!(cpu.step(&mut mem), 5);
    assert_eq!(cpu.pc, 0x0104);

    let (mut cpu, mut mem) = boot(&[0x10, 0x26, 0x00, 0x10]);
    cpu.cc.z = false; // taken
    assert_eq!(cpu.step(&mut mem), 6, "taking it costs one cycle more");
    assert_eq!(cpu.pc, 0x0114);
}

#[test]
fn bsr_and_rts_make_the_round_trip() {
    let (mut cpu, mut mem) = boot(&[0x8D, 0x10]); // BSR +$10
    mem.load(0x0112, &[0x39]); // RTS

    assert_eq!(cpu.step(&mut mem), 7);
    assert_eq!(cpu.pc, 0x0112);
    assert_eq!(cpu.s, 0x01FE);

    assert_eq!(cpu.step(&mut mem), 5);
    assert_eq!(cpu.pc, 0x0102);
    assert_eq!(cpu.s, 0x0200);
}

// --- Flags that differ from the 6800 ----------------------------------------------------

#[test]
fn tst_does_not_touch_the_carry() {
    let (mut cpu, mut mem) = boot(&[0x4D]); // TSTA
    cpu.a = 0x00;
    cpu.cc.c = true;

    cpu.step(&mut mem);
    assert!(cpu.cc.z);
    assert!(cpu.cc.c, "on the 6809 TST leaves the carry as it was");
}

#[test]
fn the_asl_overflow_comes_from_the_top_two_bits() {
    // 0x40 << 1 = 0x80: the sign changes, so there is overflow.
    let (mut cpu, mut mem) = boot(&[0x48, 0x48]); // ASLA ; ASLA
    cpu.a = 0x40;
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x80);
    assert!(cpu.cc.v, "b7 xor b6 of the original");
    assert!(!cpu.cc.c);

    // 0xC0 << 1 = 0x80: the top two bits were equal, there is no overflow.
    cpu.a = 0xC0;
    cpu.step(&mut mem);
    assert_eq!(cpu.a, 0x80);
    assert!(!cpu.cc.v);
    assert!(cpu.cc.c);
}

// --- Interrupts ----------------------------------------------------------------------------

#[test]
fn the_irq_stacks_the_entire_state() {
    let (mut cpu, mut mem) = boot(&[0x12]); // NOP
    mem.set_vector(vectors::IRQ, 0x0400);
    cpu.a = 0x11;
    cpu.b = 0x22;
    cpu.dp = 0x33;
    cpu.x = 0x4444;
    cpu.y = 0x5555;
    cpu.u = 0x6666;
    cpu.cc.i = false;
    cpu.cc.f = false;

    cpu.set_irq(true);
    assert_eq!(cpu.step(&mut mem), 19);

    assert_eq!(cpu.pc, 0x0400);
    assert!(cpu.cc.i, "the IRQ ends up masked");
    assert!(
        !cpu.cc.f,
        "but the FIRQ does not: only the fast one does that"
    );
    assert_eq!(cpu.s, 0x0200 - 12, "twelve bytes stacked");

    // Stacking order, from $01FF downwards:
    // PCL, PCH, UL, UH, YL, YH, XL, XH, DP, B, A, CC.
    assert_eq!(mem.bytes[0x01FF], 0x00, "PCL");
    assert_eq!(mem.bytes[0x01FE], 0x01, "PCH");
    assert_eq!(mem.bytes[0x01FD], 0x66, "UL");
    assert_eq!(mem.bytes[0x01FC], 0x66, "UH");
    assert_eq!(mem.bytes[0x01FB], 0x55, "YL");
    assert_eq!(mem.bytes[0x01FA], 0x55, "YH");
    assert_eq!(mem.bytes[0x01F9], 0x44, "XL");
    assert_eq!(mem.bytes[0x01F8], 0x44, "XH");
    assert_eq!(mem.bytes[0x01F7], 0x33, "DP");
    assert_eq!(mem.bytes[0x01F6], 0x22, "B");
    assert_eq!(mem.bytes[0x01F5], 0x11, "A");
    assert_eq!(mem.bytes[0x01F4] & 0x80, 0x80, "the E flag ended up set");
}

#[test]
fn the_firq_stacks_only_pc_and_cc() {
    // That is where its name comes from: it is fast because it saves almost
    // nothing.
    let (mut cpu, mut mem) = boot(&[0x12]);
    mem.set_vector(vectors::FIRQ, 0x0500);
    cpu.cc.f = false;
    cpu.cc.i = false;

    cpu.set_firq(true);
    assert_eq!(cpu.step(&mut mem), 10, "against the 19 of an IRQ");

    assert_eq!(cpu.pc, 0x0500);
    assert_eq!(cpu.s, 0x0200 - 3, "only three bytes");
    assert!(cpu.cc.f, "it masks both");
    assert!(cpu.cc.i);
    assert_eq!(mem.bytes[0x01FD] & 0x80, 0x00, "the E flag ended up clear");
}

#[test]
fn rti_pulls_according_to_the_e_flag() {
    // After a FIRQ the RTI only has to take three bytes off.
    let (mut cpu, mut mem) = boot(&[0x12]);
    mem.set_vector(vectors::FIRQ, 0x0500);
    mem.load(0x0500, &[0x3B]); // RTI
    cpu.cc.f = false;

    cpu.set_firq(true);
    cpu.step(&mut mem);
    cpu.set_firq(false);

    assert_eq!(cpu.step(&mut mem), 6, "short RTI");
    assert_eq!(cpu.pc, 0x0100);
    assert_eq!(cpu.s, 0x0200);
}

#[test]
fn the_long_rti_restores_everything() {
    let (mut cpu, mut mem) = boot(&[0x12]);
    mem.set_vector(vectors::IRQ, 0x0400);
    mem.load(0x0400, &[0x3B]); // RTI
    cpu.a = 0x11;
    cpu.x = 0x4444;
    cpu.dp = 0x33;
    cpu.cc.i = false;

    cpu.set_irq(true);
    cpu.step(&mut mem);
    cpu.set_irq(false);

    cpu.a = 0;
    cpu.x = 0;
    cpu.dp = 0;

    assert_eq!(cpu.step(&mut mem), 15, "long RTI");
    assert_eq!(cpu.a, 0x11);
    assert_eq!(cpu.x, 0x4444);
    assert_eq!(cpu.dp, 0x33);
    assert_eq!(cpu.pc, 0x0100);
    assert!(!cpu.cc.i, "it restored the CC from before");
}

#[test]
fn the_firq_has_priority_over_the_irq() {
    let (mut cpu, mut mem) = boot(&[0x12]);
    mem.set_vector(vectors::IRQ, 0x0400);
    mem.set_vector(vectors::FIRQ, 0x0500);
    cpu.cc.i = false;
    cpu.cc.f = false;

    cpu.set_irq(true);
    cpu.set_firq(true);
    cpu.step(&mut mem);

    assert_eq!(cpu.pc, 0x0500, "it serviced the fast one first");
}

#[test]
fn the_nmi_ignores_both_masks() {
    let (mut cpu, mut mem) = boot(&[0x12, 0x12]);
    mem.set_vector(vectors::NMI, 0x0600);
    cpu.cc.i = true;
    cpu.cc.f = true;

    cpu.set_nmi(true);
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0600);
}

#[test]
fn cwai_does_not_stack_twice() {
    let (mut cpu, mut mem) = boot(&[0x3C, 0xEF]); // CWAI #$EF (clears the I mask)
    mem.set_vector(vectors::IRQ, 0x0400);
    cpu.cc.i = true;

    assert_eq!(cpu.step(&mut mem), 20);
    assert!(cpu.is_waiting());
    assert!(!cpu.cc.i, "the operand's mask was applied");
    assert_eq!(cpu.s, 0x0200 - 12, "it stacked the entire state");

    cpu.set_irq(true);
    cpu.step(&mut mem);

    assert!(!cpu.is_waiting());
    assert_eq!(cpu.pc, 0x0400);
    assert_eq!(cpu.s, 0x0200 - 12, "it did not stack again");
}

#[test]
fn sync_wakes_on_the_line_even_when_masked() {
    let (mut cpu, mut mem) = boot(&[0x13, 0x12]); // SYNC ; NOP
    cpu.cc.i = true; // masked

    cpu.step(&mut mem);
    assert!(cpu.is_syncing());

    cpu.step(&mut mem);
    assert!(cpu.is_syncing(), "with no line it stays halted");

    cpu.set_irq(true);
    cpu.step(&mut mem);
    assert!(!cpu.is_syncing(), "the line wakes it up");
    assert_eq!(
        cpu.pc, 0x0101,
        "and it carries straight on, because it was masked"
    );
}

#[test]
fn swi_and_swi2_differ_in_the_masks() {
    // SWI masks both; SWI2 touches neither, because it is a system call and
    // not an error condition.
    let (mut cpu, mut mem) = boot(&[0x3F]);
    mem.set_vector(vectors::SWI, 0x0700);
    cpu.cc.i = false;
    cpu.cc.f = false;
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0700);
    assert!(cpu.cc.i && cpu.cc.f);

    let (mut cpu, mut mem) = boot(&[0x10, 0x3F]);
    mem.set_vector(vectors::SWI2, 0x0800);
    cpu.cc.i = false;
    cpu.cc.f = false;
    cpu.step(&mut mem);
    assert_eq!(cpu.pc, 0x0800);
    assert!(!cpu.cc.i && !cpu.cc.f, "SWI2 masks nothing");
}

// --- Complete program -----------------------------------------------------------------------

#[test]
fn sums_a_table_with_auto_increment() {
    // What the 6809 does in two instructions and the 6800 in four:
    //
    //        LDX  #$0500
    //        CLRB
    //        LDA  #5
    // loop:  ADDB ,X+
    //        DECA
    //        BNE  loop
    //        SWI
    let program = [
        0x8E, 0x05, 0x00, // LDX #$0500
        0x5F, // CLRB
        0x86, 0x05, // LDA #5
        0xEB, 0x80, // ADDB ,X+
        0x4A, // DECA
        0x26, 0xFB, // BNE -5
        0x3F, // SWI
    ];
    let (mut cpu, mut mem) = boot(&program);
    mem.load(0x0500, &[10, 20, 30, 40, 50]);
    mem.set_vector(vectors::SWI, 0x9000);

    for _ in 0..200 {
        cpu.step(&mut mem);
        if cpu.pc == 0x9000 {
            break;
        }
    }

    assert_eq!(cpu.pc, 0x9000, "the program did not finish");
    assert_eq!(cpu.b, 150);
    assert_eq!(cpu.x, 0x0505, "the auto-increment walked the table");
    assert_eq!(cpu.last_illegal, None);
}

#[test]
fn does_not_execute_undefined_opcodes_silently() {
    let (mut cpu, mut mem) = boot(&[0x01]);
    cpu.step(&mut mem);
    assert_eq!(cpu.last_illegal, Some((0x0100, 0x01)));
}
