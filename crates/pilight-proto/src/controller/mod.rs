//! Turning intentions ("50% brightness") into packets.

mod command;
pub use command::*;

mod intent;
pub use intent::RgbCctIntent;

mod transmitter;
pub use transmitter::{LampAddress, RgbCctTransmitter};

mod rgb_cct;
pub use rgb_cct::*;
