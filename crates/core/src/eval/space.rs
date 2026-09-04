//! The address spaces a file opens inside itself.
//!
//! The file is space 0, and every offset in the IR is a bit of it. A
//! [`Ty::Decoded`](crate::template::Ty::Decoded) field opens another: the
//! bytes its compressed run comes to, numbered from zero, with the fields
//! declared over them counting from there. A ROOT record's object is at `+0x0`
//! of the record's stream and at no address in the file at all.
//!
//! A space belongs to the node that opened it, so it is keyed by that node's
//! path, and it is thrown away when the memo is. Nothing here maps a decoded
//! byte back to a byte of the file: a byte of deflate output is a function of
//! every byte before it, and the honest answer to "which file byte is this" is
//! the whole run.
//!
//! Opening one is refused rather than attempted when the run is past the cap
//! or does not start on a byte, and refused after the fact when the decoder
//! will not read it. All three read as the bytes that are there, with the node
//! saying which happened; see [`crate::codec::Refusal`].

use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::codec::{Codec, Refusal, Step, Trace};
use crate::document::Document;
use crate::source::ArcSource;

/// Which address space something is a bit of. 0 is the file.
pub type SpaceId = u32;

/// What came of asking a `Decoded` node to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opened {
    /// The space it opened, which is never 0.
    Space(u32),
    Refused(Refusal),
}

/// Every space this reading has opened, and what each `Decoded` node came to.
/// A stream opened as a document of its own.
///
/// The difference between this and the nesting a `Decoded` node already does
/// in the listing: that reads the stream's fields inside the file's reading,
/// under the node that opened it. This is the stream *as a file*, with its own
/// template, its own reading and its own cursor, which is what a tab is. The
/// two coexist; a reader who only wants to see what a ROOT record holds never
/// opens one of these.
///
/// It stays connected to where it came from by the trace: `map_out` says which
/// bits of the run made a byte of this, and `map_in` the other way.
pub struct Space {
    /// This space's own number, which is what everything outside calls it by.
    pub id: SpaceId,
    /// The space the run was unpacked from. 0 is the file.
    pub parent: SpaceId,
    /// The `Decoded` node in `parent` that opened it.
    pub path: Vec<usize>,
    pub codec: Codec,
    /// What the decoded bytes turned out to be, which is either what the
    /// stream's own template said or, when that said only bytes, what
    /// `recognise` made of them.
    pub template: String,
    /// True when the template came from looking at the decoded bytes rather
    /// than from what the stream's own template declared. A gzip of a tar
    /// opens as a tar, and this is what says so.
    pub recognised: bool,
    doc: Document<ArcSource>,
    ev: super::Evaluator,
    trace: Trace,
}

impl Space {
    pub(super) fn new(
        id: SpaceId,
        parent: SpaceId,
        path: Vec<usize>,
        codec: Codec,
        bytes: Arc<Vec<u8>>,
        trace: Trace,
        template: crate::template::Template,
        recognised: bool,
    ) -> Space {
        Space {
            id,
            parent,
            path,
            codec,
            template: template.name.clone(),
            recognised,
            doc: Document::new(ArcSource(bytes)),
            ev: super::Evaluator::new(template),
            trace,
        }
    }

    /// The bytes this space holds.
    pub fn bytes(&self) -> &[u8] {
        &self.doc.source().0
    }

    pub fn len_bytes(&self) -> u64 {
        self.doc.len_bytes()
    }

    pub fn trace(&self) -> &Trace {
        &self.trace
    }

    /// Which step of the decoding produced a byte of this space.
    pub fn map_out(&self, byte: u64) -> Option<Step> {
        self.trace.map_out(byte)
    }

    /// Which step read a bit of the run this space was unpacked from, and so
    /// which bytes of this space that bit produced.
    pub fn map_in(&self, bit: u64) -> Option<Step> {
        self.trace.map_in(bit)
    }

    /// This space read as its template says: the same call a file gets.
    pub fn node(&mut self, path: &[usize]) -> super::R<super::NodeInfo> {
        let (ev, doc) = (&mut self.ev, &self.doc);
        ev.node(doc, path)
    }

