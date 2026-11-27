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
    Limits {
        whole: 64 << 10,
        lazy: 4 << 20,
        spacing: 8 << 10,
        in_spacing: 8 << 10,
        checkpoints: 12,
        span_cache: 40 << 10,
        span_steps: 1_000_000,
        whole_steps: crate::codec::MAX_STEPS,
        block_marks: MAX_BLOCK_MARKS,
        span: 64 << 10,
        measure: crate::codec::CAP_BYTES as u64,
        unit_input: 16 << 20,
        span_input: 16 << 20,
        step_input: STEP_INPUT,
        input: INPUT_CAP_BYTES,
        copy_input: crate::codec::CAP_BYTES,
    }
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
            assert_eq!(s.map_out(byte), Ok(whole.map_out(byte)), "{codec:?}: byte {byte}");
        }
        for bit in bits {
            assert_eq!(s.map_in(bit), Ok(whole.map_in(bit)), "{codec:?}: bit {bit}");
        }
        // Every checkpoint's own bit, and the bits either side, which is
        // where a span's trace and the one before it meet.
        for (in_bit, _, block) in s.marks() {
            for bit in [in_bit.saturating_sub(1), in_bit, in_bit + 1].into_iter().chain(block) {
                assert_eq!(s.map_in(bit), Ok(whole.map_in(bit)), "{codec:?}: bit {bit} by checkpoint {in_bit}");
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
        let t = s.trace_at_bit(bit).unwrap().expect("a span names it");
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
                    assert_eq!(s.block_of_bit(bit), Ok(Some(i as u64)), "{codec:?} level {level}: bit {bit}");
                }
                let t = s.block_trace(i as u64).expect("traces");
                t.check_tiles().unwrap_or_else(|e| panic!("{codec:?} level {level} block {i}: {e}"));
                let ours: Vec<Step> = t.steps().take(t.blocks()[0].steps.len()).collect();
                let theirs: Vec<Step> = want.steps.clone().map(|k| whole.step(k as usize).unwrap()).collect();
                assert!(ours == theirs, "{codec:?} level {level}: block {i}'s steps differ");
            }
            assert_eq!(s.block(s.blocks()).err(), Some(ReadError::Refused(Refusal::Failed)));
            // Bits outside every block: the wrapper's, and the padding.
            let first = whole.blocks()[0].in_bits.start;
            if first > 0 {
                assert_eq!(s.block_of_bit(first - 1), Ok(None));
            }
            assert_eq!(s.block_of_bit(whole.blocks().last().unwrap().in_bits.end), Ok(None));
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
        assert_eq!(s.block_of_bit(mid), Ok(Some(i as u64)), "bit {mid}");
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
            Ok(Stepped::NeedsInput(m)) => panic!("a copy in memory missing {m:?}"),
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
                            Ok(Stepped::NeedsInput(m)) => panic!("a copy in memory missing {m:?}"),
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
                        Ok(Stepped::NeedsInput(m)) => panic!("a copy in memory missing {m:?}"),
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

/// Noise that no encoder can pack.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut rand = rand_from(seed);
    (0..len).map(|_| rand() as u8).collect()
}

/// A checkpoint is due by the run's input as well as by the output, so a
/// span's input is bounded however little the span makes: 160,000 empty
/// fixed blocks, which make nothing, stored blocks, and noise packed as well
/// as an encoder can, each over 200 KB of input. With room for every
/// checkpoint, no span reads more than the input spacing and the longest
/// stretch no checkpoint can be put inside, a stored block. A byte is never
/// read from a span of no output, and such a span's trace names its bits as
/// the whole trace does.
#[test]
fn checkpoints_are_taken_by_input_as_well_as_output() {
    let data = mixed(100_000, 109);
    let noise = noise(300_000, 113);
    let limits = Limits { checkpoints: 1000, ..small() };
    let stored = inflate::DEFLATE_MARGIN as u64 * 8;
    let runs = [
        ("empty blocks", deflate_around(&[&data[..50_000], &data[50_000..]], &empty_blocks(160_000)), &data),
        ("stored blocks", miniz_oxide::deflate::compress_to_vec(&noise, 0), &noise),
        ("noise", miniz_oxide::deflate::compress_to_vec(&noise, 9), &noise),
    ];
    for (what, raw, out) in runs {
        assert!(raw.len() > 200_000, "{what}: {} bytes", raw.len());
        let s = indexed(Codec::Deflate, &raw, limits);
        let by_output = indexed(Codec::Deflate, &raw, Limits { in_spacing: u64::MAX, ..limits });
        // A stored block is never stopped in, so its spans are its blocks
        // either way, and noise packs to about as much as it comes to.
        let more = if what == "empty blocks" { 10 } else { 0 };
        assert!(s.spans() >= by_output.spans() + more, "{what}: {} spans, {} by the output alone", s.spans(), by_output.spans());
        let marks = s.marks();
        let ends = marks.iter().skip(1).map(|m| m.0).chain([raw.len() as u64 * 8]);
        for (k, (m, end)) in marks.iter().zip(ends).enumerate() {
            assert!(end - m.0 <= limits.in_spacing * 8 + stored, "{what}: span {k} reads {} bits", end - m.0);
        }
        reads_back(&s, out, 127);
        for (k, m) in marks.iter().enumerate() {
            if m.1 < s.len() {
                let holding = s.span_of_byte(m.1).expect("a span holds it");
                assert!(s.span_bytes(holding).contains(&m.1), "{what}: byte {} of span {k} read from span {holding}", m.1);
            }
        }
        let (_, whole) = crate::codec::decode_traced(Codec::Deflate, &raw).expect("decodes whole");
        let mut empty = 0;
        for k in (0..s.spans()).filter(|&k| s.span_bytes(k).is_empty()) {
            empty += 1;
            let t = s.span_trace(k).expect("traces");
            t.check_tiles().unwrap_or_else(|e| panic!("{what}: span {k}: {e}"));
            let end = marks.get(k + 1).map_or(raw.len() as u64 * 8, |m| m.0);
            for bit in (marks[k].0..end).step_by(((end - marks[k].0) / 30).max(1) as usize) {
                assert_eq!(t.map_in(bit), whole.map_in(bit), "{what}: span {k}, bit {bit}");
                assert_eq!(s.map_in(bit), Ok(whole.map_in(bit)), "{what}: span {k}, bit {bit}");
            }
        }
        assert_eq!(empty > 10, what == "empty blocks", "{what}: {empty} spans of no output");
    }
}

