//! Home Assistant MQTT bridge for `pilight`.
//!
//! Publishes a retained MQTT-discovery config per lamp, so the lights appear in
//! Home Assistant with no YAML, then translates between HA's JSON light schema
//! and the protocol's own units.
//!
//! # Topics
//!
//! ```text
//! homeassistant/light/pilight_<uuid>/config   discovery, retained
//! homeassistant/status                        HA's birth message — we listen
//! pilight/status                              our availability, retained + LWT
//! pilight/lamp/<uuid>/state                   lamp state, retained
//! pilight/lamp/<uuid>/set                     commands
//! ```
//!
//! # What Home Assistant is told
//!
//! Brightness, `hs` colour, colour temperature in Kelvin, and the nine scenes as
//! effects. `transition` and `flash` are switched off because the bulbs cannot do
//! either, and `optimistic` is off because we publish real state after every
//! command.
//!
//! # The honest caveat
//!
//! The bulbs never acknowledge anything and cannot be queried. What HA displays is
//! what we last *told* a bulb, not what it is doing — it drifts the moment someone
//! picks up a physical remote.

#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::module_name_repetitions
)]

pub mod bridge;
pub mod config;
pub mod discovery;
pub mod error;
pub mod payload;
pub mod topics;
pub mod units;

pub use bridge::Bridge;
pub use config::MqttConfig;
pub use discovery::LightDiscovery;
pub use error::{MqttError, Result};
pub use payload::LightPayload;
pub use topics::Topics;
pub use units::KelvinRange;
