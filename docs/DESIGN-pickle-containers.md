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
**What landed for torch** is under "torch.save: what landed", on 2026-09-19,
and the same warning applies to the torch paragraphs above it: three of their
guesses are wrong and the section says which.

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
bytes. So: a run of pickles then binary. `is_pickle` refuses it because the
first STOP is not the end of the file. This one needs no archive and no
cross-entry lookup: everything is in one space, so the numbers can be ordinary
typed fields. **That is what landed on 2026-09-19**; read "torch.save: the
legacy file" below rather than this paragraph, which was written from a first
reading of the bytes.

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

**The nested pickle after an object array's wrapper landed on 2026-09-19.**
`write_array` writes the padding and the numbers only in its `else` branch:
when `array.dtype.hasobject`, it calls `pickle.dump(array, file_handle,
protocol=5)` instead and there is no padding byte either. So after a wrapper
whose dtype is `|O8` comes a whole pickle -- its own PROTO, its own framing,
its own memo numbered from nought, its own STOP -- and then the outer stream
carries on with the byte after.

The form reads exactly one such pickle, by the same grammar, with the outer
stream's protocol, memo, framing and pickler put aside and put back after, so
nothing the inner pickle files reaches the outer memo and a slot number in
either names what its own stream wrote. The value it holds has to be an object
array whose shape and order are the ones the wrapper described; anything else,
a second pickle, or a byte between the nested STOP and the opcode the outer
stream carries on with, is a non-match. The wrapper's protocol range is the
outer stream's and the nested pickle declares its own, so a protocol 2 joblib
file still holds a protocol 5 pickle.

The bytes are instructions like any other. `Cursor::breaks` holds every place
the opcode walk stops and starts again -- a padding-and-numbers run, and a
nested pickle, whose STOP would otherwise end the walk of the whole file --
so the listing still names every byte and the tiling test still holds. The
tree shows the pickle as a `nested pickle` node inside the array, with a
`protocol` row of its own, the call that rebuilt the array, and the values.
`joblib/array-of-objects.joblib`, `joblib/pandas-frame-named-columns.joblib`
and `joblib/sklearn-string-labels.joblib` moved out of `does-not-read` with
it, and the folder is gone: nothing joblib writes is refused now.

The object-array production was widened with it, from text, `None` and a name
for text to every leaf the basic productions read. See "An array of pickled
objects" in `DESIGN-familiar-pickle-forms.md` for the list and for what is
still not in it.

**What is left for joblib**, in the order it is worth doing:

1. **Done on 2026-09-19.** See the two paragraphs above.
2. **Done on 2026-09-19, and not the way this expected.** A bundle holding a
   standard library value, a frame or a sparse matrix wanted no joblib row of
   its own. Extensions compose now: the mixed form is the union of every one's
   tables with the joblib wrapper allowed rather than required, so
   `joblib.dump({"trained_at": datetime.now(), "weights": arr}, path)`,
   `joblib/pandas-frame.joblib` and `joblib/scipy-csr-matrix.joblib` all read as
   `mixed-values-p4-p5-v1`, and the two joblib rows keep the two mixtures they
   were written for. See "Extensions compose: the mixed form" in
   `DESIGN-familiar-pickle-forms.md`. A frame with named columns reads too
   since the nested pickle landed later the same day.
3. `numpy.matrix` and `numpy.memmap`, which reach the same writer and would be
   named beside `ndarray`. No file in the corpus holds one.
4. **Done on 2026-09-19.** See "What each era wrote". The missing-alignment-key
   variant is tested against joblib 0.11, 0.14 and 1.1, and 0.9's `.npy`-beside
   layout is a form of its own.

## torch.save: what landed, on 2026-09-19

The ZIP, and everything in the pickle. Not the legacy file.

The code is `crates/core/src/formats/pickle/familiar/torch.rs` for the
production, one `torch` flag on `Allow` and one `Declared` row in `forms.rs`,
`crates/core/src/formats/torchzip.rs` for the template and its schema builder,
`crates/core/src/eval/pickletorch.rs` for finding a tensor's numbers and
reading them, and a probe and an `archive_by_names` branch in `recognise.rs`.
The unit tests are `familiar/tests/torch.rs` over one fixture, and the real
files are `crates/core/tests/torch_real.rs`, ten tests over `torch/`.

