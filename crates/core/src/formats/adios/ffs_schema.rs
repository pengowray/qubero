//! The types BP5's FFS records are written in, built from the formats `mmd.0`
//! keeps.
//!
//! An FFS record is a C structure laid out as the writer's compiler laid it
//! out, and nothing in the record says how. The format it was written in says
//! it: a list of subformats, each a name, a record length, a byte order and a
//! pointer size, and fields, each a name, a type written as text, a size and
//! an offset. So a record's `data` is a [`Ty::Schema`](crate::template::Ty::Schema)
//! node keyed by the record's twelve-byte format ID, and [`Formats`] finds the
//! format with that ID among the descriptions and lays the structure out.
//!
//! **Types.** A type is a base and dimensions. The bases are FFS's own words,
//! `integer`, `unsigned integer`, `float`, `char` and `string`, or the name of
//! another subformat of the same format, which is that structure written in
//! place. A dimension is a number or the name of an earlier field of the same
//! structure. Any named dimension makes the field a pointer: its bytes are an
//! offset, counted from the start of the record, to the elements in the part of
//! the record after the fixed structure, and there are as many as the named
//! fields multiply to. A `string` is a pointer too, to text ending in a nul.
//! A pointer of nought points at nothing.
//!
//! **BP5.** The field names carry meaning. `BPG_8_10_temperature` is a global
//! array (`G`) of eight-byte values of ADIOS2 type 10, double, called
//! `temperature`, and its subformat says how many dimensions and blocks it has,
//! the counts and starts of each block, where each block's values are in the
//! data file and each block's minimum and maximum. So a field named that way
//! gains its name's parts, read from the name's own bytes in `mmd.0`, and a
//! list of its blocks, each with its counts and bounds typed as the variable
//! and, where the data file was opened with it, its values. An attribute's name
//! starts with a character that is its type, and its values are typed by it.

use std::sync::Arc;

use super::shared::shaped;
use crate::eval::{Descriptions, EvalError, R};
use crate::template::{Built, Endian, Endian::*, Expr as E, KeyPart, KeyValue, SchemaBuilder, Step, StrLen, Ty as T};

/// The schema kind of a record read on its own: the structure and nothing
/// placed outside the record.
pub(super) const FORMATS: &str = "ffs";

/// The kind of a record whose variables' blocks are placed in the data file
/// read with it, which is a field the template declares before the metadata:
/// [`DATA_AT`] and, around each step, [`DATA_OFFSET`].
pub(super) const FORMATS_PLACED: &str = "ffs placed";

/// The field the walk to the formats starts from: `mmd.0`'s reading, declared
/// before the metadata that is read with it.
pub(super) const MMD: &str = "mmd_0";

/// Where the data file starts, in bytes of the space the metadata is read in.
pub(super) const DATA_AT: &str = "data_0_at";

/// Where a step's data starts in the data file, which the index says.
pub(super) const DATA_OFFSET: &str = "data_offset";

/// How deep one subformat is written inside another before the build stops.
/// BP5 writes two levels; a format that names itself would not stop at all.
const DEPTH: usize = 8;

/// The walk from a record to the formats: every block of `mmd.0`.
pub(super) fn table() -> Vec<Step> {
    vec![Step::field(MMD), Step::each()]
}

/// A record's data, typed by the format its ID names. `kind` is [`FORMATS`] or
/// [`FORMATS_PLACED`]. Offsets inside count from the start of the data.
pub(super) fn record_data(kind: &str) -> T {
    T::origin(T::schema(kind, table(), vec![id_key()]))
}

/// The twelve bytes of the format ID beside the data, as one number.
fn id_key() -> KeyPart {
    let f = |name: &str| E::within(&["format_id", name]);
    let head = f("version").mul(E::lit(1 << 24)).add(f("rep_length_top").mul(E::lit(1 << 16))).add(f("rep_length"));
    KeyPart::Int(head.mul(E::lit(1i128 << 64)).add(f("hash1").mul(E::lit(1i128 << 32))).add(f("hash2")))
}

