//! The callables worth recognising, and what they make of their arguments.
//!
//! A pickle names a callable and hands it arguments, and that is the only
//! structure it has. So knowing a pickle means knowing callables. Two things
//! are worth knowing about one:
//!
//! * **What it is**, so a row says `a pandas DataFrame` rather than
//!   `calls pandas.core.frame.DataFrame`. Nothing is read differently for it;
//!   it is a label on a name the file already wrote, the same way the hex dump
//!   reader labels a layout `xxd` after settling it.
//!
//! * **What its arguments say about a run of bytes.** This is the part that
//!   changes what is read. numpy hands the unpickler a dtype, a shape and a
//!   buffer; `datetime.datetime` hands it ten packed bytes. In both cases the
//!   bytes have a structure the file states nowhere near them, and the answer
//!   is an index into [`shapes`](super::shapes).
//!
//! **Module paths move.** numpy renamed `numpy.core` to `numpy._core` in
//! version 2, pandas has moved `Series` and `DataFrame` between modules more
//! than once, and scipy's sparse classes gained an underscore. Every pickle
//! ever written keeps the name it was written with, so the matching here is on
//! the last component or on a prefix wherever the old name is still out there,
//! and never on one exact string that happened to be current.

use super::machine::Value;
use super::shapes::{self, Packed};

/// What a payload turned out to be: which of [`shapes::cases`] to read it as,
/// and how many of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Payload {
    pub shape: usize,
    pub count: u64,
}

/// What a call makes of its arguments, where it makes anything: the bytes it
/// gives a shape to, how long they are, what shape, and how to say so.
pub(super) fn recognise(callable: &str, args: &[Value]) -> Option<(u64, u64, Payload, String)> {
    let (module, name) = callable.rsplit_once('.')?;
    match (root(module), name) {
        // A pickled ndarray. The call itself carries no data: the shape, the
        // dtype and the buffer arrive later, in the state a `BUILD` hands it,
        // which is `(version, shape, dtype, fortran, data)`.
        ("numpy", "_reconstruct") => array(args.get(4)?, args.get(2)?, args.get(1)?),
        // A view on a buffer, which is what pandas writes and what numpy
        // writes for an array that never owned its memory.
        ("numpy", "_frombuffer") => array(args.first()?, args.get(1)?, args.get(2)?),
        // One number that kept its dtype, which is what a numpy scalar in an
        // ordinary dict of metrics pickles as.
        ("numpy", "scalar") => {
            let dtype = dtype_of(args.first()?)?;
            let Value::Bytes { at, len } = args.get(1)? else { return None };
            let (shape, width) = shapes::dtype(&dtype)?;
            (*len == width).then(|| (*at, *len, Payload { shape, count: 1 }, format!("a single {dtype}")))
        }
        // The standard library's packed records. Each is the whole value of
        // the object in a byte string handed straight to the class, and each
        // is a fixed width: being wrong about the width would read a date out
        // of the field after it.
        ("datetime", "date") => packed(args.first()?, Packed::Date, "a date"),
        ("datetime", "time") => packed(args.first()?, Packed::Time, "a time of day"),
        ("datetime", "datetime") => packed(args.first()?, Packed::DateTime, "a date and time"),
        _ => None,
    }
}

/// What a rebuilder was handed to rebuild.
///
/// Some callables make nothing of their own. `copyreg._reconstructor` and
/// pandas' Cython helper are given a class and put one of those back together,
/// so a row saying what the *callable* is says the same thing for every class
/// in the file: every pandas array, whatever it holds, reads as the helper
/// that rebuilt it. The class is the first argument, and it is the answer.
///
/// Nothing when the class is one no recogniser knows, so the caller falls back
/// to naming the rebuilder, which is at least true.
pub(super) fn rebuilt(callable: &str, args: &[Value]) -> Option<&'static str> {
    let (module, name) = callable.rsplit_once('.')?;
    let hands_it_a_class = matches!(
        (root(module), name),
        ("copyreg", "_reconstructor" | "__newobj__")
            | ("pandas", "__pyx_unpickle_NDArrayBacked")
            | ("numpy", "_reconstruct")
    );
    hands_it_a_class.then(|| what(&args.first()?.global()?)).flatten()
}

