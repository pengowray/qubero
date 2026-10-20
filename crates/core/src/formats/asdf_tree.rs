//! What an ASDF tree says about the file's blocks.
//!
//! A block is bytes and a length. What the bytes are is written in the YAML
//! tree at the front of the file, as an `ndarray` node anywhere in it: which
//! block it reads (`source`), what one element is (`datatype`), in which byte
//! order, what `shape` the elements make, and, for a view, where in the block
//! it starts and how far apart its elements are. So the tree is read here, and
//! each block is typed by the nodes that name it.
//!
//! Only the event stream is walked. The tree is YAML 1.1 with tags of the
//! standard's own (`!core/ndarray-1.1.0` under a `%TAG ! tag:stsci.edu:asdf/`
//! directive), and the tag is what marks a node as an array: a mapping that
//! happens to have a `source` key is not one.

use std::sync::{Arc, Mutex};

use saphyr_parser::{Event, Parser};

use crate::eval::{Descriptions, R};
use crate::template::{Built, Encoding, Endian, Expr as E, KeyValue, SchemaBuilder, StrLen, Ty as T, Until};

/// The schema kind of a block's contents.
pub(super) const KIND: &str = "asdf block";

/// A node of the tree. Only a mapping keeps its tag, because the tag on a
/// mapping is what marks an ndarray.
#[derive(Debug, Clone)]
enum Node {
    Scalar(String),
    Seq(Vec<Node>),
    Map(Option<String>, Vec<(String, Node)>),
}

impl Node {
    fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(_, entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Node::Scalar(s) => Some(s),
            _ => None,
        }
    }

    fn int(&self) -> Option<i128> {
        self.text()?.trim().parse().ok()
    }
}

/// The tree as nodes. A document that will not parse is no tree, and every
/// block of it reads as bytes.
fn parse(text: &str) -> Option<Node> {
    enum Frame {
        Seq(usize, Vec<Node>),
        Map(usize, Option<String>, Vec<(String, Node)>, Option<String>),
    }
    let mut stack: Vec<Frame> = Vec::new();
    let mut anchors: Vec<(usize, Node)> = Vec::new();
    let mut root = None;
    let tag_of = |t: Option<std::borrow::Cow<'_, saphyr_parser::Tag>>| t.map(|t| format!("{}{}", t.handle, t.suffix));
    for event in Parser::new_from_str(text) {
        let (event, _) = event.ok()?;
        let (node, anchor) = match event {
            Event::Scalar(value, _, anchor, _) => (Node::Scalar(value.into_owned()), anchor),
            Event::Alias(id) => (anchors.iter().rev().find(|(a, _)| *a == id)?.1.clone(), 0),
            Event::SequenceStart(anchor, _) => {
                stack.push(Frame::Seq(anchor, Vec::new()));
                continue;
            }
            Event::MappingStart(anchor, tag) => {
                stack.push(Frame::Map(anchor, tag_of(tag), Vec::new(), None));
                continue;
            }
            Event::SequenceEnd | Event::MappingEnd => match stack.pop()? {
                Frame::Seq(anchor, items) => (Node::Seq(items), anchor),
                Frame::Map(anchor, tag, entries, _) => (Node::Map(tag, entries), anchor),
            },
            // The first document is the tree. A block index is a second
            // document, but it is never handed in with the tree.
            Event::DocumentEnd => break,
            _ => continue,
        };
        if anchor != 0 {
            anchors.push((anchor, node.clone()));
        }
        match stack.last_mut() {
            None => root = Some(node),
            Some(Frame::Seq(_, items)) => items.push(node),
            Some(Frame::Map(_, _, entries, pending)) => match pending.take() {
                Some(key) => entries.push((key, node)),
                // A key that is a list or a mapping is named `?`. The
                // standard's own keys are always plain text.
                None => *pending = Some(node.text().unwrap_or("?").to_string()),
            },
        }
    }
    root
}

/// Where an array's elements are.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Source {
    /// The block of this number, counted from the first in the file.
    Block(i128),
    /// The elements are written in the tree itself, under `data`.
    Inline,
    /// Another file, named by a URI. Nothing in this file holds them.
    External(String),
}

/// What one element of an array is.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Datatype {
    /// One of the standard's numeric names: `float32`, `uint16`, `bool8`.
    Scalar(String),
    /// Text of a fixed width: `[ascii, 6]` is six bytes, `[ucs4, 5]` five
    /// characters of four bytes each.
    Text { ucs4: bool, len: u64 },
    /// A record of named fields, each with a type and byte order of its own.
    Record(Vec<RecordField>),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct RecordField {
    pub(super) name: String,
    pub(super) datatype: Datatype,
    pub(super) big: Option<bool>,
    pub(super) shape: Vec<u64>,
}

