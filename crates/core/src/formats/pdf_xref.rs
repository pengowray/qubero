//! Taking a cross-reference stream apart into the rows it stands for.
//!
//! [`pdf`](super::pdf) says where the stream object is and reads its
//! dictionary, and leaves the bytes beside it as bytes, because that is what a
//! template can say: every field a template places is a run of the file, and
//! these rows are not in the file. They are what comes out of the file after it
//! is decompressed. This is the other half, and it is the same arrangement
//! [`ggml_quant`](super::ggml_quant) has with `ggml`.
//!
//! Three things stand between the bytes and the rows.
//!
//! The first is the compression. `/Filter /FlateDecode` is a zlib stream, which
//! is deflate with a two-byte header and a checksum after it. Writers exist
//! that leave the header off, so a stream that will not open as zlib is tried
//! again as raw deflate before it is given up on.
//!
//! The second is the predictor. A `/DecodeParms` of `/Predictor 12` means the
//! rows were run through PNG's row filters before being compressed, which is
//! what makes a table of mostly-similar offsets compress well. Every row then
//! carries one extra byte in front saying which of the five filters was used on
//! it, and undoing them has to run in order, since each row is written as a
//! difference from the one above.
//!
//! The third is the packing. `/W` gives the width in bytes of each of a row's
//! three numbers, so a `/W [1 2 1]` row is four bytes and a `/W [1 3 2]` row is
//! six. A width of zero means the number is not written at all and takes its
//! default, which is why a table of nothing but in-use entries can write `/W
//! [0 4 2]` and save a byte a row. The numbers are big-endian, which is the one
//! thing about this format that needs no explaining.
//!
//! What the three numbers mean depends on the first of them. Type 1 is what a
//! classic table's `n` entry was: an offset and a generation. Type 0 is what
//! `f` was. Type 2 is new, and is the point of the whole exercise: the object
//! is not in the file on its own at all, it is inside an *object stream*
//! together with others, and the two numbers are which stream and how far down
//! it. That is where a modern PDF puts most of its small objects, and it is why
//! the offsets a reader gets out of a cross-reference stream do not account for
//! anything like the whole file.

/// What [`StructDef::packed`](crate::template::StructDef::packed) calls this,
/// so the template can mark the object whose contents these are and the panel
/// can find its way back here.
pub const PACKING: &str = "pdf_xref";

/// What a row says the object is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Not in the file. The row's numbers are the next free object and the
    /// generation the next object to take this number will have.
    Free,
    /// Written in the file at an offset of its own, as every object in a
    /// classic table is.
    InFile,
    /// Inside an object stream with other objects, compressed along with them.
    InStream,
    /// A type the spec has not defined. The row is kept and its numbers are
    /// shown, because a reader looking at an unknown row wants to see it.
    Other(u64),
}

impl Kind {
    fn of(n: u64) -> Kind {
        match n {
            0 => Kind::Free,
            1 => Kind::InFile,
            2 => Kind::InStream,
            other => Kind::Other(other),
        }
    }

    /// What this kind calls its two numbers, in order.
    pub fn field_names(self) -> (&'static str, &'static str) {
        match self {
            Kind::Free => ("next free object", "generation"),
            Kind::InFile => ("offset", "generation"),
            Kind::InStream => ("in object", "index"),
            Kind::Other(_) => ("field 2", "field 3"),
        }
    }

    /// The number the row actually held, which for a type nobody has defined
    /// is the only thing there is to say about it.
    pub fn raw(self) -> u64 {
        match self {
            Kind::Free => 0,
            Kind::InFile => 1,
            Kind::InStream => 2,
            Kind::Other(n) => n,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Free => "free",
            Kind::InFile => "in file",
            Kind::InStream => "in an object stream",
            Kind::Other(_) => "unknown",
        }
    }
}

/// One row: which object it is for, what it says the object is, and the two
/// numbers whose meaning that decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub object: u64,
    pub kind: Kind,
    pub second: u64,
    pub third: u64,
    /// Where this row starts in the decompressed bytes, so a reader can be
    /// told which of them it came from. Not an offset in the file: nothing
    /// here is.
    pub at: usize,
}

