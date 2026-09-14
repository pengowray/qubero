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
//!
//! A [`Ty::Stitched`](crate::template::Ty::Stitched) field opens a space too,
//! and that one is not a buffer: it is a table of the parts it is joined from,
//! read a part at a time, with the parts that have to be unpacked kept in a
//! small cache rather than all at once. See [`Backing`] and `stitch.rs`.

use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::codec::{Codec, Refusal, Step, Trace};
use crate::document::Document;
use crate::source::ArcSource;
use crate::template::Template;

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
/// bits of the run made a byte of this, and `map_in` the other way. The trace
/// counts its bits from the start of the run, and the run is wherever the
/// stream's field is, after a gzip's header or inside a PNG's IDAT, so `run`
/// says where that is: a step's place in the file is the run's place plus the
/// step's bits, and `map_in` takes a bit of the file and takes the run's place
/// off before it asks the trace.
///
/// Its fields are read where the stream was declared, unless the template came
/// from looking at the bytes. What a stream holds is often sized, counted or
/// picked by fields outside it: a PDB stream's length is an entry in a table at
/// the front of the file, and which element of the list the stream is decides
/// what its type is. A reading of the stream's bytes alone has none of those,
/// so the tab reads the fields under the stream in the reading it was declared
/// in, and [`Tab`](super::Tab) presents them as the tab's own. A stream whose
/// bytes were recognised, a gzip of a tar, is read by a template that needs
/// nothing outside it, and has a reading of its own. See [`View`].
///
/// A stream joined from several runs opens as one of these too, when it is
/// small enough to hold whole. Its trace is every part's laid end to end, with
/// each part's bits counted along an axis of the trace's own rather than at
/// any place in a file, since the runs are scattered and a PDB's run 3 can be
/// before its run 1. `runs` says where each part's run really is, and the two
/// maps answer through it: a step is given in bits of its own part's run, as a
/// step of a stream unpacked from one run is.
pub struct Space {
    /// This space's own number, which is what everything outside calls it by.
    pub id: SpaceId,
    /// The space the run was unpacked from. 0 is the file.
    pub parent: SpaceId,
    /// The `Decoded` or `Stitched` node in `parent` that opened it.
    pub path: Vec<usize>,
    /// What the run was unpacked with. `Stored` for a joined stream, which was
    /// made by no one decoding: each part says what it was.
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
    reading: Reading,
    trace: Trace,
    /// For a joined stream, where each part's run is, in the order the parts
    /// go. Empty for a stream unpacked from one run.
    runs: Vec<JoinedRun>,
    /// For a stream unpacked from one run, where that run is. Nothing for a
    /// joined stream, whose parts each say in `runs`.
    run: Option<SingleRun>,
}

/// Where the one run a stream was unpacked from is.
///
/// The trace a decoder keeps counts bits from the front of what it was handed,
/// which is the run and not the file: byte 0 of a gzip's contents is a literal
/// three bits into the deflate, and the deflate starts after the gzip header.
/// This is what turns the one count into the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SingleRun {
    /// The space the run is a field of, and where in it the run starts and
    /// how many bits it is. The space is numbered the way the reading that
    /// unpacked the stream numbers them, as a joined stream's runs are: 0 is
    /// that reading's own document, which is the file for a stream the file's
    /// reading opened, and anything else is a stream the run sits inside.
    pub run_space: SpaceId,
    pub run_offset_bits: u64,
    pub run_bits: u64,
}

/// What reads a space's fields.
enum Reading {
    /// A reading of its own, over the space's bytes, for a template that came
    /// from looking at them.
    Own(Box<super::Evaluator>),
    /// The reading the stream was declared in, under the stream, for the
    /// template the stream declared. The template is kept for what is asked
    /// of the template rather than of the file: the diagram of what the
    /// stream holds.
    Declared { template: Template, view: View },
}

/// Where the fields of a stream opened as a tab are read, when its template is
/// the one the stream declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// The space whose reading holds the stream: 0 for the file, or a space
    /// whose bytes were recognised and so has a reading of its own. Never a
    /// space read this way itself: a stream declared inside one of those is
    /// read in the same reading, further down.
    pub reading: SpaceId,
    /// Where the stream's contents are in that reading. Every path of the tab
    /// is a path under this one.
    pub root: Vec<usize>,
}

