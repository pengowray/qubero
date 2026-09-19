//! A `torch.save` checkpoint in the format before torch 1.6, which is one file
//! rather than an archive.
//!
//! Five pickles one after another and then the numbers: the magic number, the
//! protocol version, a small dictionary saying how the machine that saved it
//! was built, the data pickle, and the list of storage keys. After them, for
//! each key in that list, an eight-byte element count and that many elements
//! of the storage's own type.
//!
//! Nothing in the file says where one pickle ends and the next begins: a
//! pickle ends at its STOP and the only way to find one is to walk the
//! opcodes. So the template is one [`Ty::Schema`](crate::template::Ty::Schema)
//! node whose builder reads the bytes and works the five boundaries out, the
//! way `torchzip.rs` works out where the pickle of an archive sits. What it
//! places is ordinary fields: each pickle at its own bytes, and each storage
//! as a count and a typed run of numbers, so a tensor's values have a place in
//! the file and the hex view can go there.
//!
//! The element *width* is the other thing the file does not state. A storage
//! says how many elements it holds and not how wide one is, and what says that
//! is the storage class in the persistent id of whichever tensor in the data
//! pickle names this key. So the builder reads that pickle too, with the same
//! Familiar Pickle Form the archive's `data.pkl` is read with, and nothing is
//! run to do it.

use std::sync::Arc;

use crate::eval::{Descriptions, R};
use crate::formats::pickle::familiar::{self, Kind, TensorType, Value};
use crate::template::{Built, Endian, Expr as E, KeyPart, KeyValue, SchemaBuilder, Template, Ty as T};

/// The schema kind of the checkpoint node.
const LEGACY: &str = "torch legacy checkpoint";

/// The first pickle of every legacy file, whole: protocol 2, a `LONG1` of ten
/// bytes, and torch's magic number `0x1950a86a20f9469cfc6c` in two's
/// complement, little-endian, and the STOP after it.
///
/// torch writes this at whatever protocol the caller asked for, and every
/// release writes protocol 2 unless told otherwise. A file saved with
/// `pickle_protocol=4` opens `80 04 95` instead and is not recognised here;
/// no sample in the collection is one.
pub(crate) const MAGIC: &[u8] = b"\x80\x02\x8a\x0a\x6c\xfc\x9c\x46\xf9\x20\x6a\xa8\x50\x19\x2e";

/// How many pickles come before the numbers, and what each of them is.
const PICKLES: [&str; 5] = ["magic", "protocol version", "system info", "data", "storage keys"];
/// Which of them is the data pickle, whose persistent ids say what each
/// storage holds, and which is the list of keys the storages are written in.
const DATA: usize = 3;
const KEYS: usize = 4;
/// How many bytes say how many elements a storage holds.
const COUNT_BYTES: u64 = 8;
/// What the count field and the numbers after it are called.
const COUNT_FIELD: &str = "count";
const NUMBERS_FIELD: &str = "numbers";
/// What the key in the third pickle's dictionary is called, which says which
/// way round the numbers were written.
const ENDIAN_KEY: &str = "little_endian";
/// The most storages the builder will place. Far past any checkpoint saved
/// this way: the format predates torch 1.6 and the models of that era have a
/// few hundred tensors.
const MOST_STORAGES: usize = 1 << 16;
/// How much of the file the builder reads to find the pickles. Everything
/// before the numbers, which is five pickles and the names in them; a
/// checkpoint of gigabytes still has only a few hundred kilobytes of that.
const MOST_HEAD: u64 = 4 << 20;

/// Whether these leading bytes are a legacy checkpoint.
///
/// The first pickle is the same fifteen bytes in every file torch has written
/// this way, which is as strong a signature as any format has.
pub(crate) fn is_torch_legacy(head: &[u8]) -> bool {
    head.starts_with(MAGIC)
}

/// One storage: the key the data pickle named it by, what one of its elements
/// is, and where its count and its numbers sit in the file.
#[derive(Debug, Clone)]
pub(crate) struct Storage {
    pub(crate) key: String,
    pub(crate) dtype: TensorType,
    /// Where the eight bytes saying how many elements it holds sit.
    pub(crate) count_at: u64,
    /// Where the numbers sit, and how many bytes of them there are.
    pub(crate) at: u64,
    pub(crate) len: u64,
}

