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
//! The axes are a list of as many as `NAXIS` says, read from the cards that
//! number them, so a tenth axis is read like the nine before it. An axis of
//! zero is read as a zero, which says there is no data; the first axis is the
//! exception, since `NAXIS1 = 0` is how a random-groups file says the group
//! parameters are all there is.
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
//! same of an image's pixels. When both cards are whole numbers that sum is
//! worked out, and each number reads as the integer on disk with what it is
//! worth beside it. The integer on disk is what an edit writes; the worth
//! takes no bits, the way a Steim word's differences hang off the word in
//! `mseed`.
//!
//! The commonest case by far is the unsigned convention: an unsigned 16-bit
//! column is written as a signed one with a zero point of 32768, so 65535 is
//! on disk as the signed 32767. That is a bias and not a reinterpretation, and
//! reading those bytes as a 16-bit unsigned number would answer 32767, which
//! is a number nobody wrote. The sum is the reading.
//!
//! A scale or a zero point with a fraction in it is not worked out and cannot
//! be, for the reason GRIB's packed values are not: an expression in this IR
//! is an integer. Those are shown beside the column, as the cards wrote them,
//! and the sum is the reader's to make.
//!
//! A row is a list of cells rather than a field per column, so a table may
//! have as many columns as the standard allows, which is 999: a keyword is
//! eight bytes and `TFORM` spends five of them. A cell is `cells[2]` in every
//! path and reads as `[2] flux`: the index is what an expression and an edit
//! are written with, and the word beside it is whatever the `TTYPE3` card
//! says. A cell works its own keywords out from where it sits, so nothing
//! here writes `TFORM1` through `TFORM999` down; the keyword is built as the
//! number a card's eight bytes read as, and a card carries that number beside
//! its keyword so a search can compare against it.
//!
//! A column's type comes from the letter in its `TFORMn` as a letter, which is
//! what a `Match` reading its `on` as text from anywhere an expression reaches
//! is for.
//!
//! What is not read here:
//!
//! - A table of more than 999 columns, which no keyword could describe: a
//!   `TFIELDS` past that reads the 999 columns the header could name.
//! - Every cell of every row still walks the header for its own `TFORMn`, and
//!   for the `TZEROn` that says whether it is unsigned. Reading the header
//!   once into a list and having the cells read the answers out of that was
//!   tried and is slower, not faster: the evaluator keeps what a node *is*
//!   and not what it *reads as*, so a field worked out from an expression is
//!   worked out again every time it is asked, and a cell reading one pays for
//!   the walk it was meant to save and for the walk to the list on top. The
//!   `columns` list is kept for what it says to a reader, not to save work.
//!   Hoisting the walk needs the evaluator to keep a value beside a shape.
//! - Which kind of table it is, is read from `TBCOL1` and `TFIELDS` rather
//!   than from `XTENSION`, which says so in text.
//! - A `TTYPEn` with an escaped quote in it reads as far as the quote. Nothing
//!   names a column that way.
//! - Heap arrays are found by walking every cell of every row, so a table of
//!   millions of rows takes that long before its heap has any children.
//! - A scale or a zero point with a fraction in it, which the integers here
//!   cannot add: the cards are shown beside the column and the sum is left to
//!   the reader. A variable-length column is not scaled at all, since its heap
//!   arrays are typed by the letter after the `P` and nothing carries the
//!   cards that far.
//! - A scaled image walks the header for `BZERO` and `BSCALE` once a pixel,
//!   so a scaled image of a million pixels is a million walks. An image that
//!   says nothing about its pixels pays none of it.
//! - A real written with no digits before the point, `TZERO1 = .5`, fails its
//!   card: the digits are read as a run that ends at the point, and a run with
//!   nothing in it is not a number. An exponent, `1.0E2`, reads as the 1 and
//!   leaves `E2` in the text after it, which is a value this declines to call
//!   a whole number rather than one it reads wrong.
//! - An axis the header says is there and does not give a length for. A
//!   missing keyword and a keyword whose value is zero both answer 0, and an
//!   axis of zero says there is no data, so a header that declares `NAXIS = 3`
//!   and writes no `NAXIS2` reads as an empty data unit rather than guessing
//!   what the missing card meant.
//! - Which keywords hold a number is a list here rather than something read
//!   from the file, since only a number can be read as one. A file that
//!   writes `NAXIS1  = '3'` fails that card and reads on.
//! - A long string joined into one value. `CONTINUE` cards read as the string
//!   each one holds, with the `&` that says more follows left where the file
//!   put it, and that is as far as it goes: a text field here is a run of
//!   bytes, and the string a long keyword means is spread over several cards
//!   with a keyword and two quotes between each pair. Joining them would need
//!   a type that reads text out of several runs at once, which the IR has no
//!   shape for; `Ty::Gather` places children and does not join them.
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
const NUMERIC: &[&str] = &["BITPIX", "PCOUNT", "GCOUNT", "TFIELDS", "THEAP", "EXTVER", "EXTLEVEL"];

/// The keywords that say what a stored number is worth rather than holding a
/// number of their own. The standard makes every one of them a real, so a
/// decimal point in any of them is a value written the way the format allows
/// and not a card to fail. See [`real_value`].
const REAL: &[&str] = &["BSCALE", "BZERO"];

