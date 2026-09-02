//! The fields a decoder's trace lays down over the run it read.
//!
//! Everything else in the IR is written down: a template says a field is
//! there and the reading finds out where. A deflate stream cannot be written
//! down. How many blocks it holds, how long each one's Huffman tables are and
//! how many symbols come after them are all answers the decoder finds while
//! decoding, and nothing short of decoding it can say. So the decoder writes
//! down what it read and this reads the fields back off that: one node per
//! block, one per code length, one per literal or match.
//!
//! Two rules keep it honest. Nothing is shown that the decoder did not read:
//! every node here stands for exactly one [`Step`], and its bits are that
//! step's bits. And no value is read twice: a literal's byte, a match's length
//! and distance, `hlit`, `hdist` are all the numbers the decoder used, carried
//! here as computed fields of no width rather than read again out of the bits
//! and possibly differently.
//!
//! A caveat on where a node sits. Deflate reads bits from the low end of a
//! byte upwards, and Qubero addresses bits from the high end downwards, so a
//! step's *byte* extent is the same either way and a step narrower than a byte
//! is highlighted at the other end of its byte from where deflate read it.
//! Nothing between the two conventions can fix that; a byte-aligned codec has
//! no such question.

use crate::codec::{Block, BlockKind, Step, StepField, StepKind, TableField, Trace};
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
        StepKind::Literal(_) | StepKind::Match { .. } | StepKind::Stored | StepKind::EndOfBlock | StepKind::Opaque
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
        _ => inner,
    };
    sized(bits, named)
}