/// A format ID read out of `mmd.0` as the same number [`id_key`] makes.
fn id_number(table: &mut dyn Descriptions, id: &[usize]) -> R<Option<i128>> {
    let mut n = 0i128;
    for (name, bits) in [("version", 8), ("rep_length_top", 8), ("rep_length", 16), ("hash1", 32), ("hash2", 32)] {
        let Some(p) = table.find(id, &[name])? else { return Ok(None) };
        let Some(v) = table.int(&p)? else { return Ok(None) };
        n = (n << bits) | (v & ((1i128 << bits) - 1));
    }
    Ok(Some(n))
}

fn failed<X>(why: impl Into<String>) -> R<X> {
    Err(EvalError::Failed(why.into()))
}

/// The builder the BP5 templates register for [`FORMATS`] and
/// [`FORMATS_PLACED`].
#[derive(Debug, Default)]
pub(super) struct Formats {
    /// Whether a variable's blocks are placed in the data file.
    pub(super) placed: bool,
}

impl SchemaBuilder for Formats {
    fn build(&self, key: &[KeyValue], table: &mut dyn Descriptions) -> R<Built> {
        let Some(want) = key.first().and_then(KeyValue::as_int) else { return failed("an FFS schema key is a format ID") };
        let records = table.records()?;
        if records.is_empty() {
            return failed(NO_FORMATS);
        }
        for record in &records {
            let Some(id) = table.find(record, &["id"])? else { continue };
            if id_number(table, &id)? != Some(want) {
                continue;
            }
            let Some(list) = table.find(record, &["description", "subformats"])? else { return failed(unreadable(want)) };
            let formats = read_formats(table, &list)?;
            if formats.is_empty() {
                return failed(unreadable(want));
            }
            let builder = Builder { formats: &formats, placed: self.placed };
            let (ty, members_from) = builder.structure(&formats[0], None, 0)?;
            return Ok(Built { ty, from: Some(record.clone()), members_from });
        }
        failed(no_format(want))
    }

    fn key_text(&self, key: &[KeyValue]) -> String {
        match key.first().and_then(KeyValue::as_int) {
            Some(id) => format!("FFS format {}", hex_id(id)),
            None => String::new(),
        }
    }
}

fn hex_id(id: i128) -> String {
    format!("{:024x}", id & ((1i128 << 96) - 1))
}

/// Why a record stays bytes when nothing read with it is `mmd.0`.
const NO_FORMATS: &str = "mmd.0 not opened: it holds the format this record is written in";

/// Why a record stays bytes when `mmd.0` is there and has no format with its
/// ID. The whole ID, so it can be searched for.
fn no_format(id: i128) -> String {
    format!("no format {} in mmd.0", hex_id(id))
}

fn unreadable(id: i128) -> String {
    format!("format {} in mmd.0 did not read", hex_id(id))
}

/// What a block's location is, where the data file was not read with the
/// metadata and so nothing is placed there.
const NOT_PLACED: &str = "Where this block's values start in data.0. Not placed: data.0 is not among the opened files.";

/// One subformat as the description says it.
struct Format {
    name: String,
    endian: Endian,
    length: i128,
    pointer: i128,
    members: Vec<Member>,
}

/// One field of a subformat as the description says it.
struct Member {
    name: String,
    ty: String,
    size: i128,
    offset: i128,
    /// The field's description, which is what its row says it was laid out
    /// from.
    path: Vec<usize>,
    /// Where the name's own bytes are, in bits of the first space, when they
    /// are in it: what a BP5 name's parts are read from.
    name_at: Option<u64>,
}

fn int_at(table: &mut dyn Descriptions, from: &[usize], names: &[&str]) -> R<Option<i128>> {
    match table.find(from, names)? {
        Some(p) => table.int(&p),
        None => Ok(None),
    }
}

