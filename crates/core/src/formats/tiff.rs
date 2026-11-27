//! TIFF: the format that asks which way round it is before it says anything
//! else.
//!
//! The first two bytes are `II` or `MM`, for Intel or Motorola, and every
//! number in the file after them is written that way. Nothing else in the tree
//! does this: a PNG is big-endian because PNG is big-endian, and a template
//! can say so once and be done. Here the answer is in the file.
//!
//! Saying it needs nothing new. A template is built by running code, so the
//! layout is written once as a function of the order and the switch on those
//! two bytes picks which of the two to use. The two copies share everything
//! they can, and neither of them has to carry the question around.
//!
//! What follows the header is a directory: a count, that many twelve-byte
//! entries, and the offset of the next directory. The entries are what makes
//! this format what it is. Each is a tag, a type, a count, and four bytes that
//! are the value itself when it fits in four and where to find it when it
//! does not. Nothing in the entry says which of the two those bytes are: it is
//! the size of the type times the count, worked out rather than read. So that
//! is what the template does, in a field of no bits called `room`, and the
//! switch below it is on whether the answer is over four.
//!
//! A value that does not fit is read where the entry says it is, which is an
//! offset from the start of the TIFF. That is the same as the start of the
//! file right up until it is not: an EXIF block is a whole TIFF written into a
//! JPEG segment, and every offset in one counts from its own `II` or `MM`
//! partway through. The whole format sits in a window of its own and its
//! offsets are anchored to that window, so the one layout is both a file and a
//! copy of a file inside something else. `tiff_part` is what a JPEG borrows:
//! the type, and the directory types it refers to by name, which have to
//! travel with it because a directory holds directories.
//!
//! Three tags do not hold a value at all. `exif ifd` is where the camera wrote
//! everything it knew, `gps ifd` is where the photograph was taken, and
//! `sub ifd` is another image entirely. All three hold the place a directory
//! begins, and all three are followed. What the first two point at is read
//! against another set of tag names: the numbers in a camera's directory mean
//! nothing like the numbers in the file's own, and reading a latitude as a
//! subfile type would be worse than not reading it at all.
//!
//! Each directory says where the next one is, as an offset into the file with
//! nothing bounding it, so a file whose directory points back at itself is a
//! ring. The directories after the first are read as one flat list,
//! `next_ifds`, which stops at an offset it has already read. The tags above
//! can ring in the same way, and the cycle guard is what makes following them
//! safe: a pointer landing on a directory of the same kind already open is
//! refused. `sub ifd` may hold a list of offsets rather than one, which is how
//! a raw file keeps several images, and every directory in the list is read.
//!
//! What a directory describes is placed in it. StripOffsets and
//! StripByteCounts give each strip of the image, TileOffsets and
//! TileByteCounts each tile, and JPEGInterchangeFormat and its length a JPEG
//! image, which in IFD 1 of an EXIF block is the thumbnail. So a byte of image
//! data reads as `strip 3` or `thumbnail` rather than as `body`.

use crate::template::{Anchor, Encoding, Endian, Endian::*, Expr as E, Part, Step, StrLen, Template, Ty as T};

/// The tags worth naming. There are hundreds more, and a number with no name
/// is still shown; these are the ones a file in the wild actually carries.
const TAG: &[(i128, &str)] = &[
    (254, "new subfile type"),
    (255, "subfile type"),
    (256, "image width"),
    (257, "image height"),
    (258, "bits per sample"),
    (259, "compression"),
    (262, "photometric interpretation"),
    (263, "thresholding"),
    (266, "fill order"),
    (269, "document name"),
    (270, "image description"),
    (271, "make"),
    (272, "model"),
    (273, "strip offsets"),
    (274, "orientation"),
    (277, "samples per pixel"),
    (278, "rows per strip"),
    (279, "strip byte counts"),
    (280, "min sample value"),
    (281, "max sample value"),
    (282, "x resolution"),
    (283, "y resolution"),
    (284, "planar configuration"),
    (288, "free offsets"),
    (289, "free byte counts"),
    (290, "grey response unit"),
    (291, "grey response curve"),
    (296, "resolution unit"),
    (297, "page number"),
    (305, "software"),
    (306, "date time"),
    (315, "artist"),
    (316, "host computer"),
    (317, "predictor"),
    (318, "white point"),
    (319, "primary chromaticities"),
    (320, "colour map"),
    (321, "halftone hints"),
    (322, "tile width"),
    (323, "tile length"),
    (324, "tile offsets"),
    (325, "tile byte counts"),
    (330, "sub ifds"),
    (338, "extra samples"),
    (339, "sample format"),
    // Exif's JPEG-compressed thumbnail or another JPEG image carried inside
    // a TIFF-family file. Sony ARW uses these for its embedded previews.
    (513, "jpeg interchange format"),
    (514, "jpeg interchange format length"),
    (529, "ycbcr coefficients"),
    (530, "ycbcr subsampling"),
    (531, "ycbcr positioning"),
    (532, "reference black white"),
    (700, "xmp"),
    (33421, "cfa repeat pattern dimensions"),
    (33422, "cfa pattern"),
    (33432, "copyright"),
    (33723, "iptc"),
    (34665, "exif ifd"),
    (34675, "icc profile"),
    (34713, "nikon nef info"),
    (34853, "gps ifd"),
    (50341, "print image matching"),
    // TIFF/EP and DNG camera-raw metadata. DNG deliberately extends TIFF
    // rather than wrapping it, and proprietary raw formats use many of the
    // same tags alongside their maker-specific records.
    (37398, "tiff/ep standard id"),
    (50706, "dng version"),
    (50707, "dng backward version"),
    (50708, "unique camera model"),
    (50709, "localised camera model"),
    (50710, "cfa plane colour"),
    (50711, "cfa layout"),
    (50712, "linearisation table"),
    (50713, "black level repeat dimensions"),
    (50714, "black level"),
    (50715, "black level delta h"),
    (50716, "black level delta v"),
    (50717, "white level"),
    (50718, "default scale"),
    (50719, "default crop origin"),
    (50720, "default crop size"),
    (50721, "colour matrix 1"),
    (50722, "colour matrix 2"),
    (50723, "camera calibration 1"),
    (50724, "camera calibration 2"),
    (50725, "reduction matrix 1"),
    (50726, "reduction matrix 2"),
    (50727, "analogue balance"),
    (50728, "as shot neutral"),
    (50729, "as shot white xy"),
    (50730, "baseline exposure"),
    (50731, "baseline noise"),
    (50732, "baseline sharpness"),
    (50733, "bayer green split"),
    (50734, "linear response limit"),
    (50735, "camera serial number"),
    (50736, "lens info"),
    (50737, "chroma blur radius"),
    (50738, "anti alias strength"),
    (50739, "shadow scale"),
    (50740, "dng private data"),
    (50741, "maker note safety"),
    (50778, "calibration illuminant 1"),
    (50779, "calibration illuminant 2"),
    (50780, "best quality scale"),
    (50781, "raw data unique id"),
    (50827, "original raw file name"),
    (50828, "original raw file data"),
    (50829, "active area"),
    (50830, "masked areas"),
    (50831, "as shot icc profile"),
    (50832, "as shot pre-profile matrix"),
    (50833, "current icc profile"),
    (50834, "current pre-profile matrix"),
];

/// The tags of the directory `exif ifd` points at: what the camera knew when
/// it took the photograph. A different set of numbers from the one above, and
/// the same numbers would mean something else there.
const EXIF_TAG: &[(i128, &str)] = &[
    (33434, "exposure time"),
    (33437, "f number"),
    (34850, "exposure program"),
    (34852, "spectral sensitivity"),
    (34855, "iso speed"),
    (34864, "sensitivity type"),
    (34866, "recommended exposure index"),
    (36864, "exif version"),
    (36867, "date taken"),
    (36868, "date digitised"),
    (36880, "offset from utc"),
    (36881, "offset from utc, taken"),
    (36882, "offset from utc, digitised"),
    (37121, "components configuration"),
    (37122, "compressed bits per pixel"),
    (37377, "shutter speed"),
    (37378, "aperture"),
    (37379, "brightness"),
    (37380, "exposure bias"),
    (37381, "widest aperture"),
    (37382, "subject distance"),
    (37383, "metering mode"),
    (37384, "light source"),
    (37385, "flash"),
    (37386, "focal length"),
    (37396, "subject area"),
    // Whatever the manufacturer wanted to keep, in a format of their own that
    // nobody else agreed to. Read as the bytes it is.
    (37500, "maker note"),
    (37510, "comment"),
    (37520, "subsecond"),
    (37521, "subsecond, taken"),
    (37522, "subsecond, digitised"),
    (40960, "flashpix version"),
    (40961, "colour space"),
    (40962, "width"),
    (40963, "height"),
    (40964, "related sound file"),
    (40965, "interoperability ifd"),
    (41486, "focal plane x resolution"),
    (41487, "focal plane y resolution"),
    (41488, "focal plane resolution unit"),
    (41492, "subject position"),
    (41493, "exposure index"),
    (41495, "sensing method"),
    (41728, "file source"),
    (41729, "scene type"),
    (41730, "colour filter pattern"),
    (41985, "custom rendered"),
    (41986, "exposure mode"),
    (41987, "white balance"),
    (41988, "digital zoom"),
    (41989, "focal length in 35mm"),
    (41990, "scene capture type"),
    (41991, "gain control"),
    (41992, "contrast"),
    (41993, "saturation"),
    (41994, "sharpness"),
    (41996, "subject distance range"),
    (42016, "image id"),
    (42032, "camera owner"),
    (42033, "body serial number"),
    (42034, "lens specification"),
    (42035, "lens make"),
    (42036, "lens model"),
    (42037, "lens serial number"),
];

