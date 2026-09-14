//! JPEG 2000: a codestream of markers, on its own or inside the boxes of a
//! JP2 file.
//!
//! The codestream is ISO/IEC 15444-1 Annex A. It opens with SOC and then SIZ,
//! which says how big the image is, how it is cut into tiles and what each
//! component is; the rest of the main header is marker segments in any order,
//! each a marker, a length that counts itself, and parameters. Then come the
//! tile-parts, each opened by SOT, and EOC ends the lot. A tile-part is SOT,
//! its own header of marker segments, SOD, and then the packets of that
//! tile-part with no count on them anywhere but the one SOT gave: `Psot` is
//! how long the whole tile-part is, from the first byte of SOT to the last
//! byte of its data. A `Psot` of zero means the tile-part runs to EOC, which
//! only the last one may say.
//!
//! So a codestream reads as one list, the way a JPEG does: SIZ and the rest of
//! the main header, then one SOT after another, each holding its tile-part,
//! until EOC. See [`codestream`].
//!
//! What reads as fields: every marker segment Part 1 defines. SIZ with each
//! component's depth and sampling, COD and COC down to the precinct sizes,
//! QCD and QCC with a step size for each subband, RGN, POC, TLM with the
//! length of every tile-part, PLM and PLT with the length of every packet,
//! PPM and PPT as far as their chunks, CRG, and COM as text when it says it
//! is text.
//!
//! What stays bytes: the packets. After SOD a tile-part is packet headers and
//! code-block data, and the headers are tag trees and bit-stuffed codes read
//! against everything the main header set up; that is a job for a reader of
//! its own, not for a layout. The SOP and EPH markers inside them are part of
//! those bytes. So are the packed packet headers PPM and PPT carry, and a
//! marker segment this does not read keeps its length and its bytes, which is
//! what the ones Part 2 and Part 15 add do here (CAP, CPF and the rest).
//!
//! The JP2 file format is Annex I: boxes, each a length, four letters and
//! contents, the same shape an MP4 is. The codestream is in the `jp2c` box and
//! reads there as it does on its own. See [`boxes`].
//!
//! One template reads both, since both are what a `.jp2` or a `.j2k` turns out
//! to be and the first two bytes say which: SOC is `FF 4F`, and a JP2 file
//! opens with the length of its signature box, twelve, as four bytes.

use crate::template::{Encoding, Endian::*, Expr as E, StrLen, Template, Ty as T, Until};

mod boxes;

/// Every marker Part 1 names, by the whole two bytes, with the abbreviation
/// the specification uses first since that is what a reader looking at a
/// JPEG 2000 file is looking for. Table A.2, and the two Part 15 adds to the
/// main header of an HTJ2K codestream.
const MARKER: &[(i128, &str)] = &[
    (0xff4f, "SOC, start of codestream"),
    (0xff51, "SIZ, image and tile size"),
    (0xff52, "COD, default coding style"),
    (0xff53, "COC, coding style for one component"),
    (0xff55, "TLM, tile-part lengths"),
    (0xff57, "PLM, packet lengths in the main header"),
    (0xff58, "PLT, packet lengths in a tile-part header"),
    (0xff5c, "QCD, default quantization"),
    (0xff5d, "QCC, quantization for one component"),
    (0xff5e, "RGN, region of interest"),
    (0xff5f, "POC, progression order change"),
    (0xff60, "PPM, packed packet headers in the main header"),
    (0xff61, "PPT, packed packet headers in a tile-part header"),
    (0xff63, "CRG, component registration"),
    (0xff64, "COM, comment"),
    (0xff90, "SOT, start of tile-part"),
    (0xff91, "SOP, start of packet"),
    (0xff92, "EPH, end of packet header"),
    (0xff93, "SOD, start of data"),
    (0xffd9, "EOC, end of codestream"),
    (0xff50, "CAP, extended capabilities"),
    (0xff59, "CPF, corresponding profile"),
];

