//! CorelDRAW CDR and CMX binary files built on RIFF.
//!
//! Both formats use little-endian RIFF chunks, including RIFF's even-byte
//! padding rule. Known chunk ids are named, while their proprietary payloads
//! stay byte-for-byte intact. LIST chunks expose their list type and one level
//! of member chunks, which is where pages, layers, resources and previews are
//! normally separated.

use crate::template::{Encoding, Endian::Little, Expr as E, StrLen, Template, Ty as T, Until};

pub fn cdr() -> Template {
    riff("cdr")
}
pub fn cmx() -> Template {
    riff("cmx")
}

fn riff(name: &str) -> Template {
    Template::new(
        name,
        T::structure(
            "CorelRiff",
            vec![
                ("magic", T::magic(b"RIFF")),
                ("file_size_minus_8", T::u32(Little)),
                ("form_type", fourcc()),
                ("chunks", T::repeat(chunk(), Until::End)),
            ],
        ),
    )
}

fn fourcc() -> T {
    T::text(StrLen::Fixed(E::lit(4)), Encoding::Ascii)
}

fn chunk() -> T {
    let pad = E::field("size").pad_to(2);
    T::structure_named(
        "CorelChunk",
        "id",
        "body",
        vec![
            ("id", fourcc()),
            ("size", T::u32(Little)),
            (
                "body",
                T::sized(
                    E::field("size"),
                    T::matches(
                        E::field("id"),
                        vec![("LIST", list())],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
            ("pad", T::bytes(pad)),
        ],
    )
    .counted_as("chunk")
}

fn list() -> T {
    T::structure(
        "CorelList",
        vec![
            ("list_type", fourcc()),
            ("members", T::repeat(raw_chunk(), Until::End)),
        ],
    )
}

fn raw_chunk() -> T {
    let pad = E::field("size").pad_to(2);
    T::structure_named(
        "CorelListMember",
        "id",
        "data",
        vec![
            ("id", fourcc()),
            ("size", T::u32(Little)),
            ("data", T::bytes(E::field("size"))),
            ("pad", T::bytes(pad)),
        ],
    )
    .counted_as("chunk")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, source::MemSource};

    #[test]
    fn odd_chunks_keep_their_riff_padding() {
        let bytes = b"RIFF\x10\0\0\0CDR9vers\x03\0\0\0abc\0";
        let d = Document::new(MemSource(bytes.to_vec()));
        let mut ev = Evaluator::new(cdr());
        assert_eq!(ev.node(&d, &[3, 0, 2]).unwrap().size_bits, 24);
        assert_eq!(ev.node(&d, &[3, 0, 3]).unwrap().size_bits, 8);
    }
}
