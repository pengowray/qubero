//! BGZF, and the genomics formats written in it.
//!
//! **BGZF** is gzip cut into blocks. Each block is a whole gzip member of at
//! most 64 KB, so an ordinary gunzip reads the file and gets the whole of what
//! went in, and each member carries one extra subfield, `BC`, holding the size
//! of the block less one. That number is the point of the format: a reader can
//! step from block to block without inflating anything, and an index can name
//! a place in the file as the block it starts in and a byte inside what that
//! block unpacks to. The file ends with a 28-byte block that unpacks to
//! nothing, so a file cut short can be told from one that finished.
//!
//! Every block is read here as the gzip member it is, through
//! [`super::gzip::member`], with the extra field's subfields named and the
//! member sized by its `BC` number rather than by where the file ends.
//!
//! **BAM** is a stream of alignment records written through BGZF: a header
//! (the SAM header text and the list of reference sequences), then records to
//! the end, each a length and that many bytes. The stream does not care where
//! the blocks were cut. A record can start near the end of one block and
//! finish in the next, and a long header can fill several.
//!
//! That is the one thing the template cannot follow. Each block opens as a
//! space of its own, and no space is the blocks joined together (see
//! `eval/space.rs`: one `Decoded` node, one buffer), so a field has nowhere to
//! stand that covers bytes of two blocks. What the template does read is what
//! lies inside the first block: the header, the references, and every record
//! that fits whole, with whatever is left at the end of the block read as the
//! start of something that continues. That is the whole header in nearly
//! every file, since htslib ends a block where the header ends. The records in
//! later blocks are read by [`super::bam_records`], which inflates from the
//! front of the file and walks the record lengths to find the records that
//! start in a given block.
//!
//! htslib also ends a block before a record that would not fit in it, so a
//! BAM it wrote has no record crossing blocks unless the record is longer
//! than a block. Other writers cut wherever 64 KB falls, and htslib's own
//! `bgzf_boundaries` test files do it on purpose.
//!
//! An uncompressed BAM stream, which is what the blocks unpack to when joined,
//! opens as `bam` and reads every record as fields, because there the stream
//! is one run.
//!
//! **BAI** is the index beside a BAM: for each reference, the bins of the
//! binning scheme with the runs of records in each, and a linear index of the
//! first record in each 16 kbp window. Every place it names is a virtual
//! offset, the block's position in the file shifted up sixteen bits with a
//! byte of that block's unpacked data in the low sixteen, so each is read here
//! as both halves. A BAI is not compressed. **CSI** is the same index with the
//! bin sizes made settings, and is written through BGZF, so it is read inside
//! a block.
//!
//! The SAM/BAM specification (samtools/hts-specs, `SAMv1.tex` and
//! `CSIv1.tex`) is what this follows.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Until, Ty as T};

/// The subfield identifier BGZF writes into every member's extra field.
const BC: &[u8; 2] = b"BC";

/// Where the `BC` number is in a block, in bytes, when `BC` is the first
/// subfield: twelve bytes of gzip header, two of identifier and two of
/// subfield length. Every writer in use puts it first, and the block's size
/// has to be known before its extra field has been read, so this is where it
/// is looked for.
const BSIZE_AT: i128 = 16;

pub fn bgzf() -> Template {
    Template::new("bgzf", T::structure("BGZF", vec![("blocks", T::repeat(block(), Until::End))]))
}

/// One block: a gzip member, as long as its `BC` number says.
///
/// The number is read ahead, at [`BSIZE_AT`], because a `Sized` needs its size
/// before anything inside it is placed; the same two bytes are read again as a
/// field of the extra field, where they have a name. When the identifier and
/// the subfield length there are not `BC` and 2, the block runs to the end of
/// the file instead, which is how a plain gzip member reads: a block that does
/// not say how long it is has not been measured, and reading it as the rest of
/// the file shows that rather than guessing.
fn block() -> T {
    let peek16 = |at: i128, endian| E::peek_at(E::lit(at * 8), 16, endian);
    let is_bc = E::lit(BSIZE_AT + 1)
        .less_than(E::Remaining)
        .both(peek16(BSIZE_AT - 4, Big).equal_to(E::lit(u16::from_be_bytes(*BC) as i128)))
        .both(peek16(BSIZE_AT - 2, Little).equal_to(E::lit(2)));
    let size = E::cond(is_bc, peek16(BSIZE_AT, Little).add(E::lit(1)), E::Remaining);
    T::sized(size, super::gzip::member("BgzfBlock", extra(), payload())).counted_as("block")
}

