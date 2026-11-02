//! Tascam firmware updates: the files a user copies to a Tascam recorder's
//! memory card to update it, such as `#PortaX8_0217.203` for the Portacapture
//! X8 and `DR05X_071.116` for the DR-05X.
//!
//! The file is a 32-byte header, the firmware image, and a 4-byte copy of the
//! header's checksum. The image and the copy are scrambled: every byte is
//! passed through a fixed substitution table (see [`crate::codec::tascam`]),
//! and both products with samples here use the same table. The header's
//! fields are big-endian: an 8-byte product code in ASCII, padded with NULs,
//! then the version, the build, the build date, a checksum, the length of the
//! file and 8 reserved bytes, which are zero. The checksum is the byte sum,
//! kept to 32 bits, of the unscrambled image. The image and the checksum copy
//! are read as two separate runs, so that the header's checksum covers the
//! image alone.
//!
//! The product code is Tascam's code for the product. The X8's code is
//! `DR-TURBO`, Tascam's internal project name for the X8. The same name is in
//! the source file paths in the X8's firmware
//! (`C:\Perforce\projects\SSR\DR-TURBO\...`). The DR-05X's code is `DR-05X`. [`is_update`] recognises a file by the other fields of its
//! header and accepts any product code.
//!
//! How the image is read depends on its first unscrambled bytes, and not on
//! the product code, so an update for a product with no sample here still
//! reads if its image is one of these two kinds:
//!
//! - An NXP i.MX RT flash image, which has `FCFB` at offset 0 and an image
//!   vector table at 0x1000, which is what `imxrt::is_imxrt_flash` tests. The
//!   X8's image is one, and it is read with the X8's layout, described below.
//!   An i.MX RT image from another product is read with the X8's layout as
//!   well, and that layout is unlikely to match the other product's image.
//! - An Analog Devices Blackfin boot stream, the format the `bfldr` template
//!   reads, which starts with a 16-byte block header that has the first flag
//!   set. Boot streams with these block headers are for the ADSP-BF50x,
//!   BF51x, BF52x, BF54x and BF59x. The DR-05X's image is one, and it is read
//!   with `bfldr`'s pieces. It holds an initcode program at offset 0 and the
//!   application at 0x748, which ends with a final block at 0x1F2AA0, and
//!   then 1,356 bytes of erased flash (0xFF). The DR-05 homebrew project found
//!   an ADSP-BF514 in the older DR-05. That has not been checked against the
//!   DR-05X.
//!
//! An image of any other kind is read as bytes.
//!
//! # The Portacapture X8's image
//!
//! Teardown photos of the X8 show an NXP MIMXRT1062DVL6B processor, a part
//! of the i.MX RT1060 family. The image is the flash it boots from, and its
//! first 8 KiB are read with the same pieces as `imxrt`: the FlexSPI
//! configuration block, the IVT, the boot data and the DCD. The IVT's `entry`
//! field holds the address of Tascam's bootloader, and the application is at
//! 0x20000. The bootloader copies everything from 0x20000 on into SDRAM at
//! 0x80000000 and jumps to it without checking anything, so an application
//! address is its flash offset plus 0x7ffe0000.
//!
//! Both programs were built with IAR Embedded Workbench and keep their startup
//! data the way IAR's linker does: an init table of routines and the ranges
//! each one fills, and the packed runs those routines unpack. Nothing marks
//! where a table is, but IAR's linker puts it straight after the 36-byte
//! routine that turns on the floating-point unit, `__iar_init_core`. So each
//! program's code is read up to the end of that routine, wherever the routine
//! is found, and the init table is read after it. The bootloader's table
//! zeroes two ranges and unpacks one `packbits` run into the DTCM. The
//! application's table zeroes seven ranges, unpacks three `lz77` runs into
//! SDRAM, the ITCM and the DTCM, and then runs the C++ constructors.
//!
//! The bootloader's packed run and the application's first packed run are
//! read where their table entries point. The application's other two runs
//! are read straight after the run before them, and each is checked against
//! its entry's pointer: in the X8 the three runs are back to back, in the
//! order the table lists them. If a release puts them somewhere else, they
//! are read from the wrong place and the check on each of them fails. What
//! each run unpacks to is checked against the length its entry gives.
//!
//! The application opens with its vector table, a stamp of the version and
//! build that should agree with the header, and then data the code uses: a
//! Qt for MCUs font map with the three TrueType fonts it lists, and four WAV
//! files, which are sounds the recorder plays. Each font and each WAV is read
//! as an embedded file that can be opened as a document of its own. Two more
//! WAVs and several hundred images are inside the code run. Only the code
//! says where they are, so they stay part of that run.
//!
//! All of the code is read as Thumb instructions: the bootloader's, the
//! application's two runs, 11 MB before the init table and half a megabyte
//! after it, and the code that runs from the ITCM. The application's runs mix
//! code with string pools, vtables and images, so parts of them are data read
//! as instructions. A listing counts a run's instructions before it shows the
//! run's row. The count reads each instruction's length from its first
//! halfword without decoding it (see `eval/insnrun.rs`), and the 11 MB run's
//! 4.2 million instructions take about a third of a second in a debug build.
//!
//! The ITCM code is linked to run at 0x400, and the template IR has no way to
//! give a run a load address, so its branch targets read as distances from
//! the instruction rather than as addresses.

use crate::codec::Codec;
use crate::template::{Check, Checksum, Covers, Encoding, Endian::*, Expr as E, Named, StrLen, Template, Ty as T, Until, Valid};

use super::{bfldr, imxrt};

/// Bytes of header before the scrambled image.
const HEADER_BYTES: i128 = 0x20;

/// Bytes in the product code that opens the header.
const PRODUCT_CODE_BYTES: usize = 8;

