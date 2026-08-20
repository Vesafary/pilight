//! Reading and revising what we believe a lamp is doing.

use crate::domain::{LampState, LampStateUpdate};
use crate::error::{DbError, Result};
use crate::models::{LampStateChangeset, LampStateRow};
use crate::pool::Pool;
use crate::schema::lamp_states;
use async_trait::async_trait;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use uuid::Uuid;

/// Storage for lamp state.
#[async_trait]
pub trait LampStateRepository: Send + Sync {
    /// The state of one lamp.
    async fn find_by_lamp(&self, lamp_id: Uuid) -> Result<Option<LampState>>;

    /// The state of every lamp.
    async fn find_all(&self) -> Result<Vec<LampState>>;

    /// Revise what we believe about a lamp.
    async fn update(&self, lamp_id: Uuid, changes: LampStateUpdate) -> Result<LampState>;

    /// Take the next V2 sequence byte, advancing the stored counter.
    ///
    /// Atomic, so two concurrent senders — the API and an MQTT handler, say —
    /// cannot hand the same number to the same bulb.
    async fn take_sequence(&self, lamp_id: Uuid) -> Result<u8>;
}

/// Postgres-backed [`LampStateRepository`].
#[derive(Clone)]
pub struct PgLampStateRepository {
    pool: Pool,
}

impl std::fmt::Debug for PgLampStateRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // deadpool's Pool is only Debug when the connection is, and
        // AsyncPgConnection is not. Report what is actually useful instead.
        f.debug_struct("PgLampStateRepository")
            .field("pool_status", &self.pool.status())
            .finish()
    }
}

impl PgLampStateRepository {
    /// Wrap a connection pool.
    #[must_use]
    pub const fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl LampStateRepository for PgLampStateRepository {
    async fn find_by_lamp(&self, lamp_id: Uuid) -> Result<Option<LampState>> {
        let mut conn = self.pool.get().await?;

        let row: Option<LampStateRow> = lamp_states::table
            .find(lamp_id)
            .select(LampStateRow::as_select())
            .first(&mut conn)
            .await
            .optional()?;

        row.map(LampState::try_from).transpose()
    }

    async fn find_all(&self) -> Result<Vec<LampState>> {
        let mut conn = self.pool.get().await?;

        let rows: Vec<LampStateRow> = lamp_states::table
            .select(LampStateRow::as_select())
            .load(&mut conn)
            .await?;

        rows.into_iter().map(LampState::try_from).collect()
    }

    async fn update(&self, lamp_id: Uuid, changes: LampStateUpdate) -> Result<LampState> {
        // Diesel would emit an `UPDATE ... SET` with an empty tail otherwise.
        if changes.is_empty() {
            return self
                .find_by_lamp(lamp_id)
                .await?
                .ok_or(DbError::LampNotFound(lamp_id));
        }

        let mut conn = self.pool.get().await?;

        let row: Option<LampStateRow> = diesel::update(lamp_states::table.find(lamp_id))
            .set(LampStateChangeset::from(&changes))
            .returning(LampStateRow::as_returning())
            .get_result(&mut conn)
            .await
            .optional()?;

        row.ok_or(DbError::LampNotFound(lamp_id))
            .and_then(LampState::try_from)
    }

    async fn take_sequence(&self, lamp_id: Uuid) -> Result<u8> {
        let mut conn = self.pool.get().await?;

        // Read-modify-write in one statement, so concurrent callers serialise on
        // the row lock rather than racing. Diesel's DSL has no remainder operator,
        // and the modulo matters: it keeps the value inside the column's CHECK
        // constraint and mirrors the protocol's wrapping u8.
        let taken: Option<i16> =
            diesel::update(lamp_states::table.find(lamp_id))
                .set(lamp_states::next_sequence.eq(
                    diesel::dsl::sql::<diesel::sql_types::SmallInt>("(next_sequence + 1) % 256"),
                ))
                .returning(lamp_states::next_sequence)
                .get_result(&mut conn)
                .await
                .optional()?;

        let next = taken.ok_or(DbError::LampNotFound(lamp_id))?;

        // `next` is what the counter became; the caller gets what it was.
        let taken = if next == 0 { 255 } else { next - 1 };

        u8::try_from(taken)
            .map_err(|_| DbError::invalid_value("next_sequence", taken, "sequence byte"))
    }
}
