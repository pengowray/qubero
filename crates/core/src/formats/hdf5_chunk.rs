//! Undoing what an HDF5 filter pipeline did to a chunk.
//!
//! A chunked dataset does not write its elements. It writes each chunk through
//! a list of filters, in the order the filter pipeline message gives, and what
//! lands in the file is the last one's output. Reading a chunk means undoing
//! them in the other order, and until that is done the bytes at the chunk's
//! address are not numbers, not text and not anything a field can describe.
//!
//! So the template leaves a filtered chunk as its bytes and this says what
//! they hold, which is the arrangement [`pdf_objstm`](super::pdf_objstm) and
//! [`ggml_quant`](super::ggml_quant) already have.
//!
//! Every step is reported, not just the answer: which filter, how many bytes
//! went in, how many came out. A chunk that went in at 53 KB and comes out at
//! 208 KB has said something about itself, and a step that could not be undone
//! has said more.
//!
//! The chunk's own b-tree key carries a filter mask, one bit per filter,
//! marking the ones that were skipped for this chunk alone. A writer sets one
//! when a filter made that chunk bigger, which deflate does for data that is
//! already compressed. A skipped filter is listed and stepped over.
//!
//! What is undone: deflate (with or without its zlib wrapper), shuffle,
//! bitshuffle (32008) with or without its LZ4 blocks, and the two checks that
//! only add bytes, fletcher32 and the 32-bit checksum. The rest are named and
//! stop the walk: `szip`, `nbit` and `scaleoffset` are real compression
//! schemes, and the other ones from 32000 up are somebody's own library, which
//! is the point of the number being that high. What has been undone up to
//! that point is still reported.

/// What [`StructDef::packed`](crate::template::StructDef::packed) calls this,
/// so the template can mark a filtered chunk and the panel can find its way
/// back here.
pub const PACKING: &str = "hdf5_chunk";

/// The largest chunk this will unpack, and the largest it will unpack it to.
/// A chunk is a few hundred kilobytes in the files people write; a claim far
/// past that is a reason to stop rather than a reason to allocate.
pub const PACKED_LIMIT: usize = 64 << 20;
const DECODED_LIMIT: usize = 256 << 20;

/// One filter, as the pipeline message wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub id: u16,
    /// What the filter was told, which for deflate is the level and for
    /// shuffle is how wide one element is.
    pub client_data: Vec<u32>,
}

/// One step of the walk back, in the order it was done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The filter's name, or its number where it has none here.
    pub filter: String,
    pub in_bytes: usize,
    pub out_bytes: usize,
    /// Set when the chunk's own mask said this filter was not applied to it,
    /// in which case nothing was done and the bytes went straight through.
    pub skipped: bool,
}

/// What the chunk turned out to hold, and what it took to get there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub packed_bytes: usize,
    pub steps: Vec<Step>,
    /// The elements' bytes, once every filter that could be undone was.
    pub bytes: Vec<u8>,
    /// Why the walk stopped early, where it did.
    pub problem: Option<String>,
}

/// The name a filter number goes by. The same list the template shows, kept
/// here as well because a panel names what it did rather than what a field
/// two levels up was called.
pub fn filter_name(id: u16) -> Option<&'static str> {
    Some(match id {
        1 => "deflate",
        2 => "shuffle",
        3 => "fletcher32",
        4 => "szip",
        5 => "nbit",
        6 => "scaleoffset",
        32000 => "lzf",
        32001 => "blosc",
        32004 => "lz4",
        32008 => "bitshuffle",
        32015 => "zstd",
        _ => return None,
    })
}

