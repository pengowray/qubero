//! The alignment records that start in one BGZF block of a BAM.
//!
//! A BAM is one stream of records written through BGZF, and the blocks are
//! cut by size, not by record. So a block's unpacked bytes do not start at a
//! record: they start wherever the block before ran out, which may be halfway
//! through a read's qualities, and the only way to know where the first record
//! in a block begins is to know where the one before it ended. That chain goes
//! back to the header. Nothing in a block says where its first record is; the
//! index beside a BAM does, as virtual offsets, but only for the records it
//! chose to point at.
//!
//! So this reads the stream from the front. It steps from block to block by
//! each block's `BC` number, inflates each one, reads the header to find where
//! the records begin, and then steps from record to record by `block_size`,
//! decoding nothing until it reaches the block asked about. Every record that
//! starts in that block is decoded in full, the last of them from as many
//! following blocks as it runs into. A record is reported with the virtual
//! offset an index would give it, the block's place in the file and the byte
//! of that block's unpacked data where the record's `block_size` is.
//!
//! The walk from the front is the cost of being exact, and it is bounded by
//! [`WALK_LIMIT`]: past that much unpacked data, the reader stops and says
//! how far it got, rather than guessing where a record starts by what looks
//! like one. htslib ends a block before a record that would not fit in it, so
//! in most files a guess from the front of the block would be right; the
//! files where it would be wrong are the ones worth opening here.
//!
//! The template reads the first block as fields (see [`super::bam`]); this is
//! the arrangement [`hdf5_chunk`](super::hdf5_chunk) and
//! [`parquet_page`](super::parquet_page) already have, for what a field
//! cannot reach.

use super::bam::{BAM_MAGIC, BASES, CIGAR_OPS};

/// What [`StructDef::packed`](crate::template::StructDef::packed) calls a BGZF
/// block, so a reader can find its way from the block under the cursor to
/// here.
pub const PACKING: &str = "bgzf";

/// How much unpacked data the walk from the front of the file reads before it
/// gives up on reaching the block asked about. A 64 KB block of short reads
/// unpacks to a few hundred records, so this is several thousand blocks and a
/// few million records in.
pub const WALK_LIMIT: u64 = 256 << 20;

/// The most one BGZF block can unpack to.
const BLOCK_LIMIT: usize = 1 << 16;

/// One reference sequence, as the header lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub name: String,
    pub length: u32,
}

/// One alignment record, every field of it, with the packed ones unpacked.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Record {
    /// Where the block this record starts in begins, in bytes of the file.
    pub block_offset: u64,
    /// Where in that block's unpacked bytes the record starts, at its
    /// `block_size`.
    pub in_block: u32,
    /// How many blocks the record's bytes are spread over, counting the one
    /// it starts in.
    pub blocks: usize,
    pub block_size: u32,
    pub ref_id: i32,
    /// Counted from zero, one less than the POS of a SAM line.
    pub pos: i32,
    pub l_read_name: u8,
    pub mapq: u8,
    pub bin: u16,
    pub n_cigar_op: u16,
    pub flag: u16,
    pub l_seq: u32,
    pub next_ref_id: i32,
    pub next_pos: i32,
    pub tlen: i32,
    pub read_name: String,
    /// The CIGAR as SAM writes it, `27M1D73M`, or `*` when there is none.
    pub cigar: String,
    /// The bases, one letter each, or `*` when there are none.
    pub seq: String,
    /// The Phred scores as stored, with no 33 added. All 0xff means the
    /// record has none.
    pub qual: Vec<u8>,
    pub tags: Vec<Tag>,
    /// Why part of the record could not be read, where part of it could not.
    pub problem: Option<String>,
}

impl Record {
    /// Where the record is as an index names it: the block's offset in the
    /// file shifted up sixteen bits, and the offset inside the block below.
    pub fn virtual_offset(&self) -> u64 {
        self.block_offset << 16 | u64::from(self.in_block)
    }
}

