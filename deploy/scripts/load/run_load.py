"""Measure real per-table gameplay processes with an offline workload, on Windows or Linux."""

import argparse
import csv
import ctypes
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import signal
import socket
import subprocess
import time
import uuid


ROOT = Path(__file__).resolve().parents[3]


def validate_ports(database_port, base_port, tables):
    ports = [database_port] + [base_port + index for index in range(max(tables))]
    if any(port == 63309 or not 1024 <= port <= 65535 for port in ports):
        raise ValueError('database and game ports must be 1024..65535 excluding 63309')


def summarize(values):
    if not values:
        raise ValueError("memory sample set is empty")
    ordered = sorted(values)
    return {"samples": len(ordered), "peak_bytes": ordered[-1],
            "p95_bytes": ordered[math.ceil(len(ordered) * 0.95) - 1]}


def memory(processes):
    if os.name == "nt":
        from ctypes import wintypes

        class MemoryStatus(ctypes.Structure):
            _fields_ = [("length", wintypes.DWORD), ("load", wintypes.DWORD)] + [
                (name, ctypes.c_uint64) for name in ["total", "available", "page_total",
                    "page_available", "virtual_total", "virtual_available", "extended"]]

        class Counters(ctypes.Structure):
            _fields_ = [("length", wintypes.DWORD), ("faults", wintypes.DWORD)] + [
                (name, ctypes.c_size_t) for name in ["peak", "resident", "pool_peak",
                    "pool", "nonpool_peak", "nonpool", "page", "page_peak"]]

        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        psapi = ctypes.WinDLL("psapi", use_last_error=True)
        kernel.GlobalMemoryStatusEx.argtypes = [ctypes.POINTER(MemoryStatus)]
        kernel.GlobalMemoryStatusEx.restype = wintypes.BOOL
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel.CloseHandle.restype = wintypes.BOOL
        psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE,
                                              ctypes.POINTER(Counters), wintypes.DWORD]
        psapi.GetProcessMemoryInfo.restype = wintypes.BOOL
        status = MemoryStatus()
        status.length = ctypes.sizeof(status)
        if not kernel.GlobalMemoryStatusEx(ctypes.byref(status)):
            raise ctypes.WinError(ctypes.get_last_error())
        resident = []
        for process in processes:
            if process.poll() is not None:
                raise RuntimeError("server exited before memory sample")
            handle = kernel.OpenProcess(0x410, False, process.pid)
            if not handle:
                raise ctypes.WinError(ctypes.get_last_error())
            try:
                counters = Counters()
                counters.length = ctypes.sizeof(counters)
                if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.length):
                    raise ctypes.WinError(ctypes.get_last_error())
                resident.append(counters.resident)
            finally:
                kernel.CloseHandle(handle)
        return resident, status.total - status.available, status.total
    resident = []
    for process in processes:
        if process.poll() is not None:
            raise RuntimeError("server exited before memory sample")
        fields = Path(f"/proc/{process.pid}/status").read_text().splitlines()
        resident.append(next(int(line.split()[1]) * 1024 for line in fields
                             if line.startswith("VmRSS:")))
    fields = {line.split(':')[0]: int(line.split()[1]) * 1024
              for line in Path('/proc/meminfo').read_text().splitlines()}
    return resident, fields["MemTotal"] - fields["MemAvailable"], fields["MemTotal"]


def runtime_environment():
    # Credentials and developer feature overrides cannot leak into the measured processes.
    names = ["PATH", "SYSTEMROOT", "WINDIR", "TEMP", "TMP", "HOME", "USERPROFILE",
             "APPDATA", "LOCALAPPDATA", "LANG", "LD_LIBRARY_PATH"]
    return {name: os.environ[name] for name in names if name in os.environ}


