//! Real JPEG 2000 files, read segment by segment and box by box, and checked
//! against glymur's reading of the same files.
//!
//! Seven files. Four are conformance codestreams from ITU-T T.803 | ISO/IEC
//! 15444-4, chosen for the segments OpenJPEG does not write: a reserved marker
//! with no parameters in the main header, TLM, POC, CRG, a binary COM and an
//! RGN in a tile-part header, 257 components and so two-byte component
//! numbers, and PPT in sixteen tile-part headers. Three are written by
//! OpenJPEG 2.5.4: sixteen tiles cut into one tile-part per resolution with
//! TLM, PLT, SOP and EPH; a JP2 of the 9-7 wavelet with the image and tile
//! grids off zero; and a JP2 of palette indices with `pclr`, `cmap` and a
//! `res ` superbox, whose boxes glymur wrote.
//!
//! The expected text is glymur 0.14.8's: its codestream parser and its box
//! parser, which read the bytes in Python with no help from OpenJPEG, printed
//! a line at a time in the form [`describe`] prints the template's reading
//! in. Main header segments are a line each. A tile-part is its SOT and where
//! its data starts and how many bytes of it there are, with its header
//! segments indented under it; the data's place is worked out from glymur's
//! SOD and `Psot`. A list is written with runs of the same value counted,
//! `[7*257]`. Each file was also read through by `opj_dump` before it was
//! kept.
//!
//! Beyond what glymur says, two things each file has to agree with itself on:
//! a TLM's lengths are its tile-parts' `Psot`s, in order, and where a
//! tile-part has PLT the packet lengths add up to exactly the data the
//! template gave it.
//!
//! The files live in the sample collection rather than here. Point
//! `QUBERO_SAMPLES` at it, or keep it beside the repository as
//! `qubero-samples`. With neither, the test says so and passes.

use std::path::PathBuf;

use qubero_core::document::Document;
use qubero_core::eval::{Evaluator, NodeInfo, Value};
use qubero_core::formats;
use qubero_core::source::MemSource;

fn sample(name: &str) -> Option<PathBuf> {
    qubero_samples::roots().into_iter().map(|r| r.join("jpeg2000").join(name)).find(|p| p.exists())
}

/// A file read with the template the sniffer picks for it.
struct Reading {
    doc: Document<MemSource>,
    ev: Evaluator,
}

impl Reading {
    fn open(name: &str) -> Option<Reading> {
        let bytes = std::fs::read(sample(name)?).unwrap();
        let sniffed = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64);
        assert_eq!(sniffed, Some("jpeg2000"), "{name}");
        Some(Reading { doc: Document::new(MemSource(bytes)), ev: Evaluator::new(formats::builtin("jpeg2000").unwrap()) })
    }

    fn node(&mut self, path: &[usize]) -> NodeInfo {
        self.ev.node(&self.doc, path).unwrap_or_else(|e| panic!("{path:?}: {e:?}"))
    }

    fn count(&mut self, path: &[usize]) -> usize {
        self.node(path).child_count as usize
    }

    /// The child of `path` called `name`.
    fn child(&mut self, path: &[usize], name: &str) -> Vec<usize> {
        let n = self.count(path);
        let i = (0..n).find(|i| self.node(&[path, &[*i]].concat()).name == name);
        [path, &[i.unwrap_or_else(|| panic!("no {name} under {path:?}"))]].concat()
    }

    fn value(&mut self, path: &[usize], name: &str) -> Value {
        let at = self.child(path, name);
        self.node(&at).value
    }

    fn int(&mut self, path: &[usize], name: &str) -> i128 {
        let at = self.child(path, name);
        let n = self.node(&at);
        n.value.as_int().unwrap_or_else(|| panic!("{name} under {path:?} holds {:?}", n.value))
    }

    /// The elements of a list, each as the path to it.
    fn each(&mut self, path: &[usize]) -> Vec<Vec<usize>> {
        (0..self.count(path)).map(|i| [path, &[i]].concat()).collect()
    }

    fn offset(&mut self, path: &[usize]) -> u64 {
        self.node(path).offset_bits / 8
    }

    fn bytes(&mut self, path: &[usize]) -> u64 {
        self.node(path).size_bits / 8
    }
}

/// A list with runs of the same value written once and counted, the way the
/// expected text writes one.
fn runs<T: ToString>(values: impl IntoIterator<Item = T>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut last: Option<(String, usize)> = None;
    for v in values.into_iter().map(|v| v.to_string()) {
        match &mut last {
            Some((prev, n)) if *prev == v => *n += 1,
            _ => {
                out.extend(last.take().map(|(p, n)| if n > 1 { format!("{p}*{n}") } else { p }));
                last = Some((v, 1));
            }
        }
    }
    out.extend(last.map(|(p, n)| if n > 1 { format!("{p}*{n}") } else { p }));
    format!("[{}]", out.join(","))
}

/// The template's reading of a file, one line per segment or box.
fn describe(r: &mut Reading) -> Vec<String> {
    let mut lines = Vec::new();
    if r.node(&[]).type_name == "Codestream" {
        codestream(r, &[], &mut lines);
        return lines;
    }
    let mut streams = Vec::new();
    boxes(r, &[], 0, &mut lines, &mut streams);
    for stream in streams {
        codestream(r, &stream, &mut lines);
    }
    lines
}

