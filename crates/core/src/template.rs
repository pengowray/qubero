//! Template IR: a description of the structure a file is expected to have.
//!
//! Nothing here is a static layout. Lengths, counts and choices are expressions
//! over earlier fields, so a template can say "an array of u32 whose count is
//! the field named `n`" or "bytes whose length is `size`, parsed as the section
//! type selected by `id`". Evaluation lives in `eval.rs`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::json;

/// Which end of the field the low bits come from.
///
/// For a field of whole bytes on a byte boundary this is byte order and
/// nothing else, which is all it used to mean. A field narrower than a byte,
/// or one that starts partway through a byte, has no bytes to order, and the
/// same question there is which end of the byte its bits are taken from. So
/// `Big` is MSB-first, the packing this IR always had, and `Little` is
/// LSB-first: the field sits at the bottom of the byte and the fields after it
/// stack upwards. A DEFLATE block header and a Zig packed struct are written
/// that way, and before this `endian` on such a field was read and ignored.
///
/// The two answers are the same answer at whole-byte widths, since taking a
/// number's bits from the bottom of each byte in turn is little-endian. It is
/// only the fields with a byte boundary inside them that the two orders
/// disagree about, and see [`crate::decode::lsb_offset`] for what happens
/// there.
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
    /// Size in bits of an earlier field, which is not the same question once a
    /// format stops counting in bytes. A JPEG XR image plane header is a run
    /// of fields a bit and three bits and four bits wide, some of them there
    /// only because an earlier one said so, and it ends by rounding up to the
    /// next byte so that what follows starts on one. Nothing can say how wide
    /// that rounding is without knowing how many bits the run came to, and
    /// `SizeOf` rounds the answer off before it can be asked.
    BitsOf(Arc<str>),
    /// This element's index in the nearest list it sits in. Zero outside one.
    Idx,
    /// The value of one element of an earlier array, by index. `Ref` names a
    /// field; this reaches inside one, which is what a list of pointers or a
    /// list of column types needs. When the elements are structures, `field`
    /// walks named fields down to the number: `tensors[i].offset` is
    /// `Elem { array: "tensors", field: ["offset"] }`. Empty when the elements
    /// are the numbers themselves.
    Elem { array: Arc<str>, index: Box<Expr>, field: Arc<[String]> },
    /// The same, for a list that is not a sibling of the field asking but sits
    /// inside one.
    ///
    /// `Elem` names a list by a single name, which reaches only a field
    /// declared beside this one. A format that keeps its description of itself
    /// in a header keeps the list in there: an NPY writes its structured dtype
    /// as a list of `('name', 'format')` inside its header dict, and the
    /// numbers that are typed by it are the header's sibling, not the list's.
    /// So `path` is a path down into an earlier field, the way [`Expr::Within`]
    /// is, and then `index` and `field` go on from the list it lands on.
    ///
    /// A field whose contents are somewhere else in the file is stepped
    /// through on the way, the same as a bare name is: an NPY's record view is
    /// declared as reading the header's own bytes over again, and naming it
    /// means the list, not the nothing standing in its place.
    ElemWithin { path: Arc<[String]>, index: Box<Expr>, field: Arc<[String]> },
    /// The value at `field` in the first element of the earlier list `array`
    /// whose `key` holds `tag`. `Elem` reaches an element by where it is,
    /// which is no use when what an element is, is written in it: a ZIP local
    /// header keeps an entry's real 64-bit size in an extra field tagged 1,
    /// in a list the writer may order as it likes, put its own records in,
    /// and leave out entirely.
    ///
    /// Zero when nothing in the list is tagged that way, or when the element
    /// found has no such field, so `Or` can name what to do without one.
    ///
    /// The label is a number or a run of bytes, since a format that keeps its
    /// records in text labels them in text: a FITS card is found by the
    /// keyword written in its first eight bytes. See [`Tag`].
    ///
    /// The label may also be worked out rather than written down, and the list
    /// may be the one this field's own element sits in. See [`TaggedRef`].
    ///
    /// Held behind a pointer because of what it costs the rest: a tag is a
    /// 128-bit number or a vector, either of which would make every expression
    /// in every template as wide and as strictly aligned as this one variant.
    /// Two formats in ninety use it.
    Tagged(Arc<TaggedRef>),
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
    /// The numbers of an earlier array field, added up. `ProductOf` is what a
    /// shape needs; this is what a table of counts needs. A JPEG Huffman
    /// segment writes how many codes there are of each of the sixteen
    /// lengths, and then that many symbols, without ever writing the total.
    SumOf(Arc<str>),
    /// The largest number in an earlier array. Tracker modules keep their
    /// pattern count implicitly as the greatest entry in the order table.
    MaxOf(Arc<str>),
    /// The next `bits` bits, read without consuming them. A field can then
    /// exist only when the byte at its own start says it does.
    ///
    /// A peek says which way round it reads, the same as a field does, and
    /// with `Little` on a peek narrower than a byte meaning the same thing it
    /// means on a field: the bits at the bottom of the byte rather than the
    /// top. See [`Endian`].
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
    /// How far it is from here to the next occurrence of `lead` that is not
    /// followed by one of `unless`, in bytes, or to the end of the container
    /// when there is none.
    ///
    /// This is what a stream of compressed data with no length on it needs.
    /// A JPEG writes its entropy-coded bits with no count anywhere: they run
    /// until the next marker, and a marker is 0xff and a byte that is not
    /// zero. Zero is how the encoder writes an 0xff that is data rather than a
    /// marker, so the escape and the terminator are the same byte and only the
    /// one after it tells them apart. `unless` is that set.
    ///
    /// `lead` is a sequence rather than a byte, because the thing that ends a
    /// stream is not always one. An H.264 Annex B start code is `00 00 01`,
    /// and the byte after it is the NAL header rather than an escape, so such
    /// a marker leaves `unless` empty. An empty `unless` is also what says
    /// that nothing has to follow the lead for it to count: with a set to
    /// check against, a lead at the very end of the container is a lead
    /// nothing has told apart from an escape and so is not a marker, and
    /// without one there is nothing to tell apart.
    ///
    /// The distance stops before the lead, so the marker belongs to whatever
    /// is declared next rather than to the stream. A container that ends
    /// without one, which is a file cut off in the middle, measures to its end
    /// rather than failing: the bytes are still there to look at, and refusing
    /// to place them would hide the very thing that went wrong.
    ToMarker { lead: Vec<u8>, unless: Vec<u8> },
    /// How far it is from here to where `needle` is written next, in bytes, or
    /// to the end of the container when it is not written again. With `last`,
    /// to the last place it is written rather than the first.
    ///
    /// `ToMarker` measures to a single byte, which is what a stream ending at
    /// a marker needs. A format that writes its structure in words needs more
    /// than one byte to look for: a PDF object ends at `endobj`, and the
    /// pointer to the table that places every one of them is written after the
    /// word `startxref`, at the end of a file that may have several of them
    /// and means the last.
    ///
    /// The distance stops before the needle, so the word belongs to whatever
    /// is declared next rather than to the run before it.
    Find { needle: Vec<u8>, last: bool },
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
    /// The value at `field`, a path that starts at a field declared before
    /// this one in the same structure (or in one it sits inside) and then goes
    /// down into it. `Ref` names a field beside this one and stops there,
    /// which is no use when what has to be read is one level in: an HDF5
    /// attribute writes its own datatype inside itself, and how wide one of
    /// its elements is, is a field of that.
    Within(Arc<[String]>),
    /// The first of the two that is not zero. Pairs with `Prev` to say "this
    /// one, or the last one that had one".
    Or(Box<Expr>, Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    /// One when the first is less than the second, and zero otherwise.
    ///
    /// There is no `if` here and this is not one. It is a number like any
    /// other, and what makes it useful is `Or`, which takes its right side
    /// only when its left side is zero: `a.less_than(b).or(c)` answers `c`
    /// while `a` is the larger, and stops at 1 once `a` is smaller. That is
    /// how a template asks a question it cannot afford to ask twice, such as
    /// looking ahead twenty bytes when fewer than twenty are left.
    ///
    /// Check which way round it is written. A guard of that shape says "if
    /// there is not room, the answer is 1 and do not ask `c`", and swapping
    /// the two sides makes it say the opposite without failing: `c` is then
    /// read in exactly the cases it was put there to avoid, and what comes
    /// back is whatever was at an offset nothing checked.
    Less(Box<Expr>, Box<Expr>),
    /// This shifted left by that many bits.
    ///
    /// A format that stores a shift count rather than a size needs it: a
    /// 16-bit Windows executable writes where a segment starts in units of
    /// however many bytes the header's alignment count says, so the offset in
    /// the table is only an offset once it has been shifted.
    Shl(Box<Expr>, Box<Expr>),
    /// This shifted right by that many bits, with no sign extension: the bits
    /// shifted off the bottom are gone and nothing is brought in at the top.
    ///
    /// The other half of reaching a field packed inside a number. `Bit` reads
    /// one bit and `Shl` puts it back where it belongs, which is how a run of
    /// packed differences was written before this existed: a five-bit field
    /// cost five `Bit`s, five `Shl`s and four `Add`s, and the expression a
    /// reader was shown was thirty terms long for a fact as simple as "bits 24
    /// to 20". Shifting the whole number down and masking it says that in one
    /// line. See [`Expr::bit_field`].
    Shr(Box<Expr>, Box<Expr>),
    /// The two numbers' bits, anded together. What a mask is.
    ///
    /// Negative numbers are anded as the two's complement they are, so a mask
    /// of -1 leaves a number alone. Nothing in the IR writes one, but a field
    /// read as signed can hold one, and quietly answering something else would
    /// be worse than answering the arithmetic.
    And(Box<Expr>, Box<Expr>),
    /// The smaller of the two, and the larger of the two.
    ///
    /// What a length that must not run past the end of the file needs.
    /// `Min(claimed, Remaining)` is the file cut off mid-transmission: the
    /// last space packet of a capture says how long it is and the recording
    /// stopped eight bytes short, and an MS-DOS header counts pages a library
    /// never wrote. Reading what is there is the answer in both cases, and
    /// refusing to place it would hide the very thing that went wrong.
    ///
    /// Written by hand this is a comparison multiplied by each side and added
    /// back together, which three templates did before this existed. `Max`
    /// is the other end of the same clamp: a program's entry point is inside
    /// the program or the program is not what it said it was.
    Min(Box<Expr>, Box<Expr>),
    Max(Box<Expr>, Box<Expr>),
    /// How many bytes of padding follow a run of `n` bytes, to bring what
    /// comes after it back onto a boundary of `align`.
    ///
    /// Every format that aligns anything needs this, and there is no
    /// remainder operator to write it with: a device tree pads a node's name
    /// to four bytes, a cpio archive pads its names and its files to four,
    /// and a systemd journal pads every object to eight. Written by hand it
    /// is `n` less the whole fours in it, four less that, and the same
    /// subtraction again to take back off the four that a run already ending
    /// on a boundary would otherwise be padded by. Three formats wrote that
    /// out before it became this.
    PadTo { n: Box<Expr>, align: u32 },
    /// One bit of a number, as one or zero.
    ///
    /// What a switch needs to key on a flag. A section of a program says it
    /// holds code by setting one bit of a word whose other bits say whether
    /// it can be written to and how it is aligned, and matching the whole word
    /// would mean listing every combination a linker happens to write.
    Bit(Box<Expr>, u32),
}

