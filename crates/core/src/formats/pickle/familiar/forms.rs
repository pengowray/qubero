//! Which forms there are, what each one allows, and the calls each one accepts
//! a REDUCE of.
//!
//! One grammar, read under one of six names. A form differs from its
//! neighbours only in which value productions it allows and which callables it
//! names, and both of those are written here rather than scattered through the
//! productions, so that widening a form is an edit to one table. The rule the
//! tables rest on is in [`object`](super::object): a module prefix says which
//! classes may be *named*, and never which callables may be *called*.

use super::{Kind, Names, Shape, Value};

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

/// Which family a form belongs to, which is what says the file used the
/// productions the form is for rather than only the ones every form has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    Basic,
    Numpy,
    Builtins,
    Library,
}

/// Which value productions a form allows beyond the basic ones. A form that
/// allows a production also requires the file to use it, so a file holding
/// nothing but basic values is read under the basic form and not another.
#[derive(Debug, Clone, Copy)]
pub(super) struct Allow {
    /// The protocol bytes this form reads, and no others.
    pub(super) protocols: &'static [u8],
    pub(super) family: Family,
    pub(super) numpy: bool,
    pub(super) builtins: bool,
    /// The module prefixes this form may name a class from. Empty for a form
    /// that names no class at all, which is where the basic, NumPy and
    /// builtins forms stand.
    pub(super) classes: &'static [&'static str],
    /// The callables this form's own library writes, and that it accepts a
    /// REDUCE of. The calls every form below protocol 4 shares are not here:
    /// the protocol decides those and [`Cursor::calls`](super::cursor::Cursor)
    /// puts them in front of these. Never anything else: see [`object`] for
    /// why a module prefix cannot stand in for this list.
    pub(super) calls: &'static [Reduce],
    /// Whether an array's values may be pickled objects rather than numbers,
    /// which is NumPy's `O8` dtype. A pandas index of column names is one, and
    /// nothing else in the corpus is.
    pub(super) object_arrays: bool,
}

/// One callable a form accepts a REDUCE of, and what the library writes as its
/// arguments. `shape` is the check against those arguments; `names` says what
/// each of them is called, and its length is the arity.
#[derive(Debug, Clone, Copy)]
pub(super) struct Reduce {
    pub(super) path: &'static str,
    /// How the callable reached the REDUCE, which is as the global itself for
    /// every call but one.
    pub(super) via: Via,
    /// What the result of the call is, in the tree.
    pub(super) what: Shape,
    pub(super) names: &'static [&'static str],
    pub(super) shape: fn(&[Value]) -> Option<()>,
}

/// How a REDUCE named its callable.
///
/// Almost always the global itself. The one exception is pandas 1.3, which
/// writes a block as a `functools.partial` over `new_block` and then calls
/// that: a REDUCE whose callable is what another REDUCE made. Nothing else may
/// be called that way, and the partial's own call is enumerated like any other,
/// so what may happen is still a list rather than a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Via {
    Global,
    Partial,
}

/// The one callable a form accepts as the maker of another callable.
pub(super) const PARTIAL: &str = "functools.partial";

/// Nothing at all, for a form that enumerates no calls of its own.
const NO_CALLS: &[Reduce] = &[];
/// The calls scikit-learn writes. One: a decision tree's array of nodes lives
/// in a `Tree`, which is constructed from how many features, classes and
/// outputs it was fitted on and handed its arrays by the BUILD after it.
const SKLEARN_CALLS: &[Reduce] = &[Reduce {
    via: Via::Global,
    path: "sklearn.tree._tree.Tree",
    what: Shape::Object,
    names: &["n_features", "n_classes", "n_outputs"],
    shape: |args| {
        matches!(
            (&args[0].kind, &args[1].kind, &args[2].kind),
            (Kind::Int { .. }, Kind::Array { .. }, Kind::Int { .. })
        )
        .then_some(())
    },
}];
/// A form that names no class, which is every form below the library ones.
const NO_CLASSES: &[&str] = &[];

