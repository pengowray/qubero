//! What the open file actually has, counted against the diagram's boxes.
//!
//! `diagram.rs` draws the format: every type the template declares, whether or
//! not the file in front of the reader holds one. That is the right picture and
//! it is half the question. A PNG's diagram has a box for every chunk type the
//! format defines, and a reader looking at one particular PNG wants to know
//! which of them this file has, how many, and where the first one is.
//!
//! So this walks the file the evaluator has already read and counts its nodes
//! against those boxes. Three things come back per box: how many the file
//! holds, which row of it each node stood on, and the path to the first one, so
//! the view can badge a box with a count, draw an unused one quietly, and take
//! a reader who asks to the bytes.
//!
//! **Keyed by the diagram's own key.** [`crate::eval::diagram::box_key`] is
//! what the drawing shares boxes by, and it is what this counts by. A census
//! keyed even slightly differently would put real counts on boxes that are not
//! there and leave the drawn ones reading empty, which is worse than no census:
//! a reader would trust it.
//!
//! **Values are not read**, the way [`super::graph`] does not read them: what a
//! field says is the expensive half of reading it and no badge shows it.
//! Breadth-first and capped, for the reason the graph is: a cap on a
//! depth-first walk keeps one deep spine of a file and none of its siblings,
//! and what a reader wants counted first is the top of the file.

use rustc_hash::FxHashMap;

use super::diagram::{box_key, is_run};
use super::*;

/// How many boxes of one kind the file holds, and where the first is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxCount {
    /// The diagram box this counts, by [`crate::eval::diagram::box_key`].
    pub key: String,
    pub count: u64,
    /// The path to the first one the walk met, in the order the walk met them,
    /// which breadth-first makes the shallowest rather than merely the first
    /// found.
    pub first_path: Vec<usize>,
    /// Which reading the path is in: 0 is the file, anything else an unpacked
    /// stream. See [`super::space`].
    pub space: u32,
}

/// The same for one row of one box: one field of a type, counted over every
/// node of that type the file holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowCount {
    pub key: String,
    pub row: usize,
    pub count: u64,
    pub first_path: Vec<usize>,
    pub space: u32,
}

/// What the file holds, against what the format can hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Census {
    pub boxes: Vec<BoxCount>,
    pub rows: Vec<RowCount>,
    /// How many nodes the walk looked at.
    pub walked: u64,
    /// True when the cap stopped it, so every count is a floor rather than a
    /// total. A view that does not say so is showing a number that looks like
    /// an answer.
    pub truncated: bool,
}

/// One node waiting to be walked, and which row of which box it stands on.
struct Waiting {
    path: Vec<usize>,
    /// The box its parent is drawn as, and its index among that parent's rows.
    /// None for the node the walk started at, which stands on nobody's row.
    row: Option<(String, usize)>,
}

