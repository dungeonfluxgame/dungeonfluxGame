#!/usr/bin/env python3
"""Static boundary guard for text-model egress and text-provider construction.

Authority: planning/text-generation-architecture.md, section "Boundary guard and
acceptance". Raw text egress may occur only in exact private backend modules and the
designated shared transport. Migrated consumers may not import, alias, re-export, call,
or construct the public text factories, providers, or old Codex process wrappers.

Checks (exit 1 on any unaccounted hit):
- Rust, crates/*/src/**/*.rs and crates/*/tests/**/*.rs. Comments are stripped; string
  contents are kept as literal values. Classes:
  import: a use or type-alias line naming a banned identifier (aliases resolved).
  call: a banned identifier followed by ( :: or {, codex_cli::text( (also through a
    use alias), NativeTextProvider .submit( in a file that names NativeTextProvider,
    Command::new( whose argument names codex or that sits in a codex_cli module.
  process: Command::new( (also via a use alias) naming codex, curl or powershell/pwsh
    (the last only in a file with an HTTP cmdlet string), directly or through a variable
    assigned in the same file from a string or env::var name that names it.
  identifier: any other code mention of a banned identifier (strings and comments are
    not code, so they never match).
  literal: an endpoint in a production string value (joined across "+" and concat!).
  split-literal: an endpoint assembled from named string constants (const, static, let)
    through format!, concat!, +, .join(, .push_str( or += within one file.
  test-literal: an endpoint in a doc comment, inside cfg(test) or #[test] scope, or in
    a *_tests.rs / tests.rs / test_*.rs file or a tests/ directory.
- crates/*/Cargo.toml (class cargo, never allowlisted). df-ai and df-provider-api may not
  reach HTTP clients, process-spawning crates, tokio "process" or df-providers through
  normal dependencies, directly or via workspace crates. reqwest may appear only in
  REQWEST_BASELINE crates; REQWEST_OWNERS may join it by a reviewed baseline edit.
- development/*.py, *.sh, *.ps1 (top level only). Class egress: an egress tool (curl,
  wget, Invoke-RestMethod, Invoke-WebRequest, irm, iwr, requests.post/put/get/request,
  urlopen) in a file that also contains an endpoint literal.
- ALLOWLIST keys (path, class, symbol) map to a recorded count, phase and reason. Actual
  hits above the recorded count fail. PHASE_CEILINGS caps the allowlisted total per
  phase at the counts recorded when written. Entries name exact files only.

Known limits: endpoints or identifiers assembled at run time (config, environment
values, generated code). Split constants and indirect Command are per-file heuristics:
macro-generated names, cross-file constants, struct fields, parameters and values from
non-constant variables are not expanded. A banned name reached only through a
differently named re-export; the receiver of .submit( in files that never name
NativeTextProvider; raw HTTP or SDK imports that avoid a literal endpoint. Cargo checks
read declared dependencies only (no feature unification, cfg evaluation, workspace-
inherited features or crates outside crates/). The runtime egress tests cover what a
lexical scan cannot prove.

Exit codes: 0 pass, 1 unaccounted hit or allowlist growth, 2 usage error.
Stdlib only (Python 3.11+ for tomllib). Makes no network calls.
"""
from __future__ import annotations

import argparse
import ast
import json
import re
import sys
from pathlib import Path

try:
    import tomllib
except ImportError:  # Python < 3.11: the Cargo check reports this instead of passing
    tomllib = None

ENDPOINTS = ("/v1/responses", "/v1/chat/completions", "/v1/systemone")
BANNED = frozenset({"registered_text_provider", "NativeTextProvider", "SystemOneChoiceProvider",
                    "NativePlateJudge", "NativeHeadLocator"})
CODEX_TEXT = "codex_cli::text"
CLASSES = ("import", "call", "identifier", "process", "literal", "split-literal", "test-literal",
           "egress", "cargo")
MACROS = frozenset({"format", "format_args", "concat"})
NOOP_CALLS = frozenset({"to_string", "to_owned", "as_str", "into", "clone", "as_ref", "to_str"})
APPEND_MARKERS = frozenset({"+", "join", "push", "push_str", "push_back"})
PS_HTTP = re.compile(r"(?:Invoke-RestMethod|Invoke-WebRequest|(?<![\w-])(?:irm|iwr)(?![\w-]))", re.I)
EXE_NAMES = re.compile(r"(?<![a-z])(curl|powershell|pwsh)(?![a-z])")
# Crates that must stay free of HTTP clients, process spawning and the provider crate.
GUARDED_CRATES = ("df-ai", "df-provider-api")
BANNED_DEPS = frozenset({"reqwest", "hyper", "hyper-util", "ureq", "isahc", "surf", "attohttpc",
                         "tokio-process", "async-process", "duct", "subprocess", "async-openai",
                         "df-providers"})
