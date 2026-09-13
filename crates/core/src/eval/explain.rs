//! What a type knows beyond the value in front of it: the values an enum
//! names, the bytes a magic field wanted, what each bit of a flags field means,
//! and which bits of a float are which.

use super::*;
use crate::formats::ggml_quant::{self, Group, Offset, Quant, Weight};
use crate::formats::{bufr_data, fits_tile, grib_values, gwf_vect, hdf5_chunk, mseed_steim, parquet_page, pdf_objstm, pdf_xref, sqlite_overflow};

/// What a type permits, as opposed to what this file happens to hold.
///
/// Four field kinds know more than their value shows: an enum knows the other
/// values it would accept, a magic field knows the bytes it wanted, a flags
/// field knows what each bit means, and a float knows which of its bits are
/// which. This is one answer for all of them, because they are one question:
/// what does this type say, beyond the number.
#[derive(Debug, Clone, PartialEq)]
pub enum Explain {
    /// The bytes the format requires, and the bytes that are there. They are
    /// equal when the field matches, and worth comparing when it does not.
    Magic { expected: Vec<u8>, actual: Vec<u8> },
    /// Every value the enum names, and the one the file holds. `current` is not
    /// always among them: a file is free to hold a value nobody named.
    /// `named` is what the value goes by, which is not always in `cases`: a
    /// format that stops naming values and starts counting them names it by
    /// the run it falls in instead.
    Enum { name: String, hex: bool, cases: Vec<(i128, String)>, current: i128, named: Option<String> },
    /// Every bit of the field, from bit 0 up, whether it is set and what it is
    /// called. A bit with no name is still a bit, and is still listed.
    Flags { name: String, raw: u128, bits: Vec<FlagBit> },
    /// A binary float, as its bits: 16, 32 or 64 of them, in value order with
    /// the byte order already resolved, so a reader can take the sign, the
    /// exponent and the significand apart without knowing how it was stored.
    /// A float, by the name of its layout rather than only its width: two
    /// sixteen-bit floats are in use and they divide their bits differently.
    Float { format: &'static str, width: u32, bits: u64 },
    /// A block of packed weights, taken apart: the block's shared scale, what
    /// it pairs with the scale, and every weight the block stands for, in the
    /// order the tensor reads them. Shown for the cursor anywhere in the block,
    /// because the packing crosses the fields: a `q4_k` weight is four bits of
    /// `qs` scaled by six bits of `scales` and two half floats at the front.
    Quant {
        /// The block layout, as ggml's own struct is named: `Q4_K`.
        kind: &'static str,
        /// How many bits one weight is worth.
        bits: u32,
        /// The block's shared scale, and what it pairs with: the `m` a `q4_1`
        /// adds, the `dmin` a K type takes away.
        d: f64,
        second: Option<Offset>,
        /// Where the block starts, so that a weight's bits can be found from
        /// the offset it carries.
        block_bits: u64,
        /// The scales the block keeps per group of weights, where it has them,
        /// and how many weights one group covers. What a K type spends twelve
        /// or sixteen bytes on, which read as bytes say nothing.
        groups: Vec<Group>,
        group_weights: u32,
        /// Taken off the packed value to get the stored one, and whether that
        /// value is read signed instead.
        bias: i32,
        signed: bool,
        weights: Vec<Weight>,
        /// Which weight the cursor is inside, where it is on one of them
        /// rather than on the block's scales.
        at: Option<usize>,
    },
    /// A cross-reference stream, decompressed and split into the rows it
    /// stands for. Shown for the cursor anywhere in the object, because the
    /// rows are not bytes of the file and there is nothing to land on: what
    /// the reader is standing on is the packed run they came out of.
    XrefRows {
        /// The widths from `/W` and the predictor from `/DecodeParms`, which
        /// say how the run was taken apart.
        widths: [u32; 3],
        predictor: Option<u32>,
        /// How many bytes the run is in the file, and how many it came to once
        /// decompressed.
        packed_bytes: u64,
        decoded_bytes: u64,
        /// How many rows there are of each kind, over the whole table rather
        /// than over the ones listed. `unknown` counts rows whose type the
        /// spec does not define, which are kept apart so the four add up to
        /// the total rather than quietly landing in one of the other three.
        free: usize,
        in_file: usize,
        in_stream: usize,
        unknown: usize,
        /// The rows themselves, up to [`XREF_ROWS_SHOWN`] of them, and how
        /// many there are altogether. A table with more says so rather than
        /// looking complete.
        rows: Vec<pdf_xref::Row>,
        total: usize,
        /// Why there are no rows, where there are none.
        problem: Option<String>,
    },
    /// An object stream, opened into the objects it holds. Shown for the
    /// cursor anywhere in the object, because the objects inside are not bytes
    /// of the file: what the reader is standing on is the compressed run they
    /// came out of.
    ObjStm {
        /// How many bytes the run is in the file, and how many it came to once
        /// decompressed.
        packed_bytes: u64,
        decoded_bytes: u64,
        /// Where the objects begin in the decompressed bytes, from `/First`.
        first: u64,
        /// The object number in `/Extends`: the object stream this one is a
        /// continuation of, where it is one. Not followed.
        extends: Option<u64>,
        /// The objects, up to [`OBJSTM_SHOWN`] of them, and how many the
        /// dictionary said there were.
        objects: Vec<pdf_objstm::Object>,
        total: usize,
        /// Why there are no objects, where there are none.
        problem: Option<String>,
    },
    /// A row too big for its page, joined back together and read.
    ///
    /// Shown for the cursor anywhere in the payload, because what is under the
    /// cursor is the part of the row that stayed: the rest is on a chain of
    /// pages elsewhere, and the columns are not at any one place in the file.
    SqliteRow {
        /// How many bytes the row claims, and how many the chain reached.
        /// Equal for a row that is whole.
        declared: u64,
        found: u64,
        /// How many bytes stayed on the row's own page.
        on_page: u64,
        /// The overflow pages, in the order the chain names them, and how many
        /// there are when that is more than [`CHAIN_SHOWN`].
        pages: Vec<u32>,
        chain_length: usize,
        /// The columns, as the record's own header says to read them.
        columns: Vec<sqlite_overflow::Column>,
        total_columns: usize,
        /// Why the walk stopped or the columns would not read.
        problem: Option<String>,
    },
    /// A chunk of a dataset, taken back through the filters it was written
    /// through. Shown for the cursor anywhere in the chunk, because what is
    /// under the cursor is the last filter's output and the elements are not
    /// in the file at all.
    Hdf5Chunk {
        /// How many bytes the chunk is in the file, and how many the elements
        /// came to once every filter that could be undone was.
        packed_bytes: u64,
        decoded_bytes: u64,
        /// Each filter, in the order it was undone, with what went in and what
        /// came out.
        steps: Vec<hdf5_chunk::Step>,
        /// The first elements, as the datatype beside the chunk reads them.
        values: Vec<String>,
        /// How many elements the decoded bytes hold, of which `values` shows
        /// the first few.
        total: u64,
        /// What one element is called, so the panel can say `f32` rather than
        /// only show numbers.
        element_type: String,
        /// Why the walk stopped early, where it did.
        problem: Option<String>,
    },
    /// A frame vector's numbers, unpacked by [`gwf_vect`]. Shown for the
    /// cursor anywhere in the packed run, because what is under the cursor is
    /// zero-suppressed bits or the differences a gzip stream came to, and the
    /// numbers are not in the file as they stand.
    ///
    /// The steps are the same kind of walk an HDF5 chunk's filters are, and
    /// are carried in the same shape, but the answer is its own: a vector is
    /// not a chunk, and its steps are not filters.
    GwfVector {
        /// The vector's `compress` field as written, the scheme in its low
        /// byte, and whether its high byte says a little-endian machine packed
        /// the words.
        compress: u16,
        little: bool,
        /// How many numbers the vector's `nData` says it holds.
        declared: u64,
        /// How many bytes the packed run is in the file, and how many the
        /// numbers came to once every step that could be done was.
        packed_bytes: u64,
        decoded_bytes: u64,
        /// Each step, in the order it was done, with what went in and what
        /// came out.
        steps: Vec<hdf5_chunk::Step>,
        /// The first numbers, how many came out, and what one is called.
        values: Vec<String>,
        total: u64,
        element_type: String,
        /// Why the unpacking stopped early, where it did.
        problem: Option<String>,
    },
    /// A GRIB2 message's packed values, worked out by [`grib_values`]: what
    /// each packed number in section 7 is worth, which takes section 5's
    /// reference value and scale factors and, for complex packing, the groups
    /// and the differencing undone. Shown for the cursor anywhere in the
    /// section's data, a value or the tables in front of them, because a
    /// value's worth is nowhere in the file and, under spatial differencing,
    /// depends on every value before it.
    GribValues {
        /// The data representation template that packed them: 0 for simple
        /// packing, 2 for complex, 3 for complex with spatial differencing,
        /// and for 3, whether the differences are first or second order.
        template: u16,
        spatial_order: u32,
        /// R, E and D, the reference value as [`grib_values::Packing::reference_text`]
        /// writes it, so a panel can show the formula a value came out of.
        reference: String,
        binary_scale: i32,
        decimal_scale: i32,
        /// The overall minimum of the differences, under spatial differencing.
        minimum: Option<i64>,
        /// How many values section 5 says section 7 holds.
        declared: u64,
        /// How many bytes section 7's data is in the file.
        packed_bytes: u64,
        /// Every step, in the order it was done.
        steps: Vec<grib_values::Step>,
        /// The first values, as text, and how many came out.
        values: Vec<String>,
        total: u64,
        /// The value the cursor is on, where it is on one packed value rather
        /// than on the tables, the padding or the minimum.
        at: Option<GribValue>,
        /// What stopped it, or what it could not say.
        problem: Option<String>,
    },
    /// A miniSEED record's samples, worked out of its data by
    /// [`mseed_steim`]. Shown for the cursor anywhere in the data, because a
    /// Steim sample is not at any one place in the file: it is every
    /// difference before it added up, and what the cursor is on is one word
    /// of one frame.
    MseedSamples {
        /// The encoding's number and what it is called, and whether the data
        /// was laid out big-endian.
        encoding: u8,
        encoding_name: String,
        big_endian: bool,
        /// How many samples the header gives, and how many bytes of data they
        /// were decoded from.
        declared: u64,
        payload_bytes: u64,
        /// The Steim steps: the integration constants, the skipped first
        /// difference, and what each frame held and gave. None for every
        /// other encoding. The frames are cut to [`MSEED_FRAMES_SHOWN`], and
        /// `frames_walked` says how many there were.
        steim: Option<mseed_steim::Steim>,
        frames_walked: u64,
        /// The rule a gain-ranged encoding's words are turned into samples by.
        rule: Option<String>,
        /// The first samples, and the last, and how many were decoded.
        values: Vec<String>,
        last: Option<String>,
        total: u64,
        /// The last sample against the reverse integration constant, where
        /// the record is Steim and every sample was decoded.
        check: Option<mseed_steim::Check>,
        /// Why fewer samples than the header gives were decoded, or none.
        problem: Option<String>,
    },
    /// A Parquet page, read the whole way: the codec, the levels, the encoding
    /// and the values. Shown for the cursor anywhere in the page, because what
    /// the hex view shows there is packed bytes and the values are nowhere in
    /// the file as they stand.
    ParquetPage {
        /// How many bytes the payload is in the file, and how many the values
        /// came to once the codec was undone.
        packed_bytes: u64,
        decoded_bytes: u64,
        /// Every step, in the order it was done: the codec, each level list,
        /// then the encoding.
        steps: Vec<parquet_page::Step>,
        /// The first values, as text.
        values: Vec<String>,
        /// How many values the page holds, of which `values` shows the first
        /// few.
        total: u64,
        /// What one value is called: `i32`, `byte array`, `dictionary index`.
        element_type: String,
        /// Why the reading stopped early, where it did.
        problem: Option<String>,
    },
    /// A tile of a FITS compressed image, decompressed. Shown for the cursor
    /// anywhere in the image's data, a row or its compressed bytes, because
    /// what is under the cursor is compressed and the pixels are not in the
    /// file as it stands. See [`fits_tile`].
    FitsTile {
        /// Which tile, counted from 0 as its row is, and how many the image
        /// has.
        index: u64,
        tiles: u64,
        /// Where the tile starts in the image, from 0 along each axis, how
        /// many pixels it has along each, and the image's own shape.
        start: Vec<u64>,
        shape: Vec<u64>,
        image_shape: Vec<u64>,
        /// `ZCMPTYPE`, and which column the tile's bytes were read from.
        algorithm: String,
        column: Option<&'static str>,
        /// How many bytes the tile is in the heap, and how many came out of
        /// undoing its compression.
        packed_bytes: u64,
        decoded_bytes: u64,
        /// Every step, in the order it was done.
        steps: Vec<fits_tile::Step>,
        /// The first pixels, as text, how many pixels were decoded, and how
        /// many the tile has.
        values: Vec<String>,
        total: u64,
        pixels: u64,
        /// What one pixel is: `i16`, `f32`.
        element_type: String,
        /// Why fewer pixels than the tile has came out, or none.
        problem: Option<String>,
    },
    /// A BUFR message's section 4, read through the tables: the steps, the
    /// descriptors expanded, a subset's values, and the value under the
    /// cursor taken apart. Shown for the cursor anywhere in the section,
    /// because the values have no boundaries in the bits and only the walk
    /// through the descriptors finds them. See [`bufr_data`].
    BufrData(Box<bufr_data::Panel>),
    /// The type has nothing to add: its value already says everything.
    Plain,
}

/// How many of a record's samples the panel shows, beside the last one. A
/// 4096-byte record holds thousands; a row of them says what the signal is
/// doing, and the count and the last sample say how far it goes.
pub const MSEED_VALUES_SHOWN: usize = 32;

/// How many of a record's frames the panel lists. A 4096-byte record has 63;
/// a payload of a megabyte has sixteen thousand, which is a number to state
/// rather than a list to read.
pub const MSEED_FRAMES_SHOWN: usize = 256;

/// How much of a FITS header is read for the keywords a compressed image's
/// tile needs. A header is a few hundred cards; a megabyte is twelve thousand,
/// and the keywords that describe the compression come near the top.
const FITS_CARDS_LIMIT: u64 = 1 << 20;

/// How many objects of an object stream are handed to a reader at once.
pub const OBJSTM_SHOWN: usize = 256;

/// How many of a chain's pages a panel names. A chain of three hundred is a
/// fact about the file worth stating as a number rather than as three hundred
/// page numbers nobody will read.
pub const CHAIN_SHOWN: usize = 32;

/// How much of an object is read to find out whether it is an object stream.
/// Its dictionary comes first and no real one runs longer than this, so an
/// image object several megabytes long is passed over for the price of a page.
const OBJSTM_DICT_PREFIX: u64 = 8 << 10;

/// The largest compressed run this will open for a panel that is redrawn every
/// time the cursor moves.
const OBJSTM_PACKED_LIMIT: u64 = 4 << 20;

/// How many rows of a cross-reference stream are handed to a reader at once.
/// A table runs to one row an object, and a panel is not the place to read a
/// hundred thousand of them.
pub const XREF_ROWS_SHOWN: usize = 512;

/// How many of a chunk's elements the panel shows. A chunk holds tens of
/// thousands; a row of them says what kind of numbers these are, which is what
/// the panel is for, and the rest are in the file for the reading.
pub const HDF5_VALUES_SHOWN: usize = 32;

/// The largest packed run this will decompress for a panel that is redrawn
/// every time the cursor moves. Four megabytes of compressed table is a file
/// with millions of objects; nothing real reaches it.
const XREF_PACKED_LIMIT: u64 = 4 << 20;

/// Which side reader a packed structure is taken apart by: the module that
/// knows what a template can only say is bytes.
///
/// Each is found from the name the template marked the structure with, and
/// each says how far above the cursor that structure may be. Adding a reader
/// is a case here, a line in [`Unpacker::of`] and [`Unpacker::reach`], and
/// the function that builds its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unpacker {
    Xref,
    ObjStm,
    Hdf5Chunk,
    GwfVect,
    SqliteRow,
    ParquetPage,
    FitsTile,
    GribValues,
    BufrData,
    /// The encoding and byte order are in the packing name, since the template
    /// has already settled both by the time it marks the data.
    Mseed { encoding: u8, big: bool },
    /// A ggml block, by the layout name. Not every layout ggml has can be
    /// taken apart here, which is only found out by asking.
    Quant,
}