impl Evaluator {
    /// Count the open file's nodes against the diagram's boxes, stopping after
    /// `limit` of them.
    ///
    /// A node that will not read is passed over rather than taking the census
    /// with it, the same way the graph carries on past a field it cannot place:
    /// a count that is short by one broken record is worth more than no count.
    pub fn census<S: Source>(&mut self, doc: &Document<S>, limit: usize) -> R<Census> {
        let mut boxes: FxHashMap<String, BoxCount> = FxHashMap::default();
        let mut rows: FxHashMap<(String, usize), RowCount> = FxHashMap::default();
        let mut queue: std::collections::VecDeque<Waiting> = std::collections::VecDeque::new();
        queue.push_back(Waiting { path: Vec::new(), row: None });
        let mut walked: u64 = 0;
        let mut truncated = false;
        while let Some(at) = queue.pop_front() {
            if walked as usize >= limit {
                truncated = true;
                break;
            }
            walked += 1;
            match self.count_node(doc, &at, &mut boxes, &mut rows, &mut queue, limit) {
                Ok(()) => {}
                // The bytes are not here or the go ran out. Either is the
                // caller's to ask again about, and a half-counted file is not
                // an answer to give as a whole one.
                Err(e) if e.interrupted() => return Err(e),
                Err(_) => {}
            }
        }
        // Biggest first, so a view that shows some of them shows the ones worth
        // showing, and so two runs over one file answer in the same order.
        let mut boxes: Vec<BoxCount> = boxes.into_values().collect();
        boxes.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
        let mut rows: Vec<RowCount> = rows.into_values().collect();
        rows.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| (&a.key, a.row).cmp(&(&b.key, b.row))));
        Ok(Census { boxes, rows, walked, truncated })
    }

    /// One node: which box it is, which row it stands on, and its children
    /// queued behind it.
    fn count_node<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &Waiting,
        boxes: &mut FxHashMap<String, BoxCount>,
        rows: &mut FxHashMap<(String, usize), RowCount>,
        queue: &mut std::collections::VecDeque<Waiting>,
        limit: usize,
    ) -> R<()> {
        self.resolve(doc, &at.path)?;
        let r = self.memo[&at.path].clone();
        // Charged like every other walk, so a census of a file with millions of
        // fields hands the caller their screen back.
        self.spend(r.offset)?;
        // What the row this node stands on is: the field's own declaration, and
        // not what it turned out to be. The diagram drew the declaration, so a
        // switch is the switch box and not the case it took.
        if let Some((key, row)) = &at.row {
            let e = rows.entry((key.clone(), *row)).or_insert_with(|| RowCount {
                key: key.clone(),
                row: *row,
                count: 0,
                first_path: at.path.clone(),
                space: r.space,
            });
            e.count += 1;
        }
        let declared = self.declared_ty(&at.path)?;
        let count = |key: &str, boxes: &mut FxHashMap<String, BoxCount>| {
            let e = boxes.entry(key.to_string()).or_insert_with(|| BoxCount {
                key: key.to_string(),
                count: 0,
                first_path: at.path.clone(),
                space: r.space,
            });
            e.count += 1;
        };
        // Which box this node's own children are rows of.
        //
        // Three shapes, and they are not the same question. A run is not one of
        // its elements: its children are, and counting the run as one would
        // make every list in the file one longer than it is. A switch is a box
        // of its own *and* the node is whatever case it took, so it counts
        // twice, once against the choice and once against the shape chosen.
        // Everything else is itself.
        let own = if is_run(&self.template, &declared) {
            None
        } else if let Some(row) = self.case_taken(&declared, &r.ty) {
            if let Some(key) = box_key(&self.template, &declared) {
                count(&key, boxes);
                let e = rows.entry((key.clone(), row)).or_insert_with(|| RowCount {
                    key: key.clone(),
                    row,
                    count: 0,
                    first_path: at.path.clone(),
                    space: r.space,
                });
                e.count += 1;
            }
            // The shape the case picked, which is what this node actually is.
            box_key(&self.template, &r.ty)
        } else {
            box_key(&self.template, &declared)
        };
        if let Some(key) = &own {
            count(key, boxes);
        }
        // The children, each knowing the row of that box it stands on. Counted
        // against what there is room for, so a list of a million elements
        // queues what the cap allows rather than a million paths.
        let room = limit.saturating_sub(queue.len());
        if room == 0 {
            return Ok(());
        }
        let count = match self.count_unless_walk(doc, &at.path)? {
            Some(n) => n,
            // A run only walking settles the length of: ask for as many as
            // there is room for and stop at the first that will not place.
            None => room as u64,
        };
        let taken = room.min(usize::try_from(count).unwrap_or(usize::MAX));
        for i in 0..taken {
            let mut child = at.path.clone();
            child.push(i);
            if self.resolve(doc, &child).is_err() {
                break;
            }
            queue.push_back(Waiting { path: child, row: own.clone().map(|k| (k, i)) });
        }
        Ok(())
    }

    /// Which case of a switch a node took, as a row of the switch's box.
    ///
    /// The resolved type is what the case picked, so the case is the one whose
    /// type says the same thing. Written out rather than compared structurally
    /// because that is what the whole scheme is keyed on: two types are one
    /// where they print alike, here and in the drawing both.
    ///
    /// Nothing for a type that is not a switch, and for a switch whose default
    /// was taken, which is the last row. A `Match` is the same question asked
    /// of a string.
    fn case_taken(&self, declared: &Ty, resolved: &Ty) -> Option<usize> {
        let cases: Vec<&Ty> = match declared {
            Ty::Switch { cases, default, .. } => cases.iter().map(|(_, t)| t).chain(Some(&**default)).collect(),
            Ty::Match { cases, default, .. } => cases.iter().map(|(_, t)| t).chain(Some(&**default)).collect(),
            _ => return None,
        };
        let want = crate::template_text::ty_text(resolved);
        cases.iter().position(|c| crate::template_text::ty_text(c) == want)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::source::MemSource;
    use crate::template::{Expr as E, Ty as T};

    fn read(t: Template, bytes: &[u8]) -> (Evaluator, Document<MemSource>) {
        (Evaluator::new(t), Document::new(MemSource(bytes.to_vec())))
    }

    #[test]
    fn a_run_is_counted_once_per_element_and_the_first_is_named() {
        let elem = T::structure("Item", vec![("tag", T::u8()), ("value", T::u8())]);
        let root = T::structure(
            "root",
            vec![("count", T::u8()), ("items", T::Array { elem: Box::new(elem), count: E::field("count") })],
        );
        let t = Template::new("test", root);
        let d = crate::eval::diagram(&t);
        let item = d.types.iter().find(|b| b.name == "Item").expect("an Item box");
        let (mut ev, doc) = read(t, &[3, 1, 2, 3, 4, 5, 6]);
        let c = ev.census(&doc, 1000).expect("a census");
        let got = c.boxes.iter().find(|b| b.key == item.key).expect("Item counted");
        assert_eq!(got.count, 3, "three items");
        assert_eq!(got.first_path, vec![1, 0], "the first is the run's first element");
        // And each of its two fields, once per element.
        let tag = c.rows.iter().find(|r| r.key == item.key && r.row == 0).expect("the tag row");
        assert_eq!(tag.count, 3);
        assert_eq!(tag.first_path, vec![1, 0, 0]);
    }

    #[test]
    fn a_type_the_file_does_not_have_is_counted_nowhere() {
        let a = T::structure("Present", vec![("x", T::u8())]);
        let b = T::structure("Absent", vec![("y", T::u8())]);
        let root = T::structure(
            "root",
            vec![
                ("kind", T::u8()),
                ("body", T::switch(E::field("kind"), vec![(1, a), (2, b)], T::bytes(E::lit(0)))),
            ],
        );
        let t = Template::new("test", root);
        let d = crate::eval::diagram(&t);
        let present = d.types.iter().find(|x| x.name == "Present").expect("a Present box");
        let absent = d.types.iter().find(|x| x.name == "Absent").expect("an Absent box");
        let (mut ev, doc) = read(t, &[1, 9]);
        let c = ev.census(&doc, 1000).expect("a census");
        assert_eq!(c.boxes.iter().find(|x| x.key == present.key).map(|x| x.count), Some(1));
        assert!(c.boxes.iter().all(|x| x.key != absent.key), "a shape the file has none of is not counted");
        // The switch stands on the case it took, which is the first row.
        let sw = d.types.iter().find(|x| x.kind == crate::eval::BoxKind::Switch).expect("a switch box");
        let taken = c.rows.iter().find(|r| r.key == sw.key).expect("a case row");
        assert_eq!((taken.row, taken.count), (0, 1));
    }

    #[test]
    fn a_png_is_counted_by_its_chunks() {
        let Some(dir) = std::env::var_os("QUBERO_SAMPLES") else { return };
        let path = std::path::Path::new(&dir).join("pico8/p8png-test.p8.png");
        let Ok(bytes) = std::fs::read(&path) else { return };
        let Some(t) = crate::formats::builtin("png") else { return };
        let d = crate::eval::diagram(&t);
        let (mut ev, doc) = read(t, &bytes);
        let c = ev.census(&doc, 20_000).expect("a census");
        let chunk = d.types.iter().find(|b| b.name == "Chunk").expect("a Chunk box");
        let ihdr = d.types.iter().find(|b| b.name == "IHDR").expect("an IHDR box");
        let chunks = c.boxes.iter().find(|b| b.key == chunk.key).map(|b| b.count).unwrap_or(0);
        assert!(chunks >= 3, "a PNG has at least a header, some data and an end: {chunks}");
        assert_eq!(c.boxes.iter().find(|b| b.key == ihdr.key).map(|b| b.count), Some(1), "one header");
    }
}
