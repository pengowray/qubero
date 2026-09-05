//! A PICO-8 cartridge, which is a picture of a cartridge with the cartridge
//! hidden inside it.
//!
//! A `.p8.png` is an ordinary PNG: 160 by 205, eight bits a channel, RGBA,
//! not interlaced, and it draws the label art anybody would expect a cart to
//! have. The program is in the low two bits of every channel of every pixel,
//! which the picture can spare. One pixel carries one byte: alpha holds bits 7
//! and 6, red 5 and 4, green 3 and 2, blue 1 and 0. 160 times 205 is 32,800
//! bytes, of which the cart is the first 0x8001 and the rest is slack.
//! See <https://pico-8.fandom.com/wiki/P8PNGFileFormat>.
//!
//! Getting at them is four steps, and this template shows all four rather than
//! the answer: the IDAT chunk is a zlib stream, what comes out of that is PNG
//! scanlines with a filter byte in front of each, what comes out of undoing
//! the filters is pixels, and what comes out of the pixels' low bits is the
//! cart. Each step is a space of its own under the one before it.
//!
//! The cart itself is a picture of PICO-8's memory, in the order the manual's
//! memory map gives: sprite sheet at 0x0000, the half of it the map shares at
//! 0x1000, the map at 0x2000, sprite flags at 0x3000, music at 0x3100, sound
//! effects at 0x3200, the Lua at 0x4300, and one byte at 0x8000 saying which
//! release of PICO-8 wrote the file.
//!
//! ## The code
//!
//! The Lua at 0x4300 is stored one of three ways, told apart by the first four
//! bytes:
//!
//! - `\0pxa`, from PICO-8 0.2.0 on. Two big-endian 16-bit lengths follow, the
//!   text's length and the whole run's including these eight bytes, and then a
//!   bit stream. A literal is a position in a 256-entry table, written as a
//!   unary prefix and then that many bits, and the byte it names moves to the
//!   front of the table. A back-reference writes its offset in 5, 10 or 15
//!   bits and its length as a chain of three-bit groups.
//! - `:c:\0`, from before that. A big-endian 16-bit length of the text, two
//!   bytes that say nothing, and then a byte stream: a byte of zero escapes a
//!   literal, a byte up to 0x3b indexes a table of the characters Lua source
//!   is mostly made of, and anything higher is a back-reference into the last
//!   4K of output.
//! - Anything else, in which case the region is the Lua source as it stands,
//!   ending at the first zero byte.
//!
//! The first two are opened, so the code reads as its text either way, with
//! every literal and every back-reference named. All three read as P8SCII
//! rather than as ASCII or UTF-8: a cart writes the button glyphs and the
//! syllabaries straight into its source, and every byte above 0x7f is one of
//! them. See [`crate::text::CodePage::P8scii`]. Neither scheme is written
//! down anywhere Lexaloffle publishes; see [`crate::codec::pico8`] for what
//! the decoders were written against.

use crate::codec::Codec;
use crate::template::{Encoding, Endian::Big, Expr as E, StrLen, Template, Ty as T};

/// A cart image, and the bytes one row of it comes to. Four channels a pixel.
const WIDTH: u32 = 160;
const HEIGHT: u32 = 205;

/// Where the Lua sits and how much room it has: 0x4300 up to 0x8000.
const CODE_LEN: i128 = 0x3d00;

/// The bytes a cart is laid out in, straight from PICO-8's memory map.
fn cart() -> T {
    T::structure(
        "Cart",
        vec![
            // The sprite sheet, 128 by 128 pixels at a nibble each. The second
            // half of it is the same memory the top half of the map uses, so a
            // cart that wants a big map draws with fewer sprites.
            ("gfx", T::bytes(E::lit(0x1000))),
            ("gfx_map_shared", T::bytes(E::lit(0x1000))),
            ("map", T::bytes(E::lit(0x1000))),
            // One byte of eight flags per sprite, then 64 songs of four bytes,
            // then 64 sound effects of 68.
            ("gfx_flags", T::bytes(E::lit(0x100))),
            ("music", T::bytes(E::lit(0x100))),
            ("sfx", T::bytes(E::lit(0x1100))),
            ("code", T::sized(E::lit(CODE_LEN), code())),
            // Which release wrote the file. A number PICO-8 counts up itself,
            // not the version string the .p8 text form carries.
            ("version", T::u8()),
            // 160 by 205 is 31 bytes more than the cart's 0x8001. Nothing
            // documents what is in them, and they are not zero: both carts in
            // the sample collection carry bytes here that nobody has explained.
            ("slack", T::bytes(E::Remaining)),
        ],
    )
}

