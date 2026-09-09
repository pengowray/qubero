//! A 7z archive: thirty-two bytes at the front that say where everything else
//! is, the packed streams, and a header at the end describing them.
//!
//! Putting the header last is what lets the archiver write an entry before it
//! knows how large the entry turned out to be, and the front of the file is
//! then a pointer to it: an offset, a length, and a checksum, with a second
//! checksum over those three so that a truncated file is told from a corrupt
//! one.
//!
//! The header is a tree of tagged blocks. A byte names the block, its contents
//! follow, and a zero byte closes the run; the blocks inside a run come in a
//! fixed order and any of them may be missing, so what says a block is there
//! is the tag byte sitting at the cursor and nothing else. That is why nearly
//! every field here is a switch on a peek rather than a field: 7z writes no
//! length in front of a block, and the only way to find the end of one is to
//! read all of it.
//!
//! **What the header is read for.** `kPackInfo` gives a base offset and a size
//! per packed stream, and the front of the file is then not one run of
//! compressed bytes but the several the archiver actually wrote. `kFilesInfo`
//! holds the names, as UTF-16 with a NUL after each. Reading those is the
//! whole point: without them an archive is two blobs, and with them it is its
//! contents.
//!
//! **The packed streams.** A stream is a folder's output, and what says how to
//! get at it is that folder's coder: `kUnPackInfo` names the codec and carries
//! whatever settings the codec does not keep in its own stream. So the streams
//! open, and an archive written a file at a time becomes its files. What a
//! folder cannot be is divided, which is why a solid archive opens as one run
//! holding all of them; see below, and [`packed_run`] for which folder a
//! stream belongs to and which coders can be run at all.
//!
//! **The compressed header.** 7z compresses its own header by default, and
//! what the offset then points at is a `kEncodedHeader` describing the one
//! stream the real header was packed into. That stream is raw LZMA1, which
//! carries none of what a decoder needs: the properties byte, the dictionary
//! size and how much comes out are all written *in the header out here*, in
//! the coder of the folder that packed it. So the reading taken from inside
//! the packed region goes as far as that coder, and the stream is opened with
//! what it says. See [`pack_info_ahead`] and [`encoded_header_stream`].
//!
//! What comes out is a `kHeader`, read by the same declarations below in a
//! space of its own. An archive written the default way and one written with
//! `7z a -mhc=off` are the same reading, one of them a level further in.
//!
//! **What is not read here.**
//!
//! - **The files inside a folder.** A folder opens; it does not divide. Where
//!   one file inside it stops and the next begins is in `kSubStreamsInfo`, out
//!   in the archive, and saying so would mean a template inside the unpacked
//!   space naming fields of space 0, which nothing can express: a space's
//!   fields count from its own start and reach nothing outside it. So the
//!   bytes of a solid folder are there, in order, and the names are there
//!   beside them, and which stretch is which is not said.
//! - **The files of an archive whose header is compressed.** Which is what
//!   7-Zip writes by default, so it is the common case and not a corner. The
//!   header out here describes one stream, its own; the streams holding the
//!   files are described only by the header that comes out of it, and reaching
//!   those fields from out here means an expression naming a field inside a
//!   decoded space. That is the same wall as the bullet above, faced from the
//!   other side. The files are the run named `before_pack_pos`, whole.
//! - **Which entries are empty.** `kEmptyFile` and `kAnti` hold one bit per
//!   *empty stream*, not per file, and how many of those there are is a count
//!   of the bits set in `kEmptyStream`. Their bytes are shown and not divided.
//! - **Folders held in another stream.** `external` says the folder list is
//!   somewhere else in the archive rather than here. Nothing writes it.
//! - **The substream CRC count when the folders already have one.** A folder
//!   holding exactly one substream lends it its own CRC, and that substream
//!   then has no digest of its own. Working out how many streams that leaves
//!   needs a sum with a condition in it, which is a shape no expression here
//!   has. 7-Zip writes folder digests only where there is no `kSubStreamsInfo`
//!   to disagree with, so the two never meet in a file it wrote; in one where
//!   they do, the walk reads too many digests and the tag that should follow
//!   them reads as something other than `kEnd`, which is where a reader can
//!   see it went wrong.

use crate::codec::Codec;
use crate::template::{
    Check, Checksum, Covers, Encoding,
    Endian::{Big, Little},
    Expr as E, Named, Packing, StrLen, Template, Time, Ty as T, Until,
};

/// What one of these starts with.
pub const MAGIC: &[u8] = b"7z\xbc\xaf\x27\x1c";

/// The start header: the magic, the version, and the three numbers locating
/// the end header, with the two checksums. Every offset the format writes is
/// counted from the end of it rather than from the start of the file.
const START_HEADER: i128 = 32;

/// The tags 7z labels the parts of its header with. One list, because one
/// byte in this format is always one of these however deep in the tree it
/// sits, and a reader looking at a tag wants its name whether or not the
/// template expected it there.
const PROPERTY_IDS: &[(i128, &str)] = &[
    (0x00, "kEnd"),
    (0x01, "kHeader"),
    (0x02, "kArchiveProperties"),
    (0x03, "kAdditionalStreamsInfo"),
    (0x04, "kMainStreamsInfo"),
    (0x05, "kFilesInfo"),
    (0x06, "kPackInfo"),
    (0x07, "kUnPackInfo"),
    (0x08, "kSubStreamsInfo"),
    (0x09, "kSize"),
    (0x0a, "kCRC"),
    (0x0b, "kFolder"),
    (0x0c, "kCodersUnPackSize"),
    (0x0d, "kNumUnPackStream"),
    (0x0e, "kEmptyStream"),
    (0x0f, "kEmptyFile"),
    (0x10, "kAnti"),
    (0x11, "kName"),
    (0x12, "kCTime"),
    (0x13, "kATime"),
    (0x14, "kMTime"),
    (0x15, "kWinAttributes"),
    (0x16, "kComment"),
    (0x17, "kEncodedHeader"),
    (0x18, "kStartPos"),
    (0x19, "kDummy"),
];

fn property_id() -> T {
    T::enumeration_hex("PropertyId", T::u8(), PROPERTY_IDS)
}

/// The codecs a coder can name, by the numbers 7-Zip's own registry binds them
/// to and under the names it prints for them.
///
/// An id is written most significant byte first in as few bytes as it takes,
/// with the count of them in the low nibble of the coder's flags, so `00` and
/// `030101` are one number each and never collide however long they were
/// written. That is why one list answers for all four widths.
///
/// The names are 7-Zip's registered spellings rather than tidied ones, so that
/// a row here reads as the same word `7z l -slt` prints. Beware the one-byte
/// filter ids: `03` is Delta and `0a`/`0b` are ARM64 and RISCV, but the `04`
/// to `09` run that looks like it should continue them is **xz's** filter
/// numbering and means nothing in a 7z header, where x86 is the four-byte
/// `03030103`. 7-Zip looks an id up by exact match and keeps no aliases.
const CODEC_IDS: &[(i128, &str)] = &[
    (0x00, "Copy"),
    (0x03, "Delta"),
    (0x0a, "ARM64"),
    (0x0b, "RISCV"),
    (0x21, "LZMA2"),
    (0x02_0302, "Swap2"),
    (0x02_0304, "Swap4"),
    (0x03_0101, "LZMA"),
    (0x03_0401, "PPMD"),
    (0x04_0108, "Deflate"),
    (0x04_0109, "Deflate64"),
    (0x04_0202, "BZip2"),
    (0x0303_0103, "BCJ"),
    (0x0303_011b, "BCJ2"),
    (0x0303_0205, "PPC"),
    (0x0303_0401, "IA64"),
    (0x0303_0501, "ARM"),
    (0x0303_0701, "ARMT"),
    (0x0303_0805, "SPARC"),
    (0x04f7_1101, "ZSTD"),
    (0x04f7_1102, "BROTLI"),
    (0x04f7_1104, "LZ4"),
    (0x04f7_1105, "LZ5"),
    (0x04f7_1106, "LIZARD"),
    (0x06f1_0701, "7zAES"),
];

/// Which codec a coder runs, as the number it is rather than the bytes it was
/// written in.
///
/// The length is the low nibble of the flags byte, which is why this is a
/// switch and not a field: the same value is one, two, three or four bytes
/// wide depending on what the archiver had to spend. Reading it as a number is
/// what lets anything switch on it, and the block below is the first thing
/// that does.
///
/// A length the format cannot mean stays bytes. The nibble holds up to fifteen
/// and 7-Zip writes at most four, so nothing else is a coder anything reads.
/// How many bytes the coder wrote its codec id in: the low nibble of the
/// flags. Asked by three of the rows of a coder, which is why it is a function
/// rather than a line repeated.
fn id_size() -> E {
    E::field("flags").and(E::lit(0x0f))
}

fn codec_id() -> T {
    let id_size = id_size();
    let named = |inner: T| T::enumeration_hex("CodecId", inner, CODEC_IDS);
    T::switch(
        id_size.clone(),
        vec![
            (1, named(T::u8())),
            (2, named(T::UInt { bits: 16, endian: Big })),
            (3, named(T::UInt { bits: 24, endian: Big })),
            (4, named(T::u32(Big))),
        ],
        T::bytes(id_size),
    )
}

