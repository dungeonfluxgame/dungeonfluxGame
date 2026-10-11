# Development plans and durable operational data

`workflow.sqlite3` contains the source-backed whole-project planning corpus using
five tables: features, tasks, dependencies, attempts and append-only devlog.
These are plans and review evidence; no Rust game or automated coordinator exists.
`quality.sqlite3` is the separate installed quality skill's hash/review/finding
cache, not an extra workflow table or a runtime telemetry database.

Preserve all databases, sidecars, backups and retained `evidence/`. They are outside
`artifacts/` and never eligible for stale-build cleanup. Live SQLite files are
local-only; scoped runners/query interfaces are future work. Back up a live WAL
DB with SQLite backup or a consistent snapshot, never just copy the main file
while writes are active.

Source files to track once Git exists:

- `schema.sql`: constraints, indexes, views and schema version.
- `backlog-catalog.json`: explicit per-case owners and expectations across all140 families.
- `expand-backlog.py`: deterministic final backlog refinement; candidate output only.
- `validate-plan-seed.py`: source/classification/payload/DAG preflight before queue or schema mutation.
- `plan-manifest.json`: deterministic plan IDs, complete briefs/models/source
  sections and fingerprints, dependencies and planned RPC service inventory.
- `refine-gap-plans.py`: coordinator-only idempotent source/brief refinement over
  the existing manifest; requires an explicit output path and never writes queue state.
- `operational-tasks.json`: separately authorized operational TODOs merged into
  fresh provisioning, including the blocked GoDaddy domain task.
- `queries.sql`: bounded read-only status/brief/review queries.
- `verify-plans.py`: read-only plan/source/coverage/DAG verification, with negative
  SQL probes on an isolated in-memory backup and retained verification evidence.
- `provision.py`: one-shot stdlib SQLite data provisioning/inspection tool.
  This is development tooling, not the application or Rust agent runner.
- `devlog.py`: parameterized append-only entry intake using a coordinator-supplied
  execution context. It validates task/attempt and immutable retry identity, with
  no queue state mutation. The context file is a local role contract; trusted
  runner identity/access binding remains future work.
- `evidence/planning-refinement-audit.json`: preserved actual critic attempts and
  append-only refinement observations, separate from reproducible planned rows.

The final `workflow.snapshot.sqlite3` was created using SQLite's backup API.
Refresh consistent backups as operational history grows; seeded plans alone do
not restore later attempt/devlog history.

From the project directory, run:

```sh
python3 development/provision.py --check-only
python3 development/verify-plans.py
sqlite3 -readonly development/workflow.sqlite3 < development/queries.sql
```

To restore/seed a database from its manifest, the trusted coordinator runs
`python3 development/provision.py`. IDs are upserted; existing queue status,
created timestamps, attempts and devlog are retained. Active plan ownership blocks
unsafe seed changes. New source assumptions require regenerated reviewed manifest
inputs, not a blind rerun of stale data. The bootstrap v1-to-v2 migration preserves
all original rows plus a durable backup and refuses active/history-bearing queues;
future schema migrations need explicit review. This tool never dispatches work.

A dependency-ready blueprint is not dispatch-ready. Freeze the next bounded task's
scope, paths, actual checks, contracts, limits and required tool capabilities before
claiming it. Feature/slice expansions share one canonical implementation child;
slice composition owns wiring only. See `../planning/plan-database.md`.

## Minute screenshot journal

See [Screenshot journey](screenshot-journey.md) for the working public-preview
recorder, active start/renew/stop lifecycle and heartbeat recovery. Actual images,
source/time/task metadata and devlog payloads are protected in `evidence/journey/`.
The first baseline captures the static project site, not a playable game. The
user-facing timeline lives at `../../outputs/dungeonflux-visual-journey.html`.
Keep browser caches/profiles in `../artifacts/tmp/journey/`, preserve retained
history, and stop the recorder at the end of an active development run.

## Detailed backlog seed pipeline

The full-project catalogue is durable and source-hashed. The standard
`refine-gap-plans.py` automatically applies `expand-backlog.py` last when the
catalogue exists; older refinement cannot silently reduce the reviewed queue to
coarse plans. Use explicit candidate outputs and independently review them before
the coordinator replaces seed files and runs provisioning.

```sh
python3 development/refine-gap-plans.py --output artifacts/tmp/plan-candidate.json --operational-output artifacts/tmp/operational-candidate.json
```

For a staging directory containing the reviewed manifest, operational manifest,
schema and a SQLite-backup copy of the baseline, use
`python3 development/provision.py --directory <stage> --root <project>` and
`python3 development/verify-plans.py --directory <stage> --root <project> --output <retained-proof.json>`.
These commands do not imply approval to mutate the authoritative queue. Preserve
all baseline execution fields and history, plus intake dependencies outside the
seed. No planning seed enables dispatch. Only frozen `atomic`/`operational` roles
can enter the worker selection view; aggregates use a separate coordinator-owned
evidence-review attempt under the same integration triggers.

## Play the game locally

Run `development/run-gameplay.sh` to build and start the local game server and
PostgreSQL. Open the `display` URL from the server's `READY` line, then open the
`player` URL in two browser tabs to play both seats. Press Ctrl-C to stop the
server; the launcher also stops PostgreSQL when it started it. To resume a saved
game, use `development/run-gameplay.sh --no-build --database <name>` with the
database name printed by the launcher. For LAN access, set the listen address
and allowed origin to your machine's IP:

```sh
DF_GAMEPLAY_LISTEN=0.0.0.0:63309 DF_GAMEPLAY_ALLOWED_ORIGINS=http://<this-machine-ip>:63309 development/run-gameplay.sh
```
