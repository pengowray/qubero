//! What an HDF5 file holds, in the file's own terms rather than the
//! template's.
//!
//! The template describes an HDF5 file exactly and says nothing about what it
//! is *for*. A reader opening a Zebrahub `.h5ad` wants to be told that it is an
//! AnnData object of 1,251 cells by 32,060 genes, that `X` is a sparse matrix
//! kept as three arrays, and that `obs` is a table of eighteen columns. All of
//! that is written in the file, in attributes AnnData agreed on and in the
//! dataspace and datatype messages every object carries; none of it is a field
//! the template could name, because it is a reading of many fields at once.
//!
//! So this walks the group tree the template already places and gathers, for
//! every object it meets: the path it goes by, what AnnData calls it
//! (`encoding-type`), the shape of its dataspace, what one element is, and how
//! its bytes are stored. That is a contents list for any HDF5 file, and the
//! `.h5ad` conventions on top of it are two attributes and a name.
//!
//! Reading `encoding-type` is what the global heap was for. The attribute's
//! value is a variable-length string, which is a length, the address of a heap
//! collection and an index into it; the objects in a collection have no fixed
//! size, so the one with that index is found by walking the collection from its
//! first object. A field cannot do that walk. A reader can, and this is the
//! reader.
//!
//! The walk is bounded: a file with more objects than [`LIMIT`] says how many
//! it showed and how many there were, rather than growing without end. Group
//! cycles cannot spin it, since an object header already seen is not opened
//! again.

use crate::document::Document;
use crate::eval::{EvalError, Evaluator, Value, R};
use crate::source::Source;

/// How many objects one walk reports. A single-cell atlas has a few hundred;
/// a file with more than this is one where a list was never going to be the
/// answer anyway.
pub const LIMIT: usize = 512;

/// How far down the group tree the walk goes. Nothing anyone writes nests this
/// deep, and a file whose links form a ring cannot make the walk longer than
/// this even if the check below somehow missed one.
const MAX_DEPTH: usize = 32;

/// Where an object's bytes are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Storage {
    /// A group holds no elements of its own.
    None,
    /// One run of bytes, and how many.
    Contiguous(u64),
    /// Chunks of these dimensions, through these filters, named as the file
    /// names them.
    Chunked { dims: Vec<u64>, filters: Vec<String> },
    /// Written into the object header itself, which is what a tiny dataset
    /// gets instead of a run of its own.
    Compact(u64),
}

/// One object of the file: a group or a dataset.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    /// Where it is in the template, so picking it can move the cursor.
    pub path: Vec<usize>,
    /// The path it goes by inside the file: `/obs/n_genes`.
    pub name: String,
    /// True for a group, which is a name for other objects and nothing else.
    pub group: bool,
    /// What AnnData calls it, from the `encoding-type` attribute: `dataframe`,
    /// `csr_matrix`, `array`, `categorical`. Empty for a file that keeps no
    /// such attribute, which is every HDF5 file that is not an `.h5ad`.
    pub encoding: String,
    /// The dataspace, longest dimension first as the file writes it.
    pub shape: Vec<u64>,
    /// What one element is: `f32`, `i64`, `20-byte text`, `variable-length`.
    pub element: String,
    pub storage: Storage,
    /// Where the object header is, for a reader who wants the address.
    pub address: u64,
}

/// What the walk made of the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Contents {
    pub objects: Vec<Object>,
    /// How many objects there were, where that is more than were kept.
    pub total: usize,
    /// The root group's `encoding-type`, which is `anndata` for an `.h5ad`
    /// written by a library that stamps it. Older ones do not, so this is
    /// empty for plenty of files that are one.
    pub encoding: String,
    /// Whether the file is an AnnData object: it says so, or it has the three
    /// things one always has.
    pub anndata: bool,
    /// What that kind of file counts in: rows of `obs` and rows of `var`,
    /// which are cells and genes. Zero where the file does not say.
    pub rows: u64,
    pub columns: u64,
}

