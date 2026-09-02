//! NASA CDF: the file a space physics mission is published as.
//!
//! MMS, Cluster, Voyager, Parker Solar Probe: what comes down from CDAWeb is
//! one of these. It shares the letters CDF with NetCDF classic and nothing
//! else; that one is `netcdf`, and it announces itself with `CDF` and a
//! version byte rather than with the number below.
//!
//! Eight bytes of signature, and then records. Every record says how long it
//! is and what kind it is, and the kinds refer to each other by offset from
//! the start of the file rather than by lying next to each other. So the
//! structure is a walk: the descriptor record at the front points at the
//! global descriptor, which points at the head of three chains, and each chain
//! is a record holding the offset of the next one of its kind.
//!
//! The chains are the variables, twice over, and the attributes. A variable
//! descriptor names a variable, says what type its values are and how they are
//! shaped, and points at the index that finds its values. There are two lists
//! of them because CDF has two kinds of variable: the rVariables, which all
//! share one shape declared once in the global descriptor, and the zVariables,
//! which each carry their own. Everything written this century is a zVariable.
//! An attribute descriptor names an attribute and heads a chain of entries,
//! one per variable it has been set on, or one for the file as a whole.
//!
//! What is not read is the values. A variable points at an index of index
//! records, and those at the blocks of records that hold the numbers; both
//! stay as sized bytes here, so the data of a file is a gap. So is an
//! attribute's value and a variable's pad value, which are the bytes at the
//! end of their own records: how to read them depends on the encoding named in
//! the descriptor record at the front of the file, which may be any of a dozen
//! machines' byte orders, and reading them the wrong way round would answer
//! with numbers no instrument measured. The records themselves, and every
//! offset in them, are big-endian whatever that encoding says.
//!
//! A compressed file says so in its second word and holds one compressed
//! record, which is the whole of the uncompressed file squeezed; the chains
//! are inside it and nothing here unpacks it. The compression parameters are a
//! record of their own that it points at.
//!
//! Version 2.x is the same idea with 32-bit offsets and a record header half
//! as wide, and it opens with the compression magic twice over. The descriptor
//! record reads here and the global descriptor is placed where it says; the
//! chains below it are not walked, because the record layouts changed with the
//! move to 64 bits and a file that old is a file to look at rather than one to
//! trust a template with.
//!
//! One more thing belongs to no record: a file whose flags say it is checksummed
//! keeps sixteen bytes of MD5 at the very end, after everything the chains
//! reach.

use crate::template::{Anchor, Encoding, Endian::Big, Expr as E, StrLen, Template, Ty as T};

/// What a version 3 file starts with.
pub const MAGIC: &[u8] = &[0xCD, 0xF3, 0x00, 0x01];

/// What a version 2.x file starts with: the word that means "not compressed",
/// written where the version 3 signature goes and again after it.
pub const MAGIC_V2: &[u8] = &[0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0xFF, 0xFF];

/// The kinds of record. Every one of them is a fixed layout after the size and
/// the type, and the ones this reads are the ones that say where something is.
const RECORD_TYPE: &[(i128, &str)] = &[
    (-1, "unused"),
    (1, "descriptor"),
    (2, "global descriptor"),
    (3, "rVariable descriptor"),
    (4, "attribute descriptor"),
    (5, "attribute g/rEntry"),
    (6, "variable index"),
    (7, "variable values"),
    (8, "zVariable descriptor"),
    (9, "attribute zEntry"),
    (10, "compressed"),
    (11, "compression parameters"),
    (12, "sparseness parameters"),
    (13, "compressed variable values"),
];

/// What the numbers in a variable or an attribute entry are. The widths are
/// not read here, only the names: nothing decodes a value.
const DATA_TYPE: &[(i128, &str)] = &[
    (1, "int1"),
    (2, "int2"),
    (4, "int4"),
    (8, "int8"),
    (11, "uint1"),
    (12, "uint2"),
    (14, "uint4"),
    (21, "real4"),
    (22, "real8"),
    (31, "epoch"),
    (32, "epoch16"),
    (33, "tt2000"),
    (41, "byte"),
    (44, "float"),
    (45, "double"),
    (51, "char"),
    (52, "uchar"),
];

