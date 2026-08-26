//! PDF: a header line, a heap of numbered objects, and a table at the end
//! saying where every one of them is written.
//!
//! The table is the whole point of the format. Nothing in the body says which
//! object comes next, and a reader is not meant to walk it: it reads the last
//! line but one for a byte offset, goes there for the cross-reference table,
//! and from then on reaches any object it wants in one jump. That is the same
//! shape as a WAD, and it is placed the same way, by fields that cost no bytes
//! where they are declared and read their contents somewhere else.
//!
//! Two things stood in the way, and both were gaps in the IR rather than in
//! the format.
//!
//! The first is that every number here is written as digits. A field read as
//! text is its bytes when an expression asks for it, so the offset `408` would
//! come out as 0x343038 and point three megabytes past the end of a file that
//! is four hundred bytes long. Digits that are a number have to be parsed
//! where the field is read, which is what `TextInt` is.
//!
//! The second is that the pointer at the end cannot be found by counting back
//! from it. `startxref` is followed by an offset of no fixed width and an
//! end-of-file marker written with whichever of three line endings the writer
//! liked, so the word itself has to be searched for. `ToMarker` looks for one
//! byte, which is no use for a format that writes its structure in words, so
//! `Find` looks for the word, and for the last one of them: a file that has
//! been added to keeps every table it ever had, and the one that counts is the
//! one written last. That search runs from the end of the file rather than the
//! front, which is not a nicety: a reader holding a window rather than a whole
//! file cannot walk three hundred megabytes to reach a word written forty
//! bytes from the end.
//!
//! What that leaves is a header that is text, a table that is text, a trailer
//! that is text, and a body placed entirely by the table.
//!
//! The table is not one run of entries but as many as the writer felt like.
//! The spec calls each one a subsection, and each is headed by the object
//! number it starts at and how many entries follow. Nothing says how many
//! subsections there are; what says the table has ended is the word `trailer`
//! after it. So the entries are one flat list measured to that word, and each
//! line of it is read as whichever of the two it looks like: an entry writes
//! `n` or `f` seventeen bytes in and a heading never does. Two numbers where
//! twenty bytes were expected costs nothing, because a line that starts where
//! the one before it ended is how the rest of the table is read anyway.
//!
//! What is not here. A cross-reference *stream*, which is how PDF 1.5 and
//! later may write the same table compressed inside an object, is not read;
//! nor is the `/Prev` chain that an incrementally saved or linearized file
//! leaves behind, so only the most recent table is followed and the objects
//! the ones before it place are not shown. A free entry other than the first
//! holds the number of the next free object rather than an offset, and nothing
//! here tells it apart from a real one, so an object placed by one is
//! whatever happens to be at that offset. The first free entry is the head of
//! that list and always writes zero, which is read as pointing at nothing.

use crate::template::{Anchor, Encoding, Endian::Big, Expr as E, StrLen, Template, Ty as T, Until};

/// The six bytes PDF counts as white space between one token and the next.
const SPACE: &[u8] = b" \t\r\n\x0c\0";

/// Where the table's lines begin: just past the word `xref`. The line ending
/// after it belongs to the first line, which skips whatever white space it
/// starts with.
fn after_xref() -> E {
    E::field("xref_offset").add(E::lit(4))
}

/// Where the trailer dictionary begins: the end of the table.
fn after_table() -> E {
    after_xref().add(E::size_of("entries"))
}