# Crates listing reqwest in any dependency table as of 2026-11-01. Growth fails.
REQWEST_BASELINE = frozenset({"df-campaign", "df-google", "df-hosted", "df-providers", "df-workos"})
# Crates that may be added to the baseline by a reviewed edit; every other crate may not.
REQWEST_OWNERS = REQWEST_BASELINE | frozenset({"df-tools", "df-launcher", "df-imagery"})
SKIP_SEGMENTS = frozenset({"target", "artifacts", "planning", "ADR", "docs", "archive"})
SHELL_TOOLS = re.compile(r"(?<![\w.-])(curl|wget|Invoke-RestMethod|Invoke-WebRequest|irm|iwr)(?![\w.-])", re.I)
PY_EGRESS = frozenset({"requests.post", "requests.put", "requests.get", "requests.request",
                       "urllib.request.urlopen", "urlopen"})
TOKEN = re.compile(
    r"(?P<ws>\s+)|(?P<line>//[^\n]*)|(?P<block>/\*)|(?P<raw>[bc]?r#*\")"
    r"|(?P<str>[bc]?\"[^\"\\]*(?:\\.[^\"\\]*)*\")"
    r"|(?P<char>'(?:\\.[^'\n]*|[^'\\\n])')|(?P<life>'[^\W\d]\w*)"
    r"|(?P<ident>(?:r#)?[^\W\d]\w*)|(?P<num>\d\w*)|(?P<punct>::|->|=>|\.\.=?|.)",
    re.DOTALL,
)



def _allow(rows):
    return {(p, c, s): {"count": n, "phase": ph, "reason": r} for p, c, s, n, ph, r in rows}


# (path, class, symbol) -> {"count", "phase", "reason"}. Derived from a scanner run on the
# tree as of 2026-11-01. Exact files only; a hit above the recorded count fails.
PHASE_REASONS = {
    'backend': 'private backend module: the sanctioned raw-egress owner behind its service',
    'provider-crate': "the provider crate's own implementation, wire tests and re-exports; reached only by the registered backends", 'image-route-config': 'route configuration handed to the image service, not egress code',
    'fixture': 'endpoint fixture in test code',
}

