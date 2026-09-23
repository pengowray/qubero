//! What each directory in a file points to.
//!
//! A directory is a list whose elements place something elsewhere in the
//! file: ZIP's central directory, a TrueType table directory, ELF's section
//! headers, a TIFF IFD's entries. The report draws each one as a ribbon, the
//! entries in the order the directory stores them joined to what they place
//! in the order the file holds it (see "The ribbon" in
//! `docs/DESIGN-report-view.md`). Nothing about it is particular to a format:
//! a directory is found by what the template declares, in three ways.
//!
//! - **An address inside each element.** An element holding a field declared
//!   with [`Ty::At`] places what that field points at. A TIFF entry whose
//!   value does not fit in it points at the value; an ELF section header
//!   points at its own name. These come from the index `placed.rs` already
//!   keeps of every stretch an address placed, so no walk is taken for them
//!   that the cursor would not take anyway.
//! - **A table of offsets.** A [`Ty::PointerList`] places child `i` at the
//!   offset in element `i` of a list declared beside it. That list is the
//!   directory: ELF's `sections` are placed by `section_headers`, a SQLite
//!   page's cells by its cell pointer array.
//! - **Descriptors.** A [`Ty::Gather`] places child `i` from record `i` of a
//!   walk to records wherever they are. The gather is the directory and the
//!   records are its entries.
//!
//! A list whose elements place several things has every one of them: an ELF
//! section header names its section's name and its section's bytes.

use std::collections::BTreeMap;

use super::*;

/// Something one entry places.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryTarget {
    pub path: Vec<usize>,
    pub name: String,
    pub offset_bits: u64,
    pub size_bits: u64,
    /// How the entry places it: `address`, `offsets` or `descriptors`.
    pub via: &'static str,
    /// True when the template reads these bytes a second time from here and
    /// counts them where they are: an ELF section's name, which belongs to
    /// the section name table. See [`crate::template::Field::aside`].
    pub aside: bool,
}

/// One element of a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub path: Vec<usize>,
    pub name: String,
    pub offset_bits: u64,
    pub size_bits: u64,
    pub targets: Vec<DirectoryTarget>,
}

/// One directory: a list, and what its elements place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directory {
    pub path: Vec<usize>,
    pub name: String,
    /// How many elements the list has, whether or not they place anything.
    pub elements: u64,
    /// How many of them place something.
    pub placing: u64,
    /// The first [`ENTRY_CAP`] elements that place something, in the order
    /// the directory stores them.
    pub entries: Vec<DirectoryEntry>,
}

/// Every directory found so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directories {
    /// In the order the file holds them.
    pub lists: Vec<Directory>,
    /// Placements the walk found and did not look at, past
    /// [`PLACEMENT_CAP`]. Each would have been an entry of some directory.
    pub unexamined: u64,
    pub done: bool,
}

/// How many entries of one directory are described. The ribbon draws a few
/// dozen before it is a smear, and a reader who wants the ten thousandth
/// entry of a ZIP's central directory goes to the listing.
pub const ENTRY_CAP: usize = 256;

/// How many placements are looked at in all. HDF5 places every object by
/// address and a large file has a few hundred thousand; finding which list
/// each sits in means placing that list's element.
pub const PLACEMENT_CAP: usize = 20_000;

/// A target before it has been named: where it is and how it was placed.
#[derive(Debug, Clone)]
struct Pending {
    path: Vec<usize>,
    via: &'static str,
    aside: bool,
}

/// What the directory stage has found, kept between goes.
#[derive(Debug, Default)]
pub(super) struct Build {
    /// The lists of offsets and the gathers the walk opened, to be looked at
    /// once it is over.
    pointer_lists: Vec<Vec<usize>>,
    gathers: Vec<Vec<usize>>,
    /// Where the stage has got to: which of the three sources, and which
    /// item of it.
    stage: u8,
    next: usize,
    placements: Vec<Vec<usize>>,
    unexamined: u64,
    /// Directory path to its entries, each entry path to what it places.
    found: BTreeMap<Vec<usize>, BTreeMap<Vec<usize>, Vec<Pending>>>,
    /// The directories found, in the order they are filled in: one a turn,
    /// once every source has been looked at.
    order: Vec<Vec<usize>>,
    lists: Vec<Directory>,
    done: bool,
}

