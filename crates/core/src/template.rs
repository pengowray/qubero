//! Template IR: a description of the structure a file is expected to have.
//!
//! Nothing here is a static layout. Lengths, counts and choices are expressions
//! over earlier fields, so a template can say "an array of u32 whose count is
//! the field named `n`" or "bytes whose length is `size`, parsed as the section
//! type selected by `id`". Evaluation lives in `eval.rs`.

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

/// An integer-valued expression. `Ref` names a field that appears earlier in the
/// same struct, or in an enclosing struct before the current field.
#[derive(Debug, Clone)]
pub enum Expr {
    Lit(i128),
    Ref(String),
    /// Size in bytes of an earlier field. Needed when a field runs to the end
    /// of a container and what came before it was variable length.
    SizeOf(String),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
}

impl Expr {
    pub fn lit(v: impl Into<i128>) -> Expr {
        Expr::Lit(v.into())
    }
    pub fn field(name: &str) -> Expr {
        Expr::Ref(name.to_string())
    }
    /// The byte size of an earlier field.
    pub fn size_of(name: &str) -> Expr {
        Expr::SizeOf(name.to_string())
    }
    pub fn add(self, rhs: Expr) -> Expr {
        Expr::Add(Box::new(self), Box::new(rhs))
    }
    pub fn sub(self, rhs: Expr) -> Expr {
        Expr::Sub(Box::new(self), Box::new(rhs))
    }
    pub fn mul(self, rhs: Expr) -> Expr {
        Expr::Mul(Box::new(self), Box::new(rhs))
    }
    pub fn div(self, rhs: Expr) -> Expr {
        Expr::Div(Box::new(self), Box::new(rhs))
    }
}

#[derive(Debug, Clone)]
pub enum Until {
    /// Repeat until the enclosing size limit (or end of file) is reached.
    End,
    /// Repeat until an element whose field `field` has the given raw bytes
    /// (that element is included).
    FieldBytes { field: String, bytes: Vec<u8> },
}

/// Names for the values an integer field is expected to take: `color_type` 6
/// reads as "rgba". The underlying integer is untouched, so expressions and
/// switches still see the number, and a value with no name is still shown.
#[derive(Debug)]
pub struct EnumDef {
    pub name: String,
    pub cases: Vec<(i128, String)>,
    /// Show the number in hex. True for sets people read in hex, such as wasm
    /// opcodes and value types.
    pub hex: bool,
}

impl EnumDef {
    pub fn label(&self, v: i128) -> Option<&str> {
        self.cases.iter().find(|(k, _)| *k == v).map(|(_, n)| n.as_str())
    }
    pub fn value_of(&self, name: &str) -> Option<i128> {
        self.cases.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(k, _)| *k)
    }
}

/// How far a text field runs. Formats disagree, so the template says which:
/// a fixed run of bytes, a fixed run whose tail is padding, or a run that ends
/// at a terminator byte.
#[derive(Debug, Clone)]
pub enum StrLen {
    /// Exactly this many bytes, all of them part of the value.
    Fixed(Expr),
    /// This many bytes, of which the value is everything before the first `pad`
    /// byte. Writing a shorter value pads the rest, so the field keeps its size.
    Padded { size: Expr, pad: u8 },
    /// Runs to the first `end` byte, which belongs to the field. A C string.
    Terminated { end: u8 },
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Ty,
}

#[derive(Debug, Clone)]
pub enum Ty {
    UInt { bits: u32, endian: Endian },
    Int { bits: u32, endian: Endian },
    F16(Endian),
    F32(Endian),
    F64(Endian),
    /// Unsigned LEB128 (as used by wasm). Signed variant reads sign-extended.
    Leb128 { signed: bool },
    /// Fixed-point: `bits` wide with `frac` fraction bits, so MP4's 16.16 rate
    /// of 0x00010000 reads as 1.
    Fixed { bits: u32, frac: u32, endian: Endian, signed: bool },
    /// Fixed bytes that must match.
    Magic(Vec<u8>),
    /// Raw bytes of computed length (in bytes).
    Bytes(Expr),
    /// UTF-8 text. How its length is decided is up to `StrLen`.
    Str { len: StrLen },
    Struct(Arc<StructDef>),
    Array { elem: Box<Ty>, count: Expr },
    Repeat { elem: Box<Ty>, until: Until },
    /// Occupies exactly `size` bytes; `inner` is parsed within that window.
    Sized { size: Expr, inner: Box<Ty> },
    /// Pick a type by the value of `on`; falls back to `default`.
    Switch { on: Expr, cases: Vec<(i128, Ty)>, default: Box<Ty> },
    /// An integer type whose values have names.
    Enum { inner: Box<Ty>, def: Arc<EnumDef> },
    /// A type from the template's table, by name. This is what makes a format
    /// whose boxes contain boxes expressible: the type refers to itself.
    Named(String),
}

#[derive(Debug)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<Field>,
}