# (path, class, symbol) -> count. Every entry is permanent and justified; none is a migration
# placeholder. A hit above the recorded count fails, and so does an entry nothing matches.
ALLOWLIST = _allow([
    # backend
    ("crates/df-hosted/src/support_ingest/support_text.rs", "literal", "/v1/responses", 1, "backend", PHASE_REASONS["backend"]),
    ("crates/df-hosted/src/support_ingest/support_text.rs", "literal", "/v1/systemone", 1, "backend", PHASE_REASONS["backend"]),
    ("crates/df-imagery/src/backends/codex_cli/process.rs", "process", "Command::new", 1, "backend", PHASE_REASONS["backend"]),
    ("crates/df-tools/src/gameplay/responses_text.rs", "call", "NativeTextProvider.submit", 1, "backend", PHASE_REASONS["backend"]),
    ("crates/df-tools/src/gameplay/responses_text.rs", "call", "registered_text_provider", 2, "backend", PHASE_REASONS["backend"]),
    ("crates/df-tools/src/gameplay/responses_text.rs", "identifier", "NativeTextProvider", 1, "backend", PHASE_REASONS["backend"]),
    ("crates/df-tools/src/gameplay/responses_text.rs", "import", "NativeTextProvider", 1, "backend", PHASE_REASONS["backend"]),
    ("crates/df-tools/src/gameplay/responses_text.rs", "import", "registered_text_provider", 1, "backend", PHASE_REASONS["backend"]),
    ("crates/df-tools/src/gameplay/systemone_text.rs", "call", "SystemOneChoiceProvider", 1, "backend", PHASE_REASONS["backend"]),
    ("crates/df-tools/src/gameplay/systemone_text.rs", "identifier", "SystemOneChoiceProvider", 1, "backend", PHASE_REASONS["backend"]),
    ("crates/df-tools/src/gameplay/systemone_text.rs", "import", "SystemOneChoiceProvider", 1, "backend", PHASE_REASONS["backend"]),
    # provider-crate
    ("crates/df-providers/src/lib.rs", "import", "NativeTextProvider", 1, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/src/lib.rs", "import", "SystemOneChoiceProvider", 1, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/src/lib.rs", "import", "registered_text_provider", 1, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/src/registry.rs", "call", "NativeTextProvider", 1, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/src/registry.rs", "call", "registered_text_provider", 1, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/src/registry.rs", "identifier", "NativeTextProvider", 1, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/src/systemone.rs", "call", "SystemOneChoiceProvider", 2, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/src/systemone.rs", "literal", "/v1/systemone", 1, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/src/text_response.rs", "call", "NativeTextProvider", 3, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/src/text_response.rs", "literal", "/v1/responses", 2, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/tests/metering.rs", "call", "SystemOneChoiceProvider", 2, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/tests/metering.rs", "import", "SystemOneChoiceProvider", 1, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/tests/native_http.rs", "call", "registered_text_provider", 6, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/tests/native_http.rs", "import", "registered_text_provider", 1, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/tests/text_response.rs", "call", "NativeTextProvider", 1, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/tests/text_response.rs", "call", "NativeTextProvider.submit", 22, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/tests/text_response.rs", "call", "registered_text_provider", 7, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/tests/text_response.rs", "import", "NativeTextProvider", 1, "provider-crate", PHASE_REASONS["provider-crate"]),
    ("crates/df-providers/tests/text_response.rs", "import", "registered_text_provider", 1, "provider-crate", PHASE_REASONS["provider-crate"]),
    # image-route-config
    ("crates/df-tools/src/gameplay/image_configuration.rs", "literal", "/v1/chat/completions", 1, "image-route-config", PHASE_REASONS["image-route-config"]),
    ("crates/df-tools/src/gameplay/image_configuration.rs", "literal", "/v1/systemone", 1, "image-route-config", PHASE_REASONS["image-route-config"]),
    # fixture
    ("crates/df-hosted/src/support_ingest/support_text.rs", "test-literal", "/v1/responses", 1, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-hosted/src/support_ingest/support_text.rs", "test-literal", "/v1/systemone", 3, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-imagery/src/backends/analysis/standalone/legacy_photo_tests.rs", "test-literal", "/v1/chat/completions", 1, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-imagery/src/config_overlay_tests.rs", "test-literal", "/v1/responses", 2, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-imagery/src/config_tests.rs", "test-literal", "/v1/responses", 1, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-imagery/src/runtime/analysis/creator_legacy_tests.rs", "test-literal", "/v1/systemone", 3, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-imagery/src/runtime/quota/tests.rs", "test-literal", "/v1/responses", 3, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-persistence/src/imagery_runtime_admission/no_output_tests.rs", "test-literal", "/v1/chat/completions", 1, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-persistence/tests/imagery_publication/intent_fixture.rs", "test-literal", "/v1/chat/completions", 1, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-providers/src/native_http.rs", "test-literal", "/v1/responses", 3, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-providers/src/systemone.rs", "test-literal", "/v1/systemone", 1, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-providers/tests/text_response.rs", "test-literal", "/v1/responses", 1, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-tools/src/gameplay/creator_portrait_route_tests.rs", "test-literal", "/v1/systemone", 5, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-tools/src/gameplay/imagery/mural_prompt.rs", "test-literal", "/v1/chat/completions", 1, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-tools/src/gameplay/imagery_tests.rs", "test-literal", "/v1/chat/completions", 3, "fixture", PHASE_REASONS["fixture"]),
    ("crates/df-tools/src/gameplay/living_scenes/plates_tests.rs", "test-literal", "/v1/chat/completions", 1, "fixture", PHASE_REASONS["fixture"]),
])
# Allowlisted-hit ceiling per phase, fixed at the counts recorded when written.
PHASE_CEILINGS = {"backend": 12, "provider-crate": 56, "image-route-config": 2, "fixture": 31}


def _is_ident(tok, value):
    return tok[0] == "ident" and tok[1] == value


def _is_punct(tok, value):
    return tok[0] == "punct" and tok[1] == value


def _lex(text):
    """Return (tokens, doc_comments). Tokens are (kind, value, line)."""
    toks, docs = [], []
    pos, line, n = 0, 1, len(text)
    while pos < n:
        if text.startswith("/*", pos):
            depth, j = 0, pos
            while j < n:
                if text.startswith("/*", j):
                    depth += 1
                    j += 2
                elif text.startswith("*/", j):
                    depth -= 1
                    j += 2
                    if depth == 0:
                        break
                else:
                    j += 1
            chunk = text[pos:j]
            if chunk.startswith("/**") and not chunk.startswith("/**/"):
                docs.append((line, chunk))
            line += chunk.count("\n")
            pos = j
            continue
        m = TOKEN.match(text, pos)
        if m is None:
            pos += 1
            continue
        kind, chunk, end = m.lastgroup, m.group(0), m.end()
        if kind == "raw":
            close = '"' + "#" * chunk.count("#")
            stop = text.find(close, end)
            stop = n if stop < 0 else stop
            toks.append(("str", text[end:stop], line))
            end = min(n, stop + len(close))
            chunk = text[pos:end]
        elif kind == "str":
            toks.append(("str", chunk[chunk.index('"') + 1:-1], line))
        elif kind == "line":
            if (chunk.startswith("///") and not chunk.startswith("////")) or chunk.startswith("//!"):
                docs.append((line, chunk))
        elif kind == "ident":
            toks.append(("ident", chunk[2:] if chunk.startswith("r#") else chunk, line))
        elif kind in ("char", "life", "num", "punct"):
            toks.append((kind, chunk, line))
        line += chunk.count("\n")
        pos = end
    return toks, docs


def _item_end(toks, j):
    """Index just past the item that starts at j (its braced body or terminating ;)."""
    depth, k, n = 0, j, len(toks)
    while k < n:
        if toks[k][0] == "punct":
            if toks[k][1] in ("(", "["):
                depth += 1
            elif toks[k][1] in (")", "]"):
                depth -= 1
            elif depth == 0 and toks[k][1] == ";":
                return k + 1
            elif depth == 0 and toks[k][1] == "{":
                level = 0
                while k < n:
                    if _is_punct(toks[k], "{"):
                        level += 1
                    elif _is_punct(toks[k], "}"):
                        level -= 1
                        if level == 0:
                            return k + 1
                    k += 1
                return n
        k += 1
    return n


def _test_mask(toks):
    """Mark tokens inside #[cfg(test)] or #[test] items (any nesting depth)."""
    mask = [False] * len(toks)
    k, n = 0, len(toks)
    while k + 1 < n:
        if _is_punct(toks[k], "#") and _is_punct(toks[k + 1], "["):
            j, depth = k + 2, 1
            while j < n and depth:
                if _is_punct(toks[j], "["):
                    depth += 1
                elif _is_punct(toks[j], "]"):
                    depth -= 1
                j += 1
            attr = [t[1] for t in toks[k + 2:j - 1]]
            if attr == ["cfg", "(", "test", ")"] or (
                    attr and attr[-1] == "test" and (len(attr) == 1 or attr[-2] == "::")):
                for x in range(k, _item_end(toks, j)):
                    mask[x] = True
            k = j
            continue
        k += 1
    return mask


def _is_test_path(rel):
    name = rel.rsplit("/", 1)[-1]
    return "/tests/" in "/" + rel or bool(re.search(r"^test_|_tests?\.rs$|^tests?\.rs$", name))


def _group_end(toks, k):
    """Index of the bracket that closes the group opened at k."""
    depth = 0
    for j in range(k, len(toks)):
        if toks[j][0] == "punct":
            if toks[j][1] in ("(", "[", "{"):
                depth += 1
            elif toks[j][1] in (")", "]", "}"):
                depth -= 1
                if depth == 0:
                    return j
    return len(toks) - 1


def _split_args(toks, a, b):
    """Top-level comma-separated (start, end) spans in toks[a:b]."""
    out, depth, start = [], 0, a
    for j in range(a, b):
        if toks[j][0] == "punct":
            if toks[j][1] in ("(", "[", "{"):
                depth += 1
            elif toks[j][1] in (")", "]", "}"):
                depth -= 1
            elif toks[j][1] == "," and depth == 0:
                out.append((start, j))
                start = j + 1
    if start < b:
        out.append((start, b))
    return out


def _assignments(toks):
    """Yield (name, rhs tokens) for each const/static/let NAME [: T] = rhs; in a token list."""
    n = len(toks)
    for k in range(n):
        if toks[k][0] != "ident" or toks[k][1] not in ("const", "static", "let"):
            continue
        j = k + 1 + (k + 1 < n and _is_ident(toks[k + 1], "mut"))
        if j >= n or toks[j][0] != "ident":
            continue
        name, j, limit = toks[j][1], j + 1, min(n, j + 15)
        while j < limit and not _is_punct(toks[j], "=") and not _is_punct(toks[j], ";"):
            j += 1
        if j >= n or not _is_punct(toks[j], "="):
            continue
        end, depth = j + 1, 0
        while end < n and not (depth == 0 and _is_punct(toks[end], ";")):
            if toks[end][0] == "punct":
                depth += (toks[end][1] in ("(", "[", "{")) - (toks[end][1] in (")", "]", "}"))
            end += 1
        yield name, toks[j + 1:end]


def _string_consts(toks):
    """name -> (value, line) for assignments of one string literal (optionally .to_string())."""
    return {name: (rhs[0][1], rhs[0][2]) for name, rhs in _assignments(toks)
            if rhs and rhs[0][0] == "str" and (len(rhs) == 1 or _is_punct(rhs[1], "."))}


def _operand(toks, k, consts):
    """Parse a string-valued operand at k into ([(text, from_const)], end), or None."""
    n = len(toks)
    while k < n and _is_punct(toks[k], "&"):
        k += 1
    if k >= n:
        return None
    t, nxt = toks[k], (toks[k + 1] if k + 1 < n else None)
    prev = toks[k - 1] if k else None
    if t[0] == "str":
        pieces, end = [(t[1], False)], k + 1
    elif (t[0] == "ident" and t[1] in consts and not (nxt and nxt[0] == "punct" and nxt[1] in ("(", "!"))
          and not (prev and (_is_punct(prev, ".") or (prev[0] == "ident" and prev[1] in ("const", "static", "let", "mut"))))):
        pieces, end = [(consts[t[1]][0], True)], k + 1
    elif (t[0] == "ident" and t[1] in MACROS and nxt and _is_punct(nxt, "!") and k + 2 < n
          and toks[k + 2][0] == "punct" and toks[k + 2][1] in ("(", "[", "{")):
        close = _group_end(toks, k + 2)
        pieces, end = _macro_pieces(toks, t[1], k + 3, close, consts), close + 1
    elif _is_punct(t, "["):
        close = _group_end(toks, k)
        if close + 3 >= n or not _is_punct(toks[close + 1], ".") or toks[close + 2][1] not in ("join", "concat"):
            return None
        sep = toks[close + 4][1] if toks[close + 2][1] == "join" and toks[close + 4][0] == "str" else ""
        pieces = []
        for s, e in _split_args(toks, k + 1, close):
            op = _operand(toks, s, consts)
            pieces.extend(op[0] if op and op[1] == e else [("\0", False)])
            pieces.append((sep, False))
        end = _group_end(toks, close + 3) + 1
    else:
        return None
    while (end + 3 < n and _is_punct(toks[end], ".") and toks[end + 1][0] == "ident"
           and toks[end + 1][1] in NOOP_CALLS and _is_punct(toks[end + 2], "(") and _is_punct(toks[end + 3], ")")):
        end += 4
    return pieces, end


def _macro_pieces(toks, name, a, b, consts):
    """Pieces produced by concat!/format!/format_args! with constants substituted."""
    args = _split_args(toks, a, b)

    def one(s, e):
        op = _operand(toks, s, consts)
        return op[0] if op and op[1] == e else [("\0", False)]
    if name == "concat":
        return [p for s, e in args for p in one(s, e)]
    fmt = next((i for i, (s, e) in enumerate(args) if e - s == 1 and toks[s][0] == "str"), None)
    if fmt is None:
        return [("\0", False)]
    positional = [one(s, e) for s, e in args[fmt + 1:]]
    text, out, pos, auto = toks[args[fmt][0]][1], [], 0, 0
    for m in re.finditer(r"\{\{|\}\}|\{([^{}:]*)(?::[^{}]*)?\}", text):
        if m.start() > pos:
            out.append((text[pos:m.start()], False))
        pos = m.end()
        if m.group(0) in ("{{", "}}"):
            out.append((m.group(0)[0], False))
            continue
        key, piece = m.group(1).strip(), None
        if key == "" or key.isdigit():
            idx = auto if key == "" else int(key)
            auto += key == ""
            piece = positional[idx] if idx < len(positional) else None
        elif key in consts:
            piece = [(consts[key][0], True)]
        out.extend(piece or [("\0", False)])
    out.append((text[pos:], False))
    return out


def _linked(conn):
    """True when the tokens between two operands append the second onto the first."""
    vals = [t[1] for t in conn]
    if not vals or len(vals) > 12 or not any(v in APPEND_MARKERS for v in vals):
        return False
    for i, t in enumerate(conn):
        if t[0] == "punct" and t[1] not in ("+", ".", ")", "(", ";", "=", "&"):
            return False
        if t[0] != "ident":
            continue
        after = vals[i + 1:i + 3]
        if not (t[1] in APPEND_MARKERS or t[1] in NOOP_CALLS or after[:1] == ["."] or after == ["+", "="]):
            return False
    return True


def _spanned(pieces):
    """Endpoints formed across two or more pieces, at least one of them a named constant."""
    found = []
    for sep in ("", "/"):
        text, bounds = "", []
        for part, from_const in pieces:
            text += (sep if text else "") + part
            bounds.append((len(text) - len(part), len(text), from_const))
        for ep in ENDPOINTS:
            for m in re.finditer(re.escape(ep), text):
                touched = [b for b in bounds if b[0] < m.end() and b[1] > m.start()]
                if len(touched) >= 2 and any(b[2] for b in touched) and ep not in found:
                    found.append(ep)
    return found


def _split_hits(toks):
    """Heuristic split-literal hits: endpoints joined from named string constants."""
    consts = _string_consts(toks)
    hits, run, line, end, k = [], [], 0, -1, 0
    while k < len(toks):
        kind, val = toks[k][:2]
        op = _operand(toks, k, consts) if kind == "str" or val in consts or val in MACROS or val in ("[", "&") else None
        if op is None:
            k += 1
            continue
        if run and not _linked(toks[end:k]):
            hits.extend((line, "split-literal", ep) for ep in _spanned(run))
            run = []
        line = line if run else toks[k][2]
        run += op[0]
        end = k = op[1]
    return hits + [(line, "split-literal", ep) for ep in _spanned(run)]


def _names_guarded_exe(value, ps_http, assigned=False):
    """A string naming codex, curl or (with an HTTP cmdlet in the file) powershell; all-caps = env var."""
    if assigned and value.isupper() and not re.search(r"(BIN|EXE|EXECUTABLE|PATH|PROGRAM|CMD|COMMAND)$", value):
        return False
    low = value.lower()
    return "codex" in low or any(ps_http or m.group(1).startswith("c") for m in EXE_NAMES.finditer(low))


def _guarded_vars(toks, ps_http):
    """Names assigned (let/const/static, same file) from a string naming a guarded executable."""
    assigns = [(name, rhs) for name, rhs in _assignments(toks) if len(rhs) <= 20
               and not any(r[0] == "punct" and r[1] in ("{", "}", "=>") for r in rhs)]
    guarded, changed = set(), True
    while changed:
        changed = False
        for name, rhs in assigns:
            if name not in guarded and any((r[0] == "str" and _names_guarded_exe(r[1], ps_http, True))
                                           or (r[0] == "ident" and r[1] in guarded) for r in rhs):
                guarded.add(name)
                changed = True
    return guarded


def _canonical(name, has_codex):
    if name in BANNED:
        return name
    if name == "text" and has_codex:
        return CODEX_TEXT
    return None


def analyze_rust(rel, text):
    """Return (line, class, symbol) hits for one Rust file."""
    toks, docs = _lex(text)
    n = len(toks)
    mask = _test_mask(toks)
    in_use = [False] * n
    alias = {}
    hits = []
    native_named, submits, cmd_names = False, [], {"Command"}
    k = 0
    while k < n:
        if _is_ident(toks[k], "use"):
            end = k
            while end < n and not _is_punct(toks[end], ";"):
                end += 1
            stmt = toks[k:end + 1]
            for x in range(k, min(end + 1, n)):
                in_use[x] = True
            names = {s[1] for s in stmt if s[0] == "ident"}
            has_codex = "codex_cli" in names
            native_named = native_named or "NativeTextProvider" in names
            for s in stmt:
                if s[0] == "ident" and s[1] in BANNED:
                    hits.append((s[2], "import", s[1]))
            if has_codex and "text" in names:
                hits.append((toks[k][2], "import", CODEX_TEXT))
                alias["text"] = CODEX_TEXT
            for x in range(1, len(stmt) - 1):
                if _is_ident(stmt[x], "as") and stmt[x - 1][0] == "ident" and stmt[x + 1][0] == "ident":
                    canon = _canonical(stmt[x - 1][1], has_codex)
                    if canon:
                        alias[stmt[x + 1][1]] = canon
                    if stmt[x - 1][1] == "Command":
                        cmd_names.add(stmt[x + 1][1])
            k = end + 1
            continue
        if (_is_ident(toks[k], "type") and k + 3 < n and toks[k + 1][0] == "ident"
                and _is_punct(toks[k + 2], "=") and toks[k + 3][0] == "ident"):
            canon = _canonical(toks[k + 3][1], False)
            if canon:
                alias[toks[k + 1][1]] = canon
        k += 1

    for k, t in enumerate(toks):
        if t[0] != "ident" or in_use[k]:
            continue
        prev = toks[k - 1] if k else None
        pprev = toks[k - 2] if k > 1 else None
        nxt = toks[k + 1] if k + 1 < n else None
        name = alias.get(t[1], t[1])
        if t[1] == "text" and prev and _is_punct(prev, "::") and pprev and _is_ident(pprev, "codex_cli"):
            name = CODEX_TEXT
        if name == CODEX_TEXT:
            if nxt and _is_punct(nxt, "("):
                hits.append((t[2], "call", CODEX_TEXT))
        elif name in BANNED:
            native_named = native_named or name == "NativeTextProvider"
            is_call = bool(nxt) and nxt[0] == "punct" and nxt[1] in ("(", "::", "{")
            hits.append((t[2], "call" if is_call else "identifier", name))
        elif t[1] == "submit" and prev and _is_punct(prev, ".") and nxt and _is_punct(nxt, "("):
            submits.append(t[2])
    if native_named:
        hits.extend((line, "call", "NativeTextProvider.submit") for line in submits)

    codex_file = "codex_cli" in rel.split("/") or any(_is_ident(t, "codex_cli") for t in toks)
    ps_http = any(t[0] == "str" and PS_HTTP.search(t[1]) for t in toks)
    guarded = _guarded_vars(toks, ps_http) if any(name in text for name in cmd_names) else set()
    for k in range(n - 3):
        if (not in_use[k] and toks[k][0] == "ident" and toks[k][1] in cmd_names and _is_punct(toks[k + 1], "::")
                and _is_ident(toks[k + 2], "new") and _is_punct(toks[k + 3], "(")):
            depth, j, names_codex = 1, k + 4, False
            while j < n and depth:
                if _is_punct(toks[j], "("):
                    depth += 1
                elif _is_punct(toks[j], ")"):
                    depth -= 1
                elif toks[j][0] in ("ident", "str") and "codex" in toks[j][1].lower():
                    names_codex = True
                elif ((toks[j][0] == "ident" and toks[j][1] in guarded)
                      or (toks[j][0] == "str" and _names_guarded_exe(toks[j][1], ps_http))):
                    names_codex = True
                j += 1
            if names_codex or codex_file:
                hits.append((toks[k][2], "process", "Command::new"))

    i = 0
    while i < n:
        if toks[i][0] != "str":
            i += 1
            continue
        seps = ("+",)
        if i >= 3 and _is_punct(toks[i - 1], "(") and _is_punct(toks[i - 2], "!") and _is_ident(toks[i - 3], "concat"):
            seps = (",",)
        parts, j = [toks[i][1]], i
        while j + 2 < n and toks[j + 1][0] == "punct" and toks[j + 1][1] in seps and toks[j + 2][0] == "str":
            parts.append(toks[j + 2][1])
            j += 2
        joined = "".join(parts)
        cls = "test-literal" if (_is_test_path(rel) or mask[i]) else "literal"
        hits.extend((toks[i][2], cls, ep) for ep in ENDPOINTS if ep in joined)
        i = j + 1

    hits.extend((line, "test-literal", ep) for line, doc in docs for ep in ENDPOINTS if ep in doc)
    hits.extend(_split_hits(toks))
    return hits


def _dotted(node):
    parts = []
    while isinstance(node, ast.Attribute):
        parts.append(node.attr)
        node = node.value
    if isinstance(node, ast.Name):
        parts.append(node.id)
    return ".".join(reversed(parts))


def _py_hits(text):
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return [(ln, "egress", m.group(1)) for ln, line in enumerate(text.splitlines(), 1)
                for m in re.finditer(r"(requests\.(?:post|put|get|request)|urlopen)\s*\(", line)]
    hits = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Call):
            name = _dotted(node.func)
            if name in PY_EGRESS or name.endswith(".urlopen"):
                hits.append((node.lineno, "egress", name))
        elif isinstance(node, ast.Constant) and isinstance(node.value, str):
            m = SHELL_TOOLS.match(node.value.strip())
            if m:
                hits.append((node.lineno, "egress", m.group(1).lower()))
    return hits