/// Every count, size and offset in the header. See [`T::SevenZipNumber`].
fn number() -> T {
    T::sevenzip_number()
}

/// A field of no bytes, for a block that is not there.
fn nothing() -> T {
    T::bytes(E::lit(0))
}

/// The rest of the header, from wherever the walk stopped being able to read
/// it.
///
/// Taking everything that is left is what makes stopping honest. The blocks
/// after this one are declared with [`T::if_room`], so once this has claimed
/// the room they quietly become nothing rather than reading a `kEnd` out of
/// the middle of a CRC.
///
/// Not `unread`, which the listing already spends on
/// [`crate::eval::Value::Unread`]: bytes that have not been fetched yet. These
/// bytes are here and have been looked at; what could not be done is make
/// sense of them.
fn unparsed() -> T {
    T::bytes(E::Remaining)
}

/// A block that is there only when the tag at the cursor is its own.
///
/// The switch is what makes the peek safe as well as what makes it optional:
/// a case is only resolved once it is chosen, so a block at the very end of
/// the header never looks past it for a tag that is not there.
fn tagged(id: i128, body: T) -> T {
    T::if_room(T::switch(E::peek(8, Big), vec![(id, body)], nothing()))
}

/// A CRC-32 for each of `count` streams, behind a byte saying whether every
/// one of them has one.
fn digests(count: E) -> T {
    let bit_vector = count.clone().add(E::lit(7)).div(E::lit(8));
    T::structure(
        "Digests",
        vec![
            ("id", property_id()),
            ("all_defined", T::u8()),
            (
                "crcs",
                T::switch(
                    E::field("all_defined"),
                    vec![(1, T::array(T::u32(Little), count))],
                    // A bit per stream saying which ones have a CRC, and then
                    // one CRC for each bit that is set. The vector is a run of
                    // bytes with no children to add up, so the count comes
                    // from the bits themselves.
                    T::inline_structure(
                        "SparseDigests",
                        vec![("defined", T::bytes(bit_vector)), ("crcs", T::array(T::u32(Little), E::pop_count("defined")))],
                    ),
                ),
            ),
        ],
    )
}

/// Where the packed streams are and how long each one is. The only block the
/// front of the file needs, and the reason the header is read at all.
fn pack_info() -> T {
    T::structure(
        "PackInfo",
        vec![
            ("id", property_id()),
            // Counted from the end of the start header, like everything else.
            ("pack_pos", number()),
            ("num_pack_streams", number()),
            // 7-Zip's own reader treats the sizes as required and gives up
            // without them, so a block that leaves them out is not a 7z
            // anything reads. Declared rather than peeked at for that reason:
            // a tag that is not `kSize` here should be visible as one.
            ("pack_sizes_id", property_id()),
            ("pack_sizes", T::array(number(), E::field("num_pack_streams"))),
            ("crcs", tagged(0x0a, digests(E::field("num_pack_streams")))),
            ("end", T::if_room(property_id())),
        ],
    )
}

/// How LZMA was set up: one byte packing three numbers, and the dictionary.
///
/// The byte is `lc + 9 * (lp + 5 * pb)`, so the three come back out by
/// dividing rather than by masking, which is why they are worked out rather
/// than read: they are not bit fields and no run of bits holds any of them.
/// `5d` is 3, 0 and 2, which is what every 7-Zip since the first writes unless
/// it was told otherwise.
///
/// These five bytes are the whole of what a raw LZMA1 stream does not carry,
/// which is why a 7z header can be unpacked at all: see [`packed_streams`].
fn lzma_properties() -> T {
    let props = E::field("props");
    let lp_and_pb = props.clone().div(E::lit(9));
    T::inline_structure(
        "LzmaProperties",
        vec![
            ("props", T::u8()),
            ("literal_context_bits", T::computed(props.clone().sub(lp_and_pb.clone().mul(E::lit(9))))),
            (
                "literal_pos_bits",
                T::computed(lp_and_pb.clone().sub(lp_and_pb.clone().div(E::lit(5)).mul(E::lit(5)))),
            ),
            ("pos_bits", T::computed(lp_and_pb.div(E::lit(5)))),
            ("dict_size", T::u32(Little)),
            ("unparsed", T::if_room(unparsed())),
        ],
    )
}

/// How LZMA2 was set up: one byte standing for the dictionary size, which is
/// `(2 | (b & 1)) << (b / 2 + 11)` for every value but 40, where it is one
/// byte short of four gigabytes. Read as the code it is; nothing here unpacks
/// LZMA2, and multiplying it out would be a number no run of the file holds.
fn lzma2_properties() -> T {
    T::inline_structure(
        "Lzma2Properties",
        vec![("dict_size_code", T::u8()), ("unparsed", T::if_room(unparsed()))],
    )
}

/// The settings a coder wrote down, read as the fields they are rather than as
/// the run of bytes they sit in.
///
/// Switched on the codec id, and on its width before that. A width the format
/// does not write leaves the id as bytes, and bytes hold no number to switch
/// on; asking the width first is what keeps a coder whose id is nonsense from
/// taking the folder around it down with it. Each id has one width, so the two
/// questions cost one row between them.
///
/// Every other codec keeps its settings as bytes. That is not a gap to be
/// filled in one go: a codec whose settings nothing here reads has none worth
/// dividing until something reads them.
fn coder_settings() -> T {
    T::switch(
        id_size(),
        vec![
            (1, T::switch(E::field("codec_id"), vec![(0x21, lzma2_properties())], unparsed())),
            (3, T::switch(E::field("codec_id"), vec![(0x03_0101, lzma_properties())], unparsed())),
        ],
        unparsed(),
    )
}

/// One step of the pipeline a folder runs its bytes through: which codec, and
/// what it was set up with.
fn coder() -> T {
    T::structure(
        "Coder",
        vec![
            // The low four bits are how long the codec id is; bit 4 says the
            // coder takes more than one stream in or gives more than one out,
            // and bit 5 that it was given settings.
            ("flags", T::u8()),
            ("codec_id", codec_id()),
            (
                // Two counts, not streams: `streams` already means byte runs
                // at the front of the file and a whole `StreamsInfo` in the
                // header, and a third meaning on one row would be one too
                // many.
                "stream_counts",
                T::switch(
                    E::field("flags").bit(4),
                    vec![(
                        1,
                        T::inline_structure(
                            "CoderStreamCounts",
                            vec![("num_in_streams", number()), ("num_out_streams", number())],
                        ),
                    )],
                    // One in, one out, written nowhere: the ordinary coder is
                    // the common case and 7z spends no bytes saying so. Read
                    // as fields all the same, so that the arithmetic below has
                    // the same two names to add up whichever kind it is.
                    T::inline_structure(
                        "CoderStreamCounts",
                        vec![
                            ("num_in_streams", T::computed(E::lit(1))),
                            ("num_out_streams", T::computed(E::lit(1))),
                        ],
                    ),
                ),
            ),
            (
                "properties",
                T::switch(
                    E::field("flags").bit(5),
                    vec![(
                        1,
                        T::inline_structure(
                            "CoderProperties",
                            vec![
                                ("properties_size", number()),
                                ("settings", T::sized(E::field("properties_size"), coder_settings())),
                            ],
                        ),
                    )],
                    nothing(),
                ),
            ),
            // How many streams the coders up to and including this one take
            // and give. A folder says how many bind pairs and packed streams
            // it has by not saying: they are worked out from these totals, and
            // nothing can add a column up across records, so each record
            // carries the running total instead.
            (
                "in_streams_so_far",
                T::computed(E::prev("in_streams_so_far").add(E::within(&["stream_counts", "num_in_streams"]))),
            ),
            (
                "out_streams_so_far",
                T::computed(E::prev("out_streams_so_far").add(E::within(&["stream_counts", "num_out_streams"]))),
            ),
            // The same codec as the row above, as a number that is always
            // there. `codec_id` is bytes when the flags nibble names a width
            // the format cannot write, and a run of bytes holds no number to
            // switch on; a switch whose subject will not resolve is an error
            // rather than a default, so anything *choosing* on the codec has
            // to ask something that always answers. Minus one for a width
            // that is no width, which is no codec's id.
            //
            // This is the wire from a folder to the bytes it packed: the
            // stream at the front of the file switches on it. See
            // [`packed_run`].
            (
                "codec_number",
                T::switch(
                    E::lit(0).less_than(id_size()).mul(id_size().less_than(E::lit(5))),
                    vec![(1, T::computed(E::field("codec_id")))],
                    T::computed(E::lit(-1)),
                ),
            ),
        ],
    )
    .counted_as("coder")
    .machinery(&["in_streams_so_far", "out_streams_so_far", "codec_number"])
}

