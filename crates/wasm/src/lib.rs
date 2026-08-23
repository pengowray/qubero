//! wasm-bindgen surface over `qubero-core`.
//!
//! Offsets cross the boundary as `f64` (exact up to 2^53, far past any file size)
//! to avoid BigInt friction on the JS side.

use qubero_core::diescript;
use qubero_core::eval::Explain;
use qubero_core::{formats, magicrule, ChunkStore, Document, EvalError, Evaluator, NodeInfo, RunKind, Span, Value};
use serde::Serialize;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Editor {
    doc: Document<ChunkStore>,
    eval: Option<Evaluator>,
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
}

#[derive(Serialize)]
struct TextDto {
    text: String,
    /// True when the field holds more than the editor will show.
    truncated: bool,
}

/// What a type permits. `kind` picks which of the rest is filled in.
#[derive(Serialize)]
struct ExplainDto {
    /// "magic" | "enum" | "flags" | "plain"
    kind: &'static str,
    /// The type's own name, for an enum or a flags field.
    name: String,
    /// Magic: the bytes the format requires, and the bytes that are there.
    expected: Vec<u8>,
    actual: Vec<u8>,
    /// Enum: every value it names, and the one in the file.
    cases: Vec<CaseDto>,
    current: f64,
    /// Enum: whether its numbers are read in hex.
    hex: bool,
    /// Flags: one entry per bit of the field, from bit 0 up.
    bits: Vec<BitDto>,
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

fn explain_dto(e: Explain) -> ExplainDto {
    let mut dto = ExplainDto {
        kind: "plain",
        name: String::new(),
        expected: Vec::new(),
        actual: Vec::new(),
        cases: Vec::new(),
        current: 0.0,
        hex: false,
        bits: Vec::new(),
    };
    match e {
        Explain::Plain => {}
        Explain::Magic { expected, actual } => {
            dto.kind = "magic";
            dto.expected = expected;
            dto.actual = actual;
        }
        Explain::Enum { name, hex, cases, current } => {
            dto.kind = "enum";
            dto.name = name;
            dto.hex = hex;
            dto.current = current as f64;
            dto.cases = cases.into_iter().map(|(value, name)| CaseDto { value: value as f64, name }).collect();
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
    Ok { node: T },
    #[serde(rename = "pending")]
    Pending { chunks: Vec<f64> },
    #[serde(rename = "error")]
    Error { message: String },
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
        Value::Str(s) => ("str", s.clone(), s.clone(), true),
        Value::Magic { ok } => {
            let s: String = if *ok { "matches".into() } else { "does not match".into() };
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
        composite: n.composite,
        editable: n.editable,
        value_bytes: n.value_bytes as f64,
        value_offset_bits: n.value_offset_bits as f64,
        read_as: n.read_as,
    }
}

fn reply<T: Serialize>(r: Result<T, EvalError>) -> String {
    let rep = match r {
        Ok(node) => Reply::Ok { node },
        Err(EvalError::Pending(m)) => Reply::Pending { chunks: m.into_iter().map(|m| m.chunk as f64).collect() },
        Err(EvalError::Failed(message)) => Reply::Error { message },
    };
    serde_json::to_string(&rep).unwrap_or_else(|e| format!("{{\"status\":\"error\",\"message\":{:?}}}", e.to_string()))
}

#[wasm_bindgen]
impl Editor {
    /// `len` is the original file length in bytes. Chunks of `chunk_size` bytes
    /// are pushed in by the host with `feed_chunk`; at most `capacity` are kept.
    #[wasm_bindgen(constructor)]
    pub fn new(len: f64, chunk_size: u32, capacity: u32) -> Editor {
        let store = ChunkStore::new(len as u64, chunk_size as u64, capacity as usize);
        Editor { doc: Document::new(store), eval: None }
    }

    fn changed(&mut self) {
        if let Some(e) = &mut self.eval {
            e.invalidate();
        }
    }

    // ----- templates -----

    pub fn template_names(&self) -> Vec<String> {
        formats::builtin_names().iter().map(|s| s.to_string()).collect()
    }

    /// Name of the built-in template matching these leading bytes, or "".
    pub fn sniff_template(&self, head: &[u8]) -> String {
        formats::sniff(head).unwrap_or("").to_string()
    }

    /// Select a built-in template by name; "" clears it. Returns false if unknown.
    pub fn set_template(&mut self, name: &str) -> bool {
        if name.is_empty() {
            self.eval = None;
            return true;
        }
        match formats::builtin(name) {
            Some(t) => {
                self.eval = Some(Evaluator::new(t));
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
        match magicrule::match_signature(rules, head) {
            Some(sig) => {
                self.eval = Some(Evaluator::new(magicrule::signature_template(name, &sig)));
                true
            }
            None => false,
        }
    }

    /// What the type at `path` permits, beyond what its value shows: the other
    /// values an enum names, the bytes a magic field wanted, or what each bit
    /// of a flags field means. JSON, in the same reply shape as the rest.
    pub fn type_info(&mut self, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut self.eval {
            None => reply::<ExplainDto>(Err(EvalError::Failed("no template".into()))),
            Some(e) => reply(e.explain(&self.doc, &p).map(explain_dto)),
        }
    }

    /// What tool produced this file, according to a bundle of Detect It Easy
    /// signature rules. `rules` is the bundle text, `head` the file's first
    /// bytes. Returns JSON, an array that is usually empty.
    ///
    /// Rules that test from the entry point need one: it is worked out from
    /// the DOS header here, and where there is none those rules are skipped
    /// rather than tested at the start of the file.
    pub fn detect_tools(&self, rules: &str, head: &[u8]) -> String {
        let db = diescript::parse_bundle(rules);
        // A file is a DOS executable or a Windows one, never both, so one
        // entry point covers whichever rules were handed over.
        let entry = diescript::pe_entry_point(head)
            .or_else(|| diescript::mz_entry_point(head, self.doc.len_bytes()));
        let found: Vec<ToolDto> = diescript::detect(&db, head, entry)
            .into_iter()
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
    pub fn template_node(&mut self, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut self.eval {
            None => reply::<NodeDto>(Err(EvalError::Failed("no template".into()))),
            Some(e) => reply(e.node(&self.doc, &p).map(dto)),
        }
    }

    /// Same envelope as `template_node`, with `node` being an array of children.
    pub fn template_children(&mut self, path: &[u32], from: f64, to: f64) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut self.eval {
            None => reply::<Vec<NodeDto>>(Err(EvalError::Failed("no template".into()))),
            Some(e) => reply(e.children(&self.doc, &p, from as u64, to as u64).map(|v| v.into_iter().map(dto).collect::<Vec<NodeDto>>())),
        }
    }

    /// Whole text of a text field, decoded in its own encoding:
    /// {status:"ok",node:{text,truncated}}.
    pub fn field_text(&mut self, path: &[u32]) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        match &mut self.eval {
            None => reply::<TextDto>(Err(EvalError::Failed("no template".into()))),
            Some(e) => reply(e.text_value(&self.doc, &p).map(|(text, truncated)| TextDto { text, truncated })),
        }
    }

    /// Every field between two bit offsets, for the annotation column:
    /// {status:"ok",node:[span,..]}. `max` caps how many come back.
    pub fn spans(&mut self, from_bit: f64, to_bit: f64, max: u32) -> String {
        match &mut self.eval {
            None => reply::<Vec<SpanDto>>(Err(EvalError::Failed("no template".into()))),
            Some(e) => reply(
                e.spans(&self.doc, from_bit as u64, to_bit as u64, max as usize)
                    .map(|v| v.into_iter().map(span_dto).collect::<Vec<SpanDto>>()),
            ),
        }
    }

    /// Path of the deepest field covering `bit`, as {status:"ok",node:[..]}.
    /// Its ancestors are the prefixes of that path.
    pub fn locate(&mut self, bit: f64) -> String {
        match &mut self.eval {
            None => reply::<Vec<usize>>(Err(EvalError::Failed("no template".into()))),
            Some(e) => reply(e.locate(&self.doc, bit as u64)),
        }
    }

    /// Write `text` into the field at `path`, encoded as that field's type.
    /// Same envelope as `template_node`; on success `node` is the bit range written.
    pub fn write_node(&mut self, path: &[u32], text: &str) -> String {
        let p: Vec<usize> = path.iter().map(|&x| x as usize).collect();
        let prepared = match &mut self.eval {
            None => return reply::<WriteDto>(Err(EvalError::Failed("no template".into()))),
            Some(e) => e.prepare_write(&self.doc, &p, text),
        };
        match prepared {
            Ok(w) => {
                self.doc.overwrite_bits(w.offset_bits, &w.data, w.n_bits);
                self.changed();
                reply(Ok(WriteDto { offset_bits: w.offset_bits as f64, size_bits: w.n_bits as f64 }))
            }
            Err(e) => reply::<WriteDto>(Err(e)),
        }
    }

    pub fn feed_chunk(&mut self, chunk: f64, data: &[u8]) {
        self.doc.source_mut().insert(chunk as u64, data.into());
    }

    pub fn has_chunk(&self, chunk: f64) -> bool {
        self.doc.source().has(chunk as u64)
    }

    pub fn chunk_size(&self) -> u32 {
        self.doc.source().chunk_size() as u32
    }

    pub fn len_bytes(&self) -> f64 {
        self.doc.len_bytes() as f64
    }

    pub fn len_bits(&self) -> f64 {
        self.doc.len_bits() as f64
    }

    /// Fill `out` with document bytes from `at`. Returns the chunk indices that
    /// were not loaded (those bytes are zero). Empty list means the read is complete.
    pub fn read_bytes(&self, at: f64, out: &mut [u8]) -> Vec<f64> {
        self.doc.read_bytes(at as u64, out).into_iter().map(|m| m.chunk as f64).collect()
    }

    pub fn read_bits(&self, at_bit: f64, n: f64, out: &mut [u8]) -> Vec<f64> {
        self.doc.read_bits(at_bit as u64, n as u64, out).into_iter().map(|m| m.chunk as f64).collect()
    }

    pub fn overwrite_bytes(&mut self, at: f64, data: &[u8]) {
        self.changed();
        self.doc.overwrite_bytes(at as u64, data);
    }
    /// Overwrite that folds into the previous undo step.
    pub fn amend_overwrite_bytes(&mut self, at: f64, data: &[u8]) {
        self.changed();
        self.doc.amend_overwrite_bytes(at as u64, data);
    }
    pub fn insert_bytes(&mut self, at: f64, data: &[u8]) {
        self.changed();
        self.doc.insert_bytes(at as u64, data);
    }
    pub fn delete_bytes(&mut self, at: f64, n: f64) {
        self.changed();
        self.doc.delete_bytes(at as u64, n as u64);
    }
    pub fn overwrite_bits(&mut self, at_bit: f64, data: &[u8], n: f64) {
        self.changed();
        self.doc.overwrite_bits(at_bit as u64, data, n as u64);
    }
    pub fn insert_bits(&mut self, at_bit: f64, data: &[u8], n: f64) {
        self.changed();
        self.doc.insert_bits(at_bit as u64, data, n as u64);
    }
    pub fn delete_bits(&mut self, at_bit: f64, n: f64) {
        self.changed();
        self.doc.delete_bits(at_bit as u64, n as u64);
    }

    /// Save plan as flat quads: kind (0 orig, 1 add, 2 materialize), doc_off, src_off, len.
    pub fn save_plan(&self) -> Vec<f64> {
        self.doc
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
        self.doc.add_bytes().to_vec()
    }

    pub fn undo(&mut self) -> bool {
        self.changed();
        self.doc.undo()
    }
    pub fn redo(&mut self) -> bool {
        self.changed();
        self.doc.redo()
    }
    pub fn can_undo(&self) -> bool {
        self.doc.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.doc.can_redo()
    }
    pub fn is_modified(&self) -> bool {
        self.doc.is_modified()
    }
    pub fn piece_count(&self) -> u32 {
        self.doc.piece_count() as u32
    }
}
