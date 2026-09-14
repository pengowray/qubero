//! What a BUFR message's section 4 holds, and the steps between the bits and
//! the values.
//!
//! [`bufr`](super::bufr) reads a message as its sections and names every
//! descriptor in section 3, which is what a template can say. It cannot say
//! where one value in section 4 ends and the next begins. That depends on the
//! descriptors expanded through Table D, on a count in the data for every
//! delayed replication, on the operators in force when each value is reached,
//! and for a compressed message on a width written in front of every value.
//! None of that is an expression over fields: it is a walk, and this is it.
//!
//! The message is handed over as bytes and its sections are found again here,
//! the way [`fits_tile`](super::fits_tile) reads a header's cards again: a
//! reader that takes bytes can be tested on bytes, and does not depend on how
//! the template happens to nest.
//!
//! # The walk
//!
//! Section 3's descriptors are taken in order. A sequence is replaced by what
//! Table D says it stands for. A replication repeats the X descriptors after it
//! Y times; with Y of zero the descriptor after it is a count, read from the
//! data like any other value, and the X after that are repeated that many
//! times. An operator changes how later elements are read. An element is a
//! value: Table B gives its width, its scale and its reference, and the value
//! is
//!
//! ```text
//! value = (packed + reference) / 10^scale
//! ```
//!
//! with a packed number of all ones meaning there is no value, except in a
//! field one bit wide, which could say nothing else.
//!
//! An uncompressed message does that once per subset, one subset after the
//! other. A compressed message does it once for all of them: every value is
//! written as the smallest packed number of any subset, then six bits saying
//! how wide the differences are, then one difference per subset. A width of
//! zero means every subset has the same value and no differences are written.
//! Text is compressed the same way, except that the six bits count bytes.
//!
//! # The operators
//!
//! - 201 and 202 add Y less 128 to the width and the scale of every numeric
//!   element after them, and 207 raises the scale by Y, the reference by a
//!   factor of 10^Y and the width by (10Y + 2) / 3 bits. None of the three
//!   touch text, code tables, flag tables, or class 31, whose replication
//!   counts must stay readable. That last is ecCodes' reading of the rule
//!   (its issue ECC-912); pybufrkit widens class 31 too, and none of the
//!   sample files puts a count under one of these operators to tell the two
//!   apart.
//! - 203 makes every element after it, until 203255, a new reference value for
//!   that element, Y bits wide and written sign and magnitude.
//! - 204 puts a field of Y bits in front of every element after it except
//!   those of class 31, and the element straight after the operator, 0-31-021,
//!   says what the field means.
//! - 205 is Y characters of text in the data, and 208 makes every text element
//!   Y characters wide.
//! - 206 says the next descriptor, which is a centre's own and in no table
//!   here, is Y bits wide.
//! - 221 says the next Y descriptors have no data, except those of classes 1
//!   to 9 and 31.
//! - 222000, 223000, 224000, 225000 and 232000 announce quality information,
//!   substituted values, statistics or replaced values for elements already
//!   read, and are followed by a bitmap: a run of 0-31-031 values, one per
//!   element, where 0 means the element has one. The elements are the ones
//!   before the first such operator, as many as the bitmap is long, which is
//!   what ecCodes and pybufrkit both do. 236000 keeps a bitmap for 237000 to use
//!   again, and 237255 forgets it. After the bitmap, 223255, 224255, 225255 and
//!   232255 are each a value of the next element the bitmap points at, read at
//!   that element's width, or one bit wider with a reference of -2^width for
//!   225255, a difference that may go either way.
//!
//! # What stops it
//!
//! A descriptor the tables do not have. A centre's own descriptors, X of 48 or
//! more or Y of 192 or more, are in a local table that is not bundled, and
//! without the width of one the position of everything after it is unknown.
//! The reading stops there and keeps what it had, and says which descriptor it
//! was in [`Reading::problem`].

use std::collections::{BTreeMap, HashMap};

use super::bufr_tables::{self, Element, Kind, Tables};
pub use super::bufr_panel::{panel, Panel, PanelCursor, PanelDescriptor, PanelValue, PANEL_ACROSS, PANEL_DESCRIPTORS, PANEL_VALUES};
use crate::bits::Bits;

/// The packing name the template gives section 4, so that the panel can find
/// this reader again.
pub const PACKING: &str = "bufr data";

/// The largest message this will read for a panel redrawn every time the
/// cursor moves. A radiosonde is a few kilobytes and a satellite message a few
/// hundred; sixteen megabytes is past anything a GTS link would carry.
pub const MESSAGE_LIMIT: usize = 16 << 20;

/// The most values it will hand back, over all the subsets together. A
/// compressed IASI message is about a hundred thousand.
pub const VALUE_LIMIT: usize = 2_000_000;

/// How deep sequences may nest. Table D's deepest is a handful; a sequence that
/// names itself is a file that has gone wrong and would otherwise never end.
const DEPTH_LIMIT: u32 = 32;

/// What sections 0, 1 and 3 say, and where section 4's data is.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Header {
    pub edition: u32,
    pub master_table: u32,
    pub centre: u32,
    pub data_category: u32,
    pub master_table_version: u32,
    pub local_table_version: u32,
    pub subsets: u32,
    pub observed: bool,
    pub compressed: bool,
    /// Section 3's descriptors as the tables write them, F then X in two
    /// digits then Y in three: 0-12-101 is `12101`.
    pub descriptors: Vec<u32>,
    /// Where section 4's data starts in the message, past its four-byte
    /// header, and how many bytes of it there are.
    pub data_offset: usize,
    pub data_len: usize,
}

/// What a value is, beyond being a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A Table B element.
    Element,
    /// A Table B element that is a delayed replication's count.
    Count,
    /// The field 204 puts in front of an element: its descriptor is the
    /// element's.
    Associated,
    /// A new reference value for the element it names, from 203.
    NewReference,
    /// A centre's own descriptor, read at the width 206 gave it.
    Local,
    /// Characters 205 put in the data.
    Characters,
    /// A value of the element a bitmap points at: a substituted value, a
    /// statistic, a replaced value. `refers_to` is that element.
    Marker,
    /// An operator, which has no data. Listed so the values read under it can
    /// be seen to follow it.
    Operator,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Element => "element",
            Role::Count => "count",
            Role::Associated => "associated",
            Role::NewReference => "reference",
            Role::Local => "local",
            Role::Characters => "characters",
            Role::Marker => "marker",
            Role::Operator => "operator",
        }
    }
}

/// One value, or no value.
#[derive(Debug, Clone, PartialEq)]
pub enum Datum {
    Missing,
    /// A measurement, already scaled.
    Number(f64),
    /// A code or flag table entry, a count, or anything else read as a whole
    /// number.
    Integer(i64),
    Text(String),
}

/// One thing read from section 4, or one operator met on the way.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// The descriptor, as the tables write it.
    pub code: u32,
    pub role: Role,
    /// Table B's name, or Table C's for an operator. Empty for a descriptor no
    /// table has.
    pub name: &'static str,
    pub unit: &'static str,
    /// The width, scale and reference it was read at, with every operator in
    /// force applied.
    pub width: u32,
    pub scale: i32,
    pub reference: i64,
    /// Where its bits start, counted from the first bit of section 4's data,
    /// and how many there are. For a compressed message that is the whole run:
    /// the smallest value, the six-bit width and every subset's difference.
    pub bit: u64,
    pub bits: u64,
    /// For a compressed message: the smallest packed number, and how wide the
    /// differences from it are. For text, the width counts bytes.
    pub base: Option<u64>,
    pub increment_width: Option<u32>,
    /// The packed number of each subset, before the reference and the scale.
    /// Empty for text and for an operator.
    pub packed: Vec<Option<u64>>,
    /// What it is worth, one per subset for a compressed message and one for
    /// an uncompressed one.
    pub values: Vec<Datum>,
    /// The element this one is about, as an index into the same subset's
    /// items: the element quality information is for, the element a marker is
    /// a value of, the element an associated field is in front of.
    pub refers_to: Option<usize>,
}

