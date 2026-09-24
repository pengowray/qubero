//! Unpacking a compressed run so the fields inside it can be read.
//!
//! A file that holds a compressed stream holds structure the reader cannot see:
//! a ROOT record is a nine-byte block header and then a zlib stream, and
//! everything the record is *for* is on the other side of it. Reading the run
//! as `bytes[3824]` is honest and useless.
//!
//! So a template may say what a run is compressed with, and the reading opens
//! it. The compressed bytes stay exactly where they are and stay exactly as
//! long as they are; what comes out of them is a second address space, and the
//! fields declared over it count from its own start. See
//! [`Ty::Decoded`](crate::template::Ty::Decoded).
//!
//! Every decoder here is pure Rust and builds for wasm32. Nothing streams: a
//! stream is opened whole or not at all, which is why there is a cap.

pub mod bzip2;
pub mod cfb;
pub mod cdfhuff;
pub mod cdfrle;
pub mod compress;
pub mod fastlz;
pub mod frames;
pub mod inflate;
pub mod jpeg;
pub mod lha;
pub mod lzma;
pub mod lz4;
pub mod pico8;
pub mod pixels;
pub mod pxu;
pub mod pytext;
pub mod rar5;
pub mod scanlines;
pub mod snappy;
pub mod xz;

use std::ops::Range;

/// The largest a decoded stream may come to. Past this the run is left as the
/// bytes it is and the node says why: a zip bomb is one line in a file and
/// gigabytes in memory, and a hex editor that opens one has stopped being a
/// hex editor.
pub const CAP_BYTES: usize = 64 * 1024 * 1024;

/// What a run is compressed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// The Workbook/Book stream of an OLE compound document, following FAT
    /// and mini-FAT chains rather than assuming adjacent sectors.
    CfbWorkbook,
    /// Not compressed at all: the bytes come out as they went in.
    ///
    /// An archive that stores a file rather than packing it has written that
    /// file into itself verbatim, so the bytes are already a document. Saying
    /// so with a codec rather than a flag of its own is what makes them a
    /// document everywhere at once: `Decoded` is what carries `space_root`,
    /// which is what the listing hangs Open unpacked off, what the hex view's
    /// chips mark, and what opens the tab. A ZIP entry written with `-0`, a
    /// RAR file stored with method 0x30 and a 7z entry packed with `copy` are
    /// all this.
    ///
    /// The trace is one step over the whole run, which is the truth: every
    /// byte out came from the byte in front of it.
    Stored,
    /// RFC 1950: two header bytes, deflate, an Adler-32.
    Zlib,
    /// RFC 1951 on its own, with nothing wrapped round it.
    Deflate,
    Zstd,
    /// One LZ4 block, with no frame header and no length in front of it. What
    /// ROOT hands to LZ4 and what an LZ4 frame's blocks hold.
    Lz4Block,
    /// One or more LZ4 frames: a magic, a descriptor, blocks each behind a
    /// size, an end mark, and whichever checksums the descriptor asked for.
    /// What Arrow compresses a buffer into when its body says LZ4_FRAME.
    ///
    /// Read whole rather than a block at a time, because a frame may link its
    /// blocks and a linked block copies from the ones before it. See
    /// [`crate::codec::lz4::frame`].
    Lz4Frame,
    /// One raw Snappy block: a varint saying how many bytes come out, and then
    /// tags to the end of the run.
    ///
    /// Not the framing format, which is the one with a stream identifier and a
    /// checksum per chunk. A Parquet page, a ROOT basket and every other place
    /// that says SNAPPY without saying framed holds this.
    Snappy,
    /// A whole Brotli stream. What Parquet's BROTLI codec packs a page with,
    /// and what a `Content-Encoding: br` response body is.
    ///
    /// Traced as one step over the run. Brotli is a context-modelled Huffman
    /// format with its own block-splitting and a built-in dictionary, and the
    /// crate that reads it will not say where its blocks were, so the map here
    /// is the honest one: these bits made those bytes, and no finer.
    Brotli,
    Xz,
    /// A whole lzip member, header and all.
    ///
    /// The run is the member rather than the LZMA stream inside it, because
    /// the stream cannot be read without the byte in front of it: LZMA1 has no
    /// header of its own, and the dictionary size and the packing of literals
    /// are things the container says. The template lays its fields over the
    /// same bytes, which is what `xz` does for the same reason.
    Lzip,
    /// A raw LZMA1 stream, given the three things it does not carry: how it
    /// was packed, how large a dictionary it wants, and how much comes out.
    ///
    /// The first codec here whose settings differ from file to file rather
    /// than from format to format. Every other one is the same arithmetic
    /// wherever it appears, so a template naming it says everything there is
    /// to say; a 7z coder writes its properties into the header, and two
    /// archives made by the same archiver on the same day can differ. See
    /// [`Packing`](crate::template::Packing), which is how a template says
    /// where the numbers are rather than what they are.
    ///
    /// `unpacked` is what the container says comes out. `None` reads to the
    /// end-of-stream marker, which lzip writes and 7z does not.
    Lzma1 { props: u8, dict_size: u32, unpacked: Option<u64> },
    /// LZMA2: chunks that carry their own packing, so unlike
    /// [`Codec::Lzma1`] nothing outside the stream has to describe it. What
    /// 7z packs with by default and what an xz block holds.
    Lzma2,
    /// One LHA entry's data, packed the way its method string names.
    ///
    /// `-lh5-` and its neighbours are LZSS against a window, with the matches
    /// and the literals under Huffman codes rebuilt every few thousand
    /// symbols. What differs between `-lh4-`, `-lh5-`, `-lh6-` and `-lh7-` is
    /// how far back a match may reach and nothing else, so the window is the
    /// setting and the method string is where a template reads it.
    ///
    /// `-lh1-` is not this: it packs against a 4K window with an adaptive
    /// Huffman tree that changes with every symbol, which is a different
    /// decoder, and it stays bytes.
    Lha { window_bits: u8 },
    /// One RAR 5 entry's data, unpacked.
    ///
    /// LZSS against a window under five Huffman tables, with three
    /// byte-transforming filters over the result. The second codec here whose
    /// settings are fields rather than facts about the format, and for a
    /// sharper reason than [`Codec::Lzma1`]'s: RAR writes no end-of-stream
    /// marker at all, so `unpacked` is not a convenience but the only thing
    /// that says where the file stops. `window_bits` is the dictionary the
    /// header declared, as a power of two from 17, and bounds how far back a
    /// match may reach.
    ///
    /// Only a self-contained entry: not solid, not encrypted, not split across
    /// volumes. Those need what came before this run, and this is handed a run.
    /// The template turns them away rather than decoding them wrongly. See
    /// [`crate::codec::rar5`].
    Rar5 { window_bits: u8, unpacked: u64 },
    /// A whole bzip2 stream, from its `BZh` onwards.
    ///
    /// The run is the stream and not a block: bzip2 packs its blocks to the
    /// bit, so only the first one starts on a byte and only the whole stream
    /// can be handed to a decoder.
    Bzip2,
    /// A whole `.Z` file: the two magic bytes, the flags, and LZW codes packed
    /// from the low bit up at a width that grows as the table fills.
    Compress,
    /// A whole gzip member, header and all. What Godot writes for its third
    /// compression mode, and the wrapper `gzip.rs` reads as fields.
    Gzip,
    /// One FastLZ block, with no header and no length in front of it. The
    /// first of Godot's five compression modes, and level 1 and level 2 of the
    /// same format told apart by the top bits of the first byte.
    ///
    /// The mode a Godot file is likeliest not to be in: the resource saver
    /// leaves the choice at its default, which is zstd. It is what
    /// `PackedByteArray.compress` picks when nothing says otherwise, though,
    /// so it reaches a file wherever a project asked for it by name.
    FastLz,
    /// Not compression: PNG's per-row filtering, undone. What comes out of an
    /// IDAT's zlib stream is rows of `1 + stride` bytes, a filter byte and a
    /// row predicted from its neighbours; what comes out of this is the
    /// pixels. `bpp` is the bytes in a pixel, which is how far back a filter
    /// looks for the byte to its left.
    PngUnfilter { stride: u32, bpp: u8 },
    /// Not compression either: one byte out of the low two bits of each
    /// channel of an RGBA pixel, alpha first. How a PICO-8 cartridge is
    /// carried inside the picture of its label.
    LowBitsArgb,
    /// The same trick at a different width: eleven bits out of one RGBA pixel,
    /// three from red, three from green, three from blue and two from alpha,
    /// packed into a byte stream low bits first. How a Picotron cartridge is
    /// carried inside the picture of its label.
    LowBitsRgba11,
    /// PICO-8's `\0pxa` code compression, written by PICO-8 0.2.0 and after.
    /// A bit stream of move-to-front literals and back-references. The run
    /// handed here is the stream alone: the eight header bytes in front of it
    /// are fields of the cart and are read by the template, not by this.
    Pico8Pxa,
    /// PICO-8's older `:c:\0` code compression: a byte stream of table
    /// indices, escaped bytes and two-byte back-references, ending at a pair
    /// of zero bytes. The run handed here starts after the same eight header
    /// bytes.
    Pico8Old,
    /// Picotron's `pxu` userdata encoding, which sits inside a POD's text
    /// where a `userdata()` value would be. The run handed here starts at the
    /// `pxu\0` and may be longer than the elements need.
    PicotronPxu,
    /// NASA CDF's run-length encoding, which counts runs of zeroes and nothing
    /// else: a zero byte escapes the count that follows it, and every other
    /// byte is itself. One of the four ways a CDF may be squeezed, and the one
    /// a file written by IDL usually is. See [`crate::codec::cdfrle`].
    CdfRle,
    /// NASA CDF's Huffman coding, compression type 2: counts for the bytes
    /// written in front, a tree built from them, and a code per byte after.
    /// See [`crate::codec::cdfhuff`].
    CdfHuffman,
    /// NASA CDF's adaptive Huffman coding, compression type 3: no table in
    /// front, and a tree that encoder and decoder both change after every
    /// byte. See [`crate::codec::cdfhuff`].
    CdfAhuff,
    /// Not compression: text whose characters are each one byte of what it
    /// stands for, which is how a pickle below protocol 3 carries a run of
    /// bytes. The run is UTF-8 and every character in it is under 0x100. See
    /// [`crate::codec::pytext`].
    Latin1Text,
    /// The same text written as a protocol 0 line, so `raw-unicode-escape`'s
    /// escaping comes off before the characters do.
    EscapedLatin1Text,
    /// Not compression: a whole PNG image's scanlines unfiltered, with the
    /// row lengths worked out from the header rather than fixed in the
    /// template. What [`Codec::PngUnfilter`] cannot be for an ordinary PNG,
    /// whose rows are as long as its width, depth and colour type make them,
    /// and for an interlaced one, which is seven images of different widths
    /// one after the other. See [`crate::codec::scanlines`].
    ///
    /// `bits_per_pixel` is the depth times the samples a pixel has. What comes
    /// out is every pass's unfiltered rows in the order they were written, so
    /// an interlaced image comes out in pass order and not in picture order.
    PngScanlines { width: u32, height: u32, bits_per_pixel: u8, interlace: bool },
    /// One scan of a sequential Huffman-coded JPEG, read into the quantized
    /// coefficients of its 8×8 blocks.
    ///
    /// The first codec here whose settings are not numbers at all. Deflate
    /// carries its tables inside the stream; a JPEG scan carries none of
    /// them. The frame, the Huffman tables, the quantization tables and the
    /// restart interval are segments the file wrote before the scan, so this
    /// is never opened from its own bytes alone: see
    /// [`Packing::JpegScan`](crate::template::Packing::JpegScan) and
    /// [`decode_traced_with`]. Asked to open a run with nothing more,
    /// it says [`Refusal::Settings`].
    ///
    /// What comes out is 64 little-endian `i16` per block, in rows, the
    /// blocks in the order the scan codes them. See [`crate::codec::jpeg`].
    JpegBaseline,
}

