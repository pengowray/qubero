//! RISC-V as a small machine runs it: the 32-bit base, the compressed
//! encodings, and the extensions a real microcontroller ships with.
//!
//! There is a decoder crate for RISC-V already, and this file exists because
//! of what it does with an encoding it has not been taught. RISC-V puts the
//! extensions in the spare corners of the base opcodes: `sh2add` is `slt` with
//! a different `funct7`, `andn` is `and` with a different one, `pack` is
//! `xor`. A decoder that reads the opcode and the `funct3` and stops has an
//! answer for all of them, and the answer is the base instruction, confidently
//! named and wrong. Measured against the RP2350 boot ROM's own listing, that
//! is 162 instructions read as something they are not, which is worse than
//! reading nothing: nothing is visibly nothing.
//!
//! So this decodes the whole word and refuses anything it cannot place. What
//! it covers is what a Raspberry Pi Pico 2's Hazard3 core implements:
//! RV32IMAC, the CSR and fence instructions, the bit manipulation sets
//! (Zba, Zbb, Zbs, Zbkb), the extra compressed encodings (Zcb, Zcmp), and
//! Hazard3's own instructions.
//!
//! Those last ones are only read when the caller says the machine is a
//! Hazard3. `h3.block` is `slt x0, x0, x0` — a hint the base standard leaves
//! for a vendor to define, and which any other RISC-V chip is free to define
//! differently or ignore. Naming it Hazard3's way in a file that never said it
//! was for a Hazard3 would be inventing a fact about the program.

use std::fmt::Write as _;

use crate::code::Insn;

/// Registers by the names an assembler and a debugger use, rather than by
/// number: `sp` and `ra` say what they are for, `x2` and `x1` do not.
const X: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "s2",
    "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4", "t5", "t6",
];

/// The eight registers a compressed instruction can name, which are the ones a
/// compiler uses most.
fn xc(three: u32) -> &'static str {
    X[(three & 7) as usize + 8]
}

fn x(five: u32) -> &'static str {
    X[(five & 31) as usize]
}

/// The saved registers a `cm.mvsa01` names, in the order that extension counts
/// them: the first two are `s0` and `s1`, and the rest carry on from `s2`.
fn sreg(three: u32) -> &'static str {
    match three & 7 {
        0 => "s0",
        1 => "s1",
        n => X[16 + n as usize],
    }
}

/// Decode one instruction. `hazard3` says whether the vendor encodings may be
/// read; without it the same bytes come back as whatever the base standard
/// says they are, which for `h3.block` is a `slt` that does nothing.
pub fn decode(bytes: &[u8], hazard3: bool) -> Option<Insn> {
    let low = u16::from_le_bytes([*bytes.first()?, *bytes.get(1)?]);
    // A word of zeroes is the encoding the standard sets aside to mean that
    // this is deliberately not an instruction, which is what a compiler puts
    // where control must never arrive.
    if low == 0 {
        return Some(Insn { len: 2, text: "unimp".to_string(), target: None });
    }
    // The low two bits say how long the instruction is: anything but `11` is
    // one of the two-byte compressed encodings.
    if low & 3 != 3 {
        return compressed(low, hazard3);
    }
    if bytes.len() < 4 {
        return None;
    }
    full(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]), hazard3)
}

// ---------------------------------------------------------------- four bytes

