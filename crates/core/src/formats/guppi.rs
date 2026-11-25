//! GUPPI raw voltages (`.raw`): the baseband format the Green Bank pulsar
//! backend wrote and Breakthrough Listen's recorders write still. The file is
//! a run of blocks, each an ASCII header of 80-byte FITS-style cards ending
//! with `END`, and then `BLOCSIZE` bytes of complex voltage samples.
//!
//! With `DIRECTIO = 1` the recorder wrote with the operating system's direct
//! I/O, which needs every write to be a multiple of 512 bytes, so the header
//! is padded to one. Without it there is no padding.
//!
//! The samples of a block are channel by channel: `OBSNCHAN` runs, each of
//! `BLOCSIZE / OBSNCHAN` bytes, and inside a channel one time sample after
//! another. A time sample is `NPOL` values of `NBITS` each. `NPOL = 4` is
//! two polarisations of complex samples, the real and the imaginary part of
//! the first and then of the second, which is how blimpy's GUPPI reader
//! splits them. So a channel holds `BLOCSIZE * 8 / (OBSNCHAN * NPOL * NBITS)`
//! time samples, 516,608 in the Voyager 1 file in the sample collection, and
//! `NPKT * PKTSIZE` there is `BLOCSIZE` too.
//!
//! Checked against the Voyager 1 file: the statistics of the 2-bit levels
//! change smoothly over each 1/64 of the block and jump between them, which
//! is what channel-by-channel order looks like and time-by-time would not; and
//! the first two values of a byte have the same statistics as each other in
//! every channel, as do the last two, which is how one polarisation's real
//! and imaginary parts behave.
//!
//! What is read and what is not:
//!
//! - 8-bit samples read as signed bytes and 2-bit samples as the codes 0 to
//!   3, first value in the top two bits. That order is taken from blimpy's
//!   `unpack_2to8`, from memory, and the check above cannot tell it from the
//!   reverse. blimpy turns the codes into the levels +40, +12, -12 and -40,
//!   in that order; the codes are what the file holds and are left as codes.
//!   4-bit samples are left as bytes.
//! - With `DIRECTIO = 1`, the data is padded to a multiple of 512 bytes too.
//!   No sample written that way was at hand.
//! - `OVERLAP`, the time samples a block repeats from the one before it for a
//!   filter to run across the join, are read as samples like any other.

use super::fits::card_at;
use super::fits_cards::card_reading;
use crate::template::{Endian::Big, Expr as E, TableFact, TableShape, Template, Ty as T, Until};

/// The cards that size and place the data, which a FITS file has no reason
/// to read as numbers. `NBITS` and `NPOL` are PSRFITS cards as well, and
/// already read as numbers.
const NUMERIC: &[&str] = &["BLOCSIZE", "OBSNCHAN", "DIRECTIO", "OVERLAP", "PKTSIZE", "NPKT", "PKTIDX"];

/// What direct I/O writes in multiples of.
const DIRECT_IO: u32 = 512;

fn card_value(name: &str) -> E {
    card_at(name, &["body", "value"])
}

/// The bytes of one channel of a block.
fn channel_bytes() -> E {
    card_value("BLOCSIZE").div(card_value("OBSNCHAN").at_least(E::lit(1)))
}

/// One channel's time samples, as wide as `NBITS` says, as a table of one
/// row a time sample and one column a value.
fn channel() -> T {
    let run = |bits: u32, elem: T| T::array(elem, E::Remaining.mul(E::lit(8)).div(E::lit(i128::from(bits))));
    let values = T::switch(
        card_value("NBITS"),
        vec![(8, run(8, T::Int { bits: 8, endian: Big })), (2, run(2, T::UInt { bits: 2, endian: Big }))],
        T::bytes(E::Remaining),
    );
    let shape = TableShape {
        columns: Some(card_value("NPOL")),
        names: vec!["pol 1 real".into(), "pol 1 imaginary".into(), "pol 2 real".into(), "pol 2 imaginary".into()],
        units: vec!["".into(), "".into(), "".into(), "".into()],
        column_word: Some("value".into()),
        row_word: Some("sample".into()),
        rate: Some(E::real(1.0).div(E::real_text(card_at("TBIN", &["body", "value"])))),
        facts: vec![
            TableFact::named("NBITS", card_value("NBITS")),
            TableFact::named("NPOL", card_value("NPOL")),
            TableFact::named("OBSNCHAN", card_value("OBSNCHAN")),
            TableFact::named("TBIN", card_value("TBIN")).in_unit("s"),
            TableFact::named("CHAN_BW", card_value("CHAN_BW")).in_unit("MHz"),
            TableFact::named("OBSFREQ", card_value("OBSFREQ")).in_unit("MHz"),
        ],
        ..Default::default()
    };
    T::structure("Channel", vec![("samples", values)]).payload(&["samples"]).field_table("samples", shape).counted_as("channel")
}