/// A decoded table, and what the dictionary said about how to decode it.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub rows: Vec<Row>,
    /// The widths from `/W`, which say how the row above was split.
    pub widths: [u32; 3],
    /// The PNG predictor from `/DecodeParms`, where there was one.
    pub predictor: Option<u32>,
    /// How many bytes the rows came to once decompressed.
    pub decoded_bytes: usize,
    /// Bytes left over after the last whole row, which a well-formed stream
    /// has none of.
    pub trailing_bytes: usize,
}

/// Why a stream could not be taken apart. Each of these is a thing a reader
/// wants told rather than a blank panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A filter this does not implement, named as the dictionary named it.
    Filter(String),
    /// No `/W`, or one that is not three numbers.
    Widths,
    /// The compressed bytes would not open, as zlib or as raw deflate.
    Compressed,
    /// A `/Predictor` other than the PNG ones, which is the TIFF predictor
    /// nothing writes here.
    Predictor(u32),
    /// A row is zero bytes wide, so the stream describes no rows at all.
    EmptyRows,
}

impl Problem {
    /// One sentence, standing on its own. Only the last of these means the
    /// table is empty; the rest mean the rows are there and were not read, and
    /// a message that led with "no rows" would be saying something untrue
    /// about the file.
    pub fn as_str(&self) -> String {
        match self {
            Problem::Filter(f) => format!("{f} compression is not supported."),
            Problem::Widths => "The stream dictionary has no /W, so the row widths are unknown.".into(),
            Problem::Compressed => "Decompression failed: the data is not valid zlib.".into(),
            Problem::Predictor(p) => {
                format!("/Predictor {p} is not supported; only the PNG predictors (10-15) are.")
            }
            Problem::EmptyRows => "The /W widths sum to 0 bytes, so the stream holds no rows.".into(),
        }
    }
}

/// Take a cross-reference stream apart, given its dictionary as text and the
/// bytes between `stream` and `endstream`.
///
/// The bytes may have the line ending after `stream` still on the front and the
/// one before `endstream` still on the end, which is how the template hands
/// them over; both are stepped over here rather than there, since only this
/// knows that what follows has to start at a byte the compression recognises.
pub fn decode(dict: &str, data: &[u8]) -> Result<Table, Problem> {
    let widths = match array_after(dict, "W") {
        Some(w) if w.len() == 3 => [w[0].max(0) as u32, w[1].max(0) as u32, w[2].max(0) as u32],
        _ => return Err(Problem::Widths),
    };
    let row_bytes = (widths[0] + widths[1] + widths[2]) as usize;
    if row_bytes == 0 {
        return Err(Problem::EmptyRows);
    }
    if let Some(f) = unsupported_filter(dict) {
        return Err(Problem::Filter(f));
    }
    let raw = inflate(trim_stream(data)).ok_or(Problem::Compressed)?;
    let predictor = number_after(dict, "Predictor").map(|p| p.max(0) as u32).filter(|p| *p > 1);
    let rows = match predictor {
        None => raw,
        // 2 is TIFF's, which nothing writes for a table. 10 and up are PNG's,
        // and which of the five was used is written on each row rather than
        // here, so they are all one case.
        Some(p) if p >= 10 => {
            let columns = number_after(dict, "Columns").map_or(row_bytes, |c| c.max(1) as usize);
            unpredict(&raw, columns)
        }
        Some(p) => return Err(Problem::Predictor(p)),
    };

    // Which object each row is for. `/Index` is pairs of "first object, how
    // many"; without one the table starts at object zero and runs for `/Size`.
    let index = array_after(dict, "Index")
        .filter(|v| v.len() >= 2 && v.len() % 2 == 0)
        .unwrap_or_else(|| vec![0, number_after(dict, "Size").unwrap_or(i64::MAX)]);

    let mut out = Vec::new();
    let mut at = 0usize;
    'runs: for pair in index.chunks(2) {
        let (first, count) = (pair[0].max(0) as u64, pair[1].max(0) as u64);
        for i in 0..count {
            if at + row_bytes > rows.len() {
                break 'runs;
            }
            let r = &rows[at..at + row_bytes];
            // A width of zero writes nothing and means the default, which the
            // spec gives only for the type: a table of nothing but in-use
            // entries may leave it out.
            let kind = if widths[0] == 0 { 1 } else { be(&r[..widths[0] as usize]) };
            let second = be(&r[widths[0] as usize..(widths[0] + widths[1]) as usize]);
            let third = be(&r[(widths[0] + widths[1]) as usize..]);
            out.push(Row { object: first + i, kind: Kind::of(kind), second, third, at });
            at += row_bytes;
        }
    }
    Ok(Table {
        rows: out,
        widths,
        predictor,
        decoded_bytes: rows.len(),
        trailing_bytes: rows.len() - at.min(rows.len()),
    })
}