/// The tags of the directory `gps ifd` points at: where the photograph was
/// taken, and when, by the satellites rather than by the camera's clock. The
/// format calls every one of them "gps something"; inside a GPS directory
/// that is not worth repeating on every row.
const GPS_TAG: &[(i128, &str)] = &[
    (0, "version id"),
    (1, "latitude ref"),
    (2, "latitude"),
    (3, "longitude ref"),
    (4, "longitude"),
    (5, "altitude ref"),
    (6, "altitude"),
    (7, "time"),
    (8, "satellites"),
    (9, "status"),
    (10, "measure mode"),
    (11, "precision"),
    (12, "speed ref"),
    (13, "speed"),
    (14, "track ref"),
    (15, "track"),
    (16, "direction ref"),
    (17, "direction"),
    (18, "map datum"),
    (19, "destination latitude ref"),
    (20, "destination latitude"),
    (21, "destination longitude ref"),
    (22, "destination longitude"),
    (23, "destination bearing ref"),
    (24, "destination bearing"),
    (25, "destination distance ref"),
    (26, "destination distance"),
    (27, "processing method"),
    (28, "area information"),
    (29, "date"),
    (30, "differential"),
    (31, "horizontal error"),
];

/// Tags found in Sony ARW IFDs. The public names come from ExifTool's EXIF
/// table; the remaining values are observed in ARW files but have no published
/// meaning, so their labels say only that they are vendor-private. Keep these
/// out of the generic TIFF, Nikon and DNG tag tables.
const SONY_TAG: &[(i128, &str)] = &[
    (513, "preview image start"),
    (514, "preview image length"),
    (0x7000, "sony raw file type"),
    (0x7001, "sony private 0x7001 (undocumented)"),
    (0x7010, "sony tone curve"),
    (0x7011, "sony private 0x7011 (undocumented)"),
    (0x7020, "sony private 0x7020 (undocumented)"),
    (0x7031, "vignetting correction"),
    (0x7032, "vignetting correction parameters"),
    (0x7034, "chromatic aberration correction"),
    (0x7035, "chromatic aberration correction parameters"),
    (0x7036, "distortion correction"),
    (0x7037, "distortion correction parameters"),
    (0x7038, "sony raw image size"),
];

/// What one value of an entry is. The count says how many of them there are,
/// so the room an entry describes is this times that.
const FIELD_TYPE: &[(i128, &str)] = &[
    (1, "byte"),
    (2, "ascii"),
    (3, "short"),
    (4, "long"),
    (5, "rational"),
    (6, "signed byte"),
    (7, "undefined"),
    (8, "signed short"),
    (9, "signed long"),
    (10, "signed rational"),
    (11, "float"),
    (12, "double"),
    (13, "ifd"),
    (16, "long8"),
    (17, "signed long8"),
    (18, "ifd8"),
];

pub fn tiff() -> Template {
    tiff_named("tiff")
}

/// Camera RAW formats in the TIFF family share the complete TIFF layout. A
/// distinct template name preserves the more precise result from sniffing and
/// makes each format available explicitly in the template picker.
pub fn camera_raw(name: &str) -> Template {
    if name == "arw" {
        let part = sony_arw_part();
        Template::new(name, part.root.clone()).with_part(&part)
    } else {
        tiff_named(name)
    }
}

fn tiff_named(name: &str) -> Template {
    let part = tiff_part();
    Template::new(name, part.root.clone()).with_part(&part)
}

/// The whole of a TIFF and the names it refers to, for a format that carries
/// one inside it. A directory holds entries that point at directories, so the
/// vocabulary has to travel with the type.
pub fn tiff_part() -> Part {
    let mut part = Part::new(tiff_file());
    for e in [Little, Big] {
        for space in SPACES {
            part = part.with_type(space.named(e), ifd(e, *space));
        }
    }
    with_jpeg(part)
}

fn sony_arw_part() -> Part {
    let mut part = Part::new(tiff_file_for(Space::Sony));
    for e in [Little, Big] {
        for space in [Space::Sony, Space::Exif, Space::Gps] {
            part = part.with_type(space.named(e), ifd(e, space));
        }
    }
    with_jpeg(part)
}

/// What the JPEG image a directory points at is read as: the JPEG template's
/// root, by name. See [`with_jpeg`].
const JPEG: &str = "tiff.Jpeg";

thread_local! {
    /// Set while [`with_jpeg`] is building the JPEG template.
    static BUILDING_JPEG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `part`, with the JPEG template's root added as [`JPEG`], so that the
/// thumbnail and preview images directories point at are read as JPEGs.
///
/// The JPEG template borrows this part for its EXIF block, so building it
/// here calls `tiff_part` again. That inner call finds `BUILDING_JPEG` set and
/// leaves the JPEG out, and this outer call adds it, so the name is there in
/// the finished template whichever of the two was asked for first. Reading
/// ends too: a thumbnail lies inside its TIFF, which lies inside the JPEG
/// segment holding it, so each JPEG read this way is shorter than the one
/// around it.
///
/// A guard here rather than a named type in `jpeg.rs`, because `jpeg()` builds
/// its root in place and registers no name another part could borrow.
fn with_jpeg(part: Part) -> Part {
    struct Done;
    impl Drop for Done {
        fn drop(&mut self) {
            BUILDING_JPEG.with(|b| b.set(false));
        }
    }
    if BUILDING_JPEG.with(|b| b.replace(true)) {
        return part;
    }
    let _done = Done;
    let jpeg = super::jpeg::jpeg();
    let mut part = part.with_type(JPEG, jpeg.root);
    // The JPEG's EXIF block is a plain TIFF, so it names `tiff.Ifd`, which a
    // Sony ARW's own part does not have. Take every name the JPEG brought
    // that this part lacks, in a fixed order.
    let mut brought: Vec<(String, T)> = jpeg.types.into_iter().filter(|(name, _)| !part.types.iter().any(|(had, _)| had == name)).collect();
    brought.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, ty) in brought {
        part = part.with_type(&name, ty);
    }
    part
}

/// The same for a JPEG XR, which is this directory read against another set of
/// names. Only the little-endian copies, because the format writes `II` and
/// never anything else; the camera's directory and the satellite's are the
/// same ones a TIFF points at, and travel with it.
pub fn jxr_part() -> Part {
    let mut part = Part::new(super::jxr::jxr_file());
    for space in [Space::Jxr, Space::Exif, Space::Gps] {
        part = part.with_type(space.named(Little), ifd(Little, space));
    }
    part
}

/// The whole of a TIFF, in a window of its own so that every offset inside it
/// counts from where it begins. On its own that is the start of the file and
/// the window changes nothing; written into an EXIF block partway through a
/// JPEG it is the only thing that makes the offsets mean anything.
pub fn tiff_file() -> T {
    tiff_file_for(Space::Tiff)
}

fn tiff_file_for(space: Space) -> T {
    T::sized(
        E::Remaining,
        T::structure(
            "TIFF",
            vec![
                // Both letters of each are the same byte, so which way round
                // this is read cannot change what it says, which is the only
                // reason it can be read at all before the answer is known.
                (
                    "byte_order",
                    T::enumeration("ByteOrder", T::u16(Big), &[(0x4949, "intel, little-endian"), (0x4d4d, "motorola, big-endian")]),
                ),
                (
                    "file",
                    T::switch(
                        E::field("byte_order"),
                        vec![(0x4949, file(Little, space)), (0x4d4d, file(Big, space))],
                        T::bytes(E::Remaining),
                    ),
                ),
            ],
        )
        // `file` is the rest of the header, written in the order the two
        // letters chose. The word adds a level to the listing and says
        // nothing, so its fields are listed beside `byte_order`.
        .contents("file"),
    )
}

/// Everything after the two letters, written the way they said.
fn file(e: Endian, space: Space) -> T {
    T::structure(
        "Header",
        vec![
            // 42, which the specification says was chosen for its deep
            // philosophical significance. BigTIFF says 43 and lays out its
            // directories differently; this reads the header and stops.
            ("version", T::enumeration("Version", T::u16(e), &[(42, "tiff"), (43, "bigtiff")])),
            ("ifd_offset", T::u32(e)),
            // The directory, wherever the header says it is. It costs no bytes
            // here, so the image below still starts where it starts.
            ("ifd", T::at_in_window(E::field("ifd_offset"), T::Named(space.named(e).into()))),
            // The strips, the tiles, any directory after the first, and the
            // values too big to sit inside an entry. Every one of them is
            // pointed at by an entry above rather than laid out in order.
            ("body", T::bytes(E::Remaining)),
            ("next_ifds", next_ifds(e, space)),
        ],
    )
    // `body` only says that the rest of the file comes after the header. The
    // directories and values placed over it say what those bytes are, so
    // where one covers a byte, the hex view names it rather than `body`.
    .field_aside("body")
    .field_elem_named_from("next_ifds", E::words("ifd {}", E::idx().add(E::lit(1))))
    .field_doc("next_ifds", "The IFDs after IFD 0, in the order they are linked. Each IFD starts at the offset in the next_ifd_offset field of the IFD before it, counted from the start of the TIFF. In an EXIF block, IFD 1 has the thumbnail. In a multi-page TIFF, each page has its own IFD.")
}