/// A block's samples: `OBSNCHAN` channels of `BLOCSIZE / OBSNCHAN` bytes
/// each. A block the file cuts short has the channels it has room for, the
/// last of them short.
fn voltages() -> T {
    let each = channel_bytes().at_least(E::lit(1));
    let count = card_value("OBSNCHAN").at_most(E::Remaining.div_ceil(each.clone()));
    T::structure("Voltages", vec![("channels", T::array(T::sized(each.at_most(E::Remaining), channel()), count))])
}

/// One block: the cards, the padding direct I/O asks for, and the samples.
fn block() -> T {
    let direct = E::lit(0).less_than(card_value("DIRECTIO"));
    T::structure(
        "Block",
        vec![
            ("cards", T::repeat(card_reading(NUMERIC), Until::FieldBytes { field: "key".into(), bytes: b"END     ".to_vec() })),
            ("header_pad", T::bytes(E::cond(direct.clone(), E::size_of("cards").pad_to(DIRECT_IO), E::lit(0)).at_most(E::Remaining))),
            ("data", T::sized(card_value("BLOCSIZE").at_most(E::Remaining), voltages())),
            ("data_pad", T::bytes(E::cond(direct, E::size_of("data").pad_to(DIRECT_IO), E::lit(0)).at_most(E::Remaining))),
        ],
    )
    .machinery(&["header_pad", "data_pad"])
    .counted_as("block")
}

pub fn guppi() -> Template {
    Template::new("guppi", T::structure("GUPPI raw", vec![("blocks", T::repeat(block(), Until::End))]))
}

