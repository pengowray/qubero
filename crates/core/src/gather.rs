//! Reading a file's scattered parts as the one stream they stand for.
//!
//! A lot of formats keep a thing in pieces. A UF2 carries a program in blocks
//! of 256 bytes, each with its own header and its own address. A SQLite record
//! too big for its page keeps what fits and puts the rest on a chain of
//! overflow pages elsewhere. A CAB folder's files run across block boundaries.
//! A filesystem's file is a chain of clusters. In every case the file holds the
//! pieces and something else in the file holds the order, and the thing the
//! pieces make is not written down anywhere as a run of bytes.
//!
//! Until now a template could not describe such a thing, because a template
//! describes bytes where they sit and these bytes are not anywhere. So the
//! formats that have this stop at the boundary and say so: `sqlite` reads the
//! part of a payload that stayed on its page and the number of the page the
//! rest went to, and stops; `uf2` reads each block's payload as bytes and does
//! not decode the program, because an instruction that begins in one block and
//! ends in the next would be read as two wrong ones.
//!
//! This is the missing piece, and it is deliberately small. [`Gathered`] is a
//! [`Source`] like any other: it holds another source and a list of the runs to
//! take from it, and answers reads against the concatenation. Anything that
//! reads a file can read one of these without knowing it is not a file, so a
//! whole template, or a disassembler, or the byte panel, works unchanged.
//!
//! What it does not do is pretend the pieces are contiguous when someone asks
//! where they are. [`Gathered::origin`] turns a stretch of the assembled stream
//! back into the stretches of the file it came from, which is a list rather
//! than one range, because a field may sit across a join. That is the honest
//! answer and it is the one a reader needs: an instruction split over two UF2
//! blocks really is in two places, and saying so is better than picking one.

use crate::source::{Missing, Source};

/// One run of the original, as a byte offset and a length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub at: u64,
    pub len: u64,
}

impl Extent {
    pub fn new(at: u64, len: u64) -> Extent {
        Extent { at, len }
    }

    pub fn end(self) -> u64 {
        self.at + self.len
    }
}

/// A file's parts, read as the stream they make.
///
/// The order is the order given. That matters: the parts of a thing are often
/// not in the file in the order they belong in, and it is the format that says
/// which order they belong in. A UF2's blocks are usually in address order and
/// are not required to be; a chain of overflow pages is in no order at all
/// until the chain has been followed.
pub struct Gathered<S> {
    inner: S,
    extents: Vec<Extent>,
    /// Where each extent begins in the assembled stream, so that a read can
    /// find the extent it lands in without walking the list. One longer than
    /// `extents`, ending with the whole length.
    starts: Vec<u64>,
}

impl<S: Source> Gathered<S> {
    /// Gather `extents` from `inner`. Runs that fall outside the source, or
    /// that are empty, are dropped rather than read as zeroes: a stream with a
    /// hole in it that nothing marks would be read as a program with a hole in
    /// it that nothing marks.
    pub fn new(inner: S, extents: impl IntoIterator<Item = Extent>) -> Gathered<S> {
        let end_of_file = inner.len_bytes();
        let extents: Vec<Extent> =
            extents.into_iter().filter(|e| e.len > 0 && e.end() <= end_of_file).collect();
        let mut starts = Vec::with_capacity(extents.len() + 1);
        let mut total = 0;
        for extent in &extents {
            starts.push(total);
            total += extent.len;
        }
        starts.push(total);
        Gathered { inner, extents, starts }
    }

    /// The runs this was made of, in the order they were given.
    pub fn extents(&self) -> &[Extent] {
        &self.extents
    }

    /// The file this was gathered from.
    pub fn source(&self) -> &S {
        &self.inner
    }

    /// Where a stretch of the assembled stream came from.
    ///
    /// The answer is a list because a stretch may cross a join, and both sides
    /// of the join are where it is. Adjacent runs of the file are merged, so a
    /// stream assembled from pieces that happened to be contiguous answers with
    /// the one range a reader would expect.
    pub fn origin(&self, at: u64, len: u64) -> Vec<Extent> {
        let mut out: Vec<Extent> = Vec::new();
        for (extent, start) in self.extents.iter().zip(&self.starts) {
            let (from, to) = (at.max(*start), (at + len).min(start + extent.len));
            if from >= to {
                continue;
            }
            let piece = Extent::new(extent.at + (from - start), to - from);
            match out.last_mut() {
                Some(last) if last.end() == piece.at => last.len += piece.len,
                _ => out.push(piece),
            }
        }
        out
    }

