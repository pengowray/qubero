//! Analog Devices Blackfin boot streams: the `.ldr` files that ADI's loader
//! (`elfloader` in VisualDSP++ and CrossCore Embedded Studio, `bfin-elf-ldr`
//! in the GNU toolchain) writes for the boot ROM of the ADSP-BF50x, BF51x,
//! BF52x, BF54x and BF59x. ADI's loader writes Intel hex by default for a
//! flash boot; this template reads the binary form, which is what a flash
//! image or a firmware update holds.
//!
//! A boot stream is a list of blocks. Each block is a 16-byte header of four
//! little-endian words, and then a payload of as many bytes as the header's
//! byte count says. The four words are the block code, the target address, the
//! byte count and the argument. The block code holds a 4-bit DMA code in bits
//! 0 to 3, the flags in bits 4 to 15, the header check in bits 16 to 23, and
//! the signature 0xAD in bits 24 to 31. The header check is chosen so that the
//! 16 bytes of the header XOR to zero, which is the test U-Boot's `ldrinfo`
//! and ldrshrink both make. Here it is a constraint on the header check field,
//! worked out from the header's fields, rather than a checksum: a checksum is
//! only taken over bytes of the file, and `tascamfw` reads a stream like this
//! one from inside a scrambled run.
//!
//! The boot ROM copies each payload to the block's target address. A fill
//! block has no payload: the boot ROM writes the argument, a 32-bit value,
//! over byte count bytes at the target address. An ignore block's payload is
//! in the stream, and the boot ROM skips it. The boot ROM stops after the
//! block with the final flag. Payloads are Blackfin code or data, and Qubero
//! has no Blackfin instruction set, so every payload is read as bytes.
//!
//! ADI's loader puts a first block, one with the first flag set, in front of
//! each application (`.dxe` file) and each initcode in the stream. Its
//! argument is what ADI's loader manual calls the next `.dxe` pointer: the
//! number of bytes from the end of the first block's header to the first block
//! of the next program. So each program is read as its first block and the
//! blocks after it, as long as that argument says, and a constraint on the
//! first block checks that those blocks fill exactly that length. The blocks
//! stop at a final block, or before a block that would run past the next
//! program, and any bytes left before the next program are read as one bytes
//! field. An
//! initcode's program ends with an init block rather than a final block, and
//! the boot ROM carries on into the next program after it runs the initcode.
//! In every stream these sources describe, a first block is an ignore block
//! with a byte count of zero. ldrshrink writes the program's entry point as a
//! first block's target address. A first block with a payload would be read
//! the same way, but none has been seen.
//!
//! Blocks that no first block groups into a program, in a stream that does
//! not open with a first block, are read as a list of their own up to a final
//! block. After the stream, a run of 0xff is read as erased flash, and
//! anything after that as bytes.
//!
//! Field names are ADI's. U-Boot's `bootrom.h` names the four words of a
//! header in `ADI_BOOT_HEADER` (`dBlockCode`, `pTargetAddress`, `dByteCount`,
//! `dArgument`) and the pieces of the block code with its `BFLAG_*` masks. The
//! flags keep ADI's names without the `BFLAG_` prefix. Where ADI's name is an
//! abbreviation, the field has a plain name and gives ADI's in its
//! description: `header_check` is `HDRCHK`, `signature` is `HDRSIGN`,
//! `dma_code` is `DMACODE`. The names `programs`, `first_block`,
//! `after_last_block`, `erased` and `rest` are this template's.
//!
//! Bit 4 of the block code is `BFLAG_SAVE` in ADI's loader manual, whose
//! `-save` switch marks the blocks to keep for low-power and power-fail
//! cycles, and in the DR-05 homebrew project's structures. U-Boot's
//! `bootrom.h` spells it `BFLAG_SAFE`. This template calls it `save`. The
//! other `BFLAG_*` masks in `bootrom.h`, from `BFLAG_HOOK` up, are flags for
//! the boot ROM's functions (`ADI_BOOT_DATA`'s `dFlags`), not bits of a block
//! header, and they overlap `HDRCHK` and `HDRSIGN`; they are not here. None of
//! the sources says what each DMA code means or what `aux` is for, so the DMA
//! code is a plain number.
//!
//! The ADSP-BF53x and BF56x boot streams are a different format and are not
//! read here: their block headers are 10 bytes (an address, a count and 16
//! bits of flags), a BF561 stream opens with a 4-byte global header, and
//! their blocks may be compressed.
//!
//! Tascam's DR-05X recorder is updated with a file that has the same 32-byte
//! header as the Portacapture X8's (see `tascamfw`), followed by a boot stream
//! of this kind, scrambled with the same table. The DR-05 homebrew project
//! found an ADSP-BF514 in the older DR-05. None of the sources below names the
//! part in the DR-05X.
//!
//! Sources:
//! - U-Boot v2015.01 (Blackfin support was removed later):
//!   `arch/blackfin/include/asm/mach-common/bits/bootrom.h` for
//!   `ADI_BOOT_HEADER` and the `BFLAG_*` masks, `common/cmd_ldrinfo.c` for
//!   the XOR test, the signature test, and how fill and final blocks are
//!   stepped over, and `common/cmd_bootldr.c`, which takes a signature of
//!   0xAD and a first byte count of zero as the mark of a stream.
//! - ADI, VisualDSP++ 5.0 Loader and Utilities Manual, revision 2.4, May
//!   2010: the next `.dxe` pointer (page 1-7), table 2-7 (`-dmawidth` sets the
//!   DMA code, `-save` sets `BFLAG_SAVE`, `-callback` and `-CRC32` fill in the
//!   argument), pages 2-18 and 2-19 (each initcode gets a first block and ends
//!   with an init block), and table 3-4 (the first flag does the job the
//!   ignore flag does in a BF53x stream).
//! - ldrshrink by Peter Lawrence (<https://github.com/majbthrd/ldrshrink>),
//!   for the ADSP-SC58x and BF70x, whose boot ROMs read the same block
//!   layout: the header check, and a first block's argument counted as the
//!   total length of the blocks after it.
//! - The DR-05 homebrew project (<https://github.com/dr05-homebrew/dr05-homebrew>),
//!   `construct/structures.py`, for the block code's bit layout.
//! - The DR-05X V1.16 update in the sample collection, whose stream has 7,826
//!   blocks in two programs and matches all of the above.
//!
//! The boot chapters of ADI's BF5xx hardware reference manuals could not be
//! downloaded, so nothing here is checked against them.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until, Valid};

