//! The fields a decoder's trace lays down over the run it read.
//!
//! Everything else in the IR is written down: a template says a field is
//! there and the reading finds out where. A deflate stream cannot be written
//! down. How many blocks it holds, how long each one's Huffman tables are and
//! how many codes come after them are all answers the decoder finds while
//! decoding, and nothing short of decoding it can say. So the decoder writes
//! down what it read and this reads the fields back off that: one node per
//! block, one per code length, one per literal or match.
//!
//! Two rules keep it honest. Nothing is shown that the decoder did not read:
//! every node here stands for exactly one [`Step`], and its bits are that
//! step's bits. And no value is read twice: `hlit`, `hdist`, a code length are
//! all the numbers the decoder used, carried here as computed fields rather
//! than read again out of the bits and possibly differently.
//!
//! A code is the one place where the two rules pull apart, and the second
//! wins. A literal's byte is not written next to the literal; it is what the
//! block's table says nine particular bits mean. So the byte goes in the
//! node's name, where a reading belongs, and the value is the bits themselves.
//! The node used to hold a `kind` and a `value` of no width, which put two
//! inferences in the column a reader checks the bytes against.
//!
//! A caveat on where a node sits. Deflate reads bits from the low end of a
//! byte upwards, and Qubero addresses bits from the high end downwards, so a
//! step's *byte* extent is the same either way and a step narrower than a byte
//! is highlighted at the other end of its byte from where deflate read it.
//! Nothing between the two conventions can fix that; a byte-aligned codec has
//! no such question. The *value* of a code does not have the problem, because
//! it is not a range: it is written out in the order the decoder read, which
//! the trace says. See `read::Evaluator::read_code_bits`.

use crate::codec::{Block, BlockKind, Step, StepField, StepKind, TableField, Trace};
use crate::eval::Sizing;
use crate::template::{Expr as E, Ty as T};

/// Which of the blocks in a trace a symbol step belongs to, and where its
/// symbols begin: worked out once when a block node is placed.
pub(super) struct BlockView<'a> {
    pub block: &'a Block,
    /// The steps of the block that come before the first symbol: the header
    /// fields and, for a dynamic block, its tables.
    pub head: std::ops::Range<u32>,
    /// The steps from the first symbol to the end of the block.
    pub symbols: std::ops::Range<u32>,
}

