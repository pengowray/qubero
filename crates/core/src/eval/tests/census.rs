use super::*;

// ----- what the file's bytes went on -----

/// Run the whole-file walk to the end, and say how many goes it took. Each go
/// gets a fresh allowance, which is what the host does between frames.
fn kinds_to_the_end(ev: &mut Evaluator, d: &Document<MemSource>) -> (KindTotals, usize) {
    let mut walk = KindWalk::new(d.len_bits());
    for goes in 1..100_000 {
        ev.begin_slice();
        let out = ev.kind_totals_step(d, &mut walk).expect("nothing to fetch, nothing to fail");
        if out.done {
            return (out, goes);
        }
    }
    panic!("the walk never finished");
}

/// What one entry of the answer says, for an assertion that does not care what
/// order the entries came in.
fn spent(totals: &KindTotals, kind: &str, type_name: &str) -> (u64, u64) {
    totals
        .totals
        .iter()
        .find(|t| t.kind == kind && t.type_name == type_name)
        .map_or((0, 0), |t| (t.bits, t.count))
}

/// Every bit of a file is either on some field or on nothing, and the two add
/// up to the file. A structure that does not fill the file leaves the rest
/// unmapped, which is what a reader asking where a format spends its bytes
/// most wants told.
#[test]
fn kind_totals_account_for_every_bit_of_a_small_file() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![("magic", T::magic(b"QB")), ("count", T::u16(Big)), ("codes", T::array(T::u8(), E::lit(4)))],
        ),
    );
    let d = doc(&[b'Q', b'B', 0, 4, 1, 2, 3, 4, 0xff, 0xff]);
    let mut ev = Evaluator::new(t);
    let (out, _) = kinds_to_the_end(&mut ev, &d);

    assert_eq!(spent(&out, "magic", "magic[2]"), (16, 1));
    assert_eq!(spent(&out, "uint", "u16 be"), (16, 1));
    // Four bytes as four fields, from one element's walk: see the run below.
    assert_eq!(spent(&out, "uint", "u8"), (32, 4));
    assert_eq!(out.covered_bits, 8 * 8);
    // The two bytes past the structure are described by nothing.
    assert_eq!(out.unmapped_bits, 2 * 8);
    assert_eq!(out.reached_bits, d.len_bits());
    assert_eq!(out.covered_bits + out.unmapped_bits, d.len_bits());
    // The structure's own bits are its children's, and counting them again
    // would say the file is twice the size it is.
    assert_eq!(spent(&out, "composite", "Root"), (0, 0));
}

/// A run of same-sized elements is counted by arithmetic, not by walking it.
/// Two hundred thousand records walked one at a time is two hundred thousand
/// goes of a bounded allowance and as many nodes; walked once and multiplied
/// it is one go, which is what makes the question answerable for a file whose
/// weights are the whole of it.
#[test]
fn a_fixed_size_run_is_counted_without_walking_it() {
    const N: usize = 200_000;
    let t = Template::new(
        "t",
        T::array(T::structure("Pair", vec![("lo", T::u16(Big)), ("hi", T::u16(Big))]), E::lit(N as i128)),
    );
    let d = doc(&vec![7u8; N * 4]);
    let mut ev = Evaluator::new(t);
    // Small enough that walking the run would run out hundreds of times over.
    ev.set_slice(Some(1_000));
    let (out, goes) = kinds_to_the_end(&mut ev, &d);

    assert_eq!(goes, 1, "a run counted by arithmetic takes one go");
    assert_eq!(spent(&out, "uint", "u16 be"), (N as u64 * 2 * 16, N as u64 * 2));
    assert_eq!(out.covered_bits, d.len_bits());
    assert_eq!(out.unmapped_bits, 0);
    // One element's worth of nodes, not two hundred thousand.
    assert!(ev.memo_len() < 32, "{} nodes left behind", ev.memo_len());
}

/// A run whose elements are each as long as their own bytes say has to be
/// walked, and a walk that long does not fit in one go. It has to come back
/// where it left off, count every element once, and not remember them.
#[test]
fn a_variable_length_run_resumes_and_is_not_remembered() {
    const N: usize = 5_000;
    let t = Template::new("t", T::array(T::cstr(), E::lit(N as i128)));
    let mut bytes = Vec::new();
    for _ in 0..N {
        bytes.extend_from_slice(b"ab\0");
    }
    let d = doc(&bytes);
    let mut ev = Evaluator::new(t);
    ev.set_slice(Some(200));
    let (out, goes) = kinds_to_the_end(&mut ev, &d);

    assert!(goes > 1, "five thousand strings do not fit in one go of two hundred");
    assert_eq!(spent(&out, "str", "cstr"), (N as u64 * 3 * 8, N as u64));
    assert_eq!(out.covered_bits, d.len_bits());
    assert_eq!(out.unmapped_bits, 0);
    // The walk drops what it has gone past, so what is left is the window it
    // keeps and not five thousand strings.
    assert!(ev.memo_len() < 64, "{} nodes left behind", ev.memo_len());
}