def analyze_script(rel, text):
    """Return (line, class, symbol) hits for one .py/.sh/.ps1 file."""
    if not any(ep in text for ep in ENDPOINTS):
        return []
    if rel.endswith(".py"):
        return _py_hits(text)
    hits = []
    for ln, line in enumerate(text.splitlines(), 1):
        if not line.lstrip().startswith("#"):
            hits.extend((ln, "egress", m.group(1).lower()) for m in SHELL_TOOLS.finditer(line))
    return hits


def analyze(rel, text):
    return analyze_rust(rel, text) if rel.endswith(".rs") else analyze_script(rel, text)


def collect_files(root):
    found = set()
    for pattern in ("crates/*/src/**/*.rs", "crates/*/tests/**/*.rs",
                    "development/*.py", "development/*.sh", "development/*.ps1"):
        found.update(p for p in root.glob(pattern) if p.is_file())
    rels = []
    for p in sorted(found):
        rel = p.relative_to(root).as_posix()
        if any(seg in SKIP_SEGMENTS for seg in rel.split("/")) or "dungeonflux.old" in rel:
            continue
        rels.append(rel)
    return rels


def _dep_tables(doc):
    """Yield (is_dev, real crate name, features) for every dependency table in a manifest."""
    for scope in [doc, *(doc.get("target") or {}).values()]:
        for table, dev in (("dependencies", False), ("build-dependencies", False), ("dev-dependencies", True)):
            for key, spec in (scope.get(table) or {}).items():
                spec = spec if isinstance(spec, dict) else {}
                yield dev, spec.get("package", key).lower().replace("_", "-"), spec.get("features", [])