/// The two kinds of image there is a reading for, as [`image`] numbers them.
const IMXRT_FLASH: i128 = 1;
const BOOT_STREAM: i128 = 2;

/// The copy of the checksum that ends the image.
const CHECKSUM_BYTES: i128 = 4;

/// IAR's `__iar_init_core`, the routine that turns on the floating-point unit,
/// and the two bytes that pad it to a word. IAR's linker puts the init table
/// straight after it, in both programs and both releases. These 36 bytes
/// occur exactly twice in each image, once in each program.
const INIT_CORE: &[u8] = &[
    0x4e, 0xf6, 0x88, 0x51, 0xce, 0xf2, 0x00, 0x01, 0x08, 0x68, 0x40, 0xf4, 0x70, 0x00, 0x08, 0x60, 0xbf, 0xf3,
    0x4f, 0x8f, 0xbf, 0xf3, 0x6f, 0x8f, 0x4f, 0xf0, 0x00, 0x70, 0xe1, 0xee, 0x10, 0x0a, 0x70, 0x47, 0x00, 0x00,
];

/// Where the application starts in the flash: what the bootloader copies from.
const APP_AT: i128 = 0x20000;

/// `RIFF`, read as a big-endian number, which opens each WAV file.
const RIFF: i128 = 0x5249_4646;

/// Whether the head of a file is the header of a Tascam update: a product code
/// of printable ASCII padded with NULs, a build date with a year from 1990 to
/// 2100, a month from 1 to 12 and a day from 1 to 31, the length of the whole
/// file (`len`) at byte 20, and 8 reserved bytes of zero.
///
/// The product code is not compared with a list, so an update for a product
/// with no sample here is recognised too. The length and the zero bytes are
/// most of the evidence.
pub fn is_update(head: &[u8], len: u64) -> bool {
    let Some(h) = head.get(..HEADER_BYTES as usize) else { return false };
    let code = &h[..PRODUCT_CODE_BYTES];
    let text = code.iter().position(|&b| b == 0).unwrap_or(PRODUCT_CODE_BYTES);
    let be16 = |at: usize| u16::from_be_bytes([h[at], h[at + 1]]);
    let be32 = |at: usize| u32::from_be_bytes([h[at], h[at + 1], h[at + 2], h[at + 3]]);
    text > 0
        && code[..text].iter().all(|b| (0x20..=0x7e).contains(b))
        && code[text..].iter().all(|&b| b == 0)
        && (1990..=2100).contains(&be16(12))
        && (1..=12).contains(&h[14])
        && (1..=31).contains(&h[15])
        && u64::from(be32(20)) == len
        && h[24..].iter().all(|&b| b == 0)
}

pub fn tascamfw() -> Template {
    Template::new(
        "tascamfw",
        T::structure(
            "TascamUpdate",
            vec![
                ("header", header()),
                ("image", T::decoded(E::Remaining.sub(E::lit(CHECKSUM_BYTES)), Codec::TascamScramble, image())),
                ("checksum_copy", T::decoded(E::lit(CHECKSUM_BYTES), Codec::TascamScramble, checksum_copy())),
            ],
        )
        .field_doc(
            "image",
            "The firmware image, unscrambled. It is read as an i.MX RT flash image if it starts with the FlexSPI configuration block tag FCFB and has an image vector table (IVT) at offset 0x1000. It is read as a Blackfin boot stream if it starts with a block header that has the first flag (BFLAG_FIRST) set. Any other image is read as bytes.",
        ),
    )
}

/// The unscrambled image, read as what its first bytes say it is. See the
/// module's notes.
fn image() -> T {
    let kind = E::cond(imxrt_flash_ahead(), E::lit(IMXRT_FLASH), E::cond(bfldr::stream_ahead(), E::lit(BOOT_STREAM), E::lit(0)));
    T::switch(kind, vec![(IMXRT_FLASH, flash()), (BOOT_STREAM, bfldr::stream())], T::bytes(E::Remaining))
}

/// The test `imxrt::is_imxrt_flash` makes, as an expression asked of the bytes
/// ahead: `FCFB` at 0, and at 0x1000 an IVT header whose version is 0x40 or
/// 0x41. Clearing the low bit of the IVT header's last byte leaves 0x40 for
/// both versions.
fn imxrt_flash_ahead() -> E {
    let tag: [u8; 4] = imxrt::TAG.try_into().expect("a four-letter tag");
    let ivt = imxrt::IVT_AT as i128;
    E::Remaining
        .greater_than(E::lit(ivt + 3))
        .both(E::peek(32, Big).equal_to(E::lit(i128::from(u32::from_be_bytes(tag)))))
        .both(E::peek_at(E::lit(ivt * 8), 32, Big).and(E::lit(0xffff_fffe_i128)).equal_to(E::lit(0xd100_2040_i128)))
}

/// The 32 bytes in front of the image, all big-endian.
fn header() -> T {
    T::structure(
        "UpdateHeader",
        vec![
            ("product_code", T::text(StrLen::Padded { size: E::lit(PRODUCT_CODE_BYTES as i128), pad: 0 }, Encoding::Ascii)),
            // 203 is V2.03.
            ("version", T::u16(Big)),
            // The number in the file name: `#PortaX8_0217.203` is build 217.
            ("build", T::u16(Big)),
            ("year", T::u16(Big)),
            ("month", T::u8()),
            ("day", T::u8()),
            ("checksum", T::u32(Big)),
            ("file_size", T::u32(Big)),
            ("reserved", T::bytes(E::lit(8))),
        ],
    )
    .field_check(
        "checksum",
        Check::of(
            Checksum::Sum32,
            Covers::Unpacked { name: Named::here("image"), len: Some(E::field("file_size").sub(E::lit(HEADER_BYTES + CHECKSUM_BYTES))) },
        ),
    )
    .field_valid("file_size", Valid::Eq(E::SpaceSize))
    .field_doc(
        "product_code",
        "Tascam's code for the product this update is for, in ASCII, padded with NULs to 8 bytes. The Portacapture X8's code is DR-TURBO, Tascam's internal project name for the X8. The same name appears in the source file paths in the X8's firmware (C:\\Perforce\\projects\\SSR\\DR-TURBO\\...). The DR-05X's code is DR-05X.",
    )
}

