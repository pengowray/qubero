//! One tile of a FITS tile-compressed image, decompressed and reported step by
//! step.
//!
//! [`fits`](super::fits) reads a compressed image as the binary table it is
//! written as: a row per tile, a descriptor in each row for that tile's
//! compressed bytes, and the heap those bytes are in. What the pixels are is
//! not in the file as it stands. A Rice coded tile is differences between
//! neighbouring pixels, each written in as few bits as its block allowed; a
//! gzipped one is a deflate stream; and a tile of a floating-point image is
//! integers that a scale, a zero point and a fixed sequence of random numbers
//! turn back into floats. None of that is something a template can say, so
//! this says it, the arrangement [`hdf5_chunk`](super::hdf5_chunk) and
//! [`parquet_page`](super::parquet_page) have: every step named, with what
//! went in and what came out, and then the pixels.
//!
//! It is handed bytes rather than nodes: the header's cards, the tile's row,
//! and the tile's compressed bytes from the heap. The cards are read here
//! rather than through the template's own reading of them, because what a
//! tile needs is a dozen keywords, two of them found by the text of another
//! card (`ZNAMEi = 'BLOCKSIZE'` says which `ZVALi` is the block size), and a
//! scale written as a real, which the template's integers cannot carry.
//!
//! ## The convention
//!
//! The FITS tiled image compression convention (Pence, Seaman and White,
//! registered with the IAU FITS Working Group, and section 10 of the FITS 4.0
//! standard). The image is `ZNAXISn` pixels along each axis and a tile is
//! `ZTILEn`, the last tile along an axis taking what is left; tiles are
//! numbered with the first axis running fastest, and tile `n` is row `n` of
//! the table. `ZCMPTYPE` names the algorithm and `ZNAMEi` and `ZVALi` its
//! parameters.
//!
//! **RICE_1.** The first pixel is written whole, `BYTEPIX` bytes of it. The
//! pixels are then taken `BLOCKSIZE` at a time (32 unless a `ZNAMEi` says
//! otherwise, and `BYTEPIX` 4 unless one says otherwise). Each pixel's
//! difference from the one before is folded into a non-negative number, 0,
//! -1, 1, -2 as 0, 1, 2, 3, and each block opens with a code in 3, 4 or 5
//! bits for a `BYTEPIX` of 1, 2 or 4. A code of 0 says every difference in
//! the block is zero and nothing else is written. The largest code, 7, 15 or
//! 26, says the differences are written whole at the pixel's width, for a
//! block too noisy to gain from coding; for four bytes that is not the most
//! five bits hold, since a difference never needs more than 25 low bits
//! before writing it whole is shorter. Any other code is one more than the
//! number of low bits `k` each difference keeps: the high part is written in
//! unary, as that many zero bits and a one, and the low `k` bits follow as
//! they are. Every bit is read from the top of its byte down. The first
//! pixel's own difference is zero and is coded like any other, so the first
//! block covers it.
//!
//! **GZIP_1 and GZIP_2.** The tile's pixels as big-endian numbers, gzipped.
//! GZIP_2 first shuffles the bytes: every pixel's first byte, then every
//! pixel's second byte, and so on, which puts bytes that change slowly next to
//! each other. How wide a pixel is comes from how many bytes came out, as it
//! does in the two libraries that write these, since a quantized float image
//! is stored as 32-bit integers whatever `ZBITPIX` says.
//!
//! **NOCOMPRESS.** The pixels as they are.
//!
//! **PLIO_1.** IRAF's pixel lists, for masks: 16-bit words, each a run of
//! zeros, a run of one value, a change to that value, or a pixel. See
//! `plio.rs`.
//!
//! **HCOMPRESS_1.** An H-transform, whose coefficients are divided by a scale
//! and written a bit plane at a time as quadtrees, and a `SMOOTH` option for
//! taking the blockiness off a lossy tile while it is transformed back. The
//! scale is in the tile's own bytes. See `hcompress.rs`.
//!
//! **Quantized floats.** A floating-point image is usually stored lossily as
//! integers: the table has `ZSCALE` and `ZZERO` columns, one pair a tile, and
//! `ZQUANTIZ` says how the integers were made. With `NO_DITHER` a pixel is
//! `stored × ZSCALE + ZZERO`. With `SUBTRACTIVE_DITHER_1` a random number `r`
//! in [0, 1) was subtracted before rounding, so a pixel is
//! `(stored - r + 0.5) × ZSCALE + ZZERO`. `SUBTRACTIVE_DITHER_2` is the same,
//! except that a stored -2147483646 is a pixel that was exactly zero. A stored
//! value equal to `ZBLANK` is a pixel that was NaN.
//!
//! The random numbers are a fixed sequence the standard gives: 10,000 values
//! from the multiplicative congruential generator `seed = 16807 × seed mod
//! (2^31 - 1)`, starting from 1, each divided by `2^31 - 1` and kept as a
//! 32-bit float. Tile `n` (from 0) starts at the value numbered
//! `(n + ZDITHER0 - 1) mod 10000`, called `iseed`, and takes the value at
//! `500 × rand[iseed]` for its first pixel and the ones after it for the
//! pixels after. When the sequence runs out, `iseed` moves on by one and the
//! next pixel starts again at `500 × rand[iseed]`. The arithmetic is done in
//! 64-bit floats and a 32-bit image keeps the result as a 32-bit float, which
//! is what the libraries do, so a pixel here is the same float to the bit.
//!
//! **A tile that would not quantize** (a tile of one value has no noise to
//! scale by) is stored as its floats instead: gzipped in a
//! `GZIP_COMPRESSED_DATA` column, or as they are in `UNCOMPRESSED_DATA`, with
//! `COMPRESSED_DATA` left empty for that row. Such a tile is read from
//! whichever of those holds it, and is not unquantized.
//!
//! What is not read: a `ZBLANK` on an integer image (the pixel is left as the
//! integer it is, which is what that keyword says it is), and a tile over
//! [`COMPRESSED_LIMIT`] or [`PIXEL_LIMIT`].
//!
//! The Rice, PLIO and HCOMPRESS decoders are in `rice.rs`, `plio.rs` and
//! `hcompress.rs`, and the unquantizing in `quantize.rs`; this keeps the
//! cards, the table, the tile, and the steps that are one line each.

