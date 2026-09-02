//! A decoded stream handed over as a document of its own.
//!
//! **This is the package B stub that package A replaces.** It is one small file
//! on purpose: everything real about the map lives in package A's codec traces,
//! and this only exists so the wasm layer and the tab strip can be built and
//! tested against the shape of the contract before the traces land.
//!
//! What it does honestly: [`Evaluator::open_space_doc`] opens the stream the
//! way the listing already does (see [`Evaluator::open_space`], which it calls)
//! and hands back the decoded bytes together with a template that reads them,
//! which is the `Decoded` node's `inner` over the opening template's
//! vocabulary. What it does not do: recognise the unpacked bytes when `inner`
//! is only `bytes` (a tar inside a gzip stays `bytes` this round), and the map.
//! [`Evaluator::map_out`] and [`Evaluator::map_in`] answer `None` for
//! everything, so the cursor link renders nothing until package A lands.
//!
//! The name is `open_space_doc` rather than `open_space` only because the
//! latter is taken by the `pub(super)` method above, which is the one package A
//! makes public; a merge that keeps both names costs one rename.

use std::ops::Range;
use std::sync::Arc;

use super::{fail, space, Evaluator, R};
use crate::codec::Refusal;
use crate::document::Document;
use crate::source::Source;
use crate::template::{Template, Ty};

/// A decoded stream as a document: its bytes, and what reads them.
pub struct SpaceDoc {
    pub bytes: Arc<Vec<u8>>,
    pub template: Template,
}

/// What came of asking for one.
pub enum OpenedDoc {
    Opened(SpaceDoc),
    /// The stream would not open, in one of the three ways.
    Refused(Refusal),
}

/// One thing a decoder did, as the reader sees it: which bits of the input it
/// read, which bytes of the output that came to, and which kind of step it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub in_bits: Range<u64>,
    pub out_bytes: Range<u64>,
    pub kind: StepKind,
}

/// The kinds of step a decoder emits. Package A fills these in; nothing here
/// produces one yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Literal,
    Match { len: u64, dist: u64 },
    Stored,
    Block,
    Header,
    Table,
    Opaque,
}

/// The output bytes a stretch of input came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutRange {
    pub out_bytes: Range<u64>,
}

impl Evaluator {
    /// Open the `Decoded` node at `path` as a document of its own.
    ///
    /// The stream is opened once however many times this is asked, because
    /// [`Evaluator::open_space`] memoises it; and it is thrown away with
    /// everything else when the document or the template changes.
    pub fn open_space_doc<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<OpenedDoc> {
        let opened = self.open_space(doc, path)?;
        let id = match opened {
            space::Opened::Space(id) => id,
            space::Opened::Refused(why) => return Ok(OpenedDoc::Refused(why)),
        };
        let Some(bytes) = self.space_bytes(id) else { return fail("this stream did not open") };
        let Some(r) = self.memo.get(path) else { return fail("not a decoded stream") };
        let Ty::Decoded { inner, .. } = &r.ty else { return fail("not a decoded stream") };
        // The vocabulary comes with the type: a compressed ROOT record's object
        // is a `Ty::Named` and every name it reaches for is on the template
        // that declared it.
        let template = Template {
            name: self.template().name.clone(),
            root: (**inner).clone(),
            types: self.template().types.clone(),
        };
        Ok(OpenedDoc::Opened(SpaceDoc { bytes, template }))
    }

    /// Which bits of the input the byte at `byte` of the space came from.
    /// Always `None` until package A's decoders emit their traces.
    pub fn map_out(&self, _path: &[usize], _byte: u64) -> Option<Step> {
        None
    }

    /// Which bytes of the output the bit at `bit` of the input came to.
    /// Always `None` until package A's decoders emit their traces.
    pub fn map_in(&self, _path: &[usize], _bit: u64) -> Option<OutRange> {
        None
    }
}