impl Ty {
    pub fn u8() -> Ty {
        Ty::UInt { bits: 8, endian: Endian::Little }
    }
    pub fn u16(e: Endian) -> Ty {
        Ty::UInt { bits: 16, endian: e }
    }
    pub fn u32(e: Endian) -> Ty {
        Ty::UInt { bits: 32, endian: e }
    }
    pub fn u64(e: Endian) -> Ty {
        Ty::UInt { bits: 64, endian: e }
    }
    pub fn i32(e: Endian) -> Ty {
        Ty::Int { bits: 32, endian: e }
    }
    /// Unsigned fixed-point, e.g. `fixed(32, 16, Big)` for MP4's 16.16.
    pub fn fixed(bits: u32, frac: u32, endian: Endian) -> Ty {
        Ty::Fixed { bits, frac, endian, signed: false }
    }
    pub fn leb_u() -> Ty {
        Ty::Leb128 { signed: false }
    }
    pub fn bytes(len: Expr) -> Ty {
        Ty::Bytes(len)
    }
    pub fn utf8(len: Expr) -> Ty {
        Ty::Str { len: StrLen::Fixed(len) }
    }
    /// Text in a field of `size` bytes, ending at the first `pad` byte.
    pub fn utf8_padded(size: Expr, pad: u8) -> Ty {
        Ty::Str { len: StrLen::Padded { size, pad } }
    }
    /// Text that ends at a NUL, which is part of the field.
    pub fn cstr() -> Ty {
        Ty::Str { len: StrLen::Terminated { end: 0 } }
    }
    pub fn magic(b: &[u8]) -> Ty {
        Ty::Magic(b.to_vec())
    }
    pub fn structure(name: &str, fields: Vec<(&str, Ty)>) -> Ty {
        Ty::Struct(Arc::new(StructDef {
            name: name.to_string(),
            fields: fields.into_iter().map(|(n, ty)| Field { name: n.to_string(), ty }).collect(),
        }))
    }
    pub fn array(elem: Ty, count: Expr) -> Ty {
        Ty::Array { elem: Box::new(elem), count }
    }
    pub fn repeat(elem: Ty, until: Until) -> Ty {
        Ty::Repeat { elem: Box::new(elem), until }
    }
    pub fn sized(size: Expr, inner: Ty) -> Ty {
        Ty::Sized { size, inner: Box::new(inner) }
    }
    pub fn switch(on: Expr, cases: Vec<(i128, Ty)>, default: Ty) -> Ty {
        Ty::Switch { on, cases, default: Box::new(default) }
    }
    pub fn enumeration(name: &str, inner: Ty, cases: &[(i128, &str)]) -> Ty {
        Ty::enum_with(name, inner, cases, false)
    }
    /// An enum whose numbers are shown in hex.
    pub fn enumeration_hex(name: &str, inner: Ty, cases: &[(i128, &str)]) -> Ty {
        Ty::enum_with(name, inner, cases, true)
    }
    fn enum_with(name: &str, inner: Ty, cases: &[(i128, &str)], hex: bool) -> Ty {
        Ty::Enum {
            inner: Box::new(inner),
            def: Arc::new(EnumDef {
                name: name.to_string(),
                cases: cases.iter().map(|(v, n)| (*v, n.to_string())).collect(),
                hex,
            }),
        }
    }

    /// The integer type under an enum, or the type itself.
    pub fn base(&self) -> &Ty {
        match self {
            Ty::Enum { inner, .. } => inner.base(),
            other => other,
        }
    }

    /// Short human-readable type name for the type table.
    pub fn display_name(&self) -> String {
        fn e(en: Endian) -> &'static str {
            match en {
                Endian::Little => "le",
                Endian::Big => "be",
            }
        }
        match self {
            Ty::UInt { bits, endian } if *bits <= 8 => format!("u{bits}"),
            Ty::UInt { bits, endian } => format!("u{bits} {}", e(*endian)),
            Ty::Int { bits, endian } if *bits <= 8 => format!("i{bits}"),
            Ty::Int { bits, endian } => format!("i{bits} {}", e(*endian)),
            Ty::F16(en) => format!("f16 {}", e(*en)),
            Ty::F32(en) => format!("f32 {}", e(*en)),
            Ty::F64(en) => format!("f64 {}", e(*en)),
            Ty::Fixed { bits, frac, endian, signed } => {
                format!("{}{}.{frac} {}", if *signed { "i" } else { "u" }, bits - frac, e(*endian))
            }
            Ty::Leb128 { signed: false } => "leb128".into(),
            Ty::Leb128 { signed: true } => "sleb128".into(),
            Ty::Magic(b) => format!("magic[{}]", b.len()),
            Ty::Bytes(_) => "bytes[]".into(),
            Ty::Str { len } => match len {
                StrLen::Fixed(_) => "utf8[]".into(),
                StrLen::Padded { pad: 0, .. } => "utf8[] nul-padded".into(),
                StrLen::Padded { pad, .. } => format!("utf8[] padded 0x{pad:02x}"),
                StrLen::Terminated { end: 0 } => "cstr".into(),
                StrLen::Terminated { end } => format!("utf8 to 0x{end:02x}"),
            },
            Ty::Struct(s) => s.name.clone(),
            Ty::Array { elem, .. } => format!("{}[]", elem.display_name()),
            Ty::Repeat { elem, .. } => format!("{}[]", elem.display_name()),
            Ty::Sized { inner, .. } => inner.display_name(),
            Ty::Switch { .. } => "switch".into(),
            Ty::Enum { def, .. } => def.name.clone(),
            Ty::Named(n) => n.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Template {
    pub name: String,
    pub root: Ty,
    /// Types a `Ty::Named` can refer to, including the root's own type when a
    /// format nests inside itself.
    pub types: HashMap<String, Ty>,
}

impl Template {
    pub fn new(name: &str, root: Ty) -> Template {
        Template { name: name.to_string(), root, types: HashMap::new() }
    }
    pub fn with_type(mut self, name: &str, ty: Ty) -> Template {
        self.types.insert(name.to_string(), ty);
        self
    }
}
