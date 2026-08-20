//! Rows of `lamp_states`.

use super::{to_u8, to_u16};
use crate::domain::{BulbMode, LampState, LampStateUpdate};
use crate::error::Result;
use crate::schema::lamp_states;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use uuid::Uuid;

/// Highest scene index an RGB+CCT bulb offers.
const MAX_SCENE: u8 = 8;
/// Highest hue in degrees.
const MAX_HUE: u16 = 359;
/// Percentages top out at 100.
const MAX_PERCENT: u8 = 100;

/// A row of `lamp_states`.
#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = lamp_states, primary_key(lamp_id))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct LampStateRow {
    /// Which lamp this describes.
    pub lamp_id: Uuid,
    /// Last commanded power state.
    pub power: bool,
    /// Last commanded mode, as stored text.
    pub bulb_mode: String,
    /// Brightness percentage.
    pub brightness: Option<i16>,
    /// Hue in degrees.
    pub hue: Option<i16>,
    /// Saturation percentage.
    pub saturation: Option<i16>,
    /// Colour temperature percentage.
    pub kelvin: Option<i16>,
    /// Active scene.
    pub scene: Option<i16>,
    /// The V2 sequence byte for the next command.
    pub next_sequence: i16,
    /// When the belief was last revised.
    pub updated_at: DateTime<Utc>,
}

impl TryFrom<LampStateRow> for LampState {
    type Error = crate::error::DbError;

    fn try_from(row: LampStateRow) -> Result<Self> {
        Ok(Self {
            lamp_id: row.lamp_id,
            power: row.power,
            bulb_mode: BulbMode::parse(&row.bulb_mode)?,
            brightness: optional_percent("brightness", row.brightness)?,
            hue: row
                .hue
                .map(|hue| to_u16("hue", i32::from(hue), MAX_HUE))
                .transpose()?,
            saturation: optional_percent("saturation", row.saturation)?,
            kelvin: optional_percent("kelvin", row.kelvin)?,
            scene: row
                .scene
                .map(|scene| to_u8("scene", scene, MAX_SCENE))
                .transpose()?,
            next_sequence: to_u8("next_sequence", row.next_sequence, u8::MAX)?,
            updated_at: row.updated_at,
        })
    }
}

fn optional_percent(column: &'static str, value: Option<i16>) -> Result<Option<u8>> {
    value.map(|v| to_u8(column, v, MAX_PERCENT)).transpose()
}

/// The row inserted alongside a new lamp.
#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = lamp_states)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewLampStateRow {
    /// Which lamp this describes.
    pub lamp_id: Uuid,
}

/// A partial update to a `lamp_states` row.
///
/// Every field is `Option`, and `None` means "leave the column alone". None of
/// these are nullable-clearing: once we know a lamp's brightness, forgetting it
/// again is not a thing the application ever wants to do.
#[derive(Debug, Clone, Default, AsChangeset)]
#[diesel(table_name = lamp_states)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct LampStateChangeset {
    /// New power state.
    pub power: Option<bool>,
    /// New mode.
    pub bulb_mode: Option<String>,
    /// New brightness.
    pub brightness: Option<i16>,
    /// New hue.
    pub hue: Option<i16>,
    /// New saturation.
    pub saturation: Option<i16>,
    /// New colour temperature.
    pub kelvin: Option<i16>,
    /// New scene.
    pub scene: Option<i16>,
}

impl From<&LampStateUpdate> for LampStateChangeset {
    fn from(update: &LampStateUpdate) -> Self {
        Self {
            power: update.power,
            bulb_mode: update.bulb_mode.map(|mode| mode.as_str().to_owned()),
            brightness: update.brightness.map(i16::from),
            hue: update.hue.map(|hue| hue.min(MAX_HUE)).map(cast_hue),
            saturation: update.saturation.map(i16::from),
            kelvin: update.kelvin.map(i16::from),
            scene: update.scene.map(i16::from),
        }
    }
}

/// Clamp a hue into the range the column's CHECK constraint accepts.
///
/// `MAX_HUE` fits `i16` comfortably, so the fallback is unreachable; it is here
/// so that a future change to `MAX_HUE` degrades rather than panics.
fn cast_hue(hue: u16) -> i16 {
    i16::try_from(hue.min(MAX_HUE)).unwrap_or(i16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> LampStateRow {
        LampStateRow {
            lamp_id: Uuid::nil(),
            power: true,
            bulb_mode: "color".into(),
            brightness: Some(60),
            hue: Some(200),
            saturation: Some(80),
            kelvin: None,
            scene: None,
            next_sequence: 42,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn a_well_formed_row_converts() {
        let state = LampState::try_from(row()).unwrap();

        assert!(state.power);
        assert_eq!(state.bulb_mode, BulbMode::Color);
        assert_eq!(state.brightness, Some(60));
        assert_eq!(state.hue, Some(200));
        assert_eq!(state.kelvin, None);
        assert_eq!(state.next_sequence, 42);
    }

    #[test]
    fn the_full_sequence_range_survives_the_narrowing() {
        // next_sequence is a u8 in the protocol but SMALLINT in the column, so 255
        // has to make it through intact.
        let state = LampState::try_from(LampStateRow {
            next_sequence: 255,
            ..row()
        })
        .unwrap();
        assert_eq!(state.next_sequence, 255);
    }

    #[test]
    fn out_of_range_columns_are_errors_not_truncations() {
        assert!(
            LampState::try_from(LampStateRow {
                brightness: Some(101),
                ..row()
            })
            .is_err()
        );
        assert!(
            LampState::try_from(LampStateRow {
                hue: Some(360),
                ..row()
            })
            .is_err()
        );
        assert!(
            LampState::try_from(LampStateRow {
                scene: Some(9),
                ..row()
            })
            .is_err()
        );
        assert!(
            LampState::try_from(LampStateRow {
                next_sequence: 256,
                ..row()
            })
            .is_err()
        );
        assert!(
            LampState::try_from(LampStateRow {
                saturation: Some(-1),
                ..row()
            })
            .is_err()
        );
    }

    #[test]
    fn an_unknown_mode_is_an_error() {
        assert!(
            LampState::try_from(LampStateRow {
                bulb_mode: "disco".into(),
                ..row()
            })
            .is_err()
        );
    }

    #[test]
    fn an_empty_update_touches_no_column() {
        let changeset = LampStateChangeset::from(&LampStateUpdate::default());

        assert!(changeset.power.is_none());
        assert!(changeset.bulb_mode.is_none());
        assert!(changeset.brightness.is_none());
        assert!(changeset.hue.is_none());
    }

    #[test]
    fn a_hue_update_also_sets_the_mode() {
        let changeset = LampStateChangeset::from(&LampStateUpdate::hue(200));

        assert_eq!(changeset.hue, Some(200));
        assert_eq!(changeset.bulb_mode.as_deref(), Some("color"));
    }

    #[test]
    fn hues_are_clamped_into_the_columns_check_constraint() {
        // The domain already wraps, but a hand-built update must not be able to
        // produce a value the CHECK constraint would reject.
        let changeset = LampStateChangeset::from(&LampStateUpdate {
            hue: Some(9_999),
            ..Default::default()
        });
        assert_eq!(changeset.hue, Some(359));
    }
}