    /// The reading over this space, for everything `node` does not cover.
    /// Lent with its document, since one is no use without the other.
    pub fn reading(&mut self) -> (&mut super::Evaluator, &Document<ArcSource>) {
        (&mut self.ev, &self.doc)
    }
}

#[derive(Default)]
pub(super) struct Spaces {
    /// The bytes of space `i + 1`. Space 0 is the file and is not here.
    bufs: Vec<Arc<Vec<u8>>>,
    /// What the decoder did to produce `bufs[i]`.
    traces: Vec<Trace>,
    /// What each `Decoded` node came to, so a stream is opened once however
    /// many times its children are asked for.
    opened: FxHashMap<Vec<usize>, Opened>,
}

impl Spaces {
    pub(crate) fn get(&self, path: &[usize]) -> Option<Opened> {
        self.opened.get(path).copied()
    }

    /// Keep a decoded buffer and its trace, and hand back the space it became.
    pub(super) fn add(&mut self, path: &[usize], bytes: Vec<u8>, trace: Trace) -> u32 {
        self.bufs.push(Arc::new(bytes));
        self.traces.push(trace);
        let id = self.bufs.len() as u32;
        self.opened.insert(path.to_vec(), Opened::Space(id));
        id
    }

    /// The trace of the decoding that made a space.
    pub(super) fn trace(&self, space: u32) -> Option<&Trace> {
        self.traces.get(space as usize - 1)
    }

    pub(super) fn refuse(&mut self, path: &[usize], why: Refusal) {
        self.opened.insert(path.to_vec(), Opened::Refused(why));
    }

    /// The bytes of a space. `space` is never 0 here: the file is read through
    /// the document, not through this.
    pub(super) fn buf(&self, space: u32) -> Option<&Arc<Vec<u8>>> {
        self.bufs.get(space as usize - 1)
    }

    /// How many bits a space holds.
    pub(super) fn len_bits(&self, space: u32) -> u64 {
        self.buf(space).map_or(0, |b| b.len() as u64 * 8)
    }

    /// Whether any stream has been opened at all. Most files hold none, and
    /// the sweep that drops decoded nodes should cost them nothing.
    pub(super) fn any(&self) -> bool {
        !self.opened.is_empty()
    }

    /// Start again from nothing. A decoded buffer is worked out from bytes of
    /// the file, so any change to the file or the template drops it: see
    /// `Memo::forget`.
    pub(super) fn forget(&mut self) {
        self.bufs.clear();
        self.traces.clear();
        self.opened.clear();
    }
}

#[cfg(test)]
mod tests {
    use crate::codec::StepKind;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::formats;
    use crate::source::MemSource;

    /// A file that is one zlib stream, over whatever is handed in.
    fn zlib_over(content: &[u8]) -> Document<MemSource> {
        Document::new(MemSource(miniz_oxide::deflate::compress_to_vec_zlib(content, 6)))
    }

    /// The `compressed` field of the zlib template.
    const RUN: &[usize] = &[6];

    #[test]
    fn a_stream_opens_as_a_document_of_its_own() {
        let d = zlib_over(b"hello, this is the text inside the stream");
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        let id = e.open_space(&d, 0, RUN).unwrap().expect("the stream opens");
        assert_eq!(id, 1);
        let space = e.space(id).expect("it is there");
        assert_eq!(space.parent, 0);
        assert_eq!(space.path, RUN);
        assert_eq!(space.bytes(), b"hello, this is the text inside the stream");
        // Asking again is the same space, not another copy of it.
        assert_eq!(e.open_space(&d, 0, RUN).unwrap(), Some(id));
        // And it reads: the template the stream declared says text.
        let node = e.space_mut(id).unwrap().node(&[0]).unwrap();
        assert_eq!(node.value, Value::Str("hello, this is the text inside the stream".into()));
    }