/// What this callable is, in words, or nothing where it is not one anybody
/// would recognise.
///
/// Matched on the top of the module path and the class name, because the rest
/// of the path is what moves between versions: `pandas.Series`,
/// `pandas.core.series.Series` and `pandas.core.api.Series` are one class and
/// a pickle in an archive somewhere says each of them.
pub(super) fn what(callable: &str) -> Option<&'static str> {
    let (module, name) = callable.rsplit_once('.')?;
    Some(match (root(module), name) {
        // numpy. An array's data reads as numbers only from protocol 3 up.
        // Before that a pickle had no bytes object, so numpy writes the array
        // as a `str` through `_codecs.encode(..., 'latin1')`, and a byte over
        // 0x7f is two bytes in the file. There is no run of array data in such
        // a file to point a type at, which is a fact about the file rather
        // than a gap here.
        ("numpy", "_reconstruct" | "ndarray") => "a numpy array",
        ("numpy", "_frombuffer") => "a numpy array over a buffer",
        ("numpy", "scalar") => "a numpy scalar",
        ("numpy", "dtype") => "a numpy dtype",

        // pandas. A frame keeps its columns in a block manager and each block
        // is an array, so these are the rows between a dataframe and its
        // numbers.
        ("pandas", "DataFrame") => "a pandas DataFrame",
        ("pandas", "Series") => "a pandas Series",
        ("pandas", "BlockManager") => "a DataFrame's column blocks",
        ("pandas", "SingleBlockManager") => "a Series' single block",
        ("pandas", "_unpickle_block") => "a block of columns sharing a dtype",
        ("pandas", "_new_Index") => "a pandas Index",
        ("pandas", "Index" | "RangeIndex" | "DatetimeIndex" | "MultiIndex" | "CategoricalIndex") => "a pandas Index",
        ("pandas", "Categorical") => "a pandas Categorical: codes and categories",
        ("pandas", "CategoricalDtype") => "a pandas dtype listing the categories",
        ("pandas", "StringDtype" | "DatetimeTZDtype" | "PeriodDtype" | "IntervalDtype") => "a pandas dtype",
        // pandas keeps its own array types -- strings, dates with a zone,
        // categories -- in a class wrapping a numpy one, and rebuilds every
        // one of them through the same Cython helper.
        ("pandas", "__pyx_unpickle_NDArrayBacked") => "a pandas array wrapping a numpy array",
        ("pandas", "DatetimeArray" | "StringArray" | "IntegerArray" | "PeriodArray" | "TimedeltaArray") => {
            "a pandas array"
        }
        ("pandas", "Timestamp") => "a pandas Timestamp",
        ("pandas", "Timedelta") => "a pandas Timedelta",

        // PyTorch. A tensor is a shape and a stride over a storage, and the
        // storage is somewhere else: `torch.save` writes a zip, keeps the
        // pickle in `data.pkl` and every storage in a file of its own named by
        // a persistent id. A plain `pickle.dumps` of a tensor has nowhere to
        // put one, so it embeds a whole legacy `torch.save` file as a byte
        // string and reopens it through `_load_from_bytes`.
        ("torch", "_rebuild_tensor" | "_rebuild_tensor_v2" | "_rebuild_tensor_v3") => {
            "a torch tensor: shape and stride over a storage"
        }
        ("torch", "_rebuild_parameter") => "a torch parameter (a trainable tensor)",
        ("torch", "_rebuild_sparse_tensor") => "a sparse torch tensor",
        ("torch", "_load_from_bytes") => "a torch storage (an embedded torch.save file)",
        ("torch", "OrderedDict") => "a state dict",

        // scipy's sparse matrices, which are three arrays and a shape: the
        // values, where each one sits along a row, and where each row starts.
        ("scipy", "csr_matrix" | "csr_array" | "_csr") => "a CSR sparse matrix",
        ("scipy", "csc_matrix" | "csc_array") => "a CSC sparse matrix",
        ("scipy", "coo_matrix" | "coo_array") => "a COO sparse matrix",
        ("scipy", "dia_matrix" | "bsr_matrix" | "lil_matrix" | "dok_matrix") => "a sparse matrix",

        // scikit-learn. An estimator is a class and a dict of what it learned,
        // and what it learned is numpy arrays, so those type themselves.
        ("sklearn", "Tree") => "a decision tree's nodes and values",
        ("sklearn", _) => "a scikit-learn object",

        // The standard library.
        ("datetime", "date") => "a date",
        ("datetime", "time") => "a time of day",
        ("datetime", "datetime") => "a date and time",
        ("datetime", "timedelta") => "a length of time",
        ("datetime", "timezone") => "a fixed offset from UTC",
        ("collections", "OrderedDict") => "an ordered dict",
        ("collections", "defaultdict") => "a dict with a default for missing keys",
        ("collections", "Counter") => "a dict of counts",
        ("collections", "deque") => "a double-ended queue",
        ("builtins", "slice") => "a slice: start, stop and step",
        ("builtins", "complex") => "a complex number",
        ("builtins", "range" | "xrange") => "a range: start, stop and step",
        ("builtins", "set") => "a set",
        ("builtins", "frozenset") => "a frozenset",
        ("builtins", "bytearray") => "a bytearray",
        ("builtins", "getattr") => "an attribute fetched by name",
        ("copyreg", "_reconstructor") => "an instance via its base class (protocol 0/1)",
        ("copyreg", "__newobj__") => "an instance via __new__ without __init__",
        // `root` takes the leading underscore off, so this is `_codecs`.
        ("codecs", "encode") => "a bytes object stored as a str (protocols 0-2)",
        _ => return None,
    })
}

