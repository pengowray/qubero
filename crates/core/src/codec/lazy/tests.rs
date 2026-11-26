//! Tests for runs read a piece at a time: reads and traces checked against the
//! whole decoders', refusals at each limit, and the pass done in steps.

use super::deflate::Phase;
use super::seekable::lock;
use super::*;
use crate::checksum::{adler32, crc32};
use crate::codec::inflate;
use crate::codec::{BlockKind, Step, StepField, StepKind};

/// Small enough that a few hundred kilobytes reach every limit: the run is
/// indexed past 64 KiB, checkpoints are 8 KiB apart until there are more
/// than 12 of them, and the cache holds 40 KiB.
fn small() -> Limits {
    Limits { whole: 64 << 10, lazy: 4 << 20, spacing: 8 << 10, checkpoints: 12, span_cache: 40 << 10, span_steps: 1_000_000, whole_steps: crate::codec::MAX_STEPS, block_marks: MAX_BLOCK_MARKS, span: 64 << 10, measure: crate::codec::CAP_BYTES as u64 }
}

fn rand_from(mut seed: u64) -> impl FnMut() -> u64 {
    move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    }
}

/// Words, runs of one byte and noise, in stretches, so that an encoder
/// writes literals, short and long matches, and blocks of every kind.
pub(crate) fn mixed(len: usize, seed: u64) -> Vec<u8> {
    let mut rand = rand_from(seed);
    let words = ["deflate ", "window ", "checkpoint ", "span ", "the ", "of ", "Rubin ", "FITS "];
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        match rand() % 4 {
            0 => {
                for _ in 0..rand() % 400 {
                    out.extend_from_slice(words[(rand() % 8) as usize].as_bytes());
                }
            }
            1 => out.extend(std::iter::repeat_n((rand() % 256) as u8, (rand() % 5000) as usize)),
            2 => out.extend((0..rand() % 3000).map(|_| rand() as u8)),
            _ => out.extend((0..rand() % 2000).map(|i| (i / 7) as u8)),
        }
    }
    out.truncate(len);
    out
}

fn gzip_of(data: &[u8], level: u8) -> Vec<u8> {
    gzip_around(data, &miniz_oxide::deflate::compress_to_vec(data, level))
}

fn gzip_around(data: &[u8], raw: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 3];
    out.extend_from_slice(b"name.fits\0");
    out.extend_from_slice(raw);
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

/// Raw deflate flushed every `every` bytes of input, as a writer that
/// flushes each line does: each flush ends a block and adds an empty
/// stored one, so a few hundred kilobytes come to hundreds of blocks.
fn flushed(data: &[u8], level: u8, every: usize) -> Vec<u8> {
    use miniz_oxide::deflate::core::{compress, create_comp_flags_from_zip_params, CompressorOxide, TDEFLFlush};
    let mut c = CompressorOxide::new(create_comp_flags_from_zip_params(level as i32, -15, 0));
    let mut out = Vec::new();
    let mut buf = vec![0u8; every * 2 + 4096];
    let chunks: Vec<&[u8]> = data.chunks(every).collect();
    for (k, chunk) in chunks.iter().enumerate() {
        let flush = if k + 1 == chunks.len() { TDEFLFlush::Finish } else { TDEFLFlush::Sync };
        let mut input = *chunk;
        loop {
            let (_, read, wrote) = compress(&mut c, input, &mut buf, flush);
            out.extend_from_slice(&buf[..wrote]);
            input = &input[read..];
            if input.is_empty() && wrote < buf.len() {
                break;
            }
        }
    }
    out
}

/// Raw deflate of `data` with a block ended every `chunk` bytes, by a
/// sync flush, which also writes an empty stored block. miniz writes 50 KB
/// as one block, and a pass under the cap only stops between blocks.
fn deflate_in_blocks(data: &[u8], chunk: usize) -> Vec<u8> {
    use miniz_oxide::deflate::core::{compress, create_comp_flags_from_zip_params, CompressorOxide, TDEFLFlush, TDEFLStatus};
    let mut c = CompressorOxide::new(create_comp_flags_from_zip_params(6, -15, 0));
    let mut buf = vec![0u8; 4 * chunk + 1024];
    let mut out = Vec::new();
    let pieces: Vec<&[u8]> = data.chunks(chunk).collect();
    for (i, piece) in pieces.iter().enumerate() {
        let flush = if i + 1 == pieces.len() { TDEFLFlush::Finish } else { TDEFLFlush::Sync };
        let (status, read, written) = compress(&mut c, piece, &mut buf, flush);
        assert!(read == piece.len() && matches!(status, TDEFLStatus::Okay | TDEFLStatus::Done), "{status:?}");
        out.extend_from_slice(&buf[..written]);
    }
    out
}

/// `raw` with the zlib or gzip wrapper round it, `data` being what it
/// inflates to.
fn wrapped(codec: Codec, raw: &[u8], data: &[u8]) -> Vec<u8> {
    match codec {
        Codec::Zlib => [&[0x78, 0x9c][..], raw, &adler32(data).to_be_bytes()].concat(),
        Codec::Gzip => {
            let head = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
            [&head[..], raw, &crc32(data).to_le_bytes(), &(data.len() as u32).to_le_bytes()].concat()
        }
        _ => raw.to_vec(),
    }
}

fn indexed(codec: Codec, packed: &[u8], limits: Limits) -> Seekable {
    match decode_or_index_with(codec, Arc::new(packed.to_vec()), limits) {
        Ok(Decoded::Indexed(s)) => s,
        Ok(Decoded::Whole(bytes, _)) => panic!("{} bytes came back whole", bytes.len()),
        Err(why) => panic!("refused: {why:?}"),
    }
}

/// Every encoder level and every wrapper, read back a piece at a time:
/// the whole output in one read, and reads of every length across the
/// span boundaries, in an order that keeps the cache turning over.
#[test]
fn a_large_run_reads_back_as_what_went_in_from_anywhere() {
    let data = mixed(300_000, 7);
    for level in [0u8, 1, 6, 9] {
        let raw = miniz_oxide::deflate::compress_to_vec(&data, level);
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, level);
        let gzip = gzip_of(&data, level);
        for (codec, packed) in [(Codec::Deflate, &raw), (Codec::Zlib, &zlib), (Codec::Gzip, &gzip)] {
            let s = indexed(codec, packed, small());
            assert_eq!(s.len(), data.len() as u64);
            assert_eq!(s.crc32(), Some(crc32(&data)));
            assert_eq!(s.adler32(), Some(adler32(&data)));
            let mut all = vec![0u8; data.len()];
            s.read(0, &mut all).expect("reads");
            assert!(all == data, "{codec:?} level {level}: the whole output differs");
            let mut rand = rand_from(level as u64 + 99);
            for _ in 0..80 {
                let at = rand() % data.len() as u64;
                let n = (rand() % 30_000).min(data.len() as u64 - at) as usize;
                let mut got = vec![0u8; n];
                s.read(at, &mut got).expect("reads");
                assert!(got == data[at as usize..at as usize + n], "{codec:?} level {level}: {n} bytes at {at} differ");
            }
            let peak = lock(&s.spans).peak;
            let largest = (0..s.spans()).map(|k| s.span_bytes(k)).map(|r| r.end - r.start).max().unwrap() as usize;
            assert!(peak <= small().span_cache + largest, "{codec:?} level {level}: the cache held {peak} bytes");
        }
    }
}

/// Past the most checkpoints there may be, every second one goes, and the
/// first stays: it is the only place span 0 can start from.
#[test]
fn checkpoints_thin_out_and_the_first_stays() {
    let data = mixed(600_000, 3);
    let s = indexed(Codec::Deflate, &miniz_oxide::deflate::compress_to_vec(&data, 6), small());
    let marks = s.marks();
    assert!(marks.len() <= small().checkpoints, "{} checkpoints", marks.len());
    assert_eq!(marks[0], (0, 0, None));
    assert!(marks.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 <= w[1].1));
    // Thinned at least once: 600 KB at 8 KiB apart is far more than 12.
    assert!(marks.windows(2).any(|w| w[1].1 - w[0].1 >= 16 << 10));
    // And some of them are inside a block, which is the case that reads
    // a block's tables again.
    assert!(marks.iter().any(|m| m.2.is_some()), "no checkpoint inside a block");
}

/// A run under the cap comes back whole, with the same bytes and the same
/// trace the whole decoders give, step for step.
#[test]
fn a_run_under_the_cap_comes_back_as_it_always_did() {
    let data = mixed(50_000, 5);
    let raw = miniz_oxide::deflate::compress_to_vec(&data, 6);
    let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
    let gzip = gzip_of(&data, 6);
    for (codec, packed) in [(Codec::Deflate, &raw), (Codec::Zlib, &zlib), (Codec::Gzip, &gzip)] {
        let (bytes, trace) = crate::codec::decode_traced(codec, packed).expect("decodes");
        let Ok(Decoded::Whole(ours, t)) = decode_or_index_with(codec, Arc::new(packed.clone()), small()) else {
            panic!("{codec:?} did not come back whole");
        };
        assert_eq!(ours, bytes);
        assert_eq!(t.len(), trace.len());
        assert!(t.steps().zip(trace.steps()).all(|(a, b)| a == b), "{codec:?}: the steps differ");
        assert_eq!(t.blocks(), trace.blocks());
        assert_eq!((t.in_bits(), t.out_bytes()), (trace.in_bits(), trace.out_bytes()));
    }
}

/// Each span's trace tiles what its decode read and wrote, and every byte
/// and bit maps to the same step through the spans as through the trace
/// of the whole run.
#[test]
fn span_traces_agree_with_the_whole_trace() {
    let data = mixed(300_000, 11);
    for (codec, packed) in [
        (Codec::Deflate, miniz_oxide::deflate::compress_to_vec(&data, 6)),
        (Codec::Gzip, gzip_of(&data, 9)),
        (Codec::Zlib, miniz_oxide::deflate::compress_to_vec_zlib(&data, 1)),
    ] {
        let (_, whole) = crate::codec::decode_traced(codec, &packed).expect("decodes whole");
        assert!(!whole.coarse());
        let s = indexed(codec, &packed, small());
        for k in 0..s.spans() {
            let t = s.span_trace(k).expect("traces");
            t.check_tiles().unwrap_or_else(|e| panic!("{codec:?} span {k}: {e}"));
            assert_eq!(t.start().1, s.span_bytes(k).start);
            assert_eq!(t.out_bytes(), s.span_bytes(k).end);
        }
        // In order, so that the one span trace kept is the one asked for
        // next nearly every time.
        let mut rand = rand_from(17);
        let mut bytes: Vec<u64> = (0..2000).map(|_| rand() % data.len() as u64).collect();
        let mut bits: Vec<u64> = (0..2000).map(|_| rand() % (packed.len() as u64 * 8)).collect();
        bytes.sort_unstable();
        bits.sort_unstable();
        for byte in bytes {
            assert_eq!(s.map_out(byte), whole.map_out(byte), "{codec:?}: byte {byte}");
        }
        for bit in bits {
            assert_eq!(s.map_in(bit), whole.map_in(bit), "{codec:?}: bit {bit}");
        }
        // Every checkpoint's own bit, and the bits either side, which is
        // where a span's trace and the one before it meet.
        for (in_bit, _, block) in s.marks() {
            for bit in [in_bit.saturating_sub(1), in_bit, in_bit + 1].into_iter().chain(block) {
                assert_eq!(s.map_in(bit), whole.map_in(bit), "{codec:?}: bit {bit} by checkpoint {in_bit}");
            }
        }
    }
}