impl Item {
    /// The value for `subset`, as text: a number to as many decimal places as
    /// its scale gives, which is how precise the file says it is.
    pub fn text(&self, subset: usize) -> String {
        match self.values.get(subset).or_else(|| self.values.first()) {
            None | Some(Datum::Missing) => String::new(),
            Some(Datum::Number(v)) => format!("{v:.*}", self.scale.max(0) as usize),
            Some(Datum::Integer(v)) => v.to_string(),
            Some(Datum::Text(s)) => s.clone(),
        }
    }

    pub fn missing(&self, subset: usize) -> bool {
        matches!(self.values.get(subset).or_else(|| self.values.first()), Some(Datum::Missing))
    }
}

/// One descriptor of section 3 expanded through Table D, as deep as the
/// sequences it came out of.
#[derive(Debug, Clone, PartialEq)]
pub struct Expanded {
    pub code: u32,
    pub depth: u32,
}

/// One thing that was done, as a sentence.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub what: String,
}

/// What came out of a message.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reading {
    pub header: Header,
    /// Which version of the tables was used, which is not always the one
    /// section 1 names. See [`bufr_tables::for_version`].
    pub tables_version: u32,
    pub steps: Vec<Step>,
    /// Section 3's descriptors expanded through Table D, before any
    /// replication is done, and how many there are.
    pub expanded: Vec<Expanded>,
    /// The items of each subset in turn. A compressed message has one list,
    /// whose items hold a value for every subset.
    pub subsets: Vec<Vec<Item>>,
    /// How many of section 4's bits the values took.
    pub bits_read: u64,
    /// What stopped the reading, or what it could not say. What was read up to
    /// that point is still here.
    pub problem: Option<String>,
}

impl Reading {
    /// How many values were read, over every subset, operators not counted.
    pub fn value_count(&self) -> usize {
        let per = |items: &Vec<Item>| items.iter().filter(|i| i.role != Role::Operator).map(|i| i.values.len()).sum::<usize>();
        self.subsets.iter().map(per).sum()
    }
}

/// A descriptor as its three numbers.
pub fn fxy(code: u32) -> (u32, u32, u32) {
    (code / 100_000, (code / 1000) % 100, code % 1000)
}

/// A replication descriptor as a sentence: what it repeats and how often.
pub fn replication_text(x: u32, y: u32) -> String {
    let things = if x == 1 { "1 descriptor".to_string() } else { format!("{x} descriptors") };
    match y {
        0 => format!("Delayed replication of {things}"),
        1 => format!("Replicate {things} once"),
        _ => format!("Replicate {things} {y} times"),
    }
}

/// What the tables call a descriptor, whatever kind it is: Table B's name,
/// Table D's title, Table C's name, or what a replication does. Empty for one
/// no table has.
pub fn describe(t: &Tables, code: u32) -> String {
    let (f, x, y) = fxy(code);
    match f {
        0 => t.element(code).map_or(String::new(), |e| e.name.to_string()),
        1 => replication_text(x, y),
        2 => bufr_tables::operator_name(code).unwrap_or("").to_string(),
        _ => t.sequence(code).map_or(String::new(), |s| s.title.to_string()),
    }
}

fn u24(b: &[u8], at: usize) -> Option<usize> {
    Some((usize::from(*b.get(at)?) << 16) | (usize::from(*b.get(at + 1)?) << 8) | usize::from(*b.get(at + 2)?))
}

fn byte(b: &[u8], at: usize) -> Option<u32> {
    b.get(at).map(|v| u32::from(*v))
}

fn u16be(b: &[u8], at: usize) -> Option<u32> {
    Some((byte(b, at)? << 8) | byte(b, at + 1)?)
}

/// Sections 0 to 3 of a message, and where section 4's data is.
pub fn header(m: &[u8]) -> Result<Header, String> {
    if !m.starts_with(b"BUFR") {
        return Err("Not read: the message does not start with BUFR.".into());
    }
    let short = || "Not read: the message ends before section 4.".to_string();
    let edition = byte(m, 7).ok_or_else(short)?;
    if !(2..=4).contains(&edition) {
        return Err(format!("Not read: this is edition {edition}, and only editions 2, 3 and 4 say how long their sections are."));
    }
    let s1 = 8;
    let len1 = u24(m, s1).ok_or_else(short)?;
    let mut h = Header { edition, master_table: byte(m, s1 + 3).ok_or_else(short)?, ..Header::default() };
    let flags = match edition {
        4 => {
            h.centre = u16be(m, s1 + 4).ok_or_else(short)?;
            h.data_category = byte(m, s1 + 10).ok_or_else(short)?;
            h.master_table_version = byte(m, s1 + 13).ok_or_else(short)?;
            h.local_table_version = byte(m, s1 + 14).ok_or_else(short)?;
            byte(m, s1 + 9).ok_or_else(short)?
        }
        3 => {
            h.centre = byte(m, s1 + 5).ok_or_else(short)?;
            h.data_category = byte(m, s1 + 8).ok_or_else(short)?;
            h.master_table_version = byte(m, s1 + 10).ok_or_else(short)?;
            h.local_table_version = byte(m, s1 + 11).ok_or_else(short)?;
            byte(m, s1 + 7).ok_or_else(short)?
        }
        _ => {
            h.centre = u16be(m, s1 + 4).ok_or_else(short)?;
            h.data_category = byte(m, s1 + 8).ok_or_else(short)?;
            h.master_table_version = byte(m, s1 + 10).ok_or_else(short)?;
            h.local_table_version = byte(m, s1 + 11).ok_or_else(short)?;
            byte(m, s1 + 7).ok_or_else(short)?
        }
    };
    let mut s3 = s1 + len1;
    if flags & 0x80 != 0 {
        s3 += u24(m, s3).ok_or_else(short)?;
    }
    let len3 = u24(m, s3).ok_or_else(short)?;
    h.subsets = u16be(m, s3 + 4).ok_or_else(short)?;
    let f3 = byte(m, s3 + 6).ok_or_else(short)?;
    h.observed = f3 & 0x80 != 0;
    h.compressed = f3 & 0x40 != 0;
    let n = len3.saturating_sub(7) / 2;
    for k in 0..n {
        let v = u16be(m, s3 + 7 + 2 * k).ok_or_else(short)?;
        h.descriptors.push(bufr_tables::code(v >> 14, (v >> 8) & 0x3F, v & 0xFF));
    }
    let s4 = s3 + len3;
    let len4 = u24(m, s4).ok_or_else(short)?;
    h.data_offset = (s4 + 4).min(m.len());
    h.data_len = len4.saturating_sub(4).min(m.len().saturating_sub(h.data_offset));
    Ok(h)
}

