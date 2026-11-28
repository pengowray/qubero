//! The kinds of node each codec family's trace lays down, and what each kind
//! is: the tables in `qubero_core::codec::tracekind`, as JSON.

use crate::*;
use qubero_core::codec::tracekind::{EnumDef, Family, Form, KindEntry, Reads, RECORD_NAME, SYMBOL_KIND};

/// One family: the word its kinds start with, and its kinds in the order a
/// reader meets them.
#[derive(Serialize)]
pub(crate) struct TraceFamilyDto {
    pub(crate) family: &'static str,
    pub(crate) kinds: Vec<TraceKindDto>,
}

#[derive(Serialize)]
pub(crate) struct TraceKindDto {
    /// `deflate.literal`, as `NodeDto::trace_kind` says it.
    pub(crate) kind: &'static str,
    /// One line for a programmer.
    pub(crate) about: &'static str,
    /// Every way a node of this kind can read. Never empty.
    pub(crate) forms: Vec<TraceFormDto>,
    /// For a row over other nodes, the kinds its children can be. Empty for
    /// every other kind.
    pub(crate) children: Vec<&'static str>,
}

#[derive(Serialize)]
pub(crate) struct TraceFormDto {
    pub(crate) reads: TraceReadsDto,
    /// `Sizing::as_str`: "fixed", "table", "encoded", "trace" or "nothing".
    pub(crate) sized: &'static str,
}

/// What a node reads as, in the IR's terms. `name` is what the type column
/// says, except for a record's fields.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum TraceReadsDto {
    /// `Ty::Traced`: a row over other nodes.
    Traced { name: &'static str },
    /// `Ty::Computed`.
    Computed,
    /// `Ty::Enum` over a computed number.
    Enum { name: &'static str, cases: Vec<EnumCaseDto> },
    /// `Ty::CodeBits`: the code's own bits.
    CodeBits { name: &'static str },
    /// `Ty::Struct`. `kind` is the value its first field, `kind`, holds.
    Struct { name: &'static str, kind: EnumCaseDto, fields: Vec<RecordFieldDto> },
    /// `Ty::Bytes`.
    Bytes,
}

#[derive(Serialize)]
pub(crate) struct EnumCaseDto {
    pub(crate) value: f64,
    pub(crate) name: &'static str,
}

#[derive(Serialize)]
pub(crate) struct RecordFieldDto {
    pub(crate) name: &'static str,
    /// The type column's word for the field: the enum's name for `kind`,
    /// `computed` for the rest.
    #[serde(rename = "type")]
    pub(crate) type_name: &'static str,
}

fn cases(def: &EnumDef) -> Vec<EnumCaseDto> {
    def.cases.iter().map(|&(value, name)| EnumCaseDto { value: value as f64, name }).collect()
}

fn reads_dto(reads: Reads) -> TraceReadsDto {
    match reads {
        Reads::Traced(name) => TraceReadsDto::Traced { name },
        Reads::Computed => TraceReadsDto::Computed,
        Reads::Enum(def) => TraceReadsDto::Enum { name: def.name, cases: cases(def) },
        Reads::CodeBits(name) => TraceReadsDto::CodeBits { name },
        Reads::Struct { kind, fields } => TraceReadsDto::Struct {
            name: RECORD_NAME,
            kind: EnumCaseDto {
                value: kind as f64,
                name: SYMBOL_KIND.cases.iter().find(|c| c.0 == kind).map_or("", |c| c.1),
            },
            fields: fields
                .iter()
                .enumerate()
                .map(|(i, &name)| RecordFieldDto { name, type_name: if i == 0 { SYMBOL_KIND.name } else { "computed" } })
                .collect(),
        },
        Reads::Bytes => TraceReadsDto::Bytes,
    }
}

fn form_dto(f: &Form) -> TraceFormDto {
    TraceFormDto { reads: reads_dto(f.reads), sized: f.sized.as_str() }
}

fn kind_dto(family: Family, e: &KindEntry) -> TraceKindDto {
    TraceKindDto { kind: e.kind, about: e.about, forms: e.forms.iter().map(form_dto).collect(), children: e.child_kinds(family).collect() }
}

pub(crate) fn trace_family_dto(family: Family) -> TraceFamilyDto {
    TraceFamilyDto { family: family.as_str(), kinds: family.kinds().iter().map(|e| kind_dto(family, e)).collect() }
}
