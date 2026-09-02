//! wasm-bindgen surface over `qubero-core`.
//!
//! Offsets cross the boundary as `f64` (exact up to 2^53, far past any file size)
//! to avoid BigInt friction on the JS side.

use qubero_core::eval::{Explain, OpenedDoc, Origin, Step as MapStep, StepKind};
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
}

impl Sheet {
    fn new(store: ChunkStore, origin: Vec<usize>) -> Sheet {
        Sheet {
            doc: Document::new(store),
            origin,
            eval: None,
            disasm: None,
            bpf: None,
            bpf_complete: false,
            ne: None,
            template: String::new(),
            scan: None,
            focus: None,
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

/// How many of a block's commonest byte values are worth naming. Enough to
/// show a block is mostly two or three values; past that the count is the
/// answer, not the list.
const COMMON_BYTES: usize = 5;

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

/// One relationship behind a field's shape, written both ways.
#[derive(Serialize)]
struct RelationDto {
    /// "length" | "count" | "type" | "value"
    role: &'static str,
    /// The expression as the template writes it.
    written: String,
    /// The same with every field's value in its place.
    substituted: String,
    /// What it comes to.
    result: String,
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
        line: s.line,
        sample: s.sample,
        parts: s.parts.into_iter().map(span_part_dto).collect(),
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
        edit_text,
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
        space: n.space as f64,
        refused: n.refused,
        decoded: n.decoded,
        space_root: n.space_root,
    }
}

/// Chunks the evaluator answered without, as the host counts them.
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
    /// `literal`, `match`, `stored`, `block`, `header`, `table` or `opaque`.
    kind: &'static str,
    /// Set for a match only: how long it is and how far back it reaches.
    #[serde(skip_serializing_if = "Option::is_none")]
    len: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dist: Option<f64>,
}

/// The output bytes a stretch of input came to.
#[derive(Serialize)]
struct OutRangeDto {
    out_start: f64,
    out_end: f64,
}

fn step_dto(s: MapStep) -> MapStepDto {
    let (kind, len, dist) = match s.kind {
        StepKind::Literal => ("literal", None, None),
        StepKind::Match { len, dist } => ("match", Some(len as f64), Some(dist as f64)),
        StepKind::Stored => ("stored", None, None),
        StepKind::Block => ("block", None, None),
        StepKind::Header => ("header", None, None),
        StepKind::Table => ("table", None, None),
        StepKind::Opaque => ("opaque", None, None),
    };
    MapStepDto {
        in_start: s.in_bits.start as f64,
        in_end: s.in_bits.end as f64,
        out_start: s.out_bytes.start as f64,
        out_end: s.out_bytes.end as f64,
        kind,
        len,
        dist,
    }
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

    /// The reading in hand. A space id past the ones open falls back to the
    /// file rather than panicking: a tab left over from before an edit asks
    /// about a space that has been forgotten, and the honest answer to that is
    /// the file, not a trap.
    /// One space's reading, for a call that names it and asks nothing else of
    /// the editor. Same fallback as `sheet`.
    fn at(&self, space: u32) -> &Sheet {
        self.sheets.get(space as usize).unwrap_or(&self.sheets[0])
    }

    fn sheet(&self) -> &Sheet {
        self.sheets.get(self.live).unwrap_or(&self.sheets[0])
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
        self.sheets.truncate(1);
        self.live = 0;
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
        let sh = &mut self.sheets[0];
        let Some(e) = &mut sh.eval else {
            return reply::<SpaceDto>(Err(EvalError::Failed("no template".into())));
        };
        // Unpacking a run is not something to do by halves: it reads the whole
        // run and decodes it, and a half-decoded stream is not a document.
        e.set_slice(None);
        let opened = e.open_space_doc(&sh.doc, &p);
        e.set_slice(Some(WORK_SLICE));
        let doc = match opened {
            Ok(OpenedDoc::Opened(d)) => d,
            Ok(OpenedDoc::Refused(why)) => {
                return reply(Ok(SpaceDto { space: 0.0, template: String::new(), refused: Some(why.as_str().to_string()) }))
            }
            Err(err) => return reply::<SpaceDto>(Err(err)),
        };
        // The bytes are all here already, so the store keeps every chunk: a
        // space has no file behind it to fetch a missing one back from.
        let bytes = &doc.bytes;
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
        let mut sheet = Sheet::new(store, p);
        let doc_template = doc.template.name.clone();
        sheet.template = doc_template.clone();
        let mut ev = Evaluator::new(doc.template);
        ev.set_slice(Some(WORK_SLICE));
        sheet.eval = Some(ev);
        self.sheets.push(sheet);
        let template = doc_template;
        reply(Ok(SpaceDto { space: (self.sheets.len() - 1) as f64, template, refused: None }))
    }

    /// Which bits of the compressed run the byte at `byte` of `space` came
    /// from, and by which step: {status:"ok",node:{..}} or a null node when the
    /// codec's map does not reach that far.
    pub fn map_out(&mut self, space: u32, byte: f64) -> String {
        let Some(origin) = self.origin_of(space) else { return reply(Ok(None::<MapStepDto>)) };
        let sh = &self.sheets[0];
        let Some(e) = &sh.eval else { return reply(Ok(None::<MapStepDto>)) };
        reply(Ok(e.map_out(&origin, byte as u64).map(step_dto)))
    }

    /// Which bytes of `space` the bit at `bit` of the compressed run came to.
    pub fn map_in(&mut self, space: u32, bit: f64) -> String {
        let Some(origin) = self.origin_of(space) else { return reply(Ok(None::<OutRangeDto>)) };
        let sh = &self.sheets[0];
        let Some(e) = &sh.eval else { return reply(Ok(None::<OutRangeDto>)) };
        reply(Ok(e.map_in(&origin, bit as u64).map(|r| OutRangeDto {
            out_start: r.out_bytes.start as f64,
            out_end: r.out_bytes.end as f64,
        })))
    }

    /// The `Decoded` node a space was unpacked from, as a path in the file.
    /// Empty for space 0 and for a space that is no longer open.
    pub fn space_origin(&self, space: u32) -> Vec<u32> {
        self.origin_of(space).map_or(Vec::new(), |p| p.iter().map(|&x| x as u32).collect())
    }

    fn origin_of(&self, space: u32) -> Option<Vec<usize>> {
        let sh = self.sheets.get(space as usize)?;
        if sh.origin.is_empty() {
            return None;
        }
        Some(sh.origin.clone())
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
                    entropy,
                    entropy_max,
                    distinct: scan.distinct() as f64,
                    common,
                }))
            }
        }
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
        self.live = 0;
        let sh = self.sm();
        sh.disasm = None;
        sh.bpf = None;
        sh.bpf_complete = false;
        sh.ne = None;
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
        // says so in its own words; this is the door being locked behind it.
        if space != 0 {
            return reply::<WriteDto>(Err(EvalError::Failed("unpacked data is read-only".into())));
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
                sh.doc.overwrite_bits(w.offset_bits, &w.data, w.n_bits);
                self.changed_at(w.offset_bits);
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

    /// What a selected run of bytes says, read every way text can be read.
    ///
    /// Only a run of whole bytes lying together: a selection made over the bits
    /// in binary mode is not characters, which is the same reason the panel's
    /// byte-reversed rows only appear for whole bytes. `first` names the
    /// encoding to put at the front, which is whatever the text view is
    /// reading the file in.
    pub fn selection_text(&self, space: u32, at_byte: f64, len: f64, first: &str) -> String {
        let sh = self.at(space);
        let want = (len as u64).min(SELECTION_TEXT_LIMIT) as usize;
        let mut buf = vec![0u8; want];
        let missing = sh.doc.read_bytes(at_byte as u64, &mut buf);
        if !missing.is_empty() {
            return String::new();
        }
        let r = qubero_core::text::readings(&buf, named_encoding(first));
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
    use qubero_core::text::Settled;
    use qubero_core::Endian;
    Some(match name {
        "UTF-8" => Settled::Utf8,
        "ASCII" => Settled::Ascii,
        "Latin-1" => Settled::Latin1,
        "CP437" => Settled::Cp437,
        "UTF-16 LE" => Settled::Utf16(Endian::Little),
        "UTF-16 BE" => Settled::Utf16(Endian::Big),
        _ => return None,
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