| Form | What it reads | Matched |
| --- | --- | --- |
| `torch-tensors-p2-p3-v1` | tensors, and plain data around them | 9 of the 10 archives in `torch/`, and `pickle/proto2-torch-state-dict.pickle` |
| `torch-tensors-p4-p5-v1` | the same at the protocol the caller may ask for | `state-dict-zip-protocol4.pt` |

There is no name at protocol 1 or 0. torch writes protocol 2 by default and
takes a higher one from the caller; it has never written either of the lower
two, and a tensor's instructions there have not been measured.

**The tensor, exactly.** `GLOBAL torch._utils _rebuild_tensor_v2`, `MARK`,
then the persistent id -- `MARK`, the word `storage`, `GLOBAL torch <X>Storage`,
the key, the device, the element count, a `None` in a legacy file, `TUPLE`,
its memo mark, `BINPERSID` -- then the storage offset, the size tuple, the
stride tuple, the requires-grad flag, an empty `collections.OrderedDict()`,
`TUPLE` and `REDUCE`. Ten storage classes are enumerated, one per dtype torch
has a class for. `_rebuild_parameter` wraps one and what comes out is the same
tensor with a note that it is a parameter.

**Why a persistent id is safe to name.** `BINPERSID` in a pickle machine hands
the tuple to the loader's `persistent_load`, which is a function of the loading
program. This reader has no such function and fetches nothing: the tuple is
read as the five or six fixed parts it is, every part checked against what
torch writes, and what comes out is a note saying where the numbers are. The
opcode is folded away inside the tensor's own run, so a `BINPERSID` anywhere
else, or over any other tuple, is a non-match.

**Recognition is by the names in the archive.** `is_torch_zip` asks the front
of the file for a stored entry named `<folder>/data.pkl`, which is what torch
writes first; `archive_by_names` asks the central directory for `data.pkl` and
`version` under one folder, which is the whole test and is what catches a
checkpoint whose pickle sits behind gigabytes of weights. `is_pickle` is
untouched.

**The template is the archive with the pickle placed in it**, not a stream
opened out of it. `torchzip` is `zip::records(true)` and one `Ty::Schema` node
whose builder finds the `data.pkl` entry and places `T::pickle()` at that
entry's own bytes, the way `adios/dataset.rs` places a BP5 dataset. That is
what makes the numbers reachable: the pickle and every storage are then
offsets in one space, so a tensor's bytes have a place in the file, the hex
view can go there, and no second space is opened for a file that is already
laid out flat. Opening the entry as a stream would also have cost the form:
`template_for` sniffs a stream's bytes, and `picklefpf` needs the whole file
in the window, so any `data.pkl` over 36 KB would have opened as the opcode
listing instead.

**Three things the note above had wrong.**

- **The entries are streamed.** The note says every entry is stored and
  aligned, which is true, but the local headers carry `0` for both sizes and
  set the streaming flag: torch's writer puts the real sizes in a descriptor
  after the data. So a walk of local records cannot get from one entry to the
  next, and `eval/pickletorch.rs` reads the central directory instead, ZIP64
  included, since a checkpoint past four gigabytes is an ordinary one.
- **`_rebuild_parameter` is not written like the tensor.** It takes three
  arguments, so protocol 2 closes them with `TUPLE3` and opens with no `MARK`
  at all. Only the tensor's six need one.
- **A state dict is not a plain `OrderedDict`.** `nn.Module.state_dict()`
  returns one filled with the weights and then given a `_metadata` attribute,
  so the pickle is `SETITEMS` and then `BUILD` over the same object. The
  contents and the attributes are two different things arriving the same way:
  `Kind::Made` has an `attrs` slot beside `state` now, and the node shows them
  as an `attributes` row under the entries. Without that, no checkpoint saved
  the ordinary way read at all, and the hand-made sample hid it.

