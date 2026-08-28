//! The block GRUB keeps its variables in, which is how a bootloader
//! remembers which kernel you last chose and whether the last boot got
//! anywhere.
//!
//! It is a text file with a fixed size: one line of signature, a line per
//! variable, and then `#` all the way to 1024 bytes. The padding is the point
//! of the format. Nothing has to allocate, so GRUB can rewrite a variable
//! from inside the boot loader by writing the block back in place, without a
//! filesystem driver that can grow a file.

use crate::template::{Encoding, Expr as E, StrLen, Template, Ty as T, Until};

/// The first line, which is also the whole of the format's magic.
pub const SIGNATURE: &[u8] = b"# GRUB Environment Block\n";

pub fn grubenv() -> Template {
    Template::new(
        "grubenv",
        T::structure(
            "GrubEnv",
            vec![
                ("signature", T::magic(SIGNATURE)),
                ("variables", T::repeat(line(), Until::End)),
            ],
        ),
    )
}

/// One line, or the run of `#` that fills the block out to its size. Told
/// apart by the byte the line starts with, since a variable's name cannot
/// begin with one: GRUB writes the padding as comment characters so that a
/// reader that does not know the size still stops at the right place.
fn line() -> T {
    T::switch(
        E::peek(8, crate::template::Endian::Big),
        vec![(i128::from(b'#'), T::bytes(E::Remaining))],
        variable(),
    )
}

fn variable() -> T {
    T::structure_named(
        "Variable",
        "name",
        "value",
        vec![
            ("name", T::text(StrLen::Terminated { end: b'=', or_end: true }, Encoding::Utf8)),
            ("value", T::text(StrLen::Terminated { end: b'\n', or_end: true }, Encoding::Utf8)),
        ],
    )
    .counted_as("variable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    fn block(vars: &[&str]) -> Vec<u8> {
        let mut v = SIGNATURE.to_vec();
        for line in vars {
            v.extend_from_slice(line.as_bytes());
            v.push(b'\n');
        }
        v.resize(1024, b'#');
        v
    }

    #[test]
    fn the_variables_read_as_lines_and_the_rest_is_padding() {
        let d = Document::new(MemSource(block(&["saved_entry=2", "boot_success=0"])));
        let mut e = Evaluator::new(grubenv());
        // Two variables and the run of padding after them.
        assert_eq!(e.node(&d, &[1]).unwrap().child_count, 3);
        assert_eq!(e.node(&d, &[1, 0, 0]).unwrap().value, Value::Str("saved_entry".into()));
        assert_eq!(e.node(&d, &[1, 0, 1]).unwrap().value, Value::Str("2".into()));
        assert_eq!(e.node(&d, &[1, 1, 0]).unwrap().value, Value::Str("boot_success".into()));
        // The padding is one run to the end, not a record per character.
        let padding = e.node(&d, &[1, 2]).unwrap();
        assert_eq!(padding.size_bits, (1024 - 25 - 14 - 15) * 8);
    }

    /// A block with nothing in it yet is the signature and 999 `#`.
    #[test]
    fn an_empty_block_is_all_padding() {
        let d = Document::new(MemSource(block(&[])));
        let mut e = Evaluator::new(grubenv());
        assert_eq!(e.node(&d, &[1]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[1, 0]).unwrap().size_bits, (1024 - 25) * 8);
    }
}