impl<'a> BlockView<'a> {
    pub fn of(trace: &'a Trace, i: u32) -> Option<BlockView<'a>> {
        let block = trace.blocks().get(i as usize)?;
        let first = (block.steps.start..block.steps.end)
            .find(|&k| trace.step(k as usize).is_some_and(|s| is_payload(&s.kind)))
            .unwrap_or(block.steps.end);
        Some(BlockView {
            block,
            head: block.steps.start..first,
            symbols: first..block.steps.end,
        })
    }

    /// Where the symbols start, as a bit of the run. The end of the block when
    /// there are none, which is what an empty block has.
    pub fn symbols_at(&self, trace: &Trace) -> u64 {
        match trace.step(self.symbols.start as usize) {
            Some(s) => s.in_bits.start,
            None => self.block.in_bits.end,
        }
    }
}

/// Whether a step is one of a block's symbols rather than its machinery.
pub(super) fn is_payload(kind: &StepKind) -> bool {
    matches!(
        kind,
        StepKind::Literal(_)
            | StepKind::Match { .. }
            | StepKind::Stored
            | StepKind::Pixel
            | StepKind::EndOfBlock
            | StepKind::Opaque
    )
}

/// What one block is called in the listing: what coded its symbols, and
/// whether it is the last.
pub(super) fn block_name(block: &Block) -> String {
    match (block.kind, block.last) {
        (k, true) => format!("{} block, last", k.as_str()),
        (k, false) => format!("{} block", k.as_str()),
    }
}

/// What a step of a block's machinery is called and what it reads as.
///
/// Two things at once because they come from the same match: the name is the
/// format's own word for the field, and the type carries the value the decoder
/// read, at the width the step took.
pub(super) fn head_field(step: &Step) -> (String, T) {
    let bits = step.in_bits.end - step.in_bits.start;
    match step.kind {
        StepKind::Header(field, value) => (field.as_str().to_string(), header_ty(field, value, bits)),
        StepKind::Table(table) => table_field(table, bits),
        // Not reached for a block's head, and better than a panic if it ever
        // is: bits nobody named.
        other => (other.as_str().to_string(), sized(bits, T::computed(E::lit(0)))),
    }
}

/// A header field's value, named where the format names its values.
fn header_ty(field: StepField, value: u32, bits: u64) -> T {
    let inner = T::computed(E::lit(value as i128));
    let named = match field {
        StepField::Bfinal => {
            T::enumeration("DeflateFinal", inner, &[(0, "more blocks follow"), (1, "the last block")])
        }
        StepField::Btype => T::enumeration(
            "DeflateBlockType",
            inner,
            &[(0, "stored"), (1, "fixed Huffman"), (2, "dynamic Huffman"), (3, "reserved")],
        ),
        // RFC 2083 section 6: how the encoder predicted this row.
        StepField::Filter => T::enumeration(
            "PngFilter",
            inner,
            &[(0, "none"), (1, "sub"), (2, "up"), (3, "average"), (4, "paeth")],
        ),
        _ => inner,
    };
    sized(bits, named)
}

/// One entry of a Huffman table, as the decoder read it.
///
/// Every name says `code length for` and then which alphabet's symbol it is
/// for, because that is what the row holds: the value is a width in bits, and
/// the symbol is what a code of that width will stand for. The short forms
/// these replaced each said the wrong thing. `symbol 77` read as the name of
/// the row rather than as what the length is for, and it was the phrase the
/// tooltip over a code in the payload used as well, for the code's index in
/// the run. `distance 3` read as a distance of three bytes, which is a real
/// thing in the same format and is not this. `code length 5` left the reader
/// to work out that the second number was a symbol of the code-length
/// alphabet rather than a second length.
fn table_field(table: TableField, bits: u64) -> (String, T) {
    let (name, value) = match table {
        TableField::CodeLen { sym, len } => (format!("code length for code-length symbol {sym}"), len as i128),
        TableField::LitLen { sym, len } => (format!("code length for symbol {sym}"), len as i128),
        TableField::Dist { sym, len } => (format!("code length for distance symbol {sym}"), len as i128),
        // A run-length code stands for several lengths at once, so it is named
        // by what it does rather than by which symbol it filled.
        TableField::Repeat { code, count, len, dist } => {
            let what = if dist { "distance codes" } else { "symbols" };
            let name = match code {
                16 => format!("repeat {len} for {count} more {what}"),
                _ => format!("{count} {what} with no code"),
            };
            (name, count as i128)
        }
    };
    (name, sized(bits, T::computed(E::lit(value))))
}

/// What one step of a block's payload is called.
///
/// The name carries the reading, because the value carries the bits. A
/// literal's byte, a match's length and distance are what the block's tables
/// said the bits mean, and none of the three is written anywhere in the file
/// as a number: saying them here is the one place they can be said without
/// pretending they were read.
pub(super) fn symbol_name(step: &Step) -> String {
    let bytes = step.out_bytes.end - step.out_bytes.start;
    match step.kind {
        StepKind::Literal(v) => format!("literal {}", byte(v)),
        StepKind::Match { len, dist } => format!("match {len} back {dist}"),
        StepKind::EndOfBlock => "end of block".to_string(),
        // Bits between the codes that are not a code. A `compress` stream pads
        // the rest of its group of eight codes out with zeroes whenever the
        // width grows or the table is cleared, and those bits are as real as
        // any others: a reader standing on one is told what they are rather
        // than shown whichever code is nearest.
        StepKind::Header(StepField::Padding, _) => "padding".to_string(),
        StepKind::Stored => "literals".to_string(),
        // One pixel read for the bits hidden in it. It yields one byte, or two
        // where a pixel carries more than eight bits, so the count is worth
        // saying even though it is nearly always one.
        StepKind::Pixel => match bytes {
            1 => "pixel".to_string(),
            n => format!("pixel, {n} bytes"),
        },
        // A run the trace stopped naming, because there were too many of them
        // to name. See `codec::MAX_STEPS`.
        _ => "codes not named one at a time".to_string(),
    }
}

/// One step of a block's payload: what it is, and the bits it is written in.
///
/// `coding` is the block's, and settles both halves of what the panel says
/// about the width. In a fixed-Huffman block RFC 1951 chose it and nothing in
/// the file could have chosen otherwise; in a dynamic one the block's own
/// code-length table did, and the same byte in the next block is very likely a
/// different width.
///
/// A step is shown as its bits only where the block was Huffman-coded, which
/// is [`BlockKind::Fixed`] and [`BlockKind::Dynamic`]: deflate's two, LHA's
/// blocks and RAR5's. Everything else keeps the record with a `kind` in it
/// that every step used to have. A stored block's one step is a payload copied
/// through, which could be sixty kilobytes and is not a code; an LZ4 sequence
/// and an LZW code are tokens with fields in them rather than one code, and
/// their bytes written out as noughts and ones say less than their numbers do;
/// and a step the trace gave up naming stands for thousands of codes at once.
pub(super) fn symbol_ty(step: &Step, coding: BlockKind) -> (String, T) {
    let bits = step.in_bits.end - step.in_bits.start;
    let bytes = step.out_bytes.end - step.out_bytes.start;
    let name = symbol_name(step);
    if let Some(code) = code_ty(step, coding) {
        return (name, sized(bits, code));
    }
    let kind = |k: i128| {
        T::enumeration(
            "SymbolKind",
            T::computed(E::lit(k)),
            &[
                (0, "literal"),
                (1, "match"),
                (2, "end of block"),
                (3, "stored"),
                (4, "not named"),
                (5, "pixel"),
                (6, "padding"),
            ],
        )
    };
    let fields = match step.kind {
        StepKind::Literal(v) => vec![("kind", kind(0)), ("value", T::computed(E::lit(v as i128)))],
        StepKind::Match { len, dist } => vec![
            ("kind", kind(1)),
            ("length", T::computed(E::lit(len as i128))),
            ("distance", T::computed(E::lit(dist as i128))),
        ],
        StepKind::EndOfBlock => vec![("kind", kind(2))],
        StepKind::Header(StepField::Padding, _) => vec![("kind", kind(6))],
        StepKind::Stored => vec![("kind", kind(3)), ("length", T::computed(E::lit(bytes as i128)))],
        StepKind::Pixel => vec![("kind", kind(5)), ("length", T::computed(E::lit(bytes as i128)))],
        _ => vec![("kind", kind(4)), ("length", T::computed(E::lit(bytes as i128)))],
    };
    (name, sized(bits, T::structure("Symbol", fields)))
}

/// The type of a step that is one entropy-coded thing, or `None` for a step
/// that is not.
///
/// Two names, because a node is not always one code. A literal and an
/// end-of-block are a single Huffman code and nothing else. A match is a
/// length code, that code's extra bits, a distance code and *its* extra bits,
/// four runs with no boundary between them that anything could name, so
/// calling the node a Huffman code would claim a shape the bits do not have.
///
/// Three widths, for the three ways the number of bits was arrived at. The
/// literal and the end-of-block take the block's answer. The match takes
/// [`Sizing::Encoded`], the same word a variable-length integer gets, and for
/// the same reason: how far it runs is settled by decoding it, since the
/// length code says how many extra bits follow it and the distance code says
/// how many follow that. Neither the format nor a table fixed the total, and
/// `Sizing::Children` would be worse than either, because there are no
/// children to add up.
fn code_ty(step: &Step, coding: BlockKind) -> Option<T> {
    if !matches!(coding, BlockKind::Fixed | BlockKind::Dynamic) {
        return None;
    }
    // Only `Dynamic` is left, the guard above having refused the rest.
    let one_code = match coding {
        BlockKind::Fixed => Sizing::Fixed,
        _ => Sizing::Table,
    };
    match step.kind {
        StepKind::Literal(_) | StepKind::EndOfBlock => Some(T::code_bits("Huffman code", one_code)),
        StepKind::Match { .. } => Some(T::code_bits("length and distance codes", Sizing::Encoded)),
        _ => None,
    }
}

/// What a stream's blocks are called as a whole, which is what the row above
/// them says.
pub(super) fn blocks_unit(kind: Option<BlockKind>) -> &'static str {
    match kind {
        Some(BlockKind::Sequences) => "sequence",
        _ => "block",
    }
}

