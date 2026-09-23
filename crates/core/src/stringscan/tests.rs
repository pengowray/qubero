//! The scanner asked of bytes that are known.

use super::*;
use super::prefix::*;
use crate::source::MemSource;

fn all(bytes: Vec<u8>) -> Vec<Hit> {
    scan(&MemSource(bytes), 0, 1000, Opts::default()).hits
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn utf16be(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

#[test]
fn finds_a_c_string_and_its_terminator() {
    let mut b = vec![0x01, 0x02, 0x03];
    b.extend(b"version 1.4\0");
    b.extend([0xff, 0xff]);
    let hits = all(b);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].text, "version 1.4");
    assert_eq!(hits[0].at, 3);
    assert_eq!(hits[0].enc, Enc::Ascii);
    assert_eq!(hits[0].term, Some(Term::Nul));
}

#[test]
fn a_run_shorter_than_the_minimum_is_not_a_string() {
    let hits = all(b"\x00abc\x00abcd\x00".to_vec());
    assert_eq!(hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["abcd"]);
}

#[test]
fn a_newline_ends_a_string() {
    let hits = all(b"\x00first line\nsecond line\x00".to_vec());
    assert_eq!(hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["first line", "second line"]);
}

/// A .NET user string heap: a length in bytes, the characters, then a flag
/// byte. Nothing terminates a string, so the flag byte and the next
/// string's length are what the shifted big-endian reading takes for a
/// terminator, and it is the reading with all the evidence.
fn us_heap(words: &[&str]) -> Vec<u8> {
    let mut b = vec![0x00, 0x00];
    for w in words {
        let chars = utf16le(w);
        b.push(chars.len() as u8 + 1);
        b.extend(chars);
        b.push(0x00);
    }
    b.extend([0x00, 0x00]);
    b
}

/// A Windows STRINGTABLE: sixteen strings to a block, each a count of code
/// units then the characters, with nothing terminating any of them. The
/// only thing that can speak for these is that the whole block is counted
/// the same way, which is what `in_a_table` is for.
fn string_table(words: &[&str]) -> Vec<u8> {
    let mut b = vec![0x00, 0x00];
    for w in words {
        b.extend((w.encode_utf16().count() as u16).to_le_bytes());
        b.extend(utf16le(w));
    }
    // A count with nothing behind it, the way a block runs out: two zero
    // bytes here would hand the last string a terminator, which is the one
    // thing a string table never gives one.
    b.extend([0x05, 0x00]);
    b
}

#[test]
fn a_block_of_counted_strings_speaks_for_every_one_of_them() {
    let words = ["Disk is not formatted", "Open file locat&ion", "(Debug)", "Windows can't format %s", "File"];
    let hits = all(string_table(&words));
    assert_eq!(hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), words);
    for hit in &hits {
        assert_eq!(hit.enc, Enc::Utf16Le, "{:?}", hit.text);
        assert_eq!(hit.term, None, "{:?}", hit.text);
        assert_eq!(hit.prefix[0].kind, PrefixKind::U16Le, "{:?}", hit.text);
        assert_eq!(hit.prefix[0].counts, Counts::Units, "{:?}", hit.text);
        assert!(!hit.prefix[0].weak, "{:?} is counted the way the block is", hit.text);
    }
}

#[test]
fn two_counted_strings_are_not_a_block() {
    // The same shape, too few to be anything but a coincidence. A number
    // one code unit wide in front of a run with no terminator is the run's
    // own boundary read twice until enough of the file agrees. Short
    // strings, since a long one speaks for itself. See `LONG_ENOUGH`.
    let hits = all(string_table(&["Disk full", "(Debug)"]));
    assert!(hits.is_empty(), "{:?}", hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>());
}

#[test]
fn packed_utf16_is_read_the_way_round_it_was_written() {
    let hits = all(us_heap(&["providerOptions", "Module", "Nothing", "vbc.exe"]));
    assert_eq!(hits.iter().map(|h| h.enc).collect::<Vec<_>>(), [Enc::Utf16Le; 4]);
    assert_eq!(
        hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(),
        ["providerOptions", "Module", "Nothing", "vbc.exe"]
    );
}

