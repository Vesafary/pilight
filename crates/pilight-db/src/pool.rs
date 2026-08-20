//! Connection pooling and schema setup.
//!
//! Everything here — queries *and* migrations — goes over `diesel-async`'s
//! `AsyncPgConnection`, which speaks the Postgres wire protocol in pure Rust via
//! `tokio-postgres`. Nothing links `libpq`, so there is no system dependency to
//! install on the Pi.

use crate::error::{DbError, Result};
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::pooled_connection::deadpool::{Object, Pool as DeadPool};
use diesel_async::{AsyncMigrationHarness, AsyncPgConnection};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

/// Migrations compiled into the binary, so deployment is a single file.
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// A pool of async Postgres connections.
pub type Pool = DeadPool<AsyncPgConnection>;

/// A connection checked out of the [`Pool`].
pub type PooledConnection = Object<AsyncPgConnection>;

/// Default pool size. Small on purpose: this drives a handful of lamps, not a
/// web-scale workload, and a Pi has limited memory.
pub const DEFAULT_POOL_SIZE: usize = 8;

/// Build a connection pool for `database_url`.
///
/// Connections are established lazily, so this succeeding does not prove the
/// database is reachable. Call [`run_migrations`] to find that out.
pub fn build_pool(database_url: &str) -> Result<Pool> {
    build_pool_with_size(database_url, DEFAULT_POOL_SIZE)
}

/// Build a connection pool with an explicit maximum size.
pub fn build_pool_with_size(database_url: &str, max_size: usize) -> Result<Pool> {
    let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(database_url);

    Pool::builder(manager)
        .max_size(max_size)
        .build()
        .map_err(DbError::from)
}

/// Apply any migrations the database has not seen yet.
///
/// Idempotent and safe to call on every start. Returns the versions applied,
/// which is empty when the schema was already current.
///
/// # Runtime
///
/// [`AsyncMigrationHarness`] wraps sync Diesel's migration machinery in
/// `block_in_place`, so this needs the **multi-threaded** tokio runtime. Under
/// `#[tokio::main(flavor = "current_thread")]` it will panic; wrap it in
/// `tokio::task::spawn_blocking` there instead.
pub async fn run_migrations(pool: &Pool) -> Result<Vec<String>> {
    let conn = pool.get().await?;
    let mut harness = AsyncMigrationHarness::new(conn);

    let applied = harness
        .run_pending_migrations(MIGRATIONS)
        .map_err(|e| DbError::Migration(e.to_string()))?;

    Ok(applied.iter().map(ToString::to_string).collect())
}

/// Roll every migration back. Intended for tests and local resets.
///
/// Carries the same runtime caveat as [`run_migrations`].
pub async fn revert_migrations(pool: &Pool) -> Result<()> {
    let conn = pool.get().await?;
    let mut harness = AsyncMigrationHarness::new(conn);

    harness
        .revert_all_migrations(MIGRATIONS)
        .map_err(|e| DbError::Migration(e.to_string()))?;

    Ok(())
}
