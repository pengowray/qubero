//! How a format writes its values and finds its parts, counted twice: over what
//! the template declares, and over the fields of the file in front of the
//! reader.
//!
//! The report's "How the format is built" section and its line of landmarks
//! are written from this. The two counts answer different questions and a
//! reader wants both: the template allows twelve kinds of chunk, and this file
//! uses four; the format has 64-bit offsets, and this file has three of them.
//!
//! **Facts, not words.** What comes back is a table of rows, each a category
//! (`number`, `text`, `placement`, ...), a kind within it, and how many fields
//! and bits of the file it covers. The web writes the sentences, the way it
//! does for `time.rs` and `problem.rs`. Every key here is part of the
//! interface and stays put.
//!
//! **A field can count in more than one row.** An enum is an enum and also the
//! 32-bit little-endian number under it; a length is a number and also how its
//! neighbour is sized. The rows of one category are about one question each,
//! and adding up rows from different categories adds up nothing.
//!
//! The template's count is a walk over the declared types, which follows each
//! named type once: a directory of directories is one declaration however deep
//! a file nests it. The file's count is taken on the walk the byte ledger and
//! the kind totals share (see `survey.rs`), so it works on a file of any size
//! a step at a time.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::shape::{expr_sizing, Sizing};
use super::*;
use crate::template::{Endian, StrLen};

/// One kind of thing a format does, and how much of it there is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRow {
    /// Which question this row answers. One of `number`, `text`, `varint`,
    /// `bit-field`, `enum`, `flags`, `magic`, `computed`, `opaque`, `padding`,
    /// `checksum`, `codec`, `placement`, `sizing`.
    pub category: &'static str,
    /// The answer within the category. See [`value_keys`] for the values,
    /// [`sizing_kind`] for `sizing`, and for `placement`: `follows` (after the
    /// field before it), `overlap` (a field of a union), `element` (one of a
    /// list laid end to end), `pointer-list`, `chain`, `gather`, the four
    /// kinds of address [`at_kind`] names, `stream` (the front of what a
    /// stream unpacked to) and `trace` (where a decoder read it).
    pub kind: String,
    /// Bits wide, for a number: 0 where the file sets the width. The
    /// alignment in bytes, for padding. 0 everywhere else.
    pub width: u32,
    /// Byte order for a number wider than a byte: `little` or `big`, and
    /// `none` for one byte or less. Empty for everything but numbers.
    pub order: &'static str,
    /// What else tells two rows of the same kind apart: a float's layout
    /// (`ieee754`, `bfloat16`, `x87`, `e4m3`, `e5m2`, `ibm`), a fixed-point
    /// split (`u16.16`), a text encoding (`utf8`, `utf16le`, ...), the
    /// instruction set of machine code, the radix of digits.
    pub detail: String,
    /// How many fields. For the template, how many declarations.
    pub fields: u64,
    /// How many bits of the file those fields cover. Always 0 for the
    /// template, which has no file.
    pub bits: u64,
    /// For a codec: how many of its streams were opened, and what they came
    /// to. A stream that has not been opened says nothing about its size.
    pub unpacked_fields: u64,
    pub unpacked_bits: u64,
}

/// One choice the format makes by a value, and how much of it the file uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceUse {
    /// The diagram's key for the switch, so the web can put this beside the
    /// switch's box. See [`crate::eval::diagram`].
    pub key: String,
    /// The field the choice is made for, as the template names it: `body`,
    /// `data`. The first one met, for a switch several fields share.
    pub name: String,
    /// How many shapes the choice can take, the default included.
    pub cases: u64,
    /// How many different ones this file took. 0 for the template.
    pub taken: u64,
    /// How many fields of this file made the choice. 0 for the template.
    pub fields: u64,
}

