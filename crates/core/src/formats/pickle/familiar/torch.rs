//! The torch productions: a tensor, the parameter that wraps one, the
//! quantised tensor whose numbers stand for real ones, and the element types a
//! file names by name.
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
//!
//! `_rebuild_tensor_v3` is the same run with `torch.storage UntypedStorage` in
//! place of the storage class, a count of bytes rather than elements, and a
//! `GLOBAL torch <dtype>` after the hooks. `_rebuild_qtensor` is the same run
//! with `(torch.per_tensor_affine, scale, zero point)` where the flag would
//! be. `torch.Size`, `torch.device` and the sparse tensor are ordinary calls
//! of enumerated callables and are in the table in [`forms`](super::forms).

use super::cursor::Cursor;
use super::forms::{Args, Reduce, Via};
use super::memo::Bound;
use super::packs::Extension;
use super::{Kind, Names, Quantizer, Shape, Tensor, TensorType, Value};

/// The module every rebuilding function is in.
const UTILS: &[&str] = &["torch._utils"];
/// The calls that rebuild a tensor, and the one that wraps a tensor in the
/// parameter a module's weights are.
///
/// `_rebuild_tensor_v2` carries the element type in the storage class it
/// names. `_rebuild_tensor_v3` names `torch.storage.UntypedStorage` instead
/// and hands the dtype over as a seventh argument, which is how torch writes
/// every type it added after the legacy storage classes were frozen: the
/// eight-bit floats and the wide unsigned integers.
///
/// `_rebuild_tensor` (no `_v2`), the nested, meta and device rebuilders and
/// `_rebuild_wrapper_subclass` are all real calls torch writes, and none is
/// here: no file in the collection holds one, and a call this reader has not
/// measured is a non-match rather than a guess.
const REBUILD_TENSOR: &str = "_rebuild_tensor_v2";
const REBUILD_TENSOR_V3: &str = "_rebuild_tensor_v3";
/// `_rebuild_qtensor(storage, offset, size, stride, quantizer, requires_grad,
/// hooks)`, which is the same tensor with the numbers standing for real ones
/// through a scale and a zero point.
const REBUILD_QTENSOR: &str = "_rebuild_qtensor";
const REBUILD_PARAMETER: &str = "_rebuild_parameter";

/// The class torch 0.4 wrote a parameter as, called with the tensor it wraps
/// and the gradient flag. `Parameter.__reduce_ex__` returned the class itself
/// then; torch 1.0 replaced it with `_rebuild_parameter` and a third argument
/// for the hooks.
const PARAMETER_MODULE: &[&str] = &["torch.nn.parameter"];
const PARAMETER: &str = "Parameter";

/// The storage `_rebuild_tensor_v3` names, which says only how many bytes
/// there are: the element type arrives as the call's last argument.
const STORAGE_MODULE: &[&str] = &["torch.storage"];
const UNTYPED_STORAGE: &str = "UntypedStorage";

/// The scheme a quantised tensor names, which is the only one whose arguments
/// this reader has measured. `per_channel_affine` hands over two tensors and
/// an axis instead, and no sample holds one.
const PER_TENSOR_AFFINE: &str = "per_tensor_affine";

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
/// on purpose are the quantised four-bit and two-bit storages, which no sample
/// holds; each would be a row here and a sample beside it.
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
    Storage { class: "ComplexFloatStorage", dtype: TensorType::Complex64 },
    Storage { class: "ComplexDoubleStorage", dtype: TensorType::Complex128 },
    Storage { class: "LongStorage", dtype: TensorType::Int64 },
    Storage { class: "IntStorage", dtype: TensorType::Int32 },
    Storage { class: "ShortStorage", dtype: TensorType::Int16 },
    Storage { class: "CharStorage", dtype: TensorType::Int8 },
    Storage { class: "ByteStorage", dtype: TensorType::UInt8 },
    Storage { class: "BoolStorage", dtype: TensorType::Bool },
    Storage { class: "QInt8Storage", dtype: TensorType::QInt8 },
    Storage { class: "QUInt8Storage", dtype: TensorType::QUInt8 },
    Storage { class: "QInt32Storage", dtype: TensorType::QInt32 },
];

