//! The block layouts ggml packs weights into, shared by every format that
//! stores them: GGUF, and the older single-file models whisper.cpp reads.
//!
//! A quantised tensor is not a run of numbers: it is a run of blocks, each
//! holding one scale (sometimes two) shared by the weights packed after it,
//! four or five or six bits apiece. Which block, and how big, is what the
//! type says, so `q4_k` is 256 weights in 144 bytes and `q8_0` is 32 in 34.
//! The blocks are all one size, so the run is counted by division and a block
//! in the middle of a tensor of a hundred thousand is one step away.
//!
//! The packed weights are left as bytes here. Unpacking them means shifting
//! four-bit fields out of a byte and scaling each by a six-bit scale that is
//! itself packed six to a byte, and no template says that: what is here says
//! where every block and every scale is, which is what a file this size is
//! usually opened to check. The numbers themselves come from
//! [`ggml_quant`](super::ggml_quant), which a block names by carrying the name
//! of its packing.
//!
//! Sizes and layouts are those of ggml's own block structs. The IQ types and
//! the ternary ones pack their weights in ways nothing outside ggml reads, so
//! those blocks are the right size and opaque inside.

use crate::template::{Endian::*, Expr as E, Ty as T};

/// How one tensor's numbers are stored. The quantised types pack several
/// weights and a shared scale into a block, which is why there are so many of
/// them. 4 and 5 were a quantisation that was withdrawn, and no file that can
/// still be loaded holds one.
pub const GGML_TYPE: &[(i128, &str)] = &[
    (0, "f32"),
    (1, "f16"),
    (2, "q4_0"),
    (3, "q4_1"),
    (6, "q5_0"),
    (7, "q5_1"),
    (8, "q8_0"),
    (9, "q8_1"),
    (10, "q2_k"),
    (11, "q3_k"),
    (12, "q4_k"),
    (13, "q5_k"),
    (14, "q6_k"),
    (15, "q8_k"),
    (16, "iq2_xxs"),
    (17, "iq2_xs"),
    (18, "iq3_xxs"),
    (19, "iq1_s"),
    (20, "iq4_nl"),
    (21, "iq3_s"),
    (22, "iq2_s"),
    (23, "iq4_xs"),
    (24, "i8"),
    (25, "i16"),
    (26, "i32"),
    (27, "i64"),
    (28, "f64"),
    (29, "iq1_m"),
    (30, "bf16"),
];


/// One tensor's numbers, by the ggml type its record names.
///
/// Each format reaches its own record its own way, so both questions come in
/// as expressions: `ty` is where the ggml type number is, and `weights_in`
/// says how many numbers the tensor holds. GGUF multiplies the shape in its
/// tensor table; an older ggml file holds both in the record itself.
pub fn weights(ty: E, weights_in: &dyn Fn() -> E) -> T {
    let f16 = || T::F16(Little);
    let raw = |n: i128| T::bytes(E::lit(n));
    // How many numbers this tensor holds, which is what its shape says and
    // not what the room before the next tensor says: a small tensor is
    // followed by padding out to the next 32-byte boundary, and that padding
    // is not weights.
    let count = |per_block: i128| weights_in().div(E::lit(per_block));
    // A run of plain numbers is one per weight; a run of blocks is one per
    // however many weights that type packs together.
    let run = |ty: T| T::array(ty, count(1));
    // A block whose weights this crate can take apart carries the name of its
    // packing, so the value panel can show the numbers rather than the bytes.
    // See [`super::ggml_quant`].
    let blocks = |per_block: i128, name: &str, fields: Vec<(&str, T)>| {
        let one = T::inline_structure(name, fields).counted_as("block");
        let one = match super::ggml_quant::by_name(name) {
            Some(_) => one.packed_as(name),
            None => one,
        };
        T::array(one, count(per_block))
    };
    // A block nothing outside ggml unpacks: the right size, and no claim
    // about what is inside it.
    let opaque = |per_block: i128, size: i128, name: &str| blocks(per_block, name, vec![("packed", raw(size))]);
    T::switch(
        ty,
        vec![
            (0, run(T::F32(Little))),
            (1, run(f16())),
            (2, blocks(32, "Q4_0", vec![("d", f16()), ("qs", raw(16))])),
            (3, blocks(32, "Q4_1", vec![("d", f16()), ("m", f16()), ("qs", raw(16))])),
            (6, blocks(32, "Q5_0", vec![("d", f16()), ("qh", T::u32(Little)), ("qs", raw(16))])),
            (7, blocks(32, "Q5_1", vec![("d", f16()), ("m", f16()), ("qh", T::u32(Little)), ("qs", raw(16))])),
            (8, blocks(32, "Q8_0", vec![("d", f16()), ("qs", T::array(T::Int { bits: 8, endian: Little }, E::lit(32)))])),
            (9, blocks(32, "Q8_1", vec![("d", f16()), ("s", f16()), ("qs", raw(32))])),
            (10, blocks(256, "Q2_K", vec![("scales", raw(16)), ("qs", raw(64)), ("d", f16()), ("dmin", f16())])),
            (11, blocks(256, "Q3_K", vec![("hmask", raw(32)), ("qs", raw(64)), ("scales", raw(12)), ("d", f16())])),
            (12, blocks(256, "Q4_K", vec![("d", f16()), ("dmin", f16()), ("scales", raw(12)), ("qs", raw(128))])),
            (13, blocks(256, "Q5_K", vec![("d", f16()), ("dmin", f16()), ("scales", raw(12)), ("qh", raw(32)), ("qs", raw(128))])),
            (14, blocks(256, "Q6_K", vec![("ql", raw(128)), ("qh", raw(64)), ("scales", raw(16)), ("d", f16())])),
            (15, blocks(256, "Q8_K", vec![("d", T::F32(Little)), ("qs", raw(256)), ("bsums", raw(32))])),
            (16, opaque(256, 66, "IQ2_XXS")),
            (17, opaque(256, 74, "IQ2_XS")),
            (18, opaque(256, 98, "IQ3_XXS")),
            (19, opaque(256, 50, "IQ1_S")),
            (20, blocks(32, "IQ4_NL", vec![("d", f16()), ("qs", raw(16))])),
            (21, opaque(256, 110, "IQ3_S")),
            (22, opaque(256, 82, "IQ2_S")),
            (23, blocks(256, "IQ4_XS", vec![("d", f16()), ("scales_h", T::u16(Little)), ("scales_l", raw(4)), ("qs", raw(128))])),
            (24, run(T::Int { bits: 8, endian: Little })),
            (25, run(T::Int { bits: 16, endian: Little })),
            (26, run(T::i32(Little))),
            (27, run(T::Int { bits: 64, endian: Little })),
            (28, run(T::F64(Little))),
            (29, opaque(256, 56, "IQ1_M")),
            (30, run(T::BF16(Little))),
            (34, opaque(256, 54, "TQ1_0")),
            (35, opaque(256, 66, "TQ2_0")),
            (39, blocks(32, "MXFP4", vec![("e", T::u8()), ("qs", raw(16))])),
            (40, opaque(64, 36, "NVFP4")),
            (41, opaque(128, 18, "Q1_0")),
        ],
        // A type added to ggml since this was written: where it is and how
        // much of it there is, and nothing invented about the rest.
        T::bytes(E::Remaining),
    )
}