/// One entry of a Huffman table, as the decoder read it.
fn table_field(table: TableField, bits: u64) -> (String, T) {
    let (name, value) = match table {
        TableField::CodeLen { sym, len } => (format!("code length {sym}"), len as i128),
        TableField::LitLen { sym, len } => (format!("symbol {sym}"), len as i128),
        TableField::Dist { sym, len } => (format!("distance {sym}"), len as i128),
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

/// One symbol: what it is, and what it said.
///
/// A literal is a byte. A match is a length and a distance. Neither has bits
/// of its own inside the symbol -- a match's length, its extra bits, its
/// distance code and *its* extra bits are four Huffman-coded runs with no
/// boundary a template could name -- so the parts are computed fields of no
/// width inside a record that is as wide as the symbol was.
pub(super) fn symbol_ty(step: &Step) -> (String, T) {
    let bits = step.in_bits.end - step.in_bits.start;
    let bytes = step.out_bytes.end - step.out_bytes.start;
    let kind = |k: i128| {
        T::enumeration(
            "SymbolKind",
            T::computed(E::lit(k)),
            &[(0, "literal"), (1, "match"), (2, "end of block"), (3, "stored"), (4, "not named")],
        )
    };
    let (name, fields) = match step.kind {
        StepKind::Literal(v) => (
            format!("literal {}", byte(v)),
            vec![("kind", kind(0)), ("value", T::computed(E::lit(v as i128)))],
        ),
        StepKind::Match { len, dist } => (
            format!("match {len} back {dist}"),
            vec![
                ("kind", kind(1)),
                ("length", T::computed(E::lit(len as i128))),
                ("distance", T::computed(E::lit(dist as i128))),
            ],
        ),
        StepKind::EndOfBlock => ("end of block".to_string(), vec![("kind", kind(2))]),
        StepKind::Stored => (
            "literals".to_string(),
            vec![("kind", kind(3)), ("length", T::computed(E::lit(bytes as i128)))],
        ),
        // A run the trace stopped naming, because there were too many of them
        // to name. See `codec::MAX_STEPS`.
        _ => (
            "symbols not named one at a time".to_string(),
            vec![("kind", kind(4)), ("length", T::computed(E::lit(bytes as i128)))],
        ),
    };
    (name, sized(bits, T::structure("Symbol", fields).counted_as("symbol")))
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
        assert_eq!(blocks.space, 0, "the blocks are bits of the file, not of the stream");
        assert_eq!(blocks.child_count, 1);
        // The one block starts where the run does and runs to the end of it.
        let block = e.node(&d, &[6, 1, 0]).unwrap();
        assert_eq!(block.offset_bits, run.offset_bits);
        assert!(block.name.ends_with("block, last"), "a block called {:?}", block.name);
    }

    /// A fixed-Huffman block's header is two fields and then its symbols; a
    /// dynamic one has its tables in between.
    #[test]
    fn a_block_shows_the_header_it_declared() {
        let (d, mut e) = reading(b"");
        let names: Vec<String> = (0..e.node(&d, &[6, 1, 0]).unwrap().child_count as usize)
            .map(|i| e.node(&d, &[6, 1, 0, i]).unwrap().name)
            .collect();
        assert_eq!(names, ["bfinal", "btype", "symbols"]);
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
        assert_eq!(names.last().unwrap(), "symbols");
        assert!(names.iter().any(|n| n.starts_with("code length ")), "no code-length rows in {names:?}");
        assert!(names.iter().any(|n| n.starts_with("symbol ")), "no literal/length rows in {names:?}");
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

    /// The symbols: a literal says its byte, a match says how far it reached
    /// and how far back.
    #[test]
    fn the_symbols_say_what_they_were() {
        let (d, mut e) = reading(b"abcabcabcabcabcabcabcabc");
        let symbols = {
            let block = e.node(&d, &[6, 1, 0]).unwrap();
            block.child_count as usize - 1
        };
        let run = e.node(&d, &[6, 1, 0, symbols]).unwrap();
        assert_eq!(run.name, "symbols");
        assert_eq!(run.unit.as_deref(), Some("symbol"));
        assert!(run.child_count >= 4, "only {} symbols", run.child_count);
        let first = e.node(&d, &[6, 1, 0, symbols, 0]).unwrap();
        assert_eq!(first.name, "literal 'a'");
        // Its `value` is the byte it stood for, and reads no bits of its own.
        let value = e.node(&d, &[6, 1, 0, symbols, 0, 1]).unwrap();
        assert_eq!(value.name, "value");
        assert_eq!(value.value.as_int(), Some(b'a' as i128));
        assert_eq!(value.size_bits, 0);
        // Somewhere in the run is a match, and it says both numbers.
        let names: Vec<String> = (0..run.child_count as usize)
            .map(|i| e.node(&d, &[6, 1, 0, symbols, i]).unwrap().name)
            .collect();
        assert!(names.iter().any(|n| n.starts_with("match ")), "no match in {names:?}");
        assert_eq!(names.last().unwrap(), "end of block");
    }

    /// A symbol's origin names the block whose tables decoded it: which nine
    /// bits a literal is, is a fact about that block and not about the bits.
    #[test]
    fn a_symbol_says_which_block_decoded_it() {
        let (d, mut e) = reading(&"the shape of the shape of the shape. ".repeat(80).into_bytes());
        let symbols = e.node(&d, &[6, 1, 0]).unwrap().child_count as usize - 1;
        let at = [6, 1, 0, symbols, 0];
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
        for path in [&[6usize, 1][..], &[6, 1, 0], &[6, 1, 0, 0], &[6, 1, 0, 1]] {
            assert!(!e.node(&d, path).unwrap().editable, "{path:?} is offered for editing");
        }
    }

    #[test]
    fn a_symbol_is_as_wide_as_the_bits_it_was_coded_in() {
        let step = Step { in_bits: 3..12, out_bytes: 0..1, kind: StepKind::Literal(b'q') };
        let (name, ty) = symbol_ty(&step);
        assert_eq!(name, "literal 'q'");
        assert!(matches!(ty, T::SizedBits { .. }));
    }

    #[test]
    fn a_match_says_how_far_it_reached_and_how_far_back() {
        let step = Step { in_bits: 0..17, out_bytes: 5..17, kind: StepKind::Match { len: 12, dist: 5 } };
        assert_eq!(symbol_ty(&step).0, "match 12 back 5");
    }

    #[test]
    fn a_run_length_code_is_named_by_what_it_did() {
        let (name, _) = table_field(TableField::Repeat { code: 18, count: 40, len: 0, dist: false }, 7);
        assert_eq!(name, "40 symbols with no code");
        let (name, _) = table_field(TableField::Repeat { code: 16, count: 5, len: 3, dist: true }, 2);
        assert_eq!(name, "repeat 3 for 5 more distance codes");
    }
}
