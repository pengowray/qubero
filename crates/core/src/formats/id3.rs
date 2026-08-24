//! ID3v2 tags, the front of most MP3 files.
//!
//! This is the format that made encodings worth doing: every text frame starts
//! with a byte saying how the rest of it is encoded, and one of the choices is
//! "UTF-16, with a byte-order mark that tells you which way round". A `Switch`
//! on that byte picks the text type, so the template says what the format says.
//!
//! Frame sizes are the plain 32-bit ones in ID3v2.3 and synchsafe (seven bits
//! per byte) in 2.4. The template switches on the tag's version field and
//! rebuilds the synchsafe value with arithmetic, since the expression language
//! has no shifts.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// The byte at the front of a text frame, and what it means.
const TEXT_ENCODING: &[(i128, &str)] =
    &[(0, "latin-1"), (1, "utf-16 bom"), (2, "utf-16 be"), (3, "utf-8")];

/// A four-character frame id as the big-endian number a switch compares.
fn cc(s: &str) -> i128 {
    s.bytes().fold(0i128, |acc, b| (acc << 8) | b as i128)
}

pub fn id3() -> Template {
    Template::new("id3", tag())
}

/// The tag itself, which is also what a WAVE file keeps in an `id3 ` chunk.
pub(super) fn tag() -> T {
    // The tag's own length is four bytes of seven bits each, so a tag can never
    // contain a run that looks like an MPEG frame header.
    let synchsafe = E::field("size_0")
        .mul(E::lit(1 << 21))
        .add(E::field("size_1").mul(E::lit(1 << 14)))
        .add(E::field("size_2").mul(E::lit(1 << 7)))
        .add(E::field("size_3"));

    T::structure(
        "ID3",
        vec![
            ("magic", T::magic(b"ID3")),
            ("version", T::u8()),
            ("revision", T::u8()),
            ("flags", T::u8()),
            ("size_0", T::u8()),
            ("size_1", T::u8()),
            ("size_2", T::u8()),
            ("size_3", T::u8()),
            (
                "frames",
                T::sized(
                    synchsafe,
                    T::repeat(frame(), Until::FieldBytes { field: "id".into(), bytes: vec![0, 0, 0, 0] }),
                ),
            ),
        ],
    )
}

