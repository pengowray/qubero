//! A joined stream read a part at a time opens without unpacking its parts,
//! so a part that will not unpack is found by the first read of it, and reads
//! as zeroes from then on. Every call that reads a tab's bytes either marks
//! those bytes or refuses with the reason; none hands back the zeroes as if
//! they were what the stream holds.
//!
//! Each test opens a stream of three blocks whose middle block's trailer
//! claims one byte more than the block holds, from a file held in memory.

use qubero_core::checksum::crc32;
use qubero_core::codec::lazy::Limits;
use qubero_wasm::Editor;
use serde_json::Value;

const CHUNK: usize = 64 * 1024;

fn parse(reply: &str) -> Value {
    serde_json::from_str(reply).unwrap()
}

/// Lines of text with a number in hex on each, which pack well.
fn lines(n: usize, tag: usize) -> Vec<u8> {
    (0..n).flat_map(|i| format!("line {tag}.{i}: {:016x}\n", (i as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15)).into_bytes()).collect()
}

/// One BGZF block holding `data`, whose trailer says it holds `claimed` bytes.
fn bgzf_block(data: &[u8], claimed: usize) -> Vec<u8> {
    let deflated = miniz_oxide::deflate::compress_to_vec(data, 6);
    let total = 12 + 6 + deflated.len() + 8;
    let mut v = vec![0x1f, 0x8b, 8, 4, 0, 0, 0, 0, 0, 0xff, 6, 0, b'B', b'C', 2, 0];
    v.extend_from_slice(&((total - 1) as u16).to_le_bytes());
    v.extend_from_slice(&deflated);
    v.extend_from_slice(&crc32(data).to_le_bytes());
    v.extend_from_slice(&(claimed as u32).to_le_bytes());
    v
}

/// The block every BGZF file ends with.
const BGZF_EOF: [u8; 28] = [
    0x1f, 0x8b, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x06, 0x00, 0x42, 0x43, 0x02, 0x00, 0x1b, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00,
];

/// `pieces` as BGZF blocks, the middle one's trailer one byte long.
fn spoiled(pieces: &[Vec<u8>]) -> Vec<u8> {
    let mid = pieces.len() / 2;
    let mut file: Vec<u8> = pieces.iter().enumerate().flat_map(|(i, p)| bgzf_block(p, p.len() + usize::from(i == mid))).collect();
    file.extend_from_slice(&BGZF_EOF);
    file
}

/// An editor over `file` read as `template`, every chunk fed, with a joined
/// stream past 16 KiB read a part at a time.
fn editor(file: &[u8], template: &str) -> Editor {
    // Room for 64 chunks, so that a read of the stream may be as long as the
    // 36 KiB a stream is recognised by.
    let mut ed = Editor::new(file.len() as f64, CHUNK as u32, 64);
    assert!(ed.set_template(template), "no template {template}");
    for (i, chunk) in file.chunks(CHUNK).enumerate() {
        ed.feed_chunk(i as f64, chunk);
    }
    ed.set_stream_limits(Limits::default(), 16 << 10);
    ed
}

/// The tab over the stream at `path`, opened.
fn tab(ed: &mut Editor, path: &[u32]) -> u32 {
    for _ in 0..10_000 {
        let reply = parse(&ed.open_space(0, path));
        match reply["status"].as_str() {
            Some("ok") => return reply["node"]["space"].as_f64().unwrap_or_else(|| panic!("no tab: {reply}")) as u32,
            Some("working") => {}
            _ => panic!("{reply}"),
        }
    }
    panic!("the stream never opened");
}

/// Three blocks of lines, the middle one spoiled, as a BGZF file's joined
/// stream: the editor, the tab, the stream as it lays out (the middle block
/// as zeroes, one byte longer than it holds), where the middle block starts
/// and how long the stream says it is, and what is said of it.
struct Spoiled {
    ed: Editor,
    tab: u32,
    stream: Vec<u8>,
    bad: std::ops::Range<usize>,
    why: String,
}

fn three_blocks() -> Spoiled {
    let pieces: Vec<Vec<u8>> = (0..3).map(|i| lines(600, i)).collect();
    let file = spoiled(&pieces);
    let mut ed = editor(&file, "bgzf");
    let tab = tab(&mut ed, &[1]);
    let n = pieces[1].len();
    let bad = pieces[0].len()..pieces[0].len() + n + 1;
    let stream = [pieces[0].clone(), vec![0; n + 1], pieces[2].clone()].concat();
    assert_eq!(ed.len_bytes(tab), stream.len() as f64);
    let why = format!("blocks[1].compressed unpacks to {n} bytes, not the {} the file says", n + 1);
    Spoiled { ed, tab, stream, bad, why }
}

/// The error a refused call answers with, or a panic naming what came back.
fn refused(reply: &str) -> String {
    let v = parse(reply);
    assert_eq!(v["status"], "error", "not refused: {v}");
    v["message"].as_str().unwrap().to_string()
}

