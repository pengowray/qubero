//! The whole-file summaries: byte classes, the profile, the ledger, extents and directories.

use crate::*;

/// What the byte-class scan has found so far. `classes` is one digit per
/// bucket in file order, the digit being `overview::Class`.
#[derive(Serialize)]
pub(crate) struct OverviewDto {
    pub(crate) done: bool,
    pub(crate) bucket_bytes: f64,
    pub(crate) total_buckets: f64,
    pub(crate) classes: String,
    pub(crate) zero_bytes: f64,
    pub(crate) text_bytes: f64,
    pub(crate) read_bytes: f64,
    /// How many of each byte value have been read, value 0 first. The whole
    /// 256 rather than the few commonest: a view drawing the spread of a file
    /// wants the shape, and the shape is the tail. Two kilobytes or so of JSON
    /// a step, since a count crosses as a float and writes its `.0`, against
    /// the 256 KiB the step read to fill it in.
    pub(crate) histogram: Vec<f64>,
}

/// The same for one block, with what the whole block's bytes turned out to be
/// rather than what each of its buckets did.
#[derive(Serialize)]
pub(crate) struct FocusDto {
    pub(crate) done: bool,
    /// The block, in bytes.
    pub(crate) start: f64,
    pub(crate) end: f64,
    pub(crate) bucket_bytes: f64,
    pub(crate) total_buckets: f64,
    pub(crate) classes: String,
    pub(crate) zero_bytes: f64,
    pub(crate) text_bytes: f64,
    pub(crate) read_bytes: f64,
    /// The same full spread as the whole-file scan carries, so a view reading
    /// one can read the other. `common` below is this sorted and cut short.
    pub(crate) histogram: Vec<f64>,
    /// Entropy over the block's bytes, and the most a block this long could
    /// reach. The pair is the honest reading: 7.9 out of 8 means dense, 7.9
    /// out of 7.9 means only that there are not many bytes here.
    pub(crate) entropy: f64,
    pub(crate) entropy_max: f64,
    /// How many byte values appear at all.
    pub(crate) distinct: f64,
    /// The values that appear most, commonest first.
    pub(crate) common: Vec<CommonByteDto>,
}

/// One byte value and how much of a block it accounts for.
#[derive(Serialize)]
pub(crate) struct CommonByteDto {
    pub(crate) value: f64,
    pub(crate) count: f64,
}

/// Where the file's bits have gone, as far as the walk has got. The same
/// stepped shape the byte-class scan answers in: partial totals every time,
/// and `done` saying when to stop asking.
#[derive(Serialize)]
pub(crate) struct KindTotalsDto {
    pub(crate) done: bool,
    /// How far into the file the walk has reached. What is past this is in
    /// none of the numbers below, so a view derives what is left to do as the
    /// file's length less this.
    pub(crate) reached_bits: f64,
    /// Bits some field covers, which is the sum of `totals`.
    pub(crate) covered_bits: f64,
    /// Bits inside the reached region that no field covers.
    pub(crate) unmapped_bits: f64,
    pub(crate) totals: Vec<KindTotalDto>,
}

/// What a format is, in a sentence or three for a reader who has never met it.
/// See `qubero_core::formats::about`.
#[derive(Serialize)]
pub(crate) struct AboutDto {
    /// The format's plain name, as the text uses it.
    pub(crate) name: &'static str,
    /// The English Wikipedia article's title, with any `#section`. Null where
    /// no article covers the format.
    pub(crate) wikipedia: Option<&'static str>,
    /// What the format stores and how. Plain text.
    pub(crate) text: &'static str,
}

/// How a format writes its values and finds its parts: the template's, or the
/// file's so far. See `qubero_core::eval::profile` for every key.
#[derive(Serialize)]
pub(crate) struct ProfileDto {
    /// True once the whole file has been counted. Always true for a template.
    pub(crate) done: bool,
    pub(crate) rows: Vec<ProfileRowDto>,
    pub(crate) choices: Vec<ChoiceDto>,
    pub(crate) facts: ProfileFactsDto,
}

#[derive(Serialize)]
pub(crate) struct ProfileRowDto {
    /// "number" | "text" | "varint" | "bit-field" | "enum" | "flags" |
    /// "magic" | "computed" | "opaque" | "padding" | "checksum" | "codec" |
    /// "placement" | "sizing"
    pub(crate) category: &'static str,
    pub(crate) kind: String,
    /// Bits wide for a number, 0 where the file sets it; the alignment in
    /// bytes for padding.
    pub(crate) width: f64,
    /// "little" | "big" | "none" for a number, "" for anything else.
    pub(crate) order: &'static str,
    pub(crate) detail: String,
    pub(crate) fields: f64,
    /// Always 0 for a template.
    pub(crate) bits: f64,
    /// For a codec: how many of its streams were opened, and what they came to.
    pub(crate) unpacked_fields: f64,
    pub(crate) unpacked_bits: f64,
}