**What a tensor shows**: `dtype` as the plain word, `shape`, `stride`,
`storage offset`, `storage` (the key), `location`, `requires grad`, `numbers`
= `12 values in data/0`, and `stored at` = `0x380, 48 bytes`, which is this
tensor's own window rather than the whole storage. The storage class stays
reachable as a named operand inside the run of instructions that rebuilt it.
Its table is the numbers in that entry: rows along the first axis, columns
along the last, every value at the offset its storage offset and stride put
it, so a transposed view reads down the storage and two windows onto one
storage read as the two tensors they are. A mapping of nothing but tensors
opens as a summary of `name, dtype, shape, values`, one row a tensor.

## torch.save: the kinds torch 2.14 writes, on 2026-09-19

The section above was written against what the earlier samples held. A matrix
run of torch 2.14 and joblib 1.6 (`tools/make_torch_joblib_matrix.py` in the
collection) wrote forty files, and eleven of them matched nothing. All eleven
read now, and thirteen are in the collection under `torch/`.

**`_rebuild_tensor_v3`.** The legacy storage classes were frozen, so every
element type torch has added since is written another way: the persistent id
names `torch.storage.UntypedStorage`, whose count is **bytes** rather than
elements, and the dtype arrives as the call's seventh argument, a global the
file names and never calls. `Cursor::rebuilt_tensor` reads either call, and
`Cursor::tensor_made` puts the count back in elements: a byte length that is
not whole elements is a non-match. The types that arrive this way are
`float8_e4m3fn`, `float8_e5m2`, `uint16`, `uint32` and `uint64`.

**The dtype names are the `names` column.** `torch.float32` is one object
rather than a class to construct, and `__reduce__` returns its name, so pickle
writes a bare `GLOBAL torch float32`. Twenty of them are enumerated in
`torch::DTYPE_NAMES`, which is the "may name, never call" mechanism the
standard library's work added. `torch` itself is still not a module a class
may be named from.

**Complex and quantised keep their storage classes.** `ComplexFloatStorage`,
`ComplexDoubleStorage`, `QInt8Storage`, `QUInt8Storage` and `QInt32Storage` are
five more rows in `STORAGES`. A complex element is a pair of floats, so the
table reads both and shows one cell in Python's own spelling for a complex
literal: `1+0j`.

**`_rebuild_qtensor`** takes the same first four arguments as a tensor and then
`(torch.per_tensor_affine, scale, zero point)`. The stored integers stay the
stored integers: the table is what the file holds, and the scale and the zero
point are two rows beside it. Nothing is dequantised, because a reader looking
at a quantised checkpoint is looking at those integers and would not be told
that the file had been rewritten on the way out. `per_channel_affine` hands
over two tensors and an axis instead and is a non-match; no sample holds one.

**`_rebuild_sparse_tensor(layout, data)`** is two tensors and the shape they
stand for. The layout is `torch.serialization._get_layout` over the layout's
own name, and only `torch.sparse_coo` is allowed: the other layouts hand over a
different tuple, which has not been measured. The indices and the values are
ordinary tensors and each opens as its own table. Nothing is densified.

**`torch.Size` and `torch.device`** are two more rows in the calls table: a
tuple subclass called with its tuple, and the device type with an optional
index. A file holding nothing but a `Size`, a device and a dtype has no tensor
in it and is still a torch file, so `Family::Torch` now requires the torch
**extension** rather than a tensor, and `Cursor::torch_named` notes the
extension wherever a global under `torch` is named. `collections.OrderedDict`
is deliberately outside that prefix: a state dict is one of those, and a state
dict is not a mixture of torch and the standard library.

**A whole module pickled as an object.** `torch.save(torch.nn.Linear(4, 3))`
saves the module rather than its state, which torch's documentation advises
against and which people write anyway. The pickle names
`torch.nn.modules.linear.Linear`, makes it with `EMPTY_TUPLE NEWOBJ`, and
gives it a state dictionary of `training`, `_parameters`, `_buffers`,
`_non_persistent_buffers_set`, eight `OrderedDict`s of hooks, `_modules`,
`in_features` and `out_features`.

**The decision, and why.** It is read, as a plain object of a class under
`torch.nn` and nothing wider. The reason is that its state is only what the
forms already read: flags, dictionaries, an empty set, empty `OrderedDict`s
and the parameters themselves. Nothing about the class is run, nothing is
constructed from values, and the prefix is `torch.nn` rather than `torch`, so
a class from anywhere else under torch is still a non-match. What that costs
is that `Family::Torch` no longer requires `instances == 0`; what it buys is
that the file people actually have on disk opens as its weights instead of as
an opcode listing. A module whose state holds anything the forms do not read
is a non-match, which is the ordinary rule and not a special case.

