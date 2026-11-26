//! Opening a stream as a tab when the answer is not ready yet, the way Open
//! unpacked in the web asks for it: `open_space` answers "pending" with the
//! chunks of the file it still needs, or "working" while a large stream is
//! unpacked a go at a time, and `unpacking` says how far that has got. And
//! the listing's side of the same wait: `template_children` answers "working"
//! with the children that are ready, and how far the pass the rest wait on
//! has got.
//!
//! The files are made here rather than taken from the sample collection, so
//! these run everywhere.

use std::collections::BTreeSet;

use qubero_core::checksum::{adler32, crc32, crc32_update};
use qubero_wasm::Editor;
use serde_json::Value;

const CHUNK: usize = 64 * 1024;

fn parse(reply: &str) -> Value {
    serde_json::from_str(reply).unwrap()
}

/// A gzip file of one member.
fn gzip_of(content: &[u8]) -> Vec<u8> {
    let mut gz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
    gz.extend(miniz_oxide::deflate::compress_to_vec(content, 6));
    gz.extend(crc32(content).to_le_bytes());
    gz.extend((content.len() as u32).to_le_bytes());
    gz
}

/// An editor over `file` with none of its chunks fed yet, keeping at most
/// `capacity` of them, read as `template`.
fn unfed(file: &[u8], capacity: u32, template: &str) -> Editor {
    let mut ed = Editor::new(file.len() as f64, CHUNK as u32, capacity);
    assert!(ed.set_template(template), "no template {template}");
    ed
}

fn feed(ed: &mut Editor, file: &[u8], chunk: u64) {
    let at = chunk as usize * CHUNK;
    ed.feed_chunk(chunk as f64, &file[at..file.len().min(at + CHUNK)]);
}

/// The chunks a pending reply asks for.
fn chunks_of(reply: &Value) -> Vec<u64> {
    reply["chunks"].as_array().unwrap().iter().map(|c| c.as_f64().unwrap() as u64).collect()
}

/// Lines of text, each with a number in hex that does not repeat, so they
/// pack to several chunks of deflate.
fn lines(n: usize) -> Vec<u8> {
    let mut x = 0x9e37_79b9_7f4a_7c15_u64;
    (0..n)
        .flat_map(|i| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            format!("line {i}: {x:016x}\n").into_bytes()
        })
        .collect()
}

/// Open unpacked pressed before the file's bytes are loaded: the reply names
/// the chunks it needs, and feeding exactly those and asking again comes, in a
/// round or a few, to the tab. With room for every chunk, no chunk is asked
/// for again once it was fed, which is what the web takes a repeat to mean:
/// that the chunks it loaded did not stay.
#[test]
fn a_stream_waiting_on_bytes_opens_once_the_chunks_it_names_are_fed() {
    let text = lines(12_000);
    let file = gzip_of(&text);
    assert!(file.len() > 2 * CHUNK, "{} bytes is too few chunks to wait on", file.len());
    let mut ed = unfed(&file, 1024, "gzip");
    let mut fed = BTreeSet::new();
    let mut rounds = 0;
    let space = loop {
        let reply = parse(&ed.open_space(0, &[1]));
        match reply["status"].as_str().unwrap() {
            "ok" => break reply["node"]["space"].as_f64().unwrap() as u32,
            "pending" => {
                let chunks = chunks_of(&reply);
                assert!(!chunks.is_empty(), "a pending reply with nothing to load: {reply}");
                for &c in &chunks {
                    assert!(fed.insert(c), "chunk {c} asked for again after it was fed");
                    feed(&mut ed, &file, c);
                }
                rounds += 1;
            }
            "working" => {}
            _ => panic!("{reply}"),
        }
        assert!(rounds < 50, "still pending after {rounds} rounds");
    };
    assert!(rounds >= 1);
    assert_ne!(space, 0);
    let mut head = vec![0u8; 64];
    assert!(ed.read_bytes(space, 0.0, &mut head).is_empty());
    assert_eq!(head, text[..64]);
}

