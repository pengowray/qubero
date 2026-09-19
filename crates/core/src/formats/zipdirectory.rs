//! A ZIP archive's central directory, read from the end of the file.
//!
//! What every reader here wants out of an archive is the same thing: the name
//! of each entry and the run of bytes its data is. That is not what
//! `formats/zip.rs` does. The template lays the archive out as the records it
//! is made of, front to back, for someone looking at the file; this answers a
//! question about it, and the answer comes from the directory at the end
//! because that is the only place an entry's name and its offset are written
//! together.
//!
//! Reads go through a closure rather than a slice, so the caller decides where
//! the bytes come from: the evaluator hands one that reads through the
//! document and may answer `Pending`, which is what lets a checkpoint open
//! before the whole of it has arrived. [`torchlegacy::layout`] is written the
//! same way.
//!
//! **Where the numbers really are.** The end record's own field says where the
//! directory begins, and that is what is read. Subtracting the directory's
//! length from the end record is not the same thing: a writer may put the
//! ZIP64 records in between even when every number fits without them, which is
//! what torch 1.5 did, and subtracting then starts the walk inside the last
//! entry. That bug was in this walk's second copy and is the reason there is
//! one of these.

use crate::eval::R;

/// The signatures the records are found by.
const END: &[u8] = b"PK\x05\x06";
const END64: &[u8] = b"PK\x06\x06";
const LOCATOR: &[u8] = b"PK\x06\x07";
const CENTRAL: &[u8] = b"PK\x01\x02";
const LOCAL: &[u8] = b"PK\x03\x04";
/// How long the fixed part of each of them is.
const END_RECORD: u64 = 22;
const CENTRAL_RECORD: u64 = 46;
const LOCAL_HEADER: u64 = 30;
const LOCATOR_RECORD: u64 = 20;
/// The ZIP64 end record, as far as the three numbers this reads.
const END64_RECORD: u64 = 56;
/// How far back from the end the end record may be: its own bytes, the
/// locator that may sit in front of it, and the longest comment a ZIP carries.
const MOST_COMMENT: u64 = (1 << 16) + END_RECORD + LOCATOR_RECORD;
/// The widest a 32-bit field may be before it is standing in for a 64-bit one
/// kept in the entry's extra field.
const WIDE32: u64 = 0xffff_ffff;
const WIDE16: u64 = 0xffff;
/// The extra field that holds those 64-bit numbers.
const ZIP64_EXTRA: u64 = 1;
/// The most entries the walk will follow. Far past any checkpoint: a storage
/// is one entry and a model of a few hundred million parameters has a few
/// hundred of them.
const MOST_ENTRIES: usize = 1 << 20;

/// One entry of an archive: what it is called, and the run its data is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub at: u64,
    pub len: u64,
    /// How the data was packed. Only a stored entry can be read where it lies.
    pub method: u16,
}

/// Every entry the central directory names, or nothing at all when this is not
/// an archive or the directory does not read.
///
/// `end` is how long the whole thing is: the file, or the decoded stream when
/// the archive is inside one.
pub fn entries(read: &mut dyn FnMut(u64, u64) -> R<Vec<u8>>, end: u64) -> R<Vec<Entry>> {
    let none = Vec::new();
    let look = end.min(MOST_COMMENT);
    if look < END_RECORD {
        return Ok(none);
    }
    let tail = read(end - look, look)?;
    // The last end record, since a comment may hold the same four bytes.
    let Some(found) = (0..=tail.len().saturating_sub(END_RECORD as usize)).rev().find(|i| tail[*i..].starts_with(END)) else {
        return Ok(none);
    };
    let at = |i: usize| -> u64 { u32::from_le_bytes(tail[found + i..found + i + 4].try_into().unwrap_or([0; 4])) as u64 };
    let short = |i: usize| -> u64 { u16::from_le_bytes(tail[found + i..found + i + 2].try_into().unwrap_or([0; 2])) as u64 };
    let (mut count, mut size, mut start) = (short(10), at(12), at(16));
    // Past four gigabytes the numbers do not fit, and the real ones are in a
    // record of its own that a locator in front of the end record points at.
    // Every checkpoint of a large model is one of these.
    if count == WIDE16 || size == WIDE32 || start == WIDE32 {
        let Some(held) = zip64_end(read, &tail, found)? else { return Ok(none) };
        (count, size, start) = held;
    }
    if size == 0 || start.checked_add(size).is_none_or(|to| to > end) {
        return Ok(none);
    }
    let held = read(start, size)?;
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while out.len() < MOST_ENTRIES && (out.len() as u64) < count.max(1) {
        let Some(record) = held.get(cursor..cursor + CENTRAL_RECORD as usize) else { break };
        if record[..4] != *CENTRAL {
            break;
        }
        let short = |i: usize| u16::from_le_bytes([record[i], record[i + 1]]) as u64;
        let long = |i: usize| u32::from_le_bytes([record[i], record[i + 1], record[i + 2], record[i + 3]]) as u64;
        let (method, mut packed) = (short(10), long(20));
        let (name_len, extra_len, comment_len) = (short(28), short(30), short(32));
        let mut local = long(42);
        let names_at = cursor + CENTRAL_RECORD as usize;
        let Some(name) = held.get(names_at..names_at + name_len as usize) else { break };
        let name = String::from_utf8_lossy(name).replace('\\', "/");
        let extra = held.get(names_at + name_len as usize..names_at + name_len as usize + extra_len as usize).unwrap_or(&[]);
        // The 64-bit sizes, in the order ZIP64 writes them and only for the
        // fields whose 32-bit place holds the mark saying so.
        if packed == WIDE32 || local == WIDE32 {
            let wide = zip64_extra(extra);
            let mut next = wide.iter().copied();
            if long(24) == WIDE32 {
                next.next();
            }
            if packed == WIDE32 {
                packed = next.next().unwrap_or(packed);
            }
            if local == WIDE32 {
                local = next.next().unwrap_or(local);
            }
        }
        cursor = names_at + (name_len + extra_len + comment_len) as usize;
        // Where the data begins, which only the local header says: the extra
        // field there is padded for alignment and is not the one the directory
        // carries.
        let Ok(head) = read(local, LOCAL_HEADER) else { continue };
        if head.len() < LOCAL_HEADER as usize || head[..4] != *LOCAL {
            continue;
        }
        let here = |i: usize| u16::from_le_bytes([head[i], head[i + 1]]) as u64;
        let data_at = local + LOCAL_HEADER + here(26) + here(28);
        if data_at.checked_add(packed).is_none_or(|to| to > end) {
            continue;
        }
        out.push(Entry { name, at: data_at, len: packed, method: method as u16 });
    }
    Ok(out)
}

