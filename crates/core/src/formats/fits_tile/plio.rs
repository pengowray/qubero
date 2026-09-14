//! PLIO_1: a tile's pixel list read back into its integers.
//!
//! IRAF's pixel lists, which CFITSIO's `pliocomp.c` carries over as `pl_l2pi`,
//! are for masks: large areas of one small value. A tile is a list of 16-bit
//! words, big-endian in the heap (`TFORMn = 1PI`). The list opens with a
//! header and then holds instructions, one word each: the top four bits say
//! what to do and the low twelve are a count or an amount.
//!
//! The header is seven words, and its third is negative (CFITSIO writes -100).
//! The second says how long the header is, so the instructions start after it,
//! and the fourth and fifth are the length of the whole list, header included,
//! as the low fifteen bits and then the rest. An older list has a header of
//! three words whose third, positive, is the length.
//!
//! The reader keeps a current value, which starts at 1, and a position, which
//! starts at the first pixel. The instructions, by their top four bits:
//!
//! - 0: the next `n` pixels are 0.
//! - 1: the current value is the next word times 4096, plus `n`. The next word
//!   is not an instruction.
//! - 2 and 3: add `n` to the current value, or take it away, and write nothing.
//! - 4: the next `n` pixels are the current value.
//! - 5: the next `n` pixels are 0, except the last, which is the current value.
//! - 6 and 7: add `n` or take it away, and the next pixel is the new value.
//!
//! A word whose top bits say none of these, which only a word read as a
//! negative number can be, is passed over. The list ends when the pixels do or
//! the words do, and every pixel it did not reach is 0.
//!
//! A run that would go past the tile's last pixel stops at it, and a 5 whose
//! run was cut short that way writes no value at its end.

use super::{commas, pixel_word, plural, stopped, Image, Step, Tile, Values};

/// PLIO: the list read into the tile's integers.
pub(super) fn step(tile: &mut Tile, image: &Image, data: &[u8], pixels: usize) -> Option<Values> {
    let name = image.algorithm.as_str();
    let words: Vec<i16> = data.chunks_exact(2).map(|w| i16::from_be_bytes([w[0], w[1]])).collect();
    let list = match read(&words, pixels) {
        Ok(list) => list,
        Err(Refused::Header) => {
            stopped(tile, name, data.len());
            tile.problem = Some(format!(
                "Stopped at {name}: the list is {}, shorter than its header (3 or 7 words).",
                plural(words.len() as u64, "word", "words")
            ));
            return None;
        }
        Err(Refused::Start(raw)) => {
            stopped(tile, name, data.len());
            tile.problem = Some(format!("Stopped at {name}: the list's second word gives the header length as {raw}, which is negative."));
            return None;
        }
    };
    tile.steps.push(Step { what: name.into(), in_bytes: data.len(), out_bytes: 0, note: note(&list, pixels) });
    if list.ran_out {
        tile.problem = Some(format!(
            "Stopped at {name}: the data ran out after {} of {}; the list says it is {}, but the tile holds only {}.",
            commas(list.values.len() as u64),
            pixel_word(pixels as u64),
            plural(list.length.max(0) as u64, "word", "words"),
            plural(words.len() as u64, "word", "words")
        ));
    }
    Some(Values::Ints(list.values))
}

/// What the list was made of, for the step's note.
fn note(list: &List, pixels: usize) -> String {
    let count = |n: usize, one: &str, many: &str| plural(n as u64, one, many);
    let instructions = list.zero_runs + list.value_runs + list.ending_runs + list.set + list.changes + list.singles;
    let mut note = format!(
        "pixel list, {}; a {}-word header, then {}: {}, {}, {}, {}, {}, {}",
        count(list.length.max(0) as usize, "word of 16 bits", "words of 16 bits"),
        list.header,
        count(instructions, "instruction", "instructions"),
        count(list.zero_runs, "run of zeros", "runs of zeros"),
        count(list.value_runs, "run of the current value", "runs of the current value"),
        count(list.ending_runs, "run of zeros ending in the value", "runs of zeros ending in the value"),
        count(list.set, "value set from two words", "values set from two words"),
        count(list.changes, "change to the value with no pixel written", "changes to the value with no pixel written"),
        count(list.singles, "change to the value with one pixel written", "changes to the value with one pixel written"),
    );
    if list.passed_over > 0 {
        note.push_str(&format!(", and {} (unknown opcode)", count(list.passed_over, "word skipped", "words skipped")));
    }
    let left = list.length.saturating_sub(list.used as i64);
    if list.finished_early && left > 0 {
        note.push_str(&format!("; all pixels written by word {}, the last {} not read", commas(list.used as u64), count(left as usize, "word", "words")));
    }
    if list.filled == pixels && pixels > 0 {
        note.push_str(&format!("; the list writes no pixels, so all {} are 0", commas(pixels as u64)));
    } else if list.filled == 1 {
        note.push_str("; the list covers all but the last pixel, which is 0");
    } else if list.filled > 0 {
        note.push_str(&format!("; the list covers all but the last {}, which are 0", pixel_word(list.filled as u64)));
    }
    note
}

