//! Picotron cartridge ROM: eight bytes of header and then the whole cartridge
//! filesystem as one LZ4 block.
//!
//! A Picotron cartridge is a folder rather than a file. The manual says so
//! plainly: "Cartridge files (.p64, .p64.png) in Picotron are logically
//! folders", and the shell copies things out of one with `cp`, so what a
//! cartridge holds is a tree of paths, each with bytes under it. The `.p64`
//! form writes that tree as a text file and the `.p64.png` form carries it in
//! an image. The `.rom` form is the tree on its own, with nothing wrapped
//! around it, which is what the exporters carry and what this reads.
//!
//! The header is the three letters `p64`, a version byte, and a four-byte
//! little-endian count of the bytes after it. Then that many bytes of one raw
//! LZ4 block: not the LZ4 frame format, which has its own magic and its own
//! block headers, but the bare sequence of literals and matches. The count is
//! the compressed length, and it came to the file length less eight in every
//! sample read, so a file that disagrees has been cut short.
//!
//! What is inside the block is [`super::picotron_pod`]: the entry list, and
//! what one entry's bytes turn out to be.
//!
//! Read against the Picotron manual and its POD and filesystem pages at
//! lexaloffle.com/dl/docs/, and worked out byte for byte from the 28 `.p64.rom`
//! files committed to github.com/akd-io/picotron, which parse to their last
//! byte under the rules above. Certain, because every one of those files is
//! accounted for: the header, the LZ4 block, the entry encoding, the escape at
//! 255 and the trailing slash. Inferred: that the byte after `p64` is a version
//! and not a flag, since every sample holds 2 and none holds anything else;
//! that 255 is the only escape, since no sample needed another; and that a
//! path is UTF-8, since none of them left ASCII.
//!
//! ## The image form
//!
//! A `.p64.png` carries that same ROM, header and all, hidden in the picture
//! of the cartridge's label. The image is 512 by 384, eight bits a channel,
//! RGBA, and every pixel carries eleven bits: the low three of red, then the
//! low three of green, the low three of blue and the low two of alpha, laid
//! end to end into a byte stream least significant bit first. 512 by 384 at
//! eleven bits is 270,336 bytes, which is the 256K of ROM the manual says a
//! cartridge shared this way may hold, and a cartridge smaller than that
//! leaves the rest zero.
//!
//! Read out of the encoder and decoder in `picotron_cart.py` of
//! thisismypassport/shrinko8, and checked byte for byte against two cartridges
//! published as images: both give `p64`, version 2, and a payload size that
//! the LZ4 block then reads out to its last byte.

use super::picotron_pod::cart;
use crate::codec::Codec;
use crate::template::{Endian::Little, Expr as E, Template, Ty as T};

/// What one of these starts with. Three lower-case letters, which is far too
/// little on its own, so `recognise` weighs the version and the length as well.
pub const MAGIC: &[u8] = b"p64";

/// The bytes of header before the compressed block.
pub const HEADER_LEN: u64 = 8;

/// The image a `.p64.png` is, and the bytes one row of it comes to. Four
/// channels a pixel, and no cartridge is any other size.
const WIDTH: u32 = 512;
const HEIGHT: u32 = 384;

pub fn p64rom() -> Template {
    Template::new("p64rom", rom(false))
}

/// The ROM: eight bytes of header and the LZ4 block the header measures.
///
/// `slack` says whether to name what is left after the block. A `.p64.rom`
/// file ends where the block ends, and an image has room the ROM did not fill.
fn rom(slack: bool) -> T {
    let mut fields = vec![
        ("magic", T::magic(MAGIC)),
        // 2 in everything read so far.
        ("version", T::u8()),
        // Of the block below, so a short file shows as one.
        ("payload_size", T::u32(Little)),
        ("cart", T::decoded(E::field("payload_size"), Codec::Lz4Block, cart())),
    ];
    if slack {
        fields.push(("slack", T::bytes(E::Remaining)));
    }
    T::structure("P64Rom", fields)
}