/// The calls pandas writes, which is the whole of what a REDUCE may do under
/// the pandas form.
///
/// A frame is a `BlockManager` over blocks of columns and two axes; a block is
/// an array, where it sits in the frame and how many axes it has; an axis is a
/// class and the dictionary that finishes it; and an array of something other
/// than numbers is an `NDArrayBacked` around one that is. The dtype of a text
/// column in pandas 3.0 is written as a call as well.
const PANDAS_CALLS: &[Reduce] = &[
    Reduce {
        via: Via::Global,
        path: "pandas.core.internals.managers.BlockManager",
        what: Shape::Object,
        names: &["blocks", "axes"],
        shape: |args| (matches!(args[0].kind, Kind::Tuple(_)) && matches!(args[1].kind, Kind::List(_))).then_some(()),
    },
    Reduce {
        via: Via::Global,
        path: "pandas._libs.internals._unpickle_block",
        what: Shape::Block,
        names: &["values", "placement", "ndim"],
        shape: |args| {
            // The placement is a slice of the frame's columns. pandas writes an
            // array of positions instead when a block's columns are not next to
            // each other, and that is a non-match until there is a file with
            // one in it.
            (holds_values(&args[0].kind)
                && matches!(args[1].kind, Kind::Made { what: Shape::Slice, .. })
                && matches!(args[2].kind, Kind::Int { .. }))
            .then_some(())
        },
    },
    Reduce {
        via: Via::Global,
        path: "pandas.core.indexes.base._new_Index",
        what: Shape::Object,
        names: &["type", "state"],
        shape: new_index,
    },
    // An index of dates is rebuilt by a call of its own, with the same two
    // arguments.
    Reduce {
        via: Via::Global,
        path: "pandas.core.indexes.datetimes._new_DatetimeIndex",
        what: Shape::Object,
        names: &["type", "state"],
        shape: new_index,
    },
    // How far apart the dates of a regular index are, which pandas writes as
    // a call of the offset class with a count and whether it was normalised.
    Reduce {
        via: Via::Global,
        path: "pandas._libs.tslibs.offsets.Day",
        what: Shape::Object,
        names: &["n", "normalize"],
        shape: |args| (matches!(args[0].kind, Kind::Int { .. }) && matches!(args[1].kind, Kind::Bool(_))).then_some(()),
    },
    Reduce {
        via: Via::Global,
        path: "pandas._libs.arrays.__pyx_unpickle_NDArrayBacked",
        what: Shape::Object,
        names: &["type", "checksum", "state"],
        shape: |args| {
            (matches!(args[0].kind, Kind::Class { .. }) && matches!(args[1].kind, Kind::Int { .. }) && matches!(args[2].kind, Kind::None))
                .then_some(())
        },
    },
    // pandas 3.0 writes the dtype of a text column as a call of its storage
    // and the value it uses for a missing entry. Both spellings of the module
    // are named, as numpy's two are.
    Reduce { via: Via::Global, path: "pandas.StringDtype", what: Shape::Object, names: &["storage", "na_value"], shape: string_dtype },
    Reduce { via: Via::Global, path: "pandas.core.arrays.string_.StringDtype", what: Shape::Object, names: &["storage", "na_value"], shape: string_dtype },
    // pandas 1.3 writes a block as a `functools.partial` over `new_block` and
    // then calls the partial. Both halves are here: the partial may be made
    // only over this one global, and only what the partial made may be called.
    Reduce {
        via: Via::Global,
        path: PARTIAL,
        what: Shape::Object,
        names: &["func"],
        shape: |args| matches!(&args[0].kind, Kind::Class { path, .. } if path == NEW_BLOCK).then_some(()),
    },
    Reduce {
        via: Via::Partial,
        path: NEW_BLOCK,
        what: Shape::Block,
        names: &["values", "placement"],
        shape: |args| {
            (holds_values(&args[0].kind) && matches!(args[1].kind, Kind::Made { what: Shape::Slice, .. })).then_some(())
        },
    },
];

/// The one global pandas makes a partial over.
const NEW_BLOCK: &str = "pandas.core.internals.blocks.new_block";

/// What a block's values may be: numbers, pickled objects, or an array of one
/// of those wrapped in a class of pandas' own.
fn holds_values(kind: &Kind) -> bool {
    matches!(kind, Kind::Array { .. } | Kind::Objects { .. } | Kind::Made { .. } | Kind::Instance { .. })
}

/// `_new_Index(cls, state)`: the class of the index and the dictionary that
/// finishes it.
fn new_index(args: &[Value]) -> Option<()> {
    (matches!(args[0].kind, Kind::Class { .. }) && matches!(args[1].kind, Kind::Dict(_))).then_some(())
}

/// `StringDtype(storage, na_value)`: a word and a float, which is a NaN.
fn string_dtype(args: &[Value]) -> Option<()> {
    (matches!(args[0].kind, Kind::Text { .. } | Kind::Ref(Names::Text { .. })) && matches!(args[1].kind, Kind::Float { .. }))
        .then_some(())
}

/// The two containers and the one byte string that protocols 2 and 3 have no
/// opcode for and write as a call instead.
///
/// A set and a frozenset are built from a list, so they are folded off the
/// stack like any other call rather than read as a fixed run. An empty byte
/// string is `bytes()` called with nothing, which is the one spelling protocol
/// 2 has for it; a byte string with anything in it goes through `_codecs` and
/// is a fixed run, in `codecs.rs`. `fix_imports` is what moves the module
/// between `builtins` and `__builtin__`, and the protocol decides which.
const SET_CALL: Reduce = Reduce { via: Via::Global, path: "builtins.set", what: Shape::Set, names: &["members"], shape: members };
const OLD_SET_CALL: Reduce = Reduce { path: "__builtin__.set", ..SET_CALL };
const FROZEN_CALL: Reduce =
    Reduce { via: Via::Global, path: "builtins.frozenset", what: Shape::FrozenSet, names: &["members"], shape: members };
const OLD_FROZEN_CALL: Reduce = Reduce { path: "__builtin__.frozenset", ..FROZEN_CALL };