#[test]
fn the_hex_view_reads_the_blocks_either_side_and_is_told_which_bytes_will_not_read() {
    let mut s = three_blocks();
    let (at, len) = (s.bad.start - 50, s.bad.len() + 100);
    let mut out = vec![0xee; len];
    assert!(s.ed.read_bytes(s.tab, at as f64, &mut out).is_empty());
    assert!(out == s.stream[at..at + len], "the bytes around the block differ, or its own are not zeroes");
    let u = parse(&s.ed.unreadable(s.tab, at as f64, len as f64));
    assert_eq!(u["node"]["why"], s.why);
    assert_eq!((u["node"]["at"].as_f64(), u["node"]["len"].as_f64()), (Some(s.bad.start as f64), Some(s.bad.len() as f64)));
    // The same for a read of bits.
    let mut bits = vec![0u8; 16];
    assert!(s.ed.read_bits(s.tab, (s.bad.start * 8) as f64, 128.0, &mut bits).is_empty());
    assert_eq!(parse(&s.ed.unreadable(s.tab, s.bad.start as f64, 16.0))["node"]["why"], s.why);
}

#[test]
fn the_text_view_marks_the_bytes_that_will_not_read_on_the_lines_they_fall_in() {
    let mut s = three_blocks();
    let mut marked = Vec::new();
    let mut from = 0.0;
    let mut starts = Vec::new();
    while from < s.stream.len() as f64 {
        let w = parse(&s.ed.text_window(s.tab, "", from, 64));
        assert!(w["missing"].as_array().unwrap().is_empty());
        let got = w["lines"].as_array().unwrap();
        assert!(!got.is_empty(), "no lines from {from}");
        for l in got {
            let (at, len) = (l["at"].as_f64().unwrap() as usize, l["len"].as_f64().unwrap() as usize);
            starts.push(at as f64);
            let overlaps = at < s.bad.end && at + len > s.bad.start;
            let marks = l["unreadable"].as_array().cloned().unwrap_or_default();
            assert_eq!(!marks.is_empty(), overlaps, "line at {at}: {marks:?}");
            for m in &marks {
                assert_eq!(m["why"], s.why);
                let (mat, mlen) = (m["at"].as_f64().unwrap() as usize, m["len"].as_f64().unwrap() as usize);
                assert!(mat >= at.max(s.bad.start) && mat + mlen <= (at + len).min(s.bad.end), "a mark outside its line or the block");
                marked.push(mat..mat + mlen);
            }
        }
        from = w["next"].as_f64().unwrap();
    }
    // Every byte of the bad block is marked, once.
    assert_eq!(marked.iter().map(|r| r.len()).sum::<usize>(), s.bad.len());
    // The index of line starts agrees with the lines, through the bad block.
    let idx = s.ed.text_index(s.tab, "", 0.0, s.stream.len() as f64);
    let count = idx[4] as usize;
    let indexed: Vec<f64> = idx[5 + count..].to_vec();
    assert_eq!(indexed, starts[..indexed.len()].to_vec(), "line starts differ between the index and the lines");
    // And going back from a line after the bad block finds the same start.
    let after = *starts.iter().find(|&&a| a > s.bad.end as f64 + 100.0).unwrap();
    let back = parse(&s.ed.text_back(s.tab, "", after + 3.0, 0));
    assert_eq!(back["start"].as_f64(), Some(after));
}

#[test]
fn the_strings_scan_finds_no_string_in_the_bytes_that_will_not_read_and_names_them() {
    let mut s = three_blocks();
    let mut from = 0.0;
    let mut said = Vec::new();
    let mut hits = 0;
    while from < s.stream.len() as f64 {
        let scan = parse(&s.ed.strings_scan(s.tab, from, 10_000, 4, "ascii"));
        for h in scan["hits"].as_array().unwrap() {
            let (at, len) = (h["at"].as_f64().unwrap() as usize, h["len"].as_f64().unwrap() as usize);
            assert!(at + len + 1 <= s.bad.start || at >= s.bad.end, "a string at {at} touches the bad block");
            hits += 1;
        }
        for u in scan["unreadable"].as_array().cloned().unwrap_or_default() {
            said.push(u["why"].as_str().unwrap().to_string());
        }
        let next = scan["next"].as_f64().unwrap();
        assert!(next > from, "the scan stood still at {from}");
        from = next;
    }
    assert!(hits > 1000, "{hits} strings found");
    assert!(!said.is_empty() && said.iter().all(|w| *w == s.why), "{said:?}");
}

#[test]
fn a_selection_over_the_bytes_that_will_not_read_is_not_read_as_text() {
    let mut s = three_blocks();
    let at = (s.bad.start - 4) as f64;
    let got = parse(&s.ed.selection_text(s.tab, at, 16.0, "", "Latin-1", "CP437"));
    assert_eq!(got["unreadable"], s.why);
    assert!(got["readings"].as_array().unwrap().is_empty() && got["refused"].as_array().unwrap().is_empty());
    assert_eq!(s.ed.selection_literal(s.tab, at, 16.0, "c"), "");
    // A selection before the block reads.
    let before = parse(&s.ed.selection_text(s.tab, 0.0, 16.0, "", "Latin-1", "CP437"));
    assert!(before.get("unreadable").is_none() && !before["readings"].as_array().unwrap().is_empty());
    assert_ne!(s.ed.selection_literal(s.tab, 0.0, 16.0, "c"), "");
}

