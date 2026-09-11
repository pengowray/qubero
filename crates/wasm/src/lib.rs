//! wasm-bindgen surface over `qubero-core`.
//!
//! Offsets cross the boundary as `f64` (exact up to 2^53, far past any file size)
//! to avoid BigInt friction on the JS side.

use qubero_core::codec::{inflate, Codec, Step as MapStep, StepKind};
use qubero_core::eval::{Explain, Graph, KindWalk, Moment, Origin, SpaceId, NO_PARENT};
use qubero_core::template::Zone;
use qubero_core::hexdump;
use qubero_core::textview;
use qubero_core::source::Source;
use qubero_core::{diescript, dosbasic};
use qubero_core::search::{self, Needle, Search, Step};
use qubero_core::{
    formats, magicrule, overview, ChunkStore, Document, EvalError, Evaluator, ExtentEstimate, NodeInfo,
    RunKind, Span, SpanPart, Value,
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Everything one reading holds: a document, what reads it, and the working
/// each format keeps beside it. There is one per address space. Space 0 is the
/// file; every other is a `Decoded` stream that was opened as a document of its
/// own, and it has its own template, its own evaluator and its own byte-class
/// scan, so two tabs never read over each other's working.
struct Sheet {
    doc: Document<ChunkStore>,
    /// Which `Decoded` node of the file this space was unpacked from. Empty for
    /// space 0, which was unpacked from nothing.
    origin: Vec<usize>,
    /// The core's number for the same space, which is what the trace behind the
    /// cursor link is asked by. Zero for the file. The core renumbers from 1
    /// whenever a reading is thrown away, so this is set again every time the
    /// stream is opened again; a tab's own number never changes.
    core_space: SpaceId,
    eval: Option<Evaluator>,
    /// What the wasm sections say about the module, when that is the template.
    /// Built on the first listing that needs it, and thrown away whenever the
    /// document changes, since it holds paths that the change may have moved.
    disasm: Option<formats::WasmModule>,
    /// The same for an eBPF object: what its sections, symbols and
    /// relocations say, so an instruction can name the map it loads and the
    /// helper it calls.
    bpf: Option<formats::ElfProgram>,
    /// Whether `bpf` includes symbols and relocations rather than only named
    /// sections for the logical overview.
    bpf_complete: bool,
    /// The same for a 16-bit Windows program, whose relocations say what its
    /// calls into other modules are calls to.
    ne: Option<formats::NeProgram>,
    /// The template in use, which decides whether a listing row goes through
    /// the disassembler.
    template: String,
    /// The byte-class scan behind the overview, run a step at a time. Thrown
    /// away on any edit: the classes describe bytes that may no longer be
    /// there.
    scan: Option<overview::Scan>,
    /// The same over the one block a reader has picked out, at whatever
    /// resolution that block's own size allows.
    focus: Option<overview::Scan>,
    /// The walk that totals the file's bits by field kind and type, run a step
    /// at a time. Thrown away on any edit for the same reason the scan is:
    /// every offset in it describes bytes that may have moved.
    kinds: Option<KindWalk>,
}

impl Sheet {
    /// A reading of one of the core's spaces.
    ///
    /// The bytes are all here already, so the store keeps every chunk: a space
    /// has no file behind it to fetch a missing one back from. The template is
    /// the one the core settled on, which is the stream's own or, where that
    /// said only bytes, whatever the unpacked bytes were recognised as.
    fn from_space(space: &mut qubero_core::eval::Space, origin: Vec<usize>) -> Sheet {
        let template = space.reading().0.template().clone();
        let bytes = space.bytes();
        let n = bytes.len() as u64;
        let chunks = (n / SPACE_CHUNK + 1) as usize;
        let mut store = ChunkStore::new(n, SPACE_CHUNK, chunks);
        for c in 0..chunks as u64 {
            let from = (c * SPACE_CHUNK) as usize;
            if from >= bytes.len() {
                break;
            }
            let to = bytes.len().min(from + SPACE_CHUNK as usize);
            store.insert(c, bytes[from..to].to_vec().into_boxed_slice());
        }
        let mut sheet = Sheet::new(store, origin);
        sheet.core_space = space.id;
        sheet.template = template.name.clone();
        let mut ev = Evaluator::new(template);
        ev.set_slice(Some(WORK_SLICE));
        sheet.eval = Some(ev);
        sheet
    }

    /// A space that is no longer there: the stream it came from was edited away
    /// or would not open a second time. It holds no bytes rather than the
    /// file's, because a tab named after a stream must never quietly show
    /// something else.
    fn empty(origin: Vec<usize>) -> Sheet {
        Sheet::new(ChunkStore::new(0, SPACE_CHUNK, 1), origin)
    }

    fn new(store: ChunkStore, origin: Vec<usize>) -> Sheet {
        Sheet {
            doc: Document::new(store),
            origin,
            core_space: 0,
            eval: None,
            disasm: None,
            bpf: None,
            bpf_complete: false,
            ne: None,
            template: String::new(),
            scan: None,
            focus: None,
            kinds: None,
        }
    }
}

/// The spaces one file has open, and which of them the caller is reading.
///
/// Every method that reads bytes or fields names a space, because a space is a
/// document: the tab strip in the interface is one `Doc` per space over this
/// one editor. The file's own bytes, its edits and its save plan stay with
/// space 0, since an unpacked stream is read-only this round.
#[wasm_bindgen]
pub struct Editor {
    /// Space 0 first, then one per opened stream, in the order they opened.
    sheets: Vec<Sheet>,
    /// The space the call in hand names. Set by every method that takes one,
    /// so a helper called part-way through a reading still finds it.
    live: usize,
}

/// A field's bytes, and whether it runs on past them.
#[derive(Serialize)]
struct BytesDto {
    bytes: Vec<u8>,
    truncated: bool,
}

#[derive(Serialize)]
struct NodeDto {
    path: Vec<usize>,
    name: String,
    #[serde(rename = "type")]
    type_name: String,
    offset_bits: f64,
    size_bits: f64,
    value: String,
    /// What the editor should start with when this value is edited.
    edit_text: String,
    /// "uint" | "int" | "float" | "bytes" | "str" | "magic" | "enum" | "flags" | "composite"
    kind: &'static str,
    ok: bool,
    child_count: f64,
    /// What one child is called, for counting them: empty when they are items.
    #[serde(skip_serializing_if = "String::is_empty")]
    unit: String,
    composite: bool,
    /// True when `write_node` accepts text for this field.
    editable: bool,
    /// Bytes of the field the value occupies; less than the size for padded
    /// and terminated text.
    value_bytes: f64,
    /// Where the value starts: past a byte-order mark, if the field has one.
    value_offset_bits: f64,
    /// How the encoding was settled, or that the bytes do not fit it.
    read_as: Option<String>,
    /// Which sibling's length, count, type or position this field settles, as
    /// an index among the parent's children. Null for a field no sibling
    /// reads, which is most of them.
    consumed_by: Option<f64>,
    /// What the template says about this field over the top of that: true for
    /// machinery, false for payload, null when it has no opinion.
    machinery: Option<bool>,
    /// True when this field is only its parent's contents, and so has no name
    /// of its own worth a level of structure.
    contents: bool,
    /// True when the template says this structure is one row rather than one
    /// row per field: a value with parts, not a part of the file. A view that
    /// gives it a heading of its own has spent a heading on a colour.
    inline: bool,
    /// True when the node's own bytes include punctuation its children do not
    /// account for: the braces of a JSON object, the brackets of an array.
    /// What the children leave over is the node's own syntax, not bytes
    /// nothing describes.
    framed: bool,
    /// Which address space `offset_bits` counts in: 0 for the file, and a
    /// number of its own for each decoded stream. A field in a space other
    /// than the file has no place in the hex view, and its offset is drawn as
    /// an offset within its stream.
    space: f64,
    /// For a compressed run that would not open: "too-large", "failed" or
    /// "unaligned". Null for every other field.
    refused: Option<String>,
    /// True for a compressed run. One that opened can be opened as a document
    /// of its own, which is what the Open unpacked button does.
    decoded: bool,
    /// True for the one node a stream holds. Its parent is the stream, so this
    /// is where the listing offers Open unpacked.
    space_root: bool,
}

/// One element of a folded run, as the value table draws it.
#[derive(Serialize)]
struct CellDto {
    /// Where the element sits in its run: the last step of its path.
    index: f64,
    offset_bits: f64,
    size_bits: f64,
    /// What the listing would say about it on a shared row, or the symbol's
    /// name for a block of a trace. Nothing here is reformatted by the view.
    text: String,
    /// What the cell shows, where that is shorter than what its tooltip says:
    /// a deflate literal is the byte, a match its two numbers. The same as
    /// `text` for everything else.
    label: String,
    /// "uint" | "int" | "float" | "bytes" | "str" | "enum" | "flags" |
    /// "composite" | "symbol" | "scale"
    kind: &'static str,
    /// False when the element's bits are not one run, which sends the view to
    /// its uniform layout: a `q5_0` weight is four bits of `qs` and a fifth
    /// bytes away in `qh`.
    contiguous: bool,
    /// True when this record reads exactly as the record before it, so the
    /// cell is drawn without its text. `text` is still what it says, for the
    /// tooltip and for the width the table is laid out to.
    repeat: bool,
}

/// What the byte-class scan has found so far. `classes` is one digit per
/// bucket in file order, the digit being `overview::Class`.
#[derive(Serialize)]
struct OverviewDto {
    done: bool,
    bucket_bytes: f64,
    total_buckets: f64,
    classes: String,
    zero_bytes: f64,
    text_bytes: f64,
    read_bytes: f64,
    /// How many of each byte value have been read, value 0 first. The whole
    /// 256 rather than the few commonest: a view drawing the spread of a file
    /// wants the shape, and the shape is the tail. Two kilobytes or so of JSON
    /// a step, since a count crosses as a float and writes its `.0`, against
    /// the 256 KiB the step read to fill it in.
    histogram: Vec<f64>,
}

/// The same for one block, with what the whole block's bytes turned out to be
/// rather than what each of its buckets did.
#[derive(Serialize)]
struct FocusDto {
    done: bool,
    /// The block, in bytes.
    start: f64,
    end: f64,
    bucket_bytes: f64,
    total_buckets: f64,
    classes: String,
    zero_bytes: f64,
    text_bytes: f64,
    read_bytes: f64,
    /// The same full spread as the whole-file scan carries, so a view reading
    /// one can read the other. `common` below is this sorted and cut short.
    histogram: Vec<f64>,
    /// Entropy over the block's bytes, and the most a block this long could
    /// reach. The pair is the honest reading: 7.9 out of 8 means dense, 7.9
    /// out of 7.9 means only that there are not many bytes here.
    entropy: f64,
    entropy_max: f64,
    /// How many byte values appear at all.
    distinct: f64,
    /// The values that appear most, commonest first.
    common: Vec<CommonByteDto>,
}

/// One byte value and how much of a block it accounts for.
#[derive(Serialize)]
struct CommonByteDto {
    value: f64,
    count: f64,
}

/// Where the file's bits have gone, as far as the walk has got. The same
/// stepped shape the byte-class scan answers in: partial totals every time,
/// and `done` saying when to stop asking.
#[derive(Serialize)]
struct KindTotalsDto {
    done: bool,
    /// How far into the file the walk has reached. What is past this is in
    /// none of the numbers below, so a view derives what is left to do as the
    /// file's length less this.
    reached_bits: f64,
    /// Bits some field covers, which is the sum of `totals`.
    covered_bits: f64,
    /// Bits inside the reached region that no field covers.
    unmapped_bits: f64,
    totals: Vec<KindTotalDto>,
}

/// One kind-and-type pair, and what the file spends on it.
#[derive(Serialize)]
struct KindTotalDto {
    /// The same word a field's row carries: "uint" | "int" | "float" |
    /// "bytes" | "str" | "magic" | "enum" | "flags" | "composite". Worked out
    /// from the type rather than from a value, so the two a value alone can
    /// carry, "unread" and "unset", never appear here.
    kind: &'static str,
    #[serde(rename = "type")]
    type_name: String,
    bits: f64,
    count: f64,
}

/// How many of a block's commonest byte values are worth naming. Enough to
/// show a block is mostly two or three values; past that the count is the
/// answer, not the list.
const COMMON_BYTES: usize = 5;

/// A scan's byte counts as the host reads them. Counts cross the boundary as
/// `f64` like every other number here, and 256 of them written out with the
/// `.0` a float carries is a couple of kilobytes of JSON: nothing beside the
/// quarter of a megabyte the step read to fill it in, or beside the bucket
/// string that already goes back every step.
fn histogram(scan: &overview::Scan) -> Vec<f64> {
    scan.histogram().iter().map(|&n| n as f64).collect()
}

#[derive(Serialize)]
struct TextDto {
    text: String,
    /// True when the field holds more than the editor will show.
    truncated: bool,
}

/// One row of a cross-reference stream, already decoded: the object it is
/// for, what it says about that object, and where that puts it. `offset` is
/// -1 for a row that names no place in the file, which is every row but an
/// in-use one.
#[derive(Serialize)]
struct XrefRowDto {
    object: f64,
    kind: &'static str,
    /// The type number the row held: 0, 1, 2, or whatever a row of a type
    /// nobody has defined wrote. Without it a panel showing an unknown row has
    /// only `kind`, which for all of them is the same word.
    type_raw: f64,
    offset: f64,
    second: f64,
    third: f64,
}

/// One object inside an object stream.
#[derive(Serialize)]
struct ObjStmObjectDto {
    number: f64,
    /// How long the object is in the decompressed bytes.
    len: f64,
    /// The object as written, cut at the limit the core keeps. `cut` says the
    /// rest was left behind.
    text: String,
    cut: bool,
}

/// One column of a row that was joined back together from the pages it spilled
/// onto.
#[derive(Serialize)]
struct SqliteColumnDto {
    /// What SQLite calls the type: `i32`, `text, 8189 bytes`, `null`.
    #[serde(rename = "type")]
    type_name: String,
    /// The value, shown the way every other value in the tree is shown.
    value: String,
    /// Which of the shapes a value takes, so it can be styled like its kind.
    value_kind: &'static str,
    /// Where it sits in the joined row, which is not where it sits in the file.
    at: f64,
    len: f64,
}

/// One filter undone on the way back to a chunk's elements.
#[derive(Serialize)]
struct ChunkStepDto {
    filter: String,
    in_bytes: f64,
    out_bytes: f64,
    /// Set when this chunk's own mask said the filter was not applied to it.
    skipped: bool,
}

/// What a type permits. `kind` picks which of the rest is filled in.
#[derive(Serialize)]
struct ExplainDto {
    /// "magic" | "enum" | "flags" | "float" | "quant" | "xref" | "objstm" | "sqliterow" | "chunk" | "plain"
    kind: &'static str,
    /// The type's own name, for an enum or a flags field.
    name: String,
    /// Magic: the bytes the format requires, and the bytes that are there.
    expected: Vec<u8>,
    actual: Vec<u8>,
    /// Enum: every value it names, and the one in the file.
    cases: Vec<CaseDto>,
    current: f64,
    /// Enum: what the value in the file is called, where that name comes from a
    /// counted run rather than from `cases`. Empty when it has no name.
    named: String,
    /// Enum: whether its numbers are read in hex.
    hex: bool,
    /// Flags: one entry per bit of the field, from bit 0 up.
    bits: Vec<BitDto>,
    /// Float: which layout it is, how many bits wide, and those bits in value
    /// order, written in hex because a 64-bit pattern does not survive a JSON
    /// number.
    format: String,
    width: f64,
    pattern: String,
    /// Quant: the block's shared scale, and what it pairs with the scale, named
    /// as the file names it. Empty name where the layout has no second number.
    scale: f64,
    second_name: String,
    second: f64,
    /// Quant: whether that second number is taken away rather than added, and
    /// whether it is multiplied by the group's own minimum first. Together with
    /// the group scales these say how a stored weight becomes a real one.
    second_subtract: bool,
    second_per_group: bool,
    /// Quant: where the block starts, so a weight's bits can be found from the
    /// offset it carries.
    block_bits: f64,
    /// Xref: the three widths from `/W`, and the PNG predictor where there was
    /// one, which is -1 where there was not.
    xref_widths: Vec<f64>,
    xref_predictor: f64,
    /// Xref: how many bytes the rows are in the file, and how many they came
    /// to once decompressed.
    xref_packed: f64,
    xref_decoded: f64,
    /// Xref: how many rows of each kind there are, over the whole table rather
    /// than over the ones listed.
    xref_free: f64,
    xref_in_file: f64,
    xref_in_stream: f64,
    xref_unknown: f64,
    /// Xref: the rows, and how many there are altogether. A table with more
    /// than `xref_rows` holds says so with `xref_total`.
    xref_rows: Vec<XrefRowDto>,
    xref_total: f64,
    /// Xref: why there are no rows, where there are none. Empty otherwise.
    /// An object stream that would not open says why here too.
    problem: String,
    /// ObjStm: how many bytes the objects are in the file, and how many they
    /// came to once decompressed.
    objstm_packed: f64,
    objstm_decoded: f64,
    /// ObjStm: the object number in `/Extends`, which is the object stream
    /// this one continues, or -1 where it continues none.
    objstm_extends: f64,
    /// ObjStm: the objects, and how many there are altogether. A stream with
    /// more than `objstm_objects` holds says so with `objstm_total`.
    objstm_objects: Vec<ObjStmObjectDto>,
    objstm_total: f64,
    /// Row: how many bytes the row claims, how many the chain reached, and how
    /// many of them stayed on the row's own page. A row that is whole has the
    /// first two equal.
    row_declared: f64,
    row_found: f64,
    row_on_page: f64,
    /// Row: the overflow pages in the order the chain names them, and how many
    /// there are when that is more than the few listed.
    row_pages: Vec<f64>,
    row_chain: f64,
    /// Row: the columns, and how many there are altogether.
    row_columns: Vec<SqliteColumnDto>,
    row_total_columns: f64,
    /// Chunk: how many bytes the chunk is in the file, and how many its
    /// elements came to once the filters were undone.
    chunk_packed: f64,
    chunk_decoded: f64,
    /// Chunk: each filter, in the order it was undone.
    chunk_steps: Vec<ChunkStepDto>,
    /// Chunk: what one element is called, the first few elements, and how many
    /// there are altogether.
    chunk_element_type: String,
    chunk_values: Vec<String>,
    chunk_total: f64,
    /// Quant: the scale the block keeps for each run of weights, where it keeps
    /// them, and how many weights one run covers. Empty for a block with one
    /// scale for all of them.
    groups: Vec<GroupDto>,
    group_weights: f64,
    /// Quant: taken off the packed value to get the stored one, and whether
    /// that value is read signed instead of biased.
    bias: f64,
    signed: bool,
    /// Quant: every weight the block stands for, in the order the tensor reads
    /// them, and which one the cursor is inside (-1 for none).
    weights: Vec<WeightDto>,
    at: f64,
}

/// One run of weights inside a block that share a scale of their own.
#[derive(Serialize)]
struct GroupDto {
    /// The scale as stored, after whatever bias the type takes off it.
    scale: f64,
    /// The minimum taken off every weight in the run, or null where the type
    /// has none.
    min: Option<f64>,
}

/// One weight of a packed block.
#[derive(Serialize)]
struct WeightDto {
    /// The stored integer, after whatever bias the layout takes off it.
    q: f64,
    /// That integer through the block's scale: the number the model reads.
    value: f64,
    /// The run holding its low bits, and the rest of the packed value where the
    /// layout keeps that somewhere else in the block.
    bits: PartDto,
    high: Option<PartDto>,
}

/// One run of bits that makes up part of a packed weight.
#[derive(Serialize)]
struct PartDto {
    /// The block field these bits are in, as the file names it.
    field: String,
    /// Where they are, counted in bits from the start of the block.
    bit: f64,
    width: f64,
    /// Where they sit in the packed value: 0 for the low part.
    shift: f64,
}

/// One field another field's shape came from.
#[derive(Serialize)]
struct OriginDto {
    /// "length" | "count" | "type" | "position" | "points"
    role: &'static str,
    /// The field as the reader would name it: `len`, or `tensors[3].offset`.
    label: String,
    /// Where it is, so the reader can go there. Empty for a bit this field
    /// points at rather than a field it came from.
    path: Vec<f64>,
    /// What it says, in brief. Empty when it could not be read.
    value: String,
    /// For "points": the bit this field's value points at.
    target_bits: Option<f64>,
}

/// How a field was placed and how it was sized, in one word each. What the
/// panel says before any other field is named: most fields are placed and
/// sized by the template alone and have no origins at all, and a section that
/// answers only when another field is involved answers for almost nothing.
#[derive(Serialize)]
struct ShapeDto {
    /// "root" | "first" | "follows" | "element" | "pointer" | "chain" |
    /// "address" | "trace" | "stream" | "unknown"
    placed: &'static str,
    /// "fixed" | "expression" | "terminated" | "remaining" | "children" |
    /// "scattered" | "count" | "encoded" | "trace" | "nothing" | "unknown"
    sized: &'static str,
}

/// What a field checks, and how much of the file that is. No bytes were read
/// to answer it, which is what lets a panel ask on every move of the cursor.
#[derive(Serialize)]
struct CheckDto {
    /// "crc32" | "crc16" | "sum8" | "sum" | "sha1" | "adler32"
    algorithm: &'static str,
    /// The bytes summed, as [offset, length], when they are a run of the file.
    over: Option<[f64; 2]>,
    /// The compressed run whose contents are summed, as [offset, length], when
    /// the summed bytes are nowhere in the file.
    unpacked_from: Option<[f64; 2]>,
    /// How many bytes the sum is over. Real when `covered_exact` is true;
    /// otherwise a stand-in reached for because the true count would take
    /// decoding, which this call must not do. See
    /// [`covered_exact`](CheckDto::covered_exact).
    covered_bytes: f64,
    /// Whether `covered_bytes` really is the number of bytes summed. False
    /// only for an unpacked run whose length nothing here has said yet: no
    /// declared length ([`Covers::Unpacked`](qubero_core::template::Covers::Unpacked)
    /// with none written down) or one member of a run
    /// ([`Covers::UnpackedMember`](qubero_core::template::Covers::UnpackedMember))
    /// whose stream nothing has opened. A view must not print `covered_bytes`
    /// as the covered count while this is false.
    covered_exact: bool,
    /// Whether the sum is over one member's share of an unpacked run — an xz
    /// block's own check — rather than the whole of what the run unpacks to.
    unpacked_member: bool,
    /// The check field's own bytes, as [offset, length, byte], when the sum is
    /// over a record the field sits inside and they are read as something else
    /// while it runs. A tar header is summed with its checksum read as spaces.
    blanked: Option<[f64; 3]>,
}

/// The moment a field means, once the template's epoch has been applied to the
/// number in it. The number itself is untouched and stays on the value row.
#[derive(Serialize)]
struct TimeDto {
    /// "at" | "unset" | "impossible". `unset` is the value a format writes when
    /// it has no time to record, and `impossible` is a number that names no
    /// moment: a year outside 1 to 9999, or a packed date that is not a date.
    state: &'static str,
    /// Seconds from 1970-01-01T00:00:00Z, negative before it. Only for "at".
    ///
    /// Well inside what an f64 holds exactly: the core refuses anything outside
    /// year 1 to year 9999, which is at most 2.5e11.
    unix_seconds: Option<f64>,
    /// The sub-second part, in nanoseconds, always 0 to 999,999,999 and never
    /// negative. Only for "at".
    nanos: Option<f64>,
    /// "utc" | "local" | "unknown". For the last two the seconds above are the
    /// digits the file wrote laid on the UTC line, so an interface prints them
    /// unchanged and says which this was. Shifting them into the reader's own
    /// zone would be inventing one the file never recorded.
    zone: &'static str,
    /// The smallest step the field can express, in nanoseconds, so an interface
    /// prints the decimal places the file actually has and no more: 1e9 for a
    /// field counting seconds, 1e3 for a journal's microseconds, 100 for a
    /// FILETIME, 2e9 for an MS-DOS time, which counts seconds in twos.
    step_nanos: f64,
}

/// What came of taking a checksum: the two forms, printed to the algorithm's
/// own width so they can be compared and shown side by side.
#[derive(Serialize)]
struct VerdictDto {
    computed: String,
    stored: String,
    ok: bool,
}

/// One relationship behind a field's shape, written both ways.
#[derive(Serialize)]
struct RelationDto {
    /// "length" | "count" | "type" | "value" | "position"
    role: &'static str,
    /// The expression as the template writes it.
    written: String,
    /// The same with every field's value in its place.
    substituted: String,
    /// What it comes to.
    result: String,
}

/// One field of a subtree, as much of it as an arrow needs. No value: what a
/// field says is what makes reading it expensive, and nothing a graph draws
/// shows it.
#[derive(Serialize)]
struct GraphNodeDto {
    path: Vec<f64>,
    name: String,
    /// The resolved type said coarsely, for a view that groups or colours by
    /// it: "u16", "u32", "f32", "bytes", "str", "struct", "array", "repeat"
    /// and the rest. See `qubero_core::eval::kind_of`.
    kind: String,
    offset_bits: f64,
    size_bits: f64,
    /// Index into the node list, or -1 for the node the graph was asked for.
    parent: f64,
    child_count: f64,
    /// True when this node has children the walk stopped short of.
    truncated: bool,
}

/// One field deciding something about another, as an arrow between two nodes.
#[derive(Serialize)]
struct GraphEdgeDto {
    /// Index into the node list: the field that decided.
    from: f64,
    /// Index into the node list: the field it decided about.
    to: f64,
    /// "length" | "count" | "type" | "position" | "value" | "name" | "width" | "points"
    role: &'static str,
}

/// The subtree under one node, and what its fields decide about each other.
#[derive(Serialize)]
struct GraphDto {
    nodes: Vec<GraphNodeDto>,
    edges: Vec<GraphEdgeDto>,
    /// How many nodes under the one asked about were left out, as far as is
    /// known.
    omitted: f64,
}

/// One Huffman-coded number of a deflate symbol: the symbol, how wide the code
/// that carried it was, the bits that followed it outright, and what the two
/// came to. See `qubero_core::codec::inflate::DecodedCode`.
#[derive(Serialize)]
struct DecodedCodeDto {
    symbol: f64,
    /// How wide the Huffman code was. Worked out again from the block's table
    /// rather than read from the trace, which does not keep it.
    code_bits: f64,
    /// How many bits followed the code, and what they said. Both zero for a
    /// literal, for the end mark, and for the lengths and distances a symbol
    /// names on its own.
    extra_bits: f64,
    extra: f64,
    /// The byte, the length, or the distance, depending which code this is.
    value: f64,
    /// Which step of the trace set this symbol's code length, so the panel can
    /// send a reader to it. Absent for a fixed block, whose tables are in RFC
    /// 1951 rather than in the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    entry: Option<f64>,
    /// The same step counted from the front of its block, which is where it
    /// sits among the block node's children.
    #[serde(skip_serializing_if = "Option::is_none")]
    entry_child: Option<f64>,
}

/// What one deflate symbol was made of.
#[derive(Serialize)]
struct DecodedStepDto {
    /// "literal", "end-of-block" or "match", the same words `StepKind::as_str`
    /// gives the rest of the interface.
    kind: &'static str,
    /// Which block of the trace it belongs to, as an index.
    block: f64,
    /// "fixed" or "dynamic": where the two tables came from.
    block_kind: &'static str,
    /// The literal/length code, which every symbol begins with.
    symbol: DecodedCodeDto,
    /// The distance code, for a match and for nothing else.
    #[serde(skip_serializing_if = "Option::is_none")]
    distance: Option<DecodedCodeDto>,
    /// The four widths added up, which is the step's own width in bits. Given
    /// rather than left to be summed so a view can show the arithmetic and its
    /// answer without doing the arithmetic itself.
    bits: f64,
}

fn decoded_code_dto(c: qubero_core::codec::inflate::DecodedCode) -> DecodedCodeDto {
    DecodedCodeDto {
        symbol: c.symbol as f64,
        code_bits: c.code_bits as f64,
        extra_bits: c.extra_bits as f64,
        extra: c.extra as f64,
        value: c.value as f64,
        entry: c.entry.map(|k| k as f64),
        entry_child: c.entry_child.map(|k| k as f64),
    }
}

fn decoded_step_dto(d: qubero_core::codec::inflate::DecodedStep) -> DecodedStepDto {
    use qubero_core::codec::inflate::SymbolMeaning;
    DecodedStepDto {
        // Through `StepKind::as_str` rather than three string literals, so a
        // step named here is named the same as the step the cursor link
        // already showed for the same bits. The values in the kinds are not
        // used and are not read: what is wanted is the word.
        kind: match d.meaning() {
            SymbolMeaning::Literal(_) => StepKind::Literal(0).as_str(),
            SymbolMeaning::EndOfBlock => StepKind::EndOfBlock.as_str(),
            SymbolMeaning::Length => StepKind::Match { len: 0, dist: 0 }.as_str(),
        },
        block: d.block as f64,
        block_kind: d.block_kind.as_str(),
        symbol: decoded_code_dto(d.symbol),
        distance: d.distance.map(decoded_code_dto),
        bits: d.bits() as f64,
    }
}

#[derive(Serialize)]
struct CaseDto {
    value: f64,
    name: String,
}

#[derive(Serialize)]
struct BitDto {
    bit: u32,
    /// Absent for a bit the format does not name.
    name: Option<String>,
    set: bool,
}

fn origin_dto(o: Origin) -> OriginDto {
    OriginDto {
        role: o.role.as_str(),
        label: o.label,
        path: o.path.into_iter().map(|x| x as f64).collect(),
        value: o.value,
        target_bits: o.target_bits.map(|b| b as f64),
    }
}

/// The graph as the host reads it. The one thing translated rather than copied
/// is the root's parent: the core says `usize::MAX`, which as a JSON number is
/// sixteen digits of nonsense, and -1 is what every other absent index here is.
fn graph_dto(g: Graph) -> GraphDto {
    GraphDto {
        nodes: g
            .nodes
            .into_iter()
            .map(|n| GraphNodeDto {
                path: n.path.into_iter().map(|x| x as f64).collect(),
                name: n.name,
                kind: n.kind,
                offset_bits: n.offset_bits as f64,
                size_bits: n.size_bits as f64,
                parent: if n.parent == NO_PARENT { -1.0 } else { n.parent as f64 },
                child_count: n.child_count as f64,
                truncated: n.truncated,
            })
            .collect(),
        edges: g
            .edges
            .into_iter()
            .map(|e| GraphEdgeDto { from: e.from as f64, to: e.to as f64, role: e.role })
            .collect(),
        omitted: g.omitted as f64,
    }
}

fn part_dto(p: qubero_core::formats::ggml_quant::Part) -> PartDto {
    PartDto { field: p.field.to_string(), bit: f64::from(p.bit), width: f64::from(p.width), shift: f64::from(p.shift) }
}

fn explain_dto(e: Explain) -> ExplainDto {
    let mut dto = ExplainDto {
        kind: "plain",
        name: String::new(),
        expected: Vec::new(),
        actual: Vec::new(),
        cases: Vec::new(),
        named: String::new(),
        current: 0.0,
        hex: false,
        bits: Vec::new(),
        format: String::new(),
        width: 0.0,
        pattern: String::new(),
        scale: 0.0,
        second_name: String::new(),
        second: 0.0,
        second_subtract: false,
        second_per_group: false,
        block_bits: 0.0,
        xref_widths: Vec::new(),
        xref_predictor: -1.0,
        xref_packed: 0.0,
        xref_decoded: 0.0,
        xref_free: 0.0,
        xref_in_file: 0.0,
        xref_in_stream: 0.0,
        xref_unknown: 0.0,
        xref_rows: Vec::new(),
        xref_total: 0.0,
        row_declared: 0.0,
        row_found: 0.0,
        row_on_page: 0.0,
        row_pages: Vec::new(),
        row_chain: 0.0,
        row_columns: Vec::new(),
        row_total_columns: 0.0,
        objstm_packed: 0.0,
        objstm_decoded: 0.0,
        objstm_extends: -1.0,
        objstm_objects: Vec::new(),
        objstm_total: 0.0,
        chunk_packed: 0.0,
        chunk_decoded: 0.0,
        chunk_steps: Vec::new(),
        chunk_element_type: String::new(),
        chunk_values: Vec::new(),
        chunk_total: 0.0,
        problem: String::new(),
        groups: Vec::new(),
        group_weights: 0.0,
        bias: 0.0,
        signed: false,
        weights: Vec::new(),
        at: -1.0,
    };
    match e {
        Explain::Plain => {}
        Explain::Magic { expected, actual } => {
            dto.kind = "magic";
            dto.expected = expected;
            dto.actual = actual;
        }
        Explain::Enum { name, hex, cases, current, named } => {
            dto.kind = "enum";
            dto.name = name;
            dto.hex = hex;
            dto.current = current as f64;
            dto.cases = cases.into_iter().map(|(value, name)| CaseDto { value: value as f64, name }).collect();
            dto.named = named.unwrap_or_default();
        }
        Explain::Quant { kind, bits, d, second, block_bits, groups, group_weights, bias, signed, weights, at } => {
            dto.kind = "quant";
            dto.name = kind.to_string();
            dto.width = f64::from(bits);
            dto.scale = d;
            if let Some(o) = second {
                dto.second_name = o.name.to_string();
                dto.second = o.value;
                dto.second_subtract = o.subtract;
                dto.second_per_group = o.per_group;
            }
            dto.block_bits = block_bits as f64;
            dto.group_weights = f64::from(group_weights);
            dto.bias = f64::from(bias);
            dto.signed = signed;
            dto.groups = groups
                .into_iter()
                .map(|g| GroupDto { scale: f64::from(g.scale), min: g.min.map(f64::from) })
                .collect();
            dto.at = at.map_or(-1.0, |i| i as f64);
            dto.weights = weights
                .into_iter()
                .map(|w| WeightDto {
                    q: f64::from(w.q),
                    value: w.value,
                    bits: part_dto(w.bits),
                    high: w.high.map(part_dto),
                })
                .collect();
        }
        Explain::XrefRows {
            widths,
            predictor,
            packed_bytes,
            decoded_bytes,
            free,
            in_file,
            in_stream,
            unknown,
            rows,
            total,
            problem,
        } => {
            use qubero_core::formats::pdf_xref::Kind;
            dto.kind = "xref";
            dto.xref_widths = widths.iter().map(|w| f64::from(*w)).collect();
            dto.xref_predictor = predictor.map_or(-1.0, f64::from);
            dto.xref_packed = packed_bytes as f64;
            dto.xref_decoded = decoded_bytes as f64;
            dto.xref_free = free as f64;
            dto.xref_in_file = in_file as f64;
            dto.xref_in_stream = in_stream as f64;
            dto.xref_unknown = unknown as f64;
            dto.xref_total = total as f64;
            dto.problem = problem.unwrap_or_default();
            dto.xref_rows = rows
                .into_iter()
                .map(|r| XrefRowDto {
                    object: r.object as f64,
                    kind: r.kind.as_str(),
                    type_raw: r.kind.raw() as f64,
                    offset: if r.kind == Kind::InFile { r.second as f64 } else { -1.0 },
                    second: r.second as f64,
                    third: r.third as f64,
                })
                .collect();
        }
        Explain::ObjStm { packed_bytes, decoded_bytes, extends, objects, total, problem, .. } => {
            dto.kind = "objstm";
            dto.objstm_packed = packed_bytes as f64;
            dto.objstm_decoded = decoded_bytes as f64;
            dto.objstm_extends = extends.map_or(-1.0, |n| n as f64);
            dto.objstm_total = total as f64;
            dto.problem = problem.unwrap_or_default();
            dto.objstm_objects = objects
                .into_iter()
                .map(|o| ObjStmObjectDto {
                    number: o.number as f64,
                    len: o.len as f64,
                    text: o.text,
                    cut: o.cut,
                })
                .collect();
        }
        Explain::SqliteRow {
            declared,
            found,
            on_page,
            pages,
            chain_length,
            columns,
            total_columns,
            problem,
        } => {
            dto.kind = "sqliterow";
            dto.row_declared = declared as f64;
            dto.row_found = found as f64;
            dto.row_on_page = on_page as f64;
            dto.row_pages = pages.into_iter().map(|p| p as f64).collect();
            dto.row_chain = chain_length as f64;
            dto.row_total_columns = total_columns as f64;
            dto.problem = problem.unwrap_or_default();
            dto.row_columns = columns
                .into_iter()
                .map(|c| {
                    let (value_kind, value, _, _) = shown(&c.value);
                    SqliteColumnDto { type_name: c.type_name, value, value_kind, at: c.at as f64, len: c.len as f64 }
                })
                .collect();
        }
        Explain::Hdf5Chunk { packed_bytes, decoded_bytes, steps, values, total, element_type, problem } => {
            dto.kind = "chunk";
            dto.chunk_packed = packed_bytes as f64;
            dto.chunk_decoded = decoded_bytes as f64;
            dto.chunk_total = total as f64;
            dto.chunk_element_type = element_type;
            dto.chunk_values = values;
            dto.problem = problem.unwrap_or_default();
            dto.chunk_steps = steps
                .into_iter()
                .map(|s| ChunkStepDto {
                    filter: s.filter,
                    in_bytes: s.in_bytes as f64,
                    out_bytes: s.out_bytes as f64,
                    skipped: s.skipped,
                })
                .collect();
        }
        Explain::Float { format, width, bits } => {
            dto.kind = "float";
            dto.format = format.to_string();
            dto.width = f64::from(width);
            dto.pattern = format!("{bits:0>width$x}", width = width as usize / 4);
        }
        Explain::Flags { name, raw, bits } => {
            dto.kind = "flags";
            dto.name = name;
            dto.current = raw as f64;
            dto.bits = bits.into_iter().map(|b| BitDto { bit: b.bit, name: b.name, set: b.set }).collect();
        }
    }
    dto
}

/// One rule's answer about what made the file.
#[derive(Serialize)]
struct ToolDto {
    /// The database's own word: `packer`, `compiler`, `protector`.
    category: String,
    name: String,
    version: Option<String>,
    /// Free text from the rule's author, passed through as written.
    options: Option<String>,
    /// The signature file that answered.
    source: String,
}

/// The range a successful `write_node` touched.
#[derive(Serialize)]
struct WriteDto {
    offset_bits: f64,
    size_bits: f64,
}

#[derive(Serialize)]
#[serde(tag = "status")]
enum Reply<T: Serialize> {
    #[serde(rename = "ok")]
    Ok {
        node: T,
        /// Chunks the answer was given without: previews that had not arrived.
        /// Fetching these and asking again fills them in.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        wanted: Vec<f64>,
    },
    #[serde(rename = "pending")]
    Pending { chunks: Vec<f64>, reached_bytes: f64 },
    #[serde(rename = "working")]
    Working { reached_bytes: f64 },
    #[serde(rename = "error")]
    Error { message: String },
}

/// One object of an HDF5 file, in the file's own terms.
#[derive(Serialize)]
struct ContentDto {
    /// Where it is in the template, so picking a row moves the cursor.
    path: Vec<usize>,
    /// The path it goes by inside the file: `/obs/n_genes`.
    name: String,
    group: bool,
    /// What the file calls it, where it says: `dataframe`, `csr_matrix`.
    encoding: String,
    shape: Vec<f64>,
    /// What one element is.
    element: String,
    /// Which of the three ways its bytes are kept: "contiguous", "compact",
    /// "chunked", or nothing at all for a group.
    storage: &'static str,
    /// How many bytes, where they are in one run.
    bytes: f64,
    /// The chunk it is kept in, where it is kept in chunks.
    chunk_dims: Vec<f64>,
    /// The filters its chunks were written through, in that order.
    filters: Vec<String>,
    /// Where its object header is.
    address: f64,
}

/// What an HDF5 file holds, and what kind of file it is.
#[derive(Serialize)]
struct ContentsDto {
    objects: Vec<ContentDto>,
    total: f64,
    /// Whether it is an AnnData object, and what the root group calls itself.
    anndata: bool,
    encoding: String,
    rows: f64,
    columns: f64,
}

/// One node of an HDF5 version 1 B-tree, as much of it as a picture of the
/// tree needs.
#[derive(Serialize)]
struct TreeNodeDto {
    path: Vec<usize>,
    /// Index into the node list, or -1 for the root.
    parent: f64,
    /// `index` for a `TREE` node, `links` for the symbol table node a group
    /// tree hangs under its bottom row. The words are the host's to translate;
    /// what crosses is which of the two kinds of node this is.
    kind: &'static str,
    address: f64,
    size_bits: f64,
    /// What the file wrote as this node's level. Zero for a link table, which
    /// sits below the levels rather than on one.
    level: f64,
    /// Rows below the root, counted by the walk.
    depth: f64,
    /// `entries_used` or `symbol_count`, the file's own count and no fraction
    /// of anything.
    entries: f64,
    /// The ends of the node's key range, or empty where the walk could not
    /// settle both. For a group tree these are link names; for a chunk tree
    /// they are the comma-separated numbers of a chunk's offset in the dataset.
    first_key: String,
    last_key: String,
    /// True when children of this node were not reached, so its count stands
    /// and its range does not.
    truncated: bool,
}

/// One HDF5 version 1 B-tree, walked into the shape it has in the file.
#[derive(Serialize)]
struct TreeDto {
    /// `group` for a tree indexing a group's links, `chunk` for one indexing a
    /// dataset's chunks. The two do not have the same silhouette: only a group
    /// tree has link tables under its bottom row.
    job: &'static str,
    nodes: Vec<TreeNodeDto>,
    omitted: f64,
    /// How many numbers one chunk key holds: one per dataset dimension plus one
    /// more that HDF5 writes as an offset inside an element and always sets to
    /// zero. Zero for a group tree.
    coords: f64,
}

/// The named parts of an ELF file. Unlike the storage template, these have
/// resolved section and symbol names rather than string-table offsets.
#[derive(Serialize)]
struct ElfContentsDto {
    sections: Vec<ElfSectionDto>,
    symbols: Vec<ElfSymbolDto>,
    symbol_total: f64,
}

#[derive(Serialize)]
struct ElfSectionDto {
    path: Vec<usize>,
    name: String,
    kind: f64,
    address: f64,
    offset: f64,
    size: f64,
}

#[derive(Serialize)]
struct ElfSymbolDto {
    path: Vec<usize>,
    source_bits: f64,
    name: String,
    kind: f64,
    section: f64,
    value: f64,
    size: f64,
}

#[derive(Serialize)]
struct IsoVolumeDto {
    descriptor_path: Vec<usize>,
    volume: String,
    joliet: bool,
    block_size: f64,
    blocks: f64,
    root_extent: f64,
    root_size: f64,
    root_source_bits: f64,
}

#[derive(Serialize)]
struct IsoDirectoryDto {
    entries: Vec<IsoEntryDto>,
    total: f64,
}

#[derive(Serialize)]
struct IsoEntryDto {
    name: String,
    directory: bool,
    extent: f64,
    size: f64,
    source_bits: f64,
    extents: f64,
    multi_extent: bool,
}

/// One entry of the annotation column.
#[derive(Serialize)]
struct SpanDto {
    path: Vec<usize>,
    name: String,
    /// What it sits inside, outermost first.
    trail: Vec<String>,
    #[serde(rename = "type")]
    type_name: String,
    offset_bits: f64,
    size_bits: f64,
    value: String,
    kind: &'static str,
    /// No field covers these bits.
    gap: bool,
    /// Fields this entry stands for, when a run of numbers is shown as one.
    count: f64,
    /// What one of those is called, singular. Null when the format has no word
    /// for them and they read as values.
    unit: Option<String>,
    /// A structure that reads on one row, already joined. Null for a field that
    /// reads as its own value.
    line: Option<String>,
    /// The first few values of a run shown as one entry.
    sample: Vec<String>,
    /// First element extents followed by the uninspected remainder.
    parts: Vec<SpanPartDto>,
    /// How a variable-length number's bits divide into framing and value.
    /// Null for a field that reads as whole bytes, which is most of them.
    bits: Option<BitRolesDto>,
    /// These bytes are a document of their own and can be opened as one.
    opens: bool,
}

/// The bit split of one variable-length number, in the order it is stored.
#[derive(Serialize)]
struct BitRolesDto {
    /// Which rule a reader has to know to follow the split. The view keys its
    /// wording off this; the core does not carry the wording.
    rule: &'static str,
    groups: Vec<BitGroupDto>,
}

#[derive(Serialize)]
struct BitGroupDto {
    bits: String,
    role: &'static str,
}

#[derive(Serialize)]
struct SpanPartDto {
    size_bits: f64,
    label: String,
    rest: bool,
}

#[derive(Serialize)]
struct ExtentEstimateDto {
    path: Vec<usize>,
    measured_items: f64,
    total_items: f64,
    measured_bits: f64,
    estimated_bits: f64,
}

fn extent_estimate_dto(estimate: ExtentEstimate) -> ExtentEstimateDto {
    ExtentEstimateDto {
        path: estimate.path,
        measured_items: estimate.measured_items as f64,
        total_items: estimate.total_items as f64,
        measured_bits: estimate.measured_bits as f64,
        estimated_bits: estimate.estimated_bits as f64,
    }
}

fn span_part_dto(part: SpanPart) -> SpanPartDto {
    SpanPartDto {
        size_bits: part.size_bits as f64,
        label: part.label,
        rest: part.rest,
    }
}

/// Whether a row is one machine instruction, which is what the type column
/// says when a template read it with a decoder.
fn is_machine(type_name: &str) -> bool {
    qubero_core::code::Isa::named(type_name).is_some()
}

fn span_dto(s: Span) -> SpanDto {
    let (kind, value, _, _) = shown(&s.value);
    SpanDto {
        path: s.path,
        name: s.name,
        trail: s.trail,
        type_name: s.type_name,
        offset_bits: s.offset_bits as f64,
        size_bits: s.size_bits as f64,
        value,
        kind,
        gap: s.gap,
        count: s.count as f64,
        unit: s.unit,
        line: s.line,
        sample: s.sample,
        parts: s.parts.into_iter().map(span_part_dto).collect(),
        opens: s.opens,
        bits: s.bits.map(|b| BitRolesDto {
            rule: b.rule,
            groups: b
                .groups
                .into_iter()
                .map(|g| BitGroupDto { bits: g.bits, role: g.role.as_str() })
                .collect(),
        }),
    }
}

/// How a value reads: its kind, what to show, what an editor starts with, and
/// whether the format says it is right.
fn shown(v: &Value) -> (&'static str, String, String, bool) {
    match v {
        Value::UInt(v) => ("uint", v.to_string(), v.to_string(), true),
        Value::Int(v) => ("int", v.to_string(), v.to_string(), true),
        Value::Float(v) => ("float", v.to_string(), v.to_string(), true),
        Value::Bytes { len, preview } => {
            let hex: Vec<String> = preview.iter().map(|b| format!("{b:02x}")).collect();
            let mut s = hex.join(" ");
            if *len as usize > preview.len() {
                s.push('…');
            }
            ("bytes", s.clone(), s, true)
        }
        // The bytes have not arrived; the row stands on what the file's own
        // table already said about where they are and how many there are.
        Value::Unread { .. } => ("unread", "\u{2026}".into(), String::new(), true),
        // A slot the file left at its format's "nobody filled this in" value.
        // The editor still starts from the number that is written there, so
        // opening the field shows what would be overwritten.
        Value::Unset(inner) => ("unset", "unset".into(), shown(inner).2, true),
        Value::Str(s) => ("str", s.clone(), s.clone(), true),
        Value::Magic { ok, bytes, expected } => {
            // How a signature reads is core's answer, not this crate's, so
            // that the listing and the type table say the same thing about
            // the same bytes. See `eval::magic_reading`.
            let s = qubero_core::eval::magic_reading(*ok, bytes, expected);
            ("magic", s.clone(), s, *ok)
        }
        Value::Composite { count } => ("composite", count.to_string(), count.to_string(), true),
        Value::Flags { raw, set, unnamed } => {
            // The names, then a count of the set bits nobody named, which is
            // the anomaly worth noticing in a field like this.
            let mut s = set.join(", ");
            if *unnamed > 0 {
                if !s.is_empty() {
                    s.push_str(", ");
                }
                s.push_str(&format!("+{unnamed} unnamed"));
            }
            if s.is_empty() {
                s.push_str("none set");
            }
            ("flags", s, raw.to_string(), true)
        }
        Value::Enum { raw, name, hex } => {
            let num = if *hex && *raw >= 0 { format!("0x{raw:02x}") } else { raw.to_string() };
            match name {
                Some(n) => ("enum", format!("{n} ({num})"), n.clone(), true),
                // A value the format does not define. Worth flagging, still editable.
                None => ("enum", format!("{num} (unknown)"), num, false),
            }
        }
    }
}

fn dto(n: NodeInfo) -> NodeDto {
    let (kind, value, edit_text, ok) = shown(&n.value);
    NodeDto {
        path: n.path,
        name: n.name,
        type_name: n.type_name,
        offset_bits: n.offset_bits as f64,
        size_bits: n.size_bits as f64,
        value,
        // The core's own answer where it has one: a JSON number is edited as
        // the digits the file wrote rather than as a reading of them.
        edit_text: n.edit_text.unwrap_or(edit_text),
        kind,
        ok,
        child_count: n.child_count as f64,
        unit: n.unit.unwrap_or_default(),
        composite: n.composite,
        editable: n.editable,
        value_bytes: n.value_bytes as f64,
        value_offset_bits: n.value_offset_bits as f64,
        read_as: n.read_as,
        consumed_by: n.consumed_by.map(|i| i as f64),
        machinery: n.machinery,
        contents: n.contents,
        inline: n.inline,
        framed: n.framed,
        space: n.space as f64,
        refused: n.refused,
        decoded: n.decoded,
        space_root: n.space_root,
    }
}

/// Chunks the evaluator answered without, as the host counts them.
fn cell_dto(c: qubero_core::eval::Cell) -> CellDto {
    CellDto {
        index: c.index as f64,
        offset_bits: c.offset_bits as f64,
        size_bits: c.size_bits as f64,
        text: c.text,
        label: c.label,
        kind: c.kind,
        contiguous: c.contiguous,
        repeat: c.repeat,
    }
}

fn kind_totals_dto(t: qubero_core::eval::KindTotals) -> KindTotalsDto {
    KindTotalsDto {
        done: t.done,
        reached_bits: t.reached_bits as f64,
        covered_bits: t.covered_bits as f64,
        unmapped_bits: t.unmapped_bits as f64,
        totals: t
            .totals
            .into_iter()
            .map(|k| KindTotalDto { kind: k.kind, type_name: k.type_name, bits: k.bits as f64, count: k.count as f64 })
            .collect(),
    }
}

fn wanted(e: &Evaluator) -> Vec<f64> {
    e.wanted().into_iter().map(|m| m.chunk as f64).collect()
}

fn reply_with<T: Serialize>(r: Result<T, EvalError>, reached_bytes: f64, wanted: Vec<f64>) -> String {
    let rep = match r {
        Ok(node) => Reply::Ok { node, wanted },
        Err(EvalError::Pending(m)) => {
            Reply::Pending { chunks: m.into_iter().map(|m| m.chunk as f64).collect(), reached_bytes }
        }
        Err(EvalError::Busy { reached_bits }) => Reply::Working { reached_bytes: (reached_bits / 8) as f64 },
        Err(EvalError::Failed(message)) => Reply::Error { message },
    };
    serde_json::to_string(&rep).unwrap_or_else(|e| format!("{{\"status\":\"error\",\"message\":{:?}}}", e.to_string()))
}

/// The same, for the callers with nothing further to say.
fn reply<T: Serialize>(r: Result<T, EvalError>) -> String {
    reply_with(r, 0.0, Vec::new())
}

/// What one step of a search found, as the host reads it. `status` is the same
/// tri-state everything else here answers with, so the caller's chunk-fetching
/// loop is the one it already has.
#[derive(Serialize)]
#[serde(tag = "step")]
enum StepDto {
    #[serde(rename = "found")]
    Found { at: f64, len: f64 },
    #[serde(rename = "more")]
    More { resume: f64 },
    #[serde(rename = "end")]
    End,
}

/// Build a needle from what the search bar holds. `kind` is "hex", "text" or
/// "regex"; `fold` only means anything for text.
fn needle(kind: &str, text: &str, fold: bool) -> Result<Needle, String> {
    match kind {
        "hex" => search::parse_hex(text).map(Needle::Bytes).ok_or_else(|| hex_trouble(text).to_string()),
        "regex" => search::Pattern::new(text).map(Needle::Regex),
        _ => {
            let bytes = text.as_bytes().to_vec();
            Ok(if fold { Needle::Fold(bytes) } else { Needle::Bytes(bytes) })
        }
    }
}

/// What is wrong with a hex needle. The two are different mistakes: a letter
/// that is not a digit is one, and a byte with one digit so far is the state
/// every valid needle passes through while it is being typed.
fn hex_trouble(text: &str) -> &'static str {
    if text.chars().any(|c| !c.is_whitespace() && !c.is_ascii_hexdigit()) {
        HEX_NOT_A_DIGIT
    } else {
        HEX_HALF_A_BYTE
    }
}