/// The sidebar's take on one code, rebuilt from a span's trace, says what
/// it says when rebuilt from the whole trace.
#[test]
fn a_code_taken_apart_through_a_span_reads_as_through_the_whole() {
    let data = mixed(300_000, 13);
    let packed = miniz_oxide::deflate::compress_to_vec(&data, 6);
    let (_, whole) = crate::codec::decode_traced(Codec::Deflate, &packed).expect("decodes whole");
    let s = indexed(Codec::Deflate, &packed, small());
    let mut rand = rand_from(23);
    let mut bits: Vec<u64> = (0..3000).map(|_| rand() % (packed.len() as u64 * 8)).collect();
    bits.sort_unstable();
    let mut compared = 0;
    for bit in bits {
        let t = s.trace_at_bit(bit).expect("a span names it");
        let (Some(ours), Some(theirs)) = (t.index_in(bit), whole.index_in(bit)) else { continue };
        let a = inflate::decode_step(&packed, &t, ours);
        let b = inflate::decode_step(&packed, &whole, theirs);
        // The block number and the step that declared a code length are
        // counted in the trace they came from, and the two traces start
        // in different places. The rest, the step's place in its block
        // included, is the same code read the same way.
        let same = |d: inflate::DecodedStep| {
            let plain = |c: inflate::DecodedCode| inflate::DecodedCode { entry: None, ..c };
            (plain(d.symbol), d.distance.map(plain), d.block_kind)
        };
        assert_eq!(a.map(same), b.map(same), "bit {bit}");
        compared += a.is_some() as usize;
    }
    assert!(compared > 500, "only {compared} codes compared");
}

/// The blocks found on the index pass are the blocks of the whole trace,
/// one for one: where each starts and ends in the run and in the output,
/// how it is coded and whether it is the last. And each block's own trace
/// holds the same steps as the whole trace does over that block.
#[test]
fn the_blocks_of_an_index_are_the_blocks_of_the_whole_trace() {
    let data = mixed(300_000, 29);
    // Level 0 writes stored blocks, so checkpoints land at block starts;
    // the others put them inside Huffman-coded blocks. Flushing makes
    // hundreds of blocks of every kind.
    let mut most = 0;
    for level in [0u8, 1, 6] {
        for (codec, packed) in [
            (Codec::Deflate, miniz_oxide::deflate::compress_to_vec(&data, level)),
            (Codec::Zlib, miniz_oxide::deflate::compress_to_vec_zlib(&data, level)),
            (Codec::Deflate, flushed(&data, level, 3000)),
            (Codec::Gzip, gzip_around(&data, &flushed(&data, level, 5000))),
        ] {
            let (_, whole) = crate::codec::decode_traced(codec, &packed).expect("decodes whole");
            let s = indexed(codec, &packed, small());
            assert_eq!(s.blocks(), whole.blocks().len() as u64, "{codec:?} level {level}");
            most = most.max(s.blocks());
            for (i, want) in whole.blocks().iter().enumerate() {
                let got = s.block(i as u64).expect("a block");
                assert_eq!(
                    (got.kind, got.last, &got.in_bits, &got.out_bytes),
                    (want.kind, want.last, &want.in_bits, &want.out_bytes),
                    "{codec:?} level {level}: block {i}"
                );
                for bit in [want.in_bits.start, want.in_bits.end - 1] {
                    assert_eq!(s.block_of_bit(bit), Some(i as u64), "{codec:?} level {level}: bit {bit}");
                }
                let t = s.block_trace(i as u64).expect("traces");
                t.check_tiles().unwrap_or_else(|e| panic!("{codec:?} level {level} block {i}: {e}"));
                let ours: Vec<Step> = t.steps().take(t.blocks()[0].steps.len()).collect();
                let theirs: Vec<Step> = want.steps.clone().map(|k| whole.step(k as usize).unwrap()).collect();
                assert!(ours == theirs, "{codec:?} level {level}: block {i}'s steps differ");
            }
            assert_eq!(s.block(s.blocks()).err(), Some(Refusal::Failed));
            // Bits outside every block: the wrapper's, and the padding.
            let first = whole.blocks()[0].in_bits.start;
            if first > 0 {
                assert_eq!(s.block_of_bit(first - 1), None);
            }
            assert_eq!(s.block_of_bit(whole.blocks().last().unwrap().in_bits.end), None);
        }
    }
    assert!(most > 100, "at most {most} blocks");
}

/// A block of more codes than a block's trace may hold has them as one
/// opaque step, which still tiles the block, and which the inspector
/// declines to take apart rather than misreading.
#[test]
fn a_block_of_too_many_codes_has_them_as_one_step() {
    let data = mixed(300_000, 37);
    let packed = flushed(&data, 6, 6000);
    let s = indexed(Codec::Deflate, &packed, Limits { span_steps: 200, ..small() });
    let mut coarse = 0;
    for i in 0..s.blocks() {
        let t = s.block_trace(i).expect("traces");
        t.check_tiles().unwrap_or_else(|e| panic!("block {i}: {e}"));
        let block = &t.blocks()[0];
        let payload: Vec<Step> = block.steps.clone().map(|k| t.step(k as usize).unwrap()).filter(|s| s.kind == StepKind::Opaque).collect();
        if let [one] = &payload[..] {
            coarse += 1;
            assert!(t.coarse());
            assert_eq!(one.in_bits.end, block.in_bits.end, "block {i}: the opaque step stops short of the block's end");
            let bit = (one.in_bits.start + one.in_bits.end) / 2;
            let k = t.index_in(bit).expect("a step reads it");
            assert_eq!(inflate::decode_step(&packed, &t, k), None, "block {i}: an opaque step taken apart");
        }
    }
    assert!(coarse > 10, "only {coarse} blocks over the limit");
}

/// Past the most block starts there may be, every second one goes and
/// the ones between are found again by decoding: every block still reads
/// as the whole trace has it, and no more starts are kept than the limit.
#[test]
fn block_starts_thin_out_and_the_rest_are_found_again() {
    let data = mixed(300_000, 31);
    let packed = flushed(&data, 1, 2000);
    let (_, whole) = crate::codec::decode_traced(Codec::Deflate, &packed).expect("decodes whole");
    let n = whole.blocks().len();
    let limits = Limits { block_marks: 4, ..small() };
    let s = indexed(Codec::Deflate, &packed, limits);
    assert!(n > 16, "only {n} blocks");
    assert!(s.blocks.list.len() <= 4 && s.blocks.stride > 1, "{} starts kept {} apart", s.blocks.list.len(), s.blocks.stride);
    assert_eq!(s.blocks(), n as u64);
    // Backwards, so that the groups turn over.
    for (i, want) in whole.blocks().iter().enumerate().rev() {
        let got = s.block(i as u64).expect("a block");
        assert_eq!((&got.in_bits, &got.out_bytes, got.kind), (&want.in_bits, &want.out_bytes, want.kind), "block {i}");
        let mid = (want.in_bits.start + want.in_bits.end) / 2;
        assert_eq!(s.block_of_bit(mid), Some(i as u64), "bit {mid}");
    }
}

/// Run `pass` to the end in steps of `out_bytes`, checking on the way that
/// it only ever moves forward.
fn in_steps(pass: Indexing, out_bytes: u64) -> (Result<Decoded, Refusal>, usize) {
    in_steps_of(pass, out_bytes, u64::MAX)
}

/// The same, with each step also stopped after about `in_bits` bits.
fn in_steps_of(mut pass: Indexing, out_bytes: u64, in_bits: u64) -> (Result<Decoded, Refusal>, usize) {
    let mut steps = 0;
    let mut was = pass.progress();
    loop {
        steps += 1;
        let stepped = pass.step(out_bytes, in_bits);
        let now = pass.progress();
        match stepped {
            Ok(Stepped::Working) => {
                assert!(now.read_bits >= was.read_bits && now.read_bits <= now.run_bits, "{was:?} then {now:?}");
                assert!(now.unpacked >= was.unpacked, "{was:?} then {now:?}");
                was = now;
            }
            Ok(Stepped::Done(d)) => return (Ok(d), steps),
            Err(why) => return (Err(why), steps),
        }
    }
}

/// The pass done in steps of every size, from smaller than one code's
/// output to larger than the run, comes to what the pass in one go comes
/// to: the same checkpoints and sums for a run past the cap, and for a run
/// under it the same bytes and the same trace, step for step. Steps stop
/// inside blocks too, so the runs are a block every 3,000 bytes and one
/// block for the whole run, which a step of 100 bytes stops in hundreds
/// of times.
#[test]
fn a_pass_in_steps_comes_to_what_one_pass_does() {
    let large = mixed(300_000, 29);
    let small_run = mixed(50_000, 31);
    for (data, raw) in [&large, &small_run]
        .into_iter()
        .flat_map(|data| [(data, deflate_in_blocks(data, 3000)), (data, miniz_oxide::deflate::compress_to_vec(data, 6))])
    {
        for codec in [Codec::Deflate, Codec::Gzip, Codec::Zlib] {
            let packed = Arc::new(wrapped(codec, &raw, data));
            let one = decode_or_index_with(codec, packed.clone(), small()).expect("decodes");
            // Steps of 200 bits also stop at the first code of a block whose
            // tables are longer than that, before any of its codes is read.
            for (out_bytes, in_bits) in [(100u64, u64::MAX), (4096, u64::MAX), (70_000, u64::MAX), (u64::MAX, u64::MAX), (u64::MAX, 200), (u64::MAX, 5000)] {
                let (got, steps) = in_steps_of(Indexing::new(codec, packed.clone(), small()).expect("starts"), out_bytes, in_bits);
                let at = format!("{codec:?}, {} bytes, steps of {out_bytes} bytes or {in_bits} bits", data.len());
                if out_bytes == 4096 || in_bits == 5000 {
                    assert!(steps > 5, "{at}: {steps} steps");
                }
                match (&one, got.expect("decodes in steps")) {
                    (Decoded::Whole(bytes, trace), Decoded::Whole(ours, t)) => {
                        assert!(ours == *bytes, "{at}: the bytes differ");
                        assert!(t.steps().eq(trace.steps()), "{at}: the steps differ");
                        assert_eq!(t.blocks(), trace.blocks(), "{at}");
                        assert_eq!((t.in_bits(), t.out_bytes()), (trace.in_bits(), trace.out_bytes()), "{at}");
                    }
                    (Decoded::Indexed(s), Decoded::Indexed(ours)) => {
                        assert_eq!(ours.marks(), s.marks(), "{at}");
                        assert_eq!((&ours.blocks, ours.blocks_end), (&s.blocks, s.blocks_end), "{at}");
                        assert_eq!((ours.len(), ours.crc32(), ours.adler32()), (s.len(), s.crc32(), s.adler32()), "{at}");
                        let mut all = vec![0u8; data.len()];
                        ours.read(0, &mut all).expect("reads");
                        assert!(all == *data, "{at}: the output differs");
                    }
                    _ => panic!("{at}: one pass and the steps disagree about holding it whole"),
                }
            }
        }
    }
}

