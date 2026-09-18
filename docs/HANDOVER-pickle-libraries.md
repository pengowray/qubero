# Familiar Pickle Forms for pandas, scikit-learn and scipy

What has to be true of a form for the library pickles, worked out on
2026-09-18 from the same objects pickled under ten environments. Read
`DESIGN-familiar-pickle-forms.md` first: this is the next pass after the basic
and numpy forms, and it keeps their rule. A form is the exact instructions
CPython writes for a value, read as a fixed structure, and nothing is run.

## The corpus

`tools/make_pickle_matrix.py` in the sample collection pickles one list of
objects under whatever interpreter runs it, at every protocol that interpreter
has, into a folder named for what it found, with a `versions.json` beside the
files. `tools/run_pickle_matrix.sh` runs it in a throwaway podman container per
environment. A venv at `~/.venvs/qubero-samples` covers the newest libraries.

| Environment | Default protocol | Libraries |
| --- | --- | --- |
| Python 2.7 (`pickle` and `cPickle`) | 0 | none |
| PyPy 2.7 (`pickle` and `cPickle`) | 0 | none |
| Python 3.4 | 3 | none |
| Python 3.6 | 3 | numpy 1.19, pandas 1.1, scikit-learn 0.24, scipy 1.5 |
| Python 3.7 | 3 | numpy 1.21, pandas 1.3, scikit-learn 1.0, scipy 1.7 |
| Python 3.8 | 4 | numpy 1.24, pandas 1.5, scikit-learn 1.3, scipy 1.10 |
| Python 3.10 | 4 | numpy 1.26, pandas 2.2, scikit-learn 1.5, scipy 1.13 |
| Python 3.12 (system) | 4 | numpy 1.26 |
| Python 3.12 (venv) | 4 | numpy 2.5, pandas 3.0, scikit-learn 1.9, scipy 1.18 |
| Python 3.13 | 4 | numpy 2.2, pandas 2.3, scikit-learn 1.7, scipy 1.16 |
| Python 3.14 | 5 | numpy 2.5, pandas 3.0, scikit-learn 1.9, scipy 1.18 |
| PyPy 3.10 | 4 | none |

On Python 3 each object is written twice, by `pickle.dump`, which is the C
pickler, and by `pickle._Pickler`, which is `pickle.py` alone (`.pypickle` in
the name). PyPy 3.10 has only the second, and its files are byte for byte what
CPython 3.10's `pickle.py` wrote, all ninety of them.

On Python 2 each object is written twice as well, by `pickle` and by `cPickle`
(`.cpickle` in the name). PyPy 2.7 has both names, and its `cPickle` is not
the pure pickler under another name: it is a Python copy of CPython's C one,
and it numbers the memo from 1 the way that one does. What it does not copy is
CPython's `cPickle` leaving the memo mark out for a value nothing else holds a
reference to.

Python 3.14 changed `pickle.DEFAULT_PROTOCOL` from 4 to 5, so from there on a
file written with no protocol given is protocol 5.

The distinct byte strings are committed, each under the oldest environment
that wrote it, with every environment's `versions.json` listing everything it
wrote. A name in one of those lists with no file beside it is the same bytes
as a file in an older folder, and `sources.tsv` says which.

## What varies, and what does not

**Basic data varies in three places, all of them now read.** The dict keys of
a protocol 3 or 4 file come out in a different order under Python 3.4, because
a dict was unordered before 3.6; the two picklers spell the tail of a container
longer than a batch differently, and the memo mark after a bytearray; and
Python 3.4 to 3.6 left a payload over 64 KiB inside the frame it was filling
where 3.7 writes it between frames. The instructions are otherwise the same
everywhere. All 44 basic files at protocol 4 and 5 match `basic-p4-p5-v5`.

**numpy varies in one word.** Two byte strings per object and protocol: numpy
1.x spells the module `numpy.core.multiarray`, numpy 2.x spells it
`numpy._core.multiarray`. All 34 numpy array and scalar files at protocol 4 and
5 match `numpy-array-p4-p5-v6`.

