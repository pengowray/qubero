//! The bytes a dump describes, as a file.
//!
//! Reading a dump gives back bytes at addresses. This makes those bytes
//! readable the way any other file is, so everything that reads a file reads
//! them without knowing where they came from: a template, the listing, the
//! overview, a disassembler. [`Gathered`](crate::gather::Gathered) does the
//! same thing for a file's own scattered pieces; this does it for bytes that
//! were never in the file at all, only written out in digits.
//!
//! Two things are worth saying about what it hands back.
//!
//! A dump need not be of a whole file. A screen holds nineteen lines, a
//! transcript holds two runs of a tool over different stretches, and a paste
//! holds whatever someone selected. So this covers from the first address
//! described to the end of the last, and the stretches in between that nothing
//! described read as zeros. [`DumpSource::holes`] says where they are, because
//! a reader looking at a run of zeros is owed the difference between a zero
//! somebody wrote down and a zero nobody did.
//!
//! And it owns its text. A [`Dump`] borrows the text it was read from, since
//! on the fast path a line is read when it is asked for; a file that outlives
//! the call that made it cannot. So the owned parts are kept and the borrow is
//! put back together on demand, which costs nothing: an index and a layout are
//! already owned, and the borrow is two words.

use super::{Dump, Index, Note};
use crate::gather::Extent;
use crate::source::{Missing, Source};
use crate::text::Settled;

/// The bytes a dump describes, readable as a file.
#[derive(Debug, Clone)]
pub struct DumpSource {
    text: Vec<u8>,
    layout: super::layout::Layout,
    index: Index,
    notes: Vec<Note>,
    skipped: Vec<u64>,
    /// The first address the dump describes, which is byte zero here.
    from: u64,
    len: u64,
}

impl DumpSource {
    /// Read `text` as a dump and hand back the file it describes.
    pub fn new(text: Vec<u8>) -> Option<DumpSource> {
        let (from, to) = {
            let dump = super::read(&text, 0)?;
            dump.span()?
        };
        let dump = super::read(&text, 0)?;
        let (layout, index, notes, skipped) = (dump.layout.clone(), dump.index.clone(), dump.notes.clone(), dump.skipped.clone());
        drop(dump);
        Some(DumpSource { text, layout, index, notes, skipped, from, len: to - from })
    }

    /// The dump this reads from, borrowed for as long as the answer is needed.
    pub fn dump(&self) -> Dump<'_> {
        Dump {
            layout: self.layout.clone(),
            index: self.index.clone(),
            notes: self.notes.clone(),
            skipped: self.skipped.clone(),
            text: &self.text,
            base: 0,
        }
    }

    /// The address in the described file that byte zero here stands for. A
    /// dump of the middle of a file starts where it starts, and saying so is
    /// what stops an offset here being read as an offset there.
    pub fn origin(&self) -> u64 {
        self.from
    }

    /// The text the dump was written as.
    pub fn text(&self) -> &[u8] {
        &self.text
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// The stretches nothing in the dump described, as offsets here. They read
    /// as zeros, which is what every unwritten byte reads as, and this is the
    /// only thing that tells the two apart.
    pub fn holes(&self) -> Vec<Extent> {
        let dump = self.dump();
        let mut out = Vec::new();
        let mut at = self.from;
        for e in dump.extents() {
            if e.at > at {
                out.push(Extent::new(at - self.from, e.at - at));
            }
            at = e.end().max(at);
        }
        out
    }

    /// How the text itself was read, which for a screen capture is the
    /// difference between the bytes it was drawn in and the characters
    /// something translated them to.
    pub fn reading(&self) -> Settled {
        super::lines::reading(&self.text).0
    }
}

impl Source for DumpSource {
    fn len_bytes(&self) -> u64 {
        self.len
    }

    fn read_bytes(&self, offset: u64, out: &mut [u8]) -> Vec<Missing> {
        out.fill(0);
        if out.is_empty() {
            return Vec::new();
        }
        let dump = self.dump();
        let want = self.from + offset;
        // A hole stops `read_at`, so each stretch is asked for on its own and
        // what falls between them is left as the zeros it was filled with.
        for e in dump.extents() {
            let start = e.at.max(want);
            let end = e.end().min(want + out.len() as u64);
            if start >= end {
                continue;
            }
            let at = (start - want) as usize;
            let n = (end - start) as usize;
            dump.read_at(start, &mut out[at..at + n]);
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    const DUMP: &str = "\
00000000: 0001 0203 0405 0607 0809 0a0b 0c0d 0e0f  ................
00000010: 1011 1213 1415 1617 1819 1a1b 1c1d 1e1f  ................
";

    #[test]
    fn a_dump_reads_as_the_file_it_describes() {
        let s = DumpSource::new(DUMP.as_bytes().to_vec()).unwrap();
        assert_eq!(s.len_bytes(), 32);
        assert_eq!(s.origin(), 0);
        let mut out = [0u8; 32];
        assert!(s.read_bytes(0, &mut out).is_empty());
        assert_eq!(out, std::array::from_fn::<u8, 32, _>(|i| i as u8));
        assert!(s.holes().is_empty());
    }

    /// A dump of the middle of a file starts where it starts, and an offset
    /// here is not an address there. Two lines, because one line is not a dump
    /// anything can lay out: with no second address to subtract, where the
    /// digits stop and the characters start is written nowhere.
    #[test]
    fn a_dump_of_the_middle_says_where_it_starts() {
        let text = "00001000: 4142 4344 4546 4748  ABCDEFGH\n00001008: 494a 4b4c 4d4e 4f50  IJKLMNOP\n";
        let s = DumpSource::new(text.as_bytes().to_vec()).unwrap();
        assert_eq!(s.origin(), 0x1000);
        assert_eq!(s.len_bytes(), 16);
        let mut out = [0u8; 16];
        s.read_bytes(0, &mut out);
        assert_eq!(&out, b"ABCDEFGHIJKLMNOP");
    }

    #[test]
    fn what_the_dump_left_out_reads_as_zeros_and_says_so() {
        // The short line keeps its columns, the way a tool writes one: the
        // characters start where they start on every other line, which is what
        // stops "ABCD" being read as two more bytes.
        let text = format!("{DUMP}00000030: 4142 4344                                ABCD\n");
        let s = DumpSource::new(text.into_bytes()).unwrap();
        assert_eq!(s.len_bytes(), 0x34);
        assert_eq!(s.holes(), vec![Extent::new(0x20, 0x10)]);
        let mut out = [0u8; 0x34];
        s.read_bytes(0, &mut out);
        assert_eq!(&out[0x20..0x30], &[0u8; 16], "the stretch nobody described");
        assert_eq!(&out[0x30..], b"ABCD");
    }

    #[test]
    fn anything_that_reads_a_file_reads_one_of_these() {
        let s = DumpSource::new(DUMP.as_bytes().to_vec()).unwrap();
        let doc = Document::new(s);
        assert_eq!(doc.len_bytes(), 32);
        let mut out = [0u8; 4];
        doc.read_bytes(4, &mut out);
        assert_eq!(out, [4, 5, 6, 7]);
    }
}
