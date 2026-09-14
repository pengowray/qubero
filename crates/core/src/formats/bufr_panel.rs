//! The panel on a BUFR message's section 4: a [`Reading`] cut down to what a
//! viewer lists, which is the header, the steps, the descriptors expanded, one
//! subset's values, and the value under the cursor taken apart.
//!
//! Apart from [`bufr_data`](super::bufr_data), which does the reading, because
//! this only arranges the answer for display and nothing in the reading
//! depends on it. `bufr_data` re-exports it, so callers still name it there.

use super::bufr_data::{describe, fxy, Item, Reading, Role};
use super::bufr_tables::{self, Kind, Tables};

/// How many values the panel lists at once. A radiosonde ascent is about
/// fifteen hundred; a list of a thousand is enough to scroll through, and a
/// message past it is shown around the cursor.
pub const PANEL_VALUES: usize = 1000;

/// How many expanded descriptors the panel lists. Table D's longest
/// expansions run to a few hundred.
pub const PANEL_DESCRIPTORS: usize = 1000;

/// How many subsets' values the panel shows for the value under the cursor in
/// a compressed message.
pub const PANEL_ACROSS: usize = 64;

/// What the panel on section 4 shows: the message's header, the steps, the
/// descriptors expanded, one subset's values, and the value under the cursor
/// taken apart.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Panel {
    pub edition: u32,
    pub master_table_version: u32,
    pub tables_version: u32,
    pub subsets: u32,
    pub compressed: bool,
    pub steps: Vec<String>,
    pub problem: Option<String>,
    pub descriptors: Vec<PanelDescriptor>,
    pub descriptors_total: u64,
    /// Which subset `values` are, counted from 0: the one the cursor is in,
    /// or the first.
    pub subset: u32,
    /// The subset's values, operators left out, from `values_start`.
    pub values: Vec<PanelValue>,
    pub values_start: u64,
    pub values_total: u64,
    pub cursor: Option<PanelCursor>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelDescriptor {
    pub code: u32,
    pub depth: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelValue {
    pub code: u32,
    pub role: &'static str,
    pub name: String,
    pub text: String,
    pub unit: String,
    pub missing: bool,
    /// The element this value is about, as its descriptor and name, for an
    /// associated field, a marker or quality information.
    pub about: Option<String>,
}

/// The value under the cursor, with what it was read from.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelCursor {
    /// Its place in `Panel::values`, where it is among those listed.
    pub index: Option<usize>,
    pub value: PanelValue,
    /// Where its bits start, counted from the first bit of section 4 with its
    /// four-byte header included, which is how a reader counting from the
    /// section's first byte will count; and how wide the value is.
    pub bit: u64,
    pub width: u32,
    pub scale: i32,
    pub reference: i64,
    pub numeric: bool,
    /// This subset's packed number, before the reference and the scale.
    pub packed: Option<u64>,
    /// A compressed value's smallest packed number and difference width, and
    /// every subset's value, the first [`PANEL_ACROSS`] of them.
    pub base: Option<u64>,
    pub increment_width: Option<u32>,
    pub across: Vec<String>,
}

fn panel_value(t: &Tables, items: &[Item], item: &Item, subset: usize) -> PanelValue {
    let mut about = item.refers_to.and_then(|k| items.get(k)).map(|e| format!("{:06} {}", e.code, e.name));
    // A value that is about another element is named by what it is, and the
    // element it is about goes beside it: the element's own name on both
    // would read as two of the element.
    let name = match item.role {
        Role::Marker => match fxy(item.code).1 {
            23 => "Substituted value",
            24 => "First-order statistic",
            25 => "Difference statistic",
            _ => "Replaced/retained value",
        }
        .to_string(),
        Role::Associated => "Associated field".to_string(),
        Role::NewReference => {
            about = Some(format!("{:06} {}", item.code, item.name));
            "New reference value".to_string()
        }
        _ if item.name.is_empty() => describe(t, item.code),
        _ => item.name.to_string(),
    };
    // A unit only for a measurement: a code table's number, a count or an
    // associated field is a number of nothing.
    let measured = matches!(item.role, Role::Element | Role::Marker) && bufr_tables::kind_of(item.unit) == Kind::Numeric;
    // The year, month, day, hour, minute and second of a date are named by
    // their unit already: `Year 2012` says it, and `2012 a` reads as a typo.
    let date_part = matches!(item.code, 4001..=4006) || matches!(item.refers_to.and_then(|k| items.get(k)).map(|e| e.code), Some(4001..=4006)) && item.role == Role::Marker;
    let unit = if measured && item.unit != "Numeric" && !date_part { item.unit.to_string() } else { String::new() };
    PanelValue {
        code: item.code,
        role: if item.role == Role::Element && item.refers_to.is_some() { "quality" } else { item.role.as_str() },
        name,
        text: item.text(subset),
        unit,
        missing: item.missing(subset),
        about,
    }
}

