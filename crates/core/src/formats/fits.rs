//! FITS: the file astronomy has written since 1979, and still writes. Hubble,
//! JWST, SDSS, Gaia and Chandra all publish in it.
//!
//! A file is a run of header-and-data units, each of them a whole number of
//! 2880-byte blocks. A header is 80-column card images, `KEY     = value /
//! comment`, up to a card that says only `END`, and then blanks to the end of
//! the block. The data after it is `|BITPIX|/8` bytes an element, as many
//! elements as `NAXIS1` through `NAXISn` multiply to, and blanks again to the
//! end of the block. Everything is big-endian, and everything numeric in the
//! header is written as text.
//!
//! So the size of the data is a sum over values found by keyword, among cards
//! whose number and order the format does not fix. That is what
//! [`Expr::tagged_bytes`](crate::template::Expr::tagged_bytes) is for: a
//! lookup by the raw bytes of a key rather than by a number, added for this.
//!
//! An extension carries two more of them. `PCOUNT` is the heap a binary table
//! keeps past its rows, `GCOUNT` the number of groups; both default to what an
//! image has, no heap and one group.
//!
//! What is not read here:
//!
//! - A value is split from its comment at the first `/`, which is wrong for a
//!   string value with a `/` in it: `DATE = '2026/09/02'` splits inside the
//!   quotes. The right rule needs the quoting state of the line, and nothing
//!   in the IR tracks it.
//! - `NAXIS1` through `NAXIS9`. A tenth axis is legal and nothing writes one.
//! - A missing keyword and a keyword whose value is zero both answer 0, so an
//!   axis genuinely declared `NAXIS3 = 0` is read as if it were not there.
//!   That is only wrong for a file whose data is empty anyway, except for
//!   random groups, where `NAXIS1 = 0` is how the format says the group
//!   parameters are all there is.
//! - Which keywords hold a number is a list here rather than something read
//!   from the file, since only a number can be read as one. A file that
//!   writes `NAXIS1  = '3'` fails that card and reads on.
//! - `CONTINUE`, the convention for a string too long for one card, is read as
//!   the separate cards it is written as.
//! - A tile-compressed image is a binary table and reads as one: the rows are
//!   the compressed tiles, and nothing here inflates them.

use crate::template::{Encoding, Endian::Big, Expr as E, StrLen, Template, Ty as T, Until};

/// The block every part of a FITS file is padded out to.
const BLOCK: u32 = 2880;
/// One card image. Eight bytes of keyword, two of `= `, seventy of value.
const CARD: i128 = 80;

/// The keywords the standard says hold a whole number, which are the ones
/// read as one. Everything else in a header is text as far as the layout
/// goes, and reading it as a number would fail on the first `SIMPLE  = T`.
/// `BSCALE` and `EXTEND` are left out for the same reason from the other
/// direction: one is a float and the other a logical.
const NUMERIC: &[&str] = &[
    "BITPIX", "NAXIS", "NAXIS1", "NAXIS2", "NAXIS3", "NAXIS4", "NAXIS5", "NAXIS6", "NAXIS7", "NAXIS8", "NAXIS9",
    "PCOUNT", "GCOUNT", "TFIELDS", "THEAP", "EXTVER", "EXTLEVEL",
];

/// A keyword as it is written in a card: eight bytes, padded with spaces.
fn keyword(name: &str) -> Vec<u8> {
    let mut b = name.as_bytes().to_vec();
    b.resize(8, b' ');
    b
}

/// The value of the card whose keyword is `name`, or zero when no card has it.
fn card_value(name: &str) -> E {
    E::tagged_bytes("cards", &["key"], &keyword(name), &["body", "value"])
}

/// One card: its keyword, the `= ` that says it has a value, and the rest.
///
/// The card is a window of exactly eighty bytes, so the value's search for a
/// `/` stops at the end of the line rather than running into the next card.
fn card() -> T {
    let numeric: Vec<(&str, T)> = NUMERIC.iter().map(|k| (*k, numeric_body())).collect();
    T::sized(
        E::lit(CARD),
        T::structure_named(
            "Card",
            "key",
            "body",
            vec![
                ("key", T::text(StrLen::Padded { size: E::lit(8), pad: b' ' }, Encoding::Ascii)),
                // Not every card has one: `END` and the comment keywords
                // leave these two bytes as part of the text.
                ("body", T::matches(E::field("key"), numeric, text_body())),
            ],
        )
        .counted_as("card"),
    )
}

