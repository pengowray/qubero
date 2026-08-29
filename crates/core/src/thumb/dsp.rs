//! The signal-processing instructions, the ones that take a lock, and a
//! handful of others the general decoder names as their neighbours.
//!
//! A Cortex-M33 does arithmetic on several small numbers packed into one
//! register: four bytes at a time or two halfwords, added with the overflow
//! either wrapping, saturating, or halved. There are six ways to handle the
//! overflow and six operations, and the encoding is a grid of the two. The
//! general decoder reads one row of that grid and names everything in the
//! others after its nearest neighbour, so `qsub8` came back as `qsub16`:
//! right operation, wrong element size, and a program that appears to be doing
//! half as much work as it is.
//!
//! Every instruction here was checked against the same list assembled by
//! clang, which `tools/isa-sweep.py` writes out.

use super::r;

/// The overflow handling, which is the top half of the grid: plain, saturating
/// at the type's limits, halved to make room, and each of those again for
/// unsigned numbers.
const HOW: [&str; 8] = ["s", "q", "sh", "", "u", "uq", "uh", ""];

/// The operation, which is the other half: add or subtract, on bytes or on
/// halfwords, and the two that add one halfword while subtracting the other.
const WHAT: [&str; 8] = ["add8", "add16", "asx", "", "sub8", "sub16", "sax", ""];

pub fn decode(hw1: u32, hw2: u32) -> Option<String> {
    if let Some(text) = parallel(hw1, hw2) {
        return Some(text);
    }
    if let Some(text) = ordered(hw1, hw2) {
        return Some(text);
    }
    if let Some(text) = misnamed(hw1, hw2) {
        return Some(text);
    }
    counted(hw1, hw2)
}

/// Arithmetic on several numbers packed into one register.
fn parallel(hw1: u32, hw2: u32) -> Option<String> {
    if hw1 & 0xff80 != 0xfa80 || hw2 & 0xf080 != 0xf000 {
        return None;
    }
    let (how, what) = (HOW[((hw2 >> 4) & 7) as usize], WHAT[((hw1 >> 4) & 7) as usize]);
    if how.is_empty() || what.is_empty() {
        return None;
    }
    Some(format!("{how}{what} {}, {}, {}", r((hw2 >> 8) & 15), r(hw1 & 15), r(hw2 & 15)))
}

/// The sum of the differences between two registers' bytes, which is how a
/// video codec measures how alike two blocks are.
fn counted(hw1: u32, hw2: u32) -> Option<String> {
    if hw1 & 0xfff0 != 0xfb70 || hw2 & 0x00f0 != 0 {
        return None;
    }
    let (rd, rn, rm) = (r((hw2 >> 8) & 15), r(hw1 & 15), r(hw2 & 15));
    // A fourth register to add the total to, or the code that says there is
    // none.
    Some(match (hw2 >> 12) & 15 {
        15 => format!("usad8 {rd}, {rn}, {rm}"),
        ra => format!("usada8 {rd}, {rn}, {rm}, {}", r(ra)),
    })
}

/// Loads and stores that also say something about ordering: the pair that
/// claims a location and checks nobody else wrote it, and the pair that makes
/// everything before or after them visible to the other core.
fn ordered(hw1: u32, hw2: u32) -> Option<String> {
    let load = match hw1 & 0xfff0 {
        0xe8d0 => true,
        0xe8c0 => false,
        _ => return None,
    };
    if hw2 & 0x0f00 != 0x0f00 {
        return None;
    }
    let (rt, rn) = (r((hw2 >> 12) & 15), r(hw1 & 15));
    let name = match ((hw2 >> 4) & 15, load) {
        (0x8, true) => "ldab",
        (0x9, true) => "ldah",
        (0xa, true) => "lda",
        (0xc, true) => "ldaexb",
        (0xd, true) => "ldaexh",
        (0xe, true) => "ldaex",
        (0x8, false) => "stlb",
        (0x9, false) => "stlh",
        (0xa, false) => "stl",
        (0xc, false) => "stlexb",
        (0xd, false) => "stlexh",
        (0xe, false) => "stlex",
        _ => return None,
    };
    // A store that claims a location reports whether it succeeded, and the
    // register it reports in sits where the plain stores keep nothing.
    if name.starts_with("stlex") {
        return Some(format!("{name} {}, {rt}, [{rn}]", r(hw2 & 15)));
    }
    if hw2 & 15 != 15 {
        return None;
    }
    Some(format!("{name} {rt}, [{rn}]"))
}