/// One optional field.
#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    /// The two letters.
    pub tag: String,
    /// The type letter as BAM stores it, which says how wide the value was:
    /// a SAM line writes every integer type as `i`.
    pub type_code: char,
    pub value: TagValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TagValue {
    Char(char),
    Int(i64),
    Float(f32),
    Text(String),
    /// A byte array written as hex digits.
    Hex(String),
    /// A `B` array of integers, with the type letter of its elements.
    Ints(char, Vec<i64>),
    Floats(Vec<f32>),
}

impl Tag {
    /// The tag as a SAM line writes it: `AS:i:-18`, `XK:B:s,-1,0,1000`.
    pub fn sam(&self) -> String {
        let (kind, value) = match &self.value {
            TagValue::Char(c) => ('A', c.to_string()),
            TagValue::Int(n) => ('i', n.to_string()),
            TagValue::Float(f) => ('f', f.to_string()),
            TagValue::Text(s) => ('Z', s.clone()),
            TagValue::Hex(s) => ('H', s.clone()),
            TagValue::Ints(sub, v) => {
                ('B', std::iter::once(sub.to_string()).chain(v.iter().map(|n| n.to_string())).collect::<Vec<_>>().join(","))
            }
            TagValue::Floats(v) => {
                ('B', std::iter::once("f".to_string()).chain(v.iter().map(|n| n.to_string())).collect::<Vec<_>>().join(","))
            }
        };
        format!("{}:{kind}:{value}", self.tag)
    }
}

/// What one block holds: the records that start in it, and what else it had
/// to read to find them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Block {
    /// Where the block is in the file, and which block it is counting from 0.
    pub block_offset: u64,
    pub index: usize,
    /// How many bytes the block is in the file, and how many it unpacks to.
    pub packed_bytes: u32,
    pub decoded_bytes: u32,
    /// How many bytes at the front of the unpacked block are the header. The
    /// header is at the front of the stream, so only the first block has any,
    /// and the blocks after it that a long header runs into.
    pub header_bytes: u32,
    /// How many bytes after those, and before the first record that starts
    /// here, belong to a record that began in an earlier block.
    pub carried: u32,
    /// The references the header lists, which is what a record's `ref_id`
    /// counts through.
    pub references: Vec<Reference>,
    /// Every record that starts in the block, in order.
    pub records: Vec<Record>,
    /// How far the walk went: blocks inflated, and what they came to.
    pub blocks_walked: usize,
    pub bytes_walked: u64,
    /// Why the walk stopped short, where it did.
    pub problem: Option<String>,
}

/// One BGZF block's place in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Member {
    at: u64,
    len: u64,
    /// Where the deflate stream is inside the member.
    data: (usize, usize),
}

/// Read the BGZF block header at `at`: the gzip header, the extra field and
/// its `BC` subfield. `None` when it is not a BGZF block.
fn member_at<E>(read: &mut impl FnMut(u64, u64) -> Result<Vec<u8>, E>, at: u64, file_len: u64) -> Result<Option<Member>, E> {
    if file_len.saturating_sub(at) < 18 {
        return Ok(None);
    }
    let head = read(at, 12)?;
    if head[..3] != [0x1f, 0x8b, 8] || head[3] & 4 == 0 {
        return Ok(None);
    }
    let xlen = u16::from_le_bytes([head[10], head[11]]) as u64;
    if file_len - at < 12 + xlen {
        return Ok(None);
    }
    let extra = read(at + 12, xlen)?;
    let mut i = 0;
    while i + 4 <= extra.len() {
        let len = u16::from_le_bytes([extra[i + 2], extra[i + 3]]) as usize;
        if &extra[i..i + 2] == b"BC" && len == 2 && i + 6 <= extra.len() {
            let total = u16::from_le_bytes([extra[i + 4], extra[i + 5]]) as u64 + 1;
            // A member whose name, comment or header check flags are set is
            // not what BGZF writes, and its deflate stream would not start
            // where this says.
            if head[3] & 0x1a != 0 || total < 12 + xlen + 8 || at + total > file_len {
                return Ok(None);
            }
            let start = 12 + xlen as usize;
            return Ok(Some(Member { at, len: total, data: (start, total as usize - 8) }));
        }
        i += 4 + len;
    }
    Ok(None)
}

