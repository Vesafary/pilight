//! How to reach the broker.

use crate::topics::{DEFAULT_DISCOVERY_PREFIX, DEFAULT_PREFIX, Topics};
use crate::units::KelvinRange;
use std::time::Duration;

/// Default MQTT port.
pub const DEFAULT_PORT: u16 = 1883;

/// Default keep-alive. Shorter than the broker's usual 60s so a dropped link is
/// noticed while HA is still showing the lights as available.
pub const DEFAULT_KEEP_ALIVE: Duration = Duration::from_secs(30);

/// How many messages may be queued towards the broker before publishing waits.
pub const DEFAULT_CAPACITY: usize = 64;

/// Everything the bridge needs to connect and lay out its topics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttConfig {
    /// Broker hostname.
    pub host: String,
    /// Broker port.
    pub port: u16,
    /// Client id. Must be unique on the broker.
    pub client_id: String,
    /// Username, if the broker wants one.
    pub username: Option<String>,
    /// Password, if the broker wants one.
    pub password: Option<String>,
    /// Prefix for this bridge's own topics.
    pub prefix: String,
    /// Prefix Home Assistant watches for discovery.
    pub discovery_prefix: String,
    /// Keep-alive interval.
    pub keep_alive: Duration,
    /// The Kelvin range the bulbs are assumed to cover.
    pub kelvin: KelvinRange,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_owned(),
            port: DEFAULT_PORT,
            client_id: "pilight".to_owned(),
            username: None,
            password: None,
            prefix: DEFAULT_PREFIX.to_owned(),
            discovery_prefix: DEFAULT_DISCOVERY_PREFIX.to_owned(),
            keep_alive: DEFAULT_KEEP_ALIVE,
            kelvin: KelvinRange::default(),
        }
    }
}

impl MqttConfig {
    /// Read the configuration from the environment.
    ///
    /// | Variable | Default |
    /// |---|---|
    /// | `PILIGHT_MQTT_HOST` | `localhost` |
    /// | `PILIGHT_MQTT_PORT` | `1883` |
    /// | `PILIGHT_MQTT_CLIENT_ID` | `pilight` |
    /// | `PILIGHT_MQTT_USERNAME` | unset |
    /// | `PILIGHT_MQTT_PASSWORD` | unset |
    /// | `PILIGHT_MQTT_PREFIX` | `pilight` |
    /// | `PILIGHT_MQTT_DISCOVERY_PREFIX` | `homeassistant` |
    /// | `PILIGHT_MIN_KELVIN` | `2700` |
    /// | `PILIGHT_MAX_KELVIN` | `6500` |
    ///
    /// Unparseable numbers fall back to the default rather than failing, so a
    /// typo in one variable cannot stop the daemon starting.
    #[must_use]
    pub fn from_env() -> Self {
        let defaults = Self::default();
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let number =
            |name: &str, fallback: u16| var(name).and_then(|v| v.parse().ok()).unwrap_or(fallback);

        let kelvin = KelvinRange::new(
            number("PILIGHT_MIN_KELVIN", defaults.kelvin.min),
            number("PILIGHT_MAX_KELVIN", defaults.kelvin.max),
        );

        Self {
            host: var("PILIGHT_MQTT_HOST").unwrap_or(defaults.host),
            port: number("PILIGHT_MQTT_PORT", defaults.port),
            client_id: var("PILIGHT_MQTT_CLIENT_ID").unwrap_or(defaults.client_id),
            username: var("PILIGHT_MQTT_USERNAME"),
            password: var("PILIGHT_MQTT_PASSWORD"),
            prefix: var("PILIGHT_MQTT_PREFIX").unwrap_or(defaults.prefix),
            discovery_prefix: var("PILIGHT_MQTT_DISCOVERY_PREFIX")
                .unwrap_or(defaults.discovery_prefix),
            keep_alive: defaults.keep_alive,
            kelvin,
        }
    }

    /// The topic layout implied by this configuration.
    #[must_use]
    pub fn topics(&self) -> Topics {
        Topics::new(&self.prefix, &self.discovery_prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_point_at_a_local_broker() {
        let config = MqttConfig::default();

        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 1883);
        assert_eq!(config.username, None);
    }

    #[test]
    fn the_topic_layout_follows_the_configured_prefixes() {
        let config = MqttConfig {
            prefix: "home/lights".to_owned(),
            discovery_prefix: "ha".to_owned(),
            ..Default::default()
        };

        assert_eq!(config.topics().availability(), "home/lights/status");
        assert_eq!(config.topics().home_assistant_status(), "ha/status");
    }

    #[test]
    fn a_reversed_kelvin_range_is_corrected() {
        let config = MqttConfig {
            kelvin: KelvinRange::new(6500, 2700),
            ..Default::default()
        };

        assert_eq!(config.kelvin.min, 2700);
        assert_eq!(config.kelvin.max, 6500);
    }
}