/// Instructions the general decoder reads as a close relative: it drops the
/// bit that says which half of a register to take, which way a table is
/// indexed, or which way to shift.
fn misnamed(hw1: u32, hw2: u32) -> Option<String> {
    // Packing one half of each of two registers into one. Which halves is the
    // whole difference between the two names, and it is one bit.
    if hw1 & 0xfff0 == 0xeac0 && hw2 & 0x0010 == 0 {
        let shift = (((hw2 >> 12) & 7) << 2) | ((hw2 >> 6) & 3);
        let (rd, rn, rm) = (r((hw2 >> 8) & 15), r(hw1 & 15), r(hw2 & 15));
        return Some(match ((hw2 >> 5) & 1, shift) {
            (0, 0) => format!("pkhbt {rd}, {rn}, {rm}"),
            (0, n) => format!("pkhbt {rd}, {rn}, {rm}, lsl #{n}"),
            (_, 0) => format!("pkhtb {rd}, {rn}, {rm}, asr #32"),
            (_, n) => format!("pkhtb {rd}, {rn}, {rm}, asr #{n}"),
        });
    }
    // A jump through a table of offsets. One bit says whether the entries are
    // bytes or halfwords, which is how far the jump can reach.
    if hw1 & 0xfff0 == 0xe8d0 && hw2 & 0xffe0 == 0xf000 {
        let (rn, rm) = (r(hw1 & 15), r(hw2 & 15));
        return Some(match hw2 & 0x10 {
            0 => format!("tbb [{rn}, {rm}]"),
            _ => format!("tbh [{rn}, {rm}, lsl #1]"),
        });
    }
    // Moving a register through a shift. The two bits that say which shift
    // are read as one, so a rotate came back as an arithmetic shift.
    if hw1 & 0xffef == 0xea4f && hw2 & 0x8000 == 0 {
        let amount = (((hw2 >> 12) & 7) << 2) | ((hw2 >> 6) & 3);
        let (rd, rm) = (r((hw2 >> 8) & 15), r(hw2 & 15));
        let s = if hw1 & 0x10 != 0 { "s" } else { "" };
        let name = ["lsl", "lsr", "asr", "ror"][((hw2 >> 4) & 3) as usize];
        return Some(match ((hw2 >> 4) & 3, amount) {
            // No shift at all is a plain move, and a rotate by nothing is the
            // one that brings the carry flag in.
            (0, 0) => format!("mov{s}.w {rd}, {rm}"),
            (3, 0) => format!("rrx{s} {rd}, {rm}"),
            // A shift right by nothing means all the way, because shifting
            // right by nothing would be the move that is already spelled
            // above.
            (1 | 2, 0) => format!("{name}{s}.w {rd}, {rm}, #32"),
            _ => format!("{name}{s}.w {rd}, {rm}, #{amount}"),
        });
    }
    // Multiply-accumulate on packed halfwords, where one bit swaps the second
    // register's halves before multiplying.
    if hw1 & 0xfff0 == 0xfb20 && hw2 & 0x00e0 == 0 {
        let x = if hw2 & 0x10 != 0 { "x" } else { "" };
        let (rd, rn, rm) = (r((hw2 >> 8) & 15), r(hw1 & 15), r(hw2 & 15));
        return Some(match (hw2 >> 12) & 15 {
            15 => format!("smuad{x} {rd}, {rn}, {rm}"),
            ra => format!("smlad{x} {rd}, {rn}, {rm}, {}", r(ra)),
        });
    }
    // The same, keeping a sixty-four bit total across two registers.
    if hw1 & 0xffe0 == 0xfbc0 && hw2 & 0x00e0 == 0x00c0 {
        let name = if hw1 & 0x10 == 0 { "smlald" } else { "smlsld" };
        let x = if hw2 & 0x10 != 0 { "x" } else { "" };
        let (lo, hi) = (r((hw2 >> 12) & 15), r((hw2 >> 8) & 15));
        return Some(format!("{name}{x} {lo}, {hi}, {}, {}", r(hw1 & 15), r(hw2 & 15)));
    }
    saturate(hw1, hw2).or_else(|| bit_field(hw1, hw2))
}

/// Clamping a value to a given number of bits. The encoding stores one less
/// than that number for the signed forms, because clamping to zero bits would
/// mean nothing; reading the field as written makes every one of them name a
/// width one short.
fn saturate(hw1: u32, hw2: u32) -> Option<String> {
    let (signed, sixteen) = match hw1 & 0xffd0 {
        0xf300 => (true, hw1 & 0x0020 != 0),
        0xf380 => (false, hw1 & 0x0020 != 0),
        _ => return None,
    };
    if hw2 & 0x8000 != 0 {
        return None;
    }
    let shift = (((hw2 >> 12) & 7) << 2) | ((hw2 >> 6) & 3);
    // The halfword forms have no room for a shift, and the encoding that would
    // hold one is how the two are told apart.
    if sixteen && (shift != 0 || hw2 & 0x0030 != 0) {
        return None;
    }
    let (rd, rn) = (r((hw2 >> 8) & 15), r(hw1 & 15));
    let width = (hw2 & 31) + u32::from(signed);
    let name = match (signed, sixteen) {
        (true, true) => "ssat16",
        (true, false) => "ssat",
        (false, true) => "usat16",
        (false, false) => "usat",
    };
    if sixteen {
        return Some(format!("{name} {rd}, #{width}, {rn}"));
    }
    Some(match ((hw2 >> 4) & 3, shift) {
        (0, 0) => format!("{name} {rd}, #{width}, {rn}"),
        (0, n) => format!("{name} {rd}, #{width}, {rn}, lsl #{n}"),
        (_, 0) => format!("{name} {rd}, #{width}, {rn}, asr #32"),
        (_, n) => format!("{name} {rd}, #{width}, {rn}, asr #{n}"),
    })
}