**What is still refused**, and what each would need:

- `numpy.matrix` in a joblib file (`joblib-matrix.joblib` in the matrix run).
  A NumPy class named outside the NumPy productions' own fixed runs, which no
  form allows. It needs a decision about whether NumPy's array subclasses are
  named at all.
- `joblib-sklearn-string-labels.joblib` from sklearn 1.9.1, which reads as far
  as `0x1be`. Not looked at this round; it is a scikit-learn shape rather than
  a torch one.
- `_rebuild_meta_tensor_no_storage`, `_rebuild_device_tensor_from_numpy`,
  `_rebuild_wrapper_subclass`, `_rebuild_tensor` (no `_v2`), the four-bit and
  two-bit quantised storages, and the sparse layouts other than COO. No sample
  holds any of them.

## torch.save: the legacy file, on 2026-09-19

The format before torch 1.6, and what `_use_new_zipfile_serialization=False`
still writes. One file: five pickles one after another -- the magic number,
the protocol version `1001`, the system info dictionary, the data pickle and
the list of storage keys -- and then, for each key in that list, an eight-byte
element count and the raw numbers.

The code is `crates/core/src/formats/torchlegacy.rs` for the template and its
schema builder, one probe in `recognise.rs`, `Descriptions::bytes` and
`Descriptions::file_len` in `crates/core/src/eval/schema.rs`, and
`Evaluator::legacy_storages` in `eval/pickletorch.rs`. The real files are
three tests in `crates/core/tests/torch_real.rs`.

**It is a `Ty::Schema` and not a `Deduce`.** The note above proposed a new
`Deduce` answered by a small `Deducer`. A schema node is the better fit and
needed one small addition rather than a new question: a builder could read
fields the template had already placed and could not read bytes, and nothing
places a field here until the pickles have been walked. So `Descriptions`
gained `bytes(at, len)`, which reads through the evaluator the way a deduced
run does -- a chunk that has not arrived says `Pending` rather than reading
as noughts -- and counts towards how far the build reaches, so an edit inside
the pickles builds the node again. `file_len` came with it, for holding the
last storage to the end of the file.

**What the builder does.** Reads the head, walks each pickle to its STOP with
`pickle::opcodes`, and places each as `T::at(.., T::sized(.., T::pickle()))`
over exactly its bytes. Then it reads the data pickle with the same Familiar
Pickle Form the archive's `data.pkl` is read with and takes the storage class
out of each tensor's persistent id, which is the one thing the storages
themselves never say: a storage writes how many elements it holds and not how
wide one is. The key list gives the order. Each storage is then an ordinary
`i64` count and a typed run of numbers at its own offset, so the numbers have
byte addresses, the hex view goes there and the reading is counted once.

Only the five pickles are read out of one head window, and four megabytes is
far more than any of them come to. The counts are read where each one sits,
one eight-byte read apiece, because they are spread through the file with a
storage's numbers between them and a checkpoint is as long as its weights.

**Every byte or nothing.** The builder places the five pickles and the
storages and then checks that they reach the end of the file exactly. A file
that opens with torch's magic number and does not add up is a node saying so
rather than fields at offsets nobody checked. A file whose system info says
`little_endian: False` is refused the same way: the numbers would be the
other way round everywhere, no machine torch runs on has been big-endian
since the format was written, and refusing is the reading nobody has to
check.

**The tensors are the ZIP's tensors.** A legacy persistent id carries one more
element than the archive's, a `None` where a view's metadata would go, and
`familiar/torch.rs` already read that. What was missing was where the numbers
are: `pickletorch.rs` walks a ZIP's central directory for the entry
`data/<key>`, and a legacy file has no directory. So `archive()` falls back to
the same layout walk and hands back the storages under the names the tensors
look for, one `data.pkl` at the data pickle and one `data/<key>` per storage.
Nothing else in the tensor reading knows which kind of file it is reading, and
`state-dict-legacy.pt` opens as the same tables `state-dict-zip.pt` does, cell
for cell.