pub fn pdf() -> Template {
    Template::new(
        "pdf",
        T::structure(
            "PDF",
            vec![
                // `%PDF-1.7`, and the one byte that ends the line. Which byte
                // that is cannot be assumed: a file written on a Mac in 1996
                // ends every line with a carriage return and may hold no line
                // feed at all, and a header that ran to the first one would
                // swallow the objects the table is about to place.
                (
                    "version",
                    T::text(StrLen::token(b"", b"\r\n"), Encoding::Ascii),
                ),
                // Where the last `startxref` is written, counted from the
                // start of the file: the search runs from this field, and the
                // header is what is in front of it.
                ("startxref_at", T::computed(E::to_last_bytes(b"startxref").add(E::size_of("version")))),
                // The offset that word points at, which is where the table is.
                ("xref_offset", T::at(E::field("startxref_at").add(E::lit(9)), number())),
                // From here down, every field is read where the table is and
                // takes up no room where it is declared.
                ("xref", T::at(E::field("xref_offset"), T::magic(b"xref"))),
                // Every line of the table, headings and entries together, up
                // to the word that ends it.
                (
                    "entries",
                    T::at(after_xref(), T::sized(E::to_bytes(b"trailer"), T::repeat(line(), Until::End))),
                ),
                // `trailer` and the dictionary after it, which runs from the
                // end of the table to the next `startxref`. Not to the last
                // one: a file that has been saved twice may keep its newest
                // table in front of everything the older one wrote, and the
                // trailer belongs to the table above it either way.
                (
                    "trailer",
                    T::at(after_table(), T::text(StrLen::Fixed(E::to_bytes(b"startxref")), Encoding::Ascii)),
                ),
                ("startxref", T::at(E::field("startxref_at"), T::magic(b"startxref"))),
                // Whatever the file ends with: `%%EOF` and the line ending, or
                // no line ending, or a stray newline in front of it.
                (
                    "eof",
                    T::at(
                        E::field("startxref_at").add(E::lit(9)).add(E::size_of("xref_offset")),
                        T::text(StrLen::Fixed(E::Remaining), Encoding::Ascii),
                    ),
                ),
                // Everything after the header, with each object at the offset
                // its own entry names. The table, the trailer and the end
                // marker sit in that stretch and belong to no object; they are
                // declared above, so the cursor lands in them rather than in
                // the space around an object.
                //
                // A heading has no offset in it at all, which is what a
                // pointer list that skips what it cannot read makes of a line
                // placing nothing.
                (
                    "objects",
                    T::pointer_list_sized("entries", &["offset"], Anchor::File, E::lit(0), object()).skipping_zero(),
                ),
            ],
        ),
    )
}

/// One line of the table: an entry that places an object, or the heading that
/// starts a subsection of them.
///
/// Which it is has to be looked at rather than counted, since nothing in front
/// of the table says how many subsections it holds. An entry writes `n` or `f`
/// after its two numbers and a heading, being two numbers and nothing else,
/// cannot.
fn line() -> T {
    T::switch(too_short_for_an_entry().or(letter_somewhere()), vec![(0, entry())], heading())
}

/// Zero while twenty bytes of table are left, and something else once fewer
/// than that are.
///
/// A look-ahead that reads past the end of the table is an error, and it takes
/// the table, the trailer and every object with it. There is a table that ends
/// that way and it is not a broken one: a hybrid-reference file writes its real
/// table as a stream inside an object and leaves a `0 0` here, an empty
/// subsection, so that a reader too old to know about streams finds something
/// where it looks. The heading is then the only line, and there are no twenty
/// bytes behind it to look into.
///
/// A line too short to be an entry is a heading whatever its bytes say, so the
/// answer is known without looking and `Or` is what stops the looking: it takes
/// the second of the two only when the first is zero. Twenty divided by what is
/// left is that test, since a division rounds towards nothing.
fn too_short_for_an_entry() -> E {
    E::lit(20).div(E::Remaining.add(E::lit(1)))
}

/// Zero when one of the three bytes an entry's letter could be written at is
/// `n` or `f`. Only asked where there are twenty bytes to ask about; see
/// [`too_short_for_an_entry`].
///
/// The letter belongs seventeen bytes in, and would be looked for there and
/// nowhere else if every table were laid out the way the spec says. A line can
/// start a byte or two late, though, because the one above it ends in whatever
/// the writer felt like and the field that measures it takes one byte of that:
/// the lines themselves step over what is left, so they read the same, and
/// only a fixed look-ahead notices.
///
/// Subtracting a letter from a byte makes a zero of the one that is it, and
/// multiplying the six makes a zero of the lot: a product is zero when any of
/// its parts is. A heading cannot be mistaken for an entry this way. Its own
/// bytes are digits, spaces and line endings, and by the seventeenth the line
/// after it has begun, which at that point is still writing digits.
fn letter_somewhere() -> E {
    let at = |n: i128| {
        let b = E::peek_at(E::lit(n * 8), 8, Big);
        b.clone().sub(E::lit(0x6e)).mul(b.sub(E::lit(0x66)))
    };
    at(17).mul(at(18)).mul(at(19))
}

