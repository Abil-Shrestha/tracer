# Agent-facing CLI contracts

`tracer` and `tr` use the same CLI. Use `--json` for machine-readable output;
it is a global flag and can appear before or after the subcommand. IDs are
opaque strings. Pass them as separate arguments, and use `--` before positional
IDs that might start with a dash. Do not interpolate titles, comments, IDs, or
actors into shell code. Actor names are coordination labels, not authentication.

## Success and failure streams

Ordinary JSON commands write one JSON value to stdout and nothing to stderr on
success. Empty query results are `[]`, not errors. Keep accepting unknown object
fields; key order and JSON whitespace are not contracts. Text output is for
people, not a parsing API.

Exceptions: `export` writes JSONL (one complete synchronization record per line),
or writes its `--output` file and leaves stdout empty. `learn` remains human text
even with `--json`. Explicit `--help`, `help`, and `--version` remain human text and
exit successfully, including when combined with `--json` where Clap accepts it.

Errors in JSON mode write a single object to **stderr**, with a nonzero exit:

```json
{"error":{"code":"command_failed","message":"command_failed: Issue absent not found"}}
```

| Stable code | Exit | Meaning / response |
| --- | --- | --- |
| `invalid_arguments` | 2 | CLI parsing failed: missing arguments, unknown flags/subcommands, invalid enum/number, conflicting flags. Fix the invocation. |
| `database_error` | 1 | Discovery, sync-lock setup, or opening/configuring SQLite failed. Check the path, permissions, and the message. Context may need an existing cache. |
| `sync_import_failed` | 1 | Automatic import failed before dispatching the requested command. Resolve malformed/conflicting/missing managed JSONL; do not assume a mutation ran. |
| `command_failed` | 1 | Command validation, ownership checks, explicit import/export, or another command operation failed. Includes missing issues and older context schemas; inspect the message. |
| `sync_publish_failed` | 1 | The command ran but publishing JSONL or acknowledging publication failed. Local SQLite may already contain the mutation. Recover before retrying mutations. |
| `output_failed` | 1 | stdout could not be written/flushed after successful execution. The operation may already be durable; stdout can be incomplete. |

Codes classify failure **stages**, not every domain condition. In particular,
`command_failed` covers both missing issues and rejected claims. Message text is
human diagnostic information and may change. Argument failures are recognized
when an exact `--json` token appears before the positional `--` separator, even
if parsing fails before reaching it. A positional string `--json` is not a flag.

Database-command success output is buffered until the required local
import/mutate/publish/acknowledge cycle succeeds. A publication failure therefore
has empty stdout in both text and JSON modes, never a premature success message.
This is an **output guarantee, not a new transaction guarantee**: a command with
multiple writes can partially mutate SQLite before failing, as before.

After a publication failure, do not blindly repeat `create` or `comment`. Inspect
local data with `context` or back it up with `export -o /safe/path/backup.jsonl`
(outside the managed `issues.jsonl`). Fix the filesystem problem, then run a
normal command such as `ready --json` to publish pending changes without repeating
the mutation. If sync conflicts prevent normal commands, inspect both snapshots
and use explicit `import --dry-run` and, only after deciding which data to keep,
`import --resolve local|incoming`. Export and explicit import retain their P0
recovery bypass of automatic import. A failed stdout write also requires checking
state before retrying a mutation.

`init --json` returns:

```json
{"status":"initialized","database":".trace/bd.db","prefix":"bd","jsonl":".trace/issues.jsonl"}
```

Paths reflect the chosen initialization path. `init --path` selects that path;
initialization does not promise a JSONL snapshot already exists. It retains the
existing initialization behavior rather than publishing an empty snapshot over
incoming work.

`import --json` returns:

```json
{"status":"imported","changed":2,"removed":0,"dry_run":false}
```

`--dry-run` changes `status` to `dry_run` and `dry_run` to `true`. Counts describe
the validated proposed changes; the import is rolled back and no managed JSONL
is published. Non-dry-run success is emitted only after publication succeeds.

## Complete issue details and explicit history limits

```sh
tracer show --json -- ISSUE_ID
tracer show --full --json -- ISSUE_ID
```

`show` preserves existing issue fields **at the top level**: `id`, `title`,
`description`, `status`, `priority`, `issue_type`, `created_at`, `updated_at`,
and populated optional fields (`design`, `acceptance_criteria`, `notes`,
`assignee`, `estimated_minutes`, `closed_at`, `external_ref`). Existing omission
rules for empty optional issue fields remain unchanged. Timestamps are RFC 3339.

The following fields are always present in JSON details:

| Field | Shape / contract |
| --- | --- |
| `labels` | All label strings; `[]` if none. |
| `dependencies` | All outgoing relationship records, hydrated from storage. Each has `issue_id`, `depends_on_id`, `type`, `created_at`, `created_by`. Target IDs are not expanded into issue objects. |
| `events` | Latest 20 events by default; all events with `--full`. |
| `comments` | Latest 5 `commented` events by default; all with `--full`. Selected independently from the entire history, not just the 20-event window. |
| `events_truncated` | True only if older events were omitted. |
| `comments_truncated` | True only if older comments were omitted. |

