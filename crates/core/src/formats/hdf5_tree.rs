//! An HDF5 version 1 B-tree read as a tree, rather than as the run of fields
//! the template places it as.
//!
//! The template already describes every node of one of these exactly: a `TREE`
//! node's entries each name a child address, and the child at that address is
//! another `TREE` or, for a group, an `SNOD` holding the links themselves. So
//! the shape is all there, spread through the template as a chain of pointer
//! hops. What is not there is the shape *as a shape*: how many nodes, how many
//! rows, how full each one is, and where in the file each one landed. A reader
//! who wants those has to follow a hop per node through the listing and hold
//! the answer in their head.
//!
//! This walks it once and hands back the nodes flat, each with its parent, so a
//! view can draw the tree without asking a question per node across the wasm
//! boundary. Modelled on [`super::h5ad::contents`], which walks the same tree
//! for a different reason: it wants the links at the bottom, and this wants
//! everything above them.
//!
//! Two facts about HDF5 shape everything here:
//!
//! - A group tree and a chunk tree do not have the same silhouette. Both are
//!   made of `TREE` nodes, but a group tree has one more real, addressable node
//!   below its bottom `TREE` row: the `SNOD` that holds the links. A chunk
//!   tree's bottom row points straight at the chunks, which are payload rather
//!   than nodes of the index. Drawing the two the same way would draw a row
//!   that is not there.
//! - A node's own key range is only unambiguous at the bottom. The first key of
//!   a group index node is a heap offset the format is free to leave empty, so
//!   reading it would print a blank where a name belongs. Every range here is
//!   therefore read at the bottom, from a link table's own names or from a
//!   level-zero chunk node's own coordinates, and every node above takes the
//!   span of the children that were actually walked. A node whose children were
//!   not all walked says no range rather than a range that is short at one end.

use crate::document::Document;
use crate::eval::{EvalError, Evaluator, R, Value};
use crate::source::Source;

/// What a root node's `parent` says: there is nothing above it.
pub const NO_PARENT: usize = usize::MAX;

/// How deep the walk will follow children. Far past anything HDF5 writes: a
/// four-level v1 tree already indexes more links than a file is likely to hold,
/// and this is the stop that a file whose child pointers form a ring hits.
const MAX_DEPTH: usize = 24;

/// How far the search for a tree will climb looking for an object header. An
/// object header is a handful of levels above anything inside it; this is the
/// stop for a path that is not under one at all.
const MAX_CLIMB: usize = 64;

/// Which of the two jobs a version 1 B-tree is doing. The `node_type` byte in
/// every node of it says which, and it is the same for the whole tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// Indexes the links of a group. Its bottom row of index nodes points at
    /// symbol table nodes, which hold the links.
    Group,
    /// Indexes the chunks of a dataset. Its bottom row points at the chunks.
    Chunk,
}

/// What one drawn box is. Two kinds, not one, because a group tree has both and
/// they hold different things: an index node holds pointers, a link table holds
/// names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A `TREE` node, at any level, root or not.
    Index,
    /// An `SNOD`. Only a group tree has these, and only under its level-zero
    /// index nodes.
    LinkTable,
}

/// One node of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Where it is in the template, so that picking it can move the cursor.
    pub path: Vec<usize>,
    /// Index into [`Tree::nodes`], or [`NO_PARENT`] for the root. Every node
    /// but the root comes after its parent in the list.
    pub parent: usize,
    pub kind: Kind,
    /// Where the node starts in the file, in bytes. This is the address the
    /// entry above it pointed at.
    pub address: u64,
    /// How many bits of the file the node occupies. An index node is read to
    /// the length of the entries it actually uses, not to the length HDF5
    /// reserved for it, so this is what is written rather than what is spare.
    pub size_bits: u64,
    /// The `node_level` the file wrote, for an index node. Zero for a link
    /// table, which has no level of its own: it hangs below the bottom row.
    pub level: u64,
    /// How many rows below the root of this tree the node sits. Counted by the
    /// walk rather than read, so a link table's depth is one past its index
    /// node's, which `level` alone cannot say.
    pub depth: usize,
    /// The count the node wrote about itself: `entries_used` for an index node,
    /// `symbol_count` for a link table. A raw count and not a fraction: HDF5
    /// bounds these with the superblock's K values, and the exact bound per
    /// node type is not settled here, so a denominator would be a number with
    /// no stated origin.
    pub entries: u64,
    /// The bottom of the node's key range, written out, and the top. Both empty
    /// where the walk could not settle them: see the module doc.
    pub first_key: String,
    pub last_key: String,
    /// True when children of this node exist that the walk did not reach, so
    /// its count is what is under it and its range is not.
    pub truncated: bool,
}

