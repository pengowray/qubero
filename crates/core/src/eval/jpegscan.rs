//! A JPEG scan's trace, read back as the numbers a figure of it needs.
//!
//! The listing reaches every code of a scan a node at a time, which is right
//! for a reader walking down to one code and wrong for a picture of where all
//! of them went: a photograph is hundreds of thousands of 8×8 blocks, and
//! placing a node for each to learn how many bits it took would place them all.
//! The trace already knows, so [`Evaluator::jpeg_scan`] reads the costs off it
//! in one pass: bits per MCU and per block, and the bits by what kind of code
//! spent them. [`Evaluator::jpeg_block`] gives one block in full, every code
//! with its bits and what it said, and the 64 coefficients the codes came to.
//!
//! Facts only. What a channel, a code or a coefficient is called on screen is
//! the interface's to say; the channel names are the colour model's own
//! letters, which is what the listing names blocks by.

use super::space::Opened;
use super::*;
use crate::codec::jpeg::{BLOCK_BYTES, ZIGZAG};
use crate::codec::{StepField, StepKind};

/// Where a JPEG scan's bits went, over the whole picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegScanMap {
    /// Where the scan's compressed run is, as bits of the space the reading
    /// reads, and how long it is.
    pub run_offset_bits: u64,
    pub run_bits: u64,
    pub width: u16,
    pub height: u16,
    /// How many MCUs the scan codes, across and down. A scan of one channel
    /// codes one block per MCU, so these are that channel's blocks.
    pub mcus_across: u32,
    pub mcus_down: u32,
    /// MCUs between restart markers; zero is none.
    pub restart_interval: u16,
    /// Every channel of the frame.
    pub channels: Vec<JpegChannel>,
    /// The channels this scan carries, by place in `channels`, in the order
    /// it codes them.
    pub scan: Vec<u8>,
    /// Bits each MCU takes, in coding order, counting the padding and the
    /// restart marker that end an interval with the MCU before them.
    pub mcu_bits: Vec<u32>,
    /// Every 8×8 block, in coding order.
    pub blocks: Vec<JpegBlockCost>,
    pub totals: JpegBitTotals,
    /// Whether the trace stopped naming codes past its limit on steps. The
    /// blocks after that are one step each, so their bits are in `unnamed`
    /// and their `codes` are 0.
    pub coarse: bool,
}

/// One channel of the frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegChannel {
    /// The colour model's letter for it (`Y`, `Cb`, `Cr`, `C`, `M`, `K`, `R`,
    /// `G`, `B`), or `component` and its id where the model has none.
    pub name: String,
    pub id: u8,
    pub h: u8,
    pub v: u8,
    pub blocks_across: u32,
    pub blocks_down: u32,
    /// Which quantization table scales it, and that table's 64 steps in rows,
    /// when a segment before the scan defined it.
    pub quant_id: u8,
    pub quant: Option<Vec<u16>>,
}

/// What one 8×8 block cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegBlockCost {
    pub channel: u8,
    pub x: u16,
    pub y: u16,
    /// Bits from its first code to the end of its last, stuffed bytes inside
    /// them included.
    pub bits: u32,
    /// How many codes: a DC, its AC codes, ZRLs and EOB.
    pub codes: u16,
}

/// Where a scan's bits went, by kind. Adds up to the run's length.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JpegBitTotals {
    /// Huffman code bits and the value bits after them, for DC differences
    /// and for AC coefficients.
    pub dc_code: u64,
    pub dc_value: u64,
    pub ac_code: u64,
    pub ac_value: u64,
    /// The code bits of EOBs and of ZRLs, which carry no value bits.
    pub eob: u64,
    pub zrl: u64,
    /// Ones filling out the last byte of an interval.
    pub padding: u64,
    /// The zero written after every `ff` of data, 8 bits each.
    pub stuffed: u64,
    /// Restart markers, 16 bits each.
    pub markers: u64,
    /// Bits the trace did not name: blocks past its limit on steps, and
    /// anything after the last MCU.
    pub unnamed: u64,
}

/// One 8×8 block in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegBlockTrace {
    /// Which block of the scan, in coding order, and which MCU it is in.
    pub index: usize,
    pub mcu: usize,
    pub channel: u8,
    pub x: u16,
    pub y: u16,
    /// The block's node in the listing, and the MCU's.
    pub path: Vec<usize>,
    pub mcu_path: Vec<usize>,
    pub codes: Vec<JpegCode>,
    /// The 64 coefficients the codes came to, in rows.
    pub coefficients: Vec<i16>,
}

