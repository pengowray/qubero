//! What the LZMA decoder is held to: the bytes `lzma-rs` gives, and a trace
//! that tiles the run it read.
//!
//! The bytes are the easy half and the one that must never move. The trace is
//! the reason this decoder exists at all, so it is checked on every stream
//! decoded here rather than in a test of its own.
//!
//! Streams to check against had to be arranged for. `lzma-rs` ships an
//! encoder, but it writes literals and an end-of-stream marker and nothing
//! else, at one setting of `lc`, `lp` and `pb` and one dictionary size: a
//! stream out of it never exercises a match, a repeated distance, a one-byte
//! short rep, or a literal coded against the byte a match would have copied,
//! and between them that is most of the format. So the streams here are packed
//! by `lzma-rust2`, which takes all of those as arguments, and read back by
//! `lzma-rs`, which is the decoder this replaces.

use std::num::NonZeroU64;

use proptest::prelude::*;

use super::*;
use crate::codec::Step;

/// Pack a raw LZMA1 stream: no header in front of it, which is the shape 7z
/// stores and the shape lzip's member holds.
fn pack1(data: &[u8], lc: u32, lp: u32, pb: u32, dict: u32, marker: bool) -> (Vec<u8>, u8) {
    use lzma_rust2::{LzmaOptions, LzmaWriter};
    let mut opts = LzmaOptions::with_preset(6);
    opts.lc = lc;
    opts.lp = lp;
    opts.pb = pb;
    opts.dict_size = dict;
    let mut w = LzmaWriter::new_no_header(Vec::new(), &opts, marker).expect("options the encoder takes");
    let props = w.props();
    std::io::Write::write_all(&mut w, data).expect("packs");
    (w.finish().expect("finishes"), props)
}

/// The same data as LZMA2. `chunk` forces chunks that reset the dictionary
/// rather than one long dependent run, which is the path a stream packed in
/// parallel takes and the one with the resets in it.
fn pack2(data: &[u8], lc: u32, lp: u32, pb: u32, dict: u32, chunk: Option<u64>) -> Vec<u8> {
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

/// What `lzma-rs` makes of a raw LZMA1 stream, given the thirteen bytes an
/// `.lzma` file would have carried in front of it. All ones in the last eight
/// is how that header says the size is unknown.
fn oracle1(stream: &[u8], props: u8, dict: u32, unpacked: Option<u64>) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut header = [0xffu8; 13];
    header[0] = props;
    header[1..5].copy_from_slice(&dict.to_le_bytes());
    if let Some(n) = unpacked {
        header[5..13].copy_from_slice(&n.to_le_bytes());
    }
    let mut input = (&header[..]).chain(stream);
    let mut out = Vec::new();
    lzma_rs::lzma_decompress(&mut input, &mut out).ok()?;
    Some(out)
}

fn oracle2(stream: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    lzma_rs::lzma2_decompress(&mut &stream[..], &mut out).ok()?;
    Some(out)
}

