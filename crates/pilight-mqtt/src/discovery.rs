//! Home Assistant MQTT discovery.
//!
//! One retained config message per lamp on
//! `homeassistant/light/<object_id>/config` makes it appear in HA with no YAML.
//! See <https://www.home-assistant.io/integrations/mqtt/#mqtt-discovery>.

use crate::topics::{PAYLOAD_OFFLINE, PAYLOAD_ONLINE, Topics, object_id};
use crate::units::{HA_BRIGHTNESS_SCALE, KelvinRange};
use pilight_db::domain::Lamp;
use serde::{Deserialize, Serialize};

/// What HA shows as the integration that created these entities.
pub const ORIGIN_NAME: &str = "pilight";

/// Shown as the manufacturer of every lamp.
pub const MANUFACTURER: &str = "MiLight";

/// Identifier of the bridge device that every lamp hangs off.
pub const BRIDGE_IDENTIFIER: &str = "pilight_bridge";

/// Where the `device` block points for support.
pub const SUPPORT_URL: &str = "https://github.com/vesafary/pilight";

/// The `origin` block: which application published this discovery message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Origin {
    /// Application name.
    pub name: String,
    /// Application version.
    pub sw_version: String,
    /// Where to go for help.
    pub support_url: String,
}

impl Default for Origin {
    fn default() -> Self {
        Self {
            name: ORIGIN_NAME.to_owned(),
            sw_version: env!("CARGO_PKG_VERSION").to_owned(),
            support_url: SUPPORT_URL.to_owned(),
        }
    }
}

/// The `device` block: how HA groups entities and draws the device page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Device {
    /// Stable identifiers for this device.
    pub identifiers: Vec<String>,
    /// Device name.
    pub name: String,
    /// Manufacturer.
    pub manufacturer: String,
    /// Model.
    pub model: String,
    /// Which device bridges this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_device: Option<String>,
    /// Where the device sits, prefilled for the user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_area: Option<String>,
}

/// A retained discovery config for one lamp.
///
/// The field names and shape are Home Assistant's, not ours — including the run
/// of booleans, which are separate capability flags in its schema.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LightDiscovery {
    /// Always `"json"`.
    pub schema: String,
    /// Entity name. `None` makes HA use the device name.
    pub name: Option<String>,
    /// Stable id so HA can track the entity across restarts and renames.
    pub unique_id: String,
    /// Suggested entity id suffix, e.g. `living_room_west` becomes
    /// `light.living_room_west`.
    ///
    /// Without this, Home Assistant builds the entity id from the device and
    /// entity names, which yields things like `light.living_room_living_room_west`
    /// when the room already appears in the name — and it is fixed at creation, so
    /// renaming the lamp afterwards does not correct it.
    pub object_id: String,
    /// Where state is published.
    pub state_topic: String,
    /// Where commands are accepted.
    pub command_topic: String,
    /// Where our online/offline status is published.
    pub availability_topic: String,
    /// Payload meaning "the bridge is up".
    pub payload_available: String,
    /// Payload meaning "the bridge is down".
    pub payload_not_available: String,
    /// Brightness is supported.
    pub brightness: bool,
    /// Brightness scale, HA's default but stated for clarity.
    pub brightness_scale: u8,
    /// Which colour modes the bulb has.
    pub supported_color_modes: Vec<String>,
    /// Colour temperature is expressed in Kelvin, not mireds.
    pub color_temp_kelvin: bool,
    /// Warmest colour the bulb produces.
    pub min_kelvin: u16,
    /// Coolest colour the bulb produces.
    pub max_kelvin: u16,
    /// Scenes are exposed as effects.
    pub effect: bool,
    /// The scene names.
    pub effect_list: Vec<String>,
    /// The bulbs cannot fade, so do not offer a transition control.
    pub transition: bool,
    /// Nor can they flash.
    pub flash: bool,
    /// Whether HA should assume a command took effect.
    pub optimistic: bool,
    /// The device this entity belongs to.
    pub device: Device,
    /// The application that published this.
    pub origin: Origin,
}

/// A Home Assistant entity id suffix for a lamp: `room_name`, slugified.
///
/// Home Assistant accepts `[a-z0-9_]`; anything else becomes an underscore, and
/// runs are collapsed so "Living room (west)" does not become
/// `living_room__west_`.
///
/// Without this, HA builds the entity id from the device and entity names, which
/// yields `light.living_room_living_room_west` when the room already appears in
/// the name — and it is fixed at creation, so renaming the lamp later does not
/// correct it.
#[must_use]
pub fn entity_object_id(lamp: &Lamp) -> String {
    let name = slugify(&lamp.name);

    match &lamp.room {
        // Don't repeat the room when the name already carries it: a lamp called
        // "Hallway" in room "Hallway" should be `hallway`, not `hallway_hallway`.
        Some(room) => {
            let room = slugify(room);
            if room.is_empty() || name.contains(&room) {
                name
            } else {
                format!("{room}_{name}")
            }
        }
        None => name,
    }
}

/// Reduce a label to the `[a-z0-9_]` Home Assistant accepts, collapsing runs.
fn slugify(value: &str) -> String {
    let mut out = String::with_capacity(value.len());

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }

    out.trim_matches('_').to_owned()
}