/// How far above the cursor a reader's packed structure is looked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// The field the cursor is on, or its parent. As deep as the fields of
    /// most packed structures go, and as far as they have been checked: a
    /// filtered HDF5 chunk is one run of bytes under the structure that marks
    /// it.
    Near,
    /// Any structure the cursor is inside, however far down. A Steim
    /// difference is four levels under a miniSEED record's data, the frame,
    /// the word, the word's shape and the difference; a FITS tile's
    /// descriptors are four levels into a row, and its compressed bytes are a
    /// byte of an array in the heap; a GRIB value under complex packing is
    /// four levels under section 7's data, the groups, the group, its values
    /// and the value.
    Anywhere,
}

impl Unpacker {
    /// The reader a packing name belongs to, if any here does.
    fn of(packing: &str) -> Option<Unpacker> {
        Some(match packing {
            pdf_xref::PACKING => Unpacker::Xref,
            pdf_objstm::PACKING => Unpacker::ObjStm,
            hdf5_chunk::PACKING => Unpacker::Hdf5Chunk,
            gwf_vect::PACKING => Unpacker::GwfVect,
            sqlite_overflow::PACKING => Unpacker::SqliteRow,
            parquet_page::PACKING => Unpacker::ParquetPage,
            fits_tile::PACKING => Unpacker::FitsTile,
            grib_values::PACKING => Unpacker::GribValues,
            bufr_data::PACKING => Unpacker::BufrData,
            _ => {
                if let Some((encoding, big)) = mseed_steim::parse_packing(packing) {
                    return Some(Unpacker::Mseed { encoding, big });
                }
                ggml_quant::by_name(packing)?;
                Unpacker::Quant
            }
        })
    }

