//! What the core gives the report view, read against real files: the format
//! profile, the byte ledger, the extent audit and the directories.
//!
//! Most of the files are the ones the eight hand-written reports were made
//! from (see `docs/DESIGN-report-view.md`), so the numbers here are the ones
//! those reports checked two ways. The rest are the damaged files the sample
//! collection keeps for the reader's sake: a WAV whose RIFF size overshoots
//! the file, one whose RIFF size stops short of its last chunk, and a MAT file
//! whose first element runs past the end.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, each test says so and passes.

use qubero_core::document::Document;
use qubero_core::eval::{template_profile, Evaluator, KindWalk, Ledger, ReportWalk};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn open(kind: &str, name: &str, template: &str) -> Option<(Document<MemSource>, Evaluator)> {
    let path = qubero_samples::roots().into_iter().map(|r| r.join(kind).join(name)).find(|p| p.exists())?;
    let doc = Document::new(MemSource(std::fs::read(&path).unwrap()));
    Some((doc, Evaluator::new(formats::template(template).expect("a template"))))
}

/// The walk, run to its end in goes of `slice` elements, or in one.
fn walk(doc: &Document<MemSource>, ev: &mut Evaluator, slice: Option<u64>) -> ReportWalk {
    ev.set_slice(slice);
    let mut w = ReportWalk::new(doc.len_bits());
    for _ in 0..1_000_000 {
        ev.begin_slice();
        ev.report_step(doc, &mut w).expect("a step");
        if w.done() {
            return w;
        }
    }
    panic!("the walk did not finish");
}

/// The bits of every row in `group` with role `role`.
fn bits(l: &Ledger, group: &str, role: &str) -> u64 {
    l.rows.iter().filter(|r| r.group == group && r.role == role).map(|r| r.bits).sum()
}

#[test]
fn a_midi_file_groups_its_events_by_what_they_are_and_keeps_its_lengths_apart() {
    let Some((doc, mut ev)) = open("midi", "format1-smpte-three-tracks.mid", "midi") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let w = walk(&doc, &mut ev, None);
    let l = w.ledger();
    assert!(l.done);
    // Every bit in exactly one row.
    assert_eq!(l.counted_bits, l.file_bits);
    // Meta events are grouped by the status that picked them, and their
    // bytes are content; the chunk IDs and sizes are machinery of their
    // chunk.
    assert!(bits(&l, "meta", "content") > 0);
    assert!(bits(&l, "note on ch2", "content") > 0);
    // `MThd`, its size, and the bit that says the division is in frames,
    // which picks how the rest of the division reads.
    assert_eq!(bits(&l, "MThd", "machinery"), 8 * 8 + 1);
    assert_eq!(l.rows.iter().filter(|r| r.role == "gap").count(), 0);
    // The file profile: big-endian 32-bit chunk sizes, and MIDI's own
    // variable-length numbers, one per event at least.
    let p = w.profile();
    assert!(p.done);
    let size = p.rows.iter().find(|r| r.category == "number" && r.width == 32).expect("32-bit numbers");
    assert_eq!((size.order, size.fields), ("big", 4));
    let vlq = p.rows.iter().find(|r| r.category == "varint" && r.kind == "vlq").expect("vlq");
    assert!(vlq.fields >= 20, "{} vlqs", vlq.fields);
    assert_eq!(p.facts.every_field_follows, Some(true));
}

