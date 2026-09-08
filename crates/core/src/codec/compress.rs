//! compress: LZW codes packed low bit first, at a width that grows.
//!
//! What `compress` wrote before gzip existed, and what a `.Z` file is. The run
//! handed here is the whole file: two magic bytes, a flags byte, and then
//! nothing but codes. The template reads those three bytes as fields over the
//! same bytes, the way `xz` does, so they are one step here and are not named
//! twice.
//!
//! The table starts as the 256 single bytes, and every code after the first
//! adds one entry: the string the code before it stood for, and the byte this
//! one starts with. That is what makes an entry a *stretch of the output*
//! rather than a chain to walk back: the two are already next to each other in
//! what has been written, so a code is a copy from further back and is traced
//! as one. A reader gets `12 back 4088` where a chain would have given a
//! number that means nothing outside the decoder.
//!
//! ## The three things that make it hard
//!
//! **Codes are packed from the low bit up**, which is the opposite of how most
//! formats fill a byte and the same way deflate reads. Bits here are counted
//! in that order, which is not how Qubero addresses bits; a step's byte extent
//! is the same either way and only a highlight narrower than a byte sits at
//! the other end of its byte. See [`crate::codec::Step`].
//!
//! **Codes are written in groups of eight.** When the table outgrows the
//! current width, or a clear throws the table away, the encoder fills the rest
//! of the group with zero bits and writes it out anyway: the decoder cannot
//! know that anything changed until it has read the group that told it. The
//! grid starts again after every such break, so nine-bit codes come in groups
//! of nine bytes and the ten-bit groups after them are counted from where the
//! padding ended rather than from the front of the file. A decoder that keeps
//! one grid across a change reads every group after it at the wrong offset.
//!
//! **In block mode, code 256 clears the table.** The entries go, the width
//! goes back to nine, and the group is padded out as above. `compress` clears
//! when the table is full and has stopped paying for itself, which on a long
//! file happens more than once.
//!
//! How much a change costs is worth writing down, because it is nothing more
//! often than not. In block mode the table hands out 257 first and the code
//! that opens the stream defines no entry, so exactly 256 codes go by before
//! nine bits stop being enough -- and 256 codes is thirty-two whole groups.
//! Every width change in a block-mode file lands on a group boundary and pads
//! out to nothing. Without block mode the table starts at 256, the crossing
//! takes 257 codes, and the padding is the seven codes left of the group. A
//! clear pads by whatever is left, since it falls wherever the encoder decided
//! the table had stopped earning its keep.
//!
//! So a decoder that skips nothing decodes most short files perfectly and
//! comes apart at the first clear, and one that always skips a whole group
//! comes apart at the first width change. That is what makes a six-byte `.Z`
//! worthless as a test of this.
//!
//! ## What the trace says
//!
//! One step a code, from the first bit of the code to the last. A code under
//! 256 is the byte it names; a code above is the stretch of output it repeats,
//! as a length and how far back it starts. The clear code is the end of a
//! block, and the padding after a clear or a width change is a step of its
//! own, because those bits are real bits of the file that produced no output
//! and a reader standing on one deserves to be told so rather than shown the
//! nearest code.
//!
//! Blocks are what block mode means by the word: one block per table, from the
//! first code after a clear to the clear that ends it. A file not in block
//! mode is one block from end to end.

