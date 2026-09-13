//! Where the chunks of an HDF5 dataset are, when the index that says so is
//! one of the four arrays a version 4 data layout message can name.
//!
//! A chunked dataset of layout version 1, 2 or 3 keeps its chunks in a version
//! 1 b-tree, and that tree is read in [`hdf5`](super::hdf5) beside the group
//! trees, since it is the same structure. Versions 4 and 5, which arrived with
//! HDF5 1.10 and are what a file written to the latest library version gets,
//! choose an index from the dataset's shape instead: a single chunk index for
//! one chunk covering the whole dataset, an implicit index for a dataset that
//! cannot grow and was never filtered, a fixed array for one that cannot grow,
//! an extensible array for one that can grow along one dimension, and a
//! version 2 b-tree for more than one. The b-tree is read where the group
//! b-tree is, in `hdf5`; the other four are here.
//!
//! What the arrays have in common, and what this module is about, is that
//! their shape is arithmetic rather than something written down. A fixed array
//! whose entries do not fit one page is paged, and a page is a power of two of
//! entries; an extensible array keeps a handful of entries in its index block
//! and the rest in data blocks and secondary blocks of doubling size, and how
//! many of each the index block points at is a base-2 logarithm of the array's
//! bounds. An implicit index is no index at all: the chunks are one run, and
//! how long a run is the dataspace rounded up a chunk at a time in every
//! dimension. Every one of those is a fold over numbers the messages hold, and
//! [`Expr::DivCeil`](crate::template::Expr::DivCeil) and
//! [`Expr::Log2`](crate::template::Expr::Log2) exist so a template can write
//! it. The formulas are the library's own, `H5EA__hdr_init` and
//! `H5FA__hdr_init`, and `chunk-indexes-large.h5` in the sample collection
//! checks every chunk they place against where h5py says it is.
//!
//! An entry of any of these arrays reaches a chunk, and an unfiltered chunk
//! reads as elements: the chunk dimensions multiplied together are how many
//! bytes one comes to, and the datatype says what one element is. A filtered
//! chunk keeps its bytes, and [`hdf5_chunk`](super::hdf5_chunk) says what they
//! hold.
//!
//! One thing here is not an index. [`datatype_copy`] is the datatype message
//! placed a second time inside the layout, so that a chunk asks a field above
//! it for its element size rather than searching back past every entry before
//! it to the message beside the layout. That search is what
//! `Expr::Sibling` does, and over a hundred thousand chunks it was the
//! difference between opening a dataset in seconds and in minutes.

use crate::template::{Endian::Little, Expr as E, Ty as T};

use super::hdf5::{addr, at_address, bit, elements, length, when, Described};

/// The datatype message, read a second time by a chunked layout and kept
/// there, so that a chunk asks a field above it rather than the message beside
/// the layout.
///
/// That is the difference between opening a dataset of a hundred thousand
/// chunks in a moment and in minutes. A sibling is looked for among the
/// earlier elements of every list the asking field sits in, innermost first,
/// and a chunk an index points at sits in that index's list of entries: asked
/// from there, the search passes every entry before this one on its way out
/// to the messages, and does it again for the next chunk.
///
/// The whole message rather than the three numbers a chunk of plain numbers
/// needs, because an element of a compound asks for its members and an
/// element of an array for its base type, and those are lists inside the
/// message. The bytes are the message's and counted there.
fn datatype_copy() -> T {
    T::at_origin(Described::beside_address(), T::Named("Datatype".into()))
}


