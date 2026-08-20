//! Notifications that something about a lamp changed.
//!
//! The MQTT bridge has to react to changes it did not make: a lamp registered
//! through the HTTP API needs announcing to Home Assistant, a deleted one needs
//! retracting, a state change needs republishing. A broadcast channel keeps that
//! coupling one-way — the API knows nothing about MQTT.

use uuid::Uuid;

/// How many events are buffered before slow subscribers start missing them.
///
/// A subscriber that lags is told so and can recover by re-reading everything,
/// which is exactly what the bridge does.
pub const EVENT_BUFFER: usize = 64;

/// Something changed about a lamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LampEvent {
    /// A lamp was registered. Announce it.
    Registered(Uuid),
    /// A lamp's name or room changed. Re-announce it.
    Updated(Uuid),
    /// A lamp was deleted. Retract it.
    Removed(Uuid),
    /// A lamp's state changed. Republish it.
    StateChanged(Uuid),
}

impl LampEvent {
    /// Which lamp the event concerns.
    #[must_use]
    pub const fn lamp_id(self) -> Uuid {
        match self {
            Self::Registered(id)
            | Self::Updated(id)
            | Self::Removed(id)
            | Self::StateChanged(id) => id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_names_its_lamp() {
        let id = Uuid::new_v4();

        for event in [
            LampEvent::Registered(id),
            LampEvent::Updated(id),
            LampEvent::Removed(id),
            LampEvent::StateChanged(id),
        ] {
            assert_eq!(event.lamp_id(), id);
        }
    }
}
