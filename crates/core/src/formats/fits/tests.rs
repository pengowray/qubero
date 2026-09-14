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
fn a_tables_heap_is_found_after_reading_a_table_further_on() {
    // Three tables of the same two rows, each row a descriptor into a heap
    // of three bytes: two for the first row and one for the second.
    let one = heap_table(&["1PB"], &[&[(2, 0)], &[(1, 2)]], &[5, 6, 7]);
    let mut b = one.clone();
    b.extend_from_slice(&one[2880..]);
    b.extend_from_slice(&one[2880..]);
    let (d, mut ev) = eval(b);
    // A row of the first table, then a row of the third, which walks over
    // the first table's unit on the way.
    ev.node(&d, &[0, 1, 3, 1, 0]).unwrap();
    ev.node(&d, &[0, 3, 3, 1, 0]).unwrap();
    // The first table's heap starts where its header's cards say, so it
    // is only found if the walk left that header in place.
    let heap = ev.node(&d, &[0, 1, 3, 2]).unwrap();
    assert_eq!(heap.child_count, 2);
    let first = ev.node(&d, &[0, 1, 3, 2, 0]).unwrap();
    let second = ev.node(&d, &[0, 1, 3, 2, 1]).unwrap();
    assert_eq!((first.offset_bits, first.size_bits), (heap.offset_bits, 2 * 8));
    assert_eq!((second.offset_bits, second.size_bits), (heap.offset_bits + 2 * 8, 8));
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
    // the column rather than spread over the cards, with the standard's
    // own default for the card that is not there.
    assert_eq!(ev.node(&d, &[0, 1, 3, 0, 0, 2]).unwrap().value, Value::Float(1.0));
    assert_eq!(ev.node(&d, &[0, 1, 3, 0, 0, 3]).unwrap().value, Value::Float(32768.0));
    assert_eq!(ev.node(&d, &[0, 1, 3, 0, 1, 2]).unwrap().value, Value::Float(1.0));
    assert_eq!(ev.node(&d, &[0, 1, 3, 0, 1, 3]).unwrap().value, Value::Float(0.0));
}

/// A zero point written with a point after it is the same whole number,
/// and is added in whole numbers. One with a fraction or an exponent is
/// added as a real, and a zero point of nought changes nothing.
#[test]
fn a_zero_point_is_added_as_a_whole_number_or_as_a_real() {
    let table = |zero: &str| {
        let card = format!("TZERO1  = {zero:>20}");
        let cards = ["TFIELDS =                    1", "TFORM1  = '1I      '", card.as_str()];
        let mut b = primary();
        b.extend_from_slice(&table_header(&cards, 1, 2, 0));
        b.extend_from_slice(&padded(vec![0xff, 0xff]));
        b
    };
    let worth = |written: &str| {
        let (d, mut ev) = eval(table(written));
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 0]).unwrap().type_name, "Scaled column", "{written}");
        ev.node(&d, &[0, 1, 3, 1, 0, 0, 0, 2, 0, 1]).unwrap().value
    };
    for (written, want) in [("32768", 32767), ("32768.", 32767), ("32768.0", 32767), ("32768.00", 32767), ("-32768", -32769), ("1", 0)] {
        assert_eq!(worth(written), Value::Int(want), "{written}");
    }
    // `2.0E+01` is twenty, and reading its digits alone would say two.
    for (written, want) in [("32768.5", 32767.5), ("0.5", -0.5), ("2.0E+01", 19.0), ("1.0E-3", -0.999), ("3.2768E4", 32767.0), ("2.0D1", 19.0)] {
        assert_eq!(worth(written), Value::Float(want), "{written}");
    }
    for written in ["0", "0.0", "0."] {
        let (d, mut ev) = eval(table(written));
        assert_eq!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 0]).unwrap().type_name, "i16 be[]", "{written}");
    }
}

