//! The sums a file writes down about itself.
//!
//! Every one of these is a fold over bytes with no state a caller has to keep,
//! so they are written as one function each rather than as a trait with an
//! update and a finish. Nothing here streams: what runs them is
//! [`crate::eval::Evaluator::run_check`], which has already read the covered
//! bytes into memory and has already refused anything past the decoders' cap.
//!
//! The formatting belongs here too. A verdict is two strings the interface puts
//! side by side, and the width they are printed to is a fact about the
//! algorithm rather than about the field: a CRC-32 is eight hex digits whether
//! the file wrote it as a `u32` or as four bytes, and a sum-8 that printed as
//! `0x0000001f` beside a stored `0x1f` would read as a mismatch to anyone
//! skimming.

/// Which sum a field holds. What tells two of these apart is the arithmetic,
/// not the width: [`Checksum::Crc32Low16`] and [`Checksum::Crc16Arc`] are both
/// sixteen bits and are different sums, and a format that meant one and got the
/// other reports every valid file as broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checksum {
    /// The reflected CRC-32 of PNG, ZIP, gzip and RAR: polynomial 0xedb88320,
    /// all ones in and all ones out.
    Crc32,
    /// The same sum kept to its bottom sixteen bits, which is what gzip's
    /// header check and a RAR 4 block header write. Not a CRC-16: the
    /// arithmetic is a CRC-32's throughout and only the stored half is narrow.
    Crc32Low16,
    /// The reflected CRC-16 with polynomial 0xa001 and nothing in or out,
    /// which is what LHA sums a file with. Called ARC after the archiver that
    /// first shipped it.
    Crc16Arc,
    /// Every byte added up, kept to eight bits. LHA's header check.
    Sum8,
    /// Every byte added up and not truncated: what a tar header writes as six
    /// octal digits. Kept to twenty-four bits, which is the width it is
    /// printed to and four times what the largest 512-byte header can reach.
    ///
    /// Not [`Checksum::Sum8`] with a wider field. A tar header of five hundred
    /// and twelve bytes reaches 0x1fe00, and the bottom eight bits of that are
    /// a different number that no tar wrote down.
    ByteSum,
    /// The *unreflected* CRC-32 an Ogg page seals itself with: the same
    /// polynomial 0x04c11db7 as [`Checksum::Crc32`] and nothing else the
    /// same. The bits of each byte go in the other way round, nothing is
    /// fed in at the start and nothing is taken out at the end.
    ///
    /// A different number from `Crc32` over the same bytes, not a variant
    /// spelling of it: `0x89a1897f` against `0xcbf43926` over `123456789`.
    /// Pointing the reflected one at an Ogg page reports every valid file as
    /// broken.
    Crc32Ogg,
    /// The same arithmetic again with all ones fed in and all ones taken out,
    /// which is what bzip2 writes over a block and over a stream. Not
    /// declared by any template yet: a block's sum covers that block's share
    /// of the unpacked bytes, and until a trace says where the blocks end
    /// there is nothing to sum it over. See `formats::bzip2`.
    Crc32Bzip2,
    /// Adler-32, the zlib trailer: two running sums modulo 65521.
    Adler32,
    /// The CRC-64 an xz block is sealed with, which several catalogues call
    /// CRC-64/XZ: the ECMA-182 polynomial 0x42f0e1eba9ea3693, reflected, with
    /// all ones fed in and all ones taken out.
    ///
    /// Named for xz rather than for ECMA because the two are different
    /// numbers. ECMA-182 itself is the same polynomial unreflected with
    /// nothing in and nothing out, and over `123456789` it comes to
    /// 0x6c40df5f0b497347 where this comes to 0x995dc9bbdf1939fa. A format
    /// that meant one and got the other calls every valid file broken, which
    /// is why [`Checksum::Crc32Ogg`] and [`Checksum::Crc32Bzip2`] are told
    /// apart in their names as well.
    ///
    /// The parameters are the xz specification's own, section 6, which gives
    /// the arithmetic as C rather than as a table of parameters for exactly
    /// this reason: the table is built by shifting right with the reversed
    /// polynomial 0xc96c5795d7870f42, and the sum starts and ends `~crc`.
    Crc64Xz,
    /// SHA-1, which git writes at the end of a file to seal it.
    Sha1,
    /// SHA-256, thirty-two bytes. What an xz stream written with `--check=sha256`
    /// seals each of its blocks with, and the digest most formats that want
    /// more than a CRC reach for.
    Sha256,
}

