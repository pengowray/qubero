//! Why a field is what it is: the explain panel's answer and the parts it is built from.

use crate::*;

/// What a type permits, one shape per panel. `kind` says which, and each
/// variant carries only the fields its own panel reads.
///
/// It was one flat struct with every panel's fields side by side, prefixed so
/// they did not collide, and every panel added twenty more that every other
/// answer then carried as zeros. Tagged, a panel's fields are its own, and
/// the host's union type on `kind` stops a panel reading another's.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum ExplainDto {
    /// The bytes the format requires, and the bytes that are there.
    Magic { expected: Vec<u8>, actual: Vec<u8> },
    Enum {
        /// The type's own name.
        name: String,
        /// Every value it names, and the one in the file.
        cases: Vec<CaseDto>,
        current: f64,
        /// What the value in the file is called, where that name comes from a
        /// counted run rather than from `cases`. Empty when it has no name.
        named: String,
        /// Whether its numbers are read in hex.
        hex: bool,
    },
    Flags {
        name: String,
        /// The field's value, and one entry per bit of it from bit 0 up.
        current: f64,
        bits: Vec<BitDto>,
    },
    /// Which layout the float is, how many bits wide, and those bits in value
    /// order, written in hex because a 64-bit pattern does not survive a JSON
    /// number.
    Float { format: String, width: f64, pattern: String },
    /// A fixed-point number: its bits in hex, in value order, and where the
    /// binary point falls.
    Fixed { bits: f64, frac: f64, signed: bool, pattern: String },
    Quant {
        /// The block layout, as ggml's own struct is named, and how many bits
        /// one weight is worth.
        name: String,
        width: f64,
        /// The block's shared scale, and what it pairs with the scale, named
        /// as the file names it. Empty name where the layout has no second
        /// number.
        scale: f64,
        second_name: String,
        second: f64,
        /// Whether that second number is taken away rather than added, and
        /// whether it is multiplied by the group's own minimum first. Together
        /// with the group scales these say how a stored weight becomes a real
        /// one.
        second_subtract: bool,
        second_per_group: bool,
        /// Where the block starts, so a weight's bits can be found from the
        /// offset it carries.
        block_bits: f64,
        /// The scale the block keeps for each run of weights, where it keeps
        /// them, and how many weights one run covers. Empty for a block with
        /// one scale for all of them.
        groups: Vec<GroupDto>,
        group_weights: f64,
        /// Taken off the packed value to get the stored one, and whether that
        /// value is read signed instead of biased.
        bias: f64,
        signed: bool,
        /// Every weight the block stands for, in the order the tensor reads
        /// them, and which one the cursor is inside (-1 for none).
        weights: Vec<WeightDto>,
        at: f64,
    },
    Xref {
        /// The three widths from `/W`, and the PNG predictor where there was
        /// one, which is -1 where there was not.
        widths: Vec<f64>,
        predictor: f64,
        /// How many bytes the rows are in the file, and how many they came to
        /// once decompressed.
        packed: f64,
        decoded: f64,
        /// How many rows of each kind there are, over the whole table rather
        /// than over the ones listed.
        free: f64,
        in_file: f64,
        in_stream: f64,
        unknown: f64,
        /// The rows, and how many there are altogether. A table with more
        /// than `rows` holds says so with `total`.
        rows: Vec<XrefRowDto>,
        total: f64,
        /// Why there are no rows, where there are none. Empty otherwise.
        problem: String,
    },
    Objstm {
        /// How many bytes the objects are in the file, and how many they came
        /// to once decompressed.
        packed: f64,
        decoded: f64,
        /// The object number in `/Extends`, which is the object stream this
        /// one continues, or -1 where it continues none.
        extends: f64,
        /// The objects, and how many there are altogether.
        objects: Vec<ObjStmObjectDto>,
        total: f64,
        /// Why the stream would not open. Empty otherwise.
        problem: String,
    },
    Sqliterow {
        /// How many bytes the row claims, how many the chain reached, and how
        /// many of them stayed on the row's own page. A row that is whole has
        /// the first two equal.
        declared: f64,
        found: f64,
        on_page: f64,
        /// The overflow pages in the order the chain names them, and how many
        /// there are when that is more than the few listed.
        pages: Vec<f64>,
        chain: f64,
        /// The columns, and how many there are altogether.
        columns: Vec<SqliteColumnDto>,
        total_columns: f64,
        problem: String,
    },
    Chunk {
        /// How many bytes the chunk is in the file, and how many its elements
        /// came to once the filters were undone.
        packed: f64,
        decoded: f64,
        /// Each filter, in the order it was undone.
        steps: Vec<ChunkStepDto>,
        /// What one element is called, the first few elements, and how many
        /// there are altogether.
        element_type: String,
        values: Vec<String>,
        total: f64,
        problem: String,
    },
    Vector {
        /// The GWF vector's `compress` field as written, and whether its words
        /// were packed little-endian.
        compress: f64,
        little_endian: bool,
        /// How many numbers `nData` says the vector holds.
        declared: f64,
        /// How many bytes the packed run is in the file, and how many the
        /// numbers came to once unpacked.
        packed: f64,
        decoded: f64,
        /// Each step, in the order it was done.
        steps: Vec<VectorStepDto>,
        /// What one number is called, the first few, and how many came out.
        element_type: String,
        values: Vec<String>,
        total: f64,
        problem: String,
    },
    Grib {
        /// The data representation template, 0, 2 or 3, and for 3 whether the
        /// differencing is first or second order.
        template: f64,
        spatial_order: f64,
        /// R as text, and E and D, for the formula a value came out of.
        reference: String,
        binary_scale: f64,
        decimal_scale: f64,
        /// The overall minimum of the differences, or null without spatial
        /// differencing.
        minimum: Option<f64>,
        /// How many values section 5 says there are, and how many bytes
        /// section 7's data is.
        declared: f64,
        packed: f64,
        /// Every step, in the order it was done.
        steps: Vec<GribStepDto>,
        /// The first values, and how many came out.
        values: Vec<String>,
        total: f64,
        /// The value the cursor is on, or null where it is not on one.
        at: Option<GribValueDto>,
        problem: String,
    },
    /// A BUFR message's section 4 read through the tables. See
    /// `qubero_core::formats::bufr_data::Panel`.
    Bufr {
        edition: f64,
        /// The version section 1 names, and the version that was used.
        master_table_version: f64,
        tables_version: f64,
        subsets: f64,
        compressed: bool,
        steps: Vec<String>,
        /// Section 3's descriptors expanded through Table D, the first
        /// thousand or so, and how many there are.
        descriptors: Vec<BufrDescriptorDto>,
        descriptors_total: f64,
        /// Which subset `values` belong to, counted from 0, where in that
        /// subset's values the list starts, and how many values it has.
        subset: f64,
        values: Vec<BufrValueDto>,
        values_start: f64,
        values_total: f64,
        /// The value under the cursor, or null where the cursor is on none.
        cursor: Option<BufrCursorDto>,
        problem: String,
    },
    Page {
        /// How many bytes the payload is in the file, and how many its values
        /// came to once the codec was undone.
        packed: f64,
        decoded: f64,
        /// Every step, in the order it was done.
        steps: Vec<PageStepDto>,
        /// What one value is called, the first few of them, and how many there
        /// are altogether.
        element_type: String,
        values: Vec<String>,
        total: f64,
        problem: String,
    },
    Samples {
        /// A miniSEED record's encoding, by name and number, and whether its
        /// data was laid out big-endian.
        encoding: String,
        encoding_number: f64,
        big_endian: bool,
        /// How many samples the header gives, and how many bytes of data they
        /// were decoded from.
        declared: f64,
        bytes: f64,
        /// The Steim steps, where the record is Steim. `steim` says whether it
        /// is; the constants are null otherwise, and the first difference is
        /// null too for a record with no samples.
        steim: bool,
        x0: Option<f64>,
        xn: Option<f64>,
        first_difference: Option<f64>,
        /// What each frame held and gave, the first few hundred of them, how
        /// many frames were walked, and how many the data has room for.
        frames: Vec<MseedFrameDto>,
        frames_walked: f64,
        frames_in_record: f64,
        /// The rule a gain-ranged encoding is decoded by. Empty otherwise.
        rule: String,
        /// The first few, the last, and how many were decoded.
        values: Vec<String>,
        last: String,
        total: f64,
        /// Whether the last sample equals the reverse integration constant, or
        /// null where there is no check to make.
        check: Option<bool>,
        problem: String,
    },
    Tile {
        /// Which tile of a FITS compressed image, counted from 0 as its row
        /// is, of how many (null for more than a u64 counts); where it starts
        /// in the image, from 0 along each axis with the first axis first; how
        /// many pixels along each; and the image's shape.
        index: f64,
        count: Option<f64>,
        start: Vec<f64>,
        shape: Vec<f64>,
        image_shape: Vec<f64>,
        /// `ZCMPTYPE`, and the column the bytes were read from, or empty.
        algorithm: String,
        column: String,
        /// Bytes in the heap, and bytes once decompressed.
        packed: f64,
        decoded: f64,
        /// Each step in order. `skipped` is never set.
        steps: Vec<PageStepDto>,
        /// The first pixels, how many were decoded, how many the tile has
        /// (null for more than a u64 counts), and what one pixel is.
        values: Vec<String>,
        total: f64,
        pixels: Option<f64>,
        element_type: String,
        problem: String,
    },
    /// The type has nothing to add.
    Plain,
}

