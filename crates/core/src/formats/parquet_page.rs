//! What a Parquet page holds, read the whole way and reported step by step.
//!
//! The template opens a page as far as a template can. It unpacks the codec,
//! it reads a dictionary page as the column's physical type, and it reads a
//! `DATA_PAGE_V2` whose values are plain or dictionary encoded. What it cannot
//! do is read a v1 data page at all, and the reason is a fact about the format
//! rather than a gap here: a v1 page writes its repetition and definition
//! levels only where the column's maximum level is above zero, and that
//! maximum is the count of OPTIONAL and REPEATED elements along the column's
//! path through the schema. The schema is a flat depth-first list at the other
//! end of the file and the column's place in it is a walk, not a lookup. So
//! four bytes at the front of a v1 page are a level run's length or they are
//! the first value, and only something that can walk the schema can say which.
//!
//! So this is handed what the schema says, in [`Column`], by a caller that can
//! walk it. It is the arrangement [`hdf5_chunk`](super::hdf5_chunk) has, for
//! the same reason and reported the same way: every step named, with what went
//! in and what came out, and then the values.
//!
//! ## What a page is made of
//!
//! A `DATA_PAGE` is repetition levels, definition levels, and values, in that
//! order, all three packed together as one run. Each level list is present
//! only when its maximum is above zero, and when present is a length-prefixed
//! RLE and bit-packed hybrid, except where the header says `BIT_PACKED`, which
//! is the older encoding with no length in front of it and the bits the other
//! way up. A `DATA_PAGE_V2` writes the two level lists outside the packed part
//! with their lengths in the header, always as the hybrid and never with a
//! length prefix. A `DICTIONARY_PAGE` is values and nothing else.
//!
//! The values are in one of eight encodings. PLAIN writes them as they are.
//! The two dictionary encodings write indices into the dictionary page, as a
//! width byte and then the hybrid. RLE is booleans, one bit each, behind a
//! four-byte length that is there in both page versions. The three
//! delta encodings pack differences at a width that changes every few dozen
//! values, and BYTE_STREAM_SPLIT takes every value apart and writes all the
//! first bytes, then all the second bytes, and so on, which compresses far
//! better for floating point and is unreadable without knowing how wide a
//! value is.
//!
//! ## The bit packing
//!
//! Everything packed in here is packed from the low bit of a byte upwards, and
//! runs on into the next byte the same way: a three-bit value that starts at
//! bit 6 has its low two bits in the top of one byte and its high bit in the
//! bottom of the next. That is the opposite of how bits are addressed in the
//! rest of this program, which is why the values are read here rather than
//! declared as fields: a field per value would name the right byte and the
//! wrong bits inside it.
//!
//! The one exception is the `BIT_PACKED` level encoding, deprecated in 2015
//! and still written by four of the sixteen sample files. It packs from the
//! *high* bit down and has no length in front of it, so it is measured here
//! rather than read: how many bytes it takes is fixed by the value count and
//! the width, which is all the step after it needs, and the levels themselves
//! are not reported. Saying so is better than reading it the other way up and
//! printing numbers that are not the levels.

use crate::codec::{self, Codec};

/// What [`StructDef::packed`](crate::template::StructDef::packed) calls this,
/// so the template can mark a page and the panel can find its way back here.
pub const PACKING: &str = "parquet_page";

/// How many values a panel is handed. A page holds tens of thousands; a row of
/// them says what kind of values these are, which is what a panel is for.
pub const VALUES_SHOWN: usize = 32;

/// The largest page this will unpack for a panel that is redrawn every time
/// the cursor moves.
pub const PACKED_LIMIT: usize = 16 << 20;

/// The physical types, by the number the footer writes.
pub const BOOLEAN: i64 = 0;
pub const INT32: i64 = 1;
pub const INT64: i64 = 2;
pub const INT96: i64 = 3;
pub const FLOAT: i64 = 4;
pub const DOUBLE: i64 = 5;
pub const BYTE_ARRAY: i64 = 6;
pub const FIXED_LEN_BYTE_ARRAY: i64 = 7;

/// The page types.
pub const DATA_PAGE: i64 = 0;
pub const DICTIONARY_PAGE: i64 = 2;
pub const DATA_PAGE_V2: i64 = 3;

/// The encodings, by the number the footer writes. There is no 1.
pub const PLAIN: i64 = 0;
pub const PLAIN_DICTIONARY: i64 = 2;
pub const RLE: i64 = 3;
pub const BIT_PACKED: i64 = 4;
pub const DELTA_BINARY_PACKED: i64 = 5;
pub const DELTA_LENGTH_BYTE_ARRAY: i64 = 6;
pub const DELTA_BYTE_ARRAY: i64 = 7;
pub const RLE_DICTIONARY: i64 = 8;
pub const BYTE_STREAM_SPLIT: i64 = 9;

/// One step of the reading, in the order it was done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// What was done: a codec's name, an encoding's name, `levels`.
    pub what: String,
    /// How many bytes it read, and how many it produced. A step that produces
    /// values rather than bytes says nothing came out, and says how many
    /// values in `note` instead.
    pub in_bytes: usize,
    pub out_bytes: usize,
    /// What it said, in words: how many values, at what width, how many
    /// levels and how high they went.
    pub note: String,
    /// Set when the step was not done at all: a v2 page whose `is_compressed`
    /// is false names its column's codec and was never run through it, and
    /// saying `snappy 127 to 127 bytes` there would read as if snappy had run.
    pub skipped: bool,
}