/// Every subformat of a format's description, the format itself first.
fn read_formats(table: &mut dyn Descriptions, list: &[usize]) -> R<Vec<Format>> {
    let mut out = Vec::new();
    for i in 0..table.count(list)? {
        let Some(sub) = table.find(list, &[&i.to_string()])? else { break };
        let endian = if int_at(table, &sub, &["byte_order"])? == Some(1) { Big } else { Little };
        let Some(body) = table.find(&sub, &["body"])? else { continue };
        let Some(name) = table.find(&body, &["name"])? else { continue };
        let name = table.text(&name)?;
        let length = int_at(table, &body, &["record_length"])?.unwrap_or(0);
        let pointer = int_at(table, &body, &["pointer_size"])?.unwrap_or(8);
        let mut members = Vec::new();
        if let Some(fields) = table.find(&body, &["fields"])? {
            for j in 0..table.count(&fields)? {
                let Some(f) = table.find(&fields, &[&j.to_string()])? else { break };
                let (Some(name_path), Some(type_path)) = (table.find(&f, &["name"])?, table.find(&f, &["type"])?) else { continue };
                let name = table.text(&name_path)?;
                let named = table.node(&name_path)?;
                let ty = table.text(&type_path)?;
                let size = int_at(table, &f, &["size"])?.unwrap_or(0);
                let offset = int_at(table, &f, &["offset"])?.unwrap_or(-1);
                let name_at = (named.space == 0).then_some(named.offset_bits);
                members.push(Member { name, ty, size, offset, path: f, name_at });
            }
        }
        out.push(Format { name, endian, length, pointer, members });
    }
    Ok(out)
}

/// A field's type written out: the base, and each dimension a number or the
/// name of the field that counts it.
struct Parsed<'a> {
    base: &'a str,
    fixed: i128,
    counted_by: Vec<&'a str>,
}

fn parse(ty: &str) -> Parsed<'_> {
    let ty = ty.trim().trim_start_matches('*').trim();
    let (base, mut rest) = match ty.find('[') {
        Some(i) => (ty[..i].trim(), &ty[i..]),
        None => (ty, ""),
    };
    let mut fixed = 1i128;
    let mut counted_by = Vec::new();
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']') else { break };
        let dim = rest[open + 1..open + close].trim();
        match dim.parse::<i128>() {
            Ok(n) => fixed = fixed.saturating_mul(n.max(0)),
            Err(_) => counted_by.push(dim),
        }
        rest = &rest[open + close + 1..];
    }
    Parsed { base, fixed, counted_by }
}

struct Builder<'a> {
    formats: &'a [Format],
    placed: bool,
}