#[derive(Serialize)]
pub(crate) struct ChoiceDto {
    /// The diagram box's key for the switch.
    pub(crate) key: String,
    pub(crate) name: String,
    /// How many shapes it can take, the default included.
    pub(crate) cases: f64,
    /// How many different ones the file took, and in how many fields. 0 for a
    /// template.
    pub(crate) taken: f64,
    pub(crate) fields: f64,
}

#[derive(Serialize)]
pub(crate) struct ProfileFactsDto {
    pub(crate) from_end: f64,
    pub(crate) placed: f64,
    pub(crate) forward: f64,
    pub(crate) backward: f64,
    pub(crate) lengths_before: f64,
    pub(crate) lengths_after: f64,
    /// Null for a template that places something by an offset, where it
    /// depends on the file.
    pub(crate) every_field_follows: Option<bool>,
}

/// Where every bit of the file goes, grouped by part, by the variant of the
/// nearest list element, and by role. See `qubero_core::eval::ledger`.
#[derive(Serialize)]
pub(crate) struct LedgerDto {
    /// True once every bit is in a row and every gap has been read.
    pub(crate) done: bool,
    /// How far the walk has reached. Past it, nothing is in any row yet.
    pub(crate) reached_bits: f64,
    pub(crate) file_bits: f64,
    pub(crate) counted_bits: f64,
    /// Largest first.
    pub(crate) rows: Vec<LedgerRowDto>,
}

#[derive(Serialize)]
pub(crate) struct LedgerRowDto {
    pub(crate) part: Vec<usize>,
    pub(crate) part_name: String,
    /// Empty where no list element is above these bits.
    pub(crate) group: String,
    /// "key" | "case" | "name" | "type" | "none" | "other"
    pub(crate) group_from: &'static str,
    /// "content" | "machinery" | "padding" | "framing" | "gap"
    pub(crate) role: &'static str,
    pub(crate) bits: f64,
    pub(crate) count: f64,
    /// For a gap or padding: bits in zero bytes, and bits not read. The rest
    /// hold something.
    pub(crate) zero_bits: f64,
    pub(crate) unscanned_bits: f64,
    pub(crate) first_path: Vec<usize>,
    pub(crate) first_offset_bits: f64,
    /// For padding, the alignment in bytes.
    pub(crate) align: f64,
}

