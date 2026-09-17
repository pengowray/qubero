//! dBase tables: the `.dbf` of dBase III, IV and 5, of FoxPro, and of every
//! shapefile's attribute table.
//!
//! A 32-byte header says how many records there are and how long the whole
//! header and one record are. Then one 32-byte descriptor per column, ended by
//! a lone 0x0D, and then the records, each a deletion flag and one fixed-width
//! text cell per column. Numbers are written as right-aligned decimal text, so
//! every cell is text and the descriptor's type only says how to read it. A
//! 0x1A after the last record is customary and not counted anywhere.
//!
//! What the record is made of is only in the descriptors, so the cells are
//! sized by looking the column up at the same index, the way `hdf4` sizes a
//! Vdata record from its header.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// The version byte, which really packs three things: the dBase generation
/// in the low bits, whether a memo file goes with the table in the high ones.
pub const VERSION: &[(i128, &str)] = &[
    (0x02, "FoxBASE"),
    (0x03, "dBase III without memo"),
    (0x04, "dBase IV without memo"),
    (0x05, "dBase 5 without memo"),
    (0x07, "Visual Objects without memo"),
    (0x30, "Visual FoxPro"),
    (0x31, "Visual FoxPro with autoincrement"),
    (0x32, "Visual FoxPro with varchar"),
    (0x43, "dBase IV SQL table without memo"),
    (0x63, "dBase IV SQL system"),
    (0x83, "dBase III with memo"),
    (0x87, "Visual Objects with memo"),
    (0x8b, "dBase IV with memo"),
    (0x8e, "dBase IV SQL table with memo"),
    (0xcb, "dBase IV SQL table with memo"),
    (0xf5, "FoxPro with memo"),
    (0xfb, "FoxBASE with memo"),
];

/// Column types, each a letter. Only the first four are dBase III's; the rest
/// came with IV, 5 and the FoxPros.
const FIELD_TYPE: &[(i128, &str)] = &[
    (b'C' as i128, "character"),
    (b'N' as i128, "numeric"),
    (b'L' as i128, "logical"),
    (b'D' as i128, "date"),
    (b'M' as i128, "memo"),
    (b'F' as i128, "float"),
    (b'B' as i128, "double"),
    (b'G' as i128, "general"),
    (b'I' as i128, "integer"),
    (b'P' as i128, "picture"),
    (b'T' as i128, "datetime"),
    (b'Y' as i128, "currency"),
    (b'+' as i128, "autoincrement"),
    (b'O' as i128, "double"),
    (b'@' as i128, "timestamp"),
    (b'V' as i128, "varchar"),
    (b'Q' as i128, "varbinary"),
];

pub fn dbf() -> Template {
    Template::new(
        "dbf",
        T::structure(
            "Dbf",
            vec![
                ("version", T::enumeration_hex("DbfVersion", T::u8(), VERSION)),
                ("year", T::u8()),
                ("month", T::u8()),
                ("day", T::u8()),
                ("record_count", T::u32(Little)),
                ("header_length", T::u16(Little)),
                ("record_length", T::u16(Little)),
                ("reserved", T::bytes(E::lit(20))),
                // One descriptor per column, and the byte after the last one is
                // the 0x0D. Counted this way rather than worked out from
                // `header_length`, because a FoxPro writer puts 263 bytes of
                // database backlink between the 0x0D and the records.
                ("fields", T::repeat(field(), Until::While(E::peek(8, Big).not_equal(E::lit(0x0D))))),
                ("terminator", T::magic(b"\x0d")),
                ("header_rest", T::bytes(E::field("header_length").sub(E::Pos))),
                ("records", T::array(T::sized(E::field("record_length"), record()), E::field("record_count"))),
                ("end_of_file", T::if_room(T::magic(b"\x1a"))),
            ],
        )
        .field_doc("year", "Years since 1900, as the table was last written.")
        .field_doc("header_length", "Bytes before the first record: this header, the descriptors, the 0x0D, and anything a writer put after it.")
        .field_doc("record_length", "One more than the columns' widths added up, for the deletion flag.")
        .field_elem_named_from("fields", E::elem_field("fields", E::idx(), &["name"]))
        .counted_as("record")
        .machinery(&["fields", "terminator", "header_rest"])
        .payload(&["records"]),
    )
}