use crate::codec::{self, Codec};

mod hcompress;
mod plio;
mod quantize;
mod rice;
#[cfg(test)]
mod tests;

pub use quantize::randoms;

/// What [`StructDef::packed`](crate::template::StructDef::packed) calls a
/// compressed image, so the template can mark one and the inspector can find
/// its way back here.
pub const PACKING: &str = "fits_tile";

/// How many of a tile's pixels a panel is handed.
pub const VALUES_SHOWN: usize = 32;

/// The largest compressed tile this will open for a panel that is redrawn
/// every time the cursor moves. A tile is a few kilobytes in the files people
/// write; a megabyte of one is a tile the size of a whole large image.
pub const COMPRESSED_LIMIT: usize = 16 << 20;

/// The most pixels a tile may have for this to decompress it: sixteen million,
/// a 4096 by 4096 tile, which as 64-bit floats is the 128 MB this stops at.
pub const PIXEL_LIMIT: u64 = 1 << 24;

/// A header's cards, each as its keyword and the text of its value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cards(Vec<(String, String)>);

impl Cards {
    /// Read the cards out of a header's bytes: eighty columns each, a keyword
    /// in the first eight, and a value after `= ` when there is one. A quoted
    /// value is the text inside its quotes, with a doubled quote read as one
    /// and the blanks it is padded with taken off; any other value is the text
    /// before a `/`, trimmed. A card with no value is left out.
    pub fn parse(bytes: &[u8]) -> Cards {
        let mut out = Vec::new();
        for card in bytes.chunks_exact(80) {
            let key = String::from_utf8_lossy(&card[..8]).trim_end().to_string();
            if key == "END" {
                break;
            }
            if &card[8..10] != b"= " {
                continue;
            }
            let rest = String::from_utf8_lossy(&card[10..]).to_string();
            let value = rest.trim_start();
            let text = if let Some(inside) = value.strip_prefix('\'') {
                let mut s = String::new();
                let mut chars = inside.chars().peekable();
                while let Some(c) = chars.next() {
                    if c == '\'' {
                        if chars.peek() == Some(&'\'') {
                            chars.next();
                            s.push('\'');
                            continue;
                        }
                        break;
                    }
                    s.push(c);
                }
                s.trim_end().to_string()
            } else {
                value.split('/').next().unwrap_or("").trim().to_string()
            };
            out.push((key, text));
        }
        Cards(out)
    }

    /// The value of the first card with this keyword.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// The same, read as a whole number.
    pub fn int(&self, key: &str) -> Option<i64> {
        self.text(key)?.parse().ok()
    }