/// Every directory after the first, as one flat list: IFD 0 says where IFD 1
/// is, IFD 1 says where IFD 2 is, and so on until one says 0.
///
/// A chain rather than a directory nested in the one before it, so that a
/// TIFF of a thousand pages is a list of a thousand rather than a tree a
/// thousand deep. The chain stops at an offset it has already visited. It does
/// not know about IFD 0, which `ifd` reads, so an IFD 0 that names itself as
/// the next one is caught here.
fn next_ifds(e: Endian, space: Space) -> T {
    let after_first = E::within(&["ifd", "next_ifd_offset"]);
    let first = E::cond(after_first.clone().equal_to(E::field("ifd_offset")), E::lit(0), after_first);
    T::chain(first, &["next_ifd_offset"], Anchor::Window, T::Named(space.named(e).into()))
}

/// Which set of names an entry's tag is read against. A directory is a
/// directory whichever one it is, but the numbers in it mean different things:
/// tag 1 is `subfile type` in a TIFF directory and `latitude ref` in a GPS
/// one, and reading a photograph's latitude as a subfile type would be worse
/// than not reading it at all.
#[derive(Clone, Copy, PartialEq)]
pub enum Space {
    Tiff,
    Sony,
    Exif,
    Gps,
    /// A JPEG XR's directory, which is this layout exactly and almost none of
    /// these numbers. See [`super::jxr`].
    Jxr,
}

impl Space {
    fn tags(self) -> &'static [(i128, &'static str)] {
        match self {
            Space::Tiff | Space::Sony => TAG,
            Space::Exif => EXIF_TAG,
            Space::Gps => GPS_TAG,
            Space::Jxr => super::jxr::TAG,
        }
    }
    /// What a reader sees the directory called.
    fn shown(self) -> &'static str {
        match self {
            Space::Tiff | Space::Sony => "Ifd",
            Space::Exif => "ExifIfd",
            Space::Gps => "GpsIfd",
            Space::Jxr => "JxrIfd",
        }
    }
    /// What it is called in the type table. A directory that holds directories
    /// has to name the one it is written like, and the whole layout is a
    /// function of the byte order, so each space is in the table twice.
    pub fn named(self, e: Endian) -> &'static str {
        match (self, e) {
            (Space::Tiff, Little) => "tiff.Ifd.le",
            (Space::Tiff, Big) => "tiff.Ifd.be",
            (Space::Sony, Little) => "arw.Ifd.le",
            (Space::Sony, Big) => "arw.Ifd.be",
            (Space::Exif, Little) => "tiff.ExifIfd.le",
            (Space::Exif, Big) => "tiff.ExifIfd.be",
            (Space::Gps, Little) => "tiff.GpsIfd.le",
            (Space::Gps, Big) => "tiff.GpsIfd.be",
            (Space::Jxr, Little) => "jxr.Ifd.le",
            (Space::Jxr, Big) => "jxr.Ifd.be",
        }
    }
    /// The tags in this space whose four bytes are the place another directory
    /// begins rather than a value.
    fn subs(self) -> &'static [(i128, Space)] {
        match self {
            Space::Tiff => SUB_IFD,
            Space::Sony => SONY_SUB_IFD,
            Space::Jxr => super::jxr::SUB_IFD,
            Space::Exif | Space::Gps => &[],
        }
    }
}

const SPACES: &[Space] = &[Space::Tiff, Space::Exif, Space::Gps];

/// One directory: how many entries, the entries, and where the next one is.
///
/// A JPEG XR's directory carries more than that. Where the picture is, is
/// spread across four entries of it, so the codestreams are placed here, after
/// the entries that say where they are and beside them rather than inside
/// them. See [`super::jxr::codestreams`].
fn ifd(e: Endian, space: Space) -> T {
    let mut fields = vec![
        ("count", T::u16(e)),
        ("entries", T::array(entry(e, space), E::field("count"))),
        // Zero when this is the last one, which is almost always.
        ("next_ifd_offset", T::u32(e)),
    ];
    if space == Space::Jxr {
        fields.extend(super::jxr::codestreams());
    }
    let images = matches!(space, Space::Tiff | Space::Sony);
    if images {
        fields.extend(image_data(e));
    }
    // The entries are what a directory is; `entries` is only the list's name,
    // so the listing shows them beside the count rather than a level down.
    // `next_ifd_offset` counts from the start of the TIFF. The chain of IFDs
    // reads it in IFD 0 and the ones after; an EXIF or GPS IFD has one too.
    let ifd = T::structure(space.shown(), fields).contents("entries").positions(&["next_ifd_offset"]);
    if !images {
        return ifd;
    }
    ifd.field_aside("strip_offsets")
        .field_aside("strip_byte_counts")
        .field_aside("tile_offsets")
        .field_aside("tile_byte_counts")
        .machinery(&["strip_offsets", "strip_byte_counts", "tile_offsets", "tile_byte_counts"])
        // A piece whose offset is past the end is passed over, and the ones
        // after it move up the list, so each is named by the number of the
        // offset that placed it rather than by where it is in the list.
        .field_elem_named_from("strips", E::words("strip {}", E::placer(E::idx())))
        .field_elem_named_from("tiles", E::words("tile {}", E::placer(E::idx())))
        .field_doc("strip_offsets", "The same bytes as StripOffsets (273), read a second time as a list. Each element holds a strip's offset and, beside it, that strip's length from StripByteCounts (279).")
        .field_doc("strip_byte_counts", "The same bytes as StripByteCounts (279), read a second time as a list. Each strip's length is looked up here by the strip's number.")
        .field_doc("tile_offsets", "The same bytes as TileOffsets (324), read a second time as a list. Each element holds a tile's offset and, beside it, that tile's length from TileByteCounts (325).")
        .field_doc("tile_byte_counts", "The same bytes as TileByteCounts (325), read a second time as a list. Each tile's length is looked up here by the tile's number.")
        .field_doc("strips", "The image data, split into strips of whole rows. Strip n starts at value n of StripOffsets (273), counted from the start of the TIFF, and its length is value n of StripByteCounts (279). Strips are numbered from 0. A strip whose offset is past the end of the TIFF is left out.")
        .field_doc("tiles", "The image data, split into tiles. Tile n starts at value n of TileOffsets (324), counted from the start of the TIFF, and its length is value n of TileByteCounts (325). Tiles are numbered from 0, left to right and then top to bottom. A tile whose offset is past the end of the TIFF is left out.")
        .field_doc("thumbnail", "The JPEG image at the offset in JPEGInterchangeFormat (513), counted from the start of the TIFF, with the length in JPEGInterchangeFormatLength (514). In IFD 1 of an EXIF block, this is the thumbnail. In a raw file, it can be a full-size preview.")
}

/// The image data a directory's entries point at: its strips or its tiles,
/// and the JPEG image of [`thumbnail`].
///
/// A strip's offset and its length are in two entries, StripOffsets and
/// StripByteCounts, each holding one number per strip. Strip `i` needs number
/// `i` of each, and no expression indexes into the values of an entry found by
/// its tag. So each of the two entries is read a second time, here, as a list
/// that can be indexed: the same bytes, which is why the copy is aside and
/// counts for nothing. Each offset is read as a record that also holds its
/// strip's length, and the strips are gathered from those records, so that a
/// strip is an element of a list and is named as one. Tiles are the same with
/// TileOffsets and TileByteCounts.
///
/// The byte counts are read first so that the offsets, which look them up,
/// come after them.
fn image_data(e: Endian) -> Vec<(&'static str, T)> {
    vec![
        ("strip_byte_counts", numbers(e, 279, |n| n)),
        ("strip_offsets", numbers(e, 273, |n| piece(n, "strip_byte_counts", 279))),
        ("strips", pieces("strip_offsets", 273, 279)),
        ("tile_byte_counts", numbers(e, 325, |n| n)),
        ("tile_offsets", numbers(e, 324, |n| piece(n, "tile_byte_counts", 325))),
        ("tiles", pieces("tile_offsets", 324, 325)),
        ("thumbnail", thumbnail()),
    ]
}

/// Field `field` of the entry with this tag, in the directory being read.
/// Zero when the directory has no such entry.
fn entry_with(tag: i128, field: &[&str]) -> E {
    E::tagged("entries", &["tag"], tag, field)
}

/// Whether the directory being read has an entry with this tag.
fn has(tag: i128) -> E {
    entry_with(tag, &["tag"]).equal_to(E::lit(tag))
}