/// One column: an eleven-byte name, a type letter, and a width. The rest of
/// the 32 bytes changed meaning with every dBase, and only the decimal count
/// of a number is the same in all of them.
fn field() -> T {
    T::inline_structure(
        "DbfField",
        vec![
            ("name", T::text(StrLen::Padded { size: E::lit(11), pad: 0 }, Encoding::Ascii)),
            ("type", T::enumeration("DbfFieldType", T::u8(), FIELD_TYPE)),
            ("address", T::u32(Little)),
            ("length", T::u8()),
            ("decimals", T::u8()),
            ("rest", T::bytes(E::lit(14))),
        ],
    )
    .field_doc("address", "Where dBase III kept the column in memory; nothing in the file.")
    .counted_as("field")
}

/// One record: whether it has been deleted, then one cell per column, as wide
/// as that column's descriptor says and named after it.
fn record() -> T {
    T::inline_structure(
        "DbfRecord",
        vec![
            ("deleted", T::enumeration("DbfDeleted", T::u8(), &[(0x20, "no"), (0x2a, "yes")])),
            ("values", T::array(cell(), E::len_of("fields"))),
        ],
    )
    .field_elem_named_from("values", E::elem_field("fields", E::idx(), &["name"]))
}

/// One cell, as wide as its column and padded with spaces: numbers on the
/// left, text on the right. The bytes are whatever the DOS code page was,
/// which for nearly every table ever written is 437.
fn cell() -> T {
    T::text(StrLen::Padded { size: E::elem_field("fields", E::idx(), &["length"]), pad: b' ' }, Encoding::Cp437)
}

