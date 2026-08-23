//! RIFF/WAVE, and the metadata chunks that bat recordings carry.
//!
//! A RIFF chunk is an id, a little-endian size, and a body padded to an even
//! number of bytes. The pad byte is outside the size but inside the file, so
//! it is a field of its own: `size` modulo two, written with a divide and a
//! multiply because the expression language has no modulo.
//!
//! GUANO (`guan`) is the metadata format bat detectors write: UTF-8 lines of
//! `Key:Value`, one per line, sometimes with a trailing NUL inside the chunk
//! size. Lines are read with a terminator that tolerates its own absence, so
//! the last line reads whether or not it ends in a newline.
//!
//! `wamd` is Wildlife Acoustics' own metadata: a flat stream of 16-bit tag,
//! 32-bit length, payload. Its tag numbers here were read out of files, not
//! from a specification.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// A four-character chunk id as the big-endian number a switch compares.
fn cc(s: &str) -> i128 {
    s.bytes().fold(0i128, |acc, b| (acc << 8) | b as i128)
}

/// The format tags a WAVE file's `fmt ` chunk can carry.
const FORMAT_TAG: &[(i128, &str)] = &[
    (0x0001, "pcm"),
    (0x0003, "ieee float"),
    (0x0006, "a-law"),
    (0x0007, "mu-law"),
    (0x5741, "w4v"),
    (0xfffe, "extensible"),
];

/// `wamd` tag numbers, as seen in Song Meter files.
const WAMD_TAG: &[(i128, &str)] = &[
    (0, "version"),
    (1, "model"),
    (2, "serial"),
    (3, "firmware"),
    (4, "prefix"),
    (5, "timestamp"),
    (6, "position"),
    (7, "temperature"),
];

pub fn wav() -> Template {
    riff("wav", chunk_body(None))
}