#[test]
fn a_sqlite_file_groups_its_pages_and_finds_the_free_space_in_them() {
    let Some((doc, mut ev)) = open("sqlite", "page512-overflow-freelist.db", "sqlite") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let w = walk(&doc, &mut ev, None);
    let l = w.ledger();
    assert!(l.done);
    assert_eq!(l.counted_bits, l.file_bits, "every byte once, cells placed over free space included");
    // Pages group by what the template reads them as: leaf, interior,
    // overflow and free. 26 free pages are leaves of the freelist, whole.
    assert_eq!(bits(&l, "FreelistLeaf", "content"), 26 * 512 * 8);
    assert!(bits(&l, "Overflow", "content") > 0);
    // The free space in the B-tree pages is a gap, nearly all of it zero.
    let leaf = l.rows.iter().find(|r| r.group == "TableLeaf" && r.role == "gap").expect("gaps in leaf pages");
    assert_eq!(leaf.unscanned_bits, 0);
    assert!(leaf.zero_bits > 0 && leaf.zero_bits <= leaf.bits);
    // Every length in the file fits the part it sizes, a spilled payload
    // included: its cell holds only what the page keeps, as the format says.
    let a = w.audit();
    assert!(a.checks.is_empty(), "{:?}", a.checks);
    assert!(a.counts.iter().any(|(v, n)| *v == "fits" && *n > 0));
    // Each page's cell pointers are a directory of its cells.
    let d = w.directories();
    assert!(d.done);
    let first = d.lists.iter().find(|l| l.name == "cell_pointers").expect("cell pointers");
    assert_eq!(first.entries[0].targets[0].via, "offsets");
}

#[test]
fn a_riff_size_past_the_end_of_the_file_is_found() {
    let Some((doc, mut ev)) = open("wav", "xc1060673-kuhls-pipistrelle-data-size-short.wav", "wav") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let w = walk(&doc, &mut ev, None);
    let a = w.audit();
    let riff = a.checks.iter().find(|c| c.length_name == "size" && c.part_name == "chunks").expect("the RIFF size");
    assert_eq!(riff.verdict, "past-file");
    assert!(riff.length_invalid, "the template's own bound on it fails too");
    assert_eq!(riff.stated.unwrap() - riff.read.unwrap(), 8 * 8, "eight bytes past the end");
    // The D500X block is the data chunk's, and so is every sample.
    let l = w.ledger();
    assert!(bits(&l, "data", "content") >= 300_980 * 8);
}

#[test]
fn a_riff_size_short_of_its_last_chunk_is_stretched_and_the_chunk_read() {
    let Some((doc, mut ev)) = open("wav", "guano-past-riff-size-pcm16.wav", "wav") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let w = walk(&doc, &mut ev, None);
    let a = w.audit();
    let riff = a.checks.iter().find(|c| c.part_name == "chunks").expect("the RIFF size");
    assert_eq!(riff.verdict, "stretched");
    assert_eq!(riff.read.unwrap() - riff.stated.unwrap(), 40 * 8);
    let l = w.ledger();
    assert_eq!(bits(&l, "guan", "content"), 160 * 8, "the GUANO text is read, not left a gap");
    assert_eq!(l.rows.iter().filter(|r| r.role == "gap").count(), 0);
    // The kind totals take the chunk in too, as the listing does.
    let mut ev = Evaluator::new(formats::template("wav").unwrap());
    let mut kinds = KindWalk::new(doc.len_bits());
    let t = ev.kind_totals_step(&doc, &mut kinds).expect("totals");
    assert!(t.done);
    assert_eq!((t.covered_bits, t.unmapped_bits), (doc.len_bits(), 0));
}

#[test]
fn a_mat_file_whose_first_element_runs_past_the_end_says_so() {
    let Some((doc, mut ev)) = open("mat/does-not-read", "malformed1.mat", "mat") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let w = walk(&doc, &mut ev, None);
    let a = w.audit();
    assert!(a.done);
    assert!(a.root_failed.is_some());
    let c = &a.checks[0];
    assert_eq!((c.verdict, c.length_name.as_str(), c.length_value), ("past-file", "bytes", Some(658_840)));
}

