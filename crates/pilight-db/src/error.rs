//! Errors from the persistence layer.

use uuid::Uuid;

/// Anything that can go wrong talking to Postgres, or converting what it returns.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DbError {
    /// The row exists but a column holds a value the domain type cannot represent.
    ///
    /// Postgres has no unsigned integers and no sum types, so the schema's `CHECK`
    /// constraints are the only thing keeping these columns in range. This is what
    /// a violated assumption looks like on the way out.
    #[error("column `{column}` holds {value}, which is not a valid {expected}")]
    InvalidStoredValue {
        /// Which column.
        column: &'static str,
        /// What was in it.
        value: String,
        /// What the domain expected.
        expected: &'static str,
    },

    /// No lamp with that id.
    #[error("no lamp with id {0}")]
    LampNotFound(Uuid),

    /// A lamp already exists at that (family, device id, group) address.
    #[error("a lamp is already registered at {remote_type}/{device_id:#06X}/{group}")]
    DuplicateAddress {
        /// The family.
        remote_type: String,
        /// The device id.
        device_id: u16,
        /// The group.
        group: u8,
    },

    /// A value failed validation before it ever reached the database.
    #[error("{0}")]
    Invalid(String),

    /// The driver rejected a value — an out-of-range group, say.
    #[error("protocol error: {0}")]
    Protocol(#[from] pilight_proto::Error),

    /// A query failed.
    #[error("database query failed")]
    Query(#[from] diesel::result::Error),

    /// A connection could not be obtained from the pool.
    #[error("could not get a database connection")]
    Pool(#[from] diesel_async::pooled_connection::deadpool::PoolError),

    /// The pool could not be built.
    #[error("could not build the database pool")]
    BuildPool(#[from] diesel_async::pooled_connection::deadpool::BuildError),

    /// Connecting to Postgres failed.
    #[error("could not connect to postgres: {0}")]
    Connection(#[from] diesel::ConnectionError),

    /// Running migrations failed.
    #[error("running migrations failed: {0}")]
    Migration(String),
}

impl DbError {
    /// Helper for the many `i16`/`i32` to domain conversions.
    pub(crate) fn invalid_value(
        column: &'static str,
        value: impl std::fmt::Display,
        expected: &'static str,
    ) -> Self {
        Self::InvalidStoredValue {
            column,
            value: value.to_string(),
            expected,
        }
    }
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, DbError>;
