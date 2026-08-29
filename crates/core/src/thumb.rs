//! The Thumb instructions the general decoder gets wrong or does not know.
//!
//! Most of Thumb is read by `yaxpeax-arm`, and this does not replace it. It
//! goes first and answers for the encodings where that decoder is silent or
//! mistaken; everything it does not claim falls through unchanged. Measured
//! against the RP2350 boot ROM's own listing, what it claims is 689 of the
//! 7539 instructions there, every one of which read as a different instruction
//! before.
//!
//! There are three reasons an encoding is here.
//!
//! The immediates. `movw r3, #0x7cd4` came back as `0x70cd4`: the four-bit
//! field at the top of the encoding was shifted sixteen places instead of
//! twelve. That is the worst kind of wrong, because the line looks right.
//! `mvn` came back as `mov`, which loses the inversion and so inverts the
//! meaning. A system register came back under another register's name.
//!
//! The ARMv8-M security instructions. `sg` marks the one place a non-secure
//! caller may enter secure code, and it read as an `ldrd`; `tt` asks what a
//! pointer is allowed to reach, and it read as a `strex`. A file that guards
//! itself is exactly the file whose guards should be legible.
//!
//! The RCP. An RP2350 has a coprocessor of Raspberry Pi's own for checking
//! that a program is running the way it was written: canaries at the edges of
//! functions, a count of the steps taken, booleans stored so that a flipped
//! bit is detectable. About one instruction in sixteen of the boot ROM is one
//! of these, and the general decoder reads them as the generic coprocessor
//! moves they are encoded as, which says nothing.
//!
//! The RCP names are only used when the caller says the chip is an RP2350.
//! Coprocessor seven belongs to whoever builds the chip, so on any other ARM
//! machine these bytes mean whatever that machine's designers decided, and the
//! honest reading there is the generic one.

use crate::code::Insn;

const R: [&str; 16] =
    ["r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "sl", "fp", "ip", "sp", "lr", "pc"];

fn r(n: u32) -> &'static str {
    R[(n & 15) as usize]
}

/// Decode one instruction, or decline it. `rcp` says whether coprocessor seven
/// may be read as the RP2350's redundancy coprocessor.
pub fn decode(bytes: &[u8], rcp: bool) -> Option<Insn> {
    let hw1 = u16::from_le_bytes([*bytes.first()?, *bytes.get(1)?]) as u32;
    // The two-byte instructions worth correcting are the two that step out of
    // secure code, which differ from an ordinary branch by one bit.
    if let Some(text) = narrow(hw1) {
        return Some(Insn { len: 2, text, target: None });
    }
    // Everything else here is four bytes, and the top five bits of the first
    // halfword are what say an instruction is that long.
    if hw1 >> 11 < 0b11101 {
        return None;
    }
    let hw2 = u16::from_le_bytes([*bytes.get(2)?, *bytes.get(3)?]) as u32;
    let text = wide(hw1, hw2, rcp)?;
    Some(Insn { len: 4, text, target: None })
}

/// The two-byte encodings this corrects: branching to code that is not secure,
/// which is an ordinary register branch with one more bit set.
fn narrow(hw1: u32) -> Option<String> {
    match hw1 & 0xff87 {
        0x4704 => Some(format!("bxns {}", r((hw1 >> 3) & 15))),
        0x4784 => Some(format!("blxns {}", r((hw1 >> 3) & 15))),
        _ => None,
    }
}

fn wide(hw1: u32, hw2: u32, rcp: bool) -> Option<String> {
    // The secure gateway, which is a single fixed word and the only
    // instruction a non-secure caller may land on.
    if hw1 == 0xe97f && hw2 == 0xe97f {
        return Some("sg".to_string());
    }
    if let Some(text) = test_target(hw1, hw2) {
        return Some(text);
    }
    if let Some(text) = wide_immediate(hw1, hw2) {
        return Some(text);
    }
    if let Some(text) = system_register(hw1, hw2) {
        return Some(text);
    }
    if let Some(text) = store_immediate(hw1, hw2) {
        return Some(text);
    }
    if rcp { redundancy(hw1, hw2) } else { None }
}

