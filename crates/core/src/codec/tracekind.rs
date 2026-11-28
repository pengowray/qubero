//! What sort of node a decoder's trace laid down, as a fixed name such as
//! `deflate.literal` or `jpegscan.mcu`, apart from the values in the node's
//! display name.
//!
//! A node under a stream's `blocks` row is named by what the decoder read:
//! `literal 'a'`, `match 6 back 18`, `MCU 3, 4`. Those names change with every
//! value, so nothing outside the core can tell two literals are the same sort
//! of node except by reading the words. A [`TraceKind`] is that sort, chosen
//! where the node is placed from the step's [`StepKind`], its [`StepField`] or
//! [`TableField`], and where the node sits; never from its name.
//!
//! The names are per codec family, because the same step means different
//! things in different codecs: a [`StepKind::Stored`] step is a stored block's
//! payload in deflate and a run of literals in LZ4. Each family lists every
//! kind it can produce in a table, which is what [`kinds`] hands out, and a
//! node's kind is an index into its family's table, so a kind is two bytes
//! and costs no allocation.
//!
//! Each row also says what a node of its kind is: a line about it, the
//! [`Form`]s it can read as, and for a row over other nodes, which kinds its
//! children are. `eval::traced` builds each node's type from the same form
//! constants, so a code's name, a record's fields and an enum's cases are
//! written once. Which form a node takes is still chosen there, by its step,
//! where it sits and how its block was coded, and nothing ties that choice to
//! the forms a row lists except the walk over the samples in
//! `tests/trace_kinds_real.rs`.

use super::{Codec, StepField, StepKind, TableField};
use crate::eval::Sizing;

/// Which part a node plays in a trace, whatever the codec.
///
/// The step roles are [`StepKind`] with the values taken out, and a header
/// field's [`StepField`] kept, since that is what tells `hlit` from `hdist`.
/// The other four are the rows a trace's nodes are grouped under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The `blocks` row under a stream, over all of its blocks.
    Blocks,
    /// One block: a deflate block, a JPEG MCU.
    Block,
    /// The `codes` row of a block, over its symbols.
    Codes,
    /// One 8x8 block of a JPEG MCU. See [`super::Unit`].
    Unit,
    /// A step that is a named header field: [`StepKind::Header`].
    Field(StepField),
    /// [`TableField::CodeLen`]: a length in the code-length alphabet.
    CodeLengthCodeLength,
    /// [`TableField::LitLen`]: a literal/length code length.
    CodeLength,
    /// [`TableField::Dist`]: a distance code length.
    DistanceCodeLength,
    /// A [`TableField::Repeat`] that repeats the length before it.
    RepeatLength,
    /// A [`TableField::Repeat`] that writes a run of zero lengths.
    ZeroLengths,
    Literal,
    Match,
    Stored,
    EndOfBlock,
    /// [`StepKind::Block`]: a whole block the trace does not read inside.
    CompressedBlock,
    /// [`StepKind::Opaque`] inside a block: codes the trace stopped naming.
    Unnamed,
    /// [`StepKind::Opaque`] after a JPEG scan's last MCU.
    Trailing,
    Pixel,
    Filtered,
    Dc,
    Ac,
    Zrl,
    Eob,
    Base64,
    Ignored,
}

/// The codecs that name their nodes, each with its own table of kinds. Every
/// codec of a family produces the same sorts of node: zlib and gzip are
/// deflate with a wrapper round it, and the wrapper's steps are outside every
/// block, so they are never nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Deflate,
    JpegScan,
    Lz4,
    Snappy,
    FastLz,
    /// `.Z`, LZW codes.
    Compress,
    /// LHA's `-lh4-` to `-lh7-`.
    Lzhuf,
    Rar5,
    /// LZMA and LZMA2, raw, in lzip, and in xz.
    Lzma,
    /// PNG's scanlines, unfiltered a row at a time.
    PngScanlines,
    /// Bytes kept in the low bits of a picture's pixels.
    LowBits,
    CdfHuffman,
    Pico8,
}

impl Family {
    pub const ALL: &[Family] = &[
        Family::Deflate,
        Family::JpegScan,
        Family::Lz4,
        Family::Snappy,
        Family::FastLz,
        Family::Compress,
        Family::Lzhuf,
        Family::Rar5,
        Family::Lzma,
        Family::PngScanlines,
        Family::LowBits,
        Family::CdfHuffman,
        Family::Pico8,
    ];

    /// The family of a codec, or `None` for a codec whose nodes have no kind
    /// yet.
    pub fn of(codec: Codec) -> Option<Family> {
        match codec {
            Codec::Deflate | Codec::Zlib | Codec::Gzip => Some(Family::Deflate),
            Codec::JpegBaseline => Some(Family::JpegScan),
            Codec::Lz4Block | Codec::Lz4Frame => Some(Family::Lz4),
            Codec::Snappy => Some(Family::Snappy),
            Codec::FastLz => Some(Family::FastLz),
            Codec::Compress => Some(Family::Compress),
            Codec::Lha { .. } => Some(Family::Lzhuf),
            Codec::Rar5 { .. } => Some(Family::Rar5),
            Codec::Lzip | Codec::Lzma1 { .. } | Codec::Lzma2 | Codec::Xz => Some(Family::Lzma),
            Codec::PngScanlines { .. } | Codec::PngUnfilter { .. } => Some(Family::PngScanlines),
            Codec::LowBitsArgb | Codec::LowBitsRgba11 => Some(Family::LowBits),
            Codec::CdfHuffman | Codec::CdfAhuff => Some(Family::CdfHuffman),
            Codec::Pico8Pxa | Codec::Pico8Old => Some(Family::Pico8),
            // Not yet: pxu and IAR's two have no sample to test them by.
            Codec::PicotronPxu | Codec::IarLz77 | Codec::IarPackbits => None,
            // These lay down no blocks, so they have no nodes to name.
            Codec::CfbWorkbook
            | Codec::Stored
            | Codec::Zstd
            | Codec::Brotli
            | Codec::Bzip2
            | Codec::CdfRle
            | Codec::Latin1Text
            | Codec::EscapedLatin1Text
            | Codec::Base64
            | Codec::QuotedPrintable
            | Codec::QEncoding
            | Codec::TascamScramble => None,
        }
    }

