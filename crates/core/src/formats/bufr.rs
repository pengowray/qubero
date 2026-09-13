//! BUFR: how the world's weather observations travel.
//!
//! A radiosonde ascent, a synoptic report from a land station, a ship's log,
//! an aircraft report, a satellite's soundings: the WMO's Global
//! Telecommunication System carries all of them as BUFR, the Binary Universal
//! Form for the Representation of meteorological data, FM 94 in the WMO Manual
//! on Codes. GRIB is its sibling for gridded fields, and the two are read the
//! same way: messages back to back, each opening with four letters and saying
//! how long it is, and inside one a fixed run of sections.
//!
//! A message is six sections. Section 0 is `BUFR`, the total length in three
//! bytes and the edition. Section 1 says who wrote it, which version of the
//! tables it was written against, what kind of data it is and when. Section 2
//! is the writer's own and is only there when a flag in section 1 says so.
//! Section 3 is the list of descriptors. Section 4 is the data, and section 5
//! is `7777`. Unlike GRIB 2's sections, BUFR's carry no numbers: which is which
//! is settled by the order they come in, so they are declared as fields rather
//! than as a run, the way GRIB 1's are.
//!
//! Editions 3 and 4 are what is written now, and they differ only in section 1:
//! edition 4 widened the centre and sub-centre to sixteen bits each, gave the
//! year four digits instead of two, added a second and an international data
//! sub-category, and swapped the order the centre and sub-centre come in.
//! Edition 2 is edition 3 with the centre alone in the two bytes where edition
//! 3 later put a sub-centre and a centre. Editions 0 and 1 have no total length
//! in section 0, so there is nothing to size one of those messages by; they
//! read as bytes up to the next message.
//!
//! What a message holds is the descriptors, and the descriptors are the whole
//! difficulty of the format. Each is sixteen bits: F in two, X in six, Y in
//! eight. F says what kind of descriptor it is. An element (F=0) names a value
//! in Table B, which says how wide it is, its unit and how it is scaled. A
//! replication (F=1) repeats the next X descriptors Y times, or, with Y of
//! zero, as many times as a count in the data says. An operator (F=2) changes
//! how what follows is read, from Table C. A sequence (F=3) stands for a list
//! of other descriptors, from Table D. Section 3 lists them at their shortest,
//! with sequences unexpanded, and section 4 is the values of the list once it
//! is expanded, packed end to end with no boundaries between them.
//!
//! So section 4 stays bytes here. Where its values are is a walk through the
//! tables and the operators that no expression could make, and for a
//! compressed message it is a reference, a width and an increment per subset
//! for every value. That walk is [`bufr_data`](super::bufr_data), and it is
//! what the panel on section 4 shows.
//!
//! What the tree does name is each descriptor in section 3, by the WMO's own
//! name for it from the current tables. The names are the same in every
//! version; what differs between versions is widths and scales, which only
//! the data needs.
//!
//! A message sent over the GTS arrives inside an envelope: a start-of-heading
//! byte, a sequence number, the abbreviated heading that says what bulletin
//! it is and who sent it (`ISMD01 OKPR 211200`), and after `7777` a line end
//! and an end-of-text byte. Those read as the text they are, between the
//! messages, so a file of several bulletins reads as its messages all the
//! same.

use std::sync::Arc;