/// `tt` and its variants: ask the memory protection unit what a pointer is
/// allowed to reach, without reaching it. Two bits say whose permissions to
/// ask about, the caller's or the unprivileged and non-secure ones.
fn test_target(hw1: u32, hw2: u32) -> Option<String> {
    if hw1 & 0xfff0 != 0xe840 || hw2 & 0xf03f != 0xf000 {
        return None;
    }
    let name = match ((hw2 >> 7) & 1, (hw2 >> 6) & 1) {
        (0, 0) => "tt",
        (0, 1) => "ttt",
        (1, 0) => "tta",
        _ => "ttat",
    };
    Some(format!("{name} {}, {}", r((hw2 >> 8) & 15), r(hw1 & 15)))
}

/// The two instructions that build a constant a halfword at a time, and the
/// one that loads a constant already inverted.
fn wide_immediate(hw1: u32, hw2: u32) -> Option<String> {
    // A sixteen-bit constant, whose bits are spread across both halfwords: the
    // top four at the end of the first, then one, then three, then eight.
    // The second halfword's top bit is what tells these apart from a wide
    // conditional branch, which shares the first halfword's pattern.
    if (hw1 & 0xfbf0 == 0xf240 || hw1 & 0xfbf0 == 0xf2c0) && hw2 & 0x8000 == 0 {
        let value = ((hw1 & 15) << 12) | (((hw1 >> 10) & 1) << 11) | (((hw2 >> 12) & 7) << 8) | (hw2 & 0xff);
        let name = if hw1 & 0x0080 == 0 { "movw" } else { "movt" };
        return Some(format!("{name} {}, #{value}", r((hw2 >> 8) & 15)));
    }
    // Load the inverse of a constant. This is the same encoding as `orn` with
    // the source register set to the program counter, which is how the machine
    // spells "there is no source register" — and reading it as a plain move
    // drops the inversion and states the opposite of what happens.
    if hw1 & 0xfbef == 0xf06f && hw2 & 0x8000 == 0 {
        let value = expand(((hw1 >> 10) & 1) << 11 | ((hw2 >> 12) & 7) << 8 | (hw2 & 0xff));
        let s = if hw1 & 0x10 != 0 { "s" } else { "" };
        return Some(format!("mvn{s}.w {}, #{value}", r((hw2 >> 8) & 15)));
    }
    None
}

/// The constant a data-processing instruction holds, which is twelve bits
/// standing for a thirty-two bit value: either a byte placed and repeated, or
/// a byte with its top bit set, rotated.
fn expand(imm12: u32) -> u32 {
    let byte = imm12 & 0xff;
    if imm12 & 0xc00 == 0 {
        return match (imm12 >> 8) & 3 {
            0 => byte,
            1 => (byte << 16) | byte,
            2 => (byte << 24) | (byte << 8),
            _ => (byte << 24) | (byte << 16) | (byte << 8) | byte,
        };
    }
    (0x80 | (byte & 0x7f)).rotate_right(imm12 >> 7)
}

/// A store to a fixed distance from a register.
///
/// The general decoder reads the twelve-bit distance as a register number and
/// a shift, so `[r4, #4]` comes back as `[r4, r4]` and the instruction appears
/// to write somewhere else entirely. It reads the matching loads correctly,
/// which is why only the stores are taken here.
fn store_immediate(hw1: u32, hw2: u32) -> Option<String> {
    let name = match hw1 & 0xfff0 {
        0xf880 => "strb.w",
        0xf8a0 => "strh.w",
        0xf8c0 => "str.w",
        _ => return None,
    };
    // A store through the program counter is not an instruction.
    if hw1 & 15 == 15 {
        return None;
    }
    let (rt, rn, offset) = (r((hw2 >> 12) & 15), r(hw1 & 15), hw2 & 0xfff);
    Some(match offset {
        0 => format!("{name} {rt}, [{rn}]"),
        n => format!("{name} {rt}, [{rn}, #{n}]"),
    })
}