/// A compressed run longer than the editor keeps chunks of can never be read
/// whole: the chunks fed for it would push out the first ones, and the next
/// reply would ask for those again. Open unpacked is refused instead, with
/// the reason, and no chunk is asked for twice.
#[test]
fn a_run_longer_than_the_store_is_refused_as_too_much_at_once() {
    // Bytes that do not pack, so the run is as long as they are.
    let mut x = 0x2545_f491_u32;
    let noise: Vec<u8> = (0..6 * CHUNK)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    let file = gzip_of(&noise);
    let mut ed = unfed(&file, 3, "gzip");
    let mut fed = BTreeSet::new();
    let mut rounds = 0;
    let reply = loop {
        let reply = parse(&ed.open_space(0, &[1]));
        match reply["status"].as_str().unwrap() {
            "pending" => {
                for c in chunks_of(&reply) {
                    assert!(fed.insert(c), "chunk {c} asked for again after it was fed");
                    feed(&mut ed, &file, c);
                }
                rounds += 1;
            }
            "working" => {}
            _ => break reply,
        }
        assert!(rounds < 10, "still pending after {rounds} rounds");
    };
    assert_eq!(reply["status"], "ok", "{reply}");
    assert_eq!(reply["node"]["refused"], "too-much-at-once", "a run of seven chunks with room for three: {reply}");
}

/// A gzip file of one member, packed at the fastest level, for a test that
/// wants megabytes of it.
fn gzip_fast(content: &[u8]) -> Vec<u8> {
    let mut gz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
    gz.extend(miniz_oxide::deflate::compress_to_vec(content, 1));
    gz.extend(crc32(content).to_le_bytes());
    gz.extend((content.len() as u32).to_le_bytes());
    gz
}

/// A zip of one entry, `content` deflated at the fastest level and named
/// `name`, with its central directory and end record.
fn zip_of(name: &[u8], content: &[u8]) -> Vec<u8> {
    let packed = miniz_oxide::deflate::compress_to_vec(content, 1);
    let crc = crc32(content);
    let sizes = |z: &mut Vec<u8>| {
        z.extend(crc.to_le_bytes());
        z.extend((packed.len() as u32).to_le_bytes());
        z.extend((content.len() as u32).to_le_bytes());
        z.extend((name.len() as u16).to_le_bytes());
    };
    let mut z = Vec::new();
    z.extend(0x0403_4b50u32.to_le_bytes());
    z.extend(20u16.to_le_bytes()); // version needed to extract
    z.extend(0u16.to_le_bytes()); // flags
    z.extend(8u16.to_le_bytes()); // deflate
    z.extend([0u8; 4]); // time and date
    sizes(&mut z);
    z.extend(0u16.to_le_bytes()); // extra field length
    z.extend(name);
    z.extend(&packed);
    let central = z.len();
    z.extend(0x0201_4b50u32.to_le_bytes());
    z.extend(20u16.to_le_bytes()); // version made by
    z.extend(20u16.to_le_bytes());
    z.extend(0u16.to_le_bytes());
    z.extend(8u16.to_le_bytes());
    z.extend([0u8; 4]);
    sizes(&mut z);
    // Extra field and comment lengths, disk, internal and external
    // attributes, and the local header's offset, which is 0.
    z.extend([0u8; 2 + 2 + 2 + 2 + 4 + 4]);
    z.extend(name);
    let size = z.len() - central;
    z.extend(0x0605_4b50u32.to_le_bytes());
    z.extend([0u8; 4]); // this disk, and the disk the directory starts on
    z.extend(1u16.to_le_bytes());
    z.extend(1u16.to_le_bytes());
    z.extend((size as u32).to_le_bytes());
    z.extend((central as u32).to_le_bytes());
    z.extend(0u16.to_le_bytes()); // comment length
    z
}

/// What came of asking the same question until it was answered, feeding
/// exactly the chunks each "pending" reply named and asking again on
/// "working", the way the web's listing does.
enum Asked {
    Answered(Value),
    /// A chunk that was fed was asked for again: the chunks fed for the
    /// answer pushed out ones it needed, so it can never come.
    Again(u64),
}

fn ask_feeding(ed: &mut Editor, file: &[u8], fed: &mut BTreeSet<u64>, mut ask: impl FnMut(&mut Editor) -> String) -> Asked {
    for _ in 0..100_000 {
        let reply = parse(&ask(ed));
        match reply["status"].as_str().unwrap() {
            "pending" => {
                for c in chunks_of(&reply) {
                    if !fed.insert(c) {
                        return Asked::Again(c);
                    }
                    feed(ed, file, c);
                }
            }
            "working" => {}
            _ => return Asked::Answered(reply),
        }
    }
    panic!("neither answered nor asked again");
}