use crate::codec::{BlockKind, Refusal, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

/// What one of these starts with.
const MAGIC: [u8; 2] = [0x1f, 0x9d];

/// The flags byte: the widest code in the low five bits, whether the encoder
/// clears the table at the top, and two bits nobody has ever used.
const MAX_BITS: u8 = 0x1f;
const BLOCK_MODE: u8 = 0x80;
const RESERVED: u8 = 0x60;

/// The magic and the flags, which every offset in the trace counts past.
const HEADER_BITS: u64 = 3 * 8;

/// Every stream starts nine bits wide, and none goes past sixteen.
const INIT_BITS: u32 = 9;
const TOP_BITS: u32 = 16;

/// In block mode, the code that throws the table away.
const CLEAR: u32 = 256;

/// How many codes the encoder buffers before writing any of them, which is
/// what a width change has to be padded out to.
const GROUP: u64 = 8;

/// A whole `.Z` file, magic and flags included.
pub fn lzw(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    read(data, TraceBuilder::default())
}

/// The same, with a step budget the tests can reach. See
/// [`crate::codec::MAX_STEPS`].
#[cfg(test)]
fn lzw_within(data: &[u8], budget: usize) -> Result<(Vec<u8>, Trace), Refusal> {
    read(data, TraceBuilder::with_budget(budget))
}

fn read(data: &[u8], mut b: TraceBuilder) -> Result<(Vec<u8>, Trace), Refusal> {
    if data.len() < 3 || data[..2] != MAGIC {
        return Err(Refusal::Failed);
    }
    let flags = data[2];
    // Bits nobody has ever written. gzip warns about these and reads on; this
    // stops, because the evidence that a file is one of these is two bytes
    // long and a third byte that says something impossible is the only other
    // evidence there is. The header still reads as fields either way: what a
    // reader loses is the decoding, and the reason is on the screen next to
    // it.
    if flags & RESERVED != 0 {
        return Err(Refusal::Failed);
    }
    let max_bits = (flags & MAX_BITS) as u32;
    if !(INIT_BITS..=TOP_BITS).contains(&max_bits) {
        return Err(Refusal::Failed);
    }
    let block_mode = flags & BLOCK_MODE != 0;
    let codes = &data[3..];
    let end = codes.len() as u64 * 8;

    // The header, as one step that produced nothing. The template lays magic,
    // block_mode and max_bits over these same three bytes.
    b.push(0, 0, StepKind::Header(StepField::Wrapper, flags as u32));

    let mut out: Vec<u8> = Vec::new();
    // The table, from code 256 up: where each entry's string sits in the
    // output and how long it is. In block mode the first slot stands for the
    // clear code itself, which is never looked up, so that a code and its
    // index stay one apart for the life of the stream.
    let mut table: Vec<(u32, u32)> = Vec::new();
    if block_mode {
        table.push((0, 0));
    }
    let ceiling = 1u32 << max_bits;
    let mut n_bits = INIT_BITS;
    // Where the reading has got to, and where the group of eight it is in
    // began. Both are bits of the code region rather than of the file.
    let mut at = 0u64;
    let mut group = 0u64;
    // The stretch of output the code before this one produced, which is what
    // the next entry is built from and what a code naming the entry being
    // defined this step has to fall back on.
    let mut last: Option<(u32, u32)> = None;
    let mut coarse = false;
    if end >= n_bits as u64 {
        b.open_block(HEADER_BITS, 0);
    }
    loop {
        // The next entry will not fit the width, so the rest of this group is
        // padding and the code after it is a bit wider.
        if n_bits < max_bits && free(&table) >= 1 << n_bits {
            pad(&mut b, &mut at, &mut group, n_bits, out.len(), coarse);
            n_bits += 1;
        }
        // Asked after the width, never before it: a file whose last code
        // widened the codes ends in a whole group of padding, and a decoder
        // that measures what is left at the old width reads a code out of it.
        // A file cut short can leave the padding past the end, which is the
        // one way `at` gets there, and is a stream that has run out.
        if at + n_bits as u64 > end {
            break;
        }
        // Too many codes to name one at a time: keep the map at the block and
        // say in the trace that this is what happened.
        if !coarse && b.over_budget() {
            coarse = true;
            b.coarsen();
            b.push(HEADER_BITS + at, out.len() as u64, StepKind::Opaque);
        }
        let code = code_at(codes, at, n_bits);
        let start = at;
        at += n_bits as u64;
        if at - group == n_bits as u64 * GROUP {
            group = at;
        }
        if block_mode && code == CLEAR {
            // A stream that opens with a clear has no table to clear and no
            // decoder reads one.
            if last.is_none() {
                return Err(Refusal::Failed);
            }
            if !coarse {
                b.push(HEADER_BITS + start, out.len() as u64, StepKind::EndOfBlock);
            }
            // The padding belongs to the block that ended: it is the tail of
            // the group that block's last code was written into.
            pad(&mut b, &mut at, &mut group, n_bits, out.len(), coarse);
            // Once the codes have stopped being named, the blocks stop too,
            // and what is left of the file is the one block the coarsening
            // opened. This is where `inflate` and this part company: a deflate
            // block still has a header to show after its symbols are given up
            // on, and a table here has nothing but its codes, so keeping the
            // boundaries would be rows holding one step apiece saying nothing.
            if !coarse {
                b.close_block(HEADER_BITS + at, out.len() as u64, BlockKind::Sequences, false);
                b.open_block(HEADER_BITS + at, out.len() as u64);
            }
            table.clear();
            n_bits = INIT_BITS;
            continue;
        }
        // What this code stands for, as a stretch of what has been written. A
        // code under 256 is a byte standing for itself and has no stretch
        // behind it.
        let repeat = match code < 256 {
            true => None,
            false => Some(match table.get((code - 256) as usize) {
                Some(&entry) => entry,
                // The one code that names the entry this step is about to
                // define. Its string can only be the last one with its own
                // first byte after it, which is a copy that overlaps what it
                // is writing.
                None if code == free(&table) => match last {
                    Some((from, len)) => (from, len + 1),
                    None => return Err(Refusal::Failed),
                },
                None => return Err(Refusal::Failed),
            }),
        };
        let len = repeat.map_or(1, |(_, len)| len);
        if out.len() + len as usize > CAP_BYTES {
            return Err(Refusal::TooLarge);
        }
        let wrote = out.len() as u32;
        match repeat {
            None => {
                if !coarse {
                    b.push(HEADER_BITS + start, wrote as u64, StepKind::Literal(code as u8));
                }
                out.push(code as u8);
            }
            Some((from, len)) => {
                if !coarse {
                    b.push(HEADER_BITS + start, wrote as u64, StepKind::Match { len, dist: wrote - from });
                }
                for k in 0..len as usize {
                    let byte = out[from as usize + k];
                    out.push(byte);
                }
            }
        }
        // Every code but the first adds an entry, which is the string before
        // it and the byte this one begins with. After a clear that entry lands
        // on the clear code's own slot and is never looked up: the encoder
        // spends it too, and the two tables stay the same size.
        if let Some((from, len)) = last {
            if free(&table) < ceiling {
                table.push((from, len + 1));
            }
        }
        last = Some((wrote, len));
    }
    b.close_block(HEADER_BITS + at, out.len() as u64, BlockKind::Sequences, true);
    // Whatever is left is the tail of the last group, which is under a code
    // wide and is nothing.
    if at < end && !coarse {
        b.push(HEADER_BITS + at, out.len() as u64, StepKind::Header(StepField::Padding, 0));
    }
    b.finish_at(HEADER_BITS + end, out.len() as u64);
    Ok((out, b.done()))
}

/// The next code the table will hand out. Kept as the table's own length so
/// the two cannot drift: entry `i` is code `256 + i`, at the top and after a
/// clear alike.
fn free(table: &[(u32, u32)]) -> u32 {
    256 + table.len() as u32
}

/// Skip to the end of the group of eight codes being read, and say in the
/// trace that the bits skipped were padding. Whatever happens, the next group
/// starts here.
///
/// There is nothing to skip when the group ended on the code just read, which
/// the caller has already said by moving `group` up to `at`: the encoder had
/// written its buffer out and had nothing left to pad. Skipping a group anyway
/// is the mistake that reads a block-mode file wrongly from its first width
/// change on, since those land on a group boundary every time.
fn pad(b: &mut TraceBuilder, at: &mut u64, group: &mut u64, n_bits: u32, out: usize, coarse: bool) {
    if *at > *group {
        if !coarse {
            b.push(HEADER_BITS + *at, out as u64, StepKind::Header(StepField::Padding, 0));
        }
        *at = *group + n_bits as u64 * GROUP;
    }
    *group = *at;
}

/// `bits` bits at `at`, the first one read being the lowest. Sixteen bits
/// starting anywhere in a byte reach three bytes and no further, and a code
/// asked for past the end reads as zeroes, which the caller has already ruled
/// out by measuring what is left.
fn code_at(data: &[u8], at: u64, bits: u32) -> u32 {
    let i = (at / 8) as usize;
    let word = *data.get(i).unwrap_or(&0) as u32
        | (*data.get(i + 1).unwrap_or(&0) as u32) << 8
        | (*data.get(i + 2).unwrap_or(&0) as u32) << 16;
    (word >> (at % 8)) & ((1 << bits) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Step;

    // The streams below were written by the `compress` encoder in
    // `tools/make_compressed_samples.py` of the sample collection, and every
    // one of them was read back with `gzip -dc` before it was pasted here. So
    // what these assert against is what a real decompressor makes of the same
    // bytes, and not what the decoder below happens to do with them.

    /// Every byte in order and then a pangram, packed without block mode, so
    /// the table starts at 256 and the crossing into ten-bit codes takes 257
    /// codes: one past the group, leaving seven codes of padding.
    const WIDE: &[u8] = b"\x1f\x9d\x10\x00\x02\x08\x18\x40\xa0\x80\x81\x03\x08\x12\x28\x58\
        \xc0\xa0\x81\x83\x07\x10\x22\x48\x98\x40\xa1\x82\x85\x0b\x18\x32\
        \x68\xd8\xc0\xa1\x83\x87\x0f\x20\x42\x88\x18\x41\xa2\x84\x89\x13\
        \x28\x52\xa8\x58\xc1\xa2\x85\x8b\x17\x30\x62\xc8\x98\x41\xa3\x86\
        \x8d\x1b\x38\x72\xe8\xd8\xc1\xa3\x87\x8f\x1f\x40\x82\x08\x19\x42\
        \xa4\x88\x91\x23\x48\x92\x28\x59\xc2\xa4\x89\x93\x27\x50\xa2\x48\
        \x99\x42\xa5\x8a\x95\x2b\x58\xb2\x68\xd9\xc2\xa5\x8b\x97\x2f\x60\
        \xc2\x88\x19\x43\xa6\x8c\x99\x33\x68\xd2\xa8\x59\xc3\xa6\x8d\x9b\
        \x37\x70\xe2\xc8\x99\x43\xa7\x8e\x9d\x3b\x78\xf2\xe8\xd9\xc3\xa7\
        \x8f\x9f\x3f\x80\x02\x09\x1a\x44\xa8\x90\xa1\x43\x88\x12\x29\x5a\
        \xc4\xa8\x91\xa3\x47\x90\x22\x49\x9a\x44\xa9\x92\xa5\x4b\x98\x32\
        \x69\xda\xc4\xa9\x93\xa7\x4f\xa0\x42\x89\x1a\x45\xaa\x94\xa9\x53\
        \xa8\x52\xa9\x5a\xc5\xaa\x95\xab\x57\xb0\x62\xc9\x9a\x45\xab\x96\
        \xad\x5b\xb8\x72\xe9\xda\xc5\xab\x97\xaf\x5f\xc0\x82\x09\x1b\x46\
        \xac\x98\xb1\x63\xc8\x92\x29\x5b\xc6\xac\x99\xb3\x67\xd0\xa2\x49\
        \x9b\x46\xad\x9a\xb5\x6b\xd8\xb2\x69\xdb\xc6\xad\x9b\xb7\x6f\xe0\
        \xc2\x89\x1b\x47\xae\x9c\xb9\x73\xe8\xd2\xa9\x5b\xc7\xae\x9d\xbb\
        \x77\xf0\xe2\xc9\x9b\x47\xaf\x9e\xbd\x7b\xf8\xf2\xe9\xdb\xc7\xaf\
        \x9f\xbf\x7f\x74\x00\x00\x00\x00\x00\x00\x00\x00\x68\x94\x01\x42\
        \x1c\x75\xa4\x31\xc6\x1a\x20\x88\x21\xc7\x1b\x77\xb8\x01\x82\x19\
        \x6f\xe0\x01\x82\x1a\x75\xb4\x01\xc7\x1c\x20\xbc\x61\x47\x19\x72\
        \x80\x00\xa0\x80\x6c\x84\xa1\x47\x1e\x20\x90\xf1\xc6\x19";

    /// The same 256 bytes and a pangram eight times, in block mode at twelve
    /// bits, with the table cleared after the three hundredth code. Crosses
    /// into ten-bit codes on a group boundary, so that change costs nothing,
    /// and then pays for the clear.
    const BLOCK: &[u8] = b"\x1f\x9d\x8c\x00\x02\x08\x18\x40\xa0\x80\x81\x03\x08\x12\x28\x58\
        \xc0\xa0\x81\x83\x07\x10\x22\x48\x98\x40\xa1\x82\x85\x0b\x18\x32\
        \x68\xd8\xc0\xa1\x83\x87\x0f\x20\x42\x88\x18\x41\xa2\x84\x89\x13\
        \x28\x52\xa8\x58\xc1\xa2\x85\x8b\x17\x30\x62\xc8\x98\x41\xa3\x86\
        \x8d\x1b\x38\x72\xe8\xd8\xc1\xa3\x87\x8f\x1f\x40\x82\x08\x19\x42\
        \xa4\x88\x91\x23\x48\x92\x28\x59\xc2\xa4\x89\x93\x27\x50\xa2\x48\
        \x99\x42\xa5\x8a\x95\x2b\x58\xb2\x68\xd9\xc2\xa5\x8b\x97\x2f\x60\
        \xc2\x88\x19\x43\xa6\x8c\x99\x33\x68\xd2\xa8\x59\xc3\xa6\x8d\x9b\
        \x37\x70\xe2\xc8\x99\x43\xa7\x8e\x9d\x3b\x78\xf2\xe8\xd9\xc3\xa7\
        \x8f\x9f\x3f\x80\x02\x09\x1a\x44\xa8\x90\xa1\x43\x88\x12\x29\x5a\
        \xc4\xa8\x91\xa3\x47\x90\x22\x49\x9a\x44\xa9\x92\xa5\x4b\x98\x32\
        \x69\xda\xc4\xa9\x93\xa7\x4f\xa0\x42\x89\x1a\x45\xaa\x94\xa9\x53\
        \xa8\x52\xa9\x5a\xc5\xaa\x95\xab\x57\xb0\x62\xc9\x9a\x45\xab\x96\
        \xad\x5b\xb8\x72\xe9\xda\xc5\xab\x97\xaf\x5f\xc0\x82\x09\x1b\x46\
        \xac\x98\xb1\x63\xc8\x92\x29\x5b\xc6\xac\x99\xb3\x67\xd0\xa2\x49\
        \x9b\x46\xad\x9a\xb5\x6b\xd8\xb2\x69\xdb\xc6\xad\x9b\xb7\x6f\xe0\
        \xc2\x89\x1b\x47\xae\x9c\xb9\x73\xe8\xd2\xa9\x5b\xc7\xae\x9d\xbb\
        \x77\xf0\xe2\xc9\x9b\x47\xaf\x9e\xbd\x7b\xf8\xf2\xe9\xdb\xc7\xaf\
        \x9f\xbf\x7f\x70\x84\x31\xc6\x1a\x20\xb4\x91\x07\x08\x62\xbc\x81\
        \x07\x08\x77\xa4\x41\x07\x1a\x20\x98\x91\x86\x1d\x65\x80\x40\xc6\
        \x1b\x7a\x94\xe1\x06\x08\x6c\xa4\x11\x47\x1d\x6f\xc8\x01\x82\x1a\
        \x75\x9c\x31\x07\x08\x01\x0e\x58\xe0\x81\x00\x01\x00\x00\x00\x62\
        \xde\xe0\x01\x71\x27\x0d\x1d\x34\x20\xcc\xa4\xb1\x53\x06\x04\x99\
        \x37\x7a\xca\xb8\x01\xc1\x26\x4d\x9c\x3a\x6f\xe4\x80\x50\x53\xe7\
        \xcc\x1c\x10\x70\xc2\x8c\x59\x03\xa2\x4d\x1e\x10\x01\x07\x16\x3c\
        \x98\x70\x61\xc3\x87\x11\x27\x56\xbc\x98\x71\x63\xc7\x8f\x21\x47\
        \x96\x3c\x99\x92\xa0\x41\x84\x0a\x19\x3a\x84\x28\x91\xa2\x45\x8c\
        \x1a\x39\x7a\x04\x29\x92\xa4\x49\x94\x02\x7d\xb2\x0c\xfa\x92\xa8\
        \xcc\xa3\x35\x95\xe2\x6c\xba\x13\xaa\xca\x9f\x2d\x85\xc2\x2c\x3a\
        \x13\xa9\xcd\xa5\x39\x9d\xf2\x8c\xba\x12\xa8\xcb\xa1\x31\x8d\xd2\
        \x4c\x7a\x93\xa9\xce\xa7\x3d\xdb\x86\xad\x1a\xb7\x6c\xd6\xba";

    /// `aaaaa`: the second code names the entry that same code is defining.
    const KWKWK: &[u8] = b"\x1f\x9d\x90\x61\x02\x06\x04";

    /// `ababab` without block mode, where 256 is an entry like any other and
    /// not a clear.
    const PLAIN: &[u8] = b"\x1f\x9d\x10\x61\xc4\x00\x04\x08";

    /// What WIDE holds.
    fn wide_text() -> Vec<u8> {
        let mut v: Vec<u8> = (0..=255u8).collect();
        v.extend_from_slice(b"the quick brown fox jumps over the lazy dog");
        v
    }

    /// What BLOCK holds.
    fn block_text() -> Vec<u8> {
        let mut v: Vec<u8> = (0..=255u8).collect();
        v.extend_from_slice(&b"pack my box with five dozen liquor jugs ".repeat(8));
        v
    }

    /// Read a stream and ask three things of the reading rather than one: the
    /// bytes, that the steps tile the input and the output, and that the steps
    /// alone rebuild the same bytes. A decoder that loses a code often enough
    /// comes out with something plausible, and the trace is what catches it.
    fn read_it(data: &[u8]) -> (Vec<u8>, Trace) {
        let (out, trace) = lzw(data).expect("reads");
        trace.check_tiles().expect("the steps tile");
        assert_eq!(replay(&trace), out, "the trace does not replay to what the decoder wrote");
        (out, trace)
    }

    /// The output built again out of the trace alone. What a map is for is to
    /// say where a byte came from, and this is that claim made checkable.
    fn replay(trace: &Trace) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        for step in trace.steps() {
            match step.kind {
                StepKind::Literal(v) => out.push(v),
                StepKind::Match { len, dist } => {
                    let from = out.len() - dist as usize;
                    for k in 0..len as usize {
                        let byte = out[from + k];
                        out.push(byte);
                    }
                }
                // The header, the padding and the clear code, none of which
                // writes anything. A step that stopped naming its codes would
                // fail the check below, which is right: this is only ever
                // asked of a trace that named them.
                _ => {}
            }
            assert_eq!(out.len() as u64, step.out_bytes.end, "a step wrote other than it said");
        }
        out
    }

    /// Every step that is padding, in order.
    fn padding(trace: &Trace) -> Vec<Step> {
        trace.steps().filter(|s| matches!(s.kind, StepKind::Header(StepField::Padding, _))).collect()
    }

    #[test]
    fn a_wider_code_waits_for_the_next_group_and_the_rest_of_this_one_is_padding() {
        let (out, trace) = read_it(WIDE);
        assert_eq!(out, wide_text());
        let pad = padding(&trace);
        assert_eq!(pad.len(), 1, "one break in the file, so one stretch of padding");
        // 257 nine-bit codes go by before the table outgrows nine bits, and
        // the group of eight ends seven codes later.
        assert_eq!(pad[0].in_bits.start, HEADER_BITS + 257 * 9);
        assert_eq!(pad[0].in_bits.end - pad[0].in_bits.start, 7 * 9);
        assert!(pad[0].out_bytes.is_empty(), "padding that produced a byte");
        // And the code after it is wider, which is the whole point: the step
        // that follows reads ten bits.
        let after = trace.map_in(pad[0].in_bits.end).expect("a step after the padding");
        assert_eq!(after.in_bits.end - after.in_bits.start, 10);
    }

    #[test]
    fn a_clear_throws_the_table_away_and_pads_out_the_group_it_fell_in() {
        let (out, trace) = read_it(BLOCK);
        assert_eq!(out, block_text());
        // One block per table: the one the clear ended, and the one after it.
        assert_eq!(trace.blocks().len(), 2);
        assert!(trace.blocks()[0].last == false && trace.blocks()[1].last);
        // The clear is the last thing in the first block bar its padding.
        let clear = trace.steps().position(|s| s.kind == StepKind::EndOfBlock).expect("a clear code");
        let pad = padding(&trace);
        assert_eq!(pad.len(), 1, "the width change fell on a group boundary, so only the clear pads");
        assert_eq!(trace.step(clear + 1).map(|s| s.kind), Some(StepKind::Header(StepField::Padding, 0)));
        // The clear came after the three hundredth code, but the grid it is
        // measured against restarted at the width change 256 codes in, so it
        // is the forty-fifth code of that grid: five into a group of eight,
        // leaving three codes of ten bits.
        assert_eq!(pad[0].in_bits.end - pad[0].in_bits.start, 3 * 10);
        // After it the codes are nine bits again and the table is empty, so
        // the first one can only be a byte.
        let next = trace.step(clear + 2).expect("a code after the padding");
        assert_eq!(next.in_bits.end - next.in_bits.start, 9);
        assert!(matches!(next.kind, StepKind::Literal(_)), "a match into a table that was just cleared");
        assert_eq!(next.in_bits.start, trace.blocks()[1].in_bits.start);
    }

    #[test]
    fn a_code_may_name_the_entry_that_same_code_is_defining() {
        let (out, trace) = read_it(KWKWK);
        assert_eq!(out, b"aaaaa");
        // The first code is the byte, and the second names the entry the
        // first one is about to define, which can only be that byte with its
        // own first byte after it: two bytes, from one back.
        let kinds: Vec<_> = trace.steps().map(|s| s.kind).collect();
        assert_eq!(kinds[1], StepKind::Literal(b'a'));
        assert_eq!(kinds[2], StepKind::Match { len: 2, dist: 1 });
        // The third is the same code again, and by then it is in the table.
        assert_eq!(kinds[3], StepKind::Match { len: 2, dist: 3 });
    }

    #[test]
    fn without_block_mode_code_256_is_an_entry_like_any_other() {
        let (out, trace) = read_it(PLAIN);
        assert_eq!(out, b"ababab");
        let kinds: Vec<_> = trace.steps().map(|s| s.kind).collect();
        assert_eq!(kinds[1], StepKind::Literal(b'a'));
        assert_eq!(kinds[2], StepKind::Literal(b'b'));
        // Code 256, which in block mode would have cleared the table and here
        // is the entry the two codes in front of it defined.
        assert_eq!(kinds[3], StepKind::Match { len: 2, dist: 2 });
        assert_eq!(kinds[4], StepKind::Match { len: 2, dist: 4 });
        assert!(!kinds.contains(&StepKind::EndOfBlock), "a clear in a file that has no clears");
        assert_eq!(trace.blocks().len(), 1);
    }

    #[test]
    fn the_header_is_one_step_and_the_codes_start_after_it() {
        let (_, trace) = read_it(KWKWK);
        let head = trace.step(0).expect("a first step");
        assert_eq!(head.kind, StepKind::Header(StepField::Wrapper, 0x90));
        assert_eq!(head.in_bits, 0..HEADER_BITS);
        assert!(head.out_bytes.is_empty());
        // A file of nothing but a header is a file of nothing, which is what
        // compressing an empty file writes.
        let (out, trace) = lzw(b"\x1f\x9d\x90").expect("reads");
        assert!(out.is_empty());
        trace.check_tiles().expect("the steps tile");
        assert!(trace.blocks().is_empty(), "a block with no codes in it");
    }

    #[test]
    fn what_is_not_one_of_these_is_refused_rather_than_guessed_at() {
        let refused = [
            b"".as_slice(),
            b"\x1f",
            b"\x1f\x9d",
            b"not compressed",
            b"\x1f\x8b\x08",
            // Bits nobody writes, and widths outside nine to sixteen.
            b"\x1f\x9d\xb0",
            b"\x1f\x9d\x88",
            b"\x1f\x9d\x91",
            // A first code with nothing behind it: the table holds single
            // bytes and nothing else yet, so 300 names nothing.
            b"\x1f\x9d\x90\x2c\x01",
        ];
        for case in refused {
            assert_eq!(lzw(case).err(), Some(Refusal::Failed), "{case:?} was read as a stream");
        }
    }

    #[test]
    fn a_stream_cut_short_or_knocked_about_stops_rather_than_panicking() {
        for n in 0..BLOCK.len() {
            let _ = lzw(&BLOCK[..n]);
        }
        // Some of these are still streams, and read as something; none of
        // them may panic or run away with the memory.
        for at in (3..BLOCK.len()).step_by(7) {
            let mut bent = BLOCK.to_vec();
            bent[at] ^= 0xff;
            if let Ok((_, trace)) = lzw(&bent) {
                trace.check_tiles().expect("a trace of bent bytes still tiles");
            }
        }
    }

    #[test]
    fn too_many_codes_to_name_coarsens_rather_than_filling_memory() {
        let (out, trace) = read_it(BLOCK);
        assert!(!trace.coarse(), "a stream this size does not reach the real budget");
        // The same stream with a budget it does reach. The bytes are the same
        // bytes and the trace still tiles; what is lost is only the naming.
        let (coarse_out, coarse) = lzw_within(BLOCK, 50).expect("reads");
        assert_eq!(coarse_out, out, "coarsening changed the bytes");
        coarse.check_tiles().expect("a coarse trace still tiles");
        assert!(coarse.coarse(), "the budget of 50 was not reached");
        assert!(coarse.len() < trace.len(), "coarsening kept as many steps as naming them");
        assert!(coarse.steps().any(|s| s.kind == StepKind::Opaque));
        for byte in (0..out.len() as u64).step_by(37) {
            assert!(coarse.map_out(byte).is_some(), "byte {byte} came from nowhere");
        }
    }
}
