//! The shapes the API speaks.
//!
//! Deliberately separate from the domain types: `LampState` carries
//! `next_sequence`, which is an implementation detail of the radio and has no
//! business in a public payload.

use pilight_db::domain::{BulbMode, Lamp, LampCommand, LampState};
use pilight_proto::RemoteType;
use pilight_service::{LampWithState, StateChange};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

/// Tell an absent field apart from an explicit `null`.
///
/// Serde collapses both to `None` by default, which would make "leave the room
/// alone" and "clear the room" indistinguishable. Deserializing into the inner
/// option and wrapping the result in `Some` keeps them apart: the field being
/// missing falls back to `Default` (`None`), while `null` becomes `Some(None)`.
#[allow(clippy::option_option)]
fn explicit_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::deserialize(deserializer).map(Some)
}

/// A lamp and its state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LampResponse {
    /// Primary key.
    pub id: Uuid,
    /// Human-facing name.
    pub name: String,
    /// Optional grouping.
    pub room: Option<String>,
    /// Which bulb family.
    pub remote_type: RemoteType,
    /// The identity transmitted as.
    pub device_id: u16,
    /// The group; 0 addresses all of them.
    pub group: u8,
    /// What we last told the lamp.
    pub state: LampStateResponse,
    /// When it was registered.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When it was last edited.
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<LampWithState> for LampResponse {
    fn from(entry: LampWithState) -> Self {
        Self::new(&entry.lamp, &entry.state)
    }
}

impl LampResponse {
    /// Build from a lamp and its state.
    #[must_use]
    pub fn new(lamp: &Lamp, state: &LampState) -> Self {
        Self {
            id: lamp.id,
            name: lamp.name.clone(),
            room: lamp.room.clone(),
            remote_type: lamp.remote_type,
            device_id: lamp.device_id,
            group: lamp.group,
            state: LampStateResponse::from(state),
            created_at: lamp.created_at,
            updated_at: lamp.updated_at,
        }
    }
}

/// What we believe a lamp is doing.
///
/// Not a reading — the bulbs cannot be queried. `stale` is not a field because
/// there is no way to know; see the crate docs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LampStateResponse {
    /// Whether it was last told to be on.
    pub power: bool,
    /// Which mode it was last driven into.
    pub bulb_mode: BulbMode,
    /// Brightness percentage.
    pub brightness: Option<u8>,
    /// Hue in degrees.
    pub hue: Option<u16>,
    /// Saturation percentage.
    pub saturation: Option<u8>,
    /// Colour temperature percentage: 0 coolest, 100 warmest.
    pub kelvin: Option<u8>,
    /// The running scene.
    pub scene: Option<u8>,
    /// When this belief was last revised.
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<&LampState> for LampStateResponse {
    fn from(state: &LampState) -> Self {
        Self {
            power: state.power,
            bulb_mode: state.bulb_mode,
            brightness: state.brightness,
            hue: state.hue,
            saturation: state.saturation,
            kelvin: state.kelvin,
            scene: state.scene,
            updated_at: state.updated_at,
        }
    }
}

/// Register a lamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewLampRequest {
    /// Human-facing name.
    pub name: String,
    /// Optional grouping.
    #[serde(default)]
    pub room: Option<String>,
    /// Which bulb family.
    pub remote_type: RemoteType,
    /// The identity to transmit as.
    pub device_id: u16,
    /// The group; 0 addresses all of them.
    pub group: u8,
}

impl From<NewLampRequest> for pilight_db::NewLamp {
    fn from(request: NewLampRequest) -> Self {
        Self {
            name: request.name,
            room: request.room,
            remote_type: request.remote_type,
            device_id: request.device_id,
            group: request.group,
        }
    }
}

/// Edit a lamp's name or room.
///
/// Absent fields are left alone. An explicit `"room": null` clears the room, which
/// is why it is a nested option.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateLampRequest {
    /// New name.
    #[serde(default)]
    pub name: Option<String>,
    /// New room; `null` clears it.
    ///
    /// The nested option is the point: the outer one says whether the field was
    /// present at all, the inner one carries its value.
    #[allow(clippy::option_option)]
    #[serde(default, deserialize_with = "explicit_option")]
    pub room: Option<Option<String>>,
}

impl From<UpdateLampRequest> for pilight_db::LampUpdate {
    fn from(request: UpdateLampRequest) -> Self {
        Self {
            name: request.name,
            room: request.room,
        }
    }
}

/// Change a lamp's state. Absent fields are left alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateRequest {
    /// Switch on or off.
    #[serde(default)]
    pub power: Option<bool>,
    /// Drop into night mode. Overrides everything else.
    #[serde(default)]
    pub night_mode: bool,
    /// Brightness percentage.
    #[serde(default)]
    pub brightness: Option<u8>,
    /// Hue in degrees.
    #[serde(default)]
    pub hue: Option<u16>,
    /// Saturation percentage.
    #[serde(default)]
    pub saturation: Option<u8>,
    /// Colour temperature percentage: 0 coolest, 100 warmest.
    #[serde(default)]
    pub kelvin: Option<u8>,
    /// Scene index.
    #[serde(default)]
    pub scene: Option<u8>,
}

impl From<StateRequest> for StateChange {
    fn from(request: StateRequest) -> Self {
        Self {
            power: request.power,
            night_mode: request.night_mode,
            brightness: request.brightness,
            hue: request.hue,
            saturation: request.saturation,
            kelvin: request.kelvin,
            scene: request.scene,
        }
    }
}

/// A bulb family.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LampTypeResponse {
    /// Stable key.
    pub slug: String,
    /// Human-readable name.
    pub display_name: String,
    /// 1 or 2.
    pub protocol_generation: u8,
    /// How many groups it addresses; 0 means groupless.
    pub num_groups: u8,
    /// Whether the driver can currently speak to it.
    pub driver_supported: bool,
}

impl From<RemoteType> for LampTypeResponse {
    fn from(remote_type: RemoteType) -> Self {
        Self {
            slug: remote_type.slug().to_owned(),
            display_name: remote_type.display_name().to_owned(),
            protocol_generation: remote_type.protocol_generation(),
            num_groups: remote_type.num_groups(),
            driver_supported: remote_type.is_driver_supported(),
        }
    }
}

/// One entry from the audit trail.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandResponse {
    /// Primary key.
    pub id: i64,
    /// Who asked.
    pub source: String,
    /// What was asked for.
    pub command: String,
    /// The argument, where there was one.
    pub argument: Option<i32>,
    /// Whether the radio accepted it.
    pub succeeded: bool,
    /// Why it failed, if it did.
    pub error: Option<String>,
    /// When it was attempted.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<LampCommand> for CommandResponse {
    fn from(command: LampCommand) -> Self {
        Self {
            id: command.id,
            source: command.source.to_string(),
            command: command.command,
            argument: command.argument,
            succeeded: command.succeeded,
            error: command.error,
            created_at: command.created_at,
        }
    }
}

/// Whether the daemon is well.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthResponse {
    /// `"ok"` or `"degraded"`.
    pub status: String,
    /// Whether the database answered.
    pub database: bool,
    /// How many lamps are registered.
    pub lamps: usize,
    /// The crate version.
    pub version: String,
}
