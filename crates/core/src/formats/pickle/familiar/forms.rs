//! Which forms there are, what each one allows, and the calls each one accepts
//! a REDUCE of.
//!
//! One grammar, read under one name per family. A form differs from its
//! neighbours only in which value productions it allows and which callables it
//! names, and both of those are written here rather than scattered through the
//! productions, so that widening a form is an edit to one table. The rule the
//! tables rest on is in [`object`](super::object): a module prefix says which
//! classes may be *named*, and never which callables may be *called*.
//!
//! One more name over all of them: the mixed form, whose tables are the union
//! of every family's. A union of enumerated sets is an enumerated set, so the
//! safety line does not move, and it is built here from [`DECLARED`] rather
//! than written out, so a family added to that table is in the union already.

use super::formnames::*;
use super::cursor::Cursor;
use super::packs::{covers, Extension};
use super::{Kind, Names, Shape, Value};


/// Which family a form belongs to, which is what says the file used the
/// productions the form is for rather than only the ones every form has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    Basic,
    Numpy,
    Builtins,
    Library,
    /// Tensors, and whatever plain data was saved beside them.
    Torch,
    /// Two or more of the extensions a file may use beside the basic grammar,
    /// counted as [`Extensions::count`](super::packs::Extensions::count)
    /// counts them.
    Mixed,
}

/// Which family a class named from this module belongs to, asked of the same
/// prefixes the forms are declared with so that the two cannot drift apart.
pub(super) fn extension_of(module: &str) -> Option<Extension> {
    DECLARED.iter().find_map(|d| covers(d.classes, module).then_some(d.extension).flatten())
}

/// The package torch spells its own calls and its own named values from, which
/// is not a module any class may be named from.
///
/// Asked wherever a global is named rather than read off the class prefix,
/// because `torch.Size`, `torch.device` and `torch.float32` are values rather
/// than classes the reader makes, and a file holding those and no tensor is
/// still a torch file. `collections.OrderedDict` is deliberately outside it: a
/// state dict is one of those, and a state dict is not a mixture of torch and
/// the standard library.
pub(super) const TORCH_PACKAGE: &[&str] = &["torch"];

/// Whether a form reads an array the way `joblib.dump` writes one, and whether
/// it holds the file to having one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Wrapped {
    /// No such production: the form reads an array where the numbers are.
    Refused,
    /// The form reads one and the file has to hold at least one, the way a
    /// form that allows a production requires the file to use it.
    Required,
    /// The form reads one and says nothing about whether the file has any,
    /// which is where the mixed form stands: the wrapper is one production
    /// among several and another of them is what made the file mixed.
    Allowed,
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
    /// Whether an array may arrive the way `joblib.dump` writes one, wrapped
    /// in an object whose state measures the run of bytes after it, and
    /// whether the file has to hold one.
    pub(super) joblib: Wrapped,
    /// The same question for the wrapper joblib wrote before 0.10, which
    /// names a `.npy` file beside the pickle and holds nothing of the numbers
    /// itself.
    pub(super) beside: Wrapped,
    pub(super) builtins: bool,
    /// Whether this form reads the calls `torch.save` writes for a tensor,
    /// which carry a persistent id naming numbers kept outside the pickle.
    pub(super) torch: bool,
    /// The module prefixes this form may name a class from. Empty for a form
    /// that names no class at all, which is where the basic, NumPy and
    /// builtins forms stand.
    pub(super) classes: &'static [&'static str],
    /// The globals this form may name and never calls, by their whole dotted
    /// path. What a `collections.defaultdict` is handed as its factory is one
    /// of a short list of builtin classes, and `builtins` is not a package any
    /// class at all may be named from, so those are enumerated here rather
    /// than reached by a module prefix.
    pub(super) names: &'static [&'static str],
    /// The callables this form's own library writes, and that it accepts a
    /// REDUCE of. The calls every form below protocol 4 shares are not here:
    /// the protocol decides those and [`Cursor::calls`](super::cursor::Cursor)
    /// puts them in front of these. Never anything else: see [`object`] for
    /// why a module prefix cannot stand in for this list.
    pub(super) calls: &'static [Reduce],
    /// Whether an array's values may be pickled objects rather than numbers,
    /// which is NumPy's `O8` dtype. A pandas index of column names is one, and
    /// so is every pandas column that is not numbers: text, dates, lists,
    /// exact numbers. What such an array may hold is what this form allows
    /// anywhere else, since the values are read against the same stack.
    pub(super) object_arrays: bool,
}