/// A whole message read: its header, its descriptors expanded, and the values
/// of every subset.
pub fn read(message: &[u8]) -> Reading {
    let mut out = Reading::default();
    let h = match header(message) {
        Ok(h) => h,
        Err(problem) => {
            out.problem = Some(problem);
            return out;
        }
    };
    out.header = h.clone();
    let t = bufr_tables::for_version(h.master_table_version);
    out.tables_version = t.version;
    out.steps.push(Step {
        what: if t.version == h.master_table_version {
            format!("Used version {} of the WMO tables, as section 1 says", t.version)
        } else {
            format!("Used version {} of the WMO tables: section 1 says version {}, which is not bundled", t.version, h.master_table_version)
        },
    });
    if h.master_table != 0 {
        out.problem = Some(format!("Not read: this uses master table {}, and only master table 0, meteorology, is bundled.", h.master_table));
        return out;
    }

    let mut expanded = Vec::new();
    let mut unknown = None;
    expand(t, &h.descriptors, 0, &mut expanded, &mut unknown);
    out.steps.push(Step {
        what: if expanded.len() == h.descriptors.len() {
            format!("No sequences to expand: {}", count(h.descriptors.len(), "descriptor"))
        } else {
            format!("Expanded {} through Table D into {}", count(h.descriptors.len(), "descriptor"), count(expanded.len(), "descriptor"))
        },
    });
    if let Some(code) = unknown {
        // Only a note: the walk may never reach the sequence, when a
        // replication around it repeats nothing, and a walk that does reach
        // it stops there and says so itself.
        out.steps.push(Step {
            what: format!("{} is not in version {} of Table D, so the descriptor list is not fully expanded", written(code), t.version),
        });
    }
    out.expanded = expanded;

    let data = &message[h.data_offset..h.data_offset + h.data_len];
    let mut bits = Bits::new(data);
    let subsets = h.subsets as usize;
    let mut effects = Effects::default();
    let mut budget = VALUE_LIMIT;
    if h.compressed {
        let mut w = Walk::new(t, subsets, true, &mut bits, &mut effects, &mut budget);
        let result = w.walk(&h.descriptors);
        let items = std::mem::take(&mut w.items);
        let bitmaps = std::mem::take(&mut w.bitmap.defined);
        drop(w);
        out.subsets.push(items);
        if let Err(Stop(why)) = result {
            out.problem = Some(why);
        }
        effects.bitmaps = bitmaps;
    } else {
        for s in 0..subsets {
            let mut w = Walk::new(t, 1, false, &mut bits, &mut effects, &mut budget);
            let result = w.walk(&h.descriptors);
            let items = std::mem::take(&mut w.items);
            let bitmaps = std::mem::take(&mut w.bitmap.defined);
            drop(w);
            out.subsets.push(items);
            if s == 0 {
                effects.bitmaps = bitmaps;
            }
            if let Err(Stop(why)) = result {
                out.problem = Some(if subsets > 1 { format!("In subset {}: {why}", s + 1) } else { why });
                break;
            }
        }
    }
    out.bits_read = bits.at as u64;

    let values = count(out.value_count(), "value");
    let bits_read = count(out.bits_read as usize, "bit");
    // With several subsets the count is of all of them together, and says so.
    let all = if subsets > 1 { " in all" } else { "" };
    out.steps.push(Step {
        what: if h.compressed {
            format!(
                "Read {} compressed, each element as its smallest packed number, a 6-bit width and one difference per subset: {values}{all} in {bits_read}",
                count(subsets, "subset"),
            )
        } else if subsets == 1 {
            format!("Read 1 subset: {values} in {bits_read}")
        } else {
            format!("Read {} one after another: {values}{all} in {bits_read}", count(subsets, "subset"))
        },
    });
    for bitmap in &effects.bitmaps {
        let present = bitmap.iter().filter(|b| **b).count();
        out.steps.push(Step { what: format!("Defined a bitmap of {}: {present} of the elements it covers are marked", count(bitmap.len(), "bit")) });
    }
    for (code, n) in &effects.counts {
        out.steps.push(Step { what: effect_text(*code, *n) });
    }
    let total = (h.data_len * 8) as u64;
    if out.problem.is_none() {
        let left = total.saturating_sub(out.bits_read);
        out.steps.push(Step {
            what: if left < 8 {
                format!("Section 4 is used to its last byte, with {} of padding", count(left as usize, "bit"))
            } else {
                format!("Section 4 has {} left over after the last value", count(left as usize, "bit"))
            },
        });
    }
    out
}

/// What an operator did, as a step: its descriptor and name, and how many
/// values it changed.
fn effect_text(code: u32, n: u64) -> String {
    let (_, x, y) = fxy(code);
    let name = bufr_tables::operator_name(code).unwrap_or("");
    let values = count(n as usize, "value");
    let signed = |v: i64| if v >= 0 { format!("+{v}") } else { v.to_string() };
    let detail = match x {
        1 => format!("{} bits of width on {values}", signed(i64::from(y) - 128)),
        2 => format!("scale {} on {values}", signed(i64::from(y) - 128)),
        3 => format!("{values} read with a new reference value"),
        4 => format!("{y} bits before each of {values}"),
        5 => format!("{values} of {y} characters"),
        6 => format!("{values} read {y} bits wide"),
        7 => format!("scale +{y}, reference × 10^{y}, width +{} on {values}", (10 * y + 2) / 3),
        8 => format!("{values} read {y} characters long"),
        21 => format!("{values} not present"),
        22 | 23 | 24 | 25 | 32 => format!("{values}, one for each element the bitmap marks"),
        _ => values,
    };
    format!("{} {name}: {detail}", written(code))
}

/// A descriptor as the tables write it, six digits: `012101`. The same way
/// the template names descriptors in section 3, so the two can be matched by
/// eye.
pub fn written(code: u32) -> String {
    format!("{code:06}")
}

fn count(n: usize, noun: &str) -> String {
    let digits = crate::encode::commas(n as u64);
    if n == 1 { format!("{digits} {noun}") } else { format!("{digits} {noun}s") }
}

/// Section 3's descriptors expanded through Table D. Replications are kept as
/// they are, since how many times a delayed one repeats is in the data.
fn expand(t: &Tables, list: &[u32], depth: u32, out: &mut Vec<Expanded>, unknown: &mut Option<u32>) {
    for &code in list {
        if out.len() >= VALUE_LIMIT {
            return;
        }
        out.push(Expanded { code, depth });
        if code / 100_000 == 3 {
            match t.sequence(code) {
                Some(s) if depth < DEPTH_LIMIT => expand(t, &s.members, depth + 1, out, unknown),
                Some(_) => {}
                None => {
                    unknown.get_or_insert(code);
                }
            }
        }
    }
}

/// Why a walk stopped.
struct Stop(String);

/// What the operators did over the whole message, for the steps.
#[derive(Default)]
struct Effects {
    /// Operator descriptor to how many values were read under it.
    counts: BTreeMap<u32, u64>,
    /// The bitmaps the first subset defined, true for an element with a value.
    bitmaps: Vec<Vec<bool>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Stage {
    #[default]
    None,
    /// Just past a 2YY000 operator: the next descriptor says whether a new
    /// bitmap follows or an old one is used again.
    Announced,
    /// A new bitmap is coming, and no 0-31-031 has been read yet.
    Waiting,
    /// Reading 0-31-031 values.
    Counting,
}

/// Where a walk is with bitmaps.
#[derive(Default)]
struct Bitmap {
    stage: Stage,
    reuse: bool,
    bits: usize,
    /// How many items there were at the most recent 2YY000 operator.
    boundary: usize,
    /// The elements a bitmap covers, fixed by the first one and kept until
    /// 235000 cancels them.
    covered: Option<Vec<usize>>,
    /// The elements the most recent bitmap says have a value, and how far
    /// through them the markers and the quality information have got.
    pointed: Vec<usize>,
    next: usize,
    /// Every bitmap defined, for the steps.
    defined: Vec<Vec<bool>>,
}

/// Whether 222000 has been met, and whether the quality information after its
/// bitmap has started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Quality {
    #[default]
    None,
    Waiting,
    Reading,
}

/// One pass over the descriptors: one subset of an uncompressed message, or
/// every subset of a compressed one.
struct Walk<'a, 'b> {
    t: &'static Tables,
    bits: &'b mut Bits<'a>,
    /// How many values each item holds.
    n: usize,
    compressed: bool,
    items: Vec<Item>,
    /// The operators in force, each as the descriptor that set it.
    width: Option<u32>,
    scale: Option<u32>,
    raise: Option<u32>,
    new_reference: Option<u32>,
    /// Each element's new reference value, and the 203 that defined it.
    references: HashMap<u32, (i64, u32)>,
    associated: Vec<u32>,
    local: Option<u32>,
    characters: Option<u32>,
    not_present: Option<(u32, u32)>,
    quality: Quality,
    bitmap: Bitmap,
    effects: &'b mut Effects,
    budget: &'b mut usize,
    depth: u32,
}

