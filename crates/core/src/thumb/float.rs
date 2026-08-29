//! The floating point unit, which the general decoder does not read at all.
//!
//! Every one of these instructions came back as `(bad)`, which on a machine
//! whose instructions vary in length is worse than it sounds: the reader steps
//! two bytes into a four-byte instruction and everything after it is read from
//! the wrong place. A Pico 2 has a single-precision unit and the SDK's own
//! maths uses it, so a program that does any arithmetic on fractions was
//! largely unreadable.
//!
//! One thing to know about how the registers are written down. A number that
//! picks one of thirty-two registers needs five bits, and these encodings were
//! designed when there were sixteen, so the fifth bit lives on its own
//! somewhere else in the word. For the single-precision registers it is the
//! low bit and the four-bit field is the high part; for the double-precision
//! ones it is the other way round. Getting that backwards names a different
//! register, so [`single`] and [`double`] are the only places it is done.

use super::r;

/// Which coprocessor field marks an instruction as this unit's, and whether
/// the operands are single or double width.
fn width(hw2: u32) -> Option<bool> {
    match (hw2 >> 8) & 15 {
        0xa => Some(false),
        0xb => Some(true),
        _ => None,
    }
}

/// A single-precision register: four bits of it in one place, and the lowest
/// bit on its own.
fn single(four: u32, low: u32) -> String {
    format!("s{}", ((four & 15) << 1) | (low & 1))
}

/// A double-precision register, where the odd bit is the highest rather than
/// the lowest.
fn double(four: u32, high: u32) -> String {
    format!("d{}", ((high & 1) << 4) | (four & 15))
}

fn reg(wide: bool, four: u32, extra: u32) -> String {
    if wide { double(four, extra) } else { single(four, extra) }
}

fn suffix(wide: bool) -> &'static str {
    if wide { "f64" } else { "f32" }
}

pub fn decode(hw1: u32, hw2: u32) -> Option<String> {
    // Everything here is one of the two coprocessors this unit answers on, and
    // sits in the instruction space the standard sets aside for one.
    if hw1 & 0xec00 != 0xec00 {
        return None;
    }
    width(hw2)?;
    transfer(hw1, hw2)
        .or_else(|| three_registers(hw1, hw2))
        .or_else(|| one_register(hw1, hw2))
        .or_else(|| memory(hw1, hw2))
}

/// Arithmetic on two registers into a third.
fn three_registers(hw1: u32, hw2: u32) -> Option<String> {
    if hw1 & 0xef00 != 0xee00 || hw2 & 0x0010 != 0 {
        return None;
    }
    let wide = width(hw2)?;
    let extended = hw1 & 0x1000 != 0;
    let (o1, o2, o3) = ((hw1 >> 7) & 1, (hw1 >> 4) & 3, (hw2 >> 6) & 1);
    let d = reg(wide, (hw2 >> 12) & 15, (hw1 >> 6) & 1);
    let n = reg(wide, hw1 & 15, (hw2 >> 7) & 1);
    let m = reg(wide, hw2 & 15, (hw2 >> 5) & 1);
    let t = suffix(wide);
    // The instructions above the ordinary arithmetic pick the larger or
    // smaller of two numbers the way the floating point standard says to, and
    // choose between two numbers on a condition without branching.
    if extended {
        return Some(match (o1, o2, o3) {
            (1, 0, 0) => format!("vmaxnm.{t} {d}, {n}, {m}"),
            (1, 0, 1) => format!("vminnm.{t} {d}, {n}, {m}"),
            (0, condition, 0) => {
                format!("vsel{}.{t} {d}, {n}, {m}", ["eq", "vs", "ge", "gt"][condition as usize])
            }
            _ => return None,
        });
    }
    let name = match (o1, o2, o3) {
        (0, 0, 0) => "vmla",
        (0, 0, 1) => "vmls",
        (0, 1, 0) => "vnmls",
        (0, 1, 1) => "vnmla",
        (0, 2, 0) => "vmul",
        (0, 2, 1) => "vnmul",
        (0, 3, 0) => "vadd",
        (0, 3, 1) => "vsub",
        (1, 0, 0) => "vdiv",
        (1, 1, 0) => "vfnms",
        (1, 1, 1) => "vfnma",
        (1, 2, 0) => "vfma",
        (1, 2, 1) => "vfms",
        _ => return None,
    };
    Some(format!("{name}.{t} {d}, {n}, {m}"))
}

