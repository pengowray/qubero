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
//! A table's data is its rows rather than one run of bytes. `NAXIS1` is how
//! wide a row is and `NAXIS2` how many there are; `TFIELDS` how many columns,
//! and `TFORMn` what one column holds. A binary table's column is an array of
//! the repeat count that `TFORMn` opens with, of the type its letter names;
//! an ASCII table's is text, as wide as `TFORMn` says, at the column `TBCOLn`
//! puts it. What is left of the data unit after the rows is the heap, which is
//! `PCOUNT` bytes and is where a variable-length column's arrays live.
//!
//! What is not read here:
//!
//! - A column's name. `TTYPEn` holds it, and a structure's field names are
//!   fixed when the template is built, so the columns are `col1` and on. That
//!   needs a name an expression can answer, which the IR has no way to say.
//! - `TFORM1` through `TFORM32`, and `TBCOL1` through `TBCOL32`, since a
//!   keyword is looked up by the name written out here. A table with more
//!   columns than that reads the first 32 and says so in the row's last field.
//! - Which kind of table it is, is read from `TBCOL1` and `TFIELDS` rather
//!   than from `XTENSION`, which says so in text: nothing in the IR picks a
//!   type by text found by keyword, only by text in a field beside it.
//! - The type letter of a column is read as the number its ASCII is, for the
//!   same reason. So a `TFORMn` card shows `74` where the file says `J`.
//! - What is in the heap. The descriptors say how long each array is and where
//!   it starts, and placing the arrays those point at needs a pointer list
//!   whose offsets are read from inside every row of a table.
//! - `TSCALn` and `TZEROn`, which say what a column's numbers mean. An
//!   unsigned 16-bit column is written as a signed one with a zero point of
//!   32768, and reads here as the signed numbers it is written as.
//! - Every column of every row asks the header again for its `TFORMn`, so a
//!   table with many rows walks the cards many times over.
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
//! - A quoted value with no closing quote runs to the end of its card rather
//!   than being called out as the unterminated string it is.
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

/// How many columns of a table are read. The keywords a column is described
/// by carry its number in their names, so every one of them has to be written
/// out here; a table with more columns than this reads the ones it has and
/// says so, in the row's last field.
const COLUMNS: usize = 32;

