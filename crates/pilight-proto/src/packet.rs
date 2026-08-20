//! The 9-byte V2 packet.

use crate::encoder::{V2_PACKET_LEN, V2Encoder};

/// Protocol id of RGB+CCT bulbs (FUT092 remotes, 4 groups).
pub const PROTOCOL_ID_RGB_CCT: u8 = 0x20;
/// Protocol id of V2 CCT-only bulbs (FUT091 remotes, 4 groups).
pub const PROTOCOL_ID_FUT091: u8 = 0x21;
/// Protocol id of the 8-group RGB+CCT panel (FUT089 / B8 remotes).
pub const PROTOCOL_ID_FUT089: u8 = 0x25;

/// Bit 7 of the command byte marks a held (long-pressed) button.
pub const HELD_FLAG: u8 = 0x80;

/// A V2 packet in plaintext form.
///
/// Construct one with [`V2Packet::new`] and hand [`V2Packet::to_encoded`] to the
/// radio; parse received traffic with [`V2Packet::from_encoded`].
///
/// See `docs/protocol.md` §4.1 for the field layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V2Packet {
    bytes: [u8; V2_PACKET_LEN],
}

impl V2Packet {
    /// Build a packet from its fields.
    ///
    /// The checksum is computed at encode time, so it is not a parameter.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        key: u8,
        protocol_id: u8,
        device_id: u16,
        command: u8,
        argument: u8,
        sequence: u8,
        group: u8,
    ) -> Self {
        let [id_high, id_low] = device_id.to_be_bytes();

        Self {
            bytes: [
                key,
                protocol_id,
                id_high,
                id_low,
                command,
                argument,
                sequence,
                group,
                0,
            ],
        }
    }

    /// Interpret nine already-obfuscated bytes.
    #[must_use]
    pub fn from_encoded(encoded: [u8; V2_PACKET_LEN]) -> Self {
        Self {
            bytes: V2Encoder::decode(encoded),
        }
    }

    /// Interpret nine plaintext bytes.
    #[must_use]
    pub const fn from_plain(bytes: [u8; V2_PACKET_LEN]) -> Self {
        Self { bytes }
    }

    /// Obfuscate the packet, filling in the checksum.
    #[must_use]
    pub fn to_encoded(self) -> [u8; V2_PACKET_LEN] {
        V2Encoder::encode(self.bytes)
    }

    /// The plaintext bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; V2_PACKET_LEN] {
        &self.bytes
    }

    /// The obfuscation key (byte 0). Travels in the clear.
    #[must_use]
    pub const fn key(&self) -> u8 {
        self.bytes[0]
    }

    /// The bulb family (byte 1).
    #[must_use]
    pub const fn protocol_id(&self) -> u8 {
        self.bytes[1]
    }

    /// The 16-bit remote identity (bytes 2–3).
    #[must_use]
    pub const fn device_id(&self) -> u16 {
        ((self.bytes[2] as u16) << 8) | self.bytes[3] as u16
    }

    /// The command byte, including the held flag (byte 4).
    #[must_use]
    pub const fn command(&self) -> u8 {
        self.bytes[4]
    }

    /// The command byte with the held flag masked off.
    #[must_use]
    pub const fn command_id(&self) -> u8 {
        self.bytes[4] & !HELD_FLAG
    }

    /// Whether the button was held rather than tapped.
    #[must_use]
    pub const fn is_held(&self) -> bool {
        self.bytes[4] & HELD_FLAG != 0
    }

    /// The command's argument (byte 5).
    #[must_use]
    pub const fn argument(&self) -> u8 {
        self.bytes[5]
    }

    /// The sequence number (byte 6).
    #[must_use]
    pub const fn sequence(&self) -> u8 {
        self.bytes[6]
    }

    /// The group byte (byte 7).
    ///
    /// Unreliable for on/off commands — derive the group from the argument instead.
    /// See `docs/protocol.md` §5.1.
    #[must_use]
    pub const fn group(&self) -> u8 {
        self.bytes[7]
    }

    /// The checksum this packet should carry.
    #[must_use]
    pub fn checksum(&self) -> u8 {
        V2Encoder::checksum(&self.bytes)
    }

    /// Whether a decoded packet's checksum byte agrees with its contents.
    ///
    /// A decoded packet carries `checksum + 2` at index 8, because the checksum is
    /// encoded with a non-zero pre-XOR addend.
    #[must_use]
    pub fn checksum_is_valid(&self) -> bool {
        self.bytes[8] == self.checksum().wrapping_add(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_the_device_id_big_endian() {
        let packet = V2Packet::new(0, PROTOCOL_ID_RGB_CCT, 0xBEEF, 0, 0, 0, 0);

        assert_eq!(packet.as_bytes()[2], 0xBE);
        assert_eq!(packet.as_bytes()[3], 0xEF);
        assert_eq!(packet.device_id(), 0xBEEF);
    }

    #[test]
    fn separates_the_held_flag_from_the_command_id() {
        let packet = V2Packet::new(0, PROTOCOL_ID_RGB_CCT, 1, 0x01 | HELD_FLAG, 0, 0, 0);

        assert!(packet.is_held());
        assert_eq!(packet.command_id(), 0x01);
        assert_eq!(packet.command(), 0x81);
    }

    #[test]
    fn survives_an_encode_decode_round_trip() {
        let packet = V2Packet::new(0x00, PROTOCOL_ID_RGB_CCT, 0x8164, 0x02, 0x51, 0x2C, 0x01);
        let recovered = V2Packet::from_encoded(packet.to_encoded());

        assert_eq!(recovered.device_id(), packet.device_id());
        assert_eq!(recovered.command(), packet.command());
        assert_eq!(recovered.argument(), packet.argument());
        assert_eq!(recovered.sequence(), packet.sequence());
        assert_eq!(recovered.group(), packet.group());
        assert!(recovered.checksum_is_valid());
    }

    #[test]
    fn detects_a_tampered_packet() {
        let packet = V2Packet::new(0x00, PROTOCOL_ID_RGB_CCT, 0x8164, 0x02, 0x51, 0x2C, 0x01);
        let mut plain = *V2Packet::from_encoded(packet.to_encoded()).as_bytes();
        plain[5] ^= 0xFF;

        assert!(!V2Packet::from_plain(plain).checksum_is_valid());
    }
}
