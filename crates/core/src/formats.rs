//! Built-in templates. These double as the test-bed for the IR: anything a
//! format needs that the IR cannot say is a gap in the IR, not in the format.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

pub fn builtin_names() -> &'static [&'static str] {
    &["png", "wasm"]
}

pub fn builtin(name: &str) -> Option<Template> {
    match name {
        "png" => Some(png()),
        "wasm" => Some(wasm()),
        _ => None,
    }
}

/// Pick a built-in template from the first bytes of a file.
pub fn sniff(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if head.starts_with(b"\0asm") {
        Some("wasm")
    } else {
        None
    }
}

pub fn png() -> Template {
    let ihdr = T::structure(
        "IHDR",
        vec![
            ("width", T::u32(Big)),
            ("height", T::u32(Big)),
            ("bit_depth", T::u8()),
            ("color_type", T::u8()),
            ("compression", T::u8()),
            ("filter", T::u8()),
            ("interlace", T::u8()),
        ],
    );
    let chunk = T::structure(
        "Chunk",
        vec![
            ("length", T::u32(Big)),
            ("type", T::utf8(E::lit(4))),
            (
                "data",
                T::sized(
                    E::field("length"),
                    // A text field in an expression is its bytes as a big-endian number.
                    T::switch(E::field("type"), vec![(0x4948_4452, ihdr)], T::bytes(E::field("length"))),
                ),
            ),
            ("crc", T::u32(Big)),
        ],
    );
    Template {
        name: "png".into(),
        root: T::structure(
            "PNG",
            vec![
                ("signature", T::magic(b"\x89PNG\r\n\x1a\n")),
                ("chunks", T::repeat(chunk, Until::FieldBytes { field: "type".into(), bytes: b"IEND".to_vec() })),
            ],
        ),
    }
}

pub fn wasm() -> Template {
    let valtype = T::u8();
    let functype = T::structure(
        "FuncType",
        vec![
            ("form", T::magic(&[0x60])),
            ("param_count", T::leb_u()),
            ("params", T::array(valtype.clone(), E::field("param_count"))),
            ("result_count", T::leb_u()),
            ("results", T::array(valtype, E::field("result_count"))),
        ],
    );
    let type_section =
        T::structure("TypeSection", vec![("count", T::leb_u()), ("types", T::array(functype, E::field("count")))]);
    let limits = T::structure(
        "Limits",
        vec![
            ("flags", T::u8()),
            ("min", T::leb_u()),
            ("max", T::switch(E::field("flags"), vec![(0, T::array(T::u8(), E::lit(0)))], T::leb_u())),
        ],
    );
    let memory_section =
        T::structure("MemorySection", vec![("count", T::leb_u()), ("memories", T::array(limits, E::field("count")))]);
    let function_section = T::structure(
        "FunctionSection",
        vec![("count", T::leb_u()), ("type_indices", T::array(T::leb_u(), E::field("count")))],
    );
    let export = T::structure(
        "Export",
        vec![
            ("name_len", T::leb_u()),
            ("name", T::utf8(E::field("name_len"))),
            ("kind", T::u8()),
            ("index", T::leb_u()),
        ],
    );
    let export_section =
        T::structure("ExportSection", vec![("count", T::leb_u()), ("exports", T::array(export, E::field("count")))]);
    let section = T::structure(
        "Section",
        vec![
            ("id", T::u8()),
            ("size", T::leb_u()),
            (
                "body",
                T::sized(
                    E::field("size"),
                    T::switch(
                        E::field("id"),
                        vec![(1, type_section), (3, function_section), (5, memory_section), (7, export_section)],
                        T::bytes(E::field("size")),
                    ),
                ),
            ),
        ],
    );
    Template {
        name: "wasm".into(),
        root: T::structure(
            "Wasm",
            vec![
                ("magic", T::magic(b"\0asm")),
                ("version", T::u32(Little)),
                ("sections", T::repeat(section, Until::End)),
            ],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn png_parses_ihdr_and_stops_at_iend() {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&640u32.to_be_bytes());
        b.extend_from_slice(&480u32.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(b"IEND");
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(b"trailing junk");
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(png());
        let chunks = ev.node(&d, &[1]).unwrap();
        assert_eq!(chunks.child_count, 2);
        let ihdr = ev.node(&d, &[1, 0, 2]).unwrap();
        assert_eq!(ihdr.type_name, "IHDR");
        assert_eq!(ev.node(&d, &[1, 0, 2, 1]).unwrap().value, Value::UInt(480));
        assert_eq!(ev.node(&d, &[1, 1, 1]).unwrap().value, Value::Str("IEND".into()));
    }

    #[test]
    fn wasm_type_section() {
        let mut b = b"\0asm".to_vec();
        b.extend_from_slice(&1u32.to_le_bytes());
        let body = [1u8, 0x60, 2, 0x7f, 0x7f, 1, 0x7f];
        b.push(1);
        b.push(body.len() as u8);
        b.extend_from_slice(&body);
        b.extend_from_slice(&[0, 3, 9, 9, 9]);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(wasm());
        assert_eq!(ev.node(&d, &[2]).unwrap().child_count, 2);
        let params = ev.node(&d, &[2, 0, 2, 1, 0, 2]).unwrap();
        assert_eq!(params.child_count, 2);
        assert_eq!(params.type_name, "u8[]");
        let custom = ev.node(&d, &[2, 1, 2]).unwrap();
        assert_eq!(custom.value, Value::Bytes { len: 3, preview: vec![9, 9, 9] });
    }
}