impl Codec {
    pub fn as_str(self) -> &'static str {
        match self {
            Codec::CfbWorkbook => "cfb-workbook",
            Codec::Stored => "stored",
            Codec::Zlib => "zlib",
            Codec::Deflate => "deflate",
            Codec::Zstd => "zstd",
            Codec::Lz4Block => "lz4 block",
            Codec::Lz4Frame => "lz4 frame",
            Codec::Snappy => "snappy",
            Codec::Brotli => "brotli",
            Codec::Xz => "xz",
            Codec::Lzip => "lzip",
            Codec::Lzma1 { .. } => "lzma",
            Codec::Lzma2 => "lzma2",
            Codec::Lha { .. } => "lzhuf",
            Codec::Rar5 { .. } => "rar5",
            Codec::Bzip2 => "bzip2",
            Codec::Compress => "compress",
            Codec::Gzip => "gzip",
            Codec::FastLz => "fastlz",
            Codec::PngUnfilter { .. } => "png unfilter",
            Codec::LowBitsArgb => "low bits argb",
            Codec::LowBitsRgba11 => "low bits rgba 11",
            Codec::Pico8Pxa => "pico-8 pxa",
            Codec::Pico8Old => "pico-8 old code",
            Codec::PicotronPxu => "picotron pxu",
            Codec::CdfRle => "cdf rle",
            Codec::CdfHuffman => "cdf huffman",
            Codec::CdfAhuff => "cdf adaptive huffman",
            Codec::Latin1Text => "latin-1 text",
            Codec::EscapedLatin1Text => "latin-1 text, escaped",
            Codec::PngScanlines { .. } => "png scanlines",
            Codec::JpegBaseline => "jpeg baseline",
        }
    }
}

/// Why a run was left as bytes. A kind rather than a sentence: the core says
/// what happened and the interface says it in words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The run, or what it would come to, is past [`CAP_BYTES`].
    TooLarge,
    /// The decoder would not read it.
    Failed,
    /// The run does not start on a byte, and no decoder reads half a byte.
    Unaligned,
    /// How the run was packed could not be worked out, so no decoder was
    /// asked. Told apart from [`Refusal::Failed`] on purpose: nothing here
    /// tried to read these bytes and nothing here is saying they are wrong.
    ///
    /// What a codec whose settings are fields runs into. See
    /// [`Packing`](crate::template::Packing): a 7z coder writes how it packed
    /// a stream into the archive's header, and a coder that packed nothing,
    /// or one this cannot read the properties of, leaves the numbers with
    /// nowhere to come from. A reader told "unpacking failed" there would go
    /// looking for damage in a file that has none.
    Settings,
    /// The run is a kind of stream this decoder was not written for, and the
    /// file says which. Nothing was decoded: a JPEG scan that is progressive
    /// or arithmetic-coded is a different bit stream from a baseline one, and
    /// reading it with the baseline rules gives coefficients that look right
    /// and are not.
    Unsupported(Unsupported),
}

/// Which kind of stream a decoder turned away, for [`Refusal::Unsupported`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsupported {
    /// A JPEG frame that sends its coefficients over several scans, a band
    /// or a bit at a time.
    Progressive,
    /// A JPEG frame coded with the arithmetic coder rather than with Huffman
    /// codes.
    Arithmetic,
    /// A lossless JPEG frame, which codes predicted samples and has no DCT.
    Lossless,
    /// A JPEG frame that is one layer of a hierarchical image, coded as the
    /// difference from the layer before it.
    Hierarchical,
    /// A JPEG frame of 12-bit samples.
    Precision12,
}

impl Unsupported {
    /// What the stream is and that it was not decoded, for the few places in
    /// the core that write a refusal out in words rather than handing the
    /// interface its tag.
    pub fn message(self) -> &'static str {
        match self {
            Unsupported::Progressive => "progressive JPEG, which Qubero doesn't decode",
            Unsupported::Arithmetic => "arithmetic-coded JPEG, which Qubero doesn't decode",
            Unsupported::Lossless => "lossless JPEG, which Qubero doesn't decode",
            Unsupported::Hierarchical => "hierarchical JPEG, which Qubero doesn't decode",
            Unsupported::Precision12 => "12-bit JPEG, which Qubero doesn't decode",
        }
    }
}

