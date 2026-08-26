//! Opening an object stream: the objects a modern PDF keeps compressed
//! together inside another object.
//!
//! A cross-reference row of type 2 does not say where its object is written,
//! because its object is not written anywhere on its own. It says which object
//! stream holds it and how far down the list it is. That is how a PDF written
//! since 1.5 keeps its small objects, and it is most of them: the catalogue,
//! the page tree, every page's dictionary, the fonts. Read only what the file
//! places at offsets of its own and a thirty-object file shows a dozen.
//!
//! The stream is an ordinary numbered object whose dictionary says
//! `/Type /ObjStm`, and its contents, once decompressed, are two runs. The
//! front is a table of `/N` pairs of ASCII numbers: an object number and where
//! that object starts. The rest is the objects themselves, written one after
//! another with nothing between them but the white space the writer felt like.
//! `/First` says where that second run begins, and the offsets in the pair
//! table are counted from there rather than from the front, so an object's
//! first byte is at `/First` plus its offset.
//!
//! There is no `obj`, no `endobj` and no generation. An object inside a stream
//! is generation zero by definition, and the numbers around it that a file
//! would otherwise write are exactly what compressing them together saves.
//!
//! Nothing here is a run of the file, so nothing here can be a field. Same
//! arrangement as [`pdf_xref`](super::pdf_xref): the template leaves the
//! compressed bytes whole and this says what they hold.
//!
//! What is not done. `/Extends`, which chains one object stream onto another
//! so that a writer can append without rewriting, is read and reported but not
//! followed. The objects are handed over as the text they are written in
//! rather than parsed into dictionaries: a reader who wants to see
//! `/Type /Page` wants to see the bytes that say so, and a PDF object parser
//! is a larger thing than this.

use super::pdf_xref::{inflate, number_after, trim_stream, unsupported_filter, value_after};

/// What [`StructDef::packed`](crate::template::StructDef::packed) calls this,
/// so the template can mark every object and the panel can find its way back
/// here for the ones that turn out to be object streams.
pub const PACKING: &str = "pdf_objstm";

/// How many bytes of an object's text are kept. Enough for a page dictionary,
/// which is what nearly all of these are; a content stream that landed here
/// would run longer and is cut.
pub const TEXT_LIMIT: usize = 512;

/// One object inside the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub number: u64,
    /// Where the object starts in the decompressed bytes, counted from the
    /// front of them rather than from `/First`. Not an offset in the file:
    /// these bytes are not in the file.
    pub at: usize,
    /// How long the object is, which is where the next one starts, or the end
    /// of the decompressed bytes for the last.
    pub len: usize,
    /// The object as written, up to [`TEXT_LIMIT`] bytes, with the white space
    /// around it taken off. `cut` says the rest was left behind.
    pub text: String,
    pub cut: bool,
}

/// An opened object stream, and what its dictionary said about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub objects: Vec<Object>,
    /// How many objects `/N` claimed, which is not always how many were there.
    pub claimed: usize,
    /// Where the objects begin, from `/First`.
    pub first: usize,
    /// How many bytes the stream came to once decompressed.
    pub decoded_bytes: usize,
    /// The object number in `/Extends`, where this stream continues another.
    pub extends: Option<u64>,
}

/// Why a stream could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A filter this does not implement, named as the dictionary named it.
    Filter(String),
    /// The compressed bytes would not open, as zlib or as raw deflate.
    Compressed,
    /// No `/N` or no `/First`, so where the pairs stop cannot be known.
    Header,
    /// The pair table is not the pairs of numbers it should be.
    Pairs,
}

impl Problem {
    /// One sentence, standing on its own.
    pub fn as_str(&self) -> String {
        match self {
            Problem::Filter(f) => format!("{f} compression is not supported."),
            Problem::Compressed => "Decompression failed: the data is not valid zlib.".into(),
            Problem::Header => "The stream dictionary has no /N and /First, so the objects cannot be found.".into(),
            Problem::Pairs => "The object numbers at the front of the stream could not be read.".into(),
        }
    }
}

/// Whether this dictionary says the object is an object stream. Every object
/// in the file is offered here, and all but a handful are not one.
pub fn is_object_stream(dict: &str) -> bool {
    value_after(dict, "Type").is_some_and(|v| {
        let name = v.strip_prefix('/').unwrap_or("");
        name.split(|c: char| !super::pdf_xref::name_char(c)).next() == Some("ObjStm")
    })
}