    /// The same, read as a real. FITS allows a `D` for the exponent.
    pub fn real(&self, key: &str) -> Option<f64> {
        self.text(key)?.replace(['D', 'd'], "E").parse().ok()
    }
}

/// One column of the table, as far as a tile needs it: what it is called,
/// where in the row it starts, and what its `TFORMn` says it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// The byte in the row the column starts at.
    pub at: usize,
    pub repeat: usize,
    /// The type letter, and for a `P` or `Q` column the letter after it.
    pub code: u8,
    pub elem: u8,
}

impl Column {
    /// How many bytes one cell of the column takes. A repeat too large to
    /// count them in is as wide as a count goes, which is wider than any row.
    fn width(&self) -> usize {
        let r = self.repeat;
        match self.code {
            b'L' | b'B' | b'A' => r,
            b'X' => r.div_ceil(8),
            b'I' => r.saturating_mul(2),
            b'J' | b'E' => r.saturating_mul(4),
            b'K' | b'D' | b'C' | b'P' => r.saturating_mul(8),
            b'M' | b'Q' => r.saturating_mul(16),
            _ => 0,
        }
    }

    /// The column's cell in `row`, or `None` where the row ends first.
    fn cell<'a>(&self, row: &'a [u8]) -> Option<&'a [u8]> {
        row.get(self.at..self.at.checked_add(self.width())?)
    }
}

/// Everything the header of a compressed image says about how to get its
/// pixels back.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// `ZCMPTYPE`, as written.
    pub algorithm: String,
    /// `ZBITPIX`: what a pixel of the image is, not what a tile stores.
    pub zbitpix: i64,
    /// `ZNAXISn` and `ZTILEn`, first axis first.
    pub shape: Vec<u64>,
    pub tile_shape: Vec<u64>,
    /// Rice's parameters, from `ZNAMEi` and `ZVALi`, with the convention's
    /// defaults where a header leaves them out.
    pub blocksize: u64,
    pub bytepix: u64,
    /// HCOMPRESS's `SMOOTH`, 0 unless a header says otherwise. Its scale is
    /// read from each tile's own bytes rather than from `ZVALi`.
    pub smooth: i64,
    /// `ZQUANTIZ`, as written, or empty.
    pub quantize: String,
    pub dither0: Option<i64>,
    /// The keyword forms of what is more often a column: a scale, a zero
    /// point and a blank value for every tile at once.
    pub zscale: Option<f64>,
    pub zzero: Option<f64>,
    pub zblank: Option<i64>,
    pub columns: Vec<Column>,
    /// `NAXIS1` and `NAXIS2`: how wide a row is and how many there are.
    pub row_bytes: usize,
    pub rows: u64,
    /// Where the heap starts, counted from the start of the data.
    pub heap_start: u64,
}

