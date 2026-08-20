//! Registering and looking up lamps.

use crate::domain::{Lamp, LampUpdate, NewLamp};
use crate::error::{DbError, Result};
use crate::models::lamp_type::id_of;
use crate::models::{LampChangeset, LampRow, NewLampRow, NewLampStateRow};
use crate::pool::Pool;
use crate::schema::{lamp_states, lamps};
use async_trait::async_trait;
use diesel::prelude::*;
use diesel::result::{DatabaseErrorKind, Error as DieselError};
use diesel_async::{AsyncConnection, RunQueryDsl};
use pilight_proto::RemoteType;
use uuid::Uuid;

/// Storage for lamps.
#[async_trait]
pub trait LampRepository: Send + Sync {
    /// Every registered lamp, ordered by room then name.
    async fn find_all(&self) -> Result<Vec<Lamp>>;

    /// One lamp by primary key.
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Lamp>>;

    /// One lamp by the address a bulb actually listens on.
    async fn find_by_address(
        &self,
        remote_type: RemoteType,
        device_id: u16,
        group: u8,
    ) -> Result<Option<Lamp>>;

    /// Every lamp in a room.
    async fn find_by_room(&self, room: &str) -> Result<Vec<Lamp>>;

    /// Register a lamp, creating its state row in the same transaction.
    async fn create(&self, lamp: NewLamp) -> Result<Lamp>;

    /// Edit a lamp's name or room.
    async fn update(&self, id: Uuid, changes: LampUpdate) -> Result<Lamp>;

    /// Remove a lamp. Its state and command history go with it.
    ///
    /// Returns whether a row was actually removed.
    async fn delete(&self, id: Uuid) -> Result<bool>;
}

/// Postgres-backed [`LampRepository`].
#[derive(Clone)]
pub struct PgLampRepository {
    pool: Pool,
}

impl std::fmt::Debug for PgLampRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // deadpool's Pool is only Debug when the connection is, and
        // AsyncPgConnection is not. Report what is actually useful instead.
        f.debug_struct("PgLampRepository")
            .field("pool_status", &self.pool.status())
            .finish()
    }
}

impl PgLampRepository {
    /// Wrap a connection pool.
    #[must_use]
    pub const fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl LampRepository for PgLampRepository {
    async fn find_all(&self) -> Result<Vec<Lamp>> {
        let mut conn = self.pool.get().await?;

        let rows: Vec<LampRow> = lamps::table
            .select(LampRow::as_select())
            .order((lamps::room.asc().nulls_last(), lamps::name.asc()))
            .load(&mut conn)
            .await?;

        rows.into_iter().map(Lamp::try_from).collect()
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Lamp>> {
        let mut conn = self.pool.get().await?;

        let row: Option<LampRow> = lamps::table
            .find(id)
            .select(LampRow::as_select())
            .first(&mut conn)
            .await
            .optional()?;

        row.map(Lamp::try_from).transpose()
    }

    async fn find_by_address(
        &self,
        remote_type: RemoteType,
        device_id: u16,
        group: u8,
    ) -> Result<Option<Lamp>> {
        let mut conn = self.pool.get().await?;

        let row: Option<LampRow> = lamps::table
            .filter(lamps::lamp_type_id.eq(id_of(remote_type)))
            .filter(lamps::device_id.eq(i32::from(device_id)))
            .filter(lamps::group_id.eq(i16::from(group)))
            .select(LampRow::as_select())
            .first(&mut conn)
            .await
            .optional()?;

        row.map(Lamp::try_from).transpose()
    }

    async fn find_by_room(&self, room: &str) -> Result<Vec<Lamp>> {
        let mut conn = self.pool.get().await?;

        let rows: Vec<LampRow> = lamps::table
            .filter(lamps::room.eq(room))
            .select(LampRow::as_select())
            .order(lamps::name.asc())
            .load(&mut conn)
            .await?;

        rows.into_iter().map(Lamp::try_from).collect()
    }

    async fn create(&self, lamp: NewLamp) -> Result<Lamp> {
        lamp.validate()?;

        let mut conn = self.pool.get().await?;
        let insert = NewLampRow::from(&lamp);

        let row = conn
            .transaction::<LampRow, DieselError, _>(async |conn| {
                // A lamp without a state row would be a lamp we can never record
                // anything about, so the two are created together or not at all.
                let row: LampRow = diesel::insert_into(lamps::table)
                    .values(&insert)
                    .returning(LampRow::as_returning())
                    .get_result(conn)
                    .await?;

                diesel::insert_into(lamp_states::table)
                    .values(NewLampStateRow { lamp_id: row.id })
                    .execute(conn)
                    .await?;

                Ok(row)
            })
            .await
            .map_err(|error| duplicate_address(error, &lamp))?;

        Lamp::try_from(row)
    }

    async fn update(&self, id: Uuid, changes: LampUpdate) -> Result<Lamp> {
        changes.validate()?;

        // An empty changeset would make Diesel emit `SET` with nothing after it.
        if changes.is_empty() {
            return self.find_by_id(id).await?.ok_or(DbError::LampNotFound(id));
        }

        let mut conn = self.pool.get().await?;

        let row: Option<LampRow> = diesel::update(lamps::table.find(id))
            .set(LampChangeset::from(&changes))
            .returning(LampRow::as_returning())
            .get_result(&mut conn)
            .await
            .optional()?;

        row.ok_or(DbError::LampNotFound(id))
            .and_then(Lamp::try_from)
    }

    async fn delete(&self, id: Uuid) -> Result<bool> {
        let mut conn = self.pool.get().await?;

        let removed = diesel::delete(lamps::table.find(id))
            .execute(&mut conn)
            .await?;

        Ok(removed > 0)
    }
}

/// Turn the unique-violation on `lamps_address_key` into something actionable.
fn duplicate_address(error: DieselError, lamp: &NewLamp) -> DbError {
    match &error {
        DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, _) => {
            DbError::DuplicateAddress {
                remote_type: lamp.remote_type.to_string(),
                device_id: lamp.device_id,
                group: lamp.group,
            }
        }
        _ => DbError::Query(error),
    }
}