def cargo_check(root):
    """Return (violations, stale notes) for crates/*/Cargo.toml dependency reachability."""
    out, stale, manifests = [], [], {}
    paths = sorted(root.glob("crates/*/Cargo.toml"))
    if paths and tomllib is None:
        return [{"path": "crates", "line": 1, "class": "cargo", "symbol": "tomllib",
                 "why": "Python 3.11+ is required to parse Cargo.toml"}], stale
    for path in paths:
        rel = path.relative_to(root).as_posix()
        if any(seg in SKIP_SEGMENTS for seg in rel.split("/")) or "dungeonflux.old" in rel:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        try:
            doc = tomllib.loads(text)
        except tomllib.TOMLDecodeError as exc:
            out.append({"path": rel, "line": 1, "class": "cargo", "symbol": "Cargo.toml",
                        "why": f"unparseable manifest: {exc}"})
            continue
        name = (doc.get("package") or {}).get("name") or path.parent.name
        manifests[name] = (rel, text, list(_dep_tables(doc)))

    def add(crate, symbol, why):
        rel, text, _ = manifests[crate]
        line = next((i for i, row in enumerate(text.splitlines(), 1) if row.startswith(symbol.rsplit(" ", 1)[-1])), 1)
        if not any(v["path"] == rel and v["symbol"] == symbol for v in out):
            out.append({"path": rel, "line": line, "class": "cargo", "symbol": symbol, "why": why})

    for guarded in GUARDED_CRATES:
        seen, queue = {guarded}, [(guarded, [guarded])]
        while guarded in manifests and queue:
            crate, route = queue.pop(0)
            for dev, dep, feats in manifests[crate][2]:
                if dev:
                    continue
                if dep in BANNED_DEPS or (dep == "tokio" and "process" in feats):
                    via = "direct normal dependency" if crate == guarded else "reachable via " + " -> ".join(route)
                    add(crate, f"{guarded} -> {dep}", f"{guarded} may not depend on {dep}: {via}")
                elif dep in manifests and dep not in seen:
                    seen.add(dep)
                    queue.append((dep, route + [dep]))
    for crate, (rel, _, deps) in sorted(manifests.items()):
        if crate in GUARDED_CRATES:
            continue
        has = any(dep == "reqwest" for _, dep, _ in deps)
        if has and crate not in REQWEST_BASELINE:
            add(crate, f"{crate} -> reqwest", "exceeds the recorded reqwest baseline"
                if crate in REQWEST_OWNERS else "reqwest outside the native owner crates")
        elif not has and crate in REQWEST_BASELINE:
            stale.append(f"{rel}: baseline lists reqwest for {crate}, found none")
    return out, stale


