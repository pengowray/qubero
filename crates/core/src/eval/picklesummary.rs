//! What a summary row of a library object says: the few words above a frame, a
//! sparse matrix or an index that tell a reader what the thing is before the
//! structure shows how it is written.
//!
//! Apart from [`picklecells`](super::picklecells), which reads the cells of the
//! tables these sit over, because this is text and that is bytes: every
//! function here ends in a `String` a reader sees, and the rules about how much
//! to show and where to cut belong beside
//! [`picklesaid`](super::picklesaid)'s rather than beside a cell reader.

use super::*;
use super::pickleframe::{class_name, frame_of, index_state, is_range, values_of};
use super::pickleparts::Says;
use super::picklesaid::MOST_SHOWN_TEXT as MOST_SHOWN;
use crate::formats::pickle::familiar::{Kind, Match, Value as Captured};
use crate::template::Cells;

impl Evaluator {
    /// What one summary row of a library object says.
    ///
    /// Worked out here rather than where the rows are listed, because every
    /// one of these reads the file: a column's name is text somewhere else in
    /// it, and a counted index is not in it at all.
    pub(super) fn pickle_summary<S: Source>(
        &mut self,
        doc: &Document<S>,
        root: &[usize],
        path: &[usize],
        found: &Match,
        object: &Captured,
        says: Says,
    ) -> R<String> {
        let r = self.memo[root].clone();
        let base = r.offset;
        // The row belongs to the object, which is the node above this one.
        let of = &path[..path.len() - 1];
        match says {
            Says::Columns | Says::Rows | Says::Index | Says::Dtypes => {
                let Some(shape) = self.frame_shape(doc, of)? else { return Ok(String::new()) };
                let Some(Cells::Computed { rows }) = shape.cells else { return Ok(String::new()) };
                Ok(match says {
                    Says::Rows => rows.to_string(),
                    Says::Columns => cut(&shape.names[1..].join(", ")),
                    Says::Dtypes => cut(
                        &shape.names[1..]
                            .iter()
                            .zip(&shape.units[1..])
                            .map(|(name, word)| format!("{name} {word}"))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    _ => self.index_summary(doc, root, &r, base, found, object)?,
                })
            }
            Says::Shape | Says::Stored | Says::Format => self.sparse_summary(doc, &r, base, found, object, says),
            // A tensor's rows: which storage it is a window onto, and where
            // in the file those numbers are.
            Says::Storage | Says::Location | Says::Numbers | Says::StoredAt => match super::pickletorch::tensor_of(object) {
                Some(tensor) => self.tensor_summary(doc, &r, base, tensor, says),
                None => Ok(String::new()),
            },
        }
    }

    /// What an index is and where its labels run from and to.
    #[allow(clippy::too_many_arguments)]
    fn index_summary<S: Source>(&mut self, doc: &Document<S>, root: &[usize], r: &Resolved, base: u64, found: &Match, object: &Captured) -> R<String> {
        let Some(frame) = frame_of(&found, object) else { return Ok(String::new()) };
        let kind = index_kind(frame.index).unwrap_or("Index");
        if is_range(frame.index) {
            let Some(state) = index_state(frame.index) else { return Ok(kind.to_string()) };
            let (start, stop, step) = self.range_bounds(doc, r, base, state)?;
            let (Some(start), Some(stop), Some(step)) = (start, stop, step) else { return Ok(kind.to_string()) };
            let by = if step == 1 { String::new() } else { format!(" by {step}") };
            return Ok(format!("{kind} {start} to {stop}{by}"));
        }
        let Some(rows) = self.index_len(doc, r, base, found, frame.index)? else { return Ok(kind.to_string()) };
        if rows == 0 {
            return Ok(format!("{kind} of no labels"));
        }
        // The first and the last label, which is what a reader wants of an
        // index of dates and is still true of one of names.
        let first = self.index_label(doc, root, r, base, found, frame.index, 0)?.value;
        let last = self.index_label(doc, root, r, base, found, frame.index, rows - 1)?.value;
        Ok(match (label_text(&first), label_text(&last)) {
            (Some(first), Some(last)) if rows > 1 => format!("{kind} {first} to {last}"),
            (Some(first), _) => format!("{kind} {first}"),
            _ => format!("{kind} of {rows} labels"),
        })
    }

    /// What a sparse matrix holds: how big it is, how many values it stores,
    /// and which of scipy's layouts it is.
    fn sparse_summary<S: Source>(
        &mut self,
        doc: &Document<S>,
        r: &Resolved,
        base: u64,
        found: &Match,
        object: &Captured,
        says: Says,
    ) -> R<String> {
        let Kind::Instance { class, state: Some(state) } = &object.kind else { return Ok(String::new()) };
        if says == Says::Format {
            // `csr_matrix` is the CSR layout, and the class's name is where
            // the file says which layout it is.
            let name = class_name(class).unwrap_or_default();
            return Ok(name.rsplit_once('_').map(|(kind, _)| kind.to_string()).unwrap_or_default());
        }
        let Kind::Dict(entries) = &state.kind else { return Ok(String::new()) };
        if says == Says::Shape {
            // The one attribute that is a tuple of whole numbers.
            let shape = entries.iter().find_map(|(_, v)| match &v.kind {
                Kind::Tuple(items) if !items.is_empty() && items.iter().all(|x| matches!(x.kind, Kind::Int { .. })) => Some(items),
                _ => None,
            });
            let Some(shape) = shape else { return Ok(String::new()) };
            let said: Vec<String> = shape
                .iter()
                .map(|x| match x.kind {
                    Kind::Int { value, .. } => value.to_string(),
                    _ => String::new(),
                })
                .collect();
            return Ok(said.join(" x "));
        }
        // How many values are stored, which is the length of the run the
        // matrix calls `data`.
        for (key, value) in entries {
            if self.pickle_text(doc, r, base, key)?.as_deref() != Some("data") {
                continue;
            }
            let Some(held) = values_of(found, value) else { break };
            let Some((rows, _)) = held.rows_and_columns() else { break };
            return Ok(rows.to_string());
        }
        Ok(String::new())
    }
}

/// Which kind of index this is, by the class `_new_Index` was handed.
fn index_kind(index: &Captured) -> Option<&str> {
    let Kind::Made { items, .. } = &index.kind else { return None };
    class_name(items.first()?)
}

/// A label as a summary shows it.
fn label_text(label: &Option<Value>) -> Option<String> {
    match label {
        Some(Value::Str(said)) => Some(said.clone()),
        Some(Value::Int(n)) => Some(n.to_string()),
        Some(Value::UInt(n)) => Some(n.to_string()),
        Some(Value::Float(f)) => Some(f.to_string()),
        _ => None,
    }
}

/// A count of `unit` from 1970 as the date it is: ISO 8601, no zone, which is
/// how pandas shows one. Midnight with nothing after it is a date alone.
///
/// The value pandas writes where it has no date is the smallest number a
/// 64-bit integer holds, which comes back as nothing the way a NaN does. Units
/// finer than a nanosecond are not read: nothing writes one and the arithmetic
/// would not fit.
pub(super) fn iso_time(count: i64, unit: &str) -> Option<String> {
    if count == i64::MIN {
        return None;
    }
    // A year and a month are calendar units rather than a length of time, so
    // they are counted on the calendar rather than in nanoseconds.
    if unit == "Y" {
        return Some(format!("{:04}-01-01", 1970 + count));
    }
    if unit == "M" {
        let months = 1970i64 * 12 + count;
        return Some(format!("{:04}-{:02}-01", months.div_euclid(12), months.rem_euclid(12) + 1));
    }
    let per: i128 = match unit {
        "W" => 604_800 * NANOS_PER_SECOND,
        "D" => 86_400 * NANOS_PER_SECOND,
        "h" => 3_600 * NANOS_PER_SECOND,
        "m" => 60 * NANOS_PER_SECOND,
        "s" => NANOS_PER_SECOND,
        "ms" => 1_000_000,
        "us" => 1_000,
        "ns" => 1,
        _ => return None,
    };
    let total = i128::from(count).checked_mul(per)?;
    let seconds = total.div_euclid(NANOS_PER_SECOND);
    let nanos = total.rem_euclid(NANOS_PER_SECOND) as u32;
    let days = i64::try_from(seconds.div_euclid(86_400)).ok()?;
    let rest = seconds.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    if rest == 0 && nanos == 0 {
        return Some(format!("{year:04}-{month:02}-{day:02}"));
    }
    let clock = format!("{:02}:{:02}:{:02}", rest / 3600, (rest / 60) % 60, rest % 60);
    let fraction = match nanos {
        0 => String::new(),
        n if n % 1_000_000 == 0 => format!(".{:03}", n / 1_000_000),
        n if n % 1_000 == 0 => format!(".{:06}", n / 1_000),
        n => format!(".{n:09}"),
    };
    Some(format!("{year:04}-{month:02}-{day:02}T{clock}{fraction}"))
}

const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// A civil date from days since 1970-01-01, by Howard Hinnant's algorithm.
/// The other way round is `eval::time::days_from_civil`, and the reason
/// neither pulls in a calendar crate is written there.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A summary row read at a glance, cut where it stops being one.
fn cut(said: &str) -> String {
    match said.char_indices().nth(MOST_SHOWN) {
        Some((at, _)) => format!("{}...", &said[..at]),
        None => said.to_string(),
    }
}