/// The operations on one register: sign changes, roots, comparisons,
/// conversions between the fractional and whole representations, and loading a
/// constant.
fn one_register(hw1: u32, hw2: u32) -> Option<String> {
    if hw1 & 0xefb0 != 0xeeb0 || hw2 & 0x0010 != 0 {
        return None;
    }
    let wide = width(hw2)?;
    let extended = hw1 & 0x1000 != 0;
    let (opc2, opc3) = (hw1 & 15, (hw2 >> 6) & 3);
    let d = reg(wide, (hw2 >> 12) & 15, (hw1 >> 6) & 1);
    let m = reg(wide, hw2 & 15, (hw2 >> 5) & 1);
    let t = suffix(wide);

    // Rounding to a whole number, and converting to one, each in the four
    // directions the floating point standard names.
    if extended {
        let way = ["a", "n", "p", "m"][(opc2 & 3) as usize];
        return Some(match (opc2 >> 2, opc3) {
            (2, 1) => format!("vrint{way}.{t} {d}, {m}"),
            (3, _) => {
                let signed = if opc3 & 2 == 0 { "u32" } else { "s32" };
                format!("vcvt{way}.{signed}.{t} {d}, {}", reg(false, hw2 & 15, (hw2 >> 5) & 1))
            }
            _ => return None,
        });
    }

    // With the low two bits of the second halfword clear this is not an
    // operation at all but a constant, spelled across the two fields that
    // would otherwise say which one.
    if opc3 == 0 {
        let imm8 = (opc2 << 4) | (hw2 & 15);
        return Some(format!("vmov.{t} {d}, #{}", constant(imm8)));
    }

    Some(match (opc2, opc3) {
        (0, 1) => format!("vmov.{t} {d}, {m}"),
        (0, 3) => format!("vabs.{t} {d}, {m}"),
        (1, 1) => format!("vneg.{t} {d}, {m}"),
        (1, 3) => format!("vsqrt.{t} {d}, {m}"),
        // Converting between this width and the half-width format, taking or
        // placing the value in the top or bottom of the register.
        (2 | 3, _) => {
            let half = if opc2 == 2 { "b" } else { "t" };
            let up = opc3 & 2 == 0;
            match up {
                true => format!("vcvt{half}.{t}.f16 {d}, {m}"),
                false => format!("vcvt{half}.f16.{t} {d}, {m}"),
            }
        }
        (4, 1) => format!("vcmp.{t} {d}, {m}"),
        (4, 3) => format!("vcmpe.{t} {d}, {m}"),
        (5, 1) => format!("vcmp.{t} {d}, #0"),
        (5, 3) => format!("vcmpe.{t} {d}, #0"),
        (6, 3) => format!("vrintz.{t} {d}, {m}"),
        (7, 1) => format!("vrintx.{t} {d}, {m}"),
        // Between the two widths this unit works in.
        (7, 3) => match wide {
            true => format!("vcvt.f32.f64 {}, {m}", reg(false, (hw2 >> 12) & 15, (hw1 >> 6) & 1)),
            false => format!("vcvt.f64.f32 {}, {m}", reg(true, (hw2 >> 12) & 15, (hw1 >> 6) & 1)),
        },
        // From a whole number to a fraction. Here the source is always a
        // single-width register, whatever the destination is.
        (8, _) => {
            let from = if opc3 & 2 == 0 { "u32" } else { "s32" };
            format!("vcvt.{t}.{from} {d}, {}", reg(false, hw2 & 15, (hw2 >> 5) & 1))
        }
        // And back, either rounding the way the mode says or towards zero.
        (12 | 13, _) => {
            let to = if opc2 == 12 { "u32" } else { "s32" };
            let exact = if opc3 & 2 == 0 { "r" } else { "" };
            format!("vcvt{exact}.{to}.{t} {}, {m}", reg(false, (hw2 >> 12) & 15, (hw1 >> 6) & 1))
        }
        _ => return None,
    })
}

