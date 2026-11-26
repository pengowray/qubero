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
/// whole: the chunks fed for it push out the first ones, and the next reply
/// asks for those again. This is the case Open unpacked has to stop on rather
/// than go round for ever.
#[test]
fn a_run_longer_than_the_store_asks_again_for_chunks_it_was_fed() {
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
    let mut again = false;
    for _ in 0..10 {
        let reply = parse(&ed.open_space(0, &[1]));
        match reply["status"].as_str().unwrap() {
            "pending" => {
                for c in chunks_of(&reply) {
                    again |= !fed.insert(c);
                    feed(&mut ed, &file, c);
                }
            }
            "working" => {}
            _ => panic!("a run of seven chunks opened with room for three: {reply}"),
        }
        if again {
            return;
        }
    }
    panic!("no chunk was asked for twice");
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
    let (file, starts) = zip_of_tars_of_zeros(size, 2);
    (file, [starts[0], starts[1]])
}

/// The same with `count` tars, named `a.tar`, `b.tar` and on.
fn zip_of_tars_of_zeros(size: usize, count: u8) -> (Vec<u8>, Vec<u64>) {
    let mut file = Vec::new();
    let mut starts = vec![0; count as usize];
    for (i, name) in (0..count).map(|i| [b'a' + i, b'.', b't', b'a', b'r']).enumerate() {
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

/// A listing showing three ZIP entries at once, each a stream that takes
/// several goes to unpack, asks for each entry's fields once a round. Only
/// two passes are kept unfinished. The entries all list, in a few rounds
/// each: a stream that finds both places taken carries the others on, and
/// its "working" replies name no pass at all rather than another entry's. No
/// entry's pass goes back. The streams are 12 MiB, under the 64 MiB a stream
/// is held whole under, which takes as many goes per megabyte as one past it.
#[test]
fn a_listing_of_three_large_entries_lists_them_all() {
    let (file, starts) = zip_of_tars_of_zeros(12 << 20, 3);
    let mut ed = fed(&file, "zip");
    let mut listed: Vec<Option<Vec<String>>> = vec![None; 3];
    let (mut read, mut working, mut waiting) = (vec![0u64; 3], 0, 0);
    let mut rounds = 0;
    while listed.iter().any(Option::is_none) {
        for i in 0..3 {
            if listed[i].is_some() {
                continue;
            }
            let reply = parse(&ed.template_children(0, &[0, i as u32, 1], 0.0, 20.0));
            match reply["status"].as_str().unwrap() {
                "ok" => listed[i] = Some(names(&reply["node"])),
                "working" => {
                    working += 1;
                    let progress = &reply["unpacking"];
                    if progress.is_null() {
                        waiting += 1;
                        continue;
                    }
                    assert_eq!(run_at(progress), starts[i], "round {rounds}: entry {i} named another entry's pass");
                    let now = progress["read_bits"].as_f64().unwrap() as u64;
                    assert!(now >= read[i], "round {rounds}: entry {i}'s pass went back from {} to {now}", read[i]);
                    read[i] = now;
                }
                _ => panic!("{reply}"),
            }
        }
        rounds += 1;
        assert!(rounds < 100, "no end to the rounds, the passes at {read:?}");
    }
    assert!(working >= 6, "{working} working replies: the streams took too few goes to wait on each other");
    assert!(waiting >= 1, "no entry waited for a place");
    for (i, fields) in listed.into_iter().enumerate() {
        assert_eq!(fields.unwrap().get(14).map(String::as_str), Some("data"), "entry {i}");
    }
}