/// Every claim a trace makes about itself, on one run: that its steps tile the
/// input and the output, that its ends are the run's own ends, that a step
/// naming a header or the properties wrote no bytes, that a literal wrote one,
/// and that a match wrote as many as it said.
fn check(trace: &Trace, data: &[u8], out: &[u8]) {
    trace.check_tiles().expect("the steps tile the run");
    assert_eq!(trace.in_bits(), data.len() as u64 * 8, "the trace stops short of the run");
    assert_eq!(trace.out_bytes(), out.len() as u64, "the trace stops short of the output");
    if trace.coarse() {
        return;
    }
    for (i, s) in trace.steps().enumerate() {
        let Step { in_bits, out_bytes, kind } = s;
        // The range coder pulls whole bytes, so no step may start or end
        // inside one. Claiming a bit would be claiming to know something the
        // format does not say; see the decoder's module doc.
        assert_eq!(in_bits.start % 8, 0, "step {i} starts inside a byte");
        assert_eq!(in_bits.end % 8, 0, "step {i} ends inside a byte");
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
                assert!(dist >= 1, "step {i} is a match of distance zero");
                assert!(u64::from(dist) <= out_bytes.start, "step {i} reaches back before the output");
                // And the bytes it claims to have copied are the bytes there.
                let at = out_bytes.start as usize;
                for k in 0..len as usize {
                    assert_eq!(out[at + k], out[at + k - dist as usize], "step {i} byte {k}");
                }
            }
            StepKind::Stored => assert!(wrote > 0, "step {i} stores nothing"),
            other => panic!("step {i} is a {other:?}, which no LZMA run should hold"),
        }
    }
    // A block's steps cover the block. The two are recorded separately, so a
    // block opened after its own header was written down holds those bytes
    // without naming them, and the listing calls bytes it holds and does not
    // name unmapped. That is invisible to the tiling check, because the steps
    // still tile the run: it is the block that is wrong, not the run.
    for (i, bl) in trace.blocks().iter().enumerate() {
        assert!(bl.steps.end > bl.steps.start, "block {i} holds no steps");
        let first = trace.step(bl.steps.start as usize).expect("a first step");
        assert_eq!(first.in_bits.start, bl.in_bits.start, "block {i} reads before its first step");
        assert_eq!(first.out_bytes.start, bl.out_bytes.start, "block {i} writes before its first step");
        let last = trace.step(bl.steps.end as usize - 1).expect("a last step");
        assert_eq!(last.in_bits.end, bl.in_bits.end, "block {i} reads past its last step");
        assert_eq!(last.out_bytes.end, bl.out_bytes.end, "block {i} writes past its last step");
    }

    // Every byte of the output came from exactly one step, and every byte of
    // the run was read by one.
    for byte in (0..out.len() as u64).step_by(7) {
        assert!(trace.map_out(byte).is_some(), "output byte {byte} came from nowhere");
    }
    for bit in (0..data.len() as u64 * 8).step_by(8 * 7) {
        assert!(trace.map_in(bit).is_some(), "input bit {bit} went nowhere");
    }
}

/// One LZMA1 stream, read both ways, against every claim above.
fn same1(data: &[u8], lc: u32, lp: u32, pb: u32, dict: u32, marker: bool) {
    let (stream, props) = pack1(data, lc, lp, pb, dict, marker);
    // How 7z states it, and how lzip does. A stream without a marker can only
    // be read with the size, so it is only asked the one way.
    let sizes: &[Option<u64>] =
        if marker { &[None, Some(data.len() as u64)] } else { &[Some(data.len() as u64)] };
    for &unpacked in sizes {
        let want = oracle1(&stream, props, dict, unpacked)
            .unwrap_or_else(|| panic!("lzma-rs will not read a stream it should: lc={lc} lp={lp} pb={pb}"));
        assert_eq!(want, data, "the encoder and lzma-rs disagree, so nothing here is being tested");
        let (out, trace) = lzma1(&stream, props, dict, unpacked)
            .unwrap_or_else(|e| panic!("refused lc={lc} lp={lp} pb={pb} dict={dict} marker={marker}: {e:?}"));
        assert_eq!(out, want, "lc={lc} lp={lp} pb={pb} dict={dict} marker={marker} unpacked={unpacked:?}");
        check(&trace, &stream, &out);
    }
}

/// One LZMA2 stream, the same way.
fn same2(data: &[u8], lc: u32, lp: u32, pb: u32, dict: u32, chunk: Option<u64>) {
    let stream = pack2(data, lc, lp, pb, dict, chunk);
    let want = oracle2(&stream).expect("lzma-rs reads it");
    assert_eq!(want, data, "the encoder and lzma-rs disagree, so nothing here is being tested");
    let (out, trace) = lzma2(&stream).unwrap_or_else(|e| panic!("refused lc={lc} lp={lp} pb={pb}: {e:?}"));
    assert_eq!(out, want, "lc={lc} lp={lp} pb={pb} dict={dict} chunk={chunk:?}");
    check(&trace, &stream, &out);
}

/// Bytes with no pattern for an encoder to find, so a stream of them is nearly
/// as long as they are. What forces LZMA2 to write more than one chunk: the
/// encoder flushes when its 64 KiB of packed output fills, and only data that
/// does not pack ever fills it. It is also what makes the encoder give up and
/// store a chunk outright, which is the other chunk type there is.
fn noise(n: usize) -> Vec<u8> {
    let mut x = 0x2545_f491_4f6c_dd1du64;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 33) as u8
        })
        .collect()
}

