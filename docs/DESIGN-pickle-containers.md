# torch.save and joblib.dump: pickles with the numbers kept outside

Written 2026-09-19 from the bytes of files made with torch 2.14 and joblib 1.6
(`tools/make_torch_joblib_samples.py` in the sample collection). joblib is
built, later the same day; torch is not. Read
`DESIGN-familiar-pickle-forms.md` first: both formats are a Familiar Pickle
Form with one more thing to say, which is where the numbers are. The rule does
not change. Nothing is run; every global and every call is named with its exact
shape; anything else is the opcode listing.

**What landed for joblib, and what the note below had wrong**, is under
"joblib.dump: what landed". Read it before the paragraphs above it: those were
written from a first reading of the bytes and two of their guesses are wrong.

The owner wants both supported. The goal is the data: a checkpoint's tensors and
a joblib file's arrays open as tables, as a NumPy array in a plain pickle does.

## torch.save

### The ZIP, since torch 1.6

An archive of stored entries (method 0, never compressed), each aligned to 64
bytes, under one folder named for the file:

| Entry | What it is |
| --- | --- |
| `<name>/data.pkl` | the pickle, protocol 2 unless the caller chose another |
| `<name>/data/0`, `data/1`, ... | one storage each: the raw bytes of the numbers, in the byte order `byteorder` says |
| `<name>/version` | `3\n` |
| `<name>/byteorder` | `little` or `big` |
| `<name>/.format_version`, `.storage_alignment`, `.data/serialization_id` | `1`, `64`, a 40 character id |

`data.pkl` is what CPython's C pickler writes (memo numbered from 0, globals in
the memo), with one opcode the forms refuse today, `BINPERSID`. A tensor is:

```
GLOBAL torch._utils _rebuild_tensor_v2
MARK
  MARK 'storage' GLOBAL torch FloatStorage '0' 'cpu' 12 TUPLE BINPERSID
  0              storage offset, in elements
  (3, 4)         size
  (4, 1)         stride, in elements
  False          requires_grad
  OrderedDict()  backward hooks, always empty in a saved file
TUPLE REDUCE
```

The persistent id is a fixed tuple: the word `storage`, the storage class (which
is the dtype: `FloatStorage` float32, `DoubleStorage`, `HalfStorage`,
`BFloat16Storage`, `LongStorage` int64, `IntStorage`, `ShortStorage`,
`CharStorage` int8, `ByteStorage` uint8, `BoolStorage`; newer files may name
`torch.storage UntypedStorage` with the dtype elsewhere, check), the key that
names the entry `data/<key>`, the device it was on, and the element count.
Nothing about a persistent id is risky when nothing is loaded: it is a tuple of
text and numbers followed by one opcode, and the form names every part of it.

So the torch family is: the basic values, `collections.OrderedDict` (the stdlib
family's filled call), `_rebuild_tensor_v2` with that exact argument shape and
that exact persistent id, and `_rebuild_parameter` around one. Enumerate others
only as the samples show them (`torch.Size`, `torch.device`, `torch.dtype`
values appear in optimizer state and checkpoints).

Recognition follows `zarrzip` and `adioszip` (`archive_by_names` in
`recognise.rs`): a ZIP whose central directory holds `*/data.pkl` and
`*/version` under one folder is `torchzip`. The template is the archive's own
records plus what the archive holds. The pieces that exist: `zip::records(true)`
for an archive read beside its contents, `T::decoded(.., Codec::Stored, ..)` for
an entry's bytes as a space with a template of its own, and the familiar form
reading a pickle inside a space. The piece that does not: a tensor's numbers are
in another entry. A tensor node needs the byte range of `data/<key>` in the
outer file, which is the local record of that name: offset of its data, plus
`storage offset * element size`. That lookup is by entry name, which is the
IR's known gap S6 (a template per ZIP entry, and a reference from one entry's
contents to another entry). Two ways through:

1. The core resolves it outside the IR, as `pickle_cells` already reads frames:
   the torch tree asks the archive's directory for the entry's data range and
   reads cells from the outer document. Least work, and the table has no byte
   addresses of its own to show unless the range is handed back too.