/// One code of a block, as the decoder read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegCode {
    /// [`StepKind::as_str`]: `dc`, `ac`, `zrl`, `eob`, or `opaque` for a
    /// block the trace did not name.
    pub kind: &'static str,
    /// Where it is, as bits of the space the reading reads.
    pub start_bit: u64,
    pub end_bit: u64,
    /// How many bits are the Huffman code, and how many the value after it.
    pub code_bits: u8,
    pub value_bits: u8,
    /// The code and value bits as noughts and ones, without the stuffed
    /// zero byte when one lies inside them.
    pub bits: String,
    pub stuffed: bool,
    /// An AC code's run of zeros before its coefficient. Zero for the rest.
    pub run: u8,
    /// Where it lands in zigzag order: 0 for a DC, the coefficient's place for
    /// an AC, the first zero for a ZRL or an EOB.
    pub k: u8,
    /// The AC coefficient, or the DC difference.
    pub value: i16,
    /// The DC coefficient the difference came to. Zero for the rest.
    pub dc: i16,
}

impl Evaluator {
    /// Where the bits of the JPEG scan at `path` went. Nothing for a node that
    /// is not a JPEG scan, or one that would not open.
    pub fn jpeg_scan<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<JpegScanMap>> {
        let Some((space, r, from)) = self.jpeg_opened(doc, path)? else { return Ok(None) };
        let (mut map, quant) = {
            let Some(trace) = self.spaces.trace(space) else { return Ok(None) };
            let Some(f) = trace.jpeg() else { return Ok(None) };
            let mut totals = JpegBitTotals::default();
            for s in trace.steps() {
                let width = s.in_bits.end - s.in_bits.start;
                let inside = trace.stuffed_in(s.in_bits.clone()) as u64 * 8;
                totals.stuffed += inside;
                match s.kind {
                    StepKind::Dc { code, size, .. } => {
                        totals.dc_code += code as u64;
                        totals.dc_value += size as u64;
                    }
                    StepKind::Ac { code, size, .. } => {
                        totals.ac_code += code as u64;
                        totals.ac_value += size as u64;
                    }
                    StepKind::Eob { code, .. } => totals.eob += code as u64,
                    StepKind::Zrl { code, .. } => totals.zrl += code as u64,
                    StepKind::Header(StepField::Padding, _) => totals.padding += width - inside,
                    StepKind::Header(StepField::Restart, _) => totals.markers += width,
                    _ => totals.unnamed += width - inside,
                }
            }
            let blocks = trace
                .units()
                .iter()
                .map(|u| {
                    let first = trace.step(u.steps.start as usize).map_or(0, |s| s.in_bits.start);
                    let last = u.steps.end.checked_sub(1).and_then(|k| trace.step(k as usize)).map_or(first, |s| s.in_bits.end);
                    let named = u.steps.len() != 1 || !matches!(trace.step(u.steps.start as usize).map(|s| s.kind), Some(StepKind::Opaque));
                    JpegBlockCost { channel: u.channel, x: u.x, y: u.y, bits: (last - first) as u32, codes: if named { u.steps.len() as u16 } else { 0 } }
                })
                .collect();
            let channels: Vec<JpegChannel> = f
                .components
                .iter()
                .enumerate()
                .map(|(i, c)| JpegChannel {
                    name: f.channel_name(i),
                    id: c.id,
                    h: c.h,
                    v: c.v,
                    blocks_across: c.blocks_across,
                    blocks_down: c.blocks_down,
                    quant_id: c.quant,
                    quant: None,
                })
                .collect();
            let quant: Vec<Option<(u32, u32)>> = f.components.iter().map(|c| c.quant_at).collect();
            let map = JpegScanMap {
                run_offset_bits: r.offset,
                run_bits: trace.in_bits(),
                width: f.width,
                height: f.height,
                mcus_across: f.mcus_across,
                mcus_down: f.mcus_down,
                restart_interval: f.restart_interval,
                channels,
                scan: f.scan.iter().map(|s| s.index).collect(),
                mcu_bits: trace.blocks().iter().map(|b| (b.in_bits.end - b.in_bits.start) as u32).collect(),
                blocks,
                totals,
                coarse: trace.coarse(),
            };
            (map, quant)
        };
        // The steps each channel is scaled by, read out of the file where the
        // decoder said the table in force was defined: a precision and id byte,
        // then 64 steps in zigzag order, of one byte or two.
        for (c, at) in map.channels.iter_mut().zip(quant) {
            let Some((start, end)) = at else { continue };
            let bytes = self.read_in(doc, r.space, (from + start as u64) * 8, (end - start) as u64 * 8)?;
            let wide = bytes.first().is_some_and(|b| b >> 4 != 0);
            let mut natural = vec![0u16; 64];
            for (k, &n) in ZIGZAG.iter().enumerate() {
                natural[n as usize] = match wide {
                    true => u16::from_be_bytes([bytes[1 + 2 * k], bytes[2 + 2 * k]]),
                    false => bytes[1 + k] as u16,
                };
            }
            c.quant = Some(natural);
        }
        Ok(Some(map))
    }

