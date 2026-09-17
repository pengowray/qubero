//! The cursor link between a stream's tab and the file, the way the web asks
//! for it: `map_out` from a byte of the tab, `map_in` from a bit of the file.
//!
//! A step's bits count from the start of the run it was read from, and the
//! step says where that run is in the file (`run_offset_bits`), whether the
//! stream was unpacked from one run or joined from several. The web adds the
//! two to mark the file tab and to write the status bar's decoder line, and
//! `map_in` takes a bit of the file. A gzip's deflate starts after its header,
//! so a link that forgot where the run is marks the magic.
//!
//! The samples live in the collection outside this repository; point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. Without it the tests say so and pass.

use std::path::PathBuf;

use qubero_wasm::Editor;
use serde_json::Value;

/// The file, fed to an editor whole and read as the template it sniffs as, or
/// as `template` when one is named.
fn editor(sample: &str, template: Option<&str>) -> Option<Editor> {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return None;
    };
    // A sample another session has not committed yet is missing from a clean
    // copy of the collection, and says so the same way.
    let Ok(bytes) = std::fs::read(root.join(sample)) else {
        eprintln!("skipped: no {sample} in the sample collection");
        return None;
    };
    let chunk = 64 * 1024;
    let mut ed = Editor::new(bytes.len() as f64, chunk, 1024);
    for (i, part) in bytes.chunks(chunk as usize).enumerate() {
        ed.feed_chunk(i as f64, part);
    }
    let name = match template {
        Some(name) => name.to_string(),
        None => {
            let head = &bytes[..bytes.len().min(ed.sniff_window() as usize)];
            ed.sniff_template(head, bytes.len() as f64, sample)
        }
    };
    assert!(!name.is_empty(), "{sample} sniffs as nothing");
    assert!(ed.set_template(&name), "no template {name}");
    Some(ed)
}

fn json(reply: &str) -> Value {
    let v: Value = serde_json::from_str(reply).unwrap();
    assert_eq!(v["status"], "ok", "{v}");
    v["node"].clone()
}

/// The space a stream of the file opened as from the file's tab, failing on
/// anything but a tab.
fn opened(ed: &mut Editor, path: &[u32]) -> u32 {
    let node = json(&ed.open_space(0, path));
    assert!(node["refused"].is_null(), "opening {path:?} was refused: {node}");
    let space = node["space"].as_f64().unwrap() as u32;
    assert_ne!(space, 0, "opening {path:?} gave no space: {node}");
    space
}

fn node(ed: &mut Editor, path: &[u32]) -> Value {
    json(&ed.template_node(0, path))
}

fn num(v: &Value, key: &str) -> u64 {
    v[key].as_f64().unwrap_or_else(|| panic!("no {key} in {v}")) as u64
}

