//! The layer between "turn the couch lamp blue" and light.
//!
//! [`LampService`] owns the one thing that cannot be shared — the radio — and
//! sequences everything around it: look the lamp up, take a sequence number,
//! transmit on a blocking thread, revise the stored state, record the attempt.
//! The MQTT bridge and the HTTP API are both thin skins over it.

#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::module_name_repetitions
)]

mod change;
pub mod error;
mod event;
mod lamp_service;

pub use change::StateChange;
pub use error::{Result, ServiceError};
pub use event::{EVENT_BUFFER, LampEvent};
pub use lamp_service::{DEFAULT_COMMAND_GAP, LampService, LampWithState, state_update_for};
