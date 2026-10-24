#!/usr/bin/env python3
"""Exercise the real launcher against owned fake tools; no server, database or provider."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parent
BASH = os.environ.get("DF_TEST_BASH", "bash")
ARTIFACTS = Path(os.environ.get("DUNGEONFLUX_TEST_ROOT", SOURCE.parent / "artifacts/tmp/launch-tests"))

class LaunchTests(unittest.TestCase):
    def setUp(self):
        ARTIFACTS.mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix="launch-", dir=ARTIFACTS)).resolve()
        (self.root / "development").mkdir()
        for name in ["run-gameplay.sh", "launch-settings.sh"]:
            shutil.copyfile(SOURCE / name, self.root / "development" / name)
        self.art = self.root / "artifacts"
        self.bin = self.art / "cache/cargo/bin"
        self.bin.mkdir(parents=True)
        self.pg = self.root / "pg"
        self.pg.mkdir()
        self.build = self.art / "build/play"
        (self.build / "debug").mkdir(parents=True)
        (self.build / "play-web").mkdir()
        (self.build / "play-web/df_tools.js").write_text("fixture")
        self.log = self.root / "commands.log"
        self.env = {k: v for k, v in os.environ.items() if not k.startswith(("DF_", "DUNGEONFLUX_"))}
        self.env.update(DUNGEONFLUX_ARTIFACT_ROOT=self.shell_path(self.art), DUNGEONFLUX_POSTGRES_BIN=self.shell_path(self.pg), LAUNCH_TEST_LOG=self.shell_path(self.log))
        self.env["PATH"] = str(self.bin) + os.pathsep + self.env["PATH"]
        self.tool(self.bin / "uname", "echo Linux")
        self.tool(self.bin / "lsof", 'case "$*" in *55518*) if test "${LAUNCH_TEST_ALREADY:-0}" = 1; then echo postgres; fi ;; esac')
        self.tool(self.bin / "cargo", 'printf "cargo %s\n" "$*" >> "$LAUNCH_TEST_LOG"')
        self.tool(self.bin / "wasm-bindgen", """if test "$1" = --version; then echo 'wasm-bindgen 0.2.129'; exit; fi
while test "$#" -gt 0; do if test "$1" = --out-dir; then shift; mkdir -p "$1"; echo fixture > "$1/df_tools.js"; fi; shift; done""")
        for name in ["initdb", "pg_ctl", "createdb", "psql", "pg_isready"]:
            self.tool(self.pg / name, f'printf "{name} %s\n" "$*" >> "$LAUNCH_TEST_LOG"; if test "${{LAUNCH_TEST_FAILURE:-}}" = {name}; then exit 8; fi')
        self.tool(self.build / "debug/df-transport-fixture", """if test "$1" = --campaign-run; then echo DATABASE=df_gameplay_demo_fixture; exit; fi
if test "${LAUNCH_TEST_FAILURE:-}" = server; then exit 7; fi
printf 'server port=%s\n' "$DF_GAMEPLAY_DATABASE_PORT" >> "$LAUNCH_TEST_LOG"
printf 0123456789abcdef.stop > "$DF_GAMEPLAY_STOP_FILE/target.receipt"
kill -TERM "$PPID"
for i in $(seq 1 100); do
 if test -f "$DF_GAMEPLAY_STOP_FILE/0123456789abcdef.stop"; then echo stopped >> "$LAUNCH_TEST_LOG"; exit; fi
 sleep .05