/// Walk the file and say what is in it.
pub fn contents<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Contents> {
    let mut out = Contents {
        objects: Vec::new(),
        total: 0,
        encoding: String::new(),
        anndata: false,
        rows: 0,
        columns: 0,
    };
    let Some(root) = root_header(ev, doc)? else { return Ok(out) };
    let mut seen = vec![addr_of(ev, doc, &root)];
    walk(ev, doc, &root, "", &mut out, &mut seen, 0)?;
    out.encoding = out.objects.iter().find(|o| o.name == "/").map(|o| o.encoding.clone()).unwrap_or_default();
    // AnnData writes the two numbers everything else in the file is measured
    // in as the shape of its obs and var tables.
    for object in &out.objects {
        match object.name.as_str() {
            "/obs/_index" | "/obs/cell_id" => out.rows = object.shape.first().copied().unwrap_or(0),
            "/var/_index" | "/var/gene_id" => out.columns = object.shape.first().copied().unwrap_or(0),
            _ => {}
        }
    }
    // A sparse `X` carries the two numbers as an attribute, which is the one
    // place they are written when the matrix is not a dataset.
    if let Some(shape) = out.objects.iter().find(|o| o.name == "/X").map(|o| o.shape.clone()) {
        if shape.len() == 2 {
            out.rows = shape[0];
            out.columns = shape[1];
        }
    }
    let has = |name: &str| out.objects.iter().any(|o| o.name == name);
    out.anndata = out.encoding == "anndata" || (has("/X") && has("/obs") && has("/var"));
    Ok(out)
}

/// The object header of the root group, which every other object hangs under.
fn root_header<S: Source>(ev: &mut Evaluator, doc: &Document<S>) -> R<Option<Vec<usize>>> {
    let Some(sb) = ev.child_named(doc, &[], "superblock")? else { return Ok(None) };
    // Version 0 and 1 name the root group with a symbol table entry; the later
    // ones name its object header outright.
    let entry = ev.child_named(doc, &sb, "root_group")?;
    let Some(entry) = entry else { return Ok(None) };
    if ev.child_named(doc, &entry, "object")?.is_some() {
        let object = ev.child_named(doc, &entry, "object")?.expect("just asked");
        return Ok(inside(ev, doc, &object)?);
    }
    Ok(inside(ev, doc, &entry)?)
}

/// What a field that reads its contents somewhere else points at, where it
/// points at anything.
fn inside<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> R<Option<Vec<usize>>> {
    if ev.node(doc, path)?.child_count == 0 {
        return Ok(None);
    }
    let mut p = path.to_vec();
    p.push(0);
    Ok(Some(p))
}

fn addr_of<S: Source>(ev: &mut Evaluator, doc: &Document<S>, path: &[usize]) -> u64 {
    ev.node(doc, path).map(|n| n.offset_bits / 8).unwrap_or(0)
}

