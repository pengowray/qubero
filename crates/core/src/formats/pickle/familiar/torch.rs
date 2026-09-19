//! The torch productions: a tensor, and the parameter that wraps one.
//!
//! `torch.save` keeps a tensor's numbers out of the pickle. What the pickle
//! holds is a call to one rebuilding function with a persistent id in its
//! arguments, and the persistent id names a storage the reader is to fetch
//! from somewhere else: an entry of the ZIP in a file written since torch 1.6,
//! a run further down the file in a legacy one.
//!
//! A persistent id is safe to name because nothing is fetched and nothing is
//! run. `BINPERSID` in a general pickle machine hands the tuple to the
//! loader's `persistent_load`, which is a function of the loading program; the
//! reader here has no such function. The tuple is read as the five or six
//! fixed parts it is, every part checked, and what comes out is a note saying
//! where the numbers are. The one opcode is folded away inside this run, so a
//! `BINPERSID` anywhere else, or over any other tuple, is a non-match.
//!
//! The call is fixed instruction for instruction:
//!
//! ```text
//! GLOBAL torch._utils _rebuild_tensor_v2
//! MARK
//!   MARK 'storage' GLOBAL torch FloatStorage '0' 'cpu' 12 TUPLE BINPERSID
//!   0              storage offset, in elements
//!   (3, 4)         size
//!   (4, 1)         stride, in elements
//!   False          requires_grad
//!   OrderedDict()  backward hooks, always empty in a saved file
//! TUPLE REDUCE
//! ```

use super::cursor::Cursor;
use super::memo::Bound;
use super::packs::Pack;
use super::{Kind, Names, Shape, Tensor, TensorType, Value};

/// The module the two rebuilding functions are in.
const UTILS: &[&str] = &["torch._utils"];
/// The one call that rebuilds a tensor, and the one that wraps a tensor in the
/// parameter a module's weights are.
///
/// `_rebuild_tensor` (no `_v2`), `_rebuild_tensor_v3`, the sparse, nested,
/// meta and device rebuilders and `_rebuild_wrapper_subclass` are all real
/// calls torch writes, and none is here: no file in the collection holds one,
/// and a call this reader has not measured is a non-match rather than a guess.
const REBUILD_TENSOR: &str = "_rebuild_tensor_v2";
const REBUILD_PARAMETER: &str = "_rebuild_parameter";

/// The word the persistent id opens with, which is what says the tuple is a
/// storage rather than the `('module', ...)` a legacy file writes for a class
/// whose source it saved.
const STORAGE_WORD: &str = "storage";

/// The class an empty dictionary of backward hooks is, in the spellings the
/// accelerator module goes by.
const COLLECTIONS: &[&str] = &["collections", "_collections"];
const ORDERED_DICT: &str = "OrderedDict";

/// One storage class, and what one of its elements is.
///
/// Every class here is one a sample in the collection names. The ones left out
/// on purpose: `torch.storage.UntypedStorage`, which arrives with
/// `_rebuild_tensor_v3` and carries its dtype as a separate argument;
/// `ComplexFloatStorage` and `ComplexDoubleStorage`; and the quantised
/// storages. Each would be a row here and a sample beside it.
pub struct Storage {
    pub class: &'static str,
    pub dtype: TensorType,
}

/// What a persistent id may name, by the class the file spells after `torch`.
pub const STORAGES: &[Storage] = &[
    Storage { class: "FloatStorage", dtype: TensorType::Float32 },
    Storage { class: "DoubleStorage", dtype: TensorType::Float64 },
    Storage { class: "HalfStorage", dtype: TensorType::Float16 },
    Storage { class: "BFloat16Storage", dtype: TensorType::BFloat16 },
    Storage { class: "LongStorage", dtype: TensorType::Int64 },
    Storage { class: "IntStorage", dtype: TensorType::Int32 },
    Storage { class: "ShortStorage", dtype: TensorType::Int16 },
    Storage { class: "CharStorage", dtype: TensorType::Int8 },
    Storage { class: "ByteStorage", dtype: TensorType::UInt8 },
    Storage { class: "BoolStorage", dtype: TensorType::Bool },
];