/// Which element of a labelled list to read, and what to read from it. The
/// body of [`Expr::Tagged`], kept out of the enum so that its 128-bit tag does
/// not set the width of every expression there is.
#[derive(Debug, Clone)]
pub struct TaggedRef {
    /// The list to look in, by the name of an earlier field. `None` means the
    /// list this field's own element sits in, and only the elements before it.
    ///
    /// A named list is the ordinary case: a ZIP entry's extra fields are a
    /// list declared beside the field asking. `None` is for the format that
    /// writes its records and the records that describe them in one run: a GWF
    /// file is a stream of structures, and what kind each one is, is written in
    /// an `FrSH` structure earlier in the same stream. There is no field to
    /// name there, because the list is the one the asking element is in, and a
    /// name reaches only fields declared *before* the field asking.
    ///
    /// Earlier elements only, and that is not a limitation to work around: a
    /// search over later ones would have to place them, and placing them is
    /// what is asking. Every format that writes a table of definitions writes
    /// it before what it defines, for the same reason.
    pub array: Option<Arc<str>>,
    /// The path to the field of an element that holds its label.
    pub key: Arc<[String]>,
    /// The label to look for.
    pub tag: Tag,
    /// The path to the field to read, in the element that has it.
    pub field: Arc<[String]>,
}

