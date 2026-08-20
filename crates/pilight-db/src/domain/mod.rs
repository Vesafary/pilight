//! Domain types.
//!
//! These are what the rest of the application works with: `u8`/`u16` where the
//! protocol uses them, enums where the database stores text, and no Diesel traits.
//! The [`crate::models`] module holds the row-shaped mirrors, and the repositories
//! convert between the two.

mod bulb_mode;
mod command;
mod lamp;
mod state;

pub use bulb_mode::BulbMode;
pub use command::{CommandSource, LampCommand, NewLampCommand};
pub use lamp::{Lamp, LampUpdate, NewLamp};
pub use state::{LampState, LampStateUpdate};