/// The numbers a sentence about reading and writing the format needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileFacts {
    /// Fields placed from the end of the file: an address worked out from the
    /// file's length, or from the last match of a signature.
    pub from_end: u64,
    /// Fields placed by an offset read from the file, rather than after the
    /// field before them: `At`, pointer lists, chains and gathers.
    pub placed: u64,
    /// Of those, how many sit at or after the field that holds the offset,
    /// and how many before it. Always 0 for the template, which cannot know.
    pub forward: u64,
    pub backward: u64,
    /// Fields that give the length or count of another field and come before
    /// it, and after it. The template counts declarations.
    pub lengths_before: u64,
    pub lengths_after: u64,
    /// True when every field follows from the bytes before it, so a reader
    /// can go from the start to the end without seeking back: nothing is
    /// placed from the end of the file and no offset points backwards. For the
    /// template, None when that depends on the file, which is whenever it
    /// places anything by an offset.
    pub every_field_follows: Option<bool>,
}

/// The whole profile, of the template or of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Sorted by category, then by the most fields.
    pub rows: Vec<ProfileRow>,
    pub choices: Vec<ChoiceUse>,
    pub facts: ProfileFacts,
    /// True once the whole file has been counted. Always true for the
    /// template.
    pub done: bool,
}

/// The key a row is added up under.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key {
    pub category: &'static str,
    pub kind: String,
    pub width: u32,
    pub order: &'static str,
    pub detail: String,
}

impl Key {
    pub(super) fn of(category: &'static str, kind: &str) -> Key {
        Key { category, kind: kind.to_string(), width: 0, order: "", detail: String::new() }
    }
    fn number(kind: &str, bits: u32, endian: Endian, detail: &str) -> Key {
        Key { category: "number", kind: kind.to_string(), width: bits, order: order_of(bits, endian), detail: detail.to_string() }
    }
}

/// What the rows come to so far.
#[derive(Debug, Clone, Default)]
pub(super) struct Tally {
    rows: BTreeMap<Key, ProfileCount>,
    /// Per switch key: the field it is for, how many cases it has, which of
    /// them were taken and by how many fields.
    choices: BTreeMap<String, (String, u64, BTreeSet<usize>, u64)>,
    pub facts: ProfileFacts,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ProfileCount {
    fields: u64,
    bits: u64,
    unpacked_fields: u64,
    unpacked_bits: u64,
}

impl Tally {
    pub(super) fn add(&mut self, key: Key, fields: u64, bits: u64) {
        let e = self.rows.entry(key).or_default();
        e.fields = e.fields.saturating_add(fields);
        e.bits = e.bits.saturating_add(bits);
    }

    pub(super) fn unpacked(&mut self, key: Key, fields: u64, bits: u64) {
        let e = self.rows.entry(key).or_default();
        e.unpacked_fields = e.unpacked_fields.saturating_add(fields);
        e.unpacked_bits = e.unpacked_bits.saturating_add(bits);
    }

    /// A switch the template declares, with its case count.
    pub(super) fn choice(&mut self, key: &str, name: &str, cases: u64) {
        self.choices.entry(key.to_string()).or_insert_with(|| (name.to_string(), cases, BTreeSet::new(), 0));
    }

    /// A field of the file that made the choice `key`, taking case `case`,
    /// standing for `n` fields.
    pub(super) fn chose(&mut self, key: &str, name: &str, cases: u64, case: Option<usize>, n: u64) {
        let e = self.choices.entry(key.to_string()).or_insert_with(|| (name.to_string(), cases, BTreeSet::new(), 0));
        if let Some(c) = case {
            e.2.insert(c);
        }
        e.3 = e.3.saturating_add(n);
    }