def hash_file(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def binary_architecture(path):
    with path.open('rb') as stream:
        header = stream.read(64)
        if header[:2] == b'MZ':
            stream.seek(int.from_bytes(header[60:64], 'little') + 4)
            machine = int.from_bytes(stream.read(2), 'little')
            return {0xaa64: 'aarch64', 0x8664: 'x86_64', 0x14c: 'x86'}.get(machine, str(machine))
        if header[:4] == b'\x7fELF':
            machine = int.from_bytes(header[18:20], 'little')
            return {183: 'aarch64', 62: 'x86_64', 3: 'x86'}.get(machine, str(machine))
    raise ValueError('unrecognized executable architecture')


def database(psql, port, command):
    subprocess.run([str(psql), '-X', '-w', '-h', '127.0.0.1', '-p', str(port),
                    '-U', 'postgres', '-d', 'postgres', '-v', 'ON_ERROR_STOP=1', '-c', command],
                   check=True, timeout=30, env=runtime_environment(), stdout=subprocess.DEVNULL)


def stop_process(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=15)


def cleanup(settings, output, processes, clients, logs, databases, stops):
    errors = []
    dropped = []

    def attempt(label, action):
        try:
            action()
        except Exception as error:
            errors.append(f'{label}: {type(error).__name__}: {error}')

    def request_stop(stop):
        receipt = stop / 'target.receipt'
        if receipt.is_file():
            name = receipt.read_text().strip()
            if Path(name).name != name or not name.endswith('.stop'):
                raise RuntimeError('invalid owned stop receipt')
            (stop / name).touch()

    def wait_for_server(process):
        try:
            process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            stop_process(process)

    def drop_database(name):
        database(settings.psql, settings.database_port, f'DROP DATABASE "{name}" WITH (FORCE)')
        dropped.append(name)

    for client in clients:
        attempt(f'client {client.pid}', lambda client=client: stop_process(client))
    for stop in stops:
        attempt('stop receipt', lambda stop=stop: request_stop(stop))
    for process in processes:
        attempt(f'server {process.pid}', lambda process=process: wait_for_server(process))
    for log in logs:
        attempt('log close', log.close)
    for name in databases:
        attempt(f'database {name}', lambda name=name: drop_database(name))
    (output / 'cleanup.json').write_text(json.dumps({
        'servers': [process.returncode for process in processes],
        'server_pids': [process.pid for process in processes],
        'clients': [client.returncode for client in clients],
        'client_pids': [client.pid for client in clients],
        'dropped_databases': dropped, 'errors': errors}) + '\n')
    if errors:
        raise RuntimeError('owned-resource cleanup failed: ' + '; '.join(errors))


def phase(settings, tables, binaries):
    output = settings.out / f'tables-{tables}'
    output.mkdir(exist_ok=False)
    processes, clients, logs, databases, stops = [], [], [], [], []
    rows = []
    started = time.monotonic()
    started_unix_ms = round(time.time() * 1000)
    csv_path = output / 'rss.csv'
    with csv_path.open('w', newline='', encoding='ascii') as csv_file:
        columns = ['timestamp_ms', 'phase', 'server_rss_bytes', 'host_used_bytes',
                   'host_total_bytes'] + [f'server_{index}_rss_bytes' for index in range(tables)]
        writer = csv.DictWriter(csv_file, fieldnames=columns)
        writer.writeheader()

        def sample(label):
            resident, host_used, host_total = memory(processes)
            row = dict(timestamp_ms=round((time.monotonic() - started) * 1000), phase=label,
                       server_rss_bytes=sum(resident), host_used_bytes=host_used,
                       host_total_bytes=host_total)
            row.update({f'server_{index}_rss_bytes': value
                        for index, value in enumerate(resident)})
            writer.writerow(row)
            csv_file.flush()
            rows.append(row)

        try:
            for _ in range(5):
                sample('baseline')
                time.sleep(1)
            for index in range(tables):
                port = settings.base_port + index
                with socket.socket() as probe:
                    probe.bind(('127.0.0.1', port))
                name = f'df_gameplay_demo_loadharness_{uuid.uuid4().hex[:12]}_{index}'
                database(settings.psql, settings.database_port, f'CREATE DATABASE "{name}"')
                databases.append(name)
                environment = runtime_environment()
                stop = output / f'stop-{index}'
                stops.append(stop)
                environment.update(DF_FAKE_PROVIDERS='on', DF_GAMEPLAY_LISTEN=f'127.0.0.1:{port}',
                    DF_GAMEPLAY_DEMO_DATABASE=name, DF_GAMEPLAY_DATABASE_PORT=str(settings.database_port),
                    DF_GAMEPLAY_DATABASE_HOST='127.0.0.1', DF_GAMEPLAY_DATABASE_USER='postgres',
                    DF_GAMEPLAY_MAX_CONNECTIONS='12', DF_GAMEPLAY_MAX_WATCH_STREAMS='12',
                    DF_GAMEPLAY_STOP_FILE=str(stop), DF_GAMEPLAY_SESSION_SECONDS=str(settings.seconds + 180),
                    DF_PROFILE='development')
                log_path = output / f'server-{index}.log'
                log = log_path.open('wb')
                logs.append(log)
                process = subprocess.Popen([str(binaries[0]), '--gameplay-demo', str(settings.web_root),
                                            str(settings.out / 'assets/ui')], env=environment,
                    cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                    creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
                processes.append(process)
                deadline = time.monotonic() + 90
                while 'READY display=' not in log_path.read_text(errors='replace'):
                    if process.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError(f'server {index} startup failed; inspect {log_path}')
                    sample('startup')
                    time.sleep(1)
            for index in range(tables):
                log = (output / f'client-{index}.log').open('wb')
                logs.append(log)
                clients.append(subprocess.Popen([str(binaries[1]), '--url',
                    f'http://127.0.0.1:{settings.base_port + index}', '--seconds', str(settings.seconds),
                    '--pace-ms', str(settings.pace_ms)], cwd=ROOT, env=runtime_environment(),
                    stdout=log, stderr=subprocess.STDOUT,
                    creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0))
            deadline = time.monotonic() + settings.seconds + 120
            while any(client.poll() is None for client in clients):
                if time.monotonic() > deadline:
                    raise RuntimeError('load clients exceeded deadline')
                if any(client.poll() not in (None, 0) for client in clients):
                    raise RuntimeError('load client failed; inspect client logs')
                sample('active')
                time.sleep(1)
            if any(client.returncode != 0 for client in clients):
                raise RuntimeError('load client failed; inspect client logs')
            active = [row for row in rows if row['phase'] == 'active']
            summary = {'tables': tables, 'seconds': settings.seconds, 'pace_ms': settings.pace_ms,
                'started_unix_ms': started_unix_ms,
                'server_sum': summarize([row['server_rss_bytes'] for row in active]),
                'server_all_phases': summarize([row['server_rss_bytes'] for row in rows]),
                'host_used': summarize([row['host_used_bytes'] for row in active]),
                'host_baseline': summarize([row['host_used_bytes'] for row in rows if row['phase'] == 'baseline']),
                'servers': [summarize([row[f'server_{index}_rss_bytes'] for row in active])
                            for index in range(tables)],
                'pids': [process.pid for process in processes], 'completed_clients': tables}
            summary['per_table_p95_bytes'] = summary['server_sum']['p95_bytes'] / tables
        finally:
            cleanup(settings, output, processes, clients, logs, databases, stops)
        (output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
        print(json.dumps(summary), flush=True)


def request_shutdown(_signal, _frame):
    raise KeyboardInterrupt


def main():
    signal.signal(signal.SIGTERM, request_shutdown)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--web-root', type=Path, required=True)
    parser.add_argument('--binary-dir', type=Path, default=ROOT / 'artifacts/build/agents/debug')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--psql', type=Path, default=Path('psql'))
    parser.add_argument('--database-port', type=int, default=55911)
    parser.add_argument('--base-port', type=int, default=63411)
    parser.add_argument('--seconds', type=int, default=600)
    parser.add_argument('--pace-ms', type=int, default=4000)
    parser.add_argument('--tables', type=int, nargs='+', default=[1, 5])
    settings = parser.parse_args()
    if not 1 <= settings.seconds <= 3600 or not 1000 <= settings.pace_ms <= 60000:
        parser.error('duration/pace out of bounds')
    if any(not 1 <= count <= 5 for count in settings.tables):
        parser.error('tables must be 1..5')
    try:
        validate_ports(settings.database_port, settings.base_port, settings.tables)
    except ValueError as error:
        parser.error(str(error))
    settings.out = settings.out.resolve()
    settings.web_root = settings.web_root.resolve()
    settings.out.mkdir(parents=True, exist_ok=False)
    for name in ['df_tools.js', 'df_tools_bg.wasm']:
        if not (settings.web_root / name).is_file():
            parser.error('a generated gameplay web package is required')
    frozen_web = settings.out / 'web'
    frozen_web.mkdir()
    for name in ['df_tools.js', 'df_tools_bg.wasm']:
        shutil.copy2(settings.web_root / name, frozen_web / name)
    settings.web_root = frozen_web
    suffix = '.exe' if os.name == 'nt' else ''
    frozen = settings.out / 'bin'
    frozen.mkdir()
    binaries = []
    for relative in ['df-transport-fixture', 'examples/load_client']:
        source = settings.binary_dir / (relative + suffix)
        target = frozen / (source.name)
        shutil.copy2(source, target)
        binaries.append(target)
    subprocess.run([str(binaries[0]), '--prepare-load-assets', str(settings.out / 'assets')],
                   check=True, timeout=30, env=runtime_environment())
    source_paths = sorted(set(ROOT.glob('crates/**/*.rs')) | set(ROOT.glob('crates/**/Cargo.toml'))
                          | set(path for path in ROOT.glob('deploy/scripts/load/*') if path.is_file())
                          | {ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', ROOT / 'rust-toolchain.toml'})
    source_hashes = {str(path.relative_to(ROOT)).replace('\\', '/'): hash_file(path)
                     for path in source_paths}
    (settings.out / 'source-hashes.json').write_text(json.dumps(source_hashes, indent=2) + '\n')
    head = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=ROOT, check=True,
                          capture_output=True, text=True).stdout.strip()
    rustc = subprocess.run(['rustc', '-vV'], cwd=ROOT, capture_output=True, text=True, timeout=20)
    provenance = {'source_head': head,
                  'source_scope': 'shared checkout observed after binary copy; concurrent source edits may differ from the compiled input',
                  'rustc_version_verbose': rustc.stdout if rustc.returncode == 0 else 'unavailable',
                  'binary_architecture': {path.name: binary_architecture(path) for path in binaries},
                  'source_digest': hashlib.sha256(json.dumps(source_hashes, sort_keys=True).encode()).hexdigest(),
                  'system': platform.system(), 'machine': platform.machine(),
                  'platform': platform.platform(), 'python': platform.python_version(),
                  'binaries': {str(path.relative_to(settings.out)): hash_file(path) for path in binaries},
                  'web_package': {name: hash_file(settings.web_root / name)
                                  for name in ['df_tools.js', 'df_tools_bg.wasm']},
                  'memory_metric': 'WorkingSetSize' if os.name == 'nt' else 'VmRSS',
                  'host_metric': 'total physical - available physical',
                  'provider_profile': 'deterministic public text / silent WAV / placeholder pictures; external providers disabled'}
    (settings.out / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
    for count in settings.tables:
        phase(settings, count, binaries)


if __name__ == '__main__':
    main()
