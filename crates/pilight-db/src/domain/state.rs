//! What we believe a lamp is currently doing.

use super::bulb_mode::BulbMode;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// The last known state of a lamp.
///
/// **This is a belief, not a reading.** MiLight bulbs never acknowledge anything
/// and cannot be queried, so every field here reflects the last command we sent.
/// It goes stale the moment someone uses a physical remote.
///
/// The optional fields are `None` until the corresponding command has been sent at
/// least once — a freshly paired bulb has a colour, but we do not know what it is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LampState {
    /// Which lamp this describes.
    pub lamp_id: Uuid,
    /// Whether the lamp was last told to be on.
    pub power: bool,
    /// Which mode it was last driven into.
    pub bulb_mode: BulbMode,
    /// Brightness percentage, `0..=100`.
    pub brightness: Option<u8>,
    /// Hue in degrees, `0..=359`.
    pub hue: Option<u16>,
    /// Saturation percentage, `0..=100`.
    pub saturation: Option<u8>,
    /// Colour temperature percentage: 0 is coolest, 100 warmest.
    pub kelvin: Option<u8>,
    /// Active scene, `0..=8`.
    pub scene: Option<u8>,
    /// The V2 sequence byte to use for the next command.
    ///
    /// Persisted so a restart does not replay numbers the bulb has already seen.
    pub next_sequence: u8,
    /// When this belief was last revised.
    pub updated_at: DateTime<Utc>,
}

/// A partial update to a lamp's state. `None` leaves a field alone.
///
/// Note the difference from [`LampState`]: here `Some(None)` explicitly clears a
/// field, while `None` means "don't touch".
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct LampStateUpdate {
    /// New power state.
    pub power: Option<bool>,
    /// New mode.
    pub bulb_mode: Option<BulbMode>,
    /// New brightness.
    pub brightness: Option<u8>,
    /// New hue.
    pub hue: Option<u16>,
    /// New saturation.
    pub saturation: Option<u8>,
    /// New colour temperature.
    pub kelvin: Option<u8>,
    /// New scene.
    pub scene: Option<u8>,
}

impl LampStateUpdate {
    /// Whether this update would change anything.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.power.is_none()
            && self.bulb_mode.is_none()
            && self.brightness.is_none()
            && self.hue.is_none()
            && self.saturation.is_none()
            && self.kelvin.is_none()
            && self.scene.is_none()
    }

    /// Record that the lamp was switched on or off.
    #[must_use]
    pub const fn power(power: bool) -> Self {
        Self {
            power: Some(power),
            bulb_mode: None,
            brightness: None,
            hue: None,
            saturation: None,
            kelvin: None,
            scene: None,
        }
    }

    /// Record a hue change, which also drives the bulb into colour mode.
    #[must_use]
    pub fn hue(hue: u16) -> Self {
        Self {
            hue: Some(hue % 360),
            bulb_mode: Some(BulbMode::Color),
            ..Self::default()
        }
    }

    /// Record a colour-temperature change.
    ///
    /// Setting a temperature forces the bulb into white mode, discarding whatever
    /// hue or scene was running — so this records the mode change too.
    #[must_use]
    pub fn kelvin(kelvin: u8) -> Self {
        Self {
            kelvin: Some(kelvin.min(100)),
            bulb_mode: Some(BulbMode::White),
            ..Self::default()
        }
    }

    /// Record a scene selection.
    #[must_use]
    pub fn scene(scene: u8) -> Self {
        Self {
            scene: Some(scene),
            bulb_mode: Some(BulbMode::Scene),
            ..Self::default()
        }
    }

    /// Record a brightness change. Does not affect the mode.
    #[must_use]
    pub fn brightness(brightness: u8) -> Self {
        Self {
            brightness: Some(brightness.min(100)),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_update_is_empty() {
        assert!(LampStateUpdate::default().is_empty());
    }

    #[test]
    fn setting_a_hue_implies_colour_mode() {
        let update = LampStateUpdate::hue(200);
        assert_eq!(update.hue, Some(200));
        assert_eq!(update.bulb_mode, Some(BulbMode::Color));
        assert!(!update.is_empty());
    }

    #[test]
    fn hues_wrap_at_a_full_turn() {
        assert_eq!(LampStateUpdate::hue(360).hue, Some(0));
        assert_eq!(LampStateUpdate::hue(725).hue, Some(5));
    }

    #[test]
    fn setting_a_temperature_implies_white_mode() {
        // This mirrors the protocol: a Kelvin command drops the bulb out of colour.
        let update = LampStateUpdate::kelvin(80);
        assert_eq!(update.kelvin, Some(80));
        assert_eq!(update.bulb_mode, Some(BulbMode::White));
    }

    #[test]
    fn selecting_a_scene_implies_scene_mode() {
        let update = LampStateUpdate::scene(3);
        assert_eq!(update.scene, Some(3));
        assert_eq!(update.bulb_mode, Some(BulbMode::Scene));
    }

    #[test]
    fn brightness_leaves_the_mode_alone() {
        // Brightness applies in both white and colour mode, so it says nothing
        // about which one the bulb is in.
        let update = LampStateUpdate::brightness(60);
        assert_eq!(update.brightness, Some(60));
        assert_eq!(update.bulb_mode, None);
    }

    #[test]
    fn percentages_are_clamped_rather_than_wrapped() {
        assert_eq!(LampStateUpdate::brightness(200).brightness, Some(100));
        assert_eq!(LampStateUpdate::kelvin(200).kelvin, Some(100));
    }
}
