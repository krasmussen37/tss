use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;

/// Get a sync state value by key.
pub fn get_sync_state(conn: &Connection, key: &str) -> Result<Option<String>> {
    let result = conn
        .query_row(
            "SELECT value FROM sync_state WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(result)
}

/// Set a sync state value (upsert).
pub fn set_sync_state(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_state (key, value, updated_at)
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
         ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Record the start of a sync run. Returns the run ID.
pub fn start_sync_run(conn: &Connection, source: &str, mode: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO sync_runs (source, mode, started_at, status)
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), 'running')",
        rusqlite::params![source, mode],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Complete a sync run with final counts.
pub fn complete_sync_run(
    conn: &Connection,
    run_id: i64,
    found: usize,
    synced: usize,
    skipped: usize,
    errors: usize,
    status: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE sync_runs SET
            completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
            transcripts_found = ?2,
            transcripts_synced = ?3,
            transcripts_skipped = ?4,
            errors = ?5,
            status = ?6
         WHERE id = ?1",
        rusqlite::params![run_id, found, synced, skipped, errors, status],
    )?;
    Ok(())
}

/// Get all local transcript IDs for a given source.
pub fn get_local_ids_for_source(conn: &Connection, source: &str) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT id FROM transcripts WHERE source = ?1")?;
    let rows = stmt.query_map([source], |row| row.get::<_, String>(0))?;
    let mut ids = HashSet::new();
    for row in rows {
        ids.insert(row?);
    }
    Ok(ids)
}