/// Undo the pipeline. `filters` is in the order the message lists them, which
/// is the order they were applied; `mask` is the chunk's own filter mask, bit
/// `i` set meaning filter `i` was skipped for this chunk. `element_size` is
/// what shuffle needs and nothing else uses.
pub fn decode(data: &[u8], filters: &[Filter], mask: u32, element_size: usize) -> Chunk {
    let mut chunk =
        Chunk { packed_bytes: data.len(), steps: Vec::new(), bytes: data.to_vec(), problem: None };
    if data.len() > PACKED_LIMIT {
        let mb = PACKED_LIMIT / (1 << 20);
        chunk.bytes.clear();
        chunk.problem = Some(format!("Not unpacked: the chunk is over this viewer's {mb} MB limit."));
        return chunk;
    }
    // The last filter applied is the first to undo.
    for (i, filter) in filters.iter().enumerate().rev() {
        let name = filter_name(filter.id).map_or_else(|| format!("filter {}", filter.id), str::to_string);
        let in_bytes = chunk.bytes.len();
        if mask >> i & 1 == 1 {
            chunk.steps.push(Step { filter: name, in_bytes, out_bytes: in_bytes, skipped: true });
            continue;
        }
        let done = match filter.id {
            1 => match inflate(&chunk.bytes) {
                Some(bytes) => Some(bytes),
                None => {
                    // A stream that will not inflate is a different answer
                    // from a filter nothing here undoes, and saying the
                    // second would be untrue of the first.
                    chunk.problem = Some("Stopped at deflate: the compressed data would not inflate.".into());
                    return chunk;
                }
            },
            2 => Some(unshuffle(&chunk.bytes, shuffle_width(filter, element_size))),
            32008 => match unbitshuffle(&chunk.bytes, filter, element_size) {
                Ok((bytes, lz4)) => {
                    // Two steps of one filter: the LZ4 blocks unpacked, then
                    // the bits of each block put back into their elements.
                    let mut in_bytes = in_bytes;
                    if lz4 {
                        chunk.steps.push(Step { filter: "bitshuffle LZ4".into(), in_bytes, out_bytes: bytes.len(), skipped: false });
                        in_bytes = bytes.len();
                    }
                    chunk.steps.push(Step { filter: name, in_bytes, out_bytes: bytes.len(), skipped: false });
                    chunk.bytes = bytes;
                    continue;
                }
                Err(why) => {
                    chunk.problem = Some(format!("Stopped at bitshuffle: {why}"));
                    return chunk;
                }
            },
            // Everything else is either a check taken off below, or a filter
            // nothing here undoes.
            _ => None,
        };
        match done {
            Some(bytes) => {
                chunk.steps.push(Step { filter: name, in_bytes, out_bytes: bytes.len(), skipped: false });
                chunk.bytes = bytes;
            }
            None if filter.id == 3 => {
                // fletcher32: a four-byte checksum after the data.
                let cut = chunk.bytes.len().saturating_sub(4);
                chunk.bytes.truncate(cut);
                chunk.steps.push(Step { filter: name, in_bytes, out_bytes: cut, skipped: false });
            }
            None => {
                chunk.problem = Some(match filter_name(filter.id) {
                    Some(known) => {
                        format!("Stopped at {known}: this filter isn't undone here, so the elements could not be decoded.")
                    }
                    None => format!(
                        "Stopped at filter {}: it isn't a filter this viewer can undo, so the elements could not be decoded.",
                        filter.id
                    ),
                });
                return chunk;
            }
        }
        if chunk.bytes.len() > DECODED_LIMIT {
            chunk.problem =
                Some(format!("Unpacking stopped at the {} MB limit; elements past that were not decoded.", DECODED_LIMIT / (1 << 20)));
            chunk.bytes.truncate(DECODED_LIMIT);
            return chunk;
        }
    }
    chunk
}

/// How wide shuffle thought an element was. The filter is told so in its
/// client data, and where it was not, the datatype's own size is the answer.
fn shuffle_width(filter: &Filter, element_size: usize) -> usize {
    match filter.client_data.first() {
        Some(&n) if n > 0 => n as usize,
        _ => element_size.max(1),
    }
}

/// Decompress, as zlib and then as raw deflate. HDF5 writes the zlib wrapper;
/// the fallback costs nothing and covers a writer that did not.
fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use miniz_oxide::inflate::decompress_to_vec_with_limit as raw;
    use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit as zlib;
    zlib(data, DECODED_LIMIT).ok().or_else(|| raw(data, DECODED_LIMIT).ok())
}