/// A scale and a zero point written with exponents, one of them Fortran's
/// `D`, over a column of integers and a column of floats. What a value is
/// worth is the reals the cards spell, and the relations panel writes the
/// sum out with them in place.
#[test]
fn a_scale_written_with_an_exponent_scales_the_column() {
    let cards = [
        "TFIELDS =                    2",
        "TFORM1  = '1J      '",
        "TSCAL1  =              2.0E+01 / twenty",
        "TZERO1  =               1.5D-1",
        "TFORM2  = '2E      '",
        "TSCAL2  =                  2.5",
        "TZERO2  =                  0.4",
    ];
    let mut b = primary();
    b.extend_from_slice(&table_header(&cards, 1, 12, 0));
    let mut row = 3i32.to_be_bytes().to_vec();
    row.extend_from_slice(&0.25f32.to_be_bytes());
    row.extend_from_slice(&(-1.5f32).to_be_bytes());
    b.extend_from_slice(&padded(row));
    let (d, mut ev) = eval(b);
    let first = [0, 1, 3, 1, 0, 0, 0];
    assert_eq!(ev.node(&d, &first).unwrap().type_name, "Scaled column");
    let scale = ev.node(&d, &[&first[..], &[0]].concat()).unwrap();
    assert_eq!((scale.type_name.as_str(), scale.value), ("computed real", Value::Float(20.0)));
    assert_eq!(ev.node(&d, &[&first[..], &[1]].concat()).unwrap().value, Value::Float(0.15));
    let worth = [&first[..], &[2, 0, 1]].concat();
    assert_eq!(ev.node(&d, &[&first[..], &[2, 0, 0]].concat()).unwrap().value, Value::Int(3));
    assert_eq!(ev.node(&d, &worth).unwrap().value, Value::Float(3.0 * 20.0 + 0.15));
    let rel = ev.relations(&d, &worth).unwrap();
    assert_eq!((rel[0].written.as_str(), rel[0].substituted.as_str()), ("stored * scale + zero", "3 * 20 + 0.15"));
    // The float column: each float the number on disk, and what it is
    // worth beside it.
    let second = [0, 1, 3, 1, 0, 0, 1];
    assert_eq!(ev.node(&d, &second).unwrap().type_name, "Scaled column");
    assert_eq!(ev.node(&d, &[&second[..], &[2, 1, 0]].concat()).unwrap().value, Value::Float(-1.5));
    assert_eq!(ev.node(&d, &[&second[..], &[2, 0, 1]].concat()).unwrap().value, Value::Float(0.25 * 2.5 + 0.4));
    assert_eq!(ev.node(&d, &[&second[..], &[2, 1, 1]].concat()).unwrap().value, Value::Float(-1.5 * 2.5 + 0.4));
    // A card that is there and spells no number fails what reads it.
    let cards = ["TFIELDS =                    1", "TFORM1  = '1J      '", "TSCAL1  =                2.5Ex"];
    let mut b = primary();
    b.extend_from_slice(&table_header(&cards, 1, 4, 0));
    b.extend_from_slice(&padded(vec![0, 0, 0, 1]));
    let (d, mut ev) = eval(b);
    assert!(ev.node(&d, &[0, 1, 3, 1, 0, 0, 0, 2, 0, 1]).is_err());
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
    // One written with an exponent, as IRAF writes them, is the real it
    // spells; and an image of floats is scaled the same way.
    let (d, mut ev) = eval(pixels(16, "3.276800E4", vec![0xff, 0xff]));
    assert_eq!(ev.node(&d, &[0, 0, 3, 0, 1]).unwrap().value, Value::Float(32767.0));
    let (d, mut ev) = eval(pixels(-32, "0.5", 2.0f32.to_be_bytes().to_vec()));
    assert_eq!(ev.node(&d, &[0, 0, 3]).unwrap().type_name, "Scaled[]");
    assert_eq!(ev.node(&d, &[0, 0, 3, 0, 1]).unwrap().value, Value::Float(2.5));
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

/// A 5 by 3 image cut into tiles of one row each, written as a table of
/// three rows whose heap is the three tiles' bytes. `ZTILE2` is left out,
/// which means a tile is one pixel deep along that axis.
fn compressed(zimage: &str) -> Vec<u8> {
    let mut b = primary();
    let cards = [
        "TFIELDS =                    1",
        "TTYPE1  = 'COMPRESSED_DATA'",
        "TFORM1  = '1PB(4)  '",
        &format!("ZIMAGE  = {zimage:>20} / extension contains compressed image"),
        "ZBITPIX =                   16",
        "ZNAXIS  =                    2",
        "ZNAXIS1 =                    5",
        "ZNAXIS2 =                    3",
        "ZTILE1  =                    5",
        "ZCMPTYPE= 'RICE_1  '           / compression algorithm",
    ];
    b.extend_from_slice(&table_header(&cards, 3, 8, 9));
    let mut data = Vec::new();
    for (count, offset) in [(4i32, 0i32), (2, 4), (3, 6)] {
        data.extend_from_slice(&count.to_be_bytes());
        data.extend_from_slice(&offset.to_be_bytes());
    }
    data.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9]);
    b.extend_from_slice(&padded(data));
    b
}