/// Inflate one member, and check it came to what its trailer says.
fn inflate(member: &Member, bytes: &[u8]) -> Option<Vec<u8>> {
    let out = miniz_oxide::inflate::decompress_to_vec_with_limit(&bytes[member.data.0..member.data.1], BLOCK_LIMIT).ok()?;
    let isize = u32::from_le_bytes(bytes[bytes.len() - 4..].try_into().ok()?);
    (out.len() as u64 == u64::from(isize)).then_some(out)
}

/// The unpacked stream, kept from the point the walk still needs onwards.
struct Stream {
    buf: Vec<u8>,
    /// Where `buf[0]` is in the stream.
    start: u64,
}

impl Stream {
    fn end(&self) -> u64 {
        self.start + self.buf.len() as u64
    }

    /// `n` bytes at `at`, when they have been unpacked.
    fn get(&self, at: u64, n: u64) -> Option<&[u8]> {
        if at < self.start || at + n > self.end() {
            return None;
        }
        let from = (at - self.start) as usize;
        Some(&self.buf[from..from + n as usize])
    }

    /// Append a block's bytes and let go of everything before `keep`.
    fn push(&mut self, bytes: &[u8], keep: u64) {
        self.buf.extend_from_slice(bytes);
        let drop = keep.saturating_sub(self.start).min(self.buf.len() as u64);
        self.buf.drain(..drop as usize);
        self.start += drop;
    }
}

fn u32_at(b: &[u8]) -> u32 {
    u32::from_le_bytes(b[..4].try_into().expect("four bytes"))
}

/// Where the walk is in the stream.
enum Phase {
    /// Before `BAM\1` and `l_text`.
    Magic,
    /// At `n_ref`.
    RefCount,
    /// Before the next of this many references.
    Refs(u32),
    Records,
}

/// The records that start in the block at `target`, a byte offset in the file
/// where a block begins.
///
/// `read` hands back `n` bytes of the file at `at`; its error is passed on as
/// it is, which is how a file still arriving says so.
pub fn records_in_block<E>(read: impl FnMut(u64, u64) -> Result<Vec<u8>, E>, file_len: u64, target: u64) -> Result<Block, E> {
    records_in_block_within(read, file_len, target, WALK_LIMIT)
}

