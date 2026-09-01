//! Which bits of a variable-length number are framing and which are the value.
//!
//! A varint's bytes do not read as bytes. Seven bits of each one are part of
//! the number and the eighth says whether there is another; an EBML integer
//! spends its leading zeros saying how wide it is. A reader shown `12` and told
//! it means 18 has to take that on trust unless the split is drawn.
//!
//! The split is decode knowledge, so it lives beside the decoders rather than
//! in whatever is drawing them. This module answers one question — for a type
//! and the bytes a field of it covers, which runs of bits are what — and says
//! nothing about how to draw the answer or what words to put beside it. The
//! rule a reader needs told is named here, not written here: the string is the
//! view's, keyed by [`BitRoles::rule`].

use crate::template::Ty;

/// What the decoder does with one run of bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitRole {
    /// Framing: the number carries on into the next byte.
    More,
    /// Framing: the number ends here.
    Stop,
    /// Framing: how many bytes wide the number is. EBML spends leading zeros
    /// on this rather than a bit per byte.
    Width,
    /// Part of the number.
    Payload,
}

impl BitRole {
    pub fn as_str(self) -> &'static str {
        match self {
            BitRole::More => "more",
            BitRole::Stop => "stop",
            BitRole::Width => "width",
            BitRole::Payload => "payload",
        }
    }
}

/// One run of bits, written out as `0` and `1` in the order they are stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitGroup {
    pub bits: String,
    pub role: BitRole,
}

/// How one field's bytes divide up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitRoles {
    /// Which rule the reader has to know to follow the split, as a key the
    /// view has copy for. Empty is not a value: a type with no rule worth
    /// saying produces no `BitRoles` at all.
    pub rule: &'static str,
    pub groups: Vec<BitGroup>,
}

/// The number ends at the first byte whose top bit is clear. LEB128, SQLite's
/// varint under nine bytes, and MIDI's VLQ all work this way.
pub const RULE_HIGH_BIT: &str = "high_bit";
/// SQLite's ninth byte, which is all value and has no continuation bit because
/// there cannot be a tenth.
pub const RULE_SQLITE_NINTH: &str = "sqlite_ninth";
/// EBML data size: leading zeros count the extra bytes, and the marker bit is
/// framing that the value does not include.
pub const RULE_EBML_SIZE: &str = "ebml_size";
/// EBML element ID: the same framing, kept as part of the number.
pub const RULE_EBML_ID: &str = "ebml_id";

/// Longest field this is worth answering for. Every scheme here is over well
/// before it, and a run of bytes longer than one is not a varint at all.
const MAX_BYTES: usize = 9;

fn bits_of(b: u8, from: u32, count: u32) -> String {
    (from..from + count).map(|i| if b >> (7 - i) & 1 == 1 { '1' } else { '0' }).collect()
}

/// How the bytes of one field of this type divide into framing and value.
/// `None` for every type that reads as whole bytes, which is most of them, and
/// for bytes that cannot be a number of this type at all.
pub fn bit_roles(ty: &Ty, bytes: &[u8]) -> Option<BitRoles> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return None;
    }
    match ty.base() {
        Ty::Leb128 { .. } | Ty::Vlq => Some(continuation(bytes, false)),
        Ty::SqliteVarint => Some(continuation(bytes, true)),
        Ty::EbmlVint { strip_marker } => ebml(bytes, *strip_marker),
        _ => None,
    }
}

/// A bit per byte saying whether another follows. `ninth` is SQLite's rule that
/// a ninth byte spends all eight of its bits on the value, since nothing can
/// follow it.
fn continuation(bytes: &[u8], ninth: bool) -> BitRoles {
    let mut groups = Vec::new();
    let mut rule = RULE_HIGH_BIT;
    for (i, &b) in bytes.iter().enumerate() {
        if ninth && i == 8 {
            rule = RULE_SQLITE_NINTH;
            groups.push(BitGroup { bits: bits_of(b, 0, 8), role: BitRole::Payload });
            break;
        }
        let role = if b & 0x80 != 0 { BitRole::More } else { BitRole::Stop };
        groups.push(BitGroup { bits: bits_of(b, 0, 1), role });
        groups.push(BitGroup { bits: bits_of(b, 1, 7), role: BitRole::Payload });
    }
    BitRoles { rule, groups }
}

/// EBML: the first set bit closes a run of zeros, and how far along it is says
/// how many bytes the number runs to.
fn ebml(bytes: &[u8], strip_marker: bool) -> Option<BitRoles> {
    let first = *bytes.first()?;
    if first == 0 {
        return None;
    }
    let marker = first.leading_zeros() + 1;
    // A field whose bytes and whose marker disagree is not this number, and
    // splitting it would be an invention rather than a reading.
    if marker as usize != bytes.len() {
        return None;
    }
    let mut groups = vec![BitGroup { bits: bits_of(first, 0, marker), role: BitRole::Width }];
    if marker < 8 {
        groups.push(BitGroup { bits: bits_of(first, marker, 8 - marker), role: BitRole::Payload });
    }
    for &b in &bytes[1..] {
        groups.push(BitGroup { bits: bits_of(b, 0, 8), role: BitRole::Payload });
    }
    let rule = if strip_marker { RULE_EBML_SIZE } else { RULE_EBML_ID };
    Some(BitRoles { rule, groups })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_byte_sqlite_varint_splits_at_the_high_bit() {
        let r = bit_roles(&Ty::SqliteVarint, &[0x12]).expect("a varint splits");
        assert_eq!(r.rule, RULE_HIGH_BIT);
        assert_eq!(r.groups, vec![
            BitGroup { bits: "0".into(), role: BitRole::Stop },
            BitGroup { bits: "0010010".into(), role: BitRole::Payload },
        ]);
    }

    #[test]
    fn a_continued_byte_is_marked_as_carrying_on() {
        let r = bit_roles(&Ty::Leb128 { signed: false }, &[0x81, 0x02]).expect("a leb splits");
        let roles: Vec<BitRole> = r.groups.iter().map(|g| g.role).collect();
        assert_eq!(roles, vec![BitRole::More, BitRole::Payload, BitRole::Stop, BitRole::Payload]);
    }

    #[test]
    fn the_sqlite_ninth_byte_is_all_value() {
        let bytes = [0x81u8; 9];
        let r = bit_roles(&Ty::SqliteVarint, &bytes).expect("nine bytes split");
        assert_eq!(r.rule, RULE_SQLITE_NINTH);
        assert_eq!(r.groups.last().expect("a last group"), &BitGroup { bits: "10000001".into(), role: BitRole::Payload });
    }

    #[test]
    fn an_ebml_marker_is_as_wide_as_the_number() {
        let r = bit_roles(&Ty::EbmlVint { strip_marker: true }, &[0x40, 0x2f]).expect("a vint splits");
        assert_eq!(r.rule, RULE_EBML_SIZE);
        assert_eq!(r.groups[0], BitGroup { bits: "01".into(), role: BitRole::Width });
        assert_eq!(r.groups[1], BitGroup { bits: "000000".into(), role: BitRole::Payload });
        assert_eq!(r.groups[2], BitGroup { bits: "00101111".into(), role: BitRole::Payload });
    }

    #[test]
    fn bytes_that_do_not_match_the_marker_are_not_split() {
        assert!(bit_roles(&Ty::EbmlVint { strip_marker: true }, &[0x40]).is_none());
        assert!(bit_roles(&Ty::UInt { bits: 8, endian: crate::template::Endian::Big }, &[0x12]).is_none());
    }
}
