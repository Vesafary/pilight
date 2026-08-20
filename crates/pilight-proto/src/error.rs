//! Error type shared by every layer of the driver.

use std::error::Error as StdError;
use std::fmt;

/// Anything that can go wrong between "set the brightness" and "bytes on the air".
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A packet was longer than the PL1167 frame buffer can hold.
    PayloadTooLong {
        /// Length that was offered.
        len: usize,
        /// Largest length the frame buffer accepts.
        max: usize,
    },
    /// A received frame was too short to contain a length byte and a CRC.
    FrameTooShort {
        /// Length that was received.
        len: usize,
    },
    /// A received frame's CRC did not match the payload.
    CrcMismatch {
        /// CRC computed over the received bytes.
        expected: u16,
        /// CRC carried by the frame.
        found: u16,
    },
    /// A received frame's length byte disagreed with how many bytes arrived.
    LengthMismatch {
        /// Length byte from the frame.
        declared: usize,
        /// Bytes actually present.
        found: usize,
    },
    /// A percentage argument was outside `0..=100`.
    PercentageOutOfRange(u8),
    /// A group id was outside `0..=max` for this remote type.
    GroupOutOfRange {
        /// Group that was requested.
        group: u8,
        /// Highest group this remote type supports.
        max: u8,
    },
    /// A channel number the nRF24 cannot tune to (its register is 7 bits, max 125).
    ChannelOutOfRange(u8),
    /// A required builder field was never set.
    MissingConfiguration(&'static str),
    /// The radio backend failed.
    Radio(Box<dyn StdError + Send + Sync>),
}

impl Error {
    /// Wrap a backend error that has no useful type of its own.
    pub fn radio(message: impl Into<String>) -> Self {
        Error::Radio(message.into().into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::PayloadTooLong { len, max } => {
                write!(
                    f,
                    "payload of {len} bytes exceeds the {max}-byte frame buffer"
                )
            }
            Error::FrameTooShort { len } => {
                write!(
                    f,
                    "frame of {len} bytes is too short to hold a length byte and a CRC"
                )
            }
            Error::CrcMismatch { expected, found } => {
                write!(
                    f,
                    "CRC mismatch: computed {expected:#06X}, frame carried {found:#06X}"
                )
            }
            Error::LengthMismatch { declared, found } => {
                write!(f, "frame declares {declared} bytes but carries {found}")
            }
            Error::PercentageOutOfRange(value) => {
                write!(f, "{value} is not a percentage in 0..=100")
            }
            Error::GroupOutOfRange { group, max } => {
                write!(
                    f,
                    "group {group} is out of range for a remote with {max} groups"
                )
            }
            Error::ChannelOutOfRange(channel) => {
                write!(f, "channel {channel} is outside the nRF24's 0..=125 range")
            }
            Error::MissingConfiguration(field) => {
                write!(f, "required setting `{field}` was never provided")
            }
            Error::Radio(source) => write!(f, "radio backend failed: {source}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Error::Radio(source) => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;