/// Reading and writing the registers that are the processor's own state rather
/// than a program's: which stack it is on, what it is allowed to interrupt,
/// and where the stack is not allowed to grow past.
fn system_register(hw1: u32, hw2: u32) -> Option<String> {
    if hw1 == 0xf3ef && hw2 & 0xf000 == 0x8000 {
        return Some(format!("mrs {}, {}", r((hw2 >> 8) & 15), special(hw2 & 0xff)));
    }
    // The two bits that say which parts of the flags a write touches are only
    // meaningful when the flags are what is being written, and every other
    // register ignores them.
    // Two bits say which parts of the flags a write touches. Anything but the
    // ordinary setting is a write to the flags themselves under a name this
    // does not spell, so it is left alone rather than named wrongly.
    if hw1 & 0xfff0 == 0xf380 && hw2 & 0xfc00 == 0x8800 {
        return Some(format!("msr {}, {}", special(hw2 & 0xff), r(hw1 & 15)));
    }
    None
}

/// What one of those registers is called. The numbering has a second copy of
/// the stack and the interrupt masks for the non-secure side of a chip that
/// has one, at a fixed distance above the first.
fn special(number: u32) -> String {
    let name = match number & 0x7f {
        0 => "APSR",
        1 => "IAPSR",
        2 => "EAPSR",
        3 => "XPSR",
        5 => "IPSR",
        6 => "EPSR",
        7 => "IEPSR",
        8 => "MSP",
        9 => "PSP",
        10 => "MSPLIM",
        11 => "PSPLIM",
        16 => "PRIMASK",
        17 => "BASEPRI",
        18 => "BASEPRI_MAX",
        19 => "FAULTMASK",
        20 => "CONTROL",
        24 => "SP",
        _ => return format!("0x{number:x}"),
    };
    if number & 0x80 != 0 { format!("{name}_NS") } else { name.to_string() }
}

