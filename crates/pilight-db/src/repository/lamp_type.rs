//! The bulb-family catalogue.
//!
//! `lamp_types` is a projection of [`RemoteType::ALL`], not hand-maintained
//! reference data: [`LampTypeRepository::sync_from_driver`] upserts the whole set
//! at startup, so adding a family to the driver needs no migration.

use crate::error::Result;
use crate::models::LampTypeRow;
use crate::models::lamp_type::{NewLampTypeRow, remote_type_of};
use crate::pool::Pool;
use crate::schema::lamp_types;
use async_trait::async_trait;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use pilight_proto::RemoteType;

/// Storage for the bulb-family catalogue.
#[async_trait]
pub trait LampTypeRepository: Send + Sync {
    /// Every known family.
    async fn find_all(&self) -> Result<Vec<RemoteType>>;

    /// Every family the driver can actually speak to.
    async fn find_supported(&self) -> Result<Vec<RemoteType>>;

    /// Bring the table in line with the driver. Idempotent.
    async fn sync_from_driver(&self) -> Result<usize>;
}

/// Postgres-backed [`LampTypeRepository`].
#[derive(Clone)]
pub struct PgLampTypeRepository {
    pool: Pool,
}

impl std::fmt::Debug for PgLampTypeRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // deadpool's Pool is only Debug when the connection is, and
        // AsyncPgConnection is not. Report what is actually useful instead.
        f.debug_struct("PgLampTypeRepository")
            .field("pool_status", &self.pool.status())
            .finish()
    }
}

impl PgLampTypeRepository {
    /// Wrap a connection pool.
    #[must_use]
    pub const fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl LampTypeRepository for PgLampTypeRepository {
    async fn find_all(&self) -> Result<Vec<RemoteType>> {
        let mut conn = self.pool.get().await?;

        let rows: Vec<LampTypeRow> = lamp_types::table
            .select(LampTypeRow::as_select())
            .order(lamp_types::id.asc())
            .load(&mut conn)
            .await?;

        rows.into_iter().map(|row| remote_type_of(row.id)).collect()
    }

    async fn find_supported(&self) -> Result<Vec<RemoteType>> {
        let mut conn = self.pool.get().await?;

        let rows: Vec<LampTypeRow> = lamp_types::table
            .filter(lamp_types::driver_supported.eq(true))
            .select(LampTypeRow::as_select())
            .order(lamp_types::id.asc())
            .load(&mut conn)
            .await?;

        rows.into_iter().map(|row| remote_type_of(row.id)).collect()
    }

    async fn sync_from_driver(&self) -> Result<usize> {
        let mut conn = self.pool.get().await?;

        let rows: Vec<NewLampTypeRow> = RemoteType::ALL
            .into_iter()
            .map(NewLampTypeRow::from_remote_type)
            .collect();

        // Upsert rather than insert: the driver's view of a family can change
        // (a family becomes supported), and startup must be repeatable.
        let written = diesel::insert_into(lamp_types::table)
            .values(&rows)
            .on_conflict(lamp_types::id)
            .do_update()
            .set((
                lamp_types::slug.eq(diesel::upsert::excluded(lamp_types::slug)),
                lamp_types::display_name.eq(diesel::upsert::excluded(lamp_types::display_name)),
                lamp_types::protocol_generation
                    .eq(diesel::upsert::excluded(lamp_types::protocol_generation)),
                lamp_types::protocol_id.eq(diesel::upsert::excluded(lamp_types::protocol_id)),
                lamp_types::num_groups.eq(diesel::upsert::excluded(lamp_types::num_groups)),
                lamp_types::driver_supported
                    .eq(diesel::upsert::excluded(lamp_types::driver_supported)),
            ))
            .execute(&mut conn)
            .await?;

        Ok(written)
    }
}