/// A card whose value is a number: the digits, read as one. The value is the
/// digits after any spaces, ending at the space or the `/` that follows them,
/// and the rest of the line is the comment.
fn numeric_body() -> T {
    valued(T::decimal(StrLen::token(&[b' '], &[b' ', b'/'])))
}

/// A card that has a value: the `= ` that says so, the value, and the comment
/// the rest of the line may hold.
fn valued(value: T) -> T {
    T::structure(
        "Value",
        vec![
            ("mark", T::text(StrLen::Fixed(E::lit(2)), Encoding::Ascii)),
            ("value", value),
            ("comment", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
        ],
    )
    .machinery(&["mark"])
    .payload(&["value"])
}

/// A card whose value is text, a logical, a date or nothing at all.
///
/// Which of the two shapes it has is written in the two bytes after the
/// keyword: `= ` says a value follows, and a card without it is a comment, a
/// line of history, or the `END` that closes the header. Everything up to the
/// first `/` of a valued card is the value; a card with no value is all text,
/// since the `/` of a `COMMENT` line separates nothing.
fn text_body() -> T {
    let plain = T::structure("Note", vec![("value", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii))]);
    T::switch(E::peek(16, Big), vec![(0x3d20, valued(text_value()))], plain)
}

/// The text of a value, up to the comment that may follow it.
fn text_value() -> T {
    T::text(StrLen::Fixed(E::to_bytes(b"/")), Encoding::Ascii)
}

/// How wide one element of the data is, in bytes: `|BITPIX|/8`. A float image
/// says -32 or -64 and means four bytes or eight.
fn element_bytes() -> E {
    let bitpix = card_value("BITPIX");
    E::lit(0).sub(bitpix.clone()).at_least(bitpix).div(E::lit(8))
}

/// How many elements the data holds: the axes multiplied together, plus the
/// heap, times the number of groups. Nothing at all when `NAXIS` is zero,
/// which is the header-only unit every file with extensions opens with.
fn element_count() -> E {
    let axes = (1..=9).fold(E::lit(1), |acc, n| acc.mul(card_value(&format!("NAXIS{n}")).or(E::lit(1))));
    let any = E::lit(0).less_than(card_value("NAXIS"));
    let groups = card_value("GCOUNT").or(E::lit(1));
    axes.add(card_value("PCOUNT")).mul(groups).mul(any)
}

/// The data, read as the type BITPIX names. The switch is over the whole
/// array rather than over one element, so the row says `i16 be[]` rather than
/// leaving the reader with `switch[]`.
fn data_array() -> T {
    let of = |ty: T| T::array(ty, placed_count());
    T::switch(
        card_value("BITPIX"),
        vec![
            (8, of(T::UInt { bits: 8, endian: Big })),
            (16, of(T::Int { bits: 16, endian: Big })),
            (32, of(T::Int { bits: 32, endian: Big })),
            (64, of(T::Int { bits: 64, endian: Big })),
            (-32, of(T::F32(Big))),
            (-64, of(T::F64(Big))),
        ],
        // A BITPIX nobody defined: the room is right, since the same number
        // sized it, and what is in it is anyone's guess.
        T::bytes(E::Remaining),
    )
}

/// How many elements to place: what the header says, and never more than the
/// room the data unit has. A file cut off mid-transmission shows the elements
/// it does have rather than refusing the header that described them.
fn placed_count() -> E {
    element_count().at_most(E::Remaining.div(element_bytes().at_least(E::lit(1))))
}

