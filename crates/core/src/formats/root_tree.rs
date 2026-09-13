//! What a ROOT file holds, in the file's own terms: its class descriptions,
//! its trees, and the baskets the events are actually written in.
//!
//! The template places every record the directory lists, names it by its key
//! and unpacks its compressed stream. What it cannot do is read the streamed
//! object inside, because reading one needs the class descriptions in the
//! `StreamerInfo` record and those are themselves streamed. So this sits
//! beside the template the way [`h5ad`](super::h5ad) sits beside HDF5: it
//! walks what the template placed, reads the two records that describe the
//! file, and says what is in them.
//!
//! ## Why the baskets are not placed
//!
//! A `TTree` holds almost none of its own bytes. `uproot-Zmumu-lz4.root` is
//! 212,813 bytes and 206,455 of them, 97 per cent, are baskets: one `TKey`
//! record per branch per few thousand entries, which the directory's key list
//! does not list and nothing in the file points at except `fBasketSeek`, an
//! array of file offsets inside each streamed `TBranch`. The tree's own record
//! is under 5 KB.
//!
//! It is worth being exact about which part of that the IR cannot do, because
//! it is not the part the handover's gap S4 names. An offset read inside a
//! compressed stream that means a place in the file already works: `Ty::At`
//! with `Anchor::File` resolves into space 0 whatever space the field naming
//! it was read in (`eval/mod.rs`, the `space` match in `place_child`), which
//! is how the same template already places an RNTuple's two envelopes from an
//! anchor inside a compressed record.
//!
//! What is missing is one level above that. `fBasketSeek` is not a field of
//! any kind. To make it one, the template would have to describe the streamed
//! `TTree` object, and its layout is not a fact about the format: it is a
//! schema written into another record of the same file, reached by
//! `fSeekInfo`, itself compressed and itself streamed. A template is a fixed
//! structure built in Rust, and no `Ty` takes its shape from bytes. So the IR
//! need is not another kind of pointer list; it is a way for a template to say
//! "read this run with the class description that record holds", which is a
//! larger thing than S4 and is what [`root_streamer`](super::root_streamer)
//! does here instead. The baskets are listed below, with the offset and the
//! length of each, and a reader can go to one.
//!
//! ## What a basket holds
//!
//! A basket is a `TKey` whose header runs on past `fTitle` with five more
//! numbers, of which `fNevBuf` is how many entries it holds and `fLast` where
//! the values stop. Its body, once unpacked, is the values of those entries
//! one after another, big-endian, and then, for a branch whose entries vary in
//! length, a table of where each entry started. The values are read here for
//! the simple case and only that: one leaf, a fixed number of elements per
//! entry, a fixed width each. A branch of variable-length arrays, a branch
//! split into sub-branches, and a `TBranchElement` of a C++ class all have
//! their baskets listed and their values left, and each says which of those it
//! is rather than failing.

use crate::codec::{self, Codec};
use crate::document::Document;
use crate::eval::{EvalError, Evaluator, Value, R};
use crate::source::Source;

use super::root_streamer::{self, Obj, Schema, Val};

/// How many trees one walk reports, and how many branches of one tree. A file
/// people write has one tree with a few hundred branches; a claim far past
/// that is a reason to stop rather than a reason to allocate.
pub const TREE_LIMIT: usize = 64;
pub const BRANCH_LIMIT: usize = 4096;
/// How many baskets of one branch are listed, and how many across a whole
/// tree. A branch of a real analysis file has thousands and an experiment's
/// tree has a thousand branches, so the second of these is what actually
/// bounds the walk; each branch still says how many baskets it has.
pub const BASKET_LIMIT: usize = 4096;
pub const TREE_BASKET_LIMIT: usize = 100_000;

/// How deep the directory walk goes, which matches the depth the template
/// itself stops at.
const MAX_DEPTH: usize = 6;

/// How deep the branches under a branch go. A tree split all the way down to
/// the members of a nested C++ class is a few levels; this is far past any of
/// them and stops a file whose branches point at each other.
const MAX_BRANCH_DEPTH: usize = 12;