/// Where one part of a joined stream opened as a document came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedRun {
    /// Which bytes of the document the part gives.
    pub out_bytes: std::ops::Range<u64>,
    /// The field the part is: a PDB page, a BGZF block's compressed run.
    pub path: Vec<usize>,
    /// Where that run is, in the space it is a field of, and how many bits it
    /// is. 0 is the file.
    pub run_space: SpaceId,
    pub run_offset_bits: u64,
    pub run_bits: u64,
    /// Whether the run was unpacked to give the part, rather than read as it
    /// sits.
    pub packed: bool,
    /// Where the part's bits start on the trace's own axis: every earlier
    /// part's run, end to end. No place in any file.
    pub(super) in_start: u64,
}

impl Space {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        id: SpaceId,
        parent: SpaceId,
        path: Vec<usize>,
        codec: Codec,
        bytes: Arc<Vec<u8>>,
        trace: Trace,
        runs: Vec<JoinedRun>,
        run: Option<SingleRun>,
        template: Template,
        view: Option<View>,
    ) -> Space {
        Space {
            id,
            parent,
            path,
            codec,
            template: template.name.clone(),
            recognised: view.is_none(),
            doc: Document::new(ArcSource(bytes)),
            reading: match view {
                None => Reading::Own(Box::new(super::Evaluator::new(template))),
                Some(view) => Reading::Declared { template, view },
            },
            trace,
            runs,
            run,
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

    /// Which step of the decoding produced a byte of this space, with its bits
    /// counted from the start of the run it read them from.
    ///
    /// For a stream unpacked from one run that is the trace's own count, and
    /// [`Space::run`] says where the run is. For a joined stream, the step of
    /// the part the byte is in, counted from the start of that part's run
    /// rather than along the trace's own axis: a byte of what a BGZF block
    /// unpacks to names bits of that block's deflate, and [`Space::run_at`]
    /// says where the block's deflate is. A byte of a stored part is the one
    /// step over that part's bytes.
    pub fn map_out(&self, byte: u64) -> Option<Step> {
        let step = self.trace.map_out(byte)?;
        if self.runs.is_empty() {
            return Some(step);
        }
        Some(in_run(step, self.run_at(byte)?))
    }

    /// Which step read a bit of the run this space was unpacked from, and so
    /// which bytes of this space that bit produced.
    ///
    /// `bit` is a bit of the space the run is a field of, not a bit of the run:
    /// for every stream the file's reading opened from a field of its own, a
    /// bit of the file. The run's place is taken off before the trace is
    /// asked, and a bit before the run or past its end is nothing of this
    /// space's. The step's bits count from the run's start, as
    /// [`Space::map_out`] counts them.
    ///
    /// A joined stream has no one run, so there the part whose run holds the
    /// bit answers, with the step's bits counted from that run's start. Nothing
    /// for a bit no part was read from.
    pub fn map_in(&self, bit: u64) -> Option<Step> {
        if self.runs.is_empty() {
            let run = self.run?;
            let in_run = bit.checked_sub(run.run_offset_bits).filter(|&at| at < run.run_bits)?;
            return self.trace.map_in(in_run);
        }
        let run = self.run_holding(bit)?;
        Some(in_run(self.trace.map_in(run.in_start + (bit - run.run_offset_bits))?, run))
    }

    /// Which part of a joined stream was read from bit `bit` of the space the
    /// stream was declared in, which is the run [`Space::map_in`] counts its
    /// step's bits from. Asked by the bit and not by the step's output, since a
    /// step that read bits and made nothing, the end of a deflate block, has
    /// its output at the start of the next part.
    pub fn run_holding(&self, bit: u64) -> Option<&JoinedRun> {
        self.runs
            .iter()
            .find(|r| r.run_space == self.parent && (r.run_offset_bits..r.run_offset_bits + r.run_bits).contains(&bit))
    }

    /// Which part of a joined stream byte `byte` is in, and where that part's
    /// run is. Nothing for a stream unpacked from one run, and past the end.
    pub fn run_at(&self, byte: u64) -> Option<&JoinedRun> {
        let i = self.runs.partition_point(|r| r.out_bytes.end <= byte);
        self.runs.get(i).filter(|r| r.out_bytes.contains(&byte))
    }

    /// Every part of a joined stream, in the order the parts go. Empty for a
    /// stream unpacked from one run.
    pub fn runs(&self) -> &[JoinedRun] {
        &self.runs
    }

    /// Where the run of a stream unpacked from one run is, which is what its
    /// steps' bits count from. Nothing for a joined stream: see
    /// [`Space::run_at`].
    pub fn run(&self) -> Option<SingleRun> {
        self.run
    }

    /// The reading over this space's own bytes, lent with its document, since
    /// one is no use without the other. Nothing for a stream read where it was
    /// declared, which has none: see [`Space::view`] and
    /// [`Evaluator::tab_node`](super::Evaluator::tab_node).
    pub fn reading(&mut self) -> Option<(&mut super::Evaluator, &Document<ArcSource>)> {
        match &mut self.reading {
            Reading::Own(ev) => Some((ev, &self.doc)),
            Reading::Declared { .. } => None,
        }
    }

    /// Where this space's fields are read, for a stream read where it was
    /// declared. Nothing for one with a reading of its own.
    pub fn view(&self) -> Option<&View> {
        match &self.reading {
            Reading::Own(_) => None,
            Reading::Declared { view, .. } => Some(view),
        }
    }

    /// What this space's bytes are read as: the template the stream declared,
    /// with the file's named types beside it, or the one its bytes were
    /// recognised as.
    pub fn read_as(&self) -> &Template {
        match &self.reading {
            Reading::Own(ev) => ev.template(),
            Reading::Declared { template, .. } => template,
        }
    }
}

/// A step of a joined stream's trace with its bits counted from the start of
/// its part's run, which is a place a reader can go, rather than along the
/// trace's own axis, which is not.
fn in_run(step: Step, run: &JoinedRun) -> Step {
    let back = |bit: u64| bit.saturating_sub(run.in_start);
    Step { in_bits: back(step.in_bits.start)..back(step.in_bits.end), ..step }
}

/// What a space's bytes are kept as.
///
/// A `Decoded` node's space is one buffer, unpacked whole the first time
/// anything inside it is asked for. A `Stitched` node's is a table of where
/// its parts are, and nothing more until a read reaches one: a BAM of a
/// gigabyte is sixteen thousand parts, and holding them all unpacked is the
/// thing a stitched space exists not to do.
pub(super) enum Backing {
    Whole { bytes: Arc<Vec<u8>>, trace: Trace },
    Stitched(Box<Stitch>),
}

/// A space made of parts that are elsewhere, joined end to end.
pub(super) struct Stitch {
    /// The `Stitched` node that opened it, which is what knows the walk its
    /// parts were found by.
    pub(super) path: Vec<usize>,
    /// Every part, by where it starts in the joined bytes. Complete before
    /// anything inside is placed, so the total is known.
    pub(super) parts: Vec<Part>,
    pub(super) len_bytes: u64,
    /// The packed parts unpacked so far, as many as fit under the cap.
    pub(super) cache: std::cell::RefCell<PartCache>,
}

impl Stitch {
    /// The part holding byte `at` of the joined bytes. A part of no bytes
    /// holds nothing, so the answer is the next part along that has any.
    pub(super) fn part_at(&self, at: u64) -> Option<usize> {
        let i = self.parts.partition_point(|p| p.start + p.len <= at);
        self.parts.get(i).filter(|p| p.start <= at && at < p.start + p.len).map(|_| i)
    }
}

/// One run of a stitched space, and where in the joined bytes it goes.
#[derive(Debug, Clone)]
pub(super) struct Part {
    /// Where the part starts in the joined bytes, and how many it gives.
    pub(super) start: u64,
    pub(super) len: u64,
    /// The field the walk landed on, which is the run in the file (or in the
    /// space the run is in). Kept so a byte of the joined stream can name the
    /// field it came from, and so an unpacked part already open as a stream of
    /// its own is read from there rather than unpacked twice.
    pub(super) path: Vec<usize>,
    pub(super) source: PartSource,
}

/// Where a part's bytes come from.
#[derive(Debug, Clone)]
pub(super) enum PartSource {
    /// Bytes as they sit: a PDB page. `at_bits` is where they start in
    /// `space`.
    Stored { space: u32, at_bits: u64 },
    /// A run that has to be unpacked first: a BGZF member's deflate. The
    /// codec was worked out when the walk reached the run, so a read never
    /// needs to ask the template anything. `Err` is a codec that could not be
    /// worked out, which fails a read of this part and nothing else.
    Packed { space: u32, at_bits: u64, size_bits: u64, codec: Result<Codec, Refusal> },
}

/// How many unpacked bytes a stitched space keeps at once, across all its
/// parts. Two hundred and fifty-six BGZF members, which is several thousand
/// short reads either side of wherever the reader is.
pub(super) const STITCH_CACHE_BYTES: usize = 16 << 20;

/// The parts of a stitched space that have been unpacked, kept until the cap
/// says one has to go, and then the one read longest ago goes first.
pub(super) struct PartCache {
    entries: FxHashMap<usize, (Arc<Vec<u8>>, u64)>,
    /// How many bytes the entries hold, and the most they have held at once.
    bytes: usize,
    pub(super) peak: usize,
    /// A count of reads, so the entry read longest ago is the one with the
    /// smallest.
    clock: u64,
    cap: usize,
}

impl PartCache {
    pub(super) fn new(cap: usize) -> PartCache {
        PartCache { entries: FxHashMap::default(), bytes: 0, peak: 0, clock: 0, cap }
    }

    /// Which parts are being kept, in no order, and how many bytes they hold.
    #[cfg(test)]
    pub(super) fn held(&self) -> (Vec<usize>, usize) {
        (self.entries.keys().copied().collect(), self.bytes)
    }

    /// Part `i`, if it is being kept, marked as read now.
    pub(super) fn get(&mut self, i: usize) -> Option<Arc<Vec<u8>>> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(&i).map(|(bytes, read)| {
            *read = clock;
            bytes.clone()
        })
    }

    /// Keep part `i`, putting back the parts read longest ago until it fits.
    /// A part larger than the whole cap is still kept, alone: the read that
    /// asked for it needs it, and the next read of another part puts it back.
    pub(super) fn put(&mut self, i: usize, bytes: Arc<Vec<u8>>) {
        while self.bytes + bytes.len() > self.cap && !self.entries.is_empty() {
            let oldest = self.entries.iter().min_by_key(|(_, (_, read))| *read).map(|(k, _)| *k).expect("not empty");
            if let Some((gone, _)) = self.entries.remove(&oldest) {
                self.bytes -= gone.len();
            }
        }
        self.clock += 1;
        self.bytes += bytes.len();
        self.peak = self.peak.max(self.bytes);
        self.entries.insert(i, (bytes, self.clock));
    }
}