/// How many elements of a list to place before handing back, so the page can
/// draw what it has and say how far it has got. Around a twentieth of a second
/// of work: short enough that the page stays under the hand, long enough that
/// handing back is not most of what is done. Asking again carries on where the
/// last go left off rather than starting over.
const WORK_SLICE: u64 = 5_000;

/// Chunk size of a space's store. The bytes are already in memory, so this only
/// decides how they are cut up; it matches the file's so the host's chunk
/// arithmetic is the same on both sides.
const SPACE_CHUNK: u64 = 64 * 1024;

/// What `open_space` came to: the space, or why it would not open.
#[derive(Serialize)]
struct SpaceDto {
    space: f64,
    /// The template reading the unpacked bytes, which is the one that declared
    /// the stream. Empty for a stream that did not open.
    template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    refused: Option<String>,
}

/// One step of a decoder, as the cursor link shows it.
#[derive(Serialize)]
struct MapStepDto {
    in_start: f64,
    in_end: f64,
    out_start: f64,
    out_end: f64,
    /// The word the interface looks the step's message up by: `literal`,
    /// `match`, `stored`, `pixel`, `header`, `table`, `end-of-block`, `block`
    /// or `opaque`. [`StepKind::as_str`] is the one place these are named.
    kind: &'static str,
    /// Which named field, for a header or a table step: `bfinal`, `hlit`,
    /// `code_len` and the rest.
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<&'static str>,
    /// What that field said, where saying it is the point.
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<f64>,
    /// A match's only: how long it is and how far back it reaches.
    #[serde(skip_serializing_if = "Option::is_none")]
    len: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dist: Option<f64>,
}