/// The largest record this reads whole. A `TTree`'s own record is kilobytes
/// and a basket is at most a few megabytes; a key claiming more than this is
/// one to refuse rather than allocate for.
const MAX_RECORD: usize = 64 << 20;

/// One member of a class, as the file's own description of that class gives
/// it.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub name: String,
    /// The C++ type as the file spells it: `double`, `TObjArray`,
    /// `long long*`.
    pub type_name: String,
    /// ROOT's type code, which says how the member is written rather than what
    /// it is called.
    pub code: i32,
    /// How many bytes one of them takes.
    pub size: i32,
    /// The dimensions of a fixed array, empty for a single value.
    pub dims: Vec<i32>,
    /// True where the member is a base class rather than a member of its own.
    pub base: bool,
    pub comment: String,
}

/// One class description out of the `StreamerInfo` record.
#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub name: String,
    pub version: i32,
    /// A number over the layout, which is how ROOT tells two versions of a
    /// class apart when neither bumped its version number.
    pub checksum: u32,
    pub members: Vec<Member>,
}

/// One leaf: the thing inside a branch that says what one entry looks like.
#[derive(Debug, Clone, PartialEq)]
pub struct Leaf {
    pub name: String,
    /// `TLeafI`, `TLeafD`, `TLeafC`: the class says the element type.
    pub class: String,
    /// How many elements one entry holds, where that is fixed.
    pub len: i64,
    /// How many bytes one element takes.
    pub width: i64,
    pub unsigned: bool,
    /// The leaf that counts this one, for a branch whose entries vary in
    /// length. Empty where the count is fixed.
    pub counted_by: String,
}

/// One basket: a record of the file that the directory does not list.
#[derive(Debug, Clone, PartialEq)]
pub struct Basket {
    /// Where the key is, from the branch's `fBasketSeek`.
    pub at: u64,
    /// How many bytes it takes up, from `fBasketBytes`. Zero for a basket the
    /// branch has room for and has not written.
    pub bytes: i64,
    /// The first entry in it, from `fBasketEntry`.
    pub first_entry: i64,
    /// How many entries it holds, which is the next `fBasketEntry` less this
    /// one.
    pub entries: i64,
}

/// Whether a branch's values can be read, and where not, why not.
#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    /// One leaf, a fixed count of elements per entry, a fixed width each.
    Fixed { width: i64, per_entry: i64, floating: bool, unsigned: bool },
    /// Not that. The text says which of the several reasons it is.
    Not(String),
}

/// One branch of a tree, and the sub-branches under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    /// The name as the file writes it, with the names of the branches above it
    /// in front: `Muon/Muon.pt`.
    pub name: String,
    pub title: String,
    /// `TBranch` for a plain number, `TBranchElement` for a member of a C++
    /// class the tree was split into.
    pub class: String,
    /// How far under a top-level branch this one is.
    pub depth: usize,
    pub entries: i64,
    /// How many bytes the branch's baskets take unpacked, and packed.
    pub total_bytes: i64,
    pub zip_bytes: i64,
    /// How long a basket's entry offset table is, or zero for a branch whose
    /// entries are all the same length.
    pub entry_offset_len: i64,
    pub leaves: Vec<Leaf>,
    pub baskets: Vec<Basket>,
    /// How many baskets there are, where that is more than were listed.
    pub basket_total: usize,
    pub reading: Reading,
}

/// One tree of the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    /// Where its key is in the template, so picking it can move the cursor.
    pub path: Vec<usize>,
    pub name: String,
    pub title: String,
    pub entries: i64,
    /// Where the tree's own record is.
    pub at: u64,
    pub branches: Vec<Branch>,
    /// How many branches there were, where that is more than were listed.
    pub branch_total: usize,
    /// What stopped the reading part way, if anything did.
    pub trouble: Option<String>,
}

/// What the walk made of the file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Contents {
    /// The class descriptions, in the order the file lists them.
    pub classes: Vec<Class>,
    /// Where the `StreamerInfo` record is in the template.
    pub schema_path: Vec<usize>,
    pub trees: Vec<Tree>,
    /// How many trees there were, where that is more than were listed.
    pub tree_total: usize,
    /// What stopped the walk, if anything did. Classes and trees read before
    /// it are still here.
    pub trouble: Option<String>,
}