/// What an element of a labelled list is labelled with. Most formats number
/// their records, and a number is what `Tag::Int` matches. A format that keeps
/// its records in text does not: a FITS header is eighty-column lines whose
/// first eight bytes are the keyword, and `NAXIS1` is a label written out
/// rather than a value counted up to. Reading those eight bytes as a number
/// would answer with 0x4e41584953312020, which is a number no file wrote and
/// nobody could check the template against.
#[derive(Debug, Clone)]
pub enum Tag {
    Int(i128),
    /// The raw bytes of the key field, compared as they are written. A key of
    /// a fixed width is padded, and the padding is part of what is compared,
    /// the same as it is for [`Until::FieldBytes`].
    Bytes(Vec<u8>),
    /// A label worked out where the question is asked, rather than one written
    /// into the template. `Int` finds the record a format fixed the number of;
    /// this finds the record *this* record points at, which is what a format
    /// that numbers its own types needs: a GWF structure carries a class byte,
    /// and which class that is, is written in an earlier structure whose class
    /// number is that byte.
    ///
    /// Worked out once, before the search, and compared as a number. A key
    /// written in text is matched by [`Tag::ComputedText`].
    Computed(Arc<Expr>),
    /// The same, for a list labelled in text: the label is whatever text the
    /// expression reaches, rather than bytes written into the template.
    ///
    /// A format that keeps its records in text and points at them from
    /// elsewhere in text needs it. `Bytes` finds the card a template names;
    /// this finds the card *this* record names, which is what a header saying
    /// `EXTNAME = 'SCI'` and a table elsewhere saying which extension it
    /// belongs to is doing.
    ///
    /// Compared as text and not as bytes, which is the difference that makes
    /// it work at all: a key field of a fixed width is padded, and the padding
    /// is part of what `Bytes` compares but no part of what the field reads
    /// as. So both sides are read through the same text primitive and the
    /// answers compared, and a template does not have to know how wide the key
    /// field of the format it is searching happens to be.
    ComputedText(Arc<Expr>),
    /// A label already worked out, once a search is under way. See `tag_now`.
    Text(String),
}