**scikit-learn varies in its data, not in its instructions.** An estimator is
`STACK_GLOBAL` of its class, `EMPTY_TUPLE`, `NEWOBJ`, then a dict of its
attributes and `BUILD`. Between releases the list of attribute names changes
(1.7 has a `tol` that 1.5 has not), `_sklearn_version` holds the release, and
the arrays inside follow numpy's spelling. So the form cannot pin an attribute
list per release. It fixes the instruction shape and reads the attribute names
as data: an object of a class under `sklearn.`, whose state is a dict of text
keys to values the other forms already read (basic values, numpy arrays, numpy
scalars, nested objects of the same kind). A decision tree also holds a
`sklearn.tree._tree.Tree` built by `REDUCE` with a structured array of nodes,
which is its own production.

**pandas varies in its instructions, by generation.** A `DataFrame` is a
`BlockManager` in every release, and what the manager's state is made of has
changed twice:

| pandas | How a block is written |
| --- | --- |
| 1.1 | the arrays directly in the manager's state, each a `numpy` `_reconstruct` |
| 1.3 | `functools.partial` over `new_block`, per block |
| 1.5, 2.2, 2.3, 3.0 | `pandas._libs.internals._unpickle_block(values, slice, ndim)`, per block |

Inside the last generation the values are an ordinary numpy array up to 2.x. In
3.0 a numeric block at protocol 5 is `numpy._core.numeric._frombuffer` over a
`BYTEARRAY8`, and a text column is a `StringArray` rebuilt through
`__pyx_unpickle_NDArrayBacked` around an object array of `SHORT_BINUNICODE`.
The axes are `_new_Index` calls: `Index` over an object array of names,
`RangeIndex` as a dict of start, stop and step, `DatetimeIndex` for dates.

So pandas wants one form per generation, newest first, each reusing the numpy
productions, and the 1.5 to 3.0 generation is the one worth writing first: it
is every file written since late 2022.

**scipy** sparse matrices are an object of a class under `scipy.sparse.` whose
state is a dict holding three numpy arrays and a shape tuple, so the
scikit-learn production with another module prefix reads them. The class moved
(`scipy.sparse.csr` to `scipy.sparse._csr`), which is data to that production.

## What has landed, on 2026-09-18, and protocols 2 and 3 on 2026-09-19

Every step of the order below. The safety line and the enumerated calls are
written out in `DESIGN-familiar-pickle-forms.md` under "The safety line for a
library object"; the code is `crates/core/src/formats/pickle/familiar/`
(`forms.rs` for the registry, `object.rs` for the class, object, BUILD and
REDUCE productions, `dtype.rs` for the structured, object and datetime dtypes)
and `crates/core/src/eval/pickleframe.rs` for a frame read as a table.

| Form | What it reads | Matched at protocol 4 and 5 |
| --- | --- | --- |
| `sklearn-estimator-p4-p5-v1` | estimators, pipelines, forests, decision trees | 33 of 33 matrix files, and both `pickle/proto4-sklearn-*` |
| `scipy-sparse-p4-p5-v1` | the three sparse matrix classes | 8 of 8 matrix files, and all three `pickle/proto4-scipy-*` |
| `pandas-frame-p4-p5-v1` | frames and series from pandas 1.1 to 3.0 | 49 of 49 matrix files, and all three `pickle/proto5-pandas-*` |

Every file in `pickle-matrix/` written at protocol 4 or 5 now matches: 168 of
168. `numpy-numeric-array-p4-p5-v5` became `numpy-array-p4-p5-v6`: it reads a
structured dtype now, so "numeric" was no longer true.

Each family has three forms now, named for the protocols each reads. At
protocols 2 and 3: `basic-p2-p3-v1`, `numpy-array-p2-p3-v1`, `builtins-values-p2-p3-v1`,
`sklearn-estimator-p2-p3-v1`, `scipy-sparse-p2-p3-v1` and
`pandas-frame-p2-p3-v1`. All 218 files in `pickle-matrix/` written at protocol
2 or 3 match, and so do `pickle/proto2-memo-over-256.pickle` and
`pickle/proto3-numpy-1-module-names.pickle`, which the earlier slices read as
non-matches. `DESIGN-familiar-pickle-forms.md` has the whole of what differs,
under "Protocols 2 and 3".

And at protocol 1: `basic-p1-v1`, `numpy-array-p1-v1`, `builtins-values-p1-v1`,
`sklearn-estimator-p1-v1`, `scipy-sparse-p1-v1` and `pandas-frame-p1-v1`. All
130 files in `pickle-matrix/` written at protocol 1 match. The libraries needed
one production there: protocol 1 had no NEWOBJ, so an object is made by
`copy_reg._reconstructor(cls, object, None)`, which is `cls.__new__(cls)`
written the long way round. See "Protocol 1" in the design document for the
other four differences, and "What protocol 0 would need" for the one below it,
which is the escaping layer a text line needs and nothing else.

