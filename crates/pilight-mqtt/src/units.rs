//! Converting between Home Assistant's units and the protocol's.
//!
//! Three mismatches to bridge:
//!
//! * **Brightness.** HA uses 0–255; the protocol uses a 0–100 percentage.
//! * **Colour temperature.** HA speaks Kelvin; the protocol has an opaque 0–100
//!   scale running from coolest to warmest. The mapping is linear over the bulb's
//!   physical range, which is a guess at the hardware, not a protocol fact.
//! * **Saturation.** HA sends a float; the protocol wants a rounded percentage.

/// Coolest colour an RGB+CCT bulb produces, in Kelvin.
///
/// MiLight does not publish this. 2700–6500 K is the range these bulbs are sold
/// as and what other implementations assume; it is configurable for that reason.
pub const DEFAULT_MAX_KELVIN: u16 = 6500;

/// Warmest colour an RGB+CCT bulb produces, in Kelvin.
pub const DEFAULT_MIN_KELVIN: u16 = 2700;

/// Home Assistant's brightness scale.
pub const HA_BRIGHTNESS_SCALE: u8 = 255;

/// The Kelvin range a lamp is assumed to cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KelvinRange {
    /// Warmest, in Kelvin.
    pub min: u16,
    /// Coolest, in Kelvin.
    pub max: u16,
}

impl Default for KelvinRange {
    fn default() -> Self {
        Self {
            min: DEFAULT_MIN_KELVIN,
            max: DEFAULT_MAX_KELVIN,
        }
    }
}

impl KelvinRange {
    /// Build a range, putting the two values in the right order.
    #[must_use]
    pub fn new(min: u16, max: u16) -> Self {
        Self {
            min: min.min(max),
            max: min.max(max),
        }
    }

    /// How wide the range is. At least 1, so the conversions never divide by zero.
    #[must_use]
    const fn span(self) -> u32 {
        let span = self.max as u32 - self.min as u32;
        if span == 0 { 1 } else { span }
    }

    /// Convert Kelvin into the protocol's 0–100 scale, where 0 is coolest.
    #[must_use]
    pub fn kelvin_to_percent(self, kelvin: u16) -> u8 {
        let clamped = u32::from(kelvin.clamp(self.min, self.max));
        // 0% is the coolest end, so distance is measured down from `max`.
        let from_cool = u32::from(self.max) - clamped;

        // Round to nearest rather than truncating: with a 3800 K span, a whole
        // percent is 38 K, and always rounding down loses a step at every boundary.
        let percent = (from_cool * 100 + self.span() / 2) / self.span();

        u8::try_from(percent.min(100)).unwrap_or(100)
    }

    /// Convert the protocol's 0–100 scale back into Kelvin.
    #[must_use]
    pub fn percent_to_kelvin(self, percent: u8) -> u16 {
        let percent = u32::from(percent.min(100));
        let from_cool = (percent * self.span() + 50) / 100;
        let kelvin = u32::from(self.max) - from_cool;

        u16::try_from(kelvin)
            .unwrap_or(self.min)
            .clamp(self.min, self.max)
    }
}

/// Convert Home Assistant's 0–255 brightness into a 0–100 percentage.
#[must_use]
pub fn brightness_to_percent(brightness: u8) -> u8 {
    // Round to nearest so that 255 is exactly 100 and 128 is 50, not 50.19 → 50.
    let percent = (u32::from(brightness) * 100 + u32::from(HA_BRIGHTNESS_SCALE) / 2)
        / u32::from(HA_BRIGHTNESS_SCALE);

    u8::try_from(percent.min(100)).unwrap_or(100)
}

/// Convert a 0–100 percentage into Home Assistant's 0–255 brightness.
#[must_use]
pub fn percent_to_brightness(percent: u8) -> u8 {
    let brightness = (u32::from(percent.min(100)) * u32::from(HA_BRIGHTNESS_SCALE) + 50) / 100;

    u8::try_from(brightness.min(u32::from(HA_BRIGHTNESS_SCALE))).unwrap_or(HA_BRIGHTNESS_SCALE)
}