/// What the columns of a row are called. A column's own name is its `TTYPEn`
/// card, which is in the file rather than in the template, and a structure's
/// field names are fixed when the template is built. See the module note.
const COL_NAMES: [&str; COLUMNS] = [
    "col1", "col2", "col3", "col4", "col5", "col6", "col7", "col8", "col9", "col10", "col11", "col12", "col13",
    "col14", "col15", "col16", "col17", "col18", "col19", "col20", "col21", "col22", "col23", "col24", "col25",
    "col26", "col27", "col28", "col29", "col30", "col31", "col32",
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

/// A part of the value of the card whose keyword is `name`: the repeat count,
/// the type letter or the width a `TFORMn` card was read as. Zero when no card
/// has that keyword, or when the value was not written in that shape.
fn card_part(name: &str, part: &str) -> E {
    E::tagged_bytes("cards", &["key"], &keyword(name), &["body", "value", "form", part])
}

/// One card: its keyword, the `= ` that says it has a value, and the rest.
///
/// The card is a window of exactly eighty bytes, so the value's search for a
/// `/` stops at the end of the line rather than running into the next card.
fn card() -> T {
    let mut cases: Vec<(String, T)> = NUMERIC.iter().map(|k| ((*k).to_string(), numeric_body())).collect();
    for n in 1..=COLUMNS {
        // Where a column starts in a row of an ASCII table, which is a number
        // like any other, and what type it holds, which is not.
        cases.push((format!("TBCOL{n}"), numeric_body()));
        cases.push((format!("TFORM{n}"), valued(tform_value())));
    }
    let body = T::Match { on: E::field("key"), cases: cases.into(), default: std::sync::Arc::new(text_body()) };
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
                ("body", body),
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
/// line of history, or the `END` that closes the header. A card with no value
/// is all text, since the `/` of a `COMMENT` line separates nothing.
///
/// A value that opens with a quote runs to the quote that closes it, wherever
/// the `/` of the comment is; anything else runs to the first `/`. Which of
/// the two it is, is the byte after the `= `, since a FITS value starts there
/// and nowhere else.
fn text_body() -> T {
    let plain = T::structure("Note", vec![("value", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii))]);
    let value = T::switch(E::peek(8, Big), vec![(0x27, quoted_value())], text_value());
    T::switch(E::peek(16, Big), vec![(0x3d20, valued(value))], plain)
}

/// The text of a value, up to the comment that may follow it.
fn text_value() -> T {
    T::text(StrLen::Fixed(E::to_bytes(b"/")), Encoding::Ascii)
}

/// A quoted string value: the quote that opens it, and the parts it is written
/// in. A `/` inside one is text, not the start of a comment, and a `''` is one
/// quote of the value rather than the end of it, so the end is the first quote
/// that no second quote follows.
///
/// The parts are that rule made into a list: each of them runs to a quote, and
/// takes the quote after it too when there is one. A string with no escape in
/// it, which is nearly all of them, is one part.
fn quoted_value() -> T {
    T::structure(
        "Text",
        vec![
            ("open", T::text(StrLen::Fixed(E::lit(1)), Encoding::Ascii)),
            ("parts", T::repeat(quoted_part(), Until::FieldValue { field: "ended".into(), value: 1 })),
        ],
    )
    .machinery(&["open"])
    .payload(&["parts"])
}

/// One part of a quoted string: the text up to the next quote, that quote, and
/// the second quote of an escaped pair when that is what it turns out to be.
/// `ended` is the answer to whether this part closed the string, which is what
/// the list repeats until.
fn quoted_part() -> T {
    let escape = T::switch(E::peek(8, Big), vec![(0x27, T::text(StrLen::Fixed(E::lit(1)), Encoding::Ascii))], T::bytes(E::lit(0)));
    T::inline_structure(
        "Part",
        vec![
            // A card with no closing quote at all measures to the end of its
            // eighty bytes, and the quote after it would then read past them.
            ("text", T::text(StrLen::Fixed(E::to_bytes(b"'").at_most(E::Remaining.sub(E::lit(1)).at_least(E::lit(0)))), Encoding::Ascii)),
            ("quote", T::text(StrLen::Fixed(E::lit(1).at_most(E::Remaining)), Encoding::Ascii)),
            ("escape", escape),
            ("ended", T::computed(E::size_of("escape").less_than(E::lit(1)))),
        ],
    )
    .machinery(&["quote", "escape", "ended"])
    .payload(&["text"])
}

/// The value of a `TFORMn` card, which says what one column of a table holds.
///
/// A binary table writes `rTa`: a repeat count, a type letter, and sometimes
/// more. An ASCII table writes `Tw.d`: a type letter, a width, and for a float
/// how many digits are after the point. The two are told apart by whether a
/// digit follows the opening quote, and both leave the same three fields to
/// look up by name: `repeat`, `code` and `width`.
///
/// The type letter is read as the number its ASCII is, since that is what the
/// column's own type switches on, and nothing in the IR switches a type on
/// text found by keyword.
fn tform_value() -> T {
    let binary = digits_then(1, 5);
    T::structure(
        "TFORM",
        vec![
            ("open", T::text(StrLen::Fixed(E::lit(1)), Encoding::Ascii)),
            ("form", T::switch(digit_peek(0), vec![(1, binary)], ascii_form())),
        ],
    )
    .machinery(&["open"])
}

/// One when the byte `n` further on is a digit, and zero when it is not.
fn digit_peek(n: i128) -> E {
    let byte = E::peek_at(E::lit(n * 8), 8, Big);
    E::lit(b'0' as i128 - 1).less_than(byte.clone()).mul(byte.less_than(E::lit(b'9' as i128 + 1)))
}

/// A binary table's `rTa`, where the count is `d` digits long if the byte
/// after those digits is not another digit, and one digit longer if it is.
/// `most` is where the walk stops: a repeat count longer than that reads as
/// that many digits, and the letter after it is read as part of the number.
fn digits_then(d: i128, most: i128) -> T {
    let form = T::structure(
        "Binary column",
        vec![
            ("repeat", T::decimal(StrLen::Fixed(E::lit(d)))),
            ("code", T::u8()),
            ("tail", T::text(StrLen::Fixed(E::to_bytes(b"/")), Encoding::Ascii)),
        ],
    );
    if d == most {
        return form;
    }
    T::switch(digit_peek(d), vec![(1, digits_then(d + 1, most))], form)
}

/// An ASCII table's `Tw.d`: the letter, the width in columns, and the digits
/// after the point that a float writes.
fn ascii_form() -> T {
    T::structure(
        "ASCII column",
        vec![
            ("code", T::u8()),
            ("width", T::decimal(StrLen::token(&[], &[b'.', b' ', b'\'']))),
            ("tail", T::text(StrLen::Fixed(E::to_bytes(b"/")), Encoding::Ascii)),
        ],
    )
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

/// Which of the three shapes a data unit has, read from the cards that
/// describe it rather than from `XTENSION`: a table's kind is a value written
/// in text, and nothing in the IR picks a type by text found by keyword.
///
/// A `TBCOL1` says where the first column of a row starts, and only an ASCII
/// table has one. A `TFIELDS` says how many columns there are, and a table of
/// either kind has one. Anything else is an image, or a unit with no data.
fn table_kind() -> E {
    let ascii = E::lit(0).less_than(card_value("TBCOL1"));
    let binary = E::lit(0).less_than(card_value("TFIELDS")).mul(E::lit(2));
    ascii.or(binary)
}

/// The data of a table: its rows, and then the heap a binary table keeps its
/// variable-length arrays in. `PCOUNT` is how many bytes that heap is, and it
/// is what is left of the data unit once the rows are placed.
fn table(row: T) -> T {
    let width = card_value("NAXIS1").at_least(E::lit(1));
    let rows = card_value("NAXIS2").at_most(E::Remaining.div(width.clone()));
    T::structure(
        "Table",
        vec![
            ("rows", T::array(T::sized(width, row).counted_as("row"), rows)),
            ("heap", T::bytes(E::Remaining)),
        ],
    )
}

/// One row of a binary table: every column laid out one after another, each of
/// them an array of as many values as its `TFORMn` says.
fn binary_row() -> T {
    let mut fields: Vec<(&str, T)> = Vec::new();
    for (i, name) in COL_NAMES.iter().enumerate() {
        fields.push((name, T::present_if(has_column(i + 1), binary_column(i + 1))));
    }
    // A table with more columns than there are names here: the rest of the row
    // is bytes, and the field says why.
    fields.push(("columns_not_read", T::present_if(E::lit(COLUMNS as i128).less_than(card_value("TFIELDS")), T::bytes(E::Remaining))));
    T::structure("Row", fields)
}

/// One when the table has an `n`th column.
fn has_column(n: usize) -> E {
    E::lit(n as i128 - 1).less_than(card_value("TFIELDS"))
}

/// One column of a binary table, as the type its `TFORMn` names and as many of
/// them as its repeat count says. A count is one when none is written.
fn binary_column(n: usize) -> T {
    let key = format!("TFORM{n}");
    let code = card_part(&key, "code");
    let r = card_part(&key, "repeat").or(E::lit(1));
    // Never more than the row has room for: a row whose columns do not add up
    // to `NAXIS1` shows the ones that fit rather than failing.
    let of = |ty: T, width: i128| T::array(ty, r.clone().at_most(E::Remaining.div(E::lit(width))));
    let pair = |name: &str, ty: T| T::inline_structure(name, vec![("re", ty.clone()), ("im", ty)]);
    let descriptor = |name: &str, ty: T| T::inline_structure(name, vec![("count", ty.clone()), ("offset", ty)]);
    let text = T::text(StrLen::Fixed(r.clone().at_most(E::Remaining)), Encoding::Ascii);
    T::switch(
        code,
        vec![
            // A logical is written as the letter `T` or `F`, or as a zero byte
            // for a value nobody set.
            (b'L' as i128, text.clone()),
            // A bit column is that many bits, rounded up to whole bytes.
            (b'X' as i128, T::bytes(r.clone().add(E::lit(7)).div(E::lit(8)).at_most(E::Remaining))),
            (b'B' as i128, of(T::UInt { bits: 8, endian: Big }, 1)),
            (b'I' as i128, of(T::Int { bits: 16, endian: Big }, 2)),
            (b'J' as i128, of(T::Int { bits: 32, endian: Big }, 4)),
            (b'K' as i128, of(T::Int { bits: 64, endian: Big }, 8)),
            (b'A' as i128, text),
            (b'E' as i128, of(T::F32(Big), 4)),
            (b'D' as i128, of(T::F64(Big), 8)),
            (b'C' as i128, of(pair("Complex", T::F32(Big)), 8)),
            (b'M' as i128, of(pair("Complex", T::F64(Big)), 16)),
            // A variable-length array is written as how many there are and
            // where in the heap they start.
            (b'P' as i128, of(descriptor("Array", T::Int { bits: 32, endian: Big }), 8)),
            (b'Q' as i128, of(descriptor("Array", T::Int { bits: 64, endian: Big }), 16)),
        ],
        // A type letter nobody defined, or a `TFORMn` written in a shape this
        // could not read: the row still has its width, and this column covers
        // none of it.
        T::bytes(E::lit(0)),
    )
}

/// One row of an ASCII table: its columns are text, each at the column
/// `TBCOLn` gives and as wide as `TFORMn` says. They are placed rather than
/// laid out one after another, since the standard lets them overlap and lets
/// gaps sit between them.
fn ascii_row() -> T {
    let mut fields: Vec<(&str, T)> = Vec::new();
    for (i, name) in COL_NAMES.iter().enumerate() {
        let n = i + 1;
        let key = format!("TFORM{n}");
        let width = card_part(&key, "width").at_least(E::lit(1)).at_most(card_value("NAXIS1").at_least(E::lit(1)));
        let at = card_value(&format!("TBCOL{n}")).sub(E::lit(1)).at_least(E::lit(0));
        let cell = T::at_in_window(at, T::text(StrLen::Fixed(width), Encoding::Ascii));
        fields.push((name, T::present_if(has_column(n), cell)));
    }
    fields.push(("columns_not_read", T::present_if(E::lit(COLUMNS as i128).less_than(card_value("TFIELDS")), T::bytes(E::lit(0)))));
    T::structure("Row", fields)
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
                    T::switch(table_kind(), vec![(1, table(ascii_row())), (2, table(binary_row()))], data_array()),
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
        assert_eq!(kind.type_name, "Text");
        // The quote that opens the string, and the one part it is written in.
        assert_eq!(text(&ev.node(&d, &[0, 1, 0, 0, 1, 1, 1, 0, 0]).unwrap().value), "BINTABLE");
    }

    /// The header of a binary table with a column of every kind this reads.
    fn table_header(cards: &[&str], rows: usize, width: usize, heap: usize) -> Vec<u8> {
        let mut all: Vec<String> = vec![
            "XTENSION= 'BINTABLE'           / binary table extension".into(),
            "BITPIX  =                    8".into(),
            "NAXIS   =                    2".into(),
            format!("NAXIS1  = {width:20} / bytes in a row"),
            format!("NAXIS2  = {rows:20} / rows"),
            format!("PCOUNT  = {heap:20} / bytes in the heap"),
            "GCOUNT  =                    1".into(),
        ];
        all.extend(cards.iter().map(|c| (*c).to_string()));
        all.push("END".into());
        let refs: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
        header(&refs)
    }

    /// A primary header with nothing in it, which is what every file with an
    /// extension opens with.
    fn primary() -> Vec<u8> {
        header(&["SIMPLE  =                    T", "BITPIX  =                    8", "NAXIS   =                    0", "EXTEND  =                    T", "END"])
    }

    #[test]
    fn a_binary_tables_rows_are_the_columns_its_tform_cards_name() {
        let mut b = primary();
        b.extend_from_slice(&table_header(
            &[
                "TFIELDS =                    4",
                "TTYPE1  = 'counts  '",
                "TFORM1  = '1J      '           / one 32-bit integer",
                "TTYPE2  = 'flux    '",
                "TFORM2  = '2E      '           / two floats",
                "TTYPE3  = 'name    '",
                "TFORM3  = '5A      '           / five characters",
                "TFORM4  = 'D       '           / one double, no count",
            ],
            2,
            4 + 8 + 5 + 8,
            0,
        ));
        let mut data = Vec::new();
        for row in 0..2i32 {
            data.extend_from_slice(&(row + 1).to_be_bytes());
            data.extend_from_slice(&1.5f32.to_be_bytes());
            data.extend_from_slice(&(-0.25f32).to_be_bytes());
            data.extend_from_slice(b"abcde");
            data.extend_from_slice(&2.5f64.to_be_bytes());
        }
        b.extend_from_slice(&padded(data));
        let (d, mut ev) = eval(b);
        let rows = ev.node(&d, &[0, 1, 2, 0]).unwrap();
        assert_eq!(rows.child_count, 2);
        // Row 1, column 1: one 32-bit integer.
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 1, 0, 0]).unwrap().value, Value::Int(2));
        // Column 2 is two floats, and the second of them is the second value.
        let flux = ev.node(&d, &[0, 1, 2, 0, 0, 1]).unwrap();
        assert_eq!((flux.type_name.as_str(), flux.child_count), ("f32 be[]", 2));
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 0, 1, 1]).unwrap().value, Value::Float(-0.25));
        // Column 3 is five characters, read as one run of text.
        assert_eq!(text(&ev.node(&d, &[0, 1, 2, 0, 0, 2]).unwrap().value), "abcde");
        // A `TFORMn` with no repeat count means one.
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 0, 3, 0]).unwrap().value, Value::Float(2.5));
        // A row is as wide as `NAXIS1` says.
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 0]).unwrap().size_bits, 25 * 8);
    }

    #[test]
    fn a_column_past_tfields_covers_nothing_and_the_heap_is_what_is_left() {
        let mut b = primary();
        b.extend_from_slice(&table_header(&["TFIELDS =                    1", "TFORM1  = '1I      '"], 3, 2, 5));
        b.extend_from_slice(&padded(vec![0u8; 3 * 2 + 5]));
        let (d, mut ev) = eval(b);
        let heap = ev.node(&d, &[0, 1, 2, 1]).unwrap();
        assert_eq!(heap.size_bits, 5 * 8);
        // The second column is not there, and covers no bytes.
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 0, 1]).unwrap().size_bits, 0);
    }

    #[test]
    fn a_variable_length_column_is_a_count_and_where_in_the_heap_it_starts() {
        let mut b = primary();
        b.extend_from_slice(&table_header(&["TFIELDS =                    1", "TFORM1  = '1PJ(3)  '"], 1, 8, 12));
        let mut data = Vec::new();
        data.extend_from_slice(&3i32.to_be_bytes());
        data.extend_from_slice(&0i32.to_be_bytes());
        data.extend_from_slice(&[9u8; 12]);
        b.extend_from_slice(&padded(data));
        let (d, mut ev) = eval(b);
        let desc = ev.node(&d, &[0, 1, 2, 0, 0, 0, 0]).unwrap();
        assert_eq!(desc.type_name, "Array");
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 0, 0, 0, 0]).unwrap().value, Value::Int(3));
        assert_eq!(ev.node(&d, &[0, 1, 2, 1]).unwrap().size_bits, 12 * 8);
    }

    #[test]
    fn an_ascii_tables_columns_are_text_where_tbcol_puts_them() {
        let mut b = primary();
        let mut all: Vec<String> = vec![
            "XTENSION= 'TABLE   '           / ASCII table extension".into(),
            "BITPIX  =                    8".into(),
            "NAXIS   =                    2".into(),
            "NAXIS1  =                   16".into(),
            "NAXIS2  =                    2".into(),
            "PCOUNT  =                    0".into(),
            "GCOUNT  =                    1".into(),
            "TFIELDS =                    2".into(),
            "TBCOL1  =                    1".into(),
            "TFORM1  = 'I5      '".into(),
            "TBCOL2  =                    7".into(),
            "TFORM2  = 'F10.3   '".into(),
            "END".into(),
        ];
        let refs: Vec<&str> = all.iter_mut().map(|s| s.as_str()).collect();
        b.extend_from_slice(&header(&refs));
        let mut data = Vec::new();
        data.extend_from_slice(b"   12     1.500");
        data.push(b' ');
        data.extend_from_slice(b"   -7     0.250");
        data.push(b' ');
        b.extend_from_slice(&padded(data));
        let (d, mut ev) = eval(b);
        assert_eq!(ev.node(&d, &[0, 1, 2, 0]).unwrap().child_count, 2);
        assert_eq!(text(&ev.node(&d, &[0, 1, 2, 0, 0, 0, 0]).unwrap().value), "12");
        assert_eq!(text(&ev.node(&d, &[0, 1, 2, 0, 0, 1, 0]).unwrap().value), "1.500");
        assert_eq!(text(&ev.node(&d, &[0, 1, 2, 0, 1, 0, 0]).unwrap().value), "-7");
    }

    #[test]
    fn a_slash_inside_a_quoted_value_is_part_of_it() {
        let b = header(&[
            "SIMPLE  =                    T",
            "BITPIX  =                    8",
            "NAXIS   =                    0",
            "DATE    = '2026/09/02'         / date of observation",
            "OBJECT  = 'it''s here'         / an escaped quote",
            "END",
        ]);
        let (d, mut ev) = eval(b);
        assert_eq!(text(&ev.node(&d, &[0, 0, 0, 3, 1, 1, 1, 0, 0]).unwrap().value), "2026/09/02");
        // The comment is what is left of the card after the closing quote.
        assert!(text(&ev.node(&d, &[0, 0, 0, 3, 1, 2]).unwrap().value).starts_with("/ date"));
        // A `''` is one quote of the value, so the string runs past it.
        let parts = ev.node(&d, &[0, 0, 0, 4, 1, 1, 1]).unwrap();
        assert_eq!(parts.child_count, 2);
        assert_eq!(text(&ev.node(&d, &[0, 0, 0, 4, 1, 1, 1, 0, 0]).unwrap().value), "it");
        assert_eq!(text(&ev.node(&d, &[0, 0, 0, 4, 1, 1, 1, 1, 0]).unwrap().value), "s here");
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