/// The RIFF frame shared by WAVE and its relatives.
pub(super) fn riff(name: &str, body: T) -> Template {
    Template::new(
        name,
        T::structure(
            "RIFF",
            vec![
                ("magic", T::magic(b"RIFF")),
                ("size", T::u32(Little)),
                ("form", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
                ("chunks", T::repeat(T::Named("Chunk".into()), Until::End)),
            ],
        ),
    )
    .with_type("Chunk", chunk(body))
    .with_type("ListItem", list_item())
}

fn chunk(body: T) -> T {
    // A chunk body is padded to an even length. The pad byte is not counted in
    // the size, so it is a field of its own: size - (size / 2) * 2.
    let pad = E::field("size").sub(E::field("size").div(E::lit(2)).mul(E::lit(2)));
    T::structure(
        "Chunk",
        vec![
            ("id", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("size", T::u32(Little)),
            ("body", T::sized(E::field("size"), body)),
            ("pad", T::bytes(pad)),
        ],
    )
}

/// What is inside a chunk, by its id. `data` is left as bytes unless a format
/// on top of WAVE knows how to read it.
pub(super) fn chunk_body(data: Option<T>) -> T {
    let mut cases = vec![
            (cc("fmt "), fmt()),
            (cc("fact"), T::structure("Fact", vec![("sample_count", T::u32(Little))])),
            (cc("guan"), guano()),
            (cc("wamd"), T::repeat(wamd_item(), Until::End)),
            (cc("cue "), cue()),
            (cc("LIST"), list()),
            (cc("ds64"), ds64()),
    ];
    if let Some(d) = data {
        cases.push((cc("data"), d));
    }
    T::switch(E::field("id"), cases, T::bytes(E::Remaining))
}

fn fmt() -> T {
    T::structure(
        "Format",
        vec![
            ("format", T::enumeration_hex("FormatTag", T::u16(Little), FORMAT_TAG)),
            ("channels", T::u16(Little)),
            ("sample_rate", T::u32(Little)),
            ("byte_rate", T::u32(Little)),
            ("block_align", T::u16(Little)),
            ("bits_per_sample", T::u16(Little)),
            // 16-byte fmt chunks stop here; longer ones carry an extension.
            ("extra", T::bytes(E::Remaining)),
        ],
    )
}

/// GUANO: one `Key:Value` per line, in UTF-8.
fn guano() -> T {
    T::repeat(
        T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Utf8),
        Until::End,
    )
}

fn wamd_item() -> T {
    T::structure(
        "Item",
        vec![
            ("tag", T::enumeration("WamdTag", T::u16(Little), WAMD_TAG)),
            ("length", T::u32(Little)),
            // Ids 1 to 6 hold text; the rest are bytes, so read them as text
            // only when they say they are.
            (
                "value",
                T::sized(
                    E::field("length"),
                    T::switch(
                        E::field("tag"),
                        (1..=7).map(|t| (t as i128, T::text(StrLen::Fixed(E::Remaining), Encoding::Latin1))).collect(),
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    )
}

fn cue() -> T {
    let point = T::structure(
        "CuePoint",
        vec![
            ("id", T::u32(Little)),
            ("position", T::u32(Little)),
            ("chunk", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("chunk_start", T::u32(Little)),
            ("block_start", T::u32(Little)),
            ("sample_offset", T::u32(Little)),
        ],
    );
    T::structure("Cue", vec![("count", T::u32(Little)), ("points", T::array(point, E::field("count")))])
}

/// A LIST holds a type and then more chunks, of whichever flavour that type says.
fn list() -> T {
    T::structure(
        "List",
        vec![
            ("type", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("items", T::repeat(T::Named("ListItem".into()), Until::End)),
        ],
    )
}

fn list_item() -> T {
    let pad = E::field("size").sub(E::field("size").div(E::lit(2)).mul(E::lit(2)));
    // A LIST member is a chunk like any other, so it is called one.
    T::structure(
        "Chunk",
        vec![
            ("id", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
            ("size", T::u32(Little)),
            (
                "body",
                T::sized(
                    E::field("size"),
                    T::switch(
                        E::field("id"),
                        vec![
                            (cc("labl"), labelled()),
                            (cc("note"), labelled()),
                        ],
                        // INFO items are NUL-terminated Latin-1 text.
                        T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Latin1),
                    ),
                ),
            ),
            ("pad", T::bytes(pad)),
        ],
    )
}

fn labelled() -> T {
    T::structure(
        "Label",
        vec![
            ("cue_id", T::u32(Little)),
            ("text", T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Latin1)),
        ],
    )
}

fn ds64() -> T {
    T::structure(
        "DataSize64",
        vec![
            ("riff_size", T::u64(Little)),
            ("data_size", T::u64(Little)),
            ("sample_count", T::u64(Little)),
            ("table_length", T::u32(Little)),
            ("table", T::bytes(E::Remaining)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn chunk_bytes(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    pub(super) fn sample() -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&1u16.to_le_bytes()); // pcm
        fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
        fmt.extend_from_slice(&500_000u32.to_le_bytes());
        fmt.extend_from_slice(&1_000_000u32.to_le_bytes());
        fmt.extend_from_slice(&2u16.to_le_bytes());
        fmt.extend_from_slice(&16u16.to_le_bytes());

        // A GUANO block, ending with a NUL inside the chunk size, as recorders write.
        let mut guano = b"GUANO|Version:1.0\nMake:Wildlife Acoustics, Inc.\nLoc Position:-26.46550 31.94508\n".to_vec();
        guano.push(0);

        let mut wamd = Vec::new();
        wamd.extend_from_slice(&1u16.to_le_bytes());
        wamd.extend_from_slice(&9u32.to_le_bytes());
        wamd.extend_from_slice(b"SM4BAT-FS");

        // One cue point, and a LIST of INFO text.
        let mut cue = 1u32.to_le_bytes().to_vec();
        cue.extend_from_slice(&7u32.to_le_bytes()); // id
        cue.extend_from_slice(&1000u32.to_le_bytes()); // position
        cue.extend_from_slice(b"data");
        cue.extend_from_slice(&0u32.to_le_bytes());
        cue.extend_from_slice(&0u32.to_le_bytes());
        cue.extend_from_slice(&1000u32.to_le_bytes()); // sample offset

        let mut list = b"INFO".to_vec();
        list.extend_from_slice(&chunk_bytes(b"IART", b"Wildlife Acoustics\0"));

        let mut body = b"WAVE".to_vec();
        body.extend_from_slice(&chunk_bytes(b"fmt ", &fmt));
        body.extend_from_slice(&chunk_bytes(b"data", &[0x11; 6]));
        body.extend_from_slice(&chunk_bytes(b"cue ", &cue));
        body.extend_from_slice(&chunk_bytes(b"LIST", &list));
        body.extend_from_slice(&chunk_bytes(b"guan", &guano));
        body.extend_from_slice(&chunk_bytes(b"wamd", &wamd));

        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn chunks_pad_to_even_and_guano_reads_as_lines() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(wav());
        assert_eq!(ev.node(&d, &[3]).unwrap().child_count, 6);

        let fmt = ev.node(&d, &[3, 0, 2]).unwrap();
        assert_eq!(fmt.type_name, "Format");
        assert_eq!(ev.node(&d, &[3, 0, 2, 0]).unwrap().value, Value::Enum { raw: 1, name: Some("pcm".into()), hex: true });
        assert_eq!(ev.node(&d, &[3, 0, 2, 2]).unwrap().value, Value::UInt(500_000));

        // The data chunk is six bytes, so no pad; guano is odd, so one.
        assert_eq!(ev.node(&d, &[3, 1, 3]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[3, 4, 3]).unwrap().size_bits, 8);

        // A cue point, then a LIST whose members are chunks like any other.
        assert_eq!(ev.node(&d, &[3, 2, 2, 0]).unwrap().value, Value::UInt(1));
        assert_eq!(ev.node(&d, &[3, 2, 2, 1, 0, 1]).unwrap().value, Value::UInt(1000));
        assert_eq!(ev.node(&d, &[3, 3, 2, 0]).unwrap().value, Value::Str("INFO".into()));
        let item = ev.node(&d, &[3, 3, 2, 1, 0]).unwrap();
        assert_eq!(item.type_name, "Chunk");
        assert_eq!(ev.node(&d, &[3, 3, 2, 1, 0, 0]).unwrap().value, Value::Str("IART".into()));
        assert_eq!(ev.node(&d, &[3, 3, 2, 1, 0, 2]).unwrap().value, Value::Str("Wildlife Acoustics".into()));

        // GUANO lines.
        let lines = ev.node(&d, &[3, 4, 2]).unwrap();
        assert_eq!(lines.child_count, 4); // three fields and the trailing NUL
        assert_eq!(ev.node(&d, &[3, 4, 2, 0]).unwrap().value, Value::Str("GUANO|Version:1.0".into()));
        assert_eq!(
            ev.node(&d, &[3, 4, 2, 2]).unwrap().value,
            Value::Str("Loc Position:-26.46550 31.94508".into())
        );

        // wamd is a stream of tagged items.
        let wamd = ev.node(&d, &[3, 5, 2, 0]).unwrap();
        assert_eq!(wamd.child_count, 3);
        assert_eq!(
            ev.node(&d, &[3, 5, 2, 0, 0]).unwrap().value,
            Value::Enum { raw: 1, name: Some("model".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[3, 5, 2, 0, 2]).unwrap().value, Value::Str("SM4BAT-FS".into()));
    }
}
