//! Errors from the MQTT bridge.

/// What can go wrong bridging to Home Assistant.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MqttError {
    /// The broker connection failed.
    #[error("mqtt client error: {0}")]
    Client(#[from] rumqttc::ClientError),

    /// A payload could not be built or parsed.
    #[error("could not encode an mqtt payload: {0}")]
    Encode(#[from] serde_json::Error),

    /// Storage failed while answering a command.
    #[error("database failed: {0}")]
    Db(#[from] pilight_db::DbError),

    /// The service failed while applying a command.
    #[error("service failed: {0}")]
    Service(#[from] pilight_service::ServiceError),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, MqttError>;
