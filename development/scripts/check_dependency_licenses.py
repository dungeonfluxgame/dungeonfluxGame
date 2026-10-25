#!/usr/bin/env python3
"""Generate offline Cargo dependency licence notices and check the reviewed inventory."""
import argparse, csv, hashlib, json, os, re, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "development" / "third-party"
INV = OUT / "inventory.csv"
ALLOWED = {"MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib"}
PREF = ("MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "ISC", "Zlib")
PROFILES = (
    ("df-server-aarch64-pc-windows-msvc", "df-server", "aarch64-pc-windows-msvc"),
    ("df-runner-aarch64-pc-windows-msvc", "df-runner", "aarch64-pc-windows-msvc"),
    ("df-tools-aarch64-pc-windows-msvc", "df-tools", "aarch64-pc-windows-msvc"),
    ("df-account-app-wasm32-unknown-unknown", "df-account-app", "wasm32-unknown-unknown"),
    ("df-tools-tv-phone-wasm32-unknown-unknown", "df-tools", "wasm32-unknown-unknown"),
)
FIELDS = ("profile", "name", "version", "source", "declared_spdx", "selected_branch", "selection_reason", "unreviewed_spdx_terms", "resolved_features", "audit_status", "license_text_paths", "license_text_sha256", "notice_paths", "notice_sha256")
TOKEN = re.compile(r"\s*(AND|OR|WITH|\(|\)|[A-Za-z0-9][A-Za-z0-9.+-]*)")


def parse_spdx(text):
    tokens, pos = [], 0
    while pos < len(text):
        m = TOKEN.match(text, pos)
        if not m: raise ValueError("unknown syntax")
        tokens.append(m.group(1)); pos = m.end()
    i = 0
    def primary():
        nonlocal i
        if i >= len(tokens): raise ValueError("incomplete")
        if tokens[i] == "(":
            i += 1; node = disjunction()
            if i >= len(tokens) or tokens[i] != ")": raise ValueError("unbalanced")
            i += 1; return node
        value = tokens[i]
        if value in {"AND", "OR", "WITH", ")"}: raise ValueError("operator")
        i += 1
        if i < len(tokens) and tokens[i] == "WITH":
            i += 1
            if i >= len(tokens) or tokens[i] in {"AND", "OR", "WITH", "(", ")"}: raise ValueError("exception")
            value = ("WITH", value, tokens[i]); i += 1
        return value
    def conjunction():
        nonlocal i
        nodes = [primary()]
        while i < len(tokens) and tokens[i] == "AND": i += 1; nodes.append(primary())
        return nodes[0] if len(nodes) == 1 else ("AND", *nodes)
    def disjunction():
        nonlocal i
        nodes = [conjunction()]
        while i < len(tokens) and tokens[i] == "OR": i += 1; nodes.append(conjunction())
        return nodes[0] if len(nodes) == 1 else ("OR", *nodes)
    if not tokens: raise ValueError("empty")
    result = disjunction()
    if i != len(tokens): raise ValueError("trailing tokens")
    return result


def choose(node):
    if isinstance(node, str): return (node, node) if node in ALLOWED else (None, "")
    if node[0] == "WITH": return None, ""
    options = [choose(child) for child in node[1:]]
    if node[0] == "AND":
        if any(item[0] is None for item in options): return None, ""
        return " AND ".join(item[0] for item in options), " AND ".join(item[1] for item in options)
    options = [item for item in options if item[0] is not None]
    if not options: return None, ""
    return min(options, key=lambda item: min((PREF.index(x) for x in item[0].split(" AND ")), default=len(PREF)))


def identifiers(node):
    if isinstance(node, str): return {node}
    if node[0] == "WITH": return {node[1], node[2]}
    return set().union(*(identifiers(child) for child in node[1:]))


def cargo(*args):
    env = os.environ.copy()
    env.update(CARGO_TARGET_DIR="C:/Users/mreca/Desktop/dungeonfluxGame/artifacts/build/agents", RUSTUP_HOME="C:/Users/mreca/Desktop/dungeonfluxGame/artifacts/cache/rustup", CARGO_HOME="C:/Users/mreca/Desktop/dungeonfluxGame/artifacts/cache/cargo", CARGO_BUILD_JOBS="2")
    result = subprocess.run(["cargo", *args], cwd=ROOT, env=env, text=True, capture_output=True)
    if result.returncode: raise RuntimeError(result.stderr.strip() or result.stdout.strip())
    return result.stdout


def texts_for(package, selected):
    root = Path(package["manifest_path"]).parent
    found = []
    for license_id in selected.split(" AND ") if selected else ():
        names = {"MIT": ("LICENSE-MIT", "MIT-LICENSE", "LICENSE"), "Apache-2.0": ("LICENSE-APACHE", "LICENSE"), "BSD-2-Clause": ("LICENSE", "LICENSE-BSD"), "BSD-3-Clause": ("LICENSE", "LICENSE-BSD"), "ISC": ("LICENSE", "LICENSE-ISC"), "Zlib": ("LICENSE", "LICENSE-ZLIB")}[license_id]
        path = next((root / name for name in names if (root / name).is_file()), None)
        if path and path not in found: found.append(path)
    if not found and root.is_dir():
        found = sorted(path for path in root.iterdir() if path.is_file() and re.match(r"(?i)^(license|copying)([-_.].*)?$", path.name))
    notices = sorted(path for path in root.iterdir() if path.is_file() and path.name.upper().startswith("NOTICE")) if root.is_dir() else []
    digest = ";".join(hashlib.sha256(path.read_bytes()).hexdigest() for path in found)
    notice_digest = ";".join(hashlib.sha256(path.read_bytes()).hexdigest() for path in notices)
    return found, notices, digest, notice_digest


def collect():
    rows = {}
    evidence = ROOT / "artifacts" / "tmp" / "lanes" / "LICAUDIT"
    for profile, root_package, target in PROFILES:
        metadata = json.loads(cargo("metadata", "--offline", "--format-version", "1", "--filter-platform", target))
        package_map = {item["id"]: item for item in metadata["packages"]}
        tree = cargo("tree", "--offline", "-e", "normal,features", "-p", root_package, "--target", target)
        evidence.mkdir(parents=True, exist_ok=True)
        with (evidence / f"tree-{profile}-features.txt").open("w", encoding="utf-8", newline="\n") as stream: stream.write(tree)
        active = {}
        for name, version in re.findall(r"([A-Za-z0-9_-]+) v(\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?)", tree):
            item = next((pkg for pkg in package_map.values() if pkg["name"] == name and pkg["version"] == version), None)
            if item: active[item["id"]] = item
        for item in active.values():
            declared = item.get("license") or "MISSING"
            try:
                expression_tree = parse_spdx(declared)
                selected, _ = choose(expression_tree)
                unreviewed_terms = sorted(identifiers(expression_tree) - ALLOWED)
            except ValueError:
                selected, unreviewed_terms = None, []
            found, notices, digest, notice_digest = texts_for(item, selected)
            status = "MISSING_DECLARATION" if declared == "MISSING" else ("HOLD_UNPARSED_OR_OUTSIDE_ALLOWLIST" if selected is None else ("MISSING_TEXT" if not found else ("ALLOWLIST_CANDIDATE_WITH_UNREVIEWED_TERMS" if unreviewed_terms else "ALLOWLIST_CANDIDATE")))
            if selected is None:
                reason = "No allowed branch selected; hold for SPDX/source review."
            elif " OR " in declared:
                reason = "Selected first allowed branch by policy priority: MIT, Apache-2.0, BSD-3-Clause, BSD-2-Clause, ISC, Zlib; owner review remains required."
            elif " AND " in declared:
                reason = "Recorded all cumulative allowed terms; every term and obligation needs owner review."
            else:
                reason = "Single allow-list identifier; exact text and obligations still need owner review."
            features = sorted({feature for name, feature in re.findall(r'([A-Za-z0-9_-]+) feature "([^"]+)"', tree) if name == item["name"]})
            source = item.get("source") or Path(item["manifest_path"]).parent.relative_to(ROOT).as_posix()
            row = {"profile": profile, "name": item["name"], "version": item["version"], "source": source, "declared_spdx": declared, "selected_branch": selected or "", "selection_reason": reason, "unreviewed_spdx_terms": ";".join(unreviewed_terms), "resolved_features": ";".join(features), "audit_status": status, "license_text_paths": ";".join(str(path) for path in found), "license_text_sha256": digest, "notice_paths": ";".join(str(path) for path in notices), "notice_sha256": notice_digest}
            rows[(profile, item["name"], item["version"])] = row
    return list(rows.values())


def save(rows):
    OUT.mkdir(parents=True, exist_ok=True)
    with INV.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=FIELDS, lineterminator="\n"); writer.writeheader(); writer.writerows(sorted(rows, key=lambda r: (r["profile"], r["name"], r["version"])))
    for profile, _, _ in PROFILES:
        groups = {}
        for row in rows:
            if row["profile"] != profile: continue
            for text_path in row["license_text_paths"].split(";") if row["license_text_paths"] else ():
                path = Path(text_path); digest = hashlib.sha256(path.read_bytes()).hexdigest()
                groups.setdefault(digest, {"path": path, "packages": set()})["packages"].add(f"{row['name']} {row['version']}")
        lines = [f"Third-party licence texts for profile: {profile}", "Generated from cargo metadata/tree. Text presence is not legal approval.", "Package audit status (missing texts and held declarations remain unresolved):"]
        for row in sorted((row for row in rows if row["profile"] == profile), key=lambda row: (row["name"], row["version"])):
            selected = row["selected_branch"] or "none"
            lines.append(f"- {row['name']} {row['version']}: declared {row['declared_spdx']}; selected {selected}; {row['audit_status']}")
        lines.append("")
        for index, (digest, group) in enumerate(sorted(groups.items()), 1):
            lines += [f"[{index}] SHA-256 {digest}", "Packages: " + ", ".join(sorted(group["packages"])), f"Source text: {group['path']}", "", group["path"].read_text(encoding="utf-8", errors="replace").rstrip(), "", "=" * 72, ""]
        for row in rows:
            if row["profile"] == profile and row["notice_paths"]:
                for notice in row["notice_paths"].split(";"):
                    path = Path(notice); lines += [f"NOTICE: {row['name']} {row['version']} ({path})", path.read_text(encoding="utf-8", errors="replace").rstrip(), "", "=" * 72, ""]
        with (OUT / f"THIRD_PARTY_NOTICES-{profile}.txt").open("w", encoding="utf-8", newline="\n") as stream: stream.write("\n".join(lines))


