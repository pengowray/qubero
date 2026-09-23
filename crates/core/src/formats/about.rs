//! What each format is, in a sentence or three for a reader who has never met
//! it, and the Wikipedia article that says more.
//!
//! The report view opens with this, before anything about the file in front of
//! the reader. It is written here rather than taken from Wikidata or Wikipedia:
//! Wikidata's one-line descriptions say too little ("lossy compression method
//! for digital images") and an article's lead says too much in the wrong order.
//! Each entry started from the article's lead and says what the format stores
//! and how, in plain words, so a reader knows what to expect before the bytes.
//!
//! Keyed by template name. A template with no entry here gets its `file(1)`
//! description in the report, or nothing.

/// One format, for a reader who has never met it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct About {
    /// The format's plain name, as the text below uses it.
    pub name: &'static str,
    /// The title of the English Wikipedia article, with any `#section`, or
    /// nothing where no article covers the format.
    pub wikipedia: Option<&'static str>,
    /// What the format stores and how. Plain text, no markup.
    pub text: &'static str,
}

/// The entry for a template name, if there is one.
pub fn about(template: &str) -> Option<&'static About> {
    ABOUT.iter().find(|(name, _)| *name == template).map(|(_, a)| a)
}

const fn a(name: &'static str, wikipedia: Option<&'static str>, text: &'static str) -> About {
    About { name, wikipedia, text }
}

