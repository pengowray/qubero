//! Bounded OLE Compound File stream extraction (MS-CFB).
use super::{CAP_BYTES, Refusal};
use std::collections::HashSet;

const END: u32 = 0xffff_fffe;
const FREE: u32 = 0xffff_ffff;
pub const MAGIC: &[u8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";
type R<T> = Result<T, Refusal>;
fn u16_at(b: &[u8], p: usize) -> R<u16> {
    Ok(u16::from_le_bytes(
        b.get(p..p + 2).ok_or(Refusal::Failed)?.try_into().unwrap(),
    ))
}
fn u32_at(b: &[u8], p: usize) -> R<u32> {
    Ok(u32::from_le_bytes(
        b.get(p..p + 4).ok_or(Refusal::Failed)?.try_into().unwrap(),
    ))
}
fn sector(b: &[u8], id: u32, size: usize) -> R<&[u8]> {
    let start = (id as usize)
        .checked_add(1)
        .and_then(|n| n.checked_mul(size))
        .ok_or(Refusal::Failed)?;
    b.get(start..start.checked_add(size).ok_or(Refusal::Failed)?)
        .ok_or(Refusal::Failed)
}
fn words(b: &[u8]) -> Vec<u32> {
    b.chunks_exact(4)
        .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
fn chain(first: u32, fat: &[u32], limit: usize) -> R<Vec<u32>> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    let mut id = first;
    while id != END {
        if ids.len() >= limit || !seen.insert(id) {
            return Err(Refusal::Failed);
        }
        ids.push(id);
        id = *fat.get(id as usize).ok_or(Refusal::Failed)?;
    }
    Ok(ids)
}
fn regular(b: &[u8], first: u32, fat: &[u32], size: usize) -> R<Vec<u8>> {
    let ids = chain(first, fat, (b.len() / size).min(CAP_BYTES / size))?;
    let mut out = Vec::new();
    for id in ids {
        out.extend_from_slice(sector(b, id, size)?);
    }
    Ok(out)
}
fn stream_size(entry: &[u8], version: u16) -> R<usize> {
    let low = u32_at(entry, 120)? as u64;
    let high = if version == 4 {
        u32_at(entry, 124)? as u64
    } else {
        0
    };
    let n = low | high << 32;
    if n > CAP_BYTES as u64 {
        return Err(Refusal::TooLarge);
    }
    Ok(n as usize)
}
fn entry_name(e: &[u8]) -> Option<String> {
    let len = u16_at(e, 64).ok()? as usize;
    if !(2..=64).contains(&len) || len % 2 != 0 || e.get(len - 2..len)? != [0, 0] {
        return None;
    }
    String::from_utf16(
        &e[..len - 2]
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect::<Vec<_>>(),
    )
    .ok()
}

/// Conservative sniffing over the available prefix: require a directory
/// stream entry, not merely the word Workbook somewhere in another document.
pub fn is_xls(b: &[u8]) -> bool {
    if !b.starts_with(MAGIC) {
        return false;
    }
    let Ok(shift @ (9 | 12)) = u16_at(b, 30) else {
        return false;
    };
    let Ok(first) = u32_at(b, 48) else {
        return false;
    };
    let Ok(dir) = sector(b, first, 1 << shift) else {
        return false;
    };
    dir.chunks_exact(128)
        .any(|e| e[66] == 2 && matches!(entry_name(e).as_deref(), Some("Workbook" | "Book")))
}

pub fn workbook(b: &[u8]) -> R<Vec<u8>> {
    if b.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    if !b.starts_with(MAGIC) || u16_at(b, 28)? != 0xfffe || u16_at(b, 32)? != 6 {
        return Err(Refusal::Failed);
    }
    let version = u16_at(b, 26)?;
    let size = match (version, u16_at(b, 30)?) {
        (3, 9) => 512,
        (4, 12) => 4096,
        _ => return Err(Refusal::Failed),
    };
    let count = u32_at(b, 44)? as usize;
    if count > b.len() / size {
        return Err(Refusal::Failed);
    }
    let mut difat = words(b.get(76..512).ok_or(Refusal::Failed)?);
    let mut id = u32_at(b, 68)?;
    let mut seen = HashSet::new();
    let n = u32_at(b, 72)? as usize;
    if n > b.len() / size {
        return Err(Refusal::Failed);
    }
    for _ in 0..n {
        if !seen.insert(id) {
            return Err(Refusal::Failed);
        }
        let s = sector(b, id, size)?;
        difat.extend(words(&s[..size - 4]));
        id = u32_at(s, size - 4)?;
    }
    if n > 0 && id != END {
        return Err(Refusal::Failed);
    }
    let ids: Vec<_> = difat.into_iter().filter(|&v| v != FREE).collect();
    if ids.len() != count {
        return Err(Refusal::Failed);
    }
    let mut fat = Vec::new();
    seen.clear();
    for id in ids {
        if !seen.insert(id) {
            return Err(Refusal::Failed);
        }
        fat.extend(words(sector(b, id, size)?));
    }
    let directory = regular(b, u32_at(b, 48)?, &fat, size)?;
    let root = directory
        .get(..128)
        .filter(|e| e[66] == 5)
        .ok_or(Refusal::Failed)?;
    // Follow the root's sibling tree so embedded workbooks are not mistaken
    // for the document's own workbook.
    let mut pending = vec![u32_at(root, 76)?];
    let mut visited = HashSet::new();
    let mut found = None;
    while let Some(id) = pending.pop() {
        if id == FREE {
            continue;
        }
        if !visited.insert(id) {
            return Err(Refusal::Failed);
        }
        let at = (id as usize).checked_mul(128).ok_or(Refusal::Failed)?;
        let e = directory
            .get(at..at.checked_add(128).ok_or(Refusal::Failed)?)
            .ok_or(Refusal::Failed)?;
        pending.extend([u32_at(e, 68)?, u32_at(e, 72)?]);
        if e[66] == 2 && matches!(entry_name(e).as_deref(), Some("Workbook" | "Book")) {
            found = Some(e);
        }
    }
    let e = found.ok_or(Refusal::Failed)?;
    let len = stream_size(e, version)?;
    let first = u32_at(e, 116)?;
    if u32_at(b, 56)? != 4096 {
        return Err(Refusal::Failed);
    }
    let mut out = if len >= 4096 {
        regular(b, first, &fat, size)?
    } else {
        let mini_fat = regular(b, u32_at(b, 60)?, &fat, size)?;
        if mini_fat.len() / size != u32_at(b, 64)? as usize {
            return Err(Refusal::Failed);
        }
        let mut mini = regular(b, u32_at(root, 116)?, &fat, size)?;
        let mini_len = stream_size(root, version)?;
        if mini.len() < mini_len {
            return Err(Refusal::Failed);
        }
        mini.truncate(mini_len);
        let mut out = Vec::new();
        for id in chain(first, &words(&mini_fat), mini.len() / 64)? {
            let at = (id as usize).checked_mul(64).ok_or(Refusal::Failed)?;
            out.extend_from_slice(
                mini.get(at..at.checked_add(64).ok_or(Refusal::Failed)?)
                    .ok_or(Refusal::Failed)?,
            );
        }
        out
    };
    if out.len() < len {
        return Err(Refusal::Failed);
    }
    out.truncate(len);
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn put(b: &mut [u8], p: usize, n: u32) {
        b[p..p + 4].copy_from_slice(&n.to_le_bytes());
    }
    fn entry(b: &mut [u8], name: &str, kind: u8, first: u32, len: usize) {
        let text: Vec<_> = name
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        b[..text.len()].copy_from_slice(&text);
        b[64..66].copy_from_slice(&(text.len() as u16).to_le_bytes());
        b[66] = kind;
        for p in [68, 72, 76] {
            put(b, p, FREE);
        }
        put(b, 116, first);
        put(b, 120, len as u32);
    }
    /// Deliberately stores the second payload sector before the first.
    pub fn fixture(payload: &[u8], mini: bool, v4: bool) -> Vec<u8> {
        let size = if v4 { 4096 } else { 512 };
        let len = if mini {
            payload.len()
        } else {
            payload.len().max(4096)
        };
        let unit = if mini { 64 } else { size };
        let count = len.div_ceil(unit);
        let physical = if mini {
            (count * 64).div_ceil(size)
        } else {
            count
        };
        let mut b = vec![0; (4 + physical) * size];
        b[..8].copy_from_slice(MAGIC);
        b[26..28].copy_from_slice(&(if v4 { 4u16 } else { 3 }).to_le_bytes());
        b[28..30].copy_from_slice(&0xfffeu16.to_le_bytes());
        b[30..32].copy_from_slice(&(if v4 { 12u16 } else { 9 }).to_le_bytes());
        b[32..34].copy_from_slice(&6u16.to_le_bytes());
        put(&mut b, 44, 1);
        put(&mut b, 48, 1);
        put(&mut b, 56, 4096);
        put(&mut b, 60, if mini { 2 } else { END });
        put(&mut b, 64, mini as u32);
        put(&mut b, 68, END);
        b[76..512].fill(0xff);
        put(&mut b, 76, 0);
        b[size..size * 2].fill(0xff);
        put(&mut b, size, 0xffff_fffd);
        put(&mut b, size + 4, END);
        if mini {
            put(&mut b, size + 8, END);
        }
        entry(
            &mut b[2 * size..2 * size + 128],
            "Root Entry",
            5,
            if mini { 3 } else { END },
            if mini { physical * size } else { 0 },
        );
        put(&mut b, 2 * size + 76, 1);
        entry(
            &mut b[2 * size + 128..2 * size + 256],
            "Workbook",
            2,
            if mini {
                count as u32 - 1
            } else {
                3 + count as u32 - 1
            },
            len,
        );
        let mut padded = vec![0; count * unit];
        padded[..payload.len()].copy_from_slice(payload);
        if mini {
            b[3 * size..4 * size].fill(0xff);
            for j in 0..physical {
                put(
                    &mut b,
                    size + (3 + j) * 4,
                    if j + 1 == physical {
                        END
                    } else {
                        (4 + j) as u32
                    },
                );
            }
            for j in 0..count {
                let id = count - 1 - j;
                put(
                    &mut b,
                    3 * size + id * 4,
                    if id == 0 { END } else { id as u32 - 1 },
                );
                b[4 * size + id * 64..4 * size + (id + 1) * 64]
                    .copy_from_slice(&padded[j * 64..(j + 1) * 64]);
            }
        } else {
            for j in 0..count {
                let id = 3 + count - 1 - j;
                put(
                    &mut b,
                    size + id * 4,
                    if id == 3 { END } else { id as u32 - 1 },
                );
                b[(id + 1) * size..(id + 2) * size]
                    .copy_from_slice(&padded[j * size..(j + 1) * size]);
            }
        }
        b
    }
    #[test]
    fn fragmented_regular_and_mini_streams_in_both_versions() {
        for v4 in [false, true] {
            for mini in [false, true] {
                let payload: Vec<_> = (0..if mini { 193 } else { 5000 })
                    .map(|n| (n % 251) as u8)
                    .collect();
                let file = fixture(&payload, mini, v4);
                assert!(is_xls(&file));
                assert_eq!(workbook(&file).unwrap(), payload);
            }
        }
    }
    #[test]
    fn cycles_truncation_and_oversized_streams_are_refused() {
        let mut b = fixture(&vec![1; 5000], false, false);
        put(&mut b, 512 + 12 * 4, 12);
        assert_eq!(workbook(&b), Err(Refusal::Failed));
        let b = fixture(&[2; 193], true, false);
        assert!(workbook(&b[..b.len() - 200]).is_err());
        let mut b = fixture(&[2; 193], true, false);
        put(&mut b, 1024 + 128 + 120, CAP_BYTES as u32 + 1);
        assert_eq!(workbook(&b), Err(Refusal::TooLarge));
    }
}