/// Whose byte order and floating-point format the values are in. This is the
/// one thing in the file that is not big-endian: the records are, and the
/// numbers inside a variable are whatever this says.
const ENCODING: &[(i128, &str)] = &[
    (1, "network"),
    (2, "Sun"),
    (3, "VAX"),
    (4, "DECstation"),
    (5, "SGi"),
    (6, "IBM PC"),
    (7, "IBM RS"),
    (8, "host"),
    (9, "PPC"),
    (11, "HP"),
    (12, "NeXT"),
    (13, "Alpha OSF/1"),
    (14, "Alpha VMS d"),
    (15, "Alpha VMS g"),
    (16, "Alpha VMS i"),
    (17, "ARM little"),
    (18, "ARM big"),
    (19, "IA64 VMS i"),
    (20, "IA64 VMS d"),
    (21, "IA64 VMS g"),
];

/// Who an attribute belongs to. An assumed scope is one the writer did not say
/// outright and the library worked out from what it was set on.
const SCOPE: &[(i128, &str)] = &[
    (1, "global"),
    (2, "variable"),
    (3, "global assumed"),
    (4, "variable assumed"),
];

/// How a compressed record was squeezed.
const COMPRESSION: &[(i128, &str)] = &[(1, "run-length"), (2, "Huffman"), (3, "adaptive Huffman"), (5, "gzip")];

fn i32be() -> T {
    T::i32(Big)
}

fn u64be() -> T {
    T::u64(Big)
}

/// An offset into the file, which is what every link in this format is. Zero
/// means there is nothing there.
fn offset() -> T {
    T::Int { bits: 64, endian: Big }
}

/// A name, in the fixed 256 bytes a version 3 file gives one, padded with nuls.
fn name() -> T {
    T::text(StrLen::Padded { size: E::lit(256), pad: 0 }, Encoding::Ascii)
}

fn data_type() -> T {
    T::enumeration("CdfDataType", i32be(), DATA_TYPE)
}

/// The chain of records that starts at `field` and runs on through the `next`
/// pointer inside each record.
///
/// Every list in a CDF is written this way: no count, no table, just a head
/// offset and a forward pointer in every record. Written as a record holding
/// the record after it, which is the only shape there was before
/// [`Ty::Chain`](crate::template::Ty::Chain), a file with two hundred
/// attributes is a tree two hundred levels deep, and the two hundredth
/// attribute sits behind two hundred rows the reader has to open one at a
/// time. It is a list, and this says so.
fn chain_of(field: &str, next: &str) -> T {
    T::chain(E::field(field), &["body", next], Anchor::File, T::Named("CdfRecord".into()))
}

/// The record `field` points at, or nothing where it holds zero. Every chain
/// and every pointer in the file ends this way rather than with a count.
fn at_record(field: &str) -> T {
    T::switch(
        E::field(field),
        vec![(0, T::bytes(E::lit(0)))],
        T::at(E::field(field), T::Named("CdfRecord".into())),
    )
}

/// The descriptor record: which release of the library wrote the file, how its
/// numbers are encoded, and where the global descriptor is. Always the first
/// record, at offset eight.
fn cdr() -> T {
    T::structure(
        "CdfDescriptor",
        vec![
            ("gdr_offset", offset()),
            ("version", i32be()),
            ("release", i32be()),
            ("encoding", T::enumeration("CdfEncoding", i32be(), ENCODING)),
            (
                "flags",
                T::flags(
                    "CdfFlags",
                    i32be(),
                    &[(0, "row-major"), (1, "single file"), (2, "checksum"), (3, "MD5 checksum")],
                ),
            ),
            ("rfu_a", i32be()),
            ("rfu_b", i32be()),
            ("increment", i32be()),
            ("rfu_d", i32be()),
            ("rfu_e", i32be()),
            // 256 bytes of the notice every CDF carries, nul-padded.
            ("copyright", T::text(StrLen::Padded { size: E::Remaining, pad: 0 }, Encoding::Ascii)),
            ("gdr", at_record("gdr_offset")),
        ],
    )
}