    /// The word every kind of the family starts with: `deflate` for
    /// `deflate.literal`.
    pub fn as_str(self) -> &'static str {
        match self {
            Family::Deflate => "deflate",
            Family::JpegScan => "jpegscan",
            Family::Lz4 => "lz4",
            Family::Snappy => "snappy",
            Family::FastLz => "fastlz",
            Family::Compress => "compress",
            Family::Lzhuf => "lzhuf",
            Family::Rar5 => "rar5",
            Family::Lzma => "lzma",
            Family::PngScanlines => "png",
            Family::LowBits => "lowbits",
            Family::CdfHuffman => "cdfhuffman",
            Family::Pico8 => "pico8",
        }
    }

    /// The family whose kinds start with `name`, as [`Family::as_str`] gives
    /// it.
    pub fn named(name: &str) -> Option<Family> {
        Family::ALL.iter().copied().find(|f| f.as_str() == name)
    }

    /// Every kind this family's nodes can be, in the order a reader meets
    /// them.
    pub fn kinds(self) -> &'static [KindEntry] {
        match self {
            Family::Deflate => DEFLATE,
            Family::JpegScan => JPEG_SCAN,
            Family::Lz4 => LZ4,
            Family::Snappy => SNAPPY,
            Family::FastLz => FASTLZ,
            Family::Compress => COMPRESS,
            Family::Lzhuf => LZHUF,
            Family::Rar5 => RAR5,
            Family::Lzma => LZMA,
            Family::PngScanlines => PNG_SCANLINES,
            Family::LowBits => LOW_BITS,
            Family::CdfHuffman => CDF_HUFFMAN,
            Family::Pico8 => PICO8,
        }
    }

    /// The row of this family's table for `role`.
    pub fn entry(self, role: Role) -> Option<&'static KindEntry> {
        self.kinds().iter().find(|e| e.role == role)
    }

    /// The role a step plays in this family. Only the repeat codes of a
    /// Huffman table need the family: deflate's code 16 repeats the length
    /// before it and 17 and 18 write zeros; RAR 5's 16 and 17 repeat and its
    /// 15, 18 and 19 write zeros; every one of LHA's writes zeros.
    pub fn step_role(self, kind: StepKind, outside_units: bool) -> Role {
        match kind {
            StepKind::Header(field, _) => Role::Field(field),
            StepKind::Table(TableField::CodeLen { .. }) => Role::CodeLengthCodeLength,
            StepKind::Table(TableField::LitLen { .. }) => Role::CodeLength,
            StepKind::Table(TableField::Dist { .. }) => Role::DistanceCodeLength,
            StepKind::Table(TableField::Repeat { code, .. }) => match repeats_length(Some(self), code) {
                true => Role::RepeatLength,
                false => Role::ZeroLengths,
            },
            StepKind::Literal(_) => Role::Literal,
            StepKind::Match { .. } => Role::Match,
            StepKind::Stored => Role::Stored,
            StepKind::EndOfBlock => Role::EndOfBlock,
            StepKind::Block => Role::CompressedBlock,
            StepKind::Opaque if outside_units => Role::Trailing,
            StepKind::Opaque => Role::Unnamed,
            StepKind::Pixel => Role::Pixel,
            StepKind::Filtered => Role::Filtered,
            StepKind::Dc { .. } => Role::Dc,
            StepKind::Ac { .. } => Role::Ac,
            StepKind::Zrl { .. } => Role::Zrl,
            StepKind::Eob { .. } => Role::Eob,
            StepKind::Base64 { .. } => Role::Base64,
            StepKind::Ignored => Role::Ignored,
        }
    }
}

/// Whether repeat code `code` of a Huffman table repeats the length before
/// it, rather than writing zeros. Deflate's 16 repeats; RAR 5's 16 and 17
/// repeat; every other repeat code writes zeros. A codec with no family is
/// taken to follow deflate.
pub fn repeats_length(family: Option<Family>, code: u8) -> bool {
    match (family, code) {
        (Some(Family::Rar5), 16 | 17) => true,
        (Some(Family::Deflate) | None, 16) => true,
        _ => false,
    }
}

/// A `Ty::Enum` a node can read as: the enum's name, and the name of each
/// value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumDef {
    pub name: &'static str,
    pub cases: &'static [(i128, &'static str)],
}

/// RFC 1951's BFINAL bit.
pub const DEFLATE_FINAL: EnumDef = EnumDef { name: "DeflateFinal", cases: &[(0, "more blocks follow"), (1, "the last block")] };

/// RFC 1951's BTYPE.
pub const DEFLATE_BLOCK_TYPE: EnumDef = EnumDef {
    name: "DeflateBlockType",
    cases: &[(0, "stored"), (1, "fixed Huffman"), (2, "dynamic Huffman"), (3, "reserved")],
};

/// RFC 2083 section 6: how the encoder predicted a scanline.
pub const PNG_FILTER: EnumDef =
    EnumDef { name: "PngFilter", cases: &[(0, "none"), (1, "sub"), (2, "up"), (3, "average"), (4, "paeth")] };

/// T.81's own names for the eight restart markers, which count 0 to 7 and
/// round again.
pub const RESTART_MARKER: EnumDef = EnumDef {
    name: "RestartMarker",
    cases: &[(0, "RST0"), (1, "RST1"), (2, "RST2"), (3, "RST3"), (4, "RST4"), (5, "RST5"), (6, "RST6"), (7, "RST7")],
};

/// The `kind` field of a [`Reads::Struct`].
pub const SYMBOL_KIND: EnumDef = EnumDef {
    name: "SymbolKind",
    cases: &[
        (0, "literal"),
        (1, "match"),
        (2, "end of block"),
        (3, "stored"),
        (4, "not named"),
        (5, "pixel"),
        (6, "padding"),
        (7, "base64 group"),
        (8, "ignored"),
    ],
};

