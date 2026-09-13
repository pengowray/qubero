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
//! The heap reads as those arrays. A `P` or `Q` column's cell is a descriptor,
//! a count and an offset into the heap, and the heap is a
//! [`Ty::Gather`](crate::template::Ty::Gather) over every descriptor in every
//! row: one element per descriptor, where it points, as many values as it
//! counts, of the type the letter after the `P` names. What no array claims is
//! a gap. A tile-compressed image is the common case, one row per tile and
//! each tile's compressed bytes an array in the heap.
//!
//! What the header says about each column is read once, into `columns`, and
//! the rows read it from there. That list covers no bytes: it is the cards
//! gathered into the shape a row is read through, and it is also where a
//! reader can see in one row why a column reads as it does.
//!
//! `TSCALn` and `TZEROn` say what the numbers in a column are worth: a stored
//! value `x` means `TZEROn + TSCALn * x`, and `BZERO` and `BSCALE` say the
//! same of an image's pixels. That sum is not worked out here and cannot be,
//! for the reason GRIB's packed values are not: both cards are reals and an
//! expression in this IR is an integer. So the two are shown beside the column
//! and the reader is told what they mean, and the bytes stay the integers they
//! are, editable as themselves.
//!
//! The one case where the standard means a type rather than a scaling is the
//! unsigned convention, and that is read. A column written signed with a zero
//! point of exactly half its range and no scaling *is* unsigned: `I` with
//! `TZERO` 32768 reads as `u16`, `J` with 2^31 as `u32`, `K` with 2^63 as
//! `u64`. `B` goes the other way, since FITS writes that one unsigned to begin
//! with: `TZERO` -128 reads it as `i8`. An image's pixels read the same way
//! from `BZERO` and `BITPIX`.
//!
//! A column is `col3` in every path and reads as `col3 flux` on the row: the
//! declared name is what an expression and an edit are written with, and the
//! word beside it is whatever the `TTYPE3` card says. A column's type comes
//! from the letter in its `TFORMn` as a letter, which is what a `Match` reading
//! its `on` as text from anywhere an expression reaches is for.
//!
//! What is not read here:
//!
//! - `TFORM1` through `TFORM32`, and `TBCOL1` through `TBCOL32`, since a
//!   keyword is looked up by the name written out here. A table with more
//!   columns than that reads the first 32 and says so in the row's last field.
//! - Which kind of table it is, is read from `TBCOL1` and `TFIELDS` rather
//!   than from `XTENSION`, which says so in text.
//! - A `TTYPEn` with an escaped quote in it reads as far as the quote. Nothing
//!   names a column that way.
//! - Heap arrays are found by walking every cell of every row, so a table of
//!   millions of rows takes that long before its heap has any children.
//! - A scaling that is not the unsigned convention. `TSCALn` and `TZEROn` are
//!   shown beside the column and the sum is left to the reader, since the two
//!   are reals and this IR's arithmetic is not. The same for `BSCALE` and
//!   `BZERO` on an image, and for a variable-length column, whose heap arrays
//!   are typed by the letter after the `P` and not scaled at all.
//! - A real written with no digits before the point, `TZERO1 = .5`, fails its
//!   card: the digits are read as a run that ends at the point, and a run with
//!   nothing in it is not a number. An exponent, `1.0E2`, reads as the 1 and
//!   leaves `E2` in the text after it, which is a value this declines to call
//!   a whole number rather than one it reads wrong.
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
//!   the tiles' descriptors and the heap is their compressed bytes, and
//!   nothing here inflates a tile with Rice or gzip into pixels.

use crate::template::{Anchor, Encoding, Endian::Big, Expr as E, Step, StrLen, Template, Ty as T, Until};

/// The block every part of a FITS file is padded out to.
const BLOCK: u32 = 2880;
/// One card image. Eight bytes of keyword, two of `= `, seventy of value.
const CARD: i128 = 80;

/// The keywords the standard says hold a whole number, which are the ones
/// read as one. Everything else in a header is text as far as the layout
/// goes, and reading it as a number would fail on the first `SIMPLE  = T`.
/// `EXTEND` is left out for the same reason from the other direction: it is a
/// logical, and `T` is not a number. The cards that hold a real have a shape
/// of their own, since a whole number written with a point after it is still a
/// whole number. See [`REAL`] and [`real_value`].
const NUMERIC: &[&str] = &[
    "BITPIX", "NAXIS", "NAXIS1", "NAXIS2", "NAXIS3", "NAXIS4", "NAXIS5", "NAXIS6", "NAXIS7", "NAXIS8", "NAXIS9",
    "PCOUNT", "GCOUNT", "TFIELDS", "THEAP", "EXTVER", "EXTLEVEL",
];

/// The keywords that say what a stored number is worth rather than holding a
/// number of their own. The standard makes every one of them a real, so a
/// decimal point in any of them is a value written the way the format allows
/// and not a card to fail. See [`real_value`].
const REAL: &[&str] = &["BSCALE", "BZERO"];

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

/// The eight bytes of a keyword read as one big-endian number, which is what
/// a card carries in `keynum` and what a lookup by a keyword worked out here
/// rather than written down compares against. See [`numbered_key`].
fn keynum(name: &str) -> i128 {
    keyword(name).iter().fold(0i128, |acc, b| (acc << 8) | i128::from(*b))
}