/// What the page turned out to hold, and what it took to get there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// How many bytes the page's payload is in the file, and how many the
    /// values came to once the codec was undone.
    pub packed_bytes: usize,
    pub decoded_bytes: usize,
    pub steps: Vec<Step>,
    /// The first values, as text.
    pub values: Vec<String>,
    /// How many values the page holds, of which `values` shows the first few.
    pub total: u64,
    /// What one value is called, so a panel can say `i32` rather than only
    /// show numbers.
    pub element_type: String,
    /// Why the reading stopped early, where it did.
    pub problem: Option<String>,
}

/// Everything about the column that a page does not carry itself.
///
/// `max_definition` and `max_repetition` are the counts of OPTIONAL-or-
/// REPEATED and REPEATED elements along the column's path through the schema,
/// which is the whole of what says whether a v1 page has levels in front of
/// its values. `type_length` is only for FIXED_LEN_BYTE_ARRAY.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Column {
    pub physical: i64,
    pub type_length: i64,
    pub codec: i64,
    pub max_definition: u32,
    pub max_repetition: u32,
}

/// What the page's own header said.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Header {
    pub page_type: i64,
    pub encoding: i64,
    pub definition_level_encoding: i64,
    pub repetition_level_encoding: i64,
    pub num_values: i64,
    /// v2 only: how many bytes of levels sit in front of the packed part.
    pub definition_levels_byte_length: i64,
    pub repetition_levels_byte_length: i64,
    /// v2 only, and true when the writer said nothing.
    pub is_compressed: bool,
}

/// The name a codec number goes by, and what it is here.
fn codec_of(id: i64) -> (&'static str, Option<Codec>) {
    match id {
        0 => ("uncompressed", Some(Codec::Stored)),
        1 => ("snappy", Some(Codec::Snappy)),
        2 => ("gzip", Some(Codec::Gzip)),
        3 => ("lzo", None),
        4 => ("brotli", Some(Codec::Brotli)),
        5 => ("lz4 (hadoop framed)", None),
        6 => ("zstd", Some(Codec::Zstd)),
        7 => ("lz4_raw", Some(Codec::Lz4Block)),
        _ => ("unknown codec", None),
    }
}

/// The name an encoding number goes by.
pub fn encoding_name(id: i64) -> &'static str {
    match id {
        PLAIN => "PLAIN",
        PLAIN_DICTIONARY => "PLAIN_DICTIONARY",
        RLE => "RLE",
        BIT_PACKED => "BIT_PACKED",
        DELTA_BINARY_PACKED => "DELTA_BINARY_PACKED",
        DELTA_LENGTH_BYTE_ARRAY => "DELTA_LENGTH_BYTE_ARRAY",
        DELTA_BYTE_ARRAY => "DELTA_BYTE_ARRAY",
        RLE_DICTIONARY => "RLE_DICTIONARY",
        BYTE_STREAM_SPLIT => "BYTE_STREAM_SPLIT",
        10 => "ALP",
        _ => "unknown encoding",
    }
}

/// What one value of this column is called.
pub fn type_name(column: &Column) -> String {
    match column.physical {
        BOOLEAN => "bool".into(),
        INT32 => "i32".into(),
        INT64 => "i64".into(),
        INT96 => "int96".into(),
        FLOAT => "f32".into(),
        DOUBLE => "f64".into(),
        BYTE_ARRAY => "byte array".into(),
        FIXED_LEN_BYTE_ARRAY => format!("{}-byte array", column.type_length.max(0)),
        _ => "unknown type".into(),
    }
}

/// How many bits an index or a level of this height needs. A maximum of 0
/// needs no bits at all: every value is the same value.
pub fn width_for(max: u32) -> u32 {
    if max == 0 {
        0
    } else {
        32 - max.leading_zeros()
    }
}