impl Image {
    /// Read what a compressed image's header says. `Err` for a header that
    /// does not describe one well enough to find a tile in.
    pub fn from_cards(cards: &Cards) -> Result<Image, String> {
        let Some(zbitpix) = cards.int("ZBITPIX") else {
            return Err("Not unpacked: the header has no ZBITPIX keyword, so the pixel type is unknown.".into());
        };
        let axes = cards.int("ZNAXIS").unwrap_or(0).clamp(0, 99) as usize;
        let mut shape = Vec::with_capacity(axes);
        let mut tile_shape = Vec::with_capacity(axes);
        for n in 1..=axes {
            let along = cards.int(&format!("ZNAXIS{n}")).unwrap_or(0).max(0) as u64;
            // A missing `ZTILEn` is a row of pixels: the whole first axis,
            // and one pixel along every other.
            let default = if n == 1 { along } else { 1 };
            shape.push(along);
            tile_shape.push(cards.int(&format!("ZTILE{n}")).filter(|t| *t > 0).map_or(default, |t| t as u64));
        }
        let mut blocksize = 32;
        let mut bytepix = 4;
        let mut smooth = 0;
        for i in 1..=999 {
            let Some(name) = cards.text(&format!("ZNAME{i}")) else { break };
            let value = cards.int(&format!("ZVAL{i}"));
            match (name.to_ascii_uppercase().as_str(), value) {
                ("BLOCKSIZE", Some(v)) => blocksize = v.max(0) as u64,
                ("BYTEPIX", Some(v)) => bytepix = v.max(0) as u64,
                ("SMOOTH", Some(v)) => smooth = v,
                _ => {}
            }
        }
        let fields = cards.int("TFIELDS").unwrap_or(0).clamp(0, 999) as usize;
        let mut columns = Vec::with_capacity(fields);
        let mut at = 0;
        for n in 1..=fields {
            let form = cards.text(&format!("TFORM{n}")).unwrap_or("");
            let digits = form.bytes().take_while(u8::is_ascii_digit).count();
            // Digits too many for a count are a repeat too wide for any row.
            let repeat = if digits == 0 { 1 } else { form[..digits].parse().unwrap_or(usize::MAX) };
            let code = form.as_bytes().get(digits).copied().unwrap_or(0).to_ascii_uppercase();
            let elem = form.as_bytes().get(digits + 1).copied().unwrap_or(0).to_ascii_uppercase();
            let name = cards.text(&format!("TTYPE{n}")).unwrap_or("").trim().to_ascii_uppercase();
            let column = Column { name, at, repeat, code, elem };
            at = at.saturating_add(column.width());
            columns.push(column);
        }
        let row_bytes = cards.int("NAXIS1").unwrap_or(0).max(0) as usize;
        let rows = cards.int("NAXIS2").unwrap_or(0).max(0) as u64;
        // Rows too many to count in bytes put the heap past the end of any
        // file, which is where the largest count leaves it too.
        let heap_start = cards.int("THEAP").map_or_else(|| (row_bytes as u64).checked_mul(rows).unwrap_or(u64::MAX), |t| t.max(0) as u64);
        Ok(Image {
            algorithm: cards.text("ZCMPTYPE").unwrap_or("").to_string(),
            zbitpix,
            shape,
            tile_shape,
            blocksize,
            bytepix,
            smooth,
            quantize: cards.text("ZQUANTIZ").unwrap_or("").to_string(),
            dither0: cards.int("ZDITHER0"),
            zscale: cards.real("ZSCALE"),
            zzero: cards.real("ZZERO"),
            zblank: cards.int("ZBLANK").or_else(|| cards.int("BLANK")),
            columns,
            row_bytes,
            rows,
            heap_start,
        })
    }

    /// How many tiles along each axis.
    fn tiles_along(&self) -> Vec<u64> {
        self.shape.iter().zip(&self.tile_shape).map(|(n, t)| n.div_ceil((*t).max(1))).collect()
    }

    /// How many tiles the image is cut into, or `None` for more than a `u64`
    /// counts, which only an [invalid](Image::invalid) header says.
    pub fn tiles(&self) -> Option<u64> {
        if self.shape.is_empty() {
            return Some(0);
        }
        product(&self.tiles_along())
    }

    /// Why no file could hold the image the header describes, when none could:
    /// it is more pixels than a `u64` counts. A tile is no more pixels than the
    /// image and there are no more tiles than pixels, so a header that passes
    /// this counts both.
    pub fn invalid(&self) -> Option<String> {
        product(&self.shape).is_none().then(|| {
            format!("Not unpacked: the header is invalid. ZNAXISn say the image is {} pixels, giving more than the maximum of 2^64.", dims(&self.shape))
        })
    }

    /// Where tile `index` starts, counted from 0 along each axis, and how many
    /// pixels it has along each: a whole tile, or what is left at the far edge.
    pub fn tile_box(&self, index: u64) -> (Vec<u64>, Vec<u64>) {
        let mut rest = index;
        let mut start = Vec::with_capacity(self.shape.len());
        let mut size = Vec::with_capacity(self.shape.len());
        for ((n, t), along) in self.shape.iter().zip(&self.tile_shape).zip(self.tiles_along()) {
            let k = if along == 0 { 0 } else { rest % along };
            rest = if along == 0 { 0 } else { rest / along };
            // `k` is less than the tiles along the axis, so `k * t` is less
            // than the axis is long, and cannot overflow.
            let from = k * t;
            start.push(from);
            size.push((*t).min(n.saturating_sub(from)));
        }
        (start, size)
    }