/// The values of the entry with this tag, read again as a list of numbers
/// wherever they are: in the entry's own four bytes when they fit there, and
/// at the entry's offset when they do not. SHORT and LONG are the two types
/// the specification allows for offsets and byte counts; any other type is
/// read as an empty list.
///
/// Values stored elsewhere are read at the entry's offset, which counts from
/// the start of the TIFF, as the entry itself reads them. Values stored in the
/// entry are read where the entry's `value` field starts. That position is an
/// address in the file rather than in the TIFF, so it is read back with
/// `at_origin`, which counts from the same place `start_of` does.
///
/// `each` says what one number is read as: the number itself, or a record
/// holding it.
fn numbers(e: Endian, tag: i128, each: impl Fn(T) -> T) -> T {
    let count = || entry_with(tag, &["count"]);
    let list = T::inline_structure(
        "Numbers",
        vec![(
            "values",
            T::switch(
                entry_with(tag, &["type"]),
                vec![(3, T::array(each(T::u16(e)), count())), (4, T::array(each(T::u32(e)), count()))],
                T::array(each(T::u32(e)), E::lit(0)),
            ),
        )],
    );
    T::when(
        has(tag),
        T::switch(
            entry_with(tag, &["room"]).div(E::lit(5)),
            vec![(0, T::at_origin(E::start_of(entry_with(tag, &["value"])), list.clone()))],
            T::at_in_window(entry_with(tag, &["value", "offset"]), list),
        ),
    )
}

/// One offset of a strip or a tile, and that piece's length: number `i` of
/// the byte counts read just before, for offset `i`. An offset with no byte
/// count to go with it has a length of 0.
fn piece(offset: T, byte_counts: &str, count_tag: i128) -> T {
    let listed = E::idx().less_than(entry_with(count_tag, &["count"]));
    T::inline_structure(
        "Piece",
        vec![
            ("offset", offset),
            ("length", T::computed(E::cond(listed, E::elem_within(&[byte_counts, "values"], E::idx(), &[]), E::lit(0)))),
        ],
    )
}

/// One piece of image data for each offset, as long as the byte count beside
/// it says. Only when the directory has both entries.
///
/// A piece whose offset is past the end of the TIFF is passed over: a
/// Panasonic RW2 writes 0xffffffff in StripOffsets and keeps the real offset
/// in a tag of its own. A piece that runs past the end is read as far as the
/// file goes, which is what a file cut short still has.
fn pieces(offsets: &str, offset_tag: i128, count_tag: i128) -> T {
    let from = vec![Step::field(offsets), Step::field("values"), Step::each()];
    let piece = T::sized(E::placer(E::field("length")).at_most(E::Remaining), T::bytes(E::Remaining));
    T::when(has(offset_tag).both(has(count_tag)), T::gather(from, E::field("offset"), Anchor::Window, E::lit(0), piece))
}

/// `what`, at `offset` bytes from the start of the TIFF and `length` bytes
/// long. Nothing when the offset is past the end of the TIFF, and only as many
/// bytes as there are when the length runs past it.
fn placed(offset: E, length: E, what: T) -> T {
    T::when(
        offset.clone().less_than(E::WindowSize),
        T::at_in_window(offset.clone(), T::sized(length.at_most(E::WindowSize.sub(offset)), what)),
    )
}

/// The JPEG image that JPEGInterchangeFormat (513) points at, as long as
/// JPEGInterchangeFormatLength (514) says. In IFD1 of an EXIF block this is
/// the thumbnail. A raw file can use the same two tags for a full-size
/// preview, in IFD0 or in a SubIFD.
///
/// Each tag holds one LONG, so `values` is the number itself. An offset of 0
/// is read as no image, since byte 0 is where the TIFF header is.
///
/// Read as a JPEG when it starts with the two bytes a JPEG starts with, and as
/// bytes when it does not.
fn thumbnail() -> T {
    let offset = entry_with(513, &["value", "values"]);
    let length = entry_with(514, &["value", "values"]);
    let one = |tag: i128| has(tag).both(entry_with(tag, &["count"]).equal_to(E::lit(1)));
    let starts_as_jpeg = E::lit(1).less_than(E::Remaining).both(E::peek(16, Big).equal_to(E::lit(0xffd8)));
    let image = T::switch(starts_as_jpeg, vec![(1, T::Named(JPEG.into()))], T::bytes(E::Remaining));
    T::when(one(513).both(one(514)).both(offset.clone().greater_than(E::lit(0))), placed(offset, length, image))
}

/// One entry: what it is, what kind of thing it holds, how many, and then four
/// bytes that are either the thing itself or where to find it.
///
/// Which of the two those four bytes are is not written down anywhere. It is
/// the size of the type times the count: four bytes or fewer and the value is
/// there, more and they are an offset to it. So the size is worked out first,
/// in a field of no bits, and the switch below is on whether it fits.
fn entry(e: Endian, space: Space) -> T {
    let mut tags = space.tags().to_vec();
    if space == Space::Sony {
        for &(id, name) in SONY_TAG {
            if let Some(existing) = tags.iter_mut().find(|(number, _)| *number == id) {
                existing.1 = name;
            } else {
                tags.push((id, name));
            }
        }
    }
    T::structure_named(
        "Entry",
        "tag",
        "",
        vec![
            ("tag", T::enumeration("Tag", T::u16(e), &tags)),
            ("type", T::enumeration("FieldType", T::u16(e), FIELD_TYPE)),
            ("count", T::u32(e)),
            ("room", room()),
            // Dividing by five is how "four or fewer" is asked, since the
            // switch takes one number rather than a comparison.
            ("value", sub_ifd(e, space, T::switch(E::field("room").div(E::lit(5)), vec![(0, here(e))], elsewhere(e)))),
        ],
    )
    .counted_as("entry")
}

/// The tags whose four bytes are not a value but the place another directory
/// begins. A camera writes almost nothing about the photograph in the
/// directory the file points at: the exposure, the lens and the time are in
/// the one that `exif ifd` points at, and where the photograph was taken is in
/// the one after that.
/// Only a TIFF directory holds these. Inside the two they point at, the same
/// numbers mean something else entirely.
const SUB_IFD: &[(i128, Space)] = &[
    // Whole other images: the preview inside a raw file, and the pages of a
    // multi-resolution one.
    (330, Space::Tiff),
    (34665, Space::Exif),
    (34853, Space::Gps),
];

const SONY_SUB_IFD: &[(i128, Space)] = &[
    (330, Space::Sony),
    (34665, Space::Exif),
    (34853, Space::Gps),
];

/// An entry that points at a directory reads it, rather than showing the
/// offset and stopping. `sub ifd` may hold a list of offsets, which never fits
/// in the entry and is written elsewhere, and then every directory in the list
/// is read.
///
/// This sits at the entry rather than inside the four bytes of the value,
/// because those four bytes are a window of their own and an offset counted
/// from inside it would land four bytes into nowhere.
fn sub_ifd(e: Endian, space: Space, otherwise: T) -> T {
    if space.subs().is_empty() {
        return otherwise;
    }
    let mut cases = space
        .subs()
        .iter()
        .map(|(tag, to)| {
            let one = T::inline_structure(
                "SubIfd",
                vec![
                    ("offset", T::u32(e)),
                    ("directory", T::at_in_window(E::field("offset"), T::Named(to.named(e).into()))),
                ],
            );
            // Two or more LONG offsets never fit in four bytes, so the entry
            // holds where the list is, and each offset in the list is where a
            // directory is. A DNG keeps its raw image and its previews this
            // way, and a NEF its raw image and its full-size JPEG.
            let offsets = E::field("type").equal_to(E::lit(4)).either(E::field("type").equal_to(E::lit(13)));
            let many = T::inline_structure(
                "SubIfds",
                vec![
                    ("offset", T::u32(e)),
                    ("values", T::at_in_window(E::field("offset"), T::array(T::u32(e), E::field("count")))),
                    (
                        "directories",
                        T::array(
                            T::at_in_window(E::elem_within(&["values"], E::idx(), &[]), T::Named(to.named(e).into())),
                            E::field("count"),
                        ),
                    ),
                ],
            );
            let many = T::switch(E::lit(1).less_than(E::field("count")).both(offsets), vec![(1, many)], otherwise.clone());
            (*tag, T::switch(E::field("count"), vec![(1, one)], many))
        })
        .collect::<Vec<_>>();
    // A JPEG XR names its pixel format with a GUID rather than a number, and
    // sixteen bytes never fit in four, so what the entry holds is an offset
    // and the GUID is read where it points.
    if space == Space::Jxr {
        cases.extend(super::jxr::guid_values(e));
    }
    T::switch(E::field("tag"), cases, otherwise)
}

/// How many bytes the values of this entry need: the size of one of them times
/// how many there are. A type nobody here knows comes to nothing, which sends
/// it down the branch that reads the four bytes and says no more than that.
fn room() -> T {
    let cases = SIZE.iter().map(|(t, w)| (*t, T::computed(E::lit(*w).mul(E::field("count"))))).collect();
    T::switch(E::field("type"), cases, T::computed(E::lit(0)))
}

/// What one value of each type takes up. `FIELD_TYPE` names them; this is what
/// they cost.
const SIZE: &[(i128, i128)] = &[
    (1, 1),
    (2, 1),
    (3, 2),
    (4, 4),
    (5, 8),
    (6, 1),
    (7, 1),
    (8, 2),
    (9, 4),
    (10, 8),
    (11, 4),
    (12, 8),
    (13, 4),
    (16, 8),
    (17, 8),
    (18, 8),
];

/// The values, in the four bytes of the entry itself, written from the first
/// of them and padded out to four whichever way round the file is.
fn here(e: Endian) -> T {
    T::sized(
        E::lit(4),
        T::inline_structure("Here", vec![("values", values(e)), ("padding", T::bytes(E::Remaining))]),
    )
}