/// SOC, SOD, EOC and EPH: the markers that are the whole of what they say.
/// Everything else is followed by a length. `FF30` to `FF3F` stand alone as
/// well, reserved by Table A.1 for markers with no parameters, and a reader
/// that gives one of those a length loses its place in everything after it.
const STANDALONE: &[i128] = &[0xff4f, 0xff93, 0xffd9, 0xff92];

/// The reserved markers with no parameters, `FF30` to `FF3F`.
const RESERVED: std::ops::RangeInclusive<i128> = 0xff30..=0xff3f;

/// SOD, which ends a tile-part header.
const SOD: [u8; 2] = [0xff, 0x93];

/// EOC, which ends the codestream.
const EOC: [u8; 2] = [0xff, 0xd9];

/// What the codestream is restricted to, from `Rsiz`. The values Part 1 and
/// its amendments name one at a time; the broadcast and IMF profiles count a
/// level in the low bits, and Part 2 and Part 15 set the top two bits, so
/// those show as the number they are.
const RSIZ: &[(i128, &str)] = &[
    (0, "Part 1, no profile"),
    (1, "profile 0"),
    (2, "profile 1"),
    (3, "2K digital cinema profile"),
    (4, "4K digital cinema profile"),
    (5, "scalable 2K digital cinema profile"),
    (6, "scalable 4K digital cinema profile"),
    (7, "long-term storage profile"),
];

/// The order packets are written in, from Table A.16: which of layer,
/// resolution, component and position the loops run over, outermost first.
const PROGRESSION: &[(i128, &str)] = &[(0, "LRCP"), (1, "RLCP"), (2, "RPCL"), (3, "PCRL"), (4, "CPRL")];

/// The wavelet, from Table A.20.
const TRANSFORMATION: &[(i128, &str)] = &[(0, "9-7 irreversible wavelet"), (1, "5-3 reversible wavelet")];

/// How the step sizes are written, from the low five bits of `Sqcd` or
/// `Sqcc`, Table A.28.
const QUANTIZATION: &[(i128, &str)] = &[(0, "no quantization"), (1, "scalar derived"), (2, "scalar expounded")];

/// The code-block style bits, Table A.19. Bit 6 is Part 15's: the code-blocks
/// are HT coded.
const CODE_BLOCK_STYLE: &[(u32, &str)] = &[
    (0, "selective arithmetic coding bypass"),
    (1, "reset context probabilities"),
    (2, "termination on each coding pass"),
    (3, "vertically causal context"),
    (4, "predictable termination"),
    (5, "segmentation symbols"),
    (6, "high-throughput block coding"),
];

pub fn jpeg2000() -> Template {
    // Two bytes decide which of the two this is. A file too short to hold them
    // is neither, and reads as boxes, which will say so.
    let first = E::cond(E::Remaining.less_than(E::lit(2)), E::lit(0), E::peek(16, Big));
    Template::new("jpeg2000", T::switch(first, vec![(0xff4f, codestream())], boxes::file())).with_type("Box", boxes::jp2_box())
}

/// A codestream: SOC, and then marker segments and tile-parts until EOC.
///
/// Main header and tile-parts are one list rather than two. Nothing in the
/// codestream says where the main header ends except that the next marker is
/// SOT, so two lists would each have to look ahead to stop, and a list of
/// segments that ends at EOC is what the bytes are. The main header is every
/// segment before the first SOT, and each SOT holds its own tile-part header.
///
/// Whatever follows EOC is named as what it is rather than read as markers
/// that are not there.
pub(crate) fn codestream() -> T {
    T::structure(
        "Codestream",
        vec![
            ("SOC", T::magic(&[0xff, 0x4f])),
            ("segments", T::repeat(segment(true), Until::FieldBytes { field: "marker".into(), bytes: EOC.to_vec() })),
            ("trailer", T::bytes(E::Remaining)),
        ],
    )
}