/// Data with something in it to match against: runs, a repeating phrase, and a
/// stretch that does not repeat at all, so one stream holds literals, matches,
/// repeated distances and short reps together.
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

#[test]
fn a_stream_reads_as_the_bytes_lzma_rs_gives_at_every_setting() {
    let data = mixed(9000);
    // `lc + lp` past four is legal in LZMA1 and never written by anything, so
    // it is walked here and nowhere else.
    for (lc, lp, pb) in [(3, 0, 2), (0, 0, 0), (0, 2, 0), (1, 1, 1), (4, 0, 4), (8, 0, 2), (2, 2, 2), (0, 4, 4)] {
        for dict in [1 << 12, 1 << 16, 1 << 20] {
            for marker in [true, false] {
                same1(&data, lc, lp, pb, dict, marker);
            }
        }
    }
}

#[test]
fn nothing_at_all_and_one_byte_are_streams_too() {
    for data in [vec![], vec![0u8], vec![b'x'; 2], vec![0xffu8; 300]] {
        for marker in [true, false] {
            same1(&data, 3, 0, 2, 1 << 16, marker);
        }
        same2(&data, 3, 0, 2, 1 << 16, None);
    }
}

/// A match longer than its distance, which is how a run of one byte is
/// written and the one case a block copy would get wrong.
#[test]
fn a_match_may_be_longer_than_the_distance_it_reads_from() {
    let mut data = vec![b'a'];
    data.extend(std::iter::repeat_n(b'a', 5000));
    data.extend_from_slice(b"ab");
    data.extend(std::iter::repeat_n(b'b', 400));
    same1(&data, 3, 0, 2, 1 << 16, true);
    same2(&data, 3, 0, 2, 1 << 16, None);

    // And the trace says so: some match copies more bytes than it reaches back.
    let (stream, props) = pack1(&data, 3, 0, 2, 1 << 16, true);
    let (_, trace) = lzma1(&stream, props, 1 << 16, None).expect("reads");
    assert!(
        trace.steps().any(|s| matches!(s.kind, StepKind::Match { len, dist } if len > dist)),
        "a run of one byte was not written as an overlapping match"
    );
}

/// `lc + lp` no greater than four, which is the one thing LZMA2 asks for that
/// LZMA1 does not. An encoder handed more writes a properties byte no LZMA2
/// decoder will read, so the pairs are drawn rather than crossed.
const LZMA2_SETTINGS: [(u32, u32, u32); 6] =
    [(3, 0, 2), (0, 0, 0), (1, 1, 1), (0, 4, 4), (4, 0, 0), (2, 2, 3)];

#[test]
fn an_lzma2_stream_reads_as_the_bytes_lzma_rs_gives() {
    let data = mixed(200_000);
    for (lc, lp, pb) in LZMA2_SETTINGS {
        same2(&data, lc, lp, pb, 1 << 16, None);
        same2(&data, lc, lp, pb, 1 << 12, Some(1 << 12));
    }
}

/// Two packed chunks in one stream, with the second carrying on the first's
/// state and dictionary rather than starting over. What a stream longer than
/// the two megabytes a chunk can name has to do, and the case where getting
/// the chunk boundary wrong shows up as wrong bytes rather than as a refusal.
#[test]
fn a_stream_past_what_one_chunk_can_name_carries_its_state_into_the_next() {
    let data = mixed(2_200_000);
    let stream = pack2(&data, 3, 0, 2, 1 << 16, None);
    let (out, trace) = lzma2(&stream).expect("reads");
    assert_eq!(out, data);
    check(&trace, &stream, &out);
    assert!(chunks(&trace) > 1, "2.2 MB came out as one chunk, so nothing here is being tested");
    // The second chunk resets nothing, so it holds no properties of its own:
    // its properties step is the width of what it carries, which is nothing.
    let props: Vec<_> = trace
        .steps()
        .filter(|s| matches!(s.kind, StepKind::Header(StepField::LzmaProps, _)))
        .collect();
    assert!(props.len() > 1);
    assert!(props[0].in_bits.end > props[0].in_bits.start, "the first chunk carries its properties");
    assert!(props[1].in_bits.is_empty(), "the second chunk claims properties it does not hold");
}