fn step_dto(s: MapStep) -> MapStepDto {
    let mut dto = MapStepDto {
        in_start: s.in_bits.start as f64,
        in_end: s.in_bits.end as f64,
        out_start: s.out_bytes.start as f64,
        out_end: s.out_bytes.end as f64,
        kind: s.kind.as_str(),
        field: None,
        value: None,
        len: None,
        dist: None,
    };
    match s.kind {
        StepKind::Header(f, v) => {
            dto.field = Some(f.as_str());
            dto.value = Some(v as f64);
        }
        // A table step is a code length: which alphabet it belongs to, and the
        // symbol and length it gave. The counts of a repeat go in `len`, which
        // is where the interface looks for "how many".
        StepKind::Table(t) => {
            use qubero_core::codec::TableField;
            dto.field = Some(match t {
                TableField::CodeLen { .. } => "code_len",
                TableField::LitLen { .. } => "lit_len",
                TableField::Dist { .. } => "dist_len",
                TableField::Repeat { .. } => "repeat",
            });
            match t {
                TableField::CodeLen { sym, len } => {
                    dto.value = Some(sym as f64);
                    dto.len = Some(len as f64);
                }
                TableField::LitLen { sym, len } | TableField::Dist { sym, len } => {
                    dto.value = Some(sym as f64);
                    dto.len = Some(len as f64);
                }
                TableField::Repeat { code, count, len, .. } => {
                    dto.value = Some(code as f64);
                    dto.len = Some(count as f64);
                    dto.dist = Some(len as f64);
                }
            }
        }
        StepKind::Literal(b) => dto.value = Some(b as f64),
        StepKind::Match { len, dist } => {
            dto.len = Some(len as f64);
            dto.dist = Some(dist as f64);
        }
        StepKind::Stored | StepKind::Pixel | StepKind::EndOfBlock | StepKind::Block | StepKind::Opaque => {}
    }
    dto
}