/// What the file is made of: where each pickle is, which way round the numbers
/// were written, and the storages after them.
#[derive(Debug, Clone)]
pub(crate) struct Layout {
    /// Where each of the five pickles starts and how long it is.
    pub(crate) pickles: Vec<(u64, u64)>,
    pub(crate) endian: Endian,
    pub(crate) storages: Vec<Storage>,
}

/// Where the pickle starting at `at` ends, which is its STOP.
///
/// The opcode walk is the pickle's own: it stops at the first STOP, so what
/// comes back is that pickle and not the one after it. A walk that ran out of
/// opcodes before reaching a STOP is not a pickle at all.
fn pickle_end(bytes: &[u8], at: u64) -> Option<u64> {
    let from = usize::try_from(at).ok()?;
    let ops = crate::formats::pickle::opcodes(bytes.get(from..)?);
    let last = ops.last()?;
    (last.code == b'.').then(|| at + last.end)
}

/// The storage class of every key the data pickle names, and the order the
/// key list puts them in.
fn tensors_by_key(bytes: &[u8], at: u64, len: u64) -> Option<Vec<(String, TensorType)>> {
    let held = window(bytes, at, len)?;
    let found = familiar::recognise(held)?;
    let mut out: Vec<(String, TensorType)> = Vec::new();
    let mut left = vec![&found.value];
    while let Some(value) = left.pop() {
        if let Kind::Tensor(t) = &value.kind {
            let key = std::str::from_utf8(held.get(t.key.0..t.key.0 + t.key.1)?).ok()?;
            if !out.iter().any(|(had, _)| had == key) {
                out.push((key.to_string(), t.dtype));
            }
        }
        push_held(&mut left, value);
    }
    Some(out)
}

/// Every value inside this one, for a walk that is bounded by the tree the
/// form already built rather than by recursion.
fn push_held<'a>(left: &mut Vec<&'a Value>, value: &'a Value) {
    match &value.kind {
        Kind::List(items) | Kind::Tuple(items) | Kind::Set(items) | Kind::FrozenSet(items) | Kind::Objects { items, .. } => left.extend(items),
        Kind::Dict(entries) => left.extend(entries.iter().flat_map(|(k, v)| [k, v])),
        Kind::Instance { state, .. } => left.extend(state.as_deref()),
        Kind::Made { items, state, attrs, .. } => {
            left.extend(items);
            left.extend(state.as_deref());
            left.extend(attrs.as_deref());
        }
        _ => {}
    }
}

/// The keys the fifth pickle lists, in the order the storages are written in,
/// which is what `sorted(serialized_storages.keys())` gave the writer.
fn storage_keys(bytes: &[u8], at: u64, len: u64) -> Option<Vec<String>> {
    let held = window(bytes, at, len)?;
    let found = familiar::recognise(held)?;
    let Kind::List(items) = &found.value.kind else { return None };
    if items.len() > MOST_STORAGES {
        return None;
    }
    items
        .iter()
        .map(|item| match item.kind {
            Kind::Text { at, len } => std::str::from_utf8(held.get(at..at + len).unwrap_or_default()).ok().map(str::to_string),
            _ => None,
        })
        .collect()
}

/// Which way round the numbers were written, which the third pickle says.
/// Nothing when that pickle is not the dictionary torch writes, in which case
/// the reading falls back to the byte order every sample has.
fn declared_endian(bytes: &[u8], at: u64, len: u64) -> Option<Endian> {
    let held = window(bytes, at, len)?;
    let found = familiar::recognise(held)?;
    let Kind::Dict(entries) = &found.value.kind else { return None };
    for (key, value) in entries {
        let Kind::Text { at, len } = key.kind else { continue };
        if held.get(at..at + len)? != ENDIAN_KEY.as_bytes() {
            continue;
        }
        let Kind::Bool(little) = value.kind else { return None };
        return Some(if little { Endian::Little } else { Endian::Big });
    }
    None
}

fn window(bytes: &[u8], at: u64, len: u64) -> Option<&[u8]> {
    let from = usize::try_from(at).ok()?;
    let to = usize::try_from(at.checked_add(len)?).ok()?;
    bytes.get(from..to)
}

