//! wasm-bindgen surface over `qubero-core`.
//!
//! Offsets cross the boundary as `f64` (exact up to 2^53, far past any file size)
//! to avoid BigInt friction on the JS side.

use qubero_core::codec::{inflate, Codec, Step as MapStep, StepKind};
use qubero_core::eval::{leap_seconds, Census, CensusState, CensusWalk, Diagram, Explain, Graph, KindWalk, Moment, Origin, SpaceId, Tab, TimeNote, NO_PARENT};
use qubero_core::template::Zone;
use qubero_core::hexdump;
use qubero_core::hexpat::Includes as _;
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
/// own, and it has its own byte-class scan, so two tabs never read over each
/// other's working.
///
/// A stream whose template came from looking at its bytes has its own
/// evaluator too. One read by the template it declared has none: its fields
/// name what is outside the stream, so they are read in the file's reading,
/// under the stream, and handed back as the tab's own. See `Editor::tab`.
struct Sheet {
    doc: Document<ChunkStore>,
    /// Which sheet's reading opened this space: 0, the file, or a stream whose
    /// bytes were recognised and so has a reading of its own. Never a stream
    /// read where it was declared, which has none: a stream opened from a tab
    /// of one of those is opened in the reading that tab is read in. 0 for the
    /// file itself.
    home: usize,
    /// Which `Decoded` or joining node of the home sheet's reading this space
    /// was unpacked from. Empty for space 0, which was unpacked from nothing.
    origin: Vec<usize>,
    /// The core's number for the same space in the home sheet's reading, which
    /// is what the trace behind the cursor link is asked by. Zero for the
    /// file. The core renumbers from 1 whenever a reading is thrown away, so
    /// this is set again every time the stream is opened again; a tab's own
    /// number never changes.
    core_space: SpaceId,
    eval: Option<Evaluator>,
    /// For a stream read where it was declared, where its contents are in the
    /// home sheet's reading. `eval` is None for those.
    view: Option<Vec<usize>>,
    /// What such a stream declares it holds, which the diagram is drawn from.
    read_as: Option<qubero_core::template::Template>,
    /// The stream was unpacked from its home as it was before an edit or a
    /// change of template, and has not been opened again since. The bytes and
    /// reading here are the old ones, kept so a tab still waiting on chunks of
    /// the file has a length to show; `Editor::ensure_open` opens the stream
    /// again before anything of the tab is answered from them.
    stale: bool,
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
    /// The Diagram view's count of the file's fields against the format's
    /// boxes, run a step at a time and thrown away with the walk above: its
    /// paths are through bytes and a template that may have changed.
    census: Option<CensusWalk>,
    /// What the last `.ksy` conversion had to say, as the JSON the panel
    /// shows. Empty for a template that did not come from a `.ksy`.
    ksy_report: String,
    /// The same for the last `.hexpat`. The two are kept apart rather than
    /// sharing one slot with a tag, because each panel asks its own question
    /// and an empty answer is how it knows the template is not its own.
    hexpat_report: String,
}

impl Sheet {
    /// A reading of one of the core's spaces.
    ///
    /// The bytes are all here already, so the store keeps every chunk: a space
    /// has no file behind it to fetch a missing one back from. The template is
    /// the one the core settled on, which is the stream's own or, where that
    /// said only bytes, whatever the unpacked bytes were recognised as. Only
    /// the second is read here; the first is read in the file's reading.
    fn from_space(space: &qubero_core::eval::Space, home: usize, origin: Vec<usize>) -> Sheet {
        let template = space.read_as().clone();
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
        sheet.home = home;
        sheet.core_space = space.id;
        sheet.template = template.name.clone();
        match space.view() {
            Some(view) => {
                sheet.view = Some(view.root.clone());
                sheet.read_as = Some(template);
            }
            None => {
                let mut ev = Evaluator::new(template);
                ev.set_slice(Some(WORK_SLICE));
                sheet.eval = Some(ev);
            }
        }
        sheet
    }

    /// A space that is no longer there: the stream it came from was edited away
    /// or would not open a second time. It holds no bytes rather than the
    /// file's, because a tab named after a stream must never quietly show
    /// something else.
    fn empty(home: usize, origin: Vec<usize>) -> Sheet {
        let mut sheet = Sheet::new(ChunkStore::new(0, SPACE_CHUNK, 1), origin);
        sheet.home = home;
        sheet
    }

