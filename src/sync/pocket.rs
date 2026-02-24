use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashSet;

use crate::db::models::{NewActionItem, NewSegment, NewTranscript};
use crate::db::Database;
use crate::sync::state;
use crate::sync::{RemoteTranscript, TranscriptConnector};

const DEFAULT_BASE_URL: &str = "https://public.heypocketai.com/api/v1";
const PAGE_SIZE: i64 = 50;

pub struct PocketConnector {
    api_key: String,
    base_url: String,
    tag_id: Option<String>,
    client: reqwest::blocking::Client,
}

impl PocketConnector {
    pub fn new(
        api_key: String,
        tag_name: Option<String>,
        base_url: Option<String>,
        db: &Database,
    ) -> Result<Self> {
        let base_url = base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let client = reqwest::blocking::Client::new();

        // Resolve tag name → UUID if needed
        let tag_id = if let Some(ref name) = tag_name {
            Some(resolve_tag_id(
                &client,
                &api_key,
                &base_url,
                name,
                &db.conn,
            )?)
        } else {
            None
        };

        Ok(Self {
            api_key,
            base_url,
            tag_id,
            client,
        })
    }

    fn get_json(&self, path: &str) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .with_context(|| format!("Failed to GET {}", url))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            bail!("Pocket API returned {} for {}: {}", status, path, text);
        }

        resp.json().context("Failed to parse Pocket API response")
    }
}

impl TranscriptConnector for PocketConnector {
    fn name(&self) -> &str {
        "pocket"
    }

    fn list_remote(&self, since: Option<&str>) -> Result<Vec<RemoteTranscript>> {
        let since_dt = since
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .or_else(|_| chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ"))
                    .context("Failed to parse since timestamp")
            })
            .transpose()?;

        let mut all = Vec::new();
        let mut page: i64 = 1;

        loop {
            let mut path = format!(
                "/public/recordings?page={}&per_page={}",
                page, PAGE_SIZE
            );
            if let Some(ref tid) = self.tag_id {
                path.push_str(&format!("&tag_ids={}", tid));
            }

            let json = self.get_json(&path)?;

            let recordings = json
                .get("data")
                .and_then(|d| d.as_array())
                .context("Unexpected response structure from Pocket list")?;

            if recordings.is_empty() {
                break;
            }

            for r in recordings {
                let id = r
                    .get("id")
                    .and_then(|v| v.as_str().map(|s| s.to_string()).or_else(|| v.as_i64().map(|n| n.to_string())))
                    .unwrap_or_default();
                let title = r
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Untitled")
                    .to_string();
                let date = r
                    .get("created_at")
                    .or_else(|| r.get("date"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                // Filter by since if provided
                if let Some(ref since_dt_val) = since_dt {
                    if let Ok(record_dt) =
                        chrono::DateTime::parse_from_rfc3339(&date)
                            .or_else(|_| chrono::DateTime::parse_from_str(&date, "%Y-%m-%dT%H:%M:%SZ"))
                    {
                        if record_dt <= *since_dt_val {
                            continue;
                        }
                    }
                }

                all.push(RemoteTranscript { id, title, date });
            }

            // Check pagination
            let last_page = json
                .get("meta")
                .and_then(|m| m.get("last_page"))
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            if page >= last_page {
                break;
            }
            page += 1;
        }

        // Sort by date descending
        all.sort_by(|a, b| b.date.cmp(&a.date));
        Ok(all)
    }

    fn fetch_one(&self, id: &str) -> Result<NewTranscript> {
        let path = format!(
            "/public/recordings/{}?include_transcript=true&include_summarizations=true",
            id
        );
        let json = self.get_json(&path)?;

        let data = json
            .get("data")
            .unwrap_or(&json);

        let rec: PocketRecording = serde_json::from_value(data.clone())
            .context("Failed to parse Pocket recording")?;

        Ok(rec.into_new_transcript())
    }
}

#[derive(Debug, Deserialize)]
struct PocketRecording {
    id: Option<serde_json::Value>,
    title: Option<String>,
    created_at: Option<String>,
    duration: Option<f64>, // seconds
    transcript: Option<PocketTranscript>,
    summarizations: Option<serde_json::Value>,
    tags: Option<Vec<PocketTag>>,
}

#[derive(Debug, Deserialize)]
struct PocketTranscript {
    text: Option<String>,
    segments: Option<Vec<PocketSegment>>,
}

#[derive(Debug, Deserialize)]
struct PocketSegment {
    speaker: Option<String>,
    text: Option<String>,
    start: Option<f64>,
    end: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct PocketTag {
    name: Option<String>,
}

impl PocketRecording {
    fn into_new_transcript(self) -> NewTranscript {
        let id = self
            .id
            .map(|v| match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            })
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let title = self.title.unwrap_or_else(|| "Untitled".to_string());
        let date = self.created_at.unwrap_or_default();
        let duration_seconds = self.duration.unwrap_or(0.0);