/// Bytes that will not pack, which is what makes an encoder store a chunk
/// outright and reset the dictionary partway through. The stored path, the
/// dictionary reset, and the two chunk types side by side in one stream.
#[test]
fn a_stream_that_will_not_pack_holds_stored_chunks_and_a_reset() {
    let data = noise(200_000);
    let stream = pack2(&data, 3, 0, 2, 1 << 12, Some(1 << 12));
    let (out, trace) = lzma2(&stream).expect("reads");
    assert_eq!(out, data);
    check(&trace, &stream, &out);
    assert!(chunks(&trace) > 1, "200 KB of noise came out as one chunk");
    let stored: Vec<_> = trace.steps().filter(|s| s.kind == StepKind::Stored).collect();
    assert!(!stored.is_empty(), "nothing was stored, so the stored path is untested");
    // A stored chunk's bytes come out as they went in, one for one.
    for s in &stored {
        let took = (s.in_bits.end - s.in_bits.start) / 8;
        assert_eq!(took, s.out_bytes.end - s.out_bytes.start, "a stored chunk changed length");
        let at = s.in_bits.start as usize / 8;
        let n = took as usize;
        assert_eq!(&stream[at..at + n], &out[s.out_bytes.start as usize..s.out_bytes.end as usize]);
    }
}

/// An LZMA chunk that starts over in the middle of a stream: new properties, a
/// fresh state, and a dictionary the matches after it may not reach back past.
///
/// Spliced rather than coaxed out of the encoder, which only writes one where
/// its buffer happens to fill. Two streams run together are one stream, since
/// every LZMA2 stream begins by resetting everything and ends with a zero byte.
#[test]
fn a_chunk_may_start_the_dictionary_over_in_the_middle_of_a_stream() {
    let first = mixed(30_000);
    let second: Vec<u8> = mixed(40_000).iter().rev().copied().collect();
    let mut stream = pack2(&first, 3, 0, 2, 1 << 16, None);
    assert_eq!(stream.pop(), Some(0x00), "an LZMA2 stream ends with a zero byte");
    stream.extend_from_slice(&pack2(&second, 0, 2, 1, 1 << 16, None));

    let want: Vec<u8> = first.iter().chain(second.iter()).copied().collect();
    assert_eq!(oracle2(&stream).expect("lzma-rs reads the splice"), want);
    let (out, trace) = lzma2(&stream).expect("reads");
    assert_eq!(out, want);
    check(&trace, &stream, &out);
    assert_eq!(chunks(&trace), 2);
    // Both chunks carry their own properties, and the two differ.
    let props: Vec<_> = trace
        .steps()
        .filter_map(|s| match s.kind {
            StepKind::Header(StepField::LzmaProps, v) => Some((v, s.in_bits.end - s.in_bits.start)),
            _ => None,
        })
        .collect();
    assert_eq!(props.len(), 2);
    assert_eq!(props[0], (0x5d, 8), "the first chunk holds one byte of properties");
    assert_eq!(props[1], (0 + 9 * (2 + 5 * 1), 8), "the second chunk holds its own");
    // Nothing in the second half reaches back into the first, because the
    // dictionary was reset between them.
    let split = first.len() as u64;
    for s in trace.steps() {
        if let StepKind::Match { dist, .. } = s.kind {
            if s.out_bytes.start >= split {
                assert!(u64::from(dist) <= s.out_bytes.start - split, "a match read across a dictionary reset");
            }
        }
    }
}

/// How many chunks a trace says a stream held.
fn chunks(trace: &Trace) -> usize {
    let n = trace.steps().filter(|s| matches!(s.kind, StepKind::Header(StepField::BlockHeader, _))).count();
    assert_eq!(trace.blocks().len(), n, "a chunk that is not a block");
    let total: u64 = trace.blocks().iter().map(|bl| bl.out_bytes.end - bl.out_bytes.start).sum();
    assert_eq!(total, trace.out_bytes(), "the blocks do not cover the output");
    n
}