    /// The column with this name.
    fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// What a tile's row says: where its bytes are, and the scale, zero point
    /// and blank value it was quantized with where the table has columns for
    /// them.
    pub fn row(&self, bytes: &[u8]) -> Row {
        // A cell too short for one element, as a repeat of 0 leaves it, is no
        // cell.
        let descriptor = |c: &Column| -> Option<(u64, u64)> {
            let cell = c.cell(bytes)?;
            match c.code {
                b'P' => Some((u64::from(be_u32(cell.get(0..4)?)), u64::from(be_u32(cell.get(4..8)?)))),
                b'Q' => Some((be_u64(cell.get(0..8)?), be_u64(cell.get(8..16)?))),
                _ => None,
            }
        };
        let real = |name: &str| -> Option<f64> {
            let c = self.column(name)?;
            let cell = c.cell(bytes)?;
            match c.code {
                b'D' => Some(f64::from_bits(be_u64(cell.get(0..8)?))),
                b'E' => Some(f64::from(f32::from_bits(be_u32(cell.get(0..4)?)))),
                _ => None,
            }
        };
        let blank = self.column("ZBLANK").and_then(|c| {
            let cell = c.cell(bytes)?;
            match c.code {
                b'J' => Some(i64::from(be_u32(cell.get(0..4)?) as i32)),
                b'K' => Some(be_u64(cell.get(0..8)?) as i64),
                b'I' => Some(i64::from(i16::from_be_bytes(cell.get(0..2)?.try_into().ok()?))),
                _ => None,
            }
        });
        // `COMPRESSED_DATA` is where a tile is, unless its row left it empty
        // and put the tile in one of the two columns for a tile that would not
        // quantize.
        let mut place = None;
        for (name, stored) in
            [("COMPRESSED_DATA", Stored::Compressed), ("GZIP_COMPRESSED_DATA", Stored::Gzip), ("UNCOMPRESSED_DATA", Stored::Uncompressed)]
        {
            let Some(c) = self.column(name) else { continue };
            let Some((count, offset)) = descriptor(c) else { continue };
            if count > 0 {
                place = Some(Place { stored, count, offset, elem: c.elem });
                break;
            }
        }
        Row {
            place,
            has_data_column: self.column("COMPRESSED_DATA").is_some(),
            zscale: real("ZSCALE").or(self.zscale),
            zzero: real("ZZERO").or(self.zzero),
            zblank: blank.or(self.zblank),
            quantized: self.zbitpix < 0 && (self.column("ZSCALE").is_some() || self.zscale.is_some()),
        }
    }
}

/// Which column a tile's bytes were in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    /// `COMPRESSED_DATA`, compressed with the image's algorithm.
    Compressed,
    /// `GZIP_COMPRESSED_DATA`: a float tile that would not quantize, gzipped.
    Gzip,
    /// `UNCOMPRESSED_DATA`: the same, as it is.
    Uncompressed,
}

impl Stored {
    /// The column's name.
    pub fn column(self) -> &'static str {
        match self {
            Stored::Compressed => "COMPRESSED_DATA",
            Stored::Gzip => "GZIP_COMPRESSED_DATA",
            Stored::Uncompressed => "UNCOMPRESSED_DATA",
        }
    }
}

/// Where a tile's bytes are in the heap: which column's descriptor pointed
/// there, how many elements, and where they start in the heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub stored: Stored,
    pub count: u64,
    pub offset: u64,
    /// The letter after the `P`, which says what one element is.
    pub elem: u8,
}

impl Place {
    /// How many bytes the elements take.
    pub fn bytes(&self) -> u64 {
        self.count.saturating_mul(match self.elem {
            b'I' => 2,
            b'J' | b'E' => 4,
            b'K' | b'D' => 8,
            _ => 1,
        })
    }
}

/// What one row of a compressed image says about its tile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row {
    /// Where the tile's bytes are. None when every data column of the row is
    /// empty.
    pub place: Option<Place>,
    pub has_data_column: bool,
    pub zscale: Option<f64>,
    pub zzero: Option<f64>,
    pub zblank: Option<i64>,
    /// Whether the image's floats were stored as quantized integers.
    pub quantized: bool,
}

/// One step of the decompression, in the order it was done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// What was done: an algorithm's name, `gzip`, a quantization method.
    pub what: String,
    /// How many bytes it read and how many it produced. A step that produced
    /// pixels rather than bytes says how many in `note`, and nothing here.
    pub in_bytes: usize,
    pub out_bytes: usize,
    pub note: String,
}

/// How a pixel is written out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Int,
    F32,
    F64,
}

