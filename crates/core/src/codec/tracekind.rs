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

use super::{Codec, StepField, StepKind, TableField};

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
            StepKind::Table(TableField::Repeat { code, .. }) => match (self, code) {
                (Family::Deflate, 16) | (Family::Rar5, 16 | 17) => Role::RepeatLength,
                _ => Role::ZeroLengths,
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

/// One row of a family's table: a role, and the name a node in that role is
/// given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindEntry {
    pub role: Role,
    pub kind: &'static str,
}

const fn k(role: Role, kind: &'static str) -> KindEntry {
    KindEntry { role, kind }
}

/// RFC 1951, read by `codec::inflate` for raw deflate, zlib and gzip.
const DEFLATE: &[KindEntry] = &[
    k(Role::Blocks, "deflate.blocks"),
    k(Role::Block, "deflate.block"),
    k(Role::Field(StepField::Bfinal), "deflate.bfinal"),
    k(Role::Field(StepField::Btype), "deflate.btype"),
    k(Role::Field(StepField::Hlit), "deflate.hlit"),
    k(Role::Field(StepField::Hdist), "deflate.hdist"),
    k(Role::Field(StepField::Hclen), "deflate.hclen"),
    k(Role::CodeLengthCodeLength, "deflate.code_length_code_length"),
    k(Role::CodeLength, "deflate.code_length"),
    k(Role::DistanceCodeLength, "deflate.distance_code_length"),
    k(Role::RepeatLength, "deflate.code_length_repeat"),
    k(Role::ZeroLengths, "deflate.code_length_zeros"),
    // A stored block: the bits up to the byte boundary, LEN and NLEN, and
    // then its bytes.
    k(Role::Field(StepField::Padding), "deflate.padding"),
    k(Role::Field(StepField::StoredLen), "deflate.stored_len"),
    k(Role::Field(StepField::StoredNlen), "deflate.stored_nlen"),
    k(Role::Codes, "deflate.codes"),
    k(Role::Stored, "deflate.stored_data"),
    k(Role::Literal, "deflate.literal"),
    k(Role::Match, "deflate.match"),
    k(Role::EndOfBlock, "deflate.end_of_block"),
    // A block's codes once the trace stopped naming them. See `MAX_STEPS`.
    k(Role::Unnamed, "deflate.unnamed_codes"),
];

/// A sequential Huffman-coded JPEG scan, read by `codec::jpeg`.
const JPEG_SCAN: &[KindEntry] = &[
    k(Role::Blocks, "jpegscan.mcus"),
    k(Role::Block, "jpegscan.mcu"),
    k(Role::Unit, "jpegscan.unit"),
    k(Role::Dc, "jpegscan.dc"),
    k(Role::Ac, "jpegscan.ac"),
    k(Role::Zrl, "jpegscan.zrl"),
    k(Role::Eob, "jpegscan.eob"),
    // A unit's codes once the trace stopped naming them. See `MAX_STEPS`.
    k(Role::Unnamed, "jpegscan.unnamed_codes"),
    // The ones that pad an interval out to a byte, and the marker after it.
    k(Role::Field(StepField::Padding), "jpegscan.padding"),
    k(Role::Field(StepField::Restart), "jpegscan.restart"),
    // Bytes after the last MCU that no code read.
    k(Role::Trailing, "jpegscan.trailing"),
];

/// LZ4 blocks, alone or in a frame, read by `codec::lz4`. A frame's block
/// starts with its size word; its checksums are outside every block.
const LZ4: &[KindEntry] = &[
    k(Role::Blocks, "lz4.blocks"),
    k(Role::Block, "lz4.block"),
    k(Role::Field(StepField::BlockHeader), "lz4.block_size"),
    k(Role::Field(StepField::Token), "lz4.token"),
    k(Role::Field(StepField::LengthExtra), "lz4.length_extra"),
    k(Role::Codes, "lz4.codes"),
    k(Role::Stored, "lz4.literals"),
    k(Role::Field(StepField::Offset), "lz4.offset"),
    k(Role::Match, "lz4.match"),
    k(Role::Unnamed, "lz4.unnamed_codes"),
];

/// One raw Snappy block, read by `codec::snappy`. The length in front of it
/// is outside the block.
const SNAPPY: &[KindEntry] = &[
    k(Role::Blocks, "snappy.blocks"),
    k(Role::Block, "snappy.block"),
    k(Role::Field(StepField::Token), "snappy.tag"),
    k(Role::Field(StepField::LengthExtra), "snappy.length_extra"),
    k(Role::Codes, "snappy.codes"),
    k(Role::Stored, "snappy.literals"),
    k(Role::Field(StepField::Offset), "snappy.offset"),
    k(Role::Match, "snappy.match"),
    k(Role::Unnamed, "snappy.unnamed_codes"),
];

/// One FastLZ block, read by `codec::fastlz`.
const FASTLZ: &[KindEntry] = &[
    k(Role::Blocks, "fastlz.blocks"),
    k(Role::Block, "fastlz.block"),
    k(Role::Field(StepField::Token), "fastlz.token"),
    k(Role::Field(StepField::LengthExtra), "fastlz.length_extra"),
    k(Role::Field(StepField::Offset), "fastlz.offset"),
    k(Role::Codes, "fastlz.codes"),
    k(Role::Stored, "fastlz.literals"),
    k(Role::Match, "fastlz.match"),
    k(Role::Field(StepField::Padding), "fastlz.padding"),
    k(Role::Unnamed, "fastlz.unnamed_codes"),
];