/// The global descriptor: the head of each of the three chains, where the file
/// ends, and the shape every rVariable shares.
fn gdr() -> T {
    T::structure(
        "CdfGlobalDescriptor",
        vec![
            ("r_vdr_head", offset()),
            ("z_vdr_head", offset()),
            ("adr_head", offset()),
            // Where the records stop. A checksummed file has sixteen more
            // bytes after this, and nothing else should.
            ("eof", offset()),
            ("n_r_vars", i32be()),
            ("num_attr", i32be()),
            ("r_max_rec", i32be()),
            ("r_num_dims", i32be()),
            ("n_z_vars", i32be()),
            // The head of the free list: records the file has finished with
            // and would write over before it grew.
            ("uir_head", offset()),
            ("rfu_c", i32be()),
            // The day the leap second table the file was written against was
            // last changed, as YYYYMMDD. Reserved until 3.6, which is why a
            // file older than that writes -1 here.
            ("leap_second_last_updated", i32be()),
            ("rfu_e", i32be()),
            ("r_dim_sizes", T::array(i32be(), E::field("r_num_dims").at_least(E::lit(0)))),
            ("r_variables", chain_of("r_vdr_head", "vdr_next")),
            ("z_variables", chain_of("z_vdr_head", "vdr_next")),
            ("attributes", chain_of("adr_head", "adr_next")),
            ("unused", chain_of("uir_head", "uir_next")),
        ],
    )
}

/// One attribute: its name, who it belongs to, and the heads of its two lists
/// of entries. An attribute has an entry per rVariable and an entry per
/// zVariable, and a global attribute keeps its values in the first list.
fn adr() -> T {
    T::structure_named(
        "CdfAttribute",
        "name",
        "",
        vec![
            ("adr_next", offset()),
            ("agr_edr_head", offset()),
            ("scope", T::enumeration("CdfScope", i32be(), SCOPE)),
            ("num", i32be()),
            ("n_gr_entries", i32be()),
            ("max_gr_entry", i32be()),
            ("rfu_a", i32be()),
            ("az_edr_head", offset()),
            ("n_z_entries", i32be()),
            ("max_z_entry", i32be()),
            ("rfu_e", i32be()),
            ("name", name()),
            ("g_entries", chain_of("agr_edr_head", "aedr_next")),
            ("z_entries", chain_of("az_edr_head", "aedr_next")),
        ],
    )
}

/// One entry of an attribute: which variable it is set on, what type its value
/// is, and the value itself, which stays bytes.
fn aedr() -> T {
    T::structure(
        "CdfAttributeEntry",
        vec![
            ("aedr_next", offset()),
            ("attr_num", i32be()),
            ("data_type", data_type()),
            // Which variable this entry is about, by its number in the list.
            ("num", i32be()),
            ("num_elements", i32be()),
            ("num_strings", i32be()),
            ("rfu_b", i32be()),
            ("rfu_c", i32be()),
            ("rfu_d", i32be()),
            ("rfu_e", i32be()),
            // The value, as wide as the record has room for. Reading it means
            // knowing the file's encoding, which is not this record's to say.
            ("value", T::bytes(E::Remaining)),
        ],
    )
}

