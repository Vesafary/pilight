//! Radio layer: framing, channel hopping and repetition on top of a transceiver.

mod config;
pub use config::*;

mod null;
pub use null::NullTransceiver;

#[cfg(feature = "nrf24")]
pub mod diagnostics;
#[cfg(feature = "nrf24")]
mod nrf24;
#[cfg(feature = "nrf24")]
pub use nrf24::Nrf24Transceiver;

use crate::error::Result;
use crate::framing::Frame;
use std::thread::sleep;
use std::time::Duration;

/// How many times a command's frame is repeated by default.
///
/// The bulbs never acknowledge anything and a single burst is routinely lost, so
/// real remotes repeat aggressively. See `docs/protocol.md` §2.4.
pub const DEFAULT_REPEATS: usize = 50;

/// Default pause between repeats.
pub const DEFAULT_GAP: Duration = Duration::from_millis(5);

/// A radio that can be tuned and told to transmit a raw payload.
///
/// Implemented by [`Nrf24Transceiver`] for real hardware, and by test doubles.
pub trait Transceiver {
    /// Apply a family's syncword and payload length. Called once, before transmitting.
    fn configure(&mut self, config: &RadioConfig) -> Result<()>;

    /// Transmit one payload on one channel, blocking until it has left the FIFO.
    fn transmit(&mut self, channel: Channel, payload: &[u8]) -> Result<()>;
}

/// Wraps a [`Transceiver`] with everything above the raw payload: PL1167 framing,
/// hopping across the family's three channels, and burst repetition.
#[derive(Debug)]
pub struct MiLightRadio<T> {
    transceiver: T,
    config: RadioConfig,
    repeats: usize,
    gap: Duration,
}

impl<T: Transceiver> MiLightRadio<T> {
    /// Configure `transceiver` for `config` and wrap it.
    pub fn new(mut transceiver: T, config: RadioConfig) -> Result<Self> {
        transceiver.configure(&config)?;

        Ok(Self {
            transceiver,
            config,
            repeats: DEFAULT_REPEATS,
            gap: DEFAULT_GAP,
        })
    }

    /// Set how many times each command is repeated.
    #[must_use]
    pub fn with_repeats(self, repeats: usize) -> Self {
        Self { repeats, ..self }
    }

    /// Set the pause between repeats.
    #[must_use]
    pub fn with_gap(self, gap: Duration) -> Self {
        Self { gap, ..self }
    }

    /// The family this radio is configured for.
    #[must_use]
    pub fn config(&self) -> &RadioConfig {
        &self.config
    }

    /// Frame `packet`, then send it on every channel, `repeats` times over.
    ///
    /// The same bytes go out every time — in particular the sequence number must
    /// not change across a burst, or the bulb treats the repeats as fresh commands.
    pub fn send(&mut self, packet: &[u8]) -> Result<()> {
        let frame = Frame::build(packet)?;

        for repeat in 0..self.repeats {
            for channel in self.config.channels {
                self.transceiver.transmit(channel, frame.as_slice())?;
            }

            let is_last = repeat + 1 == self.repeats;
            if !is_last && !self.gap.is_zero() {
                sleep(self.gap);
            }
        }

        Ok(())
    }
}