/// The name of the struct a [`Reads::Struct`] is.
pub const RECORD_NAME: &str = "Symbol";

/// The enum a header field's value is named by, for the fields whose format
/// names its values. The rest read as plain numbers.
pub fn header_enum(field: StepField) -> Option<&'static EnumDef> {
    match field {
        StepField::Bfinal => Some(&DEFLATE_FINAL),
        StepField::Btype => Some(&DEFLATE_BLOCK_TYPE),
        StepField::Filter => Some(&PNG_FILTER),
        StepField::Restart => Some(&RESTART_MARKER),
        _ => None,
    }
}

/// What a node reads as, in the IR's terms. Each is one `Ty`, inside a
/// `Ty::SizedBits` as wide as the bits the decoder read for it. The variants
/// are named as `template_text` writes the `Ty`: `traced`, `computed`,
/// `enum`, `codebits`, a struct, `bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reads {
    /// `Ty::Traced`: a row over other nodes, with no value of its own. The
    /// string is what the type column says: `blocks`, `block` or `codes`.
    /// `template_text` writes the `Ty` as `traced blocks`, `traced block`,
    /// `traced symbols` and `traced unit`.
    Traced(&'static str),
    /// `Ty::Computed`: a number the decoder read, carried as it used it.
    Computed,
    /// `Ty::Enum` over a computed number.
    Enum(&'static EnumDef),
    /// `Ty::CodeBits`: an entropy-coded symbol shown as its own bits. The
    /// string is what the type column says. A JPEG code that lies across a
    /// stuffed `00` byte adds `, plus a stuffed 00 byte` to it, or the count
    /// of them.
    CodeBits(&'static str),
    /// `Ty::Struct` named [`RECORD_NAME`]: a step that is not one code. The
    /// first field is always `kind`, a [`SYMBOL_KIND`] whose value is `kind`
    /// here; the others are computed numbers: `value` is a literal's byte,
    /// `length` how many bytes the step produced or a match copied, and
    /// `distance` how far back a match copied from.
    Struct { kind: i128, fields: &'static [&'static str] },
    /// `Ty::Bytes`: the step's bytes as they sit in the stream.
    Bytes,
}

/// One way a node of a kind can read: its type, and how its width was
/// settled. A kind has more than one when the same step reads differently by
/// where it sits or by how its block was coded: a deflate literal is a code
/// of fixed width in a fixed-Huffman block and one its block's table sized in
/// a dynamic one, and an LZ4 token is a number in front of a block's first
/// literals and a record after them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Form {
    pub reads: Reads,
    /// [`Sizing::Fixed`] when the format fixed the width, [`Sizing::Table`]
    /// when a code-length table in the file did, [`Sizing::Encoded`] when the
    /// bits themselves say where they end, [`Sizing::Trace`] when the decoder
    /// measured it, and [`Sizing::Nothing`] for a step of no width.
    pub sized: Sizing,
}

const fn form(reads: Reads, sized: Sizing) -> Form {
    Form { reads, sized }
}

/// A row over other nodes.
const fn rows(name: &'static str) -> Form {
    form(Reads::Traced(name), Sizing::Trace)
}

/// A header field or table entry the decoder read.
pub const NUMBER: Form = form(Reads::Computed, Sizing::Trace);
/// A number the decoder worked out that the run does not hold.
pub const NO_BITS: Form = form(Reads::Computed, Sizing::Nothing);

/// A header field whose format names its values. Each of these has a width
/// its format fixes: 1 bit, 2 bits, a byte, a 2-byte marker.
const fn named(def: &'static EnumDef) -> Form {
    form(Reads::Enum(def), Sizing::Fixed)
}

/// A literal or end-of-block code in a fixed-Huffman block.
pub const HUFFMAN_FIXED: Form = form(Reads::CodeBits("Huffman code"), Sizing::Fixed);
/// A literal or end-of-block code in a block that wrote its own table; and a
/// JPEG code that carries no value bits.
pub const HUFFMAN_TABLE: Form = form(Reads::CodeBits("Huffman code"), Sizing::Table);
/// A match: a length code, its extra bits, a distance code and its extra
/// bits.
pub const LENGTH_AND_DISTANCE: Form = form(Reads::CodeBits("length and distance codes"), Sizing::Encoded);
/// A JPEG DC or AC code and the value bits after it.
pub const CODE_AND_VALUE_BITS: Form = form(Reads::CodeBits("Huffman code and value bits"), Sizing::Encoded);

const fn record(kind: i128, fields: &'static [&'static str]) -> Form {
    form(Reads::Struct { kind, fields }, Sizing::Trace)
}

pub const LITERAL_RECORD: Form = record(0, &["kind", "value"]);
pub const MATCH_RECORD: Form = record(1, &["kind", "length", "distance"]);
pub const END_RECORD: Form = record(2, &["kind"]);
pub const STORED_RECORD: Form = record(3, &["kind", "length"]);
/// Codes the trace stopped naming, and a header field among a block's codes.
pub const UNNAMED_RECORD: Form = record(4, &["kind", "length"]);
pub const PIXEL_RECORD: Form = record(5, &["kind", "length"]);
pub const PADDING_RECORD: Form = record(6, &["kind"]);
pub const BASE64_RECORD: Form = record(7, &["kind", "length"]);
pub const IGNORED_RECORD: Form = record(8, &["kind"]);

/// A PNG scanline's bytes after its filter byte.
pub const FILTERED_BYTES: Form = form(Reads::Bytes, Sizing::Trace);

/// One row of a family's table: a role, the name a node in that role is
/// given, and what such a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindEntry {
    pub role: Role,
    pub kind: &'static str,
    /// One line for a programmer: what a node of this kind is in the format.
    pub about: &'static str,
    /// Every form a node of this kind can take. Never empty.
    pub forms: &'static [Form],
    /// For a row over other nodes, the roles its children can have, each a
    /// row of the same table. Empty for a node with no children of its own
    /// kind; a [`Reads::Struct`]'s fields are not kinds.
    pub children: &'static [Role],
}

