//! pxu, the way Picotron writes a userdata into a POD.
//!
//! `pod()` takes flags saying how to encode a value, and 0x1 is pxu, which the
//! manual describes only as encoding userdata "in a compressed (RLE-style)
//! form". A pxu run is not a container round a POD: it sits inside the POD's
//! text, in the place a `userdata()` value would otherwise be written, and a
//! reader scans the text for `pxu\0` and swaps each run it finds for the
//! userdata it decodes to.
//!
//! ## The run
//!
//! - `pxu\0`.
//! - Two bytes of flags, little-endian. The low four bits are the element
//!   type, 3 for `u8` and 12 for `i16`; 0x40 says a height follows the width;
//!   0x800 says both are written as four bytes rather than one; and the top
//!   four bits are the compression, 1 for none, 2 for move-to-front and 8 for
//!   run length.
//! - The width, and the height when 0x40 said so. One byte each, or four each
//!   when 0x800 said so. A userdata with no height is one row.
//! - Then `width * height` elements, however the compression says.
//!
//! With no compression the elements are written out one after another and the
//! run is a straight copy. The other two share one encoding and differ only in
//! how many bits of a token are an index: a byte of `bits` follows the sizes,
//! 4 for move-to-front and 0 for run length. A token byte then carries an
//! index in its low `bits` and a count above it. An index of all ones is not
//! an index: the element follows the token, written out. Any other index names
//! an element written before, out of a table of the last fifteen, which the
//! index also moves to the front of. A count of all ones is not the whole
//! count either: bytes follow it and are added on until one of them is not
//! 255. Whatever the element turns out to be, the count says how many copies
//! of it to write.
//!
//! Run length is the same encoding with the index gone: every token is a count
//! and every element is written out. Which is what the manual means by
//! RLE-style, and Picotron writes it for `i16`; a `u8` userdata gets
//! move-to-front instead.
//!
//! What comes out is the elements as bytes, one byte each for `u8` and two
//! little-endian bytes each for `i16`, which is the userdata as it sits in
//! memory.
//!
//! ## Where it is read from
//!
//! Nothing in a template reaches this yet. A [`crate::template::Ty::Decoded`]
//! field is told how long its run is before the run is opened, and a pxu run
//! says its length nowhere: with either compression on, the last token is
//! reached only by decoding every token before it. So a pxu run can be opened
//! from a POD's text but not laid out in it, and neither Picotron cartridge in
//! the sample collection holds one to lay out. A run handed more bytes than it
//! needs is still read: what is left over is one unnamed step at the end, so
//! the trace still tiles.
//!
//! Read out of `read_pxu` and `write_pxu` in `picotron_fs.py` of
//! thisismypassport/shrinko8, which are each other's inverse down to the
//! table's least recently used slot, so the encoding here is the one that
//! round-trips rather than one read off a decoder alone.

use super::{BlockKind, Refusal, StepField, StepKind, Trace, TraceBuilder, CAP_BYTES};

/// What a run opens with.
pub const MAGIC: &[u8] = b"pxu\0";

/// The element types that carry a compression. `u8` is one byte and `i16` is
/// two; shrinko8 refuses the rest, having never seen one.
const TYPE_U8: u16 = 3;
const TYPE_I16: u16 = 12;

/// The compressions, out of the top four bits of the flags.
const NONE: u16 = 1;
const MTF: u16 = 2;
const RLE: u16 = 8;

/// Whether a height follows the width, and whether the sizes are four bytes
/// rather than one.
const HAS_HEIGHT: u16 = 0x40;
const LONG_SIZE: u16 = 0x800;

/// A reader over the run, which is also where the trace's input positions come
/// from: everything is byte aligned, so a position in bits is a position in
/// bytes times eight.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8, Refusal> {
        let b = *self.data.get(self.at).ok_or(Refusal::Failed)?;
        self.at += 1;
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16, Refusal> {
        Ok(u16::from_le_bytes([self.u8()?, self.u8()?]))
    }

    fn u32(&mut self) -> Result<u32, Refusal> {
        Ok(u32::from_le_bytes([self.u8()?, self.u8()?, self.u8()?, self.u8()?]))
    }

    /// One element, and its bytes as they go into the output.
    fn element(&mut self, width: usize) -> Result<u16, Refusal> {
        match width {
            1 => Ok(self.u8()? as u16),
            _ => self.u16(),
        }
    }

    fn bits(&self) -> u64 {
        self.at as u64 * 8
    }
}

