//! A FITS header card: its keyword, and a body picked by that keyword that
//! reads the value as a whole number, a real, a logical, a quoted string, a
//! `CONTINUE` piece or a `TFORMn`.
//!
//! Apart from [`fits`](super::fits) because reading one eighty-byte card is a
//! job of its own. Nothing here knows about rows, cells, the heap or axes, and
//! those only ask the cards for values by keyword; `fits.rs` keeps the lookups
//! that do the asking.

use super::fits::keynum;
use crate::template::{Encoding, Endian::Big, Expr as E, StrLen, Ty as T, Until};

/// One card image. Eight bytes of keyword, two of `= `, seventy of value.
const CARD: i128 = 80;

/// The keywords the standard says hold a whole number, which are the ones
/// read as one. Everything else in a header is text as far as the layout
/// goes, and reading it as a number would fail on the first `SIMPLE  = T`.
/// `EXTEND` is left out for the same reason from the other direction: it is a
/// logical, and `T` is not a number. The cards that hold a real have a shape
/// of their own, since a whole number written with a point after it is still a
/// whole number. See [`REAL`] and [`real_value`].
const NUMERIC: &[&str] = &["BITPIX", "PCOUNT", "GCOUNT", "TFIELDS", "THEAP", "EXTVER", "EXTLEVEL", "ZBITPIX", "ZDITHER0"];

/// The logicals that decide how the data is read, which are read as the one
/// letter they hold rather than as the run of blanks and a letter a text
/// value is. `ZIMAGE = T` is what says a binary table is a compressed image,
/// and a [`T::Match`] compares whole text, so twenty blanks in front of the
/// `T` would match nothing. `SIMPLE` and `EXTEND` decide nothing here and are
/// left as the text they were.
const LOGICAL: &[&str] = &["ZIMAGE"];

/// The keywords that say what a stored number is worth rather than holding a
/// number of their own. The standard makes every one of them a real, so a
/// decimal point in any of them is a value written the way the format allows
/// and not a card to fail. See [`real_value`].
const REAL: &[&str] = &["BSCALE", "BZERO"];

/// The five-letter keywords a column or an axis carries its number in. Each
/// of them takes a body of its own, chosen by the five bytes the keyword opens
/// with rather than by the whole keyword, which is what lets a table have as
/// many columns as the standard allows. See [`numbered_key`](super::fits::numbered_key).
const NUMBERED: &[(&str, fn() -> T)] = &[
    // How many along an axis, and where a column of an ASCII table starts in
    // a row: numbers like any other.
    ("NAXIS", numeric_body),
    ("TBCOL", numeric_body),
    // The same two things said of a compressed image: how many pixels along
    // each of its axes, and how many along each axis of a tile. `ZNAXIS`
    // is six letters, so it is told apart by the five it opens with, and
    // every keyword that opens with those five is a whole number: `ZNAXIS`
    // itself and `ZNAXIS1` onwards.
    ("ZNAXI", numeric_body),
    ("ZTILE", numeric_body),
    // What a column holds, which is not a number.
    ("TFORM", tform_body),
    // What the numbers in a column are worth, which the standard lets a
    // writer put a decimal point in.
    ("TSCAL", real_body),
    ("TZERO", real_body),
];

/// One card: its keyword, the `= ` that says it has a value, and the rest.
///
/// The card is a window of exactly eighty bytes, so the value's search for a
/// `/` stops at the end of the line rather than running into the next card.
pub(super) fn card() -> T {
    // A keyword that carries a number is told from the five letters it opens
    // with, before the digits. Writing `TFORM1` through `TFORM999` out as
    // cases is what fixed how many columns a table could have.
    let by_prefix: Vec<(i128, T)> =
        NUMBERED.iter().map(|(prefix, body)| (keynum(prefix) >> 24, card_of(body()))).collect();
    T::sized(E::lit(CARD), T::switch(E::peek(64, Big).div(E::lit(1 << 24)), by_prefix, card_of(named_body())))
}

