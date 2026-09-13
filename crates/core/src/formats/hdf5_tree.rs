//! An HDF5 B-tree read as a tree, rather than as the run of fields the
//! template places it as. Both versions of one: the version 1 trees a classic
//! file is made of, and the version 2 trees a file written with a recent
//! library uses instead.
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
//!
//! # The version 2 trees
//!
//! A version 2 tree is a different structure doing the same two jobs, and one
//! more besides: it indexes a group's links, a dataset's chunks, a file's
//! shared messages, or an object's attributes, and a type byte in its header
//! says which. See the `v2` module for how it is walked and why that walk reads bytes
//! rather than fields. Three things about it decide what the drawing can say,
//! and all three are settled in this module rather than left to the view:
//!
//! - **Where the drawing stops.** A `BTHD` header is not a row of the tree: it
//!   is read to learn how to walk, not walked through, and a box for it would
//!   be a box with no siblings and one child forever. The rows are the `BTIN`
//!   and `BTLF` nodes, and they stop at the leaves. What a leaf holds is
//!   records, and a record names something that lives elsewhere: a link in a
//!   fractal heap, or a chunk of a dataset. Neither is a node of the tree, so
//!   neither is drawn — which is exactly the rule a version 1 chunk tree is
//!   already drawn by. So a version 2 tree of either job has the version 1
//!   chunk tree's silhouette, and the version 1 group tree's extra bottom row
//!   of `SNOD`s stays the one shape that has it.
//! - **What a record means.** The type byte says, and this reads two of the
//!   twelve: link names and unfiltered chunks. The shape of a tree does not
//!   depend on the type — only `record_size` does, and the header states that
//!   — so an unread type is still drawn, and [`Tree::records`] says outright
//!   that its records were not read. A tree drawn as though its records were
//!   understood when they were not is the failure this is here to avoid.
//! - **Keys.** A version 2 group's record holds the hash of a link's name and
//!   an id into the fractal heap, and no name at all. The name is in the heap,
//!   behind an id this walk does not resolve, so there is no range to show and
//!   none is shown. A hash printed where a name belongs would be a label whose
//!   value is not the thing it names. An unfiltered chunk record does hold the
//!   chunk's offset, so a chunk tree's ranges are read at the leaves and
//!   carried up by the same [`ranges`] pass the version 1 walk uses.

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

/// How many object headers [`first_tree_under`] will look at before giving up.
/// It is looking for the first group in the file that keeps its links
/// somewhere other than its own header, and a file that has one has it within
/// a few links of the root; this is the stop for a file that has none.
const MAX_OBJECTS: usize = 64;

/// Which job a B-tree is doing. For a version 1 tree the `node_type` byte in
/// every node says which, and it is the same for the whole tree; for a version
/// 2 tree the header's record type says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// Indexes the links of a group. A version 1 group tree's bottom row of
    /// index nodes points at symbol table nodes, which hold the links; a
    /// version 2 one's records point into a fractal heap, which holds them.
    Group,
    /// Indexes the chunks of a dataset. Its bottom row points at the chunks.
    Chunk,
    /// A version 2 tree doing neither: indexing a file's shared object header
    /// messages, an object's attributes, or the objects too big for the heap.
    /// The shape is drawn and [`Tree::record_type_name`] says what it indexes.
    Other,
}

/// What one drawn box is. Three kinds, not one, because the file writes three
/// different structures and they hold different things: an index node holds
/// pointers, a link table holds names, a version 2 leaf holds records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A `TREE` node at any level, or a version 2 `BTIN`. Root or not.
    Index,
    /// An `SNOD`. Only a version 1 group tree has these, and only under its
    /// level-zero index nodes.
    LinkTable,
    /// A version 2 `BTLF`: the bottom row of one of those, holding records and
    /// pointing at no node at all.
    Leaf,
}

/// How far this walk got with a version 2 tree's records. The shape of a tree
/// is drawn whichever of the three it is, because the shape depends only on
/// `record_size`, which the header states; what changes is whether anything
/// may be said about what is in the nodes.
///
/// Three states and not two, because "not read" has two quite different
/// causes and a reader who wants to know why is owed the difference: a type
/// the specification names and this does not read is a gap here, and a type
/// the specification does not name is a file this does not recognise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Records {
    /// What a record of this type holds is known, and whatever of it can be
    /// shown without reading another structure is shown. Every version 1 tree,
    /// and the two version 2 types read here.
    Read,
    /// The specification names this record type and this walk does not read
    /// it. The boxes are the file's own nodes; nothing is claimed about what
    /// is inside them.
    Unread,
    /// A record type no version of the specification names. Either a file
    /// written by something this does not know about, or bytes that are not a
    /// B-tree header at all.
    Unknown,
}