/// The constant a `vmov` holds: a sign, a short exponent and four bits of
/// significand, which between them reach the small round numbers a program
/// writes literally.
fn constant(imm8: u32) -> String {
    // The exponent is built by repeating one bit: set, it puts the value near
    // one; clear, it moves to the other end of the small range these reach.
    // Two more bits then step either side.
    let near_one = (imm8 >> 6) & 1;
    let exponent = match near_one {
        1 => 0b0111_1100 | ((imm8 >> 4) & 3),
        _ => 0b1000_0000 | ((imm8 >> 4) & 3),
    } as i32;
    let significand = 1.0 + ((imm8 & 15) as f64) / 16.0;
    let value = significand * 2f64.powi(exponent - 127);
    let value = if imm8 & 0x80 != 0 { -value } else { value };
    // Written the way an assembler writes it, so a round number looks round
    // rather than like a measurement.
    if value == value.trunc() && value.abs() < 1e9 {
        return format!("{value:.1}");
    }
    format!("{value}")
}

/// Moving a value between the two halves of the processor: one register each
/// way, a pair of them, or the unit's own status register.
fn transfer(hw1: u32, hw2: u32) -> Option<String> {
    // The status and control register, which is where a comparison leaves its
    // answer for a branch to read.
    if hw1 & 0xeff0 == 0xeef0 && hw2 & 0x0f7f == 0x0a10 {
        let name = special(hw1 & 15)?;
        return Some(match (hw2 >> 12) & 15 {
            // The code for "the flags themselves", which is what a comparison
            // is moved into so that an ordinary conditional branch can use it.
            15 => format!("vmrs APSR_nzcv, {name}"),
            rt => format!("vmrs {}, {name}", r(rt)),
        });
    }
    if hw1 & 0xeff0 == 0xeee0 && hw2 & 0x0f7f == 0x0a10 {
        return Some(format!("vmsr {}, {}", special(hw1 & 15)?, r((hw2 >> 12) & 15)));
    }
    // One register each way.
    if hw1 & 0xefe0 == 0xee00 && hw2 & 0x0f7f == 0x0a10 {
        let (rt, sn) = (r((hw2 >> 12) & 15), single(hw1 & 15, (hw2 >> 7) & 1));
        return Some(match hw1 & 0x10 {
            0 => format!("vmov {sn}, {rt}"),
            _ => format!("vmov {rt}, {sn}"),
        });
    }
    // Two at a time, which is how a pair of arguments crosses over.
    if hw1 & 0xefe0 == 0xec40 && hw2 & 0x0fd0 == 0x0a10 {
        let (rt, rt2) = (r((hw2 >> 12) & 15), r(hw1 & 15));
        let first = ((hw2 & 15) << 1) | ((hw2 >> 5) & 1);
        return Some(match hw1 & 0x10 {
            0 => format!("vmov s{first}, s{}, {rt}, {rt2}", first + 1),
            _ => format!("vmov {rt}, {rt2}, s{first}, s{}", first + 1),
        });
    }
    None
}

/// The unit's own registers, by the numbers that name them.
fn special(number: u32) -> Option<&'static str> {
    Some(match number {
        0 => "fpsid",
        1 => "fpscr",
        6 => "mvfr1",
        7 => "mvfr0",
        8 => "fpexc",
        _ => return None,
    })
}

