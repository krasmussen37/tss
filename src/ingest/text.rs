use anyhow::Result;
use std::path::Path;

use crate::db::models::{NewSegment, NewTranscript};

/// Parse a plain text file into a NewTranscript.
/// Title from filename, date from mtime, body = raw_text, single segment.
pub fn parse_text(content: &str, filepath: &Path, default_source: Option<&str>) -> Result<NewTranscript> {
    let title = filepath
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .replace(['-', '_'], " ");

    let date = std::fs::metadata(filepath)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.to_rfc3339()
        })
        .unwrap_or_default();

    let source = default_source.unwrap_or("text").to_string();
    let raw_text = content.trim().to_string();

    let segments = if raw_text.is_empty() {
        Vec::new()
    } else {
        vec![NewSegment {
            speaker: String::new(),
            text: raw_text.clone(),
            start_time: 0.0,
            end_time: 0.0,
            segment_index: 0,
        }]
    };

    Ok(NewTranscript {
        id: uuid::Uuid::new_v4().to_string(),
        title,
        date,
        duration_seconds: 0.0,
        source,
        summary: String::new(),
        raw_text,
        metadata: None,
        speakers: Vec::new(),
        segments,
        tags: Vec::new(),
        keywords: Vec::new(),
        action_items: Vec::new(),
    })
}