fn boxes(r: &mut Reading, list: &[usize], depth: usize, out: &mut Vec<String>, streams: &mut Vec<Vec<usize>>) {
    for b in r.each(list) {
        let Value::Str(kind) = r.value(&b, "TBox") else { panic!("{b:?} has no type") };
        let contents = r.child(&b, "DBox");
        let mut line = format!("{}{kind}@{} length={}", "  ".repeat(depth), r.offset(&b), r.bytes(&b));
        let f = |r: &mut Reading, name: &str| r.int(&contents, name);
        match kind.as_str() {
            "ftyp" => {
                let text = |r: &mut Reading, p: &[usize]| match r.node(p).value {
                    Value::Str(s) => s,
                    other => panic!("{other:?}"),
                };
                let brand = r.child(&contents, "BR");
                let brand = text(r, &brand);
                let list = r.child(&contents, "CL");
                let compatible: Vec<String> = r.each(&list).iter().map(|p| text(r, p)).collect();
                line += &format!(" BR={brand} MinV={} CL={}", f(r, "MinV"), runs(compatible));
            }
            "ihdr" => {
                line += &format!(
                    " HEIGHT={} WIDTH={} NC={} BPC={} C={} UnkC={} IPR={}",
                    f(r, "HEIGHT"),
                    f(r, "WIDTH"),
                    f(r, "NC"),
                    f(r, "BPC"),
                    f(r, "C"),
                    f(r, "UnkC"),
                    f(r, "IPR")
                );
            }
            "colr" => {
                line += &format!(" METH={} PREC={} APPROX={} EnumCS={}", f(r, "METH"), f(r, "PREC"), f(r, "APPROX"), f(r, "EnumCS"));
            }
            "pclr" => {
                let b_list = r.child(&contents, "B");
                let depths: Vec<i128> = r.each(&b_list).iter().map(|p| r.node(p).value.as_int().unwrap()).collect();
                let entries = r.child(&contents, "C");
                let rows = r.each(&entries);
                let row = |r: &mut Reading, p: &[usize]| -> Vec<i128> { r.each(p).iter().map(|c| r.node(c).value.as_int().unwrap()).collect() };
                let (first, last) = (row(r, &rows[0]), row(r, rows.last().unwrap()));
                line += &format!(" NE={} NPC={} B={} first={} last={}", f(r, "NE"), f(r, "NPC"), runs(depths), runs(first), runs(last));
            }
            "cmap" => {
                let channels: Vec<String> = r
                    .each(&contents)
                    .iter()
                    .map(|c| format!("{},{},{}", r.int(c, "CMP"), r.int(c, "MTYP"), r.int(c, "PCOL")))
                    .collect();
                line += &format!(" channels=[{}]", channels.join(";"));
            }
            "resc" | "resd" => {
                let which = if kind == "resc" { "c" } else { "d" };
                let real = |r: &mut Reading, name: &str| match r.value(&contents, name) {
                    Value::Float(v) => v,
                    other => panic!("{other:?}"),
                };
                line += &format!(" V={:.3} H={:.3}", real(r, &format!("VR{which}")), real(r, &format!("HR{which}")));
            }
            _ => {}
        }
        out.push(line);
        match kind.as_str() {
            "jp2h" | "res " | "uinf" => boxes(r, &contents, depth + 1, out, streams),
            "jp2c" => streams.push(contents),
            _ => {}
        }
    }
}

fn codestream(r: &mut Reading, at: &[usize], out: &mut Vec<String>) {
    let list = r.child(at, "segments");
    for seg in r.each(&list) {
        segment(r, &seg, "", out);
    }
}

/// How each component's coding parameters print, for COD and COC alike.
fn coding(r: &mut Reading, body: &[usize], style: &str) -> String {
    let flagged = r.int(body, style) & 1 == 1;
    let precincts = if flagged {
        let list = r.child(body, "precinct_sizes");
        let sizes: Vec<String> =
            r.each(&list).iter().map(|p| format!("{}x{}", 1u32 << r.int(p, "PPx"), 1u32 << r.int(p, "PPy"))).collect();
        runs(sizes)
    } else {
        "-".into()
    };
    format!(
        "levels={} codeblock={}x{} style={} xform={} precincts={precincts}",
        r.int(body, "decomposition_levels"),
        r.int(body, "code_block_width"),
        r.int(body, "code_block_height"),
        r.int(body, "code_block_style"),
        r.int(body, "transformation"),
    )
}

/// Step sizes as `exponent.mantissa`, where no quantization has no mantissa.
fn steps(r: &mut Reading, body: &[usize], name: &str) -> String {
    let at = r.child(body, name);
    let one = |r: &mut Reading, p: &[usize]| {
        let mantissa = if r.node(p).child_count == 2 && r.node(&[p, &[1]].concat()).name == "mantissa" { r.int(p, "mantissa") } else { 0 };
        format!("{}.{mantissa}", r.int(p, "exponent"))
    };
    if r.int(body, "quantization_style") == 1 {
        return runs([one(r, &at)]);
    }
    let all: Vec<String> = r.each(&at).iter().map(|p| one(r, p)).collect();
    runs(all)
}