Events include `id`, `issue_id`, `event_type`, `actor`, `created_at`, and populated
`old_value`, `new_value`, and `comment`. Arrays are newest first, with descending
numeric event ID as the timestamp tie-breaker. Event IDs need not be globally
unique after clone imports; use `export` for portable event identities and
lossless backup, not `show`. `comments` duplicates relevant events as a convenient
view; close reasons remain in `events` because they are not `commented` events.

`--full` removes **both** history limits and sets both truncation flags to false.
There is no pagination cursor. Labels, relationships, descriptions, and other
issue fields are never truncated by `show`. Text details display history counts
and a `show --full` recovery hint whenever a history window is truncated. Full
text history includes old/new values and comments.

## Compact query projections are opt-in

```sh
tracer list --compact --limit 5 --json
tracer ready --compact --limit 5 --json
```

Default `list --json` and `ready --json` retain their existing array of issue
records and existing filters/readiness semantics. `--compact --json` returns an
array with exactly this projection for each result:

```json
[{"id":"bd-example","title":"Repair parser","status":"open","priority":1,"issue_type":"bug","assignee":""}]
```

All six fields are always present, including empty `assignee`. JSON is compactly
serialized; text emits one escaped summary line per issue. Neither projection
includes descriptions, timestamps, labels, relationships, or history. Use `show`
for those details. Filtering and ordering are unchanged: priority ascending,
then creation time descending. Existing tie ordering is unspecified.

`--compact` alone does not limit result count. Query `--limit` remains optional;
`--limit 0` gives no results. To preserve the array contract, these commands do
not add a truncation wrapper. If a limited query returns exactly the requested
count, more results may exist; raise or omit `--limit` to recover them. `ready`
still may include open work assigned to other actors unless filtered. Observing
ready work is not a reservation; use `claim` and handle a losing race.

## Context observes the local cache without claiming or syncing

```sh
tracer --actor agent-1 context --limit 5 --json
```

Context opens an **existing SQLite cache read-only**, in one read transaction. It
does not initialize/migrate schema, import incoming JSONL, publish dirty data,
change assignments/statuses, or append events. It works even when the managed
JSONL is malformed. It labels `source` as `local_cache`: this is not a claim that
the cache matches the current Git checkout or the latest remote work. Incoming
JSONL may be unapplied and local dirty work may be unpublished. SQLite may use
its normal WAL/shared-memory sidecars; read-only means no data/schema mutation.

If the cache does not exist, context fails with actionable guidance instead of
silently initializing it. If an older schema cannot satisfy the queries, it fails
without migration. Run `ready --json` with the same database selection to
initialize/migrate and synchronize the cache, then retry context. Normal P0
conflict checks still apply to that refresh. Use `init --path ... --prefix ...`
when deliberately initializing a new project with a chosen prefix.

The JSON object has:

| Field | Contract |
| --- | --- |
| `actor` | `--actor`, else `TRACE_ACTOR`, `USER`, `USERNAME`, then `unknown`. |
| `source` | Always `local_cache`. |
| `limit` | Requested positive integer, default 5. |
| `owned` | `{ "items": [summary], "truncated": boolean }`: nonclosed issues assigned to this actor. An empty actor does not own unassigned work. |
| `ready` | Same section shape: open issues with no unfinished `blocks` dependencies, unassigned or assigned to this actor. |
| `blockers` | Same section shape: owned nonclosed issues with unfinished `blocks` dependencies or explicit `blocked` status. Each item extends the compact summary with `blocked_by` (target ID strings) and `blocked_by_truncated`. |
| `next_commands` | Human-readable guidance/templates for refresh, inspection, explicit claim, and expanding truncated views. Not executable shell code; replace `<ID>` and preserve the caller's `--db`/`--actor`. |

Each section independently shows at most `limit` items. Blocker ID lists also show
at most `limit`, sorted by ID, and signal omissions separately. Sections sort by
priority ascending, creation time descending, then ID ascending. Blockers are
computed from **all** owned work before applying the section limit, so truncating
owned work cannot hide the existence of a blocked item in the blockers section.
Closed targets and non-`blocks` relationships do not count as blockers. An
explicitly blocked issue with no blocking edges has `blocked_by: []`; inspect its
history for a human-supplied reason. Closed owned work is omitted.

Raise `context --limit` to recover omitted items; use `show --full` for every
relationship/history entry. Section limits bound output counts, not bytes or
query cost: summaries keep full titles, and context currently uses existing
storage queries before filtering/bounding. Likewise, default `show` bounds output
history but reads the issue's full event history to find comments independently.
Very large trackers may require a future storage-level paging API.

These additions do not change P0 claims, local cycle locking, sync conflict
resolution, dirty-state recovery, or cross-clone coordination limitations.