/// Thinning doubles both spacings, and a checkpoint is then due by either
/// at twice the distance. A place offered twice is kept once.
#[test]
fn thinning_doubles_both_spacings() {
    let limits = Limits { spacing: 100, in_spacing: 10, checkpoints: 4, ..small() };
    let mut marks = Checkpoints::new(&limits);
    let unit = || Start::Unit(0);
    // A checkpoint every 100 bytes of output, at one byte of input each.
    for i in 0..5u64 {
        marks.offer(i * 8, i * 8, i * 100, unit);
        marks.offer(i * 8, i * 8, i * 100, unit);
    }
    let kept = |m: &Checkpoints| m.list.iter().map(|c| (c.in_bit, c.out)).collect::<Vec<_>>();
    assert_eq!(kept(&marks), [(0, 0), (16, 200), (32, 400)]);
    assert_eq!((marks.spacing, marks.in_spacing), (200, 160));
    // By the output, the next is due 200 bytes on; by the input, 20 bytes on.
    marks.offer(40, 40, 599, unit);
    marks.offer(32 + 159, 32 + 159, 400, unit);
    assert_eq!(kept(&marks).len(), 3);
    marks.offer(32 + 160, 32 + 160, 400, unit);
    assert_eq!(kept(&marks).last(), Some(&(192, 400)));
    // The fifth, thinned out with every second one, the first kept.
    marks.offer(200, 200, 600, unit);
    assert_eq!(kept(&marks), [(0, 0), (32, 400), (200, 600)]);
    assert_eq!((marks.spacing, marks.in_spacing), (400, 320));
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
    let packed_bits = s.input().len() * 8;
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
        assert_eq!(s.map_out(byte), Ok(whole.map_out(byte)), "{:?}: byte {byte}", s.codec());
    }
    for bit in bits {
        assert_eq!(s.map_in(bit), Ok(whole.map_in(bit)), "{:?}: bit {bit}", s.codec());
    }
    for (in_bit, _, _) in s.marks() {
        for bit in [in_bit.saturating_sub(1), in_bit, in_bit + 1] {
            assert_eq!(s.map_in(bit), Ok(whole.map_in(bit)), "{:?}: bit {bit} by checkpoint {in_bit}", s.codec());
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
    // Room for every checkpoint, the ones the empty frames' input makes
    // included, so that none is thinned out and the one looked for below
    // is kept.
    let limits = Limits { checkpoints: 64, ..small() };
    for told in [false, true] {
        let start = || {
            let pass = Indexing::new(Codec::Zstd, packed.clone(), limits).expect("starts");
            if told { pass.known_large() } else { pass }
        };
        let Ok(Decoded::Indexed(one)) = in_steps(start(), u64::MAX).0 else { panic!("told {told}: not indexed in one go") };
        assert!(one.marks().windows(2).any(|w| w[0].1 == w[1].1), "told {told}: no span of empty frames alone");
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
                Ok(Stepped::NeedsInput(m)) => panic!("a copy in memory missing {m:?}"),
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
    assert_eq!(s.read(at, &mut [0u8; 10]).err(), Some(ReadError::Refused(Refusal::Failed)));
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
    assert_eq!(s.read(damaged, &mut [0u8; 10]).err(), Some(ReadError::Refused(Refusal::Failed)));
    assert_eq!(s.read(149_990, &mut [0u8; 10]).err(), Some(ReadError::Refused(Refusal::Failed)));
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
            // The checkpoint's own copy of the header is the run's bytes.
            assert_eq!(s.checkpoints[k].head().map(|(b, at)| (b.to_vec(), at)), head, "{codec:?} span {k}");
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
        let t = s.block_trace_from(i, inflate::Input::new(window(&code, &packed), None), true).unwrap_or_else(|e| panic!("block {i}: {e:?}"));
        assert!(shape(&t) == shape(&s.block_trace(i).expect("traces")), "block {i}: the traces differ");
    }
    for g in 0..s.blocks.list.len() {
        let to = s.blocks.list.get(g + 1).map_or(packed.len(), |m| m.bit().div_ceil(8) as usize);
        let code = copy_of(&packed, (s.blocks.list[g].bit() / 8) as usize, to);
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
        match walk.run(window(&copy, packed), 20_000, 1 << 20, &mut b, &mut |_| Ok(()), &mut sink).expect("walks") {
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
                    let again = walk.run(window(&short, packed), 20_000, 1 << 20, &mut b, &mut |_| Ok(()), &mut |_| panic!("decoded from a short window"));
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
            let t = zstd::span_trace(window(&copy, &packed), range.start, &[], range, c.out, small().span).expect("traces");
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

/// A source over `bytes` that holds only some of its chunks of `chunk`
/// bytes, as a store that keeps part of the file does: a read reports the
/// chunks it does not hold, ascending and each once, and reads zeroes there.
/// Chunks are fed and dropped through `held`. `Send + Sync`, so a
/// [`RunInput`] can be made over it.
pub(crate) struct Lossy {
    bytes: Vec<u8>,
    chunk: u64,
    pub(crate) held: std::sync::Mutex<std::collections::BTreeSet<u64>>,
    /// What it says it holds at once, as a store does; it drops nothing to
    /// keep to it.
    limit: Option<u64>,
}

impl Lossy {
    /// A source that holds none of its chunks yet, and says it holds any
    /// read whole.
    pub(crate) fn new(bytes: Vec<u8>, chunk: u64) -> Arc<Lossy> {
        Arc::new(Lossy { bytes, chunk, held: Default::default(), limit: None })
    }

    /// The same, saying it holds at most `limit` bytes at once, so that an
    /// index reading through it is held to the limits on what one read
    /// gathers.
    pub(crate) fn keeping(bytes: Vec<u8>, chunk: u64, limit: u64) -> Arc<Lossy> {
        Arc::new(Lossy { bytes, chunk, held: Default::default(), limit: Some(limit) })
    }

    pub(crate) fn feed(&self, missing: &[crate::source::Missing]) {
        lock(&self.held).extend(missing.iter().map(|m| m.chunk));
    }

    pub(crate) fn drop_all(&self) {
        lock(&self.held).clear();
    }

    /// The chunks bytes `range` of the source are in.
    pub(crate) fn chunks(&self, range: std::ops::Range<usize>) -> Vec<crate::source::Missing> {
        let (first, last) = (range.start as u64 / self.chunk, (range.end as u64).div_ceil(self.chunk));
        (first..last).map(|chunk| crate::source::Missing { chunk }).collect()
    }
}

impl crate::source::Source for Lossy {
    fn len_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn resident_limit(&self) -> Option<u64> {
        self.limit
    }

    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<crate::source::Missing> {
        let held = lock(&self.held);
        let mut missing = Vec::new();
        let (mut pos, end) = (offset, offset + out.len() as u64);
        while pos < end {
            let chunk = pos / self.chunk;
            let stop = ((chunk + 1) * self.chunk).min(end);
            let (from, to) = ((pos - offset) as usize, (stop - offset) as usize);
            match held.contains(&chunk) {
                true => out[from..to].copy_from_slice(&self.bytes[pos as usize..stop as usize]),
                false => {
                    out[from..to].fill(0);
                    missing.push(crate::source::Missing { chunk });
                }
            }
            pos = stop;
        }
        missing
    }
}

/// The run `packed` with bytes before and after it, as a file holds it, and
/// where in that file the run starts. A run's input starting anywhere but at
/// the source's first byte is what a run of the file is.
fn in_a_file(packed: &[u8]) -> (Vec<u8>, usize) {
    let before = 1500;
    let mut file = mixed(before, 3);
    file.extend_from_slice(packed);
    file.extend_from_slice(&mixed(700, 4));
    (file, before)
}

/// Read `out.len()` bytes at `at` of `s`, and say which chunks the read was
/// missing each time it was made, feeding them, until it reads.
fn read_waiting(s: &Seekable, lossy: &Lossy, at: u64, out: &mut [u8]) -> Vec<Vec<crate::source::Missing>> {
    let mut asked = Vec::new();
    loop {
        match s.read(at, out) {
            Ok(()) => return asked,
            Err(ReadError::Missing(missing)) => {
                assert!(asked.len() < 50, "asked {} times", asked.len());
                lossy.feed(&missing);
                asked.push(missing);
            }
            Err(ReadError::Refused(why)) => panic!("refused: {why:?}"),
        }
    }
}

/// An index over a source that does not hold its run reads nothing and
/// names exactly the chunks of the file each read needs, all at once, for a
/// deflate, a zstd and an xz run. Once those are fed, the read gives the
/// bytes the run comes to. Nothing is cached from a read that was missing
/// chunks, so no span looks decoded when it is not, and the chunks are named
/// in the file, past the bytes before the run.
#[test]
fn a_read_through_an_input_missing_chunks_names_them_and_reads_once_they_are_fed() {
    let data = mixed(300_000, 61);
    let (mut inside_blocks, mut header_before) = (0, 0);
    for (codec, packed, limits) in [
        (Codec::Zlib, miniz_oxide::deflate::compress_to_vec_zlib(&data, 6), Limits { spacing: 16 << 10, ..small() }),
        (Codec::Zstd, zstd_of(&data, 30_000, true), small()),
        (Codec::Xz, xz_pieces(&data, 60_000, Some(20_000)), Limits { span: 1 << 20, ..small() }),
    ] {
        let mut s = indexed(codec, &packed, limits);
        assert!(s.spans() > 3, "{codec:?}: {} spans", s.spans());
        let (file, run_at) = in_a_file(&packed);
        let lossy = Lossy::new(file, 1024);
        s.set_input(RunInput::new(lossy.clone(), run_at as u64, packed.len() as u64));
        for k in 0..s.spans() {
            let want = s.span_bytes(k);
            if want.is_empty() {
                continue;
            }
            lossy.drop_all();
            let mut got = vec![0u8; (want.end - want.start) as usize];
            let asked = read_waiting(&s, &lossy, want.start, &mut got);
            assert!(got == data[want.start as usize..want.end as usize], "{codec:?} span {k}: the bytes differ");
            assert_eq!(asked.len(), 1, "{codec:?} span {k}: asked {asked:?}");
            let named = &asked[0];
            assert!(named.windows(2).all(|w| w[0].chunk < w[1].chunk), "{codec:?} span {k}: {named:?}");
            // What is named is the span's own bytes, in the file, past what
            // comes before the run: for deflate from its checkpoint's byte to
            // the byte of the next one's bit, and not the header of the block
            // the checkpoint is inside, which the checkpoint keeps a copy of;
            // for zstd its frames, without the skippable frame in front of the
            // first. An xz read names only the blocks it decodes, which are
            // inside the span.
            let mut range = if codec == Codec::Zstd { s.frames_in(k) } else { s.span_in(k) };
            if codec == Codec::Zlib {
                range.end = s.checkpoints.get(k + 1).map_or(packed.len(), |n| n.in_bit.div_ceil(8) as usize);
            }
            let whole = lossy.chunks(run_at + range.start..run_at + range.end);
            if let (Codec::Zlib, Some(h)) = (codec, s.checkpoints[k].block()) {
                inside_blocks += 1;
                header_before += (run_at + (h / 8) as usize + inflate::MAX_HEAD_BYTES <= whole[0].chunk as usize * 1024) as usize;
            }
            match codec {
                Codec::Xz => assert!(named.iter().all(|m| whole.contains(m)), "{codec:?} span {k}: {named:?} is not in {whole:?}"),
                _ => assert_eq!(named, &whole, "{codec:?} span {k}"),
            }
        }
        // A read that is missing chunks caches nothing for its span.
        let k = s.spans() / 2;
        let want = s.span_bytes(k);
        lock(&s.spans).take(k as u64);
        lock(&s.prefixes).take(k as u64);
        lossy.drop_all();
        assert!(matches!(s.read(want.start, &mut [0u8; 16]), Err(ReadError::Missing(_))));
        assert!(lock(&s.spans).take(k as u64).is_none(), "{codec:?}: span {k} was cached from nothing");
        assert_eq!(xz_decoded(&s, k).unwrap_or(0), 0, "{codec:?}: span {k} was decoded from nothing");
    }
    // Deflate spans that start inside a block, some of them chunks past the
    // block's header, which they read from the checkpoint's copy.
    assert!(inside_blocks > 3 && header_before > 0, "{inside_blocks} deflate spans start inside a block, {header_before} chunks past its header");
}

/// An index says which chunks of the file a read of its output would name,
/// without decoding or copying anything: for every span of a deflate, a
/// zstd and an xz run read from a source with none of the run loaded, a
/// read of the span and a read from its middle into the next name the same
/// chunks as it says. A span held decoded needs nothing, and nor does an xz
/// span decoded as far as the read reaches.
#[test]
fn an_index_says_which_chunks_a_read_would_name() {
    let data = mixed(300_000, 61);
    for (codec, packed, limits) in [
        (Codec::Zlib, miniz_oxide::deflate::compress_to_vec_zlib(&data, 6), Limits { spacing: 16 << 10, ..small() }),
        (Codec::Zstd, zstd_of(&data, 30_000, true), small()),
        (Codec::Xz, xz_pieces(&data, 60_000, Some(20_000)), Limits { span: 1 << 20, ..small() }),
    ] {
        let mut s = indexed(codec, &packed, limits);
        let (file, run_at) = in_a_file(&packed);
        let lossy = Lossy::new(file, 1024);
        s.set_input(RunInput::new(lossy.clone(), run_at as u64, packed.len() as u64));
        let forget = |s: &Seekable| {
            for k in 0..s.spans() as u64 {
                lock(&s.spans).take(k);
                lock(&s.prefixes).take(k);
            }
        };
        for k in 0..s.spans() {
            let want = s.span_bytes(k);
            let mid = (want.start + want.end) / 2;
            for (at, n) in [(want.start, want.end - want.start), (mid, (want.end - want.start).min(s.len() - mid))] {
                if n == 0 {
                    continue;
                }
                forget(&s);
                lossy.drop_all();
                let said = s.missing(at, n);
                let mut got = vec![0u8; n as usize];
                match s.read(at, &mut got) {
                    Err(ReadError::Missing(named)) => assert_eq!(said, named, "{codec:?} span {k}: {n} bytes at {at}"),
                    other => panic!("{codec:?} span {k}: {other:?}"),
                }
            }
        }
        let want = s.span_bytes(s.spans() / 2);
        let mut got = vec![0u8; (want.end - want.start) as usize];
        read_waiting(&s, &lossy, want.start, &mut got);
        lossy.drop_all();
        assert_eq!(s.missing(want.start, want.end - want.start), vec![], "{codec:?}: a span held decoded");
    }
}

/// A read of a run through its input asks the source which chunks are
/// missing first, and reads nothing when some are.
#[test]
fn a_read_of_a_run_asks_which_chunks_are_missing_before_it_copies() {
    use crate::source::tests::Asked;
    let source = Arc::new(Asked::new(64, 8, &[2]));
    let input = RunInput::new(source.clone(), 4, 50);
    let mut out = [0xeeu8; 20];
    assert_eq!(input.read(4, &mut out), Err(vec![crate::source::Missing { chunk: 2 }]));
    assert_eq!((source.reads(), out), (0, [0u8; 20]));
    assert_eq!(input.missing(0..4), vec![]);
    assert!(input.read(0, &mut [0u8; 4]).is_ok());
    assert_eq!(source.reads(), 1);
}

/// An xz span read part of the way keeps what it decoded when a later read
/// of it finds chunks missing, and that read, made again once they are fed,
/// carries on from there rather than from the span's first block.
#[test]
fn an_xz_read_missing_chunks_keeps_what_the_span_decoded() {
    let data = text_and_noise();
    let packed = xz_pieces(&data, 90_000, Some(20_000));
    let mut s = indexed(Codec::Xz, &packed, Limits { span: 1 << 20, ..small() });
    // A span of a block of 90 kB, whose LZMA2 chunks are 20 kB of output
    // each, so a read of its first bytes decodes only part of it.
    let k = (0..s.spans()).find(|&k| s.span_bytes(k).end - s.span_bytes(k).start > 50_000).expect("a span of a whole block");
    let want = s.span_bytes(k);
    let (file, run_at) = in_a_file(&packed);
    let lossy = Lossy::new(file, 1024);
    s.set_input(RunInput::new(lossy.clone(), run_at as u64, packed.len() as u64));
    let mut first = vec![0u8; 1000];
    read_waiting(&s, &lossy, want.start, &mut first);
    let decoded = xz_decoded(&s, k).expect("the span is cached");
    assert!(decoded > 0 && decoded < (want.end - want.start) as usize, "{decoded} bytes decoded");
    lossy.drop_all();
    let mut last = vec![0u8; 1000];
    let at = want.end - 1000;
    assert!(matches!(s.read(at, &mut last), Err(ReadError::Missing(_))));
    assert_eq!(xz_decoded(&s, k), Some(decoded), "the decoded part of the span was dropped");
    read_waiting(&s, &lossy, at, &mut last);
    assert!(first == data[want.start as usize..want.start as usize + 1000] && last == data[at as usize..want.end as usize]);
}

/// Every other read of an index through an input missing chunks names them
/// and gives what the index over the whole run gives once they are fed: a
/// span's trace, a block, the block holding a bit, a block's trace, and the
/// two maps, for blocks whose starts were thinned out and are found again.
#[test]
fn traces_and_blocks_through_an_input_missing_chunks_wait_for_them() {
    let data = mixed(300_000, 43);
    let packed = flushed(&data, 1, 2000);
    let limits = Limits { block_marks: 4, ..small() };
    let held = indexed(Codec::Deflate, &packed, limits);
    let mut s = indexed(Codec::Deflate, &packed, limits);
    assert!(s.blocks.stride > 1 && s.blocks() > 16);
    let (file, run_at) = in_a_file(&packed);
    let lossy = Lossy::new(file, 1024);
    s.set_input(RunInput::new(lossy.clone(), run_at as u64, packed.len() as u64));
    /// Ask `f` until it stops naming chunks, feeding each time, and say how
    /// many times it named some.
    fn waiting<T>(lossy: &Lossy, mut f: impl FnMut() -> Result<T, Vec<crate::source::Missing>>) -> (T, usize) {
        for asked in 0..50 {
            match f() {
                Ok(found) => return (found, asked),
                Err(missing) => {
                    assert!(!missing.is_empty(), "missing nothing");
                    lossy.feed(&missing);
                }
            }
        }
        panic!("asked 50 times")
    }
    let missing = |e: ReadError| match e {
        ReadError::Missing(m) => m,
        ReadError::Refused(why) => panic!("refused: {why:?}"),
    };
    let mut named = 0;
    for k in (0..s.spans()).step_by(3) {
        lossy.drop_all();
        let (t, asked) = waiting(&lossy, || s.span_trace(k).map_err(missing));
        assert!(shape(&t) == shape(&held.span_trace(k).unwrap()), "span {k}");
        named += asked;
    }
    for i in (0..s.blocks()).step_by(5) {
        lossy.drop_all();
        // A block reads the run only to find a start that was thinned out
        // (see `a_block_reads_none_of_the_run`), and its trace always does.
        let (b, _) = waiting(&lossy, || s.block(i).map_err(missing));
        assert_eq!(b, held.block(i).unwrap(), "block {i}");
        lossy.drop_all();
        let (t, asked) = waiting(&lossy, || s.block_trace(i).map_err(missing));
        assert!(shape(&t) == shape(&held.block_trace(i).unwrap()), "block {i}");
        named += asked;
        let mid = (b.in_bits.start + b.in_bits.end) / 2;
        lossy.drop_all();
        assert_eq!(waiting(&lossy, || s.block_of_bit(mid)).0, Some(i), "bit {mid}");
        lossy.drop_all();
        assert_eq!(waiting(&lossy, || s.map_in(mid)).0, held.map_in(mid).unwrap(), "bit {mid}");
        lossy.drop_all();
        let byte = b.out_bytes.start;
        assert_eq!(waiting(&lossy, || s.map_out(byte)).0, held.map_out(byte).unwrap(), "byte {byte}");
    }
    assert!(named > 10, "only {named} reads named chunks");
}

/// A skippable zstd frame of `len` bytes of noise.
fn skippable_frame(len: usize, seed: u64) -> Vec<u8> {
    [&0x184d_2a50u32.to_le_bytes()[..], &(len as u32).to_le_bytes(), &noise(len, seed)].concat()
}

/// Limits under which a frame or block of 32 KiB of input or more is large,
/// and the frames or blocks of a span of small ones take less than twice
/// that.
fn by_input() -> Limits {
    Limits { spacing: 64 << 10, in_spacing: 32 << 10, checkpoints: 64, span: 1 << 20, ..small() }
}

/// A zstd run of small frames and a large one, with long skippable frames in
/// front of one frame and after the last: the large frame is a span of its
/// own, the long skippable frames start a span, and those at the end are a
/// span of no output. No other span's frames take twice the input spacing,
/// and every span reads back and traces as the whole run does.
fn frames_large_and_small() -> (Vec<u8>, std::ops::Range<usize>, [usize; 2]) {
    let text = mixed(240_000, 131);
    let parts = [
        zstd_of(&text[..120_000], 10_000, true),
        zstd_of(&noise(100_000, 137), 100_000, true),
        zstd_of(&text[120_000..180_000], 10_000, false),
        skippable_frame(100_000, 139),
        zstd_of(&text[180_000..], 10_000, true),
        skippable_frame(40_000, 149),
    ];
    let at = |i: usize| parts[..i].iter().map(Vec::len).sum::<usize>();
    // The large frame without the skippable frame in front of it, where the
    // long skippable frame starts, and where those at the end do.
    (parts.concat(), at(1) + 12..at(2), [at(3), at(5)])
}

#[test]
fn a_large_zstd_frame_is_a_span_of_its_own() {
    let (packed, frame, [skip, tail]) = frames_large_and_small();
    let s = indexed(Codec::Zstd, &packed, by_input());
    let starts: Vec<usize> = s.marks().iter().map(|m| (m.0 / 8) as usize).collect();
    for at in [frame.start - 12, frame.end, skip, tail] {
        assert!(starts.contains(&at), "no checkpoint at byte {at}: {starts:?}");
    }
    let k = s.span_of_bit(frame.start as u64 * 8).expect("a span");
    assert_eq!(s.frames_in(k), frame, "the large frame's span");
    let last = s.spans() - 1;
    assert_eq!((s.span_bytes(last).is_empty(), s.frames_in(last).len(), s.span_in(last).start), (true, 0, tail), "the skippable frames at the end");
    for k in 0..s.spans() {
        let n = s.frames_in(k).len();
        assert!(n < 2 * (32 << 10) || s.frames_in(k) == frame, "span {k}'s frames take {n} bytes");
    }
    let (bytes, whole) = crate::codec::decode_traced(Codec::Zstd, &packed).expect("decodes whole");
    reads_back(&s, &bytes, 151);
    traces_agree(&s, &whole, 157);
    // The pass in steps, and told the run is large, comes to the same.
    for told in [false, true] {
        let pass = Indexing::new(Codec::Zstd, Arc::new(packed.clone()), by_input()).expect("starts");
        let pass = if told { pass.known_large() } else { pass };
        let Ok(Decoded::Indexed(ours)) = in_steps_of(pass, 3000, 8 * 2000).0 else { panic!("told {told}: not indexed") };
        assert_eq!(ours.marks(), s.marks(), "told {told}");
    }
}

/// A read of a zstd span's bytes reads its frames and none of the skippable
/// frames in front of them, however long; the span's trace reads a piece of
/// them and their headers. Both name only those chunks.
#[test]
fn a_zstd_span_read_skips_the_skippable_frames_in_front() {
    let (packed, _, [skip, _]) = frames_large_and_small();
    let mut s = indexed(Codec::Zstd, &packed, by_input());
    let held = indexed(Codec::Zstd, &packed, by_input());
    let k = s.span_of_bit(skip as u64 * 8).expect("a span");
    assert_eq!(s.span_in(k).start, skip);
    let frames = s.frames_in(k);
    assert_eq!(frames.start, skip + 100_008 + 12, "the frame after both skippable frames");
    let (file, run_at) = in_a_file(&packed);
    let lossy = Lossy::new(file, 1024);
    s.set_input(RunInput::new(lossy.clone(), run_at as u64, packed.len() as u64));
    let want = s.span_bytes(k);
    let mut got = vec![0u8; (want.end - want.start) as usize];
    let asked = read_waiting(&s, &lossy, want.start, &mut got);
    let mut expect = vec![0u8; got.len()];
    held.read(want.start, &mut expect).expect("reads");
    assert!(got == expect);
    assert_eq!(asked, [lossy.chunks(run_at + frames.start..run_at + frames.end)], "the read names the frames' chunks alone");
    lossy.drop_all();
    let mut named = Vec::new();
    let t = loop {
        match s.span_trace(k) {
            Ok(t) => break t,
            Err(ReadError::Missing(m)) => {
                assert!(named.len() < 10, "asked {named:?}");
                lossy.feed(&m);
                named.extend(m);
            }
            Err(ReadError::Refused(why)) => panic!("refused: {why:?}"),
        }
    };
    assert!(shape(&t) == shape(&held.span_trace(k).unwrap()), "the traces differ");
    let piece = lossy.chunks(run_at + skip..run_at + skip + zstd::FRONT_PIECE);
    let second = lossy.chunks(run_at + skip + 100_008..run_at + skip + 100_008 + 8);
    let frames = lossy.chunks(run_at + frames.start..run_at + frames.end);
    assert!(named.iter().all(|m| piece.contains(m) || second.contains(m) || frames.contains(m)), "the trace read {named:?}");
}

/// An xz block of the input spacing or more, among small ones, is a span of
/// its own, and every span reads back and traces as the whole stream does.
#[test]
fn a_large_xz_block_is_a_span_of_its_own() {
    use crate::codec::xz::tests::{pack, wrap};
    let text = mixed(200_000, 163);
    let mut data: Vec<Vec<u8>> = text.chunks(10_000).map(<[u8]>::to_vec).collect();
    data.insert(8, noise(60_000, 167));
    let blocks: Vec<(Vec<u8>, Vec<u8>)> = data.iter().map(|c| (pack(c, 3, 0, 2, 1 << 16, None), c.clone())).collect();
    let packed = wrap(&blocks, true);
    let l = crate::codec::xz::layout(&packed).expect("lays out");
    let large = l.blocks[8].at..l.blocks[9].at;
    assert!(large.len() > 60_000 && (0..l.blocks.len()).filter(|&i| i != 8).all(|i| l.block_len(i) < 32 << 10));
    let s = indexed(Codec::Xz, &packed, by_input());
    let k = s.span_of_bit(large.start as u64 * 8).expect("a span");
    assert_eq!(s.span_in(k), large, "the large block's span");
    assert_eq!(s.span_units(k, &l), 8..9);
    for k in 0..s.spans() {
        let n = s.span_in(k).len();
        assert!(n < 2 * (32 << 10) + 1024 || s.span_in(k) == large, "span {k} takes {n} bytes");
    }
    let (bytes, whole) = crate::codec::decode_traced(Codec::Xz, &packed).expect("decodes whole");
    reads_back(&s, &bytes, 173);
    traces_agree(&s, &whole, 179);
}

/// The pass keeps what each kept block start's header says, so a block of an
/// index reads none of the run: through an input holding none of it, every
/// block is the block the whole trace has. Once block starts are thinned
/// out, only a block whose start or end was dropped reads the run, to find
/// it again.
#[test]
fn a_block_reads_none_of_the_run() {
    let data = mixed(300_000, 199);
    let packed = flushed(&data, 6, 3000);
    let (_, whole) = crate::codec::decode_traced(Codec::Deflate, &packed).expect("decodes whole");
    let mut kinds = [0usize; 3];
    for limits in [small(), Limits { block_marks: 4, ..small() }] {
        let mut s = indexed(Codec::Deflate, &packed, limits);
        let (file, run_at) = in_a_file(&packed);
        s.set_input(RunInput::new(Lossy::new(file, 1024), run_at as u64, packed.len() as u64));
        let stride = s.blocks.stride;
        let kept = |i: u64| i % stride == 0 || i == s.blocks();
        for (i, want) in whole.blocks().iter().enumerate() {
            let i = i as u64;
            match s.block(i) {
                Ok(got) => {
                    assert!(kept(i) && kept(i + 1), "block {i} of every {stride} kept");
                    assert_eq!((got.kind, got.last, &got.in_bits, &got.out_bytes), (want.kind, want.last, &want.in_bits, &want.out_bytes), "block {i}");
                    kinds[match got.kind {
                        BlockKind::Stored => 0,
                        BlockKind::Fixed => 1,
                        _ => 2,
                    }] += 1;
                }
                Err(ReadError::Missing(_)) => assert!(!kept(i) || !kept(i + 1), "block {i} of every {stride} kept read the run"),
                Err(ReadError::Refused(why)) => panic!("block {i}: refused: {why:?}"),
            }
        }
    }
    assert!(kinds[0] > 0 && kinds[2] > 0, "stored, fixed and dynamic blocks: {kinds:?}");
}

/// A block of more codes than a block's trace may hold is decoded only until
/// its trace turns coarse: the opaque step then runs on to the block's end,
/// which the index knows, and the trace is the one a decode to the block's
/// end makes, the last block's padding and what follows it included. The
/// block is read only as far as the codes before that point can reach, and
/// for a long block that is well short of its end.
#[test]
fn a_block_trace_stops_where_it_turns_coarse() {
    let data = mixed(300_000, 211);
    let limits = Limits { span_steps: 200, ..small() };
    let reach = inflate::MAX_HEAD_BYTES + 200 * 6;
    let (mut coarse, mut short) = (0, 0);
    for (codec, packed) in [(Codec::Deflate, flushed(&data, 6, 6000)), (Codec::Gzip, gzip_of(&data, 6)), (Codec::Zlib, wrapped(Codec::Zlib, &fixed_only(&data), &data))] {
        let held = indexed(codec, &packed, limits);
        let mut s = indexed(codec, &packed, limits);
        let (file, run_at) = in_a_file(&packed);
        let lossy = Lossy::new(file, 1024);
        s.set_input(RunInput::new(lossy.clone(), run_at as u64, packed.len() as u64));
        for i in 0..s.blocks() {
            let b = held.block(i).expect("a block");
            let to_the_end = held.block_trace_from(i, inflate::Input::whole(&packed), false).expect("traces");
            lossy.drop_all();
            let mut named = Vec::new();
            let t = loop {
                match s.block_trace(i) {
                    Ok(t) => break t,
                    Err(ReadError::Missing(m)) => {
                        lossy.feed(&m);
                        named.extend(m);
                    }
                    Err(ReadError::Refused(why)) => panic!("{codec:?} block {i}: refused: {why:?}"),
                }
            };
            assert!(shape(&t) == shape(&to_the_end), "{codec:?} block {i}: the traces differ");
            t.check_tiles().unwrap_or_else(|e| panic!("{codec:?} block {i}: {e}"));
            if to_the_end.coarse() {
                coarse += 1;
                let from = run_at + (b.in_bits.start / 8) as usize;
                assert!(named.iter().all(|m| (m.chunk as usize * 1024) < from + reach), "{codec:?} block {i}: read {named:?} from byte {from}");
                short += ((b.in_bits.end / 8) as usize + run_at > from + reach + 2048) as usize;
            }
        }
    }
    assert!(coarse > 10 && short > 5, "{coarse} coarse blocks, {short} of them read well short of their end");
}

/// A run in a file read from a store that keeps `limit` bytes at once, with
/// all of the file's chunks loaded.
fn from_a_store(packed: &[u8], limit: u64) -> RunInput {
    let (file, run_at) = in_a_file(packed);
    let n = file.len();
    let lossy = Lossy::keeping(file, 1024, limit);
    lossy.feed(&lossy.chunks(0..n));
    RunInput::new(lossy, run_at as u64, packed.len() as u64)
}

/// A zstd frame or an xz block whose input passes the limit on one, which a
/// read of its span reads whole, is refused as too much to read at once
/// when the run is read from a store that keeps only part of the file, and
/// not as too large to unpack, which it is not: in one pass, in steps
/// and told the run is large. Read from a copy in memory, the same run
/// indexes and reads back as before, and so does one that is held whole.
#[test]
fn a_frame_or_block_too_large_to_read_at_once_is_refused_only_from_a_store() {
    let limits = Limits { unit_input: 80_000, ..by_input() };
    let (frames, large, _) = frames_large_and_small();
    assert!(large.len() > 100_000);
    let text = mixed(200_000, 163);
    let mut data: Vec<Vec<u8>> = text.chunks(10_000).map(<[u8]>::to_vec).collect();
    data.insert(8, noise(100_000, 167));
    let blocks: Vec<(Vec<u8>, Vec<u8>)> = data.iter().map(|c| (crate::codec::xz::tests::pack(c, 3, 0, 2, 1 << 16, None), c.clone())).collect();
    let stream = crate::codec::xz::tests::wrap(&blocks, true);
    for (codec, packed) in [(Codec::Zstd, &frames), (Codec::Xz, &stream)] {
        let refused = decode_or_index_through(codec, Arc::new(packed.clone()), from_a_store(packed, 32 << 20), limits);
        assert_eq!(refused.err(), Some(Refusal::TooMuchAtOnce), "{codec:?}");
        let s = indexed(codec, packed, limits);
        let (bytes, _) = crate::codec::decode_traced(codec, packed).expect("decodes whole");
        reads_back(&s, &bytes, 181);
        let whole = Limits { whole: 1 << 20, ..limits };
        let held = decode_or_index_through(codec, Arc::new(packed.clone()), from_a_store(packed, 32 << 20), whole);
        assert!(matches!(held, Ok(Decoded::Whole(..))), "{codec:?}: a run held whole has no span to read");
    }
    for told in [false, true] {
        let pass = Indexing::from_copy(Codec::Zstd, Arc::new(frames.clone()), from_a_store(&frames, 32 << 20), limits).expect("starts");
        let pass = if told { pass.known_large() } else { pass };
        assert_eq!(in_steps_of(pass, 3000, 8 * 2000).0.err(), Some(Refusal::TooMuchAtOnce), "told {told}");
    }
}

/// A zstd or xz span that thinning has made larger than one read may gather
/// is refused when its bytes or its trace are read from a store that keeps
/// only part of the file, before anything is read. An xz read gathers a
/// block at a time instead, so it reads the span's bytes all the same, and
/// no answer names more chunks than the limit covers. A span under the limit
/// reads, and from a copy in memory every span does.
#[test]
fn a_span_past_what_one_read_may_gather_is_refused_from_a_store() {
    let text = mixed(400_000, 191);
    let thinned = Limits { spacing: 16 << 10, in_spacing: 16 << 10, checkpoints: 6, ..small() };
    for (codec, packed) in [(Codec::Zstd, zstd_of(&text, 10_000, true)), (Codec::Xz, xz_of(&text, 10_000))] {
        // The limit on one read does not change where the spans are, so it
        // is set to put about half of them over it.
        let s = indexed(codec, &packed, thinned);
        let mut sizes: Vec<usize> = (0..s.spans()).map(|k| if codec == Codec::Zstd { s.frames_in(k).len() } else { s.span_in(k).len() }).collect();
        sizes.sort_unstable();
        let limits = Limits { span_input: sizes[sizes.len() / 2], ..thinned };
        let held = indexed(codec, &packed, limits);
        reads_back(&held, &text, 193);
        let mut s = indexed(codec, &packed, limits);
        let (file, run_at) = in_a_file(&packed);
        let lossy = Lossy::keeping(file, 1024, 32 << 20);
        s.set_input(RunInput::new(lossy.clone(), run_at as u64, packed.len() as u64));
        let (mut over, mut under) = (0, 0);
        for k in 0..s.spans() {
            let want = s.span_bytes(k);
            let gathers = match codec {
                Codec::Zstd => s.frames_in(k).len(),
                _ => s.span_in(k).len(),
            };
            lossy.drop_all();
            let mut got = vec![0u8; (want.end - want.start) as usize];
            if gathers > limits.span_input {
                over += 1;
                assert_eq!(s.span_trace(k).err(), Some(ReadError::Refused(Refusal::TooMuchAtOnce)), "{codec:?} span {k}");
                if codec == Codec::Zstd {
                    assert_eq!(s.read(want.start, &mut got).err(), Some(ReadError::Refused(Refusal::TooMuchAtOnce)), "{codec:?} span {k}");
                    continue;
                }
            } else {
                under += 1;
                let t = loop {
                    match s.span_trace(k) {
                        Ok(t) => break t,
                        Err(ReadError::Missing(m)) => lossy.feed(&m),
                        Err(ReadError::Refused(why)) => panic!("{codec:?} span {k}: refused: {why:?}"),
                    }
                };
                assert!(shape(&t) == shape(&held.span_trace(k).unwrap()), "{codec:?} span {k}");
                lossy.drop_all();
            }
            let asked = read_waiting(&s, &lossy, want.start, &mut got);
            assert!(got == text[want.start as usize..want.end as usize], "{codec:?} span {k}");
            for named in &asked {
                assert!(named.len() <= limits.span_input / 1024 + 2, "{codec:?} span {k}: {} chunks named at once", named.len());
            }
        }
        assert!(over > 0 && under > 0, "{codec:?}: {over} spans over the limit, {under} under it");
    }
}

/// A stretch between two kept deflate block starts longer than the limit on
/// one read, which only a run of more blocks than are kept has, is refused
/// when the blocks in it are looked for from a store that keeps only part of
/// the file. Read from a copy in memory, they are found as before.
#[test]
fn a_long_stretch_between_kept_block_starts_is_refused_from_a_store() {
    let data = mixed(300_000, 197);
    let packed = flushed(&data, 1, 2000);
    let limits = Limits { block_marks: 4, unit_input: 20_000, ..small() };
    let held = indexed(Codec::Deflate, &packed, limits);
    let mut s = indexed(Codec::Deflate, &packed, limits);
    s.set_input(from_a_store(&packed, 32 << 20));
    let stride = s.blocks.stride;
    let stretch = |g: usize| s.blocks.list.get(g + 1).map_or(packed.len() as u64 * 8, |m| m.bit()).div_ceil(8) - s.blocks.list[g].bit() / 8;
    let (mut over, mut under) = (0, 0);
    for g in 0..s.blocks.list.len() {
        let i = g as u64 * stride + 1;
        if i >= s.blocks() {
            continue;
        }
        match stretch(g) > 20_000 {
            true => {
                over += 1;
                assert_eq!(s.block(i).err(), Some(ReadError::Refused(Refusal::TooMuchAtOnce)), "block {i}");
            }
            false => {
                under += 1;
                assert_eq!(s.block(i), held.block(i), "block {i}");
            }
        }
        assert!(held.block(i).is_ok(), "block {i}");
    }
    assert!(over > 0, "{over} stretches over the limit, {under} under it");
}

// ----- the pass reading its run a window at a time -----
//
// The pass reads its run through the run's input a window at a time, so a
// pass over a handle onto the file's chunk store holds a window and not the
// run. These read runs from a store of 1 KiB chunks that keeps the chunks
// read most recently and pushes out the rest, fed exactly the chunks each
// step names, as the browser feeds it.

/// [`small`], with steps that read 8 KiB of the run and the margin past it,
/// so that runs of a few hundred kilobytes take dozens of windows.
fn in_windows() -> Limits {
    Limits { step_input: 8 << 10, ..small() }
}

/// The same limits with one window over the whole run, as the pass read it
/// before it read windows: the index to compare with.
fn one_window(limits: Limits) -> Limits {
    Limits { step_input: usize::MAX, ..limits }
}

/// The run `packed` in a file read from a store of `capacity` chunks of
/// 1 KiB with none of them loaded, and the run's input, a handle onto it.
fn in_a_chunk_store(packed: &[u8], capacity: usize) -> (Vec<u8>, crate::source::ChunkStore, RunInput) {
    use crate::source::Source;
    let (file, run_at) = in_a_file(packed);
    let store = crate::source::ChunkStore::new(file.len() as u64, 1024, capacity);
    let input = RunInput::new(store.share().expect("a store hands out a handle"), run_at as u64, packed.len() as u64);
    (file, store, input)
}

/// What a pass over a store came to, fed the chunks each step named.
struct Fed {
    got: Result<Decoded, Refusal>,
    /// The chunks each step that waited named.
    waits: Vec<Vec<u64>>,
    /// The most bytes of the run the pass held between two steps.
    most_held: usize,
}

/// Step `pass` to its end, feeding `store` the chunks of `file` each step
/// names. A chunk asked for more than twice is the wait that never ends.
fn fed_pass(mut pass: Indexing, store: &mut crate::source::ChunkStore, file: &[u8], out_bytes: u64, in_bits: u64) -> Fed {
    let mut times = std::collections::BTreeMap::<u64, usize>::new();
    let (mut waits, mut most_held) = (Vec::new(), 0);
    for _ in 0..1_000_000 {
        let stepped = pass.step(out_bytes, in_bits);
        most_held = most_held.max(pass.holds());
        match stepped {
            Ok(Stepped::Working) => {}
            Ok(Stepped::NeedsInput(missing)) => {
                assert!(!missing.is_empty(), "a step that waits on nothing");
                for m in &missing {
                    let n = times.entry(m.chunk).or_default();
                    *n += 1;
                    assert!(*n <= 2, "chunk {} asked for {n} times", m.chunk);
                    let at = m.chunk as usize * 1024;
                    store.insert(m.chunk, file[at..file.len().min(at + 1024)].into());
                }
                waits.push(missing.iter().map(|m| m.chunk).collect());
            }
            Ok(Stepped::Done(d)) => return Fed { got: Ok(d), waits, most_held },
            Err(why) => return Fed { got: Err(why), waits, most_held },
        }
    }
    panic!("no end to the steps");
}

/// What two indexes of the same run have to agree on, read from each: the
/// checkpoints with their windows and copies of block headers, the block
/// starts with what their headers say, the sums, and the trace of a span in
/// every few. `ours` reads its run from a copy here, and `whole` from
/// wherever it was made.
fn same_index(ours: &mut Seekable, whole: &Seekable, packed: &[u8], at: &str) {
    assert_eq!(ours.marks(), whole.marks(), "{at}");
    for (k, (a, b)) in ours.checkpoints.iter().zip(&whole.checkpoints).enumerate() {
        assert!(a.window() == b.window(), "{at}: checkpoint {k}'s window");
        assert_eq!(a.head(), b.head(), "{at}: checkpoint {k}'s copy of its block's header");
        assert_eq!(a.own(), b.own(), "{at}: checkpoint {k}");
    }
    assert_eq!((&ours.blocks, ours.blocks_end), (&whole.blocks, whole.blocks_end), "{at}");
    assert_eq!((ours.len(), ours.crc32(), ours.adler32()), (whole.len(), whole.crc32(), whole.adler32()), "{at}");
    assert_eq!(ours.held_bytes(), whole.held_bytes(), "{at}");
    ours.set_input(RunInput::held(Arc::new(packed.to_vec())));
    for k in (0..ours.spans()).step_by((ours.spans() / 5).max(1)) {
        assert!(shape(&ours.span_trace(k).expect("traces")) == shape(&whole.span_trace(k).expect("traces")), "{at}: span {k}'s trace");
    }
    for i in (0..ours.blocks()).step_by((ours.blocks() as usize / 5).max(1)) {
        assert_eq!(ours.block(i), whole.block(i), "{at}: block {i}");
    }
}

/// The most of the run one step of a pass under `limits` reads at once.
fn window_of(codec: Codec, limits: &Limits) -> usize {
    limits.step_window(codec)
}

/// A deflate, zlib or gzip run several times longer than the chunk store
/// is indexed a window at a time from the store, each step that waits naming
/// the chunks of one window, and comes to the index a pass over the whole
/// run in memory makes: the same checkpoints, with the same windows and
/// copies of block headers, the same block starts and sums, and the same
/// traces. The pass never holds more of the run than a window and the
/// trailer. Under the cap the run comes back whole, with the same bytes and
/// trace. Runs of one block, blocks every 3,000 bytes, and blocks flushed
/// every 2,000 bytes with an empty stored block after each, told the run is
/// large and not.
#[test]
fn a_deflate_pass_reads_its_run_from_a_store_a_window_at_a_time() {
    let data = mixed(1_200_000, 211);
    let limits = in_windows();
    let window = window_of(Codec::Deflate, &limits);
    let capacity = (2 * window).div_ceil(1024) + 4;
    let raws = [miniz_oxide::deflate::compress_to_vec(&data, 6), deflate_in_blocks(&data, 3000), flushed(&data, 1, 2000)];
    for (r, raw) in raws.iter().enumerate() {
        for codec in [Codec::Deflate, Codec::Zlib, Codec::Gzip] {
            let packed = wrapped(codec, raw, &data);
            assert!(packed.len() > 2 * capacity * 1024, "{} bytes is too short for a store of {capacity} KiB", packed.len());
            for (whole_cap, told) in [(limits.whole, false), (limits.whole, true), (2 << 20, false)] {
                let limits = Limits { whole: whole_cap, ..limits };
                let at = format!("{codec:?}, run {r}, cap {whole_cap}, told {told}");
                let start = |input: RunInput, limits: Limits| {
                    let pass = Indexing::through(codec, input, limits).expect("starts");
                    if told { pass.known_large() } else { pass }
                };
                let held = RunInput::held(Arc::new(packed.clone()));
                let (whole, _) = in_steps(start(held, one_window(limits)), u64::MAX);
                let (file, mut store, input) = in_a_chunk_store(&packed, capacity);
                let fed = fed_pass(start(input, limits), &mut store, &file, 256 << 10, (256 << 10) * 8);
                assert!(fed.waits.len() > packed.len() / window / 2, "{at}: {} waits", fed.waits.len());
                for named in &fed.waits {
                    assert!(named.len() <= window / 1024 + 3, "{at}: a step named {} chunks", named.len());
                }
                assert!(fed.most_held <= window + 8 + inflate::MAX_HEAD_BYTES, "{at}: the pass held {} bytes of the run", fed.most_held);
                match (whole.expect("indexes from memory"), fed.got.expect("indexes from the store")) {
                    (Decoded::Indexed(whole), Decoded::Indexed(mut ours)) => {
                        assert!(ours.input().owned_bytes() == 0, "{at}: the index holds a copy");
                        same_index(&mut ours, &whole, &packed, &at);
                        let mut all = vec![0u8; data.len()];
                        ours.read(0, &mut all).expect("reads");
                        assert!(all == data, "{at}: the output differs");
                    }
                    (Decoded::Whole(bytes, trace), Decoded::Whole(ours, t)) => {
                        assert!(ours == bytes && bytes == data, "{at}: the bytes differ");
                        assert!(shape(&t) == shape(&trace), "{at}: the traces differ");
                    }
                    _ => panic!("{at}: from memory and from the store disagree about holding it whole"),
                }
            }
        }
    }
}

/// A zstd run of many frames several times longer than the store, with a
/// skippable frame in front of each, is indexed a window at a time from the
/// store and comes to the index one window over the whole run makes, and
/// held whole under the cap the same bytes and trace. A skippable frame
/// longer than a window is read no further than one window into it: the
/// walk skips its contents, and the next step reads from past them.
#[test]
fn a_zstd_pass_reads_its_run_from_a_store_a_window_at_a_time() {
    let data = mixed(1_500_000, 223);
    let limits = Limits { span: 1 << 20, ..in_windows() };
    let window = window_of(Codec::Zstd, &limits);
    let capacity = (2 * window).div_ceil(1024) + 4;
    let many = zstd_of(&data, 10_000, true);
    let skip = 900_000;
    let front = zstd_of(&data[..600_000], 10_000, false);
    let long = [front.clone(), skippable_frame(skip, 227), zstd_of(&data[600_000..], 10_000, true)].concat();
    for (packed, what) in [(&many, "many frames"), (&long, "a long skippable frame")] {
        assert!(packed.len() > 2 * capacity * 1024, "{what}: {} bytes is too short", packed.len());
        for (whole_cap, told) in [(limits.whole, false), (limits.whole, true), (4 << 20, false)] {
            let limits = Limits { whole: whole_cap, ..limits };
            let at = format!("{what}, cap {whole_cap}, told {told}");
            let start = |input: RunInput, limits: Limits| {
                let pass = Indexing::through(Codec::Zstd, input, limits).expect("starts");
                if told { pass.known_large() } else { pass }
            };
            let (whole, _) = in_steps(start(RunInput::held(Arc::new(packed.clone())), one_window(limits)), u64::MAX);
            let (file, mut store, input) = in_a_chunk_store(packed, capacity);
            let run_at = in_a_file(packed).1;
            let fed = fed_pass(start(input, limits), &mut store, &file, 256 << 10, (256 << 10) * 8);
            // Past the first window into the skippable frame, and short of
            // its last chunk, nothing is read.
            let inside = ((run_at + front.len() + window) / 1024 + 1) as u64..((run_at + front.len() + skip) / 1024) as u64;
            for named in &fed.waits {
                assert!(named.len() <= window / 1024 + 3, "{at}: a step named {} chunks", named.len());
                if what != "many frames" {
                    assert!(named.iter().all(|c| !inside.contains(c)), "{at}: a chunk inside the skippable frame was read");
                }
            }
            assert!(fed.most_held <= window, "{at}: the pass held {} bytes of the run", fed.most_held);
            match (whole.expect("indexes from memory"), fed.got.expect("indexes from the store")) {
                (Decoded::Indexed(whole), Decoded::Indexed(mut ours)) => {
                    same_index(&mut ours, &whole, packed, &at);
                    reads_back(&ours, &data, 229);
                }
                (Decoded::Whole(bytes, trace), Decoded::Whole(ours, t)) => {
                    assert!(ours == bytes && bytes == data, "{at}: the bytes differ");
                    assert!(shape(&t) == shape(&trace), "{at}: the traces differ");
                }
                _ => panic!("{at}: from memory and from the store disagree about holding it whole"),
            }
        }
    }
}

/// A run of empty deflate blocks makes no output, and a step told it may
/// read as much of the run as it likes still stops by the end of its
/// window: a step over empty blocks reads no more than its share of input
/// and one empty block past it, and any step no more than its share and one
/// stored block, which a step is not stopped in. The index is the one a pass
/// over the whole run makes.
#[test]
fn a_step_over_empty_blocks_stops_by_the_end_of_its_window() {
    let data = mixed(100_000, 233);
    let blocks = empty_blocks(400_000);
    let raw = deflate_around(&[&data[..20_000], &data[20_000..]], &blocks);
    let limits = in_windows();
    let mut pass = Indexing::new(Codec::Deflate, Arc::new(raw.clone()), limits).expect("starts");
    let (mut was, mut steps, mut idle) = (pass.progress(), 0, 0);
    let mut ours = loop {
        steps += 1;
        match pass.step(u64::MAX, u64::MAX).expect("steps") {
            Stepped::Done(Decoded::Indexed(s)) => break s,
            Stepped::Done(Decoded::Whole(..)) => panic!("came back whole"),
            Stepped::NeedsInput(m) => panic!("a copy in memory missing {m:?}"),
            Stepped::Working => {}
        }
        let now = pass.progress();
        let read = now.read_bits - was.read_bits;
        assert!(read <= (limits.step_input + inflate::DEFLATE_MARGIN) as u64 * 8, "step {steps} read {read} bits");
        if now.unpacked == was.unpacked {
            idle += 1;
            assert!(read <= (limits.step_input as u64 + 2) * 8, "step {steps} read {read} bits and made nothing");
        }
        was = now;
    };
    // The steps that start or end the empty blocks make output as well.
    assert!(idle + 2 >= blocks.len() / limits.step_input, "{steps} steps, {idle} of them over empty blocks alone");
    let whole = indexed(Codec::Deflate, &raw, one_window(limits));
    same_index(&mut ours, &whole, &raw, "empty blocks");
}

/// A gzip whose trailer disagrees with what came out, a zlib whose sum does,
/// and a gzip whose size in its trailer is wrong about the run being large,
/// are refused as failed when the pass reads them from a store, as from
/// memory. So is a run that is not the wrapper it is said to be, at the
/// first step.
#[test]
fn a_wrong_trailer_read_from_a_store_is_refused() {
    let data = mixed(400_000, 239);
    let window = window_of(Codec::Deflate, &in_windows());
    let capacity = (2 * window).div_ceil(1024) + 4;
    let mut gzip = gzip_of(&data, 6);
    let n = gzip.len();
    gzip[n - 8] ^= 1;
    let mut zlib = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
    let n = zlib.len();
    zlib[n - 1] ^= 1;
    let short = mixed(50_000, 241);
    let mut lying = wrapped(Codec::Gzip, &deflate_in_blocks(&short, 3000), &short);
    let n = lying.len();
    lying[n - 4..].copy_from_slice(&(1u32 << 20).to_le_bytes());
    let not_gzip = [&[0x1f, 0x8c][..], &gzip[2..]].concat();
    let runs = [(Codec::Gzip, &gzip, "a wrong CRC"), (Codec::Zlib, &zlib, "a wrong Adler"), (Codec::Gzip, &lying, "a wrong size"), (Codec::Gzip, &not_gzip, "not a gzip")];
    for (codec, packed, what) in runs {
        let (file, mut store, input) = in_a_chunk_store(packed, capacity);
        let fed = fed_pass(Indexing::through(codec, input, in_windows()).expect("starts"), &mut store, &file, 256 << 10, (256 << 10) * 8);
        assert_eq!(fed.got.err(), Some(Refusal::Failed), "{what}");
        assert_eq!(decode_or_index_with(codec, Arc::new(packed.clone()), in_windows()).err(), Some(Refusal::Failed), "{what}, from memory");
    }
}

/// A gzip whose header has an extra field of 60 KiB, a name, and a comment
/// of 100 KiB is read by the first step, which reads more of the run's head
/// until the header ends there, and is indexed from a store with room for
/// it, the header included. One whose header is longer than half of what
/// the store keeps is refused as too much to keep in memory at once, and
/// indexes from memory.
#[test]
fn a_long_gzip_header_is_read_from_a_store_with_room_for_it() {
    let data = mixed(300_000, 251);
    let raw = deflate_in_blocks(&data, 3000);
    let header = |comment: usize| {
        let mut h = vec![0x1f, 0x8b, 8, 0x04 | 0x08 | 0x10, 0, 0, 0, 0, 0, 3];
        let extra = noise(60 << 10, 253);
        h.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        h.extend_from_slice(&extra);
        h.extend_from_slice(b"name.fits\0");
        h.extend(std::iter::repeat_n(b'c', comment));
        h.push(0);
        h
    };
    let tail = [&raw[..], &crc32(&data).to_le_bytes(), &(data.len() as u32).to_le_bytes()].concat();
    let limits = in_windows();
    for (comment, opens) in [(100 << 10, true), (300 << 10, false)] {
        let packed = [header(comment), tail.clone()].concat();
        let (file, mut store, input) = in_a_chunk_store(&packed, 400);
        let fed = fed_pass(Indexing::through(Codec::Gzip, input, limits).expect("starts"), &mut store, &file, 256 << 10, (256 << 10) * 8);
        let whole = indexed(Codec::Gzip, &packed, one_window(limits));
        match opens {
            true => {
                let Ok(Decoded::Indexed(mut ours)) = fed.got else { panic!("a comment of {comment}: not indexed") };
                same_index(&mut ours, &whole, &packed, &format!("a comment of {comment}"));
            }
            false => assert_eq!(fed.got.err(), Some(Refusal::TooMuchAtOnce), "a comment of {comment}"),
        }
    }
}

/// A pass reads its run a window at a time from a source that answers any
/// read whole, or from one that keeps at least twice what a step reads at
/// once; any other pass reads a copy of the run. In the browser the store
/// keeps 32 MiB, and a step reads at most about 384 KiB. An xz stream's
/// layout reads a copy whatever the source.
#[test]
fn a_pass_reads_in_windows_only_from_a_store_with_room_for_two() {
    let limits = Limits::default();
    let packed = vec![0u8; 1000];
    for (codec, window) in [(Codec::Gzip, STEP_INPUT + inflate::DEFLATE_MARGIN), (Codec::Zstd, STEP_INPUT + crate::codec::frames::ZSTD_MARGIN)] {
        assert_eq!(window_of(codec, &limits), window);
        assert!(reads_in_windows(codec, &RunInput::held(Arc::new(packed.clone())), &limits), "{codec:?} from memory");
        assert!(reads_in_windows(codec, &from_a_store(&packed, 2 * window as u64), &limits), "{codec:?}");
        assert!(!reads_in_windows(codec, &from_a_store(&packed, 2 * window as u64 - 1), &limits), "{codec:?}");
    }
    assert!(!reads_in_windows(Codec::Xz, &RunInput::held(Arc::new(packed)), &limits));
}

/// A pass that reads its run through a handle is held to `Limits::input`:
/// a run of exactly that length is indexed from a store, and one a byte
/// longer is refused as `TooLargeInput` when the pass is made, before any of
/// it is read. A pass that reads a copy is held to `Limits::copy_input`
/// instead, and says `TooLarge` past it, whatever `Limits::input` is; an xz
/// stream's layout always reads a copy. The default limits are 512 MiB and
/// 64 MiB.
#[test]
fn a_pass_is_held_to_the_limit_of_the_way_it_reads_its_run() {
    assert_eq!((Limits::default().input, Limits::default().copy_input), (512 << 20, 64 << 20));
    let data = mixed(300_000, 307);
    let limits = in_windows();
    for codec in [Codec::Zlib, Codec::Zstd] {
        let packed = match codec {
            Codec::Zstd => zstd_of(&data, 30_000, true),
            _ => miniz_oxide::deflate::compress_to_vec_zlib(&data, 6),
        };
        let len = packed.len();
        let capacity = (2 * window_of(codec, &limits)).div_ceil(1024) + 4;
        let (file, mut store, input) = in_a_chunk_store(&packed, capacity);
        let past = Limits { input: len as u64 - 1, ..limits };
        assert_eq!(Indexing::through(codec, input.clone(), past).err(), Some(Refusal::TooLargeInput), "{codec:?}");
        let at = Limits { input: len as u64, copy_input: len - 1, ..limits };
        let fed = fed_pass(Indexing::through(codec, input.clone(), at).expect("a run of the limit starts"), &mut store, &file, u64::MAX, u64::MAX);
        assert!(matches!(fed.got, Ok(Decoded::Indexed(_))), "{codec:?}: {:?}", fed.got.err());

        let copy = Arc::new(packed.clone());
        let short = Limits { copy_input: len - 1, ..limits };
        assert_eq!(Indexing::from_copy(codec, copy.clone(), input.clone(), short).err(), Some(Refusal::TooLarge), "{codec:?}");
        assert_eq!(Indexing::new(codec, copy.clone(), short).err(), Some(Refusal::TooLarge), "{codec:?}");
        assert_eq!(decode_or_index_with(codec, copy.clone(), short).err(), Some(Refusal::TooLarge), "{codec:?}");
        assert!(Indexing::from_copy(codec, copy.clone(), input, past).is_ok(), "{codec:?}");
        assert!(matches!(decode_or_index_with(codec, copy, past), Ok(Decoded::Indexed(_))), "{codec:?}");
    }
    let xz = Arc::new(xz_pieces(&data, 60_000, Some(20_000)));
    let short = Limits { span: 1 << 20, copy_input: xz.len() - 1, input: 0, ..small() };
    assert_eq!(decode_or_index_with(Codec::Xz, xz.clone(), short).err(), Some(Refusal::TooLarge));
    assert!(matches!(decode_or_index_with(Codec::Xz, xz, Limits { copy_input: usize::MAX, ..short }), Ok(Decoded::Indexed(_))));
}