/// The children of the node at `path`, or the chunk asked for again.
fn children(ed: &mut Editor, file: &[u8], fed: &mut BTreeSet<u64>, path: &[u32]) -> Result<Vec<Value>, u64> {
    match ask_feeding(ed, file, fed, |ed| ed.template_children(0, path, 0.0, 1e9)) {
        Asked::Answered(reply) => {
            assert_eq!(reply["status"], "ok", "{reply}");
            Ok(reply["node"].as_array().unwrap().clone())
        }
        Asked::Again(c) => Err(c),
    }
}

/// The path of the child of `path` called `name`.
fn child_named(ed: &mut Editor, file: &[u8], fed: &mut BTreeSet<u64>, path: &[u32], name: &str) -> Result<Vec<u32>, u64> {
    let found = children(ed, file, fed, path)?;
    let child = found.iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("no {name} under {path:?}: {found:?}"));
    Ok(child["path"].as_array().unwrap().iter().map(|i| i.as_u64().unwrap() as u32).collect())
}

/// The node at `path`, which the listing asks for once it has the children.
fn node(ed: &mut Editor, file: &[u8], fed: &mut BTreeSet<u64>, path: &[u32]) -> Value {
    match ask_feeding(ed, file, fed, |ed| ed.template_node(0, path)) {
        Asked::Answered(reply) => {
            assert_eq!(reply["status"], "ok", "{reply}");
            reply["node"].clone()
        }
        Asked::Again(c) => panic!("chunk {c} asked for again for the node at {path:?}"),
    }
}

/// The listing of a zip whose one deflated entry is several times longer
/// than the editor keeps chunks of, as the web lists it: the entry's `data`,
/// then what is inside it.
fn list_a_long_zip_entry(ed: &mut Editor, file: &[u8], fed: &mut BTreeSet<u64>) -> Result<(Vec<u32>, Vec<Value>), u64> {
    let body = child_named(ed, file, fed, &[0, 0], "body")?;
    let data = child_named(ed, file, fed, &body, "data")?;
    let inside = children(ed, file, fed, &data)?;
    Ok((data, inside))
}

/// The entry for the zip and gzip tests below: lines of hex that pack to
/// about three times the 16 chunks those tests keep.
fn long_lines() -> Vec<u8> {
    lines(260_000)
}

/// A deflated zip entry longer than the chunk store, listed the way the web
/// lists it. The entry's run would be read in one call, which needs more
/// chunks at once than the store keeps: feeding the chunks it named pushed
/// out ones it needed, and the listing asked for them again for ever. Now
/// the entry's `data` is refused with the reason, before any of its run is
/// asked for.
#[test]
fn a_zip_entry_longer_than_the_store_is_refused_as_too_much_at_once() {
    let file = zip_of(b"lines.txt", &long_lines());
    assert!(file.len() > 3 * 16 * CHUNK, "{} bytes is too few for a store of 16 chunks", file.len());
    let mut ed = unfed(&file, 16, "zip");
    let mut fed = BTreeSet::new();
    let (data, inside) = match list_a_long_zip_entry(&mut ed, &file, &mut fed) {
        Ok(listed) => listed,
        Err(c) => panic!("chunk {c} asked for again after it was fed"),
    };
    assert!(inside.is_empty(), "{inside:?}");
    assert_eq!(node(&mut ed, &file, &mut fed, &data)["refused"], "too-much-at-once");
    // The header and the directory at the end, and none of the run between.
    assert!(fed.len() < 16, "{} chunks loaded: {fed:?}", fed.len());
    // Open unpacked gives the same reason.
    let reply = parse(&ed.open_space(0, &data));
    assert_eq!(reply["node"]["refused"], "too-much-at-once", "{reply}");
}

