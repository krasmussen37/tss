pub mod migrations;
pub mod models;
pub mod schema;

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use tracing::info;

use models::*;

pub struct Database {
    pub conn: Connection,
    pub path: PathBuf,
}

impl Database {
    /// Open (or create) the database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open database: {}", path.display()))?;

        // Performance pragmas
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA cache_size = -64000;",
        )?;

        schema::create_schema(&conn)?;
        migrations::run_migrations(&conn)?;

        info!("Opened database: {}", path.display());

        Ok(Database {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// Default database path: ~/.tss/tss.db
    pub fn default_db_path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".tss").join("tss.db"))
    }

    /// Insert a fully-formed transcript with all related data.
    pub fn insert_transcript(&self, t: &NewTranscript) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        let metadata_json = t
            .metadata
            .as_ref()
            .map(|m| serde_json::to_string(m))
            .transpose()?;

        tx.execute(
            "INSERT OR REPLACE INTO transcripts (id, title, date, duration_seconds, source, summary, raw_text, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                t.id,
                t.title,
                t.date,
                t.duration_seconds,
                t.source,
                t.summary,
                t.raw_text,
                metadata_json,
            ],
        )?;

        // Speakers
        for name in &t.speakers {
            tx.execute(
                "INSERT OR IGNORE INTO speakers (transcript_id, name) VALUES (?1, ?2)",
                rusqlite::params![t.id, name],
            )?;
        }

        // Segments
        for seg in &t.segments {
            tx.execute(
                "INSERT INTO segments (transcript_id, speaker, text, start_time, end_time, segment_index)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    t.id,
                    seg.speaker,
                    seg.text,
                    seg.start_time,
                    seg.end_time,
                    seg.segment_index,
                ],
            )?;
        }

        // Tags
        for tag in &t.tags {
            tx.execute(
                "INSERT OR IGNORE INTO tags (transcript_id, tag) VALUES (?1, ?2)",
                rusqlite::params![t.id, tag],
            )?;
        }

        // Keywords
        for kw in &t.keywords {
            tx.execute(
                "INSERT OR IGNORE INTO keywords (transcript_id, keyword) VALUES (?1, ?2)",
                rusqlite::params![t.id, kw],
            )?;
        }

        // Action items
        for ai in &t.action_items {
            let ai_meta = ai
                .metadata
                .as_ref()
                .map(|m| serde_json::to_string(m))
                .transpose()?;
            tx.execute(
                "INSERT INTO action_items (transcript_id, text, metadata) VALUES (?1, ?2, ?3)",
                rusqlite::params![t.id, ai.text, ai_meta],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Delete a transcript and all related data (cascading).
    pub fn delete_transcript(&self, id: &str) -> Result<bool> {
        let deleted = self
            .conn
            .execute("DELETE FROM transcripts WHERE id = ?1", [id])?;
        Ok(deleted > 0)
    }

    /// Get a single transcript by ID.
    pub fn get_transcript(&self, id: &str) -> Result<Option<Transcript>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, date, duration_seconds, source, summary, raw_text, metadata, created_at, updated_at
             FROM transcripts WHERE id = ?1",
        )?;

        let result = stmt
            .query_row([id], |row| {
                let metadata_str: Option<String> = row.get(7)?;
                Ok(Transcript {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    date: row.get(2)?,
                    duration_seconds: row.get(3)?,
                    source: row.get(4)?,
                    summary: row.get(5)?,
                    raw_text: row.get(6)?,
                    metadata: metadata_str
                        .and_then(|s| serde_json::from_str(&s).ok()),
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            })
            .optional()?;

        Ok(result)
    }

    /// Get segments for a transcript.
    pub fn get_segments(&self, transcript_id: &str) -> Result<Vec<Segment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, transcript_id, speaker, text, start_time, end_time, segment_index
             FROM segments WHERE transcript_id = ?1 ORDER BY segment_index",
        )?;

        let rows = stmt.query_map([transcript_id], |row| {
            Ok(Segment {
                id: row.get(0)?,
                transcript_id: row.get(1)?,
                speaker: row.get(2)?,
                text: row.get(3)?,
                start_time: row.get(4)?,
                end_time: row.get(5)?,
                segment_index: row.get(6)?,
            })
        })?;

        let mut segments = Vec::new();
        for row in rows {
            segments.push(row?);
        }
        Ok(segments)
    }

    /// Get tags for a transcript.
    pub fn get_tags(&self, transcript_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT tag FROM tags WHERE transcript_id = ?1 ORDER BY tag")?;
        let rows = stmt.query_map([transcript_id], |row| row.get(0))?;
        let mut tags = Vec::new();
        for row in rows {
            tags.push(row?);
        }
        Ok(tags)
    }

    /// Get keywords for a transcript.
    pub fn get_keywords(&self, transcript_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT keyword FROM keywords WHERE transcript_id = ?1 ORDER BY keyword")?;
        let rows = stmt.query_map([transcript_id], |row| row.get(0))?;
        let mut kws = Vec::new();
        for row in rows {
            kws.push(row?);
        }
        Ok(kws)
    }

    /// Get speakers for a transcript.
    pub fn get_speakers(&self, transcript_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM speakers WHERE transcript_id = ?1 ORDER BY name")?;
        let rows = stmt.query_map([transcript_id], |row| row.get(0))?;
        let mut names = Vec::new();
        for row in rows {
            names.push(row?);
        }
        Ok(names)
    }

    /// Get action items for a transcript.
    pub fn get_action_items(&self, transcript_id: &str) -> Result<Vec<ActionItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, transcript_id, text, metadata FROM action_items WHERE transcript_id = ?1",
        )?;
        let rows = stmt.query_map([transcript_id], |row| {
            let meta_str: Option<String> = row.get(3)?;
            Ok(ActionItem {
                id: row.get(0)?,
                transcript_id: row.get(1)?,
                text: row.get(2)?,
                metadata: meta_str.and_then(|s| serde_json::from_str(&s).ok()),
            })
        })?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    }

    /// Get database statistics.
    pub fn stats(&self) -> Result<DbStats> {
        let transcripts: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM transcripts", [], |r| r.get(0))?;
        let segments: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM segments", [], |r| r.get(0))?;
        let speakers: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM speakers", [], |r| r.get(0))?;
        let tags: i64 = self
            .conn
            .query_row("SELECT COUNT(DISTINCT tag) FROM tags", [], |r| r.get(0))?;
        let keywords: i64 = self
            .conn
            .query_row("SELECT COUNT(DISTINCT keyword) FROM keywords", [], |r| {
                r.get(0)
            })?;
        let action_items: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM action_items", [], |r| r.get(0))?;

        let mut stmt = self
            .conn
            .prepare("SELECT source, COUNT(*) FROM transcripts GROUP BY source ORDER BY source")?;
        let source_rows = stmt.query_map([], |row| {
            Ok(SourceCount {
                source: row.get(0)?,
                count: row.get(1)?,
            })
        })?;
        let mut sources = Vec::new();
        for row in source_rows {
            sources.push(row?);
        }

        let db_size_bytes = std::fs::metadata(&self.path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(DbStats {
            transcripts,
            segments,
            speakers,
            tags,
            keywords,
            action_items,
            sources,
            db_size_bytes,
        })
    }

    /// Rebuild FTS5 indexes from scratch.
    pub fn reindex(&self) -> Result<()> {
        self.conn.execute_batch(
            "INSERT INTO transcripts_fts(transcripts_fts) VALUES('rebuild');
             INSERT INTO segments_fts(segments_fts) VALUES('rebuild');",
        )?;
        info!("FTS5 indexes rebuilt");
        Ok(())
    }

    /// Check if a transcript exists.
    pub fn transcript_exists(&self, id: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM transcripts WHERE id = ?1",
            [id],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }
}

use rusqlite::OptionalExtension;
