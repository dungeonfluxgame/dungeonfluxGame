#!/usr/bin/env python3
"""Imagery boundary checker: no direct image execution outside private df-imagery backends.

Scans Rust sources for direct image-executor construction/dispatch. Comments and
string/char literals are stripped before matching, so mentions in docs or test
data are not false positives. Matching runs on the whole stripped token stream
(after stripping), so qualified constructors split across lines or around
comments are still found; each match maps back to its source line. Cargo
dependency and lib.rs export audits run on the same invocation.

Test scope is syntax-scoped, never prefix-scoped: a whole test path or an
offset inside a `cfg(test)` braced item counts; production after a closed
`cfg(test)` module does not. Fixture reasoning is call-associated and
binding-associated: a forbidden constructor in test scope is exempt only when
its own argument list carries a `loopback_fixture(...)` call, or the exact
variable passed to it was bound by `let name = ...loopback_fixture(...)`
within the 12 preceding lines without a later non-fixture reassignment. A bare
`loopback_fixture` identifier or an unrelated nearby fixture call exempts
nothing. `ImageRoute::X(_)` with an exact wildcard, match-arm patterns before
`=>`, `matches!` patterns, and `let` destructuring are inspection, not
execution; `ImageRoute::X(named)` construction and `ImageRoute::X(_named)`
payloads are flagged. `submit_image` needs its dispatch dot
(`.submit_image(`), so the provider definition itself is not a finding.
`crates/df-imagery/src/backends/` is the exact intended private execution
boundary and is not scanned for constructor findings; the rest of `df-imagery`
(including `service.rs`) still is.

Legacy transitional sites are budgeted per exact (file, symbol) with an
occurrence count and an owning-function callsite fingerprint. Only an
occurrence whose enclosing `fn` body carries the fingerprint counts as legacy;
any other occurrence fails as bypass even when the token appears elsewhere in
the file, and a present file with zero qualifying occurrences fails as stale.
Default mode reports legacy rows nonfatally; --strict fails on them too.

Exit codes: 0 pass, 1 boundary violation, 2 usage/config error.
Only stdlib; no network, no provider calls.
"""
from __future__ import annotations

import argparse
import bisect
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - interpreter predates 3.11
    tomllib = None  # type: ignore[assignment]

CONSTRUCTOR_BASES: tuple[str, ...] = (
    "NativeImageEditProvider",
    "NativeFalImageProvider",
    "NativeBflImageProvider",
    "NativeFalMediaProvider",
)
CONSTRUCTOR_RES: tuple[tuple[str, re.Pattern[str]], ...] = tuple(
    (f"{base}::new", re.compile(r"\b" + base + r"\s*::\s*new\b")) for base in CONSTRUCTOR_BASES
)
IMAGEROUTE_RE: re.Pattern[str] = re.compile(r"\bImageRoute\s*::\s*(OpenAi|Flux|Bfl)\b")
SUBMIT_IMAGE_RE: re.Pattern[str] = re.compile(r"\.\s*submit_image\s*\(")
LOOPBACK_CALL_RE: re.Pattern[str] = re.compile(r"\bloopback_fixture\s*\(")
CFG_TEST_RE: re.Pattern[str] = re.compile(r"cfg\s*\(\s*test")
PUB_MOD_BACKENDS_RE: re.Pattern[str] = re.compile(r"\bpub\s+mod\s+backends\b")
PUB_USE_BACKENDS_RE: re.Pattern[str] = re.compile(r"\bpub\s+use\s+[^;]*\bbackends\b")
USE_STMT_RE: re.Pattern[str] = re.compile(r"\buse\s+([^;]*?);")
PUB_USE_STMT_RE: re.Pattern[str] = re.compile(r"\bpub\s+use\s+([^;]*?);")
FN_RE: re.Pattern[str] = re.compile(r"\bfn\s+(\w+)")
LET_RE: re.Pattern[str] = re.compile(r"\blet\b")
MATCHES_MACRO_RE: re.Pattern[str] = re.compile(r"\bmatches\s*!")


@dataclass(frozen=True)
class LegacyRow:
    path: str
    symbol: str
    expected: int
    fingerprint: str
    removal: str