/// One basket read: what its own header says and, where the branch is simple
/// enough, the values in it.
#[derive(Debug, Clone, PartialEq)]
pub struct BasketData {
    /// How many entries the basket holds, from `fNevBuf`.
    pub entries: i64,
    /// Where the values stop and the entry offsets begin, as an offset into
    /// the unpacked body.
    pub border: usize,
    /// How many bytes the unpacked body comes to.
    pub unpacked: usize,
    /// Where each entry starts, for a branch whose entries vary in length.
    /// Empty where they do not.
    pub offsets: Vec<i32>,
    pub values: Values,
}

/// The values of one basket, in whichever shape the leaf's type gives them.
#[derive(Debug, Clone, PartialEq)]
pub enum Values {
    Ints(Vec<i64>),
    Floats(Vec<f64>),
    /// Not read, and why.
    None(String),
}

/// Walk the file and say what is in it.
///
/// The schema is read first because everything else needs it. A file with no
/// `StreamerInfo` record, which is a file written before the record existed,
/// comes back with its trouble said and no trees: without the descriptions
/// there is no way to read a `TTree`, and guessing at one is how a reader
/// invents branches that are not there.
pub fn contents<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Contents> {
    let mut out = Contents::default();
    let schema = match schema_of(ev, doc, &mut out)? {
        Some(s) => s,
        None => return Ok(out),
    };
    let mut keys = Vec::new();
    if let Some(root) = top_directory(ev, doc)? {
        collect_keys(ev, doc, &root, "", &mut keys, 0)?;
    }
    out.tree_total = keys.len();
    for key in keys.into_iter().take(TREE_LIMIT) {
        out.trees.push(read_tree(doc, &schema, key)?);
    }
    Ok(out)
}

/// A key of a directory that names a tree: where it is, what it is called, and
/// where the template put it.
struct TreeKey {
    path: Vec<usize>,
    name: String,
    title: String,
    at: u64,
}

/// The class descriptions, read out of the record the header's `fSeekInfo`
/// points at.
fn schema_of<S: Source>(ev: &mut Evaluator, doc: &Document<S>, out: &mut Contents) -> R<Option<Schema>> {
    // Both of these sit alone on the panel's summary line, where the host
    // adds what they cost (`trees not read`), so they say the fact and not
    // the consequence. The second is the one a real file produces: the
    // template places the record only where `fSeekInfo` is set.
    let Some(record) = ev.child_named(doc, &[], "streamer_info")? else {
        out.trouble = Some("no StreamerInfo record".into());
        return Ok(None);
    };
    let Some(inside) = first_child(ev, doc, &record)? else {
        out.trouble = Some("no StreamerInfo record (fSeekInfo is 0)".into());
        return Ok(None);
    };
    out.schema_path = inside.clone();
    let at = ev.node(doc, &inside)?.offset_bits / 8;
    let record = match read_record(doc, at) {
        Ok(r) => r,
        // Bytes still on their way are not a fault in the file: the caller
        // fetches them and asks again. Anything else is this record being
        // unreadable, and is said rather than thrown.
        Err(EvalError::Failed(why)) => {
            out.trouble = Some(format!("StreamerInfo record could not be read: {why}"));
            return Ok(None);
        }
        Err(other) => return Err(other),
    };
    let schema = match root_streamer::read_streamer_info(&record.body, record.key_len) {
        Ok(s) => s,
        Err(why) => {
            out.trouble = Some(format!("StreamerInfo record could not be read: {why}"));
            return Ok(None);
        }
    };
    for info in &schema.infos {
        out.classes.push(Class {
            name: info.class.clone(),
            version: info.version,
            checksum: info.checksum,
            members: info
                .elements
                .iter()
                .map(|e| Member {
                    name: e.name.clone(),
                    type_name: e.type_name.clone(),
                    code: e.etype,
                    size: e.size,
                    // A base class puts the base's checksum where a dimension
                    // would be, so its dimensions are never read as sizes.
                    dims: match e.is_base() {
                        true => Vec::new(),
                        false => e.max_index[..(e.array_dim.clamp(0, 5) as usize)].to_vec(),
                    },
                    base: e.is_base(),
                    comment: e.title.clone(),
                })
                .collect(),
        });
    }
    Ok(Some(schema))
}