/// Read a page. `payload` is the bytes the page header counted, which for a
/// v2 page includes the levels in front of the packed part.
pub fn read(payload: &[u8], header: &Header, column: &Column) -> Page {
    let mut page = Page {
        packed_bytes: payload.len(),
        decoded_bytes: 0,
        steps: Vec::new(),
        values: Vec::new(),
        total: 0,
        element_type: type_name(column),
        problem: None,
    };
    if payload.len() > PACKED_LIMIT {
        let mb = PACKED_LIMIT / (1 << 20);
        page.problem = Some(format!("Not unpacked: the page is over this viewer's {mb} MB limit."));
        return page;
    }

    // A v2 page keeps its levels out of the packed run, so they are read
    // first, from the bytes as they are in the file.
    let (levels, packed) = split_levels(payload, header);
    let read_levels = |page: &mut Page, bytes: &[u8], prefixed: bool| {
        let mut at = 0usize;
        for (name, max, encoding) in [
            ("repetition levels", column.max_repetition, header.repetition_level_encoding),
            ("definition levels", column.max_definition, header.definition_level_encoding),
        ] {
            if max == 0 {
                continue;
            }
            let (count, used) = level_run(&bytes[at.min(bytes.len())..], max, encoding, header.num_values, prefixed);
            page.steps.push(Step {
                what: name.into(),
                in_bytes: used,
                out_bytes: 0,
                note: level_note(count, width_for(max), max, encoding),
                skipped: false,
            });
            at += used;
        }
        at
    };

    let mut level_bytes = 0usize;
    if header.page_type == DATA_PAGE_V2 {
        // Sized by the header, one list after the other, never prefixed.
        let mut at = 0usize;
        for (name, max, len) in [
            ("repetition levels", column.max_repetition, header.repetition_levels_byte_length.max(0) as usize),
            ("definition levels", column.max_definition, header.definition_levels_byte_length.max(0) as usize),
        ] {
            if len == 0 {
                continue;
            }
            let run = &levels[at.min(levels.len())..(at + len).min(levels.len())];
            let (count, _) = hybrid(run, width_for(max), header.num_values.max(0) as usize);
            page.steps.push(Step {
                what: name.into(),
                in_bytes: len,
                out_bytes: 0,
                note: level_note(count.len(), width_for(max), max, RLE),
                skipped: false,
            });
            at += len;
        }
    }

    // Then the codec, over what is left.
    let (name, which) = codec_of(column.codec);
    let compressed = header.page_type != DATA_PAGE_V2 || header.is_compressed;
    let bytes = if !compressed || which == Some(Codec::Stored) {
        page.steps.push(Step {
            what: name.into(),
            in_bytes: packed.len(),
            out_bytes: packed.len(),
            note: String::new(),
            skipped: !compressed,
        });
        packed.to_vec()
    } else {
        let Some(which) = which else {
            page.steps.push(Step {
                what: name.into(),
                in_bytes: packed.len(),
                out_bytes: 0,
                note: String::new(),
                skipped: false,
            });
            page.problem =
                Some(format!("Stopped at {name}: this viewer has no {name} decoder, so the values could not be read."));
            return page;
        };
        match codec::decode(which, packed) {
            Ok(out) => {
                page.steps.push(Step {
                    what: name.into(),
                    in_bytes: packed.len(),
                    out_bytes: out.len(),
                    note: String::new(),
                    skipped: false,
                });
                out
            }
            Err(why) => {
                page.steps.push(Step {
                    what: name.into(),
                    in_bytes: packed.len(),
                    out_bytes: 0,
                    note: String::new(),
                    skipped: false,
                });
                page.problem = Some(format!("Stopped at {name}: {}.", refusal(why)));
                return page;
            }
        }
    };
    page.decoded_bytes = bytes.len();

    // A v1 page's levels are inside what the codec produced.
    if header.page_type == DATA_PAGE {
        level_bytes = read_levels(&mut page, &bytes, true);
    }
    let values = &bytes[level_bytes.min(bytes.len())..];
    values_of(&mut page, values, header, column);
    page
}

/// `1 bit each` or `3 bits each`, since a level of one bit is the ordinary
/// case and `1 bits` is the kind of thing a reader stops trusting.
fn bits(n: u32) -> String {
    if n == 1 {
        "1 bit each".into()
    } else {
        format!("{n} bits each")
    }
}

/// What a list of levels says about itself: how many there are, how wide one
/// is, how high they go, and which encoding they are in.
///
/// The maximum is the spec's own number and is what decides the width, so both
/// are worth saying: a reader who sees `1 bit each, max level 1` can check the
/// arithmetic, and one who sees only a width cannot.
///
/// `BIT_PACKED` is the deprecated level encoding, and its levels are counted
/// from the header rather than read: it packs from the high bit of each byte
/// down, which is the other way up from everything else here, so what this
/// knows is how many bytes it takes and not what is in them.
fn level_note(count: usize, width: u32, max: u32, encoding: i64) -> String {
    let how = encoding_name(encoding);
    if encoding == BIT_PACKED {
        return format!("{count} levels, {}, max level {max}, {how}, counted from the header, not read", bits(width));
    }
    format!("{count} levels, {}, max level {max}, {how}", bits(width))
}

/// The word a refusal goes by, in a sentence rather than as a tag.
fn refusal(why: codec::Refusal) -> &'static str {
    match why {
        codec::Refusal::TooLarge => "too large to unpack (over 64 MiB)",
        codec::Refusal::Failed => "unpacking failed",
        codec::Refusal::Unaligned => "not on a byte boundary",
        codec::Refusal::Settings => "the file doesn't say how this was packed",
    }
}

/// A v2 page's levels and the packed run behind them; for any other page the
/// levels are empty and the run is the whole payload.
fn split_levels<'a>(payload: &'a [u8], header: &Header) -> (&'a [u8], &'a [u8]) {
    if header.page_type != DATA_PAGE_V2 {
        return (&[], payload);
    }
    let n = (header.definition_levels_byte_length.max(0) + header.repetition_levels_byte_length.max(0)) as usize;
    let n = n.min(payload.len());
    payload.split_at(n)
}