use crate::template::{EnumDef, EnumSpan, Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

use super::bufr_tables;

/// The originating centres most BUFR anyone reads comes from, from WMO Common
/// Code table C-11. Edition 3 writes the centre in one byte and edition 4 in
/// two, and the numbers are the same table either way.
const CENTRE: &[(i128, &str)] = &[
    (1, "Melbourne"),
    (7, "US National Weather Service, NCEP"),
    (34, "Tokyo, JMA"),
    (54, "Montreal"),
    (58, "US Navy, FNMOC"),
    (74, "Exeter, UK Met Office"),
    (78, "Offenbach, DWD"),
    (80, "Rome"),
    (85, "Toulouse, Meteo France"),
    (88, "Oslo"),
    (89, "Prague"),
    (98, "Reading, ECMWF"),
    (160, "US NOAA/NESDIS"),
    (254, "EUMETSAT"),
];

/// Which master table the message was written against. Oceanography has a
/// master table of its own; almost everything is 0.
const MASTER_TABLE: &[(i128, &str)] = &[(0, "meteorology"), (10, "oceanography")];

/// What F says a descriptor is.
const DESCRIPTOR_KIND: &[(i128, &str)] = &[(0, "element"), (1, "replication"), (2, "operator"), (3, "sequence")];

fn u16be() -> T {
    T::u16(Big)
}

/// A three-byte length, which is how BUFR counts every section.
fn u24be() -> T {
    T::UInt { bits: 24, endian: Big }
}

fn bits(n: u32) -> T {
    T::UInt { bits: n, endian: Big }
}

pub fn bufr() -> Template {
    Template::new("bufr", T::repeat(T::Named("Chunk".into()), Until::End))
        .with_type("Chunk", chunk())
        .with_type("Message", message())
        .with_type("Descriptor", descriptor())
}

/// What is at the top of the file, over and over: a message, or the envelope
/// the GTS put around one, or bytes that are neither.
///
/// The envelope opens with a start-of-heading byte and closes with a line end
/// and an end-of-text byte, so a run that starts with either of those is read
/// as text. Anything else is whatever a tool left between the messages, and
/// reads as bytes up to where the next one starts, so a stray byte does not
/// take the rest of the file with it.
fn chunk() -> T {
    T::switch(
        look_ahead(),
        vec![
            (0x4255_4652, T::Named("Message".into())),
            (TRUNCATED, T::structure("Trailing", vec![("bytes", T::bytes(E::Remaining))])),
        ],
        T::switch(
            E::peek(8, Big),
            vec![(0x01, envelope()), (0x0D, envelope())],
            // At least one byte, since the look-ahead has already said these
            // four are not `BUFR`.
            T::structure("BetweenMessages", vec![("bytes", T::bytes(E::to_bytes(b"BUFR")))]),
        ),
    )
}

/// The GTS envelope between two messages, or in front of the first: the end of
/// one bulletin and the heading of the next.
fn envelope() -> T {
    T::structure("GtsEnvelope", vec![("text", T::text(StrLen::Fixed(E::to_bytes(b"BUFR")), Encoding::Ascii))])
}

/// The number a look-ahead answers when there are fewer than four bytes left.
/// Nothing can be it: it is not `BUFR`, and four bytes are always there to be
/// read when it is not the answer.
const TRUNCATED: i128 = -1;

fn look_ahead() -> E {
    E::Remaining.less_than(E::lit(4)).mul(E::lit(TRUNCATED)).or(E::peek(32, Big))
}

/// One message. The edition is the eighth byte in every edition, including the
/// two old ones where it is the fourth byte of section 1 rather than a byte of
/// section 0, and it decides whether there is a length to read at all.
fn message() -> T {
    T::structure_named(
        "Message",
        "",
        "body",
        vec![
            ("magic", T::magic(b"BUFR")),
            (
                "body",
                T::switch(
                    E::peek_at(E::lit(24), 8, Big),
                    vec![(4, edition(4)), (3, edition(3)), (2, edition(2))],
                    // Editions 0 and 1, which have no total length: the message
                    // runs to the next one, and the letters of that are the
                    // only thing that says where.
                    T::structure("EarlyEdition", vec![("bytes", T::bytes(E::to_bytes(b"BUFR")))]),
                ),
            ),
        ],
    )
}

/// Section 0's length and edition, and the sections inside the window the
/// length gives. The length counts the letters, so eight of it is read by the
/// time the sections start.
fn edition(n: i128) -> T {
    let window = E::field("total_length").sub(E::lit(8)).at_most(E::Remaining).at_least(E::lit(0));
    let name = match n {
        4 => "Bufr4",
        3 => "Bufr3",
        _ => "Bufr2",
    };
    T::structure(
        name,
        vec![
            ("total_length", u24be()),
            ("edition", T::u8()),
            (
                "sections",
                T::sized(
                    window,
                    T::structure(
                        "Sections",
                        vec![
                            ("identification", identification(n)),
                            ("local_use", optional_section()),
                            ("data_description", data_description()),
                            ("data", data()),
                            ("end", T::if_room(T::magic(b"7777"))),
                        ],
                    ),
                ),
            ),
        ],
    )
}

/// What is left of a section after `read` bytes of it, clamped so that a length
/// shorter than the fields it must hold does not run backwards and one longer
/// than the message does not run off the end.
fn rest(read: i128) -> E {
    E::field("length").at_least(E::lit(read)).sub(E::lit(read)).at_most(E::Remaining)
}

/// The flags byte in section 1. Its first bit, the top one, says whether
/// section 2 is there; the other seven are reserved.
fn section_flags() -> T {
    T::flags("SectionFlags", T::u8(), &[(7, "optional section 2")])
}

fn category() -> T {
    let cases = bufr_tables::categories().into_iter().map(|(v, n)| (v, n.to_string())).collect();
    enumeration("DataCategory", T::u8(), cases, Vec::new())
}

/// Section 1: who wrote the message, which tables it was written against, what
/// kind of data it holds and what moment it is about. Every edition leaves room
/// at the end for the writer's own use, which is the tail here.
fn identification(edition: i128) -> T {
    let mut fields = vec![("length", u24be()), ("master_table", T::enumeration("MasterTable", T::u8(), MASTER_TABLE))];
    let read = match edition {
        4 => {
            fields.extend(vec![
                ("centre", T::enumeration("Centre", u16be(), CENTRE)),
                ("subcentre", u16be()),
                ("update_sequence", T::u8()),
                ("flags", section_flags()),
                ("data_category", category()),
                ("international_subcategory", T::u8()),
                ("local_subcategory", T::u8()),
                ("master_table_version", T::u8()),
                ("local_table_version", T::u8()),
                ("year", u16be()),
                ("month", T::u8()),
                ("day", T::u8()),
                ("hour", T::u8()),
                ("minute", T::u8()),
                ("second", T::u8()),
            ]);
            22
        }
        3 => {
            fields.extend(vec![
                // The sub-centre first, then the centre: edition 4 turned the
                // two round when it widened them.
                ("subcentre", T::u8()),
                ("centre", T::enumeration("Centre", T::u8(), CENTRE)),
                ("update_sequence", T::u8()),
                ("flags", section_flags()),
                ("data_category", category()),
                ("data_subcategory", T::u8()),
                ("master_table_version", T::u8()),
                ("local_table_version", T::u8()),
                // Of the century, so 2026 is 26. Some writers put 126 here.
                ("year_of_century", T::u8()),
                ("month", T::u8()),
                ("day", T::u8()),
                ("hour", T::u8()),
                ("minute", T::u8()),
            ]);
            17
        }
        _ => {
            fields.extend(vec![
                ("centre", T::enumeration("Centre", u16be(), CENTRE)),
                ("update_sequence", T::u8()),
                ("flags", section_flags()),
                ("data_category", category()),
                ("data_subcategory", T::u8()),
                ("master_table_version", T::u8()),
                ("local_table_version", T::u8()),
                ("year_of_century", T::u8()),
                ("month", T::u8()),
                ("day", T::u8()),
                ("hour", T::u8()),
                ("minute", T::u8()),
            ]);
            17
        }
    };
    fields.push(("local", T::bytes(rest(read))));
    T::structure("Identification", fields).payload(&["data_category", "master_table_version"])
}

/// Section 2, which is there only when section 1's flag says so. Nothing else
/// in the message says: sections have no numbers, and a reader that guessed by
/// looking would find a three-byte length wherever it looked.
fn optional_section() -> T {
    T::switch(
        E::within(&["identification", "flags"]).bit(7),
        vec![(
            1,
            T::structure(
                "LocalUse",
                vec![("length", u24be()), ("reserved", T::u8()), ("local", T::bytes(rest(4)))],
            )
            .machinery(&["reserved"]),
        )],
        T::bytes(E::lit(0)),
    )
}

/// Section 3: how many subsets the data holds, whether it is compressed, and
/// the descriptors that say what one subset is.
///
/// A subset is one report: one station's observation, one satellite pixel. An
/// uncompressed message writes each subset's values in turn; a compressed one
/// writes each value once for all the subsets, as a reference and the
/// differences from it, which only works when every subset has the same shape.
///
/// The descriptors fill the section after its seven bytes of header, two bytes
/// each. Edition 3 required every section to be an even number of bytes long,
/// and a list of descriptors is always even, so there is a byte of padding
/// after them; edition 4 dropped the rule and some writers still pad.
fn data_description() -> T {
    let count = E::field("length").at_least(E::lit(7)).sub(E::lit(7)).div(E::lit(2)).at_most(E::Remaining.div(E::lit(2)));
    let padding = E::field("length").at_least(E::lit(7)).sub(E::lit(7)).modulo(E::lit(2)).at_most(E::Remaining);
    T::structure(
        "DataDescription",
        vec![
            ("length", u24be()),
            ("reserved", T::u8()),
            ("subsets", u16be()),
            ("flags", T::flags("DataFlags", T::u8(), &[(7, "observed data"), (6, "compressed")])),
            ("descriptors", T::array(T::Named("Descriptor".into()), count)),
            ("padding", T::bytes(padding)),
        ],
    )
    .machinery(&["reserved", "padding"])
    .payload(&["subsets", "flags", "descriptors"])
}

/// One descriptor: F, X and Y, and what the tables call it.
///
/// The name is a reading of the three numbers against the current tables and
/// covers no bits of its own. Table B names an element, Table D a sequence and
/// Table C an operator; a replication is named by what it does, since Y is
/// either how many times or, at zero, that a count in the data says. A
/// descriptor no table names, which is what a centre's local descriptors are
/// (X of 48 or more, or Y of 192 or more), reads as its number.
fn descriptor() -> T {
    let code = E::field("f").mul(E::lit(100_000)).add(E::field("x").mul(E::lit(1000))).add(E::field("y"));
    T::structure_named(
        "Descriptor",
        "name",
        "",
        vec![
            ("f", T::enumeration("DescriptorKind", bits(2), DESCRIPTOR_KIND)),
            ("x", bits(6)),
            ("y", bits(8)),
            (
                "name",
                T::switch(
                    E::field("f"),
                    vec![(0, element_name(code.clone())), (1, replication_name()), (2, operator_name(code.clone()))],
                    sequence_name(code),
                ),
            ),
        ],
    )
    .payload(&["name"])
}

/// An enumeration whose names are built when the template is, from the tables
/// rather than from a list written here.
fn enumeration(name: &str, inner: T, cases: Vec<(i128, String)>, spans: Vec<EnumSpan>) -> T {
    T::Enum { inner: Box::new(inner), def: Arc::new(EnumDef { name: name.into(), cases, docs: Vec::new(), spans, hex: false }) }
}

/// Every Table B entry, as its six-digit descriptor and its name.
fn element_name(code: E) -> T {
    let t = bufr_tables::latest();
    let mut cases: Vec<(i128, String)> =
        t.elements.iter().map(|(c, e)| (i128::from(*c), format!("{c:06} {}", e.name))).collect();
    cases.sort();
    enumeration("TableB", T::computed(code), cases, Vec::new())
}

/// Every Table D entry, as its six-digit descriptor and its title. A sequence
/// the table gives no title is named by its number alone.
fn sequence_name(code: E) -> T {
    let t = bufr_tables::latest();
    let mut cases: Vec<(i128, String)> = t
        .sequences
        .iter()
        .map(|(c, s)| (i128::from(*c), if s.title.is_empty() { format!("{c:06}") } else { format!("{c:06} {}", s.title) }))
        .collect();
    cases.sort();
    enumeration("TableD", T::computed(code), cases, Vec::new())
}

/// Table C. Most operators take Y as their operand and are named once for all
/// of them, written `201YYY`; those are named by X alone. The rest mean one
/// thing at Y of 0 and another at 255, and are named by their whole value.
fn operator_name(code: E) -> T {
    let operand = E::field("x").less_or_equal(E::lit(8)).either(E::field("x").equal_to(E::lit(21)));
    let by_x: Vec<(i128, String)> = (1..=8)
        .chain([21])
        .filter_map(|x| bufr_tables::operator_name(200_000 + x * 1000).map(|n| (i128::from(x), format!("2{x:02}YYY {n}"))))
        .collect();
    let exact: Vec<(i128, String)> = (22..=43)
        .flat_map(|x| [0, 255].map(move |y| 200_000 + x * 1000 + y))
        .filter_map(|c| bufr_tables::operator_name(c).map(|n| (i128::from(c), format!("{c:06} {n}"))))
        .collect();
    T::switch(
        operand,
        vec![(1, enumeration("TableC", T::computed(E::field("x")), by_x, Vec::new()))],
        enumeration("TableC", T::computed(code), exact, Vec::new()),
    )
}

/// A replication, named by what it repeats and how often. X is how many of the
/// descriptors after it are repeated; Y is how many times, or zero for a count
/// that the next descriptor gives and the data holds.
fn replication_name() -> T {
    let arms = (1..64)
        .map(|x: i128| {
            let things = if x == 1 { "1 descriptor".to_string() } else { format!("{x} descriptors") };
            let cases = vec![(0, format!("Delayed replication of {things}")), (1, format!("Replicate {things} once"))];
            let spans = vec![EnumSpan { from: 0, step: 1, label: format!("Replicate {things} {{n}} times") }];
            (x, enumeration("Replication", T::computed(E::field("y")), cases, spans))
        })
        .collect();
    T::switch(E::field("x"), arms, T::computed(E::field("y")))
}

/// Section 4: the values, packed end to end, with no boundaries between them.
/// See [`bufr_data`](super::bufr_data) for where they are and what they are
/// worth.
fn data() -> T {
    T::structure("Data", vec![("length", u24be()), ("reserved", T::u8()), ("bits", T::bytes(rest(4)))])
        .machinery(&["reserved"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A descriptor as its two bytes.
    fn fxy(f: u16, x: u16, y: u16) -> [u8; 2] {
        ((f << 14) | (x << 8) | y).to_be_bytes()
    }

    fn u24(n: usize) -> [u8; 3] {
        let b = (n as u32).to_be_bytes();
        [b[1], b[2], b[3]]
    }

    /// An edition 4 message from Prague, category 0, with section 2, two
    /// subsets, and the descriptors given. Section 4 holds `data`.
    fn message4(descriptors: &[[u8; 2]], data: &[u8]) -> Vec<u8> {
        let mut s1 = vec![0, 0, 0, 0]; // length, then master table 0
        s1.extend_from_slice(&89u16.to_be_bytes()); // Prague
        s1.extend_from_slice(&0u16.to_be_bytes());
        s1.extend_from_slice(&[0, 0x80, 0, 1, 2, 13, 0]); // update, flags, category, subcategories, versions
        s1.extend_from_slice(&2026u16.to_be_bytes());
        s1.extend_from_slice(&[9, 14, 12, 30, 5]);
        let n = s1.len();
        s1[..3].copy_from_slice(&u24(n));
        let s2 = vec![0, 0, 6, 0, 0xAB, 0xCD];
        let mut s3 = vec![0, 0, 0, 0];
        s3.extend_from_slice(&2u16.to_be_bytes());
        s3.push(0x80); // observed, not compressed
        for d in descriptors {
            s3.extend_from_slice(d);
        }
        let n = s3.len();
        s3[..3].copy_from_slice(&u24(n));
        let mut s4 = u24(4 + data.len()).to_vec();
        s4.push(0);
        s4.extend_from_slice(data);
        let body: Vec<u8> = [s1, s2, s3, s4, b"7777".to_vec()].concat();
        let mut m = b"BUFR".to_vec();
        m.extend_from_slice(&u24(body.len() + 8));
        m.push(4);
        m.extend_from_slice(&body);
        m
    }

    fn sample() -> Vec<u8> {
        message4(&[fxy(3, 1, 1), fxy(1, 1, 0), fxy(0, 31, 1), fxy(0, 12, 101), fxy(2, 1, 130)], &[1, 2, 3, 4])
    }

    #[test]
    fn a_message_reads_as_its_sections() {
        let bytes = sample();
        let len = bytes.len() as u128;
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(bufr());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 1);
        assert_eq!(ev.node(&d, &[0, 1, 0]).unwrap().value, Value::UInt(len));
        assert_eq!(ev.node(&d, &[0, 1, 1]).unwrap().value, Value::UInt(4));
        let sections = ev.node(&d, &[0, 1, 2]).unwrap();
        assert_eq!(sections.child_count, 5);
        // The end marker is the last four bytes of the message.
        let end = ev.node(&d, &[0, 1, 2, 4]).unwrap();
        assert_eq!((end.offset_bits + end.size_bits) / 8, len as u64);
    }

    #[test]
    fn edition_4_identification_has_a_wide_centre_and_a_four_digit_year() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(bufr());
        let centre = ev.node(&d, &[0, 1, 2, 0, 2]).unwrap();
        assert_eq!(centre.value, Value::Enum { raw: 89, name: Some("Prague".into()), hex: false });
        assert_eq!(centre.size_bits, 16);
        let category = ev.node(&d, &[0, 1, 2, 0, 6]).unwrap();
        assert_eq!(category.value, Value::Enum { raw: 0, name: Some("Surface data - land".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 9]).unwrap().value, Value::UInt(13));
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 11]).unwrap().value, Value::UInt(2026));
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 16]).unwrap().value, Value::UInt(5));
    }

    #[test]
    fn section_2_is_there_when_the_flag_says_so() {
        let bytes = sample();
        let d = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(bufr());
        let local = ev.node(&d, &[0, 1, 2, 1]).unwrap();
        assert_eq!(local.size_bits, 6 * 8);
        // The same message with the flag cleared and section 2 taken out.
        let at = bytes.windows(6).position(|w| w == [0, 0, 6, 0, 0xAB, 0xCD]).unwrap();
        let mut without = bytes[..at].to_vec();
        without.extend_from_slice(&bytes[at + 6..]);
        let flags = 8 + 9; // section 0, then the tenth byte of section 1
        without[flags] = 0;
        let total = without.len();
        without[4..7].copy_from_slice(&u24(total));
        let d = Document::new(MemSource(without));
        let mut ev = Evaluator::new(bufr());
        assert_eq!(ev.node(&d, &[0, 1, 2, 1]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[0, 1, 2, 2]).unwrap().type_name, "DataDescription");
        assert_eq!(ev.node(&d, &[0, 1, 2, 2, 2]).unwrap().value, Value::UInt(2));
    }

    #[test]
    fn descriptors_read_as_f_x_y_and_are_named_by_the_tables() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(bufr());
        let list = ev.node(&d, &[0, 1, 2, 2, 4]).unwrap();
        assert_eq!((list.child_count, list.size_bits), (5, 5 * 16));
        let f = ev.node(&d, &[0, 1, 2, 2, 4, 3, 0]).unwrap();
        assert_eq!((f.value.as_int(), f.size_bits), (Some(0), 2));
        let x = ev.node(&d, &[0, 1, 2, 2, 4, 3, 1]).unwrap();
        assert_eq!((x.value.as_int(), x.size_bits), (Some(12), 6));
        assert_eq!(ev.node(&d, &[0, 1, 2, 2, 4, 3, 2]).unwrap().value.as_int(), Some(101));
        let names: Vec<String> = (0..5).map(|i| ev.node(&d, &[0, 1, 2, 2, 4, i]).unwrap().name).collect();
        assert_eq!(
            names,
            vec![
                "[0] 301001 WMO block and station numbers",
                "[1] Delayed replication of 1 descriptor",
                "[2] 031001 Delayed descriptor replication factor",
                "[3] 012101 Temperature/air temperature",
                "[4] 201YYY Change data width",
            ]
        );
    }

    #[test]
    fn a_fixed_replication_says_how_many_times() {
        let d = Document::new(MemSource(message4(&[fxy(1, 2, 5), fxy(0, 12, 101), fxy(2, 22, 0), fxy(2, 37, 255)], &[])));
        let mut ev = Evaluator::new(bufr());
        let names: Vec<String> = (0..4).map(|i| ev.node(&d, &[0, 1, 2, 2, 4, i]).unwrap().name).collect();
        assert_eq!(names[0], "[0] Replicate 2 descriptors 5 times");
        assert_eq!(names[2], "[2] 222000 Quality information follows");
        assert_eq!(names[3], "[3] 237255 Cancel use defined data present bit-map");
    }

    #[test]
    fn section_4_keeps_its_length_and_its_bits() {
        let d = Document::new(MemSource(sample()));
        let mut ev = Evaluator::new(bufr());
        let data = ev.node(&d, &[0, 1, 2, 3]).unwrap();
        assert_eq!(data.type_name, "Data");
        assert_eq!(ev.node(&d, &[0, 1, 2, 3, 0]).unwrap().value, Value::UInt(8));
        assert_eq!(ev.node(&d, &[0, 1, 2, 3, 2]).unwrap().size_bits, 4 * 8);
    }

    #[test]
    fn an_edition_3_message_puts_the_subcentre_before_a_one_byte_centre() {
        let mut s1 = vec![0, 0, 18, 0, 5, 98, 0, 0, 2, 0, 13, 1, 26, 9, 14, 12, 0, 0];
        s1[2] = 18;
        let mut s3 = vec![0, 0, 10, 0, 0, 1, 0xC0];
        s3.extend_from_slice(&fxy(0, 12, 101));
        s3.push(0); // edition 3's padding to an even length
        let s4 = vec![0, 0, 6, 0, 0, 0];
        let body: Vec<u8> = [s1, s3, s4, b"7777".to_vec()].concat();
        let mut m = b"BUFR".to_vec();
        m.extend_from_slice(&u24(body.len() + 8));
        m.push(3);
        m.extend_from_slice(&body);
        let d = Document::new(MemSource(m));
        let mut ev = Evaluator::new(bufr());
        assert_eq!(ev.node(&d, &[0, 1]).unwrap().type_name, "Bufr3");
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 2]).unwrap().value, Value::UInt(5));
        let centre = ev.node(&d, &[0, 1, 2, 0, 3]).unwrap();
        assert_eq!(centre.value, Value::Enum { raw: 98, name: Some("Reading, ECMWF".into()), hex: false });
        assert_eq!(ev.node(&d, &[0, 1, 2, 0, 10]).unwrap().value, Value::UInt(26));
        // No section 2, one descriptor, a byte of padding, and compressed.
        assert_eq!(ev.node(&d, &[0, 1, 2, 1]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[0, 1, 2, 2, 4]).unwrap().child_count, 1);
        assert_eq!(ev.node(&d, &[0, 1, 2, 2, 5]).unwrap().size_bits, 8);
        let Value::Flags { set, .. } = ev.node(&d, &[0, 1, 2, 2, 3]).unwrap().value else { panic!("flags") };
        assert_eq!(set, vec!["compressed", "observed data"]);
    }

    #[test]
    fn a_gts_envelope_reads_as_text_between_messages() {
        let one = sample();
        let mut b = b"\x01\r\r\n052\r\r\nISMD01 OKPR 211200\r\r\n".to_vec();
        b.extend_from_slice(&one);
        b.extend_from_slice(b"\r\r\n\x03\x01\r\r\n053\r\r\nISMD01 OKPR 211800\r\r\n");
        b.extend_from_slice(&one);
        b.extend_from_slice(b"\r\r\n\x03");
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(bufr());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 5);
        let kinds: Vec<String> = (0..5).map(|i| ev.node(&d, &[i]).unwrap().type_name).collect();
        assert_eq!(kinds, vec!["GtsEnvelope", "Message", "GtsEnvelope", "Message", "GtsEnvelope"]);
        let Value::Str(text) = ev.node(&d, &[0, 0]).unwrap().value else { panic!("text") };
        assert!(text.contains("ISMD01 OKPR 211200"), "{text:?}");
    }

    #[test]
    fn stray_bytes_between_messages_do_not_take_the_rest_of_the_file() {
        let one = sample();
        let mut b = one.clone();
        b.extend_from_slice(&[0xFF, 0xFE]);
        b.extend_from_slice(&one);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(bufr());
        assert_eq!(ev.node(&d, &[]).unwrap().child_count, 3);
        let between = ev.node(&d, &[1]).unwrap();
        assert_eq!((between.type_name.as_str(), between.size_bits), ("BetweenMessages", 16));
        assert_eq!(ev.node(&d, &[2]).unwrap().type_name, "Message");
    }

    #[test]
    fn a_message_cut_off_reads_as_far_as_it_goes() {
        let mut bytes = sample();
        bytes.truncate(bytes.len() - 10);
        let d = Document::new(MemSource(bytes.clone()));
        let mut ev = Evaluator::new(bufr());
        let sections = ev.node(&d, &[0, 1, 2]).unwrap();
        assert_eq!((sections.offset_bits + sections.size_bits) / 8, bytes.len() as u64);
    }
}