/// The last four bytes of the image, which hold the header's checksum again.
fn checksum_copy() -> T {
    T::structure("ChecksumCopy", vec![("checksum", T::u32(Big))]).field_valid("checksum", Valid::Eq(E::within(&["header", "checksum"])))
}

/// The unscrambled flash of a Portacapture X8, with offsets counted from the
/// start of the chip.
fn flash() -> T {
    T::structure(
        "X8Flash",
        vec![
            ("boot_header", imxrt::boot_header()),
            ("padding", imxrt::padding_to_entry()),
            ("bootloader", bootloader()),
            ("application", application()),
        ],
    )
}

/// The program the boot ROM starts, which runs in place from the flash, sets
/// up the board and the display, and copies the application into SDRAM.
fn bootloader() -> T {
    const ENTRY: &[&str] = &["init_table", "packbits_init"];
    T::structure(
        "Bootloader",
        vec![
            ("vectors", imxrt::vectors()),
            ("code", imxrt::thumb(through_init_core())),
            ("init_table", T::structure("InitTable", vec![("zero_init", zero_init()), ("packbits_init", packed_init())])),
            // IAR's startup code, the interrupt stubs and the default handler.
            ("code_after_init_table", imxrt::thumb(source(ENTRY).sub(E::SpacePos))),
            // Board function pointers and the 1-bit splash screen the
            // bootloader shows while it copies.
            ("dtcm_data", packed_run(ENTRY, Codec::IarPackbits, ("data", T::bytes(E::Remaining)))),
            ("padding", T::bytes(E::lit(APP_AT).sub(E::SpacePos))),
        ],
    )
}

/// The recorder's program, as the bootloader copies it into SDRAM.
fn application() -> T {
    let entry = |name: &'static str| -> [&'static str; 2] { ["init_table", name] };
    T::structure(
        "Application",
        vec![
            ("vectors", imxrt::vectors()),
            ("version_stamp", version_stamp()),
            ("font_map", font_map()),
            ("sounds", T::repeat(sound(), Until::While(riff_ahead()))),
            // Code with the data it uses mixed in: see the module's notes.
            ("code", imxrt::thumb(through_init_core())),
            ("init_table", app_init_table()),
            ("code_after_init_table", imxrt::thumb(source(&entry("lz77_init_sdram")).sub(E::SpacePos))),
            // The three runs are written one after another, in the order the
            // table lists them.
            ("sdram_data", packed_run(&entry("lz77_init_sdram"), Codec::IarLz77, ("data", T::bytes(E::Remaining)))),
            ("itcm_code", packed_run(&entry("lz77_init_itcm"), Codec::IarLz77, ("code", imxrt::thumb(E::Remaining)))),
            ("dtcm_data", packed_run(&entry("lz77_init_dtcm"), Codec::IarLz77, ("data", T::bytes(E::Remaining)))),
            // Erased flash to the end of the image.
            ("padding", T::bytes(E::run(&[0xff]))),
        ],
    )
    .field_valid(
        "itcm_code",
        Valid::Expr {
            expr: E::start_of(E::field("itcm_code")).equal_to(source(&entry("lz77_init_itcm"))),
            msg: Some("Not where the init table's lz77_init_itcm entry points".into()),
        },
    )
    .field_valid(
        "dtcm_data",
        Valid::Expr {
            expr: E::start_of(E::field("dtcm_data")).equal_to(source(&entry("lz77_init_dtcm"))),
            msg: Some("Not where the init table's lz77_init_dtcm entry points".into()),
        },
    )
}

/// From here to the end of `__iar_init_core`, which is where the init table
/// starts.
fn through_init_core() -> E {
    E::to_bytes(INIT_CORE).add(E::lit(INIT_CORE.len() as i128))
}

/// `path` with `last` on the end.
fn path<'a>(path: &[&'a str], last: &'a str) -> Vec<&'a str> {
    path.iter().copied().chain([last]).collect()
}

/// The start of the packed run that an init table entry points to: the offset
/// in the entry's first source range, added to the position of that range.
fn source(entry: &[&str]) -> E {
    let sources = path(entry, "sources");
    E::start_of(E::elem_within(&sources, E::lit(0), &[])).add(E::elem_within(&sources, E::lit(0), &["offset"]))
}

/// The packed run an init table entry names, as long as its first source range
/// says, read as `body` once unpacked. What it unpacks to is checked against
/// the entry's first destination range.
fn packed_run(entry: &[&str], codec: Codec, body: (&str, T)) -> T {
    let first = |list: &str, field: &str| E::elem_within(&path(entry, list), E::lit(0), &[field]);
    T::decoded(
        first("sources", "length"),
        codec,
        T::structure("Unpacked", vec![("unpacked_length", T::computed(E::SpaceSize)), body])
            .field_valid("unpacked_length", Valid::Eq(first("destinations", "length"))),
    )
}

