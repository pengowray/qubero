//! xz: a stream header, some blocks, an index of the blocks, and a footer.
//!
//! The footer is what makes the rest findable. Its backward size says how
//! long the index is, in units of four bytes, so the index can be placed from
//! the end of the file without reading anything in between, and the blocks
//! are then everything between the header and the index. That is the one
//! measurement this format needs which no field in front of it gives.
//!
//! The index is the part worth having. It holds, for every block, how many
//! bytes it took and how many it produced, which is the whole shape of the
//! file without decompressing any of it.
//!
//! A block's own header says how long the compressed data is only when the
//! encoder chose to write it, which the `xz` tool does not. Without it a block
//! runs to the end of the block region, which is right for the one-block files
//! almost every `.xz` is and stops short of splitting a multi-block file the
//! index has already described.
//!
//! A block header's filter chain is read. It is the list of what the data
//! went through on its way in, innermost last: one entry per filter, each an
//! identifier, a length, and that many bytes of properties. What those
//! properties mean is the filter's own business, so each named filter reads
//! its own: LZMA2 spends its one byte on a dictionary size, delta spends its
//! one on a distance, and a branch filter has either four bytes of start
//! offset or none at all.
//!
//! Not read: the compressed data.

use crate::codec::Codec;
use crate::template::{Check, Checksum, Covers, Endian::{Big, Little}, Expr as E, Part, Template, Ty as T};

/// What one of these starts with.
pub const MAGIC: &[u8] = b"\xfd7zXZ\x00";

/// The two bytes at the very end.
const FOOTER_MAGIC: &[u8] = b"YZ";

/// What the integrity check at the end of every block is. Four of the sixteen
/// IDs are assigned and the rest are reserved, and a reserved one is left
/// unnamed here on purpose: the size of a check follows from its ID whatever
/// it is called (see [`check_size`]), but a name does not, and a template that
/// invented one would be saying which sum a file holds when nothing has said
/// that yet.
const CHECKS: &[(i128, &str)] = &[(0, "none"), (1, "crc32"), (4, "crc64"), (10, "sha256")];

/// The filters a block's chain may name. 0x21 is the compressor; the rest are
/// transforms in front of it, which do not make the data smaller themselves
/// and make it more compressible for whatever comes after them.
///
/// The branch filters, 0x04 through 0x0b, turn the relative addresses of an
/// executable's branch and call instructions into absolute ones, so that the
/// same call written in twenty places becomes the same bytes twenty times.
/// Each is one instruction set: 0x0a and 0x0b arrived long after the format
/// did, with xz 5.4 and 5.6, so an older decoder handed a stream using either
/// refuses it as a filter it does not know.
const FILTERS: &[(i128, &str)] = &[
    (0x03, "delta"),
    (0x04, "x86"),
    (0x05, "powerpc"),
    (0x06, "ia64"),
    (0x07, "arm"),
    (0x08, "armthumb"),
    (0x09, "sparc"),
    (0x0a, "arm64"),
    (0x0b, "riscv"),
    (0x21, "lzma2"),
];

/// How long the index is: the footer's backward size, eight bytes from the end
/// of the file, counted in units of four bytes and written one short. The
/// distance is in bits, and negative, which is what reads from the end.
fn index_size() -> E {
    E::peek_at(E::lit(-8 * 8), 32, Little).add(E::lit(1)).mul(E::lit(4))
}

/// A CRC-32 of bytes of the file, which is what all four of xz's own sums are.
/// The data's own check is a different thing and is not one of these: it is
/// over what a block unpacks to. See [`block_check`].
fn crc32_over(at: E, len: E) -> Check {
    Check::of(Checksum::Crc32, Covers::Run { at, len })
}

/// A block's own integrity check: `algorithm` over the bytes *this* block
/// unpacked to, taken when the stream header's check type is `id`.
///
/// Every part of that sentence is doing something. The bytes are nowhere in
/// the file, so the sum is over what the stream comes to rather than over a
/// run of it; and it is over one block's share of that, which is why it is
/// [`Covers::UnpackedMember`] and not [`Covers::Unpacked`]. The share cannot
/// be written as an expression: a block's bytes begin at the sum of every
/// earlier block's uncompressed size, and those sizes are variable-length
/// integers in the index at the far end of the file. So the decoder is asked,
/// which is the one thing that walked the blocks.
///
/// Which block is asked about is said by pointing at its packed bytes rather
/// than by counting: `compressed` is this block's own data, and the member
/// the decoder read from exactly there is this block's. That matters here
/// more than anywhere. A block header carries the size of its data only if
/// the encoder wrote it, and `xz -T1` does not, so a multi-block stream reads
/// as one block whose data runs to the last check in the region (see the
/// module doc). Counted, that block would be sealed with the last block's
/// number over the first block's bytes and a good file would report itself
/// broken; matched on where the bytes are, it says the check cannot be made.
///
/// The stream is what gets opened, not the block. A block is a run of its own
/// and could in principle be unpacked alone, but nothing here does: the codec
/// reads a stream, and the field that holds the answer is the stream's
/// `decoded`.
fn block_check(id: i128, algorithm: Checksum) -> Check {
    Check::of(
        algorithm,
        Covers::UnpackedMember {
            name: crate::template::Named::here("decoded"),
            packed: crate::template::Named::here("compressed"),
        },
    )
    .only_when(E::field("check_type").equals(E::lit(id)))
}

pub fn xz() -> Template {
    Template::new("xz", part(super::decoded_text()).root)
}

/// The same stream, for a format that carries one inside itself. A ROOT record
/// compressed with `XZ` is a nine-byte block header and then a whole xz stream,
/// footer and all, which is what makes the index at the end of it findable.
///
/// `inner` is what the blocks turn out to hold, which is the caller's business:
/// a file of its own holds text, and a ROOT record holds an object.
pub fn part(inner: T) -> Part {
    Part::new(
        T::structure(
            "XzStream",
            vec![
                ("magic", T::magic(MAGIC)),
                ("stream_flags_reserved", T::u8()),
                ("stream_flags_check_reserved", T::UInt { bits: 4, endian: Big }),
                // Which check every block ends with, which is also what says
                // how long that check is.
                ("check_type", T::enumeration("XzCheck", T::UInt { bits: 4, endian: Big }, CHECKS)),
                ("stream_flags_crc32", T::u32(Little)),
                // The length of that check, as a number the blocks can be
                // measured against rather than as bytes anybody wrote.
                ("check_size", check_size()),
                // Everything between the header and the index.
                (
                    "blocks",
                    T::sized(
                        E::Remaining.sub(index_size()).sub(E::lit(12)),
                        T::repeat(block(), crate::template::Until::End),
                    ),
                ),
                ("index", T::sized(index_size(), index())),
                ("footer", footer()),
            // What the whole stream comes to. Nothing in it can be opened on
            // its own: a block is a step of a decoder's state and not a run
            // that stands by itself, so the field that holds the answer is the
            // stream. It costs no bytes where it stands and covers the stream
            // from its first byte, which is the file when the stream is the
            // file and the block payload when a ROOT record carries one.
                ("decoded", T::at_in_window(E::lit(0), T::decoded(E::Remaining, Codec::Xz, inner))),

            ],
        )
        // The two flag bytes after the magic, and nothing else. The second of
        // the two is split into two four-bit fields here, so the length is
        // written down rather than measured off fields that round to nothing.
        .field_check("stream_flags_crc32", crc32_over(E::size_of("magic"), E::lit(2))),
    )
}