/// How many columns of a table are read, which is the standard's own limit:
/// a keyword is eight bytes, five of them spent on `TFORM`, so the number
/// after it has three digits and 999 is as high as it goes. A `TFIELDS` that
/// says more than that is a header nothing could describe.
const COLUMNS: i128 = 999;

/// How many axes a data unit may have, for the same reason: `NAXIS` spends
/// five of a keyword's eight bytes and leaves three digits.
const AXES: i128 = 999;

/// The five-letter keywords a column or an axis carries its number in. Each
/// of them takes a body of its own, chosen by the five bytes the keyword opens
/// with rather than by the whole keyword, which is what lets a table have as
/// many columns as the standard allows. See [`numbered_key`].
const NUMBERED: &[(&str, fn() -> T)] = &[
    // How many along an axis, and where a column of an ASCII table starts in
    // a row: numbers like any other.
    ("NAXIS", numeric_body),
    ("TBCOL", numeric_body),
    // What a column holds, which is not a number.
    ("TFORM", tform_body),
    // What the numbers in a column are worth, which the standard lets a
    // writer put a decimal point in.
    ("TSCAL", real_body),
    ("TZERO", real_body),
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

/// One card: its keyword, the `= ` that says it has a value, and the rest.
///
/// The card is a window of exactly eighty bytes, so the value's search for a
/// `/` stops at the end of the line rather than running into the next card.
fn card() -> T {
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

/// What the header says about each column of a table, on one row per column.
///
/// A column is described by a handful of cards whose keywords carry its
/// number, scattered through a header of any length in any order, and a reader
/// asking why a column reads as it does had to find all five. This is those
/// cards gathered into the shape the rows are read through.
///
/// It covers no bytes and nothing depends on it: the cells read their own
/// cards. Having them read this instead was tried, and it is slower rather
/// than faster, because the evaluator keeps what a node is and not what it
/// reads as: a field worked out from an expression is worked out again every
/// time it is asked, so a cell reading one pays for the walk it meant to save
/// and for the walk to this list on top. So this is here for what it says to a
/// reader, which is the whole of it.
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
/// the unsigned convention, and that is read in the cell rather than here: a
/// column written as a signed integer with a zero point of exactly half its
/// range, and no scaling, *is* an unsigned column, and an eight-bit column
/// written unsigned with a zero point of -128 is a signed one. See
/// [`binary_cell`].
fn column() -> T {
    let n = E::Idx.add(E::lit(1));
    let form = |part: &str| numbered_at("TFORM", n.clone(), &["body", "value", "form", part]);
    let written = |prefix: &str| numbered_at(prefix, n.clone(), &["body", "value", "text"]);
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
        ],
    )
    .machinery(&["elem_code", "width", "start"])
}

/// How wide one element of the data is, in bytes: `|BITPIX|/8`. A float image
/// says -32 or -64 and means four bytes or eight.
fn element_bytes() -> E {
    let bitpix = card_value("BITPIX");
    E::lit(0).sub(bitpix.clone()).at_least(bitpix).div(E::lit(8))
}

/// How many along each axis, as a list of as many as `NAXIS` says there are.
///
/// A list rather than a run of fields, for the reason a row's cells are one:
/// the keyword carries the number, so `NAXIS1` through `NAXIS9` written out
/// here stopped at nine, and `NAXIS10` is a legal keyword. It covers no bytes.
///
/// A zero is read as a zero, which it was not before: a missing keyword and a
/// keyword whose value is zero both answer 0, and every axis used to read a
/// zero as a one. An axis genuinely declared `NAXIS3 = 0` says there is no
/// data, and now the data unit is that long.
///
/// The first axis is the exception, and it is the one the standard makes an
/// exception of. `NAXIS1 = 0` in a random-groups file is how the format says
/// the group parameters are all there is, and the size it wants is
/// `GCOUNT * (PCOUNT + NAXIS2 * ... * NAXISm)`, which is this product with
/// that zero read as a one. So a zero on the first axis is a one and a zero
/// anywhere else is a zero, which is the same arithmetic either way.
fn axes() -> T {
    let along = numbered_at("NAXIS", E::Idx.add(E::lit(1)), &["body", "value"]).or(E::Idx.equals(E::lit(0)));
    T::array(T::computed(along), card_value("NAXIS").at_most(E::lit(AXES)))
}

/// How many elements the data holds: the axes multiplied together, plus the
/// heap, times the number of groups. Nothing at all when `NAXIS` is zero,
/// which is the header-only unit every file with extensions opens with.
fn element_count() -> E {
    let any = E::lit(0).less_than(card_value("NAXIS"));
    let groups = card_value("GCOUNT").or(E::lit(1));
    E::product_of("axes").add(card_value("PCOUNT")).mul(groups).mul(any)
}