/// The head of a subsection: the object number its first entry is for, and how
/// many entries follow. Neither is a fixed width, and the second starts
/// wherever the first ended.
fn heading() -> T {
    T::inline_structure("Subsection", vec![("first_object", number()), ("entry_count", number())])
        .counted_as("subsection")
}

/// One entry of the table, twenty bytes: where the object is, how many times
/// it has been replaced, and whether it is there at all.
///
/// The last of those is written seventeen bytes in, after the two numbers it
/// decides the meaning of, so the entry is picked by looking ahead at it. A
/// free entry's ten digits are not an offset at all: they are the number of
/// the next free object, a list threaded through the table with its head in
/// entry zero. Reading them as an offset is how a reader ends up parsing
/// whatever happens to be twelve bytes into the file.
///
/// The spec fixes the widths at ten digits and five, and then fixes the whole
/// entry at twenty bytes, which only works out when the line ending is two
/// bytes. Writers disagree about that often enough that the digits are scanned
/// rather than counted: a run that starts a byte late reads the same.
fn entry() -> T {
    T::switch(free_somewhere(), vec![(0, free_entry())], used_entry())
}

/// Zero when one of the three bytes an `f` could be written at is one, told
/// apart from an `n` the same way [`letter_somewhere`] tells either from a
/// digit.
fn free_somewhere() -> E {
    let at = |n: i128| E::peek_at(E::lit(n * 8), 8, Big).sub(E::lit(0x66));
    at(17).mul(at(18)).mul(at(19))
}

/// An entry that places an object.
fn used_entry() -> T {
    T::inline_structure(
        "Entry",
        vec![
            ("offset", T::decimal(StrLen::token(SPACE, b" "))),
            ("generation", generation()),
            ("kind", kind()),
            ("eol", T::bytes(E::lit(2))),
        ],
    )
    .counted_as("entry")
}

/// An entry for an object that is not there. Its offset is a field of no bits
/// worth nothing, which is what the list reads as pointing nowhere; the digits
/// where an offset would be are the next free object's number.
fn free_entry() -> T {
    T::inline_structure(
        "Free",
        vec![
            ("offset", T::computed(E::lit(0))),
            ("next_free", T::decimal(StrLen::token(SPACE, b" "))),
            ("generation", generation()),
            ("kind", kind()),
            ("eol", T::bytes(E::lit(2))),
        ],
    )
    .counted_as("entry")
}

/// How many times the object at this number has been replaced. A free entry
/// carries the number the next object to take its place will have.
fn generation() -> T {
    T::decimal(StrLen::token(b" ", b" "))
}

fn kind() -> T {
    T::enumeration("Use", T::u8(), &[(0x6e, "in use"), (0x66, "free")])
}

/// One indirect object: its number, the revision of it, and everything between
/// `obj` and `endobj`.
///
/// Nothing says how long the body is, and nothing needs to: the word `endobj`
/// closes it, wherever that falls. The body is left whole rather than taken
/// apart, so a dictionary, a stream and its bytes all read as the one run.
fn object() -> T {
    T::structure_named(
        "Object",
        "number",
        "body",
        vec![
            ("number", number()),
            ("generation", number()),
            // `obj`, and nothing after it. The word is three bytes and is not
            // a token measured to the next space: what follows it is usually
            // `<<` written straight against it, and reading to the space after
            // that would take the first key of the dictionary with it.
            ("keyword", T::magic(b"obj")),
            ("body", T::bytes(E::to_bytes(b"endobj"))),
            ("endobj", T::magic(b"endobj")),
        ],
    )
    .counted_as("object")
}

