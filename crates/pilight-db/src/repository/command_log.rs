//! The append-only record of what we transmitted.

use crate::domain::{LampCommand, NewLampCommand};
use crate::error::Result;
use crate::models::{LampCommandRow, NewLampCommandRow};
use crate::pool::Pool;
use crate::schema::lamp_commands;
use async_trait::async_trait;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use uuid::Uuid;

/// How many rows [`CommandLogRepository::recent_for_lamp`] returns if not told.
pub const DEFAULT_HISTORY_LIMIT: i64 = 50;

/// Storage for the command audit trail.
#[async_trait]
pub trait CommandLogRepository: Send + Sync {
    /// Record one attempted command.
    async fn record(&self, command: NewLampCommand) -> Result<LampCommand>;

    /// The most recent commands for a lamp, newest first.
    async fn recent_for_lamp(&self, lamp_id: Uuid, limit: Option<i64>) -> Result<Vec<LampCommand>>;

    /// Delete records older than `before`. Returns how many went.
    async fn prune(&self, before: chrono::DateTime<chrono::Utc>) -> Result<usize>;
}

/// Postgres-backed [`CommandLogRepository`].
#[derive(Clone)]
pub struct PgCommandLogRepository {
    pool: Pool,
}

impl std::fmt::Debug for PgCommandLogRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // deadpool's Pool is only Debug when the connection is, and
        // AsyncPgConnection is not. Report what is actually useful instead.
        f.debug_struct("PgCommandLogRepository")
            .field("pool_status", &self.pool.status())
            .finish()
    }
}

impl PgCommandLogRepository {
    /// Wrap a connection pool.
    #[must_use]
    pub const fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CommandLogRepository for PgCommandLogRepository {
    async fn record(&self, command: NewLampCommand) -> Result<LampCommand> {
        let mut conn = self.pool.get().await?;

        let row: LampCommandRow = diesel::insert_into(lamp_commands::table)
            .values(NewLampCommandRow::from(&command))
            .returning(LampCommandRow::as_returning())
            .get_result(&mut conn)
            .await?;

        LampCommand::try_from(row)
    }

    async fn recent_for_lamp(&self, lamp_id: Uuid, limit: Option<i64>) -> Result<Vec<LampCommand>> {
        let mut conn = self.pool.get().await?;

        let rows: Vec<LampCommandRow> = lamp_commands::table
            .filter(lamp_commands::lamp_id.eq(lamp_id))
            .select(LampCommandRow::as_select())
            .order(lamp_commands::created_at.desc())
            .limit(limit.unwrap_or(DEFAULT_HISTORY_LIMIT).max(1))
            .load(&mut conn)
            .await?;

        rows.into_iter().map(LampCommand::try_from).collect()
    }

    async fn prune(&self, before: chrono::DateTime<chrono::Utc>) -> Result<usize> {
        let mut conn = self.pool.get().await?;

        Ok(
            diesel::delete(lamp_commands::table.filter(lamp_commands::created_at.lt(before)))
                .execute(&mut conn)
                .await?,
        )
    }
}
