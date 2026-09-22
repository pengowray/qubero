# Familiar Pickle Forms (FPF)

Status: sixth implementation slice. The recogniser is in
`crates/core/src/formats/pickle/familiar/`; a match is now a template of its
own, `picklefpf`, which places the captured tree as fields.

## Implemented slice and continuation notes

FPF runs first, alongside the existing symbolic decoder. A whole-document match
bypasses that decoder, says so on the STOP row of the listing, and is read as
the object it captured by a template of its own. A non-match
retains existing hex, opcode and symbolic payload inspection. Those legacy
annotations are not FPF claims. This preserves the existing viewer while the
strict forms grow.

### How the productions are read

A pickle builds postfix. A tuple's elements are written before the opcode that
makes the tuple out of them, and a list is created empty and filled by the
opcodes after it, so a value is not a run of bytes that a reader can descend
into. Up to the fourth slice the recogniser descended anyway, and paid for it
by having to try a whole production and rewind: `EMPTY_LIST MEMOIZE` opens an
empty list and also a list of one item, and telling them apart meant parsing
the item and everything after it. That is exponential over a run of empty
containers, and it has no answer at all for `TUPLE2`, which is written after
two values that a descending reader has already handed back.

So a form now reads a file's productions against one bounded stack. Values
push; `TUPLE1` to `TUPLE3` fold the one to three things above them; `MARK`
remembers how tall the stack was, and `TUPLE`, `FROZENSET`, `APPENDS`,
`SETITEMS` and `ADDITEMS` fold everything since one. The pass is linear, the
work budget bounds it, and nothing backtracks except the two whole-production
alternatives a form adds to the basic ones (NumPy and the builtins calls),
which are tried where a string or a memo reference could stand.

This is not a general pickle stack machine and does not become one. Every
opcode is one a form enumerated; each checks that what it is folding is what
CPython writes in front of it, down to the length of a batch and the memo mark
after it; the memo holds only what a form spelled out itself; and anything
outside the list ends the match. Nothing is interpreted, nothing is skipped,
and a `REDUCE` or `BUILD` outside the fixed runs the forms name is a non-match
as before. Depth is bounded per value as the tree is built rather than by the
recursion that is no longer there.

Implemented forms. There are twenty-eight: seven families, each with one form
for protocols 4 and 5, one for protocols 2 and 3, one for protocol 1 and one
for protocol 0. The seven of a protocol range
are the same grammar over the same envelope, differing in which value
productions they allow; a file is read under the one form whose productions it
uses, and a file mixing two of them matches neither. The two ranges are named
apart because they are not the same instructions: see "Protocols 2 and 3"
below. The list that follows describes the protocol 4 and 5 forms, and that
section says what its neighbour does differently.

- `basic-p4-p5-v5`, the values a pickle writes as literals:
  - `NONE`, `NEWTRUE`, `NEWFALSE`, `BINFLOAT`.
  - Integers in the width CPython picks by range: `BININT1` for 0 to 255,
    `BININT2` for 256 to 65,535, `BININT` for the rest of a signed four-byte
    integer, and `LONG1` past that. A small number written in a wide field is
    a non-match. A `LONG1` run is two's complement, little-endian, and exactly
    as long as the number needs, which is what `save_long` writes, up to the
    255 bytes the opcode can declare. Sixteen bytes is as far as the reader's
    integer type reaches; past that the number is a node whose value is the
    decimal digits it comes to, worked out once as the form reads the run,
    with the run itself as a row beneath it. That is what a `uuid.UUID` is
    whenever its top bit is set, since a 128-bit number takes a seventeenth
    byte to say it is not negative, and what `2 ** 200` is. Protocols 0 and 1
    write the same number as a line of digits and it is read the same way.
  - UTF-8 strings and byte strings with 1, 4 and 8-byte lengths, each with the
    memo mark that follows it. A string CPython wrote with `surrogatepass`,
    and so is not UTF-8, is a non-match.
  - `BYTEARRAY8` at protocol 5, with the memo mark the C pickler files it
    under, or without it, which is what `pickle.py` wrote before Python 3.10.
    Below protocol 5 a bytearray is a call and belongs to the builtins form.
  - Tuples: `EMPTY_TUPLE`, which is the one container CPython does not
    memoise; `TUPLE1` to `TUPLE3` for one to three elements; and `MARK`
    .. `TUPLE` for four or more, each with its memo mark.
  - Lists, dictionaries and sets, created by `EMPTY_LIST`, `EMPTY_DICT` or
    `EMPTY_SET` and their memo mark, and filled the way CPython fills them.
    A list of one item is `APPEND` and a dictionary of one entry is
    `SETITEM`; everything longer is a run of batches of up to a thousand.
    Every batch but the last holds exactly a thousand. A dictionary's and a
    set's loop runs again whenever the batch it wrote was full, so a full
    batch of theirs is followed by another; a list's loop stops when it runs
    out, so a full batch of a list may be its last. What the two picklers do
    with the tail after a full batch differs, and both are read: see
    "Which pickler wrote it" below. `FROZENSET` over a `MARK` is here too,
    and is never batched, which is what `save_frozenset` does.
  - Dictionary keys and set members must be values Python can hash: numbers,
    strings, byte strings, tuples of hashable things, frozensets, the three
    singletons, a NumPy scalar, and, under the standard library form, a date, a
    time, a datetime, a span, a zone, a `Decimal`, a `Fraction` and a path. A
    key of any other kind is not something CPython could have been asked to
    write: a `Counter`, an `OrderedDict`, a `defaultdict` and a `deque` are
    mutable and Python refuses them as keys itself.
  - `BINGET` and `LONG_BINGET` where a value belongs, naming anything the
    basic productions built earlier: a string, a byte string, a tuple, a
    list, a dictionary, a set, a frozenset or a bytearray. This is how a list
    of records is written, every dictionary after the first naming its keys
    instead of spelling them again, and it is also how one list under several
    keys is written and how a list that holds itself is. A container is filed
    in the memo when it is created and before anything is put in it, so a
    name for one still being filled is ordinary rather than forward. A name
    may stand where a dictionary key belongs only when what it names hashes,
    which is decided where the thing was built.
- `numpy-array-p4-p5-v7`: the basic productions plus an array or a
  scalar, anywhere a value may stand, with at least one of them present.
  Matches exact `_reconstruct` and `scalar` sequences for
  `numpy._core.multiarray` and `numpy.core.multiarray`, and, at protocol 5
  only, the `_frombuffer` call for `numpy._core.numeric` and
  `numpy.core.numeric`, which is what NumPy writes there: the numbers travel
  as one of the call's arguments, a `BYTEARRAY8` when the array was writable
  and a byte string when it was not, followed by the dtype, the shape and the
  letter `C` or `F`. An array that is not laid out contiguously has no buffer
  to hand over and is written the protocol 4 way even at protocol 5, so both
  productions belong to one form. Supports plain
  b1/i1/i2/i4/i8/u1/u2/u4/u8/f2/f4/f8/c8/c16 dtypes, little/big endian for
  multibyte values, `|` for single-byte values, and explicit C/Fortran order.
  Dimensions use BININT1/BININT2/nonnegative BININT; shapes have 0 to 32
  dimensions with the exact EMPTY_TUPLE/TUPLE1/TUPLE2/TUPLE3/marked TUPLE
  production for their arity. Storage uses SHORT_BINBYTES/BINBYTES/BINBYTES8
  and must match shape times item size; a scalar's storage is one value wide.
  A structured dtype is read too: `V` and a width, with the column names as a
  tuple, the columns as a dictionary of each name against its own dtype and
  offset, and the width, alignment and flags after them. Every column has to
  fit inside a record and they have to be written in the order they sit in. A
  column's own dtype is a plain one, so the nesting is one level deep. Datetime
  and external-buffer dtypes/layouts fall back; an `O8` array is read here,
  by the pandas form and by the two joblib ones, which is where one turns up.
  See "An array of pickled objects" below.
  The class `_reconstruct` is handed is one of NumPy's own, enumerated, and a
  masked array is a production of its own: see the two sections below.
- `builtins-values-p4-p5-v3`: the basic productions plus the four builtins a
  pickle writes as a call rather than as a literal, with at least one present.
  `builtins.slice` of three integers or Nones, `builtins.range` of three
  integers, `builtins.complex` of two BINFLOATs, and `builtins.bytearray` of
  one byte string or of nothing at all, which is what an empty one is; each
  named by STACK_GLOBAL and called by REDUCE through the
  EMPTY_TUPLE/TUPLE1/TUPLE2/TUPLE3 its arity requires, and the empty tuple
  carries no memo mark where the counted ones do. `bytearray` is in this form
  only below protocol 5, which gave the type an opcode. No other module,
  callable or argument type is accepted. `set` and `frozenset` moved to the
  basic form, where they belong: protocol 4 writes both as literals.
- `stdlib-values-p4-p5-v2`: the standard library's own classes, which is what
  a pickle of ordinary program state is full of. Each is a `REDUCE` of one
  enumerated callable with the exact argument shape that callable is written
  with, and the list is in `familiar/stdlib.rs`:
  - `datetime.datetime`, `date` and `time`, each called with the one run of
    bytes `_getstate` packs it into, ten, four and six long, and a second
    argument holding the zone when the value is aware. The fields in the run
    are checked against what a calendar and a clock have: a year from 1 to
    9999, a month from 1 to 12, a day from 1 to 31, an hour under 24, a minute
    and a second under 60 and a microsecond under a million. From Python 3.6 a
    datetime and a time carry which side of a repeated hour they fell on, and
    `_getstate` writes that bit in the top of the month or the hour byte; it is
    written only from protocol 4, so below that the high bit is a month or an
    hour nothing has.
  - `datetime.timedelta` of three whole numbers, and `datetime.timezone` of one
    of those spans and, where the zone has one, its name.
  - `decimal.Decimal` of the text its own `str` writes, which is digits with at
    most one point and an exponent after them, or `NaN`, `sNaN` or `Infinity`
    with a sign in front. `fractions.Fraction` of two whole numbers or of the
    text `str` writes, which is `n/d` with a denominator that is never one and
    never negative.
  - `pathlib.PurePosixPath` and `PureWindowsPath`, under that module and under
    `pathlib._local`, called with however many words the path is made of.
  - `collections.Counter` of the mapping it holds, which is read as the
    counter rather than as an argument beside it.
  - `collections.OrderedDict`, `defaultdict` and `deque`, each created empty by
    the call and filled by the `SETITEMS` or `APPENDS` after it, which is how
    every other container in a pickle is built. A `defaultdict`'s factory is
    one of a short list of builtin classes, in both of the spellings
    `fix_imports` writes, or nothing at all; a class of the writing program's
    own is a non-match. The older releases wrote all three another way, handing
    the class everything it was to hold as one `iterable` argument, and both
    are read.
  - `time.struct_time` of nine whole numbers and `os.stat_result` of ten, each
    with a dictionary of the fields past the end of that run beside it, which
    is what `structseq_reduce` writes. Which fields are in that dictionary is
    the platform's: a `stat_result` carries `st_blocks` and `st_rdev` on Linux
    and does not on Windows, so the keys are words and the values are numbers,
    text or nothing. `time` and `os` are named for these two paths alone and
    are not modules a class may be named from: `os` holds `system` as well.
  - The builtin exception classes, each called with the arguments it was
    raised with, which is what `BaseException.__reduce__` hands a pickler, and
    with the instance dictionary after it when the exception carries one. The
    whole hierarchy is enumerated in `familiar/exceptions.rs`, sixty-nine
    names under `builtins` from protocol 3 up; below it `fix_imports` rewrites
    the forty-seven names Python 2 had to `exceptions` and leaves the other
    seven under `__builtin__`, and each spelling belongs to one side of
    protocol 3. The rewriting is lossy and the file is read as what it says: a
    `FileNotFoundError` written at protocol 2 says `exceptions.OSError`,
    because that is the class a Python 2 reading it would have got. What an
    exception was raised with is a message, so the arguments are numbers,
    words, byte strings, the singletons, containers of those, and other
    exceptions, which is what a group holds. An argument that is a class or an
    object of one is a non-match.
  - `uuid.UUID`, which needs no call at all: it is the plain object production
    with a 128-bit `int` in its state.
  - The builtins the builtins form reads, since a file holding a date and a
    complex number is one file. A file holding only builtins stays under
    `builtins-values-p4-p5-v3`, which is the form for it.