/// The RP2350's redundancy coprocessor.
///
/// Every one of these is a coprocessor instruction on port seven, and which
/// one it is comes down to the direction of the move, how many registers it
/// takes, and the two small numbers the encoding carries. The table is the
/// SDK's own header read backwards.
///
/// The `2` forms of the coprocessor instructions are the ones that answer
/// immediately; the others make the processor wait, so that a fault is harder
/// to step around. The suffix says which, the way the SDK's names do.
fn redundancy(hw1: u32, hw2: u32) -> Option<String> {
    // Coprocessor seven, and nothing else.
    if (hw2 >> 8) & 15 != 7 {
        return None;
    }
    let nodelay = hw1 & 0x1000 != 0;
    let delay = if nodelay { ", nodelay" } else { ", delay" };
    let rt = r((hw2 >> 12) & 15);

    // The two-register moves, which check one value against another.
    if hw1 & 0xeff0 == 0xec40 {
        let rt2 = r(hw1 & 15);
        let name = match ((hw2 >> 4) & 15, hw2 & 15) {
            (0, 8) => "rcp_b2valid",
            (1, 0) => "rcp_b2and",
            (2, 0) => "rcp_b2or",
            (3, 8) => "rcp_bxorvalid",
            (4, 0) => "rcp_bxortrue",
            (5, 8) => "rcp_bxorfalse",
            (6, 8) => "rcp_ivalid",
            (7, 0) => "rcp_iequal",
            (8, 0) => "rcp_salt_core0",
            (8, 1) => "rcp_salt_core1",
            _ => return None,
        };
        return Some(format!("{name} {rt}, {rt2}{delay}"));
    }

    // A single fixed word that stops the processor, which is what every check
    // above branches to when it fails.
    if hw1 & 0xeff0 == 0xee00 && hw2 == 0x0720 {
        return Some("rcp_panic".to_string());
    }

    // The one-register moves. Two four-bit coprocessor register numbers stand
    // for one eight-bit number: which canary, or how far through the program
    // the count should be.
    // A move to or from a coprocessor register, rather than an operation
    // inside the coprocessor: the two are told apart by one bit of the second
    // halfword.
    if hw1 & 0xef00 != 0xee00 || hw2 & 0x10 == 0 {
        return None;
    }
    let to_coprocessor = hw1 & 0x0010 == 0;
    let opc1 = (hw1 >> 5) & 7;
    let opc2 = (hw2 >> 5) & 7;
    let tag = ((hw1 & 15) << 4) | (hw2 & 15);
    Some(match (to_coprocessor, opc1, opc2) {
        (false, 0, 1) => format!("rcp_canary_get {rt}, 0x{tag:x} ({tag}){delay}"),
        (true, 0, 1) => format!("rcp_canary_check {rt}, 0x{tag:x} ({tag}){delay}"),
        (false, 1, 0) if tag == 0 => format!("rcp_canary_status {rt}{delay}"),
        (true, 1, 0) if tag == 0 => format!("rcp_bvalid {rt}{delay}"),
        (true, 2, 0) if tag == 0 => format!("rcp_btrue {rt}{delay}"),
        (true, 3, 1) if tag == 0 => format!("rcp_bfalse {rt}{delay}"),
        // Reading a random byte, which is the one of these the toolchain's own
        // listing leaves as a plain coprocessor move.
        (false, 2, 0) if tag == 0 && nodelay => format!("rcp_random_byte {rt}{delay}"),
        (true, 4, 0) => format!("rcp_count_set 0x{tag:x} ({tag}){delay}"),
        (true, 5, 1) => format!("rcp_count_check 0x{tag:x} ({tag}){delay}"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(bytes: &[u8]) -> String {
        decode(bytes, true).map(|i| i.text).unwrap_or_else(|| "(declined)".into())
    }

    /// A constant built into a register a halfword at a time. The general
    /// decoder shifted the top four bits sixteen places instead of twelve, so
    /// every one of the boot ROM's 105 of these named a different number.
    #[test]
    fn a_sixteen_bit_constant_is_the_number_the_instruction_holds() {
        assert_eq!(t(&[0x47, 0xf6, 0xd4, 0x43]), "movw r3, #31956");
        assert_eq!(t(&[0xc6, 0xf6, 0xff, 0x70]), "movt r0, #28671");
    }

    /// Loading the inverse of a constant, which read as loading the constant.
    #[test]
    fn loading_an_inverse_says_that_it_is_one() {
        assert_eq!(t(&[0x6f, 0xf0, 0x09, 0x00]), "mvn.w r0, #9");
        assert_eq!(t(&[0x7f, 0xf0, 0x00, 0x07]), "mvns.w r7, #0");
    }

    /// The security instructions, each of which read as an unrelated memory
    /// instruction: the gateway as a pair load, the permission test as a
    /// store, and the branch out of secure code as an ordinary branch.
    #[test]
    fn the_security_instructions_read_as_themselves() {
        assert_eq!(t(&[0x7f, 0xe9, 0x7f, 0xe9]), "sg");
        assert_eq!(t(&[0x45, 0xe8, 0x00, 0xf2]), "tt r2, r5");
        assert_eq!(t(&[0x40, 0xe8, 0x80, 0xf4]), "tta r4, r0");
        assert_eq!(t(&[0x40, 0xe8, 0xc0, 0xf1]), "ttat r1, r0");
        assert_eq!(t(&[0x04, 0x47]), "bxns r0");
        assert_eq!(t(&[0x84, 0x47]), "blxns r0");
    }

    /// The processor's own registers, by name. `primask` read as a general
    /// register of a mode this chip does not have, and the stack limit read as
    /// the flags.
    #[test]
    fn a_system_register_is_named() {
        assert_eq!(t(&[0xef, 0xf3, 0x10, 0x80]), "mrs r0, PRIMASK");
        assert_eq!(t(&[0xef, 0xf3, 0x0a, 0x85]), "mrs r5, MSPLIM");
        assert_eq!(t(&[0xef, 0xf3, 0x94, 0x8c]), "mrs ip, CONTROL_NS");
        assert_eq!(t(&[0x86, 0xf3, 0x0a, 0x88]), "msr MSPLIM, r6");
        assert_eq!(t(&[0x80, 0xf3, 0x11, 0x88]), "msr BASEPRI, r0");
    }

    /// A store to a fixed distance from a register, whose distance read as a
    /// register number: the boot ROM's `[r4, #4]` came back as `[r4, r4]`.
    #[test]
    fn a_store_offset_is_a_distance_not_a_register() {
        assert_eq!(t(&[0xc4, 0xf8, 0x04, 0xe0]), "str.w lr, [r4, #4]");
        assert_eq!(t(&[0xcc, 0xf8, 0x00, 0x70]), "str.w r7, [ip]");
        assert_eq!(t(&[0x84, 0xf8, 0x30, 0xe0]), "strb.w lr, [r4, #48]");
        assert_eq!(t(&[0xa4, 0xf8, 0x30, 0xe0]), "strh.w lr, [r4, #48]");
    }

    /// A wide conditional branch shares its first halfword with the
    /// instructions that build a constant, and is told apart by the second.
    /// Claiming it as one of those named a jump as an arithmetic instruction.
    #[test]
    fn a_wide_branch_is_not_a_constant() {
        assert_eq!(decode(&[0xc0, 0xf2, 0x59, 0x81], true), None);
    }

    /// The redundancy coprocessor, from the boot ROM's own bytes and its own
    /// listing's words.
    #[test]
    fn the_redundancy_coprocessor_reads() {
        assert_eq!(t(&[0x43, 0xfc, 0x70, 0x27]), "rcp_iequal r2, r3, nodelay");
        assert_eq!(t(&[0x41, 0xfc, 0x80, 0x07]), "rcp_salt_core0 r0, r1, nodelay");
        assert_eq!(t(&[0x40, 0xfc, 0x08, 0x37]), "rcp_b2valid r3, r0, nodelay");
        assert_eq!(t(&[0x06, 0xfe, 0x3c, 0x27]), "rcp_canary_check r2, 0x6c (108), nodelay");
        assert_eq!(t(&[0x16, 0xfe, 0x3c, 0x37]), "rcp_canary_get r3, 0x6c (108), nodelay");
        assert_eq!(t(&[0xa4, 0xfe, 0x38, 0x07]), "rcp_count_check 0x48 (72), nodelay");
        assert_eq!(t(&[0x84, 0xfe, 0x18, 0x07]), "rcp_count_set 0x48 (72), nodelay");
        assert_eq!(t(&[0x40, 0xfe, 0x10, 0x07]), "rcp_btrue r0, nodelay");
        assert_eq!(t(&[0x60, 0xfe, 0x30, 0x47]), "rcp_bfalse r4, nodelay");
        assert_eq!(t(&[0x20, 0xfe, 0x10, 0xc7]), "rcp_bvalid ip, nodelay");
        assert_eq!(t(&[0x30, 0xfe, 0x10, 0xf7]), "rcp_canary_status pc, nodelay");
        assert_eq!(t(&[0x00, 0xee, 0x20, 0x07]), "rcp_panic");
    }

    /// Those names belong to one chip. On any other ARM machine the same
    /// coprocessor is somebody else's, so the bytes are left to the general
    /// decoder rather than given a Raspberry Pi name.
    #[test]
    fn the_coprocessor_is_only_named_where_it_is_known() {
        let bytes = [0x43, 0xfc, 0x70, 0x27];
        assert!(decode(&bytes, true).is_some());
        assert_eq!(decode(&bytes, false), None);
        // What the chip's own security instructions mean is the standard's,
        // not Raspberry Pi's, so those are read either way.
        assert!(decode(&[0x7f, 0xe9, 0x7f, 0xe9], false).is_some());
    }
}