    pub(super) fn profile(&self, done: bool) -> Profile {
        let mut rows: Vec<ProfileRow> = self
            .rows
            .iter()
            .map(|(k, c)| ProfileRow {
                category: k.category,
                kind: k.kind.clone(),
                width: k.width,
                order: k.order,
                detail: k.detail.clone(),
                fields: c.fields,
                bits: c.bits,
                unpacked_fields: c.unpacked_fields,
                unpacked_bits: c.unpacked_bits,
            })
            .collect();
        // Category first, so a view drawing one section at a time finds them
        // together, then the commonest, then by what they are, so two runs
        // over one file answer in the same order.
        rows.sort_by(|a, b| {
            a.category
                .cmp(b.category)
                .then(b.fields.cmp(&a.fields))
                .then(b.bits.cmp(&a.bits))
                .then_with(|| (&a.kind, a.width, a.order, &a.detail).cmp(&(&b.kind, b.width, b.order, &b.detail)))
        });
        let choices = self
            .choices
            .iter()
            .map(|(key, (name, cases, taken, fields))| ChoiceUse {
                key: key.clone(),
                name: name.clone(),
                cases: *cases,
                taken: taken.len() as u64,
                fields: *fields,
            })
            .collect();
        Profile { rows, choices, facts: self.facts.clone(), done }
    }
}

/// `little` or `big` for a number wider than a byte, and `none` for one that
/// is a byte or less, which has no byte order.
fn order_of(bits: u32, e: Endian) -> &'static str {
    if bits <= 8 {
        return "none";
    }
    match e {
        Endian::Little => "little",
        Endian::Big => "big",
    }
}

