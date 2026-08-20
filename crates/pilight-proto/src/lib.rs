//! A MiLight / LimitlessLED bulb driver for the Raspberry Pi.
//!
//! Talks to RGB+CCT bulbs directly over 2.4 GHz with an nRF24L01+, replacing the
//! vendor's WiFi gateway. The on-air protocol is undocumented and community
//! reverse-engineered; `docs/protocol.md` in this repository is the full write-up.
//!
//! # Layers
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`encoder`] | V2 obfuscation — the keyed XOR and position offsets |
//! | [`packet`] | The 9-byte V2 packet and its fields |
//! | [`framing`] | PL1167 framing: length byte, CRC-16, bit reversal |
//! | [`radio`] | Channel hopping, burst repetition, the nRF24 backend |
//! | [`remote`] | The bulb families and their parameters |
//! | [`controller`] | Commands: on/off, hue, brightness, kelvin, scenes |
//!
//! # Example
//!
//! With hardware (needs the default `nrf24` feature):
//!
//! ```ignore
//! use pilight_proto::{GroupId, Nrf24Transceiver, RgbCctController};
//!
//! let radio = Nrf24Transceiver::open()?;
//! let mut lamp = RgbCctController::builder(radio)
//!     .device_id(0xBEEF)
//!     .group(GroupId::new(1, 4)?)
//!     .build()?;
//!
//! lamp.on()?;
//! lamp.set_brightness(60)?;
//! lamp.set_hue(200)?;
//! ```
//!
//! Building a frame without any hardware at all:
//!
//! ```
//! use pilight_proto::{Frame, PROTOCOL_ID_RGB_CCT, V2Packet};
//!
//! // "group 1 on", from device 0xBEEF
//! let packet = V2Packet::new(0x00, PROTOCOL_ID_RGB_CCT, 0xBEEF, 0x01, 0x01, 0x00, 0x01);
//! let frame = Frame::build(&packet.to_encoded())?;
//!
//! assert_eq!(frame.len(), 12); // length byte + 9 packet bytes + CRC
//! # Ok::<(), pilight_proto::Error>(())
//! ```
//!
//! # Caveats
//!
//! Nothing is ever acknowledged. There is no read-back from the bulbs, so any state
//! you display is state you are tracking yourself, and it drifts the moment someone
//! picks up a physical remote.

#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::doc_markdown
)]

pub mod controller;
pub mod encoder;
pub mod error;
pub mod framing;
pub mod packet;
pub mod radio;
pub mod remote;

pub use controller::*;
pub use encoder::*;
pub use error::{Error, Result};
pub use framing::*;
pub use packet::*;
pub use radio::*;
pub use remote::RemoteType;
