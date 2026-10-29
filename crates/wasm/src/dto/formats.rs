//! What single formats say beyond their fields: PDF, SQLite, GRIB, BUFR, miniSEED, JPEG, and the hardening of an executable.

use crate::*;

/// One row of a cross-reference stream, already decoded: the object it is
/// for, what it says about that object, and where that puts it. `offset` is
/// -1 for a row that names no place in the file, which is every row but an
/// in-use one.
#[derive(Serialize)]
pub(crate) struct XrefRowDto {
    pub(crate) object: f64,
    pub(crate) kind: &'static str,
    /// The type number the row held: 0, 1, 2, or whatever a row of a type
    /// nobody has defined wrote. Without it a panel showing an unknown row has
    /// only `kind`, which for all of them is the same word.
    pub(crate) type_raw: f64,
    pub(crate) offset: f64,
    pub(crate) second: f64,
    pub(crate) third: f64,
}

/// One object inside an object stream.
#[derive(Serialize)]
pub(crate) struct ObjStmObjectDto {
    pub(crate) number: f64,
    /// How long the object is in the decompressed bytes.
    pub(crate) len: f64,
    /// The object as written, cut at the limit the core keeps. `cut` says the
    /// rest was left behind.
    pub(crate) text: String,
    pub(crate) cut: bool,
}

/// One column of a row that was joined back together from the pages it spilled
/// onto.
#[derive(Serialize)]
pub(crate) struct SqliteColumnDto {
    /// What SQLite calls the type: `i32`, `text, 8189 bytes`, `null`.
    #[serde(rename = "type")]
    pub(crate) type_name: String,
    /// The value, shown the way every other value in the tree is shown.
    pub(crate) value: String,
    /// Which of the shapes a value takes, so it can be styled like its kind.
    pub(crate) value_kind: &'static str,
    /// Where it sits in the joined row, which is not where it sits in the file.
    pub(crate) at: f64,
    pub(crate) len: f64,
}

/// One step of reading a Parquet page: the codec, a list of levels, or the
/// encoding the values are in.
#[derive(Serialize)]
pub(crate) struct PageStepDto {
    /// The codec's name, the encoding's name, or which list of levels.
    pub(crate) what: String,
    pub(crate) in_bytes: f64,
    /// Zero for a step that produced values rather than bytes, which says how
    /// many of them in `note` instead.
    pub(crate) out_bytes: f64,
    pub(crate) note: String,
    /// Set when the step was not done at all: a v2 page that says
    /// `is_compressed` is false names its column's codec and never ran it.
    pub(crate) skipped: bool,
}

/// One filter undone on the way back to a chunk's elements.
#[derive(Serialize)]
pub(crate) struct ChunkStepDto {
    pub(crate) filter: String,
    pub(crate) in_bytes: f64,
    pub(crate) out_bytes: f64,
    /// Set when this chunk's own mask said the filter was not applied to it.
    pub(crate) skipped: bool,
}

/// One step of unpacking a GWF vector: gzip, zero suppression, differencing,
/// or putting complex parts back in pairs.
#[derive(Serialize)]
pub(crate) struct VectorStepDto {
    pub(crate) what: String,
    pub(crate) in_bytes: f64,
    pub(crate) out_bytes: f64,
}

/// One step of working out a GRIB message's values: a short label a problem
/// can name it by, what it came to, and how many numbers it was done to.
#[derive(Serialize)]
pub(crate) struct GribStepDto {
    pub(crate) label: String,
    pub(crate) what: String,
    pub(crate) count: f64,
}

/// The GRIB value under the cursor.
#[derive(Serialize)]
pub(crate) struct GribValueDto {
    /// Where it is in the message's run of values, from 0.
    pub(crate) index: f64,
    /// "values" for `values[index]`, "group" for `groups[group].values[position]`,
    /// "first" for `first_values[index]`.
    pub(crate) place: &'static str,
    /// Which group and where in it, and the number that field holds, for
    /// "group". Zero otherwise.
    pub(crate) group: f64,
    pub(crate) position: f64,
    pub(crate) written: f64,
    /// What it is worth, and the whole packed integer it was worked out from.
    pub(crate) value: String,
    pub(crate) packed: f64,
}

/// One descriptor of a BUFR message expanded through Table D, as deep as the
/// sequences it came out of.
#[derive(Serialize)]
pub(crate) struct BufrDescriptorDto {
    pub(crate) code: f64,
    pub(crate) depth: f64,
    pub(crate) name: String,
}

/// One value of a BUFR subset, as the panel lists it.
#[derive(Serialize)]
pub(crate) struct BufrValueDto {
    pub(crate) code: f64,
    /// "element" | "count" | "quality" | "associated" | "reference" | "local" | "characters" | "marker"
    pub(crate) role: &'static str,
    pub(crate) name: String,
    pub(crate) text: String,
    pub(crate) unit: String,
    pub(crate) missing: bool,
    /// The element an associated field, a marker, quality information or a
    /// new reference value is about, as its descriptor and name. Empty
    /// otherwise.
    pub(crate) about: String,
}