/// Every row a value of type `ty` counts in. `bits` is how wide the field
/// turned out to be, for a field of the file, which settles the width of a
/// number the file gives the width of; None for a declaration.
///
/// The kinds, by category:
///
/// - `number`: `unsigned`, `signed`, `sign-magnitude`, `float`, `fixed-point`,
///   and `digits` for a number written out as text.
/// - `text`: how far it runs. `fixed`, `padded`, `terminated`, `token` (a run
///   between separators), `length-field` (as long as another field says), and
///   `to-end` (to the end of what it is in).
/// - `varint`: `leb128`, `sleb128`, `zigzag`, `vlq`, `ebml-id`, `ebml-size`,
///   `sqlite`, `7z`.
/// - `opaque`: `bytes`, `code`, `json`, `pickle`, `entropy-coded`.
/// - `codec`: the codec's name.
/// - `enum`, `flags`, `magic`, `padding` with no kind, and `computed` with
///   `number`, `text` or `real`.
///
/// A number narrower than a byte counts as a `bit-field` as well; so does one
/// that does not start on a byte, which only a field of the file can say, and
/// the caller adds that one.
pub(super) fn value_keys(ty: &Ty, bits: Option<u64>, out: &mut Vec<Key>) {
    let whole = |b: u32, out: &mut Vec<Key>| {
        if b % 8 != 0 {
            out.push(Key::of("bit-field", ""));
        }
    };
    match ty {
        Ty::UInt { bits: b, endian } => {
            out.push(Key::number("unsigned", *b, *endian, ""));
            whole(*b, out);
        }
        Ty::Int { bits: b, endian } => {
            out.push(Key::number("signed", *b, *endian, ""));
            whole(*b, out);
        }
        Ty::SignMagnitude { bits: b, endian } => {
            out.push(Key::number("sign-magnitude", *b, *endian, ""));
            whole(*b, out);
        }
        Ty::UIntExpr { endian, .. } => {
            let b = bits.map_or(0, |b| b.min(u64::from(u32::MAX)) as u32);
            out.push(Key::number("unsigned", b, *endian, ""));
            if bits.is_some() {
                whole(b, out);
            }
        }
        Ty::F16(e) => out.push(Key::number("float", 16, *e, "ieee754")),
        Ty::BF16(e) => out.push(Key::number("float", 16, *e, "bfloat16")),
        Ty::F32(e) => out.push(Key::number("float", 32, *e, "ieee754")),
        Ty::F64(e) => out.push(Key::number("float", 64, *e, "ieee754")),
        Ty::F80(e) => out.push(Key::number("float", 80, *e, "x87")),
        Ty::F8 { e4m3 } => out.push(Key::number("float", 8, Endian::Big, if *e4m3 { "e4m3" } else { "e5m2" })),
        Ty::IbmF32(e) => out.push(Key::number("float", 32, *e, "ibm")),
        Ty::Fixed { bits: b, frac, endian, signed } => {
            let split = format!("{}{}.{frac}", if *signed { "i" } else { "u" }, b.saturating_sub(*frac));
            out.push(Key::number("fixed-point", *b, *endian, &split));
            whole(*b, out);
        }
        Ty::TextInt { len, radix } => {
            out.push(Key { category: "number", kind: "digits".into(), width: 0, order: "none", detail: radix.to_string() });
            out.push(Key { category: "text", kind: text_rule(len).into(), width: 0, order: "", detail: "ascii".into() });
        }
        Ty::Enum { inner, .. } => {
            out.push(Key::of("enum", ""));
            value_keys(inner, bits, out);
        }
        Ty::Flags { inner, .. } => {
            out.push(Key::of("flags", ""));
            value_keys(inner, bits, out);
        }
        Ty::Nullable { inner, .. } => value_keys(inner, bits, out),
        Ty::Leb128 { signed: false } => out.push(Key::of("varint", "leb128")),
        Ty::Leb128 { signed: true } => out.push(Key::of("varint", "sleb128")),
        Ty::Zigzag => out.push(Key::of("varint", "zigzag")),
        Ty::Vlq => out.push(Key::of("varint", "vlq")),
        Ty::EbmlVint { strip_marker: false } => out.push(Key::of("varint", "ebml-id")),
        Ty::EbmlVint { strip_marker: true } => out.push(Key::of("varint", "ebml-size")),
        Ty::SqliteVarint => out.push(Key::of("varint", "sqlite")),
        Ty::SevenZipNumber => out.push(Key::of("varint", "7z")),
        Ty::Magic(_) => out.push(Key::of("magic", "")),
        Ty::Computed(_) => out.push(Key::of("computed", "number")),
        Ty::ComputedText(_) => out.push(Key::of("computed", "text")),
        Ty::ComputedReal(_) => out.push(Key::of("computed", "real")),
        Ty::Bytes(e) => match padding_align(e) {
            Some(align) => out.push(Key { category: "padding", kind: String::new(), width: align, order: "", detail: String::new() }),
            None => out.push(Key::of("opaque", "bytes")),
        },
        Ty::Str { len, enc } => {
            out.push(Key { category: "text", kind: text_rule(len).into(), width: 0, order: "", detail: encoding_name(enc).into() })
        }
        Ty::Insn { isa } => out.push(Key { category: "opaque", kind: "code".into(), width: 0, order: "", detail: isa.name().to_string() }),
        Ty::Json(..) => out.push(Key::of("opaque", "json")),
        Ty::Pickle(..) => out.push(Key::of("opaque", "pickle")),
        Ty::CodeBits { .. } => out.push(Key::of("opaque", "entropy-coded")),
        Ty::Decoded { codec, .. } => out.push(Key::of("codec", codec.as_str())),
        // What holds or places other fields, and what reads as nothing: none
        // of it is a value.
        Ty::Struct(_)
        | Ty::Array { .. }
        | Ty::Repeat { .. }
        | Ty::PointerList { .. }
        | Ty::Chain { .. }
        | Ty::Gather { .. }
        | Ty::At { .. }
        | Ty::Sized { .. }
        | Ty::SizedBits { .. }
        | Ty::Origin { .. }
        | Ty::Switch { .. }
        | Ty::Match { .. }
        | Ty::When { .. }
        | Ty::Named(_)
        | Ty::Stitched { .. }
        | Ty::Schema { .. }
        | Ty::Traced { .. } => {}
    }
}

/// The alignment a run of padding pads to, in bytes, when the run is sized as
/// padding. The same test `padding_doc` in `mod.rs` makes.
pub(super) fn padding_align(e: &Expr) -> Option<u32> {
    match e {
        Expr::PadTo { align, .. } => Some(*align),
        Expr::Min(a, _) => match &**a {
            Expr::PadTo { align, .. } => Some(*align),
            _ => None,
        },
        _ => None,
    }
}