/// A cartridge as it is shared: the ROM hidden in the picture of its label.
///
/// Four steps, the same shape a PICO-8 cartridge is read in and for the same
/// reason: the IDAT chunk is a zlib stream, what comes out of that is PNG
/// scanlines with a filter byte in front of each, what comes out of undoing
/// the filters is pixels, and what comes out of the pixels' low bits is the
/// ROM, header and all.
pub fn p64png() -> Template {
    let bits = T::decoded(E::Remaining, Codec::LowBitsRgba11, rom(true));
    let scanlines = T::decoded(E::Remaining, Codec::PngUnfilter { stride: WIDTH * 4, bpp: 4 }, bits);
    let idat = T::decoded(E::field("length"), Codec::Zlib, scanlines);
    Template::new("p64png", super::png::cart_png("PicotronCart", idat))
}

/// How much a Picotron image holds: 512 by 384 pixels at eleven bits each.
const ROM_CAPACITY: u32 = WIDTH * HEIGHT * 11 / 8;

/// Whether a PNG is a Picotron cartridge: 512 by 384, eight bits a channel,
/// colour type 6, which is RGBA, and a ROM header hidden in the pixels.
///
/// The size is where it starts and not where it ends, since a holiday snap can
/// be 512 by 384 as easily as a cartridge can. So the first row of the image is
/// read the way the template reads it, through the zlib stream, the filters and
/// the low bits, and the eight bytes of ROM header it yields have to say what a
/// header says: `p64`, a version, and a payload no bigger than the picture can
/// carry. One row is 704 bytes of ROM, so a cartridge is always decided here
/// and never falls back to the size alone.
pub fn is_p64png(head: &[u8]) -> bool {
    let pixels = super::png::cart_pixels(head, WIDTH, HEIGHT);
    let stride = WIDTH as usize * 4;
    if pixels.len() < stride {
        return false;
    }
    let Ok((rom, _)) = crate::codec::pixels::low_bits_rgba11(&pixels[..stride]) else { return false };
    if rom.len() < HEADER_LEN as usize || !rom.starts_with(MAGIC) {
        return false;
    }
    // 2 in every cartridge read so far. The range is wider than that because
    // one observation is not a rule, and narrow enough that a picture whose
    // pixels happen to spell `p64` still has to get this byte right too.
    if !(1..=15).contains(&rom[3]) {
        return false;
    }
    let size = u32::from_le_bytes([rom[4], rom[5], rom[6], rom[7]]);
    size <= ROM_CAPACITY - HEADER_LEN as u32
}