/// A big-endian number of however many bytes it was written in. A field of no
/// bytes is zero, which is what a `/W` of zero means for the two numbers the
/// spec gives no other default for.
fn be(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b))
}

/// Step over the line ending the template left on either end. A zlib stream
/// starts with a byte whose low nibble is 8, and deflate is a bit stream that
/// cannot begin with a line ending either, so nothing real is lost.
pub(super) fn trim_stream(data: &[u8]) -> &[u8] {
    let start = data.iter().position(|b| !matches!(b, b'\r' | b'\n')).unwrap_or(data.len());
    let end = data.iter().rposition(|b| !matches!(b, b'\r' | b'\n')).map_or(start, |i| i + 1);
    &data[start..end.max(start)]
}

/// Decompress, as zlib and then as raw deflate. A writer that leaves the zlib
/// header off is out of spec and not rare.
pub(super) fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit as zlib;
    use miniz_oxide::inflate::decompress_to_vec_with_limit as raw;
    // A cross-reference stream holds a row per object. A hundred megabytes of
    // them would be a file with millions of objects, and a stream that claims
    // more than that is a decompression bomb rather than a table.
    const LIMIT: usize = 128 << 20;
    zlib(data, LIMIT).ok().or_else(|| raw(data, LIMIT).ok())
}

/// Undo PNG's row filters. Each row is `columns` bytes with one byte in front
/// saying which filter was used, and every filter reads the row above it, so
/// this runs top to bottom and keeps the row it just finished.
///
/// `bpp` is PNG's bytes per pixel, which decides how far back within a row the
/// filters look. A cross-reference stream is one eight-bit component per
/// column, so it is one, and the byte to the left is the byte before.
fn unpredict(data: &[u8], columns: usize) -> Vec<u8> {
    const BPP: usize = 1;
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; columns];
    let mut row = vec![0u8; columns];
    for chunk in data.chunks(columns + 1) {
        let (filter, src) = match chunk.split_first() {
            Some(x) => x,
            None => break,
        };
        row.iter_mut().for_each(|b| *b = 0);
        row[..src.len()].copy_from_slice(src);
        for i in 0..src.len() {
            let a = if i >= BPP { row[i - BPP] } else { 0 };
            let b = prev[i];
            let c = if i >= BPP { prev[i - BPP] } else { 0 };
            row[i] = row[i].wrapping_add(match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => paeth(a, b, c),
                // A filter byte the spec does not define. Leaving the row as
                // it is keeps the rest of the table readable, where stopping
                // would throw away every row below it.
                _ => 0,
            });
        }
        out.extend_from_slice(&row[..src.len()]);
        prev.copy_from_slice(&row);
    }
    out
}

/// PNG's Paeth predictor: of the byte to the left, the byte above and the byte
/// above-left, whichever is nearest to the three of them added up and the
/// above-left taken away.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// The name of a filter in the dictionary that is not `FlateDecode`, if there
/// is one. `/Filter` may be one name or an array of them; a stream with more
/// than one is not taken apart here even when Flate is among them, since the
/// order they were applied in matters and this only knows how to undo one.
pub(super) fn unsupported_filter(dict: &str) -> Option<String> {
    let after = value_after(dict, "Filter")?;
    // The value and no more of the dictionary than that: an array runs to its
    // closing bracket, and a bare name to the end of the name. Reading past
    // either finds `/DecodeParms` and calls it a filter.
    let list = match after.strip_prefix('[') {
        Some(rest) => &rest[..rest.find(']')?],
        None => {
            let rest = after.strip_prefix('/')?;
            &rest[..rest.find(|c: char| !name_char(c)).unwrap_or(rest.len())]
        }
    };
    let names: Vec<&str> = list
        .split('/')
        .map(|s| s.split(|c: char| !name_char(c)).next().unwrap_or(""))
        .filter(|s| !s.is_empty())
        .collect();
    match names.as_slice() {
        [] | ["FlateDecode"] => None,
        other => Some(other.join(" and ")),
    }
}

/// Bytes that may appear in a PDF name after the `/`.
pub(super) fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' || c == '+' || c == '#'
}