# Narrow legacy allowances: exact repo-relative file + exact symbol label, with
# an occurrence budget and a callsite fingerprint. No directory blankets.
# Default mode reports these as legacy; --strict fails. Growth fails as
# bypass; a present file with zero occurrences (or a missing fingerprint)
# fails as stale.
LEGACY_ROWS: tuple[LegacyRow, ...] = (
    LegacyRow(
        "crates/df-tools/src/gameplay/imagery.rs",
        "NativeFalImageProvider::new",
        1,
        "FalImageConfig::live",
        "phase 3 consumer migration, then phase 5 deletion",
    ),
    LegacyRow(
        "crates/df-tools/src/gameplay/imagery.rs",
        "NativeImageEditProvider::new",
        1,
        "ImageEditConfig::live",
        "phase 3 consumer migration, then phase 5 deletion",
    ),
    LegacyRow(
        "crates/df-providers/src/registry.rs",
        "NativeFalMediaProvider::new",
        1,
        "registered_fal_media_provider",
        "phase 3 migration of the generic Schnell path, then phase 5 deletion",
    ),
    LegacyRow(
        "crates/df-tools/src/gameplay/creator_portrait_startup.rs",
        "NativeImageEditProvider::new",
        1,
        "ImageEditConfig::live",
        "phase 3 creator chain migration, then phase 5 deletion",
    ),
    LegacyRow(
        "crates/df-tools/src/gameplay/creator_portrait_startup.rs",
        "NativeFalImageProvider::new",
        1,
        "FalImageConfig::live",
        "phase 3 creator chain migration, then phase 5 deletion",
    ),
    LegacyRow(
        "crates/df-tools/src/gameplay/creator_portrait_startup.rs",
        "NativeBflImageProvider::new",
        1,
        "BflImageConfig::live",
        "phase 3 creator chain migration, then phase 5 deletion",
    ),
    LegacyRow(
        "crates/df-tools/src/gameplay/creator_portrait_startup.rs",
        "ImageRoute::OpenAi",
        1,
        "ImageProvider::OpenAi",
        "phase 3 creator chain migration, then phase 5 deletion",
    ),
    LegacyRow(
        "crates/df-tools/src/gameplay/creator_portrait_startup.rs",
        "ImageRoute::Flux",
        1,
        "ImageProvider::Flux",
        "phase 3 creator chain migration, then phase 5 deletion",
    ),
    LegacyRow(
        "crates/df-tools/src/gameplay/creator_portrait_startup.rs",
        "ImageRoute::Bfl",
        1,
        "alt_settings",
        "phase 3 creator chain migration, then phase 5 deletion",
    ),
    LegacyRow(
        "crates/df-tools/src/gameplay/character_clips.rs",
        "NativeImageEditProvider::new",
        1,
        "IMAGE_EDIT_TIMEOUT",
        "phase 3 frame migration with the prior-attempt fence, then phase 5 deletion",
    ),
    LegacyRow(
        "crates/df-tools/src/gameplay/dev_control/direct.rs",
        "NativeImageEditProvider::new",
        1,
        "loopback_fixture",
        "phase 3 dev-fixture migration to the central fake-backend seam",
    ),
    LegacyRow(
        "crates/df-tools/src/gameplay/imagery/figure_masks.rs",
        "NativeImageEditProvider::new",
        1,
        "for_test",
        "phase 3 test-helper migration to the central fake-backend seam",
    ),
    LegacyRow(
        "crates/df-tools/src/gameplay/imagery_live_tests.rs",
        "NativeFalImageProvider::new",
        1,
        "FluxPerson",
        "opt-in live acceptance (#[ignore]); migrate to service-level live acceptance",
    ),
)
LEGACY_BUDGET: dict[tuple[str, str], LegacyRow] = {(r.path, r.symbol): r for r in LEGACY_ROWS}

# Test-loopback exemption is call-associated, not path-blanket and never
# line-blanket. A constructor is exempt only in test scope (a test path or a
# region after a cfg(test) marker) with fixture proof. submit_image calls and
# ImageRoute constructions in test scope are test-only wiring and exempt.
TEST_PATH_RE = re.compile(r"(?:^|/)(?:tests?/.*|.*_tests?\.rs)$")
PROVIDER_SELF_TEST_SRC = "crates/df-providers/src/"
PROVIDER_SELF_TEST_DIR = "crates/df-providers/tests/"