/// A literal's byte, as the reader would rather see it: the character when it
/// is one, and the number when it is not. A row saying `literal 10` and a row
/// saying `literal 'a'` are both answering "which byte", and neither reads as
/// the other's answer.
fn byte(v: u8) -> String {
    match v {
        0x20..=0x7e => format!("{:?}", v as char),
        _ => format!("{v:#04x}"),
    }
}

/// What a symbol says in a cell of the value table, as against what it says in
/// the listing and in its tooltip.
///
/// A row of a deflate block is a row of what the block produces, so a literal
/// reads as the byte itself and a row of them reads as the text coming out of
/// the stream. `literal 'Q'` repeated across a screen is ten lines of the same
/// two words; `Q` is one line of the file being unpacked.
///
/// A match is its two numbers with a sign between them rather than the word
/// `back`: the cells are twenty-odd characters wide with a screenful of
/// neighbours under them, and `6 back 18` spends three of those characters on
/// a word that is the same in every row. `6\u{2190}18` reads the same way at a
/// glance and lines up with its neighbours. Length first, as in every written
/// (length, distance) pair since RFC 1951, and the arrow points back to where
/// the bytes are copied from.
///
/// A distance of one gets its own mark, `\u{d7}6`. It is not really a copy at
/// all: it is deflate's run-length idiom, the byte before repeated `len`
/// times, and it is how a stream of the same byte is written. A reader
/// scanning a column of matches wants to see those apart from the rest at a
/// glance rather than read the 1 each time.
pub(super) fn symbol_label(step: &Step) -> String {
    match step.kind {
        StepKind::Literal(v) => byte_label(v),
        StepKind::Match { len, dist: 1 } => format!("\u{d7}{len}"),
        StepKind::Match { len, dist } => format!("{len}\u{2190}{dist}"),
        _ => symbol_name(step),
    }
}