/// One v1 level list: four bytes of length and then a hybrid, or, where the
/// header says `BIT_PACKED`, the older packing with no length in front of it.
///
/// Returns how many levels were read and how many bytes they took.
fn level_run(bytes: &[u8], max: u32, encoding: i64, num_values: i64, prefixed: bool) -> (usize, usize) {
    let width = width_for(max);
    let want = num_values.max(0) as usize;
    if encoding == BIT_PACKED {
        // Deprecated in 2015 and still in four of the sixteen samples. No
        // length in front of it, and the values are packed from the high bit
        // of each byte downwards, so it is measured rather than read: eight
        // values to `width` bytes, rounded up.
        let used = (want * width as usize).div_ceil(8);
        return (want, used.min(bytes.len()));
    }
    if !prefixed {
        let (values, used) = hybrid(bytes, width, want);
        return (values.len(), used);
    }
    if bytes.len() < 4 {
        return (0, bytes.len());
    }
    let len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let end = (4 + len).min(bytes.len());
    let (values, _) = hybrid(&bytes[4..end], width, want);
    (values.len(), end)
}

/// Read the values and say what they were.
fn values_of(page: &mut Page, bytes: &[u8], header: &Header, column: &Column) {
    let want = header.num_values.max(0) as usize;
    // A dictionary page is plain whatever its header calls the encoding:
    // PLAIN_DICTIONARY named the pages that *use* a dictionary, and a
    // dictionary page itself has always been plain.
    let encoding = if header.page_type == DICTIONARY_PAGE { PLAIN } else { header.encoding };
    let name = encoding_name(encoding);
    match encoding {
        PLAIN => {
            let (values, total) = plain(bytes, column, want);
            page.steps.push(Step {
                what: name.into(),
                in_bytes: bytes.len(),
                out_bytes: 0,
                note: format!("{total} values, as {}", page.element_type),
                skipped: false,
            });
            page.total = total as u64;
            page.values = values;
        }
        PLAIN_DICTIONARY | RLE_DICTIONARY => {
            let Some((&width, rest)) = bytes.split_first() else {
                page.problem =
                    Some(format!("Stopped at {name}: the page ends before the bit width byte that starts the indices."));
                return;
            };
            let (indices, used) = hybrid(rest, u32::from(width), want);
            page.steps.push(Step {
                what: name.into(),
                in_bytes: used + 1,
                out_bytes: 0,
                note: format!("{} dictionary indices, {}", indices.len(), bits(u32::from(width))),
                skipped: false,
            });
            page.total = indices.len() as u64;
            page.element_type = "dictionary index".into();
            page.values = indices.iter().take(VALUES_SHOWN).map(u64::to_string).collect();
        }
        RLE => {
            // Four bytes of length in front of the runs, in both page
            // versions. Unlike a v2 page's levels, which the page header
            // sizes, the values carry their own length here.
            if bytes.len() < 4 {
                page.problem =
                    Some(format!("Stopped at {name}: the page is under 4 bytes, too short for the length that starts it."));
                return;
            }
            let len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
            let end = (4 + len).min(bytes.len());
            let (values, _) = hybrid(&bytes[4..end], 1, want);
            let used = end;
            page.steps.push(Step {
                what: name.into(),
                in_bytes: used,
                out_bytes: 0,
                note: format!("{} bool values, 1 bit each, after a 4-byte length", values.len()),
                skipped: false,
            });
            page.total = values.len() as u64;
            page.element_type = "bool".into();
            page.values = values.iter().take(VALUES_SHOWN).map(|v| if *v == 0 { "false" } else { "true" }.to_string()).collect();
        }
        DELTA_BINARY_PACKED => delta_values(page, bytes, want, name),
        DELTA_LENGTH_BYTE_ARRAY => {
            let (lengths, used) = delta(bytes, want);
            page.steps.push(Step {
                what: "lengths (DELTA_BINARY_PACKED)".into(),
                in_bytes: used,
                out_bytes: 0,
                note: format!("{} lengths, one per value", lengths.values.len()),
                skipped: false,
            });
            let mut at = used;
            let mut out = Vec::new();
            let mut total = 0u64;
            for len in &lengths.values {
                let len = (*len).clamp(0, i64::MAX as i128) as usize;
                if at + len > bytes.len() {
                    break;
                }
                if out.len() < VALUES_SHOWN {
                    out.push(show_bytes(&bytes[at..at + len]));
                }
                at += len;
                total += 1;
            }
            page.steps.push(Step {
                what: name.into(),
                in_bytes: at - used,
                out_bytes: 0,
                note: format!("{total} values, as byte array, sized by the lengths above"),
                skipped: false,
            });
            page.total = total;
            page.values = out;
        }
        DELTA_BYTE_ARRAY => {
            // Prefix lengths, then suffixes as a DELTA_LENGTH_BYTE_ARRAY: each
            // value shares its first `prefix` bytes with the one before it,
            // which is what makes a sorted column of paths cheap.
            let (prefixes, used) = delta(bytes, want);
            let (suffixes, used2) = delta(&bytes[used.min(bytes.len())..], want);
            page.steps.push(Step {
                what: "prefix lengths (DELTA_BINARY_PACKED)".into(),
                in_bytes: used,
                out_bytes: 0,
                note: format!("{} prefix lengths", prefixes.values.len()),
                skipped: false,
            });
            page.steps.push(Step {
                what: "suffix lengths (DELTA_BINARY_PACKED)".into(),
                in_bytes: used2,
                out_bytes: 0,
                note: format!("{} suffix lengths", suffixes.values.len()),
                skipped: false,
            });
            let mut at = used + used2;
            let mut last: Vec<u8> = Vec::new();
            let mut out = Vec::new();
            let mut total = 0u64;
            for (i, len) in suffixes.values.iter().enumerate() {
                let len = (*len).clamp(0, i64::MAX as i128) as usize;
                let prefix = prefixes.values.get(i).copied().unwrap_or(0).clamp(0, i64::MAX as i128) as usize;
                if at + len > bytes.len() || prefix > last.len() {
                    break;
                }
                let mut value = last[..prefix].to_vec();
                value.extend_from_slice(&bytes[at..at + len]);
                if out.len() < VALUES_SHOWN {
                    out.push(show_bytes(&value));
                }
                last = value;
                at += len;
                total += 1;
            }
            page.steps.push(Step {
                what: name.into(),
                in_bytes: at - used - used2,
                out_bytes: 0,
                note: format!("{total} values, as byte array, each a shared prefix and its own suffix"),
                skipped: false,
            });
            page.total = total;
            page.values = out;
        }
        BYTE_STREAM_SPLIT => {
            let width = value_width(column);
            if bytes.is_empty() {
                page.problem = Some(format!("Stopped at {name}: nothing came out of the codec."));
                return;
            }
            if width == 0 {
                page.problem = Some(format!(
                    "Stopped at {name}: this column is {}, which has no fixed width, so the streams cannot be split.",
                    page.element_type
                ));
                return;
            }
            let count = bytes.len() / width;
            let mut joined = vec![0u8; count * width];
            for (k, stream) in bytes.chunks(count).take(width).enumerate() {
                for (i, byte) in stream.iter().enumerate() {
                    joined[i * width + k] = *byte;
                }
            }
            let (values, total) = plain(&joined, column, count);
            page.steps.push(Step {
                what: name.into(),
                in_bytes: bytes.len(),
                out_bytes: joined.len(),
                note: format!(
                    "{total} values, as {}, from {width} streams of {count} bytes",
                    page.element_type
                ),
                skipped: false,
            });
            page.total = total as u64;
            page.values = values;
        }
        _ => {
            page.steps.push(Step {
                what: name.into(),
                in_bytes: bytes.len(),
                out_bytes: 0,
                note: String::new(),
                skipped: false,
            });
            page.problem =
                Some(format!("Stopped at {name}: this viewer has no reader for this encoding, so the values could not be read."));
        }
    }
}