/// A tile, decompressed, and what it took.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// Which tile, of how many: `None` for more than a `u64` counts.
    pub index: u64,
    pub tiles: Option<u64>,
    /// Where it starts in the image, from 0 along each axis, and how many
    /// pixels it has along each. And how many the whole image has along each.
    pub start: Vec<u64>,
    pub shape: Vec<u64>,
    pub image_shape: Vec<u64>,
    pub algorithm: String,
    /// Which column the bytes were read from.
    pub stored: Option<Stored>,
    /// How many bytes the tile is in the heap, and how many came out of
    /// undoing its compression.
    pub packed_bytes: usize,
    pub decoded_bytes: usize,
    pub steps: Vec<Step>,
    pub kind: Kind,
    /// The pixels, as the image's type: an integer image's pixels are exact
    /// in a 64-bit float, and a 32-bit float image's are 32-bit floats.
    pub pixels: Vec<f64>,
    /// What one pixel is: `i16`, `f32`.
    pub element_type: String,
    /// Why fewer pixels than the tile has came out, or none.
    pub problem: Option<String>,
}

impl Tile {
    /// How many pixels the tile has, whether or not they were all decoded, or
    /// `None` for more than a `u64` counts.
    pub fn pixel_count(&self) -> Option<u64> {
        product(&self.shape)
    }

    /// One pixel as a panel writes it.
    pub fn text(&self, i: usize) -> String {
        let v = self.pixels[i];
        match self.kind {
            Kind::Int => format!("{}", v as i64),
            Kind::F32 => format!("{}", v as f32),
            Kind::F64 => format!("{v}"),
        }
    }
}

/// Decompress tile `index` of `image`, whose row is `row` and whose bytes,
/// read from where the row's descriptor points, are `data`.
pub fn decode(image: &Image, index: u64, row: &Row, data: &[u8]) -> Tile {
    let mut tile = unread(image, index, row, data.len(), None);
    if tile.problem.is_some() {
        return tile;
    }
    let Some(pixels) = tile.pixel_count().filter(|n| *n <= PIXEL_LIMIT) else {
        let pixels = tile.pixel_count().map_or_else(|| format!("{} pixels", dims(&tile.shape)), pixel_word);
        tile.problem = Some(format!("Not unpacked: the tile is {pixels}, over this viewer's limit of {} pixels.", commas(PIXEL_LIMIT)));
        return tile;
    };
    let pixels = pixels as usize;
    let Some(place) = row.place else {
        tile.problem = Some(if row.has_data_column {
            "Not unpacked: this tile's row has no bytes in COMPRESSED_DATA, GZIP_COMPRESSED_DATA or UNCOMPRESSED_DATA.".into()
        } else {
            "Not unpacked: the table has no COMPRESSED_DATA column.".into()
        });
        return tile;
    };
    if pixels == 0 {
        return tile;
    }

    // The integers or floats the tile's bytes come to, before any
    // quantization is undone.
    let stored = match place.stored {
        Stored::Compressed => match image.algorithm.as_str() {
            "RICE_1" | "RICE_ONE" => rice::step(&mut tile, image, data, pixels),
            "GZIP_1" => gzip_step(&mut tile, data, pixels, false, row.quantized),
            "GZIP_2" => gzip_step(&mut tile, data, pixels, true, row.quantized),
            "NOCOMPRESS" => stored_step(&mut tile, data, pixels, row.quantized, "taken as they are (ZCMPTYPE = NOCOMPRESS)"),
            "PLIO_1" => plio::step(&mut tile, image, data, pixels),
            "HCOMPRESS_1" => hcompress::step(&mut tile, image, data, pixels),
            other => {
                let name = if other.is_empty() { "ZCMPTYPE" } else { other };
                stopped(&mut tile, name, data.len());
                tile.problem = Some(format!(
                    "Stopped at {name}: ZCMPTYPE names an algorithm this viewer does not know, so the pixels could not be read."
                ));
                return tile;
            }
        },
        Stored::Gzip => gzip_step(&mut tile, data, pixels, false, false),
        Stored::Uncompressed => stored_step(&mut tile, data, pixels, false, "taken as they are (UNCOMPRESSED_DATA)"),
    };
    let Some(values) = stored else { return tile };

    if row.quantized && place.stored == Stored::Compressed {
        tile.pixels = quantize::unquantize(&mut tile, image, row, index, &values);
    } else {
        // An integer the tile stored wider or narrower than the image's own
        // type is the image's type all the same, as the libraries cast it.
        tile.pixels = match (&values, tile.kind) {
            (Values::Ints(v), Kind::Int) => v.iter().map(|x| cast(*x, image.zbitpix) as f64).collect(),
            (Values::Ints(v), Kind::F32) => v.iter().map(|x| f64::from(*x as f32)).collect(),
            (Values::Ints(v), Kind::F64) => v.iter().map(|x| *x as f64).collect(),
            (Values::Floats(v), Kind::F32) => v.iter().map(|x| f64::from(*x as f32)).collect(),
            (Values::Floats(v), _) => v.clone(),
        };
    }
    tile
}