impl Builder<'_> {
    /// A subformat laid out field by field at the offsets its description
    /// gives, with whatever is between them as padding, to its record length.
    /// `variable` is what a BP5 name said about the field holding it.
    fn structure(&self, format: &Format, variable: Option<&Variable>, depth: usize) -> R<(T, Vec<Option<Vec<usize>>>)> {
        if depth > DEPTH {
            return failed(format!("{} is written more than {DEPTH} subformats deep", format.name));
        }
        let mut members: Vec<&Member> = format.members.iter().collect();
        members.sort_by_key(|m| m.offset);
        let mut fields: Vec<(String, T)> = Vec::new();
        let mut from: Vec<Option<Vec<usize>>> = Vec::new();
        let mut aside: Vec<String> = Vec::new();
        let mut at = 0i128;
        let mut pads = 0;
        let mut pad = |fields: &mut Vec<(String, T)>, from: &mut Vec<Option<Vec<usize>>>, n: i128| {
            pads += 1;
            let name = if pads == 1 { "padding".to_string() } else { format!("padding_{pads}") };
            fields.push((name, T::bytes(E::lit(n))));
            from.push(None);
        };
        let mut declared: Vec<&str> = Vec::new();
        for m in members {
            if m.offset < at {
                continue;
            }
            if m.offset > at {
                pad(&mut fields, &mut from, m.offset - at);
            }
            let (ty, width) = self.member(format, m, &declared, depth)?;
            fields.push((m.name.clone(), ty));
            from.push(Some(m.path.clone()));
            declared.push(&m.name);
            at = m.offset + width;
        }
        if format.length > at {
            pad(&mut fields, &mut from, format.length - at);
        }
        if let Some(v) = variable {
            for (name, ty) in v.parts() {
                fields.push((name.to_string(), ty));
                from.push(None);
                aside.push(name.to_string());
            }
            if let Some(blocks) = self.blocks(v, &declared, format.endian) {
                fields.push(("blocks".to_string(), blocks));
                from.push(None);
            }
        }
        if let Some((extra, extra_aside)) = attribute(&format.name, &declared, format.endian) {
            for (name, ty) in extra {
                fields.push((name.to_string(), ty));
                from.push(None);
            }
            aside.extend(extra_aside.iter().map(|s| s.to_string()));
        }
        let mut ty = T::structure(&format.name, fields.iter().map(|(n, t)| (n.as_str(), t.clone())).collect());
        for name in &aside {
            ty = ty.field_aside(name);
        }
        Ok((ty, from))
    }

    fn subformat(&self, name: &str) -> Option<&Format> {
        self.formats.iter().skip(1).find(|f| f.name == name)
    }

    /// What one field is, and how many bytes of the structure it takes.
    fn member(&self, format: &Format, m: &Member, declared: &[&str], depth: usize) -> R<(T, i128)> {
        let e = format.endian;
        let parsed = parse(&m.ty);
        let size = m.size.max(0);
        let pointer = || T::UInt { bits: (format.pointer.clamp(1, 8) * 8) as u32, endian: e };
        let is_char = matches!(parsed.base, "char" | "unsigned char");
        if parsed.base == "string" && parsed.counted_by.is_empty() && parsed.fixed == 1 {
            return Ok((pointing(pointer(), T::cstr(), "text"), format.pointer));
        }
        // A subformat written in place, which is what a BP5 variable is: its
        // name says what the structure holds.
        let variable = Variable::parse(&m.name, m.name_at).filter(|v| v.name.is_some());
        let elem = match parsed.base {
            "string" => pointing(pointer(), T::cstr(), "text"),
            base => match self.subformat(base) {
                Some(sub) => {
                    let here = parsed.counted_by.is_empty() && parsed.fixed == 1;
                    self.structure(sub, variable.as_ref().filter(|_| here), depth + 1)?.0
                }
                None => scalar(base, size, e),
            },
        };
        if parsed.counted_by.is_empty() {
            let n = parsed.fixed.max(1);
            let width = if parsed.base == "string" { format.pointer * n } else { size * n };
            return Ok(match (n, is_char) {
                (1, _) => (elem, width),
                (_, true) => (T::bytes(E::lit(n)), width),
                _ => (T::array(elem, E::lit(n)), width),
            });
        }
        // A count this structure has not read by the time the pointer is read
        // cannot be asked, and the pointer is read as the offset it is. A
        // count called what the pointer's own fields are called would be
        // found before the real one, and is left the same way.
        let contents = if is_char { "bytes" } else { "values" };
        let askable = parsed.counted_by.iter().all(|c| declared.contains(c) && *c != "offset" && *c != contents);
        if !askable {
            return Ok((pointer(), format.pointer));
        }
        let count = parsed.counted_by.iter().map(|c| E::field(c)).reduce(|a, b| a.mul(b)).expect("a count");
        let elements = match (is_char, parsed.fixed) {
            (true, 1) => T::bytes(count),
            (true, n) => T::array(T::bytes(E::lit(n)), count),
            (false, 1) => T::array(elem, count),
            (false, n) => T::array(T::array(elem, E::lit(n)), count),
        };
        Ok((pointing(pointer(), elements, contents), format.pointer))
    }

    /// The blocks of a BP5 array variable, from the subformat's own fields:
    /// each block's counts and starts, where its values are, its bounds typed
    /// as the variable, and its values where the data file is read with this.
    fn blocks(&self, v: &Variable, members: &[&str], e: Endian) -> Option<T> {
        let has = |name: &str| members.contains(&name);
        if !["Dims", "BlockCount", "Count", "DataBlockLocation"].iter().all(|n| has(n)) {
            return None;
        }
        let size = v.size?;
        let vt = value_type(v.data_type?, size, e);
        let index = || E::field("index");
        let dims = || E::field("Dims");
        let nth = |list: &str, i: E| E::elem_within(&[list, "values"], i, &[]);
        let pointer = |list: &str| E::within(&[list, "offset"]);
        let each_dim = |list: &str| T::array(T::computed(nth(list, index().mul(dims()).add(E::idx()))), dims());
        let mut fields = vec![("index", T::computed(E::idx())), ("count", each_dim("Count"))];
        // A local array has no place in a global shape, and writes nought for
        // the pointer to where its blocks start.
        if has("Offset") {
            fields.push(("start", T::when(pointer("Offset").not_equal(E::lit(0)), each_dim("Offset"))));
        }
        fields.extend([
            ("location", T::computed(nth("DataBlockLocation", index()))),
            ("element_count", T::computed(E::product_of("count"))),
            ("row_length", T::computed(E::cond(E::lit(0).less_than(dims()), E::elem("count", dims().sub(E::lit(1))), E::lit(1)))),
        ]);
        if has("MinMax") {
            let bound = |k: i128| {
                T::when(
                    pointer("MinMax").not_equal(E::lit(0)),
                    T::at_origin(pointer("MinMax").add(index().mul(E::lit(2 * size))).add(E::lit(k * size)), vt.clone()),
                )
            };
            fields.push(("min", bound(0)));
            fields.push(("max", bound(1)));
        }
        if self.placed {
            let compressed = has("DataBlockSize");
            let len = if compressed { nth("DataBlockSize", index()) } else { E::field("element_count").mul(E::lit(size)) };
            let values = if compressed { T::bytes(E::Remaining) } else { shaped(vt, E::field("element_count"), E::field("row_length")) };
            let at = E::field(DATA_AT).add(E::field(DATA_OFFSET)).add(E::field("location"));
            fields.push(("values", T::at(at, T::sized(len, values))));
        }
        let mut block = T::structure_named("Bp5Block", "", "", fields)
            .machinery(&["index", "element_count", "row_length"])
            .field_aside("min")
            .field_aside("max");
        if !self.placed {
            block = block.field_doc("location", NOT_PLACED);
        }
        Some(T::array(block, E::field("BlockCount")))
    }
}