/// One run of weights inside a block that share a scale of their own.
#[derive(Serialize)]
pub(crate) struct GroupDto {
    /// The scale as stored, after whatever bias the type takes off it.
    pub(crate) scale: f64,
    /// The minimum taken off every weight in the run, or null where the type
    /// has none.
    pub(crate) min: Option<f64>,
}

/// One weight of a packed block.
#[derive(Serialize)]
pub(crate) struct WeightDto {
    /// The stored integer, after whatever bias the layout takes off it.
    pub(crate) q: f64,
    /// That integer through the block's scale: the number the model reads.
    pub(crate) value: f64,
    /// The run holding its low bits, and the rest of the packed value where the
    /// layout keeps that somewhere else in the block.
    pub(crate) bits: PartDto,
    pub(crate) high: Option<PartDto>,
}

/// One run of bits that makes up part of a packed weight.
#[derive(Serialize)]
pub(crate) struct PartDto {
    /// The block field these bits are in, as the file names it.
    pub(crate) field: String,
    /// Where they are, counted in bits from the start of the block.
    pub(crate) bit: f64,
    pub(crate) width: f64,
    /// Where they sit in the packed value: 0 for the low part.
    pub(crate) shift: f64,
}

/// One field another field's shape came from.
#[derive(Serialize)]
pub(crate) struct OriginDto {
    /// "length" | "count" | "type" | "position" | "points"
    pub(crate) role: &'static str,
    /// The field as the reader would name it: `len`, or `tensors[3].offset`.
    pub(crate) label: String,
    /// The same field named by every step the file stores on the way to it,
    /// where an encoding's own steps make that differ. Null otherwise.
    pub(crate) stored: Option<String>,
    /// Where it is, so the reader can go there. Empty for a bit this field
    /// points at rather than a field it came from.
    pub(crate) path: Vec<f64>,
    /// What it says, in brief. Empty when it could not be read.
    pub(crate) value: String,
    /// For "points": the bit this field's value points at.
    pub(crate) target_bits: Option<f64>,
}

