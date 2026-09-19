//! An array whose values are pickled objects, read as the rows it shows.
//!
//! NumPy's object dtype has no numbers to write, so the values are pickled one
//! by one after the array, or, when `joblib.dump` wrote the file, into a whole
//! pickle of their own in the stream where the bytes would have gone. Either
//! way the values are nodes rather than a run to read, which is what sets this
//! apart from every other array and is why it is here rather than in
//! [`pickleparts`](super::pickleparts).

use super::pickleparts::{call_of, says_array, Label, Part, NESTED_FIELD};
use crate::formats::pickle::familiar::{Dtype, Match, Storage, Value};

/// The rows an object array shows and the nodes under it.
pub(super) fn object_array<'a>(
    found: &'a Match,
    v: &'a Value,
    dimensions: &[u64],
    fortran_order: bool,
    items: &'a [Value],
    nested: &Option<usize>,
) -> (Vec<(Label, Part<'a>)>, Vec<(Label, Part<'a>)>) {

        let notes = says_array(&Dtype::Objects, dimensions, fortran_order, Storage::Raw, false);
        let mut kids = Vec::new();
        if let Some(call) = call_of(found, v) {
            kids.push((Label::Field(call.name), Part::Call(call, v)));
        }
        // `joblib.dump` has no way to write these values as bytes, so
        // it pickles the array into the stream where the bytes would
        // go. The values are in there, a pickle deeper than the ones
        // beside them.
        match nested {
            Some(_) => kids.push((Label::Field(NESTED_FIELD), Part::Nested(v))),
            None => kids.extend(items.iter().enumerate().map(|(i, x)| (Label::Index(i), Part::Value(x)))),
        }
        (notes, kids)
}
