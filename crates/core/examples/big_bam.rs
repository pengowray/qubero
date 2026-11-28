//! Make a large BAM from a small one, for trying a joined stream of tens or
//! hundreds of megabytes in the browser: the small BAM's records are
//! repeated until the BAM comes to the size asked for, and written in BGZF
//! blocks of 65,280 bytes, the size samtools writes.
//!
//! `cargo run --release -p qubero-core --example big_bam -- <small.bam> <out.bam> <MiB of BAM>`
//!
//! The records of samtools' `mpileup.1.bam`, in the sample collection, pack
//! to about a third, so 300 MiB of BAM comes to about 96 MB of file, past the
//! 64 MiB a joined stream may copy.

const BLOCK: usize = 65_280;

/// The block every BGZF file ends with.
const EOF_BLOCK: [u8; 28] = [
    0x1f, 0x8b, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x06, 0x00, 0x42, 0x43, 0x02, 0x00, 0x1b, 0x00, 0x03, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Everything the BGZF blocks of `file` unpack to, joined.
fn unpacked(file: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 18 <= file.len() {
        let xlen = u16::from_le_bytes([file[at + 10], file[at + 11]]) as usize;
        let bsize = u16::from_le_bytes([file[at + 16], file[at + 17]]) as usize + 1;
        let deflate = &file[at + 12 + xlen..at + bsize - 8];
        out.extend(miniz_oxide::inflate::decompress_to_vec(deflate).expect("a BGZF block"));
        at += bsize;
    }
    out
}

fn i32_at(b: &[u8], at: usize) -> usize {
    i32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, from, to, mib] = args.as_slice() else { panic!("usage: big_bam <small.bam> <out.bam> <MiB of BAM>") };
    let target = (mib.parse::<f64>().expect("a size in MiB") * (1 << 20) as f64) as usize;
    let bam = unpacked(&std::fs::read(from).expect("reads"));
    assert!(bam.starts_with(b"BAM\x01"), "not a BAM");
    // The header text and the references, then the records.
    let mut at = 8 + i32_at(&bam, 4);
    let n_ref = i32_at(&bam, at);
    at += 4;
    for _ in 0..n_ref {
        at += 4 + i32_at(&bam, at) + 4;
    }
    let (head, records) = bam.split_at(at);
    let mut body = head.to_vec();
    while body.len() < target {
        body.extend_from_slice(records);
    }
    let mut file = Vec::with_capacity(body.len() / 2);
    for piece in body.chunks(BLOCK) {
        let deflated = miniz_oxide::deflate::compress_to_vec(piece, 6);
        let total = 12 + 6 + deflated.len() + 8;
        file.extend_from_slice(&[0x1f, 0x8b, 8, 4, 0, 0, 0, 0, 0, 0xff, 6, 0, b'B', b'C', 2, 0]);
        file.extend_from_slice(&((total - 1) as u16).to_le_bytes());
        file.extend_from_slice(&deflated);
        file.extend_from_slice(&qubero_core::checksum::crc32(piece).to_le_bytes());
        file.extend_from_slice(&(piece.len() as u32).to_le_bytes());
    }
    file.extend_from_slice(&EOF_BLOCK);
    std::fs::write(to, &file).expect("writes");
    println!("{to}: {} bytes of BAM in {} bytes of file", body.len(), file.len());
}