The libraries needed nothing new. scikit-learn, scipy and pandas write
`GLOBAL`, `NEWOBJ` and `BUILD` below protocol 4 exactly as they write
`STACK_GLOBAL`, `NEWOBJ` and `BUILD` above it: `copyreg._reconstructor` is in
no file in the corpus. What the protocols do differ in is where the array
values sit. Protocol 2 has no opcode for a byte string, so an array's numbers
go out as the latin-1 text they spell, handed to `_codecs.encode`. The form
decodes that run once as it reads it and keeps the bytes beside the match, and
every cell of the array and of any frame holding it is read from there, so a
protocol 2 frame, series and array open as the same table, cell for cell, as
the same object at protocol 4. `a_pickled_frame_opens_as_the_table_it_holds`
and `an_array_reads_as_the_same_numbers_at_every_protocol` in
`crates/core/tests/pickle_real.rs` are that claim.

What that costs, and what would replace it: the decoded bytes are a copy, held
for as long as the match is, so a protocol 2 file of arrays is read into memory
twice. The honest shape for this is a space of its own, the way a compressed
stream gets one (`eval/space.rs`, `Spaces::add`): the numbers would then be
ordinary typed fields at ordinary offsets in that space, the hex view would
show the decoded bytes, and nothing would need a computed-cell path. What
stands in the way is that a space is entered through a `Ty::Decoded` node,
which wants a `Codec` of its own and a trace, and every exhaustive `match` on
`Codec` in the evaluator, the listing, the diagram and the graph would gain an
arm. That is the next thing worth doing for these forms, and it is a day's work
rather than an afternoon's.

A frame opens as a table, and a frame, a series and a sparse matrix say what
they hold before showing how. See "A library object as the thing it is" in the
design document.

What is left, in the order it is worth doing:

1. **Protocol 0**, which is every remaining file in the matrix: 148 of them,
   and the default Python wrote with until 3.0. See "What protocol 0 would
   need" in the design document; the short of it is that a text line is not
   the text it spells, so a text leaf needs the treatment a protocol 2 array's
   numbers already get.
2. **A protocol 2 array's numbers as a space of their own**, replacing the
   copy kept beside the match. See above.
3. **A sparse matrix as a table** of `row, column, value`, read out of the
   `data`, `indices` and `indptr` it already names. Nothing densifies.
4. **The standard library's classes**, which are what every remaining
   `proto*-everything` sample is held back by: `datetime`, `Decimal`,
   `Fraction`, `OrderedDict`, `defaultdict`, `Counter`, `deque`. Each needs the
   exact state it is rebuilt from written down, the way the library calls are.
5. **A block placed by an array** rather than by a slice, which pandas writes
   when a block's columns are not next to each other. No file in the corpus
   does, so there is nothing to test it against.

`cargo run -p qubero-core --example pickle_forms -- <file>` prints the form a
file matched, or, for one no form matched, the offset the reading reached
before it stopped, which is where the next production goes.

## Order of work

1. A generic *plain object* production: a class named by `STACK_GLOBAL` from a
   listed module prefix, `NEWOBJ` with an empty tuple, a state dict, `BUILD`.
   The prefixes are the whitelist; the instruction shape is fixed. This reads
   scikit-learn estimators and scipy sparse matrices at once.
2. `numpy` scalars (`numpy.core.multiarray.scalar`), which estimators hold.
3. The pandas `_unpickle_block` generation: `DataFrame`, `Series`, the three
   index kinds, then `Categorical` and `StringArray`.
4. The field tree for each: a `DataFrame` wants its columns named and its
   blocks offered as tables (`Evaluator::pickle_table` is where a matched
   array's shape already becomes a table).
5. Done: protocols 2 and 3, for every family rather than only the basic and
   numpy ones, since the libraries write the same structure there.
   Protocols 0 and 1 are not in scope; `DESIGN-familiar-pickle-forms.md` says
   what they would need under "What protocols 0 and 1 would need".

## Not decided

- joblib files, which is how scikit-learn's own documentation says to save a
  model. A `.joblib` is a pickle with the array bytes written between the
  instructions, so it is not a pickle any of this reads.
- torch. `torch.save` writes a ZIP holding a protocol 2 pickle with persistent
  ids for the tensor storage, and the storages as other entries of the ZIP.
