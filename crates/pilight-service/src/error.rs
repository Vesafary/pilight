//! Errors from the service layer.

use uuid::Uuid;

/// What can go wrong turning an intent into light.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServiceError {
    /// No lamp with that id is registered.
    #[error("no lamp with id {0}")]
    LampNotFound(Uuid),

    /// The lamp exists but belongs to a family the driver cannot speak to.
    #[error("{remote_type} lamps are documented but not yet drivable")]
    UnsupportedFamily {
        /// The family in question.
        remote_type: String,
    },

    /// The command itself is not valid — an out-of-range percentage, say.
    ///
    /// Distinct from [`ServiceError::Radio`] because it is the caller's mistake,
    /// and because it is caught before anything reaches the air.
    #[error("{0}")]
    InvalidCommand(pilight_proto::Error),

    /// The radio refused the command.
    #[error("radio failed: {0}")]
    Radio(#[from] pilight_proto::Error),

    /// Storage failed.
    #[error("database failed: {0}")]
    Db(#[from] pilight_db::DbError),

    /// The blocking transmit task was cancelled or panicked.
    #[error("the transmit task did not finish: {0}")]
    TransmitTask(#[from] tokio::task::JoinError),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, ServiceError>;
