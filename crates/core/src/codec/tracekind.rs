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
}

impl Family {
    pub const ALL: &[Family] = &[Family::Deflate, Family::JpegScan];

    /// The family of a codec, or `None` for a codec whose nodes have no kind
    /// yet.
    pub fn of(codec: Codec) -> Option<Family> {
        match codec {
            Codec::Deflate | Codec::Zlib | Codec::Gzip => Some(Family::Deflate),
            Codec::JpegBaseline => Some(Family::JpegScan),
            _ => None,
        }
    }

    /// Every kind this family's nodes can be, in the order a reader meets
    /// them.
    pub fn kinds(self) -> &'static [KindEntry] {
        match self {
            Family::Deflate => DEFLATE,
            Family::JpegScan => JPEG_SCAN,
        }
    }

    /// The role a step plays in this family. Only the repeat codes of a
    /// Huffman table need the family: deflate's code 16 repeats the length
    /// before it, and 17 and 18 write zeros.
    pub fn step_role(self, kind: StepKind, outside_units: bool) -> Role {
        match kind {
            StepKind::Header(field, _) => Role::Field(field),
            StepKind::Table(TableField::CodeLen { .. }) => Role::CodeLengthCodeLength,
            StepKind::Table(TableField::LitLen { .. }) => Role::CodeLength,
            StepKind::Table(TableField::Dist { .. }) => Role::DistanceCodeLength,
            StepKind::Table(TableField::Repeat { code, .. }) => match (self, code) {
                (Family::Deflate, 16) => Role::RepeatLength,
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
