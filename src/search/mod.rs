pub mod filters;

use anyhow::Result;
use serde::Serialize;

use crate::db::Database;
use filters::Filters;

#[derive(Debug, Clone, Serialize)]
pub struct TranscriptResult {
    pub id: String,
    pub title: String,
    pub date: String,
    pub source: String,
    pub duration_seconds: f64,
    pub rank: f64,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentResult {
    pub transcript_id: String,
    pub transcript_title: String,
    pub segment_id: i64,
    pub speaker: String,
    pub text: String,
    pub start_time: f64,
    pub end_time: f64,
    pub rank: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchOutput {
    pub query: String,
    pub total: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transcripts: Vec<TranscriptResult>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<SegmentResult>,
}

impl Database {
    /// Search transcripts using FTS5. Returns transcript-level results with BM25 ranking.
    pub fn search_transcripts(
        &self,
        query: &str,
        filters: &Filters,
        limit: usize,
    ) -> Result<Vec<TranscriptResult>> {
        let (filter_conditions, filter_params) = filters.transcript_conditions();

        let mut where_parts = vec!["transcripts_fts MATCH ?1".to_string()];
        // Offset filter param indices by 1 (query is ?1)
        for (i, cond) in filter_conditions.iter().enumerate() {
            // Replace ?N with ?(N+1) since ?1 is the query
            let adjusted = cond.replace(
                &format!("?{}", i + 1),
                &format!("?{}", i + 2),
            );
            where_parts.push(adjusted);
        }

        let where_clause = where_parts.join(" AND ");

        let sql = format!(
            "SELECT t.id, t.title, t.date, t.source, t.duration_seconds,
                    bm25(transcripts_fts, 5.0, 2.0, 1.0) AS rank,
                    snippet(transcripts_fts, 2, '>>>', '<<<', '...', 40) AS snippet
             FROM transcripts_fts
             JOIN transcripts t ON t.rowid = transcripts_fts.rowid
             WHERE {where_clause}
             ORDER BY rank
             LIMIT ?{}",
            filter_params.len() + 2
        );

        let mut stmt = self.conn.prepare(&sql)?;

        // Build params: query, filter_params..., limit
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.push(Box::new(query.to_string()));
        for p in filter_params {
            all_params.push(p);
        }
        all_params.push(Box::new(limit as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(TranscriptResult {
                id: row.get(0)?,
                title: row.get(1)?,
                date: row.get(2)?,
                source: row.get(3)?,
                duration_seconds: row.get(4)?,
                rank: row.get(5)?,
                snippet: row.get(6)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Search segments using FTS5. Returns segment-level results with transcript context.
    pub fn search_segments(
        &self,
        query: &str,
        filters: &Filters,
        limit: usize,
    ) -> Result<Vec<SegmentResult>> {
        let (filter_conditions, filter_params) = filters.transcript_conditions();

        let mut where_parts = vec!["segments_fts MATCH ?1".to_string()];
        for (i, cond) in filter_conditions.iter().enumerate() {
            let adjusted = cond
                .replace(
                    &format!("?{}", i + 1),
                    &format!("?{}", i + 2),
                )
                .replace("t.id", "t.id")
                .replace("t.source", "t.source")
                .replace("t.date", "t.date");
            where_parts.push(adjusted);
        }

        let where_clause = where_parts.join(" AND ");

        let sql = format!(
            "SELECT s.transcript_id, t.title, s.id, s.speaker, s.text,
                    s.start_time, s.end_time,
                    bm25(segments_fts, 2.0, 1.0) AS rank
             FROM segments_fts
             JOIN segments s ON s.rowid = segments_fts.rowid
             JOIN transcripts t ON t.id = s.transcript_id
             WHERE {where_clause}
             ORDER BY rank
             LIMIT ?{}",
            filter_params.len() + 2
        );

        let mut stmt = self.conn.prepare(&sql)?;

        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.push(Box::new(query.to_string()));
        for p in filter_params {
            all_params.push(p);
        }
        all_params.push(Box::new(limit as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(SegmentResult {
                transcript_id: row.get(0)?,
                transcript_title: row.get(1)?,
                segment_id: row.get(2)?,
                speaker: row.get(3)?,
                text: row.get(4)?,
                start_time: row.get(5)?,
                end_time: row.get(6)?,
                rank: row.get(7)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// List transcripts with optional filters and sorting.
    pub fn list_transcripts(
        &self,
        filters: &Filters,
        sort: &str,
        limit: usize,
    ) -> Result<Vec<TranscriptResult>> {
        let (filter_conditions, filter_params) = filters.transcript_conditions();

        let where_clause = if filter_conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", filter_conditions.join(" AND "))
        };

        let order_by = match sort {
            "title" => "t.title ASC",
            _ => "t.date DESC",
        };

        let sql = format!(
            "SELECT t.id, t.title, t.date, t.source, t.duration_seconds, 0.0, ''
             FROM transcripts t
             {where_clause}
             ORDER BY {order_by}
             LIMIT ?{}",
            filter_params.len() + 1
        );

        let mut stmt = self.conn.prepare(&sql)?;

        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for p in filter_params {
            all_params.push(p);
        }
        all_params.push(Box::new(limit as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(TranscriptResult {
                id: row.get(0)?,
                title: row.get(1)?,
                date: row.get(2)?,
                source: row.get(3)?,
                duration_seconds: row.get(4)?,
                rank: row.get(5)?,
                snippet: row.get(6)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }
}