impl KindEntry {
    /// The kinds of this row's children, from `family`'s table.
    pub fn child_kinds(&self, family: Family) -> impl Iterator<Item = &'static str> + '_ {
        self.children.iter().filter_map(move |&r| family.entry(r).map(|e| e.kind))
    }
}

/// A row over other nodes.
const fn r(role: Role, kind: &'static str, about: &'static str, form: &'static [Form], children: &'static [Role]) -> KindEntry {
    KindEntry { role, kind, about, forms: form, children }
}

/// A node with no children of its own kind.
const fn k(role: Role, kind: &'static str, about: &'static str, forms: &'static [Form]) -> KindEntry {
    KindEntry { role, kind, about, forms, children: &[] }
}

const BLOCKS: &[Form] = &[rows("blocks")];
const BLOCK: &[Form] = &[rows("block")];
const CODES: &[Form] = &[rows("codes")];
/// A header field that is read in front of a block's codes, and is a record
/// when it comes among them.
const NUMBER_OR_RECORD: &[Form] = &[NUMBER, UNNAMED_RECORD];
/// A literal or end-of-block in a deflate block, fixed or dynamic.
const HUFFMAN_EITHER: &[Form] = &[HUFFMAN_FIXED, HUFFMAN_TABLE];

use Role as R;
use StepField as F;

/// RFC 1951, read by `codec::inflate` for raw deflate, zlib and gzip.
const DEFLATE: &[KindEntry] = &[
    r(R::Blocks, "deflate.blocks", "Every block of the deflate stream, in order.", BLOCKS, &[R::Block]),
    r(
        R::Block,
        "deflate.block",
        "One deflate block: its header, its Huffman tables if it is dynamic, and its codes.",
        BLOCK,
        &[
            R::Field(F::Bfinal),
            R::Field(F::Btype),
            R::Field(F::Hlit),
            R::Field(F::Hdist),
            R::Field(F::Hclen),
            R::CodeLengthCodeLength,
            R::CodeLength,
            R::DistanceCodeLength,
            R::RepeatLength,
            R::ZeroLengths,
            R::Field(F::Padding),
            R::Field(F::StoredLen),
            R::Field(F::StoredNlen),
            R::Codes,
        ],
    ),
    k(R::Field(F::Bfinal), "deflate.bfinal", "BFINAL: 1 bit, set on the stream's last block.", &[named(&DEFLATE_FINAL)]),
    k(R::Field(F::Btype), "deflate.btype", "BTYPE: 2 bits, stored, fixed Huffman or dynamic Huffman.", &[named(&DEFLATE_BLOCK_TYPE)]),
    k(R::Field(F::Hlit), "deflate.hlit", "HLIT: 5 bits, the number of literal/length code lengths less 257.", &[NUMBER]),
    k(R::Field(F::Hdist), "deflate.hdist", "HDIST: 5 bits, the number of distance code lengths less 1.", &[NUMBER]),
    k(R::Field(F::Hclen), "deflate.hclen", "HCLEN: 4 bits, the number of code-length code lengths less 4.", &[NUMBER]),
    k(
        R::CodeLengthCodeLength,
        "deflate.code_length_code_length",
        "3 bits: the code length of one symbol of the code-length alphabet.",
        &[NUMBER],
    ),
    k(R::CodeLength, "deflate.code_length", "A code-length code giving one literal/length symbol's code length.", &[NUMBER]),
    k(R::DistanceCodeLength, "deflate.distance_code_length", "A code-length code giving one distance symbol's code length.", &[NUMBER]),
    k(R::RepeatLength, "deflate.code_length_repeat", "Code 16 and 2 extra bits: the previous code length, 3 to 6 more times.", &[NUMBER]),
    k(R::ZeroLengths, "deflate.code_length_zeros", "Code 17 or 18 and its extra bits: a run of symbols with no code.", &[NUMBER]),
    // A stored block: the bits up to the byte boundary, LEN and NLEN, and
    // then its bytes.
    k(R::Field(F::Padding), "deflate.padding", "The bits a stored block skips to reach a byte boundary.", &[NUMBER]),
    k(R::Field(F::StoredLen), "deflate.stored_len", "LEN: 16 bits, the number of bytes in a stored block.", &[NUMBER]),
    k(R::Field(F::StoredNlen), "deflate.stored_nlen", "NLEN: 16 bits, the one's complement of LEN.", &[NUMBER]),
    r(R::Codes, "deflate.codes", "A block's payload: its codes, or a stored block's bytes.", CODES, &[R::Stored, R::Literal, R::Match, R::EndOfBlock, R::Unnamed]),
    k(R::Stored, "deflate.stored_data", "A stored block's bytes, copied to the output as they are.", &[STORED_RECORD]),
    k(R::Literal, "deflate.literal", "A literal/length code below 256: one byte of output.", HUFFMAN_EITHER),
    k(R::Match, "deflate.match", "A length code and a distance code, each with its extra bits: a copy of earlier output.", &[LENGTH_AND_DISTANCE]),
    k(R::EndOfBlock, "deflate.end_of_block", "Code 256, which ends the block.", HUFFMAN_EITHER),
    // A block's codes once the trace stopped naming them. See `MAX_STEPS`.
    k(R::Unnamed, "deflate.unnamed_codes", "The rest of a block's codes, as one node after the trace's limit of named steps.", &[UNNAMED_RECORD]),
];

