//! What an Avro schema says about the bytes of a record.
//!
//! An Avro record is written with nothing in it but values: no field names, no
//! types, no lengths except those of strings, bytes and blocks. What is where
//! comes from the schema, a JSON document that an object container file carries
//! in its header and that a single framed record names only by number. So the
//! schema is parsed here and each type in it becomes a [`Ty`], the way an ASDF
//! tree types that file's blocks.
//!
//! Every record, enum and fixed type is named, and a named type can be used
//! again by name, including inside itself. So a named type is never written
//! out in place: wherever it is used, definition included, it is a
//! [`Ty::Schema`] node keyed by its full name, built when a node of it is first
//! read. That makes a recursive type no harder than any other.
//!
//! Rubin's alerts name their schema by number, and the schema is split across
//! files that name each other's types. Those files are bundled (see
//! `avro_alerts.rs`), and a name a file does not define is looked for in the
//! file of that name beside it, which is how `fastavro.schema.load_schema`
//! reads them for `lsst/alert_packet`.

use std::sync::{Arc, Mutex};

use rustc_hash::{FxHashMap, FxHashSet};

use crate::codec::Codec;
use crate::eval::{Descriptions, EvalError, R};
use crate::json::{self, Kind, Val};
use crate::template::{Built, Check, Checksum, Covers, Encoding, Endian, Expr as E, KeyPart, KeyValue, Named, SchemaBuilder, Step, StrLen, Time, Ty as T, Until, Valid};

use super::avro_alerts::SCHEMAS;

/// The schema kind of a type named in a container file's own schema.
pub(super) const KIND: &str = "avro";
/// The schema kind of a type named in one of Rubin's bundled alert schemas.
pub(super) const KIND_ALERT: &str = "avro alert";
/// The schema kind of a container file's block of records, which is read
/// through whichever codec the header names.
pub(super) const KIND_BLOCK: &str = "avro block";

/// The walk from anywhere in a container file to the entries of its header's
/// metadata map, where the schema and the codec are.
pub(super) fn meta_entries() -> Vec<Step> {
    vec![Step::field("header"), Step::field("meta"), Step::each(), Step::field("items"), Step::each()]
}

/// A type as the schema writes it, with every named type reduced to its name.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Avro {
    Null,
    Boolean,
    Int,
    /// A long, and the moment it counts to when its logical type is a time.
    Long(Option<Stamp>),
    Float,
    Double,
    Bytes,
    String,
    Array(Box<Avro>),
    Map(Box<Avro>),
    Union(Vec<Avro>),
    /// A record, an enum or a fixed, by its full name.
    Named(Arc<str>),
}

/// A long's logical type, where it is a count from the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Stamp {
    Millis,
    Micros,
    Nanos,
}