fn segment(r: &mut Reading, seg: &[usize], indent: &str, out: &mut Vec<String>) {
    let marker = r.int(seg, "marker");
    let o = r.offset(seg);
    let body = r.child(seg, "body");
    let b = |r: &mut Reading, name: &str| r.int(&body, name);
    let line = match marker {
        0xff51 => {
            let list = r.child(&body, "components");
            let comps = r.each(&list);
            let field = |r: &mut Reading, name: &str| -> Vec<i128> { comps.iter().map(|c| r.int(c, name)).collect() };
            let (ssiz, xr, yr) = (field(r, "Ssiz"), field(r, "XRsiz"), field(r, "YRsiz"));
            format!(
                "SIZ@{o} Rsiz={} Xsiz={} Ysiz={} XOsiz={} YOsiz={} XTsiz={} YTsiz={} XTOsiz={} YTOsiz={} Csiz={} Ssiz={} XRsiz={} YRsiz={}",
                b(r, "Rsiz"),
                b(r, "Xsiz"),
                b(r, "Ysiz"),
                b(r, "XOsiz"),
                b(r, "YOsiz"),
                b(r, "XTsiz"),
                b(r, "YTsiz"),
                b(r, "XTOsiz"),
                b(r, "YTOsiz"),
                b(r, "Csiz"),
                runs(ssiz),
                runs(xr),
                runs(yr)
            )
        }
        0xff52 => format!(
            "COD@{o} Scod={} order={} layers={} mct={} {}",
            b(r, "Scod"),
            b(r, "progression_order"),
            b(r, "layers"),
            b(r, "multiple_component_transform"),
            coding(r, &body, "Scod")
        ),
        0xff53 => format!("COC@{o} Ccoc={} Scoc={} {}", b(r, "Ccoc"), b(r, "Scoc"), coding(r, &body, "Scoc")),
        0xff5c => format!("QCD@{o} guard={} style={} steps={}", b(r, "guard_bits"), b(r, "quantization_style"), steps(r, &body, "SPqcd")),
        0xff5d => format!(
            "QCC@{o} Cqcc={} guard={} style={} steps={}",
            b(r, "Cqcc"),
            b(r, "guard_bits"),
            b(r, "quantization_style"),
            steps(r, &body, "SPqcc")
        ),
        0xff5e => format!("RGN@{o} Crgn={} Srgn={} SPrgn={}", b(r, "Crgn"), b(r, "Srgn"), b(r, "SPrgn")),
        0xff5f => {
            let list = r.child(&body, "changes");
            let changes: Vec<String> = r
                .each(&list)
                .iter()
                .map(|c| ["RSpoc", "CSpoc", "LYEpoc", "REpoc", "CEpoc", "Ppoc"].map(|n| r.int(c, n).to_string()).join(","))
                .collect();
            format!("POC@{o} changes=[{}]", changes.join(";"))
        }
        0xff55 => {
            let list = r.child(&body, "tile_parts");
            let entries = r.each(&list);
            let numbered = r.int(&body, "ST") != 0;
            let ttlm = if numbered { runs(entries.iter().map(|e| r.int(e, "Ttlm")).collect::<Vec<_>>()) } else { "-".into() };
            let ptlm = runs(entries.iter().map(|e| r.int(e, "Ptlm")).collect::<Vec<_>>());
            format!("TLM@{o} Ztlm={} Ttlm={ttlm} Ptlm={ptlm}", b(r, "Ztlm"))
        }
        0xff58 => {
            let list = r.child(&body, "Iplt");
            let lengths: Vec<i128> = r.each(&list).iter().map(|p| r.node(p).value.as_int().unwrap()).collect();
            format!("PLT@{o} Zplt={} packets={} bytes={}", b(r, "Zplt"), lengths.len(), lengths.iter().sum::<i128>())
        }
        0xff61 => {
            let headers = r.child(&body, "Ippt");
            format!("PPT@{o} Zppt={} bytes={}", b(r, "Zppt"), r.bytes(&headers))
        }
        0xff63 => {
            let list = r.child(&body, "offsets");
            let pairs = r.each(&list);
            let (x, y): (Vec<i128>, Vec<i128>) = pairs.iter().map(|p| (r.int(p, "Xcrg"), r.int(p, "Ycrg"))).unzip();
            format!("CRG@{o} Xcrg={} Ycrg={}", runs(x), runs(y))
        }
        0xff64 => {
            let text = r.child(&body, "Ccom");
            match (b(r, "Rcom"), r.node(&text).value) {
                (1, Value::Str(s)) => format!("COM@{o} Rcom=1 text='{s}'"),
                (rcom, _) => format!("COM@{o} Rcom={rcom} bytes={}", r.bytes(&text)),
            }
        }
        0xff90 => {
            let data = r.child(&body, "data");
            let line = format!(
                "SOT@{o} Isot={} Psot={} TPsot={} TNsot={} data@{}+{}",
                b(r, "Isot"),
                b(r, "Psot"),
                b(r, "TPsot"),
                b(r, "TNsot"),
                r.offset(&data),
                r.bytes(&data)
            );
            out.push(format!("{indent}{line}"));
            let header = r.child(&body, "header");
            for inner in r.each(&header) {
                if r.int(&inner, "marker") != 0xff93 {
                    segment(r, &inner, "  ", out);
                }
            }
            return;
        }
        0xffd9 => format!("EOC@{o}"),
        m if (0xff30..=0xff3f).contains(&m) => format!("{m:X}@{o}"),
        m => format!("{m:X}@{o} length={}", r.bytes(&body)),
    };
    out.push(format!("{indent}{line}"));
}