/// Round Home Assistant's float saturation or hue into the protocol's integer.
///
/// The cast is guarded: NaN, negatives and anything at or above 100 are handled
/// before it, so the value reaching it is finite and inside `(0, 100)`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[must_use]
pub fn round_percent(value: f64) -> u8 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    if value >= 100.0 {
        return 100;
    }

    value.round() as u8
}

/// Round Home Assistant's float hue into degrees, wrapping a full turn.
///
/// The cast is guarded: NaN and negatives are handled before it, and
/// `rem_euclid` bounds the rest to `[0, 360]`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[must_use]
pub fn round_hue(value: f64) -> u16 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }

    let wrapped = value.rem_euclid(360.0).round();

    // `wrapped` is in [0, 360] after rounding, so this cannot overflow.
    (wrapped as u16) % 360
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_round_trips_at_the_ends() {
        assert_eq!(brightness_to_percent(0), 0);
        assert_eq!(brightness_to_percent(255), 100);
        assert_eq!(percent_to_brightness(0), 0);
        assert_eq!(percent_to_brightness(100), 255);
    }

    #[test]
    fn brightness_rounds_to_nearest_rather_than_truncating() {
        // 128/255 is 50.196%: truncating gives 50, which is also nearest here,
        // but 3/255 is 1.18% and 2/255 is 0.78% — the latter must not become 0.
        assert_eq!(brightness_to_percent(128), 50);
        assert_eq!(brightness_to_percent(2), 1);
        assert_eq!(percent_to_brightness(50), 128);
    }

    #[test]
    fn brightness_survives_a_round_trip_within_one_percent() {
        for percent in 0..=100u8 {
            let back = brightness_to_percent(percent_to_brightness(percent));
            assert_eq!(back, percent, "{percent}% did not survive the round trip");
        }
    }

    #[test]
    fn kelvin_maps_cool_to_zero_and_warm_to_one_hundred() {
        let range = KelvinRange::default();

        assert_eq!(range.kelvin_to_percent(6500), 0, "coolest is 0%");
        assert_eq!(range.kelvin_to_percent(2700), 100, "warmest is 100%");
        assert_eq!(range.percent_to_kelvin(0), 6500);
        assert_eq!(range.percent_to_kelvin(100), 2700);
    }

    #[test]
    fn kelvin_outside_the_range_is_clamped_not_wrapped() {
        let range = KelvinRange::default();

        assert_eq!(range.kelvin_to_percent(9000), 0);
        assert_eq!(range.kelvin_to_percent(1000), 100);
    }

    #[test]
    fn kelvin_survives_a_round_trip_within_one_percent() {
        let range = KelvinRange::default();

        for percent in 0..=100u8 {
            let back = range.kelvin_to_percent(range.percent_to_kelvin(percent));
            assert_eq!(back, percent, "{percent}% did not survive the round trip");
        }
    }

    #[test]
    fn a_degenerate_kelvin_range_does_not_divide_by_zero() {
        let range = KelvinRange::new(4000, 4000);

        assert_eq!(range.kelvin_to_percent(4000), 0);
        assert_eq!(range.percent_to_kelvin(50), 4000);
    }

    #[test]
    fn a_reversed_kelvin_range_is_put_back_in_order() {
        let range = KelvinRange::new(6500, 2700);
        assert_eq!(range, KelvinRange::default());
    }

    #[test]
    fn floats_from_home_assistant_are_rounded_and_clamped() {
        assert_eq!(round_percent(29.412), 29);
        assert_eq!(round_percent(29.6), 30);
        assert_eq!(round_percent(-5.0), 0);
        assert_eq!(round_percent(500.0), 100);
        assert_eq!(round_percent(f64::NAN), 0, "NaN must not become garbage");
    }

    #[test]
    fn hues_wrap_at_a_full_turn() {
        assert_eq!(round_hue(344.0), 344);
        assert_eq!(round_hue(359.7), 0, "359.7 rounds to 360, which is 0");
        assert_eq!(round_hue(360.0), 0);
        assert_eq!(round_hue(400.0), 40);
        assert_eq!(round_hue(f64::INFINITY), 0);
    }
}