pub(super) struct Spaces {
    /// What space `i + 1` is made of. Space 0 is the file and is not here.
    backings: Vec<Backing>,
    /// What each `Decoded` or `Stitched` node came to, so a stream is opened
    /// once however many times its children are asked for.
    opened: FxHashMap<Vec<usize>, Opened>,
    /// The cap each stitched space opened from here on is given.
    cache_cap: usize,
    /// Why a stitched stream would not open as a document of its own, for the
    /// ones that were asked to and would not. Kept apart from `opened`, which
    /// says the stream itself opened: one too long to hold whole still reads a
    /// part at a time in the listing.
    whole_refused: FxHashMap<Vec<usize>, Refusal>,
    /// The most a stitched stream may come to and still be held whole.
    whole_cap: usize,
}

impl Default for Spaces {
    fn default() -> Spaces {
        Spaces {
            backings: Vec::new(),
            opened: FxHashMap::default(),
            cache_cap: STITCH_CACHE_BYTES,
            whole_refused: FxHashMap::default(),
            whole_cap: crate::codec::CAP_BYTES,
        }
    }
}

impl Spaces {
    pub(crate) fn get(&self, path: &[usize]) -> Option<Opened> {
        self.opened.get(path).copied()
    }

    /// Keep a decoded buffer and its trace, and hand back the space it became.
    pub(super) fn add(&mut self, path: &[usize], bytes: Vec<u8>, trace: Trace) -> u32 {
        self.backings.push(Backing::Whole { bytes: Arc::new(bytes), trace });
        let id = self.backings.len() as u32;
        self.opened.insert(path.to_vec(), Opened::Space(id));
        id
    }