# Exact intended private execution boundary. Only this prefix is skipped by
# the constructor scan; the rest of df-imagery is still checked.
PRIVATE_BACKEND_PREFIX = "crates/df-imagery/src/backends/"

FORBIDDEN_DEPENDENTS = ("df-tools", "df-session")
DEPENDENCY_TABLE_NAMES = ("dependencies", "dev-dependencies", "build-dependencies")
IMAGERY_CARGO = "crates/df-imagery/Cargo.toml"
IMAGERY_LIB = "crates/df-imagery/src/lib.rs"

MAX_FILE_BYTES = 8 * 1024 * 1024
FIXTURE_WINDOW_LINES = 12


@dataclass
class Finding:
    path: str
    line: int
    symbol: str
    kind: str  # "bypass" | "legacy" | "stale" | "dependency" | "export" | "alias"


def strip_rust(source: str) -> str:
    """Return source with comments/strings/chars blanked, newlines preserved."""
    out: list[str] = []
    i = 0
    n = len(source)
    state = "code"
    block_depth = 0
    raw_hashes = 0
    while i < n:
        ch = source[i]
        nxt = source[i + 1] if i + 1 < n else ""
        if state == "code":
            if ch == "/" and nxt == "/":
                state = "line"
                out.append("  ")
                i += 2
            elif ch == "/" and nxt == "*":
                state = "block"
                block_depth = 1
                out.append("  ")
                i += 2
            elif ch == '"':
                state = "str"
                out.append(" ")
                i += 1
            elif ch == "'" and _looks_like_char(source, i):
                state = "char"
                out.append(" ")
                i += 1
            elif ch == "r" and (nxt == '"' or nxt == "#"):
                j = i + 1
                hashes = 0
                while j < n and source[j] == "#":
                    hashes += 1
                    j += 1
                if j < n and source[j] == '"':
                    state = "raw"
                    raw_hashes = hashes
                    out.append(" " * (j - i + 1))
                    i = j + 1
                else:
                    out.append(ch)
                    i += 1
            else:
                out.append(ch)
                i += 1
        elif state == "line":
            if ch == "\n":
                state = "code"
                out.append("\n")
            else:
                out.append(" ")
            i += 1
        elif state == "block":
            if ch == "/" and nxt == "*":
                block_depth += 1
                out.append("  ")
                i += 2
            elif ch == "*" and nxt == "/":
                block_depth -= 1
                out.append("  ")
                i += 2
                if block_depth <= 0:
                    state = "code"
            else:
                out.append("\n" if ch == "\n" else " ")
                i += 1
        elif state == "str":
            if ch == "\\" and i + 1 < n:
                # An escaped newline (line continuation) must keep its newline
                # so later matches still map to the right source line.
                if source[i + 1] == "\n":
                    out.append(" ")
                    i += 1
                elif source[i + 1] == "\r" and i + 2 < n and source[i + 2] == "\n":
                    out.append(" ")
                    i += 1
                else:
                    out.append("  ")
                    i += 2
            elif ch == '"':
                state = "code"
                out.append(" ")
                i += 1
            else:
                out.append("\n" if ch == "\n" else " ")
                i += 1
        elif state == "char":
            if ch == "\\" and i + 1 < n:
                if source[i + 1] == "\n":
                    out.append(" ")
                    i += 1
                elif source[i + 1] == "\r" and i + 2 < n and source[i + 2] == "\n":
                    out.append(" ")
                    i += 1
                else:
                    out.append("  ")
                    i += 2
            elif ch == "'":
                state = "code"
                out.append(" ")
                i += 1
            else:
                out.append("\n" if ch == "\n" else " ")
                i += 1
        elif state == "raw":
            if ch == '"':
                j = i + 1
                ok = True
                for _ in range(raw_hashes):
                    if j < n and source[j] == "#":
                        j += 1
                    else:
                        ok = False
                        break
                if ok:
                    out.append(" " * (j - i))
                    i = j
                    state = "code"
                else:
                    out.append(" ")
                    i += 1
            else:
                out.append("\n" if ch == "\n" else " ")
                i += 1
    return "".join(out)