/// A .gz whose one member is longer than the chunk store. The member is
/// measured by inflating it from where its run starts, and that read was
/// longer than the store keeps, so the member never got a size and its fields
/// asked for the same chunks for ever. Now the measuring reads at most half
/// the store and finds no end there, so the template takes the member's run
/// to be everything before its trailer. That run is still longer than the
/// store, and it is refused with the reason.
#[test]
fn a_gzip_member_longer_than_the_store_is_refused_as_too_much_at_once() {
    let file = gzip_fast(&long_lines());
    assert!(file.len() > 3 * 16 * CHUNK, "{} bytes is too few for a store of 16 chunks", file.len());
    let mut ed = unfed(&file, 16, "gzip");
    let mut fed = BTreeSet::new();
    let compressed = match child_named(&mut ed, &file, &mut fed, &[0, 0], "compressed") {
        Ok(path) => path,
        Err(c) => panic!("chunk {c} asked for again after it was fed"),
    };
    let run = node(&mut ed, &file, &mut fed, &compressed);
    // Everything between the ten bytes of header and the eight of trailer.
    assert_eq!(run["size_bits"].as_f64().unwrap() as usize, (file.len() - 18) * 8, "{run}");
    let inside = match children(&mut ed, &file, &mut fed, &compressed) {
        Ok(inside) => inside,
        Err(c) => panic!("chunk {c} asked for again after it was fed"),
    };
    assert!(inside.is_empty(), "{inside:?}");
    assert_eq!(node(&mut ed, &file, &mut fed, &compressed)["refused"], "too-much-at-once");
    assert!(fed.len() < 16, "{} chunks loaded: {fed:?}", fed.len());
    // Open unpacked on the file's `decoded`, which joins the members, says the
    // same of the joined stream.
    let reply = parse(&ed.open_space(0, &[1]));
    assert_eq!(reply["node"]["refused"], "too-much-at-once", "{reply}");
    // And listing `decoded` where it is declared reads the member's run as
    // the joined stream's one part, which fails the same way, naming the part,
    // instead of waiting.
    let inside = match children(&mut ed, &file, &mut fed, &[1]) {
        Ok(inside) => inside,
        Err(c) => panic!("chunk {c} asked for again after it was fed"),
    };
    assert_eq!(inside.len(), 1, "{inside:?}");
    let reply = match ask_feeding(&mut ed, &file, &mut fed, |ed| ed.template_children(0, &[1, 0], 0.0, 1e9)) {
        Asked::Answered(reply) => reply,
        Asked::Again(c) => panic!("chunk {c} asked for again after it was fed"),
    };
    assert_eq!(reply["message"], "members[0].compressed is too large for the editor to keep in memory at once", "{reply}");
}