2. Close S6 properly: a node in one space that places a typed array in another
   space at a computed offset. Tensors would then be ordinary fields with hex
   view, selection and byte addresses, which is the project's "show every
   transformation step" principle. More work, and zarr-in-zip wants it too.

Take 1 first with the range exposed (so the hex view can jump to a tensor's
bytes), and write down what 2 needs when the shape of 1 is known.

A tensor's table: rows along the first axis of `size`, columns the second,
honouring `stride` and `storage offset` (a transposed view has strides that are
not C order; two tensors may share one storage). A 1-D tensor is one column, a
0-D tensor a single value. The entry row reads `float32 tensor 3 x 4`. The
summary at the top of a state dict: one row per tensor, `name`, dtype, shape,
element count, and the whole as a table is worth more than any tree.

### The legacy file, before 1.6 and with `_use_new_zipfile_serialization=False`

One file, five parts in order: a pickle of the magic number
`0x1950a86a20f9469cfc6c`, a pickle of the protocol version `1001`, a pickle of a
small dict (`protocol_version`, `little_endian`, `type_sizes`), the data pickle
(same tensor shape, persistent ids with a view tuple added), a pickle of the
list of storage keys, then for each key an 8 byte element count and the raw
bytes. So: a run of pickles then binary. `is_pickle` refuses it today because
the first STOP is not the end of the file. Recognise it by its first pickle,
which is always the same bytes, and read it as a structure of five pickles and a
list of storages, each storage placed where it is. This one needs no archive and
no cross-entry lookup: everything is in one space, so the numbers can be
ordinary typed fields.

## joblib.dump

One pickle stream, written by a subclass of the pure Python pickler (so the
`pickle.py` spelling, at protocol 4 by default on current Python), in which an
array is replaced by a small object and followed at once by its bytes:

```
... STACK_GLOBAL joblib.numpy_pickle NumpyArrayWrapper, EMPTY_TUPLE, NEWOBJ,
    state dict { subclass: numpy.ndarray, shape: (4, 6), order: 'C',
                 dtype: <the numpy dtype call>, allow_mmap: True,
                 numpy_array_alignment_bytes: 16 }, BUILD
<one byte: how many padding bytes follow> <that many 0xff bytes>   (aligns the data to 16)
<the array's bytes, shape x itemsize>
... the pickle carries on
```

The bytes after `BUILD` are not opcodes, which is why the walk stops there and
nothing recognises an uncompressed joblib file today. The form: the object
production over `joblib.numpy_pickle.NumpyArrayWrapper` (older files:
`joblib.numpy_pickle` without the alignment key and without the padding byte,
and before 0.10 a different layout with `.npy` files beside the pickle; accept
what samples show), and after its `BUILD` a fixed run the wrapper's own state
measures. The array is then exactly the NumPy array node that exists, with its
numbers in the file as they are, so the existing array table applies with no new
mechanism. An object array has no raw run: joblib pickles it in place.

`is_pickle` needs to learn the same thing, or the sniffer never reaches the
form: after a `BUILD` of that wrapper, skip the run it measures. Doing that in
the plain walk means the walk knowing one class by name. The honest alternative
is to let the familiar probe answer first for files that open like a pickle and
fail the walk, and leave the plain `pickle` listing to stop where it stops with
the rest shown as the array bytes they are.

With `compress`, the same stream sits inside zlib (the default, `compress=True`
or a level), gzip, bz2, xz, or raw LZMA. Those already open as compressed
streams with a decoded space, except raw LZMA (`5d 00 00 ..`), which nothing
recognises. Inside the space the stream is the uncompressed case, with one
difference to check: compressed files write arrays through the wrapper with
`allow_mmap` false and no alignment padding. The work is making the decoded
space's template the joblib form rather than text.

A scikit-learn model saved with joblib is the sklearn family with arrays held
this way, so it should fall out of the two together.

## joblib.dump: what landed, on 2026-09-19

The code is `crates/core/src/formats/pickle/familiar/joblib.rs` for the
production, one flag on `Allow` and two `Declared` rows in `forms.rs`,
`pickle::is_joblib` and `pickle::joblib_pickle` in
`crates/core/src/formats/pickle/mod.rs`, and one probe in `recognise.rs`. The
unit tests are `familiar/tests/joblib.rs` over two fixtures, and the real files
are three tests at the end of `crates/core/tests/pickle_real.rs`.

