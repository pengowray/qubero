//! The single-file model whisper.cpp loads: a `ggml` file from before GGUF.
//!
//! There is no key/value metadata here and no table of tensors. The header is
//! eleven numbers saying how big the model is, then the mel filterbank the
//! audio front end uses, then the vocabulary, then every tensor one after the
//! next to the end of the file. A tensor carries its own shape and its own
//! name, so where one ends is only known by reading it, and nothing in the file
//! says how many there are.
//!
//! Everything is little-endian, and the magic is the four bytes `lmgg`, which
//! is `ggml` written as a 32-bit number.

use super::ggml;
use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// Which type the weights were quantised to, as whisper.cpp writes it. The
/// number in the file also carries the quantisation format version, a thousand
/// per version, so `1009` is version 1 of `q5_1`.
const FTYPE: &[(i128, &str)] = &[
    (0, "all f32"),
    (1, "mostly f16"),
    (2, "mostly q4_0"),
    (3, "mostly q4_1"),
    (4, "mostly q4_1, some f16"),
    (7, "mostly q8_0"),
    (8, "mostly q5_0"),
    (9, "mostly q5_1"),
    (10, "mostly q2_k"),
    (11, "mostly q3_k"),
    (12, "mostly q4_k"),
    (13, "mostly q5_k"),
    (14, "mostly q6_k"),
    (15, "mostly iq2_xxs"),
    (16, "mostly iq2_xs"),
    (17, "mostly iq3_xxs"),
    (18, "mostly iq1_s"),
    (19, "mostly iq4_nl"),
    (20, "mostly iq3_s"),
    (21, "mostly iq2_s"),
    (22, "mostly iq4_xs"),
    (23, "mostly iq1_m"),
    (24, "mostly bf16"),
];

/// The size of the file's `ftype`, split the way whisper.cpp splits it: a
/// thousand per version of the quantisation format, and the type itself below
/// that.
fn quant_version() -> T {
    T::computed(E::field("ftype").div(E::lit(1000)))
}

fn weight_type() -> T {
    let ftype = || E::field("ftype");
    let rest = ftype().sub(ftype().div(E::lit(1000)).mul(E::lit(1000)));
    T::enumeration("GgmlFtype", T::computed(rest), FTYPE)
}

/// The filterbank that turns a spectrogram into the mel bands the encoder
/// expects: `n_mel` rows of `n_fft` weights.
fn filters() -> T {
    T::structure(
        "MelFilters",
        vec![
            ("n_mel", T::i32(Little)),
            ("n_fft", T::i32(Little)),
            ("data", T::array(T::F32(Little), E::field("n_mel").mul(E::field("n_fft")))),
        ],
    )
}

/// One token: a byte count and that many bytes. The bytes are not always
/// text, since a byte-level BPE vocabulary holds fragments of one.
fn token() -> T {
    T::structure_named("Token", "", "text", vec![("len", T::u32(Little)), ("text", T::utf8(E::field("len")))])
        .counted_as("token")
}

fn vocab() -> T {
    T::structure(
        "Vocab",
        vec![("len", T::i32(Little)), ("tokens", T::array(T::Named("Token".into()), E::field("len")))],
    )
}

/// One tensor: its shape, its name, and its weights. Unlike GGUF there is no
/// padding between them, so a tensor starts where the one before it ended.
fn tensor() -> T {
    T::structure_named(
        "Tensor",
        "name",
        "data",
        vec![
            ("n_dims", T::i32(Little)),
            ("name_len", T::i32(Little)),
            ("type", T::enumeration("GgmlType", T::i32(Little), ggml::GGML_TYPE)),
            ("ne", T::array(T::i32(Little), E::field("n_dims"))),
            ("name", T::utf8(E::field("name_len"))),
            // The shape is in this record rather than in a table elsewhere, so
            // both the type and the count come from the fields just above.
            ("data", ggml::weights(E::field("type"), &|| E::product_of("ne"))),
        ],
    )
    .counted_as("tensor")
}