**Recognition is the first pickle**, which is the same fifteen bytes in every
file torch has written this way: `80 02 8a 0a` and the magic number. The probe
is asked before the pickle probes, because a legacy file opens as a pickle and
is not one: its first STOP is fifteen bytes in and four more pickles follow
it. `is_pickle` is untouched. A caller who passed `pickle_protocol=4` gets
`80 04 95` instead and is not recognised; no sample in the collection is one,
and widening the signature to a protocol nothing has been measured at would be
guessing.

**What is left for the legacy file**: a big-endian one, which is refused on
purpose. Protocol 4 and the torch 0.4 to 1.5 releases landed on 2026-09-19:
see "What each era wrote".

**What is left for torch**, in the order it is worth doing:

1. **Done on 2026-09-19.** See the section above.
2. **Done on 2026-09-19**, in the section on what torch 2.14 writes:
   `_rebuild_tensor_v3`, the complex and quantised storage classes,
   `_rebuild_qtensor`, `_rebuild_sparse_tensor`, `torch.Size`, `torch.device`,
   the dtypes as named globals, and a whole module pickled as an object.
   `_rebuild_meta_tensor_no_storage`, `_rebuild_device_tensor_from_numpy`,
   `_rebuild_wrapper_subclass` and `_rebuild_tensor` (no `_v2`) are still
   refused; no sample holds one.
3. **Done on 2026-09-19.** See "What each era wrote": torch 0.4 to 2.1, ZIPs
   and legacy files alike.
4. **A tensor's numbers as a field rather than a table.** See below.

**What a proper IR answer would need**, now that the shape of it is known. The
gap is a field in one ZIP entry placed by another entry's contents: the
tensor's numbers are at `data/<key>`, and `<key>` is a string the pickle
holds. `Ty::At` places a field at an expression, so the missing piece is an
expression that resolves an archive entry by name -- something like
`E::entry_of(&["records"], E::field("key"))`, evaluated by walking the same
central directory `pickletorch.rs` walks, and a `Ty::Strided` that lays a
typed run out by a size and a stride rather than contiguously. With those two
a tensor would be an ordinary field: hex view, selection, byte addresses per
cell, and the reading counted once. Without the second, a non-contiguous view
would still need computed cells. zarr-in-zip wants the first of them too,
which is the argument for doing it rather than widening `pickletorch.rs`.

### Cells that say where their bytes are: landed on 2026-09-19

Option 1 with the range exposed, which the paragraph above said to take first.
`Evaluator::pickle_cells` no longer hands back a bare value per cell. It hands
back a `FrameCell`: the value, and a `CellAt` saying where the bytes behind it
are or why there are none.

```rust
pub enum CellAt {
    Bytes { space: u32, offset_bits: u64, size_bits: u64 },
    Counted,   // a RangeIndex label, worked out from a start and a step
    Said,      // a fact the pickle states: a tensor's dtype and shape
    Nowhere,   // this reading cannot say
}
```

`space` is the numbering `NodeInfo::space` uses, so a cell of a protocol 2
array points into the space that array's spelled numbers opened and a cell in
the file points at the file. What each of the three tables carries:

- **A pandas frame or series.** A number is its element of the run, in the
  file at protocol 3 and up and in the spelled space below it. A cell of an
  object column is the whole pickled value, opcode included, which for a
  string written earlier is the instruction that names it out of the memo. A
  categorical cell is its code, not the category's name. A `RangeIndex` label
  is `Counted`.
- **A torch tensor.** The entry's data, plus the storage offset and one step
  of the stride per axis, times the element size. A complex number is the
  whole pair. In a legacy file the same arithmetic lands inside the storage's
  own field, so the cells and the tree agree.
- **The summary over a state dict.** The name cell is the instructions that
  spell the key; the count of values is the tensor's own window in the file,
  which is what `stored at` says. The dtype and the shape are `Said`: the
  instructions that rebuild the tensor state them and no run of bytes holds
  them as a value.

**What this did not close.** A tensor is still not an ordinary field. There is
no node for it in the tree, so no hex view over it, no listing row, no
editing, and nothing the annotation column can draw. The reading is still
`pickletorch.rs` walking the central directory rather than the IR, and a
non-contiguous view is still worked out per cell. What the tables gained is
the one thing option 1 was missing: every cell can now say where it is, and
the view can select those bytes. S6 and `Ty::Strided` are still what a field
would need.