/// Open a pxu run.
pub fn pxu(data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    let mut r = Reader { data, at: 0 };
    let mut out: Vec<u8> = Vec::new();
    let mut tr = TraceBuilder::default();

    if !data.starts_with(MAGIC) {
        return Err(Refusal::Failed);
    }
    tr.open_block(0, 0);
    tr.push(0, 0, StepKind::Header(StepField::FrameHeader, 0));
    r.at = MAGIC.len();

    let at = r.bits();
    let flags = r.u16()?;
    tr.push(at, 0, StepKind::Header(StepField::PxuFlags, flags as u32));

    let width = {
        let at = r.bits();
        let w = if flags & LONG_SIZE != 0 { r.u32()? } else { r.u8()? as u32 };
        tr.push(at, 0, StepKind::Header(StepField::PxuWidth, w));
        w
    };
    let height = if flags & HAS_HEIGHT != 0 {
        let at = r.bits();
        let h = if flags & LONG_SIZE != 0 { r.u32()? } else { r.u8()? as u32 };
        tr.push(at, 0, StepKind::Header(StepField::PxuHeight, h));
        h
    } else {
        1
    };

    let width_bytes = match flags & 0xf {
        TYPE_U8 => 1usize,
        TYPE_I16 => 2,
        _ => return Err(Refusal::Failed),
    };
    let count = (width as u64) * (height as u64);
    let bytes_out = count * width_bytes as u64;
    if bytes_out > CAP_BYTES as u64 {
        return Err(Refusal::TooLarge);
    }
    let bytes_out = bytes_out as usize;

    match flags >> 12 {
        NONE => {
            // Every element written out, which is the bytes as they are.
            let at = r.bits();
            let end = r.at.checked_add(bytes_out).ok_or(Refusal::Failed)?;
            let run = data.get(r.at..end).ok_or(Refusal::Failed)?;
            if !run.is_empty() {
                tr.push(at, 0, StepKind::Stored);
            }
            out.extend_from_slice(run);
            r.at = end;
        }
        compression @ (MTF | RLE) => {
            let bits = {
                let at = r.bits();
                let b = r.u8()?;
                tr.push(at, 0, StepKind::Header(StepField::PxuBits, b as u32));
                b
            };
            // Move-to-front is written with four bits of index and run length
            // with none, and shrinko8 has never seen another width. Reading
            // one anyway would be reading a format nobody writes.
            let expected = if compression == MTF { 4 } else { 0 };
            if bits != expected || bits >= 8 {
                return Err(Refusal::Failed);
            }
            tokens(&mut r, &mut tr, &mut out, bits, bytes_out, width_bytes)?;
        }
        _ => return Err(Refusal::Failed),
    }
    // A run may be handed more bytes than it takes. What is left is named as
    // input this trace does not account for, so the steps still tile.
    if r.at < data.len() {
        tr.push(r.bits(), out.len() as u64, StepKind::Opaque);
    }
    tr.close_block(data.len() as u64 * 8, out.len() as u64, BlockKind::Sequences, true);
    tr.finish_at(data.len() as u64 * 8, out.len() as u64);
    Ok((out, tr.done()))
}