const HEX_NOT_A_DIGIT: &str = "Hex is pairs of digits 0-9 a-f, like 89 50 4e 47";
const HEX_HALF_A_BYTE: &str = "Unfinished byte: each byte is two digits, like 4e";

#[wasm_bindgen]
impl Editor {
    /// `len` is the original file length in bytes. Chunks of `chunk_size` bytes
    /// are pushed in by the host with `feed_chunk`; at most `capacity` are kept.
    #[wasm_bindgen(constructor)]
    pub fn new(len: f64, chunk_size: u32, capacity: u32) -> Editor {
        let store = ChunkStore::new(len as u64, chunk_size as u64, capacity as usize);
        Editor { sheets: vec![Sheet::new(store, Vec::new())], live: 0 }
    }


    /// One space's reading, for a call that names it and asks nothing else of
    /// the editor. A number past the spaces open falls back to the file rather
    /// than panicking; nothing should ask, since an edit reopens the spaces in
    /// place rather than dropping them and a tab's number stays good for as
    /// long as the tab does.
    fn at(&self, space: u32) -> &Sheet {
        self.sheets.get(space as usize).unwrap_or(&self.sheets[0])
    }

    fn sm(&mut self) -> &mut Sheet {
        let i = if self.live < self.sheets.len() { self.live } else { 0 };
        &mut self.sheets[i]
    }

    /// Name the space the call is about. Every space-taking method starts here.
    fn go(&mut self, space: u32) {
        self.live = space as usize;
    }

    /// Drop every unpacked stream. A space is worked out from bytes of the file,
    /// so an edit or a change of template throws it away and the tab reopens it
    /// by path. The file's own reading, space 0, stays.
    fn forget_spaces(&mut self) {
        self.live = 0;
        if self.sheets.len() < 2 {
            return;
        }
        // The spaces are not dropped, because the tabs showing them are still
        // open and a tab has to keep meaning what its title says. Each is
        // unpacked again from the stream it came from, so the tab's own number
        // stays good; the core renumbers its own spaces from 1 and the new
        // number is written back here. A stream that no longer opens leaves an
        // empty space rather than falling back to the file's bytes under the
        // stream's name.
        let origins: Vec<Vec<usize>> =
            self.sheets[1..].iter().map(|sh| sh.origin.clone()).collect();
        for (i, origin) in origins.into_iter().enumerate() {
            let file = &mut self.sheets[0];
            let opened = match &mut file.eval {
                None => None,
                Some(e) => {
                    e.set_slice(None);
                    let got = e.open_space(&file.doc, 0, &origin).ok().flatten();
                    e.set_slice(Some(WORK_SLICE));
                    got
                }
            };
            let sheet = match opened {
                Some(id) => match self.sheets[0].eval.as_mut().and_then(|e| e.space_mut(id)) {
                    Some(sp) => Sheet::from_space(sp, origin),
                    None => Sheet::empty(origin),
                },
                None => Sheet::empty(origin),
            };
            self.sheets[i + 1] = sheet;
        }
    }

    /// Open the `Decoded` stream at `path` as a document of its own, and give
    /// back the space it became: {status:"ok",node:{space,refused}}.
    ///
    /// A stream already open answers with the space it already is, so a second
    /// Open unpacked focuses the tab instead of unpacking the run again.
    /// `refused` says which of the three ways a stream would not open, and the
    /// space is then 0.
    pub fn open_space(&mut self, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        if let Some(i) = self.sheets.iter().position(|sh| !sh.origin.is_empty() && sh.origin == p) {
            let template = self.sheets[i].template.clone();
            return reply(Ok(SpaceDto { space: i as f64, template, refused: None }));
        }
        self.live = 0;
        let file = &mut self.sheets[0];
        let Some(e) = &mut file.eval else {
            return reply::<SpaceDto>(Err(EvalError::Failed("no template".into())));
        };
        // Unpacking a run is not something to do by halves: it reads the whole
        // run and decodes it, and a half-decoded stream is not a document.
        e.set_slice(None);
        let opened = e.open_space(&file.doc, 0, &p);
        e.set_slice(Some(WORK_SLICE));
        let id = match opened {
            Ok(Some(id)) => id,
            // The stream would not open. Which of the three ways is already on
            // the node, so the reply only has to say that it did not.
            Ok(None) => {
                let why = self.refusal_at(&p);
                return reply(Ok(SpaceDto { space: 0.0, template: String::new(), refused: Some(why) }));
            }
            Err(err) => return reply::<SpaceDto>(Err(err)),
        };
        let Some(sp) = self.sheets[0].eval.as_mut().and_then(|e| e.space_mut(id)) else {
            return reply::<SpaceDto>(Err(EvalError::Failed("space vanished".into())));
        };
        let sheet = Sheet::from_space(sp, p);
        let template = sheet.template.clone();
        self.sheets.push(sheet);
        reply(Ok(SpaceDto { space: (self.sheets.len() - 1) as f64, template, refused: None }))
    }

    /// Why the stream at `path` would not open, in the core's own word for it.
    fn refusal_at(&mut self, path: &[usize]) -> String {
        let file = &mut self.sheets[0];
        let Some(e) = &mut file.eval else { return "failed".into() };
        match e.node(&file.doc, path) {
            Ok(n) => n.refused.unwrap_or_else(|| "failed".into()),
            Err(_) => "failed".into(),
        }
    }

    /// Which bits of the compressed run the byte at `byte` of `space` came
    /// from, and by which step: {status:"ok",node:{..}} or a null node when the
    /// codec's map does not reach that far.
    pub fn map_out(&mut self, space: u32, byte: f64) -> String {
        let Some(core) = self.core_space(space) else { return reply(Ok(None::<MapStepDto>)) };
        let Some(e) = &self.sheets[0].eval else { return reply(Ok(None::<MapStepDto>)) };
        reply(Ok(e.map_out(core, byte as u64).map(step_dto)))
    }

    /// Which step read the bit at `bit` of the run `space` was unpacked from,
    /// and so which of its bytes that bit produced.
    pub fn map_in(&mut self, space: u32, bit: f64) -> String {
        let Some(core) = self.core_space(space) else { return reply(Ok(None::<MapStepDto>)) };
        let Some(e) = &self.sheets[0].eval else { return reply(Ok(None::<MapStepDto>)) };
        reply(Ok(e.map_in(core, bit as u64).map(step_dto)))
    }

    /// Take the deflate symbol at `bit` of the run `space` was unpacked from
    /// apart, so the sidebar can show what it was made of:
    /// {status:"ok",node:{kind,block,block_kind,symbol,distance,bits}}, or a
    /// null node when there is nothing to take apart.
    ///
    /// `bit` is a bit of the compressed run, counted the way the trace counts
    /// them, which is what `map_out` already hands back as `in_start`. The
    /// step it lands in is found the same way `map_in` finds one.
    ///
    /// Asked for rather than done on the way past. Nothing here is in the
    /// trace: a step is a packed twenty bytes that says a match copied three
    /// bytes from four back, and which symbols carried those two numbers, how
    /// wide their codes were and how many extra bits followed each is thrown
    /// away, because keeping it would cost every step of every stream in
    /// memory. So the block's two Huffman tables are rebuilt and the step's
    /// bits read again, which is worth doing for the one step under the cursor
    /// and not for the millions behind it. Nothing calls this while scrolling.
    ///
    /// Only for a stream this editor has opened as a document of its own,
    /// since that is what carries the trace and the number to ask by. A null
    /// node is the answer for every other case: a codec that is not deflate, a
    /// bit nobody read, a step that is a header or a table entry rather than a
    /// symbol, and a block whose symbols the trace stopped naming.
    pub fn decode_step(&mut self, space: u32, bit: f64) -> String {
        let none = || reply(Ok(None::<DecodedStepDto>));
        let (Some(core), Some(sh)) = (self.core_space(space), self.sheets.get(space as usize)) else {
            return none();
        };
        let origin = sh.origin.clone();
        // Which step, and whether this is a codec with symbols at all. Both
        // come off the trace, so a question with no answer is answered before
        // any of the run is read.
        let (index, want_bytes) = {
            let Some(e) = &self.sheets[0].eval else { return none() };
            let Some(sp) = e.space(core) else { return none() };
            if !matches!(sp.codec, Codec::Deflate | Codec::Zlib | Codec::Gzip) {
                return none();
            }
            let trace = sp.trace();
            let Some(index) = trace.index_in(bit as u64) else { return none() };
            let Some(step) = trace.step(index) else { return none() };
            // As much of the run as the step reaches, and no more. Bit
            // positions count from the front of the run, so a prefix ending
            // after the step is all the walk can touch.
            (index, step.in_bits.end.div_ceil(8))
        };
        // The compressed bytes, which the space does not hold: what it holds is
        // what came out. They are a field of the file, read in the file's own
        // reading, and a read can want chunks that are not here yet.
        self.live = 0;
        let file = &mut self.sheets[0];
        let Some(e) = &mut file.eval else { return none() };
        e.begin_slice();
        let run = match e.field_bytes(&file.doc, &origin, want_bytes) {
            Ok((bytes, _)) => bytes,
            Err(err) => return reply::<Option<DecodedStepDto>>(Err(err)),
        };
        let Some(e) = &self.sheets[0].eval else { return none() };
        let Some(sp) = e.space(core) else { return none() };
        reply(Ok(inflate::decode_step(&run, sp.trace(), index).map(decoded_step_dto)))
    }