/// One dimension of an array's shape. A streamed block's first is `*`: as many
/// rows as the end of the file leaves room for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Dim {
    Len(u64),
    Streamed,
}

/// One `ndarray` node of the tree.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct NdArray {
    /// Where the node is in the tree: keys joined with dots, and a list's
    /// element as `[i]`. `roman.data` is the science image of a Roman file.
    pub(super) path: String,
    pub(super) source: Source,
    pub(super) datatype: Datatype,
    pub(super) big: bool,
    pub(super) shape: Vec<Dim>,
    /// Bytes into the block the first element starts at.
    pub(super) offset: u64,
    /// Bytes from one element to the next along each dimension, when the node
    /// says. Without them the elements are packed, last dimension fastest.
    pub(super) strides: Option<Vec<i128>>,
}

fn datatype(node: &Node) -> Option<Datatype> {
    match node {
        Node::Scalar(name) => Some(Datatype::Scalar(name.clone())),
        Node::Seq(items) => match items.as_slice() {
            [Node::Scalar(kind), Node::Scalar(len)] if kind == "ascii" || kind == "ucs4" => {
                Some(Datatype::Text { ucs4: kind == "ucs4", len: len.parse().ok()? })
            }
            fields => fields
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    Some(RecordField {
                        name: f.get("name").and_then(Node::text).map_or_else(|| format!("f{i}"), str::to_string),
                        datatype: datatype(f.get("datatype")?)?,
                        big: f.get("byteorder").and_then(Node::text).map(|o| o == "big"),
                        shape: match f.get("shape") {
                            Some(Node::Seq(dims)) => dims.iter().map(|d| d.int().map(|n| n as u64)).collect::<Option<_>>()?,
                            _ => Vec::new(),
                        },
                    })
                })
                .collect::<Option<Vec<_>>>()
                .map(Datatype::Record),
        },
        Node::Map(..) => None,
    }
}

fn ndarray(path: &str, node: &Node) -> Option<NdArray> {
    let source = match (node.get("source"), node.get("data")) {
        (Some(s), _) => match s.int() {
            Some(n) => Source::Block(n),
            None => Source::External(s.text()?.to_string()),
        },
        (None, Some(_)) => Source::Inline,
        (None, None) => return None,
    };
    let shape = match node.get("shape") {
        Some(Node::Seq(dims)) => dims
            .iter()
            .map(|d| match d.text() {
                Some("*") => Some(Dim::Streamed),
                _ => d.int().map(|n| Dim::Len(n as u64)),
            })
            .collect::<Option<Vec<_>>>()?,
        _ => Vec::new(),
    };
    let strides = match node.get("strides") {
        Some(Node::Seq(s)) => Some(s.iter().map(Node::int).collect::<Option<Vec<_>>>()?),
        _ => None,
    };
    Some(NdArray {
        path: path.to_string(),
        source,
        datatype: datatype(node.get("datatype")?)?,
        big: node.get("byteorder").and_then(Node::text) == Some("big"),
        shape,
        offset: node.get("offset").and_then(Node::int).unwrap_or(0) as u64,
        strides,
    })
}

fn is_ndarray(tag: &Option<String>) -> bool {
    tag.as_deref().is_some_and(|t| t.contains("core/ndarray-"))
}

fn collect(path: &str, node: &Node, out: &mut Vec<NdArray>) {
    let join = |key: &str| if path.is_empty() { key.to_string() } else { format!("{path}.{key}") };
    match node {
        Node::Map(tag, _) if is_ndarray(tag) => out.extend(ndarray(path, node)),
        Node::Map(_, entries) => entries.iter().for_each(|(k, v)| collect(&join(k), v, out)),
        Node::Seq(items) => items.iter().enumerate().for_each(|(i, v)| collect(&format!("{path}[{i}]"), v, out)),
        Node::Scalar(_) => {}
    }
}

/// Every `ndarray` node of the tree, in the order the tree writes them.
pub(super) fn ndarrays(tree: &str) -> Vec<NdArray> {
    let mut out = Vec::new();
    if let Some(root) = parse(tree) {
        collect("", &root, &mut out);
    }
    out
}