/// One marker and whatever it says. `main` is whether this is the list the
/// codestream itself holds, where SOT opens a tile-part; a tile-part's own
/// header cannot hold another one.
fn segment(main: bool) -> T {
    let mut cases: Vec<(i128, T)> = STANDALONE.iter().copied().chain(RESERVED).map(|m| (m, nothing())).collect();
    cases.extend(vec![
        (0xff51, siz()),
        (0xff52, cod()),
        (0xff53, coc()),
        (0xff55, tlm()),
        (0xff57, plm()),
        (0xff58, plt()),
        (0xff5c, qcd()),
        (0xff5d, qcc()),
        (0xff5e, rgn()),
        (0xff5f, poc()),
        (0xff60, ppm()),
        (0xff61, ppt()),
        (0xff63, crg()),
        (0xff64, com()),
    ]);
    if main {
        cases.push((0xff90, tile_part()));
    }
    let mut names: Vec<(i128, &str)> = MARKER.to_vec();
    names.extend(RESERVED.map(|m| (m, "reserved, no parameters")));
    T::structure_named(
        "Segment",
        "marker",
        "body",
        vec![
            ("marker", T::enumeration_hex("Marker", T::u16(Big), &names)),
            ("body", T::switch(E::field("marker"), cases, unknown())),
        ],
    )
    .counted_as("segment")
}

fn nothing() -> T {
    T::bytes(E::lit(0))
}

/// A marker segment: its parameters, starting with the length, read in a
/// window as long as that length says.
///
/// Every length in a codestream counts itself and not the marker in front of
/// it, so the window starts at the length and is exactly that many bytes. A
/// segment with room left over once its fields are read still ends where its
/// length says, and a list that runs to the end of the segment, such as the
/// packet lengths in a PLT, runs to there. A length of less than two is a
/// codestream gone wrong; the window still takes the two bytes of the length,
/// so the list moves on rather than reading the same bytes for ever.
fn marker_segment(name: &str, fields: Vec<(&str, T)>) -> T {
    T::sized(E::peek(16, Big).at_least(E::lit(2)), T::structure(name, fields))
}

/// A marker this does not name, or one it names and does not read: its length
/// and its bytes.
fn unknown() -> T {
    marker_segment("MarkerSegment", vec![("length", T::u16(Big)), ("parameters", T::bytes(E::Remaining))])
}

/// The number of a component, which is one byte in a codestream of fewer than
/// 257 components and two in one with more. How many there are is SIZ's
/// `Csiz`, and SIZ is the first segment of the main header, so it is always
/// further back in the list than whatever is asking, and a tile-part header
/// asking finds it in the list outside its own.
fn component_number() -> T {
    let few = E::sibling(&["body", "Csiz"]).less_than(E::lit(257));
    T::switch(few, vec![(1, T::u8())], T::u16(Big))
}

/// How many bytes [`component_number`] took, for a segment whose remaining
/// length has to leave it out.
fn component_number_bytes() -> E {
    E::cond(E::sibling(&["body", "Csiz"]).less_than(E::lit(257)), E::lit(1), E::lit(2))
}

/// SIZ: the image, the tiles, and every component.
///
/// The image is the part of the reference grid from `XOsiz` and `YOsiz` to
/// `Xsiz` and `Ysiz`, not from zero, so its size is a difference. The tiles
/// are `XTsiz` by `YTsiz` counted from `XTOsiz` and `YTOsiz`, and how many
/// there are across and down is how many of those it takes to reach the far
/// edge. Both are worked out here beside the numbers they come from.
fn siz() -> T {
    let tiles = |far: &str, offset: &str, size: &str| T::computed(E::field(far).sub(E::field(offset)).div_ceil(E::field(size)));
    marker_segment(
        "SIZ",
        vec![
            ("Lsiz", T::u16(Big)),
            ("Rsiz", T::enumeration_hex("Rsiz", T::u16(Big), RSIZ)),
            ("Xsiz", T::u32(Big)),
            ("Ysiz", T::u32(Big)),
            ("XOsiz", T::u32(Big)),
            ("YOsiz", T::u32(Big)),
            ("XTsiz", T::u32(Big)),
            ("YTsiz", T::u32(Big)),
            ("XTOsiz", T::u32(Big)),
            ("YTOsiz", T::u32(Big)),
            ("Csiz", T::u16(Big)),
            ("components", T::array(component(), E::field("Csiz"))),
            ("width", T::computed(E::field("Xsiz").sub(E::field("XOsiz")))),
            ("height", T::computed(E::field("Ysiz").sub(E::field("YOsiz")))),
            ("tiles_across", tiles("Xsiz", "XTOsiz", "XTsiz")),
            ("tiles_down", tiles("Ysiz", "YTOsiz", "YTsiz")),
        ],
    )
}