- The library forms, one per library, described in "The safety line for a
  library object" below: `sklearn-estimator-p4-p5-v1`,
  `scipy-sparse-p4-p5-v1` and `pandas-frame-p4-p5-v1`.

Each form accepts one protocol range and refuses the other's spellings. All
of them require STOP followed immediately by EOF. The matcher uses borrowed byte ranges, a 64-level bound on how deep a
captured value may be and on how many marks may be open at once, a bound of a
million memo slots and a million things on the stack, and a budget of a million
opcodes shared across all six attempts, so an alternative that failed still
cost what it cost. The budget was a hundred thousand up to the fourth slice and
could be raised because the pass no longer backtracks over a container: work is
now linear in the file, and the number is what a real file needs rather than
what a hostile one might cost. No Python or new runtime dependency was
introduced.

### The safety line for a library object

A pickle is never run, and a library form does not change that. What it adds is
a way to name a class, and a class is the one thing every earlier form refused,
so the line is written here and held in `familiar/object.rs`.

- **A class is named only by `STACK_GLOBAL`**, at protocol 4 or 5, and only
  when its module is the package the form lists or something under it:
  `sklearn` for `sklearn-estimator-p4-p5-v1`, `scipy.sparse` for
  `scipy-sparse-p4-p5-v1`, `pandas` for `pandas-frame-p4-p5-v1`. The package
  itself counts, because pandas 3.0 spells a frame's module `pandas` where 2.x
  spelled it `pandas.core.frame`; `sklearnish` does not. NumPy's classes are
  not on any of these lists: they are named only inside the NumPy productions'
  own fixed runs, by their exact spellings, so a NumPy global cannot arrive
  anywhere else. A `BINGET` may name a class the file named earlier, under the
  same rule.
- **An object is made only by `EMPTY_TUPLE NEWOBJ`**, which is
  `cls.__new__(cls)`, or by the `EMPTY_TUPLE EMPTY_DICT NEWOBJ_EX` Python 3.4
  writes for the same thing. A `NEWOBJ` handed arguments, or a `NEWOBJ_EX`
  handed arguments of either kind, is a class being told to construct itself
  out of values, and what those mean belongs to the class. The one way a
  `NEWOBJ` with arguments is read is as one of the enumerated calls, closed by
  that opcode rather than by `REDUCE`, which is the `Via::NewObj` column of the
  calls table: a class with no `__reduce__` of its own that does take arguments
  is written that way, and `torch.Size` was one in torch 1.0 and older. The row names
  the path and checks the arguments, so it is the same enumerated set as every
  other call and not a wider door into NEWOBJ.
- **A global a form names and never calls is enumerated by its whole dotted
  path**, which is the `names` column of a form's row. `builtins` is not a
  package any class at all may be named from, so the eight classes a
  `collections.defaultdict` may be handed as its factory are written out one by
  one in both spellings, and `__builtin__.object` is there because every
  `copy_reg._reconstructor` call is handed it.
- **State arrives only by `BUILD`**, of a dictionary of attribute names or of
  the tuple a class with a `__setstate__` of its own is handed. The attribute
  names are data; the instruction shape is fixed. State values are what the
  other productions already read: basic values, NumPy arrays and scalars,
  nested objects of the same kind.
- **`REDUCE` is accepted only of an enumerated callable**, by its whole dotted
  path, with the exact argument shape that callable is written with. Never a
  `REDUCE` of whatever global happens to sit under a whitelisted module: that
  is how a pickle exploit is written, and a module whitelist can name callables
  that do work when they are called. Deciding which of a library's thousand
  functions are safe is not this reader's job, so it decides nothing and reads
  a list instead.

The list, as implemented:

| Form | Callables a REDUCE may name |
| --- | --- |
| every form with NumPy | `numpy._core.multiarray._reconstruct`, `numpy.core.multiarray._reconstruct`, `numpy._core.multiarray.scalar`, `numpy.core.multiarray.scalar`, `numpy._core.numeric._frombuffer`, `numpy.core.numeric._frombuffer`, `numpy.dtype` |
| `builtins-values-p4-p5-v3` | `builtins.slice`, `builtins.range`, `builtins.complex`, `builtins.bytearray` |
| `stdlib-values-p4-p5-v2` | `datetime.datetime` / `date` / `time` / `timedelta` / `timezone`, `decimal.Decimal`, `fractions.Fraction`, `pathlib.PurePosixPath` / `PureWindowsPath` under `pathlib` and `pathlib._local`, `collections.Counter` / `OrderedDict` / `defaultdict` / `deque`, `time.struct_time`, `os.stat_result`, and the builtin exception classes in `familiar/exceptions.rs` under `builtins`, `exceptions` and `__builtin__` |
| `sklearn-estimator-p4-p5-v1` | the NumPy ones, and, all of them in `familiar/sklearn.rs`: `sklearn.tree._tree.Tree` of a number, an array and a number; `newObj` of one class, under `sklearn.neighbors._kd_tree`, `sklearn.neighbors._ball_tree` and `sklearn.metrics._dist_metrics`, which is `cls.__new__(cls)` written as a function because a Cython class has no `__new__` a pickle can reach; the ten `sklearn._loss._loss.Cy*` losses and the five `sklearn.linear_model._sgd_fast` ones, each of nothing or of the one float it was configured with; and what `random_state` holds after a fit, which is NumPy's: `numpy.random._pickle.__bit_generator_ctor` of a class or its name, `__randomstate_ctor` and `__generator_ctor` of what that made, and `numpy.random.bit_generator.__pyx_unpickle_SeedSequence` of a class, a checksum and None. The globals it may name and never calls are the twenty-seven NumPy scalar types, the five bit generators and the seed sequence, which are `numpy::TYPE_NAMES` |
| `scipy-sparse-p4-p5-v1` | the NumPy ones |
| `torch-tensors-p4-p5-v1` | `collections.OrderedDict`, `torch.Size` of a tuple, `torch.device` of a word and an optional index, `torch.serialization._get_layout` of `torch.sparse_coo`, `torch._utils._rebuild_sparse_tensor` of a layout and two tensors, and `torch.nn.backends.thnn._get_thnn_function_backend` of nothing, which is the backend torch 1.0 and older gave every module. The calls that rebuild a tensor are matched inside their own fixed runs in `familiar/torch.rs`, not through this list, and so is `torch.nn.parameter.Parameter` of a tensor and a flag, which is how torch 0.4 wrote a parameter. The classes it may name are those under `torch.nn`, which is a whole module saved as an object; the dtypes it may name and never calls are the twenty in `torch::DTYPE_NAMES`. |
| `pandas-frame-p4-p5-v1` | the NumPy ones, `builtins.slice`, `pandas.core.internals.managers.BlockManager` of a tuple of blocks and a list of axes, `pandas._libs.internals._unpickle_block` of values, a slice and a number, `pandas.core.indexes.base._new_Index` and `pandas.core.indexes.datetimes._new_DatetimeIndex` of a class and a dictionary, `pandas._libs.arrays.__pyx_unpickle_NDArrayBacked` of a class, a number and None, `pandas.StringDtype` / `pandas.core.arrays.string_.StringDtype` of a word and a float, `pandas._libs.tslibs.offsets.Day` of a number and a flag, `functools.partial` of the one global below, and `pandas.core.internals.blocks.new_block` through that partial |

**The one exception, written down beside the rule it is an exception to.**
`REDUCE` names a global, and a global is what `STACK_GLOBAL` made. pandas 1.3
writes a block the other way: `functools.partial` over
`pandas.core.internals.blocks.new_block`, and then a `REDUCE` whose callable is
what that `REDUCE` made. The form accepts that, and only that. The partial may
be made over one global and no other, only what that partial made may be
called, and both halves are checked at both ends. So what may happen is still a
list of two entries rather than a rule about calls of calls, and since nothing
is run it is as safe as any other enumerated run of bytes: it is a fixed
sequence, matched exactly, that a reader recognises rather than executes. A
`functools.partial` over anything else is a non-match, which the tests in
`familiar/tests/pandas.rs` hold it to.

The NumPy and builtins calls are matched inside their own fixed runs rather
than through this list, which is why a failed NumPy production cannot be read a
second, looser way: the loop's `STACK_GLOBAL` will not bind a global the form
has not listed, so the file ends as a non-match rather than as a different
reading of the same bytes.

Three forms and one grammar. Each is the plain object production over one
package with that library's calls beside it, so a file is read under the form
for the library that wrote it and no other, and a pandas call cannot appear in
a scikit-learn file. The form IDs are per library because that is what a reader
compares against.

What the library forms read, and what they do not:

- **scikit-learn.** An estimator is a class under `sklearn`, made with no
  arguments, with a dictionary of attributes. The attribute names change
  between releases, so the form fixes the instructions and reads the names as
  data. A decision tree also holds a `sklearn.tree._tree.Tree`, built by
  `REDUCE` from how many features, classes and outputs it was fitted on and
  handed its arrays by the `BUILD` after it; its nodes are a structured array.
  A pipeline and a random forest hold estimators inside estimators, and an
  attribute may name an object the file made earlier, which is what a forest's
  `estimator_` is.
- **scipy.** A sparse matrix is the same production under `scipy.sparse`: a
  dictionary holding `data`, `indices`, `indptr` and a shape tuple. The class
  moved from `scipy.sparse.csr` to `scipy.sparse._csr`, which is data.
- **pandas, every release in the corpus.** A frame is a `BlockManager` and a series a
  `SingleBlockManager`. 1.1 hands the manager its axes, its blocks and the
  dictionary it versions them with as one tuple through `NEWOBJ` and `BUILD`;
  1.5 and up call `_unpickle_block` once a block. A series keeps the older
  spelling in every release, so only the frames divide. An axis is
  `_new_Index` of a class and a dictionary: `Index` over an array of names,
  `RangeIndex` as start, stop and step. A categorical and, in 3.0, a text
  column are `__pyx_unpickle_NDArrayBacked` around an array, finished by a
  `BUILD` of a tuple.