        // Segments
        let mut segments = Vec::new();
        let mut speakers_set = HashSet::new();
        let mut raw_lines = Vec::new();

        if let Some(ref transcript) = self.transcript {
            if let Some(ref segs) = transcript.segments {
                for (i, s) in segs.iter().enumerate() {
                    let speaker = s
                        .speaker
                        .clone()
                        .unwrap_or_else(|| "Unknown".to_string());
                    let text = s.text.clone().unwrap_or_default();

                    if !speaker.is_empty() {
                        speakers_set.insert(speaker.clone());
                    }
                    raw_lines.push(format!("{}: {}", speaker, text));

                    segments.push(NewSegment {
                        speaker,
                        text,
                        start_time: s.start.unwrap_or(0.0),
                        end_time: s.end.unwrap_or(0.0),
                        segment_index: i as i64,
                    });
                }
            }
        }

        let speakers: Vec<String> = speakers_set.into_iter().collect();

        // Raw text: prefer transcript.text, fall back to joined segments
        let raw_text = self
            .transcript
            .as_ref()
            .and_then(|t| t.text.clone())
            .unwrap_or_else(|| raw_lines.join("\n"));
        // Cap at 100K chars
        let raw_text = if raw_text.len() > 100_000 {
            raw_text[..100_000].to_string()
        } else {
            raw_text
        };

        // Summary from summarizations
        let mut summary = String::new();
        let mut action_items = Vec::new();

        if let Some(ref sums) = self.summarizations {
            // v2_summary: can be object {markdown: "..."} or string
            if let Some(v2sum) = sums.get("v2_summary") {
                if let Some(obj) = v2sum.as_object() {
                    if let Some(md) = obj.get("markdown").and_then(|v| v.as_str()) {
                        summary = md.to_string();
                    }
                } else if let Some(s) = v2sum.as_str() {
                    summary = s.to_string();
                }
            }

            // v2_action_items
            if let Some(v2ai) = sums.get("v2_action_items") {
                if let Some(actions) = v2ai.get("actions").and_then(|a| a.as_array()) {
                    for action in actions {
                        let label = action
                            .get("label")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let context = action
                            .get("context")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let text = if !label.is_empty() {
                            label.to_string()
                        } else {
                            context.to_string()
                        };
                        if !text.is_empty() {
                            action_items.push(NewActionItem {
                                text,
                                metadata: None,
                            });
                        }
                    }
                }
            }
        }

        // Tags
        let tags: Vec<String> = self
            .tags
            .unwrap_or_default()
            .into_iter()
            .filter_map(|t| t.name)
            .collect();

        NewTranscript {
            id,
            title,
            date,
            duration_seconds,
            source: "pocket".to_string(),
            summary,
            raw_text,
            metadata: None,
            speakers,
            segments,
            tags,
            keywords: Vec::new(),
            action_items,
        }
    }
}

/// Resolve a tag name to its UUID via the Pocket API, with caching in sync_state.
fn resolve_tag_id(
    client: &reqwest::blocking::Client,
    api_key: &str,
    base_url: &str,
    tag_name: &str,
    conn: &rusqlite::Connection,
) -> Result<String> {
    // Check cache first
    let cache_key = format!("pocket.tag_id.{}", tag_name);
    if let Some(cached) = state::get_sync_state(conn, &cache_key)? {
        return Ok(cached);
    }

    // Fetch all tags
    let url = format!("{}/public/tags", base_url);
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .context("Failed to fetch Pocket tags")?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        bail!("Pocket tags API returned {}: {}", status, text);
    }

    let json: serde_json::Value = resp.json().context("Failed to parse Pocket tags response")?;

    let tags = json
        .get("data")
        .and_then(|d| d.as_array())
        .or_else(|| json.as_array())
        .context("Unexpected tags response structure")?;

    for tag in tags {
        let name = tag.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.eq_ignore_ascii_case(tag_name) {
            let id = tag
                .get("id")
                .and_then(|v| v.as_str().map(|s| s.to_string()).or_else(|| v.as_i64().map(|n| n.to_string())))
                .context("Tag found but has no id field")?;

            // Cache it
            state::set_sync_state(conn, &cache_key, &id)?;
            return Ok(id);
        }
    }

    bail!(
        "Tag '{}' not found in Pocket. Available tags: {}",
        tag_name,
        tags.iter()
            .filter_map(|t| t.get("name").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    );
}