    /// Keep the part table of a stitched space, and hand back the space it
    /// became.
    pub(super) fn add_stitched(&mut self, path: &[usize], parts: Vec<Part>, len_bytes: u64) -> u32 {
        let cache = std::cell::RefCell::new(PartCache::new(self.cache_cap));
        self.backings.push(Backing::Stitched(Box::new(Stitch { path: path.to_vec(), parts, len_bytes, cache })));
        let id = self.backings.len() as u32;
        self.opened.insert(path.to_vec(), Opened::Space(id));
        id
    }

    /// How many unpacked bytes a stitched space opened after this may keep.
    /// For a test that wants to see the cap at work without a gigabyte file.
    #[cfg(test)]
    pub(super) fn set_cache_cap(&mut self, bytes: usize) {
        self.cache_cap = bytes;
    }

    /// The most a stitched stream may come to and still open as a document of
    /// its own. [`crate::codec::CAP_BYTES`], the cap a `Decoded` stream has.
    pub(super) fn whole_cap(&self) -> usize {
        self.whole_cap
    }

    /// The same cap made small, for a test that wants a stream past it without
    /// sixty-four megabytes of file.
    #[cfg(test)]
    pub(super) fn set_whole_cap(&mut self, bytes: usize) {
        self.whole_cap = bytes;
    }