    fn reach(self) -> Reach {
        match self {
            Unpacker::FitsTile | Unpacker::Mseed { .. } | Unpacker::GribValues => Reach::Anywhere,
            Unpacker::Xref
            | Unpacker::ObjStm
            | Unpacker::Hdf5Chunk
            | Unpacker::GwfVect
            | Unpacker::SqliteRow
            | Unpacker::ParquetPage
            // Section 4's fields are its length, a reserved byte and its bits,
            // none of them more than one level under the section.
            | Unpacker::BufrData
            | Unpacker::Quant => Reach::Near,
        }
    }
}

/// Where a packed structure was found: the cursor's path, the structure's
/// own path and what it resolved to, and the cursor's bit, which only a ggml
/// block uses to say which weight is under it.
struct Packed<'a> {
    path: &'a [usize],
    at: &'a [usize],
    r: &'a Resolved,
    at_bits: Option<u64>,
}

/// One GRIB value, the one under the cursor: where it is in the message's run
/// of values and in the tree, what it is worth, and the whole packed number it
/// was worked out from.
///
/// `packed` is not always the number the tree shows on the field. Complex
/// packing writes how far a value is above its group's reference, and spatial
/// differencing writes a difference on top of that; `packed` is what those
/// come to once undone, the X in `(R + X * 2^E) / 10^D`.
#[derive(Debug, Clone, PartialEq)]
pub struct GribValue {
    /// Where the value is in the message's run of values, from 0.
    pub index: u64,
    pub place: GribPlace,
    /// What it is worth, as [`grib_values::Packing::text`] writes it.
    pub value: String,
    pub packed: i64,
}

/// Which field of section 7 a GRIB value is.
#[derive(Debug, Clone, PartialEq)]
pub enum GribPlace {
    /// `values[index]` of simply packed data, which holds X as it is.
    Values,
    /// `groups[group].values[position]` of complex packing, and the number
    /// that field holds, which is only part of X.
    Group { group: u64, position: u64, written: i64 },
    /// `first_values[index]` under spatial differencing: one of the values
    /// written whole, which holds X as it is.
    First,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlagBit {
    pub bit: u32,
    pub name: Option<String>,
    pub set: bool,
}

impl Evaluator {
    /// What the type at `path` permits. See [`Explain`].
    /// `at_bits` is where the cursor is, which only a packed block uses: it
    /// says which of the block's weights the reader is standing on.
    pub fn explain<S: Source>(&mut self, doc: &Document<S>, path: &[usize], at_bits: Option<u64>) -> R<Explain> {
        self.resolve(doc, path)?;
        let size = self.size_of(doc, path)?;
        let r = self.memo.get(path).expect("resolved").clone();
        // A sentinel is a reading of one value, so what is worth explaining is
        // the number under it: an unset SAC float still wants the float panel.
        let ty = r.ty.without_sentinel().clone();
        Ok(match &ty {
            Ty::Magic(want) => {
                // A short read is not a failure to explain: the expected bytes
                // are known whatever the file turned out to hold.
                let actual = self.read(doc, &r, r.offset, size).unwrap_or_default();
                Explain::Magic { expected: want.clone(), actual }
            }
            Ty::Enum { def, .. } => {
                let current = self.value_at(doc, path)?.as_int().unwrap_or(0);
                Explain::Enum {
                    name: def.name.clone(),
                    hex: def.hex,
                    cases: def.cases.clone(),
                    current,
                    named: def.name_of(current),
                }
            }
            Ty::Flags { def, .. } => {
                let raw = match self.value_at(doc, path)? {
                    Value::Flags { raw, .. } => raw,
                    other => other.as_int().and_then(|v| u128::try_from(v).ok()).unwrap_or(0),
                };
                let bits = (0..size.min(64) as u32)
                    .map(|bit| FlagBit {
                        bit,
                        name: def.label(bit).map(str::to_string),
                        set: raw >> bit & 1 == 1,
                    })
                    .collect();
                Explain::Flags { name: def.name.clone(), raw, bits }
            }
            // An eight-bit float is one byte, so there is no order to it.
            Ty::F8 { e4m3 } => {
                let raw = self.read(doc, &r, r.offset, 8)?;
                Explain::Float { format: if *e4m3 { "e4m3" } else { "e5m2" }, width: 8, bits: raw[0] as u64 }
            }
            Ty::F16(e) | Ty::BF16(e) | Ty::F32(e) | Ty::F64(e) => {
                let (format, width): (&'static str, u32) = match ty {
                    Ty::F16(_) => ("binary16", 16),
                    Ty::BF16(_) => ("bfloat16", 16),
                    Ty::F32(_) => ("binary32", 32),
                    _ => ("binary64", 64),
                };
                let raw = self.read(doc, &r, r.offset, u64::from(width))?;
                Explain::Float { format, width, bits: crate::decode::read_uint(&raw, width, *e) as u64 }
            }
            _ => return self.explain_packed(doc, path, at_bits),
        })
    }

    /// What a side reader makes of the packed structure the cursor is in, or
    /// on one of the fields of.
    ///
    /// The fields are asked about as well as the structure because the fields
    /// are where a reader lands: the cursor is almost always inside `qs`, and
    /// a panel that only answered for the block itself would be blank exactly
    /// when it is wanted. How far up is looked is each reader's own, and is in
    /// [`Unpacker::reach`].
    ///
    /// The nearest packed structure answers first. A reader that turns out to
    /// have nothing to say about the bytes, which only a ggml block of a
    /// layout nobody here unpacks does, lets the walk carry on outward.
    fn explain_packed<S: Source>(&mut self, doc: &Document<S>, path: &[usize], at_bits: Option<u64>) -> R<Explain> {
        for len in (0..=path.len()).rev() {
            let at = &path[..len];
            self.resolve(doc, at)?;
            let r = self.memo.get(at).expect("resolved").clone();
            let Ty::Struct(def) = &r.ty else { continue };
            let Some(packing) = def.packed.as_deref() else { continue };
            let Some(unpacker) = Unpacker::of(packing) else { continue };
            if unpacker.reach() == Reach::Near && path.len() - len > 1 {
                continue;
            }
            let found = Packed { path, at, r: &r, at_bits };
            if let Some(explain) = self.side_reader(doc, unpacker, found)? {
                return Ok(explain);
            }
        }
        Ok(Explain::Plain)
    }

    /// The answer one side reader gives for the packed structure `found.at`.
    /// `None` only where the reader has nothing to say about these bytes and
    /// the structures further out should be asked instead.
    fn side_reader<S: Source>(&mut self, doc: &Document<S>, unpacker: Unpacker, found: Packed) -> R<Option<Explain>> {
        let Packed { path, at, r, at_bits } = found;
        Ok(Some(match unpacker {
            Unpacker::Xref => self.explain_xref(doc, at, r)?,
            Unpacker::ObjStm => self.explain_objstm(doc, at, r)?,
            Unpacker::Hdf5Chunk => self.explain_hdf5_chunk(doc, at, r)?,
            Unpacker::GwfVect => self.explain_gwf_vect(doc, at, r)?,
            Unpacker::SqliteRow => self.explain_sqlite_row(doc, at)?,
            Unpacker::ParquetPage => self.explain_parquet_page(doc, at)?,
            // The tile is found again from the cursor's own path, because
            // which tile it is depends on how far into the image the path goes.
            Unpacker::FitsTile => self.explain_fits_tile(doc, path)?,
            Unpacker::Mseed { encoding, big } => self.explain_mseed(doc, at, r, encoding, big)?,
            Unpacker::GribValues => self.explain_grib_values(doc, path, at, r)?,
            Unpacker::BufrData => self.explain_bufr(doc, at, at_bits)?,
            Unpacker::Quant => {
                let Some((kind, block, at_block)) = self.quant_block(doc, at)? else { return Ok(None) };
                quant_of(kind, block, at_block, at_bits)
            }
        }))
    }

