//! What the xz reader is held to: the bytes `lzma-rs` gives, a trace that
//! tiles the stream, and symbols inside the blocks whose chains allow it.
//!
//! Streams had to be arranged for. Nothing that ships writes xz, so a stream
//! here is either built out of an LZMA2 stream `lzma-rust2` packed, wrapped in
//! the header, index and footer the format wants, or it is bytes the `xz` tool
//! itself wrote, kept as a literal because the tool is not on every machine
//! this runs on. The built ones are what the settings are walked over; the
//! written ones are what says the reading agrees with a real encoder, which
//! is the thing a builder written from the same understanding as the reader
//! cannot say.

use super::*;
use crate::checksum::crc32;
use crate::codec::{decode, decode_traced, Codec, Step};

/// The dictionary size code written into every built block: the 8 MiB of
/// preset 6, which is at or above every dictionary the tests pack with. Our
/// own decoder allocates what the stream needs and does not read this, but
/// `lzma-rs` may, and a code below the encoder's dictionary would be a stream
/// nobody should be able to read.
const DICT_CODE: u8 = 22;

/// One of xz's variable-length integers, written the one way it may be:
/// seven bits a byte, least significant first, no trailing zero byte.
pub(crate) fn vli_of(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while v >= 0x80 {
        out.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
    out
}

/// One block's header: a length in units of four, the flags, whichever sizes
/// `sized` asked for, one LZMA2 filter, zeroes out to the length, and a CRC32.
pub(crate) fn block_header(packed: usize, unpacked: usize, sized: bool) -> Vec<u8> {
    let mut body = vec![0x00, if sized { 0xc0 } else { 0x00 }];
    if sized {
        body.extend_from_slice(&vli_of(packed as u64));
        body.extend_from_slice(&vli_of(unpacked as u64));
    }
    // The filter: which one, how many bytes of settings, and those bytes.
    body.extend_from_slice(&[0x21, 0x01, DICT_CODE]);
    // Out to a multiple of four with the CRC32 counted in, and the first byte
    // says what that came to.
    let len = (body.len() + 4).next_multiple_of(4);
    body.resize(len - 4, 0);
    body[0] = (len / 4 - 1) as u8;
    let sum = crc32(&body);
    body.extend_from_slice(&sum.to_le_bytes());
    body
}

/// An xz stream around LZMA2 streams somebody else packed: one block each,
/// with CRC32 as the check.
///
/// `sized` writes both of the sizes a block header may carry, which is what
/// the threaded encoder does and what `xz -T1` leaves out. Both are read, so
/// both are built.
pub(crate) fn wrap(blocks: &[(Vec<u8>, Vec<u8>)], sized: bool) -> Vec<u8> {
    wrap_checked(blocks, sized, 1)
}

/// The check bytes a block of `plain` ends with, for the stream flags' check
/// type. Zeros for an ID the format has reserved: it has a length and no
/// algorithm, so there is a run to write and nothing that belongs in it.
pub(crate) fn check_bytes(check_type: u8, plain: &[u8]) -> Vec<u8> {
    match check_type {
        0 => Vec::new(),
        1 => crc32(plain).to_le_bytes().to_vec(),
        4 => crate::checksum::crc64_xz(plain).to_le_bytes().to_vec(),
        10 => crate::checksum::sha256(plain).to_vec(),
        other => vec![0u8; check_size(other)],
    }
}

/// The same stream with the check named, so a test can build the default
/// settings `xz` writes (CRC64), the SHA-256 an encoder can be asked for, a
/// stream with no check at all, and one whose check nobody has defined.
pub(crate) fn wrap_checked(blocks: &[(Vec<u8>, Vec<u8>)], sized: bool, check_type: u8) -> Vec<u8> {
    let check_len = check_size(check_type);
    let flags = [0x00u8, check_type];
    let mut v = MAGIC.to_vec();
    v.extend_from_slice(&flags);
    v.extend_from_slice(&crc32(&flags).to_le_bytes());

    let mut records = Vec::new();
    for (packed, plain) in blocks {
        let header = block_header(packed.len(), plain.len(), sized);
        v.extend_from_slice(&header);
        v.extend_from_slice(packed);
        while v.len() % 4 != 0 {
            v.push(0);
        }
        v.extend_from_slice(&check_bytes(check_type, plain));
        records.extend_from_slice(&vli_of((header.len() + packed.len() + check_len) as u64));
        records.extend_from_slice(&vli_of(plain.len() as u64));
    }

    let index_at = v.len();
    v.push(0x00);
    v.extend_from_slice(&vli_of(blocks.len() as u64));
    v.extend_from_slice(&records);
    while (v.len() - index_at) % 4 != 0 {
        v.push(0);
    }
    v.extend_from_slice(&crc32(&v[index_at..]).to_le_bytes());
    let index_size = v.len() - index_at;

    let mut footer = ((index_size / 4 - 1) as u32).to_le_bytes().to_vec();
    footer.extend_from_slice(&flags);
    v.extend_from_slice(&crc32(&footer).to_le_bytes());
    v.extend_from_slice(&footer);
    v.extend_from_slice(FOOTER_MAGIC);
    v
}

/// The same data as LZMA2, the way [`crate::codec::lzma`]'s own tests pack it.
pub(crate) fn pack(data: &[u8], lc: u32, lp: u32, pb: u32, dict: u32, chunk: Option<u64>) -> Vec<u8> {
    use std::num::NonZeroU64;
    use lzma_rust2::{Lzma2Options, Lzma2Writer};
    let mut opts = Lzma2Options::with_preset(6);
    opts.lzma_options.lc = lc;
    opts.lzma_options.lp = lp;
    opts.lzma_options.pb = pb;
    opts.lzma_options.dict_size = dict;
    opts.chunk_size = chunk.and_then(NonZeroU64::new);
    let mut w = Lzma2Writer::new(Vec::new(), opts);
    std::io::Write::write_all(&mut w, data).expect("packs");
    w.finish().expect("finishes")
}

/// The same bytes as an LZMA2 stream that compresses none of them: one
/// uncompressed chunk per 64 KiB, and the end marker.
///
/// What this is for is corrupting. A byte changed inside a range-coded chunk
/// does not give the same bytes wrongly, it gives a decoder that walks off
/// into nothing and a block that comes to the wrong length, which the index
/// catches and the stream is refused over: a test wanting a file that reads
/// and fails its integrity check would never get one. An uncompressed chunk
/// has no such coupling. Change a byte of it and the block still decodes, to
/// exactly as many bytes as the index says, one of which is now wrong, which
/// is precisely the file a block check exists to catch.
///
/// The control byte is 0x01: an uncompressed chunk that resets the
/// dictionary, which is what one has to be at the start of a stream. The two
/// bytes after it are the length less one, big-endian, and the format's own
/// order for that number is the other way round from every other number in an
/// xz file.
pub(crate) fn stored_lzma2(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, chunk) in data.chunks(1 << 16).enumerate() {
        // Only the first resets the dictionary; the ones after it carry on
        // from where it left off, which is what 0x02 says.
        out.push(if i == 0 { 0x01 } else { 0x02 });
        out.extend_from_slice(&((chunk.len() - 1) as u16).to_be_bytes());
        out.extend_from_slice(chunk);
    }
    out.push(0x00);
    out
}

