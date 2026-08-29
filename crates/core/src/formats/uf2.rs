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
//! What this does not do is disassemble. The payloads are 256 bytes each and
//! the program runs across them, so an instruction may begin in one block and
//! end in the next; decoding a block on its own would misread one instruction
//! at most seams and say so with confidence. Reading the code means joining the
//! payloads in address order first, which is a piece of machinery this crate
//! does not have yet.

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
            ("magic", T::magic(MAGIC)),
            // The second magic number, which is here because one is not enough
            // to tell this from a file that happens to start with the word.
            ("magic2", T::magic(b"\x57\x51\x5d\x9e")),
            ("flags", T::flags("Flags", T::u32(Little), FLAGS)),
            // Where in the chip's memory this payload goes. This is the column
            // to read down: it is the program's own map.
            ("address", T::u32(Little)),
            // How much of the 476 bytes below is program and how much is
            // padding. A Raspberry Pi's tools write 256.
            ("payload_size", T::u32(Little)),
            ("block", T::u32(Little)),
            ("blocks", T::u32(Little)),
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
    use crate::source::MemSource;

    /// One block of a Pico 2 UF2, built here rather than found: 512 bytes with
    /// a 256-byte payload, addressed at the start of flash.
    fn one_block(family: u32, payload_size: u32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(MAGIC);
        b.extend(0x9e5d5157u32.to_le_bytes());
        // The flag that says the last header field is a chip and not a size.
        b.extend(0x00002000u32.to_le_bytes());
        b.extend(0x10000000u32.to_le_bytes());
        b.extend(payload_size.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend(family.to_le_bytes());
        b.extend(std::iter::repeat_n(0xa5, 476));
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