/// `HDRSIGN`, the last byte of every block header.
pub const SIGNATURE: u8 = 0xad;

/// Bytes in a block header: four 32-bit words.
pub const HEADER_BYTES: usize = 16;

/// The flags, numbered as bits of the 12-bit flags field, which starts at bit
/// 4 of the block code. ADI's masks are these bits shifted left by four:
/// `first` here is bit 10, and ADI's `BFLAG_FIRST` is 0x4000. Bits 2 and 3 are
/// reserved.
const FLAGS: &[(u32, &str)] = &[
    (0, "save"),
    (1, "aux"),
    (4, "fill"),
    (5, "quickboot"),
    (6, "callback"),
    (7, "init"),
    (8, "ignore"),
    (9, "indirect"),
    (FIRST, "first"),
    (FINAL, "final"),
];

const FILL: u32 = 4;
const FIRST: u32 = 10;
const FINAL: u32 = 11;

/// Whether the head of a file is a boot stream: a block header that is
/// signed, whose 16 bytes XOR to zero, and that has the first flag set.
pub fn is_bfldr(head: &[u8]) -> bool {
    let Some(h) = head.get(..HEADER_BYTES) else { return false };
    let first = 1u8 << (FIRST + 4 - 8);
    h[3] == SIGNATURE && h.iter().fold(0, |a, b| a ^ b) == 0 && h[1] & first != 0
}

pub fn bfldr() -> Template {
    Template::new("bfldr", stream())
}

/// A whole boot stream, and whatever follows it. Shared with `tascamfw`,
/// whose DR-05X update holds one.
pub(crate) fn stream() -> T {
    T::structure(
        "BootStream",
        vec![
            ("programs", T::repeat(program(), Until::While(first_block_ahead()))),
            // Blocks no first block starts: a stream written without one.
            ("blocks", T::when(block_ahead(), T::repeat(block(), Until::Cond(flag(FINAL))))),
            ("erased", T::when(E::Remaining.greater_than(E::lit(0)).both(E::peek(8, Big).equal_to(E::lit(0xff))), T::bytes(E::run(&[0xff])))),
            // Whatever is after the erased run. The stream does not say what
            // it is.
            ("rest", T::when(E::Remaining.greater_than(E::lit(0)), T::bytes(E::Remaining))),
        ],
    )
}