#[test]
fn a_zip_central_directory_points_at_each_local_header() {
    let Some((doc, mut ev)) = open("docx", "word16-table.docx", "zip") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let w = walk(&doc, &mut ev, None);
    let d = w.directories();
    assert!(d.done);
    let records = d.lists.iter().find(|l| l.path == vec![0]).expect("the records");
    assert_eq!(records.placing, 11);
    for e in &records.entries {
        let t = &e.targets[0];
        assert_eq!((t.via, t.aside), ("address", true));
        let file = e.name.split_once(' ').map(|(_, n)| n).expect("an entry names its file");
        assert!(t.name.ends_with(file), "{} lists {}", e.name, t.name);
        assert!(t.offset_bits < e.offset_bits, "the directory is after what it lists");
    }
    let p = w.profile();
    assert_eq!(p.facts.backward, 11);
    assert_eq!(p.facts.every_field_follows, Some(false));
    // The local headers are counted once, in the run of records.
    assert_eq!(w.ledger().counted_bits, doc.len_bits());
}

#[test]
fn elf_section_headers_point_at_their_names_and_their_sections() {
    let Some((doc, mut ev)) = open("elf", "busybox-x86_64", "elf") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let w = walk(&doc, &mut ev, None);
    let d = w.directories();
    let headers = d.lists.iter().find(|l| l.name == "section_headers").expect("the section headers");
    let text = headers.entries.iter().find(|e| e.name.ends_with(".text")).expect(".text");
    let vias: Vec<&str> = text.targets.iter().map(|t| t.via).collect();
    assert!(vias.contains(&"address") && vias.contains(&"offsets"), "{vias:?}");
    let code = text.targets.iter().find(|t| t.via == "offsets").unwrap();
    assert_eq!(code.size_bits, 890_662 * 8);
    // Parts are the header's own fields, since the header holds nearly all of
    // the file, and the sections are grouped by name.
    let l = w.ledger();
    let sections = l.rows.iter().find(|r| r.group == ".text").expect(".text in the ledger");
    assert_eq!(sections.part_name, "sections");
    assert_eq!(l.counted_bits, l.file_bits);
    let p = w.profile();
    assert!(p.facts.backward > 0, "the section headers come after the sections");
    let code = p.rows.iter().find(|r| r.category == "opaque" && r.kind == "code").expect("machine code");
    assert_eq!(code.detail, "x86-64");
}

#[test]
fn tiff_entries_point_at_the_values_that_do_not_fit_in_them() {
    let Some((doc, mut ev)) = open("tiff", "libtiff-quad-tile-jpeg-ii.tiff", "tiff") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let w = walk(&doc, &mut ev, None);
    let d = w.directories();
    let entries = d.lists.iter().find(|l| l.name == "entries").expect("the IFD entries");
    assert_eq!((entries.elements, entries.placing), (18, 8));
    let offsets = entries.entries.iter().find(|e| e.name.ends_with("tile offsets")).expect("tile offsets");
    assert_eq!(offsets.targets[0].size_bits, 48 * 8);
}

#[test]
fn a_walk_in_small_goes_answers_what_one_go_does() {
    let Some((doc, mut ev)) = open("sqlite", "page512-overflow-freelist.db", "sqlite") else {
        eprintln!("{}", qubero_samples::missing());
        return;
    };
    let whole = walk(&doc, &mut ev, None);
    let mut ev = Evaluator::new(formats::template("sqlite").unwrap());
    let stepped = walk(&doc, &mut ev, Some(50));
    assert_eq!(whole.ledger(), stepped.ledger());
    assert_eq!(whole.profile(), stepped.profile());
    assert_eq!(whole.audit(), stepped.audit());
    assert_eq!(whole.directories(), stepped.directories());
}

#[test]
fn a_template_profile_counts_what_png_declares() {
    let t = formats::template("png").unwrap();
    let p = template_profile(&t);
    assert!(p.done);
    assert!(p.rows.iter().any(|r| r.category == "checksum" && r.kind == "crc32"));
    assert!(p.rows.iter().any(|r| r.category == "number" && r.width == 32 && r.order == "big"));
    let data = p.choices.iter().find(|c| c.name == "data").expect("the chunk data switch");
    assert!(data.cases >= 2);
    assert_eq!(p.facts.every_field_follows, Some(true));
}
