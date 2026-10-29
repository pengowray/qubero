//! A field as the host reads it: its value, its bytes, its cells, and a write.

use crate::*;

/// A field's bytes, and whether it runs on past them.
#[derive(Serialize)]
pub(crate) struct BytesDto {
    pub(crate) bytes: Vec<u8>,
    pub(crate) truncated: bool,
}

#[derive(Serialize)]
pub(crate) struct NodeDto {
    pub(crate) path: Vec<usize>,
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) type_name: String,
    pub(crate) offset_bits: f64,
    pub(crate) size_bits: f64,
    pub(crate) value: String,
    /// What the editor should start with when this value is edited.
    pub(crate) edit_text: String,
    /// "uint" | "int" | "float" | "bytes" | "str" | "magic" | "enum" | "flags" | "composite"
    pub(crate) kind: &'static str,
    /// What is wrong with this value, when something is. Absent for the
    /// overwhelming majority of fields, which hold what their format allows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) problem: Option<ProblemDto>,
    /// Wrong values found under this node so far: invalid, then undefined.
    /// Counted over the children already read, never by reading more, so a
    /// collapsed run's count is a count so far.
    pub(crate) problems_within: [u32; 2],
    pub(crate) child_count: f64,
    /// What one child is called, for counting them: empty when they are items.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) unit: String,
    pub(crate) composite: bool,
    /// True when the children are a list's elements, whichever of the five
    /// kinds of list it is. See `NodeInfo::list`.
    pub(crate) list: bool,
    /// True when `write_node` accepts text for this field.
    pub(crate) editable: bool,
    /// Bytes of the field the value occupies; less than the size for padded
    /// and terminated text.
    pub(crate) value_bytes: f64,
    /// Where the value starts: past a byte-order mark, if the field has one.
    pub(crate) value_offset_bits: f64,
    /// Where the bytes were read, for a field written in one place and read in
    /// another. Null for a field that is where it is written.
    pub(crate) read_at: Option<f64>,
    /// How the encoding was settled, or that the bytes do not fit it.
    pub(crate) read_as: Option<String>,
    /// Which sibling's length, count, type or position this field settles, as
    /// an index among the parent's children. Null for a field no sibling
    /// reads, which is most of them.
    pub(crate) consumed_by: Option<f64>,
    /// What the template says about this field over the top of that: true for
    /// machinery, false for payload, null when it has no opinion.
    pub(crate) machinery: Option<bool>,
    /// True when this field is only its parent's contents, and so has no name
    /// of its own worth a level of structure.
    pub(crate) contents: bool,
    /// True when the template says this structure is one row rather than one
    /// row per field: a value with parts, not a part of the file. A view that
    /// gives it a heading of its own has spent a heading on a colour.
    pub(crate) inline: bool,
    /// True when the node's own bytes include punctuation its children do not
    /// account for: the braces of a JSON object, the brackets of an array.
    /// What the children leave over is the node's own syntax, not bytes
    /// nothing describes.
    pub(crate) framed: bool,
    /// Which address space `offset_bits` counts in: 0 for the file, and a
    /// number of its own for each decoded stream. A field in a space other
    /// than the file has no place in the hex view, and its offset is drawn as
    /// an offset within its stream.
    pub(crate) space: f64,
    /// For a compressed run that would not open: "too-large", "failed" or
    /// "unaligned". Null for every other field.
    pub(crate) refused: Option<String>,
    /// True for a compressed run. One that opened can be opened as a document
    /// of its own, which is what the Open unpacked button does.
    pub(crate) decoded: bool,
    /// True for the one node a stream holds. Its parent is the stream, so this
    /// is where the listing offers Open unpacked.
    pub(crate) space_root: bool,
    /// True for a field read inside a stream joined from several runs, which
    /// is not unpacked from one and has no document of its own to open.
    pub(crate) joined: bool,
    /// True when `offset_bits` and `size_bits` count each byte's bits from
    /// the bottom: a number read low bit first. See
    /// `qubero_core::eval::NodeInfo::lsb_first`.
    pub(crate) lsb_first: bool,
    /// True when the file did not write this field: the condition on an
    /// optional one came to nothing. Told apart from a size of zero, which
    /// several kinds of field that are there also have.
    pub(crate) absent: bool,
    /// What the format's own description says this field is, where the
    /// template carries it. Null for a field nobody wrote prose for, which is
    /// most of them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) doc: Option<String>,
    /// What a structure of a few fields reads as on one line, the same reading
    /// the annotation column puts beside the bytes. Null for a leaf, for a
    /// list, and for a structure too long to read on a line.
    pub(crate) line: Option<String>,
    /// True when the template says this field reads as a table: interleaved
    /// samples, a list of records. `table_shape` says what of.
    pub(crate) table: bool,
    /// The colour this structure says it is, as CSS writes one: `#dccd80`.
    /// Absent for every structure that is not a colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) colour: Option<String>,
}

/// One element of a folded run, as the value table draws it.
#[derive(Serialize)]
pub(crate) struct CellDto {
    /// Where the element sits in its run: the last step of its path.
    pub(crate) index: f64,
    pub(crate) offset_bits: f64,
    pub(crate) size_bits: f64,
    /// What the listing would say about it on a shared row, or the symbol's
    /// name for a block of a trace. Nothing here is reformatted by the view.
    pub(crate) text: String,
    /// What the cell shows, where that is shorter than what its tooltip says:
    /// a deflate literal is the byte, a match its two numbers. The same as
    /// `text` for everything else.
    pub(crate) label: String,
    /// "uint" | "int" | "float" | "bytes" | "str" | "enum" | "flags" |
    /// "composite" | "symbol" | "scale"
    pub(crate) kind: &'static str,
    /// False when the element's bits are not one run, which sends the view to
    /// its uniform layout: a `q5_0` weight is four bits of `qs` and a fifth
    /// bytes away in `qh`.
    pub(crate) contiguous: bool,
    /// True when this record reads exactly as the record before it, so the
    /// cell is drawn without its text. `text` is still what it says, for the
    /// tooltip and for the width the table is laid out to.
    pub(crate) repeat: bool,
    /// What is wrong with this element's value, when something is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) problem: Option<ProblemDto>,
    /// Wrong values found under this element so far: invalid, then undefined.
    pub(crate) problems_within: [u32; 2],
}

#[derive(Serialize)]
pub(crate) struct TextDto {
    pub(crate) text: String,
    /// True when the field holds more than the editor will show.
    pub(crate) truncated: bool,
}

/// The range a successful `write_node` touched.
#[derive(Serialize)]
pub(crate) struct WriteDto {
    pub(crate) offset_bits: f64,
    pub(crate) size_bits: f64,
}

/// How a value reads: its kind, what to show, and what an editor starts with.
/// Whether the format says it is right is `NodeDto::problem`, built in the
/// core so that every view says the same words about the same bytes.
pub(crate) fn shown(v: &Value) -> (&'static str, String, String) {
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

pub(crate) fn dto(n: NodeInfo) -> NodeDto {
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
        lsb_first: n.lsb_first,
        absent: n.absent,
        doc: n.doc,
        line: n.line,
        table: n.table,
        colour: n.colour,
    }
}

/// Chunks the evaluator answered without, as the host counts them.
pub(crate) fn cell_dto(c: qubero_core::eval::Cell) -> CellDto {
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