/// A step that stops at the start of a block, or inside one, is carried
/// on by the next step, which is told of that place again or reads the
/// block's header again. Neither may record the block's start a second
/// time. Steps of 100 bytes stop at dozens of block starts and inside
/// dozens of blocks, before the cap and after it, and the block starts
/// they record are the ones the pass in one go records: the same list,
/// the same count, the same thinning, and each checkpoint counting the
/// same blocks before it, which is what the pass counts back to at the
/// cap. Both are the blocks of the whole trace.
#[test]
fn a_pass_in_steps_records_each_block_once() {
    let data = mixed(300_000, 53);
    // Nothing stops inside a stored block, so at level 0 every stop is at
    // the start of a block, and at level 6 most are inside one. Not gzip,
    // whose trailer says the run is large, so its pass starts past the
    // cap whether told or not.
    for (level, raw) in [(0u8, flushed(&data, 0, 3000)), (6, flushed(&data, 6, 3000)), (6, miniz_oxide::deflate::compress_to_vec(&data, 6))] {
        for codec in [Codec::Deflate, Codec::Zlib] {
            let packed = Arc::new(wrapped(codec, &raw, &data));
            let (_, whole) = crate::codec::decode_traced(codec, &packed).expect("decodes whole");
            for limits in [small(), Limits { block_marks: 4, ..small() }] {
                for told in [false, true] {
                    let at = format!("{codec:?} level {level}, {} blocks, at most {} kept, told {told}", whole.blocks().len(), limits.block_marks);
                    let start = || {
                        let pass = Indexing::new(codec, packed.clone(), limits).expect("starts");
                        if told { pass.known_large() } else { pass }
                    };
                    let Ok(Decoded::Indexed(one)) = in_steps(start(), u64::MAX).0 else { panic!("{at}: not indexed in one go") };
                    // How many steps stopped at a block's start and how
                    // many inside a block, before the cap and after it.
                    let mut stops = [[0usize; 2]; 2];
                    let mut pass = start();
                    let ours = loop {
                        match pass.step(100, u64::MAX) {
                            Ok(Stepped::Working) => {}
                            Ok(Stepped::Done(Decoded::Indexed(s))) => break s,
                            Ok(Stepped::Done(Decoded::Whole(..))) => panic!("{at}: came back whole"),
                            Err(why) => panic!("{at}: refused: {why:?}"),
                        }
                        match &pass.deflate().phase {
                            Phase::Whole { from: Some(r), .. } => stops[0][r.block.is_some() as usize] += 1,
                            Phase::Index { from, .. } => stops[1][from.block.is_some() as usize] += 1,
                            _ => {}
                        }
                    };
                    let inside = (level != 0) as usize;
                    if !told {
                        assert!(stops[0][inside] > 10, "{at}: stops before the cap {:?}", stops[0]);
                    }
                    assert!(stops[1][inside] > 10, "{at}: stops after the cap {:?}", stops[1]);
                    assert_eq!((&ours.blocks, ours.blocks_end), (&one.blocks, one.blocks_end), "{at}");
                    if limits.block_marks == 4 && whole.blocks().len() > 8 {
                        assert!(ours.blocks.stride > 1, "{at}: not thinned");
                    }
                    let counts = |s: &Seekable| s.checkpoints.iter().map(|c| (c.in_bit, c.blocks())).collect::<Vec<_>>();
                    assert_eq!(counts(&ours), counts(&one), "{at}");
                    for c in &ours.checkpoints {
                        let before = whole.blocks().partition_point(|b| b.in_bits.start < c.in_bit) as u64;
                        assert_eq!(c.blocks(), before, "{at}: the checkpoint at bit {}", c.in_bit);
                    }
                    assert_eq!(ours.blocks(), whole.blocks().len() as u64, "{at}");
                    for (i, want) in whole.blocks().iter().enumerate() {
                        let got = ours.block(i as u64).expect("a block");
                        assert_eq!(
                            (got.kind, got.last, &got.in_bits, &got.out_bytes),
                            (want.kind, want.last, &want.in_bits, &want.out_bytes),
                            "{at}: block {i}"
                        );
                    }
                }
            }
        }
    }
}

/// A trace that grows too long partway through a block names that
/// block's symbols as one step, the ones from steps before as well, and
/// carries on naming blocks in the steps after, as it would in one pass.
#[test]
fn a_trace_that_names_blocks_still_does_after_a_step() {
    let data = mixed(50_000, 37);
    for packed in [deflate_in_blocks(&data, 3000), miniz_oxide::deflate::compress_to_vec(&data, 6)] {
        let packed = Arc::new(packed);
        let limits = Limits { whole_steps: 2000, ..small() };
        let Ok(Decoded::Whole(_, trace)) = decode_or_index_with(Codec::Deflate, packed.clone(), limits) else { panic!("not whole") };
        assert!(trace.coarse());
        let (got, steps) = in_steps(Indexing::new(Codec::Deflate, packed, limits).expect("starts"), 100);
        assert!(steps > 100, "{steps} steps");
        let Ok(Decoded::Whole(_, t)) = got else { panic!("not whole in steps") };
        assert!(t.coarse());
        assert!(t.steps().eq(trace.steps()), "the steps differ");
        assert_eq!(t.blocks(), trace.blocks());
    }
}

/// `n` empty fixed-Huffman blocks, none of them the last. Each is ten bits,
/// the three bits of its header and the seven-bit end of block code, and
/// makes no output. Four of them come to five bytes, so with `n` a multiple
/// of four whatever follows them starts on a byte, as a deflate block after
/// a sync flush does.
fn empty_blocks(n: usize) -> Vec<u8> {
    assert_eq!(n % 4, 0);
    let mut out = vec![0u8; n / 4 * 5];
    for k in 0..n {
        // BTYPE is 01, read low bit first, after the BFINAL bit.
        let bit = 10 * k + 1;
        out[bit / 8] |= 1 << (bit % 8);
    }
    out
}

/// Raw deflate of `pieces`, one after another, with `between` after each
/// piece but the last. Each piece but the last ends with a sync flush, which
/// ends on a byte, so the blocks in `between` are part of the same stream.
fn deflate_around(pieces: &[&[u8]], between: &[u8]) -> Vec<u8> {
    use miniz_oxide::deflate::core::{compress, create_comp_flags_from_zip_params, CompressorOxide, TDEFLFlush, TDEFLStatus};
    let mut c = CompressorOxide::new(create_comp_flags_from_zip_params(6, -15, 0));
    let mut out = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        let last = i + 1 == pieces.len();
        let mut buf = vec![0u8; 4 * piece.len() + 1024];
        let (status, read, written) = compress(&mut c, piece, &mut buf, if last { TDEFLFlush::Finish } else { TDEFLFlush::Sync });
        assert!(read == piece.len() && matches!(status, TDEFLStatus::Okay | TDEFLStatus::Done), "{status:?}");
        out.extend_from_slice(&buf[..written]);
        if !last {
            out.extend_from_slice(between);
        }
    }
    out
}

/// A run of empty blocks makes no output, so only the bits a step reads end
/// it. Two runs of 40,000 empty blocks, one before the cap and one after it,
/// are indexed in steps of 10,000 bits, dozens of which make no output, and
/// come to the index one pass makes: the same checkpoints, block starts and
/// sums, with the block starts thinned or not, and told the run is large or
/// not.
#[test]
fn a_run_of_empty_blocks_is_indexed_in_many_steps() {
    let data = mixed(300_000, 83);
    let raw = deflate_around(&[&data[..20_000], &data[20_000..200_000], &data[200_000..]], &empty_blocks(40_000));
    for codec in [Codec::Deflate, Codec::Zlib, Codec::Gzip] {
        let packed = Arc::new(wrapped(codec, &raw, &data));
        for limits in [small(), Limits { block_marks: 4, ..small() }] {
            for told in [false, true] {
                let at = format!("{codec:?}, at most {} block starts kept, told {told}", limits.block_marks);
                let start = || {
                    let pass = Indexing::new(codec, packed.clone(), limits).expect("starts");
                    if told { pass.known_large() } else { pass }
                };
                let Ok(Decoded::Indexed(one)) = in_steps(start(), u64::MAX).0 else { panic!("{at}: not indexed in one go") };
                let mut pass = start();
                let (mut steps, mut idle, mut was) = (0, 0, pass.progress());
                let ours = loop {
                    steps += 1;
                    match pass.step(u64::MAX, 10_000) {
                        Ok(Stepped::Working) => {}
                        Ok(Stepped::Done(Decoded::Indexed(s))) => break s,
                        Ok(Stepped::Done(Decoded::Whole(..))) => panic!("{at}: came back whole"),
                        Err(why) => panic!("{at}: refused: {why:?}"),
                    }
                    let now = pass.progress();
                    if now.unpacked == was.unpacked && now.read_bits > was.read_bits {
                        idle += 1;
                    }
                    was = now;
                };
                assert!(idle >= 70 && steps > idle, "{at}: {steps} steps, {idle} of them with no output");
                assert_eq!(ours.marks(), one.marks(), "{at}");
                assert_eq!((&ours.blocks, ours.blocks_end), (&one.blocks, one.blocks_end), "{at}");
                assert_eq!((ours.len(), ours.crc32(), ours.adler32()), (one.len(), one.crc32(), one.adler32()), "{at}");
                assert!(ours.blocks() > 80_000, "{at}: {} blocks", ours.blocks());
                let mut all = vec![0u8; data.len()];
                ours.read(0, &mut all).expect("reads");
                assert!(all == data, "{at}: the output differs");
            }
        }
    }
}

/// Under the cap, a run of empty blocks in front of the data is held whole
/// after steps of a few bits as after one pass, with the same trace step for
/// step, a step of one bit included.
#[test]
fn a_run_of_empty_blocks_under_the_cap_comes_back_whole_in_steps_of_bits() {
    let data = mixed(30_000, 89);
    let raw = deflate_around(&[&data[..10_000], &data[10_000..]], &empty_blocks(2_000));
    let packed = Arc::new([empty_blocks(2_000), raw].concat());
    let Ok(Decoded::Whole(bytes, trace)) = decode_or_index_with(Codec::Deflate, packed.clone(), small()) else { panic!("not whole") };
    assert!(bytes == data);
    for in_bits in [1u64, 64, 999] {
        let (got, steps) = in_steps_of(Indexing::new(Codec::Deflate, packed.clone(), small()).expect("starts"), u64::MAX, in_bits);
        let Ok(Decoded::Whole(ours, t)) = got else { panic!("steps of {in_bits} bits: not whole") };
        // An empty block has two places to stop at, its start and its end
        // of block code, three bits apart.
        assert!(steps as u64 > 40_000 / in_bits.max(10), "steps of {in_bits} bits: {steps} steps");
        assert!(ours == bytes, "steps of {in_bits} bits: the bytes differ");
        assert!(t.steps().eq(trace.steps()), "steps of {in_bits} bits: the steps differ");
        assert_eq!(t.blocks(), trace.blocks(), "steps of {in_bits} bits");
    }
}

