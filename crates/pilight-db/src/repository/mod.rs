//! Storage interfaces and their Postgres implementations.
//!
//! Each repository is a trait plus a `Pg`-prefixed implementation. The traits are
//! object-safe (via `async_trait`) so the coming axum and MQTT layers can hold an
//! `Arc<dyn LampRepository>` and be tested against a fake.

mod command_log;
mod lamp;
mod lamp_type;
mod state;

pub use command_log::{CommandLogRepository, PgCommandLogRepository};
pub use lamp::{LampRepository, PgLampRepository};
pub use lamp_type::{LampTypeRepository, PgLampTypeRepository};
pub use state::{LampStateRepository, PgLampStateRepository};

use crate::pool::Pool;

/// Every repository, sharing one pool.
///
/// A convenience for wiring: the server can hold one of these in its state rather
/// than four separate handles.
#[derive(Debug, Clone)]
pub struct Repositories {
    /// Lamps.
    pub lamps: PgLampRepository,
    /// Lamp state.
    pub states: PgLampStateRepository,
    /// Bulb families.
    pub types: PgLampTypeRepository,
    /// The command audit trail.
    pub commands: PgCommandLogRepository,
}

impl Repositories {
    /// Build every repository over `pool`.
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self {
            lamps: PgLampRepository::new(pool.clone()),
            states: PgLampStateRepository::new(pool.clone()),
            types: PgLampTypeRepository::new(pool.clone()),
            commands: PgCommandLogRepository::new(pool),
        }
    }
}