/// Loading and storing: one register at a fixed distance, or a run of them
/// through a register that may step along as it goes.
fn memory(hw1: u32, hw2: u32) -> Option<String> {
    let wide = width(hw2)?;
    let rn = hw1 & 15;
    let d = reg(wide, (hw2 >> 12) & 15, (hw1 >> 6) & 1);
    // Three bits say how the address moves: whether the offset applies before
    // the access, which way it goes, and whether the register keeps the
    // result. A fourth says which direction the data goes.
    let (before, up, writes_back, load) =
        ((hw1 >> 8) & 1 == 1, hw1 & 0x0080 != 0, hw1 & 0x0020 != 0, hw1 & 0x0010 != 0);

    // The pair that saves and restores the whole unit around a call into code
    // that is not secure. These sit in the same space with a count of zero,
    // which no real run of registers has.
    if hw1 & 0xeff0 == 0xec30 && hw2 == 0x0a00 {
        return Some(format!("vlldm {}", r(rn)));
    }
    if hw1 & 0xeff0 == 0xec20 && hw2 == 0x0a00 {
        return Some(format!("vlstm {}", r(rn)));
    }

    // One register, at a fixed distance counted in words: these registers are
    // words wide and an address between them would mean nothing.
    if before && !writes_back {
        let name = if load { "vldr" } else { "vstr" };
        let offset = (hw2 & 0xff) * 4;
        let sign = if up { "" } else { "-" };
        return Some(match offset {
            0 => format!("{name} {d}, [{}]", r(rn)),
            _ => format!("{name} {d}, [{}, #{sign}{offset}]", r(rn)),
        });
    }
    // The remaining shape is a run of registers. Not stepping the register and
    // not applying the offset first is the pair transfer, which is not memory.
    if !before && !writes_back {
        return None;
    }
    let count = if wide { (hw2 & 0xff) / 2 } else { hw2 & 0xff };
    if count == 0 {
        return None;
    }
    let list = run(&d, count);
    // Growing down from the stack pointer and shrinking back up are what a
    // function does at its edges, and each of those has its own name.
    if rn == 13 && writes_back {
        if !up && !load {
            return Some(format!("vpush {{{list}}}"));
        }
        if up && load {
            return Some(format!("vpop {{{list}}}"));
        }
    }
    let name = match (load, up) {
        (true, true) => "vldmia",
        (true, false) => "vldmdb",
        (false, true) => "vstmia",
        (false, false) => "vstmdb",
    };
    let bang = if writes_back { "!" } else { "" };
    Some(format!("{name} {}{bang}, {{{list}}}", r(rn)))
}

