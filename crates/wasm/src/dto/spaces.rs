//! Opened streams: how far unpacking has got, and how a space maps back to the file.

use crate::*;

/// How far a large stream's unpacking has got. See `Editor::unpacking`.
#[derive(Serialize)]
pub(crate) struct UnpackingDto {
    pub(crate) run_space: u32,
    pub(crate) run_offset_bits: f64,
    pub(crate) read_bits: f64,
    pub(crate) run_bits: f64,
    pub(crate) unpacked_bytes: f64,
}

/// What `open_space` came to: the space, or why it would not open.
#[derive(Serialize)]
pub(crate) struct SpaceDto {
    pub(crate) space: f64,
    /// The template reading the unpacked bytes, which is the one that declared
    /// the stream. Empty for a stream that did not open.
    pub(crate) template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) refused: Option<String>,
    /// True when the stream was joined from several runs rather than unpacked
    /// from one, which is a different thing to call the tab.
    pub(crate) joined: bool,
    /// True when every one of those runs is stored as it sits in the file, so
    /// a field of the stream can be edited where the file declares it.
    pub(crate) stored: bool,
}

/// One step of a decoder, as the cursor link shows it.
#[derive(Serialize)]
pub(crate) struct MapStepDto {
    pub(crate) in_start: f64,
    pub(crate) in_end: f64,
    pub(crate) out_start: f64,
    pub(crate) out_end: f64,
    /// The word the interface looks the step's message up by: `literal`,
    /// `match`, `stored`, `pixel`, `header`, `table`, `end-of-block`, `block`
    /// or `opaque`. [`StepKind::as_str`] is the one place these are named.
    pub(crate) kind: &'static str,
    /// Which named field, for a header or a table step: `bfinal`, `hlit`,
    /// `code_len` and the rest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) field: Option<&'static str>,
    /// What that field said, where saying it is the point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<f64>,
    /// A match's only: how long it is and how far back it reaches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) len: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) dist: Option<f64>,
    /// Where in the file the run the step was read from starts: the run the
    /// stream was unpacked from, or for a stream joined from several, the run
    /// of the part the step belongs to. The step's own bits count from there,
    /// so the file's bits are this plus them. For a step whose run is not in
    /// the file (`in_file` false), where the run starts in the stream it is in.
    pub(crate) run_offset_bits: f64,
    /// Whether the run is in the file. False for a stream declared inside
    /// another stream, such as a PNG's zlib stream inside the IDAT chunks'
    /// joined data, whose bits are that stream's and not the file's.
    pub(crate) in_file: bool,
}

/// Which way a step of a space was asked for, which is how its run is found
/// when the space was joined from several: by the byte it made, or by the bit
/// of the file it read.
pub(crate) enum AskedBy {
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
pub(crate) fn space_step_dto(e: &Evaluator, space: SpaceId, s: MapStep, asked: AskedBy) -> Option<MapStepDto> {
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
    (run_space == 0).then(|| step_dto(s, run_offset_bits, true))
}

/// The same, including a step whose run is inside another stream, marked with
/// `in_file` false, for a caller that draws the step rather than marking its
/// bits in the file.
pub(crate) fn any_space_step_dto(e: &Evaluator, space: SpaceId, s: MapStep, byte: u64) -> Option<MapStepDto> {
    let sp = e.space(space)?;
    let (run_space, run_offset_bits) = match sp.run() {
        Some(run) => (run.run_space, run.run_offset_bits),
        None => {
            let run = sp.run_at(byte)?;
            (run.run_space, run.run_offset_bits)
        }
    };
    Some(step_dto(s, run_offset_bits, run_space == 0))
}

pub(crate) fn step_dto(s: MapStep, run_offset_bits: u64, in_file: bool) -> MapStepDto {
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
        in_file,
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
        StepKind::Stored | StepKind::Pixel | StepKind::EndOfBlock | StepKind::Block | StepKind::Opaque | StepKind::Filtered => {}
        // A JPEG code: the DC difference, or an AC coefficient's value, and
        // for the AC where it lands in zigzag order.
        StepKind::Dc { diff, .. } => dto.value = Some(diff as f64),
        StepKind::Ac { value, k, .. } => {
            dto.value = Some(value as f64);
            dto.len = Some(k as f64);
        }
        StepKind::Zrl { k, .. } | StepKind::Eob { k, .. } => dto.len = Some(k as f64),
    }
    dto
}
