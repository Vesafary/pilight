//! Rows of `lamps`.

use super::{to_u8, to_u16};
use crate::domain::{Lamp, NewLamp};
use crate::error::Result;
use crate::models::lamp_type::{id_of, remote_type_of};
use crate::schema::lamps;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use uuid::Uuid;

/// A row of `lamps`.
#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = lamps)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct LampRow {
    /// Primary key.
    pub id: Uuid,
    /// Human-facing name.
    pub name: String,
    /// Optional grouping.
    pub room: Option<String>,
    /// Foreign key into `lamp_types`.
    pub lamp_type_id: i16,
    /// The driver's `u16` device id, widened because Postgres has no `u16`.
    pub device_id: i32,
    /// The group.
    pub group_id: i16,
    /// When the lamp was registered.
    pub created_at: DateTime<Utc>,
    /// When it was last edited.
    pub updated_at: DateTime<Utc>,
}

impl TryFrom<LampRow> for Lamp {
    type Error = crate::error::DbError;

    fn try_from(row: LampRow) -> Result<Self> {
        let remote_type = remote_type_of(row.lamp_type_id)?;

        Ok(Self {
            id: row.id,
            name: row.name,
            room: row.room,
            remote_type,
            device_id: to_u16("device_id", row.device_id, u16::MAX)?,
            group: to_u8("group_id", row.group_id, remote_type.num_groups())?,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// A row to insert into `lamps`.
#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = lamps)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewLampRow {
    /// Human-facing name.
    pub name: String,
    /// Optional grouping.
    pub room: Option<String>,
    /// Foreign key into `lamp_types`.
    pub lamp_type_id: i16,
    /// Widened device id.
    pub device_id: i32,
    /// The group.
    pub group_id: i16,
}

impl From<&NewLamp> for NewLampRow {
    fn from(lamp: &NewLamp) -> Self {
        Self {
            name: lamp.name.trim().to_owned(),
            room: lamp.room.as_deref().map(str::trim).map(str::to_owned),
            lamp_type_id: id_of(lamp.remote_type),
            device_id: i32::from(lamp.device_id),
            group_id: i16::from(lamp.group),
        }
    }
}

/// A partial edit to a `lamps` row.
#[derive(Debug, Clone, Default, AsChangeset)]
#[diesel(table_name = lamps)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct LampChangeset {
    /// New name.
    pub name: Option<String>,
    /// New room. The double option lets `Some(None)` clear the column.
    pub room: Option<Option<String>>,
}

impl From<&crate::domain::LampUpdate> for LampChangeset {
    fn from(update: &crate::domain::LampUpdate) -> Self {
        Self {
            name: update.name.as_deref().map(str::trim).map(str::to_owned),
            room: update
                .room
                .as_ref()
                .map(|room| room.as_deref().map(str::trim).map(str::to_owned)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LampUpdate;
    use pilight_proto::RemoteType;

    fn row() -> LampRow {
        LampRow {
            id: Uuid::nil(),
            name: "Couch".into(),
            room: Some("Living room".into()),
            lamp_type_id: id_of(RemoteType::RgbCct),
            device_id: 0xBEEF,
            group_id: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn a_well_formed_row_converts() {
        let lamp = Lamp::try_from(row()).unwrap();

        assert_eq!(lamp.remote_type, RemoteType::RgbCct);
        assert_eq!(lamp.device_id, 0xBEEF);
        assert_eq!(lamp.group, 1);
        assert!(!lamp.is_all_groups());
    }

    #[test]
    fn the_widest_device_id_survives_the_narrowing() {
        let lamp = Lamp::try_from(LampRow {
            device_id: 65_535,
            ..row()
        })
        .unwrap();
        assert_eq!(lamp.device_id, 65_535);
    }

    #[test]
    fn a_device_id_outside_u16_is_an_error_not_a_truncation() {
        assert!(
            Lamp::try_from(LampRow {
                device_id: 65_536,
                ..row()
            })
            .is_err()
        );
        assert!(
            Lamp::try_from(LampRow {
                device_id: -1,
                ..row()
            })
            .is_err()
        );
    }

    #[test]
    fn a_group_beyond_the_family_is_an_error() {
        // The column allows 0..=8; four-group families do not.
        assert!(
            Lamp::try_from(LampRow {
                group_id: 5,
                ..row()
            })
            .is_err()
        );
    }

    #[test]
    fn an_unknown_lamp_type_is_an_error() {
        assert!(
            Lamp::try_from(LampRow {
                lamp_type_id: 99,
                ..row()
            })
            .is_err()
        );
    }

    #[test]
    fn insert_rows_trim_whitespace() {
        let new = NewLamp {
            name: "  Couch  ".into(),
            room: Some("  Living room  ".into()),
            remote_type: RemoteType::RgbCct,
            device_id: 1,
            group: 1,
        };
        let row = NewLampRow::from(&new);

        assert_eq!(row.name, "Couch");
        assert_eq!(row.room.as_deref(), Some("Living room"));
    }

    #[test]
    fn a_changeset_distinguishes_clearing_from_leaving_alone() {
        let leave = LampChangeset::from(&LampUpdate::default());
        assert!(leave.room.is_none(), "None means do not touch the column");

        let clear = LampChangeset::from(&LampUpdate {
            room: Some(None),
            ..Default::default()
        });
        assert_eq!(clear.room, Some(None), "Some(None) means set it to NULL");
    }
}