/// A pass told the run is large starts with the trace off and comes to the
/// same index as one that found out at the cap; one told so of a run
/// under the cap indexes it all the same. A gzip run's trailer tells it.
#[test]
fn a_pass_told_the_run_is_large_comes_to_the_same_index() {
    let data = mixed(300_000, 43);
    let raw = Arc::new(deflate_in_blocks(&data, 3000));
    let found = in_steps(Indexing::new(Codec::Deflate, raw.clone(), small()).expect("starts"), 5000).0;
    let told = in_steps(Indexing::new(Codec::Deflate, raw, small()).expect("starts").known_large(), 5000).0;
    let (Ok(Decoded::Indexed(found)), Ok(Decoded::Indexed(told))) = (found, told) else { panic!("not indexed") };
    assert_eq!(told.marks(), found.marks());
    assert_eq!((&told.blocks, told.blocks_end), (&found.blocks, found.blocks_end));
    assert_eq!((told.len(), told.crc32(), told.adler32()), (found.len(), found.crc32(), found.adler32()));

    let short = mixed(50_000, 47);
    let raw = Arc::new(miniz_oxide::deflate::compress_to_vec(&short, 6));
    let told = in_steps(Indexing::new(Codec::Deflate, raw, small()).expect("starts").known_large(), 5000).0;
    let Ok(Decoded::Indexed(told)) = told else { panic!("not indexed") };
    assert_eq!(told.len(), short.len() as u64);

    // A gzip trailer saying 50 KB comes back whole; one saying 1 MB about
    // the same 50 KB is wrong, and refused as a wrong trailer is.
    let gzip = wrapped(Codec::Gzip, &deflate_in_blocks(&short, 3000), &short);
    let Ok(Decoded::Whole(..)) = decode_or_index_with(Codec::Gzip, Arc::new(gzip.clone()), small()) else { panic!("not whole") };
    let mut lying = gzip;
    let n = lying.len();
    lying[n - 4..].copy_from_slice(&(1u32 << 20).to_le_bytes());
    assert_eq!(decode_or_index_with(Codec::Gzip, Arc::new(lying), small()).err(), Some(Refusal::Failed));
}

/// Refusals come out of the pass in steps as they do in one go.
#[test]
fn a_pass_in_steps_refuses_what_one_pass_does() {
    let zeros = vec![0u8; 5 << 20];
    let packed = Arc::new(miniz_oxide::deflate::compress_to_vec(&zeros, 9));
    let limits = Limits { lazy: 4 << 20, ..small() };
    let (got, _) = in_steps(Indexing::new(Codec::Deflate, packed, limits).expect("starts"), 64 << 10);
    assert_eq!(got.err(), Some(Refusal::TooLargeLazy));
    let mut gzip = gzip_of(&mixed(200_000, 41), 6);
    let n = gzip.len();
    gzip[n - 8] ^= 1;
    let (got, _) = in_steps(Indexing::new(Codec::Gzip, Arc::new(gzip), small()).expect("starts"), 5000);
    assert_eq!(got.err(), Some(Refusal::Failed));
}

/// Past the limit for a run read a piece at a time, a run is refused with
/// the refusal that says so, after a pass that kept no more than the
/// checkpoints.
#[test]
fn a_run_past_the_limit_is_refused() {
    let zeros = vec![0u8; 5 << 20];
    let packed = miniz_oxide::deflate::compress_to_vec(&zeros, 9);
    let limits = Limits { lazy: 4 << 20, ..small() };
    assert_eq!(decode_or_index_with(Codec::Deflate, Arc::new(packed), limits).err(), Some(Refusal::TooLargeLazy));
}

/// A trailer that disagrees with what came out is refused, as it is for a
/// run decoded whole, and so is a run cut short.
#[test]
fn a_wrong_trailer_or_a_short_run_is_refused() {
    let data = mixed(200_000, 19);
    let mut gzip = gzip_of(&data, 6);
    let n = gzip.len();
    gzip[n - 8] ^= 1;
    assert_eq!(decode_or_index_with(Codec::Gzip, Arc::new(gzip), small()).err(), Some(Refusal::Failed));
    let mut zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
    let n = zlib.len();
    zlib[n - 1] ^= 1;
    assert_eq!(decode_or_index_with(Codec::Zlib, Arc::new(zlib), small()).err(), Some(Refusal::Failed));
    let raw = miniz_oxide::deflate::compress_to_vec(&data, 6);
    let cut = raw[..raw.len() * 2 / 3].to_vec();
    assert_eq!(decode_or_index_with(Codec::Deflate, Arc::new(cut), small()).err(), Some(Refusal::Failed));
}

/// zstd frames of `chunk` bytes each, packed by `ruzstd`, one after
/// another. With `skippable`, each has a skippable frame in front of it
/// holding its length, which is what `pzstd` writes.
pub(crate) fn zstd_of(data: &[u8], chunk: usize, skippable: bool) -> Vec<u8> {
    use ruzstd::encoding::{compress_to_vec, CompressionLevel};
    let mut out = Vec::new();
    for (i, part) in data.chunks(chunk).enumerate() {
        let level = if i % 3 == 2 { CompressionLevel::Uncompressed } else { CompressionLevel::Fastest };
        let frame = compress_to_vec(part, level);
        if skippable {
            out.extend_from_slice(&0x184d_2a50u32.to_le_bytes());
            out.extend_from_slice(&4u32.to_le_bytes());
            out.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        }
        out.extend_from_slice(&frame);
    }
    out
}

/// An xz stream of blocks of `chunk` bytes each, packed by `lzma-rust2`.
pub(crate) fn xz_of(data: &[u8], chunk: usize) -> Vec<u8> {
    use crate::codec::xz::tests::{pack, wrap};
    let blocks: Vec<(Vec<u8>, Vec<u8>)> = data.chunks(chunk).map(|c| (pack(c, 3, 0, 2, 1 << 16, None), c.to_vec())).collect();
    wrap(&blocks, true)
}

/// Reads of every length from everywhere, across span joins, give what
/// went in, and so does one read of everything.
fn reads_back(s: &Seekable, data: &[u8], seed: u64) {
    assert_eq!(s.len(), data.len() as u64);
    let mut all = vec![0u8; data.len()];
    s.read(0, &mut all).expect("reads");
    assert!(all == data, "{:?}: the whole output differs", s.codec());
    let mut rand = rand_from(seed);
    for _ in 0..120 {
        let at = rand() % data.len() as u64;
        let n = (rand() % 50_000).min(data.len() as u64 - at) as usize;
        let mut got = vec![0u8; n];
        s.read(at, &mut got).expect("reads");
        assert!(got == data[at as usize..at as usize + n], "{:?}: {n} bytes at {at} differ", s.codec());
    }
}

/// Every span's trace tiles its own bits and bytes, and every byte and bit
/// maps to the same step through the spans as through the trace of the
/// whole run.
fn traces_agree(s: &Seekable, whole: &Trace, seed: u64) {
    let packed_bits = s.packed().len() as u64 * 8;
    for k in 0..s.spans() {
        let t = s.span_trace(k).expect("traces");
        t.check_tiles().unwrap_or_else(|e| panic!("{:?} span {k}: {e}", s.codec()));
        assert_eq!(t.start().1, s.span_bytes(k).start);
        assert_eq!(t.out_bytes(), s.span_bytes(k).end);
    }
    assert_eq!(s.span_trace(s.spans() - 1).unwrap().in_bits(), packed_bits);
    let mut rand = rand_from(seed);
    let mut bytes: Vec<u64> = (0..2000).map(|_| rand() % s.len()).collect();
    let mut bits: Vec<u64> = (0..2000).map(|_| rand() % packed_bits).collect();
    bytes.sort_unstable();
    bits.sort_unstable();
    for byte in bytes {
        assert_eq!(s.map_out(byte), whole.map_out(byte), "{:?}: byte {byte}", s.codec());
    }
    for bit in bits {
        assert_eq!(s.map_in(bit), whole.map_in(bit), "{:?}: bit {bit}", s.codec());
    }
    for (in_bit, _, _) in s.marks() {
        for bit in [in_bit.saturating_sub(1), in_bit, in_bit + 1] {
            assert_eq!(s.map_in(bit), whole.map_in(bit), "{:?}: bit {bit} by checkpoint {in_bit}", s.codec());
        }
    }
}

/// A zstd run of several frames, with and without `pzstd`'s skippable
/// frames, past the cap: read back from anywhere, a checkpoint at the
/// start of a frame, and a trace per span that says what the whole
/// trace says.
#[test]
fn a_zstd_run_of_several_frames_reads_back_from_anywhere() {
    let data = mixed(400_000, 29);
    for skippable in [false, true] {
        let packed = zstd_of(&data, 30_000, skippable);
        let s = indexed(Codec::Zstd, &packed, small());
        reads_back(&s, &data, 31);
        let marks = s.marks();
        assert_eq!(marks[0], (0, 0, None));
        assert!(marks.len() > 4 && marks.len() <= small().checkpoints, "{} checkpoints", marks.len());
        assert!(s.crc32().is_none() && s.adler32().is_none());
        let (bytes, whole) = crate::codec::decode_traced(Codec::Zstd, &packed).expect("decodes whole");
        assert!(bytes == data);
        traces_agree(&s, &whole, 37);
    }
}

/// Under the cap a zstd run comes back whole, exactly as the whole
/// decoder gives it, and that is every frame and not only the first.
#[test]
fn a_zstd_run_under_the_cap_comes_back_whole_with_every_frame() {
    let data = mixed(50_000, 41);
    let packed = zstd_of(&data, 20_000, true);
    let (bytes, trace) = crate::codec::decode_traced(Codec::Zstd, &packed).expect("decodes");
    assert!(bytes == data, "{} bytes of {}", bytes.len(), data.len());
    trace.check_tiles().expect("tiles");
    // Three frames, each behind a skippable one.
    let headers = trace.steps().filter(|s| s.kind == StepKind::Header(StepField::FrameHeader, 0)).count();
    assert_eq!(headers, 6);
    let Ok(Decoded::Whole(ours, t)) = decode_or_index_with(Codec::Zstd, Arc::new(packed.clone()), small()) else {
        panic!("did not come back whole");
    };
    assert!(ours == bytes);
    assert!(t.steps().eq(trace.steps()));
}