/// What `lzma-rs` makes of a whole stream, which is what the bytes have to be.
fn oracle(stream: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    lzma_rs::xz_decompress(&mut std::io::BufReader::new(stream), &mut out).ok()?;
    Some(out)
}

/// Data with something in it to match against, so a stream of it holds
/// literals, matches, repeated distances and short reps together.
fn mixed(n: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(n);
    let mut i = 0u32;
    while v.len() < n {
        match i % 5 {
            0 => v.extend_from_slice(b"the quick brown fox jumps over the lazy dog. "),
            1 => v.extend(std::iter::repeat_n(b'-', 30)),
            2 => v.extend((0..40u32).map(|k| (k.wrapping_mul(2654435761) >> 24) as u8)),
            3 => v.extend_from_slice(b"the quick brown fox "),
            _ => v.extend_from_slice(&[0; 17]),
        }
        i += 1;
    }
    v.truncate(n);
    v
}

/// Every claim a trace makes about itself, on one stream: that its steps tile
/// it, that its ends are the stream's own ends, that a step naming a header
/// wrote nothing, that a literal wrote the byte it names, that a match copied
/// what it said, and that every byte of the output can be traced back.
fn check(trace: &Trace, data: &[u8], out: &[u8]) {
    trace.check_tiles().expect("the steps tile the stream");
    assert_eq!(trace.in_bits(), data.len() as u64 * 8, "the trace stops short of the stream");
    assert_eq!(trace.out_bytes(), out.len() as u64, "the trace stops short of the output");
    if trace.coarse() {
        return;
    }
    for (i, s) in trace.steps().enumerate() {
        let Step { in_bits, out_bytes, kind } = s;
        assert_eq!(in_bits.start % 8, 0, "step {i} starts inside a byte");
        let wrote = out_bytes.end - out_bytes.start;
        match kind {
            StepKind::Header(..) | StepKind::EndOfBlock => {
                assert_eq!(wrote, 0, "step {i} is a {kind:?} and claims to have written {wrote}")
            }
            StepKind::Literal(byte) => {
                assert_eq!(wrote, 1, "step {i} is a literal over {wrote} bytes");
                assert_eq!(out[out_bytes.start as usize], byte, "step {i} names the wrong byte");
            }
            StepKind::Match { len, dist } => {
                assert_eq!(u64::from(len), wrote, "step {i} is a match of {len} over {wrote} bytes");
                let at = out_bytes.start as usize;
                assert!(dist >= 1 && u64::from(dist) <= out_bytes.start, "step {i} reaches back {dist}");
                for k in 0..len as usize {
                    assert_eq!(out[at + k], out[at + k - dist as usize], "step {i} byte {k}");
                }
            }
            StepKind::Stored | StepKind::Block => assert!(wrote > 0, "step {i} produced nothing"),
            other => panic!("step {i} is a {other:?}, which no xz stream should hold"),
        }
    }
    for byte in (0..out.len() as u64).step_by(7) {
        assert!(trace.map_out(byte).is_some(), "output byte {byte} came from nowhere");
    }
    for bit in (0..data.len() as u64 * 8).step_by(8 * 7) {
        assert!(trace.map_in(bit).is_some(), "input bit {bit} went nowhere");
    }
}

