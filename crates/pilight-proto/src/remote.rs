//! The bulb families this protocol covers.
//!
//! A "remote type" is really a bulb family: it fixes the radio parameters, the
//! packet generation, how many groups can be addressed, and — for V2 families —
//! the protocol id byte that identifies the family inside the packet.
//!
//! See `docs/protocol.md` §2.3 and §5.

use crate::packet::{PROTOCOL_ID_FUT089, PROTOCOL_ID_FUT091, PROTOCOL_ID_RGB_CCT};
use crate::radio::RadioConfig;

/// A MiLight bulb family, named after the remote that ships with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum RemoteType {
    /// RGBW bulbs (FUT096). V1 protocol.
    Rgbw,
    /// Tunable-white bulbs (FUT007). V1 protocol.
    Cct,
    /// RGB+CCT bulbs (FUT092). V2 protocol — the family `pilight` drives.
    RgbCct,
    /// RGB-only bulbs (FUT098). V1 protocol.
    Rgb,
    /// Eight-group RGB+CCT panel (FUT089 / B8). V2 protocol.
    Fut089,
    /// Tunable-white bulbs on the V2 protocol (FUT091).
    Fut091,
    /// FUT020 groupless remotes. V1 protocol.
    Fut020,
}

impl RemoteType {
    /// Every family, in a stable order. The index doubles as a database id.
    pub const ALL: [Self; 7] = [
        Self::Rgbw,
        Self::Cct,
        Self::RgbCct,
        Self::Rgb,
        Self::Fut089,
        Self::Fut091,
        Self::Fut020,
    ];

    /// A stable machine-readable identifier, used as the database and MQTT key.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Rgbw => "rgbw",
            Self::Cct => "cct",
            Self::RgbCct => "rgb_cct",
            Self::Rgb => "rgb",
            Self::Fut089 => "fut089",
            Self::Fut091 => "fut091",
            Self::Fut020 => "fut020",
        }
    }

    /// A human-readable name, including the remote's model number.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Rgbw => "RGBW (FUT096)",
            Self::Cct => "CCT (FUT007)",
            Self::RgbCct => "RGB+CCT (FUT092)",
            Self::Rgb => "RGB (FUT098)",
            Self::Fut089 => "RGB+CCT 8-group (FUT089)",
            Self::Fut091 => "CCT V2 (FUT091)",
            Self::Fut020 => "FUT020",
        }
    }

    /// Parse a [`RemoteType::slug`].
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.slug() == slug)
    }

    /// The radio parameters this family transmits on.
    ///
    /// Several families share one config — RGB+CCT, FUT089 and FUT091 all use the
    /// same syncword and channels, and are told apart by their protocol id.
    #[must_use]
    pub const fn radio_config(self) -> RadioConfig {
        match self {
            Self::Rgbw => RadioConfig::RGBW,
            Self::Cct => RadioConfig::CCT,
            Self::RgbCct | Self::Fut089 | Self::Fut091 => RadioConfig::RGB_CCT,
            Self::Rgb => RadioConfig::RGB,
            Self::Fut020 => RadioConfig::FUT020,
        }
    }

    /// Which packet generation this family speaks: 1 (plaintext) or 2 (obfuscated).
    #[must_use]
    pub const fn protocol_generation(self) -> u8 {
        match self {
            Self::RgbCct | Self::Fut089 | Self::Fut091 => 2,
            _ => 1,
        }
    }

    /// The protocol id byte carried at index 1 of a V2 packet.
    ///
    /// `None` for V1 families, which have no such field.
    #[must_use]
    pub const fn protocol_id(self) -> Option<u8> {
        match self {
            Self::RgbCct => Some(PROTOCOL_ID_RGB_CCT),
            Self::Fut091 => Some(PROTOCOL_ID_FUT091),
            Self::Fut089 => Some(PROTOCOL_ID_FUT089),
            _ => None,
        }
    }

    /// How many addressable groups this family has. Zero means the remote is
    /// groupless and controls one zone.
    #[must_use]
    pub const fn num_groups(self) -> u8 {
        match self {
            Self::Rgbw | Self::Cct | Self::RgbCct | Self::Fut091 => 4,
            Self::Fut089 => 8,
            Self::Rgb | Self::Fut020 => 0,
        }
    }

    /// Whether `pilight` can currently drive this family.
    ///
    /// The protocol is documented for all of them; only RGB+CCT has a command
    /// layer so far.
    #[must_use]
    pub const fn is_driver_supported(self) -> bool {
        matches!(self, Self::RgbCct)
    }
}

impl std::fmt::Display for RemoteType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.slug())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_unique_and_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for kind in RemoteType::ALL {
            assert!(seen.insert(kind.slug()), "duplicate slug {}", kind.slug());
            assert_eq!(RemoteType::from_slug(kind.slug()), Some(kind));
        }
    }

    #[test]
    fn unknown_slugs_are_rejected() {
        assert_eq!(RemoteType::from_slug("fut999"), None);
        assert_eq!(RemoteType::from_slug(""), None);
    }

    #[test]
    fn only_v2_families_carry_a_protocol_id() {
        for kind in RemoteType::ALL {
            assert_eq!(
                kind.protocol_id().is_some(),
                kind.protocol_generation() == 2,
                "{kind} disagrees about its generation"
            );
        }
    }

    #[test]
    fn the_three_v2_families_share_a_radio_config_but_not_a_protocol_id() {
        let v2 = [RemoteType::RgbCct, RemoteType::Fut089, RemoteType::Fut091];

        for kind in v2 {
            assert_eq!(kind.radio_config(), RadioConfig::RGB_CCT);
        }

        let ids: Vec<Option<u8>> = v2.iter().map(|k| k.protocol_id()).collect();
        assert_eq!(ids, vec![Some(0x20), Some(0x25), Some(0x21)]);
    }

    #[test]
    fn v2_families_are_exactly_the_nine_byte_ones() {
        for kind in RemoteType::ALL {
            let is_nine_bytes = kind.radio_config().packet_len == 9;
            assert_eq!(is_nine_bytes, kind.protocol_generation() == 2, "{kind}");
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_agrees_with_the_slug() {
        // The database and MQTT topics key off slug(); serde must not invent a
        // second spelling of the same thing.
        for kind in RemoteType::ALL {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.slug()));
            assert_eq!(serde_json::from_str::<RemoteType>(&json).unwrap(), kind);
        }
    }

    #[test]
    fn groupless_families_report_zero_groups() {
        assert_eq!(RemoteType::Rgb.num_groups(), 0);
        assert_eq!(RemoteType::Fut020.num_groups(), 0);
        assert_eq!(RemoteType::Fut089.num_groups(), 8);
        assert_eq!(RemoteType::RgbCct.num_groups(), 4);
    }
}