/// One element of `datatype`, and how many bytes it takes.
fn element(dt: &Datatype, big: bool) -> Option<(T, u64)> {
    let e = if big { Endian::Big } else { Endian::Little };
    let int = |bits: u32| T::Int { bits, endian: e };
    let uint = |bits: u32| T::UInt { bits, endian: e };
    let complex = |part: T| T::inline_structure("Complex", vec![("re", part.clone()), ("im", part)]);
    Some(match dt {
        Datatype::Scalar(name) => match name.as_str() {
            "int8" => (int(8), 1),
            "uint8" => (uint(8), 1),
            // A byte holding 0 or 1.
            "bool8" => (uint(8), 1),
            "int16" => (int(16), 2),
            "uint16" => (uint(16), 2),
            "int32" => (int(32), 4),
            "uint32" => (uint(32), 4),
            "int64" => (int(64), 8),
            "uint64" => (uint(64), 8),
            "float16" => (T::F16(e), 2),
            "float32" => (T::F32(e), 4),
            "float64" => (T::F64(e), 8),
            "complex64" => (complex(T::F32(e)), 8),
            "complex128" => (complex(T::F64(e)), 16),
            _ => return None,
        },
        Datatype::Text { ucs4: false, len } => {
            (T::text(StrLen::Padded { size: E::lit(*len as i128), pad: 0 }, Encoding::Ascii), *len)
        }
        // Four bytes a character, read as the code points they are, the way
        // numpy's `U` is read in an NPY file.
        Datatype::Text { ucs4: true, len } => (T::array(uint(32), E::lit(*len as i128)), len * 4),
        Datatype::Record(fields) => {
            let mut size = 0;
            let mut out = Vec::new();
            for f in fields {
                let (mut ty, mut width) = element(&f.datatype, f.big.unwrap_or(big))?;
                for &n in f.shape.iter().rev() {
                    ty = T::array(ty, E::lit(n as i128));
                    width *= n;
                }
                size += width;
                out.push((f.name.clone(), ty));
            }
            let fields = out.iter().map(|(n, t)| (n.as_str(), t.clone())).collect();
            (T::structure("Record", fields), size)
        }
    })
}

/// `count` elements of `inner`, each `stride` bytes on from the one before.
/// Packed elements are an array; spaced ones carry the bytes between them,
/// except the last, which has nothing after it that is still this array's.
fn spaced(inner: T, width: u64, count: u64, stride: i128) -> Option<T> {
    let n = E::lit(count as i128);
    if stride == width as i128 || count <= 1 {
        return Some(T::array(inner, n));
    }
    // A stride shorter than an element, or one that walks backwards, reads
    // the same bytes twice or in an order no list can be placed in.
    let gap = u64::try_from(stride).ok()?.checked_sub(width).filter(|g| *g > 0)?;
    let elem = T::structure(
        "Strided",
        vec![("value", inner), ("skipped", T::when(E::idx().add(E::lit(1)).less_than(n.clone()), T::bytes(E::lit(gap as i128))))],
    )
    .contents("value");
    Some(T::array(elem, n))
}

/// The array an `ndarray` node reads from its block, or nothing when the
/// node says something this cannot place.
pub(super) fn array_type(a: &NdArray) -> Option<T> {
    let (elem, width) = element(&a.datatype, a.big)?;
    let mut ty = elem;
    let mut size = width;
    // Innermost dimension first, each wrapping the one inside it.
    for (i, dim) in a.shape.iter().enumerate().rev() {
        match *dim {
            Dim::Len(n) => {
                let stride = a.strides.as_ref().and_then(|s| s.get(i).copied()).unwrap_or(size as i128);
                let step = u64::try_from(stride).unwrap_or(size);
                ty = spaced(ty, size, n, stride)?;
                size = if n == 0 { 0 } else { step * (n - 1) + size };
            }
            // Rows to the end of the block, and only as the first dimension.
            Dim::Streamed if i == 0 => ty = T::repeat(ty, Until::End),
            Dim::Streamed => return None,
        }
    }
    if a.offset > 0 {
        ty = T::structure("View", vec![("before", T::bytes(E::lit(a.offset as i128))), ("view", ty)]).contents("view");
    }
    Some(ty)
}

/// What a block holds: each array the tree reads from it, all starting where
/// the block does, named by where they are in the tree.
///
/// A streamed block is the last in the file, and the tree names it `-1`, which
/// counts from the end; a block that is not streamed is named by its number.
pub(super) fn block_type(arrays: &[NdArray], block: i128, streamed: bool) -> T {
    let mine: Vec<(&str, T)> = arrays
        .iter()
        .filter(|a| a.source == Source::Block(block) || (streamed && a.source == Source::Block(-1)))
        .map(|a| (a.path.as_str(), array_type(a).unwrap_or_else(|| T::bytes(E::Remaining))))
        .collect();
    if mine.is_empty() {
        return T::bytes(E::Remaining);
    }
    T::union_structure("Arrays", mine)
}