/// One component of SIZ: its depth and how far apart its samples are on the
/// reference grid. `Ssiz` packs two numbers into a byte: the top bit says the
/// samples are signed, and the seven below it are the depth less one.
fn component() -> T {
    T::structure(
        "Component",
        vec![
            ("Ssiz", T::u8()),
            ("XRsiz", T::u8()),
            ("YRsiz", T::u8()),
            ("signed", T::computed(E::field("Ssiz").bit(7))),
            ("depth", T::computed(depth(E::field("Ssiz")))),
        ],
    )
    .counted_as("component")
}

/// The bits a sample takes, from a byte written the way `Ssiz` is: the seven
/// low bits are that less one. The JP2 header's `BPC` and a palette's `B` are
/// written the same way.
pub(crate) fn depth(byte: E) -> E {
    byte.and(E::lit(0x7f)).add(E::lit(1))
}

/// COD: how every component is coded, until a COC says otherwise for one.
///
/// `Scod` says whether the precincts have sizes of their own, and whether SOP
/// and EPH markers are written in the packets. The rest is two groups the
/// specification names SGcod and SPcod: the progression, the layers and the
/// colour transform, which hold for every component, and then the parameters
/// a COC can replace, which are laid out by [`coding_parameters`].
fn cod() -> T {
    let mut fields = vec![
        ("Lcod", T::u16(Big)),
        (
            "Scod",
            T::flags("Scod", T::u8(), &[(0, "precinct sizes defined"), (1, "SOP markers may be used"), (2, "EPH markers used")]),
        ),
        ("progression_order", T::enumeration("ProgressionOrder", T::u8(), PROGRESSION)),
        ("layers", T::u16(Big)),
        (
            "multiple_component_transform",
            T::enumeration("MultipleComponentTransform", T::u8(), &[(0, "none"), (1, "on components 0, 1 and 2")]),
        ),
    ];
    fields.extend(coding_parameters("Scod"));
    marker_segment("COD", fields)
}

/// COC: how one component is coded, in place of what COD said.
fn coc() -> T {
    let mut fields = vec![
        ("Lcoc", T::u16(Big)),
        ("Ccoc", component_number()),
        ("Scoc", T::flags("Scoc", T::u8(), &[(0, "precinct sizes defined")])),
    ];
    fields.extend(coding_parameters("Scoc"));
    marker_segment("COC", fields)
}

/// What COD calls SPcod and COC calls SPcoc: the wavelet, how many times it
/// is applied, the code-blocks, and the precinct sizes.
///
/// A code-block is `2^(xcb_minus_2 + 2)` samples wide, which is how Table A.18
/// writes it: the byte is the exponent less two. There is a precinct size for
/// each resolution, one more than the decomposition levels, but only when the
/// style byte named by `style` says precincts are defined; otherwise every
/// precinct is as big as it can be and nothing is written.
fn coding_parameters(style: &str) -> Vec<(&'static str, T)> {
    vec![
        ("decomposition_levels", T::u8()),
        ("xcb_minus_2", T::u8()),
        ("ycb_minus_2", T::u8()),
        ("code_block_style", T::flags("CodeBlockStyle", T::u8(), CODE_BLOCK_STYLE)),
        ("transformation", T::enumeration("Transformation", T::u8(), TRANSFORMATION)),
        ("precinct_sizes", T::when(E::field(style).bit(0), T::array(precinct_size(), E::field("decomposition_levels").add(E::lit(1))))),
        ("code_block_width", T::computed(E::lit(1).shl(E::field("xcb_minus_2").add(E::lit(2))))),
        ("code_block_height", T::computed(E::lit(1).shl(E::field("ycb_minus_2").add(E::lit(2))))),
    ]
}