impl Tag {
    /// How a tag reads in a sentence about a connection: a number as itself, a
    /// key written in text as that text, with padding trimmed and anything
    /// that is not printable shown as an escape, and a computed label as the
    /// expression that works it out.
    ///
    /// Nothing when the expression has no reading, so that a connection is
    /// written whole or not at all. See `eval::relate`.
    pub fn written(&self) -> Option<String> {
        Some(match self {
            Tag::Int(v) => v.to_string(),
            Tag::Bytes(b) => format!("{:?}", String::from_utf8_lossy(b).trim_end()),
            Tag::Computed(e) | Tag::ComputedText(e) => crate::eval::write_expr(e)?,
            Tag::Text(s) => format!("{s:?}"),
        })
    }
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
    /// The bit size of an earlier field, for a format that packs them tighter
    /// than a byte.
    pub fn bits_of(name: &str) -> Expr {
        Expr::BitsOf(name.into())
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
    /// Element `index` of a list reached by a path down into an earlier field,
    /// and `field` inside that element. See [`Expr::ElemWithin`].
    pub fn elem_within(path: &[&str], index: Expr, field: &[&str]) -> Expr {
        Expr::ElemWithin {
            path: path.iter().map(|s| s.to_string()).collect(),
            index: Box::new(index),
            field: field.iter().map(|s| s.to_string()).collect(),
        }
    }
    /// Field `field` of the first element of `array` whose `key` holds `tag`.
    pub fn tagged(array: &str, key: &[&str], tag: i128, field: &[&str]) -> Expr {
        Expr::tagged_by(Some(array), key, Tag::Int(tag), field)
    }
    /// The same, for a list whose elements are labelled in text: `field` of the
    /// first element of `array` whose `key` holds exactly these bytes.
    pub fn tagged_bytes(array: &str, key: &[&str], tag: &[u8], field: &[&str]) -> Expr {
        Expr::tagged_by(Some(array), key, Tag::Bytes(tag.to_vec()), field)
    }
    /// The same, for a label this record works out rather than one the format
    /// fixed: `field` of the first element of `array` whose `key` holds
    /// whatever `tag` comes to here. See [`Tag::Computed`].
    pub fn tagged_by_expr(array: &str, key: &[&str], tag: Expr, field: &[&str]) -> Expr {
        Expr::tagged_by(Some(array), key, Tag::Computed(Arc::new(tag)), field)
    }
    /// The same again, searching the list this element is in rather than a
    /// list named beside it, and only the elements before this one: `field` of
    /// the nearest earlier element whose `key` holds whatever `tag` comes to.
    ///
    /// This is the one that reaches a sibling inside the same `Repeat`, which
    /// no name can: a name reaches fields declared before the field asking,
    /// and the elements of a list are not fields. A GWF structure finds the
    /// `FrSH` that named its class this way. See [`TaggedRef::array`].
    pub fn sibling_tagged(key: &[&str], tag: Expr, field: &[&str]) -> Expr {
        Expr::tagged_by(None, key, Tag::Computed(Arc::new(tag)), field)
    }
    /// The same for a list labelled in text, where the label is text found
    /// somewhere else in the file rather than bytes the template fixed:
    /// `field` of the first element of `array` whose `key` reads as whatever
    /// text `tag` reaches. See [`Tag::ComputedText`].
    pub fn tagged_by_text(array: &str, key: &[&str], tag: Expr, field: &[&str]) -> Expr {
        Expr::tagged_by(Some(array), key, Tag::ComputedText(Arc::new(tag)), field)
    }
    /// The same again over the elements before this one, for a stream that
    /// labels its own records in text.
    pub fn sibling_tagged_text(key: &[&str], tag: Expr, field: &[&str]) -> Expr {
        Expr::tagged_by(None, key, Tag::ComputedText(Arc::new(tag)), field)
    }
    fn tagged_by(array: Option<&str>, key: &[&str], tag: Tag, field: &[&str]) -> Expr {
        Expr::Tagged(Arc::new(TaggedRef {
            array: array.map(Arc::from),
            key: key.iter().map(|s| s.to_string()).collect(),
            tag,
            field: field.iter().map(|s| s.to_string()).collect(),
        }))
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
    /// The numbers of the earlier array field `name`, added up.
    pub fn sum_of(name: &str) -> Expr {
        Expr::SumOf(name.into())
    }
    /// The largest number in the earlier array field `name`, or zero when it
    /// is empty.
    pub fn max_of(name: &str) -> Expr {
        Expr::MaxOf(name.into())
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
    /// The bytes from here to the next `lead` byte not followed by one of
    /// `unless`, or to the end of the container if there is none.
    pub fn to_marker(lead: u8, unless: &[u8]) -> Expr {
        Expr::ToMarker { lead: vec![lead], unless: unless.to_vec() }
    }
    /// The same, for a marker that is more than one byte: an H.264 Annex B
    /// start code is `00 00 01`. See [`Expr::ToMarker`].
    pub fn to_marker_seq(lead: &[u8], unless: &[u8]) -> Expr {
        Expr::ToMarker { lead: lead.to_vec(), unless: unless.to_vec() }
    }
    /// The bytes from here to the next place `needle` is written, or to the
    /// end of the container when there is none.
    pub fn to_bytes(needle: &[u8]) -> Expr {
        Expr::Find { needle: needle.to_vec(), last: false }
    }
    /// The same, to the last place it is written rather than the first.
    pub fn to_last_bytes(needle: &[u8]) -> Expr {
        Expr::Find { needle: needle.to_vec(), last: true }
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
    /// A field declared before this one, and a path down into it, e.g.
    /// `within(&["datatype", "size"])`.
    pub fn within(field: &[&str]) -> Expr {
        Expr::Within(field.iter().map(|s| s.to_string()).collect())
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
    /// One when this is less than `rhs`, and zero otherwise.
    /// This shifted left by `rhs` bits.
    pub fn shl(self, rhs: Expr) -> Expr {
        Expr::Shl(Box::new(self), Box::new(rhs))
    }
    /// This shifted right by `rhs` bits, bringing in nothing at the top.
    pub fn shr(self, rhs: Expr) -> Expr {
        Expr::Shr(Box::new(self), Box::new(rhs))
    }
    /// This and `rhs`, bit by bit.
    pub fn and(self, rhs: Expr) -> Expr {
        Expr::And(Box::new(self), Box::new(rhs))
    }
    /// A run of `width` bits of `src`, the topmost of them bit `top_bit`,
    /// counting from the least significant, read as an unsigned number.
    ///
    /// This is how a format that packs several numbers into one word is read.
    /// A Steim2 word holds five six-bit differences, and the second of them is
    /// `bit_field(word, 23, 6)`: shift the run down to the bottom and keep the
    /// bits of it. `top_bit` rather than a bottom bit, because that is the end
    /// the specifications count from and the end a reader checks against the
    /// hex.
    ///
    /// A width of zero is no bits and comes to nothing.
    pub fn bit_field(src: Expr, top_bit: u32, width: u32) -> Expr {
        if width == 0 {
            return Expr::lit(0);
        }
        let low = i128::from(top_bit) - i128::from(width) + 1;
        let mask = (1i128 << width) - 1;
        src.shr(Expr::Lit(low.max(0))).and(Expr::Lit(mask))
    }
    /// The same run of bits read as two's complement: the unsigned value less
    /// twice the weight of its top bit, which is what a sign bit is worth.
    pub fn signed_bit_field(src: Expr, top_bit: u32, width: u32) -> Expr {
        if width == 0 {
            return Expr::lit(0);
        }
        let sign = Expr::bit_field(src.clone(), top_bit, 1).shl(Expr::lit(i128::from(width)));
        Expr::bit_field(src, top_bit, width).sub(sign)
    }
    /// Bit `n` of this, counting from the least significant.
    pub fn bit(self, n: u32) -> Expr {
        Expr::Bit(Box::new(self), n)
    }
    pub fn less_than(self, rhs: Expr) -> Expr {
        Expr::Less(Box::new(self), Box::new(rhs))
    }
    /// This, or `rhs` when `rhs` is the smaller: what a length that must not
    /// run past the end of its container is written as.
    pub fn at_most(self, rhs: Expr) -> Expr {
        Expr::Min(Box::new(self), Box::new(rhs))
    }
    /// This, or `rhs` when `rhs` is the larger.
    pub fn at_least(self, rhs: Expr) -> Expr {
        Expr::Max(Box::new(self), Box::new(rhs))
    }
    /// The padding after a run of `self` bytes, to the next multiple of
    /// `align`. Nothing at all when the run already ended on one.
    pub fn pad_to(self, align: u32) -> Expr {
        Expr::PadTo { n: Box::new(self), align }
    }
}

#[derive(Debug, Clone)]
pub enum Until {
    /// Repeat until the enclosing size limit (or end of file) is reached.
    End,
    /// Repeat until an element whose field `field` has the given raw bytes
    /// (that element is included).
    FieldBytes { field: String, bytes: Vec<u8> },
    /// Repeat until an element whose field `field` reads as `value` (that
    /// element is included).
    ///
    /// `FieldBytes` compares what is written, which is what a format ending
    /// its list with a known byte string needs and what a field of no bytes
    /// cannot answer. A zstd block says it is the last one in the low bit of a
    /// three-byte header packed from that bit up, so the flag is worked out
    /// from the header rather than read, and there are no bytes of its own to
    /// compare.
    FieldValue { field: String, value: i128 },
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
    /// Names for whole runs of values, where a format stops naming them one at
    /// a time and starts counting. Tried after `cases`, in order.
    pub spans: Vec<EnumSpan>,
    /// Show the number in hex. True for sets people read in hex, such as wasm
    /// opcodes and value types.
    pub hex: bool,
}

/// A run of values that mean the same thing and differ only by a number: every
/// even value from 12 up is a SQLite blob, and how far up it is says how many
/// bytes long. `label` is written with `{n}` where that number goes.
#[derive(Debug)]
pub struct EnumSpan {
    pub from: i128,
    /// How far apart the values of the run are. One for a solid run, two for
    /// every other value, which is how a format fits two runs in one range.
    pub step: i128,
    pub label: String,
}

impl EnumSpan {
    /// How far into the run `v` is, or nothing if it is not in it.
    pub fn count(&self, v: i128) -> Option<i128> {
        if v < self.from || self.step <= 0 { return None; }
        let d = v - self.from;
        (d % self.step == 0).then_some(d / self.step)
    }
    pub fn label(&self, v: i128) -> Option<String> {
        self.count(v).map(|n| self.label.replace("{n}", &n.to_string()))
    }
}

impl EnumDef {
    pub fn label(&self, v: i128) -> Option<&str> {
        self.cases.iter().find(|(k, _)| *k == v).map(|(_, n)| n.as_str())
    }
    /// The name a value goes by, whether it is one of the named ones or one of
    /// a counted run.
    pub fn name_of(&self, v: i128) -> Option<String> {
        self.label(v)
            .map(str::to_string)
            .or_else(|| self.spans.iter().find_map(|s| s.label(v)))
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

impl StrLen {
    /// A token: any run of `skip` before it is stepped over, and the first
    /// `ends` byte after it belongs to the field. What every format that
    /// writes its values with white space between them needs.
    pub fn token(skip: &[u8], ends: &[u8]) -> StrLen {
        StrLen::Scan { skip: skip.to_vec(), ends: ends.to_vec(), comment: None }
    }
    /// The same, where a run from the first byte of `comment` to the second
    /// counts as separator too.
    pub fn token_past_comments(skip: &[u8], ends: &[u8], comment: (u8, u8)) -> StrLen {
        StrLen::Scan { skip: skip.to_vec(), ends: ends.to_vec(), comment: Some(comment) }
    }
}

/// How many elements of a [`Ty::Chain`] are followed before the walk gives up.
///
/// Far past any real chain: the largest thing anyone keeps in one is a CDF's
/// variable records, and a file with a million of those is a file nobody wrote.
/// A chain longer than this is a file whose pointers are wrong in a way that
/// happens not to close a ring, and answering with the million elements that
/// were read beats not answering.
pub const CHAIN_CAP: usize = 1_000_000;

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
    /// Where this field's *displayed* name is written in the file, when the
    /// format writes it somewhere rather than fixing it.
    ///
    /// A structure's field names are settled when the template is built, and a
    /// format that names its own fields does not work that way: a FITS table's
    /// third column is called whatever the `TTYPE3` card says, and a template
    /// can only call it `col3`. [`StructDef::named_by`] answers the same
    /// question for a whole structure, by naming a field of it; this answers
    /// it for one field, by an expression that reaches text anywhere the file
    /// keeps it, which for FITS is a card found by keyword.
    ///
    /// The declared name stays the name: every expression, every path and
    /// every write still says `col3`, because a name read out of the file can
    /// change when the file is edited and a path that moves is no path at all.
    /// This decides one thing, what the row is labelled, and the label is the
    /// declared name and then the text: `col3 flux`.
    ///
    /// Nothing when the text cannot be read or comes to nothing, which leaves
    /// the field with the name it had.
    pub name_from: Option<Expr>,
}

#[derive(Debug, Clone)]
pub enum Ty {
    UInt { bits: u32, endian: Endian },
    Int { bits: u32, endian: Endian },
    /// Sign and magnitude: the top bit says which way the number goes, and the
    /// bits below it are how far, read as an ordinary unsigned number. Not
    /// two's complement, where -1 is every bit set; here -1 is the top bit and
    /// a one.
    ///
    /// A GRIB message writes its latitudes, its scale factors and its grid
    /// increments this way, which is what most formats older than the hardware
    /// that settled on two's complement do. Reading one as an `Int` turns a
    /// southern latitude into a number near the bottom of the range, which is
    /// a plausible-looking answer and wrong by the whole width of the field.
    ///
    /// The sign bit set with a magnitude of zero is negative zero, which the
    /// value reads as 0: it is the same number, and a format that writes it
    /// means the same number. `bits` counts the sign bit, so an eight-bit
    /// field holds -127 to 127.
    SignMagnitude { bits: u32, endian: Endian },
    /// An unsigned integer as wide as an earlier field says, rather than as
    /// wide as the template says.
    ///
    /// What a format that packs its numbers to fit needs. A GRIB section 7 is
    /// the values of a grid at whatever width section 5 said would hold them,
    /// which is 11 bits or 16 or 12 and is a property of the message, not of
    /// the format. Written as a `Switch` over every width it could be, that is
    /// sixty-four cases of the same type; written as bytes, the grid is one
    /// long run with no values in it at all.
    ///
    /// The width is in bits and is read where the field is placed, so it may
    /// name any field an expression can reach. Zero is allowed and is a field
    /// of no bits: a GRIB whose values are all the same writes a width of
    /// zero and no data. A run of them is still counted and sized by
    /// arithmetic when the width does not depend on the element (see
    /// `eval::size::uniform`), so a grid of a million values is placed without
    /// walking it.
    ///
    /// `Little` is refused for a width that is not whole bytes, or at an
    /// offset that is not on one: a field packed from the bottom of its byte
    /// has to be placed before it is read, and how wide it is, is not known
    /// until it is. Every format that writes these packs them MSB-first.
    UIntExpr { bits: Box<Expr>, endian: Endian },
    /// A number with a value that means "nobody filled this in".
    ///
    /// Every slot of a SAC header exists in every file, and one nothing was
    /// written into holds -12345, or -12345.0 for a float. Shown as the number
    /// it is, that is a magnitude of twelve thousand kilometres and a time in
    /// 1962, and the reader has to know the convention to know it is neither.
    ///
    /// The type column still says what the field is, because the field is
    /// still an `i32`: this only decides how the value reads. Expressions see
    /// the number underneath, so a count that is unset still clamps and
    /// compares as the number the file holds rather than becoming an absence
    /// halfway through the arithmetic.
    Nullable { inner: Box<Ty>, unset: Unset },
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
    /// The same, for a value that is text: a field of no bits whose value is
    /// text found somewhere else in the file.
    ///
    /// `Computed` covers everything the arithmetic can answer, and a name is
    /// not one of those. A GWF structure carries a class number and what that
    /// class is called is written in an `FrSH` structure further back; the
    /// number is in the file and the word is in the file, and before this the
    /// reader was shown only the number and left to go and look the word up.
    /// A row beside it saying `trce` is the whole of what they were after.
    ///
    /// Zero bits, so it covers none of the file and moves nothing along. It is
    /// a reading of what is already there, not a claim that a byte exists.
    ComputedText(Expr),
    /// Unsigned LEB128 (as used by wasm). Signed variant reads sign-extended.
    Leb128 { signed: bool },
    /// EBML's big-endian variable-size integer. The first set bit says how
    /// many bytes the field occupies. Element IDs keep that marker as part of
    /// their value; element sizes remove it.
    EbmlVint { strip_marker: bool },
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
    /// A number written as digits: the text is the value. `StrLen` says how
    /// far it runs, the same as text does, and `radix` says what the digits
    /// are worth.
    ///
    /// Reading it as text and letting an expression have the bytes would not
    /// do. A field used as a number is its bytes as one, so `408` would come
    /// out as 0x343038, which is 3,355,192 and points nowhere near the table
    /// a PDF asked for. The parse has to happen where the field is read, so
    /// that what a pointer list is handed is the number the file wrote.
    ///
    /// Leading zeros are how a format keeps such a field a fixed width, so
    /// they are read and not complained about. A run of digits with anything
    /// else in it is an error, since a number half read is worse than none.
    TextInt { len: StrLen, radix: u32 },
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
    /// A flat list of elements found by following pointers, one to the next.
    ///
    /// `PointerList` places its children at offsets read from a table written
    /// before them, which is what a format with a directory does. A format
    /// with no directory writes the offset of the next record inside the
    /// record before it, and there is no table anywhere: a CDF's attributes
    /// are a chain, its variables are a chain, and each attribute's entries are
    /// a chain of their own. Written as nesting, the only shape that could hold
    /// it before, a file with two hundred attributes is a tree two hundred
    /// deep, and the two hundredth is behind two hundred rows a reader has to
    /// open one at a time. It is a list, and this is what says so.
    ///
    /// `first` is where the first element is, in bytes from `anchor`; `next`
    /// is the path to the field of an element that holds where the one after
    /// it is, in the same terms. Each element is placed at its own offset, the
    /// way an `At` is, so the children are scattered and need not be in order.
    ///
    /// A path rather than a name, for the same reason [`Expr::Elem`] takes
    /// one: a format that wraps every record in a size and a type keeps the
    /// forward pointer inside the wrapper, and a CDF's is `body.adr_next`.
    ///
    /// **Where it stops.** A chain is written by a program that could crash
    /// halfway, and a file that has been truncated or corrupted must not take
    /// the reader with it. So the walk ends at whichever of these comes first:
    ///
    /// - an offset of zero, which is how every format that has one of these
    ///   says "no more". A chain whose first element is genuinely at byte zero
    ///   of the file cannot be written down here, and no format writes one:
    ///   byte zero is where the magic is.
    /// - an offset of all ones, for the width of the `next` field. That is the
    ///   other way a format says it, and a `u32` of 0xffffffff is not an
    ///   offset four gigabytes into a file that is not four gigabytes long.
    /// - an offset already visited, which is a ring. Following one is not slow
    ///   but endless.
    /// - an offset at or past the end of the file.
    /// - [`CHAIN_CAP`] elements, so a file that is neither a ring nor finished
    ///   still answers.
    ///
    /// The list itself covers no bytes where it is declared, like an `At`: what
    /// covers bytes is its elements, wherever they turned out to be.
    Chain { first: Expr, next: Arc<[String]>, elem: Box<Ty>, anchor: Anchor },
    /// SQLite's variable-length integer: seven bits per byte, most significant
    /// group first, up to nine bytes, where a ninth byte contributes all eight
    /// of its bits. `Vlq` stops at four bytes and never does that, so it
    /// cannot stand in.
    SqliteVarint,
    /// A field of no bits whose contents are read somewhere else: `inner`,
    /// placed at `at` bytes from `anchor`, with nothing of it where the field
    /// itself is declared.
    ///
    /// This is what a header pointing at a table needs. A `PointerList` places
    /// children at offsets read from an earlier field, so the offsets have to
    /// be read before the bytes they point at; a WAD keeps its directory at
    /// the end of the file and says in the header where it is. Reading the
    /// directory here, at no cost in bytes, puts the offsets in hand while the
    /// cursor is still at the front, and the list of lumps can then be
    /// declared over the region they sit in.
    ///
    /// `anchor` is what the offset is counted from, the same three choices a
    /// `PointerList` has. A WAD counts from the start of the file and means
    /// it. A TIFF counts from the start of the TIFF, which is the same place
    /// until the TIFF is inside something else: the EXIF block of a JPEG is a
    /// whole TIFF file written into a segment partway through, and every
    /// offset in it counts from where that copy begins. `Anchor::Window` is
    /// the nearest `Sized` around the field, and falls back to the start of
    /// the file when there is none, so the one layout serves both.
    ///
    /// The field advances nothing, so what follows it is placed as if it were
    /// not there. Its one child is the only thing that covers bytes.
    ///
    /// A structure is still as long as its last field ends, and what this
    /// points at may be past that. The cursor lands in it anyway: every
    /// stretch an `At` puts somewhere is indexed as it is walked, and `locate`
    /// asks that index for a bit outside what the root covers. Without it an
    /// HDF5 file would answer for its first ninety-six bytes and for nothing
    /// else, since every object in one is reached by address. See
    /// `eval::placed`.
    At { anchor: Anchor, at: Expr, inner: Box<Ty> },
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
    /// One machine instruction, as long as the machine says it is. What that
    /// length is, is only known by decoding: on x86 it is anywhere from one
    /// byte to fifteen, and even on a machine whose instructions are all four
    /// bytes it takes the decoder to say whether these four are one at all.
    /// See [`crate::code`].
    Insn { isa: crate::code::Isa },
    /// A type from the template's table, by name. This is what makes a format
    /// whose boxes contain boxes expressible: the type refers to itself.
    Named(Arc<str>),
}

/// The value a format writes to mean a slot nobody filled in. See
/// [`Ty::Nullable`].
#[derive(Debug, Clone, PartialEq)]
pub enum Unset {
    Int(i128),
    /// Compared exactly, which is what a sentinel needs and what makes it a
    /// sentinel: -12345.0 is a number a float holds exactly, and a file either
    /// wrote those thirty-two bits or wrote a measurement.
    Float(f64),
}

impl Unset {
    /// Whether this is the value that means nothing was filled in.
    pub fn matches(&self, v: &crate::eval::Value) -> bool {
        match self {
            Unset::Int(want) => v.as_int() == Some(*want),
            Unset::Float(want) => matches!(v, crate::eval::Value::Float(f) if f == want),
        }
    }
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
    /// Fields that are this structure's own machinery, whatever the shapes
    /// say. What a field decides is worked out from the template itself (see
    /// [`crate::machinery`]), and a field nothing reads still ends up here
    /// when it is plumbing all the same: a fragment counter, a free-block
    /// chain, a reserved word.
    pub machinery: Vec<Arc<str>>,
    /// Fields that are the point, whatever the shapes say. A bitmap's width
    /// settles the stride of every row and is also the first thing a reader
    /// wants to know, and folding it behind the pixels would hide it.
    pub payload: Vec<Arc<str>>,
}

impl Ty {
    /// One byte, which has no byte order. It is declared `Big` all the same,
    /// because a byte-wide field partway through a byte does have a bit order
    /// and MSB-first is the one every format that writes one of these means.
    /// See [`Endian`].
    pub fn u8() -> Ty {
        Ty::UInt { bits: 8, endian: Endian::Big }
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
    /// A number whose top bit is its sign. See [`Ty::SignMagnitude`].
    pub fn sign_magnitude(bits: u32, e: Endian) -> Ty {
        Ty::SignMagnitude { bits, endian: e }
    }
    /// An unsigned integer as many bits wide as `bits` comes to when the field
    /// is read. See [`Ty::UIntExpr`].
    pub fn uint_expr(bits: Expr, e: Endian) -> Ty {
        Ty::UIntExpr { bits: Box::new(bits), endian: e }
    }
    /// `inner`, reading as unset when it holds `sentinel`. See
    /// [`Ty::Nullable`].
    pub fn unset_int(inner: Ty, sentinel: i128) -> Ty {
        Ty::Nullable { inner: Box::new(inner), unset: Unset::Int(sentinel) }
    }
    /// The same for a float slot, compared exactly.
    pub fn unset_float(inner: Ty, sentinel: f64) -> Ty {
        Ty::Nullable { inner: Box::new(inner), unset: Unset::Float(sentinel) }
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
    pub fn ebml_id() -> Ty {
        Ty::EbmlVint { strip_marker: false }
    }
    pub fn ebml_size() -> Ty {
        Ty::EbmlVint { strip_marker: true }
    }
    /// A field of no bits whose value is an expression.
    pub fn computed(e: Expr) -> Ty {
        Ty::Computed(e)
    }
    /// A field of no bits whose value is text found elsewhere in the file.
    /// See [`Ty::ComputedText`].
    pub fn computed_text(e: Expr) -> Ty {
        Ty::ComputedText(e)
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
    /// A number written as decimal digits. See [`Ty::TextInt`].
    pub fn decimal(len: StrLen) -> Ty {
        Ty::TextInt { len, radix: 10 }
    }
    /// A number written as hexadecimal digits, which is how a cpio archive
    /// writes every number in its header. See [`Ty::TextInt`].
    pub fn hex_digits(len: StrLen) -> Ty {
        Ty::TextInt { len, radix: 16 }
    }
    /// A number written as octal digits, which is how an `ar` header writes a
    /// file's mode: a Unix mode is three digits of three bits and is read in no
    /// other base. See [`Ty::TextInt`].
    pub fn octal(len: StrLen) -> Ty {
        Ty::TextInt { len, radix: 8 }
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
            fields: fields.into_iter().map(|(n, ty)| Field { name: n.into(), ty, name_from: None }).collect(),
            named_by: None,
            contents: None,
            unit: None,
            inline: false,
            packed: None,
            machinery: Vec::new(),
            payload: Vec::new(),
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
    /// Say where in the file the field called `field` gets its displayed name
    /// from. The declared name is unchanged and stays the path name; the row
    /// reads as both. See [`Field::name_from`].
    pub fn field_named_from(self, field: &str, from: Expr) -> Ty {
        match self {
            Ty::Struct(s) => {
                let mut s = (*s).clone();
                if let Some(f) = s.fields.iter_mut().find(|f| &*f.name == field) {
                    f.name_from = Some(from);
                }
                Ty::Struct(Arc::new(s))
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

    /// Names fields that are this structure's machinery however its shapes
    /// read. See [`StructDef::machinery`].
    pub fn machinery(self, names: &[&str]) -> Ty {
        match self {
            Ty::Struct(s) => Ty::Struct(Arc::new(StructDef { machinery: names.iter().map(|n| Arc::from(*n)).collect(), ..(*s).clone() })),
            other => other,
        }
    }

    /// Names fields the machinery rules must leave at full strength. See
    /// [`StructDef::payload`].
    pub fn payload(self, names: &[&str]) -> Ty {
        match self {
            Ty::Struct(s) => Ty::Struct(Arc::new(StructDef { payload: names.iter().map(|n| Arc::from(*n)).collect(), ..(*s).clone() })),
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
    /// A list found by following pointers: the first element is `first` bytes
    /// from `anchor`, and each element's field `next` says where the one after
    /// it is. See [`Ty::Chain`] for where the walk stops.
    pub fn chain(first: Expr, next: &[&str], anchor: Anchor, elem: Ty) -> Ty {
        Ty::Chain {
            first,
            next: next.iter().map(|s| s.to_string()).collect(),
            elem: Box::new(elem),
            anchor,
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
    /// One instruction of `isa`, at wherever the field is placed.
    pub fn insn(isa: crate::code::Isa) -> Ty {
        Ty::Insn { isa }
    }
    pub fn sqlite_varint() -> Ty {
        Ty::SqliteVarint
    }
    /// `inner`, read at `at` bytes from the start of the file, in a field that
    /// takes up no room where it is declared. See [`Ty::At`].
    pub fn at(at: Expr, inner: Ty) -> Ty {
        Ty::At { anchor: Anchor::File, at, inner: Box::new(inner) }
    }
    /// The same, counted from the nearest `Sized` around the field rather than
    /// from the start of the file. What a format keeps working through when a
    /// copy of it is embedded in something else.
    pub fn at_in_window(at: Expr, inner: Ty) -> Ty {
        Ty::At { anchor: Anchor::Window, at, inner: Box::new(inner) }
    }
    pub fn sized(size: Expr, inner: Ty) -> Ty {
        Ty::Sized { size, inner: Box::new(inner) }
    }
    pub fn switch(on: Expr, cases: Vec<(i128, Ty)>, default: Ty) -> Ty {
        Ty::Switch { on, cases: cases.into(), default: Arc::new(default) }
    }
    /// A field that is there only while its container still has room for it.
    ///
    /// What a format that grew a field at a time needs. A systemd journal
    /// header is as long as it says it is, and which fields are in it depends
    /// on which release wrote it: a file from 2012 stops where the fields it
    /// knew about stopped. Reading one that is not there takes the bytes of
    /// whatever follows the header instead, and every field after it as well.
    ///
    /// How much room it needs is the type's own size, so nothing has to say
    /// it twice. A type whose size depends on the bytes cannot be measured
    /// before it is read, and asks only that the container is not already
    /// finished.
    pub fn if_room(ty: Ty) -> Ty {
        Ty::present_if(Expr::lit(1), ty)
    }
    /// The same, and only where `when` says so as well: a ZIP64 record in a
    /// central directory holds only the fields whose 32-bit counterparts were
    /// left as placeholders, so each of them asks both questions. `when` is a
    /// number that is one or zero, which is what a comparison answers.
    pub fn present_if(when: Expr, ty: Ty) -> Ty {
        let bytes = crate::decode::fixed_bits(&ty).map_or(1, |b| (b + 7) / 8) as i128;
        let room = Expr::lit(bytes - 1).less_than(Expr::Remaining);
        Ty::switch(when.mul(room), vec![(1, ty)], Ty::bytes(Expr::lit(0)))
    }
    pub fn enumeration(name: &str, inner: Ty, cases: &[(i128, &str)]) -> Ty {
        Ty::enum_with(name, inner, cases, &[], false)
    }
    /// An enum whose numbers are shown in hex.
    pub fn enumeration_hex(name: &str, inner: Ty, cases: &[(i128, &str)]) -> Ty {
        Ty::enum_with(name, inner, cases, &[], true)
    }
    /// An enum that names some values outright and counts the rest: `spans` is
    /// read as (first value, how far apart, name with `{n}` for the count).
    pub fn enum_ranged(name: &str, inner: Ty, cases: &[(i128, &str)], spans: &[(i128, i128, &str)]) -> Ty {
        Ty::enum_with(name, inner, cases, spans, false)
    }
    fn enum_with(name: &str, inner: Ty, cases: &[(i128, &str)], spans: &[(i128, i128, &str)], hex: bool) -> Ty {
        Ty::Enum {
            inner: Box::new(inner),
            def: Arc::new(EnumDef {
                name: name.to_string(),
                cases: cases.iter().map(|(v, n)| (*v, n.to_string())).collect(),
                spans: spans
                    .iter()
                    .map(|(from, step, label)| EnumSpan { from: *from, step: *step, label: label.to_string() })
                    .collect(),
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
            // A sentinel is a reading of the number, not a type of its own, so
            // everything that asks what a field really is looks through it.
            Ty::Enum { inner, .. } | Ty::Nullable { inner, .. } => inner.base(),
            other => other,
        }
    }

    /// The type under a sentinel, or the type itself. A sentinel says how one
    /// value reads and nothing else, so everything asking what the field is
    /// shaped like looks straight through it. See [`Ty::Nullable`].
    pub fn without_sentinel(&self) -> &Ty {
        match self {
            Ty::Nullable { inner, .. } => inner.without_sentinel(),
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
        fn b(en: Endian) -> &'static str {
            match en {
                Endian::Little => " lsb",
                Endian::Big => "",
            }
        }
        match self {
            // Below a byte there is no byte order to name, so `endian` there
            // is the bit order and only the one that is not the default is
            // worth saying. See [`Endian`].
            Ty::UInt { bits, endian } if *bits % 8 != 0 => format!("u{bits}{}", b(*endian)),
            Ty::UInt { bits, endian } if *bits == 8 => format!("u{bits}"),
            Ty::UInt { bits, endian } => format!("u{bits} {}", e(*endian)),
            Ty::Int { bits, endian } if *bits % 8 != 0 => format!("i{bits}{}", b(*endian)),
            Ty::Int { bits, endian } if *bits == 8 => format!("i{bits}"),
            Ty::Int { bits, endian } => format!("i{bits} {}", e(*endian)),
            // Not `i{bits}`: the whole point is that it is not two's
            // complement, and a reader checking the bytes against the value
            // needs the type column to say so.
            Ty::SignMagnitude { bits, endian } if *bits % 8 != 0 => format!("sm{bits}{}", b(*endian)),
            Ty::SignMagnitude { bits, endian: _ } if *bits == 8 => format!("sm{bits}"),
            Ty::SignMagnitude { bits, endian } => format!("sm{bits} {}", e(*endian)),
            // The width names the field it is read from, so the column says
            // `u bits_per_value be` rather than a width nothing in the file
            // has. An expression with no reading leaves a question mark,
            // which is honest: the width is decided somewhere this cannot
            // write down.
            Ty::UIntExpr { bits, endian } => {
                format!("u{} {}", crate::eval::write_expr(bits).map_or("?".to_string(), |s| format!(" {s}")), e(*endian))
            }
            // A sentinel says how the number reads, not what the field is.
            Ty::Nullable { inner, .. } => inner.display_name(),
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
            Ty::EbmlVint { strip_marker: false } => "EBML ID".into(),
            Ty::EbmlVint { strip_marker: true } => "EBML size".into(),
            Ty::Computed(_) => "computed".into(),
            // Told apart from `computed` because the value column will hold a
            // word rather than a number, and a reader checking a row against
            // the bytes needs to know there are none to check against.
            Ty::ComputedText(_) => "computed text".into(),
            Ty::SqliteVarint => "varint".into(),
            Ty::Leb128 { signed: false } => "leb128".into(),
            Ty::Leb128 { signed: true } => "sleb128".into(),
            Ty::Insn { isa } => isa.name().to_string(),
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
            Ty::TextInt { radix: 10, .. } => "decimal".into(),
            Ty::TextInt { radix, .. } => format!("base-{radix} digits"),
            Ty::Struct(s) => s.name.clone(),
            Ty::Array { elem, .. } => format!("{}[]", elem.display_name()),
            Ty::Repeat { elem, .. } => format!("{}[]", elem.display_name()),
            // Not `{elem}[]`: that promises children laid out end to end,
            // and these are placed one per offset, in any order.
            Ty::PointerList { elem, .. } => format!("offsets \u{2192} {}", elem.display_name()),
            // Not `offsets → x`: those come from a table read before the
            // children, and these come one from each child.
            Ty::Chain { elem, .. } => format!("chain \u{2192} {}", elem.display_name()),
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
    /// Take in a borrowed format's vocabulary. `part.root` is the type that
    /// reads it; this is everything that type refers to by name.
    pub fn with_part(mut self, part: &Part) -> Template {
        for (name, ty) in &part.types {
            self.types.insert(name.clone(), ty.clone());
        }
        self
    }
}

/// A format that reads on its own and also inside another one: the type that
/// reads it, and the named types that type refers to.
///
/// A type that refers to itself is the whole reason this exists. Named types
/// live on the template, so a bare `Ty` handed to a borrower cannot carry the
/// names it needs, and a directory whose entries point at directories is
/// nothing but names. Handing over both together is what stops a borrower
/// placing the type and leaving the vocabulary behind.
///
/// Names are prefixed with the format they belong to, `tiff.Ifd`, because the
/// table they land in is shared: two borrowed formats should not fall out over
/// a word as ordinary as `Header`.
#[derive(Debug, Clone)]
pub struct Part {
    pub root: Ty,
    pub types: Vec<(String, Ty)>,
}

impl Part {
    pub fn new(root: Ty) -> Part {
        Part { root, types: Vec::new() }
    }
    pub fn with_type(mut self, name: &str, ty: Ty) -> Part {
        self.types.push((name.to_string(), ty));
        self
    }
}