/// The top of a module path, which is the part that does not move. `pandas`
/// out of `pandas.core.internals.managers`, `numpy` out of `numpy._core.
/// multiarray`, and the leading underscore off a private one.
fn root(module: &str) -> &str {
    let top = module.split('.').next().unwrap_or(module);
    // Two of these were spelled differently in Python 2, and a pickle written
    // then still says so: the unpickler renames them on the way in, from the
    // same table `_compat_pickle` keeps, and a reader of the file has to do
    // the same or every `copy_reg._reconstructor` in an old archive reads as
    // a call nobody knows. Which, until this, is what they did.
    match top {
        "__builtin__" => "builtins",
        "copy_reg" => "copyreg",
        _ => top.strip_prefix('_').unwrap_or(top),
    }
}

/// A run of one dtype: the data, the dtype and the shape, wherever in the
/// arguments they were.
fn array(data: &Value, dtype: &Value, shape: &Value) -> Option<(u64, u64, Payload, String)> {
    let Value::Bytes { at, len } = data else { return None };
    let dtype = dtype_of(dtype)?;
    let (index, width) = shapes::dtype(&dtype)?;
    let dims: Vec<u64> =
        shape.tuple()?.iter().map(|v| v.int().and_then(|n| u64::try_from(n).ok())).collect::<Option<_>>()?;
    let count: u64 = dims.iter().copied().try_fold(1u64, |a, b| a.checked_mul(b))?;
    // The bytes have to be exactly the array. A pickle whose length and shape
    // disagree is one to leave as bytes rather than one to read half of.
    if count.checked_mul(width)? != *len {
        return None;
    }
    // numpy's own spelling of a shape, which is what the reader will have
    // typed to make the array and what every traceback about it will say.
    let shown = match dims.len() {
        0 => "()".to_string(),
        1 => format!("({},)", dims[0]),
        _ => format!("({})", dims.iter().map(u64::to_string).collect::<Vec<_>>().join(", ")),
    };
    Some((*at, *len, Payload { shape: index, count }, format!("{dtype} array, {shown}")))
}

/// A byte string that is the whole of a packed record, and exactly as long as
/// one.
fn packed(data: &Value, kind: Packed, word: &str) -> Option<(u64, u64, Payload, String)> {
    let Value::Bytes { at, len } = data else { return None };
    let (shape, want) = shapes::packed(kind);
    (*len == want).then(|| (*at, *len, Payload { shape, count: 1 }, word.to_string()))
}

