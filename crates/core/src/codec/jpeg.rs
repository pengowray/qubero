//! Baseline JPEG: one scan's entropy-coded data, read into the quantized
//! coefficients of its 8×8 blocks, with every code written down.
//!
//! A scan is Huffman codes and the bits after them, packed from the high bit
//! of each byte down, and nothing else. Which codes mean what, how big the
//! picture is, how its channels are sampled and how often the coder starts
//! again are all segments the file wrote before the scan, so a scan read on
//! its own is bits with no meaning. [`Header::read`] takes those segments, as
//! the bytes the file holds them in, and [`scan`] reads the run with them.
//!
//! **What comes out** is the coefficients, not the picture: 64 `i16` per 8×8
//! block, little-endian, in rows (T.81's natural order, not the zigzag the
//! scan writes them in), and the blocks one after another in the order the
//! scan codes them. That is what each block stores, before any table scales
//! it or any transform turns it into samples, and a reader who wants to see
//! what a code did wants it. Dequantizing and the inverse DCT are left out on
//! purpose: they need the quantization tables, which are settings of a
//! different stage, and the samples they give are what every image viewer
//! already shows.
//!
//! **The trace** has three levels. A block of the trace is one MCU, the
//! minimum coded unit, and its [`Unit`](super::Unit)s are the 8×8 blocks in
//! it; a step is one code and its value bits. A DC step says the difference
//! and the coefficient it came to, an AC step says the run of zeros before it,
//! its value and where it lands in zigzag order, and ZRL and EOB say which
//! zeros they stand for. The padding before a restart marker and the marker
//! itself are steps of the MCU they follow.
//!
//! **The output a step makes.** Coefficients are written in zigzag order and
//! a block is laid out in rows, so what one code produced is scattered over
//! the block's 128 bytes, and a step's output has to be one run. So a code's
//! output is empty and the step that finishes a block, its EOB or the
//! coefficient at position 63, carries the whole block. Asking which step
//! made a byte of the coefficients lands on the step that closed its block,
//! not on the code that set that coefficient; the block is the finest thing
//! that is true of.
//!
//! **Byte stuffing.** A scan writes every `ff` of its data as `ff 00`, since
//! `ff` and a byte that is not zero is a marker. The zero is not data: the run
//! is cut at its restart markers and the zeros taken out before a bit is read,
//! and every bit position is mapped back to where it is in the file. A zero
//! belongs to the step that read the last bit of the `ff` before it, which is
//! the step reading when the decoder has to step over it, so a code can
//! straddle one and every bit of the run is still in exactly one step.
//! [`Trace::stuffed`](super::Trace::stuffed) says where they are.
//!
//! **What is refused.** Progressive, arithmetic-coded, lossless and
//! hierarchical frames, and 12-bit samples, each with its own
//! [`Unsupported`]: each is a different bit stream, and reading one with these
//! rules gives numbers that look like coefficients. SOF1, extended sequential,
//! is read when its samples are 8-bit, since at 8 bits it differs from
//! baseline only in allowing four tables of each kind rather than two. A
//! table the scan names that no segment before it defined is
//! [`Refusal::Settings`], which is what a Motion JPEG frame relying on the
//! tables in the standard's annex comes to.

use std::ops::Range;

use super::{BlockKind, Refusal, StepField, StepKind, Trace, TraceBuilder, Unsupported, CAP_BYTES};

/// Where each coefficient of the zigzag order goes in a block laid out in
/// rows: `ZIGZAG[k]` is the natural index of zigzag position `k`.
pub const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21,
    28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61,
    54, 47, 55, 62, 63,
];

/// How many bytes one 8×8 block comes to: 64 coefficients of two bytes.
pub const BLOCK_BYTES: usize = 128;

/// One channel of the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    /// The number the scan headers name it by.
    pub id: u8,
    /// How many blocks of it go across and down one MCU of an interleaved
    /// scan.
    pub h: u8,
    pub v: u8,
    /// Which quantization table it is scaled by.
    pub quant: u8,
    /// How many 8×8 blocks of it cover the picture, across and down, counting
    /// a block that is only partly inside it.
    pub blocks_across: u32,
    pub blocks_down: u32,
}

/// One channel of a scan and the two tables it is read with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanComponent {
    /// Which of the frame's components, by place rather than by id.
    pub index: u8,
    pub dc_table: u8,
    pub ac_table: u8,
}

/// What the channels are, worked out the way libjpeg works it out: from the
/// JFIF and Adobe segments where the file has them, and from how many
/// channels there are and what they are called where it does not. Only the
/// names of the channels depend on it; the coefficients do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorModel {
    Grey,
    YCbCr,
    Rgb,
    Cmyk,
    Ycck,
    /// Two channels, or more than four: the file says nothing about what
    /// they are.
    Unknown,
}