/// One stream, read both ways, against every claim above and against the
/// crate. The bytes are the half that must never move.
fn same(data: &[u8], stream: &[u8]) {
    let want = oracle(stream).expect("lzma-rs reads it");
    assert_eq!(want, data, "the builder and lzma-rs disagree, so nothing here is being tested");
    let (out, trace) = decode_traced(Codec::Xz, stream).expect("reads");
    assert_eq!(out, want, "our bytes are not the bytes lzma-rs gives");
    // The other door of `codec`, which must not be able to answer differently.
    assert_eq!(decode(Codec::Xz, stream).expect("reads"), want, "the two doors disagree");
    check(&trace, stream, &out);
    // And our own decoder is what read it. Without this the test would pass on
    // a stream that quietly fell back to the crate: the bytes would be the
    // crate's bytes, which is what is being compared, and the block map over
    // them tiles as happily as a trace of symbols does. A block left unopened
    // is the one thing that tells the two apart.
    assert!(!trace.steps().any(|s| s.kind == StepKind::Block), "the stream fell back to the crate");
}

/// A block whose chain is one LZMA2 filter is read to its symbols: a click on
/// a byte of the text lands on the literal or the match that wrote it, the
/// way it does inside a zip.
#[test]
fn a_stream_of_plain_lzma2_is_read_down_to_its_symbols() {
    let data = mixed(20_000);
    let stream = wrap(&[(pack(&data, 3, 0, 2, 1 << 20, None), data.clone())], false);
    let (out, trace) = decode_traced(Codec::Xz, &stream).expect("reads");
    assert_eq!(out, data);
    check(&trace, &stream, &out);

    assert!(!trace.coarse(), "twenty thousand bytes is not enough to give up naming symbols");
    assert!(
        trace.steps().any(|s| matches!(s.kind, StepKind::Literal(_))),
        "a stream traced to its symbols holds literals"
    );
    assert!(
        trace.steps().any(|s| matches!(s.kind, StepKind::Match { .. })),
        "a stream of repeating text holds matches"
    );
    // And nothing is a block-shaped step, which is what the whole run used to
    // be and what a fallback still gives.
    assert!(!trace.steps().any(|s| s.kind == StepKind::Block), "a block is left unopened");

    // Every byte of the output names the step that wrote it, and that step's
    // bits are inside the stream rather than at the front of some block's own
    // count of itself.
    for byte in [0u64, 1, 999, 10_000, data.len() as u64 - 1] {
        let s = trace.map_out(byte).unwrap_or_else(|| panic!("byte {byte} came from nowhere"));
        assert!(s.out_bytes.contains(&byte), "byte {byte} was found in {s:?}");
        assert!(
            matches!(s.kind, StepKind::Literal(_) | StepKind::Match { .. } | StepKind::Stored),
            "byte {byte} was written by a {:?}",
            s.kind
        );
        assert!(s.in_bits.end <= stream.len() as u64 * 8, "byte {byte} was read from past the stream");
    }
    // The blocks of the trace are the LZMA2 chunks, which is a shape the old
    // reading could not show at all.
    assert!(!trace.blocks().is_empty(), "the chunks inside the block are blocks of the trace");
}

/// The bytes, over the settings an encoder can be asked for, with and without
/// the sizes a block header may carry.
///
/// This is the test that matters: our own decoder is here for the trace, and
/// the bytes have to be the bytes the crate gives whatever the stream was
/// packed with.
#[test]
fn a_stream_reads_as_the_bytes_lzma_rs_gives_at_every_setting() {
    let data = mixed(60_000);
    // `lc + lp` no greater than four, which is what LZMA2 asks of a properties
    // byte and what an encoder will write.
    for (lc, lp, pb) in [(3, 0, 2), (0, 0, 0), (1, 1, 1), (0, 4, 4), (4, 0, 0), (2, 2, 3)] {
        for chunk in [None, Some(1 << 12)] {
            for sized in [false, true] {
                let packed = pack(&data, lc, lp, pb, 1 << 20, chunk);
                let stream = wrap(&[(packed, data.clone())], sized);
                same(&data, &stream);
            }
        }
    }
}

/// Nothing at all is a stream too: one block that unpacks to no bytes, and a
/// trace over a stream that produced none.
#[test]
fn a_stream_of_nothing_is_a_stream() {
    for data in [vec![], vec![b'x'], vec![0xffu8; 300]] {
        let stream = wrap(&[(pack(&data, 3, 0, 2, 1 << 16, None), data.clone())], true);
        same(&data, &stream);
    }
}