/// One program: its first block, and the blocks after it, up to where the
/// first block's argument says the next program starts.
///
/// Sized from that argument, and no further than the end of the stream, so a
/// wrong argument still reads and the constraint on `first_block` says it is
/// wrong.
pub(crate) fn program() -> T {
    let argument = E::peek_at(E::lit(96), 32, Little);
    T::sized(
        argument.add(E::lit(HEADER_BYTES as i128)).at_most(E::Remaining),
        T::structure(
            "Program",
            vec![
                ("first_block", block()),
                // Up to a final block, or to the last block that fits before
                // the next program, so that an argument that is too short
                // cuts no block after the first in two.
                ("blocks", T::repeat(block(), Until::Cond(flag(FINAL).either(block_fits().negate())))),
                // What is left before the next program: bytes after a final
                // block, which the boot ROM never reads, or the start of a
                // block that the argument cuts short.
                ("after_last_block", T::when(E::Remaining.greater_than(E::lit(0)), T::bytes(E::Remaining))),
            ],
        )
        .field_valid(
            "first_block",
            Valid::Expr {
                expr: E::within(&["first_block", "header", "argument"]).equal_to(E::size_of("blocks")),
                msg: Some("The argument does not match the total length of the blocks that follow".into()),
            },
        )
        .field_doc("after_last_block", "Bytes after the program's last whole block, up to where the first block's argument says the next program starts."),
    )
}

/// One block: the header, and the payload the header's byte count gives. A
/// fill block has no payload, and neither has a block whose byte count is
/// zero.
pub(crate) fn block() -> T {
    let fill = E::within(&["header", "flags"]).bit(FILL);
    let count = || E::within(&["header", "byte_count"]);
    T::structure(
        "Block",
        vec![("header", block_header()), ("payload", T::when(fill.negate().both(count().greater_than(E::lit(0))), T::bytes(count())))],
    )
}

/// `ADI_BOOT_HEADER`: the block code, split into its four pieces, and then the
/// target address, the byte count and the argument.
pub(crate) fn block_header() -> T {
    let bits = |n: u32| T::UInt { bits: n, endian: Little };
    T::structure(
        "BlockHeader",
        vec![
            ("dma_code", bits(4)),
            ("flags", T::flags("BlockFlags", bits(12), FLAGS)),
            ("header_check", T::u8()),
            ("signature", T::magic(&[SIGNATURE])),
            ("target_address", T::u32(Little)),
            ("byte_count", T::u32(Little)),
            ("argument", T::u32(Little)),
        ],
    )
    .field_valid("header_check", Valid::Expr { expr: header_xor().equal_to(E::lit(0)), msg: Some("The 16 header bytes do not XOR to zero".into()) })
    .field_doc("dma_code", "DMACODE: which DMA width the boot ROM uses for this block, as a code. ADI's loader sets it from its -dmawidth option.")
    .field_doc("flags", "ADI's BFLAG_ flags, named without the BFLAG_ prefix.")
    .field_doc("header_check", "HDRCHK: set so that the 16 bytes of this header XOR to zero.")
    .field_doc("signature", "HDRSIGN: 0xAD in every block header.")
    .field_doc("target_address", "TARGET ADDRESS: where the boot ROM writes this block's payload or fill value.")
    .field_doc("byte_count", "BYTE COUNT: the length of the payload. For a fill block, the number of bytes to fill, and the block has no payload.")
    .field_doc(
        "argument",
        "ARGUMENT: for a first block, the number of bytes from the end of this header to the next program. For a fill block, the 32-bit fill value. For other blocks, a value for the boot ROM, such as a callback's argument or a CRC-32.",
    )
}

/// The 16 bytes of a header XORed together, from inside the header: zero when
/// the header check is right.
///
/// Worked out from the fields rather than from the bytes, so it holds in any
/// space a header is read in. The signature is taken to be 0xAD, since a
/// header with another signature already fails on that.
fn header_xor() -> E {
    let code = E::field("dma_code")
        .bit_or(E::field("flags").shl(E::lit(4)))
        .bit_or(E::field("header_check").shl(E::lit(16)))
        .bit_or(E::lit(i128::from(SIGNATURE) << 24));
    let words = code.bit_xor(E::field("target_address")).bit_xor(E::field("byte_count")).bit_xor(E::field("argument"));
    let byte = |n: i128| words.clone().shr(E::lit(n));
    byte(0).bit_xor(byte(8)).bit_xor(byte(16)).bit_xor(byte(24)).and(E::lit(0xff))
}