/// Read a file and compare the template's reading with glymur's, line by
/// line, so a failure names the first segment the two disagree on.
fn agrees(name: &str, expected: &str) -> Option<Reading> {
    let Some(mut r) = Reading::open(name) else {
        eprintln!("{}", qubero_samples::missing());
        return None;
    };
    let got = describe(&mut r);
    let want: Vec<&str> = expected.lines().filter(|l| !l.trim().is_empty()).collect();
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g, w, "{name}, line {}", i + 1);
    }
    assert_eq!(got.len(), want.len(), "{name}: {} lines against glymur's {}", got.len(), want.len());
    Some(r)
}

/// The two things a file has to agree with itself on: TLM's lengths are the
/// tile-parts' `Psot`s in order, and a tile-part's PLT lengths add up to its
/// data.
fn self_consistent(r: &mut Reading, at: &[usize]) {
    let list = r.child(at, "segments");
    let (mut tlm, mut psot) = (Vec::new(), Vec::new());
    for seg in r.each(&list) {
        let body = r.child(&seg, "body");
        match r.int(&seg, "marker") {
            0xff55 => {
                let entries = r.child(&body, "tile_parts");
                for e in r.each(&entries) {
                    tlm.push(r.int(&e, "Ptlm"));
                }
            }
            0xff90 => {
                psot.push(r.int(&body, "Psot"));
                let header = r.child(&body, "header");
                let mut packets: Option<i128> = None;
                for inner in r.each(&header) {
                    if r.int(&inner, "marker") == 0xff58 {
                        let lengths = r.child(&[&inner[..], &[1]].concat(), "Iplt");
                        let sum: i128 = r.each(&lengths).iter().map(|p| r.node(p).value.as_int().unwrap()).sum();
                        *packets.get_or_insert(0) += sum;
                    }
                }
                if let Some(sum) = packets {
                    let data = r.child(&body, "data");
                    assert_eq!(sum, r.bytes(&data) as i128, "tile-part at {seg:?}: PLT against its data");
                }
            }
            _ => {}
        }
    }
    if !tlm.is_empty() {
        assert_eq!(tlm, psot, "TLM against the tile-parts");
    }
}

#[test]
fn a_reserved_marker_in_the_main_header_takes_no_length() {
    if let Some(mut r) = agrees("conformance-p0_02-reserved-marker.j2k", P0_02) {
        self_consistent(&mut r, &[]);
    }
}

#[test]
fn tlm_poc_crg_and_an_rgn_in_a_tile_part_header_read_as_glymur_reads_them() {
    if let Some(mut r) = agrees("conformance-p0_03-tlm-poc-crg-rgn.j2k", P0_03) {
        self_consistent(&mut r, &[]);
    }
}

#[test]
fn a_codestream_of_257_components_numbers_them_in_two_bytes() {
    if let Some(mut r) = agrees("conformance-p0_13-257-components.j2k", P0_13) {
        self_consistent(&mut r, &[]);
    }
}

#[test]
fn packed_packet_headers_in_every_tile_part_read_as_glymur_reads_them() {
    if let Some(mut r) = agrees("conformance-p1_06-ppt.j2k", P1_06) {
        self_consistent(&mut r, &[]);
    }
}

#[test]
fn sixteen_tiles_in_forty_eight_tile_parts_are_each_as_long_as_tlm_and_plt_say() {
    if let Some(mut r) = agrees("opj-tile-parts-tlm-plt.j2k", OPJ_TILE_PARTS) {
        self_consistent(&mut r, &[]);
    }
}

#[test]
fn a_jp2_with_its_grids_off_zero_says_the_same_size_in_ihdr_and_siz() {
    let Some(mut r) = agrees("opj-irreversible-offsets.jp2", OPJ_OFFSETS) else { return };
    let header = r.child(&[2], "DBox");
    let ihdr = r.child(&[&header[..], &[0]].concat(), "DBox");
    let codestream = r.child(&[3], "DBox");
    self_consistent(&mut r, &codestream);
    let siz = [&codestream[..], &[1, 0, 1]].concat();
    assert_eq!((r.int(&ihdr, "WIDTH"), r.int(&ihdr, "HEIGHT")), (r.int(&siz, "width"), r.int(&siz, "height")));
    assert_eq!(r.int(&ihdr, "NC"), r.int(&siz, "Csiz"));
    assert_eq!((r.int(&siz, "tiles_across"), r.int(&siz, "tiles_down")), (3, 3));
}