/// The same, with the walk limit given, so a test can reach it.
pub fn records_in_block_within<E>(
    mut read: impl FnMut(u64, u64) -> Result<Vec<u8>, E>,
    file_len: u64,
    target: u64,
    limit: u64,
) -> Result<Block, E> {
    let mut out = Block { block_offset: target, ..Block::default() };
    let mut stream = Stream { buf: Vec::new(), start: 0 };
    let mut phase = Phase::Magic;
    // The next byte of the stream the walk has to read.
    let mut cursor = 0u64;
    let mut header_end = None;
    // The target block's span of the stream, once it has been inflated, and
    // where every block from it on starts, for counting the blocks a record
    // is spread over.
    let mut span: Option<(u64, u64)> = None;
    let mut starts: Vec<u64> = Vec::new();
    let mut at = 0u64;
    'walk: loop {
        // Everything the unpacked bytes so far allow.
        loop {
            // Past the end of the block asked about, nothing more can start
            // in it: every record that did has been read.
            if span.is_some_and(|(_, end)| cursor >= end) && !matches!(phase, Phase::Magic) {
                break 'walk;
            }
            match phase {
                Phase::Magic => {
                    let Some(b) = stream.get(0, 8) else { break };
                    if b[..4] != *BAM_MAGIC {
                        out.problem = Some("Not a BAM: the first block does not start with BAM\\1.".into());
                        break 'walk;
                    }
                    cursor = 8 + u64::from(u32_at(&b[4..]));
                    phase = Phase::RefCount;
                }
                Phase::RefCount => {
                    let Some(b) = stream.get(cursor, 4) else { break };
                    let n = u32_at(b);
                    cursor += 4;
                    phase = Phase::Refs(n);
                }
                Phase::Refs(0) => {
                    header_end = Some(cursor);
                    phase = Phase::Records;
                }
                Phase::Refs(left) => {
                    let Some(b) = stream.get(cursor, 4) else { break };
                    let l_name = u64::from(u32_at(b));
                    let Some(b) = stream.get(cursor, 8 + l_name) else { break };
                    let name = &b[4..4 + l_name as usize];
                    let name = name.split(|c| *c == 0).next().unwrap_or_default();
                    out.references.push(Reference {
                        name: String::from_utf8_lossy(name).into_owned(),
                        length: u32_at(&b[4 + l_name as usize..]),
                    });
                    cursor += 8 + l_name;
                    phase = Phase::Refs(left - 1);
                }
                Phase::Records => {
                    let Some(b) = stream.get(cursor, 4) else { break };
                    let size = u64::from(u32_at(b));
                    let end = cursor + 4 + size;
                    if let Some((start, _)) = span.filter(|(start, _)| cursor >= *start) {
                        let Some(b) = stream.get(cursor, 4 + size) else { break };
                        let mut record = decode_record(&b[4..]);
                        record.block_size = size as u32;
                        record.block_offset = target;
                        record.in_block = (cursor - start) as u32;
                        let last = end.saturating_sub(1);
                        record.blocks = 1 + starts.iter().filter(|s| **s > cursor && **s <= last).count();
                        out.records.push(record);
                    }
                    cursor = end;
                }
            }
        }
        // More of the stream, one block.
        let member = match member_at(&mut read, at, file_len)? {
            Some(m) => m,
            // The end of the file, with the walk still wanting bytes: the
            // block was never reached, or what starts in it runs off the end.
            None if at >= file_len => {
                out.problem = Some(match (span, &phase) {
                    (None, _) => format!("No BGZF block starts at byte {target}."),
                    (Some(_), Phase::Records) => "The file ends partway through a record.".into(),
                    (Some(_), _) => "The file ends partway through the header.".into(),
                });
                break;
            }
            None => {
                out.problem = Some(format!("Stopped at byte {at}: no BGZF block starts there."));
                break;
            }
        };
        if span.is_none() && member.at > target {
            out.problem = Some(format!("No BGZF block starts at byte {target}."));
            break;
        }
        if out.bytes_walked > limit {
            let mb = limit >> 20;
            out.problem = Some(match span {
                None => format!("Stopped at this viewer's {mb} MB limit on unpacked data, before reaching this block."),
                Some(_) => format!("Stopped at this viewer's {mb} MB limit on unpacked data: a record in this block runs past the limit."),
            });
            break;
        }
        let bytes = read(member.at, member.len)?;
        let Some(data) = inflate(&member, &bytes) else {
            out.problem = Some(format!("Stopped at byte {}: the BGZF block there would not inflate.", member.at));
            break;
        };
        out.blocks_walked += 1;
        out.bytes_walked += data.len() as u64;
        let begins = stream.end();
        if member.at == target {
            out.index = out.blocks_walked - 1;
            out.packed_bytes = member.len as u32;
            out.decoded_bytes = data.len() as u32;
            span = Some((begins, begins + data.len() as u64));
        }
        if span.is_some() {
            starts.push(begins);
        }
        // Keep what the walk has not read past. The magic is read from the
        // very front, so nothing is let go before it has been.
        let keep = if matches!(phase, Phase::Magic) { 0 } else { cursor };
        stream.push(&data, keep);
        at = member.at + member.len;
    }

    if let Some((start, end)) = span {
        // A walk that stopped inside the header has not seen where it ends,
        // and everything it did see was header.
        let header_end = header_end.unwrap_or(u64::MAX).clamp(start, end);
        out.header_bytes = (header_end - start) as u32;
        let first = out.records.first().map_or(end, |r| start + u64::from(r.in_block));
        out.carried = first.saturating_sub(header_end) as u32;
    }
    Ok(out)
}