def _looks_like_char(source: str, i: int) -> bool:
    # A lifetime ('a, 'static) is not a char literal; only treat a quote that
    # closes within a short span as a char literal start.
    j = i + 1
    if j < len(source) and source[j] == "\\":
        j += 2
    else:
        j += 1
    if j < len(source) and source[j] == "'":
        return True
    return False


def _line_starts(text: str) -> list[int]:
    starts = [0]
    for match in re.finditer("\n", text):
        starts.append(match.end())
    return starts


def _offset_to_line(starts: list[int], offset: int) -> int:
    return bisect.bisect_right(starts, offset)


def _matching_paren(text: str, open_idx: int) -> int:
    depth = 0
    for i in range(open_idx, len(text)):
        if text[i] == "(":
            depth += 1
        elif text[i] == ")":
            depth -= 1
            if depth == 0:
                return i
    return -1


def _matching_brace(text: str, open_idx: int) -> int:
    depth = 0
    for i in range(open_idx, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return i
    return -1


def _find_cfg_test_ranges(stripped: str) -> list[tuple[int, int]]:
    ranges: list[tuple[int, int]] = []
    for match in CFG_TEST_RE.finditer(stripped):
        attr_close = stripped.find("]", match.end())
        if attr_close == -1:
            continue
        open_idx = stripped.find("{", attr_close)
        semi_idx = stripped.find(";", attr_close)
        if open_idx == -1:
            continue
        if semi_idx != -1 and semi_idx < open_idx:
            continue
        close_idx = _matching_brace(stripped, open_idx)
        if close_idx == -1:
            continue
        ranges.append((open_idx, close_idx))
    return ranges


def _offset_in_ranges(ranges: list[tuple[int, int]], offset: int) -> bool:
    for start, end in ranges:
        if start < offset < end:
            return True
    return False


def _is_test_scope(rel: str, stripped: str, offset: int, cfg_ranges: list[tuple[int, int]]) -> bool:
    if TEST_PATH_RE.search(rel):
        return True
    return _offset_in_ranges(cfg_ranges, offset)


def _is_provider_self_test(
    rel: str, stripped: str, offset: int, cfg_ranges: list[tuple[int, int]]
) -> bool:
    # df-providers tests exercise the crate's own constructors (loopback
    # fixtures or constructor-refusal checks without dispatch). Production
    # code in that crate, including the registry factory, is still checked.
    if rel.startswith(PROVIDER_SELF_TEST_DIR):
        return True
    if rel.startswith(PROVIDER_SELF_TEST_SRC):
        return _offset_in_ranges(cfg_ranges, offset)
    return False


def _collect_constructor_aliases(stripped: str) -> dict[str, str]:
    aliases: dict[str, str] = {}
    for use_match in USE_STMT_RE.finditer(stripped):
        body = use_match.group(1)
        if "backends" in body and "Native" not in body and "ImageRoute" not in body:
            continue
        for base in CONSTRUCTOR_BASES:
            for alias_match in re.finditer(
                r"\b" + re.escape(base) + r"\b\s+as\s+(\w+)", body
            ):
                aliases[alias_match.group(1)] = base
    return aliases


def _constructor_arg_list(stripped: str, match_end: int) -> str | None:
    j = match_end
    while j < len(stripped) and stripped[j] in " \t\r\n":
        j += 1
    if j >= len(stripped) or stripped[j] != "(":
        return None
    close = _matching_paren(stripped, j)
    if close == -1:
        return None
    return stripped[j : close + 1]


def _constructor_has_fixture_proof(
    stripped: str, stripped_lines: list[str], line: int, match_end: int
) -> bool:
    args = _constructor_arg_list(stripped, match_end)
    if args is not None and LOOPBACK_CALL_RE.search(args):
        return True
    # Variable-bound proof: the exact binding passed to this constructor was
    # itself built by a loopback_fixture(...) call within the window. An
    # unrelated nearby fixture call bound to another variable proves nothing,
    # and a reassigned binding proves nothing.
    if args is None:
        return False
    bound: set[str] = set()
    low = max(0, line - 1 - FIXTURE_WINDOW_LINES)
    window = "\n".join(stripped_lines[low : line - 1])
    for binding in re.finditer(
        r"\blet\s+(?:mut\s+)?(\w+)\s*=[^;]*?loopback_fixture\s*\(", window
    ):
        bound.add(binding.group(1))
    if not bound:
        return False
    for name in bound:
        if re.search(r"\b" + re.escape(name) + r"\b", args) is None:
            continue
        lines = window.splitlines()
        binding_line = -1
        for idx, text_line in enumerate(lines):
            if re.search(r"\blet\s+(?:mut\s+)?" + re.escape(name) + r"\b", text_line):
                binding_line = idx
                break
        if binding_line == -1:
            continue
        tainted = False
        for later in lines[binding_line + 1 :]:
            if re.search(r"\b" + re.escape(name) + r"\s*=(?![=>])", later) and (
                "loopback_fixture" not in later
            ):
                tainted = True
                break
        if not tainted:
            return True
    return False


def _statement_start(stripped: str, offset: int) -> int:
    last = max(
        stripped.rfind(";", 0, offset),
        stripped.rfind("{", 0, offset),
        stripped.rfind("}", 0, offset),
    )
    return last + 1 if last != -1 else 0


def _inside_matches_macro(stripped: str, offset: int) -> bool:
    for macro in MATCHES_MACRO_RE.finditer(stripped):
        j = macro.end()
        while j < len(stripped) and stripped[j] in " \t\r\n":
            j += 1
        if j >= len(stripped) or stripped[j] != "(":
            continue
        close = _matching_paren(stripped, j)
        if close != -1 and j < offset < close:
            return True
    return False


def _imageroute_is_pattern(stripped: str, match_start: int, match_end: int, variant: str) -> bool:
    j = match_end
    while j < len(stripped) and stripped[j] in " \t\r\n":
        j += 1
    if j >= len(stripped) or stripped[j] != "(":
        return False
    close = _matching_paren(stripped, j)
    if close == -1:
        return False
    inner = stripped[j + 1 : close].strip()
    if inner == "_":
        return True
    if _inside_matches_macro(stripped, match_start):
        return True
    # Match-arm construction sits before its fat arrow: `Pattern =>`.
    after = stripped[close + 1 : close + 201]
    semi = after.find(";")
    arrow = after.find("=>")
    if arrow != -1 and (semi == -1 or arrow < semi):
        brace = after.find("{")
        if brace == -1 or arrow < brace:
            return True
    # Destructuring assignment keeps the pattern before its single `=`.
    start = _statement_start(stripped, match_start)
    prefix = stripped[start:match_start]
    suffix = stripped[close + 1 : close + 201]
    suffix_semi = suffix.find(";")
    suffix_window = suffix if suffix_semi == -1 else suffix[:suffix_semi]
    if LET_RE.search(prefix) and re.search(r"(?<![=>])=(?![=>])", suffix_window):
        return True
    return False


def _enclosing_fn_range(stripped: str, offset: int) -> tuple[int, int] | None:
    best: tuple[int, int] | None = None
    for fn_match in FN_RE.finditer(stripped[:offset]):
        open_idx = stripped.find("{", fn_match.end())
        if open_idx == -1 or open_idx > offset:
            continue
        semi_idx = stripped.find(";", fn_match.end(), offset)
        brace_idx = stripped.find("{", fn_match.end(), offset)
        # A `fn foo();` declaration without a body owns no range.
        if semi_idx != -1 and (brace_idx == -1 or semi_idx < brace_idx):
            continue
        close_idx = _matching_brace(stripped, open_idx)
        if close_idx != -1 and open_idx < offset < close_idx:
            if best is None or open_idx > best[0]:
                best = (fn_match.start(), close_idx)
    return best


def _legacy_qualifies(
    stripped: str, line: int, stripped_lines: list[str], offset: int, fingerprint: str
) -> bool:
    fn_range = _enclosing_fn_range(stripped, offset)
    if fn_range is not None:
        return fingerprint in stripped[fn_range[0] : fn_range[1]]
    args_end = offset + 2000
    window = stripped[max(0, offset - 2000) : args_end]
    if fingerprint in window:
        low = max(0, line - 1 - FIXTURE_WINDOW_LINES)
        nearby = "\n".join(stripped_lines[low : line + FIXTURE_WINDOW_LINES])
        return fingerprint in nearby
    return False


def scan_file(path: Path, rel: str) -> tuple[list[Finding], str]:
    """Collect raw bypass candidates; legacy budgeting happens in run()."""
    try:
        if path.stat().st_size > MAX_FILE_BYTES:
            raise ValueError("file exceeds 8 MiB scan bound")
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError, ValueError) as error:
        raise RuntimeError(f"cannot read {rel}: {error}") from error
    stripped = strip_rust(text)
    if rel.startswith(PRIVATE_BACKEND_PREFIX):
        return [], stripped
    stripped_lines = stripped.splitlines()
    starts = _line_starts(stripped)
    findings: list[Finding] = []

    cfg_ranges = _find_cfg_test_ranges(stripped)
    aliases = _collect_constructor_aliases(stripped)
    alias_res = [
        (f"{base}::new via alias {alias}", re.compile(r"\b" + re.escape(alias) + r"\s*::\s*new\b"))
        for alias, base in aliases.items()
    ]

    for label, rx in CONSTRUCTOR_RES:
        for match in rx.finditer(stripped):
            line = _offset_to_line(starts, match.start())
            if _is_provider_self_test(rel, stripped, match.start(), cfg_ranges):
                continue
            if _is_test_scope(rel, stripped, match.start(), cfg_ranges) and (
                _constructor_has_fixture_proof(stripped, stripped_lines, line, match.end())
            ):
                continue
            findings.append(Finding(rel, line, label, "bypass"))

    for match in IMAGEROUTE_RE.finditer(stripped):
        variant = match.group(1)
        if _imageroute_is_pattern(stripped, match.start(), match.end(), variant):
            continue
        line = _offset_to_line(starts, match.start())
        if _is_test_scope(rel, stripped, match.start(), cfg_ranges) or _is_provider_self_test(
            rel, stripped, match.start(), cfg_ranges
        ):
            continue
        findings.append(Finding(rel, line, f"ImageRoute::{variant}", "bypass"))

    for match in SUBMIT_IMAGE_RE.finditer(stripped):
        line = _offset_to_line(starts, match.start())
        if _is_test_scope(rel, stripped, match.start(), cfg_ranges) or _is_provider_self_test(
            rel, stripped, match.start(), cfg_ranges
        ):
            continue
        findings.append(Finding(rel, line, "submit_image dispatch", "bypass"))

    for label, rx in alias_res:
        for match in rx.finditer(stripped):
            line = _offset_to_line(starts, match.start())
            findings.append(Finding(rel, line, label, "alias"))

    return findings, stripped