/// Chunked, version 4. The dimensions are written in a width the message
/// declares rather than always in four bytes, and the last of them is the size
/// of an element, as it is in version 3: so the dimensions multiplied together
/// are the bytes one whole chunk comes to, which is what places an unfiltered
/// chunk whose size nothing else writes down.
///
/// Which index is used follows from the dataset rather than being chosen: one
/// chunk covering the whole thing is a single chunk index, dimensions that
/// cannot grow are a fixed array, one that can is an extensible array, and
/// more than one that can is a version 2 b-tree. A version 1 b-tree, which is
/// all a version 3 message could name, never appears here.
pub(super) fn chunked_v4() -> T {
    T::structure(
        "Chunked",
        vec![
            (
                "flags",
                T::flags(
                    "ChunkedFlags",
                    T::u8(),
                    &[(0, "partial edge chunks unfiltered"), (1, "single chunk filtered")],
                ),
            ),
            ("dimensionality", T::u8()),
            ("dimension_width", T::u8().counted_as("bytes")),
            (
                "chunk_dimensions",
                T::array(T::uint_expr(E::field("dimension_width").mul(E::lit(8)), Little), E::field("dimensionality")),
            ),
            ("index_type", chunk_index_type()),
            (
                "index",
                T::switch(
                    E::field("index_type"),
                    vec![
                        (1, single_chunk_index()),
                        // How many entries one page of the array's data block
                        // holds, as the number of bits it takes to count them.
                        (3, T::structure("FixedArrayIndex", vec![("page_bits", T::u8())])),
                        (
                            4,
                            T::structure(
                                "ExtensibleArrayIndex",
                                vec![
                                    ("max_entry_count_bits", T::u8()),
                                    ("index_block_entries", T::u8().counted_as("entries")),
                                    ("secondary_block_min_pointers", T::u8()),
                                    ("data_block_min_entries", T::u8().counted_as("entries")),
                                    ("data_block_page_bits", T::u8()),
                                ],
                            ),
                        ),
                        (
                            5,
                            T::structure(
                                "BTree2Index",
                                vec![
                                    ("node_size", T::u32(Little).counted_as("bytes")),
                                    ("split_percent", T::u8()),
                                    ("merge_percent", T::u8()),
                                ],
                            ),
                        ),
                    ],
                    // An implicit index writes nothing: the chunks are all
                    // there, one after another, in the order they are counted.
                    T::bytes(E::lit(0)),
                ),
            ),
            ("address", addr()),
            ("datatype", datatype_copy()),
            (
                "chunks",
                T::switch(
                    E::field("index_type"),
                    vec![
                        (1, at_address("address", single_chunk())),
                        (2, at_address("address", implicit_chunks())),
                        (3, at_address("address", T::Named("FixedArray".into()))),
                        (4, at_address("address", T::Named("ExtensibleArray".into()))),
                        (5, at_address("address", T::Named("BTree2".into()))),
                    ],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
    .machinery(&["datatype"])
    .field_aside("datatype")
}

fn chunk_index_type() -> T {
    T::enumeration(
        "ChunkIndexType",
        T::u8(),
        &[
            (0, "version 1 b-tree"),
            (1, "single chunk"),
            (2, "implicit"),
            (3, "fixed array"),
            (4, "extensible array"),
            (5, "version 2 b-tree"),
        ],
    )
}

/// A single chunk index writes nothing at all unless a filter ran, in which
/// case what the filter left and which filters were skipped are here: there is
/// no entry anywhere else to keep them, since the address in the message is
/// the chunk itself rather than an index.
fn single_chunk_index() -> T {
    when(
        bit("flags", 1),
        T::structure(
            "SingleChunkIndex",
            vec![("chunk_size", length().counted_as("bytes")), ("filter_mask", T::u32(Little))],
        ),
    )
}

/// The one chunk, which is the whole dataset: as many bytes as the chunk
/// dimensions multiplied together, or as many as the filter left when one ran.
fn single_chunk() -> T {
    T::switch(
        bit("flags", 1),
        vec![(1, filtered_chunk(E::within(&["index", "chunk_size"])))],
        elements(Described::Inside, E::product_of("chunk_dimensions")),
    )
}

/// The most dimensions a dataspace can have. The library refuses a 33rd.
const MAX_RANK: usize = 32;

/// The chunks of an implicit index, which are one run. Every chunk the dataset
/// can ever have was written when it was made, one after another in the order
/// they are counted, so the address in the layout message is where the first
/// begins and each of the rest is a multiplication further on.
///
/// How many there are is written nowhere. It is the dataset's extent rounded
/// up a chunk at a time in every dimension, and those counts multiplied
/// together. The extent is the largest the dataspace allows rather than the
/// one it has now: a dataset that can grow to a fixed size has room for all of
/// it set aside at the start, and the chunks are counted across that room. A
/// dataset of 10 by 7 that can grow to 10 by 10, in chunks of 4 by 3, has three
/// rows of four chunks, and the first chunk of its second row is the fifth in
/// the run, not the fourth.
///
/// Each chunk reads as elements, the same as an unfiltered chunk an array
/// entry points at. An implicit index is never filtered: a filter makes chunks
/// of different sizes, and a run of them could not be counted by arithmetic.
fn implicit_chunks() -> T {
    T::structure(
        "ImplicitChunks",
        vec![
            ("chunks_across", T::array(chunks_across(), E::field("dimensionality").sub(E::lit(1)))),
            // Named once so that every chunk is sized by naming it: a run
            // whose elements all say the same field is counted by division
            // rather than walked.
            ("chunk_bytes", T::computed(E::product_of("chunk_dimensions"))),
            (
                "chunks",
                T::array(
                    T::sized(E::field("chunk_bytes"), elements(Described::Inside, E::field("chunk_bytes"))),
                    E::product_of("chunks_across"),
                ),
            ),
        ],
    )
    .machinery(&["chunks_across", "chunk_bytes"])
}

/// How many chunks one dimension of the dataset takes: its largest extent,
/// or its extent where the dataspace writes no largest one, divided by the
/// chunk's size along it and rounded up. A dataset 10 long in chunks of 4 has
/// three, the last of them half past the end.
///
/// The extent is in the dataspace message, a sibling of the layout message
/// this sits in, and a sibling is reached by a path with no index in it that
/// can change from one element to the next. So the dimension is chosen by a
/// case for each one a dataspace can have, which reads its extent by number.
fn chunks_across() -> T {
    let cases = (0..MAX_RANK)
        .map(|k| {
            let k_text = k.to_string();
            let extent = E::sibling(&["body", "max_dimensions", &k_text])
                .or(E::sibling(&["body", "dimensions", &k_text]));
            (k as i128, T::computed(extent.div_ceil(E::elem("chunk_dimensions", E::idx()))))
        })
        .collect();
    T::switch(E::idx(), cases, T::computed(E::lit(0)))
}

/// A chunk a filter pipeline wrote, which keeps its bytes: what is in the file
/// is the pipeline's output, and undoing it is not something a field can do.
/// Marked so the panel can find the reader that does.
fn filtered_chunk(size: E) -> T {
    T::structure("FilteredChunk", vec![("bytes", T::bytes(size))]).packed_as(super::hdf5_chunk::PACKING)
}

/// Where the chunks of a dataset whose dimensions cannot grow are: one entry
/// per chunk, in the order the chunks are counted, with nothing to search and
/// no key to compare. The array knows how many entries there will ever be
/// when it is made, which is what a fixed dataspace buys.
pub(super) fn fixed_array() -> T {
    T::structure(
        "FixedArray",
        vec![
            ("signature", T::magic(b"FAHD")),
            ("version", T::u8()),
            ("client_id", array_client_id()),
            ("entry_size", T::u8().counted_as("bytes")),
            ("page_bits", T::u8()),
            ("max_entry_count", length().counted_as("entries")),
            ("data_block_address", addr()),
            ("checksum", T::u32(Little)),
            ("data_block", at_address("data_block_address", fixed_array_data_block())),
        ],
    )
}

/// The entries of a fixed array, which are in the block itself when one page
/// holds them all and in pages after it when not.
///
/// A page is a power of two entries, two to the `page_bits` the header gives,
/// and each page ends in a checksum of its own, so one damaged page costs
/// that page rather than every entry in the array. The block begins with a
/// bitmap of which pages were ever written, one bit a page from the top bit of
/// the first byte, and a page whose bit is clear was set aside and never
/// filled: its bytes are whatever the file held there and no entries at all.
/// The pages follow the block's own checksum, all but the last of them full.
fn fixed_array_data_block() -> T {
    T::structure(
        "FixedArrayDataBlock",
        vec![
            ("signature", T::magic(b"FADB")),
            ("version", T::u8()),
            ("client_id", array_client_id()),
            ("header_address", addr()),
            ("page_entries", T::computed(E::lit(1).shl(E::field("page_bits")))),
            // Nought for a block that is not paged, which is also how many
            // bytes the bitmap then takes.
            (
                "page_count",
                T::computed(
                    E::field("page_entries")
                        .less_than(E::field("max_entry_count"))
                        .mul(E::field("max_entry_count").div_ceil(E::field("page_entries"))),
                ),
            ),
            ("page_bitmap", T::array(T::u8(), E::field("page_count").div_ceil(E::lit(8)))),
            ("entries", when(E::field("page_count").equals(E::lit(0)), entries(E::field("max_entry_count")))),
            ("checksum", T::u32(Little)),
            (
                "pages",
                T::array(
                    // The last page holds what the others left over.
                    page(
                        E::field("page_entries")
                            .at_most(E::field("max_entry_count").sub(E::idx().mul(E::field("page_entries")))),
                        page_written(E::idx()),
                    ),
                    E::field("page_count"),
                ),
            ),
        ],
    )
    .machinery(&["page_entries", "page_count"])
}

/// A run of `count` array entries. Each is sized by the header's entry size,
/// which is what lets the cursor land on the thousandth without the entries
/// before it being read.
fn entries(count: E) -> T {
    T::array(T::sized(E::field("entry_size"), array_entry()), count)
}

/// One page of an array's entries, `count` of them and a checksum, or the
/// bytes set aside for one when `written` is nought.
///
/// Sized either way, by the count and the entry size, since a page that was
/// never written takes the same room as one that was and the page after it is
/// only found by stepping over it.
fn page(count: E, written: E) -> T {
    T::sized(
        count.clone().mul(E::field("entry_size")).add(E::lit(4)),
        T::switch(
            written,
            vec![(1, T::structure("Page", vec![("entries", entries(count)), ("checksum", T::u32(Little))]))],
            T::bytes(E::Remaining),
        ),
    )
}

/// Whether bit `n` of the nearest `page_bitmap` is set. The bits run from the
/// top of the first byte down, then on to the top of the next.
fn page_written(n: E) -> E {
    let byte = E::elem("page_bitmap", n.clone().div(E::lit(8)));
    let below_top = n.clone().sub(n.div(E::lit(8)).mul(E::lit(8)));
    byte.shr(E::lit(7).sub(below_top)).and(E::lit(1))
}

/// Where the chunks of a dataset with one dimension that can grow are. The
/// first handful of entries are in the index block itself, which is the whole
/// of a small dataset's index; past that they are in data blocks the index
/// block points at, and past that in secondary blocks of doubling size.
pub(super) fn extensible_array() -> T {
    T::structure(
        "ExtensibleArray",
        vec![
            ("signature", T::magic(b"EAHD")),
            ("version", T::u8()),
            ("client_id", array_client_id()),
            // The spec calls this the element size; it is named as the fixed
            // array's is, because one entry is read by the same fields.
            ("entry_size", T::u8().counted_as("bytes")),
            ("max_entry_count_bits", T::u8()),
            ("index_block_entries", T::u8().counted_as("entries")),
            ("data_block_min_entries", T::u8().counted_as("entries")),
            ("secondary_block_min_pointers", T::u8()),
            ("data_block_page_bits", T::u8()),
            ("secondary_block_count", length().counted_as("blocks")),
            ("secondary_block_size", length().counted_as("bytes")),
            ("data_block_count", length().counted_as("blocks")),
            ("data_block_size", length().counted_as("bytes")),
            ("max_index_set", length()),
            ("entry_count", length().counted_as("entries")),
            ("index_block_address", addr()),
            ("checksum", T::u32(Little)),
            ("index_block", at_address("index_block_address", extensible_array_index_block())),
        ],
    )
}

/// The index block, and what lies past it. How many of each kind of address it
/// holds is written nowhere: the library works it out from four numbers in the
/// header, and so does this.
///
/// The entries after the index block's own are counted in super blocks. Super
/// block `u` is `2^(u/2)` data blocks of `2^((u+1)/2)` times the smallest data
/// block each, both halves rounded down: one block of the smallest size, one
/// of twice that, two of twice that, two of four times, four of four times,
/// and on, so the array doubles as it grows and no block is ever rewritten.
///
/// The first `2 log2(m)` super blocks, where `m` is the fewest data block
/// addresses the header lets a secondary block hold, are small enough that the
/// index block keeps their data block addresses itself, and there are
/// `2(m - 1)` of those. Every super block after that is a secondary block,
/// whose address is here and which holds the addresses of its own data
/// blocks. There is a secondary block address for every super block up to the
/// largest array the header allows, `2^max_entry_count_bits` entries, which
/// comes to one more than that number of bits less the base-2 logarithm of the
/// smallest data block, less the ones whose data blocks are here. The ones not
/// needed yet hold the undefined address, and so do data block addresses.
fn extensible_array_index_block() -> T {
    T::structure(
        "ExtensibleArrayIndexBlock",
        vec![
            ("signature", T::magic(b"EAIB")),
            ("version", T::u8()),
            ("client_id", array_client_id()),
            ("header_address", addr()),
            ("entries", entries(E::field("index_block_entries"))),
            (
                "data_blocks",
                T::array(index_data_block_pointer(), E::lit(2).mul(E::field("secondary_block_min_pointers").sub(E::lit(1)))),
            ),
            (
                "secondary_blocks",
                T::array(
                    secondary_block_pointer(),
                    E::lit(1)
                        .add(E::field("max_entry_count_bits"))
                        .sub(E::field("data_block_min_entries").log2())
                        .sub(E::lit(2).mul(E::field("secondary_block_min_pointers").log2())),
                ),
            ),
            ("checksum", T::u32(Little)),
        ],
    )
}

/// A data block address the index block holds, and the block.
///
/// Laid end to end, the super blocks these belong to give one block of the
/// smallest size, then three of twice that, six of four times, twelve of
/// eight times: the size doubles each time the count of blocks so far reaches
/// `3 * 2^k - 2`. So block `j` holds the smallest size times two to the base-2
/// logarithm of `2(j + 2) / 3`, which is the same thing said without a loop.
fn index_data_block_pointer() -> T {
    T::structure(
        "DataBlockPointer",
        vec![
            ("address", addr()),
            (
                "block_entries",
                T::computed(
                    E::field("data_block_min_entries").shl(E::lit(2).mul(E::idx().add(E::lit(2))).div(E::lit(3)).log2()),
                ),
            ),
            ("block", at_address("address", extensible_array_data_block(false))),
        ],
    )
    .machinery(&["block_entries"])
}

/// A secondary block address the index block holds, and the block. The first
/// is super block `2 log2(m)` and each one after it is the next.
///
/// A data block with more entries than a page holds is paged the way a fixed
/// array's block is, a checksum to each page. The bitmap saying which pages
/// were written is in the secondary block rather than in the data block, and
/// only a secondary block's data blocks are ever paged: the library refuses a
/// page smaller than the first of them, and the index block's are all smaller
/// still. With the numbers the library writes for a dataset's chunks that is
/// past 131,060 chunks.
fn secondary_block_pointer() -> T {
    T::structure(
        "SecondaryBlockPointer",
        vec![
            ("address", addr()),
            ("super_block", T::computed(E::idx().add(E::lit(2).mul(E::field("secondary_block_min_pointers").log2())))),
            ("data_block_count", T::computed(E::lit(1).shl(E::field("super_block").div(E::lit(2))))),
            (
                "block_entries",
                T::computed(E::field("data_block_min_entries").shl(E::field("super_block").add(E::lit(1)).div(E::lit(2)))),
            ),
            ("page_entries", T::computed(E::lit(1).shl(E::field("data_block_page_bits")))),
            // Nought when the data blocks are not paged. A paged one has no
            // short last page, since a data block and a page are both a power
            // of two entries.
            (
                "page_count",
                T::computed(
                    E::field("page_entries")
                        .less_than(E::field("block_entries"))
                        .mul(E::field("block_entries").div(E::field("page_entries"))),
                ),
            ),
            ("block", at_address("address", extensible_array_secondary_block())),
        ],
    )
    .machinery(&["super_block", "data_block_count", "block_entries", "page_entries", "page_count"])
}

fn extensible_array_secondary_block() -> T {
    T::structure(
        "ExtensibleArraySecondaryBlock",
        vec![
            ("signature", T::magic(b"EASB")),
            ("version", T::u8()),
            ("client_id", array_client_id()),
            ("header_address", addr()),
            ("block_offset", block_offset()),
            // Whole bytes for each data block, but the bits are numbered
            // straight through them: page `p` of data block `k` is bit
            // `k * page_count + p`, which is how the library counts.
            (
                "page_bitmap",
                T::array(T::u8(), E::field("data_block_count").mul(E::field("page_count").div_ceil(E::lit(8)))),
            ),
            ("data_blocks", T::array(secondary_data_block_pointer(), E::field("data_block_count"))),
            ("checksum", T::u32(Little)),
        ],
    )
}

/// A data block address a secondary block holds, and the block. Every data
/// block of one secondary block is the same size.
fn secondary_data_block_pointer() -> T {
    T::structure(
        "DataBlockPointer",
        vec![
            ("address", addr()),
            ("position", T::computed(E::idx())),
            ("block", at_address("address", extensible_array_data_block(true))),
        ],
    )
    .machinery(&["position"])
}

/// Where a block's first entry falls among all of the array's, which the
/// library writes into every secondary block and data block to check it has
/// the block it meant. As many whole bytes as the largest index needs.
fn block_offset() -> T {
    T::uint_expr(E::field("max_entry_count_bits").div_ceil(E::lit(8)).mul(E::lit(8)), Little)
}

/// A data block of an extensible array, `block_entries` entries long. One a
/// secondary block holds can be paged, and then its entries are in pages after
/// its checksum rather than before it.
fn extensible_array_data_block(pageable: bool) -> T {
    let mut fields = vec![
        ("signature", T::magic(b"EADB")),
        ("version", T::u8()),
        ("client_id", array_client_id()),
        ("header_address", addr()),
        ("block_offset", block_offset()),
    ];
    if pageable {
        fields.extend(vec![
            ("entries", when(E::field("page_count").equals(E::lit(0)), entries(E::field("block_entries")))),
            ("checksum", T::u32(Little)),
            (
                "pages",
                T::array(
                    page(
                        E::field("page_entries"),
                        page_written(E::field("position").mul(E::field("page_count")).add(E::idx())),
                    ),
                    E::field("page_count"),
                ),
            ),
        ]);
    } else {
        fields.extend(vec![("entries", entries(E::field("block_entries"))), ("checksum", T::u32(Little))]);
    }
    T::structure("ExtensibleArrayDataBlock", fields)
}

/// What either array calls its entries: a chunk's address, and, where a filter
/// ran, how much of the chunk was written and which filters were skipped. How
/// wide that size is, is whatever the entry has left once the address and the
/// mask have taken theirs.
fn array_entry() -> T {
    T::structure(
        "Entry",
        vec![
            ("chunk_address", addr()),
            (
                "filtered",
                T::switch(
                    E::field("client_id"),
                    vec![(
                        1,
                        T::inline_structure(
                            "Filtered",
                            vec![
                                (
                                    "chunk_size",
                                    T::uint_expr(E::field("entry_size").sub(E::lit(12)).mul(E::lit(8)), Little)
                                        .counted_as("bytes"),
                                ),
                                ("filter_mask", T::u32(Little)),
                            ],
                        ),
                    )],
                    T::bytes(E::lit(0)),
                ),
            ),
            (
                "chunk",
                T::switch(
                    E::field("client_id"),
                    vec![(1, at_address("chunk_address", filtered_chunk(E::within(&["filtered", "chunk_size"]))))],
                    at_address("chunk_address", elements(Described::Inside, E::product_of("chunk_dimensions"))),
                ),
            ),
        ],
    )
}

fn array_client_id() -> T {
    T::enumeration("ArrayClient", T::u8(), &[(0, "chunks, unfiltered"), (1, "chunks, filtered")])
}


#[cfg(test)]
mod tests {
    use super::super::hdf5::hdf5;
    use super::super::hdf5::tests::{addr_bytes, one_link_file, put, read, ALPHA_HEADER, DATA, LINK};
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// The same file with the dataset laid out the way HDF5 1.10 and later lay
    /// one out: a version 4 layout message, chunked, with a single chunk index.
    /// The message body is the same 24 bytes the version 3 one had, since a
    /// chunk covering the whole of a two-element dataset takes fewer.
    ///
    /// The dataspace and datatype messages in front of it are untouched, which
    /// is the point: nothing in the layout message says what an element is,
    /// and nothing but the chunk dimensions says how many bytes there are.
    fn single_chunk_file() -> Vec<u8> {
        let mut f = one_link_file();
        // Three messages in, each of the two before it eight bytes of header
        // and sixteen of body, and then this one's own header.
        let body = ALPHA_HEADER + 16 + 24 + 24 + 8;
        // Version 4, chunked, no flags, two dimensions written in one byte
        // each: a chunk of two elements four bytes wide. Then a single chunk
        // index, which writes nothing of its own.
        put(&mut f, body, &[4, 2, 0, 2, 1, 2, 4, 1]);
        put(&mut f, body + 8, &addr_bytes(DATA));
        f
    }

    /// The path from the layout message's body down to the elements of the one
    /// chunk: the layout, its storage, the chunks the address places, and the
    /// run inside them.
    const SINGLE_CHUNK: &[usize] = &[6, 0, 6, 2, 4, 1, 1, 8, 0, 2];

    /// A version 4 layout message places the same bytes a version 3 one did,
    /// and nothing in it writes down how many there are: the chunk dimensions
    /// multiplied together are the size of a chunk, and the last of them is
    /// the size of an element rather than a dimension. Read the dimensionality
    /// the way version 3 writes it, one too many, and the run is four times
    /// too long.
    #[test]
    fn a_version_4_layout_reads_the_single_chunk_it_points_at() {
        let f = single_chunk_file();
        let mut first = LINK.to_vec();
        first.extend_from_slice(SINGLE_CHUNK);
        first.extend_from_slice(&[0]);
        assert_eq!(read(&f, &first).1.as_int(), Some(-7));
        let mut second = LINK.to_vec();
        second.extend_from_slice(SINGLE_CHUNK);
        second.extend_from_slice(&[1]);
        assert_eq!(read(&f, &second).1.as_int(), Some(1000));

        // Two elements and no more: the chunk is eight bytes because two
        // times four is, and the array stops there.
        let mut run = LINK.to_vec();
        run.extend_from_slice(SINGLE_CHUNK);
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());
        let node = ev.node(&doc, &run).expect("elements");
        assert!(matches!(node.value, Value::Composite { count: 2 }), "{:?}", node.value);
    }

    /// Which index a version 4 message names is a byte of its own, and the
    /// address means something different under each: the one chunk, an array,
    /// a b-tree. An index type this does not know is not followed, rather than
    /// the address being read as whichever of those was guessed at.
    #[test]
    fn an_unknown_chunk_index_is_left_alone() {
        let mut f = single_chunk_file();
        let body = ALPHA_HEADER + 16 + 24 + 24 + 8;
        put(&mut f, body + 7, &[9]);
        let mut chunks = LINK.to_vec();
        chunks.extend_from_slice(&[6, 0, 6, 2, 4, 1, 1, 8]);
        let (_, value) = read(&f, &chunks);
        assert!(matches!(value, Value::Bytes { len: 0, .. }), "{value:?}");
    }


    /// The one-link file with its dataset chunked a different way: a version 4
    /// layout message naming chunk index `index_type`, with `params` for it
    /// and `address` after them. A chunk is one element, four bytes.
    fn chunked_file(index_type: u8, params: &[u8], address: u64) -> Vec<u8> {
        let mut f = one_link_file();
        let body = ALPHA_HEADER + 16 + 24 + 24 + 8;
        put(&mut f, body, &[4, 2, 0, 2, 1, 1, 4, index_type]);
        put(&mut f, body + 8, params);
        put(&mut f, body + 8 + params.len() as u64, &addr_bytes(address));
        f
    }

    /// The layout message's storage, where every chunk index hangs.
    const CHUNKED: &[usize] = &[6, 0, 6, 2, 4, 1, 1];

    /// Down from `path` by field name or list index. An address holds what it
    /// points at as its one child, and a name steps through one the way an
    /// expression does.
    fn down(ev: &mut Evaluator, doc: &Document<MemSource>, path: &[usize], steps: &[&str]) -> Vec<usize> {
        let mut p = path.to_vec();
        for step in steps {
            let mut found = ev.child_named(doc, &p, step).expect("reads");
            if found.is_none() {
                p.push(0);
                found = ev.child_named(doc, &p, step).expect("reads");
            }
            p = found.unwrap_or_else(|| panic!("nothing called {step} under {p:?}"));
        }
        p
    }

    /// The first element of the chunk an entry points at.
    fn chunk_value(ev: &mut Evaluator, doc: &Document<MemSource>, entry: &[usize]) -> Option<i128> {
        let at = down(ev, doc, entry, &["chunk", "elements", "0"]);
        ev.node(doc, &at).expect("reads").value.as_int()
    }

    /// An implicit index writes no entries, so how many chunks it has is the
    /// dataspace rounded up a chunk at a time: five elements in chunks of two
    /// are three chunks, the last of them half empty. Divided down, the run
    /// would stop a chunk short and the fifth element would not be in it.
    #[test]
    fn an_implicit_index_rounds_the_dataspace_up_to_whole_chunks() {
        let mut f = chunked_file(2, &[], DATA);
        let body = ALPHA_HEADER + 16 + 24 + 24 + 8;
        put(&mut f, body + 5, &[2, 4]);
        put(&mut f, ALPHA_HEADER + 16 + 16, &5u64.to_le_bytes());
        for k in 0..6 {
            put(&mut f, DATA + 4 * k, &(k as i32 * 10).to_le_bytes());
        }
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());
        let mut storage = LINK.to_vec();
        storage.extend_from_slice(CHUNKED);

        let chunks = down(&mut ev, &doc, &storage, &["chunks", "chunks"]);
        assert!(matches!(ev.node(&doc, &chunks).unwrap().value, Value::Composite { count: 3 }));
        let fifth = down(&mut ev, &doc, &chunks, &["2", "elements", "0"]);
        let node = ev.node(&doc, &fifth).unwrap();
        assert_eq!(node.value.as_int(), Some(40));
        assert_eq!(node.offset_bits / 8, DATA + 16);
    }

    /// A fixed array of five entries two to a page is three pages, the last
    /// holding one, after a bitmap saying which were written. The middle page
    /// here was not, and its bytes are the kind of thing a file leaves in room
    /// it set aside: read as entries, they are addresses far past the end of
    /// the file.
    #[test]
    fn a_paged_fixed_array_reads_the_pages_its_bitmap_says_were_written() {
        const HEADER: u64 = 400;
        const BLOCK: u64 = 450;
        const CHUNKS: u64 = 600;
        let mut f = chunked_file(3, &[1], HEADER);
        put(&mut f, HEADER, b"FAHD");
        put(&mut f, HEADER + 4, &[0, 0, 8, 1]);
        put(&mut f, HEADER + 8, &5u64.to_le_bytes());
        put(&mut f, HEADER + 16, &addr_bytes(BLOCK));
        put(&mut f, BLOCK, b"FADB");
        put(&mut f, BLOCK + 4, &[0, 0]);
        put(&mut f, BLOCK + 6, &addr_bytes(HEADER));
        // Pages 0 and 2, from the top bit down.
        put(&mut f, BLOCK + 14, &[0b1010_0000]);
        let pages = BLOCK + 14 + 1 + 4;
        let entry = |k: u64| match k {
            0 | 1 => pages + 8 * k,
            2 | 3 => pages + 20 + 8 * (k - 2),
            _ => pages + 40,
        };
        for k in 0..5 {
            put(&mut f, entry(k), &addr_bytes(CHUNKS + 4 * k));
            put(&mut f, CHUNKS + 4 * k, &(k as i32 + 100).to_le_bytes());
        }
        put(&mut f, pages + 20, &[0xee; 16]);
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());
        let mut storage = LINK.to_vec();
        storage.extend_from_slice(CHUNKED);

        let block = down(&mut ev, &doc, &storage, &["chunks", "data_block"]);
        let pages_at = down(&mut ev, &doc, &block, &["pages"]);
        assert!(matches!(ev.node(&doc, &pages_at).unwrap().value, Value::Composite { count: 3 }));
        let unwritten = down(&mut ev, &doc, &pages_at, &["1"]);
        assert!(matches!(ev.node(&doc, &unwritten).unwrap().value, Value::Bytes { len: 20, .. }));
        let first = down(&mut ev, &doc, &pages_at, &["0", "entries", "1"]);
        assert_eq!(chunk_value(&mut ev, &doc, &first), Some(101));
        // The short last page, whose one entry is where two full pages end.
        let last = down(&mut ev, &doc, &pages_at, &["2", "entries"]);
        assert!(matches!(ev.node(&doc, &last).unwrap().value, Value::Composite { count: 1 }));
        let last = down(&mut ev, &doc, &last, &["0"]);
        assert_eq!(ev.node(&doc, &last).unwrap().offset_bits / 8, entry(4));
        assert_eq!(chunk_value(&mut ev, &doc, &last), Some(104));
        let checksum = down(&mut ev, &doc, &pages_at, &["2", "checksum"]);
        assert_eq!(ev.node(&doc, &checksum).unwrap().offset_bits / 8, entry(4) + 8);
    }

    /// An extensible array small enough to reach every kind of block with a
    /// few entries: one entry in the index block, a page of two entries, and
    /// data blocks that start at one entry and double.
    ///
    /// With a largest array of 2^4 entries, a smallest data block of one and
    /// at least two data blocks to a secondary block, the index block holds
    /// the addresses of two data blocks (of one entry and of two) and of three
    /// secondary blocks: the first with two data blocks of two entries, the
    /// second with two of four, which is past a page and so paged, and the
    /// third with four of four. None of those counts is in the file.
    #[test]
    fn an_extensible_array_follows_its_data_blocks_and_secondary_blocks() {
        const HEADER: u64 = 400;
        const INDEX: u64 = 500;
        const J0: u64 = 600;
        const J1: u64 = 640;
        const S0: u64 = 700;
        const S0_D0: u64 = 740;
        const S1: u64 = 800;
        const S1_D1: u64 = 840;
        const CHUNKS: u64 = 1000;
        const NONE: u64 = u64::MAX;
        let mut f = chunked_file(4, &[4, 1, 2, 1, 1], HEADER);
        put(&mut f, HEADER, b"EAHD");
        put(&mut f, HEADER + 4, &[0, 0, 8, 4, 1, 1, 2, 1]);
        put(&mut f, HEADER + 60, &addr_bytes(INDEX));
        // Every block opens the same way: a signature, version and client,
        // and the header's address.
        let opening = |f: &mut Vec<u8>, at: u64, sign: &[u8]| {
            put(f, at, sign);
            put(f, at + 4, &[0, 0]);
            put(f, at + 6, &addr_bytes(HEADER));
        };
        let entry = |f: &mut Vec<u8>, at: u64, k: u64| put(f, at, &addr_bytes(CHUNKS + 4 * k));
        opening(&mut f, INDEX, b"EAIB");
        entry(&mut f, INDEX + 14, 0);
        put(&mut f, INDEX + 22, &addr_bytes(J0));
        put(&mut f, INDEX + 30, &addr_bytes(J1));
        put(&mut f, INDEX + 38, &addr_bytes(S0));
        put(&mut f, INDEX + 46, &addr_bytes(S1));
        put(&mut f, INDEX + 54, &addr_bytes(NONE));
        // Data blocks carry a one-byte offset into the array after the
        // header's address, since four bits of index fit in a byte.
        opening(&mut f, J0, b"EADB");
        put(&mut f, J0 + 14, &[1]);
        entry(&mut f, J0 + 15, 1);
        opening(&mut f, J1, b"EADB");
        put(&mut f, J1 + 14, &[2]);
        entry(&mut f, J1 + 15, 2);
        entry(&mut f, J1 + 23, 3);
        opening(&mut f, S0, b"EASB");
        put(&mut f, S0 + 14, &[4]);
        put(&mut f, S0 + 15, &addr_bytes(S0_D0));
        put(&mut f, S0 + 23, &addr_bytes(NONE));
        opening(&mut f, S0_D0, b"EADB");
        put(&mut f, S0_D0 + 14, &[4]);
        entry(&mut f, S0_D0 + 15, 4);
        entry(&mut f, S0_D0 + 23, 5);
        // Two bytes of bitmap, one for each data block, and the bits counted
        // straight through: the second block's pages are bits 2 and 3, and
        // only its second page was written.
        opening(&mut f, S1, b"EASB");
        put(&mut f, S1 + 14, &[8, 0b0001_0000, 0]);
        put(&mut f, S1 + 17, &addr_bytes(NONE));
        put(&mut f, S1 + 25, &addr_bytes(S1_D1));
        opening(&mut f, S1_D1, b"EADB");
        put(&mut f, S1_D1 + 14, &[12]);
        let pages = S1_D1 + 15 + 4;
        put(&mut f, pages, &[0xee; 16]);
        entry(&mut f, pages + 20, 14);
        entry(&mut f, pages + 28, 15);
        for k in 0..16 {
            put(&mut f, CHUNKS + 4 * k, &(k as i32 + 200).to_le_bytes());
        }
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());
        let mut storage = LINK.to_vec();
        storage.extend_from_slice(CHUNKED);
        let index = down(&mut ev, &doc, &storage, &["chunks", "index_block"]);

        let count = |ev: &mut Evaluator, at: &[usize]| ev.node(&doc, at).unwrap().child_count;
        let blocks = down(&mut ev, &doc, &index, &["data_blocks"]);
        assert_eq!(count(&mut ev, &blocks), 2);
        let secondary = down(&mut ev, &doc, &index, &["secondary_blocks"]);
        assert_eq!(count(&mut ev, &secondary), 3);

        let at = down(&mut ev, &doc, &index, &["entries", "0"]);
        assert_eq!(chunk_value(&mut ev, &doc, &at), Some(200));
        let at = down(&mut ev, &doc, &blocks, &["1", "block", "entries", "1"]);
        assert_eq!(chunk_value(&mut ev, &doc, &at), Some(203));
        let at = down(&mut ev, &doc, &secondary, &["0", "block", "data_blocks", "0", "block", "entries", "1"]);
        assert_eq!(chunk_value(&mut ev, &doc, &at), Some(205));
        let paged = down(&mut ev, &doc, &secondary, &["1", "block", "data_blocks", "1", "block", "pages"]);
        assert_eq!(count(&mut ev, &paged), 2);
        let unwritten = down(&mut ev, &doc, &paged, &["0"]);
        assert!(matches!(ev.node(&doc, &unwritten).unwrap().value, Value::Bytes { len: 20, .. }));
        let at = down(&mut ev, &doc, &paged, &["1", "entries", "1"]);
        assert_eq!(chunk_value(&mut ev, &doc, &at), Some(215));
        let nothing = down(&mut ev, &doc, &secondary, &["2", "block"]);
        assert!(matches!(ev.node(&doc, &nothing).unwrap().value, Value::Bytes { len: 0, .. }));
    }

    /// How big each of the index block's data blocks is comes from one line of
    /// arithmetic standing in for the library's loop over super blocks. Checked
    /// against that loop for thirty blocks, which is as many as an index block
    /// holds when a secondary block has at least sixteen.
    #[test]
    fn an_index_block_s_data_blocks_double_where_the_library_s_super_blocks_do() {
        const HEADER: u64 = 400;
        const INDEX: u64 = 500;
        let mut f = chunked_file(4, &[32, 4, 16, 16, 10], HEADER);
        put(&mut f, HEADER, b"EAHD");
        put(&mut f, HEADER + 4, &[0, 0, 8, 32, 4, 16, 16, 10]);
        put(&mut f, HEADER + 60, &addr_bytes(INDEX));
        put(&mut f, INDEX, b"EAIB");
        put(&mut f, INDEX + 6, &addr_bytes(HEADER));
        // Four entries, thirty data blocks and twenty-one secondary blocks, all
        // unused, and a checksum. Twenty-one is the 29 super blocks an array
        // of 2^32 entries from blocks of 16 comes to, less the 8 whose data
        // blocks the index block holds.
        put(&mut f, INDEX + 14, &[0xff; (4 + 30 + 21) * 8]);
        put(&mut f, INDEX + 14 + 55 * 8, &[0; 4]);
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());
        let mut storage = LINK.to_vec();
        storage.extend_from_slice(CHUNKED);
        let index = down(&mut ev, &doc, &storage, &["chunks", "index_block"]);
        let secondary = down(&mut ev, &doc, &index, &["secondary_blocks"]);
        assert_eq!(ev.node(&doc, &secondary).unwrap().child_count, 21);

        // H5EA__hdr_init: super block u has 2^(u/2) data blocks of
        // 2^((u+1)/2) times the smallest.
        let mut sizes = Vec::new();
        for u in 0..8u32 {
            for _ in 0..1u64 << (u / 2) {
                sizes.push(16u64 << ((u + 1) / 2));
            }
        }
        assert_eq!(sizes.len(), 30);
        for (j, want) in sizes.iter().take(30).enumerate() {
            let at = down(&mut ev, &doc, &index, &["data_blocks", &j.to_string(), "block_entries"]);
            assert_eq!(ev.node(&doc, &at).unwrap().value.as_int(), Some(*want as i128), "data block {j}");
        }
    }
}
