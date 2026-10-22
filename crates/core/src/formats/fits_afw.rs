//! What the LSST Science Pipelines' own cards say about the FITS files they
//! write, read from those cards: the names of a mask's bits, and which of the
//! saved objects is the image's PSF, WCS and so on.
//!
//! The pipelines (afw, the Rubin Observatory's image library) write an image
//! as three units, `IMAGE`, `MASK` and `VARIANCE`, and the objects the image
//! carries with it into binary tables after them, with an index. Neither what
//! a mask bit means nor which object is which is fixed. Each file says, in
//! cards, and these are the builders that read them: a
//! [`Ty::Schema`](crate::template::Ty::Schema) is a type whose shape is in the
//! file, and both of these are.
//!
//! ## Mask planes
//!
//! A mask is an image of integers, one bit a reason: a pixel of 34 is bits 1
//! and 5, which in a Rubin visit image are `SAT` and `DETECTED`. The mask's
//! header names its own bits, one card a bit, `MP_SAT = 1`, and a pipeline
//! that adds a plane adds a card: the DP2 visit images name 17 and the
//! difference images 26, and the test files of a few years earlier 7 or 9.
//!
//! The template asks for a mask's pixels as [`MASK`], keyed by where the
//! header's cards start and how wide a pixel is. The builder walks those cards
//! once, keeps every `MP_` card, and makes the pixels integers of that width
//! whose bits have those names. A plane name too long for eight columns is a
//! `HIERARCH` card, `HIERARCH MP_DETECTED_NEGATIVE = 6`, and is read from the
//! name in its body.
//!
//! ## Components
//!
//! The primary header says which saved object is which part of the image,
//! with a card a part: `HIERARCH ARCHIVE_ID_PSF = 417` says object 417 is the
//! PSF. Older files write `PSF_ID = 1` instead, with "archive ID" in the
//! comment, and files in between write both. The index has a row for every
//! piece of every object, and its first column is the object's ID, so the
//! template asks each row for [`ARCHIVE`], and the builder reads the primary's
//! cards once and makes a number that is one of those IDs read as the name of
//! the part.
//!
//! What is not read:
//!
//! - A mask whose pixels the header scales with `BZERO` or `BSCALE`, which
//!   keeps the reading every scaled image has. An unsigned 16-bit mask is
//!   written that way, as the signed number 32768 below what it is, and its
//!   top bit is then the opposite of the stored one. Naming the bits of what a
//!   pixel is worth needs flags over a worked-out number, which the IR cannot
//!   say: a flag is a bit of the bytes it covers.
//! - Two FITS files read in one reading, at the same place in spaces of the
//!   same size, with headers that differ. A build is kept by its key, and the
//!   key is where the cards start and how long the space is, since nothing an
//!   expression can say tells two spaces apart.

use crate::eval::{Descriptions, R};
use crate::template::{Built, Endian::Big, Expr as E, KeyValue, SchemaBuilder, Ty as T};

/// The schema kind of a mask's pixels.
pub(super) const MASK: &str = "fits_mask";

/// The schema kind of the part of the image an archive index row is a piece
/// of.
pub(super) const ARCHIVE: &str = "fits_archive";

/// What the bits of a mask's pixels are called, from the `MP_` cards.
#[derive(Debug, Default)]
pub(super) struct Planes;

impl SchemaBuilder for Planes {
    /// The key is where the header's cards start and how long the space is,
    /// which tell one mask from another, then how many bits a pixel is and
    /// whether it is signed.
    fn build(&self, key: &[KeyValue], table: &mut dyn Descriptions) -> R<Built> {
        let [_, _, KeyValue::Int(bits), KeyValue::Int(signed)] = key else {
            return Ok(Built::by_heart(T::bytes(E::Remaining)));
        };
        let bits = *bits as u32;
        let inner = if *signed != 0 { T::Int { bits, endian: Big } } else { T::UInt { bits, endian: Big } };
        let cards = table.records()?;
        let mut planes = Vec::new();
        for card in &cards {
            let Some((name, value)) = named_card(table, card)? else { continue };
            let Some(plane) = name.strip_prefix("MP_") else { continue };
            let Some(bit) = number(table, &value)?.and_then(|v| u32::try_from(v).ok()) else { continue };
            if bit < bits {
                planes.push((bit, plane.to_string()));
            }
        }
        planes.sort();
        let named: Vec<(u32, &str)> = planes.iter().map(|(b, n)| (*b, n.as_str())).collect();
        let pixels = T::array(T::flags("Mask planes", inner, &named), super::fits::placed_count());
        Ok(Built { ty: pixels, from: list_of(&cards), members_from: Vec::new() })
    }