#[cfg(test)]
mod tests {
    use super::super::picotron_pod::sample::rom;
    use super::super::recognise::sniff;
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A cartridge image built from a ROM: eleven bits of every pixel carry
    /// the stream, the bits above them carry a picture, and each row gets a
    /// filter byte. The reverse of what `low_bits_rgba11` does.
    fn cart_png(rom: &[u8]) -> Vec<u8> {
        let (w, h) = (WIDTH as usize, HEIGHT as usize);
        let stride = w * 4;
        let mut raw = Vec::with_capacity(h * (stride + 1));
        let (mut held, mut bits, mut at) = (0u32, 0u32, 0usize);
        for row in 0..h {
            // Filter none, so the bytes in the stream are the pixels.
            raw.push(0u8);
            for col in 0..w {
                while bits < 11 {
                    let byte = rom.get(at).copied().unwrap_or(0) as u32;
                    held |= byte << bits;
                    bits += 8;
                    at += 1;
                }
                let word = held & 0x7ff;
                held >>= 11;
                bits -= 11;
                // Something in the bits above the ones being read, so that the
                // picture is not one colour and the low bits are demonstrably
                // the only thing coming out.
                let paint = ((row + col) as u8) << 3;
                raw.push(paint | (word & 7) as u8);
                raw.push(paint | (word >> 3 & 7) as u8);
                raw.push(paint | (word >> 6 & 7) as u8);
                raw.push((paint & !3) | (word >> 9 & 3) as u8);
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

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = (data.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(data);
        v.extend_from_slice(&[0; 4]); // CRC, not checked by the template
        v
    }

    /// How many bytes 512 by 384 pixels at eleven bits each come to.
    const STREAM_LEN: u64 = (WIDTH * HEIGHT) as u64 * 11 / 8;

    #[test]
    fn the_image_carries_the_whole_rom_header_and_all() {
        let d = Document::new(MemSource(cart_png(&rom())));
        let mut ev = Evaluator::new(p64png());
        // The IDAT chunk's data, and the three spaces opened under it.
        assert_eq!(ev.node(&d, &[1, 1, 2]).unwrap().type_name, "zlib");
        assert_eq!(ev.node(&d, &[1, 1, 2, 0]).unwrap().type_name, "png unfilter");
        let bits = ev.node(&d, &[1, 1, 2, 0, 0]).unwrap();
        assert_eq!(bits.type_name, "low bits rgba 11");
        let out = ev.node(&d, &[1, 1, 2, 0, 0, 0]).unwrap();
        assert_eq!(out.type_name, "P64Rom");
        assert_eq!(out.size_bits, STREAM_LEN * 8);

        // The same header the bare .p64.rom form carries.
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 1]).unwrap().value, Value::UInt(2));
        let size = rom().len() as u64 - HEADER_LEN;
        assert_eq!(ev.node(&d, &[1, 1, 2, 0, 0, 0, 2]).unwrap().value, Value::UInt(size as u128));
        // And the entries read through the LZ4 block as they do from a ROM.
        let name = ev.node(&d, &[1, 1, 2, 0, 0, 0, 3, 0, 0, 1, 1]).unwrap();
        assert_eq!(name.value, Value::Str(".info.pod".into()));
        // The image has far more room than this cartridge needs.
        let slack = ev.node(&d, &[1, 1, 2, 0, 0, 0, 4]).unwrap();
        assert_eq!(slack.size_bits, (STREAM_LEN - rom().len() as u64) * 8);
    }

    #[test]
    fn only_a_png_of_the_right_size_and_shape_is_a_cartridge() {
        let png = cart_png(&rom());
        assert!(is_p64png(&png));
        // The same header at any other size is an ordinary PNG.
        let mut other = png.clone();
        other[19] = 100;
        assert!(!is_p64png(&other));
        // Right size, eight bits, but RGB rather than RGBA.
        let mut rgb = png.clone();
        rgb[25] = 2;
        assert!(!is_p64png(&rgb));
        // Right size and shape, but interlaced, which no cartridge is.
        let mut adam7 = png.clone();
        adam7[28] = 1;
        assert!(!is_p64png(&adam7));
        assert!(!is_p64png(b"\x89PNG\r\n\x1a\n"));
    }

    /// The size is not enough on its own: the header hidden in the first row
    /// has to read like a header. One row is 704 bytes of ROM, so this is
    /// always decided and never falls back to the size.
    #[test]
    fn a_picture_of_the_right_size_carrying_nothing_is_only_a_picture() {
        let cart = cart_png(&rom());
        assert_eq!(sniff(&cart, cart.len() as u64), Some("p64png"));

        let empty = cart_png(&[]);
        assert!(!is_p64png(&empty));
        assert_eq!(sniff(&empty, empty.len() as u64), Some("png"));

        // The magic without a version anybody wrote.
        let mut bad_version = rom();
        bad_version[3] = 0;
        assert!(!is_p64png(&cart_png(&bad_version)));

        // The magic and a version, but a payload longer than the picture can
        // carry, so the header is a coincidence rather than a header.
        let mut too_big = rom();
        too_big[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!is_p64png(&cart_png(&too_big)));
    }

    #[test]
    fn the_header_counts_the_compressed_bytes_after_it() {
        let d = Document::new(MemSource(rom()));
        let mut ev = Evaluator::new(p64rom());
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::UInt(2));
        let size = match ev.node(&d, &[2]).unwrap().value {
            Value::UInt(n) => n as u64,
            other => panic!("payload_size read as {other:?}"),
        };
        assert_eq!(size, rom().len() as u64 - HEADER_LEN);
    }
}