- **An index of dates** is an `M8` dtype, whose state is nine long rather than
  eight: it is version 4 rather than 3 and ends with the unit it counts in,
  written as `('ns', 1, 1, 1)` beside an empty dictionary in NumPy 1.x and
  beside `None` in 2.x. Both are read, and every unit NumPy has down to the
  nanosecond. A count of three days, which is a numerator other than one, is a
  non-match, and so is a unit finer than a nanosecond.
- **A block placed by an array is a non-match.** pandas writes an array of
  column positions instead of a slice when a block's columns are not next to
  each other, and no file in the corpus does.
- **A frame made from another names what the two share.** `assign` and a
  shallow copy leave the second frame holding the first one's index, the
  slices that place its blocks, and the array and dtype under a column of
  text, and pickle writes a shared object once. So the second frame's
  placement is a `BINGET` where the first wrote `builtins.slice`, and the
  same for the axis and the array. The form takes a name for a slice where it
  takes a slice, and the table follows each name to where the file wrote the
  thing, which is in the first frame and nowhere under the second:
  `made_at` in `eval/pickleparts.rs` looks from the top of the match, walking
  only into values whose span holds the offset. Two frames built separately
  share nothing and spell everything twice. The sample is
  `mixed-frames-sharing-placements-p4.pickle` and `-p2`.
- **An array of objects holds whatever the file's form allows**, in a list of
  up to a thousand a batch. A column of dates, of lists or of exact numbers is
  read the way the same values are read anywhere else in the file. An array of objects can hold
  whatever was pickled into it, and only what a form has written down is read.

### Extensions compose: the mixed form

Every form reads the basic grammar. A form for a library reads that and one
thing more, and what a file used beyond the basic grammar is its **extensions**:
the standard library's classes, NumPy's arrays, pandas, scikit-learn, scipy,
torch, and the builtins written as calls. The `form extensions` row names them.

A real pickle mixes libraries. A saved model comes with the day it was fitted
and the score it got; a frame comes with a note beside it; a dictionary of
arrays is an `OrderedDict` because the order mattered. Every one of those was a
non-match while a form allowed one extension, because each form requires the
file to have used its own productions and refuses everything else's: a date
beside an array is refused by the NumPy form at the date and by the standard
library's form at the array.

So there is one more form per protocol range, `mixed-values-p4-p5-v2` and its
three lower names, whose class prefixes, named globals and enumerated calls are
the **union** of every family's. It is built from the same `DECLARED` table the
families are declared in, in `familiar/forms.rs`, so a family added there is in
the union with no second edit.

**The safety line does not move.** A union of enumerated sets is an enumerated
set. Every production is exactly as strict under this form as it is alone: the
same argument shape for each callable, the same fixed runs, the same refusal of
a `REDUCE` of any global a form did not name. Three things make that true rather
than hopeful:

- **No two families name the same callable.** The paths in the table above are
  distinct across families, so no callable's argument shape is the union of two
  shapes. Where one family names a callable twice, as `datetime.datetime` is
  named aware and naive, both rows were already tried in turn and still are.
- **The class prefixes are disjoint**: `sklearn`, `scipy.sparse`, `pandas` and
  the eight standard library modules. NumPy's classes are still on no list at
  all, here least of all: they are named only inside the NumPy productions' own
  fixed runs, which reach `Cursor::global` with the modules written out and
  never consult the form's class list.
- **`builtins` and `__builtin__` are still not packages a class may come from.**
  They are reachable as the named globals a `defaultdict` may be handed as its
  factory, which is a short list of builtin types the reader never calls, and as
  the set and frozenset calls the protocol adds below 4.

**What the form requires.** A form that allows a production requires the file to
use it, and the mixture is what this one is for, so it requires the file to have
used **two or more** extensions. That is what keeps every existing verdict: the
widest thing the union allows that no single form does is an array of pickled
objects, which pandas needs for an index of column names, and a file of nothing
but one of those is one extension and stays a non-match. It is also why
the name is true of the file rather than true only because the form was tried
last.

**It is tried last**, after every single-extension form at its protocol range,
so a file of one extension keeps the name it already had. A list of two pandas frames
uses the builtins, NumPy and pandas productions and would satisfy the mixed
form's requirement; it matches `pandas-frame-p4-p5-v1` because that form was
tried first and is the narrower claim.

**joblib is allowed and not required.** The two joblib forms are *for* what
`joblib.dump` writes and hold the file to having a wrapper in it. The mixed form
only permits the wrapper, since something else is what made the file mixed. So a
joblib file holding dates, a pandas frame or a scipy sparse matrix is read here,
and one holding arrays or scikit-learn estimators alone keeps its own name.

**The `form extensions` row.** The header of a matched file says `form` and
then `form extensions`: the basic grammar is the form, and these are what the
file used beside it. A file the basic grammar read on its own says `none`.
What the row names is which of them the file turned out to hold, in a fixed
order. The basic grammar is not among them: every form reads it, so it is the
form itself rather than an extension of it, and the `form` row above already
says which form that is. `stdlib, numpy` for a date beside an array;
`builtins, stdlib, numpy, pandas` for a frame with a note and a date, since
pandas places a block by writing a `slice`. It is on every matched file
and not only a mixed one, for the reason the `pickler` row is: the form names a
grammar and this names what the file used, and a reader comparing two files
wants to see the same rows in both. It is worked out from the match and has no
bytes of its own.

Everything downstream reads a mixed file where the values sit rather than at the
root. A frame inside a dictionary offers the same table and the same summary
rows it offers at the root, because both are asked of the node rather than of
the file; the same goes for an array beside a date and for a list of records.

### Protocols 2 and 3

Protocol 3 is what `pickle.dump` wrote by default from Python 3.0 to 3.7, and
protocol 2 is what Python 2 wrote whenever it was asked for the highest
protocol it had and what Python 3 wrote for Python 2 to read. Between them they
are most of the pickles in the world older than 2020, so there is a form for
each family at those protocols: `basic-p2-p3-v1`, `numpy-array-p2-p3-v2`,
`builtins-values-p2-p3-v1`, `sklearn-estimator-p2-p3-v1`,
`scipy-sparse-p2-p3-v1` and `pandas-frame-p2-p3-v1`.

They are separate identifiers rather than a wider protocol range on the six
above, because a form identifier names a grammar and these are not the same
instructions. What differs:

- **A memo mark is `BINPUT` or `LONG_BINPUT` with the slot number in it**,
  where protocol 4 writes `MEMOIZE` and no number. The number is not believed:
  a slot is the count of marks written before it, so the index has to be
  exactly that. See "The memo, and what a reference may name" below for the
  one pickler that numbers from one and the one that leaves a mark out.
- **Text is `BINUNICODE` and nothing else**, whose length is four bytes however
  short the text is. `SHORT_BINUNICODE` and `BINUNICODE8` arrived with
  protocol 4 and are a non-match here, as `BINUNICODE` in its protocol 4
  places is there.
- **A callable is `GLOBAL`**, one opcode and two newline-terminated lines,
  filed in one memo slot rather than three. `STACK_GLOBAL` is protocol 4's.
  The safety line is the same one: the module has to be one the form lists or
  the whole path one of the callables it enumerated. The two lines sit inside
  the opcode, so the node carries the dotted path and no parts.
- **A byte string at protocol 2 is a call**, `_codecs.encode(text, 'latin1')`,
  because protocol 2 has no opcode for one. The run in the file is that text
  written UTF-8: a byte under 0x80 is itself and every byte above it is two.
  So the bytes are not in the file as bytes, and a value that holds them says
  which it is rather than pretending the run is the value. An empty byte
  string is `bytes()` called with nothing, which is the one spelling it has.
  Protocol 3 has `SHORT_BINBYTES` and `BINBYTES` and writes neither call.
- **A set and a frozenset are calls of the class** over a list of the members,
  which is what `set.__reduce__` returns; PyPy hands over a tuple instead, and
  both are read. `EMPTY_SET`, `ADDITEMS` and `FROZENSET` are protocol 4's. So
  the basic form names four classes below protocol 4: `set`, `frozenset` and
  `bytes`, under the names `fix_imports` writes at each protocol, and
  `_codecs.encode`. It names them so that they can be called: a class that
  reached the tree as a value is a non-match.
- **Python 2's own spellings.** `SHORT_BINSTRING` and `BINSTRING` are a
  Python 2 `str`, which is a run of bytes that was usually text; the opcode
  listing reads one as text of an encoding nobody declared, and so does the
  form: text when the bytes are UTF-8 and a byte string when they are not.
  `INT` and a line of digits is an `int` too wide for `BININT`, which is a
  text opcode inside a binary protocol; Python 2's `int` was a machine word,
  so the line covers the range between a four-byte and an eight-byte integer
  and nothing else. `LONG1` is a `long`, written as it is at protocol 4.
- **`fix_imports` renames the builtins.** Below protocol 3 the module is
  `__builtin__` and `range` is `xrange`; from protocol 3 up they are
  `builtins` and `range`. Each spelling belongs to its own protocol and a file
  using the other's is a non-match.
- **There is no framing.** A `FRAME` header is protocol 4's, and the byte
  0x95 below it is not an opcode.
- **A NumPy array's numbers reach protocol 2 as latin-1 text**, through the
  same `_codecs.encode` call, and the length checked against the shape is the
  decoded length. That length is the only thing worked out while the form
  matches; the numbers themselves come of opening the run.
  - The `numbers` row is the run in the file, which is text, and it opens as a
    space of its own: its type is `latin-1 text`, and the one thing inside it
    is the numbers, as ordinary typed fields at ordinary offsets in that
    space. So a value has an address, the ordinary table applies, the ordinary
    hex view of the space shows the bytes, and nothing is copied or computed.
    A protocol 2 frame, series and array open as the same table, cell for
    cell, as the same object at protocol 4.
  - The node says what it was decoded from, so there is no `written as` row
    for it. That row is left for the one thing the type cannot say: an array
    naming the run an earlier array wrote says `bytes an earlier array wrote`.
  - The opcode listing is not told to read the run as an array: it is a
    `BINUNICODE` and reading it as numbers would be reading the spelling.
  - The table hangs on the numbers rather than on the run, since the run holds
    one thing and a table over it would be one row. It names no columns: an
    array's columns are places along an axis rather than names anything wrote
    down, and the view heads them the way it heads every other run of numbers.
  - An array Python 2 wrote is not this. Python 2 had a type for a run of
    bytes, its `str`, so the numbers go out as `SHORT_BINSTRING` or
    `BINSTRING` and are the bytes they are, with no space and the ordinary
    table over them. No file in the corpus is one, since none of the
    Python 2 environments has NumPy, so that is a branch with a test written
    to it and no sample behind it.

Everything else is the same production. scikit-learn, scipy and pandas write
`GLOBAL`, `NEWOBJ` and `BUILD` below protocol 4 exactly as they write
`STACK_GLOBAL`, `NEWOBJ` and `BUILD` above it, so the library forms needed no
new structure: `copyreg._reconstructor` does not appear in any file in the
corpus.

### Protocol 1

Protocol 1 is what Python 2 wrote when it was asked for a binary pickle before
protocol 2 existed, and what `cPickle.dump(obj, f, 1)` wrote for years after.
There is a form for each family at it: `basic-p1-v1`, `numpy-array-p1-v2`,
`builtins-values-p1-v1`, `sklearn-estimator-p1-v1`, `scipy-sparse-p1-v1` and
`pandas-frame-p1-v1`.

