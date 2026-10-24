#!/usr/bin/env python3
"""Own a disposable, provider-free local development game and reach real screens."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
import urllib.request
import webbrowser

ROOT = Path(__file__).resolve().parent.parent
SCREENS = ("join", "room", "creation", "opening", "dialogue", "combat", "complete", "ended")
ROLES = ("display", "player1", "player2")


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--list-screens", action="store_true")
    parser.add_argument("--screen", choices=SCREENS, default="creation")
    parser.add_argument("--role", choices=ROLES, default="player1")
    parser.add_argument("--no-build", action="store_true")
    parser.add_argument("--no-open", action="store_true")
    parser.add_argument("--creator-portraits", action="store_true", help="Enable Jev appearance routing and explicit creator portraits; requires inherited OPENAI_API_KEY and DF_JEV_API_KEY")
    parser.add_argument("--creator-alternate-bfl", action="store_true", help="Route supported mature creator appearances to direct FLUX 3 Image; requires --creator-portraits and inherited BFL_API_KEY")
    parser.add_argument("--print-config", action="store_true")
    parser.add_argument("--port", type=int, default=63329)
    parser.add_argument("--database-port", type=int, default=55527)
    parser.add_argument("--artifact-root", type=Path, default=ROOT / "artifacts")
    parser.add_argument("--build-root", type=Path)
    result = parser.parse_args()
    for port in (result.port, result.database_port):
        if not 1024 <= port <= 65535:
            parser.error("ports must be 1024–65535")
    if result.port == result.database_port:
        parser.error("game and PostgreSQL ports must differ")
    if not result.list_screens and ((result.screen == "room" and result.role != "display") or (result.screen == "join" and result.role == "display")):
        parser.error("room is the shared display lobby; use --role display. join is the unjoined player entry; use --role player1 or player2")
    return result


def shell_path(path):
    value = path.as_posix()
    return "/" + value[0].lower() + value[2:] if len(value) > 2 and value[1] == ":" else value


def native_path(value):
    if os.name == "nt" and re.match(r"^/[a-zA-Z]/", value):
        return Path(value[1] + ":" + value[2:])
    return Path(value)


def bash_path():
    configured = os.environ.get("DF_DEV_BASH")
    if configured:
        return configured
    if os.name == "nt":
        candidate = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Git/bin/bash.exe"
        if candidate.is_file():
            return str(candidate)
        raise RuntimeError("Git Bash is required; set DF_DEV_BASH to its bash.exe")
    found = shutil.which("bash")
    if found:
        return found
    raise RuntimeError("bash is required; set DF_DEV_BASH")


def preview_environment(options, run_root, build):
    # Drop inherited gameplay switches and all supported provider credentials.
    credentials = {"OPENAI_API_KEY", "BFL_API_KEY", "FAL_KEY", "DF_FAL_KEY", "ELEVENLABS_API_KEY", "DF_JEV_API_KEY", "WORLD_LABS_API_KEY", "WORLDLABS_API_KEY"}
    env = {key: value for key, value in os.environ.items()
           if key not in credentials and not key.startswith(("DF_GAMEPLAY_", "DF_DEV_", "DF_JEV_", "DUNGEONFLUX_"))}
    env.update({
        "DUNGEONFLUX_ARTIFACT_ROOT": shell_path(options.artifact_root.resolve()),
        "DUNGEONFLUX_BUILD_ROOT": shell_path(build),
        "DUNGEONFLUX_PLAY_ROOT": shell_path(run_root / "play"),
        "DUNGEONFLUX_TMP_ROOT": shell_path(run_root / "tmp"),
        "DF_GAMEPLAY_LOAD_DOTENV": "off",
        "DF_GAMEPLAY_DATABASE_PORT": str(options.database_port),
        "DF_GAMEPLAY_LISTEN": f"127.0.0.1:{options.port}",
        "DF_GAMEPLAY_DEV_CONTROL": "on",
        "DF_GAMEPLAY_DEV_OVERRIDES": "off",
        "DF_GAMEPLAY_STORYTELLER": "off", "DF_GAMEPLAY_VOICE": "off",
        "DF_GAMEPLAY_IMAGERY": "off", "DF_GAMEPLAY_PLATES": "off",
        "DF_GAMEPLAY_STT": "off", "DF_GAMEPLAY_MUSIC": "off",
        "DF_GAMEPLAY_HOME_VIDEO": "off", "DF_GAMEPLAY_SCENE_VIDEO": "off",
    })
    if getattr(options, "creator_portraits", False):
        key = os.environ.get("OPENAI_API_KEY", "")
        if not key:
            raise RuntimeError("--creator-portraits requires inherited OPENAI_API_KEY; dotenv is not loaded")
        env["DF_GAMEPLAY_CREATOR_OPENAI_KEY"] = key
        env["DF_GAMEPLAY_CREATOR_PORTRAITS"] = "on"
        jev_key = os.environ.get("DF_JEV_API_KEY", "")
        if not jev_key:
            raise RuntimeError("--creator-portraits requires inherited DF_JEV_API_KEY; dotenv is not loaded")
        env["DF_GAMEPLAY_CREATOR_JEV_API_KEY"] = jev_key
        if os.environ.get("DF_JEV_BASE_URL"):
            env["DF_GAMEPLAY_CREATOR_JEV_BASE_URL"] = os.environ["DF_JEV_BASE_URL"]
    if getattr(options, "creator_alternate_bfl", False):
        if not getattr(options, "creator_portraits", False):
            raise RuntimeError("--creator-alternate-bfl requires --creator-portraits")
        key = os.environ.get("BFL_API_KEY", "")
        if not key:
            raise RuntimeError("--creator-alternate-bfl requires inherited BFL_API_KEY; dotenv is not loaded")
        env["DF_GAMEPLAY_CREATOR_BFL_KEY"] = key
    if options.screen == "combat":
        env["DF_GAMEPLAY_TEST_FIGHT"] = "2"
    return env


def request(url, token, body):
    request = urllib.request.Request(url, data=json.dumps(body).encode(),
                                     headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=25) as response:
        return json.load(response)


def wait_ready(process, reader):
    deadline = time.monotonic() + 900
    control = None
    ready = False
    while time.monotonic() < deadline:
        line = reader.readline()
        match = re.search(r"DEV CONTROL on url=(\S+) token_file=(.*?) \(token never printed\)", line)
        if match:
            control = match.group(1), native_path(match.group(2))
        if line.startswith("READY "):
            ready = True
        if ready and control:
            return control
        if process.poll() is not None:
            raise RuntimeError(f"launcher exited with {process.returncode}; inspect the owned launch log")
        time.sleep(.05)
    raise RuntimeError("startup exceeded 15 minutes; inspect the owned launch log")


def stop_owned(process, run_root, server, token, options):
    if process.poll() is not None:
        return
    if server and token:
        try:
            request(server + "/dev/shutdown", token, {})
        except (OSError, ValueError):
            pass  # The exact local stop receipt remains the fallback.
    for receipt in (run_root / "play/run").glob("stop.*/target.receipt"):
        marker = receipt.read_text().strip()
        if re.fullmatch(r"[a-f0-9]{16}\.stop", marker):
            (receipt.parent / marker).touch()
    try:
        process.wait(timeout=35)
    except subprocess.TimeoutExpired:
        # Only this wrapper's launcher tree is targeted, never a port or process-name match.
        if os.name == "nt":
            subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], capture_output=True, check=False)
        else:
            process.terminate()
        process.wait(timeout=10)
        # PostgreSQL detaches from pg_ctl, so it may outlive the launcher tree.
        # Only this invocation's private data directory and matching port are eligible.
        pgdata = run_root / "play/pgdata"
        pid_file = pgdata / "postmaster.pid"
        if pid_file.is_file():
            lines = pid_file.read_text().splitlines()
            if len(lines) >= 4 and lines[3] == str(options.database_port):
                pg_ctl = options.artifact_root / "build/postgres-g05/install/bin" / ("pg_ctl.exe" if os.name == "nt" else "pg_ctl")
                subprocess.run([str(pg_ctl), "-D", str(pgdata), "-m", "fast", "-w", "stop"], capture_output=True, timeout=15, check=False)
        raise RuntimeError("owned launcher did not stop gracefully; forced its process tree closed")


def main():
    options = arguments()
    if options.list_screens:
        print("join: player1/player2, unjoined browser entry")
        print("room: display, shared room lobby")
        print("creation/opening/dialogue/combat/complete: display/player1/player2, actual phases")
        print("ended: display/player1/player2, actual adventure-ended flag")
        print("Actual server phases reached through validated play; later phases may refuse or time out.")
        print("Desktop sign-in, campaign questionnaire and new appearance designer are not implemented screens.")
        return 0
    build = (options.build_root or options.artifact_root / "build/dev-screens").resolve()
    if options.print_config:
        print(json.dumps({"screen": options.screen, "role": options.role, "listen": f"127.0.0.1:{options.port}",
                          "database_port": options.database_port, "providers": "explicit creator portrait only" if options.creator_portraits else "off", "dotenv": "off",
                          "fresh_database": True, "build": not options.no_build, "build_root": str(build)}, indent=2))
        return 0
    run_root = options.artifact_root.resolve() / "tmp/dev-screens" / f"{time.time_ns()}-{os.getpid()}"
    run_root.mkdir(parents=True)
    env = preview_environment(options, run_root, build)
    command = [bash_path(), str(ROOT / "development/run-gameplay.sh"), "--harbor", "--new-game"]
    if options.no_build:
        command.append("--no-build")
    server = token = None
    print(f"Owned run: {run_root}", flush=True)
    print(f"Source: {ROOT}; screen={options.screen}; role={options.role}; providers={'creator portrait only' if options.creator_portraits else 'off'}", flush=True)
    with (run_root / "launch.log").open("w", encoding="utf-8") as log:
        process = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                                   creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        try:
            with (run_root / "launch.log").open(encoding="utf-8", errors="replace") as reader:
                server, token_file = wait_ready(process, reader)
            token = token_file.read_text().strip()
            executable = build / "debug" / ("df-transport-fixture.exe" if os.name == "nt" else "df-transport-fixture")
            if options.screen not in ("join", "room"):
                driven = subprocess.run([str(executable), "dev", "--server", server, "--token-file", str(token_file),
                                         "--timeout", "180", "play", "--to", options.screen], env=env,
                                        capture_output=True, text=True, timeout=190)
                (run_root / "phase.json").write_text(driven.stdout or driven.stderr, encoding="utf-8")
                if driven.returncode != 0:
                    raise RuntimeError(f"could not reach {options.screen}; inspect phase.json (no phase was fabricated)")
            if options.role == "display":
                url = server + "/gameplay/display"
            elif options.screen in ("join", "room"):
                url = server + "/gameplay/player"
            else:
                issued = request(server + "/dev/screens/ticket", token, {"seat": options.role[-1]})
                url = server + issued["path"]
            print(f"Preview: {url}", flush=True)
            print("Player links are one-use and expire in 60 seconds. Ctrl-C stops this owned run.", flush=True)
            if not options.no_open:
                if not webbrowser.open(url):
                    print("Browser could not open automatically; open the Preview URL above.", flush=True)
            return process.wait()
        finally:
            stop_owned(process, run_root, server, token, options)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
