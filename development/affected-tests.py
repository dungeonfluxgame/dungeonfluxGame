"""Run only the tests touched by the working tree's changes, and check what depends on them.

For every changed or new `.rs` file under `crates/<crate>/src/`, the module path it belongs to
(`src/look/studio.rs` -> `look::studio_tests::`) becomes a test-name filter, and the crate's library
tests run with those filters only. Crates in the workspace that depend on a changed crate, directly
or through others, get `cargo check --lib --tests` instead of their tests. A change outside `src/`
(Cargo.toml, tests/, assets) or to `lib.rs`/`main.rs` alone only compiles the crate; with
`--whole-crate` a change outside `src/` runs that crate's tests unfiltered.

Usage: py -3 development/affected-tests.py [--base REV] [--dry-run] [--whole-crate]
Uses the CARGO_TARGET_DIR already set; run it from the repository root.
"""

import argparse
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent


def git_lines(*args):
    out = subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True)
    return [line for line in out.stdout.splitlines() if line]


def changed_files(base):
    files = set(git_lines("diff", "--name-only", base))
    files.update(git_lines("ls-files", "--others", "--exclude-standard"))
    return sorted(files)


def module_filter(src_relative):
    """`look/studio.rs` or `look/studio_tests.rs` -> `look::studio_tests::`;
    `lib.rs`/`main.rs` -> None (compile only)."""
    parts = list(pathlib.PurePosixPath(src_relative).with_suffix("").parts)
    if parts[-1] in ("lib", "main"):
        return None
    if parts[-1] == "mod":
        parts.pop()
    if not parts:
        return None
    # A module's tests live in its sibling `<name>_tests.rs`, so they run as
    # `<parents>::<name>_tests::<test>`: filter on exactly that path.
    if not parts[-1].endswith("_tests"):
        parts[-1] = parts[-1] + "_tests"
    return "::".join(parts) + "::"


def workspace_graph():
    meta = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"],
            cwd=ROOT, check=True, capture_output=True, text=True,
        ).stdout
    )
    members = {pkg["name"]: pkg for pkg in meta["packages"]}
    dependents = {name: set() for name in members}
    for name, pkg in members.items():
        for dep in pkg["dependencies"]:
            if dep["name"] in members and dep.get("kind") in (None, "normal"):
                dependents[dep["name"]].add(name)
    return members, dependents


def crate_of(path, members):
    parts = pathlib.PurePosixPath(path).parts
    if len(parts) >= 2 and parts[0] == "crates" and parts[1] in members:
        return parts[1]
    return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="HEAD", help="revision to diff against (default HEAD)")
    parser.add_argument("--dry-run", action="store_true", help="print the commands only")
    parser.add_argument("--whole-crate", action="store_true",
                        help="run a crate's tests unfiltered when a change is outside src/")
    args = parser.parse_args()

    members, dependents = workspace_graph()
    filters = {}  # crate -> set of filters; None in the set means the whole crate
    for path in changed_files(args.base):
        crate = crate_of(path, members)
        if crate is None:
            continue
        parts = pathlib.PurePosixPath(path).parts
        if len(parts) > 3 and parts[2] == "src" and path.endswith(".rs"):
            wanted = filters.setdefault(crate, set())
            module = module_filter("/".join(parts[3:]))
            # lib.rs/main.rs mostly declare modules: compiling the crate covers them.
            if module is not None:
                wanted.add(module)
        elif args.whole_crate:
            filters.setdefault(crate, set()).add(None)
        else:
            filters.setdefault(crate, set())

    downstream = set()
    queue = list(filters)
    while queue:
        for user in dependents[queue.pop()]:
            if user not in filters and user not in downstream:
                downstream.add(user)
                queue.append(user)

    commands = []
    for crate in sorted(filters):
        wanted = filters[crate]
        if not wanted:
            commands.append(["cargo", "--offline", "check", "-p", crate, "--lib", "--tests"])
        elif None in wanted:
            commands.append(["cargo", "--offline", "test", "-p", crate, "--lib"])
        else:
            commands.append(["cargo", "--offline", "test", "-p", crate, "--lib", "--",
                             *sorted(wanted)])
    for crate in sorted(downstream):
        commands.append(["cargo", "--offline", "check", "-p", crate, "--lib", "--tests"])

    if not commands:
        print("affected-tests: no crate source changed")
        return 0
    for command in commands:
        print("affected-tests:", " ".join(command), flush=True)
        if not args.dry_run:
            result = subprocess.run(command, cwd=ROOT)
            if result.returncode != 0:
                print("affected-tests: FAILED", " ".join(command))
                return result.returncode
    print("affected-tests: PASS" if not args.dry_run else "affected-tests: dry run")
    return 0


if __name__ == "__main__":
    sys.exit(main())