It is the protocol 2 grammar with five things different:

- **There is no `PROTO` opener.** A file starts at its first value, so the
  envelope has nothing in it and the header's `protocol` row is worked out
  rather than read: the row says `1`, and what says so is the form that
  matched, since the file carries no such byte. A file using binary opcodes
  with no opener is protocol 1; one using none of them is protocol 0.
- **`True` and `False` are the integer lines `I01` and `I00`**, which are text
  opcodes inside a binary protocol and are not the integers one and nought:
  those are `I1` and `I0` where they are written as lines at all, and the
  leading zero is the whole of the difference. `NEWTRUE` and `NEWFALSE`
  arrived with protocol 2 and are a non-match here.
- **There is one tuple opcode.** `TUPLE1` to `TUPLE3` arrived with protocol 2,
  so every tuple but the empty one is a `MARK` and a `TUPLE`, and a fixed run
  that writes a tuple of a known length opens it before its elements rather
  than closing it after them. `EMPTY_TUPLE` is protocol 1's already.
- **A `long` is a line of digits** ending in the `L` Python 2 spelled one
  with, which Python 3 writes too. `LONG1` is protocol 2's.
- **An object is made by `copy_reg._reconstructor`**, which is
  `cls.__new__(cls)` written the long way round: the class, the class it
  inherits its layout from, and the argument that base is constructed with.
  Only `object` and `None` are accepted, which is the one shape the NEWOBJ
  production already takes; a different base is a class being told to
  construct itself out of values, and what those mean belongs to the class.
  `object` is the one class a form may name and never call.

Everything else is the protocol 2 production, batching and all: `APPENDS` and
`SETITEMS` are protocol 1's, and so are `BINPUT`, `BINGET`, `BINUNICODE`,
`SHORT_BINSTRING` and the `_codecs.encode` call a byte string goes through.

### Protocol 0

Protocol 0 is the text protocol: what Python wrote by default until Python 3.0
and what `pickle.dumps(obj)` gave anyone who never named one. There is a form
for each family at it: `basic-p0-v1`, `numpy-array-p0-v2`,
`builtins-values-p0-v1`, `sklearn-estimator-p0-v1`, `scipy-sparse-p0-v1` and
`pandas-frame-p0-v1`. The reading is the same bounded byte cursor and the same
stack; what changes is that every value is an opcode and a line.

- **A line that spells its value is a node, not a leaf.** A `V` or an `S` line
  whose run holds no escape and nothing above 0x7f is what it is: a text leaf
  over exactly that run, read by the machinery every other field uses, which
  is most keys and most names. A line that does hold one is a node whose value
  is the thing it spells, worked out once as the form reads it, with a single
  `line` row under it holding the run as the file wrote it. So the row reads as
  what the string is, the bytes stay where they are and stay editable as what
  they are, and nothing on screen claims the spelling is the value.
- **The escapes are exactly the ones a pickler writes.** A `V` line is
  `raw-unicode-escape`: `\uXXXX` and `\UXXXXXXXX`, in lower-case
  hexadecimal, and every byte under 0x100 as itself. Before that `save_str`
  replaces the backslash, the newline, the carriage return, the NUL and the DOS
  end-of-file with their own `\uXXXX` (Python 2 replaced the first two), so a
  backslash in the line always opens an escape and one that does not is a file
  no pickler wrote. An `S` line is Python 2's `repr` of a `str`: the quote
  `repr` chose, and inside it `\\`, `\'`, `\"`, `\n`, `\r`, `\t` and
  `\xNN` and nothing else.
- **Numbers are spelled, not packed.** `L` is every integer Python 3 writes at
  this protocol, small ones included, with the `L` Python 2 spelled a `long`
  with; `I` is a Python 2 `int`, and `I01` and `I00` are the two singletons.
  `F` is a float, in either of the two spellings a pickler writes: `repr` from
  the pure picklers and from every C pickler since Python 3.6, and `%.17g`
  from Python 2's `cPickle` and Python 3.4's C one, which drops the point from
  a whole number so that `3.0` goes out as `3`. Both are read, and neither
  says which pickler wrote the file: the difference is the release. A leading
  zero, a plus sign, an exponent without its sign or a digit `repr` would not
  have written is a non-match.
- **The memo marks are `PUT` and `GET` lines.** The numbering rule is the one
  protocols 1 to 3 hold to, and Python 2's `cPickle` numbers from one here as
  well, which the corpus confirms.
- **Containers are made over a `MARK`**: `(d` for a dictionary, `(l` for a
  list, `(t` for the empty tuple, each filled one entry at a time. There is no
  batching at protocol 0, which the corpus confirms: 26,712 `APPEND` and
  25,912 `SETITEM` against no `APPENDS` or `SETITEMS` at all. So the tell that
  says which pickler wrote a file is not there, and a protocol 0 file says
  nothing either way unless its memo numbers from one.
- **An array's numbers are two layers deep.** They reach the file as
  `_codecs.encode` of a `V` line, so the escaping comes off first and the
  latin-1 after it. The run opens as a space through both layers at once and
  its type reads `latin-1 text, escaped`, exactly as protocol 2's opens
  through one, so a frame, a series and an array at protocol 0 open as the
  same table, cell for cell, as the same object at protocol 5.
- **A line that spells its text is read again rather than kept.** What a `V`
  or `S` line stands for is worked out from the run whenever something wants
  it: the run is in the file, and a copy of every escaped string in a file is
  a copy nothing needs. `familiar::spelled` is the one reading of a line, and
  `codec::pytext` is the one reading of the escaping under it, which is what
  lets a node open a protocol 0 array's numbers with the same decoder the
  recogniser measured them with.
- **A text named where the file wrote it stays a reference.** GraalPy hands
  back one object for two equal strings, so the second `_codecs.encode` of the
  same packed run is a `BINGET`. The value is the reference, not the text: its
  row is the two bytes the file wrote, with a `refers to` row saying what is at
  the other end, and the run is read and counted once, under the value that
  spelled it. At protocol 0 the run it names is a line, and a line is not
  always the text it stands for, so `lines::is_named_text` is the one rule that
  says which, by the protocol and the run, and the recogniser and the reading
  both ask it. Two dates of the same day in one file used to be the one thing
  in the matrix no form read.

### NumPy's own array classes

`_reconstruct` is handed `self.__class__`, so the class written in it is the
array's own and a class anyone defined can reach that argument. Four of
NumPy's are read and nothing else is, each by its whole dotted path:
`numpy.ndarray`, which is nearly every array; `numpy.matrix`, which is an
array held to two dimensions; `numpy.memmap`, which is an array a reader
may keep in a file and which `__reduce__` pickles with its numbers like any
other; and `numpy.recarray`, spelled `numpy.rec.recarray` from NumPy 2, which
is a structured array whose columns are attributes as well as columns. None of
the four changes how the numbers are read, so all four are the same array with
a different word on the node: a matrix says `matrix` where an ndarray says
`array`, and the dtype, the shape, the order and the table are what they were.
The joblib wrapper's `subclass` key reads the same list.

A class from outside NumPy is a non-match. What such a class does to an array
when it is rebuilt is that class's business, and a reader shown the numbers
under its name would be shown something the file does not say.

**A record array's dtype is the one dtype written as a class.** Every other
dtype is `numpy.dtype('<f8', False, True)` with the letters it is spelled by;
a record array's is `numpy.dtype(numpy.record, False, True)`, the class
itself. Both NumPy 1.26 and 2.5 write that class under `numpy`, which is what
`record.__module__` says whichever release made it, even though the array
class beside it moved from `numpy` to `numpy.rec` between the two. The state
the BUILD hands the dtype is an ordinary structured dtype's, so the column
names, types and offsets are read exactly as they are for the record array
scikit-learn writes its tree of nodes as; what the letters would have said,
the width, is in that state alone and is read from there. A `recarray` whose
dtype is not a record is a non-match: `numpy.rec.array` builds the dtype out
of the columns it is given, and an array of plain numbers viewed as the class
has not been measured. The table is the structured array's table, columns by
name.

### A masked array is two arrays and a fill value

`numpy.ma.MaskedArray` is rebuilt by
`numpy.ma.core._mareconstruct(MaskedArray, ndarray, (0,), 'b')`, where the
last argument is a text and not the byte string an ordinary array's
placeholder is. The BUILD after it is handed a seven-part tuple:
`__getstate__` in `numpy/ma/core.py` is
`data_state + (getmaskarray(self).tobytes(cf), self._fill_value)`, so the
first five parts are an ordinary array's version, shape, dtype, storage order
and numbers, and the two after them are the mask and the fill.

The node is typed `masked array` and holds four rows: the run of instructions
that rebuilt it, `data`, `mask` and `fill value`. `data` and `mask` are each
an array in their own right, with the state's shape and order and with the
state's dtype on one and the mask's own dtype on the other, which is what
`make_mask_descr` makes of the state's: `|b1` for a plain dtype, and for a
structured one a record of the same column names with one boolean apiece and
nothing between them. So each has its own run, its own address and its own
table, and at protocols 0 to 2 each opens its numbers as a space of its own
the way every other array does.

The node's own table is the numbers, chunked into rows the way an array's
table chunks one, with the entries the mask hides shown empty. Such a cell
keeps the address of the bytes it would have read: the number is in the file
and the array says not to count it, which is a different nothing from a value
the file does not hold, so a reader can still click through to those bytes and
the cell's hover says `Masked`, with or without the address columns. The data
and the mask each also have their own ordinary table, under their own rows.

Three things are worth knowing about what NumPy writes:

- **The mask is always written out.** `getmaskarray` makes one for an array
  whose own mask is the `nomask` singleton, so a file holding a masked array
  with nothing hidden still holds a run of noughts as long as the numbers are.
- **The fill value is `None` or an array of no dimensions.** `None` is what an
  array that kept NumPy's default for its dtype writes, and the pickle does
  not say what that default came to.
- **A structured dtype masks a column of a row.** `make_mask_descr` gives such
  an array a mask of one boolean per column rather than one per entry, so the
  two runs are different widths: a record of an `i4` and an `f8` is sixteen
  bytes with its second column eight in, and its mask is two bytes with its
  second column one in. The node's table is then the named columns of the
  record, one row per record, and a cell is blank where the mask hides that
  column of that row. An array of pickled objects is still refused: there is
  no run of bytes for a mask to be laid over.

### What stays a non-match, and why

- **A name pointing at a slot no form could say anything about.** The memo
  still holds `Opaque` for the intermediate values inside a NumPy or builtins
  call, such as the tuple of arguments it was handed, and a `BINGET` naming
  one of those is a non-match as before. What a form built itself may be
  named; what it only matched its way past may not. What a builtins call made
  is something the form built, so a slice, a range, a complex and a bytearray
  may be named again like any other value.
- **A container written with `POP` or `POP_MARK`.** A tuple that holds itself
  cannot be built postfix, so CPython writes the elements, throws them away
  and names the tuple the recursion already made. No form accepts either
  opcode, so that file is a non-match.