/// One variable. `z` says which of the two kinds: a zVariable writes its own
/// shape, and an rVariable takes the shape the global descriptor declared and
/// writes only which of those dimensions it varies along.
fn vdr(z: bool) -> T {
    let dims = || match z {
        true => E::field("z_num_dims").at_least(E::lit(0)),
        false => E::field("r_num_dims").at_least(E::lit(0)),
    };
    let mut fields = vec![
        ("vdr_next", offset()),
        ("data_type", data_type()),
        // The highest record number written, counting from zero, so -1 is a
        // variable with nothing in it yet.
        ("max_rec", i32be()),
        ("vxr_head", offset()),
        ("vxr_tail", offset()),
        (
            "flags",
            T::flags("CdfVariableFlags", i32be(), &[(0, "record variance"), (1, "pad value"), (2, "compressed")]),
        ),
        // How the records this variable has not been given are stored: not at
        // all, filled with the pad value, or filled with the previous record.
        ("s_records", T::enumeration("CdfSparseness", i32be(), &[(0, "none"), (1, "padded"), (2, "previous")])),
        ("rfu_b", i32be()),
        ("rfu_c", i32be()),
        ("rfu_f", i32be()),
        ("num_elems", i32be()),
        ("num", i32be()),
        // Where the compression or sparseness parameters are, when the flags
        // say there are any.
        ("cpr_or_spr_offset", offset()),
        ("blocking_factor", i32be()),
        ("name", name()),
    ];
    if z {
        fields.push(("z_num_dims", i32be()));
        fields.push(("z_dim_sizes", T::array(i32be(), dims())));
    }
    // Which of the dimensions the values actually vary along. A dimension that
    // does not vary is one value repeated, and is not stored.
    fields.push(("dim_varys", T::array(i32be(), dims())));
    fields.push(("pad_value", T::bytes(E::Remaining)));
    fields.push(("values_index", at_record("vxr_head")));
    T::structure_named(if z { "CdfZVariable" } else { "CdfRVariable" }, "name", "", fields)
}

/// The whole of an uncompressed file, squeezed into one record. The chains are
/// inside it; nothing here unpacks it.
fn ccr() -> T {
    T::structure(
        "CdfCompressed",
        vec![
            ("cpr_offset", offset()),
            // How large it was before, which is what a reader allocates.
            ("u_size", u64be()),
            ("rfu_a", i32be()),
            ("data", T::bytes(E::Remaining)),
            ("parameters", at_record("cpr_offset")),
        ],
    )
}

/// How something was compressed, and with what settings: the gzip level, or
/// nothing at all for the other three.
fn cpr() -> T {
    T::structure(
        "CdfCompressionParameters",
        vec![
            ("c_type", T::enumeration("CdfCompression", i32be(), COMPRESSION)),
            ("rfu_a", i32be()),
            ("p_count", i32be()),
            ("c_parms", T::array(i32be(), E::field("p_count").at_least(E::lit(0)))),
        ],
    )
}

/// A record the file has finished with, and the two it sits between in the
/// free list. Not walked: what it holds is whatever was written there before.
fn uir() -> T {
    T::structure("CdfUnused", vec![("uir_next", offset()), ("uir_prev", offset()), ("free", T::bytes(E::Remaining))])
}

/// Any record: how long it is, what it is, and that many bytes read as the
/// layout its type names.
///
/// Sizing the body from the record's own size is what makes the trailing
/// fields safe. An attribute entry's value, a variable's pad value and the
/// notice in the descriptor record all run to the end of their record and
/// nothing else says how long they are.
fn record() -> T {
    let body = T::switch(
        E::field("type"),
        vec![
            (-1, uir()),
            (1, cdr()),
            (2, gdr()),
            (3, vdr(false)),
            (4, adr()),
            (5, aedr()),
            (8, vdr(true)),
            (9, aedr()),
            (10, ccr()),
            (11, cpr()),
        ],
        // The index of where a variable's values are, the values themselves,
        // and the sparseness parameters: sized, named, and not opened.
        T::bytes(E::Remaining),
    );
    T::structure_named(
        "CdfRecord",
        "type",
        "body",
        vec![
            ("size", u64be()),
            ("type", T::enumeration("CdfRecordType", i32be(), RECORD_TYPE)),
            ("body", T::sized(E::field("size").sub(E::lit(12)).at_least(E::lit(0)), body)),
        ],
    )
}

