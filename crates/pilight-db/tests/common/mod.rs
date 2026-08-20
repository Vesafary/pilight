//! Harness for the Postgres integration tests.
//!
//! These need a real database. Point `PILIGHT_TEST_DATABASE_URL` (or
//! `DATABASE_URL`) at one and they run; leave both unset and they report a skip
//! rather than passing quietly.
//!
//! ```sh
//! docker run -d --rm --name pilight-test-pg -p 55432:5432 \
//!     -e POSTGRES_USER=pilight -e POSTGRES_PASSWORD=pilight \
//!     -e POSTGRES_DB=pilight_test postgres:16-alpine
//!
//! export PILIGHT_TEST_DATABASE_URL=postgres://pilight:pilight@localhost:55432/pilight_test
//! cargo test -p pilight-db
//! ```

use diesel_async::RunQueryDsl;
use pilight_db::pool::Pool;
use pilight_db::repository::LampTypeRepository;
use pilight_db::{Repositories, build_pool, run_migrations};
use std::sync::OnceLock;
use tokio::sync::{Mutex, MutexGuard};

/// Serialises tests: they share one database and truncate between runs.
static DB_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Migrations run once per test binary, not once per test.
static MIGRATED: OnceLock<()> = OnceLock::new();

fn database_url() -> Option<String> {
    std::env::var("PILIGHT_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
}

/// A ready-to-use database, exclusive to the current test.
pub struct TestDb {
    /// Every repository, over one pool.
    pub repos: Repositories,
    /// The pool, for tests that need raw SQL.
    pub pool: Pool,
    /// Held for the lifetime of the test so no two run concurrently.
    _guard: MutexGuard<'static, ()>,
}

impl TestDb {
    /// Connect, migrate, and hand back an empty database.
    ///
    /// Returns `None` — after printing why — when no database is configured, so
    /// that `cargo test` on a machine without Postgres is still useful.
    pub async fn connect() -> Option<Self> {
        let Some(url) = database_url() else {
            // cargo captures output from passing tests, so a quiet skip would show
            // up as a green run that tested nothing. That is fine on a laptop
            // without Postgres; it is not fine in CI, where it would hide every
            // regression in this crate. So: skip locally, fail loudly in CI.
            assert!(
                std::env::var("CI").is_err(),
                "PILIGHT_TEST_DATABASE_URL is not set, so the Postgres integration \
                 tests would be skipped. Refusing to do that in CI. Start a database \
                 (`docker compose up -d postgres`) and set the variable."
            );
            eprintln!(
                "SKIPPING: set PILIGHT_TEST_DATABASE_URL to run the Postgres integration tests"
            );
            return None;
        };

        let guard = DB_LOCK.get_or_init(|| Mutex::new(())).lock().await;
        let pool = build_pool(&url).expect("the pool should build");

        if MIGRATED.get().is_none() {
            run_migrations(&pool)
                .await
                .expect("migrations should apply");
            let _ = MIGRATED.set(());
        }

        let repos = Repositories::new(pool.clone());
        repos
            .types
            .sync_from_driver()
            .await
            .expect("lamp types should sync");

        let db = Self {
            repos,
            pool,
            _guard: guard,
        };
        db.truncate().await;
        Some(db)
    }

    /// Empty every table except the synced `lamp_types` catalogue.
    pub async fn truncate(&self) {
        let mut conn = self.pool.get().await.expect("a connection");

        // lamp_states and lamp_commands both cascade from lamps.
        diesel::sql_query("TRUNCATE lamps, lamp_commands RESTART IDENTITY CASCADE")
            .execute(&mut conn)
            .await
            .expect("truncate should succeed");
    }
}

/// Skip the test body when no database is configured.
///
/// Used as `let db = require_db!();` at the top of each test.
#[macro_export]
macro_rules! require_db {
    () => {
        match $crate::common::TestDb::connect().await {
            Some(db) => db,
            None => return,
        }
    };
}