/// A sequential Huffman-coded JPEG scan, read by `codec::jpeg`.
const JPEG_SCAN: &[KindEntry] = &[
    r(R::Blocks, "jpegscan.mcus", "Every MCU of the scan, in order.", BLOCKS, &[R::Block]),
    r(
        R::Block,
        "jpegscan.mcu",
        "One minimum coded unit: each component's 8x8 blocks for one area of the image, in the scan's order.",
        BLOCK,
        &[R::Unit, R::Field(F::Padding), R::Field(F::Restart), R::Trailing],
    ),
    r(R::Unit, "jpegscan.unit", "One 8x8 block of one component: its DC code, then its AC codes.", BLOCK, &[R::Dc, R::Ac, R::Zrl, R::Eob, R::Unnamed]),
    k(R::Dc, "jpegscan.dc", "A DC code and its value bits: the change from the same component's previous DC.", &[HUFFMAN_TABLE, CODE_AND_VALUE_BITS]),
    k(R::Ac, "jpegscan.ac", "An AC code and its value bits: a run of zeros and one nonzero coefficient.", &[CODE_AND_VALUE_BITS]),
    k(R::Zrl, "jpegscan.zrl", "The code for AC symbol 0xF0: sixteen zero coefficients.", &[HUFFMAN_TABLE]),
    k(R::Eob, "jpegscan.eob", "The code for AC symbol 0x00: every coefficient left in the block is zero.", &[HUFFMAN_TABLE]),
    // A unit's codes once the trace stopped naming them. See `MAX_STEPS`.
    k(R::Unnamed, "jpegscan.unnamed_codes", "The rest of a block's codes, as one node after the trace's limit of named steps.", &[UNNAMED_RECORD]),
    // The ones that pad an interval out to a byte, and the marker after it.
    k(R::Field(F::Padding), "jpegscan.padding", "The 1 bits that fill out the last byte of a restart interval or of the scan.", &[PADDING_RECORD]),
    k(R::Field(F::Restart), "jpegscan.restart", "A restart marker, FFD0 to FFD7, between two intervals.", &[named(&RESTART_MARKER)]),
    // Bytes after the last MCU that no code read.
    k(R::Trailing, "jpegscan.trailing", "Bytes after the last MCU that no code read.", &[UNNAMED_RECORD]),
];

/// LZ4 blocks, alone or in a frame, read by `codec::lz4`. A frame's block
/// starts with its size word; its checksums are outside every block.
const LZ4: &[KindEntry] = &[
    r(R::Blocks, "lz4.blocks", "Every LZ4 block, in order.", BLOCKS, &[R::Block]),
    r(
        R::Block,
        "lz4.block",
        "One LZ4 block: a frame's block size word, then its sequences.",
        BLOCK,
        &[R::Field(F::BlockHeader), R::Field(F::Token), R::Field(F::LengthExtra), R::Field(F::Offset), R::Codes],
    ),
    k(R::Field(F::BlockHeader), "lz4.block_size", "A frame's 32-bit block size word; its high bit marks a stored block.", &[NUMBER]),
    k(R::Field(F::Token), "lz4.token", "A sequence's token: 4 bits of literal length and 4 of match length.", NUMBER_OR_RECORD),
    k(R::Field(F::LengthExtra), "lz4.length_extra", "Bytes adding to a length that did not fit in the token.", NUMBER_OR_RECORD),
    r(R::Codes, "lz4.codes", "A block's sequences, from its first literals on.", CODES, &[R::Field(F::Token), R::Field(F::LengthExtra), R::Stored, R::Field(F::Offset), R::Match, R::Unnamed]),
    k(R::Stored, "lz4.literals", "A sequence's literal bytes, or a stored block's bytes, copied as they are.", &[STORED_RECORD]),
    k(R::Field(F::Offset), "lz4.offset", "A match's 16-bit little-endian offset.", NUMBER_OR_RECORD),
    k(R::Match, "lz4.match", "The copy a match makes, from the offset and length before it.", &[MATCH_RECORD]),
    k(R::Unnamed, "lz4.unnamed_codes", "The rest of a block's sequences, as one node after the trace's limit of named steps.", &[UNNAMED_RECORD]),
];

/// One raw Snappy block, read by `codec::snappy`. The length in front of it
/// is outside the block.
const SNAPPY: &[KindEntry] = &[
    r(R::Blocks, "snappy.blocks", "Every Snappy block, in order.", BLOCKS, &[R::Block]),
    r(R::Block, "snappy.block", "One Snappy block: a run of literal and copy elements.", BLOCK, &[R::Field(F::Token), R::Field(F::LengthExtra), R::Codes]),
    k(R::Field(F::Token), "snappy.tag", "An element's tag byte: its type in the low 2 bits and a length.", NUMBER_OR_RECORD),
    k(R::Field(F::LengthExtra), "snappy.length_extra", "Bytes giving a literal's length when the tag could not.", NUMBER_OR_RECORD),
    r(R::Codes, "snappy.codes", "A block's elements, from its first literal bytes on.", CODES, &[R::Field(F::Token), R::Field(F::LengthExtra), R::Stored, R::Field(F::Offset), R::Match, R::Unnamed]),
    k(R::Stored, "snappy.literals", "A literal element's bytes, copied as they are.", &[STORED_RECORD]),
    // A block starts with literal bytes, since there is nothing yet to copy.
    k(R::Field(F::Offset), "snappy.offset", "A copy element's offset, 1, 2 or 4 bytes.", &[UNNAMED_RECORD]),
    k(R::Match, "snappy.match", "The copy a copy element makes, from the offset and length before it.", &[MATCH_RECORD]),
    k(R::Unnamed, "snappy.unnamed_codes", "The rest of a block's elements, as one node after the trace's limit of named steps.", &[UNNAMED_RECORD]),
];

