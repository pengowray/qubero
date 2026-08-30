//! Bencode: four types, and every number in one written as digits.
//!
//! An integer is `i`, digits, `e`. A byte string is its length in digits, a
//! colon, and that many bytes. A list is `l`, values, `e`. A dictionary is
//! `d`, alternating keys and values, `e`, and the keys are byte strings in
//! sorted order. That is the whole format, and a torrent is one dictionary of
//! it covering the file.
//!
//! Three letters open a value and one closes a container, so what a value is
//! comes from peeking at its own first byte: the marker field takes that byte
//! when it is a letter and no bytes at all when it is a digit, which is what a
//! byte string starts with. Read as a number, a field of no bytes is zero, so
//! the switch under it has one case per letter and reads a byte string in the
//! default.
//!
//! The `e` that ends a container is a value of its own, the way a CBOR break
//! is, which is what lets a list be a run of values that stops at the element
//! whose marker is that letter. A dictionary's entries pair two values up, and
//! the `e` lands in the key: only the end marker is one byte long, since the
//! shortest byte string is `0:`.
//!
//! Nothing marks the front of a bencoded file, so recognising one is parsing
//! it. See [`is_bencode`].

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// The letters that open a value, and the one that closes a container. A byte
/// string opens with a digit of its own length and has no letter to take.
const MARKER: &[(i128, &str)] = &[
    (b'd' as i128, "dictionary"),
    (b'e' as i128, "end"),
    (b'i' as i128, "integer"),
    (b'l' as i128, "list"),
];

pub fn bencode() -> Template {
    Template::new("bencode", T::Named("Value".into())).with_type("Value", value())
}

fn value() -> T {
    T::structure_named("Value", "marker", "body", vec![("marker", marker()), ("body", body())])
}

/// The letter this value opens with, where there is one. A byte string has
/// none, and the field is then no bytes at all: read as a number that is zero,
/// which is the case [`body`] has no letter for.
fn marker() -> T {
    let letter = T::enumeration("Bencode", T::u8(), MARKER);
    let cases = MARKER.iter().map(|(v, _)| (*v, letter.clone())).collect();
    T::switch(E::peek(8, Big), cases, T::bytes(E::lit(0)))
}

fn body() -> T {
    T::switch(
        E::field("marker"),
        vec![
            // The digits of an integer, up to the `e` that ends it. Negative
            // numbers are written with a minus, which the parse reads.
            (b'i' as i128, T::decimal(StrLen::Terminated { end: b'e', or_end: false })),
            (b'l' as i128, T::repeat(T::Named("Value".into()), ends("marker"))),
            (b'd' as i128, T::repeat(entry(), ends("key"))),
            // The end of a container says everything it has to say in its
            // letter.
            (b'e' as i128, T::bytes(E::lit(0))),
        ],
        byte_string(),
    )
}

/// A run of values ends at an `e`, which is a value in its own right: the
/// element that closes the container is that letter and nothing else, so
/// `field` is whichever of the element's own fields the letter lands in.
fn ends(field: &str) -> Until {
    Until::FieldBytes { field: field.into(), bytes: vec![b'e'] }
}

/// One key and the value under it. The `e` that ends the dictionary lands in
/// `key` as a value of one byte, and there is no value beside it to read: only
/// the end marker is a value one byte long, since the shortest byte string is
/// `0:`.
fn entry() -> T {
    T::structure_named(
        "Entry",
        "key",
        "value",
        vec![
            ("key", T::Named("Value".into())),
            ("value", T::switch(E::size_of("key"), vec![(1, T::bytes(E::lit(0)))], T::Named("Value".into()))),
        ],
    )
    .counted_as("entry")
}

/// A length in digits, a colon, and that many bytes.
///
/// What those bytes are, the format does not say: a torrent's `announce` is a
/// URL and its `pieces` is a run of SHA-1 hashes, and both are written this
/// way. `Encoding::Unknown` reads them as UTF-8 when they are UTF-8 and as
/// Latin-1 when they are not, and says which it did.
fn byte_string() -> T {
    T::structure_named(
        "ByteString",
        "",
        "text",
        vec![
            ("length", T::decimal(StrLen::Terminated { end: b':', or_end: false })),
            ("text", T::text(StrLen::Fixed(E::field("length")), Encoding::Unknown)),
        ],
    )
}

