//! What a caller wants a lamp to do, separated from who sends it.
//!
//! Splitting intent from transmission is what lets one radio serve many lamps:
//! the identity (`device_id`, group, sequence) comes from storage, while the
//! intent carries only the change being asked for. It is also the natural unit for
//! an audit log, an MQTT message, or an HTTP request body.

use super::command::{
    BRIGHTNESS_OFFSET, COLOR_OFFSET, GroupId, KELVIN_INTERVAL, KELVIN_SCALE_END, NUM_MODES,
    RgbCctCommand, RgbCctOnOffArgument, SATURATION_OFFSET, check_percentage, hue_to_raw,
    to_v2_scale,
};
use crate::error::Result;
use crate::packet::HELD_FLAG;

/// A single change to ask of an RGB+CCT lamp.
///
/// Deliberately **not** `#[non_exhaustive]`: everything that consumes an intent
/// lives in this workspace, and adding a variant should break those matches so the
/// new case gets handled rather than silently falling through a wildcard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbCctIntent {
    /// Switch the group on or off.
    Power(bool),
    /// Drop into night mode — a long press on the group's OFF button.
    NightMode,
    /// Set the hue, in degrees. Puts the bulb into colour mode.
    Hue(u16),
    /// Set brightness, as a percentage.
    Brightness(u8),
    /// Set saturation, as a percentage. Only meaningful in colour mode.
    Saturation(u8),
    /// Set colour temperature: 0 is coolest, 100 warmest. Forces white mode.
    Kelvin(u8),
    /// Select a scene.
    Scene(u8),
    /// Speed the running scene up.
    SceneSpeedUp,
    /// Slow the running scene down.
    SceneSpeedDown,
}

impl RgbCctIntent {
    /// Turn the intent into the `(command, argument)` bytes for a V2 packet.
    ///
    /// The command byte includes the held flag where the intent implies one.
    /// `group` is needed because on/off encodes the target group in its argument.
    pub fn encode(self, group: GroupId) -> Result<(u8, u8)> {
        let (command, argument) = match self {
            Self::Power(on) => (RgbCctCommand::OnOff, group.on_off_argument(on)),
            Self::NightMode => {
                let argument = group.on_off_argument(false);
                return Ok((u8::from(RgbCctCommand::OnOff) | HELD_FLAG, argument));
            }
            Self::Hue(degrees) => (
                RgbCctCommand::Color,
                COLOR_OFFSET.wrapping_add(hue_to_raw(degrees)),
            ),
            Self::Brightness(percent) => (
                RgbCctCommand::BrightnessOrSaturation,
                BRIGHTNESS_OFFSET.wrapping_add(check_percentage(percent)?),
            ),
            Self::Saturation(percent) => (
                RgbCctCommand::BrightnessOrSaturation,
                SATURATION_OFFSET.wrapping_add(check_percentage(percent)?),
            ),
            Self::Kelvin(percent) => (
                RgbCctCommand::Kelvin,
                to_v2_scale(
                    check_percentage(percent)?,
                    KELVIN_SCALE_END,
                    KELVIN_INTERVAL,
                    true,
                ),
            ),
            Self::Scene(scene) => (RgbCctCommand::Mode, scene % NUM_MODES),
            Self::SceneSpeedUp => (
                RgbCctCommand::OnOff,
                RgbCctOnOffArgument::ModeSpeedUp.into(),
            ),
            Self::SceneSpeedDown => (
                RgbCctCommand::OnOff,
                RgbCctOnOffArgument::ModeSpeedDown.into(),
            ),
        };

        Ok((u8::from(command), argument))
    }

    /// A short, stable name for logs and audit records.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Power(true) => "on",
            Self::Power(false) => "off",
            Self::NightMode => "night_mode",
            Self::Hue(_) => "hue",
            Self::Brightness(_) => "brightness",
            Self::Saturation(_) => "saturation",
            Self::Kelvin(_) => "kelvin",
            Self::Scene(_) => "scene",
            Self::SceneSpeedUp => "scene_speed_up",
            Self::SceneSpeedDown => "scene_speed_down",
        }
    }

    /// The intent's numeric argument, where it has one.
    #[must_use]
    pub const fn argument(self) -> Option<i32> {
        match self {
            Self::Hue(degrees) => Some(degrees as i32),
            Self::Brightness(v) | Self::Saturation(v) | Self::Kelvin(v) | Self::Scene(v) => {
                Some(v as i32)
            }
            Self::Power(_) | Self::NightMode | Self::SceneSpeedUp | Self::SceneSpeedDown => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(n: u8) -> GroupId {
        GroupId::new(n, 4).unwrap()
    }

    #[test]
    fn power_encodes_the_group_in_its_argument() {
        assert_eq!(
            RgbCctIntent::Power(true).encode(group(1)).unwrap(),
            (0x01, 1)
        );
        assert_eq!(
            RgbCctIntent::Power(false).encode(group(1)).unwrap(),
            (0x01, 6)
        );
        assert_eq!(
            RgbCctIntent::Power(true).encode(group(0)).unwrap(),
            (0x01, 0)
        );
    }

    #[test]
    fn night_mode_is_a_held_off() {
        let (command, argument) = RgbCctIntent::NightMode.encode(group(1)).unwrap();

        assert_eq!(command, 0x01 | HELD_FLAG);
        assert_eq!(argument, 6, "the OFF argument for group 1");
    }

    #[test]
    fn brightness_and_saturation_share_a_command_but_not_a_range() {
        let (bright_cmd, bright_arg) = RgbCctIntent::Brightness(0).encode(group(1)).unwrap();
        let (sat_cmd, sat_arg) = RgbCctIntent::Saturation(0).encode(group(1)).unwrap();

        assert_eq!(bright_cmd, sat_cmd, "0x04 does both");
        assert_eq!(bright_arg, 0x8F);
        assert_eq!(sat_arg, 0x0D);
    }

    #[test]
    fn kelvin_runs_from_coolest_to_warmest() {
        assert_eq!(RgbCctIntent::Kelvin(0).encode(group(1)).unwrap().1, 0x94);
        assert_eq!(RgbCctIntent::Kelvin(100).encode(group(1)).unwrap().1, 0xCC);
    }

    #[test]
    fn percentages_beyond_one_hundred_are_refused() {
        assert!(RgbCctIntent::Brightness(101).encode(group(1)).is_err());
        assert!(RgbCctIntent::Saturation(200).encode(group(1)).is_err());
        assert!(RgbCctIntent::Kelvin(255).encode(group(1)).is_err());
    }

    #[test]
    fn scenes_wrap_rather_than_overflow() {
        assert_eq!(RgbCctIntent::Scene(0).encode(group(1)).unwrap(), (0x05, 0));
        assert_eq!(RgbCctIntent::Scene(9).encode(group(1)).unwrap(), (0x05, 0));
        assert_eq!(RgbCctIntent::Scene(255).encode(group(1)).unwrap().1, 3);
    }

    #[test]
    fn names_and_arguments_describe_the_intent_for_an_audit_log() {
        assert_eq!(RgbCctIntent::Power(true).name(), "on");
        assert_eq!(RgbCctIntent::Power(true).argument(), None);
        assert_eq!(RgbCctIntent::Brightness(60).name(), "brightness");
        assert_eq!(RgbCctIntent::Brightness(60).argument(), Some(60));
        assert_eq!(RgbCctIntent::Hue(200).argument(), Some(200));
    }
}
