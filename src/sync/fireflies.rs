use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashSet;

use crate::db::models::{NewActionItem, NewSegment, NewTranscript};
use crate::sync::{RemoteTranscript, TranscriptConnector};

const FIREFLIES_ENDPOINT: &str = "https://api.fireflies.ai/graphql";
const PAGE_SIZE: i64 = 50;

pub struct FirefliesConnector {
    api_key: String,
    client: reqwest::blocking::Client,
}

impl FirefliesConnector {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::blocking::Client::new(),
        }
    }

    fn graphql_request(&self, query: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "query": query });
        let resp = self
            .client
            .post(FIREFLIES_ENDPOINT)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .context("Failed to send request to Fireflies API")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            bail!("Fireflies API returned {}: {}", status, text);
        }

        let json: serde_json::Value = resp.json().context("Failed to parse Fireflies response")?;

        if let Some(errors) = json.get("errors") {
            bail!("Fireflies GraphQL errors: {}", errors);
        }

        Ok(json)
    }
}

impl TranscriptConnector for FirefliesConnector {
    fn name(&self) -> &str {
        "fireflies"
    }

    fn list_remote(&self, since: Option<&str>) -> Result<Vec<RemoteTranscript>> {
        let since_ms: Option<i64> = since
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .or_else(|_| chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ"))
                    .map(|dt| dt.timestamp_millis())
                    .context("Failed to parse since timestamp")
            })
            .transpose()?;

        let mut all = Vec::new();
        let mut skip: i64 = 0;

        loop {
            let query = format!(
                r#"query {{ transcripts(limit: {}, skip: {}) {{ id title date }} }}"#,
                PAGE_SIZE, skip
            );

            let json = self.graphql_request(&query)?;
            let transcripts = json
                .get("data")
                .and_then(|d| d.get("transcripts"))
                .and_then(|t| t.as_array())
                .context("Unexpected response structure from Fireflies list query")?;

            if transcripts.is_empty() {
                break;
            }

            for t in transcripts {
                let id = t
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let title = t
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Untitled")
                    .to_string();

                // date is epoch milliseconds
                let date_ms = t
                    .get("date")
                    .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                    .unwrap_or(0);

                let date_iso = epoch_ms_to_iso(date_ms);

                // Filter by since if provided
                if let Some(since_ms_val) = since_ms {
                    if date_ms <= since_ms_val {
                        continue;
                    }
                }

                all.push(RemoteTranscript {
                    id,
                    title,
                    date: date_iso,
                });
            }

            if (transcripts.len() as i64) < PAGE_SIZE {
                break;
            }
            skip += PAGE_SIZE;
        }

        // Sort by date descending (newest first)
        all.sort_by(|a, b| b.date.cmp(&a.date));
        Ok(all)
    }

    fn fetch_one(&self, id: &str) -> Result<NewTranscript> {
        // Known quirk: use inline string interpolation for transcript(id:), not variables
        let query = format!(
            r#"query {{ transcript(id: "{}") {{
                id title date duration organizer_email participants
                summary {{ keywords action_items overview shorthand_bullet }}
                sentences {{ text speaker_name start_time end_time }}
            }} }}"#,
            id
        );

        let json = self.graphql_request(&query)?;
        let t = json
            .get("data")
            .and_then(|d| d.get("transcript"))
            .context("No transcript data in Fireflies response")?;

        let ff: FirefliesTranscript =
            serde_json::from_value(t.clone()).context("Failed to parse Fireflies transcript")?;

        Ok(ff.into_new_transcript())
    }
}

#[derive(Debug, Deserialize)]
struct FirefliesTranscript {
    id: String,
    title: Option<String>,
    date: Option<serde_json::Value>, // epoch ms (number or string)
    duration: Option<f64>,           // minutes
    organizer_email: Option<String>,
    participants: Option<Vec<String>>,
    summary: Option<FirefliesSummary>,
    sentences: Option<Vec<FirefliesSentence>>,
}

#[derive(Debug, Deserialize)]
struct FirefliesSummary {
    keywords: Option<Vec<String>>,
    action_items: Option<String>, // text block, not structured
    overview: Option<String>,
    shorthand_bullet: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FirefliesSentence {
    text: Option<String>,
    speaker_name: Option<String>,
    start_time: Option<f64>,
    end_time: Option<f64>,
}

impl FirefliesTranscript {
    fn into_new_transcript(self) -> NewTranscript {
        let title = self.title.unwrap_or_else(|| "Untitled".to_string());

        // Date: epoch ms → ISO-8601
        let date_ms = self
            .date
            .as_ref()
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(0);
        let date = epoch_ms_to_iso(date_ms);

        // Duration: minutes → seconds
        let duration_seconds = self.duration.unwrap_or(0.0) * 60.0;

        // Sentences → segments
        let sentences = self.sentences.unwrap_or_default();
        let mut segments = Vec::new();
        let mut speakers_set = HashSet::new();
        let mut raw_lines = Vec::new();

        for (i, s) in sentences.iter().enumerate() {
            let speaker = s
                .speaker_name
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
                start_time: s.start_time.unwrap_or(0.0),
                end_time: s.end_time.unwrap_or(0.0),
                segment_index: i as i64,
            });
        }

        let speakers: Vec<String> = speakers_set.into_iter().collect();
        let raw_text = raw_lines.join("\n");

        // Summary
        let mut summary_parts = Vec::new();
        let mut keywords = Vec::new();
        let mut action_items = Vec::new();

        if let Some(ref sum) = self.summary {
            if let Some(ref overview) = sum.overview {
                if !overview.is_empty() {
                    summary_parts.push(overview.clone());
                }
            }
            if let Some(ref bullet) = sum.shorthand_bullet {
                if !bullet.is_empty() {
                    summary_parts.push(bullet.clone());
                }
            }
            if let Some(ref kw) = sum.keywords {
                keywords = kw.clone();
            }
            if let Some(ref ai_text) = sum.action_items {
                // Parse action_items text block: skip **header** lines and short lines
                for line in ai_text.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.len() < 5 {
                        continue;
                    }
                    if trimmed.starts_with("**") && trimmed.ends_with("**") {
                        continue;
                    }
                    // Strip leading bullet/dash
                    let clean = trimmed
                        .trim_start_matches('-')
                        .trim_start_matches('•')
                        .trim_start_matches('*')
                        .trim();
                    if !clean.is_empty() {
                        action_items.push(NewActionItem {
                            text: clean.to_string(),
                            metadata: None,
                        });
                    }
                }
            }
        }

        let summary = summary_parts.join("\n\n");

        // Metadata
        let mut meta = serde_json::Map::new();
        if let Some(email) = self.organizer_email {
            meta.insert("organizer_email".into(), serde_json::Value::String(email));
        }
        if let Some(participants) = self.participants {
            meta.insert(
                "participants".into(),
                serde_json::Value::Array(
                    participants
                        .into_iter()
                        .map(serde_json::Value::String)
                        .collect(),
                ),
            );
        }
        let metadata = if meta.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(meta))
        };

        NewTranscript {
            id: self.id,
            title,
            date,
            duration_seconds,
            source: "fireflies".to_string(),
            summary,
            raw_text,
            metadata,
            speakers,
            segments,
            tags: Vec::new(),
            keywords,
            action_items,
        }
    }
}

fn epoch_ms_to_iso(ms: i64) -> String {
    let secs = ms / 1000;
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}