/// A version 2.x record: the same idea with a 32-bit size and 32-bit offsets.
fn record_v2() -> T {
    let cdr = T::structure(
        "Cdf2Descriptor",
        vec![
            ("gdr_offset", T::u32(Big)),
            ("version", i32be()),
            ("release", i32be()),
            ("encoding", T::enumeration("CdfEncoding", i32be(), ENCODING)),
            ("flags", T::flags("CdfFlags", i32be(), &[(0, "row-major"), (1, "single file")])),
            ("rfu_a", i32be()),
            ("rfu_b", i32be()),
            ("increment", i32be()),
            ("rfu_d", i32be()),
            ("rfu_e", i32be()),
            ("copyright", T::text(StrLen::Padded { size: E::Remaining, pad: 0 }, Encoding::Ascii)),
            (
                "gdr",
                T::switch(
                    E::field("gdr_offset"),
                    vec![(0, T::bytes(E::lit(0)))],
                    T::at(E::field("gdr_offset"), T::Named("Cdf2Record".into())),
                ),
            ),
        ],
    );
    T::structure_named(
        "Cdf2Record",
        "type",
        "body",
        vec![
            ("size", T::u32(Big)),
            ("type", T::enumeration("CdfRecordType", i32be(), RECORD_TYPE)),
            ("body", T::sized(E::field("size").sub(E::lit(8)).at_least(E::lit(0)), T::switch(E::field("type"), vec![(1, cdr)], T::bytes(E::Remaining)))),
        ],
    )
}