/// Somewhere inside the card whose keyword is `name`. Zero, or nothing at
/// all for text, when no card has that keyword or the card has no such part.
fn card_at(name: &str, field: &[&str]) -> E {
    E::tagged_bytes("cards", &["key"], &keyword(name), field)
}

/// The value of the card whose keyword is `name`, or zero when no card has it.
fn card_value(name: &str) -> E {
    card_at(name, &["body", "value"])
}

/// The keyword `prefix` followed by the number `n`, as the number a card's
/// `keynum` holds: `numbered_key("TFORM", 7)` is what `TFORM7  ` reads as.
///
/// The column and axis keywords carry their number in their name, so a
/// template that wants the seventh of them has either to write `TFORM7` out or
/// to work the keyword out where it asks. Writing them out is what fixed the
/// number of columns a table could have; this is the way past that, and it
/// costs an integer compare a card instead of a run of bytes compared a card.
///
/// Every prefix here is five bytes long, which leaves the three bytes a
/// keyword has left over for up to three digits. The digits are the number
/// picked apart with division, since the IR has no way to write a number as
/// text: `7` is `'7'`, a space and a space, `71` is `'7'`, `'1'` and a space.
fn numbered_key(prefix: &str, n: E) -> E {
    assert_eq!(prefix.len(), 5, "a numbered FITS keyword is five letters and up to three digits");
    // The five letters where a keyword writes them, with the three spaces it
    // is padded with taken off: the digits go where that padding was.
    let head = (keynum(prefix) >> 24) << 24;
    let digit = |v: E| v.add(E::lit(i128::from(b'0')));
    let blank = i128::from(b' ');
    let tens = n.clone().div(E::lit(10));
    let hundreds = n.clone().div(E::lit(100));
    let units = n.clone().sub(tens.clone().mul(E::lit(10)));
    let middle = tens.clone().sub(hundreds.clone().mul(E::lit(10)));
    let one = n.clone().less_than(E::lit(10));
    let two = E::lit(9).less_than(n.clone()).mul(n.clone().less_than(E::lit(100)));
    let three = E::lit(99).less_than(n.clone());
    let packed = |a: E, b: i128, c: i128| a.mul(E::lit(1 << 16)).add(E::lit((b << 8) | c));
    let short = one.mul(packed(digit(n), blank, blank));
    let medium = two.mul(digit(tens).mul(E::lit(1 << 16)).add(digit(units.clone()).mul(E::lit(1 << 8))).add(E::lit(blank)));
    let long = three.mul(digit(hundreds).mul(E::lit(1 << 16)).add(digit(middle).mul(E::lit(1 << 8))).add(digit(units)));
    E::lit(head).add(short).add(medium).add(long)
}

/// Somewhere inside the card whose keyword is `prefix` followed by `n`, where
/// `n` is worked out where the question is asked rather than written here.
fn numbered_at(prefix: &str, n: E, field: &[&str]) -> E {
    E::tagged_by_expr("cards", &["keynum"], numbered_key(prefix, n), field)
}

/// The text of the quoted value of the card whose keyword is `name`.
///
/// A quoted FITS value is a list of parts, because a `''` inside one is a
/// quote of the value rather than the end of it. A name with an escaped quote
/// in it is not a thing anyone writes, so this reads the first part, which for
/// every real card is the whole of the value.
fn card_text(name: &str) -> E {
    E::tagged_bytes("cards", &["key"], &keyword(name), &["body", "value", "parts", "0", "text"])
}

/// One card: its keyword, the `= ` that says it has a value, and the rest.
///
/// The card is a window of exactly eighty bytes, so the value's search for a
/// `/` stops at the end of the line rather than running into the next card.
fn card() -> T {
    let mut cases: Vec<(String, T)> = NUMERIC.iter().map(|k| ((*k).to_string(), numeric_body())).collect();
    for k in REAL {
        cases.push(((*k).to_string(), valued(real_value())));
    }
    for n in 1..=COLUMNS {
        // Where a column starts in a row of an ASCII table, which is a number
        // like any other, and what type it holds, which is not.
        cases.push((format!("TBCOL{n}"), numeric_body()));
        cases.push((format!("TFORM{n}"), valued(tform_value())));
        // What the numbers in a column are worth, which the standard lets a
        // writer put a decimal point in.
        cases.push((format!("TSCAL{n}"), valued(real_value())));
        cases.push((format!("TZERO{n}"), valued(real_value())));
    }
    let body = T::Match { on: E::field("key"), cases: cases.into(), default: std::sync::Arc::new(text_body()) };
    T::sized(
        E::lit(CARD),
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
        .counted_as("card"),
    )
}

/// A card whose value is a number: the digits, read as one. The value is the
/// digits after any spaces, ending at the space or the `/` that follows them,
/// and the rest of the line is the comment.
fn numeric_body() -> T {
    valued(T::decimal(StrLen::token(&[b' '], &[b' ', b'/'])))
}

