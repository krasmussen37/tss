use unicode_width::UnicodeWidthStr;

use crate::db::models::*;
use crate::search::{SegmentResult, TranscriptResult};

/// Format duration in seconds to human-readable string.
pub fn format_duration(seconds: f64) -> String {
    let total = seconds as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Truncate a string to fit within max_width (respecting unicode width).
fn truncate(s: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(s) <= max_width {
        return s.to_string();
    }
    let mut result = String::new();
    let mut width = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + cw + 3 > max_width {
            result.push_str("...");
            break;
        }
        result.push(ch);
        width += cw;
    }
    result
}

/// Format transcript search results as a table.
pub fn print_transcript_results(results: &[TranscriptResult], query: &str) {
    if results.is_empty() {
        println!("No results for \"{query}\"");
        return;
    }

    println!(
        "{} result{} for \"{}\":\n",
        results.len(),
        if results.len() == 1 { "" } else { "s" },
        query
    );

    // Header
    println!(
        "  {:<42} {:<12} {:<10} {:<8}",
        "TITLE", "DATE", "SOURCE", "DURATION"
    );
    println!("  {}", "-".repeat(76));

    for r in results {
        let date_short = r.date.get(..10).unwrap_or(&r.date);
        println!(
            "  {:<42} {:<12} {:<10} {:<8}",
            truncate(&r.title, 40),
            date_short,
            r.source,
            format_duration(r.duration_seconds),
        );

        // Snippet
        if !r.snippet.is_empty() {
            let snippet = r.snippet.replace('\n', " ");
            println!("  {}", truncate(&format!("  {snippet}"), 76));
        }

        println!("  id: {}\n", r.id);
    }
}

/// Format segment search results as a table.
pub fn print_segment_results(results: &[SegmentResult], query: &str) {
    if results.is_empty() {
        println!("No segment results for \"{query}\"");
        return;
    }

    println!(
        "{} segment{} for \"{}\":\n",
        results.len(),
        if results.len() == 1 { "" } else { "s" },
        query
    );

    for r in results {
        let time = format_timestamp(r.start_time);
        let text = truncate(&r.text.replace('\n', " "), 80);
        println!("  [{time}] {}: {text}", r.speaker);
        println!(
            "  └─ {} ({})\n",
            truncate(&r.transcript_title, 50),
            r.transcript_id
        );
    }
}

/// Format transcript list as a table.
pub fn print_transcript_list(results: &[TranscriptResult]) {
    if results.is_empty() {
        println!("No transcripts found.");
        return;
    }

    println!("{} transcript{}:\n", results.len(), if results.len() == 1 { "" } else { "s" });

    println!(
        "  {:<42} {:<12} {:<10} {:<8}",
        "TITLE", "DATE", "SOURCE", "DURATION"
    );
    println!("  {}", "-".repeat(76));

    for r in results {
        let date_short = r.date.get(..10).unwrap_or(&r.date);
        println!(
            "  {:<42} {:<12} {:<10} {:<8}",
            truncate(&r.title, 40),
            date_short,
            r.source,
            format_duration(r.duration_seconds),
        );
        println!("  id: {}\n", r.id);
    }
}

/// Format a single transcript's details for `tss show`.
pub fn print_transcript_detail(t: &Transcript, speakers: &[String], tags: &[String], keywords: &[String], action_items: &[ActionItem], segment_count: usize) {
    println!("Transcript: {}", t.title);
    println!("  ID:       {}", t.id);
    println!("  Date:     {}", t.date);
    println!("  Source:   {}", t.source);
    println!("  Duration: {}", format_duration(t.duration_seconds));
    println!("  Segments: {segment_count}");

    if !speakers.is_empty() {
        println!("  Speakers: {}", speakers.join(", "));
    }
    if !tags.is_empty() {
        println!("  Tags:     {}", tags.join(", "));
    }
    if !keywords.is_empty() {
        println!("  Keywords: {}", truncate(&keywords.join(", "), 72));
    }

    if !t.summary.is_empty() {
        println!("\nSummary:");
        for line in t.summary.lines() {
            println!("  {line}");
        }
    }

    if !action_items.is_empty() {
        println!("\nAction Items ({}):", action_items.len());
        for ai in action_items {
            println!("  - {}", truncate(&ai.text, 76));
        }
    }
}

/// Format segments for `tss expand`.
pub fn print_segments(segments: &[Segment], speaker_filter: Option<&str>) {
    if segments.is_empty() {
        println!("No segments found.");
        return;
    }

    let filtered: Vec<&Segment> = if let Some(speaker) = speaker_filter {
        let lower = speaker.to_lowercase();
        segments
            .iter()
            .filter(|s| s.speaker.to_lowercase().contains(&lower))
            .collect()
    } else {
        segments.iter().collect()
    };

    println!("{} segment{}:\n", filtered.len(), if filtered.len() == 1 { "" } else { "s" });

    let mut last_speaker = String::new();
    for seg in &filtered {
        let time = format_timestamp(seg.start_time);
        if seg.speaker != last_speaker {
            if !last_speaker.is_empty() {
                println!();
            }
            println!("  {} [{time}]:", seg.speaker);
            last_speaker = seg.speaker.clone();
        }
        println!("    {}", seg.text);
    }
    println!();
}

/// Print database stats.
pub fn print_stats(stats: &DbStats) {
    println!("Database Statistics:");
    println!("  Transcripts:  {}", stats.transcripts);
    println!("  Segments:     {}", stats.segments);
    println!("  Speakers:     {}", stats.speakers);
    println!("  Tags:         {}", stats.tags);
    println!("  Keywords:     {}", stats.keywords);
    println!("  Action Items: {}", stats.action_items);
    println!("  DB Size:      {}", format_bytes(stats.db_size_bytes));
    println!("\n  Sources:");
    for sc in &stats.sources {
        println!("    {:<16} {}", sc.source, sc.count);
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn format_timestamp(seconds: f64) -> String {
    let total = seconds as u64;
    let m = total / 60;
    let s = total % 60;
    format!("{m:02}:{s:02}")
}