/// One precinct size, for one resolution: the height exponent in the top four
/// bits and the width exponent in the bottom four, so a byte of `0x77` is a
/// precinct of 128 by 128.
fn precinct_size() -> T {
    T::inline_structure("PrecinctSize", vec![("PPy", T::UInt { bits: 4, endian: Big }), ("PPx", T::UInt { bits: 4, endian: Big })])
}

/// QCD: the step sizes every component is quantized with, until a QCC says
/// otherwise for one.
fn qcd() -> T {
    let mut fields = vec![("Lqcd", T::u16(Big))];
    fields.extend(quantization("Sqcd", "SPqcd", E::field("Lqcd").sub(E::lit(3))));
    marker_segment("QCD", fields)
}

/// QCC: the step sizes for one component.
fn qcc() -> T {
    let mut fields = vec![("Lqcc", T::u16(Big)), ("Cqcc", component_number())];
    fields.extend(quantization("Sqcc", "SPqcc", E::field("Lqcc").sub(E::lit(3)).sub(component_number_bytes())));
    marker_segment("QCC", fields)
}

/// The style byte and the step sizes, which QCD and QCC share. `bytes` is how
/// many bytes of step sizes the segment's length leaves.
///
/// The style byte is two numbers: guard bits in the top three and the style in
/// the bottom five. With no quantization each subband has an exponent, a byte
/// each; scalar expounded gives each subband an exponent and a mantissa in two
/// bytes; scalar derived writes one of those, for the lowest subband, and
/// works the rest out from it.
///
/// A step size belongs to a subband, and there are three for every
/// decomposition level and one more, so the list is `3 * NL + 1` long. `NL`
/// is COD's to say, and it is not taken from there: COD may come after QCD in
/// the main header, a COC can give one component other levels, and a
/// tile-part header can change both. What the segment's own length leaves
/// does say it, for certain, and so the levels are worked out from that
/// instead, which is what a decoder does too. `decomposition_levels` is that
/// number, beside the list whose length says it.
fn quantization(style: &'static str, steps: &'static str, bytes: E) -> Vec<(&'static str, T)> {
    let exponent = T::inline_structure(
        "Exponent",
        vec![("exponent", T::UInt { bits: 5, endian: Big }), ("reserved", T::UInt { bits: 3, endian: Big })],
    );
    let step = || {
        T::inline_structure(
            "StepSize",
            vec![("exponent", T::UInt { bits: 5, endian: Big }), ("mantissa", T::UInt { bits: 11, endian: Big })],
        )
    };
    let subbands = E::cond(E::field("quantization_style").equal_to(E::lit(2)), bytes.clone().div(E::lit(2)), bytes);
    vec![
        (style, T::u8()),
        ("guard_bits", T::computed(E::field(style).shr(E::lit(5)))),
        ("quantization_style", T::enumeration("QuantizationStyle", T::computed(E::field(style).and(E::lit(0x1f))), QUANTIZATION)),
        (
            "decomposition_levels",
            T::when(E::field("quantization_style").not_equal(E::lit(1)), T::computed(subbands.clone().sub(E::lit(1)).div(E::lit(3)))),
        ),
        (
            steps,
            T::switch(
                E::field("quantization_style"),
                vec![
                    (0, T::array(exponent, E::field("decomposition_levels").mul(E::lit(3)).add(E::lit(1)))),
                    (1, step()),
                    (2, T::array(step(), E::field("decomposition_levels").mul(E::lit(3)).add(E::lit(1)))),
                ],
                T::bytes(E::Remaining),
            ),
        ),
    ]
}