### The numbers as an ordinary field: landed on 2026-09-19

A tensor whose elements run through its storage in order now has a `numbers`
child that is a typed run placed in the entry the pickle named: `f32[12]` at
0x380 for `layer.weight` of `state-dict-zip.pt`. Byte addresses, a listing
row, the hex view, selection and editing are all the machinery every other
field uses, and the table over it is the ordinary array table rather than
cells the core works out.

**What a `Place` may say.** The spike's question was whether a node a parse
synthesises can point outside the bytes of the node that holds it. It can.
`Evaluator::place_pickle_child` hands back a `Place` with its own offset and
limit, and nothing between there and `remember` holds it to its parent: the
"extends beyond its parent" checks in `place_child` and `size_within` are
against the limit the place carries, which is the run's own end. What had to
be added is one fact the rest of the reading needs, on `Resolved` and on
`Place`: `elsewhere`, true for a node a parse put somewhere rather than after
the sibling before it. That is what `Ty::At` says for a field a template
placed, and the kind totals ask the two questions together
(`eval/kinds.rs`, the `deferred` queue): without it the walk would move its
cursor to another entry of the archive and count every byte in between as a
gap.

**Nothing is counted twice.** `zip::records(true)` already marks each entry's
`data` as a second reading, which is what `torchzip` asks for, so the bytes
belong to whatever reads them as what they are. Before this they belonged to
nothing and read as a gap.

**Where the parts are.** `Part::Numbers` in `eval/pickleparts.rs` (no bytes of
its own, since the run is nowhere near the instructions that named it),
`Tensor::contiguous` in `familiar/captured.rs`, `Evaluator::tensor_numbers` in
`eval/pickletorch.rs`, the arm in `place_pickle_child` and the table in
`pickle_table`.

**A view that is neither C nor Fortran order** keeps what it had: the `numbers`
and `stored at` rows and a table whose cells are worked out at the strides.
Every tensor now carries an `order` row, `C` or `Fortran` in the words an
array's own `order` row uses and `not contiguous` for the third answer, which
is torch's word and NumPy's. A transposed two-dimensional tensor is Fortran
order and so is a run: `shared-storage-views-zip.pt`'s `grid` is a field, and
its table is the run as the file holds it, four rows of six, each one column
of the tensor. What is left with worked-out cells is a slice, a broadcast and
a transpose of three axes or more, and no sample in the collection holds one,
so the third answer is tested against made-up strides and not against a file.

**Two tensors over one storage** place two runs over overlapping bytes, which
is what `whole`, `tail` and `grid` are in `shared-storage-views-zip.pt`: 96
bytes at 0x380, 48 at 0x3b0, and 96 at 0x380 again. Each is the reading its
own reader asked for and none is a view of the others, so none can be marked
a second reading in advance the way `Field::aside` marks one. What is true of
the file is that the bytes are numbers once, so the kind totals count the
first run over a stretch and pass over a run overlapping it:
`KindWalk::count_placed`, a list of stretches sorted by where they start, one
binary search a run.

**A legacy file counts them in the storage.** There the storages are fields
already -- `torchlegacy.rs` places an element count and a typed run apiece --
so a tensor's own run over the same bytes is genuinely a second reading, and
the node carries `Resolved::aside` to say so. That is `Field::aside` for a
node a parse made rather than a template declared, and `Evaluator::aside`
asks the node before it asks the parent's field. Both halves of the question
are asked, in `Evaluator::storages_are_fields`: the template has to be the one
that places the storages, and the runs in hand have to be the ones its layout
walk found. A legacy file read as the contents of something else is the same
bytes with nothing placing them, and calling its tensors a second reading
would leave the numbers counted nowhere.

**What is still open.**

- **The lookup is still Rust.** `pickletorch.rs` walks the central directory
  for `data/<key>`; the IR still cannot say "the data of the entry named X",
  which is `E::entry_of` in the paragraph above. The `numbers` field reports
  no expression that placed it, so the inspector's depends-on graph does not
  show the archive's directory. Closing it means the expression, its text
  form in `template_text.rs`, its diagram and graph arms, and a `Ty` for the
  run; zarr-in-ZIP wants the same expression.