/// A pointer: its offset, and what it points at when the offset is not nought.
fn pointing(pointer: T, target: T, contents: &str) -> T {
    T::structure_named(
        "FfsPointer",
        "",
        contents,
        vec![("offset", pointer), (contents, T::when(E::field("offset").not_equal(E::lit(0)), T::at_origin(E::field("offset"), target)))],
    )
}

/// One value of an FFS base type `size` bytes wide.
fn scalar(base: &str, size: i128, e: Endian) -> T {
    let bits = (size * 8) as u32;
    let int_width = matches!(size, 1 | 2 | 4 | 8);
    match base {
        "integer" | "signed integer" | "enumeration" if int_width => T::Int { bits, endian: e },
        "unsigned integer" | "unsigned" | "boolean" | "char" | "unsigned char" if int_width => T::UInt { bits, endian: e },
        "float" | "double" if size == 4 => T::F32(e),
        "float" | "double" if size == 8 => T::F64(e),
        _ => T::bytes(E::lit(size.max(0))),
    }
}

/// `adios2::DataType`, which is what a BP5 name's type number and an
/// attribute's type character count. Not BP3 and BP4's `DataTypes`, which
/// numbers the same types differently.
const ADIOS2_TYPES: &[(i128, &str)] = &[
    (0, "none"),
    (1, "int8"),
    (2, "int16"),
    (3, "int32"),
    (4, "int64"),
    (5, "uint8"),
    (6, "uint16"),
    (7, "uint32"),
    (8, "uint64"),
    (9, "float"),
    (10, "double"),
    (11, "long double"),
    (12, "complex float"),
    (13, "complex double"),
    (14, "string"),
    (15, "char"),
    (16, "struct"),
];

/// How a BP5 variable is shaped, by the letter after `BP` in its name.
const SHAPES: &[(i128, &str)] = &[
    (b'g' as i128, "global value"),
    (b'G' as i128, "global array"),
    (b'J' as i128, "joined array"),
    (b'l' as i128, "local value"),
    (b'L' as i128, "local array"),
    (b'U' as i128, "unknown"),
];

/// One value of ADIOS2 type `data_type`, `size` bytes wide, or the bytes where
/// the width is not the type's.
fn value_type(data_type: i128, size: i128, e: Endian) -> T {
    let complex = |name: &str, part: T| T::inline_structure(name, vec![("real", part.clone()), ("imaginary", part)]);
    let t = match (data_type, size) {
        (1 | 15, 1) => T::Int { bits: 8, endian: e },
        (2, 2) => T::Int { bits: 16, endian: e },
        (3, 4) => T::Int { bits: 32, endian: e },
        (4, 8) => T::Int { bits: 64, endian: e },
        (5, 1) => T::u8(),
        (6, 2) => T::u16(e),
        (7, 4) => T::u32(e),
        (8, 8) => T::u64(e),
        (9, 4) => T::F32(e),
        (10, 8) => T::F64(e),
        (12, 8) => complex("ComplexFloat", T::F32(e)),
        (13, 16) => complex("ComplexDouble", T::F64(e)),
        _ => T::bytes(E::lit(size.max(0))),
    };
    t
}