#[test]
fn the_shifted_reading_of_big_endian_text_does_not_take_its_place() {
    // The same heap the other way round, where the shifted reading is the
    // little-endian one. Nothing about the bytes says which way round they
    // were meant except the terminators, so this is what asks whether the
    // rule that settles it has a side.
    let mut b = vec![0x00, 0x00];
    for w in ["providerOptions", "Module", "Nothing"] {
        b.extend(utf16be(w));
        b.extend([0x00, 0x00]);
    }
    let hits = all(b);
    assert_eq!(hits.iter().map(|h| h.enc).collect::<Vec<_>>(), [Enc::Utf16Be; 3]);
    assert_eq!(hits[0].text, "providerOptions");
}

#[test]
fn strings_run_together_by_a_separator_are_still_strings() {
    // A run is allowed a stray character, and the byte ending one line
    // read together with the byte counting the next is a stray character.
    // A heap of them is one run of every line at once, and the last line's
    // terminator speaks for the whole of it.
    let hits = all(us_heap(&["Do While ", "NotInheritable ", "Protected Friend ", "Structure "]));
    assert_eq!(
        hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(),
        ["Do While ", "NotInheritable ", "Protected Friend ", "Structure "]
    );
}

#[test]
fn a_string_between_two_stretches_of_rubbish_is_found() {
    // Read two bytes at a time, eight-bit text either side of a wide
    // string is a run of characters from all over Unicode, and it reaches
    // over the string. Trimming the ends of that run cannot reach the
    // string, since the run's own page sits in the rubbish as well.
    let mut b = vec![0x00, 0x00];
    b.extend(b"cmdidTileHorz\0csz\0psz\0");
    b.push(0x1f);
    b.extend(utf16le("providerOptions"));
    b.extend([0x00, 0x05, 0x76, 0x00, 0x62, 0x00, 0x00, 0x00]);
    let hits = all(b);
    assert!(
        hits.iter().any(|h| h.enc == Enc::Utf16Le && h.text == "providerOptions"),
        "{:?}",
        hits.iter().map(|h| (h.enc, h.text.as_str())).collect::<Vec<_>>()
    );
}

#[test]
fn a_length_eight_bytes_wide_says_so() {
    // What GGUF counts every key and every string value by.
    let mut b = vec![0x00, 0x00];
    b.extend(20u64.to_le_bytes());
    b.extend(b"general.architecture");
    b.extend([0x00, 0x00]);
    let readings = &all(b)[0].prefix;
    assert_eq!(readings[0].kind, PrefixKind::U64Le);
    assert_eq!(readings[0].value, 20);
    assert_eq!(readings[0].counts, Counts::Bytes);
}

#[test]
fn a_pascal_string_says_which_byte_counted_it() {
    let mut b = vec![0xff, 0x0b];
    b.extend(b"version 1.4");
    b.push(0xff);
    let hits = all(b);
    assert_eq!(hits.len(), 1);
    let p = &hits[0].prefix;
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].kind, PrefixKind::U8);
    assert_eq!(p[0].value, 11);
    assert_eq!(p[0].counts, Counts::Bytes);
    assert_eq!(p[0].at, 1);
    assert!(!p[0].with_terminator);
}

#[test]
fn a_length_that_counts_the_terminator_says_so() {
    let mut b = vec![0xff, 0x0c];
    b.extend(b"version 1.4\0");
    let hits = all(b);
    let p = &hits[0].prefix;
    assert_eq!(p[0].kind, PrefixKind::U8);
    assert_eq!(p[0].value, 12);
    assert!(p[0].with_terminator);
}

#[test]
fn a_thirty_two_bit_length_is_read_before_the_bytes_inside_it() {
    let mut b = vec![0xff, 0x00, 0x00, 0x00, 0x0b];
    b.extend(b"version 1.4");
    b.push(0xff);
    let readings = &all(b)[0].prefix;
    assert_eq!(readings[0].kind, PrefixKind::U32Be);
    assert_eq!(readings[0].value, 11);
    // The same bytes are a 16-bit and an 8-bit eleven as well, and all
    // three readings are true of them.
    assert!(readings.iter().any(|p| p.kind == PrefixKind::U16Be));
    assert!(readings.iter().any(|p| p.kind == PrefixKind::U8));
}

#[test]
fn a_little_endian_length_reads_only_one_way() {
    let mut b = vec![0xff, 0x0b, 0x00, 0x00, 0x00];
    b.extend(b"version 1.4");
    b.push(0xff);
    let readings = &all(b)[0].prefix;
    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].kind, PrefixKind::U32Le);
}

