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
use qubero_core::codec::{self, inflate, Codec, StepKind};
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

/// A compressed run longer than the editor keeps chunks of is read a window
/// at a time, each window a quarter of the store, into a copy kept from one
/// call to the next, so Open unpacked comes to the tab. Here the store keeps
/// three chunks and the run is seven: the member is measured, and the joined
/// stream reads it again, so each chunk is asked for at most twice.
#[test]
fn a_run_longer_than_the_store_opens_a_window_at_a_time() {
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
    let mut feeder = Feeder::new(&file);
    let reply = feeder.ask(&mut ed, |ed| ed.open_space(0, &[1]));
    assert_eq!(reply["status"], "ok", "{reply}");
    let tab = reply["node"]["space"].as_f64().unwrap_or_else(|| panic!("no tab: {reply}")) as u32;
    assert_ne!(tab, 0);
    let mut head = vec![0u8; 64];
    assert!(ed.read_bytes(tab, 0.0, &mut head).is_empty());
    assert_eq!(head, noise[..64]);
    let mut tail = vec![0u8; 64];
    assert!(ed.read_bytes(tab, (noise.len() - 64) as f64, &mut tail).is_empty());
    assert_eq!(tail, noise[noise.len() - 64..]);
    assert!(feeder.most_times() <= 2, "a chunk asked for {} times", feeder.most_times());
    // A window of 48 KiB, under a chunk, touches at most two.
    assert!(feeder.widest <= 2, "a reply named {} chunks", feeder.widest);
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

/// Feeds an editor the chunks its "pending" replies name, and asks again on
/// "working", the way the web's listing does, keeping count of how many
/// times each chunk was asked for and of the most chunks one reply named. A
/// chunk can be asked for again once later ones have pushed it out of the
/// store, as when a member is measured and then unpacked; a chunk asked for
/// many times over is the wait that never ends, and fails the test.
struct Feeder<'a> {
    file: &'a [u8],
    times: std::collections::BTreeMap<u64, usize>,
    /// The most chunks any one "pending" reply named.
    widest: usize,
}

impl<'a> Feeder<'a> {
    fn new(file: &'a [u8]) -> Feeder<'a> {
        Feeder { file, times: Default::default(), widest: 0 }
    }

    /// The answer `ask` comes to.
    fn ask(&mut self, ed: &mut Editor, mut ask: impl FnMut(&mut Editor) -> String) -> Value {
        for _ in 0..100_000 {
            let reply = parse(&ask(ed));
            match reply["status"].as_str().unwrap() {
                "pending" => {
                    let chunks = chunks_of(&reply);
                    assert!(!chunks.is_empty(), "a pending reply with nothing to load: {reply}");
                    self.widest = self.widest.max(chunks.len());
                    for c in chunks {
                        let n = self.times.entry(c).or_default();
                        *n += 1;
                        assert!(*n <= 3, "chunk {c} asked for {n} times");
                        feed(ed, self.file, c);
                    }
                }
                "working" => {}
                _ => return reply,
            }
        }
        panic!("no answer");
    }

    /// The most times one chunk was asked for.
    fn most_times(&self) -> usize {
        self.times.values().copied().max().unwrap_or(0)
    }

    /// The children of the node at `path`.
    fn children(&mut self, ed: &mut Editor, path: &[u32]) -> Vec<Value> {
        let reply = self.ask(ed, |ed| ed.template_children(0, path, 0.0, 1e9));
        assert_eq!(reply["status"], "ok", "{reply}");
        reply["node"].as_array().unwrap().clone()
    }

    /// The path of the child of `path` called `name`.
    fn child_named(&mut self, ed: &mut Editor, path: &[u32], name: &str) -> Vec<u32> {
        let found = self.children(ed, path);
        let child = found.iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("no {name} under {path:?}: {found:?}"));
        child["path"].as_array().unwrap().iter().map(|i| i.as_u64().unwrap() as u32).collect()
    }

    /// The node at `path`, which the listing asks for once it has the
    /// children.
    fn node(&mut self, ed: &mut Editor, path: &[u32]) -> Value {
        let reply = self.ask(ed, |ed| ed.template_node(0, path));
        assert_eq!(reply["status"], "ok", "{reply}");
        reply["node"].clone()
    }
}

/// The entry for the zip and gzip tests below: lines of hex that pack to
/// about three times the 16 chunks those tests keep.
fn long_lines() -> Vec<u8> {
    lines(260_000)
}