/// A run of consecutive registers, written the way an assembler writes one:
/// each of them when there are few, and the ends joined by a dash when many.
fn run(first: &str, count: u32) -> String {
    let (letter, number) = first.split_at(1);
    let start: u32 = number.parse().unwrap_or(0);
    if count == 1 {
        return first.to_string();
    }
    if count <= 3 {
        return (0..count).map(|i| format!("{letter}{}", start + i)).collect::<Vec<_>>().join(", ");
    }
    format!("{letter}{start} - {letter}{}", start + count - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(bytes: [u8; 4]) -> String {
        let hw1 = u16::from_le_bytes([bytes[0], bytes[1]]) as u32;
        let hw2 = u16::from_le_bytes([bytes[2], bytes[3]]) as u32;
        decode(hw1, hw2).unwrap_or_else(|| "(declined)".into())
    }

    /// The arithmetic, none of which read at all before. The bytes and the
    /// words are clang's, from the list `tools/isa-sweep.py` assembles.
    #[test]
    fn the_arithmetic_reads() {
        assert_eq!(t([0x30, 0xee, 0x81, 0x0a]), "vadd.f32 s0, s1, s2");
        assert_eq!(t([0x72, 0xee, 0x62, 0x1a]), "vsub.f32 s3, s4, s5");
        assert_eq!(t([0x20, 0xee, 0x81, 0x0a]), "vmul.f32 s0, s1, s2");
        assert_eq!(t([0x80, 0xee, 0x81, 0x0a]), "vdiv.f32 s0, s1, s2");
        assert_eq!(t([0xa0, 0xee, 0x81, 0x0a]), "vfma.f32 s0, s1, s2");
        assert_eq!(t([0xb1, 0xee, 0xe0, 0x0a]), "vsqrt.f32 s0, s1");
        assert_eq!(t([0xb4, 0xee, 0x60, 0x0a]), "vcmp.f32 s0, s1");
        assert_eq!(t([0x80, 0xfe, 0x81, 0x0a]), "vmaxnm.f32 s0, s1, s2");
    }

    /// The register a field names, which takes its lowest bit from elsewhere
    /// in the word: reading the four-bit field alone would name `s1` as `s0`.
    #[test]
    fn a_register_takes_its_last_bit_from_elsewhere() {
        assert_eq!(t([0xb0, 0xee, 0x60, 0x0a]), "vmov.f32 s0, s1");
        assert_eq!(t([0x72, 0xee, 0x62, 0x1a]), "vsub.f32 s3, s4, s5");
    }

    /// Converting between fractions and whole numbers, where which way round
    /// it goes is spread over three fields.
    #[test]
    fn the_conversions_say_which_way_they_go() {
        assert_eq!(t([0xbd, 0xee, 0xe0, 0x0a]), "vcvt.s32.f32 s0, s1");
        assert_eq!(t([0xbc, 0xee, 0xe0, 0x0a]), "vcvt.u32.f32 s0, s1");
        assert_eq!(t([0xb8, 0xee, 0xe0, 0x0a]), "vcvt.f32.s32 s0, s1");
        assert_eq!(t([0xb8, 0xee, 0x60, 0x0a]), "vcvt.f32.u32 s0, s1");
        assert_eq!(t([0xbd, 0xee, 0x60, 0x0a]), "vcvtr.s32.f32 s0, s1");
        assert_eq!(t([0xb2, 0xee, 0x60, 0x0a]), "vcvtb.f32.f16 s0, s1");
    }

    /// Moving between the two halves of the processor, and the constant an
    /// instruction can carry without loading it from anywhere.
    #[test]
    fn the_moves_and_the_constants_read() {
        assert_eq!(t([0x10, 0xee, 0x90, 0x0a]), "vmov r0, s1");
        assert_eq!(t([0x00, 0xee, 0x90, 0x0a]), "vmov s1, r0");
        assert_eq!(t([0x51, 0xec, 0x11, 0x0a]), "vmov r0, r1, s2, s3");
        assert_eq!(t([0xf1, 0xee, 0x10, 0xfa]), "vmrs APSR_nzcv, fpscr");
        assert_eq!(t([0xe1, 0xee, 0x10, 0x0a]), "vmsr fpscr, r0");
        assert_eq!(t([0xb7, 0xee, 0x00, 0x0a]), "vmov.f32 s0, #1.0");
    }

    /// Loading and storing, where three bits say how the address moves and
    /// two of the combinations have names of their own.
    #[test]
    fn the_loads_and_stores_read() {
        assert_eq!(t([0x90, 0xed, 0x00, 0x0a]), "vldr s0, [r0]");
        assert_eq!(t([0x90, 0xed, 0x01, 0x0a]), "vldr s0, [r0, #4]");
        assert_eq!(t([0x00, 0xed, 0x01, 0x0a]), "vstr s0, [r0, #-4]");
        assert_eq!(t([0xb0, 0xec, 0x03, 0x0a]), "vldmia r0!, {s0, s1, s2}");
        assert_eq!(t([0x2d, 0xed, 0x02, 0x0a]), "vpush {s0, s1}");
        assert_eq!(t([0xbd, 0xec, 0x02, 0x0a]), "vpop {s0, s1}");
        assert_eq!(t([0x30, 0xec, 0x00, 0x0a]), "vlldm r0");
    }
}