/// One version 1 B-tree, walked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    pub job: Job,
    /// Root first, then every node reached, each after its parent.
    pub nodes: Vec<Node>,
    /// Children that exist and were not walked, as far as the walk knows.
    pub omitted: u64,
    /// How many numbers one chunk key holds, for a chunk tree. HDF5 writes one
    /// per dataset dimension plus one more that is the element size, so this is
    /// one greater than the dataset's rank. Zero for a group tree.
    pub coords: u64,
}

/// The version 1 B-tree the field at `path` belongs to, walked, or `None` where
/// there is no such tree to be had.
///
/// `path` is where the reader's cursor is, which is usually not a node of a
/// tree. Three questions are asked of it in turn, and the first that answers
/// wins:
///
/// 1. Is the cursor inside a tree already? Then climb to that tree's root. The
///    climb is not "the topmost `TREE` above this one": a group's tree sits,
///    in the template, underneath the tree of the group above it, so climbing
///    all the way would answer with the root group's tree wherever the reader
///    stood. It climbs only while each step up is an entry's child of the node
///    above, which is what being in the same tree means.
/// 2. Is the cursor inside an object header? Then the tree that header names:
///    its symbol table message's for a group, its data layout message's for a
///    chunked dataset. This is what makes picking a dataset in the Logical tab
///    draw that dataset's chunk tree, since an object header is above its tree
///    rather than inside it.
/// 3. Neither: the root group's tree, which every file written with a version 0
///    or 1 superblock has.
///
/// `limit` caps the nodes walked. A file whose group holds a million links has
/// a quarter of a million link tables, and a picture of a quarter of a million
/// boxes is not a picture; the cap says how many were left out instead.
pub fn tree<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], limit: usize) -> R<Option<Tree>> {
    let Some(root) = find_root(ev, doc, path)? else { return Ok(None) };
    walk(ev, doc, &root, limit).map(Some)
}

/// Which tree to answer with. See [`tree`] for the three questions.
///
/// The first two are asked innermost first rather than in order, because both
/// can be true at once and the wrong one of the two is badly wrong. Every
/// object header in a classic HDF5 file is reached through a link, and that
/// link lives in a link table of the *parent* group's tree. So a reader who
/// picks a dataset in the Logical tab has a cursor that is, truthfully, inside
/// the parent group's tree, and answering with that would draw the same tree
/// wherever they went. The object header is deeper than the link table that
/// points at it, and a tree of that object's own is deeper still, so taking
/// whichever of the two is innermost answers the question the reader asked.
fn find_root<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<Option<Vec<usize>>> {
    let node = enclosing_node(ev, doc, path)?;
    let header = enclosing_header(ev, doc, path)?;
    let node_depth = node.as_ref().map(Vec::len);
    let header_depth = header.as_ref().map(Vec::len);
    if let Some(inside) = node.clone() {
        if header_depth.is_none_or(|h| h < inside.len()) {
            return Ok(Some(climb(ev, doc, inside)?));
        }
    }
    if let Some(header) = header {
        if let Some(found) = tree_of_header(ev, doc, &header)? {
            return Ok(Some(found));
        }
        // An object with no tree of its own: a group small enough to keep its
        // links as messages, or a dataset kept contiguously. The tree the
        // cursor is in is still a tree, and is a better answer than the root
        // group's, which is further from where the reader is standing.
        if let (Some(inside), Some(_)) = (node, node_depth) {
            return Ok(Some(climb(ev, doc, inside)?));
        }
    }
    let Some(root) = super::h5ad::root_header(ev, doc)? else { return Ok(None) };
    tree_of_header(ev, doc, &root)
}

