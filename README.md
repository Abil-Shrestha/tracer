# Tracer

Lightweight issue tracker for AI agents. Tracks dependencies between tasks and coordinates multiple agents working on the same project.

## What It Does

- Track tasks with dependencies (task B blocks task A)
- Find work that's ready to start (no blockers)
- Multiple agents can work together and leave comments
- Git-based storage (JSONL files)

## Install

```bash
cargo install --git https://github.com/Abil-Shrestha/tracer
```

Requires Rust 1.89 or newer: https://rustup.rs/

## Usage

Generated issue IDs use `<prefix>-<32 lowercase hex characters>` (128 bits of OS
randomness), so independent clones can create issues without coordinating a counter.
Use the ID returned by `create` in later commands. Legacy numeric IDs such as `bd-1`
in the examples below and explicit IDs supplied with `create --id` remain valid and
are never remapped; duplicate IDs in a database are rejected.

```bash
tracer init                                    # Initialize in your project
tracer create "Task name" -p 1 -t feature     # Create issue
tracer ready                                   # See available work
tracer update bd-1 --status in_progress       # Start work
tracer comment bd-1 "Working on this"         # Leave comment
tracer close bd-1                              # Close issue
```

## Multi-Agent Coordination

```bash
# Agent 1 starts work
tracer --actor agent-1 update bd-1 --status in_progress
tracer comment bd-1 "Working on auth API"

# Agent 2 sees it
tracer show bd-1  # Shows assignee and comments
tracer comment bd-1 "I'll test it when ready"

# Agent 1 finishes
tracer close bd-1
```

Auto-assigns agent when status changes to in_progress. Comments show up in `tracer show`.

## Features

- Dependency tracking (blocks, parent-child, related, discovered-from)
- Multi-agent coordination via comments and auto-assign
- JSON output for AI agents (`--json` flag)
- Git-friendly storage (JSONL)
- Auto-discovers database like git does

## Commands

```bash
tracer create "Title" [-p priority] [-t type]
tracer list [--status STATUS]
tracer show <id>
tracer update <id> --status STATUS
tracer close <id>
tracer comment <id> "message"
tracer dep add <from> <to> --type TYPE
tracer ready
tracer stats
```

Add `--json` to any command for JSON output.

## Sync safety and recovery

SQLite holds local working state; `issues.jsonl` beside the database is the managed
Git snapshot. Use one database per directory. Other JSONL filenames are backups,
not automatically selected sync inputs; migrate an old custom filename with
`tracer import -i old-name.jsonl`. Keep the SQLite database until pending work is
published successfully.

Exports include all issue fields and timestamps, dependencies, labels, comments,
and audit events. Event `sync_id` is portable; the original numeric `id` is retained
but is not globally unique. Imports do not create synthetic audit events. Old JSONL
without labels/events is accepted and preserves those unrepresented collections
already in SQLite; it cannot restore data an old exporter omitted. Upgrade all
writers: old clients do not understand this lossless format.

Normal commands auto-import new IDs and identical records. **Changed existing
records require explicit resolution**, including changes received when the local
database is clean: exported edits on two clones can diverge, and timestamps or a
clean dirty flag cannot prove ancestry. Local pending edits survive an unchanged
incoming record. Imports validate the entire input, allow forward references, and
commit all rows and sync bookkeeping in one transaction, or change nothing.
Malformed JSONL, duplicate IDs, invalid references, and identity collisions abort
before command dispatch. A missing previously synchronized file also stops sync.

When sync stops, preserve both sides before choosing:

```bash
tracer export -o local-backup.jsonl     # Reads SQLite, bypassing auto-import
cp .trace/issues.jsonl incoming-backup.jsonl
# Edit a COPY into resolved.jsonl: remove Git markers and reconcile records.
tracer import -i resolved.jsonl --resolve incoming --dry-run
tracer import -i resolved.jsonl --resolve incoming
```

- `--resolve incoming` chooses the input's **whole record**, including empty
  collections/removals, for existing IDs. Omitting a previously synchronized ID
  deletes it; omitting a never-synchronized local ID preserves it. This can discard
  local edits/history: review the backup first. All retained references must exist.
- `--resolve local` keeps the local whole record on differences/deletions, while
  still adding new incoming IDs. Neither flag merges fields or comments for you.
- `--skip-existing` adds new IDs only and ignores omissions; it still validates
  input and rejects collisions. Use it for partial/filtered exports, not deletion.
- `--dry-run` runs the same validation/transaction and rolls it back without
  publishing. Explicit import/export bypass auto-import so recovery remains possible.
- The same issue ID with a different `created_at`, or a reused `sync_id` with
  different event contents, **cannot be forced**. Rename the colliding issue and
  its references in the resolved copy; preserve the correct original timestamps.

Local CLI cycles hold an OS advisory lock from before opening SQLite through
import, mutation, publication, and dirty acknowledgement. Do not delete the
`.tracer-sync.lock` file; a crash releases its kernel lock automatically. Do not
commit lock files, SQLite files, or `.tracer-export-*.tmp` files. Publication writes
a same-directory temporary file, fsyncs it, atomically renames it, then fsyncs the
directory before clearing dirty state. A publication error returns failure while
retaining local data; export a backup and retry after fixing the filesystem. A
crash after rename but before acknowledgement is safe to retry without new events.

This is **not cross-clone locking**, a distributed claim service, or an automatic
field-level merge. Keep Git operations/external file edits outside CLI cycles;
noncooperating writers are not covered by the lock. Library callers must also hold
`SyncSession` around the whole cycle. Durability requires a filesystem supporting
atomic same-directory rename and file/directory fsync (verified on local Linux);
unsupported operations fail rather than acknowledge an unsafe publication.

## Documentation

- [AGENTS.md](./AGENTS.md) - AI agent integration guide
- [MULTI_AGENT.md](./MULTI_AGENT.md) - Multi-agent coordination
- [CHANGELOG.md](./CHANGELOG.md) - Version history

## License

MIT