/// One node of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Where it is in the template, so that picking it can move the cursor.
    ///
    /// Empty for a version 2 node the template does not place at the address
    /// this walk read it from, which in a well-formed tree is none of them: see
    /// the `v2` module. A box with no path still goes to its bytes, because the
    /// address is its own fact; what it cannot do is open in the Listing, since
    /// there is no field there to open.
    pub path: Vec<usize>,
    /// Index into [`Tree::nodes`], or [`NO_PARENT`] for the root. Every node
    /// but the root comes after its parent in the list.
    pub parent: usize,
    pub kind: Kind,
    /// The four bytes written at [`Node::address`]: `TREE`, `SNOD`, `BTIN` or
    /// `BTLF`. Carried per node rather than worked out from [`Node::kind`] and
    /// the tree's version, so that what a reader is told to expect at an
    /// address is what this walk actually checked for there.
    pub sign: &'static str,
    /// Where the node starts in the file, in bytes. This is the address the
    /// entry above it pointed at.
    pub address: u64,
    /// How many bits of the file the node occupies. An index node is read to
    /// the length of the entries it actually uses, not to the length HDF5
    /// reserved for it, so this is what is written rather than what is spare.
    pub size_bits: u64,
    /// The `node_level` the file wrote, for a version 1 index node. Zero for a
    /// link table, which has no level of its own: it hangs below the bottom
    /// row.
    ///
    /// A version 2 node writes no level of its own, so this is the header's
    /// `depth` less the rows walked to reach it. Counting it that way and not
    /// some other way is what keeps level 0 meaning the bottom row of index
    /// nodes in both versions, which is the one thing a reader comparing two
    /// files needs it to mean.
    pub level: u64,
    /// How many rows below the root of this tree the node sits. Counted by the
    /// walk rather than read, so a link table's depth is one past its index
    /// node's, which `level` alone cannot say.
    pub depth: usize,
    /// The count the node wrote about itself: `entries_used` for a version 1
    /// index node, `symbol_count` for a link table, the record count for a
    /// version 2 node of either kind. A raw count and not a fraction: HDF5
    /// bounds these with the superblock's K values and, for version 2, with an
    /// arithmetic on the node size that this walk does compute but the view
    /// has no reason to be shown, so a denominator here would be a number with
    /// no stated origin.
    ///
    /// A version 2 internal node's record count is one less than the number of
    /// children it points at, because a version 2 tree is a B-tree and not a
    /// B+ tree: a record sits between every two children and is not repeated
    /// below. The count is the file's own number and the children are the
    /// boxes drawn under it, so both facts are there to be read.
    pub entries: u64,
    /// The bottom of the node's key range, written out, and the top. Both empty
    /// where the walk could not settle them: see the module doc.
    pub first_key: String,
    pub last_key: String,
    /// True when children of this node exist that the walk did not reach, so
    /// its count is what is under it and its range is not.
    pub truncated: bool,
    /// Where the node's first entry starts, counted in bits from
    /// [`Node::address`], and how many bits one entry takes. Every node here
    /// writes its entries at a fixed stride, so these two place all
    /// [`Node::entries`] of them without a list per node: entry `i` starts at
    /// `address * 8 + first_entry_bits + i * entry_bits`. Without them a view
    /// has a node's bytes and no way to divide them, which is a box a reader
    /// can open and not a box a reader can point at.
    ///
    /// What one entry covers is whatever the file writes per entry, and that
    /// is a different thing in each of the four kinds of node:
    ///
    /// - A `TREE` entry is a key and the child address under it: the pair the
    ///   template places as one element of the node's `entries`, so a view
    ///   dividing on this stride divides exactly where the Listing does. The
    ///   key that closes the node's range is not an entry. It sits alone at
    ///   `first_entry_bits + entries * entry_bits`, and so do the slots HDF5
    ///   reserved and has not filled yet, which hold whatever was written
    ///   there last; neither is placed by these two numbers.
    /// - An `SNOD` entry is one symbol table entry: a link's name offset, its
    ///   object header address, and the cache HDF5 keeps beside them. Not the
    ///   name, which is in the group's heap somewhere else entirely.
    /// - A `BTLF` or `BTIN` entry is one record, `record_size` bytes of it.
    ///   The child pointers a `BTIN` writes after its records are not on this
    ///   stride and are not entries: there is one more of them than there are
    ///   records, and how wide one is is nowhere in the file (see the `v2`
    ///   module's `Shape`). Nor is the four-byte checksum either kind ends
    ///   with.
    ///
    /// Both zero together where the walk could not settle the stride, which
    /// here is a version 1 node with no entries in it: an empty array is
    /// nothing to measure, and a node with no entries is nothing to divide. A
    /// view must divide nothing on a zero, rather than take it for a stride.
    pub first_entry_bits: u64,
    pub entry_bits: u64,
}

/// One B-tree, walked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    pub job: Job,
    /// 1 or 2. Which structure this is, not which HDF5 release wrote it: the
    /// two are different formats doing the same job and a reader who is shown
    /// only one of the two silhouettes should be told which one it is.
    pub version: u8,
    /// The record type byte a version 2 header writes, and the name the
    /// specification gives it. Zero and empty for a version 1 tree, which has
    /// no such byte: what its nodes hold is settled by [`Job`] instead.
    pub record_type: u8,
    pub record_type_name: &'static str,
    /// How far this walk got with the records. See [`Records`].
    pub records: Records,
    /// How many records the tree holds in all, which a version 2 header writes
    /// in a field of its own. Zero for a version 1 tree, which writes no such
    /// number anywhere.
    ///
    /// Not the sum of the walked nodes' counts, and the difference matters
    /// twice over. It is the file's own number, so it stands when the walk was
    /// capped. And a version 2 tree is a B-tree rather than a B+ tree: its
    /// internal nodes hold records of their own that are not repeated in the
    /// leaves, so the bottom row's counts add up to less than the whole and a
    /// view that summed them would print a total the file disagrees with.
    pub records_total: u64,
    /// Root first, then every node reached, each after its parent.
    pub nodes: Vec<Node>,
    /// Children that exist and were not walked, as far as the walk knows.
    pub omitted: u64,
    /// How many numbers one chunk key holds, for a chunk tree. Zero for a
    /// group tree, and zero where no key was read.
    ///
    /// The two versions write a different count of them and the difference is
    /// not cosmetic. A version 1 chunk key holds one number per dataset
    /// dimension and then one more, an offset within an element, which is
    /// always zero; so this is one greater than the rank. A version 2
    /// unfiltered chunk record holds the dimensions and nothing else, so this
    /// is the rank exactly. Whatever writes the label has to know which,
    /// because a reader counting the numbers in `950, 950, 0` and being told
    /// the wrong rule gets a dimension that does not exist.
    pub coords: u64,
    /// True where each key's last number is the always-zero offset within an
    /// element, which is what a version 1 chunk key ends with and a version 2
    /// record does not.
    pub coords_pad: bool,
}

