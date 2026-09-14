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
//! **PLIO_1 and HCOMPRESS_1** are named and not decoded. PLIO is for masks of
//! small integers and HCOMPRESS is a wavelet transform; no sample here uses
//! either.
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
//! What is not read: the parameters of PLIO and HCOMPRESS, a `ZBLANK` on an
//! integer image (the pixel is left as the integer it is, which is what that
//! keyword says it is), and a tile over [`COMPRESSED_LIMIT`] or
//! [`PIXEL_LIMIT`].

use std::sync::OnceLock;

use crate::bits::Bits;
use crate::codec::{self, Codec};

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

/// How many random numbers the dithering sequence has.
const N_RANDOM: usize = 10_000;

/// The stored value `SUBTRACTIVE_DITHER_2` writes for a pixel that was
/// exactly zero.
const ZERO_VALUE: i64 = -2_147_483_646;

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
        for i in 1..=999 {
            let Some(name) = cards.text(&format!("ZNAME{i}")) else { break };
            let value = cards.int(&format!("ZVAL{i}"));
            match (name.to_ascii_uppercase().as_str(), value) {
                ("BLOCKSIZE", Some(v)) => blocksize = v.max(0) as u64,
                ("BYTEPIX", Some(v)) => bytepix = v.max(0) as u64,
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

    /// How many tiles the image is cut into, or `u64::MAX` for more than that.
    pub fn tiles(&self) -> u64 {
        if self.shape.is_empty() {
            return 0;
        }
        self.tiles_along().iter().fold(1, |n, along| n.saturating_mul(*along))
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
    /// Which tile, of how many.
    pub index: u64,
    pub tiles: u64,
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
    /// `u64::MAX` for more than that, which is over [`PIXEL_LIMIT`].
    pub fn pixel_count(&self) -> u64 {
        self.shape.iter().fold(1, |n, along| n.saturating_mul(*along))
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
    let pixels = tile.pixel_count();
    if pixels > PIXEL_LIMIT {
        tile.problem = Some(format!(
            "Not unpacked: the tile is {}, over this viewer's limit of {} pixels.",
            pixel_word(pixels),
            commas(PIXEL_LIMIT)
        ));
        return tile;
    }
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
            "RICE_1" | "RICE_ONE" => rice_step(&mut tile, image, data, pixels),
            "GZIP_1" => gzip_step(&mut tile, data, pixels, false, row.quantized),
            "GZIP_2" => gzip_step(&mut tile, data, pixels, true, row.quantized),
            "NOCOMPRESS" => stored_step(&mut tile, data, pixels, row.quantized, "taken as they are (ZCMPTYPE = NOCOMPRESS)"),
            name @ ("PLIO_1" | "HCOMPRESS_1") => {
                stopped(&mut tile, name, data.len());
                tile.problem = Some(format!("Stopped at {name}: this viewer has no {name} decoder, so the pixels could not be read."));
                return tile;
            }
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
        tile.pixels = unquantize(&mut tile, image, row, index, &values);
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
/// not read, so that a panel still says which tile and how large.
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
        problem,
    }
}

/// A tile of an image whose header could not be read: nothing but why.
pub fn unreadable(problem: String) -> Tile {
    Tile {
        index: 0,
        tiles: 0,
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

/// Rice: the bits taken apart into the tile's integers.
fn rice_step(tile: &mut Tile, image: &Image, data: &[u8], pixels: usize) -> Option<Values> {
    let name = image.algorithm.clone();
    let bytepix = image.bytepix as usize;
    if !matches!(bytepix, 1 | 2 | 4) {
        stopped(tile, &name, data.len());
        tile.problem = Some(format!(
            "Stopped at {name}: BYTEPIX is {bytepix}, and this viewer's Rice decoder reads 1, 2 or 4 bytes per pixel."
        ));
        return None;
    }
    let blocksize = image.blocksize as usize;
    if blocksize == 0 {
        stopped(tile, &name, data.len());
        tile.problem = Some(format!("Stopped at {name}: BLOCKSIZE is 0, and a block must hold at least 1 pixel."));
        return None;
    }
    let rice = rice(data, pixels, bytepix, blocksize);
    let made = rice.values.len();
    tile.decoded_bytes = made * bytepix;
    let blocks = rice.coded + rice.zero + rice.whole;
    tile.steps.push(Step {
        what: name.clone(),
        in_bytes: data.len(),
        out_bytes: made * bytepix,
        note: format!(
            "{} per pixel; first pixel stored whole, then {} of {}: {} Rice-coded, {} all-zero, {} uncoded at {} bits per difference",
            plural(bytepix as u64, "byte", "bytes"),
            plural(blocks as u64, "block", "blocks"),
            pixel_word(blocksize as u64),
            commas(rice.coded as u64),
            commas(rice.zero as u64),
            commas(rice.whole as u64),
            bytepix * 8
        ),
    });
    if let Some(code) = rice.bad_code {
        let largest = [0, 7, 15, 0, 26][bytepix];
        tile.problem = Some(format!(
            "Stopped at {name}: block code {code} after {} of {}; the largest for {bytepix}-byte pixels is {largest}.",
            commas(made as u64),
            pixel_word(pixels as u64)
        ));
    } else if made < pixels {
        tile.problem = Some(format!("Stopped at {name}: the data ran out after {} of {}.", commas(made as u64), pixel_word(pixels as u64)));
    }
    Some(Values::Ints(rice.values))
}

/// What a Rice tile came to, and how its blocks were written.
struct Rice {
    values: Vec<i64>,
    coded: usize,
    zero: usize,
    whole: usize,
    /// A block code past the largest the width has, where one stopped the
    /// walk.
    bad_code: Option<u64>,
}

/// Rice decoding, as the convention describes it. See the module doc.
fn rice(data: &[u8], pixels: usize, bytepix: usize, blocksize: usize) -> Rice {
    let width = 8 * bytepix as u32;
    let (code_bits, largest) = match bytepix {
        1 => (3, 7),
        2 => (4, 15),
        _ => (5, 26),
    };
    let mask: u64 = if width == 64 { u64::MAX } else { (1 << width) - 1 };
    let mut out = Rice { values: Vec::with_capacity(pixels), coded: 0, zero: 0, whole: 0, bad_code: None };
    if pixels == 0 {
        return out;
    }
    // Read from the top of each byte down.
    let mut bits = Bits::new(data);
    let Some(first) = bits.take(width) else { return out };
    let mut last = first;
    let signed = |v: u64| -> i64 {
        if bytepix == 1 {
            v as i64
        } else {
            let shift = 64 - width;
            ((v << shift) as i64) >> shift
        }
    };
    while out.values.len() < pixels {
        let Some(code) = bits.take(code_bits) else { return out };
        let n = blocksize.min(pixels - out.values.len());
        if code == 0 {
            out.zero += 1;
            out.values.extend(std::iter::repeat_n(signed(last), n));
            continue;
        }
        if code > largest {
            out.bad_code = Some(code);
            return out;
        }
        let whole = code == largest;
        if whole {
            out.whole += 1;
        } else {
            out.coded += 1;
        }
        let low = (code - 1) as u32;
        for _ in 0..n {
            let folded = if whole {
                let Some(v) = bits.take(width) else { return out };
                v
            } else {
                let Some(high) = bits.unary() else { return out };
                let Some(bottom) = bits.take(low) else { return out };
                (high << low) | bottom
            };
            // 0, 1, 2, 3 back to 0, -1, 1, -2.
            let difference = if folded & 1 == 0 { folded >> 1 } else { !(folded >> 1) };
            last = last.wrapping_add(difference) & mask;
            out.values.push(signed(last));
        }
    }
    out
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

/// The standard's random numbers, made once.
pub fn randoms() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| seeds().map(|seed| (seed / 2147483647.0) as f32).collect())
}

/// The generator's seeds, in order, the way the standard writes it out: in
/// 64-bit floats, which hold every product exactly.
fn seeds() -> impl Iterator<Item = f64> {
    let a = 16807.0f64;
    let m = 2147483647.0f64;
    let mut seed = 1.0f64;
    (0..N_RANDOM).map(move |_| {
        let t = a * seed;
        seed = t - m * (t / m).trunc();
        seed
    })
}

/// The tile's integers turned back into the floats they were quantized from.
fn unquantize(tile: &mut Tile, image: &Image, row: &Row, index: u64, values: &Values) -> Vec<f64> {
    let ints: &[i64] = match values {
        Values::Ints(v) => v,
        Values::Floats(_) => return Vec::new(),
    };
    let scale = row.zscale.unwrap_or(1.0);
    let zero = row.zzero.unwrap_or(0.0);
    let f32_image = image.zbitpix == -32;
    let keep = |v: f64| if f32_image { f64::from(v as f32) } else { v };
    let method = if image.quantize.is_empty() { "NO_DITHER" } else { image.quantize.as_str() };
    let dither = match method {
        "NO_DITHER" => None,
        "SUBTRACTIVE_DITHER_1" => Some(false),
        "SUBTRACTIVE_DITHER_2" => Some(true),
        other => {
            stopped(tile, other, 0);
            tile.problem = Some(format!(
                "Stopped at {other}: ZQUANTIZ names a method this viewer does not know, so the integers could not be turned back into floats."
            ));
            return Vec::new();
        }
    };
    // The zero point with its sign as the operator, so that a negative one
    // reads as a subtraction rather than as `+ -45.6`.
    let plus = if zero.is_sign_negative() { format!("\u{2212} {}", -zero) } else { format!("+ {zero}") };
    let keywords = "(ZSCALE and ZZERO for this tile)";
    let mut blanks = 0usize;
    let mut zeros = 0usize;
    let mut out = Vec::with_capacity(ints.len());
    let Some(two) = dither else {
        tile.steps.push(Step { what: method.into(), in_bytes: 0, out_bytes: 0, note: format!("float = integer × {scale} {plus} {keywords}") });
        for &q in ints {
            if Some(q) == row.zblank {
                blanks += 1;
                out.push(f64::NAN);
            } else {
                out.push(keep(q as f64 * scale + zero));
            }
        }
        blank_rows(tile, row, blanks, None);
        return out;
    };
    tile.steps.push(Step {
        what: method.into(),
        in_bytes: 0,
        out_bytes: 0,
        note: format!("float = (integer \u{2212} r + 0.5) × {scale} {plus} {keywords}"),
    });
    let Some(dither0) = image.dither0 else {
        tile.problem = Some(format!("Stopped at {method}: this dither needs a ZDITHER0 keyword, and the header has none."));
        return Vec::new();
    };
    let rand = randoms();
    // Summed wider than an i64, since ZDITHER0 can be any i64.
    let mut iseed = (i128::from(index) + i128::from(dither0) - 1).rem_euclid(N_RANDOM as i128) as usize;
    let mut next = (rand[iseed] * 500.0) as usize;
    tile.steps.push(Step {
        what: "dither".into(),
        in_bytes: 0,
        out_bytes: 0,
        note: format!("r from the standard's fixed sequence of 10,000 random numbers; ZDITHER0 = {dither0}, this tile starting at r[{next}]"),
    });
    for &q in ints {
        if Some(q) == row.zblank {
            blanks += 1;
            out.push(f64::NAN);
        } else if two && q == ZERO_VALUE {
            zeros += 1;
            out.push(0.0);
        } else {
            out.push(keep((q as f64 - f64::from(rand[next]) + 0.5) * scale + zero));
        }
        next += 1;
        if next == N_RANDOM {
            iseed = (iseed + 1) % N_RANDOM;
            next = (rand[iseed] * 500.0) as usize;
        }
    }
    blank_rows(tile, row, blanks, two.then_some(zeros));
    out
}

/// A row for the pixels that were blank, and for the method that keeps zeros
/// exact, one for the pixels that were zero. Neither when there were none.
fn blank_rows(tile: &mut Tile, row: &Row, blanks: usize, zeros: Option<usize>) {
    if let (true, Some(v)) = (blanks > 0, row.zblank) {
        let n = blanks as u64;
        let note = format!("{} stored as ZBLANK = {v}, which means blank, read as NaN", pixel_word(n));
        tile.steps.push(Step { what: "blank".into(), in_bytes: 0, out_bytes: 0, note });
    }
    if let Some(z) = zeros.filter(|z| *z > 0) {
        let note = format!("{} stored as {ZERO_VALUE}, which SUBTRACTIVE_DITHER_2 reserves for exactly 0.0", pixel_word(z as u64));
        tile.steps.push(Step { what: "zero".into(), in_bytes: 0, out_bytes: 0, note });
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

fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn be_u64(b: &[u8]) -> u64 {
    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bits written from the top of each byte down, the way Rice reads them.
    #[derive(Default)]
    struct Writer {
        bytes: Vec<u8>,
        bits: usize,
    }

    impl Writer {
        fn put(&mut self, v: u64, n: u32) {
            for i in (0..n).rev() {
                if self.bits % 8 == 0 {
                    self.bytes.push(0);
                }
                let bit = (v >> i) & 1;
                let last = self.bytes.len() - 1;
                self.bytes[last] |= (bit as u8) << (7 - self.bits % 8);
                self.bits += 1;
            }
        }
    }

    /// A Rice encoder written from the description, for the decoder to be
    /// checked against. It picks its own code per block: all zero where every
    /// difference is, whole where `whole` says, and otherwise `k` low bits.
    fn rice_encode(pixels: &[i64], bytepix: usize, blocksize: usize, pick: impl Fn(usize, &[u64]) -> Option<u32>) -> Vec<u8> {
        let width = 8 * bytepix as u32;
        let (code_bits, largest) = match bytepix {
            1 => (3, 7),
            2 => (4, 15),
            _ => (5, 26),
        };
        let mask: u64 = (1u64 << width) - 1;
        let mut w = Writer::default();
        w.put(pixels[0] as u64 & mask, width);
        let mut last = pixels[0] as u64 & mask;
        let folded: Vec<u64> = pixels
            .iter()
            .map(|p| {
                let now = *p as u64 & mask;
                let d = now.wrapping_sub(last) & mask;
                last = now;
                // Sign-extend the difference at the pixel's width, then fold.
                let shift = 64 - width;
                let d = ((d << shift) as i64) >> shift;
                if d >= 0 { (d as u64) << 1 } else { (!(d as u64) << 1) | 1 }
            })
            .collect();
        for (b, block) in folded.chunks(blocksize).enumerate() {
            if block.iter().all(|f| *f == 0) {
                w.put(0, code_bits);
                continue;
            }
            match pick(b, block) {
                None => {
                    w.put(largest, code_bits);
                    for f in block {
                        w.put(*f, width);
                    }
                }
                Some(k) => {
                    w.put(u64::from(k) + 1, code_bits);
                    for f in block {
                        for _ in 0..(f >> k) {
                            w.put(0, 1);
                        }
                        w.put(1, 1);
                        w.put(f & ((1 << k) - 1), k);
                    }
                }
            }
        }
        w.bytes
    }

    #[test]
    fn the_random_numbers_are_the_standards_sequence() {
        // The standard's own check: the 10,000th seed is 1043618065.
        assert_eq!(seeds().last(), Some(1043618065.0));
        let r = randoms();
        assert_eq!(r.len(), 10_000);
        assert_eq!(r[0], (16807.0 / 2147483647.0) as f32);
    }

    #[test]
    fn rice_reads_every_kind_of_block_at_every_width() {
        for bytepix in [1usize, 2, 4] {
            let span = if bytepix == 1 { 255 } else if bytepix == 2 { 60_000 } else { 4_000_000_000 };
            let mut pixels = Vec::new();
            for i in 0..100i64 {
                // A ramp, then a flat run, then noise across the whole range.
                let v = match i {
                    0..=40 => 100 + i * 3,
                    41..=72 => 50,
                    _ => ((i * 7919) % span) - if bytepix == 1 { 0 } else { span / 2 },
                };
                pixels.push(v);
            }
            // As few low bits as keep the unary part short, and never so many
            // that the code would be the one that says whole.
            let most = [0, 5, 13, 0, 24][bytepix];
            let data = rice_encode(&pixels, bytepix, 16, |b, block| {
                let top = block.iter().max().map_or(0, |m| 64 - m.leading_zeros());
                if b == 5 { None } else { Some(top.saturating_sub(2).min(most)) }
            });
            let r = rice(&data, pixels.len(), bytepix, 16);
            let want: Vec<i64> = pixels
                .iter()
                .map(|p| match bytepix {
                    1 => i64::from(*p as u8),
                    2 => i64::from(*p as i16),
                    _ => i64::from(*p as i32),
                })
                .collect();
            assert_eq!(r.values, want, "bytepix {bytepix}");
            assert!(r.zero >= 1 && r.whole == 1 && r.coded >= 1, "bytepix {bytepix}");
        }
    }

    #[test]
    fn a_code_past_the_largest_stops_the_walk() {
        // A 4-byte pixel of 7, then the code 27, which five bits hold and no
        // block may have.
        let mut w = Writer::default();
        w.put(7, 32);
        w.put(27, 5);
        w.put(0, 3);
        let r = rice(&w.bytes, 4, 4, 32);
        assert_eq!((r.values.len(), r.bad_code), (0, Some(27)));
    }

    #[test]
    fn rice_that_runs_out_of_bits_stops_where_it_did() {
        let pixels: Vec<i64> = (0..64).map(|i| i * 5).collect();
        let data = rice_encode(&pixels, 4, 32, |_, _| Some(4));
        let r = rice(&data[..data.len() / 2], pixels.len(), 4, 32);
        assert!(!r.values.is_empty() && r.values.len() < 64);
        assert_eq!(r.values[..], pixels[..r.values.len()]);
    }

    #[test]
    fn unshuffling_puts_each_pixels_bytes_back_together() {
        // Two 16-bit pixels, 0x1234 and 0x5678, shuffled into their high bytes
        // then their low bytes.
        assert_eq!(unshuffle(&[0x12, 0x56, 0x34, 0x78], 2), vec![0x12, 0x34, 0x56, 0x78]);
    }

    /// The cards of a compressed image, as a header writes them.
    fn cards(lines: &[&str]) -> Cards {
        let mut b = Vec::new();
        for l in lines {
            let mut c = l.as_bytes().to_vec();
            c.resize(80, b' ');
            b.extend_from_slice(&c);
        }
        Cards::parse(&b)
    }

    #[test]
    fn a_tile_at_the_far_edge_is_what_is_left() {
        let c = cards(&[
            "ZBITPIX =                   32",
            "ZNAXIS  =                    2",
            "ZNAXIS1 =                   50",
            "ZNAXIS2 =                   60",
            "ZTILE1  =                   20",
            "ZTILE2  =                   16",
            "ZCMPTYPE= 'RICE_1  '",
            "END",
        ]);
        let image = Image::from_cards(&c).unwrap();
        assert_eq!(image.tiles(), 12);
        assert_eq!(image.tile_box(0), (vec![0, 0], vec![20, 16]));
        assert_eq!(image.tile_box(2), (vec![40, 0], vec![10, 16]));
        assert_eq!(image.tile_box(11), (vec![40, 48], vec![10, 12]));
        assert_eq!((image.blocksize, image.bytepix), (32, 4));
    }

    /// A corrupt header can give axes and tiles, or a column's repeat, that
    /// multiply to more than a count holds. The count is then the largest
    /// there is: more tiles than can be numbered, a tile over the pixel limit,
    /// and a cell past the end of its row.
    #[test]
    fn a_count_too_large_to_hold_is_the_largest_there_is() {
        let square = |side: u64, tile: u64| {
            let lines = [
                "ZBITPIX =                    8".to_string(),
                "ZNAXIS  =                    2".into(),
                format!("ZNAXIS1 = {side:20}"),
                format!("ZNAXIS2 = {side:20}"),
                format!("ZTILE1  = {tile:20}"),
                format!("ZTILE2  = {tile:20}"),
                "ZCMPTYPE= 'NOCOMPRESS'".into(),
            ];
            let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
            Image::from_cards(&cards(&refs)).unwrap()
        };
        let row = Row {
            place: Some(Place { stored: Stored::Compressed, count: 4, offset: 0, elem: b'B' }),
            has_data_column: true,
            zscale: None,
            zzero: None,
            zblank: None,
            quantized: false,
        };
        // Tiles of one pixel, 2^80 of them.
        let image = square(1 << 40, 1);
        assert_eq!(image.tiles(), u64::MAX);
        assert_eq!(unread(&image, 0, &row, 4, None).tiles, u64::MAX);
        // One tile of 2^80 pixels.
        let t = decode(&square(1 << 40, 1 << 40), 0, &row, &[1, 2, 3, 4]);
        assert_eq!((t.tiles, t.pixel_count()), (1, u64::MAX));
        assert_eq!(t.problem.as_deref(), Some("Not unpacked: the tile is 18,446,744,073,709,551,615 pixels, over this viewer's limit of 16,777,216 pixels."));

        // A column whose repeat is more bytes than a count holds, of each
        // width, as the data column and as a column before it. And one whose
        // repeat is more than a count holds before it is multiplied at all.
        // Every four bytes of the row say 4, so a P descriptor is four bytes at
        // offset 4.
        let bytes: Vec<u8> = [0, 0, 0, 4].repeat(16);
        let place = |forms: &[String]| {
            let mut lines = vec!["ZBITPIX =                    8".to_string(), format!("TFIELDS = {:20}", forms.len())];
            for (n, form) in forms.iter().enumerate() {
                let name = if n + 1 == forms.len() { "COMPRESSED_DATA" } else { "ZSCALE" };
                lines.push(format!("TTYPE{}  = '{name}'", n + 1));
                lines.push(format!("TFORM{}  = '{form}'", n + 1));
            }
            let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
            Image::from_cards(&cards(&refs)).unwrap().row(&bytes)
        };
        for (code, width) in [("I", 2), ("J", 4), ("K", 8), ("D", 8), ("M", 16), ("PB", 8), ("QB", 16)] {
            let repeat = usize::MAX / width + 1;
            let wide = place(&[format!("{repeat}{code}")]);
            assert_eq!((wide.place, wide.has_data_column), (None, true), "{repeat}{code}");
            let after = place(&[format!("{repeat}{code}"), "1PB".into()]);
            assert_eq!((after.place, after.zscale), (None, None), "{repeat}{code} before the data column");
        }
        let after = place(&[format!("{}L", usize::MAX), "1E".into(), "1PB".into()]);
        assert_eq!((after.place, after.zscale), (None, None));
        assert_eq!(place(&["99999999999999999999PB".into()]).place, None);
        assert_eq!(place(&["1PB".into()]).place, Some(Place { stored: Stored::Compressed, count: 4, offset: 4, elem: b'B' }));
    }

    /// A corrupt header can give a column a repeat of 0, so its cell has no
    /// bytes. That is no cell: the row has no place in it, and its scale, zero
    /// point and blank are the header's keywords.
    #[test]
    fn a_cell_too_short_for_one_element_is_no_cell() {
        // Every four bytes of the row say 4.
        let bytes: Vec<u8> = [0, 0, 0, 4].repeat(16);
        let row = |columns: &[(&str, &str)]| {
            let mut lines = vec![
                "ZBITPIX =                  -32".to_string(),
                "ZSCALE  =                  2.5".into(),
                "ZZERO   =                  1.5".into(),
                "ZBLANK  =                   -5".into(),
                format!("TFIELDS = {:20}", columns.len()),
            ];
            for (n, (name, form)) in columns.iter().enumerate() {
                lines.push(format!("TTYPE{}  = '{name}'", n + 1));
                lines.push(format!("TFORM{}  = '{form}'", n + 1));
            }
            let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
            Image::from_cards(&cards(&refs)).unwrap().row(&bytes)
        };
        for form in ["0PB", "0QB"] {
            let r = row(&[("COMPRESSED_DATA", form)]);
            assert_eq!((r.place, r.has_data_column), (None, true), "{form}");
        }
        // An empty column takes no bytes, so the next one starts where it does.
        let r = row(&[("COMPRESSED_DATA", "0PB"), ("GZIP_COMPRESSED_DATA", "1PB")]);
        assert_eq!(r.place, Some(Place { stored: Stored::Gzip, count: 4, offset: 4, elem: b'B' }));
        for form in ["0D", "0E"] {
            let r = row(&[("ZSCALE", form), ("ZZERO", form)]);
            assert_eq!((r.zscale, r.zzero), (Some(2.5), Some(1.5)), "{form}");
        }
        for form in ["0J", "0K", "0I"] {
            assert_eq!(row(&[("ZBLANK", form)]).zblank, Some(-5), "{form}");
        }
        assert_eq!(row(&[("ZBLANK", "1J")]).zblank, Some(4));
    }

    /// `ZDITHER0` can be any 64-bit number, and the first random number a tile
    /// takes is counted from it and from the tile's number, round the sequence.
    #[test]
    fn the_dither_seed_wraps_round_the_sequence_whatever_it_is() {
        let r = randoms();
        let want = |iseed: usize| f64::from(((4.0 - f64::from(r[(r[iseed] * 500.0) as usize]) + 0.5) * 0.5 + 10.0) as f32);
        // i64::MAX is 5807 more than a multiple of 10,000; one less than
        // i64::MIN is 4191 more.
        assert_eq!(quantized("SUBTRACTIVE_DITHER_1", &[4], i64::MAX, 1).pixels, [want(5807)]);
        assert_eq!(quantized("SUBTRACTIVE_DITHER_1", &[4], i64::MIN, 0).pixels, [want(4191)]);
    }

    #[test]
    fn a_quoted_value_keeps_its_escaped_quote_and_loses_its_padding() {
        let c = cards(&["ZNAME1  = 'BLOCKSIZE'          / compression block size", "OBJECT  = 'it''s  '", "ZSCALE  =      2.5D-1 / real"]);
        assert_eq!(c.text("ZNAME1"), Some("BLOCKSIZE"));
        assert_eq!(c.text("OBJECT"), Some("it's"));
        assert_eq!(c.real("ZSCALE"), Some(0.25));
    }

    /// A one-row image of `pixels` quantized with this method, from a row
    /// whose scale and zero point are 0.5 and 10.
    fn quantized(method: &str, ints: &[i32], dither0: i64, index: u64) -> Tile {
        let mut lines = vec![
            "ZBITPIX =                  -32".to_string(),
            "ZNAXIS  =                    1".to_string(),
            format!("ZNAXIS1 = {:20}", ints.len()),
            "ZCMPTYPE= 'NOCOMPRESS'".to_string(),
            format!("ZDITHER0= {dither0:20}"),
            "ZSCALE  =                  0.5".to_string(),
            "ZZERO   =                 10.0".to_string(),
            "ZBLANK  =          -2147483648".to_string(),
        ];
        if !method.is_empty() {
            lines.push(format!("ZQUANTIZ= '{method}'"));
        }
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let image = Image::from_cards(&cards(&refs)).unwrap();
        let mut data = Vec::new();
        for i in ints {
            data.extend_from_slice(&i.to_be_bytes());
        }
        let row = Row {
            place: Some(Place { stored: Stored::Compressed, count: data.len() as u64, offset: 0, elem: b'B' }),
            has_data_column: true,
            zscale: image.zscale,
            zzero: image.zzero,
            zblank: image.zblank,
            quantized: true,
        };
        decode(&image, index, &row, &data)
    }

    #[test]
    fn no_dither_is_a_scale_and_a_zero_point() {
        let t = quantized("", &[0, 4, i32::MIN], 1, 0);
        assert_eq!(t.pixels[..2], [10.0, 12.0]);
        assert!(t.pixels[2].is_nan());
        let whats: Vec<&str> = t.steps.iter().map(|s| s.what.as_str()).collect();
        assert_eq!(whats, ["stored", "read", "NO_DITHER", "blank"]);
        assert_eq!(t.steps[2].note, "float = integer × 0.5 + 10 (ZSCALE and ZZERO for this tile)");
        assert_eq!(t.steps[3].note, "1 pixel stored as ZBLANK = -2147483648, which means blank, read as NaN");
    }

    #[test]
    fn dithering_takes_its_random_numbers_from_where_the_tile_and_the_seed_say() {
        let r = randoms();
        // Tile 2 with ZDITHER0 5 starts at iseed 6.
        let t = quantized("SUBTRACTIVE_DITHER_1", &[4, 4, -2147483646], 5, 2);
        let start = (r[6] * 500.0) as usize;
        let want = |q: f64, k: usize| f64::from(((q - f64::from(r[start + k]) + 0.5) * 0.5 + 10.0) as f32);
        assert_eq!(t.pixels[0], want(4.0, 0));
        assert_eq!(t.pixels[1], want(4.0, 1));
        // Only the second method keeps a zero.
        assert_eq!(t.pixels[2], want(-2147483646.0, 2));
        let t = quantized("SUBTRACTIVE_DITHER_2", &[4, -2147483646, 4], 5, 2);
        assert_eq!((t.pixels[1], t.pixels[2]), (0.0, want(4.0, 2)));
    }

    #[test]
    fn the_random_numbers_wrap_to_the_next_seed_when_they_run_out() {
        let r = randoms();
        // A tile long enough to run off the end of the sequence.
        let start = (r[0] * 500.0) as usize;
        let n = N_RANDOM - start + 3;
        let ints = vec![0i32; n];
        let t = quantized("SUBTRACTIVE_DITHER_1", &ints, 1, 0);
        let restart = (r[1] * 500.0) as usize;
        let want = |k: usize| f64::from(((0.0 - f64::from(r[k]) + 0.5) * 0.5 + 10.0) as f32);
        assert_eq!(t.pixels[N_RANDOM - start - 1], want(N_RANDOM - 1));
        assert_eq!(t.pixels[N_RANDOM - start], want(restart));
    }

    #[test]
    fn an_algorithm_without_a_decoder_is_named() {
        let image = Image::from_cards(&cards(&["ZBITPIX =                   16", "ZNAXIS  =                    1", "ZNAXIS1 =                    4", "ZCMPTYPE= 'HCOMPRESS_1'"])).unwrap();
        let row = Row {
            place: Some(Place { stored: Stored::Compressed, count: 3, offset: 0, elem: b'B' }),
            has_data_column: true,
            zscale: None,
            zzero: None,
            zblank: None,
            quantized: false,
        };
        let t = decode(&image, 0, &row, &[1, 2, 3]);
        assert!(t.pixels.is_empty());
        assert!(t.problem.unwrap().contains("HCOMPRESS_1"));
    }
}