    /// A miniSEED record's data, decoded into samples.
    ///
    /// The template said which encoding and byte order it laid the data out
    /// in, by the packing name, so the only thing looked up here is the sample
    /// count. It is a field of the record in both versions of the format,
    /// before the data, and found the way a template expression finds one.
    fn explain_mseed<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        r: &Resolved,
        encoding: u8,
        big: bool,
    ) -> R<Explain> {
        let bits = self.size_of(doc, at)?;
        let declared = self
            .find_field(at, "sample_count")
            .and_then(|p| self.node(doc, &p).ok())
            .and_then(|n| n.value.as_int())
            .unwrap_or(0)
            .max(0) as usize;
        // Over the limit, the bytes are not read at all.
        let record = if bits / 8 > mseed_steim::PAYLOAD_LIMIT as u64 {
            mseed_steim::refused((bits / 8) as usize, encoding, big, declared)
        } else {
            mseed_steim::decode(&self.read(doc, r, r.offset, bits)?, encoding, big, declared)
        };
        let shown = record.samples.len().min(MSEED_VALUES_SHOWN);
        let check = record.check();
        let mut steim = record.steim.clone();
        let frames_walked = steim.as_ref().map_or(0, |s| s.frames.len()) as u64;
        if let Some(s) = steim.as_mut() {
            s.frames.truncate(MSEED_FRAMES_SHOWN);
        }
        Ok(Explain::MseedSamples {
            encoding,
            encoding_name: mseed_steim::encoding_name(encoding),
            big_endian: big,
            declared: declared as u64,
            payload_bytes: record.payload_bytes as u64,
            steim,
            frames_walked,
            rule: mseed_steim::rule(encoding).map(str::to_string),
            values: (0..shown).map(|i| record.text(i)).collect(),
            last: (!record.samples.is_empty()).then(|| record.text(record.samples.len() - 1)),
            total: record.samples.len() as u64,
            check,
            problem: record.problem,
        })
    }

    /// The BUFR message whose section 4 is at `at`, read through the tables.
    ///
    /// The message is found by walking up to the node the template calls
    /// `Message`, and handed to [`bufr_data`] as bytes: every section it
    /// needs is inside it, and the reader finds them again itself. Where the
    /// cursor is, counted from the first bit of section 4's data, says which
    /// value to take apart.
    fn explain_bufr<S: Source>(&mut self, doc: &Document<S>, at: &[usize], at_bits: Option<u64>) -> R<Explain> {
        let mut message = at.to_vec();
        while self.node(doc, &message)?.type_name != "Message" {
            if message.pop().is_none() {
                return Ok(Explain::Plain);
            }
        }
        self.resolve(doc, &message)?;
        let r = self.memo.get(&message).expect("resolved").clone();
        let bits = self.size_of(doc, &message)?;
        if bits / 8 > bufr_data::MESSAGE_LIMIT as u64 {
            let mb = bufr_data::MESSAGE_LIMIT >> 20;
            let panel = bufr_data::Panel { problem: Some(format!("Not read: the message is over this viewer's {mb} MB limit.")), ..Default::default() };
            return Ok(Explain::BufrData(Box::new(panel)));
        }
        let bytes = self.read(doc, &r, r.offset, bits)?;
        let reading = bufr_data::read(&bytes);
        let data_start = r.offset + reading.header.data_offset as u64 * 8;
        let cursor = at_bits.and_then(|b| b.checked_sub(data_start));
        Ok(Explain::BufrData(Box::new(bufr_data::panel(&reading, cursor))))
    }

    /// The tile of a FITS compressed image that `path` is in, as a panel
    /// shows it: the steps, and the first of its pixels.
    fn explain_fits_tile<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Explain> {
        let Some(tile) = self.fits_tile(doc, path)? else { return Ok(Explain::Plain) };
        let shown = tile.pixels.len().min(fits_tile::VALUES_SHOWN);
        Ok(Explain::FitsTile {
            index: tile.index,
            tiles: tile.tiles,
            values: (0..shown).map(|i| tile.text(i)).collect(),
            total: tile.pixels.len() as u64,
            pixels: tile.pixel_count(),
            start: tile.start,
            shape: tile.shape,
            image_shape: tile.image_shape,
            algorithm: tile.algorithm,
            column: tile.stored.map(fits_tile::Stored::column),
            packed_bytes: tile.packed_bytes as u64,
            decoded_bytes: tile.decoded_bytes as u64,
            steps: tile.steps,
            element_type: tile.element_type,
            problem: tile.problem,
        })
    }

    /// The tile of a FITS compressed image that `path` is in, decompressed.
    /// `None` when `path` is not inside a compressed image's data.
    ///
    /// Which tile is read from where the path goes: a row is its own tile, and
    /// an array in the heap is the tile of the row whose descriptor placed it,
    /// which the gather knows. Anywhere else in the image, which is the fields
    /// that say what it is and cover no bytes, is the first tile.
    ///
    /// The header's cards, the row and the tile's bytes are handed to
    /// [`fits_tile`] as bytes, read from where the template put them. See that
    /// module for why the cards are read again there.
    pub fn fits_tile<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<fits_tile::Tile>> {
        for len in (1..=path.len()).rev() {
            let at = &path[..len];
            self.resolve(doc, at)?;
            let r = self.memo.get(at).expect("resolved").clone();
            let Ty::Struct(def) = &r.ty else { continue };
            if def.packed.as_deref() != Some(fits_tile::PACKING) {
                continue;
            }
            let rows = def.fields.iter().position(|f| &*f.name == "rows");
            let heap = def.fields.iter().position(|f| &*f.name == "heap");
            return self.fits_tile_in(doc, at, &r, path, rows, heap).map(Some);
        }
        Ok(None)
    }

