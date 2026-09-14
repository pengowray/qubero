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

use super::diagram::{box_identity, box_key, elem_of, is_choice, is_run};
use super::*;

/// How many elements of one run the walk looks inside. Past this the run is
/// counted rather than walked: they are all the same type, so the count is
/// known, and what is inside one of them is what is inside the first.
const RUN_SAMPLE: usize = 32;

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

/// The key of a type, written out once and remembered.
///
/// A free function rather than a closure over the evaluator: the walk needs the
/// evaluator mutably between two of these, and a closure holding the template
/// would keep it borrowed across both.
fn key_for(t: &Template, ty: &Ty, keys: &mut FxHashMap<usize, Option<String>>) -> Option<String> {
    match box_identity(t, ty) {
        Some(id) => keys.entry(id).or_insert_with(|| box_key(t, ty)).clone(),
        None => box_key(t, ty),
    }
}

/// One node waiting to be walked, and which row of which box it stands on.
struct Waiting {
    path: Vec<usize>,
    /// The box its parent is drawn as, and its index among that parent's rows.
    /// None for the node the walk started at, which stands on nobody's row.
    row: Option<(String, usize)>,
    /// True when the run this came out of already counted it, so walking it is
    /// for what is inside it and it must not be counted again.
    counted: bool,
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
        queue.push_back(Waiting { path: Vec::new(), row: None, counted: false });
        let mut walked: u64 = 0;
        let mut truncated = false;
        // The key of a type is the type written out, and writing one out per
        // node is what a census costs if nothing is remembered: a WAV of five
        // thousand nodes took two minutes before this. Keyed by the pointer the
        // IR already shares, so each type is written out once.
        let mut keys: FxHashMap<usize, Option<String>> = FxHashMap::default();
        while let Some(at) = queue.pop_front() {
            if walked as usize >= limit {
                truncated = true;
                break;
            }
            walked += 1;
            match self.count_node(doc, &at, &mut boxes, &mut rows, &mut queue, limit, &mut keys, &mut truncated) {
                Ok(()) => {}
                // The bytes are not here yet, or this go's allowance ran out.
                // Neither is a reason to answer nothing: what has been counted
                // is true, and `truncated` says it is a floor. Returning the
                // error instead would leave a view that only ever asks once
                // with no counts at all, which is what happened.
                Err(e) if e.interrupted() => {
                    truncated = true;
                    break;
                }
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
        keys: &mut FxHashMap<usize, Option<String>>,
        truncated: &mut bool,
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
        // The choice this node made, for a node that is one: a switch is a box
        // of its own and its row is the case taken.
        let chose = self.case_taken(&declared, &r.ty);
        if is_choice(&self.template, &declared) {
            if let Some(key) = key_for(&self.template, &declared, keys) {
                // The choice was made, whether or not which way can be named:
                // counting it only when the case is known left the box reading
                // "this file has none of these" over a file that takes it on
                // every chunk.
                count(&key, boxes);
                if let Some(row) = chose {
                    let e = rows.entry((key.clone(), row)).or_insert_with(|| RowCount {
                        key: key.clone(),
                        row,
                        count: 0,
                        first_path: at.path.clone(),
                        space: r.space,
                    });
                    e.count += 1;
                }
            }
        }
        // Whether this node is a run of something rather than one of it.
        //
        // Asked of what it turned out to be as well as of its declaration,
        // because a switch that picks a list is a list: a WAV's `data` chunk is
        // declared as a choice and resolves to half a million samples, and read
        // only off the declaration it was not a run, so its elements were
        // neither capped nor counted in bulk. That one miss was the whole cost
        // of a census on a WAV.
        let run = if is_run(&self.template, &declared) {
            Some(declared.clone())
        } else if is_run(&self.template, &r.ty) {
            Some(r.ty.clone())
        } else {
            None
        };
        // Which box this node's own children are rows of. A run's children are
        // its elements and stand on nobody's row; counting the run as one of
        // them would make every list in the file one longer than it is.
        let own = match &run {
            Some(_) => None,
            // A node declared as a choice *is* whatever it resolved to, whether
            // or not the case it came from could be named. Falling back to the
            // declaration here was a quiet fault: the node then counted as the
            // choice, and its children — the fields of the shape the choice
            // picked — stood on the choice's rows, so a PNG's chunk header put
            // its `width` on the row that says `'IHDR'` and the box for IHDR
            // itself was never counted at all.
            None if is_choice(&self.template, &declared) => key_for(&self.template, &r.ty, keys),
            None => key_for(&self.template, &declared, keys),
        };
        if let Some(key) = &own {
            if !at.counted {
                count(key, boxes);
            }
        }
        // The children, each knowing the row of that box it stands on. Counted
        // against what there is room for, so a list of a million elements
        // queues what the cap allows rather than a million paths.
        let room = limit.saturating_sub(queue.len());
        if room == 0 {
            return Ok(());
        }
        let known = self.count_unless_walk(doc, &at.path)?;
        let count = match known {
            Some(n) => n,
            // A run only walking settles the length of: ask for as many as
            // there is room for and stop at the first that will not place.
            None => room as u64,
        };
        // A long run counted rather than walked.
        //
        // Every element of a run is declared the same way, so a WAV's half a
        // million samples are half a million of one box and the count is known
        // the moment the length is. Walking them to find that out cost a
        // hundred seconds on a five-thousand-sample file and spent the whole
        // budget on one list, so the ones past `RUN_SAMPLE` are counted here
        // and not walked. Their own insides stay a floor, which is what
        // `truncated` says.
        //
        // Not for a run of choices: what one of those turns out to be is not
        // settled by the declaration, so counting them all as the first one
        // would be a number the file does not say.
        let mut bulk = false;
        if let (Some(n), Some(elem)) = (known, run.as_ref().and_then(|t| elem_of(&self.template, t))) {
            if n as usize > RUN_SAMPLE && !is_choice(&self.template, elem) {
                if let Some(key) = key_for(&self.template, elem, keys) {
                    let e = boxes.entry(key.clone()).or_insert_with(|| BoxCount {
                        key,
                        count: 0,
                        first_path: {
                            let mut p = at.path.clone();
                            p.push(0);
                            p
                        },
                        space: r.space,
                    });
                    e.count += n;
                    bulk = true;
                }
            }
        }
        // A run's children are capped whatever its length turned out to be.
        // They are all the same type, so past a handful the walk is paying per
        // element for a fact it already has; and a run whose length only
        // walking settles would otherwise queue the whole budget. A structure's
        // children are its fields and are never capped: each is a different row
        // and dropping one drops a row of the picture.
        let cap = if run.is_some() { room.min(RUN_SAMPLE) } else { room };
        let taken = cap.min(usize::try_from(count).unwrap_or(usize::MAX));
        // What was not walked is what `truncated` is for. A run counted in bulk
        // has its own count right; what is inside the elements past the sample
        // is what is short.
        if (taken as u64) < count || (known.is_none() && taken == cap) {
            *truncated = true;
        }
        for i in 0..taken {
            let mut child = at.path.clone();
            child.push(i);
            if self.resolve(doc, &child).is_err() {
                break;
            }
            queue.push_back(Waiting { path: child, row: own.clone().map(|k| (k, i)), counted: bulk });
        }
        Ok(())
    }

    /// Which case of a switch a node took, as a row of the switch's box.
    ///
    /// The resolved type is what the case picked, so the case is the one it
    /// came from. Asked twice, cheaply, rather than once expensively:
    ///
    /// 1. **By pointer.** Resolving a switch clones the case's type, and a
    ///    `Ty::Struct` clone shares its `Arc`, so the case a node took is the
    ///    one whose structure is literally the same object. Exact, and free.
    /// 2. **By the type column's own words**, for the cases that are not
    ///    structures: `u16 le` is not `u16 be`, which is the whole of what
    ///    tells two numeric cases apart.
    ///
    /// Not by the rendering the boxes are keyed on, which was the first way
    /// this was written: that renders the whole of a resolved type per node,
    /// and a WAV of five thousand nodes took over two minutes to count.
    ///
    /// Nothing for a type that is not a switch. A switch whose default was
    /// taken answers with the last row, which is where the default is drawn.
    fn case_taken(&self, declared: &Ty, resolved: &Ty) -> Option<usize> {
        let cases: Vec<&Ty> = match declared {
            Ty::Switch { cases, default, .. } => cases.iter().map(|(_, t)| t).chain(Some(&**default)).collect(),
            Ty::Match { cases, default, .. } => cases.iter().map(|(_, t)| t).chain(Some(&**default)).collect(),
            _ => return None,
        };
        if let Some(got) = super::diagram::struct_of(&self.template, resolved) {
            if let Some(i) = cases
                .iter()
                .position(|c| super::diagram::struct_of(&self.template, c).is_some_and(|c| std::sync::Arc::ptr_eq(c, got)))
            {
                return Some(i);
            }
        }
        let want = resolved.display_name();
        cases.iter().position(|c| c.display_name() == want)
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
    fn a_long_run_is_counted_in_full_without_being_walked() {
        // A thousand elements, and a walk allowed a hundred nodes. The count is
        // still a thousand, because every element of a run is the same type and
        // the length says how many: walking them would only spend the budget
        // arriving at a number already known.
        let elem = T::structure("Sample", vec![("left", T::u8()), ("right", T::u8())]);
        let root = T::structure("root", vec![("samples", T::Array { elem: Box::new(elem), count: E::lit(1000) })]);
        let t = Template::new("test", root);
        let d = crate::eval::diagram(&t);
        let sample = d.types.iter().find(|b| b.name == "Sample").expect("a Sample box");
        let (mut ev, doc) = read(t, &vec![7u8; 2000]);
        let c = ev.census(&doc, 100).expect("a census");
        assert_eq!(c.boxes.iter().find(|b| b.key == sample.key).map(|b| b.count), Some(1000));
        assert!(c.walked <= 100, "walked {} nodes for a thousand samples", c.walked);
        // And the first is still where it is, so a reader can be taken to it.
        assert_eq!(c.boxes.iter().find(|b| b.key == sample.key).map(|b| b.first_path.clone()), Some(vec![0, 0]));
    }

    #[test]
    fn a_chunk_of_a_cartridge_png_is_counted_as_the_shape_it_is() {
        // The PICO-8 cartridge reads a PNG's chunks, and each chunk's body is a
        // choice wrapped in a window. The node is whatever the choice picked,
        // and its fields are that shape's rows: counted as the choice instead,
        // a header's `width` stood on the row that says `'IHDR'`.
        let Some(dir) = std::env::var_os("QUBERO_SAMPLES") else { return };
        let path = std::path::Path::new(&dir).join("pico8/p8png-test.p8.png");
        let Ok(bytes) = std::fs::read(&path) else { return };
        let Some(t) = crate::formats::builtin("p8png") else { return };
        let d = crate::eval::diagram(&t);
        let (mut ev, doc) = read(t, &bytes);
        let c = ev.census(&doc, 20_000).expect("a census");
        let by_key: std::collections::HashMap<&str, u64> =
            c.boxes.iter().map(|b| (b.key.as_str(), b.count)).collect();
        for name in ["IHDR", "tEXt"] {
            let Some(b) = d.types.iter().find(|b| b.name == name) else { continue };
            assert_eq!(by_key.get(b.key.as_str()).copied(), Some(1), "{name} is in this file exactly once");
        }
        // No box is counted more often than there are chunks to hold one. A
        // shape reached once per chunk is four; anything past that would be a
        // node counted twice over.
        let chunks = d
            .types
            .iter()
            .find(|b| b.name == "Chunk")
            .and_then(|b| by_key.get(b.key.as_str()).copied())
            .unwrap_or(0);
        for b in &d.types {
            let got = by_key.get(b.key.as_str()).copied().unwrap_or(0);
            assert!(got <= chunks.max(1), "{} counted {got} times in a file of {chunks} chunks", b.name);
        }
        // And the choice's own rows are its cases, counted once per chunk that
        // took one, never once per field of the shape it picked.
        let Some(sw) = d.types.iter().find(|b| b.kind == crate::eval::BoxKind::Switch && b.name.contains("type"))
        else {
            return;
        };
        let taken: u64 = c.rows.iter().filter(|r| r.key == sw.key).map(|r| r.count).sum();
        let chunks = by_key.get(sw.key.as_str()).copied().unwrap_or(0);
        assert!(taken <= chunks, "{taken} cases taken by {chunks} chunks");
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