/// Several blocks in one stream, which is what `xz --block-size` writes and
/// what a threaded encoder always writes.
///
/// The stitching is the thing under test: each block is decoded on its own,
/// with its own dictionary and its own range coder, and the steps of all of
/// them have to come out as one trace over one run. Getting the offsets wrong
/// gives a trace that does not tile, and getting the output offsets wrong
/// gives one that tiles and points at the wrong bytes, so both are asked.
#[test]
fn blocks_are_stitched_into_one_run() {
    let parts: Vec<Vec<u8>> = (0..4).map(|i| mixed(5000 + i * 700)).collect();
    let blocks: Vec<(Vec<u8>, Vec<u8>)> =
        parts.iter().map(|p| (pack(p, 3, 0, 2, 1 << 20, None), p.clone())).collect();
    let data: Vec<u8> = parts.concat();
    let stream = wrap(&blocks, false);
    same(&data, &stream);

    let (out, trace) = decode_traced(Codec::Xz, &stream).expect("reads");
    assert_eq!(out, data);
    // A byte from every block, found through the trace, read from inside that
    // block's own bytes of the stream. The last is what a wrong output offset
    // would put in the wrong place.
    let mut at = 0u64;
    let mut block_at = HEADER as u64;
    for (i, part) in parts.iter().enumerate() {
        let byte = at + part.len() as u64 / 2;
        let s = trace.map_out(byte).unwrap_or_else(|| panic!("byte {byte} of block {i} came from nowhere"));
        assert!(s.in_bits.start >= block_at * 8, "block {i} read from before itself: {s:?}");
        at += part.len() as u64;
        block_at += (blocks[i].0.len() + 16).next_multiple_of(4) as u64;
        assert!(s.in_bits.end <= block_at * 8, "block {i} read past itself: {s:?}");
    }
    assert_eq!(at, data.len() as u64);
}

/// Three blocks the `xz` tool wrote, with no sizes in the block headers,
/// which is what the single-threaded encoder writes.
///
/// `xz -9 --block-size=200` over six hundred bytes of repeating text.
const MULTI_BLOCK: &[u8] = &[
    0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x04, 0xe6, 0xd6, 0xb4, 0x46, //
    0x02, 0x00, 0x21, 0x01, 0x1c, 0x00, 0x00, 0x00, 0x10, 0xcf, 0x58, 0xcc, //
    0xe0, 0x00, 0xc7, 0x00, 0x32, 0x5d, 0x00, 0x3a, 0x1a, 0x08, 0xce, 0x76, //
    0xc7, 0xe5, 0xe9, 0xd6, 0x07, 0x34, 0xc3, 0xd1, 0x0e, 0xbf, 0xce, 0x55, //
    0xe1, 0xaa, 0xbd, 0xe0, 0xe4, 0x8f, 0x98, 0x01, 0xdd, 0x8d, 0xe5, 0x07, //
    0x54, 0x9e, 0x65, 0x25, 0x5f, 0x27, 0x3a, 0x6a, 0x7e, 0xb4, 0xd3, 0x49, //
    0x03, 0x89, 0xce, 0x4b, 0x93, 0x30, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, //
    0x51, 0xb8, 0x87, 0x6a, 0x61, 0xf3, 0xa3, 0x43, 0x02, 0x00, 0x21, 0x01, //
    0x1c, 0x00, 0x00, 0x00, 0x10, 0xcf, 0x58, 0xcc, 0xe0, 0x00, 0xc7, 0x00, //
    0x30, 0x5d, 0x00, 0x35, 0x1d, 0x49, 0xde, 0xb3, 0x04, 0xe9, 0x0e, 0xa5, //
    0xe9, 0x06, 0x14, 0x7e, 0x3a, 0xce, 0x2c, 0x7d, 0xc0, 0x4f, 0x55, 0x4f, //
    0xae, 0x4e, 0x55, 0xbf, 0xa0, 0x94, 0xe8, 0xb2, 0xa4, 0x9c, 0x4c, 0x81, //
    0x63, 0x48, 0xde, 0xe0, 0x29, 0x5f, 0xd2, 0xeb, 0x4f, 0xad, 0x2a, 0x6e, //
    0x2c, 0x58, 0x00, 0x00, 0x1e, 0x20, 0xc4, 0xbc, 0x7b, 0x54, 0x1e, 0xbc, //
    0x02, 0x00, 0x21, 0x01, 0x1c, 0x00, 0x00, 0x00, 0x10, 0xcf, 0x58, 0xcc, //
    0xe0, 0x00, 0xc7, 0x00, 0x31, 0x5d, 0x00, 0x32, 0x1b, 0xc9, 0x15, 0x88, //
    0x50, 0x56, 0x42, 0xff, 0x80, 0x62, 0x72, 0xab, 0x59, 0x5d, 0x1d, 0x32, //
    0x4f, 0xe2, 0x1f, 0xed, 0x20, 0x0a, 0xaa, 0xe0, 0xb8, 0xc9, 0x15, 0xda, //
    0xb1, 0x64, 0xd6, 0x06, 0x5f, 0x38, 0x9c, 0x1b, 0x2b, 0x66, 0xea, 0x36, //
    0xd3, 0x67, 0xd4, 0x95, 0x5a, 0xeb, 0xe0, 0x00, 0x00, 0x00, 0x00, 0x00, //
    0xed, 0xf8, 0xc5, 0xb3, 0x7e, 0xd6, 0xa3, 0x9b, 0x00, 0x03, 0x4e, 0xc8, //
    0x01, 0x4c, 0xc8, 0x01, 0x4d, 0xc8, 0x01, 0x00, 0x8e, 0xea, 0x58, 0x01, //
    0x14, 0x17, 0x3b, 0x30, 0x03, 0x00, 0x00, 0x00, 0x00, 0x04, 0x59, 0x5a,
];

