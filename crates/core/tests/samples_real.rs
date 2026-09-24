//! A smoke test over the sample collection, which lives outside this
//! repository because the files are large and none of them are ours.
//!
//! Point `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`, which is where `tools/build_index.py` in that folder
//! expects to find this one. With neither, the test says so and passes: a
//! checkout without the collection is not a broken checkout.
//!
//! What it checks is that every file still reads: the same template is picked,
//! the root's fields all resolve, and the instructions at both ends of every
//! run of code decode. A decoder crate that changes its mind about a byte
//! shows up here rather than in the editor.

use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::Evaluator;
use qubero_core::formats;
use qubero_core::source::MemSource;

/// Files that do not read yet, by their path under the collection with `/`
/// between the parts. Each is reported and does not fail the test. One that
/// starts reading does fail it, and so does one that is no longer in the
/// collection, so this list cannot go stale.
const KNOWN_FAILURES: &[&str] = &[
    // Lotus 1-2-3 worksheets, release 2 and release 3. No template matches
    // them, because nothing has been written for any Lotus worksheet yet.
    // They stay in the collection as the samples to write one against.
    "wk1/sheetjs-lotus-wk1.wk1",
    "wk3/sheetjs-lotus-wk3.wk3",
    // Word and PowerPoint files in the compound file container. Nothing reads
    // either program's streams, and the container has no template of its own
    // to fall back on: `thumbsdb` and `xls` read it, but only for the one
    // stream each of them is for.
    "doc/libreoffice7-table.doc",
    "doc/word16-table.doc",
    "dot/word16-binary-template.dot",
    "pot/powerpoint16-binary-template.pot",
    "pps/powerpoint16-binary-show.pps",
    "ppt/libreoffice7-one-slide.ppt",
    "ppt/powerpoint16-one-slide.ppt",
    // Excel 2.x, 3.0 and 4.0 worksheets. The `xls` template reads BIFF5 and
    // BIFF8 records only, and these earlier versions number their BOF record
    // differently, so chosen by hand it shows each file as one run of bytes.
    "xls/sheetjs-biff2-single-sheet.xls",
    "xls/sheetjs-biff3-single-sheet.xls",
    "xls/sheetjs-biff4-single-sheet.xls",
    // A bundled Kaitai template exists for each of these, and does not read
    // the sample even when chosen by hand, so naming it by extension would
    // only swap "no template" for an error. `ksy:dicom` stops at the root,
    // because the converter leaves `p_is_transfer_syntax_change_explicit`
    // unresolved. `ksy:openpgp_message` has no case for an old-format packet
    // of indeterminate length, which is what gpg writes for compressed data.
    // Once either reads, it belongs in `KAITAI_BY_EXTENSION` in
    // `formats/recognise.rs`: DICOM has `DICM` 128 bytes in to check.
    "dicom/synthetic-secondary-capture-gray8.dcm",
    "openpgp/gpg-literal-store.gpg",
    // Packet captures. The Kaitai `pcap` was left out of the bundle, because
    // it picks the byte order of the whole file from the magic and the IR has
    // no form for that. Nothing has been written for pcapng.
    "pcap/ethernet-ipv4-udp.pcap",
    "pcapng/ethernet-ipv4-udp.pcapng",
    // Formats that open with a magic, for which no template exists yet. The
    // WebP is a RIFF, and `ksy:riff` would read the outer chunk and nothing
    // inside it.
    "dds/magick-dxt5-rgba.dds",
    "exr/magick-rgb-half.exr",
    "flac/ffmpeg-sine-mono-16k.flac",
    "font/qubero-fixture.woff",
    "font/qubero-fixture.woff2",
    "hdr/magick-rgbe.hdr",
    "jxl/magick-rgba-8x8.jxl",
    "nrrd/volume-int16-gzip.nrrd",
    "nrrd/volume-int16-raw.nrrd",
    "wavpack/ffmpeg-lossless-mono.wv",
    "webp/pillow-rgba-lossless.webp",
    // Text formats. No template reads a file of lines or of XML elements.
    "dif/libreoffice7-dif.dif",
    "fods/libreoffice7-flat-xml.fods",
    "slk/libreoffice7-sylk.slk",
    "xml/sheetjs-spreadsheetml2003.xml",
    "xml/word16-wordprocessingml2003.xml",
];

/// Extensions of files that sit among the samples and say something about
/// them, with what each is. They are not read, because reading them would test
/// nothing about a format. Anything else in the collection is a sample and
/// has to read, a `versions.json` beside a folder of pickles included: that
/// one is JSON, and the JSON template reads it.
const ABOUT_A_SAMPLE: &[(&str, &str)] = &[
    ("md", "notes written for a person"),
    ("tsv", "a list: where a folder's files came from, or which environments wrote each pickle of `pickle-matrix/`"),
    ("py", "a script that made the samples beside it"),
    ("dis", "a binary's own toolchain reading it, the answer key `examples/dis_diff.rs` measures a decoder against"),
];