impl Checksum {
    /// The word that crosses the boundary, as
    /// [`Placed::as_str`](crate::eval::Placed::as_str) is: a view keys wording
    /// off these, so they stay put.
    ///
    /// Two sums share `crc16`. What a reader wants from that line is how wide
    /// the number is and roughly what kind of thing made it; which polynomial
    /// LHA chose is not something an interface can act on, and a name that
    /// carried it would be a name nothing else in the interface could match.
    pub fn as_str(self) -> &'static str {
        match self {
            Checksum::Crc32 => "crc32",
            Checksum::Crc32Low16 | Checksum::Crc16Arc => "crc16",
            Checksum::Crc32Ogg | Checksum::Crc32Bzip2 => "crc32",
            Checksum::Sum8 => "sum8",
            Checksum::ByteSum => "sum",
            Checksum::Adler32 => "adler32",
            Checksum::Crc64Xz => "crc64",
            Checksum::Sha1 => "sha1",
            Checksum::Sha256 => "sha256",
        }
    }

    /// How many hex digits the number is printed to, so that the computed and
    /// the stored form of it can be compared as strings.
    pub fn digits(self) -> usize {
        match self {
            Checksum::Crc32 | Checksum::Crc32Ogg | Checksum::Crc32Bzip2 | Checksum::Adler32 => 8,
            Checksum::Crc32Low16 | Checksum::Crc16Arc => 4,
            Checksum::Sum8 => 2,
            Checksum::ByteSum => 6,
            Checksum::Crc64Xz => 16,
            Checksum::Sha1 => 40,
            Checksum::Sha256 => 64,
        }
    }

    /// True for the sums here that are wider than a number: their stored form
    /// is read as bytes rather than as an integer a template gave an endianness
    /// to, and their computed form is written without an `0x`.
    ///
    /// A CRC-64 is not one of them. It is sixty-four bits and it is a number:
    /// xz writes it little-endian, the way it writes its CRC-32, and a reader
    /// comparing it against a sum of their own wants the number rather than
    /// eight bytes in the order the file happened to store them.
    pub fn is_digest(self) -> bool {
        matches!(self, Checksum::Sha1 | Checksum::Sha256)
    }

    /// This sum over `bytes`, as the interface prints it.
    pub fn over(self, bytes: &[u8]) -> String {
        match self {
            Checksum::Sha1 => hex_bytes(&sha1(bytes)),
            Checksum::Sha256 => hex_bytes(&sha256(bytes)),
            Checksum::Crc32 => hex(crc32(bytes) as u128, self),
            Checksum::Crc64Xz => hex(crc64_xz(bytes) as u128, self),
            Checksum::Crc32Low16 => hex((crc32(bytes) & 0xffff) as u128, self),
            Checksum::Crc32Ogg => hex(crc32_msb(bytes, 0, 0) as u128, self),
            Checksum::Crc32Bzip2 => hex(crc32_msb(bytes, !0, !0) as u128, self),
            Checksum::Crc16Arc => hex(crc16_arc(bytes) as u128, self),
            Checksum::Sum8 => hex(sum8(bytes) as u128, self),
            Checksum::ByteSum => hex(byte_sum(bytes) as u128, self),
            Checksum::Adler32 => hex(adler32(bytes) as u128, self),
        }
    }

    /// A number a field holds, printed the same way, so the two can be
    /// compared. Wider bits than the sum has are dropped rather than reported:
    /// a `u32` field holding a sixteen-bit check has sixteen bits of nothing at
    /// the top, and a template that pointed the wrong algorithm at a field
    /// should fail on the arithmetic and not on a stray high byte.
    pub fn stored(self, value: u128) -> String {
        hex(value, self)
    }
}

/// `0x` and the algorithm's own width. What the interface prints, and what two
/// of them are compared as.
fn hex(value: u128, of: Checksum) -> String {
    let width = of.digits();
    let mask = if width >= 32 { u128::MAX } else { (1u128 << (width * 4)) - 1 };
    format!("0x{:0width$x}", value & mask, width = width)
}

/// A digest as the forty characters it is written as everywhere else: no `0x`,
/// since nothing shows a hash as a number.
pub fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// The table the reflected CRC-32 is worked out from, built once.
fn crc32_table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (n, slot) in t.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 == 0 { c >> 1 } else { 0xedb8_8320 ^ (c >> 1) };
            }
            *slot = c;
        }
        t
    })
}