/// How many bytes the stream's check takes after every block. A field of no
/// bits: the number is in the header's check type, and the blocks need it as
/// a length.
///
/// All sixteen IDs have a size, not just the four with names. After 0, which
/// is no check at all, they run in groups of three that double: 1 to 3 are
/// four bytes, 4 to 6 are eight, 7 to 9 are sixteen, 10 to 12 are thirty-two,
/// and 13 to 15 are sixty-four.
///
/// A reserved ID is reserved, not unknown, and that distinction is the whole
/// point of this. Nobody can take the sum in a stream whose check type is 2,
/// but everybody knows it is four bytes long, and those four bytes are in the
/// file after every block. Sized as none, the block padding after them, the
/// next block, and the index are all four bytes out, and a stream that reads
/// as rubbish looks like a broken file rather than an unsupported one. That is
/// why the size is worked out from the ID and the name is not: an unassigned
/// ID has a length nobody has to guess at, and no name anybody may invent.
fn check_size() -> T {
    T::switch(
        E::field("check_type"),
        vec![(0, T::computed(E::lit(0)))],
        // The groups of three, as arithmetic: four bytes doubled once per
        // group above the first. Zero is switched out above rather than
        // written into this, since it is the one ID with no group of its own.
        T::computed(E::lit(4).shl(E::field("check_type").sub(E::lit(1)).div(E::lit(3)))),
    )
}

/// One block: a header whose first byte says how long it is, the compressed
/// data, padding to a multiple of four, and the check.
fn block() -> T {
    T::structure(
        "XzBlock",
        vec![
            // The header's length in units of four bytes, written one short.
            ("header_size", T::u8()),
            ("uncompressed_size_present", T::UInt { bits: 1, endian: Big }),
            ("compressed_size_present", T::UInt { bits: 1, endian: Big }),
            ("flags_reserved", T::UInt { bits: 4, endian: Big }),
            // How many filters the data went through, written one short.
            ("filter_count", T::UInt { bits: 2, endian: Big }),
            ("compressed_size", T::present_if(E::field("compressed_size_present"), T::leb_u())),
            ("uncompressed_size", T::present_if(E::field("uncompressed_size_present"), T::leb_u())),
            // The filter chain and the padding after it, which fill the
            // header out to the length its first byte gave.
            //
            // The padding is inside this window rather than beside it because
            // the window is what stops a bad chain: a filter is as long as its
            // own `properties_size` says, and nothing checks that number
            // against the header it is written in. Bounded here, the worst a
            // header claiming a filter with two hundred bytes of properties
            // can do is eat its own padding; unbounded, it would read the
            // CRC32 and the compressed data after it as properties, and every
            // field of the block after that would be somewhere else.
            (
                "filter_flags",
                T::sized(
                    chain_size(),
                    T::structure(
                        "XzFilterFlags",
                        vec![
                            // One more filter than the count says. A header
                            // can always claim four of them, since the count
                            // is two bits wide and nothing has checked it
                            // against the header it sits in, so a claimed
                            // filter with no room left is a row with nothing
                            // in it rather than a failure that would take the
                            // filters that are there down with it. See
                            // `filter`, where the room is asked about.
                            ("filters", T::array(filter(), E::field("filter_count").add(E::lit(1)))),
                            // Zero bytes out to the length the header gave.
                            ("header_padding", T::bytes(E::Remaining)),
                        ],
                    ),
                ),
            ),
            ("header_crc32", T::u32(Little)),
            // The compressed data. Its length is in the header only when the
            // encoder wrote it there; without it, everything left in the block
            // region but the check, which takes the block's own padding in
            // with the data since nothing left says where one ends.
            (
                "compressed",
                T::bytes(E::field("compressed_size").or(E::Remaining.sub(E::field("check_size")).at_least(E::lit(0)))),
            ),
            ("block_padding", T::bytes(E::size_of("compressed").pad_to(4))),
            // The check, as long as `check_size` says. The two sums that fit
            // in a word read as numbers, which is what a reader comparing one
            // against a checksum of their own wants to see. Everything else
            // reads as its bytes: a SHA-256 is not a number anybody reads, and
            // a reserved ID is a run of bytes nobody can take the sum of but
            // everybody has to step over.
            (
                "check",
                T::switch(
                    E::field("check_type"),
                    vec![(1, T::u32(Little)), (4, T::u64(Little))],
                    T::bytes(E::field("check_size")),
                ),
            ),
        ],
    )
    .counted_as("block")
    // The block header, from its first byte to the sum itself. The first byte
    // gives the header's whole length in units of four, written one short, and
    // the four bytes of the sum come off the end of that.
    .field_check(
        "header_crc32",
        crc32_over(E::lit(0), E::field("header_size").add(E::lit(1)).mul(E::lit(4)).sub(E::lit(4))),
    )
    // And the check over the data, which is the only sum in an xz file that is
    // about the file's own contents rather than about the container around
    // them. Three of them, one per check type the format has assigned, and the
    // guards are what pick: the field holds one number and is compared against
    // one algorithm's answer.
    //
    // Two IDs are left out and neither is a failure. Check type 0 is no check,
    // so there is nothing written down to compare against and nothing to say.
    // The eleven reserved IDs have a size, which is why the bytes are still
    // stepped over correctly (see `check_size`), and no algorithm: a stream
    // using one was written by something that knows a sum this does not, and
    // the honest answer is that the check is not made rather than that it
    // failed.
    .field_check("check", block_check(1, Checksum::Crc32))
    .field_check("check", block_check(4, Checksum::Crc64Xz))
    .field_check("check", block_check(10, Checksum::Sha256))
}

/// How much of the block header the filter chain and its padding have to fill:
/// what the first byte said the header comes to, less that byte and the flags
/// byte, less whichever of the two sizes were written, less the four bytes of
/// CRC32 at the end. The six is those two bytes and those four.
///
/// Clamped at both ends. Nothing has checked `header_size` against anything by
/// the time it is used here, so a header claiming more than the file holds is
/// cut to what is left of the block region, less the CRC32 that has to come
/// out of it; a header claiming less than the fields already read is nothing at
/// all rather than a negative length. Either way the number it claimed is one
/// row above, so a reader can see what happened.
fn chain_size() -> E {
    E::field("header_size")
        .add(E::lit(1))
        .mul(E::lit(4))
        .sub(E::lit(6))
        .sub(E::size_of("compressed_size"))
        .sub(E::size_of("uncompressed_size"))
        .at_least(E::lit(0))
        .at_most(E::Remaining.sub(E::lit(4)).at_least(E::lit(0)))
}