/// The same six hundred bytes through the threaded encoder, which writes both
/// of the sizes a block header may carry.
const MULTI_BLOCK_SIZED: &[u8] = &[
    0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x04, 0xe6, 0xd6, 0xb4, 0x46, //
    0x03, 0xc0, 0x3a, 0xc8, 0x01, 0x21, 0x01, 0x1c, 0x00, 0x00, 0x00, 0x00, //
    0xde, 0x10, 0x00, 0x1a, 0xe0, 0x00, 0xc7, 0x00, 0x32, 0x5d, 0x00, 0x3a, //
    0x1a, 0x08, 0xce, 0x76, 0xc7, 0xe5, 0xe9, 0xd6, 0x07, 0x34, 0xc3, 0xd1, //
    0x0e, 0xbf, 0xce, 0x55, 0xe1, 0xaa, 0xbd, 0xe0, 0xe4, 0x8f, 0x98, 0x01, //
    0xdd, 0x8d, 0xe5, 0x07, 0x54, 0x9e, 0x65, 0x25, 0x5f, 0x27, 0x3a, 0x6a, //
    0x7e, 0xb4, 0xd3, 0x49, 0x03, 0x89, 0xce, 0x4b, 0x93, 0x30, 0x06, 0x00, //
    0x00, 0x00, 0x00, 0x00, 0x51, 0xb8, 0x87, 0x6a, 0x61, 0xf3, 0xa3, 0x43, //
    0x03, 0xc0, 0x38, 0xc8, 0x01, 0x21, 0x01, 0x1c, 0x00, 0x00, 0x00, 0x00, //
    0xe3, 0xc0, 0xf5, 0x1e, 0xe0, 0x00, 0xc7, 0x00, 0x30, 0x5d, 0x00, 0x35, //
    0x1d, 0x49, 0xde, 0xb3, 0x04, 0xe9, 0x0e, 0xa5, 0xe9, 0x06, 0x14, 0x7e, //
    0x3a, 0xce, 0x2c, 0x7d, 0xc0, 0x4f, 0x55, 0x4f, 0xae, 0x4e, 0x55, 0xbf, //
    0xa0, 0x94, 0xe8, 0xb2, 0xa4, 0x9c, 0x4c, 0x81, 0x63, 0x48, 0xde, 0xe0, //
    0x29, 0x5f, 0xd2, 0xeb, 0x4f, 0xad, 0x2a, 0x6e, 0x2c, 0x58, 0x00, 0x00, //
    0x1e, 0x20, 0xc4, 0xbc, 0x7b, 0x54, 0x1e, 0xbc, 0x03, 0xc0, 0x39, 0xc8, //
    0x01, 0x21, 0x01, 0x1c, 0x00, 0x00, 0x00, 0x00, 0xdd, 0xab, 0x37, 0xf1, //
    0xe0, 0x00, 0xc7, 0x00, 0x31, 0x5d, 0x00, 0x32, 0x1b, 0xc9, 0x15, 0x88, //
    0x50, 0x56, 0x42, 0xff, 0x80, 0x62, 0x72, 0xab, 0x59, 0x5d, 0x1d, 0x32, //
    0x4f, 0xe2, 0x1f, 0xed, 0x20, 0x0a, 0xaa, 0xe0, 0xb8, 0xc9, 0x15, 0xda, //
    0xb1, 0x64, 0xd6, 0x06, 0x5f, 0x38, 0x9c, 0x1b, 0x2b, 0x66, 0xea, 0x36, //
    0xd3, 0x67, 0xd4, 0x95, 0x5a, 0xeb, 0xe0, 0x00, 0x00, 0x00, 0x00, 0x00, //
    0xed, 0xf8, 0xc5, 0xb3, 0x7e, 0xd6, 0xa3, 0x9b, 0x00, 0x03, 0x52, 0xc8, //
    0x01, 0x50, 0xc8, 0x01, 0x51, 0xc8, 0x01, 0x00, 0x75, 0x95, 0x7b, 0x86, //
    0x14, 0x17, 0x3b, 0x30, 0x03, 0x00, 0x00, 0x00, 0x00, 0x04, 0x59, 0x5a,
];

/// What both of those hold.
fn six_hundred() -> Vec<u8> {
    let mut v = b"the quick brown fox jumps over the lazy dog. ".repeat(20);
    v.truncate(600);
    v
}

/// A stream a real encoder wrote in blocks, read to its symbols and to the
/// right bytes. The builder above and the reader were written from the same
/// understanding of the format, so a stream neither of them made is the only
/// thing that can say the understanding is right.
#[test]
fn the_blocks_an_encoder_wrote_read_as_what_it_packed() {
    for (name, stream) in [("without sizes", MULTI_BLOCK), ("with sizes", MULTI_BLOCK_SIZED)] {
        let (out, trace) = decode_traced(Codec::Xz, stream).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(out, six_hundred(), "{name}");
        check(&trace, stream, &out);
        assert_eq!(layout(stream).expect("lays out").blocks.len(), 3, "{name}");
        assert!(
            trace.steps().any(|s| matches!(s.kind, StepKind::Match { .. })),
            "{name}: repeating text packs to matches"
        );
    }
}

