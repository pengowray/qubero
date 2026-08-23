//! Turning the piece list into a plan for writing the document out.
//!
//! The host composes the output from lazy slices of the original file and the
//! add buffer, so saving a 5 GiB file with three edits copies no more than the
//! host's own I/O needs. Only stretches that are not byte-aligned (after bit-level
//! edits) have to be materialised by reading through the piece table.
//!
//! If the document length is not a whole number of bytes, the final partial byte
//! is zero-padded on output.

use crate::piece::{PieceTable, Src};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// Copy `len` bytes from the original file starting at `src_off`.
    Orig,
    /// Copy `len` bytes from the add buffer starting at `src_off`.
    Add,
    /// Read `len` bytes of the document at `doc_off` through the piece table.
    Materialize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub kind: RunKind,
    pub doc_off: u64,
    pub src_off: u64,
    pub len: u64,
}

pub fn save_plan(table: &PieceTable) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut pos: u64 = 0;
    for p in table.pieces() {
        let aligned = pos % 8 == 0 && p.bit_off % 8 == 0 && p.bit_len % 8 == 0;
        let run = if aligned {
            Run {
                kind: match p.src {
                    Src::Orig => RunKind::Orig,
                    Src::Add => RunKind::Add,
                },
                doc_off: pos / 8,
                src_off: p.bit_off / 8,
                len: p.bit_len / 8,
            }
        } else {
            let start = pos / 8;
            let end = (pos + p.bit_len).div_ceil(8);
            Run { kind: RunKind::Materialize, doc_off: start, src_off: 0, len: end - start }
        };
        push_run(&mut runs, run);
        pos += p.bit_len;
    }
    runs.retain(|r| r.len > 0);
    runs
}

/// Append a run, resolving overlap with the previous run. Overlap only happens
/// around materialised stretches, which always win.
fn push_run(runs: &mut Vec<Run>, mut run: Run) {
    while let Some(last) = runs.last_mut() {
        let last_end = last.doc_off + last.len;
        if run.doc_off >= last_end {
            break;
        }
        match (last.kind, run.kind) {
            (RunKind::Materialize, RunKind::Materialize) => {
                let end = (run.doc_off + run.len).max(last_end);
                last.len = end - last.doc_off;
                return;
            }
            (RunKind::Materialize, _) => {
                // Trim the front of the new direct run.
                let cut = last_end - run.doc_off;
                if cut >= run.len {
                    return;
                }
                run.doc_off += cut;
                run.src_off += cut;
                run.len -= cut;
                break;
            }
            (_, RunKind::Materialize) => {
                // Trim the tail of the previous direct run; drop it if consumed.
                if run.doc_off <= last.doc_off {
                    runs.pop();
                    continue;
                }
                last.len = run.doc_off - last.doc_off;
                break;
            }
            _ => unreachable!("direct runs never overlap"),
        }
    }
    runs.push(run);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::source::{MemSource, Source};

    fn apply(d: &Document<MemSource>, plan: &[Run]) -> Vec<u8> {
        let mut out = Vec::new();
        for r in plan {
            assert_eq!(r.doc_off, out.len() as u64, "runs must be contiguous");
            match r.kind {
                RunKind::Orig => out.extend_from_slice(&d.source().0[r.src_off as usize..][..r.len as usize]),
                RunKind::Add => out.extend_from_slice(&d.add_bytes()[r.src_off as usize..][..r.len as usize]),
                RunKind::Materialize => {
                    let mut buf = vec![0; r.len as usize];
                    d.read_bytes(r.doc_off, &mut buf);
                    out.extend_from_slice(&buf);
                }
            }
        }
        out
    }

    #[test]
    fn byte_edits_produce_direct_runs_only() {
        let mut d = Document::new(MemSource((0..100u8).collect()));
        d.overwrite_bytes(10, b"XYZ");
        d.delete_bytes(50, 5);
        d.insert_bytes(80, b"hi");
        let plan = d.save_plan();
        assert!(plan.iter().all(|r| r.kind != RunKind::Materialize));
        let mut expect = vec![0; d.len_bytes() as usize];
        d.read_bytes(0, &mut expect);
        assert_eq!(apply(&d, &plan), expect);
    }

    #[test]
    fn bit_edits_round_trip() {
        let orig: Vec<u8> = (0..200u8).map(|i| i.wrapping_mul(91)).collect();
        let mut d = Document::new(MemSource(orig));
        let mut seed = 4242u64;
        let mut rnd = |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % m
        };
        for _ in 0..60 {
            let len = d.len_bits();
            let n = rnd(20) + 1;
            match rnd(3) {
                0 => d.insert_bits(rnd(len + 1), &[rnd(256) as u8, rnd(256) as u8, 0], n),
                1 if len > n => d.delete_bits(rnd(len - n), n),
                _ => d.overwrite_bytes(rnd(d.len_bytes()), &[rnd(256) as u8]),
            }
            let plan = d.save_plan();
            let mut expect = vec![0; d.len_bytes() as usize];
            d.read_bytes(0, &mut expect);
            assert_eq!(apply(&d, &plan), expect);
            assert_eq!(plan.iter().map(|r| r.len).sum::<u64>(), d.len_bytes());
        }
        assert!(d.source().len_bytes() > 0);
    }
}