impl Refusal {
    /// The word the interface looks the message up by.
    pub fn as_str(self) -> &'static str {
        match self {
            Refusal::TooLarge => "too-large",
            Refusal::Failed => "failed",
            Refusal::Settings => "settings",
            Refusal::Unaligned => "unaligned",
            Refusal::Unsupported(Unsupported::Progressive) => "progressive",
            Refusal::Unsupported(Unsupported::Arithmetic) => "arithmetic",
            Refusal::Unsupported(Unsupported::Lossless) => "lossless",
            Refusal::Unsupported(Unsupported::Hierarchical) => "hierarchical",
            Refusal::Unsupported(Unsupported::Precision12) => "12-bit",
        }
    }
}

/// How many steps a trace may hold before it stops recording one per symbol.
///
/// A 64 MiB deflate output is tens of millions of literals, and a step apiece
/// is more memory than the bytes they describe. Past this the decoder keeps
/// decoding and keeps the map, but a block's symbols are recorded as one
/// [`StepKind::Opaque`] step covering the whole run: the trace still tiles the
/// input and the output, it just stops naming every byte.
pub const MAX_STEPS: usize = 4_000_000;

/// Which named field of a header or a sequence a step is.
///
/// Not one enum per codec: a step is a step, and the interface wants a word
/// for it rather than a shape to match on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepField {
    /// Deflate: the bit that says this is the last block.
    Bfinal,
    /// Deflate: stored, fixed Huffman, or dynamic Huffman.
    Btype,
    /// Deflate: how many literal/length code lengths follow, less 257.
    Hlit,
    /// Deflate: how many distance code lengths follow, less 1.
    Hdist,
    /// Deflate: how many code-length code lengths follow, less 4.
    Hclen,
    /// A stored block's length, and its one's complement.
    StoredLen,
    StoredNlen,
    /// The bits between the last block and the byte boundary, which a decoder
    /// reads past and nothing means.
    Padding,
    /// Bytes before or after the deflate stream a wrapper put there: zlib's
    /// two header bytes and its Adler-32.
    Wrapper,
    /// LZ4 and FastLZ: the byte that says whether literals or a match follow,
    /// and how much of each.
    Token,
    /// The bytes extending a length past what the token could hold.
    LengthExtra,
    /// How far back the match reads.
    Offset,
    /// zstd, xz: a frame header.
    FrameHeader,
    /// zstd, xz: one block's header.
    BlockHeader,
    /// PNG: the byte in front of a scanline saying how the row was predicted.
    Filter,
    /// xz: a block's integrity check, and the index and stream footer
    /// together. Its value is how many bytes the check takes; the index and
    /// the footer say nothing, being measured by the step after them.
    Footer,
    /// RAR 5: a symbol that produces no byte and instead names a run of the
    /// output and one of three transforms to run over it once that run has been
    /// unpacked. Its value is which transform.
    FilterDef,
    /// pxu: the two bytes saying the element type, whether a height follows
    /// the width, how wide the sizes are, and which compression was used.
    PxuFlags,
    /// pxu: how many elements a row is, and how many rows there are.
    PxuWidth,
    PxuHeight,
    /// pxu: how many of a token's low bits are an index into the table of
    /// elements written before.
    PxuBits,
    /// LZMA: the byte packing the literal context bits, the literal position
    /// bits and the position bits. Its value is that byte.
    ///
    /// Often a step of no width at all. LZMA1 carries nothing in front of its
    /// stream, so lzip fixes these numbers by convention and 7z writes them
    /// into the archive's header: the step says what the decoder was told
    /// without claiming the run holds it. In LZMA2 a chunk that resets the
    /// properties does carry the byte, and there the step has the width of it.
    LzmaProps,
    /// LZMA: the five bytes that prime the range coder. The first is ignored
    /// and the other four are the interval the stream starts inside.
    RangeInit,
    /// Snappy: the varint in front of a block saying how many bytes it comes
    /// to. Its value is that number.
    UnpackedSize,
    /// CDF Huffman: the runs of byte counts in front of the codes, which the
    /// tree the codes are read by is built from. Its value is how many byte
    /// values were given a count.
    FrequencyTable,
    /// LZ4 frame: the xxHash-32 after a block, of the bytes the block holds.
    /// Its value is the checksum as written, a little-endian word.
    BlockChecksum,
    /// LZ4 frame: the xxHash-32 after the end mark, of everything the frame
    /// came to. Its value is the checksum as written.
    ContentChecksum,
    /// PNG: which of Adam7's seven passes a scanline belongs to, 1 to 7. A
    /// step of no width: the file writes it nowhere, and it is where the row
    /// falls in the stream that says it, so the step says what the decoder
    /// worked out without claiming the run holds it, as
    /// [`StepField::LzmaProps`] does for a 7z coder's settings.
    Pass,
    /// PNG: which row of its pass a scanline is, from 0, and of no width for
    /// the same reason. In an image that is not interlaced it is the row of
    /// the picture.
    Row,
    /// JPEG: a restart marker between two intervals of a scan, `ff d0` to
    /// `ff d7`. Its value is the marker's number, 0 to 7, which counts the
    /// intervals round and round so a decoder can tell one went missing.
    Restart,
}

impl StepField {
    /// The word the interface looks the field's name up by.
    pub fn as_str(self) -> &'static str {
        match self {
            StepField::Bfinal => "bfinal",
            StepField::Btype => "btype",
            StepField::Hlit => "hlit",
            StepField::Hdist => "hdist",
            StepField::Hclen => "hclen",
            StepField::StoredLen => "len",
            StepField::StoredNlen => "nlen",
            StepField::Padding => "padding",
            StepField::Wrapper => "wrapper",
            StepField::Token => "token",
            StepField::LengthExtra => "length_extra",
            StepField::Offset => "offset",
            StepField::FrameHeader => "frame_header",
            StepField::BlockHeader => "block_header",
            StepField::Filter => "filter",
            StepField::Footer => "footer",
            StepField::FilterDef => "filter_def",
            StepField::PxuFlags => "pxu_flags",
            StepField::PxuWidth => "pxu_width",
            StepField::PxuHeight => "pxu_height",
            StepField::PxuBits => "pxu_bits",
            StepField::LzmaProps => "lzma_props",
            StepField::RangeInit => "range_init",
            StepField::UnpackedSize => "unpacked_size",
            StepField::FrequencyTable => "frequency_table",
            StepField::BlockChecksum => "block_checksum",
            StepField::ContentChecksum => "content_checksum",
            StepField::Pass => "pass",
            StepField::Row => "row",
            StepField::Restart => "restart",
        }
    }
}

/// One entry of a Huffman table as the decoder read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableField {
    /// One of the code-length alphabet's own lengths, three bits: which
    /// symbol of that alphabet, and how long its code is.
    CodeLen { sym: u8, len: u8 },
    /// A literal/length code length, given outright.
    LitLen { sym: u16, len: u8 },
    /// A distance code length, given outright.
    Dist { sym: u16, len: u8 },
    /// Code 16, 17 or 18: repeat the length before it, or a run of zeroes.
    /// `count` symbols get `len`, and `dist` says which table they fill.
    Repeat { code: u8, count: u16, len: u8, dist: bool },
}