/// The CRC-32 of PNG, ZIP, gzip and RAR.
pub fn crc32(bytes: &[u8]) -> u32 {
    let t = crc32_table();
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc = t[((crc ^ b as u32) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

/// The table the unreflected CRC-32 is worked out from, built once. The same
/// polynomial as [`crc32_table`] and the other way up: the bits of a byte are
/// fed in from the top rather than from the bottom, so the shifts go the
/// other way and the polynomial is written as it is spelled rather than
/// reversed.
fn crc32_msb_table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (n, slot) in t.iter_mut().enumerate() {
            let mut c = (n as u32) << 24;
            for _ in 0..8 {
                c = if c & 0x8000_0000 == 0 { c << 1 } else { (c << 1) ^ 0x04c1_1db7 };
            }
            *slot = c;
        }
        t
    })
}

/// The unreflected CRC-32, with whatever a format feeds in at the start and
/// takes out at the end. Ogg does neither; bzip2 does both with all ones.
pub fn crc32_msb(bytes: &[u8], init: u32, xor_out: u32) -> u32 {
    let t = crc32_msb_table();
    let mut crc = init;
    for &b in bytes {
        crc = (crc << 8) ^ t[(((crc >> 24) ^ b as u32) & 0xff) as usize];
    }
    crc ^ xor_out
}

/// The table xz's CRC-64 is worked out from, built once. The polynomial is
/// the reversed spelling of ECMA-182's 0x42f0e1eba9ea3693, and the shift goes
/// right, which is what "reflected" comes to in a table: the same shape as
/// [`crc32_table`] with a wider word.
fn crc64_table() -> &'static [u64; 256] {
    static TABLE: std::sync::OnceLock<[u64; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u64; 256];
        for (n, slot) in t.iter_mut().enumerate() {
            let mut c = n as u64;
            for _ in 0..8 {
                c = if c & 1 == 0 { c >> 1 } else { 0xc96c_5795_d787_0f42 ^ (c >> 1) };
            }
            *slot = c;
        }
        t
    })
}

/// The CRC-64 an xz block's check holds, as the format's own specification
/// gives it in section 6: all ones in, all ones out, bits fed in from the
/// bottom of each byte.
///
/// Written from that code rather than from a catalogue on purpose. The
/// specification says there are incompatible variations of CRC-64 and prints
/// the arithmetic instead of naming one, and it is right to: the same
/// polynomial with nothing fed in and nothing taken out is ECMA-182, a
/// different number over the same bytes. See [`Checksum::Crc64Xz`].
pub fn crc64_xz(bytes: &[u8]) -> u64 {
    let t = crc64_table();
    let mut crc = u64::MAX;
    for &b in bytes {
        crc = t[((crc ^ b as u64) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

/// LHA's CRC-16: reflected, polynomial 0xa001, nothing in and nothing out.
pub fn crc16_arc(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 == 0 { crc >> 1 } else { (crc >> 1) ^ 0xa001 };
        }
    }
    crc
}

/// Every byte added up, kept to eight bits.
pub fn sum8(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |a, &b| a.wrapping_add(b))
}

/// Every byte added up, kept to twenty-four bits. What a tar header seals
/// itself with, once its own checksum field has been read as spaces.
///
/// The mask is the width the verdict is printed to and nothing more: no tar
/// header can reach it, and a template pointing this at a run long enough to
/// wrap has a bug the arithmetic should not hide by widening under it.
pub fn byte_sum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |a, &b| a.wrapping_add(b as u32)) & 0xff_ffff
}

/// Adler-32, the zlib trailer. Two running sums modulo 65521, the second of
/// the first, taken in blocks short enough that neither can overflow.
pub fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in bytes.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

/// SHA-1, twenty bytes.
///
/// Written out rather than pulled in. A dependency for eighty lines of shifts
/// would be a dependency the wasm build carries, and this is not being used to
/// vouch for anything: git writes a SHA-1 at the end of its index to catch a
/// truncated write, and that is the whole of what this answers. The vectors in
/// the tests below are RFC 3174's.
pub fn sha1(bytes: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476, 0xc3d2_e1f0];
    // The message, a one bit, zeroes, and the length in bits as a 64-bit
    // big-endian number, all of it a whole number of 64-byte blocks. Only the
    // tail is built: the blocks before it are read straight out of the input.
    let bits = (bytes.len() as u64).wrapping_mul(8);
    let whole = bytes.len() / 64 * 64;
    let mut tail = Vec::with_capacity(128);
    tail.extend_from_slice(&bytes[whole..]);
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&bits.to_be_bytes());

    for block in bytes[..whole].chunks_exact(64).chain(tail.chunks_exact(64)) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// The round constants: the first thirty-two bits of the fractional part of