/// Whether the front of a file is a GUPPI block header: cards of printable
/// text, `BLOCSIZE`, `OBSNCHAN` and `NBITS` among them, ending with `END`
/// inside `head`. A FITS file opens with `SIMPLE` and is not one.
pub fn is_guppi(head: &[u8]) -> bool {
    if head.starts_with(b"SIMPLE  =") || head.starts_with(b"XTENSION=") {
        return false;
    }
    let mut seen = [false; 3];
    for card in head.chunks_exact(80) {
        if !card.iter().all(|b| (0x20..0x7f).contains(b)) {
            return false;
        }
        let key = &card[..8];
        if key == b"END     " {
            return seen.iter().all(|s| *s);
        }
        if &card[8..10] != b"= " && key != b"COMMENT " && key != b"HISTORY " && key != b"        " {
            return false;
        }
        for (i, k) in [b"BLOCSIZE", b"OBSNCHAN", b"NBITS   "].iter().enumerate() {
            seen[i] |= key == *k;
        }
    }
    false
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// A block header of these cards and `END`, padded to 512 bytes when
    /// `DIRECTIO = 1` is among them.
    pub(crate) fn header(cards: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (k, v) in cards {
            // A string starts in column 11, a number ends in column 30.
            let card = if v.starts_with('\'') { format!("{k:<8}= {v}") } else { format!("{k:<8}= {v:>20}") };
            out.extend_from_slice(card.as_bytes());
            out.resize(out.len().next_multiple_of(80), b' ');
        }
        out.extend_from_slice(b"END");
        out.resize(out.len().next_multiple_of(80), b' ');
        if cards.iter().any(|(k, v)| *k == "DIRECTIO" && v.trim() == "1") {
            out.resize(out.len().next_multiple_of(512), 0);
        }
        out
    }

    fn cards(nbits: &str, blocsize: usize, directio: &str) -> Vec<(&'static str, String)> {
        vec![
            ("BACKEND", "'GUPPI   '".into()),
            ("NBITS", nbits.into()),
            ("OBSNCHAN", "2".into()),
            ("NPOL", "4".into()),
            ("CHAN_BW", "2.9296875".into()),
            ("OBSFREQ", "9281.25".into()),
            ("TBIN", "3.41333333333E-07".into()),
            ("DIRECTIO", directio.into()),
            ("BLOCSIZE", blocsize.to_string()),
        ]
    }

    fn block(nbits: &str, data: &[u8], directio: &str) -> Vec<u8> {
        let c = cards(nbits, data.len(), directio);
        let refs: Vec<(&str, &str)> = c.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let mut out = header(&refs);
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn a_block_header_is_recognised_and_a_fits_file_is_not() {
        let b = block("8", &[0; 16], "0");
        assert!(is_guppi(&b));
        assert_eq!(crate::formats::sniff(&b, b.len() as u64), Some("guppi"));
        let mut fits = b"SIMPLE  =                    T".to_vec();
        fits.resize(80, b' ');
        fits.extend_from_slice(&b);
        assert!(!is_guppi(&fits));
    }

    #[test]
    fn eight_bit_samples_are_channel_by_channel_and_signed() {
        // Two channels of two time samples of four values.
        let data: Vec<u8> = (0u8..16).map(|i| i.wrapping_mul(17)).collect();
        let b = [block("8", &data, "0"), block("8", &data, "0")].concat();
        let d = Document::new(MemSource(b.clone()));
        let mut ev = Evaluator::new(guppi());
        assert_eq!(ev.node(&d, &[0]).unwrap().child_count, 2);
        let channels = ev.child_named(&d, &[0, 1, 2], "channels").unwrap().unwrap();
        assert_eq!(ev.node(&d, &channels).unwrap().child_count, 2);
        let second = [channels.clone(), vec![1, 0]].concat();
        let run = ev.node(&d, &second).unwrap();
        assert_eq!((run.type_name.as_str(), run.child_count), ("i8[]", 8));
        // The second channel opens with the ninth byte, 8 * 17 = 136, which
        // signed is -120.
        assert_eq!(ev.node(&d, &[second.clone(), vec![0]].concat()).unwrap().value.as_int(), Some(-120));
        let shape = ev.table_shape(&d, &second).unwrap().expect("a shape");
        assert_eq!(shape.columns, Some(4));
        assert_eq!(shape.rate, Some(1.0 / 3.41333333333e-7));
        assert_eq!(shape.names[0], "pol 1 real");
        let facts: Vec<(&str, &str)> = shape.facts.iter().map(|f| (f.label.as_str(), f.unit.as_str())).collect();
        assert_eq!(facts[3], ("TBIN", "s"));
        // The two blocks fill the file.
        let last = ev.node(&d, &[0, 1]).unwrap();
        assert_eq!(last.offset_bits + last.size_bits, b.len() as u64 * 8);
    }

    #[test]
    fn two_bit_samples_read_top_bits_first() {
        let b = block("2", &[0b00_01_10_11, 0b11_10_01_00], "0");
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(guppi());
        let channels = ev.child_named(&d, &[0, 0, 2], "channels").unwrap().unwrap();
        let first = [channels, vec![0, 0]].concat();
        let values: Vec<i128> = (0..4).map(|i| ev.node(&d, &[first.clone(), vec![i]].concat()).unwrap().value.as_int().unwrap()).collect();
        assert_eq!(values, [0, 1, 2, 3]);
    }

    #[test]
    fn direct_io_pads_the_header_to_512_bytes() {
        let b = block("8", &[7; 512], "1");
        let d = Document::new(MemSource(b.clone()));
        let mut ev = Evaluator::new(guppi());
        let pad = ev.node(&d, &[0, 0, 1]).unwrap();
        assert_eq!((pad.offset_bits + pad.size_bits) % (512 * 8), 0);
        assert!(pad.size_bits > 0);
        let data = ev.node(&d, &[0, 0, 2]).unwrap();
        assert_eq!(data.size_bits, 512 * 8);
        assert_eq!(ev.node(&d, &[0]).unwrap().size_bits, b.len() as u64 * 8);
    }

    #[test]
    fn a_block_cut_short_has_the_channels_it_has_room_for() {
        let mut b = block("8", &[1; 16], "0");
        b.truncate(b.len() - 5);
        let d = Document::new(MemSource(b));
        let mut ev = Evaluator::new(guppi());
        let channels = ev.child_named(&d, &[0, 0, 2], "channels").unwrap().unwrap();
        assert_eq!(ev.node(&d, &channels).unwrap().child_count, 2);
        let second = [channels, vec![1, 0]].concat();
        assert_eq!(ev.node(&d, &second).unwrap().child_count, 3);
        assert!(matches!(ev.node(&d, &[second, vec![2]].concat()).unwrap().value, Value::Int(1)));
    }
}