/// What a decoder did over one stretch of its input.
///
/// The one source of both the map between the two spaces and the fields the
/// compressed space shows: nothing is drawn that the decoder did not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// A named field of a block or frame header, and what it said. A field
    /// that says nothing on its own -- padding, a wrapper's bytes -- says 0.
    Header(StepField, u32),
    /// A Huffman code length, or a run of them.
    Table(TableField),
    /// One byte, given as itself.
    Literal(u8),
    /// `len` bytes copied from `dist` bytes back in the output.
    Match { len: u32, dist: u32 },
    /// Bytes copied through as they came: a deflate stored block's payload,
    /// an LZ4 literal run.
    Stored,
    /// Deflate symbol 256.
    EndOfBlock,
    /// A whole block, for a codec whose insides this round does not read.
    Block,
    /// Input the decoder read and this trace does not name.
    Opaque,
    /// One pixel, read for the bits somebody hid in it. Its output range is
    /// the bytes that pixel completed, which is one for a cart's pixels and
    /// one or two where a pixel carries eleven bits.
    Pixel,
    /// A row of bytes each written as its difference from a guess made from
    /// the bytes beside and above it: a PNG scanline after its filter byte.
    /// As many bytes out as in, and none of them is the byte it stands for.
    Filtered,
    /// JPEG: the first coefficient of a block, written as its difference from
    /// the same channel's previous block. A Huffman code of `code` bits whose
    /// symbol is `size`, then `size` bits of the difference itself. `dc` is
    /// the coefficient the difference came to, which no bit of the file holds
    /// and which is the one number a reader of the block wants.
    Dc { code: u8, size: u8, diff: i16, dc: i16 },
    /// JPEG: one nonzero coefficient after the first. A Huffman code of `code`
    /// bits whose symbol packs `run`, how many zeros come before it, and
    /// `size`, how many bits the value takes; then those bits. `k` is where it
    /// lands in the block's zigzag order, 1 to 63.
    Ac { code: u8, run: u8, size: u8, k: u8, value: i16 },
    /// JPEG: sixteen zeros, the symbol `0xf0`, written where a run of zeros is
    /// too long for one coefficient's code to say. `k` is where the first of
    /// them lands.
    Zrl { code: u8, k: u8 },
    /// JPEG: the symbol `0x00`, which says every coefficient from zigzag
    /// position `k` to 63 is zero. Not deflate's [`StepKind::EndOfBlock`]: that
    /// ends a block of the stream, and this ends one 8×8 block of a picture.
    Eob { code: u8, k: u8 },
}

impl StepKind {
    /// The word the interface looks the step's message up by.
    pub fn as_str(self) -> &'static str {
        match self {
            StepKind::Header(..) => "header",
            StepKind::Table(_) => "table",
            StepKind::Literal(_) => "literal",
            StepKind::Match { .. } => "match",
            StepKind::Stored => "stored",
            StepKind::EndOfBlock => "end-of-block",
            StepKind::Block => "block",
            StepKind::Opaque => "opaque",
            StepKind::Pixel => "pixel",
            StepKind::Filtered => "filtered",
            StepKind::Dc { .. } => "dc",
            StepKind::Ac { .. } => "ac",
            StepKind::Zrl { .. } => "zrl",
            StepKind::Eob { .. } => "eob",
        }
    }

    /// How many bits of a step are its Huffman code, for a step that is one
    /// code and the value bits after it. What a reader needs to tell the two
    /// apart, since only the code is looked up in a table.
    pub fn code_bits(self) -> Option<u8> {
        match self {
            StepKind::Dc { code, .. } | StepKind::Ac { code, .. } | StepKind::Zrl { code, .. } | StepKind::Eob { code, .. } => {
                Some(code)
            }
            _ => None,
        }
    }
}

/// One step of a decoding: which bits of the input it read and which bytes of
/// the output it produced.
///
/// `in_bits` are bits of the compressed run counted from its front, in the
/// order the codec reads them, which is not always how Qubero addresses bits.
/// Deflate takes the least significant bit of a byte first, so its bit 0 is
/// the *low* bit of byte 0; Qubero's bit 0 is the high bit of byte 0. Within a
/// byte that is the reverse of a field address, and across bytes the two
/// agree, so the byte extent of a step is the same either way and only a
/// sub-byte highlight differs. Byte-aligned codecs have no such question.
///
/// [`Trace::lsb_first`] says which way this trace counted, for the one caller
/// that has to turn a step's bits back into the bits themselves rather than
/// into a range.
///
/// Either range may be empty: a header field produces no output, and a match
/// of length 3 reads no input of its own past the code that named it.
///
/// The bits are the run's own, every byte of it counted, even where the
/// decoder read the run through a transform that takes bytes out. A JPEG scan
/// writes every `ff` of its data as `ff 00`, and the zero is not data: the
/// decoder reads past it, and a code may begin before it and end after it.
/// Such a step's range covers the zero, since the range is where the code is
/// in the file, and [`Trace::stuffed`] says which bytes inside it are not the
/// code's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub in_bits: Range<u64>,
    pub out_bytes: Range<u64>,
    pub kind: StepKind,
}

/// A step as it is kept: starts only, with the ends taken from the step after
/// it. Twenty bytes rather than fifty-six, which is the difference between a
/// large file's trace fitting in memory and not.
#[derive(Debug, Clone, Copy)]
struct RawStep {
    in_start: u32,
    out_start: u32,
    a: u32,
    b: u32,
    tag: u8,
}

/// How a deflate block said its symbols were coded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Stored,
    Fixed,
    Dynamic,
    /// An LZ4 block, which has no blocks inside it and no tables: one run of
    /// sequences from the front of it to the back.
    Sequences,
    /// Pixels, read for the bits somebody hid in them. Not a compressed block
    /// at all: one run of pixels from the front of the image to the back.
    Pixels,
    /// A block whose insides this round does not read: a zstd or xz block.
    Opaque,
    /// One PNG scanline: which pass and row it is, its filter byte, and the
    /// filtered row. See [`crate::codec::scanlines`].
    Scanline,
    /// A JPEG MCU, the minimum coded unit: one 8×8 block of every channel the
    /// scan carries, or several of a channel sampled more finely than the
    /// others, coded one after another. `x` and `y` count MCUs across and
    /// down the picture. Its 8×8 blocks are the trace's [`Unit`]s.
    Mcu { x: u16, y: u16 },
}

impl BlockKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BlockKind::Stored => "stored",
            BlockKind::Fixed => "fixed",
            BlockKind::Dynamic => "dynamic",
            BlockKind::Sequences => "sequences",
            BlockKind::Pixels => "pixels",
            BlockKind::Opaque => "opaque",
            BlockKind::Scanline => "scanline",
            BlockKind::Mcu { .. } => "mcu",
        }
    }
}

/// A run of steps inside a block that the codec reads as a thing of its own:
/// one 8×8 block of a JPEG MCU, which T.81 calls a data unit.
///
/// A second level of grouping below [`Block`], because a JPEG scan has two and
/// a reader wants both. An MCU is what restart intervals count and what a map
/// of where the bits went is drawn in; an 8×8 block is what the coefficients
/// belong to and what one channel of the picture is made of. Neither is the
/// other: a 4:2:0 MCU is six blocks of three channels.
///
/// `channel` is the block's component, by its place in the frame rather than
/// by its id; `x` and `y` count that channel's blocks across and down. What
/// comes out of the unit is its 64 coefficients, which the unit's last step
/// carries: see [`crate::codec::jpeg`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// Indices into the trace's steps, inside one block's.
    pub steps: Range<u32>,
    pub channel: u8,
    pub x: u16,
    pub y: u16,
}

/// One block of a decoding, and which steps belong to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Indices into the trace's steps.
    pub steps: Range<u32>,
    pub kind: BlockKind,
    /// Whether the format said this was the last one.
    pub last: bool,
    pub in_bits: Range<u64>,
    pub out_bytes: Range<u64>,
}

/// One member of a run: a piece the *container* produced as a unit, rather
/// than a piece the coder made while producing it.
///
/// The two are different divisions of the same bytes and both are real. An xz
/// stream is a list of blocks, each compressed on its own and each sealed with
/// its own integrity check over what it came to; inside one of those blocks
/// the LZMA2 coder has chunks of its own, and those are what [`Block`] holds.
/// One xz block is many LZMA2 chunks, and a stream that fell back to a crate
/// has no chunks at all and still has its blocks. So a member cannot be read
/// off the blocks, off the steps, or off anything else in the trace: the
/// container writes it down while it walks its own headers, or nobody knows
/// it. gzip's word for the same thing is a member, which is where the name
/// comes from.
///
/// What it is for is the checks. An xz block's check covers that block's
/// slice of the output and no other, and the offset of that slice is the
/// running sum of every earlier block's size, which is not a number any field
/// of the file holds. See [`Covers::UnpackedMember`](crate::template::Covers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Which bits of the run produced those bytes: the member's packed data,
    /// without the header in front of it or the check behind it. What a reader
    /// is sent to when the thing being talked about is nowhere in the file,
    /// the way the summed bytes of a check over unpacked data are.
    pub in_bits: Range<u64>,
    /// Which bytes of the output it produced.
    pub out_bytes: Range<u64>,
}

