//! A miniSEED record's samples, worked out from the bytes that hold them.
//!
//! The templates in [`mseed`](super::mseed) and [`mseed3`](super::mseed3) lay
//! a record's data out as far as its shape goes: Steim frames as their code
//! words and data words, each word as the differences its code names, and a
//! fixed-width run as its numbers. What no template can do is add the
//! differences up. A sample is the one before it plus a difference, which is a
//! running sum, and no expression in the IR carries a value from one element
//! of a list to the next. So this says what the samples are, the arrangement
//! [`hdf5_chunk`](super::hdf5_chunk) has for a filtered chunk.
//!
//! Every step is reported, not only the answer. For Steim that is the frame
//! count, the two integration constants in frame 0, how many differences each
//! frame held and how many of them became samples, the first difference (which
//! a reader skips, since it is measured from the last sample of the record
//! before), and then the check the format was built with: the last sample has
//! to come out equal to the reverse integration constant. A record that fails
//! it has said something about itself, and the panel says which two numbers
//! disagreed.
//!
//! The sample count comes from the header and the walk stops there. A frame
//! is written a word at a time and a Steim2 word holds up to seven
//! differences whether or not seven are left, so the last frame of a record
//! usually holds more differences than there are samples to make.
//!
//! The bytes are read here rather than the template's own difference fields,
//! though the template has already cut every word up. A 4096-byte record is a
//! few thousand difference fields, several of them computed, and the panel is
//! asked again every time the cursor moves; reading sixty-four bytes at a time
//! is the cheaper way to the same numbers. That it is the same numbers is
//! checked rather than hoped: the tests put the same frames through the
//! template and through this, in both byte orders, and compare every
//! difference.
//!
//! The byte order is the part to get right, and it follows the template's
//! notes. A big-endian record is read as it lies. A little-endian 2.4 record
//! swaps each difference that is a whole number of bytes on its own (so four
//! 8-bit differences are the four bytes in order), and swaps a word that packs
//! differences into bit fields whole, before cutting it up. miniSEED 3 writes
//! its Steim frames big-endian inside a little-endian record, and the template
//! says which order it laid the frames out in, so this never has to guess.
//!
//! The same question is answered for every other encoding a record can be in,
//! so that a record says what its samples are whichever way it was written:
//! 16, 24 and 32-bit integers and 32 and 64-bit floats are read at their width
//! and byte order, and three of the gain-ranged encodings are worked out by
//! the rules the SEED 2.4 manual gives for them in Appendix D:
//!
//! - CDSN (16): the low fourteen bits less 8191, times 1, 4, 16 or 128 as the
//!   top two bits say.
//! - SRO (30): the low twelve bits as a two's complement number, times two to
//!   the power of ten less the top four bits.
//! - DWWSSN (32): a 16-bit two's complement number, which the template already
//!   reads as one.
//!
//! GEOSCOPE's three encodings, the US National Network's, Graefenberg's and
//! IPG Strasbourg's have no rule in the manual or in libmseed's documentation,
//! so they are named and left as the words they are. A guessed rule would
//! give numbers that look like samples.

use crate::template::Endian;

/// The start of what [`StructDef::packed`](crate::template::StructDef::packed)
/// is set to on a record's data. The encoding and the byte order follow it, so
/// the reader is told both by the template that laid the data out rather than
/// working either out again.
const PACKING: &str = "mseed_samples";

/// The largest payload this will decode. A record is 512 or 4096 bytes in the
/// files people write, and miniSEED 3 allows any length up to four gigabytes;
/// a claim far past a megabyte is a reason to stop rather than to allocate.
pub const PAYLOAD_LIMIT: usize = 1 << 20;

/// A Steim frame: a code word and fifteen data words.
const FRAME: usize = 64;

/// The packing name for one encoding in one byte order, for a template to set.
pub fn packing(encoding: u8, e: Endian) -> String {
    let order = if matches!(e, Endian::Big) { "be" } else { "le" };
    format!("{PACKING}:{encoding}:{order}")
}

/// The encoding and byte order a packing name carries, where it is one of
/// these. `true` is big-endian.
pub fn parse_packing(packing: &str) -> Option<(u8, bool)> {
    let rest = packing.strip_prefix(PACKING)?.strip_prefix(':')?;
    let (encoding, order) = rest.split_once(':')?;
    let big = match order {
        "be" => true,
        "le" => false,
        _ => return None,
    };
    Some((encoding.parse().ok()?, big))
}

