//! The stretches of the file the hex view colours, and what each one is.

use crate::*;

/// One entry of the annotation column.
#[derive(Serialize)]
pub(crate) struct SpanDto {
    pub(crate) path: Vec<usize>,
    pub(crate) name: String,
    /// What it sits inside, outermost first.
    pub(crate) trail: Vec<String>,
    #[serde(rename = "type")]
    pub(crate) type_name: String,
    pub(crate) offset_bits: f64,
    pub(crate) size_bits: f64,
    /// Whether the two numbers above count each byte's bits from the bottom.
    pub(crate) lsb_first: bool,
    pub(crate) value: String,
    pub(crate) kind: &'static str,
    /// No field covers these bits.
    pub(crate) gap: bool,
    /// Fields this entry stands for, when a run of numbers is shown as one.
    pub(crate) count: f64,
    /// What one of those is called, singular. Null when the format has no word
    /// for them and they read as values.
    pub(crate) unit: Option<String>,
    /// A structure that reads on one row, already joined. Null for a field that
    /// reads as its own value.
    pub(crate) line: Option<String>,
    /// The first few values of a run shown as one entry.
    pub(crate) sample: Vec<String>,
    /// First element extents followed by the uninspected remainder.
    pub(crate) parts: Vec<SpanPartDto>,
    /// How a variable-length number's bits divide into framing and value.
    /// Null for a field that reads as whole bytes, which is most of them.
    pub(crate) bits: Option<BitRolesDto>,
    /// These bytes are a document of their own and can be opened as one.
    pub(crate) opens: bool,
    /// The template wrote this structure to hold one value in several fields,
    /// so the name it gave the structure is the template's own bookkeeping and
    /// is not shown to a reader.
    pub(crate) inline: bool,
    /// What is wrong with this field's value, when something is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) problem: Option<ProblemDto>,
    /// Wrong values found under this span so far: invalid, then undefined.
    pub(crate) problems_within: [u32; 2],
    /// The colour this structure says it is. Absent for everything else.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) colour: Option<String>,
}

/// The bit split of one variable-length number, in the order it is stored.
/// What is wrong with one value, in the words the core built. `tier` says who
/// says so: `invalid` is the format ruling the value out, `undefined` is
/// Qubero having no name for it.
#[derive(Serialize)]
pub(crate) struct ProblemDto {
    pub(crate) tier: &'static str,
    pub(crate) text: String,
}

pub(crate) fn problem_dto(p: qubero_core::eval::Problem) -> ProblemDto {
    ProblemDto {
        tier: match p.tier {
            qubero_core::eval::Tier::Invalid => "invalid",
            qubero_core::eval::Tier::Undefined => "undefined",
        },
        text: p.text,
    }
}

#[derive(Serialize)]
pub(crate) struct BitRolesDto {
    /// Which rule a reader has to know to follow the split. The view keys its
    /// wording off this; the core does not carry the wording.
    pub(crate) rule: &'static str,
    pub(crate) groups: Vec<BitGroupDto>,
}

#[derive(Serialize)]
pub(crate) struct BitGroupDto {
    pub(crate) bits: String,
    pub(crate) role: &'static str,
}

#[derive(Serialize)]
pub(crate) struct SpanPartDto {
    pub(crate) size_bits: f64,
    pub(crate) label: String,
    pub(crate) rest: bool,
}

#[derive(Serialize)]
pub(crate) struct ExtentEstimateDto {
    pub(crate) path: Vec<usize>,
    pub(crate) measured_items: f64,
    pub(crate) total_items: f64,
    pub(crate) measured_bits: f64,
    pub(crate) estimated_bits: f64,
}

pub(crate) fn extent_estimate_dto(estimate: ExtentEstimate) -> ExtentEstimateDto {
    ExtentEstimateDto {
        path: estimate.path,
        measured_items: estimate.measured_items as f64,
        total_items: estimate.total_items as f64,
        measured_bits: estimate.measured_bits as f64,
        estimated_bits: estimate.estimated_bits as f64,
    }
}

pub(crate) fn span_part_dto(part: SpanPart) -> SpanPartDto {
    SpanPartDto {
        size_bits: part.size_bits as f64,
        label: part.label,
        rest: part.rest,
    }
}

/// Whether a row is one machine instruction, which is what the type column
/// says when a template read it with a decoder.
pub(crate) fn is_machine(type_name: &str) -> bool {
    qubero_core::code::Isa::named(type_name).is_some()
}

pub(crate) fn span_dto(s: Span) -> SpanDto {
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
        lsb_first: s.lsb_first,
        value,
        kind,
        gap: s.gap,
        count: s.count as f64,
        unit: s.unit,
        line: s.line,
        sample: s.sample,
        parts: s.parts.into_iter().map(span_part_dto).collect(),
        opens: s.opens,
        colour: s.colour,
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
