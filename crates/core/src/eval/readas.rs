//! What the template reads a stretch of bytes as, where that makes any text
//! found there a coincidence.
//!
//! The strings view finds printable runs without a template, and most of what
//! it finds in a binary is there by chance: x86 function prologues are
//! `AWAVAUATUSH`, loud 16-bit samples have two printable bytes, and a JPEG
//! quantization table counts up through the digits. A template already knows
//! which bytes are numbers, instructions or packed data, so the view can say
//! so beside each string that falls inside one. It says so rather than hiding
//! the string, because text inside what the template calls samples is as
//! likely to be the template reading the wrong bytes: a bat recorder's
//! metadata block read as sound is the case that showed it.

use super::*;

/// What a template reads a run of bytes as, of the readings that printable
/// bytes turn up in by chance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotText {
    /// A list of numbers: integers, floats, enums or flags.
    Numbers,
    /// A list of machine instructions.
    Code,
    /// A stream a codec unpacks, such as deflate or LZMA.
    Packed,
}

impl NotText {
    pub fn name(self) -> &'static str {
        match self {
            NotText::Numbers => "numbers",
            NotText::Code => "code",
            NotText::Packed => "packed",
        }
    }
}

/// The run of numbers, run of instructions or packed stream that a stretch of
/// bytes lies inside.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadAs {
    pub kind: NotText,
    /// The run or the stream, in the tab's paths.
    pub path: Vec<usize>,
    /// Its name, as the listing gives it.
    pub name: String,
    /// What one element is, `i16 le` or `x86-64`, or the codec's name for a
    /// packed stream.
    pub what: String,
    pub offset_bits: u64,
    pub size_bits: u64,
}

impl Evaluator {
    /// Whether the node at `path` is a run or stream that text turns up in by
    /// chance, and what one element of it is.
    ///
    /// Only lists laid end to end count. A list whose elements are placed by
    /// pointer has no stretch of the file of its own, and a single number is
    /// often four letters a template chose to read as one: a chunk ID or a
    /// signature written as a `u32`.
    pub(super) fn not_text(&self, path: &[usize]) -> Option<(NotText, String)> {
        match &self.memo[path].ty {
            Ty::Decoded { codec, .. } => packs(codec).then(|| (NotText::Packed, codec.as_str().to_string())),
            Ty::Array { elem, .. } | Ty::Repeat { elem, .. } => {
                let elem = plain(&self.template, elem);
                let kind = element_kind(value_kind(&self.template, elem))?;
                Some((kind, elem.display_name()))
            }
            _ => None,
        }
    }

    /// The run of numbers or instructions, or the packed stream, holding every
    /// bit of `[from, to)`, if one does.
    ///
    /// Both ends are asked, and the answer is only given when they land in the
    /// same run: a string that starts in a table's last value and runs on into
    /// the name after it is not inside the table.
    pub(super) fn read_as_under<S: Source>(&mut self, doc: &Document<S>, root: &[usize], from: u64, to: u64) -> R<Option<ReadAs>> {
        if to <= from {
            return Ok(None);
        }
        let Some((first, kind, what)) = self.not_text_run(doc, root, from)? else { return Ok(None) };
        let Some((last, ..)) = self.not_text_run(doc, root, to - 1)? else { return Ok(None) };
        if first != last {
            return Ok(None);
        }
        let r = self.memo[&first].clone();
        let size = self.size_of(doc, &first)?;
        let name = self.label(doc, &first, &r)?;
        Ok(Some(ReadAs { kind, path: first, name, what, offset_bits: r.offset, size_bits: size }))
    }

    /// The run holding `bit`, and what it is.
    fn not_text_run<S: Source>(&mut self, doc: &Document<S>, root: &[usize], bit: u64) -> R<Option<(Vec<usize>, NotText, String)>> {
        let found = match self.locate_run_under(doc, root, bit) {
            Ok(found) => found,
            Err(e) if e.interrupted() => return Err(e),
            // Past the end of what the template reads, or somewhere it could
            // not read: nothing says these bytes are not text.
            Err(_) => return Ok(None),
        };
        if let Some((kind, what)) = self.not_text(&found) {
            return Ok(Some((found, kind, what)));
        }
        // A list whose element type is chosen per element, by a switch, is only
        // known to hold numbers once the element is read. The walk went on to
        // it, and the list is its parent.
        let Some((_, parent)) = found.split_last() else { return Ok(None) };
        if found.len() <= root.len() || !matches!(self.memo[parent].ty, Ty::Array { .. } | Ty::Repeat { .. }) {
            return Ok(None);
        }
        let elem = &self.memo[&found].ty;
        let Some(kind) = element_kind(value_kind(&self.template, elem)) else { return Ok(None) };
        Ok(Some((parent.to_vec(), kind, elem.display_name())))
    }
}

/// The reading a list's elements give it, where it is one text turns up in
/// by chance.
fn element_kind(kind: &str) -> Option<NotText> {
    match kind {
        "uint" | "int" | "float" | "enum" | "flags" => Some(NotText::Numbers),
        "insn" => Some(NotText::Code),
        _ => None,
    }
}

/// The type an element is declared as, past the names and wrappers that say
/// nothing about what it holds.
fn plain<'t>(template: &'t Template, mut ty: &'t Ty) -> &'t Ty {
    loop {
        ty = match ty {
            Ty::Named(n) => match template.types.get(&**n) {
                Some(t) => t,
                None => return ty,
            },
            Ty::Sized { inner, .. } | Ty::SizedBits { inner, .. } | Ty::Origin { inner } | Ty::When { inner, .. } => inner,
            _ => return ty,
        }
    }
}

/// Whether what a codec reads is packed, so that printable bytes in it are
/// chance. A stored stream is its own contents, and a text codec's input is
/// the text.
fn packs(codec: &Packing) -> bool {
    use crate::codec::Codec;
    !matches!(
        codec,
        Packing::Fixed(Codec::Stored | Codec::CfbWorkbook | Codec::Latin1Text | Codec::EscapedLatin1Text)
    )
}

impl<S: Source> Tab<'_, S> {
    /// The run of numbers or instructions, or the packed stream, holding every
    /// bit of `[from, to)` of the tab, if one does.
    pub fn read_as(&mut self, from: u64, to: u64) -> R<Option<ReadAs>> {
        let root = self.root().to_vec();
        let Some(mut found) = self.ev.read_as_under(self.doc, &root, from, to)? else { return Ok(None) };
        let Some(path) = self.path_out(&found.path) else { return Ok(None) };
        found.path = path;
        Ok(Some(found))
    }
}