/// What a BP5 variable's name says about it: `BPG_8_10_temperature`, or with a
/// derived variable's expression in base64 between dashes,
/// `BPG-24-ZXhwcg==-8_10_temperature`. A global value's name is only the
/// letter and the name, `BPg_step_count`.
struct Variable {
    /// Where the name's bytes are, in bytes of the first space.
    at: Option<i128>,
    shape: usize,
    expression: Option<(usize, usize)>,
    size_digits: Option<(usize, usize)>,
    type_digits: Option<(usize, usize)>,
    size: Option<i128>,
    data_type: Option<i128>,
    name: Option<(usize, usize)>,
}

impl Variable {
    fn parse(name: &str, at: Option<u64>) -> Option<Variable> {
        let b = name.as_bytes();
        if b.len() < 4 || &b[..2] != b"BP" || !SHAPES.iter().any(|(c, _)| *c == b[2] as i128) {
            return None;
        }
        let digits = |from: usize| -> Option<(usize, usize)> {
            let n = b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
            (n > 0).then_some((from, n))
        };
        let mut v = Variable { at: at.map(|bits| (bits / 8) as i128), shape: 2, expression: None, size_digits: None, type_digits: None, size: None, data_type: None, name: None };
        let mut i = 3;
        if b[i] == b'-' {
            let (from, n) = digits(i + 1)?;
            let len: usize = name[from..from + n].parse().ok()?;
            let start = from + n + 1;
            if b.get(from + n) != Some(&b'-') || b.get(start + len) != Some(&b'-') {
                return None;
            }
            v.expression = Some((start, len));
            i = start + len;
        } else if b[i] != b'_' {
            return None;
        }
        i += 1;
        // A size and a type, or straight to the name.
        if let Some((from, n)) = digits(i) {
            if b.get(from + n) == Some(&b'_') {
                if let Some((tfrom, tn)) = digits(from + n + 1) {
                    if b.get(tfrom + tn) == Some(&b'_') {
                        v.size_digits = Some((from, n));
                        v.type_digits = Some((tfrom, tn));
                        v.size = name[from..from + n].parse().ok();
                        v.data_type = name[tfrom..tfrom + tn].parse().ok();
                        i = tfrom + tn + 1;
                    }
                }
            }
        }
        if i < b.len() {
            v.name = Some((i, b.len() - i));
        }
        Some(v)
    }

    /// The name's parts as fields reading the name's own bytes.
    fn parts(&self) -> Vec<(&'static str, T)> {
        let Some(at) = self.at else { return Vec::new() };
        let place = |from: usize, ty: T| T::at(E::lit(at + from as i128), ty);
        let mut out = vec![("shape", place(self.shape, T::enumeration("Bp5Shape", T::u8(), SHAPES)))];
        if let Some((from, n)) = self.expression {
            out.push(("expression", place(from, T::utf8(E::lit(n as i128)))));
        }
        if let Some((from, n)) = self.size_digits {
            out.push(("element_size", place(from, T::decimal(StrLen::Fixed(E::lit(n as i128))))));
        }
        // The digits, and what they name. An enumeration reads a number out of
        // bytes, not out of digits, so it reads the digits' field.
        if let Some((from, n)) = self.type_digits {
            out.push(("type_code", place(from, T::decimal(StrLen::Fixed(E::lit(n as i128))))));
            out.push(("data_type", T::enumeration("Adios2Type", T::computed(E::within(&["type_code"])), ADIOS2_TYPES)));
        }
        if let Some((from, n)) = self.name {
            out.push(("variable", place(from, T::utf8(E::lit(n as i128)))));
        }
        out
    }
}

/// What an attribute's type character says: `'0'` and the ADIOS2 type, and 18
/// more for an array.
fn attribute_types() -> Vec<(i128, String)> {
    let mut out = Vec::new();
    for (n, name) in ADIOS2_TYPES {
        out.push((b'0' as i128 + n, name.to_string()));
        out.push((b'0' as i128 + 18 + n, format!("{name} array")));
    }
    out
}

