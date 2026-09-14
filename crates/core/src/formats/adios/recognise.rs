//! Which ADIOS2 template a file is, from its first bytes and its length.

// ---------------------------------------------------------------------------
// Recognition
//
// Which ADIOS2 template a file is, from its first bytes and its length, is
// asked twice, since the two kinds of evidence belong to different bands of
// `PROBES`. What each file can be told by:
//
// - Every BP4 file and a BP5 `md.idx` open with `ADIOS-BP v`, a release, and a
//   word saying which file it is, with the BP version at byte 37. That is a
//   signature and it is sound.
// - A BP5 `md.0` has no header. A step written by one writer is a total and
//   two sizes that add up to it with their sixteen bytes, followed by an FFS
//   format ID; that is claimed. A step of several writers is not.
// - A BP5 `mmd.0` has no header either. It opens with an ID length of twelve, a
//   description length that fits the file, an FFS ID of version 2 or 3, and a
//   description whose length, in its own first two bytes, is four times the
//   length the ID gives. Four fields agreeing is claimed.
// - A BP5 `data.N` is bytes the metadata points at, and nothing marks it.
// - A BP3 file keeps its version at the end, which a sniff only sees when the
//   whole file is in the window. Its front is a process group, or for a file
//   whose data is in subfiles a process group index, and either names its step
//   twice, as digits and as a number; that agreeing, with the rest of the
//   header in bounds, is claimed.

/// Both questions, in the order `PROBES` asks them.
#[cfg(test)]
pub(super) fn sniff(head: &[u8], len: u64) -> Option<&'static str> {
    sniff_signed(head, len).or_else(|| sniff_agreeing(head, len))
}

/// The files that sign themselves: every BP4 file and a BP5 index.
pub fn sniff_signed(head: &[u8], _len: u64) -> Option<&'static str> {
    headed(head)
}

/// The files recognised by fields that agree with each other rather than by a
/// signature: a BP5 `md.0` or `mmd.0`, and a BP3 file.
pub fn sniff_agreeing(head: &[u8], len: u64) -> Option<&'static str> {
    if is_bp5_metametadata(head, len) {
        return Some("adiosbp5mmd");
    }
    if is_bp5_metadata(head, len) {
        return Some("adiosbp5md");
    }
    if is_bp3(head, len) {
        return Some("adiosbp3");
    }
    None
}


fn headed(head: &[u8]) -> Option<&'static str> {
    if !head.starts_with(b"ADIOS-BP v") || head.len() < 64 || head[36] > 1 {
        return None;
    }
    let tag = &head[10..32];
    let word = tag.iter().position(|&b| b == b' ').map(|i| &tag[i + 1..])?;
    let is = |w: &[u8]| word.starts_with(w) && word[w.len()..].iter().all(|&b| b == 0);
    match head[37] {
        4 if is(b"Index Table") => Some("adiosbp4idx"),
        4 if is(b"Metadata") => Some("adiosbp4md"),
        4 if is(b"Data") => Some("adiosbp4data"),
        5 if is(b"Index Table") => Some("adiosbp5idx"),
        _ => None,
    }
}

fn u64_le(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
}

fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}

fn u16_le(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes(s.try_into().unwrap()))
}

fn u16_be(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_be_bytes(s.try_into().unwrap()))
}

fn is_bp5_metametadata(head: &[u8], len: u64) -> bool {
    let (Some(12), Some(info)) = (u64_le(head, 0), u64_le(head, 8)) else { return false };
    let (Some(&version), Some(&top), Some(rep)) = (head.get(16), head.get(17), u16_be(head, 18)) else { return false };
    let (Some(length), Some(&rep_version)) = (u16_be(head, 28), head.get(31)) else { return false };
    let ideal = (top as u64) << 18 | (rep as u64) << 2;
    let written = length as u64 | (u16_be(head, 34).unwrap_or(0) as u64) << 16;
    (version == 2 || version == 3)
        && rep != 0
        && rep_version > 0
        && info.checked_add(28).is_some_and(|n| n <= len)
        && written >= 8
        && written <= info
        && written >> 2 == ideal >> 2
}