/// The `Directory` structure of the file's top directory.
fn top_directory<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Option<Vec<usize>>> {
    let Some(at) = ev.child_named(doc, &[], "directory")? else { return Ok(None) };
    let Some(record) = first_child(ev, doc, &at)? else { return Ok(None) };
    ev.child_named(doc, &record, "directory")
}

/// Every key under `dir` that names a tree, and the same for the directories
/// under it. `prefix` is the path of directory names above this one, which is
/// how a tree in `one/two` is told from one in `three`.
fn collect_keys<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    dir: &[usize],
    prefix: &str,
    out: &mut Vec<TreeKey>,
    depth: usize,
) -> R<()> {
    if depth > MAX_DEPTH || out.len() >= TREE_LIMIT * 4 {
        return Ok(());
    }
    let Some(at) = ev.child_named(doc, dir, "keys")? else { return Ok(()) };
    let Some(list) = first_child(ev, doc, &at)? else { return Ok(()) };
    let Some(keys) = ev.child_named(doc, &list, "keys")? else { return Ok(()) };
    let n = ev.node(doc, &keys)?.child_count.min(BRANCH_LIMIT as u64);
    for i in 0..n {
        let mut entry = keys.clone();
        entry.push(i as usize);
        let class = field_text(ev, doc, &entry, "fClassName")?;
        let name = field_text(ev, doc, &entry, "fName")?;
        let full = match prefix.is_empty() {
            true => name.clone(),
            false => format!("{prefix}/{name}"),
        };
        if class == "TTree" || class == "TNtuple" || class == "TNtupleD" {
            out.push(TreeKey {
                path: entry.clone(),
                name: full,
                title: field_text(ev, doc, &entry, "fTitle")?,
                at: field_int(ev, doc, &entry, "fSeekKey")?.unwrap_or(0).max(0) as u64,
            });
            continue;
        }
        if class != "TDirectory" && class != "TDirectoryFile" {
            continue;
        }
        // A directory key points at a record holding another directory, which
        // the template follows down to its own depth.
        let Some(at) = ev.child_named(doc, &entry, "record")? else { continue };
        let Some(record) = first_child(ev, doc, &at)? else { continue };
        let Some(inner) = ev.child_named(doc, &record, "directory")? else { continue };
        collect_keys(ev, doc, &inner, &full, out, depth + 1)?;
    }
    Ok(())
}

/// One tree: its record read, its object decoded, its branches walked.
fn read_tree<S: Source>(doc: &Document<S>, schema: &Schema, key: TreeKey) -> R<Tree> {
    let mut tree = Tree {
        path: key.path,
        name: key.name,
        title: key.title,
        entries: 0,
        at: key.at,
        branches: Vec::new(),
        branch_total: 0,
        trouble: None,
    };
    let record = match read_record(doc, key.at) {
        Ok(r) => r,
        Err(EvalError::Failed(why)) => {
            tree.trouble = Some(why);
            return Ok(tree);
        }
        Err(other) => return Err(other),
    };
    let mut cur = root_streamer::Cursor::new(&record.body, record.key_len);
    let object = match root_streamer::read_class(&mut cur, schema, &record.class, 0) {
        Ok(o) => o,
        Err(why) => {
            tree.trouble = Some(why);
            return Ok(tree);
        }
    };
    tree.trouble = object.trouble.clone();
    tree.entries = object.int("fEntries").unwrap_or(0);
    if tree.name.is_empty() {
        tree.name = object.str("fName").to_string();
    }
    let mut branches = Vec::new();
    let mut total = 0usize;
    let mut room = TREE_BASKET_LIMIT;
    if let Some(list) = object.obj("fBranches") {
        walk_branches(list, "", 0, &mut branches, &mut total, &mut room);
    }
    tree.branch_total = total;
    tree.branches = branches;
    Ok(tree)
}