/// The B-tree the field at `path` belongs to, walked, or `None` where there is
/// no such tree to be had.
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
///    above, which is what being in the same tree means. A version 2 tree needs
///    no climb of that sort: the template places every node of one inside the
///    `BTHD` header, under the root node, so the header found above the cursor
///    is the whole answer.
/// 2. Is the cursor inside an object header? Then the tree that header names:
///    its symbol table message's or its link info message's for a group, its
///    data layout message's for a chunked dataset. This is what makes picking a
///    dataset in the Logical tab draw that dataset's chunk tree, since an object
///    header is above its tree rather than inside it.
/// 3. Neither: the root group's tree, and then, where the root group has none,
///    the first tree under it. See [`first_tree_under`].
///
/// `limit` caps the nodes walked. A file whose group holds a million links has
/// a quarter of a million link tables, and a picture of a quarter of a million
/// boxes is not a picture; the cap says how many were left out instead.
pub fn tree<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize], limit: usize) -> R<Option<Tree>> {
    let Some(root) = find_root(ev, doc, path)? else { return Ok(None) };
    match spot_of(ev, doc, &root)? {
        Some(Spot::V2Header) => v2::walk(ev, doc, &root, limit).map(Some),
        _ => walk(ev, doc, &root, limit).map(Some),
    }
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
    let node_depth = node.as_ref().map(|(p, _)| p.len());
    let header_depth = header.as_ref().map(Vec::len);
    if let Some((inside, spot)) = node.clone() {
        if header_depth.is_none_or(|h| h < inside.len()) {
            return Ok(Some(rooted(ev, doc, inside, spot)?));
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
        if let (Some((inside, spot)), Some(_)) = (node, node_depth) {
            return Ok(Some(rooted(ev, doc, inside, spot)?));
        }
    }
    let Some(root) = super::h5ad::root_header(ev, doc)? else { return Ok(None) };
    if let Some(found) = tree_of_header(ev, doc, &root)? {
        return Ok(Some(found));
    }
    first_tree_under(ev, doc, &root)
}

/// The top of the tree the node at `from` is in.
///
/// A version 1 node climbs to its own root; a version 2 header is already the
/// top of one, since the template places every node of one inside its header.
fn rooted<S: Source>(ev: &mut Evaluator, doc: &Document<S>, from: Vec<usize>, spot: Spot) -> R<Vec<usize>> {
    match spot {
        Spot::V2Header => Ok(from),
        Spot::V1(_) => climb(ev, doc, from),
    }
}

/// What sits at `path` when the answer is not a drawn box: the header of a
/// version 2 tree, which is read to learn how to walk rather than walked
/// through. See the module doc for why it is not a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spot {
    V1(Kind),
    V2Header,
}

/// The innermost node of a tree, or version 2 tree header, at or above `path`.
fn enclosing_node<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    path: &[usize],
) -> R<Option<(Vec<usize>, Spot)>> {
    let mut at = path.to_vec();
    for _ in 0..MAX_CLIMB {
        if let Some(spot) = spot_of(ev, doc, &at)? {
            return Ok(Some((at, spot)));
        }
        if at.is_empty() {
            return Ok(None);
        }
        at.pop();
    }
    Ok(None)
}

/// The first tree under an object that has none of its own, breadth first
/// through the links it holds.
///
/// Only ever reached from the third of [`tree`]'s three questions, and only
/// for a file whose root group has no tree: which is every file a recent
/// library wrote whose root holds few enough links to keep them as messages.
/// Without this such a file answers "no tree" for a cursor that has not been
/// moved yet, while the group one link down has a version 2 tree of two
/// thousand records in it, and a reader is told the file has no B-tree when it
/// has one. A version 1 file never reaches this: its root group always has a
/// tree, and question three answers with it.
///
/// Bounded twice over, because this follows pointers a file it does not trust
/// wrote: [`MAX_OBJECTS`] object headers looked at in all, and each one only
/// while its address has not been seen before, so a file whose links form a
/// ring stops rather than spinning.
fn first_tree_under<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    root: &[usize],
) -> R<Option<Vec<usize>>> {
    let mut queue: std::collections::VecDeque<Vec<usize>> = std::collections::VecDeque::new();
    let mut seen: Vec<u64> = Vec::new();
    queue.push_back(root.to_vec());
    let mut looked = 0usize;
    while let Some(header) = queue.pop_front() {
        looked += 1;
        if looked > MAX_OBJECTS {
            return Ok(None);
        }
        let at = ev.node(doc, &header).map(|n| n.offset_bits).unwrap_or(u64::MAX);
        if seen.contains(&at) {
            continue;
        }
        seen.push(at);
        if let Some(found) = tree_of_header(ev, doc, &header)? {
            return Ok(Some(found));
        }
        for message in super::h5ad::collect_messages(ev, doc, &header, 0)? {
            let Some(kind) = ev.child_named(doc, &message, "type")? else { continue };
            // A link message, which is how a group with few links keeps one.
            if ev.node(doc, &kind)?.value.as_int() != Some(0x06) {
                continue;
            }
            let Some(body) = ev.child_named(doc, &message, "body")? else { continue };
            // A hard link keeps the address in a `target` of its own; a soft
            // or external one keeps a string there and reaches no header, so
            // asking for the object is what tells the two apart.
            let Some(target) = ev.child_named(doc, &body, "target")? else { continue };
            let Some(object) = ev.child_named(doc, &target, "object")? else { continue };
            let Some(object) = super::h5ad::inside(ev, doc, &object)? else { continue };
            if queue.len() + looked <= MAX_OBJECTS {
                queue.push_back(object);
            }
        }
    }
    Ok(None)
}