/// The innermost node of a tree at or above `path`, where there is one.
fn enclosing_node<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<Option<Vec<usize>>> {
    let mut at = path.to_vec();
    for _ in 0..MAX_CLIMB {
        if kind_of(ev, doc, &at)?.is_some() {
            return Ok(Some(at));
        }
        if at.is_empty() {
            return Ok(None);
        }
        at.pop();
    }
    Ok(None)
}

/// The innermost object header at or above `path`, where there is one.
fn enclosing_header<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<Option<Vec<usize>>> {
    let mut at = path.to_vec();
    for _ in 0..MAX_CLIMB {
        // An object header is the one structure here holding both a count of
        // messages and the messages themselves. Asked by name rather than by
        // the template's own type name, so that renaming a structure does not
        // quietly stop this finding one.
        if has_field(ev, doc, &at, "message_count")? && has_field(ev, doc, &at, "messages")? {
            return Ok(Some(at));
        }
        if at.is_empty() {
            return Ok(None);
        }
        at.pop();
    }
    Ok(None)
}

/// Up from a node to the root of the tree it is in.
fn climb<S: Source>(ev: &mut Evaluator, doc: &Document<S>, from: Vec<usize>) -> R<Vec<usize>> {
    let mut at = from;
    for _ in 0..MAX_DEPTH {
        match tree_parent(ev, doc, &at)? {
            Some(up) => at = up,
            None => break,
        }
    }
    Ok(at)
}

/// The index node whose entry points at the node at `at`, where the step up is
/// that and nothing else.
///
/// The template places a child four levels under its parent node: the parent's
/// `entries` array, the entry, the entry's `child` (a field of no bytes that
/// says where its one value is), and that value. Anything that does not have
/// exactly that shape above it is the top of its tree.
fn tree_parent<S: Source>(ev: &mut Evaluator, doc: &Document<S>, at: &[usize]) -> R<Option<Vec<usize>>> {
    if at.len() < 4 {
        return Ok(None);
    }
    let entry = &at[..at.len() - 2];
    let Some(child) = ev.child_named(doc, entry, "child")? else { return Ok(None) };
    if child != at[..at.len() - 1] {
        return Ok(None);
    }
    let up = at[..at.len() - 4].to_vec();
    if kind_of(ev, doc, &up)? == Some(Kind::Index) { Ok(Some(up)) } else { Ok(None) }
}

/// The tree an object header names: a group's links, or a chunked dataset's
/// chunks. `None` for an object that has neither, which is every dataset kept
/// contiguously and every group small enough to keep its links as messages.
fn tree_of_header<S: Source>(ev: &mut Evaluator, doc: &Document<S>, header: &[usize]) -> R<Option<Vec<usize>>> {
    for message in super::h5ad::collect_messages(ev, doc, header, 0)? {
        let Some(kind) = ev.child_named(doc, &message, "type")? else { continue };
        let kind = ev.node(doc, &kind)?.value.as_int().unwrap_or(-1);
        let Some(body) = ev.child_named(doc, &message, "body")? else { continue };
        // A symbol table message names the heap the tree is placed inside, and
        // the tree under it.
        if kind == 0x11 {
            let Some(heap) = ev.child_named(doc, &body, "heap")? else { continue };
            let Some(heap) = super::h5ad::inside(ev, doc, &heap)? else { continue };
            let Some(found) = ev.child_named(doc, &heap, "tree")? else { continue };
            let Some(found) = super::h5ad::inside(ev, doc, &found)? else { continue };
            if kind_of(ev, doc, &found)?.is_some() {
                return Ok(Some(found));
            }
        }
        // A data layout message wraps the layout in a version, and a chunked
        // layout names its tree `chunks`.
        if kind == 0x08 {
            let Some(layout) = ev.child_named(doc, &body, "body")? else { continue };
            let Some(storage) = ev.child_named(doc, &layout, "storage")? else { continue };
            let Some(found) = ev.child_named(doc, &storage, "chunks")? else { continue };
            let Some(found) = super::h5ad::inside(ev, doc, &found)? else { continue };
            if kind_of(ev, doc, &found)?.is_some() {
                return Ok(Some(found));
            }
            continue;
        }
    }
    Ok(None)
}