/// The tokens, for either of the two compressions that have them.
///
/// `bits` is how many of a token's low bits are an index, which is none at all
/// for run length: the mask is then zero, every index is the escape, and every
/// element is written out. So the two are one loop.
fn tokens(
    r: &mut Reader,
    tr: &mut TraceBuilder,
    out: &mut Vec<u8>,
    bits: u8,
    bytes_out: usize,
    width_bytes: usize,
) -> Result<(), Refusal> {
    let mask = (1u16 << bits) - 1;
    // The count a token can hold before it needs bytes after it.
    let ext_count = 1u32 << (8 - bits);
    // The table of elements written before, and which of them was used least
    // recently. Both are `mask` long, which is one short of the indices a
    // token can name, since the last of those is the escape.
    let mut mapping: Vec<u16> = (0..mask).collect();
    let mut recent: Vec<u16> = (0..mask).collect();

    while out.len() < bytes_out {
        let at = r.bits();
        let token = r.u8()?;
        let index = token as u16 & mask;
        let value = if index == mask {
            let v = r.element(width_bytes)?;
            // The new element takes the slot of the one used least recently.
            // The table is left in the order it was: the encoder does the
            // same, which is what makes the two each other's inverse.
            if bits != 0 {
                let slot = *recent.last().ok_or(Refusal::Failed)? as usize;
                *mapping.get_mut(slot).ok_or(Refusal::Failed)? = v;
            }
            v
        } else {
            move_to_front(&mut recent, index);
            *mapping.get(index as usize).ok_or(Refusal::Failed)?
        };

        let mut copies = 1u32 + (token as u32 >> bits);
        if copies == ext_count {
            loop {
                let more = r.u8()?;
                copies += more as u32;
                if more != 0xff {
                    break;
                }
            }
        }
        let written = copies as usize * width_bytes;
        if out.len() + written > bytes_out {
            return Err(Refusal::Failed);
        }

        // The first copy is what the token and whatever followed it said, and
        // the rest is that copy again, which is a match one element back.
        let start = out.len() as u64;
        tr.push(at, start, if width_bytes == 1 { StepKind::Literal(value as u8) } else { StepKind::Stored });
        for _ in 0..copies {
            out.extend_from_slice(&value.to_le_bytes()[..width_bytes]);
        }
        if copies > 1 {
            let len = (copies - 1) * width_bytes as u32;
            tr.push(r.bits(), start + width_bytes as u64, StepKind::Match { len, dist: width_bytes as u32 });
        }
    }
    Ok(())
}