    /// The `Decoded` node a space was unpacked from, as a path in the file.
    /// Empty for space 0 and for a space that is no longer open.
    pub fn space_origin(&self, space: u32) -> Vec<u32> {
        match self.sheets.get(space as usize) {
            Some(sh) if !sh.origin.is_empty() => sh.origin.iter().map(|&x| x as u32).collect(),
            _ => Vec::new(),
        }
    }

    /// True when the template reading a space came from looking at the unpacked
    /// bytes rather than from what the stream declared: a gzip of a tar opens
    /// as a tar, and this is what says so.
    pub fn space_recognised(&self, space: u32) -> bool {
        let Some(core) = self.core_space(space) else { return false };
        self.sheets[0].eval.as_ref().and_then(|e| e.space(core)).is_some_and(|s| s.recognised)
    }

    /// The core's number for one of this editor's spaces, if it is still open.
    fn core_space(&self, space: u32) -> Option<SpaceId> {
        let sh = self.sheets.get(space as usize)?;
        if sh.core_space == 0 {
            return None;
        }
        Some(sh.core_space)
    }

    fn changed(&mut self) {
        self.forget_spaces();
        let sh = self.sm();
        if let Some(e) = &mut sh.eval {
            e.invalidate();
        }
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
        sh.scan = None;
        sh.focus = None;
        sh.kinds = None;
    }

    /// An edit that replaced bits in place at `bit`. What the template made of
    /// the bytes before it still holds, so only the rest is worked out again.
    fn changed_at(&mut self, bit: u64) {
        self.forget_spaces();
        let sh = self.sm();
        if let Some(e) = &mut sh.eval {
            e.invalidate_from(bit);
        }
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
        sh.scan = None;
        sh.focus = None;
        sh.kinds = None;
    }

    /// One step of the byte-class scan behind the overview: at most a window
    /// of the file read and classified. The reply is the usual tri-state, with
    /// `node` carrying everything found so far, so the host can draw a partial
    /// map while the rest is read. `done` on the node says when to stop asking.
    pub fn overview_step(&mut self, space: u32, buckets: u32) -> String {
        self.go(space);
        let sh = self.sm();
        let len = sh.doc.len_bytes();
        let want = overview::Scan::range(0, len, u64::from(buckets));
        // A scan already covering the same bytes at the same resolution
        // carries on; anything else starts over, since its classes describe a
        // different division of a different file.
        let same = matches!(&sh.scan, Some(s) if s.end() == len && s.bucket_bytes() == want.bucket_bytes());
        if !same {
            sh.scan = Some(want);
        }
        let scan = sh.scan.as_mut().expect("just built");
        match scan.step(&sh.doc) {
            overview::ScanStep::Pending(m) => reply::<OverviewDto>(Err(EvalError::Pending(m))),
            step => reply(Ok(OverviewDto {
                done: step == overview::ScanStep::Done,
                bucket_bytes: scan.bucket_bytes() as f64,
                total_buckets: scan.total_buckets() as f64,
                classes: scan.classes().iter().map(|&c| char::from(b'0' + c)).collect(),
                zero_bytes: scan.zero_bytes() as f64,
                text_bytes: scan.text_bytes() as f64,
                read_bytes: scan.read_bytes() as f64,
                histogram: histogram(scan),
            })),
        }
    }