/// A frame past the most one span may come to is refused as a frame too
/// large, which is what a file of one frame is, and so is a frame past it
/// after the cap, held to the span limit when the cap is larger, and in a
/// pass told the run is large. A frame that asks for a window over 128 MiB
/// is refused as too large, since nothing says how large the frame is. Past
/// the limit for a run read a piece at a time, a run of small frames is
/// refused with the refusal that says so.
#[test]
fn a_zstd_frame_past_the_span_limit_is_refused() {
    let frame = Some(Refusal::TooLargeUnit(crate::codec::SpanUnit::ZstdFrame));
    let data = mixed(300_000, 43);
    let one = zstd_of(&data, data.len(), false);
    assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(one), small()).err(), frame);
    // A frame of 150 KB is finished while the run may still be held whole,
    // under 200 KB, and found past the span limit once it may not.
    let held = Limits { whole: 200_000, span: 100_000, ..small() };
    let first_large = [zstd_of(&data[..150_000], 150_000, false), zstd_of(&data[150_000..], 30_000, false)].concat();
    assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(first_large), held).err(), frame);
    let mut sizes = zstd_of(&data, 30_000, false);
    sizes.extend(zstd_of(&data[..100_000], 100_000, false));
    assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(sizes.clone()), small()).err(), frame);
    let told = Indexing::new(Codec::Zstd, Arc::new(sizes), small()).expect("starts").known_large();
    assert_eq!(in_steps(told, u64::MAX).0.err(), frame);
    // Past the cap, then a frame of one raw block whose header asks for a
    // window of 1 TiB.
    let mut wide = zstd_of(&data, 30_000, false);
    wide.extend_from_slice(&[0x28, 0xb5, 0x2f, 0xfd, 0x00, (40 - 10) << 3]);
    wide.extend_from_slice(&(1u32 | 4 << 3).to_le_bytes()[..3]);
    wide.extend_from_slice(b"tiny");
    assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(wide), small()).err(), Some(Refusal::TooLarge));
    let many = zstd_of(&data, 30_000, true);
    let limits = Limits { lazy: 200_000, ..small() };
    assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(many), limits).err(), Some(Refusal::TooLargeLazy));
}

/// The zstd pass done in steps of every size, from smaller than one block's
/// output to larger than the run, comes to what the pass in one go comes
/// to: for a run past the cap the same checkpoints and bytes, and for a run
/// under it the same bytes and the same trace, step for step. Frames of
/// 400 KB are three windows long, so steps stop inside them, both before
/// the decoder has let go of any of a frame and after; frames of 30 KB are
/// shorter than their window, with and without `pzstd`'s skippable frames.
#[test]
fn a_zstd_pass_in_steps_comes_to_what_one_pass_does() {
    let data = mixed(1_200_000, 61);
    let long = Limits { span: 1 << 20, ..small() };
    let held = Limits { whole: 2 << 20, ..long };
    let runs = [
        (zstd_of(&data[..400_000], 30_000, false), small(), false),
        (zstd_of(&data[..400_000], 30_000, true), small(), false),
        (zstd_of(&data, 400_000, true), long, false),
        (zstd_of(&data, 400_000, true), held, true),
    ];
    for (i, (packed, limits, whole)) in runs.into_iter().enumerate() {
        let packed = Arc::new(packed);
        let one = decode_or_index_with(Codec::Zstd, packed.clone(), limits).expect("decodes");
        assert_eq!(matches!(one, Decoded::Whole(..)), whole, "run {i}");
        for out_bytes in [100u64, 4096, 70_000, u64::MAX] {
            let (got, steps) = in_steps(Indexing::new(Codec::Zstd, packed.clone(), limits).expect("starts"), out_bytes);
            let at = format!("run {i}, steps of {out_bytes}");
            if out_bytes == 4096 {
                assert!(steps > 8, "{at}: {steps} steps");
            }
            match (&one, got.expect("decodes in steps")) {
                (Decoded::Whole(bytes, trace), Decoded::Whole(ours, t)) => {
                    assert!(ours == *bytes && ours == data, "{at}: the bytes differ");
                    assert!(t.steps().eq(trace.steps()), "{at}: the steps differ");
                    assert_eq!((t.in_bits(), t.out_bytes()), (trace.in_bits(), trace.out_bytes()), "{at}");
                }
                (Decoded::Indexed(s), Decoded::Indexed(ours)) => {
                    assert_eq!(ours.marks(), s.marks(), "{at}");
                    assert_eq!(ours.len(), s.len(), "{at}");
                    let mut all = vec![0u8; ours.len() as usize];
                    ours.read(0, &mut all).expect("reads");
                    assert!(all == data[..all.len()], "{at}: the output differs");
                }
                _ => panic!("{at}: one pass and the steps disagree about holding it whole"),
            }
        }
    }
}

/// A step of the zstd pass stops between two blocks of a frame, so a step
/// of 16 KiB over frames of 400 KB lets go of no more than its share, one
/// block and one window, 128 KiB each here, and never of a whole frame.
#[test]
fn a_zstd_step_stops_inside_a_frame() {
    let data = mixed(1_200_000, 67);
    let packed = Arc::new(zstd_of(&data, 400_000, true));
    let mut pass = Indexing::new(Codec::Zstd, packed, Limits { span: 1 << 20, ..small() }).expect("starts");
    let (mut was, mut steps) = (pass.progress().unpacked, 0);
    loop {
        steps += 1;
        let done = matches!(pass.step(16 << 10, u64::MAX).expect("steps"), Stepped::Done(_));
        let now = pass.progress().unpacked;
        assert!(now - was <= (16 + 128 + 128) << 10, "step {steps}: {} bytes", now - was);
        was = now;
        if done {
            break;
        }
    }
    assert!(steps > 3 * 400_000 / (128 << 10), "{steps} steps");
}

/// `n` skippable frames, and then `n` zstd frames of no output. The first
/// of those frames counts the skippable frames as its own, so a checkpoint
/// taken there starts at the first skippable frame, however many steps
/// stopped among them.
fn empty_zstd_frames(n: usize) -> Vec<u8> {
    let empty = ruzstd::encoding::compress_to_vec(&[][..], ruzstd::encoding::CompressionLevel::Fastest);
    let skip = [&0x184d_2a50u32.to_le_bytes()[..], &0u32.to_le_bytes()].concat();
    [skip.repeat(n), empty.repeat(n)].concat()
}

/// Frames that make no output, and skippable frames, end a zstd step by the
/// bytes it reads, as deflate's empty blocks do. Runs of them before the cap
/// and after it are indexed in steps of 1,000 bytes, dozens of which make no
/// output, and come to the index one pass makes; under the cap, to the same
/// bytes and trace step for step.
#[test]
fn a_run_of_empty_zstd_frames_is_indexed_in_many_steps() {
    let data = mixed(300_000, 97);
    let empty = empty_zstd_frames(2_000);
    let packed = [
        zstd_of(&data[..20_000], 20_000, false),
        empty.clone(),
        zstd_of(&data[20_000..200_000], 30_000, true),
        empty.clone(),
        zstd_of(&data[200_000..], 30_000, false),
    ]
    .concat();
    let packed = Arc::new(packed);
    for told in [false, true] {
        let start = || {
            let pass = Indexing::new(Codec::Zstd, packed.clone(), small()).expect("starts");
            if told { pass.known_large() } else { pass }
        };
        let Ok(Decoded::Indexed(one)) = in_steps(start(), u64::MAX).0 else { panic!("told {told}: not indexed in one go") };
        let skips_at = zstd_of(&data[..20_000], 20_000, false).len() as u64 * 8;
        assert!(one.marks().iter().any(|m| m.0 == skips_at), "told {told}: no checkpoint at the first skippable frame");
        let mut pass = start();
        let (mut steps, mut idle, mut was) = (0, 0, pass.progress());
        let ours = loop {
            steps += 1;
            match pass.step(u64::MAX, 8 * 1000) {
                Ok(Stepped::Working) => {}
                Ok(Stepped::Done(Decoded::Indexed(s))) => break s,
                Ok(Stepped::Done(Decoded::Whole(..))) => panic!("told {told}: came back whole"),
                Err(why) => panic!("told {told}: refused: {why:?}"),
            }
            let now = pass.progress();
            if now.unpacked == was.unpacked && now.read_bits > was.read_bits {
                idle += 1;
            }
            was = now;
        };
        let per_run = empty.len() / 1000;
        assert!(idle >= per_run && steps > idle, "told {told}: {steps} steps, {idle} of them with no output");
        assert_eq!(ours.marks(), one.marks(), "told {told}");
        assert_eq!(ours.len(), one.len(), "told {told}");
        reads_back(&ours, &data, 101);
    }

    let short = Arc::new([zstd_of(&data[..10_000], 10_000, true), empty, zstd_of(&data[10_000..40_000], 10_000, true)].concat());
    let Ok(Decoded::Whole(bytes, trace)) = decode_or_index_with(Codec::Zstd, short.clone(), small()) else { panic!("not whole") };
    assert!(bytes == data[..40_000]);
    for in_bits in [8u64, 8 * 100, 8 * 5000] {
        let (got, steps) = in_steps_of(Indexing::new(Codec::Zstd, short.clone(), small()).expect("starts"), u64::MAX, in_bits);
        let Ok(Decoded::Whole(ours, t)) = got else { panic!("steps of {in_bits} bits: not whole") };
        assert!(steps > 4_000 / (in_bits as usize / 8).max(8), "steps of {in_bits} bits: {steps} steps");
        assert!(ours == bytes, "steps of {in_bits} bits: the bytes differ");
        assert!(t.steps().eq(trace.steps()), "steps of {in_bits} bits: the steps differ");
    }
}

/// A zstd pass told the run is large starts with the trace off and comes to
/// the same index as one that found out at the cap; one told so of a run
/// under the cap indexes it all the same.
#[test]
fn a_zstd_pass_told_the_run_is_large_comes_to_the_same_index() {
    let data = mixed(400_000, 59);
    let packed = Arc::new(zstd_of(&data, 30_000, true));
    let found = in_steps(Indexing::new(Codec::Zstd, packed.clone(), small()).expect("starts"), 5000).0;
    let told = in_steps(Indexing::new(Codec::Zstd, packed, small()).expect("starts").known_large(), 5000).0;
    let (Ok(Decoded::Indexed(found)), Ok(Decoded::Indexed(told))) = (found, told) else { panic!("not indexed") };
    assert_eq!(told.marks(), found.marks());
    assert_eq!(told.len(), found.len());
    reads_back(&told, &data, 61);

    let short = Arc::new(zstd_of(&data[..50_000], 20_000, true));
    let told = in_steps(Indexing::new(Codec::Zstd, short, small()).expect("starts").known_large(), 5000).0;
    let Ok(Decoded::Indexed(told)) = told else { panic!("not indexed") };
    reads_back(&told, &data[..50_000], 67);
}

/// Refusals come out of the zstd pass in steps as they do in one go: a
/// frame past the span limit, and a run past the limit for one read a piece
/// at a time.
#[test]
fn a_zstd_pass_in_steps_refuses_what_one_pass_does() {
    let data = mixed(300_000, 43);
    let one = Arc::new(zstd_of(&data, data.len(), false));
    let many = Arc::new(zstd_of(&data, 30_000, true));
    let lazy = Limits { lazy: 200_000, ..small() };
    for (packed, limits, why) in [(one, small(), Refusal::TooLargeUnit(crate::codec::SpanUnit::ZstdFrame)), (many, lazy, Refusal::TooLargeLazy)] {
        for told in [false, true] {
            let pass = Indexing::new(Codec::Zstd, packed.clone(), limits).expect("starts");
            let pass = if told { pass.known_large() } else { pass };
            assert_eq!(in_steps(pass, 5000).0.err(), Some(why), "told {told}");
        }
    }
}