/// How far a text field runs, as a `text` row's kind.
fn text_rule(len: &StrLen) -> &'static str {
    match len {
        StrLen::Fixed(e) => match expr_sizing(e) {
            Sizing::Remaining => "to-end",
            Sizing::Expression | Sizing::Encoded => "length-field",
            _ => "fixed",
        },
        StrLen::Padded { .. } => "padded",
        StrLen::Terminated { .. } => "terminated",
        StrLen::Scan { .. } => "token",
    }
}

/// A text encoding's name in a `text` row. Not `Encoding::short`, which is
/// written for the type column and is not a key.
fn encoding_name(enc: &crate::template::Encoding) -> &'static str {
    use crate::template::Encoding as E;
    match enc {
        E::Utf8 => "utf8",
        E::Ascii => "ascii",
        E::Latin1 => "latin1",
        E::Cp437 | E::Cp437Screen => "cp437",
        E::Utf16(Endian::Little) => "utf16le",
        E::Utf16(Endian::Big) => "utf16be",
        E::Bom { .. } => "bom",
        E::Unknown => "unknown",
        E::P8scii => "p8scii",
        E::Ebcdic => "ebcdic",
    }
}

/// How a field's length was settled, as a `sizing` row's kind, or nothing for
/// a field whose width is in its type's name or that covers no bytes.
///
/// `fixed` (a number the format fixes), `length-field` (worked out from other
/// fields), `count` (so many elements of a type), `terminator` (it ends where
/// a byte or an element says), `to-end` (it fills what is left), `fields` (as
/// long as its fields come to), `offsets` (a list of places, as far as the
/// room reaches), `self-delimiting` (its own bytes say where it ends: a
/// varint, an instruction), `decoder` (what a decoder read), `other`.
pub(super) fn sizing_kind(s: Sizing) -> Option<&'static str> {
    Some(match s {
        Sizing::Type | Sizing::Nothing => return None,
        Sizing::Fixed => "fixed",
        Sizing::Expression => "length-field",
        Sizing::Count => "count",
        Sizing::Terminated => "terminator",
        Sizing::Remaining => "to-end",
        Sizing::Children => "fields",
        Sizing::Scattered => "offsets",
        Sizing::Encoded => "self-delimiting",
        Sizing::Trace | Sizing::Table => "decoder",
        Sizing::Unknown => "other",
    })
}

/// Where an `At` counts its offset from, as a `placement` row's kind:
/// `at-start` (the file, or the start of this copy of the format or of the
/// stream it is read in), `at-parent` (the nearest window around it),
/// `at-end` (an address worked out from the end of the file), `at-other`.
pub(super) fn at_kind(anchor: crate::template::Anchor, at: &Expr) -> &'static str {
    use crate::template::Anchor;
    if from_end(at) {
        return "at-end";
    }
    match anchor {
        Anchor::File | Anchor::Space | Anchor::Origin => "at-start",
        Anchor::Window => "at-parent",
        Anchor::SelfAligned(_) => "at-other",
    }
}