/// One header-and-data unit: the cards, the blanks that pad them to a block,
/// the data those cards sized, and the blanks that pad that.
fn hdu() -> T {
    T::structure(
        "HDU",
        vec![
            ("cards", T::repeat(card(), Until::FieldBytes { field: "key".into(), bytes: keyword("END") })),
            ("header_pad", T::bytes(E::size_of("cards").pad_to(BLOCK))),
            (
                "data",
                T::sized(
                    element_bytes().mul(element_count()).at_most(E::Remaining),
                    data_array(),
                ),
            ),
            ("data_pad", T::bytes(E::size_of("data").pad_to(BLOCK).at_most(E::Remaining))),
        ],
    )
    .machinery(&["header_pad", "data_pad"])
    .counted_as("HDU")
}

pub fn fits() -> Template {
    Template::new("fits", T::structure("FITS", vec![("hdus", T::repeat(hdu(), Until::End))]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// Pad a run of cards out to the block a FITS header is written in.
    fn header(cards: &[&str]) -> Vec<u8> {
        let mut b = Vec::new();
        for c in cards {
            let mut card = c.as_bytes().to_vec();
            assert!(card.len() <= 80, "card too long: {c}");
            card.resize(80, b' ');
            b.extend_from_slice(&card);
        }
        b.resize(b.len().div_ceil(2880) * 2880, b' ');
        b
    }

    /// Pad a data unit out to the block it is written in.
    fn padded(mut data: Vec<u8>) -> Vec<u8> {
        data.resize(data.len().div_ceil(2880) * 2880, 0);
        data
    }

    /// A 3 by 2 image of 16-bit integers, which is twelve bytes of data in a
    /// block of its own.
    fn image() -> Vec<u8> {
        let mut b = header(&[
            "SIMPLE  =                    T / conforms to FITS standard",
            "BITPIX  =                   16 / 16-bit integers",
            "NAXIS   =                    2",
            "NAXIS1  =                    3",
            "NAXIS2  =                    2",
            "END",
        ]);
        let mut data = Vec::new();
        for v in [1i16, -2, 3, -4, 5, -6] {
            data.extend_from_slice(&v.to_be_bytes());
        }
        b.extend_from_slice(&padded(data));
        b
    }

    /// The text of a value, with the blanks a card is padded with taken off.
    fn text(v: &Value) -> String {
        match v {
            Value::Str(s) => s.trim().to_string(),
            other => panic!("not text: {other:?}"),
        }
    }

    fn eval(bytes: Vec<u8>) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(bytes)), Evaluator::new(fits()))
    }

    #[test]
    fn the_header_runs_to_the_end_card_and_then_to_the_block() {
        let (d, mut ev) = eval(image());
        let cards = ev.node(&d, &[0, 0, 0]).unwrap();
        assert_eq!(cards.child_count, 6);
        assert_eq!(cards.size_bits, 6 * 80 * 8);
        // The blanks after the last card fill the rest of the 2880 block.
        let pad = ev.node(&d, &[0, 0, 1]).unwrap();
        assert_eq!(pad.size_bits, (2880 - 6 * 80) * 8);
    }

    #[test]
    fn a_card_reads_as_a_keyword_a_value_and_a_comment() {
        let (d, mut ev) = eval(image());
        let key = ev.node(&d, &[0, 0, 0, 1, 0]).unwrap();
        assert_eq!(key.value, Value::Str("BITPIX".into()));
        assert_eq!(ev.node(&d, &[0, 0, 0, 1, 1, 1]).unwrap().value, Value::Int(16));
        let comment = ev.node(&d, &[0, 0, 0, 1, 1, 2]).unwrap();
        // The rest of the line, blanks and all: a card is padded, not trimmed.
        assert_eq!(text(&comment.value), "/ 16-bit integers");
        // A card is eighty bytes whatever is written in it.
        assert_eq!(ev.node(&d, &[0, 0, 0, 1]).unwrap().size_bits, 640);
    }

    #[test]
    fn the_data_is_sized_and_typed_by_cards_found_by_keyword() {
        let (d, mut ev) = eval(image());
        let data = ev.node(&d, &[0, 0, 2]).unwrap();
        assert_eq!(data.size_bits, 12 * 8);
        assert_eq!((data.type_name.as_str(), data.child_count), ("i16 be[]", 6));
        assert_eq!(ev.node(&d, &[0, 0, 2, 1]).unwrap().value, Value::Int(-2));
        // And the data is padded to a block of its own.
        assert_eq!(ev.node(&d, &[0, 0, 3]).unwrap().size_bits, (2880 - 12) * 8);
    }

    #[test]
    fn a_negative_bitpix_is_a_float_of_that_many_bits() {
        let mut b = header(&[
            "SIMPLE  =                    T",
            "BITPIX  =                  -32 / IEEE single precision",
            "NAXIS   =                    1",
            "NAXIS1  =                    2",
            "END",
        ]);
        let mut data = Vec::new();
        for v in [1.5f32, -0.25] {
            data.extend_from_slice(&v.to_be_bytes());
        }
        b.extend_from_slice(&padded(data));
        let (d, mut ev) = eval(b);
        let array = ev.node(&d, &[0, 0, 2]).unwrap();
        assert_eq!((array.type_name.as_str(), array.child_count), ("f32 be[]", 2));
        assert_eq!(ev.node(&d, &[0, 0, 2, 0]).unwrap().value, Value::Float(1.5));
    }

    #[test]
    fn a_header_only_unit_has_no_data_at_all() {
        let b = header(&["SIMPLE  =                    T", "BITPIX  =                    8", "NAXIS   =                    0", "EXTEND  =                    T", "END"]);
        let (d, mut ev) = eval(b);
        assert_eq!(ev.node(&d, &[0, 0, 2]).unwrap().size_bits, 0);
        // `EXTEND` says T, which is a logical and not a number: reading it as
        // one would fail the card, so it stays text.
        assert_eq!(text(&ev.node(&d, &[0, 0, 0, 3, 1, 1]).unwrap().value), "T");
        // And `END` has no value at all, so the card is one run of text.
        let end = ev.node(&d, &[0, 0, 0, 4, 1]).unwrap();
        assert_eq!((end.type_name.as_str(), end.child_count), ("Note", 1));
    }

    /// A primary header with no data, then a binary table whose rows are
    /// followed by a heap: `PCOUNT` is that heap, and it counts in bytes
    /// because a table's BITPIX is 8.
    #[test]
    fn an_extensions_data_includes_its_heap() {
        let mut b = header(&[
            "SIMPLE  =                    T",
            "BITPIX  =                    8",
            "NAXIS   =                    0",
            "EXTEND  =                    T",
            "END",
        ]);
        b.extend_from_slice(&header(&[
            "XTENSION= 'BINTABLE'           / binary table extension",
            "BITPIX  =                    8",
            "NAXIS   =                    2",
            "NAXIS1  =                    4 / bytes in a row",
            "NAXIS2  =                    3 / rows",
            "PCOUNT  =                    6 / bytes in the heap",
            "GCOUNT  =                    1",
            "TFIELDS =                    1",
            "END",
        ]));
        b.extend_from_slice(&padded(vec![7u8; 12 + 6]));
        let (d, mut ev) = eval(b);
        let hdus = ev.node(&d, &[0]).unwrap();
        assert_eq!(hdus.child_count, 2);
        // Twelve bytes of rows and six of heap.
        let data = ev.node(&d, &[0, 1, 2]).unwrap();
        assert_eq!(data.size_bits, 18 * 8);
        // The extension starts on the block after the primary header.
        assert_eq!(ev.node(&d, &[0, 1]).unwrap().offset_bits, 2880 * 8);
        let kind = ev.node(&d, &[0, 1, 0, 0, 1, 1]).unwrap();
        assert_eq!(text(&kind.value), "'BINTABLE'");
    }

    #[test]
    fn a_card_says_which_cards_sized_the_data() {
        use crate::eval::Role;
        let (d, mut ev) = eval(image());
        let o = ev.origins(&d, &[0, 0, 2]).unwrap();
        let seen: Vec<_> = o.iter().map(|x| (x.role, x.label.clone(), x.value.clone())).collect();
        assert!(seen.iter().any(|(r, l, v)| *r == Role::Length && l.starts_with("cards[1]") && v == "16"), "{seen:?}");
        assert!(seen.iter().any(|(_, l, v)| l.starts_with("cards[3]") && v == "3"), "{seen:?}");
    }
}