/// What the decoder settled before it read a bit of the scan, kept on the
/// trace for whatever wants to lay the blocks out again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanFacts {
    pub width: u16,
    pub height: u16,
    /// Every channel of the frame, including the ones this scan does not
    /// carry.
    pub components: Vec<Component>,
    /// The channels this scan carries, in the order it codes them.
    pub scan: Vec<ScanComponent>,
    /// How many MCUs the scan codes, across and down. A scan of one channel
    /// codes one block per MCU, so these are that channel's blocks.
    pub mcus_across: u32,
    pub mcus_down: u32,
    /// How many MCUs come between restart markers. Zero is none.
    pub restart_interval: u16,
    pub color: ColorModel,
}

impl ScanFacts {
    /// What a channel is called: `Y`, `Cb`, `Cr`, or the letters of the other
    /// models, and the channel's id where the model has no name for it.
    pub fn channel_name(&self, index: usize) -> String {
        let names: &[&str] = match self.color {
            ColorModel::Grey => &["Y"],
            ColorModel::YCbCr => &["Y", "Cb", "Cr"],
            ColorModel::Rgb => &["R", "G", "B"],
            ColorModel::Cmyk => &["C", "M", "Y", "K"],
            ColorModel::Ycck => &["Y", "Cb", "Cr", "K"],
            ColorModel::Unknown => &[],
        };
        match names.get(index) {
            Some(name) => name.to_string(),
            None => format!("component {}", self.components.get(index).map_or(index as u8, |c| c.id)),
        }
    }

    /// How many 8×8 blocks one MCU holds.
    pub fn blocks_per_mcu(&self) -> u32 {
        match self.scan.as_slice() {
            [_] => 1,
            many => many.iter().map(|s| u32::from(self.components[s.index as usize].h) * u32::from(self.components[s.index as usize].v)).sum(),
        }
    }
}

/// The frame, the tables and the scan header, read out of the segments
/// before the scan.
pub struct Header {
    pub facts: ScanFacts,
    dc: Vec<Huffman>,
    ac: Vec<Huffman>,
}

/// One Huffman table, built the way Annex C builds it, with a lookup for the
/// codes nine bits long or shorter.
#[derive(Debug, Clone)]
struct Huffman {
    /// Indexed by the next nine bits: the code's length in the high byte and
    /// its symbol in the low one, or zero where no code that short matches.
    fast: Vec<u16>,
    /// For each length, the largest code of that length, or -1 for none.
    maxcode: [i32; 17],
    /// For each length, the smallest code of that length and where its
    /// symbol is in `values`.
    mincode: [i32; 17],
    valptr: [i32; 17],
    values: Vec<u8>,
}

impl Huffman {
    /// A table from its sixteen counts and its symbols, as DHT writes them.
    /// Refused when the counts give a code longer than its length allows,
    /// which is a table no encoder wrote.
    fn build(counts: &[u8; 16], values: &[u8]) -> Result<Huffman, Refusal> {
        let mut fast = vec![0u16; 512];
        let mut maxcode = [-1i32; 17];
        let mut mincode = [0i32; 17];
        let mut valptr = [0i32; 17];
        let mut code = 0i32;
        let mut k = 0usize;
        for len in 1..=16usize {
            let n = counts[len - 1] as usize;
            if n > 0 {
                valptr[len] = k as i32;
                mincode[len] = code;
                for _ in 0..n {
                    let sym = *values.get(k).ok_or(Refusal::Failed)?;
                    if len <= 9 {
                        let from = (code as usize) << (9 - len);
                        for e in &mut fast[from..from + (1 << (9 - len))] {
                            *e = (len as u16) << 8 | sym as u16;
                        }
                    }
                    code += 1;
                    k += 1;
                }
                maxcode[len] = code - 1;
            }
            // A code of all ones is not allowed, so the codes of each length
            // have to leave room for one more.
            if code >= 1 << len {
                return Err(Refusal::Failed);
            }
            code <<= 1;
        }
        Ok(Huffman { fast, maxcode, mincode, valptr, values: values.to_vec() })
    }

    /// The next code: its length in bits and its symbol.
    fn decode(&self, r: &mut Reader) -> Result<(u8, u8), Refusal> {
        let bits = r.peek16();
        let e = self.fast[(bits >> 7) as usize];
        let (len, sym) = if e != 0 {
            ((e >> 8) as u8, e as u8)
        } else {
            let mut found = None;
            for len in 10..=16usize {
                let code = (bits >> (16 - len)) as i32;
                if code <= self.maxcode[len] {
                    let at = self.valptr[len] + code - self.mincode[len];
                    found = Some((len as u8, *self.values.get(at as usize).ok_or(Refusal::Failed)?));
                    break;
                }
            }
            found.ok_or(Refusal::Failed)?
        };
        r.skip(len as u32)?;
        Ok((len, sym))
    }
}

