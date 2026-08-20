//! A lamp: a paired (family, device id, group) with a name attached.

use crate::error::{DbError, Result};
use chrono::{DateTime, Utc};
use pilight_proto::RemoteType;
use uuid::Uuid;

/// A registered lamp.
///
/// The `(remote_type, device_id, group)` triple is what a bulb physically listens
/// for; the name and room are ours. Group 0 addresses every group of that device
/// id at once.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Lamp {
    /// Primary key.
    pub id: Uuid,
    /// Human-facing name.
    pub name: String,
    /// Optional grouping for the UI.
    pub room: Option<String>,
    /// Which bulb family this is.
    pub remote_type: RemoteType,
    /// The 16-bit identity we transmit as.
    pub device_id: u16,
    /// The group, `0..=num_groups`.
    pub group: u8,
    /// When the lamp was registered.
    pub created_at: DateTime<Utc>,
    /// When it was last edited.
    pub updated_at: DateTime<Utc>,
}

impl Lamp {
    /// The address a bulb listens on, as a tuple.
    #[must_use]
    pub const fn address(&self) -> (RemoteType, u16, u8) {
        (self.remote_type, self.device_id, self.group)
    }

    /// Whether this lamp addresses every group of its device id.
    #[must_use]
    pub const fn is_all_groups(&self) -> bool {
        self.group == 0
    }
}

/// Fields needed to register a lamp.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct NewLamp {
    /// Human-facing name. Must not be blank.
    pub name: String,
    /// Optional grouping for the UI.
    pub room: Option<String>,
    /// Which bulb family this is.
    pub remote_type: RemoteType,
    /// The 16-bit identity we transmit as.
    pub device_id: u16,
    /// The group, `0..=num_groups` for the family.
    pub group: u8,
}

impl NewLamp {
    /// Check the fields the database cannot check for us.
    ///
    /// The `CHECK` constraints cover ranges, but not "does group 6 make sense for a
    /// four-group family" — that depends on the type, and is enforced here.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(DbError::Invalid("a lamp needs a name".into()));
        }
        if self
            .room
            .as_ref()
            .is_some_and(|room| room.trim().is_empty())
        {
            return Err(DbError::Invalid(
                "room must be absent rather than blank".into(),
            ));
        }
        if !self.remote_type.is_driver_supported() {
            return Err(DbError::Invalid(format!(
                "{} is documented but not yet drivable",
                self.remote_type
            )));
        }
        if self.group > self.remote_type.num_groups() {
            return Err(DbError::Protocol(pilight_proto::Error::GroupOutOfRange {
                group: self.group,
                max: self.remote_type.num_groups(),
            }));
        }
        Ok(())
    }
}

/// A partial edit to a lamp. `None` leaves a field alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct LampUpdate {
    /// New name.
    pub name: Option<String>,
    /// New room. `Some(None)` clears it.
    pub room: Option<Option<String>>,
}

impl LampUpdate {
    /// Whether this update would change anything.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.name.is_none() && self.room.is_none()
    }

    /// Check the fields the database cannot check for us.
    pub fn validate(&self) -> Result<()> {
        if self.name.as_ref().is_some_and(|n| n.trim().is_empty()) {
            return Err(DbError::Invalid("a lamp needs a name".into()));
        }
        if self
            .room
            .as_ref()
            .and_then(Option::as_ref)
            .is_some_and(|room| room.trim().is_empty())
        {
            return Err(DbError::Invalid(
                "room must be absent rather than blank".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> NewLamp {
        NewLamp {
            name: "Couch".into(),
            room: Some("Living room".into()),
            remote_type: RemoteType::RgbCct,
            device_id: 0xBEEF,
            group: 1,
        }
    }

    #[test]
    fn a_well_formed_lamp_validates() {
        assert!(valid().validate().is_ok());
    }

    #[test]
    fn group_zero_is_allowed_and_means_all_groups() {
        let lamp = NewLamp {
            group: 0,
            ..valid()
        };
        assert!(lamp.validate().is_ok());
    }

    #[test]
    fn a_group_beyond_the_family_is_rejected() {
        // RGB+CCT has four groups, so 5 is nonsense even though the column allows it.
        let lamp = NewLamp {
            group: 5,
            ..valid()
        };
        assert!(matches!(
            lamp.validate(),
            Err(DbError::Protocol(pilight_proto::Error::GroupOutOfRange {
                group: 5,
                max: 4
            }))
        ));
    }

    #[test]
    fn eight_group_families_allow_higher_groups() {
        // ...but FUT089 has no command layer yet, so it is refused for that reason.
        let lamp = NewLamp {
            remote_type: RemoteType::Fut089,
            group: 8,
            ..valid()
        };
        let error = lamp.validate().unwrap_err();
        assert!(
            matches!(error, DbError::Invalid(ref m) if m.contains("not yet drivable")),
            "expected an unsupported-family error, got {error}"
        );
    }

    #[test]
    fn blank_names_and_rooms_are_rejected() {
        assert!(
            NewLamp {
                name: "   ".into(),
                ..valid()
            }
            .validate()
            .is_err()
        );
        assert!(
            NewLamp {
                room: Some(" ".into()),
                ..valid()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn an_empty_update_changes_nothing() {
        assert!(LampUpdate::default().is_empty());
        assert!(
            !LampUpdate {
                name: Some("Lamp".into()),
                ..Default::default()
            }
            .is_empty()
        );
    }

    #[test]
    fn an_update_can_clear_the_room_but_not_blank_it() {
        let clear = LampUpdate {
            room: Some(None),
            ..Default::default()
        };
        assert!(clear.validate().is_ok());

        let blank = LampUpdate {
            room: Some(Some("  ".into())),
            ..Default::default()
        };
        assert!(blank.validate().is_err());
    }
}