/// Every branch of a `TObjArray` of them, and the branches under each.
fn walk_branches(
    list: &Obj,
    prefix: &str,
    depth: usize,
    out: &mut Vec<Branch>,
    total: &mut usize,
    room: &mut usize,
) {
    for item in list.list("elements").iter().flatten() {
        *total += 1;
        if out.len() >= BRANCH_LIMIT || depth > MAX_BRANCH_DEPTH {
            continue;
        }
        let name = item.str("fName");
        let full = match prefix.is_empty() {
            true => name.to_string(),
            false => format!("{prefix}/{name}"),
        };
        out.push(branch_of(item, &full, depth, room));
        if let Some(kids) = item.obj("fBranches") {
            walk_branches(kids, &full, depth + 1, out, total, room);
        }
    }
}

fn branch_of(obj: &Obj, name: &str, depth: usize, room: &mut usize) -> Branch {
    let leaves = leaves_of(obj);
    let entry_offset_len = obj.int("fEntryOffsetLen").unwrap_or(0);
    let split = obj.obj("fBranches").map(|b| b.list("elements").len()).unwrap_or(0);
    let mut branch = Branch {
        name: name.to_string(),
        title: obj.str("fTitle").to_string(),
        class: obj.class.clone(),
        depth,
        entries: obj.int("fEntries").unwrap_or(0),
        total_bytes: obj.int("fTotBytes").unwrap_or(0),
        zip_bytes: obj.int("fZipBytes").unwrap_or(0),
        entry_offset_len,
        reading: how_to_read(obj, &leaves, entry_offset_len, split),
        leaves,
        baskets: Vec::new(),
        basket_total: 0,
    };
    // `fWriteBasket` is how many have been written; the three arrays are all
    // `fMaxBaskets` long and the rest of each is zero.
    let written = obj.int("fWriteBasket").unwrap_or(0).max(0) as usize;
    let seeks = obj.ints("fBasketSeek");
    let bytes = obj.ints("fBasketBytes");
    let entry = obj.ints("fBasketEntry");
    let count = written.min(seeks.len());
    branch.basket_total = count;
    // Per branch and across the whole tree. A NanoAOD has a thousand branches
    // of a few hundred baskets each, and a list of every one of them is a
    // million rows nobody asked for; `basket_total` still says how many there
    // are.
    let listed = count.min(BASKET_LIMIT).min(*room);
    *room -= listed;
    for i in 0..listed {
        let first = entry.get(i).copied().unwrap_or(0);
        let next = entry.get(i + 1).copied().unwrap_or(branch.entries);
        branch.baskets.push(Basket {
            at: seeks[i].max(0) as u64,
            bytes: bytes.get(i).copied().unwrap_or(0),
            first_entry: first,
            entries: (next - first).max(0),
        });
    }
    branch
}

fn leaves_of(obj: &Obj) -> Vec<Leaf> {
    let Some(list) = obj.obj("fLeaves") else { return Vec::new() };
    let mut out = Vec::new();
    for leaf in list.list("elements").iter().flatten() {
        out.push(Leaf {
            name: leaf.str("fName").to_string(),
            class: leaf.class.clone(),
            len: leaf.int("fLen").unwrap_or(0),
            width: leaf.int("fLenType").unwrap_or(0),
            unsigned: leaf.int("fIsUnsigned").unwrap_or(0) != 0,
            counted_by: match leaf.get("fLeafCount") {
                Some(Val::Obj(count)) => count.str("fName").to_string(),
                _ => String::new(),
            },
        });
    }
    out
}