/// the cube roots of the first sixty-four primes, which is where FIPS 180-4
/// says they come from. The eight words the sum starts at are the same thing
/// over square roots, and are written into [`sha256`] itself.
const SHA256_K: [u32; 64] = [
    0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5,
    0x3956_c25b, 0x59f1_11f1, 0x923f_82a4, 0xab1c_5ed5,
    0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3,
    0x72be_5d74, 0x80de_b1fe, 0x9bdc_06a7, 0xc19b_f174,
    0xe49b_69c1, 0xefbe_4786, 0x0fc1_9dc6, 0x240c_a1cc,
    0x2de9_2c6f, 0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da,
    0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7,
    0xc6e0_0bf3, 0xd5a7_9147, 0x06ca_6351, 0x1429_2967,
    0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc, 0x5338_0d13,
    0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85,
    0xa2bf_e8a1, 0xa81a_664b, 0xc24b_8b70, 0xc76c_51a3,
    0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070,
    0x19a4_c116, 0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5,
    0x391c_0cb3, 0x4ed8_aa4a, 0x5b9c_ca4f, 0x682e_6ff3,
    0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208,
    0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7, 0xc671_78f2,
];

/// SHA-256, thirty-two bytes.
///
/// Written out for the reason [`sha1`] is: a dependency for a hundred lines of
/// shifts is a dependency the wasm build carries, and nothing here is vouching
/// for anything. An xz block written with `--check=sha256` seals itself with
/// this, and answering whether those thirty-two bytes are the ones the data
/// comes to is the whole of what it is for.
///
/// The padding is SHA-1's, which is no coincidence: both take a one bit, then
/// zeroes to fifty-six bytes of the last block, then the length in bits as a
/// big-endian sixty-four-bit number. What differs is the compression
/// function, and it differs completely: eight working words rather than five,
/// a message schedule built from four shifted copies rather than one exclusive
/// or, and a constant per round rather than one per twenty.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a,
        0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19,
    ];
    // As in `sha1`: only the tail is built, and the blocks before it are read
    // out of the input where they already are.
    let bits = (bytes.len() as u64).wrapping_mul(8);
    let whole = bytes.len() / 64 * 64;
    let mut tail = Vec::with_capacity(128);
    tail.extend_from_slice(&bytes[whole..]);
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&bits.to_be_bytes());

    for block in bytes[..whole].chunks_exact(64).chain(tail.chunks_exact(64)) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        // The other forty-eight words of the schedule, each mixed out of four
        // earlier ones.
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut last] = h;
        for (i, &k) in SHA256_K.iter().enumerate() {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let t1 = last.wrapping_add(s1).wrapping_add(choice).wrapping_add(k).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            last = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, word) in h.iter_mut().zip([a, b, c, d, e, f, g, last]) {
            *slot = slot.wrapping_add(word);
        }
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// The check value every CRC catalogue prints: the sum over the nine ASCII
/// digits. Three sums here share a polynomial and agree on nothing else, and
/// a table built the wrong way up passes every test written against its own
/// output.
#[cfg(test)]
mod crc_check_values {
    use super::*;

