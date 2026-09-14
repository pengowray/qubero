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
//! A binary table whose header says `ZIMAGE = T` is that case, and reads as a
//! compressed image: the algorithm `ZCMPTYPE` names, how many pixels along
//! each axis of the image (`ZNAXISn`) and of a tile (`ZTILEn`), and then the
//! same columns, rows and heap as any table, with each row counted as the tile
//! it is. None of that changes what a byte reads as, since the file is a
//! table; it says what the table is for. `ZIMAGE` is read as the letter it
//! holds so that a match on it can say `T`.
//!
//! What the header says about each column is on one row per column, in
//! `columns`. That list covers no bytes and nothing depends on it: it is the
//! cards a column is described by, which are scattered through the header,
//! gathered into the shape the rows are read through, so that a reader asking
//! why a column reads as it does has the whole answer in one place.
//!
//! `TSCALn` and `TZEROn` say what the numbers in a column are worth: a stored
//! value `x` means `TZEROn + TSCALn * x`, and `BZERO` and `BSCALE` say the
//! same of an image's pixels. That sum is worked out, and each number reads as
//! the number on disk with what it is worth beside it. The number on disk is
//! what an edit writes; the worth takes no bits, the way a Steim word's
//! differences hang off the word in `mseed`.
//!
//! The commonest case by far is the unsigned convention: an unsigned 16-bit
//! column is written as a signed one with a zero point of 32768, so 65535 is
//! on disk as the signed 32767. That is a bias and not a reinterpretation, and
//! reading those bytes as a 16-bit unsigned number would answer 32767, which
//! is a number nobody wrote. The sum is the reading.
//!
//! When both cards are whole numbers and the column holds integers, the sum is
//! made in whole numbers, which is exact however large it is: a 64-bit column
//! written unsigned has a zero point of 2^63, past where a double counts in
//! ones. A card with a fraction or an exponent in it, `TSCAL2 = 2.5` or
//! `BZERO = 3.276800E4`, and a column of floats, are worked out as reals, from
//! the real each card's text spells. The worth of a float then comes from the
//! float as its row shows it, the shortest decimal that reads back as its
//! bits, so it agrees with astropy, which works from the bits, to about eight
//! significant figures rather than to the last digit.
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
//! - A variable-length column is not scaled at all, since its heap arrays are
//!   typed by the letter after the `P` and nothing carries the cards that far.
//! - A scaled image walks the header for `BZERO` and `BSCALE` once a pixel,
//!   so a scaled image of a million pixels is a million walks. An image that
//!   says nothing about its pixels pays none of it.
//! - A real written with no digits before the point, `TZERO1 = .5`, reads as
//!   the real it is but fails the test of whether it changes anything, which
//!   reads the digits either side of the point as whole numbers, and a run
//!   with nothing in it is not a number. The column fails with it.
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
//! - A tile's pixels. A compressed image's rows and heap read as the table
//!   they are, the rows the tiles' descriptors and the heap their compressed
//!   bytes, and no field here holds a pixel: undoing Rice, gzip and the
//!   quantization of a float image is a running sum, a deflate stream and a
//!   sequence of random numbers, none of which an expression can carry.
//!   [`fits_tile`](super::fits_tile) does it beside the template, for the
//!   tile under the cursor. The compression parameters in `ZNAMEi` and
//!   `ZVALi` are cards like any other here, since which `ZVALi` is the block
//!   size is a search by the text of another card.

use super::fits_cards::card;
use crate::template::{Anchor, Encoding, Endian::Big, Expr as E, Step, StrLen, Template, Ty as T, Until};

/// The block every part of a FITS file is padded out to.
const BLOCK: u32 = 2880;

/// How many columns of a table are read, which is the standard's own limit:
/// a keyword is eight bytes, five of them spent on `TFORM`, so the number
/// after it has three digits and 999 is as high as it goes. A `TFIELDS` that
/// says more than that is a header nothing could describe.
const COLUMNS: i128 = 999;

/// How many axes a data unit may have, for the same reason: `NAXIS` spends
/// five of a keyword's eight bytes and leaves three digits.
const AXES: i128 = 999;

/// A keyword as it is written in a card: eight bytes, padded with spaces.
fn keyword(name: &str) -> Vec<u8> {
    let mut b = name.as_bytes().to_vec();
    b.resize(8, b' ');
    b
}