/// The part of a joined stream a field starts in. See
/// [`qubero_core::eval::PartHit`].
#[derive(Serialize)]
pub(crate) struct StitchedPartDto {
    /// Which part, from 0, and how many there are.
    pub(crate) index: f64,
    pub(crate) parts: f64,
    /// The run the part is, as a path to go to and as a reader names it, and
    /// as every stored step names it where that differs (null otherwise).
    pub(crate) path: Vec<f64>,
    pub(crate) label: String,
    pub(crate) stored: Option<String>,
    /// The field's first byte inside what the part gives, and how much that is.
    pub(crate) in_part: f64,
    pub(crate) part_len: f64,
    /// Where the run starts, in the space it is a field of: 0 is the file.
    pub(crate) run_offset_bits: f64,
    pub(crate) run_space: f64,
    /// True when the run was unpacked to give the part.
    pub(crate) packed: bool,
    /// For a BGZF block: the two halves of the byte's virtual offset, the
    /// block's place in the file and the byte in what it unpacks to.
    pub(crate) block_offset: Option<f64>,
    pub(crate) in_block: Option<f64>,
}

pub(crate) fn stitched_part_dto(h: qubero_core::eval::PartHit) -> StitchedPartDto {
    StitchedPartDto {
        index: h.index as f64,
        parts: h.parts as f64,
        path: h.path.iter().map(|&x| x as f64).collect(),
        label: h.label,
        stored: h.stored,
        in_part: h.in_part as f64,
        part_len: h.part_len as f64,
        run_offset_bits: h.run_offset_bits as f64,
        run_space: h.run_space as f64,
        packed: h.packed,
        // Past 2^53 a number stops being exact in JavaScript, and a virtual
        // offset reaches that at a block eight petabytes into the file. The
        // halves are what a reader checks against an index anyway.
        block_offset: h.virtual_offset.map(|v| (v >> 16) as f64),
        in_block: h.virtual_offset.map(|v| (v & 0xffff) as f64),
    }
}

/// How a field was placed and how it was sized, in one word each. What the
/// panel says before any other field is named: most fields are placed and
/// sized by the template alone and have no origins at all, and a section that
/// answers only when another field is involved answers for almost nothing.
#[derive(Serialize)]
pub(crate) struct ShapeDto {
    /// "root" | "first" | "follows" | "element" | "pointer" | "chain" |
    /// "address" | "trace" | "stream" | "unknown"
    pub(crate) placed: &'static str,
    /// "fixed" | "expression" | "terminated" | "remaining" | "children" |
    /// "scattered" | "count" | "encoded" | "trace" | "nothing" | "unknown"
    pub(crate) sized: &'static str,
}