/// An init table entry that zeroes memory: the routine, then a size and an
/// address for each range, ended by a size of zero.
fn zero_init() -> T {
    let more = E::Remaining.greater_than(E::lit(3)).both(E::peek(32, Little).not_equal(E::lit(0)));
    T::structure(
        "ZeroInit",
        vec![
            ("routine", routine()),
            ("ranges", T::repeat(T::structure("ZeroRange", vec![("size", T::u32(Little)), ("destination", T::u32(Little))]), Until::While(more))),
            ("end_marker", T::u32(Little)),
        ],
    )
}

/// The first word of every init table entry: where the routine that handles
/// the entry is, counted from this word. The routine returns the address of
/// the next entry, so the routine is what decides how long this one is.
fn routine() -> T {
    T::i32(Little)
}

/// An init table entry that unpacks a run: how many source and destination
/// ranges there are, then the ranges. A source range's offset is counted from
/// the range's own first byte; a destination range is an address.
fn packed_init() -> T {
    let bits = |n: u32| T::UInt { bits: n, endian: Little };
    T::structure(
        "PackedInit",
        vec![
            ("routine", routine()),
            // The low bit's meaning for source ranges is not known. It is 0 in
            // every X8 entry.
            ("source_count", T::inline_structure("SourceCount", vec![("unknown_low_bit", bits(1)), ("count", bits(31))])),
            // The low bit says the addresses count from the static base
            // register rather than from zero.
            ("destination_count", T::inline_structure("DestinationCount", vec![("sb_relative", bits(1)), ("count", bits(31))])),
            (
                "sources",
                T::array(T::structure("SourceRange", vec![("offset", T::i32(Little)), ("length", T::u32(Little))]), E::within(&["source_count", "count"])),
            ),
            (
                "destinations",
                T::array(
                    T::structure("DestinationRange", vec![("address", T::u32(Little)), ("length", T::u32(Little))]),
                    E::within(&["destination_count", "count"]),
                ),
            ),
        ],
    )
}

/// The application's init table: zeroing, the three packed runs, and the
/// routine that runs the C++ constructors, which takes no parameters and is
/// the last entry.
fn app_init_table() -> T {
    T::structure(
        "InitTable",
        vec![
            ("zero_init", zero_init()),
            ("lz77_init_sdram", packed_init()),
            ("lz77_init_itcm", packed_init()),
            ("lz77_init_dtcm", packed_init()),
            ("call_ctors", T::structure("CallCtors", vec![("routine", routine())])),
        ],
    )
}

/// The version and build again, at 0x400 into the application. Both should be
/// the header's.
fn version_stamp() -> T {
    T::structure("VersionStamp", vec![("version", T::u16(Big)), ("build", T::u16(Big)), ("reserved", T::bytes(E::lit(12)))])
        .field_valid("version", Valid::Eq(E::within(&["header", "version"])))
        .field_valid("build", Valid::Eq(E::within(&["header", "build"])))
}

/// A Monotype Spark font map, which is how Qt for MCUs picks a font for a
/// character: ranges of code points, each naming the font that draws it by its
/// offset from the start of the map, and then the fonts. Big-endian, and as
/// long as it says eight bytes in.
///
/// Monotype does not publish the layout. The names here come from what the
/// X8's map holds and which of its values match each other. Words whose
/// meaning is unknown are named by their offset: three in the map header and
/// two in each range.
fn font_map() -> T {
    let names = |count: &str| T::array(T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Ascii), E::field(count));
    T::sized(
        E::peek_at(E::lit(64), 32, Big),
        T::structure(
            "FontMap",
            vec![
                ("magic", T::magic(b"FTMP")),
                ("version", T::u32(Big)),
                ("size", T::u32(Big)),
                ("font_count", T::u16(Big)),
                ("range_count", T::u16(Big)),
                ("language_count", T::u16(Big)),
                ("style_count", T::u16(Big)),
                ("languages_size", T::u32(Big)),
                ("styles_size", T::u32(Big)),
                ("unknown_1c", T::u32(Big)),
                ("unknown_20", T::u32(Big)),
                ("unknown_24", T::u32(Big)),
                ("name_size", T::u32(Big)),
                ("name", T::text(StrLen::Fixed(E::field("name_size")), Encoding::Ascii)),
                ("languages", names("language_count")),
                ("languages_padding", T::bytes(E::field("languages_size").sub(E::size_of("languages")))),
                ("styles", names("style_count")),
                ("styles_padding", T::bytes(E::field("styles_size").sub(E::size_of("styles")))),
                ("ranges", T::array(font_range(), E::field("range_count"))),
                ("fonts", T::array(font(), E::field("font_count"))),
            ],
        ),
    )
}

/// One range of code points and the font that draws them. Every range names
/// its font by offset and size, and also by its number in the map, which is
/// what the fonts are found by.
fn font_range() -> T {
    T::structure(
        "FontRange",
        vec![
            ("first_code_point", T::u32(Big)),
            ("last_code_point", T::u32(Big)),
            ("font_offset", T::u32(Big)),
            ("font_length", T::u32(Big)),
            ("font_index", T::u32(Big)),
            ("unknown_14", T::u32(Big)),
            ("unknown_18", T::u32(Big)),
        ],
    )
}

/// One of the map's fonts: where the first range that names it says, and as
/// long as that range says. A TrueType file, which can be opened as a
/// document of its own.
fn font() -> T {
    let range = |field: &str| E::tagged_by_expr("ranges", &["font_index"], E::idx(), &[field]);
    T::structure(
        "Font",
        vec![
            // Any bytes between the end of the previous field and the offset
            // this font's range gives. There are none in the X8's map: the two
            // zero bytes after the first font are counted in its length.
            ("padding", T::bytes(range("font_offset").sub(E::Pos))),
            ("file", T::decoded(range("font_length"), Codec::Stored, super::decoded_object())),
        ],
    )
}

/// Whether the bytes ahead open a WAV file.
fn riff_ahead() -> E {
    E::Remaining.greater_than(E::lit(7)).both(E::peek(32, Big).equal_to(E::lit(RIFF)))
}