/// What one frame gave: how many differences its codes named, and how many of
/// those became samples. The two differ in the last frame a record needs.
///
/// `used` counts samples, so that adding it up frame by frame says which
/// samples each frame made. That makes the record's first difference count in
/// frame 0 even though its value is skipped: it is where sample 0 stands, and
/// sample 0 is the forward integration constant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub held: usize,
    pub used: usize,
}

/// The Steim part of the walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Steim {
    /// The forward integration constant, which is the first sample, and the
    /// reverse one, which the last sample has to equal.
    pub x0: i32,
    pub xn: i32,
    /// The first difference in the record. It is measured from the last sample
    /// of the record before, so it is read and not used.
    pub first_difference: Option<i32>,
    /// The frames walked, in order, up to the one the last sample came from.
    pub frames: Vec<Frame>,
    /// How many whole frames the payload has room for, walked or not.
    pub frames_in_record: usize,
}

/// How a sample is written out: as a whole number, or as the float it was
/// stored as. A 32-bit float printed through a 64-bit one grows digits it
/// never had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Int,
    F32,
    F64,
}

/// What a record's data turned out to hold, and what it took to get there.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub encoding: u8,
    pub big: bool,
    /// The number of samples the header gives.
    pub declared: usize,
    pub payload_bytes: usize,
    /// Set for Steim1 and Steim2, which are the encodings with steps between
    /// the bytes and the samples.
    pub steim: Option<Steim>,
    pub kind: Kind,
    pub samples: Vec<f64>,
    /// Why the walk stopped short of the declared count, or why nothing was
    /// decoded, where either happened.
    pub problem: Option<String>,
}

/// The check a Steim record carries: its last sample against the reverse
/// integration constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Check {
    pub last: i32,
    pub xn: i32,
}

impl Check {
    pub fn passed(&self) -> bool {
        self.last == self.xn
    }
}

impl Record {
    /// The reverse integration constant check, where there is one to make:
    /// a Steim record whose every declared sample was decoded. A record that
    /// stopped short has no last sample to compare, and says why instead.
    pub fn check(&self) -> Option<Check> {
        let steim = self.steim.as_ref()?;
        if self.declared == 0 || self.samples.len() < self.declared {
            return None;
        }
        Some(Check { last: *self.samples.last()? as i32, xn: steim.xn })
    }

    /// One sample as the panel writes it.
    pub fn text(&self, i: usize) -> String {
        let v = self.samples[i];
        match self.kind {
            // SRO's gain can divide as well as multiply, so a gain-ranged
            // sample is not always whole.
            Kind::Int if v.fract() == 0.0 => format!("{}", v as i64),
            Kind::Int | Kind::F64 => format!("{v}"),
            Kind::F32 => format!("{}", v as f32),
        }
    }
}

/// The rule a gain-ranged encoding is worked out by, as the SEED manual gives
/// it, for the panel to show beside the samples. None for every encoding that
/// is not gain-ranged or has no rule here.
pub fn rule(encoding: u8) -> Option<&'static str> {
    Some(match encoding {
        16 => "each 16-bit word: (low 14 bits - 8191) × 1, 4, 16 or 128, chosen by its top 2 bits",
        30 => "each 16-bit word: (low 12 bits, two's complement) × 2^(10 - top 4 bits)",
        _ => return None,
    })
}