/// Bits in the order deflate packs them, from the low bit of each byte up.
#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, value: u32, len: u32) {
        self.acc |= u64::from(value) << self.n;
        self.n += len;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// A Huffman code, which deflate writes from its top bit down.
    fn code(&mut self, code: u32, len: u32) {
        for i in (0..len).rev() {
            self.put((code >> i) & 1, 1);
        }
    }

    /// A literal byte in the fixed codes.
    fn literal(&mut self, v: u8) {
        match v {
            0..=143 => self.code(0x30 + u32::from(v), 8),
            _ => self.code(0x190 + u32::from(v) - 144, 9),
        }
    }

    fn done(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// One deflate block in the fixed codes: `head` as literals, then `zeros`
/// zero bytes, nearly all of them as copies of 258 bytes from one byte back.
/// Written by hand because packing seventy megabytes with a compressor built
/// for debugging takes longer than the test does.
fn deflate_head_then_zeros(head: &[u8], zeros: usize) -> Vec<u8> {
    let mut b = Bits::default();
    b.put(1, 1); // the last block
    b.put(1, 2); // in the fixed codes
    for &v in head {
        b.literal(v);
    }
    b.literal(0);
    let mut left = zeros - 1;
    while left >= 258 {
        b.code(0xc5, 8); // length 258, code 285
        b.code(0, 5); // distance 1
        left -= 258;
    }
    for _ in 0..left {
        b.literal(0);
    }
    b.code(0, 7); // end of block
    b.done()
}

/// A tar of one file of `size` zero bytes, as its header and how many zero
/// bytes follow it: the file's, rounded up to a block, and the two empty
/// blocks that end the archive.
fn tar_of_zeros(size: usize) -> (Vec<u8>, usize) {
    let mut header = vec![0u8; 512];
    header[..9].copy_from_slice(b"zeros.bin");
    header[100..108].copy_from_slice(b"0000644\0");
    header[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    header[136..148].copy_from_slice(b"15245725034\0");
    header[156] = b'0';
    header[257..265].copy_from_slice(b"ustar\x0000");
    header[148..156].copy_from_slice(b"        ");
    let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    (header, size.div_ceil(512) * 512 + 1024)
}

/// That tar packed as a zlib stream: 70 MiB of tar from under half a
/// megabyte of file, which is past the 64 MiB a stream is unpacked whole
/// under.
fn zlib_of_a_tar_of_zeros(size: usize) -> Vec<u8> {
    let (header, zeros) = tar_of_zeros(size);
    let mut file = vec![0x78, 0x01];
    file.extend(deflate_head_then_zeros(&header, zeros));
    // The Adler-32 of the header and then the zeros, which leave the first
    // sum where the header left it and add it to the second once a byte.
    let a_b = adler32(&header);
    let (a, b) = (a_b & 0xffff, a_b >> 16);
    let b = ((u64::from(b) + zeros as u64 * u64::from(a)) % 65521) as u32;
    file.extend(((b << 16) | a).to_be_bytes());
    file
}

/// The same tar as a deflate stream of its own, with the CRC-32 and the
/// length of what it unpacks to.
fn deflate_of_a_tar_of_zeros(size: usize) -> (Vec<u8>, u32, u32) {
    let (header, zeros) = tar_of_zeros(size);
    let block = vec![0u8; 1 << 20];
    let mut crc = crc32(&header);
    let mut left = zeros;
    while left > 0 {
        let n = left.min(block.len());
        crc = crc32_update(crc, &block[..n]);
        left -= n;
    }
    (deflate_head_then_zeros(&header, zeros), crc, (header.len() + zeros) as u32)
}

/// The same tar packed as a gzip of one member.
fn gzip_of_a_tar_of_zeros(size: usize) -> Vec<u8> {
    let (packed, crc, len) = deflate_of_a_tar_of_zeros(size);
    let mut file = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
    file.extend(packed);
    file.extend(crc.to_le_bytes());
    file.extend(len.to_le_bytes());
    file
}

/// A ZIP of two tars of zeros, each deflated, and where in the file each
/// one's deflate run starts, in bits. Only the local records and the end
/// record are written: the template reads the records in order and needs
/// nothing from a central directory.
fn zip_of_two_tars_of_zeros(size: usize) -> (Vec<u8>, [u64; 2]) {
    let mut file = Vec::new();
    let mut starts = [0; 2];
    for (i, name) in [b"a.tar", b"b.tar"].into_iter().enumerate() {
        let (packed, crc, len) = deflate_of_a_tar_of_zeros(size);
        file.extend(b"PK\x03\x04");
        file.extend(20u16.to_le_bytes()); // version needed
        file.extend(0u16.to_le_bytes()); // flags
        file.extend(8u16.to_le_bytes()); // deflate
        file.extend([0u8; 4]); // time and date
        file.extend(crc.to_le_bytes());
        file.extend((packed.len() as u32).to_le_bytes());
        file.extend(len.to_le_bytes());
        file.extend((name.len() as u16).to_le_bytes());
        file.extend(0u16.to_le_bytes()); // no extra field
        file.extend(name);
        starts[i] = file.len() as u64 * 8;
        file.extend(packed);
    }
    let directory = file.len() as u32;
    file.extend(b"PK\x05\x06");
    file.extend([0u8; 12]); // disks, entries and the directory's size: none
    file.extend(directory.to_le_bytes());
    file.extend(0u16.to_le_bytes()); // no comment
    (file, starts)
}

/// What one go of a call about the tab comes to: done, or still working with
/// how far the unpacking has got. Anything else fails the test.
fn go(reply: &str, ed: &Editor, space: u32) -> Option<Value> {
    let reply = parse(reply);
    match reply["status"].as_str().unwrap() {
        "ok" => None,
        "working" => Some(parse(&ed.unpacking(space))["node"].clone()),
        _ => panic!("{reply}"),
    }
}

/// How far a progress reply says the pass has read, failing on no progress.
fn read_bits(progress: &Value) -> u64 {
    assert!(!progress.is_null(), "working, and no unpacking under way");
    progress["read_bits"].as_f64().unwrap() as u64
}

/// A tab over a stream whose bytes were recognised has a reading of its own,
/// but after an edit its stream is unpacked again in the reading it was opened
/// from, a go at a time. Until that is done every call about the tab answers
/// "working", and the tab's progress is that pass's, not its own reading's.
#[test]
fn a_recognised_tab_opened_again_after_an_edit_says_how_far_unpacking_has_got() {
    let file = zlib_of_a_tar_of_zeros(70 << 20);
    let mut ed = Editor::new(file.len() as f64, CHUNK as u32, 1024);
    for (i, part) in file.chunks(CHUNK).enumerate() {
        ed.feed_chunk(i as f64, part);
    }
    assert!(ed.set_template("zlib"));
    // The zlib template's `compressed`, opened from the file's tab.
    let run = [6];
    let (mut goes, mut read) = (0, 0);
    let tab = loop {
        let reply = ed.open_space(0, &run);
        match go(&reply, &ed, 0) {
            None => break parse(&reply)["node"]["space"].as_f64().unwrap() as u32,
            Some(progress) => {
                let now = read_bits(&progress);
                assert!(now >= read, "{read} then {now}");
                read = now;
                goes += 1;
            }
        }
        assert!(goes < 10_000);
    };
    assert!(goes >= 2, "unpacked in {goes} goes");
    assert!(ed.space_recognised(tab), "the tar was not recognised");
    assert_eq!(name(&mut ed, tab), "zeros.bin");

    // An edit that changes nothing still lets go of every stream.
    let mut first = [0u8];
    assert!(ed.read_bytes(0, 0.0, &mut first).is_empty());
    ed.overwrite_bytes(0.0, &first);
    let (mut goes, mut read) = (0, 0);
    while let Some(progress) = go(&ed.template_node(tab, &[]), &ed, tab) {
        let now = read_bits(&progress);
        assert!(now >= read, "{read} then {now}");
        read = now;
        goes += 1;
        assert!(goes < 10_000);
    }
    assert!(goes >= 2, "opened again in {goes} goes");
    assert!(read > 0);
    assert!(parse(&ed.unpacking(tab))["node"].is_null(), "still unpacking once the tab answers");
    assert_eq!(name(&mut ed, tab), "zeros.bin");
}

/// A stream opened from a tab whose bytes were recognised is unpacked in that
/// tab's own reading, and the tab says how far it has got: a gzip of a gzip,
/// the inner one recognised and holding 70 MiB of tar.
#[test]
fn a_stream_opened_inside_a_recognised_tab_says_how_far_unpacking_has_got() {
    let inner = gzip_of_a_tar_of_zeros(70 << 20);
    let file = gzip_of(&inner);
    let mut ed = Editor::new(file.len() as f64, CHUNK as u32, 1024);
    for (i, part) in file.chunks(CHUNK).enumerate() {
        ed.feed_chunk(i as f64, part);
    }
    assert!(ed.set_template("gzip"));
    // The gzip's `decoded`, which is the inner gzip, a few hundred kilobytes.
    let reply = parse(&ed.open_space(0, &[1]));
    assert_eq!(reply["status"], "ok", "{reply}");
    let gzip = reply["node"]["space"].as_f64().unwrap() as u32;
    assert!(ed.space_recognised(gzip), "the inner gzip was not recognised");
    let (mut goes, mut read) = (0, 0);
    let tar = loop {
        let reply = ed.open_space(gzip, &[1]);
        match go(&reply, &ed, gzip) {
            None => break parse(&reply)["node"]["space"].as_f64().unwrap() as u32,
            Some(progress) => {
                let now = read_bits(&progress);
                assert!(now >= read, "{read} then {now}");
                read = now;
                goes += 1;
            }
        }
        assert!(goes < 10_000);
    };
    assert!(goes >= 2, "unpacked in {goes} goes");
    assert!(read > 0);
    assert!(ed.space_recognised(tar), "the tar was not recognised");
    assert_eq!(name(&mut ed, tar), "zeros.bin");
}

/// The name of the first file in the tar the tab reads.
fn name(ed: &mut Editor, tab: u32) -> String {
    let reply = parse(&ed.field_text(tab, &[0, 0, 0]));
    assert_eq!(reply["status"], "ok", "{reply}");
    reply["node"]["text"].as_str().unwrap().trim_end_matches('\0').to_string()
}

/// An editor over all of `file`, read as `template`.
fn fed(file: &[u8], template: &str) -> Editor {
    let mut ed = Editor::new(file.len() as f64, CHUNK as u32, 1024);
    for (i, part) in file.chunks(CHUNK).enumerate() {
        ed.feed_chunk(i as f64, part);
    }
    assert!(ed.set_template(template), "no template {template}");
    ed
}

/// The names of the nodes in a reply's array.
fn names(nodes: &Value) -> Vec<String> {
    nodes.as_array().map_or(Vec::new(), |a| a.iter().map(|n| n["name"].as_str().unwrap().to_string()).collect())
}

/// Where the pass a reply's `unpacking` names starts, in bits.
fn run_at(progress: &Value) -> u64 {
    assert!(!progress.is_null(), "working, and no unpacking under way");
    progress["run_offset_bits"].as_f64().unwrap() as u64
}

/// While a gzip member's stream is unpacked a go at a time, the listing asks
/// for the member's fields and gets the ones before the stream, its header,
/// as `ready` on each "working" reply, with how far the pass has got as
/// `unpacking`. The fields after the stream wait with it.
#[test]
fn a_gzip_members_header_lists_while_its_stream_is_unpacked() {
    let mut ed = fed(&gzip_of_a_tar_of_zeros(70 << 20), "gzip");
    let member = [0, 0];
    let mut seen = Vec::new();
    let (mut goes, mut header, mut read) = (0, 0, 0);
    let all = loop {
        let reply = parse(&ed.template_children(0, &member, 0.0, 20.0));
        match reply["status"].as_str().unwrap() {
            "ok" => break names(&reply["node"]),
            "working" => {
                let ready = names(&reply["ready"]);
                assert!(!ready.contains(&"compressed".to_string()), "the stream was ready: {reply}");
                if ready.len() == 10 {
                    header += 1;
                    assert_eq!(run_at(&reply["unpacking"]), 80, "the member's run starts after its ten bytes of header");
                    let now = reply["unpacking"]["read_bits"].as_f64().unwrap() as u64;
                    assert!(now >= read, "{read} then {now}");
                    read = now;
                }
                seen.push(ready);
            }
            _ => panic!("{reply}"),
        }
        goes += 1;
        assert!(goes < 10_000);
    };
    assert_eq!(all.len(), 13);
    assert!(seen.iter().all(|ready| all.starts_with(ready)), "{seen:?} against {all:?}");
    assert!(header >= 2, "the header was listed in {header} of {goes} goes");
    assert_eq!(all[10], "compressed");
}

/// A listing carrying on one stream while Open unpacked opens another, in
/// the same reading: two entries of a ZIP. Each "working" reply to the
/// listing says how far its own entry's pass has got, and `unpacking` asked
/// right after Open unpacked's "working" reply says how far the other has.
#[test]
fn a_listing_and_open_unpacked_each_say_how_far_their_own_stream_has_got() {
    let (file, starts) = zip_of_two_tars_of_zeros(70 << 20);
    let mut ed = fed(&file, "zip");
    // Each record is its signature and its body, and the body's fifteenth
    // field is the data.
    let (body, data) = ([0, 0, 1], [0, 1, 1, 14]);
    let (mut listed, mut opened) = (None, None);
    let (mut listing, mut opening) = (0, 0);
    while listed.is_none() || opened.is_none() {
        if listed.is_none() {
            let reply = parse(&ed.template_children(0, &body, 0.0, 20.0));
            match reply["status"].as_str().unwrap() {
                "ok" => listed = Some(names(&reply["node"])),
                "working" => {
                    listing += 1;
                    assert!(!names(&reply["ready"]).contains(&"data".to_string()), "{reply}");
                    assert_eq!(run_at(&reply["unpacking"]), starts[0], "listing go {listing}");
                }
                _ => panic!("{reply}"),
            }
        }
        if opened.is_none() {
            let reply = parse(&ed.open_space(0, &data));
            match reply["status"].as_str().unwrap() {
                "ok" => opened = Some(reply["node"]["space"].as_f64().unwrap() as u32),
                "working" => {
                    opening += 1;
                    assert_eq!(run_at(&parse(&ed.unpacking(0))["node"]), starts[1], "opening go {opening}");
                }
                _ => panic!("{reply}"),
            }
        }
        assert!(listing + opening < 10_000);
    }
    assert!(listing >= 2 && opening >= 2, "{listing} goes listing, {opening} opening");
    assert_eq!(listed.unwrap().get(14).map(String::as_str), Some("data"));
    let tab = opened.unwrap();
    assert!(ed.space_recognised(tab), "the second tar was not recognised");
    assert_eq!(name(&mut ed, tab), "zeros.bin");
}
