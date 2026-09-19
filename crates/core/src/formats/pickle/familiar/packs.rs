//! What a file turned out to hold, as against what a form allows it to.
//!
//! A form is a grammar and requires the file to use what it is for. This is
//! the other half of that: which families the file actually used, kept as the
//! bits of one word so that a form attempt carries it and puts it back on a
//! rewind. It is what the mixed form counts two of and what the `families`
//! header row names.
//!
//! [`pack_of`](super::forms::pack_of) is in [`forms`](super::forms), beside
//! the table it reads the prefixes out of.

/// One family whose own productions a file used.
///
/// Not a form. A form is a grammar and requires the file to use what it is
/// for; this is what the file turned out to have used, which under the mixed
/// form is several at once. The order is the order the `families` row names
/// them in: the language's own values, then the standard library, then the
/// array libraries, then how the arrays reached the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pack {
    Builtins,
    Stdlib,
    Numpy,
    Scipy,
    Sklearn,
    Pandas,
    Torch,
    /// Not a family of values: how the arrays were written. Named in the row
    /// because a reader wants to know, and not counted towards the two the
    /// mixed form wants, because a file of nothing but arrays `joblib.dump`
    /// wrote holds one family's worth of data however it was written.
    Joblib,
}

/// Every pack in that order, with what the row calls it.
const EVERY: &[(Pack, &str)] = &[
    (Pack::Builtins, "builtins"),
    (Pack::Stdlib, "stdlib"),
    (Pack::Numpy, "numpy"),
    (Pack::Scipy, "scipy"),
    (Pack::Sklearn, "sklearn"),
    (Pack::Pandas, "pandas"),
    (Pack::Torch, "torch"),
    (Pack::Joblib, "joblib"),
];

/// What the `families` row calls the grammar every form reads and every file
/// is read against, which is the name the plain form already goes by.
const BASIC_PACK: &str = "basic";

/// Which of them a file used, as the bits of one word, so that a form attempt
/// carries it in the cursor and puts it back on a rewind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Packs(u8);

impl Packs {
    pub(super) fn add(&mut self, pack: Pack) {
        self.0 |= 1 << pack as u8;
    }

    /// The same, for a production that already keeps a count of itself.
    pub(super) fn set(&mut self, pack: Pack, used: bool) {
        if used {
            self.add(pack);
        }
    }

    fn has(self, pack: Pack) -> bool {
        self.0 & (1 << pack as u8) != 0
    }

    /// How many families of values the file used, which is what the mixed
    /// form wants two of.
    pub(super) fn families(self) -> usize {
        EVERY.iter().filter(|(pack, _)| *pack != Pack::Joblib && self.has(*pack)).count()
    }

    /// What the `families` row says: the basic grammar the file was read
    /// against, and then everything it used beyond it.
    pub(super) fn names(self) -> String {
        let mut out = String::from(BASIC_PACK);
        for (_, name) in EVERY.iter().filter(|(pack, _)| self.has(*pack)) {
            out.push_str(", ");
            out.push_str(name);
        }
        out
    }
}

/// Whether a module is under one of these packages: the package itself or
/// anything below it, and nothing that merely starts with the same letters, so
/// that `pandas` reaches `pandas.core.frame` and `sklearnish` is neither.
pub(super) fn covers(packages: &[&str], module: &str) -> bool {
    packages
        .iter()
        .any(|package| module.strip_prefix(package).is_some_and(|rest| rest.is_empty() || rest.starts_with('.')))
}