/// Every element type a file may name by its own name, which is what
/// `_rebuild_tensor_v3` is handed and what a `torch.dtype` saved as a value
/// is.
///
/// A dtype is a global the form names and never calls: `torch.float32` is one
/// object rather than a class to construct, and pickle writes it as a GLOBAL.
/// So the list is the whole of what may be named, in torch's own spelling.
/// The aliases (`torch.float`, `torch.long`) are the same objects as the names
/// here and pickle writes the canonical name for each, so naming one is naming
/// a row of this table.
pub const DTYPES: &[Storage] = &[
    Storage { class: "float16", dtype: TensorType::Float16 },
    Storage { class: "bfloat16", dtype: TensorType::BFloat16 },
    Storage { class: "float32", dtype: TensorType::Float32 },
    Storage { class: "float64", dtype: TensorType::Float64 },
    Storage { class: "complex64", dtype: TensorType::Complex64 },
    Storage { class: "complex128", dtype: TensorType::Complex128 },
    Storage { class: "float8_e4m3fn", dtype: TensorType::Float8E4M3FN },
    Storage { class: "float8_e5m2", dtype: TensorType::Float8E5M2 },
    Storage { class: "int8", dtype: TensorType::Int8 },
    Storage { class: "uint8", dtype: TensorType::UInt8 },
    Storage { class: "int16", dtype: TensorType::Int16 },
    Storage { class: "int32", dtype: TensorType::Int32 },
    Storage { class: "int64", dtype: TensorType::Int64 },
    Storage { class: "uint16", dtype: TensorType::UInt16 },
    Storage { class: "uint32", dtype: TensorType::UInt32 },
    Storage { class: "uint64", dtype: TensorType::UInt64 },
    Storage { class: "bool", dtype: TensorType::Bool },
    Storage { class: "qint8", dtype: TensorType::QInt8 },
    Storage { class: "quint8", dtype: TensorType::QUInt8 },
    Storage { class: "qint32", dtype: TensorType::QInt32 },
];

/// The same names as whole dotted paths, which is what a form's `names` column
/// is: the globals it may name and never calls.
///
/// Written out rather than made up from the table above, because a form's
/// tables are what a reader compares against and a const table cannot be
/// built at run time. `a_dtype_is_named_in_both_tables` holds the two to each
/// other.
pub const DTYPE_NAMES: &[&str] = &[
    "torch.float16",
    "torch.bfloat16",
    "torch.float32",
    "torch.float64",
    "torch.complex64",
    "torch.complex128",
    "torch.float8_e4m3fn",
    "torch.float8_e5m2",
    "torch.int8",
    "torch.uint8",
    "torch.int16",
    "torch.int32",
    "torch.int64",
    "torch.uint16",
    "torch.uint32",
    "torch.uint64",
    "torch.bool",
    "torch.qint8",
    "torch.quint8",
    "torch.qint32",
];

/// The module every one of them is named from.
const TORCH: &[&str] = &["torch"];