#[test]
fn a_table_that_says_zimage_is_a_compressed_image_of_tiles() {
    let (d, mut ev) = eval(compressed("T"));
    let data = [0usize, 1, 3];
    let at = |tail: &[usize]| -> Vec<usize> { data.iter().chain(tail).copied().collect() };
    assert_eq!(ev.node(&d, &data).unwrap().type_name, "Compressed image");
    // What the image is, before the table it is written as.
    assert_eq!(text(&ev.node(&d, &at(&[0])).unwrap().value), "RICE_1");
    let axes = |ev: &mut Evaluator, field: usize| -> Vec<i128> {
        let n = ev.node(&d, &at(&[field])).unwrap().child_count as usize;
        (0..n).map(|i| ev.node(&d, &at(&[field, i])).unwrap().value.as_int().unwrap()).collect()
    };
    assert_eq!(axes(&mut ev, 1), vec![5, 3]);
    // The missing `ZTILE2` is one.
    assert_eq!(axes(&mut ev, 2), vec![5, 1]);
    // None of that covers a byte: the rows start where the data does.
    let rows = ev.node(&d, &at(&[4])).unwrap();
    assert_eq!(rows.offset_bits, ev.node(&d, &data).unwrap().offset_bits);
    // A row is a tile, and reads as the table's row would.
    assert_eq!(rows.child_count, 3);
    let tile = ev.node(&d, &at(&[4, 1])).unwrap();
    assert_eq!(tile.type_name, "Tile");
    assert_eq!(ev.node(&d, &at(&[4, 1, 0, 0])).unwrap().name, "[0] COMPRESSED_DATA");
    assert_eq!(ev.node(&d, &at(&[4, 1, 0, 0, 1, 0, 0])).unwrap().value, Value::Int(2));
    // And the heap is the tiles' bytes, where the rows point.
    let heap = ev.node(&d, &at(&[5])).unwrap();
    assert_eq!(heap.child_count, 3);
    let third = ev.node(&d, &at(&[5, 2])).unwrap();
    assert_eq!((third.type_name.as_str(), third.child_count), ("u8[]", 3));
    assert_eq!(third.offset_bits, heap.offset_bits + 6 * 8);
}

#[test]
fn a_table_that_says_zimage_is_false_is_a_table() {
    let (d, mut ev) = eval(compressed("F"));
    assert_eq!(ev.node(&d, &[0, 1, 3]).unwrap().type_name, "Table");
    assert_eq!(ev.node(&d, &[0, 1, 3, 2]).unwrap().child_count, 3);
}

/// A compressed image of one tile, four bytes stored as they are, in a table
/// whose `NAXIS1`, `NAXIS2` and `THEAP` cards say whatever they are given.
/// Each of `given` takes the place of the card with its keyword, or is added.
fn one_tile(width: usize, rows: usize, theap: Option<u64>, given: &[String]) -> Vec<u8> {
    let mut cards = vec![
        "TFIELDS =                    1".to_string(),
        "TTYPE1  = 'COMPRESSED_DATA'".into(),
        "TFORM1  = '1PB     '".into(),
        "ZIMAGE  =                    T".into(),
        "ZBITPIX =                    8".into(),
        "ZNAXIS  =                    1".into(),
        "ZNAXIS1 =                    4".into(),
        "ZCMPTYPE= 'NOCOMPRESS'".into(),
    ];
    cards.extend(theap.map(|t| format!("THEAP   = {t:20}")));
    for card in given {
        match cards.iter_mut().find(|c| c[..8] == card[..8]) {
            Some(c) => c.clone_from(card),
            None => cards.push(card.clone()),
        }
    }
    let refs: Vec<&str> = cards.iter().map(|s| s.as_str()).collect();
    let mut b = primary();
    b.extend_from_slice(&table_header(&refs, rows, width, 4));
    let mut data = Vec::new();
    data.extend_from_slice(&4i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&[1, 2, 3, 4]);
    b.extend_from_slice(&padded(data));
    b
}