/// How many bytes one value of a fixed-width type takes, or zero for the types
/// that have no such width.
fn value_width(column: &Column) -> usize {
    match column.physical {
        INT32 | FLOAT => 4,
        INT64 | DOUBLE => 8,
        INT96 => 12,
        FIXED_LEN_BYTE_ARRAY => column.type_length.max(0) as usize,
        _ => 0,
    }
}

/// PLAIN values: fixed-width ones back to back, byte arrays each behind a
/// four-byte length, booleans a bit each from the low bit up.
fn plain(bytes: &[u8], column: &Column, want: usize) -> (Vec<String>, usize) {
    if column.physical == BOOLEAN {
        let total = want.min(bytes.len() * 8);
        let shown = (0..total.min(VALUES_SHOWN))
            .map(|i| if bytes[i / 8] >> (i % 8) & 1 == 1 { "true".into() } else { "false".into() })
            .collect();
        return (shown, total);
    }
    if column.physical == BYTE_ARRAY {
        let mut at = 0usize;
        let mut out = Vec::new();
        let mut total = 0usize;
        while at + 4 <= bytes.len() {
            let len = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
            if at + 4 + len > bytes.len() {
                break;
            }
            if out.len() < VALUES_SHOWN {
                out.push(show_bytes(&bytes[at + 4..at + 4 + len]));
            }
            at += 4 + len;
            total += 1;
        }
        return (out, total);
    }
    let width = value_width(column);
    if width == 0 {
        return (Vec::new(), 0);
    }
    let total = bytes.len() / width;
    let shown = bytes
        .chunks_exact(width)
        .take(VALUES_SHOWN)
        .map(|v| one_value(v, column.physical))
        .collect();
    (shown, total)
}

/// One fixed-width value, as the text a reader wants to see.
fn one_value(bytes: &[u8], physical: i64) -> String {
    match physical {
        INT32 => i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).to_string(),
        INT64 => {
            let mut v = [0u8; 8];
            v.copy_from_slice(&bytes[..8]);
            i64::from_le_bytes(v).to_string()
        }
        FLOAT => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).to_string(),
        DOUBLE => {
            let mut v = [0u8; 8];
            v.copy_from_slice(&bytes[..8]);
            f64::from_le_bytes(v).to_string()
        }
        // INT96 is three little-endian words Impala wrote a nanosecond
        // timestamp into: the first two are the nanoseconds of the day and the
        // third is a Julian day number. Shown as its bytes, since nothing here
        // declares it as a moment.
        _ => show_bytes(bytes),
    }
}