#[test]
fn a_lone_byte_length_is_shown_for_what_it_is() {
    // The byte in front of any run is one that could not be part of it,
    // and the values that could not be are the small ones, which is what a
    // short length looks like. So a match here is worth showing and is not
    // worth believing on its own.
    let mut b = vec![0xff, 0x0b];
    b.extend(b"version 1.4");
    b.push(0xff);
    let hits = all(b);
    assert!(hits[0].prefix[0].weak);
}

#[test]
fn a_table_of_counted_strings_vouches_for_every_number_in_it() {
    // The same byte-wide number, once a neighbour is counted the same way.
    // Two of them landing exactly where the string after them starts is a
    // table, and a table is a fact about the file.
    let mut b = vec![0xff];
    for word in ["version 1.4", "release", "beta 3"] {
        b.push(word.len() as u8);
        b.extend(word.as_bytes());
    }
    b.push(0xff);
    let hits = all(b);
    assert_eq!(hits.len(), 3);
    for h in &hits {
        assert_eq!(h.prefix[0].kind, PrefixKind::U8);
        assert!(!h.prefix[0].weak, "{:?} should be vouched for by its neighbours", h.text);
    }
}

#[test]
fn a_number_wider_than_a_character_is_not_the_run_boundary() {
    let mut b = vec![0xff, 0x0b, 0x00];
    b.extend(b"version 1.4");
    b.push(0xff);
    let readings = &all(b)[0].prefix;
    let wide = readings.iter().find(|p| p.kind == PrefixKind::U16Le).expect("a u16 reading");
    assert!(!wide.weak);
}

#[test]
fn a_dotnet_seven_bit_length_is_found() {
    // 200 characters, written as .NET's BinaryWriter writes a length.
    let text = "d".repeat(200);
    let mut b = vec![0x00, 0xc8, 0x01];
    b.extend(text.as_bytes());
    b.push(0x00);
    let readings = &all(b)[0].prefix;
    assert!(readings.iter().any(|p| p.kind == PrefixKind::Leb128 && p.value == 200 && p.raw == [0xc8, 0x01]));
}

#[test]
fn utf16_le_is_found_at_an_odd_offset() {
    let mut b = vec![0x01, 0x02, 0x03];
    b.extend(utf16le("Open file"));
    b.extend([0x00, 0x00]);
    let hits = all(b);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].text, "Open file");
    assert_eq!(hits[0].enc, Enc::Utf16Le);
    assert_eq!(hits[0].at, 3);
    assert_eq!(hits[0].term, Some(Term::NulNul));
}

#[test]
fn utf16_be_is_found() {
    let mut b = vec![0x00, 0x00];
    b.extend(utf16be("Cannot open"));
    b.extend([0x00, 0x00]);
    let hits = all(b);
    assert_eq!(hits.iter().map(|h| h.enc).collect::<Vec<_>>(), [Enc::Utf16Be]);
    assert_eq!(hits[0].text, "Cannot open");
}

#[test]
fn a_utf16_length_counting_code_units_says_so() {
    let mut b = vec![0x00, 0x00, 0x0d, 0x00];
    b.extend(utf16le("Cannot open a"));
    b.extend([0x00, 0x00]);
    let readings = &all(b)[0].prefix;
    assert!(readings.iter().any(|p| p.kind == PrefixKind::U16Le && p.counts == Counts::Units && p.value == 13));
}

#[test]
fn an_ascii_run_is_not_reported_a_second_time_as_wide_characters() {
    let hits = all(b"\x00the quick brown fox jumps\x00".to_vec());
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].enc, Enc::Ascii);
}

#[test]
fn a_lone_surrogate_does_not_end_a_wtf16_string() {
    let mut units: Vec<u16> = "photo".encode_utf16().collect();
    units.push(0xd83d);
    units.extend("done".encode_utf16());
    let mut b = vec![0x00, 0x00];
    b.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    b.extend([0x00, 0x00]);
    let hits = all(b);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].lone_surrogates);
    assert_eq!(hits[0].text, "photo\u{fffd}done");
    assert_eq!(hits[0].chars, 10);
}

