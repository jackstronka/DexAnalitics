//! Database connection wrapper and migration runner.
//!
//! Provides a unified interface for database operations including
//! connection management, repository access, and schema migrations.

use super::{PoolRepository, PriceRepository, SimulationRepository};
use sqlx::PgPool;
use std::sync::Arc;

/// Database connection wrapper for repositories.
///
/// Manages the PostgreSQL connection pool and provides factory methods
/// for creating repository instances.
#[derive(Clone)]
pub struct Database {
    pool: Arc<PgPool>,
}

impl Database {
    /// Creates a new Database wrapper from a connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    /// Creates a new database connection from a connection string.
    ///
    /// # Arguments
    /// * `database_url` - PostgreSQL connection string in the format:
    ///   `postgres://user:password@host:port/database`
    ///
    /// # Errors
    /// Returns an error if the connection fails.
    ///
    /// # Examples
    /// ```ignore
    /// let db = Database::connect("postgres://user:pass@localhost/mydb").await?;
    /// ```
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPool::connect(database_url).await?;
        Ok(Self::new(pool))
    }

    /// Returns a reference to the connection pool.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Creates a PoolRepository instance.
    #[must_use]
    pub fn pools(&self) -> PoolRepository {
        PoolRepository::new(self.pool.clone())
    }

    /// Creates a SimulationRepository instance.
    #[must_use]
    pub fn simulations(&self) -> SimulationRepository {
        SimulationRepository::new(self.pool.clone())
    }

    /// Creates a PriceRepository instance.
    #[must_use]
    pub fn prices(&self) -> PriceRepository {
        PriceRepository::new(self.pool.clone())
    }

    /// Runs database migrations.
    ///
    /// Executes the initial schema migration. Splits the migration file
    /// by semicolons and executes each statement separately to support
    /// multiple SQL commands.
    ///
    /// # Errors
    /// Returns an error if any migration statement fails.
    pub async fn migrate(&self) -> Result<(), sqlx::Error> {
        // Keep migrations simple: include fixed SQL files and run each statement idempotently.
        // (No migration table yet; statements are written with IF NOT EXISTS.)
        let migrations: [&str; 4] = [
            include_str!("../../migrations/001_initial_schema.sql"),
            include_str!("../../migrations/002_position_stream_performance.sql"),
            include_str!("../../migrations/003_stream_pnl_snapshots.sql"),
            include_str!("../../migrations/004_stream_snapshot_mints_prices.sql"),
        ];

        for migration_sql in migrations {
            // This runner is intentionally simple and statement-based, but we must NOT split on
            // semicolons that appear in SQL comments. Strip full-line comments first.
            let without_comments = migration_sql
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !t.starts_with("--")
                })
                .collect::<Vec<_>>()
                .join("\n");

            // Split by semicolons and execute each statement separately.
            for statement in without_comments.split(';') {
                let trimmed = statement.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Err(e) = sqlx::query(trimmed).execute(self.pool.as_ref()).await {
                    let preview = if trimmed.len() > 320 {
                        format!("{}…", &trimmed[..320])
                    } else {
                        trimmed.to_string()
                    };
                    tracing::warn!(
                        error = %e,
                        statement_preview = %preview,
                        "DB migrate statement failed"
                    );
                    return Err(e);
                }
            }
        }
        Ok(())
    }
}
