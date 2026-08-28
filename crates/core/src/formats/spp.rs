//! CCSDS Space Packets (CCSDS 133.0-B-2): what a spacecraft's telemetry is a
//! stream of, and what a command sent to one is.
//!
//! A packet is a six-octet header and then its data field, and the whole of
//! the header that says how long that is, is two octets holding a count one
//! less than the length. There is no magic, no start-of-packet marker, and no
//! checksum: a packet is found only by having read the one before it, which
//! is what makes a stream of them worth walking rather than reading by eye.
//!
//! What is inside the data field is the mission's business. The standard says
//! only that a secondary header may come first, that its format is registered
//! with the mission rather than here, and that a packet whose APID is all
//! ones is an idle packet, sent to keep a downlink busy when there is nothing
//! to say. A capture is mostly those.

use crate::template::{Endian::Big, Expr as E, Template, Ty as T, Until};

/// How long the header before the data field is.
const PRIMARY_HEADER: usize = 6;

/// Telemetry from the spacecraft, or a command to it.
const TYPES: &[(i128, &str)] = &[(0, "telemetry"), (1, "telecommand")];

/// Whether the data field is a piece of something larger. Most streams say
/// `unsegmented` for everything and use the APID to sort packets out.
const SEQUENCE_FLAGS: &[(i128, &str)] = &[
    (0, "continuation segment"),
    (1, "first segment"),
    (2, "last segment"),
    (3, "unsegmented"),
];

/// The one APID the standard reserves. Everything else is named by the
/// mission, so a number with no name here is the ordinary case rather than
/// something unrecognised.
const APIDS: &[(i128, &str)] = &[(0x7ff, "idle packet")];

pub fn spp() -> Template {
    Template::new(
        "spp",
        T::structure("SpacePackets", vec![("packets", T::repeat(packet(), Until::End))]),
    )
}

fn packet() -> T {
    T::structure_named(
        "SpacePacket",
        "apid",
        "data",
        vec![
            // Zero, and only zero: the field exists so that some later
            // standard could define a packet that is not this one.
            ("version", T::UInt { bits: 3, endian: Big }),
            ("packet_type", T::enumeration("PacketType", T::UInt { bits: 1, endian: Big }, TYPES)),
            ("secondary_header_flag", T::UInt { bits: 1, endian: Big }),
            // The name of the application the packet belongs to, and the only
            // thing in the header that says what the data field holds.
            ("apid", T::enumeration_hex("Apid", T::UInt { bits: 11, endian: Big }, APIDS)),
            (
                "sequence_flags",
                T::enumeration("SequenceFlags", T::UInt { bits: 2, endian: Big }, SEQUENCE_FLAGS),
            ),
            // Counts on, per APID, modulo 16384, so a gap in it is a packet
            // that did not arrive. A telecommand may use it as a name for the
            // packet instead of a count.
            ("sequence_count", T::UInt { bits: 14, endian: Big }),
            // One fewer than the length of the data field, which is what
            // makes a packet of no data impossible to write and a packet of
            // one octet the smallest there is.
            ("data_length", T::u16(Big)),
            // The secondary header, if the flag says there is one, and then
            // the user data. Where one ends and the other starts is
            // registered with the mission and is not in the packet.
            //
            // A capture is a recording that was stopped, so the last packet
            // of one is regularly shorter than it says it is. What there is
            // of it is still the packet.
            ("data", T::bytes(E::field("data_length").add(E::lit(1)).at_most(E::Remaining))),
        ],
    )
    .counted_as("packet")
}

