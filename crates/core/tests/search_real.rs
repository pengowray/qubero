//! Searching a real file, through the same chunk store the browser uses, so
//! the streaming path is the one under test rather than a buffer in memory.

use qubero_core::search::{Needle, Search, Step};
use qubero_core::source::ChunkStore;
use qubero_core::Document;

const CHUNK: u64 = 64 * 1024;

/// Every match, feeding chunks in as the search asks for them.
fn all(bytes: &[u8], s: &Search, start: u64) -> Vec<u64> {
    let mut d = Document::new(ChunkStore::new(bytes.len() as u64, CHUNK, 8));
    let mut out = Vec::new();
    let mut at = start;
    loop {
        match s.step(&d, at) {
            Step::Found { at: hit, .. } => {
                out.push(hit);
                at = if s.backward { hit } else { hit + 1 };
            }
            Step::More { resume } => at = resume,
            Step::End => return out,
            Step::Pending(missing) => {
                for m in missing {
                    let from = (m.chunk * CHUNK) as usize;
                    let to = (from + CHUNK as usize).min(bytes.len());
                    d.source_mut().insert(m.chunk, bytes[from..to].to_vec().into_boxed_slice());
                }
            }
        }
    }
}

#[test]
fn finds_a_name_in_the_editors_own_binary() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/src/pkg/qubero_wasm_bg.wasm");
    let Ok(bytes) = std::fs::read(path) else {
        eprintln!("skipped: no wasm build at {path}");
        return;
    };
    let needle = b"qubero_wasm_bg.js".to_vec();
    let forward = all(&bytes, &Search::forward(Needle::Bytes(needle.clone())), 0);
    assert!(!forward.is_empty(), "the import module name is in every build");
    eprintln!("{} matches, first at {}", forward.len(), forward[0]);

    // The same matches in the other order, which is the check that the two
    // directions agree rather than each being self-consistent.
    let mut back = all(&bytes, &Search::backward(Needle::Bytes(needle)), bytes.len() as u64);
    back.reverse();
    assert_eq!(back, forward);

    // The magic number is at the very start, which is the edge a window that
    // began one byte late would miss.
    assert_eq!(all(&bytes, &Search::forward(Needle::Bytes(b"\0asm".to_vec())), 0).first(), Some(&0));
}