/// A number written as digits, with the white space in front of it and the one
/// byte that ends it.
fn number() -> T {
    T::decimal(StrLen::token(SPACE, SPACE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// Where the table is: the last `xref` that starts a line, which is not
    /// the one inside the `startxref` below it.
    fn table_at(bytes: &[u8]) -> usize {
        bytes.windows(5).rposition(|w| w == b"\nxref").expect("a table") + 1
    }

    /// A three-object file with a classic table, built the way a writer builds
    /// one: the objects first, then the offsets they landed at.
    fn pdf_bytes(eol: &str) -> Vec<u8> {
        // An entry is twenty bytes however the rest of the file is written, so
        // a one-byte line ending is padded with the space the spec asks for.
        let entry_eol = if eol.len() == 1 { format!(" {eol}") } else { eol.to_string() };
        let bodies = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] >>", "<< /Type /Page >>"];
        let mut v = format!("%PDF-1.7{eol}").into_bytes();
        let mut at = Vec::new();
        for (i, body) in bodies.iter().enumerate() {
            at.push(v.len());
            v.extend_from_slice(format!("{} 0 obj{eol}{body}{eol}endobj{eol}", i + 1).as_bytes());
        }
        let table = v.len();
        v.extend_from_slice(format!("xref{eol}0 {}{eol}", bodies.len() + 1).as_bytes());
        v.extend_from_slice(format!("0000000000 65535 f{entry_eol}").as_bytes());
        for a in &at {
            v.extend_from_slice(format!("{a:010} 00000 n{entry_eol}").as_bytes());
        }
        v.extend_from_slice(format!("trailer{eol}<< /Size 4 /Root 1 0 R >>{eol}").as_bytes());
        v.extend_from_slice(format!("startxref{eol}{table}{eol}%%EOF").as_bytes());
        v
    }

    /// The same file, but with its table split into a subsection per object
    /// the way a writer that has saved four times leaves it.
    fn pdf_bytes_in_subsections(eol: &str) -> Vec<u8> {
        let entry_eol = if eol.len() == 1 { format!(" {eol}") } else { eol.to_string() };
        let bodies = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] >>", "<< /Type /Page >>"];
        let mut v = format!("%PDF-1.7{eol}").into_bytes();
        let mut at = Vec::new();
        for (i, body) in bodies.iter().enumerate() {
            at.push(v.len());
            v.extend_from_slice(format!("{} 0 obj{eol}{body}{eol}endobj{eol}", i + 1).as_bytes());
        }
        let table = v.len();
        v.extend_from_slice(format!("xref{eol}0 1{eol}").as_bytes());
        v.extend_from_slice(format!("0000000000 65535 f{entry_eol}").as_bytes());
        for (i, a) in at.iter().enumerate() {
            v.extend_from_slice(format!("{} 1{eol}", i + 1).as_bytes());
            v.extend_from_slice(format!("{a:010} 00000 n{entry_eol}").as_bytes());
        }
        v.extend_from_slice(format!("trailer{eol}<< /Size 4 /Root 1 0 R >>{eol}").as_bytes());
        v.extend_from_slice(format!("startxref{eol}{table}{eol}%%EOF").as_bytes());
        v
    }

    #[test]
    fn the_table_is_found_by_the_word_at_the_end() {
        let bytes = pdf_bytes("\n");
        let table = table_at(&bytes);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(pdf());
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Str("%PDF-1.7".into()));
        // The offset is digits, and reads as the number those digits spell
        // rather than as the bytes they are.
        assert_eq!(ev.node(&d, &[2, 0]).unwrap().value, Value::Int(table as i128));
        // The word it points at is there.
        assert_eq!(ev.node(&d, &[3, 0]).unwrap().value, Value::Magic { ok: true, bytes: b"xref".to_vec() });
        // The first line of the table is the heading of its one subsection.
        assert_eq!(ev.node(&d, &[4, 0, 0]).unwrap().type_name, "Subsection");
        assert_eq!(ev.node(&d, &[4, 0, 0, 0]).unwrap().value, Value::Int(0));
        assert_eq!(ev.node(&d, &[4, 0, 0, 1]).unwrap().value, Value::Int(4));
    }

    #[test]
    fn every_entry_reads_and_the_free_one_points_at_nothing() {
        let d = Document::new(MemSource(pdf_bytes("\n")));
        let mut ev = Evaluator::new(pdf());
        let entries = ev.node(&d, &[4, 0]).unwrap();
        // One heading and the four entries under it.
        assert_eq!(entries.child_count, 5);
        // The head of the free list: it points at no object, and its digits
        // are the next free object's number rather than an offset.
        assert_eq!(ev.node(&d, &[4, 0, 1]).unwrap().type_name, "Free");
        assert_eq!(ev.node(&d, &[4, 0, 1, 0]).unwrap().value, Value::Int(0));
        assert_eq!(ev.node(&d, &[4, 0, 1, 0]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[4, 0, 1, 1]).unwrap().value, Value::Int(0));
        assert_eq!(ev.node(&d, &[4, 0, 1, 2]).unwrap().value, Value::Int(65535));
        assert_eq!(
            ev.node(&d, &[4, 0, 1, 3]).unwrap().value,
            Value::Enum { raw: 0x66, name: Some("free".into()), hex: false }
        );
        // An entry that does place an object keeps the plain shape.
        assert_eq!(ev.node(&d, &[4, 0, 2]).unwrap().type_name, "Entry");
        // Every entry is twenty bytes.
        assert_eq!(ev.node(&d, &[4, 0, 1]).unwrap().size_bits, 20 * 8);
        assert_eq!(ev.node(&d, &[4, 0, 4]).unwrap().size_bits, 20 * 8);
    }

    #[test]
    fn the_objects_are_placed_by_the_table() {
        let bytes = pdf_bytes("\n");
        let first = bytes.windows(7).position(|w| w == b"1 0 obj").expect("an object");
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(pdf());
        let objects = ev.node(&d, &[8]).unwrap();
        // Five lines: the heading places nothing and neither does the free
        // entry, so three of them are objects.
        assert_eq!(objects.child_count, 5);
        assert_eq!(ev.node(&d, &[8, 0]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[8, 1]).unwrap().size_bits, 0);
        let one = ev.node(&d, &[8, 2]).unwrap();
        assert_eq!(one.offset_bits, first as u64 * 8);
        assert_eq!(ev.node(&d, &[8, 2, 0]).unwrap().value, Value::Int(1));
        assert_eq!(ev.node(&d, &[8, 2, 1]).unwrap().value, Value::Int(0));
        assert_eq!(
            ev.node(&d, &[8, 2, 2]).unwrap().value,
            Value::Magic { ok: true, bytes: b"obj".to_vec() }
        );
        // The body runs to `endobj`, which is the field after it.
        assert_eq!(
            ev.node(&d, &[8, 2, 4]).unwrap().value,
            Value::Magic { ok: true, bytes: b"endobj".to_vec() }
        );
        assert_eq!(ev.node(&d, &[8, 4, 0]).unwrap().value, Value::Int(3));
    }

    /// A table written as a subsection per object reads as the same objects:
    /// the headings between them are lines that place nothing.
    #[test]
    fn a_table_written_in_several_subsections_places_every_object() {
        for eol in ["\n", "\r\n"] {
            let bytes = pdf_bytes_in_subsections(eol);
            let first = bytes.windows(7).position(|w| w == b"1 0 obj").expect("an object");
            let d = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(pdf());
            // Four headings, four entries.
            assert_eq!(ev.node(&d, &[4, 0]).unwrap().child_count, 8, "{eol:?}");
            assert_eq!(ev.node(&d, &[4, 0, 0]).unwrap().type_name, "Subsection", "{eol:?}");
            assert_eq!(ev.node(&d, &[4, 0, 1]).unwrap().type_name, "Free", "{eol:?}");
            assert_eq!(ev.node(&d, &[4, 0, 2]).unwrap().type_name, "Subsection", "{eol:?}");
            assert_eq!(ev.node(&d, &[4, 0, 2, 0]).unwrap().value, Value::Int(1), "{eol:?}");
            assert_eq!(ev.node(&d, &[4, 0, 3]).unwrap().type_name, "Entry", "{eol:?}");
            // The heading is a line of the table and a child of the object
            // list, and places nothing; the entry under it places object one.
            assert_eq!(ev.node(&d, &[8, 2]).unwrap().size_bits, 0, "{eol:?}");
            assert_eq!(ev.node(&d, &[8, 3]).unwrap().offset_bits, first as u64 * 8, "{eol:?}");
            assert_eq!(ev.node(&d, &[8, 3, 0]).unwrap().value, Value::Int(1), "{eol:?}");
            // And the trailer is the trailer, not the subsections the old reading
            // swallowed along with it.
            let trailer = ev.node(&d, &[5, 0]).unwrap().value;
            assert_eq!(trailer, Value::Str(format!("trailer{eol}<< /Size 4 /Root 1 0 R >>{eol}")), "{eol:?}");
        }
    }

    #[test]
    fn the_trailer_and_the_end_marker_read_as_the_text_they_are() {
        let d = Document::new(MemSource(pdf_bytes("\n")));
        let mut ev = Evaluator::new(pdf());
        assert_eq!(ev.node(&d, &[5, 0]).unwrap().value, Value::Str("trailer\n<< /Size 4 /Root 1 0 R >>\n".into()));
        assert_eq!(ev.node(&d, &[6, 0]).unwrap().value, Value::Magic { ok: true, bytes: b"startxref".to_vec() });
        assert_eq!(ev.node(&d, &[7, 0]).unwrap().value, Value::Str("%%EOF".into()));
    }

    /// The same file with the other line ending, where nothing is where it was
    /// and every number is a byte wider.
    #[test]
    fn a_file_written_with_crlf_reads_the_same() {
        let bytes = pdf_bytes("\r\n");
        let table = table_at(&bytes);
        let first = bytes.windows(7).position(|w| w == b"1 0 obj").expect("an object");
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(pdf());
        assert_eq!(ev.node(&d, &[2, 0]).unwrap().value, Value::Int(table as i128));
        assert_eq!(ev.node(&d, &[4, 0, 0, 1]).unwrap().value, Value::Int(4));
        assert_eq!(ev.node(&d, &[4, 0, 4, 1]).unwrap().value, Value::Int(0));
        assert_eq!(ev.node(&d, &[8, 2]).unwrap().offset_bits, first as u64 * 8);
    }

    #[test]
    fn a_pdf_is_known_by_the_word_it_starts_with() {
        assert_eq!(crate::formats::sniff(b"%PDF-1.7\n1 0 obj\n"), Some("pdf"));
    }

    /// A file that has been saved twice holds two tables, and the one that
    /// counts is the one written last.
    #[test]
    fn the_last_table_is_the_one_followed() {
        let mut bytes = pdf_bytes("\n");
        let second = bytes.len();
        // An added revision: one more object, its own table, and its own
        // pointer at the end.
        bytes.extend_from_slice(b"\n4 0 obj\n<< /Type /Font >>\nendobj\n");
        let table = bytes.len();
        bytes.extend_from_slice(format!("xref\n0 1\n0000000000 65535 f\ntrailer\n<< /Size 5 >>\nstartxref\n{table}\n%%EOF").as_bytes());
        assert!(second < table);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(pdf());
        assert_eq!(ev.node(&d, &[2, 0]).unwrap().value, Value::Int(table as i128));
        assert_eq!(ev.node(&d, &[4, 0, 0, 1]).unwrap().value, Value::Int(1));
    }
    /// A table with an empty subsection and no entries at all, which is what a
    /// hybrid-reference file writes: the real table is a stream inside an
    /// object, and this one is here so that a reader too old to know about
    /// that finds something where it looks. The heading is the only line, and
    /// there are no twenty bytes behind it to look ahead into.
    #[test]
    fn a_table_with_an_empty_subsection_still_reads() {
        let mut v = b"%PDF-1.5\n1 0 obj\n<< /Type /Catalog >>\nendobj\n".to_vec();
        let table = v.len();
        v.extend_from_slice(b"xref\n0 0\ntrailer\n<< /Size 1 /XRefStm 17 >>\n");
        v.extend_from_slice(format!("startxref\n{table}\n%%EOF").as_bytes());
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(pdf());
        assert_eq!(ev.node(&d, &[2, 0]).unwrap().value, Value::Int(table as i128));
        let lines = ev.node(&d, &[4, 0]).unwrap();
        assert_eq!(lines.child_count, 1);
        assert_eq!(ev.node(&d, &[4, 0, 0]).unwrap().type_name, "Subsection");
        assert_eq!(ev.node(&d, &[4, 0, 0, 1]).unwrap().value, Value::Int(0));
        assert_eq!(
            ev.node(&d, &[5, 0]).unwrap().value,
            Value::Str("trailer\n<< /Size 1 /XRefStm 17 >>\n".into())
        );
        assert_eq!(ev.node(&d, &[8]).unwrap().child_count, 1);
        assert_eq!(ev.node(&d, &[8, 0]).unwrap().size_bits, 0);
    }
}
