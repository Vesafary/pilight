//! PL1167 frame construction.
//!
//! The bulbs speak PL1167. An nRF24L01+ can imitate one, but only if we build the
//! parts of the frame the nRF24 does not generate itself: the length byte and the
//! CRC. The nRF24's address matcher swallows the preamble, syncword and trailer
//! (see [`RadioConfig::address`](crate::RadioConfig::address)).
//!
//! On top of that the PL1167 is LSB-first while the nRF24 is MSB-first, so every
//! byte is bit-reversed on the way out and on the way in.
//!
//! See `docs/protocol.md` §3.

use crate::error::{Error, Result};

/// Reflected CRC-16/CCITT polynomial used by the PL1167.
pub const CRC_POLY: u16 = 0x8408;

/// Largest frame we will build or accept, matching the nRF24's FIFO.
pub const MAX_FRAME_LEN: usize = 32;

/// Bytes a frame adds on top of the packet: one length byte plus a two-byte CRC.
pub const FRAME_OVERHEAD: usize = 3;

/// Reverse the bit order of a byte.
///
/// The PL1167 clocks bits out LSB-first; the nRF24 does it MSB-first.
#[must_use]
pub const fn reverse_bits(byte: u8) -> u8 {
    byte.reverse_bits()
}

/// PL1167 CRC-16: polynomial [`CRC_POLY`], initial value 0, no final XOR.
///
/// Computed over the length byte plus the packet, before bit reversal.
#[must_use]
pub fn crc16(data: &[u8]) -> u16 {
    data.iter().fold(0u16, |state, byte| {
        (0..8)
            .fold((state, *byte), |(state, byte), _| {
                let next = if (u16::from(byte) ^ state) & 0x01 != 0 {
                    (state >> 1) ^ CRC_POLY
                } else {
                    state >> 1
                };
                (next, byte >> 1)
            })
            .0
    })
}

/// A complete PL1167 frame, ready to hand to the radio as an nRF24 payload.
///
/// Fixed-capacity so that building one allocates nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    bytes: [u8; MAX_FRAME_LEN],
    len: usize,
}

impl Frame {
    /// Wrap a packet: prepend its length, append the CRC, and bit-reverse everything.
    ///
    /// ```
    /// use pilight_proto::Frame;
    ///
    /// let packet = [0x00, 0xDB, 0x33, 0xC6, 0x66, 0xD1, 0xBA, 0x66, 0x9F];
    /// let frame = Frame::build(&packet).unwrap();
    /// assert_eq!(frame.len(), 12);
    /// ```
    pub fn build(packet: &[u8]) -> Result<Self> {
        let len = packet.len() + FRAME_OVERHEAD;
        if len > MAX_FRAME_LEN {
            return Err(Error::PayloadTooLong {
                len,
                max: MAX_FRAME_LEN,
            });
        }

        // Cannot fail: MAX_FRAME_LEN is far below u8::MAX, and len was just checked.
        let packet_len = u8::try_from(packet.len()).map_err(|_| Error::PayloadTooLong {
            len,
            max: MAX_FRAME_LEN,
        })?;

        // The CRC covers the length byte and the packet, in their un-reversed form.
        let mut plain = [0u8; MAX_FRAME_LEN];
        plain[0] = packet_len;
        plain[1..=packet.len()].copy_from_slice(packet);
        let crc = crc16(&plain[..=packet.len()]);

        let mut bytes = [0u8; MAX_FRAME_LEN];
        for (out, byte) in bytes.iter_mut().zip(plain[..=packet.len()].iter()) {
            *out = reverse_bits(*byte);
        }
        let [crc_low, crc_high] = crc.to_le_bytes();
        bytes[len - 2] = reverse_bits(crc_low);
        bytes[len - 1] = reverse_bits(crc_high);

        Ok(Self { bytes, len })
    }

    /// Undo [`Frame::build`]: bit-reverse, verify the CRC, and strip the length byte.
    ///
    /// Returns the packet as a `Frame` whose slice is the packet body.
    pub fn parse(wire: &[u8]) -> Result<Self> {
        if wire.len() < FRAME_OVERHEAD || wire.len() > MAX_FRAME_LEN {
            return Err(Error::FrameTooShort { len: wire.len() });
        }

        let mut plain = [0u8; MAX_FRAME_LEN];
        for (out, byte) in plain.iter_mut().zip(wire.iter()) {
            *out = reverse_bits(*byte);
        }

        let body_len = wire.len() - 2;
        let expected = crc16(&plain[..body_len]);
        let found = u16::from(plain[body_len]) | (u16::from(plain[body_len + 1]) << 8);
        if expected != found {
            return Err(Error::CrcMismatch { expected, found });
        }

        let declared = plain[0] as usize;
        if declared != body_len - 1 {
            return Err(Error::LengthMismatch {
                declared,
                found: body_len - 1,
            });
        }

        let mut bytes = [0u8; MAX_FRAME_LEN];
        bytes[..declared].copy_from_slice(&plain[1..=declared]);

        Ok(Self {
            bytes,
            len: declared,
        })
    }

    /// The frame's bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// How many bytes the frame holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the frame is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_nothing_is_zero() {
        assert_eq!(crc16(&[]), 0);
    }

    #[test]
    fn build_rejects_a_packet_that_cannot_fit() {
        let oversized = [0u8; MAX_FRAME_LEN];
        assert!(matches!(
            Frame::build(&oversized),
            Err(Error::PayloadTooLong {
                max: MAX_FRAME_LEN,
                ..
            })
        ));
    }

    #[test]
    fn parse_rejects_a_frame_whose_length_byte_lies() {
        // Build a valid frame, then rewrite the length byte and fix up the CRC so
        // that only the length check can catch it.
        let packet = [1u8, 2, 3, 4];
        let mut plain = vec![u8::try_from(packet.len()).unwrap()];
        plain.extend_from_slice(&packet);
        plain[0] = 99;

        let crc = crc16(&plain);
        let [crc_low, crc_high] = crc.to_le_bytes();
        let mut wire: Vec<u8> = plain.iter().map(|b| reverse_bits(*b)).collect();
        wire.push(reverse_bits(crc_low));
        wire.push(reverse_bits(crc_high));

        assert!(matches!(
            Frame::parse(&wire),
            Err(Error::LengthMismatch {
                declared: 99,
                found: 4
            })
        ));
    }

    #[test]
    fn parse_rejects_a_runt() {
        assert!(matches!(
            Frame::parse(&[0x01, 0x02]),
            Err(Error::FrameTooShort { len: 2 })
        ));
    }
}
