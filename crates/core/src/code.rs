//! Machine code: how long one instruction is, and what it says.
//!
//! A template describes bytes where they sit, and for a fixed-width machine it
//! could describe an instruction itself. For a variable-width one it cannot:
//! how long an x86 instruction is, is only known by decoding it, and what it
//! means takes a table nobody would write twice. So this wraps decoders that
//! already exist, and the IR gets one field type whose length is whatever the
//! decoder says.
//!
//! Every decode answers, including the ones that fail. A section of code holds
//! padding between functions, tables of addresses, and on ARM the constants a
//! function loads from beside itself; none of that is an instruction. A byte
//! that decodes to nothing is a step of the machine's smallest instruction, so
//! that whatever follows the rubbish is read as code again rather than lost.

use std::fmt::Write as _;

use raki::Decode as _;
use yaxpeax_arch::{Decoder, LengthedInstruction as _, U8Reader};

/// The instruction sets a file can say it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Isa {
    /// 64-bit x86, which every desktop and server program is.
    X86_64,
    /// 32-bit x86, which the older half of them are.
    X86_32,
    /// 16-bit x86: an MS-DOS program, and the first instructions any PC runs.
    X86_16,
    /// 64-bit ARM, which a phone and a recent Mac are.
    Aarch64,
    /// 32-bit ARM, in the four-byte encoding.
    Arm,
    /// The same machine in its two-byte encoding, which is what an ARM
    /// microcontroller runs and what a Windows on ARM program is built as.
    Thumb,
    Riscv32,
    Riscv64,
    /// The RISC-V half of an RP2350, the chip in a Raspberry Pi Pico 2. The
    /// same 32-bit machine with the extensions that core implements and the
    /// instructions its designers added, which are read as themselves only
    /// here: elsewhere they are encodings the standard leaves undefined.
    Hazard3,
}

impl Isa {
    /// What this machine is called, for the type column.
    pub fn name(self) -> &'static str {
        match self {
            Isa::X86_64 => "x86-64",
            Isa::X86_32 => "x86",
            Isa::X86_16 => "x86-16",
            Isa::Aarch64 => "arm64",
            Isa::Arm => "arm",
            Isa::Thumb => "thumb",
            Isa::Riscv32 => "riscv32",
            Isa::Riscv64 => "riscv64",
            Isa::Hazard3 => "hazard3",
        }
    }

    /// The machine a type column named, for a pass that has the name and
    /// wants the decoder.
    pub fn named(name: &str) -> Option<Isa> {
        [
            Isa::X86_64,
            Isa::X86_32,
            Isa::X86_16,
            Isa::Aarch64,
            Isa::Arm,
            Isa::Thumb,
            Isa::Riscv32,
            Isa::Riscv64,
            Isa::Hazard3,
        ]
        .into_iter()
        .find(|isa| isa.name() == name)
    }

    /// The smallest an instruction can be, which is how far to step over
    /// something that is not one.
    fn step(self) -> usize {
        match self {
            Isa::X86_64 | Isa::X86_32 | Isa::X86_16 => 1,
            Isa::Aarch64 | Isa::Arm => 4,
            Isa::Thumb | Isa::Riscv32 | Isa::Riscv64 | Isa::Hazard3 => 2,
        }
    }

    /// The most it can be, which is how many bytes are worth reading to decode
    /// one.
    pub fn longest(self) -> usize {
        match self {
            Isa::X86_64 | Isa::X86_32 | Isa::X86_16 => 15,
            Isa::Riscv32 | Isa::Riscv64 | Isa::Hazard3 => 4,
            _ => 4,
        }
    }
}

/// One decoded instruction: how many bytes it took, and what it says.
#[derive(Debug, Clone, PartialEq)]
pub struct Insn {
    pub len: usize,
    pub text: String,
    /// Where a branch goes, as a distance from the first byte of this
    /// instruction. `None` for everything that is not a branch, and for the
    /// machines whose decoder here does not say.
    pub target: Option<i64>,
}

