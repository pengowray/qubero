//! Which pickler wrote a file, as far as its bytes say, and the one number the
//! picklers do not agree on.
//!
//! Seven programs write the pickles in the collection. CPython has `_pickle`,
//! which is C, and `pickle.py` beside it; Python 2 had `cPickle`, which is a
//! different program again, and PyPy 2.7 ships a Python copy of that one. The
//! three interpreters outside CPython's family bring one or two more each:
//! Jython's `cPickle` is written in Java, IronPython's `cPickle` and `_pickle`
//! in C#, and GraalPy's `_pickle` in Java.
//!
//! Most of what tells an interpreter apart is not the pickler at all. Jython
//! spelling a protocol 0 escape in upper case, GraalPy naming `_collections`,
//! IronPython handing `datetime` its fields instead of its packed bytes: each
//! of those comes out of the runtime's own classes and its own text routines,
//! and both of that interpreter's picklers write it. Only a pickler's own
//! behaviour reaches this file: how it numbers the memo, how long a batch it
//! writes, how it ends a container, and whether it files a bytearray.
//!
//! So a reading sharpens rather than switching. A file that numbers the memo
//! from one was written by one of the three `cPickle`s, and a batch of 1,024
//! in the same file narrows that to Jython's. [`refine`] is that relation, and
//! two readings neither of which is a case of the other are a file no single
//! pickler wrote.

/// Which pickler wrote the file, as far as its bytes say.
///
/// The variants are not seven programs but seven statements, each true of the
/// programs [`Pickler::name`] lists. A statement with another below it in
/// [`refine`] is the broader one, and a file says the sharpest that its
/// spellings support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pickler {
    /// Nothing in the file tells them apart, which is most files.
    Undetermined,
    /// The tails a C pickler writes: a batch for whatever is left over, even
    /// when that is nothing.
    C,
    /// The tails `pickle.py` writes: APPEND or SETITEM for a single item left
    /// over, and nothing at all for none.
    Python,
    /// The memo numbered from one, which is what the three `cPickle`s do and
    /// no other pickler does.
    CPickle,
    /// Jython's `cPickle`, known by its batch of 1,024.
    Jython,
    /// IronPython's `cPickle`, known by the memo slot it takes for an object
    /// before writing what the object is made of.
    IronCPickle,
    /// GraalPy's `_pickle`, known by the protocol 0 float lines it writes in
    /// Java's spelling rather than Python's.
    Graal,
}

impl Pickler {
    /// What the `pickler` row says.
    ///
    /// Each names every program that writes the spelling the file showed, so
    /// that the row is true of the file rather than true of CPython. A reader
    /// who knows which interpreter the file came from can narrow it further
    /// than the bytes can.
    pub fn name(self) -> &'static str {
        match self {
            Pickler::Undetermined => "unnamed: nothing in this file is spelled two ways",
            Pickler::C => "_pickle (CPython's C pickler, or GraalPy's written in Java)",
            Pickler::Python => "pickle.py (the pure Python pickler), or one of IronPython's written in C#",
            Pickler::CPickle => "cPickle (Python 2's C pickler, PyPy 2.7's Python copy of it, or Jython's written in Java)",
            Pickler::Jython => "cPickle (Jython's, written in Java)",
            Pickler::IronCPickle => "cPickle (IronPython's, written in C#)",
            Pickler::Graal => "_pickle (GraalPy's, written in Java)",
        }
    }

    /// The statement this one is a case of, and so on up to
    /// [`Pickler::Undetermined`], which every file starts at.
    fn broader(self) -> Pickler {
        match self {
            Pickler::Undetermined => Pickler::Undetermined,
            Pickler::C | Pickler::Python | Pickler::CPickle => Pickler::Undetermined,
            Pickler::Graal => Pickler::C,
            // IronPython's `cPickle` ends a container the way `pickle.py`
            // ends one, which is the broader statement its files start with.
            Pickler::IronCPickle => Pickler::Python,
            Pickler::Jython => Pickler::CPickle,
        }
    }

    /// Whether this statement is the other one or a case of it.
    fn under(self, other: Pickler) -> bool {
        let mut here = self;
        loop {
            if here == other {
                return true;
            }
            if here == Pickler::Undetermined {
                return false;
            }
            here = here.broader();
        }
    }

    /// How many entries this pickler puts in one batch. Jython's is the one
    /// that is not a thousand, and [`WIDE_BATCH`] is what it writes instead.
    pub(super) fn batch(self) -> usize {
        match self {
            Pickler::Jython => WIDE_BATCH,
            _ => super::MAX_BATCH,
        }
    }
}

/// The batch Jython's `cPickle` writes, which is `BATCHSIZE` in
/// `src/org/python/modules/cPickle.java`. Every other pickler writes a
/// thousand.
pub(super) const WIDE_BATCH: usize = 1024;

/// The sharper of two readings of the same file, or nothing when neither is a
/// case of the other.
///
/// A file is written by one pickler, so every spelling in it has to be that
/// pickler's. A file whose containers end the way the C pickler ends one and
/// whose memo is numbered from one was written by neither.
pub(super) fn refine(seen: Pickler, which: Pickler) -> Option<Pickler> {
    if which.under(seen) {
        return Some(which);
    }
    seen.under(which).then_some(seen)
}