/// What a cart's code comes to: the same one-field space
/// [`super::decoded_text`] opens, read as P8SCII rather than as UTF-8.
fn code_text() -> T {
    T::structure_named(
        "DecodedText",
        "",
        "text",
        vec![("text", T::text(StrLen::Fixed(E::Remaining), Encoding::P8scii))],
    )
}

/// The Lua region, as whichever of the three shapes its first four bytes say.
fn code() -> T {
    // The header of each compressed form: the magic, the length of the text it
    // unpacks to, and, for the newer one, the length of the run itself.
    let pxa = T::structure(
        "Pxa",
        vec![
            ("magic", T::magic(b"\0pxa")),
            ("text_len", T::u16(Big)),
            ("packed_len", T::u16(Big)),
            // `packed_len` counts the eight header bytes above as well, so
            // the stream itself is that much shorter. Clamped at zero because
            // a cart damaged into saying less than eight would otherwise ask
            // for a run of a negative length, and a field that reads as empty
            // says what is there better than one that refuses to be read.
            (
                "packed",
                T::decoded(
                    E::field("packed_len").sub(E::lit(8)).at_least(E::lit(0)),
                    Codec::Pico8Pxa,
                    code_text(),
                ),
            ),
            ("unused", T::bytes(E::Remaining)),
        ],
    );
    let old = T::structure(
        "OldCompressed",
        vec![
            ("magic", T::magic(b":c:\0")),
            ("text_len", T::u16(Big)),
            ("reserved", T::bytes(E::lit(2))),
            // Nothing says how long this one is: it ends at a pair of zero
            // bytes, and the rest of the region is room it did not need.
            ("packed", T::decoded(E::Remaining, Codec::Pico8Old, code_text())),
        ],
    );
    let plain = T::structure_named(
        "PlainCode",
        "",
        "text",
        vec![("text", T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::P8scii))],
    );
    // The first four bytes as one big-endian number, which is what tells the
    // three apart without reading any of them twice.
    T::switch(E::Peek { bits: 32, endian: Big }, vec![(0x0070_7861, pxa), (0x3a63_3a00, old)], plain)
}

pub fn p8png() -> Template {
    // The four steps, innermost last. Every one of them is a run of bytes that
    // stays where it is, with a space of its own opened over what it produces.
    let pixels = T::decoded(E::Remaining, Codec::LowBitsArgb, cart());
    let scanlines =
        T::decoded(E::Remaining, Codec::PngUnfilter { stride: WIDTH * 4, bpp: 4 }, pixels);
    let idat = T::decoded(E::field("length"), Codec::Zlib, scanlines);
    Template::new("p8png", super::png::cart_png("Pico8Cart", idat))
}

/// Where the code lives in the cart, and the row of the image the byte at that
/// offset falls in: 0x4300 is pixel 17,152, which is row 107 column 32.
const CODE_AT: usize = 0x4300;
const CODE_ROW: usize = CODE_AT / WIDTH as usize;

/// The byte saying which release of PICO-8 wrote the file, and its row, which
/// is the last one in the image.
const VERSION_AT: usize = 0x8000;
const VERSION_ROW: usize = VERSION_AT / WIDTH as usize;

/// Whether a PNG is a PICO-8 cartridge: 160 by 205, eight bits a channel,
/// colour type 6, which is RGBA, and code where a cart keeps its code.
///
/// The size is where it starts. Any picture can be 160 by 205, so the pixels
/// are read the way the template reads them, through the zlib stream, the
/// filters and the low bits, and the cart's own landmarks are checked in what
/// comes out: 0x4300 has to open one of the three ways code is stored, and the
/// version byte at 0x8000 has to be a release number rather than anything at
/// all.
///
/// ## When the code is out of reach
///
/// A sniff sees the first [`SNIFF_WINDOW`](super::recognise::SNIFF_WINDOW)
/// bytes of the file, 36 KB of it, and 0x4300 is most of the way down the
/// image. A cart whose picture packs well is decided here; one whose picture is
/// noisy enough that its IDAT runs past the window is accepted on its size
/// alone, the way this test worked before. Accepting too much is the right way
/// round for a probe to be wrong: refusing a real cart leaves the editor unable
/// to open a file it understands, while accepting a busy picture of exactly
/// this size costs the reader one wrong guess they can correct from the type
/// list.
pub fn is_p8png(head: &[u8]) -> bool {
    if !super::png::is_size(head, WIDTH, HEIGHT) {
        return false;
    }
    let pixels = super::png::cart_pixels(head, WIDTH, HEIGHT);
    let stride = WIDTH as usize * 4;
    let rows = pixels.len() / stride;
    if rows <= CODE_ROW {
        // The picture did not fit in the window. Nothing was read that could
        // say no, so the size has the last word.
        return true;
    }
    let Ok((cart, _)) = crate::codec::pixels::low_bits_argb(&pixels[..rows * stride]) else { return true };
    if cart.len() < CODE_AT + 16 || !is_code(&cart[CODE_AT..CODE_AT + 16]) {
        return false;
    }
    // Only when the whole image came out, since 0x8000 is in the last row.
    if rows > VERSION_ROW && cart[VERSION_AT] > 64 {
        return false;
    }
    true
}