/// The first stream of the file unpacked from one run that is a field of the
/// file, not of another stream. Not at the file's very start unless `anywhere`:
/// a run at bit 0 is one where forgetting its place goes unseen, and an xz or a
/// zstd file is one run from its first byte.
fn first_single_run(ed: &mut Editor, path: &mut Vec<u32>, depth: usize, anywhere: bool) -> Option<Vec<u32>> {
    let n = node(ed, path);
    if n["decoded"] == true && n["joined"] != true && num(&n, "space") == 0 && n["refused"].is_null() {
        return ((anywhere || num(&n, "offset_bits") > 0) && num(&n, "size_bits") > 0).then(|| path.clone());
    }
    if depth == 0 || n["decoded"] == true {
        return None;
    }
    for i in 0..num(&n, "child_count").min(64) as u32 {
        path.push(i);
        let found = first_single_run(ed, path, depth - 1, anywhere);
        path.pop();
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Where a step marks the file: its bits counted from its run's start, plus
/// where the run is. What the status bar's decoder line prints, too.
fn file_bits(step: &Value) -> (u64, u64) {
    let at = num(step, "run_offset_bits");
    (at + num(step, "in_start"), at + num(step, "in_end"))
}

/// A stream unpacked from one run links to the bits of that run in the file:
/// every byte of the tab to bits inside the run, byte 0 to the first bits the
/// decoder read that made anything, and those bits back to byte 0. A bit of the
/// file before the run is nothing of the stream's.
fn single_run_links_the_file(sample: &str, template: Option<&str>) {
    let Some(mut ed) = editor(sample, template) else { return };
    let path = first_single_run(&mut ed, &mut Vec::new(), 10, false)
        .or_else(|| first_single_run(&mut ed, &mut Vec::new(), 10, true))
        .unwrap_or_else(|| panic!("{sample} has no unpacked stream"));
    let run = node(&mut ed, &path);
    eprintln!("{sample}: {path:?} {} {}, bits {} + {}", run["name"], run["type"], run["offset_bits"], run["size_bits"]);
    let (from, to) = (num(&run, "offset_bits"), num(&run, "offset_bits") + num(&run, "size_bits"));
    let space = opened(&mut ed, &path);
    let len = json(&ed.template_node(space, &[]));
    let len = num(&len, "size_bits") / 8;
    assert!(len > 0, "{sample}: {path:?} unpacks to nothing");

    let mut checked = 0;
    for byte in [0, len / 2, len - 1] {
        let step = json(&ed.map_out(space, byte as f64));
        assert!(!step.is_null(), "{sample}: byte {byte} of {path:?} came from nowhere");
        assert_eq!(num(&step, "run_offset_bits"), from, "{sample}: byte {byte}'s step says its run is elsewhere: {step}");
        let (start, end) = file_bits(&step);
        assert!(from <= start && end <= to, "{sample}: byte {byte} marks bits {start}..{end}, outside the run at {from}..{to}");
        if end > start {
            // And the bit leads back to the same step, asked as a bit of the file.
            let back = json(&ed.map_in(space, start as f64));
            assert!(!back.is_null(), "{sample}: bit {start} of the file maps to nothing");
            assert_eq!((num(&back, "in_start"), num(&back, "out_start")), (num(&step, "in_start"), num(&step, "out_start")), "{sample}: bit {start}");
            assert_eq!(file_bits(&back), (start, end));
            checked += 1;
        }
    }
    assert!(checked > 0, "{sample}: no step read any bits");
    // The last bit before the run is the container's, not the stream's.
    if from > 0 {
        assert!(json(&ed.map_in(space, (from - 1) as f64)).is_null(), "{sample}: bit {} before the run maps into it", from - 1);
        assert!(json(&ed.map_in(space, 0.0)).is_null(), "{sample}: the file's first bit maps into the stream");
    }
    // Nor is a bit past its end.
    assert!(json(&ed.map_in(space, to as f64)).is_null(), "{sample}: bit {to} after the run maps into it");
}

#[test]
fn gzip_byte_0_marks_the_first_literal_not_the_magic() {
    let Some(mut ed) = editor("gzip/gnu-gzip-9-with-name.gz", Some("gzip")) else { return };
    let compressed = [0, 0, 10];
    let run = node(&mut ed, &compressed);
    assert_eq!(run["name"], "compressed");
    let from = num(&run, "offset_bits");
    assert!(from >= 80, "the deflate starts after the ten-byte header, not at bit {from}");
    let space = opened(&mut ed, &compressed);
    let step = json(&ed.map_out(space, 0.0));
    assert_eq!(step["kind"], "literal", "{step}");
    assert_eq!(num(&step, "run_offset_bits"), from);
    // The literal comes after the block's header, three bits for a fixed or
    // a dynamic block and its tables for the second.
    let (start, end) = file_bits(&step);
    assert!(start >= from + 3 && end > start, "byte 0 marks bits {start}..{end} with the run at {from}");
    // The decoder read those bits as that literal: its code, taken apart
    // again, is the byte the tab has at 0. `decode_step` counts in the run.
    let code = json(&ed.decode_step(space, num(&step, "in_start") as f64));
    assert_eq!(code["kind"], "literal", "{code}");
    let mut first = [0u8];
    assert!(ed.read_bytes(space, 0.0, &mut first).is_empty());
    assert_eq!(code["symbol"]["value"].as_f64().map(|v| v as u8), Some(first[0]), "{code}");
    // And the other way: the first bit of that literal in the file is byte 0,
    // and the same number as a bit of the run is somewhere in the header.
    let back = json(&ed.map_in(space, start as f64));
    assert_eq!((num(&back, "out_start"), num(&back, "out_end")), (0, 1), "{back}");
    let header = json(&ed.map_in(space, num(&step, "in_start") as f64));
    assert!(header.is_null(), "bit {} of the file is the gzip header's: {header}", num(&step, "in_start"));
}

#[test]
fn gzip() {
    single_run_links_the_file("gzip/gnu-gzip-9-with-name.gz", Some("gzip"));
}

#[test]
fn zlib_inside_a_png_idat() {
    single_run_links_the_file("pico8/0-saka.p8.png", None);
}

#[test]
fn zip_entry() {
    single_run_links_the_file("compressed/streamed.zip", None);
}

#[test]
fn lz4_block_in_a_parquet_page() {
    single_run_links_the_file("parquet/lz4_raw_compressed.parquet", None);
}

#[test]
fn fastlz_in_a_godot_resource() {
    single_run_links_the_file("godot/godot4-probe-fastlz.res", None);
}

#[test]
fn xz() {
    single_run_links_the_file("compressed/hello.txt.xz", None);
}

#[test]
fn zstd() {
    single_run_links_the_file("compressed/hello.txt.zst", None);
}

#[test]
fn lzma2_in_a_7z() {
    single_run_links_the_file("compressed/nested-dirs-nonsolid.7z", None);
}

#[test]
fn lzma_in_a_7z_header() {
    single_run_links_the_file("compressed/nested-dirs-header-lzma.7z", None);
}

#[test]
fn lzma_in_an_lzip() {
    single_run_links_the_file("compressed/hello.lz", None);
}

#[test]
fn bzip2() {
    single_run_links_the_file("compressed/hello.txt.bz2", None);
}

#[test]
fn lzw_in_a_compress_file() {
    single_run_links_the_file("compressed/words.Z", None);
}

#[test]
fn rar5() {
    single_run_links_the_file("compressed/rar5-one-file.rar", None);
}

#[test]
fn lha() {
    single_run_links_the_file("lha/lha255e-lh5.lzh", None);
}

#[test]
fn cdf_huffman() {
    single_run_links_the_file("cdf/d103a2x-ahuff.cdf", None);
}

/// A stream inside a stream has its run in the bits of that stream, which are
/// nowhere in the file, so its tab marks nothing on the file tab. The IDAT's
/// zlib, which is a field of the file, still does.
#[test]
fn a_stream_inside_a_stream_marks_nothing_on_the_file() {
    let Some(mut ed) = editor("pico8/p8png-test.p8.png", Some("p8png")) else { return };
    let zlib = opened(&mut ed, &[1, 1, 2]);
    assert!(!json(&ed.map_out(zlib, 0.0)).is_null());
    // The cart's packed code, under the IDAT's unfiltered pixels.
    let code_path = [1, 1, 2, 0, 0, 0, 6, 3];
    assert_ne!(num(&node(&mut ed, &code_path), "space"), 0);
    let code = opened(&mut ed, &code_path);
    assert!(json(&ed.map_out(code, 0.0)).is_null());
    let run = node(&mut ed, &code_path);
    assert!(json(&ed.map_in(code, num(&run, "offset_bits") as f64)).is_null());
    assert!(json(&ed.map_in(code, 0.0)).is_null());
}

/// The control: a joined stream already said where each part's run is.
fn joined_links_the_file(sample: &str, template: &str, path: &[u32]) {
    let Some(mut ed) = editor(sample, Some(template)) else { return };
    let reply = json(&ed.open_space(0, path));
    assert_eq!(reply["joined"], true, "{reply}");
    let space = opened(&mut ed, path);
    let step = json(&ed.map_out(space, 0.0));
    assert!(!step.is_null());
    let (start, end) = file_bits(&step);
    assert!(end > start, "{step}");
    let back = json(&ed.map_in(space, start as f64));
    assert_eq!(file_bits(&back), (start, end), "{back}");
    assert_eq!(num(&back, "out_start"), num(&step, "out_start"));
}

#[test]
fn joined_pdb_stream() {
    joined_links_the_file("pdb/msvc-x64-260-modules.pdb", "pdb", &[11, 0, 2, 2, 4, 1]);
}

fn samples() -> Option<PathBuf> {
    let named = std::env::var_os("QUBERO_SAMPLES").map(PathBuf::from);
    let beside = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../qubero-samples"));
    named.into_iter().chain(std::iter::once(beside)).find(|p| p.is_dir())
}
