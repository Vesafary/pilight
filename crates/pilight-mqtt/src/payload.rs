//! Home Assistant's JSON light payloads.
//!
//! The `json` schema of HA's MQTT light: one object carrying state, brightness,
//! colour mode and colour. See
//! <https://www.home-assistant.io/integrations/light.mqtt/>.

use crate::units::{
    KelvinRange, brightness_to_percent, percent_to_brightness, round_hue, round_percent,
};
use pilight_db::domain::{BulbMode, LampState};
use pilight_proto::RgbCctIntent;
use pilight_service::StateChange;
use serde::{Deserialize, Serialize};

/// Number of scenes an RGB+CCT bulb offers, exposed to HA as effects.
pub const NUM_SCENES: u8 = 9;

/// The effect name for a scene number.
#[must_use]
pub fn scene_effect_name(scene: u8) -> String {
    format!("scene_{scene}")
}

/// Every effect name, for the discovery payload's `effect_list`.
#[must_use]
pub fn effect_list() -> Vec<String> {
    (0..NUM_SCENES).map(scene_effect_name).collect()
}

/// Parse an effect name back into a scene number.
#[must_use]
pub fn scene_from_effect(effect: &str) -> Option<u8> {
    effect
        .strip_prefix("scene_")?
        .parse::<u8>()
        .ok()
        .filter(|scene| *scene < NUM_SCENES)
}

/// The colour part of a JSON light payload.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ColorPayload {
    /// Hue, 0–360.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
    /// Saturation, 0–100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s: Option<f64>,
}

/// A state or command message.
///
/// Home Assistant sends and expects the same shape, so one type serves both.
/// Everything is optional: a command carries only what changed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LightPayload {
    /// `"ON"` or `"OFF"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Brightness on HA's 0–255 scale.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brightness: Option<u8>,
    /// Which colour mode is active. Only meaningful on state messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_mode: Option<String>,
    /// Colour temperature in Kelvin, because discovery sets `color_temp_kelvin`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_temp: Option<u16>,
    /// Hue and saturation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorPayload>,
    /// The running scene, as an effect name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

impl LightPayload {
    /// Whether the payload asks for the light to be on.
    #[must_use]
    pub fn wants_on(&self) -> Option<bool> {
        match self.state.as_deref()?.to_ascii_uppercase().as_str() {
            "ON" => Some(true),
            "OFF" => Some(false),
            _ => None,
        }
    }

    /// Render a lamp's stored state as a payload for Home Assistant.
    #[must_use]
    pub fn from_state(state: &LampState, kelvin: KelvinRange) -> Self {
        // HA insists color_mode names a mode listed in supported_color_modes, so
        // night and scene — which are not colour modes — report as the mode whose
        // values they last used.
        let color_mode = match state.bulb_mode {
            BulbMode::Color | BulbMode::Scene => "hs",
            BulbMode::White | BulbMode::Night => "color_temp",
        };

        Self {
            state: Some(if state.power { "ON" } else { "OFF" }.to_owned()),
            brightness: state.brightness.map(percent_to_brightness),
            color_mode: Some(color_mode.to_owned()),
            color_temp: state.kelvin.map(|k| kelvin.percent_to_kelvin(k)),
            color: match (state.hue, state.saturation) {
                (None, None) => None,
                (hue, saturation) => Some(ColorPayload {
                    h: hue.map(f64::from),
                    s: saturation.map(f64::from),
                }),
            },
            effect: match state.bulb_mode {
                BulbMode::Scene => state.scene.map(scene_effect_name),
                _ => None,
            },
        }
    }

    /// Turn a command payload into a partial state change.
    ///
    /// The floats Home Assistant sends are rounded here; the *ordering* of the
    /// resulting packets is [`StateChange`]'s job, so that the HTTP API and this
    /// bridge cannot drift apart on a question the bulbs care about.
    #[must_use]
    pub fn to_change(&self, kelvin: KelvinRange) -> StateChange {
        if self.wants_on() == Some(false) {
            return StateChange {
                power: Some(false),
                ..StateChange::default()
            };
        }

        let color = self.color.unwrap_or_default();

        StateChange {
            // Only pass on an explicit ON. A bare brightness change on a lamp HA
            // already believes is on should not cost an extra packet of airtime.
            power: self.wants_on().filter(|on| *on).map(|_| true),
            night_mode: false,
            brightness: self.brightness.map(brightness_to_percent),
            hue: color.h.map(round_hue),
            saturation: color.s.map(round_percent),
            kelvin: self.color_temp.map(|t| kelvin.kelvin_to_percent(t)),
            scene: self.effect.as_deref().and_then(scene_from_effect),
        }
    }

