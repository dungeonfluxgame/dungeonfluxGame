# Offline table memory qualification

The runner measures one gameplay process per table: first one table, then five.
Each process has its own scratch PostgreSQL database and a real TV subscription
plus five phones on the generated gRPC-over-WebSocket bridge. Phones join, create
five heroes and submit server offers no faster than four-second intervals. Combat
beats finish in sequence using the shared renderer and reading durations; phone
input also waits for that simulated TV completion. Successful runs
require accepted gameplay receipts and successful signed silent-audio delivery.
External providers are disabled; the server preserves deterministic authored harbor text and its matching captions,
serves silent WAV for registered lines, and loads generated 32x32 placeholder art.
This measures the authored harbor scenario. It does not emulate live provider
latency/concurrency, real image sizes, video generation, or long campaign growth.

Use a matching generated gameplay web package from the same checkout, containing
`df_tools.js` and `df_tools_bg.wasm`. The RPC load client does not execute browser rendering. Its
web package is loaded by the server and hashed in provenance. For browser/WASM
build instructions use `development/tooling.md`; retain the exact package used.

Windows (the runner records OS and binary architecture; scratch PostgreSQL on port 55911):

```powershell
powershell -ExecutionPolicy Bypass -File deploy/scripts/load/run-load.ps1 -WebRoot <absolute-generated-web-directory> -OutDir artifacts/tmp/agents/LOADHARNESS/windows-run
```

Linux droplet later requires Python 3.11+, a populated Linux-native pinned Rust
toolchain and dependency caches at the paths exported by `run-load.sh`, and the
matching generated web package. Copying Windows toolchain/build caches does not
provide a Linux installation.

Use an already-running, isolated PostgreSQL 18 scratch cluster bound to loopback,
with the provided development trust-auth contract and a `postgres` role allowed
to CREATE and DROP the unique test databases. The runner uses password-free `-w`
connections, sanitizes credential overrides, and the native development server
has no password input. An ordinary password-auth Debian PostgreSQL installation
does not satisfy these prerequisites. Never target production data. This command
does not install, configure, start or stop PostgreSQL:

```sh
PSQL=/usr/bin/psql PGPORT=55911 bash deploy/scripts/load/run-load.sh /absolute/generated-web-directory artifacts/tmp/agents/LOADHARNESS/linux-run 600
```

The wrappers build once. To rerun a fixed already-built binary, use
`python[3] deploy/scripts/load/run_load.py --web-root <path> --out <fresh-path>`.
Use `--binary-dir` to select a different build directory. `--tables 1` or
`--tables 5` selects an individual phase; the default is both in sequence.

Only loopback ports 63411 through 63415 are used; the load client rejects 63309.
Databases have unique `df_gameplay_demo_loadharness_` names and are dropped in
cleanup. Fake mode keeps the complete hero library inside each owned scratch
database instead of opening the shared development library. Servers stop via their owned stop receipt; a bounded terminate/kill is
used only if that exact owned child does not exit. All subprocesses are reaped.
No PostgreSQL service is started, stopped or reconfigured.

Each phase retains `rss.csv` with one-second samples and phase labels, five
seconds of whole-host baseline, separate per-server RSS, aggregate server RSS,
whole-host physical memory used, `summary.json` with nearest-rank p95 and peak,
server/client logs, and `cleanup.json`. `provenance.json` records platform,
architecture and hashes of the frozen binaries and web package. Failed workload
runs fail nonzero and retain raw samples; they produce no sizing summary.
Windows RSS is WorkingSetSize, and Linux RSS is VmRSS. Summed per-process RSS can
count shared resident pages more than once. Whole-host use includes PostgreSQL,
load clients and unrelated applications; do not attribute its delta solely to
tables, and do not transfer Windows numbers directly to Linux.

Use the larger of the one-table p95 and the worst individual-table p95 in the
five-table run as the conservative per-table input. Compare that with aggregate
five-table peak/p95; add an explicit OS/PostgreSQL/service reserve and leave at
least 30% headroom when choosing a provisional RAM class. Confirm on the
actual Linux architecture with representative real media sizes before treating
that class as production capacity. No regional availability or pricing is implied.
