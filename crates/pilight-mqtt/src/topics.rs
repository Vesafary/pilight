//! Topic layout.
//!
//! ```text
//! homeassistant/light/<object_id>/config   discovery, retained
//! homeassistant/status                     HA's own birth/will — we listen
//! pilight/status                           our availability, retained + LWT
//! pilight/lamp/<uuid>/state                lamp state, retained
//! pilight/lamp/<uuid>/set                  commands, we subscribe
//! ```

use uuid::Uuid;

/// Default prefix for this bridge's own topics.
pub const DEFAULT_PREFIX: &str = "pilight";

/// Default prefix Home Assistant watches for discovery.
pub const DEFAULT_DISCOVERY_PREFIX: &str = "homeassistant";

/// Payload published when the bridge is connected.
pub const PAYLOAD_ONLINE: &str = "online";

/// Payload left as the will, and published on a clean shutdown.
pub const PAYLOAD_OFFLINE: &str = "offline";

/// Builds every topic the bridge uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topics {
    prefix: String,
    discovery_prefix: String,
}

impl Default for Topics {
    fn default() -> Self {
        Self::new(DEFAULT_PREFIX, DEFAULT_DISCOVERY_PREFIX)
    }
}

impl Topics {
    /// Build with explicit prefixes. Trailing slashes are trimmed.
    #[must_use]
    pub fn new(prefix: &str, discovery_prefix: &str) -> Self {
        Self {
            prefix: prefix.trim_matches('/').to_owned(),
            discovery_prefix: discovery_prefix.trim_matches('/').to_owned(),
        }
    }

    /// Our availability topic. Also used as the last will.
    #[must_use]
    pub fn availability(&self) -> String {
        format!("{}/status", self.prefix)
    }

    /// Where a lamp's state is published.
    #[must_use]
    pub fn state(&self, lamp_id: Uuid) -> String {
        format!("{}/lamp/{lamp_id}/state", self.prefix)
    }

    /// Where a lamp's commands arrive.
    #[must_use]
    pub fn command(&self, lamp_id: Uuid) -> String {
        format!("{}/lamp/{lamp_id}/set", self.prefix)
    }

    /// One subscription covering every lamp's commands.
    #[must_use]
    pub fn command_wildcard(&self) -> String {
        format!("{}/lamp/+/set", self.prefix)
    }

    /// Where a lamp's discovery config goes.
    #[must_use]
    pub fn discovery(&self, lamp_id: Uuid) -> String {
        format!(
            "{}/light/{}/config",
            self.discovery_prefix,
            object_id(lamp_id)
        )
    }

    /// Home Assistant's own birth and will topic.
    ///
    /// HA publishes `online` here when it starts. Discovery messages are retained,
    /// but a restarted HA that has forgotten an entity only recovers it if we
    /// re-announce — so the bridge subscribes and republishes on `online`.
    #[must_use]
    pub fn home_assistant_status(&self) -> String {
        format!("{}/status", self.discovery_prefix)
    }

    /// Recover a lamp id from a command topic.
    ///
    /// Returns `None` for anything that is not one of our command topics, which is
    /// how the bridge ignores traffic it did not ask for.
    #[must_use]
    pub fn lamp_id_from_command(&self, topic: &str) -> Option<Uuid> {
        let rest = topic.strip_prefix(&self.prefix)?.strip_prefix("/lamp/")?;
        let id = rest.strip_suffix("/set")?;

        Uuid::parse_str(id).ok()
    }
}

/// The discovery object id for a lamp.
///
/// Home Assistant restricts these to `[a-zA-Z0-9_-]`, so the UUID's hyphens are
/// fine but a prefix keeps them from colliding with another integration's ids.
#[must_use]
pub fn object_id(lamp_id: Uuid) -> String {
    format!("pilight_{}", lamp_id.simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> Uuid {
        Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap()
    }

    #[test]
    fn topics_have_the_documented_shape() {
        let topics = Topics::default();

        assert_eq!(topics.availability(), "pilight/status");
        assert_eq!(
            topics.state(id()),
            "pilight/lamp/6ba7b810-9dad-11d1-80b4-00c04fd430c8/state"
        );
        assert_eq!(
            topics.command(id()),
            "pilight/lamp/6ba7b810-9dad-11d1-80b4-00c04fd430c8/set"
        );
        assert_eq!(topics.command_wildcard(), "pilight/lamp/+/set");
        assert_eq!(topics.home_assistant_status(), "homeassistant/status");
        assert_eq!(
            topics.discovery(id()),
            "homeassistant/light/pilight_6ba7b8109dad11d180b400c04fd430c8/config"
        );
    }

    #[test]
    fn object_ids_use_only_characters_home_assistant_accepts() {
        let object = object_id(id());

        assert!(
            object
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "{object} contains characters HA rejects"
        );
    }

    #[test]
    fn command_topics_round_trip_back_to_a_lamp_id() {
        let topics = Topics::default();
        assert_eq!(
            topics.lamp_id_from_command(&topics.command(id())),
            Some(id())
        );
    }

    #[test]
    fn traffic_we_did_not_ask_for_is_ignored() {
        let topics = Topics::default();

        assert_eq!(
            topics.lamp_id_from_command("pilight/lamp/not-a-uuid/set"),
            None
        );
        assert_eq!(topics.lamp_id_from_command(&topics.state(id())), None);
        assert_eq!(topics.lamp_id_from_command("other/lamp/x/set"), None);
        assert_eq!(topics.lamp_id_from_command(""), None);
    }

    #[test]
    fn prefixes_are_configurable_and_trailing_slashes_forgiven() {
        let topics = Topics::new("/home/lights/", "/ha/");

        assert_eq!(topics.availability(), "home/lights/status");
        assert_eq!(topics.home_assistant_status(), "ha/status");
        assert_eq!(
            topics.lamp_id_from_command(&topics.command(id())),
            Some(id()),
            "a custom prefix must still parse back"
        );
    }
}