impl<'a, 'b> Walk<'a, 'b> {
    fn new(t: &'static Tables, n: usize, compressed: bool, bits: &'b mut Bits<'a>, effects: &'b mut Effects, budget: &'b mut usize) -> Self {
        Walk {
            t,
            bits,
            n,
            compressed,
            items: Vec::new(),
            width: None,
            scale: None,
            raise: None,
            new_reference: None,
            references: HashMap::new(),
            associated: Vec::new(),
            local: None,
            characters: None,
            not_present: None,
            quality: Quality::None,
            bitmap: Bitmap::default(),
            effects,
            budget,
            depth: 0,
        }
    }

    fn effect(&mut self, code: u32) {
        *self.effects.counts.entry(code).or_default() += 1;
    }

    fn walk(&mut self, list: &[u32]) -> Result<(), Stop> {
        let mut i = 0;
        while i < list.len() {
            let code = list[i];
            let (f, x, y) = fxy(code);
            // 221: the next Y descriptors have no data, apart from the
            // classes that say where and when.
            if let Some((op, left)) = self.not_present {
                self.not_present = if left > 1 { Some((op, left - 1)) } else { None };
                if f == 0 && !(1..=9).contains(&x) && x != 31 {
                    self.effect(op);
                    i += 1;
                    continue;
                }
            }
            if f == 0 {
                if let Some(op) = self.new_reference {
                    self.new_reference_value(code, op)?;
                    i += 1;
                    continue;
                }
            }
            if let Some(op) = self.local.take() {
                self.local_value(code, op)?;
                i += 1;
                continue;
            }
            if self.bitmap.stage != Stage::None {
                self.bitmap_stage(code);
            }
            match f {
                0 => {
                    self.element(code, Role::Element)?;
                    i += 1;
                }
                1 => {
                    let span = x as usize;
                    if y == 0 {
                        let Some(&factor) = list.get(i + 1) else {
                            return Err(Stop(format!("{} has no replication factor after it.", written(code))));
                        };
                        if !matches!(factor, 31000 | 31001 | 31002 | 31011 | 31012) {
                            return Err(Stop(format!(
                                "{} is followed by {}, which is not a replication factor.",
                                written(code),
                                written(factor)
                            )));
                        }
                        let at = self.element(factor, Role::Count)?;
                        let times = self.count_of(at)?;
                        let body = body(list, i + 2, span, code)?;
                        if matches!(factor, 31011 | 31012) {
                            // Delayed repetition: the data is written once and
                            // stands for every repetition.
                            if times > 0 {
                                let from = self.items.len();
                                self.walk(body)?;
                                let once: Vec<Item> = self.items[from..].to_vec();
                                for _ in 1..times {
                                    self.spend(once.len())?;
                                    self.items.extend(once.iter().cloned());
                                }
                            }
                        } else {
                            for _ in 0..times {
                                self.walk(body)?;
                            }
                        }
                        i += 2 + span;
                    } else {
                        let body = body(list, i + 1, span, code)?;
                        for _ in 0..y {
                            self.walk(body)?;
                        }
                        i += 1 + span;
                    }
                }
                2 => {
                    self.operator(code)?;
                    i += 1;
                }
                _ => {
                    let Some(s) = self.t.sequence(code) else {
                        return Err(Stop(format!(
                            "{} is not in version {} of Table D, so what it expands to is unknown and nothing after it can be read.",
                            written(code),
                            self.t.version
                        )));
                    };
                    if self.depth >= DEPTH_LIMIT {
                        return Err(Stop(format!("Table D sequences nest more than {DEPTH_LIMIT} deep at {}.", written(code))));
                    }
                    self.depth += 1;
                    let members = s.members.clone();
                    self.walk(&members)?;
                    self.depth -= 1;
                    i += 1;
                }
            }
        }
        Ok(())
    }

    /// Takes `n` values off the budget, or stops when it has run out.
    fn spend(&mut self, n: usize) -> Result<(), Stop> {
        let cost = n * self.n;
        if cost > *self.budget {
            return Err(Stop(format!(
                "The message has more than {} values, this viewer's limit, so reading stopped there.",
                crate::encode::commas(VALUE_LIMIT as u64)
            )));
        }
        *self.budget -= cost;
        Ok(())
    }

    /// A delayed replication's count. In a compressed message every subset
    /// must have the same one, since they share their descriptors.
    fn count_of(&self, at: usize) -> Result<usize, Stop> {
        let item = &self.items[at];
        let mut found = None;
        for v in &item.values {
            let Datum::Integer(c) = v else {
                return Err(Stop(format!("The replication factor {} is missing, so the number of repeats is unknown.", written(item.code))));
            };
            if found.is_some_and(|f| f != *c) {
                return Err(Stop(format!(
                    "The replication factor {} differs between subsets, which a compressed message cannot have.",
                    written(item.code)
                )));
            }
            found = Some(*c);
        }
        Ok(found.unwrap_or(0).max(0) as usize)
    }

    fn operator(&mut self, code: u32) -> Result<(), Stop> {
        let (_, x, y) = fxy(code);
        let set = |y: u32| (y != 0).then_some(code);
        match x {
            1 => self.width = set(y),
            2 => self.scale = set(y),
            3 => match y {
                255 => self.new_reference = None,
                0 => {
                    self.new_reference = None;
                    self.references.clear();
                }
                _ => self.new_reference = Some(code),
            },
            4 => {
                if y == 0 {
                    self.associated.pop();
                } else {
                    self.associated.push(code);
                }
            }
            5 => return self.characters_value(code, y),
            6 => self.local = Some(code),
            7 => self.raise = set(y),
            8 => self.characters = set(y),
            21 => self.not_present = (y > 0).then_some((code, y)),
            22 | 23 | 24 | 25 | 32 if y == 0 => {
                self.bitmap.stage = Stage::Announced;
                self.bitmap.boundary = self.items.len();
                if x == 22 {
                    self.quality = Quality::Waiting;
                }
            }
            23 | 24 | 25 | 32 if y == 255 => return self.marker(code),
            35 if y == 0 => {
                self.bitmap.covered = None;
                self.bitmap.pointed.clear();
            }
            36 if y == 0 => {}
            37 if y == 0 => self.bitmap.next = 0,
            37 if y == 255 => {
                if self.bitmap.reuse {
                    self.bitmap.pointed.clear();
                }
            }
            41 | 42 | 43 if y == 0 || y == 255 => {}
            _ => {
                return Err(Stop(format!("{} is an operator this viewer does not know, so nothing after it can be read.", written(code))));
            }
        }
        self.push_operator(code);
        Ok(())
    }

    fn push_operator(&mut self, code: u32) {
        let bit = self.bits.at as u64;
        self.items.push(Item {
            code,
            role: Role::Operator,
            name: bufr_tables::operator_name(code).unwrap_or(""),
            unit: "",
            width: 0,
            scale: 0,
            reference: 0,
            bit,
            bits: 0,
            base: None,
            increment_width: None,
            packed: Vec::new(),
            values: Vec::new(),
            refers_to: None,
        });
    }