/// One FastLZ block, read by `codec::fastlz`.
const FASTLZ: &[KindEntry] = &[
    r(R::Blocks, "fastlz.blocks", "Every FastLZ block, in order.", BLOCKS, &[R::Block]),
    r(R::Block, "fastlz.block", "One FastLZ block: a run of literal runs and matches.", BLOCK, &[R::Field(F::Token), R::Codes]),
    k(R::Field(F::Token), "fastlz.token", "An instruction byte: a literal run's length, or a match's length and high offset bits.", NUMBER_OR_RECORD),
    // A block starts with a literal run, so these are always among its codes.
    k(R::Field(F::LengthExtra), "fastlz.length_extra", "Bytes adding to a match's length.", &[UNNAMED_RECORD]),
    k(R::Field(F::Offset), "fastlz.offset", "A match's low offset byte, or bytes.", &[UNNAMED_RECORD]),
    r(
        R::Codes,
        "fastlz.codes",
        "A block's instructions, from its first literal run on.",
        CODES,
        &[R::Field(F::Token), R::Field(F::LengthExtra), R::Field(F::Offset), R::Stored, R::Match, R::Field(F::Padding), R::Unnamed],
    ),
    k(R::Stored, "fastlz.literals", "A literal run's bytes, copied as they are.", &[STORED_RECORD]),
    k(R::Match, "fastlz.match", "The copy a match makes, from the length and offset before it.", &[MATCH_RECORD]),
    k(R::Field(F::Padding), "fastlz.padding", "Bytes after the last instruction that no instruction read.", &[PADDING_RECORD]),
    k(R::Unnamed, "fastlz.unnamed_codes", "The rest of a block's instructions, as one node after the trace's limit of named steps.", &[UNNAMED_RECORD]),
];

/// A `.Z` file's LZW codes, read by `codec::compress`. A block runs to a
/// clear code, and the zero bits that pad a group of codes when the width
/// changes are inside it.
const COMPRESS: &[KindEntry] = &[
    r(R::Blocks, "compress.blocks", "The stream's runs of codes, each ending at a clear code.", BLOCKS, &[R::Block]),
    r(R::Block, "compress.block", "The codes from one dictionary reset to the next.", BLOCK, &[R::Codes]),
    r(R::Codes, "compress.codes", "A block's LZW codes.", CODES, &[R::Literal, R::Match, R::EndOfBlock, R::Field(F::Padding), R::Unnamed]),
    k(R::Literal, "compress.literal", "A code below 256: one byte of output.", &[LITERAL_RECORD]),
    k(R::Match, "compress.match", "A code above 255 other than the clear code: a string the dictionary holds, copied from earlier output.", &[MATCH_RECORD]),
    k(R::EndOfBlock, "compress.clear", "The clear code, 256, which resets the dictionary.", &[END_RECORD]),
    k(R::Field(F::Padding), "compress.padding", "Zero bits filling out a group of 8 codes when the code width grows or the dictionary is reset.", &[PADDING_RECORD]),
    k(R::Unnamed, "compress.unnamed_codes", "The rest of a block's codes, as one node after the trace's limit of named steps.", &[UNNAMED_RECORD]),
];

/// LHA's `-lh5-` family, read by `codec::lha`: per block, a symbol count,
/// three Huffman tables and the codes.
const LZHUF: &[KindEntry] = &[
    r(R::Blocks, "lzhuf.blocks", "Every block of the member's data, in order.", BLOCKS, &[R::Block]),
    r(
        R::Block,
        "lzhuf.block",
        "One block: its code count, three Huffman tables and its codes.",
        BLOCK,
        &[R::Field(F::BlockHeader), R::CodeLengthCodeLength, R::CodeLength, R::DistanceCodeLength, R::ZeroLengths, R::Codes],
    ),
    k(R::Field(F::BlockHeader), "lzhuf.symbol_count", "16 bits: how many codes the block holds.", &[NUMBER]),
    k(R::CodeLengthCodeLength, "lzhuf.code_length_code_length", "One code length of the table the literal/length table is coded with.", &[NUMBER]),
    k(R::CodeLength, "lzhuf.code_length", "One literal/length symbol's code length.", &[NUMBER]),
    k(R::DistanceCodeLength, "lzhuf.offset_code_length", "One offset symbol's code length.", &[NUMBER]),
    k(R::ZeroLengths, "lzhuf.code_length_zeros", "A run of symbols with no code.", &[NUMBER]),
    r(R::Codes, "lzhuf.codes", "A block's codes.", CODES, &[R::Literal, R::Match, R::Unnamed]),
    k(R::Literal, "lzhuf.literal", "A code below 256: one byte of output.", &[HUFFMAN_TABLE]),
    k(R::Match, "lzhuf.match", "A length code and an offset code, each with its extra bits: a copy of earlier output.", &[LENGTH_AND_DISTANCE]),
    k(R::Unnamed, "lzhuf.unnamed_codes", "The rest of a block's codes, as one node after the trace's limit of named steps.", &[UNNAMED_RECORD]),
];

/// RAR 5's compressed blocks, read by `codec::rar5`.
const RAR5: &[KindEntry] = &[
    r(R::Blocks, "rar5.blocks", "Every compressed block of the file's data, in order.", BLOCKS, &[R::Block]),
    r(
        R::Block,
        "rar5.block",
        "One compressed block: its header, its Huffman tables if it has new ones, and its codes.",
        BLOCK,
        &[
            R::Field(F::BlockHeader),
            R::CodeLengthCodeLength,
            R::CodeLength,
            R::DistanceCodeLength,
            R::RepeatLength,
            R::ZeroLengths,
            R::Field(F::FilterDef),
            R::Codes,
        ],
    ),
    k(R::Field(F::BlockHeader), "rar5.block_header", "The block's flags, checksum and size.", &[NUMBER]),
    k(R::CodeLengthCodeLength, "rar5.code_length_code_length", "4 bits: one code length of the table the other tables are coded with.", &[NUMBER]),
    k(R::CodeLength, "rar5.code_length", "One literal/length symbol's code length.", &[NUMBER]),
    k(R::DistanceCodeLength, "rar5.distance_code_length", "One distance, alignment or length symbol's code length.", &[NUMBER]),
    k(R::RepeatLength, "rar5.code_length_repeat", "Code 16 or 17 and its count: the previous code length, repeated.", &[NUMBER]),
    k(R::ZeroLengths, "rar5.code_length_zeros", "Code 18 or 19 and its count, or in the first table a 15 and a count nibble: a run of zero lengths.", &[NUMBER]),
    r(R::Codes, "rar5.codes", "A block's codes.", CODES, &[R::Literal, R::Match, R::Field(F::FilterDef), R::Field(F::Padding), R::Unnamed]),
    k(R::Literal, "rar5.literal", "A code below 256: one byte of output.", &[HUFFMAN_TABLE]),
    k(R::Match, "rar5.match", "A length code and a distance code, each with its extra bits, or a repeat of a recent distance: a copy of earlier output.", &[LENGTH_AND_DISTANCE]),
    k(R::Field(F::FilterDef), "rar5.filter", "Code 256 and its fields: a filter to run over a range of the output.", NUMBER_OR_RECORD),
    k(R::Field(F::Padding), "rar5.padding", "The bits after a block's last code, up to where its header said it ends.", &[PADDING_RECORD]),
    k(R::Unnamed, "rar5.unnamed_codes", "The rest of a block's codes, as one node after the trace's limit of named steps.", &[UNNAMED_RECORD]),
];