    pub(super) fn refuse_whole(&mut self, path: &[usize], why: Refusal) {
        self.whole_refused.insert(path.to_vec(), why);
    }

    /// Why the stitched stream at `path` would not open as a document of its
    /// own, when it was asked to and would not.
    pub(super) fn whole_refusal(&self, path: &[usize]) -> Option<Refusal> {
        self.whole_refused.get(path).copied()
    }

    /// The trace of the decoding that made a space. Nothing for a stitched
    /// space, which was not made by one decoding.
    pub(super) fn trace(&self, space: u32) -> Option<&Trace> {
        match self.backings.get(space.checked_sub(1)? as usize)? {
            Backing::Whole { trace, .. } => Some(trace),
            Backing::Stitched(_) => None,
        }
    }

    pub(super) fn refuse(&mut self, path: &[usize], why: Refusal) {
        self.opened.insert(path.to_vec(), Opened::Refused(why));
    }

    /// The bytes of a space held whole. `space` is never 0 here: the file is
    /// read through the document, not through this. Nothing for a stitched
    /// space, whose bytes are nowhere in one piece.
    pub(super) fn buf(&self, space: u32) -> Option<&Arc<Vec<u8>>> {
        match self.backings.get(space.checked_sub(1)? as usize)? {
            Backing::Whole { bytes, .. } => Some(bytes),
            Backing::Stitched(_) => None,
        }
    }

    /// The part table of a stitched space.
    pub(super) fn stitch(&self, space: u32) -> Option<&Stitch> {
        match self.backings.get(space.checked_sub(1)? as usize)? {
            Backing::Stitched(s) => Some(s),
            Backing::Whole { .. } => None,
        }
    }

    /// How many bits a space holds.
    pub(super) fn len_bits(&self, space: u32) -> u64 {
        match space.checked_sub(1).and_then(|i| self.backings.get(i as usize)) {
            Some(Backing::Whole { bytes, .. }) => bytes.len() as u64 * 8,
            Some(Backing::Stitched(s)) => s.len_bytes * 8,
            None => 0,
        }
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
        self.backings.clear();
        self.opened.clear();
        self.whole_refused.clear();
    }
}

#[cfg(test)]
mod tests {
    use crate::codec::StepKind;
    use crate::document::Document;
    use crate::eval::{Evaluator, SingleRun, Value};
    use crate::formats;
    use crate::source::MemSource;

    /// A run whose codec is three numbers the file wrote down, which is what
    /// [`Packing::Lzma1`](crate::template::Packing::Lzma1) exists for.
    ///
    /// A made-up container: the five bytes an LZMA header spends on its
    /// settings, the size that comes out, and then the stream. No format is
    /// laid out quite like this; what is being tested is that the numbers
    /// reach the decoder from *fields*, which is the whole of the change, and
    /// a real archive's are in another part of the file entirely.
    #[test]
    fn a_codec_takes_its_settings_from_fields_of_the_file() {
        use crate::template::{Expr as E, Packing, Template, Ty as T};

        let text = b"what a 7z coder writes down in its header and nowhere else";
        // The stream alone, taken from the `alone` file lzma writes: thirteen
        // bytes of header and then the range-coded bits.
        let alone = {
            let mut out = Vec::new();
            lzma_rs::lzma_compress(&mut &text[..], &mut out).expect("packs");
            out
        };
        let (props, dict) = (alone[0], u32::from_le_bytes(alone[1..5].try_into().unwrap()));
        let mut bytes = vec![props];
        bytes.extend_from_slice(&dict.to_le_bytes());
        bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&alone[13..]);