fn is_bp5_metadata(head: &[u8], len: u64) -> bool {
    let (Some(total), Some(meta), Some(attr)) = (u64_le(head, 0), u64_le(head, 8), u64_le(head, 16)) else { return false };
    let Some(&version) = head.get(24) else { return false };
    let adds_up = meta.checked_add(attr).and_then(|n| n.checked_add(16)) == Some(total);
    adds_up && total.checked_add(8).is_some_and(|n| n <= len) && meta >= 24 && (version == 2 || version == 3) && u16_be(head, 26).is_some_and(|r| r != 0)
}

fn is_bp3(head: &[u8], len: u64) -> bool {
    bp3_footer(head, len) || bp3_group_front(head, len) || bp3_index_front(head, len)
}

/// The footer of a BP3 file the sniff can see all of, little-endian or not.
fn bp3_footer(head: &[u8], len: u64) -> bool {
    if head.len() as u64 != len || head.len() < 56 {
        return false;
    }
    let f = &head[head.len() - 56..];
    if !f.starts_with(b"ADIOS-BP v") || f[55] != 3 || f[52] > 1 {
        return false;
    }
    let read = |at: usize| {
        let b: [u8; 8] = f[at..at + 8].try_into().unwrap();
        if f[52] == 1 { u64::from_be_bytes(b) } else { u64::from_le_bytes(b) }
    };
    let (pg, vars, attrs) = (read(28), read(36), read(44));
    pg <= vars && vars <= attrs && attrs <= len - 56
}

/// A step name, which is the step written as digits, and the step after it.
fn step_named_twice(head: &[u8], at: usize) -> Option<usize> {
    let n = u16_le(head, at)? as usize;
    let digits = head.get(at + 2..at + 2 + n)?;
    if n == 0 || n > 10 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let text: u64 = std::str::from_utf8(digits).ok()?.parse().ok()?;
    (u32_le(head, at + 2 + n)? as u64 == text).then_some(at + 2 + n + 4)
}

/// A name of printable characters, and where it ends.
fn printable_name(head: &[u8], at: usize) -> Option<usize> {
    let n = u16_le(head, at)? as usize;
    let name = head.get(at + 2..at + 2 + n)?;
    name.iter().all(|b| (0x20..0x7f).contains(b)).then_some(at + 2 + n)
}

/// A BP3 file that starts with its first process group, little-endian.
fn bp3_group_front(head: &[u8], len: u64) -> bool {
    let Some(length) = u64_le(head, 0) else { return false };
    if length.checked_add(8).is_none_or(|n| n > len) || !matches!(head.get(8), Some(b'y' | b'n')) {
        return false;
    }
    let Some(after_name) = printable_name(head, 9) else { return false };
    let Some(after_step) = step_named_twice(head, after_name + 4) else { return false };
    let (Some(&count), Some(methods)) = (head.get(after_step), u16_le(head, after_step + 1)) else { return false };
    methods as usize == 3 * count as usize
}

/// A BP3 file of indices alone, which starts with its process group index.
fn bp3_index_front(head: &[u8], len: u64) -> bool {
    let (Some(count), Some(length)) = (u64_le(head, 0), u64_le(head, 8)) else { return false };
    if count == 0 || length.checked_add(16).is_none_or(|n| n > len) {
        return false;
    }
    let Some(entry) = u16_le(head, 16) else { return false };
    let Some(after_name) = printable_name(head, 18) else { return false };
    if !matches!(head.get(after_name), Some(b'y' | b'n')) {
        return false;
    }
    let Some(after_step) = step_named_twice(head, after_name + 5) else { return false };
    after_step + 8 == 18 + entry as usize
}