/// The dtype of a value, as the string `npy` spells it.
///
/// A dtype reaches here as whatever the machine made of `numpy.dtype(...)`
/// followed by the `BUILD` that gives it its byte order. Where the `BUILD`
/// happened the byte order is already on the front; where it did not, the
/// dtype is a bare call and reads as byte-order-free, which the lookup then
/// accepts only for the types one byte wide.
fn dtype_of(v: &Value) -> Option<String> {
    match v {
        Value::Dtype(descr) => Some(descr.to_string()),
        Value::Call { callable, args } => {
            let full = callable.global()?;
            let (module, name) = full.rsplit_once('.')?;
            if root(module) != "numpy" || name != "dtype" {
                return None;
            }
            Some(format!("|{}", args.first()?.text()?))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn dtype(descr: &str) -> Value {
        Value::Dtype(descr.into())
    }

    fn shape(dims: &[i128]) -> Value {
        Value::Tuple(dims.iter().map(|d| Value::Int(*d)).collect())
    }

    #[test]
    fn a_reconstructed_array_is_read_as_its_dtype_and_shape() {
        let args =
            [Value::Int(1), shape(&[4, 6]), dtype("<f4"), Value::Bool(false), Value::Bytes { at: 0x9b, len: 96 }];
        let (at, _, payload, word) = recognise("numpy._core.multiarray._reconstruct", &args).expect("an array");
        assert_eq!(at, 0x9b);
        assert_eq!(payload.count, 24);
        assert!(word.contains("(4, 6)"), "said {word:?}");
    }

    /// numpy 1 wrote `numpy.core`, numpy 2 writes `numpy._core`, and a file
    /// from before 2024 is the commoner of the two.
    #[test]
    fn both_spellings_of_the_module_are_read() {
        let args = [Value::Bytes { at: 8, len: 16 }, dtype("<f8"), shape(&[2]), Value::Text("C".into())];
        for module in ["numpy._core.numeric", "numpy.core.numeric"] {
            assert!(recognise(&format!("{module}._frombuffer"), &args).is_some(), "{module} not recognised");
        }
    }

    /// Bytes that are not the length the shape and the dtype call for are left
    /// as bytes. Reading half an array is worse than reading none.
    #[test]
    fn a_length_that_does_not_agree_is_left_alone() {
        let args = [Value::Bytes { at: 0, len: 95 }, dtype("<f4"), shape(&[4, 6]), Value::Text("C".into())];
        assert!(recognise("numpy._core.numeric._frombuffer", &args).is_none());
    }

    /// An object array's data is a list of pickled objects, not a buffer, so
    /// nothing here should claim it.
    #[test]
    fn an_object_array_is_not_an_array_of_bytes() {
        let args = [Value::Int(1), shape(&[3]), dtype("|O"), Value::Bool(false), Value::Collection];
        assert!(recognise("numpy._core.multiarray._reconstruct", &args).is_none());
    }

    /// The three packed records are three fixed widths, and a byte string of
    /// the wrong length is not one of them however it was named.
    #[test]
    fn a_packed_date_is_recognised_only_at_its_own_width() {
        for (callable, width) in [("datetime.date", 4), ("datetime.time", 6), ("datetime.datetime", 10)] {
            let right = [Value::Bytes { at: 3, len: width }];
            let (at, len, _, _) = recognise(callable, &right).unwrap_or_else(|| panic!("{callable}"));
            assert_eq!((at, len), (3, width));
            let wrong = [Value::Bytes { at: 3, len: width + 1 }];
            assert!(recognise(callable, &wrong).is_none(), "{callable} at the wrong width");
        }
    }

    /// The names are matched on the top of the module path, because the rest
    /// of it moves: every one of these spellings is in some archive somewhere.
    #[test]
    fn a_class_is_named_wherever_its_module_moved_to() {
        for name in ["pandas.DataFrame", "pandas.core.frame.DataFrame", "pandas.core.api.DataFrame"] {
            assert_eq!(what(name), Some("a pandas DataFrame"), "{name}");
        }
        for name in ["scipy.sparse.csr.csr_matrix", "scipy.sparse._csr.csr_matrix"] {
            assert!(what(name).is_some_and(|w| w.contains("CSR")), "{name}");
        }
        assert_eq!(what("collections.OrderedDict"), Some("an ordered dict"));
        assert!(what("torch._utils._rebuild_tensor_v2").is_some_and(|w| w.contains("tensor")));
        assert!(what("sklearn.ensemble._forest.RandomForestClassifier").is_some());
        assert!(what("_codecs.encode").is_some(), "the private module keeps its underscore in the file");
        // What a Python 2 pickle calls them, which is most of what is in an
        // old archive.
        assert_eq!(what("copy_reg._reconstructor"), what("copyreg._reconstructor"));
        assert_eq!(what("__builtin__.xrange"), what("builtins.range"));
        assert!(what("__builtin__.complex").is_some());
        assert_eq!(what("mymodule.MyClass"), None, "nothing is invented for a class nobody knows");
        let _ = Arc::<[Value]>::from(&[][..]);
    }
}