fn full(w: u32, hazard3: bool) -> Option<Insn> {
    let (rd, rs1, rs2) = ((w >> 7) & 31, (w >> 15) & 31, (w >> 20) & 31);
    let funct3 = (w >> 12) & 7;
    let funct7 = w >> 25;
    let text = match w & 0x7f {
        // Both instructions that build a big constant write it as the top
        // twenty bits, which is how an assembler wants it read back.
        0x37 => format!("lui {}, 0x{:x}", x(rd), w >> 12),
        0x17 => format!("auipc {}, 0x{:x}", x(rd), w >> 12),
        0x6f => {
            let off = jal_offset(w);
            return Some(Insn { len: 4, text: format!("jal {}, {}", x(rd), rel(off)), target: Some(off as i64) });
        }
        0x67 if funct3 == 0 => format!("jalr {}, {}({})", x(rd), imm_i(w), x(rs1)),
        0x63 => {
            let name = ["beq", "bne", "", "", "blt", "bge", "bltu", "bgeu"][funct3 as usize];
            if name.is_empty() {
                return None;
            }
            let off = branch_offset(w);
            let text = format!("{name} {}, {}, {}", x(rs1), x(rs2), rel(off));
            return Some(Insn { len: 4, text, target: Some(off as i64) });
        }
        0x03 => {
            let name = ["lb", "lh", "lw", "", "lbu", "lhu", "", ""][funct3 as usize];
            if name.is_empty() {
                return None;
            }
            format!("{name} {}, {}({})", x(rd), imm_i(w), x(rs1))
        }
        0x23 => {
            let name = ["sb", "sh", "sw", "", "", "", "", ""][funct3 as usize];
            if name.is_empty() {
                return None;
            }
            format!("{name} {}, {}({})", x(rs2), imm_s(w), x(rs1))
        }
        0x13 => op_imm(w, rd, rs1, rs2, funct3, funct7)?,
        0x33 => op(w, rd, rs1, rs2, funct3, funct7)?,
        0x0f => match funct3 {
            0 => fence(w),
            1 => "fence.i".to_string(),
            _ => return None,
        },
        0x2f if funct3 == 2 => atomic(w, rd, rs1, rs2)?,
        0x73 => system(w, rd, rs1, rs2, funct3)?,
        // Hazard3 puts its bit extraction in the first of the two opcodes the
        // standard sets aside for a vendor.
        0x0b if hazard3 => {
            // Bits 31:29 are reserved and must be zero; bits 28:26 hold one
            // less than the number of bits the mask keeps.
            if w >> 29 != 0 {
                return None;
            }
            let size = ((w >> 26) & 7) + 1;
            match funct3 {
                0 => format!("h3.bextm {}, {}, {}, {size}", x(rd), x(rs1), x(rs2)),
                4 if (w >> 25) & 1 == 0 => format!("h3.bextmi {}, {}, {}, {size}", x(rd), x(rs1), rs2),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(Insn { len: 4, text, target: None })
}

/// The opcode that holds an operation on a register and a constant, and the
/// shifts, and most of what the bit manipulation extensions add.
fn op_imm(w: u32, rd: u32, rs1: u32, rs2: u32, funct3: u32, funct7: u32) -> Option<String> {
    let (d, s) = (x(rd), x(rs1));
    let shamt = rs2;
    Some(match funct3 {
        0 => format!("addi {d}, {s}, {}", imm_i(w)),
        2 => format!("slti {d}, {s}, {}", imm_i(w)),
        3 => format!("sltiu {d}, {s}, {}", imm_i(w)),
        4 => format!("xori {d}, {s}, {}", imm_i(w)),
        6 => format!("ori {d}, {s}, {}", imm_i(w)),
        7 => format!("andi {d}, {s}, {}", imm_i(w)),
        // A left shift and the several extensions that share its encoding,
        // told apart by the seven bits above the shift amount.
        1 => match (funct7, shamt) {
            (0b0000000, _) => format!("slli {d}, {s}, 0x{shamt:x}"),
            (0b0010100, _) => format!("bseti {d}, {s}, 0x{shamt:x}"),
            (0b0100100, _) => format!("bclri {d}, {s}, 0x{shamt:x}"),
            (0b0110100, _) => format!("binvi {d}, {s}, 0x{shamt:x}"),
            // These count or extend rather than shift, so the shift amount
            // field is not an amount but the choice of which one.
            (0b0110000, 0) => format!("clz {d}, {s}"),
            (0b0110000, 1) => format!("ctz {d}, {s}"),
            (0b0110000, 2) => format!("cpop {d}, {s}"),
            (0b0110000, 4) => format!("sext.b {d}, {s}"),
            (0b0110000, 5) => format!("sext.h {d}, {s}"),
            // Interleaving a register's two halves, and its inverse, which
            // sit either side of the shift's `funct3`.
            (0b0000100, 15) => format!("zip {d}, {s}"),
            _ => return None,
        },
        5 => match (funct7, shamt) {
            (0b0000000, _) => format!("srli {d}, {s}, 0x{shamt:x}"),
            (0b0100000, _) => format!("srai {d}, {s}, 0x{shamt:x}"),
            (0b0110000, _) => format!("rori {d}, {s}, 0x{shamt:x}"),
            (0b0100100, _) => format!("bexti {d}, {s}, 0x{shamt:x}"),
            (0b0010100, 7) => format!("orc.b {d}, {s}"),
            (0b0110100, 7) => format!("brev8 {d}, {s}"),
            // Byte reverse names the width it reverses, and on a 32-bit
            // machine that is the whole register.
            (0b0110100, 24) => format!("rev8 {d}, {s}"),
            (0b0000100, 15) => format!("unzip {d}, {s}"),
            _ => return None,
        },
        _ => return None,
    })
}

/// The opcode that holds an operation on two registers: the base arithmetic,
/// multiply and divide, and the rest of bit manipulation.
fn op(_w: u32, rd: u32, rs1: u32, rs2: u32, funct3: u32, funct7: u32) -> Option<String> {
    let (d, a, b) = (x(rd), x(rs1), x(rs2));
    let name = match (funct7, funct3) {
        (0b0000000, 0) => "add",
        (0b0000000, 1) => "sll",
        (0b0000000, 2) => "slt",
        (0b0000000, 3) => "sltu",
        (0b0000000, 4) => "xor",
        (0b0000000, 5) => "srl",
        (0b0000000, 6) => "or",
        (0b0000000, 7) => "and",
        (0b0100000, 0) => "sub",
        (0b0100000, 4) => "xnor",
        (0b0100000, 5) => "sra",
        (0b0100000, 6) => "orn",
        (0b0100000, 7) => "andn",
        (0b0000001, 0) => "mul",
        (0b0000001, 1) => "mulh",
        (0b0000001, 2) => "mulhsu",
        (0b0000001, 3) => "mulhu",
        (0b0000001, 4) => "div",
        (0b0000001, 5) => "divu",
        (0b0000001, 6) => "rem",
        (0b0000001, 7) => "remu",
        (0b0010000, 2) => "sh1add",
        (0b0010000, 4) => "sh2add",
        (0b0010000, 6) => "sh3add",
        (0b0000101, 1) => "clmul",
        (0b0000101, 2) => "clmulr",
        (0b0000101, 3) => "clmulh",
        (0b0000101, 4) => "min",
        (0b0000101, 5) => "minu",
        (0b0000101, 6) => "max",
        (0b0000101, 7) => "maxu",
        (0b0110000, 1) => "rol",
        (0b0110000, 5) => "ror",
        (0b0010100, 1) => "bset",
        (0b0100100, 1) => "bclr",
        (0b0100100, 5) => "bext",
        (0b0110100, 1) => "binv",
        // Zero-extending a halfword is packing it with nothing, and the
        // standard gives that spelling its own name.
        (0b0000100, 4) if rs2 == 0 => return Some(format!("zext.h {d}, {a}")),
        (0b0000100, 4) => "pack",
        (0b0000100, 7) => "packh",
        _ => return None,
    };
    Some(format!("{name} {d}, {a}, {b}"))
}

/// A fence, which names the accesses it orders on each side of itself.
fn fence(w: u32) -> String {
    let flags = |bits: u32| {
        let mut s = String::new();
        for (bit, letter) in [(8, 'i'), (4, 'o'), (2, 'r'), (1, 'w')] {
            if bits & bit != 0 {
                s.push(letter);
            }
        }
        if s.is_empty() { "0".to_string() } else { s }
    };
    let (pred, succ) = ((w >> 24) & 15, (w >> 20) & 15);
    if pred == 15 && succ == 15 { "fence".to_string() } else { format!("fence {}, {}", flags(pred), flags(succ)) }
}

/// The atomic memory operations, which is how one core takes a lock another
/// core can see.
fn atomic(w: u32, rd: u32, rs1: u32, rs2: u32) -> Option<String> {
    let name = match w >> 27 {
        0b00010 if rs2 == 0 => "lr.w",
        0b00011 => "sc.w",
        0b00001 => "amoswap.w",
        0b00000 => "amoadd.w",
        0b00100 => "amoxor.w",
        0b01100 => "amoand.w",
        0b01000 => "amoor.w",
        0b10000 => "amomin.w",
        0b10100 => "amomax.w",
        0b11000 => "amominu.w",
        0b11100 => "amomaxu.w",
        _ => return None,
    };
    // Two bits say whether this access may be reordered before or after the
    // ones around it, and they are written as a suffix.
    let order = match (w >> 25) & 3 {
        0b11 => ".aqrl",
        0b10 => ".aq",
        0b01 => ".rl",
        _ => "",
    };
    Some(if name == "lr.w" {
        format!("lr.w{order} {}, ({})", x(rd), x(rs1))
    } else {
        format!("{name}{order} {}, {}, ({})", x(rd), x(rs2), x(rs1))
    })
}

/// The system opcode: the handful of instructions that change privilege or
/// halt, and everything that reads or writes a control register.
fn system(w: u32, rd: u32, rs1: u32, _rs2: u32, funct3: u32) -> Option<String> {
    if funct3 == 0 {
        return Some(
            match w >> 20 {
                0x000 => "ecall",
                0x001 => "ebreak",
                0x002 => "uret",
                0x102 => "sret",
                0x302 => "mret",
                0x7b2 => "dret",
                0x105 => "wfi",
                _ => return None,
            }
            .to_string(),
        );
    }
    let csr = csr_name(w >> 20);
    Some(match funct3 {
        1 => format!("csrrw {}, {csr}, {}", x(rd), x(rs1)),
        2 => format!("csrrs {}, {csr}, {}", x(rd), x(rs1)),
        3 => format!("csrrc {}, {csr}, {}", x(rd), x(rs1)),
        // The immediate forms put a five-bit constant where the source
        // register would be, so the field is a number rather than a name.
        5 => format!("csrrwi {}, {csr}, {rs1}", x(rd)),
        6 => format!("csrrsi {}, {csr}, {rs1}", x(rd)),
        7 => format!("csrrci {}, {csr}, {rs1}", x(rd)),
        _ => return None,
    })
}

// ----------------------------------------------------------------- two bytes

fn compressed(w: u16, hazard3: bool) -> Option<Insn> {
    let w = w as u32;
    let funct3 = w >> 13;
    let text = match w & 3 {
        0 => quadrant0(w, funct3)?,
        1 => return quadrant1(w, funct3, hazard3),
        2 => quadrant2(w, funct3)?,
        _ => return None,
    };
    Some(Insn { len: 2, text, target: None })
}

/// The compressed loads and stores, whose offsets are unsigned and scaled.
fn quadrant0(w: u32, funct3: u32) -> Option<String> {
    let (rd, rs1) = (xc(w >> 2), xc(w >> 7));
    // The word-sized offset, with its bits scattered as the encoding packs them.
    let off_w = ((w >> 4) & 4) | ((w >> 7) & 0x38) | ((w << 1) & 0x40);
    let off_d = ((w >> 7) & 0x38) | ((w << 1) & 0xc0);
    Some(match funct3 {
        0 => {
            // Adding to the stack pointer, which is how a frame is made. An
            // offset of zero is not this instruction but the encoding the
            // standard reserves to mean nothing at all.
            let imm = ((w >> 1) & 0x3c0) | ((w >> 7) & 0x30) | ((w >> 2) & 8) | ((w >> 4) & 4);
            if imm == 0 {
                return None;
            }
            format!("c.addi4spn {rd}, sp, {imm}")
        }
        1 => format!("c.fld {rd}, {off_d}({rs1})"),
        2 => format!("c.lw {rd}, {off_w}({rs1})"),
        3 => format!("c.flw {rd}, {off_w}({rs1})"),
        // The byte and halfword accesses, which the base compressed set left
        // out and Zcb puts back.
        4 => {
            // The two offset bits are stored the other way up from every
            // other compressed offset: the high one is the lower bit of the
            // instruction.
            let byte_off = ((w >> 6) & 1) | ((w >> 4) & 2);
            let half_off = (w >> 4) & 2;
            match (w >> 10) & 7 {
                0 => format!("c.lbu {rd}, {byte_off}({rs1})"),
                1 if w & 0x40 == 0 => format!("c.lhu {rd}, {half_off}({rs1})"),
                1 => format!("c.lh {rd}, {half_off}({rs1})"),
                2 => format!("c.sb {rd}, {byte_off}({rs1})"),
                3 if w & 0x40 == 0 => format!("c.sh {rd}, {half_off}({rs1})"),
                _ => return None,
            }
        }
        5 => format!("c.fsd {rd}, {off_d}({rs1})"),
        6 => format!("c.sw {rd}, {off_w}({rs1})"),
        _ => format!("c.fsw {rd}, {off_w}({rs1})"),
    })
}

/// The compressed constants, jumps and branches, and the small arithmetic.
fn quadrant1(w: u32, funct3: u32, hazard3: bool) -> Option<Insn> {
    let rd = (w >> 7) & 31;
    let imm = sign_extend(((w >> 2) & 31) | ((w >> 7) & 0x20), 6);
    let text = match funct3 {
        0 if rd == 0 => "c.nop".to_string(),
        0 => format!("c.addi {}, {imm}", x(rd)),
        1 => {
            let off = cj_offset(w);
            return Some(Insn { len: 2, text: format!("c.jal {}", rel(off)), target: Some(off as i64) });
        }
        2 if rd != 0 => format!("c.li {}, {imm}", x(rd)),
        // One encoding for two instructions, told apart by which register the
        // constant lands in: the stack pointer gets the scaled form a function
        // opens and closes its frame with.
        3 if rd == 2 => {
            let v = sign_extend(
                ((w >> 3) & 0x200) | ((w >> 2) & 0x10) | ((w << 1) & 0x40) | ((w << 4) & 0x180) | ((w << 3) & 0x20),
                10,
            );
            if v == 0 {
                return None;
            }
            format!("c.addi16sp sp, {v}")
        }
        3 if rd != 0 => {
            let v = sign_extend(((w >> 2) & 31) | ((w >> 7) & 0x20), 6);
            if v == 0 {
                return None;
            }
            format!("c.lui {}, 0x{:x}", x(rd), (v as u32) & 0xfffff)
        }
        4 => quadrant1_arith(w)?,
        5 => {
            let off = cj_offset(w);
            return Some(Insn { len: 2, text: format!("c.j {}", rel(off)), target: Some(off as i64) });
        }
        6 | 7 => {
            let off = cb_offset(w);
            let name = if funct3 == 6 { "c.beqz" } else { "c.bnez" };
            let text = format!("{name} {}, {}", xc(w >> 7), rel(off));
            return Some(Insn { len: 2, text, target: Some(off as i64) });
        }
        _ => return None,
    };
    let _ = hazard3;
    Some(Insn { len: 2, text, target: None })
}

/// The compressed shifts, mask and register-to-register arithmetic, all
/// packed into one `funct3` and told apart by the bits below it.
fn quadrant1_arith(w: u32) -> Option<String> {
    let d = xc(w >> 7);
    let shamt = ((w >> 2) & 31) | ((w >> 7) & 0x20);
    Some(match (w >> 10) & 3 {
        0 => format!("c.srli {d}, {shamt}"),
        1 => format!("c.srai {d}, {shamt}"),
        2 => format!("c.andi {d}, {}", sign_extend(((w >> 2) & 31) | ((w >> 7) & 0x20), 6)),
        _ => {
            let s = xc(w >> 2);
            match (w >> 12) & 1 {
                0 => match (w >> 5) & 3 {
                    0 => format!("c.sub {d}, {s}"),
                    1 => format!("c.xor {d}, {s}"),
                    2 => format!("c.or {d}, {s}"),
                    _ => format!("c.and {d}, {s}"),
                },
                // With the top bit set these would be the 64-bit word
                // operations, which a 32-bit machine does not have; Zcb uses
                // the space for a multiply and for widening a narrow value.
                _ => match (w >> 5) & 3 {
                    2 => format!("c.mul {d}, {s}"),
                    3 => match (w >> 2) & 7 {
                        0 => format!("c.zext.b {d}"),
                        1 => format!("c.sext.b {d}"),
                        2 => format!("c.zext.h {d}"),
                        3 => format!("c.sext.h {d}"),
                        5 => format!("c.not {d}"),
                        _ => return None,
                    },
                    _ => return None,
                },
            }
        }
    })
}

/// The compressed stack accesses, the register moves and jumps through a
/// register, and the extension that saves and restores a whole frame at once.
fn quadrant2(w: u32, funct3: u32) -> Option<String> {
    let rd = (w >> 7) & 31;
    let rs2 = (w >> 2) & 31;
    Some(match funct3 {
        0 => format!("c.slli {}, {}", x(rd), ((w >> 2) & 31) | ((w >> 7) & 0x20)),
        1 => format!("c.fldsp {}, {}(sp)", x(rd), ((w >> 7) & 0x20) | ((w >> 2) & 0x18) | ((w << 4) & 0x1c0)),
        2 if rd != 0 => format!("c.lwsp {}, {}(sp)", x(rd), ((w >> 7) & 0x20) | ((w >> 2) & 0x1c) | ((w << 4) & 0xc0)),
        3 => format!("c.flwsp {}, {}(sp)", x(rd), ((w >> 7) & 0x20) | ((w >> 2) & 0x1c) | ((w << 4) & 0xc0)),
        4 => match ((w >> 12) & 1, rd, rs2) {
            (0, d, 0) if d != 0 => format!("c.jr {}", x(d)),
            (0, d, s) if d != 0 => format!("c.mv {}, {}", x(d), x(s)),
            (1, 0, 0) => "c.ebreak".to_string(),
            (1, d, 0) if d != 0 => format!("c.jalr {}", x(d)),
            (1, d, s) if d != 0 => format!("c.add {}, {}", x(d), x(s)),
            _ => return None,
        },
        // A 64-bit float store on a machine that has one; on a machine that
        // does not, the encoding a compiler uses to open and close a frame in
        // a single instruction.
        5 => return zcmp(w),
        6 => format!("c.swsp {}, {}(sp)", x(rs2), ((w >> 7) & 0x3c) | ((w >> 1) & 0xc0)),
        _ => format!("c.fswsp {}, {}(sp)", x(rs2), ((w >> 7) & 0x3c) | ((w >> 1) & 0xc0)),
    })
}

/// The compressed push and pop, which stand for the whole prologue or epilogue
/// of a function: adjust the stack, and save or restore a run of registers.
fn zcmp(w: u32) -> Option<String> {
    // Moves between the argument registers and the saved ones, which a
    // compiler emits around a call rather than at a function's edges.
    if (w >> 10) & 7 == 3 {
        let (a, b) = (sreg(w >> 7), sreg(w >> 2));
        return match (w >> 5) & 3 {
            1 => Some(format!("cm.mvsa01 {a}, {b}")),
            3 => Some(format!("cm.mva01s {a}, {b}")),
            _ => None,
        };
    }
    let name = match (w >> 8) & 0x1f {
        0b11000 => "cm.push",
        0b11010 => "cm.pop",
        0b11100 => "cm.popretz",
        0b11110 => "cm.popret",
        _ => return None,
    };
    let rlist = (w >> 4) & 15;
    let list = register_list(rlist)?;
    // How far the stack moves: enough for the registers being saved, rounded
    // to sixteen bytes, plus whatever more the instruction asks for.
    let base = match rlist {
        4..=7 => 16,
        8..=11 => 32,
        12..=14 => 48,
        _ => 64,
    };
    let bytes = base + ((w >> 2) & 3) * 16;
    // Push writes the adjustment as the negative number it is: the stack
    // grows down.
    Some(if name == "cm.push" {
        format!("cm.push {{{list}}}, -{bytes}")
    } else {
        format!("{name} {{{list}}}, {bytes}")
    })
}

/// Which registers a push or pop covers. The list always starts at the return
/// address and runs through the saved registers in order, so one number says
/// how far along it stops.
fn register_list(rlist: u32) -> Option<String> {
    Some(match rlist {
        4 => "ra".to_string(),
        5 => "ra, s0".to_string(),
        // The last value covers every saved register; the ones before it stop
        // one short each time.
        15 => "ra, s0-s11".to_string(),
        6..=14 => format!("ra, s0-s{}", rlist - 5),
        _ => return None,
    })
}

// -------------------------------------------------------------------- pieces

fn imm_i(w: u32) -> i32 {
    sign_extend(w >> 20, 12)
}

fn imm_s(w: u32) -> i32 {
    sign_extend(((w >> 7) & 31) | ((w >> 20) & 0xfe0), 12)
}

fn branch_offset(w: u32) -> i32 {
    sign_extend(((w >> 7) & 0x1e) | ((w >> 20) & 0x7e0) | ((w << 4) & 0x800) | ((w >> 19) & 0x1000), 13)
}

fn jal_offset(w: u32) -> i32 {
    sign_extend(((w >> 20) & 0x7fe) | ((w >> 9) & 0x800) | (w & 0xff000) | ((w >> 11) & 0x100000), 21)
}

fn cj_offset(w: u32) -> i32 {
    sign_extend(
        ((w >> 2) & 0xe) | ((w >> 7) & 0x10) | ((w << 3) & 0x20) | ((w >> 1) & 0x40) | ((w << 1) & 0x80)
            | ((w >> 1) & 0x300)
            | ((w << 2) & 0x400)
            | ((w >> 1) & 0x800),
        12,
    )
}

fn cb_offset(w: u32) -> i32 {
    sign_extend(
        ((w >> 2) & 6) | ((w >> 7) & 0x18) | ((w << 3) & 0x20) | ((w << 1) & 0xc0) | ((w >> 4) & 0x100),
        9,
    )
}

fn sign_extend(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

/// A branch target, written the way the other decoders here write one: a
/// distance from where the instruction is, because that is what the
/// instruction holds and a file has no addresses of its own.
fn rel(offset: i32) -> String {
    let mut s = String::new();
    let _ = write!(s, "${}0x{:x}", if offset < 0 { "-" } else { "+" }, offset.unsigned_abs());
    s
}

/// What a control register is called. There are several thousand numbers and a
/// name for perhaps a hundred; the rest are written as the number, which is
/// what a toolchain does and is honest about what is known.
fn csr_name(number: u32) -> String {
    let name = match number {
        0x001 => "fflags",
        0x002 => "frm",
        0x003 => "fcsr",
        0x300 => "mstatus",
        0x301 => "misa",
        0x302 => "medeleg",
        0x303 => "mideleg",
        0x304 => "mie",
        0x305 => "mtvec",
        0x306 => "mcounteren",
        0x310 => "mstatush",
        0x320 => "mcountinhibit",
        0x340 => "mscratch",
        0x341 => "mepc",
        0x342 => "mcause",
        0x343 => "mtval",
        0x344 => "mip",
        0x3a0 => "pmpcfg0",
        0x3a1 => "pmpcfg1",
        0x3a2 => "pmpcfg2",
        0x3a3 => "pmpcfg3",
        0x7a0 => "tselect",
        0x7a1 => "tdata1",
        0x7a2 => "tdata2",
        0x7a3 => "tdata3",
        0x7b0 => "dcsr",
        0x7b1 => "dpc",
        0x7b2 => "dscratch0",
        0x7b3 => "dscratch1",
        0xb00 => "mcycle",
        0xb02 => "minstret",
        0xb80 => "mcycleh",
        0xb82 => "minstreth",
        0xc00 => "cycle",
        0xc01 => "time",
        0xc02 => "instret",
        0xc80 => "cycleh",
        0xc81 => "timeh",
        0xc82 => "instreth",
        0xf11 => "mvendorid",
        0xf12 => "marchid",
        0xf13 => "mimpid",
        0xf14 => "mhartid",
        0xf15 => "mconfigptr",
        // Hazard3's own registers. Unlike its instructions these need no
        // permission: the numbers are in the range the standard hands to an
        // implementation, so nothing else can mean anything by them.
        0xbd0 => "h3.pmpcfgm0",
        0xbe0 => "h3.meiea",
        0xbe1 => "h3.meipa",
        0xbe2 => "h3.meifa",
        0xbe3 => "h3.meipra",
        0xbe4 => "h3.meinext",
        0xbe5 => "h3.meicontext",
        0xbf0 => "h3.msleep",
        0xbf1 => "h3.misa",
        0xbff => "h3.dmdata0",
        n if (0x3b0..=0x3ef).contains(&n) => return format!("pmpaddr{}", n - 0x3b0),
        n => return format!("0x{n:x}"),
    };
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(bytes: &[u8]) -> String {
        decode(bytes, true).map(|i| i.text).unwrap_or_else(|| "(bad)".into())
    }

    /// The base instructions, which any RISC-V decoder gets right and which
    /// are here so a change to the extensions cannot quietly break them.
    #[test]
    fn the_base_instructions_read() {
        assert_eq!(t(&[0x13, 0x05, 0x10, 0x00]), "addi a0, zero, 1");
        assert_eq!(t(&[0x33, 0x85, 0xc5, 0x02]), "mul a0, a1, a2");
        assert_eq!(t(&[0x73, 0x00, 0x50, 0x10]), "wfi");
        assert_eq!(t(&[0x0f, 0x10, 0x00, 0x00]), "fence.i");
        assert_eq!(t(&[0x01, 0x00]), "c.nop");
        assert_eq!(t(&[0x82, 0x80]), "c.jr ra");
    }

    /// The extensions whose encodings sit in the spare corners of the base
    /// opcodes. Every one of these was read as a base instruction before, so
    /// each line here is one place the file used to lie. The bytes and the
    /// text are the RP2350 boot ROM's and its toolchain's, not made up here.
    #[test]
    fn an_extension_is_not_the_base_instruction_it_sits_beside() {
        assert_eq!(t(&[0xb3, 0xc6, 0x76, 0x20]), "sh2add a3, a3, t2");
        assert_eq!(t(&[0x33, 0x24, 0xe4, 0x21]), "sh1add s0, s0, t5");
        assert_eq!(t(&[0x33, 0x74, 0x24, 0x41]), "andn s0, s0, s2");
        assert_eq!(t(&[0xb3, 0x48, 0x8e, 0x40]), "xnor a7, t3, s0");
        assert_eq!(t(&[0xb3, 0x45, 0xb4, 0x08]), "pack a1, s0, a1");
        assert_eq!(t(&[0x33, 0x17, 0xd0, 0x28]), "bset a4, zero, a3");
        assert_eq!(t(&[0x33, 0x57, 0x87, 0x60]), "ror a4, a4, s0");
        assert_eq!(t(&[0xb3, 0xd8, 0x96, 0x48]), "bext a7, a3, s1");
        assert_eq!(t(&[0x93, 0x94, 0x15, 0x60]), "ctz s1, a1");
        assert_eq!(t(&[0x13, 0x94, 0x24, 0x60]), "cpop s0, s1");
        assert_eq!(t(&[0x93, 0xd4, 0x84, 0x69]), "rev8 s1, s1");
        assert_eq!(t(&[0x93, 0xd4, 0x04, 0x61]), "rori s1, s1, 0x10");
        assert_eq!(t(&[0x13, 0xde, 0x87, 0x48]), "bexti t3, a5, 0x8");
        assert_eq!(t(&[0x93, 0x97, 0xb7, 0x28]), "bseti a5, a5, 0xb");
        // Both spellings of zero-extending a halfword, four bytes and two.
        assert_eq!(t(&[0x33, 0xc5, 0x05, 0x08]), "zext.h a0, a1");
        assert_eq!(t(&[0xe9, 0x9e]), "c.zext.h a3");
    }

    /// The compressed encodings the base set left out, and the push and pop
    /// that stand for a whole prologue. The expected text is what the RP2350
    /// boot ROM's own listing writes for these bytes.
    #[test]
    fn the_extra_compressed_encodings_read() {
        assert_eq!(t(&[0x4d, 0x9d]), "c.mul a0, a1");
        assert_eq!(t(&[0x61, 0x9d]), "c.zext.b a0");
        assert_eq!(t(&[0x72, 0xb8]), "cm.push {ra, s0-s2}, -16");
        assert_eq!(t(&[0x7e, 0xbe]), "cm.popret {ra, s0-s2}, 64");
        assert_eq!(t(&[0x72, 0xba]), "cm.pop {ra, s0-s2}, 16");
        assert_eq!(t(&[0x26, 0xac]), "cm.mvsa01 s0, s1");
    }

    /// Hazard3's own instructions, and the fact that they are only Hazard3's
    /// when the caller says the machine is one.
    #[test]
    fn a_vendor_instruction_needs_the_vendor_named() {
        let bytes = [0x8b, 0xc6, 0x37, 0x08];
        assert_eq!(t(&bytes), "h3.bextmi a3, a5, 3, 3");
        // The same bytes on a machine that was never said to be a Hazard3 are
        // an opcode nobody has defined, and come back as nothing.
        assert_eq!(decode(&bytes, false), None);
        assert_eq!(t(&[0x0b, 0x85, 0xc5, 0x00]), "h3.bextm a0, a1, a2, 1");
    }

    /// A branch says how far it goes, in the one unit a reader of a file can
    /// use: a distance from the first byte of the instruction.
    #[test]
    fn a_branch_says_how_far_it_goes() {
        // A jump to itself, which is how a program hangs.
        assert_eq!(decode(&[0x6f, 0x00, 0x00, 0x00], false).unwrap().target, Some(0));
        assert_eq!(decode(&[0xef, 0x00, 0x80, 0x00], false).unwrap().target, Some(8));
        assert_eq!(decode(&[0x13, 0x05, 0x10, 0x00], false).unwrap().target, None);
    }
}
