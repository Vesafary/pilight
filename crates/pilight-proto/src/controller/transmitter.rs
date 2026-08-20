//! Sending intents to arbitrary lamps over one radio.

use super::command::GroupId;
use super::intent::RgbCctIntent;
use crate::error::Result;
use crate::packet::{PROTOCOL_ID_RGB_CCT, V2Packet};
use crate::radio::{MiLightRadio, RadioConfig, Transceiver};
use std::time::Duration;

/// Which bulb group a command is aimed at.
///
/// This is the whole of a lamp's identity as far as the radio is concerned; the
/// name, room and remembered state live elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LampAddress {
    /// The 16-bit identity to transmit as.
    pub device_id: u16,
    /// The group, `0` meaning all groups.
    pub group: GroupId,
}

impl LampAddress {
    /// Build an address, validating the group against a four-group remote.
    pub const fn new(device_id: u16, group: GroupId) -> Self {
        Self { device_id, group }
    }
}

/// Sends RGB+CCT commands to any lamp, over a single radio.
///
/// Unlike [`RgbCctController`](super::RgbCctController), this holds no lamp
/// identity of its own: the address and sequence number are supplied per call.
/// That is what a service driving many lamps from one nRF24 needs.
///
/// Sending **blocks** — a command is repeated across three channels with a pause
/// between bursts, which takes a few hundred milliseconds. Call it from a blocking
/// context, not directly on an async runtime.
#[derive(Debug)]
pub struct RgbCctTransmitter<T: Transceiver> {
    radio: MiLightRadio<T>,
    key: u8,
}

impl<T: Transceiver> RgbCctTransmitter<T> {
    /// Configure `transceiver` for RGB+CCT traffic and wrap it.
    pub fn new(transceiver: T) -> Result<Self> {
        Ok(Self {
            radio: MiLightRadio::new(transceiver, RadioConfig::RGB_CCT)?,
            key: 0x00,
        })
    }

    /// Set how many times each command is repeated.
    #[must_use]
    pub fn with_repeats(mut self, repeats: usize) -> Self {
        self.radio = self.radio.with_repeats(repeats);
        self
    }

    /// Set the pause between repeats.
    #[must_use]
    pub fn with_gap(mut self, gap: Duration) -> Self {
        self.radio = self.radio.with_gap(gap);
        self
    }

    /// The obfuscation key byte. There is no benefit to varying it.
    #[must_use]
    pub const fn with_key(mut self, key: u8) -> Self {
        self.key = key;
        self
    }

    /// Send one intent to one lamp, using the sequence number given.
    ///
    /// The caller owns the sequence: it must stay fixed across the repeats of a
    /// single command (which this handles) and differ between commands (which it
    /// does not — see `pilight-db`'s `take_sequence`).
    pub fn send(&mut self, address: LampAddress, sequence: u8, intent: RgbCctIntent) -> Result<()> {
        let (command, argument) = intent.encode(address.group)?;

        self.send_raw(address, sequence, command, argument)
    }

    /// Send explicit command and argument bytes.
    ///
    /// Escape hatch for the parts of the protocol [`RgbCctIntent`] does not model.
    pub fn send_raw(
        &mut self,
        address: LampAddress,
        sequence: u8,
        command: u8,
        argument: u8,
    ) -> Result<()> {
        let packet = V2Packet::new(
            self.key,
            PROTOCOL_ID_RGB_CCT,
            address.device_id,
            command,
            argument,
            sequence,
            address.group.get(),
        );

        self.radio.send(&packet.to_encoded())
    }

    /// The radio underneath, for callers that need its configuration.
    #[must_use]
    pub const fn radio(&self) -> &MiLightRadio<T> {
        &self.radio
    }
}