/// Decode a record's data. `payload` is the bytes the template placed as the
/// data, `big` the byte order it laid them out in, and `declared` the sample
/// count from the header.
pub fn decode(payload: &[u8], encoding: u8, big: bool, declared: usize) -> Record {
    if payload.len() > PAYLOAD_LIMIT {
        return refused(payload.len(), encoding, big, declared);
    }
    let mut record = Record {
        encoding,
        big,
        declared,
        payload_bytes: payload.len(),
        steim: None,
        kind: Kind::Int,
        samples: Vec::new(),
        problem: None,
    };
    match encoding {
        10 | 11 => steim(&mut record, payload, encoding == 11),
        1 | 32 => fixed(&mut record, payload, 2, |b| f64::from(int(b, big) as i16)),
        2 => fixed(&mut record, payload, 3, |b| f64::from(sign_extend(int(b, big), 24))),
        3 => fixed(&mut record, payload, 4, |b| f64::from(int(b, big) as i32)),
        4 => {
            record.kind = Kind::F32;
            fixed(&mut record, payload, 4, |b| f64::from(f32::from_bits(int(b, big))))
        }
        5 => {
            record.kind = Kind::F64;
            fixed(&mut record, payload, 8, |b| {
                let raw: [u8; 8] = b.try_into().expect("eight bytes");
                if big { f64::from_be_bytes(raw) } else { f64::from_le_bytes(raw) }
            })
        }
        16 => fixed(&mut record, payload, 2, |b| {
            let w = int(b, big);
            // Offset binary, and a gain of 2 to the 0, 2, 4 or 7.
            let mantissa = (w & 0x3fff) as i32 - 8191;
            f64::from(mantissa << [0, 2, 4, 7][(w >> 14) as usize & 3])
        }),
        30 => fixed(&mut record, payload, 2, |b| {
            let w = int(b, big);
            let mantissa = sign_extend(w & 0x0fff, 12);
            let exponent = 10 - (w >> 12) as i32;
            f64::from(mantissa) * 2f64.powi(exponent)
        }),
        _ => {
            record.problem = Some(format!(
                "Not decoded: there is no documented rule for turning {} words into samples.",
                encoding_name(encoding)
            ));
        }
    }
    record
}

/// The answer for data over [`PAYLOAD_LIMIT`], which says so without the bytes
/// having been read: a caller that knows the length can ask for this instead
/// of reading a gigabyte to be told no.
pub fn refused(payload_bytes: usize, encoding: u8, big: bool, declared: usize) -> Record {
    let mb = PAYLOAD_LIMIT / (1 << 20);
    Record {
        encoding,
        big,
        declared,
        payload_bytes,
        steim: None,
        kind: Kind::Int,
        samples: Vec::new(),
        problem: Some(format!("Not decoded: the data is over the {mb} MB limit for decoding.")),
    }
}

/// What an encoding is called, as the 2.4 template names it. miniSEED 3 kept
/// the numbers, and the names of every encoding this decodes are the same in
/// both.
pub fn encoding_name(encoding: u8) -> String {
    super::mseed::encoding_name(encoding).map_or_else(|| format!("unknown encoding {encoding}"), str::to_string)
}

/// A count as people read one: 5980 as `5,980`.
fn commas(n: usize) -> String {
    crate::encode::commas(n as u64)
}

/// A run of samples each `width` bytes wide, as many as the header gave and
/// the payload has room for.
fn fixed(record: &mut Record, payload: &[u8], width: usize, read: impl Fn(&[u8]) -> f64) {
    let room = payload.len() / width;
    let n = record.declared.min(room);
    record.samples = payload.chunks_exact(width).take(n).map(read).collect();
    if room < record.declared {
        record.problem = Some(format!(
            "The data has room for only {} samples; the header says {}.",
            commas(room),
            commas(record.declared)
        ));
    }
}

/// Up to four bytes as an unsigned number in the given order.
fn int(bytes: &[u8], big: bool) -> u32 {
    let fold = |acc: u32, &b: &u8| acc << 8 | u32::from(b);
    if big { bytes.iter().fold(0, fold) } else { bytes.iter().rev().fold(0, fold) }
}

/// The low `bits` bits of `v` as a two's complement number.
fn sign_extend(v: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((v << shift) as i32) >> shift
}