/// A frame whose checksum does not match what it decodes to is refused, as
/// a gzip trailer that does not match is: decoded whole, and indexed past
/// the cap in one go and in steps. The damage is to a checksum, and to a
/// byte inside a frame of raw blocks, which without the checksum decodes to
/// bytes that are not what was packed.
#[test]
fn a_zstd_frame_whose_checksum_does_not_match_is_refused() {
    use ruzstd::encoding::{compress_to_vec, CompressionLevel};
    let data = mixed(400_000, 71);
    let frames: Vec<Vec<u8>> = data
        .chunks(30_000)
        .enumerate()
        .map(|(i, part)| compress_to_vec(part, if i % 3 == 2 { CompressionLevel::Uncompressed } else { CompressionLevel::Fastest }))
        .collect();
    // Every frame says it has a checksum: the descriptor's third bit.
    assert!(frames.iter().all(|f| f[4] & 0x04 != 0));
    let run: Vec<u8> = frames.concat();
    let start = |k: usize| frames[..k].iter().map(|f| f.len()).sum::<usize>();
    // The checksum of frame 7, its last four bytes, and a byte in the
    // middle of frame 8, which is raw.
    let sum = start(8) - 2;
    let content = start(8) + frames[8].len() / 2;
    for at in [sum, content] {
        let mut bad = run.clone();
        bad[at] ^= 0x10;
        let bad = Arc::new(bad);
        assert_eq!(decode_or_index_with(Codec::Zstd, bad.clone(), small()).err(), Some(Refusal::Failed), "byte {at}");
        assert_eq!(in_steps(Indexing::new(Codec::Zstd, bad.clone(), small()).expect("starts"), 5000).0.err(), Some(Refusal::Failed), "byte {at}");
        let whole = Limits { whole: 1 << 20, ..small() };
        assert_eq!(decode_or_index_with(Codec::Zstd, bad.clone(), whole).err(), Some(Refusal::Failed), "byte {at}");
        let short = bad[..start(9)].to_vec();
        assert_eq!(crate::codec::decode_traced(Codec::Zstd, &short).err(), Some(Refusal::Failed), "byte {at}");
    }
    // Undamaged, the same run reads.
    let s = indexed(Codec::Zstd, &run, small());
    reads_back(&s, &data, 73);
}

/// An xz stream of several blocks past the cap, laid out from its index:
/// read back from anywhere, a checkpoint at the start of a block, a trace
/// per span that says what the whole trace says, and members that are the
/// whole trace's.
#[test]
fn an_xz_stream_of_several_blocks_reads_back_from_anywhere() {
    let data = mixed(400_000, 47);
    let packed = xz_of(&data, 30_000);
    let s = indexed(Codec::Xz, &packed, small());
    reads_back(&s, &data, 53);
    let marks = s.marks();
    assert_eq!(marks[0], (0, 0, None));
    assert!(marks.len() > 4 && marks.len() <= small().checkpoints, "{} checkpoints", marks.len());
    let (bytes, whole) = crate::codec::decode_traced(Codec::Xz, &packed).expect("decodes whole");
    assert!(bytes == data);
    assert!(!whole.coarse());
    traces_agree(&s, &whole, 59);
    assert_eq!(s.members(), whole.members());
}

/// One block past the most one span may come to is refused as a block too
/// large, which is what a file of one block is; past the limit for a run
/// read a piece at a time, so is a stream of small blocks, with the refusal
/// that says so. Under the cap the stream comes back whole as it always did.
#[test]
fn an_xz_block_past_the_span_limit_is_refused() {
    let data = mixed(300_000, 61);
    let one = xz_of(&data, data.len());
    assert_eq!(decode_or_index_with(Codec::Xz, Arc::new(one), small()).err(), Some(Refusal::TooLargeUnit(crate::codec::SpanUnit::XzBlock)));
    let limits = Limits { lazy: 200_000, ..small() };
    assert_eq!(decode_or_index_with(Codec::Xz, Arc::new(xz_of(&data, 30_000)), limits).err(), Some(Refusal::TooLargeLazy));
    let under = xz_of(&data[..50_000], 20_000);
    let (bytes, trace) = crate::codec::decode_traced(Codec::Xz, &under).expect("decodes");
    let Ok(Decoded::Whole(ours, t)) = decode_or_index_with(Codec::Xz, Arc::new(under), small()) else {
        panic!("did not come back whole");
    };
    assert!(ours == bytes);
    assert!(t.steps().eq(trace.steps()));
}

/// Nothing decodes an xz stream to index it, so a block that does not
/// come to what the index says opens, and its span is refused when read.
/// The spans either side still read.
#[test]
fn an_xz_block_the_index_is_wrong_about_is_refused_when_read() {
    use crate::codec::xz::tests::{pack, wrap};
    let data = mixed(300_000, 67);
    let mut blocks: Vec<(Vec<u8>, Vec<u8>)> = data.chunks(30_000).map(|c| (pack(c, 3, 0, 2, 1 << 16, None), c.to_vec())).collect();
    // The fifth block's packed data is the sixth's, so it decodes to a
    // stream of its own and not to what its index record says.
    blocks[4].0 = blocks[5].0.clone();
    blocks[4].1.truncate(29_000);
    let s = indexed(Codec::Xz, &wrap(&blocks, false), Limits { spacing: 1, ..small() });
    let k = s.span_of_byte(4 * 30_000).unwrap();
    let at = s.span_bytes(k).start;
    assert_eq!(s.read(at, &mut [0u8; 10]).err(), Some(Refusal::Failed));
    let mut first = vec![0u8; 30_000];
    s.read(0, &mut first).expect("the first block reads");
    assert!(first == data[..30_000]);
}

/// An xz stream of blocks of `block` bytes each, whose LZMA2 is a stream for
/// every `piece` bytes of the block, run together: one block holds many
/// chunks, each starting the dictionary over. With no `piece`, each block is
/// one stream as `lzma-rust2` packs it, which ends a chunk at 64 KiB of
/// packed data, and the chunks after the first carry its state on.
fn xz_pieces(data: &[u8], block: usize, piece: Option<usize>) -> Vec<u8> {
    use crate::codec::xz::tests::{pack, wrap};
    let lzma2 = |c: &[u8]| {
        let mut run = Vec::new();
        for p in c.chunks(piece.unwrap_or(c.len()).max(1)) {
            run.extend_from_slice(&pack(p, 3, 0, 2, 1 << 16, None));
            assert_eq!(run.pop(), Some(0x00), "an LZMA2 stream ends with a zero byte");
        }
        run.push(0x00);
        run
    };
    let blocks: Vec<(Vec<u8>, Vec<u8>)> = data.chunks(block).map(|c| (lzma2(c), c.to_vec())).collect();
    wrap(&blocks, true)
}

/// How far span `k` of an xz run has been decoded, from the cache of what
/// reads have reached. Nothing when no read has reached it, or the cache
/// let it go.
fn xz_decoded(s: &Seekable, k: usize) -> Option<usize> {
    let mut cache = lock(&s.prefixes);
    let p = cache.take(k as u64)?;
    let (n, weight) = (p.decoded(), p.weight());
    cache.put(k as u64, p, weight);
    Some(n)
}

/// Text, then noise an encoder cannot pack, then text again: an xz stream
/// of it holds stored chunks as well as packed ones.
fn text_and_noise() -> Vec<u8> {
    let mut rand = rand_from(89);
    let mut data = mixed(300_000, 83);
    data.extend((0..150_000).map(|_| rand() as u8));
    data.extend(mixed(250_000, 97));
    data
}

/// Text with every second 500 bytes of it noise, which packs to about half:
/// enough that `lzma-rust2` fills more than one chunk of 64 KiB for a block
/// of 200 kB, so the chunks after the first carry its state on.
fn half_noise() -> Vec<u8> {
    let mut rand = rand_from(103);
    let text = mixed(700_000, 107);
    text.chunks(500).enumerate().flat_map(|(i, c)| if i % 2 == 0 { c.to_vec() } else { c.iter().map(|_| rand() as u8).collect() }).collect()
}

/// A read of an xz span decodes up to the end of the LZMA2 chunk its last
/// byte is in and no further, and a read further on carries on from there.
/// Bytes read front to back, in random order, and partly early and partly
/// later, are the stream's bytes, from blocks of many chunks that start over,
/// of chunks that carry on, of stored chunks, and from spans of several
/// blocks. The span traces are the whole trace's, and the cache holds no more
/// than its limit and the newest span.
#[test]
fn an_xz_span_is_decoded_only_as_far_as_a_read_reaches() {
    let (text, half) = (text_and_noise(), half_noise());
    let limits = Limits { span: 256 << 10, span_cache: 200 << 10, ..small() };
    for (data, block, piece) in [(&text, 150_000, Some(4096)), (&half, 200_000, None), (&text, 40_000, Some(10_000))] {
        let packed = xz_pieces(data, block, piece);
        let (bytes, whole) = crate::codec::decode_traced(Codec::Xz, &packed).expect("decodes whole");
        assert!(bytes == *data);
        // The whole trace's blocks are the LZMA2 chunks.
        let chunks = whole.blocks();
        let blocks = data.len().div_ceil(block);
        assert!(chunks.len() >= blocks * 2, "{} chunks in {blocks} blocks", chunks.len());
        match piece {
            Some(_) => assert!(chunks.iter().any(|c| c.kind == BlockKind::Stored), "no stored chunks in blocks of {block}"),
            // A packed chunk that resets neither the state nor the dictionary.
            None => assert!(chunks.iter().any(|c| (0x80..0xa0).contains(&packed[(c.in_bits.start / 8) as usize])), "no chunk carries on"),
        }
        let chunk_end = |byte: u64| chunks.iter().find(|c| c.out_bytes.contains(&byte)).expect("a chunk made it").out_bytes.end;
        let span_len = |s: &Seekable, k: usize| (s.span_bytes(k).end - s.span_bytes(k).start) as usize;
        let check = |s: &Seekable, at: u64, n: usize| {
            let mut got = vec![0u8; n];
            s.read(at, &mut got).unwrap_or_else(|e| panic!("blocks of {block}: {n} bytes at {at}: {e:?}"));
            assert!(got == data[at as usize..at as usize + n], "blocks of {block}: {n} bytes at {at} differ");
        };
        // How far the span holding the last byte of a read of `n` at `at`
        // should be decoded once the read is done.
        let reached = |s: &Seekable, at: u64, n: usize| {
            let last = at + n as u64 - 1;
            let k = s.span_of_byte(last).unwrap();
            let start = s.span_bytes(k).start;
            assert_eq!(xz_decoded(s, k), Some((chunk_end(last) - start) as usize), "blocks of {block}: {n} bytes at {at}, span {k}");
        };

        // The first bytes of a span decode the first chunk of it.
        let s = indexed(Codec::Xz, &packed, limits);
        if block == 40_000 {
            assert!((0..s.spans()).any(|k| span_len(&s, k) > block), "no span of two blocks");
        }
        for k in 0..s.spans() {
            let start = s.span_bytes(k).start;
            check(&s, start, 10);
            reached(&s, start, 10);
        }

        // Front to back in pieces, each carrying on from the one before.
        let s = indexed(Codec::Xz, &packed, limits);
        let mut rand = rand_from(block as u64);
        let mut at = 0u64;
        while at < s.len() {
            let n = (1 + rand() % 20_000).min(s.len() - at) as usize;
            check(&s, at, n);
            reached(&s, at, n);
            at += n as u64;
        }

        // In random order, on a run nothing has read yet.
        let s = indexed(Codec::Xz, &packed, limits);
        for _ in 0..200 {
            let at = rand() % s.len();
            check(&s, at, (1 + rand() % 30_000).min(s.len() - at) as usize);
        }
        let largest = (0..s.spans()).map(|k| span_len(&s, k)).max().unwrap();
        let peak = lock(&s.prefixes).peak;
        assert!(peak <= limits.span_cache + largest + (64 << 10), "blocks of {block}: the cache held {peak} bytes, spans up to {largest}");
        assert!(s.held_bytes() <= limits.span_cache + largest + (64 << 10));

        // Part of a span early and the rest later agrees with all of it at
        // once and with the later part alone.
        for k in 0..s.spans() {
            let start = s.span_bytes(k).start;
            let (a, b) = (span_len(&s, k) / 3, span_len(&s, k) * 2 / 3);
            let early = indexed(Codec::Xz, &packed, limits);
            check(&early, start, a);
            check(&early, start + a as u64, b - a);
            let alone = indexed(Codec::Xz, &packed, limits);
            check(&alone, start + a as u64, b - a);
            let once = indexed(Codec::Xz, &packed, limits);
            check(&once, start, b);
        }
        traces_agree(&s, &whole, block as u64);
    }
}