/// A pipeline of coders and the wiring between them, unpacking to one stream.
/// One folder is one solid block: everything in it has to be unpacked to get
/// at anything in it.
fn folder() -> T {
    let last_coder = |field: &str| E::elem_field("coders", E::field("num_coders").sub(E::lit(1)), &[field]);
    // Every output but the last is wired to an input, and every input not
    // wired to an output is fed by a packed stream. Neither count is written
    // down; both fall out of the totals.
    let bind_pairs = last_coder("out_streams_so_far").sub(E::lit(1));
    let packed = last_coder("in_streams_so_far").sub(bind_pairs.clone());
    T::structure(
        "Folder",
        vec![
            ("num_coders", number()),
            ("coders", T::array(coder(), E::field("num_coders"))),
            (
                "bind_pairs",
                T::array(
                    T::inline_structure("BindPair", vec![("in_index", number()), ("out_index", number())])
                        .counted_as("pair"),
                    bind_pairs,
                ),
            ),
            // Which packed stream feeds which input, written only when there
            // is more than one and the order could be in doubt.
            (
                "packed_indices",
                T::switch(E::lit(1).less_than(packed.clone()), vec![(1, T::array(number(), packed.clone()))], nothing()),
            ),
            ("out_streams_so_far", T::computed(E::prev("out_streams_so_far").add(last_coder("out_streams_so_far")))),
            // How many of the archive's packed streams this folder and the
            // ones in front of it have taken between them. `kPackInfo` writes
            // one list of sizes for the whole archive and never says which
            // folder any of them belongs to; what says so is the order, and
            // this running total is where a folder's share of it ends.
            //
            // So a folder that takes one stream took the one numbered
            // `packed_so_far - 1`, and that is how a stream at the front of
            // the file finds the coder that packed it. See [`packed_run`].
            ("packed_so_far", T::computed(E::prev("packed_so_far").add(packed))),
        ],
    )
    .counted_as("folder")
    .machinery(&["out_streams_so_far", "packed_so_far"])
}

/// The folders, and how large each one comes out.
fn unpack_info() -> T {
    let num_folders = E::field("num_folders");
    // One size per output stream of every folder, in folder order, which is
    // the last folder's running total.
    let out_streams = E::elem_field("folders", num_folders.clone().sub(E::lit(1)), &["out_streams_so_far"]);
    T::structure(
        "UnPackInfo",
        vec![
            ("id", property_id()),
            ("folders_id", property_id()),
            ("num_folders", number()),
            ("external", T::u8()),
            (
                "folders",
                T::switch(
                    E::field("external"),
                    vec![(0, T::array(folder(), num_folders.clone()))],
                    // The folders are in one of the archive's own streams
                    // rather than here. Nothing writes this, and following it
                    // would mean unpacking a stream to find out how to unpack
                    // the streams.
                    T::inline_structure(
                        "ExternalFolders",
                        vec![("data_stream_index", number()), ("unparsed", unparsed())],
                    ),
                ),
            ),
            ("unpack_sizes_id", property_id()),
            (
                "unpack_sizes",
                T::switch(
                    num_folders.clone(),
                    vec![(0, T::array(number(), E::lit(0)))],
                    T::array(number(), out_streams),
                ),
            ),
            ("folder_crcs", tagged(0x0a, digests(num_folders))),
            ("end", T::if_room(property_id())),
        ],
    )
}

/// How the folders divide into the files that came out of them. A solid
/// archive is one folder holding every file, and this is what says which
/// stretch of it each file is.
fn substreams_info() -> T {
    let num_folders = E::within(&["unpack_info", "num_folders"]);
    T::structure(
        "SubStreamsInfo",
        vec![
            ("id", property_id()),
            ("substreams_per_folder_id", T::switch(E::peek(8, Big), vec![(0x0d, property_id())], nothing())),
            (
                // 7z calls this `kNumUnPackStream`, which reads as one number
                // and is one per folder. Named for what the list holds, since
                // the row above it keeps the format's own word in view.
                "substreams_per_folder",
                T::switch(
                    E::size_of("substreams_per_folder_id"),
                    // Written nowhere when every folder holds one file, which
                    // is what a non-solid archive is. Read as a one for each
                    // folder rather than left out, so that the sums below have
                    // a list to add up either way.
                    vec![(0, T::array(T::computed(E::lit(1)), num_folders.clone()))],
                    T::array(number(), num_folders.clone()),
                ),
            ),
            ("substream_sizes_id", T::switch(E::peek(8, Big), vec![(0x09, property_id())], nothing())),
            // The last file in a folder has no size written for it: it is
            // whatever is left of the folder. So a folder of `n` files spends
            // `n - 1` numbers here, and one of a single file spends none,
            // which is why an archive of one file per folder has no `kSize` at
            // all and this count comes to nought on its own.
            //
            // Not `unpack_sizes`, which the block above already uses for one
            // size per folder. These are one per file, and 7z means a
            // different thing by "unpack stream" in each of the two blocks.
            ("substream_sizes", T::array(number(), E::sum_of("substreams_per_folder").sub(num_folders))),
            ("crcs", tagged(0x0a, digests(E::sum_of("substreams_per_folder")))),
            ("end", T::if_room(property_id())),
        ],
    )
}

/// Where the archive's bytes are, what unpacks them, and how they divide.
/// The same three blocks whether they describe the archive's contents or the
/// one stream a compressed header is kept in.
///
/// `tagged_itself` is the difference between the two. Inside a header the run
/// is introduced by `kMainStreamsInfo` and that byte belongs to it; inside a
/// `kEncodedHeader` the tag has already been spent naming the header, and the
/// blocks start straight away.
fn streams_info(name: &str, tagged_itself: bool) -> T {
    let mut fields = Vec::new();
    if tagged_itself {
        fields.push(("id", property_id()));
    }
    fields.extend([
        ("pack_info", tagged(0x06, pack_info())),
        ("unpack_info", tagged(0x07, unpack_info())),
        ("substreams_info", tagged(0x08, substreams_info())),
        ("end", T::if_room(property_id())),
    ]);
    T::structure(name, fields)
}