/// A place in one restart interval's data, with the stuffing already out.
struct Reader<'a> {
    data: &'a [u8],
    pos: u64,
}

impl Reader<'_> {
    /// The next sixteen bits, with ones past the end: T.81 pads the last byte
    /// with ones, and a code is looked up by more bits than it takes.
    fn peek16(&self) -> u32 {
        let at = (self.pos / 8) as usize;
        let byte = |i: usize| *self.data.get(at + i).unwrap_or(&0xff) as u32;
        let acc = byte(0) << 16 | byte(1) << 8 | byte(2);
        (acc >> (8 - self.pos % 8)) & 0xffff
    }

    fn skip(&mut self, n: u32) -> Result<(), Refusal> {
        if self.pos + n as u64 > self.data.len() as u64 * 8 {
            return Err(Refusal::Failed);
        }
        self.pos += n as u64;
        Ok(())
    }

    /// `n` bits as a number, sixteen at most.
    fn take(&mut self, n: u32) -> Result<u32, Refusal> {
        if n == 0 {
            return Ok(0);
        }
        let v = self.peek16() >> (16 - n);
        self.skip(n)?;
        Ok(v)
    }
}

/// A value of `size` bits as T.81's EXTEND reads it: the lower half of the
/// range is the negative numbers.
fn extend(v: u32, size: u8) -> i32 {
    if size == 0 {
        0
    } else if v < 1 << (size - 1) {
        v as i32 - (1 << size) + 1
    } else {
        v as i32
    }
}

/// Which frame markers are which kind of JPEG, for the refusal.
fn process(marker: u8) -> Option<Unsupported> {
    match marker {
        0xc0 | 0xc1 => None,
        0xc2 => Some(Unsupported::Progressive),
        0xc3 => Some(Unsupported::Lossless),
        0xc5..=0xc7 => Some(Unsupported::Hierarchical),
        _ => Some(Unsupported::Arithmetic),
    }
}

fn is_frame(marker: u8) -> bool {
    matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf)
}

fn u16_at(b: &[u8], at: usize) -> Result<u16, Refusal> {
    Ok(u16::from_be_bytes([*b.get(at).ok_or(Refusal::Failed)?, *b.get(at + 1).ok_or(Refusal::Failed)?]))
}