/// Whether this branch's baskets hold values something can read straight
/// through, and where not, which of the reasons it is.
///
/// Each of these is a real shape in a real file, not a guard against the
/// impossible: `uproot-small-flat-tree.root` has a fixed array branch, a
/// variable-length one and a string branch side by side, and a file from an
/// experiment is mostly split `TBranchElement`s.
///
/// The sentence in a `Not` stands where `signed 32-bit × 10` would on the
/// panel's branch row, and that row is cut off at the rail's width, so each
/// says what the branch holds first and `not read here` last. A split branch
/// says neither: its values are in the sub-branches listed under it, and
/// nothing is withheld.
fn how_to_read(obj: &Obj, leaves: &[Leaf], entry_offset_len: i64, split: usize) -> Reading {
    if split > 0 {
        return Reading::Not(format!("split into {split} sub-branches"));
    }
    if obj.class != "TBranch" {
        return Reading::Not(format!("C++ objects in a {}, not read here", obj.class));
    }
    if leaves.len() != 1 {
        return Reading::Not(format!("{} leaves, not read here", leaves.len()));
    }
    let leaf = &leaves[0];
    if !leaf.counted_by.is_empty() {
        return Reading::Not(format!("{} values per entry, not read here", leaf.counted_by));
    }
    if entry_offset_len != 0 {
        return Reading::Not("variable-length entries, not read here".into());
    }
    if leaf.class == "TLeafC" {
        return Reading::Not("one string per entry, not read here".into());
    }
    if leaf.len <= 0 || leaf.width <= 0 {
        return Reading::Not("leaf with no width, not read here".into());
    }
    let floating = matches!(leaf.class.as_str(), "TLeafF" | "TLeafD" | "TLeafF16" | "TLeafD32");
    if !floating && !matches!(leaf.class.as_str(), "TLeafB" | "TLeafS" | "TLeafI" | "TLeafL" | "TLeafO" | "TLeafG") {
        return Reading::Not(format!("{} leaves, not read here", leaf.class));
    }
    Reading::Fixed { width: leaf.width, per_entry: leaf.len, floating, unsigned: leaf.unsigned }
}

/// Read one basket: its own header, its entry offsets, and, where the branch
/// allows it, the values.
///
/// `at` is a `fBasketSeek` from the branch. Nothing in the template placed it,
/// so this reads the key itself.
pub fn read_basket<S: Source>(doc: &Document<S>, at: u64, reading: &Reading) -> R<BasketData> {
    let record = read_record(doc, at)?;
    let Some(head) = record.basket else {
        return Err(EvalError::Failed(format!("key at @0x{at:x} is not a TBasket")));
    };
    let border = (head.last - record.key_len as i64).max(0) as usize;
    let border = border.min(record.body.len());
    let mut data = BasketData {
        entries: head.entries,
        border,
        unpacked: record.body.len(),
        offsets: Vec::new(),
        values: Values::None(String::new()),
    };
    // What follows the values, where anything does, is one four-byte offset
    // per entry and one more for the end. The offsets count from the start of
    // the key, so the key's length comes off each.
    if border < record.body.len() {
        let table = &record.body[border..];
        let mut offsets = Vec::new();
        for chunk in table.chunks_exact(4) {
            offsets.push(i32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) - record.key_len as i32);
        }
        // The first of them is the length of the table itself rather than an
        // offset, and the last entry runs to the border.
        if !offsets.is_empty() {
            offsets.remove(0);
        }
        if let Some(last) = offsets.last_mut() {
            *last = border as i32;
        }
        data.offsets = offsets;
    }
    data.values = match reading {
        Reading::Not(why) => Values::None(why.clone()),
        Reading::Fixed { width, per_entry, floating, unsigned } => {
            let want = (*width as usize) * (*per_entry as usize) * head.entries.max(0) as usize;
            if want > border {
                Values::None(format!("basket holds {border} bytes of values, leaf expects {want}"))
            } else {
                read_values(&record.body[..want], *width as usize, *floating, *unsigned)
            }
        }
    };
    Ok(data)
}

fn read_values(data: &[u8], width: usize, floating: bool, unsigned: bool) -> Values {
    if floating {
        let mut out = Vec::with_capacity(data.len() / width.max(1));
        for c in data.chunks_exact(width) {
            out.push(match width {
                4 => f64::from(f32::from_be_bytes([c[0], c[1], c[2], c[3]])),
                8 => f64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]),
                _ => return Values::None(format!("{width}-byte floats, not read here")),
            });
        }
        return Values::Floats(out);
    }
    let mut out = Vec::with_capacity(data.len() / width.max(1));
    for c in data.chunks_exact(width) {
        out.push(match (width, unsigned) {
            (1, false) => i64::from(c[0] as i8),
            (1, true) => i64::from(c[0]),
            (2, false) => i64::from(i16::from_be_bytes([c[0], c[1]])),
            (2, true) => i64::from(u16::from_be_bytes([c[0], c[1]])),
            (4, false) => i64::from(i32::from_be_bytes([c[0], c[1], c[2], c[3]])),
            (4, true) => i64::from(u32::from_be_bytes([c[0], c[1], c[2], c[3]])),
            (8, _) => i64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]),
            _ => return Values::None(format!("{width}-byte integers, not read here")),
        });
    }
    Values::Ints(out)
}

