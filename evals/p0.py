#!/usr/bin/env python3
"""Black-box P0 acceptance scenarios; only the CLI and portable JSONL are used."""
import argparse
import concurrent.futures
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time


class Workspace:
    def __init__(self, binary, path):
        self.binary = binary
        self.path = path
        path.mkdir(parents=True, exist_ok=True)
        self.db = path / "issues.db"

    def run(self, *args, actor="eval", succeeds=True):
        result = subprocess.run(
            [str(self.binary), "--db", str(self.db), "--actor", actor, "--json", *map(str, args)],
            cwd=self.path, capture_output=True, text=True, timeout=30,
        )
        if succeeds is not None:
            assert (result.returncode == 0) == succeeds, (
                f"{args}: exit={result.returncode}; stderr={result.stderr[:1000]}"
            )
        return result

    def json(self, *args, **kwargs):
        return json.loads(self.run(*args, **kwargs).stdout)

    def create(self, title, *args):
        return self.json("create", title, *args)["id"]

    def records(self):
        return [json.loads(line) for line in self.run("export").stdout.splitlines()]

    def clone(self, path):
        # Every subprocess exited; no connection/WAL writer remains open.
        shutil.copytree(self.path, path)
        return Workspace(self.binary, path)

    def write(self, name, records):
        path = self.path / name
        path.write_text("".join(json.dumps(record) + "\n" for record in records))
        return path


def competing_workers(workspace):
    task = workspace.create("Exactly one worker may start")
    barrier = threading.Barrier(8)

    def claim(index):
        barrier.wait(timeout=10)
        args = ("claim", task) if index % 2 else ("update", task, "--status", "in_progress")
        return workspace.run(*args, actor=f"worker-{index}", succeeds=None)

    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(claim, range(8)))
    winners = [result for result in results if result.returncode == 0]
    assert len(winners) == 1, f"Expected one successful claim, got {len(winners)}"
    owner = json.loads(winners[0].stdout)["assignee"]
    state = workspace.json("show", task)
    assert (state["assignee"], state["status"]) == (owner, "in_progress")
    snapshot = workspace.records()
    workspace.run("claim", task, actor=owner)
    assert workspace.json("show", task) == state, "Owner retry changed issue state"
    assert workspace.records() == snapshot, "Owner retry changed audit history"
    workspace.run("release", task, actor="other", succeeds=False)
    assert workspace.records() == snapshot, "Rejected release changed data"
    workspace.run("release", task, "--force", actor="coordinator")
    workspace.run("claim", task, actor="replacement")
    workspace.run("claim", task, actor=owner, succeeds=False)
    assert workspace.json("show", task)["assignee"] == "replacement"


def clone_handoff(workspace):
    first = workspace.create("API contract", "--labels", "backend,review")
    dependent = workspace.create("Consumer", "--deps", f"blocks:{first}")
    workspace.run("comment", first, "Keep this handoff across machines", actor="author")
    workspace.run("claim", first, actor="author")
    workspace.run("close", first, "--reason", "Contract verified", actor="author")
    baseline = workspace.records()
    record = next(record for record in baseline if record["id"] == first)
    assert record["labels"] == ["backend", "review"]
    assert record["closed_at"] and record["status"] == "closed"
    assert any(event.get("comment") == "Keep this handoff across machines" for event in record["events"])
    assert any(event.get("comment") == "Contract verified" for event in record["events"])
    source = workspace.write("transfer.jsonl", list(reversed(baseline)))
    other = Workspace(workspace.binary, workspace.path / "fresh-clone")
    other.run("import", "--input", source)
    other.run("import", "--input", source)
    assert other.records() == baseline, "Round-trip changed data or duplicated events"
    assert dependent in {issue["id"] for issue in other.json("ready")}
    other.run("claim", dependent, actor="consumer")
    assert other.json("show", dependent)["assignee"] == "consumer"