/// LZMA, read by `codec::lzma`: one block for an LZMA1 stream, one per chunk
/// for LZMA2, raw, in lzip or in xz. A container's own headers and checks are
/// outside every block.
const LZMA: &[KindEntry] = &[
    r(R::Blocks, "lzma.blocks", "The LZMA1 stream as one block, or each LZMA2 chunk.", BLOCKS, &[R::Block]),
    r(
        R::Block,
        "lzma.block",
        "One LZMA1 stream or LZMA2 chunk: its header, then its range-coded symbols.",
        BLOCK,
        &[R::Field(F::BlockHeader), R::Field(F::LzmaProps), R::Field(F::RangeInit), R::Codes],
    ),
    k(R::Field(F::BlockHeader), "lzma.chunk_header", "An LZMA2 chunk's control byte and sizes.", &[NUMBER]),
    k(R::Field(F::LzmaProps), "lzma.properties", "The lc, lp and pb byte; of no width where the container gave it.", &[NUMBER, NO_BITS]),
    k(R::Field(F::RangeInit), "lzma.range_init", "The 5 bytes that start the range decoder.", &[NUMBER]),
    r(R::Codes, "lzma.codes", "A block's symbols, or an uncompressed chunk's bytes.", CODES, &[R::Literal, R::Match, R::EndOfBlock, R::Stored, R::Unnamed]),
    k(R::Literal, "lzma.literal", "A range-coded literal: one byte of output.", &[LITERAL_RECORD]),
    k(R::Match, "lzma.match", "A range-coded match or repeated match: a copy of earlier output.", &[MATCH_RECORD]),
    k(R::EndOfBlock, "lzma.end_marker", "The end-of-stream marker, a match with distance 0xFFFFFFFF.", &[END_RECORD]),
    k(R::Stored, "lzma.stored_data", "An uncompressed LZMA2 chunk's bytes, copied as they are.", &[STORED_RECORD]),
    k(R::Unnamed, "lzma.unnamed_codes", "The rest of a block's symbols, as one node after the trace's limit of named steps.", &[UNNAMED_RECORD]),
];

/// PNG's scanlines, read by `codec::scanlines` and `codec::pixels`. A row's
/// pass and row number are steps of no width.
const PNG_SCANLINES: &[KindEntry] = &[
    r(R::Blocks, "png.scanlines", "Every scanline of the image, pass by pass when it is interlaced.", BLOCKS, &[R::Block]),
    r(R::Block, "png.scanline", "One scanline: its filter byte and its filtered bytes.", BLOCK, &[R::Field(F::Pass), R::Field(F::Row), R::Field(F::Filter), R::Filtered]),
    k(R::Field(F::Pass), "png.pass", "Which Adam7 pass the scanline is in, 1 to 7; worked out, not stored.", &[NO_BITS]),
    k(R::Field(F::Row), "png.row", "Which row of its pass the scanline is, from 0; worked out, not stored.", &[NO_BITS]),
    k(R::Field(F::Filter), "png.filter", "The filter type byte in front of the scanline.", &[named(&PNG_FILTER)]),
    k(R::Filtered, "png.filtered_row", "The scanline's bytes after its filter byte, still filtered.", &[FILTERED_BYTES]),
];

/// Bytes kept in the low bits of a picture's pixels, read by
/// `codec::pixels`.
const LOW_BITS: &[KindEntry] = &[
    r(R::Blocks, "lowbits.blocks", "The pixels that carry the hidden bytes.", BLOCKS, &[R::Block]),
    r(R::Block, "lowbits.pixels", "A run of pixels, read in order.", BLOCK, &[R::Codes]),
    r(R::Codes, "lowbits.codes", "The pixels, one at a time.", CODES, &[R::Pixel]),
    k(R::Pixel, "lowbits.pixel", "One pixel whose low bits carry a byte, or two.", &[PIXEL_RECORD]),
];

/// NASA CDF's Huffman and adaptive Huffman coding, read by `codec::cdfhuff`.
const CDF_HUFFMAN: &[KindEntry] = &[
    r(R::Blocks, "cdfhuffman.blocks", "The coded run as one block.", BLOCKS, &[R::Block]),
    r(R::Block, "cdfhuffman.block", "The byte counts the tree is built from, if any, then the codes.", BLOCK, &[R::Field(F::FrequencyTable), R::Codes]),
    k(R::Field(F::FrequencyTable), "cdfhuffman.frequency_table", "The runs of byte counts the Huffman tree is built from.", &[NUMBER]),
    r(R::Codes, "cdfhuffman.codes", "The block's codes.", CODES, &[R::Literal, R::EndOfBlock, R::Field(F::Padding), R::Unnamed]),
    k(R::Literal, "cdfhuffman.literal", "A Huffman code: one byte of output.", &[HUFFMAN_TABLE]),
    k(R::EndOfBlock, "cdfhuffman.end_of_stream", "The code that ends the stream.", &[HUFFMAN_TABLE]),
    k(R::Field(F::Padding), "cdfhuffman.padding", "The bits after the end code, to the end of the run.", &[PADDING_RECORD]),
    k(R::Unnamed, "cdfhuffman.unnamed_codes", "The rest of the codes, as one node after the trace's limit of named steps.", &[UNNAMED_RECORD]),
];