/// Bytes as a reader wants them: the text where it is text, and hex where it
/// is not.
fn show_bytes(bytes: &[u8]) -> String {
    let printable = bytes.iter().all(|b| (0x20..0x7f).contains(b));
    if printable && !bytes.is_empty() {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join("")
    }
}

/// The RLE and bit-packed hybrid, at a width something outside it fixed.
///
/// A run starts with an unsigned varint. Its low bit says which of the two
/// kinds follows; the rest is a count. An even header repeats one value,
/// written in as many whole bytes as the width needs, `count` times. An odd
/// header introduces `count` groups of eight values packed at the width.
///
/// Stops at `want` values, or when the bytes run out. Returns the values and
/// how many bytes were read.
pub fn hybrid(bytes: &[u8], width: u32, want: usize) -> (Vec<u64>, usize) {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() && out.len() < want {
        let Some((header, used)) = varint(&bytes[at..]) else { break };
        at += used;
        let count = (header >> 1) as usize;
        if header & 1 == 0 {
            // A width of zero writes no bytes: every value is the same value.
            let take = width.div_ceil(8) as usize;
            if at + take > bytes.len() {
                break;
            }
            let mut value = 0u64;
            for (i, byte) in bytes[at..at + take].iter().enumerate() {
                value |= u64::from(*byte) << (8 * i);
            }
            at += take;
            let n = count.min(want - out.len());
            out.extend(std::iter::repeat_n(value, n));
            if n < count {
                break;
            }
        } else {
            let values = count * 8;
            let take = (values * width as usize).div_ceil(8);
            if at + take > bytes.len() {
                break;
            }
            let run = &bytes[at..at + take];
            for i in 0..values.min(want - out.len()) {
                out.push(bits_low_first(run, i * width as usize, width));
            }
            at += take;
        }
    }
    (out, at)
}

/// `width` bits starting at bit `from`, counting from the low bit of the first
/// byte upwards and running on into the next byte the same way.
fn bits_low_first(bytes: &[u8], from: usize, width: u32) -> u64 {
    let mut value = 0u64;
    for i in 0..width as usize {
        let bit = from + i;
        let Some(byte) = bytes.get(bit / 8) else { break };
        value |= u64::from(byte >> (bit % 8) & 1) << i;
    }
    value
}

/// An unsigned LEB128: seven bits a byte, low group first, the top bit set on
/// every byte but the last.
fn varint(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for (i, byte) in bytes.iter().take(10).enumerate() {
        value |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None
}

/// A zigzag varint: the sign in the low bit, so a small negative number is a
/// small number.
fn zigzag(bytes: &[u8]) -> Option<(i128, usize)> {
    let (raw, used) = varint(bytes)?;
    Some(((raw >> 1) as i128 ^ -((raw & 1) as i128), used))
}

/// What one DELTA_BINARY_PACKED stream said, and what it took to say it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delta {
    pub block_size: u64,
    pub miniblocks: u64,
    pub declared: u64,
    pub values: Vec<i128>,
    /// The bit width of every miniblock read, in order, which is the whole of
    /// what a delta stream costs and the thing `delta_binary_packed.parquet`
    /// was built to show.
    pub widths: Vec<u8>,
}

/// DELTA_BINARY_PACKED: a header, then blocks of miniblocks.
///
/// The header is the block size, how many miniblocks a block holds, how many
/// values there are altogether, and the first value. Every value after it is
/// written as the difference from the one before, and every block subtracts
/// its own smallest difference before packing, so a run of equal values costs
/// nothing but a zero width. A block gives one bit width per miniblock and
/// then the miniblocks, each packed at its own width from the low bit up.
///
/// Returns the stream and how many bytes it took, so a caller that has more
/// behind it, as DELTA_LENGTH_BYTE_ARRAY does, knows where to carry on.
pub fn delta(bytes: &[u8], want: usize) -> (Delta, usize) {
    let mut out = Delta::default();
    let mut at = 0usize;
    let take = |at: &mut usize| {
        let got = varint(&bytes[(*at).min(bytes.len())..]);
        match got {
            Some((v, used)) => {
                *at += used;
                Some(v)
            }
            None => None,
        }
    };
    let (Some(block_size), Some(miniblocks), Some(declared)) = (take(&mut at), take(&mut at), take(&mut at)) else {
        return (out, at);
    };
    out.block_size = block_size;
    out.miniblocks = miniblocks;
    out.declared = declared;
    let Some((first, used)) = zigzag(&bytes[at.min(bytes.len())..]) else { return (out, at) };
    at += used;
    let wanted = want.min(declared as usize).max(1);
    out.values.push(first);
    if miniblocks == 0 || block_size == 0 || block_size % miniblocks != 0 {
        return (out, at);
    }
    let per_miniblock = (block_size / miniblocks) as usize;
    let mut value = first;
    while out.values.len() < wanted && at < bytes.len() {
        let Some((min_delta, used)) = zigzag(&bytes[at..]) else { break };
        at += used;
        let widths_at = at;
        if at + miniblocks as usize > bytes.len() {
            break;
        }
        at += miniblocks as usize;
        for m in 0..miniblocks as usize {
            let width = bytes[widths_at + m];
            out.widths.push(width);
            let take = (per_miniblock * width as usize).div_ceil(8);
            if at + take > bytes.len() {
                return (out, bytes.len());
            }
            let run = &bytes[at..at + take];
            at += take;
            for i in 0..per_miniblock {
                if out.values.len() >= wanted {
                    break;
                }
                let delta = bits_low_first(run, i * width as usize, u32::from(width));
                value = value.wrapping_add(min_delta).wrapping_add(delta as i128);
                out.values.push(value);
            }
            if out.values.len() >= wanted {
                break;
            }
        }
    }
    (out, at)
}