#[test]
fn a_palette_image_reads_as_its_palette_and_resolution_boxes() {
    let Some(mut r) = agrees("glymur-palette-res.jp2", GLYMUR_PALETTE) else { return };
    let codestream = r.child(&[3], "DBox");
    self_consistent(&mut r, &codestream);
    // Every palette entry, not only the ends the expected text has: the colours
    // make_jpeg2000_samples.py wrote.
    let header = r.child(&[2], "DBox");
    let palette = r.child(&[&header[..], &[2]].concat(), "DBox");
    let entries = r.child(&palette, "C");
    for (i, entry) in r.each(&entries).iter().enumerate() {
        let i = i as i128;
        let got: Vec<i128> = r.each(entry).iter().map(|c| r.node(c).value.as_int().unwrap()).collect();
        assert_eq!(got, vec![(i * 17) % 256, (i * 71) % 256, (255 - i * 13) % 256], "entry {i}");
    }
    let xml = r.child(&[4], "DBox");
    assert_eq!(r.node(&xml).value, Value::Str("<note>sixteen colours, drawn by make_jpeg2000_samples.py</note>".into()));
}

const P0_02: &str = r#"
SIZ@2 Rsiz=1 Xsiz=127 Ysiz=126 XOsiz=0 YOsiz=0 XTsiz=127 YTsiz=126 XTOsiz=0 YTOsiz=0 Csiz=1 Ssiz=[7] XRsiz=[2] YRsiz=[1]
COD@45 Scod=6 order=0 layers=6 mct=0 levels=3 codeblock=64x64 style=52 xform=0 precincts=-
COC@59 Ccoc=0 Scoc=0 levels=3 codeblock=32x32 style=52 xform=1 precincts=-
QCD@70 guard=3 style=0 steps=[8.0,9.0*2,10.0,9.0*2,10.0,9.0*2,10.0]
COM@85 Rcom=1 text='Creator: AV-J2K (c) 2000,2001 Algo Vision'
FF30@132
SOT@134 Isot=0 Psot=6047 TPsot=0 TNsot=1 data@148+6033
EOC@6181
"#;

const P0_03: &str = r#"
SIZ@2 Rsiz=1 Xsiz=256 Ysiz=256 XOsiz=0 YOsiz=0 XTsiz=128 YTsiz=128 XTOsiz=0 YTOsiz=0 Csiz=1 Ssiz=[131] XRsiz=[1] YRsiz=[1]
COD@45 Scod=2 order=3 layers=8 mct=0 levels=1 codeblock=64x64 style=0 xform=1 precincts=-
QCD@59 guard=2 style=1 steps=[0.0]
QCC@66 Cqcc=0 guard=2 style=0 steps=[4.0,5.0*2,6.0]
POC@76 changes=[0,0,8,33,255,0]
CRG@87 Xcrg=[65424] Ycrg=[32558]
COM@95 Rcom=1 text='Creator: AV-J2K (c) 2000,2001 Algo Vision'
COM@142 Rcom=1 text='Creator: AV-J2K (c) 2000,2001 Algo Vision Technology'
COM@200 Rcom=0 bytes=62
TLM@268 Ztlm=0 Ttlm=[0,1,2,3] Ptlm=[4267,2117,4080,2081]
SOT@298 Isot=0 Psot=4267 TPsot=0 TNsot=1 data@319+4246
  RGN@310 Crgn=0 Srgn=0 SPrgn=7
SOT@4565 Isot=1 Psot=2117 TPsot=0 TNsot=1 data@4579+2103
SOT@6682 Isot=2 Psot=4080 TPsot=0 TNsot=1 data@6696+4066
SOT@10762 Isot=3 Psot=2081 TPsot=0 TNsot=1 data@10776+2067
EOC@12843
"#;

const P0_13: &str = r#"
SIZ@2 Rsiz=1 Xsiz=1 Ysiz=1 XOsiz=0 YOsiz=0 XTsiz=1 YTsiz=1 XTOsiz=0 YTOsiz=0 Csiz=257 Ssiz=[7*257] XRsiz=[1*257] YRsiz=[1*257]
COD@813 Scod=0 order=1 layers=1 mct=1 levels=1 codeblock=32x32 style=16 xform=1 precincts=-
COC@827 Ccoc=2 Scoc=0 levels=1 codeblock=64x64 style=0 xform=1 precincts=-
QCD@839 guard=2 style=0 steps=[8.0,9.0*2,10.0]
QCC@848 Cqcc=1 guard=3 style=0 steps=[9.0,10.0*2,11.0]
QCC@859 Cqcc=2 guard=2 style=0 steps=[9.0,10.0*2,11.0]
RGN@870 Crgn=3 Srgn=0 SPrgn=11
POC@878 changes=[0,0,1,33,128,1;0,128,1,33,257,4]
COM@900 Rcom=1 text='Creator: AV-J2K (c) 2000,2001 Algo Vision'
SOT@947 Isot=0 Psot=1537 TPsot=0 TNsot=1 data@961+1523
EOC@2484
"#;

