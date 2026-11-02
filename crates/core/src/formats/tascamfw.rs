//! Tascam Portacapture X8 firmware updates: the `#PortaX8_0217.203` a user
//! copies to the recorder's card to update it.
//!
//! The file is a 32-byte header and then an image of the recorder's whole
//! flash chip, with every byte of the image passed through the fixed table
//! Tascam scrambles its updates with (see [`crate::codec::tascam`]). The header
//! names the recorder by its project code, `DR-TURBO`, gives the version, the
//! build and the date of the build, and ends with a checksum and the length of
//! the file. The checksum is the sum of the image's bytes once the table is
//! undone, kept to 32 bits, and the last four bytes of the image are the same
//! number again, scrambled like the rest. So the image is read as two runs:
//! everything up to those four bytes, which the header's checksum covers, and
//! the four bytes themselves.
//!
//! The image is the flash an NXP i.MX RT1060 boots from, and its first 8 KiB
//! are read with the pieces `imxrt` reads such a flash with: the FlexSPI
//! configuration block, the IVT, the boot data and the DCD. The IVT's entry
//! points at Tascam's bootloader, and the application is at 0x20000. The
//! bootloader copies everything from 0x20000 on into SDRAM at 0x80000000 and
//! jumps to it without checking anything, so an application address is its
//! flash offset plus 0x7ffe0000.
//!
//! Both programs were built with IAR Embedded Workbench and keep their startup
//! data the way IAR's linker does: an init table of routines and the ranges
//! each one fills, and the packed runs those routines unpack. Nothing marks
//! where a table is, but IAR's linker puts it straight after the 36-byte
//! routine that turns on the floating-point unit, `__iar_init_core`. So each
//! program's code is read up to the end of that routine, wherever it turns
//! up, and the init table after it. The bootloader's table zeroes two ranges
//! and unpacks one `packbits` run into the DTCM. The application's zeroes
//! seven, unpacks three `lz77` runs into SDRAM, the ITCM and the DTCM, and
//! then runs the C++ constructors. Each run is read where its table entry
//! points, and what it unpacks to is checked against the length the entry
//! gives for it.
//!
//! The application opens with its vector table, a stamp of the version and
//! build that should agree with the header, and then data its code is linked
//! around: a Qt for MCUs font map with the three TrueType fonts it lists, and
//! four WAV files, sounds the recorder plays. Each font and each WAV is a
//! file of its own and opens as one. Two more WAVs and several hundred images
//! are inside the code, where only the code says where they are, and are left
//! as part of it.
//!
//! The bootloader's code and the code that runs from the ITCM are read as
//! Thumb instructions. The application's own code is not: its two runs, 11 MB
//! before the init table and half a megabyte after it, are read as bytes.
//! They are code mixed with string pools, vtables and images, so much of what
//! a decoder made of them would be instructions nobody wrote, and a run of
//! instructions is only counted by decoding every one of them: the 11 MB run
//! took over a minute to count in a debug build, and a listing counts a run
//! before it can draw the row that holds it.
//!
//! The ITCM code is linked to run at 0x400 and the IR has no way to say so, so
//! its branch targets read as distances from the instruction rather than as
//! addresses.
//!
//! The image's layout is this recorder's. Another product's update with the
//! same header would need its own reading of the image, which is why the
//! product code is a magic number here rather than a field that may hold
//! anything.

use crate::codec::Codec;
use crate::template::{Check, Checksum, Covers, Encoding, Endian::*, Expr as E, Named, StrLen, Template, Ty as T, Until, Valid};

use super::imxrt;

/// The X8's project code at TEAC, which opens every update file for it and is
/// what identifies one.
pub const MAGIC: &[u8] = b"DR-TURBO";

/// Bytes of header before the scrambled image.
const HEADER_BYTES: i128 = 0x20;

/// The copy of the checksum that ends the image.
const CHECKSUM_BYTES: i128 = 4;

/// IAR's `__iar_init_core`, the routine that turns on the floating-point unit,
/// and the two bytes that pad it to a word. IAR's linker puts the init table
/// straight after it, in both programs and both releases, and nowhere else in
/// either image are these 36 bytes written.
const INIT_CORE: &[u8] = &[
    0x4e, 0xf6, 0x88, 0x51, 0xce, 0xf2, 0x00, 0x01, 0x08, 0x68, 0x40, 0xf4, 0x70, 0x00, 0x08, 0x60, 0xbf, 0xf3,
    0x4f, 0x8f, 0xbf, 0xf3, 0x6f, 0x8f, 0x4f, 0xf0, 0x00, 0x70, 0xe1, 0xee, 0x10, 0x0a, 0x70, 0x47, 0x00, 0x00,
];