/// What a tab reads at `at`, `n` bytes of it, with every byte there.
fn tab_bytes(ed: &mut Editor, tab: u32, at: usize, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    assert!(ed.read_bytes(tab, at as f64, &mut out).is_empty(), "the tab's bytes were not all there");
    out
}

/// A deflated zip entry several times longer than the chunk store, listed
/// the way the web lists it: the entry's `data`, then what is inside it. Its
/// run used to be read in one call, which needs more chunks at once than the
/// store keeps; then it was refused for that. Now the pass over it takes its
/// place and reads the run into it a window at a time, a quarter of the
/// store, so the contents list, and Open unpacked gives a tab. The code
/// panel's read of a step near the run's end used to read the run from its
/// front to the step in one call, and was refused for that; now it reads only
/// the bytes the step is in, and nothing for a step that is not a code.
#[test]
fn a_zip_entry_longer_than_the_store_lists_a_window_at_a_time() {
    let text = long_lines();
    let file = zip_of(b"lines.txt", &text);
    assert!(file.len() > 3 * 16 * CHUNK, "{} bytes is too few for a store of 16 chunks", file.len());
    let mut ed = unfed(&file, 16, "zip");
    let mut feeder = Feeder::new(&file);
    let body = feeder.child_named(&mut ed, &[0, 0], "body");
    let data = feeder.child_named(&mut ed, &body, "data");
    let inside = feeder.children(&mut ed, &data);
    assert!(!inside.is_empty(), "nothing listed inside the entry");
    assert!(feeder.node(&mut ed, &data)["refused"].is_null());
    // The run's chunks once each, a window of four at a time.
    assert_eq!(feeder.most_times(), 1, "{:?}", feeder.times);
    assert!(feeder.widest <= 4, "a reply named {} chunks", feeder.widest);
    let reply = feeder.ask(&mut ed, |ed| ed.open_space(0, &data));
    let tab = reply["node"]["space"].as_f64().unwrap_or_else(|| panic!("no tab: {reply}")) as u32;
    assert_eq!(tab_bytes(&mut ed, tab, 0, 64), text[..64]);
    assert_eq!(tab_bytes(&mut ed, tab, text.len() - 64, 64), text[text.len() - 64..]);

    // The trace of a run this long stops naming codes before its end, past
    // `MAX_STEPS`, so a step near the end is a coarse block's one step and
    // not a code. The code panel answers null for it at once, with none of
    // the run in the store, rather than reading the run up to the step.
    let run_bits = feeder.node(&mut ed, &data)["size_bits"].as_f64().unwrap() as u64;
    let packed = miniz_oxide::deflate::compress_to_vec(&text, 1);
    assert_eq!(run_bits, packed.len() as u64 * 8);
    let (_, trace) = codec::decode_traced(Codec::Deflate, &packed).expect("the run inflates");
    let near_end = run_bits - 64;
    let k = trace.index_in(near_end).expect("a step near the end");
    assert!(trace.coarse() && inflate::step_bytes(&trace, k).is_none(), "{:?}", trace.step(k));
    for c in 0..16 {
        feed(&mut ed, &file, c);
    }
    let reply = parse(&ed.decode_step(tab, near_end as f64));
    assert_eq!(reply["status"], "ok", "{reply}");
    assert!(reply["node"].is_null(), "{reply}");
}

/// Whether a code the editor took apart, as the JSON it answers with, is
/// `want`.
fn code_is(got: &Value, want: &inflate::DecodedCode) -> bool {
    let n = |k: &str| got.get(k).and_then(Value::as_f64);
    n("symbol") == Some(f64::from(want.symbol))
        && n("code_bits") == Some(f64::from(want.code_bits))
        && n("extra_bits") == Some(f64::from(want.extra_bits))
        && n("extra") == Some(f64::from(want.extra))
        && n("value") == Some(f64::from(want.value))
        && n("entry") == want.entry.map(f64::from)
        && n("entry_child") == want.entry_child.map(f64::from)
}