/// One object: what its messages say about it, and then its links.
fn walk<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    header: &[usize],
    name: &str,
    out: &mut Contents,
    seen: &mut Vec<u64>,
    depth: usize,
) -> R<()> {
    let messages = collect_messages(ev, doc, header, 0)?;
    let mut object = Object {
        path: header.to_vec(),
        name: if name.is_empty() { "/".to_string() } else { name.to_string() },
        group: false,
        encoding: String::new(),
        shape: Vec::new(),
        element: String::new(),
        storage: Storage::None,
        address: addr_of(ev, doc, header),
    };
    let mut links: Vec<(String, Vec<usize>)> = Vec::new();
    let mut filters: Vec<String> = Vec::new();
    for message in &messages {
        let Some(kind) = ev.child_named(doc, message, "type")? else { continue };
        let kind = ev.node(doc, &kind)?.value.as_int().unwrap_or(-1);
        let Some(body) = ev.child_named(doc, message, "body")? else { continue };
        match kind {
            // Dataspace, datatype, layout: what a dataset is.
            0x01 => object.shape = dimensions(ev, doc, &body)?,
            0x03 => object.element = element_name(ev, doc, &body)?,
            0x08 => object.storage = storage(ev, doc, &body, &filters)?,
            0x0b => filters = filter_names(ev, doc, &body)?,
            0x0c => match attribute(ev, doc, &body)? {
                Some((key, text, _)) if key == "encoding-type" => object.encoding = text,
                // A sparse matrix is a group, so it has no dataspace of its
                // own: how big it is, is written here and nowhere else.
                Some((key, _, numbers)) if key == "shape" && object.shape.is_empty() => {
                    object.shape = numbers;
                }
                _ => {}
            },
            // A group, in either of the two ways one is written.
            0x11 => {
                object.group = true;
                links.extend(symbol_table_links(ev, doc, &body)?);
            }
            0x06 => {
                object.group = true;
                if let Some(link) = link_message(ev, doc, &body)? {
                    links.push(link);
                }
            }
            // A group with more links than fit as messages keeps them in a
            // fractal heap instead, and they are links all the same.
            0x02 => {
                object.group = true;
                links.extend(heap_links(ev, doc, &body)?);
            }
            _ => {}
        }
    }
    // The filter pipeline may be read after the layout message that needs it.
    if let Storage::Chunked { dims, filters: had } = &object.storage {
        if had.is_empty() && !filters.is_empty() {
            object.storage = Storage::Chunked { dims: dims.clone(), filters: filters.clone() };
        }
    }
    if out.objects.len() < LIMIT {
        out.objects.push(object);
    }
    out.total += 1;
    if depth >= MAX_DEPTH {
        return Ok(());
    }
    links.sort_by(|a, b| a.0.cmp(&b.0));
    // A group written both ways at once, or a heap block whose free space
    // still holds a link that was moved, can name the same thing twice. The
    // first of them is the answer: they are sorted by name, so the two sit
    // together and the later one adds nothing.
    links.dedup_by(|a, b| a.0 == b.0);
    for (link_name, header) in links {
        if out.total >= LIMIT * 4 {
            return Ok(());
        }
        let at = addr_of(ev, doc, &header);
        // A hard link can point back at a group already open, which is a legal
        // file and a walk with no end. An object read once is read once.
        if seen.contains(&at) {
            continue;
        }
        seen.push(at);
        let full = format!("{}/{}", name.trim_end_matches('/'), link_name);
        match walk(ev, doc, &header, &full, out, seen, depth + 1) {
            Ok(()) => {}
            Err(e) if e.interrupted() => return Err(e),
            // One object that will not read says nothing about the others.
            Err(_) => continue,
        }
    }
    Ok(())
}

/// Every message of an object header, following the continuations that hold
/// the ones that did not fit. Those nest: a header with a lot to say writes a
/// continuation whose last message is another continuation, so this follows
/// them as far as they go.
fn collect_messages<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    header: &[usize],
    depth: usize,
) -> R<Vec<Vec<usize>>> {
    let Some(messages) = ev.child_named(doc, header, "messages")? else { return Ok(Vec::new()) };
    collect_from(ev, doc, &messages, depth)
}

/// One list of messages, and the lists its continuations point at.
fn collect_from<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    list: &[usize],
    depth: usize,
) -> R<Vec<Vec<usize>>> {
    let mut out = Vec::new();
    if depth > MAX_DEPTH {
        return Ok(out);
    }
    let n = ev.node(doc, list)?.child_count;
    for i in 0..n {
        let mut message = list.to_vec();
        message.push(i as usize);
        let Some(kind) = ev.child_named(doc, &message, "type")? else { continue };
        if ev.node(doc, &kind)?.value.as_int() != Some(0x10) {
            out.push(message);
            continue;
        }
        // A continuation: the messages it points at are this object's own. A
        // version 2 header wraps them in a signed block and a version 1 one
        // does not; either way what is wanted is the list inside.
        let Some(body) = ev.child_named(doc, &message, "body")? else { continue };
        let Some(pointer) = ev.child_named(doc, &body, "messages")? else { continue };
        let Some(inner) = inside(ev, doc, &pointer)? else { continue };
        let inner = ev.child_named(doc, &inner, "messages")?.unwrap_or(inner);
        match collect_from(ev, doc, &inner, depth + 1) {
            Ok(more) => out.extend(more),
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => continue,
        }
    }
    Ok(out)
}

/// The dimensions of a dataspace.
fn dimensions<S: Source>(ev: &mut Evaluator, doc: &Document<S>, body: &[usize]) -> R<Vec<u64>> {
    let Some(dims) = ev.child_named(doc, body, "dimensions")? else { return Ok(Vec::new()) };
    let n = ev.node(doc, &dims)?.child_count;
    let mut out = Vec::new();
    for i in 0..n.min(8) {
        let mut d = dims.clone();
        d.push(i as usize);
        out.push(ev.node(doc, &d)?.value.as_int().unwrap_or(0).max(0) as u64);
    }
    Ok(out)
}

