#!/usr/bin/env python3
"""Workspace hygiene gate: run before every handoff (AGENTS.md, "Workspace hygiene").

Checks the working tree against a base revision (default HEAD) and fails on mess an agent leaves
behind: stray files, CRLF, conflict markers, debug output, unreasoned suppressions, untracked TODOs,
over-long Rust lines, loose files at the top of artifacts/tmp and too many build directories.

Usage: py -3 development/hygiene-check.py [--base REV] [--max-build-dirs N]
Exit status 0 means RESULT PASS.
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
STRAY_SUFFIXES = {".log", ".orig", ".rej", ".bak", ".tmp", ".swp", ".patch", ".diff", ".out"}
TEXT_SUFFIXES = {".rs", ".toml", ".md", ".css", ".html", ".json", ".py", ".sh", ".sql", ".txt", ".yml"}
CONFLICT = re.compile(r"^(<<<<<<< |>>>>>>> |=======$)")
DEBUG_OUTPUT = re.compile(r"\b(dbg!|todo!|unimplemented!)\(")
PRINT_OUTPUT = re.compile(r"\b(println!|eprintln!|print!|eprint!)\(")
BARE_ALLOW = re.compile(r"#\[allow\((?![^)]*reason\s*=)")
BARE_TODO = re.compile(r"\b(TODO|FIXME|XXX)\b(?!\([A-Z]+-\d+\))")
MAXIMUM_COLUMNS = 100


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True,
                          encoding="utf-8", errors="replace", check=True).stdout


def changed_paths(base):
    tracked = git("diff", "--name-only", base).split()
    untracked = git("ls-files", "--others", "--exclude-standard").split()
    return sorted(set(tracked)), sorted(set(untracked))


def added_lines(base, path):
    """The lines a file gained since `base` (all lines for a new file), with their line numbers."""
    full = ROOT / path
    if not full.is_file():
        return []
    if path in git("ls-files", "--others", "--exclude-standard").split():
        text = full.read_text(encoding="utf-8", errors="replace")
        return list(enumerate(text.splitlines(), start=1))
    lines, number = [], 0
    for line in git("diff", "-U0", base, "--", path).splitlines():
        hunk = re.match(r"^@@ -\d+(?:,\d+)? \+(\d+)", line)
        if hunk:
            number = int(hunk.group(1))
        elif line.startswith("+") and not line.startswith("+++"):
            lines.append((number, line[1:]))
            number += 1
    return lines


def is_test_or_tool(path):
    parts = Path(path).parts
    name = Path(path).name
    return ("tests" in parts or "benches" in parts or "examples" in parts or name.endswith("_tests.rs")
            or "/bin/" in path or path.endswith("main.rs") or not path.startswith("crates/"))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", default="HEAD")
    parser.add_argument("--max-build-dirs", type=int, default=4)
    args = parser.parse_args()
    problems = []
    tracked, untracked = changed_paths(args.base)
    for path in untracked:
        suffix = Path(path).suffix.lower()
        if suffix in STRAY_SUFFIXES or Path(path).name.lower().startswith(("scratch", "tmp", "temp")):
            problems.append(f"stray file (belongs in artifacts/tmp/<task>/ or nowhere): {path}")
    for path in tracked + untracked:
        if path == "development/hygiene-check.py":
            continue
        full = ROOT / path
        if not full.is_file() or full.suffix.lower() not in TEXT_SUFFIXES:
            continue
        if b"\r\n" in full.read_bytes():
            problems.append(f"CRLF line endings (the repository is LF): {path}")
        for number, line in added_lines(args.base, path):
            where = f"{path}:{number}"
            if CONFLICT.match(line):
                problems.append(f"conflict marker: {where}")
            if path.endswith(".rs"):
                if DEBUG_OUTPUT.search(line):
                    problems.append(f"debug macro left in: {where}")
                if PRINT_OUTPUT.search(line) and not is_test_or_tool(path):
                    problems.append(f"print in library code (use the audit path): {where}")
                if BARE_ALLOW.search(line):
                    problems.append(f"#[allow] without reason = \"...\": {where}")
                if len(line) > MAXIMUM_COLUMNS:
                    problems.append(f"line over {MAXIMUM_COLUMNS} columns ({len(line)}): {where}")
            if path.endswith((".rs", ".css", ".py", ".sh")) and BARE_TODO.search(line):
                problems.append(f"TODO without a tracked ID such as TODO(DEV-12): {where}")
    loose = [p.name for p in (ROOT / "artifacts" / "tmp").glob("*") if p.is_file()]
    for name in loose:
        problems.append(f"loose file at the top of artifacts/tmp (use artifacts/tmp/<task>/): {name}")
    build = ROOT / "artifacts" / "build"
    build_dirs = sorted(p.name for p in build.glob("*") if p.is_dir()) if build.is_dir() else []
    if len(build_dirs) > args.max_build_dirs:
        problems.append(f"{len(build_dirs)} build directories in artifacts/build (at most "
                        f"{args.max_build_dirs}; reuse your lane's): {', '.join(build_dirs)}")
    print(f"hygiene check against {args.base}: {len(tracked)} changed, {len(untracked)} new file(s)")
    for problem in problems:
        print(f"  {problem}")
    print("RESULT PASS" if not problems else f"RESULT FAIL ({len(problems)} problem(s))")
    return 0 if not problems else 1


if __name__ == "__main__":
    sys.exit(main())
