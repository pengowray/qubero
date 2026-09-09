//! What the 7z template is checked against.
//!
//! Half of this is a small archive writer: `header_of`, `archive` and the
//! coder and folder builders put together the bytes a 7-Zip would have
//! written, so a test can say what shape it means rather than carrying a
//! blob nobody can read. They live here rather than beside the template
//! because they are the larger half of what was one seventeen-hundred line
//! file, and because nothing outside the tests builds an archive.

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