/// What one element of a dataset is, in a word.
fn element_name<S: Source>(ev: &mut Evaluator, doc: &Document<S>, body: &[usize]) -> R<String> {
    let class = match ev.child_named(doc, body, "class")? {
        Some(p) => ev.node(doc, &p)?.value.as_int().unwrap_or(-1),
        None => -1,
    };
    let size = match ev.child_named(doc, body, "size")? {
        Some(p) => ev.node(doc, &p)?.value.as_int().unwrap_or(0).max(0) as u64,
        None => 0,
    };
    let signed = match ev.child_named(doc, body, "bit_field")? {
        Some(p) => ev.node(doc, &p)?.value.as_int().unwrap_or(0) & 8 == 8,
        None => false,
    };
    Ok(match (class, size) {
        (0, n) if signed => format!("i{}", n * 8),
        (0, n) => format!("u{}", n * 8),
        (1, 2) => "f16".into(),
        (1, 4) => "f32".into(),
        (1, 8) => "f64".into(),
        (3, n) => format!("{n}-byte text"),
        (6, n) => format!("{n}-byte compound"),
        (8, _) => "enumerated".into(),
        (9, _) => "variable-length".into(),
        (_, 0) => String::new(),
        (_, n) => format!("{n}-byte element"),
    })
}

/// Where a dataset's bytes are, from its layout message.
fn storage<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    body: &[usize],
    filters: &[String],
) -> R<Storage> {
    let Some(layout) = ev.child_named(doc, body, "body")? else { return Ok(Storage::None) };
    let Some(class) = ev.child_named(doc, &layout, "layout_class")? else { return Ok(Storage::None) };
    let class = ev.node(doc, &class)?.value.as_int().unwrap_or(-1);
    let Some(store) = ev.child_named(doc, &layout, "storage")? else { return Ok(Storage::None) };
    let size = |ev: &mut Evaluator, doc: &Document<S>| -> R<u64> {
        Ok(match ev.child_named(doc, &store, "size")? {
            Some(p) => ev.node(doc, &p)?.value.as_int().unwrap_or(0).max(0) as u64,
            None => 0,
        })
    };
    Ok(match class {
        0 => Storage::Compact(size(ev, doc)?),
        1 => Storage::Contiguous(size(ev, doc)?),
        2 => {
            let mut dims = Vec::new();
            if let Some(list) = ev.child_named(doc, &store, "chunk_dimensions")? {
                let n = ev.node(doc, &list)?.child_count;
                for i in 0..n.min(8) {
                    let mut d = list.clone();
                    d.push(i as usize);
                    dims.push(ev.node(doc, &d)?.value.as_int().unwrap_or(0).max(0) as u64);
                }
            }
            Storage::Chunked { dims, filters: filters.to_vec() }
        }
        _ => Storage::None,
    })
}

/// The filters a dataset's chunks were written through, in that order.
fn filter_names<S: Source>(ev: &mut Evaluator, doc: &Document<S>, body: &[usize]) -> R<Vec<String>> {
    let Some(list) = ev.child_named(doc, body, "filters")? else { return Ok(Vec::new()) };
    let n = ev.node(doc, &list)?.child_count;
    let mut out = Vec::new();
    for i in 0..n.min(16) {
        let mut filter = list.clone();
        filter.push(i as usize);
        let Some(id) = ev.child_named(doc, &filter, "filter_id")? else { continue };
        let raw = ev.node(doc, &id)?.value.as_int().unwrap_or(-1);
        out.push(match super::hdf5_chunk::filter_name(raw.clamp(0, i128::from(u16::MAX)) as u16) {
            Some(name) => name.to_string(),
            None => format!("filter {raw}"),
        });
    }
    Ok(out)
}

