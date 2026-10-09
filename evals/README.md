# P0 acceptance evals

These black-box scenarios exercise the built CLI in disposable directories. They
use only Python's standard library, require no model/API credentials, and never
touch the repository's tracker. A scenario must satisfy every assertion in every
round; one failure makes the runner exit nonzero. JSON output is available for CI.

```bash
cargo build --locked --bin tracer
python3 evals/p0.py --rounds 5 --json
# Also test a packaged binary:
python3 evals/p0.py --binary /path/to/tracer --rounds 5
```

| Scenario | Wrong implementation it detects |
| --- | --- |
| Competing workers | Startup import undoes a claim; claim alias bypasses exclusivity; retries append audit events; another actor releases ownership. |
| Clone handoff | JSONL loses labels, comments, closure timestamps or dependencies; repeated import duplicates events. |
| Independent clone IDs | Database copies reuse counters; additive import drops one clone's issues or remaps legacy IDs. |
| Invalid input | A late malformed row or identity collision partially imports; automatic sync ignores errors; dry-run writes the managed file. |
| Divergent clones | Already-exported claims overwrite each other without consent; explicit local/incoming resolution chooses the wrong record. |

The runner uses actual competing subprocesses; scheduling is intentionally not
fixed. More rounds increase the opportunity to find race regressions, not a claim
of exhaustive concurrency proof. Rust regression tests additionally cover storage
transactions, injected failures, legacy migration, and the publication/acknowledge
crash window. Neither suite simulates hardware power loss, authenticates actors,
nor promises exclusive claims across disconnected clones.