| Form | What it reads | Matched |
| --- | --- | --- |
| `joblib-arrays-p4-p5-v1` | arrays, and plain data around them | 9 of the 9 protocol 4 files in `joblib/` |
| `joblib-arrays-p2-p3-v1` | the same at the protocol the caller may ask for | `dict-of-arrays-protocol2.joblib` |
| `joblib-sklearn-p4-p5-v1` | an estimator whose arrays are written this way | both scikit-learn models |
| `joblib-sklearn-p2-p3-v1` | the same | no file in the collection yet |

There is no name at protocol 1 or 0: joblib builds the wrapper with `NEWOBJ`,
which arrived at protocol 2, so the two lower ranges of the `Declared` row are
the empty string and `forms()` leaves them out.

**The wrapper, exactly.** `STACK_GLOBAL joblib.numpy_pickle NumpyArrayWrapper`,
`EMPTY_TUPLE`, `NEWOBJ`, `EMPTY_DICT`, `MARK`, then six entries in the order
`__init__` sets them -- `subclass` (`numpy.ndarray` and nothing else yet),
`shape`, `order`, `dtype` (the numpy dtype production), `allow_mmap` (a flag,
either way round) and `numpy_array_alignment_bytes` -- then `SETITEMS` and
`BUILD`. After the BUILD: one byte saying how much padding follows, that many
`0xff`, and `product(shape) * itemsize` bytes of numbers. The value is the
`Kind::Array` a plain pickled array is, at the offset the numbers really sit
at, so the tree, `pickle_said` and the array table needed nothing added.

**The padding count is not read and believed.** joblib works it out from where
the byte itself sits: `alignment - ((offset_of_the_byte + 1) % alignment)`,
which is never nought, since a run already on the alignment is pushed a whole
alignment further. The form checks that, checks every padding byte is `0xff`,
and holds the alignment to a power of two no larger than 128. A missing
`numpy_array_alignment_bytes` key is a named variant: joblib 1.1 and older had
no such attribute and wrote no padding byte either, so the run starts at the
BUILD. The form reads that, and no file in the collection is one, so it is
untested.

**Three things the note above had wrong.**

- **A compressor changes nothing.** The guess was `allow_mmap` false and no
  padding inside one. It is not so in joblib 1.6: `self.buffered` is true only
  for `BinaryZlibFile`, which is the old `.z` path, and `numpy_array_alignment_bytes`
  is left out only when `file_handle.tell()` raises. Every compressed sample
  unpacks to a stream that is **byte for byte** the uncompressed file, the
  padding included, counted from the position in the unpacked stream.
  `a_compressed_joblib_file_opens_as_the_joblib_file_it_holds` is that claim.
  So there is one form and no compressed variant of it, and the only thing a
  compressor changes is where the form is read.
- **An object array is not "pickled in place" in any ordinary sense.** joblib
  writes `pickle.dump(array, handle, protocol=5)` into the stream after the
  BUILD: a whole pickle of its own, with its own PROTO, its own memo, its own
  framing and its own STOP, and then the outer pickle's STOP after that. That
  is a production nobody has written, and the one sample's values are a string,
  a `None` and an integer, which the existing array-of-objects production does
  not take either. `joblib/does-not-read/array-of-objects.joblib`.
- **The decoded space needed no work at all.** `Evaluator::template_for`
  already sniffs a stream whose declared template says only bytes, so the
  moment the `joblib` probe existed every compressed file opened as the joblib
  file it holds.

**Recognition is where the walk stops.** `is_pickle` is untouched: a joblib
file is not a pickle by its rule and never will be, since the opcodes run out
before the end. The `joblib` probe asks for three things, each cheap: the
protocol opener `80 02` to `80 05`, `joblib.numpy_pickle` named by a
`SHORT_BINUNICODE`, `BINUNICODE`, `BINUNICODE8` or `GLOBAL` among the opcodes
walked, and the walk ending inside the window rather than at the end of it.
That works on a head as well as on a whole file, so a joblib file of any size
is recognised from its first 36 KB and `web/src/doc.ts` needed no new upgrade
path. A file that is recognised and that no form reads keeps the name -- that
is what puts the object-array sample in `does-not-read` rather than nowhere.