/// A chunk whose data is damaged is found by a read that reaches it, and not
/// before: the chunks in front of it read, and so does every other block.
/// A read that is refused drops what was decoded of its span, so the
/// chunks in front still read after it.
#[test]
fn a_damaged_xz_chunk_is_refused_by_a_read_that_reaches_it() {
    let data = mixed(300_000, 101);
    let mut packed = xz_pieces(&data, 50_000, Some(4096));
    let (_, whole) = crate::codec::decode_traced(Codec::Xz, &packed).expect("decodes whole");
    // The fifth chunk of the third block starts the dictionary over, so a
    // code of all ones reads a match with nothing in front of it to copy.
    let primes: Vec<Step> = whole.steps().filter(|s| s.kind == StepKind::Header(StepField::RangeInit, 5)).collect();
    let fifth = primes.iter().filter(|s| (100_000..150_000).contains(&s.out_bytes.start)).nth(4).expect("five chunks in the third block");
    let at = (fifth.in_bits.start / 8) as usize;
    packed[at + 1..at + 5].fill(0xff);
    let damaged = fifth.out_bytes.start;

    let s = indexed(Codec::Xz, &packed, Limits { spacing: 1, ..small() });
    let k = s.span_of_byte(100_000).unwrap();
    assert_eq!(s.span_bytes(k), 100_000..150_000);
    let mut got = vec![0u8; 1000];
    s.read(100_000, &mut got).expect("the chunks in front of the damage read");
    assert!(got == data[100_000..101_000]);
    assert_eq!(s.read(damaged, &mut [0u8; 10]).err(), Some(Refusal::Failed));
    assert_eq!(s.read(149_990, &mut [0u8; 10]).err(), Some(Refusal::Failed));
    s.read(100_000, &mut got).expect("the chunks in front of the damage read again");
    assert!(got == data[100_000..101_000]);
    let mut rest = vec![0u8; 150_000];
    s.read(150_000, &mut rest).expect("the blocks after it read");
    assert!(rest == data[150_000..]);
}

/// Past the first window of a frame each block is a step of its own, and
/// the step says the bytes that block made: a raw block's step reads
/// exactly the bytes it writes. Before that the blocks are one step. The
/// same through the spans of a run indexed with frames this long.
#[test]
fn zstd_blocks_past_the_window_are_steps_of_their_own() {
    let data = mixed(1_200_000, 71);
    let packed = zstd_of(&data, 400_000, true);
    let (bytes, whole) = crate::codec::decode_traced(Codec::Zstd, &packed).expect("decodes");
    assert!(bytes == data);
    let mut raw = 0;
    let mut merged = 0;
    for step in whole.steps().filter(|s| s.kind == StepKind::Block) {
        let at = (step.in_bits.start / 8) as usize;
        let h = u32::from_le_bytes([packed[at - 3], packed[at - 2], packed[at - 1], 0]);
        let (kind, size) = ((h >> 1) & 3, (h >> 3) as u64);
        let reads = step.in_bits.end / 8 - step.in_bits.start / 8;
        let writes = step.out_bytes.end - step.out_bytes.start;
        if kind == 0 && reads == size {
            assert_eq!(writes, size, "raw block at byte {at}");
            assert!(packed[at..at + size as usize] == bytes[step.out_bytes.start as usize..step.out_bytes.end as usize]);
            raw += 1;
        }
        // No block may come to more than 128 KiB, so a step that does is
        // more than one block.
        merged += (writes > 128 << 10) as usize;
    }
    assert!(raw >= 2, "{raw} raw blocks checked");
    assert_eq!(merged, 3, "one step over the first window of each frame");
    let s = indexed(Codec::Zstd, &packed, Limits { span: 1 << 20, ..small() });
    reads_back(&s, &data, 73);
    traces_agree(&s, &whole, 79);
}

// Decoding from a window: a decoder handed a copy of only some of the run's
// bytes, told which byte of the run they start at, makes what it makes from
// the whole run. Each copy holds no more than the decode needs, so a decode
// that reads a byte outside it, or counts a position from the copy's front
// rather than the run's, is refused or disagrees.

/// What two traces of the same bits have to agree on.
fn shape(t: &Trace) -> (Vec<Step>, Vec<crate::codec::Block>, Vec<crate::codec::Member>, (u64, u64), u64, u64, bool) {
    (t.steps().collect(), t.blocks().to_vec(), t.members().to_vec(), t.start(), t.in_bits(), t.out_bytes(), t.coarse())
}

/// A copy of bytes `from..to` of `run`, and the byte it starts at.
fn copy_of(run: &[u8], from: usize, to: usize) -> (Vec<u8>, usize) {
    let to = to.min(run.len());
    (run[from..to].to_vec(), from)
}

/// The window over a copy made by [`copy_of`], as a window of `run`.
fn window<'a>(copy: &'a (Vec<u8>, usize), run: &[u8]) -> crate::codec::Window<'a> {
    crate::codec::Window::of(&copy.0, copy.1, run.len()).expect("inside the run")
}

/// What a decode of deflate span `k` needs, copied out of the run: from the
/// byte its checkpoint's bit is in to the byte holding the bit before the
/// next checkpoint's (or the run's end, for the last span), and the header
/// of the block the checkpoint is inside, [`inflate::MAX_HEAD_BYTES`] of it.
fn span_copies(s: &Seekable, packed: &[u8], k: usize) -> ((Vec<u8>, usize), Option<(Vec<u8>, usize)>) {
    let c = &s.checkpoints[k];
    let to = s.checkpoints.get(k + 1).map_or(packed.len(), |n| n.in_bit.div_ceil(8) as usize);
    let code = copy_of(packed, (c.in_bit / 8) as usize, to);
    let head = c.block().map(|h| copy_of(packed, (h / 8) as usize, (h / 8) as usize + inflate::MAX_HEAD_BYTES));
    (code, head)
}

/// Raw deflate of `data` in fixed-Huffman blocks only.
fn fixed_only(data: &[u8]) -> Vec<u8> {
    use miniz_oxide::deflate::core::deflate_flags::TDEFL_FORCE_ALL_STATIC_BLOCKS;
    use miniz_oxide::deflate::core::{compress, create_comp_flags_from_zip_params, CompressorOxide, TDEFLFlush, TDEFLStatus};
    let mut c = CompressorOxide::new(create_comp_flags_from_zip_params(6, -15, 0) | TDEFL_FORCE_ALL_STATIC_BLOCKS);
    let mut buf = vec![0u8; data.len() * 2 + 1024];
    let (status, read, written) = compress(&mut c, data, &mut buf, TDEFLFlush::Finish);
    assert!(read == data.len() && status == TDEFLStatus::Done, "{status:?}");
    buf.truncate(written);
    buf
}

/// Every deflate span, decoded and traced from copies of only the bytes it
/// needs, comes to the bytes and the trace it comes to from the whole run:
/// stored blocks, fixed and dynamic blocks with checkpoints inside them,
/// every wrapper, and the last span with the trailer.
#[test]
fn a_deflate_span_from_a_window_is_the_span_from_the_whole_run() {
    let data = mixed(300_000, 41);
    let (mut inside, mut header_before, mut kinds) = (0, 0, [0usize; 3]);
    for (codec, packed) in [
        (Codec::Deflate, miniz_oxide::deflate::compress_to_vec(&data, 0)),
        (Codec::Deflate, miniz_oxide::deflate::compress_to_vec(&data, 9)),
        (Codec::Zlib, miniz_oxide::deflate::compress_to_vec_zlib(&data, 6)),
        (Codec::Gzip, gzip_of(&data, 1)),
        (Codec::Deflate, flushed(&data, 6, 300)),
        (Codec::Gzip, gzip_around(&data, &fixed_only(&data))),
    ] {
        let s = indexed(codec, &packed, Limits { spacing: 4 << 10, checkpoints: 100, ..small() });
        assert!(s.spans() > 4, "{codec:?}: {} spans", s.spans());
        for k in 0..s.spans() {
            let (code, head) = span_copies(&s, &packed, k);
            let input = inflate::Input::new(window(&code, &packed), head.as_ref().map(|h| window(h, &packed)));
            let want = s.span_bytes(k);
            let mut bytes = vec![0u8; (want.end - want.start) as usize];
            s.read(want.start, &mut bytes).expect("reads");
            let got = s.deflate_span(k, input).unwrap_or_else(|e| panic!("{codec:?} span {k}: {e:?}"));
            assert!(got == bytes, "{codec:?} span {k}: the bytes differ");
            let t = s.deflate_span_trace(k, input).unwrap_or_else(|e| panic!("{codec:?} span {k}: {e:?}"));
            assert!(shape(&t) == shape(&s.span_trace(k).expect("traces")), "{codec:?} span {k}: the traces differ");
            if let Some(h) = s.checkpoints[k].block() {
                inside += 1;
                header_before += (h / 8 < code.1 as u64) as usize;
                let at = (h / 8) as usize;
                let bits = u16::from_le_bytes([packed[at], packed.get(at + 1).copied().unwrap_or(0)]) >> (h % 8);
                kinds[((bits >> 1) & 3) as usize] += 1;
            }
        }
        // A window that starts a byte late does not hold the checkpoint's
        // bit, and one without the header of the block the checkpoint is in
        // cannot rebuild its codes.
        let k = (1..s.spans()).find(|&k| s.checkpoints[k].block().is_some_and(|h| h / 8 + 2 < s.checkpoints[k].in_bit / 8));
        if let Some(k) = k {
            let (code, head) = span_copies(&s, &packed, k);
            let late = copy_of(&packed, code.1 + 1, code.1 + code.0.len());
            assert!(s.deflate_span(k, inflate::Input::new(window(&late, &packed), head.as_ref().map(|h| window(h, &packed)))).is_err());
            assert!(s.deflate_span(k, inflate::Input::new(window(&code, &packed), None)).is_err());
        }
    }
    assert!(inside > 50 && header_before > 50, "{inside} checkpoints inside blocks, {header_before} with the header before the window");
    assert!(kinds[1] > 0 && kinds[2] > 0, "checkpoints inside fixed and dynamic blocks: {kinds:?}");
}