const P1_06: &str = r#"
SIZ@2 Rsiz=2 Xsiz=12 Ysiz=12 XOsiz=0 YOsiz=0 XTsiz=3 YTsiz=3 XTOsiz=0 YTOsiz=0 Csiz=3 Ssiz=[7*3] XRsiz=[1*3] YRsiz=[1*3]
COD@51 Scod=6 order=3 layers=1 mct=1 levels=4 codeblock=64x32 style=40 xform=0 precincts=-
QCD@65 guard=3 style=2 steps=[14.1821,14.1845*2,14.1868,13.1925*2,13.2007,11.32*2,11.131,11.2002*2,11.1888]
COM@96 Rcom=1 text='Creator: AV-J2K (c) 2000,2001 Algo Vision'
SOT@143 Isot=0 Psot=349 TPsot=0 TNsot=1 data@268+224
  PPT@155 Zppt=0 bytes=106
SOT@492 Isot=1 Psot=161 TPsot=0 TNsot=1 data@555+98
  PPT@504 Zppt=0 bytes=44
SOT@653 Isot=2 Psot=315 TPsot=0 TNsot=1 data@771+197
  PPT@665 Zppt=0 bytes=99
SOT@968 Isot=3 Psot=203 TPsot=0 TNsot=1 data@1050+121
  PPT@980 Zppt=0 bytes=63
SOT@1171 Isot=4 Psot=178 TPsot=0 TNsot=1 data@1242+107
  PPT@1183 Zppt=0 bytes=52
SOT@1349 Isot=5 Psot=222 TPsot=0 TNsot=1 data@1431+140
  PPT@1361 Zppt=0 bytes=63
SOT@1571 Isot=6 Psot=179 TPsot=0 TNsot=1 data@1641+109
  PPT@1583 Zppt=0 bytes=51
SOT@1750 Isot=7 Psot=191 TPsot=0 TNsot=1 data@1821+120
  PPT@1762 Zppt=0 bytes=52
SOT@1941 Isot=8 Psot=288 TPsot=0 TNsot=1 data@2051+178
  PPT@1953 Zppt=0 bytes=91
SOT@2229 Isot=9 Psot=161 TPsot=0 TNsot=1 data@2292+98
  PPT@2241 Zppt=0 bytes=44
SOT@2390 Isot=10 Psot=216 TPsot=0 TNsot=1 data@2472+134
  PPT@2402 Zppt=0 bytes=63
SOT@2606 Isot=11 Psot=138 TPsot=0 TNsot=1 data@2660+84
  PPT@2618 Zppt=0 bytes=35
SOT@2744 Isot=12 Psot=152 TPsot=0 TNsot=1 data@2806+90
  PPT@2756 Zppt=0 bytes=43
SOT@2896 Isot=13 Psot=176 TPsot=0 TNsot=1 data@2969+103
  PPT@2908 Zppt=0 bytes=54
SOT@3072 Isot=14 Psot=151 TPsot=0 TNsot=1 data@3134+89
  PPT@3084 Zppt=0 bytes=43
SOT@3223 Isot=15 Psot=131 TPsot=0 TNsot=1 data@3276+78
  PPT@3235 Zppt=0 bytes=34
EOC@3354
"#;

const OPJ_TILE_PARTS: &str = r#"
SIZ@2 Rsiz=0 Xsiz=128 Ysiz=128 XOsiz=0 YOsiz=0 XTsiz=32 YTsiz=32 XTOsiz=0 YTOsiz=0 Csiz=3 Ssiz=[7*3] XRsiz=[1*3] YRsiz=[1*3]
COD@51 Scod=7 order=2 layers=1 mct=1 levels=2 codeblock=64x64 style=0 xform=1 precincts=[16x16*3]
QCD@68 guard=2 style=0 steps=[8.0,9.0*2,10.0,9.0*2,10.0]
TLM@80 Ztlm=0 Ttlm=[0*3,1*3,2*3,3*3,4*3,5*3,6*3,7*3,8*3,9*3,10*3,11*3,12*3,13*3,14*3,15*3] Ptlm=[215,166,151,213,141,167,190,125,219,193,143,200,221,161,151,197,163,172,176,120,240,192,131,212,173,109,254,148,127,259,204,164,168,212,159,163,178,113,240,153,127,254,220,146,168,205,147,178]
COM@326 Rcom=1 text='tiles of 32 by 32, one tile-part per resolution'
SOT@379 Isot=0 Psot=215 TPsot=0 TNsot=3 data@401+193
  PLT@391 Zplt=0 packets=3 bytes=193
SOT@594 Isot=0 Psot=166 TPsot=1 TNsot=3 data@616+144
  PLT@606 Zplt=0 packets=3 bytes=144
SOT@760 Isot=0 Psot=151 TPsot=2 TNsot=3 data@791+120
  PLT@772 Zplt=0 packets=12 bytes=120
SOT@911 Isot=1 Psot=213 TPsot=0 TNsot=3 data@933+191
  PLT@923 Zplt=0 packets=3 bytes=191
SOT@1124 Isot=1 Psot=141 TPsot=1 TNsot=3 data@1146+119
  PLT@1136 Zplt=0 packets=3 bytes=119
SOT@1265 Isot=1 Psot=167 TPsot=2 TNsot=3 data@1296+136
  PLT@1277 Zplt=0 packets=12 bytes=136
SOT@1432 Isot=2 Psot=190 TPsot=0 TNsot=3 data@1454+168
  PLT@1444 Zplt=0 packets=3 bytes=168