/// Everything a decoder recorded about one run.
///
/// The steps tile the input bits and the output bytes exactly: step `i`'s
/// ranges end where step `i + 1`'s begin, the first begins at zero, and the
/// last ends at the run's own length. That is asserted rather than assumed,
/// by [`Trace::check_tiles`].
#[derive(Debug, Clone, Default)]
pub struct Trace {
    steps: Vec<RawStep>,
    blocks: Vec<Block>,
    members: Vec<Member>,
    end_in_bits: u64,
    end_out_bytes: u64,
    /// Whether the trace gave up naming every symbol; see [`MAX_STEPS`].
    coarse: bool,
    /// Whether `in_bits` count the low bit of a byte first. See [`Step`].
    lsb_first: bool,
    /// The blocks' own pieces, for a codec whose blocks have them. See
    /// [`Unit`].
    units: Vec<Unit>,
    /// Where the bytes the decoder read past sit, as the bit each starts at.
    /// See [`Trace::stuffed`].
    stuffed: Vec<u64>,
    /// What a JPEG scan's decoder settled before it read a bit: the frame's
    /// size and channels, and which tables the scan used. Nothing for every
    /// other codec.
    jpeg: Option<std::sync::Arc<jpeg::ScanFacts>>,
}

impl Trace {
    /// The pieces the blocks are read in, in order, for a codec that has
    /// them: a JPEG scan's 8×8 blocks. Empty for every other codec.
    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    /// The units of block `i`, as a range of indices into [`Trace::units`].
    /// A halving, since the units come in the order their blocks do.
    pub fn units_of(&self, i: usize) -> Range<usize> {
        let Some(block) = self.blocks.get(i) else { return 0..0 };
        let from = self.units.partition_point(|u| u.steps.start < block.steps.start);
        let to = self.units.partition_point(|u| u.steps.start < block.steps.end);
        from..to
    }

    /// Where each byte the decoder read past starts, as a bit of the run, in
    /// order: the zero after every `ff` in a JPEG scan's data. Each lies inside
    /// the step that read the last bit of the `ff` before it, so a step's bits
    /// less these bytes are the bits it decoded. Empty for every codec that
    /// reads its run as it is.
    pub fn stuffed(&self) -> &[u64] {
        &self.stuffed
    }

    /// How many of the bytes the decoder read past lie inside `bits`.
    pub fn stuffed_in(&self, bits: Range<u64>) -> usize {
        let from = self.stuffed.partition_point(|&b| b < bits.start);
        let to = self.stuffed.partition_point(|&b| b < bits.end);
        to - from
    }

    /// What a JPEG scan's decoder settled before reading any of it. Nothing
    /// for every other codec.
    pub fn jpeg(&self) -> Option<&jpeg::ScanFacts> {
        self.jpeg.as_deref()
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The container's own pieces of this run, in the order it wrote them.
    /// Empty for every codec that has none to declare, which is all of them
    /// but xz. See [`Member`].
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// Whether the trace stopped naming every symbol because there were too
    /// many of them. The interface says so rather than pretending the file
    /// held no literals.
    pub fn coarse(&self) -> bool {
        self.coarse
    }

    /// Which way round the bits inside a byte are counted, for a caller that
    /// wants the bits of a step and not only the range they cover.
    ///
    /// A range is the same either way, so nothing that highlights or measures
    /// a step has to ask. Writing a code out as the noughts and ones the
    /// decoder read does: deflate's first bit of a code is the low bit of a
    /// byte and LHA's is the high bit, and printing one in the other's order
    /// reverses every code within its byte. False is Qubero's own order, high
    /// bit down, which is also the right answer for a codec that reads whole
    /// bytes.
    pub fn lsb_first(&self) -> bool {
        self.lsb_first
    }

    pub fn in_bits(&self) -> u64 {
        self.end_in_bits
    }

    pub fn out_bytes(&self) -> u64 {
        self.end_out_bytes
    }

    /// Step `i`, built out of its own start and the next one's.
    pub fn step(&self, i: usize) -> Option<Step> {
        let raw = self.steps.get(i)?;
        let (in_end, out_end) = match self.steps.get(i + 1) {
            Some(next) => (next.in_start as u64, next.out_start as u64),
            None => (self.end_in_bits, self.end_out_bytes),
        };
        Some(Step {
            in_bits: raw.in_start as u64..in_end,
            out_bytes: raw.out_start as u64..out_end,
            kind: unpack(*raw),
        })
    }

    pub fn steps(&self) -> impl Iterator<Item = Step> + '_ {
        (0..self.steps.len()).map(move |i| self.step(i).expect("in range"))
    }

    /// Which step produced a byte of the output, as an index.
    ///
    /// A halving rather than a walk: the starts are sorted because the trace
    /// tiles. Steps that produced nothing are skipped, since a byte belongs to
    /// the step that made it and not to the header before it.
    pub fn index_out(&self, byte: u64) -> Option<usize> {
        if byte >= self.end_out_bytes || u32::try_from(byte).is_err() {
            return None;
        }
        let byte = byte as u32;
        // The last step whose out_start is at or below `byte`, then forward
        // past any empty ones sharing that start.
        let mut lo = 0usize;
        let mut hi = self.steps.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.steps[mid].out_start <= byte { lo = mid + 1 } else { hi = mid }
        }
        let mut i = lo.checked_sub(1)?;
        // Walk back to the first step with this start, then forward to the one
        // that is not empty.
        while i > 0 && self.steps[i].out_start == self.steps[i - 1].out_start {
            i -= 1;
        }
        while self.step(i)?.out_bytes.is_empty() {
            i += 1;
        }
        Some(i)
    }

    /// Which step produced a byte of the output.
    pub fn map_out(&self, byte: u64) -> Option<Step> {
        self.step(self.index_out(byte)?)
    }

    /// Which step read a bit of the input, as an index.
    pub fn index_in(&self, bit: u64) -> Option<usize> {
        if bit >= self.end_in_bits || u32::try_from(bit).is_err() {
            return None;
        }
        let bit = bit as u32;
        let mut lo = 0usize;
        let mut hi = self.steps.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.steps[mid].in_start <= bit { lo = mid + 1 } else { hi = mid }
        }
        let mut i = lo.checked_sub(1)?;
        while i > 0 && self.steps[i].in_start == self.steps[i - 1].in_start {
            i -= 1;
        }
        while self.step(i)?.in_bits.is_empty() {
            i += 1;
        }
        Some(i)
    }

    /// Which step read a bit of the input.
    pub fn map_in(&self, bit: u64) -> Option<Step> {
        self.step(self.index_in(bit)?)
    }

    /// That the steps tile the input bits and the output bytes exactly, with
    /// nothing skipped and nothing counted twice. Says what is wrong rather
    /// than panicking, so a test can name the file.
    pub fn check_tiles(&self) -> Result<(), String> {
        let mut at_in = 0u64;
        let mut at_out = 0u64;
        for (i, raw) in self.steps.iter().enumerate() {
            if raw.in_start as u64 != at_in {
                return Err(format!("step {i} reads from bit {} where bit {at_in} was next", raw.in_start));
            }
            if raw.out_start as u64 != at_out {
                return Err(format!("step {i} writes at byte {} where byte {at_out} was next", raw.out_start));
            }
            let s = self.step(i).expect("in range");
            if s.in_bits.end < s.in_bits.start || s.out_bytes.end < s.out_bytes.start {
                return Err(format!("step {i} runs backwards: {s:?}"));
            }
            at_in = s.in_bits.end;
            at_out = s.out_bytes.end;
        }
        if at_in != self.end_in_bits {
            return Err(format!("the steps read {at_in} bits of {}", self.end_in_bits));
        }
        if at_out != self.end_out_bytes {
            return Err(format!("the steps wrote {at_out} bytes of {}", self.end_out_bytes));
        }
        // The members, which are a coarser division of the same output and are
        // held to less: they have to come in order and stay inside the run,
        // and they need not tile it. A check is taken over one of these, so a
        // member reaching past what the decoder produced, or overlapping the
        // one before it, is a slice of somebody else's bytes.
        let mut after = 0u64;
        for (i, m) in self.members.iter().enumerate() {
            if m.out_bytes.start < after || m.out_bytes.end < m.out_bytes.start {
                return Err(format!("member {i} writes {:?} after byte {after}", m.out_bytes));
            }
            if m.out_bytes.end > self.end_out_bytes || m.in_bits.end > self.end_in_bits {
                return Err(format!("member {i} runs past the end of the run: {m:?}"));
            }
            after = m.out_bytes.end;
        }
        // The units, which divide the blocks further and are held to the
        // blocks' order: each inside one block, none overlapping the one
        // before it. They need not tile a block, since a JPEG MCU ends with
        // the padding and the restart marker after its last 8×8 block.
        let mut after = 0u32;
        for (i, u) in self.units.iter().enumerate() {
            if u.steps.start < after || u.steps.end < u.steps.start || u.steps.end as usize > self.steps.len() {
                return Err(format!("unit {i} holds steps {:?} after step {after}", u.steps));
            }
            if !self.blocks.iter().any(|b| b.steps.start <= u.steps.start && u.steps.end <= b.steps.end) {
                return Err(format!("unit {i} is in no one block: {:?}", u.steps));
            }
            after = u.steps.end;
        }
        // And every byte read past lies inside the run, once each, in order.
        if self.stuffed.windows(2).any(|w| w[1] < w[0] + 8) || self.stuffed.last().is_some_and(|&b| b + 8 > self.end_in_bits) {
            return Err("the bytes read past overlap or run past the end of the run".into());
        }
        Ok(())
    }
}

