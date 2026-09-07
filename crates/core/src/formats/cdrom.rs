//! Raw CD-ROM sector images: the `.bin` half of a `.bin`/`.cue` pair.
//!
//! A disc reader hands back what is written on the disc, which is 2352 bytes
//! per sector: twelve bytes of sync, a three-byte address, a mode byte, and
//! then whatever that mode says. A CD-ROM filesystem sees only the 2048 user
//! bytes in the middle of that, so an image of the same disc written as
//! [`crate::formats::iso9660`] is 2048 bytes a sector and has none of the rest.
//! Both are called `.bin`, or `.img`, or nothing at all.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// Bytes on the disc for every 2048 the filesystem sees.
pub const SECTOR: usize = 2352;

/// The twelve bytes that begin every sector of a disc that is not audio. The
/// only run of ten `ff` bytes a data sector may contain, since everything
/// after it is scrambled before it is written.
pub const SYNC: [u8; 12] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];

/// Mode 0 is named for what it is rather than for its number, since a sector
/// of nothing is worth saying outright. The other two are named as the disc
/// names them: "data" would be true of a Mode 2 Form 1 sector as well, and is
/// already the name of a submode flag and of a field.
const MODES: &[(i128, &str)] = &[(0, "empty"), (1, "mode 1"), (2, "mode 2")];

/// What the eight bytes after a Mode 2 header say about the sector.
const SUBMODE: &[(u32, &str)] = &[
    (0, "end of record"),
    (1, "video"),
    (2, "audio"),
    (3, "data"),
    (4, "trigger"),
    (5, "form 2"),
    (6, "real time"),
    (7, "end of file"),
];

pub fn cdrom() -> Template {
    Template::new("cdrom", T::structure("RawCdImage", vec![("sectors", T::repeat(sector(), Until::End))]))
}

/// One sector as the disc holds it.
///
/// Sized rather than left to add itself up, so that the sector after this one
/// is found by multiplying rather than by reading: a disc is a third of a
/// million of these, and the evaluator turns a run of same-sized elements into
/// a division. See `Evaluator::stride`.
fn sector() -> T {
    T::sized(
        E::lit(SECTOR as i128),
        T::structure_named(
            "CdSector",
            "address",
            "body",
            vec![
                ("sync", T::magic(&SYNC)),
                // Where the sector sits on the disc: minutes, seconds and
                // frames of playing time at 75 frames a second. Sector zero is
                // 00:02:00, since the two seconds before it are the lead-in.
                //
                // This is what names the row, because it is what tells one
                // sector from another. Naming them by mode instead gave a
                // column of sectors all called "data", which is true of every
                // sector on the disc and no help in finding one.
                //
                // Kept as its three bytes rather than read as three numbers.
                // Each is two decimal digits packed into a byte, which is not
                // a number this IR can read: as a `u8` the frame `0x74` would
                // say 116, and every frame from ten up would be wrong. The
                // bytes show as `00 02 16`, which is the address written the
                // way a disc writes it. A packed-decimal type is what this
                // really wants.
                ("address", T::bytes(E::lit(3))),
                ("mode", T::enumeration("SectorMode", T::u8(), MODES)),
                ("body", T::switch(E::field("mode"), vec![(1, mode1()), (2, mode2())], T::bytes(E::Remaining))),
            ],
        )
        .counted_as("sector"),
    )
}

/// Mode 1: 2048 bytes for the filesystem, then the checks.
fn mode1() -> T {
    T::structure(
        "Mode1Sector",
        vec![
            ("data", T::bytes(E::lit(2048))),
            ("edc", T::u32(Little)),
            ("reserved", T::bytes(E::lit(8))),
            ("ecc_p", T::bytes(E::lit(172))),
            ("ecc_q", T::bytes(E::lit(104))),
        ],
    )
}

