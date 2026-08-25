//! CBOR: every value is one byte and then whatever that byte called for.
//!
//! The first byte splits into three bits of major type and five of additional
//! information. The five bits are the value itself when they are under 24, and
//! otherwise say how many bytes of value follow: one, two, four or eight. A
//! value of 31 means the length is not written at all and the item runs until
//! a break byte, which is 0xff and is itself an item.
//!
//! There is no shift here, so the two halves of that byte are arithmetic on
//! it: the major type is the byte over 32, and the low five bits are what is
//! left after taking that back out. Every switch keys on one of those two
//! expressions, and the item type refers to itself by name, which is what
//! makes an array of maps of arrays readable to the bottom.
//!
//! Nothing marks the front of a CBOR file, so this is a template to pick
//! rather than one to guess at.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// The eight major types.
const MAJOR: &[(i128, &str)] = &[
    (0, "unsigned"),
    (1, "negative"),
    (2, "bytes"),
    (3, "text"),
    (4, "array"),
    (5, "map"),
    (6, "tag"),
    (7, "simple"),
];

/// The whole first bytes worth naming: the ones that are a complete value on
/// their own, and the break that ends a list of unwritten length.
const INITIAL: &[(i128, &str)] = &[(0xf4, "false"), (0xf5, "true"), (0xf6, "null"), (0xf7, "undefined"), (0xff, "break")];

/// The tags in common use, of which the first two are why CBOR can carry a
/// date without anybody agreeing on a string format for one.
const TAG: &[(i128, &str)] = &[
    (0, "date-time string"),
    (1, "epoch seconds"),
    (2, "unsigned bignum"),
    (3, "negative bignum"),
    (4, "decimal fraction"),
    (5, "bigfloat"),
    (21, "base64url expected"),
    (24, "encoded cbor"),
    (32, "uri"),
    (35, "regexp"),
    (55799, "cbor magic"),
];

/// The major type: the first byte over 32.
fn major() -> E {
    E::field("initial").div(E::lit(32))
}

/// The additional information: the five bits left after the major type.
fn ai() -> E {
    E::field("initial").sub(E::field("initial").div(E::lit(32)).mul(E::lit(32)))
}

pub fn cbor() -> Template {
    Template::new("cbor", T::Named("Item".into())).with_type("Item", item())
}

fn item() -> T {
    T::structure_named(
        "Item",
        "",
        "value",
        vec![
            ("initial", T::enumeration_hex("Initial", T::u8(), INITIAL)),
            ("major", T::enumeration("Major", T::computed(major()), MAJOR)),
            ("argument", argument()),
            ("value", body()),
        ],
    )
}

/// What follows the first byte when the five bits said a length or a value was
/// written out. A float is the same bytes read the other way, and the three
/// first bytes that mean one say so outright.
fn argument() -> T {
    T::switch(
        E::field("initial"),
        vec![(0xf9, T::F16(Big)), (0xfa, T::F32(Big)), (0xfb, T::F64(Big))],
        T::switch(
            ai(),
            vec![(24, T::u8()), (25, T::u16(Big)), (26, T::u32(Big)), (27, T::u64(Big))],
            T::bytes(E::lit(0)),
        ),
    )
}

/// The length or count an item declared, whether it was in the five bits or in
/// the bytes after them. Both are asked for by hand rather than with "the
/// first of these that is not zero", since a length of zero written out in a
/// following byte is a real thing and that test cannot see it.
fn counted(make: fn(E) -> T) -> T {
    let from_argument = make(E::field("argument"));
    T::switch(
        ai(),
        vec![
            (24, from_argument.clone()),
            (25, from_argument.clone()),
            (26, from_argument.clone()),
            (27, from_argument),
            // No length at all: the pieces run until a break item.
            (31, T::repeat(T::Named("Item".into()), Until::FieldBytes { field: "initial".into(), bytes: vec![0xff] })),
        ],
        make(ai()),
    )
}

fn body() -> T {
    T::switch(
        major(),
        vec![
            (2, counted(|n| T::bytes(n))),
            (3, counted(|n| T::utf8(n))),
            (4, counted(|n| T::array(T::Named("Item".into()), n).counted_as("item"))),
            (5, counted(|n| T::array(pair(), n).counted_as("pair"))),
            // A tag is a name for the one item after it.
            (6, T::structure("Tagged", vec![("tag", T::enumeration("Tag", T::computed(tag_number()), TAG)), ("item", T::Named("Item".into()))])),
        ],
        // Unsigned, negative and simple values are all said by the first byte
        // and the argument after it, so there is nothing more to read.
        T::bytes(E::lit(0)),
    )
}

/// The tag number, which is the argument when one was written and the five
/// bits when it was not. Named, since the numbers are a registry.
///
/// This one does take "the first that is not zero", so tag 0 written the long
/// way, as 0xd8 0x00, reads as tag 24. Encoders write the short form, and the
/// alternative is another five-case switch for a number that is shown rather
/// than used to measure anything.
fn tag_number() -> E {
    E::field("argument").or(ai())
}

fn pair() -> T {
    T::inline_structure("Pair", vec![("key", T::Named("Item".into())), ("value", T::Named("Item".into()))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn a_small_integer_is_the_whole_of_its_first_byte() {
        let d = Document::new(MemSource(vec![0x0a]));
        let mut ev = Evaluator::new(cbor());
        assert_eq!(ev.node(&d, &[2]).unwrap().size_bits, 0);
        assert_eq!(ev.node(&d, &[3]).unwrap().size_bits, 0);
        assert_eq!(
            ev.node(&d, &[1]).unwrap().value,
            Value::Enum { raw: 0, name: Some("unsigned".into()), hex: false }
        );
    }

    #[test]
    fn a_map_of_text_to_values_reads_all_the_way_down() {
        // {"a": [1, 2], "b": "hi"}
        let bytes = vec![
            0xa2, // map of two pairs
            0x61, b'a', // "a"
            0x82, 0x01, 0x02, // [1, 2]
            0x61, b'b', // "b"
            0x62, b'h', b'i', // "hi"
        ];
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(cbor());
        let map = ev.node(&d, &[3]).unwrap();
        assert_eq!(map.child_count, 2);
        assert_eq!(ev.node(&d, &[3, 0, 0, 3]).unwrap().value, Value::Str("a".into()));
        // The array under that key, and the second of its two items.
        assert_eq!(ev.node(&d, &[3, 0, 1, 3]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[3, 0, 1, 3, 1, 0]).unwrap().value, Value::Enum { raw: 2, name: None, hex: true });
        assert_eq!(ev.node(&d, &[3, 1, 1, 3]).unwrap().value, Value::Str("hi".into()));
    }

    #[test]
    fn a_length_written_out_is_read_from_the_argument() {
        // A text string of five bytes, with the length in a byte of its own.
        let mut bytes = vec![0x78, 0x05];
        bytes.extend_from_slice(b"hello");
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(cbor());
        assert_eq!(ev.node(&d, &[2]).unwrap().value, Value::UInt(5));
        assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Str("hello".into()));
    }

    #[test]
    fn a_list_of_unwritten_length_runs_to_the_break() {
        // [_ 1, 2] followed by the break byte.
        let d = Document::new(MemSource(vec![0x9f, 0x01, 0x02, 0xff]));
        let mut ev = Evaluator::new(cbor());
        let items = ev.node(&d, &[3]).unwrap();
        // Two items, and the break that ends them.
        assert_eq!(items.child_count, 3);
        assert_eq!(
            ev.node(&d, &[3, 2, 0]).unwrap().value,
            Value::Enum { raw: 0xff, name: Some("break".into()), hex: true }
        );
    }
}
