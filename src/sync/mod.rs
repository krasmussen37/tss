pub mod fireflies;
pub mod pocket;
pub mod state;

use anyhow::{bail, Result};
use std::collections::HashSet;
use std::time::Instant;

use crate::db::models::NewTranscript;
use crate::db::Database;

/// A remote transcript listing entry (lightweight, no full content).
#[derive(Debug, Clone)]
pub struct RemoteTranscript {
    pub id: String,
    pub title: String,
    pub date: String,
}

/// Trait that all source connectors implement.
pub trait TranscriptConnector {
    /// Connector name (used as source label in DB).
    fn name(&self) -> &str;

    /// List remote transcripts. If `since` is Some, only return transcripts
    /// newer than that ISO-8601 timestamp. If None, return all.
    fn list_remote(&self, since: Option<&str>) -> Result<Vec<RemoteTranscript>>;

    /// Fetch full transcript data for a single ID, transform to NewTranscript.
    fn fetch_one(&self, id: &str) -> Result<NewTranscript>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SyncMode {
    Initial,
    Incremental,
    Audit,
}

impl SyncMode {
    pub fn as_str(&self) -> &str {
        match self {
            SyncMode::Initial => "initial",
            SyncMode::Incremental => "incremental",
            SyncMode::Audit => "audit",
        }
    }
}

/// Result of an initial or incremental sync.
pub struct SyncReport {
    pub source: String,
    pub mode: SyncMode,
    pub remote_total: usize,
    pub already_local: usize,
    pub synced: usize,
    pub skipped: usize,
    pub failed: usize,
    pub duration_secs: f64,
}

/// Result of an audit comparison.
pub struct AuditReport {
    pub source: String,
    pub remote_total: usize,
    pub local_total: usize,
    pub missing_locally: Vec<RemoteTranscript>,
    pub orphaned_locally: Vec<String>,
}

pub struct SyncOptions {
    pub yes: bool,
    pub dry_run: bool,
}

/// Run an initial or incremental sync.
pub fn run_sync(
    connector: &dyn TranscriptConnector,
    db: &Database,
    mode: SyncMode,
    opts: &SyncOptions,
) -> Result<SyncReport> {
    let start = Instant::now();
    let source = connector.name().to_string();

    // Determine cursor
    let cursor = match mode {
        SyncMode::Incremental => {
            let key = format!("{}.last_sync_at", source);
            state::get_sync_state(&db.conn, &key)?
        }
        _ => None,
    };

    // List remote transcripts
    eprintln!("Listing remote transcripts...");
    let remote = connector.list_remote(cursor.as_deref())?;
    let remote_total = remote.len();

    // Diff against local DB
    let mut new_transcripts = Vec::new();
    let mut already_local = 0usize;
    for rt in &remote {
        if db.transcript_exists(&rt.id)? {
            already_local += 1;
        } else {
            new_transcripts.push(rt);
        }
    }

    let new_count = new_transcripts.len();

    if new_count == 0 {
        eprintln!("  Found {} transcripts, {} already synced, 0 new", remote_total, already_local);
        let run_id = state::start_sync_run(&db.conn, &source, mode.as_str())?;
        state::complete_sync_run(&db.conn, run_id, remote_total, 0, already_local, 0, "completed")?;
        // Update cursor even if nothing new (we checked)
        if mode == SyncMode::Initial || mode == SyncMode::Incremental {
            let key = format!("{}.last_sync_at", source);
            let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            state::set_sync_state(&db.conn, &key, &now)?;
        }
        return Ok(SyncReport {
            source,
            mode,
            remote_total,
            already_local,
            synced: 0,
            skipped: 0,
            failed: 0,
            duration_secs: start.elapsed().as_secs_f64(),
        });
    }

    // Show summary
    if !remote.is_empty() {
        let dates: Vec<&str> = remote.iter().map(|r| r.date.as_str()).collect();
        let min_date = dates.iter().min().unwrap_or(&"?");
        let max_date = dates.iter().max().unwrap_or(&"?");
        eprintln!(
            "  Found {} transcripts ({} to {})",
            remote_total, min_date, max_date
        );
        eprintln!(
            "  Local: {} already synced, {} new",
            already_local, new_count
        );
    }

    if opts.dry_run {
        eprintln!("\n[dry-run] Would sync {} transcripts", new_count);
        for (i, rt) in new_transcripts.iter().enumerate() {
            eprintln!(
                "  [{:>3}/{}] {} ({})",
                i + 1,
                new_count,
                rt.title,
                &rt.date[..10.min(rt.date.len())]
            );
        }
        return Ok(SyncReport {
            source,
            mode,
            remote_total,
            already_local,
            synced: 0,
            skipped: new_count,
            failed: 0,
            duration_secs: start.elapsed().as_secs_f64(),
        });
    }

    // Confirmation for initial sync
    if mode == SyncMode::Initial && !opts.yes {
        eprintln!("\nInitial sync: {} transcripts to download.", new_count);
        eprint!("Proceed? [Y/n] ");
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        let answer = answer.trim();
        if !answer.is_empty() && !answer.eq_ignore_ascii_case("y") {
            eprintln!("Cancelled.");
            return Ok(SyncReport {
                source,
                mode,
                remote_total,
                already_local,
                synced: 0,
                skipped: new_count,
                failed: 0,
                duration_secs: start.elapsed().as_secs_f64(),
            });
        }
    }

    // Download and insert
    let run_id = state::start_sync_run(&db.conn, &source, mode.as_str())?;
    let mut synced = 0usize;
    let mut failed = 0usize;
    let width = format!("{}", new_count).len();

    for (i, rt) in new_transcripts.iter().enumerate() {
        match connector.fetch_one(&rt.id) {
            Ok(transcript) => {
                let seg_count = transcript.segments.len();
                let ai_count = transcript.action_items.len();
                match db.insert_transcript(&transcript) {
                    Ok(()) => {
                        synced += 1;
                        eprintln!(
                            "  [{:>width$}/{}] {} ({}) — {} segments, {} action items",
                            i + 1,
                            new_count,
                            rt.title,
                            &rt.date[..10.min(rt.date.len())],
                            seg_count,
                            ai_count,
                        );
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!(
                            "  [{:>width$}/{}] FAILED to insert {}: {}",
                            i + 1,
                            new_count,
                            rt.title,
                            e,
                        );
                    }
                }
            }
            Err(e) => {
                failed += 1;
                eprintln!(
                    "  [{:>width$}/{}] FAILED to fetch {}: {}",
                    i + 1,
                    new_count,
                    rt.title,
                    e,
                );
            }
        }
    }

    let status = if failed > 0 && synced == 0 {
        "failed"
    } else {
        "completed"
    };
    state::complete_sync_run(
        &db.conn,
        run_id,
        remote_total,
        synced,
        already_local,
        failed,
        status,
    )?;

    // Update cursor
    let key = format!("{}.last_sync_at", source);
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    state::set_sync_state(&db.conn, &key, &now)?;

    let duration = start.elapsed().as_secs_f64();
    eprintln!(
        "\nSync complete: {} synced, {} failed ({:.1}s)",
        synced, failed, duration
    );

    Ok(SyncReport {
        source,
        mode,
        remote_total,
        already_local,
        synced,
        skipped: 0,
        failed,
        duration_secs: duration,
    })
}

/// Run an audit: full remote-vs-local reconciliation.
pub fn run_audit(
    connector: &dyn TranscriptConnector,
    db: &Database,
    opts: &SyncOptions,
) -> Result<AuditReport> {
    let source = connector.name().to_string();

    eprintln!("Scanning all remote transcripts...");
    let remote = connector.list_remote(None)?;
    let remote_total = remote.len();

    let remote_ids: HashSet<String> = remote.iter().map(|r| r.id.clone()).collect();
    let local_ids = state::get_local_ids_for_source(&db.conn, &source)?;
    let local_total = local_ids.len();

    // Missing locally: on remote but not in DB
    let missing_locally: Vec<RemoteTranscript> = remote
        .iter()
        .filter(|r| !local_ids.contains(&r.id))
        .cloned()
        .collect();

    // Orphaned locally: in DB but not on remote
    let orphaned_locally: Vec<String> = local_ids
        .iter()
        .filter(|id| !remote_ids.contains(*id))
        .cloned()
        .collect();

    eprintln!("  Remote: {} transcripts", remote_total);
    eprintln!("  Local:  {} transcripts (source={})", local_total, source);

    if missing_locally.is_empty() && orphaned_locally.is_empty() {
        eprintln!("\nNo discrepancies found. Local and remote are in sync.");
        let run_id = state::start_sync_run(&db.conn, &source, "audit")?;
        state::complete_sync_run(&db.conn, run_id, remote_total, 0, local_total, 0, "completed")?;
        return Ok(AuditReport {
            source,
            remote_total,
            local_total,
            missing_locally,
            orphaned_locally,
        });
    }

    eprintln!("\nDiscrepancies found:\n");

    if !missing_locally.is_empty() {
        eprintln!("  Missing locally ({}):", missing_locally.len());
        for rt in &missing_locally {
            eprintln!(
                "    {}  {:50} {}",
                &rt.id[..16.min(rt.id.len())],
                rt.title,
                &rt.date[..10.min(rt.date.len())]
            );
        }
    } else {
        eprintln!("  Missing locally: (none)");
    }

    eprintln!();

    if !orphaned_locally.is_empty() {
        eprintln!("  Orphaned locally ({}):", orphaned_locally.len());
        for id in &orphaned_locally {
            eprintln!("    {}", id);
        }
    } else {
        eprintln!("  Orphaned locally: (none)");
    }

    if opts.dry_run {
        eprintln!("\n[dry-run] No changes made.");
        return Ok(AuditReport {
            source,
            remote_total,
            local_total,
            missing_locally,
            orphaned_locally,
        });
    }

    // Prompt for action
    eprintln!();
    if !missing_locally.is_empty() {
        eprint!("Action? [s]ync missing / [j]son export / [n]othing: ");
    } else {
        eprint!("Action? [d]elete orphans / [j]son export / [n]othing: ");
    }

    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    let answer = answer.trim().to_lowercase();

    let run_id = state::start_sync_run(&db.conn, &source, "audit")?;

    match answer.as_str() {
        "s" if !missing_locally.is_empty() => {
            let total = missing_locally.len();
            let mut synced = 0usize;
            let mut errors = 0usize;
            for (i, rt) in missing_locally.iter().enumerate() {
                match connector.fetch_one(&rt.id) {
                    Ok(transcript) => match db.insert_transcript(&transcript) {
                        Ok(()) => {
                            synced += 1;
                            eprintln!("  [{}/{}] {}", i + 1, total, rt.title);
                        }
                        Err(e) => {
                            errors += 1;
                            eprintln!("  [{}/{}] FAILED: {}", i + 1, total, e);
                        }
                    },
                    Err(e) => {
                        errors += 1;
                        eprintln!("  [{}/{}] FAILED to fetch: {}", i + 1, total, e);
                    }
                }
            }
            state::complete_sync_run(
                &db.conn,
                run_id,
                remote_total,
                synced,
                local_total,
                errors,
                "completed",
            )?;
            eprintln!(
                "\nAudit complete: {} synced, {} orphans.",
                synced,
                orphaned_locally.len()
            );
        }
        "d" if !orphaned_locally.is_empty() => {
            let mut deleted = 0usize;
            for id in &orphaned_locally {
                match db.delete_transcript(id) {
                    Ok(true) => deleted += 1,
                    Ok(false) => eprintln!("  Warning: {} not found during delete", id),
                    Err(e) => eprintln!("  Error deleting {}: {}", id, e),
                }
            }
            state::complete_sync_run(
                &db.conn,
                run_id,
                remote_total,
                0,
                local_total,
                0,
                "completed",
            )?;
            eprintln!("\nDeleted {} orphaned transcripts.", deleted);
        }
        "j" => {
            let export = serde_json::json!({
                "source": source,
                "remote_total": remote_total,
                "local_total": local_total,
                "missing_locally": missing_locally.iter().map(|r| {
                    serde_json::json!({"id": r.id, "title": r.title, "date": r.date})
                }).collect::<Vec<_>>(),
                "orphaned_locally": orphaned_locally,
            });
            println!("{}", serde_json::to_string_pretty(&export)?);
            state::complete_sync_run(
                &db.conn,
                run_id,
                remote_total,
                0,
                local_total,
                0,
                "completed",
            )?;
        }
        _ => {
            eprintln!("No changes made.");
            state::complete_sync_run(
                &db.conn,
                run_id,
                remote_total,
                0,
                local_total,
                0,
                "completed",
            )?;
        }
    }

    Ok(AuditReport {
        source,
        remote_total,
        local_total,
        missing_locally,
        orphaned_locally,
    })
}

/// Build a connector from source name + resolved API key.
pub fn build_connector(
    source: &str,
    api_key: String,
    tag: Option<String>,
    config: Option<&crate::config::SourceConfig>,
    db: &Database,
) -> Result<Box<dyn TranscriptConnector>> {
    match source {
        "fireflies" => Ok(Box::new(fireflies::FirefliesConnector::new(api_key))),
        "pocket" => {
            let tag_name = tag.or_else(|| {
                config.and_then(|c| c.default_tag.clone())
            });
            let base_url = config.and_then(|c| c.base_url.clone());
            Ok(Box::new(pocket::PocketConnector::new(
                api_key, tag_name, base_url, db,
            )?))
        }
        _ => bail!("Unknown source: {}. Supported: fireflies, pocket", source),
    }
}
