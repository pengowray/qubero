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
//! ## The code, which this round does not open
//!
//! The Lua at 0x4300 is stored one of three ways, told apart by the first four
//! bytes:
//!
//! - `\0pxa`, from PICO-8 0.2.0 on. Two big-endian 16-bit lengths follow, the
//!   text's length and the compressed run's, and then a bit stream. A literal
//!   is a position in a 256-entry table, written as a unary prefix and then
//!   that many bits, and the byte it names moves to the front of the table. A
//!   back-reference writes its offset in 5, 10 or 15 bits and its length as a
//!   chain of three-bit groups. Taken from the decoder in `src/pxa.rs` of
//!   shanecelis/pico8_decompress, since no published document says it.
//! - `:c:\0`, from before that. A big-endian 16-bit length of the text, two
//!   bytes that say nothing, and then a byte stream: a byte of zero escapes a
//!   literal, a byte up to 0x3b indexes a table of the characters Lua source
//!   is mostly made of, and anything higher is a back-reference into the last
//!   4K of output.
//! - Anything else, in which case the region is the Lua source as it stands,
//!   ASCII, ending at the first zero byte.
//!
//! Both schemes are read here as the header and then the bytes. Unpacking them
//! wants two more decoders, and they are not written yet.

use crate::codec::Codec;
use crate::template::{Encoding, Endian::Big, Expr as E, StrLen, Template, Ty as T, Until};

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
            ("packed", T::bytes(E::field("packed_len"))),
            ("unused", T::bytes(E::Remaining)),
        ],
    );
    let old = T::structure(
        "OldCompressed",
        vec![
            ("magic", T::magic(b":c:\0")),
            ("text_len", T::u16(Big)),
            ("reserved", T::bytes(E::lit(2))),
            ("packed", T::bytes(E::Remaining)),
        ],
    );
    let plain = T::structure_named(
        "PlainCode",
        "",
        "text",
        vec![("text", T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Ascii))],
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
    let chunk = T::structure_named(
        "Chunk",
        "type",
        "data",
        vec![
            ("length", T::u32(Big)),
            ("type", T::utf8(E::lit(4))),
            (
                "data",
                T::sized(
                    E::field("length"),
                    T::switch(
                        E::field("type"),
                        vec![
                            (0x4948_4452, super::png::ihdr()),
                            (0x7445_5874, super::png::text()),
                            (0x4944_4154, idat),
                        ],
                        T::bytes(E::field("length")),
                    ),
                ),
            ),
            ("crc", T::u32(Big)),
        ],
    );
    Template::new(
        "p8png",
        T::structure(
            "Pico8Cart",
            vec![
                ("signature", T::magic(b"\x89PNG\r\n\x1a\n")),
                (
                    "chunks",
                    T::repeat(chunk, Until::FieldBytes { field: "type".into(), bytes: b"IEND".to_vec() }),
                ),
            ],
        ),
    )
}

/// Whether a PNG's header says it is the size and shape a cart is: 160 by 205,
/// eight bits a channel, colour type 6, which is RGBA.
///
/// Not proof. Any 160 by 205 RGBA PNG passes, and the template reads one as a
/// cart full of nothing much. Nothing else in the format announces itself, and
/// PICO-8 itself decides the same way.
pub fn is_p8png(head: &[u8]) -> bool {
    if !head.starts_with(b"\x89PNG\r\n\x1a\n") || head.get(12..16) != Some(b"IHDR") {
        return false;
    }
    let (Some(w), Some(h)) = (dword(head, 16), dword(head, 20)) else { return false };
    w == WIDTH && h == HEIGHT && head.get(24) == Some(&8) && head.get(25) == Some(&6)
}

fn dword(head: &[u8], at: usize) -> Option<u32> {
    let bytes: [u8; 4] = head.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(cart.len(), (WIDTH * HEIGHT) as usize);
        let stride = (WIDTH * 4) as usize;
        let mut raw = Vec::with_capacity(HEIGHT as usize * (stride + 1));
        for row in 0..HEIGHT as usize {
            // Filter none, so the bytes in the stream are the pixels. The
            // unfilter tests upstream cover the other four.
            raw.push(0u8);
            for col in 0..WIDTH as usize {
                let byte = cart[row * WIDTH as usize + col];
                // Something in the high six bits so that the picture is not
                // all one colour and the low bits are demonstrably the only
                // thing being read.
                let paint = ((row + col) as u8) << 2;
                let ch = |bits: u8| paint | bits;
                raw.push(ch(byte >> 4 & 3)); // red
                raw.push(ch(byte >> 2 & 3)); // green
                raw.push(ch(byte & 3)); // blue
                raw.push(ch(byte >> 6 & 3)); // alpha
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

    /// A cart with something recognisable at each landmark.
    fn a_cart() -> Vec<u8> {
        let mut c = vec![0u8; (WIDTH * HEIGHT) as usize];
        c[0] = 0x5a; // first byte of the sprite sheet
        c[0x2000] = 0x77; // first byte of the map
        c[0x4300..0x4308].copy_from_slice(b"\0pxa\x00\x42\x00\x0a");
        c[0x4308..0x4312].copy_from_slice(b"packedcode");
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
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 1]).unwrap().value, Value::UInt(0x42));
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 2]).unwrap().value, Value::UInt(10));
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 3]).unwrap().size_bits, 10 * 8);

        // Uncompressed Lua, which is what a cart written before 0.2.0 without
        // enough code to be worth packing holds.
        let mut c = a_cart();
        c[0x4300..0x4310].copy_from_slice(b"print(\"hi\")\0\0\0\0\0");
        let d = Document::new(MemSource(cart_png(&c)));
        let mut ev = Evaluator::new(p8png());
        let code = ev.node(&d, &[1, 1, 2, 0, 0, 0, 6]).unwrap();
        assert_eq!(code.type_name, "PlainCode");
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 0]).unwrap().value, Value::Str("print(\"hi\")".into()));

        // And the scheme before pxa.
        let mut c = a_cart();
        c[0x4300..0x4308].copy_from_slice(b":c:\0\x01\x00\0\0");
        let d = Document::new(MemSource(cart_png(&c)));
        let mut ev = Evaluator::new(p8png());
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6]).unwrap().type_name, "OldCompressed");
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 6, 1]).unwrap().value, Value::UInt(256));
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
        assert!(!is_p8png(b"\x89PNG\r\n\x1a\n"));
    }
}
