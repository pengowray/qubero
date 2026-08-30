//! UF2: the file you drag onto a microcontroller that has appeared as a USB
//! drive.
//!
//! It exists because of what is on the other end. The chip is pretending to be
//! a memory stick, and the thing copying the file is a desktop operating
//! system that may write the blocks in any order, in any sizes, and may write
//! some of them twice. So every block is made to stand alone: 512 bytes, a
//! magic number at each end, its own destination address, and its own count of
//! how many blocks there are altogether. A block that arrives is written; a
//! block that arrives twice is written twice and does no harm; and the chip
//! knows it has the whole program when it has seen them all.
//!
//! That leaves a file where the same 512 bytes repeat with only the numbers
//! changing, which reads well: the address column is the program's memory map,
//! and a gap in it is a gap in the program.
//!
//! A Raspberry Pi Pico's UF2 says which chip it is for, and that is worth more
//! here than anywhere else in this crate. Two of the numbers mean a Pico 2
//! running the ARM half of its processor and one means the same chip running
//! the RISC-V half, and those are the only files that say so. Both halves have
//! instructions that only their maker defines, which a decoder may not name
//! without being told the chip: see [`code::Isa`](crate::code::Isa).
//!
//! The template alone stops at the payload, and it has to: the payloads are
//! 256 bytes each and the program runs across them, so an instruction may
//! begin in one block and end in the next, and decoding a block on its own
//! would misread one instruction at most seams and say so with confidence.
//! [`image`] is the way past that. It reads the headers, sorts the payloads by
//! the address each says it goes to, and hands back the runs of the file they
//! occupy, which [`Gathered`](crate::gather::Gathered) reads as the one stream
//! they make. A decoder run over that sees the program the chip sees.

use crate::code::Isa;
use crate::gather::Extent;
use crate::source::Source;
use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// The first magic number, which is also what picks this template out of a
/// directory of files.
pub const MAGIC: &[u8] = b"UF2\n";

/// The four bits of `flags` that mean anything, and what each one changes.
const FLAGS: &[(u32, &str)] = &[
    (0, "not main flash"),
    (12, "file container"),
    (13, "family id present"),
    (14, "md5 present"),
    (15, "extension tags present"),
];

/// The chips whose makers have registered a number, as far as a Raspberry Pi
/// is concerned. The number sits where a file container would keep its total
/// size, and the flag above says which of the two it is.
///
/// The three RP2350 numbers are one chip in three arrangements: its ARM
/// processor with the security extension's two worlds either in play or not,
/// and its RISC-V processor, which is the same silicon running different cores.
const FAMILIES: &[(i128, &str)] = &[
    (0xe48bff55, "CYW43 firmware"),
    (0xe48bff56, "RP2040"),
    (0xe48bff57, "absolute address"),
    (0xe48bff58, "data"),
    (0xe48bff59, "RP2350, Arm, secure"),
    (0xe48bff5a, "RP2350, RISC-V"),
    (0xe48bff5b, "RP2350, Arm, non-secure"),
];

/// A block's header, as much of it as assembling the program needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    address: u64,
    payload: Extent,
    family: u32,
    named_family: bool,
}

/// One stretch of the chip's memory the file fills, and the runs of the file
/// that fill it.
///
/// A UF2 is a set of these rather than one, for the same reason a program has
/// several segments: a Pico 2 image puts code in flash and may put other
/// things elsewhere, and the addresses in between belong to neither.
pub struct Run {
    /// Where the first byte goes in the chip.
    pub address: u64,
    /// The runs of the file holding it, in address order, to be read as one
    /// stream by [`Gathered`](crate::gather::Gathered).
    pub extents: Vec<Extent>,
}

