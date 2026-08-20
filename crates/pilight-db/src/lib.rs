//! Postgres persistence for `pilight`: lamps, their families, and their state.
//!
//! # Shape
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`domain`] | What the application works with: `u8`s, enums, no Diesel |
//! | [`models`] | Row-shaped mirrors of the tables, plus the conversions |
//! | [`repository`] | Storage traits and their Postgres implementations |
//! | [`pool`] | Connection pooling and migrations |
//! | [`schema`] | Diesel's generated table definitions |
//!
//! # A warning about truth
//!
//! MiLight bulbs never acknowledge anything and cannot be queried. Everything in
//! `lamp_states` is what we last *told* a bulb, not what it is doing. It goes stale
//! the moment someone picks up a physical remote. Treat it as a cache of intent.
//!
//! # Getting started
//!
//! ```no_run
//! use pilight_db::{Repositories, build_pool, run_migrations};
//! use pilight_db::repository::LampTypeRepository;
//!
//! # async fn example() -> Result<(), pilight_db::DbError> {
//! let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
//!
//! let pool = build_pool(&url)?;
//! run_migrations(&pool).await?;
//!
//! let repos = Repositories::new(pool);
//! repos.types.sync_from_driver().await?;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::module_name_repetitions
)]

pub mod domain;
pub mod error;
pub mod models;
pub mod pool;
pub mod repository;
/// Diesel's table definitions, generated from the live schema by
/// `diesel print-schema`. Regenerate after every migration; do not hand-edit.
#[rustfmt::skip]
#[allow(missing_docs, clippy::missing_docs_in_private_items)]
pub mod schema;

pub use domain::{
    BulbMode, CommandSource, Lamp, LampCommand, LampState, LampStateUpdate, LampUpdate, NewLamp,
    NewLampCommand,
};
pub use error::{DbError, Result};
pub use pool::{
    MIGRATIONS, Pool, build_pool, build_pool_with_size, revert_migrations, run_migrations,
};
pub use repository::Repositories;

/// Re-exported so callers do not need a direct `pilight-proto` dependency just to
/// name a bulb family.
pub use pilight_proto::RemoteType;