/// A card whose value is a real number: the digits before the point, and the
/// digits after it as a number of their own.
///
/// `BSCALE`, `BZERO`, `TSCALn` and `TZEROn` are the cards that say what a
/// stored number is worth, and the standard writes all four as reals: `1`,
/// `1.0` and `1.0E0` are the same value and a file may hold any of them. They
/// cannot be read as one number here, since an expression in this IR is an
/// integer and there is no float to read digits into.
///
/// So they are read as two. The digits before the point are the value a whole
/// number has, and the digits after it are what says whether it is one: `1`,
/// `1.` and `1.00` all read as 1 with nothing after the point, and `1.5` reads
/// as 1 with a 5. That is the whole of what the unsigned convention needs to
/// be sure of, which is that a zero point is exactly 32768 and not nearly.
///
/// The run of digits before the point ends at the point, so the point is part
/// of it; what follows is read only when a digit follows, so `1.` has nothing
/// after it. An exponent ends both runs and stays in the text after them,
/// which reads `1.0E2` as 1 and not as 100. Nothing in the standard scales a
/// column by a power of ten written that way, and a value this cannot read as
/// a whole number is one it declines to call unsigned.
///
/// The card itself still reads as the text it is written as, and the two runs
/// of digits are a second reading of those same bytes, laid back over them
/// and counted nowhere. `TZERO3 = 0.4` on a card says 0.4, which is what the
/// file says; a reading that answered 0 there would be a number nobody wrote.
fn real_value() -> T {
    let digits = |ends: &[u8]| T::decimal(StrLen::token(&[b' ', b'+'], ends));
    let parts = T::inline_structure(
        "Parts",
        vec![
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
    let form = T::structure(
        "Binary column",
        vec![
            ("repeat", T::decimal(StrLen::Fixed(E::lit(d)))),
            ("code", T::text(StrLen::Fixed(E::lit(1)), Encoding::Ascii)),
            ("elem_code", elem_code),
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
            ("code", T::text(StrLen::Fixed(E::lit(1)), Encoding::Ascii)),
            ("width", T::decimal(StrLen::token(&[], &[b'.', b' ', b'\'']))),
            ("tail", T::text(StrLen::Fixed(E::to_bytes(b"/")), Encoding::Ascii)),
        ],
    )
}

/// What the header says about each column of a table, read once for the whole
/// data unit rather than once per cell.
///
/// A column is described by a handful of cards with its number in their names,
/// and before this every cell of every row walked the header for each of them:
/// a table of three thousand rows and twenty columns asked sixty thousand
/// questions of forty cards. This asks them once a column, and a cell reads
/// the answer out of this list by where it sits.
///
/// It covers no bytes. What it says is already in the file, a card at a time;
/// this is those cards gathered into the shape the rows are read through, and
/// a reader looking for why a column reads as it does can see the whole of the
/// answer on one row rather than hunting the header for five keywords.
fn columns() -> T {
    T::array(column(), card_value("TFIELDS").at_most(E::lit(COLUMNS as i128)))
}

/// One column of a table, as its cards describe it.
///
/// `scale` and `zero` are what the numbers in the column are worth: the
/// standard says a stored value `x` means `TZEROn + TSCALn * x`, and that is
/// as far as this goes, because the IR's arithmetic is over integers and both
/// of those are reals. So the two are shown beside the column and the reader
/// is told what they mean, the same way GRIB shows a packed value's reference
/// and scale without pretending to have the measurement. What is on disk stays
/// on disk, and stays editable as the integer it is.
///
/// The one case where the standard means something a type can say outright is
/// the unsigned convention: a column written as a signed integer with a zero
/// point of exactly half its range, and no scaling, *is* an unsigned column,
/// and an eight-bit column written unsigned with a zero point of -128 is a
/// signed one. `unsigned` is the answer to that, and a column it holds for is
/// read as the type the convention means rather than as the one the letter
/// names on its own.
fn column() -> T {
    let n = E::Idx.add(E::lit(1));
    let form = |part: &str| numbered_at("TFORM", n.clone(), &["body", "value", "form", part]);
    let real = |prefix: &str, part: &str| numbered_at(prefix, n.clone(), &["body", "value", "parts", part]);
    let written = |prefix: &str| numbered_at(prefix, n.clone(), &["body", "value", "text"]);
    // A zero point the convention gives a meaning to, by the letter the column
    // is written as: half the range of a signed type, or -128 for the one type
    // FITS writes unsigned to begin with.
    let want = |v: i128| T::computed(E::lit(v));
    let zero_want = T::matches(
        E::field("code"),
        vec![("B", want(-128)), ("I", want(32768)), ("J", want(2_147_483_648)), ("K", want(9_223_372_036_854_775_808))],
        want(0),
    );
    // Both worth-cards whole numbers and the scale exactly one: anything else
    // is a scaling, and a scaling is not a type.
    let whole = |prefix: &str| real(prefix, "frac").equals(E::lit(0));
    let unsigned = whole("TSCAL")
        .mul(whole("TZERO"))
        .mul(real("TSCAL", "int").or(E::lit(1)).equals(E::lit(1)))
        .mul(real("TZERO", "int").equals(E::field("zero_want")))
        .mul(E::lit(1).sub(E::field("zero_want").equals(E::lit(0))));
    T::inline_structure(
        "Column",
        vec![
            ("code", T::computed_text(form("code"))),
            ("repeat", T::computed(form("repeat").or(E::lit(1)))),
            // What the numbers in this column are worth, as the cards wrote
            // them: `TZERO3 = 0.4` says 0.4 here too. The sum is the reader's
            // to make, since these are reals and this IR has only integers.
            ("scale", T::computed_text(written("TSCAL"))),
            ("zero", T::computed_text(written("TZERO"))),
            // The letter after a `P` or a `Q`, which says what the heap array
            // a descriptor points at holds. Nothing for any other column.
            ("elem_code", T::computed_text(form("elem_code"))),
            // What an ASCII table's column is: how wide, and where in the row.
            ("width", T::computed(form("width"))),
            ("start", T::computed(numbered_at("TBCOL", n.clone(), &["body", "value"]))),
            ("zero_want", zero_want),
            ("unsigned", T::computed(unsigned)),
        ],
    )
    .machinery(&["elem_code", "width", "start", "zero_want", "unsigned"])
}

/// What column `n` of the table says about itself, from [`columns`].
fn column_says(n: usize, field: &str) -> E {
    E::elem_field("columns", E::lit(n as i128 - 1), &[field])
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
    // The unsigned convention, for an image: `BZERO` exactly half the range of
    // the type `BITPIX` names and `BSCALE` one means the pixels are unsigned,
    // and a `BZERO` of -128 on the one type FITS writes unsigned means signed.
    // See [`column`] for the same reading of a table's columns.
    let real = |name: &str, part: &str| card_at(name, &["body", "value", "parts", part]);
    let exact = real("BSCALE", "frac")
        .equals(E::lit(0))
        .mul(real("BZERO", "frac").equals(E::lit(0)))
        .mul(real("BSCALE", "int").or(E::lit(1)).equals(E::lit(1)));
    let swapped = |want: i128, ty: T, plain: T| {
        T::switch(exact.clone().mul(real("BZERO", "int").equals(E::lit(want))), vec![(1, of(ty))], of(plain))
    };
    T::switch(
        card_value("BITPIX"),
        vec![
            (8, swapped(-128, T::Int { bits: 8, endian: Big }, T::UInt { bits: 8, endian: Big })),
            (16, swapped(32768, T::UInt { bits: 16, endian: Big }, T::Int { bits: 16, endian: Big })),
            (32, swapped(2_147_483_648, T::UInt { bits: 32, endian: Big }, T::Int { bits: 32, endian: Big })),
            (64, swapped(9_223_372_036_854_775_808, T::UInt { bits: 64, endian: Big }, T::Int { bits: 64, endian: Big })),
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
    // A row of no width is no row at all: without the check, a table that
    // says so would lay a row over every byte of its heap.
    let any = E::lit(0).less_than(card_value("NAXIS1"));
    let rows = card_value("NAXIS2").mul(any).at_most(E::Remaining.div(width.clone()));
    T::structure(
        "Table",
        vec![
            // What the header said about each column, read once here rather
            // than again in every cell of every row. It covers no bytes; see
            // [`columns`].
            ("columns", columns()),
            ("rows", T::array(T::sized(width, row).counted_as("row"), rows)),
            ("heap", T::sized(E::Remaining, heap())),
        ],
    )
}

/// The heap a binary table keeps its variable-length arrays in: every array
/// some descriptor points at, each where its descriptor says.
///
/// The descriptors are in the rows, one in every cell of a `P` or `Q` column,
/// so the walk to them goes into every row and every column. A column of
/// anything else holds no descriptor: its cells are numbers, which the walk
/// passes without stepping into, or text, which is not a list at all. An
/// offset counts from the start of the heap, and the heap starts `THEAP` bytes
/// into the data, which is right after the rows unless a writer left room.
///
/// What no array claims is a gap. A heap is often exactly the arrays in it,
/// and a writer is free to leave room, or to have two descriptors share one
/// array, which reads as two elements over the same bytes.
fn heap() -> T {
    let from = vec![Step::field("rows"), Step::each(), Step::fields(&COL_NAMES), Step::each()];
    let starts = card_value("THEAP").or(card_value("NAXIS1").mul(card_value("NAXIS2")));
    T::gather(from, E::field("offset"), Anchor::Window, starts, heap_array())
}

/// One heap array: as many elements as its descriptor counts, of the type its
/// column's `TFORMn` names after the `P`. Never more than the heap has room
/// for, so a count that runs off the end of it shows what is there.
fn heap_array() -> T {
    let count = E::placer(E::field("count"));
    let of = |ty: T, width: i128| T::array(ty, count.clone().at_most(E::Remaining.div(E::lit(width))));
    let pair = |name: &str, ty: T| T::inline_structure(name, vec![("re", ty.clone()), ("im", ty)]);
    let text = T::text(StrLen::Fixed(count.clone().at_most(E::Remaining)), Encoding::Ascii);
    T::matches(
        E::placer(E::field("elem_code")),
        vec![
            ("L", text.clone()),
            ("X", T::bytes(count.clone().add(E::lit(7)).div(E::lit(8)).at_most(E::Remaining))),
            ("B", of(T::UInt { bits: 8, endian: Big }, 1)),
            ("I", of(T::Int { bits: 16, endian: Big }, 2)),
            ("J", of(T::Int { bits: 32, endian: Big }, 4)),
            ("K", of(T::Int { bits: 64, endian: Big }, 8)),
            ("A", text),
            ("E", of(T::F32(Big), 4)),
            ("D", of(T::F64(Big), 8)),
            ("C", of(pair("Complex", T::F32(Big)), 8)),
            ("M", of(pair("Complex", T::F64(Big)), 16)),
        ],
        // A letter nobody defined: the array is somewhere, and what is in it
        // is not something this can say.
        T::bytes(E::lit(0)),
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
    named_columns(T::structure("Row", fields))
}

/// Give every column the name its `TTYPEn` card holds. The declared name stays
/// `col3`, which is what a path and an expression are written with; the row
/// reads `col3 flux`. See [`crate::template::Field::name_from`].
fn named_columns(row: T) -> T {
    (1..=COLUMNS).fold(row, |row, n| row.field_named_from(COL_NAMES[n - 1], card_text(&format!("TTYPE{n}"))))
}

/// One when the table has an `n`th column.
fn has_column(n: usize) -> E {
    E::lit(n as i128 - 1).less_than(card_value("TFIELDS"))
}

/// One column of a binary table, as the type its `TFORMn` names and as many of
/// them as its repeat count says. A count is one when none is written.
///
/// The type is picked by the letter as a letter. `TFORMn` is a value written in
/// text and found by keyword, and a `Match` reads its `on` as text from
/// wherever an expression reaches it, so nothing here has to turn `J` into 74
/// and back.
fn binary_column(n: usize) -> T {
    let code = column_says(n, "code");
    let r = column_says(n, "repeat");
    // Never more than the row has room for: a row whose columns do not add up
    // to `NAXIS1` shows the ones that fit rather than failing.
    let of = |ty: T, width: i128| T::array(ty, r.clone().at_most(E::Remaining.div(E::lit(width))));
    let pair = |name: &str, ty: T| T::inline_structure(name, vec![("re", ty.clone()), ("im", ty)]);
    // A column the unsigned convention gives another type to is read as that
    // type: the bytes are the same bytes and the numbers in them are what the
    // header said they were. See [`column`].
    let swapped = |want: T, plain: T, width: i128| {
        T::switch(column_says(n, "unsigned"), vec![(1, of(want, width))], of(plain, width))
    };
    // A descriptor also says what its heap array holds, read from this
    // column's `TFORMn`. The array is in the heap, which is not inside the
    // column and cannot find the card for it: the letter is asked here, where
    // the column number is known, and the heap asks the descriptor.
    let elem_code = T::computed_text(column_says(n, "elem_code"));
    let descriptor = |name: &str, ty: T| {
        T::inline_structure(name, vec![("count", ty.clone()), ("offset", ty), ("elem_code", elem_code.clone())])
    };
    let text = T::text(StrLen::Fixed(r.clone().at_most(E::Remaining)), Encoding::Ascii);
    T::matches(
        code,
        vec![
            // A logical is written as the letter `T` or `F`, or as a zero byte
            // for a value nobody set.
            ("L", text.clone()),
            // A bit column is that many bits, rounded up to whole bytes.
            ("X", T::bytes(r.clone().add(E::lit(7)).div(E::lit(8)).at_most(E::Remaining))),
            ("B", swapped(T::Int { bits: 8, endian: Big }, T::UInt { bits: 8, endian: Big }, 1)),
            ("I", swapped(T::UInt { bits: 16, endian: Big }, T::Int { bits: 16, endian: Big }, 2)),
            ("J", swapped(T::UInt { bits: 32, endian: Big }, T::Int { bits: 32, endian: Big }, 4)),
            ("K", swapped(T::UInt { bits: 64, endian: Big }, T::Int { bits: 64, endian: Big }, 8)),
            ("A", text),
            ("E", of(T::F32(Big), 4)),
            ("D", of(T::F64(Big), 8)),
            ("C", of(pair("Complex", T::F32(Big)), 8)),
            ("M", of(pair("Complex", T::F64(Big)), 16)),
            // A variable-length array is written as how many there are and
            // where in the heap they start.
            ("P", of(descriptor("Descriptor", T::Int { bits: 32, endian: Big }), 8)),
            ("Q", of(descriptor("Descriptor", T::Int { bits: 64, endian: Big }), 16)),
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
        let width = column_says(n, "width").at_least(E::lit(1)).at_most(card_value("NAXIS1").at_least(E::lit(1)));
        let at = column_says(n, "start").sub(E::lit(1)).at_least(E::lit(0));
        let cell = T::at_in_window(at, T::text(StrLen::Fixed(width), Encoding::Ascii));
        fields.push((name, T::present_if(has_column(n), cell)));
    }
    named_columns(T::structure("Row", fields))
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
    use crate::eval::{Evaluator, Role, Value};
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
        let key = ev.node(&d, &[0, 0, 0, 1, 1]).unwrap();
        assert_eq!(key.value, Value::Str("BITPIX".into()));
        assert_eq!(ev.node(&d, &[0, 0, 0, 1, 2, 1]).unwrap().value, Value::Int(16));
        let comment = ev.node(&d, &[0, 0, 0, 1, 2, 2]).unwrap();
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
        assert_eq!(text(&ev.node(&d, &[0, 0, 0, 3, 2, 1]).unwrap().value), "T");
        // And `END` has no value at all, so the card is one run of text.
        let end = ev.node(&d, &[0, 0, 0, 4, 2]).unwrap();
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
        let kind = ev.node(&d, &[0, 1, 0, 0, 2, 1]).unwrap();
        assert_eq!(kind.type_name, "Text");
        // The quote that opens the string, and the one part it is written in.
        assert_eq!(text(&ev.node(&d, &[0, 1, 0, 0, 2, 1, 1, 0, 0]).unwrap().value), "BINTABLE");
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
        let rows = ev.node(&d, &[0, 1, 2, 1]).unwrap();
        assert_eq!(rows.child_count, 2);
        // Row 1, column 1: one 32-bit integer.
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 1, 0, 0]).unwrap().value, Value::Int(2));
        // Column 2 is two floats, and the second of them is the second value.
        let flux = ev.node(&d, &[0, 1, 2, 1, 0, 1]).unwrap();
        assert_eq!((flux.type_name.as_str(), flux.child_count), ("f32 be[]", 2));
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 1, 1]).unwrap().value, Value::Float(-0.25));
        // Column 3 is five characters, read as one run of text.
        assert_eq!(text(&ev.node(&d, &[0, 1, 2, 1, 0, 2]).unwrap().value), "abcde");
        // A `TFORMn` with no repeat count means one.
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 3, 0]).unwrap().value, Value::Float(2.5));
        // A row is as wide as `NAXIS1` says.
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0]).unwrap().size_bits, 25 * 8);
        // Every column reads under the name its `TTYPEn` card gives it, with
        // the declared name kept in front: that is the one a path is written
        // with, and it does not move when the header is edited.
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 0]).unwrap().name, "col1 counts");
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 1]).unwrap().name, "col2 flux");
        // A column with no `TTYPEn` keeps the name the template gave it.
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 3]).unwrap().name, "col4");
        // And the row says which card the name came from.
        let seen: Vec<_> =
            ev.origins(&d, &[0, 1, 2, 1, 0, 1]).unwrap().into_iter().map(|o| (o.role, o.value)).collect();
        assert!(seen.iter().any(|(r, v)| *r == Role::Name && v.trim() == "flux"), "{seen:?}");
        // The type letter is a letter. It used to read as 74, the number `J`
        // is in ASCII, because the type was picked by a number.
        let n = ev.node(&d, &[0, 1, 0]).unwrap().child_count;
        let card = (0..n as usize)
            .find(|i| ev.node(&d, &[0, 1, 0, *i]).unwrap().name.contains("TFORM1"))
            .expect("a TFORM1 card");
        // The card is a keyword and a body; the body is `= `, the TFORM value
        // and the comment, and the value is a quote and the form inside it.
        assert_eq!(text(&ev.node(&d, &[0, 1, 0, card, 2, 1, 1, 1]).unwrap().value), "J");
    }

    #[test]
    fn a_column_past_tfields_covers_nothing_and_the_heap_is_what_is_left() {
        let mut b = primary();
        b.extend_from_slice(&table_header(&["TFIELDS =                    1", "TFORM1  = '1I      '"], 3, 2, 5));
        b.extend_from_slice(&padded(vec![0u8; 3 * 2 + 5]));
        let (d, mut ev) = eval(b);
        let heap = ev.node(&d, &[0, 1, 2, 2]).unwrap();
        assert_eq!(heap.size_bits, 5 * 8);
        // The second column is not there, and covers no bytes.
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 1]).unwrap().size_bits, 0);
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
        let desc = ev.node(&d, &[0, 1, 2, 1, 0, 0, 0]).unwrap();
        assert_eq!(desc.type_name, "Descriptor");
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 0, 0, 0]).unwrap().value, Value::Int(3));
        // The letter after the `P` says what the array holds, and the
        // descriptor carries it so the heap can ask.
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 0, 0, 2]).unwrap().value, Value::Str("J".into()));
        assert_eq!(ev.node(&d, &[0, 1, 2, 2]).unwrap().size_bits, 12 * 8);
        // And the heap is that array: three 32-bit integers where the
        // descriptor pointed, which is the front of the heap.
        let heap = ev.node(&d, &[0, 1, 2, 2]).unwrap();
        assert_eq!(heap.child_count, 1);
        let array = ev.node(&d, &[0, 1, 2, 2, 0]).unwrap();
        assert_eq!((array.type_name.as_str(), array.child_count), ("i32 be[]", 3));
        assert_eq!((array.offset_bits, array.size_bits), (heap.offset_bits, 12 * 8));
        assert_eq!(ev.node(&d, &[0, 1, 2, 2, 0, 2]).unwrap().value, Value::Int(0x0909_0909));
    }

    /// A table of `rows` rows of the columns `forms` names, whose cells are
    /// the descriptors `cells` gives row by row, and whose heap is `heap`.
    fn heap_table(forms: &[&str], cells: &[&[(i32, i32)]], heap: &[u8]) -> Vec<u8> {
        let mut cards = vec![format!("TFIELDS = {:20}", forms.len())];
        for (i, f) in forms.iter().enumerate() {
            cards.push(format!("TFORM{}  = '{f:<8}'", i + 1));
        }
        let refs: Vec<&str> = cards.iter().map(|s| s.as_str()).collect();
        let mut b = primary();
        b.extend_from_slice(&table_header(&refs, cells.len(), 8 * forms.len(), heap.len()));
        let mut data = Vec::new();
        for row in cells {
            for (count, offset) in row.iter() {
                data.extend_from_slice(&count.to_be_bytes());
                data.extend_from_slice(&offset.to_be_bytes());
            }
        }
        data.extend_from_slice(heap);
        b.extend_from_slice(&padded(data));
        b
    }

    #[test]
    fn an_empty_cell_covers_no_heap() {
        let b = heap_table(&["1PB"], &[&[(0, 0)], &[(2, 0)]], &[5, 6]);
        let (d, mut ev) = eval(b);
        let heap = [0, 1, 2, 2];
        assert_eq!(ev.node(&d, &heap).unwrap().child_count, 2);
        // The first row's array is there, and has nothing in it.
        let empty = ev.node(&d, &[0, 1, 2, 2, 0]).unwrap();
        assert_eq!((empty.child_count, empty.size_bits), (0, 0));
        let full = ev.node(&d, &[0, 1, 2, 2, 1]).unwrap();
        assert_eq!((full.child_count, full.size_bits), (2, 16));
        assert_eq!(ev.node(&d, &[0, 1, 2, 2, 1, 1]).unwrap().value, Value::UInt(6));
    }

    #[test]
    fn two_variable_columns_in_one_row_both_reach_the_heap() {
        // Three bytes for the first column and two 16-bit numbers for the
        // second, one after the other in the heap.
        let b = heap_table(&["1PB", "1PI"], &[&[(3, 0), (2, 3)]], &[1, 2, 3, 0x12, 0x34, 0xff, 0xfe]);
        let (d, mut ev) = eval(b);
        assert_eq!(ev.node(&d, &[0, 1, 2, 2]).unwrap().child_count, 2);
        let bytes = ev.node(&d, &[0, 1, 2, 2, 0]).unwrap();
        let words = ev.node(&d, &[0, 1, 2, 2, 1]).unwrap();
        assert_eq!((bytes.type_name.as_str(), bytes.child_count), ("u8[]", 3));
        assert_eq!((words.type_name.as_str(), words.child_count), ("i16 be[]", 2));
        assert_eq!(words.offset_bits, bytes.offset_bits + 3 * 8);
        assert_eq!(ev.node(&d, &[0, 1, 2, 2, 1, 1]).unwrap().value, Value::Int(-2));
        // Each says which cell put it there: the second column of the only row.
        let placed = ev.origins(&d, &[0, 1, 2, 2, 1]).unwrap();
        assert_eq!(placed[0].label, "rows[0].col2[0]");
        assert_eq!(placed[0].path, vec![0, 1, 2, 1, 0, 1, 0]);
    }

    #[test]
    fn heap_bytes_no_array_claims_are_a_gap() {
        // Ten bytes of heap; the arrays take the first three and two more
        // after a gap, and leave the end over.
        let b = heap_table(&["1PB"], &[&[(3, 0)], &[(2, 6)]], &[1, 2, 3, 0, 0, 0, 7, 8, 0, 0]);
        let (d, mut ev) = eval(b);
        let heap = ev.node(&d, &[0, 1, 2, 2]).unwrap();
        let start = heap.offset_bits;
        let spans = ev.spans(&d, start, start + heap.size_bits, 100).unwrap();
        // A short array is a row per value, as a short run always is, so what
        // is compared is which bytes are covered and which are gaps.
        let gaps: Vec<(u64, u64)> =
            spans.iter().filter(|s| s.gap).map(|s| ((s.offset_bits - start) / 8, s.size_bits / 8)).collect();
        assert_eq!(gaps, vec![(3, 3), (8, 2)]);
        let covered: Vec<u64> = spans.iter().filter(|s| !s.gap).map(|s| (s.offset_bits - start) / 8).collect();
        assert_eq!(covered, vec![0, 1, 2, 6, 7]);
        // And the cursor in a gap stands on the heap itself.
        assert_eq!(ev.locate(&d, start + 4 * 8).unwrap(), vec![0, 1, 2, 2]);
        assert_eq!(ev.locate(&d, start + 7 * 8).unwrap(), vec![0, 1, 2, 2, 1, 1]);
    }

    /// A column's keywords are worked out where they are asked rather than
    /// written out here, so a column past the ninth is found by the same
    /// arithmetic: `TZERO12` is a number this builds, not a name in the
    /// template.
    #[test]
    fn a_column_number_of_two_digits_is_found_by_the_keyword_it_works_out() {
        let mut cards = vec!["TFIELDS =                   12".to_string()];
        for n in 1..=12 {
            cards.push(format!("{:<8}= '1I      '", format!("TFORM{n}")));
        }
        cards.push("TZERO12 =                32768".into());
        let refs: Vec<&str> = cards.iter().map(|s| s.as_str()).collect();
        let mut b = primary();
        b.extend_from_slice(&table_header(&refs, 1, 24, 0));
        b.extend_from_slice(&padded(vec![0xff; 24]));
        let (d, mut ev) = eval(b);
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 11]).unwrap().type_name, "u16 be[]");
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 10]).unwrap().type_name, "i16 be[]");
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 0]).unwrap().type_name, "i16 be[]");
    }

    /// A column of signed 16-bit integers with a zero point of exactly 32768
    /// is an unsigned 16-bit column: that is what the convention means, so
    /// that is what it reads as. The bytes are the bytes either way.
    #[test]
    fn a_zero_point_of_half_the_range_reads_the_column_as_unsigned() {
        let cards = [
            "TFIELDS =                    2",
            "TFORM1  = '1I      '",
            "TZERO1  =                32768",
            "TSCAL1  =                    1",
            "TFORM2  = '1I      '",
        ];
        let mut b = primary();
        b.extend_from_slice(&table_header(&cards, 1, 4, 0));
        // 0xffff: 65535 read as the unsigned column it is, -1 read as the
        // signed one the letter alone would name.
        b.extend_from_slice(&padded(vec![0xff, 0xff, 0xff, 0xff]));
        let (d, mut ev) = eval(b);
        let unsigned = ev.node(&d, &[0, 1, 2, 1, 0, 0]).unwrap();
        assert_eq!(unsigned.type_name, "u16 be[]");
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 0, 0]).unwrap().value, Value::UInt(65535));
        let signed = ev.node(&d, &[0, 1, 2, 1, 0, 1]).unwrap();
        assert_eq!(signed.type_name, "i16 be[]");
        assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 1, 0]).unwrap().value, Value::Int(-1));
        // And what the header said the numbers are worth is on one row beside
        // the column rather than spread over the cards.
        assert_eq!(text(&ev.node(&d, &[0, 1, 2, 0, 0, 2]).unwrap().value), "1");
        assert_eq!(text(&ev.node(&d, &[0, 1, 2, 0, 0, 3]).unwrap().value), "32768");
    }

    /// A zero point written with a point after it is the same whole number;
    /// one with a fraction after the point is a scaling, and a scaling is not
    /// a type.
    #[test]
    fn a_zero_point_that_is_not_a_whole_number_is_a_scaling_and_not_a_type() {
        let table = |zero: &str| {
            let card = format!("TZERO1  = {zero:>20}");
            let cards = ["TFIELDS =                    1", "TFORM1  = '1I      '", card.as_str()];
            let mut b = primary();
            b.extend_from_slice(&table_header(&cards, 1, 2, 0));
            b.extend_from_slice(&padded(vec![0xff, 0xff]));
            b
        };
        for written in ["32768", "32768.", "32768.0", "32768.00"] {
            let (d, mut ev) = eval(table(written));
            assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 0]).unwrap().type_name, "u16 be[]", "{written}");
        }
        for written in ["32768.5", "32767", "-32768"] {
            let (d, mut ev) = eval(table(written));
            assert_eq!(ev.node(&d, &[0, 1, 2, 1, 0, 0]).unwrap().type_name, "i16 be[]", "{written}");
        }
    }

    /// An image says the same thing with `BZERO`, and `BITPIX` 8 says it the
    /// other way round: FITS writes that one unsigned, so a zero point of
    /// -128 is how a file writes signed bytes.
    #[test]
    fn an_images_zero_point_reads_its_pixels_as_the_type_the_convention_means() {
        let pixels = |bitpix: i32, zero: &str, bytes: Vec<u8>| {
            let mut b = header(&[
                "SIMPLE  =                    T",
                &format!("BITPIX  = {bitpix:20}"),
                "NAXIS   =                    1",
                "NAXIS1  =                    1",
                &format!("BZERO   = {zero:>20}"),
                "END",
            ]);
            b.extend_from_slice(&padded(bytes));
            b
        };
        let (d, mut ev) = eval(pixels(16, "32768", vec![0xff, 0xff]));
        assert_eq!(ev.node(&d, &[0, 0, 2]).unwrap().type_name, "u16 be[]");
        assert_eq!(ev.node(&d, &[0, 0, 2, 0]).unwrap().value, Value::UInt(65535));
        let (d, mut ev) = eval(pixels(8, "-128", vec![0xff]));
        assert_eq!(ev.node(&d, &[0, 0, 2]).unwrap().type_name, "i8[]");
        assert_eq!(ev.node(&d, &[0, 0, 2, 0]).unwrap().value, Value::Int(-1));
        // And a zero point that is not the convention's leaves the pixels as
        // the type BITPIX names.
        let (d, mut ev) = eval(pixels(16, "100", vec![0xff, 0xff]));
        assert_eq!(ev.node(&d, &[0, 0, 2]).unwrap().type_name, "i16 be[]");
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
        assert_eq!(ev.node(&d, &[0, 1, 2, 1]).unwrap().child_count, 2);
        assert_eq!(text(&ev.node(&d, &[0, 1, 2, 1, 0, 0, 0]).unwrap().value), "12");
        assert_eq!(text(&ev.node(&d, &[0, 1, 2, 1, 0, 1, 0]).unwrap().value), "1.500");
        assert_eq!(text(&ev.node(&d, &[0, 1, 2, 1, 1, 0, 0]).unwrap().value), "-7");
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
        assert_eq!(text(&ev.node(&d, &[0, 0, 0, 3, 2, 1, 1, 0, 0]).unwrap().value), "2026/09/02");
        // The comment is what is left of the card after the closing quote.
        assert!(text(&ev.node(&d, &[0, 0, 0, 3, 2, 2]).unwrap().value).starts_with("/ date"));
        // A `''` is one quote of the value, so the string runs past it.
        let parts = ev.node(&d, &[0, 0, 0, 4, 2, 1, 1]).unwrap();
        assert_eq!(parts.child_count, 2);
        assert_eq!(text(&ev.node(&d, &[0, 0, 0, 4, 2, 1, 1, 0, 0]).unwrap().value), "it");
        assert_eq!(text(&ev.node(&d, &[0, 0, 0, 4, 2, 1, 1, 1, 0]).unwrap().value), "s here");
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