impl Header {
    /// Read the segments before a scan: the bytes from just after the
    /// start-of-image marker to the end of the scan's own header, which is
    /// the last thing in them.
    ///
    /// A table defined twice means the later one, which is the one in force
    /// when the scan starts. An earlier scan's data is passed over the way
    /// the template passes over it, to the next marker that is neither a
    /// stuffed `ff` nor a restart.
    pub fn read(bytes: &[u8]) -> Result<Header, Refusal> {
        let mut frame: Option<(u8, u8, u16, u16, Vec<(u8, u8, u8, u8)>)> = None;
        let mut hierarchical = false;
        let mut dc: [Option<Huffman>; 4] = Default::default();
        let mut ac: [Option<Huffman>; 4] = Default::default();
        let mut restart = 0u16;
        let (mut jfif, mut adobe) = (false, None);
        let mut scan: Option<(Vec<(u8, u8, u8)>, [u8; 4])> = None;
        let mut at = 0usize;
        while at < bytes.len() {
            if bytes[at] != 0xff {
                return Err(Refusal::Failed);
            }
            // Any number of `ff` may pad the space before a marker.
            while bytes.get(at + 1) == Some(&0xff) {
                at += 1;
            }
            let Some(&marker) = bytes.get(at + 1) else { return Err(Refusal::Failed) };
            if matches!(marker, 0xd0..=0xd9 | 0x01) {
                at += 2;
                continue;
            }
            let len = u16_at(bytes, at + 2)? as usize;
            if len < 2 || at + 2 + len > bytes.len() {
                return Err(Refusal::Failed);
            }
            let body = &bytes[at + 4..at + 2 + len];
            let end = at + 2 + len;
            match marker {
                m if is_frame(m) => {
                    let (&p, n) = (body.first().ok_or(Refusal::Failed)?, *body.get(5).ok_or(Refusal::Failed)? as usize);
                    let mut comps = Vec::with_capacity(n);
                    for i in 0..n {
                        let c = body.get(6 + 3 * i..9 + 3 * i).ok_or(Refusal::Failed)?;
                        comps.push((c[0], c[1] >> 4, c[1] & 15, c[2]));
                    }
                    frame = Some((m, p, u16_at(body, 1)?, u16_at(body, 3)?, comps));
                }
                0xde => hierarchical = true,
                0xc4 => {
                    let mut i = 0;
                    while i < body.len() {
                        let (class, id) = (body[i] >> 4, body[i] & 15);
                        let counts: [u8; 16] = body.get(i + 1..i + 17).ok_or(Refusal::Failed)?.try_into().expect("sixteen");
                        let total: usize = counts.iter().map(|&c| c as usize).sum();
                        let values = body.get(i + 17..i + 17 + total).ok_or(Refusal::Failed)?;
                        if id > 3 || class > 1 || total > 256 {
                            return Err(Refusal::Failed);
                        }
                        let table = Huffman::build(&counts, values)?;
                        if class == 0 { dc[id as usize] = Some(table) } else { ac[id as usize] = Some(table) }
                        i += 17 + total;
                    }
                }
                0xdd => restart = u16_at(body, 0)?,
                0xe0 => jfif |= body.starts_with(b"JFIF\0"),
                0xee if body.starts_with(b"Adobe") && body.len() >= 12 => adobe = Some(body[11]),
                0xda => {
                    let n = *body.first().ok_or(Refusal::Failed)? as usize;
                    let mut comps = Vec::with_capacity(n);
                    for i in 0..n {
                        let c = body.get(1 + 2 * i..3 + 2 * i).ok_or(Refusal::Failed)?;
                        comps.push((c[0], c[1] >> 4, c[1] & 15));
                    }
                    let tail = body.get(1 + 2 * n..4 + 2 * n).ok_or(Refusal::Failed)?;
                    let spectral = [tail[0], tail[1], tail[2] >> 4, tail[2] & 15];
                    if end == bytes.len() {
                        scan = Some((comps, spectral));
                        break;
                    }
                    // An earlier scan: step over its data to the next marker
                    // that ends it.
                    let mut e = end;
                    while e < bytes.len() {
                        if bytes[e] == 0xff && !matches!(bytes.get(e + 1), Some(0x00 | 0xd0..=0xd7)) {
                            break;
                        }
                        e += 1;
                    }
                    at = e;
                    continue;
                }
                _ => {}
            }
            at = end;
        }
        let Some((marker, precision, height, width, frame)) = frame else { return Err(Refusal::Settings) };
        if hierarchical {
            return Err(Refusal::Unsupported(Unsupported::Hierarchical));
        }
        if let Some(kind) = process(marker) {
            return Err(Refusal::Unsupported(kind));
        }
        match precision {
            8 => {}
            12 => return Err(Refusal::Unsupported(Unsupported::Precision12)),
            _ => return Err(Refusal::Failed),
        }
        let Some((scan, spectral)) = scan else { return Err(Refusal::Failed) };
        // A sequential scan carries every coefficient at full precision.
        if spectral != [0, 63, 0, 0] {
            return Err(Refusal::Failed);
        }
        // A height of zero is a promise that a DNL segment after the first
        // scan will say it, and nothing before the scan can.
        if width == 0 || height == 0 || frame.is_empty() || scan.is_empty() || scan.len() > 4 {
            return Err(Refusal::Failed);
        }
        if frame.iter().any(|&(_, h, v, _)| !(1..=4).contains(&h) || !(1..=4).contains(&v)) {
            return Err(Refusal::Failed);
        }
        let hmax = frame.iter().map(|c| c.1 as u32).max().expect("not empty");
        let vmax = frame.iter().map(|c| c.2 as u32).max().expect("not empty");
        let components: Vec<Component> = frame
            .iter()
            .map(|&(id, h, v, quant)| {
                let across = (width as u32 * h as u32).div_ceil(hmax);
                let down = (height as u32 * v as u32).div_ceil(vmax);
                Component { id, h, v, quant, blocks_across: across.div_ceil(8), blocks_down: down.div_ceil(8) }
            })
            .collect();
        let mut members = Vec::with_capacity(scan.len());
        for &(id, dc_table, ac_table) in &scan {
            let index = components.iter().position(|c| c.id == id).ok_or(Refusal::Failed)?;
            if members.iter().any(|m: &ScanComponent| m.index as usize == index) {
                return Err(Refusal::Failed);
            }
            // A table named and never defined: the file does not say how the
            // scan was coded.
            if dc_table > 3 || ac_table > 3 || dc[dc_table as usize].is_none() || ac[ac_table as usize].is_none() {
                return Err(Refusal::Settings);
            }
            members.push(ScanComponent { index: index as u8, dc_table, ac_table });
        }
        let (mcus_across, mcus_down) = match members.as_slice() {
            [one] => {
                let c = &components[one.index as usize];
                (c.blocks_across, c.blocks_down)
            }
            _ => ((width as u32).div_ceil(8 * hmax), (height as u32).div_ceil(8 * vmax)),
        };
        let ids: Vec<u8> = components.iter().map(|c| c.id).collect();
        let color = match components.len() {
            1 => ColorModel::Grey,
            3 if jfif => ColorModel::YCbCr,
            3 => match adobe {
                Some(0) => ColorModel::Rgb,
                Some(_) => ColorModel::YCbCr,
                None if ids == [b'R', b'G', b'B'] => ColorModel::Rgb,
                None => ColorModel::YCbCr,
            },
            4 => match adobe {
                Some(0) | None => ColorModel::Cmyk,
                Some(_) => ColorModel::Ycck,
            },
            _ => ColorModel::Unknown,
        };
        let facts = ScanFacts { width, height, components, scan: members, mcus_across, mcus_down, restart_interval: restart, color };
        if facts.blocks_per_mcu() > 10 {
            return Err(Refusal::Failed);
        }
        let dc = facts.scan.iter().map(|s| dc[s.dc_table as usize].clone().expect("checked")).collect();
        let ac = facts.scan.iter().map(|s| ac[s.ac_table as usize].clone().expect("checked")).collect();
        Ok(Header { facts, dc, ac })
    }
}

