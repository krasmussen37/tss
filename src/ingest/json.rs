use anyhow::{Context, Result};
use serde::Deserialize;

use crate::db::models::{NewActionItem, NewSegment, NewTranscript};

/// Native TSS JSON format.
#[derive(Debug, Deserialize)]
pub struct JsonTranscript {
    pub id: Option<String>,
    pub title: Option<String>,
    pub date: Option<String>,
    pub duration_seconds: Option<f64>,
    pub source: Option<String>,
    pub summary: Option<String>,
    pub raw_text: Option<String>,
    pub segments: Option<Vec<JsonSegment>>,
    pub speakers: Option<Vec<JsonSpeaker>>,
    pub tags: Option<Vec<String>>,
    pub keywords: Option<Vec<String>>,
    pub action_items: Option<Vec<JsonActionItem>>,
    pub metadata: Option<serde_json::Value>,

    // Legacy Python format fields — packed into metadata
    pub organizer_email: Option<String>,
    pub transcript_url: Option<String>,
    pub audio_url: Option<String>,
    pub file_path: Option<String>,
    pub participants: Option<Vec<String>>,
    pub crm_people_ids: Option<Vec<String>>,
    pub crm_company_ids: Option<Vec<String>>,
    pub crm_deal_ids: Option<Vec<String>>,
    #[serde(rename = "_metadata")]
    pub underscore_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct JsonSegment {
    pub speaker: Option<String>,
    pub text: Option<String>,
    pub start: Option<f64>,
    pub end: Option<f64>,
    // Also accept start_time/end_time
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct JsonSpeaker {
    pub id: Option<serde_json::Value>,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct JsonActionItem {
    pub title: Option<String>,
    pub description: Option<String>,
    pub text: Option<String>,
    pub subtasks: Option<serde_json::Value>,
    pub priority: Option<String>,
}

/// Parse a JSON string into a NewTranscript.
pub fn parse_json(content: &str, default_source: Option<&str>) -> Result<NewTranscript> {
    let jt: JsonTranscript =
        serde_json::from_str(content).context("Failed to parse JSON transcript")?;

    let id = jt
        .id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let title = jt.title.unwrap_or_else(|| "Untitled".to_string());
    let date = jt.date.unwrap_or_default();
    let duration = jt.duration_seconds.unwrap_or(0.0);
    let source = jt
        .source
        .or_else(|| default_source.map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string());
    let summary = jt.summary.unwrap_or_default();
    let raw_text = jt.raw_text.unwrap_or_default();

    // Build metadata from legacy fields
    let mut meta = jt.metadata.unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
    if let serde_json::Value::Object(ref mut map) = meta {
        if let Some(v) = &jt.organizer_email {
            map.insert("organizer_email".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = &jt.transcript_url {
            map.insert("transcript_url".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = &jt.audio_url {
            map.insert("audio_url".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = &jt.file_path {
            map.insert("file_path".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = &jt.participants {
            map.insert("participants".into(), serde_json::json!(v));
        }
        if let Some(v) = &jt.crm_people_ids {
            map.insert("crm_people_ids".into(), serde_json::json!(v));
        }
        if let Some(v) = &jt.crm_company_ids {
            map.insert("crm_company_ids".into(), serde_json::json!(v));
        }
        if let Some(v) = &jt.crm_deal_ids {
            map.insert("crm_deal_ids".into(), serde_json::json!(v));
        }
        if let Some(v) = &jt.underscore_metadata {
            map.insert("_original_metadata".into(), v.clone());
        }
    }

    let metadata = if meta == serde_json::Value::Object(serde_json::Map::new()) {
        None
    } else {
        Some(meta)
    };

    // Speakers
    let speakers: Vec<String> = jt
        .speakers
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| s.name)
        .collect();

    // Segments
    let segments: Vec<NewSegment> = jt
        .segments
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(i, s)| NewSegment {
            speaker: s.speaker.unwrap_or_default(),
            text: s.text.unwrap_or_default(),
            start_time: s.start.or(s.start_time).unwrap_or(0.0),
            end_time: s.end.or(s.end_time).unwrap_or(0.0),
            segment_index: i as i64,
        })
        .collect();

    let tags = jt.tags.unwrap_or_default();
    let keywords = jt.keywords.unwrap_or_default();

    let action_items: Vec<NewActionItem> = jt
        .action_items
        .unwrap_or_default()
        .into_iter()
        .map(|ai| {
            let text = ai
                .text
                .or(ai.title)
                .or(ai.description)
                .unwrap_or_default();
            let ai_meta = match (&ai.subtasks, &ai.priority) {
                (None, None) => None,
                _ => {
                    let mut m = serde_json::Map::new();
                    if let Some(st) = ai.subtasks {
                        m.insert("subtasks".into(), st);
                    }
                    if let Some(p) = ai.priority {
                        m.insert("priority".into(), serde_json::Value::String(p));
                    }
                    Some(serde_json::Value::Object(m))
                }
            };
            NewActionItem {
                text,
                metadata: ai_meta,
            }
        })
        .collect();

    Ok(NewTranscript {
        id,
        title,
        date,
        duration_seconds: duration,
        source,
        summary,
        raw_text,
        metadata,
        speakers,
        segments,
        tags,
        keywords,
        action_items,
    })
}