pub fn cdf() -> Template {
    let root = T::structure(
        "Cdf",
        vec![
            ("magic", T::enumeration_hex("CdfMagic", T::u32(Big), &[(0xCDF3_0001, "CDF 3"), (0x0000_FFFF, "CDF 2.x")])),
            (
                "compression",
                T::enumeration_hex(
                    "CdfCompressed",
                    T::u32(Big),
                    &[(0x0000_FFFF, "uncompressed"), (0xCCCC_0001, "compressed")],
                ),
            ),
            // The first record, which is the descriptor record of an
            // uncompressed file and the compressed record of a squeezed one.
            // Everything else in the file is reached from it.
            (
                "first_record",
                T::switch(
                    E::field("magic"),
                    vec![(0x0000_FFFF, T::Named("Cdf2Record".into()))],
                    T::Named("CdfRecord".into()),
                ),
            ),
        ],
    );
    Template::new("cdf", root).with_type("CdfRecord", record()).with_type("Cdf2Record", record_v2())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::{Evaluator, Value}, source::MemSource};

    fn be32(v: i32) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }
    fn be64(v: i64) -> Vec<u8> {
        v.to_be_bytes().to_vec()
    }

    /// A record: its size, its type, and its body.
    fn rec(kind: i32, body: Vec<u8>) -> Vec<u8> {
        let mut v = ((body.len() + 12) as u64).to_be_bytes().to_vec();
        v.extend(be32(kind));
        v.extend(body);
        v
    }

    /// A name in the 256 bytes one gets.
    fn nm(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.resize(256, 0);
        v
    }

    /// A small version 3 file: a descriptor record, a global descriptor with
    /// one dimension declared for the rVariables, one rVariable, one
    /// zVariable, and one global attribute with one entry.
    ///
    /// Written twice over, once to measure the records and once with the
    /// offsets that measuring settled, which is what a writer does.
    fn file() -> Vec<u8> {
        let cdr_at = 8i64;
        let build = |o: [i64; 5]| {
            let [gdr_at, rvdr_at, zvdr_at, adr_at, aedr_at] = o;
            let mut cdr = be64(gdr_at);
            cdr.extend(be32(3)); // version
            cdr.extend(be32(8)); // release
            cdr.extend(be32(1)); // network encoding
            cdr.extend(be32(3)); // row-major, single file
            cdr.extend(be32(0));
            cdr.extend(be32(0));
            cdr.extend(be32(0)); // increment
            cdr.extend(be32(-1));
            cdr.extend(be32(-1));
            let mut notice = b"Common Data Format (CDF)".to_vec();
            notice.resize(256, 0);
            cdr.extend(notice);

            let mut gdr = be64(rvdr_at);
            gdr.extend(be64(zvdr_at));
            gdr.extend(be64(adr_at));
            gdr.extend(be64(0)); // eof, which nothing here reads back
            gdr.extend(be32(1)); // one rVariable
            gdr.extend(be32(1)); // one attribute
            gdr.extend(be32(0)); // rMaxRec
            gdr.extend(be32(2)); // two rVariable dimensions
            gdr.extend(be32(1)); // one zVariable
            gdr.extend(be64(0)); // no free list
            gdr.extend(be32(0));
            gdr.extend(be32(-1));
            gdr.extend(be32(-1));
            gdr.extend(be32(4)); // rDimSizes
            gdr.extend(be32(5));

            let vdr = |z: bool, name: &str, dims: &[i32]| {
                let mut v = be64(0); // no next of this kind
                v.extend(be32(45)); // double
                v.extend(be32(-1)); // nothing written yet
                v.extend(be64(0)); // no index
                v.extend(be64(0));
                v.extend(be32(1)); // record variance, no pad value
                v.extend(be32(0));
                v.extend(be32(0));
                v.extend(be32(-1));
                v.extend(be32(-1));
                v.extend(be32(1)); // one element
                v.extend(be32(0)); // number zero
                v.extend(be64(0));
                v.extend(be32(0));
                v.extend(nm(name));
                if z {
                    v.extend(be32(dims.len() as i32));
                    for d in dims {
                        v.extend(be32(*d));
                    }
                }
                for _ in dims {
                    v.extend(be32(1)); // varies
                }
                v
            };

            let mut adr = be64(0); // one attribute only
            adr.extend(be64(aedr_at));
            adr.extend(be32(1)); // global
            adr.extend(be32(0));
            adr.extend(be32(1)); // one entry
            adr.extend(be32(0));
            adr.extend(be32(0));
            adr.extend(be64(0)); // no zEntries
            adr.extend(be32(0));
            adr.extend(be32(0));
            adr.extend(be32(-1));
            adr.extend(nm("TITLE"));

            let mut aedr = be64(0);
            aedr.extend(be32(0)); // attribute zero
            aedr.extend(be32(51)); // char
            aedr.extend(be32(0));
            aedr.extend(be32(5)); // five of them
            aedr.extend(be32(1));
            for _ in 0..4 {
                aedr.extend(be32(0));
            }
            aedr.extend_from_slice(b"depth");

            let mut b = MAGIC.to_vec();
            b.extend_from_slice(&[0x00, 0x00, 0xFF, 0xFF]);
            b.extend(rec(1, cdr));
            b.extend(rec(2, gdr));
            b.extend(rec(3, vdr(false, "r_field", &[1, 1])));
            b.extend(rec(8, vdr(true, "sea_temp", &[3])));
            b.extend(rec(4, adr));
            b.extend(rec(5, aedr));
            b
        };
        // One pass to measure, one to write. Every record but the first is
        // reached by an offset, so the offsets have to be known first.
        let zeros = build([0; 5]);
        let sizes: Vec<i64> = {
            let mut at = cdr_at;
            let mut out = Vec::new();
            for _ in 0..6 {
                out.push(at);
                let size = i64::from_be_bytes(zeros[at as usize..at as usize + 8].try_into().unwrap());
                at += size;
            }
            out
        };
        build([sizes[1], sizes[2], sizes[3], sizes[4], sizes[5]])
    }

    #[test]
    fn the_first_record_says_where_the_global_descriptor_is() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(cdf());
        assert_eq!(
            e.node(&d, &[0]).unwrap().value,
            Value::Enum { raw: 0xCDF3_0001, name: Some("CDF 3".into()), hex: true }
        );
        assert_eq!(
            e.node(&d, &[1]).unwrap().value,
            Value::Enum { raw: 0x0000_FFFF, name: Some("uncompressed".into()), hex: true }
        );
        let cdr = e.node(&d, &[2, 2]).unwrap();
        assert_eq!(cdr.type_name, "CdfDescriptor");
        assert_eq!(e.node(&d, &[2, 2, 1]).unwrap().value, Value::Int(3));
        assert_eq!(e.node(&d, &[2, 2, 10]).unwrap().value, Value::Str("Common Data Format (CDF)".into()));
        // The global descriptor, reached by the offset rather than by lying
        // next to it.
        let gdr = e.node(&d, &[2, 2, 11, 0, 2]).unwrap();
        assert_eq!(gdr.type_name, "CdfGlobalDescriptor");
    }

    #[test]
    fn a_records_body_is_as_long_as_the_record_says() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(cdf());
        let size = e.node(&d, &[2, 0]).unwrap().value.as_int().unwrap();
        assert_eq!(e.node(&d, &[2, 2]).unwrap().size_bits as i128, (size - 12) * 8);
        // The notice runs to the end of the record and nothing says how long
        // it is but the record's own size.
        assert_eq!(e.node(&d, &[2, 2, 10]).unwrap().size_bits, 256 * 8);
    }

    #[test]
    fn a_z_variable_carries_its_own_shape() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(cdf());
        let z = e.node(&d, &[2, 2, 11, 0, 2, 15, 0, 2]).unwrap();
        assert_eq!(z.type_name, "CdfZVariable");
        assert_eq!(z.name, "body sea_temp");
        assert_eq!(e.node(&d, &[2, 2, 11, 0, 2, 15, 0, 2, 1]).unwrap().value.as_int(), Some(45));
        assert_eq!(e.node(&d, &[2, 2, 11, 0, 2, 15, 0, 2, 15]).unwrap().value, Value::Int(1));
        assert_eq!(e.node(&d, &[2, 2, 11, 0, 2, 15, 0, 2, 16, 0]).unwrap().value, Value::Int(3));
    }

    #[test]
    fn an_r_variable_takes_the_shape_the_global_descriptor_declared() {
        // Two dimensions, declared once in the global descriptor, so the
        // rVariable writes two variances and no sizes of its own.
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(cdf());
        let r = e.node(&d, &[2, 2, 11, 0, 2, 14, 0, 2]).unwrap();
        assert_eq!(r.type_name, "CdfRVariable");
        assert_eq!(r.name, "body r_field");
        assert_eq!(e.node(&d, &[2, 2, 11, 0, 2, 14, 0, 2, 15]).unwrap().child_count, 2);
    }

    #[test]
    fn an_attribute_heads_a_chain_of_entries() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(cdf());
        let adr = e.node(&d, &[2, 2, 11, 0, 2, 16, 0, 2]).unwrap();
        assert_eq!(adr.name, "body TITLE");
        assert_eq!(e.node(&d, &[2, 2, 11, 0, 2, 16, 0, 2, 2]).unwrap().value.as_int(), Some(1));
        let entry = e.node(&d, &[2, 2, 11, 0, 2, 16, 0, 2, 12, 0, 2]).unwrap();
        assert_eq!(entry.type_name, "CdfAttributeEntry");
        // Five characters of value, which stay bytes: how to read them is the
        // file's encoding to say, not this record's.
        assert_eq!(e.node(&d, &[2, 2, 11, 0, 2, 16, 0, 2, 12, 0, 2, 10]).unwrap().size_bits, 5 * 8);
    }

    #[test]
    fn every_chain_is_a_flat_list_however_long_it_is() {
        let d = Document::new(MemSource(file()));
        let mut e = Evaluator::new(cdf());
        let gdr = [2, 2, 11, 0, 2];
        let at = |e: &mut Evaluator, i: usize| {
            let mut p = gdr.to_vec();
            p.push(i);
            e.node(&d, &p).unwrap()
        };
        // Each list is one row with its elements under it, rather than a
        // record that holds the next record that holds the next record.
        assert_eq!(at(&mut e, 14).child_count, 1); // rVariables
        assert_eq!(at(&mut e, 15).child_count, 1); // zVariables
        let attrs = at(&mut e, 16);
        assert_eq!(attrs.child_count, 1);
        assert_eq!(attrs.type_name, "chain \u{2192} CdfRecord");
        // The list covers no bytes where it stands; the records it found do.
        assert_eq!(attrs.size_bits, 0);
        // A chain whose head is zero is a list of nothing, not a broken file.
        assert_eq!(at(&mut e, 17).child_count, 0); // the free list
        // The attribute's own entries are a list too.
        assert_eq!(e.node(&d, &[2, 2, 11, 0, 2, 16, 0, 2, 12]).unwrap().child_count, 1);
    }

    #[test]
    fn a_compressed_file_holds_one_record_and_the_settings_that_made_it() {
        let mut cpr = be32(5); // gzip
        cpr.extend(be32(0));
        cpr.extend(be32(1));
        cpr.extend(be32(6)); // level six
        let mut ccr = be64(0); // filled in below
        ccr.extend(be64(4096)); // what it was before
        ccr.extend(be32(0));
        ccr.extend_from_slice(b"\x78\x9c squeezed");
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&[0xCC, 0xCC, 0x00, 0x01]);
        let cpr_at = (8 + 12 + ccr.len()) as i64;
        ccr[0..8].copy_from_slice(&cpr_at.to_be_bytes());
        b.extend(rec(10, ccr));
        b.extend(rec(11, cpr));
        let d = Document::new(MemSource(b));
        let mut e = Evaluator::new(cdf());
        assert_eq!(
            e.node(&d, &[1]).unwrap().value,
            Value::Enum { raw: 0xCCCC_0001, name: Some("compressed".into()), hex: true }
        );
        let body = e.node(&d, &[2, 2]).unwrap();
        assert_eq!(body.type_name, "CdfCompressed");
        assert_eq!(e.node(&d, &[2, 2, 1]).unwrap().value, Value::UInt(4096));
        let parms = e.node(&d, &[2, 2, 4, 0, 2]).unwrap();
        assert_eq!(parms.type_name, "CdfCompressionParameters");
        assert_eq!(
            e.node(&d, &[2, 2, 4, 0, 2, 0]).unwrap().value,
            Value::Enum { raw: 5, name: Some("gzip".into()), hex: false }
        );
        assert_eq!(e.node(&d, &[2, 2, 4, 0, 2, 3, 0]).unwrap().value, Value::Int(6));
    }

    #[test]
    fn a_version_two_file_reads_its_header_and_its_32_bit_offsets() {
        // The GDR sits straight after the CDR: eight bytes of signature, then
        // the CDR's own eight-byte header and its body.
        let gdr_at = 8 + 8 + 4 + 9 * 4 + 5;
        let mut cdr = (gdr_at as u32).to_be_bytes().to_vec();
        cdr.extend(be32(2));
        cdr.extend(be32(7));
        cdr.extend(be32(1));
        cdr.extend(be32(1));
        for _ in 0..5 {
            cdr.extend(be32(0));
        }
        cdr.extend_from_slice(b"NSSDC");
        let mut b = MAGIC_V2.to_vec();
        let mut r = ((cdr.len() + 8) as u32).to_be_bytes().to_vec();
        r.extend(be32(1));
        r.extend(cdr);
        b.extend(r);
        // A global descriptor this does not open, at the offset the CDR gave.
        b.extend(16u32.to_be_bytes());
        b.extend(be32(2));
        b.extend(be64(0));
        let d = Document::new(MemSource(b));
        let mut e = Evaluator::new(cdf());
        assert_eq!(
            e.node(&d, &[0]).unwrap().value,
            Value::Enum { raw: 0x0000_FFFF, name: Some("CDF 2.x".into()), hex: true }
        );
        let cdr = e.node(&d, &[2, 2]).unwrap();
        assert_eq!(cdr.type_name, "Cdf2Descriptor");
        assert_eq!(e.node(&d, &[2, 2, 0]).unwrap().value, Value::UInt(gdr_at as u128));
        assert_eq!(e.node(&d, &[2, 2, 1]).unwrap().value, Value::Int(2));
        assert_eq!(e.node(&d, &[2, 2, 10]).unwrap().value, Value::Str("NSSDC".into()));
        // The record it points at, read as far as its size and its type.
        assert_eq!(e.node(&d, &[2, 2, 11, 0, 0]).unwrap().value, Value::UInt(16));
    }
}
