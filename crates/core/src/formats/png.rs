//! PNG: signature plus a chunk stream that ends at IEND.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// PNG colour types. 1, 5 and 7 are not defined by the spec, so a file holding
/// one shows the number with no name.
const COLOR_TYPE: &[(i128, &str)] = &[
    (0, "greyscale"),
    (2, "rgb"),
    (3, "indexed"),
    (4, "greyscale alpha"),
    (6, "rgba"),
];

/// The header chunk, which every PNG opens with and which says what shape the
/// image is. Shared with the cartridge templates, PICO-8 and Picotron, which
/// read the same chunk and then go looking for what the picture is carrying.
pub(crate) fn ihdr() -> T {
    T::structure(
        "IHDR",
        vec![
            ("width", T::u32(Big)),
            ("height", T::u32(Big)),
            ("bit_depth", T::u8()),
            ("color_type", T::enumeration("ColorType", T::u8(), COLOR_TYPE)),
            ("compression", T::enumeration("Compression", T::u8(), &[(0, "deflate")])),
            ("filter", T::enumeration("FilterMethod", T::u8(), &[(0, "adaptive")])),
            ("interlace", T::enumeration("Interlace", T::u8(), &[(0, "none"), (1, "adam7")])),
        ],
    )
}

/// tEXt: a NUL-terminated keyword, then the text filling the rest. Both are
/// Latin-1 by the spec, not UTF-8; iTXt is the chunk that carries UTF-8.
pub(crate) fn text() -> T {
    T::structure(
        "tEXt",
        vec![
            ("keyword", T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Latin1)),
            (
                "text",
                T::text(StrLen::Fixed(E::field("length").sub(E::size_of("keyword"))), Encoding::Latin1),
            ),
        ],
    )
}

/// A PNG that carries a cartridge, as PICO-8 and Picotron both write one.
///
/// The same chunk stream an ordinary PNG is, with `idat` in place of the bytes
/// an IDAT chunk would otherwise be: whatever the cartridge template wants to
/// open over what the picture is carrying. `name` is what the file is called
/// in the listing.
pub(crate) fn cart_png(name: &'static str, idat: T) -> T {
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
                        vec![(0x4948_4452, ihdr()), (0x7445_5874, text()), (0x4944_4154, idat)],
                        T::bytes(E::field("length")),
                    ),
                ),
            ),
            ("crc", T::u32(Big)),
        ],
    );
    T::structure(
        name,
        vec![
            ("signature", T::magic(b"\x89PNG\r\n\x1a\n")),
            ("chunks", T::repeat(chunk, Until::FieldBytes { field: "type".into(), bytes: b"IEND".to_vec() })),
        ],
    )
}

/// Whether a PNG's header says the image is `width` by `height`, eight bits a
/// channel, colour type 6, which is RGBA, and not interlaced. The shape both
/// cartridge formats are written in, and the first of the two questions asked
/// of a picture that might be one.
///
/// Interlacing is in here because Adam7 lays an image out in seven passes, so a
/// row of the stream is not a row of the image and none of the arithmetic in
/// [`cart_pixels`] holds. No cartridge is interlaced.
pub(crate) fn is_size(head: &[u8], width: u32, height: u32) -> bool {
    if !head.starts_with(b"\x89PNG\r\n\x1a\n") || head.get(12..16) != Some(b"IHDR") {
        return false;
    }
    let (Some(w), Some(h)) = (dword(head, 16), dword(head, 20)) else { return false };
    w == width
        && h == height
        && head.get(24) == Some(&8)
        && head.get(25) == Some(&6)
        && head.get(28) == Some(&0)
}

