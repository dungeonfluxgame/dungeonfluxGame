#!/usr/bin/env python3
"""Imagery boundary checker: no direct image execution outside private df-imagery backends.

Scans Rust sources for direct image-executor construction/dispatch. Comments and
string/char literals are stripped before matching, so mentions in docs or test
data are not false positives. Matching runs on the whole stripped token stream
(after stripping), so qualified constructors split across lines or around
comments are still found; each match maps back to its source line. Cargo
dependency and lib.rs export audits run on the same invocation.

Test scope is syntax-scoped with an exact `cfg(test)` token: a whole test
path or an offset inside a `cfg(test)` braced item counts; `cfg(test_mode)`
and other predicates are production, as are compound predicates the scanner
does not prove require test. Production after a closed `cfg(test)` module
does not count. Fixture reasoning is config-argument proof: a forbidden
constructor in test scope is exempt only when its whole top-level first
argument is an `ExactConfig::loopback_fixture(...)` expression with only
explicit `.unwrap()`/`.expect(...)` suffixes, or that argument is one
immutable variable bound by `let name = ExactConfig::loopback_fixture(...)`
in the same function and still visible in the constructor lexical block
(both top-level counts as the same scope) with no intervening shadow or
reassignment before the call. A bare identifier, a block mentioning a
fixture but evaluating to live config, a fixture in another argument, an
unrelated binding, an expired inner-block binding, a `for`/closure/`let`
pattern shadow, a `mut`/typed/destructured binding, or any other
initializer shape exempts nothing.
`ImageRoute::X(_)` with an exact wildcard, match-arm patterns directly
before their own `=>` (no `,`/`;`/`}` between), the pattern argument after
the first top-level comma in `matches!`, and `let` patterns before the same
assignment are inspection, not execution; `matches!` values, arm-body
constructions after `=>`, `let` expressions after `=`,
`ImageRoute::X(named)` construction and `ImageRoute::X(_named)` payloads are
flagged. `submit_image` needs its dispatch dot (`.submit_image(`), so the
provider definition itself is not a finding. `crates/df-imagery/src/backends/`
is the exact intended private execution boundary and is not scanned for
constructor findings; the rest of `df-imagery` (including `service.rs`)
still is. Supported syntax is deliberately narrow; raw HTTP, scripts, type
aliases, and wrapper constructors remain outside this guard.

Legacy transitional sites are budgeted per exact (file, symbol) with an
occurrence count and an owning-function callsite fingerprint matched at the
exact token offset. Only an occurrence whose enclosing `fn` carries the
fingerprint counts, with constructor config tokens also required in the
actual config argument slot (`*Config::live` in the first argument,
`IMAGE_EDIT_TIMEOUT` in the third; registry factory requires its owning
function name); a fingerprint elsewhere (credentials, nested block) never
qualifies. Any other occurrence fails as bypass and a present file with
zero qualifying occurrences fails as stale. Default mode reports legacy
rows nonfatally; --strict fails on them too. Export aliases resolve
through same-file `use` trees to a fixed point, tracking module and item
aliases. Browser-manifest audits forbid unconditional, WASM-target,
double-negation, or other-predicate `df-client/df-tools -> df-imagery`
edges (renames/workspace included); only an exact native-only
`cfg(not(target_arch = "wasm32"))` edge passes in either manifest.

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
    "NativeHeadLocator",
    "NativePlateJudge",
    "NativeFalSegmenter",
)
CONSTRUCTOR_RES: tuple[tuple[str, re.Pattern[str]], ...] = tuple(
    (f"{base}::new", re.compile(r"\b" + base + r"\s*::\s*new\b")) for base in CONSTRUCTOR_BASES
)
IMAGEROUTE_RE: re.Pattern[str] = re.compile(r"\bImageRoute\s*::\s*(OpenAi|Flux|Bfl)\b")
SUBMIT_IMAGE_RE: re.Pattern[str] = re.compile(r"\.\s*submit_image\s*\(")
LOOPBACK_CALL_RE: re.Pattern[str] = re.compile(r"\bloopback_fixture\s*\(")
CFG_TEST_RE: re.Pattern[str] = re.compile(r"cfg\s*\(\s*test\s*\)")
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
LEGACY_ROWS: tuple[LegacyRow, ...] = ()
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
CLIENT_CARGO = "crates/df-client/Cargo.toml"
CLIENT_FORBIDDEN = "df-imagery"
TOOLS_CARGO = "crates/df-tools/Cargo.toml"
_FIXTURE_HEAD_RE: re.Pattern[str] = re.compile(
    r"^(?:[A-Za-z_]\w*::)*\w+Config::loopback_fixture\s*\(",
)
_LIVE_HEAD_RE: re.Pattern[str] = re.compile(
    r"^((?:[A-Za-z_]\w*::)*\w+Config::live)\s*\(",
)
_TIMEOUT_ONLY_RE: re.Pattern[str] = re.compile(
    r"^(?:[A-Za-z_]\w*::)*IMAGE_EDIT_TIMEOUT$",
)
_NATIVE_CFG_NORMALIZED = 'cfg(not(target_arch="wasm32"))'


def _normalize_cfg_segment(segment: str) -> str:
    return re.sub(r"\s+", "", segment)


def _is_whole_fixture_expr(expr: str) -> bool:
    text = expr.strip()
    head = _FIXTURE_HEAD_RE.match(text)
    if head is None:
        return False
    close_idx = _matching_paren(text, head.end() - 1)
    if close_idx == -1:
        return False
    pos = close_idx + 1
    total = len(text)
    while True:
        while pos < total and text[pos] in " \t\r\n":
            pos += 1
        if pos >= total:
            return True
        if text[pos] != ".":
            return False
        pos += 1
        while pos < total and text[pos] in " \t\r\n":
            pos += 1
        name_start = pos
        while pos < total and (text[pos].isalnum() or text[pos] == "_"):
            pos += 1
        name = text[name_start:pos]
        if name not in ("unwrap", "expect"):
            return False
        while pos < total and text[pos] in " \t\r\n":
            pos += 1
        if pos >= total or text[pos] != "(":
            return False
        close = _matching_paren(text, pos)
        if close == -1:
            return False
        if name == "unwrap" and text[pos + 1 : close].strip() != "":
            return False
        pos = close + 1


def _is_whole_live_config_expr(arg: str, fingerprint: str) -> bool:
    text = arg.strip()
    head = _LIVE_HEAD_RE.match(text)
    if head is None:
        return False
    if fingerprint not in head.group(1):
        return False
    close_idx = _matching_paren(text, head.end() - 1)
    if close_idx == -1:
        return False
    return text[close_idx + 1 :].strip() == ""


def _is_whole_timeout_expr(arg: str) -> bool:
    return _TIMEOUT_ONLY_RE.match(arg.strip()) is not None

MAX_FILE_BYTES = 8 * 1024 * 1024
FIXTURE_WINDOW_LINES = 12


@dataclass
class Finding:
    path: str
    line: int
    symbol: str
    kind: str  # "bypass" | "legacy" | "stale" | "dependency" | "export" | "alias"
    offset: int = 0


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


def _split_top_level_args(inner: str) -> list[str]:
    parts: list[str] = []
    depth = 0
    current: list[str] = []
    for ch in inner:
        if ch in "([{":
            depth += 1
            current.append(ch)
        elif ch in ")]}":
            depth = max(0, depth - 1)
            current.append(ch)
        elif ch == "," and depth == 0:
            parts.append("".join(current))
            current = []
        else:
            current.append(ch)
    parts.append("".join(current))
    return parts


def _constructor_first_arg(stripped: str, match_end: int) -> str | None:
    args = _constructor_arg_list(stripped, match_end)
    if args is None or len(args) < 2:
        return None
    inner = args[1:-1]
    split = _split_top_level_args(inner)
    if not split:
        return None
    return split[0].strip()


_SINGLE_IDENT_RE: re.Pattern[str] = re.compile(r"^[A-Za-z_]\w*$")
_ASSIGN_RE: re.Pattern[str] = re.compile(r"(?<![=!<>-])=(?![=>])")
_LET_BIND_RE: re.Pattern[str] = re.compile(r"\blet\s+(?:mut\s+)?(\w+)\b")


def _brace_depth(text: str, offset: int) -> int:
    return text.count("{", 0, offset) - text.count("}", 0, offset)


def _block_still_open(text: str, bind_end: int, use_offset: int, bind_depth: int) -> bool:
    depth = bind_depth
    for ch in text[bind_end:use_offset]:
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth < bind_depth:
                return False
    return True


def _constructor_has_fixture_proof(
    stripped: str, stripped_lines: list[str], line: int, match_end: int
) -> bool:
    # Config-argument proof only: the whole top-level first argument is an
    # `ExactConfig::loopback_fixture(...)` expression (explicit unwrap/expect
    # suffixes only), or is one immutable variable bound by
    # `let name = ExactConfig::loopback_fixture(...)` in the same function
    # and still visible in the constructor lexical block, with no
    # intervening shadow or reassignment before the call. A block that only
    # mentions the fixture but evaluates to live config proves nothing;
    # unsupported binders conservatively invalidate. Unknown syntax refuses.
    first = _constructor_first_arg(stripped, match_end)
    if first is None:
        return False
    if _is_whole_fixture_expr(first):
        return True
    if _SINGLE_IDENT_RE.match(first) is None:
        return False
    name = first
    use_offset = match_end
    fn_here = _enclosing_fn(stripped, use_offset)
    best_bind_start = -1
    best_bind_end = -1
    for binding in re.finditer(
        r"\blet\s+" + re.escape(name) + r"\b\s*=\s*", stripped[:use_offset]
    ):
        init = stripped[binding.end() : use_offset]
        semi = init.find(";")
        if semi != -1:
            init = init[:semi]
        if not _is_whole_fixture_expr(init):
            continue
        binding_fn = _enclosing_fn(stripped, binding.start())
        if (fn_here is None) != (binding_fn is None):
            continue
        if fn_here is not None:
            if binding_fn is None or binding_fn[0] != fn_here[0]:
                continue
        best_bind_start = binding.start()
        best_bind_end = binding.end()
    if best_bind_end == -1:
        return False
    bind_depth = _brace_depth(stripped, best_bind_start)
    use_depth = _brace_depth(stripped, use_offset)
    if bind_depth > use_depth:
        return False
    if not _block_still_open(stripped, best_bind_end, use_offset, bind_depth):
        return False
    between = stripped[best_bind_end:use_offset]
    # Any later `let` pattern mentioning the name invalidates, including
    # destructuring (`let (name, ..) =`) that the plain `let name` check
    # would miss. The original binding itself ends before `between`.
    if re.search(r"\blet\b[^;=]*\b" + re.escape(name) + r"\b[^;=]*=", between) is not None:
        return False
    # `for name in ...` rebinds the identifier for the loop body.
    if re.search(r"\bfor\b[^;{}]*\b" + re.escape(name) + r"\b", between) is not None:
        return False
    # Closure parameters (`|name|`, `|a, name|`) shadow the outer binding.
    if re.search(r"\|[^\n|;{}]*\b" + re.escape(name) + r"\b[^\n|;{}]*\|", between) is not None:
        return False
    # A nested `fn` binder or a match arm (`Some(name) =>`) rebinds it.
    if re.search(r"\bfn\b[^;{]*\b" + re.escape(name) + r"\b", between) is not None:
        return False
    if re.search(r"\b" + re.escape(name) + r"\b[^\n;{}]*=>", between) is not None:
        return False
    if re.search(r"\b" + re.escape(name) + r"\s*=(?![=>])", between):
        return False
    return True


def _statement_start(stripped: str, offset: int) -> int:
    last = max(
        stripped.rfind(";", 0, offset),
        stripped.rfind("{", 0, offset),
        stripped.rfind("}", 0, offset),
    )
    return last + 1 if last != -1 else 0


def _enclosing_matches_range(stripped: str, offset: int) -> tuple[int, int] | None:
    best: tuple[int, int] | None = None
    for macro in MATCHES_MACRO_RE.finditer(stripped):
        j = macro.end()
        while j < len(stripped) and stripped[j] in " \t\r\n":
            j += 1
        if j >= len(stripped) or stripped[j] != "(":
            continue
        close = _matching_paren(stripped, j)
        if close != -1 and j < offset < close:
            if best is None or j > best[0]:
                best = (j, close)
    return best


def _first_top_level_comma(stripped: str, open_idx: int, close_idx: int) -> int:
    depth = 0
    for i in range(open_idx + 1, close_idx):
        ch = stripped[i]
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth = max(0, depth - 1)
        elif ch == "," and depth == 0:
            return i
    return -1


def _is_matches_pattern_arg(stripped: str, offset: int) -> bool:
    span = _enclosing_matches_range(stripped, offset)
    if span is None:
        return False
    comma = _first_top_level_comma(stripped, span[0], span[1])
    if comma == -1:
        return False
    return offset > comma


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
    if _enclosing_matches_range(stripped, match_start) is not None:
        return _is_matches_pattern_arg(stripped, match_start)
    # A route after the current arm arrow is an expression, never a pattern.
    # Only a route directly before its own `=>` (no `,`/`;`/`}` between)
    # is a match-arm pattern.
    after = stripped[close + 1 : close + 201]
    arrow = after.find("=>")
    if arrow != -1:
        terminator = -1
        for marker in (";", ",", "}"):
            pos = after.find(marker)
            if pos != -1 and (terminator == -1 or pos < terminator):
                terminator = pos
        if terminator == -1 or arrow < terminator:
            brace = after.find("{")
            if brace == -1 or arrow < brace:
                return True
    # Destructuring keeps the pattern before the same `let` assignment.
    start = _statement_start(stripped, match_start)
    prefix = stripped[start:match_start]
    if LET_RE.search(prefix) is None:
        return False
    if re.search(r"(?<![=!<>-])=(?![=>])", prefix):
        return False
    suffix = stripped[close + 1 : close + 201]
    suffix_semi = suffix.find(";")
    suffix_window = suffix if suffix_semi == -1 else suffix[:suffix_semi]
    if re.search(r"(?<![=!<>-])=(?![=>])", suffix_window):
        return True
    return False


def _enclosing_fn(stripped: str, offset: int) -> tuple[int, int, str] | None:
    best: tuple[int, int, str] | None = None
    for fn_match in FN_RE.finditer(stripped[:offset]):
        name = fn_match.group(1)
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
                best = (open_idx, close_idx, name)
    return best


def _enclosing_fn_range(stripped: str, offset: int) -> tuple[int, int] | None:
    found = _enclosing_fn(stripped, offset)
    if found is None:
        return None
    return (found[0], found[1])


def _constructor_args_at(stripped: str, offset: int) -> str | None:
    tail = stripped[offset : offset + 200]
    marker = re.search(r"::\s*new\b", tail)
    if marker is None:
        return None
    end = offset + marker.end()
    return _constructor_arg_list(stripped, end)


def _legacy_qualifies(
    stripped: str,
    line: int,
    stripped_lines: list[str],
    offset: int,
    fingerprint: str,
    symbol: str = "",
) -> bool:
    # Exact token offsets only; no nearby fallback. A site outside any
    # function body never qualifies.
    fn = _enclosing_fn(stripped, offset)
    if fn is None:
        return False
    body = stripped[fn[0] : fn[1]]
    if fingerprint not in body:
        return False
    if symbol.endswith("::new"):
        if fingerprint in ("loopback_fixture", "for_test", "FluxPerson"):
            return True
        args = _constructor_args_at(stripped, offset)
        if args is None:
            return False
        split = _split_top_level_args(args[1:-1])
        if fingerprint == "IMAGE_EDIT_TIMEOUT":
            return len(split) >= 3 and _is_whole_timeout_expr(split[2])
        return len(split) >= 1 and _is_whole_live_config_expr(split[0], fingerprint)
    return True


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
            findings.append(Finding(rel, line, label, "bypass", match.start()))

    for match in IMAGEROUTE_RE.finditer(stripped):
        variant = match.group(1)
        if _imageroute_is_pattern(stripped, match.start(), match.end(), variant):
            continue
        line = _offset_to_line(starts, match.start())
        if _is_test_scope(rel, stripped, match.start(), cfg_ranges) or _is_provider_self_test(
            rel, stripped, match.start(), cfg_ranges
        ):
            continue
        findings.append(Finding(rel, line, f"ImageRoute::{variant}", "bypass", match.start()))

    for match in SUBMIT_IMAGE_RE.finditer(stripped):
        line = _offset_to_line(starts, match.start())
        if _is_test_scope(rel, stripped, match.start(), cfg_ranges) or _is_provider_self_test(
            rel, stripped, match.start(), cfg_ranges
        ):
            continue
        findings.append(Finding(rel, line, "submit_image dispatch", "bypass", match.start()))

    for label, rx in alias_res:
        for match in rx.finditer(stripped):
            line = _offset_to_line(starts, match.start())
            findings.append(Finding(rel, line, label, "alias", match.start()))

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


def _trail_is_native_only(trail: list[str]) -> bool:
    for part in trail:
        if _normalize_cfg_segment(part) == _NATIVE_CFG_NORMALIZED:
            return True
    return False


def _audit_browser_manifest(
    root: Path, cargo_rel: str, workspace_deps: dict[str, object]
) -> list[Finding]:
    cargo = root / cargo_rel
    if not cargo.is_file():
        return []
    try:
        text = cargo.read_text(encoding="utf-8")
    except OSError as error:
        raise RuntimeError(f"cannot read {cargo_rel}: {error}") from error
    try:
        data = tomllib.loads(text)
    except Exception as error:
        raise RuntimeError(f"cannot parse {cargo_rel}: {error}") from error
    findings: list[Finding] = []

    def resolve_browser_workspace(dep_name: str) -> str | None:
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
                native_only = _trail_is_native_only(trail)
                for dep_name, spec in node.items():
                    pkg: str | None = None
                    label = dep_name
                    if dep_name == CLIENT_FORBIDDEN:
                        pkg = CLIENT_FORBIDDEN
                    elif isinstance(spec, dict) and spec.get("package") == CLIENT_FORBIDDEN:
                        raw = spec.get("package")
                        pkg = raw if isinstance(raw, str) else CLIENT_FORBIDDEN
                        label = f"{pkg} (renamed as {dep_name})"
                    elif isinstance(spec, dict) and spec.get("workspace"):
                        resolved = resolve_browser_workspace(dep_name)
                        if resolved == CLIENT_FORBIDDEN:
                            pkg = CLIENT_FORBIDDEN
                            label = (
                                f"{pkg} (renamed as {dep_name})"
                                if dep_name != CLIENT_FORBIDDEN
                                else dep_name
                            )
                    if pkg == CLIENT_FORBIDDEN and not native_only:
                        findings.append(
                            Finding(
                                cargo_rel,
                                _find_dep_line(text, dep_name),
                                label,
                                "dependency",
                            )
                        )
            for dep_name, spec in node.items():
                walk(spec, trail + [dep_name])
        elif isinstance(node, list):
            for item in node:
                walk(item, trail)

    walk(data, [])
    return findings


def audit_client_dependencies(root: Path) -> list[Finding]:
    # planning/image-generation-architecture.md:454: no WASM client
    # dependency on native image execution. Only an exact native-only
    # `cfg(not(target_arch = "wasm32"))` edge on df-imagery passes in
    # df-client and df-tools; unconditional, WASM-target, double-negation,
    # and other predicates fail, renames/workspace included.
    if tomllib is None:
        raise RuntimeError("tomllib is unavailable; need Python 3.11+ to parse Cargo.toml")
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
    findings: list[Finding] = []
    findings.extend(_audit_browser_manifest(root, CLIENT_CARGO, workspace_deps))
    findings.extend(_audit_browser_manifest(root, TOOLS_CARGO, workspace_deps))
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
    use_bodies = [m.group(1) for m in USE_STMT_RE.finditer(stripped)]
    for body in use_bodies:
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
    for _ in range(10):
        grown = False
        for body in use_bodies:
            for alias in re.finditer(r"\b(\w+)\s+as\s+(\w+)", body):
                left, right = alias.group(1), alias.group(2)
                if left in mod_aliases and right not in mod_aliases:
                    mod_aliases.add(right)
                    grown = True
                elif left in backend_items and right not in backend_items:
                    backend_items.add(right)
                    grown = True
            for chained in re.finditer(r"\b(\w+)\s*::\s*(\w+)\s+as\s+(\w+)", body):
                if chained.group(1) in mod_aliases and chained.group(3) not in backend_items:
                    backend_items.add(chained.group(3))
                    grown = True
            for direct in re.finditer(r"\b(\w+)\s*::\s*(\w+)", body):
                if direct.group(1) in mod_aliases and direct.group(2) not in backend_items:
                    if re.search(
                        r"\b" + re.escape(direct.group(2)) + r"\s+as\b", body
                    ) is None:
                        backend_items.add(direct.group(2))
                        grown = True
            grouped_chain = re.search(r"\b(\w+)\s*::\s*\{([^}]*)\}", body)
            if grouped_chain and grouped_chain.group(1) in mod_aliases:
                for part in grouped_chain.group(2).split(","):
                    part = part.strip()
                    alias_pair = re.match(r"(\w+)\s+as\s+(\w+)", part)
                    if alias_pair and alias_pair.group(2) not in backend_items:
                        backend_items.add(alias_pair.group(2))
                        grown = True
                    elif re.match(r"\w+$", part) and part not in backend_items:
                        backend_items.add(part)
                        grown = True
        if not grown:
            break
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


RETIRED_PROVIDER_TYPES = CONSTRUCTOR_BASES + (
    "ImageEditConfig", "FalImageConfig", "BflImageConfig", "HeadLocateConfig",
    "PlateJudgeConfig", "FalSegmentConfig",
)

def audit_retired_provider_exports(root: Path) -> list[Finding]:
    findings: list[Finding] = []
    for path in sorted((root / "crates/df-providers/src").rglob("*.rs")):
        stripped = strip_rust(path.read_text(encoding="utf-8"))
        starts = _line_starts(stripped)
        for match in re.finditer(r"\bpub\s+(?:struct|enum|type)\s+(\w+)", stripped):
            if match.group(1) in RETIRED_PROVIDER_TYPES:
                findings.append(Finding(path.relative_to(root).as_posix(),
                    _offset_to_line(starts, match.start()),
                    "retired public image executor: " + match.group(1), "export"))
        for match in PUB_USE_STMT_RE.finditer(stripped):
            for name in RETIRED_PROVIDER_TYPES:
                if re.search(r"\b" + re.escape(name) + r"\b", match.group(1)):
                    findings.append(Finding(path.relative_to(root).as_posix(),
                        _offset_to_line(starts, match.start()),
                        "retired public image executor re-export: " + name, "export"))
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
        offset = finding.offset
        if _legacy_qualifies(stripped, finding.line, lines, offset, row.fingerprint, finding.symbol):
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
        qualifies = _legacy_qualifies(
            stripped, finding.line, lines, finding.offset, row.fingerprint, finding.symbol
        )
        used = seen.get(key, 0)
        if qualifies:
            seen[key] = used + 1
            if used < row.expected:
                findings.append(
                    Finding(finding.path, finding.line, finding.symbol, "legacy", finding.offset)
                )
            else:
                findings.append(
                    Finding(finding.path, finding.line, finding.symbol, "bypass", finding.offset)
                )
        else:
            findings.append(
                Finding(finding.path, finding.line, finding.symbol, "bypass", finding.offset)
            )
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
    findings.extend(audit_client_dependencies(root))
    findings.extend(audit_exports(root))
    findings.extend(audit_retired_provider_exports(root))
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