/// The BUFR value under the cursor, with what it was read from.
#[derive(Serialize)]
pub(crate) struct BufrCursorDto {
    /// Its place in `values`, or -1 where it is not among those listed.
    pub(crate) index: f64,
    pub(crate) value: BufrValueDto,
    /// Where its bits start, from section 4's first bit, and how wide it is.
    pub(crate) bit: f64,
    pub(crate) width: f64,
    pub(crate) scale: f64,
    pub(crate) reference: f64,
    pub(crate) numeric: bool,
    /// Null where there is no packed number: text, or a missing value.
    pub(crate) packed: Option<f64>,
    /// A compressed value's smallest packed number and difference width, or
    /// null for an uncompressed message.
    pub(crate) base: Option<f64>,
    pub(crate) increment_width: Option<f64>,
    /// Every subset's value, the first few dozen, for a compressed message.
    /// An empty string is a missing value.
    pub(crate) across: Vec<String>,
}

pub(crate) fn bufr_value_dto(v: qubero_core::formats::bufr_data::PanelValue) -> BufrValueDto {
    BufrValueDto {
        code: f64::from(v.code),
        role: v.role,
        name: v.name,
        text: v.text,
        unit: v.unit,
        missing: v.missing,
        about: v.about.unwrap_or_default(),
    }
}

/// One Steim frame of a miniSEED record: how many differences its codes named,
/// and how many of them became samples.
#[derive(Serialize)]
pub(crate) struct MseedFrameDto {
    pub(crate) held: f64,
    pub(crate) used: f64,
}

/// A filter walk's steps as the host reads them.
pub(crate) fn chunk_steps(steps: Vec<qubero_core::formats::hdf5_chunk::Step>) -> Vec<ChunkStepDto> {
    steps
        .into_iter()
        .map(|s| ChunkStepDto { filter: s.filter, in_bytes: s.in_bytes as f64, out_bytes: s.out_bytes as f64, skipped: s.skipped })
        .collect()
}

/// A JPEG scan's costs, as `jpeg_scan` hands them over. See
/// [`qubero_core::eval::JpegScanMap`].
#[derive(Serialize)]
pub(crate) struct JpegScanDto {
    pub(crate) run_offset_bits: f64,
    pub(crate) run_bits: f64,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) mcus_across: u32,
    pub(crate) mcus_down: u32,
    pub(crate) restart_interval: u16,
    pub(crate) channels: Vec<JpegChannelDto>,
    pub(crate) scan: Vec<u8>,
    pub(crate) mcu_bits: Vec<u32>,
    /// One entry per 8×8 block, in coding order, in each of these five.
    pub(crate) block_channel: Vec<u8>,
    pub(crate) block_x: Vec<u16>,
    pub(crate) block_y: Vec<u16>,
    pub(crate) block_bits: Vec<u32>,
    pub(crate) block_codes: Vec<u16>,
    pub(crate) totals: JpegTotalsDto,
    pub(crate) coarse: bool,
    /// How many steps a trace names before it stops; see
    /// [`qubero_core::codec::MAX_STEPS`].
    pub(crate) step_limit: f64,
}

#[derive(Serialize)]
pub(crate) struct JpegChannelDto {
    pub(crate) name: String,
    pub(crate) id: u8,
    pub(crate) h: u8,
    pub(crate) v: u8,
    pub(crate) blocks_across: u32,
    pub(crate) blocks_down: u32,
    pub(crate) quant_id: u8,
    /// The 64 steps in rows, or null when no segment before the scan
    /// defined the table.
    pub(crate) quant: Option<Vec<u16>>,
}

#[derive(Serialize)]
pub(crate) struct JpegTotalsDto {
    pub(crate) dc_code: f64,
    pub(crate) dc_value: f64,
    pub(crate) ac_code: f64,
    pub(crate) ac_value: f64,
    pub(crate) eob: f64,
    pub(crate) zrl: f64,
    pub(crate) padding: f64,
    pub(crate) stuffed: f64,
    pub(crate) markers: f64,
    pub(crate) unnamed: f64,
}

pub(crate) fn jpeg_scan_dto(m: qubero_core::eval::JpegScanMap) -> JpegScanDto {
    let t = m.totals;
    JpegScanDto {
        run_offset_bits: m.run_offset_bits as f64,
        run_bits: m.run_bits as f64,
        width: m.width,
        height: m.height,
        mcus_across: m.mcus_across,
        mcus_down: m.mcus_down,
        restart_interval: m.restart_interval,
        channels: m
            .channels
            .into_iter()
            .map(|c| JpegChannelDto {
                name: c.name,
                id: c.id,
                h: c.h,
                v: c.v,
                blocks_across: c.blocks_across,
                blocks_down: c.blocks_down,
                quant_id: c.quant_id,
                quant: c.quant,
            })
            .collect(),
        scan: m.scan,
        mcu_bits: m.mcu_bits,
        block_channel: m.blocks.iter().map(|b| b.channel).collect(),
        block_x: m.blocks.iter().map(|b| b.x).collect(),
        block_y: m.blocks.iter().map(|b| b.y).collect(),
        block_bits: m.blocks.iter().map(|b| b.bits).collect(),
        block_codes: m.blocks.iter().map(|b| b.codes).collect(),
        totals: JpegTotalsDto {
            dc_code: t.dc_code as f64,
            dc_value: t.dc_value as f64,
            ac_code: t.ac_code as f64,
            ac_value: t.ac_value as f64,
            eob: t.eob as f64,
            zrl: t.zrl as f64,
            padding: t.padding as f64,
            stuffed: t.stuffed as f64,
            markers: t.markers as f64,
            unnamed: t.unnamed as f64,
        },
        coarse: m.coarse,
        step_limit: qubero_core::codec::MAX_STEPS as f64,
    }
}