/// A card with this body: the keyword, the keyword as a number, and the rest.
fn card_of(body: T) -> T {
    T::structure_named(
        "Card",
        "key",
        "body",
        vec![
            // The keyword as one number, so that a lookup may work out the
            // keyword it is after. It takes none of the card: the eight
            // bytes it reads are the ones `key` reads. See [`numbered_key`].
            ("keynum", T::computed(E::peek(64, Big))),
            ("key", T::text(StrLen::Padded { size: E::lit(8), pad: b' ' }, Encoding::Ascii)),
            // Not every card has one: `END` and the comment keywords
            // leave these two bytes as part of the text.
            ("body", body),
        ],
    )
    .machinery(&["keynum"])
    .counted_as("card")
}

/// The body of a card whose whole keyword the standard fixed, picked by that
/// keyword. Everything else in a header is text.
fn named_body() -> T {
    let mut cases: Vec<(String, T)> = NUMERIC.iter().map(|k| ((*k).to_string(), numeric_body())).collect();
    cases.extend(REAL.iter().map(|k| ((*k).to_string(), real_body())));
    cases.extend(LOGICAL.iter().map(|k| ((*k).to_string(), logical_body())));
    cases.push(("CONTINUE".to_string(), continue_body()));
    T::Match { on: E::field("key"), cases: cases.into(), default: std::sync::Arc::new(text_body()) }
}

/// A card whose value is a `TFORMn`, and one whose value is a real.
fn tform_body() -> T {
    valued(tform_value())
}
fn real_body() -> T {
    valued(real_value())
}

/// A card whose value is a number: the digits, read as one. The value is the
/// digits after any spaces, ending at the space or the `/` that follows them,
/// and the rest of the line is the comment.
fn numeric_body() -> T {
    valued(T::decimal(StrLen::token(&[b' '], &[b' ', b'/'])))
}

/// A card whose value is a logical: the `T` or `F` after any blanks, read as
/// that one letter, and the rest of the line as the comment. See [`LOGICAL`].
fn logical_body() -> T {
    valued(T::text(StrLen::token(&[b' '], &[b' ', b'/']), Encoding::Ascii))
}