def _find_dep_line(text: str, name: str) -> int:
    for lineno, line in enumerate(text.splitlines(), start=1):
        code = line.split("#", 1)[0]
        if re.search(r"\b" + re.escape(name) + r"\b", code):
            return lineno
    return 0


def audit_dependencies(root: Path) -> list[Finding]:
    cargo = root / IMAGERY_CARGO
    if not cargo.is_file():
        return [Finding(IMAGERY_CARGO, 0, "missing Cargo.toml", "dependency")]
    if tomllib is None:
        raise RuntimeError("tomllib is unavailable; need Python 3.11+ to parse Cargo.toml")
    try:
        text = cargo.read_text(encoding="utf-8")
    except OSError as error:
        raise RuntimeError(f"cannot read {IMAGERY_CARGO}: {error}") from error
    try:
        data = tomllib.loads(text)
    except Exception as error:
        raise RuntimeError(f"cannot parse {IMAGERY_CARGO}: {error}") from error
    findings: list[Finding] = []
    try:
        workspace_text = (root / "Cargo.toml").read_text(encoding="utf-8")
        workspace_data = tomllib.loads(workspace_text)
        workspace_deps: dict[str, object] = {}
        workspace_node = workspace_data.get("workspace", {})
        if isinstance(workspace_node, dict):
            deps_node = workspace_node.get("dependencies", {})
            if isinstance(deps_node, dict):
                workspace_deps = deps_node
    except (OSError, ValueError, AttributeError):
        workspace_deps = {}

    def flag(name: str) -> None:
        findings.append(Finding(IMAGERY_CARGO, _find_dep_line(text, name), name, "dependency"))

    def flag_workspace(dep_name: str, pkg: str) -> None:
        findings.append(
            Finding(
                IMAGERY_CARGO,
                _find_dep_line(text, dep_name),
                f"{pkg} (renamed as {dep_name})",
                "dependency",
            )
        )

    def resolve_workspace(dep_name: str, spec: object) -> str | None:
        if not isinstance(spec, dict) or not spec.get("workspace"):
            return None
        target = workspace_deps.get(dep_name)
        if isinstance(target, dict):
            pkg = target.get("package")
            if isinstance(pkg, str):
                return pkg
            return dep_name
        if isinstance(target, str):
            return dep_name
        return dep_name

    def walk(node: object, trail: list[str]) -> None:
        if isinstance(node, dict):
            head = trail[-1] if trail else ""
            if head in DEPENDENCY_TABLE_NAMES or head.endswith("dependencies"):
                for dep_name, spec in node.items():
                    if dep_name in FORBIDDEN_DEPENDENTS:
                        flag(dep_name)
                    elif isinstance(spec, dict) and spec.get("package") in FORBIDDEN_DEPENDENTS:
                        pkg = spec.get("package")
                        findings.append(
                            Finding(
                                IMAGERY_CARGO,
                                _find_dep_line(text, dep_name),
                                f"{pkg} (renamed as {dep_name})",
                                "dependency",
                            )
                        )
                    elif isinstance(spec, dict) and spec.get("workspace"):
                        resolved = resolve_workspace(dep_name, spec)
                        if resolved in FORBIDDEN_DEPENDENTS:
                            flag_workspace(dep_name, resolved)
                        elif dep_name in FORBIDDEN_DEPENDENTS:
                            flag(dep_name)
            for dep_name, spec in node.items():
                walk(spec, trail + [dep_name])
        elif isinstance(node, list):
            for item in node:
                walk(item, trail)

    walk(data, [])
    return findings