/// One filter of the chain: which filter, how many bytes of settings it wrote,
/// and those bytes.
///
/// The order is the order the data went through on the way in, so the last
/// filter is the one that produced the bytes in the block and the first is the
/// one that saw the file. A chain is at most four long and in practice is one
/// or two: LZMA2 on its own, or a transform and then LZMA2.
///
/// Every one of the three asks whether there is room for it, because the count
/// that says how many of these there are is two bits wide and the header it is
/// written in may be twelve bytes long. A filter the header has no room for is
/// three empty rows, which is a truthful reading of a header that claimed one
/// and did not write it; the alternative is a list that will not be read, and
/// that would hide the filters the header did write.
fn filter() -> T {
    T::structure(
        "XzFilter",
        vec![
            ("filter_id", T::if_room(T::enumeration_hex("XzFilterId", T::leb_u(), FILTERS))),
            ("properties_size", T::if_room(T::leb_u())),
            // Kept inside what is left of the header, so that a length longer
            // than the header it sits in reads as far as the header goes
            // rather than into whatever is written after it.
            ("properties", T::sized(E::field("properties_size").at_most(E::Remaining), properties())),
        ],
    )
    .counted_as("filter")
}

/// What a filter's properties hold, which is the filter's own business: the
/// list of filter flags says how many bytes there are and nothing else about
/// them.
///
/// A filter nobody here names keeps its bytes as bytes. That is not a gap in
/// the reading: the length was written down by the encoder, so the chain still
/// walks past it correctly, and what is missing is only what the numbers mean.
fn properties() -> T {
    T::switch(
        E::field("filter_id"),
        vec![
            (0x03, delta()),
            (0x04, bcj()),
            (0x05, bcj()),
            (0x06, bcj()),
            (0x07, bcj()),
            (0x08, bcj()),
            (0x09, bcj()),
            (0x0a, bcj()),
            (0x0b, bcj()),
            (0x21, lzma2()),
        ],
        T::bytes(E::Remaining),
    )
}

/// The delta filter's one byte: how far back to look for the byte that each
/// byte of the data is stored as a difference from. Four, for a file of 32-bit
/// samples, turns a slowly changing waveform into small numbers.
fn delta() -> T {
    T::structure(
        "XzDeltaProperties",
        // Written one short, since a distance of nothing would be a filter
        // that does nothing: 0 is one byte back and 255 is 256 bytes back.
        // Named for what is written rather than for the distance, so that the
        // row cannot be read as the distance itself.
        vec![("distance_minus_one", T::u8())],
    )
}

/// A branch filter's properties: four bytes of start offset, or nothing.
///
/// The offset is what address the first byte of the data is at, which the
/// filter needs to turn a relative branch into an absolute address. Nothing
/// written means zero, which is what an encoder handed a whole executable
/// writes, and it is why a properties length of zero is ordinary here and
/// nowhere else in the chain.
fn bcj() -> T {
    T::switch(
        E::field("properties_size"),
        vec![(4, T::structure("XzBcjProperties", vec![("start_offset", T::u32(Little))]))],
        T::bytes(E::Remaining),
    )
}

/// LZMA2's one byte: two bits nobody has used yet, and six that say how large
/// a dictionary the decoder has to allocate.
fn lzma2() -> T {
    T::structure(
        "XzLzma2Properties",
        vec![
            ("properties_reserved", T::UInt { bits: 2, endian: Big }),
            // Not a size. See `dictionary_bytes` below for what it encodes.
            ("dictionary_size_code", T::UInt { bits: 6, endian: Big }),
            ("dictionary_bytes", dictionary_bytes()),
        ],
    )
}

/// What the dictionary size code comes to, as a number of bytes. A field of no
/// bits, the way `check_size` is: the six bits above hold a code and a reader
/// wants a size.
///
/// The code is a one-bit mantissa and a five-bit exponent: the bottom bit
/// chooses 2 or 3, the rest is a shift, and the two multiply out to a size
/// between 4 KiB at code 0 and 3 GiB at code 39, stepping by halves of a
/// power of two. Code 40 is the exception and means the largest dictionary
/// there is, one byte short of 4 GiB, which the arithmetic cannot reach.
///
/// Above 40 there is no size. The encoding runs out, and liblzma refuses such
/// a stream rather than allocating something; so this field is empty there
/// instead of holding what the shift would have produced, which would be a
/// number no decoder would ever agree with. The code itself is still shown,
/// which is the whole of what the file said.
fn dictionary_bytes() -> T {
    let code = E::field("dictionary_size_code");
    let size = E::lit(2).add(code.clone().and(E::lit(1))).shl(code.clone().div(E::lit(2)).add(E::lit(11)));
    T::switch(
        // Every code above 40 folded onto 41, so that one case covers them.
        code.at_most(E::lit(41)),
        vec![(40, T::computed(E::lit(0xffff_ffffi64))), (41, T::bytes(E::lit(0)))],
        T::computed(size),
    )
}

/// The index: one record per block, saying what the block cost and what it
/// held. This is the file's shape, and it is here rather than in the blocks
/// so that a decoder can seek without reading them.
fn index() -> T {
    T::structure(
        "XzIndex",
        vec![
            ("indicator", T::magic(&[0])),
            ("record_count", T::leb_u()),
            (
                "records",
                T::array(
                    T::inline_structure(
                        "XzIndexRecord",
                        vec![
                            // The block without its padding: header and data
                            // and check.
                            ("unpadded_size", T::leb_u()),
                            ("uncompressed_size", T::leb_u()),
                        ],
                    )
                    .counted_as("record"),
                    E::field("record_count"),
                ),
            ),
            ("index_padding", T::bytes(E::Remaining.sub(E::lit(4)).at_least(E::lit(0)))),
            ("index_crc32", T::u32(Little)),
        ],
    )
    // Everything in the index but the sum. Measured off the fields rather than
    // off the window they sit in, since what the window comes to is worked out
    // from the far end of the file and means nothing from in here.
    .field_check(
        "index_crc32",
        crc32_over(
            E::lit(0),
            E::size_of("indicator")
                .add(E::size_of("record_count"))
                .add(E::size_of("records"))
                .add(E::size_of("index_padding")),
        ),
    )
}