/// The counts and the place of the directory as ZIP64 writes them, found
/// through the locator that sits in front of the end record.
fn zip64_end(read: &mut dyn FnMut(u64, u64) -> R<Vec<u8>>, tail: &[u8], found: usize) -> R<Option<(u64, u64, u64)>> {
    let Some(at) = found.checked_sub(LOCATOR_RECORD as usize) else { return Ok(None) };
    let Some(locator) = tail.get(at..at + LOCATOR_RECORD as usize) else { return Ok(None) };
    if locator[..4] != *LOCATOR {
        return Ok(None);
    }
    let where_at = u64::from_le_bytes(locator[8..16].try_into().unwrap_or([0; 8]));
    let record = read(where_at, END64_RECORD)?;
    if record.len() < END64_RECORD as usize || record[..4] != *END64 {
        return Ok(None);
    }
    let long = |i: usize| u64::from_le_bytes(record[i..i + 8].try_into().unwrap_or([0; 8]));
    Ok(Some((long(32), long(40), long(48))))
}

/// The 64-bit numbers an entry's ZIP64 extra field holds, in the order it
/// writes them: the unpacked size, the packed size, where the local header is,
/// and which disk it is on. Only the ones whose 32-bit place held the mark are
/// written, so the caller takes them in turn.
fn zip64_extra(extra: &[u8]) -> Vec<u64> {
    let mut at = 0usize;
    while let Some(head) = extra.get(at..at + 4) {
        let id = u16::from_le_bytes([head[0], head[1]]) as u64;
        let len = u16::from_le_bytes([head[2], head[3]]) as usize;
        let Some(body) = extra.get(at + 4..at + 4 + len) else { return Vec::new() };
        if id == ZIP64_EXTRA {
            return body.chunks_exact(8).map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8]))).collect();
        }
        at += 4 + len;
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader over bytes that are all here, which is what a test has.
    fn over(bytes: &[u8]) -> impl FnMut(u64, u64) -> R<Vec<u8>> + '_ {
        move |at: u64, len: u64| {
            let from = at as usize;
            let to = from.saturating_add(len as usize).min(bytes.len());
            Ok(bytes.get(from.min(bytes.len())..to).unwrap_or(&[]).to_vec())
        }
    }

    fn found(bytes: &[u8]) -> Vec<Entry> {
        entries(&mut over(bytes), bytes.len() as u64).unwrap()
    }

    /// One stored entry, its local header and its data, from an offset the
    /// caller chooses so that a directory can name it.
    fn local(name: &str, data: &[u8]) -> Vec<u8> {
        let mut out = LOCAL.to_vec();
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        out
    }

    /// The directory record naming an entry whose local header is at `at`.
    fn central(name: &str, len: usize, at: u32) -> Vec<u8> {
        let mut out = CENTRAL.to_vec();
        out.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&(len as u32).to_le_bytes());
        out.extend_from_slice(&(len as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        // The extra and comment lengths, the disk number, and the two
        // attribute words, which this reading has no use for.
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out
    }

    fn end(count: u16, size: u32, start: u32, comment: &[u8]) -> Vec<u8> {
        let mut out = END.to_vec();
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        out.extend_from_slice(comment);
        out
    }

    /// The ZIP64 end record and the locator that finds it, as torch 1.5 writes
    /// them: between the directory and the ordinary end record.
    fn end64(count: u64, size: u64, start: u64, at: u64) -> Vec<u8> {
        let mut out = END64.to_vec();
        out.extend_from_slice(&(END64_RECORD - 12).to_le_bytes());
        out.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(LOCATOR);
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out
    }

    /// Two entries, read as the names and the runs their data is.
    #[test]
    fn the_directory_names_every_entry_and_says_where_its_data_is() {
        let first = local("archive/data.pkl", b"a pickle");
        let second = local("archive/data/0", b"numbers!");
        let mut bytes = first.clone();
        let second_at = bytes.len() as u32;
        bytes.extend_from_slice(&second);
        let directory_at = bytes.len() as u32;
        let mut directory = central("archive/data.pkl", 8, 0);
        directory.extend_from_slice(&central("archive/data/0", 8, second_at));
        bytes.extend_from_slice(&directory);
        bytes.extend_from_slice(&end(2, directory.len() as u32, directory_at, b""));
        let held = found(&bytes);
        assert_eq!(held.len(), 2);
        assert_eq!(held[0], Entry { name: "archive/data.pkl".into(), at: 30 + 16, len: 8, method: 0 });
        assert_eq!(&bytes[held[1].at as usize..(held[1].at + held[1].len) as usize], b"numbers!");
    }

    /// A comment holding the end record's own four bytes: the last one is the
    /// end record, and the reading is what it was without the comment.
    #[test]
    fn four_bytes_in_a_comment_are_not_the_end_record() {
        let entry = local("one", b"xy");
        let mut bytes = entry.clone();
        let directory_at = bytes.len() as u32;
        let directory = central("one", 2, 0);
        bytes.extend_from_slice(&directory);
        bytes.extend_from_slice(&end(1, directory.len() as u32, directory_at, b"PK\x05\x06 in a comment"));
        assert_eq!(found(&bytes).len(), 1);
    }

    /// torch 1.5 writes the ZIP64 records between the directory and the end
    /// record although every number fits without them. The end record's own
    /// field says where the directory is, so the walk still starts there;
    /// subtracting the directory's length from the end record would start it
    /// inside the last entry, which is the bug this reading was written for.
    #[test]
    fn the_end_record_says_where_the_directory_is_rather_than_how_far_back() {
        let entry = local("one", b"xy");
        let mut bytes = entry.clone();
        let directory_at = bytes.len() as u64;
        let directory = central("one", 2, 0);
        bytes.extend_from_slice(&directory);
        let end64_at = bytes.len() as u64;
        bytes.extend_from_slice(&end64(1, directory.len() as u64, directory_at, end64_at));
        bytes.extend_from_slice(&end(1, directory.len() as u32, directory_at as u32, b""));
        let held = found(&bytes);
        assert_eq!(held.len(), 1, "{held:?}");
        assert_eq!(held[0].name, "one");
        // And the same archive with the marks that say the real numbers are in
        // the ZIP64 record reads them out of it.
        let mut wide = bytes.clone();
        let at = wide.len() - END_RECORD as usize;
        wide[at + 10..at + 12].copy_from_slice(&WIDE16.to_le_bytes()[..2]);
        wide[at + 16..at + 20].copy_from_slice(&(WIDE32 as u32).to_le_bytes());
        assert_eq!(found(&wide), held);
    }

    /// Nothing at all for bytes that are not an archive, and for an end record
    /// naming a directory that is not in the file.
    #[test]
    fn what_is_not_an_archive_reads_as_no_entries() {
        assert!(found(b"").is_empty());
        assert!(found(b"not a zip, not nearly long enough to hold an end record").is_empty());
        let mut bytes = local("one", b"xy");
        bytes.extend_from_slice(&end(1, 46, 0xffff, b""));
        assert!(found(&bytes).is_empty());
    }

    /// The extra field is read by its own identifier, and a field that is not
    /// the ZIP64 one is walked past rather than read as it.
    #[test]
    fn the_wide_numbers_come_from_the_field_that_says_it_holds_them() {
        let mut extra = vec![0x99, 0x99, 8, 0];
        extra.extend_from_slice(&7u64.to_le_bytes());
        assert!(zip64_extra(&extra).is_empty());
        extra.extend_from_slice(&[1, 0, 16, 0]);
        extra.extend_from_slice(&11u64.to_le_bytes());
        extra.extend_from_slice(&22u64.to_le_bytes());
        assert_eq!(zip64_extra(&extra), vec![11, 22]);
        // A length running past what the field holds is read as nothing.
        assert!(zip64_extra(&[1, 0, 16, 0, 1, 2, 3]).is_empty());
    }
}
