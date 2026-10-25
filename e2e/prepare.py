"""Prepare isolated local data and rendered sites; never starts a game server or provider."""

import argparse
import hashlib
import json
import secrets
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
AREA = ROOT / "artifacts/tmp/agents/FLOWMAP"
PSQL = ROOT / "artifacts/build/postgres-g05/install/bin/psql.exe"
BASH = Path("C:/Program Files/Git/usr/bin/bash.exe")


def sql(statement):
    return subprocess.run([str(PSQL), "-X", "-v", "ON_ERROR_STOP=1", "-h", "127.0.0.1",
                           "-p", "55911", "-U", "postgres", "-d", "postgres", "-Atc", statement],
                          cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()


def source_manifest():
    files = [ROOT / "Cargo.lock", ROOT / "rust-toolchain.toml", ROOT / "content/customer-prices.json"]
    for directory in ("crates/df-hosted", "crates/df-portal", "crates/df-account-app",
                      "crates/df-customer", "crates/df-campaign", "crates/df-finance", "crates/df-persistence"):
        files.extend(path for path in (ROOT / directory).rglob("*")
                     if path.is_file() and path.suffix in (".rs", ".toml", ".sql", ".css", ".html"))
    return {str(path.relative_to(ROOT)).replace("\\", "/"): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(files)}


def render_sites():
    business = AREA / "business.env"
    business.write_text("\n".join((
        "DF_LEGAL_ENTITY_NAME=FLOWMAP synthetic seller", "DF_LEGAL_ENTITY_KIND=test fixture",
        "DF_TRADING_NAME=DungeonFlux", "DF_SUPPORT_EMAIL=support@example.test",
        "DF_BILLING_EMAIL=billing@example.test", "DF_PRIVACY_EMAIL=privacy@example.test",
        "DF_SECURITY_EMAIL=security@example.test", "DF_STATEMENT_DESCRIPTOR=FLOWMAP TEST",
        "DF_SELLER_PUBLISHED=false")) + "\n", encoding="ascii", newline="\n")
    for mode in ("open", "closed"):
        target = AREA / f"site-{mode}"
        shutil.copytree(ROOT / "docs", target, dirs_exist_ok=True)
        import os
        env = dict(os.environ, DF_PILOT_OPEN=str(mode == "open").lower(),
                   DF_ACCOUNTS_OPEN=str(mode == "open").lower())
        result = subprocess.run([str(BASH), "deploy/scripts/render-site.sh",
                                 target.relative_to(ROOT).as_posix(), business.relative_to(ROOT).as_posix()],
                                cwd=ROOT, env=env,
                                capture_output=True, text=True)
        (AREA / f"render-{mode}.txt").write_text(result.stdout + result.stderr, encoding="utf-8", newline="\n")
        result.check_returncode()


def diagnostic_bundle():
    target = AREA / "bundle"
    assets = target / "assets"
    assets.mkdir(parents=True, exist_ok=True)
    bindgen = ROOT / "artifacts/cache/cargo/bin/wasm-bindgen.exe"
    wasm = ROOT / "artifacts/build/agents/wasm32-unknown-unknown/release/df_account_app.wasm"
    subprocess.run([str(bindgen), "--target", "web", "--no-typescript", "--out-name", "account_app",
                    "--out-dir", str(assets), str(wasm)], cwd=ROOT, check=True)
    app = ROOT / "crates/df-account-app/web"
    shutil.copy2(app / "index.html", target / "index.html")
    shutil.copy2(app / "google-g.svg", assets / "google-g.svg")
    shutil.copy2(ROOT / "docs/assets/establishing-harbor-canal-rowboat-bridge.webp", assets / "landing.webp")
    (assets / "app.css").write_bytes(b"".join((app / f"css/{name}.css").read_bytes()
                                             for name in ("base", "customer", "campaign", "admin")))
    (assets / "boot.js").write_text('import init from "/app/assets/account_app.js";\n'
                                    'init({ module_or_path: "/app/assets/account_app_bg.wasm" });\n',
                                    encoding="ascii", newline="\n")
    return {str(path.relative_to(AREA)).replace("\\", "/"): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in target.rglob("*") if path.is_file()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "cleanup", "manifest"))
    args = parser.parse_args()
    AREA.mkdir(parents=True, exist_ok=True)
    state_file = AREA / "stack.json"
    if args.action == "manifest":
        (AREA / "source-current.json").write_text(json.dumps(source_manifest(), indent=2) + "\n")
        return
    if args.action == "cleanup":
        state = json.loads(state_file.read_text())
        database = state["database"]
        if not database.startswith("flowmap_") or not database.replace("_", "").isalnum():
            raise ValueError("FLOW-SAFETY: database is outside this lane")
        # DROP refuses active connections. Never force out another process or touch shared PG settings.
        sql(f"DROP DATABASE IF EXISTS {database}")
        (AREA / "cleanup.txt").write_text(f"Dropped owned database {database}; no forced disconnects.\n")
        return
    if state_file.exists():
        raise FileExistsError("Existing stack state must be reviewed/cleaned before preparing another run")
    render_sites()
    bundle_hashes = diagnostic_bundle()
    database = "flowmap_" + secrets.token_hex(8)
    sql(f"CREATE DATABASE {database}")
    state = {"database": database, "origin": "http://127.0.0.1:49271",
             "site_open": "http://127.0.0.1:49272", "site_closed": "http://127.0.0.1:49273",
             "bundle_note": "Diagnostic generated bundle only; build-web release size gate failed. Not deployable.",
             "bundle_hashes": bundle_hashes, "source": source_manifest()}
    state_file.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8", newline="\n")
    print("Prepared local-only fixture; stack.json contains its non-secret database name and source hashes.")


if __name__ == "__main__":
    main()