/// One restart interval's data with the stuffing out, and where it was.
struct Piece {
    /// Where its first byte is, as a byte of the run.
    start: usize,
    data: Vec<u8>,
    /// The indices in `data` of each `ff` that had a zero after it.
    stuffed: Vec<u32>,
    /// Where it ends in the run: at the restart marker after it, or where the
    /// run's data stops.
    end: usize,
    /// The number of the restart marker after it, when one is.
    marker: Option<u8>,
}

impl Piece {
    /// Where bit `p` of the data is, as a bit of the run. A stuffed zero
    /// comes before the first bit after its `ff`, so a step that ends with an
    /// `ff`'s last bit ends after the zero.
    fn file_bit(&self, p: u64) -> u64 {
        let d = p / 8;
        let skipped = self.stuffed.partition_point(|&s| (s as u64) < d) as u64;
        (self.start as u64 + d + skipped) * 8 + p % 8
    }
}

/// Cut the run at its restart markers and take the stuffing out. Also says
/// where the data stops, which is the end of the run unless an `ff` in it is
/// followed by something that is neither a zero nor a restart.
fn pieces(run: &[u8]) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut cur = Piece { start: 0, data: Vec::new(), stuffed: Vec::new(), end: 0, marker: None };
    let mut i = 0;
    while i < run.len() {
        let b = run[i];
        if b == 0xff {
            match run.get(i + 1) {
                Some(0x00) => {
                    cur.stuffed.push(cur.data.len() as u32);
                    cur.data.push(0xff);
                    i += 2;
                    continue;
                }
                Some(&m @ 0xd0..=0xd7) => {
                    cur.end = i;
                    cur.marker = Some(m - 0xd0);
                    let next = Piece { start: i + 2, data: Vec::new(), stuffed: Vec::new(), end: 0, marker: None };
                    out.push(std::mem::replace(&mut cur, next));
                    i += 2;
                    continue;
                }
                // A marker, or an `ff` with nothing after it: the data stops.
                _ => break,
            }
        }
        cur.data.push(b);
        i += 1;
    }
    cur.end = i;
    out.push(cur);
    out
}