/// A stream `liblzma` wrote with delta in front of LZMA2, taken from the
/// template's own tests. The chain is what makes it unreadable here.
const DELTA_LZMA2: &[u8] = &[
    0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x01, 0x69, 0x22, 0xde, 0x36, //
    0x02, 0x01, 0x03, 0x01, 0x03, 0x21, 0x01, 0x16, 0x97, 0x8f, 0x71, 0xfc, //
    0xe0, 0x00, 0x80, 0x00, 0x39, 0x5d, 0x00, 0x3a, 0x1a, 0x08, 0xce, 0x7b, //
    0xb2, 0x93, 0xe7, 0xa9, 0x8f, 0x7d, 0xbc, 0xe6, 0x24, 0x3e, 0x91, 0xce, //
    0x05, 0x88, 0x3e, 0x8e, 0x48, 0xd1, 0x7f, 0x7e, 0xcd, 0x53, 0x0a, 0x62, //
    0xee, 0xda, 0xc7, 0x5b, 0x39, 0xb7, 0x0a, 0xef, 0x02, 0x15, 0xdc, 0x52, //
    0xb8, 0xd0, 0x26, 0xc0, 0x68, 0xa7, 0xa2, 0x7e, 0x9d, 0x4e, 0x35, 0x26, //
    0x93, 0x97, 0xb8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x78, 0xe7, 0x06, 0x27, //
    0x00, 0x01, 0x51, 0x81, 0x01, 0x00, 0x00, 0x00, 0xc8, 0xf1, 0xbd, 0x30, //
    0x3e, 0x30, 0x0d, 0x8b, 0x02, 0x00, 0x00, 0x00, 0x00, 0x01, 0x59, 0x5a,
];

/// The same text through the x86 branch filter with a start offset, and with
/// SHA-256 as the check.
const X86_AT_4096: &[u8] = &[
    0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x0a, 0xe1, 0xfb, 0x0c, 0xa1, //
    0x03, 0x01, 0x04, 0x04, 0x00, 0x10, 0x00, 0x00, 0x21, 0x01, 0x16, 0x00, //
    0x5c, 0xb1, 0xc8, 0xfb, 0xe0, 0x00, 0x80, 0x00, 0x30, 0x5d, 0x00, 0x3a, //
    0x1a, 0x08, 0xce, 0x76, 0xc7, 0xe5, 0xe9, 0xd6, 0x07, 0x34, 0xc3, 0xd1, //
    0x0e, 0xbf, 0xce, 0x55, 0xe1, 0xaa, 0xbd, 0xe0, 0xe4, 0x8f, 0x98, 0x01, //
    0xdd, 0x8d, 0xe5, 0x07, 0x54, 0x9e, 0x65, 0x25, 0x5f, 0x27, 0x3a, 0x6a, //
    0x7e, 0xb4, 0xd3, 0x49, 0x27, 0xa8, 0xf1, 0x09, 0x8c, 0x00, 0x00, 0x00, //
    0x4d, 0xb3, 0xfd, 0xe5, 0x09, 0xdf, 0xed, 0xa3, 0x10, 0xe4, 0xfb, 0x80, //
    0x7e, 0x24, 0x38, 0x20, 0xa9, 0x65, 0x1b, 0xf3, 0x5a, 0xc8, 0xf3, 0x17, //
    0x58, 0x07, 0x52, 0x4d, 0x1b, 0x90, 0xf2, 0x68, 0x00, 0x01, 0x68, 0x81, //
    0x01, 0x00, 0x00, 0x00, 0xad, 0xa7, 0xc8, 0x13, 0xb6, 0xe9, 0xdf, 0x1c, //
    0x02, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x59, 0x5a,
];

/// The same again with no start offset, which is what an encoder handed a
/// whole file writes, and CRC64 as the check.
const X86_AT_NOTHING: &[u8] = &[
    0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x04, 0xe6, 0xd6, 0xb4, 0x46, //
    0x02, 0x01, 0x04, 0x00, 0x21, 0x01, 0x16, 0x00, 0x0d, 0x86, 0x35, 0x1f, //
    0xe0, 0x00, 0x80, 0x00, 0x30, 0x5d, 0x00, 0x3a, 0x1a, 0x08, 0xce, 0x76, //
    0xc7, 0xe5, 0xe9, 0xd6, 0x07, 0x34, 0xc3, 0xd1, 0x0e, 0xbf, 0xce, 0x55, //
    0xe1, 0xaa, 0xbd, 0xe0, 0xe4, 0x8f, 0x98, 0x01, 0xdd, 0x8d, 0xe5, 0x07, //
    0x54, 0x9e, 0x65, 0x25, 0x5f, 0x27, 0x3a, 0x6a, 0x7e, 0xb4, 0xd3, 0x49, //
    0x27, 0xa8, 0xf1, 0x09, 0x8c, 0x00, 0x00, 0x00, 0x7f, 0x3d, 0xc4, 0x61, //
    0x7e, 0x20, 0x6b, 0x6f, 0x00, 0x01, 0x4c, 0x81, 0x01, 0x00, 0x00, 0x00, //
    0x8d, 0xe0, 0xf5, 0x8f, 0xb1, 0xc4, 0x67, 0xfb, 0x02, 0x00, 0x00, 0x00, //
    0x00, 0x04, 0x59, 0x5a,
];