    fn new(store: ChunkStore, origin: Vec<usize>) -> Sheet {
        Sheet {
            doc: Document::new(store),
            home: 0,
            origin,
            core_space: 0,
            eval: None,
            view: None,
            read_as: None,
            stale: false,
            disasm: None,
            bpf: None,
            bpf_complete: false,
            ne: None,
            template: String::new(),
            scan: None,
            focus: None,
            kinds: None,
            census: None,
            ksy_report: String::new(),
            hexpat_report: String::new(),
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
    /// What is wrong with this value, when something is. Absent for the
    /// overwhelming majority of fields, which hold what their format allows.
    #[serde(skip_serializing_if = "Option::is_none")]
    problem: Option<ProblemDto>,
    /// Wrong values found under this node so far: invalid, then undefined.
    /// Counted over the children already read, never by reading more, so a
    /// collapsed run's count is a count so far.
    problems_within: [u32; 2],
    child_count: f64,
    /// What one child is called, for counting them: empty when they are items.
    #[serde(skip_serializing_if = "String::is_empty")]
    unit: String,
    composite: bool,
    /// True when the children are a list's elements, whichever of the five
    /// kinds of list it is. See `NodeInfo::list`.
    list: bool,
    /// True when `write_node` accepts text for this field.
    editable: bool,
    /// Bytes of the field the value occupies; less than the size for padded
    /// and terminated text.
    value_bytes: f64,
    /// Where the value starts: past a byte-order mark, if the field has one.
    value_offset_bits: f64,
    /// Where the bytes were read, for a field written in one place and read in
    /// another. Null for a field that is where it is written.
    read_at: Option<f64>,
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
    /// True for a field read inside a stream joined from several runs, which
    /// is not unpacked from one and has no document of its own to open.
    joined: bool,
    /// True when the file did not write this field: the condition on an
    /// optional one came to nothing. Told apart from a size of zero, which
    /// several kinds of field that are there also have.
    absent: bool,
    /// What the format's own description says this field is, where the
    /// template carries it. Null for a field nobody wrote prose for, which is
    /// most of them.
    #[serde(skip_serializing_if = "Option::is_none")]
    doc: Option<String>,
    /// What a structure of a few fields reads as on one line, the same reading
    /// the annotation column puts beside the bytes. Null for a leaf, for a
    /// list, and for a structure too long to read on a line.
    line: Option<String>,
    /// True when the template says this field reads as a table: interleaved
    /// samples, a list of records. `table_shape` says what of.
    table: bool,
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
    /// What is wrong with this element's value, when something is.
    #[serde(skip_serializing_if = "Option::is_none")]
    problem: Option<ProblemDto>,
    /// Wrong values found under this element so far: invalid, then undefined.
    problems_within: [u32; 2],
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

/// One step of reading a Parquet page: the codec, a list of levels, or the
/// encoding the values are in.
#[derive(Serialize)]
struct PageStepDto {
    /// The codec's name, the encoding's name, or which list of levels.
    what: String,
    in_bytes: f64,
    /// Zero for a step that produced values rather than bytes, which says how
    /// many of them in `note` instead.
    out_bytes: f64,
    note: String,
    /// Set when the step was not done at all: a v2 page that says
    /// `is_compressed` is false names its column's codec and never ran it.
    skipped: bool,
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

/// One step of unpacking a GWF vector: gzip, zero suppression, differencing,
/// or putting complex parts back in pairs.
#[derive(Serialize)]
struct VectorStepDto {
    what: String,
    in_bytes: f64,
    out_bytes: f64,
}

/// One step of working out a GRIB message's values: a short label a problem
/// can name it by, what it came to, and how many numbers it was done to.
#[derive(Serialize)]
struct GribStepDto {
    label: String,
    what: String,
    count: f64,
}

/// The GRIB value under the cursor.
#[derive(Serialize)]
struct GribValueDto {
    /// Where it is in the message's run of values, from 0.
    index: f64,
    /// "values" for `values[index]`, "group" for `groups[group].values[position]`,
    /// "first" for `first_values[index]`.
    place: &'static str,
    /// Which group and where in it, and the number that field holds, for
    /// "group". Zero otherwise.
    group: f64,
    position: f64,
    written: f64,
    /// What it is worth, and the whole packed integer it was worked out from.
    value: String,
    packed: f64,
}

/// One descriptor of a BUFR message expanded through Table D, as deep as the
/// sequences it came out of.
#[derive(Serialize)]
struct BufrDescriptorDto {
    code: f64,
    depth: f64,
    name: String,
}

/// One value of a BUFR subset, as the panel lists it.
#[derive(Serialize)]
struct BufrValueDto {
    code: f64,
    /// "element" | "count" | "quality" | "associated" | "reference" | "local" | "characters" | "marker"
    role: &'static str,
    name: String,
    text: String,
    unit: String,
    missing: bool,
    /// The element an associated field, a marker, quality information or a
    /// new reference value is about, as its descriptor and name. Empty
    /// otherwise.
    about: String,
}

/// The BUFR value under the cursor, with what it was read from.
#[derive(Serialize)]
struct BufrCursorDto {
    /// Its place in `values`, or -1 where it is not among those listed.
    index: f64,
    value: BufrValueDto,
    /// Where its bits start, from section 4's first bit, and how wide it is.
    bit: f64,
    width: f64,
    scale: f64,
    reference: f64,
    numeric: bool,
    /// Null where there is no packed number: text, or a missing value.
    packed: Option<f64>,
    /// A compressed value's smallest packed number and difference width, or
    /// null for an uncompressed message.
    base: Option<f64>,
    increment_width: Option<f64>,
    /// Every subset's value, the first few dozen, for a compressed message.
    /// An empty string is a missing value.
    across: Vec<String>,
}

fn bufr_value_dto(v: qubero_core::formats::bufr_data::PanelValue) -> BufrValueDto {
    BufrValueDto {
        code: f64::from(v.code),
        role: v.role,
        name: v.name,
        text: v.text,
        unit: v.unit,
        missing: v.missing,
        about: v.about.unwrap_or_default(),
    }
}

/// One Steim frame of a miniSEED record: how many differences its codes named,
/// and how many of them became samples.
#[derive(Serialize)]
struct MseedFrameDto {
    held: f64,
    used: f64,
}

/// What a type permits, one shape per panel. `kind` says which, and each
/// variant carries only the fields its own panel reads.
///
/// It was one flat struct with every panel's fields side by side, prefixed so
/// they did not collide, and every panel added twenty more that every other
/// answer then carried as zeros. Tagged, a panel's fields are its own, and
/// the host's union type on `kind` stops a panel reading another's.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum ExplainDto {
    /// The bytes the format requires, and the bytes that are there.
    Magic { expected: Vec<u8>, actual: Vec<u8> },
    Enum {
        /// The type's own name.
        name: String,
        /// Every value it names, and the one in the file.
        cases: Vec<CaseDto>,
        current: f64,
        /// What the value in the file is called, where that name comes from a
        /// counted run rather than from `cases`. Empty when it has no name.
        named: String,
        /// Whether its numbers are read in hex.
        hex: bool,
    },
    Flags {
        name: String,
        /// The field's value, and one entry per bit of it from bit 0 up.
        current: f64,
        bits: Vec<BitDto>,
    },
    /// Which layout the float is, how many bits wide, and those bits in value
    /// order, written in hex because a 64-bit pattern does not survive a JSON
    /// number.
    Float { format: String, width: f64, pattern: String },
    /// A fixed-point number: its bits in hex, in value order, and where the
    /// binary point falls.
    Fixed { bits: f64, frac: f64, signed: bool, pattern: String },
    Quant {
        /// The block layout, as ggml's own struct is named, and how many bits
        /// one weight is worth.
        name: String,
        width: f64,
        /// The block's shared scale, and what it pairs with the scale, named
        /// as the file names it. Empty name where the layout has no second
        /// number.
        scale: f64,
        second_name: String,
        second: f64,
        /// Whether that second number is taken away rather than added, and
        /// whether it is multiplied by the group's own minimum first. Together
        /// with the group scales these say how a stored weight becomes a real
        /// one.
        second_subtract: bool,
        second_per_group: bool,
        /// Where the block starts, so a weight's bits can be found from the
        /// offset it carries.
        block_bits: f64,
        /// The scale the block keeps for each run of weights, where it keeps
        /// them, and how many weights one run covers. Empty for a block with
        /// one scale for all of them.
        groups: Vec<GroupDto>,
        group_weights: f64,
        /// Taken off the packed value to get the stored one, and whether that
        /// value is read signed instead of biased.
        bias: f64,
        signed: bool,
        /// Every weight the block stands for, in the order the tensor reads
        /// them, and which one the cursor is inside (-1 for none).
        weights: Vec<WeightDto>,
        at: f64,
    },
    Xref {
        /// The three widths from `/W`, and the PNG predictor where there was
        /// one, which is -1 where there was not.
        widths: Vec<f64>,
        predictor: f64,
        /// How many bytes the rows are in the file, and how many they came to
        /// once decompressed.
        packed: f64,
        decoded: f64,
        /// How many rows of each kind there are, over the whole table rather
        /// than over the ones listed.
        free: f64,
        in_file: f64,
        in_stream: f64,
        unknown: f64,
        /// The rows, and how many there are altogether. A table with more
        /// than `rows` holds says so with `total`.
        rows: Vec<XrefRowDto>,
        total: f64,
        /// Why there are no rows, where there are none. Empty otherwise.
        problem: String,
    },
    Objstm {
        /// How many bytes the objects are in the file, and how many they came
        /// to once decompressed.
        packed: f64,
        decoded: f64,
        /// The object number in `/Extends`, which is the object stream this
        /// one continues, or -1 where it continues none.
        extends: f64,
        /// The objects, and how many there are altogether.
        objects: Vec<ObjStmObjectDto>,
        total: f64,
        /// Why the stream would not open. Empty otherwise.
        problem: String,
    },
    Sqliterow {
        /// How many bytes the row claims, how many the chain reached, and how
        /// many of them stayed on the row's own page. A row that is whole has
        /// the first two equal.
        declared: f64,
        found: f64,
        on_page: f64,
        /// The overflow pages in the order the chain names them, and how many
        /// there are when that is more than the few listed.
        pages: Vec<f64>,
        chain: f64,
        /// The columns, and how many there are altogether.
        columns: Vec<SqliteColumnDto>,
        total_columns: f64,
        problem: String,
    },
    Chunk {
        /// How many bytes the chunk is in the file, and how many its elements
        /// came to once the filters were undone.
        packed: f64,
        decoded: f64,
        /// Each filter, in the order it was undone.
        steps: Vec<ChunkStepDto>,
        /// What one element is called, the first few elements, and how many
        /// there are altogether.
        element_type: String,
        values: Vec<String>,
        total: f64,
        problem: String,
    },
    Vector {
        /// The GWF vector's `compress` field as written, and whether its words
        /// were packed little-endian.
        compress: f64,
        little_endian: bool,
        /// How many numbers `nData` says the vector holds.
        declared: f64,
        /// How many bytes the packed run is in the file, and how many the
        /// numbers came to once unpacked.
        packed: f64,
        decoded: f64,
        /// Each step, in the order it was done.
        steps: Vec<VectorStepDto>,
        /// What one number is called, the first few, and how many came out.
        element_type: String,
        values: Vec<String>,
        total: f64,
        problem: String,
    },
    Grib {
        /// The data representation template, 0, 2 or 3, and for 3 whether the
        /// differencing is first or second order.
        template: f64,
        spatial_order: f64,
        /// R as text, and E and D, for the formula a value came out of.
        reference: String,
        binary_scale: f64,
        decimal_scale: f64,
        /// The overall minimum of the differences, or null without spatial
        /// differencing.
        minimum: Option<f64>,
        /// How many values section 5 says there are, and how many bytes
        /// section 7's data is.
        declared: f64,
        packed: f64,
        /// Every step, in the order it was done.
        steps: Vec<GribStepDto>,
        /// The first values, and how many came out.
        values: Vec<String>,
        total: f64,
        /// The value the cursor is on, or null where it is not on one.
        at: Option<GribValueDto>,
        problem: String,
    },
    /// A BUFR message's section 4 read through the tables. See
    /// `qubero_core::formats::bufr_data::Panel`.
    Bufr {
        edition: f64,
        /// The version section 1 names, and the version that was used.
        master_table_version: f64,
        tables_version: f64,
        subsets: f64,
        compressed: bool,
        steps: Vec<String>,
        /// Section 3's descriptors expanded through Table D, the first
        /// thousand or so, and how many there are.
        descriptors: Vec<BufrDescriptorDto>,
        descriptors_total: f64,
        /// Which subset `values` belong to, counted from 0, where in that
        /// subset's values the list starts, and how many values it has.
        subset: f64,
        values: Vec<BufrValueDto>,
        values_start: f64,
        values_total: f64,
        /// The value under the cursor, or null where the cursor is on none.
        cursor: Option<BufrCursorDto>,
        problem: String,
    },
    Page {
        /// How many bytes the payload is in the file, and how many its values
        /// came to once the codec was undone.
        packed: f64,
        decoded: f64,
        /// Every step, in the order it was done.
        steps: Vec<PageStepDto>,
        /// What one value is called, the first few of them, and how many there
        /// are altogether.
        element_type: String,
        values: Vec<String>,
        total: f64,
        problem: String,
    },
    Samples {
        /// A miniSEED record's encoding, by name and number, and whether its
        /// data was laid out big-endian.
        encoding: String,
        encoding_number: f64,
        big_endian: bool,
        /// How many samples the header gives, and how many bytes of data they
        /// were decoded from.
        declared: f64,
        bytes: f64,
        /// The Steim steps, where the record is Steim. `steim` says whether it
        /// is; the constants are null otherwise, and the first difference is
        /// null too for a record with no samples.
        steim: bool,
        x0: Option<f64>,
        xn: Option<f64>,
        first_difference: Option<f64>,
        /// What each frame held and gave, the first few hundred of them, how
        /// many frames were walked, and how many the data has room for.
        frames: Vec<MseedFrameDto>,
        frames_walked: f64,
        frames_in_record: f64,
        /// The rule a gain-ranged encoding is decoded by. Empty otherwise.
        rule: String,
        /// The first few, the last, and how many were decoded.
        values: Vec<String>,
        last: String,
        total: f64,
        /// Whether the last sample equals the reverse integration constant, or
        /// null where there is no check to make.
        check: Option<bool>,
        problem: String,
    },
    Tile {
        /// Which tile of a FITS compressed image, counted from 0 as its row
        /// is, of how many (null for more than a u64 counts); where it starts
        /// in the image, from 0 along each axis with the first axis first; how
        /// many pixels along each; and the image's shape.
        index: f64,
        count: Option<f64>,
        start: Vec<f64>,
        shape: Vec<f64>,
        image_shape: Vec<f64>,
        /// `ZCMPTYPE`, and the column the bytes were read from, or empty.
        algorithm: String,
        column: String,
        /// Bytes in the heap, and bytes once decompressed.
        packed: f64,
        decoded: f64,
        /// Each step in order. `skipped` is never set.
        steps: Vec<PageStepDto>,
        /// The first pixels, how many were decoded, how many the tile has
        /// (null for more than a u64 counts), and what one pixel is.
        values: Vec<String>,
        total: f64,
        pixels: Option<f64>,
        element_type: String,
        problem: String,
    },
    /// The type has nothing to add.
    Plain,
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
    /// The same field named by every step the file stores on the way to it,
    /// where an encoding's own steps make that differ. Null otherwise.
    stored: Option<String>,
    /// Where it is, so the reader can go there. Empty for a bit this field
    /// points at rather than a field it came from.
    path: Vec<f64>,
    /// What it says, in brief. Empty when it could not be read.
    value: String,
    /// For "points": the bit this field's value points at.
    target_bits: Option<f64>,
}

/// The part of a joined stream a field starts in. See
/// [`qubero_core::eval::PartHit`].
#[derive(Serialize)]
struct StitchedPartDto {
    /// Which part, from 0, and how many there are.
    index: f64,
    parts: f64,
    /// The run the part is, as a path to go to and as a reader names it, and
    /// as every stored step names it where that differs (null otherwise).
    path: Vec<f64>,
    label: String,
    stored: Option<String>,
    /// The field's first byte inside what the part gives, and how much that is.
    in_part: f64,
    part_len: f64,
    /// Where the run starts, in the space it is a field of: 0 is the file.
    run_offset_bits: f64,
    run_space: f64,
    /// True when the run was unpacked to give the part.
    packed: bool,
    /// For a BGZF block: the two halves of the byte's virtual offset, the
    /// block's place in the file and the byte in what it unpacks to.
    block_offset: Option<f64>,
    in_block: Option<f64>,
}

fn stitched_part_dto(h: qubero_core::eval::PartHit) -> StitchedPartDto {
    StitchedPartDto {
        index: h.index as f64,
        parts: h.parts as f64,
        path: h.path.iter().map(|&x| x as f64).collect(),
        label: h.label,
        stored: h.stored,
        in_part: h.in_part as f64,
        part_len: h.part_len as f64,
        run_offset_bits: h.run_offset_bits as f64,
        run_space: h.run_space as f64,
        packed: h.packed,
        // Past 2^53 a number stops being exact in JavaScript, and a virtual
        // offset reaches that at a block eight petabytes into the file. The
        // halves are what a reader checks against an index anyway.
        block_offset: h.virtual_offset.map(|v| (v >> 16) as f64),
        in_block: h.virtual_offset.map(|v| (v & 0xffff) as f64),
    }
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

/// What table a field reads as, with the template's expressions worked out in
/// the file at hand. See [`qubero_core::eval::TableShapeInfo`].
#[derive(Serialize)]
struct TableShapeDto {
    /// Elements per row. Null when one element is one row, and null as well
    /// when the template said how many but this file does not answer.
    columns: Option<f64>,
    /// What the columns are called, used when there are exactly this many.
    names: Vec<String>,
    /// The units the columns are measured in, as UCUM codes, parallel to
    /// `names`. An empty string for a column with no unit.
    units: Vec<String>,
    /// What to call a column that `names` does not reach: "channel" gives
    /// "channel 1", "channel 2". Null when the format has no word for one.
    column_word: Option<String>,
    /// What one row is: "sample", "record". Null when the format has no word.
    row_word: Option<String>,
    /// Rows per second, when the rows are spaced in time. Null otherwise.
    rate: Option<f64>,
    /// The fields that describe the table, to be shown above it with links to
    /// where they are stored.
    facts: Vec<TableFactDto>,
    /// Where a row's cells are, for a table whose rows are nodes rather than a
    /// run of values. Null for every table a template declares, where a row is
    /// `columns` values and a reader works the rest out from the count.
    cells: Option<CellsDto>,
}

/// Where a table's cells are, for a table whose rows are nodes rather than a
/// run of values. See [`qubero_core::template::Cells`].
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum CellsDto {
    /// Named nodes inside named rows: a pickled list of records, whose keys
    /// are written beside its values.
    Named {
        /// The type a node under the field has to be to be a row.
        row: String,
        /// The type a node inside a row has to be to be a cell, named by its
        /// column.
        cell: String,
        /// The field inside a cell holding what it is worth, for a cell that
        /// is a name and a value. Null when the cell is the value.
        value: Option<String>,
    },
    /// Cells the core works out, a window of rows at a time, which is a pandas
    /// frame. `rows` is how many there are; `Editor::pickle_cells` reads them.
    Computed { rows: f64 },
}

/// One cell of a table the core works out. `text` is empty and `kind` is
/// "absent" for a cell the file has no value for, which is the same nothing a
/// record without one of the table's keys shows.
#[derive(Serialize)]
struct FrameCellDto {
    text: String,
    kind: &'static str,
}

/// One field that describes a table: what it is called, where it is, and what
/// it says.
#[derive(Serialize)]
struct TableFactDto {
    /// The field as the reader would name it: `body.sample_rate`.
    label: String,
    /// Where it is, so the reader can go there. Empty when it is nowhere this
    /// reading can point at.
    path: Vec<f64>,
    /// What it says, in brief. Empty when it could not be read.
    value: String,
}

/// The moment a field means, once the template's epoch has been applied to the
/// number in it. The number itself is untouched and stays on the value row.
#[derive(Serialize)]
struct TimeDto {
    /// "at" | "leap" | "unset" | "impossible". `leap` is an instant inside a
    /// leap second, second 60 of a minute. `unset` is the value a format writes
    /// when it has no time to record, and `impossible` is a number that names
    /// no moment: a year outside 1 to 9999, or a packed date that is not a
    /// date.
    state: &'static str,
    /// Seconds from 1970-01-01T00:00:00Z, negative before it. Only for "at" and
    /// "leap"; for "leap" it is second 59 of the minute, and the moment is the
    /// second after it, which a Unix count has no number for.
    ///
    /// Well inside what an f64 holds exactly: the core refuses anything outside
    /// year 1 to year 9999, which is at most 2.5e11.
    unix_seconds: Option<f64>,
    /// The sub-second part, in nanoseconds, always 0 to 999,999,999 and never
    /// negative. Only for "at" and "leap".
    nanos: Option<f64>,
    /// "past_leap_second_table" | "before_leap_seconds" | null. What has to be
    /// shown beside a moment that is right as far as it goes: a count on a
    /// clock with leap seconds that falls after the last day the table of them
    /// vouches for, or before 1972, when UTC had none.
    note: Option<&'static str>,
    /// The first day the leap-second table does not vouch for, as seconds from
    /// 1970-01-01T00:00:00Z. Only with the first of those notes, so the words
    /// can name the day.
    leap_table_expires: Option<f64>,
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
    /// The expression as the template writes it, with a field reached through
    /// an encoding's own steps named the way the format names it.
    written: String,
    /// The expression exactly as the template writes it, where that differs
    /// from `written`. Null otherwise.
    template: Option<String>,
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

/// One row of a diagram box: one field of a type, or one case of a switch.
#[derive(Serialize)]
struct DiagramRowDto {
    name: String,
    /// The type as the listing's type column writes it.
    type_text: String,
    /// How long the field runs, or the expression that decides it. Empty when
    /// only reading a file settles it.
    size_text: String,
    /// Where it starts inside its own type, or the address it reads its
    /// contents at. Empty when neither is fixed by the template.
    pos_text: String,
    /// True when the field holds a list of elements. See `diagram::Row::list`.
    list: bool,
    /// The word the listing gives a field of this type, so the same field is
    /// the same colour in both: "uint", "str", "magic", "composite" and the
    /// rest. See `qubero_core::eval::value_kind`.
    kind: &'static str,
}

/// One type of the format, and its fields.
#[derive(Serialize)]
struct DiagramBoxDto {
    /// What the type is called: the structure's own name in the template.
    name: String,
    /// Where the walk first reached it, for the reader who wants to know how
    /// they would get there.
    path: String,
    /// What tells this type from every other. `diagram_census` counts the open
    /// file's nodes by the same key, so a view can say how many of this box the
    /// file holds.
    key: String,
    /// "seq" | "instances" | "switch"
    kind: &'static str,
    /// The type this one was written inside, for a box the template gave no
    /// name of its own. Absent for a named type.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    rows: Vec<DiagramRowDto>,
}

/// One connection, from the row that decides to the row it decides about.
#[derive(Serialize)]
struct DiagramEdgeDto {
    /// Box index and row index: where the edge leaves.
    from: (f64, f64),
    /// Box index where it lands.
    to: f64,
    /// Row index in that box, absent for an edge to the box as a whole, which
    /// is what naming a type is.
    #[serde(skip_serializing_if = "Option::is_none")]
    to_row: Option<f64>,
    /// "length" | "count" | "type" | "position" | "value" | "name" | "width" | "condition" | "case"
    role: &'static str,
    /// The expression the edge stands for, as the template writes it. Empty for
    /// a declaration rather than an expression.
    label: String,
}

/// The format as boxes and arrows, read off the template rather than a file.
#[derive(Serialize)]
struct DiagramDto {
    types: Vec<DiagramBoxDto>,
    edges: Vec<DiagramEdgeDto>,
    /// Named types of the template with no box here.
    omitted: f64,
}

fn diagram_dto(d: Diagram) -> DiagramDto {
    DiagramDto {
        types: d
            .types
            .into_iter()
            .map(|b| DiagramBoxDto {
                name: b.name,
                path: b.path,
                key: b.key,
                kind: b.kind.as_str(),
                parent: b.parent,
                rows: b
                    .rows
                    .into_iter()
                    .map(|r| DiagramRowDto {
                        name: r.name,
                        type_text: r.type_text,
                        size_text: r.size_text,
                        pos_text: r.pos_text,
                        list: r.list,
                        kind: r.kind,
                    })
                    .collect(),
            })
            .collect(),
        edges: d
            .edges
            .into_iter()
            .map(|e| DiagramEdgeDto {
                from: (e.from.0 as f64, e.from.1 as f64),
                to: e.to.0 as f64,
                to_row: e.to.1.map(|r| r as f64),
                role: e.role.as_str(),
                label: e.label,
            })
            .collect(),
        omitted: f64::from(d.omitted),
    }
}

/// How many of one diagram box the open file holds, and where the first is.
#[derive(Serialize)]
struct BoxCountDto {
    /// Matches `DiagramBoxDto::key`.
    key: String,
    count: f64,
    /// Child indices from the root, and which reading they are in: 0 is the
    /// file, anything else an unpacked stream.
    first_path: Vec<f64>,
    space: f64,
}

/// The same for one row of one box.
#[derive(Serialize)]
struct RowCountDto {
    key: String,
    row: f64,
    count: f64,
    first_path: Vec<f64>,
    space: f64,
}

/// What the open file holds, against what the format can hold.
#[derive(Serialize)]
struct CensusDto {
    boxes: Vec<BoxCountDto>,
    rows: Vec<RowCountDto>,
    /// How many nodes the walk has looked at.
    walked: f64,
    /// `done` when every node was counted. Otherwise every count is a floor,
    /// and this says what to do about it: `working` asks again at once,
    /// `waiting` asks again when the bytes it asked for land, and `capped`
    /// asks again only with a higher limit.
    state: &'static str,
}

fn census_dto(c: Census) -> CensusDto {
    CensusDto {
        boxes: c
            .boxes
            .into_iter()
            .map(|b| BoxCountDto {
                key: b.key,
                count: b.count as f64,
                first_path: b.first_path.into_iter().map(|x| x as f64).collect(),
                space: f64::from(b.space),
            })
            .collect(),
        rows: c
            .rows
            .into_iter()
            .map(|r| RowCountDto {
                key: r.key,
                row: r.row as f64,
                count: r.count as f64,
                first_path: r.first_path.into_iter().map(|x| x as f64).collect(),
                space: f64::from(r.space),
            })
            .collect(),
        walked: c.walked as f64,
        state: match c.state {
            CensusState::Done => "done",
            CensusState::Working => "working",
            CensusState::Waiting => "waiting",
            CensusState::Capped => "capped",
        },
    }
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
        stored: o.stored,
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
    let floats = |v: Vec<u64>| v.into_iter().map(|n| n as f64).collect();
    match e {
        Explain::Plain => ExplainDto::Plain,
        Explain::Magic { expected, actual } => ExplainDto::Magic { expected, actual },
        Explain::Enum { name, hex, cases, current, named } => ExplainDto::Enum {
            name,
            hex,
            current: current as f64,
            cases: cases.into_iter().map(|(value, name)| CaseDto { value: value as f64, name }).collect(),
            named: named.unwrap_or_default(),
        },
        Explain::Quant { kind, bits, d, second, block_bits, groups, group_weights, bias, signed, weights, at } => {
            ExplainDto::Quant {
                name: kind.to_string(),
                width: f64::from(bits),
                scale: d,
                second_name: second.as_ref().map(|o| o.name.to_string()).unwrap_or_default(),
                second: second.as_ref().map_or(0.0, |o| o.value),
                second_subtract: second.as_ref().is_some_and(|o| o.subtract),
                second_per_group: second.as_ref().is_some_and(|o| o.per_group),
                block_bits: block_bits as f64,
                group_weights: f64::from(group_weights),
                bias: f64::from(bias),
                signed,
                groups: groups
                    .into_iter()
                    .map(|g| GroupDto { scale: f64::from(g.scale), min: g.min.map(f64::from) })
                    .collect(),
                at: at.map_or(-1.0, |i| i as f64),
                weights: weights
                    .into_iter()
                    .map(|w| WeightDto {
                        q: f64::from(w.q),
                        value: w.value,
                        bits: part_dto(w.bits),
                        high: w.high.map(part_dto),
                    })
                    .collect(),
            }
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
            ExplainDto::Xref {
                widths: widths.iter().map(|w| f64::from(*w)).collect(),
                predictor: predictor.map_or(-1.0, f64::from),
                packed: packed_bytes as f64,
                decoded: decoded_bytes as f64,
                free: free as f64,
                in_file: in_file as f64,
                in_stream: in_stream as f64,
                unknown: unknown as f64,
                total: total as f64,
                problem: problem.unwrap_or_default(),
                rows: rows
                    .into_iter()
                    .map(|r| XrefRowDto {
                        object: r.object as f64,
                        kind: r.kind.as_str(),
                        type_raw: r.kind.raw() as f64,
                        offset: if r.kind == Kind::InFile { r.second as f64 } else { -1.0 },
                        second: r.second as f64,
                        third: r.third as f64,
                    })
                    .collect(),
            }
        }
        Explain::ObjStm { packed_bytes, decoded_bytes, extends, objects, total, problem, .. } => ExplainDto::Objstm {
            packed: packed_bytes as f64,
            decoded: decoded_bytes as f64,
            extends: extends.map_or(-1.0, |n| n as f64),
            total: total as f64,
            problem: problem.unwrap_or_default(),
            objects: objects
                .into_iter()
                .map(|o| ObjStmObjectDto { number: o.number as f64, len: o.len as f64, text: o.text, cut: o.cut })
                .collect(),
        },
        Explain::SqliteRow { declared, found, on_page, pages, chain_length, columns, total_columns, problem } => {
            ExplainDto::Sqliterow {
                declared: declared as f64,
                found: found as f64,
                on_page: on_page as f64,
                pages: pages.into_iter().map(|p| p as f64).collect(),
                chain: chain_length as f64,
                total_columns: total_columns as f64,
                problem: problem.unwrap_or_default(),
                columns: columns
                    .into_iter()
                    .map(|c| {
                        let (value_kind, value, _) = shown(&c.value);
                        SqliteColumnDto { type_name: c.type_name, value, value_kind, at: c.at as f64, len: c.len as f64 }
                    })
                    .collect(),
            }
        }
        Explain::FitsTile {
            index,
            tiles,
            start,
            shape,
            image_shape,
            algorithm,
            column,
            packed_bytes,
            decoded_bytes,
            steps,
            values,
            total,
            pixels,
            element_type,
            problem,
        } => ExplainDto::Tile {
            index: index as f64,
            count: tiles.map(|n| n as f64),
            start: floats(start),
            shape: floats(shape),
            image_shape: floats(image_shape),
            algorithm,
            column: column.unwrap_or_default().to_string(),
            packed: packed_bytes as f64,
            decoded: decoded_bytes as f64,
            values,
            total: total as f64,
            pixels: pixels.map(|n| n as f64),
            element_type,
            problem: problem.unwrap_or_default(),
            steps: steps
                .into_iter()
                .map(|s| PageStepDto {
                    what: s.what,
                    in_bytes: s.in_bytes as f64,
                    out_bytes: s.out_bytes as f64,
                    note: s.note,
                    skipped: false,
                })
                .collect(),
        },
        Explain::ParquetPage { packed_bytes, decoded_bytes, steps, values, total, element_type, problem } => ExplainDto::Page {
            packed: packed_bytes as f64,
            decoded: decoded_bytes as f64,
            total: total as f64,
            element_type,
            values,
            problem: problem.unwrap_or_default(),
            steps: steps
                .into_iter()
                .map(|s| PageStepDto {
                    what: s.what,
                    in_bytes: s.in_bytes as f64,
                    out_bytes: s.out_bytes as f64,
                    note: s.note,
                    skipped: s.skipped,
                })
                .collect(),
        },
        Explain::Hdf5Chunk { packed_bytes, decoded_bytes, steps, values, total, element_type, problem } => ExplainDto::Chunk {
            packed: packed_bytes as f64,
            decoded: decoded_bytes as f64,
            total: total as f64,
            element_type,
            values,
            problem: problem.unwrap_or_default(),
            steps: chunk_steps(steps),
        },
        Explain::BufrData(p) => {
            let p = *p;
            ExplainDto::Bufr {
                edition: f64::from(p.edition),
                master_table_version: f64::from(p.master_table_version),
                tables_version: f64::from(p.tables_version),
                subsets: f64::from(p.subsets),
                compressed: p.compressed,
                steps: p.steps,
                descriptors: p
                    .descriptors
                    .into_iter()
                    .map(|d| BufrDescriptorDto { code: f64::from(d.code), depth: f64::from(d.depth), name: d.name })
                    .collect(),
                descriptors_total: p.descriptors_total as f64,
                subset: f64::from(p.subset),
                values: p.values.into_iter().map(bufr_value_dto).collect(),
                values_start: p.values_start as f64,
                values_total: p.values_total as f64,
                cursor: p.cursor.map(|c| BufrCursorDto {
                    index: c.index.map_or(-1.0, |i| i as f64),
                    value: bufr_value_dto(c.value),
                    bit: c.bit as f64,
                    width: f64::from(c.width),
                    scale: f64::from(c.scale),
                    reference: c.reference as f64,
                    numeric: c.numeric,
                    packed: c.packed.map(|v| v as f64),
                    base: c.base.map(|v| v as f64),
                    increment_width: c.increment_width.map(f64::from),
                    across: c.across,
                }),
                problem: p.problem.unwrap_or_default(),
            }
        }
        Explain::GribValues {
            template,
            spatial_order,
            reference,
            binary_scale,
            decimal_scale,
            minimum,
            declared,
            packed_bytes,
            steps,
            values,
            total,
            at,
            problem,
        } => ExplainDto::Grib {
            template: f64::from(template),
            spatial_order: f64::from(spatial_order),
            reference,
            binary_scale: f64::from(binary_scale),
            decimal_scale: f64::from(decimal_scale),
            minimum: minimum.map(|m| m as f64),
            declared: declared as f64,
            packed: packed_bytes as f64,
            values,
            total: total as f64,
            problem: problem.unwrap_or_default(),
            steps: steps
                .into_iter()
                .map(|s| GribStepDto { label: s.label, what: s.what, count: s.count as f64 })
                .collect(),
            at: at.map(|v| {
                use qubero_core::eval::GribPlace;
                let (place, group, position, written) = match v.place {
                    GribPlace::Values => ("values", 0, 0, 0),
                    GribPlace::First => ("first", 0, 0, 0),
                    GribPlace::Group { group, position, written } => ("group", group, position, written),
                };
                GribValueDto {
                    index: v.index as f64,
                    place,
                    group: group as f64,
                    position: position as f64,
                    written: written as f64,
                    value: v.value,
                    packed: v.packed as f64,
                }
            }),
        },
        Explain::GwfVector {
            compress,
            little,
            declared,
            packed_bytes,
            decoded_bytes,
            steps,
            values,
            total,
            element_type,
            problem,
        } => ExplainDto::Vector {
            compress: f64::from(compress),
            little_endian: little,
            declared: declared as f64,
            packed: packed_bytes as f64,
            decoded: decoded_bytes as f64,
            total: total as f64,
            element_type,
            values,
            problem: problem.unwrap_or_default(),
            steps: steps
                .into_iter()
                .map(|s| VectorStepDto { what: s.filter, in_bytes: s.in_bytes as f64, out_bytes: s.out_bytes as f64 })
                .collect(),
        },
        Explain::MseedSamples {
            encoding,
            encoding_name,
            big_endian,
            declared,
            payload_bytes,
            steim,
            frames_walked,
            rule,
            values,
            last,
            total,
            check,
            problem,
        } => ExplainDto::Samples {
            encoding: encoding_name,
            encoding_number: f64::from(encoding),
            big_endian,
            declared: declared as f64,
            bytes: payload_bytes as f64,
            steim: steim.is_some(),
            x0: steim.as_ref().map(|s| f64::from(s.x0)),
            xn: steim.as_ref().map(|s| f64::from(s.xn)),
            first_difference: steim.as_ref().and_then(|s| s.first_difference.map(f64::from)),
            frames_in_record: steim.as_ref().map_or(0.0, |s| s.frames_in_record as f64),
            frames: steim.map_or_else(Vec::new, |s| {
                s.frames.into_iter().map(|f| MseedFrameDto { held: f.held as f64, used: f.used as f64 }).collect()
            }),
            frames_walked: frames_walked as f64,
            rule: rule.unwrap_or_default(),
            values,
            last: last.unwrap_or_default(),
            total: total as f64,
            check: check.map(|c| c.passed()),
            problem: problem.unwrap_or_default(),
        },
        Explain::Float { format, width, bits } => ExplainDto::Float {
            format: format.to_string(),
            width: f64::from(width),
            pattern: format!("{bits:0>width$x}", width = width as usize / 4),
        },
        Explain::Fixed { bits, frac, signed, raw } => ExplainDto::Fixed {
            bits: f64::from(bits),
            frac: f64::from(frac),
            signed,
            pattern: format!("{raw:0>width$x}", width = bits.div_ceil(4) as usize),
        },
        Explain::Flags { name, raw, bits } => ExplainDto::Flags {
            name,
            current: raw as f64,
            bits: bits.into_iter().map(|b| BitDto { bit: b.bit, name: b.name, set: b.set }).collect(),
        },
    }
}

/// A filter walk's steps as the host reads them.
fn chunk_steps(steps: Vec<qubero_core::formats::hdf5_chunk::Step>) -> Vec<ChunkStepDto> {
    steps
        .into_iter()
        .map(|s| ChunkStepDto { filter: s.filter, in_bytes: s.in_bytes as f64, out_bytes: s.out_bytes as f64, skipped: s.skipped })
        .collect()
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

/// What a `.ksy` the reader pasted in resolves its `meta/imports` against:
/// the files they supplied alongside it first, then the bundled collection.
///
/// The bundled half is what makes `imports: [/common/vlq_base128_le]` work
/// without the reader hunting down a second file: `common/` is seven small
/// files that half the Kaitai library leans on, and they are already here.
/// Their own copy still wins, so a file of theirs by the same name is the one
/// used.
struct PastedImports(qubero_core::ksy::MapImports);

impl qubero_core::ksy::Imports for PastedImports {
    fn load(&self, name: &str) -> Option<String> {
        self.0.load(name).or_else(|| qubero_core::ksy::BundledImports.load(name))
    }
}

/// One entry in the template chooser.
#[derive(Serialize)]
struct TemplateChoiceDto {
    /// What `set_template` takes: a built-in's own name, or `ksy:` and a
    /// bundled format's `meta/id`.
    name: String,
    /// The format's `meta/title`, for a bundled Kaitai format that has one.
    /// Empty otherwise, and the chooser then shows the name.
    title: String,
    /// `builtin` or `kaitai`.
    source: &'static str,
    /// A bundled Kaitai format's `meta/file-extension`, lowercase. Empty for a
    /// built-in, whose extensions the web app lists beside its identity rules.
    ext: Vec<String>,
    /// The bytes a bundled Kaitai format pins, as offset and lowercase hex.
    magic: Vec<(u64, String)>,
}

/// `meta/file-extension` read off a `.ksy` without parsing the rest of it:
/// the chooser lists every bundled format, and a hundred full parses to get
/// one word each would be paid on every file opened.
fn ksy_extensions(text: &str) -> Vec<String> {
    let clean = |e: &str| e.trim().trim_matches(['"', '\'']).to_lowercase();
    let mut out = Vec::new();
    let mut in_meta = false;
    let mut in_list = false;
    for line in text.lines() {
        let bare = line.split(" #").next().unwrap_or("").trim_end();
        if bare.is_empty() {
            continue;
        }
        if !bare.starts_with(' ') {
            in_meta = bare == "meta:";
            in_list = false;
            continue;
        }
        if !in_meta {
            continue;
        }
        let t = bare.trim();
        if in_list {
            if let Some(item) = t.strip_prefix("- ") {
                out.push(clean(item));
                continue;
            }
            in_list = false;
        }
        if let Some(rest) = t.strip_prefix("file-extension:") {
            let rest = rest.trim();
            if rest.is_empty() {
                in_list = true;
            } else {
                let rest = rest.trim_start_matches('[').trim_end_matches(']');
                out.extend(rest.split(',').map(clean).filter(|e| !e.is_empty()));
            }
        }
    }
    out
}

/// What a `.ksy` conversion had to say, for the panel.
#[derive(Serialize)]
struct KsyReportDto {
    /// The format's own id, `meta/id`, which is the name the template goes by
    /// once it is in use. The menu shows it in place of a built-in's name.
    name: String,
    /// One per field converted, in the order the `.ksy` writes them.
    fields: Vec<KsyLineDto>,
    /// Everything the IR could not say, each with what was left in its place.
    gaps: Vec<KsyLineDto>,
    /// Everything said exactly, but not the way the `.ksy` said it.
    notes: Vec<KsyLineDto>,
}

/// The `meta/imports` map both `.ksy` entries take, or what was wrong with it.
fn ksy_imports(imports_json: &str) -> Result<std::collections::HashMap<String, String>, String> {
    if imports_json.trim().is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    serde_json::from_str(imports_json).map_err(|e| format!("the imports are not a JSON object of name to text: {e}"))
}

/// A conversion done for the panel and thrown away: the report, and the
/// template it produced written out as text. Nothing is read with it, so the
/// document keeps whatever template it had.
#[derive(Serialize)]
struct KsyPreviewDto {
    report: KsyReportDto,
    /// The template as [`template_text`](Editor::template_text) writes it.
    text: String,
}

/// One line of the report: where in the `.ksy`, the text there, and what of it.
#[derive(Serialize)]
struct KsyLineDto {
    path: String,
    source: String,
    message: String,
}

fn ksy_report_dto(report: &qubero_core::ksy::Report, name: &str) -> KsyReportDto {
    KsyReportDto {
        name: name.to_string(),
        fields: report
            .fields
            .iter()
            .map(|f| KsyLineDto {
                path: f.path.clone(),
                source: f.source.clone(),
                message: f.message.clone(),
            })
            .collect(),
        gaps: report
            .gaps
            .iter()
            .map(|g| KsyLineDto {
                path: g.path.clone(),
                source: g.source.clone(),
                message: g.reason.clone(),
            })
            .collect(),
        notes: report
            .notes
            .iter()
            .map(|n| KsyLineDto {
                path: n.path.clone(),
                source: n.source.clone(),
                message: n.message.clone(),
            })
            .collect(),
    }
}

// ---- ImHex patterns ----

/// What a `.hexpat` the reader pasted in resolves its `#include`s and
/// `import`s against: the files they supplied alongside it, then the bundled
/// patterns, and then the declaration-only table the core keeps.
///
/// The upstream `includes/` tree is GPL-2.0 and is not here, so a pattern that
/// asks for a file nothing can answer for has to be told which file to hand
/// over. That is what `asked` records: every path this was asked for and could
/// not answer, which the panel turns into a box to paste that file into.
struct PastedIncludes {
    files: qubero_core::hexpat::MapIncludes,
    asked: std::cell::RefCell<Vec<String>>,
}

impl qubero_core::hexpat::Includes for PastedIncludes {
    fn load(&self, path: &str) -> Option<String> {
        if let Some(text) = self.files.load(path) {
            return Some(text);
        }
        if let Some(text) = qubero_core::hexpat::bundled::BundledIncludes.load(path) {
            return Some(text);
        }
        let mut asked = self.asked.borrow_mut();
        if !asked.iter().any(|p| p == path) {
            asked.push(path.to_string());
        }
        None
    }
}

impl PastedIncludes {
    fn new(files: std::collections::HashMap<String, String>) -> Self {
        PastedIncludes {
            files: qubero_core::hexpat::MapIncludes(files),
            asked: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// The paths this could not answer for and the core's own table cannot
    /// either, which is what the reader still has to supply.
    fn missing(&self) -> Vec<String> {
        self.asked
            .borrow()
            .iter()
            .filter(|path| qubero_core::hexpat::includes::builtin(&strip_pat(path)).is_none())
            .cloned()
            .collect()
    }
}

/// A path with its `.pat` or `.hexpat` off, which is the form the two spellings
/// of an include agree on.
fn strip_pat(path: &str) -> String {
    let path = path.replace('\\', "/");
    for extension in [".hexpat", ".pat"] {
        if let Some(stem) = path.strip_suffix(extension) {
            return stem.to_string();
        }
    }
    path
}

/// Every `#include` and `import` written in a pattern, as the resolver would
/// see the path, minus the ones already answered for.
///
/// Parsing stops at the first path it cannot find, so the recorder alone would
/// name one file at a time and a pattern wanting four would take four rounds.
/// This says what the file asks for up front. It is a scan and not a parse, so
/// it sees only what this file writes: an include's own includes appear as they
/// are supplied.
fn hexpat_wanted(text: &str, supplied: &std::collections::HashMap<String, String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let path = if let Some(rest) = line.strip_prefix("#include") {
            let rest = rest.trim();
            rest.strip_prefix('<')
                .and_then(|r| r.split('>').next())
                .or_else(|| rest.strip_prefix('"').and_then(|r| r.split('"').next()))
                .map(|p| p.trim().to_string())
        } else if let Some(rest) = line.strip_prefix("import ") {
            // `import std.mem;`, and `import * from fs.mbr as MBR;`, which
            // names its file after the `from`.
            let rest = rest.trim().trim_end_matches(';');
            let rest = rest.split(" as ").next().unwrap_or(rest).trim();
            let rest = rest.rsplit(" from ").next().unwrap_or(rest).trim();
            if rest.is_empty() || rest.contains(' ') {
                None
            } else {
                Some(rest.replace('.', "/"))
            }
        } else {
            None
        };
        let Some(path) = path else { continue };
        let key = strip_pat(&path);
        if qubero_core::hexpat::includes::builtin(&key).is_some() {
            continue;
        }
        if qubero_core::hexpat::bundled::BundledIncludes.load(&path).is_some() {
            continue;
        }
        if supplied.keys().any(|name| strip_pat(name) == key) {
            continue;
        }
        if !out.iter().any(|p| *p == path) {
            out.push(path);
        }
    }
    out
}

/// What a `.hexpat` conversion said, or what was wrong with the pattern, in the
/// panel's terms.
///
/// `missing` is the include files the pattern asks for and nothing here has.
/// It comes back with a failure and with a success alike, because a pattern can
/// convert without one of its includes and be the poorer for it.
#[derive(Serialize)]
struct HexpatReportDto {
    /// The name the template goes by once it is in use.
    name: String,
    fields: Vec<KsyLineDto>,
    gaps: Vec<KsyLineDto>,
    notes: Vec<KsyLineDto>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    missing: Vec<String>,
}

/// A conversion done for the panel and thrown away: the report, and the
/// template it produced written out as text.
#[derive(Serialize)]
struct HexpatPreviewDto {
    report: HexpatReportDto,
    text: String,
}

/// A failed conversion, with the include files the pattern wanted. The message
/// leads with the line and column, which is where the panel puts it.
#[derive(Serialize)]
struct HexpatFailedDto {
    message: String,
    missing: Vec<String>,
}

/// The failure envelope for the two `.hexpat` entries.
///
/// The shared [`Reply::Error`] carries a message and nothing else, and an
/// include file the pattern is waiting for is not a message: it is a thing for
/// the reader to hand over. Flattened, so a caller reading `status` and
/// `message` sees exactly what it sees from every other entry.
#[derive(Serialize)]
struct HexpatFailedReply {
    status: &'static str,
    #[serde(flatten)]
    node: HexpatFailedDto,
}

fn hexpat_report_dto(
    report: &qubero_core::report::Report,
    name: &str,
    missing: Vec<String>,
) -> HexpatReportDto {
    let line = |path: &str, source: &str, message: &str| KsyLineDto {
        path: path.to_string(),
        source: source.to_string(),
        message: message.to_string(),
    };
    HexpatReportDto {
        name: name.to_string(),
        fields: report.fields.iter().map(|f| line(&f.path, &f.source, &f.message)).collect(),
        gaps: report.gaps.iter().map(|g| line(&g.path, &g.source, &g.reason)).collect(),
        notes: report.notes.iter().map(|n| line(&n.path, &n.source, &n.message)).collect(),
        missing,
    }
}

/// The error as the panel shows it: `line:col: what was wrong`, with the file
/// named only when it is not the one in the box.
fn hexpat_error(error: &qubero_core::hexpat::HexpatError, name: &str) -> String {
    if error.file.is_empty() || error.file == name {
        format!("{}: {}", error.pos, error.message)
    } else {
        format!("{} {}: {}", error.file, error.pos, error.message)
    }
}

/// The name a converted pattern goes by: what the caller passed, or `pattern`
/// for a paste with no file behind it.
fn hexpat_name(name: &str) -> String {
    let name = name.trim().trim_end_matches(".hexpat").trim_end_matches(".pat").trim();
    if name.is_empty() {
        "pattern".to_string()
    } else {
        name.to_string()
    }
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

/// One node of an HDF5 B-tree, as much of it as a picture of the tree needs.
#[derive(Serialize)]
struct TreeNodeDto {
    /// Where the node is in the template. Empty for a version 2 node below the
    /// root, which the template does not place: the host takes such a box to
    /// its bytes and does not try to open it in the Listing, because there is
    /// no field there to open.
    path: Vec<usize>,
    /// Index into the node list, or -1 for the root.
    parent: f64,
    /// `index` for a `TREE` or a `BTIN`, `links` for the symbol table node a
    /// version 1 group tree hangs under its bottom row, `leaf` for a `BTLF`.
    /// The words are the host's to translate; what crosses is which of the
    /// three kinds of node this is.
    kind: &'static str,
    /// The four bytes written at `address`. Sent rather than worked out from
    /// `kind` and the tree's version, so that what a reader is told to expect
    /// at an address is what the walk checked for there.
    sign: &'static str,
    address: f64,
    size_bits: f64,
    /// What the file wrote as this node's level, for a version 1 index node.
    /// Zero for a link table, which sits below the levels rather than on one.
    /// A version 2 node writes no level, so this is the header's depth less
    /// the rows walked to reach it, which keeps level 0 meaning the bottom row
    /// of index nodes in both versions.
    level: f64,
    /// Rows below the root, counted by the walk.
    depth: f64,
    /// `entries_used`, `symbol_count`, or a version 2 node's record count. The
    /// file's own number and no fraction of anything. A version 2 internal
    /// node points at one more child than it holds records, because a record
    /// sits between every two children.
    entries: f64,
    /// The ends of the node's key range, or empty where the walk could not
    /// settle both, and empty throughout a version 2 group tree, whose records
    /// hold the hash of a name rather than the name. For a version 1 group
    /// tree these are link names; for a chunk tree of either version they are
    /// the comma-separated numbers of a chunk's offset in the dataset.
    first_key: String,
    last_key: String,
    /// True when children of this node were not reached, so its count stands
    /// and its range does not.
    truncated: bool,
    /// Where the node's first entry starts, in bits from `address`, and how
    /// many bits one entry takes, so the host can place all `entries` of them
    /// without a list: entry `i` is at `address * 8 + first_entry_bits + i *
    /// entry_bits`. What one entry covers is a key and a child address for a
    /// `TREE`, one symbol table entry for an `SNOD`, one record for a `BTLF`
    /// or a `BTIN`. A `TREE`'s closing key and a `BTIN`'s child pointers are
    /// not entries and are not on the stride.
    ///
    /// Both zero where the walk could not settle the stride, which is the one
    /// case a host must leave the node undivided rather than take the zero for
    /// a width.
    first_entry_bits: f64,
    entry_bits: f64,
}

/// One HDF5 B-tree, walked into the shape it has in the file.
#[derive(Serialize)]
struct TreeDto {
    /// `group` for a tree indexing a group's links, `chunk` for one indexing a
    /// dataset's chunks, `other` for a version 2 tree indexing neither: a
    /// file's shared messages, an object's attributes, or its huge objects.
    /// Only a version 1 group tree has link tables under its bottom row, so
    /// `job` alone does not settle the silhouette; `version` does.
    job: &'static str,
    /// 1 or 2: which of the two structures this is.
    version: f64,
    /// The record type byte a version 2 header writes, and the name the
    /// specification gives it. Zero and empty for a version 1 tree, which has
    /// no such byte.
    record_type: f64,
    record_type_name: &'static str,
    /// How far the walk got with the records: `read` where what a record holds
    /// is known here, `unread` where the specification names the type and this
    /// does not read it, `unknown` where no version of the specification names
    /// it. The shape is drawn whichever of the three it is, because the shape
    /// depends only on the record size, and the host says outright when the
    /// records behind a drawn shape were not read.
    records: &'static str,
    /// How many records the tree holds in all, which a version 2 header writes
    /// in a field of its own. Zero for a version 1 tree. Not the sum of the
    /// nodes' counts: a version 2 internal node holds records the leaves under
    /// it do not repeat, so a host that summed the bottom row would print a
    /// total the file disagrees with, and this stands when the walk was capped.
    records_total: f64,
    nodes: Vec<TreeNodeDto>,
    omitted: f64,
    /// How many numbers one chunk key holds. Zero for a group tree and for a
    /// tree whose records were not read.
    coords: f64,
    /// True where the last of those numbers is the always-zero offset within
    /// an element, which a version 1 chunk key ends with and a version 2
    /// record does not. Without it a reader counting the numbers in
    /// `950, 950, 0` gets a rank one too high.
    coords_pad: bool,
}

/// The named parts of an ELF file. Unlike the storage template, these have
/// resolved section and symbol names rather than string-table offsets.
#[derive(Serialize)]
struct ElfContentsDto {
    sections: Vec<ElfSectionDto>,
    symbols: Vec<ElfSymbolDto>,
    symbol_total: f64,
}

/// What a ROOT file holds, read beside the template.
#[derive(Serialize)]
struct RootContentsDto {
    classes: Vec<RootClassDto>,
    /// Where the `StreamerInfo` record is in the template.
    schema_path: Vec<usize>,
    trees: Vec<RootTreeDto>,
    tree_total: f64,
    /// What stopped the walk, empty where nothing did.
    trouble: String,
}

#[derive(Serialize)]
struct RootClassDto {
    name: String,
    version: f64,
    checksum: f64,
    members: Vec<RootMemberDto>,
}

#[derive(Serialize)]
struct RootMemberDto {
    name: String,
    /// The C++ type as the file spells it.
    type_name: String,
    /// ROOT's own type code, which says how the member is written.
    code: f64,
    size: f64,
    /// The dimensions of a fixed array, empty for a single value.
    dims: Vec<f64>,
    /// True where this is a base class rather than a member of its own.
    base: bool,
    comment: String,
}

#[derive(Serialize)]
struct RootTreeDto {
    /// Where the tree's key is in the template, so picking a row moves the
    /// cursor.
    path: Vec<usize>,
    name: String,
    title: String,
    entries: f64,
    address: f64,
    branches: Vec<RootBranchDto>,
    branch_total: f64,
    trouble: String,
}

#[derive(Serialize)]
struct RootBranchDto {
    name: String,
    title: String,
    /// `TBranch` for numbers, `TBranchElement` for a member of a split C++
    /// class.
    class: String,
    depth: f64,
    entries: f64,
    total_bytes: f64,
    zip_bytes: f64,
    basket_total: f64,
    /// One value's width in bytes and how many of them an entry holds, or zero
    /// for a branch whose values are not read.
    width: f64,
    per_entry: f64,
    floating: bool,
    unsigned: bool,
    /// Why the values are not read, empty where they are.
    unread: String,
    leaves: Vec<RootLeafDto>,
    baskets: Vec<RootBasketDto>,
}

#[derive(Serialize)]
struct RootLeafDto {
    name: String,
    class: String,
    len: f64,
    width: f64,
    unsigned: bool,
    /// The leaf that counts this one, empty where the count is fixed.
    counted_by: String,
}

/// One basket: a record of the file that no key list lists and no field
/// places, so the host takes a row to its bytes and cannot open it in the
/// Listing.
#[derive(Serialize)]
struct RootBasketDto {
    address: f64,
    bytes: f64,
    first_entry: f64,
    entries: f64,
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
    /// The template wrote this structure to hold one value in several fields,
    /// so the name it gave the structure is the template's own bookkeeping and
    /// is not shown to a reader.
    inline: bool,
    /// What is wrong with this field's value, when something is.
    #[serde(skip_serializing_if = "Option::is_none")]
    problem: Option<ProblemDto>,
    /// Wrong values found under this span so far: invalid, then undefined.
    problems_within: [u32; 2],
}

/// The bit split of one variable-length number, in the order it is stored.
/// What is wrong with one value, in the words the core built. `tier` says who
/// says so: `invalid` is the format ruling the value out, `undefined` is
/// Qubero having no name for it.
#[derive(Serialize)]
struct ProblemDto {
    tier: &'static str,
    text: String,
}

fn problem_dto(p: qubero_core::eval::Problem) -> ProblemDto {
    ProblemDto {
        tier: match p.tier {
            qubero_core::eval::Tier::Invalid => "invalid",
            qubero_core::eval::Tier::Undefined => "undefined",
        },
        text: p.text,
    }
}

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
    let (kind, value, _) = shown(&s.value);
    SpanDto {
        path: s.path,
        problem: s.problem.map(problem_dto),
        problems_within: [s.problems_within.0, s.problems_within.1],
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
        inline: s.inline,
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

/// How a value reads: its kind, what to show, and what an editor starts with.
/// Whether the format says it is right is `NodeDto::problem`, built in the
/// core so that every view says the same words about the same bytes.
fn shown(v: &Value) -> (&'static str, String, String) {
    match v {
        Value::UInt(v) => ("uint", v.to_string(), v.to_string()),
        Value::Int(v) => ("int", v.to_string(), v.to_string()),
        Value::Float(v) => ("float", v.to_string(), v.to_string()),
        Value::Bytes { len, preview } => {
            let hex: Vec<String> = preview.iter().map(|b| format!("{b:02x}")).collect();
            let mut s = hex.join(" ");
            if *len as usize > preview.len() {
                s.push('…');
            }
            ("bytes", s.clone(), s)
        }
        // The bytes have not arrived; the row stands on what the file's own
        // table already said about where they are and how many there are.
        Value::Unread { .. } => ("unread", "\u{2026}".into(), String::new()),
        // A slot the file left at its format's "nobody filled this in" value.
        // The editor still starts from the number that is written there, so
        // opening the field shows what would be overwritten.
        Value::Unset(inner) => ("unset", "unset".into(), shown(inner).2),
        Value::Str(s) => ("str", s.clone(), s.clone()),
        Value::Magic { bytes, .. } => {
            // How a signature reads is core's answer, not this crate's, so
            // that the listing and the type table say the same thing about
            // the same bytes. What is wrong with a signature that does not
            // match is `problem`, not part of the value. See
            // `eval::magic_reading`.
            let s = qubero_core::eval::magic_reading(bytes);
            ("magic", s.clone(), s)
        }
        Value::Composite { count } => ("composite", count.to_string(), count.to_string()),
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
            ("flags", s, raw.to_string())
        }
        Value::Enum { raw, name, hex } => {
            let num = if *hex && *raw >= 0 { format!("0x{raw:02x}") } else { raw.to_string() };
            match name {
                Some(n) => ("enum", format!("{n} ({num})"), n.clone()),
                // A value the format does not define. Worth flagging, still editable.
                None => ("enum", format!("{num} (unknown)"), num),
            }
        }
    }
}

fn dto(n: NodeInfo) -> NodeDto {
    let (kind, value, edit_text) = shown(&n.value);
    // A field the file did not write has nothing to show. Its node reads as an
    // empty composite, and a value column saying `0` there is a number nobody
    // wrote and one a reader would take for the field's contents. `absent` is
    // the whole of what there is to say about the row.
    let (value, edit_text) = if n.absent { (String::new(), String::new()) } else { (value, edit_text) };
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
        problem: n.problem.map(problem_dto),
        problems_within: [n.problems_within.0, n.problems_within.1],
        child_count: n.child_count as f64,
        unit: n.unit.unwrap_or_default(),
        composite: n.composite,
        list: n.list,
        editable: n.editable,
        value_bytes: n.value_bytes as f64,
        value_offset_bits: n.value_offset_bits as f64,
        read_at: n.read_at.map(|at| at as f64),
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
        joined: n.joined,
        absent: n.absent,
        doc: n.doc,
        line: n.line,
        table: n.table,
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
        problem: c.problem.map(problem_dto),
        problems_within: [c.problems_within.0, c.problems_within.1],
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

/// The chunks a reading waits on, as a byte read reports chunks not loaded.
/// Nothing for any other error: the read then answers from what is there.
fn chunks_of(err: EvalError) -> Vec<f64> {
    match err {
        EvalError::Pending(m) => m.into_iter().map(|m| m.chunk as f64).collect(),
        _ => Vec::new(),
    }
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
    /// True when the stream was joined from several runs rather than unpacked
    /// from one, which is a different thing to call the tab.
    joined: bool,
    /// True when every one of those runs is stored as it sits in the file, so
    /// a field of the stream can be edited where the file declares it.
    stored: bool,
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
    /// Where in the file the run the step was read from starts: the run the
    /// stream was unpacked from, or for a stream joined from several, the run
    /// of the part the step belongs to. The step's own bits count from there,
    /// so the file's bits are this plus them.
    run_offset_bits: f64,
}

/// Which way a step of a space was asked for, which is how its run is found
/// when the space was joined from several: by the byte it made, or by the bit
/// of the file it read.
enum AskedBy {
    Byte(u64),
    Bit(u64),
}

/// A step of a space, with where its run is: the one run the space was
/// unpacked from, or, for a space joined from several, the run of the part the
/// step belongs to. Nothing for a step of a run that is not in the file, which
/// has no bits there to mark: a stream declared inside another stream has its
/// run in that stream's bytes.
///
/// `e` is the file's reading, so a run's space 0 is the file.
fn space_step_dto(e: &Evaluator, space: SpaceId, s: MapStep, asked: AskedBy) -> Option<MapStepDto> {
    let sp = e.space(space)?;
    let (run_space, run_offset_bits) = match sp.run() {
        Some(run) => (run.run_space, run.run_offset_bits),
        None => {
            let run = match asked {
                AskedBy::Byte(byte) => sp.run_at(byte),
                AskedBy::Bit(bit) => sp.run_holding(bit),
            }?;
            (run.run_space, run.run_offset_bits)
        }
    };
    (run_space == 0).then(|| step_dto(s, run_offset_bits))
}

fn step_dto(s: MapStep, run_offset_bits: u64) -> MapStepDto {
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
        run_offset_bits: run_offset_bits as f64,
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

    /// Name the space the call is about, opening its stream again first if an
    /// edit has let go of it. Every space-taking method starts here. The error
    /// is the reply to give instead: the reopen waits on chunks of the file.
    fn go(&mut self, space: u32) -> Result<(), String> {
        self.live = space as usize;
        self.ensure_open(space).map_err(|err| reply::<()>(Err(err)))
    }

    /// Let go of every unpacked stream. A space is worked out from bytes of
    /// the file, so an edit or a change of template throws it away and the
    /// tab reopens it by path. The file's own reading, space 0, stays.
    ///
    /// The spaces are not dropped, because the tabs showing them are still
    /// open and a tab has to keep meaning what its title says. Each is marked
    /// stale instead, and `ensure_open` unpacks it again from the stream it
    /// came from the next time anything of the tab is asked about, so the
    /// tab's own number stays good; the core renumbers its own spaces from 1
    /// and the new number is written back then. Opening it then rather than
    /// here is what makes the reopen see the edit: every caller of this has
    /// the edit still to make, or the reading still to tell of it, and a
    /// stream unpacked here would be unpacked from the file as it was. It is
    /// also at most one unpacking per tab however many edits came first.
    fn forget_spaces(&mut self) {
        self.live = 0;
        for sh in self.sheets.iter_mut().skip(1) {
            sh.stale = true;
        }
    }

    /// Open the stream behind `space` again if an edit or a change of template
    /// has let go of it since, whichever way the tab reads it. Nothing to do
    /// for the file, or for a stream still open.
    ///
    /// Its home is opened first, since a stream opened from a recognised one
    /// is opened again in that one's new reading: a sheet's home is always
    /// opened before it. The error is only ever that a reading waits on chunks
    /// of the file, and the sheet then stays as it was until it is asked
    /// again. A stream that no longer opens is not an error: it leaves the tab
    /// empty rather than reading whatever the new reading has at the old path.
    fn ensure_open(&mut self, space: u32) -> Result<(), EvalError> {
        let i = space as usize;
        if i == 0 || i >= self.sheets.len() {
            return Ok(());
        }
        let home = self.sheets[i].home;
        if home != i {
            self.ensure_open(home as u32)?;
        }
        // A sheet still holding a stream its home's reading has since dropped
        // is as stale as one an edit marked. The core lets go of every space
        // whenever a reading is invalidated, so this catches a change that
        // did not pass through `forget_spaces`, and a home opened again since
        // with a reading of its own that has nothing open yet.
        let dropped = self.sheets[i].core_space != 0 && self.core_space(space).is_none();
        if !self.sheets[i].stale && !dropped {
            return Ok(());
        }
        self.reopen(i)
    }

    /// Open the stream at `path` of sheet `home`'s own reading, with no
    /// allowance: unpacking a run is not something to do by halves, since it
    /// reads the whole run and decodes it, and a half-decoded stream is not a
    /// document. Nothing when the sheet has no reading.
    fn open_in(&mut self, home: usize, path: &[usize]) -> Result<Option<SpaceId>, EvalError> {
        let Some(sh) = self.sheets.get_mut(home) else { return Ok(None) };
        let Some(e) = &mut sh.eval else { return Ok(None) };
        e.set_slice(None);
        let got = e.open_space(&sh.doc, 0, path);
        e.set_slice(Some(WORK_SLICE));
        got
    }

    /// The sheet for a stream `open_in` opened, or an empty one for a stream
    /// that did not open.
    fn sheet_for(&self, opened: Option<SpaceId>, home: usize, origin: Vec<usize>) -> Sheet {
        let space = opened.and_then(|id| self.sheets[home].eval.as_ref()?.space(id));
        match space {
            Some(sp) => Sheet::from_space(sp, home, origin),
            None => Sheet::empty(home, origin),
        }
    }

    /// What reads the fields of `space`, lent for one call: its own reading, or
    /// for a stream read where it was declared, its home's, under the stream.
    /// The error is the reply to give instead, when nothing reads it, or when
    /// opening the stream again waits on chunks of the file.
    fn tab(&mut self, space: u32) -> Result<Tab<'_, ChunkStore>, String> {
        self.go(space)?;
        let i = if self.live < self.sheets.len() { self.live } else { 0 };
        let no_template = || reply::<()>(Err(EvalError::Failed("no template".into())));
        match self.sheets[i].view.clone() {
            None => {
                let sh = &mut self.sheets[i];
                let e = sh.eval.as_mut().ok_or_else(no_template)?;
                Ok(Tab::new(e, &sh.doc, Vec::new()))
            }
            Some(root) => {
                let h = self.sheets[i].home;
                let home = &mut self.sheets[h];
                let e = home.eval.as_mut().ok_or_else(no_template)?;
                Ok(Tab::new(e, &home.doc, root))
            }
        }
    }

    /// Open the stream sheet `i` came from again, in its home's reading as it
    /// is now. A stream that no longer opens leaves an empty sheet; one waiting
    /// on bytes of the file says so, stays stale, and is opened when it is
    /// asked again.
    fn reopen(&mut self, i: usize) -> Result<(), EvalError> {
        let (home, origin) = (self.sheets[i].home, self.sheets[i].origin.clone());
        let opened = match self.open_in(home, &origin) {
            Ok(opened) => opened,
            Err(err) if err.interrupted() => return Err(err),
            Err(_) => None,
        };
        self.sheets[i] = self.sheet_for(opened, home, origin);
        Ok(())
    }

    /// Open the `Decoded` or joined stream at `path` of the tab over `space` as
    /// a document of its own, and give back the space it became:
    /// {status:"ok",node:{space,refused}}.
    ///
    /// `path` is the tab's, which is a path of the file only in the file's own
    /// tab. A tab over a stream read where it was declared has its fields under
    /// the stream in its home's reading, so the stream is opened there, under
    /// the tab's root, and is then the same stream the file's own tab would
    /// open. A tab with a reading of its own opens it in that reading.
    ///
    /// A stream already open answers with the space it already is, so a second
    /// Open unpacked focuses the tab instead of unpacking the run again.
    /// `refused` says which of the three ways a stream would not open, and the
    /// space is then 0.
    pub fn open_space(&mut self, space: u32, path: &[u32]) -> String {
        if let Err(err) = self.ensure_open(space) {
            return reply::<SpaceDto>(Err(err));
        }
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let (home, at) = match self.sheets.get(space as usize) {
            Some(sh) if space != 0 => match &sh.view {
                Some(root) => (sh.home, [root.as_slice(), &p].concat()),
                None => (space as usize, p),
            },
            _ => (0, p),
        };
        let known = self.sheets.iter().enumerate().skip(1).find(|(_, sh)| sh.home == home && sh.origin == at).map(|(i, _)| i);
        if let Some(i) = known {
            let template = self.sheets[i].template.clone();
            let (joined, stored) = self.space_joined(i as u32);
            return reply(Ok(SpaceDto { space: i as f64, template, refused: None, joined, stored }));
        }
        self.live = 0;
        if self.sheets.get(home).is_none_or(|sh| sh.eval.is_none()) {
            return reply::<SpaceDto>(Err(EvalError::Failed("no template".into())));
        }
        let id = match self.open_in(home, &at) {
            Ok(Some(id)) => id,
            // The stream would not open. Which of the three ways is already on
            // the node, so the reply only has to say that it did not.
            Ok(None) => {
                let why = self.refusal_at(home, &at);
                return reply(Ok(SpaceDto { space: 0.0, template: String::new(), refused: Some(why), joined: false, stored: false }));
            }
            Err(err) => return reply::<SpaceDto>(Err(err)),
        };
        if self.sheets[home].eval.as_ref().and_then(|e| e.space(id)).is_none() {
            return reply::<SpaceDto>(Err(EvalError::Failed("space vanished".into())));
        }
        let sheet = self.sheet_for(Some(id), home, at);
        let template = sheet.template.clone();
        self.sheets.push(sheet);
        let space = (self.sheets.len() - 1) as u32;
        let (joined, stored) = self.space_joined(space);
        reply(Ok(SpaceDto { space: space as f64, template, refused: None, joined, stored }))
    }

    /// Whether one of this editor's spaces is a stream joined from several
    /// runs, and whether every one of those runs is stored as it sits in the
    /// file, which only a stream opened in the file's reading can say.
    fn space_joined(&self, space: u32) -> (bool, bool) {
        let Some((home, sp)) = self.core_space_of(space) else { return (false, false) };
        let runs = sp.runs();
        (!runs.is_empty(), home == 0 && !runs.is_empty() && runs.iter().all(|r| !r.packed && r.run_space == 0))
    }

    /// Why the stream at `path` of sheet `home`'s reading would not open, in
    /// the core's own word for it. A joined stream too long to hold whole says
    /// so only when asked, since its node still reads.
    fn refusal_at(&mut self, home: usize, path: &[usize]) -> String {
        let Some(sh) = self.sheets.get_mut(home) else { return "failed".into() };
        let Some(e) = &mut sh.eval else { return "failed".into() };
        if let Some(why) = e.open_refusal(path) {
            return why.as_str().into();
        }
        match e.node(&sh.doc, path) {
            Ok(n) => n.refused.unwrap_or_else(|| "failed".into()),
            Err(_) => "failed".into(),
        }
    }

    /// Which bits of the compressed run the byte at `byte` of `space` came
    /// from, and by which step: {status:"ok",node:{..}} or a null node when the
    /// codec's map does not reach that far. The step's bits count from the
    /// start of its run, and `run_offset_bits` says where in the file that is.
    ///
    /// Null too for a stream whose run is bits of another stream and not of
    /// the file: one opened in a recognised stream's reading, or one declared
    /// inside a stream the file declares.
    pub fn map_out(&mut self, space: u32, byte: f64) -> String {
        if let Err(err) = self.ensure_open(space) {
            return reply::<Option<MapStepDto>>(Err(err));
        }
        let Some(core) = self.file_core_space(space) else { return reply(Ok(None::<MapStepDto>)) };
        let Some(e) = &self.sheets[0].eval else { return reply(Ok(None::<MapStepDto>)) };
        reply(Ok(e.map_out(core, byte as u64).and_then(|s| space_step_dto(e, core, s, AskedBy::Byte(byte as u64)))))
    }

    /// Which step read the bit at `bit` of the file, when that bit is in the
    /// run `space` was unpacked from, and so which of its bytes that bit
    /// produced. Counted in the file and not in the run, since the file tab's
    /// cursor is what asks.
    pub fn map_in(&mut self, space: u32, bit: f64) -> String {
        if let Err(err) = self.ensure_open(space) {
            return reply::<Option<MapStepDto>>(Err(err));
        }
        let Some(core) = self.file_core_space(space) else { return reply(Ok(None::<MapStepDto>)) };
        let Some(e) = &self.sheets[0].eval else { return reply(Ok(None::<MapStepDto>)) };
        reply(Ok(e.map_in(core, bit as u64).and_then(|s| space_step_dto(e, core, s, AskedBy::Bit(bit as u64)))))
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
        if let Err(err) = self.ensure_open(space) {
            return reply::<Option<DecodedStepDto>>(Err(err));
        }
        let none = || reply(Ok(None::<DecodedStepDto>));
        let (Some(core), Some(sh)) = (self.core_space(space), self.sheets.get(space as usize)) else {
            return none();
        };
        let (home, origin) = (sh.home, sh.origin.clone());
        // Which step, and whether this is a codec with symbols at all. Both
        // come off the trace, so a question with no answer is answered before
        // any of the run is read.
        let (index, want_bytes) = {
            let Some(e) = &self.sheets[home].eval else { return none() };
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
        // what came out. They are a field of the reading that opened the
        // stream, the file's or a recognised stream's, and a read of the
        // file's can want chunks that are not here yet.
        self.live = home;
        let at = &mut self.sheets[home];
        let Some(e) = &mut at.eval else { return none() };
        e.begin_slice();
        let run = match e.field_bytes(&at.doc, &origin, want_bytes) {
            Ok((bytes, _)) => bytes,
            Err(err) => return reply::<Option<DecodedStepDto>>(Err(err)),
        };
        let Some(e) = &self.sheets[home].eval else { return none() };
        let Some(sp) = e.space(core) else { return none() };
        reply(Ok(inflate::decode_step(&run, sp.trace(), index).map(decoded_step_dto)))
    }

    /// The `Decoded` node a space was unpacked from, as a path in the reading
    /// that opened it: the file's, or a recognised stream's. Empty for space 0
    /// and for a space that is no longer open.
    pub fn space_origin(&self, space: u32) -> Vec<u32> {
        match self.sheets.get(space as usize) {
            Some(sh) if space != 0 => sh.origin.iter().map(|&x| x as u32).collect(),
            _ => Vec::new(),
        }
    }

    /// True when the template reading a space came from looking at the unpacked
    /// bytes rather than from what the stream declared: a gzip of a tar opens
    /// as a tar, and this is what says so.
    pub fn space_recognised(&mut self, space: u32) -> bool {
        self.open_again(space);
        self.core_space_of(space).is_some_and(|(_, s)| s.recognised)
    }

    /// The core's space for one of this editor's spaces, if it is still open,
    /// with the sheet whose reading holds it.
    ///
    /// Still open as the same stream: the core numbers its spaces from 1 again
    /// once a reading is thrown away, and a stream opened again since then may
    /// have been given the number this one had.
    fn core_space_of(&self, space: u32) -> Option<(usize, &qubero_core::eval::Space)> {
        let sh = self.sheets.get(space as usize)?;
        if space == 0 || sh.core_space == 0 {
            return None;
        }
        let open = self.sheets.get(sh.home)?.eval.as_ref()?.space(sh.core_space)?;
        (open.parent == 0 && open.path == sh.origin).then_some((sh.home, open))
    }

    /// `ensure_open` for a call that answers from whatever the tab holds when
    /// the reopen waits on chunks of the file: the old bytes, the old length,
    /// or the old reading, which the next call opens again.
    fn open_again(&mut self, space: u32) {
        let _ = self.ensure_open(space);
    }

    /// The core's number for one of this editor's spaces, if it is still open.
    fn core_space(&self, space: u32) -> Option<SpaceId> {
        self.core_space_of(space).map(|(_, sp)| sp.id)
    }

    /// The same, only for a space the file's reading opened, which is the only
    /// reading whose space 0 is the file. Whether its run is bits of the file
    /// or of a stream it was declared inside is the run's to say, and
    /// `space_step_dto` asks it, for a stream unpacked from one run and for
    /// each part of a joined one alike.
    fn file_core_space(&self, space: u32) -> Option<SpaceId> {
        match self.core_space_of(space)? {
            (0, sp) => Some(sp.id),
            _ => None,
        }
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
        sh.census = None;
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
        sh.census = None;
    }

    /// One step of the byte-class scan behind the overview: at most a window
    /// of the file read and classified. The reply is the usual tri-state, with
    /// `node` carrying everything found so far, so the host can draw a partial
    /// map while the rest is read. `done` on the node says when to stop asking.
    pub fn overview_step(&mut self, space: u32, buckets: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
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
        if let Err(why) = self.go(space) {
            return why;
        }
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
        // The walk is this sheet's own, and a stream read where it was
        // declared walks it in the file's reading, so it is taken out of the
        // sheet for the length of the call. After the sheet is opened again,
        // if it has to be, since that starts the sheet over.
        if let Err(why) = self.tab(space) {
            return why;
        }
        let sh = self.sm();
        let len = sh.doc.len_bits();
        let mut kinds = sh.kinds.take();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        // A walk built for a file of another length is about other bytes.
        if !matches!(&kinds, Some(w) if w.file_bits() == len) {
            kinds = Some(tab.kind_walk(len));
        }
        let walk = kinds.as_mut().expect("just built");
        tab.ev.begin_slice();
        let out = tab.kind_totals_step(walk);
        let reached = (tab.ev.reached_bits() / 8) as f64;
        self.sm().kinds = kinds;
        reply_with(out.map(kind_totals_dto), reached, Vec::new())
    }

    // ----- templates -----

    /// Every template a reader may pick, as JSON: the built-in ones first,
    /// then the bundled Kaitai Struct formats.
    ///
    /// Each carries the name [`set_template`](Self::set_template) takes, the
    /// title to show for it, and which of the two kinds it is, because the
    /// chooser groups them. A built-in has no title of its own: the name is
    /// what it has always been shown as, and the web side already knows how to
    /// spell each one out. A bundled format's title is its `meta/title`, which
    /// is empty for the third of them that carry none.
    pub fn template_names(&self) -> String {
        let builtins = formats::builtin_names().into_iter().map(|name| TemplateChoiceDto {
            name: name.to_string(),
            title: String::new(),
            source: "builtin",
            ext: Vec::new(),
            magic: Vec::new(),
        });
        let kaitai = qubero_core::ksy::bundled::all()
            .iter()
            .filter(|entry| entry.offered)
            .map(|entry| TemplateChoiceDto {
                name: entry.name.to_string(),
                title: entry.title.to_string(),
                source: "kaitai",
                ext: ksy_extensions(entry.text),
                magic: entry.signature.iter().map(|(at, bytes)| (*at, bytes.iter().map(|b| format!("{b:02x}")).collect())).collect(),
            });
        let imhex = qubero_core::hexpat::bundled::all()
            .iter()
            .filter(|entry| entry.offered)
            .map(|entry| TemplateChoiceDto {
                name: entry.name.to_string(),
                title: entry.title.to_string(),
                source: "hexpat",
                // An ImHex pattern says nothing about file extensions; what it
                // says about the files it is for is its magic.
                ext: Vec::new(),
                magic: entry.signature.iter().map(|(at, bytes)| (*at, bytes.iter().map(|b| format!("{b:02x}")).collect())).collect(),
            });
        serde_json::to_string(&builtins.chain(kaitai).chain(imhex).collect::<Vec<_>>()).unwrap_or_default()
    }

    /// The template in use, written out as text: every type, every field and
    /// every expression behind them. Empty when no template is selected.
    ///
    /// The file's own template, space 0, since that is the one `set_template`
    /// sets; an unpacked stream is read by whatever its `Decoded` node declared
    /// and has no template of the reader's choosing. Read-only, and read in one
    /// go: it is a page of text, not a walk.
    pub fn template_text(&self) -> String {
        match self.sheets[0].eval.as_ref() {
            Some(e) => qubero_core::template_text::render(e.template()),
            None => String::new(),
        }
    }

    /// Best current projection for a variable-size array being walked, or an
    /// empty string when no unfinished walk has enough information yet.
    pub fn extent_estimate(&mut self, space: u32) -> String {
        let Ok(tab) = self.tab(space) else { return String::new() };
        tab.extent_estimate()
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
    /// header is a table of offsets weighs its pointers against. `name` is
    /// the file's name, asked only when the bytes fit two bundled formats
    /// that share a magic, as `.shp` and `.shx` do.
    pub fn sniff_template(&self, head: &[u8], file_len: f64, name: &str) -> String {
        formats::sniff_named(head, file_len as u64, name).unwrap_or("").to_string()
    }

    /// The longest file worth reading whole to ask `pickle_is_familiar` about.
    pub fn pickle_familiar_limit(&self) -> f64 {
        formats::pickle::FAMILIAR_MOST_BYTES as f64
    }

    /// Whether the whole of a pickle matches a Familiar Pickle Form. Asked
    /// about a file `sniff_template` called `pickle` from a window shorter
    /// than the file, which is every pickle holding an array of any size.
    pub fn pickle_is_familiar(&self, whole: &[u8]) -> bool {
        formats::pickle::is_familiar(whole)
    }

    /// How many trailing bytes `sniff_template_ends` wants: enough for a ZIP's
    /// end record, its comment and a central directory of some thousands of
    /// entries.
    pub fn sniff_tail_window(&self) -> f64 {
        formats::SNIFF_TAIL_WINDOW as f64
    }

    /// The same question asked of the leading bytes and the trailing ones, for
    /// a file the leading bytes alone call a ZIP: its central directory names
    /// every entry, and a BP5 dataset or a Zarr store whose files sit behind a
    /// large data file is told by those names. `tail` is the last bytes of the
    /// file.
    pub fn sniff_template_ends(&self, head: &[u8], tail: &[u8], file_len: f64) -> String {
        formats::sniff_ends(head, tail, file_len as u64).unwrap_or("").to_string()
    }

    /// What a BGZF file holds, from the front of its first block in these
    /// leading bytes: `bam`, `csi`, `vcf`, `bed`, `fasta` or `text`, or "" when
    /// that cannot be told. See `formats::bgzf_contents`.
    pub fn bgzf_contents(&self, head: &[u8]) -> String {
        formats::bgzf_contents(head).unwrap_or("").to_string()
    }

    /// Select a template by name; "" clears it. Returns false if unknown.
    ///
    /// A name starting `ksy:` is one of the bundled Kaitai Struct formats and
    /// one starting `hexpat:` is one of the bundled ImHex patterns, converted
    /// here and now. The conversion report is kept beside the template, so
    /// [`ksy_report`](Self::ksy_report) and
    /// [`hexpat_report`](Self::hexpat_report) answer for a bundled format
    /// exactly as they do for a description the reader pasted in: a bundled
    /// format may have gaps, and they are only honest if they are visible.
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
        sh.census = None;
        sh.template = name.to_string();
        sh.ksy_report = String::new();
        sh.hexpat_report = String::new();
        if name.is_empty() {
            sh.eval = None;
            return true;
        }
        let template = if let Some(id) = name.strip_prefix(qubero_core::ksy::bundled::PREFIX) {
            match qubero_core::ksy::bundled::template(id) {
                Some(Ok(converted)) => {
                    sh.ksy_report =
                        serde_json::to_string(&ksy_report_dto(&converted.report, &converted.template.name)).unwrap_or_default();
                    Some(converted.template)
                }
                // A bundled description that no longer converts is a broken
                // build, not a format the reader chose wrongly; there is
                // nothing to select and nothing useful to say here about why.
                Some(Err(_)) | None => None,
            }
        } else if let Some(id) = name.strip_prefix(qubero_core::hexpat::bundled::PREFIX) {
            match qubero_core::hexpat::bundled::template(id) {
                Some(Ok(converted)) => {
                    sh.hexpat_report = serde_json::to_string(&hexpat_report_dto(
                        &converted.report,
                        &converted.template.name,
                        Vec::new(),
                    ))
                    .unwrap_or_default();
                    Some(converted.template)
                }
                Some(Err(_)) | None => None,
            }
        } else {
            formats::builtin(name)
        };
        match template {
            Some(t) => {
                let mut e = Evaluator::new(t);
                e.set_slice(Some(WORK_SLICE));
                sh.eval = Some(e);
                true
            }
            None => false,
        }
    }

    /// Convert a Kaitai Struct `.ksy` and read the file with what comes out.
    ///
    /// `imports_json` is a JSON object mapping an import name to the text of
    /// that `.ksy`, for a format whose `meta/imports` names others; `{}` where
    /// it imports nothing. What comes back is the usual reply envelope: `ok`
    /// with the conversion report, or `error` with the path in the `.ksy` and
    /// what was wrong with it.
    ///
    /// The report is worth reading even on success. A `.ksy` says things the
    /// IR cannot, and each of those is a gap in the report with the path, the
    /// text it came from and the reason; the field is left as bytes rather
    /// than guessed at. See [`ksy_report`](Self::ksy_report).
    pub fn set_ksy_template(&mut self, text: &str, imports_json: &str) -> String {
        let map = match ksy_imports(imports_json) {
            Ok(map) => map,
            Err(message) => {
                return serde_json::to_string(&Reply::<KsyReportDto>::Error { message }).unwrap_or_default();
            }
        };
        let imports = PastedImports(qubero_core::ksy::MapImports(map));
        let converted = match qubero_core::ksy::convert(text, &imports) {
            Ok(converted) => converted,
            Err(e) => {
                return serde_json::to_string(&Reply::<KsyReportDto>::Error { message: e.to_string() })
                    .unwrap_or_default();
            }
        };
        // A different template may not have the stream a space came from; see
        // `set_template`, which throws the same working away for the same
        // reasons.
        self.forget_spaces();
        let report = ksy_report_dto(&converted.report, &converted.template.name);
        let sh = self.sm();
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
        sh.kinds = None;
        sh.census = None;
        sh.template = converted.template.name.clone();
        let mut e = Evaluator::new(converted.template);
        e.set_slice(Some(WORK_SLICE));
        sh.eval = Some(e);
        sh.ksy_report = serde_json::to_string(&report).unwrap_or_default();
        sh.hexpat_report = String::new();
        serde_json::to_string(&Reply::Ok { node: report, wanted: Vec::new() }).unwrap_or_default()
    }

    /// The report from the last `.ksy` converted into this space, as JSON.
    /// Empty when the template in use did not come from one.
    pub fn ksy_report(&mut self, space: u32) -> String {
        self.open_again(space);
        self.at(space).ksy_report.clone()
    }

    /// A bundled format's `.ksy`, byte for byte, by its `meta/id` (the part of
    /// `ksy:png` after the colon). Empty for an id nothing here has.
    ///
    /// This is the file as it was copied from the Kaitai Struct library, which
    /// is what the converter panel shows a reader who asks to see the
    /// description behind a shipped format.
    pub fn bundled_ksy_text(&self, id: &str) -> String {
        qubero_core::ksy::bundled::find(id).map(|b| b.text.to_string()).unwrap_or_default()
    }

    /// Convert a `.ksy` and say what it became, without reading anything with
    /// it. The document keeps the template it had.
    ///
    /// This is what the converter panel calls as the text is typed: the reply
    /// carries the same report [`set_ksy_template`](Self::set_ksy_template)
    /// gives, plus the template written out as text, so a reader sees what a
    /// `.ksy` would produce before deciding to read the file with it.
    pub fn preview_ksy_template(&self, text: &str, imports_json: &str) -> String {
        let map = match ksy_imports(imports_json) {
            Ok(map) => map,
            Err(message) => {
                return serde_json::to_string(&Reply::<KsyPreviewDto>::Error { message }).unwrap_or_default();
            }
        };
        let imports = PastedImports(qubero_core::ksy::MapImports(map));
        let converted = match qubero_core::ksy::convert(text, &imports) {
            Ok(converted) => converted,
            Err(e) => {
                return serde_json::to_string(&Reply::<KsyPreviewDto>::Error { message: e.to_string() })
                    .unwrap_or_default();
            }
        };
        let node = KsyPreviewDto {
            report: ksy_report_dto(&converted.report, &converted.template.name),
            text: qubero_core::template_text::render(&converted.template),
        };
        serde_json::to_string(&Reply::Ok { node, wanted: Vec::new() }).unwrap_or_default()
    }

    /// Convert an ImHex pattern and read the file with what comes out.
    ///
    /// `includes_json` is a JSON object mapping an include path such as
    /// `std/mem.pat` to the text of that file, for a pattern that includes one
    /// Qubero does not have; `{}` where it needs none. `name` is what the
    /// template goes by afterwards, which is the file's name where it has one.
    ///
    /// What comes back is the usual reply envelope. `ok` carries the conversion
    /// report; `error` carries `line:col` and what was wrong there, plus the
    /// include paths the pattern asks for and nothing here can answer.
    ///
    /// The report is worth reading even on success. The ImHex pattern language
    /// has an imperative half that no template can hold, and every piece of it
    /// is a gap in the report with the line it came from; the field is left as
    /// bytes rather than guessed at. See [`hexpat_report`](Self::hexpat_report).
    pub fn set_hexpat_template(&mut self, text: &str, includes_json: &str, name: &str) -> String {
        let map = match ksy_imports(includes_json) {
            Ok(map) => map,
            Err(message) => {
                return serde_json::to_string(&Reply::<HexpatReportDto>::Error { message }).unwrap_or_default();
            }
        };
        let wanted = hexpat_wanted(text, &map);
        let includes = PastedIncludes::new(map);
        let name = hexpat_name(name);
        let converted = match qubero_core::hexpat::convert_named(&name, text, &includes) {
            Ok(converted) => converted,
            Err(e) => {
                let mut missing = wanted;
                for path in includes.missing() {
                    if !missing.contains(&path) {
                        missing.push(path);
                    }
                }
                let node = HexpatFailedDto { message: hexpat_error(&e, &name), missing };
                return serde_json::to_string(&HexpatFailedReply { status: "error", node }).unwrap_or_default();
            }
        };
        // A different template may not have the stream a space came from; see
        // `set_template`, which throws the same working away for the same
        // reasons.
        self.forget_spaces();
        let report = hexpat_report_dto(&converted.report, &converted.template.name, includes.missing());
        let sh = self.sm();
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
        sh.kinds = None;
        sh.census = None;
        sh.template = converted.template.name.clone();
        let mut e = Evaluator::new(converted.template);
        e.set_slice(Some(WORK_SLICE));
        sh.eval = Some(e);
        sh.ksy_report = String::new();
        sh.hexpat_report = serde_json::to_string(&report).unwrap_or_default();
        serde_json::to_string(&Reply::Ok { node: report, wanted: Vec::new() }).unwrap_or_default()
    }

    /// The report from the last ImHex pattern converted into this space, as
    /// JSON. Empty when the template in use did not come from one.
    pub fn hexpat_report(&mut self, space: u32) -> String {
        self.open_again(space);
        self.at(space).hexpat_report.clone()
    }

    /// A bundled pattern's `.hexpat`, byte for byte, by its id (the part of
    /// `hexpat:vhd` after the colon). Empty for an id nothing here has.
    ///
    /// This is the file as it was copied from ImHex-Patterns, which is what the
    /// converter panel shows a reader who asks to see the pattern behind a
    /// shipped format.
    pub fn bundled_hexpat_text(&self, id: &str) -> String {
        qubero_core::hexpat::bundled::find(id).map(|b| b.text.to_string()).unwrap_or_default()
    }

    /// Convert an ImHex pattern and say what it became, without reading
    /// anything with it. The document keeps the template it had.
    ///
    /// This is what the converter panel calls as the text is typed: the reply
    /// carries the same report [`set_hexpat_template`](Self::set_hexpat_template)
    /// gives, plus the template written out as text.
    pub fn preview_hexpat_template(&self, text: &str, includes_json: &str, name: &str) -> String {
        let map = match ksy_imports(includes_json) {
            Ok(map) => map,
            Err(message) => {
                return serde_json::to_string(&Reply::<HexpatPreviewDto>::Error { message }).unwrap_or_default();
            }
        };
        let wanted = hexpat_wanted(text, &map);
        let includes = PastedIncludes::new(map);
        let name = hexpat_name(name);
        let converted = match qubero_core::hexpat::convert_named(&name, text, &includes) {
            Ok(converted) => converted,
            Err(e) => {
                let mut missing = wanted;
                for path in includes.missing() {
                    if !missing.contains(&path) {
                        missing.push(path);
                    }
                }
                let node = HexpatFailedDto { message: hexpat_error(&e, &name), missing };
                return serde_json::to_string(&HexpatFailedReply { status: "error", node }).unwrap_or_default();
            }
        };
        let node = HexpatPreviewDto {
            report: hexpat_report_dto(&converted.report, &converted.template.name, includes.missing()),
            text: qubero_core::template_text::render(&converted.template),
        };
        serde_json::to_string(&Reply::Ok { node, wanted: Vec::new() }).unwrap_or_default()
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
        // A signature template has no stream for a space to have come from;
        // see `set_template`.
        self.forget_spaces();
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
        sh.census = None;
        sh.template = String::new();
        // A signature template came from a `file(1)` rule and from no format
        // description, so the reports of the last one are about another
        // template and would have the panel and the overview note talking
        // about a template nothing is reading the file with.
        sh.ksy_report = String::new();
        sh.hexpat_report = String::new();
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
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let at = (at_bits >= 0.0).then(|| at_bits as u64);
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.explain(&p, at).map(explain_dto))
    }

    /// Which fields settled the shape of the one at `path`, and where this one
    /// points if it holds an offset. JSON, in the same reply shape as the rest;
    /// usually an empty list, since most fields are placed and sized outright.
    pub fn origins(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.origins(&p).map(|v| v.into_iter().map(origin_dto).collect::<Vec<_>>()))
    }

    /// How the field at `path` was placed and how it was sized, in one word
    /// each. JSON, in the same reply shape as the rest.
    ///
    /// Answered for every field, which is what tells it from `origins`: a `u32`
    /// in a header has no origins and is still somewhere for a reason, and the
    /// reason is that the field in front of it ended there.
    pub fn shape(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.shape(&p).map(|s| ShapeDto { placed: s.placed.as_str(), sized: s.sized.as_str() }))
    }

    /// Which part of a stream joined from several runs the field at `path`
    /// starts in, or null for a field that is not inside one. JSON, in the
    /// same reply shape as the rest.
    ///
    /// A PDB stream in pieces and a BAM read through its BGZF blocks are the
    /// two such streams. What comes back names the run the field's first byte
    /// is kept in, which is a field of the file with a place to go to, and how
    /// far into what that run gives the byte is; for a BGZF block, the virtual
    /// offset an index would name the byte by. Nothing is unpacked to answer.
    pub fn part_of(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.part_of(&p).map(|h| h.map(stitched_part_dto)))
    }

    /// The same answer for byte `byte` of a joined stream opened as a tab of
    /// its own, asked by the byte rather than by a field. Null for a tab that
    /// was unpacked from one run, and for the file.
    ///
    /// The tab's own reading knows nothing of the runs, since its bytes are
    /// its own. The reading that opened it does, under the node that joined them, and
    /// byte `byte` of the tab is byte `byte` of the stream there: the parts are
    /// laid end to end in the same order and cut at the same length. The node
    /// the stream holds says which space that is.
    pub fn part_at(&mut self, space: u32, byte: f64) -> String {
        let none = || reply(Ok(None::<StitchedPartDto>));
        if let Err(err) = self.ensure_open(space) {
            return reply::<Option<StitchedPartDto>>(Err(err));
        }
        if space == 0 || !self.space_joined(space).0 {
            return none();
        }
        let Some((home, mut root)) = self.sheets.get(space as usize).map(|sh| (sh.home, sh.origin.clone())) else { return none() };
        root.push(0);
        let at = &mut self.sheets[home];
        let Some(e) = &mut at.eval else { return none() };
        e.begin_slice();
        let hit = e.node(&at.doc, &root).and_then(|n| e.part_of(&at.doc, n.space, byte as u64));
        reply(hit.map(|h| h.map(stitched_part_dto)))
    }

    /// What the field at `path` checks, or null when it checks nothing. JSON,
    /// in the same reply shape as the rest.
    ///
    /// Cheap, and asked of any field: no bytes of the covered run are read, so
    /// a panel can ask on every move of the cursor and decide from
    /// `covered_bytes` whether to take the sum without being asked. Taking it
    /// is `run_check`.
    pub fn check_of(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.check_of(&p).map(|c| {
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

    /// The moment the field at `path` means, or null when it means none. JSON,
    /// in the same reply shape as the rest.
    ///
    /// Cheap, and asked of any field: the field's own bytes are read, which a
    /// panel showing its value has already paid for, and nothing else is. The
    /// stored number is not in here, because it is already on the value row and
    /// this is an addition to it rather than a replacement for it.
    pub fn time_of(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match self.tab(space) {
            Err(why) => why,
            Ok(mut tab) => {
                tab.ev.begin_slice();
                reply(tab.time_of(&p).map(|t| {
                    t.map(|t| {
                        let (state, unix_seconds, nanos) = match t.moment {
                            Moment::At { unix_seconds, nanos } => ("at", Some(unix_seconds as f64), Some(nanos as f64)),
                            Moment::LeapSecond { unix_seconds, nanos } => ("leap", Some(unix_seconds as f64), Some(nanos as f64)),
                            Moment::Unset => ("unset", None, None),
                            Moment::Impossible => ("impossible", None, None),
                        };
                        let note = t.note.map(|n| match n {
                            TimeNote::PastLeapSecondTable => "past_leap_second_table",
                            TimeNote::BeforeLeapSeconds => "before_leap_seconds",
                        });
                        TimeDto {
                            state,
                            unix_seconds,
                            nanos,
                            note,
                            leap_table_expires: matches!(t.note, Some(TimeNote::PastLeapSecondTable))
                                .then_some(leap_seconds::EXPIRES as f64),
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

    /// The rows `from` up to `to` of the pandas frame at `path`, each as one
    /// cell a column. JSON, in the same reply shape as the rest.
    ///
    /// A frame's cells are not nodes of the tree: its values are in blocks
    /// written the other way up from the frame, a categorical column is codes
    /// into another array, and a counted index is not written down at all. So
    /// the reading is the core's and the view asks for the rows it is showing.
    pub fn pickle_cells(&mut self, space: u32, path: &[u32], from: f64, to: f64) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let (from, to) = (from.max(0.0) as u64, to.max(0.0) as u64);
        match self.tab(space) {
            Err(why) => why,
            Ok(mut tab) => {
                tab.ev.begin_slice();
                reply(tab.pickle_cells(&p, from, to).map(|rows| {
                    rows.into_iter()
                        .map(|row| {
                            row.into_iter()
                                .map(|cell| match cell {
                                    None => FrameCellDto { text: String::new(), kind: "absent" },
                                    Some(v) => {
                                        let (kind, text, _) = shown(&v);
                                        FrameCellDto { text, kind }
                                    }
                                })
                                .collect::<Vec<FrameCellDto>>()
                        })
                        .collect::<Vec<Vec<FrameCellDto>>>()
                }))
            }
        }
    }

    /// What table the field at `path` reads as, or null when the template
    /// makes no such claim about it. JSON, in the same reply shape as the
    /// rest.
    ///
    /// Asked once when a table is opened, not on the way past: it reads the
    /// fields the shape names, which is a handful of numbers somewhere else in
    /// the file. `NodeDto::table` is the cheap answer to whether there is one
    /// at all. A part of the shape this file does not answer comes back null
    /// rather than guessed; bytes that have not arrived answer pending like
    /// every other call.
    pub fn table_shape(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match self.tab(space) {
            Err(why) => why,
            Ok(mut tab) => {
                tab.ev.begin_slice();
                reply(tab.table_shape(&p).map(|t| {
                    t.map(|t| TableShapeDto {
                        columns: t.columns.map(|c| c as f64),
                        names: t.names,
                        units: t.units,
                        column_word: t.column_word,
                        row_word: t.row_word,
                        rate: t.rate.map(|r| r as f64),
                        facts: t
                            .facts
                            .into_iter()
                            .map(|o| TableFactDto {
                                label: o.label,
                                path: o.path.into_iter().map(|x| x as f64).collect(),
                                value: o.value,
                            })
                            .collect(),
                        cells: t.cells.map(|c| match c {
                            qubero_core::template::Cells::Named { row, cell, value } => CellsDto::Named {
                                row: row.to_string(),
                                cell: cell.to_string(),
                                value: value.map(|v| v.to_string()),
                            },
                            qubero_core::template::Cells::Computed { rows } => CellsDto::Computed { rows: rows as f64 },
                        }),
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
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.run_check(&p).map(|v| v.map(|v| VerdictDto { computed: v.computed, stored: v.stored, ok: v.ok })))
    }

    /// The same question asked of every field under `path` at once: the nodes
    /// of the subtree and every connection between two of them, ready to be
    /// laid out. JSON, in the same reply shape as the rest.
    ///
    /// `limit` caps the nodes. The walk is breadth-first, so what a cap keeps
    /// is the top of the format rather than one deep spine of it, and the
    /// answer says how many nodes it left out.
    pub fn graph(&mut self, space: u32, path: &[u32], limit: u32) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.graph(&p, limit as usize).map(graph_dto))
    }

    /// The template as boxes and arrows: one box per type, one row per field,
    /// and one edge per connection between two of them. JSON, in the same reply
    /// shape as the rest.
    ///
    /// About the format rather than about the file. Nothing here is read from
    /// the document, no node is resolved, and the answer is the same for every
    /// file the same template opens. `space` picks which template, since an
    /// unpacked stream is read by one of its own.
    pub fn template_diagram(&mut self, space: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        match (&sh.eval, &sh.read_as) {
            (Some(e), _) => reply(Ok(diagram_dto(qubero_core::eval::diagram(e.template())))),
            (None, Some(t)) => reply(Ok(diagram_dto(qubero_core::eval::diagram(t)))),
            (None, None) => reply::<DiagramDto>(Err(EvalError::Failed("no template".into()))),
        }
    }

    /// The open file's nodes counted against the diagram's boxes: how many of
    /// each the file holds, which rows they stood on, and the path to the first
    /// of each. JSON, in the same reply shape as the rest.
    ///
    /// One go of a count kept between calls, so each call carries on from the
    /// last and answers with everything counted so far. `limit` caps the nodes
    /// walked in all, and raising it carries a capped count on. The node's
    /// `state` says whether to ask again; bytes the count is waiting on come
    /// back as `wanted`, so they are fetched and the change they make asks
    /// again. An edit or a new template throws the count away, and the next
    /// call starts it over.
    pub fn diagram_census(&mut self, space: u32, limit: u32) -> String {
        // Taken out of the sheet for the length of the call, the way the kind
        // walk is: see `kind_totals_step`.
        if let Err(why) = self.tab(space) {
            return why;
        }
        let sh = self.sm();
        let len = sh.doc.len_bits();
        let mut census = sh.census.take();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        if !matches!(&census, Some(w) if w.file_bits() == len) {
            census = Some(tab.census_walk(len));
        }
        let walk = census.as_mut().expect("just built");
        tab.ev.begin_slice();
        let out = tab.census_step(walk, limit as usize);
        let reached = (tab.ev.reached_bits() / 8) as f64;
        let wanted = wanted(tab.ev);
        self.sm().census = census;
        reply_with(out.map(census_dto), reached, wanted)
    }

    /// The relationships behind the shape of the field at `path`, written out:
    /// the expression as the template holds it, the same with every field's
    /// value in its place, and what it comes to. JSON, in the same reply shape
    /// as the rest. Empty for a field the template placed and sized outright,
    /// and for one whose expression has no reading in that notation.
    pub fn relations(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.relations(&p).map(|v| {
            v.into_iter()
                .map(|r| RelationDto {
                    role: r.role.as_str(),
                    written: r.written,
                    template: r.template,
                    substituted: r.substituted,
                    result: r.result,
                })
                .collect::<Vec<_>>()
        }))
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
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        let r = tab.node(&p).map(dto);
        reply_with(r, (tab.ev.reached_bits() / 8) as f64, wanted(tab.ev))
    }

    /// Same envelope as `template_node`, with `node` being an array of children.
    pub fn template_children(&mut self, space: u32, path: &[u32], from: f64, to: f64) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        let r = tab.children(&p, from as u64, to as u64).map(|v| v.into_iter().map(dto).collect::<Vec<NodeDto>>());
        reply_with(r, (tab.ev.reached_bits() / 8) as f64, wanted(tab.ev))
    }

    /// The elements of the folded run at `path` whose bits overlap
    /// `from_bit..to_bit`, at most `max` of them, for the value table beside
    /// the bytes: {status:"ok",node:[cell,..]}. Same envelope as
    /// `template_children`. `path` is what a span with a count carries, or a
    /// block of a decoded stream's trace.
    pub fn run_cells(&mut self, space: u32, path: &[u32], from_bit: f64, to_bit: f64, max: u32) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        let r = tab
            .run_cells(&p, from_bit as u64, to_bit as u64, max as usize)
            .map(|v| v.into_iter().map(cell_dto).collect::<Vec<CellDto>>());
        reply_with(r, (tab.ev.reached_bits() / 8) as f64, wanted(tab.ev))
    }

    /// Whole text of a text field, decoded in its own encoding:
    /// {status:"ok",node:{text,truncated}}.
    pub fn field_text(&mut self, space: u32, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.text_value(&p).map(|(text, truncated)| TextDto { text, truncated }))
    }

    /// The first `limit` bytes of a field, read in whatever address space the
    /// field is in: {status:"ok",node:{bytes:[..],truncated}}. Use this rather
    /// than `read_bits` at the node's offset, which is the file and is the
    /// wrong bytes for anything inside a decoded stream.
    pub fn field_bytes(&mut self, space: u32, path: &[u32], limit: u32) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.field_bytes(&p, u64::from(limit)).map(|(bytes, truncated)| BytesDto { bytes, truncated }))
    }

    /// Every field between two bit offsets, for the annotation column:
    /// {status:"ok",node:[span,..]}. `max` caps how many come back.
    pub fn spans(&mut self, space: u32, from_bit: f64, to_bit: f64, max: u32) -> String {
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        let found = match tab.spans(from_bit as u64, to_bit as u64, max as usize) {
            Ok(v) => v,
            Err(err) => return reply::<Vec<SpanDto>>(Err(err)),
        };
        // Named through the tables of the program the sheet reads, which a
        // stream read where it was declared does not: its rows stay as the
        // template reads them.
        let named = self.name_instructions(found);
        reply(Ok(named))
    }

    /// Whether the space is read as an HDF5 file, whole or inside another
    /// format: {status:"ok",node:true}. What every HDF5 panel is offered on,
    /// rather than the template's name, since `mat` reads a level 5 file with
    /// no HDF5 in it as well as a level 7.3 file that is one. See
    /// `h5ad::holds_hdf5`.
    ///
    /// Pending while the few bytes near the front that decide it are still to
    /// come, so the host asks for them and asks again when they land.
    pub fn holds_hdf5(&mut self, space: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        // A stream read where it was declared has no reading of its own for a
        // superblock to be the root of, and asked of the file's this would
        // answer for the file.
        let Some(e) = &mut sh.eval else { return reply(Ok(false)) };
        e.begin_slice();
        reply(qubero_core::formats::h5ad::holds_hdf5(e, &sh.doc))
    }

    /// What an HDF5 file holds, read in the file's own terms rather than the
    /// template's: {status:"ok",node:{objects,..}}. Empty for a file that holds
    /// no HDF5, since nothing else here has a group tree to walk.
    pub fn contents(&mut self, space: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        let Some(e) = &mut sh.eval else {
            return reply::<ContentsDto>(Err(EvalError::Failed("no template".into())));
        };
        e.begin_slice();
        match qubero_core::formats::h5ad::holds_hdf5(e, &sh.doc) {
            Ok(true) => {}
            Ok(false) => {
                return reply(Ok(ContentsDto {
                    objects: Vec::new(),
                    total: 0.0,
                    anndata: false,
                    encoding: String::new(),
                    rows: 0.0,
                    columns: 0.0,
                }))
            }
            Err(err) => return reply::<ContentsDto>(Err(err)),
        }
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

    /// The HDF5 B-tree the field at `path` belongs to, of either version,
    /// walked into the shape it has in the file:
    /// {status:"ok",node:{job,version,nodes,..}}, or a null node where the file
    /// has no tree to answer with.
    ///
    /// `path` is where the cursor is, which is usually not a node of a tree.
    /// The core works out which tree that means: the one the cursor is inside,
    /// else the one the object header it is inside names, else the root
    /// group's, else the first tree under the root group. `limit` caps the
    /// nodes walked, and the answer says how many children it left out.
    pub fn btree(&mut self, space: u32, path: &[u32], limit: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let Some(e) = &mut sh.eval else {
            return reply::<Option<TreeDto>>(Err(EvalError::Failed("no template".into())));
        };
        e.begin_slice();
        match qubero_core::formats::h5ad::holds_hdf5(e, &sh.doc) {
            Ok(true) => {}
            Ok(false) => return reply(Ok(None::<TreeDto>)),
            Err(err) => return reply::<Option<TreeDto>>(Err(err)),
        }
        let found = match qubero_core::formats::hdf5_tree::tree(e, &sh.doc, &p, limit as usize) {
            Ok(t) => t,
            Err(err) => return reply::<Option<TreeDto>>(Err(err)),
        };
        reply(Ok(found.map(|t| {
            use qubero_core::formats::hdf5_tree::{Job, Kind, Records, NO_PARENT};
            TreeDto {
                job: match t.job {
                    Job::Group => "group",
                    Job::Chunk => "chunk",
                    Job::Other => "other",
                },
                version: f64::from(t.version),
                record_type: f64::from(t.record_type),
                record_type_name: t.record_type_name,
                records: match t.records {
                    Records::Read => "read",
                    Records::Unread => "unread",
                    Records::Unknown => "unknown",
                },
                records_total: t.records_total as f64,
                omitted: t.omitted as f64,
                coords: t.coords as f64,
                coords_pad: t.coords_pad,
                nodes: t
                    .nodes
                    .into_iter()
                    .map(|n| TreeNodeDto {
                        path: n.path,
                        parent: if n.parent == NO_PARENT { -1.0 } else { n.parent as f64 },
                        kind: match n.kind {
                            Kind::Index => "index",
                            Kind::LinkTable => "links",
                            Kind::Leaf => "leaf",
                        },
                        sign: n.sign,
                        address: n.address as f64,
                        size_bits: n.size_bits as f64,
                        level: n.level as f64,
                        depth: n.depth as f64,
                        entries: n.entries as f64,
                        first_key: n.first_key,
                        last_key: n.last_key,
                        truncated: n.truncated,
                        first_entry_bits: n.first_entry_bits as f64,
                        entry_bits: n.entry_bits as f64,
                    })
                    .collect(),
            }
        })))
    }

    /// Named ELF sections and a bounded prefix of its symbols. The semantic
    /// pass is cached because resolving names crosses several linked tables.
    pub fn elf_contents(&mut self, space: u32, symbol_limit: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        // A stream read where it was declared carries the file's template name
        // and is not a program: the tables are the file's.
        if (sh.template != "elf" && sh.template != "bpf") || sh.view.is_some() {
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
        let sections = program.sections.iter().map(|section| ElfSectionDto {
            path: section.path.clone(),
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

    /// What a CERN ROOT file holds, read beside the template: the class
    /// descriptions out of its `StreamerInfo` record, and every tree with its
    /// branches, leaves and baskets.
    ///
    /// A channel of its own rather than a second use of `contents`, which is
    /// HDF5's: what crosses for an HDF5 object is a dataspace and a storage
    /// layout, and what crosses for a ROOT tree is a class description and a
    /// list of offsets nothing in the template placed. The two have no shape in
    /// common but the idea.
    pub fn root_contents(&mut self, space: u32) -> String {
        if let Err(why) = self.go(space) {
            return why;
        }
        let sh = self.sm();
        // The same for a record of a ROOT file opened as a tab, which is not a
        // ROOT file.
        if sh.template != "root" || sh.view.is_some() {
            return reply::<RootContentsDto>(Err(EvalError::Failed("not a ROOT template".into())));
        }
        let Some(e) = &mut sh.eval else {
            return reply::<RootContentsDto>(Err(EvalError::Failed("no template".into())));
        };
        // The walk reads whole records and decodes them; charging it the
        // listing's slice would stop it part way through a tree with nothing
        // to resume from. It is bounded instead by the limits in `root_tree`.
        e.set_slice(None);
        let found = qubero_core::formats::root_tree::contents(e, &sh.doc);
        e.set_slice(Some(WORK_SLICE));
        let found = match found {
            Ok(c) => c,
            Err(err) => return reply::<RootContentsDto>(Err(err)),
        };
        reply(Ok(RootContentsDto {
            schema_path: found.schema_path,
            trouble: found.trouble.unwrap_or_default(),
            tree_total: found.tree_total as f64,
            classes: found
                .classes
                .into_iter()
                .map(|c| RootClassDto {
                    name: c.name,
                    version: c.version as f64,
                    checksum: c.checksum as f64,
                    members: c
                        .members
                        .into_iter()
                        .map(|m| RootMemberDto {
                            name: m.name,
                            type_name: m.type_name,
                            code: m.code as f64,
                            size: m.size as f64,
                            dims: m.dims.into_iter().map(|d| d as f64).collect(),
                            base: m.base,
                            comment: m.comment,
                        })
                        .collect(),
                })
                .collect(),
            trees: found
                .trees
                .into_iter()
                .map(|t| RootTreeDto {
                    path: t.path,
                    name: t.name,
                    title: t.title,
                    entries: t.entries as f64,
                    address: t.at as f64,
                    branch_total: t.branch_total as f64,
                    trouble: t.trouble.unwrap_or_default(),
                    branches: t
                        .branches
                        .into_iter()
                        .map(|b| {
                            use qubero_core::formats::root_tree::Reading;
                            // The words a reader sees are the host's business.
                            // What crosses is the shape of one value and, where
                            // there is no such shape, the sentence saying why.
                            let (width, per_entry, floating, unsigned, unread) = match b.reading {
                                Reading::Fixed { width, per_entry, floating, unsigned } => {
                                    (width as f64, per_entry as f64, floating, unsigned, String::new())
                                }
                                Reading::Not(why) => (0.0, 0.0, false, false, why),
                            };
                            RootBranchDto {
                                name: b.name,
                                title: b.title,
                                class: b.class,
                                depth: b.depth as f64,
                                entries: b.entries as f64,
                                total_bytes: b.total_bytes as f64,
                                zip_bytes: b.zip_bytes as f64,
                                basket_total: b.basket_total as f64,
                                width,
                                per_entry,
                                floating,
                                unsigned,
                                unread,
                                leaves: b
                                    .leaves
                                    .into_iter()
                                    .map(|l| RootLeafDto {
                                        name: l.name,
                                        class: l.class,
                                        len: l.len as f64,
                                        width: l.width as f64,
                                        unsigned: l.unsigned,
                                        counted_by: l.counted_by,
                                    })
                                    .collect(),
                                baskets: b
                                    .baskets
                                    .into_iter()
                                    .map(|k| RootBasketDto {
                                        address: k.at as f64,
                                        bytes: k.bytes as f64,
                                        first_entry: k.first_entry as f64,
                                        entries: k.entries as f64,
                                    })
                                    .collect(),
                            }
                        })
                        .collect(),
                })
                .collect(),
        }))
    }

    /// The primary ISO 9660 volume and its root-directory pointer.
    pub fn iso_volume(&mut self, space: u32) -> String {
        self.open_again(space);
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
    pub fn iso_directory(&mut self, space: u32, extent: f64, size: f64, block_size: f64, limit: u32, joliet: bool) -> String {
        self.open_again(space);
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
        let mut tab = match self.tab(space) {
            Ok(tab) => tab,
            Err(why) => return why,
        };
        tab.ev.begin_slice();
        reply(tab.locate(bit as u64))
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
        if let Err(why) = self.go(space) {
            return why;
        }
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
        if let Err(why) = self.go(space) {
            return why;
        }
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

    pub fn has_chunk(&mut self, space: u32, chunk: f64) -> bool {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.source().has(chunk as u64)
    }

    pub fn chunk_size(&self) -> u32 {
        let sh = &self.sheets[0];
        sh.doc.source().chunk_size() as u32
    }

    pub fn len_bytes(&mut self, space: u32) -> f64 {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.len_bytes() as f64
    }

    pub fn len_bits(&mut self, space: u32) -> f64 {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.len_bits() as f64
    }

    /// Fill `out` with document bytes from `at`. Returns the chunk indices that
    /// were not loaded (those bytes are zero). Empty list means the read is complete.
    ///
    /// A tab whose stream has to be opened again first, and whose reopen waits
    /// on chunks of the file, answers with those: a tab's chunks are always
    /// the file's, and it is asked again once they are fed.
    pub fn read_bytes(&mut self, space: u32, at: f64, out: &mut [u8]) -> Vec<f64> {
        if let Err(waits) = self.ensure_open(space) {
            return chunks_of(waits);
        }
        let sh = self.at(space);
        sh.doc.read_bytes(at as u64, out).into_iter().map(|m| m.chunk as f64).collect()
    }

    pub fn read_bits(&mut self, space: u32, at_bit: f64, n: f64, out: &mut [u8]) -> Vec<f64> {
        if let Err(waits) = self.ensure_open(space) {
            return chunks_of(waits);
        }
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
    pub fn can_undo(&mut self, space: u32) -> bool {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.can_undo()
    }
    pub fn can_redo(&mut self, space: u32) -> bool {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.can_redo()
    }
    pub fn is_modified(&mut self, space: u32) -> bool {
        self.open_again(space);
        let sh = self.at(space);
        sh.doc.is_modified()
    }
    pub fn piece_count(&mut self, space: u32) -> u32 {
        self.open_again(space);
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
    pub fn text_window(&mut self, space: u32, encoding: &str, from: f64, want: u32) -> String {
        self.open_again(space);
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
    pub fn text_index(&mut self, space: u32, encoding: &str, from: f64, to: f64) -> Vec<f64> {
        self.open_again(space);
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
    pub fn text_back(&mut self, space: u32, encoding: &str, at: f64, lines: u32) -> String {
        self.open_again(space);
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
    pub fn strings_scan(&mut self, space: u32, from: f64, want: u32, min_chars: u32, encodings: &str) -> String {
        self.open_again(space);
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
    pub fn selection_text(&mut self, space: u32, at_byte: f64, len: f64, first: &str, page_a: &str, page_b: &str) -> String {
        self.open_again(space);
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
    pub fn selection_literal(&mut self, space: u32, at_byte: f64, len: f64, lang: &str) -> String {
        self.open_again(space);
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

/// The characters the hex view's text column writes, one per byte value, as a
/// string 256 characters long. U+FFFD stands where the column has nothing to
/// show for a byte and writes its stand-in instead.
///
/// A whole table rather than a character at a time: the column is redrawn a
/// few hundred cells at a go, and a boundary crossing per cell would put the
/// cost of the choice on every frame instead of on the choosing. The caller
/// holds the answer until the reader picks another column.
///
/// An empty string comes back for a name the core does not know, which is what
/// a caller with a stale choice saved gets, and is its cue to fall back.
#[wasm_bindgen]
pub fn glyph_column(name: &str) -> String {
    qubero_core::hexdump::glyphs::Glyphs::by_name(name).map(|g| g.column()).unwrap_or_default()
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