def judge(hits, scanned, extra=(), extra_stale=()):
    """Apply the allowlist and ceilings to (path, line, class, symbol) hits."""
    groups = {}
    for hit in hits:
        groups.setdefault((hit[0], hit[2], hit[3]), []).append(hit)
    violations, errors, stale = [], [], []
    allowed = {}
    for key, rows in sorted(groups.items()):
        entry = ALLOWLIST.get(key)
        if entry and len(rows) <= entry["count"]:
            allowed[entry["phase"]] = allowed.get(entry["phase"], 0) + len(rows)
            continue
        why = f"exceeds recorded count {entry['count']}" if entry else "no allowlist entry"
        for path, line, cls, sym in rows:
            violations.append({"path": path, "line": line, "class": cls, "symbol": sym, "why": why})
    for key, entry in sorted(ALLOWLIST.items()):
        actual = len(groups.get(key, []))
        if actual < entry["count"]:
            stale.append(f"{key[0]} [{key[1]}] {key[2]}: recorded {entry['count']}, found {actual}")
    for phase, total in sorted(allowed.items()):
        ceiling = PHASE_CEILINGS.get(phase)
        if ceiling is None or total > ceiling:
            errors.append(f"phase {phase}: {total} allowlisted hits exceed ceiling {ceiling}")
    violations.extend(extra)
    stale.extend(extra_stale)
    by_class = {c: 0 for c in CLASSES}
    for v in violations:
        by_class[v["class"]] = by_class.get(v["class"], 0) + 1
    return {"files_scanned": scanned, "total_hits": len(hits), "violations": violations,
            "violations_by_class": by_class, "allowlisted_by_phase": allowed,
            "allowlisted_total": sum(allowed.values()), "stale": stale, "errors": errors,
            "ok": not violations and not errors}