/// A chain with a transform in it is left alone. The stream still lays out,
/// which is what keeps the block map available, but not one block of it is
/// read here.
///
/// The delta stream is why the check has to be exact. Delta does not change
/// how long the data is, so reading its LZMA2 and stopping there produces
/// something exactly as long as the index says and wrong in nearly every
/// byte: the length checks would pass and the reader would be shown a lie.
/// The second half of this test is that lie, so a chain check that stopped
/// looking at the first filter would fail here rather than ship.
#[test]
fn a_chain_with_a_filter_in_it_is_not_read_here() {
    for (name, stream) in
        [("delta", DELTA_LZMA2), ("x86 at 4096", X86_AT_4096), ("x86 at nothing", X86_AT_NOTHING)]
    {
        let l = layout(stream).unwrap_or_else(|| panic!("{name}: the stream still lays out"));
        assert_eq!(l.blocks.len(), 1, "{name}");
        assert!(!l.plain, "{name}: a chain of two filters is not one LZMA2 filter");
        // `lzma-rs` runs no filter but LZMA2 either, so what the fallback has
        // to say about these is that it cannot read them. That is the
        // behaviour this change had to keep, not one it had to fix.
        assert_eq!(decode_traced(Codec::Xz, stream).err(), Some(Refusal::Failed), "{name}");
        assert_eq!(decode(Codec::Xz, stream).err(), Some(Refusal::Failed), "{name}");
    }

    // What reading the delta stream's LZMA2 alone would have given: the right
    // number of bytes, and not the text.
    let l = layout(DELTA_LZMA2).expect("lays out");
    let b = &l.blocks[0];
    let (inner, _) = lzma::lzma2(&DELTA_LZMA2[b.at + b.header..b.at + b.header + b.packed]).expect("reads");
    assert_eq!(inner.len() as u64, b.unpacked, "delta leaves the length alone, which is the trap");
    assert!(!inner.starts_with(b"the quick"), "the LZMA2 under a delta filter is not the file");
}

/// A dictionary size code past what the encoding can spell, which is the one
/// way to reach the fallback with bytes still at the end of it.
///
/// Every filter chain this refuses is also a chain `lzma-rs` refuses, so the
/// tests above can only show the fallback failing. This one shows it working:
/// the block is plain LZMA2 except for a properties byte no encoder would
/// write, our own reading turns it down on that alone, and what comes back is
/// the crate's bytes with the block map over them. Which is what the whole
/// fallback is for, and would otherwise go untested.
#[test]
fn a_dictionary_the_format_cannot_spell_is_left_to_the_crate() {
    let data = mixed(900);
    let stream = wrap(&[(pack(&data, 3, 0, 2, 1 << 20, None), data.clone())], false);
    for code in [MAX_DICT_CODE + 1, 63] {
        // The one built block's header sits right behind the stream header:
        // its size byte, its flags, the filter, the properties byte, three
        // bytes of padding, and its own CRC32 over all of that.
        let mut bad = stream.clone();
        bad[HEADER + 4] = code;
        let sum = crc32(&bad[HEADER..HEADER + 8]);
        bad[HEADER + 8..HEADER + 12].copy_from_slice(&sum.to_le_bytes());

        assert!(!layout(&bad).expect("still lays out").plain, "code {code} is not a size");
        let (out, trace) = decode_traced(Codec::Xz, &bad).unwrap_or_else(|e| panic!("code {code}: {e:?}"));
        assert_eq!(out, data, "code {code}: the crate still reads it");
        assert_eq!(decode(Codec::Xz, &bad).expect("reads"), data, "code {code}: the two doors disagree");
        check(&trace, &bad, &out);
        // A map of the block and nothing inside it, which is what the whole
        // stream used to get.
        assert!(trace.steps().any(|s| s.kind == StepKind::Block), "code {code}: no block map");
        assert!(
            !trace.steps().any(|s| matches!(s.kind, StepKind::Literal(_))),
            "code {code}: the fallback named symbols it never read"
        );
    }
}

/// Where each block's bytes ended up in the output, which is the one question
/// a block's integrity check needs answered and no field of the file answers.
///
/// Both ways of reading a stream have to answer it the same, and the two
/// arrive at it differently: reading the blocks ourselves counts what came
/// out of each one, and falling back to the crate takes the sizes from the
/// index and adds them up. The dictionary code is what forces the second, as
/// in the test above.
#[test]
fn every_block_says_which_of_the_bytes_it_produced() {
    let parts: Vec<Vec<u8>> = (0..3).map(|i| mixed(700 + i * 300)).collect();
    let blocks: Vec<(Vec<u8>, Vec<u8>)> =
        parts.iter().map(|p| (pack(p, 3, 0, 2, 1 << 20, None), p.clone())).collect();
    let stream = wrap_checked(&blocks, false, 4);

    let (out, trace) = decode_traced(Codec::Xz, &stream).expect("reads");
    let members = trace.members().to_vec();
    assert_eq!(members.len(), parts.len(), "one member per block");
    let mut at = 0u64;
    for (i, part) in parts.iter().enumerate() {
        let m = &members[i];
        assert_eq!(m.out_bytes.start, at, "block {i} starts where block {} ended", i.wrapping_sub(1));
        assert_eq!(m.out_bytes.end - m.out_bytes.start, part.len() as u64, "block {i} is as long as it packed");
        assert_eq!(&out[m.out_bytes.start as usize..m.out_bytes.end as usize], &part[..], "block {i}");
        // And the sum over that slice is the number the file wrote after the
        // block, which is the whole of what the check comes to.
        assert_eq!(
            crate::checksum::crc64_xz(part).to_le_bytes().to_vec(),
            check_bytes(4, part),
            "block {i}: the builder and the sum disagree"
        );
        // The packed bytes a member names are inside the stream and hold the
        // block's data rather than its header.
        assert!(m.in_bits.end <= stream.len() as u64 * 8, "block {i} reads past the stream");
        assert!(m.in_bits.start >= HEADER as u64 * 8, "block {i} reads the stream header");
        at = m.out_bytes.end;
    }
    assert_eq!(at, out.len() as u64, "the blocks account for every byte of the output");

    // The same stream with a dictionary code no encoder would write, which
    // sends it to the crate whole. The blocks are the same blocks and the
    // members say so.
    let mut bad = stream.clone();
    bad[HEADER + 4] = MAX_DICT_CODE + 1;
    let sum = crc32(&bad[HEADER..HEADER + 8]);
    bad[HEADER + 8..HEADER + 12].copy_from_slice(&sum.to_le_bytes());
    let (fell_back, t2) = decode_traced(Codec::Xz, &bad).expect("the crate reads it");
    assert_eq!(fell_back, out, "the two readings are of the same bytes");
    assert!(t2.steps().any(|s| s.kind == StepKind::Block), "this stream was meant to fall back");
    assert_eq!(t2.members(), &members[..], "the fallback places the blocks where the reading did");
}