- **A class the file names**, unless its module is one a library form lists.
  See "The safety line for a library object" above: the rule did not go, it
  was written down.
- **`LONG4`**, which CPython writes only past 2^2040. A `LONG1` declares up to
  255 bytes and every width of it is read; nothing goes wider.
- **A string that is not UTF-8**, which `surrogatepass` lets through.
- **A file spelled by both picklers at once.** Each spelling below is one a
  real pickler writes, and a file was written by one pickler, so a file
  showing the C one at one batch edge and `pickle.py`'s at another was
  written by neither.
- **A line no pickler wrote**: an escape outside the two lists above, a
  backslash that opens none, a `STRING` line whose quotes do not match or do
  not close, a number with a leading zero or a plus in front of it, and a
  float spelled a way neither `repr` nor `%.17g` spells one.
- **A class the file named and did not call**, under a form that names no
  classes. Below protocol 4 a set and a byte string are calls, so the basic
  form names four classes, and it names them so that they can be called. A
  class that reached the tree as a value rather than being folded away by its
  call is a class the reader would be shown as data.
- **An `INT` line that is not what Python 2 wrote**: a number a four-byte
  BININT holds, a leading zero, a plus sign, a space, or anything past an
  eight-byte integer. Python 2's `int` was a machine word, so the line covers
  exactly the range between the two.
- **`_codecs.encode` under any encoding but `latin1`**, which is a byte string
  this cannot read back, and a `latin1` run holding a character above 0x100,
  which is a text no pickler put there: every byte of the original became the
  character of the same number.
- **A `long` that fits a four-byte integer**, which Python 2 wrote as LONG1
  where Python 3 writes BININT. The rule that a small number in a wide field
  is a non-match is the protocol 4 one, kept as it is: no file in the corpus
  has one, so widening it would be widening on a guess.

### Which pickler wrote it

CPython ships two picklers. `_pickle` is the C one, which `pickle.dump` uses
wherever it imports; `pickle.py` is the pure Python one beside it, reachable as
`pickle._Pickler`, and the only one PyPy 3 has. Python 2 had a third,
`cPickle`, which is a different program from `_pickle`; PyPy 2.7 ships a
Python copy of it under the same name. All of them are ordinary, so a form
reads any of them, and the match says which the file shows. The row is
`pickler` in the header of the familiar-form template, beside `message` and
`form`, and since 2026-09-19 it says one of exactly seven things:

- `any (every known pickler writes this data the same way)`
- `_pickle (CPython's in C, or GraalPy's in Java)`
- `pickle.py (pure Python), or an IronPython pickler (C#)`
- `cPickle (Python 2's in C, PyPy 2.7's in Python, or Jython's in Java)`
- `cPickle (Jython's, in Java)`
- `cPickle (IronPython's, in C#)`
- `_pickle (GraalPy's, in Java)`

One pattern throughout: the module's name, then whose it is and what it is
written in.

Seven programs write the pickles in the collection, and the seven statements
above are not those seven programs: each is a spelling, and it names every
program that writes it. A reading sharpens rather than switching, which is
what `picklers.rs` holds: a memo numbered from 1 was written by one of the
three `cPickle`s, and a batch of 1,024 in the same file narrows that to
Jython's. Two readings neither of which is a case of the other are a file no
single pickler wrote, and a non-match. `HANDOVER-pickle-libraries.md` has each
tell and where in that pickler's source it lives, under "Four more
interpreters".

The first four strings were widened on 2026-09-19 and the last three are new.
The old wordings each claimed something the bytes do not say: `the only one
PyPy 3 has` was already false of Python 2's own `pickle.py` files in the
collection, and `_pickle or pickle.py (they write this data identically)`
named CPython's two picklers for a file that may have come from any of the
seven.

What does **not** reach this row is the interpreter. Jython spelling a
protocol 0 escape in upper case, GraalPy naming `_collections`, IronPython
handing `datetime` its fields rather than its packed bytes: each of those is
the runtime's own classes and text routines, and both of that interpreter's
picklers write it. Those are read as alternative spellings, the way
`numpy._core` is read beside `numpy.core`.

The last is most files: the two Python 3 picklers agree everywhere but the
tail of a container longer than a batch and the memo mark after a bytearray,
and a file with neither says nothing either way. The third is every file whose
memo numbers from 1, batch edge or no batch edge, and it names the module
rather than which of its two implementations wrote the file: they number the
memo the same way. None of this is in the form name, because the form is the
same grammar either way.

The tells, measured across the whole `pickle-matrix` corpus at protocols 1 to
5 and checked against `_batch_appends`, `_batch_setitems` and `save_set` in
`pickle.py` and `batch_list_exact`, `batch_dict_exact` and `save_set` in
`Modules/_pickle.c`:

| After a full batch of a thousand | `_pickle` | `pickle.py` |
| --- | --- | --- |
| a list with one item left | `MARK x APPENDS` | `x APPEND` |
| a dictionary with one entry left | `MARK k v SETITEMS` | `k v SETITEM` |
| a set with one member left | `MARK x ADDITEMS` | the same |
| a list with nothing left | nothing | nothing |
| a dictionary or set with nothing left | `MARK SETITEMS` / `MARK ADDITEMS` | nothing |

A list's loop asks whether it has reached the end, so both picklers stop after
a full batch that finished the list; a dictionary's and a set's loop runs again
whenever the batch it wrote was full, and the C one therefore writes the empty
batch that `pickle.py` skips. At protocol 4 and 5 neither has a shorthand for a
set of one, so a set says nothing about its writer; below protocol 4 a set is a
call over a list, and the list ends the way that pickler ends a list, so there
a set does say.

Python 2 has two picklers of its own, `pickle` and `cPickle`, and they are told
apart by the memo rather than by a batch edge: `cPickle` numbers its first slot
1 where every other pickler numbers it 0, and it leaves the mark out for a value
nothing else in the program holds a reference to. So a file numbering from one
is `cPickle`'s whether or not a long container is in it. Jython's numbers from
one as well, so that on its own no longer names one program: the row says
`cPickle (Python 2's in C, PyPy 2.7's in Python, or Jython's in Java)` and a
second spelling narrows it.

`cPickle` also does not spell its batch tails the way the other C pickler does.
CPython's walks a list through an iterator, the way `pickle.py` does, and a
dictionary the way the C picklers do, so its lists end one way and its
dictionaries the other; PyPy's is a Python copy that walks both the `pickle.py`
way. So under that numbering a list ends one way only, a dictionary ends either
way, and the same way throughout the file.