#[test]
fn a_surrogate_pair_is_one_character_and_two_code_units() {
    let mut b = vec![0x00, 0x00];
    b.extend(utf16le("hi \u{1f600} there"));
    b.extend([0x00, 0x00]);
    let hits = all(b);
    assert!(!hits[0].lone_surrogates);
    assert_eq!(hits[0].text, "hi \u{1f600} there");
    assert_eq!(hits[0].chars, 10);
    assert_eq!(hits[0].units, 11);
}

#[test]
fn utf8_is_told_from_ascii() {
    let mut b = vec![0x00];
    b.extend("Gr\u{fc}\u{df}e aus K\u{f6}ln".as_bytes());
    b.push(0x00);
    let hits = all(b);
    assert_eq!(hits[0].enc, Enc::Utf8);
    assert_eq!(hits[0].text, "Gr\u{fc}\u{df}e aus K\u{f6}ln");
    assert_eq!(hits[0].chars, 14);
    assert_eq!(hits[0].len, 17);
}

#[test]
fn a_chain_of_counted_strings_is_split_into_the_strings_it_is() {
    // Two strings whose length bytes are themselves printable, so the
    // whole thing is one run of text and reading it as one string would
    // be wrong.
    let a = "a".repeat(40);
    let c = "c".repeat(50);
    let mut b = vec![0x00, 40u8];
    b.extend(a.as_bytes());
    b.push(50u8);
    b.extend(c.as_bytes());
    b.push(0x00);
    let hits = all(b);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].text, a);
    assert_eq!(hits[1].text, c);
    assert_eq!(hits[0].prefix[0].kind, PrefixKind::U8);
    assert_eq!(hits[0].at, 2);
    assert_eq!(hits[0].prefix[0].at, 1);
    assert_eq!(hits[1].at, 43);
    assert_eq!(hits[1].prefix[0].at, 42);
    assert_eq!(hits[1].prefix[0].value, 50);
}

#[test]
fn a_run_that_only_nearly_tiles_is_left_as_one_string() {
    let a = "a".repeat(40);
    let c = "c".repeat(50);
    let mut b = vec![0x00, 40u8];
    b.extend(a.as_bytes());
    b.push(49u8); // one short: the chain does not reach the end
    b.extend(c.as_bytes());
    b.push(0x00);
    let hits = all(b);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].len, 92);
}

#[test]
fn a_run_past_the_cap_is_cut_and_says_so() {
    let mut b = vec![0x00];
    b.extend(vec![b'z'; (MAX_BYTES as usize) + 100]);
    b.push(0x00);
    let hits = all(b);
    assert_eq!(hits.len(), 2);
    assert!(hits[0].cut);
    assert_eq!(hits[0].len, MAX_BYTES);
    assert_eq!(hits[1].at, 1 + MAX_BYTES);
    assert_eq!(hits[1].len, 100);
    assert!(!hits[1].cut);
}

#[test]
fn a_run_longer_than_one_window_says_so_at_every_join() {
    // Longer than the window and than the lookahead past it, so the read
    // stops in the middle of it as well as the length limit does.
    let mut b = vec![0u8; (WINDOW - 10) as usize];
    b.extend(vec![b'z'; 20_000]);
    b.push(0);
    let hits = all(b);
    assert!(hits.len() > 4);
    for pair in hits.windows(2) {
        let (a, next) = (&pair[0], &pair[1]);
        assert!(a.cut, "a piece before another one must say the text carries on");
        assert_eq!(a.at + a.len, next.at, "the pieces have to be one run with no gap");
    }
    let last = hits.last().expect("a run this long is at least one string");
    assert!(!last.cut);
    assert_eq!(last.term, Some(Term::Nul));
    let total: u64 = hits.iter().map(|h| h.len).sum();
    assert_eq!(total, 20_000);
}

#[test]
fn a_string_across_a_window_join_is_found_once_and_whole() {
    let filler = (WINDOW - 20) as usize;
    let mut b = vec![0u8; filler];
    b.extend(b"across the join here");
    b.extend(vec![0u8; 100]);
    let hits = all(b);
    let found: Vec<&str> = hits.iter().map(|h| h.text.as_str()).collect();
    assert_eq!(found, ["across the join here"]);
    assert_eq!(hits[0].at, filler as u64);
}

#[test]
fn scanning_carries_on_from_where_it_stopped() {
    let mut b = vec![0u8; 4];
    b.extend(b"first\0");
    b.extend(b"second\0");
    b.extend(b"third\0");
    let src = MemSource(b);
    let one = scan(&src, 0, 2, Opts::default());
    assert_eq!(one.hits.len(), 2);
    let two = scan(&src, one.next, 10, Opts::default());
    assert_eq!(two.hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["third"]);
    assert_eq!(two.next, src.len_bytes());
}

