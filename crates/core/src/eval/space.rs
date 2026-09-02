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

use crate::codec::Refusal;

/// What came of asking a `Decoded` node to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opened {
    /// The space it opened, which is never 0.
    Space(u32),
    Refused(Refusal),
}

/// Every space this reading has opened, and what each `Decoded` node came to.
#[derive(Default)]
pub(super) struct Spaces {
    /// The bytes of space `i + 1`. Space 0 is the file and is not here.
    bufs: Vec<Arc<Vec<u8>>>,
    /// What each `Decoded` node came to, so a stream is opened once however
    /// many times its children are asked for.
    opened: FxHashMap<Vec<usize>, Opened>,
}

impl Spaces {
    pub(crate) fn get(&self, path: &[usize]) -> Option<Opened> {
        self.opened.get(path).copied()
    }

    /// Keep a decoded buffer and hand back the space it became.
    pub(super) fn add(&mut self, path: &[usize], bytes: Vec<u8>) -> u32 {
        self.bufs.push(Arc::new(bytes));
        let id = self.bufs.len() as u32;
        self.opened.insert(path.to_vec(), Opened::Space(id));
        id
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
        self.opened.clear();
    }
}