impl Run {
    pub fn len(&self) -> u64 {
        self.extents.iter().map(|e| e.len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.extents.is_empty()
    }
}

/// What a UF2 carries.
pub struct Image {
    /// The stretches of memory it fills, lowest address first. Separate runs
    /// are separate because the addresses between them are not in the file,
    /// and joining them would put two instructions next to each other that are
    /// not next to each other in the chip.
    pub runs: Vec<Run>,
    /// The chip every block that named one agreed on.
    pub family: Option<u32>,
}

impl Image {
    /// Which machine's instructions this holds, if the file said.
    ///
    /// This is the only place in the crate that gets to answer, and that is
    /// why it is worth having: the vendor instructions of both halves of an
    /// RP2350 may only be named when something has said which chip it is, and
    /// the family number is that something.
    pub fn isa(&self) -> Option<Isa> {
        match self.family? {
            // An original Pico, whose processor has no coprocessor of
            // Raspberry Pi's for a decoder to name.
            0xe48bff56 => Some(Isa::Thumb),
            0xe48bff59 | 0xe48bff5b => Some(Isa::Rp2350Arm),
            0xe48bff5a => Some(Isa::Hazard3),
            _ => None,
        }
    }
}

/// Whether a family number names a processor, as against saying something
/// about where the bytes go or what they are. Only a processor decides a
/// decoder, and only these vote on which one.
fn is_processor(family: u32) -> bool {
    matches!(family, 0xe48bff56 | 0xe48bff59 | 0xe48bff5a | 0xe48bff5b)
}

/// Read the headers and work out what the file fills, and with what.
///
/// The blocks are taken in address order rather than in the order they are
/// written, because nothing requires those to agree: the format is built to
/// survive an operating system that copies blocks in whatever order it likes,
/// and a file that has been through one may have kept that order.
///
/// Blocks that are not program are left out. A file container carries a file
/// rather than an image, and a block marked as not going to the main flash is
/// going somewhere else; putting either among the code would be making up the
/// program.
pub fn image<S: Source>(source: &S) -> Image {
    let mut headers: Vec<Header> = Vec::new();
    let mut at = 0;
    let mut block = [0u8; 32];
    while at + BLOCK <= source.len_bytes() {
        source.read_bytes(at, &mut block);
        if let Some(header) = header(&block, at) {
            headers.push(header);
        }
        at += BLOCK;
    }
    // A chip every block that named one agreed on, or nothing. One file with
    // two processors named in it is not a file about one processor, and
    // guessing which to believe would be choosing a decoder on a coin toss.
    let named: Vec<u32> = headers.iter().filter(|h| h.named_family && is_processor(h.family)).map(|h| h.family).collect();
    let family = match named.first() {
        Some(first) if named.iter().all(|f| f == first) => Some(*first),
        _ => None,
    };
    // Only the processor's own blocks are the program: the rest are bytes
    // going somewhere at an address of their own.
    if family.is_some() {
        headers.retain(|h| !h.named_family || is_processor(h.family));
    }
    // By address, and within one address by where the block is in the file,
    // latest first. The same address twice is a file written that way, and the
    // chip is left holding whatever was written last, so that is the block to
    // keep: sorting the latest to the front is what makes the dedup below,
    // which keeps the first of each run, keep the right one.
    headers.sort_by(|a, b| a.address.cmp(&b.address).then(b.payload.at.cmp(&a.payload.at)));
    headers.dedup_by_key(|h| h.address);

    // Cut the blocks into runs wherever the addresses stop being consecutive.
    let mut runs: Vec<Run> = Vec::new();
    let mut next_address = None;
    for header in headers {
        match (next_address, runs.last_mut()) {
            (Some(expected), Some(run)) if expected == header.address => run.extents.push(header.payload),
            _ => runs.push(Run { address: header.address, extents: vec![header.payload] }),
        }
        next_address = Some(header.address + header.payload.len);
    }
    Image { runs, family }
}

/// One block's header, or nothing if what is at `at` is not a block, or is a
/// block carrying something other than program.
fn header(bytes: &[u8; 32], at: u64) -> Option<Header> {
    let word = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    if word(0) != 0x0a324655 || word(4) != 0x9e5d5157 {
        return None;
    }
    let flags = word(8);
    // Not the program: a file being carried rather than an image, or bytes
    // bound somewhere other than the flash the program runs from.
    if flags & 0x1001 != 0 {
        return None;
    }
    let size = word(16);
    if size == 0 || size as u64 > PAYLOAD {
        return None;
    }
    Some(Header {
        address: word(12) as u64,
        payload: Extent::new(at + HEADER, size as u64),
        family: word(28),
        named_family: flags & 0x2000 != 0,
    })
}

/// The fixed sizes every block has: the whole block, its header, and the most
/// payload that leaves room for the magic number at the end.
const BLOCK: u64 = 512;
const HEADER: u64 = 32;
const PAYLOAD: u64 = 476;

pub fn uf2() -> Template {
    Template::new("uf2", T::repeat(block(), Until::End))
}

/// One block: a header of eight numbers, the payload, padding out to a fixed
/// size, and a magic number at the end so that a reader who has lost its place
/// can find the edge again.
fn block() -> T {
    T::structure(
        "Block",
        vec![
            ("magic_start0", T::magic(MAGIC)),
            // The second magic number, which is here because one is not enough
            // to tell this from a file that happens to start with the word.
            // Both keep the names the format's own description gives them.
            ("magic_start1", T::magic(b"\x57\x51\x5d\x9e")),
            ("flags", T::flags("Flags", T::u32(Little), FLAGS)),
            // Where in the chip's memory this payload goes. This is the column
            // to read down: it is the program's own map.
            ("address", T::u32(Little)),
            // How much of the 476 bytes below is program and how much is
            // padding. A Raspberry Pi's tools write 256.
            ("payload_size", T::u32(Little)),
            // Which block this is, and how many there are. A reader knows
            // it has the whole program when it has seen every number once.
            ("block_number", T::u32(Little)),
            ("block_count", T::u32(Little)),
            // One field, two meanings, and the flags say which: the chip this
            // is for, or the size of the file being carried.
            ("family", T::enumeration_hex("Family", T::u32(Little), FAMILIES)),
            ("payload", T::bytes(E::field("payload_size"))),
            // Everything between the payload and the last four bytes is unused
            // space the format keeps so that every block is the same length.
            ("padding", T::bytes(E::lit(476).sub(E::field("payload_size")))),
            ("magic_end", T::magic(b"\x30\x6f\xb1\x0a")),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::Evaluator;
    use crate::gather::Gathered;
    use crate::source::MemSource;

    /// One block of a Pico 2 UF2, built here rather than found: 512 bytes with
    /// a 256-byte payload, addressed at the start of flash.
    fn one_block(family: u32, payload_size: u32) -> Vec<u8> {
        block_at(family, payload_size, 0x10000000, 0, 1, &[])
    }

    /// A block with everything about it chosen, and a payload given as its
    /// first bytes followed by padding.
    fn block_at(family: u32, payload_size: u32, address: u32, number: u32, total: u32, payload: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(MAGIC);
        b.extend(0x9e5d5157u32.to_le_bytes());
        // The flag that says the last header field is a chip and not a size.
        b.extend(0x00002000u32.to_le_bytes());
        b.extend(address.to_le_bytes());
        b.extend(payload_size.to_le_bytes());
        b.extend(number.to_le_bytes());
        b.extend(total.to_le_bytes());
        b.extend(family.to_le_bytes());
        b.extend(payload);
        b.extend(std::iter::repeat_n(0xa5, 476 - payload.len()));
        b.extend(0x0ab16f30u32.to_le_bytes());
        assert_eq!(b.len(), 512);
        b
    }

    fn read(bytes: Vec<u8>) -> Vec<(String, String)> {
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(uf2());
        let block = ev.node(&doc, &[0]).unwrap();
        (0..block.child_count as usize)
            .map(|i| {
                let node = ev.node(&doc, &[0, i]).unwrap();
                (node.name.to_string(), format!("{:?}", node.value))
            })
            .collect()
    }

    /// The header reads, and the number that says which chip reads as the chip
    /// rather than as a number nobody can look up.
    #[test]
    fn a_block_says_which_chip_it_is_for() {
        let fields = read(one_block(0xe48bff5a, 256));
        let family = fields.iter().find(|(name, _)| name == "family").expect("a family field");
        assert!(family.1.contains("RP2350, RISC-V"), "{:?}", family);
        let size = fields.iter().find(|(name, _)| name == "payload_size").expect("a size field");
        assert!(size.1.contains("256"), "{:?}", size);
    }

    /// An instruction that begins in one block and ends in the next reads as
    /// the one instruction it is.
    ///
    /// This is what [`image`] is for. The payloads are 256 bytes and a four-byte
    /// instruction may start at the 255th, so decoding a block on its own would
    /// read two bytes of a call as a whole instruction and the other two as the
    /// front of another. Every RP2350 firmware has a couple of hundred of these.
    #[test]
    fn an_instruction_across_a_join_reads_as_one_instruction() {
        // `bl` to itself, cut in half: two bytes at the end of one payload and
        // two at the start of the next.
        let mut first = vec![0u8; 254];
        first.extend([0xff, 0xf7]);
        let mut file = block_at(0xe48bff59, 256, 0x10000000, 0, 2, &first);
        file.extend(block_at(0xe48bff59, 256, 0x10000100, 1, 2, &[0xfe, 0xff]));

        let source = MemSource(file);
        let program = image(&source);
        // One stretch of memory, because the two blocks are consecutive.
        assert_eq!(program.runs.len(), 1);
        assert_eq!(program.runs[0].address, 0x10000000);
        assert_eq!(program.runs[0].len(), 512);
        let isa = program.isa().expect("the family names a processor");

        let code = Gathered::new(&source, program.runs[0].extents.iter().copied());
        let mut bytes = vec![0u8; code.len_bytes() as usize];
        code.read_bytes(0, &mut bytes);
        let insn = crate::code::decode(isa, &bytes[254..258]);
        assert_eq!(insn.len, 4);
        assert!(insn.text.starts_with("bl"), "{}", insn.text);
        // And it is in two places, which is the truth about where it is.
        assert_eq!(code.origin(254, 4).len(), 2);
    }

    /// One address written twice keeps what the chip would be left holding,
    /// which is the block written last.
    #[test]
    fn the_last_block_to_claim_an_address_is_the_one_kept() {
        let mut file = block_at(0xe48bff59, 4, 0x10000000, 0, 2, &[1, 2, 3, 4]);
        file.extend(block_at(0xe48bff59, 4, 0x10000000, 1, 2, &[5, 6, 7, 8]));
        let source = MemSource(file);
        let program = image(&source);
        assert_eq!(program.runs.len(), 1);
        let code = Gathered::new(&source, program.runs[0].extents.iter().copied());
        let mut bytes = [0u8; 4];
        code.read_bytes(0, &mut bytes);
        assert_eq!(bytes, [5, 6, 7, 8]);
    }

    /// Addresses that are not consecutive are not one stretch of memory.
    /// Joining them would put two instructions next to each other that the
    /// chip never puts next to each other.
    #[test]
    fn a_gap_in_the_addresses_is_a_gap_in_the_program() {
        let mut file = block_at(0xe48bff59, 256, 0x10000000, 0, 2, &[]);
        file.extend(block_at(0xe48bff59, 256, 0x20000000, 1, 2, &[]));
        let program = image(&MemSource(file));
        assert_eq!(program.runs.len(), 2);
        assert_eq!(program.runs[0].address, 0x10000000);
        assert_eq!(program.runs[1].address, 0x20000000);
    }

    /// A block that says where it goes rather than which chip it is for does
    /// not get a vote on the chip, and does not stop the rest from agreeing.
    #[test]
    fn only_a_processor_chooses_the_decoder() {
        let mut file = block_at(0xe48bff57, 256, 0x11000000, 0, 1, &[]);
        file.extend(block_at(0xe48bff5a, 256, 0x10000000, 0, 1, &[]));
        assert_eq!(image(&MemSource(file)).isa(), Some(crate::code::Isa::Hazard3));
        // Two chips named in one file is not one chip.
        let mut split = block_at(0xe48bff59, 256, 0x10000000, 0, 1, &[]);
        split.extend(block_at(0xe48bff5a, 256, 0x10000100, 0, 1, &[]));
        assert_eq!(image(&MemSource(split)).isa(), None);
    }

    /// The payload is as long as the block said it was, and the rest of the
    /// space is accounted for rather than left out.
    #[test]
    fn the_payload_is_as_long_as_the_block_says() {
        for size in [256u32, 476, 32] {
            let fields = read(one_block(0xe48bff56, size));
            let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
            assert!(names.contains(&"payload"), "{names:?}");
            assert!(names.contains(&"magic_end"), "{names:?}");
        }
    }
}