/// Move `value` to the front of the table of what was used recently, shifting
/// everything that was in front of it back one.
fn move_to_front(recent: &mut [u16], value: u16) {
    let Some(at) = recent.iter().position(|&v| v == value) else { return };
    recent[..=at].rotate_right(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header of a run: the magic, the flags, and the sizes.
    fn header(kind: u16, compression: u16, width: u8, height: Option<u8>) -> Vec<u8> {
        let mut flags = kind | (compression << 12);
        if height.is_some() {
            flags |= HAS_HEIGHT;
        }
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&flags.to_le_bytes());
        v.push(width);
        if let Some(h) = height {
            v.push(h);
        }
        v
    }

    #[test]
    fn an_uncompressed_run_is_the_elements_as_they_are() {
        let mut v = header(TYPE_U8, NONE, 4, None);
        v.extend_from_slice(&[9, 8, 7, 6]);
        let (out, trace) = pxu(&v).unwrap();
        assert_eq!(out, vec![9, 8, 7, 6]);
        assert_eq!(trace.check_tiles(), Ok(()));
    }

    /// Run length: every token is a count and every element follows it.
    #[test]
    fn run_length_writes_a_count_and_then_the_element() {
        let mut v = header(TYPE_I16, RLE, 5, None);
        v.push(0); // bits
        // Three copies of 0x0102, then two of 0xfffe.
        v.extend_from_slice(&[2, 0x02, 0x01]);
        v.extend_from_slice(&[1, 0xfe, 0xff]);
        let (out, trace) = pxu(&v).unwrap();
        assert_eq!(out, vec![0x02, 0x01, 0x02, 0x01, 0x02, 0x01, 0xfe, 0xff, 0xfe, 0xff]);
        assert_eq!(trace.check_tiles(), Ok(()));

        // Four steps: the three header fields and the bits byte produce
        // nothing, then a run of three is one element and a match of two more.
        let steps: Vec<_> = trace.steps().filter(|s| !s.out_bytes.is_empty()).collect();
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[1].kind, StepKind::Match { len: 4, dist: 2 });
    }

    /// A count of all ones is not a count: bytes follow and are added on,
    /// which is how a run longer than a token can say gets written.
    #[test]
    fn a_full_count_is_continued_in_the_bytes_after_it() {
        let mut v = header(TYPE_U8, RLE, 255, Some(2));
        v.push(0);
        // A token of all ones is 256, and the byte after the element adds 253
        // more. The element comes first: the count is finished only once the
        // token has said what is being counted.
        v.extend_from_slice(&[0xff, 7, 253]);
        // The 510th of 510.
        v.extend_from_slice(&[0, 3]);
        let (out, trace) = pxu(&v).unwrap();
        assert_eq!(out.len(), 510);
        assert_eq!(out[509], 3);
        assert!(out[..509].iter().all(|&b| b == 7));
        assert_eq!(trace.check_tiles(), Ok(()));
    }

    /// Move-to-front: an index of 15 is an element written out, and any other
    /// index is one of the fifteen kept, which using it moves to the front.
    ///
    /// A written-out element takes the slot of the one used least recently and
    /// does not itself move to the front, so two of them in a row take the
    /// same slot and the first is gone. That is what the encoder does too, so
    /// the two agree; it is only worth spelling out because it looks wrong.
    #[test]
    fn move_to_front_names_an_element_it_has_already_seen() {
        let mut v = header(TYPE_U8, MTF, 6, None);
        v.push(4); // bits
        // Written out, into slot 14, which nothing has used.
        v.extend_from_slice(&[0x0f, 100]);
        // Slot 14 twice over, which moves it to the front.
        v.extend_from_slice(&[0x1e]);
        // Written out again. Slot 13 is the least recently used one now.
        v.extend_from_slice(&[0x0f, 200]);
        // Both slots still say what they were given.
        v.extend_from_slice(&[0x0d, 0x0e]);
        let (out, trace) = pxu(&v).unwrap();
        assert_eq!(out, vec![100, 100, 100, 200, 200, 100]);
        assert_eq!(trace.check_tiles(), Ok(()));
    }

    #[test]
    fn a_run_with_bytes_after_it_names_them_rather_than_reading_them() {
        let mut v = header(TYPE_U8, NONE, 2, None);
        v.extend_from_slice(&[1, 2]);
        v.extend_from_slice(b", 3 }");
        let (out, trace) = pxu(&v).unwrap();
        assert_eq!(out, vec![1, 2]);
        assert_eq!(trace.check_tiles(), Ok(()));
        assert_eq!(trace.steps().last().unwrap().kind, StepKind::Opaque);
    }

    /// The trace is what the listing is built from, so it has to survive being
    /// read back as fields: the block's head is the five header steps, and its
    /// symbols are the elements.
    #[test]
    fn the_steps_read_back_as_the_fields_of_a_block() {
        use crate::document::Document;
        use crate::eval::Evaluator;
        use crate::source::MemSource;
        use crate::template::{Expr as E, Template, Ty as T};

        let mut v = header(TYPE_U8, MTF, 2, None);
        v.push(4);
        v.extend_from_slice(&[0x1f, 42]);

        let d = Document::new(MemSource(v));
        let t = Template::new("t", T::decoded(E::Remaining, super::super::Codec::PicotronPxu, T::bytes(E::Remaining)));
        let mut ev = Evaluator::new(t);
        assert_eq!(ev.node(&d, &[]).unwrap().type_name, "picotron pxu");
        // What came out, and then the machinery that produced it.
        assert_eq!(ev.node(&d, &[0]).unwrap().size_bits, 2 * 8);
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().type_name, "block");
        let head: Vec<_> = (0..4).map(|i| ev.node(&d, &[1, 0, i]).unwrap().name).collect();
        assert_eq!(head, vec!["frame_header", "pxu_flags", "pxu_width", "pxu_bits"]);
    }

    #[test]
    fn what_is_not_a_run_is_refused_rather_than_guessed_at() {
        assert_eq!(pxu(b"not a run").unwrap_err(), Refusal::Failed);
        // An element type nobody writes.
        let v = header(5, NONE, 1, None);
        assert_eq!(pxu(&v).unwrap_err(), Refusal::Failed);
        // A compression nobody writes.
        let v = header(TYPE_U8, 3, 1, None);
        assert_eq!(pxu(&v).unwrap_err(), Refusal::Failed);
        // Cut off before its elements.
        let v = header(TYPE_U8, NONE, 4, None);
        assert_eq!(pxu(&v).unwrap_err(), Refusal::Failed);
    }
}
