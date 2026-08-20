//! Rows of `lamp_commands`.

use crate::domain::{CommandSource, LampCommand, NewLampCommand};
use crate::error::Result;
use crate::schema::lamp_commands;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use uuid::Uuid;

/// A row of `lamp_commands`.
#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = lamp_commands)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct LampCommandRow {
    /// Primary key.
    pub id: i64,
    /// Which lamp it was aimed at.
    pub lamp_id: Uuid,
    /// Who asked, as stored text.
    pub source: String,
    /// What was asked for.
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

impl TryFrom<LampCommandRow> for LampCommand {
    type Error = crate::error::DbError;

    fn try_from(row: LampCommandRow) -> Result<Self> {
        Ok(Self {
            id: row.id,
            lamp_id: row.lamp_id,
            source: CommandSource::parse(&row.source)?,
            command: row.command,
            argument: row.argument,
            succeeded: row.succeeded,
            error: row.error,
            created_at: row.created_at,
        })
    }
}

/// A row to insert into `lamp_commands`.
#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = lamp_commands)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewLampCommandRow {
    /// Which lamp it was aimed at.
    pub lamp_id: Uuid,
    /// Who asked.
    pub source: String,
    /// What was asked for.
    pub command: String,
    /// The argument, where the command takes one.
    pub argument: Option<i32>,
    /// Whether the radio accepted it.
    pub succeeded: bool,
    /// Why it failed, if it did.
    pub error: Option<String>,
}

impl From<&NewLampCommand> for NewLampCommandRow {
    fn from(command: &NewLampCommand) -> Self {
        Self {
            lamp_id: command.lamp_id,
            source: command.source.as_str().to_owned(),
            command: command.command.clone(),
            argument: command.argument,
            // The `error_iff_failed` constraint ties these two together, so derive
            // one from the other rather than trusting a caller to keep them in step.
            succeeded: command.is_success(),
            error: command.error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_successful_command_carries_no_error() {
        let row = NewLampCommandRow::from(&NewLampCommand::succeeded(
            Uuid::nil(),
            CommandSource::Api,
            "on",
            None,
        ));

        assert!(row.succeeded);
        assert_eq!(row.error, None);
    }

    #[test]
    fn a_failed_command_always_carries_one() {
        // The schema's error_iff_failed CHECK would reject any other combination.
        let row = NewLampCommandRow::from(&NewLampCommand::failed(
            Uuid::nil(),
            CommandSource::Cli,
            "brightness",
            Some(60),
            "radio busy",
        ));

        assert!(!row.succeeded);
        assert_eq!(row.error.as_deref(), Some("radio busy"));
    }

    #[test]
    fn rows_convert_back_into_the_domain() {
        let row = LampCommandRow {
            id: 7,
            lamp_id: Uuid::nil(),
            source: "mqtt".into(),
            command: "hue".into(),
            argument: Some(200),
            succeeded: true,
            error: None,
            created_at: Utc::now(),
        };
        let command = LampCommand::try_from(row).unwrap();

        assert_eq!(command.source, CommandSource::Mqtt);
        assert_eq!(command.argument, Some(200));
    }

    #[test]
    fn an_unknown_source_is_an_error() {
        let row = LampCommandRow {
            id: 1,
            lamp_id: Uuid::nil(),
            source: "telepathy".into(),
            command: "on".into(),
            argument: None,
            succeeded: true,
            error: None,
            created_at: Utc::now(),
        };
        assert!(LampCommand::try_from(row).is_err());
    }
}