/// Mode 2 with the CD-XA subheader, which is what a PlayStation disc, a Video
/// CD and a Photo CD are written in.
///
/// The subheader is written twice so that a reader can tell which copy is
/// wrong. Bit five of the submode byte picks the form: Form 1 is a filesystem
/// sector with the same 2048 bytes and the same checks as Mode 1, and Form 2
/// gives 276 of its bytes back to the data and keeps no correction, which is
/// what streamed audio and video want.
fn mode2() -> T {
    T::structure(
        "Mode2Sector",
        vec![
            ("file_number", T::u8()),
            ("channel_number", T::u8()),
            ("submode", T::flags("Submode", T::u8(), SUBMODE)),
            ("coding", T::u8()),
            ("file_number_copy", T::u8()),
            ("channel_number_copy", T::u8()),
            ("submode_copy", T::flags("Submode", T::u8(), SUBMODE)),
            ("coding_copy", T::u8()),
            (
                "body",
                T::switch(E::field("submode").and(E::lit(0x20)), vec![(0x20, form2())], form1()),
            ),
        ],
    )
}

fn form1() -> T {
    T::structure(
        "Mode2Form1",
        vec![
            ("data", T::bytes(E::lit(2048))),
            ("edc", T::u32(Little)),
            ("ecc_p", T::bytes(E::lit(172))),
            ("ecc_q", T::bytes(E::lit(104))),
        ],
    )
}

fn form2() -> T {
    T::structure("Mode2Form2", vec![("data", T::bytes(E::lit(2324))), ("edc", T::u32(Little))])
}

/// Whether this is a disc read sector by sector rather than file by file.
///
/// Nothing in the file says so. `.bin` is what a dumper calls whatever it
/// wrote, and the filesystem's own `CD001` sits at sector 16, which is 37,656
/// bytes in and past the window a sniffer is given. So the shape of the
/// sectors is the whole of the evidence, and it is enough: the sync pattern is
/// ten `ff` bytes in a row, which a data sector cannot otherwise contain
/// because everything after it is scrambled on the way to the disc, and it
/// repeats every 2352 bytes with an address that counts up in minutes, seconds
/// and frames.
///
/// Asked of four sectors rather than one. One sync is a run of `ff` bytes,
/// which a firmware image or a disk full of erased flash has by the thousand;
/// four of them 2352 bytes apart, each carrying the next address in sequence,
/// is a disc.
pub fn is_cdrom(head: &[u8], len: u64) -> bool {
    if len < (SECTOR * 4) as u64 || !len.is_multiple_of(SECTOR as u64) {
        return false;
    }
    let mut last = None;
    for i in 0..4 {
        let at = i * SECTOR;
        let Some(s) = head.get(at..at + 16) else { return false };
        if s[..12] != SYNC || !matches!(s[15], 0 | 1 | 2) {
            return false;
        }
        let Some(frames) = address(s[12], s[13], s[14]) else { return false };
        if last.is_some_and(|was| frames != was + 1) {
            return false;
        }
        last = Some(frames);
    }
    true
}