/// Whether sixteen bytes at 0x4300 are the start of a cart's code.
///
/// Two of the three shapes name themselves. The third is Lua as it was typed,
/// and a cart with less of it than sixteen bytes pads the rest with NULs, so
/// what is asked of plain text is: a printable first byte, printable or
/// whitespace up to the first NUL, and nothing but NULs after that.
fn is_code(bytes: &[u8]) -> bool {
    if bytes.starts_with(b":c:\0") || bytes.starts_with(b"\0pxa") {
        return true;
    }
    let mut ended = false;
    for (i, &b) in bytes.iter().enumerate() {
        if ended {
            if b != 0 {
                return false;
            }
            continue;
        }
        match b {
            0 if i > 0 => ended = true,
            b'\n' | b'\r' | b'\t' => {}
            0x20..=0x7e => {}
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::recognise::{sniff, SNIFF_WINDOW};
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = (data.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(data);
        v.extend_from_slice(&[0; 4]); // CRC, not checked by the template
        v
    }

    /// A cart PNG built from a cart: the low bits of every channel carry the
    /// bytes, the high bits carry a picture, and each row gets a filter byte.
    fn cart_png(cart: &[u8]) -> Vec<u8> {
        cart_png_painted(cart, false)
    }

    /// The same, with `noisy` saying whether the picture is a gradient, which
    /// packs into a few kilobytes, or random, which packs into none at all and
    /// gives an IDAT longer than a sniff ever sees.
    fn cart_png_painted(cart: &[u8], noisy: bool) -> Vec<u8> {
        assert_eq!(cart.len(), (WIDTH * HEIGHT) as usize);
        let stride = (WIDTH * 4) as usize;
        let mut raw = Vec::with_capacity(HEIGHT as usize * (stride + 1));
        let mut seed = 0x1234_5678u32;
        for row in 0..HEIGHT as usize {
            // Filter none, so the bytes in the stream are the pixels. The
            // unfilter tests upstream cover the other four.
            raw.push(0u8);
            for col in 0..WIDTH as usize {
                let byte = cart[row * WIDTH as usize + col];
                // Something in the high six bits so that the picture is not
                // all one colour and the low bits are demonstrably the only
                // thing being read.
                let mut paint = [((row + col) as u8) << 2; 4];
                if noisy {
                    // A different value in every channel of every pixel, so
                    // that the picture packs into more than the sniff window.
                    for p in paint.iter_mut() {
                        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        *p = (seed >> 24) as u8 & !3;
                    }
                }
                raw.push(paint[0] | (byte >> 4 & 3)); // red
                raw.push(paint[1] | (byte >> 2 & 3)); // green
                raw.push(paint[2] | (byte & 3)); // blue
                raw.push(paint[3] | (byte >> 6 & 3)); // alpha
            }
        }
        let mut ihdr = WIDTH.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&HEIGHT.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&chunk(b"IHDR", &ihdr));
        png.extend_from_slice(&chunk(b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6)));
        png.extend_from_slice(&chunk(b"IEND", b""));
        png
    }

    /// `print("hi")` as a pxa stream: eleven literals, each a table position
    /// written narrow and moved to the front. Written out rather than built,
    /// because building it would mean an encoder nothing else needs; the
    /// decoder's own tests cover the shapes this does not.
    const PXA_PRINT_HI: [u8; 13] =
        [15, 240, 4, 183, 63, 192, 35, 172, 15, 95, 95, 37, 251];

    /// The same text in the older scheme: table indices, an escape apiece for
    /// the two quotes and the round brackets, and the pair of zeroes that ends
    /// it.
    const OLD_PRINT_HI: [u8; 15] =
        [28, 30, 21, 26, 32, 42, 0, 34, 20, 21, 0, 34, 43, 0, 0];

    /// A cart with something recognisable at each landmark.
    fn a_cart() -> Vec<u8> {
        let mut c = vec![0u8; (WIDTH * HEIGHT) as usize];
        c[0] = 0x5a; // first byte of the sprite sheet
        c[0x2000] = 0x77; // first byte of the map
        // Eleven characters of text, and eight header bytes over thirteen of
        // stream, which is what `packed_len` counts.
        c[0x4300..0x4308].copy_from_slice(b"\0pxa\x00\x0b\x00\x15");
        c[0x4308..0x4308 + PXA_PRINT_HI.len()].copy_from_slice(&PXA_PRINT_HI);
        c[0x8000] = 41; // what the carts in the sample collection say
        c
    }

    #[test]
    fn the_chain_reads_from_the_png_through_to_the_cart() {
        let d = Document::new(MemSource(cart_png(&a_cart())));
        let mut ev = Evaluator::new(p8png());
        // The IDAT chunk's data, and the three spaces opened under it.
        let idat = ev.node(&d, &[1, 1, 2]).unwrap();
        assert_eq!(idat.type_name, "zlib");
        let scanlines = ev.node(&d, &[1, 1, 2, 0]).unwrap();
        assert_eq!(scanlines.type_name, "png unfilter");
        let pixels = ev.node(&d, &[1, 1, 2, 0, 0]).unwrap();
        assert_eq!(pixels.type_name, "low bits argb");
        let cart = ev.node(&d, &[1, 1, 2, 0, 0, 0]).unwrap();
        assert_eq!(cart.type_name, "Cart");
        // 32,800 bytes: the cart and the 31 bytes of slack after it.
        assert_eq!(cart.size_bits, (WIDTH * HEIGHT) as u64 * 8);
        // The version byte is the eighth field, at 0x8000 of the cart's space.
        let version = ev.node(&d, &[1, 1, 2, 0, 0, 0, 7]).unwrap();
        assert_eq!(version.value, Value::UInt(41));
        assert_eq!(version.offset_bits, 0x8000 * 8);
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 8]).unwrap().size_bits, 31 * 8);
    }

    #[test]
    fn the_code_region_says_which_of_the_three_shapes_it_is() {
        let d = Document::new(MemSource(cart_png(&a_cart())));
        let mut ev = Evaluator::new(p8png());
        let code = ev.node(&d, &[1, 1, 2, 0, 0, 0, 6]).unwrap();
        assert_eq!(code.type_name, "Pxa");
        assert_eq!(code.offset_bits, 0x4300 * 8);
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 1]).unwrap().value, Value::UInt(11));
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 2]).unwrap().value, Value::UInt(21));
        // The run is the stream alone, the eight header bytes taken off.
        let packed = ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 3]).unwrap();
        assert_eq!(packed.size_bits, 13 * 8);
        assert_eq!(packed.type_name, "pico-8 pxa");
        // And what it comes to is the Lua, in a space of its own.
        assert_eq!(
            ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 3, 0, 0]).unwrap().value,
            Value::Str("print(\"hi\")".into())
        );

        // Uncompressed Lua, which is what a cart written before 0.2.0 without
        // enough code to be worth packing holds.
        let mut c = a_cart();
        c[0x4300..0x4310].copy_from_slice(b"print(\"hi\")\0\0\0\0\0");
        let d = Document::new(MemSource(cart_png(&c)));
        let mut ev = Evaluator::new(p8png());
        let code = ev.node(&d, &[1, 1, 2, 0, 0, 0, 6]).unwrap();
        assert_eq!(code.type_name, "PlainCode");
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 0]).unwrap().value, Value::Str("print(\"hi\")".into()));

    }

    /// A `packed_len` smaller than the eight header bytes it counts, which no
    /// encoder writes and a damaged cart may hold. The clamp reads it as no
    /// stream at all rather than as a run of a negative length, so the field
    /// is empty, its text is empty, and everything after it in the cart still
    /// reads.
    #[test]
    fn a_packed_len_shorter_than_its_own_header_reads_as_no_stream() {
        let mut c = a_cart();
        c[0x4306..0x4308].copy_from_slice(&3u16.to_be_bytes());
        let d = Document::new(MemSource(cart_png(&c)));
        let mut ev = Evaluator::new(p8png());
        let packed = ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 3]).unwrap();
        assert_eq!(packed.size_bits, 0);
        assert_eq!(packed.type_name, "pico-8 pxa");
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 3, 0, 0]).unwrap().value, Value::Str(String::new()));
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 7]).unwrap().value, Value::UInt(41));
    }

    /// The scheme before pxa, from the PNG through to its Lua. The decoder's
    /// own tests cover the stream; this covers the template picking the older
    /// shape out of the first four bytes and reading it in place.
    #[test]
    fn the_old_scheme_reads_through_to_its_lua() {
        let mut c = a_cart();
        c[0x4300..0x4400].fill(0);
        c[0x4300..0x4308].copy_from_slice(b":c:\0\x00\x0b\0\0");
        c[0x4308..0x4308 + OLD_PRINT_HI.len()].copy_from_slice(&OLD_PRINT_HI);
        let d = Document::new(MemSource(cart_png(&c)));
        let mut ev = Evaluator::new(p8png());
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6]).unwrap().type_name, "OldCompressed");
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 1]).unwrap().value, Value::UInt(11));
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 3]).unwrap().type_name, "pico-8 old code");
        assert_eq!(
            ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 3, 0, 0]).unwrap().value,
            Value::Str("print(\"hi\")".into())
        );
    }

    /// A cart's text is P8SCII, so a byte above 0x7f is the glyph PICO-8 draws
    /// there rather than a broken UTF-8 sequence. 0x8e is the O button, which
    /// is in the sample collection's older cart twice.
    #[test]
    fn code_reads_as_p8scii_rather_than_utf8() {
        let mut c = a_cart();
        c[0x4300..0x430d].copy_from_slice(b"if(btn(\x8e))\0\0\0");
        let d = Document::new(MemSource(cart_png(&c)));
        let mut ev = Evaluator::new(p8png());
        assert_eq!(
            ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 0]).unwrap().value,
            Value::Str("if(btn(\u{1f17e}))".into())
        );
    }

    #[test]
    fn only_a_png_of_the_right_size_and_shape_is_a_cart() {
        let png = cart_png(&a_cart());
        assert!(is_p8png(&png));
        // The same header at any other size is an ordinary PNG.
        let mut other = png.clone();
        other[19] = 100;
        assert!(!is_p8png(&other));
        // Right size, eight bits, but RGB rather than RGBA.
        let mut rgb = png.clone();
        rgb[25] = 2;
        assert!(!is_p8png(&rgb));
        // Right size and shape, but interlaced, which no cart is.
        let mut adam7 = png.clone();
        adam7[28] = 1;
        assert!(!is_p8png(&adam7));
        assert!(!is_p8png(b"\x89PNG\r\n\x1a\n"));
    }

    /// The size is not enough on its own: a picture of the right size whose
    /// pixels carry nothing is a PNG, and one carrying a cart is a cart.
    #[test]
    fn a_picture_of_the_right_size_carrying_nothing_is_only_a_picture() {
        let cart = cart_png(&a_cart());
        // If the picture ever stops fitting in the window this test starts
        // passing through the fallback, and would prove nothing.
        assert!(cart.len() < SNIFF_WINDOW, "the cart PNG no longer fits in the sniff window");
        assert!(is_p8png(&cart));
        assert_eq!(sniff(&cart, cart.len() as u64), Some("p8png"));

        let empty = cart_png(&vec![0u8; (WIDTH * HEIGHT) as usize]);
        assert!(empty.len() < SNIFF_WINDOW);
        assert!(!is_p8png(&empty));
        assert_eq!(sniff(&empty, empty.len() as u64), Some("png"));

        // Noise at 0x4300 is not code either.
        let mut noise = a_cart();
        for (i, b) in noise[CODE_AT..CODE_AT + 16].iter_mut().enumerate() {
            *b = 0x80 | i as u8;
        }
        assert!(!is_p8png(&cart_png(&noise)));

        // And a version byte no release of PICO-8 ever wrote.
        let mut future = a_cart();
        future[VERSION_AT] = 200;
        assert!(!is_p8png(&cart_png(&future)));
    }

    /// A cart whose picture is too busy to pack into the sniff window is taken
    /// on its size, since the bytes that would say otherwise are not there.
    #[test]
    fn a_cart_whose_idat_outruns_the_window_is_taken_on_its_size() {
        let png = cart_png_painted(&a_cart(), true);
        assert!(png.len() > SNIFF_WINDOW, "the noisy picture packed too well to test the fallback");
        let head = &png[..SNIFF_WINDOW];
        // Well short of row 107, so nothing was read that could say no.
        let rows = super::super::png::cart_pixels(head, WIDTH, HEIGHT).len() / (WIDTH as usize * 4);
        assert!(rows > 0 && rows <= CODE_ROW, "{rows} rows came out of the window");
        assert!(is_p8png(head));
        assert_eq!(sniff(head, png.len() as u64), Some("p8png"));
    }
}
