//! Open every stream a listing would offer to open as a tab, and say which of
//! those tabs does not read.
//!
//! Every sample is read with the template it sniffs as, the tree is walked for
//! the first node inside each stream, which is the row the listing hangs its
//! Open button on, and each of those streams is opened the way the web opens
//! one: a stream read by the template it declared is read where it was
//! declared, through [`Tab`], and one whose bytes were recognised gets a fresh
//! reading over a copy of them.
//!
//! `fresh` as a sixth argument reads every tab the way they were all read
//! before, with a fresh reading of the template the space settled on over a
//! copy of its bytes. That reading has nothing above its root, so a type that
//! names a field of the structure that declared the stream fails there, and
//! this is how many did.
//!
//! `cargo run -p qubero-core --example tab_probe -- <dir> [template|-] [streams per file] [nodes per file] [depth] [fresh]`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use qubero_core::document::Document;
use qubero_core::eval::{EvalError, Evaluator, Tab, View};
use qubero_core::formats;
use qubero_core::source::{MemSource, Source};

/// How many nodes of one file the walk looking for streams may read, unless
/// the fourth argument says otherwise.
const NODES_PER_FILE: usize = 6000;
/// How many children of one node the walk looks at: the first ones and the
/// last few, since what breaks is usually at one end or the other.
const FIRST_CHILDREN: usize = 32;
const LAST_CHILDREN: usize = 4;
/// How deep the walk goes, in path components, unless the fifth argument says
/// otherwise.
const DEPTH: usize = 14;
/// How many children of a tab's root are read after the root itself.
const TAB_CHILDREN: u64 = 8;

#[derive(Default)]
struct Tally {
    ok: usize,
    root: usize,
    child: usize,
    refused: usize,
    error: usize,
}

fn main() {
    // A debug build's frames are several times a release build's, and the
    // main thread on Windows has a megabyte. See `DEEPEST_PATH` in `eval`.
    let sweep = std::thread::Builder::new().stack_size(256 << 20).spawn(sweep).expect("a thread to sweep on");
    sweep.join().expect("the sweep finishes");
}

fn sweep() {
    let root = std::env::args().nth(1).expect("usage: tab_probe <dir> [template|-] [streams per file] [nodes per file] [depth] [fresh]");
    let only = std::env::args().nth(2).filter(|a| a != "-");
    let cap: usize = std::env::args().nth(3).and_then(|d| d.parse().ok()).unwrap_or(24);
    let nodes: usize = std::env::args().nth(4).and_then(|d| d.parse().ok()).unwrap_or(NODES_PER_FILE);
    let depth: usize = std::env::args().nth(5).and_then(|d| d.parse().ok()).unwrap_or(DEPTH);
    let fresh = std::env::args().nth(6).is_some_and(|a| a == "fresh");
    let mut files = Vec::new();
    collect(Path::new(&root), &mut files);
    files.sort();
    let mut total = Tally::default();
    let mut by_template: BTreeMap<String, Tally> = BTreeMap::new();
    for path in files {
        if path.components().any(|c| c.as_os_str() == "does-not-read") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Some(name) = formats::sniff(&bytes[..bytes.len().min(formats::SNIFF_WINDOW)], bytes.len() as u64) else {
            continue;
        };
        if only.as_deref().is_some_and(|want| want != name) {
            continue;
        }
        let Some(template) = formats::template(name) else { continue };
        let doc = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(template);
        let mut streams = Vec::new();
        let mut budget = nodes;
        find_streams(&mut ev, &doc, &[], &mut budget, (cap, depth), &mut streams);
        for (stream, tab_name) in streams {
            let what = format!("{} as {name} {stream:?} {tab_name}", path.display());
            let tally = by_template.entry(name.to_string()).or_default();
            let id = match ev.open_space(&doc, 0, &stream) {
                Ok(Some(id)) => id,
                Ok(None) => {
                    total.refused += 1;
                    tally.refused += 1;
                    continue;
                }
                Err(e) => {
                    total.error += 1;
                    tally.error += 1;
                    println!("OPEN  {what}: {e:?}");
                    continue;
                }
            };
            let space = ev.space(id).expect("just opened");
            let inner = space.template.clone();
            let got = match (fresh, space.view().cloned()) {
                (false, Some(View { reading: 0, root })) => read_tab(&mut Tab::new(&mut ev, &doc, root)),
                _ => {
                    let tab_doc = Document::new(MemSource(space.bytes().to_vec()));
                    let mut tab = Evaluator::new(space.read_as().clone());
                    read_tab(&mut Tab::new(&mut tab, &tab_doc, Vec::new()))
                }
            };
            match got {
                Ok(()) => {
                    total.ok += 1;
                    tally.ok += 1;
                }
                Err((true, e)) => {
                    total.root += 1;
                    tally.root += 1;
                    println!("ROOT  {what} (tab as {inner}): {e:?}");
                }
                Err((false, e)) => {
                    total.child += 1;
                    tally.child += 1;
                    println!("CHILD {what} (tab as {inner}): {e:?}");
                }
            }
        }
    }
    println!();
    for (name, t) in &by_template {
        println!(
            "{name:>12}: {} read, {} fail at the root, {} fail below it, {} refused, {} would not open",
            t.ok, t.root, t.child, t.refused, t.error
        );
    }
    println!(
        "total: {} read, {} fail at the root, {} fail below it, {} refused, {} would not open",
        total.ok, total.root, total.child, total.refused, total.error
    );
}

/// The root of a tab and its first few children, as a listing opening the tab
/// would read them. `true` with the error when the root itself failed.
fn read_tab<S: Source>(tab: &mut Tab<'_, S>) -> Result<(), (bool, EvalError)> {
    let root = tab.node(&[]).map_err(|e| (true, e))?;
    for i in 0..root.child_count.min(TAB_CHILDREN) {
        let child = tab.node(&[i as usize]).map_err(|e| (false, e))?;
        for j in 0..child.child_count.min(TAB_CHILDREN) {
            tab.node(&[i as usize, j as usize]).map_err(|e| (false, e))?;
        }
    }
    Ok(())
}

/// Every stream under `path` that the listing offers to open, as the path of
/// the stream and the name of the row the offer hangs on. A joined stream too
/// long to hold whole is not offered, as the listing does not offer it.
fn find_streams(
    ev: &mut Evaluator,
    doc: &Document<MemSource>,
    path: &[usize],
    budget: &mut usize,
    (cap, depth): (usize, usize),
    out: &mut Vec<(Vec<usize>, String)>,
) {
    if *budget == 0 || out.len() >= cap || path.len() > depth {
        return;
    }
    *budget -= 1;
    let Ok(node) = ev.node(doc, path) else { return };
    if node.space_root && !path.is_empty() && !(node.joined && node.size_bits > qubero_core::codec::CAP_BYTES as u64 * 8) {
        out.push((path[..path.len() - 1].to_vec(), node.name.clone()));
    }
    let count = node.child_count as usize;
    let ends: Vec<usize> = if count > FIRST_CHILDREN + LAST_CHILDREN {
        (0..FIRST_CHILDREN).chain(count - LAST_CHILDREN..count).collect()
    } else {
        (0..count).collect()
    };
    for i in ends {
        let mut child = path.to_vec();
        child.push(i);
        find_streams(ev, doc, &child, budget, (cap, depth), out);
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tools" || n == ".git" || n == "hexdump") {
                continue;
            }
            collect(&path, out);
        } else if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        } else if path.extension().is_none_or(|e| e != "md" && e != "tsv" && e != "py" && e != "dis") {
            out.push(path);
        }
    }
}