/// The row for a step that was reached and could not be done, which the
/// problem after it names: nothing came out of it.
fn stopped(tile: &mut Tile, what: &str, in_bytes: usize) {
    tile.steps.push(Step { what: what.into(), in_bytes, out_bytes: 0, note: String::new() });
}

/// A tile whose bytes were not decompressed: where it is and what it is, and
/// `problem` to say why not. What a caller answers with for a tile it would
/// not read, so that a panel still says which tile and how large. An
/// [invalid](Image::invalid) header is the problem instead, since whatever else
/// went wrong follows from it.
pub fn unread(image: &Image, index: u64, row: &Row, packed_bytes: usize, problem: Option<String>) -> Tile {
    let (start, shape) = image.tile_box(index);
    let (kind, element_type) = image_type(image.zbitpix);
    Tile {
        index,
        tiles: image.tiles(),
        start,
        shape,
        image_shape: image.shape.clone(),
        algorithm: image.algorithm.clone(),
        stored: row.place.map(|p| p.stored),
        packed_bytes,
        decoded_bytes: 0,
        steps: Vec::new(),
        kind,
        pixels: Vec::new(),
        element_type: element_type.to_string(),
        problem: image.invalid().or(problem),
    }
}

/// A tile of an image whose header could not be read: nothing but why.
pub fn unreadable(problem: String) -> Tile {
    Tile {
        index: 0,
        tiles: Some(0),
        start: Vec::new(),
        shape: Vec::new(),
        image_shape: Vec::new(),
        algorithm: String::new(),
        stored: None,
        packed_bytes: 0,
        decoded_bytes: 0,
        steps: Vec::new(),
        kind: Kind::Int,
        pixels: Vec::new(),
        element_type: String::new(),
        problem: Some(problem),
    }
}

/// What a tile's bytes read as before quantization is undone.
enum Values {
    Ints(Vec<i64>),
    Floats(Vec<f64>),
}

/// How an image of this `ZBITPIX` writes a pixel, and what it is called.
fn image_type(zbitpix: i64) -> (Kind, &'static str) {
    match zbitpix {
        8 => (Kind::Int, "u8"),
        16 => (Kind::Int, "i16"),
        32 => (Kind::Int, "i32"),
        64 => (Kind::Int, "i64"),
        -32 => (Kind::F32, "f32"),
        -64 => (Kind::F64, "f64"),
        _ => (Kind::Int, "i32"),
    }
}

/// An integer as the image's integer type holds it.
fn cast(v: i64, zbitpix: i64) -> i64 {
    match zbitpix {
        8 => i64::from(v as u8),
        16 => i64::from(v as i16),
        32 => i64::from(v as i32),
        _ => v,
    }
}

/// gzip, and for GZIP_2 the bytes put back together, then read as numbers.
fn gzip_step(tile: &mut Tile, data: &[u8], pixels: usize, shuffled: bool, quantized: bool) -> Option<Values> {
    let bytes = match codec::decode(Codec::Gzip, data) {
        Ok(b) => b,
        Err(why) => {
            stopped(tile, "gzip", data.len());
            tile.problem = Some(format!("Stopped at gzip: {}.", refusal(why)));
            return None;
        }
    };
    tile.decoded_bytes = bytes.len();
    tile.steps.push(Step { what: "gzip".into(), in_bytes: data.len(), out_bytes: bytes.len(), note: String::new() });
    let width = width_of(tile, "gzip", bytes.len(), pixels)?;
    let bytes = if shuffled && width > 1 {
        let back = unshuffle(&bytes, width);
        tile.steps.push(Step {
            what: "unshuffle".into(),
            in_bytes: bytes.len(),
            out_bytes: back.len(),
            note: format!(
                "each pixel's {width} bytes put back together: GZIP_2 stores the first byte of every pixel, then the second byte of every pixel, and so on"
            ),
        });
        back
    } else {
        bytes
    };
    Some(read_numbers(tile, &bytes, width, quantized))
}

/// NOCOMPRESS, or a tile kept as it is: the bytes read as numbers. `note`
/// says which of the two.
fn stored_step(tile: &mut Tile, data: &[u8], pixels: usize, quantized: bool, note: &str) -> Option<Values> {
    tile.decoded_bytes = data.len();
    tile.steps.push(Step { what: "stored".into(), in_bytes: data.len(), out_bytes: data.len(), note: note.into() });
    let width = width_of(tile, "stored", data.len(), pixels)?;
    Some(read_numbers(tile, data, width, quantized))
}