/// Read one scan's run into its blocks' coefficients, and say which bits made
/// which.
pub fn scan(run: &[u8], header: &Header) -> Result<(Vec<u8>, Trace), Refusal> {
    let f = &header.facts;
    let mcus = f.mcus_across as u64 * f.mcus_down as u64;
    let per_mcu = f.blocks_per_mcu() as u64;
    let units = mcus * per_mcu;
    if units * BLOCK_BYTES as u64 > CAP_BYTES as u64 {
        return Err(Refusal::TooLarge);
    }
    let mut out = vec![0u8; units as usize * BLOCK_BYTES];
    let pieces = pieces(run);
    let mut b = TraceBuilder::default();
    // Every stuffed zero, as the bit of the run it starts at.
    for p in &pieces {
        for (n, &s) in p.stuffed.iter().enumerate() {
            b.stuffed((p.start as u64 + s as u64 + n as u64 + 1) * 8);
        }
    }
    // Where each MCU's blocks go, in the order they are coded: which of the
    // scan's channels, and how far across and down that channel's blocks.
    let layout: Vec<(usize, u32, u32)> = match f.scan.as_slice() {
        [_] => vec![(0, 0, 0)],
        many => many
            .iter()
            .enumerate()
            .flat_map(|(s, sc)| {
                let c = f.components[sc.index as usize];
                (0..c.v as u32).flat_map(move |v| (0..c.h as u32).map(move |h| (s, h, v)))
            })
            .collect(),
    };
    let interval = match f.restart_interval {
        0 => mcus,
        n => n as u64,
    };
    let mut unit = 0u64;
    let mut mcu = 0u64;
    let mut piece_at = 0usize;
    while mcu < mcus {
        let Some(piece) = pieces.get(piece_at) else { return Err(Refusal::Failed) };
        // The markers count round from 0 to 7, and one out of turn is one
        // missing or one too many.
        if piece_at > 0 && pieces[piece_at - 1].marker != Some(((piece_at - 1) % 8) as u8) {
            return Err(Refusal::Failed);
        }
        let mut r = Reader { data: &piece.data, pos: 0 };
        let mut pred = [0i32; 4];
        let last_here = (mcu + interval).min(mcus);
        while mcu < last_here {
            let (mx, my) = ((mcu % f.mcus_across as u64) as u32, (mcu / f.mcus_across as u64) as u32);
            b.open_block(piece.file_bit(r.pos), unit * BLOCK_BYTES as u64);
            for &(s, h, v) in &layout {
                let sc = f.scan[s];
                let (x, y) = match f.scan.len() {
                    1 => (mx, my),
                    _ => {
                        let c = f.components[sc.index as usize];
                        (mx * c.h as u32 + h, my * c.v as u32 + v)
                    }
                };
                let first = b.steps() as u32;
                let at = unit * BLOCK_BYTES as u64;
                let coefs = block(&mut r, piece, &header.dc[s], &header.ac[s], &mut pred[s], at, &mut b)?;
                let dst = &mut out[at as usize..at as usize + BLOCK_BYTES];
                for (k, &c) in coefs.iter().enumerate() {
                    let n = ZIGZAG[k] as usize;
                    dst[2 * n..2 * n + 2].copy_from_slice(&c.to_le_bytes());
                }
                b.unit(first..b.steps() as u32, sc.index, x as u16, y as u16);
                unit += 1;
            }
            mcu += 1;
            let out_at = unit * BLOCK_BYTES as u64;
            let end_at = if mcu < last_here {
                piece.file_bit(r.pos)
            } else {
                // The end of an interval: the rest of the last byte is
                // padding, which T.81 says is ones and which nothing reads.
                let whole = r.pos.div_ceil(8) * 8;
                if whole > r.pos {
                    b.push(piece.file_bit(r.pos), out_at, StepKind::Header(StepField::Padding, 0));
                }
                let read_to = piece.file_bit(whole);
                if mcu < mcus {
                    // A restart marker comes next, straight after the
                    // padding. Bytes between the two are data no code meant.
                    let Some(n) = piece.marker.filter(|_| read_to == piece.end as u64 * 8) else {
                        return Err(Refusal::Failed);
                    };
                    b.push(read_to, out_at, StepKind::Header(StepField::Restart, n as u32));
                    read_to + 16
                } else {
                    tail(&mut b, &pieces[piece_at..], read_to, out_at, run.len());
                    run.len() as u64 * 8
                }
            };
            b.close_block(end_at, out_at, BlockKind::Mcu { x: mx as u16, y: my as u16 }, mcu == mcus);
        }
        piece_at += 1;
    }
    b.finish_at(run.len() as u64 * 8, out.len() as u64);
    b.jpeg(f.clone());
    Ok((out, b.done()))
}

/// Whatever the run holds after the last MCU's padding, as steps of that MCU:
/// nothing, in a file written by the book. A restart marker some encoders
/// write after the last interval is named as one, and any other bytes are
/// bits no code read. `pieces` starts with the piece the last MCU was in, and
/// `at` is where its padding ended. Past the last piece is anything after an
/// `ff` that was neither stuffing nor a restart, which a run the template
/// measured does not hold.
fn tail(b: &mut TraceBuilder, pieces: &[Piece], mut at: u64, out_at: u64, run_len: usize) {
    for (i, p) in pieces.iter().enumerate() {
        let from = if i == 0 { at } else { p.start as u64 * 8 };
        if p.end as u64 * 8 > from {
            b.push(from, out_at, StepKind::Opaque);
        }
        at = (p.end as u64 * 8).max(from);
        if let Some(n) = p.marker {
            b.push(at, out_at, StepKind::Header(StepField::Restart, n as u32));
            at += 16;
        }
    }
    if run_len as u64 * 8 > at {
        b.push(at, out_at, StepKind::Opaque);
    }
}