/// A stream that says how much comes out, and no marker at the end of it,
/// which is what 7z writes and what lzip does not.
#[test]
fn a_stream_with_a_size_and_no_marker_stops_where_the_size_says() {
    let data = mixed(4000);
    let (stream, props) = pack1(&data, 3, 0, 2, 1 << 16, false);
    let (out, trace) = lzma1(&stream, props, 1 << 16, Some(data.len() as u64)).expect("reads");
    assert_eq!(out, data);
    check(&trace, &stream, &out);
    assert!(!trace.steps().any(|s| s.kind == StepKind::EndOfBlock), "a marker in a stream without one");

    // Asked to read the same stream to a marker it does not have, it runs out
    // of input and says so rather than returning what it managed.
    assert!(lzma1(&stream, props, 1 << 16, None).is_err(), "a stream with no marker read to one");
    // And asked for more than the stream holds.
    assert!(lzma1(&stream, props, 1 << 16, Some(data.len() as u64 + 1000)).is_err());
}

/// The one thing the trace is for: a byte of the output names the input it was
/// being read out of, and that input is inside the run.
#[test]
fn a_byte_of_the_output_names_input_that_is_really_there() {
    let data = mixed(3000);
    let (stream, props) = pack1(&data, 3, 0, 2, 1 << 16, true);
    let (out, trace) = lzma1(&stream, props, 1 << 16, None).expect("reads");
    let mut zero_width = 0;
    for byte in 0..out.len() as u64 {
        let step = trace.map_out(byte).expect("every byte comes from a step");
        assert!(step.in_bits.end <= stream.len() as u64 * 8, "byte {byte} points past the run");
        assert!(step.out_bytes.contains(&byte));
        if step.in_bits.is_empty() {
            zero_width += 1;
        }
    }
    // Most symbols pull nothing. That is the format, not a fault, and a test
    // that did not see it would mean the attribution had quietly become a
    // guess spread evenly over the input.
    assert!(zero_width > 0, "every symbol pulled input, which no range coder does");
}

/// A stream with more symbols in it than the trace may name keeps the map and
/// says it stopped naming them.
#[test]
fn too_many_symbols_coarsens_rather_than_filling_memory() {
    let data = mixed(400_000);
    let (stream, props) = pack1(&data, 3, 0, 2, 1 << 20, true);
    let (out, trace) = lzma1(&stream, props, 1 << 20, None).expect("reads");
    assert_eq!(out, data);
    assert!(!trace.coarse(), "the real budget is not reached by a stream this size");

    // The same stream against a budget it does reach. Not the real one, which
    // needs a hundred megabytes of input; the path is the same path.
    let (coarse_out, coarse) = lzma1_within(&stream, props, 1 << 20, None, 40).expect("reads");
    assert_eq!(coarse_out, out, "coarsening changed the bytes");
    coarse.check_tiles().expect("a coarse trace still tiles");
    assert!(coarse.coarse(), "the budget of 40 was not reached");
    assert!(coarse.len() < trace.len(), "coarsening kept as many steps as naming them");
    assert!(coarse.steps().any(|s| s.kind == StepKind::Opaque));
    assert!(!coarse.steps().any(|s| matches!(s.kind, StepKind::Literal(_))));
    for byte in (0..out.len() as u64).step_by(997) {
        assert!(coarse.map_out(byte).is_some(), "byte {byte} came from nowhere");
    }
}

/// The same across chunk boundaries, which is the case a single stream cannot
/// reach: once the budget is spent, every later chunk starts already coarse and
/// records one step for all of its symbols. That is the path where the trace
/// could stop tiling, because the giving-up happens in the middle of one chunk
/// and the chunks after it never begin naming anything.
#[test]
fn coarsening_holds_across_a_chunk_boundary() {
    let data = mixed(2_200_000);
    let stream = pack2(&data, 3, 0, 2, 1 << 16, None);
    let (out, trace) = lzma2(&stream).expect("reads");
    assert!(chunks(&trace) > 1, "one chunk, so no boundary is being crossed");

    for budget in [1, 4, 40, 5000] {
        let (coarse_out, coarse) = lzma2_within(&stream, budget).expect("reads");
        assert_eq!(coarse_out, out, "a budget of {budget} changed the bytes");
        coarse.check_tiles().unwrap_or_else(|e| panic!("a budget of {budget} stopped tiling: {e}"));
        assert!(coarse.coarse(), "a budget of {budget} was not reached");
        assert!(coarse.len() < trace.len(), "a budget of {budget} named as much as naming it all");
        assert!(!coarse.steps().any(|s| matches!(s.kind, StepKind::Literal(_))));
        // The chunk headers survive: what is lost is the symbols inside them.
        assert_eq!(chunks(&coarse), chunks(&trace), "a budget of {budget} lost a chunk");
        for byte in (0..out.len() as u64).step_by(9973) {
            assert!(coarse.map_out(byte).is_some(), "byte {byte} came from nowhere");
        }
    }
}

