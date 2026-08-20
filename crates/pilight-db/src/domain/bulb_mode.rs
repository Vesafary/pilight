//! Which of a bulb's mutually exclusive modes is active.

use crate::error::DbError;

/// The mode an RGB+CCT bulb is in.
///
/// These are exclusive: setting a colour temperature drops the bulb out of colour
/// mode, and selecting a scene leaves both behind. The protocol offers no way to
/// ask which one is active, so this is inferred from the last command sent.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum BulbMode {
    /// Tunable white: brightness and colour temperature apply.
    #[default]
    White,
    /// Colour: hue and saturation apply.
    Color,
    /// A built-in scene is running.
    Scene,
    /// Night mode: a dim warm glow, set by holding the OFF button.
    Night,
}

impl BulbMode {
    /// Every mode, for exhaustive iteration.
    pub const ALL: [Self; 4] = [Self::White, Self::Color, Self::Scene, Self::Night];

    /// The value stored in `lamp_states.bulb_mode`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Color => "color",
            Self::Scene => "scene",
            Self::Night => "night",
        }
    }

    /// Parse a stored value.
    pub fn parse(value: &str) -> Result<Self, DbError> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == value)
            .ok_or_else(|| DbError::invalid_value("bulb_mode", value, "bulb mode"))
    }
}

impl std::fmt::Display for BulbMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_round_trips_through_its_stored_form() {
        for mode in BulbMode::ALL {
            assert_eq!(BulbMode::parse(mode.as_str()).unwrap(), mode);
        }
    }

    #[test]
    fn an_unknown_mode_is_an_error_not_a_default() {
        let error = BulbMode::parse("disco").unwrap_err();
        assert!(matches!(error, DbError::InvalidStoredValue { .. }));
    }

    #[test]
    fn white_is_the_default() {
        assert_eq!(BulbMode::default(), BulbMode::White);
    }
}
