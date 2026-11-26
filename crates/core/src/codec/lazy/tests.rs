//! Tests for runs read a piece at a time: reads and traces checked against the
//! whole decoders', refusals at each limit, and the pass done in steps.

use super::deflate::Phase;
use super::seekable::lock;
use super::*;
use crate::checksum::{adler32, crc32};
use crate::codec::inflate;
use crate::codec::{Step, StepField, StepKind};

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
fn in_steps(mut pass: Indexing, out_bytes: u64) -> (Result<Decoded, Refusal>, usize) {
    let mut steps = 0;
    let mut was = pass.progress();
    loop {
        steps += 1;
        let stepped = pass.step(out_bytes);
        let now = pass.progress();
        match stepped {
            Ok(Stepped::Working) => {
                assert!(now.read_bits >= was.read_bits && now.read_bits <= now.run_bits, "{was:?} then {now:?}");
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
            for out_bytes in [100u64, 4096, 70_000, u64::MAX] {
                let (got, steps) = in_steps(Indexing::new(codec, packed.clone(), small()).expect("starts"), out_bytes);
                let at = format!("{codec:?}, {} bytes, steps of {out_bytes}", data.len());
                if out_bytes == 4096 {
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
                        match pass.step(100) {
                            Ok(Stepped::Working) => {}
                            Ok(Stepped::Done(Decoded::Indexed(s))) => break s,
                            Ok(Stepped::Done(Decoded::Whole(..))) => panic!("{at}: came back whole"),
                            Err(why) => panic!("{at}: refused: {why:?}"),
                        }
                        match &pass.phase {
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

/// A frame past the most one span may come to is refused as too large,
/// which is what a file of one frame is. Past the limit for a run read a
/// piece at a time, a run of small frames is refused with the refusal that
/// says so.
#[test]
fn a_zstd_frame_past_the_span_limit_is_refused() {
    let data = mixed(300_000, 43);
    let one = zstd_of(&data, data.len(), false);
    assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(one), small()).err(), Some(Refusal::TooLarge));
    let mut sizes = zstd_of(&data, 30_000, false);
    sizes.extend(zstd_of(&data[..100_000], 100_000, false));
    assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(sizes), small()).err(), Some(Refusal::TooLarge));
    let many = zstd_of(&data, 30_000, true);
    let limits = Limits { lazy: 200_000, ..small() };
    assert_eq!(decode_or_index_with(Codec::Zstd, Arc::new(many), limits).err(), Some(Refusal::TooLargeLazy));
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

/// One block past the most one span may come to is refused as too large,
/// which is what a file of one block is; past the limit for a run read a
/// piece at a time, so is a stream of small blocks, with the refusal that
/// says so. Under the cap the stream comes back whole as it always did.
#[test]
fn an_xz_block_past_the_span_limit_is_refused() {
    let data = mixed(300_000, 61);
    let one = xz_of(&data, data.len());
    assert_eq!(decode_or_index_with(Codec::Xz, Arc::new(one), small()).err(), Some(Refusal::TooLarge));
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
