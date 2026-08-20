//! HTTP API for `pilight`.
//!
//! Registering lamps and driving them directly, for anything that is not Home
//! Assistant — a dashboard, a script, `curl`.
//!
//! # Resources
//!
//! | Method | Path | Does |
//! |---|---|---|
//! | `GET` | `/health` | Liveness. No token needed. |
//! | `GET` | `/api/v1/lamp-types` | Which bulb families exist. |
//! | `GET` | `/api/v1/lamps` | Every lamp with its state. |
//! | `POST` | `/api/v1/lamps` | Register a lamp. |
//! | `GET` | `/api/v1/lamps/{id}` | One lamp. |
//! | `PATCH` | `/api/v1/lamps/{id}` | Rename, or move room. |
//! | `DELETE` | `/api/v1/lamps/{id}` | Forget a lamp. |
//! | `PUT` | `/api/v1/lamps/{id}/state` | Change what it is doing. |
//! | `GET` | `/api/v1/lamps/{id}/history` | What we sent, and whether it worked. |
//! | `POST` | `/api/v1/lamps/{id}/pair` | Adopt a power-cycled bulb. |
//! | `POST` | `/api/v1/lamps/{id}/unpair` | Factory-reset a power-cycled bulb. |
//!
//! # Envelope
//!
//! Every response has the same shape:
//!
//! ```json
//! { "success": true,  "data": { … }, "error": null }
//! { "success": false, "data": null,  "error": "no lamp with id …" }
//! ```
//!
//! # A warning about truth
//!
//! The state this API reports is what we last *told* a bulb. MiLight bulbs never
//! acknowledge anything and cannot be queried, so it drifts the moment someone
//! picks up a physical remote.

#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::module_name_repetitions
)]

mod app_state;
pub mod auth;
pub mod dto;
pub mod error;
pub mod response;
pub mod routes;

pub use app_state::AppState;
pub use auth::ApiToken;
pub use error::ApiError;
pub use response::{ApiResponse, PageMeta, Pagination};
pub use routes::{API_PREFIX, app};