    /// Expand a command payload into the intents needed to satisfy it.
    #[must_use]
    pub fn to_intents(&self, kelvin: KelvinRange) -> Vec<RgbCctIntent> {
        self.to_change(kelvin).to_intents()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn state() -> LampState {
        LampState {
            lamp_id: Uuid::nil(),
            power: true,
            bulb_mode: BulbMode::Color,
            brightness: Some(60),
            hue: Some(200),
            saturation: Some(80),
            kelvin: Some(50),
            scene: None,
            next_sequence: 0,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn effects_round_trip() {
        for scene in 0..NUM_SCENES {
            assert_eq!(scene_from_effect(&scene_effect_name(scene)), Some(scene));
        }
        assert_eq!(effect_list().len(), NUM_SCENES as usize);
    }

    #[test]
    fn unknown_effects_are_rejected() {
        assert_eq!(scene_from_effect("colorloop"), None);
        assert_eq!(scene_from_effect("scene_99"), None, "out of range");
        assert_eq!(scene_from_effect("scene_"), None);
    }

    #[test]
    fn state_renders_with_a_colour_mode_home_assistant_accepts() {
        let payload = LightPayload::from_state(&state(), KelvinRange::default());

        assert_eq!(payload.state.as_deref(), Some("ON"));
        assert_eq!(payload.color_mode.as_deref(), Some("hs"));
        assert_eq!(payload.brightness, Some(153), "60% of 255");
        assert_eq!(payload.color.unwrap().h, Some(200.0));
    }

    #[test]
    fn night_and_scene_modes_report_a_supported_colour_mode() {
        // HA rejects a color_mode that is not in supported_color_modes, so these
        // two must map onto one of the real ones rather than their own name.
        for (mode, expected) in [
            (BulbMode::Night, "color_temp"),
            (BulbMode::Scene, "hs"),
            (BulbMode::White, "color_temp"),
            (BulbMode::Color, "hs"),
        ] {
            let payload = LightPayload::from_state(
                &LampState {
                    bulb_mode: mode,
                    ..state()
                },
                KelvinRange::default(),
            );
            assert_eq!(payload.color_mode.as_deref(), Some(expected), "{mode}");
        }
    }

    #[test]
    fn a_scene_is_reported_as_an_effect() {
        let payload = LightPayload::from_state(
            &LampState {
                bulb_mode: BulbMode::Scene,
                scene: Some(3),
                ..state()
            },
            KelvinRange::default(),
        );
        assert_eq!(payload.effect.as_deref(), Some("scene_3"));
    }

    #[test]
    fn a_lamp_we_know_nothing_about_omits_colour() {
        let payload = LightPayload::from_state(
            &LampState {
                hue: None,
                saturation: None,
                kelvin: None,
                brightness: None,
                ..state()
            },
            KelvinRange::default(),
        );

        assert_eq!(payload.color, None);
        assert_eq!(payload.color_temp, None);
        assert_eq!(payload.brightness, None);
    }

    #[test]
    fn turning_off_ignores_everything_else_in_the_message() {
        let payload = LightPayload {
            state: Some("OFF".into()),
            brightness: Some(255),
            color: Some(ColorPayload {
                h: Some(200.0),
                s: Some(80.0),
            }),
            ..Default::default()
        };

        assert_eq!(
            payload.to_intents(KelvinRange::default()),
            vec![RgbCctIntent::Power(false)]
        );
    }

    #[test]
    fn a_combined_command_is_ordered_so_later_intents_do_not_undo_earlier_ones() {
        let payload = LightPayload {
            state: Some("ON".into()),
            brightness: Some(255),
            color_temp: Some(4000),
            color: Some(ColorPayload {
                h: Some(200.0),
                s: Some(80.0),
            }),
            ..Default::default()
        };

        let intents = payload.to_intents(KelvinRange::default());

        // Kelvin forces white mode and saturation only applies in colour mode, so
        // temperature must precede hue, hue must precede saturation, and
        // brightness — which survives a mode switch — comes last.
        let position = |needle: &RgbCctIntent| {
            intents
                .iter()
                .position(|i| std::mem::discriminant(i) == std::mem::discriminant(needle))
        };
        let kelvin = position(&RgbCctIntent::Kelvin(0)).unwrap();
        let hue = position(&RgbCctIntent::Hue(0)).unwrap();
        let saturation = position(&RgbCctIntent::Saturation(0)).unwrap();
        let brightness = position(&RgbCctIntent::Brightness(0)).unwrap();

        assert!(kelvin < hue, "temperature must not undo the hue");
        assert!(
            hue < saturation,
            "saturation only applies once in colour mode"
        );
        assert!(saturation < brightness);
        assert_eq!(intents[0], RgbCctIntent::Power(true));
    }

    #[test]
    fn a_bare_brightness_command_does_not_invent_a_power_intent() {
        // HA sends brightness alone when dragging a slider on a lamp it already
        // believes is on; an extra ON packet would be wasted airtime.
        let payload = LightPayload {
            brightness: Some(128),
            ..Default::default()
        };

        assert_eq!(
            payload.to_intents(KelvinRange::default()),
            vec![RgbCctIntent::Brightness(50)]
        );
    }

    #[test]
    fn an_empty_command_asks_for_nothing() {
        assert!(
            LightPayload::default()
                .to_intents(KelvinRange::default())
                .is_empty()
        );
    }

    #[test]
    fn state_is_parsed_case_insensitively() {
        let on = LightPayload {
            state: Some("on".into()),
            ..Default::default()
        };
        assert_eq!(on.wants_on(), Some(true));

        let nonsense = LightPayload {
            state: Some("maybe".into()),
            ..Default::default()
        };
        assert_eq!(nonsense.wants_on(), None);
    }

    #[test]
    fn payloads_round_trip_through_json() {
        let payload = LightPayload::from_state(&state(), KelvinRange::default());
        let json = serde_json::to_string(&payload).unwrap();
        let back: LightPayload = serde_json::from_str(&json).unwrap();

        assert_eq!(back, payload);
        assert!(
            !json.contains("null"),
            "absent fields are omitted, not null: {json}"
        );
    }

    #[test]
    fn a_real_home_assistant_command_parses() {
        // The shape HA actually publishes for the json schema.
        let json = r#"{"state":"ON","brightness":180,"color":{"h":344.0,"s":29.412}}"#;
        let payload: LightPayload = serde_json::from_str(json).unwrap();

        let intents = payload.to_intents(KelvinRange::default());
        assert_eq!(intents[0], RgbCctIntent::Power(true));
        assert!(intents.contains(&RgbCctIntent::Hue(344)));
        assert!(intents.contains(&RgbCctIntent::Saturation(29)));
        assert!(intents.contains(&RgbCctIntent::Brightness(71)));
    }
}