/// Split an object's body into its dictionary and the bytes of its stream.
///
/// The keyword is `stream` and then a line ending, which is what tells it from
/// the same six letters inside a name: a dictionary holding `/Subtype
/// /Image` is safe, but one naming a `/StreamType` would not be if the word
/// alone were looked for.
///
/// Returns nothing for an object with no stream in it, which is most of them.
pub fn split_body(body: &[u8]) -> Option<(&str, &[u8])> {
    let mut from = 0;
    let (keyword, at) = loop {
        let i = from + body[from..].windows(6).position(|w| w == b"stream")?;
        match body.get(i + 6) {
            Some(b'\n') => break (i, i + 7),
            Some(b'\r') if body.get(i + 7) == Some(&b'\n') => break (i, i + 8),
            // A carriage return on its own is not allowed to end this one
            // keyword, which is the one place the spec is strict about it.
            _ => from = i + 6,
        }
    };
    let dict = std::str::from_utf8(&body[..keyword]).ok()?;
    // The template measured the body to `endobj`, so `endstream` is still on
    // the end of it. Everything before that is the stream's.
    let end = body.windows(9).rposition(|w| w == b"endstream").unwrap_or(body.len());
    Some((dict, &body[at..end.max(at)]))
}

/// Open an object stream, given its dictionary as text and the bytes between
/// `stream` and `endstream`.
pub fn decode(dict: &str, data: &[u8]) -> Result<Stream, Problem> {
    let (Some(n), Some(first)) = (number_after(dict, "N"), number_after(dict, "First")) else {
        return Err(Problem::Header);
    };
    let (claimed, first) = (n.max(0) as usize, first.max(0) as usize);
    if let Some(f) = unsupported_filter(dict) {
        return Err(Problem::Filter(f));
    }
    let raw = inflate(trim_stream(data)).ok_or(Problem::Compressed)?;

    // The pair table runs to `/First` and no further. A stream whose `/First`
    // is past its own end has nothing to read there and says so, rather than
    // reading the objects as if they were numbers.
    let head = std::str::from_utf8(raw.get(..first.min(raw.len())).unwrap_or(&[])).map_err(|_| Problem::Pairs)?;
    let nums: Vec<u64> = head.split_whitespace().map_while(|t| t.parse().ok()).collect();
    if nums.len() < 2 {
        return Err(Problem::Pairs);
    }

    // Two numbers a time, and no more pairs than `/N` promised: a table with a
    // stray number after it should not turn the objects into one.
    let pairs: Vec<(u64, usize)> =
        nums.chunks_exact(2).take(claimed).map(|p| (p[0], first.saturating_add(p[1] as usize))).collect();

    let mut objects = Vec::with_capacity(pairs.len());
    for (i, &(number, at)) in pairs.iter().enumerate() {
        // Where the next one starts is how long this one is. The offsets are
        // the writer's and are not promised to rise, so a pair pointing back
        // over the one before it gives a length of nothing rather than a
        // range that will not slice.
        let end = pairs.get(i + 1).map_or(raw.len(), |n| n.1).min(raw.len());
        let at = at.min(raw.len());
        let len = end.saturating_sub(at);
        let cut = len > TEXT_LIMIT;
        let text = String::from_utf8_lossy(&raw[at..at + len.min(TEXT_LIMIT)]).trim().to_string();
        objects.push(Object { number, at, len, text, cut });
    }
    Ok(Stream {
        objects,
        claimed,
        first,
        decoded_bytes: raw.len(),
        extends: number_after(dict, "Extends").map(|n| n.max(0) as u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deflate(data: &[u8]) -> Vec<u8> {
        miniz_oxide::deflate::compress_to_vec_zlib(data, 6)
    }

    /// Build a stream the way a writer does: the objects first, then the pair
    /// table in front of them once their offsets are known.
    fn objstm(bodies: &[(u64, &str)]) -> (String, Vec<u8>) {
        let mut pairs = String::new();
        let mut objs = String::new();
        for (n, body) in bodies {
            pairs.push_str(&format!("{n} {} ", objs.len()));
            objs.push_str(body);
            objs.push(' ');
        }
        let first = pairs.len();
        let dict = format!("<</Type/ObjStm/N {}/First {first}/Filter/FlateDecode>>", bodies.len());
        (dict, deflate(format!("{pairs}{objs}").as_bytes()))
    }

    #[test]
    fn the_objects_come_out_with_their_numbers_and_their_text() {
        let (dict, data) = objstm(&[(1, "<</Type/Catalog/Pages 2 0 R>>"), (2, "<</Type/Pages/Kids[3 0 R]>>")]);
        let s = decode(&dict, &data).unwrap();
        assert_eq!(s.claimed, 2);
        assert_eq!(s.extends, None);
        assert_eq!(s.objects.iter().map(|o| o.number).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(s.objects[0].text, "<</Type/Catalog/Pages 2 0 R>>");
        assert_eq!(s.objects[1].text, "<</Type/Pages/Kids[3 0 R]>>");
        assert!(!s.objects[0].cut);
        // The last object runs to the end of the decompressed bytes.
        assert_eq!(s.objects[1].at + s.objects[1].len, s.decoded_bytes);
    }

    #[test]
    fn an_object_longer_than_the_limit_is_cut_and_says_so() {
        let long = "x".repeat(TEXT_LIMIT + 50);
        let (dict, data) = objstm(&[(7, &long)]);
        let s = decode(&dict, &data).unwrap();
        assert!(s.objects[0].cut);
        assert_eq!(s.objects[0].text.len(), TEXT_LIMIT);
    }

    #[test]
    fn only_a_dictionary_that_says_objstm_is_one() {
        assert!(is_object_stream("<</Type/ObjStm/N 3>>"));
        assert!(!is_object_stream("<</Type/ObjStmX/N 3>>"));
        assert!(!is_object_stream("<</Type/XRef/N 3>>"));
        assert!(!is_object_stream("<</Length 12>>"));
    }

    /// The keyword ends a line. Six letters inside a name do not.
    #[test]
    fn the_stream_keyword_is_the_one_that_ends_a_line() {
        let body = b"<</StreamType/Odd/Length 4>>\nstream\nDATA\nendstream\n".as_slice();
        let (dict, data) = split_body(body).unwrap();
        assert_eq!(dict, "<</StreamType/Odd/Length 4>>\n");
        assert_eq!(data, b"DATA\n");
        assert_eq!(split_body(b"<</Type/Page>>\n"), None);
    }

    #[test]
    fn a_stream_that_will_not_open_says_which_way_it_failed() {
        assert_eq!(decode("<</Type/ObjStm/First 4>>", &deflate(b"1 0 ")), Err(Problem::Header));
        assert_eq!(decode("<</N 1/First 4>>", b"not zlib at all"), Err(Problem::Compressed));
        assert_eq!(
            decode("<</N 1/First 4/Filter/LZWDecode>>", &deflate(b"1 0 ")),
            Err(Problem::Filter("LZWDecode".into()))
        );
        // A pair table of one number is not pairs.
        assert_eq!(decode("<</N 1/First 2>>", &deflate(b"1 <</Type/Page>>")), Err(Problem::Pairs));
    }

    /// `/N` says how many, and a number after the table does not add one.
    #[test]
    fn no_more_pairs_are_read_than_the_dictionary_promised() {
        let raw = b"1 0 2 3 9 9 <<>> <<>>";
        let dict = format!("<</Type/ObjStm/N 2/First {}>>", raw.len() - 10);
        let s = decode(&dict, &deflate(raw)).unwrap();
        assert_eq!(s.objects.len(), 2);
        assert_eq!(s.objects.iter().map(|o| o.number).collect::<Vec<_>>(), vec![1, 2]);
    }

    /// An offset past the end of the decompressed bytes is clamped rather than
    /// panicking, and the object it names comes out empty.
    #[test]
    fn an_offset_past_the_end_gives_an_empty_object() {
        let s = decode("<</Type/ObjStm/N 1/First 6/Extends 9 0 R>>", &deflate(b"1 900 <<>>")).unwrap();
        assert_eq!(s.objects[0].len, 0);
        assert_eq!(s.objects[0].text, "");
        assert_eq!(s.extends, Some(9));
    }
}