def audit_exports(root: Path) -> list[Finding]:
    lib = root / IMAGERY_LIB
    if not lib.is_file():
        return [Finding(IMAGERY_LIB, 0, "missing lib.rs", "export")]
    try:
        text = lib.read_text(encoding="utf-8")
    except OSError as error:
        raise RuntimeError(f"cannot read {IMAGERY_LIB}: {error}") from error
    stripped = strip_rust(text)
    starts = _line_starts(stripped)
    findings: list[Finding] = []
    for match in PUB_MOD_BACKENDS_RE.finditer(stripped):
        findings.append(
            Finding(IMAGERY_LIB, _offset_to_line(starts, match.start()),
                    "public backends module", "export")
        )
    for match in PUB_USE_BACKENDS_RE.finditer(stripped):
        findings.append(
            Finding(IMAGERY_LIB, _offset_to_line(starts, match.start()),
                    "public backends re-export", "export")
        )
    mod_aliases: set[str] = set()
    backend_items: set[str] = set()
    for use_match in USE_STMT_RE.finditer(stripped):
        body = use_match.group(1)
        if "backends" not in body:
            continue
        for alias in re.finditer(r"\bbackends\s+as\s+(\w+)", body):
            mod_aliases.add(alias.group(1))
        grouped = re.search(r"\bbackends\s*::\s*\{([^}]*)\}", body)
        if grouped:
            for part in grouped.group(1).split(","):
                part = part.strip()
                alias_pair = re.match(r"(\w+)\s+as\s+(\w+)", part)
                if alias_pair:
                    backend_items.add(alias_pair.group(2))
                elif re.match(r"\w+$", part):
                    backend_items.add(part)
            continue
        for item in re.finditer(r"\bbackends\s*::\s*(\w+)\s+as\s+(\w+)", body):
            backend_items.add(item.group(2))
        for item in re.finditer(r"\bbackends\s*::\s*(\w+)", body):
            if re.search(r"\b" + re.escape(item.group(1)) + r"\s+as\b", body) is None:
                backend_items.add(item.group(1))
    for pub_match in PUB_USE_STMT_RE.finditer(stripped):
        body = pub_match.group(1)
        if "backends" in body:
            continue
        hit = False
        for alias in mod_aliases:
            if re.search(r"\b" + re.escape(alias) + r"\b", body):
                hit = True
                break
        if not hit:
            for item in backend_items:
                if re.search(r"\b" + re.escape(item) + r"\b", body):
                    hit = True
                    break
        if hit:
            findings.append(
                Finding(IMAGERY_LIB, _offset_to_line(starts, pub_match.start()),
                        "public backends re-export", "export")
            )
    return findings


