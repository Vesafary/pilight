//! Row-shaped mirrors of the tables.
//!
//! These exist only to satisfy Diesel. Postgres has no unsigned integers and no
//! sum types, so every field here is wider or vaguer than the domain type it maps
//! to; the `TryFrom` impls are where that gap is closed, and where a row that
//! violates an assumption turns into [`crate::DbError::InvalidStoredValue`]
//! instead of a panic.

mod lamp;
mod lamp_command;
mod lamp_state;
pub(crate) mod lamp_type;

pub use lamp::{LampChangeset, LampRow, NewLampRow};
pub use lamp_command::{LampCommandRow, NewLampCommandRow};
pub use lamp_state::{LampStateChangeset, LampStateRow, NewLampStateRow};
pub use lamp_type::{LampTypeRow, NewLampTypeRow};

use crate::error::{DbError, Result};

/// Narrow a stored integer to `u8`, naming the column if it does not fit.
pub(crate) fn to_u8(column: &'static str, value: i16, max: u8) -> Result<u8> {
    u8::try_from(value)
        .ok()
        .filter(|v| *v <= max)
        .ok_or_else(|| DbError::invalid_value(column, value, "0..=100 percentage"))
}

/// Narrow a stored integer to `u16`, naming the column if it does not fit.
pub(crate) fn to_u16(column: &'static str, value: i32, max: u16) -> Result<u16> {
    u16::try_from(value)
        .ok()
        .filter(|v| *v <= max)
        .ok_or_else(|| DbError::invalid_value(column, value, "16-bit identifier"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrowing_accepts_values_inside_the_range() {
        assert_eq!(to_u8("brightness", 0, 100).unwrap(), 0);
        assert_eq!(to_u8("brightness", 100, 100).unwrap(), 100);
        assert_eq!(to_u16("device_id", 65_535, u16::MAX).unwrap(), 65_535);
    }

    #[test]
    fn narrowing_rejects_negatives_rather_than_wrapping() {
        // The obvious bug this guards against: `value as u8` turning -1 into 255.
        assert!(to_u8("brightness", -1, 100).is_err());
        assert!(to_u16("device_id", -1, u16::MAX).is_err());
    }

    #[test]
    fn narrowing_rejects_values_past_the_maximum() {
        assert!(to_u8("brightness", 101, 100).is_err());
        assert!(to_u16("device_id", 65_536, u16::MAX).is_err());
    }

    #[test]
    fn the_error_names_the_offending_column() {
        let error = to_u8("saturation", 250, 100).unwrap_err();
        assert!(
            matches!(
                error,
                DbError::InvalidStoredValue {
                    column: "saturation",
                    ..
                }
            ),
            "expected the column name in the error, got {error}"
        );
    }
}