    /// One step of the scan over a single block, at whatever resolution that
    /// block's own size allows. Asking about a different block starts a new
    /// one; asking about the same block carries the current one on.
    ///
    /// This is what answers the question the whole-file map cannot: every
    /// bucket of a block can read as dense while the first part of it is
    /// zeroes, because a bucket is judged as a whole.
    pub fn overview_focus_step(&mut self, space: u32, from: f64, to: f64, buckets: u32) -> String {
        self.go(space);
        let sh = self.sm();
        let (from, to) = (from as u64, to as u64);
        let fresh = !matches!(&sh.focus, Some(f) if f.start() == from && f.end() == to);
        if fresh {
            sh.focus = Some(overview::Scan::range(from, to, u64::from(buckets)));
        }
        let scan = sh.focus.as_mut().expect("just built");
        match scan.step(&sh.doc) {
            overview::ScanStep::Pending(m) => reply::<FocusDto>(Err(EvalError::Pending(m))),
            step => {
                let (entropy, entropy_max) = scan.entropy();
                let hist = scan.histogram();
                let mut common: Vec<CommonByteDto> = hist
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| **n > 0)
                    .map(|(v, n)| CommonByteDto { value: v as f64, count: *n as f64 })
                    .collect();
                common.sort_by(|a, b| b.count.total_cmp(&a.count));
                common.truncate(COMMON_BYTES);
                reply(Ok(FocusDto {
                    done: step == overview::ScanStep::Done,
                    start: scan.start() as f64,
                    end: scan.end() as f64,
                    bucket_bytes: scan.bucket_bytes() as f64,
                    total_buckets: scan.total_buckets() as f64,
                    classes: scan.classes().iter().map(|&c| char::from(b'0' + c)).collect(),
                    zero_bytes: scan.zero_bytes() as f64,
                    text_bytes: scan.text_bytes() as f64,
                    read_bytes: scan.read_bytes() as f64,
                    histogram: hist.iter().map(|&n| n as f64).collect(),
                    entropy,
                    entropy_max,
                    distinct: scan.distinct() as f64,
                    common,
                }))
            }
        }
    }

    /// One step of the walk that totals the file's bits by field kind and
    /// type. The reply is the usual tri-state, with `node` carrying the totals
    /// so far, so a view can draw a partial answer while the rest is worked
    /// out. `done` on the node says when to stop asking.
    ///
    /// What has not been walked yet is in none of the totals. The caller has
    /// `reached_bits` and the file's length and derives the rest from those,
    /// rather than being handed a bucket for it that would look like a finding
    /// about the file.
    ///
    /// An edit throws the walk away, and the next step starts it over.
    pub fn kind_totals_step(&mut self, space: u32) -> String {
        self.go(space);
        let sh = self.sm();
        let len = sh.doc.len_bits();
        let Some(e) = &mut sh.eval else {
            return reply::<KindTotalsDto>(Err(EvalError::Failed("no template".into())));
        };
        // A walk built for a file of another length is about other bytes.
        if !matches!(&sh.kinds, Some(w) if w.file_bits() == len) {
            sh.kinds = Some(KindWalk::new(len));
        }
        let walk = sh.kinds.as_mut().expect("just built");
        e.begin_slice();
        let out = e.kind_totals_step(&sh.doc, walk);
        reply_with(out.map(kind_totals_dto), (e.reached_bits() / 8) as f64, Vec::new())
    }

    // ----- templates -----

    pub fn template_names(&self) -> Vec<String> {
        formats::builtin_names().iter().map(|s| s.to_string()).collect()
    }

    /// Best current projection for a variable-size array being walked, or an
    /// empty string when no unfinished walk has enough information yet.
    pub fn extent_estimate(&self, space: u32) -> String {
        let sh = self.at(space);
        sh.eval
            .as_ref()
            .and_then(Evaluator::extent_estimate)
            .map(extent_estimate_dto)
            .map(|estimate| serde_json::to_string(&estimate).unwrap_or_default())
            .unwrap_or_default()
    }

    /// How many leading bytes `sniff_template` wants. A few formats keep what
    /// identifies them well past the start of the file, so a shorter head
    /// silently misses them.
    pub fn sniff_window(&self) -> f64 {
        formats::SNIFF_WINDOW as f64
    }

    /// Name of the built-in template matching these leading bytes, or "".
    /// `file_len` is the length of the whole file, which a format whose
    /// header is a table of offsets weighs its pointers against.
    pub fn sniff_template(&self, head: &[u8], file_len: f64) -> String {
        formats::sniff(head, file_len as u64).unwrap_or("").to_string()
    }

    /// Select a built-in template by name; "" clears it. Returns false if unknown.
    pub fn set_template(&mut self, name: &str) -> bool {
        // A different template may not have the stream a space came from, so
        // the spaces are worked out again against it before anything else.
        self.forget_spaces();
        let sh = self.sm();
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
        // Every path in the walk is a path through the template that was in
        // use, and under another template the same path is another field. The
        // byte-class scan beside it is about bytes and stands; this does not.
        sh.kinds = None;
        sh.template = name.to_string();
        if name.is_empty() {
            sh.eval = None;
            return true;
        }
        match formats::builtin(name) {
            Some(t) => {
                let mut e = Evaluator::new(t);
                e.set_slice(Some(WORK_SLICE));
                sh.eval = Some(e);
                true
            }
            None => false,
        }
    }

    /// Build a template from a `file(1)` rule file and select it, for a format
    /// with no built-in. `rules` is the text of the one rule file the
    /// identification named, `head` the file's first bytes.
    ///
    /// What comes out covers the format's signature and nothing else, so most
    /// of the file stays unannotated. Returns false when the rule pins no fixed
    /// bytes to a fixed place, which is the honest answer for a format found by
    /// searching rather than by looking.
    pub fn set_magic_template(&mut self, name: &str, rules: &str, head: &[u8]) -> bool {
        self.live = 0;
        let sh = self.sm();
        // A signature template covers a format's first bytes only, so whatever
        // full template was in use no longer applies.
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
        // The paths the walk holds are paths through the template it is
        // leaving. See `set_template`.
        sh.kinds = None;
        sh.template = String::new();
        match magicrule::match_signature(rules, head) {
            Some(sig) => {
                sh.eval = Some(Evaluator::new(magicrule::signature_template(name, &sig)));
                true
            }
            None => false,
        }
    }

    /// What the type at `path` permits, beyond what its value shows: the other
    /// values an enum names, the bytes a magic field wanted, or what each bit
    /// of a flags field means. JSON, in the same reply shape as the rest.
    ///
    /// `at_bits` is where the cursor is. Only a block of packed weights uses
    /// it, to say which weight the reader is standing on.
    pub fn type_info(&mut self, space: u32, path: &[u32], at_bits: f64) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let at = (at_bits >= 0.0).then(|| at_bits as u64);
        match &mut sh.eval {
            None => reply::<ExplainDto>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                reply(e.explain(&sh.doc, &p, at).map(explain_dto))
            }
        }
    }

    /// Which fields settled the shape of the one at `path`, and where this one
    /// points if it holds an offset. JSON, in the same reply shape as the rest;
    /// usually an empty list, since most fields are placed and sized outright.
    pub fn origins(&mut self, space: u32, path: &[u32]) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut sh.eval {
            None => reply::<Vec<OriginDto>>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                reply(e.origins(&sh.doc, &p).map(|v| v.into_iter().map(origin_dto).collect::<Vec<_>>()))
            }
        }
    }

    /// How the field at `path` was placed and how it was sized, in one word
    /// each. JSON, in the same reply shape as the rest.
    ///
    /// Answered for every field, which is what tells it from `origins`: a `u32`
    /// in a header has no origins and is still somewhere for a reason, and the
    /// reason is that the field in front of it ended there.
    pub fn shape(&mut self, space: u32, path: &[u32]) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut sh.eval {
            None => reply::<ShapeDto>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                reply(e.shape(&sh.doc, &p).map(|s| ShapeDto { placed: s.placed.as_str(), sized: s.sized.as_str() }))
            }
        }
    }

    /// What the field at `path` checks, or null when it checks nothing. JSON,
    /// in the same reply shape as the rest.
    ///
    /// Cheap, and asked of any field: no bytes of the covered run are read, so
    /// a panel can ask on every move of the cursor and decide from
    /// `covered_bytes` whether to take the sum without being asked. Taking it
    /// is `run_check`.
    pub fn check_of(&mut self, space: u32, path: &[u32]) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut sh.eval {
            None => reply::<Option<CheckDto>>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                reply(e.check_of(&sh.doc, &p).map(|c| {
                    c.map(|c| CheckDto {
                        algorithm: c.algorithm,
                        over: c.over.map(|(at, len)| [at as f64, len as f64]),
                        unpacked_from: c.unpacked_from.map(|(at, len)| [at as f64, len as f64]),
                        covered_bytes: c.covered_bytes as f64,
                        covered_exact: c.covered_exact,
                        unpacked_member: c.unpacked_member,
                        blanked: c.blanked.map(|b| [b.at as f64, b.len as f64, b.byte as f64]),
                    })
                }))
            }
        }
    }

    /// The moment the field at `path` means, or null when it means none. JSON,
    /// in the same reply shape as the rest.
    ///
    /// Cheap, and asked of any field: the field's own bytes are read, which a
    /// panel showing its value has already paid for, and nothing else is. The
    /// stored number is not in here, because it is already on the value row and
    /// this is an addition to it rather than a replacement for it.
    pub fn time_of(&mut self, space: u32, path: &[u32]) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut sh.eval {
            None => reply::<Option<TimeDto>>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                reply(e.time_of(&sh.doc, &p).map(|t| {
                    t.map(|t| {
                        let (state, unix_seconds, nanos) = match t.moment {
                            Moment::At { unix_seconds, nanos } => ("at", Some(unix_seconds as f64), Some(nanos as f64)),
                            Moment::Unset => ("unset", None, None),
                            Moment::Impossible => ("impossible", None, None),
                        };
                        TimeDto {
                            state,
                            unix_seconds,
                            nanos,
                            zone: match t.zone {
                                Zone::Utc => "utc",
                                Zone::Local => "local",
                                Zone::Unknown => "unknown",
                            },
                            step_nanos: t.step_nanos as f64,
                        }
                    })
                }))
            }
        }
    }

    /// Take the checksum at `path` and compare it with what the file wrote.
    /// JSON, in the same reply shape as the rest; null when the field checks
    /// nothing.
    ///
    /// This reads the covered bytes and unpacks the covered stream, so it is
    /// asked for rather than done on the way past, and it answers `pending`
    /// like any other read when the bytes are not here yet. A check that
    /// cannot be made is an error with the reason in it, never a mismatch.
    pub fn run_check(&mut self, space: u32, path: &[u32]) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut sh.eval {
            None => reply::<Option<VerdictDto>>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                reply(
                    e.run_check(&sh.doc, &p)
                        .map(|v| v.map(|v| VerdictDto { computed: v.computed, stored: v.stored, ok: v.ok })),
                )
            }
        }
    }

    /// The same question asked of every field under `path` at once: the nodes
    /// of the subtree and every connection between two of them, ready to be
    /// laid out. JSON, in the same reply shape as the rest.
    ///
    /// `limit` caps the nodes. The walk is breadth-first, so what a cap keeps
    /// is the top of the format rather than one deep spine of it, and the
    /// answer says how many nodes it left out.
    pub fn graph(&mut self, space: u32, path: &[u32], limit: u32) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut sh.eval {
            None => reply::<GraphDto>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                reply(e.graph(&sh.doc, &p, limit as usize).map(graph_dto))
            }
        }
    }

    /// The relationships behind the shape of the field at `path`, written out:
    /// the expression as the template holds it, the same with every field's
    /// value in its place, and what it comes to. JSON, in the same reply shape
    /// as the rest. Empty for a field the template placed and sized outright,
    /// and for one whose expression has no reading in that notation.
    pub fn relations(&mut self, space: u32, path: &[u32]) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut sh.eval {
            None => reply::<Vec<RelationDto>>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                reply(e.relations(&sh.doc, &p).map(|v| {
                    v.into_iter()
                        .map(|r| RelationDto {
                            role: r.role.as_str(),
                            written: r.written,
                            substituted: r.substituted,
                            result: r.result,
                        })
                        .collect::<Vec<_>>()
                }))
            }
        }
    }

    /// What tool produced this file, according to a bundle of Detect It Easy
    /// signature rules. `rules` is the bundle text, `head` the file's first
    /// bytes. Returns JSON, an array that is usually empty.
    ///
    /// Rules asking where the file starts running, what its sections are
    /// called or where its overlay begins are answered from the bytes here.
    /// A question the file cannot answer means the rule does not match, rather
    /// than being answered from somewhere else.
    ///
    /// One answer is not from the database: a DOS BASIC program names the
    /// runtime it was built against in its own loader stub, which no rule in
    /// the database can match on because the stub sits at a different place in
    /// every program. It is credited to this editor rather than to a rule file.
    pub fn detect_tools(&self, rules: &str, head: &[u8]) -> String {
        let sh = &self.sheets[0];
        let db = diescript::parse_bundle(rules);
        // What the file says about itself: where it starts running, what its
        // sections are called, where the overlay begins. Worked out once.
        let facts = diescript::Facts::of(head, sh.doc.len_bytes());
        let found: Vec<ToolDto> = diescript::detect(&db, head, &facts)
            .into_iter()
            .chain(dosbasic::detect(head, sh.doc.len_bytes()))
            .map(|d| ToolDto {
                category: d.category,
                name: d.name,
                version: d.version,
                options: d.options,
                source: d.source,
            })
            .collect();
        serde_json::to_string(&found).unwrap_or_else(|_| "[]".to_string())
    }

    /// JSON: {status:"ok",node} | {status:"pending",chunks} | {status:"error",message}
    pub fn template_node(&mut self, space: u32, path: &[u32]) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut sh.eval {
            None => reply::<NodeDto>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                let r = e.node(&sh.doc, &p).map(dto);
                reply_with(r, (e.reached_bits() / 8) as f64, wanted(e))
            }
        }
    }

    /// Same envelope as `template_node`, with `node` being an array of children.
    pub fn template_children(&mut self, space: u32, path: &[u32], from: f64, to: f64) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut sh.eval {
            None => reply::<Vec<NodeDto>>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                let r = e
                    .children(&sh.doc, &p, from as u64, to as u64)
                    .map(|v| v.into_iter().map(dto).collect::<Vec<NodeDto>>());
                reply_with(r, (e.reached_bits() / 8) as f64, wanted(e))
            }
        }
    }

    /// The elements of the folded run at `path` whose bits overlap
    /// `from_bit..to_bit`, at most `max` of them, for the value table beside
    /// the bytes: {status:"ok",node:[cell,..]}. Same envelope as
    /// `template_children`. `path` is what a span with a count carries, or a
    /// block of a decoded stream's trace.
    pub fn run_cells(&mut self, space: u32, path: &[u32], from_bit: f64, to_bit: f64, max: u32) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut sh.eval {
            None => reply::<Vec<CellDto>>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                let r = e
                    .run_cells(&sh.doc, &p, from_bit as u64, to_bit as u64, max as usize)
                    .map(|v| v.into_iter().map(cell_dto).collect::<Vec<CellDto>>());
                reply_with(r, (e.reached_bits() / 8) as f64, wanted(e))
            }
        }
    }

    /// Whole text of a text field, decoded in its own encoding:
    /// {status:"ok",node:{text,truncated}}.
    pub fn field_text(&mut self, space: u32, path: &[u32]) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut sh.eval {
            None => reply::<TextDto>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                reply(e.text_value(&sh.doc, &p).map(|(text, truncated)| TextDto { text, truncated }))
            }
        }
    }

    /// The first `limit` bytes of a field, read in whatever address space the
    /// field is in: {status:"ok",node:{bytes:[..],truncated}}. Use this rather
    /// than `read_bits` at the node's offset, which is the file and is the
    /// wrong bytes for anything inside a decoded stream.
    pub fn field_bytes(&mut self, space: u32, path: &[u32], limit: u32) -> String {
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut sh.eval {
            None => reply::<BytesDto>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                reply(
                    e.field_bytes(&sh.doc, &p, u64::from(limit))
                        .map(|(bytes, truncated)| BytesDto { bytes, truncated }),
                )
            }
        }
    }

    /// Every field between two bit offsets, for the annotation column:
    /// {status:"ok",node:[span,..]}. `max` caps how many come back.
    pub fn spans(&mut self, space: u32, from_bit: f64, to_bit: f64, max: u32) -> String {
        self.go(space);
        let sh = self.sm();
        let Some(e) = &mut sh.eval else {
            return reply::<Vec<SpanDto>>(Err(EvalError::Failed("no template".into())));
        };
        e.begin_slice();
        let found = match e.spans(&sh.doc, from_bit as u64, to_bit as u64, max as usize) {
            Ok(v) => v,
            Err(err) => return reply::<Vec<SpanDto>>(Err(err)),
        };
        let named = self.name_instructions(found);
        reply(Ok(named))
    }

    /// What an HDF5 file holds, read in the file's own terms rather than the
    /// template's: {status:"ok",node:{objects,..}}. Empty for every other
    /// format, since nothing else here has a group tree to walk.
    pub fn contents(&mut self, space: u32) -> String {
        self.go(space);
        let sh = self.sm();
        if sh.template != "hdf5" {
            return reply(Ok(ContentsDto {
                objects: Vec::new(),
                total: 0.0,
                anndata: false,
                encoding: String::new(),
                rows: 0.0,
                columns: 0.0,
            }));
        }
        let Some(e) = &mut sh.eval else {
            return reply::<ContentsDto>(Err(EvalError::Failed("no template".into())));
        };
        e.begin_slice();
        let found = match qubero_core::formats::h5ad::contents(e, &sh.doc) {
            Ok(c) => c,
            Err(err) => return reply::<ContentsDto>(Err(err)),
        };
        reply(Ok(ContentsDto {
            total: found.total as f64,
            anndata: found.anndata,
            encoding: found.encoding,
            rows: found.rows as f64,
            columns: found.columns as f64,
            objects: found
                .objects
                .into_iter()
                .map(|o| {
                    use qubero_core::formats::h5ad::Storage;
                    // The words a reader sees are the host's business; what
                    // crosses is which of the three ways the bytes are kept,
                    // how many there are, and the chunk it is kept in.
                    let (storage, bytes, chunk_dims, filters) = match o.storage {
                        Storage::None => ("", 0.0, Vec::new(), Vec::new()),
                        Storage::Contiguous(n) => ("contiguous", n as f64, Vec::new(), Vec::new()),
                        Storage::Compact(n) => ("compact", n as f64, Vec::new(), Vec::new()),
                        Storage::Chunked { dims, filters } => {
                            ("chunked", 0.0, dims.into_iter().map(|d| d as f64).collect(), filters)
                        }
                    };
                    ContentDto {
                        path: o.path,
                        name: o.name,
                        group: o.group,
                        encoding: o.encoding,
                        shape: o.shape.into_iter().map(|d| d as f64).collect(),
                        element: o.element,
                        storage,
                        bytes,
                        chunk_dims,
                        filters,
                        address: o.address as f64,
                    }
                })
                .collect(),
        }))
    }

    /// The HDF5 version 1 B-tree the field at `path` belongs to, walked into
    /// the shape it has in the file: {status:"ok",node:{job,nodes,..}}, or a
    /// null node where the file has no such tree to answer with.
    ///
    /// `path` is where the cursor is, which is usually not a node of a tree.
    /// The core works out which tree that means: the one the cursor is inside,
    /// else the one the object header it is inside names, else the root
    /// group's. `limit` caps the nodes walked, and the answer says how many
    /// children it left out.
    pub fn btree(&mut self, space: u32, path: &[u32], limit: u32) -> String {
        self.go(space);
        let sh = self.sm();
        if sh.template != "hdf5" {
            return reply(Ok(None::<TreeDto>));
        }
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let Some(e) = &mut sh.eval else {
            return reply::<Option<TreeDto>>(Err(EvalError::Failed("no template".into())));
        };
        e.begin_slice();
        let found = match qubero_core::formats::hdf5_tree::tree(e, &sh.doc, &p, limit as usize) {
            Ok(t) => t,
            Err(err) => return reply::<Option<TreeDto>>(Err(err)),
        };
        reply(Ok(found.map(|t| {
            use qubero_core::formats::hdf5_tree::{Job, Kind, NO_PARENT};
            TreeDto {
                job: match t.job {
                    Job::Group => "group",
                    Job::Chunk => "chunk",
                },
                omitted: t.omitted as f64,
                coords: t.coords as f64,
                nodes: t
                    .nodes
                    .into_iter()
                    .map(|n| TreeNodeDto {
                        path: n.path,
                        parent: if n.parent == NO_PARENT { -1.0 } else { n.parent as f64 },
                        kind: match n.kind {
                            Kind::Index => "index",
                            Kind::LinkTable => "links",
                        },
                        address: n.address as f64,
                        size_bits: n.size_bits as f64,
                        level: n.level as f64,
                        depth: n.depth as f64,
                        entries: n.entries as f64,
                        first_key: n.first_key,
                        last_key: n.last_key,
                        truncated: n.truncated,
                    })
                    .collect(),
            }
        })))
    }

    /// Named ELF sections and a bounded prefix of its symbols. The semantic
    /// pass is cached because resolving names crosses several linked tables.
    pub fn elf_contents(&mut self, space: u32, symbol_limit: u32) -> String {
        self.go(space);
        let sh = self.sm();
        if sh.template != "elf" && sh.template != "bpf" {
            return reply::<ElfContentsDto>(Err(EvalError::Failed("not an ELF template".into())));
        }
        let Some(e) = &mut sh.eval else {
            return reply::<ElfContentsDto>(Err(EvalError::Failed("no template".into())));
        };
        let need_symbols = symbol_limit > 0;
        if sh.bpf.is_none() || (need_symbols && !sh.bpf_complete) {
            e.set_slice(None);
            let read = if need_symbols {
                formats::ElfProgram::read(e, &sh.doc)
            } else {
                formats::ElfProgram::read_sections(e, &sh.doc)
            };
            e.set_slice(Some(WORK_SLICE));
            match read {
                Ok(program) => {
                    sh.bpf = Some(program);
                    sh.bpf_complete = need_symbols;
                }
                Err(error) => return reply::<ElfContentsDto>(Err(error)),
            }
        }
        let Some(program) = &sh.bpf else {
            return reply::<ElfContentsDto>(Err(EvalError::Failed("could not resolve ELF tables".into())));
        };
        let sections = program.sections.iter().enumerate().map(|(i, section)| ElfSectionDto {
            path: vec![7, 14, 0, i],
            name: section.name.clone(),
            kind: section.kind as f64,
            address: section.addr as f64,
            offset: section.offset as f64,
            size: section.size as f64,
        }).collect();
        let symbols = program.symbols.iter().take(symbol_limit as usize).map(|symbol| ElfSymbolDto {
            path: symbol.path.clone(),
            source_bits: symbol.source_bits as f64,
            name: symbol.name.clone(),
            kind: symbol.kind as f64,
            section: symbol.section as f64,
            value: symbol.value as f64,
            size: symbol.size as f64,
        }).collect();
        reply(Ok(ElfContentsDto {
            sections,
            symbols,
            symbol_total: program.symbol_total as f64,
        }))
    }

    /// The primary ISO 9660 volume and its root-directory pointer.
    pub fn iso_volume(&self, space: u32) -> String {
        let sh = self.at(space);
        if sh.template != "iso9660" {
            return reply::<IsoVolumeDto>(Err(EvalError::Failed("not an ISO 9660 template".into())));
        }
        let mut descriptor = vec![0u8; 2048];
        let mut primary = None;
        let mut joliet = None;
        for i in 0..64usize {
            let at = (16 + i as u64) * 2048;
            if at + 2048 > sh.doc.len_bytes() {
                break;
            }
            let missing = sh.doc.read_bytes(at, &mut descriptor);
            if !missing.is_empty() {
                return reply::<IsoVolumeDto>(Err(EvalError::Pending(missing)));
            }
            if &descriptor[1..6] != b"CD001" {
                continue;
            }
            if descriptor[0] == 255 {
                break;
            }
            let is_primary = descriptor[0] == 1;
            let is_joliet = descriptor[0] == 2
                && matches!(&descriptor[88..91], b"%/@" | b"%/C" | b"%/E");
            if !is_primary && !is_joliet {
                continue;
            }
            let le16 = |p: usize| u16::from_le_bytes([descriptor[p], descriptor[p + 1]]) as u64;
            let le32 = |p: usize| u32::from_le_bytes(descriptor[p..p + 4].try_into().unwrap()) as u64;
            let volume = if is_joliet {
                let decoded: String = descriptor[40..72]
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .filter_map(|c| char::from_u32(c as u32))
                    .collect();
                decoded.trim_end_matches([' ', '\0']).to_string()
            } else {
                String::from_utf8_lossy(&descriptor[40..72]).trim_end_matches([' ', '\0']).to_string()
            };
            let root = 156usize;
            let found = IsoVolumeDto {
                descriptor_path: vec![1, i, 3],
                volume,
                joliet: is_joliet,
                block_size: le16(128) as f64,
                blocks: le32(80) as f64,
                root_extent: le32(root + 2) as f64,
                root_size: le32(root + 10) as f64,
                root_source_bits: ((at + root as u64) * 8) as f64,
            };
            if is_joliet {
                joliet = Some(found);
            } else {
                primary = Some(found);
            }
        }
        match joliet.or(primary) {
            Some(volume) => reply(Ok(volume)),
            None => reply::<IsoVolumeDto>(Err(EvalError::Failed("primary volume descriptor not found".into()))),
        }
    }

    /// One ISO directory, bounded for display but counted in full. Child
    /// directories are read only when their logical row is opened.
    pub fn iso_directory(&self, space: u32, extent: f64, size: f64, block_size: f64, limit: u32, joliet: bool) -> String {
        let sh = self.at(space);
        if sh.template != "iso9660" {
            return reply::<IsoDirectoryDto>(Err(EvalError::Failed("not an ISO 9660 template".into())));
        }
        let block = block_size as u64;
        let len = size as u64;
        let at = (extent as u64).saturating_mul(block);
        if block == 0 || len > 32 * 1024 * 1024 || at.saturating_add(len) > sh.doc.len_bytes() {
            return reply::<IsoDirectoryDto>(Err(EvalError::Failed("invalid or unusually large ISO directory".into())));
        }
        let mut bytes = vec![0u8; len as usize];
        let missing = sh.doc.read_bytes(at, &mut bytes);
        if !missing.is_empty() {
            return reply::<IsoDirectoryDto>(Err(EvalError::Pending(missing)));
        }
        let mut entries: Vec<IsoEntryDto> = Vec::new();
        let mut total = 0usize;
        let mut last_name = String::new();
        let mut last_multi = false;
        let mut pos = 0usize;
        while pos < bytes.len() {
            let record_len = bytes[pos] as usize;
            if record_len == 0 {
                let next = ((pos as u64 / block) + 1) * block;
                pos = next as usize;
                continue;
            }
            if record_len < 34 || pos + record_len > bytes.len() {
                break;
            }
            let name_len = bytes[pos + 32] as usize;
            if pos + 33 + name_len > pos + record_len {
                break;
            }
            let raw = &bytes[pos + 33..pos + 33 + name_len];
            if raw != [0] && raw != [1] {
                let mut name: String = if joliet {
                    raw.chunks_exact(2)
                        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                        .filter_map(|c| char::from_u32(c as u32))
                        .collect()
                } else {
                    String::from_utf8_lossy(raw).into_owned()
                };
                // Rock Ridge NM carries the POSIX name in the System Use
                // area. It wins over the restricted ISO identifier.
                let mut system = pos + 33 + name_len + usize::from(name_len % 2 == 0);
                let end = pos + record_len;
                let mut rock_ridge = String::new();
                while system + 4 <= end {
                    let field_len = bytes[system + 2] as usize;
                    if field_len < 4 || system + field_len > end {
                        break;
                    }
                    if &bytes[system..system + 2] == b"NM" && field_len >= 5 {
                        rock_ridge.push_str(&String::from_utf8_lossy(&bytes[system + 5..system + field_len]));
                    }
                    system += field_len;
                }
                if !rock_ridge.is_empty() {
                    name = rock_ridge;
                } else {
                    if let Some(version) = name.rfind(';') {
                        if name[version + 1..].chars().all(|c| c.is_ascii_digit()) {
                            name.truncate(version);
                        }
                    }
                    if name.ends_with('.') {
                        name.pop();
                    }
                }
                let multi = bytes[pos + 25] & 0x80 != 0;
                let continuation = last_multi && name == last_name;
                let le32 = |p: usize| u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap()) as u64;
                let part_size = le32(pos + 10) as f64;
                if continuation {
                    if let Some(previous) = entries.last_mut() {
                        if previous.name == name {
                            previous.size += part_size;
                            previous.extents += 1.0;
                            previous.multi_extent = multi;
                        }
                    }
                } else {
                    total += 1;
                }
                if !continuation && entries.len() < limit as usize {
                    entries.push(IsoEntryDto {
                        name: name.clone(),
                        directory: bytes[pos + 25] & 2 != 0,
                        extent: le32(pos + 2) as f64,
                        size: part_size,
                        source_bits: ((at + pos as u64) * 8) as f64,
                        extents: 1.0,
                        multi_extent: multi,
                    });
                }
                last_name = name;
                last_multi = multi;
            }
            pos += record_len;
        }
        reply(Ok(IsoDirectoryDto { entries, total: total as f64 }))
    }

    /// Rewrite instruction rows through a disassembler, so a call names the
    /// function it calls. Anything that does not work out keeps the row the
    /// template already produced: a name is an improvement on a number, not a
    /// requirement for reading the file.
    fn name_instructions(&mut self, found: Vec<Span>) -> Vec<SpanDto> {
        let sh = self.sm();
        match sh.template.as_str() {
            "wasm" => self.name_wasm(found),
            "bpf" => self.name_bpf(found),
            "elf" => self.name_machine(found),
            "ne" => self.name_ne(found),
            _ => found.into_iter().map(span_dto).collect(),
        }
    }

    fn name_wasm(&mut self, found: Vec<Span>) -> Vec<SpanDto> {
        let sh = self.sm();
        if !found.iter().any(|s| s.type_name == "Instr") {
            return found.into_iter().map(span_dto).collect();
        }
        let Some(e) = &mut sh.eval else { return found.into_iter().map(span_dto).collect() };
        if sh.disasm.is_none() {
            // The module may not have streamed in far enough yet, in which case
            // this is worth trying again on the next screenful.
            sh.disasm = formats::WasmModule::read(e, &sh.doc).ok();
        }
        let Some(m) = &sh.disasm else { return found.into_iter().map(span_dto).collect() };
        found
            .into_iter()
            .map(|s| {
                let named = if s.type_name == "Instr" { m.instruction_line(e, &sh.doc, &s.path).ok() } else { None };
                let mut dto = span_dto(s);
                if let Some(line) = named {
                    dto.line = Some(line);
                }
                dto
            })
            .collect()
    }

    /// The same for eBPF, where what a line needs is in the object's tables
    /// rather than in the instruction.
    fn name_bpf(&mut self, found: Vec<Span>) -> Vec<SpanDto> {
        let sh = self.sm();
        if !found.iter().any(|s| s.type_name == "BpfInsn") {
            return found.into_iter().map(span_dto).collect();
        }
        let Some(e) = &mut sh.eval else { return found.into_iter().map(span_dto).collect() };
        if !sh.bpf_complete {
            sh.bpf = formats::ElfProgram::read(e, &sh.doc).ok();
            sh.bpf_complete = sh.bpf.is_some();
        }
        let Some(p) = &sh.bpf else { return found.into_iter().map(span_dto).collect() };
        found
            .into_iter()
            .map(|s| {
                let named = if s.type_name == "BpfInsn" { p.instruction_line(e, &sh.doc, &s.path).ok() } else { None };
                let mut dto = span_dto(s);
                if let Some(line) = named {
                    dto.line = Some(line);
                }
                dto
            })
            .collect()
    }

    /// Rewrite machine instructions through the symbol table, so a call says
    /// the name of what it calls. A file with no symbols keeps every row it
    /// had, which is most of what is on a disk: a program is usually stripped
    /// before it ships.
    fn name_machine(&mut self, found: Vec<Span>) -> Vec<SpanDto> {
        let sh = self.sm();
        if !found.iter().any(|s| is_machine(&s.type_name)) {
            return found.into_iter().map(span_dto).collect();
        }
        let Some(e) = &mut sh.eval else { return found.into_iter().map(span_dto).collect() };
        if !sh.bpf_complete {
            // Reading the symbol table of a whole program is more than one
            // screenful of work, and stopping halfway would mean starting
            // again on every screenful after it. So this one runs to an
            // answer, and what it costs is paid once.
            e.set_slice(None);
            sh.bpf = formats::ElfProgram::read(e, &sh.doc).ok();
            sh.bpf_complete = sh.bpf.is_some();
            e.set_slice(Some(WORK_SLICE));
        }
        let Some(p) = &sh.bpf else { return found.into_iter().map(span_dto).collect() };
        found
            .into_iter()
            .map(|s| {
                let named = match is_machine(&s.type_name) {
                    true => p.machine_line(e, &sh.doc, &s.path).ok().flatten(),
                    false => None,
                };
                let mut dto = span_dto(s);
                if let Some(line) = named {
                    dto.line = Some(line);
                }
                dto
            })
            .collect()
    }

    /// Rewrite the instructions of a 16-bit Windows program through its
    /// relocations, so a call into another module says which function of it
    /// the loader will point the call at.
    fn name_ne(&mut self, found: Vec<Span>) -> Vec<SpanDto> {
        let sh = self.sm();
        if !found.iter().any(|s| is_machine(&s.type_name)) {
            return found.into_iter().map(span_dto).collect();
        }
        let Some(e) = &mut sh.eval else { return found.into_iter().map(span_dto).collect() };
        if sh.ne.is_none() {
            // The same as for a program's symbols: read once, in full.
            e.set_slice(None);
            sh.ne = formats::NeProgram::read(e, &sh.doc).ok();
            e.set_slice(Some(WORK_SLICE));
        }
        let Some(p) = &sh.ne else { return found.into_iter().map(span_dto).collect() };
        found
            .into_iter()
            .map(|s| {
                let named = match (is_machine(&s.type_name), formats::NeProgram::segment_of(&s.path)) {
                    (true, Some(segment)) => p.instruction_line(e, &sh.doc, &s.path, segment).ok().flatten(),
                    _ => None,
                };
                let mut dto = span_dto(s);
                if let Some(line) = named {
                    dto.line = Some(line);
                }
                dto
            })
            .collect()
    }

    /// Path of the deepest field covering `bit`, as {status:"ok",node:[..]}.
    /// Its ancestors are the prefixes of that path.
    pub fn locate(&mut self, space: u32, bit: f64) -> String {
        self.go(space);
        let sh = self.sm();
        match &mut sh.eval {
            None => reply::<Vec<usize>>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                e.begin_slice();
                reply(e.locate(&sh.doc, bit as u64))
            }
        }
    }

    /// Write `text` into the field at `path`, encoded as that field's type.
    /// Same envelope as `template_node`; on success `node` is the bit range written.
    pub fn write_node(&mut self, space: u32, path: &[u32], text: &str) -> String {
        // A byte of an unpacked stream is a function of every compressed byte
        // before it, so there is nowhere to put a change to one. The interface
        // says so in its own words before a write gets here; this is the same
        // refusal one layer out, in the same words the evaluator would use if
        // it were reached, rather than five lowercase ones of its own.
        if space != 0 {
            return reply::<WriteDto>(Err(EvalError::Failed(qubero_core::encode::UNPACKED_MSG.into())));
        }
        self.go(space);
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let prepared = match &mut sh.eval {
            None => return reply::<WriteDto>(Err(EvalError::Failed("no template".into()))),
            Some(e) => {
                // An edit is not something to do by halves, so this one runs to
                // an answer however long it takes.
                e.set_slice(None);
                let prepared = e.prepare_write(&sh.doc, &p, text);
                e.set_slice(Some(WORK_SLICE));
                prepared
            }
        };
        match prepared {
            Ok(w) => {
                let resized = w.n_bits != w.old_bits;
                sh.doc.replace_bits(w.offset_bits, &w.data, w.n_bits, w.old_bits);
                // Bytes that moved are bytes the template read at an offset
                // they no longer sit at, so nothing worked out before still
                // stands. An in-place write keeps everything up to it.
                if resized {
                    self.changed();
                } else {
                    self.changed_at(w.offset_bits);
                }
                reply(Ok(WriteDto { offset_bits: w.offset_bits as f64, size_bits: w.n_bits as f64 }))
            }
            Err(e) => reply::<WriteDto>(Err(e)),
        }
    }

    // ----- searching -----

    /// Whether the search bar holds something that can be searched for, and
    /// what is wrong with it when not. Empty string means it is fine.
    /// `typing` suppresses the one complaint that is not a mistake yet: a hex
    /// byte with a single digit so far, which every valid needle passes
    /// through on its way to being typed.
    pub fn check_needle(&self, kind: &str, text: &str, typing: bool) -> String {
        match needle(kind, text, false) {
            Ok(_) => String::new(),
            Err(why) if typing && why == HEX_HALF_A_BYTE => String::new(),
            Err(why) => why,
        }
    }

    /// One step of a search, from byte `from`. The reply is the same tri-state
    /// as the rest: a step, the chunks it wants, or what is wrong with it.
    pub fn search_step(&mut self, space: u32, kind: &str, text: &str, fold: bool, backward: bool, from: f64) -> String {
        self.go(space);
        let sh = self.sm();
        let n = match needle(kind, text, fold) {
            Ok(n) => n,
            Err(why) => return reply::<StepDto>(Err(EvalError::Failed(why))),
        };
        if n.is_empty() {
            return reply(Ok(StepDto::End));
        }
        let s = if backward { Search::backward(n) } else { Search::forward(n) };
        reply(match s.step(&sh.doc, from as u64) {
            Step::Found { at, len } => Ok(StepDto::Found { at: at as f64, len: len as f64 }),
            Step::More { resume } => Ok(StepDto::More { resume: resume as f64 }),
            Step::End => Ok(StepDto::End),
            Step::Pending(m) => Err(EvalError::Pending(m)),
        })
    }

    /// Put `with` where a match was found. The caller carries on from the end
    /// of what was written: a replacement of a different length has moved
    /// every byte behind it.
    pub fn replace_at(&mut self, at: f64, len: f64, with: &[u8]) {
        self.live = 0;
        let sh = self.sm();
        search::replace(&mut sh.doc, at as u64, len as u64, with);
        self.changed();
    }

    /// Fold the edits that follow into one undo step.
    pub fn begin_batch(&mut self) {
        self.live = 0;
        let sh = self.sm();
        sh.doc.begin_batch();
    }

    pub fn end_batch(&mut self) {
        self.live = 0;
        let sh = self.sm();
        sh.doc.end_batch();
    }

    pub fn feed_chunk(&mut self, chunk: f64, data: &[u8]) {
        self.live = 0;
        let sh = self.sm();
        sh.doc.source_mut().insert(chunk as u64, data.into());
    }

    pub fn has_chunk(&self, space: u32, chunk: f64) -> bool {
        let sh = self.at(space);
        sh.doc.source().has(chunk as u64)
    }

    pub fn chunk_size(&self) -> u32 {
        let sh = &self.sheets[0];
        sh.doc.source().chunk_size() as u32
    }

    pub fn len_bytes(&self, space: u32) -> f64 {
        let sh = self.at(space);
        sh.doc.len_bytes() as f64
    }

    pub fn len_bits(&self, space: u32) -> f64 {
        let sh = self.at(space);
        sh.doc.len_bits() as f64
    }

    /// Fill `out` with document bytes from `at`. Returns the chunk indices that
    /// were not loaded (those bytes are zero). Empty list means the read is complete.
    pub fn read_bytes(&self, space: u32, at: f64, out: &mut [u8]) -> Vec<f64> {
        let sh = self.at(space);
        sh.doc.read_bytes(at as u64, out).into_iter().map(|m| m.chunk as f64).collect()
    }

    pub fn read_bits(&self, space: u32, at_bit: f64, n: f64, out: &mut [u8]) -> Vec<f64> {
        let sh = self.at(space);
        sh.doc.read_bits(at_bit as u64, n as u64, out).into_iter().map(|m| m.chunk as f64).collect()
    }

    pub fn overwrite_bytes(&mut self, at: f64, data: &[u8]) {
        self.changed_at(at as u64 * 8);
        self.live = 0;
        let sh = self.sm();
        sh.doc.overwrite_bytes(at as u64, data);
    }
    /// Overwrite that folds into the previous undo step.
    pub fn amend_overwrite_bytes(&mut self, at: f64, data: &[u8]) {
        self.changed_at(at as u64 * 8);
        self.live = 0;
        let sh = self.sm();
        sh.doc.amend_overwrite_bytes(at as u64, data);
    }
    pub fn insert_bytes(&mut self, at: f64, data: &[u8]) {
        self.changed();
        self.live = 0;
        let sh = self.sm();
        sh.doc.insert_bytes(at as u64, data);
    }
    pub fn delete_bytes(&mut self, at: f64, n: f64) {
        self.changed();
        self.live = 0;
        let sh = self.sm();
        sh.doc.delete_bytes(at as u64, n as u64);
    }
    pub fn overwrite_bits(&mut self, at_bit: f64, data: &[u8], n: f64) {
        self.changed_at(at_bit as u64);
        self.live = 0;
        let sh = self.sm();
        sh.doc.overwrite_bits(at_bit as u64, data, n as u64);
    }
    pub fn insert_bits(&mut self, at_bit: f64, data: &[u8], n: f64) {
        self.changed();
        self.live = 0;
        let sh = self.sm();
        sh.doc.insert_bits(at_bit as u64, data, n as u64);
    }
    pub fn delete_bits(&mut self, at_bit: f64, n: f64) {
        self.changed();
        self.live = 0;
        let sh = self.sm();
        sh.doc.delete_bits(at_bit as u64, n as u64);
    }

    /// Save plan as flat quads: kind (0 orig, 1 add, 2 materialize), doc_off, src_off, len.
    pub fn save_plan(&self) -> Vec<f64> {
        let sh = &self.sheets[0];
        sh.doc
            .save_plan()
            .iter()
            .flat_map(|r| {
                let k = match r.kind {
                    RunKind::Orig => 0.0,
                    RunKind::Add => 1.0,
                    RunKind::Materialize => 2.0,
                };
                [k, r.doc_off as f64, r.src_off as f64, r.len as f64]
            })
            .collect()
    }

    pub fn add_bytes(&self) -> Vec<u8> {
        let sh = &self.sheets[0];
        sh.doc.add_bytes().to_vec()
    }

    pub fn undo(&mut self) -> bool {
        self.changed();
        self.live = 0;
        let sh = self.sm();
        sh.doc.undo()
    }
    pub fn redo(&mut self) -> bool {
        self.changed();
        self.live = 0;
        let sh = self.sm();
        sh.doc.redo()
    }
    pub fn can_undo(&self, space: u32) -> bool {
        let sh = self.at(space);
        sh.doc.can_undo()
    }
    pub fn can_redo(&self, space: u32) -> bool {
        let sh = self.at(space);
        sh.doc.can_redo()
    }
    pub fn is_modified(&self, space: u32) -> bool {
        let sh = self.at(space);
        sh.doc.is_modified()
    }
    pub fn piece_count(&self, space: u32) -> u32 {
        let sh = self.at(space);
        sh.doc.piece_count() as u32
    }

    /// How the file reads as text: the encoding, whether that was a guess, and
    /// how many bytes of byte-order mark sit in front of the first line.
    /// `encoding` names one to use instead, or is empty to let the file decide.
    pub fn text_reading(&self, encoding: &str) -> String {
        let head = self.head(64);
        let r = named_encoding(encoding).map_or_else(|| textview::reading(&head), |s| textview::reading_as(s, &head));
        serde_json::to_string(&TextReadingDto {
            encoding: r.settled.name().to_string(),
            mark: r.mark as u32,
            guessed: r.guessed,
            unit: r.settled.unit() as u32,
        })
        .unwrap_or_default()
    }

    /// Lines starting at `from`, which must be where a line starts.
    pub fn text_window(&self, space: u32, encoding: &str, from: f64, want: u32) -> String {
        let sh = self.at(space);
        let head = self.head(64);
        let r = named_encoding(encoding).map_or_else(|| textview::reading(&head), |s| textview::reading_as(s, &head));
        let w = textview::window(&sh.doc, r, from as u64, want as usize);
        serde_json::to_string(&TextWindowDto {
            next: w.next as f64,
            missing: w.missing.iter().map(|m| m.chunk as f64).collect(),
            lines: w
                .lines
                .iter()
                .map(|l| TextLineDto {
                    at: l.at as f64,
                    len: l.len as f64,
                    ending: l.ending.name().to_string(),
                    text: l.text.clone(),
                    escapes: l.escapes.iter().flat_map(|(a, n)| [*a, *n]).collect(),
                    lossy: l.lossy,
                })
                .collect(),
        })
        .unwrap_or_default()
    }

    /// Where every line in `[from, to)` starts, `from` included, which must be
    /// where a line starts.
    ///
    /// Packed into one array of doubles rather than JSON: an index of a large
    /// file is hundreds of thousands of numbers, and spelling each of them out
    /// and parsing it back costs more than the scan does. The layout is
    /// `[next, lf, cr, crlf, missing count, ...missing chunks, ...starts]`.
    pub fn text_index(&self, space: u32, encoding: &str, from: f64, to: f64) -> Vec<f64> {
        let sh = self.at(space);
        let head = self.head(64);
        let r = named_encoding(encoding).map_or_else(|| textview::reading(&head), |s| textview::reading_as(s, &head));
        let idx = textview::text_index(&sh.doc, r, from as u64, to as u64);
        let mut out = Vec::with_capacity(5 + idx.missing.len() + idx.starts.len());
        out.push(idx.next as f64);
        out.push(idx.lf as f64);
        out.push(idx.cr as f64);
        out.push(idx.crlf as f64);
        out.push(idx.missing.len() as f64);
        out.extend(idx.missing.iter().map(|m| m.chunk as f64));
        out.extend(idx.starts.iter().map(|s| *s as f64));
        out
    }

    /// Where the line holding `at` starts, and where `lines` line starts back
    /// from there is. Both in one call, because scrolling text upwards wants
    /// the second and clicking in it wants the first.
    pub fn text_back(&self, space: u32, encoding: &str, at: f64, lines: u32) -> String {
        let sh = self.at(space);
        let head = self.head(64);
        let r = named_encoding(encoding).map_or_else(|| textview::reading(&head), |s| textview::reading_as(s, &head));
        let (start, missing) = textview::line_start(&sh.doc, r, at as u64);
        let (back, more) = textview::back(&sh.doc, r, at as u64, lines as usize);
        serde_json::to_string(&TextBackDto {
            start: start as f64,
            back: back as f64,
            missing: missing.iter().chain(more.iter()).map(|m| m.chunk as f64).collect(),
        })
        .unwrap_or_default()
    }

    /// The strings in the file from `from` onwards, for a file with no
    /// template and no encoding of its own.
    ///
    /// `encodings` names which readings to look for, comma separated, out of
    /// `ascii`, `utf16le` and `utf16be`; empty means all three. The reply is
    /// the usual tri-state: strings, or the chunks it needs before it can
    /// answer. `next` is where the caller carries on from, which is not the
    /// end of the last string when the scan stopped for want of hits.
    pub fn strings_scan(&self, space: u32, from: f64, want: u32, min_chars: u32, encodings: &str) -> String {
        use qubero_core::stringscan;
        let sh = self.at(space);
        let pick = |name: &str| encodings.is_empty() || encodings.split(',').any(|e| e.trim() == name);
        let opts = stringscan::Opts {
            min_chars: min_chars as usize,
            ascii: pick("ascii"),
            utf16le: pick("utf16le"),
            utf16be: pick("utf16be"),
        };
        let s = stringscan::scan(&sh.doc, from as u64, want as usize, opts);
        serde_json::to_string(&StringsScanDto {
            next: s.next as f64,
            missing: s.missing.iter().map(|m| m.chunk as f64).collect(),
            hits: s
                .hits
                .iter()
                .map(|h| StringHitDto {
                    at: h.at as f64,
                    len: h.len as f64,
                    enc: h.enc.name().to_string(),
                    chars: h.chars,
                    units: h.units,
                    text: h.text.clone(),
                    lone_surrogates: h.lone_surrogates,
                    terminator: h.term.map_or(0, |t| t.bytes()) as u32,
                    cut: h.cut,
                    prefix: h
                        .prefix
                        .iter()
                        .map(|p| StringPrefixDto {
                            kind: p.kind.name().to_string(),
                            at: p.at as f64,
                            bytes: p.raw.clone(),
                            value: p.value as f64,
                            counts: p.counts.name().to_string(),
                            with_terminator: p.with_terminator,
                            weak: p.weak,
                        })
                        .collect(),
                })
                .collect(),
        })
        .unwrap_or_default()
    }

    /// What a selected run of bytes says, read every way text can be read.
    ///
    /// Only a run of whole bytes lying together: a selection made over the bits
    /// in binary mode is not characters, which is the same reason the panel's
    /// byte-reversed rows only appear for whole bytes. `first` names the
    /// encoding to put at the front, which is whatever the text view is
    /// reading the file in.
    pub fn selection_text(&self, space: u32, at_byte: f64, len: f64, first: &str, page_a: &str, page_b: &str) -> String {
        use qubero_core::text::CodePage;
        let sh = self.at(space);
        let want = (len as u64).min(SELECTION_TEXT_LIMIT) as usize;
        let mut buf = vec![0u8; want];
        let missing = sh.doc.read_bytes(at_byte as u64, &mut buf);
        if !missing.is_empty() {
            return String::new();
        }
        let a = CodePage::by_name(page_a).unwrap_or(CodePage::Latin1);
        let b = CodePage::by_name(page_b).unwrap_or(CodePage::Cp437);
        let r = qubero_core::text::readings(&buf, named_encoding(first), a, b);
        serde_json::to_string(&SelectionTextDto {
            readings: r
                .agreed
                .iter()
                .map(|(who, text)| ReadingDto {
                    encodings: who.iter().map(|s| s.name().to_string()).collect(),
                    text: text.clone(),
                })
                .collect(),
            refused: r.refused.iter().map(|s| s.name().to_string()).collect(),
            read: want as f64,
            all: want as u64 >= len as u64,
        })
        .unwrap_or_default()
    }

    /// The same bytes written as a string literal in one language: what to
    /// paste into a parser being written against the file. Empty while the
    /// bytes are still being fetched, and cut to the same length the readings
    /// are, so the row says the same run the rows above it do.
    pub fn selection_literal(&self, space: u32, at_byte: f64, len: f64, lang: &str) -> String {
        use qubero_core::text::Lang;
        let sh = self.at(space);
        let want = (len as u64).min(SELECTION_TEXT_LIMIT) as usize;
        let mut buf = vec![0u8; want];
        let missing = sh.doc.read_bytes(at_byte as u64, &mut buf);
        if !missing.is_empty() {
            return String::new();
        }
        qubero_core::text::literal(Lang::by_name(lang).unwrap_or(Lang::C), &buf)
    }

    /// The first `n` bytes, for questions that only the front of the file
    /// answers. A chunk that is not here yet reads as zeros, which settles the
    /// encoding as Latin-1 until it arrives.
    fn head(&self, n: u64) -> Vec<u8> {
        let sh = &self.sheets[0];
        let n = n.min(sh.doc.len_bytes());
        let mut out = vec![0u8; n as usize];
        sh.doc.read_bytes(0, &mut out);
        out
    }
}

