pub mod json;
pub mod markdown;
pub mod migrate;
pub mod text;

use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::Path;
use tracing::info;

use crate::db::models::NewTranscript;
use crate::db::Database;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Format {
    Json,
    Markdown,
    Text,
}

impl Format {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "json" => Some(Format::Json),
            "markdown" | "md" => Some(Format::Markdown),
            "text" | "txt" => Some(Format::Text),
            _ => None,
        }
    }

    pub fn detect_from_extension(path: &Path) -> Option<Self> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("json") => Some(Format::Json),
            Some("md" | "markdown") => Some(Format::Markdown),
            Some("txt" | "text") => Some(Format::Text),
            _ => None,
        }
    }
}

/// Ingest one or more paths (files or directories). Returns count of ingested transcripts.
pub fn ingest_paths(
    db: &Database,
    paths: &[String],
    default_source: Option<&str>,
    format_override: Option<Format>,
    dry_run: bool,
) -> Result<usize> {
    let mut count = 0;

    for path_str in paths {
        let path = Path::new(path_str);
        if path.is_dir() {
            count += ingest_directory(db, path, default_source, format_override, dry_run)?;
        } else if path.is_file() {
            count += ingest_file(db, path, default_source, format_override, dry_run)?;
        } else {
            // Try glob pattern
            let matches: Vec<_> = glob::glob(path_str)
                .with_context(|| format!("Invalid path or glob pattern: {path_str}"))?
                .filter_map(|r| r.ok())
                .collect();

            if matches.is_empty() {
                bail!("No files found matching: {path_str}");
            }

            for entry in matches {
                if entry.is_file() {
                    count += ingest_file(db, &entry, default_source, format_override, dry_run)?;
                }
            }
        }
    }

    Ok(count)
}

/// Ingest from stdin.
pub fn ingest_stdin(
    db: &Database,
    default_source: Option<&str>,
    format_override: Option<Format>,
    dry_run: bool,
) -> Result<usize> {
    let mut content = String::new();
    std::io::stdin()
        .read_to_string(&mut content)
        .context("Failed to read from stdin")?;

    if content.trim().is_empty() {
        bail!("Empty input from stdin");
    }

    let format = format_override.unwrap_or_else(|| {
        // Try to detect: if it starts with { it's JSON, if it starts with --- it's markdown
        let trimmed = content.trim();
        if trimmed.starts_with('{') {
            Format::Json
        } else if trimmed.starts_with("---") {
            Format::Markdown
        } else {
            Format::Text
        }
    });

    let transcript = parse_content(&content, "stdin", format, default_source)?;

    if dry_run {
        println!(
            "  [dry-run] Would ingest: {} ({}, {} segments)",
            transcript.title,
            transcript.source,
            transcript.segments.len()
        );
        return Ok(1);
    }

    db.insert_transcript(&transcript)?;
    info!("Ingested from stdin: {}", transcript.title);
    Ok(1)
}

fn ingest_directory(
    db: &Database,
    dir: &Path,
    default_source: Option<&str>,
    format_override: Option<Format>,
    dry_run: bool,
) -> Result<usize> {
    let mut count = 0;

    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            count += ingest_directory(db, &path, default_source, format_override, dry_run)?;
        } else if path.is_file() {
            // Only process known extensions unless format is overridden
            if format_override.is_some() || Format::detect_from_extension(&path).is_some() {
                count += ingest_file(db, &path, default_source, format_override, dry_run)?;
            }
        }
    }

    Ok(count)
}

fn ingest_file(
    db: &Database,
    path: &Path,
    default_source: Option<&str>,
    format_override: Option<Format>,
    dry_run: bool,
) -> Result<usize> {
    let format = format_override
        .or_else(|| Format::detect_from_extension(path))
        .with_context(|| format!("Cannot determine format for: {}", path.display()))?;

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read: {}", path.display()))?;

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let transcript = parse_content(&content, filename, format, default_source)?;

    if dry_run {
        println!(
            "  [dry-run] Would ingest: {} ({}, {} segments)",
            transcript.title,
            transcript.source,
            transcript.segments.len()
        );
        return Ok(1);
    }

    if db.transcript_exists(&transcript.id)? {
        info!("Skipping duplicate: {} ({})", transcript.title, transcript.id);
        return Ok(0);
    }

    db.insert_transcript(&transcript)?;
    info!("Ingested: {} ({})", transcript.title, path.display());
    Ok(1)
}

fn parse_content(
    content: &str,
    filename: &str,
    format: Format,
    default_source: Option<&str>,
) -> Result<NewTranscript> {
    match format {
        Format::Json => json::parse_json(content, default_source),
        Format::Markdown => markdown::parse_markdown(content, filename, default_source),
        Format::Text => {
            text::parse_text(content, Path::new(filename), default_source)
        }
    }
}
