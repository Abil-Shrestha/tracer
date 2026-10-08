# Multi-Agent Coordination

Tracer now supports simple multi-agent coordination through comments and automatic assignee tracking.

## Features

### 1. Commenting

Agents can leave comments on issues to communicate:

```bash
tracer comment bd-1 "Started working on the API"
tracer comment bd-1 "Need help with authentication"
```

Comments are shown in `tracer show`:

```bash
tracer show bd-1

bd-1 Implement auth API
Status: in_progress
Assignee: claude-1

Recent comments:
  cursor-2 (5 min ago): "Need help with authentication"
  claude-1 (15 min ago): "Started working on the API"
```

### 2. Exclusive local ownership

Use a distinct, nonempty actor name for each agent and claim work before starting:

```bash
# Actor comes from --actor, TRACE_ACTOR, or USER, in that order
tracer --actor claude-1 claim bd-1

# Compatible alias: a status-only update uses the same atomic claim
tracer --actor claude-1 update bd-1 --status in_progress

# Owner voluntarily gives the task back
tracer --actor claude-1 release bd-1

# Manual recovery after confirming the previous worker has stopped
tracer --actor coordinator release bd-1 --force
```

Claim checks readiness and sets `assignee` and `in_progress` in one SQLite
transaction. It accepts open work with no owner (or already assigned to you).
It rejects another owner's assignment, closed or explicitly blocked work, and
any unfinished direct `blocks` dependency. Other dependency types do not block.
An unowned `in_progress` task is not ready: recover it with an administrative
status update to `open` before claiming. The same owner's retry on unblocked
`in_progress` work succeeds without changing timestamps or adding events.
Conflicts exit nonzero with the owner or blocking reason and leave the issue
unchanged. `ready` is a discovery snapshot, not a reservation: only a successful
claim grants ownership.

Release requires the current owner unless `--force` is supplied. It clears the
assignee and returns `in_progress` to `open`; it preserves `blocked` and `closed`
statuses, including the closing timestamp. Releasing unassigned work fails.
There is no lease or expiry. Stop an abandoned worker before forced recovery;
the command does not terminate or fence that worker.

Ordinary updates do not clear ownership. `update --assignee` requires `--force`;
an `in_progress` update combined with other fields also requires `--force`.
Prefer claiming first, then updating metadata. For intentional administrative
reassignment (which can override readiness and ownership checks), use:

```bash
tracer --actor coordinator update bd-1 --assignee cursor-2 --status in_progress --force
```

Forced updates set only the supplied fields; they do not implicitly assign the
actor. The library's `Storage::update_issue` remains administrative for imports
and maintenance; workers must use `claim_issue` / `release_issue`.

**Scope:** exclusivity coordinates cooperating agents using the **same local
SQLite database**. Actor names are identifiers, not authentication. This is not
a general edit permission system: metadata updates, dependency changes, and
closing tasks remain collaborative. Imports and explicit administrative writes
can change ownership. Git/JSONL synchronization does **not** lock tasks across
disconnected clones or separate databases.

### 3. Assignee Visibility

Assignees are shown in all issue listings:

```bash
tracer list --status in_progress

bd-1 Implement auth [in_progress, Assignee: claude-1]
bd-2 Write tests [in_progress, Assignee: cursor-2]
bd-3 Fix bug [in_progress, Assignee: gpt-4]
```

## Multi-Agent Workflow

### Agent 1 (Claude) starts work:
```bash
tracer --actor claude-1 update bd-1 --status in_progress
tracer --actor claude-1 comment bd-1 "Working on JWT implementation"
```

### Agent 2 (Cursor) sees the activity:
```bash
tracer show bd-1
# Sees claude-1 is working on it with recent comment

tracer comment bd-1 "I can help with frontend once API is ready"
```

### Agent 1 completes:
```bash
tracer comment bd-1 "API complete, ready for testing"
tracer close bd-1 --reason "Implementation done"
```

### Agent 2 picks up dependent work:
```bash
tracer ready
# bd-2 is now unblocked

tracer --actor cursor-2 update bd-2 --status in_progress
tracer comment bd-2 "Starting tests for the new API"
```

## Key Points

- **Simple**: No registration, no sessions, just comments and assignee field
- **Actor identification**: Via `--actor` flag or `$TRACE_ACTOR` or `$USER` env var
- **Auto-assign**: Happens automatically when status changes to `in_progress`
- **Communication**: Through comments visible in `tracer show`
- **Coordination**: Agents see who's working on what via assignee field

## Example Session

```bash
# Three agents working together
$ tracer list --status in_progress

bd-1 Implement auth [P1, in_progress, Assignee: claude-1]
bd-2 Write tests [P1, in_progress, Assignee: cursor-2]
bd-3 Fix bug [P0, in_progress, Assignee: gpt-4]

$ tracer show bd-1

bd-1 Implement authentication API
Status: in_progress
Assignee: claude-1

Recent comments:
  cursor-2 (10 min ago): "I can help with frontend once ready"
  claude-1 (20 min ago): "Working on JWT implementation"
  claude-1 (30 min ago): "Started on this issue"
```