/// A code deep in a deflated zip entry several times longer than the chunk
/// store, taken apart for the code panel, with only the chunks of the file's
/// front in the store. The editor reads the bytes the code is in, so its one
/// "pending" reply names only the chunks those bytes are in, not the run's
/// chunks from its front to the code; and once they are fed, what it answers
/// is what taking the code apart against the whole run gives.
#[test]
fn a_code_deep_in_a_run_longer_than_the_store_is_read_from_its_own_bytes() {
    let text = lines(100_000);
    let name = b"lines.txt";
    let file = zip_of(name, &text);
    // The entry's run, which starts after the 30 bytes of its local header
    // and its name.
    let packed = miniz_oxide::deflate::compress_to_vec(&text, 1);
    let run_at = (30 + name.len()) as u64;
    assert_eq!(file[run_at as usize..][..packed.len()], packed[..]);
    let store = 4;
    assert!(packed.len() > 3 * store * CHUNK, "{} bytes is too few for a store of {store} chunks", packed.len());
    let mut ed = unfed(&file, store as u32, "zip");
    let mut feeder = Feeder::new(&file);
    let body = feeder.child_named(&mut ed, &[0, 0], "body");
    let data = feeder.child_named(&mut ed, &body, "data");
    let reply = feeder.ask(&mut ed, |ed| ed.open_space(0, &data));
    let tab = reply["node"]["space"].as_f64().unwrap_or_else(|| panic!("no tab: {reply}")) as u32;

    // A match three quarters of the way along the run, taken apart against
    // the whole of it.
    let (_, trace) = codec::decode_traced(Codec::Deflate, &packed).expect("the run inflates");
    assert!(!trace.coarse(), "the trace stopped naming codes");
    let deep = packed.len() as u64 * 8 * 3 / 4;
    let k = trace.steps().position(|s| matches!(s.kind, StepKind::Match { .. }) && s.in_bits.start >= deep).expect("a match that deep");
    let step = trace.step(k).unwrap();
    let want = inflate::decode_step(&packed, &trace, k).expect("the whole run takes the code apart");
    let want_distance = want.distance.as_ref().expect("a match has a distance");

    // The chunks of the file the code's bits are in, which are past the
    // first `store` chunks; feeding those pushes everything else out.
    let own: Vec<u64> = ((run_at * 8 + step.in_bits.start) / 8 / CHUNK as u64..=(run_at * 8 + step.in_bits.end - 1) / 8 / CHUNK as u64).collect();
    assert!(own[0] >= store as u64, "the code is in chunk {}, among the first {store}", own[0]);
    for c in 0..store as u64 {
        feed(&mut ed, &file, c);
    }
    for &c in &own {
        assert!(!ed.has_chunk(0, c as f64), "chunk {c} is still in the store");
    }
    let bit = step.in_bits.start as f64;
    let reply = parse(&ed.decode_step(tab, bit));
    assert_eq!(reply["status"], "pending", "{reply}");
    assert_eq!(chunks_of(&reply), own, "{reply}");
    for &c in &own {
        feed(&mut ed, &file, c);
    }
    let reply = parse(&ed.decode_step(tab, bit));
    assert_eq!(reply["status"], "ok", "{reply}");
    let got = &reply["node"];
    assert_eq!(got["kind"], "match", "{got}");
    assert_eq!(got["block"].as_f64(), Some(f64::from(want.block)), "{got}");
    assert_eq!(got["block_kind"], want.block_kind.as_str(), "{got}");
    assert_eq!(got["bits"].as_f64(), Some(f64::from(want.bits())), "{got}");
    assert_eq!(want.bits() as u64, step.in_bits.end - step.in_bits.start);
    assert!(code_is(&got["symbol"], &want.symbol), "{got} is not {want:?}");
    assert!(code_is(&got["distance"], want_distance), "{got} is not {want:?}");
    // And the run between the file's front and the code was never loaded.
    for c in store as u64..own[0] {
        assert!(!ed.has_chunk(0, c as f64), "chunk {c} was loaded");
    }
}

