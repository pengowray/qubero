//! Template IR: a description of the structure a file is expected to have.
//!
//! Nothing here is a static layout. Lengths, counts and choices are expressions
//! over earlier fields, so a template can say "an array of u32 whose count is
//! the field named `n`" or "bytes whose length is `size`, parsed as the section
//! type selected by `id`". Evaluation lives in `eval.rs`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::json;

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
    /// Names are shared rather than copied: a type is cloned every time an
    /// element of a list is placed, and a list may have a million elements.
    Ref(Arc<str>),
    /// Bytes from here to the end of the enclosing container: what a field that
    /// runs to the end is worth. An MP4 box of size 0 means exactly this.
    Remaining,
    /// Size in bytes of an earlier field. Needed when a field runs to the end
    /// of a container and what came before it was variable length.
    SizeOf(Arc<str>),
    /// This element's index in the nearest list it sits in. Zero outside one.
    Idx,
    /// The value of one element of an earlier array, by index. `Ref` names a
    /// field; this reaches inside one, which is what a list of pointers or a
    /// list of column types needs. When the elements are structures, `field`
    /// walks named fields down to the number: `tensors[i].offset` is
    /// `Elem { array: "tensors", field: ["offset"] }`. Empty when the elements
    /// are the numbers themselves.
    Elem { array: Arc<str>, index: Box<Expr>, field: Arc<[String]> },
    /// The numbers of one earlier array multiplied together: what a shape
    /// describes. A GGUF tensor says it is 2560 by 5120 and never says it is
    /// 13,107,200 numbers, and the room between one tensor and the next is not
    /// the answer either, since a small tensor is followed by padding.
    /// Reached the same way as `Elem`, but landing on an array rather than on
    /// a number: `tensors[i].dims` is
    /// `Product { array: "tensors", index: Idx, field: ["dims"] }`.
    Product { array: Arc<str>, index: Box<Expr>, field: Arc<[String]> },
    /// The numbers of an earlier array field, multiplied together. `Product`
    /// reaches into one element of a list of records; this one names an array
    /// that is a sibling of the field asking, which is how an older ggml file
    /// writes a tensor's shape: the record holds `ne` itself rather than a
    /// table of records to look in.
    ProductOf(Arc<str>),
    /// The next `bits` bits, read without consuming them. A field can then
    /// exist only when the byte at its own start says it does.
    ///
    /// A peek says which way round it reads, the same as a field does. Bits
    /// narrower than a byte are packed the one way whatever it says, which is
    /// also what a field of them does.
    Peek { bits: u32, endian: Endian },
    /// `bits` bits read `skip` bits further on, without consuming anything.
    /// `Peek` looks at the field's own first byte, which is no use when what
    /// decides the shape of a record is written after the fields it decides:
    /// an LHA header says at offset 20 which of three layouts the twenty
    /// bytes before it were written in. Looking past the end of the container
    /// is an error, the same as `Peek`, so a record too short to hold the
    /// byte says so rather than guessing at it.
    ///
    /// How far to look is itself an expression, and a negative distance counts
    /// back from the end of the container rather than forward from here. That
    /// is what reaches a signature written at the end of a file: a TGA 2.0 is
    /// told from a TGA by the last eight bytes of one, wherever in the file the
    /// asking is done.
    PeekAt { skip: Box<Expr>, bits: u32, endian: Endian },
    /// The value of field `name` in the element before this one, in the nearest
    /// enclosing list. Zero for the first element, and for anything not in a
    /// list. This is what a format carrying state between elements needs.
    Prev(Arc<str>),
    /// The value at `field` in the nearest earlier element that has one,
    /// searching backwards through the enclosing list and then outwards
    /// through the lists that one sits in. `Prev` asks only the element just
    /// before, which is no use when what a chunk means was settled by a chunk
    /// further back: a WAVE `data` chunk is samples of whatever width `fmt `
    /// declared, however many chunks sit in between, and one sample is two
    /// lists further in again. Zero when nothing earlier has it, so `Or` can
    /// name what to do without one.
    Sibling(Arc<[String]>),
    /// The first of the two that is not zero. Pairs with `Prev` to say "this
    /// one, or the last one that had one".
    Or(Box<Expr>, Box<Expr>),
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
        Expr::Ref(name.into())
    }
    /// The byte size of an earlier field.
    pub fn size_of(name: &str) -> Expr {
        Expr::SizeOf(name.into())
    }
    /// This element's index in the nearest enclosing list.
    pub fn idx() -> Expr {
        Expr::Idx
    }
    /// Element `index` of the earlier array field `array`.
    pub fn elem(array: &str, index: Expr) -> Expr {
        Expr::Elem { array: array.into(), index: Box::new(index), field: Arc::from(Vec::new()) }
    }
    /// Field `field` of element `index` of the earlier array `array`, for an
    /// array whose elements are structures rather than numbers.
    pub fn elem_field(array: &str, index: Expr, field: &[&str]) -> Expr {
        Expr::Elem {
            array: array.into(),
            index: Box::new(index),
            field: field.iter().map(|s| s.to_string()).collect(),
        }
    }
    /// The numbers of the array at `field` inside `array[index]`, multiplied
    /// together.
    pub fn product(array: &str, index: Expr, field: &[&str]) -> Expr {
        Expr::Product {
            array: array.into(),
            index: Box::new(index),
            field: field.iter().map(|s| s.to_string()).collect(),
        }
    }
    /// The numbers of the earlier array field `name`, multiplied together.
    pub fn product_of(name: &str) -> Expr {
        Expr::ProductOf(name.into())
    }
    /// The next `bits` bits without consuming them, read the given way round.
    pub fn peek(bits: u32, endian: Endian) -> Expr {
        Expr::Peek { bits, endian }
    }
    /// `bits` bits, `skip` bits further on, without consuming them. A negative
    /// `skip` counts back from the end of the container.
    pub fn peek_at(skip: Expr, bits: u32, endian: Endian) -> Expr {
        Expr::PeekAt { skip: Box::new(skip), bits, endian }
    }
    /// Field `name` of the previous element of the enclosing list.
    pub fn prev(name: &str) -> Expr {
        Expr::Prev(name.into())
    }
    /// The value at `field` in the nearest earlier element of the enclosing
    /// list that has one, e.g. `sibling(&["body", "bits_per_sample"])`.
    pub fn sibling(field: &[&str]) -> Expr {
        Expr::Sibling(field.iter().map(|s| s.to_string()).collect())
    }
    /// This, or `rhs` when this is zero.
    pub fn or(self, rhs: Expr) -> Expr {
        Expr::Or(Box::new(self), Box::new(rhs))
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

/// Names for the individual bits of an integer field: a PE's `characteristics`
/// is eight independent answers packed into sixteen bits, and reading it as the
/// number 550 asks the reader to do the unpacking. Bits with no name still
/// exist and are still shown, because a set bit nobody named is exactly the
/// kind of thing worth noticing.
#[derive(Debug)]
pub struct FlagsDef {
    pub name: String,
    /// Bit number, counting from the least significant, and what it means.
    pub bits: Vec<(u32, String)>,
}

impl FlagsDef {
    pub fn label(&self, bit: u32) -> Option<&str> {
        self.bits.iter().find(|(b, _)| *b == bit).map(|(_, n)| n.as_str())
    }
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

/// What the bytes of a text field mean. The last two are for formats that do
/// not say outright: one where the bytes announce themselves, one where nobody
/// knows and a guess is the honest answer.
#[derive(Debug, Clone)]
pub enum Encoding {
    Utf8,
    Ascii,
    /// ISO 8859-1, where every byte is a character.
    Latin1,
    /// The DOS code page, which fills the high half with box drawing and accents.
    Cp437,
    Utf16(Endian),
    /// A byte-order mark at the front decides; without one, `fallback`.
    Bom { fallback: Box<Encoding> },
    /// The format does not say. Read as UTF-8 if the bytes are valid UTF-8,
    /// otherwise Latin-1, and say which was used.
    Unknown,
}

impl Encoding {
    /// Short name for the type column.
    pub fn short(&self) -> String {
        match self {
            Encoding::Utf8 => "utf8".into(),
            Encoding::Ascii => "ascii".into(),
            Encoding::Latin1 => "latin1".into(),
            Encoding::Cp437 => "cp437".into(),
            Encoding::Utf16(Endian::Little) => "utf16le".into(),
            Encoding::Utf16(Endian::Big) => "utf16be".into(),
            Encoding::Bom { .. } => "text bom".into(),
            Encoding::Unknown => "text?".into(),
        }
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
    /// Skips any leading bytes in `skip`, then runs to the first byte in
    /// `ends`, which belongs to the field. What a format separating its
    /// fields by whitespace needs, where `Terminated` can name only one byte
    /// and cannot be told to step over the run before the value: a netpbm
    /// header writes its numbers with a space, a tab or a newline between
    /// them, and any run of those, in whatever mixture.
    ///
    /// With `comment`, a run from the first byte to the second is stepped over
    /// too, wherever one appears among the separators. That is a comment in
    /// every format that writes its numbers as text: `#` to the end of the
    /// line in a netpbm file. Only in the run before the value, which is where
    /// anything that writes them puts them.
    ///
    /// The skipped bytes and the terminator are the format's business, not
    /// the value's, so neither is part of what the field reads as. Writing one
    /// would have to decide how much whitespace to put back, so these are
    /// read-only, as terminated fields are.
    Scan { skip: Vec<u8>, ends: Vec<u8>, comment: Option<(u8, u8)> },
    /// Runs to the first `end` byte, which belongs to the field. A C string.
    /// With `or_end`, a field with no terminator in it runs to the end of its
    /// container instead of failing, which is what a last line without a
    /// newline needs. Such a field is read-only: writing one would have to add
    /// the terminator, and that would change the size.
    Terminated { end: u8, or_end: bool },
}

/// Where the offsets in a `PointerList` count from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// The start of the nearest `Sized` window around the list, which is the
    /// unit a format that keeps offsets inside itself counts from: a page of a
    /// database, a table of a font. Without one, the start of the file.
    Window,
    /// The start of the file.
    File,
    /// The list's own start, rounded up to a multiple of this many bytes.
    /// GGUF's tensor data starts at the end of the tensor table aligned to
    /// `general.alignment`, which is almost always 32; a file that sets it to
    /// something else places its tensors quietly wrong here, since nothing
    /// generic can read a metadata value by key.
    SelfAligned(u32),
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: Arc<str>,
    pub ty: Ty,
}

#[derive(Debug, Clone)]
pub enum Ty {
    UInt { bits: u32, endian: Endian },
    Int { bits: u32, endian: Endian },
    F16(Endian),
    /// Brain float: the top half of an f32, so it reaches as far as a float
    /// does and holds three digits rather than seven. Not an f16, which trades
    /// the other way: same sixteen bits, five of exponent and ten of fraction.
    BF16(Endian),
    F32(Endian),
    F64(Endian),
    /// Eighty bits: a sign, fifteen of exponent and sixty-four of significand
    /// with its leading one written out rather than assumed. The long double
    /// of the 68881 and the x87, and what an AIFF writes its sample rate as.
    /// Read into an f64, which keeps 53 of those 64 bits, so a value using all
    /// of them rounds; the numbers formats actually store this way, sample
    /// rates among them, are exact.
    F80(Endian),
    /// An eight-bit float, which is how the weights of a quantised model are
    /// written now. `e4m3` spends four bits on the exponent and three on the
    /// fraction and reaches 448; it has no infinities, and only an exponent
    /// and a fraction of all ones is not a number. The other, `e5m2`, spends
    /// five and two, reaches 57344, and does have them.
    F8 { e4m3: bool },
    /// A field of no bits whose value is worked out rather than read. What it
    /// takes to say "the same as the last one" without inventing a byte.
    Computed(Expr),
    /// Unsigned LEB128 (as used by wasm). Signed variant reads sign-extended.
    Leb128 { signed: bool },
    /// MIDI's variable-length quantity: seven bits per byte, most significant
    /// group first, high bit set on every byte but the last. LEB128 packs the
    /// same seven bits the other way round, so it cannot stand in for this.
    Vlq,
    /// Fixed-point: `bits` wide with `frac` fraction bits, so MP4's 16.16 rate
    /// of 0x00010000 reads as 1.
    Fixed { bits: u32, frac: u32, endian: Endian, signed: bool },
    /// Fixed bytes that must match.
    Magic(Vec<u8>),
    /// Raw bytes of computed length (in bytes).
    Bytes(Expr),
    /// Text. `StrLen` says how far it runs, `enc` says what the bytes mean.
    Str { len: StrLen, enc: Encoding },
    Struct(Arc<StructDef>),
    Array { elem: Box<Ty>, count: Expr },
    Repeat { elem: Box<Ty>, until: Until },
    /// Children placed at offsets read from an earlier array of numbers,
    /// rather than one after another. Element `i` starts at
    /// `anchor + adjust + offsets[i]`, so the children can be in any order and
    /// need not fill the space. The list itself runs from where it is declared
    /// to the end of its container, which is the region those offsets point
    /// into; declare it last. Anything no child covers reads as a gap.
    /// `field` reaches into `offsets` when its elements are structures: a GGUF
    /// tensor table holds each offset inside a record, not as a bare number.
    /// With `to_next`, a child runs to the start of the next child above it
    /// (or the end of the list), for formats that store no per-child size.
    /// With `skip_zero`, an offset of zero points at nothing too, which is
    /// what a format writes in a fixed-size table of pointers for the entries
    /// it has nothing for: a Minecraft region keeps room for 1024 chunks and
    /// writes zero for every one the world has not reached yet.
    /// With `skip_missing`, an entry of `offsets` with no such field points at
    /// nothing rather than making the list unreadable: it keeps its place
    /// among the children and covers no bytes. A safetensors header holds the
    /// file's own metadata among the tensors, and no weights belong to it.
    PointerList {
        offsets: Arc<str>,
        field: Arc<[String]>,
        anchor: Anchor,
        adjust: Expr,
        elem: Box<Ty>,
        to_next: bool,
        skip_missing: bool,
        skip_zero: bool,
    },
    /// SQLite's variable-length integer: seven bits per byte, most significant
    /// group first, up to nine bytes, where a ninth byte contributes all eight
    /// of its bits. `Vlq` stops at four bytes and never does that, so it
    /// cannot stand in.
    SqliteVarint,
    /// A field of no bits whose contents are read somewhere else in the file:
    /// `inner`, placed at `at` bytes from the start of the file, with nothing
    /// of it where the field itself is declared.
    ///
    /// This is what a header pointing at a table needs. A `PointerList` places
    /// children at offsets read from an earlier field, so the offsets have to
    /// be read before the bytes they point at; a WAD keeps its directory at
    /// the end of the file and says in the header where it is. Reading the
    /// directory here, at no cost in bytes, puts the offsets in hand while the
    /// cursor is still at the front, and the list of lumps can then be
    /// declared over the region they sit in.
    ///
    /// The field advances nothing, so what follows it is placed as if it were
    /// not there. Its one child is the only thing that covers bytes.
    ///
    /// A structure is still as long as its last field ends, and what this
    /// points at may be past that. The cursor can only land in what the
    /// structure covers, so a table sitting beyond the end of everything else
    /// is read and shown but not landed on. In a format that keeps its table
    /// inside itself, which is every format that does this, something else
    /// already covers the stretch and the question does not come up.
    At { at: Expr, inner: Box<Ty> },
    /// Occupies exactly `size` bytes; `inner` is parsed within that window.
    Sized { size: Expr, inner: Box<Ty> },
    /// Pick a type by the value of `on`; falls back to `default`. The cases
    /// are shared rather than owned: resolving a field clones its type, and
    /// a switch with thirteen cases is cloned once per element of a list
    /// that may run to millions.
    Switch { on: Expr, cases: Arc<[(i128, Ty)]>, default: Arc<Ty> },
    /// An integer type whose values have names.
    Enum { inner: Box<Ty>, def: Arc<EnumDef> },
    /// An integer type whose bits have names.
    Flags { inner: Box<Ty>, def: Arc<FlagsDef> },
    /// Text read as the JSON it holds, so that every value inside it is a
    /// node of its own at the bytes it is written at. A safetensors file is a
    /// JSON header and then the weights that header describes, and reading the
    /// header as one long string would leave the file's whole structure inside
    /// a single row.
    ///
    /// A template writes `Ty::json()`, which is the whole text; the shapes
    /// below that are what the values found inside it are given.
    Json(json::Shape),
    /// Pick a type by the text of an earlier field, for a format that names
    /// its types in words rather than in numbers: a safetensors tensor says
    /// `"dtype": "F8_E4M3"`. `on` names the field, the way `Switch` does with
    /// a number.
    Match { on: Expr, cases: Arc<[(String, Ty)]>, default: Arc<Ty> },
    /// A type from the template's table, by name. This is what makes a format
    /// whose boxes contain boxes expressible: the type refers to itself.
    Named(Arc<str>),
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<Field>,
    /// Which of this structure's fields names it. A RIFF chunk is identified
    /// by its `id`, not by the name of the field holding its contents, and
    /// nothing generic can work out which sibling that is: guessing at the
    /// first primitive child works for RIFF and fails on PNG, where the length
    /// comes before the type.
    pub named_by: Option<String>,
    /// Which field is merely this structure's contents. Its name says nothing
    /// the structure has not already said, so the linear views leave it out of
    /// the trail: `sections[9] code` beats `sections[9] code, body`. The field
    /// tree keeps it, because there it is a row the reader opens.
    pub contents: Option<String>,
    /// What one of these is called when they are counted: a run of quantised
    /// weights is `97,280 blocks`, not `97,280 items`. Only the format knows
    /// the word, so only the format sets it; without one, a list counts its
    /// children as items.
    pub unit: Option<Arc<str>>,
    /// Read this structure as one thing rather than as its fields. A wasm
    /// instruction is an opcode and its immediate, and splitting those across
    /// two rows says less than one row saying `local.get 0`. Only the linear
    /// views honour it: `locate` still walks inside, so the cursor keeps its
    /// bit precision and the field tree still opens the structure up.
    pub inline: bool,
    /// Names a packing whose contents only the format can take apart, where
    /// the fields say where the packed bytes are but not what they hold. A
    /// ggml `q4_k` block is 256 weights in 144 bytes, packed in an order no
    /// template can describe; this is the hook the format's own unpacker is
    /// found by.
    pub packed: Option<Arc<str>>,
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
    /// An eight-bit float. See [`Ty::F8`].
    pub fn f8(e4m3: bool) -> Ty {
        Ty::F8 { e4m3 }
    }
    pub fn leb_u() -> Ty {
        Ty::Leb128 { signed: false }
    }
    /// A field of no bits whose value is an expression.
    pub fn computed(e: Expr) -> Ty {
        Ty::Computed(e)
    }
    pub fn vlq() -> Ty {
        Ty::Vlq
    }
    pub fn bytes(len: Expr) -> Ty {
        Ty::Bytes(len)
    }
    pub fn utf8(len: Expr) -> Ty {
        Ty::text(StrLen::Fixed(len), Encoding::Utf8)
    }
    /// Text in a field of `size` bytes, ending at the first `pad` byte.
    pub fn utf8_padded(size: Expr, pad: u8) -> Ty {
        Ty::text(StrLen::Padded { size, pad }, Encoding::Utf8)
    }
    /// UTF-8 that ends at a NUL, which is part of the field.
    pub fn cstr() -> Ty {
        Ty::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Utf8)
    }
    pub fn text(len: StrLen, enc: Encoding) -> Ty {
        Ty::Str { len, enc }
    }
    /// The text in this field, read as the JSON it holds.
    pub fn json() -> Ty {
        Ty::Json(json::Shape::Doc)
    }
    /// Pick a type by the text of the field `on` names.
    pub fn matches(on: Expr, cases: Vec<(&str, Ty)>, default: Ty) -> Ty {
        Ty::Match {
            on,
            cases: cases.into_iter().map(|(k, t)| (k.to_string(), t)).collect(),
            default: Arc::new(default),
        }
    }
    pub fn magic(b: &[u8]) -> Ty {
        Ty::Magic(b.to_vec())
    }
    pub fn structure(name: &str, fields: Vec<(&str, Ty)>) -> Ty {
        Ty::Struct(Arc::new(StructDef {
            name: name.to_string(),
            fields: fields.into_iter().map(|(n, ty)| Field { name: n.into(), ty }).collect(),
            named_by: None,
            contents: None,
            unit: None,
            inline: false,
            packed: None,
        }))
    }
    /// A structure that one of its own fields names, and one field that is
    /// only its contents. Either may be empty. See [`StructDef::named_by`] and
    /// [`StructDef::contents`].
    pub fn structure_named(name: &str, named_by: &str, contents: &str, fields: Vec<(&str, Ty)>) -> Ty {
        let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
        match Ty::structure(name, fields) {
            Ty::Struct(s) => {
                Ty::Struct(Arc::new(StructDef { named_by: some(named_by), contents: some(contents), ..(*s).clone() }))
            }
            other => other,
        }
    }
    /// What one of these is called when a list of them is counted, e.g.
    /// `block`, so the row reads `97,280 blocks`. See [`StructDef::unit`].
    pub fn counted_as(self, unit: &str) -> Ty {
        match self {
            Ty::Struct(s) => Ty::Struct(Arc::new(StructDef { unit: Some(unit.into()), ..(*s).clone() })),
            other => other,
        }
    }

    /// A structure whose contents are packed in a way only the format can take
    /// apart, named so that the format's unpacker can be found again. See
    /// [`StructDef::packed`].
    pub fn packed_as(self, packing: &str) -> Ty {
        match self {
            Ty::Struct(s) => Ty::Struct(Arc::new(StructDef { packed: Some(packing.into()), ..(*s).clone() })),
            other => other,
        }
    }

    /// A structure the linear views show on one row, rather than one row per
    /// field. See [`StructDef::inline`].
    pub fn inline_structure(name: &str, fields: Vec<(&str, Ty)>) -> Ty {
        match Ty::structure(name, fields) {
            Ty::Struct(s) => Ty::Struct(Arc::new(StructDef { inline: true, ..(*s).clone() })),
            other => other,
        }
    }
    pub fn array(elem: Ty, count: Expr) -> Ty {
        Ty::Array { elem: Box::new(elem), count }
    }
    pub fn repeat(elem: Ty, until: Until) -> Ty {
        Ty::Repeat { elem: Box::new(elem), until }
    }
    /// Elements at the offsets held in an earlier array field.
    pub fn pointer_list(offsets: &str, anchor: Anchor, adjust: Expr, elem: Ty) -> Ty {
        Ty::PointerList {
            offsets: offsets.into(),
            field: Arc::from(Vec::new()),
            anchor,
            adjust,
            elem: Box::new(elem),
            to_next: false,
            skip_missing: false,
            skip_zero: false,
        }
    }
    /// A pointer list whose offsets sit inside the records of `offsets`, in
    /// field `field`, and whose children run to the next child's start.
    pub fn pointer_list_records(offsets: &str, field: &[&str], anchor: Anchor, adjust: Expr, elem: Ty) -> Ty {
        Ty::PointerList {
            offsets: offsets.into(),
            field: field.iter().map(|s| s.to_string()).collect(),
            anchor,
            adjust,
            elem: Box::new(elem),
            to_next: true,
            skip_missing: false,
            skip_zero: false,
        }
    }
    /// A pointer list whose children have a size of their own, and where an
    /// entry that names no offset points at nothing. See
    /// [`Ty::PointerList::skip_missing`].
    pub fn pointer_list_sized(offsets: &str, field: &[&str], anchor: Anchor, adjust: Expr, elem: Ty) -> Ty {
        Ty::PointerList {
            offsets: offsets.into(),
            field: field.iter().map(|s| s.to_string()).collect(),
            anchor,
            adjust,
            elem: Box::new(elem),
            to_next: false,
            skip_missing: true,
            skip_zero: false,
        }
    }
    /// A pointer list where an offset of zero points at nothing rather than at
    /// the anchor. See [`Ty::PointerList::skip_zero`].
    pub fn skipping_zero(self) -> Ty {
        match self {
            Ty::PointerList { offsets, field, anchor, adjust, elem, to_next, skip_missing, .. } => {
                Ty::PointerList { offsets, field, anchor, adjust, elem, to_next, skip_missing, skip_zero: true }
            }
            other => other,
        }
    }
    pub fn sqlite_varint() -> Ty {
        Ty::SqliteVarint
    }
    /// `inner`, read at `at` bytes from the start of the file, in a field that
    /// takes up no room where it is declared. See [`Ty::At`].
    pub fn at(at: Expr, inner: Ty) -> Ty {
        Ty::At { at, inner: Box::new(inner) }
    }
    pub fn sized(size: Expr, inner: Ty) -> Ty {
        Ty::Sized { size, inner: Box::new(inner) }
    }
    pub fn switch(on: Expr, cases: Vec<(i128, Ty)>, default: Ty) -> Ty {
        Ty::Switch { on, cases: cases.into(), default: Arc::new(default) }
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
    /// An integer whose bits are named. `bits` counts from the least
    /// significant, which is how every format that has them numbers them.
    pub fn flags(name: &str, inner: Ty, bits: &[(u32, &str)]) -> Ty {
        Ty::Flags {
            inner: Box::new(inner),
            def: Arc::new(FlagsDef {
                name: name.to_string(),
                bits: bits.iter().map(|(b, n)| (*b, n.to_string())).collect(),
            }),
        }
    }

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
            Ty::BF16(en) => format!("bf16 {}", e(*en)),
            Ty::F8 { e4m3: true } => "f8 e4m3".into(),
            Ty::F8 { e4m3: false } => "f8 e5m2".into(),
            Ty::F32(en) => format!("f32 {}", e(*en)),
            Ty::F64(en) => format!("f64 {}", e(*en)),
            Ty::F80(en) => format!("f80 {}", e(*en)),
            Ty::Fixed { bits, frac, endian, signed } => {
                format!("{}{}.{frac} {}", if *signed { "i" } else { "u" }, bits - frac, e(*endian))
            }
            Ty::Vlq => "vlq".into(),
            Ty::Computed(_) => "computed".into(),
            Ty::SqliteVarint => "varint".into(),
            Ty::Leb128 { signed: false } => "leb128".into(),
            Ty::Leb128 { signed: true } => "sleb128".into(),
            Ty::Magic(b) => format!("magic[{}]", b.len()),
            Ty::Bytes(_) => "bytes[]".into(),
            Ty::Str { len, enc } => {
                let e = enc.short();
                match len {
                    StrLen::Fixed(_) => format!("{e}[]"),
                    StrLen::Padded { pad: 0, .. } => format!("{e} nul-pad"),
                    StrLen::Padded { pad, .. } => format!("{e} pad 0x{pad:02x}"),
                    StrLen::Terminated { end: 0, .. } if matches!(enc, Encoding::Utf8) => "cstr".into(),
                    StrLen::Terminated { end: 0, .. } => format!("{e} cstr"),
                    StrLen::Terminated { end, .. } => format!("{e} to 0x{end:02x}"),
                    StrLen::Scan { .. } => format!("{e} token"),
                }
            }
            Ty::Struct(s) => s.name.clone(),
            Ty::Array { elem, .. } => format!("{}[]", elem.display_name()),
            Ty::Repeat { elem, .. } => format!("{}[]", elem.display_name()),
            // Not `{elem}[]`: that promises children laid out end to end,
            // and these are placed one per offset, in any order.
            Ty::PointerList { elem, .. } => format!("offsets \u{2192} {}", elem.display_name()),
            Ty::At { inner, .. } => format!("at \u{2192} {}", inner.display_name()),
            Ty::Sized { inner, .. } => inner.display_name(),
            Ty::Switch { .. } => "switch".into(),
            Ty::Match { .. } => "switch".into(),
            Ty::Json(shape) => shape.name().to_string(),
            Ty::Enum { def, .. } => def.name.clone(),
            Ty::Flags { def, .. } => def.name.clone(),
            Ty::Named(n) => n.to_string(),
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