/// Decode one record from the bytes after its `block_size`.
///
/// What cannot be read is said on the record, and what could be read before
/// it is kept: a record whose tags run wrong still has a name and a position.
pub fn decode_record(body: &[u8]) -> Record {
    let mut r = Record { block_size: body.len() as u32, ..Record::default() };
    if body.len() < 32 {
        r.problem = Some("The 32 bytes of fixed fields run past the end of the record.".into());
        return r;
    }
    let i32_at = |at: usize| i32::from_le_bytes(body[at..at + 4].try_into().expect("four bytes"));
    let u16_at = |at: usize| u16::from_le_bytes([body[at], body[at + 1]]);
    r.ref_id = i32_at(0);
    r.pos = i32_at(4);
    r.l_read_name = body[8];
    r.mapq = body[9];
    r.bin = u16_at(10);
    r.n_cigar_op = u16_at(12);
    r.flag = u16_at(14);
    r.l_seq = u32_at(&body[16..]);
    r.next_ref_id = i32_at(20);
    r.next_pos = i32_at(24);
    r.tlen = i32_at(28);

    let mut at = 32usize;
    let mut take = |n: usize, what: &str, r: &mut Record| -> Option<std::ops::Range<usize>> {
        match at.checked_add(n).filter(|end| *end <= body.len()) {
            Some(end) => {
                let range = at..end;
                at = end;
                Some(range)
            }
            None => {
                r.problem = Some(format!("The {what} runs past the end of the record."));
                None
            }
        }
    };
    let Some(name) = take(usize::from(r.l_read_name), "read name", &mut r) else { return r };
    let name = &body[name];
    r.read_name = String::from_utf8_lossy(name.split(|c| *c == 0).next().unwrap_or_default()).into_owned();

    let Some(cigar) = take(usize::from(r.n_cigar_op) * 4, "CIGAR", &mut r) else { return r };
    r.cigar = match r.n_cigar_op {
        0 => "*".into(),
        _ => body[cigar]
            .chunks(4)
            .map(|op| {
                let v = u32_at(op);
                let code = CIGAR_OPS.get((v & 15) as usize).map_or('?', |c| *c as char);
                format!("{}{code}", v >> 4)
            })
            .collect(),
    };

    let l_seq = r.l_seq as usize;
    let Some(seq) = take(l_seq.div_ceil(2), "sequence", &mut r) else { return r };
    r.seq = match l_seq {
        0 => "*".into(),
        _ => (0..l_seq).map(|i| BASES[usize::from(body[seq.start + i / 2] >> (4 * (1 - i % 2)) & 15)] as char).collect(),
    };
    let Some(qual) = take(l_seq, "quality string", &mut r) else { return r };
    r.qual = body[qual].to_vec();

    let (tags, problem) = decode_tags(&body[at..]);
    r.tags = tags;
    if problem.is_some() {
        r.problem = problem;
    }
    r
}