/// The values, somewhere else in the file, with the entry saying where. The
/// offset counts from the start of the TIFF, which is why it is anchored to
/// the window rather than to the file: in an EXIF block those are not the
/// same place.
fn elsewhere(e: Endian) -> T {
    T::inline_structure(
        "Elsewhere",
        vec![
            ("offset", T::u32(e)),
            ("values", T::at_in_window(E::field("offset"), values(e))),
        ],
    )
}

/// However many values of whatever type this entry holds, wherever they are.
/// Text and undefined bytes are a run rather than a list of one-byte things,
/// which is what they are; everything else is one value when the count says
/// one and a list when it says more.
fn values(e: Endian) -> T {
    let mut cases = vec![
        (2, T::text(StrLen::Padded { size: E::field("count"), pad: 0 }, Encoding::Ascii)),
        (7, T::bytes(E::field("count"))),
    ];
    for (t, _) in SIZE {
        if let Some(one) = one(e, *t) {
            cases.push((
                *t,
                T::switch(E::field("count"), vec![(1, one.clone())], T::array(one, E::field("count"))),
            ));
        }
    }
    // A type this does not know, in the room it turned out to need, which for
    // an unknown type is the four bytes of the entry and nothing more.
    T::switch(E::field("type"), cases, T::bytes(E::Remaining))
}

/// One value of a type, or nothing for the two that are read as a run.
fn one(e: Endian, t: i128) -> Option<T> {
    Some(match t {
        1 => T::u8(),
        3 => T::u16(e),
        4 | 13 => T::u32(e),
        5 => ratio(e, false),
        6 => T::Int { bits: 8, endian: e },
        8 => T::Int { bits: 16, endian: e },
        9 => T::i32(e),
        10 => ratio(e, true),
        11 => T::F32(e),
        12 => T::F64(e),
        16 | 18 => T::u64(e),
        17 => T::Int { bits: 64, endian: e },
        _ => return None,
    })
}