/// What a field checks, and how much of the file that is. No bytes were read
/// to answer it, which is what lets a panel ask on every move of the cursor.
#[derive(Serialize)]
pub(crate) struct CheckDto {
    /// "crc32" | "crc16" | "sum8" | "sum" | "sha1" | "adler32"
    pub(crate) algorithm: &'static str,
    /// The bytes summed, as [offset, length], when they are a run of the file.
    pub(crate) over: Option<[f64; 2]>,
    /// The compressed run whose contents are summed, as [offset, length], when
    /// the summed bytes are nowhere in the file.
    pub(crate) unpacked_from: Option<[f64; 2]>,
    /// How many bytes the sum is over. Real when `covered_exact` is true;
    /// otherwise a stand-in reached for because the true count would take
    /// decoding, which this call must not do. See
    /// [`covered_exact`](CheckDto::covered_exact).
    pub(crate) covered_bytes: f64,
    /// Whether `covered_bytes` really is the number of bytes summed. False
    /// only for an unpacked run whose length nothing here has said yet: no
    /// declared length ([`Covers::Unpacked`](qubero_core::template::Covers::Unpacked)
    /// with none written down) or one member of a run
    /// ([`Covers::UnpackedMember`](qubero_core::template::Covers::UnpackedMember))
    /// whose stream nothing has opened. A view must not print `covered_bytes`
    /// as the covered count while this is false.
    pub(crate) covered_exact: bool,
    /// Whether the sum is over one member's share of an unpacked run — an xz
    /// block's own check — rather than the whole of what the run unpacks to.
    pub(crate) unpacked_member: bool,
    /// The check field's own bytes, as [offset, length, byte], when the sum is
    /// over a record the field sits inside and they are read as something else
    /// while it runs. A tar header is summed with its checksum read as spaces.
    pub(crate) blanked: Option<[f64; 3]>,
}

/// What table a field reads as, with the template's expressions worked out in
/// the file at hand. See [`qubero_core::eval::TableShapeInfo`].
#[derive(Serialize)]
pub(crate) struct TableShapeDto {
    /// Elements per row. Null when one element is one row, and null as well
    /// when the template said how many but this file does not answer.
    pub(crate) columns: Option<f64>,
    /// What the columns are called, used when there are exactly this many.
    pub(crate) names: Vec<String>,
    /// The units the columns are measured in, as UCUM codes, parallel to
    /// `names`. An empty string for a column with no unit.
    pub(crate) units: Vec<String>,
    /// What to call a column that `names` does not reach: "channel" gives
    /// "channel 1", "channel 2". Null when the format has no word for one.
    pub(crate) column_word: Option<String>,
    /// What one row is: "sample", "record". Null when the format has no word.
    pub(crate) row_word: Option<String>,
    /// Rows per second, when the rows are spaced in time. Null otherwise.
    pub(crate) rate: Option<f64>,
    /// The fields that describe the table, to be shown above it with links to
    /// where they are stored.
    pub(crate) facts: Vec<TableFactDto>,
    /// Where a row's cells are, for a table whose rows are nodes rather than a
    /// run of values. Null for every table a template declares, where a row is
    /// `columns` values and a reader works the rest out from the count.
    pub(crate) cells: Option<CellsDto>,
}

/// Where a table's cells are, for a table whose rows are nodes rather than a
/// run of values. See [`qubero_core::template::Cells`].
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum CellsDto {
    /// Named nodes inside named rows: a pickled list of records, whose keys
    /// are written beside its values.
    Named {
        /// The type a node under the field has to be to be a row.
        row: String,
        /// The type a node inside a row has to be to be a cell, named by its
        /// column.
        cell: String,
        /// The field inside a cell holding what it is worth, for a cell that
        /// is a name and a value. Null when the cell is the value.
        value: Option<String>,
    },
    /// Cells the core works out, a window of rows at a time, which is a pandas
    /// frame. `rows` is how many there are; `Editor::pickle_cells` reads them.
    Computed { rows: f64 },
}

/// One cell of a table the core works out. `text` is empty and `kind` is
/// "absent" for a cell the file has no value for, which is the same nothing a
/// record without one of the table's keys shows.
///
/// `at` is where the cell's own bytes are. A row of one of these tables is not
/// a run of the file: a frame's row is one value out of each of several
/// blocks, so the address belongs to the cell and not to the row.
///
/// `masked` is the other nothing: a masked array's mask hides this entry, so
/// the cell shows nothing although the number is in the file at the address it
/// carries. Two different facts, so two fields rather than a third `kind`.
#[derive(Serialize)]
pub(crate) struct FrameCellDto {
    pub(crate) text: String,
    pub(crate) kind: &'static str,
    pub(crate) at: CellAtDto,
    pub(crate) masked: bool,
}

/// Where a cell's bytes are, or why it has none. See
/// [`qubero_core::eval::CellAt`].
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum CellAtDto {
    /// A run of the address space `space`, counted the way every other address
    /// in that space is. 0 is the file.
    Bytes { space: f64, offset_bits: f64, size_bits: f64 },
    /// Counted out rather than written down, which is a `RangeIndex` label.
    Counted,
    /// Nowhere this reading can point at.
    Nowhere,
}