/// An attribute: its name, what it says where that is a string, and the
/// numbers it holds where it holds numbers. A sparse matrix is a group with no
/// dataspace of its own and its shape written here, which is the one place a
/// contents list can learn how big it is.
fn attribute<S: Source>(ev: &mut Evaluator, doc: &Document<S>, body: &[usize]) -> R<Option<(String, String, Vec<u64>)>> {
    let Some(name) = ev.child_named(doc, body, "name")? else { return Ok(None) };
    let Value::Str(name) = ev.node(doc, &name)?.value else { return Ok(None) };
    let Some(data) = ev.child_named(doc, body, "data")? else { return Ok(None) };
    let Some(elements) = ev.child_named(doc, &data, "elements")? else { return Ok(None) };
    let count = ev.node(doc, &elements)?.child_count;
    if count == 0 {
        return Ok(Some((name, String::new(), Vec::new())));
    }
    let mut numbers = Vec::new();
    let mut text = String::new();
    for i in 0..count.min(8) {
        let mut element = elements.clone();
        element.push(i as usize);
        match ev.node(doc, &element)?.value {
            Value::Str(s) if i == 0 => text = s,
            // A variable-length string is a note saying where the string is,
            // and the walk to it is what a reader can do and a field cannot.
            Value::Composite { .. } if i == 0 => {
                text = vlen_string(ev, doc, &element)?.unwrap_or_default();
            }
            other => {
                if let Some(n) = other.as_int() {
                    numbers.push(n.max(0) as u64);
                }
            }
        }
    }
    Ok(Some((name, text, numbers)))
}

/// The string a variable-length element points at: the object with that index
/// in the global heap collection at that address, found by walking the
/// collection from its first object.
fn vlen_string<S: Source>(ev: &mut Evaluator, doc: &Document<S>, at: &[usize]) -> R<Option<String>> {
    let number = |ev: &mut Evaluator, doc: &Document<S>, name: &str| -> R<i128> {
        Ok(match ev.child_named(doc, at, name)? {
            Some(p) => ev.node(doc, &p)?.value.as_int().unwrap_or(-1),
            None => -1,
        })
    };
    let want = number(ev, doc, "object_index")?;
    let length = number(ev, doc, "length")?;
    if want < 0 || length < 0 {
        return Ok(None);
    }
    let Some(collection) = ev.child_named(doc, at, "collection")? else { return Ok(None) };
    let Some(heap) = inside(ev, doc, &collection)? else { return Ok(None) };
    let Some(objects) = ev.child_named(doc, &heap, "objects")? else { return Ok(None) };
    let n = ev.node(doc, &objects)?.child_count;
    for i in 0..n {
        let mut object = objects.clone();
        object.push(i as usize);
        let Some(index) = ev.child_named(doc, &object, "object_index")? else { continue };
        if ev.node(doc, &index)?.value.as_int() != Some(want) {
            continue;
        }
        let Some(data) = ev.child_named(doc, &object, "data")? else { return Ok(None) };
        let info = ev.node(doc, &data)?;
        let bytes = read_bytes(doc, info.offset_bits, length.min(4096) as u64)?;
        return Ok(Some(String::from_utf8_lossy(&bytes).trim_end_matches('\0').to_string()));
    }
    Ok(None)
}

/// The bytes themselves, since what an object of a heap holds is a run of them
/// and not a field.
fn read_bytes<S: Source>(doc: &Document<S>, at_bits: u64, len: u64) -> R<Vec<u8>> {
    let mut buf = vec![0u8; len as usize];
    let missing = doc.read_bits(at_bits, len * 8, &mut buf);
    if !missing.is_empty() {
        return Err(EvalError::Pending(missing));
    }
    Ok(buf)
}

/// The links a symbol table message leads to: every entry of every node of the
/// b-tree under the heap it names.
fn symbol_table_links<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    body: &[usize],
) -> R<Vec<(String, Vec<usize>)>> {
    let mut out = Vec::new();
    let Some(heap) = ev.child_named(doc, body, "heap")? else { return Ok(out) };
    let Some(heap) = inside(ev, doc, &heap)? else { return Ok(out) };
    let Some(tree) = ev.child_named(doc, &heap, "tree")? else { return Ok(out) };
    let Some(tree) = inside(ev, doc, &tree)? else { return Ok(out) };
    nodes(ev, doc, &tree, &mut out, 0)?;
    Ok(out)
}