/// Text typed into the text view, as the bytes it is in the file's encoding.
///
/// The answer is a refusal or a run of bytes, never both and never a guess: an
/// encoding that has no room for a character says which character, since the
/// reader is owed the difference between "this file cannot hold that" and
/// "this file was read as the wrong thing". A file read as CP437 that is
/// really Latin-1 will take a character the other would refuse, and the
/// refusal is where that is found out.
#[wasm_bindgen]
pub fn text_encode(encoding: &str, settled: &str, text: &str) -> String {
    use qubero_core::text::{encode_settled, Settled};
    let enc = named_encoding(encoding).or_else(|| named_encoding(settled)).unwrap_or(Settled::Utf8);
    match encode_settled(enc, text) {
        Ok(bytes) => serde_json::to_string(&TextEncodeDto { bytes, refused: String::new() }),
        Err(c) => serde_json::to_string(&TextEncodeDto { bytes: Vec::new(), refused: c.to_string() }),
    }
    .unwrap_or_default()
}

/// How much of a selection is read as text. Long enough to hold a paragraph,
/// which is what someone selecting a stretch to read is after; past it the
/// rows say how much they are showing.
const SELECTION_TEXT_LIMIT: u64 = 4096;

#[derive(Serialize)]
struct SelectionTextDto {
    readings: Vec<ReadingDto>,
    /// Encodings the bytes do not fit, named rather than shown.
    refused: Vec<String>,
    /// Bytes actually read, which is short of the selection when it is long.
    read: f64,
    all: bool,
}

