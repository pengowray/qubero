//! Framing: where CPython puts a frame boundary, what it writes between two
//! frames rather than inside one, and the boundaries a form must not take.

use super::*;

/// Frames, as CPython writes them: a run of them, with a payload of
/// [`BIG_PAYLOAD`] bytes or more written between two rather than inside
/// one.
#[test]
fn a_payload_too_large_to_frame_sits_between_frames() {
    let head = cat(&[b"}\x94(", &word("a"), b"K\x01", &word("big")]);
    let tail = cat(&[b"\x94", &word("c"), b"K\x02u."]);
    let carrying = |size: usize| {
        let mut out = vec![b'B'];
        out.extend_from_slice(&(size as u32).to_le_bytes());
        out.resize(out.len() + size, 0xa5);
        out
    };
    let build = |payload: &[u8]| {
        let mut out = vec![0x80, 4, 0x95];
        out.extend_from_slice(&(head.len() as u64).to_le_bytes());
        out.extend_from_slice(&head);
        out.extend_from_slice(payload);
        out.push(0x95);
        out.extend_from_slice(&(tail.len() as u64).to_le_bytes());
        out.extend_from_slice(&tail);
        out
    };
    let whole = build(&carrying(BIG_PAYLOAD));
    let found = recognise(&whole).unwrap();
    assert_eq!(found.form, "basic-p4-p5-v4");
    let Kind::Dict(entries) = &found.value.kind else { panic!("dict") };
    assert_eq!(entries.len(), 3);
    assert!(matches!(entries[1].1.kind, Kind::Bytes { len, .. } if len == BIG_PAYLOAD));

    // The first frame one byte short of the payload's opcode, so the
    // opcode would begin inside a frame rather than after one.
    let mut early = whole.clone();
    early[3] -= 1;
    assert!(recognise(&early).is_none(), "a frame that ends before the payload");
    // The whole body inside one frame, payload and all.
    let inside = cat(&[&head, &carrying(BIG_PAYLOAD), &tail]);
    let mut once = vec![0x80, 4, 0x95];
    once.extend_from_slice(&(inside.len() as u64).to_le_bytes());
    once.extend_from_slice(&inside);
    assert!(recognise(&once).is_none(), "a large payload inside a frame");
    // A payload one byte short of large, written between frames anyway.
    assert!(recognise(&build(&carrying(BIG_PAYLOAD - 1))).is_none(), "a small payload between frames");
    // No new frame after the payload.
    let mut headless = vec![0x80, 4, 0x95];
    headless.extend_from_slice(&(head.len() as u64).to_le_bytes());
    headless.extend_from_slice(&head);
    headless.extend_from_slice(&carrying(BIG_PAYLOAD));
    headless.extend_from_slice(&tail);
    assert!(recognise(&headless).is_none(), "no frame after the payload");
    // Truncation, since a frame's length is a claim about bytes that may
    // not have arrived.
    for end in [0, 3, 11, head.len() + 11, whole.len() - 1] {
        assert!(recognise(&whole[..end]).is_none(), "accepted {end} bytes");
    }

    // A frame that ends early with no large payload behind it. The short
    // unframed tail is only ever what CPython leaves after one of those,
    // so an empty frame followed by the whole object is a non-match, and
    // so is a frame that stops one value before the STOP.
    let mut empty = vec![0x80, 4, 0x95];
    empty.extend_from_slice(&0u64.to_le_bytes());
    empty.extend_from_slice(b"N.");
    assert!(recognise(&empty).is_none(), "a frame holding none of the object");
    let mut early_stop = vec![0x80, 4, 0x95];
    let body = cat(&[b"]\x94(K\x01K\x02e."]);
    early_stop.extend_from_slice(&((body.len() - 1) as u64).to_le_bytes());
    early_stop.extend_from_slice(&body);
    assert!(recognise(&early_stop).is_none(), "a frame that ends before the STOP");
}

/// A frame ends in front of an object and nowhere else, and the fixed run
/// that rebuilds a NumPy array is full of objects.
///
/// CPython's framer commits a frame at the start of every `save`, so in
/// any file over 64 KiB the boundary lands wherever the writer happened
/// to be: in front of the dtype, in front of one of the eight values its
/// state holds, in front of a dimension, in front of the numbers. A form
/// that only looked for a boundary between one value of the file and the
/// next read none of those files at all.
#[test]
fn a_frame_may_end_in_front_of_anything_a_call_was_given() {
    let body = two_arrays(&get(17));
    let whole = framed(&body);
    assert!(recognise(&whole).is_some());
    // Each of these is a place inside the run of instructions that
    // rebuilds an array, named by the bytes it begins with.
    for (what, opens) in [
        ("the reconstructor's module", &b"\x8c\x16numpy"[..]),
        ("the placeholder shape", b"K\0\x85\x94"),
        ("the placeholder byte string", b"C\x01b\x94"),
        ("the state tuple", b"(K\x01"),
        ("the first dimension", b"K\x01K\x02\x85\x94"),
        ("the dtype class's module", b"h\x05\x8c\x05dtype"),
        ("the dtype's letters", b"\x8c\x02i1\x94"),
        ("the flags the dtype is built with", b"\x89\x88\x87\x94R\x94"),
        ("the byte order", b"\x8c\x01|\x94NNN"),
        ("the first of the state's Nones", b"NNNJ"),
        ("the alignment", b"K\0t\x94b\x89C\x02"),
        ("the storage order flag", b"\x89C\x02\x01\x02\x94"),
        ("the numbers", b"C\x02\x01\x02\x94"),
        ("the second array's dtype", b"h\x11\x89"),
    ] {
        let at = body.windows(opens.len()).position(|w| w == opens).unwrap_or_else(|| panic!("no {what}"));
        assert!(recognise(&in_two_frames(&body, at)).is_some(), "a frame ending before {what}");
    }
    // And nowhere else: a frame that ends in the middle of an object is
    // not something the framer ever writes.
    let at = body.windows(4).position(|w| w == b"\x8c\x02i1").unwrap() + 2;
    assert!(recognise(&in_two_frames(&body, at)).is_none(), "a frame ending inside a word");
}