/// What a decoder writes its trace through, so the tiling is kept by
/// construction: a step begins where the one before it ended.
#[derive(Default)]
pub(crate) struct TraceBuilder {
    trace: Trace,
    /// Where the block being recorded started, and its first step.
    block_start: Option<(u64, u64, u32)>,
    /// How many steps this trace may hold before it coarsens. Unset is
    /// [`MAX_STEPS`], which is what a decoder handed a whole run uses.
    ///
    /// Two callers set it. `codec::xz` decodes a stream one block at a time
    /// and gives each block what the ones before it left, so the ceiling is
    /// the stream's rather than the block's; and the tests set it low, since
    /// reaching [`MAX_STEPS`] honestly takes a hundred megabytes of input and
    /// the path that gives up naming symbols is the one path that can leave a
    /// trace not tiling.
    budget: Option<usize>,
}

impl TraceBuilder {
    /// Record a step read from `in_bits` and producing `out_bytes`, both
    /// absolute. The ends are taken from the next step, so they are only
    /// checked here.
    pub(crate) fn push(&mut self, in_start: u64, out_start: u64, kind: StepKind) {
        self.trace.steps.push(pack(in_start, out_start, kind));
    }

    /// Where the trace has got to, which is what the decoder must keep in
    /// step with.
    pub(crate) fn finish_at(&mut self, in_bits: u64, out_bytes: u64) {
        self.trace.end_in_bits = in_bits;
        self.trace.end_out_bytes = out_bytes;
    }

    pub(crate) fn steps(&self) -> usize {
        self.trace.steps.len()
    }

    pub(crate) fn over_budget(&self) -> bool {
        self.trace.steps.len() >= self.budget.unwrap_or(MAX_STEPS)
    }

    pub(crate) fn with_budget(budget: usize) -> TraceBuilder {
        TraceBuilder { budget: Some(budget), ..TraceBuilder::default() }
    }

    /// Take a trace of part of this run into this one, shifted to where that
    /// part sits: `in_bits` is the bit its first byte is at, and `out_bytes`
    /// is how much output everything before it produced.
    ///
    /// What stitching one trace out of several decodings needs, which is what
    /// an xz stream of several blocks is. A step keeps its start and takes its
    /// end from the step after it, so the tiling holds only if the caller
    /// pushes whatever follows at exactly `in_bits + t.in_bits()` and
    /// `out_bytes + t.out_bytes()`. That is the one thing this cannot check
    /// for itself, and [`Trace::check_tiles`] is what catches getting it
    /// wrong.
    ///
    /// The blocks come across renumbered onto the steps they now are, and so
    /// does having given up naming symbols: a trace that is coarse in one of
    /// its parts is coarse.
    pub(crate) fn absorb(&mut self, t: &Trace, in_bits: u64, out_bytes: u64) {
        // Two traces counting the bits inside a byte differently cannot be one
        // trace: every step of one would name the wrong bits. Nothing stitched
        // so far reads bits at all, so this is a guard against a future caller
        // rather than a case that arises.
        debug_assert_eq!(t.lsb_first, self.trace.lsb_first, "traces that count bits differently cannot be stitched");
        let base = self.trace.steps.len() as u32;
        self.trace.steps.extend(t.steps.iter().map(|raw| RawStep {
            in_start: (in_bits + u64::from(raw.in_start)) as u32,
            out_start: (out_bytes + u64::from(raw.out_start)) as u32,
            ..*raw
        }));
        self.trace.blocks.extend(t.blocks.iter().map(|bl| Block {
            steps: bl.steps.start + base..bl.steps.end + base,
            kind: bl.kind,
            last: bl.last,
            in_bits: in_bits + bl.in_bits.start..in_bits + bl.in_bits.end,
            out_bytes: out_bytes + bl.out_bytes.start..out_bytes + bl.out_bytes.end,
        }));
        // Shifted the same way, for the same reason. Nothing stitched so far
        // has any: an LZMA2 stream is not a container and declares none, and
        // the members of an xz stream are written by the stitcher itself.
        self.trace.members.extend(t.members.iter().map(|m| Member {
            in_bits: in_bits + m.in_bits.start..in_bits + m.in_bits.end,
            out_bytes: out_bytes + m.out_bytes.start..out_bytes + m.out_bytes.end,
        }));
        // A trace with pieces inside its blocks, or bytes it read past, is a
        // JPEG scan's, and nothing stitches those; carried the same way all
        // the same, so a future caller cannot lose them.
        self.trace.units.extend(t.units.iter().map(|u| Unit { steps: u.steps.start + base..u.steps.end + base, ..u.clone() }));
        self.trace.stuffed.extend(t.stuffed.iter().map(|b| in_bits + b));
        self.trace.coarse |= t.coarse;
    }

    /// Take the trace of one part of a stream joined from several runs into
    /// this one, shifted to `in_bits` and `out_bytes` as [`Self::absorb`]
    /// shifts.
    ///
    /// Two things differ from `absorb`. The members inside the part are not
    /// carried: the part is itself one member of the joined stream, which the
    /// caller writes, and a member inside it would overlap that one. And the
    /// order the part counted bits in is taken rather than checked, since a
    /// joined stream's parts read none of each other's bits and a stored part
    /// reads whole bytes, which either order counts alike.
    pub(crate) fn absorb_part(&mut self, t: &Trace, in_bits: u64, out_bytes: u64) {
        let members = self.trace.members.len();
        let lsb_first = self.trace.lsb_first;
        self.trace.lsb_first = t.lsb_first;
        self.absorb(t, in_bits, out_bytes);
        self.trace.members.truncate(members);
        self.trace.lsb_first |= lsb_first;
    }

    /// Say that one of the container's own pieces of this run reads from
    /// `in_bits` and comes to `out_bytes`. See [`Member`].
    ///
    /// Written down at the end of the piece rather than opened and closed the
    /// way a block is, because the container knows both ends before it starts:
    /// an xz block's two sizes are in the index, and the decoding is what is
    /// held against them rather than what discovers them.
    pub(crate) fn member(&mut self, in_bits: Range<u64>, out_bytes: Range<u64>) {
        self.trace.members.push(Member { in_bits, out_bytes });
    }

    pub(crate) fn coarsen(&mut self) {
        self.trace.coarse = true;
    }

    /// Say that this decoder's bit reader takes the low bit of a byte first,
    /// which the steps' `in_bits` are then counted in. Called once, before any
    /// step is pushed, by the codecs whose readers do: deflate, LZW and
    /// PICO-8's. A reader that takes the high bit first, and a codec that
    /// reads whole bytes, leave it alone. See [`Trace::lsb_first`].
    pub(crate) fn counts_low_bit_first(&mut self) {
        self.trace.lsb_first = true;
    }

    /// Drop every step from `from` on, so a block's symbols can be replaced by
    /// one step covering all of them.
    pub(crate) fn truncate(&mut self, from: usize) {
        self.trace.steps.truncate(from);
    }