pub fn whisper() -> Template {
    let root = T::structure(
        "Whisper",
        vec![
            ("magic", T::magic(b"lmgg")),
            ("n_vocab", T::i32(Little)),
            ("n_audio_ctx", T::i32(Little)),
            ("n_audio_state", T::i32(Little)),
            ("n_audio_head", T::i32(Little)),
            ("n_audio_layer", T::i32(Little)),
            ("n_text_ctx", T::i32(Little)),
            ("n_text_state", T::i32(Little)),
            ("n_text_head", T::i32(Little)),
            ("n_text_layer", T::i32(Little)),
            ("n_mels", T::i32(Little)),
            ("ftype", T::i32(Little)),
            ("quant_version", quant_version()),
            ("weight_type", weight_type()),
            ("filters", filters()),
            ("vocab", vocab()),
            // Nothing says how many tensors there are: they run to the end of
            // the file, and the last one ends exactly there.
            ("tensors", T::repeat(T::Named("Tensor".into()), Until::End)),
        ],
    );
    Template::new("whisper", root).with_type("Token", token()).with_type("Tensor", tensor())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A model of the shape whisper.cpp writes, small enough to hold here: two
    /// mel bands of three weights, two vocabulary entries, and two tensors, one
    /// of floats and one quantised.
    fn file() -> Vec<u8> {
        let mut b = b"lmgg".to_vec();
        for n in [5i32, 1500, 512, 8, 6, 448, 512, 8, 6, 80, 1009] {
            b.extend_from_slice(&n.to_le_bytes());
        }
        b.extend_from_slice(&2i32.to_le_bytes()); // n_mel
        b.extend_from_slice(&3i32.to_le_bytes()); // n_fft
        for i in 0..6 {
            b.extend_from_slice(&(i as f32).to_le_bytes());
        }
        b.extend_from_slice(&2i32.to_le_bytes()); // vocabulary entries
        for w in ["hello", "!"] {
            b.extend_from_slice(&(w.len() as u32).to_le_bytes());
            b.extend_from_slice(w.as_bytes());
        }
        // encoder.ln_post.bias: four f32 weights.
        b.extend_from_slice(&1i32.to_le_bytes()); // n_dims
        b.extend_from_slice(&21i32.to_le_bytes()); // name length
        b.extend_from_slice(&0i32.to_le_bytes()); // f32
        b.extend_from_slice(&4i32.to_le_bytes()); // ne[0]
        b.extend_from_slice(b"encoder.ln_post.bias\0");
        b.extend_from_slice(&[0; 16]);
        // encoder.conv1.weight: 64 weights as two q5_1 blocks of 24 bytes.
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&20i32.to_le_bytes());
        b.extend_from_slice(&7i32.to_le_bytes()); // q5_1
        b.extend_from_slice(&8i32.to_le_bytes());
        b.extend_from_slice(&8i32.to_le_bytes());
        b.extend_from_slice(b"encoder.conv1.weight");
        b.extend_from_slice(&[0; 48]);
        b
    }

    #[test]
    fn the_header_says_how_big_the_model_is() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(whisper());
        assert_eq!(ev.node(&d, &[3]).unwrap().value, Value::Int(512)); // n_audio_state
        assert_eq!(ev.node(&d, &[11]).unwrap().value, Value::Int(1009)); // ftype
    }

    #[test]
    fn the_size_of_the_weights_reads_as_a_version_and_a_type() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(whisper());
        assert_eq!(ev.node(&d, &[12]).unwrap().value, Value::Int(1));
        let ty = ev.node(&d, &[13]).unwrap();
        assert_eq!(ty.value, Value::Enum { raw: 9, name: Some("mostly q5_1".into()), hex: false });
        // Neither takes a byte of the file.
        assert_eq!(ty.size_bits, 0);
    }

    #[test]
    fn the_filterbank_is_as_wide_as_its_own_two_numbers_say() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(whisper());
        assert_eq!(ev.node(&d, &[14, 2]).unwrap().child_count, 6);
    }

    #[test]
    fn the_vocabulary_holds_the_words_it_counts() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(whisper());
        assert_eq!(ev.node(&d, &[15, 1]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[15, 1, 0, 1]).unwrap().value, Value::Str("hello".into()));
        assert_eq!(ev.node(&d, &[15, 1, 1, 1]).unwrap().value, Value::Str("!".into()));
    }

    #[test]
    fn the_tensors_run_to_the_end_of_the_file() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(whisper());
        assert_eq!(ev.node(&d, &[16]).unwrap().child_count, 2);
        assert_eq!(ev.node(&d, &[16, 0, 4]).unwrap().value, Value::Str("encoder.ln_post.bias\0".into()));
        // Four f32 weights, because that is what its own shape says.
        let first = ev.node(&d, &[16, 0, 5]).unwrap();
        assert_eq!((first.type_name.as_str(), first.child_count), ("f32 le[]", 4));
    }

    #[test]
    fn a_quantised_tensor_reads_as_the_blocks_its_type_packs() {
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(whisper());
        // 8 by 8 weights in blocks of 32, so two blocks of 24 bytes.
        let data = ev.node(&d, &[16, 1, 5]).unwrap();
        assert_eq!(data.child_count, 2);
        assert_eq!(ev.node(&d, &[16, 1, 5, 0]).unwrap().size_bits, 24 * 8);
        // The last of them ends where the file does.
        let last = ev.node(&d, &[16, 1, 5, 1]).unwrap();
        assert_eq!(last.offset_bits + last.size_bits, d.len_bits());
    }

    #[test]
    fn a_tensors_weights_say_which_of_its_own_fields_shaped_them() {
        use crate::eval::Role;
        let d = Document::new(MemSource(file()));
        let mut ev = Evaluator::new(whisper());
        let o = ev.origins(&d, &[16, 1, 5]).unwrap();
        let seen: Vec<_> = o.iter().map(|x| (x.role, x.label.as_str(), x.value.as_str())).collect();
        assert!(seen.contains(&(Role::Type, "type", "q5_1")), "{seen:?}");
        assert!(seen.contains(&(Role::Count, "ne", "64")), "{seen:?}");
    }
}
