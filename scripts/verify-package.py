#!/usr/bin/env python3
"""Validate our Linux archive contract, then exercise both extracted commands."""
import argparse
import hashlib
from pathlib import Path
import subprocess
import tarfile
import tempfile


def verify(directory, rounds):
    entries = (directory / "SHA256SUMS").read_text().splitlines()
    assert len(entries) == 1, "Expected exactly one supported download"
    digest, filename = entries[0].split()
    assert Path(filename).name == filename, "Checksum filename must be a basename"
    archive = directory / filename
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == digest, "Checksum mismatch"
    assert filename.startswith("tracer-v") and filename.endswith("-x86_64-unknown-linux-musl.tar.gz")
    name = filename.removesuffix(".tar.gz")
    version = name.removeprefix("tracer-v").removesuffix("-x86_64-unknown-linux-musl")
    files = {"tracer", "tr", "LICENSE", "README.md", "INSTALL.md", "BUILD-INFO",
             ".agents/skills/tracking-work-with-tracer/SKILL.md", "docs/agent-cli.md"}
    directories = {"", "docs", ".agents", ".agents/skills", ".agents/skills/tracking-work-with-tracer"}
    expected = {f"{name}/{path}" if path else name for path in files | directories}
    repo = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="tracer-package-") as temporary:
        with tarfile.open(archive, "r:gz") as package:
            members = package.getmembers()
            assert len(members) == len(expected), "Duplicate or unexpected archive entries"
            assert {member.name for member in members} == expected, "Incorrect archive contents"
            assert len({member.mtime for member in members}) == 1, "Non-normalized timestamps"
            for member in members:
                relative = member.name.removeprefix(name).lstrip("/")
                is_directory = relative in directories
                assert member.isdir() if is_directory else member.isfile(), "Links/devices are forbidden"
                mode = 0o755 if is_directory or relative in {"tracer", "tr"} else 0o644
                assert member.mode == mode, f"Incorrect mode for {member.name}"
                assert member.uid == member.gid == 0, "Non-normalized ownership"
            # Extraction is safe after the exact path/type allowlist above, even
            # on Python versions predating tarfile's extraction filters.
            package.extractall(temporary)
        extracted = Path(temporary) / name
        for binary in ("tracer", "tr"):
            path = extracted / binary
            headers = subprocess.check_output(["readelf", "-l", str(path)], text=True)
            dynamic = subprocess.check_output(["readelf", "-d", str(path)], text=True)
            assert "INTERP" not in headers and "(NEEDED)" not in dynamic, "Binary is not static"
            actual = subprocess.check_output([str(path), "--version"], text=True).strip()
            _, reported_version = actual.split()
            assert reported_version == version, f"Unexpected version: {actual}"
            subprocess.run(["python3", str(repo / "evals/p0.py"), "--binary", str(path),
                            "--rounds", str(rounds)], check=True)
        print(f"Verified checksum, contents, modes, static linkage and both commands: {filename}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="Directory containing archive and SHA256SUMS")
    parser.add_argument("--rounds", type=int, default=5)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    verify(args.directory.resolve(), args.rounds)