/// Minutes, seconds and frames as one count of frames, or nothing when the
/// bytes are not the two decimal digits each that a disc address is written
/// in. Seventy-five frames to the second, sixty seconds to the minute.
fn address(minute: u8, second: u8, frame: u8) -> Option<u64> {
    let digits = |b: u8| ((b >> 4) < 10 && (b & 0xf) < 10).then(|| (b >> 4) as u64 * 10 + (b & 0xf) as u64);
    let (m, s, f) = (digits(minute)?, digits(second)?, digits(frame)?);
    (s < 60 && f < 75).then_some((m * 60 + s) * 75 + f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::Evaluator;
    use crate::source::MemSource;
    use crate::Value;

    /// One Mode 2 Form 1 sector, laid out the way a PlayStation disc is.
    fn xa_sector(lba: u64, form2: bool) -> Vec<u8> {
        let mut v = SYNC.to_vec();
        let n = lba + 150;
        let bcd = |x: u64| ((x / 10) << 4 | (x % 10)) as u8;
        v.extend([bcd(n / (75 * 60)), bcd((n / 75) % 60), bcd(n % 75), 2]);
        let submode = if form2 { 0x28 } else { 0x08 };
        for _ in 0..2 {
            v.extend([0, 0, submode, 0]);
        }
        v.resize(SECTOR, 0x5a);
        v
    }

    #[test]
    fn a_disc_is_read_sector_by_sector() {
        let mut v = Vec::new();
        for lba in 0..21 {
            v.extend(xa_sector(lba, lba == 3));
        }
        assert!(is_cdrom(&v, v.len() as u64));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(cdrom());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 21);
        assert_eq!(e.node(&d, &[0, 1]).unwrap().size_bits, SECTOR as u64 * 8);
        assert_eq!(e.node(&d, &[0, 1]).unwrap().offset_bits, SECTOR as u64 * 8);
        // The second sector of the disc is two seconds and one frame into the
        // playing time, written as three bytes of two decimal digits each.
        let mut digits = |path: &[usize]| match e.node(&d, path).unwrap().value {
            Value::Bytes { preview, .. } => preview,
            other => panic!("{other:?}"),
        };
        assert_eq!(digits(&[0, 1, 1]), vec![0x00, 0x02, 0x01]);
        assert_eq!(digits(&[0, 20, 1]), vec![0x00, 0x02, 0x20], "frame 20, not 0x14");
        // Frame 74 is the last of a second, and is written 0x74 rather than
        // 0x4a. That is the whole reason these are not u8. Counted from the
        // start of the playing time, so two seconds ahead of sector zero.
        assert_eq!(address(0x00, 0x02, 0x74), Some(150 + 74));
        assert_eq!(address(0x00, 0x02, 0x4a), None, "0x4a has a digit no decimal number has");
        assert_eq!(address(0x00, 0x60, 0x00), None, "sixty seconds is a minute");
    }

    #[test]
    fn bit_five_of_the_submode_picks_the_form() {
        let mut v = Vec::new();
        for lba in 0..4 {
            v.extend(xa_sector(lba, lba == 3));
        }
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(cdrom());
        // The body of the XA sector, which is the ninth field of the mode 2
        // structure sitting in the sector's fourth.
        let form1 = e.node(&d, &[0, 0, 3, 8]).unwrap();
        let form2 = e.node(&d, &[0, 3, 3, 8]).unwrap();
        assert_eq!(form1.child_count, 4, "form 1 keeps its correction");
        assert_eq!(form2.child_count, 2, "form 2 gives it to the data");
    }

    #[test]
    fn a_file_that_is_not_a_disc_is_turned_away() {
        let ok = {
            let mut v = Vec::new();
            for lba in 0..4 {
                v.extend(xa_sector(lba, false));
            }
            v
        };
        assert!(is_cdrom(&ok, ok.len() as u64));

        // Erased flash is a run of ff bytes and nothing else, which holds the
        // sync pattern everywhere and an address nowhere.
        let erased = vec![0xff; SECTOR * 8];
        assert!(!is_cdrom(&erased, erased.len() as u64));

        // A sector that does not carry the next address is a coincidence.
        let mut jumbled = ok.clone();
        jumbled[SECTOR + 14] = 0x40;
        assert!(!is_cdrom(&jumbled, jumbled.len() as u64));

        // An address is two decimal digits to the byte, so 0x0a is not one.
        let mut hex = ok.clone();
        hex[14] = 0x0a;
        assert!(!is_cdrom(&hex, hex.len() as u64));

        // A file whose length is not a whole number of sectors was written by
        // something else, whatever its first bytes say.
        assert!(!is_cdrom(&ok, ok.len() as u64 - 1));
        assert!(!is_cdrom(&[], 0));
    }
}