static ABOUT: &[(&str, About)] = &[
    ("jpeg", a("JPEG", Some("JPEG"),
        "JPEG stores photographs and other images with smooth changes of color, using lossy compression: some detail is discarded to make the file smaller. It splits the picture into 8 × 8 blocks of pixels, transforms each block with the discrete cosine transform, rounds away fine detail, and packs what remains with Huffman coding. Most files keep color at a lower resolution than brightness.")),
    ("png", a("PNG", Some("PNG"),
        "PNG stores images without loss. It compresses the pixels row by row with zlib, after replacing each row with its differences from the pixels to its left or above, which compresses better. The file is a series of chunks, each with a length, a four-letter type, and a CRC.")),
    ("gif", a("GIF", Some("GIF"),
        "GIF stores images of up to 256 colors, and animations made of several such images. It compresses the pixels without loss using LZW. Each image uses the color table in the file's header or one of its own.")),
    ("bmp", a("BMP", Some("BMP file format"),
        "BMP is the Windows bitmap format: a file header, an information header, an optional color table, and the pixels as rows, usually uncompressed and stored from the bottom row up.")),
    ("tiff", a("TIFF", Some("TIFF"),
        "TIFF stores images described by tags, held in directories called IFDs. Each tag holds a small value or points to a larger one elsewhere in the file, and one file can have several IFDs, for pages or thumbnails. Camera raw formats such as DNG, CR2, and NEF are built on TIFF.")),
    ("zip", a("ZIP", Some("ZIP (file format)"),
        "ZIP stores files, each compressed on its own, usually with deflate. Each file's data follows a local header, and a central directory at the end of the archive lists every file with the position of its local header. Word documents, Java archives, and many other formats are ZIP files with a set layout inside.")),
    ("gzip", a("gzip", Some("Gzip"),
        "gzip stores one file compressed with deflate. A header can give the file's original name and time, and a CRC-32 and the uncompressed length end the file.")),
    ("zlib", a("zlib", Some("Zlib"),
        "zlib wraps a deflate stream in a 2-byte header and an Adler-32 checksum. Deflate replaces repeated strings with references back to an earlier copy and packs the result with Huffman codes.")),
    ("bzip2", a("bzip2", Some("Bzip2"),
        "bzip2 compresses data in blocks of up to 900 KB, using the Burrows–Wheeler transform, move-to-front coding, and Huffman coding.")),
    ("xz", a("xz", Some("XZ Utils"),
        "xz stores one compressed file, usually compressed with LZMA2, in blocks with checksums, followed by an index of the blocks.")),
    ("tar", a("tar", Some("Tar (computing)"),
        "tar stores files one after another, each after a 512-byte header that gives its name, size, owner, permissions, and times. Data is padded to a multiple of 512 bytes, and nothing is compressed. A compressed tar file is a tar file inside gzip, bzip2, or xz.")),
    ("7z", a("7z", Some("7z"),
        "7z stores compressed files, usually with LZMA or LZMA2. The header that lists the files and says how they are compressed comes at the end of the archive, and is often compressed too.")),
    ("rar5", a("RAR 5", Some("RAR (file format)"),
        "RAR 5 stores compressed files as a series of blocks, each with a CRC-32, a size, and a type, using RAR's own compression method.")),
    ("elf", a("ELF", Some("Executable and Linkable Format"),
        "ELF is the format of executables, libraries, and object files on Linux and most other Unix-like systems. It describes its contents twice: as named sections, for linkers and debuggers, and as segments, which the loader maps into memory.")),
    ("pe", a("PE", Some("Portable Executable"),
        "PE is the format of Windows programs (.exe) and libraries (.dll). An MS-DOS header and stub program come first, then a PE header, a table of sections, and the sections themselves, which the loader maps into memory at their virtual addresses.")),
    ("macho", a("Mach-O", Some("Mach-O"),
        "Mach-O is the format of executables and libraries on macOS and iOS. A header and a list of load commands say which segments to map into memory, which libraries to load, and where the code starts.")),
    ("wasm", a("WebAssembly", Some("WebAssembly"),
        "WebAssembly stores compiled code for a stack-based virtual machine that runs in web browsers and elsewhere. A module is a series of sections, such as types, imports, functions, and code, and most of its numbers are LEB128 variable-length integers.")),
    ("sqlite", a("SQLite", Some("SQLite"),
        "An SQLite database is a single file divided into pages of one size. Each table and index is a B-tree of pages, and a row too big for its page continues on overflow pages. Pages no longer in use are kept on a freelist for reuse.")),
    ("wav", a("WAV", Some("WAV"),
        "WAV stores audio, usually as uncompressed PCM samples. It is a RIFF file: a series of chunks, each with a four-character ID and a length. The fmt chunk gives the sample format, and the data chunk holds the samples.")),
    ("midi", a("Standard MIDI File", Some("MIDI#Standard MIDI files"),
        "A Standard MIDI File stores music as timed instructions for a synthesizer, such as which note to start and stop, not as recorded sound. It holds a header chunk and one chunk per track. Each track is a list of events, each written after the time since the previous one.")),
    ("id3", a("ID3", Some("ID3"),
        "ID3 stores information about an MP3 file, such as the title, the artist, and cover art, as frames in a tag before the audio.")),
    ("mp4", a("MP4", Some("MP4 file format"),
        "MP4 stores video, audio, and subtitles as tracks in one file. It is a tree of boxes, each with a length and a four-character type. The moov box describes the tracks, and the mdat box holds the compressed samples they point to.")),
    ("mkv", a("Matroska", Some("Matroska"),
        "Matroska stores video, audio, and subtitles as a tree of EBML elements, each with a variable-length ID and size. WebM is a restricted form of Matroska.")),
    ("ogg", a("Ogg", Some("Ogg"),
        "Ogg carries streams of audio or video, usually Vorbis, Opus, or Theora, cut into pages. Each page has a header with a sequence number, a position in time, and a CRC, and a packet of a stream can continue from one page to the next.")),
    ("pdf", a("PDF", Some("PDF"),
        "PDF stores documents as numbered objects, such as pages, fonts, and images, written as text with binary streams inside. A cross-reference table near the end gives each object's position, so a reader can start from the end and fetch only the objects it needs.")),
    ("iso9660", a("ISO 9660", Some("ISO 9660"),
        "ISO 9660 is the file system of CD-ROMs and of the disc images copied from them. The disc is a series of 2,048-byte sectors: 16 unused sectors, then volume descriptors, then directories and files that refer to each other by sector number.")),
    ("hdf5", a("HDF5", Some("Hierarchical Data Format"),
        "HDF5 stores large scientific datasets in one file, as a tree of groups and datasets, like folders and files. Datasets can be split into chunks and compressed, and B-trees index the chunks and the groups.")),
    ("fits", a("FITS", Some("FITS"),
        "FITS stores astronomical images and tables. Each part starts with a header of 80-character text lines, each giving a keyword and its value, and continues with binary data. Headers and data both come in blocks of 2,880 bytes.")),
    ("npy", a("NPY", Some("NumPy"),
        "NPY stores one NumPy array: a short header, written as a Python dictionary, that gives the element type, the shape, and the order, followed by the array's raw data.")),
    ("parquet", a("Parquet", Some("Apache Parquet"),
        "Parquet stores tables by column rather than by row, so each column compresses well and a reader can load only the columns it needs. The metadata, in a footer at the end of the file, gives the position of each column's data.")),
    ("json", a("JSON", Some("JSON"),
        "JSON stores data as text: objects of named values, arrays, strings, numbers, true, false, and null.")),
    ("pickle", a("pickle", Some("Serialization#Pickle"),
        "Pickle is Python's own format for saving objects. A pickle is a program for a small stack machine, and running its opcodes rebuilds the objects. Loading a pickle can run any code the file names.")),
    ("gguf", a("GGUF", Some("Llama.cpp#GGUF file format"),
        "GGUF stores machine learning models for llama.cpp and programs like it: key-value metadata, a table describing each tensor, and the tensors' weights, often quantized to 8 bits or fewer per weight.")),
    ("safetensors", a("safetensors", None,
        "safetensors stores the tensors of a machine learning model: an 8-byte length, a JSON header giving each tensor's type, shape, and byte range, and then the raw data.")),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_names_a_template() {
        let names = crate::formats::builtin_names();
        for (key, _) in ABOUT {
            assert!(names.contains(key), "about.rs has an entry for {key}, which is not a template name");
        }
    }

    #[test]
    fn no_entry_twice() {
        for (i, (key, _)) in ABOUT.iter().enumerate() {
            assert!(ABOUT[i + 1..].iter().all(|(other, _)| other != key), "{key} has two entries");
        }
    }

    #[test]
    fn text_has_no_em_dashes() {
        for (key, a) in ABOUT {
            assert!(!a.text.contains('\u{2014}'), "{key}: the text has an em-dash");
        }
    }
}