#[test]
fn turning_an_encoding_off_leaves_the_others() {
    let mut b = vec![0x00, 0x00];
    b.extend(utf16le("Open file"));
    b.extend([0x00, 0x00]);
    b.extend(b"plain ascii\0");
    let src = MemSource(b);
    let both = scan(&src, 0, 100, Opts::default()).hits;
    assert_eq!(both.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["Open file", "plain ascii"]);
    let opts = Opts { ascii: false, ..Opts::default() };
    let hits = scan(&src, 0, 100, opts).hits;
    assert_eq!(hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["Open file"]);
}

#[test]
fn one_wide_character_repeated_is_padding_and_not_a_string() {
    // x86 pads with 0x90, which two bytes at a time is one character over
    // and over. Coherent, printable, and not a string.
    let mut b = vec![0x00, 0x00];
    b.extend(vec![0x90u8; 40]);
    b.extend([0x00, 0x00]);
    assert!(all(b).is_empty());
}

#[test]
fn a_wide_string_keeps_itself_and_not_what_sits_in_front_of_it() {
    // The bytes around a string in a binary are whatever the compiler put
    // there, and two of them are usually some character. The run reaches
    // over them and has to be cut back, or the characters they add make an
    // otherwise coherent run look like six scripts at once.
    let mut b = vec![0xa0, 0xee, 0xe8, 0xb9];
    b.extend(utf16le("Open file"));
    b.extend([0x00, 0x00, 0x5c, 0x7c]);
    let hits = all(b);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].text, "Open file");
    assert_eq!(hits[0].at, 4);
    assert_eq!(hits[0].term, Some(Term::NulNul));
}

#[test]
fn a_wide_string_keeps_itself_and_not_what_sits_after_it() {
    // The same at the other end, where the number in front is what says
    // the run is a string at all.
    // Twenty-six bytes, counted by a number four bytes wide: two bytes
    // more than the run's own boundary needed, which is what makes it
    // worth believing. See `evidence`.
    let mut b = vec![0xff, 0x1a, 0x00, 0x00, 0x00];
    b.extend(utf16le("Cannot open a"));
    b.extend([0x5c, 0x7c, 0x29, 0x99, 0x00]);
    let hits = all(b);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].text, "Cannot open a");
    assert_eq!(hits[0].prefix[0].kind, PrefixKind::U32Le);
    assert_eq!(hits[0].prefix[0].value, 26);
}

#[test]
fn a_wide_run_with_nothing_to_say_for_itself_is_not_reported() {
    // Sixteen-bit numbers that happen to be printable characters. Quiet
    // audio and a table of small integers both look like this, and neither
    // is a string: no zero after it, no number in front that comes to its
    // length.
    let mut b = vec![0xa0, 0xee, 0xe8, 0xb9];
    b.extend(utf16le("Open file"));
    b.extend([0x5c, 0x7c, 0x29, 0x99]);
    assert!(all(b).is_empty());
}

#[test]
fn a_wide_character_does_not_carry_compiled_code_over_the_minimum() {
    // c6 8b is a valid UTF-8 character and welds ")" onto "D$P". Neither
    // half is four characters long and neither is a string.
    let b = b"\x00)\xc6\x8bD$P\x00".to_vec();
    assert!(all(b).is_empty());
}

#[test]
fn a_run_of_wide_characters_from_six_scripts_is_not_a_string() {
    // Every one of these is a printable character and no two are from the
    // same part of Unicode, which is what a stretch of compiled code read
    // two bytes at a time looks like.
    let units: [u16; 6] = [0x0045, 0x6400, 0x0a86, 0xa000, 0xfb2c, 0x0069];
    let mut b = vec![0x00, 0x00];
    b.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    b.extend([0x00, 0x00]);
    assert!(all(b).is_empty());
}

#[test]
fn an_unreadable_chunk_is_asked_for_rather_than_guessed_at() {
    use crate::source::Missing;
    struct Absent;
    impl Source for Absent {
        fn len_bytes(&self) -> u64 {
            1 << 20
        }
        fn read_bytes(&self, _offset: u64, out: &mut [u8]) -> Vec<Missing> {
            out.fill(0);
            vec![Missing { chunk: 0 }]
        }
    }
    let s = scan(&Absent, 0, 10, Opts::default());
    assert!(s.hits.is_empty());
    assert_eq!(s.next, 0);
    assert_eq!(s.missing.len(), 1);
}

