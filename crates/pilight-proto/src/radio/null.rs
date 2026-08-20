//! A transceiver that transmits nothing.

use super::Transceiver;
use super::config::{Channel, RadioConfig};
use crate::error::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A [`Transceiver`] that accepts every payload and drops it on the floor.
///
/// For running the stack on a machine with no radio: the database, the MQTT
/// bridge and the HTTP API all behave normally, and nothing reaches the air.
///
/// **It always succeeds.** A caller cannot tell it apart from a working radio by
/// return value alone, so anything user-facing should say plainly when it is in
/// use. It keeps a count of what it swallowed so tests and health endpoints can
/// tell the difference.
#[derive(Debug, Clone, Default)]
pub struct NullTransceiver {
    sent: Arc<AtomicUsize>,
}

impl NullTransceiver {
    /// Build one.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many payloads have been discarded.
    #[must_use]
    pub fn discarded(&self) -> usize {
        self.sent.load(Ordering::Relaxed)
    }
}

impl Transceiver for NullTransceiver {
    fn configure(&mut self, _config: &RadioConfig) -> Result<()> {
        Ok(())
    }

    fn transmit(&mut self, _channel: Channel, _payload: &[u8]) -> Result<()> {
        self.sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_counts_what_it_swallows() {
        let mut radio = NullTransceiver::new();
        assert_eq!(radio.discarded(), 0);

        radio.transmit(Channel::new(8), &[0; 12]).unwrap();
        radio.transmit(Channel::new(39), &[0; 12]).unwrap();

        assert_eq!(radio.discarded(), 2);
    }

    #[test]
    fn clones_share_one_counter() {
        // The service holds one behind an Arc; a health check holding a clone must
        // see the same numbers.
        let radio = NullTransceiver::new();
        let mut clone = radio.clone();

        clone.transmit(Channel::new(8), &[0; 12]).unwrap();

        assert_eq!(radio.discarded(), 1);
    }
}