    fn key_text(&self, _key: &[KeyValue]) -> String {
        "mask planes".to_string()
    }
}

/// Which part of the image each saved object is, from the primary header's
/// `ARCHIVE_ID_*` cards.
#[derive(Debug, Default)]
pub(super) struct Components;

impl SchemaBuilder for Components {
    fn build(&self, _key: &[KeyValue], table: &mut dyn Descriptions) -> R<Built> {
        let cards = table.records()?;
        let mut parts: Vec<(i128, String)> = Vec::new();
        for card in &cards {
            let Some((name, value)) = named_card(table, card)? else { continue };
            let part = match name.strip_prefix("ARCHIVE_ID_") {
                Some(part) => part.to_string(),
                // The older spelling, which only the comment tells from any
                // other keyword ending in `_ID`.
                None => match name.strip_suffix("_ID") {
                    Some(part) if comment(table, card)?.contains("archive ID") => part.to_string(),
                    _ => continue,
                },
            };
            let Some(id) = number(table, &value)? else { continue };
            if !parts.iter().any(|(i, p)| *i == id && *p == part) {
                parts.push((id, part));
            }
        }
        // The ID is the first cell of the row asking, which is a list of one
        // number; a row whose ID no card names is no part, and has no field.
        let id = E::elem_field("cells", E::lit(0), &["0"]);
        let named = parts.iter().fold(E::lit(0), |any, (i, _)| any.or(id.clone().equals(E::lit(*i))));
        let cases: Vec<(i128, &str)> = parts.iter().map(|(i, p)| (*i, p.as_str())).collect();
        let ty = T::when(named, T::enumeration("Component", T::computed(id), &cases));
        Ok(Built { ty, from: list_of(&cards), members_from: Vec::new() })
    }

    fn key_text(&self, _key: &[KeyValue]) -> String {
        "archive components".to_string()
    }
}

/// The list the walk found `cards` in, which is the header as a whole: what
/// a reader asking why a value reads this way is sent to.
fn list_of(cards: &[Vec<usize>]) -> Option<Vec<usize>> {
    cards.first().map(|c| c[..c.len() - 1].to_vec())
}

/// A card's keyword, the long one for a `HIERARCH` card, and where its value
/// is. Nothing for a card with no value.
fn named_card(table: &mut dyn Descriptions, card: &[usize]) -> R<Option<(String, Vec<usize>)>> {
    let Some(key) = table.find(card, &["key"])? else { return Ok(None) };
    let key = table.text(&key)?;
    let name = match key.trim_end() {
        "HIERARCH" => match table.find(card, &["body", "name"])? {
            Some(at) => table.text(&at)?.trim().to_string(),
            None => return Ok(None),
        },
        short => short.to_string(),
    };
    Ok(table.find(card, &["body", "value"])?.map(|value| (name, value)))
}

/// The comment of a card, or nothing.
fn comment(table: &mut dyn Descriptions, card: &[usize]) -> R<String> {
    match table.find(card, &["body", "comment"])? {
        Some(at) => table.text(&at),
        None => Ok(String::new()),
    }
}

/// The whole number a card's value is written as. A number in a `HIERARCH`
/// card reads as a real, whose text is a field of its own; in a card whose
/// keyword the template does not list it is the text itself.
fn number(table: &mut dyn Descriptions, value: &[usize]) -> R<Option<i128>> {
    let at = match table.find(value, &["text"])? {
        Some(text) => text,
        None => value.to_vec(),
    };
    Ok(table.text(&at)?.trim().parse::<i128>().ok())
}
