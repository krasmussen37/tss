use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use tracing::info;

use crate::db::models::{NewActionItem, NewSegment, NewTranscript};
use crate::db::Database;

/// Migrate transcripts from the legacy Python transcripts.db into the tss database.
pub fn migrate_from_python_db(db: &Database, source_path: &Path, dry_run: bool) -> Result<MigrateStats> {
    let src = Connection::open(source_path)
        .with_context(|| format!("Failed to open source database: {}", source_path.display()))?;

    let mut stats = MigrateStats::default();

    // Read all transcripts
    let mut stmt = src.prepare(
        "SELECT id, source, title, date, duration_seconds, organizer_email,
                raw_text, summary, transcript_url, audio_url, file_path,
                created_at, updated_at
         FROM transcripts ORDER BY date",
    )?;

    let transcript_rows: Vec<TranscriptRow> = stmt
        .query_map([], |row| {
            Ok(TranscriptRow {
                id: row.get(0)?,
                source: row.get(1)?,
                title: row.get(2)?,
                date: row.get(3)?,
                duration_seconds: row.get(4)?,
                organizer_email: row.get(5)?,
                raw_text: row.get(6)?,
                summary: row.get(7)?,
                transcript_url: row.get(8)?,
                audio_url: row.get(9)?,
                file_path: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    info!("Found {} transcripts to migrate", transcript_rows.len());

    for trow in &transcript_rows {
        if !dry_run && db.transcript_exists(&trow.id)? {
            stats.skipped += 1;
            continue;
        }

        // Segments
        let segments = read_segments(&src, &trow.id)?;

        // Speakers
        let speakers = read_speakers(&src, &trow.id)?;

        // Tags
        let tags = read_tags(&src, &trow.id)?;

        // Keywords
        let keywords = read_keywords(&src, &trow.id)?;

        // Action items
        let action_items = read_action_items(&src, &trow.id)?;

        // Pack legacy fields into metadata
        let mut meta = serde_json::Map::new();
        if let Some(ref v) = trow.organizer_email {
            meta.insert("organizer_email".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(ref v) = trow.transcript_url {
            meta.insert("transcript_url".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(ref v) = trow.audio_url {
            meta.insert("audio_url".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(ref v) = trow.file_path {
            meta.insert("file_path".into(), serde_json::Value::String(v.clone()));
        }

        // Participants
        let participants = read_participants(&src, &trow.id)?;
        if !participants.is_empty() {
            meta.insert("participants".into(), serde_json::json!(participants));
        }

        let metadata = if meta.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(meta))
        };

        let new_t = NewTranscript {
            id: trow.id.clone(),
            title: trow.title.clone(),
            date: trow.date.clone(),
            duration_seconds: trow.duration_seconds,
            source: trow.source.clone(),
            summary: trow.summary.clone(),
            raw_text: trow.raw_text.clone(),
            metadata,
            speakers,
            segments,
            tags,
            keywords,
            action_items,
        };

        if dry_run {
            println!(
                "  [dry-run] Would import: {} ({}, {} segments)",
                new_t.title,
                new_t.source,
                new_t.segments.len()
            );
        } else {
            db.insert_transcript(&new_t)?;
        }
        stats.imported += 1;
    }

    Ok(stats)
}

#[derive(Debug, Default)]
pub struct MigrateStats {
    pub imported: usize,
    pub skipped: usize,
}

struct TranscriptRow {
    id: String,
    source: String,
    title: String,
    date: String,
    duration_seconds: f64,
    organizer_email: Option<String>,
    raw_text: String,
    summary: String,
    transcript_url: Option<String>,
    audio_url: Option<String>,
    file_path: Option<String>,
    #[allow(dead_code)]
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
}

fn read_segments(src: &Connection, transcript_id: &str) -> Result<Vec<NewSegment>> {
    let mut stmt = src.prepare(
        "SELECT speaker, text, start_time, end_time, segment_index
         FROM transcript_segments WHERE transcript_id = ?1 ORDER BY segment_index",
    )?;
    let rows = stmt.query_map([transcript_id], |row| {
        Ok(NewSegment {
            speaker: row.get(0)?,
            text: row.get(1)?,
            start_time: row.get(2)?,
            end_time: row.get(3)?,
            segment_index: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn read_speakers(src: &Connection, transcript_id: &str) -> Result<Vec<String>> {
    let mut stmt = src.prepare(
        "SELECT speaker_name FROM transcript_speakers WHERE transcript_id = ?1",
    )?;
    let rows = stmt.query_map([transcript_id], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn read_tags(src: &Connection, transcript_id: &str) -> Result<Vec<String>> {
    let mut stmt = src.prepare(
        "SELECT tag FROM transcript_tags WHERE transcript_id = ?1",
    )?;
    let rows = stmt.query_map([transcript_id], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn read_keywords(src: &Connection, transcript_id: &str) -> Result<Vec<String>> {
    let mut stmt = src.prepare(
        "SELECT keyword FROM transcript_keywords WHERE transcript_id = ?1",
    )?;
    let rows = stmt.query_map([transcript_id], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn read_action_items(src: &Connection, transcript_id: &str) -> Result<Vec<NewActionItem>> {
    let mut stmt = src.prepare(
        "SELECT title, description, subtasks, priority
         FROM action_items WHERE transcript_id = ?1",
    )?;
    let rows = stmt.query_map([transcript_id], |row| {
        let title: String = row.get(0)?;
        let description: String = row.get(1)?;
        let subtasks: Option<String> = row.get(2)?;
        let priority: Option<String> = row.get(3)?;

        let text = if description.is_empty() {
            title
        } else {
            description
        };

        let mut meta = serde_json::Map::new();
        if let Some(ref st) = subtasks {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(st) {
                meta.insert("subtasks".into(), v);
            }
        }
        if let Some(ref p) = priority {
            meta.insert("priority".into(), serde_json::Value::String(p.clone()));
        }

        let metadata = if meta.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(meta))
        };

        Ok(NewActionItem { text, metadata })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn read_participants(src: &Connection, transcript_id: &str) -> Result<Vec<String>> {
    let mut stmt = src.prepare(
        "SELECT email FROM transcript_participants WHERE transcript_id = ?1",
    )?;
    let rows = stmt.query_map([transcript_id], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