/// One callable a form accepts a REDUCE of, and what the library writes as its
/// arguments. `shape` is the check against those arguments; `names` says what
/// each of them is called, and its length is the arity.
///
/// A callable written with more than one argument shape has a row each, and
/// the first whose arity and shape both fit is the one the file matched: a
/// `datetime.datetime` takes a tzinfo when it is aware and not when it is
/// naive, and `collections.deque` was written three different ways across the
/// releases in the corpus.
#[derive(Debug, Clone, Copy)]
pub(super) struct Reduce {
    pub(super) path: &'static str,
    /// How the callable reached the REDUCE, which is as the global itself for
    /// every call but one.
    pub(super) via: Via,
    /// What the result of the call is, in the tree.
    pub(super) what: Shape,
    pub(super) names: &'static [&'static str],
    /// What the arguments are beyond their count, and what becomes of the
    /// result. [`Args::Fixed`] for all but the standard library's containers.
    pub(super) args: Args,
    pub(super) shape: fn(&Cursor, &[Value]) -> Option<()>,
}

/// What a call's arguments are beyond their number, and what the opcodes after
/// it may do to the result.
///
/// The standard library's containers are the reason this is not always
/// [`Args::Fixed`]. A path is its parts, however many there are; a `Counter`
/// is called with the mapping it holds; and an `OrderedDict`, a `defaultdict`
/// and a `deque` are called empty and filled by the `SETITEMS` or `APPENDS`
/// after them, which is a container being built the way every other container
/// in a pickle is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Args {
    /// A fixed few, one per name in `names`.
    Fixed,
    /// However many of one thing, under the one word `names` holds.
    Many,
    /// The one argument is what the result holds, so it is read as the
    /// result's contents rather than as an argument beside them.
    Contents,
    /// The result is empty and the opcodes after it fill it the way a
    /// dictionary is filled.
    FillsDict,
    /// The same, filled the way a list is.
    FillsList,
}

/// How a call named its callable, and which opcode closed it.
///
/// Almost always the global itself, closed by REDUCE. pandas 1.3 writes a
/// block as a `functools.partial` over `new_block` and then calls that: a
/// REDUCE whose callable is what another REDUCE made. And a class that takes
/// arguments but defines no reduce of its own is written with NEWOBJ instead,
/// which is how torch spelled `torch.Size` until it gave the class a
/// `__reduce__`. Nothing else may be called any of those ways, and each row
/// says which one it is, so what may happen is still a list rather than a
/// rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Via {
    Global,
    Partial,
    /// `cls.__new__(cls, *args)`, closed by NEWOBJ rather than REDUCE.
    NewObj,
}

/// The one callable a form accepts as the maker of another callable.
pub(super) const PARTIAL: &str = "functools.partial";

/// Nothing at all, for a form that enumerates no calls of its own.
const NO_CALLS: &[Reduce] = &[];

use super::sklearn::{SKLEARN_CALLS, SKLEARN_CLASSES};
use super::torch::{TORCH_CALLS, TORCH_CLASSES};