    /// Where the walk is with a bitmap, given the descriptor about to be read.
    fn bitmap_stage(&mut self, code: u32) {
        match self.bitmap.stage {
            Stage::Announced => match code {
                236000 => {
                    self.bitmap.reuse = true;
                    self.bitmap.stage = Stage::Waiting;
                    self.bitmap.bits = 0;
                }
                237000 => self.bitmap.stage = Stage::None,
                // A bitmap written straight after the operator rather than
                // by a replication: this is its first bit.
                31031 => {
                    self.bitmap.reuse = false;
                    self.bitmap.stage = Stage::Counting;
                    self.bitmap.bits = 1;
                }
                _ => {
                    self.bitmap.reuse = false;
                    self.bitmap.stage = Stage::Waiting;
                    self.bitmap.bits = 0;
                }
            },
            Stage::Waiting => {
                if code == 31031 {
                    self.bitmap.stage = Stage::Counting;
                    self.bitmap.bits = 1;
                }
            }
            Stage::Counting => {
                if code == 31031 {
                    self.bitmap.bits += 1;
                } else {
                    self.define_bitmap();
                    self.bitmap.stage = Stage::None;
                }
            }
            Stage::None => {}
        }
    }

    /// The bitmap just read: the last run of 0-31-031 values, matched against
    /// the elements before the first bitmap operator.
    fn define_bitmap(&mut self) {
        let n = self.bitmap.bits;
        let present: Vec<bool> = self.items[self.items.len().saturating_sub(n)..]
            .iter()
            .map(|it| matches!(it.values.first(), Some(Datum::Integer(0))))
            .collect();
        if self.bitmap.covered.is_none() {
            let mut covered = Vec::new();
            for k in (0..self.bitmap.boundary.min(self.items.len())).rev() {
                if matches!(self.items[k].role, Role::Element | Role::Count) {
                    covered.push(k);
                    if covered.len() == n {
                        break;
                    }
                }
            }
            covered.reverse();
            self.bitmap.covered = Some(covered);
        }
        let covered = self.bitmap.covered.clone().unwrap_or_default();
        self.bitmap.pointed = covered.iter().zip(&present).filter(|(_, p)| **p).map(|(k, _)| *k).collect();
        self.bitmap.next = 0;
        self.bitmap.defined.push(present);
    }

    /// The next element the bitmap says has a value, or nothing past its end.
    fn next_pointed(&mut self) -> Option<usize> {
        let k = self.bitmap.pointed.get(self.bitmap.next).copied();
        self.bitmap.next += 1;
        k
    }

    /// Whether this element takes a value of the class 33 quality information
    /// that follows 222000, and which element it is for.
    fn quality_link(&mut self, x: u32) -> Option<usize> {
        if x == 33 {
            if self.quality == Quality::Waiting {
                self.quality = Quality::Reading;
            }
            if self.quality == Quality::Reading {
                return self.next_pointed();
            }
        } else if self.quality == Quality::Reading {
            self.quality = Quality::None;
        }
        None
    }

    fn element(&mut self, code: u32, role: Role) -> Result<usize, Stop> {
        let (_, x, _) = fxy(code);
        let Some(e) = self.t.element(code) else {
            // A local descriptor is in no version of the WMO's table, so the
            // version is not worth naming for one.
            return Err(Stop(if x >= 48 || code % 1000 >= 192 {
                format!(
                    "{} is a local descriptor, defined by the originating centre and not in Table B, so its width is unknown and nothing after it can be read.",
                    written(code)
                )
            } else {
                format!(
                    "{} is not in version {} of Table B, so its width is unknown and nothing after it can be read.",
                    written(code),
                    self.t.version
                )
            }));
        };
        if !self.associated.is_empty() && x != 31 {
            self.associated_value(code, e)?;
        }
        let link = self.quality_link(x);
        let at = self.value(code, role, e, None)?;
        if link.is_some() {
            self.items[at].refers_to = link;
            self.effect(222000);
        }
        Ok(at)
    }

    /// One Table B value, read at the width, scale and reference the operators
    /// in force give it. `against` is a marker's: the element the bitmap points
    /// at, whose descriptor this is read as, and 225255's wider difference.
    fn value(&mut self, code: u32, role: Role, e: &'static Element, against: Option<(u32, bool)>) -> Result<usize, Stop> {
        let (_, x, _) = fxy(code);
        let kind = e.kind();
        if kind == Kind::Text {
            let bytes = match self.characters {
                Some(op) => {
                    self.effect(op);
                    fxy(op).2
                }
                None => e.width / 8,
            };
            return self.text(against.map_or(code, |a| a.0), role, e.name, e.unit, bytes);
        }
        let (mut width, mut scale, mut reference) = (i64::from(e.width), e.scale, e.reference);
        if kind == Kind::Numeric && x != 31 {
            if let Some(op) = self.raise {
                let y = fxy(op).2;
                width += i64::from((10 * y + 2) / 3);
                scale += y as i32;
                reference = reference.saturating_mul(10i64.saturating_pow(y));
                self.effect(op);
            }
            if let Some(op) = self.width {
                width += i64::from(fxy(op).2) - 128;
                self.effect(op);
            }
            if let Some(op) = self.scale {
                scale += fxy(op).2 as i32 - 128;
                self.effect(op);
            }
        }
        if let Some((r, op)) = self.references.get(&code).copied() {
            reference = r;
            self.effect(op);
        }
        if let Some((_, difference)) = against {
            if difference {
                reference = -(1i64 << width.clamp(0, 62));
                width += 1;
            }
        }
        if !(0..=64).contains(&width) {
            return Err(Stop(format!("{} would be {width} bits wide, a width this viewer cannot read.", written(code))));
        }
        let width = width as u32;
        let missing = code != 31031 && width > 1;
        let (bit, packed, base, increment_width) = self.packed(width, missing, code)?;
        let values = packed
            .iter()
            .map(|p| match p {
                None => Datum::Missing,
                Some(v) if kind == Kind::Numeric => {
                    let whole = i128::from(*v) + i128::from(reference);
                    if scale == 0 {
                        Datum::Integer(whole as i64)
                    } else {
                        Datum::Number(whole as f64 / 10f64.powi(scale))
                    }
                }
                Some(v) => Datum::Integer(*v as i64),
            })
            .collect();
        self.spend(1)?;
        let bits = self.bits.at as u64 - bit;
        self.items.push(Item {
            code: against.map_or(code, |a| a.0),
            role,
            name: e.name,
            unit: e.unit,
            width,
            scale: if kind == Kind::Numeric { scale } else { 0 },
            reference: if kind == Kind::Numeric { reference } else { 0 },
            bit,
            bits,
            base,
            increment_width,
            packed,
            values,
            refers_to: None,
        });
        Ok(self.items.len() - 1)
    }

    /// A packed number `width` bits wide: once, or for a compressed message a
    /// smallest value, a six-bit width, and a difference per subset.
    #[allow(clippy::type_complexity)]
    fn packed(&mut self, width: u32, can_be_missing: bool, code: u32) -> Result<(u64, Vec<Option<u64>>, Option<u64>, Option<u32>), Stop> {
        let bit = self.bits.at as u64;
        let ended = || Stop(format!("Section 4 ends inside the value of {}.", written(code)));
        let ones = |w: u32| if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        let one = self.bits.take(width).ok_or_else(ended)?;
        if !self.compressed {
            let v = (!(can_be_missing && one == ones(width))).then_some(one);
            return Ok((bit, vec![v], None, None));
        }
        let wide = self.bits.take(6).ok_or_else(ended)? as u32;
        let mut out = Vec::with_capacity(self.n);
        if wide == 0 {
            let v = (!(can_be_missing && one == ones(width))).then_some(one);
            out.resize(self.n, v);
        } else {
            for _ in 0..self.n {
                let d = self.bits.take(wide).ok_or_else(ended)?;
                out.push((!(can_be_missing && d == ones(wide))).then(|| one.wrapping_add(d)));
            }
        }
        Ok((bit, out, Some(one), Some(wide)))
    }