/// Steim1 or Steim2: frame by frame, word by word, difference by difference,
/// until the header's count of samples is made.
fn steim(record: &mut Record, payload: &[u8], two: bool) {
    let frames_in_record = payload.len() / FRAME;
    if frames_in_record == 0 {
        if record.declared > 0 {
            record.problem = Some(format!(
                "Not decoded: only {} bytes of data, less than one {FRAME}-byte frame.",
                payload.len()
            ));
        }
        return;
    }
    let word = |f: usize, w: usize| -> [u8; 4] { payload[f * FRAME + w * 4..f * FRAME + w * 4 + 4].try_into().expect("four") };
    let x0 = int(&word(0, 1), record.big) as i32;
    let xn = int(&word(0, 2), record.big) as i32;
    let mut s = Steim { x0, xn, first_difference: None, frames: Vec::new(), frames_in_record };
    // Differences seen so far, the skipped first one included.
    let mut seen = 0usize;
    let mut last = x0;
    let mut diffs = Vec::with_capacity(7);
    'frames: for f in 0..frames_in_record {
        if seen >= record.declared {
            break;
        }
        let nibbles = int(&word(f, 0), record.big);
        let mut frame = Frame { held: 0, used: 0 };
        // Frame 0 spends words 1 and 2 on the integration constants.
        let first = if f == 0 { 3 } else { 1 };
        for w in first..16 {
            let code = nibbles >> (30 - 2 * w) & 3;
            diffs.clear();
            if let Err(sub) = differences(word(f, w), code, two, record.big, &mut diffs) {
                // Words past the last sample are not the record's business:
                // some writers leave whatever was in the buffer there.
                if seen < record.declared {
                    record.problem = Some(format!(
                        "Decoding stopped at frame {f}, word {w}: Steim2 code {code} with sub-code {sub} is not defined. {} of {} samples decoded.",
                        commas(record.samples.len()),
                        commas(record.declared)
                    ));
                    s.frames.push(frame);
                    break 'frames;
                }
                break;
            }
            for &d in &diffs {
                frame.held += 1;
                if seen < record.declared {
                    if seen == 0 {
                        s.first_difference = Some(d);
                        record.samples.push(f64::from(x0));
                    } else {
                        last = last.wrapping_add(d);
                        record.samples.push(f64::from(last));
                    }
                    frame.used += 1;
                }
                seen += 1;
            }
        }
        s.frames.push(frame);
    }
    if record.problem.is_none() && record.samples.len() < record.declared {
        record.problem = Some(format!(
            "The frames ran out after {} samples; the header says {}.",
            commas(record.samples.len()),
            commas(record.declared)
        ));
    }
    record.steim = Some(s);
}

/// The differences one data word holds, by its code, pushed onto `out`. The
/// error is the Steim2 sub-code for a word whose code and sub-code together
/// name nothing: code 2 with sub-code 0, or code 3 with sub-code 3.
pub fn differences(word: [u8; 4], code: u32, two: bool, big: bool, out: &mut Vec<i32>) -> Result<(), u32> {
    let whole = int(&word, big);
    match (two, code) {
        (_, 0) => {}
        // Four 8-bit differences are four bytes, in the order they lie in
        // either byte order.
        (_, 1) => out.extend(word.iter().map(|&b| i32::from(b as i8))),
        (false, 2) => {
            out.push(i32::from(int(&word[..2], big) as i16));
            out.push(i32::from(int(&word[2..], big) as i16));
        }
        (false, _) => out.push(whole as i32),
        (true, 2) => {
            let (bits, n) = match whole >> 30 {
                1 => (30, 1),
                2 => (15, 2),
                3 => (10, 3),
                sub => return Err(sub),
            };
            unpack(whole, bits, n, out);
        }
        (true, _) => {
            let (bits, n) = match whole >> 30 {
                0 => (6, 5),
                1 => (5, 6),
                2 => (4, 7),
                sub => return Err(sub),
            };
            unpack(whole, bits, n, out);
        }
    }
    Ok(())
}