impl From<CellAt> for CellAtDto {
    fn from(at: CellAt) -> Self {
        match at {
            CellAt::Bytes { space, offset_bits, size_bits } => {
                CellAtDto::Bytes { space: space as f64, offset_bits: offset_bits as f64, size_bits: size_bits as f64 }
            }
            CellAt::Counted => CellAtDto::Counted,
            CellAt::Nowhere => CellAtDto::Nowhere,
        }
    }
}

/// One field that describes a table: what it is called, where it is, and what
/// it says.
#[derive(Serialize)]
pub(crate) struct TableFactDto {
    /// The field as the reader would name it: `body.sample_rate`.
    pub(crate) label: String,
    /// Where it is, so the reader can go there. Empty when it is nowhere this
    /// reading can point at.
    pub(crate) path: Vec<f64>,
    /// What it says, in brief. Empty when it could not be read.
    pub(crate) value: String,
}

/// The moment a field means, once the template's epoch has been applied to the
/// number in it. The number itself is untouched and stays on the value row.
#[derive(Serialize)]
pub(crate) struct TimeDto {
    /// "at" | "leap" | "unset" | "impossible". `leap` is an instant inside a
    /// leap second, second 60 of a minute. `unset` is the value a format writes
    /// when it has no time to record, and `impossible` is a number that names
    /// no moment: a year outside 1 to 9999, or a packed date that is not a
    /// date.
    pub(crate) state: &'static str,
    /// Seconds from 1970-01-01T00:00:00Z, negative before it. Only for "at" and
    /// "leap"; for "leap" it is second 59 of the minute, and the moment is the
    /// second after it, which a Unix count has no number for.
    ///
    /// Well inside what an f64 holds exactly: the core refuses anything outside
    /// year 1 to year 9999, which is at most 2.5e11.
    pub(crate) unix_seconds: Option<f64>,
    /// The sub-second part, in nanoseconds, always 0 to 999,999,999 and never
    /// negative. Only for "at" and "leap".
    pub(crate) nanos: Option<f64>,
    /// "past_leap_second_table" | "before_leap_seconds" | null. What has to be
    /// shown beside a moment that is right as far as it goes: a count on a
    /// clock with leap seconds that falls after the last day the table of them
    /// vouches for, or before 1972, when UTC had none.
    pub(crate) note: Option<&'static str>,
    /// The first day the leap-second table does not vouch for, as seconds from
    /// 1970-01-01T00:00:00Z. Only with the first of those notes, so the words
    /// can name the day.
    pub(crate) leap_table_expires: Option<f64>,
    /// "utc" | "local" | "unknown". For the last two the seconds above are the
    /// digits the file wrote laid on the UTC line, so an interface prints them
    /// unchanged and says which this was. Shifting them into the reader's own
    /// zone would be inventing one the file never recorded.
    pub(crate) zone: &'static str,
    /// The smallest step the field can express, in nanoseconds, so an interface
    /// prints the decimal places the file actually has and no more: 1e9 for a
    /// field counting seconds, 1e3 for a journal's microseconds, 100 for a
    /// FILETIME, 2e9 for an MS-DOS time, which counts seconds in twos.
    pub(crate) step_nanos: f64,
}

/// What came of taking a checksum: the two forms, printed to the algorithm's
/// own width so they can be compared and shown side by side.
#[derive(Serialize)]
pub(crate) struct VerdictDto {
    pub(crate) computed: String,
    pub(crate) stored: String,
    pub(crate) ok: bool,
}

/// One relationship behind a field's shape, written both ways.
#[derive(Serialize)]
pub(crate) struct RelationDto {
    /// "length" | "count" | "type" | "value" | "position"
    pub(crate) role: &'static str,
    /// The expression as the template writes it, with a field reached through
    /// an encoding's own steps named the way the format names it.
    pub(crate) written: String,
    /// The expression exactly as the template writes it, where that differs
    /// from `written`. Null otherwise.
    pub(crate) template: Option<String>,
    /// The same with every field's value in its place.
    pub(crate) substituted: String,
    /// What it comes to.
    pub(crate) result: String,
}

/// One Huffman-coded number of a deflate symbol: the symbol, how wide the code
/// that carried it was, the bits that followed it outright, and what the two
/// came to. See `qubero_core::codec::inflate::DecodedCode`.
#[derive(Serialize)]
pub(crate) struct DecodedCodeDto {
    pub(crate) symbol: f64,
    /// How wide the Huffman code was. Worked out again from the block's table
    /// rather than read from the trace, which does not keep it.
    pub(crate) code_bits: f64,
    /// How many bits followed the code, and what they said. Both zero for a
    /// literal, for the end mark, and for the lengths and distances a symbol
    /// names on its own.
    pub(crate) extra_bits: f64,
    pub(crate) extra: f64,
    /// The byte, the length, or the distance, depending which code this is.
    pub(crate) value: f64,
    /// Which step of the trace set this symbol's code length, so the panel can
    /// send a reader to it. Absent for a fixed block, whose tables are in RFC
    /// 1951 rather than in the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) entry: Option<f64>,
    /// The same step counted from the front of its block, which is where it
    /// sits among the block node's children.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) entry_child: Option<f64>,
}