/// Undo the bitshuffle filter (32008, from Kiyoshi Masui's `bitshuffle`
/// library, which Breakthrough Listen writes its filterbank files with).
/// Returns the elements' bytes, and whether the chunk was LZ4 blocks.
///
/// The filter's client data is the library's version (two numbers), how wide
/// an element is, how many elements a block holds (0 for the library's
/// default), and the compression: 0 for none, 2 for LZ4, 3 for zstd.
///
/// A compressed chunk starts with the size the elements come to, as a
/// big-endian u64, and a block's size in bytes, as a big-endian u32. Then
/// each block is a big-endian u32 size and that many bytes of one LZ4 block.
/// Every block holds the block size's worth of elements but the last, which
/// holds what is left rounded down to a multiple of 8. The last 0 to 7
/// elements are not in a block at all: they follow the blocks as they are.
///
/// Inside a block the bits are transposed. For a block of `n` elements, each
/// `elem` bytes wide, the block is `8 * elem` rows of `n` bits: row
/// `8 * j + k` is bit `k` (counted from the low bit) of byte `j` of every
/// element in turn, and element `i`'s bit is bit `i % 8` of byte `i / 8` of
/// the row.
pub fn unbitshuffle(data: &[u8], filter: &Filter, element_size: usize) -> Result<(Vec<u8>, bool), String> {
    let cd = |i: usize| filter.client_data.get(i).copied().unwrap_or(0) as usize;
    let elem = if cd(2) > 0 { cd(2) } else { element_size.max(1) };
    let compression = cd(4);
    let be32 = |b: &[u8], at: usize| b.get(at..at + 4).map(|s| u32::from_be_bytes(s.try_into().unwrap()) as usize);
    let short = || "the chunk is shorter than its header.".to_string();
    let (total, block_elems, mut at) = match compression {
        0 => (data.len(), if cd(3) > 0 { cd(3) } else { default_block(elem) }, 0),
        2 => {
            let total = data.get(..8).map(|s| u64::from_be_bytes(s.try_into().unwrap())).ok_or_else(short)?;
            let block_bytes = be32(data, 8).ok_or_else(short)?;
            if total > DECODED_LIMIT as u64 {
                return Err(format!("the chunk says it comes to {total} bytes, over this viewer's limit."));
            }
            (total as usize, block_bytes / elem, 12)
        }
        3 => return Err("its blocks are zstd, which isn't undone here.".into()),
        n => return Err(format!("compression {n} isn't one the library writes.")),
    };
    if block_elems == 0 || block_elems % 8 != 0 {
        return Err(format!("a block of {block_elems} elements isn't a multiple of 8."));
    }
    let count = total / elem;
    let mut out = Vec::with_capacity(total);
    let mut done = 0;
    while count - done >= 8 {
        let n = block_elems.min((count - done) / 8 * 8);
        let bytes = n * elem;
        let block = if compression == 2 {
            let size = be32(data, at).ok_or("the chunk ends before its last block.")?;
            let end = (at + 4).checked_add(size).filter(|&e| e <= data.len()).ok_or("a block runs past the end of the chunk.")?;
            let packed = &data[at + 4..end];
            at = end;
            let mut block = vec![0u8; bytes];
            match lz4_flex::block::decompress_into(packed, &mut block) {
                Ok(got) if got == bytes => block,
                _ => return Err("an LZ4 block would not unpack to the size it should.".into()),
            }
        } else {
            let raw = data.get(at..at + bytes).ok_or("a block runs past the end of the chunk.")?;
            at += bytes;
            raw.to_vec()
        };
        out.extend_from_slice(&untranspose_bits(&block, n, elem));
        done += n;
    }
    // The last few elements, as they are.
    let rest = total - out.len();
    out.extend_from_slice(data.get(at..at + rest).ok_or("the chunk ends before its last elements.")?);
    Ok((out, compression == 2))
}

/// How many elements the library puts in a block when it is not told: what
/// fits in 8 KB, rounded down to a multiple of 8, and never under 128.
fn default_block(elem: usize) -> usize {
    (8192 / elem / 8 * 8).max(128)
}