impl Stamp {
    fn time(self) -> Time {
        match self {
            Stamp::Millis => Time::unix_millis(),
            Stamp::Micros => Time::unix_micros(),
            Stamp::Nanos => Time::unix_nanos(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Field {
    pub(super) name: String,
    pub(super) ty: Avro,
    pub(super) doc: Option<String>,
}

/// What a name stands for.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Def {
    Record { doc: Option<String>, fields: Vec<Field> },
    Enum { doc: Option<String>, symbols: Vec<String> },
    Fixed { size: u64 },
}

/// A whole schema: the type it is, and what each name in it stands for.
#[derive(Debug, Clone, Default)]
pub(super) struct Schema {
    pub(super) top: Option<Avro>,
    pub(super) defs: FxHashMap<Arc<str>, Def>,
    /// Every name used, whether or not it was defined.
    used: FxHashSet<Arc<str>>,
}

impl Schema {
    /// Parse one schema document into this one: its names are added to the
    /// ones already here. The first document parsed is the schema's type.
    pub(super) fn add(&mut self, text: &[u8]) -> Result<(), String> {
        let val = json::parse(text)?;
        let ty = self.parse(&val, "")?;
        self.top.get_or_insert(ty);
        Ok(())
    }

    /// Names used and not defined.
    pub(super) fn missing(&self) -> Vec<Arc<str>> {
        let mut out: Vec<_> = self.used.iter().filter(|n| self.def(n).is_none()).cloned().collect();
        out.sort();
        out
    }

    /// What a name stands for. A short name inside a namespace is looked for
    /// in that namespace first, as the specification says; a writer that meant
    /// the null namespace is found by the name alone.
    pub(super) fn def(&self, name: &str) -> Option<&Def> {
        self.defs.get(name).or_else(|| self.defs.get(name.rsplit('.').next()?))
    }

    fn parse(&mut self, val: &Val, ns: &str) -> Result<Avro, String> {
        match &val.kind {
            Kind::Text(name) => Ok(self.by_name(name, ns)),
            Kind::Array(branches) => Ok(Avro::Union(branches.iter().map(|b| self.parse(b, ns)).collect::<Result<_, _>>()?)),
            Kind::Object(members) => self.parse_object(members, ns),
            _ => Err(format!("a type is a name, a list or an object, not {}", &val_text(val))),
        }
    }

    fn by_name(&mut self, name: &str, ns: &str) -> Avro {
        match name {
            "null" => Avro::Null,
            "boolean" => Avro::Boolean,
            "int" => Avro::Int,
            "long" => Avro::Long(None),
            "float" => Avro::Float,
            "double" => Avro::Double,
            "bytes" => Avro::Bytes,
            "string" => Avro::String,
            _ => {
                let full: Arc<str> = full_name(name, None, ns).into();
                self.used.insert(full.clone());
                Avro::Named(full)
            }
        }
    }

    fn parse_object(&mut self, members: &[(String, Val)], ns: &str) -> Result<Avro, String> {
        let get = |key: &str| members.iter().find(|(k, _)| k == key).map(|(_, v)| v);
        let text = |key: &str| get(key).and_then(|v| if let Kind::Text(s) = &v.kind { Some(s.as_str()) } else { None });
        let doc = text("doc").map(str::to_string);
        let Some(ty) = get("type") else { return Err("a type object has no \"type\"".into()) };
        let Kind::Text(kind) = &ty.kind else {
            // `{"type": {...}}` and `{"type": [...]}` are the type inside.
            return self.parse(ty, ns);
        };
        let named = |this: &mut Schema| -> Result<(Arc<str>, String), String> {
            let Some(name) = text("name") else { return Err(format!("a {kind} has no name")) };
            let full = full_name(name, text("namespace"), ns);
            let inner = full.rsplit_once('.').map_or("", |(n, _)| n).to_string();
            let full: Arc<str> = full.into();
            this.used.insert(full.clone());
            Ok((full, inner))
        };
        match kind.as_str() {
            "record" | "error" => {
                let (full, inner) = named(self)?;
                // Defined before its fields are read, so a field may name it.
                self.defs.insert(full.clone(), Def::Record { doc: doc.clone(), fields: Vec::new() });
                let Some(Kind::Array(list)) = get("fields").map(|v| &v.kind) else { return Err(format!("record {full} has no fields")) };
                let mut fields = Vec::new();
                for f in list {
                    let Kind::Object(fm) = &f.kind else { return Err(format!("a field of {full} is not an object")) };
                    let fget = |key: &str| fm.iter().find(|(k, _)| k == key).map(|(_, v)| v);
                    let ftext = |key: &str| fget(key).and_then(|v| if let Kind::Text(s) = &v.kind { Some(s.clone()) } else { None });
                    let Some(name) = ftext("name") else { return Err(format!("a field of {full} has no name")) };
                    let Some(ty) = fget("type") else { return Err(format!("field {name} of {full} has no type")) };
                    fields.push(Field { name, ty: self.parse(ty, &inner)?, doc: ftext("doc") });
                }
                self.defs.insert(full.clone(), Def::Record { doc, fields });
                Ok(Avro::Named(full))
            }
            "enum" => {
                let (full, _) = named(self)?;
                let Some(Kind::Array(list)) = get("symbols").map(|v| &v.kind) else { return Err(format!("enum {full} has no symbols")) };
                let symbols = list.iter().map(|s| if let Kind::Text(s) = &s.kind { s.clone() } else { val_text(s) }).collect();
                self.defs.insert(full.clone(), Def::Enum { doc, symbols });
                Ok(Avro::Named(full))
            }
            "fixed" => {
                let (full, _) = named(self)?;
                let Some(Kind::Int(size)) = get("size").map(|v| &v.kind) else { return Err(format!("fixed {full} has no size")) };
                let size = u64::try_from(*size).map_err(|_| format!("fixed {full} has a size of {size}"))?;
                self.defs.insert(full.clone(), Def::Fixed { size });
                Ok(Avro::Named(full))
            }
            "array" => Ok(Avro::Array(Box::new(self.parse(get("items").ok_or("an array has no \"items\"")?, ns)?))),
            "map" => Ok(Avro::Map(Box::new(self.parse(get("values").ok_or("a map has no \"values\"")?, ns)?))),
            "long" => Ok(Avro::Long(match text("logicalType") {
                Some("timestamp-millis" | "local-timestamp-millis") => Some(Stamp::Millis),
                Some("timestamp-micros" | "local-timestamp-micros") => Some(Stamp::Micros),
                Some("timestamp-nanos" | "local-timestamp-nanos") => Some(Stamp::Nanos),
                _ => None,
            })),
            // Any other logical type reads as the type underneath it, which is
            // what the specification says a reader that does not know it does.
            name => Ok(self.by_name(name, ns)),
        }
    }
}

/// A name made full: a name with a dot in it already is, and one without
/// takes the namespace given beside it, or else the one it is written in.
fn full_name(name: &str, namespace: Option<&str>, ns: &str) -> String {
    if name.contains('.') {
        return name.to_string();
    }
    match namespace.unwrap_or(ns) {
        "" => name.to_string(),
        n => format!("{n}.{name}"),
    }
}

fn val_text(val: &Val) -> String {
    match &val.kind {
        Kind::Text(s) => s.clone(),
        Kind::Int(n) => n.to_string(),
        Kind::Float(f) => f.to_string(),
        Kind::Bool(b) => b.to_string(),
        Kind::Null => "null".into(),
        Kind::Object(_) => "an object".into(),
        Kind::Array(_) => "a list".into(),
    }
}

/// The last part of a full name, which is what a reader calls the type.
fn short(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// What a type is called in a list of a union's branches.
fn label(a: &Avro) -> String {
    match a {
        Avro::Null => "null".into(),
        Avro::Boolean => "boolean".into(),
        Avro::Int => "int".into(),
        Avro::Long(_) => "long".into(),
        Avro::Float => "float".into(),
        Avro::Double => "double".into(),
        Avro::Bytes => "bytes".into(),
        Avro::String => "string".into(),
        Avro::Array(item) => format!("array of {}", label(item)),
        Avro::Map(value) => format!("map of {}", label(value)),
        Avro::Union(b) => b.iter().map(label).collect::<Vec<_>>().join(" or "),
        Avro::Named(n) => short(n).to_string(),
    }
}

/// How a named type is reached from wherever it is used: the schema kind, the
/// walk to the descriptions, and the key parts in front of its name.
#[derive(Debug, Clone)]
pub(super) struct Reach {
    kind: &'static str,
    table: Vec<Step>,
    key: Vec<KeyPart>,
}

impl Reach {
    /// A type of a container file's own schema.
    pub(super) fn file() -> Reach {
        Reach { kind: KIND, table: meta_entries(), key: Vec::new() }
    }

    /// A type of the bundled alert schema with this ID.
    pub(super) fn alert(id: E) -> Reach {
        Reach { kind: KIND_ALERT, table: Vec::new(), key: vec![KeyPart::Int(id)] }
    }

    /// The node for the type named `name`, and for the schema's own type when
    /// `name` is empty.
    pub(super) fn named(&self, name: &str) -> T {
        let mut key = self.key.clone();
        key.push(KeyPart::TextLit(name.into()));
        T::schema(self.kind, self.table.clone(), key)
    }

    /// What a value of `a` is read as.
    pub(super) fn ty(&self, a: &Avro) -> T {
        match a {
            // Nothing is written for a null, so it is a field that is not
            // there.
            Avro::Null => T::when(E::lit(0), T::bytes(E::lit(0))),
            Avro::Boolean => T::enumeration("boolean", T::u8(), &[(0, "false"), (1, "true")]),
            Avro::Int | Avro::Long(_) => T::zigzag(),
            Avro::Float => T::F32(Endian::Little),
            Avro::Double => T::F64(Endian::Little),
            Avro::Bytes => bytes(),
            Avro::String => T::structure_named(
                "string",
                "",
                "text",
                vec![("length", T::zigzag()), ("text", T::text(StrLen::Fixed(E::field("length")), Encoding::Utf8))],
            ),
            Avro::Array(item) => blocks(self.ty(item)),
            Avro::Map(value) => {
                let entry = T::structure("entry", vec![("key", self.ty(&Avro::String)), ("value", self.ty(value))]).named_by("key");
                blocks(entry)
            }
            Avro::Union(branches) => self.union(branches),
            Avro::Named(name) => self.named(name),
        }
    }

    /// A branch number, then the value of that branch.
    fn union(&self, branches: &[Avro]) -> T {
        let names: Vec<String> = branches.iter().map(label).collect();
        let cases: Vec<(i128, &str)> = names.iter().enumerate().map(|(i, n)| (i as i128, n.as_str())).collect();
        let values = branches.iter().enumerate().map(|(i, b)| (i as i128, self.ty(b))).collect();
        let mut ty = T::structure_named(
            &names.join(" or "),
            "",
            "value",
            vec![("branch", T::enumeration("branch", T::zigzag(), &cases)), ("value", T::switch(E::field("branch"), values, T::bytes(E::lit(0))))],
        )
        .field_valid("branch", Valid::InEnum);
        if let Some(stamp) = branches.iter().find_map(|b| if let Avro::Long(Some(s)) = b { Some(*s) } else { None }) {
            ty = ty.field_time("value", stamp.time());
        }
        ty
    }

    /// The type a name stands for, written out.
    fn def(&self, name: &str, def: &Def) -> T {
        match def {
            Def::Record { doc, fields } => {
                let mut ty = T::structure(short(name), fields.iter().map(|f| (f.name.as_str(), self.ty(&f.ty))).collect());
                for f in fields {
                    if let Some(d) = &f.doc {
                        ty = ty.field_doc(&f.name, d);
                    }
                    if let Avro::Long(Some(stamp)) = f.ty {
                        ty = ty.field_time(&f.name, stamp.time());
                    }
                }
                match doc {
                    Some(d) => ty.doc(d),
                    None => ty,
                }
            }
            Def::Enum { symbols, .. } => {
                let cases: Vec<(i128, &str)> = symbols.iter().enumerate().map(|(i, s)| (i as i128, s.as_str())).collect();
                T::enumeration(short(name), T::zigzag(), &cases)
            }
            Def::Fixed { size } => T::bytes(E::lit(*size as i128)),
        }
    }

    /// What the node keyed `name` is in `schema`.
    pub(super) fn build(&self, schema: &Schema, name: &str) -> R<T> {
        if name.is_empty() {
            return match &schema.top {
                Some(top) => Ok(self.ty(top)),
                None => Err(EvalError::Failed("The schema is empty.".into())),
            };
        }
        match schema.def(name) {
            Some(def) => Ok(self.def(name, def)),
            None => Err(EvalError::Failed(format!("The schema refers to the type \"{name}\" but never defines it."))),
        }
    }
}

/// A length, then that many bytes. Bytes that are a file of their own open as
/// one, the way a stored ZIP member does: a Rubin alert's cutouts are FITS.
fn bytes() -> T {
    let data = T::switch(
        E::lit(0).less_than(E::field("length")),
        vec![(1, T::decoded(E::field("length"), Codec::Stored, super::decoded_object()))],
        T::bytes(E::lit(0)),
    );
    T::structure_named("bytes", "", "data", vec![("length", T::zigzag()), ("data", data)])
}

/// An array's or a map's items, written in blocks: a count, then that many
/// items, until a count of zero. A negative count is the same number of items
/// with their size in bytes written after it, so that a reader can skip them.
fn blocks(item: T) -> T {
    let count = || E::field("count");
    let items = E::cond(count().less_than(E::lit(0)), E::lit(0).sub(count()), count());
    let block = T::structure_named(
        "block",
        "",
        "items",
        vec![("count", T::zigzag()), ("size", T::when(count().less_than(E::lit(0)), T::zigzag())), ("items", T::array(item, items))],
    )
    .counted_as("block");
    T::repeat(block, Until::FieldValue { field: "count".into(), value: 0 })
}

/// What a container file's block of records is, by the codec its header names.
/// `count` and `size` are the block's own fields, read before this, and the
/// node is already inside a window of `size` bytes.
fn block_data(codec: &str) -> Option<T> {
    let records = T::array(Reach::file().named(""), E::field("count"));
    Some(match codec {
        "null" => records,
        "deflate" => T::decoded(E::Remaining, Codec::Deflate, records),
        // Each block is one raw Snappy block and then the CRC-32 of what it
        // unpacks to, big-endian.
        "snappy" => T::structure_named(
            "SnappyBlock",
            "",
            "data",
            vec![("data", T::decoded(E::Remaining.sub(E::lit(4)), Codec::Snappy, records)), ("crc32", T::u32(Endian::Big))],
        )
        .field_check("crc32", Check::of(Checksum::Crc32, Covers::Unpacked { name: Named::here("data"), len: None })),
        "zstandard" => T::decoded(E::Remaining, Codec::Zstd, records),
        "bzip2" => T::decoded(E::Remaining, Codec::Bzip2, records),
        "xz" => T::decoded(E::Remaining, Codec::Xz, records),
        _ => return None,
    })
}

/// The text of a metadata entry's key, and where its value is.
fn entry<D: Descriptions + ?Sized>(table: &mut D, at: &[usize]) -> R<Option<(String, Vec<usize>)>> {
    let Some(key) = table.find(at, &["key", "text"])? else { return Ok(None) };
    let Some(value) = table.find(at, &["value"])? else { return Ok(None) };
    Ok(Some((table.text(&key)?, value)))
}

/// The metadata entry keyed `want`: where it is, and where its value is.
fn find_entry(table: &mut dyn Descriptions, want: &str) -> R<Option<(Vec<usize>, Vec<usize>)>> {
    for record in table.records()? {
        if let Some((key, value)) = entry(table, &record)? {
            if key == want {
                return Ok(Some((record, value)));
            }
        }
    }
    Ok(None)
}

/// The builder for [`KIND`]: types of the schema a container file's header
/// holds, keyed by name.
#[derive(Debug, Default)]
pub(super) struct FileTypes {
    /// The last schema parsed, by its text. Every named type asks.
    last: Mutex<Option<(Vec<u8>, Arc<Result<Schema, String>>)>>,
}

impl FileTypes {
    fn schema(&self, text: Vec<u8>) -> Arc<Result<Schema, String>> {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, schema)) = last.as_ref().filter(|(t, _)| *t == text) {
            return schema.clone();
        }
        let mut schema = Schema::default();
        let parsed = Arc::new(schema.add(&text).map(|()| schema));
        *last = Some((text, parsed.clone()));
        parsed
    }
}

impl SchemaBuilder for FileTypes {
    fn build(&self, key: &[KeyValue], table: &mut dyn Descriptions) -> R<Built> {
        let [KeyValue::Text(name)] = key else { return Err(EvalError::Failed("not an Avro type".into())) };
        let Some((record, value)) = find_entry(table, "avro.schema")? else {
            return Err(EvalError::Failed("The file header has no \"avro.schema\" entry.".into()));
        };
        let Some(json) = table.find(&value, &["json"])? else {
            return Err(EvalError::Failed("The \"avro.schema\" entry in the file header is not text.".into()));
        };
        let node = table.node(&json)?;
        let text = table.bytes(node.offset_bits / 8, node.size_bits / 8)?;
        let schema = self.schema(text);
        let schema = match schema.as_ref() {
            Ok(s) => s,
            Err(why) => return Err(EvalError::Failed(format!("The \"avro.schema\" entry in the file header is not a valid schema: {why}"))),
        };
        Ok(Built { ty: Reach::file().build(schema, name)?, from: Some(record), members_from: Vec::new() })
    }

    fn key_text(&self, key: &[KeyValue]) -> String {
        match key {
            [KeyValue::Text(n)] if n.is_empty() => "record".into(),
            [KeyValue::Text(n)] => short(n).to_string(),
            _ => "type".into(),
        }
    }
}

/// The builder for [`KIND_BLOCK`]: a block's records, through the codec the
/// header names. A header with no codec stores its blocks as they are.
#[derive(Debug, Default)]
pub(super) struct Blocks;

impl SchemaBuilder for Blocks {
    fn build(&self, _key: &[KeyValue], table: &mut dyn Descriptions) -> R<Built> {
        let (codec, from) = match find_entry(table, "avro.codec")? {
            Some((record, value)) => match table.find(&value, &["text"])? {
                Some(text) => (table.text(&text)?, Some(record)),
                None => return Err(EvalError::Failed("The \"avro.codec\" entry in the file header is not text.".into())),
            },
            None => ("null".to_string(), None),
        };
        match block_data(&codec) {
            Some(ty) => Ok(Built { ty, from, members_from: Vec::new() }),
            None => Err(EvalError::Failed(format!("Shown as compressed bytes: Qubero cannot decompress the {codec} codec named in the file header."))),
        }
    }

    fn key_text(&self, _key: &[KeyValue]) -> String {
        "block".into()
    }
}

/// Whether a schema ID is one of the bundled alert schemas.
pub(super) fn is_alert_id(id: i128) -> bool {
    SCHEMAS.iter().any(|(i, _, _)| *i == id)
}

/// The bundled alert schema with this ID, its files read the way
/// `lsst.alert.packet.Schema.from_file` reads them: the alert's own file, and
/// then the file named after each type it uses and does not define.
pub(super) fn alert_schema(id: i128) -> Result<Schema, String> {
    let files: Vec<(&str, &str)> = SCHEMAS.iter().filter(|(i, _, _)| *i == id).map(|(_, n, t)| (*n, *t)).collect();
    let Some((_, root)) = files.iter().find(|(n, _)| n.ends_with(".alert.avsc")) else {
        return Err(format!(
            "Shown as bytes: this record is written with Rubin alert schema {}.{}, and Qubero includes only schemas {}.",
            id / 100,
            id % 100,
            alert_versions()
        ));
    };
    let mut schema = Schema::default();
    schema.add(root.as_bytes())?;
    let mut tried = FxHashSet::default();
    loop {
        let missing: Vec<_> = schema.missing().into_iter().filter(|n| tried.insert(n.clone())).collect();
        if missing.is_empty() {
            return Ok(schema);
        }
        for name in missing {
            if let Some((_, text)) = files.iter().find(|(n, _)| n.strip_suffix(".avsc") == Some(&*name)) {
                schema.add(text.as_bytes())?;
            }
        }
    }
}

/// The bundled versions, as a reader would write them: `3.0 to 11.1`.
fn alert_versions() -> String {
    let first = SCHEMAS.first().map_or(0, |s| s.0);
    let last = SCHEMAS.last().map_or(0, |s| s.0);
    format!("{}.{} to {}.{}", first / 100, first % 100, last / 100, last % 100)
}

/// The builder for [`KIND_ALERT`]: types of a bundled alert schema, keyed by
/// its ID and the type's name.
#[derive(Debug, Default)]
pub(super) struct AlertTypes {
    parsed: Mutex<FxHashMap<i128, Arc<Result<Schema, String>>>>,
}

impl SchemaBuilder for AlertTypes {
    fn build(&self, key: &[KeyValue], _table: &mut dyn Descriptions) -> R<Built> {
        let [KeyValue::Int(id), KeyValue::Text(name)] = key else { return Err(EvalError::Failed("not an alert type".into())) };
        let schema = {
            let mut parsed = self.parsed.lock().unwrap_or_else(|e| e.into_inner());
            parsed.entry(*id).or_insert_with(|| Arc::new(alert_schema(*id))).clone()
        };
        let schema = match schema.as_ref() {
            Ok(s) => s,
            Err(why) => return Err(EvalError::Failed(why.clone())),
        };
        Ok(Built::by_heart(Reach::alert(E::lit(*id)).build(schema, name)?))
    }

    fn key_text(&self, key: &[KeyValue]) -> String {
        match key {
            [KeyValue::Int(id), KeyValue::Text(n)] if n.is_empty() => format!("alert schema {}.{}", id / 100, id % 100),
            [_, KeyValue::Text(n)] => short(n).to_string(),
            _ => "type".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_take_the_namespace_they_are_written_in() {
        let mut s = Schema::default();
        s.add(br#"{"type": "record", "name": "a", "namespace": "x.y", "fields": [
            {"name": "b", "type": {"type": "enum", "name": "e", "symbols": ["P", "Q"]}},
            {"name": "c", "type": "e"},
            {"name": "d", "type": ["null", "a"]},
            {"name": "f", "type": {"type": "fixed", "name": "z.f", "size": 16}},
            {"name": "t", "type": {"type": "long", "logicalType": "timestamp-micros"}}
        ]}"#)
            .unwrap();
        assert_eq!(s.top, Some(Avro::Named("x.y.a".into())));
        let Some(Def::Record { fields, .. }) = s.def("x.y.a") else { panic!("{:?}", s.defs) };
        assert_eq!(fields[1].ty, Avro::Named("x.y.e".into()));
        assert_eq!(fields[2].ty, Avro::Union(vec![Avro::Null, Avro::Named("x.y.a".into())]));
        assert_eq!(fields[3].ty, Avro::Named("z.f".into()));
        assert_eq!(fields[4].ty, Avro::Long(Some(Stamp::Micros)));
        assert!(s.missing().is_empty());
    }

    #[test]
    fn every_bundled_alert_schema_resolves_all_its_names() {
        let ids: FxHashSet<i128> = SCHEMAS.iter().map(|s| s.0).collect();
        assert_eq!(ids.len(), 15);
        for id in ids {
            let s = alert_schema(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert!(s.missing().is_empty(), "{id}: {:?}", s.missing());
            let Some(Avro::Named(top)) = &s.top else { panic!("{id}: {:?}", s.top) };
            assert!(top.ends_with(".alert"), "{id}: {top}");
        }
    }

    #[test]
    fn an_id_that_is_not_bundled_says_which_are() {
        let e = alert_schema(1205).unwrap_err();
        assert_eq!(e, "Shown as bytes: this record is written with Rubin alert schema 12.5, and Qubero includes only schemas 3.0 to 11.1.");
    }
}