/// The innermost object header at or above `path`, where there is one.
fn enclosing_header<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<Option<Vec<usize>>> {
    let mut at = path.to_vec();
    for _ in 0..MAX_CLIMB {
        // An object header holds messages, and holds one of two things beside
        // them depending on its version: a version 1 header writes how many
        // there are, and a version 2 one writes flags and no count. Both are
        // asked for, because the two structures the template gives them share
        // a name and nothing else, and a test for the count alone finds no
        // header at all in a file a recent library wrote. Asked by field name
        // rather than by the template's own type name, so that renaming a
        // structure does not quietly stop this finding one.
        //
        // The continuation block an object's messages spill into has messages
        // and neither of the other two, which is what keeps it out: its
        // messages belong to the header that points at it, and answering with
        // the block would answer with an object that has no tree.
        let headed = has_field(ev, doc, &at, "message_count")? || has_field(ev, doc, &at, "header_flags")?;
        if headed && has_field(ev, doc, &at, "messages")? {
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
/// contiguously, every group small enough to keep its links as messages, and
/// every chunked dataset whose chunks are indexed some other way than by a
/// tree. There are four of those other ways, and the two arrays among them are
/// what a recent library reaches for first.
///
/// Four messages can name one, because two of them have a version 2 answer
/// beside the version 1 answer they had before: a group is named either by a
/// symbol table message or by a link info message, and a chunked dataset's
/// index is named by a data layout message whose `index_type` decides which
/// structure sits at the address.
fn tree_of_header<S: Source>(ev: &mut Evaluator, doc: &Document<S>, header: &[usize]) -> R<Option<Vec<usize>>> {
    for message in super::h5ad::collect_messages(ev, doc, header, 0)? {
        let Some(kind) = ev.child_named(doc, &message, "type")? else { continue };
        let kind = ev.node(doc, &kind)?.value.as_int().unwrap_or(-1);
        let Some(body) = ev.child_named(doc, &message, "body")? else { continue };
        // A link info message names the fractal heap the links are in and the
        // version 2 tree that indexes them by the hash of their names. The
        // heap is where the links are; the tree is the shape, and the shape is
        // what this is after.
        if kind == 0x02 {
            let Some(found) = ev.child_named(doc, &body, "name_index")? else { continue };
            let Some(found) = super::h5ad::inside(ev, doc, &found)? else { continue };
            if spot_of(ev, doc, &found)?.is_some() {
                return Ok(Some(found));
            }
            continue;
        }
        // A symbol table message names the heap the tree is placed inside, and
        // the tree under it.
        if kind == 0x11 {
            let Some(heap) = ev.child_named(doc, &body, "heap")? else { continue };
            let Some(heap) = super::h5ad::inside(ev, doc, &heap)? else { continue };
            let Some(found) = ev.child_named(doc, &heap, "tree")? else { continue };
            let Some(found) = super::h5ad::inside(ev, doc, &found)? else { continue };
            if spot_of(ev, doc, &found)?.is_some() {
                return Ok(Some(found));
            }
        }
        // A data layout message wraps the layout in a version, and a chunked
        // layout names its index `chunks` whichever of the five it is. Asking
        // what is at the address rather than reading `index_type` is what
        // makes the same two lines serve a version 1 tree and a version 2 one:
        // a fixed array or an extensible array there answers neither, and is
        // passed over rather than drawn as a tree it is not.
        if kind == 0x08 {
            let Some(layout) = ev.child_named(doc, &body, "body")? else { continue };
            let Some(storage) = ev.child_named(doc, &layout, "storage")? else { continue };
            let Some(found) = ev.child_named(doc, &storage, "chunks")? else { continue };
            let Some(found) = super::h5ad::inside(ev, doc, &found)? else { continue };
            if spot_of(ev, doc, &found)?.is_some() {
                return Ok(Some(found));
            }
            continue;
        }
    }
    Ok(None)
}

/// Which kind of version 1 node sits at `path`, or `None` for anything that is
/// not one.
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

/// The same question with the version 2 header among the answers: what is at
/// `path`, where it is the top of a tree or a node of one.
///
/// A version 2 header and not a version 2 node, because the template places
/// every node of one of those inside the header: the root node is a field of
/// it, and each node below is a field of the pointer above it. So a cursor
/// anywhere in a version 2 tree is a cursor inside the header, and the header
/// is what a walk starts from anyway.
fn spot_of<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<Option<Spot>> {
    if let Some(kind) = kind_of(ev, doc, path)? {
        return Ok(Some(Spot::V1(kind)));
    }
    // Two fields no other structure in this template has together: where the
    // root node is, and how many records are in it.
    if has_field(ev, doc, path, "root_node_address")? && has_field(ev, doc, path, "root_record_count")? {
        return Ok(Some(Spot::V2Header));
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
    let mut out = Tree {
        job: Job::Group,
        version: 1,
        record_type: 0,
        record_type_name: "",
        records: Records::Read,
        records_total: 0,
        nodes: Vec::new(),
        omitted: 0,
        coords: 0,
        coords_pad: true,
    };
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
        sign: if kind == Kind::LinkTable { "SNOD" } else { "TREE" },
        address: info.offset_bits / 8,
        size_bits: info.size_bits,
        level: 0,
        depth,
        entries: 0,
        first_key: String::new(),
        last_key: String::new(),
        truncated: false,
        first_entry_bits: 0,
        entry_bits: 0,
    });
    if kind == Kind::LinkTable {
        out.nodes[here].entries = field_int(ev, doc, at, "symbol_count")?.unwrap_or(0);
        let (first, last) = link_names(ev, doc, at)?;
        out.nodes[here].first_key = first;
        out.nodes[here].last_key = last;
        let (at_bits, wide) = stride(ev, doc, at, "symbols", info.offset_bits)?;
        out.nodes[here].first_entry_bits = at_bits;
        out.nodes[here].entry_bits = wide;
        return Ok(());
    }
    let level = field_int(ev, doc, at, "node_level")?.unwrap_or(0);
    let used = field_int(ev, doc, at, "entries_used")?.unwrap_or(0);
    out.nodes[here].level = level;
    out.nodes[here].entries = used;
    // Before the three returns below, and not after them: a level-zero chunk
    // node returns as soon as it has read its keys, and it is the node a
    // reader most wants divided, since each of its entries is a chunk.
    let (at_bits, wide) = stride(ev, doc, at, "entries", info.offset_bits)?;
    out.nodes[here].first_entry_bits = at_bits;
    out.nodes[here].entry_bits = wide;
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

/// Where the first element of the array `name` starts, in bits from `from`,
/// and how many bits one element takes. Both zero for an array with nothing in
/// it. See [`Node::first_entry_bits`].
///
/// Read from where the template put the entries rather than worked out from
/// the widths a superblock declares, because those widths are not one number
/// and not all of them are nearby. A group tree's key is a heap offset as wide
/// as the file's lengths; a chunk tree's is a size, a filter mask and one
/// offset per dimension of the dataset, and the dimension count is in a message
/// of an object header somewhere above the tree. The template has already read
/// all of it to place the entry, and an entry it placed is one a reader can
/// open; a width computed here beside it would be a second answer to the
/// question, free to disagree with the first.
///
/// One element measured and not two differenced: the elements of an array sit
/// one after another, so an element's own length is the step to the next one,
/// and a node holding a single entry is measured the same way as a node holding
/// two thousand.
fn stride<S: Source>(ev: &mut Evaluator, doc: &Document<S>, at: &[usize], name: &str, from: u64) -> R<(u64, u64)> {
    let Some(array) = ev.child_named(doc, at, name)? else { return Ok((0, 0)) };
    if ev.node(doc, &array)?.child_count == 0 {
        return Ok((0, 0));
    }
    let mut first = array;
    first.push(0);
    let first = ev.node(doc, &first)?;
    Ok((first.offset_bits.saturating_sub(from), first.size_bits))
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
///
/// A version 2 tree is served by the same pass and for the same reason, with
/// one thing about it worth saying out loud. A version 2 internal node holds
/// records of its own, which a version 1 index node does not, so its own range
/// is not only its children's. It does not have to be: a B-tree's records sit
/// *between* its children, so the span of the children brackets the span of
/// the records, and a range taken from the bottom is the node's whole range
/// either way.
///
/// Only called for a tree that has keys at all. A version 2 group tree has
/// none, and running this over one would mark every node above the leaves
/// short, which is a claim about the walk that is not true: nothing was
/// missed, there was simply nothing to read.
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
        // A version 2 leaf reads its own range from its own records, the way
        // the two above do, so what it read stands whatever happened below it.
        // There is nothing below it.
        let own = own || node.kind == Kind::Leaf;
        if !own && node.truncated {
            node.first_key = String::new();
            node.last_key = String::new();
        }
    }
}

/// The version 2 walk.
///
/// A `BTIN` writes, after its records, one pointer per child: the child's
/// address, how many records are in it, and, where the child is itself a
/// `BTIN`, how many records are in it and everything below it. The widths of
/// the last two are nowhere in the file. They are worked out from the node
/// size, the record size and the tree's depth, by an iteration over the levels
/// with a base-two logarithm in it.
///
/// This does that arithmetic in Rust (`Shape`) and then reads the nodes as
/// bytes, straight from the document, the way `h5ad`'s heap reader does. It was
/// written that way when the template's expressions had no logarithm and could
/// place nothing below the root. They have one now, and the template places
/// every node, with the same arithmetic written as the header's `levels`. The
/// walk still reads bytes, for what that buys and because two readings of the
/// same widths that agree on every real file are worth more than one: a width
/// out by a byte in either puts every child after the first somewhere else.
/// Three things follow from reading bytes rather than fields:
///
/// - **Paths are looked up, not walked.** Each node's [`Node::path`] is the
///   template's `node` under the pointer this walk followed, and is kept only
///   where that field lands on the address the walk read. A node the template
///   places somewhere else, or not at all, has an empty path: it can still go
///   to its bytes and still say everything the readout says, and what it
///   cannot do is open in the Listing. That is never a node of a well-formed
///   tree.
/// - **Little slice accounting.** `Evaluator::begin_slice` charges a walk for
///   the fields it reads, and a raw read is charged nothing; only the path
///   lookups are charged. What bounds this
///   instead is the node cap the caller passes and `MAX_BYTES`, and both are
///   reported as omitted nodes rather than silently applied.
/// - **Every number is checked.** These are files nobody vouched for. A node
///   size, a record size, a depth, a record count and an address are all read
///   from the file and all of them are bounded before they are used; a pointer
///   outside the document refuses rather than reading, because reading past the
///   end comes back as bytes still on their way and the view would then ask
///   again forever.
mod v2 {
    use super::{Job, Kind, Node, Records, Tree, NO_PARENT};
    use crate::document::Document;
    use crate::eval::{EvalError, Evaluator, R};
    use crate::source::Source;
    use std::collections::{HashSet, VecDeque};

    /// The signature, version and type byte every node opens with: what sits in
    /// front of the records.
    const HEAD: u64 = 6;
    /// Those three and the checksum at the end, which is what a node spends on
    /// being a node. HDF5's own name for the sum is the metadata prefix size.
    const PREFIX: u64 = HEAD + 4;
    /// How wide an address is. The template reads every address in an HDF5 file
    /// as the eight bytes it almost always is rather than as the width the
    /// superblock declares, so a file with narrower addresses has its header
    /// misread before this sees it. What happens then is that the root node's
    /// address fails the bounds check below and no tree is drawn, which is the
    /// right answer for a header this cannot read.
    const ADDR: u64 = 8;
    /// The largest node this will read. HDF5's default is 512 bytes for a
    /// group's index and a few kilobytes for a dataset's; this is far past
    /// anything a library writes and is the stop for a `node_size` field that
    /// is not one.
    const MAX_NODE: u64 = 1 << 22;
    /// The most bytes one walk will read. The node cap bounds how many nodes
    /// are drawn and this bounds what they cost, because the two are not the
    /// same question: four thousand nodes of a megabyte each is four gigabytes
    /// read to draw one picture.
    const MAX_BYTES: u64 = 64 << 20;
    /// The deepest tree this will work out a `Shape` for. A version 2 tree of
    /// depth eight with HDF5's own node sizes indexes more records than a file
    /// can hold; this is the stop for a `depth` field that is not a depth.
    const MAX_LEVELS: u64 = 24;

    /// How wide the counts in a child pointer are, level by level.
    ///
    /// Every number here comes out of the HDF5 library's own header setup,
    /// because there is nowhere else for it to come from: the file writes the
    /// node size, the record size and the depth, and the widths are implied.
    /// The chain is
    ///
    /// - a leaf holds `(node_size - PREFIX) / record_size` records at most;
    /// - the count of records in a child is written in as many bytes as that
    ///   number needs, and it is the same width at every level;
    /// - a pointer written by a node at level `d` is an address, that count,
    ///   and a running total for level `d - 1`, which is itself as many bytes
    ///   as the most records that can sit at or under a level `d - 1` node;
    /// - so how many records fit in a level `d` node depends on how wide a
    ///   level `d` pointer is, and the running total at level `d` depends on
    ///   that in turn.
    ///
    /// Level 0 has no running total at all, which is why a pointer from the
    /// level 1 nodes just above the leaves is two bytes shorter than a pointer
    /// from the level 2 nodes above those. Get that one width wrong and every
    /// child address after the first is read from the middle of the pointer
    /// before it, and the walk follows addresses into the middle of the file.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Shape {
        node_size: u64,
        record_size: u64,
        /// How wide a child pointer written by a node at this level is. Index 0
        /// is never used: a leaf writes no pointers.
        pointer: Vec<u64>,
        /// The most records a node at this level may hold, which is what a
        /// record count read out of the file is checked against.
        most: Vec<u64>,
        /// How wide the "records in this child" field of a pointer is. One
        /// width for the whole tree, taken from the leaves, which hold the
        /// most.
        count: u64,
    }

    impl Shape {
        /// The widths for a tree of this node size, record size and depth, or
        /// `None` where those three are not a tree: a node too small to hold
        /// its own prefix, a record too big for a node, a depth past
        /// `MAX_LEVELS`, or a level that turns out to hold no records at all,
        /// which would be a tree that cannot be walked.
        fn read(node_size: u64, record_size: u64, depth: u64) -> Option<Shape> {
            if node_size < PREFIX + 1 || node_size > MAX_NODE || record_size == 0 || depth > MAX_LEVELS {
                return None;
            }
            if record_size > node_size - PREFIX {
                return None;
            }
            let leaf = (node_size - PREFIX) / record_size;
            if leaf == 0 {
                return None;
            }
            let count = enc_size(leaf);
            let mut most = vec![leaf];
            let mut pointer = vec![0u64];
            // How many records sit at or under one node of the level below,
            // which is what the running total in a pointer has to be wide
            // enough to write. No bytes at all at level 0: a pointer at a
            // leaf's own level writes no total, because a leaf has nothing
            // below it.
            let mut running = leaf;
            let mut running_width = 0u64;
            for _ in 1..=depth {
                let width = ADDR + count + running_width;
                if node_size < PREFIX + width {
                    return None;
                }
                let here = (node_size - (PREFIX + width)) / (record_size + width);
                if here == 0 {
                    return None;
                }
                // The most records at or under one node of this level: its own
                // records, and the whole of each of its `here + 1` children.
                running = here.saturating_mul(running.saturating_add(1)).saturating_add(running);
                running_width = enc_size(running);
                most.push(here);
                pointer.push(width);
            }
            Some(Shape { node_size, record_size, pointer, most, count })
        }

        /// The width of a pointer written by a node at `level`, or `None` for a
        /// level this shape does not have.
        fn pointer(&self, level: u64) -> Option<u64> {
            usize::try_from(level).ok().and_then(|i| self.pointer.get(i)).copied()
        }

        /// The most records a node at `level` may hold, or `None` for a level
        /// this shape does not have.
        fn most(&self, level: u64) -> Option<u64> {
            usize::try_from(level).ok().and_then(|i| self.most.get(i)).copied()
        }
    }

    /// How many bytes it takes to write `n`: the position of its top set bit,
    /// divided by eight, plus one. HDF5 sizes both counts in a child pointer
    /// this way. Never zero, because a count of zero is still written.
    fn enc_size(n: u64) -> u64 {
        let bits = if n == 0 { 1 } else { 64 - u64::from(n.leading_zeros()) };
        (bits - 1) / 8 + 1
    }

    /// Every node of the version 2 tree whose header is at `header`, breadth
    /// first, for the reason the version 1 walk gives: a cap has to fall
    /// somewhere, and cutting across keeps the rows nearest the root, which is
    /// the part of a B-tree that says how it branches.
    pub(super) fn walk<S: Source>(ev: &mut Evaluator, doc: &Document<S>, header: &[usize], limit: usize) -> R<Tree> {
        let kind = super::field_int(ev, doc, header, "type")?.unwrap_or(u64::MAX);
        let (job, records, name) = classify(kind);
        let mut out = Tree {
            job,
            version: 2,
            record_type: u8::try_from(kind).unwrap_or(0),
            record_type_name: name,
            records,
            records_total: super::field_int(ev, doc, header, "record_count")?.unwrap_or(0),
            nodes: Vec::new(),
            omitted: 0,
            coords: 0,
            coords_pad: false,
        };
        let node_size = super::field_int(ev, doc, header, "node_size")?.unwrap_or(0);
        let record_size = super::field_int(ev, doc, header, "record_size")?.unwrap_or(0);
        let depth = super::field_int(ev, doc, header, "depth")?.unwrap_or(u64::MAX);
        let root_at = super::field_int(ev, doc, header, "root_node_address")?.unwrap_or(u64::MAX);
        let root_records = super::field_int(ev, doc, header, "root_record_count")?.unwrap_or(0);
        let Some(shape) = Shape::read(node_size, record_size, depth) else {
            return Err(EvalError::Failed("not a version 2 b-tree header".into()));
        };
        // How many numbers an unfiltered chunk record holds: the record is an
        // address and then one offset per dataset dimension, and nothing else,
        // so the rank falls out of the record size. A record size that is not
        // an address plus a whole number of offsets is not one of these, and
        // says so rather than reading offsets out of the wrong place.
        if out.records == Records::Read && job == Job::Chunk {
            if record_size > ADDR && (record_size - ADDR).is_multiple_of(8) {
                out.coords = (record_size - ADDR) / 8;
            } else {
                out.records = Records::Unread;
            }
        }
        // The root node's path. Every node under it is found from its parent's
        // path when it is reached; see `placed`.
        let root_path = match ev.child_named(doc, header, "root_node")? {
            Some(field) => crate::formats::h5ad::inside(ev, doc, &field)?.unwrap_or_default(),
            None => Vec::new(),
        };

        let mut queue: VecDeque<Waiting> = VecDeque::new();
        let mut seen: HashSet<u64> = HashSet::new();
        let mut spent: u64 = 0;
        queue.push_back(Waiting {
            at: root_at,
            level: depth,
            records: root_records,
            parent: NO_PARENT,
            index: 0,
            depth: 0,
        });
        while let Some(next) = queue.pop_front() {
            if out.nodes.len() >= limit.max(1) || spent >= MAX_BYTES {
                out.omitted += queue.len() as u64 + 1;
                if let Some(node) = out.nodes.get_mut(next.parent) {
                    node.truncated = true;
                }
                break;
            }
            // A node whose address has already been walked is a ring. Following
            // one is not slow but endless, and a file that has one is a file
            // that was written wrong or truncated mid-write.
            if !seen.insert(next.at) {
                if let Some(node) = out.nodes.get_mut(next.parent) {
                    node.truncated = true;
                }
                continue;
            }
            let path = match out.nodes.get(next.parent) {
                None => root_path.clone(),
                Some(parent) => {
                    let above = parent.path.clone();
                    placed(ev, doc, &above, next.index, next.at)?
                }
            };
            match add(doc, &next, &path, &shape, &mut out, &mut queue, limit, &mut spent) {
                Ok(()) => {}
                Err(e) if e.interrupted() => return Err(e),
                // One node this could not read. Its siblings still read, and a
                // tree missing one box says more than an error in place of the
                // whole picture; the root is the exception, since with that
                // unread there is no tree to answer with.
                Err(e) => {
                    if next.parent == NO_PARENT {
                        return Err(e);
                    }
                    if let Some(node) = out.nodes.get_mut(next.parent) {
                        node.truncated = true;
                    }
                }
            }
        }
        // Only where there are keys to carry up. A group tree has none, and the
        // pass would read every node above the leaves as short.
        if out.coords > 0 {
            super::ranges(&mut out);
        }
        Ok(out)
    }

    /// A child that has been pointed at and not yet read. The level and the
    /// record count come from the pointer that named it, because a version 2
    /// node writes neither about itself: it holds records and says nothing
    /// about how many, and which of the two kinds it is is its signature.
    struct Waiting {
        at: u64,
        level: u64,
        records: u64,
        parent: usize,
        /// Which of the parent's pointers named it, which is also which of the
        /// template's `children` holds it.
        index: u64,
        depth: usize,
    }

    /// Where the template places the child that pointer `index` of the node at
    /// `parent` names, or an empty path where it places none there.
    ///
    /// Checked against the address the walk read before it is handed back. A
    /// path is what a double-click opens, and a path to the wrong node would
    /// open a plausible node that is not the one the reader pressed, which is
    /// worse than opening nothing. So a parent with no path, a node the
    /// template cannot read, and one it reads at another address all come back
    /// empty, and only a read interrupted for want of bytes is passed on.
    fn placed<S: Source>(ev: &mut Evaluator, doc: &Document<S>, parent: &[usize], index: u64, at: u64) -> R<Vec<usize>> {
        if parent.is_empty() {
            return Ok(Vec::new());
        }
        let found = (|| -> R<Option<Vec<usize>>> {
            let Some(mut child) = ev.child_named(doc, parent, "children")? else { return Ok(None) };
            child.push(usize::try_from(index).unwrap_or(usize::MAX));
            let Some(node) = ev.child_named(doc, &child, "node")? else { return Ok(None) };
            let Some(node) = crate::formats::h5ad::inside(ev, doc, &node)? else { return Ok(None) };
            Ok((ev.node(doc, &node)?.offset_bits == at * 8).then_some(node))
        })();
        match found {
            Ok(path) => Ok(path.unwrap_or_default()),
            Err(e) if e.interrupted() => Err(e),
            Err(_) => Ok(Vec::new()),
        }
    }

    /// One node: what it holds, and its children queued behind it.
    #[allow(clippy::too_many_arguments)]
    fn add<S: Source>(
        doc: &Document<S>,
        next: &Waiting,
        path: &[usize],
        shape: &Shape,
        out: &mut Tree,
        queue: &mut VecDeque<Waiting>,
        limit: usize,
        spent: &mut u64,
    ) -> R<()> {
        let leaf = next.level == 0;
        let Some(most) = shape.most(next.level) else {
            return Err(EvalError::Failed("b-tree node below the depth its header declares".into()));
        };
        // A count past what a node of this size can hold is a count read out of
        // the wrong bytes, and going on with it would read records out of the
        // wrong bytes too.
        if next.records > most {
            return Err(EvalError::Failed("b-tree node holds more records than it has room for".into()));
        }
        let pointers = if leaf { 0 } else { next.records + 1 };
        let width = if leaf { 0 } else { shape.pointer(next.level).unwrap_or(0) };
        let used = PREFIX + next.records * shape.record_size + pointers * width;
        if used > shape.node_size {
            return Err(EvalError::Failed("b-tree node is longer than the node size its header declares".into()));
        }
        let bytes = read_at(doc, next.at, used)?;
        *spent += used;
        let sign = match &bytes[..4] {
            b"BTLF" if leaf => "BTLF",
            b"BTIN" if !leaf => "BTIN",
            // Not "no signature": the level says which of the two belongs here,
            // and a `BTLF` where a `BTIN` should be is a tree whose depth and
            // whose nodes disagree. Either way the bytes are not the node the
            // pointer promised, and nothing below them is worth following.
            _ => return Err(EvalError::Failed("not the b-tree node the pointer above it named".into())),
        };
        let here = out.nodes.len();
        out.nodes.push(Node {
            path: path.to_vec(),
            parent: next.parent,
            kind: if leaf { Kind::Leaf } else { Kind::Index },
            sign,
            address: next.at,
            // What is written, not what is reserved. A version 2 node is
            // allocated at its full node size and only the front of it is
            // written, so measuring one to the node size would draw every node
            // of a tree the same size and say nothing about how full it is.
            size_bits: used * 8,
            level: next.level,
            depth: next.depth,
            entries: next.records,
            first_key: String::new(),
            last_key: String::new(),
            truncated: false,
            // Both kinds write their records straight after the signature, the
            // version and the type byte, each `record_size` long, which is
            // where [`chunk_keys`] reads them from. `HEAD` and not `PREFIX`:
            // the four bytes that make up the difference are the checksum at
            // the far end of the node, not something in front of the records.
            //
            // Settled even for a node holding no records, because the header
            // settles it and not the node: there is simply nothing at the
            // stride. The child pointers a `BTIN` writes after its records are
            // not on it; see [`Node::first_entry_bits`].
            first_entry_bits: HEAD * 8,
            entry_bits: shape.record_size * 8,
        });
        if leaf {
            if out.coords > 0 {
                let (first, last) = chunk_keys(&bytes, next.records, shape.record_size, out.coords);
                out.nodes[here].first_key = first;
                out.nodes[here].last_key = last;
            }
            return Ok(());
        }
        if next.depth >= super::MAX_DEPTH {
            out.nodes[here].truncated = true;
            out.omitted += pointers;
            return Ok(());
        }
        // Room for what is in and what is already waiting, the way the version
        // 1 walk counts it: a child queued now is a node later.
        let room = limit.saturating_sub(out.nodes.len() + queue.len());
        let taken = room.min(usize::try_from(pointers).unwrap_or(usize::MAX));
        let from = HEAD + next.records * shape.record_size;
        for i in 0..taken as u64 {
            let at = from + i * width;
            queue.push_back(Waiting {
                at: le(&bytes, at, ADDR),
                level: next.level - 1,
                records: le(&bytes, at + ADDR, shape.count),
                parent: here,
                index: i,
                depth: next.depth + 1,
            });
        }
        if (taken as u64) < pointers {
            out.nodes[here].truncated = true;
            out.omitted += pointers - taken as u64;
        }
        Ok(())
    }

    /// `len` bytes at `at`, or a refusal.
    ///
    /// The bounds check is before the read and not after it, and that order is
    /// the point. A document reads past its own end as zeros or as bytes still
    /// on their way, and a walk that took the second answer for a pointer into
    /// nowhere would hand the view a "still reading" it can never finish
    /// waiting for.
    fn read_at<S: Source>(doc: &Document<S>, at: u64, len: u64) -> R<Vec<u8>> {
        let end = at.saturating_add(len);
        if len < 4 || end > doc.len_bytes() {
            return Err(EvalError::Failed("b-tree node outside the file".into()));
        }
        let mut buf = vec![0u8; len as usize];
        let missing = doc.read_bits(at * 8, len * 8, &mut buf);
        if !missing.is_empty() {
            return Err(EvalError::Pending(missing));
        }
        Ok(buf)
    }

    /// A little-endian number of `len` bytes at `at`, or zero where the bytes
    /// are not all there. The caller has already measured the node against its
    /// node size, so a short read here would be a bug rather than a file.
    fn le(bytes: &[u8], at: u64, len: u64) -> u64 {
        let (Ok(at), Ok(len)) = (usize::try_from(at), usize::try_from(len.min(8))) else { return 0 };
        let Some(slice) = bytes.get(at..at.saturating_add(len)) else { return 0 };
        let mut out = 0u64;
        for (i, b) in slice.iter().enumerate() {
            out |= u64::from(*b) << (8 * i);
        }
        out
    }

    /// The chunk offsets of the first and last record of a leaf, written out.
    ///
    /// Two reads a node rather than one a record: the records are in order, so
    /// the two ends are the range and whatever is between them is the
    /// Listing's business. The same rule the version 1 walk reads its chunk
    /// keys by, and one difference worth knowing: a version 1 key ends with an
    /// offset within an element that is always zero, and one of these does not.
    /// See [`Tree::coords_pad`].
    fn chunk_keys(bytes: &[u8], records: u64, record_size: u64, coords: u64) -> (String, String) {
        if records == 0 {
            return (String::new(), String::new());
        }
        let first = chunk_key(bytes, 0, record_size, coords);
        let last = if records == 1 { first.clone() } else { chunk_key(bytes, records - 1, record_size, coords) };
        (first, last)
    }

    fn chunk_key(bytes: &[u8], i: u64, record_size: u64, coords: u64) -> String {
        let at = HEAD + i * record_size + ADDR;
        let mut parts: Vec<String> = Vec::new();
        for k in 0..coords.min(16) {
            parts.push(le(bytes, at + k * 8, 8).to_string());
        }
        parts.join(", ")
    }

    /// What this tree indexes, how far its records are read, and what the
    /// specification calls its type.
    ///
    /// Two of the twelve are read: link names, whose records name a link in the
    /// group's fractal heap, and unfiltered chunks, whose records name a chunk
    /// and hold its offset in the dataset. The rest keep their shape and say so.
    ///
    /// Filtered chunks are the near miss. A record of one holds the chunk's
    /// address, then how many bytes the filtered chunk takes, then which
    /// filters were skipped, and then the offsets. The second of those is as
    /// wide as the layout message decided it needed to be, and that message is
    /// somewhere else in the object header, so the offsets sit at an unknown
    /// place in the record until it has been read. Rather than guess a width
    /// and print offsets from wherever that landed, these are drawn as shape
    /// and nothing else.
    fn classify(kind: u64) -> (Job, Records, &'static str) {
        let name = crate::formats::hdf5::BTREE2_TYPE
            .iter()
            .find(|(n, _)| u64::try_from(*n).is_ok_and(|n| n == kind))
            .map(|(_, name)| *name);
        let Some(name) = name else { return (Job::Other, Records::Unknown, "") };
        match kind {
            5 => (Job::Group, Records::Read, name),
            10 => (Job::Chunk, Records::Read, name),
            6 => (Job::Group, Records::Unread, name),
            11 => (Job::Chunk, Records::Unread, name),
            _ => (Job::Other, Records::Unread, name),
        }
    }
}