The walk has to have given up *inside* the window, and not merely been cut off
by the end of it: every long pickle's walk is cut off by the end of the window,
so the weaker test would claim any long pickle holding the words
`joblib.numpy_pickle` as a string. Nothing real is lost by asking for the stop
itself, since the wrapper's name and the bytes that end the walk are two
hundred bytes apart. The one file that falls through is a joblib file with more
than 36 KB of plain data in front of its first array: that one is `Cut` to
`is_pickle`, so it sniffs as `pickle`, `doc.ts` reads the whole of it and
`is_familiar` says yes, and it opens as `picklefpf` under the pickle label. The
data is all there; only the name is the wrong one.

The template is `joblib`, the same `T::pickle()` reading `picklefpf` uses, so
that the File type dialog says `joblib file (familiar form)` rather than
calling it a pickle it is not.

**Raw LZMA is read now**, because the codec was already there:
`Codec::Lzma1` and `Packing::Lzma1` exist for 7z, so `formats/lzma.rs` is a
thirteen-byte header over them. Recognition holds the settings byte to the
`5d` every writer of that format emits rather than to the 225 the field allows:
a PlayStation texture opens `10 00 00 00` and passes every other test. A `.lzma`
with other settings still opens by its extension or by naming the template.

**What is left for joblib**, in the order it is worth doing:

1. The nested pickle after an object array's wrapper, which is the one sample
   in `does-not-read`. It wants a whole stream read with a memo, a framing and
   a protocol of its own, and it wants the array-of-objects production widened
   past text and `None`.
2. **A bundle holding a standard library value.** The two joblib rows carry no
   `STDLIB_CALLS`, so `joblib.dump({"trained_at": datetime.now(), "weights":
   arr}, path)` matches no form, and that is an ordinary thing to save. One
   more `Declared` row reusing `stdlib::STDLIB_CALLS` and its modules, or the
   stdlib calls added to the arrays row, which is the same question the plain
   families answer by having a row each.
3. `numpy.matrix` and `numpy.memmap`, which reach the same writer and would be
   named beside `ndarray`. No file in the corpus holds one.
4. A frame or a sparse matrix dumped this way, which is one more `Declared` row
   each, reusing that family's `classes` and `calls` the way the scikit-learn
   row is reused here.
5. Older joblib. The form is written for the layout 1.2 and later write, with
   the missing-alignment-key variant named; 0.9 and earlier wrote `.npy` files
   beside the pickle, which is a different format. Containers, the way the
   pickle matrix was made.

## Samples, and when they go into the collection

The `joblib/` samples are in the collection since 2026-09-19, eighteen of them,
with `array-of-objects.joblib` under `does-not-read`. The generator gained a
0-d array, an empty array, a list of three arrays and one small array that is
also a fixture in this repository.

`torch/` is still held back in the session scratch folder, because nothing
reads one and every one of them would fail `samples_real` for every session:
state dict as ZIP and legacy, one tensor, every dtype, shared storage and
views, a training checkpoint, protocol 4. Regenerate with the script (it writes
both folders into the collection root, so move `torch/` back out); commit them
with the change that reads them. The collection already has
`pickle/proto2-torch-state-dict.pickle`, a `data.pkl` on its own.

Older writers matter as much here as they did for pickle: torch 1.x ZIPs, torch
0.4 to 1.5 legacy files, joblib 0.9 to 1.x. Containers can make them the way the
pickle matrix was made (`pip install torch==1.13.1+cpu` and so on); do that
before calling either form done.

## Order of work

1. ~~joblib uncompressed: the wrapper production and the run after it.~~ Done.
2. ~~joblib compressed: the decoded space's template, and raw LZMA
   recognition.~~ Done, and the first of those needed no work.
3. The torch family in the pickle (persistent id, `_rebuild_tensor_v2`), so
   `data.pkl` on its own reads with tensors that say where their numbers are.
4. `torchzip`: recognition by names, the archive beside its contents, tensors
   reading their numbers from the entry they name, the summary table.
5. The legacy torch file.
6. Older versions from containers, and the samples into the collection.