    /// The tile `path` is in, of the compressed image at `at`.
    fn fits_tile_in<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        r: &Resolved,
        path: &[usize],
        rows: Option<usize>,
        heap: Option<usize>,
    ) -> R<fits_tile::Tile> {
        // The cards are the first field of the unit the data is in.
        let mut cards_path = at[..at.len() - 1].to_vec();
        cards_path.push(0);
        self.resolve(doc, &cards_path)?;
        let rc = self.memo.get(&cards_path).expect("resolved").clone();
        let cards_bits = self.size_of(doc, &cards_path)?.min(FITS_CARDS_LIMIT * 8);
        let cards = fits_tile::Cards::parse(&self.read(doc, &rc, rc.offset, cards_bits)?);
        let image = match fits_tile::Image::from_cards(&cards) {
            Ok(image) => image,
            Err(why) => return Ok(fits_tile::unreadable(why)),
        };

        let below = path.get(at.len()).copied();
        let next = path.get(at.len() + 1).copied();
        let mut index = match (below, next) {
            (Some(k), Some(i)) if Some(k) == rows => i as u64,
            (Some(k), Some(i)) if Some(k) == heap => {
                let mut list = at.to_vec();
                list.push(k);
                let record = self.gathered_record(doc, &list, i)?;
                record.get(at.len() + 1).copied().unwrap_or(0) as u64
            }
            _ => 0,
        };
        index = index.min(image.rows.saturating_sub(1));

        let row_bits = image.row_bytes as u64 * 8;
        let row_bytes = self.read(doc, r, r.offset + index * row_bits, row_bits)?;
        let row = image.row(&row_bytes);
        let Some(place) = row.place else { return Ok(fits_tile::decode(&image, index, &row, &[])) };
        let bytes = place.bytes();
        if bytes > fits_tile::COMPRESSED_LIMIT as u64 {
            let mb = fits_tile::COMPRESSED_LIMIT >> 20;
            let problem = format!("Not unpacked: the tile is over this viewer's {mb} MB limit.");
            return Ok(fits_tile::unread(&image, index, &row, bytes as usize, Some(problem)));
        }
        let from = r.offset + (image.heap_start + place.offset) * 8;
        match self.read(doc, r, from, bytes * 8) {
            Ok(data) => Ok(fits_tile::decode(&image, index, &row, &data)),
            Err(_) => {
                let heap_bytes = (self.size_of(doc, at)? / 8).saturating_sub(image.heap_start);
                let problem = format!(
                    "Not unpacked: the descriptor points past the end of the heap ({} bytes at offset {}; the heap is {} bytes).",
                    crate::encode::commas(bytes),
                    crate::encode::commas(place.offset),
                    crate::encode::commas(heap_bytes)
                );
                Ok(fits_tile::unread(&image, index, &row, bytes as usize, Some(problem)))
            }
        }
    }

    /// The block at `path` taken apart into its numbers, if it is one of the
    /// ggml packings this crate knows: the layout, the weights, and where the
    /// block starts in bits. `None` for everything else, which is every field
    /// that is not a block and every block whose weights only ggml can unpack.
    ///
    /// The value panel asks this for the block under the cursor and the value
    /// table asks it for every block over a screenful, and there is one answer
    /// to what a block holds.
    pub(super) fn quant_block<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
    ) -> R<Option<(Quant, ggml_quant::Block, u64)>> {
        self.resolve(doc, path)?;
        let r = self.memo.get(path).expect("resolved").clone();
        let Ty::Struct(def) = &r.ty else { return Ok(None) };
        let Some(packing) = def.packed.clone() else { return Ok(None) };
        let Some(kind) = ggml_quant::by_name(&packing) else { return Ok(None) };
        let bytes = self.read(doc, &r, r.offset, kind.block_bytes() as u64 * 8)?;
        Ok(ggml_quant::unpack(kind, &bytes).map(|b| (kind, b, r.offset)))
    }

    /// A row that spilled, followed onto the pages it spilled onto and read as
    /// the columns it holds.
    ///
    /// `at` is the payload, whose parent is the cell the row belongs to: the
    /// cell is where the row's declared length is written, and the length is
    /// what says when the chain has given up everything it owes.
    fn explain_sqlite_row<S: Source>(&mut self, doc: &Document<S>, at: &[usize]) -> R<Explain> {
        let Some((_, cell)) = at.split_last() else { return Ok(Explain::Plain) };
        let found = sqlite_overflow::payload(self, doc, cell)?;
        // The encoding the database header names, which an assembled row has
        // no way to look up for itself. A file that does not say reads as
        // UTF-8, which is what SQLite itself assumes.
        let encoding = self
            .child_named(doc, &[], "text_encoding")?
            .and_then(|p| self.node(doc, &p).ok())
            .and_then(|n| n.value.as_int())
            .unwrap_or(1);
        let row = sqlite_overflow::read(doc, &found, encoding);
        let on_page = found.extents.first().map(|e| e.len).unwrap_or(0);
        Ok(Explain::SqliteRow {
            declared: found.declared,
            found: found.found,
            on_page,
            chain_length: found.pages.len(),
            pages: found.pages.into_iter().take(CHAIN_SHOWN).collect(),
            columns: row.columns,
            total_columns: row.total,
            // The walk's own trouble first: columns that would not read are
            // usually a consequence of a chain that did not finish.
            problem: found.problem.or(row.problem),
        })
    }

    /// A page of a column chunk, read the whole way.
    ///
    /// Three things have to be found before the bytes mean anything, and they
    /// are in three different places. The page's own header says which kind of
    /// page it is, how many values it holds and which encoding they are in.
    /// The column chunk two levels up says the physical type and the codec.
    /// And the schema, at the far end of the file, says how deep the column
    /// sits, which is the whole of what decides whether a v1 page has levels
    /// in front of its values. See [`parquet_page`].
    fn explain_parquet_page<S: Source>(&mut self, doc: &Document<S>, at: &[usize]) -> R<Explain> {
        use crate::formats::parquet_page as pp;

        let empty = |problem: Option<String>| Explain::ParquetPage {
            packed_bytes: 0,
            decoded_bytes: 0,
            steps: Vec::new(),
            values: Vec::new(),
            total: 0,
            element_type: String::new(),
            problem,
        };
        // The page's three fields: the header, the type worked out from it,
        // and the payload.
        let mut header_fields = at.to_vec();
        header_fields.extend([0, 0]);
        let field = |ev: &mut Self, list: &[usize], id: i128| -> R<Option<i128>> {
            let Some(p) = ev.thrift_field(doc, list, id)? else { return Ok(None) };
            Ok(ev.node(doc, &p)?.value.as_int())
        };
        let page_type = field(self, &header_fields, 1)?.unwrap_or(0) as i64;
        // Which of the three sub-headers carries the value count and the
        // encoding depends on the kind of page, and so do the field numbers
        // inside it: a v2 header calls its encoding field 4 and the older one
        // calls it 2.
        let (sub, enc_id) = match page_type {
            pp::DICTIONARY_PAGE => (7, 2),
            pp::DATA_PAGE_V2 => (8, 4),
            _ => (5, 2),
        };
        let mut inner = Vec::new();
        if let Some(p) = self.thrift_field(doc, &header_fields, sub)? {
            inner = p;
            inner.push(0);
        }
        let mut header = pp::Header { page_type, is_compressed: true, ..pp::Header::default() };
        if !inner.is_empty() {
            header.num_values = field(self, &inner, 1)?.unwrap_or(0) as i64;
            header.encoding = field(self, &inner, enc_id)?.unwrap_or(0) as i64;
            if page_type == pp::DATA_PAGE_V2 {
                header.definition_levels_byte_length = field(self, &inner, 5)?.unwrap_or(0) as i64;
                header.repetition_levels_byte_length = field(self, &inner, 6)?.unwrap_or(0) as i64;
                // A boolean reads as 1 for true and 2 for false, and a field
                // nothing wrote is not there at all; the default is true.
                header.is_compressed = field(self, &inner, 7)? != Some(2);
            } else {
                header.definition_level_encoding = field(self, &inner, 3)?.unwrap_or(0) as i64;
                header.repetition_level_encoding = field(self, &inner, 4)?.unwrap_or(0) as i64;
            }
        }

        // The column chunk this page was placed by, found by walking back up
        // rather than by counting levels: the arrangement between the two is
        // the template's business and may change.
        let mut chunk = at.to_vec();
        loop {
            if self.node(doc, &chunk).map(|n| n.type_name == "ColumnChunk").unwrap_or(false) {
                break;
            }
            if chunk.pop().is_none() {
                return Ok(empty(Some("The page is not under a column chunk.".into())));
            }
        }
        let column_index = chunk.last().copied().unwrap_or(0);
        let mut chunk_fields = chunk.clone();
        chunk_fields.push(0);
        let mut meta = Vec::new();
        if let Some(p) = self.thrift_field(doc, &chunk_fields, 3)? {
            meta = p;
            meta.push(0);
        }
        let mut column = pp::Column::default();
        if !meta.is_empty() {
            column.physical = field(self, &meta, 1)?.unwrap_or(0) as i64;
            column.codec = field(self, &meta, 4)?.unwrap_or(0) as i64;
        }
        let (max_definition, max_repetition, type_length) = self.parquet_levels(doc, column_index)?;
        column.max_definition = max_definition;
        column.max_repetition = max_repetition;
        column.type_length = type_length;

        // The payload, which for a v2 page takes in the levels in front of the
        // packed part as well as the packed part itself.
        let mut payload = at.to_vec();
        payload.push(2);
        self.resolve(doc, &payload)?;
        let r = self.memo.get(&payload).expect("resolved").clone();
        let bits = self.size_of(doc, &payload)?;
        if bits / 8 > pp::PACKED_LIMIT as u64 {
            let mb = pp::PACKED_LIMIT / (1 << 20);
            return Ok(empty(Some(format!("Not unpacked: the page is over this viewer's {mb} MB limit."))));
        }
        let bytes = self.read(doc, &r, r.offset, bits)?;
        let page = pp::read(&bytes, &header, &column);
        Ok(Explain::ParquetPage {
            packed_bytes: page.packed_bytes as u64,
            decoded_bytes: page.decoded_bytes as u64,
            steps: page.steps,
            values: page.values,
            total: page.total,
            element_type: page.element_type,
            problem: page.problem,
        })
    }

    /// The path to the value of the Thrift field numbered `id` in the field
    /// list at `list`, or nothing when the writer left it out.
    ///
    /// A compact-protocol struct is a list of fields in whatever order the
    /// writer chose, each carrying its own number, so this is a search and not
    /// an index. Every field reads as four children: the header byte, the kind
    /// worked out from it, the number, and the value.
    fn thrift_field<S: Source>(&mut self, doc: &Document<S>, list: &[usize], id: i128) -> R<Option<Vec<usize>>> {
        let n = self.child_count(doc, list)?;
        for i in 0..n.min(256) {
            let mut at = list.to_vec();
            at.push(i as usize);
            let mut which = at.clone();
            which.push(2);
            if self.node(doc, &which).ok().and_then(|n| n.value.as_int()) == Some(id) {
                at.push(3);
                return Ok(Some(at));
            }
        }
        Ok(None)
    }

    /// How deep the `nth` column sits in the schema: its maximum definition
    /// level, its maximum repetition level, and the width of one value where
    /// the type has a fixed one.
    ///
    /// The schema is written depth first, root first, each element saying how
    /// many children follow it, and the leaves are the columns in the order
    /// the row groups list them. So the walk is the lookup: nothing names a
    /// column chunk's schema element, and matching on `path_in_schema` would
    /// be a guess, because a list's element is called `element` wherever it
    /// appears.
    ///
    /// A definition level counts every element along the path that is not
    /// REQUIRED; a repetition level counts the REPEATED ones. The root itself
    /// counts for neither.
    fn parquet_levels<S: Source>(&mut self, doc: &Document<S>, nth: usize) -> R<(u32, u32, i64)> {
        // footer -> FileMetaData -> fields -> the one numbered 2.
        let mut fields = vec![1usize, 0, 0];
        let mut list = match self.thrift_field(doc, &fields, 2)? {
            Some(p) => p,
            None => return Ok((0, 0, 0)),
        };
        // The list's elements, past the header fields Thrift writes in front
        // of them.
        let elems = match self.child_index(doc, &list, "elems")? {
            Some(i) => {
                list.push(i);
                list
            }
            None => return Ok((0, 0, 0)),
        };
        let n = self.child_count(doc, &elems)?.min(4096) as usize;
        // Every element's repetition type, how many children it has, and how
        // wide a fixed-length value of it is, read once.
        let mut read = Vec::with_capacity(n);
        for i in 0..n {
            let mut at = elems.clone();
            at.extend([i, 0]);
            let of = |ev: &mut Self, id: i128| -> R<i128> {
                let Some(p) = ev.thrift_field(doc, &at, id)? else { return Ok(-1) };
                Ok(ev.node(doc, &p)?.value.as_int().unwrap_or(-1))
            };
            read.push((of(self, 3)?, of(self, 5)?, of(self, 2)?));
        }
        fields.clear();

        // The walk: an element with no children is a column, and the ones
        // above it on the stack are what it is nested inside.
        let mut leaf = 0usize;
        let mut definition = 0u32;
        let mut repetition = 0u32;
        // How many children are still owed at each level above.
        let mut owed: Vec<i128> = Vec::new();
        let mut levels: Vec<(u32, u32)> = Vec::new();
        for (i, (repetition_type, children, length)) in read.iter().copied().enumerate() {
            if i > 0 {
                // OPTIONAL is 1 and REPEATED is 2; a missing field means
                // REQUIRED, which the footer may leave out.
                if repetition_type >= 1 {
                    definition += 1;
                }
                if repetition_type == 2 {
                    repetition += 1;
                }
            }
            if children > 0 {
                owed.push(children);
                levels.push((definition, repetition));
            } else {
                if i > 0 {
                    if leaf == nth {
                        return Ok((definition, repetition, length.max(0) as i64));
                    }
                    leaf += 1;
                }
                // Close off every parent this was the last child of.
                while let Some(left) = owed.last_mut() {
                    *left -= 1;
                    if *left > 0 {
                        let (d, r) = *levels.last().expect("a level per owed group");
                        definition = d;
                        repetition = r;
                        break;
                    }
                    owed.pop();
                    levels.pop();
                    match levels.last() {
                        Some((d, r)) => {
                            definition = *d;
                            repetition = *r;
                        }
                        None => {
                            definition = 0;
                            repetition = 0;
                        }
                    }
                }
            }
        }
        Ok((0, 0, 0))
    }

    /// The alignment records that start in the BGZF block the cursor is in,
    /// read by [`bam_records`] from the front of the file. `None` where the
    /// cursor is not in a BGZF block of the file itself.
    ///
    /// Not an [`Explain`] yet, because nothing draws one: the panel would be a
    /// list of steps and records, and it waits on the shared step list the
    /// other readers' panels are to move to. Until then this is how a test or
    /// an example asks.
    ///
    /// Any node inside the block answers, including the fields of the first
    /// block's header and the bytes a later block unpacks to, since those are
    /// where the cursor is. A BGZF file inside another stream is not walked:
    /// the reader reads the file, and that file is not this one.
    pub fn bam_block<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Option<crate::formats::bam_records::Block>> {
        use crate::formats::bam_records;
        for len in (1..=path.len()).rev() {
            let at = &path[..len];
            self.resolve(doc, at)?;
            let r = self.memo.get(at).expect("resolved").clone();
            let Ty::Struct(def) = &r.ty else { continue };
            if def.packed.as_deref() != Some(bam_records::PACKING) {
                continue;
            }
            if r.space != 0 {
                return Ok(None);
            }
            let block = r.offset / 8;
            let read = |at: u64, n: u64| self.read_in(doc, 0, at * 8, n * 8);
            return bam_records::records_in_block(read, doc.len_bytes(), block).map(Some);
        }
        Ok(None)
    }

    /// A chunk of a dataset, undone.
    ///
    /// Everything needed to undo it is written elsewhere in the object header:
    /// the filter pipeline message says which filters and in what order, the
    /// datatype message says how wide one element is, and the chunk's own
    /// b-tree entry says which filters were skipped for this chunk. All three
    /// are found the way the template finds them, by looking back through the
    /// messages this chunk hangs under.
    fn explain_hdf5_chunk<S: Source>(&mut self, doc: &Document<S>, at: &[usize], r: &Resolved) -> R<Explain> {
        let packed_bits = self.size_of(doc, at)?;
        let packed_bytes = packed_bits / 8;
        let filters = self.hdf5_filters(doc, at)?;
        let element_size = self.hdf5_datatype_part(doc, at, "size")?.unwrap_or(0).max(0) as usize;
        let mask = self
            .find_field(at, "filter_mask")
            .and_then(|p| self.node(doc, &p).ok())
            .and_then(|n| n.value.as_int())
            .unwrap_or(0)
            .max(0) as u32;

        let empty = |problem: Option<String>| Explain::Hdf5Chunk {
            packed_bytes,
            decoded_bytes: 0,
            steps: Vec::new(),
            values: Vec::new(),
            total: 0,
            element_type: String::new(),
            problem,
        };
        if packed_bytes as usize > hdf5_chunk::PACKED_LIMIT {
            let mb = hdf5_chunk::PACKED_LIMIT / (1 << 20);
            return Ok(empty(Some(format!("Not unpacked: the chunk is over this viewer's {mb} MB limit."))));
        }
        let bytes = self.read(doc, r, r.offset, packed_bits)?;
        let chunk = hdf5_chunk::decode(&bytes, &filters, mask, element_size);
        let (element_type, values, total) = self.hdf5_values(doc, at, &chunk.bytes, element_size)?;
        Ok(Explain::Hdf5Chunk {
            packed_bytes,
            decoded_bytes: chunk.bytes.len() as u64,
            steps: chunk.steps,
            values,
            total,
            element_type,
            problem: chunk.problem,
        })
    }

    /// The numbers of a packed IGWD frame vector, unpacked. See [`gwf_vect`].
    ///
    /// Everything needed is in the vector itself, in the fields before its
    /// data: `compress` for the scheme and the byte order, `type` for the
    /// width, `nData` for how many numbers, which zero suppression needs
    /// because its last word is padded. The steps are shaped as an HDF5
    /// chunk's are, since the two are the same kind of walk.
    fn explain_gwf_vect<S: Source>(&mut self, doc: &Document<S>, at: &[usize], r: &Resolved) -> R<Explain> {
        let packed_bits = self.size_of(doc, at)?;
        let packed_bytes = packed_bits / 8;
        let mut field = |name: &str| {
            self.find_field(at, name).and_then(|p| self.node(doc, &p).ok()).and_then(|n| n.value.as_int()).unwrap_or(-1)
        };
        let (compress, vect_type, n_data) = (field("compress"), field("type"), field("nData"));
        let clamp = |v: i128| v.clamp(0, i128::from(u16::MAX)) as u16;
        let (compress, vect_type, declared) = (clamp(compress), clamp(vect_type), n_data.max(0) as u64);
        let unpacked = if packed_bytes as usize > gwf_vect::PACKED_LIMIT {
            // Over the limit, the bytes are not read at all.
            gwf_vect::refused(packed_bytes as usize, compress)
        } else {
            gwf_vect::decode(&self.read(doc, r, r.offset, packed_bits)?, compress, vect_type, declared)
        };
        let (element_type, values, total) = gwf_vect::values(&unpacked.bytes, vect_type, unpacked.little);
        Ok(Explain::GwfVector {
            compress,
            little: unpacked.little,
            declared,
            packed_bytes,
            decoded_bytes: unpacked.bytes.len() as u64,
            steps: unpacked.steps,
            values,
            total,
            element_type,
            problem: unpacked.problem,
        })
    }

    /// A GRIB2 message's section 7 data, worked out into the values it stands
    /// for. See [`grib_values`].
    ///
    /// Section 7 has already copied in the numbers it needs to place its own
    /// runs, the group counts and the three tables' widths, so those are read
    /// from its fields. What a value is worth also takes section 5's reference
    /// value and its two scale factors, which nothing in section 7 depends on
    /// and nothing copies, so they are found the way the template finds the
    /// others: back through the sections to the nearest one with a packing
    /// template in it.
    ///
    /// `path` is the cursor, which says which value it is on, if any. The
    /// template is told apart by the fields the structure has: simple packing
    /// has a run of `values`, complex packing has `groups`, and spatial
    /// differencing puts `first_values` in front of them.
    fn explain_grib_values<S: Source>(
        &mut self,
        doc: &Document<S>,
        path: &[usize],
        at: &[usize],
        r: &Resolved,
    ) -> R<Explain> {
        let Ty::Struct(def) = &r.ty else { return Ok(Explain::Plain) };
        let index_of = |name: &str| def.fields.iter().position(|f| &*f.name == name);
        let (groups, first_values, simple_values) = (index_of("groups"), index_of("first_values"), index_of("values"));
        let template: u16 = match (groups, first_values) {
            (None, _) => 0,
            (Some(_), None) => 2,
            (Some(_), Some(_)) => 3,
        };

        let earlier = |ev: &mut Self, field: &[&str]| -> R<Option<Value>> {
            let field: Vec<String> = field.iter().map(|s| s.to_string()).collect();
            let Some(p) = ev.sibling_field_path(doc, at, &field)? else { return Ok(None) };
            Ok(Some(ev.node(doc, &p)?.value))
        };
        let int = |v: Option<Value>| v.and_then(|v| v.as_int()).unwrap_or(0);
        let reference = match earlier(self, &["body", "template", "reference_value"])? {
            Some(Value::Float(f)) => f as f32,
            _ => 0.0,
        };
        let binary_scale = int(earlier(self, &["body", "template", "binary_scale_factor"])?) as i32;
        let decimal_scale = int(earlier(self, &["body", "template", "decimal_scale_factor"])?) as i32;
        let missing_value_management =
            int(earlier(self, &["body", "template", "missing_value_management"])?).clamp(0, 255) as u32;
        let declared = int(earlier(self, &["body", "number_of_values"])?).max(0) as u64;
        let mut own = |name: &str| -> R<u32> {
            Ok(self.field_under(doc, at, name)?.unwrap_or(0).clamp(0, i128::from(u32::MAX)) as u32)
        };
        let packing = grib_values::Packing {
            reference,
            binary_scale,
            decimal_scale,
            missing_value_management,
            bits_per_value: own("bits_per_value")?,
            n_groups: own("n_groups")?,
            group_widths_reference: own("group_widths_reference")?,
            group_widths_bits: own("group_widths_bits")?,
            group_lengths_reference: own("group_lengths_reference")?,
            group_length_increment: own("group_length_increment")?,
            last_group_length: own("last_group_length")?,
            group_lengths_bits: own("group_lengths_bits")?,
            spatial_order: own("spatial_differencing_order")?,
            extra_bytes: own("extra_bytes")?,
        };
        let count = own("count")? as usize;
        let minimum = self.field_under(doc, at, "overall_minimum")?.map(|v| v as i64);

        let packed_bits = self.size_of(doc, at)?;
        let packed_bytes = packed_bits / 8;
        let reading = if packed_bytes > grib_values::PACKED_LIMIT as u64 {
            // Over the limit, the bytes are not read at all.
            grib_values::refused()
        } else {
            let bytes = self.read(doc, r, r.offset, packed_bits)?;
            if template == 0 { grib_values::simple(&packing, &bytes, count) } else { grib_values::complex(&packing, &bytes) }
        };

        // Which value the cursor is on, counted through the whole message:
        // simple packing's run is one list, the first values of spatial
        // differencing are the first of it, and a complex group's values
        // start where the groups before it left off.
        let rel = &path[at.len()..];
        let place = match rel {
            [v, i, ..] if simple_values == Some(*v) => Some((*i as u64, GribPlace::Values)),
            [f, k, ..] if first_values == Some(*f) => Some((*k as u64, GribPlace::First)),
            [g, n, v, i, ..] if groups == Some(*g) => {
                let mut group = at.to_vec();
                group.extend([*g, *n]);
                if self.child_index(doc, &group, "values")? == Some(*v) {
                    group.extend([*v, *i]);
                    let written = self.node(doc, &group)?.value.as_int().unwrap_or(0) as i64;
                    let mut before = 0u64;
                    for k in 0..*n {
                        let mut earlier_group = at.to_vec();
                        earlier_group.extend([*g, k]);
                        before += self.field_under(doc, &earlier_group, "count")?.unwrap_or(0).max(0) as u64;
                    }
                    let place = GribPlace::Group { group: *n as u64, position: *i as u64, written };
                    Some((before + *i as u64, place))
                } else {
                    None
                }
            }
            _ => None,
        };
        let at_value = place.and_then(|(index, place)| {
            let k = usize::try_from(index).ok()?;
            let value = packing.text(*reading.values.get(k)?);
            Some(GribValue { index, place, value, packed: *reading.packed.get(k)? })
        });

        Ok(Explain::GribValues {
            template,
            spatial_order: packing.spatial_order,
            reference: packing.reference_text(),
            binary_scale,
            decimal_scale,
            minimum,
            declared,
            packed_bytes,
            total: reading.values.len() as u64,
            values: reading.values.iter().take(grib_values::SHOWN).map(|v| packing.text(*v)).collect(),
            at: at_value,
            steps: reading.steps,
            problem: reading.problem,
        })
    }

    /// The filters this chunk's dataset was written through, in the order the
    /// pipeline message lists them, which is the order they were applied.
    fn hdf5_filters<S: Source>(&mut self, doc: &Document<S>, at: &[usize]) -> R<Vec<hdf5_chunk::Filter>> {
        let Some(list) = self.hdf5_message_part(doc, at, &["body", "filters"])? else {
            return Ok(Vec::new());
        };
        let n = self.child_count(doc, &list)?;
        let mut filters = Vec::new();
        for i in 0..n {
            let mut p = list.clone();
            p.push(i as usize);
            let id = self.field_under(doc, &p, "filter_id")?.unwrap_or(-1);
            if id < 0 {
                continue;
            }
            let mut client_data = Vec::new();
            if let Some(mut data) = self.child_index(doc, &p, "client_data")?.map(|j| {
                let mut q = p.clone();
                q.push(j);
                q
            }) {
                let count = self.child_count(doc, &data)?;
                for k in 0..count {
                    data.push(k as usize);
                    if let Some(v) = self.node(doc, &data)?.value.as_int() {
                        client_data.push(v.clamp(0, i128::from(u32::MAX)) as u32);
                    }
                    data.pop();
                }
            }
            filters.push(hdf5_chunk::Filter { id: id.clamp(0, i128::from(u16::MAX)) as u16, client_data });
        }
        Ok(filters)
    }

    /// One field of the datatype message that describes this chunk's elements.
    fn hdf5_datatype_part<S: Source>(&mut self, doc: &Document<S>, at: &[usize], name: &str) -> R<Option<i128>> {
        let Some(p) = self.hdf5_message_part(doc, at, &["body", name])? else { return Ok(None) };
        Ok(self.node(doc, &p)?.value.as_int())
    }

    /// The path to `field` inside the nearest earlier message of the list this
    /// chunk hangs under. Same search `Expr::Sibling` does, and for the same
    /// reason: what a chunk is made of is written in the messages beside the
    /// one that placed it.
    fn hdf5_message_part<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        field: &[&str],
    ) -> R<Option<Vec<usize>>> {
        let field: Vec<String> = field.iter().map(|s| s.to_string()).collect();
        let mut cur = at.to_vec();
        while let Some(idx) = cur.pop() {
            let listy = matches!(
                self.memo.get(&cur).map(|r| &r.ty),
                Some(Ty::Array { .. } | Ty::Repeat { .. } | Ty::PointerList { .. } | Ty::Chain { .. } | Ty::Gather { .. })
            );
            if !listy {
                continue;
            }
            for earlier in (0..idx).rev() {
                let mut p = cur.clone();
                p.push(earlier);
                if self.descend(doc, &mut p, &field)? {
                    return Ok(Some(p));
                }
            }
        }
        Ok(None)
    }

    /// A numeric field of a structure, by name.
    fn field_under<S: Source>(&mut self, doc: &Document<S>, at: &[usize], name: &str) -> R<Option<i128>> {
        let Some(j) = self.child_index(doc, at, name)? else { return Ok(None) };
        let mut p = at.to_vec();
        p.push(j);
        Ok(self.node(doc, &p)?.value.as_int())
    }

    /// The decoded bytes read as the dataset's elements, so the panel can show
    /// numbers rather than the bytes they are packed into.
    fn hdf5_values<S: Source>(
        &mut self,
        doc: &Document<S>,
        at: &[usize],
        bytes: &[u8],
        size: usize,
    ) -> R<(String, Vec<String>, u64)> {
        let class = self.hdf5_datatype_part(doc, at, "class")?.unwrap_or(-1);
        let bits = self.hdf5_datatype_part(doc, at, "bit_field")?.unwrap_or(0);
        let (big, signed) = (bits & 1 == 1, bits & 8 == 8);
        if size == 0 || bytes.is_empty() {
            return Ok((String::new(), Vec::new(), 0));
        }
        let endian = if big { crate::template::Endian::Big } else { crate::template::Endian::Little };
        let width = size as u32 * 8;
        let name = match (class, size, signed) {
            (0, _, true) => format!("i{width}"),
            (0, _, false) => format!("u{width}"),
            (1, 2, _) => "f16".to_string(),
            (1, 4, _) => "f32".to_string(),
            (1, 8, _) => "f64".to_string(),
            (3, _, _) => format!("{size}-byte text"),
            _ => format!("{size}-byte element"),
        };
        let total = (bytes.len() / size) as u64;
        let values = bytes
            .chunks_exact(size)
            .take(HDF5_VALUES_SHOWN)
            .map(|b| match (class, size) {
                (0, _) if signed => crate::decode::read_int(b, width, endian).to_string(),
                (0, _) => crate::decode::read_uint(b, width, endian).to_string(),
                (1, 2) => crate::decode::narrow_f16(crate::decode::read_uint(b, 16, endian) as u16).to_string(),
                (1, 4) => crate::decode::narrow_f32(f32::from_bits(crate::decode::read_uint(b, 32, endian) as u32)).to_string(),
                (1, 8) => f64::from_bits(crate::decode::read_uint(b, 64, endian) as u64).to_string(),
                (3, _) => String::from_utf8_lossy(b).trim_end_matches('\0').to_string(),
                _ => b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(""),
            })
            .collect();
        Ok((name, values, total))
    }

    /// A cross-reference stream taken apart. The dictionary beside the packed
    /// run says how, so both are read from the object the cursor is in.
    ///
    /// A run that will not decode is still an answer: what the dictionary said
    /// is worth showing next to the reason, since between them they are how a
    /// reader works out whether the file is odd or this is.
    fn explain_xref<S: Source>(&mut self, doc: &Document<S>, at: &[usize], r: &Resolved) -> R<Explain> {
        let Ty::Struct(def) = &r.ty else { return Ok(Explain::Plain) };
        let field = |name: &str| def.fields.iter().position(|f| &*f.name == name);
        let (Some(d), Some(p)) = (field("dictionary"), field("rows")) else { return Ok(Explain::Plain) };

        let mut dict_path = at.to_vec();
        dict_path.push(d);
        let Value::Str(dict) = self.node(doc, &dict_path)?.value else { return Ok(Explain::Plain) };

        let mut rows_path = at.to_vec();
        rows_path.push(p);
        self.resolve(doc, &rows_path)?;
        let packed_bits = self.size_of(doc, &rows_path)?;
        let rr = self.memo.get(&rows_path).expect("resolved").clone();
        let packed_bytes = packed_bits / 8;

        let answer = |problem: Option<String>, t: Option<pdf_xref::Table>| {
            let t = t.unwrap_or(pdf_xref::Table {
                rows: Vec::new(),
                widths: [0, 0, 0],
                predictor: None,
                decoded_bytes: 0,
                trailing_bytes: 0,
            });
            let mut counts = (0usize, 0usize, 0usize, 0usize);
            for row in &t.rows {
                match row.kind {
                    pdf_xref::Kind::Free => counts.0 += 1,
                    pdf_xref::Kind::InFile => counts.1 += 1,
                    pdf_xref::Kind::InStream => counts.2 += 1,
                    pdf_xref::Kind::Other(_) => counts.3 += 1,
                }
            }
            Explain::XrefRows {
                widths: t.widths,
                predictor: t.predictor,
                packed_bytes,
                decoded_bytes: t.decoded_bytes as u64,
                free: counts.0,
                in_file: counts.1,
                in_stream: counts.2,
                unknown: counts.3,
                total: t.rows.len(),
                rows: t.rows.into_iter().take(XREF_ROWS_SHOWN).collect(),
                problem,
            }
        };

        if packed_bytes > XREF_PACKED_LIMIT {
            let mb = XREF_PACKED_LIMIT / (1 << 20);
            let msg = format!("The compressed data is over the {mb} MB limit and was not decompressed.");
            return Ok(answer(Some(msg), None));
        }
        let bytes = self.read(doc, &rr, rr.offset, packed_bits)?;
        Ok(match pdf_xref::decode(&dict, &bytes) {
            Ok(t) => answer(None, Some(t)),
            Err(p) => answer(Some(p.as_str()), None),
        })
    }

    /// An object stream opened. Every object in the file arrives here, because
    /// only the dictionary inside one says whether it is an object stream, so
    /// the first thing this does is read enough of the body to find out and
    /// hand back `Plain` for the objects that are not.
    fn explain_objstm<S: Source>(&mut self, doc: &Document<S>, at: &[usize], r: &Resolved) -> R<Explain> {
        let Ty::Struct(def) = &r.ty else { return Ok(Explain::Plain) };
        let Some(b) = def.fields.iter().position(|f| &*f.name == "body") else { return Ok(Explain::Plain) };

        let mut body_path = at.to_vec();
        body_path.push(b);
        self.resolve(doc, &body_path)?;
        let body_bits = self.size_of(doc, &body_path)?;
        let br = self.memo.get(&body_path).expect("resolved").clone();

        // The dictionary and no more of the object than that. A body with no
        // `stream` keyword in its first few kilobytes is not an object stream,
        // and neither is one whose dictionary says it is something else.
        let head = self.read(doc, &br, br.offset, body_bits.min(OBJSTM_DICT_PREFIX * 8))?;
        let Some((dict, _)) = pdf_objstm::split_body(&head) else { return Ok(Explain::Plain) };
        if !pdf_objstm::is_object_stream(dict) {
            return Ok(Explain::Plain);
        }

        let packed_bytes = body_bits / 8;
        let answer = |problem: Option<String>, s: Option<pdf_objstm::Stream>| {
            let s = s.unwrap_or(pdf_objstm::Stream {
                objects: Vec::new(),
                claimed: 0,
                first: 0,
                decoded_bytes: 0,
                extends: None,
            });
            Explain::ObjStm {
                packed_bytes,
                decoded_bytes: s.decoded_bytes as u64,
                first: s.first as u64,
                extends: s.extends,
                total: s.objects.len(),
                objects: s.objects.into_iter().take(OBJSTM_SHOWN).collect(),
                problem,
            }
        };

        if packed_bytes > OBJSTM_PACKED_LIMIT {
            let mb = OBJSTM_PACKED_LIMIT / (1 << 20);
            let msg = format!("The compressed data is over the {mb} MB limit and was not decompressed.");
            return Ok(answer(Some(msg), None));
        }
        let body = self.read(doc, &br, br.offset, body_bits)?;
        let Some((dict, data)) = pdf_objstm::split_body(&body) else { return Ok(Explain::Plain) };
        Ok(match pdf_objstm::decode(dict, data) {
            Ok(s) => answer(None, Some(s)),
            Err(p) => answer(Some(p.as_str()), None),
        })
    }

    fn value_at<S: Source>(&mut self, doc: &Document<S>, path: &[usize]) -> R<Value> {
        Ok(self.node(doc, path)?.value)
    }
}

/// One unpacked block as an answer, with the cursor matched to the weight whose
/// bits it is inside.
fn quant_of(kind: Quant, block: ggml_quant::Block, block_bits: u64, at_bits: Option<u64>) -> Explain {
    // Either run of a split weight identifies it: the cursor is a bit, so the
    // one bit of `qh` a five-bit weight keeps there belongs to it and nothing
    // else.
    let holds = |p: &ggml_quant::Part, rel: u64| u64::from(p.bit) <= rel && rel < u64::from(p.bit + p.width);
    let at = at_bits.and_then(|c| c.checked_sub(block_bits)).and_then(|rel| {
        block
            .weights
            .iter()
            .position(|w| holds(&w.bits, rel) || w.high.is_some_and(|h| holds(&h, rel)))
    });
    Explain::Quant {
        kind: kind.name(),
        bits: kind.bits(),
        d: block.d,
        second: block.second,
        block_bits,
        groups: block.groups,
        group_weights: block.group_weights,
        bias: block.bias,
        signed: block.signed,
        weights: block.weights,
        at,
    }
}