/// Put one block's bits back into its `n` elements of `elem` bytes. See
/// [`unbitshuffle`] for the order they are in.
fn untranspose_bits(block: &[u8], n: usize, elem: usize) -> Vec<u8> {
    let mut out = vec![0u8; n * elem];
    let row_bytes = n / 8;
    for j in 0..elem {
        for k in 0..8 {
            let row = &block[(8 * j + k) * row_bytes..][..row_bytes];
            for (g, &byte) in row.iter().enumerate() {
                if byte == 0 {
                    continue;
                }
                for b in 0..8 {
                    out[(g * 8 + b) * elem + j] |= (byte >> b & 1) << k;
                }
            }
        }
    }
    out
}

/// Undo the shuffle: the filter writes all the first bytes of every element,
/// then all the second bytes, and so on, which puts the bytes that vary least
/// next to each other and gives deflate something to work with. Putting them
/// back is the same walk the other way round.
///
/// The tail that does not fill a whole element is left where it is, which is
/// what the filter does with it too.
pub fn unshuffle(data: &[u8], width: usize) -> Vec<u8> {
    if width <= 1 || data.len() < width {
        return data.to_vec();
    }
    let elements = data.len() / width;
    let mut out = vec![0u8; data.len()];
    let mut at = 0;
    for byte in 0..width {
        for element in 0..elements {
            out[element * width + byte] = data[at];
            at += 1;
        }
    }
    // Whatever did not make up a whole element was written after the planes.
    out[elements * width..].copy_from_slice(&data[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shuffle(data: &[u8], width: usize) -> Vec<u8> {
        let elements = data.len() / width;
        let mut out = Vec::with_capacity(data.len());
        for byte in 0..width {
            for element in 0..elements {
                out.push(data[element * width + byte]);
            }
        }
        out.extend_from_slice(&data[elements * width..]);
        out
    }

    fn numbers() -> Vec<u8> {
        (0..1000i32).flat_map(|n| n.to_le_bytes()).collect()
    }

    #[test]
    fn a_shuffled_run_comes_back_the_way_it_went_in() {
        let data = numbers();
        assert_eq!(unshuffle(&shuffle(&data, 4), 4), data);
        // A tail that does not fill an element stays where it was put.
        let mut odd = data.clone();
        odd.extend_from_slice(&[0xaa, 0xbb]);
        assert_eq!(unshuffle(&shuffle(&odd, 4), 4), odd);
    }

    /// The order matters and it is the reverse of the order the message lists:
    /// shuffle ran first and deflate ran on its output, so deflate is undone
    /// first.
    #[test]
    fn a_chunk_is_undone_in_the_reverse_of_the_order_it_was_done() {
        let data = numbers();
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&shuffle(&data, 4), 6);
        let filters = vec![
            Filter { id: 2, client_data: vec![4] },
            Filter { id: 1, client_data: vec![6] },
        ];
        let chunk = decode(&packed, &filters, 0, 4);
        assert_eq!(chunk.problem, None);
        assert_eq!(chunk.bytes, data);
        let steps: Vec<(&str, usize, usize)> =
            chunk.steps.iter().map(|s| (s.filter.as_str(), s.in_bytes, s.out_bytes)).collect();
        assert_eq!(steps[0].0, "deflate");
        assert_eq!(steps[0].1, packed.len());
        assert_eq!(steps[1], ("shuffle", data.len(), data.len()));
    }

    /// A filter the chunk's own mask says was skipped is named and stepped
    /// over: the bytes went into the file without it.
    #[test]
    fn a_filter_the_mask_skipped_is_not_undone() {
        let data = numbers();
        let filters = vec![Filter { id: 1, client_data: vec![6] }];
        let chunk = decode(&data, &filters, 1, 4);
        assert_eq!(chunk.bytes, data);
        assert!(chunk.steps[0].skipped);
        assert_eq!(chunk.problem, None);
    }

    /// A filter nothing here undoes stops the walk and says so, and what was
    /// undone before it is still reported.
    #[test]
    fn a_filter_this_does_not_know_stops_the_walk_and_says_where() {
        let data = numbers();
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
        let filters = vec![
            Filter { id: 32001, client_data: vec![] },
            Filter { id: 1, client_data: vec![6] },
        ];
        let chunk = decode(&packed, &filters, 0, 4);
        assert_eq!(chunk.steps.len(), 1);
        assert_eq!(chunk.bytes, data);
        assert!(chunk.problem.expect("a problem").contains("blosc"));
    }

    /// A checksum is not a change to the data: four bytes come off the end and
    /// what is left is what went in.
    #[test]
    fn a_checksum_is_taken_off_rather_than_undone() {
        let mut data = numbers();
        let want = data.clone();
        data.extend_from_slice(&[1, 2, 3, 4]);
        let chunk = decode(&data, &[Filter { id: 3, client_data: vec![] }], 0, 4);
        assert_eq!(chunk.bytes, want);
        assert_eq!(chunk.problem, None);
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// Twenty u32s, `i * 1000003`, as h5py and hdf5plugin 5 wrote them in one
    /// chunk through bitshuffle: with LZ4 (client data 0,4,4,0,2) and without
    /// (0,4,4,0,0). Sixteen of them make one block and the last four follow it
    /// as they are.
    const BITSHUFFLE_LZ4: &str = "0000000000000050000020000000003bf00daaaa6666b4b438c7c00700f8aaaaccccf0f0aa55cc66f078007f00801000f6025a5ac6393ef8fe0754559899e0e100fe0001005000000000003024f40073660301b6a81201f9ea2101";
    const BITSHUFFLE_PLAIN: &str = "aaaa6666b4b438c7c00700f8aaaaccccf0f0aa55cc66f078007f0080aaaacccc5a5ac6393ef8fe0754559899e0e100fe000000000000000000000000000000003024f40073660301b6a81201f9ea2101";

    fn twenty() -> Vec<u8> {
        (0..20u32).flat_map(|i| (i * 1_000_003).to_le_bytes()).collect()
    }

    #[test]
    fn a_bitshuffled_chunk_is_unpacked_and_its_bits_put_back() {
        let filter = Filter { id: 32008, client_data: vec![0, 4, 4, 0, 2] };
        let chunk = decode(&hex(BITSHUFFLE_LZ4), std::slice::from_ref(&filter), 0, 4);
        assert_eq!(chunk.problem, None);
        assert_eq!(chunk.bytes, twenty());
        let steps: Vec<(&str, usize, usize)> = chunk.steps.iter().map(|s| (s.filter.as_str(), s.in_bytes, s.out_bytes)).collect();
        assert_eq!(steps, [("bitshuffle LZ4", 91, 80), ("bitshuffle", 80, 80)]);
        // Without LZ4 the chunk is the transposed blocks and nothing else.
        let plain = Filter { id: 32008, client_data: vec![0, 4, 4, 0, 0] };
        let chunk = decode(&hex(BITSHUFFLE_PLAIN), &[plain], 0, 4);
        assert_eq!((chunk.bytes, chunk.problem), (twenty(), None));
        assert_eq!(chunk.steps.len(), 1);
    }

    /// A block of one-byte elements: each of the eight rows is one bit of
    /// every element, the low bit first.
    #[test]
    fn a_block_of_bytes_is_eight_rows_of_bits() {
        // Eight elements, so each row is one byte.
        let mut block = vec![0u8; 8];
        block[0] = 0b0000_0001; // bit 0 of element 0
        block[7] = 0b1000_0001; // bit 7 of elements 0 and 7
        assert_eq!(untranspose_bits(&block, 8, 1), [0b1000_0001, 0, 0, 0, 0, 0, 0, 0b1000_0000]);
    }

    /// A chunk cut short says so rather than unpacking what it can.
    #[test]
    fn a_bitshuffled_chunk_cut_short_stops_the_walk() {
        let filter = Filter { id: 32008, client_data: vec![0, 4, 4, 0, 2] };
        let packed = hex(BITSHUFFLE_LZ4);
        let chunk = decode(&packed[..40], &[filter], 0, 4);
        assert!(chunk.problem.expect("a problem").starts_with("Stopped at bitshuffle"));
    }
}