/// Which kind of node sits at `path`, or `None` for anything that is not one.
///
/// Asked by the fields a node has rather than by its signature, which would be
/// a read of the file per question, and this is asked of every ancestor of the
/// cursor on every move.
fn kind_of<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<Option<Kind>> {
    if has_field(ev, doc, path, "node_level")? && has_field(ev, doc, path, "entries_used")? {
        return Ok(Some(Kind::Index));
    }
    if has_field(ev, doc, path, "symbol_count")? && has_field(ev, doc, path, "symbols")? {
        return Ok(Some(Kind::LinkTable));
    }
    Ok(None)
}

/// Whether the structure at `path` has a field of this name. A path that will
/// not read has no fields, which is the answer rather than an error: the
/// question is asked while climbing past whatever the cursor happens to be in.
fn has_field<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], name: &str) -> R<bool> {
    match ev.child_named(doc, path, name) {
        Ok(found) => Ok(found.is_some()),
        Err(e) if e.interrupted() => Err(e),
        Err(_) => Ok(false),
    }
}

/// Every node of the tree rooted at `root`, breadth first.
///
/// Breadth first for the reason `graph.rs` gives: a cap has to fall somewhere,
/// and cutting a depth-first walk keeps one spine and no siblings, which is the
/// opposite of what a reader asking about the shape wants. Cut across, what is
/// kept is the rows nearest the root, which is the part of a B-tree that says
/// how it branches.
fn walk<S: Source>(ev: &mut Evaluator, doc: &Document<S>, root: &[usize], limit: usize) -> R<Tree> {
    let mut out = Tree { job: Job::Group, nodes: Vec::new(), omitted: 0, coords: 0 };
    let mut queue: std::collections::VecDeque<(Vec<usize>, usize, usize)> = std::collections::VecDeque::new();
    queue.push_back((root.to_vec(), NO_PARENT, 0));
    while let Some((at, parent, depth)) = queue.pop_front() {
        if out.nodes.len() >= limit.max(1) {
            out.omitted += queue.len() as u64 + 1;
            if let Some(node) = out.nodes.get_mut(parent) {
                node.truncated = true;
            }
            break;
        }
        match add(ev, doc, &at, parent, depth, &mut out, &mut queue, limit) {
            Ok(()) => {}
            Err(e) if e.interrupted() => return Err(e),
            // A node the template cannot read here. Its siblings still read,
            // and a tree missing one box says more than an error in place of
            // the whole picture. The root is the exception: with that
            // unreadable there is no tree to answer with.
            Err(e) => {
                if parent == NO_PARENT {
                    return Err(e);
                }
                if let Some(node) = out.nodes.get_mut(parent) {
                    node.truncated = true;
                }
            }
        }
    }
    ranges(&mut out);
    Ok(out)
}