#[test]
fn the_overview_of_a_stream_with_bytes_that_will_not_read_is_refused_with_the_reason() {
    let mut s = three_blocks();
    let mut why = None;
    for _ in 0..1000 {
        let r = parse(&s.ed.overview_step(s.tab, 64));
        match r["status"].as_str() {
            Some("ok") if r["node"]["done"] == true => panic!("the overview finished over the zeroes: {r}"),
            Some("ok") => {}
            Some("error") => {
                why = Some(r["message"].as_str().unwrap().to_string());
                break;
            }
            other => panic!("{other:?}: {r}"),
        }
    }
    assert_eq!(why.as_deref(), Some(s.why.as_str()));
    // Every later step is refused at once, rather than scanning from the
    // front to the same block again.
    assert_eq!(refused(&s.ed.overview_step(s.tab, 64)), s.why);
    // A focus on the bad block is refused, and one on the first block reads.
    let focus = s.ed.overview_focus_step(s.tab, s.bad.start as f64, s.bad.end as f64, 16);
    assert_eq!(refused(&focus), s.why);
    let first = parse(&s.ed.overview_focus_step(s.tab, 0.0, 1000.0, 16));
    assert_eq!(first["status"], "ok", "{first}");
}

/// The calls that read a tab's bytes through a format's own reader answer
/// for this stream, which is text and none of their formats, without reading
/// the bad block, and so with nothing made of its zeroes.
#[test]
fn the_format_readers_give_nothing_made_of_the_bytes_that_will_not_read() {
    let mut s = three_blocks();
    let tab = s.tab;
    let answers = [
        s.ed.holds_hdf5(tab),
        s.ed.contents(tab),
        s.ed.btree(tab, &[0], 10),
        s.ed.elf_contents(tab, 10),
        s.ed.root_contents(tab),
        s.ed.hardening(tab),
        s.ed.jpeg_scan(tab, &[0]),
        s.ed.jpeg_block(tab, &[0], 0),
        s.ed.iso_volume(tab),
        s.ed.iso_directory(tab, 0.0, 2048.0, 2048.0, 10, false),
    ];
    for a in answers {
        let v = parse(&a);
        let data = v["status"] == "ok" && !(v["node"].is_null() || v["node"] == false || v["node"]["objects"].as_array().is_some_and(|o| o.is_empty()));
        assert!(!data, "{v}");
    }
}

/// An ISO 9660 image of 24 sectors, its primary volume descriptor at sector
/// 16 and its terminator at sector 22, cut into gzip members of two sectors
/// each, read with the gzip template. Its first 36 KiB, which the stream is
/// recognised by, are whole. The member holding sectors 18 and 19 claims one
/// byte more than it holds, so the descriptors read after the primary one
/// reach it. Without the refusal, `iso_volume` would answer from the primary
/// descriptor as if the zeroes after it were empty sectors.
#[test]
fn the_iso_volume_and_directory_reads_are_refused_where_they_reach_a_bad_member() {
    let mut iso = vec![0u8; 24 * 2048];
    let pvd = &mut iso[16 * 2048..17 * 2048];
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[6] = 1;
    pvd[40..72].fill(b' ');
    pvd[40..46].copy_from_slice(b"RETRO ");
    pvd[128..130].copy_from_slice(&2048u16.to_le_bytes());
    pvd[130..132].copy_from_slice(&2048u16.to_be_bytes());
    let end = &mut iso[22 * 2048..23 * 2048];
    end[0] = 255;
    end[1..6].copy_from_slice(b"CD001");
    end[6] = 1;
    let pieces: Vec<&[u8]> = iso.chunks(4096).collect();
    let mut file: Vec<u8> = pieces.iter().enumerate().flat_map(|(i, p)| bgzf_block(p, p.len() + usize::from(i == 9))).collect();
    file.extend_from_slice(&BGZF_EOF);
    let mut ed = editor(&file, "gzip");
    let tab = tab(&mut ed, &[1]);
    assert_eq!(ed.len_bytes(tab), (iso.len() + 1) as f64);
    let why = "part index 9 unpacks to 4096 bytes, not the 4097 the file says";
    let named = refused(&ed.iso_volume(tab));
    assert!(named == why || named == "members[9].compressed unpacks to 4096 bytes, not the 4097 the file says", "{named}");
    let dir = refused(&ed.iso_directory(tab, 18.0, 2048.0, 2048.0, 10, false));
    assert_eq!(dir, named);
    // A directory in the whole members reads.
    let fine = parse(&ed.iso_directory(tab, 17.0, 2048.0, 2048.0, 10, false));
    assert_eq!(fine["status"], "ok", "{fine}");
}