- **The hex view does not lead back to a tensor.** Clicking a byte of
  `data/0` lands on the ZIP record's `data` field: `placed::Index` is walked
  from the template's types and a `Ty::Pickle` node's synthesised children
  are not in it. Selecting the tensor's `numbers` in the tree does go to the
  bytes, which is the direction a reader asks for first.
- **A legacy tensor has no row pointing at its storage.** Its `numbers` field
  is the right bytes and is counted where the storage is, but nothing in the
  tree says the two are the same run. A reference row the reader can follow
  is what that wants, and `Says` has no arm for it yet.
- **The computed-cells path is still there for every tensor.** Only a tensor
  that is not contiguous is offered it now, since `pickle_table` answers
  nothing for one with a run, but `Evaluator::pickle_cells` still answers for
  any of them, which is what `cells_real.rs` asks of it.

## What each era wrote, on 2026-09-19

Nine container environments: eight with both libraries, from torch 0.4.1 with
joblib 0.11 to torch 2.14 with joblib 1.6, and one with joblib 0.9.4 alone
(`/home/pengo/qubero-container-matrix/`, made by
`tools/make_torch_joblib_matrix.py` and `tools/run_torch_joblib_matrix.sh` in
the collection). Every file in it reads now except the three named at the
end.

**torch, by release.** Each row is what that release wrote that the one above
it did not.

| Release | What is different |
| --- | --- |
| 0.4.1 | `_rebuild_tensor_v2`'s sixth argument is the tensor's own `_backward_hooks`, which is `None` until a hook is registered. A parameter is `torch.nn.parameter.Parameter(tensor, requires_grad)`, the class called with its tensor. `torch.Size` is closed by NEWOBJ. A module saved whole names its class through the persistent id `('module', cls, source_file, source)` and its state holds `_backend`. |
| 1.0.1 | The hooks become an empty `OrderedDict()`: see the note "Don't serialize hooks" in `torch/tensor.py`. A parameter becomes `_rebuild_parameter(tensor, requires_grad, OrderedDict())`. |
| 1.5.1 | `torch.Size` has a `__reduce__` now and is closed by REDUCE. `_backend` is gone from a module's state. Neither bound is tight: `Size` was NEWOBJ in 1.0 and is REDUCE here, and `nn.Module.__init__` still set `self._backend` in 1.1, so the two changes fall between 1.0 and 1.5 and between 1.1 and 1.5. This is the last release that writes the legacy file by default and the first that can write the ZIP with `_use_new_zipfile_serialization=True`. That archive writes `version` before `data.pkl` and puts the ZIP64 end records between the central directory and the ordinary end record. |
| 1.8.1 | The archive's folder is `archive/` rather than the file's own name. |
| 1.13.1 | Storage keys are `0, 1, 2` rather than the addresses the buffers were at. |
| 2.1.2 | `byteorder` and `.data/serialization_id` join the archive. |
| 2.14 | `.format_version` and `.storage_alignment` join it, and the dtypes with no storage class arrive through `_rebuild_tensor_v3`. |

Where each difference lives in the writer: `torch/tensor.py`'s
`__reduce_ex__` and `torch/nn/parameter.py`'s for the first two rows,
`torch/serialization.py`'s `persistent_id` and `_save`/`_legacy_save` for the
rest.

What the reading needed for all of it: the `None` hooks and the
`Parameter(...)` call in `familiar/torch.rs`, a `Via::NewObj` row for the
older `torch.Size`, the module persistent id and one `Reduce` row for
`torch.nn.backends.thnn._get_thnn_function_backend`, `MAGIC_P4` in
`torchlegacy.rs` for a legacy file saved at protocol 4, and one fix in
`recognise.rs`: `zip_directory_names` worked out where the central directory
begins by subtracting its length from the end record, which is wrong for any
archive with something between the two, and torch 1.5 writes exactly that.
The end record's own offset field is used now.

**joblib, by release.**