/// The extra field, as the subfields RFC 1952 says it holds: two letters, a
/// length, and that many bytes. BGZF's is `BC` and holds a sixteen-bit number;
/// anything else a writer put beside it is left as its bytes.
fn extra() -> T {
    let subfield = T::structure_named(
        "ExtraSubfield",
        "id",
        "data",
        vec![
            ("id", T::text(StrLen::Fixed(E::lit(2)), Encoding::Latin1)),
            ("length", T::u16(Little)),
            (
                "data",
                T::sized(
                    E::field("length"),
                    T::matches(
                        E::field("id"),
                        // The size of the whole block, less one, so that a
                        // block of 65,536 bytes still fits in sixteen bits.
                        vec![("BC", T::structure("BgzfSize", vec![("bsize", T::u16(Little))]))],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
        ],
    );
    T::structure(
        "Extra",
        vec![("length", T::u16(Little)), ("subfields", T::sized(E::field("length"), T::repeat(subfield, Until::End)))],
    )
}

/// What a block unpacks to.
///
/// The first block of a BAM opens as the start of its stream, which is where
/// the header is. Only the first: a later block that happened to unpack to
/// the same four bytes is in the middle of somebody's read. Anything else is
/// read as text, which reads as bytes where the bytes are not text, so a
/// `.vcf.gz` shows its lines and the later blocks of a BAM show their bytes.
/// Nothing a template can ask tells a later block whether the first one was a
/// BAM without opening it too, and every block opening the first to find out
/// would be the whole file's first block inflated once per block.
///
/// The peek is guarded because the last block of every BGZF file unpacks to
/// nothing, and four bytes cannot be looked at in none.
fn payload() -> T {
    let first_with_room = E::idx().equal_to(E::lit(0)).both(E::lit(3).less_than(E::Remaining));
    let magic = |m: &[u8]| u32::from_be_bytes(m.try_into().expect("four bytes")) as i128;
    T::switch(
        first_with_room,
        vec![(
            1,
            T::switch(E::peek(32, Big), vec![(magic(BAM_MAGIC), bam_stream())], super::decoded_text()),
        )],
        super::decoded_text(),
    )
}

/// What a BAM stream opens with.
pub const BAM_MAGIC: &[u8] = b"BAM\x01";

/// An uncompressed BAM stream: what the blocks of a BAM unpack to, joined.
pub fn bam() -> Template {
    Template::new("bam", bam_stream())
}

/// A BAM stream from its first byte, read as far as its window reaches.
///
/// Every field after the text asks whether there is room for it, because the
/// window is often one block and a header may not fit in one. `mpileup.1.bam`
/// in samtools' tests writes 119 KB of header text and its first block holds
/// 64 KB of it, so there the text is cut short and nothing follows it. The
/// references are read one at a time for the same reason, and stop at the
/// count or at the end of the window, whichever comes first.
fn bam_stream() -> T {
    T::structure(
        "Bam",
        vec![
            ("magic", T::magic(BAM_MAGIC)),
            // How long the header text is, which may include NUL padding after
            // it: some writers leave room to rewrite the header in place.
            ("l_text", T::u32(Little)),
            ("text", T::text(StrLen::Fixed(min(E::field("l_text"), E::Remaining)), Encoding::Utf8)),
            ("n_ref", T::if_room(T::u32(Little))),
            (
                "references",
                T::when(
                    E::field("n_ref").greater_than(E::lit(0)),
                    T::repeat(
                        whole_or_cut(reference(), E::peek(32, Little).add(E::lit(8))),
                        Until::Cond(E::idx().add(E::lit(1)).greater_or_equal(E::field("n_ref"))),
                    ),
                ),
            ),
            ("records", T::repeat(whole_or_cut(record(), E::peek(32, Little).add(E::lit(4))), Until::End)),
        ],
    )
}

/// The smaller of two numbers.
fn min(a: E, b: E) -> E {
    E::Min(Box::new(a), Box::new(b))
}

/// `whole` where the window still holds all `size` bytes of it, which is
/// worked out from a length at its front, and otherwise the bytes that are
/// left, read as the start of something the next block finishes.
///
/// Four bytes are needed to read the length at all, so a window with fewer
/// left is cut short without looking.
fn whole_or_cut(whole: T, size: E) -> T {
    let fits = E::lit(3).less_than(E::Remaining).both(size.less_or_equal(E::Remaining));
    T::switch(fits, vec![(1, whole)], continued())
}

/// The last bytes of a block, where what they start carries on into the next
/// block and is not read here.
fn continued() -> T {
    T::structure("Continued", vec![("bytes", T::bytes(E::Remaining))])
}

/// One reference sequence: its name, with the NUL counted in the length, and
/// how many bases it has.
fn reference() -> T {
    T::structure_named(
        "Reference",
        "name",
        "",
        vec![
            ("l_name", T::u32(Little)),
            ("name", T::text(StrLen::Padded { size: E::field("l_name"), pad: 0 }, Encoding::Utf8)),
            ("l_ref", T::u32(Little)),
        ],
    )
}

/// The bits of `flag`, as samtools names them.
const FLAG_BITS: &[(u32, &str)] = &[
    (0, "paired"),
    (1, "proper pair"),
    (2, "unmapped"),
    (3, "mate unmapped"),
    (4, "reverse strand"),
    (5, "mate reverse strand"),
    (6, "first in pair"),
    (7, "second in pair"),
    (8, "secondary"),
    (9, "failed QC"),
    (10, "duplicate"),
    (11, "supplementary"),
];

/// The operations a CIGAR is written in, by the number BAM stores for each.
pub const CIGAR_OPS: &[u8; 9] = b"MIDNSHP=X";

/// The bases a four-bit code stands for, by the code.
pub const BASES: &[u8; 16] = b"=ACMGRSVTWYHKDBN";

/// One alignment record: `block_size`, then the fixed fields, the read name,
/// the CIGAR, the sequence, the qualities and the tags, in that order and all
/// of them counted in `block_size`.
///
/// The tags are what is left of the record after everything with a length of
/// its own, and that sum is written out as the tags' size rather than read as
/// the rest of the record, so a record whose lengths disagree with its
/// `block_size` shows where rather than quietly reading into the next one.
fn record() -> T {
    let op = T::structure(
        "CigarOp",
        vec![
            // `op_len << 4 | op`, little-endian. The four low bits are the
            // operation and sit at the bottom of the first byte, so they cannot
            // be a field of their own without crossing a byte boundary the
            // wrong way, and are worked out from the number instead.
            ("value", T::u32(Little)),
            ("op_len", T::computed(E::field("value").shr(E::lit(4)))),
            ("op", T::enumeration("CigarOpCode", T::computed(E::field("value").and(E::lit(15))), &cigar_cases())),
        ],
    );
    let base = T::enumeration("Base", T::UInt { bits: 4, endian: Big }, &base_cases());
    let before_tags = E::lit(32)
        .add(E::field("l_read_name"))
        .add(E::field("n_cigar_op").mul(E::lit(4)))
        .add(E::field("l_seq").add(E::lit(1)).div(E::lit(2)))
        .add(E::field("l_seq"));
    T::structure_named(
        "AlignmentRecord",
        "read_name",
        "",
        vec![
            ("block_size", T::u32(Little)),
            ("refID", T::Int { bits: 32, endian: Little }),
            // Counted from zero, so one less than the POS a SAM line shows.
            ("pos", T::Int { bits: 32, endian: Little }),
            ("l_read_name", T::u8()),
            ("mapq", T::u8()),
            ("bin", T::u16(Little)),
            ("n_cigar_op", T::u16(Little)),
            ("flag", T::flags("Flag", T::u16(Little), FLAG_BITS)),
            ("l_seq", T::u32(Little)),
            ("next_refID", T::Int { bits: 32, endian: Little }),
            ("next_pos", T::Int { bits: 32, endian: Little }),
            ("tlen", T::Int { bits: 32, endian: Little }),
            ("read_name", T::text(StrLen::Padded { size: E::field("l_read_name"), pad: 0 }, Encoding::Utf8)),
            ("cigar", T::array(op, E::field("n_cigar_op"))),
            // Two bases a byte, the first in the high four bits.
            ("seq", T::array(base, E::field("l_seq"))),
            // The low four bits of the last byte when the read is an odd
            // length, which belong to no base.
            ("seq_pad", T::when(E::field("l_seq").modulo(E::lit(2)), T::UInt { bits: 4, endian: Big })),
            // Phred scores as they are, not the printable characters a SAM
            // line adds 33 to. All 0xff when the record has none.
            ("qual", T::array(T::u8(), E::field("l_seq"))),
            ("tags", T::sized(E::field("block_size").sub(before_tags), T::repeat(tag(), Until::End))),
        ],
    )
    .counted_as("record")
}

fn cigar_cases() -> Vec<(i128, &'static str)> {
    const NAMES: [&str; 9] = ["M", "I", "D", "N", "S", "H", "P", "=", "X"];
    NAMES.iter().enumerate().map(|(i, n)| (i as i128, *n)).collect()
}

fn base_cases() -> Vec<(i128, &'static str)> {
    const NAMES: [&str; 16] = ["=", "A", "C", "M", "G", "R", "S", "V", "T", "W", "Y", "H", "K", "D", "B", "N"];
    NAMES.iter().enumerate().map(|(i, n)| (i as i128, *n)).collect()
}

/// One optional field: two letters, a type letter, and a value as wide as the
/// type says. A type letter the specification does not define leaves the rest
/// of the record as bytes, since nothing then says how long the value is.
fn tag() -> T {
    let scalar = || {
        vec![
            ("c", T::Int { bits: 8, endian: Big }),
            ("C", T::u8()),
            ("s", T::Int { bits: 16, endian: Little }),
            ("S", T::u16(Little)),
            ("i", T::Int { bits: 32, endian: Little }),
            ("I", T::u32(Little)),
            ("f", T::F32(Little)),
        ]
    };
    let array = T::structure(
        "TagArray",
        vec![
            ("subtype", T::text(StrLen::Fixed(E::lit(1)), Encoding::Latin1)),
            ("count", T::u32(Little)),
            ("values", T::array(T::matches(E::field("subtype"), scalar(), T::bytes(E::Remaining)), E::field("count"))),
        ],
    );
    let mut cases = scalar();
    cases.push(("A", T::text(StrLen::Fixed(E::lit(1)), Encoding::Latin1)));
    cases.push(("Z", T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Utf8)));
    // A byte array written as hex digits, which is how BAM kept `H` from SAM.
    cases.push(("H", T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Ascii)));
    cases.push(("B", array));
    T::structure_named(
        "Tag",
        "tag",
        "value",
        vec![
            ("tag", T::text(StrLen::Fixed(E::lit(2)), Encoding::Latin1)),
            ("val_type", T::text(StrLen::Fixed(E::lit(1)), Encoding::Latin1)),
            ("value", T::matches(E::field("val_type"), cases, T::bytes(E::Remaining))),
        ],
    )
}

/// Whether the first bytes of a file are a BGZF block: a gzip header with the
/// extra flag set and a `BC` subfield of two bytes somewhere in its extra
/// field. A plain gzip file has no such subfield, and this is what tells the
/// two apart; both open with the same three bytes.
pub(crate) fn is_bgzf(head: &[u8]) -> bool {
    if head.len() < 18 || head[..3] != [0x1f, 0x8b, 8] || head[3] & 4 == 0 {
        return false;
    }
    let xlen = u16::from_le_bytes([head[10], head[11]]) as usize;
    let Some(extra) = head.get(12..12 + xlen) else { return false };
    let mut at = 0;
    while at + 4 <= extra.len() {
        let len = u16::from_le_bytes([extra[at + 2], extra[at + 3]]) as usize;
        if extra[at..at + 2] == *BC && len == 2 {
            return true;
        }
        at += 4 + len;
    }
    false
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// The header of a BAM stream: the text and the references.
    pub(crate) fn bam_header(text: &str, refs: &[(&str, u32)]) -> Vec<u8> {
        let mut v = BAM_MAGIC.to_vec();
        v.extend_from_slice(&(text.len() as u32).to_le_bytes());
        v.extend_from_slice(text.as_bytes());
        v.extend_from_slice(&(refs.len() as u32).to_le_bytes());
        for (name, len) in refs {
            v.extend_from_slice(&(name.len() as u32 + 1).to_le_bytes());
            v.extend_from_slice(name.as_bytes());
            v.push(0);
            v.extend_from_slice(&len.to_le_bytes());
        }
        v
    }

    /// An alignment record as a test writes it: the fixed fields that vary,
    /// the CIGAR as (length, operation letter) pairs, the bases as letters,
    /// and the tags already written out as bytes.
    pub(crate) struct Rec<'a> {
        pub name: &'a str,
        pub ref_id: i32,
        pub pos: i32,
        pub mapq: u8,
        pub flag: u16,
        pub cigar: &'a [(u32, u8)],
        pub seq: &'a str,
        pub qual: &'a [u8],
        pub tags: &'a [u8],
    }

    pub(crate) fn record_bytes(r: &Rec) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&r.ref_id.to_le_bytes());
        body.extend_from_slice(&r.pos.to_le_bytes());
        body.push(r.name.len() as u8 + 1);
        body.push(r.mapq);
        body.extend_from_slice(&4680u16.to_le_bytes());
        body.extend_from_slice(&(r.cigar.len() as u16).to_le_bytes());
        body.extend_from_slice(&r.flag.to_le_bytes());
        body.extend_from_slice(&(r.seq.len() as u32).to_le_bytes());
        body.extend_from_slice(&(-1i32).to_le_bytes());
        body.extend_from_slice(&(-1i32).to_le_bytes());
        body.extend_from_slice(&0i32.to_le_bytes());
        body.extend_from_slice(r.name.as_bytes());
        body.push(0);
        for (len, op) in r.cigar {
            let code = CIGAR_OPS.iter().position(|c| c == op).expect("a CIGAR letter") as u32;
            body.extend_from_slice(&(len << 4 | code).to_le_bytes());
        }
        let codes: Vec<u8> =
            r.seq.bytes().map(|b| BASES.iter().position(|c| *c == b).expect("a base letter") as u8).collect();
        for pair in codes.chunks(2) {
            body.push(pair[0] << 4 | pair.get(1).copied().unwrap_or(0));
        }
        body.extend_from_slice(r.qual);
        body.extend_from_slice(r.tags);
        let mut v = (body.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(&body);
        v
    }

    /// One tag of every type the specification defines, in order.
    pub(crate) fn every_tag() -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(b"XAAq");
        t.extend_from_slice(b"XBc\xfe");
        t.extend_from_slice(b"XCC\xfe");
        t.extend_from_slice(b"XDs");
        t.extend_from_slice(&(-300i16).to_le_bytes());
        t.extend_from_slice(b"XES");
        t.extend_from_slice(&65000u16.to_le_bytes());
        t.extend_from_slice(b"XFi");
        t.extend_from_slice(&(-70000i32).to_le_bytes());
        t.extend_from_slice(b"XGI");
        t.extend_from_slice(&3_000_000_000u32.to_le_bytes());
        t.extend_from_slice(b"XHf");
        t.extend_from_slice(&1.5f32.to_le_bytes());
        t.extend_from_slice(b"XIZhello\0");
        t.extend_from_slice(b"XJH1AE3\0");
        t.extend_from_slice(b"XKBs");
        t.extend_from_slice(&3u32.to_le_bytes());
        for n in [-1i16, 0, 1000] {
            t.extend_from_slice(&n.to_le_bytes());
        }
        t.extend_from_slice(b"XLBf");
        t.extend_from_slice(&1u32.to_le_bytes());
        t.extend_from_slice(&0.25f32.to_le_bytes());
        t
    }

    /// An odd-length read with every tag type on it, and an even one with
    /// none, which between them reach every field a record has.
    pub(crate) fn two_records() -> (Vec<u8>, Vec<u8>) {
        let tags = every_tag();
        let odd = Rec {
            name: "read/1",
            ref_id: 0,
            pos: 99,
            mapq: 60,
            flag: 0x63,
            cigar: &[(3, b'S'), (4, b'M')],
            seq: "ACGTNAC",
            qual: &[30, 31, 32, 33, 2, 40, 41],
            tags: &tags,
        };
        let even = Rec {
            name: "r2",
            ref_id: 1,
            pos: 5,
            mapq: 255,
            flag: 0x10,
            cigar: &[(2, b'M'), (1, b'D'), (2, b'M')],
            seq: "GGTT",
            qual: &[0xff; 4],
            tags: &[],
        };
        (record_bytes(&odd), record_bytes(&even))
    }

    fn at_named(ev: &mut Evaluator, d: &Document<MemSource>, path: &[usize], names: &[&str]) -> Vec<usize> {
        let mut at = path.to_vec();
        for name in names {
            at = ev.child_named(d, &at, name).unwrap().unwrap_or_else(|| panic!("no {name} under {at:?}"));
        }
        at
    }

    #[test]
    fn an_uncompressed_bam_stream_reads_every_record_as_fields() {
        let (odd, even) = two_records();
        let mut stream = bam_header("@HD\tVN:1.6\n", &[("chr1", 1000), ("chr2", 2000)]);
        stream.extend_from_slice(&odd);
        stream.extend_from_slice(&even);
        let d = Document::new(MemSource(stream));
        let mut ev = Evaluator::new(bam());
        let refs = at_named(&mut ev, &d, &[], &["references"]);
        assert_eq!(ev.node(&d, &refs).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[refs.clone(), vec![1, 1]].concat()).unwrap().value, Value::Str("chr2".into()));
        let records = at_named(&mut ev, &d, &[], &["records"]);
        assert_eq!(ev.node(&d, &records).unwrap().child_count, 2);

        let first = [records.clone(), vec![0]].concat();
        assert_eq!(ev.node(&d, &first).unwrap().type_name, "AlignmentRecord");
        let name = at_named(&mut ev, &d, &first, &["read_name"]);
        assert_eq!(ev.node(&d, &name).unwrap().value, Value::Str("read/1".into()));
        let cigar = at_named(&mut ev, &d, &first, &["cigar"]);
        let op = at_named(&mut ev, &d, &[cigar.clone(), vec![0]].concat(), &["op"]);
        let len = at_named(&mut ev, &d, &[cigar, vec![0]].concat(), &["op_len"]);
        assert_eq!(ev.node(&d, &len).unwrap().value.as_int(), Some(3));
        assert!(format!("{:?}", ev.node(&d, &op).unwrap().value).contains('S'), "{:?}", ev.node(&d, &op).unwrap().value);
        let seq = at_named(&mut ev, &d, &first, &["seq"]);
        assert_eq!(ev.node(&d, &seq).unwrap().child_count, 7);
        let pad = at_named(&mut ev, &d, &first, &["seq_pad"]);
        assert_eq!(ev.node(&d, &pad).unwrap().size_bits, 4);
        let tags = at_named(&mut ev, &d, &first, &["tags"]);
        assert_eq!(ev.node(&d, &tags).unwrap().child_count, 12);
        let value = |ev: &mut Evaluator, i: usize| {
            let at = at_named(ev, &d, &[tags.clone(), vec![i]].concat(), &["value"]);
            ev.node(&d, &at).unwrap().value
        };
        assert_eq!(value(&mut ev, 0), Value::Str("q".into()));
        assert_eq!(value(&mut ev, 1).as_int(), Some(-2));
        assert_eq!(value(&mut ev, 2).as_int(), Some(254));
        assert_eq!(value(&mut ev, 3).as_int(), Some(-300));
        assert_eq!(value(&mut ev, 4).as_int(), Some(65000));
        assert_eq!(value(&mut ev, 5).as_int(), Some(-70000));
        assert_eq!(value(&mut ev, 6).as_int(), Some(3_000_000_000));
        assert_eq!(value(&mut ev, 7), Value::Float(1.5));
        assert_eq!(value(&mut ev, 8), Value::Str("hello".into()));
        assert_eq!(value(&mut ev, 9), Value::Str("1AE3".into()));
        let array = at_named(&mut ev, &d, &[tags.clone(), vec![10]].concat(), &["value", "values"]);
        assert_eq!(ev.node(&d, &array).unwrap().child_count, 3);
        assert_eq!(ev.node(&d, &[array, vec![0]].concat()).unwrap().value.as_int(), Some(-1));

        // The even read has no pad and no tags, and ends the stream exactly.
        let second = [records, vec![1]].concat();
        let pad = at_named(&mut ev, &d, &second, &["seq_pad"]);
        assert_eq!(ev.node(&d, &pad).unwrap().size_bits, 0);
        let node = ev.node(&d, &second).unwrap();
        assert_eq!(node.offset_bits + node.size_bits, d.len_bits());
    }

    #[test]
    fn the_first_block_of_a_bam_reads_the_header_and_the_records_that_fit_in_it() {
        let (odd, even) = two_records();
        let mut first = bam_header("@HD\tVN:1.6\n", &[("chr1", 1000)]);
        first.extend_from_slice(&odd);
        first.extend_from_slice(&even[..10]);
        let mut file = bgzf_block(&first);
        file.extend_from_slice(&bgzf_block(&even[10..]));
        file.extend_from_slice(&EOF_BLOCK);
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(bgzf());
        let payload = [at_named(&mut ev, &d, &[0, 0], &["compressed"]), vec![0]].concat();
        assert_eq!(ev.node(&d, &payload).unwrap().type_name, "Bam");
        let records = at_named(&mut ev, &d, &payload, &["records"]);
        assert_eq!(ev.node(&d, &records).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[records.clone(), vec![0]].concat()).unwrap().type_name, "AlignmentRecord");
        let cut = ev.node(&d, &[records, vec![1]].concat()).unwrap();
        assert_eq!((cut.type_name.as_str(), cut.size_bits), ("Continued", 80));

        // The second block is the rest of that record, and reads as its bytes.
        let later = [at_named(&mut ev, &d, &[0, 1], &["compressed"]), vec![0]].concat();
        assert_ne!(ev.node(&d, &later).unwrap().type_name, "Bam");
        // And the last block unpacks to nothing without a peek failing on it.
        let eof = at_named(&mut ev, &d, &[0, 2], &["compressed"]);
        assert!(ev.node(&d, &eof).is_ok());
    }

    /// A header longer than the first block: the text runs to the end of the
    /// block and nothing after it is read, rather than the next four bytes of
    /// text being taken for the reference count.
    #[test]
    fn a_header_longer_than_the_first_block_stops_where_the_block_does() {
        let text = "@CO\t".to_string() + &"x".repeat(200) + "\n";
        let header = bam_header(&text, &[("chr1", 1000)]);
        let mut file = bgzf_block(&header[..100]);
        file.extend_from_slice(&bgzf_block(&header[100..]));
        file.extend_from_slice(&EOF_BLOCK);
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(bgzf());
        let payload = [at_named(&mut ev, &d, &[0, 0], &["compressed"]), vec![0]].concat();
        let text = at_named(&mut ev, &d, &payload, &["text"]);
        assert_eq!(ev.node(&d, &text).unwrap().size_bits, 92 * 8);
        let n_ref = at_named(&mut ev, &d, &payload, &["n_ref"]);
        assert_eq!(ev.node(&d, &n_ref).unwrap().size_bits, 0);
        let records = at_named(&mut ev, &d, &payload, &["records"]);
        assert_eq!(ev.node(&d, &records).unwrap().child_count, 0);
    }

    /// One BGZF block holding `data`, compressed at the default level, the way
    /// htslib writes one.
    pub(crate) fn bgzf_block(data: &[u8]) -> Vec<u8> {
        let deflated = miniz_oxide::deflate::compress_to_vec(data, 6);
        let total = 12 + 6 + deflated.len() + 8;
        let mut v = vec![0x1f, 0x8b, 8, 4, 0, 0, 0, 0, 0, 0xff, 6, 0, b'B', b'C', 2, 0];
        v.extend_from_slice(&((total - 1) as u16).to_le_bytes());
        v.extend_from_slice(&deflated);
        v.extend_from_slice(&crate::checksum::crc32(data).to_le_bytes());
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v
    }

    /// The block every BGZF file ends with, as the specification writes it out.
    pub(crate) const EOF_BLOCK: [u8; 28] = [
        0x1f, 0x8b, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x06, 0x00, 0x42, 0x43, 0x02, 0x00, 0x1b, 0x00,
        0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn a_bgzf_file_is_a_list_of_blocks_each_as_long_as_its_bc_number() {
        let mut file = bgzf_block(b"first block of text");
        let first_len = file.len();
        file.extend_from_slice(&bgzf_block(b"and the second"));
        file.extend_from_slice(&EOF_BLOCK);
        let d = Document::new(MemSource(file.clone()));
        let mut ev = Evaluator::new(bgzf());
        let blocks = ev.node(&d, &[0]).unwrap();
        assert_eq!(blocks.child_count, 3);
        let first = ev.node(&d, &[0, 0]).unwrap();
        assert_eq!(first.size_bits, first_len as u64 * 8);
        let eof = ev.node(&d, &[0, 2]).unwrap();
        assert_eq!((eof.offset_bits / 8, eof.size_bits / 8), (file.len() as u64 - 28, 28));

        // The extra field names its subfield and the number in it.
        let extra = ev.child_named(&d, &[0, 0], "extra").unwrap().unwrap();
        let id = [extra.clone(), vec![1, 0, 0]].concat();
        assert_eq!(ev.node(&d, &id).unwrap().value, Value::Str("BC".into()));
        let bsize = [extra, vec![1, 0, 2, 0]].concat();
        assert_eq!(ev.node(&d, &bsize).unwrap().value.as_int(), Some(first_len as i128 - 1));

        // Each member's checksum covers what that member unpacks to.
        for i in 0..3 {
            let crc = ev.child_named(&d, &[0, i], "crc32").unwrap().unwrap();
            assert!(ev.run_check(&d, &crc).unwrap().expect("a verdict").ok, "block {i}");
        }
    }

    #[test]
    fn a_bgzf_file_is_told_from_a_plain_gzip_by_its_bc_subfield() {
        let file = bgzf_block(b"x");
        assert!(is_bgzf(&file));
        assert_eq!(super::super::sniff(&file, file.len() as u64), Some("bgzf"));
        // A gzip member with a name and no extra field.
        let mut plain = vec![0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 3];
        plain.extend_from_slice(b"hello.txt\0\x03\x00");
        plain.extend_from_slice(&[0; 8]);
        assert!(!is_bgzf(&plain));
        assert_eq!(super::super::sniff(&plain, plain.len() as u64), Some("gzip"));
        // An extra field with some other subfield in it is still plain gzip.
        let mut other = vec![0x1f, 0x8b, 8, 0x04, 0, 0, 0, 0, 0, 3, 6, 0, b'A', b'p', 2, 0, 1, 2];
        other.extend_from_slice(&[0x03, 0x00]);
        other.extend_from_slice(&[0; 8]);
        assert!(!is_bgzf(&other));
        assert_eq!(super::super::sniff(&other, other.len() as u64), Some("gzip"));
    }
}
