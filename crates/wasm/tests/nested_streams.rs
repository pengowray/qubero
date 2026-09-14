//! A stream opened from inside a tab, the way the web opens one: by the tab's
//! space and a path of the tab.
//!
//! A tab's paths are the file's only in the file's own tab. A tab over a
//! stream read where it was declared has its fields under the stream in the
//! file's reading, and a tab over a stream whose bytes were recognised has a
//! reading of its own. Both samples live in the collection outside this
//! repository; point `QUBERO_SAMPLES` at it, or keep it beside the repository
//! as `qubero-samples`. Without it the tests say so and pass.

use std::path::PathBuf;

use qubero_wasm::Editor;
use serde_json::Value;

/// The file, fed to an editor whole and read as `template`.
fn editor(sample: &str, template: &str) -> Option<Editor> {
    let Some(root) = samples() else {
        eprintln!("skipped: no sample collection (set QUBERO_SAMPLES)");
        return None;
    };
    let bytes = std::fs::read(root.join(sample)).unwrap();
    let chunk = 64 * 1024;
    let mut ed = Editor::new(bytes.len() as f64, chunk, 1024);
    for (i, part) in bytes.chunks(chunk as usize).enumerate() {
        ed.feed_chunk(i as f64, part);
    }
    assert!(ed.set_template(template), "no template {template}");
    Some(ed)
}

/// The space a stream opened as, failing on anything but a tab.
fn opened(ed: &mut Editor, space: u32, path: &[u32]) -> u32 {
    let reply: Value = serde_json::from_str(&ed.open_space(space, path)).unwrap();
    assert_eq!(reply["status"], "ok", "opening {path:?} from space {space}: {reply}");
    assert!(reply["node"]["refused"].is_null(), "opening {path:?} from space {space} was refused: {reply}");
    let opened = reply["node"]["space"].as_f64().unwrap() as u32;
    assert_ne!(opened, 0, "opening {path:?} from space {space} gave no space: {reply}");
    opened
}

/// What a tab says a field is: its name, type and size.
fn node(ed: &mut Editor, space: u32, path: &[u32]) -> (String, String, u64) {
    let reply: Value = serde_json::from_str(&ed.template_node(space, path)).unwrap();
    assert_eq!(reply["status"], "ok", "{path:?} of space {space}: {reply}");
    let n = &reply["node"];
    (n["name"].as_str().unwrap().to_string(), n["type"].as_str().unwrap().to_string(), n["size_bits"].as_f64().unwrap() as u64)
}

#[test]
fn a_stream_inside_a_declared_stream_opens_from_the_stream_tab() {
    let Some(mut ed) = editor("pico8/p8png-test.p8.png", "p8png") else { return };
    // The IDAT's zlib, then in its tab the PNG unfiltering it holds, then the
    // low bits of the pixels, and in that tab the cart's packed code.
    let zlib = opened(&mut ed, 0, &[1, 1, 2]);
    let unfiltered = opened(&mut ed, zlib, &[]);
    let pixels = opened(&mut ed, unfiltered, &[]);
    let code = opened(&mut ed, pixels, &[6, 3]);
    assert_eq!(node(&mut ed, pixels, &[6, 3]).1, "pico-8 pxa");
    // Each tab reads what the file's own tab reads under the same stream.
    let file = node(&mut ed, 0, &[1, 1, 2, 0, 0, 0, 6, 3, 0]);
    assert_eq!(file.1, "DecodedText");
    assert_eq!(node(&mut ed, code, &[]), file);
    // And it is the same stream the file's tab opens, not a second one.
    assert_eq!(opened(&mut ed, 0, &[1, 1, 2, 0, 0, 0, 6, 3]), code);
    assert_eq!(opened(&mut ed, pixels, &[6, 3]), code);
    // The cursor link marks bits of the file. The IDAT's run is bits of the
    // file; the cart's is bits of the pixels, so it marks nothing there.
    assert!(maps(&ed.map_out(zlib, 0.0)));
    assert!(!maps(&ed.map_out(code, 0.0)));
    assert!(!maps(&ed.map_in(code, 0.0)));
    // An edit to the file opens every stream again, and each tab still reads
    // the stream its title names.
    same_byte_again(&mut ed);
    assert_eq!(node(&mut ed, code, &[]), file);
}

/// Whether a `map_out` or `map_in` reply found a step.
fn maps(reply: &str) -> bool {
    let reply: Value = serde_json::from_str(reply).unwrap();
    assert_eq!(reply["status"], "ok", "{reply}");
    !reply["node"].is_null()
}

/// Write the file's first byte over itself: an edit that changes nothing,
/// which still throws every stream away to be opened again.
fn same_byte_again(ed: &mut Editor) {
    let mut first = [0u8];
    assert!(ed.read_bytes(0, 0.0, &mut first).is_empty());
    ed.overwrite_bytes(0.0, &first);
}

#[test]
fn a_declared_stream_inside_a_recognised_stream_opens_from_its_tab() {
    let Some(mut ed) = editor("zarr/v2-zipstore-group-zlib-stored.zip", "zarrzip") else { return };
    // A zip entry whose bytes are a zlib stream, read as one in a reading of
    // its own, and the compressed run that stream declares.
    let entry = opened(&mut ed, 0, &[0, 4, 1, 14]);
    let inner = node(&mut ed, entry, &[6, 0]);
    let run = opened(&mut ed, entry, &[6]);
    assert_ne!(run, entry);
    assert_eq!(node(&mut ed, run, &[]), inner);
    assert_eq!(opened(&mut ed, entry, &[6]), run);
    assert!(!maps(&ed.map_out(run, 0.0)));
    // The entry is opened again in the file's new reading, and the run in the
    // entry's new reading.
    same_byte_again(&mut ed);
    assert_eq!(node(&mut ed, run, &[]), inner);
}

#[test]
fn a_joined_stream_the_file_declares_still_marks_the_file() {
    let Some(mut ed) = editor("pdb/msvc-x64-260-modules.pdb", "pdb") else { return };
    // The type stream, joined from the pages the directory lists. Its runs
    // are pages of the file wherever they are, so it still marks the file.
    let tpi = [11, 0, 2, 2, 4, 1];
    let reply: Value = serde_json::from_str(&ed.open_space(0, &tpi)).unwrap();
    assert_eq!(reply["node"]["joined"], true, "{reply}");
    let space = opened(&mut ed, 0, &tpi);
    assert!(maps(&ed.map_out(space, 0.0)));
}

fn samples() -> Option<PathBuf> {
    let named = std::env::var_os("QUBERO_SAMPLES").map(PathBuf::from);
    let beside = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../qubero-samples"));
    named.into_iter().chain(std::iter::once(beside)).find(|p| p.is_dir())
}
