# tss — Transcript Search

Fast CLI for indexing and searching meeting transcripts. Built on SQLite FTS5 — zero runtime dependencies, single static binary.

## Install

```bash
git clone https://github.com/krasmussen37/tss.git
cd tss
cargo build --release
cp target/release/tss ~/.local/bin/
```

Requires Rust toolchain. Database auto-creates at `~/.tss/tss.db` on first run.

## Usage

### Ingest transcripts

```bash
tss ingest meetings/                          # directory of JSON/md/txt files
tss ingest transcript.json                    # single file
tss ingest *.md --source zoom                 # override source label
echo '{"title":"Quick note","raw_text":"..."}' | tss ingest --stdin
tss ingest meeting.json --dry-run             # preview without importing
```

Supported formats:

- **JSON** — native format with segments, speakers, tags, keywords, action items
- **Markdown** — YAML frontmatter + `## Speaker (MM:SS)` headings for segments
- **Plain text** — title from filename, date from mtime, body as single segment

### Search

```bash
tss search "quarterly review"                 # full-text search across transcripts
tss search "auth" --speaker "Alice"           # filter by speaker
tss search "roadmap" --source zoom            # filter by source
tss search "budget" --from 2025-01-01 --to 2025-06-30
tss search "onboarding" --tag engineering
tss search "deploy" --segments                # search at segment level
tss search "pricing" --limit 5 --json         # JSON output
```

FTS5 supports phrase queries (`"exact phrase"`), boolean operators (`word1 OR word2`), prefix matching (`deploy*`), and column filters (`title:roadmap`).

### Browse

```bash
tss list                                      # all transcripts, newest first
tss list --source otter --limit 10            # filtered listing
tss list --sort title                         # sort alphabetically
tss show <id>                                 # transcript details, summary, action items
tss expand <id>                               # full segments with speaker attribution
tss expand <id> --speaker "Bob"               # filter to one speaker
```

### Manage

```bash
tss stats                                     # counts, sources, db size
tss info                                      # version, schema, db path
tss delete <id>                               # remove a transcript
tss reindex                                   # rebuild FTS5 indexes
```

### Migrate from legacy DB

```bash
tss migrate /path/to/transcripts.db           # import from Python transcript DB
tss migrate /path/to/transcripts.db --dry-run # preview
```

## Global flags

| Flag | Description |
|------|-------------|
| `--json` | Machine-readable JSON output on all commands |
| `--db <path>` | Use a specific database file (default: `~/.tss/tss.db`) |

The `TSS_DB` environment variable also sets the database path.

## JSON ingest format

```json
{
  "id": "unique-id",
  "title": "Weekly Standup",
  "date": "2025-03-15T10:00:00Z",
  "duration_seconds": 1800,
  "source": "zoom",
  "summary": "Discussed sprint progress and blockers.",
  "raw_text": "Full concatenated transcript text...",
  "speakers": [
    {"name": "Alice"},
    {"name": "Bob"}
  ],
  "segments": [
    {"speaker": "Alice", "text": "Let's get started.", "start": 0.0, "end": 2.5},
    {"speaker": "Bob", "text": "Sounds good.", "start": 3.0, "end": 4.2}
  ],
  "tags": ["standup", "engineering"],
  "keywords": ["sprint", "blockers"],
  "action_items": [
    {"text": "Alice to update the dashboard by Friday"}
  ]
}
```

All fields except `raw_text` are optional. IDs are auto-generated if omitted.

## Markdown ingest format

```markdown
---
title: Weekly Standup
date: 2025-03-15
source: zoom
tags: [standup, engineering]
speakers: [Alice, Bob]
---

## Alice (00:00)
Let's get started with updates.

## Bob (00:30)
The API migration is on track.
```

Frontmatter is optional. Without `## Speaker (timestamp)` headings, the body is stored as a single segment.

## License

MIT
