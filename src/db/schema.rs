use anyhow::Result;
use rusqlite::Connection;

pub fn create_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        -- Version tracking
        CREATE TABLE IF NOT EXISTS tss_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        -- Core tables
        CREATE TABLE IF NOT EXISTS transcripts (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            date TEXT NOT NULL,
            duration_seconds REAL NOT NULL DEFAULT 0,
            source TEXT NOT NULL DEFAULT 'unknown',
            summary TEXT NOT NULL DEFAULT '',
            raw_text TEXT NOT NULL DEFAULT '',
            metadata TEXT,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );

        CREATE TABLE IF NOT EXISTS speakers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            transcript_id TEXT NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            UNIQUE(transcript_id, name)
        );

        CREATE TABLE IF NOT EXISTS segments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            transcript_id TEXT NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
            speaker TEXT NOT NULL DEFAULT '',
            text TEXT NOT NULL DEFAULT '',
            start_time REAL NOT NULL DEFAULT 0,
            end_time REAL NOT NULL DEFAULT 0,
            segment_index INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS tags (
            transcript_id TEXT NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
            tag TEXT NOT NULL,
            PRIMARY KEY (transcript_id, tag)
        );

        CREATE TABLE IF NOT EXISTS keywords (
            transcript_id TEXT NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
            keyword TEXT NOT NULL,
            PRIMARY KEY (transcript_id, keyword)
        );

        CREATE TABLE IF NOT EXISTS action_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            transcript_id TEXT NOT NULL REFERENCES transcripts(id) ON DELETE CASCADE,
            text TEXT NOT NULL DEFAULT '',
            metadata TEXT
        );

        -- Indexes for common filters
        CREATE INDEX IF NOT EXISTS idx_transcripts_date ON transcripts(date);
        CREATE INDEX IF NOT EXISTS idx_transcripts_source ON transcripts(source);
        CREATE INDEX IF NOT EXISTS idx_segments_transcript ON segments(transcript_id);
        CREATE INDEX IF NOT EXISTS idx_segments_speaker ON segments(speaker);
        CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag);
        CREATE INDEX IF NOT EXISTS idx_keywords_keyword ON keywords(keyword);

        -- FTS5 virtual tables (content-sync mode)
        CREATE VIRTUAL TABLE IF NOT EXISTS transcripts_fts USING fts5(
            title,
            summary,
            raw_text,
            content='transcripts',
            content_rowid='rowid'
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS segments_fts USING fts5(
            text,
            speaker,
            content='segments',
            content_rowid='rowid'
        );

        -- Triggers to keep FTS5 in sync with content tables

        -- transcripts insert
        CREATE TRIGGER IF NOT EXISTS transcripts_ai AFTER INSERT ON transcripts BEGIN
            INSERT INTO transcripts_fts(rowid, title, summary, raw_text)
            VALUES (new.rowid, new.title, new.summary, new.raw_text);
        END;

        -- transcripts delete
        CREATE TRIGGER IF NOT EXISTS transcripts_ad AFTER DELETE ON transcripts BEGIN
            INSERT INTO transcripts_fts(transcripts_fts, rowid, title, summary, raw_text)
            VALUES ('delete', old.rowid, old.title, old.summary, old.raw_text);
        END;

        -- transcripts update
        CREATE TRIGGER IF NOT EXISTS transcripts_au AFTER UPDATE ON transcripts BEGIN
            INSERT INTO transcripts_fts(transcripts_fts, rowid, title, summary, raw_text)
            VALUES ('delete', old.rowid, old.title, old.summary, old.raw_text);
            INSERT INTO transcripts_fts(rowid, title, summary, raw_text)
            VALUES (new.rowid, new.title, new.summary, new.raw_text);
        END;

        -- segments insert
        CREATE TRIGGER IF NOT EXISTS segments_ai AFTER INSERT ON segments BEGIN
            INSERT INTO segments_fts(rowid, text, speaker)
            VALUES (new.rowid, new.text, new.speaker);
        END;

        -- segments delete
        CREATE TRIGGER IF NOT EXISTS segments_ad AFTER DELETE ON segments BEGIN
            INSERT INTO segments_fts(segments_fts, rowid, text, speaker)
            VALUES ('delete', old.rowid, old.text, old.speaker);
        END;

        -- segments update
        CREATE TRIGGER IF NOT EXISTS segments_au AFTER UPDATE ON segments BEGIN
            INSERT INTO segments_fts(segments_fts, rowid, text, speaker)
            VALUES ('delete', old.rowid, old.text, old.speaker);
            INSERT INTO segments_fts(rowid, text, speaker)
            VALUES (new.rowid, new.text, new.speaker);
        END;
        ",
    )?;

    // Set schema version
    conn.execute(
        "INSERT OR REPLACE INTO tss_meta (key, value) VALUES ('schema_version', '1')",
        [],
    )?;

    Ok(())
}