/// A DELTA_BINARY_PACKED page, reported: the header, the widths its miniblocks
/// used, and the values.
fn delta_values(page: &mut Page, bytes: &[u8], want: usize, name: &str) {
    let (stream, used) = delta(bytes, want.max(1));
    let widest = stream.widths.iter().copied().max().unwrap_or(0);
    page.steps.push(Step {
        what: name.into(),
        in_bytes: used,
        out_bytes: 0,
        note: format!(
            "{} values, block size {}, {} miniblocks per block, delta bit width up to {widest}",
            stream.values.len(),
            stream.block_size,
            stream.miniblocks
        ),
        skipped: false,
    });
    page.total = stream.values.len() as u64;
    page.values = stream.values.iter().take(VALUES_SHOWN).map(i128::to_string).collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unsigned varint, for building a fixture by hand.
    fn uvarint(mut n: u64) -> Vec<u8> {
        let mut out = Vec::new();
        while n >= 128 {
            out.push(n as u8 | 128);
            n >>= 7;
        }
        out.push(n as u8);
        out
    }

    fn zz(n: i64) -> Vec<u8> {
        uvarint(((n << 1) ^ (n >> 63)) as u64)
    }

    /// A run that repeats one value: an even header, then the value in as many
    /// whole bytes as the width needs.
    #[test]
    fn an_rle_run_repeats_one_value() {
        // Header 5 << 1 = 10, then the value 2 in one byte, at width 3.
        let (values, used) = hybrid(&[10, 2], 3, 100);
        assert_eq!(values, vec![2, 2, 2, 2, 2]);
        assert_eq!(used, 2);
        // At width 9 the value takes two bytes, low byte first.
        let (values, used) = hybrid(&[4, 0x01, 0x01], 9, 100);
        assert_eq!(values, vec![257, 257]);
        assert_eq!(used, 3);
    }

    /// A width of zero writes no value at all, which is what a column with one
    /// distinct value comes to.
    #[test]
    fn a_zero_width_run_reads_nothing_and_repeats_it() {
        let (values, used) = hybrid(&[8], 0, 100);
        assert_eq!(values, vec![0, 0, 0, 0]);
        assert_eq!(used, 1);
    }

    /// A bit-packed run: an odd header giving the number of groups of eight,
    /// then the values packed from the low bit of each byte upwards.
    #[test]
    fn a_bit_packed_run_reads_from_the_low_bit_up() {
        // One group of eight at two bits: 0b11100100 is 0, 1, 2, 3 from the
        // bottom, and the second byte is four zeroes.
        let (values, used) = hybrid(&[3, 0b1110_0100, 0], 2, 100);
        assert_eq!(values, vec![0, 1, 2, 3, 0, 0, 0, 0]);
        assert_eq!(used, 3);
        // Three bits each, so a value straddles a byte boundary: 0, 1, 2, 3,
        // 4, 5, 6, 7 packs to 88 c6 fa.
        let (values, _) = hybrid(&[3, 0x88, 0xc6, 0xfa], 3, 100);
        assert_eq!(values, vec![0, 1, 2, 3, 4, 5, 6, 7]);
    }

    /// Both kinds in one stream, and a count that stops it early.
    #[test]
    fn runs_follow_one_another_and_stop_at_the_count() {
        let bytes = [10u8, 2, 3, 0b1110_0100, 0];
        let (values, used) = hybrid(&bytes, 2, 100);
        assert_eq!(values, vec![2, 2, 2, 2, 2, 0, 1, 2, 3, 0, 0, 0, 0]);
        assert_eq!(used, 5);
        // Asked for six, it reads the repeated run and one packed value.
        let (values, _) = hybrid(&bytes, 2, 6);
        assert_eq!(values, vec![2, 2, 2, 2, 2, 0]);
    }

    /// A stream that ends in the middle of a run gives what it had.
    #[test]
    fn a_truncated_run_gives_what_was_there() {
        assert_eq!(hybrid(&[3, 0xff], 8, 100).0, Vec::<u64>::new());
        assert_eq!(hybrid(&[10], 8, 100).0, Vec::<u64>::new());
    }

    /// One delta block, built by hand: five values one apart, so every
    /// difference is the block's own minimum and every miniblock is zero bits
    /// wide.
    #[test]
    fn a_delta_block_of_equal_differences_costs_no_bits() {
        let mut bytes = Vec::new();
        bytes.extend(uvarint(128)); // block size
        bytes.extend(uvarint(4)); // miniblocks per block
        bytes.extend(uvarint(5)); // values altogether
        bytes.extend(zz(1)); // the first value
        bytes.extend(zz(1)); // the block's smallest difference
        bytes.extend([0, 0, 0, 0]); // one width per miniblock
        let (stream, used) = delta(&bytes, 100);
        assert_eq!(stream.block_size, 128);
        assert_eq!(stream.miniblocks, 4);
        assert_eq!(stream.declared, 5);
        assert_eq!(stream.values, vec![1, 2, 3, 4, 5]);
        assert_eq!(used, bytes.len());
        assert_eq!(stream.widths, vec![0]);
    }

    /// A block whose differences are not all the same: the minimum comes off
    /// every one of them and what is left is packed at the width given.
    #[test]
    fn a_delta_block_packs_what_is_left_after_the_minimum() {
        let mut bytes = Vec::new();
        bytes.extend(uvarint(32)); // block size
        bytes.extend(uvarint(1)); // one miniblock of 32
        bytes.extend(uvarint(4)); // four values
        bytes.extend(zz(10)); // the first
        bytes.extend(zz(-2)); // the smallest difference
        bytes.push(2); // two bits a difference
        // Differences 0, 1, 3 above the minimum, so values 8, 7, 8.
        let mut packed = vec![0u8; (32 * 2usize).div_ceil(8)];
        for (i, v) in [0u8, 1, 3].into_iter().enumerate() {
            packed[i * 2 / 8] |= v << (i * 2 % 8);
        }
        bytes.extend(packed);
        let (stream, _) = delta(&bytes, 100);
        assert_eq!(stream.values, vec![10, 8, 7, 8]);
        assert_eq!(stream.widths, vec![2]);
    }

    /// A whole v1 page, built by hand: no levels, plain four-byte integers.
    #[test]
    fn a_page_with_no_levels_is_all_values() {
        let mut body = Vec::new();
        for v in [1i32, 2, 3] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        let header = Header { page_type: DATA_PAGE, encoding: PLAIN, num_values: 3, ..Header::default() };
        let column = Column { physical: INT32, codec: 0, ..Column::default() };
        let page = read(&body, &header, &column);
        assert_eq!(page.problem, None);
        assert_eq!(page.values, vec!["1", "2", "3"]);
        assert_eq!(page.total, 3);
        // The codec, then the values: no level steps, because the column has
        // no level above zero.
        assert_eq!(page.steps.len(), 2);
        assert_eq!(page.steps[0].what, "uncompressed");
        assert_eq!(page.steps[1].what, "PLAIN");
    }

    /// The same page from an optional column: four bytes of length and a
    /// hybrid run of definition levels come first, and the values start after
    /// them.
    #[test]
    fn an_optional_column_writes_its_definition_levels_first() {
        let levels = [3u8, 1]; // one run: three values at level 1
        let mut body = (levels.len() as u32).to_le_bytes().to_vec();
        body.extend_from_slice(&levels);
        for v in [7i32, 8, 9] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        let header = Header {
            page_type: DATA_PAGE,
            encoding: PLAIN,
            definition_level_encoding: RLE,
            num_values: 3,
            ..Header::default()
        };
        let column = Column { physical: INT32, max_definition: 1, ..Column::default() };
        let page = read(&body, &header, &column);
        assert_eq!(page.values, vec!["7", "8", "9"]);
        assert_eq!(page.steps.len(), 3);
        assert_eq!(page.steps[1].what, "definition levels");
        assert_eq!(page.steps[1].in_bytes, 6, "four bytes of length and the run");
    }

    /// BYTE_STREAM_SPLIT puts every first byte together, then every second
    /// byte, and so on, so a column of floats that differ only in their low
    /// bits packs into long runs of the same byte.
    #[test]
    fn byte_stream_split_puts_a_value_back_together() {
        let values = [1.0f32, 2.0, 3.0];
        let mut split = vec![0u8; 12];
        for (i, v) in values.iter().enumerate() {
            for (k, byte) in v.to_le_bytes().into_iter().enumerate() {
                split[k * 3 + i] = byte;
            }
        }
        let header = Header { page_type: DATA_PAGE, encoding: BYTE_STREAM_SPLIT, num_values: 3, ..Header::default() };
        let column = Column { physical: FLOAT, ..Column::default() };
        let page = read(&split, &header, &column);
        assert_eq!(page.values, vec!["1", "2", "3"]);
        assert_eq!(page.total, 3);
    }

    /// A codec nothing here reads says so and stops, rather than handing on
    /// bytes it did not open.
    #[test]
    fn a_codec_with_no_decoder_says_so() {
        let header = Header { page_type: DATA_PAGE, encoding: PLAIN, num_values: 1, ..Header::default() };
        // 3 is LZO.
        let column = Column { physical: INT32, codec: 3, ..Column::default() };
        let page = read(&[0; 8], &header, &column);
        assert!(page.problem.as_deref().is_some_and(|p| p.contains("lzo")), "{:?}", page.problem);
        assert_eq!(page.steps.len(), 1);
    }

    #[test]
    fn a_level_is_as_wide_as_its_maximum_needs() {
        assert_eq!(width_for(0), 0);
        assert_eq!(width_for(1), 1);
        assert_eq!(width_for(2), 2);
        assert_eq!(width_for(3), 2);
        assert_eq!(width_for(4), 3);
        assert_eq!(width_for(255), 8);
    }
}