/// The optional fields, from the first to the end of the record.
fn decode_tags(mut b: &[u8]) -> (Vec<Tag>, Option<String>) {
    let mut tags = Vec::new();
    while !b.is_empty() {
        if b.len() < 3 {
            return (tags, Some("The record ends partway through a tag.".into()));
        }
        let tag = String::from_utf8_lossy(&b[..2]).into_owned();
        let code = b[2] as char;
        b = &b[3..];
        let short = |tags: Vec<Tag>, tag: &str| (tags, Some(format!("Tag {tag} runs past the end of the record.")));
        let value = match code {
            'A' | 'c' | 'C' | 's' | 'S' | 'i' | 'I' | 'f' => {
                let Some((v, n)) = scalar(code, b) else { return short(tags, &tag) };
                b = &b[n..];
                v
            }
            'Z' | 'H' => {
                let Some(end) = b.iter().position(|c| *c == 0) else { return short(tags, &tag) };
                let text = String::from_utf8_lossy(&b[..end]).into_owned();
                b = &b[end + 1..];
                if code == 'Z' { TagValue::Text(text) } else { TagValue::Hex(text) }
            }
            'B' => {
                if b.len() < 5 {
                    return short(tags, &tag);
                }
                let sub = b[0] as char;
                let count = u32_at(&b[1..]) as usize;
                let Some(width) = width(sub) else {
                    return (tags, Some(format!("Tag {tag} is an array with element type {sub}, which the specification does not define; the tags after it were not read.")));
                };
                let Some(bytes) = b.get(5..count.checked_mul(width).and_then(|n| n.checked_add(5)).unwrap_or(usize::MAX)) else {
                    return short(tags, &tag);
                };
                let v = match sub {
                    'f' => TagValue::Floats(bytes.chunks(4).map(|c| f32::from_le_bytes(c.try_into().expect("four bytes"))).collect()),
                    _ => TagValue::Ints(
                        sub,
                        bytes.chunks(width).map(|c| match scalar(sub, c) {
                            Some((TagValue::Int(n), _)) => n,
                            _ => 0,
                        }).collect(),
                    ),
                };
                b = &b[5 + bytes.len()..];
                v
            }
            other => {
                return (tags, Some(format!("Tag {tag} has type {other}, which the specification does not define; the tags after it were not read.")));
            }
        };
        tags.push(Tag { tag, type_code: code, value });
    }
    (tags, None)
}

/// How many bytes one value of a numeric type letter takes.
fn width(code: char) -> Option<usize> {
    Some(match code {
        'A' | 'c' | 'C' => 1,
        's' | 'S' => 2,
        'i' | 'I' | 'f' => 4,
        _ => return None,
    })
}