/// A literal's byte as the value table shows it: the character, so the cells
/// read as the text they decode to. A space would be an empty cell, so it
/// keeps the quotes that show there is a byte there at all, and anything not
/// printable reads as its number.
fn byte_label(v: u8) -> String {
    match v {
        0x21..=0x7e => (v as char).to_string(),
        0x20 => "' '".to_string(),
        _ => format!("{v:#04x}"),
    }
}

fn sized(bits: u64, inner: T) -> T {
    T::SizedBits { bits: E::lit(bits as i128), inner: Box::new(inner) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::Evaluator;
    use crate::source::MemSource;

    /// The zlib template's `compressed` field, which is the deflate run.
    const RUN: &[usize] = &[6];

    fn reading(content: &[u8]) -> (Document<MemSource>, Evaluator) {
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(content, 6);
        (Document::new(MemSource(packed)), Evaluator::new(crate::formats::builtin("zlib").unwrap()))
    }

    /// A stream has two children: what came out of it, and what the decoder
    /// read to get there.
    #[test]
    fn a_stream_shows_its_contents_and_then_its_blocks() {
        let (d, mut e) = reading(b"structure and the shape of it and the shape of the shape");
        let run = e.node(&d, RUN).unwrap();
        assert_eq!(run.child_count, 2);
        let blocks = e.node(&d, &[6, 1]).unwrap();
        assert_eq!(blocks.name, "blocks");
        assert_eq!(blocks.type_name, "blocks");
        assert_eq!(blocks.unit.as_deref(), Some("block"));
        assert_eq!(blocks.machinery, Some(true), "the blocks are machinery for the contents");
        assert_eq!(e.node(&d, &[6, 0]).unwrap().machinery, None, "the contents are not machinery");
        assert_eq!(blocks.space, 0, "the blocks are bits of the file, not of the stream");
        assert_eq!(blocks.child_count, 1);
        // The one block starts where the run does and runs to the end of it.
        let block = e.node(&d, &[6, 1, 0]).unwrap();
        assert_eq!(block.offset_bits, run.offset_bits);
        assert!(block.name.ends_with("block, last"), "a block called {:?}", block.name);
    }

    /// A fixed-Huffman block's header is two fields and then its codes; a
    /// dynamic one has its tables in between.
    #[test]
    fn a_block_shows_the_header_it_declared() {
        let (d, mut e) = reading(b"");
        let names: Vec<String> = (0..e.node(&d, &[6, 1, 0]).unwrap().child_count as usize)
            .map(|i| e.node(&d, &[6, 1, 0, i]).unwrap().name)
            .collect();
        assert_eq!(names, ["bfinal", "btype", "codes"]);
        let bfinal = e.node(&d, &[6, 1, 0, 0]).unwrap();
        assert_eq!(bfinal.size_bits, 1);
        assert_eq!(
            bfinal.value,
            crate::eval::Value::Enum { raw: 1, name: Some("the last block".into()), hex: false }
        );
        let btype = e.node(&d, &[6, 1, 0, 1]).unwrap();
        assert_eq!(btype.size_bits, 2);
        assert_eq!(btype.offset_bits, bfinal.offset_bits + 1);
    }

    /// A dynamic block writes its two tables down, and the reader can open
    /// them: one row per code length, named by the symbol it is for.
    #[test]
    fn a_dynamic_blocks_tables_are_rows() {
        let (d, mut e) = reading(&"the shape of the shape of the shape. ".repeat(80).into_bytes());
        let block = e.node(&d, &[6, 1, 0]).unwrap();
        let names: Vec<String> =
            (0..block.child_count as usize).map(|i| e.node(&d, &[6, 1, 0, i]).unwrap().name).collect();
        assert_eq!(&names[..5], ["bfinal", "btype", "hlit", "hdist", "hclen"]);
        assert_eq!(names.last().unwrap(), "codes");
        assert!(
            names.iter().any(|n| n.starts_with("code length for code-length symbol ")),
            "no code-length-alphabet rows in {names:?}"
        );
        assert!(
            names.iter().any(|n| n.starts_with("code length for symbol ")),
            "no literal/length rows in {names:?}"
        );
        // A table row is measured, not fixed and not looked up: the decoder
        // read as many bits as the code-length code took. Only a code in the
        // payload gets the table answer.
        let entry = names.iter().position(|n| n.starts_with("code length for symbol ")).expect("a table row");
        assert_eq!(e.shape(&d, &[6, 1, 0, entry]).unwrap().sized, Sizing::Trace);
        let hlit = e.shape(&d, &[6, 1, 0, 2]).unwrap();
        assert_eq!(hlit.sized, Sizing::Trace, "hlit is measured by the decoder, not read off a table");
        assert_eq!(hlit.placed, crate::eval::Placed::Trace);
        // Every row is as wide as the bits the decoder read for it, and they
        // follow one another with nothing in between.
        let mut at = block.offset_bits;
        for i in 0..block.child_count as usize {
            let child = e.node(&d, &[6, 1, 0, i]).unwrap();
            assert_eq!(child.offset_bits, at, "child {i} ({}) is not where the one before it ended", child.name);
            at += child.size_bits;
        }
        assert_eq!(at, block.offset_bits + block.size_bits, "the block's children do not fill it");
    }

    /// The codes: the name says what one stood for, the value is the bits it
    /// was written in, and there is nothing under it.
    #[test]
    fn the_codes_say_what_they_were() {
        let (d, mut e) = reading(b"abcabcabcabcabcabcabcabc");
        let codes = {
            let block = e.node(&d, &[6, 1, 0]).unwrap();
            block.child_count as usize - 1
        };
        let run = e.node(&d, &[6, 1, 0, codes]).unwrap();
        assert_eq!(run.name, "codes");
        assert_eq!(run.type_name, "codes");
        assert_eq!(run.unit.as_deref(), Some("code"));
        assert!(run.child_count >= 4, "only {} codes", run.child_count);
        let first = e.node(&d, &[6, 1, 0, codes, 0]).unwrap();
        assert_eq!(first.name, "literal 'a'");
        assert_eq!(first.type_name, "Huffman code");
        // A leaf. The byte it stood for is in the name, because that is a
        // reading and not something written at these bits.
        assert_eq!(first.child_count, 0);
        // The value is the code, as wide as the node.
        let bits = match &first.value {
            crate::eval::Value::Str(s) => s.clone(),
            other => panic!("a code read as {other:?}"),
        };
        assert_eq!(bits.len() as u64, first.size_bits);
        assert!(bits.chars().all(|c| c == '0' || c == '1'), "a code that reads {bits:?}");
        // Somewhere in the run is a match, and it says both numbers.
        let names: Vec<String> =
            (0..run.child_count as usize).map(|i| e.node(&d, &[6, 1, 0, codes, i]).unwrap().name).collect();
        assert!(names.iter().any(|n| n.starts_with("match ")), "no match in {names:?}");
        assert_eq!(names.last().unwrap(), "end of block");
        let m = names.iter().position(|n| n.starts_with("match ")).expect("a match");
        assert_eq!(e.node(&d, &[6, 1, 0, codes, m]).unwrap().type_name, "length and distance codes");
    }

    /// Where a code is and how wide it is, in the two words the panel prints.
    ///
    /// The first code of a block is the first thing in the run; every one
    /// after it starts where the one before it ended, which is what an
    /// entropy-coded run is. Neither is "wherever the decoder put it", which
    /// is the non-answer these replaced.
    #[test]
    fn a_code_follows_the_code_before_it() {
        let (d, mut e) = reading(b"abcabcabcabcabcabcabcabc");
        let codes = e.node(&d, &[6, 1, 0]).unwrap().child_count as usize - 1;
        use crate::eval::Placed;
        assert_eq!(e.shape(&d, &[6, 1, 0, codes, 0]).unwrap().placed, Placed::First);
        assert_eq!(e.shape(&d, &[6, 1, 0, codes, 1]).unwrap().placed, Placed::Follows);
        // The run itself, and the block, are still the decoder's doing.
        assert_eq!(e.shape(&d, &[6, 1, 0, codes]).unwrap().placed, Placed::Trace);
        assert_eq!(e.shape(&d, &[6, 1, 0]).unwrap().placed, Placed::Trace);
    }

    /// Which of the two answers about a code's width the block gives.
    ///
    /// A fixed-Huffman block's widths are RFC 1951's and nothing in the file
    /// could have changed them. A dynamic block's are its own table's, and
    /// saying "fixed by the format" about those would be false.
    #[test]
    fn a_codes_width_comes_from_the_format_or_from_the_block() {
        let (d, mut e) = reading(b"");
        let fixed = e.node(&d, &[6, 1, 0, 1]).unwrap();
        assert_eq!(fixed.value.as_int(), Some(1), "miniz wrote something other than a fixed block");
        assert_eq!(e.shape(&d, &[6, 1, 0, 2, 0]).unwrap().sized, Sizing::Fixed);

        let (d, mut e) = reading(&"the shape of the shape of the shape. ".repeat(80).into_bytes());
        let codes = e.node(&d, &[6, 1, 0]).unwrap().child_count as usize - 1;
        assert_eq!(e.node(&d, &[6, 1, 0, 1]).unwrap().value.as_int(), Some(2), "not a dynamic block");
        let run = e.node(&d, &[6, 1, 0, codes]).unwrap();
        let widths: Vec<Sizing> = (0..run.child_count as usize)
            .map(|i| e.shape(&d, &[6, 1, 0, codes, i]).unwrap().sized)
            .collect();
        assert!(widths.contains(&Sizing::Table), "no code took its width from the block's table");
        assert!(!widths.contains(&Sizing::Fixed), "a dynamic block said its widths were fixed by the format");
        // A match is neither: it is a length code, its extra bits, a distance
        // code and its extra bits, and only decoding it says how far it runs.
        let names: Vec<String> =
            (0..run.child_count as usize).map(|i| e.node(&d, &[6, 1, 0, codes, i]).unwrap().name).collect();
        let m = names.iter().position(|n| n.starts_with("match ")).expect("a match");
        assert_eq!(widths[m], Sizing::Encoded);
    }

    /// The bits a code reads as, pinned against RFC 1951.
    ///
    /// The one thing about the value that could silently be wrong. Deflate
    /// reads the low bit of a byte first, Qubero addresses bits from the high
    /// bit down, and reading the same nine bits the wrong way round gives a
    /// different string that still looks like a Huffman code. Section 3.2.6
    /// fixes the codes of a fixed block: symbol 256, end-of-block, is seven
    /// zero bits, and a literal 'a' is 0x30 + 97 = 145 in eight bits, which is
    /// `10010001`. If the order ever flips, both of these change.
    #[test]
    fn a_fixed_blocks_codes_read_as_rfc_1951_writes_them() {
        let (d, mut e) = reading(b"a");
        assert_eq!(e.node(&d, &[6, 1, 0, 1]).unwrap().value.as_int(), Some(1), "not a fixed block");
        let str_of = |e: &mut Evaluator, i: usize| match e.node(&d, &[6, 1, 0, 2, i]).unwrap().value {
            crate::eval::Value::Str(s) => s,
            other => panic!("a code read as {other:?}"),
        };
        assert_eq!(e.node(&d, &[6, 1, 0, 2, 0]).unwrap().name, "literal 'a'");
        assert_eq!(str_of(&mut e, 0), "10010001");
        assert_eq!(e.node(&d, &[6, 1, 0, 2, 1]).unwrap().name, "end of block");
        assert_eq!(str_of(&mut e, 1), "0000000");
    }

    /// A code's origin names the block whose tables decoded it: which nine
    /// bits a literal is, is a fact about that block and not about the bits.
    #[test]
    fn a_code_says_which_block_decoded_it() {
        let (d, mut e) = reading(&"the shape of the shape of the shape. ".repeat(80).into_bytes());
        let codes = e.node(&d, &[6, 1, 0]).unwrap().child_count as usize - 1;
        let at = [6, 1, 0, codes, 0];
        let origins = e.origins(&d, &at).unwrap();
        let table = origins
            .iter()
            .find(|o| o.path == [6, 1, 0])
            .unwrap_or_else(|| panic!("no row naming the block in {origins:?}"));
        assert_eq!(table.role, crate::eval::Role::Type);
        assert!(table.label.contains("dynamic"), "the block is called {:?}", table.label);
        // And the stream it all came out of is still named.
        assert!(e.origins(&d, &[6, 0, 0]).unwrap().iter().any(|o| o.value == "deflate"));
    }

    /// Nothing the decoder read is editable: the bits are the file's, but what
    /// they mean is a Huffman code, and writing a number back into one is not
    /// a thing this offers.
    #[test]
    fn nothing_read_from_a_trace_is_written_back() {
        let (d, mut e) = reading(b"no writing here");
        for path in [&[6usize, 1][..], &[6, 1, 0], &[6, 1, 0, 0], &[6, 1, 0, 1], &[6, 1, 0, 2, 0]] {
            assert!(!e.node(&d, path).unwrap().editable, "{path:?} is offered for editing");
        }
        // And a reader who asks anyway is told why, rather than told to use
        // the hex view: typing over these bits there breaks the stream too.
        let crate::eval::EvalError::Failed(why) = e.prepare_write(&d, &[6, 1, 0, 2, 0], "0").unwrap_err() else {
            panic!("writing a code was not refused outright");
        };
        assert!(why.contains("shift every code after it"), "the refusal reads {why:?}");
    }

    /// The annotation column over a deflate run says which block the bytes are
    /// in, not which Huffman code: one entry per block, each saying how many
    /// codes it holds. Before this every code was an entry of its own, and one
    /// reading `unmapped 9 bits`.
    #[test]
    fn the_column_over_a_stream_names_its_blocks() {
        let (d, mut e) = reading(&"the shape of the shape of the shape. ".repeat(2000).into_bytes());
        let run = e.node(&d, RUN).unwrap();
        let spans = e.spans(&d, run.offset_bits, run.offset_bits + run.size_bits, 5000).unwrap();
        assert!(!spans.is_empty());
        for s in &spans {
            assert!(!s.gap, "a gap over a decoded stream: {} of {} bits", s.name, s.size_bits);
        }
        let blocks: Vec<_> = spans.iter().filter(|s| s.name.contains("block")).collect();
        assert!(!blocks.is_empty(), "no block entries in {:?}", spans.iter().map(|s| &s.name).collect::<Vec<_>>());
        for b in &blocks {
            assert!(b.count > 0, "{} stands for no codes", b.name);
            assert_eq!(b.unit.as_deref(), Some("code"));
        }
        // A block is thousands of codes, so a handful of entries covers the
        // whole run rather than one per code.
        assert!(spans.len() < 32, "{} entries over the run", spans.len());
    }

    /// And working the column out does not place a node per code: the whole
    /// run costs a bounded number of them, so scrolling through a stream does
    /// not fill memory with literals.
    #[test]
    fn naming_the_blocks_places_no_codes() {
        let (d, mut e) = reading(&"the shape of the shape of the shape. ".repeat(2000).into_bytes());
        let run = e.node(&d, RUN).unwrap();
        let before = e.memo.len();
        e.spans(&d, run.offset_bits, run.offset_bits + run.size_bits, 5000).unwrap();
        let grew = e.memo.len() - before;
        assert!(grew < 200, "{grew} nodes placed to name the blocks of one stream");
        // A second pass over the same window places nothing further.
        let settled = e.memo.len();
        e.spans(&d, run.offset_bits, run.offset_bits + run.size_bits, 5000).unwrap();
        assert_eq!(e.memo.len(), settled, "asking again placed more nodes");
    }

    #[test]
    fn a_code_is_as_wide_as_the_bits_it_was_written_in() {
        let step = Step { in_bits: 3..12, out_bytes: 0..1, kind: StepKind::Literal(b'q') };
        let (name, ty) = symbol_ty(&step, BlockKind::Dynamic);
        assert_eq!(name, "literal 'q'");
        let T::SizedBits { bits, inner } = ty else { panic!("a code with no width") };
        assert!(matches!(bits, E::Lit(9)), "nine bits read, written down as {bits:?}");
        assert!(matches!(*inner, T::CodeBits { .. }), "a code that is not read as its bits");
    }

    /// A step that is not one entropy-coded thing keeps the record it had. A
    /// stored block's payload can be sixty kilobytes, and sixty kilobytes of
    /// noughts and ones is not a code however it is written.
    #[test]
    fn a_step_that_is_not_a_code_is_not_shown_as_bits() {
        let stored = Step { in_bits: 0..8 * 4096, out_bytes: 0..4096, kind: StepKind::Stored };
        let (name, ty) = symbol_ty(&stored, BlockKind::Stored);
        assert_eq!(name, "literals");
        let T::SizedBits { inner, .. } = ty else { panic!("no width") };
        assert!(matches!(*inner, T::Struct(_)), "a stored payload written out as bits");
        // Nor does an LZ4 sequence, whose token is bytes with fields in it,
        // and it keeps the record it always had.
        let seq = Step { in_bits: 0..24, out_bytes: 0..9, kind: StepKind::Match { len: 9, dist: 4 } };
        let T::SizedBits { inner, .. } = symbol_ty(&seq, BlockKind::Sequences).1 else { panic!("no width") };
        let T::Struct(s) = &*inner else { panic!("an LZ4 sequence called a Huffman code") };
        assert_eq!(s.fields.iter().map(|f| &*f.name).collect::<Vec<_>>(), ["kind", "length", "distance"]);
        // An LZW code likewise: `compress` writes literals and matches too.
        let lzw = Step { in_bits: 9..18, out_bytes: 0..1, kind: StepKind::Literal(b'q') };
        let T::SizedBits { inner, .. } = symbol_ty(&lzw, BlockKind::Sequences).1 else { panic!("no width") };
        let T::Struct(s) = &*inner else { panic!("an LZW code called a Huffman code") };
        assert_eq!(s.fields.iter().map(|f| &*f.name).collect::<Vec<_>>(), ["kind", "value"]);
    }

    /// What a code shows in a cell of the value table: what it decoded to, so
    /// a row of them reads as the text the block produces. A space and a
    /// control byte would both be blank cells, so both keep a form that shows
    /// there is a byte there.
    #[test]
    fn a_code_in_a_cell_reads_as_what_it_decodes_to() {
        let lit = |v: u8| symbol_label(&Step { in_bits: 0..9, out_bytes: 0..1, kind: StepKind::Literal(v) });
        assert_eq!(lit(b'Q'), "Q");
        assert_eq!(lit(b' '), "' '");
        assert_eq!(lit(b'\n'), "0x0a");
        assert_eq!(lit(0xff), "0xff");
        let step = Step { in_bits: 0..17, out_bytes: 5..17, kind: StepKind::Match { len: 6, dist: 18 } };
        assert_eq!(symbol_label(&step), "6\u{2190}18");
        // A distance of one is the run-length idiom and gets its own mark.
        let run = Step { in_bits: 0..17, out_bytes: 5..17, kind: StepKind::Match { len: 40, dist: 1 } };
        assert_eq!(symbol_label(&run), "\u{d7}40");
        let end = Step { in_bits: 0..7, out_bytes: 0..0, kind: StepKind::EndOfBlock };
        assert_eq!(symbol_label(&end), "end of block");
    }

    #[test]
    fn a_match_says_how_far_it_reached_and_how_far_back() {
        let step = Step { in_bits: 0..17, out_bytes: 5..17, kind: StepKind::Match { len: 12, dist: 5 } };
        assert_eq!(symbol_ty(&step, BlockKind::Fixed).0, "match 12 back 5");
    }

    #[test]
    fn a_run_length_code_is_named_by_what_it_did() {
        let (name, _) = table_field(TableField::Repeat { code: 18, count: 40, len: 0, dist: false }, 7);
        assert_eq!(name, "40 symbols with no code");
        let (name, _) = table_field(TableField::Repeat { code: 16, count: 5, len: 3, dist: true }, 2);
        assert_eq!(name, "repeat 3 for 5 more distance codes");
    }

    /// A table row holds a width, and says which alphabet's symbol the width
    /// is for. `distance 3` used to read as a distance of three bytes, which
    /// is a real thing in the same format and is not what the row holds.
    #[test]
    fn a_table_row_says_which_symbol_its_length_is_for() {
        let (name, _) = table_field(TableField::LitLen { sym: 77, len: 9 }, 4);
        assert_eq!(name, "code length for symbol 77");
        let (name, _) = table_field(TableField::Dist { sym: 3, len: 5 }, 4);
        assert_eq!(name, "code length for distance symbol 3");
        let (name, _) = table_field(TableField::CodeLen { sym: 16, len: 2 }, 3);
        assert_eq!(name, "code length for code-length symbol 16");
    }
}