/// Whether an expression places something from the end of the file: it reads
/// how long the space is, or finds the last match of a signature.
pub(super) fn from_end(e: &Expr) -> bool {
    match e {
        Expr::SpaceSize => true,
        Expr::Find { last, .. } => *last,
        Expr::Add(a, b)
        | Expr::Sub(a, b)
        | Expr::Mul(a, b)
        | Expr::Div(a, b)
        | Expr::Mod(a, b)
        | Expr::DivCeil(a, b)
        | Expr::Or(a, b)
        | Expr::Either(a, b)
        | Expr::Both(a, b)
        | Expr::And(a, b)
        | Expr::BitOr(a, b)
        | Expr::BitXor(a, b)
        | Expr::Less(a, b)
        | Expr::Eq(a, b)
        | Expr::Ne(a, b)
        | Expr::Le(a, b)
        | Expr::Gt(a, b)
        | Expr::Ge(a, b)
        | Expr::Shl(a, b)
        | Expr::Shr(a, b)
        | Expr::Min(a, b)
        | Expr::Max(a, b) => from_end(a) || from_end(b),
        Expr::Cond { when, then, otherwise } => from_end(when) || from_end(then) || from_end(otherwise),
        Expr::Not(a) | Expr::BitNot(a) | Expr::Log2(a) | Expr::Pow2(a) | Expr::Pow10(a) | Expr::Trunc(a) | Expr::Bit(a, _) => {
            from_end(a)
        }
        Expr::PadTo { n, .. } => from_end(n),
        Expr::PeekAt { skip, .. } => from_end(skip),
        Expr::PeekIn { at, .. } => from_end(at),
        Expr::StartOf(a) | Expr::Placer(a) | Expr::RealText(a) => from_end(a),
        _ => false,
    }
}

/// The profile of a template: every declaration it holds, counted once, with
/// each named type followed the first time it is met.
pub fn template_profile(t: &Template) -> Profile {
    let mut w = Declared { t, tally: Tally::default(), named: HashSet::new(), placing: 0 };
    w.ty(&t.root, None, false, 0);
    let mut types: Vec<&String> = t.types.keys().collect();
    // A type the root never reaches is still part of the format, and is
    // counted: a template that keeps its record shapes in the table and picks
    // them by name holds most of what it knows there.
    types.sort();
    for name in types {
        if w.named.insert(name.clone()) {
            let ty = t.types[name].clone();
            w.ty(&ty, None, false, 0);
        }
    }
    let placing = w.placing;
    let mut tally = w.tally;
    tally.facts.every_field_follows = if tally.facts.from_end > 0 {
        Some(false)
    } else if placing > 0 {
        None
    } else {
        Some(true)
    };
    tally.profile(true)
}

/// The walk over a template's declarations.
struct Declared<'a> {
    t: &'a Template,
    tally: Tally,
    /// The named types already walked.
    named: HashSet<String>,
    /// Declarations that place something by an offset read from the file.
    placing: u64,
}

