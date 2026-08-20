//! A partial state change, and the order its intents must be sent in.
//!
//! Both the HTTP API and the MQTT bridge accept "on, half brightness, blue" in one
//! request. The protocol cannot say that in one packet, and it is not indifferent
//! to order — so the expansion lives here, once, rather than in each of them.

use pilight_proto::RgbCctIntent;

/// A partial change to a lamp. `None` leaves a property alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateChange {
    /// Switch on or off.
    pub power: Option<bool>,
    /// Drop into night mode. Overrides `power`.
    pub night_mode: bool,
    /// Brightness percentage.
    pub brightness: Option<u8>,
    /// Hue in degrees.
    pub hue: Option<u16>,
    /// Saturation percentage.
    pub saturation: Option<u8>,
    /// Colour temperature percentage: 0 coolest, 100 warmest.
    pub kelvin: Option<u8>,
    /// Scene index.
    pub scene: Option<u8>,
}

impl StateChange {
    /// Whether this change asks for anything.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.power.is_none()
            && !self.night_mode
            && self.brightness.is_none()
            && self.hue.is_none()
            && self.saturation.is_none()
            && self.kelvin.is_none()
            && self.scene.is_none()
    }

    /// Expand into intents, in the order they must be transmitted.
    ///
    /// The order is not the order of the fields, and it matters:
    ///
    /// * **Off short-circuits.** Nothing else in the request is worth sending.
    /// * **Night mode short-circuits** for the same reason.
    /// * **Scene, then temperature, then hue, then saturation, then brightness.**
    ///   A Kelvin command drags the bulb out of colour mode, so it must precede
    ///   the hue; saturation only applies once the bulb is *in* colour mode, so it
    ///   must follow it; brightness survives a mode switch, so it goes last.
    ///
    /// Getting this order wrong is why colours appear not to stick.
    #[must_use]
    pub fn to_intents(&self) -> Vec<RgbCctIntent> {
        if self.night_mode {
            return vec![RgbCctIntent::NightMode];
        }
        if self.power == Some(false) {
            return vec![RgbCctIntent::Power(false)];
        }

        let mut intents = Vec::new();

        // Only say "on" when it was asked for. A bare brightness change on a lamp
        // that is already on should not cost an extra packet of airtime.
        if self.power == Some(true) {
            intents.push(RgbCctIntent::Power(true));
        }
        if let Some(scene) = self.scene {
            intents.push(RgbCctIntent::Scene(scene));
        }
        if let Some(kelvin) = self.kelvin {
            intents.push(RgbCctIntent::Kelvin(kelvin));
        }
        if let Some(hue) = self.hue {
            intents.push(RgbCctIntent::Hue(hue));
        }
        if let Some(saturation) = self.saturation {
            intents.push(RgbCctIntent::Saturation(saturation));
        }
        if let Some(brightness) = self.brightness {
            intents.push(RgbCctIntent::Brightness(brightness));
        }

        intents
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(intents: &[RgbCctIntent], needle: RgbCctIntent) -> Option<usize> {
        intents
            .iter()
            .position(|i| std::mem::discriminant(i) == std::mem::discriminant(&needle))
    }

    #[test]
    fn an_empty_change_asks_for_nothing() {
        assert!(StateChange::default().is_empty());
        assert!(StateChange::default().to_intents().is_empty());
    }

    #[test]
    fn turning_off_ignores_everything_else() {
        let change = StateChange {
            power: Some(false),
            brightness: Some(100),
            hue: Some(200),
            ..Default::default()
        };

        assert_eq!(change.to_intents(), vec![RgbCctIntent::Power(false)]);
    }

    #[test]
    fn night_mode_wins_over_everything_including_power() {
        let change = StateChange {
            night_mode: true,
            power: Some(true),
            brightness: Some(100),
            ..Default::default()
        };

        assert_eq!(change.to_intents(), vec![RgbCctIntent::NightMode]);
    }

    #[test]
    fn intents_are_ordered_so_later_ones_do_not_undo_earlier_ones() {
        let change = StateChange {
            power: Some(true),
            brightness: Some(50),
            hue: Some(200),
            saturation: Some(80),
            kelvin: Some(40),
            scene: Some(3),
            night_mode: false,
        };
        let intents = change.to_intents();

        let scene = position(&intents, RgbCctIntent::Scene(0)).unwrap();
        let kelvin = position(&intents, RgbCctIntent::Kelvin(0)).unwrap();
        let hue = position(&intents, RgbCctIntent::Hue(0)).unwrap();
        let saturation = position(&intents, RgbCctIntent::Saturation(0)).unwrap();
        let brightness = position(&intents, RgbCctIntent::Brightness(0)).unwrap();

        assert_eq!(intents[0], RgbCctIntent::Power(true));
        assert!(
            scene < kelvin,
            "a scene must not survive the temperature change"
        );
        assert!(
            kelvin < hue,
            "temperature forces white mode, so it precedes hue"
        );
        assert!(
            hue < saturation,
            "saturation only applies once in colour mode"
        );
        assert!(saturation < brightness, "brightness survives a mode switch");
    }

    #[test]
    fn a_bare_brightness_change_does_not_invent_a_power_intent() {
        let change = StateChange {
            brightness: Some(60),
            ..Default::default()
        };

        assert_eq!(change.to_intents(), vec![RgbCctIntent::Brightness(60)]);
    }

    #[test]
    fn turning_on_with_nothing_else_sends_exactly_one_intent() {
        let change = StateChange {
            power: Some(true),
            ..Default::default()
        };

        assert_eq!(change.to_intents(), vec![RgbCctIntent::Power(true)]);
    }
}
