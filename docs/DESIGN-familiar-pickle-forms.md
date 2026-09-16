# Familiar Pickle Forms (FPF)

Status: third implementation slice. The recogniser is in
`crates/core/src/formats/pickle/familiar.rs`; a match is now a template of its
own, `picklefpf`, which places the captured tree as fields.

## Implemented slice and continuation notes

FPF runs first, alongside the existing symbolic decoder. A whole-document match
bypasses that decoder, says so on the STOP row of the listing, and is read as
the object it captured by a template of its own. A non-match
retains existing hex, opcode and symbolic payload inspection. Those legacy
annotations are not FPF claims. This preserves the existing viewer while the
strict forms grow.

Implemented forms:

- `basic-p4-p5-v2`: null, bool, BININT1/BININT2/BININT, BINFLOAT, UTF-8
  strings and byte strings with 1/4/8-byte lengths (with required MEMOIZE), empty tuples, and
  explicit empty/single/batched list and string-key dictionary productions.
  Only one batch of 2–1000 entries is supported. The empty/single-container
  ambiguity uses an explicit local alternative with a shared work budget that
  is never reset by rewinding. Nonempty tuples, long integers, shared references,
  cycles and sets remain unsupported.
- `numpy-numeric-array-p4-p5-v2`: a standalone array or one string-key dictionary
  entry holding an array. Matches exact `_reconstruct` sequences for
  `numpy._core.multiarray` and `numpy.core.multiarray`, with fixed memo positions
  (numpy occupies slot 3 standalone, slot 5 in the dictionary). Supports plain
  b1/i1/i2/i4/i8/u1/u2/u4/u8/f2/f4/f8/c8/c16 dtypes, little/big endian for multibyte
  values, `|` for single-byte values, and explicit C/Fortran order. Dimensions use
  BININT1/BININT2/nonnegative BININT; shapes have 0–32 dimensions with the exact
  EMPTY_TUPLE/TUPLE1/TUPLE2/TUPLE3/marked TUPLE production for their arity. Storage
  uses SHORT_BINBYTES/BINBYTES/BINBYTES8 and must match shape times item size.
  Object, structured, datetime and external-buffer dtypes/layouts fall back.

Both forms accept protocol 4/5 with either no frame or exactly one frame spanning
the complete body. They require STOP followed immediately by EOF. The matcher
uses borrowed byte ranges, a 64-level recursion bound and a shared budget of
100,000 values. No Python or new runtime dependency was introduced.

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
  message, the form ID and the protocol byte, and then `data`. A dictionary
  holds entries, an entry holds its key and its value, an array says its dtype,
  shape and storage order before its numbers. A file no form matches fails to
  resolve, so the chooser falls back to `pickle`.

Ranking: `PROBES` asks `picklefpf` immediately before `pickle`, and only when
the sniff window holds the whole file, since a form matches all of a file or
none of it. So a matched file over `SNIFF_WINDOW` (36 KiB) still opens as the
listing, and the reader picks the other template. `picklefpf` is deliberately
not in `WEAK_TEMPLATES`: parsing to the end is thin evidence and yields to
file(1), but a reviewed grammar that accounted for every opcode and operand in
the file is stronger than any rule keyed on its first bytes.

Of the sibling corpus, three files match today:
`proto4-numpy-array.pickle` under `numpy-numeric-array-p4-p5-v2`, and
`awa2-pose-antelope.pickle` and `awa2-pose-elephant.pickle` under
`basic-p4-p5-v2`. The rest do not, and
`crates/core/tests/pickle_real.rs` writes the whole matrix out file by file so
that a form growing quietly is a failing test.

### What is not exposed yet

An array's numbers read in storage order and are not folded into rows: a 4-by-6
matrix is 24 values, and the shape is a row beside them rather than the shape of
the listing. Nothing here navigates from a value to the opcodes that built it,
and nothing writes a value back through the recogniser. The frame header inside
the `header` node is unnamed: what its length means belongs to the listing.

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
2. Widen the corpus a form can speak for. Six protocol 4 samples that a reader
   would expect to match do not: `proto4-builtins`, `proto4-collections`,
   `proto4-numpy-shapes`, `proto4-numpy-dtypes`, `proto4-numpy-byte-order` and
   `proto4-numpy-shared-dtype`. Each needs a reviewed production, not a
   loosened one.
3. Add provenance-backed NumPy fixtures for the expanded branches, then add
   protocol-5 `_frombuffer` and multiple-frame forms. Give each new production
   exact constants and negative mutation tests. Keep frame bytes visible to the
   grammar and validate boundaries; do not merely strip FRAME instructions.
4. Add nonempty tuples and big integers deliberately, and extend container
   batching beyond one batch. Keep work bounded across every alternative.
5. Add typed memo bindings for specific repeated dtype/name forms; then build
   pandas and estimator forms from reviewed complete structures.
6. Move recognition onto a chunk-aware source cursor for large tensors. Current
   evaluator size caps still apply. Do not relax completeness to obtain previews.

The remaining sections describe the longer-term architecture and acceptance
criteria; they are not claims that all listed coverage has shipped.

Validation: the pickle unit tests include fourteen FPF tests, four of which read
a fixture through the `picklefpf` template and check names, values and byte
ranges; ten `pickle_real` integration tests pass against the sibling corpus,
including the corpus match matrix and a walk of the decoded array's 24 numbers.
The browser test is `web/test/pickle.browser.mjs`: it checks that a matched
sample opens as the familiar form with its form ID and decoded values, that the
chooser offers both templates and switches between them, that the STOP row of
the listing names the form and the other template, and that an unfamiliar
program still reads as a PVM listing with no FPF claim.

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