/// Whether one of the flags is set in the block this is asked inside.
fn flag(bit: u32) -> E {
    E::within(&["header", "flags"]).bit(bit)
}

/// Whether a whole block fits in what is left: a header, and the payload its
/// byte count gives unless it is a fill block.
fn block_fits() -> E {
    let header = HEADER_BYTES as i128;
    let fill = E::peek(32, Little).bit(FILL + 4);
    let count = E::peek_at(E::lit(64), 32, Little);
    E::Remaining.greater_than(E::lit(header - 1)).both(fill.either(E::Remaining.greater_or_equal(count.add(E::lit(header)))))
}

/// Whether the bytes ahead are a signed block header.
fn block_ahead() -> E {
    E::Remaining.greater_than(E::lit(HEADER_BYTES as i128 - 1)).both(E::peek_at(E::lit(24), 8, Big).equal_to(E::lit(i128::from(SIGNATURE))))
}

/// Whether the bytes ahead are a signed block header with the first flag set.
fn first_block_ahead() -> E {
    block_ahead().both(E::peek(32, Little).bit(FIRST + 4))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// ADI's masks for the low half of the block code, as a stream writes
    /// them: DMA code 1, the code every block of the DR-05X stream has.
    pub(crate) const DMA_1: u16 = 0x0001;
    pub(crate) const FILL_BLOCK: u16 = 0x0100;
    pub(crate) const INIT_BLOCK: u16 = 0x0800;
    pub(crate) const IGNORE_BLOCK: u16 = 0x1000;
    pub(crate) const FIRST_BLOCK: u16 = 0x4000;
    pub(crate) const FINAL_BLOCK: u16 = 0x8000;

    /// One block: a header with its check byte filled in, then the payload.
    /// `code` is the low sixteen bits of the block code.
    pub(crate) fn block(code: u16, target: u32, count: u32, argument: u32, payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&code.to_le_bytes());
        v.extend_from_slice(&[0, SIGNATURE]);
        for w in [target, count, argument] {
            v.extend_from_slice(&w.to_le_bytes());
        }
        v[2] = v.iter().fold(0, |a, b| a ^ b);
        v.extend_from_slice(payload);
        v
    }

    /// A program: a first block whose argument is the length of `blocks`,
    /// then the blocks.
    pub(crate) fn program(entry: u32, blocks: &[Vec<u8>]) -> Vec<u8> {
        let rest: Vec<u8> = blocks.concat();
        let mut v = block(DMA_1 | FIRST_BLOCK | IGNORE_BLOCK, entry, 0, rest.len() as u32, &[]);
        v.extend_from_slice(&rest);
        v
    }

    /// An initcode program and an application, the way the DR-05X stream is
    /// laid out, and then erased flash.
    fn two_programs() -> Vec<u8> {
        let init = program(
            0xffa0_0000,
            &[block(DMA_1, 0xff80_4000, 4, 0, b"data"), block(DMA_1, 0xffa0_0000, 8, 0, b"initcode"), block(DMA_1 | INIT_BLOCK, 0xffa0_0000, 0, 0, &[])],
        );
        let app = program(
            0xffa0_0000,
            &[
                block(DMA_1 | FILL_BLOCK, 0x1000, 0x40, 0, &[]),
                block(DMA_1, 0x2000, 6, 0, b"sdram!"),
                block(DMA_1 | FINAL_BLOCK, 0xffa0_0000, 0, 0, &[]),
            ],
        );
        let mut v = [init, app].concat();
        v.extend_from_slice(&[0xff; 12]);
        v
    }

    fn named(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> Vec<usize> {
        let mut p = path.to_vec();
        for name in names {
            // A number steps to that element of a list.
            if let Ok(i) = name.parse::<usize>() {
                p.push(i);
                continue;
            }
            p = ev.child_named(d, &p, name).unwrap().unwrap_or_else(|| panic!("no {name} under {p:?}"));
        }
        p
    }

    fn node(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> crate::eval::NodeInfo {
        let p = named(ev, d, path, names);
        ev.node(d, &p).unwrap()
    }

    fn int(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> i128 {
        let p = named(ev, d, path, names);
        let v = ev.node(d, &p).unwrap().value;
        v.as_int().unwrap_or_else(|| panic!("{names:?} is not a number: {v:?}"))
    }

    /// Offset and size in bytes.
    fn extent(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> (u64, u64) {
        let p = named(ev, d, path, names);
        let n = ev.node(d, &p).unwrap();
        (n.offset_bits / 8, n.size_bits / 8)
    }

    fn verdict(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> (bool, String) {
        let p = named(ev, d, path, names);
        let v = ev.valid_of(d, &p).unwrap().unwrap_or_else(|| panic!("{names:?} has no constraint"));
        (v.ok, v.text)
    }

    fn at(path: &[usize], i: usize) -> Vec<usize> {
        [path, &[i]].concat()
    }

    #[test]
    fn the_head_of_a_boot_stream_is_recognised() {
        let v = two_programs();
        assert!(is_bfldr(&v));
        assert!(!is_bfldr(&v[..15]), "too short for a header");
        // A block that is not a first block does not open a stream.
        assert!(!is_bfldr(&block(DMA_1 | FINAL_BLOCK, 0xffa0_0000, 0, 0, &[])));
        let mut unsigned = v.clone();
        unsigned[3] = 0xac;
        unsigned[2] ^= 0xad ^ 0xac;
        assert!(!is_bfldr(&unsigned), "a header that XORs to zero but is not signed");
        let mut unchecked = v.clone();
        unchecked[4] ^= 1;
        assert!(!is_bfldr(&unchecked), "a header that does not XOR to zero");
    }

    #[test]
    fn an_imxrt_image_and_random_bytes_are_not_taken_for_a_boot_stream() {
        let image = crate::formats::imxrt::tests::image(&[]);
        assert!(!is_bfldr(&image));
        assert_ne!(crate::formats::sniff(&image, image.len() as u64), Some("bfldr"));
        // Bytes from a fixed-seed xorshift: one head in about 130,000 random
        // ones passes all three tests, so none of these should.
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        for _ in 0..2000 {
            let mut head = [0u8; HEADER_BYTES];
            for b in head.iter_mut() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *b = x as u8;
            }
            assert!(!is_bfldr(&head), "{head:02x?}");
        }
    }

    #[test]
    fn a_stream_is_read_as_programs_where_the_first_blocks_say() {
        let v = two_programs();
        assert_eq!(crate::formats::sniff(&v, v.len() as u64), Some("bfldr"));
        let d = Document::new(MemSource(v.clone()));
        let mut ev = Evaluator::new(bfldr());
        let ev = &mut ev;
        let programs = named(ev, &d, &[], &["programs"]);
        assert_eq!(ev.node(&d, &programs).unwrap().child_count, 2);
        let init = at(&programs, 0);
        let init_len = 16 * 4 + 4 + 8;
        assert_eq!(extent(ev, &d, &init, &[]), (0, init_len));
        assert_eq!(int(ev, &d, &init, &["first_block", "header", "argument"]), init_len as i128 - 16);
        assert!(verdict(ev, &d, &init, &["first_block"]).0);
        let blocks = named(ev, &d, &init, &["blocks"]);
        assert_eq!(ev.node(&d, &blocks).unwrap().child_count, 3, "an initcode ends with its init block");
        assert_eq!(int(ev, &d, &at(&blocks, 2), &["header", "flags"]), 1 << 7);
        assert!(node(ev, &d, &init, &["after_last_block"]).absent);

        let app = at(&programs, 1);
        assert_eq!(extent(ev, &d, &app, &[]).0, init_len);
        assert!(verdict(ev, &d, &app, &["first_block"]).0);
        let blocks = named(ev, &d, &app, &["blocks"]);
        assert_eq!(ev.node(&d, &blocks).unwrap().child_count, 3);
        assert_eq!(int(ev, &d, &app, &["first_block", "header", "target_address"]), 0xffa0_0000);
        assert!(node(ev, &d, &app, &["first_block", "payload"]).absent);
        let end = v.len() as u64 - 12;
        let (app_at, app_len) = extent(ev, &d, &app, &[]);
        assert_eq!(app_at + app_len, end);

        assert!(node(ev, &d, &[], &["blocks"]).absent);
        assert_eq!(extent(ev, &d, &[], &["erased"]), (end, 12));
        assert!(node(ev, &d, &[], &["rest"]).absent);
        // Every header in the stream passes its check.
        for program in [&init, &app] {
            assert!(verdict(ev, &d, program, &["first_block", "header", "header_check"]).0);
            let blocks = named(ev, &d, program, &["blocks"]);
            for i in 0..ev.node(&d, &blocks).unwrap().child_count as usize {
                assert!(verdict(ev, &d, &at(&blocks, i), &["header", "header_check"]).0, "block {i}");
            }
        }
    }

    #[test]
    fn a_fill_block_has_no_payload_and_an_ignore_block_keeps_its_payload() {
        let v = program(
            0xffa0_0000,
            &[
                block(DMA_1 | FILL_BLOCK, 0x1000, 0x400, 0xdead_beef, &[]),
                block(DMA_1 | IGNORE_BLOCK, 0x2000, 4, 0, b"skip"),
                block(DMA_1 | FINAL_BLOCK, 0xffa0_0000, 0, 0, &[]),
            ],
        );
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(bfldr());
        let ev = &mut ev;
        let blocks = named(ev, &d, &[], &["programs", "0", "blocks"]);
        let fill = at(&blocks, 0);
        assert_eq!(extent(ev, &d, &fill, &[]), (16, 16), "the header and nothing after it");
        assert!(node(ev, &d, &fill, &["payload"]).absent);
        assert_eq!(int(ev, &d, &fill, &["header", "byte_count"]), 0x400);
        assert_eq!(int(ev, &d, &fill, &["header", "argument"]), 0xdead_beef);
        let Value::Flags { set, .. } = node(ev, &d, &fill, &["header", "flags"]).value else { panic!("not flags") };
        assert_eq!(set, vec!["fill".to_string()]);
        let ignore = at(&blocks, 1);
        assert_eq!(extent(ev, &d, &ignore, &["payload"]), (48, 4));
        assert_eq!(extent(ev, &d, &at(&blocks, 2), &[]).0, 52, "the block after the ignored payload");
        assert_eq!(int(ev, &d, &ignore, &["header", "dma_code"]), 1);
    }

    #[test]
    fn a_header_that_does_not_xor_to_zero_is_flagged() {
        let mut v = two_programs();
        // The target address of the initcode's first data block.
        v[16 + 4] ^= 0x10;
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(bfldr());
        let blocks = named(&mut ev, &d, &[], &["programs", "0", "blocks"]);
        let (ok, text) = verdict(&mut ev, &d, &at(&blocks, 0), &["header", "header_check"]);
        assert!(!ok);
        assert_eq!(text, "Fails the check: The 16 header bytes do not XOR to zero");
        assert!(verdict(&mut ev, &d, &at(&blocks, 1), &["header", "header_check"]).0, "the next block is still right");
    }

    #[test]
    fn a_header_with_the_wrong_signature_is_flagged() {
        let mut v = two_programs();
        // The initcode's first data block, with the signature and the check
        // byte both changed, so that the header still XORs to zero.
        v[16 + 3] = 0xac;
        v[16 + 2] ^= 0xad ^ 0xac;
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(bfldr());
        let blocks = named(&mut ev, &d, &[], &["programs", "0", "blocks"]);
        let sign = named(&mut ev, &d, &at(&blocks, 0), &["header", "signature"]);
        assert!(matches!(ev.node(&d, &sign).unwrap().value, Value::Magic { ok: false, .. }), "{:?}", ev.node(&d, &sign).unwrap().value);
        assert!(ev.node(&d, &sign).unwrap().problem.is_some());
    }

    #[test]
    fn a_final_block_ends_a_stream_with_no_first_block() {
        let mut v = [block(DMA_1, 0x1000, 4, 0, b"code"), block(DMA_1 | FINAL_BLOCK, 0xffa0_0000, 0, 0, &[])].concat();
        // Erased flash, then something written after it.
        v.extend_from_slice(&[0xff; 8]);
        v.extend_from_slice(&block(DMA_1, 0x2000, 0, 0, &[]));
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(bfldr());
        let ev = &mut ev;
        assert_eq!(node(ev, &d, &[], &["programs"]).child_count, 0);
        let blocks = named(ev, &d, &[], &["blocks"]);
        assert_eq!(ev.node(&d, &blocks).unwrap().child_count, 2);
        assert_eq!(extent(ev, &d, &[], &["erased"]), (36, 8));
        assert_eq!(extent(ev, &d, &[], &["rest"]), (44, 16));
    }

    #[test]
    fn a_final_block_before_the_end_of_a_program_fails_the_first_blocks_check() {
        let mut v = program(0xffa0_0000, &[block(DMA_1 | FINAL_BLOCK, 0xffa0_0000, 0, 0, &[]), block(DMA_1, 0x1000, 4, 0, b"late")]);
        v.extend_from_slice(&[0xff; 4]);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(bfldr());
        let ev = &mut ev;
        let program = named(ev, &d, &[], &["programs", "0"]);
        assert_eq!(node(ev, &d, &program, &["blocks"]).child_count, 1);
        assert_eq!(extent(ev, &d, &program, &["after_last_block"]), (32, 20));
        let (ok, text) = verdict(ev, &d, &program, &["first_block"]);
        assert!(!ok);
        assert_eq!(text, "Fails the check: The argument does not match the total length of the blocks that follow");
        assert_eq!(extent(ev, &d, &[], &["erased"]), (52, 4));
    }

    #[test]
    fn a_first_block_whose_argument_ends_inside_a_block_is_flagged() {
        let mut v = program(0xffa0_0000, &[block(DMA_1, 0x1000, 4, 0, b"code"), block(DMA_1, 0x2000, 4, 0, b"data"), block(DMA_1 | FINAL_BLOCK, 0xffa0_0000, 0, 0, &[])]);
        // An argument two bytes into the second block's payload, and the
        // check byte fixed to match.
        let short = 20 + 16 + 2u32;
        let old = u32::from_le_bytes(v[12..16].try_into().unwrap());
        v[12..16].copy_from_slice(&short.to_le_bytes());
        v[2] ^= (old ^ short).to_le_bytes().iter().fold(0, |a, b| a ^ b);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(bfldr());
        let ev = &mut ev;
        let program = named(ev, &d, &[], &["programs", "0"]);
        assert_eq!(extent(ev, &d, &program, &[]), (0, 16 + 38));
        assert!(verdict(ev, &d, &program, &["first_block", "header", "header_check"]).0);
        assert!(!verdict(ev, &d, &program, &["first_block"]).0);
        // The block the argument cuts is not read as a block.
        assert_eq!(node(ev, &d, &program, &["blocks"]).child_count, 1);
        assert_eq!(extent(ev, &d, &program, &["after_last_block"]), (36, 18));
        assert_eq!(extent(ev, &d, &[], &["rest"]).0, 54);
    }

    #[test]
    fn a_first_block_whose_argument_is_zero_is_a_program_of_one_block() {
        // A writer that leaves the argument at zero: the first block is a
        // program on its own, and the blocks after it are read up to the
        // final block.
        let mut v = block(DMA_1 | FIRST_BLOCK | IGNORE_BLOCK, 0xffa0_0000, 0, 0, &[]);
        v.extend_from_slice(&block(DMA_1, 0x1000, 4, 0, b"code"));
        v.extend_from_slice(&block(DMA_1 | FINAL_BLOCK, 0xffa0_0000, 0, 0, &[]));
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(bfldr());
        let ev = &mut ev;
        let program = named(ev, &d, &[], &["programs", "0"]);
        assert_eq!(extent(ev, &d, &program, &[]), (0, 16));
        assert!(verdict(ev, &d, &program, &["first_block"]).0);
        assert_eq!(node(ev, &d, &[], &["programs"]).child_count, 1);
        assert_eq!(node(ev, &d, &[], &["blocks"]).child_count, 2);
    }

    #[test]
    fn a_first_block_whose_argument_runs_past_the_stream_is_flagged() {
        let mut v = program(0xffa0_0000, &[block(DMA_1 | FINAL_BLOCK, 0xffa0_0000, 0, 0, &[])]);
        // Point the argument 0x100 bytes further on, and fix the check byte.
        v[13] ^= 0x01;
        v[2] ^= 0x01;
        let d = Document::new(MemSource(v.clone()));
        let mut ev = Evaluator::new(bfldr());
        let ev = &mut ev;
        let program = named(ev, &d, &[], &["programs", "0"]);
        assert_eq!(extent(ev, &d, &program, &[]), (0, v.len() as u64), "stopped at the end of the stream");
        assert!(!verdict(ev, &d, &program, &["first_block"]).0);
        assert!(verdict(ev, &d, &program, &["first_block", "header", "header_check"]).0);
    }
}