def evaluate(root):
    rels = collect_files(root)
    hits = []
    for rel in rels:
        text = (root / rel).read_text(encoding="utf-8", errors="replace")
        hits.extend((rel, line, cls, sym) for line, cls, sym in analyze(rel, text))
    return judge(hits, len(rels), *cargo_check(root))


def _format(r):
    lines = [f"check-text-boundary: {r['files_scanned']} files scanned, {r['total_hits']} hits",
             "unaccounted violations by class:"]
    lines += [f"  {c}: {r['violations_by_class'].get(c, 0)}" for c in CLASSES]
    lines.append("allowlisted hits by phase:")
    lines += [f"  {ph}: {r['allowlisted_by_phase'].get(ph, 0)} (ceiling {PHASE_CEILINGS[ph]})"
              for ph in sorted(PHASE_CEILINGS)]
    lines += [f"allowlisted total: {r['allowlisted_total']}",
              f"unaccounted total: {len(r['violations'])}"]
    for v in r["violations"][:200]:
        lines.append(f"  VIOLATION {v['path']}:{v['line']} [{v['class']}] {v['symbol']} ({v['why']})")
    lines += [f"  ERROR {e}" for e in r["errors"]]
    lines += [f"  note: stale allowlist entry {s}" for s in r["stale"]]
    lines.append("RESULT: " + ("PASS" if r["ok"] else "FAIL"))
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description="Text-model egress and provider boundary guard.")
    parser.add_argument("--json", action="store_true", help="print the result as JSON")
    parser.add_argument("--root", default=str(Path(__file__).resolve().parent.parent),
                        help="repository root (default: parent of development/)")
    args = parser.parse_args(argv)
    root = Path(args.root)
    if not root.is_dir():
        print(f"check-text-boundary: no such directory: {root}", file=sys.stderr)
        return 2
    result = evaluate(root)
    # An allowlist entry that nothing matches any more is a leftover exemption, so it fails the run.
    result["ok"] = result["ok"] and not result["stale"]
    print(json.dumps(result, indent=2, sort_keys=True) if args.json else _format(result))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