def check(rows):
    if not INV.is_file(): print(f"Missing reviewed inventory: {INV}", file=sys.stderr); return 1
    with INV.open(newline="", encoding="utf-8") as stream: baseline = {(r["profile"], r["name"], r["version"]): r for r in csv.DictReader(stream)}
    current = {(r["profile"], r["name"], r["version"]): r for r in rows}
    errors = []
    for key, row in current.items():
        old = baseline.get(key)
        if old is None:
            reason = "outside allow-list" if row["audit_status"] == "HOLD_UNPARSED_OR_OUTSIDE_ALLOWLIST" else "requires inventory review (" + row["audit_status"] + ")"
            errors.append(f"new dependency {reason}: {key}: {row['declared_spdx']}")
        elif any(old[field] != row[field] for field in ("source", "declared_spdx", "selected_branch", "selection_reason", "unreviewed_spdx_terms", "resolved_features", "audit_status", "license_text_sha256", "notice_sha256")):
            errors.append(f"licence record changed; review inventory: {key}: {row['declared_spdx']}")
    errors.extend(f"stale inventory entry: {key}" for key in baseline.keys() - current.keys())
    if errors: print("Dependency licence check failed:\n" + "\n".join("- " + e for e in sorted(errors)), file=sys.stderr); return 1
    print(f"Dependency licence check passed: {len(current)} profile/package records match inventory."); return 0


def main():
    parser = argparse.ArgumentParser(); parser.add_argument("--generate", action="store_true", help="write reviewed inventory and notices"); args = parser.parse_args()
    try:
        rows = collect()
        if args.generate: save(rows); print(f"Generated {len(rows)} profile/package records."); return 0
        return check(rows)
    except (OSError, RuntimeError, ValueError) as error: print(f"dependency licence audit failed: {error}", file=sys.stderr); return 2

if __name__ == "__main__": raise SystemExit(main())