/// RGN: a region of interest in one component, as how many bit-planes its
/// coefficients are shifted up by. Style 0 is the only one Part 1 has, the
/// implicit maximum shift.
fn rgn() -> T {
    marker_segment(
        "RGN",
        vec![
            ("Lrgn", T::u16(Big)),
            ("Crgn", component_number()),
            ("Srgn", T::enumeration("RoiStyle", T::u8(), &[(0, "implicit")])),
            ("SPrgn", T::u8()),
        ],
    )
}

/// POC: progression changes, each a box of layers, resolutions and
/// components to write in an order of its own. The ends are exclusive, and
/// with fewer than 257 components a `CEpoc` of 0 means 256.
fn poc() -> T {
    let change = T::structure(
        "ProgressionChange",
        vec![
            ("RSpoc", T::u8()),
            ("CSpoc", component_number()),
            ("LYEpoc", T::u16(Big)),
            ("REpoc", T::u8()),
            ("CEpoc", component_number()),
            ("Ppoc", T::enumeration("ProgressionOrder", T::u8(), PROGRESSION)),
        ],
    )
    .counted_as("change");
    marker_segment("POC", vec![("Lpoc", T::u16(Big)), ("changes", T::repeat(change, Until::End))])
}

/// TLM: the length of every tile-part, so a reader can reach a tile without
/// walking the ones before it.
///
/// `Stlm` says how wide the two numbers of each entry are. Two bits give the
/// width of the tile number, `ST`, in bytes, where zero means no tile number
/// is written and the tile-parts are one to a tile and in order; one bit says
/// the length is four bytes rather than two. Each entry's `Ptlm` is the `Psot`
/// of a tile-part.
fn tlm() -> T {
    let entry = T::inline_structure(
        "TilePartLength",
        vec![
            ("Ttlm", T::when(E::field("ST").not_equal(E::lit(0)), T::uint_expr(E::field("ST").mul(E::lit(8)), Big))),
            ("Ptlm", T::uint_expr(E::field("SP").add(E::lit(1)).mul(E::lit(16)), Big)),
        ],
    );
    let each = E::field("ST").add(E::field("SP").add(E::lit(1)).mul(E::lit(2)));
    marker_segment(
        "TLM",
        vec![
            ("Ltlm", T::u16(Big)),
            ("Ztlm", T::u8()),
            ("Stlm", T::u8()),
            ("ST", T::computed(E::bit_field(E::field("Stlm"), 5, 2))),
            ("SP", T::computed(E::field("Stlm").bit(6))),
            ("tile_parts", T::array(entry, E::field("Ltlm").sub(E::lit(4)).div(each))),
        ],
    )
}

/// PLT: the length of every packet in the tile-part, in the order they are
/// written. Each length is seven bits a byte, most significant first, with
/// the top bit set on every byte but the last, which is how MIDI writes a
/// delta time. `Zplt` numbers the PLT segments of one tile-part header.
fn plt() -> T {
    marker_segment("PLT", vec![("Lplt", T::u16(Big)), ("Zplt", T::u8()), ("Iplt", T::repeat(T::vlq(), Until::End))])
}

/// PLM: packet lengths for every tile-part, gathered in the main header. Each
/// tile-part's share is `Nplm` bytes of lengths written as PLT writes them.
///
/// One tile-part's share may be split across two PLM segments, and the second
/// then goes on without an `Nplm` of its own. That is not followed here: the
/// second segment's first byte reads as an `Nplm`, and its lengths from there
/// on are wrong.
fn plm() -> T {
    let part = T::structure(
        "PacketLengths",
        vec![("Nplm", T::u8()), ("Iplm", T::sized(E::field("Nplm"), T::repeat(T::vlq(), Until::End)))],
    )
    .counted_as("tile-part");
    marker_segment("PLM", vec![("Lplm", T::u16(Big)), ("Zplm", T::u8()), ("tile_parts", T::repeat(part, Until::End))])
}

