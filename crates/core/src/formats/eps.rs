//! Encapsulated PostScript, including the DOS EPS binary preview header.
//!
//! Plain EPS is a DSC line stream beginning `%!PS-Adobe-`. The binary variant
//! points independently at its PostScript program and optional WMF/TIFF
//! previews, so all three are placed at their declared file offsets.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

pub fn eps() -> Template {
    Template::new(
        "eps",
        T::switch(
            E::peek(32, Little),
            vec![(0xc6d3_d0c5, binary())],
            postscript(),
        ),
    )
}

fn binary() -> T {
    T::structure(
        "DosEps",
        vec![
            ("magic", T::magic(b"\xc5\xd0\xd3\xc6")),
            ("postscript_offset", T::u32(Little)),
            ("postscript_size", T::u32(Little)),
            ("wmf_offset", T::u32(Little)),
            ("wmf_size", T::u32(Little)),
            ("tiff_offset", T::u32(Little)),
            ("tiff_size", T::u32(Little)),
            ("checksum", T::u16(Little)),
            (
                "postscript",
                T::at(
                    E::field("postscript_offset"),
                    T::sized(E::field("postscript_size"), postscript()),
                ),
            ),
            (
                "wmf_preview",
                T::at(E::field("wmf_offset"), T::bytes(E::field("wmf_size"))),
            ),
            (
                "tiff_preview",
                T::at(E::field("tiff_offset"), T::bytes(E::field("tiff_size"))),
            ),
        ],
    )
}

fn postscript() -> T {
    T::structure(
        "EncapsulatedPostScript",
        vec![
            (
                "header",
                T::text(
                    StrLen::Terminated {
                        end: b'\n',
                        or_end: true,
                    },
                    Encoding::Ascii,
                ),
            ),
            ("lines", T::repeat(line(), Until::End)),
        ],
    )
}

fn line() -> T {
    T::structure_named(
        "PostScriptLine",
        "text",
        "",
        vec![(
            "text",
            T::text(
                StrLen::Terminated {
                    end: b'\n',
                    or_end: true,
                },
                Encoding::Latin1,
            ),
        )],
    )
    .counted_as("line")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    #[test]
    fn dos_header_places_the_program_and_preview() {
        let ps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 1 1\n";
        let mut v = b"\xc5\xd0\xd3\xc6".to_vec();
        v.extend_from_slice(&30u32.to_le_bytes());
        v.extend_from_slice(&(ps.len() as u32).to_le_bytes());
        v.extend_from_slice(&(30u32 + ps.len() as u32).to_le_bytes());
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0xffffu16.to_le_bytes());
        v.extend_from_slice(ps);
        v.extend_from_slice(&[1, 2]);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(eps());
        assert_eq!(ev.node(&d, &[8, 0]).unwrap().offset_bits, 30 * 8);
        assert_eq!(ev.node(&d, &[9, 0]).unwrap().size_bits, 16);
    }
}