#[test]
fn big_endian_text_straight_after_eight_bit_text_leaves_it_whole() {
    // A TrueType `name` table: the Mac Roman names, then the Windows names in
    // UTF-16 BE, with nothing between. Read little-endian from a byte early,
    // the wide text takes the last letter of the Mac Roman names for its
    // first; read little-endian from a byte late, it takes the zero in front
    // for the Mac Roman names' terminator.
    let mut b = vec![0x00, 0x2a, 0x00, 0xed];
    b.extend(b"Qubero FixtureRegular");
    b.extend(utf16be("Qubero FixtureRegular"));
    b.extend([0x00, 0x00, 0x00, 0x02]);
    let hits = all(b);
    assert_eq!(hits.iter().map(|h| (h.at, h.enc)).collect::<Vec<_>>(), [(4, Enc::Ascii), (25, Enc::Utf16Be)]);
    assert_eq!(hits[0].text, "Qubero FixtureRegular");
    assert_eq!(hits[1].text, "Qubero FixtureRegular");
    // The zero after the Mac Roman names is the first byte of the next one.
    assert_eq!(hits[0].term, None);
}

#[test]
fn little_endian_text_after_a_c_string_keeps_to_its_own_bytes() {
    // The same bytes as a TrueType `name` table, but in a list of C strings,
    // where the zero is the terminator of the one in front.
    let mut b = vec![0x00];
    b.extend(b"BCryptGetProperty\0");
    b.extend(utf16le("HashDigestLength"));
    b.extend([0x00, 0x00]);
    b.extend(b"BCryptCreateHash\0");
    let hits = all(b);
    assert_eq!(
        hits.iter().map(|h| (h.at, h.enc, h.text.as_str())).collect::<Vec<_>>(),
        [(1, Enc::Ascii, "BCryptGetProperty"), (19, Enc::Utf16Le, "HashDigestLength"), (53, Enc::Ascii, "BCryptCreateHash")]
    );
    assert_eq!(hits[0].term, Some(Term::Nul));
}

#[test]
fn little_endian_text_after_a_few_printable_bytes_keeps_its_first_letter() {
    // Three printable bytes are not a string, so the letter they would need
    // to make one stays with the wide text it starts.
    let mut b = vec![0x00, b'x', b'y', b'z'];
    b.extend(utf16le("Hello there"));
    b.extend([0x00, 0x00]);
    let hits = all(b);
    assert_eq!(hits.iter().map(|h| (h.at, h.enc)).collect::<Vec<_>>(), [(4, Enc::Utf16Le)]);
    assert_eq!(hits[0].text, "Hello there");
}

#[test]
fn a_little_endian_string_after_a_terminated_one_is_read_little_endian() {
    // From a minidump. The second string has no terminator and runs into a
    // byte that reads as one more character big-endian, so the big-endian
    // reading starting on the terminator's second zero is the longer one.
    let mut b = vec![0x00, 0x00];
    b.extend(utf16le("WinSta0\\Default"));
    b.extend([0x00, 0x00]);
    b.extend(utf16le("C:\\src\\crashpad\\"));
    b.extend([0x30, 0x75, 0xa6, 0x02, 0x00, 0x00]);
    let hits = all(b);
    assert_eq!(
        hits.iter().map(|h| (h.at, h.enc, h.text.as_str())).collect::<Vec<_>>(),
        [(2, Enc::Utf16Le, "WinSta0\\Default"), (34, Enc::Utf16Le, "C:\\src\\crashpad\\")]
    );
    assert_eq!(hits[0].term, Some(Term::NulNul));
}

#[test]
fn eight_bytes_of_ff_in_front_of_a_string_are_not_a_length() {
    // All ones is -1, and read as a 64-bit length it is most of the way to the
    // top of a usize, where adding the string's offset to it wraps. A CDF
    // file fills its unused offsets this way.
    let mut b = vec![0xff; 8];
    b.extend(b"TITLE\0");
    let hits = all(b);
    assert_eq!(hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["TITLE"]);
    assert!(hits[0].prefix.is_empty());
}