| Release | What is different |
| --- | --- |
| 0.9.4 | Each array is a `.npy` file beside the pickle, one apiece, and the pickle holds a `joblib.numpy_pickle.NDArrayWrapper` naming it: `filename`, `subclass`, `allow_mmap` and nothing else. The main file is an ordinary pickle by every test there is. |
| 0.11 to 1.1 | The array is written into the stream after the wrapper, with no `numpy_array_alignment_bytes` key and no padding byte, so the numbers begin at the BUILD. The form already read this variant and no file had tested it. |
| 1.2 and after | The alignment key, the padding count and the padding. |

The protocol is the interpreter's, not joblib's: Python 3.6 and 3.7 write 3
and 3.8 and after write 4, so the same joblib release is read under the p2-p3
form or the p4-p5 one depending on what ran it.

The 0.9 layout is a form of its own, `joblib-npy-files-p2-p3-v1` and
`joblib-npy-files-p4-p5-v1`, because the numbers are somewhere else: the value
is a `Shape::ArrayFile` whose one row is the file to open, the way a torch
tensor in a bare `data.pkl` says which archive entry its numbers are in. What
a reader sees over one is `array in bundle.joblib_01.npy`, and the `.npy`
beside it opens under the `npy` template with no new mechanism at all.

**What still does not read**, and why:

- `numpy.matrix` in a joblib file, from every release 0.11 to 1.6. The
  wrapper's `subclass` reads `numpy.ndarray` and no other class, because
  whether NumPy's array subclasses are named at all is a decision nobody has
  made. `joblib/does-not-read/v1.6-numpy-matrix.joblib`.
- A plain pickle of a NumPy object array, which is what joblib 0.9 wrote for
  one: it has no wrapper, so no joblib form reads it, and the NumPy forms have
  `object_arrays` off. Widening them would move the verdict of every
  `mixed-array-of-tuples` file in the collection, so it wants its own pass.
- joblib 0.9's compressed file, which is not zlib but joblib's own `ZF`
  container: `ZF0x226` and a length, then the stream. A thirteen-byte header
  over a codec that already exists, the way `formats/lzma.rs` is, and no
  sample of it is in the collection yet.

## Samples, and when they go into the collection

The `joblib/` samples are in the collection since 2026-09-19, twenty-one of
them and none refused. The generator gained a 0-d array, an empty array, a
list of three arrays, one small array that is also a fixture in this
repository, a frame with named columns and a text column, and a
`LogisticRegression` fitted on labels that are strings, whose `classes_` is
an object array and so a nested pickle.

The `torch/` samples are in the collection since 2026-09-19, twenty-six of
them, three of which are legacy files: the original thirteen, and thirteen
more from the torch 2.14 matrix run, which are the kinds the form refused
until the same day. A legacy file is **not byte-reproducible**: a
storage's key is the address its buffer happened to be at, so every run of the
generator writes different keys. Check `git status` after running it and
commit the bytes you tested against.
The generator gained a module's parameters, a 0-d and an empty tensor, a real
`model.state_dict()` beside an `optimizer.state_dict()`, and a storage longer
than any sniff window. `pickle/proto2-torch-state-dict.pickle` is still a
`data.pkl` on its own and was rewritten the same day: it spelled the storage
class as a plain string where torch's `persistent_id` hands the pickler the
class itself, so it claimed to be what torch writes and was not.

Older writers mattered as much here as they did for pickle, and the container
matrix covers them since 2026-09-19: thirteen `torch/v*.pt` from releases 0.4
to 2.1, six `joblib/v*.joblib` from 0.11 and 1.1, and `joblib/v0.9-npy-files/`,
which keeps the generator's own file names because each pickle in it names its
`.npy` by name.

## Order of work

1. ~~joblib uncompressed: the wrapper production and the run after it.~~ Done.
2. ~~joblib compressed: the decoded space's template, and raw LZMA
   recognition.~~ Done, and the first of those needed no work.
3. ~~The torch family in the pickle (persistent id, `_rebuild_tensor_v2`), so
   `data.pkl` on its own reads with tensors that say where their numbers
   are.~~ Done.
4. ~~`torchzip`: recognition by names, the archive beside its contents,
   tensors reading their numbers from the entry they name, the summary
   table.~~ Done.
5. ~~The legacy torch file.~~ Done on 2026-09-19.
6. ~~Older versions from containers.~~ Done on 2026-09-19. The samples are in
   the collection.