/// The last twelve bytes, which say how long the index is and repeat the
/// stream flags so that a reader working backwards knows the check as well.
fn footer() -> T {
    T::structure(
        "XzFooter",
        vec![
            ("footer_crc32", T::u32(Little)),
            ("backward_size", T::u32(Little)),
            ("stream_flags", T::u16(Big)),
            ("magic", T::magic(FOOTER_MAGIC)),
        ],
    )
    // The two fields after it, which is the one sum here that covers bytes
    // written later than itself rather than earlier.
    .field_check(
        "footer_crc32",
        crc32_over(E::size_of("footer_crc32"), E::size_of("backward_size").add(E::size_of("stream_flags"))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::xz::tests as xz_codec;
    use crate::{checksum::crc32, document::Document, eval::{EvalError, Evaluator, Value}, source::MemSource};

    /// A block header of twelve bytes: one LZMA2 filter with the smallest
    /// dictionary there is, three bytes of padding, and four zero bytes where
    /// the builder puts the CRC32.
    const LZMA2_HEADER: &[u8] = &[0x02, 0x00, 0x21, 0x01, 0x00, 0, 0, 0, 0, 0, 0, 0];

    /// A stream of one block whose header does not say how long its data is,
    /// which is what `xz` writes.
    fn stream(data: &[u8]) -> Vec<u8> {
        stream_of(1, 4, LZMA2_HEADER, data)
    }

    /// The same, with the check named and as long as that check is, and with
    /// the block header given rather than assumed. Every CRC32 in it is worked
    /// out here, so the only thing wrong with what comes back is that the
    /// compressed data is not compressed data.
    ///
    /// The last four bytes of `header` are overwritten with its own CRC32,
    /// which is why a caller writes zeros there.
    fn stream_of(check_type: u8, check_len: usize, header: &[u8], data: &[u8]) -> Vec<u8> {
        let flags = [0x00, check_type];
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&flags);
        v.extend_from_slice(&crc32(&flags).to_le_bytes());

        let mut header = header.to_vec();
        let up_to_crc = header.len() - 4;
        let crc = crc32(&header[..up_to_crc]);
        header[up_to_crc..].copy_from_slice(&crc.to_le_bytes());
        let block_start = v.len();
        v.extend_from_slice(&header);
        v.extend_from_slice(data);
        while (v.len() - block_start) % 4 != 0 {
            v.push(0);
        }
        v.extend_from_slice(&vec![0u8; check_len]);
        // The size of the block without its padding: header, data and check.
        let unpadded = (header.len() + data.len() + check_len) as u8;

        let index_start = v.len();
        v.extend_from_slice(&[0x00, 0x01, unpadded, data.len() as u8]);
        while (v.len() - index_start) % 4 != 0 {
            v.push(0);
        }
        v.extend_from_slice(&crc32(&v[index_start..]).to_le_bytes());
        let index_size = v.len() - index_start;

        let mut footer = ((index_size / 4 - 1) as u32).to_le_bytes().to_vec();
        footer.extend_from_slice(&flags);
        v.extend_from_slice(&crc32(&footer).to_le_bytes());
        v.extend_from_slice(&footer);
        v.extend_from_slice(FOOTER_MAGIC);
        v
    }

    /// The path of the field called `name` under `path`. By name, so that a
    /// template that grows a field does not renumber every test below.
    fn field(e: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Vec<usize> {
        e.child_named(d, path, name).unwrap().unwrap_or_else(|| panic!("no field called {name} under {path:?}"))
    }

    /// What that field reads as.
    fn int(e: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> Option<i128> {
        let p = field(e, d, path, name);
        e.node(d, &p).unwrap().value.as_int()
    }

    /// How many bytes it takes.
    fn bytes(e: &mut Evaluator, d: &Document<MemSource>, path: &[usize], name: &str) -> u64 {
        let p = field(e, d, path, name);
        e.node(d, &p).unwrap().size_bits / 8
    }

    /// Element `i` of the list at `path`.
    fn elem(path: &[usize], i: usize) -> Vec<usize> {
        let mut p = path.to_vec();
        p.push(i);
        p
    }

    /// The filters of the first block of a stream, and the block they are in.
    fn chain(e: &mut Evaluator, d: &Document<MemSource>) -> (Vec<usize>, Vec<usize>) {
        let blocks = field(e, d, &[], "blocks");
        let block = elem(&blocks, 0);
        let flags = field(e, d, &block, "filter_flags");
        let filters = field(e, d, &flags, "filters");
        (block, filters)
    }

    #[test]
    fn the_footer_places_the_index_and_the_index_places_the_blocks() {
        let d = Document::new(MemSource(stream(b"compressed bytes here")));
        let mut e = Evaluator::new(xz());
        assert_eq!(e.node(&d, &[3]).unwrap().value.as_int(), Some(1));
        assert_eq!(e.node(&d, &[5]).unwrap().value.as_int(), Some(4));
        // One block, whose data runs to the padding and the check.
        assert_eq!(e.node(&d, &[6]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[6, 0, 9]).unwrap().size_bits, 24 * 8);
        // One index record, saying the same block's two sizes.
        assert_eq!(e.node(&d, &[7, 2]).unwrap().child_count, 1);
        assert_eq!(e.node(&d, &[7, 2, 0, 0]).unwrap().value.as_int(), Some(37));
        assert_eq!(e.node(&d, &[8, 3]).unwrap().size_bits, 2 * 8);
    }

    /// Every check ID has a length, and only four of them have a name. A
    /// stream whose check is a reserved ID is a stream nothing can verify and
    /// everything can read: the bytes are there, as many as the group says,
    /// and the index behind them lands where it should.
    ///
    /// Read as no bytes at all, which is what an unnamed ID used to come to,
    /// the block's data would swallow the check and the index would be read
    /// from the wrong place. So the index record is what is asserted here.
    #[test]
    fn a_reserved_check_id_is_as_long_as_its_group_says() {
        for (check_type, len) in [(2u8, 4usize), (3, 4), (5, 8), (7, 16), (9, 16), (11, 32), (13, 64), (15, 64)] {
            let d = Document::new(MemSource(stream_of(check_type, len, LZMA2_HEADER, b"data")));
            let mut e = Evaluator::new(xz());
            assert_eq!(int(&mut e, &d, &[], "check_size"), Some(len as i128), "check {check_type}");
            let blocks = field(&mut e, &d, &[], "blocks");
            let block = elem(&blocks, 0);
            assert_eq!(bytes(&mut e, &d, &block, "check"), len as u64, "check {check_type}");
            // The record behind it, which is only readable if the check took
            // the right number of bytes.
            let index = field(&mut e, &d, &[], "index");
            let records = field(&mut e, &d, &index, "records");
            let record = elem(&records, 0);
            let want = (12 + 4 + len) as i128;
            assert_eq!(int(&mut e, &d, &record, "unpadded_size"), Some(want), "check {check_type}");
        }
    }

    /// A stream `liblzma` wrote with two filters: delta at a distance of four
    /// bytes, and then LZMA2 with the 8 MiB dictionary of preset 6.
    const DELTA_LZMA2: &[u8] = &[
        0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x01, 0x69, 0x22, 0xde, 0x36, //
        0x02, 0x01, 0x03, 0x01, 0x03, 0x21, 0x01, 0x16, 0x97, 0x8f, 0x71, 0xfc, //
        0xe0, 0x00, 0x80, 0x00, 0x39, 0x5d, 0x00, 0x3a, 0x1a, 0x08, 0xce, 0x7b, //
        0xb2, 0x93, 0xe7, 0xa9, 0x8f, 0x7d, 0xbc, 0xe6, 0x24, 0x3e, 0x91, 0xce, //
        0x05, 0x88, 0x3e, 0x8e, 0x48, 0xd1, 0x7f, 0x7e, 0xcd, 0x53, 0x0a, 0x62, //
        0xee, 0xda, 0xc7, 0x5b, 0x39, 0xb7, 0x0a, 0xef, 0x02, 0x15, 0xdc, 0x52, //
        0xb8, 0xd0, 0x26, 0xc0, 0x68, 0xa7, 0xa2, 0x7e, 0x9d, 0x4e, 0x35, 0x26, //
        0x93, 0x97, 0xb8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x78, 0xe7, 0x06, 0x27, //
        0x00, 0x01, 0x51, 0x81, 0x01, 0x00, 0x00, 0x00, 0xc8, 0xf1, 0xbd, 0x30, //
        0x3e, 0x30, 0x0d, 0x8b, 0x02, 0x00, 0x00, 0x00, 0x00, 0x01, 0x59, 0x5a,
    ];

    /// The same text through the x86 branch filter with a start offset of
    /// 0x1000, then LZMA2, with SHA-256 as the check.
    const X86_AT_4096: &[u8] = &[
        0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x0a, 0xe1, 0xfb, 0x0c, 0xa1, //
        0x03, 0x01, 0x04, 0x04, 0x00, 0x10, 0x00, 0x00, 0x21, 0x01, 0x16, 0x00, //
        0x5c, 0xb1, 0xc8, 0xfb, 0xe0, 0x00, 0x80, 0x00, 0x30, 0x5d, 0x00, 0x3a, //
        0x1a, 0x08, 0xce, 0x76, 0xc7, 0xe5, 0xe9, 0xd6, 0x07, 0x34, 0xc3, 0xd1, //
        0x0e, 0xbf, 0xce, 0x55, 0xe1, 0xaa, 0xbd, 0xe0, 0xe4, 0x8f, 0x98, 0x01, //
        0xdd, 0x8d, 0xe5, 0x07, 0x54, 0x9e, 0x65, 0x25, 0x5f, 0x27, 0x3a, 0x6a, //
        0x7e, 0xb4, 0xd3, 0x49, 0x27, 0xa8, 0xf1, 0x09, 0x8c, 0x00, 0x00, 0x00, //
        0x4d, 0xb3, 0xfd, 0xe5, 0x09, 0xdf, 0xed, 0xa3, 0x10, 0xe4, 0xfb, 0x80, //
        0x7e, 0x24, 0x38, 0x20, 0xa9, 0x65, 0x1b, 0xf3, 0x5a, 0xc8, 0xf3, 0x17, //
        0x58, 0x07, 0x52, 0x4d, 0x1b, 0x90, 0xf2, 0x68, 0x00, 0x01, 0x68, 0x81, //
        0x01, 0x00, 0x00, 0x00, 0xad, 0xa7, 0xc8, 0x13, 0xb6, 0xe9, 0xdf, 0x1c, //
        0x02, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x59, 0x5a,
    ];

    /// The same again with no start offset at all, which is what an encoder
    /// handed a whole file writes, and CRC64 as the check.
    const X86_AT_NOTHING: &[u8] = &[
        0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x04, 0xe6, 0xd6, 0xb4, 0x46, //
        0x02, 0x01, 0x04, 0x00, 0x21, 0x01, 0x16, 0x00, 0x0d, 0x86, 0x35, 0x1f, //
        0xe0, 0x00, 0x80, 0x00, 0x30, 0x5d, 0x00, 0x3a, 0x1a, 0x08, 0xce, 0x76, //
        0xc7, 0xe5, 0xe9, 0xd6, 0x07, 0x34, 0xc3, 0xd1, 0x0e, 0xbf, 0xce, 0x55, //
        0xe1, 0xaa, 0xbd, 0xe0, 0xe4, 0x8f, 0x98, 0x01, 0xdd, 0x8d, 0xe5, 0x07, //
        0x54, 0x9e, 0x65, 0x25, 0x5f, 0x27, 0x3a, 0x6a, 0x7e, 0xb4, 0xd3, 0x49, //
        0x27, 0xa8, 0xf1, 0x09, 0x8c, 0x00, 0x00, 0x00, 0x7f, 0x3d, 0xc4, 0x61, //
        0x7e, 0x20, 0x6b, 0x6f, 0x00, 0x01, 0x4c, 0x81, 0x01, 0x00, 0x00, 0x00, //
        0x8d, 0xe0, 0xf5, 0x8f, 0xb1, 0xc4, 0x67, 0xfb, 0x02, 0x00, 0x00, 0x00, //
        0x00, 0x04, 0x59, 0x5a,
    ];

    /// A chain of two filters, read as two filters rather than as a run of
    /// bytes: which filter, in the order the data went through them, and what
    /// each one was told.
    #[test]
    fn a_real_chain_reads_as_delta_and_then_lzma2() {
        let d = Document::new(MemSource(DELTA_LZMA2.to_vec()));
        let mut e = Evaluator::new(xz());
        let (block, filters) = chain(&mut e, &d);
        assert_eq!(e.node(&d, &filters).unwrap().child_count, 2);

        let delta = elem(&filters, 0);
        assert_eq!(int(&mut e, &d, &delta, "filter_id"), Some(0x03));
        assert_eq!(int(&mut e, &d, &delta, "properties_size"), Some(1));
        let props = field(&mut e, &d, &delta, "properties");
        // Four bytes back, written as three.
        assert_eq!(int(&mut e, &d, &props, "distance_minus_one"), Some(3));

        let lzma2 = elem(&filters, 1);
        assert_eq!(int(&mut e, &d, &lzma2, "filter_id"), Some(0x21));
        let props = field(&mut e, &d, &lzma2, "properties");
        assert_eq!(int(&mut e, &d, &props, "properties_reserved"), Some(0));
        // Preset 6, which is the 8 MiB dictionary the spec's table gives for
        // a code of 22: a mantissa of 2 shifted by 22.
        assert_eq!(int(&mut e, &d, &props, "dictionary_size_code"), Some(22));
        assert_eq!(int(&mut e, &d, &props, "dictionary_bytes"), Some(8 * 1024 * 1024));

        // The chain filled the header exactly, so there is nothing to pad.
        let flags = field(&mut e, &d, &block, "filter_flags");
        assert_eq!(bytes(&mut e, &d, &flags, "header_padding"), 0);
        let crc = field(&mut e, &d, &block, "header_crc32");
        let got = e.run_check(&d, &crc).unwrap().expect("a block header has a CRC32");
        assert!(got.ok, "computed {}, stored {}", got.computed, got.stored);
    }

    /// A branch filter writes four bytes of start offset or none at all, and
    /// both are ordinary. The four bytes are the address the data came from;
    /// nothing written means zero.
    #[test]
    fn a_branch_filter_writes_a_start_offset_or_nothing() {
        let d = Document::new(MemSource(X86_AT_4096.to_vec()));
        let mut e = Evaluator::new(xz());
        let (block, filters) = chain(&mut e, &d);
        let x86 = elem(&filters, 0);
        assert_eq!(int(&mut e, &d, &x86, "filter_id"), Some(0x04));
        assert_eq!(int(&mut e, &d, &x86, "properties_size"), Some(4));
        let props = field(&mut e, &d, &x86, "properties");
        assert_eq!(int(&mut e, &d, &props, "start_offset"), Some(0x1000));
        // What was left of the sixteen bytes the header claimed.
        let flags = field(&mut e, &d, &block, "filter_flags");
        assert_eq!(bytes(&mut e, &d, &flags, "header_padding"), 1);
        // SHA-256, which is thirty-two bytes and not a number.
        assert_eq!(int(&mut e, &d, &[], "check_size"), Some(32));
        assert_eq!(bytes(&mut e, &d, &block, "check"), 32);

        let d = Document::new(MemSource(X86_AT_NOTHING.to_vec()));
        let mut e = Evaluator::new(xz());
        let (block, filters) = chain(&mut e, &d);
        let x86 = elem(&filters, 0);
        assert_eq!(int(&mut e, &d, &x86, "filter_id"), Some(0x04));
        assert_eq!(int(&mut e, &d, &x86, "properties_size"), Some(0));
        assert_eq!(bytes(&mut e, &d, &x86, "properties"), 0);
        assert_eq!(int(&mut e, &d, &[], "check_size"), Some(8));
        assert_eq!(bytes(&mut e, &d, &block, "check"), 8);
        let crc = field(&mut e, &d, &block, "header_crc32");
        let got = e.run_check(&d, &crc).unwrap().expect("a block header has a CRC32");
        assert!(got.ok, "computed {}, stored {}", got.computed, got.stored);
    }

    /// The dictionary size code, over the whole of what it can say. The two
    /// ends and the step in between are the spec's own table: 4 KiB at 0, half
    /// as much again at 1, and a byte short of 4 GiB at 40, which is the one
    /// value the arithmetic does not produce.
    ///
    /// Above 40 the encoding has run out, and the field holds nothing rather
    /// than the number the shift would have given: a decoder handed such a
    /// stream refuses it, so there is no dictionary size to show.
    #[test]
    fn the_dictionary_size_code_is_a_mantissa_and_an_exponent() {
        for (code, want) in [
            (0u8, Some(4 * 1024)),
            (1, Some(6 * 1024)),
            (22, Some(8 * 1024 * 1024)),
            (39, Some(3 * 1024 * 1024 * 1024)),
            (40, Some(4 * 1024 * 1024 * 1024 - 1)),
            (41, None),
            (63, None),
        ] {
            let mut header = LZMA2_HEADER.to_vec();
            header[4] = code;
            let d = Document::new(MemSource(stream_of(1, 4, &header, b"data")));
            let mut e = Evaluator::new(xz());
            let (_, filters) = chain(&mut e, &d);
            let props = field(&mut e, &d, &elem(&filters, 0), "properties");
            assert_eq!(int(&mut e, &d, &props, "dictionary_size_code"), Some(code as i128), "code {code}");
            let size = field(&mut e, &d, &props, "dictionary_bytes");
            match want {
                Some(n) => assert_eq!(e.node(&d, &size).unwrap().value.as_int(), Some(n), "code {code}"),
                // Not a size of zero: a field that is not there, which is what
                // a code the encoding does not reach has to come to.
                None => {
                    let v = e.node(&d, &size).unwrap().value;
                    assert!(matches!(v, Value::Bytes { len: 0, .. }), "code {code} gave {v:?}");
                }
            }
        }
    }

    /// Nothing in a block header has been checked against anything when the
    /// filter chain is read: the CRC32 that would say the header is sound is
    /// behind the chain, and reaching it means walking the chain first. So a
    /// header that lies has to be read anyway, and the reading has to stop
    /// where the header says the header stops.
    #[test]
    fn a_filter_chain_that_lies_stops_at_the_end_of_the_header() {
        // Properties two hundred bytes long in a header with six bytes of
        // room, which without a window would read the CRC32, the compressed
        // data and the index as one filter's settings.
        // `0xc8 0x01` is two hundred as one of xz's variable-length integers.
        let header = &[0x02, 0x00, 0x21, 0xc8, 0x01, 0, 0, 0, 0, 0, 0, 0];
        let d = Document::new(MemSource(stream_of(1, 4, header, b"data")));
        let mut e = Evaluator::new(xz());
        let (block, filters) = chain(&mut e, &d);
        let f = elem(&filters, 0);
        assert_eq!(int(&mut e, &d, &f, "properties_size"), Some(200));
        assert_eq!(bytes(&mut e, &d, &f, "properties"), 3);
        let flags = field(&mut e, &d, &block, "filter_flags");
        assert_eq!(e.node(&d, &flags).unwrap().size_bits, 6 * 8);
        // The fields after the chain are where the header said they would be,
        // which is the whole point: the CRC32 still verifies.
        let crc = field(&mut e, &d, &block, "header_crc32");
        let got = e.run_check(&d, &crc).unwrap().expect("a block header has a CRC32");
        assert!(got.ok, "computed {}, stored {}", got.computed, got.stored);
    }

    /// Four filters claimed in a header with room for one and a bit. The count
    /// is two bits wide, so any header can claim four, and a file nobody
    /// vouched for does.
    ///
    /// What comes back is a row per filter the header claimed: the one that is
    /// really there, and then the header's own zero bytes read as the filters
    /// it said were written, down to a row with nothing in it once there is no
    /// room left. The alternative was a list that refuses to be read, which
    /// would take the good filter down with the claimed ones.
    #[test]
    fn a_header_claiming_more_filters_than_it_holds_reads_what_is_there() {
        let header = &[0x02, 0x03, 0x21, 0x01, 0x00, 0, 0, 0, 0, 0, 0, 0];
        let d = Document::new(MemSource(stream_of(1, 4, header, b"data")));
        let mut e = Evaluator::new(xz());
        let (block, filters) = chain(&mut e, &d);
        assert_eq!(e.node(&d, &filters).unwrap().child_count, 4);
        let first = elem(&filters, 0);
        assert_eq!(int(&mut e, &d, &first, "filter_id"), Some(0x21));
        let props = field(&mut e, &d, &first, "properties");
        assert_eq!(int(&mut e, &d, &props, "dictionary_bytes"), Some(4096));
        // The last of the four had nothing left to read at all.
        assert_eq!(e.node(&d, &elem(&filters, 3)).unwrap().size_bits, 0);
        // None of which left the window: six bytes claimed, six bytes read.
        let flags = field(&mut e, &d, &block, "filter_flags");
        assert_eq!(e.node(&d, &flags).unwrap().size_bits, 6 * 8);
        let total: u64 = (0..4).map(|i| e.node(&d, &elem(&filters, i)).unwrap().size_bits).sum();
        assert_eq!(total + bytes(&mut e, &d, &flags, "header_padding") * 8, 6 * 8);
        let crc = field(&mut e, &d, &block, "header_crc32");
        let got = e.run_check(&d, &crc).unwrap().expect("a block header has a CRC32");
        assert!(got.ok, "computed {}, stored {}", got.computed, got.stored);
    }

    /// A stream that really is one: every block packed as LZMA2 by the crate
    /// the codec's own tests pack with, and sealed with whichever check the
    /// stream flags name. Built there rather than here because a check is only
    /// worth testing over data that really came out of a decoder.
    /// `sized` writes both of the sizes a block header may carry, which is
    /// what the threaded encoder does and what `xz -T1` leaves out. It is the
    /// difference between a multi-block stream the template can split and one
    /// it reads as a single block; see `block_check`.
    fn real(parts: &[&[u8]], check_type: u8, sized: bool) -> Vec<u8> {
        let blocks: Vec<(Vec<u8>, Vec<u8>)> =
            parts.iter().map(|p| (xz_codec::pack(p, 3, 0, 2, 1 << 20, None), p.to_vec())).collect();
        xz_codec::wrap_checked(&blocks, sized, check_type)
    }

    /// Something with enough shape in it to pack: a few hundred bytes that are
    /// neither all the same nor all different.
    fn text(seed: u8) -> Vec<u8> {
        let mut v = Vec::new();
        while v.len() < 400 {
            v.extend_from_slice(b"the quick brown fox jumps over the lazy dog. ");
            v.push(seed);
        }
        v
    }

    /// The check field of block `i`.
    fn block_check(e: &mut Evaluator, d: &Document<MemSource>, i: usize) -> Vec<usize> {
        let blocks = field(e, d, &[], "blocks");
        field(e, d, &elem(&blocks, i), "check")
    }

    /// The bytes the stream unpacks to, opened the way a tab opens one.
    ///
    /// The `decoded` field costs no bytes where it stands and holds the run as
    /// its one child, so the stream is one step in from the field's own path.
    fn unpacked(e: &mut Evaluator, d: &Document<MemSource>) -> Vec<u8> {
        let p = elem(&field(e, d, &[], "decoded"), 0);
        let id = e.open_space(d, 0, &p).unwrap().expect("the stream opens");
        e.space(id).expect("the space is there").bytes().to_vec()
    }

    /// A block's check is taken, and it is taken over what that block unpacked
    /// to rather than over anything in the file.
    ///
    /// One case per check the format has assigned. `xz` writes the second of
    /// them unless it is told otherwise, and it is the one that had no
    /// arithmetic here at all until this: a default `.xz` was opened with
    /// nothing on screen able to say whether its data was intact.
    #[test]
    fn a_block_check_is_taken_over_the_bytes_that_block_unpacked_to() {
        for (check_type, algorithm) in [(1u8, "crc32"), (4, "crc64"), (10, "sha256")] {
            let data = text(1);
            let d = Document::new(MemSource(real(&[&data], check_type, false)));
            let mut e = Evaluator::new(xz());
            let p = block_check(&mut e, &d, 0);
            let info = e.check_of(&d, &p).unwrap().unwrap_or_else(|| panic!("check {check_type} checks nothing"));
            assert_eq!(info.algorithm, algorithm);
            // Not a run of the file: the summed bytes are not in the file at
            // all, and what a reader is sent to instead is this block's own
            // packed data.
            assert_eq!(info.over, None, "check {check_type}");
            let (at, len) = info.unpacked_from.unwrap_or_else(|| panic!("check {check_type} points nowhere"));
            let blocks = field(&mut e, &d, &[], "blocks");
            let packed = field(&mut e, &d, &elem(&blocks, 0), "compressed");
            let node = e.node(&d, &packed).unwrap();
            assert_eq!((at, len), (node.offset_bits / 8, node.size_bits / 8), "check {check_type}");

            let v = e.run_check(&d, &p).unwrap().unwrap_or_else(|| panic!("check {check_type} was not taken"));
            assert!(v.ok, "check {check_type}: computed {}, stored {}", v.computed, v.stored);
        }
    }

    /// Three blocks, each sealed over its own share of the output.
    ///
    /// The second half is what says the shares are really separate. Give each
    /// block the sum of the block before it and every one of them has to fail:
    /// a check that quietly summed the whole stream, or always the front of
    /// it, would pass one of these and fail the rest, and a check that summed
    /// nothing would pass all three.
    #[test]
    fn each_block_is_checked_against_its_own_share_of_the_output() {
        let parts = [text(1), text(2), text(3)];
        let stream = real(&[&parts[0], &parts[1], &parts[2]], 4, true);
        let d = Document::new(MemSource(stream.clone()));
        let mut e = Evaluator::new(xz());
        let mut runs = Vec::new();
        for i in 0..3 {
            let p = block_check(&mut e, &d, i);
            let node = e.node(&d, &p).unwrap();
            runs.push((node.offset_bits as usize / 8, node.size_bits as usize / 8));
            let v = e.run_check(&d, &p).unwrap().expect("a block has a check");
            assert!(v.ok, "block {i}: computed {}, stored {}", v.computed, v.stored);
        }

        let mut shuffled = stream.clone();
        for (i, &(at, len)) in runs.iter().enumerate() {
            let (from, _) = runs[(i + runs.len() - 1) % runs.len()];
            shuffled[at..at + len].copy_from_slice(&stream[from..from + len]);
        }
        let d = Document::new(MemSource(shuffled));
        let mut e = Evaluator::new(xz());
        for i in 0..3 {
            let p = block_check(&mut e, &d, i);
            let v = e.run_check(&d, &p).unwrap().expect("a block has a check");
            assert!(!v.ok, "block {i} passed with another block's sum: {}", v.computed);
        }
    }

    /// The same three blocks with no sizes in their headers, which is what
    /// `xz` writes. The template cannot split them: a block whose header does
    /// not say how long its data is runs to the end of the block region, so
    /// what comes back is one block holding all three and the last block's
    /// check.
    ///
    /// The check has to refuse there, and refuse is the whole assertion. The
    /// numbers do not match and never could, and a reading that took them as a
    /// mismatch would put a red verdict on a file that is perfectly good.
    #[test]
    fn blocks_the_template_cannot_split_report_no_verdict_rather_than_a_wrong_one() {
        let parts = [text(1), text(2), text(3)];
        let d = Document::new(MemSource(real(&[&parts[0], &parts[1], &parts[2]], 4, false)));
        let mut e = Evaluator::new(xz());
        let blocks = field(&mut e, &d, &[], "blocks");
        assert_eq!(e.node(&d, &blocks).unwrap().child_count, 1, "this is the reading the test is about");
        // The stream is fine and opens to all three blocks' bytes.
        assert_eq!(unpacked(&mut e, &d), parts.concat());

        let p = block_check(&mut e, &d, 0);
        assert!(e.check_of(&d, &p).unwrap().is_some(), "the field is still a checksum");
        let e2 = e.run_check(&d, &p).expect_err("no verdict may be reached here");
        assert!(matches!(e2, EvalError::Failed(_)), "{e2:?}");
    }

    /// A block whose data was changed opens, shows its bytes, and says the
    /// check failed. All three, and the first two are the point: a tool for
    /// looking at damaged files that refuses to show one is no use on the day
    /// it is needed.
    ///
    /// The data is packed as an uncompressed LZMA2 chunk so that changing a
    /// byte changes a byte. A byte changed inside a range-coded chunk does not
    /// give the file with one byte wrong; it gives a block that decodes to the
    /// wrong length or not at all, which the index catches and which is a
    /// different failure from the one this is about.
    #[test]
    fn a_block_whose_bytes_were_changed_opens_and_says_the_check_failed() {
        let data = text(1);
        let stream = xz_codec::wrap_checked(&[(xz_codec::stored_lzma2(&data), data.clone())], false, 4);
        // Into the middle of the stored chunk: past the stream header, the
        // block header, and the chunk's own control byte and size.
        let mut broken = stream.clone();
        let at = 12 + 12 + 3 + data.len() / 2;
        broken[at] ^= 0xff;
        assert_ne!(broken, stream, "the test changed nothing");

        let d = Document::new(MemSource(broken));
        let mut e = Evaluator::new(xz());
        // The file reads: the blocks are there, the index is there.
        let blocks = field(&mut e, &d, &[], "blocks");
        assert_eq!(e.node(&d, &blocks).unwrap().child_count, 1);
        // And the bytes are there to look at, changed byte and all.
        let out = unpacked(&mut e, &d);
        assert_eq!(out.len(), data.len(), "the block still unpacks to what the index says");
        assert_ne!(out, data, "the changed byte is in what comes back");
        assert_eq!(out[data.len() / 2], data[data.len() / 2] ^ 0xff);

        let p = block_check(&mut e, &d, 0);
        let v = e.run_check(&d, &p).unwrap().expect("the check is still taken");
        assert!(!v.ok, "a changed byte passed the block check");
        assert_ne!(v.computed, v.stored);
    }

    /// A stream with no check, and a stream whose check nobody has defined.
    /// Neither is a failure and neither is a pass: the field says nothing at
    /// all, which is the only true thing to say about a sum that was never
    /// written or whose arithmetic has never been published.
    ///
    /// The rest of the stream still has to read, which is the reason the size
    /// of a reserved check is worked out from its ID rather than left unknown:
    /// those bytes are in the file whether or not anybody can sum them.
    #[test]
    fn a_check_that_is_none_or_reserved_reports_neither_pass_nor_failure() {
        for check_type in [0u8, 2, 5, 15] {
            let data = text(1);
            let d = Document::new(MemSource(real(&[&data], check_type, false)));
            let mut e = Evaluator::new(xz());
            let p = block_check(&mut e, &d, 0);
            assert!(e.check_of(&d, &p).unwrap().is_none(), "check type {check_type} claims to check something");
            assert!(e.run_check(&d, &p).unwrap().is_none(), "check type {check_type} returned a verdict");
            // The bytes after it still line up: the index reads, and it says
            // what the block held.
            let index = field(&mut e, &d, &[], "index");
            assert_eq!(int(&mut e, &d, &index, "record_count"), Some(1), "check type {check_type}");
            let records = field(&mut e, &d, &index, "records");
            assert_eq!(
                int(&mut e, &d, &elem(&records, 0), "uncompressed_size"),
                Some(data.len() as i128),
                "check type {check_type}"
            );
            assert_eq!(unpacked(&mut e, &d), data, "check type {check_type}: the stream still opens");
        }
    }

    /// A stream the crate read, rather than one read here, still gets its
    /// block check taken.
    ///
    /// The dictionary size code is what forces that, as in the codec's own
    /// tests: it is a properties byte no encoder would write, so the chain is
    /// left alone and `lzma-rs` is handed the stream whole. What comes back
    /// then is a map of the blocks and nothing inside them, and the block's
    /// check still covers that block's bytes, so the map has to be enough to
    /// say where they are.
    #[test]
    fn a_stream_read_by_the_crate_is_still_checked() {
        let data = text(1);
        let mut stream = real(&[&data], 4, false);
        stream[16] = 63;
        let sum = crc32(&stream[12..20]);
        stream[20..24].copy_from_slice(&sum.to_le_bytes());

        let d = Document::new(MemSource(stream));
        let mut e = Evaluator::new(xz());
        assert_eq!(unpacked(&mut e, &d), data, "the crate reads it even though we would not");
        let p = block_check(&mut e, &d, 0);
        let v = e.run_check(&d, &p).unwrap().expect("a block read by the crate still has a check");
        assert!(v.ok, "computed {}, stored {}", v.computed, v.stored);
    }

    /// A header claiming a thousand bytes in a block region of twenty. The
    /// block is refused rather than read, which is the right answer: a header
    /// longer than the file it is in is not a header, and there is nothing to
    /// show a reader in place of it.
    ///
    /// What the test is really for is where the refusal stops. The index and
    /// the footer are placed from the end of the file and owe the block
    /// nothing, so they still read, and the file still says how many blocks
    /// were meant to be there and how long each of them should have been.
    #[test]
    fn a_header_longer_than_the_file_is_refused_and_the_index_still_reads() {
        let header = &[0xff, 0x00, 0x21, 0x01, 0x00, 0, 0, 0, 0, 0, 0, 0];
        let d = Document::new(MemSource(stream_of(1, 4, header, b"data")));
        let mut e = Evaluator::new(xz());
        let blocks = field(&mut e, &d, &[], "blocks");
        assert!(e.node(&d, &blocks).is_err(), "a header of a thousand bytes in twenty must not read");
        assert_eq!(int(&mut e, &d, &[], "check_size"), Some(4));
        let index = field(&mut e, &d, &[], "index");
        assert_eq!(int(&mut e, &d, &index, "record_count"), Some(1));
        let records = field(&mut e, &d, &index, "records");
        assert_eq!(int(&mut e, &d, &elem(&records, 0), "unpadded_size"), Some(20));
        let footer = field(&mut e, &d, &[], "footer");
        let crc = field(&mut e, &d, &footer, "footer_crc32");
        let got = e.run_check(&d, &crc).unwrap().expect("a footer has a CRC32");
        assert!(got.ok, "computed {}, stored {}", got.computed, got.stored);
    }
}