SOT@1622 Isot=2 Psot=125 TPsot=1 TNsot=3 data@1644+103
  PLT@1634 Zplt=0 packets=3 bytes=103
SOT@1747 Isot=2 Psot=219 TPsot=2 TNsot=3 data@1778+188
  PLT@1759 Zplt=0 packets=12 bytes=188
SOT@1966 Isot=3 Psot=193 TPsot=0 TNsot=3 data@1988+171
  PLT@1978 Zplt=0 packets=3 bytes=171
SOT@2159 Isot=3 Psot=143 TPsot=1 TNsot=3 data@2181+121
  PLT@2171 Zplt=0 packets=3 bytes=121
SOT@2302 Isot=3 Psot=200 TPsot=2 TNsot=3 data@2333+169
  PLT@2314 Zplt=0 packets=12 bytes=169
SOT@2502 Isot=4 Psot=221 TPsot=0 TNsot=3 data@2524+199
  PLT@2514 Zplt=0 packets=3 bytes=199
SOT@2723 Isot=4 Psot=161 TPsot=1 TNsot=3 data@2745+139
  PLT@2735 Zplt=0 packets=3 bytes=139
SOT@2884 Isot=4 Psot=151 TPsot=2 TNsot=3 data@2915+120
  PLT@2896 Zplt=0 packets=12 bytes=120
SOT@3035 Isot=5 Psot=197 TPsot=0 TNsot=3 data@3057+175
  PLT@3047 Zplt=0 packets=3 bytes=175
SOT@3232 Isot=5 Psot=163 TPsot=1 TNsot=3 data@3254+141
  PLT@3244 Zplt=0 packets=3 bytes=141
SOT@3395 Isot=5 Psot=172 TPsot=2 TNsot=3 data@3426+141
  PLT@3407 Zplt=0 packets=12 bytes=141
SOT@3567 Isot=6 Psot=176 TPsot=0 TNsot=3 data@3589+154
  PLT@3579 Zplt=0 packets=3 bytes=154
SOT@3743 Isot=6 Psot=120 TPsot=1 TNsot=3 data@3765+98
  PLT@3755 Zplt=0 packets=3 bytes=98
SOT@3863 Isot=6 Psot=240 TPsot=2 TNsot=3 data@3894+209
  PLT@3875 Zplt=0 packets=12 bytes=209
SOT@4103 Isot=7 Psot=192 TPsot=0 TNsot=3 data@4125+170
  PLT@4115 Zplt=0 packets=3 bytes=170
SOT@4295 Isot=7 Psot=131 TPsot=1 TNsot=3 data@4317+109
  PLT@4307 Zplt=0 packets=3 bytes=109
SOT@4426 Isot=7 Psot=212 TPsot=2 TNsot=3 data@4457+181
  PLT@4438 Zplt=0 packets=12 bytes=181
SOT@4638 Isot=8 Psot=173 TPsot=0 TNsot=3 data@4660+151
  PLT@4650 Zplt=0 packets=3 bytes=151
SOT@4811 Isot=8 Psot=109 TPsot=1 TNsot=3 data@4833+87
  PLT@4823 Zplt=0 packets=3 bytes=87
SOT@4920 Isot=8 Psot=254 TPsot=2 TNsot=3 data@4951+223
  PLT@4932 Zplt=0 packets=12 bytes=223
SOT@5174 Isot=9 Psot=148 TPsot=0 TNsot=3 data@5196+126
  PLT@5186 Zplt=0 packets=3 bytes=126
SOT@5322 Isot=9 Psot=127 TPsot=1 TNsot=3 data@5344+105
  PLT@5334 Zplt=0 packets=3 bytes=105
SOT@5449 Isot=9 Psot=259 TPsot=2 TNsot=3 data@5480+228
  PLT@5461 Zplt=0 packets=12 bytes=228
SOT@5708 Isot=10 Psot=204 TPsot=0 TNsot=3 data@5730+182
  PLT@5720 Zplt=0 packets=3 bytes=182
SOT@5912 Isot=10 Psot=164 TPsot=1 TNsot=3 data@5934+142
  PLT@5924 Zplt=0 packets=3 bytes=142
SOT@6076 Isot=10 Psot=168 TPsot=2 TNsot=3 data@6107+137
  PLT@6088 Zplt=0 packets=12 bytes=137
SOT@6244 Isot=11 Psot=212 TPsot=0 TNsot=3 data@6266+190
  PLT@6256 Zplt=0 packets=3 bytes=190
SOT@6456 Isot=11 Psot=159 TPsot=1 TNsot=3 data@6478+137
  PLT@6468 Zplt=0 packets=3 bytes=137
SOT@6615 Isot=11 Psot=163 TPsot=2 TNsot=3 data@6646+132
  PLT@6627 Zplt=0 packets=12 bytes=132
SOT@6778 Isot=12 Psot=178 TPsot=0 TNsot=3 data@6800+156
  PLT@6790 Zplt=0 packets=3 bytes=156
SOT@6956 Isot=12 Psot=113 TPsot=1 TNsot=3 data@6978+91
  PLT@6968 Zplt=0 packets=3 bytes=91
SOT@7069 Isot=12 Psot=240 TPsot=2 TNsot=3 data@7100+209
  PLT@7081 Zplt=0 packets=12 bytes=209
