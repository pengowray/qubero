//! Finds the test material that lives outside this repository: the sample
//! collection, and the format libraries the oracle tests read against. It is
//! not any of them. They have their own repositories, because they are large
//! and none of them are ours.
//!
//! Every test that reads a real file asks here where it is, so that they all
//! look in the same places and all say the same thing when there is nothing to
//! read. Two places, in this order:
//!
//! 1. What `QUBERO_SAMPLES` names. Several, separated by `;`, are allowed, and
//!    each is taken as the root of a collection rather than as a format folder.
//! 2. The first directory called `qubero-samples` beside this checkout or
//!    beside an ancestor of it.
//!
//! The second is what makes this worth a crate. The lookup every test used to
//! carry was `CARGO_MANIFEST_DIR/../../../qubero-samples`, exactly one level
//! up from the checkout, which resolves from a normal checkout and from
//! nowhere else. A git worktree under `.claude/worktrees/NAME` is four levels
//! deeper, so the collection was invisible from one: the tests found no files,
//! read nothing, and reported `ok`. The walk up the ancestors finds it from
//! either, and a run in a worktree now means what a run in the checkout means.
//!
//! `.cargo/config.toml` could carry a `QUBERO_SAMPLES` under `[env]` instead,
//! and cannot be made to work: cargo reads the config nearest the working
//! directory, which in a worktree is the worktree's own committed copy, and a
//! `relative = true` path there resolves against the worktree rather than the
//! checkout. One path cannot be right in both.

use std::path::{Path, PathBuf};

/// The name the collection's own directory has, wherever it has been put.
const NAME: &str = "qubero-samples";

/// Every collection to hand, nearest first, each one a directory that exists.
/// Empty where there is none.
///
/// A path is listed once however many ways it was reached: `QUBERO_SAMPLES`
/// usually names the same directory the walk finds, and a caller that sweeps
/// every root would otherwise read every file twice.
pub fn roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for c in candidates() {
        if !c.is_dir() {
            continue;
        }
        let c = std::fs::canonicalize(&c).unwrap_or(c);
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// The collection, or `None` where there is none.
pub fn root() -> Option<PathBuf> {
    roots().into_iter().next()
}

/// The folder for one format, `hdf5` or `pe` or `hexdump`, in the first
/// collection that has it. `None` where no collection to hand holds that
/// folder, which is what a test reading one format asks about: another
/// format's folder is no use to it.
pub fn dir(name: &str) -> Option<PathBuf> {
    roots().into_iter().map(|r| r.join(name)).find(|p| p.is_dir())
}

/// Why there is nothing to read, for the line a test prints when it skips.
/// Names what was looked for, so that the reader can put it there.
pub fn missing() -> String {
    if let Some(r) = root() {
        return format!("skipped: nothing this test reads in the sample collection at {}", r.display());
    }
    match std::env::var_os("QUBERO_SAMPLES") {
        Some(set) => format!("skipped: QUBERO_SAMPLES names {:?}, which is not a directory", Path::new(&set)),
        None => format!("skipped: no sample collection. Put `{NAME}` beside this checkout, or point QUBERO_SAMPLES at it."),
    }
}

/// The same, for a test that wants one format's folder and found the
/// collection without it.
pub fn missing_dir(name: &str) -> String {
    match root() {
        Some(r) => format!("skipped: no `{name}` folder in the sample collection at {}", r.display()),
        None => missing(),
    }
}

/// Another checkout to read against: `kaitai_struct` for the `.ksy` corpus,
/// `ImHex-Patterns` for the `.hexpat` one. `None` where it is not on this
/// machine.
///
/// Unlike the collection these are ordinary clones of other people's
/// repositories, kept wherever the machine keeps such things, so the places to
/// look are: beside this checkout or an ancestor of it, then `~/github`, then
/// `D:/github`, which is where the Windows machine keeps them. Each test that
/// reads one still takes its own variable first, for a checkout somewhere
/// else.
pub fn checkout(name: &str) -> Option<PathBuf> {
    let mut out: Vec<PathBuf> = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().map(|a| a.join(name)).collect();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        out.push(Path::new(&home).join("github").join(name));
    }
    out.push(Path::new("D:/github").join(name));
    out.into_iter().find(|p| p.is_dir()).map(|p| std::fs::canonicalize(&p).unwrap_or(p))
}

/// Everywhere to look, in order, whether or not it is there.
fn candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(set) = std::env::var_os("QUBERO_SAMPLES") {
        let set = set.to_string_lossy().into_owned();
        out.extend(set.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    // From this crate's own directory rather than the caller's, so that the
    // walk starts at a known depth in the checkout however deep the test
    // calling it sits.
    out.extend(Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().map(|a| a.join(NAME)));
    out
}

#[cfg(test)]
mod tests {
    /// Not that the collection is there, which a checkout is allowed to be
    /// without, but that the answers agree with each other.
    #[test]
    fn the_roots_are_directories_and_listed_once() {
        let roots = super::roots();
        for r in &roots {
            assert!(r.is_dir(), "{} is not a directory", r.display());
            assert_eq!(roots.iter().filter(|o| *o == r).count(), 1, "{} is listed twice", r.display());
        }
        assert_eq!(super::root().is_some(), !roots.is_empty());
        assert!(super::missing().starts_with("skipped: "));
        // A name nothing is called answers None rather than a directory that
        // happens to be an ancestor of this crate.
        assert_eq!(super::checkout("not-a-checkout-of-anything"), None);
        for found in [super::checkout("kaitai_struct"), super::checkout("ImHex-Patterns")].into_iter().flatten() {
            assert!(found.is_dir());
        }
    }
}