/// The pixel bytes a cartridge image hides its payload in, as far as the bytes
/// of the file a sniff has been given reach.
///
/// A cartridge announces nothing about itself in the file: the only way to tell
/// one from a holiday snap of the same size is to go and read what is hidden in
/// the pixels, and that means running the front of the image through the same
/// three steps the template does. The chunks inside `head` are walked, the IDAT
/// data in them gathered, that stream partly inflated, and the whole rows which
/// came out unfiltered. A row cut off in the middle is dropped, since a filter
/// needs a whole row above it.
///
/// The result is the unfiltered pixels: `width * 4` bytes a row, and however
/// many rows the head reached. Empty if the header is not the right size and
/// shape, or if what came out is not scanlines. A caller reads its own payload
/// out of the rows it was given and decides on those; the rows past the end of
/// the head are the caller's problem, not this function's.
pub(crate) fn cart_pixels(head: &[u8], width: u32, height: u32) -> Vec<u8> {
    if !is_size(head, width, height) {
        return Vec::new();
    }
    let stride = width as usize * 4;
    // What a complete image comes to, filter bytes and all, which is the most
    // output worth keeping and far less than a hostile stream would produce.
    let cap = height as usize * (stride + 1);
    // The chunk stream starts after the eight-byte signature. IHDR is the first
    // chunk in it and is walked past like any other chunk which is not an IDAT.
    let mut at = 8;
    let mut idat: Vec<u8> = Vec::new();
    while at + 8 <= head.len() {
        let Some(len) = dword(head, at) else { break };
        let kind = &head[at + 4..at + 8];
        if kind == b"IEND" {
            break;
        }
        let start = at + 8;
        // A length is four bytes wide and says whatever it likes, so the end of
        // the chunk is worked out in a width that cannot wrap.
        let Some(end) = start.checked_add(len as usize) else { break };
        if kind == b"IDAT" {
            // The last IDAT in the head is the one running past its end, and
            // the bytes of it which are here are the point of the exercise.
            idat.extend_from_slice(&head[start..end.min(head.len())]);
        }
        let Some(next) = end.checked_add(4) else { break };
        if next > head.len() {
            break;
        }
        at = next;
    }
    let raw = crate::codec::inflate::inflate_prefix(&idat, cap);
    let rows = raw.len() / (stride + 1);
    if rows == 0 {
        return Vec::new();
    }
    match crate::codec::pixels::unfilter(&raw[..rows * (stride + 1)], width * 4, 4) {
        Ok((pixels, _)) => pixels,
        Err(_) => Vec::new(),
    }
}

fn dword(head: &[u8], at: usize) -> Option<u32> {
    let bytes: [u8; 4] = head.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

pub fn png() -> Template {
    let (ihdr, text) = (ihdr(), text());
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
                    // A text field in an expression is its bytes as a big-endian number.
                    T::switch(
                        E::field("type"),
                        vec![(0x4948_4452, ihdr), (0x7445_5874, text)],
                        T::bytes(E::field("length")),
                    ),
                ),
            ),
            ("crc", T::u32(Big)),
        ],
    );
    Template::new(
        "png",
        T::structure(
            "PNG",
            vec![
                ("signature", T::magic(b"\x89PNG\r\n\x1a\n")),
                ("chunks", T::repeat(chunk, Until::FieldBytes { field: "type".into(), bytes: b"IEND".to_vec() })),
            ],
        ),
    )
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

    #[test]
    fn text_chunk_splits_at_the_nul() {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&chunk(b"tEXt", b"Author\0Ada Lovelace"));
        b.extend_from_slice(&chunk(b"IEND", b""));
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(png());
        let keyword = ev.node(&d, &[1, 0, 2, 0]).unwrap();
        assert_eq!(keyword.value, Value::Str("Author".into()));
        assert_eq!(keyword.type_name, "latin1 cstr");
        assert_eq!(keyword.size_bits, 7 * 8); // the NUL belongs to the keyword
        let text = ev.node(&d, &[1, 0, 2, 1]).unwrap();
        assert_eq!(text.value, Value::Str("Ada Lovelace".into()));
        assert_eq!(text.offset_bits, (8 + 8 + 7) * 8);
    }

    #[test]
    fn text_chunk_without_a_nul_is_an_error() {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&chunk(b"tEXt", b"nokeyword"));
        b.extend_from_slice(&chunk(b"IEND", b""));
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(png());
        assert!(ev.node(&d, &[1, 0, 2, 0]).is_err());
    }

    #[test]
    fn png_parses_ihdr_and_stops_at_iend() {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&640u32.to_be_bytes());
        b.extend_from_slice(&480u32.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(b"IEND");
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(b"trailing junk");
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(png());
        let chunks = ev.node(&d, &[1]).unwrap();
        assert_eq!(chunks.child_count, 2);
        let ihdr = ev.node(&d, &[1, 0, 2]).unwrap();
        assert_eq!(ihdr.type_name, "IHDR");
        assert_eq!(ev.node(&d, &[1, 0, 2, 1]).unwrap().value, Value::UInt(480));
        assert_eq!(ev.node(&d, &[1, 1, 1]).unwrap().value, Value::Str("IEND".into()));
        let color = ev.node(&d, &[1, 0, 2, 3]).unwrap();
        assert_eq!(color.type_name, "ColorType");
        assert_eq!(color.value, Value::Enum { raw: 6, name: Some("rgba".into()), hex: false });
    }
}