    /// Text `bytes` characters long, once or compressed.
    fn text(&mut self, code: u32, role: Role, name: &'static str, unit: &'static str, bytes: u32) -> Result<usize, Stop> {
        let bit = self.bits.at as u64;
        let ended = || Stop(format!("Section 4 ends inside the text of {}.", written(code)));
        let first = self.bits.bytes(bytes as usize).ok_or_else(ended)?;
        let mut values = Vec::new();
        let mut increment_width = None;
        if self.compressed {
            let wide = self.bits.take(6).ok_or_else(ended)? as usize;
            increment_width = Some(wide as u32);
            if wide == 0 {
                values.resize(self.n, characters(&first));
            } else {
                for _ in 0..self.n {
                    values.push(characters(&self.bits.bytes(wide).ok_or_else(ended)?));
                }
            }
        } else {
            values.push(characters(&first));
        }
        self.spend(1)?;
        let bits = self.bits.at as u64 - bit;
        self.items.push(Item {
            code,
            role,
            name,
            unit,
            width: bytes * 8,
            scale: 0,
            reference: 0,
            bit,
            bits,
            base: None,
            increment_width,
            packed: Vec::new(),
            values,
            refers_to: None,
        });
        Ok(self.items.len() - 1)
    }

    /// 204's field in front of an element, as wide as every 204 in force
    /// together.
    fn associated_value(&mut self, code: u32, e: &'static Element) -> Result<(), Stop> {
        let width: u32 = self.associated.iter().map(|op| fxy(*op).2).sum();
        let ops = self.associated.clone();
        for op in ops {
            self.effect(op);
        }
        let (bit, packed, base, increment_width) = self.packed(width, false, code)?;
        self.spend(1)?;
        let values = packed.iter().map(|p| p.map_or(Datum::Missing, |v| Datum::Integer(v as i64))).collect();
        let bits = self.bits.at as u64 - bit;
        self.items.push(Item {
            code,
            role: Role::Associated,
            name: e.name,
            unit: "",
            width,
            scale: 0,
            reference: 0,
            bit,
            bits,
            base,
            increment_width,
            packed,
            values,
            // The element is the item after this one.
            refers_to: Some(self.items.len() + 1),
        });
        Ok(())
    }

    /// 203: a new reference value for `code`, sign and magnitude, in place of
    /// a value.
    fn new_reference_value(&mut self, code: u32, op: u32) -> Result<(), Stop> {
        let Some(e) = self.t.element(code) else {
            return Err(Stop(format!("{} is not in version {} of Table B, so it has no reference value to change.", written(code), self.t.version)));
        };
        let width = fxy(op).2;
        let (bit, packed, base, increment_width) = self.packed(width, false, code)?;
        let signed = |v: u64| {
            let top = 1u64 << (width - 1).min(63);
            if v & top != 0 { -((v & (top - 1)) as i64) } else { v as i64 }
        };
        let reference = packed.first().copied().flatten().map_or(0, signed);
        self.references.insert(code, (reference, op));
        self.effect(op);
        self.spend(1)?;
        let bits = self.bits.at as u64 - bit;
        let values = packed.iter().map(|p| p.map_or(Datum::Missing, |v| Datum::Integer(signed(v)))).collect();
        self.items.push(Item {
            code,
            role: Role::NewReference,
            name: e.name,
            unit: "",
            width,
            scale: 0,
            reference: 0,
            bit,
            bits,
            base,
            increment_width,
            packed,
            values,
            refers_to: None,
        });
        Ok(())
    }

    /// 206: the next descriptor, whatever it is, is `Y` bits of a whole number.
    fn local_value(&mut self, code: u32, op: u32) -> Result<(), Stop> {
        let width = fxy(op).2;
        let (bit, packed, base, increment_width) = self.packed(width, width > 1, code)?;
        self.effect(op);
        self.spend(1)?;
        let bits = self.bits.at as u64 - bit;
        let values = packed.iter().map(|p| p.map_or(Datum::Missing, |v| Datum::Integer(v as i64))).collect();
        self.items.push(Item {
            code,
            role: Role::Local,
            name: self.t.element(code).map_or("", |e| e.name),
            unit: "",
            width,
            scale: 0,
            reference: 0,
            bit,
            bits,
            base,
            increment_width,
            packed,
            values,
            refers_to: None,
        });
        Ok(())
    }

    /// 205: `y` characters of text, named by the operator.
    fn characters_value(&mut self, code: u32, y: u32) -> Result<(), Stop> {
        self.effect(code);
        self.text(code, Role::Characters, bufr_tables::operator_name(code).unwrap_or(""), "", y)?;
        Ok(())
    }

    /// 223255, 224255, 225255 or 232255: a value of the next element the bitmap
    /// points at, read as that element.
    fn marker(&mut self, code: u32) -> Result<(), Stop> {
        let Some(target) = self.next_pointed() else {
            return Err(Stop(format!("The bitmap has no marked element left for {} to refer to.", written(code))));
        };
        let element = self.items[target].code;
        let Some(e) = self.t.element(element) else {
            return Err(Stop(format!("{} refers to {}, which is not in Table B.", written(code), written(element))));
        };
        if !self.associated.is_empty() {
            self.associated_value(element, e)?;
        }
        self.effect(code);
        let at = self.value(element, Role::Marker, e, Some((code, fxy(code).1 == 25)))?;
        self.items[at].refers_to = Some(target);
        Ok(())
    }
}

/// The X descriptors a replication repeats, from `start`.
fn body(list: &[u32], start: usize, span: usize, code: u32) -> Result<&[u32], Stop> {
    list.get(start..start + span).ok_or_else(|| {
        Stop(format!("{} repeats {} but only {} follow it.", written(code), count(span, "descriptor"), list.len().saturating_sub(start)))
    })
}

