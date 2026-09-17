//! A space's fields as a tab reads them.
//!
//! A stream opened as a document of its own is read by the template it
//! declared, and that template names what is outside the stream as freely as
//! anything else in the file does. A PDB stream is as long as an entry in the
//! table at the front of the file says, and which element of the list it is
//! decides its type; an HDF4 dataset's values are laid out by the dimension
//! record beside them. A reading of the stream's bytes on their own has none of
//! that above its root, and fails there.
//!
//! So a tab over a stream whose template is the one it declared reads the
//! fields under the stream in the reading it was declared in, which already
//! reads them, and this presents them as the tab's own: every path in and out
//! is under the stream's contents, every offset is in the tab's bytes, and a
//! node of those bytes is in space 0, as it would be in a reading of the tab
//! alone. Nothing is read twice and nothing is kept twice. An edit to the file
//! drops the spaces with the rest of the reading, and the tab is opened again
//! over what the edit left.
//!
//! A stream whose template came from looking at its bytes is read by a
//! template that needs nothing outside them, and its tab is a reading of its
//! own. That reading goes through here too, with no root, where every
//! question is passed through as it is asked.

use super::*;
use super::census::{Census, CensusWalk};
use super::origin::Origin;

/// What reads a tab's fields, lent for one question.
pub struct Tab<'a, S: Source> {
    /// The reading the fields are read in: the tab's own, or the one the
    /// stream was declared in.
    pub ev: &'a mut Evaluator,
    /// The document that reading reads.
    pub doc: &'a Document<S>,
    /// Where the tab's fields are in that reading. Empty for a reading of the
    /// tab's own bytes.
    root: Vec<usize>,
    /// The space the stream's contents are counted in there, once asked. 0 for
    /// a reading of the tab's own bytes.
    inner: Option<SpaceId>,
}

