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

And at protocol 1 and protocol 0, named the same way: `basic-p1-v1` and
`basic-p0-v1`, and their five neighbours each. All 130 files written at
protocol 1 and all 148 written at protocol 0 match, so every one of the 664
files in `pickle-matrix/` now matches a form.

The libraries needed one production below protocol 2: neither of those
protocols had NEWOBJ, so an object is made by
`copy_reg._reconstructor(cls, object, None)`, which is `cls.__new__(cls)`
written the long way round. See "Protocol 1" and "Protocol 0" in the design
document for the rest. The one thing protocol 0 needed that nothing else did
is a value model for a line that spells its value rather than being it: such a
line is a node whose value is what it spells, with a `line` row under it
holding the run the file wrote. Most keys and names hold no escape and are
still leaves over their own bytes.

The libraries needed nothing new. scikit-learn, scipy and pandas write
`GLOBAL`, `NEWOBJ` and `BUILD` below protocol 4 exactly as they write
`STACK_GLOBAL`, `NEWOBJ` and `BUILD` above it: `copyreg._reconstructor` is in
no file in the corpus. What the protocols do differ in is where the array
values sit. Protocol 2 has no opcode for a byte string, so an array's numbers
go out as the latin-1 text they spell, handed to `_codecs.encode`, and
protocol 0 writes that same text as an escaped line. Either way the run opens
as a space of its own and the numbers are ordinary fields of it, so a
protocol 0 or 2 frame, series and array open as the same table, cell for cell,
as the same object at protocol 4. `a_pickled_frame_opens_as_the_table_it_holds`
and `an_array_reads_as_the_same_numbers_at_every_protocol` in
`crates/core/tests/pickle_real.rs` are that claim, and
`a_spelled_array_opens_its_numbers_as_a_space` is the claim that a value in
one of those spaces has an address in it.

A frame opens as a table, and a frame, a series and a sparse matrix say what
they hold before showing how. See "A library object as the thing it is" in the
design document.

## Where the code is, after the refactor of 2026-09-19

| File | What it holds | Lines |
| --- | --- | --- |
The line counts are from that day; the two moves of 2026-09-19 that follow the
scikit-learn breadth sweep are `familiar/sklearn.rs`, which holds
`SKLEARN_CALLS` and `SKLEARN_CLASSES` the way `stdlib.rs` holds its own,
`familiar/torch.rs`, which took `TORCH_CALLS` and `TORCH_CLASSES` out of
`forms.rs` for the same reason, and `familiar/integer.rs`, which is
`Cursor::integer` and the widths CPython writes a whole number in, out of
`basic.rs`.

| `familiar/mod.rs` | how a match is made: the envelope, the budget, `recognise` | 424 |
| `familiar/joblib.rs` | the array wrapper `joblib.dump` writes, the run after it, and the pickle it writes instead for an array of objects | 294 |
| `familiar/captured.rs` | what a match is made of: `Value`, `Kind`, `Shape`, `Dtype`, `Storage`, `Tensor` | 648 |
| `familiar/forms.rs` | the families, declared once each and read at every protocol, and the mixed form's union of them | 607 |
| `familiar/formnames.rs` | the name each form goes by, which is what the rest of the program says to this one | 121 |
| `familiar/python2.rs` | the three spellings Python 2 had and Python 3 dropped | 121 |
| `familiar/picklers.rs` | the seven statements the `pickler` row makes, and how one sharpens another | 118 |
| `eval/pickleobjects.rs` | an array of pickled objects, read as the rows it shows | 37 |
| `familiar/packs.rs` | which families a file turned out to use, as the bits of one word | 95 |
| `familiar/basic.rs` | the stack a pickle is read against | 657 |
| `familiar/values.rs` | the leaf productions, and what Python can hash | 187 |
| `familiar/lines.rs` | protocol 0: the lines, and the two escapings | 256 |
| `familiar/codecs.rs` | a byte string below protocol 3 | 177 |
| `familiar/cursor.rs`, `memo.rs` | bytes and frames; the slots a file names things in | 454, 334 |
| `familiar/numpy.rs`, `dtype.rs`, `builtins.rs`, `object.rs` | the productions each form adds | 455, 353, 116, 385 |
| `familiar/torch.rs` | a tensor, and the persistent id that says where its numbers are | 261 |
| `familiar/stdlib.rs` | the standard library's calls, and what each argument has to be | 400 |
| `eval/pickleparts.rs` | what a node of the tree is made of, a kind to an arm | 711 |
| `eval/pickletree.rs` | placing and naming those, and the table shapes | 556 |
| `eval/picklesaid.rs` | what a value comes to in a few words | 116 |
| `eval/picklestd.rs` | a date, an exact number, an id or a path as the text Python writes it in | 335 |
| `eval/pickleframe.rs`, `picklecells.rs` | a frame read as a table, and its cells | 428, 477 |
| `eval/picklesummary.rs` | what a summary row of a frame, an index or a sparse matrix says | 232 |
| `eval/pickletorch.rs` | where a tensor's numbers are, in an archive or in a legacy file, and the cells read there | 603 |
| `formats/zipdirectory.rs` | an archive's central directory, read from the end: each entry's name and the run its data is | 337 |
| `formats/torchzip.rs`, `torchlegacy.rs` | the two things `torch.save` writes: the archive, and the five pickles and their storages | 142, 289 |

**Adding a family of forms is one file and one row.** Write the productions
beside the ones they read like (`numpy.rs` is the model), add the callables it
accepts a REDUCE of as a `const NAME_CALLS: &[Reduce]` in `forms.rs`, add the
four form identifiers, and add one `Declared` row naming them. The protocol
spellings are already the cursor's business: a family says nothing about
protocols beyond the name each of its forms goes by, and `forms()` expands the
row over the four ranges. The calls every form below protocol 4 shares, and the
object maker below protocol 2, are added by `Cursor::calls` from the protocol
rather than written into the row.

## The standard library's classes: landed on 2026-09-19

Measured from a fresh matrix run that adds seventeen `stdlib-*` objects to
every environment, and read the same day. The code is
`crates/core/src/formats/pickle/familiar/stdlib.rs` for the calls and
`crates/core/src/eval/picklestd.rs` for what each value reads as;
`DESIGN-familiar-pickle-forms.md` has the whole of what the form takes, under
`stdlib-values-p4-p5-v1` and its three neighbours.

| Form | What it reads | Matched in `pickle-matrix/` |
| --- | --- | --- |
| `stdlib-values-p4-p5-v1` | dates, spans, zones, exact numbers, ids, paths, counters, ordered and defaulting dictionaries, queues | 52 of 52 at protocol 4 and 5 |
| `stdlib-values-p2-p3-v1` | the same | 91 of 91 |
| `stdlib-values-p1-v1` | the same | 66 of 66 |
| `stdlib-values-p0-v1` | the same | 78 of 78 |

Every file in `pickle-matrix/` matches a form: 992 of 992, at every protocol
from 0 to 5 and from every pickler each environment has. The matrix was 664
files before the `stdlib-*` objects were added to it.