/// What the file is made of, worked out from the bytes of its head.
///
/// Nothing at all when the bytes are not a legacy checkpoint, when a pickle
/// does not end, or when a storage the key list names is one the data pickle
/// said nothing about, since there is then no width to read its numbers at.
pub(crate) fn layout(bytes: &[u8], file_len: u64) -> Option<Layout> {
    if !is_torch_legacy(bytes) {
        return None;
    }
    let mut pickles = Vec::new();
    let mut at = 0u64;
    for _ in 0..PICKLES.len() {
        let end = pickle_end(bytes, at)?;
        pickles.push((at, end - at));
        at = end;
    }
    let endian = declared_endian(bytes, pickles[2].0, pickles[2].1).unwrap_or(Endian::Little);
    let dtypes = tensors_by_key(bytes, pickles[DATA].0, pickles[DATA].1)?;
    let keys = storage_keys(bytes, pickles[KEYS].0, pickles[KEYS].1)?;
    let mut storages = Vec::new();
    for key in keys {
        let (_, dtype) = dtypes.iter().find(|(had, _)| *had == key)?;
        let count = u64::from_le_bytes(window(bytes, at, COUNT_BYTES)?.try_into().ok()?);
        let len = count.checked_mul(dtype.width())?;
        let data = at + COUNT_BYTES;
        if data.checked_add(len)? > file_len {
            return None;
        }
        storages.push(Storage { key, dtype: *dtype, count_at: at, at: data, len });
        at = data + len;
    }
    // Every byte of the file is a pickle, a count or a run of numbers. One
    // left over is a file laid out some other way, and reading it as this one
    // would be placing fields at offsets nothing checked.
    (at == file_len).then_some(Layout { pickles, endian, storages })
}

/// A legacy `torch.save` file: the five pickles, and the storages after them.
pub fn torch_legacy() -> Template {
    let root = T::schema(LEGACY, Vec::new(), vec![KeyPart::TextLit("torch".into())]);
    Template::new("torchlegacy", root).with_schema(LEGACY, Arc::new(Checkpoint))
}

/// The builder for [`LEGACY`].
#[derive(Debug)]
struct Checkpoint;

impl SchemaBuilder for Checkpoint {
    fn build(&self, _key: &[KeyValue], table: &mut dyn Descriptions) -> R<Built> {
        let head = table.bytes(0, MOST_HEAD)?;
        let Some(found) = layout(&head, table.file_len()) else {
            let ty = T::structure(NAME, Vec::new()).doc(NOT_LEGACY);
            return Ok(Built { ty, from: None, members_from: Vec::new() });
        };
        let mut fields: Vec<(String, T)> = Vec::new();
        for (name, (at, len)) in PICKLES.iter().zip(&found.pickles) {
            fields.push(((*name).to_string(), T::at(E::lit(*at as i128), T::sized(E::lit(*len as i128), T::pickle()))));
        }
        for storage in &found.storages {
            let elem = crate::eval::element_ty(storage.dtype, found.endian);
            let count = E::lit((storage.len / storage.dtype.width()) as i128);
            let held = T::structure(
                STORAGE_NAME,
                vec![
                    (COUNT_FIELD, T::at(E::lit(storage.count_at as i128), T::Int { bits: 64, endian: found.endian })),
                    (NUMBERS_FIELD, T::at(E::lit(storage.at as i128), T::array(elem, count))),
                ],
            );
            fields.push((storage.key.clone(), held));
        }
        let named: Vec<(&str, T)> = fields.iter().map(|(n, t)| (n.as_str(), t.clone())).collect();
        Ok(Built { ty: T::structure(NAME, named), from: None, members_from: Vec::new() })
    }

    fn key_text(&self, _key: &[KeyValue]) -> String {
        "PyTorch checkpoint (legacy)".to_string()
    }
}

/// What the node is called, and what it says when the bytes are not laid out
/// the way this format is.
const NAME: &str = "TorchLegacy";
const STORAGE_NAME: &str = "storage";
const NOT_LEGACY: &str =
    "Not laid out as a legacy checkpoint: this file opens with torch's magic number, but its five pickles and the storages after them do not account for every byte. Open it as a Python pickle to see what it does hold.";