def collect_sources(root: Path) -> list[Path]:
    return sorted((root / "crates").rglob("*.rs"))


def run(root: Path, strict: bool) -> list[Finding]:
    if not (root / "crates").is_dir():
        raise RuntimeError("root has no crates/ directory")
    raw: list[Finding] = []
    stripped_by_rel: dict[str, str] = {}
    present: set[str] = set()
    for path in collect_sources(root):
        rel = path.relative_to(root).as_posix()
        present.add(rel)
        file_findings, stripped = scan_file(path, rel)
        raw.extend(file_findings)
        stripped_by_rel[rel] = stripped
    findings: list[Finding] = []
    qualified_counts: dict[tuple[str, str], int] = {}
    for finding in raw:
        key = (finding.path, finding.symbol)
        row = LEGACY_BUDGET.get(key)
        if row is None:
            continue
        stripped = stripped_by_rel.get(finding.path, "")
        lines = stripped.splitlines()
        starts = _line_starts(stripped)
        offset = starts[finding.line - 1] if 1 <= finding.line <= len(starts) else 0
        if _legacy_qualifies(stripped, finding.line, lines, offset, row.fingerprint):
            qualified_counts[key] = qualified_counts.get(key, 0) + 1
    counts: dict[tuple[str, str], int] = {}
    for finding in raw:
        counts[(finding.path, finding.symbol)] = counts.get((finding.path, finding.symbol), 0) + 1
    seen: dict[tuple[str, str], int] = {}
    for finding in raw:
        key = (finding.path, finding.symbol)
        row = LEGACY_BUDGET.get(key)
        if row is None:
            findings.append(finding)
            continue
        stripped = stripped_by_rel.get(finding.path, "")
        lines = stripped.splitlines()
        starts = _line_starts(stripped)
        offset = starts[finding.line - 1] if 1 <= finding.line <= len(starts) else 0
        qualifies = _legacy_qualifies(stripped, finding.line, lines, offset, row.fingerprint)
        used = seen.get(key, 0)
        if qualifies:
            seen[key] = used + 1
            if used < row.expected:
                findings.append(Finding(finding.path, finding.line, finding.symbol, "legacy"))
            else:
                findings.append(Finding(finding.path, finding.line, finding.symbol, "bypass"))
        else:
            findings.append(Finding(finding.path, finding.line, finding.symbol, "bypass"))
    for key, row in LEGACY_BUDGET.items():
        if key[0] not in present:
            continue
        if qualified_counts.get(key, 0) == 0:
            if counts.get(key, 0) == 0:
                findings.append(
                    Finding(row.path, 0,
                            f"stale allowance: {row.symbol} (expected {row.expected})",
                            "stale")
                )
            else:
                findings.append(
                    Finding(row.path, 0,
                            f"stale allowance: {row.symbol} callsite changed "
                            f"(missing {row.fingerprint!r} in owning function)",
                            "stale")
                )
    findings.extend(audit_dependencies(root))
    findings.extend(audit_exports(root))
    _ = strict
    return findings


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Enforce the df-imagery execution boundary.")
    parser.add_argument("--root", default=".", help="workspace root (default: .)")
    parser.add_argument("--strict", action="store_true",
                        help="fail on legacy allowlist rows too")
    parser.add_argument("--format", choices=("text", "json"), default="text")
    args = parser.parse_args(argv)
    root = Path(args.root)
    try:
        findings = run(root, args.strict)
    except RuntimeError as error:
        print(f"check-imagery-boundary: error: {error}", file=sys.stderr)
        return 2
    legacy = [f for f in findings if f.kind == "legacy"]
    hard = [f for f in findings if f.kind != "legacy"]
    failed = bool(hard) or (args.strict and bool(legacy))
    if args.format == "json":
        print(json.dumps({
            "strict": args.strict,
            "violations": [vars(f) for f in findings],
            "result": "FAIL" if failed else "PASS",
        }))
    else:
        if not findings:
            print("check-imagery-boundary: PASS (no direct image executors outside df-imagery)")
        for f in findings:
            print(f"{f.path}:{f.line}: {f.kind}: {f.symbol}")
        print("check-imagery-boundary: " + ("FAIL" if failed else "PASS"))
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
