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
    /// Adler-32, the zlib trailer: two running sums modulo 65521.
    Adler32,
    /// SHA-1, which git writes at the end of a file to seal it.
    Sha1,
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
            Checksum::Sum8 => "sum8",
            Checksum::Adler32 => "adler32",
            Checksum::Sha1 => "sha1",
        }
    }

    /// How many hex digits the number is printed to, so that the computed and
    /// the stored form of it can be compared as strings.
    pub fn digits(self) -> usize {
        match self {
            Checksum::Crc32 | Checksum::Adler32 => 8,
            Checksum::Crc32Low16 | Checksum::Crc16Arc => 4,
            Checksum::Sum8 => 2,
            Checksum::Sha1 => 40,
        }
    }

    /// True for the one sum here that is wider than a number: its stored form
    /// is read as bytes rather than as an integer a template gave an endianness
    /// to, and its computed form is written without an `0x`.
    pub fn is_digest(self) -> bool {
        matches!(self, Checksum::Sha1)
    }

    /// This sum over `bytes`, as the interface prints it.
    pub fn over(self, bytes: &[u8]) -> String {
        match self {
            Checksum::Sha1 => hex_bytes(&sha1(bytes)),
            Checksum::Crc32 => hex(crc32(bytes) as u128, self),
            Checksum::Crc32Low16 => hex((crc32(bytes) & 0xffff) as u128, self),
            Checksum::Crc16Arc => hex(crc16_arc(bytes) as u128, self),
            Checksum::Sum8 => hex(sum8(bytes) as u128, self),
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