/// How wide a pixel is, from how many bytes `step` gave for how many pixels.
fn width_of(tile: &mut Tile, step: &str, bytes: usize, pixels: usize) -> Option<usize> {
    let width = bytes / pixels;
    let problem = if bytes % pixels != 0 {
        format!(
            "Stopped at read: {step} gave {} for {}, which is not a whole number of bytes per pixel.",
            plural(bytes as u64, "byte", "bytes"),
            pixel_word(pixels as u64)
        )
    } else if !matches!(width, 1 | 2 | 4 | 8) {
        format!(
            "Stopped at read: {step} gave {} for {}, which is {width} bytes per pixel; FITS pixels are 1, 2, 4 or 8 bytes.",
            plural(bytes as u64, "byte", "bytes"),
            pixel_word(pixels as u64)
        )
    } else {
        return Some(width);
    };
    stopped(tile, "read", bytes);
    tile.problem = Some(problem);
    None
}

/// Every pixel's first byte, then every pixel's second byte, and so on, put
/// back into pixels.
fn unshuffle(bytes: &[u8], width: usize) -> Vec<u8> {
    let n = bytes.len() / width;
    let mut out = vec![0u8; bytes.len()];
    for b in 0..width {
        for i in 0..n {
            out[i * width + b] = bytes[b * n + i];
        }
    }
    out
}

/// Big-endian numbers of `width` bytes: floats when the image is of floats and
/// was not quantized, integers otherwise. A quantized image stores integers
/// whatever its `ZBITPIX` says.
fn read_numbers(tile: &mut Tile, bytes: &[u8], width: usize, quantized: bool) -> Values {
    let floats = tile.kind != Kind::Int && !quantized && width >= 4;
    let name = match (floats, width) {
        (true, 4) => "f32",
        (true, _) => "f64",
        (false, 1) => "u8",
        (false, 2) => "i16",
        (false, 4) => "i32",
        (false, _) => "i64",
    };
    let count = bytes.len() / width;
    tile.steps.push(Step {
        what: "read".into(),
        in_bytes: bytes.len(),
        out_bytes: 0,
        note: format!("{}, as big-endian {name}", pixel_word(count as u64)),
    });
    let chunks = bytes.chunks_exact(width);
    if floats {
        Values::Floats(match width {
            4 => chunks.map(|c| f64::from(f32::from_bits(be_u32(c)))).collect(),
            _ => chunks.map(|c| f64::from_bits(be_u64(c))).collect(),
        })
    } else {
        Values::Ints(match width {
            1 => chunks.map(|c| i64::from(c[0])).collect(),
            2 => chunks.map(|c| i64::from(i16::from_be_bytes([c[0], c[1]]))).collect(),
            4 => chunks.map(|c| i64::from(be_u32(c) as i32)).collect(),
            _ => chunks.map(|c| be_u64(c) as i64).collect(),
        })
    }
}

/// Why the codec would not open the bytes, in words.
fn refusal(why: codec::Refusal) -> &'static str {
    match why {
        codec::Refusal::TooLarge => "too large to unpack (over 64 MiB)",
        codec::Refusal::Failed => "unpacking failed",
        codec::Refusal::Unaligned => "not on a byte boundary",
        codec::Refusal::Settings => "the file doesn't say how this was packed",
    }
}

/// A count as people read one: 5980 as `5,980`.
fn commas(n: u64) -> String {
    crate::encode::commas(n)
}

/// A count and its noun: `1 pixel`, `5,980 pixels`.
fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", commas(n), if n == 1 { one } else { many })
}

fn pixel_word(n: u64) -> String {
    plural(n, "pixel", "pixels")
}

/// How many there are of something counted along each axis, or `None` for
/// more than a `u64` counts. None along any axis is none at all.
fn product(along: &[u64]) -> Option<u64> {
    if along.contains(&0) {
        return Some(0);
    }
    along.iter().try_fold(1u64, |n, a| n.checked_mul(*a))
}

/// Lengths along each axis as a panel writes them: `440 × 300`.
fn dims(along: &[u64]) -> String {
    along.iter().map(|n| commas(*n)).collect::<Vec<_>>().join(" × ")
}

fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn be_u64(b: &[u8]) -> u64 {
    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}