/// Every length checked against the part it sizes. See
/// `qubero_core::eval::extent`.
#[derive(Serialize)]
pub(crate) struct ExtentAuditDto {
    pub(crate) done: bool,
    /// How many checks came to each verdict, fits included.
    pub(crate) counts: Vec<VerdictCountDto>,
    /// Every check that does not fit, in the order the walk met them, up to
    /// 500.
    pub(crate) checks: Vec<ExtentCheckDto>,
    /// Where the root's own fields end, and how long the file is.
    pub(crate) root_end_bits: f64,
    pub(crate) space_bits: f64,
    /// The reader's reason, when the root would not read at all.
    pub(crate) root_failed: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct VerdictCountDto {
    pub(crate) verdict: &'static str,
    pub(crate) count: f64,
}

#[derive(Serialize)]
pub(crate) struct ExtentCheckDto {
    /// Empty path and name, null value, where no field of its own gives the
    /// length.
    pub(crate) length_path: Vec<usize>,
    pub(crate) length_name: String,
    pub(crate) length_value: Option<f64>,
    pub(crate) length_invalid: bool,
    pub(crate) part_path: Vec<usize>,
    pub(crate) part_name: String,
    /// "length": `stated` and `read` are bits. "count": they are elements.
    pub(crate) role: &'static str,
    pub(crate) offset_bits: f64,
    pub(crate) stated: Option<f64>,
    pub(crate) read: Option<f64>,
    pub(crate) content_bits: Option<f64>,
    pub(crate) room_bits: f64,
    pub(crate) space_bits: f64,
    pub(crate) adjusted: bool,
    /// "fits" | "short" | "stretched" | "past-parent" | "past-file" |
    /// "unreadable"
    pub(crate) verdict: &'static str,
    pub(crate) why: String,
}

/// Every list whose elements place something elsewhere, and what they place.
/// See `qubero_core::eval::directory`.
#[derive(Serialize)]
pub(crate) struct DirectoriesDto {
    pub(crate) done: bool,
    /// Placements past the cap that were not looked at.
    pub(crate) unexamined: f64,
    pub(crate) lists: Vec<DirectoryDto>,
}

#[derive(Serialize)]
pub(crate) struct DirectoryDto {
    pub(crate) path: Vec<usize>,
    pub(crate) name: String,
    pub(crate) elements: f64,
    pub(crate) placing: f64,
    /// The first 256 elements that place something, in stored order.
    pub(crate) entries: Vec<DirectoryEntryDto>,
}

#[derive(Serialize)]
pub(crate) struct DirectoryEntryDto {
    pub(crate) path: Vec<usize>,
    pub(crate) name: String,
    pub(crate) offset_bits: f64,
    pub(crate) size_bits: f64,
    pub(crate) targets: Vec<DirectoryTargetDto>,
}

#[derive(Serialize)]
pub(crate) struct DirectoryTargetDto {
    pub(crate) path: Vec<usize>,
    pub(crate) name: String,
    pub(crate) offset_bits: f64,
    pub(crate) size_bits: f64,
    /// "address" | "offsets" | "descriptors"
    pub(crate) via: &'static str,
    /// True when these bytes are counted where they are rather than here.
    pub(crate) aside: bool,
}

/// Every run of numbers with text in it. See
/// `qubero_core::eval::textnum`.
#[derive(Serialize)]
pub(crate) struct TextInNumbersDto {
    pub(crate) done: bool,
    /// The shortest text counted, in characters.
    pub(crate) min_chars: f64,
    /// Bytes of numbers the walk found, and how many of them were read.
    pub(crate) numeric_bytes: f64,
    pub(crate) scanned_bytes: f64,
    pub(crate) runs: Vec<NumbersWithTextDto>,
}

#[derive(Serialize)]
pub(crate) struct NumbersWithTextDto {
    pub(crate) path: Vec<usize>,
    pub(crate) name: String,
    /// What one element is: `i16 le`.
    pub(crate) what: String,
    pub(crate) element_bits: f64,
    pub(crate) offset_bits: f64,
    pub(crate) size_bits: f64,
    pub(crate) scanned_bytes: f64,
    pub(crate) texts: f64,
    pub(crate) text_bytes: f64,
    /// The first five, in file order.
    pub(crate) first: Vec<TextRunDto>,
}

#[derive(Serialize)]
pub(crate) struct TextRunDto {
    pub(crate) offset_bits: f64,
    pub(crate) size_bits: f64,
    /// "ASCII" | "UTF-8" | "UTF-16 LE" | "UTF-16 BE"
    pub(crate) encoding: &'static str,
    pub(crate) text: String,
}

/// One kind-and-type pair, and what the file spends on it.
#[derive(Serialize)]
pub(crate) struct KindTotalDto {
    /// The same word a field's row carries: "uint" | "int" | "float" |
    /// "bytes" | "str" | "magic" | "enum" | "flags" | "composite". Worked out
    /// from the type rather than from a value, so the two a value alone can
    /// carry, "unread" and "unset", never appear here.
    pub(crate) kind: &'static str,
    #[serde(rename = "type")]
    pub(crate) type_name: String,
    pub(crate) bits: f64,
    pub(crate) count: f64,
}

/// How many of a block's commonest byte values are worth naming. Enough to
/// show a block is mostly two or three values; past that the count is the
/// answer, not the list.
pub(crate) const COMMON_BYTES: usize = 5;

/// A scan's byte counts as the host reads them. Counts cross the boundary as
/// `f64` like every other number here, and 256 of them written out with the
/// `.0` a float carries is a couple of kilobytes of JSON: nothing beside the
/// quarter of a megabyte the step read to fill it in, or beside the bucket
/// string that already goes back every step.
pub(crate) fn histogram(scan: &overview::Scan) -> Vec<f64> {
    scan.histogram().iter().map(|&n| n as f64).collect()
}

pub(crate) fn kind_totals_dto(t: qubero_core::eval::KindTotals) -> KindTotalsDto {
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

pub(crate) fn profile_dto(p: qubero_core::eval::Profile) -> ProfileDto {
    let f = p.facts;
    ProfileDto {
        done: p.done,
        rows: p
            .rows
            .into_iter()
            .map(|r| ProfileRowDto {
                category: r.category,
                kind: r.kind,
                width: f64::from(r.width),
                order: r.order,
                detail: r.detail,
                fields: r.fields as f64,
                bits: r.bits as f64,
                unpacked_fields: r.unpacked_fields as f64,
                unpacked_bits: r.unpacked_bits as f64,
            })
            .collect(),
        choices: p
            .choices
            .into_iter()
            .map(|c| ChoiceDto { key: c.key, name: c.name, cases: c.cases as f64, taken: c.taken as f64, fields: c.fields as f64 })
            .collect(),
        facts: ProfileFactsDto {
            from_end: f.from_end as f64,
            placed: f.placed as f64,
            forward: f.forward as f64,
            backward: f.backward as f64,
            lengths_before: f.lengths_before as f64,
            lengths_after: f.lengths_after as f64,
            every_field_follows: f.every_field_follows,
        },
    }
}

pub(crate) fn ledger_dto(l: qubero_core::eval::Ledger, reached_bits: u64) -> LedgerDto {
    LedgerDto {
        done: l.done,
        reached_bits: reached_bits as f64,
        file_bits: l.file_bits as f64,
        counted_bits: l.counted_bits as f64,
        rows: l
            .rows
            .into_iter()
            .map(|r| LedgerRowDto {
                part: r.part,
                part_name: r.part_name,
                group: r.group,
                group_from: r.group_from,
                role: r.role,
                bits: r.bits as f64,
                count: r.count as f64,
                zero_bits: r.zero_bits as f64,
                unscanned_bits: r.unscanned_bits as f64,
                first_path: r.first_path,
                first_offset_bits: r.first_offset_bits as f64,
                align: f64::from(r.align),
            })
            .collect(),
    }
}

pub(crate) fn extent_audit_dto(a: qubero_core::eval::ExtentAudit) -> ExtentAuditDto {
    ExtentAuditDto {
        done: a.done,
        counts: a.counts.into_iter().map(|(verdict, n)| VerdictCountDto { verdict, count: n as f64 }).collect(),
        checks: a
            .checks
            .into_iter()
            .map(|c| ExtentCheckDto {
                length_path: c.length_path,
                length_name: c.length_name,
                length_value: c.length_value.map(|v| v as f64),
                length_invalid: c.length_invalid,
                part_path: c.part_path,
                part_name: c.part_name,
                role: c.role,
                offset_bits: c.offset_bits as f64,
                stated: c.stated.map(|v| v as f64),
                read: c.read.map(|v| v as f64),
                content_bits: c.content_bits.map(|v| v as f64),
                room_bits: c.room_bits as f64,
                space_bits: c.space_bits as f64,
                adjusted: c.adjusted,
                verdict: c.verdict,
                why: c.why,
            })
            .collect(),
        root_end_bits: a.root_end_bits as f64,
        space_bits: a.space_bits as f64,
        root_failed: a.root_failed,
    }
}

pub(crate) fn directories_dto(d: qubero_core::eval::Directories) -> DirectoriesDto {
    DirectoriesDto {
        done: d.done,
        unexamined: d.unexamined as f64,
        lists: d
            .lists
            .into_iter()
            .map(|l| DirectoryDto {
                path: l.path,
                name: l.name,
                elements: l.elements as f64,
                placing: l.placing as f64,
                entries: l
                    .entries
                    .into_iter()
                    .map(|e| DirectoryEntryDto {
                        path: e.path,
                        name: e.name,
                        offset_bits: e.offset_bits as f64,
                        size_bits: e.size_bits as f64,
                        targets: e
                            .targets
                            .into_iter()
                            .map(|t| DirectoryTargetDto {
                                path: t.path,
                                name: t.name,
                                offset_bits: t.offset_bits as f64,
                                size_bits: t.size_bits as f64,
                                via: t.via,
                                aside: t.aside,
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub(crate) fn text_in_numbers_dto(t: qubero_core::eval::TextInNumbers) -> TextInNumbersDto {
    TextInNumbersDto {
        done: t.done,
        min_chars: t.min_chars as f64,
        numeric_bytes: t.numeric_bytes as f64,
        scanned_bytes: t.scanned_bytes as f64,
        runs: t
            .runs
            .into_iter()
            .map(|r| NumbersWithTextDto {
                path: r.path,
                name: r.name,
                what: r.what,
                element_bits: r.element_bits as f64,
                offset_bits: r.offset_bits as f64,
                size_bits: r.size_bits as f64,
                scanned_bytes: r.scanned_bytes as f64,
                texts: r.texts as f64,
                text_bytes: r.text_bytes as f64,
                first: r
                    .first
                    .into_iter()
                    .map(|x| TextRunDto { offset_bits: x.offset_bits as f64, size_bits: x.size_bits as f64, encoding: x.encoding, text: x.text })
                    .collect(),
            })
            .collect(),
    }
}