    #[test]
    fn each_crc32_gives_the_number_its_catalogue_prints() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926, "reflected, as PNG and ZIP write it");
        assert_eq!(crc32_msb(b"123456789", 0, 0), 0x89a1_897f, "Ogg: nothing in, nothing out");
        assert_eq!(crc32_msb(b"123456789", !0, !0), 0xfc89_1918, "bzip2: all ones in and out");
    }

    /// The two are not two spellings of one number, which is the mistake a
    /// template makes by reaching for the sum it already had.
    #[test]
    fn the_reflected_and_unreflected_sums_are_different_numbers() {
        for text in [b"".as_slice(), b"a", b"OggS", b"the quick brown fox"] {
            assert_ne!(crc32(text) == 0, crc32_msb(text, 0, 0) != 0, "{text:?}");
        }
        assert_ne!(crc32(b"OggS"), crc32_msb(b"OggS", 0, 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crc32_matches_the_numbers_every_format_quotes() {
        // The check value every CRC catalogue gives for this polynomial.
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
        // And PNG's own: an IHDR of a one-pixel greyscale image.
        assert_eq!(crc32(&[0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]), 0x3a7e_9b55);
    }

    /// The number the catalogues print for CRC-64/XZ, and the one they print
    /// for the variant it is most likely to be confused with.
    ///
    /// The second assertion is the point of the first. ECMA-182 is the same
    /// polynomial read the other way up with nothing fed in and nothing taken
    /// out, and a table built that way passes every test written against its
    /// own output; what it does not do is agree with a `.xz` file.
    #[test]
    fn the_crc64_is_the_one_xz_uses_and_not_plain_ecma_182() {
        assert_eq!(crc64_xz(b"123456789"), 0x995d_c9bb_df19_39fa);
        assert_ne!(crc64_xz(b"123456789"), 0x6c40_df5f_0b49_7347, "that number is ECMA-182's");
        // All ones in and all ones out cancel over no bytes at all, which is
        // what an xz block holding nothing writes.
        assert_eq!(crc64_xz(b""), 0);
        assert_eq!(Checksum::Crc64Xz.over(b"123456789"), "0x995dc9bbdf1939fa");
    }

    #[test]
    fn the_sha256_matches_the_fips_vectors() {
        assert_eq!(hex_bytes(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex_bytes(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        // Longer than one block, so the padding is appended to a tail rather
        // than to the whole message and the length is written into a second
        // block: the path a one-block test cannot reach.
        assert_eq!(
            hex_bytes(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // 55 and 56 bytes: the last length that fits in its own block beside
        // the padding, and the first that does not.
        assert_eq!(hex_bytes(&sha256(&[b'a'; 55])), "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318");
        assert_eq!(hex_bytes(&sha256(&[b'a'; 56])), "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a");
        // Exactly one block, where the padding is a whole block by itself.
        assert_eq!(hex_bytes(&sha256(&[b'a'; 64])), "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb");
        // A million a's: FIPS's own long vector, and the one that would catch
        // a length counted in bytes rather than in bits.
        assert_eq!(
            hex_bytes(&sha256(&[b'a'; 1_000_000])),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        assert_eq!(Checksum::Sha256.over(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn the_crc16_is_the_arc_one_and_not_ccitt() {
        // 0xbb3d is ARC's check value; CCITT's is 0x29b1, and a template that
        // meant one and got the other calls every valid file broken.
        assert_eq!(crc16_arc(b"123456789"), 0xbb3d);
        assert_eq!(crc16_arc(b""), 0);
    }

    #[test]
    fn the_small_sums_are_what_they_say() {
        assert_eq!(sum8(&[0xff, 0x02]), 0x01);
        assert_eq!(sum8(b""), 0);
        assert_eq!(adler32(b"123456789"), 0x091e_01de);
        assert_eq!(adler32(b""), 1);
    }

    #[test]
    fn the_sha1_matches_the_rfc_vectors() {
        assert_eq!(hex_bytes(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex_bytes(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hex_bytes(&sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // A million a's: the block loop, the padding that needs a block of its
        // own, and the length past what a byte count would hold.
        assert_eq!(hex_bytes(&sha1(&[b'a'; 1_000_000])), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
        // Exactly 64 bytes: the input divides into blocks and the padding is a
        // whole block by itself, which is where an off-by-one lives.
        assert_eq!(hex_bytes(&sha1(&[b'a'; 64])), "0098ba824b5c16427bd7a1122a5a442a25ec644d");
        // 55 and 56 bytes: the last place the length fits in the block, and the
        // first place it does not.
        assert_eq!(hex_bytes(&sha1(&[b'a'; 55])), "c1c8bbdc22796e28c0e15163d20899b65621d65a");
        assert_eq!(hex_bytes(&sha1(&[b'a'; 56])), "c2db330f6083854c99d4b5bfb6e8f29f201be699");
    }

    #[test]
    fn a_sum_is_printed_to_its_own_width() {
        assert_eq!(Checksum::Crc32.over(b"123456789"), "0xcbf43926");
        assert_eq!(Checksum::Crc32Low16.over(b"123456789"), "0x3926");
        assert_eq!(Checksum::Sum8.over(&[0xff, 0x02]), "0x01");
        assert_eq!(Checksum::Sha1.over(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        // A field wider than its sum: the bits above the sum are not compared,
        // so a `u32` holding a sixteen-bit check still reads as one.
        assert_eq!(Checksum::Crc32Low16.stored(0x0000_3926), "0x3926");
    }
}