/// One JPEG block, as `jpeg_block` hands it over. See
/// [`qubero_core::eval::JpegBlockTrace`].
#[derive(Serialize)]
pub(crate) struct JpegBlockDto {
    pub(crate) index: u32,
    pub(crate) mcu: u32,
    pub(crate) channel: u8,
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) path: Vec<u32>,
    pub(crate) mcu_path: Vec<u32>,
    pub(crate) codes: Vec<JpegCodeDto>,
    pub(crate) coefficients: Vec<i16>,
}

#[derive(Serialize)]
pub(crate) struct JpegCodeDto {
    pub(crate) kind: &'static str,
    pub(crate) start_bit: f64,
    pub(crate) end_bit: f64,
    pub(crate) code_bits: u8,
    pub(crate) value_bits: u8,
    pub(crate) bits: String,
    pub(crate) stuffed: bool,
    pub(crate) run: u8,
    pub(crate) k: u8,
    pub(crate) value: i16,
    pub(crate) dc: i16,
}

pub(crate) fn jpeg_block_dto(b: qubero_core::eval::JpegBlockTrace) -> JpegBlockDto {
    JpegBlockDto {
        index: b.index as u32,
        mcu: b.mcu as u32,
        channel: b.channel,
        x: b.x,
        y: b.y,
        path: b.path.iter().map(|&i| i as u32).collect(),
        mcu_path: b.mcu_path.iter().map(|&i| i as u32).collect(),
        codes: b
            .codes
            .into_iter()
            .map(|c| JpegCodeDto {
                kind: c.kind,
                start_bit: c.start_bit as f64,
                end_bit: c.end_bit as f64,
                code_bits: c.code_bits,
                value_bits: c.value_bits,
                bits: c.bits,
                stuffed: c.stuffed,
                run: c.run,
                k: c.k,
                value: c.value,
                dc: c.dc,
            })
            .collect(),
        coefficients: b.coefficients,
    }
}

/// What a program was built to withstand, as `hardening` hands it over. See
/// [`qubero_core::formats::hardening::Hardening`].
#[derive(Serialize)]
pub(crate) struct HardeningDto {
    /// `elf`, `pe` or `macho`.
    pub(crate) format: &'static str,
    pub(crate) parts: Vec<HardeningPartDto>,
}

#[derive(Serialize)]
pub(crate) struct HardeningPartDto {
    /// Empty for one program; the CPU for each slice of a universal Mach-O.
    pub(crate) name: String,
    /// The program's header node.
    pub(crate) path: Vec<usize>,
    pub(crate) rows: Vec<HardeningRowDto>,
}

#[derive(Serialize)]
pub(crate) struct HardeningRowDto {
    pub(crate) key: &'static str,
    pub(crate) state: &'static str,
    /// `good`, `partial`, `bad`, `info`, `unknown` or `n/a`.
    pub(crate) verdict: &'static str,
    pub(crate) count: Option<f64>,
    pub(crate) total: Option<f64>,
    pub(crate) items: Vec<String>,
    pub(crate) more: Vec<String>,
    /// Most direct first; at most a dozen.
    pub(crate) evidence: Vec<EvidenceDto>,
}

#[derive(Serialize)]
pub(crate) struct EvidenceDto {
    /// The node holding the bytes, or empty when no field covers them.
    pub(crate) path: Vec<usize>,
    pub(crate) offset_bits: f64,
    pub(crate) size_bits: f64,
    pub(crate) what: &'static str,
    pub(crate) name: String,
}

pub(crate) fn hardening_dto(h: qubero_core::formats::hardening::Hardening) -> HardeningDto {
    HardeningDto {
        format: h.format,
        parts: h
            .parts
            .into_iter()
            .map(|p| HardeningPartDto {
                name: p.name,
                path: p.path,
                rows: p
                    .rows
                    .into_iter()
                    .map(|r| HardeningRowDto {
                        key: r.key,
                        state: r.state,
                        verdict: r.rating.as_str(),
                        count: r.count.map(|n| n as f64),
                        total: r.total.map(|n| n as f64),
                        items: r.items,
                        more: r.more,
                        evidence: r
                            .evidence
                            .into_iter()
                            .map(|e| EvidenceDto {
                                path: e.path,
                                offset_bits: e.offset_bits as f64,
                                size_bits: e.size_bits as f64,
                                what: e.what,
                                name: e.name,
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}