    /// Which extent holds a byte of the assembled stream, by bisection.
    fn extent_at(&self, at: u64) -> usize {
        match self.starts.binary_search(&at) {
            Ok(i) => i,
            Err(i) => i - 1,
        }
    }
}

impl<S: Source> Source for Gathered<S> {
    fn len_bytes(&self) -> u64 {
        *self.starts.last().unwrap_or(&0)
    }

    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        let mut missing: Vec<Missing> = Vec::new();
        let mut done = 0usize;
        let mut at = offset;
        while done < out.len() {
            let index = self.extent_at(at);
            let Some(extent) = self.extents.get(index) else {
                // Past the end of everything gathered. The trait's callers do
                // not read here, and zeroes are what an unloaded chunk reads
                // as, so this stays quiet rather than panicking.
                out[done..].fill(0);
                break;
            };
            let within = at - self.starts[index];
            let take = (extent.len - within).min((out.len() - done) as u64) as usize;
            missing.extend(self.inner.read_bytes(extent.at + within, &mut out[done..done + take]));
            done += take;
            at += take as u64;
        }
        // The chunks are the underlying file's, so several extents landing in
        // one chunk report it several times.
        missing.sort_by_key(|m| m.chunk);
        missing.dedup();
        missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;

    fn file() -> MemSource {
        MemSource((0..64u8).collect())
    }

    fn read(g: &Gathered<MemSource>, at: u64, len: usize) -> Vec<u8> {
        let mut out = vec![0; len];
        g.read_bytes(at, &mut out);
        out
    }

    /// The pieces read as one stream, and a read that crosses a join gets both
    /// sides of it.
    #[test]
    fn the_pieces_read_as_the_stream_they_make() {
        let g = Gathered::new(file(), [Extent::new(10, 4), Extent::new(30, 4)]);
        assert_eq!(g.len_bytes(), 8);
        assert_eq!(read(&g, 0, 8), vec![10, 11, 12, 13, 30, 31, 32, 33]);
        // A read that starts and ends inside one piece.
        assert_eq!(read(&g, 5, 2), vec![31, 32]);
        // And one that straddles the join, which is the whole point.
        assert_eq!(read(&g, 3, 2), vec![13, 30]);
    }

    /// The order given is the order read, whatever order the pieces are in.
    #[test]
    fn the_order_is_the_one_the_format_gave() {
        let g = Gathered::new(file(), [Extent::new(30, 2), Extent::new(10, 2)]);
        assert_eq!(read(&g, 0, 4), vec![30, 31, 10, 11]);
    }

    /// Where a stretch of the stream came from, which is a list when it sits
    /// across a join.
    #[test]
    fn a_stretch_says_which_parts_of_the_file_it_is() {
        let g = Gathered::new(file(), [Extent::new(10, 4), Extent::new(30, 4)]);
        assert_eq!(g.origin(0, 4), vec![Extent::new(10, 4)]);
        assert_eq!(g.origin(1, 2), vec![Extent::new(11, 2)]);
        assert_eq!(g.origin(3, 2), vec![Extent::new(13, 1), Extent::new(30, 1)]);
        assert_eq!(g.origin(0, 8), vec![Extent::new(10, 4), Extent::new(30, 4)]);
    }

    /// Pieces that happen to sit next to each other in the file answer as the
    /// one range they are, rather than as the several the list held.
    #[test]
    fn parts_that_touch_are_reported_as_one() {
        let g = Gathered::new(file(), [Extent::new(10, 4), Extent::new(14, 4)]);
        assert_eq!(g.origin(0, 8), vec![Extent::new(10, 8)]);
    }

    /// A run that reaches past the end of the file is not gathered: reading it
    /// would be reading zeroes nobody wrote, and a stream with invented bytes
    /// in the middle is worse than a shorter stream.
    #[test]
    fn a_run_off_the_end_is_left_out() {
        let g = Gathered::new(file(), [Extent::new(60, 4), Extent::new(62, 4), Extent::new(0, 0)]);
        assert_eq!(g.extents(), [Extent::new(60, 4)]);
        assert_eq!(g.len_bytes(), 4);
    }

    /// A document reads one of these the way it reads a file, which is the
    /// point: nothing downstream has to know.
    #[test]
    fn a_gathered_stream_is_a_source_like_any_other() {
        use crate::document::Document;
        let g = Gathered::new(file(), [Extent::new(0, 2), Extent::new(62, 2)]);
        let doc = Document::new(g);
        assert_eq!(doc.len_bits(), 4 * 8);
    }
}