/// A `.Z` file's LZW codes, read by `codec::compress`. A block runs to a
/// clear code, and the zero bits that pad a group of codes when the width
/// changes are inside it.
const COMPRESS: &[KindEntry] = &[
    k(Role::Blocks, "compress.blocks"),
    k(Role::Block, "compress.block"),
    k(Role::Codes, "compress.codes"),
    k(Role::Literal, "compress.literal"),
    k(Role::Match, "compress.match"),
    k(Role::EndOfBlock, "compress.clear"),
    k(Role::Field(StepField::Padding), "compress.padding"),
    k(Role::Unnamed, "compress.unnamed_codes"),
];

/// LHA's `-lh5-` family, read by `codec::lha`: per block, a symbol count,
/// three Huffman tables and the codes.
const LZHUF: &[KindEntry] = &[
    k(Role::Blocks, "lzhuf.blocks"),
    k(Role::Block, "lzhuf.block"),
    k(Role::Field(StepField::BlockHeader), "lzhuf.symbol_count"),
    k(Role::CodeLengthCodeLength, "lzhuf.code_length_code_length"),
    k(Role::CodeLength, "lzhuf.code_length"),
    k(Role::DistanceCodeLength, "lzhuf.offset_code_length"),
    k(Role::ZeroLengths, "lzhuf.code_length_zeros"),
    k(Role::Codes, "lzhuf.codes"),
    k(Role::Literal, "lzhuf.literal"),
    k(Role::Match, "lzhuf.match"),
    k(Role::Unnamed, "lzhuf.unnamed_codes"),
];

/// RAR 5's compressed blocks, read by `codec::rar5`.
const RAR5: &[KindEntry] = &[
    k(Role::Blocks, "rar5.blocks"),
    k(Role::Block, "rar5.block"),
    k(Role::Field(StepField::BlockHeader), "rar5.block_header"),
    k(Role::CodeLengthCodeLength, "rar5.code_length_code_length"),
    k(Role::CodeLength, "rar5.code_length"),
    k(Role::DistanceCodeLength, "rar5.distance_code_length"),
    k(Role::RepeatLength, "rar5.code_length_repeat"),
    k(Role::ZeroLengths, "rar5.code_length_zeros"),
    k(Role::Codes, "rar5.codes"),
    k(Role::Literal, "rar5.literal"),
    k(Role::Match, "rar5.match"),
    k(Role::Field(StepField::FilterDef), "rar5.filter"),
    k(Role::Field(StepField::Padding), "rar5.padding"),
    k(Role::Unnamed, "rar5.unnamed_codes"),
];

/// LZMA, read by `codec::lzma`: one block for an LZMA1 stream, one per chunk
/// for LZMA2, raw, in lzip or in xz. A container's own headers and checks are
/// outside every block.
const LZMA: &[KindEntry] = &[
    k(Role::Blocks, "lzma.blocks"),
    k(Role::Block, "lzma.block"),
    k(Role::Field(StepField::BlockHeader), "lzma.chunk_header"),
    k(Role::Field(StepField::LzmaProps), "lzma.properties"),
    k(Role::Field(StepField::RangeInit), "lzma.range_init"),
    k(Role::Codes, "lzma.codes"),
    k(Role::Literal, "lzma.literal"),
    k(Role::Match, "lzma.match"),
    k(Role::EndOfBlock, "lzma.end_marker"),
    k(Role::Stored, "lzma.stored_data"),
    k(Role::Unnamed, "lzma.unnamed_codes"),
];

/// PNG's scanlines, read by `codec::scanlines` and `codec::pixels`. A row's
/// pass and row number are steps of no width.
const PNG_SCANLINES: &[KindEntry] = &[
    k(Role::Blocks, "png.scanlines"),
    k(Role::Block, "png.scanline"),
    k(Role::Field(StepField::Pass), "png.pass"),
    k(Role::Field(StepField::Row), "png.row"),
    k(Role::Field(StepField::Filter), "png.filter"),
    k(Role::Filtered, "png.filtered_row"),
];

/// Bytes kept in the low bits of a picture's pixels, read by
/// `codec::pixels`.
const LOW_BITS: &[KindEntry] = &[
    k(Role::Blocks, "lowbits.blocks"),
    k(Role::Block, "lowbits.pixels"),
    k(Role::Codes, "lowbits.codes"),
    k(Role::Pixel, "lowbits.pixel"),
];

/// NASA CDF's Huffman and adaptive Huffman coding, read by `codec::cdfhuff`.
const CDF_HUFFMAN: &[KindEntry] = &[
    k(Role::Blocks, "cdfhuffman.blocks"),
    k(Role::Block, "cdfhuffman.block"),
    k(Role::Field(StepField::FrequencyTable), "cdfhuffman.frequency_table"),
    k(Role::Codes, "cdfhuffman.codes"),
    k(Role::Literal, "cdfhuffman.literal"),
    k(Role::EndOfBlock, "cdfhuffman.end_of_stream"),
    k(Role::Field(StepField::Padding), "cdfhuffman.padding"),
    k(Role::Unnamed, "cdfhuffman.unnamed_codes"),
];

/// PICO-8's two code compressions, read by `codec::pico8`.
const PICO8: &[KindEntry] = &[
    k(Role::Blocks, "pico8.blocks"),
    k(Role::Block, "pico8.block"),
    k(Role::Codes, "pico8.codes"),
    k(Role::Literal, "pico8.literal"),
    k(Role::Match, "pico8.match"),
    k(Role::EndOfBlock, "pico8.end"),
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
        self.family.kinds()[self.row as usize].kind
    }

    pub fn family(self) -> Family {
        self.family
    }

    pub fn role(self) -> Role {
        self.family.kinds()[self.row as usize].role
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
            let prefix = rows[0].kind.split('.').next().unwrap();
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
