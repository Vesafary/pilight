//! V2 packet obfuscation.
//!
//! Bytes 1..=8 of a V2 packet are obfuscated with a keyed XOR plus a per-position
//! additive offset selected by the (plaintext) key byte at index 0. See
//! `docs/protocol.md` §4.2 for the derivation.
//!
//! **Every operation here is mod-256.** The offsets routinely push a byte past
//! `0xFF`, so all arithmetic uses the wrapping variants; using `+`/`-` makes debug
//! builds abort on the first packet.

/// Number of bytes in a V2 packet.
pub const V2_PACKET_LEN: usize = 9;

/// Keys in `[JUMP_START, JUMP_START + 0x80)` get `0x80` added to their offset.
const V2_OFFSET_JUMP_START: u8 = 0x54;

/// Per-position additive offsets, indexed by `[position - 1][key % 4]`.
const V2_OFFSETS: [[u8; 4]; 8] = [
    [0x45, 0x1F, 0x14, 0x5C], // 1: protocol id
    [0x2B, 0xC9, 0xE3, 0x11], // 2: device id, high byte
    [0x6D, 0x5F, 0x8A, 0x2B], // 3: device id, low byte
    [0xAF, 0x03, 0x1D, 0xF3], // 4: command
    [0x1A, 0xE2, 0xF0, 0xD1], // 5: argument
    [0x04, 0xD8, 0x71, 0x42], // 6: sequence
    [0xAF, 0x04, 0xDD, 0x07], // 7: group
    [0x61, 0x13, 0x38, 0x64], // 8: checksum
];

/// The checksum byte is encoded with this extra pre-XOR addend; every other byte uses 0.
const CHECKSUM_S1: u8 = 2;

/// Stateless encoder/decoder for the V2 obfuscation scheme.
///
/// ```
/// use pilight_proto::V2Encoder;
///
/// let plain = [0x00, 0x20, 0xBE, 0xEF, 0x01, 0x01, 0x00, 0x01, 0x00];
/// let encoded = V2Encoder::encode(plain);
/// assert_eq!(encoded, [0x00, 0xDB, 0x33, 0xC6, 0x66, 0xD1, 0xBA, 0x66, 0x9F]);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V2Encoder;

impl V2Encoder {
    /// Expand the plaintext key byte into the one-byte XOR key.
    ///
    /// The high and low nibbles are mangled independently; see `docs/protocol.md` §4.2.
    #[must_use]
    pub const fn xor_key(key: u8) -> u8 {
        let shift = if (key & 0x0F) < 0x04 { 0 } else { 1 };
        let x = (((key & 0xF0) >> 4) + shift + 6) % 8;
        let msn = (((4 + x) ^ 1) & 0x0F) << 4;
        let lsn = (((key & 0x0F) + 4) ^ 2) & 0x0F;

        msn | lsn
    }

    /// The additive offset for a packet position, given the key byte.
    ///
    /// `position` is 1-based and must be in `1..=8`.
    #[must_use]
    const fn offset(position: usize, key: u8, jump_start: u8) -> u8 {
        let in_jump_window =
            jump_start > 0 && key >= jump_start && key < jump_start.wrapping_add(0x80);
        let bump = if in_jump_window { 0x80 } else { 0 };

        V2_OFFSETS[position - 1][(key % 4) as usize].wrapping_add(bump)
    }

    #[must_use]
    const fn encode_byte(byte: u8, s1: u8, xor_key: u8, s2: u8) -> u8 {
        (byte.wrapping_add(s1) ^ xor_key).wrapping_add(s2)
    }

    #[must_use]
    const fn decode_byte(byte: u8, s1: u8, xor_key: u8, s2: u8) -> u8 {
        (byte.wrapping_sub(s2) ^ xor_key).wrapping_sub(s1)
    }

    /// The checksum a plaintext packet should carry.
    ///
    /// It is the XOR key plus the sum of bytes 1..=7. Byte 0 (the key itself) and
    /// byte 8 (the checksum slot) are excluded, so whatever is sitting in the
    /// checksum slot is ignored.
    #[must_use]
    pub fn checksum(packet: &[u8; V2_PACKET_LEN]) -> u8 {
        packet[1..8]
            .iter()
            .fold(Self::xor_key(packet[0]), |sum, byte| {
                sum.wrapping_add(*byte)
            })
    }

    /// Obfuscate a plaintext packet, filling in the checksum.
    ///
    /// Byte 0 is passed through untouched — it is the key, and it travels in the clear.
    #[must_use]
    pub fn encode(packet: [u8; V2_PACKET_LEN]) -> [u8; V2_PACKET_LEN] {
        let key = Self::xor_key(packet[0]);
        let checksum = Self::checksum(&packet);

        let mut out = packet;
        for position in 1..=7 {
            out[position] = Self::encode_byte(
                packet[position],
                0,
                key,
                Self::offset(position, packet[0], V2_OFFSET_JUMP_START),
            );
        }
        // The checksum slot deliberately skips the jump-start correction.
        out[8] = Self::encode_byte(checksum, CHECKSUM_S1, key, Self::offset(8, packet[0], 0));

        out
    }

    /// Recover the plaintext of an obfuscated packet.
    ///
    /// Byte 8 comes back as `checksum + 2`, because the checksum is encoded with a
    /// non-zero `s1`. Use [`V2Packet::checksum_is_valid`](crate::V2Packet::checksum_is_valid)
    /// rather than comparing it directly.
    #[must_use]
    pub fn decode(packet: [u8; V2_PACKET_LEN]) -> [u8; V2_PACKET_LEN] {
        let key = Self::xor_key(packet[0]);

        let mut out = packet;
        for position in 1..=8 {
            out[position] = Self::decode_byte(
                packet[position],
                0,
                key,
                Self::offset(position, packet[0], V2_OFFSET_JUMP_START),
            );
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_wraps_instead_of_panicking() {
        // 0x8A + 0x80 overflows a u8; position 3 with key%4 == 2 and a key inside
        // the jump-start window is the case that used to abort debug builds.
        assert_eq!(V2Encoder::offset(3, 0x56, V2_OFFSET_JUMP_START), 0x0A);
    }

    #[test]
    fn jump_start_window_is_inclusive_at_the_bottom_and_exclusive_at_the_top() {
        assert_eq!(V2Encoder::offset(1, 0x53, V2_OFFSET_JUMP_START), 0x5C);
        assert_eq!(V2Encoder::offset(1, 0x54, V2_OFFSET_JUMP_START), 0xC5);
        assert_eq!(V2Encoder::offset(1, 0xD3, V2_OFFSET_JUMP_START), 0xDC);
        assert_eq!(V2Encoder::offset(1, 0xD4, V2_OFFSET_JUMP_START), 0x45);
    }

    #[test]
    fn checksum_ignores_the_key_byte_and_the_checksum_slot() {
        let a = [0x00, 0x20, 0xBE, 0xEF, 0x01, 0x01, 0x00, 0x01, 0x00];
        let b = [0x00, 0x20, 0xBE, 0xEF, 0x01, 0x01, 0x00, 0x01, 0xFF];

        assert_eq!(V2Encoder::checksum(&a), V2Encoder::checksum(&b));
    }

    #[test]
    fn encode_never_panics_for_any_input() {
        for key in 0..=u8::MAX {
            let packet = [key, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
            let _ = V2Encoder::decode(V2Encoder::encode(packet));
        }
    }
}