#[test]
fn every_sample_still_reads() {
    let Some(root) = samples() else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let mut files = Vec::new();
    collect(&root, &mut files);
    assert!(!files.is_empty(), "no files under {}", root.display());
    // `read_dir` hands entries back in whatever order the filesystem keeps
    // them, which differs between machines. Sorted, the same run is the same
    // run everywhere.
    files.sort();
    let (mut read_count, mut refused_count) = (0, 0);
    let (mut failures, mut known) = (Vec::new(), Vec::new());
    let mut known_seen = vec![false; KNOWN_FAILURES.len()];
    for path in files {
        // A file under a folder of this name is kept because Qubero refuses
        // it: nested deeper than anything can read, or ending in the middle
        // of itself. One of those that reads is the failure worth catching.
        let meant_to_fail = path.components().any(|c| c.as_os_str() == "does-not-read");
        let shown = relative(&root, &path);
        let known_at = KNOWN_FAILURES.iter().position(|k| *k == shown);
        if let Some(i) = known_at {
            known_seen[i] = true;
        }
        let (name, out) = attempt(&path);
        let name = name.unwrap_or("no template");
        // Every failure is kept and the test goes on, so one file that does
        // not read cannot hide the files after it.
        match (out, meant_to_fail, known_at.is_some()) {
            (Err(why), true, _) if name == "no template" => failures.push(format!("{shown} ({name}): {why}, so nothing tested it")),
            (Err(_), true, _) => refused_count += 1,
            (Ok(()), true, _) => failures.push(format!("{shown} ({name}): reads, but files in does-not-read should not read")),
            (Err(why), false, true) => known.push(format!("{shown} ({name}): {why}")),
            (Ok(()), false, true) => failures.push(format!("{shown} ({name}): reads now, so take it off KNOWN_FAILURES")),
            (Err(why), false, false) => failures.push(format!("{shown} ({name}): {why}")),
            (Ok(()), false, false) => read_count += 1,
        }
    }
    for (k, seen) in KNOWN_FAILURES.iter().zip(known_seen) {
        if !seen {
            failures.push(format!("{k} (no template): listed in KNOWN_FAILURES but not in the collection"));
        }
    }
    eprintln!("{read_count} samples read, {refused_count} refused as they should be");
    if !known.is_empty() {
        eprintln!("{} known not to read:\n  {}", known.len(), known.join("\n  "));
    }
    assert!(failures.is_empty(), "{} samples failed:\n  {}", failures.len(), failures.join("\n  "));
}

/// Pick a template for one file and read it. The template's name comes back
/// beside the result, or `None` when nothing matched. A panic inside a
/// template is caught and said like any other failure, so it cannot end the
/// walk either.
fn attempt(path: &Path) -> (Option<&'static str>, Result<(), String>) {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return (None, Err(format!("cannot be opened: {e}"))),
    };
    let head = &bytes[..bytes.len().min(0x9000)];
    // A `.COM` file has no header to say what it is, so the extension is
    // what says it. Everything else the file itself announces. The name
    // breaks a tie the bytes cannot, as between `.shp` and `.shx`, and claims
    // a format with no signature, as `.tga` is, only when the bytes agree.
    let name = match path.extension().is_some_and(|e| e.eq_ignore_ascii_case("com")) {
        true => "com",
        false => match formats::sniff_named(head, bytes.len() as u64, &path.to_string_lossy()) {
            Some(name) => name,
            None => return (None, Err("no template matches".to_string())),
        },
    };
    let Some(template) = formats::template(name) else {
        return (Some(name), Err(format!("sniffed as {name}, but there is no template of that name")));
    };
    eprintln!("--- {} as {name}", path.display());
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(template);
        read(&mut ev, &doc, &[], 0)
    }));
    let out = out.unwrap_or_else(|panic| {
        let said = panic.downcast_ref::<String>().map(String::as_str).or_else(|| panic.downcast_ref::<&str>().copied());
        Err(format!("panicked: {}", said.unwrap_or("with no message")))
    });
    (Some(name), out)
}

/// A file's path under the collection, with `/` between the parts on every
/// platform, which is how `KNOWN_FAILURES` spells them.
fn relative(root: &Path, path: &Path) -> String {
    let under = path.strip_prefix(root).unwrap_or(path);
    under.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Resolve a node and enough of what is under it to prove the file reads. A
/// long list is read at both ends rather than throughout: what breaks is the
/// first element or the last, and reading a million rows would make this test
/// something nobody runs.
///
/// The first node that does not read is the answer, said rather than thrown,
/// since a file kept to be refused is one this is supposed to find.
fn read(ev: &mut Evaluator, doc: &Document<MemSource>, path: &[usize], depth: usize) -> Result<(), String> {
    if depth > 4 {
        return Ok(());
    }
    let node = match ev.node(doc, path) {
        Ok(n) => n,
        Err(e) => return Err(format!("{path:?} does not read: {e:?}")),
    };
    let count = node.child_count as usize;
    let ends: Vec<usize> = if count > 8 {
        (0..4).chain(count - 4..count).collect()
    } else {
        (0..count).collect()
    };
    for i in ends {
        let mut child = path.to_vec();
        child.push(i);
        read(ev, doc, &child, depth + 1)?;
    }
    Ok(())
}

fn samples() -> Option<PathBuf> {
    qubero_samples::root()
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tools" || n == ".git") {
                continue;
            }
            // `hexdump/` is not a folder of formats. It holds one file of
            // bytes that are deliberately no format at all and a dozen text
            // captures describing it, which `hexdump_real.rs` reads as what
            // they describe rather than as what they are.
            if path.file_name().is_some_and(|n| n == "hexdump") {
                continue;
            }
            collect(&path, out);
        } else if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            // The repository's own files: the collection is versioned by its
            // lists, and those are not samples.
            continue;
        } else if path.extension().is_some_and(|e| ABOUT_A_SAMPLE.iter().any(|(ext, _)| e == *ext)) {
            continue;
        } else {
            out.push(path);
        }
    }
}