/// Bytes that are not a stream are refused, and nothing panics. This reads
/// files nobody vouched for: a broken member must not take the listing down.
#[test]
fn broken_streams_are_refused_rather_than_panicking() {
    assert_eq!(lzip(b"not an lzip member at all, no").err(), Some(Refusal::Failed));
    // A member of the right shape whose version is one nothing here reads.
    let mut short = b"LZIP\x00\x17".to_vec();
    short.extend_from_slice(&[0; 20]);
    assert_eq!(lzip(&short).err(), Some(Refusal::Failed));
    // Too short to hold a header and a trailer, let alone a stream.
    assert_eq!(lzip(b"LZIP\x01\x17").err(), Some(Refusal::Failed));

    // Properties no byte may spell, and an LZMA2 stream that never starts.
    assert_eq!(lzma1(b"\x00\x00\x00\x00\x00abc", 225, 1 << 16, None).err(), Some(Refusal::Failed));
    for case in [&b""[..], &[0x03][..], &[0x80][..], &[0x80, 0, 0, 0, 0][..], &[0xff; 40][..], &[0x02, 0xff, 0xff][..]] {
        assert!(lzma2(case).is_err(), "{case:?} was read as a stream");
    }

    // A real stream cut short at every length, and corrupted a byte at a time.
    // Some of these are still streams, of something else; what matters is that
    // none is a panic and that whatever does read still tiles.
    let data = mixed(2000);
    let (stream, props) = pack1(&data, 3, 0, 2, 1 << 16, true);
    for n in 0..stream.len() {
        if let Ok((out, trace)) = lzma1(&stream[..n], props, 1 << 16, None) {
            trace.check_tiles().unwrap_or_else(|e| panic!("cut to {n}: {e}"));
            assert_eq!(trace.out_bytes(), out.len() as u64);
        }
    }
    for i in (0..stream.len()).step_by(3) {
        for xor in [0x01u8, 0x40, 0xff] {
            let mut bad = stream.clone();
            bad[i] ^= xor;
            if let Ok((out, trace)) = lzma1(&bad, props, 1 << 16, None) {
                trace.check_tiles().unwrap_or_else(|e| panic!("byte {i} ^ {xor:#x}: {e}"));
                assert_eq!(trace.out_bytes(), out.len() as u64);
            }
        }
    }
    let packed2 = pack2(&data, 3, 0, 2, 1 << 12, Some(1 << 12));
    for i in (0..packed2.len()).step_by(3) {
        for xor in [0x01u8, 0x80, 0xff] {
            let mut bad = packed2.clone();
            bad[i] ^= xor;
            if let Ok((out, trace)) = lzma2(&bad) {
                trace.check_tiles().unwrap_or_else(|e| panic!("lzma2 byte {i} ^ {xor:#x}: {e}"));
                assert_eq!(trace.out_bytes(), out.len() as u64);
            }
        }
    }
}

/// A match that reaches back before the start of the output is not a match.
/// Reached by hand, since no encoder writes one: the first symbol of a stream
/// cannot be a match, so a stream saying it is has to be refused rather than
/// read out of whatever was in memory.
#[test]
fn a_match_reaching_before_the_output_is_refused() {
    // The properties, five bytes to prime the coder, and then bits that will
    // be read as something. What is checked is that nothing panics and nothing
    // comes out claiming to have copied bytes that were never written.
    for seed in 0..64u8 {
        let mut stream = vec![0u8, 0, 0, 0, 0];
        stream.extend((0..64u8).map(|i| i.wrapping_mul(seed).wrapping_add(seed)));
        if let Ok((out, trace)) = lzma1(&stream, 0x5d, 1 << 16, None) {
            check(&trace, &stream, &out);
        }
    }
}