/// Decode the instruction at the front of `bytes`. Never fails: bytes that are
/// not an instruction come back as `(bad)` over as many of them as the machine
/// counts in.
pub fn decode(isa: Isa, bytes: &[u8]) -> Insn {
    if bytes.is_empty() {
        return Insn { len: 0, text: String::new(), target: None };
    }
    let decoded = match isa {
        Isa::X86_64 => x86_64(bytes),
        Isa::X86_32 => x86_32(bytes),
        Isa::X86_16 => x86_16(bytes),
        Isa::Aarch64 => aarch64(bytes),
        Isa::Arm => arm(bytes),
        Isa::Thumb => thumb(bytes),
        Isa::Riscv32 => crate::riscv::decode(bytes, false),
        Isa::Hazard3 => crate::riscv::decode(bytes, true),
        Isa::Riscv64 => riscv(bytes, true),
    };
    let mut insn =
        decoded.unwrap_or_else(|| Insn { len: isa.step().min(bytes.len()), text: "(bad)".into(), target: None });
    // A decoder that knows which of its operands is an address has already
    // said so, exactly. Reading the distance back out of the text is for the
    // ones that only write it down.
    if insn.target.is_none() {
        insn.target = relative_target(isa, &insn.text, insn.len);
    }
    insn
}

/// Where a branch goes, read out of the text the decoder wrote.
///
/// Both yaxpeax decoders write a branch as a distance with a `$` for where it
/// is counted from, and the two of them count from different places: x86 from
/// the end of the instruction, because that is what the instruction holds, and
/// ARM from the start, because that is what its instruction holds. This
/// answers in the one unit a reader of a file can use, which is a distance
/// from the first byte of the instruction.
///
/// The 32-bit RISC-V decoder in this crate answers for itself and never
/// reaches here. The 64-bit one is `raki`, which writes a distance as an
/// operand like any other number with nothing to say that this one is an
/// address, so its branches go unmarked rather than guessed at.
pub fn relative_target(isa: Isa, text: &str, len: usize) -> Option<i64> {
    let at = text.find('$')?;
    let rest = &text[at + 1..];
    let (sign, digits) = match rest.as_bytes().first()? {
        b'+' => (1i64, &rest[1..]),
        b'-' => (-1i64, &rest[1..]),
        _ => return None,
    };
    let digits = digits.strip_prefix("0x")?;
    let end = digits.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(digits.len());
    let value = i64::from_str_radix(&digits[..end], 16).ok()?;
    let from_start = match isa {
        Isa::X86_64 | Isa::X86_32 | Isa::X86_16 => len as i64,
        _ => 0,
    };
    Some(from_start + sign * value)
}

fn x86_64(bytes: &[u8]) -> Option<Insn> {
    let mut reader = U8Reader::new(bytes);
    let insn = yaxpeax_x86::long_mode::InstDecoder::default().decode(&mut reader).ok()?;
    Some(Insn { len: insn.len().to_const() as usize, text: insn.to_string(), target: None })
}

fn x86_32(bytes: &[u8]) -> Option<Insn> {
    let mut reader = U8Reader::new(bytes);
    let insn = yaxpeax_x86::protected_mode::InstDecoder::default().decode(&mut reader).ok()?;
    Some(Insn { len: insn.len().to_const() as usize, text: insn.to_string(), target: None })
}

fn x86_16(bytes: &[u8]) -> Option<Insn> {
    let mut reader = U8Reader::new(bytes);
    let insn = yaxpeax_x86::real_mode::InstDecoder::default().decode(&mut reader).ok()?;
    Some(Insn { len: insn.len().to_const() as usize, text: insn.to_string(), target: None })
}

fn aarch64(bytes: &[u8]) -> Option<Insn> {
    let mut reader = U8Reader::new(bytes);
    let insn = yaxpeax_arm::armv8::a64::InstDecoder::default().decode(&mut reader).ok()?;
    Some(Insn { len: 4, text: insn.to_string(), target: None })
}

fn arm(bytes: &[u8]) -> Option<Insn> {
    let mut reader = U8Reader::new(bytes);
    let insn = yaxpeax_arm::armv7::InstDecoder::armv7().decode(&mut reader).ok()?;
    Some(Insn { len: 4, text: insn.to_string(), target: None })
}

