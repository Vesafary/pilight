//! The audit trail of what we transmitted.

use crate::error::DbError;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Where a command came from.
///
/// Worth recording because the bulbs cannot be asked what happened: when the
/// lights are wrong, this is the only account of who told them what.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandSource {
    /// The HTTP API.
    Api,
    /// An MQTT message.
    Mqtt,
    /// The command-line tool.
    Cli,
    /// A timer or automation.
    Schedule,
    /// Observed from a physical remote rather than sent by us.
    Sniffer,
}

impl CommandSource {
    /// Every source, for exhaustive iteration.
    pub const ALL: [Self; 5] = [
        Self::Api,
        Self::Mqtt,
        Self::Cli,
        Self::Schedule,
        Self::Sniffer,
    ];

    /// The value stored in `lamp_commands.source`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Mqtt => "mqtt",
            Self::Cli => "cli",
            Self::Schedule => "schedule",
            Self::Sniffer => "sniffer",
        }
    }

    /// Parse a stored value.
    pub fn parse(value: &str) -> Result<Self, DbError> {
        Self::ALL
            .into_iter()
            .find(|source| source.as_str() == value)
            .ok_or_else(|| DbError::invalid_value("source", value, "command source"))
    }
}

impl std::fmt::Display for CommandSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One recorded transmission.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LampCommand {
    /// Primary key.
    pub id: i64,
    /// Which lamp it was aimed at.
    pub lamp_id: Uuid,
    /// Who asked for it.
    pub source: CommandSource,
    /// What was asked for, e.g. `"brightness"`.
    pub command: String,
    /// The argument, where the command takes one.
    pub argument: Option<i32>,
    /// Whether the radio accepted it.
    pub succeeded: bool,
    /// Why it failed, if it did.
    pub error: Option<String>,
    /// When it was attempted.
    pub created_at: DateTime<Utc>,
}

/// A command to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLampCommand {
    /// Which lamp it was aimed at.
    pub lamp_id: Uuid,
    /// Who asked for it.
    pub source: CommandSource,
    /// What was asked for.
    pub command: String,
    /// The argument, where the command takes one.
    pub argument: Option<i32>,
    /// Why it failed. `None` means it succeeded.
    pub error: Option<String>,
}

impl NewLampCommand {
    /// Record a command that worked.
    #[must_use]
    pub fn succeeded(
        lamp_id: Uuid,
        source: CommandSource,
        command: impl Into<String>,
        argument: Option<i32>,
    ) -> Self {
        Self {
            lamp_id,
            source,
            command: command.into(),
            argument,
            error: None,
        }
    }

    /// Record a command that did not.
    #[must_use]
    pub fn failed(
        lamp_id: Uuid,
        source: CommandSource,
        command: impl Into<String>,
        argument: Option<i32>,
        error: impl std::fmt::Display,
    ) -> Self {
        Self {
            lamp_id,
            source,
            command: command.into(),
            argument,
            error: Some(error.to_string()),
        }
    }

    /// Whether this record describes a success.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.error.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_round_trips_through_its_stored_form() {
        for source in CommandSource::ALL {
            assert_eq!(CommandSource::parse(source.as_str()).unwrap(), source);
        }
    }

    #[test]
    fn an_unknown_source_is_rejected() {
        assert!(CommandSource::parse("telepathy").is_err());
    }

    #[test]
    fn success_and_failure_are_distinguished_by_the_error_field() {
        let id = Uuid::nil();
        let ok = NewLampCommand::succeeded(id, CommandSource::Api, "on", None);
        assert!(ok.is_success());
        assert_eq!(ok.error, None);

        let bad =
            NewLampCommand::failed(id, CommandSource::Mqtt, "brightness", Some(60), "no radio");
        assert!(!bad.is_success());
        assert_eq!(bad.error.as_deref(), Some("no radio"));
    }
}