/// One node: what it says about itself, and its children queued behind it.
fn add<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    at: &[usize],
    parent: usize,
    depth: usize,
    out: &mut Tree,
    queue: &mut std::collections::VecDeque<(Vec<usize>, usize, usize)>,
    limit: usize,
) -> R<()> {
    let Some(kind) = kind_of(ev, doc, at)? else {
        return Err(EvalError::Failed("not a b-tree node".into()));
    };
    // Reading the node charges the walk's allowance the way every other reader
    // of this file's template does, so a tree of a hundred thousand nodes hands
    // the reader their screen back rather than holding the thread until it has
    // finished. See `Evaluator::begin_slice`.
    let info = ev.node(doc, at)?;
    let here = out.nodes.len();
    out.nodes.push(Node {
        path: at.to_vec(),
        parent,
        kind,
        address: info.offset_bits / 8,
        size_bits: info.size_bits,
        level: 0,
        depth,
        entries: 0,
        first_key: String::new(),
        last_key: String::new(),
        truncated: false,
    });
    if kind == Kind::LinkTable {
        out.nodes[here].entries = field_int(ev, doc, at, "symbol_count")?.unwrap_or(0);
        let (first, last) = link_names(ev, doc, at)?;
        out.nodes[here].first_key = first;
        out.nodes[here].last_key = last;
        return Ok(());
    }
    let level = field_int(ev, doc, at, "node_level")?.unwrap_or(0);
    let used = field_int(ev, doc, at, "entries_used")?.unwrap_or(0);
    out.nodes[here].level = level;
    out.nodes[here].entries = used;
    if parent == NO_PARENT {
        // Every node of one tree carries the same `node_type`; the root's is
        // read and the rest are taken on its word, which saves a read a node.
        out.job = match field_int(ev, doc, at, "node_type")? {
            Some(1) => Job::Chunk,
            _ => Job::Group,
        };
    }
    let Some(entries) = ev.child_named(doc, at, "entries")? else { return Ok(()) };
    // The bottom row of a chunk tree points at the chunks themselves, which are
    // the dataset's payload rather than nodes of its index. Their coordinates
    // are the node's own key range and are read here; nothing is drawn for
    // them, because a box per chunk is the Listing's job and not a picture's.
    if out.job == Job::Chunk && level == 0 {
        let (first, last, coords) = chunk_keys(ev, doc, &entries, used)?;
        out.nodes[here].first_key = first;
        out.nodes[here].last_key = last;
        if coords > 0 {
            out.coords = coords;
        }
        return Ok(());
    }
    if depth >= MAX_DEPTH {
        out.nodes[here].truncated = true;
        out.omitted += used;
        return Ok(());
    }
    // Room for what is already in and what is already waiting: a child queued
    // now is a node later, and counting only the nodes would queue far more
    // than the cap allows and then drop most of them.
    let room = limit.saturating_sub(out.nodes.len() + queue.len());
    let taken = room.min(usize::try_from(used).unwrap_or(usize::MAX));
    for i in 0..taken {
        let mut entry = entries.clone();
        entry.push(i);
        let Some(child) = ev.child_named(doc, &entry, "child")? else { continue };
        let Some(child) = super::h5ad::inside(ev, doc, &child)? else { continue };
        queue.push_back((child, here, depth + 1));
    }
    if (taken as u64) < used {
        out.nodes[here].truncated = true;
        out.omitted += used - taken as u64;
    }
    Ok(())
}

/// A named field of a structure, read as a number.
fn field_int<S: Source>(ev: &mut Evaluator, doc: &Document<S>, at: &[usize], name: &str) -> R<Option<u64>> {
    let Some(field) = ev.child_named(doc, at, name)? else { return Ok(None) };
    Ok(ev.node(doc, &field)?.value.as_int().and_then(|v| u64::try_from(v).ok()))
}

/// The first and last link name in a link table, which is where a group tree's
/// key ranges are read from.
///
/// Two reads a table rather than one a link: the links are in name order, so
/// the two ends are the range and the 4,094 names between them are the
/// Listing's business.
fn link_names<S: Source>(ev: &mut Evaluator, doc: &Document<S>, at: &[usize]) -> R<(String, String)> {
    let Some(symbols) = ev.child_named(doc, at, "symbols")? else { return Ok((String::new(), String::new())) };
    let n = ev.node(doc, &symbols)?.child_count;
    if n == 0 {
        return Ok((String::new(), String::new()));
    }
    let first = link_name(ev, doc, &symbols, 0)?;
    let last = if n == 1 { first.clone() } else { link_name(ev, doc, &symbols, n - 1)? };
    Ok((first, last))
}