/// Where the value of `/key` begins, with `/key` matched whole: a dictionary
/// holding `/Length1` does not answer for `/Length`.
pub(super) fn value_after<'a>(dict: &'a str, key: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(i) = dict[from..].find('/') {
        let start = from + i + 1;
        let end = dict[start..].find(|c: char| !name_char(c)).map_or(dict.len(), |n| start + n);
        if &dict[start..end] == key {
            return Some(dict[end..].trim_start());
        }
        from = start;
    }
    None
}

/// The number written after `/key`, where one is.
pub(super) fn number_after(dict: &str, key: &str) -> Option<i64> {
    let after = value_after(dict, key)?;
    let end = after.find(|c: char| !c.is_ascii_digit() && c != '-').unwrap_or(after.len());
    after[..end].parse().ok()
}

/// The numbers of the array written after `/key`, where one is. A key whose
/// value is not an array answers nothing rather than the first number after it.
pub(super) fn array_after(dict: &str, key: &str) -> Option<Vec<i64>> {
    let after = value_after(dict, key)?;
    let inside = after.strip_prefix('[')?;
    let end = inside.find(']')?;
    Some(inside[..end].split_whitespace().filter_map(|t| t.parse().ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compress the way a writer does, so the test exercises the real path
    /// rather than a hand-made stand-in.
    fn deflate(data: &[u8]) -> Vec<u8> {
        miniz_oxide::deflate::compress_to_vec_zlib(data, 6)
    }

    #[test]
    fn a_key_is_matched_whole_and_not_as_a_prefix() {
        let d = "<</Length1 99/Length 12/W[1 2 1]/Size 3>>";
        assert_eq!(number_after(d, "Length"), Some(12));
        assert_eq!(number_after(d, "Length1"), Some(99));
        assert_eq!(number_after(d, "Size"), Some(3));
        assert_eq!(number_after(d, "Len"), None);
        assert_eq!(array_after(d, "W"), Some(vec![1, 2, 1]));
        // A key whose value is a number is not an array, and says so.
        assert_eq!(array_after(d, "Size"), None);
    }

    #[test]
    fn a_filter_that_is_not_flate_is_named_rather_than_guessed_at() {
        assert_eq!(unsupported_filter("<</Filter/FlateDecode>>"), None);
        assert_eq!(unsupported_filter("<</Filter [/FlateDecode]>>"), None);
        assert_eq!(unsupported_filter("<</Filter/LZWDecode>>"), Some("LZWDecode".into()));
        assert_eq!(
            unsupported_filter("<</Filter[/ASCII85Decode/FlateDecode]>>"),
            Some("ASCII85Decode and FlateDecode".into())
        );
        assert_eq!(unsupported_filter("<</W[1 2 1]>>"), None);
    }

    /// Three rows of the three kinds, packed one byte, two bytes, one byte.
    #[test]
    fn rows_are_split_by_the_widths_the_dictionary_gives() {
        let raw: Vec<u8> = vec![
            0, 0x00, 0x03, 0xff, // free, next free 3, generation 255
            1, 0x01, 0x2c, 0x00, // in the file at 300, generation 0
            2, 0x00, 0x09, 0x05, // inside object 9, fifth along
        ];
        let t = decode("<</W[1 2 1]/Size 3/Filter/FlateDecode>>", &deflate(&raw)).unwrap();
        assert_eq!(t.widths, [1, 2, 1]);
        assert_eq!(t.predictor, None);
        assert_eq!(t.trailing_bytes, 0);
        assert_eq!(
            t.rows,
            vec![
                Row { object: 0, kind: Kind::Free, second: 3, third: 255, at: 0 },
                Row { object: 1, kind: Kind::InFile, second: 300, third: 0, at: 4 },
                Row { object: 2, kind: Kind::InStream, second: 9, third: 5, at: 8 },
            ]
        );
    }

    /// `/Index` says which objects the rows are for, in runs, the way a table
    /// written in subsections does.
    #[test]
    fn the_object_numbers_come_from_the_index() {
        let raw: Vec<u8> = vec![1, 0x00, 0x10, 0, 1, 0x00, 0x20, 0, 1, 0x00, 0x30, 0];
        let t = decode("<</W[1 2 1]/Index[0 1 40 2]/Filter/FlateDecode>>", &deflate(&raw)).unwrap();
        assert_eq!(t.rows.iter().map(|r| r.object).collect::<Vec<_>>(), vec![0, 40, 41]);
        assert_eq!(t.rows.iter().map(|r| r.second).collect::<Vec<_>>(), vec![0x10, 0x20, 0x30]);
    }

    /// A `/W` whose first number is zero writes no type at all, and every row
    /// is then an in-use one.
    #[test]
    fn a_width_of_zero_means_the_default_rather_than_zero() {
        let raw: Vec<u8> = vec![0x00, 0x10, 0x00, 0x20];
        let t = decode("<</W[0 2 0]/Size 2/Filter/FlateDecode>>", &deflate(&raw)).unwrap();
        assert_eq!(t.rows.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![Kind::InFile, Kind::InFile]);
        assert_eq!(t.rows.iter().map(|r| (r.second, r.third)).collect::<Vec<_>>(), vec![(0x10, 0), (0x20, 0)]);
    }

    /// The PNG filters, undone. Row two is written as the difference from row
    /// one, which is what makes a table of rising offsets compress at all.
    #[test]
    fn png_row_filters_are_undone_before_the_rows_are_split() {
        // Three four-byte rows, each with its filter byte in front. The second
        // uses Up, so its bytes are what to add to the row above; the third
        // uses Sub, so its bytes are differences along the row.
        let filtered: Vec<u8> = vec![
            0, 1, 0x00, 0x10, 0x00, // None:  01 0010 00
            2, 0, 0x00, 0x10, 0x00, // Up:    +0 +0010 +0  ->  01 0020 00
            1, 1, 0x00, 0x2f, 0xd1, // Sub:   each byte plus the one before it
        ];
        let dict = "<</W[1 2 1]/Size 3/Filter/FlateDecode/DecodeParms<</Columns 4/Predictor 12>>>>";
        let t = decode(dict, &deflate(&filtered)).unwrap();
        assert_eq!(t.predictor, Some(12));
        assert_eq!(t.decoded_bytes, 12);
        assert_eq!(t.rows[0], Row { object: 0, kind: Kind::InFile, second: 0x0010, third: 0, at: 0 });
        assert_eq!(t.rows[1], Row { object: 1, kind: Kind::InFile, second: 0x0020, third: 0, at: 4 });
        // 1, then 1+0=1, then 1+0x2f=0x30, then 0x30+0xd1=0x01.
        assert_eq!(t.rows[2], Row { object: 2, kind: Kind::InFile, second: 0x0130, third: 0x01, at: 8 });
    }

    #[test]
    fn a_stream_that_will_not_open_says_so_rather_than_guessing() {
        let dict = "<</W[1 2 1]/Size 1/Filter/FlateDecode>>";
        assert_eq!(decode(dict, b"not a zlib stream at all"), Err(Problem::Compressed));
        assert_eq!(decode("<</Size 1>>", &deflate(&[1, 0, 0, 0])), Err(Problem::Widths));
        assert_eq!(decode("<</W[0 0 0]>>", &deflate(&[1])), Err(Problem::EmptyRows));
        assert_eq!(
            decode("<</W[1 2 1]/Filter/LZWDecode>>", &deflate(&[1, 0, 0, 0])),
            Err(Problem::Filter("LZWDecode".into()))
        );
        assert_eq!(
            decode("<</W[1 2 1]/Size 1/Filter/FlateDecode/DecodeParms<</Predictor 2>>>>", &deflate(&[1, 0, 0, 0])),
            Err(Problem::Predictor(2))
        );
    }

    /// The line endings the template hands over on either end are not part of
    /// the compressed data.
    #[test]
    fn the_line_endings_around_the_data_are_stepped_over() {
        let mut framed = b"\r\n".to_vec();
        framed.extend_from_slice(&deflate(&[1, 0x00, 0x10, 0x00]));
        framed.extend_from_slice(b"\n");
        let t = decode("<</W[1 2 1]/Size 1/Filter/FlateDecode>>", &framed).unwrap();
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.rows[0].second, 0x10);
    }

    /// A stream holding fewer rows than `/Index` promises is read as far as it
    /// goes, rather than failing and showing nothing.
    #[test]
    fn a_short_stream_gives_up_the_rows_it_has() {
        let raw: Vec<u8> = vec![1, 0x00, 0x10, 0x00];
        let t = decode("<</W[1 2 1]/Index[0 50]/Filter/FlateDecode>>", &deflate(&raw)).unwrap();
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.trailing_bytes, 0);
    }
}