/// The module every one of them is named from.
const TORCH: &[&str] = &["torch"];

/// The most dimensions a tensor may declare. Torch's own limit is 64 for most
/// operations; the shape is checked against the storage either way, so this
/// only bounds the work of reading one.
const MAX_DIMENSIONS: usize = 64;

impl Cursor<'_> {
    /// A tensor, or the parameter that wraps one. Both begin by naming a
    /// callable in `torch._utils`, so either fails at its first word when the
    /// value in hand is something else.
    pub(super) fn torch_value(&mut self) -> Option<Value> {
        let start = self.at;
        for production in [Cursor::rebuilt_tensor, Cursor::rebuilt_parameter] {
            let here = self.save();
            match production(self, start) {
                Some(value) => return Some(value),
                None => self.restore(here),
            }
        }
        None
    }

    /// `torch._utils._rebuild_tensor_v2(persistent id, offset, size, stride,
    /// requires_grad, OrderedDict())`.
    fn rebuilt_tensor(&mut self, start: usize) -> Option<Value> {
        self.global(UTILS, REBUILD_TENSOR, "rebuild module", "rebuild call")?;
        self.atoms(&[b"("])?;
        let (storage_class, dtype, key, location, count) = self.persistent_id()?;
        let offset = self.elements()?;
        let size = self.extents()?;
        let stride = self.extents()?;
        if size.len() != stride.len() || size.len() > MAX_DIMENSIONS {
            return None;
        }
        let requires_grad = self.read_flag()?;
        self.empty_hooks()?;
        self.exact(b"t")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        self.memoize(Bound::Made { what: Shape::Tensor, at: start, hashable: false })?;
        let tensor = Tensor { dtype, storage_class, key, location, count, offset, size, stride, requires_grad, parameter: false };
        // A view has to fit in the storage the persistent id named. A tensor
        // reaching past the end of its own storage is not something torch
        // wrote, and reading one would read whatever is next in the file.
        if tensor.reach()? > count {
            return None;
        }
        self.finish_call("tensor rebuild call", start, self.at);
        self.tensors += 1;
        self.packs.add(Pack::Torch);
        Some(self.span(start, Kind::Tensor(tensor)))
    }

    /// `torch._utils._rebuild_parameter(tensor, requires_grad,
    /// OrderedDict())`, which is what a module's weights are wrapped in.
    ///
    /// What comes out is the tensor it wraps, with a note that it was one: a
    /// parameter is a tensor that gradients are kept for, and the rows a
    /// reader wants are the tensor's. The `requires_grad` the parameter
    /// carries is the one that counts, so it replaces the tensor's.
    fn rebuilt_parameter(&mut self, start: usize) -> Option<Value> {
        self.global(UTILS, REBUILD_PARAMETER, "rebuild module", "rebuild call")?;
        // Three arguments, so the tuple closes with TUPLE3 and opens with no
        // MARK at all. The tensor's own six need a MARK, which is why the two
        // calls are written differently.
        self.open_tuple()?;
        let inner = self.at;
        let held = self.rebuilt_tensor(inner)?;
        let requires_grad = self.read_flag()?;
        self.empty_hooks()?;
        self.close_tuple(3)?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        self.memoize(Bound::Made { what: Shape::Tensor, at: start, hashable: false })?;
        let Kind::Tensor(tensor) = held.kind else { return None };
        let tensor = Tensor { requires_grad, parameter: true, ..tensor };
        // The tensor inside closed a run of its own. The parameter is one act
        // with it, so the two runs become one and the names both matched are
        // under the parameter.
        self.join_calls("parameter rebuild call", start);
        Some(self.span(start, Kind::Tensor(tensor)))
    }

    /// The persistent id, which is the whole of what the pickle says about
    /// where the numbers are: the word `storage`, the storage class, the key
    /// that names the run, the device it was on, how many elements it holds,
    /// and, in a legacy file, a sixth part that is always `None`.
    #[allow(clippy::type_complexity)]
    fn persistent_id(&mut self) -> Option<(&'static str, TensorType, (usize, usize), (usize, usize), u64)> {
        self.atoms(&[b"("])?;
        if let Some((at, len)) = self.word_or_reference(STORAGE_WORD)? {
            self.says("persistent id kind", at, len);
        }
        let held = self.storage_class()?;
        let key = self.text_run_or_reference("storage key")?;
        let location = self.text_run_or_reference("location")?;
        let count = self.elements()?;
        // The view metadata a legacy file writes as a sixth part. torch has
        // set it to `None` since storage views were dropped, and a tuple there
        // would be a storage placed inside another storage, which nothing has
        // written for years and no sample holds.
        if self.peek() == Some(b'N') {
            self.gate()?;
            self.byte()?;
        }
        self.exact(b"t")?;
        self.memoize(Bound::Opaque)?;
        // BINPERSID takes what the tuple made and pushes what the loader would
        // have fetched. Nothing is fetched here: the tuple is read and folded
        // away, and this opcode is the whole of what it means.
        self.exact(b"Q")?;
        Some((held.class, held.dtype, key, location, count))
    }

    /// One of the enumerated storage classes, named `torch.<class>`.
    fn storage_class(&mut self) -> Option<&'static Storage> {
        for held in STORAGES {
            let here = self.save();
            if self.global(TORCH, held.class, "storage module", "storage class").is_some() {
                return Some(held);
            }
            self.restore(here);
        }
        None
    }

    /// The empty dictionary of backward hooks every saved tensor carries.
    ///
    /// Folded away rather than read as a value: torch writes it for a
    /// compatibility its own comments call a note to itself, it is empty in
    /// every file anything ever saved, and a dictionary with something in it
    /// is hooks the tensor would be given, which is a non-match.
    fn empty_hooks(&mut self) -> Option<()> {
        let at = self.at;
        self.global(COLLECTIONS, ORDERED_DICT, "hooks module", "hooks class")?;
        self.empty_tuple()?;
        self.exact(b"R")?;
        self.memoize(Bound::Made { what: Shape::OrderedDict, at, hashable: false })?;
        Some(())
    }

    /// A nonnegative whole number, in whichever width the protocol wrote it.
    fn elements(&mut self) -> Option<u64> {
        let Kind::Int { value, .. } = self.integer()?.kind else { return None };
        u64::try_from(value).ok()
    }

    /// A size or a stride, which is a tuple of nonnegative whole numbers
    /// written the way every other shape in a pickle is. A tensor with no
    /// dimensions writes the empty tuple for both.
    fn extents(&mut self) -> Option<Vec<u64>> {
        self.dimensions()
    }

    /// A text the file spelled here or named where it spelled it earlier, and
    /// the run it sits in wherever that is.
    ///
    /// A key and a device name are data rather than words a form knows, so
    /// neither can be matched against a spelling. A reference comes back as
    /// the run the text was first written in, so the row over it reads the
    /// same text either way; only a run spelled here is named inside the call,
    /// since a BINGET has no bytes of its own to name.
    fn text_run_or_reference(&mut self, says: &'static str) -> Option<(usize, usize)> {
        self.gate()?;
        if self.at_reference() {
            return match self.reference()? {
                Bound::Text { at, len } => Some((*at, *len)),
                _ => None,
            };
        }
        let value = self.text()?;
        let (Kind::Text { at, len } | Kind::Ref(Names::Text { at, len })) = value.kind else { return None };
        self.says(says, at, len);
        Some((at, len))
    }
}