/// What one deflate symbol was made of.
#[derive(Serialize)]
pub(crate) struct DecodedStepDto {
    /// "literal", "end-of-block" or "match", the same words `StepKind::as_str`
    /// gives the rest of the interface.
    pub(crate) kind: &'static str,
    /// Which block of the trace it belongs to, as an index.
    pub(crate) block: f64,
    /// "fixed" or "dynamic": where the two tables came from.
    pub(crate) block_kind: &'static str,
    /// The literal/length code, which every symbol begins with.
    pub(crate) symbol: DecodedCodeDto,
    /// The distance code, for a match and for nothing else.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) distance: Option<DecodedCodeDto>,
    /// The four widths added up, which is the step's own width in bits. Given
    /// rather than left to be summed so a view can show the arithmetic and its
    /// answer without doing the arithmetic itself.
    pub(crate) bits: f64,
}

pub(crate) fn decoded_code_dto(c: qubero_core::codec::inflate::DecodedCode) -> DecodedCodeDto {
    DecodedCodeDto {
        symbol: c.symbol as f64,
        code_bits: c.code_bits as f64,
        extra_bits: c.extra_bits as f64,
        extra: c.extra as f64,
        value: c.value as f64,
        entry: c.entry.map(|k| k as f64),
        entry_child: c.entry_child.map(|k| k as f64),
    }
}

pub(crate) fn decoded_step_dto(d: qubero_core::codec::inflate::DecodedStep) -> DecodedStepDto {
    use qubero_core::codec::inflate::SymbolMeaning;
    DecodedStepDto {
        // Through `StepKind::as_str` rather than three string literals, so a
        // step named here is named the same as the step the cursor link
        // already showed for the same bits. The values in the kinds are not
        // used and are not read: what is wanted is the word.
        kind: match d.meaning() {
            SymbolMeaning::Literal(_) => StepKind::Literal(0).as_str(),
            SymbolMeaning::EndOfBlock => StepKind::EndOfBlock.as_str(),
            SymbolMeaning::Length => StepKind::Match { len: 0, dist: 0 }.as_str(),
        },
        block: d.block as f64,
        block_kind: d.block_kind.as_str(),
        symbol: decoded_code_dto(d.symbol),
        distance: d.distance.map(decoded_code_dto),
        bits: d.bits() as f64,
    }
}

#[derive(Serialize)]
pub(crate) struct CaseDto {
    pub(crate) value: f64,
    pub(crate) name: String,
}

#[derive(Serialize)]
pub(crate) struct BitDto {
    pub(crate) bit: u32,
    /// Absent for a bit the format does not name.
    pub(crate) name: Option<String>,
    pub(crate) set: bool,
}

pub(crate) fn origin_dto(o: Origin) -> OriginDto {
    OriginDto {
        role: o.role.as_str(),
        label: o.label,
        stored: o.stored,
        path: o.path.into_iter().map(|x| x as f64).collect(),
        value: o.value,
        target_bits: o.target_bits.map(|b| b as f64),
    }
}

pub(crate) fn part_dto(p: qubero_core::formats::ggml_quant::Part) -> PartDto {
    PartDto { field: p.field.to_string(), bit: f64::from(p.bit), width: f64::from(p.width), shift: f64::from(p.shift) }
}