/// What a persistent id said: which storage class, what one element is where
/// the class says, where the numbers are and how long the storage is.
struct Persistent {
    storage_class: &'static str,
    /// Nothing for an untyped storage, whose element type is the call's last
    /// argument rather than the storage class.
    dtype: Option<TensorType>,
    key: (usize, usize),
    location: (usize, usize),
    count: u64,
    /// Whether `count` is bytes rather than elements, which is what an
    /// untyped storage counts in.
    bytes: bool,
}

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
        for production in [Cursor::rebuilt_tensor, Cursor::rebuilt_qtensor, Cursor::rebuilt_parameter, Cursor::called_parameter] {
            let here = self.save();
            match production(self, start) {
                Some(value) => return Some(value),
                None => self.restore(here),
            }
        }
        None
    }

    /// `torch._utils._rebuild_tensor_v2(persistent id, offset, size, stride,
    /// requires_grad, OrderedDict())`, and the `_v3` that ends with the dtype.
    fn rebuilt_tensor(&mut self, start: usize) -> Option<Value> {
        let third = {
            let here = self.save();
            match self.global(UTILS, REBUILD_TENSOR, "rebuild module", "rebuild call") {
                Some(_) => false,
                None => {
                    self.restore(here);
                    self.global(UTILS, REBUILD_TENSOR_V3, "rebuild module", "rebuild call")?;
                    true
                }
            }
        };
        self.atoms(&[b"("])?;
        let held = self.persistent_id(third)?;
        let offset = self.elements()?;
        let size = self.extents()?;
        let stride = self.extents()?;
        if size.len() != stride.len() || size.len() > MAX_DIMENSIONS {
            return None;
        }
        let requires_grad = self.read_flag()?;
        self.empty_hooks()?;
        // `_rebuild_tensor_v3` names an untyped storage, so the element type
        // comes after the hooks as a dtype of torch's own. `_v2` carries it in
        // the storage class and writes nothing here.
        let dtype = match third {
            true => self.dtype_named()?,
            false => held.dtype?,
        };
        self.exact(b"t")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        self.memoize(Bound::Made { what: Shape::Tensor, at: start, hashable: false })?;
        self.tensor_made(start, held, dtype, offset, size, stride, requires_grad, None)
    }

    /// `torch._utils._rebuild_qtensor(persistent id, offset, size, stride,
    /// (torch.per_tensor_affine, scale, zero point), requires_grad,
    /// OrderedDict())`.
    ///
    /// The numbers stay the whole numbers the file holds. What the scheme says
    /// they stand for is read and shown beside them, because a reader looking
    /// at a quantised checkpoint is looking at the stored integers and would
    /// not be told that the file had been rewritten on the way out.
    fn rebuilt_qtensor(&mut self, start: usize) -> Option<Value> {
        self.global(UTILS, REBUILD_QTENSOR, "rebuild module", "rebuild call")?;
        self.atoms(&[b"("])?;
        let held = self.persistent_id(false)?;
        let offset = self.elements()?;
        let size = self.extents()?;
        let stride = self.extents()?;
        if size.len() != stride.len() || size.len() > MAX_DIMENSIONS {
            return None;
        }
        self.open_tuple()?;
        self.global(TORCH, PER_TENSOR_AFFINE, "scheme module", "scheme")?;
        let Kind::Float { value: scale, .. } = self.binfloat()?.kind else { return None };
        let Kind::Int { value: zero_point, .. } = self.integer()?.kind else { return None };
        self.close_tuple(3)?;
        self.memoize(Bound::Opaque)?;
        let requires_grad = self.read_flag()?;
        self.empty_hooks()?;
        self.exact(b"t")?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        self.memoize(Bound::Made { what: Shape::Tensor, at: start, hashable: false })?;
        let dtype = held.dtype?;
        self.tensor_made(start, held, dtype, offset, size, stride, requires_grad, Some(Quantizer { scale, zero_point }))
    }

    /// The tensor the call just read made, checked against the storage it
    /// named and counted as one of the file's.
    #[allow(clippy::too_many_arguments)]
    fn tensor_made(
        &mut self,
        start: usize,
        held: Persistent,
        dtype: TensorType,
        offset: u64,
        size: Vec<u64>,
        stride: Vec<u64>,
        requires_grad: bool,
        quantizer: Option<Quantizer>,
    ) -> Option<Value> {
        // An untyped storage counts bytes where a typed one counts elements,
        // so the count is put in the same units as everything else here: a
        // length of bytes that is not whole elements is a non-match.
        let count = match held.bytes {
            true => match held.count % dtype.width() {
                0 => held.count / dtype.width(),
                _ => return None,
            },
            false => held.count,
        };
        let Persistent { storage_class, key, location, .. } = held;
        let tensor =
            Tensor { dtype, storage_class, key, location, count, offset, size, stride, requires_grad, parameter: false, quantizer };
        // A view has to fit in the storage the persistent id named. A tensor
        // reaching past the end of its own storage is not something torch
        // wrote, and reading one would read whatever is next in the file.
        if tensor.reach()? > count {
            return None;
        }
        self.finish_call("tensor rebuild call", start, self.at);
        self.tensors += 1;
        self.extensions.add(Extension::Torch);
        Some(self.span(start, Kind::Tensor(tensor)))
    }

    /// One of the enumerated dtypes, named `torch.<name>`: a global the form
    /// names and never calls.
    fn dtype_named(&mut self) -> Option<TensorType> {
        for held in DTYPES {
            let here = self.save();
            if self.global(TORCH, held.class, "dtype module", "dtype").is_some() {
                return Some(held.dtype);
            }
            self.restore(here);
        }
        None
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

    /// `torch.nn.parameter.Parameter(tensor, requires_grad)`, which is how
    /// torch 0.4 wrote a module's weights: the class itself, called with the
    /// tensor it wraps.
    ///
    /// Nothing is constructed. The call is read as the tensor it holds with a
    /// note that it was a parameter, the same reading `_rebuild_parameter`
    /// gets, so a checkpoint from that release opens as the same rows a newer
    /// one does. The class is named here as one enumerated callable with one
    /// argument shape, not as a class under a module prefix.
    fn called_parameter(&mut self, start: usize) -> Option<Value> {
        self.global(PARAMETER_MODULE, PARAMETER, "parameter module", "parameter class")?;
        self.open_tuple()?;
        let inner = self.at;
        let held = self.rebuilt_tensor(inner)?;
        let requires_grad = self.read_flag()?;
        self.close_tuple(2)?;
        self.memoize(Bound::Opaque)?;
        self.exact(b"R")?;
        self.memoize(Bound::Made { what: Shape::Tensor, at: start, hashable: false })?;
        let Kind::Tensor(tensor) = held.kind else { return None };
        let tensor = Tensor { requires_grad, parameter: true, ..tensor };
        // The tensor inside closed a run of its own, and the call around it is
        // one act with it, so the two runs become one.
        self.join_calls("parameter call", start);
        Some(self.span(start, Kind::Tensor(tensor)))
    }

    /// The persistent id, which is the whole of what the pickle says about
    /// where the numbers are: the word `storage`, the storage class, the key
    /// that names the run, the device it was on, how many elements it holds,
    /// and, in a legacy file, a sixth part that is always `None`.
    fn persistent_id(&mut self, untyped: bool) -> Option<Persistent> {
        self.atoms(&[b"("])?;
        if let Some((at, len)) = self.word_or_reference(STORAGE_WORD)? {
            self.says("persistent id kind", at, len);
        }
        let held = match untyped {
            true => {
                self.global(STORAGE_MODULE, UNTYPED_STORAGE, "storage module", "storage class")?;
                &Storage { class: UNTYPED_STORAGE, dtype: TensorType::UInt8 }
            }
            false => self.storage_class()?,
        };
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
        Some(Persistent {
            storage_class: held.class,
            dtype: (!untyped).then_some(held.dtype),
            key,
            location,
            count,
            bytes: untyped,
        })
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

    /// The backward hooks every saved tensor carries, which are nothing in
    /// both the spellings torch has written.
    ///
    /// Folded away rather than read as a value: torch writes it for a
    /// compatibility its own comments call a note to itself, it is empty in
    /// every file anything ever saved, and a dictionary with something in it
    /// is hooks the tensor would be given, which is a non-match.
    ///
    /// torch 0.4 handed `Tensor.__reduce_ex__` the tensor's own
    /// `_backward_hooks`, which is `None` until a hook is registered, so every
    /// file that release wrote has a `None` here. The note "Don't serialize
    /// hooks" in torch 1.0's `torch/tensor.py` is where it became an empty
    /// `OrderedDict()` instead.
    fn empty_hooks(&mut self) -> Option<()> {
        let at = self.at;
        self.gate()?;
        if self.peek() == Some(b'N') {
            self.byte()?;
            return Some(());
        }
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

/// The calls a torch file writes that are not the tensor production's own
/// fixed run.
///
/// A state dict is a `collections.OrderedDict`, which the standard library's
/// table already describes, so the row is shared rather than copied. The rest
/// are torch's own values: the shape of a tensor written on its own, the
/// device a tensor was on, and the sparse tensor that is two tensors and the
/// shape they stand for.
pub(super) const TORCH_CALLS: &[Reduce] = &[
    super::stdlib::ORDERED_DICT,
    // `torch.Size` is a tuple subclass, so it is the class called with the
    // tuple of extents.
    Reduce {
        via: Via::Global,
        path: "torch.Size",
        what: Shape::Size,
        names: &["extents"],
        args: Args::Contents,
        shape: |_c, args| matches!(args[0].kind, Kind::Tuple(_)).then_some(()),
    },
    // The same call, closed with NEWOBJ, which is what torch 1.5 and older
    // wrote: `Size` had no `__reduce__` of its own then, so pickle fell back
    // to `cls.__new__(cls, (2, 3))` for the tuple subclass.
    Reduce {
        via: Via::NewObj,
        path: "torch.Size",
        what: Shape::Size,
        names: &["extents"],
        args: Args::Contents,
        shape: |_c, args| matches!(args[0].kind, Kind::Tuple(_)).then_some(()),
    },
    // `torch.device('cpu')` and `torch.device('cuda', 0)`, which is the type
    // of device and, where the file has more than one of them, which.
    Reduce {
        via: Via::Global,
        path: "torch.device",
        what: Shape::Object,
        names: &["type"],
        args: Args::Fixed,
        shape: |_c, args| said(&args[0]).then_some(()),
    },
    Reduce {
        via: Via::Global,
        path: "torch.device",
        what: Shape::Object,
        names: &["type", "index"],
        args: Args::Fixed,
        shape: |_c, args| (said(&args[0]) && matches!(args[1].kind, Kind::Int { .. })).then_some(()),
    },
    // The layout a sparse tensor is stored in, which torch writes as a call
    // of one function over the layout's own name. Only the one layout whose
    // data this reader has measured: the others hand over a different tuple.
    Reduce {
        via: Via::Global,
        path: "torch.serialization._get_layout",
        what: Shape::Object,
        names: &["layout"],
        args: Args::Fixed,
        shape: |c, args| (c.text_of(&args[0]) == Some(SPARSE_COO)).then_some(()),
    },
    // `_rebuild_sparse_tensor(layout, (indices, values, size, is_coalesced))`.
    // Nothing is densified: the two tensors are read where they are and the
    // shape they stand for is beside them.
    Reduce {
        via: Via::Global,
        path: "torch._utils._rebuild_sparse_tensor",
        what: Shape::SparseTensor,
        names: &["layout", "data"],
        args: Args::Fixed,
        shape: |_c, args| (matches!(args[0].kind, Kind::Made { what: Shape::Object, .. }) && sparse_data(&args[1].kind)).then_some(()),
    },
];

/// The one sparse layout whose stored form this reader has measured.
const SPARSE_COO: &str = "torch.sparse_coo";

/// Whether a value is text, spelled here or named where the file spelled it.
fn said(value: &Value) -> bool {
    matches!(value.kind, Kind::Text { .. } | Kind::Ref(Names::Text { .. }))
}

/// What a COO sparse tensor is stored as: the indices, the values, the shape
/// the two stand for, and whether torch knew the indices were in order. The
/// last is left out by releases before it existed.
fn sparse_data(kind: &Kind) -> bool {
    let Kind::Tuple(held) = kind else { return false };
    let (indices, values, size, rest) = match held.as_slice() {
        [indices, values, size] => (indices, values, size, true),
        [indices, values, size, coalesced] => (indices, values, size, matches!(coalesced.kind, Kind::Bool(_) | Kind::None)),
        _ => return false,
    };
    matches!(indices.kind, Kind::Tensor(_)) && matches!(values.kind, Kind::Tensor(_)) && matches!(size.kind, Kind::Made { what: Shape::Size, .. }) && rest
}

/// The package torch's own classes are named from, which is the modules a
/// saved module's class may come from and nothing wider: a whole `nn.Module`
/// pickled as an object is a class under here with a state dict for state.
pub(super) const TORCH_CLASSES: &[&str] = &["torch.nn"];