    /// A stream whose template says only "bytes" opens as whatever the bytes
    /// turn out to be. A gzip of a tar is a tar.
    #[test]
    fn bytes_that_know_what_they_are_open_as_that()
    {
        let d = zlib_over(&tar(b"notes.txt", b"a file inside a tar inside a stream"));
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        let id = e.open_space(&d, 0, RUN).unwrap().expect("the stream opens");
        let space = e.space(id).unwrap();
        assert_eq!(space.template, "tar");
        assert!(space.recognised, "the template came from the template, not from the bytes");
    }

    /// A stream inside a stream is a space beside its parent, and says which
    /// one it came out of.
    #[test]
    fn a_stream_inside_a_stream_opens_beside_it() {
        let inner = miniz_oxide::deflate::compress_to_vec_zlib(b"two deep", 6);
        let d = zlib_over(&inner);
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        let first = e.open_space(&d, 0, RUN).unwrap().expect("the outer stream opens");
        assert_eq!(e.space(first).unwrap().template, "zlib");
        let second = e.open_space(&d, first, RUN).unwrap().expect("the inner stream opens");
        assert_eq!(second, 2);
        assert_eq!(e.space(second).unwrap().parent, first);
        assert_eq!(e.space(second).unwrap().bytes(), b"two deep");
    }

    /// Every byte of a space came from a step, and every bit of the run it was
    /// unpacked from was read by one.
    #[test]
    fn the_map_runs_both_ways() {
        let text = "map me both ways. ".repeat(50).into_bytes();
        let d = zlib_over(&text);
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        let id = e.open_space(&d, 0, RUN).unwrap().unwrap();
        let space = e.space(id).unwrap();
        for byte in 0..text.len() as u64 {
            let step = space.map_out(byte).unwrap_or_else(|| panic!("byte {byte} came from nowhere"));
            assert!(step.out_bytes.contains(&byte));
            assert!(matches!(step.kind, StepKind::Literal(_) | StepKind::Match { .. } | StepKind::Stored | StepKind::Pixel));
            // And the bits it read lead back to it.
            assert_eq!(space.map_in(step.in_bits.start).map(|s| s.kind), Some(step.kind));
        }
        assert_eq!(space.map_out(text.len() as u64), None);
    }

    /// Editing the file drops the spaces: a decoded byte is worked out from
    /// bytes of the file, and a tab over one that has changed is stale.
    #[test]
    fn an_edit_to_the_file_closes_the_spaces() {
        let mut d = zlib_over(b"before");
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        assert!(e.open_space(&d, 0, RUN).unwrap().is_some());
        assert_eq!(e.spaces_open().count(), 1);
        d.overwrite_bytes(3, &[0x00]);
        e.invalidate_from(3 * 8);
        assert_eq!(e.spaces_open().count(), 0);
        assert!(e.space(1).is_none());
    }

    /// One tar member, which is a 512-byte header and the file rounded up to
    /// the next 512, then two empty blocks to end the archive.
    fn tar(name: &[u8], body: &[u8]) -> Vec<u8> {
        let mut head = [0u8; 512];
        head[..name.len()].copy_from_slice(name);
        head[100..107].copy_from_slice(b"0000644");
        head[108..115].copy_from_slice(b"0000000");
        head[116..123].copy_from_slice(b"0000000");
        let size = format!("{:011o} ", body.len());
        head[124..136].copy_from_slice(size.as_bytes());
        head[136..148].copy_from_slice(b"00000000000 ");
        head[156] = b'0';
        head[257..263].copy_from_slice(b"ustar\0");
        head[263..265].copy_from_slice(b"00");
        // The checksum is worked out with its own field read as spaces.
        head[148..156].copy_from_slice(b"        ");
        let sum: u32 = head.iter().map(|&b| b as u32).sum();
        head[148..154].copy_from_slice(format!("{sum:06o}").as_bytes());
        head[154] = 0;
        head[155] = b' ';
        let mut out = head.to_vec();
        out.extend_from_slice(body);
        out.resize(512 + body.len().next_multiple_of(512), 0);
        out.resize(out.len() + 1024, 0);
        out
    }
}