#[test]
fn the_top_three_bits_take_sixteenths_off_the_power_below_them() {
    // 8 MiB with nothing off it, which is what lzip's default level packs
    // with, and the byte every sample here carries.
    assert_eq!(dict_size(0x17), Some(1 << 23));
    // The same power with seven sixteenths off, which lands below the
    // power beneath it.
    assert_eq!(dict_size(0xf7), Some((1 << 23) - (1 << 23) / 16 * 7));
    // The smallest size, whose top bits mean nothing.
    assert_eq!(dict_size(0xec), Some(1 << 12));
    // Powers the format does not have.
    assert_eq!(dict_size(0x0b), None);
    assert_eq!(dict_size(0x1e), None);
}

#[test]
fn the_properties_byte_spells_three_numbers_and_refuses_to_spell_more() {
    assert_eq!(Props::new(0x5d, false), Ok(Props { lc: 3, lp: 0, pb: 2 }));
    assert_eq!(Props::new(0, false), Ok(Props { lc: 0, lp: 0, pb: 0 }));
    assert_eq!(Props::new(224, false), Ok(Props { lc: 8, lp: 4, pb: 4 }));
    // Past what the arithmetic can reach.
    assert_eq!(Props::new(225, false), Err(Refusal::Failed));
    // LZMA2 asks for `lc + lp` no greater than four; LZMA1 does not.
    assert!(Props::new(224, true).is_err());
    assert!(Props::new(0x5d, true).is_ok());
}

proptest! {
    // Each case packs and decodes a stream twice over, so the count is kept
    // low enough that the suite stays a suite.
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Bytes with no shape to them, which is the case with the fewest matches
    /// and the most literals.
    #[test]
    fn random_bytes_read_the_same_either_way(
        data in proptest::collection::vec(any::<u8>(), 0..4000),
        lc in 0u32..9,
        lp in 0u32..5,
        pb in 0u32..5,
        which in 0usize..LZMA2_SETTINGS.len(),
    ) {
        // LZMA1 takes any of the 225 the byte can spell; LZMA2 takes only the
        // ones whose `lc + lp` is no more than four.
        same1(&data, lc, lp, pb, 1 << 16, true);
        same1(&data, lc, lp, pb, 1 << 16, false);
        let (lc, lp, pb) = LZMA2_SETTINGS[which];
        same2(&data, lc, lp, pb, 1 << 16, None);
    }

    /// Bytes drawn from a small alphabet, which is the case with the most
    /// matches: long runs, near misses, and distances that repeat.
    #[test]
    fn repetitive_bytes_read_the_same_either_way(
        parts in proptest::collection::vec(0usize..6, 1..300),
        which in 0usize..LZMA2_SETTINGS.len(),
    ) {
        let (lc, lp, pb) = LZMA2_SETTINGS[which];
        let pieces: [&[u8]; 6] = [b"abcabc", b"aaaaaaaaaa", b"the quick brown fox ", b"\x00\x00\x00\x00", b"z", b"0123456789abcdef"];
        let data: Vec<u8> = parts.iter().flat_map(|&i| pieces[i].iter().copied()).collect();
        same1(&data, lc, lp, pb, 1 << 12, true);
        same2(&data, lc, lp, pb, 1 << 12, Some(1 << 12));
    }

    /// A dictionary small enough that the encoder cannot reach back over the
    /// whole input, so distances stay near and the low slots get used.
    #[test]
    fn a_small_dictionary_reads_the_same_either_way(
        data in proptest::collection::vec(0u8..4, 0..6000),
        pb in 0u32..5,
    ) {
        same1(&data, 3, 0, pb, 1 << 12, true);
        same2(&data, 3, 0, pb, 1 << 12, Some(1 << 12));
    }

    /// Bytes that are not a stream, which must be refused rather than read,
    /// and must never panic or run on for ever.
    #[test]
    fn nonsense_is_refused_without_panicking(data in proptest::collection::vec(any::<u8>(), 0..600)) {
        if let Ok((out, trace)) = lzma1(&data, 0x5d, 1 << 16, None) {
            check(&trace, &data, &out);
        }
        if let Ok((out, trace)) = lzma2(&data) {
            check(&trace, &data, &out);
        }
        let mut member = b"LZIP\x01\x17".to_vec();
        member.extend_from_slice(&data);
        member.extend_from_slice(&[0; 20]);
        if let Ok((out, trace)) = lzip(&member) {
            check(&trace, &member, &out);
        }
    }
}