/// How far a scan got, and whether it got there.
enum Parsed {
    /// The value ends at this offset.
    Ends(usize),
    /// The bytes given ran out before the value did. Whether that means the
    /// window is short or the file is, only the caller knows.
    Cut,
    /// Not a bencoded value.
    No,
}

/// How deep a file may nest and still be claimed.
///
/// This is not a limit on bencode, which has none. It is where the scan below
/// gives out: it calls itself once a level, and a debug build runs a megabyte
/// of stack out at about 2500 of them. A quarter of that leaves room to spare
/// and still covers the deepest bencode anyone has published, which is
/// libtorrent's 908-level `v2_deep_recursion.torrent`. Real torrents nest five
/// deep, or a dozen when a v2 file tree carries a long path.
///
/// Whether such a file can be *read* is a separate question and no longer this
/// one's business. The evaluator refuses a node past `DEEPEST_PATH`, which is
/// about twenty levels of bencode, and says so in an error. A file between
/// that and this is recognised as bencode and reads as an error saying it
/// nests too deep, which is more use than not recognising it at all.
const MAX_DEPTH: u32 = 1024;

/// What may follow the dictionary and the file still be one bencoded
/// dictionary: nothing, or the newline a tool that wrote the file through a
/// text mode put there. Five of libtorrent's own test torrents end that way.
fn only_space(b: &[u8]) -> bool {
    b.iter().all(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n'))
}

/// Whether the file is one bencoded dictionary and nothing else.
///
/// There is no signature to match. A torrent opens `d8:announce`, or
/// `d7:comment`, or whichever key sorts first in the one it happens to be, so
/// the cheap test is the shape of that first key: a `d`, digits, and a colon.
/// What settles it is parsing the dictionary, because a file that opens that
/// way and is not bencode stops making sense within a few bytes.
pub(super) fn is_bencode(head: &[u8], len: u64) -> bool {
    let Some(rest) = head.strip_prefix(b"d") else { return false };
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || rest.get(digits) != Some(&b':') {
        return false;
    }
    match scan(head, 0, 0) {
        // The dictionary covers the file, give or take the newline a tool
        // that saved it through a text mode left on the end. Anything else
        // after it is a file with something else on the end, and that is not
        // this. The trailing byte is still not part of the dictionary: the
        // template reads one value, and what follows it stays a gap.
        Parsed::Ends(end) => {
            end as u64 <= len && (end as u64 == len || (head.len() as u64 == len && only_space(&head[end..])))
        }
        // The window ran out before the dictionary did, which is what a
        // torrent bigger than the window looks like. With the whole file in
        // hand it means the file itself was cut off, and then there is no
        // dictionary to claim.
        Parsed::Cut => (head.len() as u64) < len,
        Parsed::No => false,
    }
}

/// Where the value starting at `at` ends, or why it does not.
fn scan(b: &[u8], at: usize, depth: u32) -> Parsed {
    if depth > MAX_DEPTH {
        return Parsed::No;
    }
    match b.get(at) {
        None => Parsed::Cut,
        Some(b'i') => {
            let mut i = at + 1;
            if b.get(i) == Some(&b'-') {
                i += 1;
            }
            let first = i;
            while b.get(i).is_some_and(|c| c.is_ascii_digit()) {
                i += 1;
            }
            match b.get(i) {
                None => Parsed::Cut,
                Some(b'e') if i > first => Parsed::Ends(i + 1),
                _ => Parsed::No,
            }
        }
        Some(b'l') => {
            let mut i = at + 1;
            loop {
                match b.get(i) {
                    None => return Parsed::Cut,
                    Some(b'e') => return Parsed::Ends(i + 1),
                    _ => match scan(b, i, depth + 1) {
                        Parsed::Ends(end) => i = end,
                        other => return other,
                    },
                }
            }
        }
        Some(b'd') => {
            let mut i = at + 1;
            loop {
                match b.get(i) {
                    None => return Parsed::Cut,
                    Some(b'e') => return Parsed::Ends(i + 1),
                    // A key is a byte string, whatever the value beside it is.
                    Some(c) if !c.is_ascii_digit() => return Parsed::No,
                    _ => {}
                }
                for _ in 0..2 {
                    match scan(b, i, depth + 1) {
                        Parsed::Ends(end) => i = end,
                        other => return other,
                    }
                }
            }
        }
        Some(c) if c.is_ascii_digit() => {
            let mut i = at;
            let mut n: u64 = 0;
            while let Some(d) = b.get(i).filter(|d| d.is_ascii_digit()) {
                let Some(next) = n.checked_mul(10).and_then(|n| n.checked_add((d - b'0') as u64)) else {
                    return Parsed::No;
                };
                n = next;
                i += 1;
            }
            match b.get(i) {
                None => Parsed::Cut,
                Some(b':') => match (i as u64 + 1).saturating_add(n) {
                    end if end <= b.len() as u64 => Parsed::Ends(end as usize),
                    _ => Parsed::Cut,
                },
                _ => Parsed::No,
            }
        }
        _ => Parsed::No,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    fn read(bytes: &[u8]) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(bytes.to_vec())), Evaluator::new(bencode()))
    }

    #[test]
    fn an_integer_is_its_digits_between_two_letters() {
        let (d, mut ev) = read(b"i42e");
        assert_eq!(
            ev.node(&d, &[0]).unwrap().value,
            Value::Enum { raw: b'i' as i128, name: Some("integer".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Int(42));
        // The letter, the digits and the `e` after them.
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, 4 * 8);

        // The minus is part of the number, not something before it.
        let (d, mut ev) = read(b"i-3e");
        assert_eq!(ev.node(&d, &[1]).unwrap().value, Value::Int(-3));
    }

    #[test]
    fn a_byte_string_is_as_long_as_the_digits_before_its_colon() {
        let (d, mut ev) = read(b"4:spam");
        // No letter opens one, so the marker field is no bytes at all.
        assert_eq!(ev.node(&d, &[0]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().value, Value::Int(4));
        assert_eq!(ev.node(&d, &[1, 1]).unwrap().value, Value::Str("spam".into()));
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, 6 * 8);

        // A string of nothing is still two bytes, and still a string.
        let (d, mut ev) = read(b"0:");
        assert_eq!(ev.node(&d, &[1, 1]).unwrap().value, Value::Str(String::new()));
    }

    #[test]
    fn a_list_runs_to_the_value_that_is_only_an_e() {
        let (d, mut ev) = read(b"l4:spami42ee");
        let items = ev.node(&d, &[1]).unwrap();
        // Two values, and the end marker that closed them.
        assert_eq!(items.child_count, 3);
        assert_eq!(ev.node(&d, &[1, 0, 1, 1]).unwrap().value, Value::Str("spam".into()));
        assert_eq!(ev.node(&d, &[1, 1, 1]).unwrap().value, Value::Int(42));
        assert_eq!(
            ev.node(&d, &[1, 2, 0]).unwrap().value,
            Value::Enum { raw: b'e' as i128, name: Some("end".into()), hex: false }
        );
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, 12 * 8);
    }

    #[test]
    fn a_dictionary_pairs_its_values_up_and_names_them_by_their_keys() {
        let (d, mut ev) = read(b"d3:cow3:moo4:spam4:eggse");
        let entries = ev.node(&d, &[1]).unwrap();
        // Two entries, and the one the end marker landed in.
        assert_eq!(entries.child_count, 3);
        assert_eq!(entries.unit.as_deref(), Some("entry"));
        assert_eq!(ev.node(&d, &[1, 0, 0, 1, 1]).unwrap().value, Value::Str("cow".into()));
        assert_eq!(ev.node(&d, &[1, 0, 1, 1, 1]).unwrap().value, Value::Str("moo".into()));
        assert_eq!(ev.node(&d, &[1, 1, 0, 1, 1]).unwrap().value, Value::Str("spam".into()));
        assert_eq!(ev.node(&d, &[1, 1, 1, 1, 1]).unwrap().value, Value::Str("eggs".into()));
        // An entry is named by its key, which is worth following two levels
        // down: the key is a value, and the value is a string in a length.
        assert_eq!(ev.node(&d, &[1, 0]).unwrap().name, "[0] cow");
        // The last entry is the letter that ended the dictionary, and nothing
        // is read for the value beside it.
        assert_eq!(ev.node(&d, &[1, 2, 0]).unwrap().size_bits, 8);
        assert_eq!(ev.node(&d, &[1, 2, 1]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, 24 * 8);
    }

    #[test]
    fn a_dictionary_holding_a_list_reads_all_the_way_down() {
        let (d, mut ev) = read(b"d4:listli1ei2eee");
        let list = ev.node(&d, &[1, 0, 1, 1]).unwrap();
        assert_eq!(list.child_count, 3);
        assert_eq!(ev.node(&d, &[1, 0, 1, 1, 1, 1]).unwrap().value, Value::Int(2));
        assert_eq!(ev.node(&d, &[]).unwrap().size_bits, 16 * 8);
    }

    #[test]
    fn a_file_is_bencode_when_the_whole_of_it_parses() {
        let whole = |b: &[u8]| is_bencode(b, b.len() as u64);
        assert!(whole(b"d8:announce5:where4:infod6:lengthi12eee"));
        assert!(whole(b"d0:0:e"));
        // A dictionary that stops short of the end, and one that runs past it.
        // White space on the end is its own case, tested below.
        assert!(!whole(b"d3:cow3:mooe4:spam"));
        assert!(!whole(b"d3:cow4:mooe"));
        // Not a dictionary, and a dictionary whose first key is not a string.
        assert!(!whole(b"l3:cowe"));
        assert!(!whole(b"di1ei2ee"));
        assert!(!whole(b"d3:cow3:moo"));

        // A window shorter than the file is as much as can be seen, and it
        // parses as far as it goes. The same bytes as a whole file do not.
        let torrent = b"d8:announce5:where6:pieces20:";
        assert!(is_bencode(torrent, 1 << 20));
        assert!(!is_bencode(torrent, torrent.len() as u64));
    }

    #[test]
    fn a_newline_left_on_the_end_does_not_stop_it_being_bencode() {
        let whole = |b: &[u8]| is_bencode(b, b.len() as u64);
        // Five of libtorrent's own test torrents end with one of these,
        // which is what a tool that wrote the file through a text mode does.
        assert!(whole(b"d3:cow3:mooe\n"));
        assert!(whole(b"d3:cow3:mooe\r\n"));
        assert!(whole(b"d3:cow3:mooe  \n"));
        // Anything that is not white space still is something else on the end.
        assert!(!whole(b"d3:cow3:mooe0"));
        assert!(!whole(b"d3:cow3:mooe\nd3:cow3:mooe"));
    }

    #[test]
    fn a_file_nested_deeper_than_the_scan_goes_is_not_claimed() {
        // The cap is where this scan gives out, not where bencode does, and
        // not where reading one gives out either. A file past it reads as
        // nothing rather than as a file that would take the scan's stack.
        let nest = |n: usize| {
            let mut b = b"d1:a".to_vec();
            b.extend(std::iter::repeat(b'l').take(n));
            b.extend(std::iter::repeat(b'e').take(n + 1));
            b
        };
        let ok = nest(MAX_DEPTH as usize - 4);
        assert!(is_bencode(&ok, ok.len() as u64));
        let deep = nest(MAX_DEPTH as usize + 4);
        assert!(!is_bencode(&deep, deep.len() as u64));
    }

    #[test]
    fn a_torrent_is_recognised_ahead_of_every_other_probe() {
        let mut bytes = b"d8:announce36:http://bt1.archive.org:6969/announce4:infod6:lengthi1234eee".to_vec();
        let len = bytes.len() as u64;
        assert_eq!(crate::formats::sniff(&bytes, len), Some("bencode"));

        // Nesting deep enough to walk the scan off the stack is not bencode
        // as far as this is concerned.
        let deep = MAX_DEPTH as usize + 4;
        bytes = b"d1:a".to_vec();
        bytes.extend(std::iter::repeat(b'l').take(deep));
        bytes.extend(std::iter::repeat(b'e').take(deep + 1));
        let len = bytes.len() as u64;
        assert_eq!(crate::formats::sniff(&bytes, len), None);
    }
}
