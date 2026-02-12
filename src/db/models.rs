use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub id: String,
    pub title: String,
    pub date: String,
    pub duration_seconds: f64,
    pub source: String,
    pub summary: String,
    pub raw_text: String,
    pub metadata: Option<serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Speaker {
    pub id: i64,
    pub transcript_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub id: i64,
    pub transcript_id: String,
    pub speaker: String,
    pub text: String,
    pub start_time: f64,
    pub end_time: f64,
    pub segment_index: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionItem {
    pub id: i64,
    pub transcript_id: String,
    pub text: String,
    pub metadata: Option<serde_json::Value>,
}

/// Data needed to insert a new transcript (no auto-generated fields).
#[derive(Debug, Clone)]
pub struct NewTranscript {
    pub id: String,
    pub title: String,
    pub date: String,
    pub duration_seconds: f64,
    pub source: String,
    pub summary: String,
    pub raw_text: String,
    pub metadata: Option<serde_json::Value>,
    pub speakers: Vec<String>,
    pub segments: Vec<NewSegment>,
    pub tags: Vec<String>,
    pub keywords: Vec<String>,
    pub action_items: Vec<NewActionItem>,
}

#[derive(Debug, Clone)]
pub struct NewSegment {
    pub speaker: String,
    pub text: String,
    pub start_time: f64,
    pub end_time: f64,
    pub segment_index: i64,
}

#[derive(Debug, Clone)]
pub struct NewActionItem {
    pub text: String,
    pub metadata: Option<serde_json::Value>,
}

/// Stats returned by `tss stats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbStats {
    pub transcripts: i64,
    pub segments: i64,
    pub speakers: i64,
    pub tags: i64,
    pub keywords: i64,
    pub action_items: i64,
    pub sources: Vec<SourceCount>,
    pub db_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCount {
    pub source: String,
    pub count: i64,
}