/// `set(members)` and `frozenset(members)`: the members, written out for the
/// call and never named out of the memo, since the container holding them is
/// made for the call. CPython hands over a list and PyPy a tuple.
fn members(args: &[Value]) -> Option<()> {
    matches!(args[0].kind, Kind::List(_) | Kind::Tuple(_)).then_some(())
}

/// The calls every form reads below protocol 4, whatever else it reads: the
/// two containers and the byte string that protocol has no opcode for.
pub(super) const BELOW_FOUR: &[Reduce] = &[SET_CALL, OLD_SET_CALL, FROZEN_CALL, OLD_FROZEN_CALL];

/// How an object of a class is made below protocol 2, which had no NEWOBJ.
///
/// `copy_reg._reconstructor(cls, base, state)` is what `object.__reduce_ex__`
/// returns there: the class, the class it inherits its layout from, and the
/// argument that base is constructed with. Only `object` and `None`, which is
/// `cls.__new__(cls)` written the long way round and is the one shape the
/// NEWOBJ production already accepts. A different base class is a class being
/// told to construct itself out of values, which is the class's business.
pub(super) const RECONSTRUCTOR: &str = "copy_reg._reconstructor";
/// The base class it is handed, which a form may name and never calls.
pub(super) const BASE_CLASS: &str = "__builtin__.object";
pub(super) const MAKE_OBJECT: &[Reduce] = &[Reduce {
    via: Via::Global,
    path: RECONSTRUCTOR,
    what: Shape::Object,
    names: &["type", "base", "state"],
    shape: |args| {
        (matches!(&args[0].kind, Kind::Class { .. })
            && matches!(&args[1].kind, Kind::Class { path, .. } if path == BASE_CLASS)
            && matches!(args[2].kind, Kind::None))
        .then_some(())
    },
}];

/// One family of forms, declared once and read at every protocol.
///
/// A family is what a file is *about*: plain data, arrays, one library, the
/// standard library. The protocol it was written at decides the spellings, and
/// every one of those is already protocol-dependent in the cursor, so a family
/// says nothing about protocols here beyond the name each of its forms goes by.
/// Adding one is this row and the calls it names, and nothing else.
struct Declared {
    /// What each of the family's forms is called, in the order of [`RANGES`].
    ids: [&'static str; 4],
    family: Family,
    numpy: bool,
    builtins: bool,
    classes: &'static [&'static str],
    calls: &'static [Reduce],
    object_arrays: bool,
}

/// The protocol ranges a form is written for, in the order a file is tried
/// against them. The protocol byte tells them apart at the second byte of a
/// file, or at the first opcode of one with no opener, so a file only ever
/// does the work of the forms its own protocol has.
const RANGES: [&[u8]; 4] = [&[4, 5], &[2, 3], &[1], &[0]];

/// Every family. The names are written out rather than made up from the
/// family and the range, because a form identifier is what a reader compares
/// against and each carries its own revision.
const DECLARED: &[Declared] = &[
    Declared {
        ids: [BASIC, BASIC23, BASIC1, BASIC0],
        family: Family::Basic,
        numpy: false,
        builtins: false,
        classes: NO_CLASSES,
        calls: NO_CALLS,
        object_arrays: false,
    },
    Declared { ids: [NUMPY, NUMPY23, NUMPY1, NUMPY0], family: Family::Numpy, numpy: true, ..PLAIN },
    Declared { ids: [BUILTINS, BUILTINS23, BUILTINS1, BUILTINS0], family: Family::Builtins, builtins: true, ..PLAIN },
    Declared {
        ids: [SKLEARN, SKLEARN23, SKLEARN1, SKLEARN0],
        family: Family::Library,
        numpy: true,
        classes: &["sklearn"],
        calls: SKLEARN_CALLS,
        ..PLAIN
    },
    Declared {
        ids: [SCIPY, SCIPY23, SCIPY1, SCIPY0],
        family: Family::Library,
        numpy: true,
        classes: &["scipy.sparse"],
        ..PLAIN
    },
    Declared {
        ids: [PANDAS, PANDAS23, PANDAS1, PANDAS0],
        family: Family::Library,
        numpy: true,
        builtins: true,
        classes: &["pandas"],
        calls: PANDAS_CALLS,
        object_arrays: true,
    },
];

/// What a family that says nothing else reads, so that a row names only what
/// makes it different.
const PLAIN: Declared = Declared {
    ids: [BASIC, BASIC23, BASIC1, BASIC0],
    family: Family::Basic,
    numpy: false,
    builtins: false,
    classes: NO_CLASSES,
    calls: NO_CALLS,
    object_arrays: false,
};

/// Every form: each family at each protocol range.
pub(super) fn forms() -> Vec<(&'static str, Allow)> {
    DECLARED
        .iter()
        .flat_map(|d| {
            RANGES.iter().enumerate().map(move |(at, protocols)| {
                let allow = Allow {
                    protocols,
                    family: d.family,
                    numpy: d.numpy,
                    builtins: d.builtins,
                    classes: d.classes,
                    calls: d.calls,
                    object_arrays: d.object_arrays,
                };
                (d.ids[at], allow)
            })
        })
        .collect()
}