/// A form that names no class, which is every form below the library ones,
/// and a form that names no global it never calls, which is every form but the
/// standard library's.
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
        args: Args::Fixed,
        shape: |_c, args| (matches!(args[0].kind, Kind::Tuple(_)) && matches!(args[1].kind, Kind::List(_))).then_some(()),
    },
    Reduce {
        via: Via::Global,
        path: "pandas._libs.internals._unpickle_block",
        what: Shape::Block,
        names: &["values", "placement", "ndim"],
        args: Args::Fixed,
        shape: |_c, args| {
            // The placement is a slice of the frame's columns. pandas writes an
            // array of positions instead when a block's columns are not next to
            // each other, and that is a non-match until there is a file with
            // one in it.
            (holds_values(&args[0].kind)
                && places(&args[1].kind)
                && matches!(args[2].kind, Kind::Int { .. }))
            .then_some(())
        },
    },
    Reduce {
        via: Via::Global,
        path: "pandas.core.indexes.base._new_Index",
        what: Shape::Object,
        names: &["type", "state"],
        args: Args::Fixed,
        shape: new_index,
    },
    // An index of dates is rebuilt by a call of its own, with the same two
    // arguments.
    Reduce {
        via: Via::Global,
        path: "pandas.core.indexes.datetimes._new_DatetimeIndex",
        what: Shape::Object,
        names: &["type", "state"],
        args: Args::Fixed,
        shape: new_index,
    },
    // How far apart the dates of a regular index are, which pandas writes as
    // a call of the offset class with a count and whether it was normalised.
    Reduce {
        via: Via::Global,
        path: "pandas._libs.tslibs.offsets.Day",
        what: Shape::Object,
        names: &["n", "normalize"],
        args: Args::Fixed,
        shape: |_c, args| (matches!(args[0].kind, Kind::Int { .. }) && matches!(args[1].kind, Kind::Bool(_))).then_some(()),
    },
    Reduce {
        via: Via::Global,
        path: "pandas._libs.arrays.__pyx_unpickle_NDArrayBacked",
        what: Shape::Object,
        names: &["type", "checksum", "state"],
        args: Args::Fixed,
        shape: |_c, args| {
            (matches!(args[0].kind, Kind::Class { .. }) && matches!(args[1].kind, Kind::Int { .. }) && matches!(args[2].kind, Kind::None))
                .then_some(())
        },
    },
    // pandas 3.0 writes the dtype of a text column as a call of its storage
    // and the value it uses for a missing entry. Both spellings of the module
    // are named, as numpy's two are.
    Reduce { via: Via::Global, path: "pandas.StringDtype", what: Shape::Object, names: &["storage", "na_value"], args: Args::Fixed, shape: string_dtype },
    Reduce { via: Via::Global, path: "pandas.core.arrays.string_.StringDtype", what: Shape::Object, names: &["storage", "na_value"], args: Args::Fixed, shape: string_dtype },
    // pandas 1.3 writes a block as a `functools.partial` over `new_block` and
    // then calls the partial. Both halves are here: the partial may be made
    // only over this one global, and only what the partial made may be called.
    Reduce {
        via: Via::Global,
        path: PARTIAL,
        what: Shape::Object,
        names: &["func"],
        args: Args::Fixed,
        shape: |_c, args| matches!(&args[0].kind, Kind::Class { path, .. } if path == NEW_BLOCK).then_some(()),
    },
    Reduce {
        via: Via::Partial,
        path: NEW_BLOCK,
        what: Shape::Block,
        names: &["values", "placement"],
        args: Args::Fixed,
        shape: |_c, args| {
            (holds_values(&args[0].kind) && places(&args[1].kind)).then_some(())
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

/// Where a block sits: a slice, or a name for a slice the file wrote earlier.
/// Two frames whose blocks sit in the same places share the slice, which is
/// what `assign` and a shallow copy leave behind, and the second frame names
/// it out of the memo rather than spelling it again.
fn places(kind: &Kind) -> bool {
    matches!(kind, Kind::Made { what: Shape::Slice, .. } | Kind::Ref(Names::Made { what: Shape::Slice, .. }))
}

/// `_new_Index(cls, state)`: the class of the index and the dictionary that
/// finishes it.
fn new_index(_c: &Cursor, args: &[Value]) -> Option<()> {
    (matches!(args[0].kind, Kind::Class { .. }) && matches!(args[1].kind, Kind::Dict(_))).then_some(())
}

/// `StringDtype(storage, na_value)`: a word and a float, which is a NaN.
fn string_dtype(_c: &Cursor, args: &[Value]) -> Option<()> {
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
const SET_CALL: Reduce = Reduce { via: Via::Global, path: "builtins.set", what: Shape::Set, names: &["members"], args: Args::Fixed, shape: members };
const OLD_SET_CALL: Reduce = Reduce { path: "__builtin__.set", ..SET_CALL };
const FROZEN_CALL: Reduce =
    Reduce { via: Via::Global, path: "builtins.frozenset", what: Shape::FrozenSet, names: &["members"], args: Args::Fixed, shape: members };
const OLD_FROZEN_CALL: Reduce = Reduce { path: "__builtin__.frozenset", ..FROZEN_CALL };

/// `set(members)` and `frozenset(members)`: the members, written out for the
/// call and never named out of the memo, since the container holding them is
/// made for the call. CPython hands over a list and PyPy a tuple.
fn members(_c: &Cursor, args: &[Value]) -> Option<()> {
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
    args: Args::Fixed,
    shape: |_c, args| {
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
    joblib: Wrapped,
    beside: Wrapped,
    builtins: bool,
    torch: bool,
    classes: &'static [&'static str],
    /// Which family the classes this row whitelists belong to, which is what
    /// [`pack_of`] answers with. Nothing for a row that whitelists none: the
    /// three productions that are nobody's classes keep a count of themselves
    /// and the bit is read off that instead.
    extension: Option<Extension>,
    names: &'static [&'static str],
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
        joblib: Wrapped::Refused,
        beside: Wrapped::Refused,
        builtins: false,
        torch: false,
        classes: NO_CLASSES,
        extension: None,
        names: NO_CLASSES,
        calls: NO_CALLS,
        object_arrays: false,
    },
    Declared { ids: [NUMPY, NUMPY23, NUMPY1, NUMPY0], family: Family::Numpy, numpy: true, ..PLAIN },
    Declared { ids: [BUILTINS, BUILTINS23, BUILTINS1, BUILTINS0], family: Family::Builtins, builtins: true, ..PLAIN },
    Declared {
        ids: [SKLEARN, SKLEARN23, SKLEARN1, SKLEARN0],
        family: Family::Library,
        numpy: true,
        classes: SKLEARN_CLASSES,
        extension: Some(Extension::Sklearn),
        names: super::numpy::TYPE_NAMES,
        calls: SKLEARN_CALLS,
        // An estimator made of other estimators holds them in an array of
        // objects: `GradientBoostingClassifier.estimators_` is one per stage.
        // The values in it are what this form allows anywhere else, since
        // they are read against the same stack.
        object_arrays: true,
        ..PLAIN
    },
    Declared {
        ids: [SCIPY, SCIPY23, SCIPY1, SCIPY0],
        family: Family::Library,
        numpy: true,
        classes: &["scipy.sparse"],
        extension: Some(Extension::Scipy),
        names: super::numpy::TYPE_NAMES,
        ..PLAIN
    },
    Declared {
        ids: [PANDAS, PANDAS23, PANDAS1, PANDAS0],
        family: Family::Library,
        numpy: true,
        builtins: true,
        classes: &["pandas"],
        extension: Some(Extension::Pandas),
        names: super::numpy::TYPE_NAMES,
        calls: PANDAS_CALLS,
        object_arrays: true,
        joblib: Wrapped::Refused,
        beside: Wrapped::Refused,
        torch: false,
    },
    // The standard library's own classes. Its builtins are the ones the
    // builtins form already reads, so a file mixing a date with a complex
    // number is read here and one holding only the second stays there.
    Declared {
        ids: [STDLIB, STDLIB23, STDLIB1, STDLIB0],
        family: Family::Library,
        builtins: true,
        classes: super::stdlib::STDLIB_MODULES,
        extension: Some(Extension::Stdlib),
        names: super::stdlib::FACTORIES,
        calls: super::stdlib::STDLIB_CALLS,
        ..PLAIN
    },
    // What `joblib.dump` wrote, after the extensions it wraps, so that a
    // plain pickle of arrays or of estimators keeps the name it already had.
    // Object arrays are read here and nowhere else in the NumPy family:
    // joblib has no way to write one as a run of bytes, so it pickles the
    // array into the stream instead, and a file of arrays is a file of
    // whatever dtypes were dumped into it.
    Declared {
        ids: [JOBLIB, JOBLIB23, NOT_WRITTEN, NOT_WRITTEN],
        family: Family::Numpy,
        numpy: true,
        joblib: Wrapped::Required,
        object_arrays: true,
        ..PLAIN
    },
    // What joblib wrote before 0.10, which is the same idea with the numbers
    // in another file: one `.npy` beside the pickle per array, and a wrapper
    // in the pickle naming each. The layout differs, so the name does, and a
    // file of either kind is read under one row and never the other.
    Declared {
        ids: [JOBLIB_NPY, JOBLIB_NPY23, NOT_WRITTEN, NOT_WRITTEN],
        family: Family::Numpy,
        numpy: true,
        beside: Wrapped::Required,
        ..PLAIN
    },
    // What `torch.save` writes. A tensor's storage class is named inside the
    // tensor's own fixed run and never reaches the tree, and `collections` is
    // named through the one shared call rather than as a package a class may
    // come from, so a state dict is a torch file rather than a mixture of
    // torch and the standard library. `torch.nn` is here because a whole
    // module pickled as an object is a class under it; nothing wider is, and
    // `torch` itself is not, so a torch dtype is named through `names` and
    // never as a class.
    Declared {
        ids: [TORCH, TORCH23, NOT_WRITTEN, NOT_WRITTEN],
        family: Family::Torch,
        torch: true,
        classes: TORCH_CLASSES,
        extension: Some(Extension::Torch),
        names: super::torch::DTYPE_NAMES,
        calls: TORCH_CALLS,
        ..PLAIN
    },
    Declared {
        ids: [JOBLIB_SKLEARN, JOBLIB_SKLEARN23, NOT_WRITTEN, NOT_WRITTEN],
        family: Family::Library,
        numpy: true,
        joblib: Wrapped::Required,
        classes: SKLEARN_CLASSES,
        extension: Some(Extension::Sklearn),
        names: super::numpy::TYPE_NAMES,
        calls: SKLEARN_CALLS,
        // An estimator fitted on labels that are not numbers keeps them in
        // `classes_`, and a column of them in a frame is an array of objects.
        object_arrays: true,
        ..PLAIN
    },
];

/// What a family that says nothing else reads, so that a row names only what
/// makes it different.
const PLAIN: Declared = Declared {
    ids: [BASIC, BASIC23, BASIC1, BASIC0],
    family: Family::Basic,
    numpy: false,
    joblib: Wrapped::Refused,
    beside: Wrapped::Refused,
    builtins: false,
    torch: false,
    classes: NO_CLASSES,
    extension: None,
    names: NO_CLASSES,
    calls: NO_CALLS,
    object_arrays: false,
};

/// What the mixed form names: every family's class prefixes, named globals and
/// enumerated calls in one table each.
///
/// Gathered from [`DECLARED`] rather than written out, so that the union is
/// the families and cannot fall behind them. A row kept twice is kept once
/// here: scikit-learn's one call is named by the plain row and by the joblib
/// one over it, and the reading of it is the same either way.
struct Union {
    classes: Vec<&'static str>,
    names: Vec<&'static str>,
    calls: Vec<Reduce>,
}

fn union() -> &'static Union {
    static UNION: std::sync::OnceLock<Union> = std::sync::OnceLock::new();
    UNION.get_or_init(|| {
        let mut u = Union { classes: Vec::new(), names: Vec::new(), calls: Vec::new() };
        for d in DECLARED {
            for package in d.classes {
                if !u.classes.contains(package) {
                    u.classes.push(package);
                }
            }
            for name in d.names {
                if !u.names.contains(name) {
                    u.names.push(name);
                }
            }
            for call in d.calls {
                let seen = |had: &Reduce| had.path == call.path && had.via == call.via && had.names == call.names;
                if !u.calls.iter().any(seen) {
                    u.calls.push(*call);
                }
            }
        }
        u
    })
}