/// How many lists of offsets and gathers are looked at. A SQLite page has
/// one list of cell pointers, and a large database is a quarter of a million
/// pages; the first few hundred say what the rest look like.
pub const LIST_CAP: usize = 256;

impl Build {
    pub(super) fn directories(&self) -> Directories {
        let mut lists = self.lists.clone();
        lists.sort_by_key(|d| d.entries.first().map_or(u64::MAX, |e| e.offset_bits));
        Directories { lists, unexamined: self.unexamined, done: self.done }
    }

    /// A list of offsets or a gather the walk opened, kept for later unless
    /// enough have been.
    pub(super) fn list(&mut self, path: &[usize], gather: bool) {
        let lists = if gather { &mut self.gathers } else { &mut self.pointer_lists };
        if lists.len() < LIST_CAP {
            lists.push(path.to_vec());
        } else {
            self.unexamined += 1;
        }
    }

    fn add(&mut self, dir: Vec<usize>, entry: Vec<usize>, target: Pending) {
        let entries = self.found.entry(dir).or_default();
        entries.entry(entry).or_default().push(target);
    }

    /// Carry the stage on until it is done or the allowance runs out.
    pub(super) fn step<S: Source>(&mut self, ev: &mut Evaluator, doc: &Document<S>, root: &[usize]) -> R<()> {
        while !self.done {
            match self.stage {
                // Every stretch an address placed, from the index the cursor
                // already asks. Only in the file: the index is of the file.
                0 => {
                    if root.is_empty() {
                        while !ev.placed.done {
                            ev.index_placements(doc)?;
                        }
                        self.placements = ev.placed.stretches.iter().map(|p| p.path.clone()).collect();
                        if self.placements.len() > PLACEMENT_CAP {
                            self.unexamined = (self.placements.len() - PLACEMENT_CAP) as u64;
                            self.placements.truncate(PLACEMENT_CAP);
                        }
                    }
                    self.stage = 1;
                    self.next = 0;
                }
                1 => {
                    let Some(path) = self.placements.get(self.next).cloned() else {
                        self.stage = 2;
                        self.next = 0;
                        continue;
                    };
                    ev.spend(0)?;
                    self.address(ev, doc, root, &path)?;
                    self.next += 1;
                }
                2 => {
                    let Some(list) = self.pointer_lists.get(self.next).cloned() else {
                        self.stage = 3;
                        self.next = 0;
                        continue;
                    };
                    self.offsets(ev, doc, &list)?;
                    self.next += 1;
                }
                3 => {
                    let Some(list) = self.gathers.get(self.next).cloned() else {
                        self.stage = 4;
                        self.next = 0;
                        self.order = self.found.keys().cloned().collect();
                        continue;
                    };
                    self.descriptors(ev, doc, &list)?;
                    self.next += 1;
                }
                _ => {
                    let Some(dir) = self.order.get(self.next).cloned() else {
                        self.done = true;
                        continue;
                    };
                    let d = describe(ev, doc, &dir, &self.found[&dir])?;
                    self.lists.push(d);
                    self.next += 1;
                }
            }
        }
        Ok(())
    }

    /// An address at `path`: the directory is the list whose element it is
    /// in, at whatever depth.
    fn address<S: Source>(&mut self, ev: &mut Evaluator, doc: &Document<S>, root: &[usize], path: &[usize]) -> R<()> {
        if !path.starts_with(root) {
            return Ok(());
        }
        match ev.resolve(doc, path) {
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => return Ok(()),
            Ok(()) => {}
        }
        // A chain's or a gather's element is indexed too, as a stretch of its
        // own; those are the other two sources' business.
        if !matches!(ev.memo[path].ty, Ty::At { .. }) {
            return Ok(());
        }
        let mut target = path.to_vec();
        target.push(0);
        for k in (root.len() + 1..=path.len()).rev() {
            let parent = &path[..k - 1];
            match ev.resolve(doc, parent) {
                Err(e) if e.interrupted() => return Err(e),
                Err(_) => return Ok(()),
                Ok(()) => {}
            }
            if is_list(&ev.memo[parent].ty) {
                let aside = ev.aside(path);
                self.add(parent.to_vec(), path[..k].to_vec(), Pending { path: target, via: "address", aside });
                return Ok(());
            }
        }
        Ok(())
    }

