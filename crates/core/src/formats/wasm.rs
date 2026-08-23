//! WebAssembly binary format: header plus a run of sections.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// Section ids from the core spec, in id order.
const SECTION: &[(i128, &str)] = &[
    (0, "custom"),
    (1, "type"),
    (2, "import"),
    (3, "function"),
    (4, "table"),
    (5, "memory"),
    (6, "global"),
    (7, "export"),
    (8, "start"),
    (9, "element"),
    (10, "code"),
    (11, "data"),
    (12, "data count"),
];

const VALTYPE: &[(i128, &str)] = &[
    (0x7f, "i32"),
    (0x7e, "i64"),
    (0x7d, "f32"),
    (0x7c, "f64"),
    (0x7b, "v128"),
    (0x70, "funcref"),
    (0x6f, "externref"),
];

const EXPORT_KIND: &[(i128, &str)] = &[(0, "func"), (1, "table"), (2, "memory"), (3, "global")];

pub fn wasm() -> Template {
    let valtype = T::enumeration("ValType", T::u8(), VALTYPE);
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
            ("flags", T::enumeration("LimitsFlags", T::u8(), &[(0, "min only"), (1, "min and max")])),
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
            ("kind", T::enumeration("ExportKind", T::u8(), EXPORT_KIND)),
            ("index", T::leb_u()),
        ],
    );
    let export_section =
        T::structure("ExportSection", vec![("count", T::leb_u()), ("exports", T::array(export, E::field("count")))]);
    let section = T::structure(
        "Section",
        vec![
            ("id", T::enumeration("SectionId", T::u8(), SECTION)),
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
        assert_eq!(params.type_name, "ValType[]");
        // The section id is an enum and the body switch still keys off its number.
        let id = ev.node(&d, &[2, 0, 0]).unwrap();
        assert_eq!(id.value, Value::Enum { raw: 1, name: Some("type".into()) });
        assert_eq!(ev.node(&d, &[2, 0, 2]).unwrap().type_name, "TypeSection");
        assert_eq!(ev.node(&d, &[2, 0, 2, 1, 0, 2, 0]).unwrap().value, Value::Enum { raw: 0x7f, name: Some("i32".into()) });
        let custom = ev.node(&d, &[2, 1, 2]).unwrap();
        assert_eq!(custom.value, Value::Bytes { len: 3, preview: vec![9, 9, 9] });
    }
}
