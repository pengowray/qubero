use super::*;
use crate::source::MemSource;
use crate::template::{Anchor, Endian::*, Expr as E, Step, Ty as T};

mod basics;
mod bits;
mod census;
mod explain;
mod expressions;
mod graph;
mod lookups;
mod optional;
mod placement;
mod pointers;
mod reading;
mod reals;
mod streams;
mod tabs;
mod walking;

fn doc(bytes: &[u8]) -> Document<MemSource> {
    Document::new(MemSource(bytes.to_vec()))
}

/// A template whose items sit at offsets held in an earlier array, in the
/// order the offsets are in rather than the order they sit in.
fn pointer_template() -> Template {
    let item = T::structure("Item", vec![("len", T::u8()), ("text", T::utf8(E::field("len")))]);
    Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("count", T::u8()),
                ("ptrs", T::array(T::u16(Big), E::field("count"))),
                ("items", T::pointer_list("ptrs", Anchor::Window, E::lit(0), item)),
            ],
        ),
    )
}

// count, two offsets, a byte belonging to nothing, then the two items with
// the later one pointed at first.
const POINTED: &[u8] = &[2, 0, 10, 0, 6, 0xff, 3, b'b', b'e', b'e', 2, b'o', b'k'];

fn failure(e: EvalError) -> String {
    match e {
        EvalError::Failed(s) => s,
        other => panic!("not a failure: {other:?}"),
    }
}

fn d5() -> Document<MemSource> {
    doc(&[0, 1, 0, 2, 0])
}