/// A card whose value is a real number: the digits before the point, and the
/// digits after it as a number of their own.
///
/// `BSCALE`, `BZERO`, `TSCALn` and `TZEROn` are the cards that say what a
/// stored number is worth, and the standard writes all four as reals: `1`,
/// `1.0` and `1.0E0` are the same value and a file may hold any of them. The
/// value is the text, and what a scale is worth is the real that text spells,
/// read by `real(...)` where the worth is worked out.
///
/// The two runs of digits are for deciding which way to work it out. A whole
/// number with no exponent is summed in whole numbers, which is exact at any
/// size, and anything else in reals. The digits before the point are the value
/// a whole number has, and the digits after it are what says whether it is
/// one: `1`, `1.` and `1.00` all read as 1 with nothing after the point, and
/// `1.5` reads as 1 with a 5. That is what the unsigned convention on a 64-bit
/// column needs to be sure of, which is that a zero point is exactly 2^63 and
/// not the nearest double to it.
///
/// The run of digits before the point ends at the point, so the point is part
/// of it; what follows is read only when a digit follows, so `1.` has nothing
/// after it. An exponent ends both runs and stays in the text after them, and
/// `exp` says it is there, which sends the sum to reals.
///
/// The card itself still reads as the text it is written as, and the two runs
/// of digits are a second reading of those same bytes, laid back over them
/// and counted nowhere. `TZERO3 = 0.4` on a card says 0.4, which is what the
/// file says; a reading that answered 0 there would be a number nobody wrote.
fn real_value() -> T {
    let digits = |ends: &[u8]| T::decimal(StrLen::token(&[b' ', b'+'], ends));
    // Whether the value carries an exponent, which is a power of ten this
    // cannot multiply by: the digits before and after the point are read
    // either way, and `exp` is what says they are not the whole value. The
    // window this sits in ends at the comment, so a letter found inside it is
    // the value's own and not a word from the comment.
    let letter = |c: &[u8]| E::to_bytes(c).less_than(E::Remaining);
    let exp = letter(b"E").or(letter(b"e")).or(letter(b"D")).or(letter(b"d"));
    let parts = T::inline_structure(
        "Parts",
        vec![
            ("exp", T::computed(exp)),
            ("int", digits(&[b' ', b'/', b'.', b'E', b'e', b'D', b'd'])),
            ("frac", T::present_if(digit_peek(0), digits(&[b' ', b'/', b'E', b'e', b'D', b'd']))),
        ],
    );
    T::sized(
        E::to_bytes(b"/"),
        T::structure(
            "Real",
            vec![
                ("text", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
                ("parts", T::at_in_window(E::lit(0), parts)),
            ],
        )
        .field_aside("parts")
        .machinery(&["parts"])
        .payload(&["text"]),
    )
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

/// A `CONTINUE` card: the rest of a string too long for one card.
///
/// A string value that ends in `&` is not finished. The card after it writes
/// `CONTINUE` where a keyword goes, leaves out the `= ` that says a value
/// follows, and holds the next piece of the string as a quoted value of its
/// own, with its own `&` when a third card follows. The file keeps a long
/// value in pieces because a card is eighty columns and a filename is not.
///
/// So a `CONTINUE` card reads as the string it holds, with the `&` left where
/// the file put it and the comment after it read as one. Without this it read
/// as one undifferentiated run of text, quotes and all.
///
/// A card that says `CONTINUE` and holds no string is left as the text it is.
fn continue_body() -> T {
    let quoted = T::structure(
        "Continued",
        vec![
            ("lead", T::text(StrLen::Fixed(E::to_bytes(b"'")), Encoding::Ascii)),
            ("value", quoted_value()),
            ("comment", T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii)),
        ],
    )
    .machinery(&["lead"])
    .payload(&["value"]);
    T::switch(E::to_bytes(b"'").less_than(E::Remaining), vec![(1, quoted)], text_body())
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
/// how many digits are after the point. Both leave the same three fields to
/// look up by name: `repeat`, `code` and `width`.
///
/// Which of the two a card holds is read from where its digits are. A digit
/// before the letter is a repeat count, so the card is a binary table's. A
/// digit after the letter is a width, so it is an ASCII table's. A letter with
/// no digits at all is a binary table's with the count left out, which the
/// standard allows and means one: `TFORM1 = 'I'` is one 16-bit integer, and an
/// ASCII column with no width is a card nobody can read either way.
///
/// The type letter is read as the number its ASCII is, since that is what the
/// column's own type switches on, and nothing in the IR switches a type on
/// text found by keyword.
fn tform_value() -> T {
    // No leading digit: a width after the letter says ASCII, and nothing
    // after it says a binary column whose count was left out.
    let by_letter = T::switch(digit_peek(1), vec![(1, ascii_form())], binary_form(None));
    T::structure(
        "TFORM",
        vec![
            ("open", T::text(StrLen::Fixed(E::lit(1)), Encoding::Ascii)),
            ("form", T::switch(digit_peek(0), vec![(1, digits_then(1, 5))], by_letter)),
        ],
    )
    .machinery(&["open"])
}

/// The fields of a binary table's `TFORMn`. `digits` is how many digits the
/// repeat count is written in, or nothing at all for a card that left the
/// count out, where the count reads as the one the standard says it means.
fn binary_form(digits: Option<i128>) -> T {
    // A variable-length column writes the type of what its arrays hold as a
    // second letter, `1PB`: `P` says the cell is a descriptor and `B` says the
    // heap array it points at is bytes. Every other column has no second
    // letter, and this is nothing there.
    let letter = T::text(StrLen::Fixed(E::lit(1)), Encoding::Ascii);
    let elem_code = T::switch(
        E::field("code"),
        vec![(b'P' as i128, letter.clone()), (b'Q' as i128, letter)],
        T::bytes(E::lit(0)),
    );
    let repeat = match digits {
        Some(d) => T::decimal(StrLen::Fixed(E::lit(d))),
        None => T::computed(E::lit(1)),
    };
    T::structure(
        "Binary column",
        vec![
            ("repeat", repeat),
            ("code", T::text(StrLen::Fixed(E::lit(1)), Encoding::Ascii)),
            ("elem_code", elem_code),
            ("tail", T::text(StrLen::Fixed(E::to_bytes(b"/")), Encoding::Ascii)),
        ],
    )
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
    let form = binary_form(Some(d));
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
            ("code", T::text(StrLen::Fixed(E::lit(1)), Encoding::Ascii)),
            ("width", T::decimal(StrLen::token(&[], &[b'.', b' ', b'\'']))),
            ("tail", T::text(StrLen::Fixed(E::to_bytes(b"/")), Encoding::Ascii)),
        ],
    )
}