    pub(crate) fn open_block(&mut self, in_bits: u64, out_bytes: u64) {
        self.block_start = Some((in_bits, out_bytes, self.trace.steps.len() as u32));
    }

    pub(crate) fn close_block(&mut self, in_bits: u64, out_bytes: u64, kind: BlockKind, last: bool) {
        let Some((in_start, out_start, step_start)) = self.block_start.take() else { return };
        self.trace.blocks.push(Block {
            steps: step_start..self.trace.steps.len() as u32,
            kind,
            last,
            in_bits: in_start..in_bits,
            out_bytes: out_start..out_bytes,
        });
    }

    /// Say that steps `steps` are one of the block's own pieces. See [`Unit`].
    pub(crate) fn unit(&mut self, steps: Range<u32>, channel: u8, x: u16, y: u16) {
        self.trace.units.push(Unit { steps, channel, x, y });
    }

    /// Say that the decoder read past the byte starting at bit `bit`. See
    /// [`Trace::stuffed`].
    pub(crate) fn stuffed(&mut self, bit: u64) {
        self.trace.stuffed.push(bit);
    }

    pub(crate) fn jpeg(&mut self, facts: jpeg::ScanFacts) {
        self.trace.jpeg = Some(std::sync::Arc::new(facts));
    }

    pub(crate) fn done(self) -> Trace {
        self.trace
    }
}

const TAG_HEADER: u8 = 1;
const TAG_CODELEN: u8 = 2;
const TAG_LITLEN: u8 = 3;
const TAG_DIST: u8 = 4;
const TAG_REPEAT: u8 = 5;
const TAG_LITERAL: u8 = 6;
const TAG_MATCH: u8 = 7;
const TAG_STORED: u8 = 8;
const TAG_END: u8 = 9;
const TAG_BLOCK: u8 = 10;
const TAG_OPAQUE: u8 = 11;
const TAG_PIXEL: u8 = 12;
const TAG_FILTERED: u8 = 13;
// JPEG's start at 20, which leaves the numbers after 12 to kinds added beside
// them: a tag only has to differ from every other tag.
const TAG_DC: u8 = 20;
const TAG_AC: u8 = 21;
const TAG_ZRL: u8 = 22;
const TAG_EOB: u8 = 23;

fn pack(in_start: u64, out_start: u64, kind: StepKind) -> RawStep {
    let (tag, a, b) = match kind {
        StepKind::Header(f, v) => (TAG_HEADER, f as u32, v),
        StepKind::Table(TableField::CodeLen { sym, len }) => (TAG_CODELEN, sym as u32, len as u32),
        StepKind::Table(TableField::LitLen { sym, len }) => (TAG_LITLEN, sym as u32, len as u32),
        StepKind::Table(TableField::Dist { sym, len }) => (TAG_DIST, sym as u32, len as u32),
        StepKind::Table(TableField::Repeat { code, count, len, dist }) => {
            (TAG_REPEAT, code as u32 | (count as u32) << 8 | (len as u32) << 24, dist as u32)
        }
        StepKind::Literal(v) => (TAG_LITERAL, v as u32, 0),
        StepKind::Match { len, dist } => (TAG_MATCH, len, dist),
        StepKind::Stored => (TAG_STORED, 0, 0),
        StepKind::EndOfBlock => (TAG_END, 0, 0),
        StepKind::Block => (TAG_BLOCK, 0, 0),
        StepKind::Opaque => (TAG_OPAQUE, 0, 0),
        StepKind::Pixel => (TAG_PIXEL, 0, 0),
        StepKind::Filtered => (TAG_FILTERED, 0, 0),
        StepKind::Dc { code, size, diff, dc } => {
            (TAG_DC, diff as u16 as u32 | (dc as u16 as u32) << 16, code as u32 | (size as u32) << 8)
        }
        StepKind::Ac { code, run, size, k, value } => {
            (TAG_AC, value as u16 as u32 | (k as u32) << 16, code as u32 | (run as u32) << 8 | (size as u32) << 16)
        }
        StepKind::Zrl { code, k } => (TAG_ZRL, k as u32, code as u32),
        StepKind::Eob { code, k } => (TAG_EOB, k as u32, code as u32),
    };
    RawStep { in_start: in_start as u32, out_start: out_start as u32, a, b, tag }
}

fn unpack(raw: RawStep) -> StepKind {
    match raw.tag {
        TAG_HEADER => StepKind::Header(FIELDS[raw.a as usize], raw.b),
        TAG_CODELEN => StepKind::Table(TableField::CodeLen { sym: raw.a as u8, len: raw.b as u8 }),
        TAG_LITLEN => StepKind::Table(TableField::LitLen { sym: raw.a as u16, len: raw.b as u8 }),
        TAG_DIST => StepKind::Table(TableField::Dist { sym: raw.a as u16, len: raw.b as u8 }),
        TAG_REPEAT => StepKind::Table(TableField::Repeat {
            code: raw.a as u8,
            count: (raw.a >> 8) as u16,
            len: (raw.a >> 24) as u8,
            dist: raw.b != 0,
        }),
        TAG_LITERAL => StepKind::Literal(raw.a as u8),
        TAG_MATCH => StepKind::Match { len: raw.a, dist: raw.b },
        TAG_STORED => StepKind::Stored,
        TAG_END => StepKind::EndOfBlock,
        TAG_BLOCK => StepKind::Block,
        TAG_PIXEL => StepKind::Pixel,
        TAG_FILTERED => StepKind::Filtered,
        TAG_DC => StepKind::Dc {
            code: raw.b as u8,
            size: (raw.b >> 8) as u8,
            diff: raw.a as u16 as i16,
            dc: (raw.a >> 16) as u16 as i16,
        },
        TAG_AC => StepKind::Ac {
            code: raw.b as u8,
            run: (raw.b >> 8) as u8,
            size: (raw.b >> 16) as u8,
            k: (raw.a >> 16) as u8,
            value: raw.a as u16 as i16,
        },
        TAG_ZRL => StepKind::Zrl { code: raw.b as u8, k: raw.a as u8 },
        TAG_EOB => StepKind::Eob { code: raw.b as u8, k: raw.a as u8 },
        _ => StepKind::Opaque,
    }
}

/// The header fields in the order [`StepField`] declares them, so a packed
/// step can be read back. Kept beside the enum on purpose: adding a field
/// without adding it here is caught by the test below.
const FIELDS: [StepField; 30] = [
    StepField::Bfinal,
    StepField::Btype,
    StepField::Hlit,
    StepField::Hdist,
    StepField::Hclen,
    StepField::StoredLen,
    StepField::StoredNlen,
    StepField::Padding,
    StepField::Wrapper,
    StepField::Token,
    StepField::LengthExtra,
    StepField::Offset,
    StepField::FrameHeader,
    StepField::BlockHeader,
    StepField::Filter,
    StepField::Footer,
    StepField::FilterDef,
    StepField::PxuFlags,
    StepField::PxuWidth,
    StepField::PxuHeight,
    StepField::PxuBits,
    StepField::LzmaProps,
    StepField::RangeInit,
    StepField::UnpackedSize,
    StepField::FrequencyTable,
    StepField::BlockChecksum,
    StepField::ContentChecksum,
    StepField::Pass,
    StepField::Row,
    StepField::Restart,
];