/// Every form: each family at each protocol range it is written at, and then
/// the mixed form at each of them.
///
/// The mixed form is last because it is the widest, and being last is what
/// keeps every file that already had a name: a file of one family matches the
/// form for that family before this one is tried at all.
pub(super) fn forms() -> Vec<(&'static str, Allow)> {
    let mut out: Vec<(&'static str, Allow)> = DECLARED
        .iter()
        .flat_map(|d| {
            RANGES.iter().enumerate().map(move |(at, protocols)| {
                let allow = Allow {
                    protocols,
                    family: d.family,
                    numpy: d.numpy,
                    joblib: d.joblib,
                    beside: d.beside,
                    builtins: d.builtins,
                    torch: d.torch,
                    classes: d.classes,
                    names: d.names,
                    calls: d.calls,
                    object_arrays: d.object_arrays,
                };
                (d.ids[at], allow)
            })
        })
        .filter(|(id, _)| !id.is_empty())
        .collect();
    let all = union();
    out.extend(RANGES.iter().enumerate().map(|(at, protocols)| {
        let allow = Allow {
            protocols,
            family: Family::Mixed,
            numpy: true,
            joblib: Wrapped::Allowed,
            // Permitted rather than required, the way the wrapper above it
            // is: a file that holds one and a value of another family is a
            // mixture, and one that holds nothing else is read under the row
            // written for that layout.
            beside: Wrapped::Allowed,
            builtins: true,
            torch: true,
            classes: &all.classes,
            names: &all.names,
            calls: &all.calls,
            object_arrays: true,
        };
        (MIXED_IDS[at], allow)
    }));
    out
}