/// `n` signed differences of `bits` each from the bottom of a word, first
/// difference highest. What the differences leave of the thirty bits sits
/// above them, which only the seven 4-bit case has any of.
fn unpack(word: u32, bits: u32, n: u32, out: &mut Vec<i32>) {
    for i in 0..n {
        let shift = (n - 1 - i) * bits;
        out.push(sign_extend(word >> shift & ((1 << bits) - 1), bits));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::Evaluator;
    use crate::formats::mseed::mseed;
    use crate::source::MemSource;

    /// One data word as a test states it: a Steim code, and for Steim2's bit
    /// packed words a sub-code, the width of each difference and the
    /// differences.
    #[derive(Clone)]
    struct Word {
        code: u32,
        sub: u32,
        bits: u32,
        values: Vec<i32>,
    }

    fn w8(values: [i32; 4]) -> Word {
        Word { code: 1, sub: 0, bits: 8, values: values.to_vec() }
    }

    fn packed(code: u32, sub: u32, bits: u32, values: &[i32]) -> Word {
        Word { code, sub, bits, values: values.to_vec() }
    }

    /// The four bytes of a word in the given order, following the rules the
    /// module notes give for a little-endian record.
    fn word_bytes(w: &Word, two: bool, big: bool) -> [u8; 4] {
        let order = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        match (two, w.code) {
            (_, 1) => w.values.iter().map(|&v| v as u8).collect::<Vec<_>>().try_into().unwrap(),
            (false, 2) => {
                let half = |v: i32| if big { (v as i16).to_be_bytes() } else { (v as i16).to_le_bytes() };
                let (a, b) = (half(w.values[0]), half(w.values[1]));
                [a[0], a[1], b[0], b[1]]
            }
            (false, 3) => order(w.values[0] as u32),
            (true, _) => {
                let n = w.values.len() as u32;
                let mask = (1u32 << w.bits) - 1;
                let mut v = w.sub << 30;
                for (i, &d) in w.values.iter().enumerate() {
                    v |= (d as u32 & mask) << ((n - 1 - i as u32) * w.bits);
                }
                order(v)
            }
            _ => [0; 4],
        }
    }

    /// Frames holding `words` after the two constants, sixteen words a frame,
    /// with empty words filling the last one out.
    fn frames(words: &[Word], x0: i32, xn: i32, two: bool, big: bool) -> Vec<u8> {
        let order = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut out = Vec::new();
        let mut rest = words.iter().peekable();
        let mut first = true;
        while first || rest.peek().is_some() {
            let mut nibbles = 0u32;
            let mut body = Vec::new();
            if first {
                body.extend_from_slice(&order(x0 as u32));
                body.extend_from_slice(&order(xn as u32));
            }
            for n in if first { 3 } else { 1 }..16 {
                match rest.next() {
                    Some(w) => {
                        nibbles |= w.code << (30 - 2 * n);
                        body.extend_from_slice(&word_bytes(w, two, big));
                    }
                    None => body.extend_from_slice(&[0; 4]),
                }
            }
            out.extend_from_slice(&order(nibbles));
            out.extend_from_slice(&body);
            first = false;
        }
        out
    }

    /// A 2.4 record around `data`: the fixed header, blockette 1000, and the
    /// data from byte 64, sized to the next power of two that holds it.
    fn record(data: &[u8], encoding: u8, count: u16, big: bool) -> Vec<u8> {
        let u16b = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let exponent = (64 + data.len()).next_power_of_two().trailing_zeros().max(8) as u8;
        let mut v = b"000001D TEST   BHZXX".to_vec();
        v.extend_from_slice(&u16b(2004));
        v.extend_from_slice(&u16b(350));
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(&u16b(0));
        v.extend_from_slice(&u16b(count));
        v.extend_from_slice(&u16b(1));
        v.extend_from_slice(&u16b(1));
        v.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0]);
        v.extend_from_slice(&u16b(64));
        v.extend_from_slice(&u16b(48));
        v.extend_from_slice(&u16b(1000));
        v.extend_from_slice(&u16b(0));
        v.extend_from_slice(&[encoding, u8::from(big), exponent, 0]);
        v.resize(64, 0);
        v.extend_from_slice(data);
        v.resize(1 << exponent, 0);
        v
    }

    /// The differences the template reads out of the first frame of a record
    /// built by [`record`], in order: every leaf named `d0` to `d6`.
    fn template_differences(file: Vec<u8>) -> Vec<i32> {
        const DATA: usize = 20;
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(mseed());
        let mut out = Vec::new();
        fn walk(ev: &mut Evaluator, d: &Document<MemSource>, path: &mut Vec<usize>, out: &mut Vec<i32>) {
            let node = ev.node(d, path).unwrap();
            if node.child_count == 0 {
                if node.name.len() == 2 && node.name.starts_with('d') {
                    out.push(node.value.as_int().unwrap() as i32);
                }
                return;
            }
            for i in 0..node.child_count as usize {
                path.push(i);
                walk(ev, d, path, out);
                path.pop();
            }
        }
        walk(&mut ev, &d, &mut vec![0, 0, DATA, 0], &mut out);
        out
    }

    /// Every difference one hand-built frame's words hold, through the byte
    /// reader, in the order written.
    fn reader_differences(data: &[u8], two: bool, big: bool) -> Vec<i32> {
        let nibbles = int(&data[..4], big);
        let mut out = Vec::new();
        for w in 3..16 {
            let code = nibbles >> (30 - 2 * w) & 3;
            differences(data[w * 4..w * 4 + 4].try_into().unwrap(), code, two, big, &mut out).unwrap();
        }
        out
    }

    /// Samples from differences the way the format means: the first sample is
    /// x0, the first difference is skipped, and each sample after is the one
    /// before plus its difference.
    fn integrate(x0: i32, diffs: &[i32], count: usize) -> Vec<f64> {
        let mut out = vec![f64::from(x0)];
        let mut last = x0;
        for &d in diffs.iter().skip(1).take(count.saturating_sub(1)) {
            last += d;
            out.push(f64::from(last));
        }
        out
    }

    /// Every Steim2 word form, each with a negative difference at the bottom
    /// of its range so a sign taken from the wrong bit shows, in both byte
    /// orders. The same frame goes through the template, and the two have to
    /// agree difference for difference before the samples are compared.
    #[test]
    fn every_steim2_word_form_reads_the_same_here_as_in_the_template() {
        let words = vec![
            w8([0, -128, 127, -1]),
            packed(2, 1, 30, &[-(1 << 29)]),
            packed(2, 2, 15, &[-16384, 16383]),
            packed(2, 3, 10, &[-512, 511, -3]),
            packed(3, 0, 6, &[-32, 31, -1, 5, -7]),
            packed(3, 1, 5, &[-16, 15, -2, 3, 0, -1]),
            packed(3, 2, 4, &[-8, 7, -1, 1, 2, -3, 4]),
        ];
        let diffs: Vec<i32> = words.iter().flat_map(|w| w.values.clone()).collect();
        let x0 = 100;
        let count = diffs.len();
        let want = integrate(x0, &diffs, count);
        let xn = *want.last().unwrap() as i32;
        for big in [true, false] {
            let data = frames(&words, x0, xn, true, big);
            assert_eq!(reader_differences(&data, true, big), diffs, "big={big}: reader");
            assert_eq!(template_differences(record(&data, 11, count as u16, big)), diffs, "big={big}: template");
            let r = decode(&data, 11, big, count);
            assert_eq!(r.problem, None, "big={big}");
            assert_eq!(r.samples, want, "big={big}");
            let steim = r.steim.as_ref().unwrap();
            assert_eq!((steim.x0, steim.xn, steim.first_difference), (x0, xn, Some(0)));
            assert_eq!(steim.frames, vec![Frame { held: count, used: count }]);
            assert!(r.check().unwrap().passed(), "big={big}");
        }
    }

    /// Steim1's three data word forms, the same way.
    #[test]
    fn every_steim1_word_form_reads_the_same_here_as_in_the_template() {
        let words = vec![
            w8([5, -128, 127, -1]),
            Word { code: 2, sub: 0, bits: 16, values: vec![-32768, 32767] },
            Word { code: 3, sub: 0, bits: 32, values: vec![-100_000] },
        ];
        let diffs: Vec<i32> = words.iter().flat_map(|w| w.values.clone()).collect();
        let want = integrate(-7, &diffs, diffs.len());
        let xn = *want.last().unwrap() as i32;
        for big in [true, false] {
            let data = frames(&words, -7, xn, false, big);
            assert_eq!(reader_differences(&data, false, big), diffs, "big={big}: reader");
            assert_eq!(template_differences(record(&data, 10, diffs.len() as u16, big)), diffs, "big={big}: template");
            let r = decode(&data, 10, big, diffs.len());
            assert_eq!(r.samples, want, "big={big}");
            assert_eq!(r.steim.unwrap().first_difference, Some(5));
        }
    }

    /// A frame holds seven differences in a word whether or not seven are
    /// needed, and the header's count is where the samples stop. The frame
    /// still says how many it held.
    #[test]
    fn the_samples_stop_at_the_count_the_header_gives() {
        let words = vec![packed(3, 2, 4, &[0, 1, 1, 1, 1, 1, 1]), packed(3, 2, 4, &[1, 1, 1, 1, 1, 1, 1])];
        let data = frames(&words, 1, 10, true, true);
        let r = decode(&data, 11, true, 10);
        assert_eq!(r.samples, (1..=10).map(f64::from).collect::<Vec<_>>());
        assert_eq!(r.steim.as_ref().unwrap().frames, vec![Frame { held: 14, used: 10 }]);
        assert_eq!(r.problem, None);
        assert!(r.check().unwrap().passed());
    }

    /// A reverse constant that does not match the last sample is a failed
    /// check with both numbers, not a problem that hides the samples.
    #[test]
    fn a_last_sample_that_misses_the_reverse_constant_fails_the_check_with_both_numbers() {
        let data = frames(&[w8([0, 1, 1, 1])], 1, 99, true, true);
        let r = decode(&data, 11, true, 4);
        assert_eq!(r.samples, vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(r.problem, None);
        let check = r.check().unwrap();
        assert_eq!((check.last, check.xn, check.passed()), (4, 99, false));
    }

    /// Steim2 code 2 with sub-code 0 names nothing. The walk stops there, says
    /// where, and keeps the samples it had made.
    #[test]
    fn an_undefined_steim2_sub_code_stops_the_walk_and_says_where() {
        let words = vec![w8([0, 1, 1, 1]), packed(2, 0, 30, &[5])];
        let data = frames(&words, 1, 9, true, true);
        let r = decode(&data, 11, true, 6);
        assert_eq!(r.samples, vec![1.0, 2.0, 3.0, 4.0]);
        let problem = r.problem.clone().unwrap();
        assert!(problem.contains("frame 0, word 4"), "{problem}");
        assert_eq!(r.check(), None);
    }

    /// A record that runs out of frames says how far it got.
    #[test]
    fn frames_that_run_out_before_the_count_say_so() {
        let data = frames(&[w8([0, 1, 1, 1])], 1, 9, true, true);
        let r = decode(&data, 11, true, 9);
        assert_eq!(r.samples.len(), 4);
        assert!(r.problem.unwrap().contains("after 4 samples; the header says 9"));
        // And a payload with no frame at all.
        let r = decode(&[0; 10], 11, true, 3);
        assert!(r.steim.is_none() && r.samples.is_empty());
        assert!(r.problem.is_some());
    }

    /// The fixed-width encodings, in both byte orders.
    #[test]
    fn a_fixed_width_run_is_read_at_its_width_and_byte_order() {
        for big in [true, false] {
            let bytes = |v: &[u8]| if big { v.to_vec() } else { v.iter().rev().copied().collect() };
            let r = decode(&[bytes(&(-2i16).to_be_bytes()), bytes(&7i16.to_be_bytes())].concat(), 1, big, 2);
            assert_eq!(r.samples, vec![-2.0, 7.0]);
            let r = decode(&bytes(&[0xff, 0xff, 0xfe]), 2, big, 1);
            assert_eq!(r.samples, vec![-2.0]);
            let r = decode(&bytes(&1.5f32.to_be_bytes()), 4, big, 1);
            assert_eq!((r.samples[0], r.text(0)), (1.5, "1.5".to_string()));
            let r = decode(&bytes(&0.1f64.to_be_bytes()), 5, big, 1);
            assert_eq!(r.samples, vec![0.1]);
        }
        // Less room than the header claims.
        let r = decode(&[0, 1, 0, 2, 0], 1, true, 5);
        assert_eq!(r.samples, vec![1.0, 2.0]);
        assert!(r.problem.unwrap().contains("room for only 2 samples; the header says 5"));
    }

    /// The two gain-ranged rules, from the SEED manual's own descriptions.
    #[test]
    fn the_gain_ranged_rules_are_the_ones_the_seed_manual_gives() {
        // CDSN: gain code 2 multiplies by 16, and 8191 is zero.
        let w: u16 = (2 << 14) | 8200;
        assert_eq!(decode(&w.to_be_bytes(), 16, true, 1).samples, vec![9.0 * 16.0]);
        // SRO: a mantissa of -3 at gain code 12 is -3 × 2^-2.
        let w: u16 = (12 << 12) | 0x0ffd;
        assert_eq!(decode(&w.to_le_bytes(), 30, false, 1).samples, vec![-0.75]);
        // GEOSCOPE has no rule here, and says so rather than guessing.
        let r = decode(&[0; 4], 14, true, 2);
        assert!(r.samples.is_empty() && r.problem.is_some());
    }

    #[test]
    fn a_packing_name_carries_the_encoding_and_the_byte_order() {
        assert_eq!(parse_packing(&packing(11, Endian::Big)), Some((11, true)));
        assert_eq!(parse_packing(&packing(4, Endian::Little)), Some((4, false)));
        assert_eq!(parse_packing("hdf5_chunk"), None);
    }
}