impl<'a, S: Source> Tab<'a, S> {
    /// The fields under `root` of the reading `ev` makes of `doc`, as a tab's.
    /// An empty `root` is a reading of the tab's own bytes, or of the file.
    pub fn new(ev: &'a mut Evaluator, doc: &'a Document<S>, root: Vec<usize>) -> Tab<'a, S> {
        let inner = root.is_empty().then_some(0);
        Tab { ev, doc, root, inner }
    }

    /// Where the tab's fields are in the reading, empty for a reading of the
    /// tab's own bytes.
    pub fn root(&self) -> &[usize] {
        &self.root
    }

    /// The path in the reading of the tab's field at `path`.
    pub fn path_in(&self, path: &[usize]) -> Vec<usize> {
        [self.root.as_slice(), path].concat()
    }

    /// The tab's path for the reading's `path`, or nothing for a field that is
    /// not under the stream: one of the fields outside it that decided
    /// something about it, which the tab has no row for.
    pub fn path_out(&self, path: &[usize]) -> Option<Vec<usize>> {
        path.strip_prefix(self.root.as_slice()).map(<[usize]>::to_vec)
    }

    /// The space the stream's contents are counted in, in the reading.
    fn inner(&mut self) -> R<SpaceId> {
        if let Some(space) = self.inner {
            return Ok(space);
        }
        self.ev.resolve(self.doc, &self.root)?;
        let space = self.ev.memo[self.root.as_slice()].space;
        self.inner = Some(space);
        Ok(space)
    }

    /// The tab's number for a space of the reading. The stream's contents are
    /// the tab's own bytes, which a tab calls 0, and the file, which a field
    /// inside the stream can still read from, takes the number they had, so
    /// that it is not taken for the tab's bytes either.
    fn space_out(&mut self, space: SpaceId) -> R<SpaceId> {
        let inner = self.inner()?;
        Ok(if space == inner {
            0
        } else if space == 0 {
            inner
        } else {
            space
        })
    }

    /// A node of the reading as the tab's.
    ///
    /// Besides the path and the space, two things a node of the stream says
    /// about where it is are not true of the tab: that it is inside a stream
    /// joined from several runs, since the tab's bytes are the joined stream
    /// and its offsets count from the front of them, and, for the tab's root,
    /// that it is the first node of a space, which is where a listing offers
    /// to open a stream as a tab.
    fn node_out(&mut self, mut n: NodeInfo) -> R<NodeInfo> {
        if self.root.is_empty() {
            return Ok(n);
        }
        n.path = self.path_out(&n.path).unwrap_or_default();
        n.space = self.space_out(n.space)?;
        if n.space == 0 {
            n.joined = false;
        }
        if n.path.is_empty() {
            n.space_root = false;
        }
        Ok(n)
    }

    pub fn node(&mut self, path: &[usize]) -> R<NodeInfo> {
        let n = self.ev.node(self.doc, &self.path_in(path))?;
        self.node_out(n)
    }

    pub fn children(&mut self, path: &[usize], from: u64, to: u64) -> R<Vec<NodeInfo>> {
        let found = self.ev.children(self.doc, &self.path_in(path), from, to)?;
        found.into_iter().map(|n| self.node_out(n)).collect()
    }

    pub fn text_value(&mut self, path: &[usize]) -> R<(String, bool)> {
        self.ev.text_value(self.doc, &self.path_in(path))
    }

    pub fn field_bytes(&mut self, path: &[usize], limit: u64) -> R<(Vec<u8>, bool)> {
        self.ev.field_bytes(self.doc, &self.path_in(path), limit)
    }

    pub fn explain(&mut self, path: &[usize], at_bits: Option<u64>) -> R<Explain> {
        self.ev.explain(self.doc, &self.path_in(path), at_bits)
    }

    pub fn shape(&mut self, path: &[usize]) -> R<Shape> {
        self.ev.shape(self.doc, &self.path_in(path))
    }

    pub fn check_of(&mut self, path: &[usize]) -> R<Option<CheckInfo>> {
        self.ev.check_of(self.doc, &self.path_in(path))
    }

    pub fn run_check(&mut self, path: &[usize]) -> R<Option<Verdict>> {
        self.ev.run_check(self.doc, &self.path_in(path))
    }

    pub fn time_of(&mut self, path: &[usize]) -> R<Option<TimeInfo>> {
        self.ev.time_of(self.doc, &self.path_in(path))
    }

    /// What table the field at `path` is, with each fact's path mapped into
    /// the tab's own numbering the way `origins` does it.
    pub fn table_shape(&mut self, path: &[usize]) -> R<Option<TableShapeInfo>> {
        let found = self.ev.table_shape(self.doc, &self.path_in(path))?;
        Ok(found.map(|t| TableShapeInfo {
            facts: t
                .facts
                .into_iter()
                .map(|o| Origin { path: self.path_out(&o.path).unwrap_or_default(), ..o })
                .collect(),
            ..t
        }))
    }

    pub fn relations(&mut self, path: &[usize]) -> R<Vec<Relation>> {
        self.ev.relations(self.doc, &self.path_in(path))
    }

    pub fn run_cells(&mut self, path: &[usize], from_bit: u64, to_bit: u64, max: usize) -> R<Vec<Cell>> {
        self.ev.run_cells(self.doc, &self.path_in(path), from_bit, to_bit, max)
    }

    /// Which fields settled the shape of the one at `path`. One outside the
    /// stream keeps its name and what it says, and loses its path: the tab has
    /// no row for it to go to.
    pub fn origins(&mut self, path: &[usize]) -> R<Vec<Origin>> {
        let found = self.ev.origins(self.doc, &self.path_in(path))?;
        Ok(found.into_iter().map(|o| Origin { path: self.path_out(&o.path).unwrap_or_default(), ..o }).collect())
    }

    /// Which part of a joined stream the field at `path` starts in. Asked by
    /// the space the field is counted in in the reading, which is the one the
    /// part table belongs to, rather than by the tab's.
    pub fn part_of(&mut self, path: &[usize]) -> R<Option<PartHit>> {
        let n = self.ev.node(self.doc, &self.path_in(path))?;
        self.ev.part_of(self.doc, n.space, n.offset_bits / 8)
    }

    pub fn graph(&mut self, path: &[usize], limit: usize) -> R<Graph> {
        let mut g = self.ev.graph(self.doc, &self.path_in(path), limit)?;
        for n in &mut g.nodes {
            n.path = self.path_out(&n.path).unwrap_or_default();
        }
        Ok(g)
    }

    /// A count of the tab's fields against the diagram's boxes, to be carried
    /// on a go at a time with `census_step`.
    pub fn census_walk(&self, bits: u64) -> CensusWalk {
        CensusWalk::under(bits, self.root.clone())
    }

    pub fn census_step(&mut self, walk: &mut CensusWalk, limit: usize) -> R<Census> {
        let mut c = self.ev.census_step(self.doc, walk, limit)?;
        for b in &mut c.boxes {
            b.first_path = self.path_out(&b.first_path).unwrap_or_default();
            b.space = self.space_out(b.space)?;
        }
        for r in &mut c.rows {
            r.first_path = self.path_out(&r.first_path).unwrap_or_default();
            r.space = self.space_out(r.space)?;
        }
        Ok(c)
    }

    /// The deepest of the tab's fields covering bit `bit` of the tab.
    pub fn locate(&mut self, bit: u64) -> R<Vec<usize>> {
        let root = self.root.clone();
        let found = self.ev.locate_under(self.doc, &root, bit)?;
        Ok(self.path_out(&found).unwrap_or_default())
    }

    /// Every field across a stretch of the tab, for the annotation column.
    pub fn spans(&mut self, from: u64, to: u64, max: usize) -> R<Vec<Span>> {
        let root = self.root.clone();
        let mut found = self.ev.spans_under(self.doc, &root, from, to, max)?;
        if root.is_empty() {
            return Ok(found);
        }
        for span in &mut found {
            let Some(path) = self.path_out(&span.path) else { continue };
            // The tab's root is not offered as a stream to open, being the
            // tab; a root that is itself a stream still is.
            if path.is_empty() && span.opens {
                let n = self.node(&[])?;
                span.opens = n.decoded && n.refused.is_none();
            }
            span.path = path;
        }
        Ok(found)
    }

    /// A walk totalling the tab's bits by kind, over `bits` bits.
    pub fn kind_walk(&self, bits: u64) -> KindWalk {
        KindWalk::under(bits, self.root.clone())
    }

    pub fn kind_totals_step(&mut self, walk: &mut KindWalk) -> R<KindTotals> {
        self.ev.kind_totals_step(self.doc, walk)
    }

    /// The most advanced unfinished walk of a list among the tab's fields.
    pub fn extent_estimate(&self) -> Option<ExtentEstimate> {
        let mut found = self.ev.extent_estimate_under(&self.root)?;
        found.path = self.path_out(&found.path)?;
        Some(found)
    }
}
