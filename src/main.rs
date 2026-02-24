use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tss::config;
use tss::db::Database;
use tss::ingest;
use tss::output::{json as json_out, table};
use tss::search::filters::Filters;
use tss::sync;

#[derive(Parser)]
#[command(name = "tss", version, about = "Transcript Search — fast FTS5-powered search over meeting transcripts")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,

    /// Path to database file (default: ~/.tss/tss.db)
    #[arg(long, global = true, env = "TSS_DB")]
    db: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Search transcripts using full-text search
    Search {
        /// Search query (FTS5 syntax: phrases, boolean, prefix*)
        query: String,

        /// Filter by speaker name (partial match)
        #[arg(long)]
        speaker: Option<String>,

        /// Filter by source (e.g. zoom, otter, teams, fireflies)
        #[arg(long)]
        source: Option<String>,

        /// Filter by date range start (YYYY-MM-DD)
        #[arg(long)]
        from: Option<String>,

        /// Filter by date range end (YYYY-MM-DD)
        #[arg(long)]
        to: Option<String>,

        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,

        /// Search at segment level instead of transcript level
        #[arg(long)]
        segments: bool,

        /// Maximum results to return
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// List transcripts
    List {
        /// Filter by source
        #[arg(long)]
        source: Option<String>,

        /// Filter by speaker name (partial match)
        #[arg(long)]
        speaker: Option<String>,

        /// Filter by date range start
        #[arg(long)]
        from: Option<String>,

        /// Filter by date range end
        #[arg(long)]
        to: Option<String>,

        /// Sort by: date (default) or title
        #[arg(long, default_value = "date")]
        sort: String,

        /// Maximum results
        #[arg(long, default_value = "50")]
        limit: usize,
    },

    /// Show transcript details
    Show {
        /// Transcript ID
        id: String,
    },

    /// Expand transcript segments
    Expand {
        /// Transcript ID
        id: String,

        /// Filter segments by speaker
        #[arg(long)]
        speaker: Option<String>,

        /// Highlight segments matching query
        #[arg(long)]
        query: Option<String>,
    },

    /// Ingest transcripts from files or stdin
    Ingest {
        /// File or directory paths to ingest
        paths: Vec<String>,

        /// Read from stdin
        #[arg(long)]
        stdin: bool,

        /// Override source label
        #[arg(long)]
        source: Option<String>,

        /// Force format: json, markdown, text
        #[arg(long)]
        format: Option<String>,

        /// Preview without importing
        #[arg(long)]
        dry_run: bool,
    },

    /// Migrate from legacy Python transcripts.db
    Migrate {
        /// Path to the source transcripts.db
        db_path: PathBuf,

        /// Preview without importing
        #[arg(long)]
        dry_run: bool,
    },

    /// Show database statistics
    Stats,

    /// Delete a transcript
    Delete {
        /// Transcript ID
        id: String,

        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },

    /// Rebuild FTS5 indexes
    Reindex,

    /// Show database info
    Info,

    /// Sync transcripts from remote sources
    Sync {
        /// Source: fireflies, pocket
        source: String,

        /// Run full audit — compare all remote vs local, report discrepancies
        #[arg(long)]
        audit: bool,

        /// Force initial sync (ignore cursor, re-scan all)
        #[arg(long)]
        full: bool,

        /// Skip confirmation prompts
        #[arg(long, short)]
        yes: bool,

        /// Override API key
        #[arg(long)]
        api_key: Option<String>,

        /// Filter by tag (source-specific, e.g., pocket channel tag)
        #[arg(long)]
        tag: Option<String>,

        /// Preview: list what would be synced without downloading
        #[arg(long)]
        dry_run: bool,
    },

    /// Manage tss configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Create default config file with template
    Init,
    /// Show current configuration (redacts secrets)
    Show,
    /// Print config file path
    Path,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let json_output = cli.json;

    let db_path = cli
        .db
        .unwrap_or_else(|| Database::default_db_path().expect("Could not determine default DB path"));

    let db = Database::open(&db_path)?;

    match cli.command {
        Commands::Search {
            query,
            speaker,
            source,
            from,
            to,
            tag,
            segments,
            limit,
        } => {
            let filters = Filters {
                speaker,
                source,
                from_date: from,
                to_date: to,
                tag,
            };

            if segments {
                let results = db.search_segments(&query, &filters, limit)?;
                if json_output {
                    json_out::print_json(&serde_json::json!({
                        "query": query,
                        "total": results.len(),
                        "segments": results,
                    }))?;
                } else {
                    table::print_segment_results(&results, &query);
                }
            } else {
                let results = db.search_transcripts(&query, &filters, limit)?;
                if json_output {
                    json_out::print_json(&serde_json::json!({
                        "query": query,
                        "total": results.len(),
                        "transcripts": results,
                    }))?;
                } else {
                    table::print_transcript_results(&results, &query);
                }
            }
        }

        Commands::List {
            source,
            speaker,
            from,
            to,
            sort,
            limit,
        } => {
            let filters = Filters {
                speaker,
                source,
                from_date: from,
                to_date: to,
                tag: None,
            };
            let results = db.list_transcripts(&filters, &sort, limit)?;
            if json_output {
                json_out::print_json(&results)?;
            } else {
                table::print_transcript_list(&results);
            }
        }

        Commands::Show { id } => {
            let t = db
                .get_transcript(&id)?
                .with_context(|| format!("Transcript not found: {id}"))?;
            let speakers = db.get_speakers(&id)?;
            let tags = db.get_tags(&id)?;
            let keywords = db.get_keywords(&id)?;
            let action_items = db.get_action_items(&id)?;
            let segments = db.get_segments(&id)?;

            if json_output {
                json_out::print_json(&serde_json::json!({
                    "transcript": t,
                    "speakers": speakers,
                    "tags": tags,
                    "keywords": keywords,
                    "action_items": action_items,
                    "segment_count": segments.len(),
                }))?;
            } else {
                table::print_transcript_detail(&t, &speakers, &tags, &keywords, &action_items, segments.len());
            }
        }

        Commands::Expand {
            id,
            speaker,
            query: _,
        } => {
            let t = db
                .get_transcript(&id)?
                .with_context(|| format!("Transcript not found: {id}"))?;

            let segments = db.get_segments(&id)?;

            if json_output {
                json_out::print_json(&serde_json::json!({
                    "transcript_id": id,
                    "title": t.title,
                    "segments": segments,
                }))?;
            } else {
                println!("Transcript: {} ({})\n", t.title, id);
                table::print_segments(&segments, speaker.as_deref());
            }
        }

        Commands::Ingest {
            paths,
            stdin,
            source,
            format,
            dry_run,
        } => {
            let format_enum = format
                .as_deref()
                .map(|f| {
                    ingest::Format::from_str(f)
                        .with_context(|| format!("Unknown format: {f}. Use: json, markdown, text"))
                })
                .transpose()?;

            let count = if stdin {
                ingest::ingest_stdin(&db, source.as_deref(), format_enum, dry_run)?
            } else if paths.is_empty() {
                bail!("No paths provided. Use --stdin to read from stdin.");
            } else {
                ingest::ingest_paths(&db, &paths, source.as_deref(), format_enum, dry_run)?
            };

            let action = if dry_run { "Would ingest" } else { "Ingested" };
            println!("{action} {count} transcript{}", if count == 1 { "" } else { "s" });
        }

        Commands::Migrate { db_path: src, dry_run } => {
            println!("Migrating from: {}", src.display());
            let stats = ingest::migrate::migrate_from_python_db(&db, &src, dry_run)?;
            if dry_run {
                println!(
                    "\n[dry-run] Would import {} transcripts ({} already exist)",
                    stats.imported, stats.skipped
                );
            } else {
                println!(
                    "Imported {} transcripts ({} skipped as duplicates)",
                    stats.imported, stats.skipped
                );
            }
        }

        Commands::Stats => {
            let stats = db.stats()?;
            if json_output {
                json_out::print_json(&stats)?;
            } else {
                table::print_stats(&stats);
            }
        }

        Commands::Delete { id, force } => {
            let t = db
                .get_transcript(&id)?
                .with_context(|| format!("Transcript not found: {id}"))?;

            if !force {
                eprint!("Delete \"{}\" ({})? [y/N] ", t.title, id);
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    println!("Cancelled.");
                    return Ok(());
                }
            }

            db.delete_transcript(&id)?;
            println!("Deleted: {} ({})", t.title, id);
        }

        Commands::Reindex => {
            println!("Rebuilding FTS5 indexes...");
            db.reindex()?;
            println!("Done.");
        }

        Commands::Sync {
            source,
            audit,
            full,
            yes,
            api_key,
            tag,
            dry_run,
        } => {
            // Validate source name early
            let valid_sources = ["fireflies", "pocket"];
            if !valid_sources.contains(&source.as_str()) {
                bail!(
                    "Unknown source: {}. Supported: {}",
                    source,
                    valid_sources.join(", ")
                );
            }

            let cfg = config::TssConfig::load()?;
            let source_config = cfg.source_config(&source);

            // Resolve credential
            let env_var = format!("{}_API_KEY", source.to_uppercase());
            eprintln!("Resolving credentials...");
            let resolved_key =
                config::resolve_credential(api_key.as_deref(), &env_var, source_config)?;

            let connector = sync::build_connector(
                &source,
                resolved_key,
                tag,
                source_config,
                &db,
            )?;

            let opts = sync::SyncOptions { yes, dry_run };

            if audit {
                let report = sync::run_audit(connector.as_ref(), &db, &opts)?;
                if json_output {
                    json_out::print_json(&serde_json::json!({
                        "source": report.source,
                        "mode": "audit",
                        "remote_total": report.remote_total,
                        "local_total": report.local_total,
                        "missing_locally": report.missing_locally.len(),
                        "orphaned_locally": report.orphaned_locally.len(),
                    }))?;
                }
            } else {
                // Determine mode: audit flag checked above, so here it's initial or incremental
                let cursor_key = format!("{}.last_sync_at", source);
                let has_cursor = sync::state::get_sync_state(&db.conn, &cursor_key)?.is_some();

                let mode = if full || !has_cursor {
                    sync::SyncMode::Initial
                } else {
                    sync::SyncMode::Incremental
                };

                let report = sync::run_sync(connector.as_ref(), &db, mode, &opts)?;
                if json_output {
                    json_out::print_json(&serde_json::json!({
                        "source": report.source,
                        "mode": mode.as_str(),
                        "remote_total": report.remote_total,
                        "already_local": report.already_local,
                        "synced": report.synced,
                        "skipped": report.skipped,
                        "failed": report.failed,
                        "duration_secs": report.duration_secs,
                    }))?;
                }
            }
        }

        Commands::Config { action } => {
            match action {
                ConfigAction::Init => {
                    let created = config::init_config()?;
                    let path = config::config_path()?;
                    if created {
                        println!("Created config: {}", path.display());
                    } else {
                        println!("Config already exists: {}", path.display());
                    }
                }
                ConfigAction::Show => {
                    let cfg = config::TssConfig::load()?;
                    let path = config::config_path()?;
                    println!("Config: {}\n", path.display());
                    println!("{}", cfg.display_redacted());
                }
                ConfigAction::Path => {
                    let path = config::config_path()?;
                    println!("{}", path.display());
                }
            }
        }

        Commands::Info => {
            let stats = db.stats()?;
            let schema_ver: String = db
                .conn
                .query_row(
                    "SELECT value FROM tss_meta WHERE key = 'schema_version'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or_else(|_| "unknown".to_string());

            if json_output {
                json_out::print_json(&serde_json::json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "schema_version": schema_ver,
                    "db_path": db.path.display().to_string(),
                    "db_size_bytes": stats.db_size_bytes,
                    "transcripts": stats.transcripts,
                    "segments": stats.segments,
                }))?;
            } else {
                println!("tss v{}", env!("CARGO_PKG_VERSION"));
                println!("  Schema:      v{schema_ver}");
                println!("  Database:    {}", db.path.display());
                println!("  Size:        {}", format_bytes(stats.db_size_bytes));
                println!("  Transcripts: {}", stats.transcripts);
                println!("  Segments:    {}", stats.segments);
            }
        }
    }

    Ok(())
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