#[derive(Serialize)]
struct ReadingDto {
    /// The encodings that agree on this reading, the likeliest first.
    encodings: Vec<String>,
    text: String,
}

#[derive(Serialize)]
struct TextEncodeDto {
    bytes: Vec<u8>,
    /// The character the encoding has no room for, or empty.
    refused: String,
}

/// An encoding named across the boundary, or nothing to let the file decide.
fn named_encoding(name: &str) -> Option<qubero_core::text::Settled> {
    use qubero_core::text::{CodePage, Settled};
    use qubero_core::Endian;
    Some(match name {
        "UTF-8" => Settled::Utf8,
        "ASCII" => Settled::Ascii,
        "UTF-16 LE" => Settled::Utf16(Endian::Little),
        "UTF-16 BE" => Settled::Utf16(Endian::Big),
        // Every single-byte page is named the same way it is shown.
        _ => Settled::SingleByte(CodePage::by_name(name)?),
    })
}

#[derive(Serialize)]
struct TextReadingDto {
    encoding: String,
    mark: u32,
    guessed: bool,
    unit: u32,
}

#[derive(Serialize)]
struct TextWindowDto {
    lines: Vec<TextLineDto>,
    missing: Vec<f64>,
    next: f64,
}

#[derive(Serialize)]
struct TextLineDto {
    at: f64,
    len: f64,
    ending: String,
    text: String,
    /// Escape sequences as flat pairs of character index and length.
    escapes: Vec<u32>,
    lossy: bool,
}

#[derive(Serialize)]
struct StringsScanDto {
    hits: Vec<StringHitDto>,
    missing: Vec<f64>,
    next: f64,
}

#[derive(Serialize)]
struct StringHitDto {
    at: f64,
    len: f64,
    enc: String,
    chars: u32,
    units: u32,
    text: String,
    lone_surrogates: bool,
    /// Bytes of zero after the text: none, one, or two after a wide string.
    terminator: u32,
    cut: bool,
    prefix: Vec<StringPrefixDto>,
}

#[derive(Serialize)]
struct StringPrefixDto {
    kind: String,
    at: f64,
    bytes: Vec<u8>,
    value: f64,
    counts: String,
    with_terminator: bool,
    weak: bool,
}

#[derive(Serialize)]
struct TextBackDto {
    start: f64,
    back: f64,
    missing: Vec<f64>,
}

/// What a text file turned out to be a dump of, if anything.
///
/// Standalone rather than a method on [`Editor`], because a dump is text the
/// host already has in hand and the file it describes is not the file that is
/// open. The host reads this, offers what it says, and opens the recovered
/// bytes as a document of their own.
#[derive(Serialize, Default)]
struct DumpScanDto {
    /// Empty when the layout matches no tool that is recognised here, which
    /// changes nothing about how it was read.
    tool: String,
    /// "regular" when the lines were regular enough to be read by arithmetic.
    tier: String,
    /// The first address described and the end of the last.
    from: f64,
    to: f64,
    /// Bytes the dump actually spells out, which is fewer than `to - from`
    /// when it skips stretches.
    covered: f64,
    address_base: String,
    address_digits: u32,
    bytes_per_line: u32,
    group: u32,
    upper: bool,
    reversed_groups: bool,
    characters: String,
    /// What the dump did not settle, which was taken as the usual thing.
    assumed: Vec<String>,
    /// Stretches of the described file the dump covers, as start/end pairs.
    extents: Vec<f64>,
    /// Stretches inside that span nobody described, as start/end pairs.
    holes: Vec<f64>,
    /// Paths or file names the dump gave.
    names: Vec<String>,
    /// A length the dump stated, or -1. Not the same as what it went on to
    /// write, which is the point of keeping it.
    stated_length: f64,
    /// Command lines a transcript kept.
    commands: Vec<String>,
    /// Lines that were not part of the dump.
    skipped_lines: u32,
    /// Bytes whose two spellings disagree, capped: a dump read the wrong way
    /// disagrees everywhere, and the first few say so as well as all of them.
    conflicts: Vec<DumpConflictDto>,
}

#[derive(Serialize)]
struct DumpConflictDto {
    at: f64,
    wrote: String,
    digits: u8,
}

/// Read `text` as a hex dump and say what it holds. Returns "" when it is not
/// one, or when it is too big to read in one go.
#[wasm_bindgen]
pub fn dump_scan(text: &[u8]) -> String {
    if text.len() > hexdump::LIMIT {
        return String::new();
    }
    let Some(dump) = hexdump::read(text, 0) else { return String::new() };
    let Some((from, to)) = dump.span() else { return String::new() };
    let l = &dump.layout;
    let mut dto = DumpScanDto {
        tool: l.looks_like().unwrap_or("").to_string(),
        tier: match dump.tier() {
            hexdump::Tier::Regular => "regular",
            hexdump::Tier::Irregular => "irregular",
        }
        .to_string(),
        from: from as f64,
        to: to as f64,
        covered: dump.byte_count() as f64,
        address_base: l.address.as_ref().map_or("", |a| a.base.name()).to_string(),
        address_digits: l.address.as_ref().and_then(|a| a.digits).unwrap_or(0) as u32,
        bytes_per_line: l.bytes_per_line as u32,
        group: l.group as u32,
        upper: l.upper,
        reversed_groups: l.order == hexdump::layout::Order::ReversedInGroup,
        characters: l.text.as_ref().map_or(String::new(), |t| t.glyphs.name().to_string()),
        assumed: l.assumed.iter().map(|a| format!("{a:?}")).collect(),
        stated_length: -1.0,
        skipped_lines: dump.skipped.len() as u32,
        ..Default::default()
    };
    let mut at = from;
    for e in dump.extents() {
        dto.extents.push(e.at as f64);
        dto.extents.push(e.end() as f64);
        if e.at > at {
            dto.holes.push(at as f64);
            dto.holes.push(e.at as f64);
        }
        at = e.end().max(at);
    }
    for n in &dump.notes {
        match n {
            hexdump::Note::Named(s) => dto.names.push(s.clone()),
            hexdump::Note::Length(v) => dto.stated_length = *v as f64,
            hexdump::Note::Command(s) => dto.commands.push(s.clone()),
        }
    }
    dto.conflicts = dump
        .conflicts()
        .into_iter()
        .take(64)
        .map(|(at, wrote, digits)| DumpConflictDto { at: at as f64, wrote: wrote.to_string(), digits })
        .collect();
    serde_json::to_string(&dto).unwrap_or_default()
}

/// The bytes a dump describes, from its first address to the end of its last.
/// A stretch the dump skipped reads as zeros; `dump_scan` says where those
/// are, so a reader is never left to guess which zeros were written down.
#[wasm_bindgen]
pub fn dump_bytes(text: &[u8]) -> Vec<u8> {
    let Some(source) = hexdump::source::DumpSource::new(text.to_vec()) else { return Vec::new() };
    let mut out = vec![0u8; source.len_bytes() as usize];
    source.read_bytes(0, &mut out);
    out
}