/// One value of a fixed-width type, and how many bytes it took.
fn scalar(code: char, b: &[u8]) -> Option<(TagValue, usize)> {
    let n = width(code)?;
    let b = b.get(..n)?;
    let v = match code {
        'A' => TagValue::Char(b[0] as char),
        'c' => TagValue::Int(i64::from(b[0] as i8)),
        'C' => TagValue::Int(i64::from(b[0])),
        's' => TagValue::Int(i64::from(i16::from_le_bytes([b[0], b[1]]))),
        'S' => TagValue::Int(i64::from(u16::from_le_bytes([b[0], b[1]]))),
        'i' => TagValue::Int(i64::from(i32::from_le_bytes(b.try_into().ok()?))),
        'I' => TagValue::Int(i64::from(u32::from_le_bytes(b.try_into().ok()?))),
        _ => TagValue::Float(f32::from_le_bytes(b.try_into().ok()?)),
    };
    Some((v, n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::bam::tests::{bam_header, bgzf_block, record_bytes, two_records, Rec, EOF_BLOCK};

    fn walk(file: &[u8], target: u64) -> Block {
        let len = file.len() as u64;
        let read = |at: u64, n: u64| -> Result<Vec<u8>, ()> { Ok(file[at as usize..(at + n) as usize].to_vec()) };
        records_in_block(read, len, target).unwrap()
    }

    /// Offsets of each block in a file built from these unpacked blocks.
    fn build(blocks: &[&[u8]]) -> (Vec<u8>, Vec<u64>) {
        let mut file = Vec::new();
        let mut at = Vec::new();
        for b in blocks {
            at.push(file.len() as u64);
            file.extend_from_slice(&bgzf_block(b));
        }
        at.push(file.len() as u64);
        file.extend_from_slice(&EOF_BLOCK);
        (file, at)
    }

    #[test]
    fn a_record_decodes_into_every_field_and_every_tag_type() {
        let (odd, even) = two_records();
        let r = decode_record(&odd[4..]);
        assert_eq!(r.problem, None);
        assert_eq!((r.ref_id, r.pos, r.mapq, r.flag), (0, 99, 60, 0x63));
        assert_eq!(r.read_name, "read/1");
        assert_eq!(r.cigar, "3S4M");
        assert_eq!(r.seq, "ACGTNAC");
        assert_eq!(r.qual, vec![30, 31, 32, 33, 2, 40, 41]);
        let sam: Vec<String> = r.tags.iter().map(Tag::sam).collect();
        assert_eq!(
            sam,
            [
                "XA:A:q", "XB:i:-2", "XC:i:254", "XD:i:-300", "XE:i:65000", "XF:i:-70000", "XG:i:3000000000", "XH:f:1.5",
                "XI:Z:hello", "XJ:H:1AE3", "XK:B:s,-1,0,1000", "XL:B:f,0.25",
            ]
        );
        // The type letter BAM stored is kept, since SAM throws it away.
        assert_eq!(r.tags.iter().map(|t| t.type_code).collect::<String>(), "AcCsSiIfZHBB");

        let r = decode_record(&even[4..]);
        assert_eq!((r.cigar.as_str(), r.seq.as_str(), r.tags.len()), ("2M1D2M", "GGTT", 0));
    }

    #[test]
    fn a_record_with_no_cigar_or_sequence_reads_as_stars() {
        let bytes = record_bytes(&Rec { name: "u", ref_id: -1, pos: -1, mapq: 0, flag: 4, cigar: &[], seq: "", qual: &[], tags: &[] });
        let r = decode_record(&bytes[4..]);
        assert_eq!((r.cigar.as_str(), r.seq.as_str(), r.problem), ("*", "*", None));
    }

    /// A type letter nobody defined leaves the rest of the record unread, and
    /// says which tag it was, rather than guessing a width.
    #[test]
    fn a_tag_of_a_type_nobody_defined_stops_the_tags_and_says_which() {
        let mut tags = b"XAAq".to_vec();
        tags.extend_from_slice(b"XQq\x01\x02");
        let bytes = record_bytes(&Rec { name: "t", ref_id: 0, pos: 0, mapq: 0, flag: 0, cigar: &[], seq: "A", qual: &[9], tags: &tags });
        let r = decode_record(&bytes[4..]);
        assert_eq!(r.tags.len(), 1);
        assert!(r.problem.expect("a problem").contains("XQ"));
    }

    /// Three records laid over three blocks so that the second starts in the
    /// first block and ends in the second, and the fourth runs over the whole
    /// of the third block into a fourth.
    #[test]
    fn the_records_that_start_in_a_block_are_found_by_walking_from_the_header() {
        let tags: Vec<u8> = Vec::new();
        let long_seq = "ACGT".repeat(40);
        let long_qual = vec![20u8; 160];
        let rec = |name: &str| record_bytes(&Rec { name, ref_id: 0, pos: 7, mapq: 1, flag: 0, cigar: &[(4, b'M')], seq: "ACGT", qual: &[1, 2, 3, 4], tags: &tags });
        let long = record_bytes(&Rec { name: "long", ref_id: 1, pos: 9, mapq: 2, flag: 16, cigar: &[(160, b'M')], seq: &long_seq, qual: &long_qual, tags: &[] });
        let (r1, r2, r3) = (rec("one"), rec("two"), rec("three"));
        let mut stream = bam_header("@HD\tVN:1.6\n", &[("chr1", 100), ("chr2", 200)]);
        let header_len = stream.len();
        for r in [&r1, &r2, &r3, &long] {
            stream.extend_from_slice(r);
        }
        // Cut: block 0 ends ten bytes into two; block 1 ends 30 bytes into
        // long; block 2 is 100 bytes of long; block 3 is the rest.
        let a = header_len + r1.len() + 10;
        let b = header_len + r1.len() + r2.len() + r3.len() + 30;
        let c = b + 100;
        let (file, at) = build(&[&stream[..a], &stream[a..b], &stream[b..c], &stream[c..]]);

        let first = walk(&file, at[0]);
        assert_eq!(first.problem, None);
        assert_eq!(first.references.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["chr1", "chr2"]);
        assert_eq!(first.records.iter().map(|r| r.read_name.as_str()).collect::<Vec<_>>(), ["one", "two"]);
        assert_eq!((first.header_bytes, first.carried), (header_len as u32, 0));
        assert_eq!(first.records[1].blocks, 2);
        assert_eq!(first.records[0].in_block, header_len as u32);

        let second = walk(&file, at[1]);
        assert_eq!(second.problem, None);
        assert_eq!(second.records.iter().map(|r| r.read_name.as_str()).collect::<Vec<_>>(), ["three", "long"]);
        assert_eq!((second.header_bytes, second.carried), (0, r2.len() as u32 - 10));
        assert_eq!(second.records[0].in_block, r2.len() as u32 - 10);
        assert_eq!(second.records[0].virtual_offset(), at[1] << 16 | u64::from(second.records[0].in_block));
        let long = &second.records[1];
        assert_eq!((long.blocks, long.seq.as_str(), long.cigar.as_str()), (3, long_seq.as_str(), "160M"));

        // A block holding nothing but the middle of a record starts none.
        let third = walk(&file, at[2]);
        assert_eq!((third.records.len(), third.carried, third.decoded_bytes), (0, 100, 100));
        assert_eq!(third.problem, None);

        // The last block unpacks to nothing and starts nothing, and the walk
        // says nothing is wrong.
        let eof = walk(&file, at[4]);
        assert_eq!((eof.records.len(), eof.decoded_bytes, eof.problem), (0, 0, None));

        // An offset where no block starts is said to be one.
        assert!(walk(&file, at[1] + 1).problem.expect("a problem").contains("No BGZF block"));
    }

    #[test]
    fn a_header_that_runs_into_a_later_block_is_counted_as_header_there() {
        let text = "@CO\t".to_string() + &"y".repeat(300) + "\n";
        let (odd, _) = two_records();
        let mut stream = bam_header(&text, &[("chr1", 100)]);
        let header_len = stream.len();
        stream.extend_from_slice(&odd);
        let (file, at) = build(&[&stream[..100], &stream[100..]]);
        let later = walk(&file, at[1]);
        assert_eq!(later.problem, None);
        assert_eq!((later.header_bytes as usize, later.carried), (header_len - 100, 0));
        assert_eq!(later.records.len(), 1);
        assert_eq!(later.records[0].read_name, "read/1");
        let first = walk(&file, at[0]);
        assert_eq!((first.records.len(), first.header_bytes, first.carried), (0, 100, 0));
    }

    #[test]
    fn a_bgzf_file_that_is_not_a_bam_says_so() {
        let (file, at) = build(&[b"##fileformat=VCFv4.2\n"]);
        assert!(walk(&file, at[0]).problem.expect("a problem").starts_with("Not a BAM"));
    }

    #[test]
    fn the_walk_stops_at_its_limit_and_says_it_did() {
        let (odd, even) = two_records();
        let mut stream = bam_header("", &[]);
        for _ in 0..40 {
            stream.extend_from_slice(&odd);
            stream.extend_from_slice(&even);
        }
        let blocks: Vec<&[u8]> = stream.chunks(500).collect();
        let (file, at) = build(&blocks);
        let len = file.len() as u64;
        let read = |a: u64, n: u64| -> Result<Vec<u8>, ()> { Ok(file[a as usize..(a + n) as usize].to_vec()) };
        let got = records_in_block_within(read, len, at[at.len() - 2], 1000).unwrap();
        assert!(got.problem.expect("a problem").starts_with("Stopped at this viewer's 0 MB limit"));
        // And within the limit the same block reads.
        let read = |a: u64, n: u64| -> Result<Vec<u8>, ()> { Ok(file[a as usize..(a + n) as usize].to_vec()) };
        let got = records_in_block_within(read, len, at[at.len() - 2], 1 << 20).unwrap();
        assert_eq!(got.problem, None);
        assert!(!got.records.is_empty());
    }
}