/// The five numbers a basket's key carries past the three strings every key
/// ends with.
#[derive(Debug, Clone, Copy)]
struct BasketHead {
    entries: i64,
    /// Where the values stop, counted from the start of the key.
    last: i64,
}

/// One record read straight out of the file: the key's class and length, and
/// the contents unpacked.
struct Record {
    class: String,
    key_len: usize,
    body: Vec<u8>,
    /// Set where the key went on past `fTitle` the way a basket's does.
    basket: Option<BasketHead>,
}

/// Read the record whose key is at `at`, unpacking its contents.
///
/// This reads bytes rather than fields, the way
/// [`hdf5_tree`](super::hdf5_tree) does, and for the same two reasons: a
/// basket is at an offset no field holds, and two readings of the same widths
/// that agree on every real file are worth more than one. Every number is
/// bounded before it is used, because nobody vouched for this file.
fn read_record<S: Source>(doc: &Document<S>, at: u64) -> R<Record> {
    // Enough for the fixed part of a key and the three strings, at their
    // longest in any file anyone writes.
    let head = read_at(doc, at, 512.min(doc.len_bytes().saturating_sub(at)) as usize)?;
    // These reach the panel after `StreamerInfo record could not be read: `
    // and at the front of a tree's own row, so each names the key by the
    // address the rest of the app writes and then says what is wrong with it.
    if head.len() < 42 {
        return Err(EvalError::Failed(format!("key at @0x{at:x}: fewer than 42 bytes left in the file")));
    }
    let nbytes = i32::from_be_bytes([head[0], head[1], head[2], head[3]]) as i64;
    let version = i16::from_be_bytes([head[4], head[5]]);
    let objlen = i32::from_be_bytes([head[6], head[7], head[8], head[9]]) as i64;
    let key_len = i16::from_be_bytes([head[14], head[15]]) as i64;
    let wide = version > 1000;
    let mut p = 18 + if wide { 16 } else { 8 };
    if key_len < p as i64 || key_len > 512 || nbytes < key_len || nbytes > MAX_RECORD as i64 || objlen < 0 {
        return Err(EvalError::Failed(format!("key at @0x{at:x}: lengths out of range")));
    }
    let mut strings = Vec::new();
    for _ in 0..3 {
        if p >= head.len() {
            return Err(EvalError::Failed(format!("key at @0x{at:x}: name strings cut short")));
        }
        let n = head[p] as usize;
        p += 1;
        if p + n > head.len() {
            return Err(EvalError::Failed(format!("key at @0x{at:x}: name strings cut short")));
        }
        strings.push(String::from_utf8_lossy(&head[p..p + n]).into_owned());
        p += n;
    }
    // A basket's key carries five more numbers and a byte after the strings,
    // all of them inside `fKeylen`.
    let basket = match strings[0] == "TBasket" && p + 19 <= head.len() {
        false => None,
        true => {
            let at_last = p + 2 + 4 + 4 + 4;
            Some(BasketHead {
                entries: i64::from(i32::from_be_bytes([
                    head[p + 10],
                    head[p + 11],
                    head[p + 12],
                    head[p + 13],
                ])),
                last: i64::from(i32::from_be_bytes([
                    head[at_last],
                    head[at_last + 1],
                    head[at_last + 2],
                    head[at_last + 3],
                ])),
            })
        }
    };
    let packed = (nbytes - key_len) as usize;
    if objlen as usize > MAX_RECORD {
        return Err(EvalError::Failed(format!(
            "key at @0x{at:x}: {objlen} bytes unpacked, over the {} MiB limit",
            MAX_RECORD >> 20
        )));
    }
    let raw = read_at(doc, at + key_len as u64, packed)?;
    let body = match packed == objlen as usize {
        true => raw,
        false => unpack(&raw, objlen as usize).map_err(EvalError::Failed)?,
    };
    Ok(Record { class: strings.remove(0), key_len: key_len as usize, body, basket })
}

