//! The names the forms go by: one per family and protocol range, and the four
//! the mixed form goes by.
//!
//! Apart from [`forms`](super::forms) because they are what the rest of the
//! program says to this one: a test names a form, `pickle_forms` prints one,
//! and the collection's own notes quote them. A name changes when the grammar
//! it stands for changes, and nothing else in that file has to be read to
//! change one.

/// The forms, each a named grammar over the same envelope. They differ in
/// which value productions they allow, and every one of those is enumerated.
pub(super) const BASIC: &str = "basic-p4-p5-v5";
pub(super) const NUMPY: &str = "numpy-array-p4-p5-v6";
pub(super) const BUILTINS: &str = "builtins-values-p4-p5-v3";
/// The library forms. Each is the plain object production over one package,
/// with the calls that library writes enumerated beside it, so a file is read
/// under the form for the library that wrote it and no other.
pub(super) const SKLEARN: &str = "sklearn-estimator-p4-p5-v1";
pub(super) const SCIPY: &str = "scipy-sparse-p4-p5-v1";
pub(super) const PANDAS: &str = "pandas-frame-p4-p5-v1";

/// The same six families at protocols 2 and 3, which is what Python wrote by
/// default from 3.0 to 3.7 and what Python 2 wrote whenever it was asked for
/// the highest protocol it had.
///
/// Separate identifiers rather than a wider protocol range on the six above,
/// because the instructions are not the same ones: a memo mark is BINPUT with
/// an index rather than MEMOIZE, a callable is GLOBAL's two lines rather than
/// STACK_GLOBAL, a set and a byte string are calls rather than literals, and
/// there is no framing. A reader comparing form identifiers is comparing
/// grammars, so the two are named apart.
pub(super) const BASIC23: &str = "basic-p2-p3-v1";
pub(super) const NUMPY23: &str = "numpy-array-p2-p3-v1";
pub(super) const BUILTINS23: &str = "builtins-values-p2-p3-v1";
pub(super) const SKLEARN23: &str = "sklearn-estimator-p2-p3-v1";
pub(super) const SCIPY23: &str = "scipy-sparse-p2-p3-v1";
pub(super) const PANDAS23: &str = "pandas-frame-p2-p3-v1";

/// The same families at protocol 1, which is what Python 2 wrote when it was
/// asked for a binary pickle before protocol 2 existed, and what
/// `cPickle.dump(obj, f, 1)` wrote for years after.
///
/// Named apart again, and for the same reason: a file at protocol 1 has no
/// PROTO opener, writes `True` and `False` as integer lines, has one tuple
/// opcode rather than four, writes a `long` as a line of digits, and builds an
/// object through `copy_reg._reconstructor` rather than NEWOBJ.
pub(super) const BASIC1: &str = "basic-p1-v1";
pub(super) const NUMPY1: &str = "numpy-array-p1-v1";
pub(super) const BUILTINS1: &str = "builtins-values-p1-v1";
pub(super) const SKLEARN1: &str = "sklearn-estimator-p1-v1";
pub(super) const SCIPY1: &str = "scipy-sparse-p1-v1";
pub(super) const PANDAS1: &str = "pandas-frame-p1-v1";

/// And at protocol 0, the text protocol: what Python wrote by default until
/// Python 3.0 and what `pickle.dumps(obj)` gave anyone who never named one.
///
/// Every value there is an opcode and a line, so a number is `repr`, a text is
/// escaped, and a container is filled one entry at a time with no batching in
/// it at all.
pub(super) const BASIC0: &str = "basic-p0-v1";
pub(super) const NUMPY0: &str = "numpy-array-p0-v1";
pub(super) const BUILTINS0: &str = "builtins-values-p0-v1";
pub(super) const SKLEARN0: &str = "sklearn-estimator-p0-v1";
pub(super) const SCIPY0: &str = "scipy-sparse-p0-v1";
pub(super) const PANDAS0: &str = "pandas-frame-p0-v1";

/// The standard library's own classes, which is what a pickle of ordinary
/// program state is full of: dates, ordered and defaulting dictionaries,
/// counters, queues, exact numbers, ids and paths. Each is a `REDUCE` of one
/// enumerated callable with the argument shape that callable is written with,
/// and `uuid.UUID` is the plain object production. See
/// [`stdlib`](super::stdlib).
pub(super) const STDLIB: &str = "stdlib-values-p4-p5-v1";
pub(super) const STDLIB23: &str = "stdlib-values-p2-p3-v1";
pub(super) const STDLIB1: &str = "stdlib-values-p1-v1";
pub(super) const STDLIB0: &str = "stdlib-values-p0-v1";

/// What `joblib.dump` writes: the same families, with every array replaced by
/// the wrapper joblib puts in front of the array's own bytes. See
/// [`joblib`](super::joblib).
///
/// Two rows rather than one, because a joblib file is a file of whatever was
/// dumped into it: arrays and plain data in one, scikit-learn's estimators in
/// the other. Neither is a copy of the family it extends: the second names the
/// same classes and the same calls the plain scikit-learn row does. Anything
/// else dumped this way, a frame or a sparse matrix or a date beside an array,
/// is a mixture and is read under [`MIXED`], which permits the wrapper rather
/// than requiring it.
///
/// Only at protocols 2 and up. joblib builds the wrapper with NEWOBJ, which
/// arrived at protocol 2, so the two lower ranges have no name here at all.
pub(super) const JOBLIB: &str = "joblib-arrays-p4-p5-v1";
pub(super) const JOBLIB23: &str = "joblib-arrays-p2-p3-v1";
pub(super) const JOBLIB_SKLEARN: &str = "joblib-sklearn-p4-p5-v1";
pub(super) const JOBLIB_SKLEARN23: &str = "joblib-sklearn-p2-p3-v1";
/// What `torch.save` writes: tensors, and whatever plain data was saved
/// beside them. See [`torch`](super::torch).
///
/// Only at protocols 2 and up. torch writes protocol 2 by default and takes a
/// higher one from the caller; it has never written 1 or 0, and a tensor's
/// instructions at those protocols have not been measured, so the family has
/// no name there.
pub(super) const TORCH: &str = "torch-tensors-p4-p5-v1";
pub(super) const TORCH23: &str = "torch-tensors-p2-p3-v1";
/// A range this family is never written at, which [`forms`] leaves out.
pub(super) const NOT_WRITTEN: &str = "";

/// A file holding values of more than one family, which is what a pickle of
/// ordinary program state is: a date beside an array, a frame beside a note, a
/// fitted model beside the day it was fitted.
///
/// Every family's classes, calls and named globals at once, with the joblib
/// wrapper allowed and not required. Tried after all of them, so a file of one
/// family keeps the name it already had, and it requires the file to have used
/// two families of values, so that the name it goes by is true of it.
pub(super) const MIXED: &str = "mixed-values-p4-p5-v1";
pub(super) const MIXED23: &str = "mixed-values-p2-p3-v1";
pub(super) const MIXED1: &str = "mixed-values-p1-v1";
pub(super) const MIXED0: &str = "mixed-values-p0-v1";
/// Its four names, in the order of [`RANGES`], as a family's row holds them.
pub(super) const MIXED_IDS: [&str; 4] = [MIXED, MIXED23, MIXED1, MIXED0];