/// Open a compressed run and say what the decoder did to it.
///
/// The bytes are the same bytes [`decode`] gives; the trace is what tells the
/// reader which bits of the run made which bytes of what came out. How fine
/// the trace is depends on the codec: deflate, LZ4 and LZMA are read by our
/// own decoders and traced per symbol, and so is xz wherever the filter chain
/// in its blocks is one this can run; zstd keeps its crate and is traced per
/// block; and bzip2 keeps a crate that will not say where a block ended, so
/// it is one step over the whole stream.
pub fn decode_traced(codec: Codec, data: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    if data.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    let (out, trace) = match codec {
        Codec::CfbWorkbook => {
            let out = cfb::workbook(data)?;
            let trace = frames::whole(data.len(), out.len());
            (out, trace)
        }
        Codec::Stored => (data.to_vec(), frames::whole(data.len(), data.len())),
        Codec::Deflate => inflate::inflate(data)?,
        Codec::Zlib => inflate::zlib(data)?,
        Codec::Lz4Block => lz4::block(data)?,
        Codec::Lz4Frame => lz4::frame(data)?,
        Codec::Snappy => snappy::block(data)?,
        Codec::Brotli => frames::brotli(data)?,
        Codec::Zstd => frames::zstd(data)?,
        Codec::Xz => xz::stream(data)?,
        Codec::Lzip => lzma::lzip(data)?,
        Codec::Lzma1 { props, dict_size, unpacked } => lzma::lzma1(data, props, dict_size, unpacked)?,
        Codec::Lzma2 => lzma::lzma2(data)?,
        Codec::Lha { window_bits } => lha::entry(data, window_bits)?,
        Codec::Rar5 { window_bits, unpacked } => rar5::entry(data, window_bits, unpacked)?,
        Codec::Bzip2 => bzip2::stream(data)?,
        Codec::Compress => compress::lzw(data)?,
        Codec::Gzip => inflate::gzip(data)?,
        Codec::FastLz => fastlz::block(data)?,
        Codec::PngUnfilter { stride, bpp } => pixels::unfilter(data, stride, bpp)?,
        Codec::LowBitsArgb => pixels::low_bits_argb(data)?,
        Codec::LowBitsRgba11 => pixels::low_bits_rgba11(data)?,
        Codec::Pico8Pxa => pico8::pxa(data)?,
        Codec::Pico8Old => pico8::old(data)?,
        Codec::PicotronPxu => pxu::pxu(data)?,
        Codec::CdfRle => cdfrle::stream(data)?,
        Codec::CdfHuffman => cdfhuff::huffman(data)?,
        Codec::CdfAhuff => cdfhuff::adaptive(data)?,
        Codec::Latin1Text => pytext::latin1_text(data)?,
        Codec::EscapedLatin1Text => pytext::escaped_latin1_text(data)?,
        Codec::PngScanlines { width, height, bits_per_pixel, interlace } => {
            let shape = scanlines::Geometry::new(width as u64, height as u64, bits_per_pixel as u64, interlace);
            scanlines::unfilter_image(data, &shape.ok_or(Refusal::Settings)?)?
        }
        // A scan's tables are in the segments before it, and this was handed
        // the scan alone. See `decode_traced_with`.
        Codec::JpegBaseline => return Err(Refusal::Settings),
    };
    if out.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    Ok((out, trace))
}

/// Open a compressed run whose settings are bytes written somewhere else in
/// the file, and say what the decoder did to it.
///
/// `settings` is those bytes, in the layout the codec's own standard writes
/// them in. One codec takes any: [`Codec::JpegBaseline`], which is handed
/// every segment of the image from the one after the start-of-image marker up
/// to the scan's own header, and reads its frame, its tables and its restart
/// interval out of them. Every other codec takes the run alone, and is opened
/// the way [`decode_traced`] opens it whatever `settings` holds.
pub fn decode_traced_with(codec: Codec, data: &[u8], settings: &[u8]) -> Result<(Vec<u8>, Trace), Refusal> {
    match codec {
        Codec::JpegBaseline => {
            if data.len() > CAP_BYTES || settings.len() > CAP_BYTES {
                return Err(Refusal::TooLarge);
            }
            let header = jpeg::Header::read(settings)?;
            let (out, trace) = jpeg::scan(data, &header)?;
            if out.len() > CAP_BYTES {
                return Err(Refusal::TooLarge);
            }
            Ok((out, trace))
        }
        other => decode_traced(other, data),
    }
}

/// How many bytes of `data` a stream packed with `codec` takes, for a codec
/// whose streams end themselves. Deflate does: its last block says it is the
/// last. Nothing else here does yet, and asking about one of those is
/// [`Refusal::Settings`], the same answer a run packed a way this cannot work
/// out gets: nothing says how long it is.
///
/// What a template sizes a run by when the format wrote no length and the run
/// is not the last thing in its container: a gzip of several members is one
/// deflate stream and trailer after another, and the second member starts
/// wherever the first stream stopped. See [`inflate::inflate_len`].
pub fn stream_len(codec: Codec, data: &[u8]) -> Result<u64, Refusal> {
    if data.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    match codec {
        Codec::Deflate => inflate::inflate_len(data),
        _ => Err(Refusal::Settings),
    }
}

/// Open a compressed run. `data` is the whole of it.
pub fn decode(codec: Codec, data: &[u8]) -> Result<Vec<u8>, Refusal> {
    if data.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    let out = match codec {
        // One decoder, not two: the bytes a reader sees have to be the bytes
        // the trace describes, so the traced path is the only path.
        Codec::CfbWorkbook
        | Codec::Stored
        | Codec::Zlib
        | Codec::Deflate
        | Codec::Lz4Block
        | Codec::Lz4Frame
        | Codec::Snappy
        | Codec::Brotli
        | Codec::PngUnfilter { .. }
        | Codec::LowBitsArgb
        | Codec::LowBitsRgba11
        | Codec::Pico8Pxa
        | Codec::Pico8Old
        | Codec::PicotronPxu
        | Codec::Lzip
        | Codec::Lzma1 { .. }
        | Codec::Lzma2
        | Codec::Lha { .. }
        | Codec::Rar5 { .. }
        | Codec::Bzip2
        | Codec::Compress
        | Codec::Gzip
        | Codec::CdfRle
        | Codec::CdfHuffman
        | Codec::CdfAhuff
        | Codec::Latin1Text
        | Codec::EscapedLatin1Text
        | Codec::PngScanlines { .. }
        | Codec::FastLz => {
            decode_traced(codec, data)?.0
        }
        Codec::Zstd => zstd(data)?,
        // Not the traced path, but the same decisions and the same decoder:
        // see `xz::bytes`, which is the traced path with the recording left
        // out rather than a second reading of the format.
        Codec::Xz => xz::bytes(data)?,
        Codec::JpegBaseline => return Err(Refusal::Settings),
    };
    if out.len() > CAP_BYTES {
        return Err(Refusal::TooLarge);
    }
    Ok(out)
}

/// Zstandard, read a frame at a time. A file compressed by `zstd` is one
/// frame; ROOT writes one per block. Concatenated frames are read through to
/// the end, which is what the format says a decoder does.
fn zstd(data: &[u8]) -> Result<Vec<u8>, Refusal> {
    use std::io::Read;
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(data).map_err(|_| Refusal::Failed)?;
    // One byte past the cap, so a stream that only just fits is told from one
    // that does not.
    let mut out = Vec::new();
    match decoder.by_ref().take(CAP_BYTES as u64 + 1).read_to_end(&mut out) {
        Ok(_) if out.len() > CAP_BYTES => Err(Refusal::TooLarge),
        Ok(_) => Ok(out),
        Err(_) => Err(Refusal::Failed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zlib_stream_comes_back_as_what_went_in() {
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(b"hello hello hello", 6);
        assert_eq!(decode(Codec::Zlib, &packed).unwrap(), b"hello hello hello");
    }

    #[test]
    fn raw_deflate_has_no_header_on_it() {
        let packed = miniz_oxide::deflate::compress_to_vec(b"deflate me", 6);
        assert_eq!(decode(Codec::Deflate, &packed).unwrap(), b"deflate me");
        // The same bytes read as zlib are not a zlib stream.
        assert_eq!(decode(Codec::Zlib, &packed), Err(Refusal::Failed));
    }

    #[test]
    fn an_lz4_block_is_sized_by_trying_since_nothing_in_it_says() {
        let long = "lz4 block ".repeat(5000);
        let packed = lz4_flex::block::compress(long.as_bytes());
        assert_eq!(decode(Codec::Lz4Block, &packed).unwrap(), long.as_bytes());
    }

    #[test]
    fn bytes_that_are_not_a_stream_are_refused_rather_than_guessed_at() {
        assert_eq!(decode(Codec::Zlib, b"not compressed"), Err(Refusal::Failed));
        assert_eq!(decode(Codec::Zstd, b"not compressed"), Err(Refusal::Failed));
        assert_eq!(decode(Codec::Xz, b"not compressed"), Err(Refusal::Failed));
        assert_eq!(decode(Codec::Lzip, b"not compressed at all, not even a bit"), Err(Refusal::Failed));
    }

}