    /// Block `index` of the JPEG scan at `path`, in coding order, with every
    /// code the decoder read for it and the coefficients they came to.
    pub fn jpeg_block<S: Source>(&mut self, doc: &Document<S>, path: &[usize], index: usize) -> R<Option<JpegBlockTrace>> {
        let Some((space, r, _)) = self.jpeg_opened(doc, path)? else { return Ok(None) };
        let (mut block, steps, stuffed, span) = {
            let Some(trace) = self.spaces.trace(space) else { return Ok(None) };
            let Some(unit) = trace.units().get(index) else { return Ok(None) };
            let mcu = trace.blocks().partition_point(|b| b.steps.end <= unit.steps.start);
            let child = match traced::UnitsView::of(trace, mcu as u32) {
                Some(v) => v.head.len() + (index - v.units.start),
                None => return Ok(None),
            };
            let steps: Vec<crate::codec::Step> = unit.steps.clone().filter_map(|k| trace.step(k as usize)).collect();
            let (Some(first), Some(last)) = (steps.first(), steps.last()) else { return Ok(None) };
            let span = (first.in_bits.start / 8 * 8, last.in_bits.end.div_ceil(8) * 8);
            let stuffed: Vec<u64> = trace.stuffed().iter().copied().filter(|&b| b >= span.0 && b < span.1).collect();
            let coefficients = match self.spaces.buf(space) {
                Some(buf) => buf
                    .get(index * BLOCK_BYTES..(index + 1) * BLOCK_BYTES)
                    .map(|b| b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            let block = JpegBlockTrace {
                index,
                mcu,
                channel: unit.channel,
                x: unit.x,
                y: unit.y,
                path: [path, &[1, mcu, child]].concat(),
                mcu_path: [path, &[1, mcu]].concat(),
                codes: Vec::new(),
                coefficients,
            };
            (block, steps, stuffed, span)
        };
        let bytes = self.read_in(doc, r.space, r.offset + span.0, span.1 - span.0)?;
        let shifted: Vec<u64> = stuffed.iter().map(|b| b - span.0).collect();
        block.codes = steps
            .iter()
            .map(|s| {
                let range = s.in_bits.start - span.0..s.in_bits.end - span.0;
                let has = shifted.iter().any(|b| range.contains(b));
                let bits = crate::codec::jpeg::code_string(&bytes, range, &shifted);
                let (code_bits, value_bits, run, k, value, dc) = match s.kind {
                    StepKind::Dc { code, size, diff, dc } => (code, size, 0, 0, diff, dc),
                    StepKind::Ac { code, run, size, k, value } => (code, size, run, k, value, 0),
                    StepKind::Zrl { code, k } | StepKind::Eob { code, k } => (code, 0, 0, k, 0, 0),
                    _ => (0, 0, 0, 0, 0, 0),
                };
                JpegCode {
                    kind: s.kind.as_str(),
                    start_bit: r.offset + s.in_bits.start,
                    end_bit: r.offset + s.in_bits.end,
                    code_bits,
                    value_bits,
                    bits,
                    stuffed: has,
                    run,
                    k,
                    value,
                    dc,
                }
            })
            .collect();
        Ok(Some(block))
    }

    /// The space the JPEG scan at `path` opened into, the node, and where the
    /// image's segments start in bytes. Nothing for a node that is not a JPEG
    /// scan, or a scan that would not open.
    fn jpeg_opened<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<(SpaceId, Resolved, u64)>> {
        self.resolve(doc, path)?;
        let r = self.memo[path].clone();
        let Ty::Decoded { codec: Packing::JpegScan { segments }, .. } = &r.ty else { return Ok(None) };
        let from = match self.eval_expr(doc, path, segments) {
            Ok(v) => match u64::try_from(v) {
                Ok(v) => v,
                Err(_) => return Ok(None),
            },
            Err(e) if e.interrupted() => return Err(e),
            Err(_) => return Ok(None),
        };
        match self.open_space_at(doc, path)? {
            Opened::Space(id) => Ok(Some((id, r, from))),
            Opened::Refused(_) => Ok(None),
        }
    }
}