/// Whether this file is a stream of space packets.
///
/// Nothing marks the front of a packet, so the only evidence is that the
/// lengths chain: read a header, step over the data field it declares, and
/// find another header where it said the next one would be. A few in a row is
/// weak, so this asks for eight, or for the whole file when it is shorter
/// than that.
///
/// The version field is three zero bits, which is most of what stops this
/// claiming any file beginning with a small byte. A file of zeros chains
/// perfectly well as seven-octet packets of nothing, so a stream in which no
/// header says anything at all is refused: the cost of that is a capture
/// consisting entirely of APID 0 packets with empty data fields, which is not
/// a capture of anything.
pub fn is_spp(head: &[u8], len: u64) -> bool {
    const ENOUGH: usize = 8;
    let mut at = 0usize;
    let mut packets = 0usize;
    let mut anything = false;
    while packets < ENOUGH {
        let Some(header) = head.get(at..at + PRIMARY_HEADER) else { break };
        if header[0] >> 5 != 0 {
            return false;
        }
        anything |= header.iter().any(|&b| b != 0);
        let data = u16::from_be_bytes([header[4], header[5]]) as usize + 1;
        let next = at + PRIMARY_HEADER + data;
        // The last packet of a capture cut off mid-transmission is still a
        // packet, and so is one that runs past the window this is reading.
        if next as u64 > len {
            break;
        }
        at = next;
        packets += 1;
        if head.get(at).is_none() {
            break;
        }
    }
    anything && (packets >= ENOUGH || (packets > 0 && at as u64 == len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document,
        eval::{Evaluator, Value},
        source::MemSource,
    };

    fn packet_bytes(apid: u16, seq: u16, data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&apid.to_be_bytes()); // version 0, telemetry, no secondary header
        v.extend_from_slice(&(0xc000 | seq).to_be_bytes()); // unsegmented
        v.extend_from_slice(&((data.len() - 1) as u16).to_be_bytes());
        v.extend_from_slice(data);
        v
    }

    fn stream(n: u16) -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..n {
            v.extend_from_slice(&packet_bytes(0x17, i, b"a message"));
        }
        v
    }

    #[test]
    fn a_packet_is_found_by_the_length_of_the_one_before_it() {
        let mut v = packet_bytes(0x17, 5052, b"hello");
        v.extend_from_slice(&packet_bytes(0x7ff, 0, b"idle idle idle"));
        let d = Document::new(MemSource(v));
        let mut e = Evaluator::new(spp());
        assert_eq!(e.node(&d, &[0]).unwrap().child_count, 2);
        assert_eq!(e.node(&d, &[0, 0, 3]).unwrap().value.as_int(), Some(0x17));
        assert_eq!(e.node(&d, &[0, 0, 5]).unwrap().value.as_int(), Some(5052));
        // The length in the header is one fewer than the data field.
        assert_eq!(e.node(&d, &[0, 0, 6]).unwrap().value.as_int(), Some(4));
        assert_eq!(e.node(&d, &[0, 0, 7]).unwrap().size_bits, 5 * 8);
        let idle = e.node(&d, &[0, 1, 3]).unwrap().value;
        assert!(matches!(idle, Value::Enum { raw: 0x7ff, name: Some(_), .. }), "idle not named: {idle:?}");
        assert_eq!(e.node(&d, &[0, 1, 7]).unwrap().size_bits, 14 * 8);
    }

    /// The three bits at the front of every packet, which a stream of them
    /// has to agree on for the walk to mean anything.
    #[test]
    fn a_stream_is_recognised_by_its_lengths_chaining() {
        let v = stream(8);
        assert!(is_spp(&v, v.len() as u64));
        // A short capture is allowed when it ends exactly on a boundary.
        let two = stream(2);
        assert!(is_spp(&two, two.len() as u64));
        // Cut a byte off the end and the chain no longer lands anywhere.
        assert!(!is_spp(&two[..two.len() - 1], two.len() as u64 - 1));
        // A version other than zero is not this standard.
        let mut wrong = stream(8);
        wrong[0] = 0x20;
        assert!(!is_spp(&wrong, wrong.len() as u64));
        // A file of zeros chains as packets of nothing and is turned away.
        assert!(!is_spp(&vec![0; 700], 700));
    }
}
