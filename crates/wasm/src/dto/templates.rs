//! Choosing a template, and what a Kaitai or ImHex conversion reports.

use crate::*;

/// One rule's answer about what made the file.
#[derive(Serialize)]
pub(crate) struct ToolDto {
    /// The database's own word: `packer`, `compiler`, `protector`.
    pub(crate) category: String,
    pub(crate) name: String,
    pub(crate) version: Option<String>,
    /// Free text from the rule's author, passed through as written.
    pub(crate) options: Option<String>,
    /// The signature file that answered.
    pub(crate) source: String,
}

/// One entry in the template chooser.
#[derive(Serialize)]
pub(crate) struct TemplateChoiceDto {
    /// What `set_template` takes: a built-in's own name, or `ksy:` and a
    /// bundled format's `meta/id`.
    pub(crate) name: String,
    /// The format's `meta/title`, for a bundled Kaitai format that has one.
    /// Empty otherwise, and the chooser then shows the name.
    pub(crate) title: String,
    /// `builtin` or `kaitai`.
    pub(crate) source: &'static str,
    /// A bundled Kaitai format's `meta/file-extension`, lowercase. Empty for a
    /// built-in, whose extensions the web app lists beside its identity rules.
    pub(crate) ext: Vec<String>,
    /// The bytes a bundled Kaitai format pins, as offset and lowercase hex.
    pub(crate) magic: Vec<(u64, String)>,
}

/// What a `.ksy` conversion had to say, for the panel.
#[derive(Serialize)]
pub(crate) struct KsyReportDto {
    /// The format's own id, `meta/id`, which is the name the template goes by
    /// once it is in use. The menu shows it in place of a built-in's name.
    pub(crate) name: String,
    /// One per field converted, in the order the `.ksy` writes them.
    pub(crate) fields: Vec<KsyLineDto>,
    /// Everything the IR could not say, each with what was left in its place.
    pub(crate) gaps: Vec<KsyLineDto>,
    /// Everything said exactly, but not the way the `.ksy` said it.
    pub(crate) notes: Vec<KsyLineDto>,
}

/// A conversion done for the panel and thrown away: the report, and the
/// template it produced written out as text. Nothing is read with it, so the
/// document keeps whatever template it had.
#[derive(Serialize)]
pub(crate) struct KsyPreviewDto {
    pub(crate) report: KsyReportDto,
    /// The template as [`template_text`](Editor::template_text) writes it.
    pub(crate) text: String,
}

/// One line of the report: where in the `.ksy`, the text there, and what of it.
#[derive(Serialize)]
pub(crate) struct KsyLineDto {
    pub(crate) path: String,
    pub(crate) source: String,
    pub(crate) message: String,
}

pub(crate) fn ksy_report_dto(report: &qubero_core::ksy::Report, name: &str) -> KsyReportDto {
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

/// What a `.hexpat` conversion said, or what was wrong with the pattern, in the
/// panel's terms.
///
/// `missing` is the include files the pattern asks for and nothing here has.
/// It comes back with a failure and with a success alike, because a pattern can
/// convert without one of its includes and be the poorer for it.
#[derive(Serialize)]
pub(crate) struct HexpatReportDto {
    /// The name the template goes by once it is in use.
    pub(crate) name: String,
    pub(crate) fields: Vec<KsyLineDto>,
    pub(crate) gaps: Vec<KsyLineDto>,
    pub(crate) notes: Vec<KsyLineDto>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) missing: Vec<String>,
}

/// A conversion done for the panel and thrown away: the report, and the
/// template it produced written out as text.
#[derive(Serialize)]
pub(crate) struct HexpatPreviewDto {
    pub(crate) report: HexpatReportDto,
    pub(crate) text: String,
}

/// A failed conversion, with the include files the pattern wanted. The message
/// leads with the line and column, which is where the panel puts it.
#[derive(Serialize)]
pub(crate) struct HexpatFailedDto {
    pub(crate) message: String,
    pub(crate) missing: Vec<String>,
}

/// The failure envelope for the two `.hexpat` entries.
///
/// The shared [`Reply::Error`] carries a message and nothing else, and an
/// include file the pattern is waiting for is not a message: it is a thing for
/// the reader to hand over. Flattened, so a caller reading `status` and
/// `message` sees exactly what it sees from every other entry.
#[derive(Serialize)]
pub(crate) struct HexpatFailedReply {
    pub(crate) status: &'static str,
    #[serde(flatten)]
    pub(crate) node: HexpatFailedDto,
}

pub(crate) fn hexpat_report_dto(
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
