use anyhow::Result;

use crate::db::models::{NewSegment, NewTranscript};

/// Parse a markdown file with optional YAML frontmatter into a NewTranscript.
///
/// Expected format:
/// ```
/// ---
/// title: Meeting Title
/// date: 2026-01-15
/// source: manual
/// tags: [tag1, tag2]
/// speakers: [Alice, Bob]
/// ---
///
/// ## Alice (00:30)
/// Some text here.
///
/// ## Bob (01:15)
/// Response text.
/// ```
pub fn parse_markdown(content: &str, filename: &str, default_source: Option<&str>) -> Result<NewTranscript> {
    let (frontmatter, body) = split_frontmatter(content);

    // Parse frontmatter if present
    let mut title = filename_to_title(filename);
    let mut date = String::new();
    let mut source = default_source.unwrap_or("markdown").to_string();
    let mut tags: Vec<String> = Vec::new();
    let mut speakers: Vec<String> = Vec::new();
    let mut metadata: Option<serde_json::Value> = None;

    if let Some(fm) = frontmatter {
        if let Ok(yaml) = serde_yaml::from_str::<serde_json::Value>(&fm) {
            if let Some(obj) = yaml.as_object() {
                if let Some(v) = obj.get("title").and_then(|v| v.as_str()) {
                    title = v.to_string();
                }
                if let Some(v) = obj.get("date").and_then(|v| v.as_str()) {
                    date = v.to_string();
                }
                if let Some(v) = obj.get("source").and_then(|v| v.as_str()) {
                    source = v.to_string();
                }
                if let Some(arr) = obj.get("tags").and_then(|v| v.as_array()) {
                    tags = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                }
                if let Some(arr) = obj.get("speakers").and_then(|v| v.as_array()) {
                    speakers = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                }

                // Store remaining frontmatter fields as metadata
                let mut extra = serde_json::Map::new();
                for (k, v) in obj {
                    if !["title", "date", "source", "tags", "speakers"].contains(&k.as_str()) {
                        extra.insert(k.clone(), v.clone());
                    }
                }
                if !extra.is_empty() {
                    metadata = Some(serde_json::Value::Object(extra));
                }
            }
        }
    }

    // Parse body for segments
    let segments = parse_speaker_segments(body);

    // If no speaker segments found, treat entire body as one segment
    let segments = if segments.is_empty() && !body.trim().is_empty() {
        vec![NewSegment {
            speaker: String::new(),
            text: body.trim().to_string(),
            start_time: 0.0,
            end_time: 0.0,
            segment_index: 0,
        }]
    } else {
        segments
    };

    // Collect speakers from segments if not in frontmatter
    if speakers.is_empty() {
        let mut seen = std::collections::HashSet::new();
        for seg in &segments {
            if !seg.speaker.is_empty() && seen.insert(seg.speaker.clone()) {
                speakers.push(seg.speaker.clone());
            }
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    let raw_text = body.trim().to_string();

    Ok(NewTranscript {
        id,
        title,
        date,
        duration_seconds: 0.0,
        source,
        summary: String::new(),
        raw_text,
        metadata,
        speakers,
        segments,
        tags,
        keywords: Vec::new(),
        action_items: Vec::new(),
    })
}

fn split_frontmatter(content: &str) -> (Option<String>, &str) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (None, content);
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    if let Some(end) = after_first.find("\n---") {
        let fm = after_first[..end].trim().to_string();
        let body_start = 3 + end + 4; // skip past closing ---\n
        let body = if body_start < trimmed.len() {
            &trimmed[body_start..]
        } else {
            ""
        };
        (Some(fm), body)
    } else {
        (None, content)
    }
}

fn filename_to_title(filename: &str) -> String {
    let stem = std::path::Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    stem.replace(['-', '_'], " ")
}

/// Parse `## Speaker (MM:SS)` headings into segments.
fn parse_speaker_segments(body: &str) -> Vec<NewSegment> {
    // Match: ## Speaker Name (MM:SS) or ## Speaker Name (HH:MM:SS)
    let heading_re = regex::Regex::new(
        r"(?m)^##\s+(.+?)\s*(?:\((\d{1,2}:\d{2}(?::\d{2})?)\))?\s*$"
    ).unwrap();

    let mut segments = Vec::new();
    let mut matches: Vec<(usize, usize, String, f64)> = Vec::new();

    for cap in heading_re.captures_iter(body) {
        let full_match: regex::Match = cap.get(0).unwrap();
        let speaker = cap[1].trim().to_string();
        let timestamp = cap
            .get(2)
            .map(|m: regex::Match| parse_timestamp(m.as_str()))
            .unwrap_or(0.0);
        matches.push((full_match.start(), full_match.end(), speaker, timestamp));
    }

    for (i, (_start, end, speaker, timestamp)) in matches.iter().enumerate() {
        let text_start = *end;
        let text_end = if i + 1 < matches.len() {
            matches[i + 1].0
        } else {
            body.len()
        };

        let text = body[text_start..text_end].trim().to_string();
        if !text.is_empty() {
            segments.push(NewSegment {
                speaker: speaker.clone(),
                text,
                start_time: *timestamp,
                end_time: 0.0,
                segment_index: i as i64,
            });
        }
    }

    // Set end_time from next segment's start_time
    for i in 0..segments.len().saturating_sub(1) {
        segments[i].end_time = segments[i + 1].start_time;
    }

    segments
}

fn parse_timestamp(ts: &str) -> f64 {
    let parts: Vec<&str> = ts.split(':').collect();
    match parts.len() {
        2 => {
            let m: f64 = parts[0].parse().unwrap_or(0.0);
            let s: f64 = parts[1].parse().unwrap_or(0.0);
            m * 60.0 + s
        }
        3 => {
            let h: f64 = parts[0].parse().unwrap_or(0.0);
            let m: f64 = parts[1].parse().unwrap_or(0.0);
            let s: f64 = parts[2].parse().unwrap_or(0.0);
            h * 3600.0 + m * 60.0 + s
        }
        _ => 0.0,
    }
}
