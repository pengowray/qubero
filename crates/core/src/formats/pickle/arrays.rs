//! Recognising the calls that mean "these bytes are an array".
//!
//! Almost every pickle worth opening holds numbers rather than objects: a
//! model's weights, a dataframe's columns, a metric written down once a
//! second for a week. All of them arrive the same way. numpy hands the
//! unpickler a callable, a dtype, a shape and a run of bytes, and the
//! unpickler puts them together. So the run of bytes is typed here by
//! recognising the callable and reading the arguments beside it.
//!
//! Three callables cover it, at two module paths each. numpy renamed
//! `numpy.core` to `numpy._core` in version 2, and every pickle written before
//! 2024 says the old one, so both are matched and neither is preferred.
//!
//! | Callable | Arguments | Written by |
//! | --- | --- | --- |
//! | `multiarray._reconstruct` | filled in later by `BUILD`, as `(1, shape, dtype, fortran, data)` | `pickle.dumps(array)` |
//! | `numeric._frombuffer` | `(data, dtype, shape, order)` | pandas, and numpy for a view on a buffer |
//! | `multiarray.scalar` | `(dtype, data)` | one number that kept its dtype |
//!
//! What comes back is an index into [`npy::dtypes`], which is the same table
//! the `.npy` reader uses. That is deliberate: a `.npy` file and a pickled
//! array hold the same bytes described the same way, and two tables would
//! drift.
//!
//! None of this reaches a pickle written at protocol 2 or below, and nothing
//! is wrong when it does not. Those protocols have no bytes object, so numpy
//! writes an array as Latin-1 text handed to `_codecs.encode`: the file holds
//! the text's UTF-8, not the array's bytes, and there is nothing in place to
//! type. Protocol 3 brought `BINBYTES` and every array since is verbatim.

use std::sync::OnceLock;

use super::machine::{Array, Value};
use crate::formats::npy;

/// What a call turns out to make of its arguments: where the bytes are, what
/// they hold, and how to say so in a row.
pub(super) fn recognise(callable: &str, args: &[Value]) -> Option<(u64, u64, Array, String)> {
    let (module, name) = callable.rsplit_once('.')?;
    if !matches!(module, "numpy._core.multiarray" | "numpy.core.multiarray" | "numpy._core.numeric" | "numpy.core.numeric") {
        return None;
    }
    match name {
        // The state a `BUILD` hands `_reconstruct`: a version, the shape, the
        // dtype, whether it is Fortran-ordered, and the data.
        "_reconstruct" => array(args.get(4)?, args.get(2)?, args.get(1)?),
        // A view on a buffer: the data first, then the dtype and the shape.
        "_frombuffer" => array(args.first()?, args.get(1)?, args.get(2)?),
        // One value that kept its dtype, which is what a numpy number in an
        // ordinary dict pickles as.
        "scalar" => {
            let dtype = dtype_of(args.first()?)?;
            let Value::Bytes { at, len } = args.get(1)? else { return None };
            let (index, width) = lookup(&dtype)?;
            (*len == width).then(|| (*at, *len, Array { dtype: index, elements: 1 }, format!("one {dtype}")))
        }
        _ => None,
    }
}

/// The three things an array needs, wherever in the arguments they were.
fn array(data: &Value, dtype: &Value, shape: &Value) -> Option<(u64, u64, Array, String)> {
    let Value::Bytes { at, len } = data else { return None };
    let dtype = dtype_of(dtype)?;
    let (index, width) = lookup(&dtype)?;
    let dims: Vec<u64> = shape.tuple()?.iter().map(|v| v.int().and_then(|n| u64::try_from(n).ok())).collect::<Option<_>>()?;
    let elements: u64 = dims.iter().copied().try_fold(1u64, |a, b| a.checked_mul(b))?;
    // The bytes have to be exactly the array. A pickle whose length and shape
    // disagree is one to leave as bytes rather than one to read half of.
    if elements.checked_mul(width)? != *len {
        return None;
    }
    // numpy's own spelling of a shape, which is what the reader will have
    // typed to make the array and what every traceback about it will say.
    let shown = match dims.len() {
        0 => "()".to_string(),
        1 => format!("({},)", dims[0]),
        _ => format!("({})", dims.iter().map(u64::to_string).collect::<Vec<_>>().join(", ")),
    };
    Some((*at, *len, Array { dtype: index, elements }, format!("{dtype} array, {shown}")))
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
            let name = callable.global()?;
            if !matches!(name.as_str(), "numpy.dtype" | "numpy._core.multiarray.dtype") {
                return None;
            }
            Some(format!("|{}", args.first()?.text()?))
        }
        _ => None,
    }
}

/// Where this dtype sits in [`npy::dtypes`], and how wide one value of it is.
fn lookup(descr: &str) -> Option<(usize, u64)> {
    static TABLE: OnceLock<Vec<(String, u64)>> = OnceLock::new();
    let table = TABLE.get_or_init(|| npy::dtypes().into_iter().map(|(k, _, w)| (k, w as u64)).collect());
    table.iter().position(|(k, _)| k == descr).map(|i| (i, table[i].1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dtype(descr: &str) -> Value {
        Value::Dtype(descr.into())
    }

    fn shape(dims: &[i128]) -> Value {
        Value::Tuple(dims.iter().map(|d| Value::Int(*d)).collect())
    }

    #[test]
    fn a_reconstructed_array_is_read_as_its_dtype_and_shape() {
        let args = [
            Value::Int(1),
            shape(&[4, 6]),
            dtype("<f4"),
            Value::Bool(false),
            Value::Bytes { at: 0x9b, len: 96 },
        ];
        let (at, _, array, word) = recognise("numpy._core.multiarray._reconstruct", &args).expect("an array");
        assert_eq!(at, 0x9b);
        assert_eq!(array.elements, 24);
        assert!(word.contains("(4, 6)"), "said {word:?}");
    }

    /// numpy 1 wrote `numpy.core`, numpy 2 writes `numpy._core`, and a file
    /// from before 2024 is the commoner of the two.
    #[test]
    fn both_spellings_of_the_module_are_read() {
        let args = [Value::Bytes { at: 8, len: 16 }, dtype("<f8"), shape(&[2]), Value::Text("C".into())];
        for module in ["numpy._core.numeric", "numpy.core.numeric"] {
            let got = recognise(&format!("{module}._frombuffer"), &args);
            assert!(got.is_some(), "{module} not recognised");
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

    #[test]
    fn a_scalar_keeps_its_dtype() {
        let args = [dtype("<i8"), Value::Bytes { at: 4, len: 8 }];
        let (_, _, array, word) = recognise("numpy._core.multiarray.scalar", &args).expect("a scalar");
        assert_eq!(array.elements, 1);
        assert!(word.contains("<i8"), "said {word:?}");
    }

    #[test]
    fn nothing_else_is_recognised() {
        let args = [Value::Bytes { at: 0, len: 8 }];
        assert!(recognise("pandas.DataFrame", &args).is_none());
        assert!(recognise("numpy._core.multiarray._reconstruct", &[]).is_none());
    }
}