/// The eight bytes of a keyword read as one big-endian number, which is what
/// a card carries in `keynum` and what a lookup by a keyword worked out here
/// rather than written down compares against. See [`numbered_key`].
pub(super) fn keynum(name: &str) -> i128 {
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
/// Nearly every prefix here is five bytes long, which leaves the three bytes a
/// keyword has left over for up to three digits. The digits are the number
/// picked apart with division, since the IR has no way to write a number as
/// text: `7` is `'7'`, a space and a space, `71` is `'7'`, `'1'` and a space.
///
/// `ZNAXIS` is the one of six, which leaves two bytes and so two digits. That
/// is 99 axes, and no compressed image has more than a handful.
fn numbered_key(prefix: &str, n: E) -> E {
    let digit = |v: E| v.add(E::lit(i128::from(b'0')));
    let blank = i128::from(b' ');
    if prefix.len() == 6 {
        let head = (keynum(prefix) >> 16) << 16;
        let tens = n.clone().div(E::lit(10));
        let units = n.clone().sub(tens.clone().mul(E::lit(10)));
        let one = n.clone().less_than(E::lit(10));
        let two = E::lit(9).less_than(n.clone()).mul(n.clone().less_than(E::lit(100)));
        let short = one.mul(digit(n).mul(E::lit(1 << 8)).add(E::lit(blank)));
        let medium = two.mul(digit(tens).mul(E::lit(1 << 8)).add(digit(units)));
        return E::lit(head).add(short).add(medium);
    }
    assert_eq!(prefix.len(), 5, "a numbered FITS keyword is five or six letters and the digits that fit after them");
    // The five letters where a keyword writes them, with the three spaces it
    // is padded with taken off: the digits go where that padding was.
    let head = (keynum(prefix) >> 24) << 24;
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
/// standard says a stored value `x` means `TZEROn + TSCALn * x`, and these are
/// the two reals the cards spell, `2.0E+01` included, with the standard's own
/// defaults of one and nought for a card that is not there. The sum itself is
/// made in the cells, where each value reads as the number on disk with what
/// it is worth beside it. See [`binary_cell`].
fn column() -> T {
    let n = E::Idx.add(E::lit(1));
    let form = |part: &str| numbered_at("TFORM", n.clone(), &["body", "value", "form", part]);
    let written = |prefix: &str| E::real_text(numbered_at(prefix, n.clone(), &["body", "value", "text"]));
    T::inline_structure(
        "Column",
        vec![
            ("code", T::computed_text(form("code"))),
            ("repeat", T::computed(form("repeat").or(E::lit(1)))),
            // What the numbers in this column are worth, as the reals the
            // cards spell: `TZERO3 = 0.4` says 0.4 here too.
            ("scale", T::computed_real(written("TSCAL").or(E::real(1.0)))),
            ("zero", T::computed_real(written("TZERO"))),
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
    // a zero point of half its range, and that sum is made in whole numbers;
    // a card with a fraction or an exponent in it is made in reals, and so is
    // an image of floats. See [`worth_of`].
    //
    // The test sits outside the array, so it is made once for the whole image
    // rather than once a pixel; the sum inside it is made per pixel, and it
    // walks the header for both cards, so a scaled image of a million pixels
    // is a million walks. Only an image that says it is scaled pays it.
    let real = |name: &str, part: &str| card_at(name, &["body", "value", "parts", part]);
    let scale = real("BSCALE", "int").or(E::lit(1));
    // The same two in reals, for a card with a fraction or an exponent in it.
    let real_scale = E::real_text(card_at("BSCALE", &["body", "value", "text"])).or(E::real(1.0));
    let real_zero = E::real_text(card_at("BZERO", &["body", "value", "text"]));
    let reals = |ty: T| T::array(worth_of(ty, real_scale.clone(), real_zero.clone(), false), placed_count());
    // Whether the cards change a pixel at all, the way a column's are asked.
    // See [`binary_cell`].
    let said = E::cond(
        card_at("BZERO", &["keynum"]).or(card_at("BSCALE", &["keynum"])),
        real("BZERO", "int")
            .or(real("BZERO", "frac"))
            .or(real("BZERO", "exp"))
            .or(scale.clone().sub(E::lit(1)))
            .or(real("BSCALE", "frac"))
            .or(real("BSCALE", "exp")),
        E::lit(0),
    );
    let scaled = |ty: T| {
        let plain = of(ty.clone());
        let with = T::array(worth_of(ty.clone(), scale.clone(), real("BZERO", "int"), true), placed_count());
        let whole = T::switch(real("BSCALE", "frac"), vec![(0, with)], reals(ty.clone()));
        let both = T::switch(real("BZERO", "frac"), vec![(0, whole)], reals(ty.clone()));
        let plain_scale = T::switch(real("BSCALE", "exp"), vec![(0, both)], reals(ty.clone()));
        let both = T::switch(real("BZERO", "exp"), vec![(0, plain_scale)], reals(ty));
        T::switch(said.clone(), vec![(0, plain)], both)
    };
    let scaled_float = |ty: T| T::switch(said.clone(), vec![(0, of(ty.clone()))], reals(ty));
    T::switch(
        card_value("BITPIX"),
        vec![
            (8, scaled(T::UInt { bits: 8, endian: Big })),
            (16, scaled(T::Int { bits: 16, endian: Big })),
            (32, scaled(T::Int { bits: 32, endian: Big })),
            (64, scaled(T::Int { bits: 64, endian: Big })),
            (-32, scaled_float(T::F32(Big))),
            (-64, scaled_float(T::F64(Big))),
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
    T::structure("Table", table_fields(row, "row"))
}

/// The fields every binary table has, whatever it holds: what the header says
/// of each column, the rows, each of them counted as a `unit`, and the heap.
fn table_fields(row: T, unit: &str) -> Vec<(&'static str, T)> {
    let width = card_value("NAXIS1").at_least(E::lit(1));
    // A row of no width is no row at all: without the check, a table that
    // says so would lay a row over every byte of its heap.
    let any = E::lit(0).less_than(card_value("NAXIS1"));
    let rows = card_value("NAXIS2").mul(any).at_most(E::Remaining.div(width.clone()));
    vec![
        // What the header said about each column, read once here rather
        // than again in every cell of every row. It covers no bytes; see
        // [`columns`].
        ("columns", columns()),
        ("rows", T::array(T::sized(width, row).counted_as(unit), rows)),
        ("heap", T::sized(E::Remaining, heap())),
    ]
}

/// A binary table, which is a compressed image when its header says
/// `ZIMAGE = T` and a table like any other when it does not.
fn binary_table() -> T {
    T::matches(card_at("ZIMAGE", &["body", "value"]), vec![("T", compressed_image())], table(binary_row()))
}

/// A tile-compressed image: a binary table with a row per tile, as FITS
/// writes an image too large to keep whole.
///
/// The image is cut into tiles of `ZTILEn` pixels along each axis, the last
/// tile along an axis taking what is left, and each tile is compressed on its
/// own with the algorithm `ZCMPTYPE` names. A row holds a descriptor for that
/// tile's compressed bytes, which are an array in the heap, and for an image
/// of floats the `ZSCALE` and `ZZERO` its integers were quantized with. The
/// rows are counted in tiles, and tile `n` is row `n`: the first tile along
/// the first axis is the first row, and the first axis is the one that runs
/// fastest.
///
/// What the image is, and what a tile is, are said first: the algorithm, and
/// how many pixels along each axis of the image and of a tile. They cover no
/// bytes, and the rows and the heap read as they would in any table, since
/// what is in the file is a table. What a tile holds once it is decompressed
/// is not in the file at all, and is worked out beside the template by
/// [`fits_tile`](super::fits_tile), which the inspector shows for the cursor
/// anywhere in the image's data.
fn compressed_image() -> T {
    let axes = card_value("ZNAXIS").at_most(E::lit(99));
    let along = |prefix: &str| numbered_at(prefix, E::Idx.add(E::lit(1)), &["body", "value"]);
    // A tile the header gives no size for along an axis is the whole image
    // along the first axis and one pixel along every other, which is what the
    // convention says a missing `ZTILEn` means: a row of pixels at a time.
    let tile = along("ZTILE").or(E::Idx.less_than(E::lit(1)).mul(card_value("ZNAXIS1"))).or(E::lit(1));
    let mut fields = vec![
        ("algorithm", T::computed_text(card_at("ZCMPTYPE", &["body", "value", "parts", "0", "text"]))),
        ("image_shape", T::array(T::computed(along("ZNAXIS")), axes.clone())),
        ("tile_shape", T::array(T::computed(tile), axes)),
    ];
    fields.extend(table_fields(tile_row(), "tile"));
    T::structure("Compressed image", fields).packed_as(super::fits_tile::PACKING)
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

/// One row of a compressed image, which is one tile: the same cells as any
/// binary table's row, under the name of what the row is.
fn tile_row() -> T {
    row_named("Tile", binary_cell())
}

/// A row as a list of cells, each named by its `TTYPEn` card. The index stays
/// the path name, so `rows[0].cells[2]` is what an expression and an edit are
/// written with, and the row reads `[2] flux`.
fn row_of(cell: T) -> T {
    row_named("Row", cell)
}

/// The same, under a name of its own.
fn row_named(name: &str, cell: T) -> T {
    let label = numbered_at("TTYPE", column_number(), &["body", "value", "parts", "0", "text"]);
    T::structure(name, vec![("cells", T::array(cell, table_columns()))])
        .payload(&["cells"])
        .field_elem_named_from("cells", label)
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
    // number on disk stays the number on disk and stays editable as one;
    // what the header says it means sits beside it. See [`worth_of`].
    let card = |prefix: &str| numbered_at(prefix, n.clone(), &["body", "value", "text"]);
    let column = |ty: T, width: i128, whole: bool| {
        // What the cards say, read on the cell rather than on each value in
        // it: a value sits in a list of its own, and `Idx` answers for the
        // nearest list around the field asking, which down there is the run
        // of values and not the run of cells. The cell knows which column it
        // is; the values reach its answer by looking outwards.
        let values = T::array(
            worth_of(ty, E::field("scale"), E::field("zero"), whole),
            r.clone().at_most(E::Remaining.div(E::lit(width))),
        );
        let (scale, zero) = match whole {
            true => (T::computed(real("TSCAL", "int").or(E::lit(1))), T::computed(real("TZERO", "int"))),
            false => (
                T::computed_real(E::real_text(card("TSCAL")).or(E::real(1.0))),
                T::computed_real(E::real_text(card("TZERO"))),
            ),
        };
        T::structure("Scaled column", vec![("scale", scale), ("zero", zero), ("values", values)])
            .machinery(&["scale", "zero"])
            .payload(&["values"])
    };
    // Whether the cards change a value at all. A column with neither card
    // pays for two searches and falls through: `Cond` asks the rest only when
    // one of them is there, and most columns have neither. A zero point that
    // is not nought, or a scale that is not one, in any part of how it is
    // written, is a change; `1.0E0` is counted as one, which costs a row that
    // says so and nothing else.
    let present = |prefix: &str| numbered_at(prefix, n.clone(), &["keynum"]);
    let said = E::cond(
        present("TZERO").or(present("TSCAL")),
        real("TZERO", "int")
            .or(real("TZERO", "frac"))
            .or(real("TZERO", "exp"))
            .or(real("TSCAL", "int").or(E::lit(1)).sub(E::lit(1)))
            .or(real("TSCAL", "frac"))
            .or(real("TSCAL", "exp")),
        E::lit(0),
    );
    let scaled = |ty: T, width: i128| {
        let plain = of(ty.clone(), width);
        let reals = column(ty.clone(), width, false);
        // Both cards whole numbers with no exponent on them, and the sum is
        // made in whole numbers, which is exact however large: the unsigned
        // convention on a 64-bit column is a zero point of 2^63, and a double
        // stops counting in ones at 2^53. Anything else is made in reals.
        let whole = T::switch(real("TSCAL", "frac"), vec![(0, column(ty, width, true))], reals.clone());
        let both = T::switch(real("TZERO", "frac"), vec![(0, whole)], reals.clone());
        let plain_scale = T::switch(real("TSCAL", "exp"), vec![(0, both)], reals.clone());
        let both = T::switch(real("TZERO", "exp"), vec![(0, plain_scale)], reals);
        T::switch(said.clone(), vec![(0, plain)], both)
    };
    // A float column has no whole-number sum to make, and is scaled in reals
    // whenever its cards say anything.
    let scaled_float = |ty: T, width: i128| T::switch(said.clone(), vec![(0, of(ty.clone(), width))], column(ty, width, false));
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
            ("E", scaled_float(T::F32(Big), 4)),
            ("D", scaled_float(T::F64(Big), 8)),
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
/// unsigned would answer with a number nobody wrote. The sum is the reading.
///
/// `whole` says both cards are whole numbers and the stored number is an
/// integer, and then the sum is made in whole numbers and is exact however
/// large it is. Otherwise it is made in reals: a scale of 2.5, a zero point of
/// 0.4, a card written `2.0E+01`, or a column of floats.
fn worth_of(stored: T, scale: E, zero: E, whole: bool) -> T {
    let worth = E::field("stored").mul(scale).add(zero);
    let worth = if whole { T::computed(worth) } else { T::computed_real(worth) };
    T::inline_structure("Scaled", vec![("stored", stored), ("worth", worth)]).payload(&["stored"])
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
                    T::switch(table_kind(), vec![(1, table(ascii_row())), (2, binary_table())], data_array()),
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
mod tests;