/// Where the application starts in the flash: what the bootloader copies from.
const APP_AT: i128 = 0x20000;

/// `RIFF`, read as a big-endian number, which opens each WAV file.
const RIFF: i128 = 0x5249_4646;

pub fn tascamfw() -> Template {
    Template::new(
        "tascamfw",
        T::structure(
            "TascamUpdate",
            vec![
                ("header", header()),
                ("image", T::decoded(E::Remaining.sub(E::lit(CHECKSUM_BYTES)), Codec::TascamScramble, flash())),
                ("checksum_copy", T::decoded(E::lit(CHECKSUM_BYTES), Codec::TascamScramble, checksum_copy())),
            ],
        ),
    )
}

/// The 32 bytes in front of the image, all big-endian.
fn header() -> T {
    T::structure(
        "UpdateHeader",
        vec![
            ("product_code", T::magic(MAGIC)),
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
}

/// The last four bytes of the image, which hold the header's checksum again.
fn checksum_copy() -> T {
    T::structure("ChecksumCopy", vec![("checksum", T::u32(Big))]).field_valid("checksum", Valid::Eq(E::within(&["header", "checksum"])))
}

/// The unscrambled flash, with offsets counted from the start of the chip.
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
            // Code and data mixed, as bytes: see the module's notes for why
            // these two are not read as instructions.
            ("code", T::bytes(through_init_core())),
            ("init_table", app_init_table()),
            ("code_after_init_table", T::bytes(source(&entry("lz77_init_sdram")).sub(E::SpacePos))),
            // The three runs are written one after another, in the order the
            // table lists them.
            ("sdram_data", packed_run(&entry("lz77_init_sdram"), Codec::IarLz77, ("data", T::bytes(E::Remaining)))),
            ("itcm_code", packed_run(&entry("lz77_init_itcm"), Codec::IarLz77, ("code", imxrt::thumb(E::Remaining)))),
            ("dtcm_data", packed_run(&entry("lz77_init_dtcm"), Codec::IarLz77, ("data", T::bytes(E::Remaining)))),
            // Erased flash to the end of the image.
            ("padding", T::bytes(E::run(&[0xff]))),
        ],
    )
    .field_valid("itcm_code", Valid::Expr { expr: E::start_of(E::field("itcm_code")).equal_to(source(&entry("lz77_init_itcm"))), msg: None })
    .field_valid("dtcm_data", Valid::Expr { expr: E::start_of(E::field("dtcm_data")).equal_to(source(&entry("lz77_init_dtcm"))), msg: None })
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

/// Where the packed run an init table entry names starts: its first source
/// range holds an offset counted from that range's own first byte.
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
            ("end", T::u32(Little)),
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
            ("source_count", T::inline_structure("SourceCount", vec![("flag", bits(1)), ("count", bits(31))])),
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
/// Monotype does not publish the layout. What is named here is what the X8's
/// map holds and how its numbers line up; the three words nothing lines up
/// with are named by their offset.
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
            ("font_size", T::u32(Big)),
            ("font_index", T::u32(Big)),
            ("unknown_14", T::u32(Big)),
            ("unknown_18", T::u32(Big)),
        ],
    )
}

/// One of the map's fonts: where the first range that names it says, and as
/// long as that range says. A TrueType file, which opens as one.
fn font() -> T {
    let range = |field: &str| E::tagged_by_expr("ranges", &["font_index"], E::idx(), &[field]);
    T::structure(
        "Font",
        vec![
            // Whatever lies between the end of the last thing read and this
            // font. Two bytes before the second font in the X8's map, and
            // nothing before the others.
            ("padding", T::bytes(range("font_offset").sub(E::Pos))),
            ("file", T::decoded(range("font_size"), Codec::Stored, super::decoded_object())),
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

    /// The update file for that flash: the header, then the flash and its sum
    /// scrambled.
    fn update(flash: &[u8]) -> Vec<u8> {
        let sum = flash.iter().fold(0u32, |a, &b| a.wrapping_add(u32::from(b)));
        let mut scramble = [0u8; 256];
        for (b, &plain) in UNSCRAMBLE.iter().enumerate() {
            scramble[plain as usize] = b as u8;
        }
        let mut v = MAGIC.to_vec();
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
        assert_eq!(extent(&mut ev, &d, &FLASH, &["boot_header", "dcd", "not_a_known_command"]), (0x1054, 8));
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