/// One node of a group's b-tree, and the nodes under it.
fn nodes<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    node: &[usize],
    out: &mut Vec<(String, Vec<usize>)>,
    depth: usize,
) -> R<()> {
    if depth > MAX_DEPTH || out.len() > LIMIT * 4 {
        return Ok(());
    }
    // A leaf holds the links themselves.
    if let Some(symbols) = ev.child_named(doc, node, "symbols")? {
        let n = ev.node(doc, &symbols)?.child_count;
        for i in 0..n {
            let mut link = symbols.clone();
            link.push(i as usize);
            let Some(name) = ev.child_named(doc, &link, "name")? else { continue };
            let Some(name) = inside(ev, doc, &name)? else { continue };
            let Value::Str(name) = ev.node(doc, &name)?.value else { continue };
            let Some(object) = ev.child_named(doc, &link, "object")? else { continue };
            let Some(header) = inside(ev, doc, &object)? else { continue };
            out.push((name, header));
        }
        return Ok(());
    }
    let Some(entries) = ev.child_named(doc, node, "entries")? else { return Ok(()) };
    let n = ev.node(doc, &entries)?.child_count;
    for i in 0..n {
        let mut entry = entries.clone();
        entry.push(i as usize);
        let Some(child) = ev.child_named(doc, &entry, "child")? else { continue };
        let Some(child) = inside(ev, doc, &child)? else { continue };
        match nodes(ev, doc, &child, out, depth + 1) {
            Ok(()) => {}
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => continue,
        }
    }
    Ok(())
}

/// The links in a group's fractal heap: every one in the root block, or in
/// each of the blocks the root block's table points at.
fn heap_links<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    body: &[usize],
) -> R<Vec<(String, Vec<usize>)>> {
    let mut out = Vec::new();
    let Some(heap) = ev.child_named(doc, body, "heap")? else { return Ok(out) };
    let Some(heap) = inside(ev, doc, &heap)? else { return Ok(out) };
    let Some(root) = ev.child_named(doc, &heap, "root_block")? else { return Ok(out) };
    let Some(root) = inside(ev, doc, &root)? else { return Ok(out) };
    // The root is either the objects themselves or the table saying where the
    // blocks holding them are.
    if let Some(children) = ev.child_named(doc, &root, "children")? {
        let n = ev.node(doc, &children)?.child_count;
        for i in 0..n.min(LIMIT as u64 * 4) {
            let mut entry = children.clone();
            entry.push(i as usize);
            let Some(block) = ev.child_named(doc, &entry, "block")? else { continue };
            let Some(block) = inside(ev, doc, &block)? else { continue };
            match block_links(ev, doc, &block, &mut out) {
                Ok(()) => {}
                Err(e) if e.interrupted() => return Err(e),
                Err(_) => continue,
            }
        }
        return Ok(out);
    }
    block_links(ev, doc, &root, &mut out)?;
    Ok(out)
}

/// The links in one block of a heap. What is not a link is not one: free
/// space keeps its own name and is stepped over.
fn block_links<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    block: &[usize],
    out: &mut Vec<(String, Vec<usize>)>,
) -> R<()> {
    let Some(links) = ev.child_named(doc, block, "links")? else { return Ok(()) };
    let n = ev.node(doc, &links)?.child_count;
    for i in 0..n {
        let mut link = links.clone();
        link.push(i as usize);
        match link_message(ev, doc, &link) {
            Ok(Some(found)) => out.push(found),
            Ok(None) => {}
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => continue,
        }
    }
    Ok(())
}

/// A link message, which is how a group written by a newer library names what
/// is in it.
fn link_message<S: Source>(
    ev: &mut Evaluator,
    doc: &Document<S>,
    body: &[usize],
) -> R<Option<(String, Vec<usize>)>> {
    let Some(name) = ev.child_named(doc, body, "name")? else { return Ok(None) };
    let Value::Str(name) = ev.node(doc, &name)?.value else { return Ok(None) };
    let Some(target) = ev.child_named(doc, body, "target")? else { return Ok(None) };
    let Some(object) = ev.child_named(doc, &target, "object")? else { return Ok(None) };
    let Some(header) = inside(ev, doc, &object)? else { return Ok(None) };
    Ok(Some((name, header)))
}