**What the shapes turned out to be**, all verified against the bytes:

| Callable | Arguments |
| --- | --- |
| `datetime.datetime` | one byte string of 10, or that and a `tzinfo` when aware |
| `datetime.date`, `datetime.time` | one byte string of 4 and of 6, the second also with a `tzinfo` |
| `datetime.timedelta` | days, seconds, microseconds |
| `datetime.timezone` | one `timedelta`, or that and a name |
| `decimal.Decimal` | one text, which is what `str(Decimal)` writes |
| `fractions.Fraction` | two integers, or one text `n/d` |
| `collections.Counter` | one dictionary, read as the counter rather than as an argument |
| `collections.OrderedDict`, `defaultdict`, `deque` | called empty and filled by the `SETITEMS` or `APPENDS` after them, or, in the older releases, handed everything they hold as one list |
| `pathlib.PurePosixPath`, `PureWindowsPath` | however many words the path is made of |
| `uuid.UUID` | no call: the plain object production with a 128-bit `int` in its state |

**Four things the measurement had not seen**, each found by running the forms
over the whole matrix and each now read:

- **`NEWOBJ_EX`.** Python 3.4 writes `EMPTY_TUPLE EMPTY_DICT NEWOBJ_EX` where
  every release after it writes `EMPTY_TUPLE NEWOBJ`, so a `uuid.UUID` from
  3.4 needed the third opcode. Arguments of either kind are refused for the
  same reason `NEWOBJ`'s are.
- **`__builtin__.long` and `__builtin__.unicode`.** `fix_imports` renames a
  `defaultdict`'s factory on the way down to a protocol Python 2 could read:
  `int` becomes `long` and `str` becomes `unicode`. Python 2 writing its own
  `int` and `str` is the pair without the rename, and all four are on the list.
- **Python 2's two-argument `bytearray`.** Python 2 had no type for a run of
  bytes to hand the class, so it writes `bytearray(text, 'latin-1')` where
  Python 3 writes `bytearray(bytes)`. That belongs to the builtins form rather
  than to this one, and `stdlib-range-slice` from Python 2 is what found it.
- **A packed run reaches the reader four ways.** Protocol 3 and up write a byte
  string; protocol 2 hands the bytes to `_codecs.encode` as the latin-1 text
  they spell; protocol 0 writes that text again as an escaped line; and Python
  2's `str` is the bytes, which reads as text when they happen to be UTF-8.
  `Cursor::packed_bytes` reads all four, and `Evaluator::packed` reads them
  again on the way to the screen.

**Two things that were not the standard library's** and were fixed on the way:

- An integer past sixteen bytes had no reading at all, and a `uuid.UUID` is
  128 bits: whenever its top bit is set it goes out as seventeen bytes, the
  last of them the nought that says it is not negative. `LONG1` now reads at
  every width the opcode can declare, up to 255 bytes, and a number past
  sixteen is a node whose value is the digits it comes to with the run beneath
  it. That made `unfamiliar-huge-integer` familiar, and it was renamed.
- A protocol 0 line that spells its value rather than holding it had no type
  and failed to place. It is a node now, typed `text` or `bytes`, with its
  `line` row under it. No file in the collection had one until
  `stdlib-records.p0` arrived.

**How the family was added**, which is the recipe above with one thing more.
`STDLIB_CALLS` and its validators went in `familiar/stdlib.rs`, four
identifiers and one `Declared` row went in `forms.rs`, and the row's `classes`
is
`&["datetime", "_datetime", "collections", "_collections", "decimal", "fractions", "uuid", "pathlib"]`.
`builtins` is deliberately not on that list: a module prefix says which classes
may be named, and every class under `builtins` is too many to name. The eight a
`defaultdict` may be handed as its factory are enumerated instead, by their
whole dotted path in both of the spellings `fix_imports` writes, in a new
`names` column on `Allow` that means "may name, never call".
`__builtin__.object`, which used to be a special case in `may_name`, is what
that column generalises.

The one new mechanism is a call whose result the opcodes after it fill. A
`Reduce` says so with `Args::FillsDict` or `Args::FillsList`, `reduced` gives
the result an empty `Kind::Dict` or `Kind::List` as its `state`, and `one` and
`batch` in `basic.rs` look through a `Made`'s state to find the container they
are filling. `Args::Many` is a path, whose arguments are its parts however many
there are, and `Args::Contents` is a `Counter`, whose one argument is the
counter itself. A callable written more than one way has a row each, and
`reduced` asks every row of that name rather than the first, which is what a
naive and an aware `datetime` needed.

**What is not read**, and what each `proto*-everything` sample now stops at:
`ValueError`, spelled `exceptions.ValueError` below protocol 3 and
`builtins.ValueError` from 3 up, which is an exception rebuilt by `REDUCE` from
the message it was raised with. `proto4-collections` reads its `OrderedDict`,
`defaultdict`, `Counter` and `deque` and stops at a namedtuple class the
writing file defined, handed values by `NEWOBJ`.

## Four more interpreters, measured