/// A stream that stops in the middle, at every length there is. Nothing may
/// panic, and what comes back is either a refusal or bytes with a trace that
/// tiles them; a guess dressed up as a reading is the one wrong answer.
#[test]
fn a_stream_that_stops_early_is_refused_rather_than_guessed_at() {
    let data = mixed(3000);
    let stream = wrap(&[(pack(&data, 3, 0, 2, 1 << 20, None), data.clone())], true);
    for cut in 0..stream.len() {
        match decode_traced(Codec::Xz, &stream[..cut]) {
            Ok((out, trace)) => {
                trace.check_tiles().unwrap_or_else(|e| panic!("cut at {cut} gave a trace that does not tile: {e}"));
                assert_eq!(trace.out_bytes(), out.len() as u64, "cut at {cut}");
            }
            Err(Refusal::Failed | Refusal::TooLarge) => {}
            Err(e) => panic!("cut at {cut} gave {e:?}"),
        }
    }
}

/// A byte of the stream changed, everywhere it can be changed. The same
/// promise: no panic, and nothing that says more than it knows.
#[test]
fn a_stream_with_a_byte_changed_says_no_more_than_it_knows() {
    let data = mixed(600);
    let stream = wrap(&[(pack(&data, 3, 0, 2, 1 << 20, None), data.clone())], true);
    for at in 0..stream.len() {
        for xor in [0x01u8, 0x80, 0xff] {
            let mut bad = stream.clone();
            bad[at] ^= xor;
            match decode_traced(Codec::Xz, &bad) {
                Ok((out, trace)) => {
                    trace
                        .check_tiles()
                        .unwrap_or_else(|e| panic!("byte {at} ^ {xor:#x} gave a trace that does not tile: {e}"));
                    assert_eq!(trace.out_bytes(), out.len() as u64, "byte {at} ^ {xor:#x}");
                }
                Err(Refusal::Failed | Refusal::TooLarge) => {}
                Err(e) => panic!("byte {at} ^ {xor:#x} gave {e:?}"),
            }
        }
    }
}

/// Bytes that are not a stream give nothing rather than a wrong shape.
#[test]
fn nonsense_is_not_a_stream() {
    assert!(layout(b"not an xz stream at all, not even close").is_none());
    assert_eq!(decode_traced(Codec::Xz, b"nope").err(), Some(Refusal::Failed));
    assert_eq!(decode(Codec::Xz, b"not compressed").err(), Some(Refusal::Failed));
}

#[test]
fn a_variable_length_integer_reads_seven_bits_a_byte() {
    let mut at = 0;
    assert_eq!(vli(&[0x00], &mut at), Some(0));
    let mut at = 0;
    assert_eq!(vli(&[0x7f], &mut at), Some(127));
    let mut at = 0;
    assert_eq!(vli(&[0x80, 0x01], &mut at), Some(128));
    // Nine bytes that never end is not a number, and neither is a number with
    // a zero byte on the end of it: there is one spelling of each.
    let mut at = 0;
    assert_eq!(vli(&[0x80; 9], &mut at), None);
    let mut at = 0;
    assert_eq!(vli(&[0x80, 0x00], &mut at), None);
    // And what this test is really for: it writes the same numbers back.
    for n in [0u64, 1, 127, 128, 300, 65_536, u64::from(u32::MAX)] {
        let bytes = vli_of(n);
        let mut at = 0;
        assert_eq!(vli(&bytes, &mut at), Some(n), "{n} came back as something else");
        assert_eq!(at, bytes.len(), "{n} was read at the wrong length");
    }
}

/// All sixteen check IDs have a length, and the blocks are measured against
/// it: a check read as the wrong length puts every field after it, and the
/// index behind them, in the wrong place. The same arithmetic the template
/// does, held to the same table.
#[test]
fn every_check_id_has_a_length() {
    for (id, want) in
        [(0u8, 0usize), (1, 4), (2, 4), (3, 4), (4, 8), (5, 8), (7, 16), (9, 16), (10, 32), (11, 32), (13, 64), (15, 64)]
    {
        assert_eq!(check_size(id), want, "check {id}");
    }
}