def independent_clone_ids(workspace):
    shared = workspace.create("Shared legacy issue", "--id", "legacy-7")
    before = workspace.records()
    other = workspace.clone(workspace.path.parent / (workspace.path.name + "-copy"))
    left = [workspace.create(f"Left {i}") for i in range(8)]
    right = [other.create(f"Right {i}") for i in range(8)]
    assert len(set(left + right + [shared])) == 17, "Independent clones reused an ID"
    incoming = other.write("new-issues.jsonl", other.records())
    workspace.run("import", "--input", incoming, "--skip-existing")
    records = workspace.records()
    assert {record["id"] for record in records} == set(left + right + [shared])
    assert next(record for record in records if record["id"] == shared) == before[0]


def invalid_input_is_atomic(workspace):
    workspace.create("Keep unchanged")
    before = workspace.records()
    managed = workspace.path / "issues.jsonl"
    original = managed.read_bytes()
    bad = dict(before[0], id="different-id", title="Must not be partially inserted", events=[])
    malformed = workspace.write("malformed.jsonl", [bad])
    with malformed.open("a") as output:
        output.write("<<<<<<< unresolved conflict\n")
    workspace.run("import", "--input", malformed, succeeds=False)
    assert workspace.records() == before
    assert managed.read_bytes() == original
    collision = dict(before[0], created_at="2000-01-01T00:00:00Z")
    invalid = workspace.write("collision.jsonl", [bad, collision])
    workspace.run("import", "--input", invalid, "--resolve", "incoming", succeeds=False)
    assert workspace.records() == before, "ID collision caused partial import"
    managed.write_text("not JSON\n")
    workspace.run("create", "Must not run through bad sync", succeeds=False)
    assert managed.read_text() == "not JSON\n"
    assert workspace.records() == before
    recovery = workspace.write("recovery.jsonl", before)
    workspace.run("import", "--input", recovery, "--dry-run")
    assert managed.read_text() == "not JSON\n", "Dry-run published data"
    workspace.run("import", "--input", recovery)
    assert workspace.records() == before
    assert managed.read_bytes() == original


def divergent_clones_require_resolution(workspace):
    task = workspace.create("Shared task")
    other = workspace.clone(workspace.path.parent / (workspace.path.name + "-copy"))
    workspace.run("claim", task, actor="local-worker")
    other.run("claim", task, actor="remote-worker")
    local = workspace.records()
    remote = other.records()
    incoming = workspace.write("remote.jsonl", remote)
    # Both sides already exported. A clean dirty flag is not evidence of ancestry.
    workspace.run("import", "--input", incoming, succeeds=False)
    assert workspace.records() == local
    workspace.run("import", "--input", incoming, "--resolve", "local")
    assert workspace.records() == local
    workspace.run("import", "--input", incoming, "--resolve", "incoming", "--dry-run")
    assert workspace.records() == local
    workspace.run("import", "--input", incoming, "--resolve", "incoming")
    assert workspace.records() == remote
    workspace.run("claim", task, actor="local-worker", succeeds=False)
    assert workspace.json("show", task)["assignee"] == "remote-worker"


SCENARIOS = [competing_workers, clone_handoff, independent_clone_ids,
             invalid_input_is_atomic, divergent_clones_require_resolution]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/tracer"))
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f"Build Tracer first; binary not found: {binary}")
    results = []
    with tempfile.TemporaryDirectory(prefix="tracer-evals-") as root:
        for scenario in SCENARIOS:
            start = time.monotonic()
            result = {"scenario": scenario.__name__, "passed": True, "rounds": 0}
            try:
                for index in range(args.rounds):
                    scenario(Workspace(binary, Path(root) / f"{scenario.__name__}-{index}"))
                    result["rounds"] += 1
            except Exception as error:
                result.update(passed=False, error=f"{type(error).__name__}: {error}")
            result["seconds"] = round(time.monotonic() - start, 3)
            results.append(result)
    passed = sum(result["passed"] for result in results)
    report = {"passed": passed, "total": len(results), "results": results}
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        for result in results:
            print(f"{'PASS' if result['passed'] else 'FAIL'} {result['scenario']}: "
                  f"{result['rounds']}/{args.rounds} rounds "
                  f"{result.get('error', '')}")
        print(f"{passed}/{len(results)} P0 scenarios passed")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
