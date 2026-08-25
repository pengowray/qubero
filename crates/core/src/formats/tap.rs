//! ZX Spectrum .TAP: the bytes the ROM tape routines would have loaded, with a
//! length in front of each block instead of the tone and the timing.
//!
//! A block is a length and that many bytes. The first of those bytes is the
//! flag the ROM checked: 0x00 for a header and 0xff for the data after it. A
//! header block is seventeen bytes describing what comes next, and it is the
//! reason a Spectrum tape could say `Program: MANIC` before loading anything.
//!
//! The last byte of every block is a checksum: all the bytes before it in the
//! block, exclusive-ored together, flag included.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

/// What a header block says is coming.
const BLOCK_TYPE: &[(i128, &str)] = &[(0, "program"), (1, "number array"), (2, "character array"), (3, "code")];

pub fn tap() -> Template {
    Template::new(
        "tap",
        T::structure("TAP", vec![("blocks", T::repeat(block(), Until::End))]),
    )
}

fn block() -> T {
    T::structure_named(
        "Block",
        "flag",
        "body",
        vec![
            ("length", T::u16(Little)),
            ("flag", T::enumeration("Flag", T::u8(), &[(0x00, "header"), (0xff, "data")])),
            (
                "body",
                T::sized(
                    E::field("length").sub(E::lit(2)),
                    T::switch(E::field("flag"), vec![(0, header())], T::bytes(E::Remaining)),
                ),
            ),
            ("checksum", T::u8()),
        ],
    )
    .counted_as("block")
}

/// The seventeen bytes of a header, of which the last four mean different
/// things for each kind of block: for a program they are the auto-run line and
/// where the variables start, and for code they are the load address.
fn header() -> T {
    T::structure(
        "Header",
        vec![
            ("type", T::enumeration("BlockType", T::u8(), BLOCK_TYPE)),
            ("name", T::text(StrLen::Padded { size: E::lit(10), pad: b' ' }, Encoding::Ascii)),
            ("data_length", T::u16(Little)),
            ("parameter_1", T::u16(Little)),
            ("parameter_2", T::u16(Little)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn a_header_block_names_the_program_the_data_block_after_it_holds() {
        let mut header = vec![0u8, 0]; // flag, then a program
        header.extend_from_slice(b"manic     ");
        header.extend_from_slice(&6912u16.to_le_bytes());
        header.extend_from_slice(&10u16.to_le_bytes());
        header.extend_from_slice(&6912u16.to_le_bytes());
        header.push(0); // checksum, not checked here

        let mut v = (header.len() as u16).to_le_bytes().to_vec();
        v.extend_from_slice(&header);
        let mut data = vec![0xffu8];
        data.extend_from_slice(&[0x11; 8]);
        data.push(0);
        v.extend_from_slice(&(data.len() as u16).to_le_bytes());
        v.extend_from_slice(&data);

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(tap());
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2);
        assert_eq!(
            ev.node(&d, &[0, 0, 1]).unwrap().value,
            Value::Enum { raw: 0, name: Some("header".into()), hex: false }
        );
        // The name is padded with spaces, not NULs, which is what the ROM did.
        assert_eq!(ev.node(&d, &[0, 0, 2, 1]).unwrap().value, Value::Str("manic".into()));
        assert_eq!(ev.node(&d, &[0, 0, 2, 2]).unwrap().value, Value::UInt(6912));
        assert_eq!(ev.node(&d, &[0, 1, 2]).unwrap().size_bits, 8 * 8);
    }
}