    /// A list of offsets: its children, each placed by the element of the
    /// table beside it at the same index.
    fn offsets<S: Source>(&mut self, ev: &mut Evaluator, doc: &Document<S>, list: &[usize]) -> R<()> {
        ev.resolve(doc, list)?;
        let Ty::PointerList { offsets, .. } = ev.memo[list].ty.clone() else { return Ok(()) };
        let Some(mut table) = ev.find_field(list, &offsets) else { return Ok(()) };
        ev.resolve(doc, &table)?;
        if matches!(ev.memo[&table].ty, Ty::At { .. }) {
            table.push(0);
            ev.resolve(doc, &table)?;
        }
        let n = ev.child_count(doc, list)?.min(ENTRY_CAP as u64) as usize;
        // Every child is placed before any is written down, so a go that
        // stops part way adds nothing twice when it is asked again.
        let mut found = Vec::new();
        for i in 0..n {
            let mut child = list.to_vec();
            child.push(i);
            match ev.resolve(doc, &child).and_then(|()| ev.size_of(doc, &child)) {
                Err(e) if e.interrupted() => return Err(e),
                // An entry that points at nothing, or at something that will
                // not read, places nothing.
                Err(_) => continue,
                Ok(0) if ev.memo[&child].declared_size == Some(0) => continue,
                Ok(_) => {}
            }
            let mut entry = table.clone();
            entry.push(i);
            found.push((entry, child));
        }
        for (entry, child) in found {
            self.add(table.clone(), entry, Pending { path: child, via: "offsets", aside: false });
        }
        Ok(())
    }

    /// A gather: each child with the record that placed it.
    fn descriptors<S: Source>(&mut self, ev: &mut Evaluator, doc: &Document<S>, list: &[usize]) -> R<()> {
        let n = ev.child_count(doc, list)?.min(ENTRY_CAP as u64) as usize;
        let records: Vec<Vec<usize>> = ev.list(list).gather.as_ref().map(|g| g.records.iter().take(n).cloned().collect()).unwrap_or_default();
        for (i, record) in records.into_iter().enumerate() {
            let mut child = list.to_vec();
            child.push(i);
            self.add(list.to_vec(), record, Pending { path: child, via: "descriptors", aside: false });
        }
        Ok(())
    }
}

/// Whether a type is a list, of any of the five kinds.
fn is_list(ty: &Ty) -> bool {
    matches!(ty, Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. })
}

/// Name and measure one directory and the first of its entries.
fn describe<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    dir: &[usize],
    entries: &BTreeMap<Vec<usize>, Vec<Pending>>,
) -> R<Directory> {
    ev.resolve(doc, dir)?;
    let r = ev.memo[dir].clone();
    let name = ev.label(doc, dir, &r)?;
    let elements = match ev.child_count(doc, dir) {
        Ok(n) => n,
        Err(e) if e.interrupted() => return Err(e),
        Err(_) => entries.len() as u64,
    };
    let mut out = Vec::new();
    // In stored order: the entries of one list sort by their index, which is
    // the last step of their path.
    let mut order: Vec<&Vec<usize>> = entries.keys().collect();
    order.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    for entry in order.into_iter().take(ENTRY_CAP) {
        let (name, offset_bits, size_bits) = measure(ev, doc, entry)?;
        let mut targets = Vec::new();
        for t in &entries[entry] {
            let (name, offset_bits, size_bits) = measure(ev, doc, &t.path)?;
            targets.push(DirectoryTarget { path: t.path.clone(), name, offset_bits, size_bits, via: t.via, aside: t.aside });
        }
        out.push(DirectoryEntry { path: entry.clone(), name, offset_bits, size_bits, targets });
    }
    Ok(Directory { path: dir.to_vec(), name, elements, placing: entries.len() as u64, entries: out })
}

/// A node's name, start and size.
fn measure<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<(String, u64, u64)> {
    ev.resolve(doc, path)?;
    let size = ev.size_of(doc, path)?;
    let r = ev.memo[path].clone();
    let name = ev.label(doc, path, &r)?;
    Ok((name, r.offset, size))
}