/// A run that fills its container with elements whose length their own bytes
/// give cannot be counted without decoding all of it, so the walk counts them
/// as it goes and stops when the room runs out.
#[test]
fn a_run_with_no_count_is_walked_to_the_end_of_its_room() {
    let t = Template::new("t", T::repeat(T::cstr(), Until::End));
    let d = doc(b"one\0two\0three\0");
    let mut ev = Evaluator::new(t);
    let (out, _) = kinds_to_the_end(&mut ev, &d);
    assert_eq!(spent(&out, "str", "cstr"), (14 * 8, 3));
    assert_eq!(out.covered_bits, d.len_bits());
    assert_eq!(out.unmapped_bits, 0);
}

/// A field that points back at bytes its siblings have already described is a
/// second reading of them, not more of the file. npy writes its dtype as text
/// and then a record view over the same text; xz lists its blocks and then
/// points at the whole stream to say what it unpacks to. Counting both would
/// say the file is twice the size it is.
#[test]
fn a_view_over_bytes_the_fields_describe_is_not_counted_twice() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![("a", T::u32(Big)), ("view", T::at_in_window(E::lit(0), T::bytes(E::lit(4))))],
        ),
    );
    let d = doc(&[1, 2, 3, 4]);
    let mut ev = Evaluator::new(t);
    let (out, _) = kinds_to_the_end(&mut ev, &d);
    assert_eq!(spent(&out, "uint", "u32 be"), (32, 1));
    assert_eq!(spent(&out, "bytes", "bytes[]"), (0, 0));
    assert_eq!(out.covered_bits, 32);
    assert_eq!(out.unmapped_bits, 0);

    // The same field pointing somewhere the run has not covered is a field
    // like any other: this is how an AppleDouble reaches its data fork.
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![("a", T::u32(Big)), ("fork", T::at_in_window(E::lit(4), T::bytes(E::lit(4))))],
        ),
    );
    let d = doc(&[1, 2, 3, 4, 5, 6, 7, 8]);
    let mut ev = Evaluator::new(t);
    let (out, _) = kinds_to_the_end(&mut ev, &d);
    assert_eq!(spent(&out, "bytes", "bytes[]"), (32, 1));
    assert_eq!(out.covered_bits, 64);
    assert_eq!(out.unmapped_bits, 0);
}

/// Two addresses naming the same thing reach it twice and count it once. An
/// HDF5 group can hold any number of links to one object, and the template
/// follows every one, since each is a way a reader gets there; counted every
/// time, a group of two thousand links to one dataset covers more bits than
/// its file has.
///
/// Something else at the same place is not the same thing: an address to the
/// same start with a different length still counts.
#[test]
fn a_thing_two_addresses_name_is_counted_once() {
    let t = Template::new(
        "t",
        T::structure(
            "Root",
            vec![
                ("first", T::u8()),
                ("second", T::u8()),
                ("one", T::at(E::field("first"), T::structure("Thing", vec![("a", T::u16(Big))]))),
                ("two", T::at(E::field("second"), T::structure("Thing", vec![("a", T::u16(Big))]))),
                ("wider", T::at(E::field("first"), T::bytes(E::lit(4)))),
            ],
        ),
    );
    let d = doc(&[4, 4, 0, 0, 1, 2, 3, 4]);
    let mut ev = Evaluator::new(t);
    let (out, _) = kinds_to_the_end(&mut ev, &d);
    assert_eq!(spent(&out, "uint", "u16 be"), (16, 1));
    assert_eq!(spent(&out, "bytes", "bytes[]"), (32, 1));
    assert!(out.covered_bits <= d.len_bits(), "{out:?}");
}

/// Bytes a structure does not cover are a gap. Nothing but the gap is
/// recorded: a composite's own bits are its children's, and the only bits it
/// contributes are the ones it is left holding.
#[test]
fn a_composite_contributes_only_what_its_children_leave_over() {
    // A window of eight bytes holding a structure that fills four.
    let t = Template::new("t", T::sized(E::lit(8), T::structure("Head", vec![("a", T::u32(Big))])));
    let d = doc(&[0; 8]);
    let mut ev = Evaluator::new(t);
    let (out, _) = kinds_to_the_end(&mut ev, &d);
    assert_eq!(spent(&out, "uint", "u32 be"), (32, 1));
    assert_eq!(out.unmapped_bits, 32);
    assert_eq!(out.covered_bits + out.unmapped_bits, d.len_bits());
}