fn read_at<S: Source>(doc: &Document<S>, at: u64, len: usize) -> R<Vec<u8>> {
    if at.saturating_add(len as u64) > doc.len_bytes() {
        return Err(EvalError::Failed(format!("{len} bytes at @0x{at:x} run past the end of the file")));
    }
    let mut out = vec![0u8; len];
    let missing = doc.read_bytes(at, &mut out);
    if !missing.is_empty() {
        return Err(EvalError::Pending(missing));
    }
    Ok(out)
}

/// Undo the compression a record's contents were written with.
///
/// ROOT packs in blocks of at most sixteen mebibytes unpacked, so a large
/// record is several of these one after another. Each opens with two letters
/// naming the algorithm, the version of it, and the two sizes three bytes each
/// and little-endian in a format that is big-endian everywhere else.
fn unpack(data: &[u8], want: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(want.min(1 << 20));
    let mut at = 0usize;
    while at + 9 <= data.len() && out.len() < want {
        let head = &data[at..at + 9];
        let packed = head[3] as usize | (head[4] as usize) << 8 | (head[5] as usize) << 16;
        let block = &data[at + 9..];
        if packed > block.len() {
            return Err(format!("a compressed block claims {packed} bytes with {} left", block.len()));
        }
        let block = &block[..packed];
        let (codec, block) = match &head[0..2] {
            b"ZL" => (Codec::Zlib, block),
            b"XZ" => (Codec::Xz, block),
            b"ZS" => (Codec::Zstd, block),
            // ROOT writes the xxhash-64 of the packed bytes and then a bare
            // LZ4 block, with no frame header in front of it.
            b"L4" if block.len() >= 8 => (Codec::Lz4Block, &block[8..]),
            // The zlib of ROOT 3: raw deflate, with no two-byte header on it.
            b"CS" => (Codec::Deflate, block),
            other => {
                return Err(format!("a block compressed with {}, not read here", String::from_utf8_lossy(other)))
            }
        };
        let piece = codec::decode(codec, block).map_err(|why| format!("a block failed to unpack: {}", why.as_str()))?;
        out.extend_from_slice(&piece);
        at += 9 + packed;
    }
    if out.len() != want {
        return Err(format!("blocks unpacked to {} bytes, key says {want}", out.len()));
    }
    Ok(out)
}

/// What a field that reads its contents somewhere else points at, where it
/// points at anything.
fn first_child<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<Option<Vec<usize>>> {
    if ev.node(doc, path)?.child_count == 0 {
        return Ok(None);
    }
    let mut p = path.to_vec();
    p.push(0);
    Ok(Some(p))
}

fn field_int<S: Source>(ev: &mut Evaluator, doc: &Document<S>, at: &[usize], name: &str) -> R<Option<i64>> {
    let Some(p) = ev.child_named(doc, at, name)? else { return Ok(None) };
    Ok(ev.node(doc, &p)?.value.as_int().map(|v| v as i64))
}

/// The text of a `TString` field, which is a length and then the bytes.
fn field_text<S: Source>(ev: &mut Evaluator, doc: &Document<S>, at: &[usize], name: &str) -> R<String> {
    let Some(p) = ev.child_named(doc, at, name)? else { return Ok(String::new()) };
    let Some(text) = ev.child_named(doc, &p, "text")? else { return Ok(String::new()) };
    Ok(match ev.node(doc, &text)?.value {
        Value::Str(s) => s,
        _ => String::new(),
    })
}

/// Read a basket out of a branch by its place in the branch's list.
pub fn branch_basket<S: Source>(doc: &Document<S>, branch: &Branch, index: usize) -> R<BasketData> {
    let Some(basket) = branch.baskets.get(index) else {
        return Err(EvalError::Failed(format!("branch {} has no basket {index}", branch.name)));
    };
    read_basket(doc, basket.at, &branch.reading)
}