impl Declared<'_> {
    /// Count one declared type. `place` is how its container places it, when
    /// that is worth a row; `sized` says a window around it has already
    /// counted how it is sized; `name` is the field it is declared as.
    fn ty(&mut self, ty: &Ty, place: Option<&'static str>, sized: bool, depth: u32) {
        self.ty_named(ty, place, sized, depth, "")
    }

    fn ty_named(&mut self, ty: &Ty, place: Option<&'static str>, sized: bool, depth: u32, name: &str) {
        if depth > 64 {
            return;
        }
        if let Some(p) = place {
            self.tally.add(Key::of("placement", p), 1, 0);
        }
        let mut keys = Vec::new();
        value_keys(ty, None, &mut keys);
        for k in keys {
            self.tally.add(k, 1, 0);
        }
        let own = |s: Sizing, tally: &mut Tally| {
            if !sized {
                if let Some(kind) = sizing_kind(s) {
                    tally.add(Key::of("sizing", kind), 1, 0);
                }
            }
        };
        let d = depth + 1;
        match ty {
            Ty::Named(n) => {
                if self.named.insert(n.to_string()) {
                    if let Some(t) = self.t.types.get(&**n) {
                        let t = t.clone();
                        // Placed where the name is used, which the row for the
                        // name above has already counted.
                        self.ty_named(&t, None, sized, d, name);
                    }
                }
            }
            Ty::Struct(s) => {
                own(Sizing::Children, &mut self.tally);
                let measures = machinery::measurers(s);
                for (i, f) in s.fields.iter().enumerate() {
                    if measures.get(i).copied().flatten().is_some() {
                        self.tally.facts.lengths_before += 1;
                    }
                    for c in &f.checks {
                        self.tally.add(Key::of("checksum", c.algorithm.as_str()), 1, 0);
                    }
                    if let Some(c) = &f.elem_check {
                        self.tally.add(Key::of("checksum", c.algorithm.as_str()), 1, 0);
                    }
                    let place = if s.overlap { "overlap" } else { "follows" };
                    // An address takes no room where it is declared, so it is
                    // not placed after anything: what it places is.
                    let place = if matches!(f.ty, Ty::At { .. }) { None } else { Some(place) };
                    self.ty_named(&f.ty, place, false, d, &f.name);
                }
            }
            Ty::Array { elem, .. } => {
                own(Sizing::Count, &mut self.tally);
                self.ty_named(elem, Some("element"), false, d, name);
            }
            Ty::Repeat { elem, until } => {
                let s = match until {
                    crate::template::Until::End => Sizing::Remaining,
                    _ => Sizing::Terminated,
                };
                own(s, &mut self.tally);
                self.ty_named(elem, Some("element"), false, d, name);
            }
            Ty::PointerList { elem, adjust, .. } => {
                own(Sizing::Scattered, &mut self.tally);
                self.placing += 1;
                self.tally.facts.placed += 1;
                if from_end(adjust) {
                    self.tally.facts.from_end += 1;
                }
                self.ty_named(elem, Some("pointer-list"), false, d, name);
            }
            Ty::Chain { elem, first, adjust, .. } => {
                self.placing += 1;
                self.tally.facts.placed += 1;
                if from_end(first) || from_end(adjust) {
                    self.tally.facts.from_end += 1;
                }
                self.ty_named(elem, Some("chain"), false, d, name);
            }
            Ty::Gather { elem, offset, adjust, .. } => {
                self.placing += 1;
                self.tally.facts.placed += 1;
                if from_end(offset) || from_end(adjust) {
                    self.tally.facts.from_end += 1;
                }
                self.ty_named(elem, Some("gather"), false, d, name);
            }
            Ty::At { anchor, at, inner } => {
                self.placing += 1;
                self.tally.facts.placed += 1;
                if from_end(at) {
                    self.tally.facts.from_end += 1;
                }
                self.ty_named(inner, Some(at_kind(*anchor, at)), false, d, name);
            }
            Ty::Sized { size, inner } => {
                own(expr_sizing(size), &mut self.tally);
                self.ty_named(inner, None, true, d, name);
            }
            Ty::SizedBits { bits, inner } => {
                own(expr_sizing(bits), &mut self.tally);
                self.ty_named(inner, None, true, d, name);
            }
            // The value rows above already looked inside an enum, a set of
            // flags and a sentinel, and a number is as wide as its type says;
            // only the wrappers that are not values are walked into.
            Ty::Origin { inner } | Ty::When { inner, .. } => self.ty_named(inner, None, sized, d, name),
            Ty::Switch { cases, default, .. } => {
                self.tally.choice(&super::diagram::box_key(self.t, ty).unwrap_or_default(), name, cases.len() as u64 + 1);
                for (_, c) in cases.iter() {
                    self.ty_named(c, None, sized, d, name);
                }
                self.ty_named(default, None, sized, d, name);
            }
            Ty::Match { cases, default, .. } => {
                self.tally.choice(&super::diagram::box_key(self.t, ty).unwrap_or_default(), name, cases.len() as u64 + 1);
                for (_, c) in cases.iter() {
                    self.ty_named(c, None, sized, d, name);
                }
                self.ty_named(default, None, sized, d, name);
            }
            Ty::Decoded { inner, .. } | Ty::Stitched { inner, .. } => {
                self.ty_named(inner, Some("stream"), false, d, name);
            }
            Ty::Bytes(e) => {
                if padding_align(e).is_some() {
                    if !sized {
                        self.tally.add(Key::of("sizing", "alignment"), 1, 0);
                    }
                } else {
                    own(expr_sizing(e), &mut self.tally);
                }
            }
            Ty::Str { len, .. } | Ty::TextInt { len, .. } => {
                let s = match len {
                    StrLen::Fixed(e) | StrLen::Padded { size: e, .. } => expr_sizing(e),
                    StrLen::Terminated { .. } | StrLen::Scan { .. } => Sizing::Terminated,
                };
                own(s, &mut self.tally);
            }
            Ty::UIntExpr { bits, .. } => own(expr_sizing(bits), &mut self.tally),
            Ty::Leb128 { .. } | Ty::Zigzag | Ty::Vlq | Ty::SqliteVarint | Ty::SevenZipNumber | Ty::EbmlVint { .. } | Ty::Insn { .. } | Ty::Json(..) | Ty::Pickle(..) => {
                own(Sizing::Encoded, &mut self.tally)
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{Anchor, Encoding, Expr as E, StrLen, Template, Ty as T, Until};

    fn row<'a>(p: &'a Profile, category: &str, kind: &str) -> Option<&'a ProfileRow> {
        p.rows.iter().find(|r| r.category == category && r.kind == kind)
    }

    #[test]
    fn a_template_counts_each_declaration_once() {
        let chunk = T::structure(
            "Chunk",
            vec![
                ("length", T::u32(Endian::Big)),
                ("kind", T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)),
                ("data", T::sized(E::field("length"), T::switch(E::field("kind"), vec![(1, T::u16(Endian::Big))], T::bytes(E::Remaining)))),
                ("crc", T::u32(Endian::Big)),
            ],
        );
        let root = T::structure(
            "File",
            vec![("magic", T::magic(b"PNG!")), ("chunks", T::repeat(T::Named("Chunk".into()), Until::End))],
        );
        let t = Template::new("test", root).with_type("Chunk", chunk);
        let p = template_profile(&t);
        // Two big-endian u32 and one u16 in a case.
        let n32 = p.rows.iter().find(|r| r.category == "number" && r.width == 32).expect("u32 row");
        assert_eq!((n32.fields, n32.order, n32.kind.as_str()), (2, "big", "unsigned"));
        assert_eq!(row(&p, "text", "fixed").map(|r| r.fields), Some(1));
        assert_eq!(row(&p, "sizing", "length-field").map(|r| r.fields), Some(1));
        assert_eq!(row(&p, "sizing", "to-end").map(|r| r.fields), Some(1), "the repeat; the default case is inside a window");
        assert_eq!(p.choices.len(), 1);
        assert_eq!(p.choices[0].cases, 2);
        assert_eq!(p.facts.lengths_before, 1, "length sizes data; kind only picks its shape");
        assert_eq!(p.facts.every_field_follows, Some(true));
        // Chunk is named once and walked once, however it is reached.
        assert_eq!(row(&p, "magic", "").map(|r| r.fields), Some(1));
    }

    #[test]
    fn an_address_from_the_end_makes_the_template_unstreamable() {
        let root = T::structure(
            "File",
            vec![
                ("head", T::u8()),
                ("tail", T::Named("Tail".into())),
            ],
        );
        let tail = T::At { anchor: Anchor::File, at: E::SpaceSize.sub(E::lit(4)), inner: Box::new(T::u32(Endian::Little)) };
        let t = Template::new("test", root).with_type("Tail", tail);
        let p = template_profile(&t);
        assert_eq!(p.facts.from_end, 1);
        assert_eq!(p.facts.every_field_follows, Some(false));
        assert_eq!(row(&p, "placement", "at-end").map(|r| r.fields), Some(1));
        let forward = T::structure("File", vec![("offset", T::u8()), ("there", T::at(E::field("offset"), T::u8()))]);
        let p = template_profile(&Template::new("test", forward));
        assert_eq!(p.facts.every_field_follows, None, "depends on where the offset points");
        assert_eq!(row(&p, "placement", "at-start").map(|r| r.fields), Some(1));
    }
}