fn frame() -> T {
    // In 2.4 the four size bytes carry seven bits each, so the number the field
    // reads as is not the size. Take the four bytes back out of it by dividing,
    // then put them together shifted by sevens.
    let raw = E::field("size");
    let b0 = raw.clone().div(E::lit(1 << 24));
    let b1 = raw.clone().div(E::lit(1 << 16)).sub(b0.clone().mul(E::lit(256)));
    let b2 = raw.clone().div(E::lit(1 << 8)).sub(raw.clone().div(E::lit(1 << 16)).mul(E::lit(256)));
    let b3 = raw.clone().sub(raw.clone().div(E::lit(256)).mul(E::lit(256)));
    let synchsafe = b0
        .mul(E::lit(1 << 21))
        .add(b1.mul(E::lit(1 << 14)))
        .add(b2.mul(E::lit(1 << 7)))
        .add(b3);

    T::structure_named(
        "Frame",
        "id",
        "body",
        vec![
            ("id", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("size", T::u32(Big)),
            ("flags", T::u16(Big)),
            (
                "body",
                T::switch(
                    E::field("version"),
                    vec![(4, T::sized(synchsafe, body()))],
                    T::sized(E::field("size"), body()),
                ),
            ),
        ],
    )
}

fn body() -> T {
    let mut cases: Vec<(i128, T)> = Vec::new();
    // Every T*** frame is an encoding byte followed by text; the ones people
    // actually look at are named here so the tree reads.
    for id in ["TIT2", "TPE1", "TPE2", "TALB", "TCON", "TRCK", "TYER", "TDRC", "TCOM", "TENC", "TSSE", "TPOS"] {
        cases.push((cc(id), text_frame()));
    }
    cases.push((cc("COMM"), comment_frame()));
    // Inside the frame's window, "the rest of it" is the honest length, and it
    // is right whichever way the version encodes the size.
    T::switch(E::field("id"), cases, T::bytes(E::Remaining))
}

/// The text of a frame, in whichever encoding its first byte names.
fn text(len: E) -> T {
    T::switch(
        E::field("encoding"),
        vec![
            (0, T::text(StrLen::Fixed(len.clone()), Encoding::Latin1)),
            // 1 is UTF-16 with a byte-order mark. The fallback matters: a writer
            // that leaves the mark out meant big-endian.
            (1, T::text(StrLen::Fixed(len.clone()), Encoding::Bom { fallback: Box::new(Encoding::Utf16(Big)) })),
            (2, T::text(StrLen::Fixed(len.clone()), Encoding::Utf16(Big))),
            (3, T::text(StrLen::Fixed(len.clone()), Encoding::Utf8)),
        ],
        // An encoding byte the standard does not define: read it as something,
        // and say that the something was a guess.
        T::text(StrLen::Fixed(len), Encoding::Unknown),
    )
}

fn text_frame() -> T {
    T::structure(
        "TextFrame",
        vec![
            ("encoding", T::enumeration("TextEncoding", T::u8(), TEXT_ENCODING)),
            ("text", text(E::Remaining)),
        ],
    )
}

fn comment_frame() -> T {
    T::structure(
        "Comment",
        vec![
            ("encoding", T::enumeration("TextEncoding", T::u8(), TEXT_ENCODING)),
            ("language", T::text(StrLen::Fixed(E::lit(3)), Encoding::Ascii)),
            ("text", text(E::Remaining)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn frame_bytes(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_be_bytes());
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(body);
        v
    }

    fn tag() -> Vec<u8> {
        let mut frames = Vec::new();
        // UTF-16 with a little-endian mark.
        let mut tit2 = vec![1u8, 0xff, 0xfe];
        for u in "Blue".encode_utf16() {
            tit2.extend_from_slice(&u.to_le_bytes());
        }
        frames.extend_from_slice(&frame_bytes(b"TIT2", &tit2));
        // Latin-1: one byte per character, including the accent.
        frames.extend_from_slice(&frame_bytes(b"TPE1", &[0, b'B', b'j', 0xf6, b'r', b'k']));
        // UTF-8.
        frames.extend_from_slice(&frame_bytes(b"TCON", &[3, b'P', b'o', b'p']));

        let size = frames.len();
        let mut out = b"ID3".to_vec();
        out.extend_from_slice(&[3, 0, 0]); // version 2.3, no flags
        out.extend_from_slice(&[
            (size >> 21) as u8 & 0x7f,
            (size >> 14) as u8 & 0x7f,
            (size >> 7) as u8 & 0x7f,
            size as u8 & 0x7f,
        ]);
        out.extend_from_slice(&frames);
        out
    }

    #[test]
    fn version_4_frame_sizes_are_synchsafe() {
        // One frame of 200 bytes: 0xC8 as a plain u32, but 0x01 0x48 synchsafe.
        let body = vec![3u8; 200];
        let mut frames = b"TXXX".to_vec();
        frames.extend_from_slice(&[0, 0, 1, 0x48]); // 200, seven bits per byte
        frames.extend_from_slice(&[0, 0]);
        frames.extend_from_slice(&body);

        let size = frames.len();
        let mut out = b"ID3".to_vec();
        out.extend_from_slice(&[4, 0, 0]); // version 2.4
        out.extend_from_slice(&[
            (size >> 21) as u8 & 0x7f,
            (size >> 14) as u8 & 0x7f,
            (size >> 7) as u8 & 0x7f,
            size as u8 & 0x7f,
        ]);
        out.extend_from_slice(&frames);

        let d = Document::new(MemSource(out));
        let mut ev = Evaluator::new(id3());
        assert_eq!(ev.node(&d, &[8]).unwrap().child_count, 1);
        // The body is 200 bytes, not the 328 the raw number would say.
        assert_eq!(ev.node(&d, &[8, 0, 3]).unwrap().size_bits, 200 * 8);
        assert_eq!(ev.node(&d, &[8, 0]).unwrap().size_bits, 210 * 8);
    }

    #[test]
    fn text_frames_read_in_the_encoding_their_first_byte_names() {
        let d = Document::new(MemSource(tag()));
        let mut ev = Evaluator::new(id3());
        let frames = ev.node(&d, &[8]).unwrap();
        assert_eq!(frames.child_count, 3);

        // TIT2: UTF-16, and the mark says little-endian.
        assert_eq!(ev.node(&d, &[8, 0, 0]).unwrap().value, Value::Str("TIT2".into()));
        let title = ev.node(&d, &[8, 0, 3, 1]).unwrap();
        assert_eq!(title.value, Value::Str("Blue".into()));
        assert_eq!(title.read_as.as_deref(), Some("Read as UTF-16 LE, from a byte-order mark"));
        assert_eq!(title.value_offset_bits, title.offset_bits + 16);

        // TPE1: Latin-1, where 0xF6 is a letter rather than half a character.
        let artist = ev.node(&d, &[8, 1, 3, 1]).unwrap();
        assert_eq!(artist.value, Value::Str("Bj\u{00f6}rk".into()));
        assert_eq!(artist.type_name, "latin1[]");

        // TCON: UTF-8, named outright, so nothing to report.
        let genre = ev.node(&d, &[8, 2, 3, 1]).unwrap();
        assert_eq!(genre.value, Value::Str("Pop".into()));
        assert_eq!(genre.read_as, None);

        // Writing keeps the encoding: an accent stays one Latin-1 byte.
        let w = ev.prepare_write(&d, &[8, 1, 3, 1], "Bj\u{00f8}rn").unwrap();
        assert_eq!(w.data, vec![b'B', b'j', 0xf8, b'r', b'n']);
        // And a character Latin-1 cannot hold is refused rather than mangled.
        assert!(ev.prepare_write(&d, &[8, 1, 3, 1], "Bj\u{5c3d}rk").is_err());
    }
}