/// PPM: the packet headers of every tile-part, taken out of the packets and
/// gathered in the main header. Each tile-part's share is a length and that
/// many bytes, and the headers inside stay bytes. A share may be split across
/// PPM segments, as a PLM's may, and is not followed here either: the share
/// is cut at the end of its segment, and the next segment reads the rest of it
/// as an `Nppm`.
fn ppm() -> T {
    let part = T::structure("PackedHeaders", vec![("Nppm", T::u32(Big)), ("Ippm", T::bytes(E::field("Nppm").at_most(E::Remaining)))])
        .counted_as("tile-part");
    marker_segment("PPM", vec![("Lppm", T::u16(Big)), ("Zppm", T::u8()), ("tile_parts", T::repeat(part, Until::End))])
}

/// PPT: the packet headers of one tile-part, taken out of its packets. The
/// headers stay bytes.
fn ppt() -> T {
    marker_segment("PPT", vec![("Lppt", T::u16(Big)), ("Zppt", T::u8()), ("Ippt", T::bytes(E::Remaining))])
}

/// CRG: where each component sits on the reference grid, in 65536ths of a
/// sample, one pair for each component.
fn crg() -> T {
    let offset = T::inline_structure("Registration", vec![("Xcrg", T::u16(Big)), ("Ycrg", T::u16(Big))]);
    marker_segment("CRG", vec![("Lcrg", T::u16(Big)), ("offsets", T::array(offset, E::field("Lcrg").sub(E::lit(2)).div(E::lit(4))))])
}

/// COM: a comment, which `Rcom` says is either bytes or ISO/IEC 8859-15 text.
/// That differs from Latin-1 in eight characters, the euro sign among them,
/// and is read as Latin-1, which is the nearest this has.
fn com() -> T {
    marker_segment(
        "COM",
        vec![
            ("Lcom", T::u16(Big)),
            ("Rcom", T::enumeration("CommentRegistration", T::u16(Big), &[(0, "binary data"), (1, "ISO 8859-15 text")])),
            (
                "Ccom",
                T::switch(E::field("Rcom"), vec![(1, T::text(StrLen::Fixed(E::Remaining), Encoding::Latin1))], T::bytes(E::Remaining)),
            ),
        ],
    )
}

/// SOT and the tile-part it opens: SOT's own fields, the tile-part header up
/// to and including SOD, and the packets.
///
/// `Psot` counts from the first byte of the SOT marker, which the list read
/// before this, to the end of the packets, so what is left for the packets is
/// that less the marker, SOT's parameters and the header. When `Psot` is zero
/// the tile-part runs to EOC instead. EOC cannot be in packet data: an
/// arithmetic coder writing a byte of `FF` keeps the next one below `90`, and
/// so do the packet headers, so the first `FF D9` is the end. A codestream cut
/// off before its EOC runs to the end of what there is.
///
/// `TNsot` is how many tile-parts the tile has, where the encoder said, and
/// zero where it did not.
fn tile_part() -> T {
    let data = E::field("Psot").sub(E::lit(2)).sub(E::field("Lsot")).sub(E::size_of("header"));
    T::structure(
        "TilePart",
        vec![
            ("Lsot", T::u16(Big)),
            ("Isot", T::u16(Big)),
            ("Psot", T::u32(Big)),
            ("TPsot", T::u8()),
            ("TNsot", T::u8()),
            ("header", T::repeat(segment(false), Until::FieldBytes { field: "marker".into(), bytes: SOD.to_vec() })),
            (
                "data",
                T::switch(E::field("Psot"), vec![(0, T::bytes(E::to_bytes(&EOC)))], T::bytes(data.at_least(E::lit(0)).at_most(E::Remaining))),
            ),
        ],
    )
}

#[cfg(test)]
mod tests;