pub(crate) fn explain_dto(e: Explain) -> ExplainDto {
    let floats = |v: Vec<u64>| v.into_iter().map(|n| n as f64).collect();
    match e {
        Explain::Plain => ExplainDto::Plain,
        Explain::Magic { expected, actual } => ExplainDto::Magic { expected, actual },
        Explain::Enum { name, hex, cases, current, named } => ExplainDto::Enum {
            name,
            hex,
            current: current as f64,
            cases: cases.into_iter().map(|(value, name)| CaseDto { value: value as f64, name }).collect(),
            named: named.unwrap_or_default(),
        },
        Explain::Quant { kind, bits, d, second, block_bits, groups, group_weights, bias, signed, weights, at } => {
            ExplainDto::Quant {
                name: kind.to_string(),
                width: f64::from(bits),
                scale: d,
                second_name: second.as_ref().map(|o| o.name.to_string()).unwrap_or_default(),
                second: second.as_ref().map_or(0.0, |o| o.value),
                second_subtract: second.as_ref().is_some_and(|o| o.subtract),
                second_per_group: second.as_ref().is_some_and(|o| o.per_group),
                block_bits: block_bits as f64,
                group_weights: f64::from(group_weights),
                bias: f64::from(bias),
                signed,
                groups: groups
                    .into_iter()
                    .map(|g| GroupDto { scale: f64::from(g.scale), min: g.min.map(f64::from) })
                    .collect(),
                at: at.map_or(-1.0, |i| i as f64),
                weights: weights
                    .into_iter()
                    .map(|w| WeightDto {
                        q: f64::from(w.q),
                        value: w.value,
                        bits: part_dto(w.bits),
                        high: w.high.map(part_dto),
                    })
                    .collect(),
            }
        }
        Explain::XrefRows {
            widths,
            predictor,
            packed_bytes,
            decoded_bytes,
            free,
            in_file,
            in_stream,
            unknown,
            rows,
            total,
            problem,
        } => {
            use qubero_core::formats::pdf_xref::Kind;
            ExplainDto::Xref {
                widths: widths.iter().map(|w| f64::from(*w)).collect(),
                predictor: predictor.map_or(-1.0, f64::from),
                packed: packed_bytes as f64,
                decoded: decoded_bytes as f64,
                free: free as f64,
                in_file: in_file as f64,
                in_stream: in_stream as f64,
                unknown: unknown as f64,
                total: total as f64,
                problem: problem.unwrap_or_default(),
                rows: rows
                    .into_iter()
                    .map(|r| XrefRowDto {
                        object: r.object as f64,
                        kind: r.kind.as_str(),
                        type_raw: r.kind.raw() as f64,
                        offset: if r.kind == Kind::InFile { r.second as f64 } else { -1.0 },
                        second: r.second as f64,
                        third: r.third as f64,
                    })
                    .collect(),
            }
        }
        Explain::ObjStm { packed_bytes, decoded_bytes, extends, objects, total, problem, .. } => ExplainDto::Objstm {
            packed: packed_bytes as f64,
            decoded: decoded_bytes as f64,
            extends: extends.map_or(-1.0, |n| n as f64),
            total: total as f64,
            problem: problem.unwrap_or_default(),
            objects: objects
                .into_iter()
                .map(|o| ObjStmObjectDto { number: o.number as f64, len: o.len as f64, text: o.text, cut: o.cut })
                .collect(),
        },
        Explain::SqliteRow { declared, found, on_page, pages, chain_length, columns, total_columns, problem } => {
            ExplainDto::Sqliterow {
                declared: declared as f64,
                found: found as f64,
                on_page: on_page as f64,
                pages: pages.into_iter().map(|p| p as f64).collect(),
                chain: chain_length as f64,
                total_columns: total_columns as f64,
                problem: problem.unwrap_or_default(),
                columns: columns
                    .into_iter()
                    .map(|c| {
                        let (value_kind, value, _) = shown(&c.value);
                        SqliteColumnDto { type_name: c.type_name, value, value_kind, at: c.at as f64, len: c.len as f64 }
                    })
                    .collect(),
            }
        }
        Explain::FitsTile {
            index,
            tiles,
            start,
            shape,
            image_shape,
            algorithm,
            column,
            packed_bytes,
            decoded_bytes,
            steps,
            values,
            total,
            pixels,
            element_type,
            problem,
        } => ExplainDto::Tile {
            index: index as f64,
            count: tiles.map(|n| n as f64),
            start: floats(start),
            shape: floats(shape),
            image_shape: floats(image_shape),
            algorithm,
            column: column.unwrap_or_default().to_string(),
            packed: packed_bytes as f64,
            decoded: decoded_bytes as f64,
            values,
            total: total as f64,
            pixels: pixels.map(|n| n as f64),
            element_type,
            problem: problem.unwrap_or_default(),
            steps: steps
                .into_iter()
                .map(|s| PageStepDto {
                    what: s.what,
                    in_bytes: s.in_bytes as f64,
                    out_bytes: s.out_bytes as f64,
                    note: s.note,
                    skipped: false,
                })
                .collect(),
        },
        Explain::ParquetPage { packed_bytes, decoded_bytes, steps, values, total, element_type, problem } => ExplainDto::Page {
            packed: packed_bytes as f64,
            decoded: decoded_bytes as f64,
            total: total as f64,
            element_type,
            values,
            problem: problem.unwrap_or_default(),
            steps: steps
                .into_iter()
                .map(|s| PageStepDto {
                    what: s.what,
                    in_bytes: s.in_bytes as f64,
                    out_bytes: s.out_bytes as f64,
                    note: s.note,
                    skipped: s.skipped,
                })
                .collect(),
        },
        Explain::Hdf5Chunk { packed_bytes, decoded_bytes, steps, values, total, element_type, problem } => ExplainDto::Chunk {
            packed: packed_bytes as f64,
            decoded: decoded_bytes as f64,
            total: total as f64,
            element_type,
            values,
            problem: problem.unwrap_or_default(),
            steps: chunk_steps(steps),
        },
        Explain::BufrData(p) => {
            let p = *p;
            ExplainDto::Bufr {
                edition: f64::from(p.edition),
                master_table_version: f64::from(p.master_table_version),
                tables_version: f64::from(p.tables_version),
                subsets: f64::from(p.subsets),
                compressed: p.compressed,
                steps: p.steps,
                descriptors: p
                    .descriptors
                    .into_iter()
                    .map(|d| BufrDescriptorDto { code: f64::from(d.code), depth: f64::from(d.depth), name: d.name })
                    .collect(),
                descriptors_total: p.descriptors_total as f64,
                subset: f64::from(p.subset),
                values: p.values.into_iter().map(bufr_value_dto).collect(),
                values_start: p.values_start as f64,
                values_total: p.values_total as f64,
                cursor: p.cursor.map(|c| BufrCursorDto {
                    index: c.index.map_or(-1.0, |i| i as f64),
                    value: bufr_value_dto(c.value),
                    bit: c.bit as f64,
                    width: f64::from(c.width),
                    scale: f64::from(c.scale),
                    reference: c.reference as f64,
                    numeric: c.numeric,
                    packed: c.packed.map(|v| v as f64),
                    base: c.base.map(|v| v as f64),
                    increment_width: c.increment_width.map(f64::from),
                    across: c.across,
                }),
                problem: p.problem.unwrap_or_default(),
            }
        }
        Explain::GribValues {
            template,
            spatial_order,
            reference,
            binary_scale,
            decimal_scale,
            minimum,
            declared,
            packed_bytes,
            steps,
            values,
            total,
            at,
            problem,
        } => ExplainDto::Grib {
            template: f64::from(template),
            spatial_order: f64::from(spatial_order),
            reference,
            binary_scale: f64::from(binary_scale),
            decimal_scale: f64::from(decimal_scale),
            minimum: minimum.map(|m| m as f64),
            declared: declared as f64,
            packed: packed_bytes as f64,
            values,
            total: total as f64,
            problem: problem.unwrap_or_default(),
            steps: steps
                .into_iter()
                .map(|s| GribStepDto { label: s.label, what: s.what, count: s.count as f64 })
                .collect(),
            at: at.map(|v| {
                use qubero_core::eval::GribPlace;
                let (place, group, position, written) = match v.place {
                    GribPlace::Values => ("values", 0, 0, 0),
                    GribPlace::First => ("first", 0, 0, 0),
                    GribPlace::Group { group, position, written } => ("group", group, position, written),
                };
                GribValueDto {
                    index: v.index as f64,
                    place,
                    group: group as f64,
                    position: position as f64,
                    written: written as f64,
                    value: v.value,
                    packed: v.packed as f64,
                }
            }),
        },
        Explain::GwfVector {
            compress,
            little,
            declared,
            packed_bytes,
            decoded_bytes,
            steps,
            values,
            total,
            element_type,
            problem,
        } => ExplainDto::Vector {
            compress: f64::from(compress),
            little_endian: little,
            declared: declared as f64,
            packed: packed_bytes as f64,
            decoded: decoded_bytes as f64,
            total: total as f64,
            element_type,
            values,
            problem: problem.unwrap_or_default(),
            steps: steps
                .into_iter()
                .map(|s| VectorStepDto { what: s.filter, in_bytes: s.in_bytes as f64, out_bytes: s.out_bytes as f64 })
                .collect(),
        },
        Explain::MseedSamples {
            encoding,
            encoding_name,
            big_endian,
            declared,
            payload_bytes,
            steim,
            frames_walked,
            rule,
            values,
            last,
            total,
            check,
            problem,
        } => ExplainDto::Samples {
            encoding: encoding_name,
            encoding_number: f64::from(encoding),
            big_endian,
            declared: declared as f64,
            bytes: payload_bytes as f64,
            steim: steim.is_some(),
            x0: steim.as_ref().map(|s| f64::from(s.x0)),
            xn: steim.as_ref().map(|s| f64::from(s.xn)),
            first_difference: steim.as_ref().and_then(|s| s.first_difference.map(f64::from)),
            frames_in_record: steim.as_ref().map_or(0.0, |s| s.frames_in_record as f64),
            frames: steim.map_or_else(Vec::new, |s| {
                s.frames.into_iter().map(|f| MseedFrameDto { held: f.held as f64, used: f.used as f64 }).collect()
            }),
            frames_walked: frames_walked as f64,
            rule: rule.unwrap_or_default(),
            values,
            last: last.unwrap_or_default(),
            total: total as f64,
            check: check.map(|c| c.passed()),
            problem: problem.unwrap_or_default(),
        },
        Explain::Float { format, width, bits } => ExplainDto::Float {
            format: format.to_string(),
            width: f64::from(width),
            pattern: format!("{bits:0>width$x}", width = width as usize / 4),
        },
        Explain::Fixed { bits, frac, signed, raw } => ExplainDto::Fixed {
            bits: f64::from(bits),
            frac: f64::from(frac),
            signed,
            pattern: format!("{raw:0>width$x}", width = bits.div_ceil(4) as usize),
        },
        Explain::Flags { name, raw, bits } => ExplainDto::Flags {
            name,
            current: raw as f64,
            bits: bits.into_iter().map(|b| BitDto { bit: b.bit, name: b.name, set: b.set }).collect(),
        },
    }
}