/// The panel for a reading, with the cursor `cursor_bit` bits into section 4's
/// data, or nowhere in it.
pub fn panel(r: &Reading, cursor_bit: Option<u64>) -> Panel {
    let t = bufr_tables::for_version(r.header.master_table_version);
    let h = &r.header;
    let mut p = Panel {
        edition: h.edition,
        master_table_version: h.master_table_version,
        tables_version: r.tables_version,
        subsets: h.subsets,
        compressed: h.compressed,
        steps: r.steps.iter().map(|s| s.what.clone()).collect(),
        problem: r.problem.clone(),
        descriptors_total: r.expanded.len() as u64,
        descriptors: r
            .expanded
            .iter()
            .take(PANEL_DESCRIPTORS)
            .map(|e| PanelDescriptor { code: e.code, depth: e.depth, name: describe(t, e.code) })
            .collect(),
        ..Panel::default()
    };
    // Which list of items, which subset, and which item the cursor is on.
    let holds = |i: &Item, bit: u64| i.role != Role::Operator && i.bit <= bit && bit < i.bit + i.bits.max(1);
    let (list, subset, under) = match cursor_bit {
        Some(bit) if h.compressed => {
            let under = r.subsets.first().and_then(|items| items.iter().position(|i| holds(i, bit)));
            // On a difference, the subset it is the difference of.
            let subset = under
                .and_then(|k| {
                    let i = &r.subsets[0][k];
                    let (w, n) = (u64::from(i.increment_width?), i.width as u64 + 6);
                    let into = bit.checked_sub(i.bit + n)?;
                    let bytes = if i.packed.is_empty() { 8 } else { 1 };
                    (w > 0).then(|| (into / (w * bytes)) as usize)
                })
                .unwrap_or(0);
            (0, subset.min(h.subsets.saturating_sub(1) as usize), under)
        }
        Some(bit) => {
            let s = r.subsets.iter().position(|items| items.iter().any(|i| holds(i, bit))).unwrap_or(0);
            (s, 0, r.subsets.get(s).and_then(|items| items.iter().position(|i| holds(i, bit))))
        }
        None => (0, 0, None),
    };
    let Some(items) = r.subsets.get(list) else { return p };
    let shown_subset = if h.compressed { subset } else { list };
    p.subset = shown_subset as u32;
    let values: Vec<(usize, &Item)> = items.iter().enumerate().filter(|(_, i)| i.role != Role::Operator).collect();
    p.values_total = values.len() as u64;
    let at = under.and_then(|k| values.iter().position(|(j, _)| *j == k));
    let start = at.map_or(0, |a| a.saturating_sub(PANEL_VALUES / 2)).min(values.len().saturating_sub(PANEL_VALUES));
    p.values_start = start as u64;
    p.values = values.iter().skip(start).take(PANEL_VALUES).map(|(_, i)| panel_value(t, items, i, subset)).collect();
    if let (Some(k), Some(a)) = (under, at) {
        let i = &items[k];
        p.cursor = Some(PanelCursor {
            index: (a >= start && a < start + PANEL_VALUES).then(|| a - start),
            value: panel_value(t, items, i, subset),
            bit: i.bit + 32,
            width: i.width,
            scale: i.scale,
            reference: i.reference,
            // Scaled against a reference: a measurement, a count, a marker.
            // Not text, and not a code or flag table, whose number is the
            // value as written.
            numeric: !i.packed.is_empty()
                && matches!(i.role, Role::Element | Role::Count | Role::Marker)
                && bufr_tables::kind_of(i.unit) == Kind::Numeric,
            packed: i.packed.get(subset).copied().flatten(),
            base: i.base,
            increment_width: i.increment_width,
            across: if h.compressed {
                (0..i.values.len().min(PANEL_ACROSS)).map(|s| if i.missing(s) { String::new() } else { i.text(s) }).collect()
            } else {
                Vec::new()
            },
        });
    }
    p
}