/// A corrupt header can give a row width, a row count or a heap offset that is
/// more bits than a u64 counts. The tile is refused the way it would be for a
/// width or an offset merely past the end of the file, instead of overflowing.
#[test]
fn a_tile_placed_past_what_bits_can_count_is_refused() {
    let tile = |width: usize, rows: usize, theap: Option<u64>| {
        let (d, mut ev) = eval(one_tile(width, rows, theap, &[]));
        ev.fits_tile(&d, &[0, 1, 3]).map(|t| t.expect("a tile")).map_err(|e| match e {
            crate::eval::EvalError::Failed(s) => s,
            other => panic!("not a failure: {other:?}"),
        })
    };
    let fine = tile(8, 1, None).unwrap();
    assert_eq!((fine.problem, fine.pixels), (None, vec![1.0, 2.0, 3.0, 4.0]));
    // A row too wide: past a u64 once multiplied by eight, and only once added
    // to where the rows start.
    for width in [1 << 61, (1 << 61) - 1] {
        assert_eq!(tile(width, 1, None).unwrap_err(), "runs past the end of its container", "NAXIS1 = {width}");
    }
    // The heap starts after the rows when no THEAP says otherwise, and here the
    // rows are more bytes than a u64 counts. Then a THEAP past a u64 once in
    // bits, and once added to where the data starts.
    let past_heap = |t: crate::formats::fits_tile::Tile| t.problem.is_some_and(|p| p.starts_with("Not unpacked: the descriptor points past the end of the heap"));
    assert!(past_heap(tile(8, 1 << 62, None).unwrap()));
    for theap in [i64::MAX as u64, (1 << 61) - 1] {
        assert!(past_heap(tile(8, 1, Some(theap)).unwrap()), "THEAP = {theap}");
    }
}

/// A corrupt header can give axes whose product is more pixels, and so tiles
/// or a tile of more, than a u64 counts, or a data column wider than one
/// counts bytes. The tile is still placed and described, and not unpacked.
#[test]
fn a_tile_whose_header_counts_past_a_u64_is_described_and_not_unpacked() {
    let tile = |cards: &[String]| {
        let (d, mut ev) = eval(one_tile(8, 1, None, cards));
        ev.fits_tile(&d, &[0, 1, 3]).unwrap().expect("a tile")
    };
    let square = |tile: u64| -> Vec<String> {
        let side = 1u64 << 40;
        let mut cards = vec!["ZNAXIS  =                    2".to_string()];
        cards.extend([1, 2].map(|n| format!("ZNAXIS{n} = {side:20}")));
        cards.extend([1, 2].map(|n| format!("ZTILE{n}  = {tile:20}")));
        cards
    };
    let invalid = "Not unpacked: the header is invalid. ZNAXISn say the image is 1,099,511,627,776 Ã— 1,099,511,627,776 pixels, more than 2^64 in all.";
    // Tiles of one pixel, 2^80 of them.
    let t = tile(&square(1));
    assert_eq!((t.tiles, t.pixel_count(), t.shape, t.packed_bytes, t.problem.as_deref()), (None, Some(1), vec![1, 1], 4, Some(invalid)));
    // One tile of 2^80 pixels.
    let t = tile(&square(1 << 40));
    assert_eq!((t.tiles, t.pixel_count(), t.problem.as_deref()), (Some(1), None, Some(invalid)));
    // A data column 2^64 bytes wide has no cell in the row, so no bytes.
    let t = tile(&["TFORM1  = '2305843009213693952PB'".to_string()]);
    assert_eq!(t.problem.as_deref(), Some("Not unpacked: this tile's row has no bytes in COMPRESSED_DATA, GZIP_COMPRESSED_DATA or UNCOMPRESSED_DATA."));
}

/// `ZNAXIS` is six letters, so its number has two bytes to be written in
/// rather than three, and the keyword a tenth axis is found by is worked
/// out that way.
#[test]
fn a_six_letter_keyword_takes_two_digits() {
    let worked_out = |prefix: &str, n: i128| -> i128 {
        let t = Template::new("key", T::structure("Key", vec![("key", T::computed(numbered_key(prefix, E::lit(n))))]));
        let doc = Document::new(MemSource(Vec::new()));
        Evaluator::new(t).node(&doc, &[0]).unwrap().value.as_int().unwrap()
    };
    for (n, name) in [(1, "ZNAXIS1"), (9, "ZNAXIS9"), (10, "ZNAXIS10"), (42, "ZNAXIS42")] {
        assert_eq!(worked_out("ZNAXIS", n), keynum(name), "{name}");
    }
    for (n, name) in [(1, "ZTILE1"), (10, "ZTILE10"), (100, "ZTILE100")] {
        assert_eq!(worked_out("ZTILE", n), keynum(name), "{name}");
    }
}