/// Copying or clearing a run of bits inside a register. The encoding names the
/// last bit of the run; a reader wants to know how many bits there are, and
/// the difference is what makes one of these legible.
fn bit_field(hw1: u32, hw2: u32) -> Option<String> {
    if hw1 & 0xfbf0 != 0xf360 || hw2 & 0x8020 != 0 {
        return None;
    }
    let lsb = (((hw2 >> 12) & 7) << 2) | ((hw2 >> 6) & 3);
    let msb = hw2 & 31;
    if msb < lsb {
        return None;
    }
    let (rd, width) = (r((hw2 >> 8) & 15), msb - lsb + 1);
    // A source of all ones is the encoding for clearing rather than copying.
    Some(match hw1 & 15 {
        15 => format!("bfc {rd}, #{lsb}, #{width}"),
        rn => format!("bfi {rd}, {}, #{lsb}, #{width}", r(rn)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(bytes: [u8; 4]) -> String {
        let hw1 = u16::from_le_bytes([bytes[0], bytes[1]]) as u32;
        let hw2 = u16::from_le_bytes([bytes[2], bytes[3]]) as u32;
        decode(hw1, hw2).unwrap_or_else(|| "(declined)".into())
    }

    /// The element size, which the general decoder read from the wrong row of
    /// the grid: every byte-wide instruction came back named as its halfword
    /// neighbour, so the program appeared to be doing half the work.
    #[test]
    fn the_element_size_is_the_one_the_encoding_says() {
        assert_eq!(t([0xc1, 0xfa, 0x12, 0xf0]), "qsub8 r0, r1, r2");
        assert_eq!(t([0xd1, 0xfa, 0x12, 0xf0]), "qsub16 r0, r1, r2");
        assert_eq!(t([0x81, 0xfa, 0x42, 0xf0]), "uadd8 r0, r1, r2");
        assert_eq!(t([0x81, 0xfa, 0x02, 0xf0]), "sadd8 r0, r1, r2");
        assert_eq!(t([0x81, 0xfa, 0x62, 0xf0]), "uhadd8 r0, r1, r2");
        assert_eq!(t([0xe1, 0xfa, 0x02, 0xf0]), "ssax r0, r1, r2");
        assert_eq!(t([0xe1, 0xfa, 0x42, 0xf0]), "usax r0, r1, r2");
    }

    /// Taking a location and letting it go, which is how one core waits for
    /// another. None of these read at all before.
    #[test]
    fn the_instructions_that_take_a_lock_read() {
        assert_eq!(t([0xd1, 0xe8, 0xef, 0x0f]), "ldaex r0, [r1]");
        assert_eq!(t([0xd1, 0xe8, 0xaf, 0x0f]), "lda r0, [r1]");
        assert_eq!(t([0xc1, 0xe8, 0xaf, 0x0f]), "stl r0, [r1]");
        assert_eq!(t([0xc2, 0xe8, 0xe0, 0x1f]), "stlex r0, r1, [r2]");
        assert_eq!(t([0xc2, 0xe8, 0xc0, 0x1f]), "stlexb r0, r1, [r2]");
    }

    /// A number that has to be read differently from how it is stored: a
    /// clamp's width counts from one, and a bit field's encoding names its
    /// last bit where a reader wants its length.
    #[test]
    fn a_width_is_a_count_not_the_field_as_written() {
        assert_eq!(t([0x01, 0xf3, 0x07, 0x00]), "ssat r0, #8, r1");
        assert_eq!(t([0x81, 0xf3, 0x08, 0x00]), "usat r0, #8, r1");
        assert_eq!(t([0x61, 0xf3, 0x0b, 0x10]), "bfi r0, r1, #4, #8");
        assert_eq!(t([0x6f, 0xf3, 0x0b, 0x10]), "bfc r0, #4, #8");
    }

    /// Instructions named after a close relative: which half of a register is
    /// taken, how wide a jump table's entries are, and which way to shift.
    #[test]
    fn a_neighbouring_instruction_is_not_this_one() {
        assert_eq!(t([0xc1, 0xea, 0x22, 0x20]), "pkhtb r0, r1, r2, asr #8");
        assert_eq!(t([0xc1, 0xea, 0x02, 0x00]), "pkhbt r0, r1, r2");
        assert_eq!(t([0xd0, 0xe8, 0x11, 0xf0]), "tbh [r0, r1, lsl #1]");
        assert_eq!(t([0xd0, 0xe8, 0x01, 0xf0]), "tbb [r0, r1]");
        assert_eq!(t([0x4f, 0xea, 0x31, 0x20]), "ror.w r0, r1, #8");
        assert_eq!(t([0x21, 0xfb, 0x12, 0x30]), "smladx r0, r1, r2, r3");
    }
}
