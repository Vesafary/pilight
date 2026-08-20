//! Rows of `lamp_types`.

use crate::error::{DbError, Result};
use crate::schema::lamp_types;
use diesel::prelude::*;
use pilight_proto::RemoteType;

/// A row of `lamp_types`.
#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = lamp_types)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct LampTypeRow {
    /// Stable id, matching the family's index in [`RemoteType::ALL`].
    pub id: i16,
    /// Machine-readable key.
    pub slug: String,
    /// Human-readable name.
    pub display_name: String,
    /// 1 or 2.
    pub protocol_generation: i16,
    /// Byte 1 of a V2 packet; `None` for V1 families.
    pub protocol_id: Option<i16>,
    /// Addressable groups; 0 means groupless.
    pub num_groups: i16,
    /// Whether the driver can currently speak to this family.
    pub driver_supported: bool,
}

/// A row to insert into `lamp_types`.
#[derive(Debug, Clone, Insertable, AsChangeset)]
#[diesel(table_name = lamp_types)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewLampTypeRow {
    /// Stable id.
    pub id: i16,
    /// Machine-readable key.
    pub slug: String,
    /// Human-readable name.
    pub display_name: String,
    /// 1 or 2.
    pub protocol_generation: i16,
    /// Byte 1 of a V2 packet; `None` for V1 families.
    pub protocol_id: Option<i16>,
    /// Addressable groups.
    pub num_groups: i16,
    /// Whether the driver can currently speak to this family.
    pub driver_supported: bool,
}

impl NewLampTypeRow {
    /// Derive a row from the driver's own notion of a family.
    ///
    /// This is the single source of truth: the table is a projection of
    /// [`RemoteType::ALL`], synced at startup rather than seeded by a migration,
    /// so adding a family needs no new migration.
    #[must_use]
    pub fn from_remote_type(remote_type: RemoteType) -> Self {
        Self {
            id: id_of(remote_type),
            slug: remote_type.slug().to_owned(),
            display_name: remote_type.display_name().to_owned(),
            protocol_generation: i16::from(remote_type.protocol_generation()),
            protocol_id: remote_type.protocol_id().map(i16::from),
            num_groups: i16::from(remote_type.num_groups()),
            driver_supported: remote_type.is_driver_supported(),
        }
    }
}

/// The stable database id of a family: its index in [`RemoteType::ALL`].
///
/// Never reorder that array — these ids are foreign keys.
#[must_use]
pub fn id_of(remote_type: RemoteType) -> i16 {
    RemoteType::ALL
        .iter()
        .position(|candidate| *candidate == remote_type)
        .and_then(|index| i16::try_from(index).ok())
        .expect("RemoteType::ALL contains every variant and is far shorter than i16::MAX")
}

/// Recover a family from its stable database id.
pub fn remote_type_of(id: i16) -> Result<RemoteType> {
    usize::try_from(id)
        .ok()
        .and_then(|index| RemoteType::ALL.get(index).copied())
        .ok_or_else(|| DbError::invalid_value("lamp_type_id", id, "lamp type id"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_for_every_family() {
        for remote_type in RemoteType::ALL {
            assert_eq!(remote_type_of(id_of(remote_type)).unwrap(), remote_type);
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<i16> = RemoteType::ALL.iter().copied().map(id_of).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    #[test]
    fn an_unknown_id_is_an_error() {
        assert!(remote_type_of(-1).is_err());
        assert!(remote_type_of(99).is_err());
    }

    #[test]
    fn rows_mirror_the_driver() {
        let row = NewLampTypeRow::from_remote_type(RemoteType::RgbCct);

        assert_eq!(row.slug, "rgb_cct");
        assert_eq!(row.protocol_generation, 2);
        assert_eq!(row.protocol_id, Some(0x20));
        assert_eq!(row.num_groups, 4);
        assert!(row.driver_supported);
    }

    #[test]
    fn v1_families_have_no_protocol_id() {
        let row = NewLampTypeRow::from_remote_type(RemoteType::Rgbw);

        assert_eq!(row.protocol_generation, 1);
        assert_eq!(row.protocol_id, None);
    }
}