The same run covers Jython 2.7.4 (`cPickle` written in Java), IronPython 2.7.12
and 3.4.2 (C#) and GraalPy 24 for Python 3.11 (`_pickle` written in Java). None
of their files is in the collection yet, and none would match today.

- **Jython's `cPickle` is a fourth memo rule.** It numbers from 1, as CPython's
  does, but it marks every value including the one-character strings CPython's
  leaves out, and it does not batch at a thousand: a list of 1,001 is one
  `MARK .. APPENDS` and a dictionary of 1,000 has no empty batch behind it. So
  the batch-edge tells say nothing about it and the form would have to allow a
  batch longer than `MAX_BATCH` under that numbering.
- **How much is new**: 63 of Jython's 180 basic and stdlib files are byte for
  byte CPython 2.7's, 60 of IronPython 2.7's 176, 74 of IronPython 3.4's 160
  against Python 3.4, and 284 of GraalPy's 383 against Python 3.10. The rest is
  where the rules differ, and some of the Python 2-era part is dict ordering,
  which is data rather than grammar.
- **Two of them wrote files their own pickler could not**: IronPython 2.7's
  `cPickle` raises `UnicodeEncodeError` on `basic-long-dict` and
  `basic-many-frames` at protocols 1 and 2, and GraalPy's `pickle.py` raises
  `AssertionError` on `stdlib-datetime-aware` at protocol 0. Those are in the
  new `failed` map in `versions.json` rather than in the folder, so there is
  nothing to match and nothing to refuse.

## Four more interpreters: their grammars landed on 2026-09-19

Every `basic-*` and `stdlib-*` file the four wrote is read now, 898 of 899,
and the one that is not is named at the end of this section. The samples are
**not in the collection yet**: `tools/collect_pickle_matrix.py`, the matrix
test and `age()` are the work left, and they are written out under "What is
left" below.

| Environment | Files | Matched before | Matched after |
| --- | --- | --- | --- |
| Jython 2.7.4 | 180 | 158 | 180 |
| IronPython 2.7.12 | 176 | 82 | 176 |
| IronPython 3.4.2 | 160 | 118 | 160 |
| GraalPy 24 (Python 3.11) | 383 | 315 | 382 |

**Two axes, and they must not be confused.** Most of what tells these files
apart is the runtime, not its pickler, and both of an interpreter's picklers
write it: an accelerator module named `_collections` or `_datetime`, a
`datetime` handed its fields instead of its packed bytes, a `bytearray` or a
`bytes` handed latin-1 text at a protocol that has an opcode for bytes, an
escape spelled in upper case, one memo slot standing for two equal strings.
None of those may move the `pickler` row, and none of them does. What does
move it is a pickler's own behaviour: how it numbers the memo, how long a
batch it writes, how it ends a container. `familiar/picklers.rs` holds that
side on its own, as seven statements with a sharpening relation between them.

**Jython's `cPickle`**, `src/org/python/modules/cPickle.java`:

- `BATCHSIZE` is 1024, not a thousand. `Cursor::batch` is the length this
  file's pickler writes, fixed by the first full batch with another behind it,
  and a batch past a thousand is Jython's and is read only at protocols 0 to 2.
- `batch_appends` writes MARK and APPENDS however short a list is, so a list of
  one item has no APPEND. That is the tell most of its files show.
- `batch_setitems` writes SETITEM for a single entry left over and nothing at
  all for none, so its dictionaries end the way `pickle.py` ends one and a
  dictionary of exactly 1,024 has no empty batch behind it. Nothing in the
  corpus is 1,024 long; this is read off the source rather than measured.
- `save_tuple` takes the same path for an empty tuple as for any other and
  ends it with a `put`, so at protocol 0 the empty tuple is filed in the memo.
  CPython 2's `cPickle` and PyPy 2.7's copy both leave it out, which is what
  separates the three.
- `putMemo` returns `memo.size() + 1`, so the memo numbers from 1, and `put`
  has no reference count to check, so nothing is left out.
- `PyUnicode_EncodeRawUnicodeEscape` writes upper-case hexadecimal where
  CPython writes lower. A protocol 0 `UNICODE` line is held to one case
  throughout rather than to either.

**IronPython 2.7's `cPickle`**, `Src/IronPython.Modules/cPickle.cs`:

- `MemoizeNew` is called when the pickler starts saving an object and
  `WritePut` when it has finished, so the slot an object takes is numbered
  before the slots its callable and arguments take and its mark is written
  after theirs. The memo is therefore not filled in order. `Bound::Reserved`
  is a slot the file numbered past, `Memo::bind_at` fills one in later, and
  `Cursor::numbering` asks at the end of the file which of the two picklers
  that leave a gap wrote it: left empty it is a `cPickle` numbering from 1,
  filled out of turn it is this one. The gap is bounded by `MAX_DEPTH`, so one
  opcode cannot cost the reader a table.
- `_batchSize` is a thousand and its container tails are `pickle.py`'s.
- Its `str` is a .NET string, so `str.__reduce__` writes
  `__builtin__.bytes(text, 'latin-1')` where CPython 2 writes the bytes. That
  is the runtime, not the pickler, and it is read beside `_codecs.encode`.
- Its `datetime`, `date` and `time` are managed classes whose `__reduce__`
  hands the constructor its fields.

**IronPython 3.4** has one pickler, its `_pickle` in C#, and `versions.json`
lists no `.pypickle` files. It names `_datetime` and `_collections`, writes the
same field-by-field date classes, writes `bytearray(text, 'latin-1')` at
protocol 3 and 4 where CPython writes a byte string, and drops a `deque`'s
`maxlen`. Its container tails are `pickle.py`'s, so **it has no tell of its
own** and its files carry the broader statement.

**GraalPy's `_pickle`**, written in Java, spells a protocol 0 float the way
`Double.toString` does: `1.0E308`, `Infinity`, `NaN`. Its `pickle.py` writes
`repr` like every other copy, so that line is the pickler and it is the one
GraalPy tell. Everything else of GraalPy's is the runtime: `_collections`, and
one memo slot standing for two equal strings, which reaches the file as a
reference where CPython spells the run again.

**The one file that does not read**, and why:
`graalpy3.11/stdlib-datetime.p0.pypickle.pickle`. Two dates hold the same
packed run; GraalPy gives both the same slot, so the second date's
`_codecs.encode` names the escaped protocol 0 line the first one already
decoded. The form decodes such a line once and keeps the bytes beside the
match, and reading the same run a second time would decode what it had already
replaced. The fix is for a decoded run to carry both readings, which is the
same change as "an array's decoded numbers as a space of their own" below.

## What is left

1. **The collection and the tests.** `age()` in
   `tools/collect_pickle_matrix.py` has to sort CPython before every other
   implementation of the same version, or a file both wrote is kept under
   `ironpython2.7/` because `i` sorts before `p`. Then
   `collect_pickle_matrix.py` over the matrix run, `build_index.py`, the
   `sources.tsv` rows, and the matrix test in `crates/core/tests/pickle_real.rs`,
   which panics on an environment folder it has no rule for.
2. **A `pickler` row that can say IronPython 3.4.** Nothing that pickler
   writes is its own. The bytes do show the interpreter, through `_datetime`
   and the field-by-field dates, and IronPython 3.4 has one pickler, so the
   inference is available; it crosses the two axes above, and it is not made.

What is left from the earlier pass, in the order it is worth doing:

1. **A byte string below protocol 3 as the bytes it spells**, wherever one is
   written, rather than only where an array holds one. See "A decoded run as a
   space of its own" below for what that takes and why dates were left out of
   it.
2. **A sparse matrix as a table** of `row, column, value`, read out of the
   `data`, `indices` and `indptr` it already names. Nothing densifies.
3. **An exception rebuilt from its message**, which is what every remaining
   `proto*-everything` sample is held back by, and a namedtuple, which is what
   `proto4-collections` is held back by. The first needs the exact state
   `ValueError` is rebuilt from written down; the second names a class the
   writing file defined, and no list can hold that.
4. **A block placed by an array** rather than by a slice, which pandas writes
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

## The torch kinds the current release writes: landed on 2026-09-19

`DESIGN-pickle-containers.md`, under "torch.save: the kinds torch 2.14 writes",
has the whole of it: `_rebuild_tensor_v3` with an untyped storage and the dtype
as an argument, the complex and quantised storage classes, `_rebuild_qtensor`,
`_rebuild_sparse_tensor`, `torch.Size`, `torch.device`, the dtypes as globals
the form names and never calls, and a whole `nn.Module` pickled as an object of
a class under `torch.nn`.

Two things that belong to this file rather than that one, because they are
about how a form is declared:

- **A family may require an extension rather than a count.** `Family::Torch`
  asked for `tensors > 0`, which is not what a torch file is: a saved
  `torch.Size`, device and dtype is one and has no tensor in it. It asks for
  the torch extension now, and `Cursor::torch_named` sets that bit wherever a
  global under `torch` is named. `collections` is outside that prefix on
  purpose, or a state dict would read as a mixture of torch and the standard
  library.
- **The torch row has a class prefix now**, `torch.nn`, and `Family::Torch` no
  longer requires `instances == 0`. That is what lets a whole module read, and
  it is the narrowest prefix that does: a class from anywhere else under torch
  is still a non-match.

## An array of objects holds more than leaves: landed on 2026-09-19

A pandas column of objects holds whatever Python was holding: dates, lists,
tuples, exact numbers, and the missing entries between them. Until now such an
array held leaves only, so a frame with one of those columns matched nothing.

What changed is where the values are read. The list an array of objects is
handed is an ordinary list, created empty and filled by the opcodes after it,
so it is now read against the same stack the rest of the file is read against.
`Cursor::object` was split into `Cursor::step`, one opcode at a time, and
`Cursor::filled_list`, which seeds that stack with the list and steps until the
array's shape says the list is full. Both are in `familiar/basic.rs`;
`familiar/numpy.rs` calls the second where it used to call a leaf reader of its
own.

Three things follow from reading the values through the one stack rather than
beside it:

- **The widening is the form's, not the array's.** A value in the array is
  whatever that file's form allows anywhere else, with the same depth bound,
  the same work budget, the same batch lengths and the same memo. A class from
  a module the form does not list is refused at `may_name` wherever it is
  written, so `unfamiliar-frame-of-instances-p4.pickle` and `-p2` match
  nothing.
- **The mixed form picks up the frames that need it.** A column of
  `datetime.date` or of `decimal.Decimal` uses the standard library's
  productions, so the file holds two extensions and is read under the mixed
  form; a column of lists is containers of plain values, which every form
  already reads, so that frame keeps the pandas form.
- **joblib follows with no work.** The nested pickle joblib writes where an
  array of objects would have had its numbers is read by the same grammar, so
  `joblib/pandas-frame-of-dates.joblib` reads for the same reason.

One bound had to be added. An array of objects inside an array of objects is
read from inside the run that made the outer one, which is recursion on the
program's own stack as well as depth in the tree, so `Cursor::nesting` counts
the open runs and `Cursor::too_deep` adds it to the depth a fold is checking.

The frame table shows such a cell the way the tree's own row shows the value:
`pickle_text` first, and then the few-word reading from `picklesaid.rs`, so a
date reads as its ISO spelling and a list as `list of 3`. The column header
says `object` rather than `str` where the values are not all text, which is
pandas' own word for the dtype.

## joblib: landed on 2026-09-19

`joblib.dump` writes a pickle with each array's bytes in the stream after a
small object describing them, so it is not a pickle the plain walk can read.
Four forms now read one: `joblib-arrays-p4-p5-v1` and `joblib-arrays-p2-p3-v1`
for arrays and the data around them, `joblib-sklearn-p4-p5-v1` and
`joblib-sklearn-p2-p3-v1` for a model saved the way scikit-learn's own
documentation says to save one. Every sample in `joblib/` reads but the object
array, which is under `does-not-read`. The whole of it, including the two
things the design note guessed wrong, is in `docs/DESIGN-pickle-containers.md`
under "joblib.dump: what landed".

**How the family was added**, which is the recipe above with two things more. A
form may now allow the joblib production, which is one `bool` on `Allow` and on
`Declared`, and a form that allows it requires the file to hold one. And a
family may have no name at a protocol range: the `Declared` row's `ids` holds
the empty string there and `forms()` leaves it out, which is what a family
built with `NEWOBJ` needs. The scikit-learn row and the joblib-scikit-learn row
share one `SKLEARN_CLASSES` and one `SKLEARN_CALLS` rather than either of them
holding a copy.

## Extensions compose: landed on 2026-09-19

A form allowed one extension, and a pickle that mixed two matched
nothing: `{"when": datetime, "weights": ndarray}` was refused by the NumPy form
at the date and by the standard library's at the array. Six ordinary files
written with numpy 2.5, pandas 3.0 and scikit-learn 1.9 were the measurement,
and five of the six read as nothing at all.

There is now one more form per protocol range, `mixed-values-p4-p5-v1` and its
three lower names, whose class prefixes, named globals and enumerated calls are
the union of every family's. The union is gathered from `DECLARED` itself, so a
family added there is in it with no second edit. `DESIGN-familiar-pickle-forms.md`
has the whole of it under "Extensions compose: the mixed form", including why a
union of enumerated sets is still an enumerated set and what was checked before
believing that.

Three things are worth carrying forward:

- **It is tried last and it requires two extensions.** Being last keeps every
  file that already had a name. Requiring two is what keeps every file that had
  none: the widest thing the union allows that no single form does is an array
  of pickled objects, and a file of nothing but one of those is one extension
  and stays a non-match. The verdict over `pickle/`, `pickle-matrix/`
  and `joblib/`, 1,076 files, is byte for byte what it was.
- **`Allow::joblib` is three-state now** (`Wrapped::Refused`, `Required`,
  `Allowed`) rather than a `bool`. The two joblib forms are *for* what
  `joblib.dump` writes and hold the file to having a wrapper; the mixed form
  only permits one. So `joblib/stdlib-and-arrays.joblib`,
  `joblib/pandas-frame.joblib` and `joblib/scipy-csr-matrix.joblib` read now,
  and the plain array and estimator files keep their own names.
- **The header says `form extensions` under `form`.** `Extensions` in
  `packs.rs` is a bitset of which of them the file used, put back on a rewind
  like every other counter; `extension_of` answers which extension a module
  belongs to by asking the same `DECLARED` prefixes the forms are declared
  with, so the two cannot drift. The row is on every matched file, leaves out
  the basic grammar because that is the form itself, says `none` for a file
  that used nothing else, and reads `builtins, stdlib, numpy, pandas` for a
  frame with a note and a date. It was called `families` until 2026-09-19.

`tools/make_mixed_pickle_samples.py` in the collection writes the twelve
`pickle/mixed-*.pickle` files, six objects at protocol 4 and at protocol 2.

## torch: landed on 2026-09-19

`torch.save` writes a ZIP holding a protocol 2 pickle whose tensors name their
numbers with a persistent id, and the numbers are other entries of the same
archive. Two forms read the pickle, `torch-tensors-p2-p3-v1` and
`torch-tensors-p4-p5-v1`, and a template called `torchzip` reads the archive
around it. Every sample in `torch/` reads but the legacy file. The whole of
it is in `docs/DESIGN-pickle-containers.md` under "torch.save: what landed",
including the three things the design note guessed wrong.

**How the family was added**, which is the recipe above with three things
more.

- **A form may allow a production that names no class.** A tensor's storage
  class is named inside the tensor's own fixed run and never reaches the tree,
  so the torch row's `classes` is empty and `collections.OrderedDict` is named
  through the calls table rather than as a package classes may come from.
  That is what keeps a state dict a torch file rather than a mixture of torch
  and the standard library, and it needed one change in `holds_class`: a
  callable a `Made` carries is the thing an enumerated call named in order to
  be called, checked against the list when the REDUCE was read, and not a
  class handed to the reader as data.
- **A call's contents and its attributes are two slots.**
  `nn.Module.state_dict()` is an `OrderedDict` filled by `SETITEMS` and then
  given a `_metadata` attribute by `BUILD`, so `Kind::Made` has `attrs`
  beside `state` and the node shows an `attributes` row under the entries.
  Every checkpoint saved the ordinary way needs it.
- **A template may place a pickle inside an archive.** `torchzip` is the
  archive's records and one `Ty::Schema` node that puts `T::pickle()` at the
  `data.pkl` entry's own bytes, which is `adios/dataset.rs`'s move. The pickle
  and the storages then share one space, which is what lets a tensor's table
  read the entry the tensor named.

`cargo test -p qubero-core --test torch_real` is the real-files test, ten of
them, and it asserts the tables cell for cell against what the generator
wrote.

## scikit-learn breadth: landed on 2026-09-19

A `LogisticRegression` fitted on labels that are strings did not read, and
`LogisticRegression` is the model saved more often than any other. That one
file was the measurement that said the earlier pass had been tested on the
estimators it happened to have rather than on the ones people save. So the
sweep: `tools/make_sklearn_breadth_samples.py` in the collection fits
twenty-seven estimators on sixteen rows of three features and writes each one
twice, once with `pickle.dumps` at protocol 4 and once with `joblib.dump`,
which are the two ways scikit-learn's own documentation says to save a model.

Before the sweep, fourteen of the twenty-seven read plainly. After it,
twenty-six do, and the one that does not is written down below with its
reason.

| Estimator | Plain | joblib |
| --- | --- | --- |
| LogisticRegression | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| LogisticRegression on string labels | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| Ridge, Lasso, LinearSVC, GaussianNB | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| SGDClassifier | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| SVC | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| KNeighborsClassifier | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| DecisionTreeRegressor, RandomForestClassifier | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| GradientBoostingClassifier | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| HistGradientBoostingClassifier | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| KMeans, PCA, StandardScaler, MinMaxScaler | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| OneHotEncoder, LabelEncoder, SimpleImputer | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| TfidfVectorizer | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| CountVectorizer | `sklearn-estimator-p4-p5-v1` | `sklearn-estimator-p4-p5-v1` |
| Pipeline of a scaler and a model | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| MLPClassifier | `sklearn-estimator-p4-p5-v1` | `joblib-sklearn-p4-p5-v1` |
| ColumnTransformer | `mixed-values-p4-p5-v1` | `mixed-values-p4-p5-v1` |
| KNeighborsClassifier on a sparse matrix | `mixed-values-p4-p5-v1` | `mixed-values-p4-p5-v1` |
| GridSearchCV | no form | no form |

A `CountVectorizer` holds no array at all: its vocabulary is a dictionary of
words to positions. So there is no wrapper for `joblib.dump` to write and the
file is byte for byte the plain pickle, which is the form it reads as. The two
`mixed-values` rows are files holding a second family beside scikit-learn's: a
column transformer places its columns with `builtins.slice`, which is the
language's own value, and a nearest-neighbour model fitted on a sparse matrix
keeps that matrix, which is scipy's.

**What the non-matches turned out to be**, seven causes across thirteen files,
each found by running `pickle_forms` over the sweep and reading the bytes at
the offset it stopped:

- **A dtype of fixed-width text.** `numpy.dtype('U3')` is three characters and
  twelve bytes, and unlike every other plain dtype the width is in the state
  rather than in the letters: `S5` writes `5, 1, 0` where a number's dtype
  writes `-1, -1, 0`, and `U3` writes `12, 4, 8`. `dtype.rs` reads those now
  and holds the width in the state to the width the letters come to. This is
  what the reported file was stopping at, and what a `LabelEncoder` is made of.
- **A NumPy scalar type named and never called.** `OneHotEncoder(dtype=numpy.float64)`
  and `CountVectorizer(dtype=numpy.int64)` keep the type they will make their
  numbers in as a plain setting, so it reaches the file as a global that
  nothing calls. `numpy::TYPE_NAMES` enumerates the twenty-seven of them by
  their whole dotted path and the scikit-learn, scipy, pandas and joblib rows
  name it, which is the same `names` mechanism a `defaultdict`'s factory goes
  through. `numpy` is still a package no class at all may be named from.
- **An array whose numbers are the run an earlier array wrote.** A fitted
  `SVC` writes `_probA` and `_probB` both empty, which is one byte string to
  Python, so the second array is a BINGET where the run would be. `numbers`
  in `numpy.rs` reads that at protocol 3 and up, and the node says
  `written as: bytes an earlier array wrote` rather than placing a run that
  sits outside it.
- **The Cython losses.** `sklearn._loss._loss.CyHalfBinomialLoss()`,
  `sklearn._loss.link.LogitLink()` and
  `sklearn.linear_model._sgd_fast.Hinge(1.0)` are Cython classes whose
  `__reduce__` hands the class back with nothing or with the one number it was
  configured with. Fifteen of them are enumerated in the new
  `familiar/sklearn.rs`, each by its whole dotted path and with the argument
  shape measured against scikit-learn 1.9's bytes.
- **`newObj`.** A Cython class has no `__new__` a pickle can reach through
  NEWOBJ, so the module keeps a one-line function that does it. Three modules
  have one: `sklearn.neighbors._kd_tree`, `sklearn.neighbors._ball_tree` and
  `sklearn.metrics._dist_metrics`. Each takes one argument, the class, which
  came through the same `may_name` check as every other class, and the BUILD
  after it is the state exactly as for an estimator.
- **What `random_state` holds after a fit.** A `RandomState` or a `Generator`
  is one call over a bit generator and the bit generator is one call over its
  class: `__randomstate_ctor`, `__generator_ctor` and `__bit_generator_ctor`,
  all in `numpy.random._pickle`, with `__pyx_unpickle_SeedSequence` beside
  them for a `Generator`'s seed. The five bit generator classes and the seed
  sequence are named and never called. The rows are declared in `numpy.rs`
  because they are NumPy's, and listed in `SKLEARN_CALLS` because that is
  where a file holds one. `numpy::TYPE_NAMES` is now a list of every NumPy
  global a form may name rather than of scalar types alone, so its name is one
  word narrower than what is in it.
- **A structured dtype naming a column out of the memo.** A histogram
  gradient boosting model's nodes have an `is_categorical` column, and the
  estimator has already written that word, so NumPy hands the pickler one
  string for both. `column_name` in `dtype.rs` reads a reference there.

One row moved that no file in the collection noticed: the plain scikit-learn
form has `object_arrays: true` now, which the joblib row over it already had.
`GradientBoostingClassifier.estimators_` is an array of objects, one tree per
stage, so the plain pickle used to fall to the mixed form while its joblib
twin read as scikit-learn's. The verdict of all 1,433 files in `pickle/`,
`pickle-matrix/`, `joblib/` and `torch/` is otherwise byte for byte what it
was.

**What is still refused, and why.** `GridSearchCV.cv_results_` is a dictionary
of `numpy.ma.MaskedArray`, which is an array, a mask of which of its entries
count and a fill value, rebuilt by `numpy.ma.core._mareconstruct(MaskedArray,
ndarray, (0,), b'b')` and finished by a BUILD whose state is a seven-part
tuple: the version, the shape, the dtype, the storage order, the numbers, the
mask and the fill. Reading it as a call with an opaque tuple under it would be
easy and would show bytes where the numbers are; reading it properly means a
production of its own that reads the data and the mask as the two arrays they
are, which is what the sparse matrix table wants as well. The samples are
`pickle/unfamiliar-sklearn-grid-search-cv.pickle` and
`joblib/does-not-read/sklearn-grid-search-cv.joblib`.

Two smaller things left where they are:

- **A `CyHalfMultinomialLoss` is written the Cython `__pyx_unpickle_` way**
  rather than as a call of its class, which is one `__pyx_unpickle_<Name>`
  function per class and so a list that grows with every class. Only a
  multiclass boosted model holds one; the binary one in the collection does
  not.
- **A table over an array whose numbers are elsewhere.** The node says where
  the run is and does not place it, so there is no run under it for a table to
  walk. Both files in the collection that do this hold empty arrays, so there
  is nothing to show either way. A decoded run as a space of its own is the
  change that would give such an array a table, and it is the next item under
  "What is left".

## A decoded run as a space of its own: landed on 2026-09-19

A protocol 2 array written by Python 3 stores its numbers as latin-1 text,
handed to `_codecs.encode`; at protocol 0 that text is escaped again to fit on
a line. Before this the recogniser decoded such a run once and kept the bytes
beside the match, the tree carried a `written as` note, and the table over the
array had its cells worked out by the core with no byte addresses under them.
Now the run opens the way every other packed run in a file opens.

**What a reader sees.** The `numbers` row is the run in the file, at the
offset the pickler wrote it, typed `latin-1 text` or `latin-1 text, escaped`.
Under it, in a space of its own, is the run of numbers: `f32 le[]` at byte 0
of that space, with a value at every ordinary offset. The table hangs on the
numbers rather than on the run, and "Show byte addresses" over it says where
each value is in the decoded bytes. The `written as` note is gone, because the
node's own type says what it was decoded from; the note stays for the one
thing a type cannot say, an array naming the run an earlier array wrote.

**What it is made of.**

- `codec/pytext.rs` holds both decoders and the `raw-unicode-escape`
  un-escaper, which `familiar/lines.rs` calls too: one reading of a line,
  wherever it is asked for. `Codec::Latin1Text` and
  `Codec::EscapedLatin1Text` are three arms in `codec.rs` and nothing
  anywhere else, as the spike measured. The trace is a step a character, with
  runs of characters written as themselves coalesced, so a selection in the
  space maps back to the bytes of the run that spelled it; past the step
  budget it falls back to one step over the whole run.
- `pickletree::place_pickle_child`'s `Part::Data` arm returns
  `T::decoded(len, codec, numbers_ty(..))` for a run that is not `Storage::Raw`.
  A bare `Ty::Decoded` has no length of its own, which `Ty::decoded()` handles
  by wrapping it in a `Ty::Sized`.
- `Evaluator::pickle_table` answers for the numbers node rather than the run,
  and `table.rs::table_at` routes a `Ty::Decoded` parent to it. A table over
  the run itself would be a one-row table, since the run holds one thing.
- `pickleparts::locate` is the inverse of `spot`: it walks the tree down to
  the node whose run starts at a byte, which is what a frame's cell reader
  needs to open a space with. `Numbers` carries that path instead of the
  decoded bytes, and `picklecells::space_value` opens the space and reads the
  cell out of its buffer. A frame's cells are still `Cells::Computed`, since a
  frame is assembled across blocks, but they are read through the spaces.
- Nothing in `web/src` was keyed on a codec name or on `written as`, so the
  interface needed no change.

**The Python 2 arrays are not this.** Python 2 had a type for a run of bytes,
so its arrays go out as `BINSTRING` and the numbers are the bytes. Those open
no space and read as they did.

**What a frame's cells still cannot do** is carry an address. `computedPlan`
in `web/src/tableplan.ts` hands every computed row `offsetBits: 0`, and a
frame's row is not one run anyway: its cells are in different blocks and now
in different spaces. Giving them addresses means a `(space, offset, size)` per
*cell* rather than per row, through the wasm binding, `computedPlan` and a
`tableview` whose two address columns are per row. That is a job of its own.

**What did not come for free.** The earlier note said the GraalPy file would
read once nothing was replaced. It would not. GraalPy hands back one object
for two equal strings, so the second `_codecs.encode` of the same packed date
is a `BINGET`, and `encoded_text` read that reference as a counted text and
held its run to being UTF-8. At protocol 0 the run is a line, and a line is
not always the text it stands for. `lines::is_named_text` is the one rule
that says which, by the protocol and the run: the recogniser asks it before it
builds the value, and `familiar::named` asks it again when the reading wants
the bytes. The
matrix now reads with no exceptions and `UNREAD` in `pickle_real.rs` is empty.

**What was left alone, and why.**

- *A `Kind::Spelled` line as a space.* An escaped text line already shows its
  transformation: the node's value is what the line spells and its `line` row
  is the run the file holds. Making it a space would put a level and an
  "Open unpacked" affordance under every protocol 0 string with an accent or
  a backslash in it, which in a file of strings is most of the tree, and would
  show the reader nothing the two rows do not.
- *A packed date at protocols 0 to 2.* Its run is not a date's run: it is the
  text argument of a general `_codecs.encode`, the same one a `bytes` value
  and a `bytearray` are written with. Opening it for dates alone would make a
  date's run a space while an identical byte string beside it stayed text. A
  date has no field structure at any protocol either: at protocol 4 the run is
  a `packed` row of bytes with the date worked out on the row above, so the
  space would open onto bytes, not onto a year and a month. The change
  worth making is the general one: a byte string below protocol 3 opens as the
  bytes it spells, whatever holds it. That wants the `text` argument of a
  `Shape::Bytes` call placed as a decoded node, an inner type per holder, and
  tests for each.

## Not decided

- The legacy torch file, which is a run of five pickles and then the
  storages. `is_pickle` refuses it because the first STOP is not the end, and
  sizing a field at a pickle's STOP is something the IR cannot say. See
  "What is left for torch" in `docs/DESIGN-pickle-containers.md`.

  One guess in the paragraph this replaces turned out wrong and is worth
  keeping: a checkpoint does **not** fall to the mixed form. The torch row
  reads plain values around its tensors the way the joblib rows read plain
  data around their arrays, so `{"epoch": 3, "loss": 0.125, "model":
  OrderedDict, "note": str}` is `torch-tensors-p2-p3-v1`. A checkpoint holding
  a date or a NumPy array beside its weights is two families and falls to the
  mixed form, which is where that guess was right.

## Older releases of torch and joblib: landed on 2026-09-19

A second matrix run, nine container environments: eight with both libraries,
from torch 0.4.1 with joblib 0.11 to torch 2.14 with joblib 1.6, and one with
joblib 0.9.4 alone. What each
era wrote and where in the writer each difference lives is in
`docs/DESIGN-pickle-containers.md` under "What each era wrote". Every file of
that run reads now but two, which the same section names.

**How the families were widened**, which is the recipe above with four things
more.

- **An older spelling is a new enumerated row, never a looser one.** torch 0.4
  wrote a parameter as `Parameter(tensor, requires_grad)` and torch 1.0 wrote
  `_rebuild_parameter(tensor, requires_grad, OrderedDict())`; both are
  productions of their own in `familiar/torch.rs` and each matches its own run
  exactly. The one place a production was widened rather than added is the
  backward hooks, which are `None` or an empty `OrderedDict()` and nothing
  else.
- **A `NEWOBJ` with arguments may close an enumerated call.** `Via::NewObj`
  beside `Via::Global` and `Via::Partial`, and `new_object` looks for a row of
  that kind before it falls through to `cls.__new__(cls)`. The tail of
  `reduced` became `call_made` so the two opcodes make the same value.
- **A persistent id may name a class.** A legacy `torch.save` of a whole
  module writes `('module', cls, source_file, source)`, and what comes out is
  the class, with the source text as a row inside the run. It opens with a
  MARK rather than with a name, so it is tried in `basic.rs`'s opcode loop
  rather than among the productions `push` reaches.
- **A form may read an array that is not in the file.** joblib before 0.10
  wrote each array as a `.npy` beside the pickle, so `Shape::ArrayFile` is a
  value whose one row is the file to open, `Allow` and `Declared` gained a
  `beside: Wrapped` column beside `joblib: Wrapped`, and the two layouts are
  named apart: `joblib-npy-files-*` against `joblib-arrays-*`.

One fix outside the forms, and it is the one that would have gone unnoticed:
`zip_directory_names` in `recognise.rs` found the central directory by
subtracting its length from the end record. torch 1.5 writes the ZIP64 end
records in between, so the walk started inside the last entry and the archive
read as a plain ZIP. The end record's own offset field says where the
directory is, and that is what is read now. `eval/pickletorch.rs` already did
it the right way.

## Every computed cell says where its bytes are: landed on 2026-09-19

Three of the tables the reader gets are worked out by the core rather than laid
out as a run of fields: a pandas frame or series, a torch tensor, and the
summary over a state dict. Until now their cells carried no address at all,
and `computedPlan` in `web/src/tableplan.ts` handed every row `offsetBits: 0,
sizeBits: 0`, so with "Show byte addresses" on every row of a frame said
`@0x0 · 0 bytes`.

The address is on the cell rather than on the row, because a row of one of
these tables is not a run of the file. A frame's row is one value out of each
of several blocks; at protocol 2 two cells of one row can even be in two
different spaces, since the numbers were spelled as latin-1 text and the
strings beside them were not. `DESIGN-pickle-containers.md` has the `CellAt`
type and what each table's cells carry.

What the view does with it, when the addresses are on:

- A row whose cells are one run of one space is drawn as before: where it
  starts, and how long it is.
- A row whose cells are in several places shows the first cell's address and
  `per cell` for the size, with "This row's cells are in different places.
  Hover a cell for its address." on both.
- A row with no bytes at all says which nothing it is: `computed` for a
  counted index, `not stored` for a fact the pickle states, `unknown`
  otherwise.
- Every cell's hover gains a line: its address and size, or the reason it has
  none.
- Clicking a cell selects that cell's bytes in the hex view. A cell inside an
  unpacked stream, or with no bytes, leaves the cursor where it is rather than
  moving it somewhere the reader did not click. Shift-click still extends the
  row selection.

With the addresses off nothing changed: no address column, and a cell's hover
is its text and any problem on it, as before.

`crates/core/tests/cells_real.rs` checks the addresses against the bytes they
name: it reads the run each cell points at and compares it with what the cell
says. `crates/core/tests/joblib_real.rs` is the joblib half of
`pickle_real.rs`, split out unchanged in the commit before it.

## A named run is read where it was written: landed on 2026-09-20

`cargo test -p qubero-core --test kinds_real` failed on four GraalPy files:
`stdlib-datetime.p1.pypickle.pickle`, `stdlib-datetime.p2.pypickle.pickle`,
`stdlib-datetime-aware.p1.pypickle.pickle` and
`stdlib-datetime-aware.p2.pypickle.pickle`. Each counted about 40% more bytes
than it holds.

**The cause.** GraalPy hands back one object for two equal strings, so the
second `_codecs.encode` of the same packed day is a `BINGET`.
`Cursor::encoded_text` came back from that reference with `Kind::Text` holding
the run it names, which is a value whose bytes are a hundred bytes away from
the value itself. Two things followed. The `text` row was placed on the first
date's run, so that run was counted twice. And `pickleparts::parts` lays a
node's children out in file order and fills what is between them with the
instructions there, so a child before its parent sent the cursor backwards and
the fill after it named every instruction from the run's end to the parent's,
most of the file, as bytes of one byte string.

**The fix**, which is the convention the rest of the tree already keeps: a
reference is read as the reference it is. `encoded_text` returns
`Kind::Ref(Names::Text)` at every protocol now rather than only at protocol 0,
so the row is the two bytes of the `BINGET` with a `refers to` row saying what
is at the other end, and the run is counted once, under the date that spelled
it. `Cursor::exact_word` was already written this way. `Cursor::encode_call`,
which is where an array's numbers reach protocol 2, takes a reference of a run
that is the text it stands for and reads the numbers there, so the arrays are
byte for byte what they were.

Nothing else had to change: `Cursor::packed_bytes` and
`Evaluator::packed` already read both spellings, and `familiar::named` is
still the one reading of a run a reference names.
`a_date_naming_an_earlier_date_s_run_reads_it_and_counts_nothing` in
`familiar/tests/stdlib.rs` is the claim, over two protocol 2 dates of the same
day. `kinds_real` is worth adding to the merge gate: it is the only test that
would have caught this, it walks every file in the collection and it takes
about nine minutes.

## NumPy's own array classes and masked arrays: landed on 2026-09-20

Two things NumPy writes that no form read. `DESIGN-familiar-pickle-forms.md`
has the whole of both, under "NumPy's own array classes" and "A masked array
is two arrays and a fill value"; what belongs here is what moved.

**The array class is enumerated now.** `_reconstruct` is handed
`self.__class__`, and the production named `numpy.ndarray` and no other.
`numpy::ARRAY_CLASSES` is the list: `numpy.ndarray`, `numpy.matrix` and
`numpy.memmap`, each by its whole dotted path and each with the word the node
goes by. `Kind::Array` and `Kind::Objects` carry it, `shape_of` returns it,
and the memo files a value under the class it is, so a reference to a matrix
says `matrix at 0x...`. The joblib wrapper's `subclass` key reads the same
list, which is what `joblib/v1.6-numpy-matrix.joblib` needed.

`numpy.memmap` was guessed to reduce to a plain ndarray. It does not:
`__reduce__` is `ndarray`'s and writes `self.__class__`, so NumPy 2.5 writes
`_reconstruct(numpy.memmap, (0,), b'b')` with the numbers in the pickle and
nothing about the file it was mapped from. Measured, not read off the note.

`numpy.rec.recarray` is left, and the reason is in the design document: its
dtype is a class where every other dtype is letters.

**A masked array is a production of its own**, `Cursor::mareconstructed` in
`numpy.rs`, tried where `reconstructed` is tried, so every form that reads
arrays reads masked ones. `Kind::Masked` holds the data, the mask and the fill
as three values; the first two are `Kind::Array`s over their own runs, so the
tree, the spaces at protocols 0 to 2, the addresses and the per-array tables
all come from the machinery that was already there.

**What moved.** `pickle/unfamiliar-sklearn-grid-search-cv.pickle` is
`pickle/sklearn-grid-search-cv.pickle` and matches `sklearn-estimator-p4-p5-v1`;
`joblib/does-not-read/sklearn-grid-search-cv.joblib` is
`joblib/sklearn-grid-search-cv.joblib` and matches `joblib-sklearn-p4-p5-v1`;
`joblib/does-not-read/v1.6-numpy-matrix.joblib` is `joblib/v1.6-numpy-matrix.joblib`
and matches `joblib-arrays-p4-p5-v1`. `joblib/does-not-read/` is gone, because
nothing in `joblib/` is refused any more. Twelve new samples in `pickle/`,
written by `tools/make_numpy_subclass_samples.py`: a matrix, a memmap and four
masked arrays, each at protocol 4 and at protocol 2. No other verdict in
`pickle/`, `pickle-matrix/`, `joblib/` or `torch/` moved.

**One row name changed**: `ndarray reconstruct call` is
`array reconstruct call`, because the call rebuilds whichever of the three
classes the file named.

**The masked array's table landed on 2026-09-20**, a day after the rest of
this. `Evaluator::pickle_table` gives the masked node `Cells::Computed { rows }`
and `Evaluator::masked_cells` in `picklecells.rs` reads it: the run chunked
into rows the way an array's own table chunks one, each value through
`Evaluator::number_at` so a protocol 0 or 2 array reads through the space its
numbers opened, and the mask through `Evaluator::mask_at` over the `|b1` run
beside it.

A masked cell is `FrameCell { value: None, at: CellAt::Bytes { .. }, masked: true }`.
It shows empty and **keeps its address**, because the number is in the file and
the array only says not to count it: a reader who wants the stored number
clicks the cell and the hex view selects those bytes. That is a different fact
from a cell the file has no value for, so it is a field of its own rather than
a third `kind`, and the interface says which: the hover reads
`Masked: the array does not count this value.` above the address line. A mask
this reading cannot read leaves every cell showing its number, rather than
blanking a table on a doubt.

`a_masked_array_s_hidden_cells_are_empty_and_still_say_where_they_are` in
`cells_real.rs` checks the four samples cell for cell at both protocols, and
`a_fitted_search_s_masked_arrays_read_as_their_numbers` does the same over
every masked array in `pickle/sklearn-grid-search-cv.pickle`, which is what the
production was written for.

## joblib 0.9's last two pieces: landed on 2026-09-20

`/home/pengo/qubero-container-matrix/py3.6-joblib0.9/` is joblib 0.9.4 under
Python 3.6, and two of the files it wrote had no reading.
`DESIGN-pickle-containers.md` has both under "What each era wrote"; what
belongs here is the shape of each change.

**An object array under the NumPy form.** joblib 0.9 wrote a wrapper only
where there were numbers to write, so an array of objects reached the stream
as an ordinary NumPy pickle with nothing of joblib's around it. The question
was whether the NumPy form should read one at all. It should: an array of
objects is what `pickle.dumps` writes for one, with no library anywhere near
it, and the mixed form is for a file holding two families rather than for a
file holding a kind of array. `object_arrays: true` on the NumPy row is the
whole change.

**Exactly one verdict moved**, and the previous agent's reason for leaving it
turned out not to hold: `pickle/proto4-numpy-object-array.pickle` was the
opcode listing and is `numpy-array-p4-p5-v6`. The `mixed-array-of-tuples`
files did not move, because each of them holds a date beside the array and is
two families whichever way the flag goes.

**`joblibzfile`**, in `crates/core/src/formats/joblibzfile.rs`: `ZF`, the
unpacked length as text, and a zlib stream. Registered in `formats/mod.rs`,
`recognise.rs`, `web/src/filetype.ts` and `web/src/identity.ts`. The probe
reads the whole header rather than the two bytes, because `ZF` on its own
would claim anything: the length has to be what `hex()` writes padded with
spaces, and a zlib stream has to begin exactly where that field ends.

`joblib/v0.9-array-of-objects.joblib` and
`joblib/v0.9-dict-of-arrays-zfile.joblib` are the samples, and
`a_joblib_0_9_compressed_file_is_its_own_container` and
`a_joblib_0_9_object_array_has_no_wrapper` in `joblib_versions.rs` are the
claims.

## Two splits, on 2026-09-20

Neither changes what anything reads: `cargo run --example pickle_forms` over
`pickle-matrix/`, `pickle/`, `joblib/` and `torch/` is byte for byte what it
was, and `torch_real`, `torch_versions`, `cells_real` and `pickle_real` are
unchanged.

**`crates/core/src/formats/zipdirectory.rs`** is the ZIP central directory
read from the end of a file: each entry's name and the run its data is. It was
about 180 lines inside `eval/pickletorch.rs`, which is a file about tensors,
and it is 603 lines there now rather than 730. Reads go through a
`&mut dyn FnMut(u64, u64) -> R<Vec<u8>>` the way `torchlegacy::layout` does, so
the evaluator hands one that may answer `Pending` and the module itself is
testable against bytes in hand. Five unit tests, including the one the move was
for: torch 1.5 writes the ZIP64 records between the directory and the end
record although every number fits without them, so the end record's own offset
field is what says where the directory is.

**`zip_directory_names` in `recognise.rs` was left where it is**, and the
reason is written above it. It answers a different question: a sniffer has a
window rather than a file, wants the names only, and wants nothing at all when
the directory is not in the window. The shared reader resolves each entry's
*data*, which only the local header says, so it reads one header per entry and
passes over an entry whose header it cannot reach -- which in a sniffer would
silently drop the very names it is deciding on. The one thing the two used to
disagree about is where the directory begins, and both read it out of the end
record's own field now.

**`crates/core/src/eval/picklesummary.rs`** is `pickle_summary`,
`index_summary`, `sparse_summary` and `iso_time`, out of `picklecells.rs` and
beside `picklesaid.rs`. Every one of them ends in a string a reader sees;
`picklecells.rs` is 477 lines now rather than 690 and is about bytes.