/// A value written once per file, for the files that have one.
fn per_file(elem: T) -> T {
    T::structure(
        "PerFileValues",
        vec![
            ("all_defined", T::u8()),
            (
                "defined",
                T::switch(
                    E::field("all_defined"),
                    vec![(1, nothing())],
                    T::bytes(E::field("num_files").add(E::lit(7)).div(E::lit(8))),
                ),
            ),
            ("external", T::u8()),
            (
                "values",
                T::switch(
                    E::field("all_defined").mul(E::lit(1).sub(E::field("external"))),
                    vec![(1, T::array(elem, E::field("num_files")))],
                    // Either the values are in one of the archive's own
                    // streams, or how many there are is a count of set bits.
                    // The block says how long it is, so stopping here costs
                    // only this block.
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// The same, for `kCTime`, `kATime` and `kMTime`, which hold a Windows
/// FILETIME per file.
///
/// Declared on `values`, which is the array rather than one of its elements: a
/// declaration on a list is about what is in it. The property's own field is
/// unnamed here, since which of the three this is comes from the `id` byte and
/// its entry in the property table, so the array is the only place to say it.
fn per_file_filetime() -> T {
    per_file(T::u64(Little)).field_time("values", Time::filetime())
}

/// The names, one after another, as UTF-16 with a NUL between them. A
/// directory is written as a name with slashes in it and nothing else marks
/// it: what says an entry is a directory is that it has no stream.
fn names() -> T {
    T::structure(
        "Names",
        vec![
            ("external", T::u8()),
            (
                "names",
                T::switch(
                    E::field("external"),
                    vec![(
                        0,
                        T::repeat(
                            T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Utf16(Little)),
                            Until::End,
                        ),
                    )],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// What one of the per-file blocks holds. Every one of them says how long it
/// is, so a block this does not divide costs nothing but itself.
fn property_value() -> T {
    T::switch(
        E::field("id"),
        vec![
            // One bit per file, in file order, saying which entries have no
            // stream of their own: the directories, and the files of no bytes.
            (0x0e, T::array(T::UInt { bits: 1, endian: Big }, E::field("num_files"))),
            (0x11, names()),
            (0x12, per_file_filetime()),
            (0x13, per_file_filetime()),
            (0x14, per_file_filetime()),
            (0x15, per_file(T::u32(Little))),
        ],
        // `kEmptyFile` and `kAnti` hold a bit per empty stream rather than per
        // file, and how many of those there are is a count of set bits.
        // `kDummy` is padding an archiver put in to line the names up. Both
        // read as what they are: bytes.
        T::bytes(E::Remaining),
    )
}

/// One tagged block of the file table, with its length in front of it.
///
/// This is the one run in the header a reader can walk without understanding
/// it, because every block here says how long it is. That is why the file
/// table is a list and the stream blocks above are named fields: only these
/// can be stepped over.
fn file_property() -> T {
    T::structure_named(
        "FileProperty",
        "id",
        "body",
        vec![
            ("id", property_id()),
            (
                "body",
                T::switch(
                    E::field("id"),
                    vec![(0x00, nothing())],
                    T::inline_structure(
                        "PropertyBody",
                        vec![("size", number()), ("value", T::sized(E::field("size"), property_value()))],
                    ),
                ),
            ),
        ],
    )
    .counted_as("property")
}

/// The file table: how many entries there are, and then a block per column.
///
/// Not a record per file. 7z writes the names together, then the times
/// together, then the attributes, so a file is a position in several lists
/// rather than a run of bytes anywhere.
fn files_info() -> T {
    T::structure(
        "FilesInfo",
        vec![
            ("id", property_id()),
            ("num_files", number()),
            (
                "properties",
                T::repeat(file_property(), Until::FieldValue { field: "id".to_string(), value: 0 }),
            ),
        ],
    )
}

/// The header proper: what is in the archive, and what the archive holds.
fn header() -> T {
    T::structure(
        "Header",
        vec![
            ("id", property_id()),
            ("archive_properties", tagged(0x02, archive_properties())),
            ("main_streams", tagged(0x04, streams_info("MainStreamsInfo", true))),
            ("files_info", tagged(0x05, files_info())),
            ("end", T::if_room(property_id())),
            // Nought for a header this read all of. Anything else is a header
            // that stopped making sense partway, and saying so beats letting
            // the bytes go uncounted.
            ("unparsed", T::if_room(unparsed())),
        ],
    )
}

/// Whatever the archiver wanted to record about the archive as a whole. Each
/// one says how long it is, so they are walked without being understood.
fn archive_properties() -> T {
    T::structure(
        "ArchiveProperties",
        vec![
            ("id", property_id()),
            (
                "properties",
                T::repeat(
                    T::structure_named(
                        "ArchiveProperty",
                        "id",
                        "body",
                        vec![
                            ("id", property_id()),
                            (
                                "body",
                                T::switch(
                                    E::field("id"),
                                    vec![(0x00, nothing())],
                                    T::inline_structure(
                                        "ArchivePropertyBody",
                                        vec![("size", number()), ("value", T::bytes(E::field("size")))],
                                    ),
                                ),
                            ),
                        ],
                    )
                    .counted_as("property"),
                    Until::FieldValue { field: "id".to_string(), value: 0 },
                ),
            ),
        ],
    )
}

/// What the offset at the front of the file points at: the header, or a
/// description of the one stream the header was compressed into.
fn next_header() -> T {
    T::switch(
        E::peek(8, Big),
        vec![
            (0x01, header()),
            (
                0x17,
                T::structure(
                    "EncodedHeader",
                    vec![
                        ("id", property_id()),
                        ("streams", streams_info("StreamsInfo", false)),
                        ("unparsed", T::if_room(unparsed())),
                    ],
                ),
            ),
        ],
        // A tag that is neither is a file this cannot read past its first
        // byte. Read as a tag and a run all the same: which byte it is, is
        // the thing worth seeing.
        T::structure("UnknownHeader", vec![("id", property_id()), ("unparsed", T::if_room(unparsed()))]),
    )
}

/// The answers a `kPackInfo` would have given, for a header that has none: no
/// streams, and nothing in front of them.
///
/// A field of no bytes rather than a missing field, so that the three runs
/// below ask their questions once instead of once per way the header could
/// disappoint them.
fn no_pack_info() -> T {
    T::inline_structure(
        "NoPackInfo",
        vec![
            ("pack_pos", T::computed(E::lit(0))),
            ("num_pack_streams", T::computed(E::lit(0))),
            ("pack_sizes", T::array(number(), E::lit(0))),
        ],
    )
}

/// The same for a `kUnPackInfo`: no folders, and no sizes for them to come
/// out at.
///
/// Wanted for the same reason and by a stricter reader. A switch whose subject
/// will not resolve is an error and not a default, so a run that chooses what
/// it is by asking how many folders there are has to be able to ask that of a
/// header with no folder block at all.
fn no_unpack_info() -> T {
    T::inline_structure(
        "NoUnPackInfo",
        vec![
            ("num_folders", T::computed(E::lit(0))),
            ("folders", T::array(folder(), E::lit(0))),
            ("unpack_sizes", T::array(number(), E::lit(0))),
        ],
    )
}

/// Enough of the end header to place the packed streams, and to open them.
///
/// Read here, from inside the packed region, because that is where the answer
/// is wanted and an expression only ever reaches backwards: the header is the
/// archive's last field, and by the time it is declared these bytes have long
/// been placed. The same bytes are read again there, in full, and counted
/// there; this reading is put aside so that nothing counts them twice.
///
/// Every branch answers with a `kPackInfo` and a `kUnPackInfo` under those
/// names and with the tag the header started with, so the runs below ask one
/// question rather than one per kind. A header this cannot read the start of
/// answers `kEnd`, which no header begins with, so asking what kind is ahead
/// never has to be preceded by asking whether there is one; a header with no
/// folder block answers no folders, for the same reason and to a stricter
/// reader.
///
/// The `kUnPackInfo` is why the reading goes past the sizes at all. That is
/// where the folders are, and a folder is what says which codec packed a
/// stream and how it was set up: LZMA1 wrote down the three numbers its stream
/// does not carry, and every other codec is at least named. A packed stream
/// that cannot reach its folder cannot be opened, whether it holds the
/// archive's own header or the archive's files.
fn pack_info_ahead() -> T {
    let start = |name: &str, before: Vec<(&'static str, T)>| {
        let mut fields = vec![("id", property_id())];
        fields.extend(before);
        fields.push(("pack_info", pack_info_or_none()));
        fields.push(("unpack_info", unpack_info_or_none()));
        T::structure(name, fields)
    };
    let no_start = |name: &str| {
        T::structure(
            name,
            vec![("id", T::computed(E::lit(0))), ("pack_info", no_pack_info()), ("unpack_info", no_unpack_info())],
        )
    };
    T::switch(
        E::Remaining,
        vec![(0, no_start("NoHeader"))],
        T::switch(
            E::peek(8, Big),
            vec![
                (
                    0x01,
                    start(
                        "HeaderStart",
                        vec![
                            ("archive_properties", tagged(0x02, archive_properties())),
                            ("main_streams_id", T::if_room(property_id())),
                        ],
                    ),
                ),
                (0x17, start("EncodedHeaderStart", Vec::new())),
            ],
            no_start("UnknownHeaderStart"),
        ),
    )
}

/// A `kPackInfo` if the tag at the cursor says so, and zeros if not.
fn pack_info_or_none() -> T {
    T::switch(
        E::Remaining,
        vec![(0, no_pack_info())],
        T::switch(E::peek(8, Big), vec![(0x06, pack_info())], no_pack_info()),
    )
}

/// A `kUnPackInfo` if the tag at the cursor says so, and no folders if not.
fn unpack_info_or_none() -> T {
    T::switch(
        E::Remaining,
        vec![(0, no_unpack_info())],
        T::switch(E::peek(8, Big), vec![(0x07, unpack_info())], no_unpack_info()),
    )
}

/// One packed stream, as long as `kSize` said, opened as what the folder that
/// owns it unpacks to.
///
/// The stream is a folder's whole output: one file in an archive written a
/// file at a time, and every file of a solid one run together. Which files
/// those are and where each of them starts is in `kSubStreamsInfo`, in offsets
/// counted inside the unpacked run, and dividing it by them would mean a
/// template laid over one space in the numbering of another. So a solid folder
/// opens as one run, and the names beside it stay names. See the module doc.
///
/// **Which folder.** `kPackInfo` writes one list of sizes for the archive and
/// never says whose any of them is; the folders take them in order, and where
/// each folder's share ends is [`folder`]'s `packed_so_far`. A folder that
/// took one stream took the one numbered `packed_so_far - 1`, so a stream
/// belongs to the folder at its own index exactly when that folder took one
/// stream and ended its run there. Anything else stays bytes: a single-coder
/// folder standing behind a folder that took two cannot be found without a
/// search backwards through the list, which no expression here can say.
///
/// **Which codec.** One coder, and one this can run. A folder of more than one
/// coder is a filter chain, BCJ into LZMA2 or the four-way split of BCJ2, and
/// nothing here runs a chain: the second coder's input is the first's output
/// and there is no space to read it from. A folder whose one coder is AES is
/// the same answer for a different reason. Both stay bytes.
///
/// The guards are asked in order and each is a switch, which is why they are
/// nested rather than multiplied together: a switch that cannot work out its
/// subject is an error and not a default, so nothing may ask about a folder
/// before the question of whether there is one has been answered.
fn packed_run() -> T {
    let size = || E::elem_within(&["placed_by", "pack_info", "pack_sizes"], E::idx(), &[]);
    let bytes = || T::bytes(size());
    let folder = |field: &[&str]| E::elem_within(&["placed_by", "unpack_info", "folders"], E::idx(), field);
    let setting = |field: &str| folder(&["coders", "0", "properties", "settings", field]);
    let packed = |codec| T::decoded(size(), codec, super::decoded_text());
    // What LZMA1 does not carry. The size is the folder's own output size,
    // which is the last of its outputs and so the last of the run of sizes it
    // owns; a folder of one coder has one output and `out_streams_so_far` is
    // where it ends. Given rather than left open, because 7z writes no
    // end-of-stream marker.
    let unpacked =
        E::elem_within(&["placed_by", "unpack_info", "unpack_sizes"], folder(&["out_streams_so_far"]).sub(E::lit(1)), &[]);
    // The ids are 7-Zip's own, from `DOC/Methods.txt`. Every other codec, and
    // every filter, stays bytes: a coder this cannot run is a run of bytes
    // with a name on it, which is what it was before any of this.
    let by_codec = T::switch(
        folder(&["coders", "0", "codec_number"]),
        vec![
            (0x00, packed(Codec::Stored)),
            (0x21, packed(Codec::Lzma2)),
            (
                0x03_0101,
                T::decoded_as(
                    size(),
                    Packing::Lzma1 {
                        props: setting("props"),
                        dict_size: setting("dict_size"),
                        unpacked: Some(unpacked),
                    },
                    super::decoded_text(),
                ),
            ),
            (0x04_0108, packed(Codec::Deflate)),
            (0x04_0202, packed(Codec::Bzip2)),
            (0x04f7_1101, packed(Codec::Zstd)),
        ],
        bytes(),
    );
    // This folder's run of packed streams ends at this stream, so with the row
    // below saying it took exactly one, this stream is the one it took.
    let ends_here = T::switch(folder(&["packed_so_far"]).sub(E::idx()), vec![(1, by_codec)], bytes());
    let one_coder = T::switch(folder(&["num_coders"]), vec![(1, ends_here)], bytes());
    // And before any of that, whether there is a folder at this index at all.
    // A header this could not read the folders of answers no folders rather
    // than no field, so this asks once and the rows above never have to.
    T::switch(
        E::idx().less_than(E::within(&["placed_by", "unpack_info", "num_folders"])),
        vec![(1, one_coder)],
        bytes(),
    )
}

/// The stream a `kEncodedHeader` describes, opened as the header it is.
///
/// This is the one packed stream a 7z archive describes well enough to unpack
/// from out here. Raw LZMA1 carries no header of its own, so a decoder has to
/// be told the properties byte, the dictionary size and how much comes out;
/// the coder read by [`pack_info_ahead`] wrote the first two down and
/// `kCodersUnPackSize` the third. `unpacked` is given rather than left open on
/// purpose: 7z writes no end-of-stream marker, and a decoder told the size is
/// unknown reads on into the header bytes sitting behind the stream.
///
/// What comes out is a `kHeader`, so the space it opens reads as one, names
/// and all. An expression here that will not resolve leaves the run as the
/// bytes it is with the reason on it, which is what a header packed some way
/// this cannot work out should say.
fn encoded_header_stream() -> T {
    let coder = |field: &str| {
        E::within(&["placed_by", "unpack_info", "folders", "0", "coders", "0", "properties", "settings", field])
    };
    T::decoded_as(
        E::elem_within(&["placed_by", "pack_info", "pack_sizes"], E::idx(), &[]),
        Packing::Lzma1 {
            props: coder("props"),
            dict_size: coder("dict_size"),
            // The first output size of the first folder, which for the single
            // coder a header is packed with is the header itself.
            unpacked: Some(E::within(&["placed_by", "unpack_info", "unpack_sizes", "0"])),
        },
        header(),
    )
}

/// The compressed bytes at the front of the file, divided into the streams
/// `kPackInfo` says they are.
fn packed_streams() -> T {
    T::structure(
        "PackedStreams",
        vec![
            (
                "placed_by",
                T::at(
                    E::lit(START_HEADER).add(E::field("next_header_offset")),
                    T::sized(E::field("next_header_size"), pack_info_ahead()),
                ),
            ),
            // Nought for an archive whose header is not compressed. For one
            // whose header is, this is every file in the archive: the header
            // out here describes only the stream it was itself compressed
            // into, and puts that stream after everything else. Named for the
            // field that sizes it, so a reader can check the two against each
            // other.
            ("before_pack_pos", T::bytes(E::within(&["placed_by", "pack_info", "pack_pos"]))),
            // The first of them is the header itself when the header was
            // compressed, and it is opened as one. Every other stream is a
            // folder's, and opens as what that folder unpacks to: see
            // [`packed_run`], which is also where the streams this cannot
            // open stay bytes.
            (
                "streams",
                T::array(
                    T::switch(
                        E::idx(),
                        vec![(0, T::switch(E::within(&["placed_by", "id"]), vec![(0x17, encoded_header_stream())], packed_run()))],
                        packed_run(),
                    ),
                    E::within(&["placed_by", "pack_info", "num_pack_streams"]),
                ),
            ),
            // Room between the last stream and the header that no stream
            // claims. Nought in anything 7-Zip writes. Not `unclaimed`: the
            // run above is equally unclaimed, and a reader has to be able to
            // tell the two rows apart at a glance.
            ("after_streams", T::bytes(E::Remaining)),
        ],
    )
    .field_aside("placed_by")
}

pub fn sevenzip() -> Template {
    Template::new(
        "7z",
        T::structure(
            "SevenZipArchive",
            vec![
                ("magic", T::magic(MAGIC)),
                ("version_major", T::u8()),
                ("version_minor", T::u8()),
                // A CRC-32 of the twenty bytes after it, which is what says
                // the three numbers below can be trusted.
                ("start_header_crc", T::u32(Little)),
                // Where the header is, counted from the end of these
                // thirty-two bytes rather than from the start of the file.
                ("next_header_offset", T::u64(Little)),
                ("next_header_size", T::u64(Little)),
                ("next_header_crc", T::u32(Little)),
                // Everything the archive holds, as the archiver packed it,
                // divided by a reading of the header taken from in here. See
                // `packed_streams`.
                ("packed_streams", T::sized(E::field("next_header_offset"), T::if_room(packed_streams()))),
                // The header itself, in its own place at the end of the file,
                // where its bytes are counted. Everything in it was already
                // read above; this is the same walk, and the one a reader
                // scrolling down the file arrives at.
                ("next_header", T::sized(E::field("next_header_size"), T::if_room(next_header()))),
            ],
        )
        .field_aside("header_ahead")
        // The twenty bytes after it: the three numbers that say where the
        // header is, and nothing else. This is what tells a truncated archive
        // from a corrupt one, since a reader that cannot trust these cannot
        // trust anything it reaches through them.
        .field_check(
            "start_header_crc",
            Check::of(Checksum::Crc32, Covers::Run { at: E::lit(12), len: E::lit(20) }),
        )
        // The header as it sits at the end of the file, packed or not. Not
        // what it unpacks to: 7z sums the bytes it wrote, and the digest for
        // the unpacked header is the folder's own `kCRC` inside it.
        .field_check("next_header_crc", Check::of(Checksum::Crc32, Covers::Field { name: Named::here("next_header") })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, eval::Evaluator, eval::Value, source::MemSource};

    /// A number in the shortest form 7z writes it in: the count of bytes to
    /// follow as leading ones, whatever is left of the first byte as the top
    /// of the value, and the rest low byte first.
    fn num(v: u64) -> Vec<u8> {
        for extra in 0..8u32 {
            let bits = (7 - extra) + 8 * extra;
            if v < (1u64 << bits) {
                let mark = if extra == 0 { 0 } else { (0xffu16 << (8 - extra)) as u8 };
                let mut out = vec![mark | (v >> (8 * extra)) as u8];
                out.extend((0..extra).map(|i| (v >> (8 * i)) as u8));
                return out;
            }
        }
        let mut out = vec![0xff];
        out.extend_from_slice(&v.to_le_bytes());
        out
    }

    /// The thirty-two bytes at the front, then the packed bytes, then the
    /// header. The checksums are left at zero: nothing here reads them, and a
    /// test that had to keep them right would be testing CRC-32.
    fn archive(packed: &[u8], header: &[u8]) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&[0, 4]);
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(packed.len() as u64).to_le_bytes());
        v.extend_from_slice(&(header.len() as u64).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(packed);
        v.extend_from_slice(header);
        v
    }

    /// A name as `kName` writes it: UTF-16, little end first, and a NUL.
    fn utf16(name: &str) -> Vec<u8> {
        name.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
    }

    /// A header describing one stream per size, one folder each, with the
    /// names given. The shape 7-Zip writes for `-mhc=off -ms=off`.
    fn header(sizes: &[u64], names: &[&str]) -> Vec<u8> {
        header_with(sizes, names, &[])
    }

    /// The same, with `extra` file properties written before the names. Each is
    /// a tag and its body; the length in between is worked out here, the way
    /// every block of the file table carries its own.
    fn header_with(sizes: &[u64], names: &[&str], extra: &[(u8, Vec<u8>)]) -> Vec<u8> {
        // One coder, a one-byte codec id, and that id is `00`, which is store.
        // So each stream unpacks to itself, which is the shortest folder that
        // reads as anything at all.
        let folders: Vec<Vec<u8>> = sizes.iter().map(|_| vec![0x01, 0x01, 0x00]).collect();
        header_of(sizes, &folders, sizes, names, extra)
    }

    /// A plain header, with each part written out rather than worked out from
    /// the others: a size per packed stream, a folder per folder, a size per
    /// folder output, and the names.
    ///
    /// A folder is given whole, since what varies between the archives tested
    /// here is inside one: how many coders it runs, which they are, and how
    /// they are wired. `[0x01, 0x01, 0x00]` is the plainest of them.
    fn header_of(
        pack_sizes: &[u64],
        folders: &[Vec<u8>],
        unpack_sizes: &[u64],
        names: &[&str],
        extra: &[(u8, Vec<u8>)],
    ) -> Vec<u8> {
        let mut h = vec![0x01, 0x04, 0x06];
        h.extend(num(0));
        h.extend(num(pack_sizes.len() as u64));
        h.push(0x09);
        for s in pack_sizes {
            h.extend(num(*s));
        }
        h.push(0x00);
        h.extend([0x07, 0x0b]);
        h.extend(num(folders.len() as u64));
        h.push(0x00);
        for f in folders {
            h.extend_from_slice(f);
        }
        h.push(0x0c);
        for s in unpack_sizes {
            h.extend(num(*s));
        }
        h.extend([0x00, 0x00]);
        h.push(0x05);
        h.extend(num(names.len() as u64));
        for (tag, body) in extra {
            h.push(*tag);
            h.extend(num(body.len() as u64));
            h.extend(body);
        }
        h.push(0x11);
        let mut body = vec![0x00];
        for n in names {
            body.extend(utf16(n));
        }
        h.extend(num(body.len() as u64));
        h.extend(body);
        h.extend([0x00, 0x00]);
        h
    }

    /// The same header, with a `kCRC` block inside `kUnPackInfo` whose
    /// `AllAreDefined` byte is zero: a bit vector saying which folders have a
    /// checksum, and then one checksum for each bit that is set.
    fn header_with_sparse_crcs(sizes: &[u64], names: &[&str], defined: &[bool]) -> Vec<u8> {
        let whole = header_with(sizes, names, &[]);
        // `kUnPackInfo` ends with the two zero bytes before `kFilesInfo`.
        let at = whole.windows(3).position(|w| w == [0x00, 0x00, 0x05]).expect("end of kUnPackInfo");
        let mut bits = vec![0u8; defined.len().div_ceil(8)];
        for (i, on) in defined.iter().enumerate() {
            if *on {
                bits[i / 8] |= 0x80 >> (i % 8);
            }
        }
        let mut block = vec![0x0a, 0x00];
        block.extend(bits);
        for (i, on) in defined.iter().enumerate() {
            if *on {
                block.extend((0x1000_0000u32 + i as u32).to_le_bytes());
            }
        }
        let mut out = whole[..at].to_vec();
        out.extend(block);
        out.extend_from_slice(&whole[at..]);
        out
    }

    /// A checksum for some of the folders and not others. How many follow is a
    /// count of the bits set in the vector, which nothing in the file writes
    /// down; before there was an expression for it the block was read as far
    /// as the vector and the rest of the header was given up on.
    #[test]
    fn a_sparse_digest_block_has_a_checksum_for_each_bit_that_is_set() {
        let sizes = [4u64, 4, 4];
        let bytes = archive(b"aaaabbbbcccc", &header_with_sparse_crcs(&sizes, &["a", "b", "c"], &[true, false, true]));
        let (d, mut e) = read(bytes);
        // The digest block of the unpack info: a bit vector, then a checksum
        // for each bit set in it.
        let sparse = [8usize, 2, 2, 7, 2];
        assert_eq!(e.node(&d, &sparse).unwrap().type_name, "SparseDigests");
        let crcs = [8usize, 2, 2, 7, 2, 1];
        assert_eq!(e.node(&d, &crcs).unwrap().child_count, 2, "two bits set, two checksums");
        assert_eq!(e.node(&d, &[8, 2, 2, 7, 2, 1, 0]).unwrap().value, Value::UInt(0x1000_0000));
        assert_eq!(e.node(&d, &[8, 2, 2, 7, 2, 1, 1]).unwrap().value, Value::UInt(0x1000_0002));
        // And the header past the block still reads, so the walk stepped over
        // exactly the bytes the count accounted for.
        // And nothing is left over: the walk stepped past the block over
        // exactly the bytes the count accounted for, so the file table after
        // it read as a file table rather than as the rest of the header.
        let over = e.child_named(&d, &[8], "unparsed").unwrap().expect("unparsed");
        assert_eq!(e.node(&d, &over).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[8, 3, 1]).unwrap().value.as_int(), Some(3), "three files, read past the digests");
    }

    fn read(bytes: Vec<u8>) -> (Document<MemSource>, Evaluator) {
        (Document::new(MemSource(bytes)), Evaluator::new(sevenzip()))
    }

    #[test]
    fn the_front_places_the_packed_streams_and_the_header_after_them() {
        let (d, mut e) = read(archive(b"packed bytes", &header(&[12], &["one"])));
        assert_eq!(e.node(&d, &[7]).unwrap().offset_bits, 32 * 8);
        assert_eq!(e.node(&d, &[7]).unwrap().size_bits, 12 * 8);
        // The header is the last field and starts where the packed streams
        // stop, so the three parts tile the whole file between them.
        let header = e.node(&d, &[8]).unwrap();
        assert_eq!(header.offset_bits, 44 * 8);
        assert_eq!(e.node(&d, &[]).unwrap().size_bits, (44 + header.size_bits / 8) * 8);
    }

    /// The header is read twice: once from inside the packed region, which
    /// needs it before its streams can be placed, and once in its own place at
    /// the end. Only the second is counted, or the archive would measure
    /// longer than it is.
    #[test]
    fn the_header_is_read_from_the_front_and_counted_at_the_back() {
        let (d, mut e) = read(archive(b"packed bytes", &header(&[12], &["one"])));
        assert_eq!(e.node(&d, &[7, 0]).unwrap().size_bits, 0, "reading ahead takes no room where it is declared");
        // Both readings land on the same bytes and say the same thing.
        assert_eq!(e.node(&d, &[7, 0, 0]).unwrap().offset_bits, e.node(&d, &[8]).unwrap().offset_bits);
        assert_eq!(e.node(&d, &[7, 0, 0, 0]).unwrap().value, e.node(&d, &[8, 0]).unwrap().value);
    }

    /// An archive with nothing in it: the header sits straight after the
    /// front, because there are no packed streams to skip.
    #[test]
    fn an_empty_archive_has_no_packed_streams_at_all() {
        let (d, mut e) = read(archive(b"", b""));
        assert_eq!(e.node(&d, &[7]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[8]).unwrap().offset_bits, 32 * 8);
    }

    /// The one number format in here that nothing else reads. `81 9f` is 415:
    /// the count of bytes to follow is unary from the top of the first byte,
    /// the bytes after it are the bottom of the value, and the bits left in
    /// the first byte sit above them. Read as a LEB128 it is 4001, as an EBML
    /// size 415 by accident and 0x19f nowhere.
    #[test]
    fn a_number_keeps_its_high_bits_in_the_byte_that_counts_the_rest() {
        for value in [0u64, 1, 127, 128, 205, 415, 3891, 0xffff, 0x1234_5678, u64::MAX] {
            let written = num(value);
            let (d, mut e) = read(archive(b"", &header(&[value], &["x"])));
            // The first pack size, which is where the number under test went.
            let node = e.node(&d, &[8, 2, 1, 4, 0]).unwrap();
            assert_eq!(node.value, Value::UInt(value as u128), "{value} written as {written:02x?}");
            assert_eq!(node.size_bits, written.len() as u64 * 8, "{value} is {} bytes", written.len());
        }
    }

    /// What the whole exercise is for: the front of the file stops being one
    /// blob and becomes the streams the archiver actually wrote, each at its
    /// own offset and its own length.
    #[test]
    fn the_pack_info_divides_the_front_of_the_file_into_streams() {
        let packed = vec![0u8; 3 + 5 + 400];
        let (d, mut e) = read(archive(&packed, &header(&[3, 5, 400], &["a", "b", "c"])));
        assert_eq!(e.node(&d, &[7, 2]).unwrap().child_count, 3);
        let at = |e: &mut Evaluator, i: usize| {
            let n = e.node(&d, &[7, 2, i]).unwrap();
            (n.offset_bits / 8, n.size_bits / 8)
        };
        assert_eq!(at(&mut e, 0), (32, 3));
        assert_eq!(at(&mut e, 1), (35, 5));
        assert_eq!(at(&mut e, 2), (40, 400));
        // Nothing before the first stream and nothing after the last: the
        // streams tile the whole of the packed region.
        assert_eq!(e.node(&d, &[7, 1]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[7, 3]).unwrap().size_bits, 0);
    }

    #[test]
    fn the_file_table_reads_every_name_as_the_text_it_is() {
        let names = ["docs", "docs/chapter one.md", "\u{e9}t\u{e9}.txt"];
        let (d, mut e) = read(archive(b"abc", &header(&[3], &names)));
        // files_info, its properties, the first of them, its body, its value,
        // and the names inside that.
        assert_eq!(e.node(&d, &[8, 3, 2, 0, 1, 1, 1]).unwrap().child_count, 3);
        for (i, want) in names.iter().enumerate() {
            let node = e.node(&d, &[8, 3, 2, 0, 1, 1, 1, i]).unwrap();
            assert_eq!(node.value, Value::Str((*want).into()));
            // Two bytes a character, and the NUL that ends it.
            assert_eq!(node.size_bits / 8, want.encode_utf16().count() as u64 * 2 + 2);
        }
    }

    /// A coder that says nothing about its streams takes one in and gives one
    /// out, and a folder of one such coder has no bind pairs and no list of
    /// packed stream indices. None of those numbers is written in the file.
    #[test]
    fn a_plain_coder_takes_one_stream_in_and_gives_one_out() {
        let (d, mut e) = read(archive(b"abc", &header(&[3], &["a"])));
        let folder = [8usize, 2, 2, 4, 0];
        let streams = [folder.as_slice(), &[1, 0, 2]].concat();
        assert_eq!(e.node(&d, &[streams.as_slice(), &[0]].concat()).unwrap().value, Value::Int(1));
        assert_eq!(e.node(&d, &[streams.as_slice(), &[1]].concat()).unwrap().value, Value::Int(1));
        // Worked out rather than read, so they cost no bytes.
        assert_eq!(e.node(&d, &streams).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[folder.as_slice(), &[2]].concat()).unwrap().child_count, 0);
        assert_eq!(e.node(&d, &[folder.as_slice(), &[3]].concat()).unwrap().size_bits, 0);
    }

    /// A header 7z compressed into a stream of its own. The archive out here
    /// can still say where that stream is, and says so about nothing else: the
    /// files are in the run before it, which only the compressed header
    /// describes.
    /// A `kEncodedHeader` describing one stream of twenty bytes, a hundred
    /// bytes into the packed region, unpacking to three hundred. The shape
    /// 7-Zip writes when it compresses its own header, with numbers small
    /// enough to check by eye; the stream itself is zeros, so nothing here
    /// unpacks.
    fn encoded_header() -> Vec<u8> {
        let mut h = vec![0x17, 0x06];
        h.extend(num(100));
        h.extend(num(1));
        h.push(0x09);
        h.extend(num(20));
        h.extend([0x00, 0x07, 0x0b]);
        h.extend(num(1));
        h.push(0x00);
        // One LZMA1 coder with five bytes of settings, which is what 7-Zip
        // compresses a header with.
        h.extend([0x01, 0x23, 0x03, 0x01, 0x01, 0x05, 0x5d, 0x00, 0x10, 0x00, 0x00]);
        h.push(0x0c);
        h.extend(num(300));
        h.extend([0x00, 0x00]);
        h
    }

    #[test]
    fn a_compressed_header_places_its_own_stream_and_no_others() {
        let (d, mut e) = read(archive(&vec![0u8; 120], &encoded_header()));
        let id = e.node(&d, &[8, 0]).unwrap();
        assert_eq!(id.value, Value::Enum { raw: 0x17, name: Some("kEncodedHeader".into()), hex: true });
        // The hundred bytes before the header's own stream are named as bytes
        // this cannot divide, rather than divided wrongly.
        assert_eq!(e.node(&d, &[7, 1]).unwrap().size_bits, 100 * 8);
        let stream = e.node(&d, &[7, 2, 0]).unwrap();
        assert_eq!((stream.offset_bits / 8, stream.size_bits / 8), (132, 20));
        assert_eq!(e.node(&d, &[7, 3]).unwrap().size_bits, 0);
    }

    /// A header whose first byte is neither kind still reads as far as that
    /// byte, and the packed bytes stay one run rather than becoming an error.
    #[test]
    fn a_header_of_an_unknown_kind_leaves_the_packed_bytes_whole() {
        let (d, mut e) = read(archive(b"packed bytes", &[0x42, 0x99, 0x99]));
        assert_eq!(e.node(&d, &[8, 0]).unwrap().value, Value::Enum { raw: 0x42, name: None, hex: true });
        assert_eq!(e.node(&d, &[8, 1]).unwrap().size_bits, 2 * 8);
        // The packed region is as long as the front said either way, so the
        // field standing for it proves nothing on its own. What proves the
        // fallback ran is that its runs are there and tile it: no room in
        // front of the streams, no streams, and one run holding the lot.
        assert_eq!(e.node(&d, &[7]).unwrap().size_bits, 12 * 8);
        assert_eq!(e.node(&d, &[7, 1]).unwrap().size_bits, 0);
        assert_eq!(e.node(&d, &[7, 2]).unwrap().child_count, 0);
        assert_eq!(e.node(&d, &[7, 3]).unwrap().size_bits, 12 * 8);
    }

    /// The codec id is as many bytes as the flags nibble says, read as one
    /// number whichever that was. A store coder writes it in one byte and LZMA
    /// in three, and the two answer from the same list.
    #[test]
    fn a_coder_says_which_codec_it_runs() {
        let (d, mut e) = read(archive(b"abc", &header(&[3], &["a"])));
        let stored = e.node(&d, &[8, 2, 2, 4, 0, 1, 0, 1]).unwrap();
        assert_eq!(stored.value, Value::Enum { raw: 0x00, name: Some("Copy".into()), hex: true });
        assert_eq!(stored.size_bits, 8, "one byte, because the flags nibble said one");

        let (d, mut e) = read(archive(&vec![0u8; 120], &encoded_header()));
        let lzma = e.node(&d, &[8, 1, 1, 4, 0, 1, 0, 1]).unwrap();
        assert_eq!(lzma.value, Value::Enum { raw: 0x03_0101, name: Some("LZMA".into()), hex: true });
        assert_eq!(lzma.size_bits, 3 * 8, "three bytes, and the same number a one-byte id would be");
    }

    /// The reading taken from inside the packed region goes as far as the
    /// folders, whichever kind of header is ahead, so the coder that packed a
    /// stream is in scope where that stream is placed. Reaching it is what
    /// lets the stream be opened at all.
    #[test]
    fn the_reading_ahead_finds_the_coders_the_streams_were_packed_with() {
        let (d, mut e) = read(archive(&vec![0u8; 120], &encoded_header()));
        // packed_streams, the reading ahead, what it read, its unpack info.
        let props = [7usize, 0, 0, 2, 4, 0, 1, 0, 3, 1, 0];
        assert_eq!(e.node(&d, &props).unwrap().value, Value::UInt(0x5d));
        // And it still costs nothing where it stands: these bytes are counted
        // at the end of the file, where the header actually is.
        assert_eq!(e.node(&d, &[7, 0]).unwrap().size_bits, 0);
        // A plain header is read as far as the same block, one field further
        // along because the tag naming the main streams is spent on the way.
        let (d, mut e) = read(archive(b"packed bytes", &header(&[12], &["one"])));
        assert_eq!(e.node(&d, &[7, 0, 0, 0]).unwrap().value.as_int(), Some(0x01));
        let codec = [7usize, 0, 0, 4, 4, 0, 1, 0, 6];
        assert_eq!(e.node(&d, &codec).unwrap().value, Value::Int(0x00), "the store coder, as a number to switch on");
        assert_eq!(e.node(&d, &[7, 0]).unwrap().size_bits, 0, "and still counted at the back");
    }

    /// A compressed header packed some way this cannot work out stays the
    /// bytes it is. The coder here is `copy`, which writes no properties at
    /// all, so the three numbers LZMA would need are not there to be found.
    #[test]
    fn a_compressed_header_this_cannot_unpack_stays_bytes() {
        let mut h = vec![0x17, 0x06];
        h.extend(num(100));
        h.extend(num(1));
        h.push(0x09);
        h.extend(num(20));
        h.extend([0x00, 0x07, 0x0b]);
        h.extend(num(1));
        // One coder, a one-byte id, and that id is `00`: no settings follow.
        h.extend([0x00, 0x01, 0x01, 0x00, 0x0c]);
        h.extend(num(300));
        h.extend([0x00, 0x00]);
        let (d, mut e) = read(archive(&vec![0u8; 120], &h));
        let stream = e.node(&d, &[7, 2, 0]).expect("the stream is still a field");
        assert_eq!(stream.size_bits, 20 * 8, "as long as kSize said, opened or not");
        assert_eq!(e.open_space(&d, 0, &[7, 2, 0]).expect("an answer, not an error"), None);
    }

    /// The five bytes an LZMA coder writes are five bytes of settings, and
    /// what they say is reachable rather than shown as a blob. The properties
    /// byte is three numbers got by dividing, not by masking.
    #[test]
    fn an_lzma_coder_says_how_it_packed_the_stream() {
        let (d, mut e) = read(archive(&vec![0u8; 120], &encoded_header()));
        let settings = [8usize, 1, 1, 4, 0, 1, 0, 3, 1];
        let at = |e: &mut Evaluator, i: usize| e.node(&d, &[settings.as_slice(), &[i]].concat()).unwrap().value;
        assert_eq!(at(&mut e, 0), Value::UInt(0x5d));
        assert_eq!(at(&mut e, 1), Value::Int(3), "literal context bits");
        assert_eq!(at(&mut e, 2), Value::Int(0), "literal position bits");
        assert_eq!(at(&mut e, 3), Value::Int(2), "position bits");
        // `00 10 00 00`, low byte first: four kibibytes, which is all the
        // dictionary a header of a few hundred bytes can use.
        assert_eq!(at(&mut e, 4), Value::UInt(4096));
        // The three worked-out numbers cost no bytes, and the block is the
        // five the coder said it was.
        let block = e.node(&d, &settings).unwrap();
        assert_eq!(block.size_bits, 5 * 8);
        assert_eq!(e.node(&d, &[settings.as_slice(), &[5]].concat()).unwrap().size_bits, 0, "nothing left over");
    }

    /// A codec whose settings nothing here reads keeps them as the bytes they
    /// are, and the walk steps over exactly as many as the block said.
    #[test]
    fn a_codec_this_does_not_know_keeps_its_settings_whole() {
        // A one-byte id of 0xfe, which is no codec, with three bytes of
        // settings behind it.
        let mut h = vec![0x01, 0x04, 0x06];
        h.extend([num(0), num(1)].concat());
        h.push(0x09);
        h.extend(num(4));
        h.extend([0x00, 0x07, 0x0b]);
        h.extend(num(1));
        h.extend([0x00, 0x01, 0x21, 0xfe, 0x03, 0xaa, 0xbb, 0xcc, 0x0c]);
        h.extend(num(4));
        h.extend([0x00, 0x00, 0x05]);
        h.extend(num(1));
        h.push(0x11);
        let body = [vec![0x00], utf16("kept")].concat();
        h.extend(num(body.len() as u64));
        h.extend(body);
        h.extend([0x00, 0x00]);
        let (d, mut e) = read(archive(b"abcd", &h));
        let settings = [8usize, 2, 2, 4, 0, 1, 0, 3, 1];
        assert_eq!(e.node(&d, &settings).unwrap().size_bits, 3 * 8);
        // And the file table past it still reads, so nothing was miscounted.
        assert_eq!(e.node(&d, &[8, 3, 2, 0, 1, 1, 1, 0]).unwrap().value, Value::Str("kept".into()));
    }

    /// A folder packed with `copy` opens, and what comes out is what went in.
    /// Nothing is decompressed, and that is the point: a stored run is a
    /// document where it sits, and saying so with a codec is what lets a
    /// reader be sent to it.
    #[test]
    fn a_stored_folder_opens_as_the_bytes_it_already_is() {
        let (d, mut e) = read(archive(b"aaabbbbbcc", &header(&[3, 5, 2], &["a", "b", "c"])));
        for (i, want) in [&b"aaa"[..], b"bbbbb", b"cc"].iter().enumerate() {
            let id = e.open_space(&d, 0, &[7, 2, i]).expect("no error").expect("a stored stream opens");
            assert_eq!(e.space(id).expect("it is there").bytes(), *want);
        }
    }

    /// The one coder whose settings the folder has to carry. A raw LZMA1
    /// stream says none of how it was packed, so the properties byte and the
    /// dictionary size come from the coder and the size that comes out from
    /// `kCodersUnPackSize`, all three of them read where the stream is placed.
    /// Two of them, because these are the only settings read per stream rather
    /// than fixed by the template, and a second folder is what says they are
    /// read at the stream asking. At the first stream an expression that had
    /// lost its place would answer the same as one that had not.
    #[test]
    fn an_lzma_folder_opens_with_the_settings_its_coder_wrote_down() {
        let texts: [Vec<u8>; 2] = [
            b"a folder packed with LZMA1, which carries none of how it was packed. ".repeat(8),
            b"the second folder, packed on its own, with its own coder to say so. ".repeat(3),
        ];
        let (mut folders, mut packed, mut pack_sizes, mut unpack_sizes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for text in &texts {
            let mut alone = Vec::new();
            lzma_rs::lzma_compress(&mut &text[..], &mut alone).expect("packs");
            // What `lzma` writes in front of the stream is what 7z writes into
            // the coder instead: the properties byte and the dictionary size,
            // and then eight bytes of unpacked size, which 7z keeps in
            // `kCodersUnPackSize` rather than here.
            let mut folder = vec![0x01, 0x23, 0x03, 0x01, 0x01, 0x05, alone[0]];
            folder.extend_from_slice(&alone[1..5]);
            folders.push(folder);
            pack_sizes.push((alone.len() - 13) as u64);
            unpack_sizes.push(text.len() as u64);
            packed.extend_from_slice(&alone[13..]);
        }
        let header = header_of(&pack_sizes, &folders, &unpack_sizes, &["first.txt", "second.txt"], &[]);
        let (d, mut e) = read(archive(&packed, &header));
        for (i, text) in texts.iter().enumerate() {
            let id = e.open_space(&d, 0, &[7, 2, i]).expect("no error").unwrap_or_else(|| panic!("folder {i} opens"));
            assert_eq!(e.space(id).expect("it is there").bytes(), &text[..], "folder {i}");
        }
    }

    /// The codecs whose settings are a property of the format rather than of
    /// the file. Nothing is read out of the coder for these: the id is the
    /// whole of what the header has to say, and getting it wrong is the one
    /// way this can go wrong quietly. The ids are 7-Zip's own.
    #[test]
    fn a_folder_opens_under_whichever_codec_its_id_names() {
        let text = b"the same bytes, packed three ways, and the id is what tells them apart. ".repeat(6);
        let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::new(9));
        std::io::Write::write_all(&mut bz, &text).expect("packs");
        let cases: [(&str, Vec<u8>, Vec<u8>); 3] = [
            ("Copy", vec![0x01, 0x01, 0x00], text.clone()),
            ("Deflate", vec![0x01, 0x03, 0x04, 0x01, 0x08], miniz_oxide::deflate::compress_to_vec(&text, 6)),
            ("BZip2", vec![0x01, 0x03, 0x04, 0x02, 0x02], bz.finish().expect("packs")),
        ];
        for (name, folder, packed) in cases {
            let header = header_of(&[packed.len() as u64], &[folder], &[text.len() as u64], &["packed"], &[]);
            let (d, mut e) = read(archive(&packed, &header));
            let id = e.open_space(&d, 0, &[7, 2, 0]).expect("no error").unwrap_or_else(|| panic!("{name} opens"));
            assert_eq!(e.space(id).expect("it is there").bytes(), &text[..], "{name}");
        }
    }

    /// A folder of more than one coder is a filter chain, and nothing here
    /// runs one: the second coder's input is the first's output, which is not
    /// a run of the file anything can be pointed at. The bytes stay bytes.
    #[test]
    fn a_folder_of_two_coders_stays_bytes() {
        // Two store coders, one wired into the other: the second's input,
        // which is input 1, is fed by the first's output, which is output 0.
        // That leaves one input unwired, so the folder still takes one packed
        // stream and one size is written per output.
        let folder = vec![0x02, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00];
        let header = header_of(&[4], &[folder], &[4, 4], &["chained"], &[]);
        let (d, mut e) = read(archive(b"abcd", &header));
        let node = e.node(&d, &[7, 2, 0]).expect("the stream is still a field");
        assert_eq!(node.size_bits, 4 * 8, "as long as kSize said, opened or not");
        assert!(!node.decoded, "a chain is not something this can run");
    }

    /// A folder standing behind one that took two packed streams stays bytes,
    /// even though it is a plain folder this could otherwise open.
    ///
    /// This is the guard that matters. A stream is the folder at its own index
    /// only while every folder in front has taken one stream each, and after a
    /// folder that took two the numbering has slipped: stream 1 belongs to
    /// folder 0 and folder 1's own stream is number 2. Finding that out from
    /// here would mean searching the folders backwards for the one whose run
    /// covers this index, which no expression can say, so the answer is bytes
    /// rather than the wrong file.
    #[test]
    fn a_folder_behind_one_that_took_two_streams_stays_bytes() {
        // A coder that says its stream counts out loud: two in, one out. No
        // output is bound to an input, so both inputs are fed by packed
        // streams and the folder writes down which is which.
        let wide = vec![0x01, 0x11, 0x00, 0x02, 0x01, 0x00, 0x01];
        let header = header_of(&[2, 3, 4], &[wide, vec![0x01, 0x01, 0x00]], &[5, 4], &["wide", "plain"], &[]);
        let (d, mut e) = read(archive(b"aabbbcccc", &header));
        // The second folder is one store coder and nothing else, and it is
        // still not opened: what it cannot be told is which stream is its.
        assert_eq!(e.node(&d, &[8, 2, 2, 4, 1, 0]).expect("num_coders").value.as_int(), Some(1));
        for i in 0..3 {
            assert!(!e.node(&d, &[7, 2, i]).expect("a field").decoded, "stream {i}");
        }
    }

    /// A stream no folder claims stays bytes. Which folder owns a stream is
    /// the order they were written in and nothing else, so a stream past the
    /// last folder belongs to nobody and is not the last folder's.
    #[test]
    fn a_stream_past_the_last_folder_stays_bytes() {
        let header = header_of(&[3, 4], &[vec![0x01, 0x01, 0x00]], &[3], &["only"], &[]);
        let (d, mut e) = read(archive(b"abcdefg", &header));
        let first = e.node(&d, &[7, 2, 0]).expect("a field");
        assert!(first.decoded, "the one folder there is owns the first stream");
        let second = e.node(&d, &[7, 2, 1]).expect("a field");
        assert_eq!(second.size_bits, 4 * 8);
        assert!(!second.decoded, "and nothing owns the second");
    }

    /// `kEmptyStream` is a bit per file, and a count of files that is not a
    /// whole number of bytes leaves a tail of bits belonging to nothing. The
    /// bits that are there are read; the rest of the byte is a gap, not a
    /// fourth entry.
    #[test]
    fn a_bit_per_file_stops_at_the_number_of_files() {
        // Three files, of which the first and the third have no stream.
        let empty = (0x0e, vec![0b1010_0000]);
        let (d, mut e) = read(archive(b"abc", &header_with(&[3], &["dir", "f.txt", "also"], &[empty])));
        let bits = e.node(&d, &[8, 3, 2, 0, 1, 1]).unwrap();
        assert_eq!(bits.child_count, 3, "one bit a file, and no more");
        // The block is a byte long, which is what the file said, so the five
        // bits after the three are room the format spends and nothing claims.
        assert_eq!(bits.size_bits, 8);
        let bit = |e: &mut Evaluator, i: usize| e.node(&d, &[8, 3, 2, 0, 1, 1, i]).unwrap().value;
        assert_eq!(bit(&mut e, 0), Value::UInt(1));
        assert_eq!(bit(&mut e, 1), Value::UInt(0));
        assert_eq!(bit(&mut e, 2), Value::UInt(1));
        // The names still read, so the walk stepped over the block correctly.
        assert_eq!(e.node(&d, &[8, 3, 2, 1, 1, 1, 1, 1]).unwrap().value, Value::Str("f.txt".into()));
    }
}
