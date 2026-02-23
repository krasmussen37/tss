use anyhow::Result;
use rusqlite::Connection;
use tracing::info;

/// Run all pending migrations. Currently a no-op since v1 schema is created
/// fresh by schema.rs. Future schema changes will be added here as numbered
/// migrations.
pub fn run_migrations(conn: &Connection) -> Result<()> {
    // Ensure migrations tracking table exists
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tss_migrations (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );",
    )?;

    run_migration(conn, 1, "add_sync_tables", |c| {
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS sync_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE IF NOT EXISTS sync_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                mode TEXT NOT NULL,
                started_at TEXT NOT NULL,
                completed_at TEXT,
                transcripts_found INTEGER DEFAULT 0,
                transcripts_synced INTEGER DEFAULT 0,
                transcripts_skipped INTEGER DEFAULT 0,
                errors INTEGER DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'running'
            );",
        )?;
        Ok(())
    })?;

    Ok(())
}

#[allow(dead_code)]
fn run_migration<F>(conn: &Connection, id: i64, name: &str, f: F) -> Result<()>
where
    F: FnOnce(&Connection) -> Result<()>,
{
    let already_applied: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM tss_migrations WHERE id = ?1",
        [id],
        |row| row.get(0),
    )?;

    if already_applied {
        return Ok(());
    }

    f(conn)?;

    conn.execute(
        "INSERT INTO tss_migrations (id, name) VALUES (?1, ?2)",
        rusqlite::params![id, name],
    )?;

    info!("Applied migration {id}: {name}");
    Ok(())
}