        let make_named = |props: &str| Template::new(
            "made-up",
            T::structure(
                "Packed",
                vec![
                    ("props", T::u8()),
                    ("dict_size", T::u32(crate::template::Endian::Little)),
                    ("unpacked_size", T::u32(crate::template::Endian::Little)),
                    (
                        "stream",
                        T::decoded_as(
                            E::Remaining,
                            Packing::Lzma1 {
                                props: E::field(props),
                                dict_size: E::field("dict_size"),
                                unpacked: Some(E::field("unpacked_size")),
                            },
                            T::bytes(E::Remaining),
                        ),
                    ),
                ],
            ),
        );
        let d = Document::new(MemSource(bytes));
        let mut e = Evaluator::new(make_named("props"));
        let id = e.open_space(&d, 0, &[3]).unwrap().expect("the stream opens");
        assert_eq!(e.space(id).expect("it is there").bytes(), text);

        // The same run with the properties byte changed stays bytes. A codec
        // worked out from the file can be worked out wrongly, and the answer
        // for that is the answer for any stream that will not unpack: the run
        // is what it is and the node says why, rather than other bytes.
        let mut wrong = d.source().0.clone();
        wrong[0] = 0xff;
        let wrong = Document::new(MemSource(wrong));
        let mut e = Evaluator::new(make_named("props"));
        assert_eq!(e.open_space(&wrong, 0, &[3]).unwrap(), None, "a stream packed a way this cannot read stays bytes");

        // And a template naming a field this file has none of answers the same
        // way. Which field holds a codec's settings is often chosen by a switch
        // on what the file says the codec is, so a template that is right for
        // the files that have them names nothing in the files that do not: a
        // 7z header packed with `copy` writes no properties at all. The run
        // stays bytes, and the node it sits in still reads.
        let mut e = Evaluator::new(make_named("no_such_field"));
        assert!(e.node(&d, &[3]).is_ok(), "a run that will not open is still a field");
        assert_eq!(e.open_space(&d, 0, &[3]).unwrap(), None, "a name that is not there is not an error");

        // The two say different things, which is the point of telling them
        // apart. Nothing read the bytes in the second case, so nothing is
        // claiming they are wrong: a reader told unpacking failed there would
        // go looking for damage in a file that has none.
        let mut e = Evaluator::new(make_named("no_such_field"));
        assert_eq!(e.node(&d, &[3]).unwrap().refused.as_deref(), Some("settings"));
        let mut e = Evaluator::new(make_named("props"));
        assert_eq!(e.node(&wrong, &[3]).unwrap().refused.as_deref(), Some("failed"));
    }

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
        let node = e.tab_node(&d, id, &[0]).unwrap();
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
    ///
    /// The step's bits count from the start of the run, and the run is where
    /// the stream's field is: two bytes into a zlib file, after the header. So
    /// the bit of the file that leads back to a step is the run's place plus
    /// the step's bits, and the header's bits lead nowhere.
    #[test]
    fn the_map_runs_both_ways() {
        let text = "map me both ways. ".repeat(50).into_bytes();
        let d = zlib_over(&text);
        let mut e = Evaluator::new(formats::builtin("zlib").unwrap());
        let field = e.node(&d, RUN).unwrap();
        assert_eq!(field.offset_bits, 16, "the run starts after the two header bytes");
        let id = e.open_space(&d, 0, RUN).unwrap().unwrap();
        let space = e.space(id).unwrap();
        let run = SingleRun { run_space: 0, run_offset_bits: field.offset_bits, run_bits: field.size_bits };
        assert_eq!(space.run(), Some(run));
        for byte in 0..text.len() as u64 {
            let step = space.map_out(byte).unwrap_or_else(|| panic!("byte {byte} came from nowhere"));
            assert!(step.out_bytes.contains(&byte));
            assert!(matches!(step.kind, StepKind::Literal(_) | StepKind::Match { .. } | StepKind::Stored | StepKind::Pixel));
            assert!(step.in_bits.end <= field.size_bits, "{:?} is past the run's {} bits", step.in_bits, field.size_bits);
            // And the bits it read, as bits of the file, lead back to it.
            assert_eq!(space.map_in(field.offset_bits + step.in_bits.start), Some(step));
        }
        assert_eq!(space.map_out(text.len() as u64), None);
        for bit in 0..field.offset_bits {
            assert_eq!(space.map_in(bit), None, "bit {bit} is the zlib header's, not the run's");
        }
        assert_eq!(space.map_in(field.offset_bits + field.size_bits), None, "the Adler-32 after the run");
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