/// A .gz of two members, the first longer than the chunk store. A member is
/// measured by inflating it from where its run starts, and that read used to
/// be capped at half the store: a member longer than that measured as
/// nothing, and the template took it to run to the last trailer, so the
/// file read as one member. Now the walk reads its bytes a window at a time
/// as far as the stream goes, so the first member gets its own size, and the
/// fields under it, the joined stream over both members, and the tab over
/// that all read.
#[test]
fn a_gzip_member_longer_than_the_store_is_measured_and_its_stream_reads() {
    let text = long_lines();
    let short = b"and a short member after it\n";
    let first = gzip_fast(&text);
    let file = [first.clone(), gzip_of(short)].concat();
    assert!(first.len() > 3 * 16 * CHUNK, "{} bytes is too few for a store of 16 chunks", first.len());
    let mut ed = unfed(&file, 16, "gzip");
    let mut feeder = Feeder::new(&file);
    assert_eq!(feeder.children(&mut ed, &[0]).len(), 2, "the members");
    let compressed = feeder.child_named(&mut ed, &[0, 0], "compressed");
    let run = feeder.node(&mut ed, &compressed);
    // Everything between the member's ten bytes of header and eight of
    // trailer, and not on to the second member's trailer.
    assert_eq!(run["size_bits"].as_f64().unwrap() as usize, (first.len() - 18) * 8, "{run}");
    assert!(!feeder.children(&mut ed, &compressed).is_empty(), "nothing listed inside the member's stream");
    assert!(feeder.node(&mut ed, &compressed)["refused"].is_null());
    // The file's `decoded`, which joins the members, listed where it is
    // declared: its one child reads the first member's stream as a part.
    assert_eq!(feeder.children(&mut ed, &[1]).len(), 1);
    let reply = feeder.ask(&mut ed, |ed| ed.template_children(0, &[1, 0], 0.0, 1e9));
    assert_eq!(reply["status"], "ok", "{reply}");
    // And Open unpacked on it gives a tab over both members.
    let reply = feeder.ask(&mut ed, |ed| ed.open_space(0, &[1]));
    let tab = reply["node"]["space"].as_f64().unwrap_or_else(|| panic!("no tab: {reply}")) as u32;
    assert_eq!(tab_bytes(&mut ed, tab, 0, 64), text[..64]);
    assert_eq!(tab_bytes(&mut ed, tab, text.len(), short.len()), short);
    assert!(feeder.most_times() <= 3, "{:?}", feeder.times);
    assert!(feeder.widest <= 4, "a reply named {} chunks", feeder.widest);
}

/// A BGZF block, as `formats::bam`'s tests write one: a gzip member with the
/// `BC` extra field giving the block's size.
fn bgzf_block(data: &[u8]) -> Vec<u8> {
    let deflated = miniz_oxide::deflate::compress_to_vec(data, 6);
    let total = 12 + 6 + deflated.len() + 8;
    let mut v = vec![0x1f, 0x8b, 8, 4, 0, 0, 0, 0, 0, 0xff, 6, 0, b'B', b'C', 2, 0];
    v.extend_from_slice(&((total - 1) as u16).to_le_bytes());
    v.extend_from_slice(&deflated);
    v.extend_from_slice(&crc32(data).to_le_bytes());
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v
}

/// The block every BGZF file ends with.
const BGZF_EOF: [u8; 28] = [
    0x1f, 0x8b, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x06, 0x00, 0x42, 0x43, 0x02, 0x00, 0x1b, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00,
];

/// A BGZF file whose blocks come to three times the chunk store, opened as a
/// document of its own. Opening it reads every block, and each block's read
/// fits in the store while all of them do not. The walk over the blocks
/// keeps the blocks read so far from one call to the next, and asks for the
/// chunks of the blocks after the one it waits on as well, up to a window,
/// so it opens in a round a window rather than a round a block.
#[test]
fn a_joined_stream_whose_parts_come_to_more_than_the_store_opens() {
    let pieces: Vec<Vec<u8>> = (0..140).map(|i| lines(2_000 + i)).collect();
    let mut file: Vec<u8> = pieces.iter().flat_map(|p| bgzf_block(p)).collect();
    file.extend_from_slice(&BGZF_EOF);
    assert!(file.len() > 3 * 16 * CHUNK, "{} bytes is too few for a store of 16 chunks", file.len());
    let joined = pieces.concat();
    let mut ed = unfed(&file, 16, "bgzf");
    let mut feeder = Feeder::new(&file);
    let reply = feeder.ask(&mut ed, |ed| ed.open_space(0, &[1]));
    let tab = reply["node"]["space"].as_f64().unwrap_or_else(|| panic!("no tab: {reply}")) as u32;
    assert_eq!(tab_bytes(&mut ed, tab, 0, 64), joined[..64]);
    assert_eq!(tab_bytes(&mut ed, tab, joined.len() - 64, 64), joined[joined.len() - 64..]);
    assert!(feeder.most_times() <= 3, "{:?}", feeder.times);
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