/// The data, read as the type BITPIX names. The switch is over the whole
/// array rather than over one element, so the row says `i16 be[]` rather than
/// leaving the reader with `switch[]`.
fn data_array() -> T {
    let of = |ty: T| T::array(ty, placed_count());
    // What the header says a pixel is worth: `BZERO + BSCALE * stored`, the
    // same reading a table's columns get from `TZEROn` and `TSCALn`. The
    // commonest case is the unsigned convention, an image written signed with
    // a zero point of half its range. See [`worth_of`].
    //
    // The test sits outside the array, so it is made once for the whole image
    // rather than once a pixel; the sum inside it is made per pixel, and it
    // walks the header for both cards, so a scaled image of a million pixels
    // is a million walks. Only an image that says it is scaled pays it.
    let real = |name: &str, part: &str| card_at(name, &["body", "value", "parts", part]);
    let scale = real("BSCALE", "int").or(E::lit(1));
    let scaled = |ty: T| {
        let plain = of(ty.clone());
        let with = T::array(worth_of(ty, scale.clone(), real("BZERO", "int")), placed_count());
        let whole = T::switch(real("BSCALE", "frac"), vec![(0, with)], plain.clone());
        let both = T::switch(real("BZERO", "frac"), vec![(0, whole)], plain.clone());
        T::switch(real("BZERO", "int").or(scale.clone().sub(E::lit(1))), vec![(0, plain)], both)
    };
    T::switch(
        card_value("BITPIX"),
        vec![
            (8, scaled(T::UInt { bits: 8, endian: Big })),
            (16, scaled(T::Int { bits: 16, endian: Big })),
            (32, scaled(T::Int { bits: 32, endian: Big })),
            (64, scaled(T::Int { bits: 64, endian: Big })),
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
    // Into every row, every cell of it, and every descriptor in a cell that
    // holds any. A cell that is not a variable-length column has no
    // `descriptors` field, and the walk goes no further down that way.
    let from = vec![
        Step::field("rows"),
        Step::each(),
        Step::field("cells"),
        Step::each(),
        Step::field("descriptors"),
        Step::each(),
    ];
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

/// One row of a binary table: a cell per column, laid out one after another,
/// each of them an array of as many values as its `TFORMn` says.
///
/// A list rather than a field per column. The keywords a column is described
/// by carry its number in their names, and a structure's field names are
/// fixed when the template is built, so a field per column meant writing
/// `col1` through `col32` out and stopping there. A list has no such limit:
/// the cell knows which column it is from where it sits, and works its
/// keywords out from that. See [`numbered_key`] and
/// [`crate::template::Field::elem_name_from`].
fn binary_row() -> T {
    row_of(binary_cell())
}

/// A row as a list of cells, each named by its `TTYPEn` card. The index stays
/// the path name, so `rows[0].cells[2]` is what an expression and an edit are
/// written with, and the row reads `[2] flux`.
fn row_of(cell: T) -> T {
    let name = numbered_at("TTYPE", column_number(), &["body", "value", "parts", "0", "text"]);
    T::structure("Row", vec![("cells", T::array(cell, table_columns()))]).field_elem_named_from("cells", name)
}

/// Which column a cell is: where it sits in the row, counted from one, since
/// that is how the keywords that describe it are numbered.
fn column_number() -> E {
    E::Idx.add(E::lit(1))
}

/// How many columns the table has, and never more than a keyword can number.
fn table_columns() -> E {
    card_value("TFIELDS").at_most(E::lit(COLUMNS))
}

/// One cell of a binary table's row, as the type its column's `TFORMn` names
/// and as many of them as its repeat count says. A count is one when none is
/// written.
///
/// The type is picked by the letter as a letter. `TFORMn` is a value written in
/// text and found by keyword, and a `Match` reads its `on` as text from
/// wherever an expression reaches it, so nothing here has to turn `J` into 74
/// and back.
///
/// The unsigned convention is read as a run of switches rather than as one
/// test, because a switch reads its subject once and an equality reads it
/// twice, and every one of these subjects is a walk of the header. A column
/// with no `TZEROn` card pays for one walk and falls straight through.
fn binary_cell() -> T {
    let n = column_number();
    let form = |part: &str| numbered_at("TFORM", n.clone(), &["body", "value", "form", part]);
    let real = |prefix: &str, part: &str| numbered_at(prefix, n.clone(), &["body", "value", "parts", part]);
    let r = form("repeat").or(E::lit(1));
    // Never more than the row has room for: a row whose columns do not add up
    // to `NAXIS1` shows the ones that fit rather than failing.
    let of = |ty: T, width: i128| T::array(ty, r.clone().at_most(E::Remaining.div(E::lit(width))));
    let pair = |name: &str, ty: T| T::inline_structure(name, vec![("re", ty.clone()), ("im", ty)]);
    // A column whose cards say its numbers are worth something else. The
    // integer on disk stays the integer on disk and stays editable as one;
    // what the header says it means sits beside it. See [`worth_of`].
    let scale = real("TSCAL", "int").or(E::lit(1));
    let scaled = |ty: T, width: i128| {
        let plain = of(ty.clone(), width);
        // What the cards say, read on the cell rather than on each value in
        // it: a value sits in a list of its own, and `Idx` answers for the
        // nearest list around the field asking, which down there is the run
        // of values and not the run of cells. The cell knows which column it
        // is; the values reach its answer by looking outwards.
        let values = T::array(
            worth_of(ty, E::field("scale"), E::field("zero")),
            r.clone().at_most(E::Remaining.div(E::lit(width))),
        );
        let with = T::structure(
            "Scaled column",
            vec![
                ("scale", T::computed(scale.clone())),
                ("zero", T::computed(real("TZERO", "int"))),
                ("values", values),
            ],
        )
        .machinery(&["scale", "zero"])
        .payload(&["values"]);
        // Both cards whole numbers, or this cannot say what a value means.
        let whole = T::switch(real("TSCAL", "frac"), vec![(0, with)], plain.clone());
        let both = T::switch(real("TZERO", "frac"), vec![(0, whole)], plain.clone());
        // Nothing said, or nothing that changes a value: the plain type. The
        // scale is only looked up when there is no zero point, since `Or`
        // stops at the first answer and most columns have neither card.
        let said = real("TZERO", "int").or(scale.clone().sub(E::lit(1)));
        T::switch(said, vec![(0, plain)], both)
    };
    let text = T::text(StrLen::Fixed(r.clone().at_most(E::Remaining)), Encoding::Ascii);
    T::matches(
        form("code"),
        vec![
            // A logical is written as the letter `T` or `F`, or as a zero byte
            // for a value nobody set.
            ("L", text.clone()),
            // A bit column is that many bits, rounded up to whole bytes.
            ("X", T::bytes(r.clone().add(E::lit(7)).div(E::lit(8)).at_most(E::Remaining))),
            ("B", scaled(T::UInt { bits: 8, endian: Big }, 1)),
            ("I", scaled(T::Int { bits: 16, endian: Big }, 2)),
            ("J", scaled(T::Int { bits: 32, endian: Big }, 4)),
            ("K", scaled(T::Int { bits: 64, endian: Big }, 8)),
            ("A", text),
            ("E", of(T::F32(Big), 4)),
            ("D", of(T::F64(Big), 8)),
            ("C", of(pair("Complex", T::F32(Big)), 8)),
            ("M", of(pair("Complex", T::F64(Big)), 16)),
            // A variable-length array is written as how many there are and
            // where in the heap they start.
            ("P", descriptors(T::Int { bits: 32, endian: Big }, 8, r.clone())),
            ("Q", descriptors(T::Int { bits: 64, endian: Big }, 16, r)),
        ],
        // A type letter nobody defined, or a `TFORMn` written in a shape this
        // could not read: the row still has its width, and this column covers
        // none of it.
        T::bytes(E::lit(0)),
    )
}

/// One number of a column the header says is worth something else: the integer
/// on disk, and beside it what `TSCALn` and `TZEROn` say it means.
///
/// `worth = TZEROn + TSCALn * stored`, which is what the standard says and
/// what a reader is after. It is a field of no bits: the bytes are the stored
/// integer's, it is those bytes that an edit writes, and this is a reading of
/// them, the way a Steim word's differences hang off the word in `mseed`.
///
/// The commonest case by far is the unsigned convention, a column written
/// signed with a zero point of half its range. That is a bias and not a
/// reinterpretation: 65535 is written as the signed 32767, whose bytes read as
/// a 16-bit unsigned number are 32767 and not 65535, so reading the column as
/// unsigned would answer with a number nobody wrote. The sum is the reading,
/// and the sum is exact whenever both cards are whole numbers.
fn worth_of(stored: T, scale: E, zero: E) -> T {
    let worth = E::field("stored").mul(scale).add(zero);
    T::inline_structure("Scaled", vec![("stored", stored), ("worth", T::computed(worth))]).payload(&["stored"])
}

/// The cell of a variable-length column: the descriptors in it, and the letter
/// that says what the heap arrays they point at hold.
///
/// The letter is a card of this column's, the `B` of `1PB`, and it is read
/// here rather than in the heap because the heap has no way back to the card:
/// an array in it is placed by a descriptor, and the array asks that
/// descriptor with [`crate::template::Expr::Placer`]. It sits on the cell
/// rather than on each descriptor because a cell knows which column it is and
/// a descriptor inside one does not: [`crate::template::Expr::Idx`] answers
/// for the nearest list around the field asking, which for a descriptor is the
/// run of descriptors beside it. The descriptors reach it by looking outwards,
/// the way any field reaches one declared around it.
fn descriptors(ty: T, width: i128, repeat: E) -> T {
    let elem_code = numbered_at("TFORM", column_number(), &["body", "value", "form", "elem_code"]);
    let one = T::inline_structure("Descriptor", vec![("count", ty.clone()), ("offset", ty)]);
    T::structure(
        "Descriptors",
        vec![
            ("elem_code", T::computed_text(elem_code)),
            ("descriptors", T::array(one, repeat.at_most(E::Remaining.div(E::lit(width))))),
        ],
    )
    .machinery(&["elem_code"])
    .payload(&["descriptors"])
}

/// One row of an ASCII table: its columns are text, each at the column
/// `TBCOLn` gives and as wide as `TFORMn` says. They are placed rather than
/// laid out one after another, since the standard lets them overlap and lets
/// gaps sit between them.
fn ascii_row() -> T {
    let n = column_number();
    let width = numbered_at("TFORM", n.clone(), &["body", "value", "form", "width"])
        .at_least(E::lit(1))
        .at_most(card_value("NAXIS1").at_least(E::lit(1)));
    let at = numbered_at("TBCOL", n, &["body", "value"]).sub(E::lit(1)).at_least(E::lit(0));
    row_of(T::at_in_window(at, T::text(StrLen::Fixed(width), Encoding::Ascii)))
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
            // How long the data is along each axis, which is what sizes it.
            // No bytes of its own: the cards it reads are where these live.
            ("axes", axes()),
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
        let data = ev.node(&d, &[0, 0, 3]).unwrap();
        assert_eq!(data.size_bits, 12 * 8);
        assert_eq!((data.type_name.as_str(), data.child_count), ("i16 be[]", 6));
        assert_eq!(ev.node(&d, &[0, 0, 3, 1]).unwrap().value, Value::Int(-2));
        // And the data is padded to a block of its own.
        assert_eq!(ev.node(&d, &[0, 0, 4]).unwrap().size_bits, (2880 - 12) * 8);
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
        let array = ev.node(&d, &[0, 0, 3]).unwrap();
        assert_eq!((array.type_name.as_str(), array.child_count), ("f32 be[]", 2));
        assert_eq!(ev.node(&d, &[0, 0, 3, 0]).unwrap().value, Value::Float(1.5));
    }

    #[test]
    fn a_header_only_unit_has_no_data_at_all() {
        let b = header(&["SIMPLE  =                    T", "BITPIX  =                    8", "NAXIS   =                    0", "EXTEND  =                    T", "END"]);
        let (d, mut ev) = eval(b);
        assert_eq!(ev.node(&d, &[0, 0, 3]).unwrap().size_bits, 0);
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
        let data = ev.node(&d, &[0, 1, 3]).unwrap();
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
        let rows = ev.node(&d, &[0, 1, 3, 1]).unwrap();
        assert_eq!(rows.child_count, 2);
        // Row 1, column 1: one 32-bit integer.
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 1, 0, 0, 0]).unwrap().value, Value::Int(2));
        // Column 2 is two floats, and the second of them is the second value.
        let flux = ev.node(&d, &[0, 1, 3, 1, 0, 0, 1]).unwrap();
        assert_eq!((flux.type_name.as_str(), flux.child_count), ("f32 be[]", 2));
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 1, 1]).unwrap().value, Value::Float(-0.25));
        // Column 3 is five characters, read as one run of text.
        assert_eq!(text(&ev.node(&d, &[0, 1, 3, 1, 0, 0, 2]).unwrap().value), "abcde");
        // A `TFORMn` with no repeat count means one.
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 3, 0]).unwrap().value, Value::Float(2.5));
        // A row is as wide as `NAXIS1` says.
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0]).unwrap().size_bits, 25 * 8);
        // Every column reads under the name its `TTYPEn` card gives it, with
        // its place in the row kept in front: that is the one a path is
        // written with, and it does not move when the header is edited.
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 0]).unwrap().name, "[0] counts");
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 1]).unwrap().name, "[1] flux");
        // A column with no `TTYPEn` keeps the bare index.
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 3]).unwrap().name, "[3]");
        // And the row says which card the name came from.
        let seen: Vec<_> =
            ev.origins(&d, &[0, 1, 3, 1, 0, 0, 1]).unwrap().into_iter().map(|o| (o.role, o.value)).collect();
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
        let heap = ev.node(&d, &[0, 1, 3, 2]).unwrap();
        assert_eq!(heap.size_bits, 5 * 8);
        // The row has one cell, since the header declared one column.
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0]).unwrap().child_count, 1);
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
        let desc = ev.node(&d, &[0, 1, 3, 1, 0, 0, 0, 1, 0]).unwrap();
        assert_eq!(desc.type_name, "Descriptor");
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 0, 1, 0, 0]).unwrap().value, Value::Int(3));
        // The letter after the `P` says what the arrays hold, and the cell
        // carries it so the heap can ask the descriptor that placed one.
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 0, 0]).unwrap().value, Value::Str("J".into()));
        assert_eq!(ev.node(&d, &[0, 1, 3, 2]).unwrap().size_bits, 12 * 8);
        // And the heap is that array: three 32-bit integers where the
        // descriptor pointed, which is the front of the heap.
        let heap = ev.node(&d, &[0, 1, 3, 2]).unwrap();
        assert_eq!(heap.child_count, 1);
        let array = ev.node(&d, &[0, 1, 3, 2, 0]).unwrap();
        assert_eq!((array.type_name.as_str(), array.child_count), ("i32 be[]", 3));
        assert_eq!((array.offset_bits, array.size_bits), (heap.offset_bits, 12 * 8));
        assert_eq!(ev.node(&d, &[0, 1, 3, 2, 0, 2]).unwrap().value, Value::Int(0x0909_0909));
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
        let heap = [0, 1, 3, 2];
        assert_eq!(ev.node(&d, &heap).unwrap().child_count, 2);
        // The first row's array is there, and has nothing in it.
        let empty = ev.node(&d, &[0, 1, 3, 2, 0]).unwrap();
        assert_eq!((empty.child_count, empty.size_bits), (0, 0));
        let full = ev.node(&d, &[0, 1, 3, 2, 1]).unwrap();
        assert_eq!((full.child_count, full.size_bits), (2, 16));
        assert_eq!(ev.node(&d, &[0, 1, 3, 2, 1, 1]).unwrap().value, Value::UInt(6));
    }

    #[test]
    fn two_variable_columns_in_one_row_both_reach_the_heap() {
        // Three bytes for the first column and two 16-bit numbers for the
        // second, one after the other in the heap.
        let b = heap_table(&["1PB", "1PI"], &[&[(3, 0), (2, 3)]], &[1, 2, 3, 0x12, 0x34, 0xff, 0xfe]);
        let (d, mut ev) = eval(b);
        assert_eq!(ev.node(&d, &[0, 1, 3, 2]).unwrap().child_count, 2);
        let bytes = ev.node(&d, &[0, 1, 3, 2, 0]).unwrap();
        let words = ev.node(&d, &[0, 1, 3, 2, 1]).unwrap();
        assert_eq!((bytes.type_name.as_str(), bytes.child_count), ("u8[]", 3));
        assert_eq!((words.type_name.as_str(), words.child_count), ("i16 be[]", 2));
        assert_eq!(words.offset_bits, bytes.offset_bits + 3 * 8);
        assert_eq!(ev.node(&d, &[0, 1, 3, 2, 1, 1]).unwrap().value, Value::Int(-2));
        // Each says which cell put it there: the second column of the only row.
        let placed = ev.origins(&d, &[0, 1, 3, 2, 1]).unwrap();
        assert_eq!(placed[0].label, "rows[0].cells[1].descriptors[0]");
        assert_eq!(placed[0].path, vec![0, 1, 3, 1, 0, 0, 1, 1, 0]);
    }

    #[test]
    fn heap_bytes_no_array_claims_are_a_gap() {
        // Ten bytes of heap; the arrays take the first three and two more
        // after a gap, and leave the end over.
        let b = heap_table(&["1PB"], &[&[(3, 0)], &[(2, 6)]], &[1, 2, 3, 0, 0, 0, 7, 8, 0, 0]);
        let (d, mut ev) = eval(b);
        let heap = ev.node(&d, &[0, 1, 3, 2]).unwrap();
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
        assert_eq!(ev.locate(&d, start + 4 * 8).unwrap(), vec![0, 1, 3, 2]);
        assert_eq!(ev.locate(&d, start + 7 * 8).unwrap(), vec![0, 1, 3, 2, 1, 1]);
    }

    /// A binary table may leave the repeat count out of a `TFORMn`, which is
    /// what astropy writes for a column of one value. A letter with a digit
    /// after it is an ASCII table's width instead.
    #[test]
    fn a_tform_with_no_repeat_count_is_one_value_of_that_type() {
        let cards = ["TFIELDS =                    2", "TFORM1  = 'I       '", "TFORM2  = '2I      '"];
        let mut b = primary();
        b.extend_from_slice(&table_header(&cards, 1, 6, 0));
        b.extend_from_slice(&padded(vec![0, 1, 0, 2, 0, 3]));
        let (d, mut ev) = eval(b);
        let one = ev.node(&d, &[0, 1, 3, 1, 0, 0, 0]).unwrap();
        assert_eq!((one.type_name.as_str(), one.child_count, one.size_bits), ("i16 be[]", 1, 16));
        let two = ev.node(&d, &[0, 1, 3, 1, 0, 0, 1]).unwrap();
        assert_eq!((two.type_name.as_str(), two.child_count, two.size_bits), ("i16 be[]", 2, 32));
    }

    /// A string too long for one card ends in `&` and goes on in the cards
    /// after it, each of which reads as the piece it holds.
    #[test]
    fn a_continue_card_reads_as_the_piece_of_the_string_it_holds() {
        let b = header(&[
            "SIMPLE  =                    T",
            "BITPIX  =                    8",
            "NAXIS   =                    0",
            "FILENAME= 'a name too long for one card, so it &'",
            "CONTINUE  'goes on here&'",
            "CONTINUE  '.' / and the comment is on the last one",
            "END",
        ]);
        let (d, mut ev) = eval(b);
        // The first card is an ordinary quoted value, `&` and all.
        let first = text(&ev.node(&d, &[0, 0, 0, 3, 2, 1, 1, 0, 0]).unwrap().value);
        assert_eq!(first, "a name too long for one card, so it &");
        // The cards after it have no `= ` and were one run of text before:
        // each reads as the string it holds.
        assert_eq!(text(&ev.node(&d, &[0, 0, 0, 4, 2, 1, 1, 0, 0]).unwrap().value), "goes on here&");
        assert_eq!(text(&ev.node(&d, &[0, 0, 0, 5, 2, 1, 1, 0, 0]).unwrap().value), ".");
        // And what follows the string on the last one is its comment.
        let comment = text(&ev.node(&d, &[0, 0, 0, 5, 2, 2]).unwrap().value);
        assert_eq!(comment, "/ and the comment is on the last one");
    }

    /// An axis past the ninth is legal, and the axes are a list now, so it is
    /// read. Ten axes of two are 1024 elements.
    #[test]
    fn a_tenth_axis_is_read_like_the_nine_before_it() {
        let mut cards: Vec<String> = vec![
            "SIMPLE  =                    T".into(),
            "BITPIX  =                    8".into(),
            "NAXIS   =                   10".into(),
        ];
        for n in 1..=10 {
            cards.push(format!("{:<8}=                    2", format!("NAXIS{n}")));
        }
        cards.push("END".into());
        let refs: Vec<&str> = cards.iter().map(|s| s.as_str()).collect();
        let mut b = header(&refs);
        b.extend_from_slice(&padded(vec![7u8; 1024]));
        let (d, mut ev) = eval(b);
        let axes = ev.node(&d, &[0, 0, 2]).unwrap();
        assert_eq!(axes.child_count, 10);
        let data = ev.node(&d, &[0, 0, 3]).unwrap();
        assert_eq!((data.size_bits, data.child_count), (1024 * 8, 1024));
    }

    /// An axis declared zero says there is no data, and is read as saying it.
    /// The first axis is the exception the standard makes: `NAXIS1 = 0` is how
    /// a random-groups file says the group parameters are all there is.
    #[test]
    fn an_axis_of_zero_is_no_data_unless_it_is_the_first_one() {
        let unit = |axes: &[(&str, i64)], pcount: i64, gcount: i64, bytes: usize| {
            let mut cards: Vec<String> = vec![
                "SIMPLE  =                    T".into(),
                "BITPIX  =                    8".into(),
                format!("NAXIS   = {:20}", axes.len()),
            ];
            cards.extend(axes.iter().map(|(k, v)| format!("{k:<8}= {v:20}")));
            cards.push(format!("PCOUNT  = {pcount:20}"));
            cards.push(format!("GCOUNT  = {gcount:20}"));
            cards.push("END".into());
            let refs: Vec<&str> = cards.iter().map(|s| s.as_str()).collect();
            let mut b = header(&refs);
            b.extend_from_slice(&padded(vec![7u8; bytes]));
            b
        };
        // A middle axis of zero: no elements, and the data unit is empty.
        let (d, mut ev) = eval(unit(&[("NAXIS1", 4), ("NAXIS2", 0), ("NAXIS3", 5)], 0, 1, 0));
        assert_eq!(ev.node(&d, &[0, 0, 3]).unwrap().size_bits, 0);
        // Random groups: `NAXIS1 = 0`, and the size is the group parameters
        // and the group data, once per group.
        let (d, mut ev) = eval(unit(&[("NAXIS1", 0), ("NAXIS2", 3)], 2, 4, 20));
        let data = ev.node(&d, &[0, 0, 3]).unwrap();
        assert_eq!((data.size_bits, data.child_count), (20 * 8, 20));
    }

    /// A row is a list of cells rather than a field per column, so a table may
    /// have as many columns as the standard allows rather than as many as
    /// there were names written out here. Forty is past the old cap of 32.
    #[test]
    fn a_table_of_more_columns_than_a_name_was_written_for_reads_all_of_them() {
        let columns = 40usize;
        let mut cards = vec![format!("TFIELDS = {columns:20}")];
        for n in 1..=columns {
            cards.push(format!("{:<8}= '1J      '", format!("TFORM{n}")));
            cards.push(format!("{:<8}= 'c{n}'", format!("TTYPE{n}")));
        }
        let refs: Vec<&str> = cards.iter().map(|s| s.as_str()).collect();
        let mut b = primary();
        b.extend_from_slice(&table_header(&refs, 1, columns * 4, 0));
        let mut data = Vec::new();
        for n in 1..=columns {
            data.extend_from_slice(&(n as i32).to_be_bytes());
        }
        b.extend_from_slice(&padded(data));
        let (d, mut ev) = eval(b);
        let cells = ev.node(&d, &[0, 1, 3, 1, 0, 0]).unwrap();
        assert_eq!(cells.child_count, columns as u64);
        // The last column, which nothing before this could name.
        let last = ev.node(&d, &[0, 1, 3, 1, 0, 0, columns - 1]).unwrap();
        assert_eq!(last.type_name, "i32 be[]");
        assert_eq!(last.name, "[39] c40");
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, columns - 1, 0]).unwrap().value, Value::Int(columns as i128));
        // And the row is still exactly as wide as `NAXIS1` said.
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0]).unwrap().size_bits, (columns * 4 * 8) as u64);
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
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 11]).unwrap().type_name, "Scaled column");
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 10]).unwrap().type_name, "i16 be[]");
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 0]).unwrap().type_name, "i16 be[]");
    }

    /// A column with a zero point reads as the integer on disk and what that
    /// integer is worth. The unsigned convention is the common case: 65535 is
    /// written as the signed 32767, and 32767 is what is on disk.
    #[test]
    fn a_zero_point_says_what_the_integer_on_disk_is_worth() {
        let cards = [
            "TFIELDS =                    2",
            "TFORM1  = '1I      '",
            "TZERO1  =                32768",
            "TSCAL1  =                    1",
            "TFORM2  = '1I      '",
        ];
        let mut b = primary();
        b.extend_from_slice(&table_header(&cards, 1, 4, 0));
        // 0xffff is -1 either way; the first column says it means 32767.
        b.extend_from_slice(&padded(vec![0xff, 0xff, 0xff, 0xff]));
        let (d, mut ev) = eval(b);
        let scaled = ev.node(&d, &[0, 1, 3, 1, 0, 0, 0]).unwrap();
        assert_eq!(scaled.type_name, "Scaled column");
        // The bytes are two, and what the header said takes none of them.
        assert_eq!(scaled.size_bits, 16);
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 0, 2, 0, 0]).unwrap().value, Value::Int(-1));
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 0, 2, 0, 1]).unwrap().value, Value::Int(32767));
        let signed = ev.node(&d, &[0, 1, 3, 1, 0, 0, 1]).unwrap();
        assert_eq!(signed.type_name, "i16 be[]");
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 1, 0]).unwrap().value, Value::Int(-1));
        // And what the header said the numbers are worth is on one row beside
        // the column rather than spread over the cards.
        assert_eq!(text(&ev.node(&d, &[0, 1, 3, 0, 0, 2]).unwrap().value), "1");
        assert_eq!(text(&ev.node(&d, &[0, 1, 3, 0, 0, 3]).unwrap().value), "32768");
    }

    /// A zero point written with a point after it is the same whole number,
    /// and a whole number is one this can add. One with a fraction after the
    /// point is not, and the column says what the card said instead.
    #[test]
    fn a_zero_point_is_only_added_when_it_is_a_whole_number() {
        let table = |zero: &str| {
            let card = format!("TZERO1  = {zero:>20}");
            let cards = ["TFIELDS =                    1", "TFORM1  = '1I      '", card.as_str()];
            let mut b = primary();
            b.extend_from_slice(&table_header(&cards, 1, 2, 0));
            b.extend_from_slice(&padded(vec![0xff, 0xff]));
            b
        };
        for written in ["32768", "32768.", "32768.0", "32768.00", "-32768", "1"] {
            let (d, mut ev) = eval(table(written));
            assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 0]).unwrap().type_name, "Scaled column", "{written}");
        }
        // A fraction, and a zero point of zero, which changes nothing.
        for written in ["32768.5", "0.5", "0"] {
            let (d, mut ev) = eval(table(written));
            assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 0]).unwrap().type_name, "i16 be[]", "{written}");
        }
    }

    /// An image says the same thing with `BZERO`, over its pixels.
    #[test]
    fn an_images_zero_point_says_what_its_pixels_are_worth() {
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
        // Unsigned 16-bit pixels: -1 on disk, and 32767 is what it means.
        let (d, mut ev) = eval(pixels(16, "32768", vec![0xff, 0xff]));
        assert_eq!(ev.node(&d, &[0, 0, 3]).unwrap().type_name, "Scaled[]");
        assert_eq!(ev.node(&d, &[0, 0, 3, 0, 0]).unwrap().value, Value::Int(-1));
        assert_eq!(ev.node(&d, &[0, 0, 3, 0, 1]).unwrap().value, Value::Int(32767));
        // The other way round, on the one type FITS writes unsigned: 255 on
        // disk, and 127 is what it means.
        let (d, mut ev) = eval(pixels(8, "-128", vec![0xff]));
        assert_eq!(ev.node(&d, &[0, 0, 3, 0, 0]).unwrap().value, Value::UInt(255));
        assert_eq!(ev.node(&d, &[0, 0, 3, 0, 1]).unwrap().value, Value::Int(127));
        // A zero point of nothing leaves the pixels as the type BITPIX names.
        let (d, mut ev) = eval(pixels(16, "0", vec![0xff, 0xff]));
        assert_eq!(ev.node(&d, &[0, 0, 3]).unwrap().type_name, "i16 be[]");
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
        assert_eq!(ev.node(&d, &[0, 1, 3, 1]).unwrap().child_count, 2);
        assert_eq!(text(&ev.node(&d, &[0, 1, 3, 1, 0, 0, 0, 0]).unwrap().value), "12");
        assert_eq!(text(&ev.node(&d, &[0, 1, 3, 1, 0, 0, 1, 0]).unwrap().value), "1.500");
        assert_eq!(text(&ev.node(&d, &[0, 1, 3, 1, 1, 0, 0, 0]).unwrap().value), "-7");
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
        let o = ev.origins(&d, &[0, 0, 3]).unwrap();
        let seen: Vec<_> = o.iter().map(|x| (x.role, x.label.clone(), x.value.clone())).collect();
        assert!(seen.iter().any(|(r, l, v)| *r == Role::Length && l.starts_with("cards[1]") && v == "16"), "{seen:?}");
        // How many elements comes from the axes, and an axis says which card
        // it read: one hop further than it used to be, and the hop is a row
        // the reader can see.
        assert!(seen.iter().any(|(r, l, v)| *r == Role::Count && l == "axes" && v == "6"), "{seen:?}");
        let axis = ev.origins(&d, &[0, 0, 2, 0]).unwrap();
        let from: Vec<_> = axis.iter().map(|x| (x.label.clone(), x.value.clone())).collect();
        assert!(from.iter().any(|(l, v)| l.starts_with("cards[3]") && v == "3"), "{from:?}");
    }
}
