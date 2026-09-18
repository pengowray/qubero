//! What one value of a pickle comes to in a few words: the row of the entry
//! that holds it, and what a line spells rather than is.
//!
//! Apart from the placement in [`pickletree`](super::pickletree) because this
//! is the one part of the reading that decides how much of a value to show and
//! where to cut it, and because a new kind of value wants a word here whether
//! or not it wants a row of its own.

use super::*;
use crate::formats::pickle::familiar::{Dtype, Kind, Match, Names, Value};

/// How much of what a reference names the `refers to` row shows. A repeated
/// dictionary key is a word or two; anything longer is cut rather than filling
/// a row meant to be read at a glance. Fewer bytes than characters, because a
/// byte string is shown in hex and takes three columns a byte.
pub(super) const MOST_SHOWN_TEXT: usize = 120;
pub(super) const MOST_SHOWN_BYTES: usize = 32;

/// What a reference names, as the row saying so shows it: the text itself,
/// or the bytes in hex when the slot holds a byte string. `whole` is how long
/// the thing is, so that a long one says it was cut.
pub(super) fn shown(bytes: &[u8], text: bool, whole: usize) -> String {
    let mut said = match text {
        // The read may have stopped inside a character, so take what is whole.
        true => match std::str::from_utf8(bytes) {
            Ok(said) => said.to_string(),
            Err(e) => String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned(),
        },
        false => bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "),
    };
    if bytes.len() < whole {
        said.push_str("...");
    }
    said
}

/// What a protocol 0 line spells, for a row that shows the value rather than
/// the spelling. Cut the way a text read out of the file is, since a line as
/// long as a page is a line a reader takes in at a glance or not at all.
pub(super) fn spelling_of(found: &Match, at: usize, bytes: bool) -> String {
    let Some(held) = found.decoded(at) else { return String::new() };
    match bytes {
        true => shown(&held[..held.len().min(MOST_SHOWN_BYTES)], false, held.len()),
        false => {
            let said = String::from_utf8_lossy(held);
            shown(&held[..fits(&said, MOST_SHOWN_TEXT)], true, held.len())
        }
    }
}

/// How many bytes of a text to take for a row, cut on a character boundary.
pub(super) fn fits(said: &str, most: usize) -> usize {
    match said.len() <= most {
        true => said.len(),
        false => (0..=most).rev().find(|n| said.is_char_boundary(*n)).unwrap_or(0),
    }
}

impl Evaluator {
    /// One value in a few words, for the row of the entry that holds it: the
    /// value itself when it is a single thing, and what kind of thing and how
    /// much of it otherwise. Nothing for a value there is no short word for,
    /// which leaves the row counting its fields as it did.
    pub(super) fn pickle_said<S: Source>(&self, doc: &Document<S>, found: &Match, whole: &Resolved, base: u64, v: &Value) -> R<Option<String>> {
        let read = |at: usize, len: usize, text: bool| -> R<String> {
            let most = len.min(if text { MOST_SHOWN_TEXT } else { MOST_SHOWN_BYTES });
            Ok(shown(&self.read(doc, whole, base + at as u64 * 8, most as u64 * 8)?, text, len))
        };
        let many = |what: &str, n: usize| if n == 0 { format!("empty {what}") } else { format!("{what} of {n}") };
        let across = |dimensions: &[u64]| dimensions.iter().map(u64::to_string).collect::<Vec<_>>().join(" x ");
        let path_of = |v: &Value| match &v.kind {
            Kind::Class { path, .. } => Some(path.clone()),
            _ => None,
        };
        Ok(match &v.kind {
            Kind::None => Some("None".into()),
            Kind::Bool(b) => Some(if *b { "True" } else { "False" }.into()),
            Kind::Int { value, .. } => Some(value.to_string()),
            Kind::Float { value, .. } => Some(value.to_string()),
            Kind::Text { at, len } | Kind::Ref(Names::Text { at, len }) => Some(read(*at, *len, true)?),
            // A line that spells a value rather than being it reads as what
            // it spells, which the form worked out when it matched.
            Kind::Spelled { at, bytes, .. } => Some(spelling_of(found, *at, *bytes)),
            Kind::Bytes { at, len } | Kind::Ref(Names::Bytes { at, len }) => Some(read(*at, *len, false)?),
            Kind::Ref(Names::Made { what, at, .. }) => Some(format!("{} at {:#04x}", what.name(), base as usize / 8 + at)),
            Kind::List(items) => Some(many("list", items.len())),
            Kind::Tuple(items) => Some(many("tuple", items.len())),
            Kind::Set(items) => Some(many("set", items.len())),
            Kind::FrozenSet(items) => Some(many("frozenset", items.len())),
            Kind::Dict(entries) => Some(many("dict", entries.len())),
            // The word the table's column header uses, `float64`, rather than
            // NumPy's letters: `<f8` reads as less than something.
            Kind::Array { dtype: Dtype::Record { .. }, dimensions, .. } => Some(format!("record array {}", across(dimensions))),
            Kind::Array { dtype, dimensions, .. } => Some(format!("{} array {}", super::pickleframe::dtype_word(dtype), across(dimensions))),
            Kind::Objects { dimensions, .. } => Some(format!("object array {}", across(dimensions))),
            Kind::Class { path, .. } => Some(path.clone()),
            Kind::Instance { class, .. } => path_of(class),
            Kind::Made { callable, .. } => path_of(callable),
            Kind::Object { what, .. } => Some(what.name().into()),
            Kind::DType(Dtype::Record { .. }) => None,
            Kind::DType(dtype) => Some(super::pickleframe::dtype_word(dtype)),
        })
    }
}
