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

Requires Rust toolchain: https://rustup.rs/

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

## Documentation

- [AGENTS.md](./AGENTS.md) - AI agent integration guide
- [MULTI_AGENT.md](./MULTI_AGENT.md) - Multi-agent coordination
- [CHANGELOG.md](./CHANGELOG.md) - Version history

## License

MIT