/// One 8×8 block: a DC difference and then AC codes until the block is full
/// or an EOB ends it. Gives the 64 coefficients in zigzag order, and records
/// a step per code unless the trace has stopped naming them, in which case
/// the block is one step.
#[allow(clippy::too_many_arguments)]
fn block(
    r: &mut Reader,
    piece: &Piece,
    dc: &Huffman,
    ac: &Huffman,
    pred: &mut i32,
    out_at: u64,
    b: &mut TraceBuilder,
) -> Result<[i16; 64], Refusal> {
    let mut zz = [0i16; 64];
    let named = !b.over_budget();
    if !named {
        b.coarsen();
        b.push(piece.file_bit(r.pos), out_at, StepKind::Opaque);
    }
    let at = r.pos;
    let (code, size) = dc.decode(r)?;
    if size > 15 {
        return Err(Refusal::Failed);
    }
    let diff = extend(r.take(size as u32)?, size);
    *pred += diff;
    let dc_value = i16::try_from(*pred).map_err(|_| Refusal::Failed)?;
    zz[0] = dc_value;
    if named {
        b.push(piece.file_bit(at), out_at, StepKind::Dc { code, size, diff: diff as i16, dc: dc_value });
    }
    let mut k = 1u8;
    while k < 64 {
        let at = r.pos;
        let (code, rs) = ac.decode(r)?;
        let (run, size) = (rs >> 4, rs & 15);
        if size == 0 {
            if run == 15 {
                if named {
                    b.push(piece.file_bit(at), out_at, StepKind::Zrl { code, k });
                }
                k += 16;
                if k > 64 {
                    return Err(Refusal::Failed);
                }
                continue;
            }
            if run != 0 {
                return Err(Refusal::Failed);
            }
            if named {
                b.push(piece.file_bit(at), out_at, StepKind::Eob { code, k });
            }
            break;
        }
        k += run;
        if k > 63 {
            return Err(Refusal::Failed);
        }
        let value = extend(r.take(size as u32)?, size);
        let value = i16::try_from(value).map_err(|_| Refusal::Failed)?;
        zz[k as usize] = value;
        if named {
            b.push(piece.file_bit(at), out_at, StepKind::Ac { code, run, size, k, value });
        }
        k += 1;
    }
    Ok(zz)
}