/// Whether these are the front bytes of a dBase table. Nothing marks one, so
/// the whole header has to agree with itself: a known version, a date that is
/// one, a header length that is 32 bytes plus 32 per column plus the 0x0D,
/// with the 0x0D there, a record length that is one more than the columns'
/// widths add up to, and a file that is exactly the header and the records,
/// with or without the customary 0x1A after them.
///
/// `head` may stop short of the header, so every read of it is a `get`, and
/// what is out of reach is taken on trust; the length equation still has to
/// hold. A FoxPro backlink after the 0x0D is allowed, the one departure from
/// the dBase III shape.
pub fn is_dbf(head: &[u8], len: u64) -> bool {
    let Some(&version) = head.first() else { return false };
    if !VERSION.iter().any(|(v, _)| *v == version as i128) {
        return false;
    }
    let (Some(&month), Some(&day)) = (head.get(2), head.get(3)) else { return false };
    if month > 12 || day > 31 {
        return false;
    }
    let u16_at = |at: usize| head.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as u64);
    let (Some(records), Some(header_length), Some(record_length)) = (
        head.get(4..8).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as u64),
        u16_at(8),
        u16_at(10),
    ) else {
        return false;
    };
    // 32 for this header, 32 per column, one for the 0x0D; a FoxPro table
    // then has 263 bytes of backlink. Anything else is not a header.
    const BACKLINK: u64 = 263;
    let descriptors = match header_length.checked_sub(33) {
        Some(n) if n % 32 == 0 => n,
        Some(n) if n >= BACKLINK && (n - BACKLINK) % 32 == 0 => n - BACKLINK,
        _ => return false,
    };
    let fields = descriptors / 32;
    if fields == 0 || record_length == 0 {
        return false;
    }
    if len != header_length + records * record_length && len != header_length + records * record_length + 1 {
        return false;
    }
    if head.get(header_length as usize - 1).is_some_and(|&b| b != 0x0D) {
        return false;
    }
    // Each descriptor in reach: a name that could be one, a type letter that
    // is one, and a width. The widths add up to the record, less its flag.
    let mut width = 1;
    for i in 0..fields as usize {
        let Some(d) = head.get(32 + i * 32..64 + i * 32) else { return width <= record_length };
        let name_ok = d[0].is_ascii_alphabetic() || d[0] == b'_';
        if !name_ok || !FIELD_TYPE.iter().any(|(t, _)| *t == d[11] as i128) {
            return false;
        }
        width += d[16] as u64;
    }
    width == record_length
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A table of one character column and one row: the smallest there is.
    pub fn one_station() -> Vec<u8> {
        let mut v = vec![0x03, 26, 9, 15];
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&65u16.to_le_bytes());
        v.extend_from_slice(&13u16.to_le_bytes());
        v.extend_from_slice(&[0; 20]);
        v.extend_from_slice(b"STATION\0\0\0\0C\0\0\0\0\x0c\0");
        v.extend_from_slice(&[0; 14]);
        v.push(0x0D);
        v.extend_from_slice(b" A           ");
        v.push(0x1A);
        assert_eq!(v.len(), 79);
        v
    }

    #[test]
    fn a_table_of_one_column_and_one_row_reads_as_that() {
        let d = Document::new(MemSource(one_station()));
        let mut ev = Evaluator::new(dbf());
        assert_eq!(ev.node(&d, &[4]).unwrap().value, Value::UInt(1));
        let fields = ev.node(&d, &[8]).unwrap();
        assert_eq!(fields.child_count, 1);
        assert_eq!(ev.node(&d, &[8, 0, 0]).unwrap().value, Value::Str("STATION".into()));
        assert_eq!(ev.node(&d, &[8, 0, 3]).unwrap().value, Value::UInt(12));
        // The one record: a flag and a cell twelve bytes wide.
        let records = ev.node(&d, &[11]).unwrap();
        assert_eq!(records.child_count, 1);
        let cell = ev.node(&d, &[11, 0, 1, 0]).unwrap();
        assert_eq!(cell.offset_bits, 66 * 8);
        assert_eq!(cell.size_bits, 12 * 8);
        assert_eq!(cell.value, Value::Str("A".into()));
        assert_eq!(ev.node(&d, &[12]).unwrap().offset_bits, 78 * 8);
    }

    #[test]
    fn the_probe_takes_the_smallest_table_and_the_one_without_the_end_mark() {
        let bytes = one_station();
        assert!(is_dbf(&bytes, bytes.len() as u64));
        assert!(is_dbf(&bytes[..78], 78));
        // A head cut short of the descriptors still answers from the arithmetic.
        assert!(is_dbf(&bytes[..12], 79));
    }

    #[test]
    fn the_probe_refuses_what_only_looks_like_one() {
        let bytes = one_station();
        let n = bytes.len() as u64;
        let mut wrong_version = bytes.clone();
        wrong_version[0] = 0x06;
        assert!(!is_dbf(&wrong_version, n));
        let mut no_terminator = bytes.clone();
        no_terminator[64] = 0;
        assert!(!is_dbf(&no_terminator, n));
        let mut header_off = bytes.clone();
        header_off[8] = 66;
        assert!(!is_dbf(&header_off, n));
        let mut record_off = bytes.clone();
        record_off[10] = 14;
        assert!(!is_dbf(&record_off, n));
        let mut month_off = bytes.clone();
        month_off[2] = 13;
        assert!(!is_dbf(&month_off, n));
        let mut bad_type = bytes.clone();
        bad_type[43] = b'X';
        assert!(!is_dbf(&bad_type, n));
        // The length has to be the header and the records and at most one more.
        assert!(!is_dbf(&bytes, n + 1));
        assert!(!is_dbf(&bytes, n - 2));
        // Noise, however much of it opens with a version byte.
        let mut noise = vec![0x03u8; 256];
        for (i, b) in noise.iter_mut().enumerate().skip(1) {
            *b = (i as u8).wrapping_mul(97).wrapping_add(13);
        }
        assert!(!is_dbf(&noise, 256));
        assert!(!is_dbf(&[], 0));
        assert!(!is_dbf(&[0x03, 0, 0], 3));
    }
}