/// The fields a BP5 attribute record gains: the type its name starts with, the
/// name after it, and for a number its values typed by it.
fn attribute(format: &str, declared: &[&str], e: Endian) -> Option<(Vec<(&'static str, T)>, Vec<&'static str>)> {
    let prim = format == "PrimAttr" && ["name", "TotalElementSize", "Values"].iter().all(|n| declared.contains(n));
    let strs = format == "StrAttr" && ["name", "ElementCount", "Values"].iter().all(|n| declared.contains(n));
    if !prim && !strs {
        return None;
    }
    let types = attribute_types();
    let cases: Vec<(i128, &str)> = types.iter().map(|(k, v)| (*k, v.as_str())).collect();
    // BP5 always names an attribute, so the pointer is not asked whether it
    // points anywhere.
    let name_at = || E::within(&["name", "offset"]);
    let mut fields = vec![
        ("type", T::at_origin(name_at(), T::enumeration("Bp5AttributeType", T::u8(), &cases))),
        ("attribute", T::at_origin(name_at().add(E::lit(1)), T::cstr())),
    ];
    if prim {
        let values_at = || E::within(&["Values", "offset"]);
        let mut by_type = Vec::new();
        for (n, _) in ADIOS2_TYPES {
            let width = match n {
                1 | 5 | 15 => 1,
                2 | 6 => 2,
                3 | 7 | 9 => 4,
                4 | 8 | 10 | 12 => 8,
                13 => 16,
                _ => continue,
            };
            let elem = value_type(*n, width, e);
            let count = E::field("TotalElementSize").div(E::lit(width));
            let array = T::at_origin(values_at(), T::array(elem.clone(), count));
            by_type.push((b'0' as i128 + 18 + n, array));
            by_type.push((b'0' as i128 + n, T::at_origin(values_at(), elem)));
        }
        fields.push(("values", T::when(values_at().not_equal(E::lit(0)), T::switch(E::within(&["type"]), by_type, T::bytes(E::lit(0))))));
    }
    let aside = if prim { vec!["type", "attribute", "values"] } else { vec!["type", "attribute"] };
    Some((fields, aside))
}

/// The two kinds, registered on a template that reads BP5 metadata.
pub(super) fn register(t: crate::template::Template) -> crate::template::Template {
    t.with_schema(FORMATS, Arc::new(Formats { placed: false })).with_schema(FORMATS_PLACED, Arc::new(Formats { placed: true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(name: &str, at: Option<(usize, usize)>) -> Option<&str> {
        at.map(|(from, n)| &name[from..from + n])
    }

    /// A BP5 name's parts: the shape letter, a derived variable's expression,
    /// the element size and type, and the name, which may itself hold
    /// underscores.
    #[test]
    fn a_bp5_name_splits_into_the_parts_that_describe_the_variable() {
        let name = "BPG_8_10_temperature";
        let v = Variable::parse(name, Some(80)).unwrap();
        assert_eq!((v.size, v.data_type, part(name, v.name), v.at), (Some(8), Some(10), Some("temperature"), Some(10)));
        let name = "BPL_4_9_sea_surface";
        let v = Variable::parse(name, None).unwrap();
        assert_eq!((v.size, v.data_type, part(name, v.name)), (Some(4), Some(9), Some("sea_surface")));
        let name = "BPg_step_count";
        let v = Variable::parse(name, None).unwrap();
        assert_eq!((v.size, v.data_type, part(name, v.name)), (None, None, Some("step_count")));
        let name = "BPG-12-ZXhwcj0xLjA=-8_10_derived";
        let v = Variable::parse(name, None).unwrap();
        assert_eq!((part(name, v.expression), v.data_type, part(name, v.name)), (Some("ZXhwcj0xLjA="), Some(10), Some("derived")));
        assert!(Variable::parse("BitFieldCount", None).is_none());
        assert!(Variable::parse("BPX_8_10_x", None).is_none());
    }

    #[test]
    fn a_field_type_is_a_base_and_its_dimensions() {
        let p = parse("char[16][BlockCount]");
        assert_eq!((p.base, p.fixed, p.counted_by), ("char", 16, vec!["BlockCount"]));
        let p = parse("unsigned integer");
        assert_eq!((p.base, p.fixed, p.counted_by.len()), ("unsigned integer", 1, 0));
        let p = parse("*MetaArrayMM8[DBCount][Dims]");
        assert_eq!((p.base, p.counted_by), ("MetaArrayMM8", vec!["DBCount", "Dims"]));
    }
}