/// The builder for [`KIND`]: the tree is the one record its walk reaches, and
/// the key is the block's number and whether it is streamed.
#[derive(Debug, Default)]
pub(super) struct Blocks {
    /// The last tree parsed and what was in it. Every block asks, and a Roman
    /// file has twenty-seven of them over one 50 KB tree.
    last: Mutex<Option<(String, Arc<Vec<NdArray>>)>>,
}

impl Blocks {
    fn arrays(&self, tree: String) -> Arc<Vec<NdArray>> {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, arrays)) = last.as_ref().filter(|(text, _)| *text == tree) {
            return arrays.clone();
        }
        let arrays = Arc::new(ndarrays(&tree));
        *last = Some((tree, arrays.clone()));
        arrays
    }
}

impl SchemaBuilder for Blocks {
    fn build(&self, key: &[KeyValue], table: &mut dyn Descriptions) -> R<Built> {
        let [KeyValue::Int(block), KeyValue::Int(streamed)] = key else {
            return Ok(Built::by_heart(T::bytes(E::Remaining)));
        };
        let Some(tree) = table.records()?.into_iter().next() else {
            return Ok(Built::by_heart(T::bytes(E::Remaining)));
        };
        // The tree's bytes and not its text: a field's text is cut at 4 KB,
        // which is what a reader retypes, and a Roman tree is 50 KB.
        let node = table.node(&tree)?;
        let bytes = table.bytes(node.offset_bits / 8, node.size_bits / 8)?;
        let arrays = self.arrays(String::from_utf8_lossy(&bytes).into_owned());
        Ok(Built { ty: block_type(&arrays, *block, *streamed != 0), from: Some(tree), members_from: Vec::new() })
    }

    fn key_text(&self, key: &[KeyValue]) -> String {
        match key {
            [KeyValue::Int(n), ..] => format!("block {n}"),
            _ => "block".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = "%YAML 1.1\n%TAG ! tag:stsci.edu:asdf/\n--- !core/asdf-1.1.0\n\
        roman: !<asdf://stsci.edu/datamodels/roman/tags/wfi_image-1.7.0>\n\
        \x20 data: !core/ndarray-1.1.0\n    source: 12\n    datatype: float32\n    byteorder: little\n    shape: [4088, 4088]\n\
        \x20 steps:\n  - &a !core/ndarray-1.1.0 {source: 0, datatype: int16, byteorder: big, shape: [2]}\n  - *a\n\
        \x20 note: {source: 3}\n\
        inline: !core/ndarray-1.1.0 {data: [1, 2], datatype: int8, shape: [2]}\n\
        rows: !core/ndarray-1.1.0\n  source: -1\n  datatype:\n  - {name: id, datatype: uint32}\n  - {name: name, datatype: [ascii, 4], byteorder: big}\n  byteorder: little\n  shape: ['*']\n\
        ...\n";

    #[test]
    fn every_ndarray_is_found_by_its_tag_and_named_by_its_path() {
        let found = ndarrays(TREE);
        let paths: Vec<_> = found.iter().map(|a| a.path.as_str()).collect();
        // `note` has a `source` but no tag, so it is not an array.
        assert_eq!(paths, ["roman.data", "roman.steps[0]", "roman.steps[1]", "inline", "rows"]);
        let data = &found[0];
        assert_eq!(data.source, Source::Block(12));
        assert_eq!(data.datatype, Datatype::Scalar("float32".into()));
        assert_eq!((data.big, data.shape.clone()), (false, vec![Dim::Len(4088), Dim::Len(4088)]));
        // An alias is the node its anchor named.
        assert_eq!(found[2].source, Source::Block(0));
        assert!(found[2].big);
        assert_eq!(found[3].source, Source::Inline);
    }

    #[test]
    fn a_record_datatype_keeps_its_fields_and_a_streamed_shape_its_star() {
        let rows = ndarrays(TREE).pop().unwrap();
        assert_eq!(rows.source, Source::Block(-1));
        assert_eq!(rows.shape, vec![Dim::Streamed]);
        let Datatype::Record(fields) = &rows.datatype else { panic!("{:?}", rows.datatype) };
        assert_eq!(fields[0].name, "id");
        assert_eq!(fields[1].datatype, Datatype::Text { ucs4: false, len: 4 });
        assert_eq!(fields[1].big, Some(true));
    }

    #[test]
    fn a_tree_that_will_not_parse_has_no_arrays() {
        assert!(ndarrays("--- {a: [1, 2\n...\n").is_empty());
    }
}