/// One WAV file, as long as its RIFF header says, and the zero bytes after it
/// up to a four-byte boundary.
fn sound() -> T {
    T::structure(
        "Sound",
        vec![
            ("wav", T::decoded(E::peek_at(E::lit(32), 32, Little).add(E::lit(8)), Codec::Stored, super::decoded_object())),
            ("padding", T::bytes(E::size_of("wav").pad_to(4))),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::tascam::UNSCRAMBLE;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::formats::imxrt::tests::{boot_header, dcd_with_a_stray, XIP};
    use crate::source::MemSource;

    /// `push {r3-r7, lr}` and `movs r4, #0`: something for a code run to hold.
    const CODE: &[u8] = &[0xf8, 0xb5, 0x00, 0x24];

    fn le(v: &mut Vec<u8>, words: &[u32]) {
        for w in words {
            v.extend_from_slice(&w.to_le_bytes());
        }
    }

    fn be(v: &mut Vec<u8>, words: &[u32]) {
        for w in words {
            v.extend_from_slice(&w.to_be_bytes());
        }
    }

    /// A vector table: a stack pointer, a reset handler, and one default
    /// handler in every other slot.
    fn vectors(v: &mut Vec<u8>, stack: u32, reset: u32, default: u32) {
        let end = v.len() + 1024;
        le(v, &[stack, reset]);
        while v.len() < end {
            le(v, &[default]);
        }
    }

    /// Bytes as one `lz77` token of literals and nothing else.
    fn lz77_literals(bytes: &[u8]) -> Vec<u8> {
        assert!((2..=257).contains(&bytes.len()));
        let mut v = vec![0x00, (bytes.len() - 2) as u8];
        v.extend_from_slice(bytes);
        v
    }

    /// An init table entry that unpacks one run, with its source offset left
    /// for `patch` to fill in. Returns where that offset is.
    fn packed_entry(v: &mut Vec<u8>, packed: usize, address: u32, unpacked: usize) -> usize {
        le(v, &[0xffc2_b033, 2, 2]);
        let at = v.len();
        le(v, &[0, packed as u32, address, unpacked as u32]);
        at
    }

    /// Point the source range at `at` to `run`, counted from the range.
    fn patch(v: &mut [u8], at: usize, run: usize) {
        v[at..at + 4].copy_from_slice(&((run as i32 - at as i32) as u32).to_le_bytes());
    }

    /// A small font map: two fonts, three ranges naming them, and two bytes
    /// of padding between the fonts.
    fn font_map() -> Vec<u8> {
        let fonts: [&[u8]; 2] = [b"\x00\x01\x00\x00ab", b"\x00\x01\x00\x00wxyz"];
        let head = 0x2c + 8 + 8 + 8 + 3 * 28;
        let second = head + fonts[0].len() + 2;
        let size = second + fonts[1].len();
        let mut f = b"FTMP".to_vec();
        be(&mut f, &[0x10000, size as u32]);
        f.extend_from_slice(&[0, 2, 0, 3, 0, 2, 0, 1]);
        be(&mut f, &[8, 8, 0x128f6, 0x49ba, 0, 8]);
        f.extend_from_slice(b"Fontmap1ja\0ko\0\0\0regular\0");
        be(&mut f, &[0, 0x9fff, head as u32, fonts[0].len() as u32, 0, 0, 0]);
        be(&mut f, &[0xf900, 0xffff, head as u32, fonts[0].len() as u32, 0, 0, 0]);
        be(&mut f, &[0x2b0, 0x3ff, second as u32, fonts[1].len() as u32, 1, 1, 0]);
        assert_eq!(f.len(), head);
        f.extend_from_slice(fonts[0]);
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(fonts[1]);
        f
    }

    /// An X8-shaped flash: the boot header, a bootloader with its init table
    /// and packbits run, and an application with a version stamp, a font map,
    /// two WAVs, an init table and three lz77 runs.
    fn flash() -> Vec<u8> {
        let mut v = boot_header(&dcd_with_a_stray());
        vectors(&mut v, 0x2008_0000, XIP + 0xa0d5, XIP + 0xa3f1);
        v.extend_from_slice(CODE);
        v.extend_from_slice(INIT_CORE);
        le(&mut v, &[0xffff_97ef, 0x388, 0x2000_5148, 0xe1000, 0x8660_0000, 0]);
        // Copy four bytes, then write 0xaa four times.
        let packbits = [0x03, 1, 2, 3, 4, 0xfc, 0xaa];
        let boot_source = packed_entry(&mut v, packbits.len(), 0x2000_0000, 8);
        v.extend_from_slice(CODE);
        let run = v.len();
        patch(&mut v, boot_source, run);
        v.extend_from_slice(&packbits);
        v.resize(APP_AT as usize, 0xff);

        vectors(&mut v, 0x2001_1cc8, 0x8173_206d, 0x8180_72e5);
        v.extend_from_slice(&[0x00, 0xcb, 0x00, 0xd9]);
        v.extend_from_slice(&[0; 12]);
        v.extend_from_slice(&font_map());
        // Two WAVs, the first padded to a four-byte boundary.
        v.extend_from_slice(b"RIFF\x06\x00\x00\x00WAVEab\x00\x00");
        v.extend_from_slice(b"RIFF\x08\x00\x00\x00WAVEabcd");
        v.extend_from_slice(CODE);
        v.extend_from_slice(INIT_CORE);
        le(&mut v, &[0xff5c_b6df, 0x10, 0x2001_1cc8, 0]);
        let runs = [lz77_literals(b"sdram!"), lz77_literals(CODE), lz77_literals(b"tc")];
        let unpacked = [6, 4, 2];
        let sources: Vec<usize> = [0x8190_0000, 0x400, 0x2000_0000]
            .iter()
            .zip(runs.iter().zip(unpacked))
            .map(|(&address, (run, n))| packed_entry(&mut v, run.len(), address, n))
            .collect();
        le(&mut v, &[0x0005_ca65]);
        v.extend_from_slice(CODE);
        for (at, run) in sources.iter().zip(&runs) {
            let here = v.len();
            patch(&mut v, *at, here);
            v.extend_from_slice(run);
        }
        v.extend_from_slice(&[0xff; 16]);
        v
    }

    /// The X8's product code.
    const DR_TURBO: &[u8; 8] = b"DR-TURBO";

    /// The update file for that flash: the header, then the flash and its sum
    /// scrambled. The header gives the X8's version, build and date, which
    /// the X8's version stamp is checked against.
    fn update(flash: &[u8]) -> Vec<u8> {
        update_for(DR_TURBO, flash)
    }

    /// The same, for any product code and image.
    fn update_for(code: &[u8; 8], flash: &[u8]) -> Vec<u8> {
        let sum = flash.iter().fold(0u32, |a, &b| a.wrapping_add(u32::from(b)));
        let mut scramble = [0u8; 256];
        for (b, &plain) in UNSCRAMBLE.iter().enumerate() {
            scramble[plain as usize] = b as u8;
        }
        let mut v = code.to_vec();
        v.extend_from_slice(&[0x00, 0xcb, 0x00, 0xd9, 0x07, 0xe9, 8, 19]);
        be(&mut v, &[sum, (HEADER_BYTES as usize + flash.len() + 4) as u32, 0, 0]);
        v.extend(flash.iter().chain(&sum.to_be_bytes()).map(|&b| scramble[b as usize]));
        v
    }

    /// The node a path of names reaches. `0` steps into what a packed run
    /// unpacks to, which has no name of its own to be found by.
    fn named(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> Vec<usize> {
        let mut p = path.to_vec();
        for name in names {
            if *name == "0" {
                p.push(0);
                continue;
            }
            p = ev.child_named(d, &p, name).unwrap().unwrap_or_else(|| panic!("no {name} under {p:?}"));
        }
        p
    }

    fn int(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> i128 {
        let p = named(ev, d, path, names);
        let v = ev.node(d, &p).unwrap().value;
        v.as_int().unwrap_or_else(|| panic!("{names:?} is not a number: {v:?}"))
    }

    fn holds(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> bool {
        let p = named(ev, d, path, names);
        let verdict = ev.valid_of(d, &p).unwrap().unwrap_or_else(|| panic!("{names:?} has no constraint"));
        verdict.ok
    }

    /// Offset and size in bytes, in whichever space the node is read in.
    fn extent(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> (u64, u64) {
        let p = named(ev, d, path, names);
        let n = ev.node(d, &p).unwrap();
        (n.offset_bits / 8, n.size_bits / 8)
    }

    /// The unpacked flash: the image run, and the one thing it unpacks to.
    const FLASH: [usize; 2] = [1, 0];

    #[test]
    fn the_header_seals_the_image_and_the_last_word_repeats_the_sum() {
        let d = Document::new(MemSource(update(&flash())));
        let mut ev = Evaluator::new(tascamfw());
        assert_eq!(int(&mut ev, &d, &[0], &["version"]), 203);
        assert_eq!(int(&mut ev, &d, &[0], &["build"]), 217);
        assert_eq!(int(&mut ev, &d, &[0], &["year"]), 2025);
        assert!(holds(&mut ev, &d, &[0], &["file_size"]));
        let sum = named(&mut ev, &d, &[0], &["checksum"]);
        let verdict = ev.run_check(&d, &sum).unwrap().expect("a check");
        assert!(verdict.ok, "computed {}, stored {}", verdict.computed, verdict.stored);
        // The copy is in a run of its own, and is checked against the header
        // from inside it.
        assert!(holds(&mut ev, &d, &[2, 0], &["checksum"]));
        assert_eq!(ev.node(&d, &FLASH).unwrap().type_name, "X8Flash");
        let code = named(&mut ev, &d, &[0], &["product_code"]);
        assert!(matches!(&ev.node(&d, &code).unwrap().value, Value::Str(s) if s == "DR-TURBO"), "{:?}", ev.node(&d, &code).unwrap().value);
    }

    /// The first 32 bytes of the two products' updates in the sample
    /// collection, `#PortaX8_0217.203` and `DR05X_071.116`, and the lengths of
    /// those files.
    const X8_HEAD: [u8; 32] = *b"DR-TURBO\x00\xcb\x00\xd9\x07\xe9\x08\x13\x7b\x86\x74\x1b\x01\x85\x00\x20\0\0\0\0\0\0\0\0";
    const X8_LEN: u64 = 25_493_536;
    const DR05X_HEAD: [u8; 32] = *b"DR-05X\0\0\x00\x74\x00\x47\x07\xe7\x0c\x0f\x0a\xdd\x23\x53\x00\x1f\x30\x20\0\0\0\0\0\0\0\0";
    const DR05X_LEN: u64 = 2_043_936;

    #[test]
    fn the_probe_takes_both_products_headers_and_refuses_near_misses() {
        assert!(is_update(&X8_HEAD, X8_LEN));
        assert!(is_update(&DR05X_HEAD, DR05X_LEN));
        assert_eq!(crate::formats::sniff(&X8_HEAD, X8_LEN), Some("tascamfw"));
        assert_eq!(crate::formats::sniff(&DR05X_HEAD, DR05X_LEN), Some("tascamfw"));
        assert!(!is_update(&DR05X_HEAD[..31], DR05X_LEN), "too short for a header");
        assert!(!is_update(&DR05X_HEAD, DR05X_LEN + 1), "the length at byte 20 is not the file's");
        assert!(!is_update(&X8_HEAD, DR05X_LEN));
        let changed = |at: usize, b: u8| {
            let mut h = DR05X_HEAD;
            h[at] = b;
            h
        };
        for (at, b, why) in [
            (31, 1, "a reserved byte is not zero"),
            (24, 0x80, "a reserved byte is not zero"),
            (2, 0xb0, "a product code byte is not ASCII"),
            (0, 0x1b, "a product code byte is a control character"),
            (0, 0, "the product code is empty"),
            (7, b'A', "a letter after the NUL padding"),
            (14, 13, "month 13"),
            (14, 0, "month 0"),
            (15, 32, "day 32"),
            (12, 0x05, "the year 1511"),
        ] {
            assert!(!is_update(&changed(at, b), DR05X_LEN), "{why}");
        }
    }

    /// What `test` answers at the start of `bytes`, asked the way a template
    /// asks it: whether a field that is there only when it is nonzero is
    /// there.
    fn ahead(test: E, bytes: &[u8]) -> bool {
        let probe = Template::new("probe", T::structure("Probe", vec![("image", T::when(test, T::bytes(E::Remaining)))]));
        let d = Document::new(MemSource(bytes.to_vec()));
        let mut ev = Evaluator::new(probe);
        !ev.node(&d, &[0]).unwrap().absent
    }

    #[test]
    fn the_image_tests_agree_with_the_probes_they_copy() {
        let stream = crate::formats::bfldr::tests::two_programs();
        let flash = flash();
        let mut old_ivt = flash.clone();
        old_ivt[imxrt::IVT_AT + 3] = 0x40;
        let mut no_ivt = flash.clone();
        no_ivt[imxrt::IVT_AT] = 0xd2;
        let mut no_tag = flash.clone();
        no_tag[0] = b'X';
        let images = [flash.clone(), old_ivt, no_ivt, no_tag, flash[..imxrt::IVT_AT + 3].to_vec(), stream.clone(), vec![0xff; 64], Vec::new()];
        for (i, image) in images.iter().enumerate() {
            assert_eq!(ahead(imxrt_flash_ahead(), image), imxrt::is_imxrt_flash(image), "image {i}");
            assert_eq!(ahead(bfldr::stream_ahead(), image), bfldr::is_bfldr(image), "image {i}");
        }
        assert!(ahead(imxrt_flash_ahead(), &flash));
        assert!(ahead(bfldr::stream_ahead(), &stream));
    }

    #[test]
    fn an_image_that_is_a_boot_stream_is_read_with_bfldr() {
        let stream = crate::formats::bfldr::tests::two_programs();
        let d = Document::new(MemSource(update_for(b"DR-05X\0\0", &stream)));
        let mut ev = Evaluator::new(tascamfw());
        let code = named(&mut ev, &d, &[0], &["product_code"]);
        assert!(matches!(&ev.node(&d, &code).unwrap().value, Value::Str(s) if s == "DR-05X"), "{:?}", ev.node(&d, &code).unwrap().value);
        let sum = named(&mut ev, &d, &[0], &["checksum"]);
        assert!(ev.run_check(&d, &sum).unwrap().expect("a check").ok);
        assert!(holds(&mut ev, &d, &[2, 0], &["checksum"]));
        let image = ev.node(&d, &FLASH).unwrap();
        assert_eq!((image.type_name.as_str(), image.size_bits / 8), ("BootStream", stream.len() as u64));
        let programs = named(&mut ev, &d, &FLASH, &["programs"]);
        assert_eq!(ev.node(&d, &programs).unwrap().child_count, 2);
        assert_eq!(extent(&mut ev, &d, &FLASH, &["erased"]), (stream.len() as u64 - 12, 12));
    }

    #[test]
    fn an_image_of_another_kind_is_read_as_bytes() {
        // FCFB with no IVT after it, and the header of a block that is not a
        // first block.
        let images: [&[u8]; 3] = [&[0x55; 64], b"FCFB and nothing else", &crate::formats::bfldr::tests::block(0x8001, 0xffa0_0000, 0, 0, &[])];
        for image in images {
            let d = Document::new(MemSource(update_for(b"DR-99\0\0\0", image)));
            let mut ev = Evaluator::new(tascamfw());
            let node = ev.node(&d, &FLASH).unwrap();
            assert!(matches!(node.value, Value::Bytes { .. }), "{:?}", node.value);
            assert_eq!(node.size_bits / 8, image.len() as u64);
            assert_eq!(node.problem, None);
            let sum = named(&mut ev, &d, &[0], &["checksum"]);
            assert!(ev.run_check(&d, &sum).unwrap().expect("a check").ok);
        }
    }

    #[test]
    fn a_changed_byte_fails_the_checksum() {
        let mut v = update(&flash());
        v[0x40] ^= 1;
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(tascamfw());
        let sum = named(&mut ev, &d, &[0], &["checksum"]);
        assert!(!ev.run_check(&d, &sum).unwrap().expect("a check").ok);
    }

    #[test]
    fn the_unpacked_image_starts_with_an_imxrt_boot_header() {
        let d = Document::new(MemSource(update(&flash())));
        let mut ev = Evaluator::new(tascamfw());
        let tag = named(&mut ev, &d, &FLASH, &["boot_header", "flash_config", "mem_config", "tag"]);
        assert!(matches!(ev.node(&d, &tag).unwrap().value, Value::Magic { ok: true, .. }), "{:?}", ev.node(&d, &tag).unwrap().value);
        assert_eq!(int(&mut ev, &d, &FLASH, &["boot_header", "ivt", "entry"]), (XIP + 0x2000) as i128);
        assert!(holds(&mut ev, &d, &FLASH, &["boot_header", "dcd"]));
        assert_eq!(extent(&mut ev, &d, &FLASH, &["boot_header", "dcd", "rest_from_unknown_tag"]), (0x1054, 8));
        assert_eq!(extent(&mut ev, &d, &FLASH, &["padding"]).1, 0x2000 - 0x105c);
    }

    #[test]
    fn the_bootloader_is_read_up_to_its_init_table_and_its_packed_run_unpacks() {
        let d = Document::new(MemSource(update(&flash())));
        let mut ev = Evaluator::new(tascamfw());
        let boot = named(&mut ev, &d, &FLASH, &["bootloader"]);
        assert_eq!(int(&mut ev, &d, &boot, &["vectors", "reset"]), (XIP + 0xa0d5) as i128);
        let table_at = 0x2400 + CODE.len() + INIT_CORE.len();
        assert_eq!(extent(&mut ev, &d, &boot, &["code"]), (0x2400, (CODE.len() + INIT_CORE.len()) as u64));
        assert_eq!(extent(&mut ev, &d, &boot, &["init_table"]), (table_at as u64, 0x34));
        let ranges = named(&mut ev, &d, &boot, &["init_table", "zero_init", "ranges"]);
        assert_eq!(ev.node(&d, &ranges).unwrap().child_count, 2);
        assert_eq!(int(&mut ev, &d, &boot, &["init_table", "packbits_init", "destination_count", "count"]), 1);
        assert_eq!(extent(&mut ev, &d, &boot, &["code_after_init_table"]).1, CODE.len() as u64);
        assert_eq!(int(&mut ev, &d, &boot, &["dtcm_data", "0", "unpacked_length"]), 8);
        assert!(holds(&mut ev, &d, &boot, &["dtcm_data", "0", "unpacked_length"]));
        let padding = extent(&mut ev, &d, &boot, &["padding"]);
        assert_eq!(padding.0 + padding.1, APP_AT as u64);
    }

    #[test]
    fn the_application_holds_fonts_sounds_and_three_packed_runs() {
        let flash = flash();
        let d = Document::new(MemSource(update(&flash)));
        let mut ev = Evaluator::new(tascamfw());
        let app = named(&mut ev, &d, &FLASH, &["application"]);
        assert_eq!(int(&mut ev, &d, &app, &["vectors", "reset"]), 0x8173_206d);
        assert!(holds(&mut ev, &d, &app, &["version_stamp", "version"]));
        assert!(holds(&mut ev, &d, &app, &["version_stamp", "build"]));

        // The second font is placed by its offset, past two bytes of padding.
        let map = named(&mut ev, &d, &app, &["font_map"]);
        let map_at = ev.node(&d, &map).unwrap().offset_bits / 8;
        let fonts = named(&mut ev, &d, &map, &["fonts"]);
        assert_eq!(ev.node(&d, &fonts).unwrap().child_count, 2);
        let second = [&fonts[..], &[1]].concat();
        assert_eq!(extent(&mut ev, &d, &second, &["padding"]).1, 2);
        let (at, size) = extent(&mut ev, &d, &second, &["file"]);
        assert_eq!((at - map_at, size), (0x98 + 6 + 2, 8));
        let file = named(&mut ev, &d, &second, &["file"]);
        assert!(ev.node(&d, &file).unwrap().decoded);

        let sounds = named(&mut ev, &d, &app, &["sounds"]);
        assert_eq!(ev.node(&d, &sounds).unwrap().child_count, 2);
        assert_eq!(extent(&mut ev, &d, &[&sounds[..], &[0]].concat(), &["wav"]).1, 14);
        assert_eq!(extent(&mut ev, &d, &[&sounds[..], &[0]].concat(), &["padding"]).1, 2);
        assert_eq!(extent(&mut ev, &d, &[&sounds[..], &[1]].concat(), &["wav"]).1, 16);

        // The code before the init table is the two instructions of `CODE`
        // and the eleven of `__iar_init_core`, and the code after it is
        // `CODE` again.
        let code = named(&mut ev, &d, &app, &["code"]);
        assert_eq!(ev.node(&d, &code).unwrap().child_count, 13);
        let first = ev.node(&d, &[&code[..], &[0]].concat()).unwrap();
        assert!(matches!(&first.value, Value::Str(text) if text.starts_with("push")), "{:?}", first.value);
        let after = named(&mut ev, &d, &app, &["code_after_init_table"]);
        assert_eq!(ev.node(&d, &after).unwrap().child_count, 2);
        assert_eq!(extent(&mut ev, &d, &app, &["init_table"]).1, 0x10 + 3 * 0x1c + 4);
        for (name, n) in [("sdram_data", 6), ("itcm_code", 4), ("dtcm_data", 2)] {
            assert_eq!(int(&mut ev, &d, &app, &[name, "0", "unpacked_length"]), n, "{name}");
            assert!(holds(&mut ev, &d, &app, &[name, "0", "unpacked_length"]), "{name}");
        }
        assert!(holds(&mut ev, &d, &app, &["itcm_code"]));
        assert!(holds(&mut ev, &d, &app, &["dtcm_data"]));
        let code = named(&mut ev, &d, &app, &["itcm_code", "0", "code"]);
        let first = ev.node(&d, &[&code[..], &[0]].concat()).unwrap();
        let Value::Str(text) = first.value else { panic!("not an instruction: {:?}", first.value) };
        assert!(text.starts_with("push"), "{text}");
        let padding = extent(&mut ev, &d, &app, &["padding"]);
        assert_eq!(padding, (flash.len() as u64 - 16, 16));
    }
}