/// Why a list could not be read at all.
#[derive(Debug, PartialEq)]
enum Refused {
    /// Fewer words than the header needs.
    Header,
    /// A header whose second word, the header's length, is negative, which
    /// would start the instructions before the list.
    Start(i64),
}

/// What a list came to, and what it was made of.
#[derive(Debug, Default)]
struct List {
    values: Vec<i64>,
    /// How many words the header is: 7, or 3 for the older form.
    header: usize,
    /// How long the header says the list is, header included.
    length: i64,
    /// How many words were read, header included.
    used: usize,
    zero_runs: usize,
    value_runs: usize,
    ending_runs: usize,
    set: usize,
    changes: usize,
    singles: usize,
    passed_over: usize,
    /// Pixels after the last one the list reached, made 0.
    filled: usize,
    /// Whether the words ran out before the list's length or the tile's
    /// pixels did.
    ran_out: bool,
    /// Whether every pixel was made before the list's length was reached.
    finished_early: bool,
}

/// `pl_l2pi`, reading every pixel of the tile from the start. See the module
/// doc.
///
/// Positions are counted in `i64`, which the tile's limit keeps far from
/// overflowing: a word moves the position or the value by at most 4,095 and
/// sets the value to at most 2^27 or so, and a tile under the compressed
/// limit is under nine million words.
fn read(words: &[i16], pixels: usize) -> Result<List, Refused> {
    let word = |i: usize| words.get(i).map(|w| i64::from(*w));
    let Some(third) = word(2) else { return Err(Refused::Header) };
    let mut list = List::default();
    // Where the instructions start, counted from 0.
    let first = if third > 0 {
        list.header = 3;
        list.length = third;
        3
    } else {
        let (Some(header), Some(low), Some(high)) = (word(1), word(3), word(4)) else { return Err(Refused::Header) };
        list.header = 7;
        list.length = (high << 15) + low;
        if header < 0 {
            return Err(Refused::Start(header));
        }
        header as usize
    };
    list.used = first.min(words.len());
    let npix = pixels as i64;
    let mut values: Vec<i64> = Vec::with_capacity(pixels);
    if npix <= 0 || list.length <= 0 {
        list.filled = pixels;
        list.values = vec![0; pixels];
        return Ok(list);
    }
    // The pixel the next instruction starts at, counted from 1, and the
    // current value.
    let mut x1: i64 = 1;
    let mut pv: i64 = 1;
    let mut ip = first;
    while (ip as i64) < list.length {
        let Some(w) = word(ip) else {
            list.ran_out = true;
            list.values = values;
            return Ok(list);
        };
        list.used = ip + 1;
        let opcode = w / 4096;
        let data = w & 4095;
        match opcode {
            0 | 4 | 5 => {
                let x2 = x1 + data - 1;
                let i1 = x1.max(1);
                let i2 = x2.min(npix);
                let np = i2 - i1 + 1;
                if np > 0 {
                    if opcode == 4 {
                        values.extend(std::iter::repeat_n(pv, np as usize));
                    } else {
                        values.extend(std::iter::repeat_n(0, np as usize));
                        if let (5, true, Some(last)) = (opcode, i2 == x2, values.last_mut()) {
                            *last = pv;
                        }
                    }
                }
                match opcode {
                    0 => list.zero_runs += 1,
                    4 => list.value_runs += 1,
                    _ => list.ending_runs += 1,
                }
                x1 = x2 + 1;
            }
            1 => {
                let Some(high) = word(ip + 1) else {
                    list.ran_out = true;
                    list.values = values;
                    return Ok(list);
                };
                list.used = ip + 2;
                pv = (high << 12) + data;
                list.set += 1;
                // The word after is the value's high part, not an instruction.
                ip += 2;
                continue;
            }
            2 | 3 => {
                pv = if opcode == 2 { pv + data } else { pv - data };
                list.changes += 1;
            }
            6 | 7 => {
                pv = if opcode == 6 { pv + data } else { pv - data };
                if (1..=npix).contains(&x1) {
                    values.push(pv);
                }
                list.singles += 1;
                x1 += 1;
            }
            _ => list.passed_over += 1,
        }
        if x1 > npix {
            list.finished_early = true;
            break;
        }
        ip += 1;
    }
    list.filled = pixels - values.len();
    values.resize(pixels, 0);
    list.values = values;
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::super::tests::cards;
    use super::super::{decode, Place, Row, Stored};
    use super::*;

    /// A one-row image of 32-bit integers, `pixels` long, compressed as the
    /// list `words`, decoded the way the panel decodes it.
    fn decoded(words: &[i16], pixels: usize) -> Tile {
        let lines = ["ZBITPIX =                   32".to_string(), "ZNAXIS  =                    1".into(), format!("ZNAXIS1 = {pixels:20}"), "ZCMPTYPE= 'PLIO_1  '".into()];
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let image = Image::from_cards(&cards(&refs)).unwrap();
        let data: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
        let row = Row {
            place: Some(Place { stored: Stored::Compressed, count: words.len() as u64, offset: 0, elem: b'I' }),
            has_data_column: true,
            zscale: None,
            zzero: None,
            zblank: None,
            quantized: false,
        };
        decode(&image, 0, &row, &data)
    }

    #[test]
    fn the_note_says_what_the_list_did_not_write_and_did_not_read() {
        let note = |words: &[i16], pixels: usize| decoded(words, pixels).steps[0].note.clone();
        assert_eq!(
            note(&list(&[op(4, 2)]), 5),
            "pixel list, 8 words of 16 bits; a 7-word header, then 1 instruction: 0 runs of zeros, 1 run of the current value, 0 runs of zeros ending in the value, 0 values set from two words, 0 changes to the value with no pixel written, 0 changes to the value with one pixel written; the list covers all but the last 3 pixels, which are 0"
        );
        assert!(note(&list(&[op(4, 2)]), 3).ends_with("; the list covers all but the last pixel, which is 0"));
        let mut empty = list(&[op(4, 2)]);
        empty[3] = 0;
        assert!(note(&empty, 3).ends_with("; the list writes no pixels, so all 3 are 0"));
        assert!(note(&list(&[op(4, 5), op(4, 5)]), 5).ends_with("; all pixels written by word 8, the last 1 word not read"));
        assert!(note(&list(&[0xf000, op(4, 1)]), 1).ends_with(", and 1 word skipped (unknown opcode)"));
    }

    #[test]
    fn a_list_that_is_refused_or_runs_out_says_why() {
        let problem = |words: &[i16], pixels: usize| decoded(words, pixels).problem.unwrap_or_default();
        assert_eq!(problem(&[0, 7], 4), "Stopped at PLIO_1: the list is 2 words, shorter than its header (3 or 7 words).");
        assert_eq!(problem(&[0, -3, -100, 9, 0, 0, 0], 4), "Stopped at PLIO_1: the list's second word gives the header length as -3, which is negative.");
        let mut short = list(&[op(4, 2), op(0, 2)]);
        short[3] = 40;
        short.pop();
        let t = decoded(&short, 8);
        assert_eq!(t.pixels, [1.0, 1.0]);
        assert_eq!(
            t.problem.as_deref(),
            Some("Stopped at PLIO_1: the data ran out after 2 of 8 pixels; the list says it is 40 words, but the tile holds only 8 words.")
        );
    }

    /// A list with the seven-word header CFITSIO writes, round these
    /// instructions.
    fn list(instructions: &[u16]) -> Vec<i16> {
        let length = 7 + instructions.len();
        let mut w = vec![0, 7, -100, (length % 32768) as i16, (length / 32768) as i16, 0, 0];
        w.extend(instructions.iter().map(|i| *i as i16));
        w
    }

    const fn op(code: u16, n: u16) -> u16 {
        code * 4096 + n
    }

    #[test]
    fn every_instruction_does_what_the_list_says() {
        let words = list(&[
            op(0, 3),     // three zeros
            op(4, 2),     // the current value, which starts at 1, twice
            op(1, 0x234), // the value is 0x1234...
            0x0001,       // ...from this word and the twelve bits before
            op(4, 1),
            op(2, 6),     // up by six, nothing written
            op(5, 3),     // two zeros, then the value
            op(3, 0x240), // down to 0x1000
            op(6, 1),     // up by one and written
            op(7, 2),     // down by two and written
        ]);
        let got = read(&words, 12).unwrap();
        assert_eq!(got.values, [0, 0, 0, 1, 1, 0x1234, 0, 0, 0x123a, 0x0ffb, 0x0ff9, 0]);
        assert_eq!((got.zero_runs, got.value_runs, got.ending_runs, got.set, got.changes, got.singles), (1, 2, 1, 1, 2, 2));
        assert_eq!((got.header, got.length, got.used, got.filled, got.ran_out), (7, 17, 17, 1, false));
    }

    #[test]
    fn a_run_past_the_last_pixel_stops_at_it_and_writes_no_value_at_its_end() {
        let words = list(&[op(4, 2), op(5, 10)]);
        let got = read(&words, 5).unwrap();
        assert_eq!(got.values, [1, 1, 0, 0, 0]);
        // The reading stops once the pixels are all made, whatever is left.
        let words = list(&[op(4, 5), op(4, 5)]);
        let got = read(&words, 5).unwrap();
        assert_eq!((got.values, got.used), (vec![1; 5], 8));
    }

    #[test]
    fn the_older_header_is_three_words_and_its_length() {
        let words = [0, 0, 5, op(0, 1) as i16, op(4, 2) as i16];
        let got = read(&words, 4).unwrap();
        assert_eq!((got.values, got.header, got.filled), (vec![0, 1, 1, 0], 3, 1));
    }

    #[test]
    fn a_word_read_as_negative_is_passed_over_or_is_a_run_of_zeros() {
        // -1 is 0xffff: divided by 4096 toward zero it is instruction 0, a run
        // of 4,095 zeros. -4096 is instruction -1, which is nothing.
        let mut words = list(&[]);
        words.extend([-4096, op(4, 1) as i16, -1, op(4, 1) as i16]);
        words[3] = words.len() as i16;
        let got = read(&words, 4100).unwrap();
        assert_eq!((got.passed_over, got.zero_runs, got.value_runs), (1, 1, 2));
        assert_eq!((got.values[0], got.values[1], got.values[4096], got.values[4097]), (1, 0, 1, 0));
    }

    #[test]
    fn a_broken_list_is_refused_or_stops_where_its_words_do() {
        assert_eq!(read(&[0, 7], 4).unwrap_err(), Refused::Header);
        assert_eq!(read(&[0, 7, -100, 9], 4).unwrap_err(), Refused::Header);
        assert_eq!(read(&[0, -3, -100, 9, 0, 0, 0], 4).unwrap_err(), Refused::Start(-3));
        // A list that says it is longer than its words.
        let mut words = list(&[op(4, 2), op(0, 2)]);
        words[3] = 40;
        words.pop();
        let got = read(&words, 8).unwrap();
        assert_eq!((got.values, got.ran_out), (vec![1, 1], true));
        // A value whose second word is missing.
        let mut words = list(&[op(1, 5)]);
        words[3] = 9;
        let got = read(&words, 3).unwrap();
        assert!(got.ran_out && got.values.is_empty());
        // A list of no length, or a header past the end, is all zeros, which
        // is what CFITSIO leaves.
        let mut words = list(&[op(4, 2)]);
        words[3] = 0;
        assert_eq!(read(&words, 3).unwrap().values, [0, 0, 0]);
        let words = [0, 400, -100, 9, 0, 0, 0, op(4, 2) as i16, 0];
        let got = read(&words, 3).unwrap();
        assert_eq!((got.values, got.ran_out), (vec![0, 0, 0], false));
        // One that starts past its words and says it is longer runs out.
        let words = [0, 400, -100, 1000, 0, 0, 0, op(4, 2) as i16, 0];
        assert!(read(&words, 3).unwrap().ran_out);
    }

    /// Lists of random words, with headers random and sound. None may panic,
    /// and every one that does not run out makes the tile's pixels exactly.
    #[test]
    fn random_words_never_panic() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..3000 {
            let n = (next() % 60) as usize;
            let mut words: Vec<i16> = (0..n).map(|_| next() as i16).collect();
            if round % 2 == 0 && n >= 7 {
                words[1] = (next() % 9) as i16;
                words[2] = -100;
                words[3] = (next() % 70) as i16;
                words[4] = 0;
            }
            let pixels = (next() % 5000) as usize;
            if let Ok(got) = read(&words, pixels) {
                assert!(got.ran_out || got.values.len() == pixels, "round {round}");
                assert!(got.values.len() <= pixels, "round {round}");
            }
        }
    }

    #[test]
    fn a_long_list_of_the_largest_words_does_not_overflow() {
        // Every word raises the value by 4,095 and writes it, until the value
        // is past what a 32-bit integer holds.
        let mut words = list(&[]);
        let n = 600_000usize;
        words.extend(std::iter::repeat_n(op(6, 4095) as i16, n));
        let length = words.len();
        words[3] = (length % 32768) as i16;
        words[4] = (length / 32768) as i16;
        let got = read(&words, n).unwrap();
        assert_eq!(got.values.last(), Some(&(1 + 4095 * n as i64)));
        assert!(1 + 4095 * n as i64 > i64::from(i32::MAX));
    }
}
