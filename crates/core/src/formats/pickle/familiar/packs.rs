//! What a file turned out to hold, as against what a form allows it to.
//!
//! Every form reads the basic grammar, and a form for a library reads that and
//! more. This is which of the more a file actually used, kept as the bits of
//! one word so that a form attempt carries it and puts it back on a rewind. It
//! is what the mixed form counts two of and what the `form extensions` header
//! row names.
//!
//! [`extension_of`](super::forms::extension_of) is in [`forms`](super::forms),
//! beside the table it reads the prefixes out of.

/// One extension of the basic grammar whose own productions a file used.
///
/// Not a form. A form is a grammar and requires the file to use what it is
/// for; this is what the file turned out to have used, which under the mixed
/// form is several at once. The order is the order the `form extensions` row
/// names them in: the language's own values, then the standard library, then
/// the array libraries, then how the arrays reached the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Extension {
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

/// Every extension in that order, with what the row calls it.
const EVERY: &[(Extension, &str)] = &[
    (Extension::Builtins, "builtins"),
    (Extension::Stdlib, "stdlib"),
    (Extension::Numpy, "numpy"),
    (Extension::Scipy, "scipy"),
    (Extension::Sklearn, "sklearn"),
    (Extension::Pandas, "pandas"),
    (Extension::Torch, "torch"),
    (Extension::Joblib, "joblib"),
];

/// What the row says for a file that used none of them, which is a file the
/// basic grammar read on its own.
const NO_EXTENSIONS: &str = "none";

/// Which of them a file used, as the bits of one word, so that a form attempt
/// carries it in the cursor and puts it back on a rewind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Extensions(u8);

impl Extensions {
    pub(super) fn add(&mut self, extension: Extension) {
        self.0 |= 1 << extension as u8;
    }

    /// The same, for a production that already keeps a count of itself.
    pub(super) fn set(&mut self, extension: Extension, used: bool) {
        if used {
            self.add(extension);
        }
    }

    fn has(self, extension: Extension) -> bool {
        self.0 & (1 << extension as u8) != 0
    }

    /// How many families of values the file used, which is what the mixed
    /// form wants two of.
    pub(super) fn families(self) -> usize {
        EVERY.iter().filter(|(extension, _)| *extension != Extension::Joblib && self.has(*extension)).count()
    }

    /// What the `form extensions` row says: everything the file used beyond
    /// the basic grammar, which every form reads and which is the form itself
    /// rather than an extension of it.
    pub(super) fn names(self) -> String {
        let said: Vec<&str> = EVERY.iter().filter(|(extension, _)| self.has(*extension)).map(|(_, name)| *name).collect();
        match said.is_empty() {
            true => NO_EXTENSIONS.to_string(),
            false => said.join(", "),
        }
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