/// A rational: two numbers rather than one. A resolution of 300 dots an inch
/// is written 300 over 1, and an exposure of a two-hundredth of a second is
/// written 1 over 200, which is the reason the format has the type at all.
fn ratio(e: Endian, signed: bool) -> T {
    let n = || if signed { T::i32(e) } else { T::u32(e) };
    T::inline_structure("Rational", vec![("numerator", n()), ("denominator", n())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    #[test]
    fn jpeg_interchange_tag_keeps_its_standard_name_outside_sony_arw() {
        let mut bytes = tiff_bytes(true);
        bytes[18..20].copy_from_slice(&513u16.to_le_bytes());
        let d = Document::new(MemSource(bytes));
        for (template, label) in [
            (tiff(), "jpeg interchange format"),
            (camera_raw("arw"), "preview image start"),
        ] {
            let mut ev = Evaluator::new(template);
            assert_eq!(
                ev.node(&d, &[1, 2, 0, 1, 0, 0]).unwrap().value,
                Value::Enum {
                    raw: 513,
                    name: Some(label.into()),
                    hex: false,
                }
            );
        }
    }

    #[test]
    fn sony_arw_real_ifds_name_all_observed_tags() {
        let samples = std::env::var_os("QUBERO_SAMPLES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../qubero-samples"))
            });
        let path = samples.join("cameraraw/sony-ilce-7s-14bit-compressed.arw");
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(camera_raw("arw"));
        let ifd0 = [1, 2, 0, 1];
        let raw_ifd = [1, 2, 0, 1, 11, 4, 1, 0, 1];
        let exif_ifd = [1, 2, 0, 1, 15, 4, 1, 0, 1];
        for (prefix, count) in [
            (ifd0.as_slice(), 18),
            (raw_ifd.as_slice(), 27),
            (exif_ifd.as_slice(), 37),
        ] {
            for index in 0..count {
                let path = [prefix, &[index, 0]].concat();
                let tag = ev.node(&d, &path).unwrap_or_else(|error| panic!("{path:?}: {error:?}"));
                assert!(
                    matches!(tag.value, Value::Enum { name: Some(_), .. }),
                    "undefined tag at {path:?}: {:?}",
                    tag.value
                );
            }
        }
        assert_eq!(
            ev.node(&d, &[1, 2, 0, 1, 12, 0]).unwrap().value,
            Value::Enum {
                raw: 513,
                name: Some("preview image start".into()),
                hex: false,
            }
        );
        assert_eq!(
            ev.node(&d, &[1, 2, 0, 1, 13, 0]).unwrap().value,
            Value::Enum {
                raw: 514,
                name: Some("preview image length".into()),
                hex: false,
            }
        );
    }

    /// The same picture written both ways round: a two-entry directory at the
    /// end, and eight bytes of image before it.
    fn tiff_bytes(little: bool) -> Vec<u8> {
        let u16b = |v: u16| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
        let u32b = |v: u32| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };

        let mut v = if little { b"II".to_vec() } else { b"MM".to_vec() };
        v.extend_from_slice(&u16b(42));
        v.extend_from_slice(&u32b(16)); // the directory sits after the image
        v.extend_from_slice(&[0xaa; 8]); // the image, at byte 8
        assert_eq!(v.len(), 16);

        v.extend_from_slice(&u16b(2));
        for (tag, kind, count, value) in [(256u16, 3u16, 1u32, 640u32), (273, 4, 1, 8)] {
            v.extend_from_slice(&u16b(tag));
            v.extend_from_slice(&u16b(kind));
            v.extend_from_slice(&u32b(count));
            // A value that fits is written from the first of the four bytes
            // and padded out, whichever way round the file is. Writing a
            // short as a long would put it in the wrong two bytes.
            if kind == 3 {
                v.extend_from_slice(&u16b(value as u16));
                v.extend_from_slice(&[0, 0]);
            } else {
                v.extend_from_slice(&u32b(value));
            }
        }
        v.extend_from_slice(&u32b(0)); // no directory after this one
        v
    }

    /// A file whose one entry is `exif ifd`, pointing at a directory with one
    /// entry of its own. `exif_at` is where that second directory begins, so a
    /// test can send it somewhere it should not go.
    fn with_sub_ifd(tag: u16, exif_at: u32) -> Vec<u8> {
        let u16b = |v: u16| v.to_le_bytes().to_vec();
        let u32b = |v: u32| v.to_le_bytes().to_vec();
        let entry = |tag: u16, kind: u16, count: u32, value: u32| {
            let mut v = u16b(tag);
            v.extend_from_slice(&u16b(kind));
            v.extend_from_slice(&u32b(count));
            v.extend_from_slice(&u32b(value));
            v
        };
        let mut v = b"II".to_vec();
        v.extend_from_slice(&u16b(42));
        v.extend_from_slice(&u32b(8));
        // The first directory, at byte 8, ending at byte 26.
        v.extend_from_slice(&u16b(1));
        v.extend_from_slice(&entry(tag, 4, 1, exif_at));
        v.extend_from_slice(&u32b(0));
        assert_eq!(v.len(), 26);
        // The one it points at: the focal length, as a rational just past the
        // end of it, because eight bytes do not fit in four.
        v.extend_from_slice(&u16b(1));
        v.extend_from_slice(&entry(37386, 5, 1, 44));
        v.extend_from_slice(&u32b(0));
        assert_eq!(v.len(), 44);
        v.extend_from_slice(&u32b(50));
        v.extend_from_slice(&u32b(1));
        v
    }

    #[test]
    fn a_tag_that_points_at_a_directory_is_read_as_the_directory_it_points_at() {
        let d = Document::new(MemSource(with_sub_ifd(34665, 26)));
        let mut ev = Evaluator::new(tiff());
        let entry = [1, 2, 0, 1, 0];
        let at = |tail: &[usize]| [entry.as_slice(), tail].concat();
        assert_eq!(
            ev.node(&d, &at(&[0])).unwrap().value,
            Value::Enum { raw: 34665, name: Some("exif ifd".into()), hex: false }
        );
        // The four bytes are the offset, and the directory they point at costs
        // nothing where it is named, so the entry is still twelve bytes.
        assert_eq!(ev.node(&d, &at(&[4, 0])).unwrap().value, Value::UInt(26));
        let sub = ev.node(&d, &at(&[4, 1])).unwrap();
        assert_eq!(sub.size_bits, 0);
        let ifd = ev.node(&d, &at(&[4, 1, 0])).unwrap();
        assert_eq!(ifd.type_name, "ExifIfd");
        assert_eq!(ifd.offset_bits, 26 * 8);
        // Read against the camera's names rather than the file's: 37386 is a
        // tag nobody has in a TIFF directory.
        assert_eq!(
            ev.node(&d, &at(&[4, 1, 0, 1, 0, 0])).unwrap().value,
            Value::Enum { raw: 37386, name: Some("focal length".into()), hex: false }
        );
        // Eight bytes do not fit in four, so the value is elsewhere again:
        // an offset, and the pair of numbers it points at.
        let num = ev.node(&d, &at(&[4, 1, 0, 1, 0, 4, 1, 0, 0])).unwrap();
        assert_eq!(num.name, "numerator");
        assert_eq!(num.value, Value::UInt(50));
    }

    #[test]
    fn a_directory_that_points_back_at_the_one_holding_it_is_refused() {
        // A `sub ifd` entry saying the directory it points at is the one
        // holding it. Following that would read the same directory for ever.
        // The two the camera writes cannot do this: what they point at is read
        // against another set of names, and nothing in those points back.
        let d = Document::new(MemSource(with_sub_ifd(330, 8)));
        let mut ev = Evaluator::new(tiff());
        assert!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 1, 0]).is_err());
        // The rest of the file still reads, and so does the cursor.
        assert_eq!(ev.node(&d, &[1, 2, 0, 0]).unwrap().value, Value::UInt(1));
        assert_eq!(ev.locate(&d, 8 * 8).unwrap(), vec![1, 2, 0, 0]);
    }

    #[test]
    fn the_same_file_both_ways_round_reads_the_same() {
        for little in [true, false] {
            let d = Document::new(MemSource(tiff_bytes(little)));
            let mut ev = Evaluator::new(tiff());
            let order = ev.node(&d, &[0]).unwrap();
            let want = if little { "intel, little-endian" } else { "motorola, big-endian" };
            assert_eq!(order.value, Value::Enum { raw: if little { 0x4949 } else { 0x4d4d }, name: Some(want.into()), hex: false });
            assert_eq!(ev.node(&d, &[1, 0]).unwrap().value, Value::Enum { raw: 42, name: Some("tiff".into()), hex: false });
            assert_eq!(ev.node(&d, &[1, 1]).unwrap().value, Value::UInt(16));

            // The directory, at the far end of the file and costing nothing
            // where it is named.
            assert_eq!(ev.node(&d, &[1, 2]).unwrap().size_bits, 0);
            let ifd = ev.node(&d, &[1, 2, 0]).unwrap();
            assert_eq!(ifd.offset_bits, 16 * 8);
            assert_eq!(ifd.type_name, "Ifd");
            assert_eq!(ev.node(&d, &[1, 2, 0, 0]).unwrap().value, Value::UInt(2));

            let entries = ev.node(&d, &[1, 2, 0, 1]).unwrap();
            assert_eq!(entries.child_count, 2);
            assert_eq!(
                ev.node(&d, &[1, 2, 0, 1, 0, 0]).unwrap().value,
                Value::Enum { raw: 256, name: Some("image width".into()), hex: false }
            );
            assert_eq!(
                ev.node(&d, &[1, 2, 0, 1, 0, 1]).unwrap().value,
                Value::Enum { raw: 3, name: Some("short".into()), hex: false }
            );
            // A short with a count of one fits in the four bytes, so it is
            // read there and the two bytes left over are padding.
            assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 3]).unwrap().value, Value::Int(2), "room");
            assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 0]).unwrap().value, Value::UInt(640), "{little}");
            assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 1]).unwrap().size_bits, 16);
            assert_eq!(
                ev.node(&d, &[1, 2, 0, 1, 1, 0]).unwrap().value,
                Value::Enum { raw: 273, name: Some("strip offsets".into()), hex: false }
            );
            assert_eq!(ev.node(&d, &[1, 2, 0, 1, 1, 4, 0]).unwrap().value, Value::UInt(8));
            assert_eq!(ev.node(&d, &[1, 2, 0, 2]).unwrap().value, Value::UInt(0));
        }
    }

    #[test]
    fn the_image_is_still_covered_and_the_cursor_finds_both_of_them() {
        let d = Document::new(MemSource(tiff_bytes(true)));
        let mut ev = Evaluator::new(tiff());
        // Everything after the header, which the entries point into.
        let body = ev.node(&d, &[1, 3]).unwrap();
        assert_eq!(body.offset_bits, 8 * 8);
        // A byte of the image, and a byte of the directory that describes it.
        assert_eq!(ev.locate(&d, 10 * 8).unwrap(), vec![1, 3]);
        // The count sits at 16, so the first entry starts at 18: its tag is
        // the two bytes there, and its type the two after them.
        assert_eq!(ev.locate(&d, 18 * 8).unwrap(), vec![1, 2, 0, 1, 0, 0]);
        assert_eq!(ev.locate(&d, 20 * 8).unwrap(), vec![1, 2, 0, 1, 0, 1]);
    }

    #[test]
    fn a_value_too_big_for_its_entry_is_read_where_the_entry_says_it_is() {
        // Two entries whose values do not fit: a resolution, which is two
        // numbers, and a name, which is longer than four letters.
        let mut v = b"MM".to_vec();
        v.extend_from_slice(&42u16.to_be_bytes());
        v.extend_from_slice(&8u32.to_be_bytes());
        v.extend_from_slice(&2u16.to_be_bytes());
        for (tag, kind, count, at) in [(282u16, 5u16, 1u32, 38u32), (305, 2, 8, 46)] {
            v.extend_from_slice(&tag.to_be_bytes());
            v.extend_from_slice(&kind.to_be_bytes());
            v.extend_from_slice(&count.to_be_bytes());
            v.extend_from_slice(&at.to_be_bytes());
        }
        v.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(v.len(), 38);
        v.extend_from_slice(&300u32.to_be_bytes()); // three hundred
        v.extend_from_slice(&1u32.to_be_bytes()); // over one
        v.extend_from_slice(b"qubero\0\0");

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(tiff());
        // Eight bytes needed and four to hold them, so the four are an offset.
        assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 3]).unwrap().value, Value::Int(8));
        assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 0]).unwrap().value, Value::UInt(38));
        let ratio = ev.node(&d, &[1, 2, 0, 1, 0, 4, 1, 0]).unwrap();
        assert_eq!(ratio.type_name, "Rational");
        assert_eq!(ratio.offset_bits, 38 * 8);
        assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 1, 0, 0]).unwrap().value, Value::UInt(300));
        assert_eq!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 1, 0, 1]).unwrap().value, Value::UInt(1));

        // Text keeps the room the count gives it and reads as what is written
        // before the padding, which is how the format stores a name.
        let name = ev.node(&d, &[1, 2, 0, 1, 1, 4, 1, 0]).unwrap();
        assert_eq!(name.value, Value::Str("qubero".into()));
        assert_eq!(name.size_bits, 8 * 8);
    }

    #[test]
    fn an_entry_claiming_more_than_the_file_holds_is_an_error_for_that_entry_alone() {
        // Two entries, the first saying its text is two hundred letters long
        // when there are eight. The second is written correctly.
        let mut v = b"MM".to_vec();
        v.extend_from_slice(&42u16.to_be_bytes());
        v.extend_from_slice(&8u32.to_be_bytes());
        v.extend_from_slice(&2u16.to_be_bytes());
        for (tag, kind, count, at) in [(305u16, 2u16, 200u32, 38u32), (306, 2, 8, 38)] {
            v.extend_from_slice(&tag.to_be_bytes());
            v.extend_from_slice(&kind.to_be_bytes());
            v.extend_from_slice(&count.to_be_bytes());
            v.extend_from_slice(&at.to_be_bytes());
        }
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(b"qubero\0\0");

        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(tiff());
        // The tag and the type of the broken entry still read: what it says
        // about itself is readable even where what it points at is not.
        assert_eq!(
            ev.node(&d, &[1, 2, 0, 1, 0, 0]).unwrap().value,
            Value::Enum { raw: 305, name: Some("software".into()), hex: false }
        );
        assert!(ev.node(&d, &[1, 2, 0, 1, 0, 4, 1, 0]).is_err());
        // And the entry beside it is unharmed.
        assert_eq!(ev.node(&d, &[1, 2, 0, 1, 1, 4, 1, 0]).unwrap().value, Value::Str("qubero".into()));
    }

    /// A little-endian TIFF: the header, then `data` from byte 8, then one
    /// directory holding these entries, then the values too big to sit in an
    /// entry. Each entry is a tag, a type (3 for SHORT, 4 for LONG) and its
    /// values. `next` is the directory's next IFD offset.
    fn le_tiff(data: &[u8], entries: &[(u16, u16, &[u32])], next: u32) -> Vec<u8> {
        let ifd_at = 8 + data.len() as u32;
        let mut v = b"II".to_vec();
        v.extend_from_slice(&42u16.to_le_bytes());
        v.extend_from_slice(&ifd_at.to_le_bytes());
        v.extend_from_slice(data);
        let mut overflow_at = ifd_at + 2 + 12 * entries.len() as u32 + 4;
        let mut overflow = Vec::new();
        v.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, values) in entries {
            let bytes: Vec<u8> = values
                .iter()
                .flat_map(|x| if *kind == 3 { (*x as u16).to_le_bytes().to_vec() } else { x.to_le_bytes().to_vec() })
                .collect();
            v.extend_from_slice(&tag.to_le_bytes());
            v.extend_from_slice(&kind.to_le_bytes());
            v.extend_from_slice(&(values.len() as u32).to_le_bytes());
            if bytes.len() <= 4 {
                v.extend_from_slice(&bytes);
                v.extend(std::iter::repeat(0).take(4 - bytes.len()));
            } else {
                v.extend_from_slice(&overflow_at.to_le_bytes());
                overflow_at += bytes.len() as u32;
                overflow.extend_from_slice(&bytes);
            }
        }
        v.extend_from_slice(&next.to_le_bytes());
        v.extend_from_slice(&overflow);
        v
    }

    /// The path of field `field` of the first directory, then `tail`.
    fn in_ifd0(field: usize, tail: &[usize]) -> Vec<usize> {
        [&[1, 2, 0, field][..], tail].concat()
    }

    const STRIPS: usize = 5;
    const TILES: usize = 8;

    #[test]
    fn one_strip_whose_offset_and_length_fit_in_their_entries_is_placed_and_named() {
        let d = Document::new(MemSource(le_tiff(&[1, 2, 3, 4, 5, 6], &[(256, 3, &[3]), (273, 4, &[8]), (279, 4, &[6])], 0)));
        let mut ev = Evaluator::new(tiff());
        let strips = ev.node(&d, &in_ifd0(STRIPS, &[])).unwrap();
        assert_eq!(strips.name, "strips");
        assert_eq!(strips.child_count, 1);
        // The six bytes after the header, and the byte under the cursor is
        // in them.
        let strip = ev.node(&d, &in_ifd0(STRIPS, &[0])).unwrap();
        assert_eq!((strip.offset_bits / 8, strip.size_bits / 8), (8, 6));
        assert!(strip.name.contains("strip 0"), "{}", strip.name);
        assert_eq!(ev.locate(&d, 10 * 8).unwrap(), in_ifd0(STRIPS, &[0]));
    }

    #[test]
    fn strips_listed_elsewhere_are_placed_from_short_and_long_values() {
        // Three strips of 2, 3 and 1 bytes, out of order in the file. The
        // offsets are SHORTs and the byte counts LONGs, and neither list fits
        // in four bytes.
        let data = [0xa0, 0xa1, 0xb0, 0xb1, 0xb2, 0xc0];
        let entries: &[(u16, u16, &[u32])] = &[(273, 3, &[13, 8, 10]), (279, 4, &[1, 2, 3])];
        let d = Document::new(MemSource(le_tiff(&data, entries, 0)));
        let mut ev = Evaluator::new(tiff());
        assert_eq!(ev.node(&d, &in_ifd0(STRIPS, &[])).unwrap().child_count, 3);
        for (i, (at, len)) in [(13u64, 1u64), (8, 2), (10, 3)].into_iter().enumerate() {
            let strip = ev.node(&d, &in_ifd0(STRIPS, &[i])).unwrap();
            assert_eq!((strip.offset_bits / 8, strip.size_bits / 8), (at, len), "strip {i}");
            assert!(strip.name.contains(&format!("strip {i}")), "{}", strip.name);
        }
        // Two SHORT offsets take four bytes, so they sit in the entry itself.
        let entries: &[(u16, u16, &[u32])] = &[(273, 3, &[10, 8]), (279, 3, &[3, 2])];
        let d = Document::new(MemSource(le_tiff(&data, entries, 0)));
        let mut ev = Evaluator::new(tiff());
        let second = ev.node(&d, &in_ifd0(STRIPS, &[1])).unwrap();
        assert_eq!((second.offset_bits / 8, second.size_bits / 8), (8, 2));
    }

    #[test]
    fn tiles_are_placed_from_their_offsets_and_byte_counts() {
        let data = [1, 1, 2, 2, 3, 3, 4, 4];
        let entries: &[(u16, u16, &[u32])] = &[(322, 3, &[16]), (323, 3, &[16]), (324, 4, &[8, 10, 12, 14]), (325, 4, &[2, 2, 2, 2])];
        let d = Document::new(MemSource(le_tiff(&data, entries, 0)));
        let mut ev = Evaluator::new(tiff());
        let tile = ev.node(&d, &in_ifd0(TILES, &[3])).unwrap();
        assert_eq!((tile.offset_bits / 8, tile.size_bits / 8), (14, 2));
        assert_eq!(ev.locate(&d, 12 * 8).unwrap(), in_ifd0(TILES, &[2]));
        assert!(ev.node(&d, &in_ifd0(TILES, &[2])).unwrap().name.contains("tile 2"));
        // No strips in this file, so the list is not there.
        assert_eq!(ev.node(&d, &in_ifd0(STRIPS, &[])).unwrap().child_count, 0);
    }

    #[test]
    fn a_strip_past_the_end_is_left_out_and_one_running_past_it_is_cut_short() {
        // A Panasonic RW2 writes 0xffffffff as its strip offset. The second
        // strip says it is a thousand bytes long, and the file ends sooner.
        let data = [9, 9, 9, 9];
        let entries: &[(u16, u16, &[u32])] = &[(273, 4, &[0xffff_ffff, 8]), (279, 4, &[100, 1000])];
        let file = le_tiff(&data, entries, 0);
        let len = file.len() as u64;
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(tiff());
        // The first strip is passed over, and the second keeps its number.
        assert_eq!(ev.node(&d, &in_ifd0(STRIPS, &[])).unwrap().child_count, 1);
        let second = ev.node(&d, &in_ifd0(STRIPS, &[0])).unwrap();
        assert!(second.name.contains("strip 1"), "{}", second.name);
        assert_eq!(second.offset_bits / 8, 8);
        assert_eq!(second.size_bits / 8, len - 8);
    }

    const THUMBNAIL: usize = 9;

    /// `tiff` as the EXIF block of a JPEG that holds nothing else. The TIFF
    /// starts 12 bytes into the file.
    fn in_exif(tiff: &[u8]) -> Vec<u8> {
        let mut v = vec![0xff, 0xd8, 0xff, 0xe1];
        v.extend_from_slice(&(tiff.len() as u16 + 8).to_be_bytes());
        v.extend_from_slice(b"Exif\0\0");
        v.extend_from_slice(tiff);
        v.extend_from_slice(&[0xff, 0xd9]);
        v
    }

    /// The path of the TIFF inside a file made by `in_exif`: segment 0, its
    /// body, contents, App1, data, Exif, tiff.
    const EXIF: [usize; 6] = [1, 0, 1, 1, 1, 1];

    #[test]
    fn the_jpeg_that_513_and_514_point_at_is_placed_from_the_start_of_the_tiff() {
        let image = [0xff, 0xd8, 0xff, 0xd9];
        let file = le_tiff(&image, &[(513, 4, &[8]), (514, 4, &[4])], 0);
        let d = Document::new(MemSource(file.clone()));
        let mut ev = Evaluator::new(tiff());
        let thumbnail = ev.node(&d, &in_ifd0(THUMBNAIL, &[0])).unwrap();
        assert_eq!((thumbnail.offset_bits / 8, thumbnail.size_bits / 8), (8, 4));
        assert_eq!(ev.node(&d, &in_ifd0(THUMBNAIL, &[])).unwrap().name, "thumbnail");

        // The same TIFF in an EXIF block: offset 8 from the TIFF is 20 from
        // the file, and the end of the TIFF is where the segment ends.
        let d = Document::new(MemSource(in_exif(&file)));
        let mut ev = Evaluator::new(crate::formats::jpeg());
        let thumbnail = ev.node(&d, &[&EXIF[..], &in_ifd0(THUMBNAIL, &[0])].concat()).unwrap();
        assert_eq!((thumbnail.offset_bits / 8, thumbnail.size_bits / 8), (20, 4));
    }

    /// A TIFF the way a camera writes one into a JPEG: IFD 0 with one entry,
    /// IFD 1 after it holding only the thumbnail's two tags, and then the
    /// thumbnail.
    fn with_ifd1(thumbnail: &[u8]) -> Vec<u8> {
        let entry = |tag: u16, value: u32| {
            let mut v = tag.to_le_bytes().to_vec();
            v.extend_from_slice(&4u16.to_le_bytes());
            v.extend_from_slice(&1u32.to_le_bytes());
            v.extend_from_slice(&value.to_le_bytes());
            v
        };
        let mut v = b"II".to_vec();
        v.extend_from_slice(&42u16.to_le_bytes());
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&entry(256, 1));
        v.extend_from_slice(&26u32.to_le_bytes());
        assert_eq!(v.len(), 26);
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&entry(513, 56));
        v.extend_from_slice(&entry(514, thumbnail.len() as u32));
        v.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(v.len(), 56);
        v.extend_from_slice(thumbnail);
        v
    }

    const NEXT_IFDS: usize = 4;

    #[test]
    fn ifd1_is_read_from_the_next_ifd_offset_of_ifd0_and_its_thumbnail_placed() {
        let file = with_ifd1(&[0xff, 0xd8, 0xff, 0xd9]);
        let d = Document::new(MemSource(in_exif(&file)));
        let mut ev = Evaluator::new(crate::formats::jpeg());
        let next = [&EXIF[..], &[1, NEXT_IFDS]].concat();
        assert_eq!(ev.node(&d, &next).unwrap().child_count, 1);
        let ifd1 = ev.node(&d, &[&next[..], &[0]].concat()).unwrap();
        assert_eq!(ifd1.offset_bits / 8, 12 + 26);
        assert!(ifd1.name.contains("ifd 1"), "{}", ifd1.name);
        let thumbnail = [&next[..], &[0, THUMBNAIL, 0]].concat();
        let node = ev.node(&d, &thumbnail).unwrap();
        assert_eq!((node.offset_bits / 8, node.size_bits / 8), (12 + 56, 4));
        assert!(ev.locate(&d, (12 + 57) * 8).unwrap().starts_with(&thumbnail));
    }

    #[test]
    fn a_thumbnail_is_read_as_a_jpeg_and_so_is_the_thumbnail_inside_it() {
        // A JPEG whose thumbnail is a JPEG with a thumbnail of its own. The
        // outer TIFF starts at 12 and its thumbnail at 12 + 56. That is a JPEG
        // whose TIFF starts 12 further on, at 80, and whose thumbnail is the
        // four bytes at 80 + 56.
        let inner = in_exif(&with_ifd1(&[0xff, 0xd8, 0xff, 0xd9]));
        let d = Document::new(MemSource(in_exif(&with_ifd1(&inner))));
        let mut ev = Evaluator::new(crate::formats::jpeg());
        let thumbnail = [&EXIF[..], &[1, NEXT_IFDS, 0, THUMBNAIL, 0]].concat();
        let outer = ev.node(&d, &thumbnail).unwrap();
        assert_eq!(outer.type_name, "JPEG");
        assert_eq!(outer.offset_bits / 8, 68);
        let p = ev.locate(&d, 136 * 8).unwrap();
        let soi = ev.node(&d, &p).unwrap();
        assert_eq!((soi.offset_bits / 8, soi.size_bits / 8, soi.name.as_str()), (136, 2, "soi"));
        assert!(p.len() > thumbnail.len() * 2, "{p:?}");
        // A thumbnail that does not start as a JPEG is read as its bytes.
        let d = Document::new(MemSource(in_exif(&with_ifd1(&[1, 2, 3]))));
        let mut ev = Evaluator::new(crate::formats::jpeg());
        let bytes = ev.node(&d, &thumbnail).unwrap();
        assert_eq!((bytes.offset_bits / 8, bytes.size_bits / 8), (68, 3));
    }

    #[test]
    fn a_jpeg_inside_a_sony_arw_reads_its_own_exif_block() {
        // The ARW template's own directories are Sony's, and the EXIF block
        // inside the JPEG is a plain TIFF whose directory is `tiff.Ifd`.
        let inner = in_exif(&with_ifd1(&[0xff, 0xd8, 0xff, 0xd9]));
        let d = Document::new(MemSource(with_ifd1(&inner)));
        let mut ev = Evaluator::new(camera_raw("arw"));
        let jpeg = [1, NEXT_IFDS, 0, THUMBNAIL, 0];
        assert_eq!(ev.node(&d, &jpeg).unwrap().type_name, "JPEG");
        let ifd = ev.node(&d, &[&jpeg[..], &EXIF[..], &[1, 2, 0]].concat()).unwrap();
        assert_eq!((ifd.type_name.as_str(), ifd.offset_bits / 8), ("Ifd", 56 + 12 + 8));
    }

    #[test]
    fn the_last_of_twenty_thousand_strips_is_found_from_a_fresh_start() {
        // One row per strip is a common way to write a TIFF, and each strip's
        // offset and length are looked up from two lists of 20,000.
        let n = 20_000u32;
        let data = vec![0x5a; n as usize];
        let offsets: Vec<u32> = (0..n).map(|i| 8 + i).collect();
        let counts = vec![1u32; n as usize];
        let file = le_tiff(&data, &[(273, 4, &offsets), (279, 3, &counts)], 0);
        let d = Document::new(MemSource(file));
        let mut ev = Evaluator::new(tiff());
        let started = std::time::Instant::now();
        let p = ev.locate(&d, (8 + n as u64 - 1) * 8).unwrap();
        let strip = ev.node(&d, &p).unwrap();
        assert!(strip.name.contains(&format!("strip {}", n - 1)), "{}", strip.name);
        assert_eq!(strip.offset_bits / 8, 8 + n as u64 - 1);
        assert!(started.elapsed().as_secs() < 10, "took {:?}", started.elapsed());
    }

    /// A file from the sample collection, or nothing when it is not there.
    fn sample(path: &str) -> Option<Vec<u8>> {
        let samples = std::env::var_os("QUBERO_SAMPLES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../qubero-samples")));
        std::fs::read(samples.join(path)).ok()
    }

    #[test]
    fn real_pages_and_an_exif_thumbnail_are_placed_where_their_tags_say() {
        // Three pages of three strips each: IFD 1's strips are at 568, 707
        // and 846, 139 bytes each.
        if let Some(bytes) = sample("tiff/pillow-three-pages-deflate-strips.tif") {
            let d = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(tiff());
            assert_eq!(ev.node(&d, &[1, NEXT_IFDS]).unwrap().child_count, 2);
            for (i, at) in [568u64, 707, 846].into_iter().enumerate() {
                let strip = ev.node(&d, &[1, NEXT_IFDS, 0, STRIPS, i]).unwrap();
                assert_eq!((strip.offset_bits / 8, strip.size_bits / 8), (at, 139), "strip {i}");
            }
        }
        // IFD 1 says the thumbnail is 658 bytes, 278 from the start of the
        // TIFF, and the byte there is the thumbnail's start of image.
        if let Some(bytes) = sample("jpeg/pillow-exif-ifd1-thumbnail.jpg") {
            let tiff_at = bytes.windows(6).position(|w| w == b"Exif\0\0").unwrap() as u64 + 6;
            let d = Document::new(MemSource(bytes));
            let mut ev = Evaluator::new(crate::formats::jpeg());
            let p = ev.locate(&d, (tiff_at + 278) * 8).unwrap();
            let soi = ev.node(&d, &p).unwrap();
            assert_eq!((soi.offset_bits / 8, soi.name.as_str()), (tiff_at + 278, "soi"));
            let thumbnail = ev.node(&d, &p[..p.len() - 1]).unwrap();
            assert_eq!((thumbnail.type_name.as_str(), thumbnail.size_bits / 8), ("JPEG", 658));
        }
    }

    #[test]
    fn an_ifd0_that_names_itself_as_the_next_one_is_read_once() {
        let d = Document::new(MemSource(le_tiff(&[], &[(256, 3, &[1])], 8)));
        let mut ev = Evaluator::new(tiff());
        let next = ev.node(&d, &[1, NEXT_IFDS]).unwrap();
        assert_eq!(next.child_count, 0);
    }

    #[test]
    fn every_directory_a_list_of_sub_ifds_points_at_is_read_with_its_strips() {
        // Two sub-IFDs of two entries each, at 8 and 38, then five bytes of
        // strips: two for the first and three for the second.
        let sub = |strip_at: u32, len: u32| {
            let mut v = 2u16.to_le_bytes().to_vec();
            for (tag, value) in [(273u16, strip_at), (279, len)] {
                v.extend_from_slice(&tag.to_le_bytes());
                v.extend_from_slice(&4u16.to_le_bytes());
                v.extend_from_slice(&1u32.to_le_bytes());
                v.extend_from_slice(&value.to_le_bytes());
            }
            v.extend_from_slice(&0u32.to_le_bytes());
            v
        };
        let mut data = sub(68, 2);
        data.extend_from_slice(&sub(70, 3));
        data.extend_from_slice(&[1, 1, 2, 2, 2]);
        let d = Document::new(MemSource(le_tiff(&data, &[(330, 4, &[8, 38])], 0)));
        let mut ev = Evaluator::new(tiff());
        let value = [1, 2, 0, 1, 0, 4];
        let at = |tail: &[usize]| [&value[..], tail].concat();
        assert_eq!(ev.node(&d, &at(&[1, 0, 1])).unwrap().value, Value::UInt(38));
        let second = ev.node(&d, &at(&[2, 1, 0])).unwrap();
        assert_eq!(second.type_name, "Ifd");
        assert_eq!(second.offset_bits / 8, 38);
        let strip = ev.node(&d, &at(&[2, 1, 0, STRIPS, 0])).unwrap();
        assert_eq!((strip.offset_bits / 8, strip.size_bits / 8), (70, 3));
    }

    #[test]
    fn a_thumbnail_needs_both_its_tags() {
        // 513 with no 514 says where an image starts and not how long it is.
        let d = Document::new(MemSource(le_tiff(&[0xff, 0xd8, 0xff, 0xd9], &[(513, 4, &[8])], 0)));
        let mut ev = Evaluator::new(tiff());
        assert_eq!(ev.node(&d, &in_ifd0(THUMBNAIL, &[])).unwrap().child_count, 0);
        assert_eq!(ev.locate(&d, 9 * 8).unwrap(), vec![1, 3]);
    }

    #[test]
    fn a_byte_order_the_format_does_not_have_is_left_alone() {
        let mut v = b"XX".to_vec();
        v.extend_from_slice(&[0; 20]);
        let d = Document::new(MemSource(v));
        let mut ev = Evaluator::new(tiff());
        assert_eq!(ev.node(&d, &[0]).unwrap().value, Value::Enum { raw: 0x5858, name: None, hex: false });
        // Nothing is read as a number that could be either way round.
        assert_eq!(ev.node(&d, &[1]).unwrap().size_bits, 20 * 8);
    }

    /// Every IFD's `next_ifd_offset` is a position, including an EXIF or GPS
    /// IFD's that nothing follows. Its entry count is not.
    #[test]
    fn every_next_ifd_offset_is_a_position() {
        let t = tiff();
        for name in ["Ifd", "ExifIfd", "GpsIfd"] {
            crate::machinery::assert_marks(&t, name, &["next_ifd_offset"], &["count"]);
        }
    }
}