All of that is measured from the corpus and checked against `Modules/cPickle.c`
on CPython's 2.7 branch. `put` is `if (Py_REFCNT(ob) < 2 || self->fast) return
0;`, which is the mark left out. `put2` numbers a slot `PyDict_Size(self->memo)
+ 1`, under a comment reading "Make sure memo keys are positive!", which is the
base of one. `save_list` hands `batch_list` an iterator and `batch_list` calls
`PyIter_Next`, which is a new reference, so a list item is always filed; there
is no `batch_list_exact`, which is why a list of its ends the way `pickle.py`
ends one. `save_dict` calls `batch_dict_exact`, which walks the dictionary with
`PyDict_Next` and borrows, so a key or a value the program made on the spot has
a reference count of one and no mark. And a string shorter than two characters
goes through `save_string(self, args, 0)`, which neither files it nor looks it
up, so the second empty string in a file is spelled again rather than named.

The other tell is the memo mark after `BYTEARRAY8`. The C pickler has always
filed a bytearray in the memo; `pickle.py` did not until Python 3.10, which is
the whole of the difference between `numpy-1d-int64.p5.pickle` and its
`.pypickle` twin under Python 3.8. So a `BYTEARRAY8` with no mark behind it is
`pickle.py`'s, and specifically `pickle.py` of Python 3.8 or 3.9; the form
records it as `pickle.py` rather than naming a release, since the same bytes
cannot say more than that. Every slot after such a bytearray is numbered one
lower, so the mark is read rather than skipped.

### An array of pickled objects

NumPy's `O8` dtype has no numbers to write: the values are Python objects, so
the array is handed a list of them and they are pickled one by one after it.
There is nothing to measure, which is why `Dtype::width` has no answer for one
and why `fits` refuses to be asked.

**Which values one may hold**, since 2026-09-19: whatever the file's form
allows anywhere else. The list is an ordinary list, created empty and filled by
the opcodes after it, so it is read against the same stack the rest of the file
is read against rather than value by value. Text, numbers and the missing
entries between them, which is what a pandas column of objects usually holds;
containers of those, which is what a column of lists or of tuples is; and, when
the file's form allows the standard library, a `datetime.date` or a
`decimal.Decimal`, which is what a column of dates or of money is. The bounds
are the ones everything else is held to: the same depth, the same work budget,
the same batch lengths, the same memo.

Reading them through the one stack is what makes the widening exactly as
strict as the rest of the grammar rather than a second, looser reading. An
array of objects under the pandas form may hold what a pandas file may hold,
and one under the NumPy form may hold what a NumPy file may hold, which is
plain values and containers of them. A class the form's own prefixes do not cover is
refused at `may_name` wherever it is written, so an array of objects holding an
instance of a class no extension enumerates matches nothing, the way it did
before. The samples that say so are `unfamiliar-frame-of-instances-p4.pickle`
and `-p2`.

One thing follows rather than being arranged: a frame with a column of dates
uses the standard library's productions, so it is a mixture and is read under
the mixed form where a plain frame keeps the pandas one. A column of lists is
containers of plain values, which every form already reads, so that frame keeps
the pandas form. A `datetime.datetime` column is neither case: pandas writes
one as an `M8` array of counts, which is numbers.

Counting is what says where the list ends. The array's shape says how many
values there are, and the opcode after the last one belongs to the run that
made the array, so the fill stops at the count and a batch that overshoots it
is a non-match.

**Which forms allow it** is a flag in the `DECLARED` table, `object_arrays`,
and it is on for the NumPy form, the scikit-learn form, the pandas form, the
two joblib forms and the mixed form. Everywhere else an `O8` dtype is a
non-match.

The NumPy form was the last of those, on 2026-09-20. An array of objects is
what `pickle.dumps` writes for one, with no library anywhere near it, so
reading it under the form for what NumPy writes is the honest place for it:
the file is an array of objects rather than a mixture of families, and the
mixed form is for a file that holds two. One verdict moved and no other:
`pickle/proto4-numpy-object-array.pickle` was the opcode listing and is
`numpy-array-p4-p5-v7`. The `mixed-array-of-tuples` files did not move, and the
reason is that each of them holds a date beside the array, so it really is two
families; a file of nothing but an array of objects was the only kind that had
nowhere to go.

**A whole pickle inside the stream.** `joblib.dump` writes every array as a
wrapper and a run of bytes after it, and an object array has no bytes, so
`write_array` calls `pickle.dump(array, file_handle, protocol=5)` and lets a
second writer write into the same file. What lands in the stream is a whole
pickle: its own PROTO, its own framing, its own memo numbered from nought, its
own STOP, and then the outer stream carries on with the byte after. There is no
padding in front of it either, since the branch that writes the padding is the
other one.

It is read by the same grammar, with the outer stream's protocol, memo,
framing, pickler and joblib flag put aside and put back afterwards. So nothing
the inner pickle files reaches the outer memo, a slot number in either names
what its own stream wrote, the two may be at different protocols, which they
always are, and a wrapper of joblib's own inside the nested pickle is a
non-match, since `pickle.dump` writes none. Exactly one pickle follows the
wrapper, the value it holds has to be an object array whose shape and order are
the ones the wrapper described, and a byte between its STOP and the opcode the
outer stream carries on with is a non-match.

The bytes are instructions like any other, so the listing has to walk them.
`Cursor::breaks` is every place the opcode walk stops and starts again: a
joblib run of padding and numbers, which is not opcodes at all, and a nested
pickle, whose STOP would otherwise end the walk of the whole file. The tree
shows the pickle as a `nested pickle` node inside the array, with a `protocol`
row of its own, the run of instructions that rebuilt the array, and the values,
so a matched file still has no byte left over.

### The memo, and what a reference may name

A slot is bound when the file writes a memo mark, and the slot number is the
count of marks before it, so every mark a production consumes is accounted for
or the numbering drifts. Below protocol 4 the mark carries that number, and it
is checked rather than believed: a put to any slot but the next one is a
non-match. Two picklers make that arithmetic less simple than it looks.
Python 2's `cPickle` numbers its first slot 1, so the first mark in a file
fixes the base at 0 or 1 and every slot after it is counted from there;
anything but those two numbers is a non-match. And `cPickle` writes no mark at
all for a value whose reference count is one when it is written, which is why
a dictionary key it made on the spot is spelled again rather than named while a
list item, held by the pickler's own iterator, is filed. A missing mark files
no slot and the numbering carries on where it was. No other pickler leaves one
out, so a missing mark is read only while the numbering has not already shown
itself to start at nought. A form binds only what it spelled out itself: a text,
a byte string, a module-and-callable pair it named exactly, or a finished NumPy
dtype. Everything else it builds is opaque.

BINGET and LONG_BINGET are then accepted only where the grammar expects one of
those, and only to a slot already holding exactly it. So the second array of a
file may name `numpy._core.multiarray._reconstruct`, the `numpy.ndarray` class,
the `numpy.dtype` class, the text `numpy`, the placeholder byte string `b'b'`,
the byte-order letter, or the whole dtype the first array built, and a
reference to anything else is a non-match. A dtype's slot is written at the
REDUCE that makes it and filled in at the BUILD just after, which is where its
byte order arrives. There is no memo interpreter and no fixed slot numbers.

### Framing

Frames are read as CPython's framer writes them, rather than as one frame
spanning the body. A file may be unframed, or a run of frames; a frame may end
only between two objects, and the next begins there. The last frame ends where
the STOP does.

A payload of 64 KiB or more has two spellings, both read:

- From Python 3.7 it is written between frames. The frame being filled is
  committed so that it ends exactly at that opcode byte, the opcode and its
  bytes sit outside any frame, and a new frame begins immediately after them.
  The one exception is a large payload with fewer than four bytes left to
  write after it, which is written with no FRAME header in front because that
  is the framer's minimum size; two large payloads fewer than four bytes apart
  are the same case and are not matched.
- Python 3.4 to 3.6 had no path for writing bytes outside a frame, so the
  payload went into the frame being filled. That frame is then over its target
  and is committed at the next `save`, so it holds the payload and ends at the
  object after it, with the next frame beginning there. `basic-large-bytes` at
  protocol 4 from Python 3.4 and 3.6 is this, and it is what the fifth slice
  read as a non-match.

A small payload at a frame boundary, a frame reaching past the end of the
file, a frame that holds a large payload and then runs on past the next
object, and a frame that ends anywhere else with no large payload behind it,
are all non-matches. Which spelling a file uses says which Python wrote it and
not which pickler: the two picklers of one release agree.

Where a boundary may fall is the part that is easy to get wrong, and the fifth
slice got it wrong until it was measured. CPython commits a frame at the start
of every `save`, and a value inside a fixed run is a `save` like any other. The
run that rebuilds a NumPy array holds thirty of them: the module word, the
callable word, the placeholder shape, the placeholder byte string, each
dimension, the dtype's letters, the two flags it is built with, the byte order,
each of the eight values of its state, the storage-order flag and the numbers.
So a frame may end in front of any of those, and in any file over 64 KiB one
does. The forms now take the boundary in front of each rather than only between
one value of the file and the next, which is what
`familiar-numpy-large-p5.pickle` and the `a_frame_may_end_in_front_of_anything`
test are for. The opcodes that are not a `save` are REDUCE, BUILD,
STACK_GLOBAL, MEMOIZE and the opcodes that fold: no frame ends in front of
those. The one place the reading is looser than the writer is the MARK that
opens a batch, which is written before the `save` that follows it rather than
by one, and where a boundary is accepted although CPython never puts one.

### What is exposed now

`recognise` hands back a capture tree in which every node carries the bytes its
production consumed, and every leaf also carries the bytes its value proper sits
in. `Ty::Pickle` places that tree as fields, the way `Ty::Json` places parsed
JSON: `crates/core/src/eval/pickletree.rs` is the placement, and the nodes that
hold others are the only ones that keep the type. Every leaf is given the
ordinary type its bytes are (`BININT2` is a little-endian `u16` at its two
operand bytes, a text is UTF-8 at its counted run, an array is its dtype's
element type repeated by its shape), so reading, display and editing are the
machinery every other field uses, with no second code path.

Two templates now read a pickle, and the file decides which is offered:

- `pickle`, the opcode listing, unchanged. On a match its STOP row names the
  form and the other template as well as saying the contract's sentence.
- `picklefpf`, "Python pickle (familiar form)", which shows the object. Its
  root holds a `header` over the protocol envelope, carrying the contract's
  message, the form ID, which pickler the file shows and the protocol byte,
  and then `data`. A dictionary
  holds entries, an entry holds its key and its value, an array says its dtype,
  shape and storage order before its numbers. A file no form matches fails to
  resolve, so the chooser falls back to `pickle`.

A matched file has no byte left over, and that is the point rather than a
tidiness. A form fixes its instructions: the MEMOIZE after a string and the
SETITEM that files it under its key are not noise around the data, they are
the shape of the data, written down and matched exactly. So each of them is a
field named for what `pickletools` calls it, sitting inside the value it
builds and marked as that value's machinery, which folds it away for a reader
following the data and keeps it named for one following the program. The run
that rebuilds a NumPy array is a couple of dozen instructions and one act, so
it is one `ndarray reconstruct call` field holding the names and letters the
form matched inside it: `module`, `callable`, `class module`, `class`,
`dtype class`, `dtype` and `byte order`, each read as the text it is. NumPy's
protocol 5 call is `ndarray frombuffer call`, and it was handed the numbers as
one of its arguments rather than after it, so the `numbers` field sits inside
the call and the array has only the call beneath it.
`crates/core/tests/pickle_real.rs` walks every matched sample and asserts
that every node's children tile it, so an instruction that stopped being
named would fail rather than quietly become a gap.

One of the standard library's values reads as the text Python writes it in,
worked out from the run it was packed into rather than read where it sits, and
`eval/picklestd.rs` is that reading. A datetime is ISO 8601 with a `T` in it,
`2020-01-02T03:04:05.678901`, with the offset from UTC after it when the value
is aware and its zone is one the file spelled out; a date is `2020-01-02` and a
time `03:04:05.678901`. A `timedelta` reads as Python's own `str`,
`1 day, 0:00:02.000003`; a `Decimal` and a `Fraction` as the text they were
built from; a `uuid.UUID` as the hyphenated hexadecimal everything else writes
an id in; and a path as its parts joined the way its own class joins them. The
packed bytes stay where they are, as a `packed` row inside the value, and every
argument keeps the name Python gives it.

An `OrderedDict`, a `defaultdict` and a `Counter` show their entries exactly as
a dictionary does, with the class on a `class` row inside them as an object's
is, and each reads as what it holds and how much of it: `OrderedDict of 4`. A
`deque` shows its items the same way. So a list of `OrderedDict`s is a records
table like a list of dictionaries, which is what `stdlib-records` is.

A whole number past sixteen bytes is a node whose value is the digits it comes
to, with the run it was written in as a row beneath it: `bytes` for the
two's-complement run `LONG1` writes. There is no integer type that wide, so the
number is worked out once as the form reads the run and never read back out of
the file.

Every number written as a line of digits is the same shape, whatever its
width. The run is the spelling and not the number, and reading it as a
little-endian integer of its own length is how `L1L` used to show as 49; so an
`INT` line, a `LONG` line and every integer at protocols 0 and 1 are a node
whose value is the number, with the `line` row under it. A protocol 0 line that
spells a string rather than holding it is the same shape again, typed `text` or
`bytes`.

A matched list of records is a table, and the core says so rather than the
interface working it out. `[{"id": 1, "name": "a"}, ...]` is how rows are
pickled when nobody reached for pandas, and the keys are written in the file
beside the values, so `Evaluator::pickle_table` hands back a `TableShape` whose
`names` are every key any row has, in the order first met, and whose `cells`
says where a cell is: the rows are the `dict` nodes under the list, a cell is
the `entry` inside a row named by its column, and what it is worth is that
entry's `value`. `TableShape::cells` is new, and it is the first thing in the
table IR that describes a table whose rows are nodes rather than a run of
values. `web/src/picklerecords.ts` walks that answer now instead of deciding
for itself which node is a table and what its columns are called.

A `BINGET` where a value belongs is two bytes that say nothing on their own, so
a reference is a node of its own, typed `reference`, holding a `refers to` row
and the `BINGET` beside it. The row is worked out rather than read in place:
what the reference names sits wherever the file first wrote it, which is
outside the reference and often outside the whole value it is part of. A long
one is cut at 120 bytes, and a named byte string is shown in hex. A named
container is not shown at all: the row says what it is and where the file
wrote it, `list at 0x0b`, since a copy under the reference would be a value
the file says twice and holds once, and the container a self-reference names
has not been finished at the point the name is read. So a reference is always
the two bytes of its `BINGET` and never grows a subtree. A dictionary
entry whose key is a named string is still called by that string; one whose key
is a whole number is called by its digits; a key of any other kind leaves the
entry numbered by its place, because a float or a tuple written out as a name
would read as something the file says and is not.

Ranking: `PROBES` asks `picklefpf` immediately before `pickle`, and only when
the sniff window holds the whole file, since a form matches all of a file or
none of it. So a matched file over `SNIFF_WINDOW` (36 KiB) still opens as the
listing, and the reader picks the other template. `picklefpf` is deliberately
not in `WEAK_TEMPLATES`: parsing to the end is thin evidence and yields to
file(1), but a reviewed grammar that accounted for every opcode and operand in
the file is stronger than any rule keyed on its first bytes.

Of the sibling corpus, a hundred and eleven files match today. The twelve `familiar-` files
and the three `unfamiliar-` ones were written for this: the first half is plain
data written the ordinary way and the second half is pickles Python loads and a
form must still refuse, so a form that grew without anyone saying so fails on
one half or the other.

| File | Form, or why not |
| --- | --- |
| `awa2-pose-antelope.pickle` | `basic-p4-p5-v5` |
| `awa2-pose-elephant.pickle` | `basic-p4-p5-v5` |
| `proto4-unframed-payload.pickle` | `basic-p4-p5-v5`, over two frames and a payload between them |
| `familiar-records.pickle` | `basic-p4-p5-v5`: twenty records whose keys are named out of the memo |
| `familiar-long-containers.pickle` | `basic-p4-p5-v5`: a list of 1,001 and a dictionary of exactly 1,000 |
| `familiar-mixed-keys.pickle` | `basic-p4-p5-v5`: keys that are not strings |
| `familiar-tuples-and-sets.pickle` | `basic-p4-p5-v5`: every tuple arity, a set and a frozenset |
| `familiar-big-integers.pickle` | `basic-p4-p5-v5`: BININT and LONG1 on both sides of the boundary |
| `familiar-bytearray-p5.pickle` | `basic-p4-p5-v5`: BYTEARRAY8 |
| `familiar-bytearray-p4.pickle` | `builtins-values-p4-p5-v3`: the same object as a call to the class |
| `familiar-pure-python-batches.pickle` | `basic-p4-p5-v5`: the pickler in `pickle.py` ending a list of 1,001 its own way |
| `proto4-builtins.pickle` | `builtins-values-p4-p5-v3` |
| `proto4-numpy-array.pickle` | `numpy-array-p4-p5-v7` |
| `proto4-numpy-byte-order.pickle` | `numpy-array-p4-p5-v7` |
| `proto4-numpy-dtypes.pickle` | `numpy-array-p4-p5-v7` |
| `proto4-numpy-shapes.pickle` | `numpy-array-p4-p5-v7`, including a scalar |
| `proto4-numpy-shared-dtype.pickle` | `numpy-array-p4-p5-v7` |
| `familiar-numpy-large-p5.pickle` | `numpy-array-p4-p5-v7`: numbers too large to frame, so the boundary lands inside the call |
| `familiar-shared-list.pickle` | `basic-p4-p5-v5`: one list under two keys, named the second time |
| `familiar-recursive-list.pickle` | `basic-p4-p5-v5`: a list holding itself |
| `familiar-huge-integer.pickle` | `basic-p4-p5-v5`: two to the two hundredth, which needs twenty-six bytes |
| `proto4-datetime.pickle` | `stdlib-values-p4-p5-v2`: packed dates, times and spans |
| `unfamiliar-class-instance.pickle` | an instance of a class the file names |
| `unfamiliar-optimized.pickle` | `pickletools.optimize` took the memo marks out |
| `unfamiliar-lone-surrogate.pickle` | a string that is not UTF-8 |
| `proto*-everything.pickle` | `stdlib-values-*`: the dates, the exact numbers and the `ValueError`, at every protocol from 0 to 5 |
| `proto4-exceptions.pickle` | `stdlib-values-p4-p5-v2`: six exceptions, one of them a group |
| `proto4-structseq.pickle` | `stdlib-values-p4-p5-v2`: a `time.struct_time` and an `os.stat_result` |
| `proto*-numpy-1-recarray.pickle`, `proto*-numpy-2-recarray.pickle` | `numpy-array-*`: a record array, whose dtype is a class |
| `proto*-numpy-1-masked-record.pickle`, `proto*-numpy-2-masked-record.pickle` | `numpy-array-*`: a masked array of a structured dtype |
| `proto4-collections.pickle` | NEWOBJ of a namedtuple class the writing file defined, under `__main__`, which is exactly what the safety line refuses and must stay refused; its OrderedDict, defaultdict, Counter and deque are read |
| `proto4-newobj.pickle` | NEWOBJ and NEWOBJ_EX of arbitrary classes |
| `proto0-persistent-id.pickle`, `handmade-*` | persistent ids, and the opcodes CPython reads and never writes |
| `proto2-memo-over-256.pickle` | `basic-p2-p3-v1` |
| `proto*-persistent-id`, `proto2-extension-registry`, `proto5-out-of-band` | persistent ids, the extension registry and external buffers, all out of scope |
| `proto3-numpy-1-module-names.pickle` | `numpy-array-p2-p3-v2` |
| `proto4-numpy-object-array.pickle` | `numpy-array-p4-p5-v7`: an object dtype, whose values are pickled after the array |
| `proto4-scipy-coo-matrix`, `proto4-scipy-csc-matrix`, `proto4-scipy-csr-matrix` | `scipy-sparse-p4-p5-v1` |
| `proto4-sklearn-pipeline`, `proto4-sklearn-random-forest` | `sklearn-estimator-p4-p5-v1` |
| `proto5-pandas-dataframe`, `proto5-pandas-series`, `proto5-pandas-index-types` | `pandas-frame-p4-p5-v1` |
| `proto2-torch-state-dict` | protocol 2, and persistent ids for the tensor storage |

The six `everything` files read whole from 2026-09-23: the exception their
`ValueError` is, is the last thing they held. What is left is
`proto4-collections`, and it stays a non-match. It reads its `OrderedDict`,
`defaultdict`, `Counter` and `deque` and stops at a namedtuple class the
writing file defined, handed values by `NEWOBJ`. A class under `__main__`, or
any module the forms do not enumerate, is exactly what the safety line refuses,
and must stay refused: the file names a class this reader has no description
of, and reading one would be reading any class at all.

A namedtuple from a module the forms *do* enumerate is a different question,
and two of those are read now. `time.struct_time` and `os.stat_result` are
structseq rather than namedtuple, and the bytes are regular: the class, a
tuple of the numbers it reads as a sequence, a dictionary of the fields past
the end of that run, and `REDUCE`. `urllib.parse.ParseResult` is a real
namedtuple and is not read: it is `NEWOBJ` of a tuple subclass, its module is
spelled `urllib.parse` from protocol 3 and `urlparse` below it, and it has
three siblings in the same module with the same shape. No file in the
collection holds one.

`crates/core/tests/pickle_real.rs` writes the whole matrix out file by file so
that a form growing quietly is a failing test, and separately flips two bits of
every instruction byte in every matched sample, truncates at every instruction
boundary, and appends a value after the STOP.

### A library object as the thing it is

What a reader opens a pickled frame for is the data, and the data is not where
the tree puts it. So a matched frame, series or sparse matrix opens with a few
rows saying what it holds, before the structure that holds it, and a frame
opens as a table.

The summary rows are `columns`, `rows`, `index` and `dtypes` for a frame or a
series, and `shape`, `stored values` and `format` for a sparse matrix. They are
worked out from the match and the file and have no bytes of their own, the way
the header's rows do. An estimator has none: its attributes already are its
summary.

The table is `Cells::Computed` in the table IR, and the core reads the cells.
A frame's values are in blocks, each written the other way up from the frame,
so the frame's rows run along a block's **second** axis and column `j` of a
block is `j * rows` values in. Each block's `slice` says which of the frame's
columns its rows are, and the column names come from the first axis of the
manager, so the table's columns are the frame's columns in frame order and not
transposed. The index is the first column, headed by the index's own name or by
`index`; a counted index is counted out rather than read, since it is nowhere in
the file. A series is the same table with one value column, headed by the
series' name or by `value`. A categorical cell shows the category its code
names, and a code of -1 is nothing at all; so is a NaN, a `None` and the value
pandas writes where it has no date. A date shows as the date, ISO 8601 with no
zone, worked out from the count and the unit its dtype carries, and the column's
type says the unit: `date (datetime64[ns])`.

`Evaluator::pickle_cells` reads a window of rows and hands back a value a cell,
and the view asks for the rows it is drawing. Nothing about this is in the tree:
a cell of a frame is not a node, and the rows of a frame are not a run of bytes,
so a bit of the file is in no row rather than in the wrong one.

An object's node is typed `object` and reads as its class path on the `class`
row inside it rather than in the type column, because the type column is an
enumeration of shapes and a class path is data. The class row carries the whole
dotted path as its own value, so an object says what it is without being opened.

### What is not exposed yet, for a library object

A sparse matrix says what it holds and names its three arrays, and nothing lays
those out as the matrix they describe. Densifying one is not the answer; a
table of `row, column, value` would be.

### What is not exposed yet

An array's numbers read in storage order and are not folded into rows: a 4-by-6
matrix is 24 values, and the shape is a row beside them rather than the shape of
the listing. An instruction's operand is shown as its bytes rather than read: a
`BININT1` holding a dimension is two bytes in the listing and the shape it
belongs to is the row above. Nothing here navigates from a value to the opcodes
that built it, and nothing writes a value back through the recogniser: an edit
invalidates the recognition, and a changed instruction byte means the form no
longer matches at all.

The committed matrix fixture was copied from the existing sibling sample corpus;
its producer version is unknown. Its expected payload is a 4-by-6 matrix of f32
values 0 through 23. The protocol-5 alternative is an explicitly supported
grammar branch, not evidence that a particular NumPy release emits that layout.
The newer test variants are explicit transformations of that fixture: remove the
dictionary wrapper and adjust the numpy memo reference, or replace declared
shape, dtype, byte-order, storage-order and payload fields. They validate the
grammar and captured metadata but do not establish additional producer-version
provenance. Array dimensions and Fortran order are preserved in Rust captures;
the current viewer displays flat physical storage rather than logical rows.

Next steps, in order:

1. Fold an array's numbers by its shape, and navigate from a value to the
   opcodes that built it. Editing a captured value is a separate question: the
   recognition is invalidated by the edit and has to be made again.
2. Done, for the libraries and for the standard library: a class the file names
   may be a declared data field when its module is one a form lists, and "The
   safety line for a library object" above is the whole of the rule.
   `datetime`, `Decimal`, `Fraction`, `OrderedDict`, `defaultdict`, `Counter`,
   `deque`, `UUID` and the path classes each have the exact state they are
   rebuilt from written down, in `familiar/stdlib.rs`, and `time.struct_time`
   and `os.stat_result` beside them. The exceptions landed on 2026-09-23, in
   `familiar/exceptions.rs`: the whole builtin hierarchy, in the three module
   spellings the protocols write it in. What is left is a namedtuple the
   writing file defined, which no list can hold and which stays refused.
3. Done: a name may point at a container. The `refers to` row says what it is
   and where the file wrote it, `list at 0x0b`, so the reference stays the two
   bytes it is and the reader is sent to the bytes rather than shown a copy of
   them. What is still open is navigation: the row names an offset and does
   not take the reader there.
4. Done: the protocol 2 and 3 alternatives, and protocols 1 and 0 below them,
   so every family is written at all four ranges.
5. Done, for the shapes the corpus holds: `sklearn-estimator-p4-p5-v1`,
   `scipy-sparse-p4-p5-v1` and `pandas-frame-p4-p5-v1`. What is left is the
   datetime index, pandas 1.3, and the `DataFrame` table; see
   `HANDOVER-pickle-libraries.md`.
6. Move recognition onto a chunk-aware source cursor for large tensors. Current
   evaluator size caps still apply. Do not relax completeness to obtain previews.

The remaining sections describe the longer-term architecture and acceptance
criteria; they are not claims that all listed coverage has shipped.

Validation: the pickle unit tests include a hundred and forty-two FPF tests, eleven of which
read a fixture through the `picklefpf` template and check names, values and
byte ranges, and the rest of which build their own bytes to exercise one set
of alternatives each: what a later array may name out of the memo, what a
reference may not name, a reference to a container and to a container still
being filled, the two picklers' batch tails and the refusal to read a file
showing both, the memo mark after a bytearray, the NumPy scalar call, the
protocol 5 `_frombuffer` call with a writable and a read-only buffer, an
instruction moved, dropped, added or written in another width, the two ways a
large payload is framed, the builtins calls, and which form a file is read
under. Eighteen of them are the grammars below protocol 4: the memo slot a mark
may go in, the mark `cPickle` leaves out, the byte string written as a call and
the encodings it refuses, the set built from a list or a tuple, the class that
is never called, the `INT` line, a Python 2 `str`, an array Python 2 wrote, the
builtins under their Python 2 names, the spellings neither protocol range
shares with the other, and the opcodes no pickler writes, a protocol 1 file read without an opener, the
`long` protocol 1 writes as a line, a protocol 0 file read out of its lines, a
line with an escape in it read as the thing it spells, the lines no pickler
wrote, and ordinary text files that are not a familiar form whatever their
bytes walk as. Nine are the standard library's classes: a packed date, time and
datetime and the runs that are none of them, the fold bit and the protocols
that do not write it, the texts `Decimal` and `Fraction` are built from and the
ones Python would not have written, the three containers filled after the call
and the ways they may not be filled, the factories a `defaultdict` may be
handed and the ones it may not, a `Counter`'s mapping and a path's parts, an id
holding a number too wide for the reader's integer type and the `NEWOBJ_EX`
Python 3.4 writes, and the calls of names no form enumerated. Eighteen
`pickle_real` integration
tests pass against the sibling corpus, including the corpus match matrix, a
list of `OrderedDict`s opening as a table cell for cell at every protocol, the
per-file mutation sweep, a walk of the decoded array's 24 numbers, a walk of
twenty-one protocol 2 and 3 files of the matrix through the template with no
byte left over, and the same frame, series and array opening as the same table
at every protocol from 0 to 5.

The forms are also run over `pickle-matrix/` in the sample collection, which
is now committed: the same objects written by CPython 2.7, 3.4, 3.6, 3.7, 3.8,
3.10, 3.12, 3.13 and 3.14 and by PyPy 2.7 and 3.10, with NumPy 1.19 to 2.5
beside them where the release had one, at every protocol each has and from
every pickler each has. Every one of the 992 files matches, at every protocol
from 0 to 5. The matrix grew from 664 on 2026-09-19, when seventeen
`stdlib-*` objects were added to every environment: dates and aware dates,
ordered and defaulting dictionaries, counters, queues, exact numbers,
fractions, ids, paths, complex numbers, ranges and slices, and a list of
records with a date, a decimal and a counter in each.

Per family and protocol range: 44, 93, 67 and 77 of plain data; 34, 28, 14 and
21 of arrays and scalars; 33, 36, 18 and 18 scikit-learn; 8, 8, 4 and 5 scipy;
49, 53, 27 and 27 pandas frames and series; 52, 91, 66 and 78 of the standard
library's classes; and 6, 13, 10 and 12 whose values are builtins alone, which
stay under the builtins form because a file has to use a form's own
productions to be read under it.

The browser test is `web/test/pickle.browser.mjs`: it checks that a matched
sample opens as the familiar form with its form ID, its pickler row and its
decoded values, that the chooser offers both templates and switches between
them, that the STOP row of the listing names the form and the other template,
and that an unfamiliar program still reads as a PVM listing with no FPF
claim.

## Contract

A Familiar Pickle Form is a named, reviewed grammar for a complete pickle
instruction sequence. Instructions are delimiters and constants in that grammar.
Only explicitly declared data fields vary. Recognition extracts those fields
directly in Rust; it does not interpret a general pickle stack machine.

The entire stream must match, including STOP and EOF, before Qubero publishes
any extracted values or typed payloads. A familiar fragment inside an unfamiliar
program does not qualify. A non-match says nothing about whether Python could
load the file or whether the file is malicious.

On success, display exactly:

> Matched a Familiar Pickle Form: bypassed Pickle stack machine decoding.

Show the form identifier separately, with extracted data linked to its source
bytes. Keep hex and the PVM listing accessible. On non-match, show hex and the
PVM listing; label any symbolic decompilation separately from FPF extraction.
Do not display the success message for a whitelist match, partial match, or
successful symbolic execution.

## Relationship to the current code

- `crates/core/src/formats/pickle/mod.rs` builds the opcode listing and attaches
  `machine::Program` as its deducer. Its `opcodes` helper returns a partial vector
  on malformed input and stops at STOP without requiring EOF. FRAME is read as
  an eight-byte operand, without validating the boundary it declares.
- `machine.rs` models stack and memo operations. Its `Reading` deliberately keeps
  partial annotations. This is useful for inspection but is not an FPF matcher.
- `known.rs` recognises reconstructed values and uses broad module matching.
  These rules must not become FPF acceptance rules. Exact module and callable
  spellings belong to each form, including individually enumerated aliases.
- `shapes.rs` and the NumPy dtype table are reusable output descriptions once a
  form has validated its captures. Reusing these does not require running the VM.
- `eval/deduced.rs` currently reads the entire document to compute deductions,
  capped at 64 MiB on 32-bit targets and 256 MiB on others. This is a limitation
  for tensor files and must be explicit in an initial implementation.

Do not place FPF after `machine::run`. Route recognition first. A matched file
must never call the symbolic machine to establish its extracted values. On a
non-match the existing machine may remain an inspection-only decompiler, but
its payload deductions must not silently become FPF extraction. The initial
implementation preserves those deductions as the existing fallback path, as
requested; only an actual FPF match emits the success message.

## Proposed Rust components

1. A bounded cursor reads opcode boundaries and borrows operands from the source.
   It reports completion, truncation, unknown opcode, and limit exhaustion
   explicitly. It does not create a value stack or execute opcode effects.
2. Frame validation checks lengths with checked arithmetic, rejects nested frames
   and instructions crossing frame boundaries, and requires the final frame to
   end correctly. Forms specify supported framing arrangements explicitly.
3. A form registry dispatches by exact discriminating prefixes. Each matcher
   consumes fixed sequences, typed captures, and declared recursive productions.
   No arbitrary opcode skipping, generic REDUCE/BUILD handling, or backtracking
   through every combination of forms.
4. Capture validation checks relationships such as dtype width, shape, byte order,
   storage order and payload length. Payloads remain byte ranges, not copies.
5. A successful result contains form ID/revision, extracted value descriptions,
   and source ranges. Commit it only after complete validation. Keep Pending,
   NoMatch, Malformed and LimitExceeded distinct internally; all retain the
   inspection view. Editing invalidates recognition with the existing document
   cache lifecycle.

The initial deducer adapter can expose array captures using the existing
PayloadShape/PayloadCount queries. A separate result surface is needed for the
success message and a full basic-value tree: the existing machine's Collection
value does not retain list or dictionary contents.

## How strict is a form?

Strict means every allowed variation is written down and tested. Integer widths,
string lengths, tuple arity, batching, memo operations, and frame placement must
not be normalised away before matching. A form can contain explicit branches for
these differences; encountering a new variation produces a non-match until it
has been reviewed.

A simple illustrative body for a one-entry string-to-integer dictionary is:

```text
EMPTY_DICT MEMOIZE
SHORT_BINUNICODE <key bytes with declared length> MEMOIZE
BININT1 <unsigned byte>
SETITEM
STOP EOF
```

Its protocol header and any frame envelope are also part of the form. BININT2
is not accepted by this production merely because it denotes an integer; it
requires an explicit alternative. The example is a grammar sketch, not a claim
of a verified producer-version profile.

Nested data needs a finite recursive grammar, not one literal template per
object size. Recursion follows productions such as list-of-values and
dictionary-of-string/value-pairs, with bounded depth and element count.

Memo references need equally explicit rules. Initial forms can reject repeated
references and cycles. Later forms may bind names, dtypes or completed captured
values to memo slots and match references to those exact bindings. Do not ignore
MEMOIZE or accept an arbitrary BINGET. Reject unresolved, forward, and cyclic
references until a dedicated form defines them. Preserve dictionary entries as
pairs until duplicate-key handling is specified; do not silently turn them into
a JavaScript object with different key semantics.

## Coverage sequence

1. Small protocol 4/5 basic-value forms: null, booleans, integers, floats, text,
   bytes, lists, tuples and string-key dictionaries. Enumerate precise encoding
   branches. Preserve integers beyond JavaScript's exact range and distinguish
   bytes, tuples, non-finite floats and negative zero in the output model.
2. Standalone dense NumPy arrays and scalars with supported plain dtypes and
   inline storage. Recognise exact reconstruction and dtype state sequences.
   Validate nonnegative dimensions and checked product times item size against
   payload length; handle zero-dimensional and zero-length arrays explicitly.
   Preserve C/Fortran order and byte order. Initially reject object/structured
   dtypes and external buffers.
3. Specific pandas Series/DataFrame forms composed of accepted array and index
   forms, with exact state keys and ordering. Validate axes, column lengths,
   blocks and missing-value representation. A recognised array nested inside an
   unknown DataFrame form is insufficient to accept the document.
4. Specific scikit-learn estimator state forms, exposing stored parameters and
   arrays. There is no general sklearn-instance form and no method execution.
   Unknown attributes or state versions cause a non-match. Prediction behavior
   is outside extraction.
5. Separate container forms for tensor archives and external buffers. Validate
   the archive and every storage reference before publishing a complete match.
   This requires its own resource and path handling, beyond standalone pickle.

Version labels describe the fixture provenance, not an inferred Python/library
version: identical bytes cannot reveal which of several producers wrote them.
Different producer releases may share one reviewed form.

## Evidence and acceptance tests

The sibling `qubero-samples/pickle` corpus already includes NumPy, pandas,
scikit-learn, protocol variants and unusual opcodes. Treat these as candidate
fixtures, not as automatic acceptance specifications. Record producer versions,
creation parameters, protocol and expected extracted data when that provenance
is available. Unknown provenance must remain marked unknown.

Python may be used in an isolated development experiment to generate fixtures,
as described in the proposal. No Python is required by Qubero or its Rust
regression tests. A byte corpus with checked-in expectations is sufficient for
normal testing; observations suggest candidate forms, never auto-install them.

For every accepted form, test:

- Expected values, types, dimensions, ordering and source byte ranges.
- Truncation at each structural boundary, extra bytes after STOP, concatenated
  pickles, and invalid frame boundaries.
- Inserted/deleted/reordered instructions, changed globals, altered memo targets,
  extra state keys, and an otherwise familiar payload followed by unfamiliar code.
- Invalid lengths, UTF-8, shape multiplication, byte-order markers, recursion,
  element/memo counts and work-budget limits.
- Mutations inside valid payload captures that remain valid and change only the
  captured data; random payload bytes must never be read as instructions.
- No partial publication, no symbolic-machine invocation on success, inspection
  availability on non-match, pending chunk reads and cache invalidation on edits.

All parser allocations and loops need budgets before work is performed, including
tokenisation itself. Keep a shared work budget across candidate forms. Large-file
support should eventually use a source cursor that skips payload ranges while
validating their bounds and all remaining structure, rather than copying a tensor
file into the current whole-document deducer.