SOT@7309 Isot=13 Psot=153 TPsot=0 TNsot=3 data@7331+131
  PLT@7321 Zplt=0 packets=3 bytes=131
SOT@7462 Isot=13 Psot=127 TPsot=1 TNsot=3 data@7484+105
  PLT@7474 Zplt=0 packets=3 bytes=105
SOT@7589 Isot=13 Psot=254 TPsot=2 TNsot=3 data@7620+223
  PLT@7601 Zplt=0 packets=12 bytes=223
SOT@7843 Isot=14 Psot=220 TPsot=0 TNsot=3 data@7865+198
  PLT@7855 Zplt=0 packets=3 bytes=198
SOT@8063 Isot=14 Psot=146 TPsot=1 TNsot=3 data@8085+124
  PLT@8075 Zplt=0 packets=3 bytes=124
SOT@8209 Isot=14 Psot=168 TPsot=2 TNsot=3 data@8240+137
  PLT@8221 Zplt=0 packets=12 bytes=137
SOT@8377 Isot=15 Psot=205 TPsot=0 TNsot=3 data@8399+183
  PLT@8389 Zplt=0 packets=3 bytes=183
SOT@8582 Isot=15 Psot=147 TPsot=1 TNsot=3 data@8604+125
  PLT@8594 Zplt=0 packets=3 bytes=125
SOT@8729 Isot=15 Psot=178 TPsot=2 TNsot=3 data@8760+147
  PLT@8741 Zplt=0 packets=12 bytes=147
EOC@8907
"#;

const GLYMUR_PALETTE: &str = r#"
jP  @0 length=12
ftyp@12 length=20 BR=jp2  MinV=0 CL=[jp2 ]
jp2h@32 length=171
  ihdr@40 length=22 HEIGHT=48 WIDTH=64 NC=1 BPC=7 C=7 UnkC=0 IPR=0
  colr@62 length=15 METH=1 PREC=0 APPROX=0 EnumCS=16
  pclr@77 length=62 NE=16 NPC=3 B=[7*3] first=[0*2,255] last=[255,41,60]
  cmap@139 length=20 channels=[0,1,0;0,1,1;0,1,2]
  res @159 length=44
    resc@167 length=18 V=11811.000 H=11811.000
    resd@185 length=18 V=2834.600 H=5669.300
jp2c@203 length=321
xml @524 length=71
SIZ@213 Rsiz=0 Xsiz=64 Ysiz=48 XOsiz=0 YOsiz=0 XTsiz=64 YTsiz=48 XTOsiz=0 YTOsiz=0 Csiz=1 Ssiz=[7] XRsiz=[1] YRsiz=[1]
COD@256 Scod=0 order=0 layers=1 mct=0 levels=2 codeblock=64x64 style=0 xform=1 precincts=-
QCD@270 guard=2 style=0 steps=[8.0,9.0*2,10.0,9.0*2,10.0]
COM@282 Rcom=1 text='Created by OpenJPEG version 2.5.4'
SOT@321 Isot=0 Psot=201 TPsot=0 TNsot=1 data@335+187
EOC@522
"#;

const OPJ_OFFSETS: &str = r#"
jP  @0 length=12
ftyp@12 length=20 BR=jp2  MinV=0 CL=[jp2 ]
jp2h@32 length=45
  ihdr@40 length=22 HEIGHT=128 WIDTH=128 NC=3 BPC=7 C=7 UnkC=0 IPR=0
  colr@62 length=15 METH=1 PREC=0 APPROX=0 EnumCS=16
jp2c@77 length=3953
SIZ@87 Rsiz=0 Xsiz=135 Ysiz=131 XOsiz=7 YOsiz=3 XTsiz=64 YTsiz=64 XTOsiz=2 YTOsiz=1 Csiz=3 Ssiz=[7*3] XRsiz=[1*3] YRsiz=[1*3]
COD@136 Scod=0 order=0 layers=1 mct=1 levels=3 codeblock=64x64 style=0 xform=0 precincts=-
QCD@150 guard=2 style=2 steps=[12.1848,12.1872*2,12.1896,10.5*2,10.71,10.2003*2,10.1890]
COM@175 Rcom=1 text='Created by OpenJPEG version 2.5.4'
SOT@214 Isot=0 Psot=704 TPsot=0 TNsot=1 data@228+690
SOT@918 Isot=1 Psot=978 TPsot=0 TNsot=1 data@932+964
SOT@1896 Isot=2 Psot=65 TPsot=0 TNsot=1 data@1910+51
SOT@1961 Isot=3 Psot=929 TPsot=0 TNsot=1 data@1975+915
SOT@2890 Isot=4 Psot=936 TPsot=0 TNsot=1 data@2904+922
SOT@3826 Isot=5 Psot=71 TPsot=0 TNsot=1 data@3840+57
SOT@3897 Isot=6 Psot=44 TPsot=0 TNsot=1 data@3911+30
SOT@3941 Isot=7 Psot=44 TPsot=0 TNsot=1 data@3955+30
SOT@3985 Isot=8 Psot=43 TPsot=0 TNsot=1 data@3999+29
EOC@4028
"#;