/// Each block's trace, and the block starts found again between two kept
/// ones, come out the same from a copy of only that block's or that group's
/// bytes as from the whole run.
#[test]
fn deflate_blocks_from_a_window_are_the_blocks_from_the_whole_run() {
    let data = mixed(300_000, 43);
    let packed = flushed(&data, 1, 2000);
    let s = indexed(Codec::Deflate, &packed, Limits { block_marks: 4, ..small() });
    assert!(s.blocks.stride > 1 && s.blocks() > 16);
    for i in 0..s.blocks() {
        let b = s.block(i).expect("a block");
        let to = if i + 1 < s.blocks() { b.in_bits.end.div_ceil(8) as usize } else { packed.len() };
        let code = copy_of(&packed, (b.in_bits.start / 8) as usize, to);
        let t = s.block_trace_from(i, inflate::Input::new(window(&code, &packed), None)).unwrap_or_else(|e| panic!("block {i}: {e:?}"));
        assert!(shape(&t) == shape(&s.block_trace(i).expect("traces")), "block {i}: the traces differ");
    }
    for g in 0..s.blocks.list.len() {
        let to = s.blocks.list.get(g + 1).map_or(packed.len(), |m| m.0.div_ceil(8) as usize);
        let code = copy_of(&packed, (s.blocks.list[g].0 / 8) as usize, to);
        let got = s.group_from(g, inflate::Input::new(window(&code, &packed), None)).expect("found again");
        assert_eq!(got, s.group_from(g, inflate::Input::whole(&packed)).expect("found again"), "group {g}");
    }
}

/// A code taken apart from the bytes its bits are in, at most 7 of them,
/// reads as it does from the whole run; from a window that starts after the
/// code does, or ends before it does, it does not read at all.
#[test]
fn a_code_taken_apart_from_its_own_bytes_reads_as_from_the_whole_run() {
    let data = mixed(100_000, 47);
    let mut compared = 0;
    for (codec, packed) in [(Codec::Zlib, miniz_oxide::deflate::compress_to_vec_zlib(&data, 6)), (Codec::Deflate, flushed(&data, 6, 300))] {
        let (_, trace) = crate::codec::decode_traced(codec, &packed).expect("decodes whole");
        for (k, step) in trace.steps().enumerate() {
            let (from, to) = ((step.in_bits.start / 8) as usize, step.in_bits.end.div_ceil(8) as usize);
            let whole = inflate::decode_step(&packed, &trace, k);
            let Some(want) = whole else { continue };
            assert!(to - from <= 7, "step {k} is in {} bytes", to - from);
            assert_eq!(inflate::decode_step_at(&packed[from..to], from as u64, &trace, k), Some(want), "{codec:?} step {k}");
            assert_eq!(inflate::decode_step_at(&packed[from + 1..to], from as u64 + 1, &trace, k), None, "{codec:?} step {k}, a byte late");
            // Without the byte the step's last bit is in.
            let short = ((step.in_bits.end - 1) / 8) as usize;
            assert_eq!(inflate::decode_step_at(&packed[from..short], from as u64, &trace, k), None, "{codec:?} step {k}, cut short");
            compared += 1;
        }
    }
    assert!(compared > 10_000, "only {compared} codes compared");
}

/// What a zstd walk over the whole of `packed` comes to when it is handed a
/// window of `size` bytes from wherever it stopped, and on
/// [`Walked::Needs`](crate::codec::frames::Walked) exactly the bytes it asked
/// for, with how many times it asked for each kind of thing: a frame header,
/// a skippable frame's header, a block, and a frame's last block with its
/// checksum. A window one byte short of what it asked for has it ask again
/// for the same and read nothing.
fn zstd_walk_in_windows(packed: &[u8], size: usize) -> (Vec<u8>, Trace, [usize; 4]) {
    use crate::codec::frames::{zstd_done, Walked, ZstdWalk};
    let mut walk = ZstdWalk::new(0..packed.len(), 0, crate::codec::CAP_BYTES as u64);
    let mut b = crate::codec::TraceBuilder::default();
    let mut out = Vec::new();
    let mut asked = [0usize; 4];
    let (mut from, mut to) = (0, size);
    loop {
        let copy = copy_of(packed, from, to);
        let mut sink = |bytes: &[u8]| {
            out.extend_from_slice(bytes);
            Ok(())
        };
        match walk.run(window(&copy, packed), 20_000, 1 << 20, &mut b, &mut |_, _| {}, &mut sink).expect("walks") {
            Walked::Ended(end) => return (out, zstd_done(b, packed.len(), end), asked),
            Walked::Stopped => (from, to) = (walk.reached(), walk.reached() + size),
            Walked::Needs(need) => {
                assert_eq!(need.start, walk.reached());
                assert!(need.end > copy.1 + copy.0.len(), "asked for {need:?}, which {from}..{to} holds");
                let magic = u32::from_le_bytes(packed[need.start..need.start + 4].try_into().unwrap());
                asked[match magic {
                    0xfd2f_b528 => 0,
                    0x184d_2a50..=0x184d_2a5f => 1,
                    _ if packed[need.start] & 1 == 1 => 3,
                    _ => 2,
                }] += 1;
                if need.len() > 1 {
                    let short = copy_of(packed, need.start, need.end - 1);
                    let again = walk.run(window(&short, packed), 20_000, 1 << 20, &mut b, &mut |_, _| {}, &mut |_| panic!("decoded from a short window"));
                    assert_eq!(again, Ok(Walked::Needs(need.clone())));
                }
                (from, to) = (need.start, need.end.max(need.start + size));
            }
        }
    }
}

/// A zstd walk handed windows of the run, from a byte at a time to all of
/// it, comes to the bytes and the trace a walk over the whole run comes to,
/// with and without skippable frames, asking on the way for frame headers,
/// skippable headers, blocks and blocks with their checksums.
#[test]
fn a_zstd_walk_in_windows_comes_to_what_one_walk_does() {
    let data = mixed(400_000, 53);
    // Frames of 30 kB are one block each; frames of 140 kB are two.
    let mut asked_at_a_byte = [0usize; 4];
    for (chunk, skippable) in [(30_000, false), (30_000, true), (140_000, false), (140_000, true)] {
        let packed = zstd_of(&data, chunk, skippable);
        let (bytes, whole) = crate::codec::frames::zstd(&packed).expect("decodes whole");
        assert!(bytes == data);
        for size in [1, 7, 1000, 70_000, packed.len()] {
            let (got, trace, asked) = zstd_walk_in_windows(&packed, size);
            let what = format!("frames of {chunk}, skippable {skippable}, windows of {size}");
            assert!(got == data, "{what}: the bytes differ");
            assert!(shape(&trace) == shape(&whole), "{what}: the traces differ");
            if size == 1 {
                assert_eq!(asked[1] > 0, skippable, "{what}: asked for {asked:?}");
                asked_at_a_byte.iter_mut().zip(asked).for_each(|(n, a)| *n += a);
            }
            if size == packed.len() {
                assert_eq!(asked, [0; 4], "{what}");
            }
        }
    }
    assert!(asked_at_a_byte.iter().all(|&n| n > 0), "asked for {asked_at_a_byte:?}");
}

/// Every zstd span, decoded and traced from a copy of only its own frames,
/// comes to what it comes to from the whole run.
#[test]
fn a_zstd_span_from_a_window_is_the_span_from_the_whole_run() {
    let data = mixed(400_000, 59);
    for skippable in [false, true] {
        let packed = zstd_of(&data, 30_000, skippable);
        let s = indexed(Codec::Zstd, &packed, small());
        assert!(s.spans() > 4);
        for k in 0..s.spans() {
            let range = s.span_in(k);
            let copy = copy_of(&packed, range.start, range.end);
            let (c, want) = (&s.checkpoints[k], s.span_bytes(k));
            let got = zstd::decode_span(window(&copy, &packed), range.clone(), c.out, (want.end - want.start) as usize, small().span).expect("decodes");
            let mut bytes = vec![0u8; got.len()];
            s.read(want.start, &mut bytes).expect("reads");
            assert!(got == bytes, "skippable {skippable}, span {k}: the bytes differ");
            let t = zstd::span_trace(window(&copy, &packed), range, c.out, small().span).expect("traces");
            assert!(shape(&t) == shape(&s.span_trace(k).expect("traces")), "skippable {skippable}, span {k}: the traces differ");
        }
    }
}

/// Every xz span, read and traced from a copy of only its own blocks, comes
/// to what it comes to from the whole stream: blocks of one LZMA2 stream
/// whose chunks carry its state on, blocks of many that start over, and
/// stored chunks. A read part-way into a span from the copy, and one further
/// on carrying on from where it stopped, read the same bytes too.
#[test]
fn an_xz_span_from_a_window_is_the_span_from_the_whole_stream() {
    let mut spans = 0;
    for (data, block, piece) in [(half_noise(), 200_000, None), (text_and_noise(), 90_000, Some(20_000))] {
        let packed = xz_pieces(&data, block, piece);
        let l = crate::codec::xz::layout(&packed).expect("lays out");
        let s = indexed(Codec::Xz, &packed, Limits { span: 1 << 20, ..small() });
        for k in 0..s.spans() {
            let (range, units) = (s.span_in(k), s.span_units(k, &l));
            let copy = copy_of(&packed, range.start, range.end);
            let (c, want) = (&s.checkpoints[k], s.span_bytes(k));
            let t = xz::span_trace(window(&copy, &packed), &l, k, range, units.clone(), c.out).expect("traces");
            assert!(shape(&t) == shape(&s.span_trace(k).expect("traces")), "span {k}: the traces differ");
            let len = (want.end - want.start) as usize;
            let mut bytes = vec![0u8; len];
            s.read(want.start, &mut bytes).expect("reads");
            lock(&s.prefixes).take(k as u64);
            let (mut first, mut rest) = (vec![0u8; len / 3], vec![0u8; len - len / 2]);
            xz::read_span(&s.prefixes, window(&copy, &packed), &l, units.clone(), k, len / 5, &mut first).expect("reads");
            xz::read_span(&s.prefixes, window(&copy, &packed), &l, units, k, len / 2, &mut rest).expect("reads");
            assert!(first == bytes[len / 5..len / 5 + len / 3] && rest == bytes[len / 2..], "span {k}: the bytes differ");
            spans += 1;
        }
    }
    assert!(spans > 8, "{spans} spans");
}