fn link_name<S: Source>(ev: &mut Evaluator, doc: &Document<S>, symbols: &[usize], i: u64) -> R<String> {
    let mut link = symbols.to_vec();
    link.push(i as usize);
    let Some(name) = ev.child_named(doc, &link, "name")? else { return Ok(String::new()) };
    let Some(name) = super::h5ad::inside(ev, doc, &name)? else { return Ok(String::new()) };
    match ev.node(doc, &name)?.value {
        Value::Str(text) => Ok(text),
        _ => Ok(String::new()),
    }
}

/// The coordinates of the first and last chunk a level-zero chunk node indexes,
/// and how many numbers one of those coordinates holds.
///
/// The numbers are offsets into the dataset counted in elements, not chunk
/// numbers, and the last of them is not a dimension of the dataset at all: HDF5
/// writes one extra that is an offset within an element and is always zero.
/// Both facts have to reach whatever writes the label, or a reader will take
/// `[0, 0, 0]` for a three-dimensional dataset's first chunk.
fn chunk_keys<S: Source>(ev: &mut Evaluator, doc: &Document<S>, entries: &[usize], used: u64) -> R<(String, String, u64)> {
    if used == 0 {
        return Ok((String::new(), String::new(), 0));
    }
    let (first, coords) = chunk_key(ev, doc, entries, 0)?;
    let (last, _) = if used == 1 { (first.clone(), coords) } else { chunk_key(ev, doc, entries, used - 1)? };
    Ok((first, last, coords))
}

fn chunk_key<S: Source>(ev: &mut Evaluator, doc: &Document<S>, entries: &[usize], i: u64) -> R<(String, u64)> {
    let mut entry = entries.to_vec();
    entry.push(i as usize);
    let Some(offsets) = ev.child_named(doc, &entry, "offsets")? else { return Ok((String::new(), 0)) };
    let n = ev.node(doc, &offsets)?.child_count;
    let mut parts: Vec<String> = Vec::new();
    for k in 0..n.min(16) {
        let mut one = offsets.clone();
        one.push(k as usize);
        parts.push(ev.node(doc, &one)?.value.as_int().unwrap_or(0).to_string());
    }
    Ok((parts.join(", "), n))
}

/// Every node above the bottom given the span of the children that were walked.
///
/// Bottom up, over a list the walk left in breadth-first order, so one pass
/// backwards has every child settled before its parent is asked about. A parent
/// whose children were not all walked keeps no range at all: half a range read
/// as a whole one is the kind of number this app exists not to print.
fn ranges(out: &mut Tree) {
    for i in (0..out.nodes.len()).rev() {
        let parent = out.nodes[i].parent;
        if parent == NO_PARENT {
            continue;
        }
        let (first, last, short) =
            (out.nodes[i].first_key.clone(), out.nodes[i].last_key.clone(), out.nodes[i].truncated);
        let up = &mut out.nodes[parent];
        if short || first.is_empty() {
            up.truncated = true;
            continue;
        }
        // The children arrive in reverse here, so the last one seen is the
        // node's first key and the first one seen is its last.
        if up.last_key.is_empty() {
            up.last_key = last;
        }
        up.first_key = first;
    }
    // A node the pass above marked short may already have taken half a range
    // from the children that were walked, and half a range read as a whole one
    // is worse than none. Only a node that read its own range keeps it: a link
    // table's names are its own, and so are a level-zero chunk node's
    // coordinates, and neither depends on a child having been reached.
    let job = out.job;
    for node in &mut out.nodes {
        let own = node.kind == Kind::LinkTable || (job == Job::Chunk && node.level == 0);
        if !own && node.truncated {
            node.first_key = String::new();
            node.last_key = String::new();
        }
    }
}