impl LightDiscovery {
    /// Build the discovery config for a lamp.
    #[must_use]
    pub fn for_lamp(lamp: &Lamp, topics: &Topics, kelvin: KelvinRange) -> Self {
        Self {
            schema: "json".to_owned(),
            // The entity takes its name from the device, which avoids HA showing
            // "Couch Couch" on the device page.
            name: None,
            unique_id: object_id(lamp.id),
            object_id: entity_object_id(lamp),
            state_topic: topics.state(lamp.id),
            command_topic: topics.command(lamp.id),
            availability_topic: topics.availability(),
            payload_available: PAYLOAD_ONLINE.to_owned(),
            payload_not_available: PAYLOAD_OFFLINE.to_owned(),
            brightness: true,
            brightness_scale: HA_BRIGHTNESS_SCALE,
            supported_color_modes: vec!["color_temp".to_owned(), "hs".to_owned()],
            color_temp_kelvin: true,
            min_kelvin: kelvin.min,
            max_kelvin: kelvin.max,
            effect: true,
            effect_list: crate::payload::effect_list(),
            transition: false,
            flash: false,
            // The bulbs never acknowledge anything, so we publish state ourselves
            // as soon as a command is sent; HA must not also guess.
            optimistic: false,
            device: Device {
                identifiers: vec![object_id(lamp.id)],
                name: lamp.name.clone(),
                manufacturer: MANUFACTURER.to_owned(),
                model: lamp.remote_type.display_name().to_owned(),
                via_device: Some(BRIDGE_IDENTIFIER.to_owned()),
                suggested_area: lamp.room.clone(),
            },
            origin: Origin::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use pilight_proto::RemoteType;
    use uuid::Uuid;

    fn lamp() -> Lamp {
        Lamp {
            id: Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap(),
            name: "Couch".into(),
            room: Some("Living room".into()),
            remote_type: RemoteType::RgbCct,
            device_id: 0xBEEF,
            group: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn entity_ids_do_not_repeat_the_room() {
        // Regression: "Living room" + "Living room (west)" produced
        // light.living_room_living_room_west in Home Assistant.
        assert_eq!(entity_object_id(&lamp()), "living_room_couch");

        let west = Lamp {
            name: "West".into(),
            ..lamp()
        };
        assert_eq!(entity_object_id(&west), "living_room_west");

        let roomless = Lamp {
            name: "Hallway".into(),
            room: None,
            ..lamp()
        };
        assert_eq!(entity_object_id(&roomless), "hallway");
    }

    #[test]
    fn entity_ids_use_only_characters_home_assistant_accepts() {
        let awkward = Lamp {
            name: "  (west) -- #2!  ".into(),
            room: Some("Living  room".into()),
            ..lamp()
        };
        let id = entity_object_id(&awkward);

        assert_eq!(id, "living_room_west_2");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        );
    }

    #[test]
    fn the_config_names_the_json_schema_and_our_topics() {
        let topics = Topics::default();
        let config = LightDiscovery::for_lamp(&lamp(), &topics, KelvinRange::default());

        assert_eq!(config.schema, "json");
        assert_eq!(config.state_topic, topics.state(lamp().id));
        assert_eq!(config.command_topic, topics.command(lamp().id));
        assert_eq!(config.availability_topic, topics.availability());
    }

    #[test]
    fn the_colour_mode_reported_in_state_is_one_we_declared() {
        // HA drops a state message whose color_mode is not in this list, which is
        // the kind of bug that shows up as "the light never updates".
        let config = LightDiscovery::for_lamp(&lamp(), &Topics::default(), KelvinRange::default());

        for mode in ["hs", "color_temp"] {
            assert!(
                config.supported_color_modes.contains(&mode.to_owned()),
                "state payloads can report {mode}"
            );
        }
    }

    #[test]
    fn kelvin_bounds_match_the_range_used_for_conversion() {
        let range = KelvinRange::new(3000, 6000);
        let config = LightDiscovery::for_lamp(&lamp(), &Topics::default(), range);

        assert!(config.color_temp_kelvin, "mireds are deprecated");
        assert_eq!(config.min_kelvin, 3000);
        assert_eq!(config.max_kelvin, 6000);
    }

    #[test]
    fn capabilities_the_bulbs_lack_are_switched_off() {
        let config = LightDiscovery::for_lamp(&lamp(), &Topics::default(), KelvinRange::default());

        assert!(!config.transition, "the bulbs cannot fade");
        assert!(!config.flash);
        assert!(
            !config.optimistic,
            "we publish real state, HA must not guess"
        );
    }

    #[test]
    fn the_room_becomes_a_suggested_area() {
        let config = LightDiscovery::for_lamp(&lamp(), &Topics::default(), KelvinRange::default());
        assert_eq!(config.device.suggested_area.as_deref(), Some("Living room"));

        let roomless = Lamp {
            room: None,
            ..lamp()
        };
        let config =
            LightDiscovery::for_lamp(&roomless, &Topics::default(), KelvinRange::default());
        assert_eq!(config.device.suggested_area, None);
    }

    #[test]
    fn unique_ids_are_stable_and_survive_a_rename() {
        let renamed = Lamp {
            name: "Reading lamp".into(),
            ..lamp()
        };

        let a = LightDiscovery::for_lamp(&lamp(), &Topics::default(), KelvinRange::default());
        let b = LightDiscovery::for_lamp(&renamed, &Topics::default(), KelvinRange::default());

        assert_eq!(
            a.unique_id, b.unique_id,
            "HA would create a duplicate entity"
        );
    }

    #[test]
    fn the_payload_serialises_without_nulls() {
        let config = LightDiscovery::for_lamp(&lamp(), &Topics::default(), KelvinRange::default());
        let json = serde_json::to_string(&config).unwrap();

        // `name: null` is meaningful to HA (use the device name), so it stays;
        // nothing else should be null.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let nulls: Vec<&String> = value
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, v)| v.is_null())
            .map(|(k, _)| k)
            .collect();

        assert_eq!(nulls, vec!["name"], "unexpected nulls in {json}");
    }
}