fn thumb(bytes: &[u8]) -> Option<Insn> {
    let mut reader = U8Reader::new(bytes);
    let insn = yaxpeax_arm::armv7::InstDecoder::default_thumb().decode(&mut reader).ok()?;
    let len = if insn.wide { 4 } else { 2 };
    Some(Insn { len, text: insn.to_string(), target: None })
}

/// RISC-V, where the low two bits of the first byte say whether the
/// instruction is the compressed two-byte form or the full four.
fn riscv(bytes: &[u8], sixty_four: bool) -> Option<Insn> {
    let isa = if sixty_four { raki::Isa::Rv64 } else { raki::Isa::Rv32 };
    if bytes[0] & 0b11 != 0b11 {
        let word = u16::from_le_bytes([bytes[0], *bytes.get(1)?]);
        let insn = word.decode(isa).ok()?;
        return Some(Insn { len: 2, text: text_of(&insn), target: None });
    }
    if bytes.len() < 4 {
        return None;
    }
    let word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let insn = word.decode(isa).ok()?;
    Some(Insn { len: 4, text: text_of(&insn), target: None })
}

/// A RISC-V instruction as text. The mnemonic is lowercased because the
/// compressed ones come back as `C.nop`, and every other machine here writes
/// its instructions in lower case, as does every RISC-V toolchain.
fn text_of(insn: &raki::Instruction) -> String {
    let mut out = String::new();
    let _ = write!(out, "{insn}");
    match out.split_once(' ') {
        Some((mnemonic, rest)) => format!("{} {rest}", mnemonic.to_lowercase()),
        None => out.to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_machine_reads_its_own_instructions() {
        // mov eax, 1
        assert_eq!(decode(Isa::X86_64, &[0xb8, 0x01, 0x00, 0x00, 0x00]).len, 5);
        // ret
        assert_eq!(decode(Isa::X86_64, &[0xc3]), Insn { len: 1, text: "ret".into(), target: None });
        // aarch64: ret
        let ret = decode(Isa::Aarch64, &[0xc0, 0x03, 0x5f, 0xd6]);
        assert_eq!(ret.len, 4);
        assert!(ret.text.contains("ret"), "{}", ret.text);
        // riscv: addi a0, zero, 1 (32-bit) and a compressed nop
        assert_eq!(decode(Isa::Riscv64, &[0x13, 0x05, 0x10, 0x00]).len, 4);
        // The compressed form, whose mnemonic the decoder writes as `C.nop`.
        assert_eq!(decode(Isa::Riscv64, &[0x01, 0x00]), Insn { len: 2, text: "c.nop".into(), target: None });
    }

    /// Both decoders write a branch as a distance, and they count it from
    /// different places. What comes back is counted from one place: the first
    /// byte of the instruction.
    #[test]
    fn a_branch_says_how_far_it_goes_from_where_it_is() {
        // call the next instruction, which is five bytes on.
        assert_eq!(decode(Isa::X86_64, &[0xe8, 0, 0, 0, 0]).target, Some(5));
        // A jump to itself, which is how a program hangs.
        assert_eq!(decode(Isa::X86_64, &[0xeb, 0xfe]).target, Some(0));
        // bl to its own address, and a branch eight bytes on.
        assert_eq!(decode(Isa::Aarch64, &[0x00, 0x00, 0x00, 0x94]).target, Some(0));
        assert_eq!(decode(Isa::Aarch64, &[0x02, 0x00, 0x00, 0x14]).target, Some(8));
        // Anything that is not a branch goes nowhere.
        assert_eq!(decode(Isa::X86_64, &[0xc3]).target, None);
    }

    #[test]
    fn bytes_that_are_not_an_instruction_step_by_the_smallest_one() {
        let bad = decode(Isa::X86_64, &[0x06]);
        assert_eq!(bad, Insn { len: 1, text: "(bad)".into(), target: None });
        // Four bytes of nothing on a machine whose instructions are four bytes.
        assert_eq!(decode(Isa::Aarch64, &[0xff, 0xff, 0xff, 0xff]).len, 4);
    }
}
