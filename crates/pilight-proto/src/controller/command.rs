//! RGB+CCT (FUT092) command and argument encoding.
//!
//! See `docs/protocol.md` §5.2. Two things make this family awkward:
//!
//! * Command `0x04` sets **either** brightness or saturation, depending on whether
//!   the bulb is currently in white or colour mode. The offsets differ, so the
//!   ranges do not collide in practice.
//! * There is no explicit "white" command — sending a Kelvin command is what drives
//!   the bulb out of colour mode.

use crate::error::{Error, Result};

/// Command byte values for RGB+CCT bulbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RgbCctCommand {
    /// On, off, night mode, and scene speed — disambiguated by the argument.
    OnOff = 0x01,
    /// Set hue.
    Color = 0x02,
    /// Set colour temperature. Also forces the bulb into white mode.
    Kelvin = 0x03,
    /// Set brightness (white mode) or saturation (colour mode).
    BrightnessOrSaturation = 0x04,
    /// Select a scene.
    Mode = 0x05,
}

impl From<RgbCctCommand> for u8 {
    fn from(command: RgbCctCommand) -> Self {
        command as u8
    }
}

/// Arguments to [`RgbCctCommand::OnOff`] that are not group selections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RgbCctOnOffArgument {
    /// Speed the running scene up.
    ModeSpeedUp = 0x0A,
    /// Slow the running scene down.
    ModeSpeedDown = 0x0B,
}

impl From<RgbCctOnOffArgument> for u8 {
    fn from(argument: RgbCctOnOffArgument) -> Self {
        argument as u8
    }
}

/// Number of scenes an RGB+CCT bulb cycles through.
pub const NUM_MODES: u8 = 9;

/// Hue arguments start here rather than at zero.
pub const COLOR_OFFSET: u8 = 0x5F;
/// Brightness arguments start here.
pub const BRIGHTNESS_OFFSET: u8 = 0x8F;
/// Saturation arguments start here.
pub const SATURATION_OFFSET: u8 = 0x0D;
/// Where the Kelvin scale ends (100% = warmest). It starts at `0x94` and counts
/// *down* in steps of two, wrapping through `0x00`, to land here.
pub const KELVIN_SCALE_END: u8 = 0xCC;
/// The Kelvin scale advances two units per percent.
pub const KELVIN_INTERVAL: u8 = 2;

/// A validated group id.
///
/// Group 0 addresses every group at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupId {
    group: u8,
    max: u8,
}

impl GroupId {
    /// Validate `group` against a remote with `max` groups.
    pub const fn new(group: u8, max: u8) -> Result<Self> {
        if group > max {
            return Err(Error::GroupOutOfRange { group, max });
        }
        Ok(Self { group, max })
    }

    /// The group number. Zero means "all groups".
    #[must_use]
    pub const fn get(self) -> u8 {
        self.group
    }

    /// How many groups the remote this id was built for has.
    #[must_use]
    pub const fn max(self) -> u8 {
        self.max
    }

    /// Whether this id addresses every group.
    #[must_use]
    pub const fn is_all(self) -> bool {
        self.group == 0
    }

    /// The argument byte for an on/off command.
    ///
    /// ON is the group number as-is; OFF is offset past the whole ON block.
    #[must_use]
    pub const fn on_off_argument(self, on: bool) -> u8 {
        if on {
            self.group
        } else {
            self.group + self.max + 1
        }
    }
}

/// Convert a percentage into one of the "counts down to zero, then back up" scales
/// the V2 protocol uses for Kelvin and (on some families) brightness.
///
/// See `docs/protocol.md` §5.2.
pub const fn to_v2_scale(percent: u8, end_value: u8, interval: u8, reverse: bool) -> u8 {
    // Saturating, not `-`: this is public, and a debug build would abort on 101.
    let value = if reverse {
        100u8.saturating_sub(percent)
    } else {
        percent
    };

    value.wrapping_mul(interval).wrapping_add(end_value)
}

/// Validate that `percent` is in `0..=100`.
pub const fn check_percentage(percent: u8) -> Result<u8> {
    if percent > 100 {
        return Err(Error::PercentageOutOfRange(percent));
    }
    Ok(percent)
}

/// Rescale a hue in degrees (`0..360`) onto the protocol's 0–255 colour wheel.
#[must_use]
pub const fn hue_to_raw(degrees: u16) -> u8 {
    (((degrees % 360) as u32 * 255 / 360) & 0xFF) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_off_argument_offsets_off_past_the_on_block() {
        let group = GroupId::new(1, 4).unwrap();
        assert_eq!(group.on_off_argument(true), 1);
        assert_eq!(group.on_off_argument(false), 6);

        let all = GroupId::new(0, 4).unwrap();
        assert_eq!(all.on_off_argument(true), 0);
        assert_eq!(all.on_off_argument(false), 5);
    }

    #[test]
    fn eight_group_remotes_shift_the_off_block_further_out() {
        let group = GroupId::new(1, 8).unwrap();
        assert_eq!(group.on_off_argument(false), 10);
    }

    #[test]
    fn kelvin_scale_runs_from_coolest_to_warmest() {
        // The scale starts at 0x94, counts down through 0x00, and lands on 0xCC:
        // [0x94, 0x92, .., 0x00, .., 0xCE, 0xCC].
        assert_eq!(
            to_v2_scale(0, KELVIN_SCALE_END, KELVIN_INTERVAL, true),
            0x94
        );
        assert_eq!(
            to_v2_scale(100, KELVIN_SCALE_END, KELVIN_INTERVAL, true),
            0xCC
        );
        assert_eq!(
            to_v2_scale(74, KELVIN_SCALE_END, KELVIN_INTERVAL, true),
            0x00
        );
        assert_eq!(
            to_v2_scale(75, KELVIN_SCALE_END, KELVIN_INTERVAL, true),
            0xFE
        );
    }

    #[test]
    fn hue_wraps_at_a_full_turn() {
        assert_eq!(hue_to_raw(0), 0);
        assert_eq!(hue_to_raw(360), 0);
        assert_eq!(hue_to_raw(180), 127);
        assert_eq!(hue_to_raw(359), 254);
    }

    #[test]
    fn percentages_above_one_hundred_are_rejected() {
        assert!(check_percentage(100).is_ok());
        assert!(matches!(
            check_percentage(101),
            Err(Error::PercentageOutOfRange(101))
        ));
    }
}