done
exit 9""")

    @staticmethod
    def shell_path(path):
        value = path.as_posix()
        return "/" + value[0].lower() + value[2:] if len(value) > 2 and value[1] == ":" else value

    def tool(self, path, body):
        path.write_text("#!/bin/sh\n" + body + "\n", encoding="utf-8", newline="\n")
        path.chmod(0o755)

    def run_launcher(self, *args, **overrides):
        if overrides.get("LAUNCH_TEST_ALREADY") == "1":
            pgdata = self.art / "tmp/play/pgdata"
            pgdata.mkdir(parents=True, exist_ok=True)
            port = overrides.get("LAUNCH_TEST_CLUSTER_PORT", "55518")
            (pgdata / "postmaster.pid").write_text(f"123\nfixture\n0\n{port}\n")
        return subprocess.run([BASH, str(self.root / "development/run-gameplay.sh"), *args], env=self.env | overrides, text=True, capture_output=True, timeout=15)

    def test_preflight_is_side_effect_free_and_resolves_aliases(self):
        for overrides, port in [({}, 55517), ({"DF_GAMEPLAY_DATABASE_PORT": "55518"}, 55518), ({"DF_GAMEPLAY_DEMO_DATABASE_PORT": "55519"}, 55519)]:
            result = self.run_launcher("--print-config", **overrides)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"database_port={port}", result.stdout)
            self.assertFalse(self.log.exists())
            self.assertFalse((self.art / "tmp/play").exists())

    def test_conflicting_or_invalid_ports_refuse_without_exposing_input(self):
        for overrides in [{"DF_GAMEPLAY_DATABASE_PORT": "private-secret"}, {"DF_GAMEPLAY_DEMO_DATABASE_PORT": "1023"}, {"DF_GAMEPLAY_DATABASE_PORT": "65536"}, {"DF_GAMEPLAY_DATABASE_PORT": "+55517"}, {"DF_GAMEPLAY_DATABASE_PORT": "55517 "}, {"DF_GAMEPLAY_DATABASE_PORT": "55518", "DF_GAMEPLAY_DEMO_DATABASE_PORT": "55519"}]:
            result = self.run_launcher("--print-config", **overrides)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("private-secret", result.stderr)
            self.assertFalse(self.log.exists())

    def test_campaign_build_once_port_propagation_and_exact_graceful_stop(self):
        other = self.art / "tmp/play/run/stop.other-run"
        other.mkdir(parents=True)
        (other / "target.receipt").write_text("1111111111111111.stop")
        result = self.run_launcher("--campaign", "fixture", DF_GAMEPLAY_DATABASE_PORT="55518")
        self.assertEqual(result.returncode, 143, result.stdout + result.stderr)
        log = self.log.read_text()
        self.assertEqual(sum("cargo " in line and "--bin df-transport-fixture" in line for line in log.splitlines()), 1, log)
        for tool in ["psql", "createdb"]:
            self.assertTrue(any(line.startswith(tool + " ") and "-p 55518" in line for line in log.splitlines()), log)
        self.assertIn("-p 55518 -h 127.0.0.1", log)
        self.assertIn("server port=55518", log)
        self.assertIn("stopped", log)
        self.assertFalse((other / "1111111111111111.stop").exists())
        self.assertEqual((other / "target.receipt").read_text(), "1111111111111111.stop")
        directories = [path for path in (self.art / "tmp/play/run").glob("stop.*") if path != other]
        self.assertEqual(len(directories), 1)
        self.assertTrue((directories[0] / "0123456789abcdef.stop").is_file())

    def test_owned_postgres_stops_after_database_or_server_failure(self):
        for failure in ["psql", "createdb", "server"]:
            if self.log.exists():
                self.log.unlink()
            result = self.run_launcher("--no-build", LAUNCH_TEST_FAILURE=failure)
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            log = self.log.read_text()
            self.assertTrue(any(line.startswith("pg_ctl ") and " stop" in line for line in log.splitlines()), log)

    def test_existing_postgres_is_never_stopped(self):
        result = self.run_launcher("--no-build", DF_GAMEPLAY_DATABASE_PORT="55518", LAUNCH_TEST_ALREADY="1", LAUNCH_TEST_FAILURE="server")
        self.assertEqual(result.returncode, 7, result.stdout + result.stderr)
        log = self.log.read_text()
        self.assertFalse(any(line.startswith("pg_ctl ") and " stop" in line for line in log.splitlines()), log)

    def test_running_cluster_on_another_port_does_not_admit_a_foreign_listener(self):
        result = self.run_launcher("--no-build", DF_GAMEPLAY_DATABASE_PORT="55518", LAUNCH_TEST_ALREADY="1", LAUNCH_TEST_CLUSTER_PORT="55517")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        log = self.log.read_text()
        self.assertNotIn("psql ", log)
        self.assertNotIn("createdb ", log)
        self.assertFalse(any(line.startswith("pg_ctl ") and " stop" in line for line in log.splitlines()), log)

    def test_dev_startup_skips_dotenv_and_refuses_unknown_guard(self):
        (self.root / ".env").write_text("OPENAI_API_KEY=private-secret\n")
        self.tool(self.bin / "sed", 'echo unexpected-dotenv-read >> "$LAUNCH_TEST_LOG"; exit 9')
        result = self.run_launcher("--no-build", DF_GAMEPLAY_LOAD_DOTENV="off", LAUNCH_TEST_FAILURE="server")
        self.assertEqual(result.returncode, 7, result.stdout + result.stderr)
        self.assertNotIn("unexpected-dotenv-read", self.log.read_text())
        result = self.run_launcher("--print-config", DF_GAMEPLAY_LOAD_DOTENV="typo")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("private-secret", result.stdout + result.stderr)

    def test_preflight_refuses_supplied_database_and_argument_conflicts(self):
        for args in [("--database", "private-secret"), ("--campaign", "fixture", "--new-game"), ("--status",), ("--campaign", "fixture", "--database", "df_gameplay_demo_ok")]:
            result = self.run_launcher("--print-config", *args)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("private-secret", result.stderr)
            self.assertFalse(self.log.exists())

if __name__ == "__main__":
    unittest.main()