/// CCITT IA5 text as a string: trailing spaces and nulls taken off, and a
/// field of all ones, which is how text says it has no value, as missing.
fn characters(b: &[u8]) -> Datum {
    if !b.is_empty() && b.iter().all(|c| *c == 0xFF) {
        return Datum::Missing;
    }
    let s: String = b.iter().map(|c| if (0x20..0x7F).contains(c) { char::from(*c) } else { ' ' }).collect();
    Datum::Text(s.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bits written most significant first.
    #[derive(Default)]
    struct Writer {
        bytes: Vec<u8>,
        at: usize,
    }

    impl Writer {
        fn put(&mut self, width: u32, v: u64) -> &mut Self {
            for k in (0..width).rev() {
                if self.at % 8 == 0 {
                    self.bytes.push(0);
                }
                if v >> k & 1 == 1 {
                    let last = self.bytes.len() - 1;
                    self.bytes[last] |= 0x80 >> (self.at % 8);
                }
                self.at += 1;
            }
            self
        }

        fn text(&mut self, s: &[u8]) -> &mut Self {
            for c in s {
                self.put(8, u64::from(*c));
            }
            self
        }
    }

    fn fxy16(code: u32) -> [u8; 2] {
        let (f, x, y) = fxy(code);
        (((f << 14) | (x << 8) | y) as u16).to_be_bytes()
    }

    fn u24(n: usize) -> [u8; 3] {
        let b = (n as u32).to_be_bytes();
        [b[1], b[2], b[3]]
    }

    /// An edition 4 message against version 46 of the tables with the given
    /// subsets, compression and descriptors, and section 4 holding `data`.
    fn message(subsets: u16, compressed: bool, descriptors: &[u32], data: &[u8]) -> Vec<u8> {
        message_v(46, subsets, compressed, descriptors, data)
    }

    fn message_v(version: u8, subsets: u16, compressed: bool, descriptors: &[u32], data: &[u8]) -> Vec<u8> {
        let mut s1 = u24(22).to_vec();
        s1.push(0);
        s1.extend_from_slice(&98u16.to_be_bytes());
        s1.extend_from_slice(&0u16.to_be_bytes());
        s1.extend_from_slice(&[0, 0, 2, 0, 0, version, 0]);
        s1.extend_from_slice(&2026u16.to_be_bytes());
        s1.extend_from_slice(&[9, 14, 12, 0, 0]);
        let mut s3 = u24(7 + 2 * descriptors.len()).to_vec();
        s3.push(0);
        s3.extend_from_slice(&subsets.to_be_bytes());
        s3.push(if compressed { 0xC0 } else { 0x80 });
        for d in descriptors {
            s3.extend_from_slice(&fxy16(*d));
        }
        let mut s4 = u24(4 + data.len()).to_vec();
        s4.push(0);
        s4.extend_from_slice(data);
        let body: Vec<u8> = [s1, s3, s4, b"7777".to_vec()].concat();
        let mut m = b"BUFR".to_vec();
        m.extend_from_slice(&u24(body.len() + 8));
        m.push(4);
        m.extend_from_slice(&body);
        m
    }

    fn values(r: &Reading, subset: usize) -> Vec<(u32, String)> {
        r.subsets[subset]
            .iter()
            .filter(|i| i.role != Role::Operator)
            .map(|i| (i.code, if i.missing(0) { "missing".to_string() } else { i.text(0) }))
            .collect()
    }

    #[test]
    fn a_station_and_a_temperature_read_as_their_values() {
        // 3-01-001 is a block number (7 bits) and a station number (10 bits),
        // then 0-12-101 is a temperature in hundredths of a kelvin, 16 bits.
        let mut w = Writer::default();
        w.put(7, 11).put(10, 518).put(16, 27315 + 250);
        let r = read(&message(1, false, &[301001, 12101], &w.bytes));
        assert_eq!(r.problem, None);
        assert_eq!(r.tables_version, 46);
        assert_eq!(values(&r, 0), vec![(1001, "11".into()), (1002, "518".into()), (12101, "275.65".into())]);
        let t = &r.subsets[0][2];
        assert_eq!((t.bit, t.bits, t.unit, t.scale), (17, 16, "K", 2));
        assert_eq!(r.bits_read, 33);
        let steps: Vec<&str> = r.steps.iter().map(|s| s.what.as_str()).collect();
        assert!(steps[1].contains("2 descriptors") && steps[1].contains("into 4"), "{steps:?}");
        assert!(steps.last().unwrap().contains("7 bits of padding"), "{steps:?}");
        // The expansion keeps the sequence and puts what it stands for under it.
        assert_eq!(r.expanded.iter().map(|e| (e.code, e.depth)).collect::<Vec<_>>(), vec![(301001, 0), (1001, 1), (1002, 1), (12101, 0)]);
    }

    #[test]
    fn all_ones_is_missing_except_in_one_bit() {
        let mut w = Writer::default();
        w.put(16, 0xFFFF).put(1, 1);
        let r = read(&message(1, false, &[12101, 31031], &w.bytes));
        assert_eq!(values(&r, 0), vec![(12101, "missing".into()), (31031, "1".into())]);
    }

    #[test]
    fn a_delayed_replication_repeats_as_many_times_as_the_data_says() {
        // 1-01-000 then 0-31-001, a count of 3, then three pressures.
        let mut w = Writer::default();
        w.put(8, 3).put(14, 850).put(14, 700).put(14, 500);
        let r = read(&message(1, false, &[101000, 31001, 7004], &w.bytes));
        assert_eq!(r.problem, None);
        let got = values(&r, 0);
        assert_eq!(got.len(), 4);
        assert_eq!(got[0], (31001, "3".into()));
        assert_eq!(r.subsets[0][0].role, Role::Count);
        // Pressure is in pascals scaled by -1: a packed 850 is 8500 Pa.
        assert_eq!(got[1], (7004, "8500".into()));
        assert_eq!(got[3], (7004, "5000".into()));
    }

    #[test]
    fn a_fixed_replication_repeats_y_times() {
        let mut w = Writer::default();
        w.put(16, 27315).put(16, 27000).put(16, 27415).put(16, 27100);
        let r = read(&message(1, false, &[102002, 12101, 12103], &w.bytes));
        assert_eq!(r.problem, None);
        let got = values(&r, 0);
        assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), vec![12101, 12103, 12101, 12103]);
        assert_eq!(got[2].1, "274.15");
    }

    #[test]
    fn subsets_of_an_uncompressed_message_follow_one_another() {
        let mut w = Writer::default();
        w.put(16, 27315).put(16, 27415);
        let r = read(&message(2, false, &[12101], &w.bytes));
        assert_eq!(r.subsets.len(), 2);
        assert_eq!(values(&r, 1), vec![(12101, "274.15".into())]);
        assert_eq!(r.subsets[1][0].bit, 16);
    }

    #[test]
    fn a_compressed_value_is_a_smallest_a_width_and_a_difference_per_subset() {
        // Three subsets. Temperatures of 273.15, 274.15 and missing: the
        // smallest is 27315, the differences are 0, 100 and all ones in 7 bits.
        // Then a station name the same in all three, and a block number that
        // differs, as text of width 0 and a number.
        let mut w = Writer::default();
        w.put(16, 27315).put(6, 7).put(7, 0).put(7, 100).put(7, 127);
        let r = read(&message(3, true, &[12101], &w.bytes));
        assert_eq!(r.problem, None);
        let t = &r.subsets[0][0];
        assert_eq!(t.values, vec![Datum::Number(273.15), Datum::Number(274.15), Datum::Missing]);
        assert_eq!((t.base, t.increment_width, t.bits), (Some(27315), Some(7), 16 + 6 + 21));
        assert_eq!(r.value_count(), 3);
    }

    #[test]
    fn compressed_text_counts_its_widths_in_bytes() {
        let mut w = Writer::default();
        // 0-01-019, long station or site name, is 32 characters. The smallest
        // is all zeros, then 3 bytes a subset.
        w.text(&[0u8; 32]).put(6, 3).text(b"ABC").text(b"XYZ");
        let r = read(&message(2, true, &[1019], &w.bytes));
        assert_eq!(r.problem, None);
        assert_eq!(r.subsets[0][0].values, vec![Datum::Text("ABC".into()), Datum::Text("XYZ".into())]);
        // And a width of zero is the same text in every subset.
        let mut w = Writer::default();
        let mut name = b"PRAHA".to_vec();
        name.resize(32, b' ');
        w.text(&name).put(6, 0);
        let r = read(&message(2, true, &[1019], &w.bytes));
        assert_eq!(r.subsets[0][0].values, vec![Datum::Text("PRAHA".into()), Datum::Text("PRAHA".into())]);
    }

    #[test]
    fn change_width_and_scale_apply_to_numbers_and_not_to_counts() {
        // 2-01-132 adds 4 bits, 2-02-129 adds 1 to the scale. Temperature is
        // then 20 bits at a scale of 3. The count after them keeps its 8.
        let mut w = Writer::default();
        w.put(20, 273_150).put(8, 1).put(16, 100);
        let r = read(&message(1, false, &[201132, 202129, 12101, 201000, 202000, 101000, 31001, 12101], &w.bytes));
        assert_eq!(r.problem, None);
        let got = values(&r, 0);
        assert_eq!(got[0], (12101, "273.150".into()));
        assert_eq!(got[1], (31001, "1".into()));
        assert_eq!(got[2], (12101, "1.00".into()));
        let steps: Vec<&str> = r.steps.iter().map(|s| s.what.as_str()).collect();
        assert!(steps.iter().any(|s| s.starts_with("201132 Change data width: +4 bits of width on 1 value")), "{steps:?}");
    }

    #[test]
    fn a_new_reference_value_is_sign_and_magnitude() {
        // 2-03-010: 0-12-101's reference becomes -5 (ten bits, the top one
        // set), then 2-03-255 ends the definitions, and the temperature is
        // read against it.
        let mut w = Writer::default();
        w.put(10, 0x200 | 5).put(16, 27320);
        let r = read(&message(1, false, &[203010, 12101, 203255, 12101], &w.bytes));
        assert_eq!(r.problem, None);
        let items: Vec<&Item> = r.subsets[0].iter().filter(|i| i.role != Role::Operator).collect();
        assert_eq!((items[0].role, items[0].values[0].clone()), (Role::NewReference, Datum::Integer(-5)));
        assert_eq!(items[1].values[0], Datum::Number(273.15));
        assert_eq!(items[1].reference, -5);
    }

    #[test]
    fn an_associated_field_comes_before_each_element_but_not_its_significance() {
        // 2-04-004, then 0-31-021 says what the four bits mean and has none in
        // front of it, then a temperature with four bits in front.
        let mut w = Writer::default();
        w.put(6, 1).put(4, 9).put(16, 27315);
        let r = read(&message(1, false, &[204004, 31021, 12101, 204000], &w.bytes));
        assert_eq!(r.problem, None);
        let items: Vec<&Item> = r.subsets[0].iter().filter(|i| i.role != Role::Operator).collect();
        assert_eq!(items.len(), 3);
        assert_eq!((items[0].code, items[0].role), (31021, Role::Element));
        assert_eq!((items[1].role, items[1].values[0].clone(), items[1].width), (Role::Associated, Datum::Integer(9), 4));
        assert_eq!(items[2].values[0], Datum::Number(273.15));
    }

    #[test]
    fn characters_in_the_data_and_a_changed_text_width() {
        // 2-05-003 is three characters; 2-08-004 makes the 32-character name
        // four characters.
        let mut w = Writer::default();
        w.text(b"abc").text(b"OKPR");
        let r = read(&message(1, false, &[205003, 208004, 1019, 208000], &w.bytes));
        assert_eq!(r.problem, None);
        let got = values(&r, 0);
        assert_eq!(got, vec![(205003, "abc".into()), (1019, "OKPR".into())]);
    }

    #[test]
    fn a_local_descriptor_is_read_at_the_width_206_gives_it() {
        let mut w = Writer::default();
        w.put(8, 42).put(16, 27315);
        let r = read(&message(1, false, &[206008, 12192, 12101], &w.bytes));
        assert_eq!(r.problem, None);
        let items: Vec<&Item> = r.subsets[0].iter().filter(|i| i.role != Role::Operator).collect();
        assert_eq!((items[0].code, items[0].role, items[0].values[0].clone()), (12192, Role::Local, Datum::Integer(42)));
        assert_eq!(items[1].values[0], Datum::Number(273.15));
    }

    #[test]
    fn data_not_present_skips_all_but_the_classes_that_place_it() {
        // 2-21-003: of the next three, the temperature has no bits and the
        // year and the block number, classes 4 and 1, do.
        let mut w = Writer::default();
        w.put(12, 2026).put(7, 11).put(16, 27315);
        let r = read(&message(1, false, &[221003, 4001, 12101, 1001, 12101], &w.bytes));
        assert_eq!(r.problem, None);
        let got = values(&r, 0);
        assert_eq!(got, vec![(4001, "2026".into()), (1001, "11".into()), (12101, "273.15".into())]);
    }

    #[test]
    fn increase_scale_reference_and_width_together() {
        // 2-07-001 on 0-12-101: scale 3, reference 0, width 16 + 4 = 20.
        let mut w = Writer::default();
        w.put(20, 273_150);
        let r = read(&message(1, false, &[207001, 12101, 207000], &w.bytes));
        assert_eq!(r.problem, None);
        let t = r.subsets[0].iter().find(|i| i.code == 12101).unwrap();
        assert_eq!((t.width, t.scale), (20, 3));
        assert_eq!(t.values[0], Datum::Number(273.15));
    }

    #[test]
    fn a_bitmap_points_substituted_values_at_the_elements_that_have_them() {
        // Two temperatures, then 2-23-000 and a bitmap of two bits saying the
        // second has a substituted value, then 2-23-255 reads one at the
        // temperature's width.
        let mut w = Writer::default();
        w.put(16, 27315).put(16, 28315).put(1, 1).put(1, 0).put(16, 28000);
        let r = read(&message(1, false, &[12101, 12103, 223000, 31031, 31031, 223255], &w.bytes));
        assert_eq!(r.problem, None);
        let marker = r.subsets[0].iter().find(|i| i.role == Role::Marker).expect("a marker");
        assert_eq!(marker.code, 223255);
        assert_eq!(marker.values[0], Datum::Number(280.0));
        let target = marker.refers_to.expect("points at an element");
        assert_eq!(r.subsets[0][target].code, 12103);
        assert!(r.steps.iter().any(|s| s.what.contains("bitmap of 2 bits: 1 of")), "{:?}", r.steps);
    }

    #[test]
    fn a_difference_marker_is_one_bit_wider_and_may_be_negative() {
        let mut w = Writer::default();
        // A temperature, a one-bit bitmap, then a 17-bit difference of -1.00
        // K: packed 65436 against a reference of -65536.
        w.put(16, 27315).put(1, 0).put(17, 65436);
        let r = read(&message(1, false, &[12101, 225000, 31031, 225255], &w.bytes));
        assert_eq!(r.problem, None);
        let marker = r.subsets[0].iter().find(|i| i.role == Role::Marker).unwrap();
        assert_eq!((marker.width, marker.reference), (17, -65536));
        assert_eq!(marker.values[0], Datum::Number(-1.0));
    }

    #[test]
    fn quality_information_is_attached_to_the_elements_the_bitmap_names() {
        let mut w = Writer::default();
        // Two temperatures, a bitmap saying only the first has quality
        // information, then 0-33-007, per cent confidence, of 70.
        w.put(16, 27315).put(16, 28315).put(1, 0).put(1, 1).put(7, 70);
        let r = read(&message(1, false, &[12101, 12103, 222000, 31031, 31031, 33007], &w.bytes));
        assert_eq!(r.problem, None);
        let q = r.subsets[0].iter().find(|i| i.code == 33007).unwrap();
        assert_eq!(q.values[0], Datum::Integer(70));
        assert_eq!(r.subsets[0][q.refers_to.unwrap()].code, 12101);
    }

    #[test]
    fn a_local_descriptor_with_no_width_stops_the_reading_and_says_why() {
        let mut w = Writer::default();
        w.put(16, 27315).put(16, 1);
        let r = read(&message(1, false, &[12101, 12204, 12101], &w.bytes));
        let problem = r.problem.clone().expect("a problem");
        assert!(problem.contains("012204") && problem.contains("local descriptor"), "{problem}");
        // What came before it is kept.
        assert_eq!(values(&r, 0), vec![(12101, "273.15".into())]);
    }

    #[test]
    fn a_version_that_is_not_bundled_says_which_was_used() {
        let r = read(&message_v(20, 1, false, &[12101], &[0x6A, 0xB3]));
        assert_eq!(r.tables_version, 46);
        assert!(r.steps[0].what.contains("version 20"), "{:?}", r.steps[0]);
        // Version 13 widens nothing here, and is bundled.
        let r = read(&message_v(13, 1, false, &[12101], &[0x6A, 0xB3]));
        assert_eq!(r.tables_version, 13);
        assert_eq!(values(&r, 0), vec![(12101, "273.15".into())]);
    }

    #[test]
    fn the_header_finds_section_4_past_an_optional_section() {
        let m = message(1, false, &[12101], &[0x6A, 0xB3]);
        let h = header(&m).unwrap();
        assert_eq!((h.edition, h.centre, h.master_table_version, h.subsets, h.compressed), (4, 98, 46, 1, false));
        assert_eq!(&m[h.data_offset..h.data_offset + h.data_len], &[0x6A, 0xB3]);
        assert!(header(b"GRIB").is_err());
    }
}
