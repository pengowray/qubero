// A rule written `0 string PAR\0` is about four bytes, the last of them NUL.
// The engine used to drop that NUL from the end of a rule's string, so the
// rule matched anything starting `PAR`, a Parquet file included. The fix is
// in crates/vendor/pure-magic; this is what keeps it.

/// A head padded with bytes that are plainly not text. Padding with zeros
/// would have the engine read the whole thing as ASCII text and skip every
/// rule whose string holds a NUL, which is a different quirk from the one
/// being tested here.
fn padded(start: &[u8]) -> Vec<u8> {
    let mut head = start.to_vec();
    head.extend((head.len()..4096).map(|i| (i * 97 + 13) as u8));
    head
}

fn message(head: &[u8]) -> String {
    let json = qubero_magic::identify(head);
    json.split_once("\"message\":\"")
        .and_then(|(_, rest)| rest.split_once("\",\"mime\""))
        .map(|(m, _)| m.to_string())
        .unwrap_or_default()
}

#[test]
fn a_parquet_file_is_not_a_parity_archive() {
    let head = padded(b"PAR1\x15\x04\x15\x40\x15\x40\x4c\x15\x10\x15\x04\x00");
    assert_eq!(message(&head), "Apache Parquet file");
}

#[test]
fn a_parity_archive_still_is_one() {
    let head = padded(b"PAR\0");
    assert!(message(&head).starts_with("PARity archive data"), "{}", message(&head));
}