/// The bits of `range` of the run less the stuffed bytes inside it, as
/// noughts and ones: what a code in the trace reads as.
pub fn code_string(run_bytes: &[u8], range: Range<u64>, stuffed: &[u64]) -> String {
    let mut s = String::with_capacity((range.end - range.start) as usize);
    let mut bit = range.start;
    let mut skip = stuffed.partition_point(|&b| b < range.start);
    while bit < range.end {
        if stuffed.get(skip) == Some(&bit) {
            bit += 8;
            skip += 1;
            continue;
        }
        let Some(&byte) = run_bytes.get((bit / 8) as usize) else { break };
        s.push(if byte >> (7 - bit % 8) & 1 == 1 { '1' } else { '0' });
        bit += 1;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Segments with their lengths written for them.
    fn seg(marker: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![0xff, marker];
        v.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    /// One DC table whose only symbol is 0 and one AC table whose only symbol
    /// is EOB, each a one-bit code `0`: every block is two bits, and every
    /// coefficient is zero.
    fn flat(width: u16, height: u16, comps: &[(u8, u8)], restart: u16) -> Vec<u8> {
        let mut v = Vec::new();
        let mut sof = vec![8];
        sof.extend_from_slice(&height.to_be_bytes());
        sof.extend_from_slice(&width.to_be_bytes());
        sof.push(comps.len() as u8);
        for (i, &(h, vv)) in comps.iter().enumerate() {
            sof.extend_from_slice(&[i as u8 + 1, h << 4 | vv, 0]);
        }
        v.extend(seg(0xc0, &sof));
        let mut dht = vec![0x00, 1];
        dht.extend_from_slice(&[0; 15]);
        dht.push(0);
        dht.push(0x10);
        dht.push(1);
        dht.extend_from_slice(&[0; 15]);
        dht.push(0);
        v.extend(seg(0xc4, &dht));
        if restart > 0 {
            v.extend(seg(0xdd, &restart.to_be_bytes()));
        }
        let mut sos = vec![comps.len() as u8];
        for i in 0..comps.len() {
            sos.extend_from_slice(&[i as u8 + 1, 0x00]);
        }
        sos.extend_from_slice(&[0, 63, 0]);
        v.extend(seg(0xda, &sos));
        v
    }

    #[test]
    fn a_block_of_nothing_is_two_codes() {
        let head = Header::read(&flat(8, 8, &[(1, 1)], 0)).unwrap();
        // Two one-bit codes, both 0, and six bits of padding, which are ones.
        let (out, trace) = scan(&[0b0011_1111], &head).unwrap();
        assert_eq!(out, vec![0; 128]);
        trace.check_tiles().unwrap();
        let kinds: Vec<_> = trace.steps().map(|s| (s.kind, s.in_bits)).collect();
        assert_eq!(
            kinds,
            vec![
                (StepKind::Dc { code: 1, size: 0, diff: 0, dc: 0 }, 0..1),
                (StepKind::Eob { code: 1, k: 1 }, 1..2),
                (StepKind::Header(StepField::Padding, 0), 2..8),
            ]
        );
        assert_eq!(trace.units().len(), 1);
        assert_eq!(trace.blocks().len(), 1);
        // The EOB closes the block, so it carries the block's bytes.
        assert_eq!(trace.step(1).unwrap().out_bytes, 0..128);
    }

    #[test]
    fn four_two_zero_codes_six_blocks_an_mcu() {
        // 16 by 16: one MCU of four Y blocks, a Cb and a Cr.
        let head = Header::read(&flat(16, 16, &[(2, 2), (1, 1), (1, 1)], 0)).unwrap();
        assert_eq!(head.facts.blocks_per_mcu(), 6);
        assert_eq!((head.facts.mcus_across, head.facts.mcus_down), (1, 1));
        // Twelve bits of codes, all 0, and four of padding.
        let (out, trace) = scan(&[0x00, 0x0f], &head).unwrap();
        assert_eq!(out.len(), 6 * 128);
        trace.check_tiles().unwrap();
        let at: Vec<_> = trace.units().iter().map(|u| (u.channel, u.x, u.y)).collect();
        assert_eq!(at, vec![(0, 0, 0), (0, 1, 0), (0, 0, 1), (0, 1, 1), (1, 0, 0), (2, 0, 0)]);
    }

    #[test]
    fn a_restart_marker_is_a_step_of_the_mcu_before_it() {
        let head = Header::read(&flat(16, 8, &[(1, 1)], 1)).unwrap();
        let run = [0b0011_1111, 0xff, 0xd0, 0b0011_1111];
        let (_, trace) = scan(&run, &head).unwrap();
        trace.check_tiles().unwrap();
        let first = &trace.blocks()[0];
        assert_eq!(first.in_bits, 0..24);
        let last = trace.step(first.steps.end as usize - 1).unwrap();
        assert_eq!(last.kind, StepKind::Header(StepField::Restart, 0));
        assert_eq!(last.in_bits, 8..24);
        assert_eq!(trace.blocks()[1].in_bits, 24..32);
        // A marker out of turn is refused.
        let run = [0b0011_1111, 0xff, 0xd3, 0b0011_1111];
        assert_eq!(scan(&run, &head).err(), Some(Refusal::Failed));
    }

    #[test]
    fn a_code_that_straddles_a_stuffed_byte_covers_it() {
        // A DC table whose one symbol is 8, as an 8-bit code of zeros, and
        // an AC table whose EOB is the one-bit code 0. The DC's eight value
        // bits are ones and fall on an `ff`, whose zero comes next.
        let mut v = Vec::new();
        v.extend(seg(0xc0, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        let mut dht = vec![0x00];
        dht.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
        dht.push(8);
        dht.push(0x10);
        dht.push(1);
        dht.extend_from_slice(&[0; 15]);
        dht.push(0);
        v.extend(seg(0xc4, &dht));
        v.extend(seg(0xda, &[1, 1, 0x00, 0, 63, 0]));
        let head = Header::read(&v).unwrap();
        // 0000_0000 | 1111_1111 (00) | 0111_1111: DC code, the value 255,
        // the EOB, and seven bits of padding.
        let run = [0x00, 0xff, 0x00, 0x7f];
        let (out, trace) = scan(&run, &head).unwrap();
        trace.check_tiles().unwrap();
        assert_eq!(trace.stuffed(), &[16]);
        let dc = trace.step(0).unwrap();
        assert_eq!(dc.kind, StepKind::Dc { code: 8, size: 8, diff: 255, dc: 255 });
        // Sixteen bits of code and value, and the stuffed zero after them.
        assert_eq!(dc.in_bits, 0..24);
        assert_eq!(code_string(&run, dc.in_bits.clone(), trace.stuffed()), "0000000011111111");
        assert_eq!(i16::from_le_bytes([out[0], out[1]]), 255);
        let eob = trace.step(1).unwrap();
        assert_eq!(eob.in_bits, 24..25);
    }

    #[test]
    fn what_baseline_does_not_cover_is_refused_by_name() {
        let mut v = flat(8, 8, &[(1, 1)], 0);
        v[1] = 0xc2;
        assert_eq!(Header::read(&v).err(), Some(Refusal::Unsupported(Unsupported::Progressive)));
        v[1] = 0xc9;
        assert_eq!(Header::read(&v).err(), Some(Refusal::Unsupported(Unsupported::Arithmetic)));
        v[1] = 0xc3;
        assert_eq!(Header::read(&v).err(), Some(Refusal::Unsupported(Unsupported::Lossless)));
        v[1] = 0xc1;
        v[4] = 12;
        assert_eq!(Header::read(&v).err(), Some(Refusal::Unsupported(Unsupported::Precision12)));
        // A scan naming a table nothing defined says the file does not say.
        let mut v = flat(8, 8, &[(1, 1)], 0);
        let n = v.len();
        v[n - 4] = 0x11;
        assert_eq!(Header::read(&v).err(), Some(Refusal::Settings));
    }
}