/// PICO-8's two code compressions, read by `codec::pico8`.
const PICO8: &[KindEntry] = &[
    r(R::Blocks, "pico8.blocks", "The compressed code as one block.", BLOCKS, &[R::Block]),
    r(R::Block, "pico8.block", "The compressed code's literals and matches.", BLOCK, &[R::Codes]),
    r(R::Codes, "pico8.codes", "The block's literals and matches.", CODES, &[R::Literal, R::Match, R::EndOfBlock]),
    k(R::Literal, "pico8.literal", "One character of the code.", &[LITERAL_RECORD]),
    k(R::Match, "pico8.match", "A copy of earlier output.", &[MATCH_RECORD]),
    k(R::EndOfBlock, "pico8.end", "The end of the code: a zero byte after a pxa raw run, or two zero bytes in the old format.", &[END_RECORD]),
];

/// Every kind a codec's nodes can be. Empty for a codec whose nodes have no
/// kind yet.
pub fn kinds(codec: Codec) -> &'static [KindEntry] {
    Family::of(codec).map_or(&[], Family::kinds)
}

/// The kind of one node a trace laid down: which family's table, and which
/// row of it. Two bytes, copied rather than allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceKind {
    family: Family,
    row: u8,
}

impl TraceKind {
    /// The kind of a node in `role`, or `None` when the family's table has no
    /// row for it, which a test over the samples catches.
    pub fn of(family: Family, role: Role) -> Option<TraceKind> {
        let row = family.kinds().iter().position(|e| e.role == role)?;
        Some(TraceKind { family, row: row as u8 })
    }

    /// The name, such as `deflate.literal`.
    pub fn as_str(self) -> &'static str {
        self.entry().kind
    }

    pub fn family(self) -> Family {
        self.family
    }

    pub fn role(self) -> Role {
        self.entry().role
    }

    /// The row of the family's table this kind is.
    pub fn entry(self) -> &'static KindEntry {
        &self.family.kinds()[self.row as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row of a table is a different role and a different name, every
    /// name starts with the family's prefix, and a kind names itself back.
    #[test]
    fn every_table_is_one_row_per_role_and_name() {
        for &family in Family::ALL {
            let rows = family.kinds();
            assert!(rows.len() < 256);
            let prefix = family.as_str();
            assert_eq!(Family::named(prefix), Some(family));
            for (i, e) in rows.iter().enumerate() {
                assert!(rows[..i].iter().all(|o| o.role != e.role), "{family:?} lists {:?} twice", e.role);
                assert!(rows[..i].iter().all(|o| o.kind != e.kind), "{family:?} lists {} twice", e.kind);
                assert_eq!(e.kind.split('.').next(), Some(prefix), "{} in {family:?}", e.kind);
                let name = &e.kind[prefix.len() + 1..];
                assert!(name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'), "{}", e.kind);
                let kind = TraceKind::of(family, e.role).unwrap();
                assert_eq!(kind.as_str(), e.kind);
                assert_eq!(kind.role(), e.role);
            }
        }
        assert_eq!(std::mem::size_of::<Option<TraceKind>>(), 2);
    }

    /// Every row says what it is and how it reads; a row over other nodes
    /// reads as rows and names children from its own table, and nothing else
    /// has children; a record's first field is its kind, from the enum that
    /// names it.
    #[test]
    fn every_kind_has_a_description_and_a_shape() {
        for &family in Family::ALL {
            let rows = family.kinds();
            assert_eq!(rows[0].role, Role::Blocks, "{family:?} does not start with its blocks row");
            for e in rows {
                assert!(!e.about.is_empty() && e.about.ends_with('.'), "{} has no one-line description", e.kind);
                assert!(!e.about.contains('\u{2014}'), "{} uses an em dash", e.kind);
                assert!(!e.forms.is_empty(), "{} has no forms", e.kind);
                let over_rows = matches!(e.role, Role::Blocks | Role::Block | Role::Codes | Role::Unit);
                for f in e.forms {
                    assert_eq!(matches!(f.reads, Reads::Traced(_)), over_rows, "{} reads as {:?}", e.kind, f.reads);
                    if let Reads::Struct { kind, fields } = f.reads {
                        assert_eq!(fields.first(), Some(&"kind"), "{}", e.kind);
                        assert!(SYMBOL_KIND.cases.iter().any(|c| c.0 == kind), "{}", e.kind);
                    }
                }
                assert_eq!(!e.children.is_empty(), over_rows, "{} children {:?}", e.kind, e.children);
                for (i, c) in e.children.iter().enumerate() {
                    assert!(family.entry(*c).is_some(), "{} names a child {c:?} its table has no row for", e.kind);
                    assert!(!e.children[..i].contains(c), "{} names {c:?} twice", e.kind);
                }
                assert_eq!(e.child_kinds(family).count(), e.children.len());
            }
        }
    }

    #[test]
    fn deflate_repeat_codes_are_told_apart() {
        let repeat = |code| StepKind::Table(TableField::Repeat { code, count: 3, len: 0, dist: false });
        assert_eq!(Family::Deflate.step_role(repeat(16), false), Role::RepeatLength);
        assert_eq!(Family::Deflate.step_role(repeat(17), false), Role::ZeroLengths);
        assert_eq!(Family::Deflate.step_role(repeat(18), false), Role::ZeroLengths);
    }

    #[test]
    fn opaque_after_the_last_unit_is_trailing() {
        assert_eq!(Family::JpegScan.step_role(StepKind::Opaque, true), Role::Trailing);
        assert_eq!(Family::JpegScan.step_role(StepKind::Opaque, false), Role::Unnamed);
    }
}
