//! Per-bulb-family radio parameters.
//!
//! See `docs/protocol.md` §2.3. Each family uses its own PL1167 syncword and its
//! own trio of channels; a receiver only hears traffic whose syncword it is
//! configured for, which is why the families cannot interfere with one another.

use crate::error::{Error, Result};
use crate::framing::reverse_bits;

/// Highest channel the nRF24's `RF_CH` register can hold.
pub const MAX_NRF24_CHANNEL: u8 = 125;

/// The nRF24 channel register sits two channels above the PL1167 channel number.
const NRF24_CHANNEL_OFFSET: u8 = 2;

/// Number of channels each family hops between.
pub const NUM_CHANNELS: usize = 3;

/// The nRF24 address width used to absorb the PL1167 preamble, syncword and trailer.
pub const ADDRESS_LEN: usize = 5;

/// A PL1167 channel number.
///
/// Stored in PL1167 terms; [`Channel::to_nrf24`] converts to what the nRF24 wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Channel(u8);

impl Channel {
    /// Wrap a PL1167 channel number.
    ///
    /// # Panics
    ///
    /// Panics if the channel does not map into the nRF24's range. Use
    /// [`Channel::try_new`] for runtime input.
    #[must_use]
    pub const fn new(channel: u8) -> Self {
        assert!(
            channel <= MAX_NRF24_CHANNEL - NRF24_CHANNEL_OFFSET,
            "channel out of range"
        );
        Self(channel)
    }

    /// Wrap a PL1167 channel number, rejecting anything the nRF24 cannot tune to.
    pub const fn try_new(channel: u8) -> Result<Self> {
        if channel > MAX_NRF24_CHANNEL - NRF24_CHANNEL_OFFSET {
            return Err(Error::ChannelOutOfRange(channel));
        }
        Ok(Self(channel))
    }

    /// The PL1167 channel number.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// The value to write to the nRF24's `RF_CH` register.
    #[must_use]
    pub const fn to_nrf24(self) -> u8 {
        self.0 + NRF24_CHANNEL_OFFSET
    }

    /// Centre frequency in MHz.
    #[must_use]
    pub const fn frequency_mhz(self) -> u16 {
        2400 + self.to_nrf24() as u16
    }
}

/// Everything the radio needs to know to talk to one bulb family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RadioConfig {
    /// Human-readable family name, for logging.
    pub name: &'static str,
    /// Low half of the PL1167 syncword.
    pub syncword0: u16,
    /// High half of the PL1167 syncword.
    pub syncword3: u16,
    /// Fixed preamble byte.
    pub preamble: u8,
    /// Fixed trailer nibble, folded into the nRF24 address.
    pub trailer: u8,
    /// Packet length for this family, in bytes.
    pub packet_len: usize,
    /// The three channels this family hops between.
    pub channels: [Channel; NUM_CHANNELS],
}

impl RadioConfig {
    /// RGBW bulbs (FUT096). V1 protocol, 7-byte packets.
    pub const RGBW: Self = Self {
        name: "rgbw",
        syncword0: 0x147A,
        syncword3: 0x258B,
        preamble: 0xAA,
        trailer: 0x05,
        packet_len: 7,
        channels: [Channel::new(9), Channel::new(40), Channel::new(71)],
    };

    /// CCT bulbs (FUT007). V1 protocol, 7-byte packets.
    pub const CCT: Self = Self {
        name: "cct",
        syncword0: 0x050A,
        syncword3: 0x55AA,
        preamble: 0xAA,
        trailer: 0x05,
        packet_len: 7,
        channels: [Channel::new(4), Channel::new(39), Channel::new(74)],
    };

    /// RGB+CCT bulbs — FUT092, FUT089 and FUT091 all share this radio config.
    /// V2 protocol, 9-byte packets. This is what `pilight` drives.
    pub const RGB_CCT: Self = Self {
        name: "rgb_cct",
        syncword0: 0x7236,
        syncword3: 0x1809,
        preamble: 0xAA,
        trailer: 0x05,
        packet_len: 9,
        channels: [Channel::new(8), Channel::new(39), Channel::new(70)],
    };

    /// RGB bulbs (FUT098). V1 protocol, 6-byte packets.
    pub const RGB: Self = Self {
        name: "rgb",
        syncword0: 0x9AAB,
        syncword3: 0xBCCD,
        preamble: 0x55,
        trailer: 0x0A,
        packet_len: 6,
        channels: [Channel::new(3), Channel::new(38), Channel::new(73)],
    };

    /// FUT020 remotes. V1 protocol, 6-byte packets.
    pub const FUT020: Self = Self {
        name: "fut020",
        syncword0: 0x50A0,
        syncword3: 0xAA55,
        preamble: 0xAA,
        trailer: 0x0A,
        packet_len: 6,
        channels: [Channel::new(6), Channel::new(41), Channel::new(76)],
    };

    /// The nRF24 address that makes the radio match this family's PL1167 syncword.
    ///
    /// The trailer is 4 bits, so it would push packet data out of byte alignment.
    /// Folding preamble, syncword and trailer into a 5-byte address makes the
    /// nRF24's address matcher consume them for us. Index 0 is the byte the nRF24
    /// clocks out last, matching the convention of `openWritingPipe`-style APIs.
    ///
    /// See `docs/protocol.md` §3.2.
    #[must_use]
    pub const fn address(&self) -> [u8; ADDRESS_LEN] {
        [
            reverse_bits((((self.syncword3 >> 12) & 0x0F) as u8) | ((self.trailer << 4) & 0xF0)),
            reverse_bits(((self.syncword3 >> 4) & 0xFF) as u8),
            reverse_bits(
                (((self.syncword0 >> 12) & 0x0F) as u8) + (((self.syncword3 << 4) & 0xF0) as u8),
            ),
            reverse_bits(((self.syncword0 >> 4) & 0xFF) as u8),
            reverse_bits((((self.syncword0 << 4) & 0xF0) as u8) | (self.preamble & 0x0F)),
        ]
    }

    /// Length of the nRF24 payload for this family: length byte + packet + CRC.
    #[must_use]
    pub const fn payload_len(&self) -> usize {
        self.packet_len + crate::framing::FRAME_OVERHEAD
    }
}
