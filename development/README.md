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
database name printed by the launcher.

Edit `content/harbor-journey/tuning.json` to tune hero AC, proficiency, base HP,
ability arrays/background bonuses, Second Wind and weapon dice; bandit stats,
attack/damage and weapon dice; critical threshold, round ticks, combat log depth,
Short Rest seconds and narrative intervention budget. `pack.json` supplies scene
text, offer labels, creation options, equipment, room code and the private courier clue.
Restart to apply changes; re-run without `--database` for a fresh game.
`DF_GAMEPLAY_PACK_DIR` selects another directory containing both required files.
The startup line prints both file revisions; departures from SRD values show
`CUSTOM RULES` there and a `+custom-<revision>` suffix on the in-game rules label.
Saved games refuse changed revisions; start a new database after editing either file.

For LAN access, set the listen address
and allowed origin to your machine's IP:

```sh
DF_GAMEPLAY_LISTEN=0.0.0.0:63309 DF_GAMEPLAY_ALLOWED_ORIGINS=http://<this-machine-ip>:63309 development/run-gameplay.sh
```

### Content rating (mature by default)

This is an adult game, so the written and spoken content is `mature` unless you switch it:
`DF_GAMEPLAY_CONTENT_RATING=mature` (the default when unset) or `restrained`. Any other value
refuses startup. The startup line reads `CONTENT rating=mature` or `CONTENT rating=restrained`.
`run-gameplay.sh` passes the variable through when you set it; it sets nothing itself.

```sh
DF_GAMEPLAY_CONTENT_RATING=restrained development/run-gameplay.sh
```

- `mature`: the Dungeon Master runs a table of adults. Violence can be bloody and its
  consequences described, horror can be horrifying, villains can be cruel, interrogation and
  torture may appear as dark fiction, characters may swear when it fits them, and romance and
  seduction are played as adult scenes (an NPC still answers by its own temperament: a devout or
  uninterested one declines, an amused one plays along, an interested one reciprocates). The
  fight, victory and defeat prose, the speaking antagonists' lines, and the NPC flirting, threat
  and curt reactions come from the mature tables (`content/adventure-tables/mature-overlay.json`).
- `restrained`: the earlier behaviour, unchanged. Tense heroic fantasy, no gore, no strong
  language, nothing explicit; the shipped tables are used as written and a seed produces the same
  bytes it always did.

What neither mode changes: no sexual content involving minors or any character presented as
underage (the tables' `young` characters are young adults, and the model is told so); no sexual
content about real people (a character flagged as based on a real person's likeness is excluded);
what players say is untrusted input in both modes; all secret-safety rules (a model reply that
names a secret term is refused in both); the game rules and mechanics; and the text model's own
policy. A refusal or empty reply falls back to the prepared in-character line and is never
retried with reworded instructions to get around it. The setting is read once at startup. A saved
adventure keeps the text it was generated with.

### Speech input (hold to talk)

On the phone, a "Hold to talk" button sits under the dialogue text box. The player holds it
(pointer, touch, or Space/Enter), speaks for up to 15 seconds and releases; the server
transcribes the clip and the words appear in the same text box. The player edits them and
presses Send (the same path as a typed line) or Clear. Typing works throughout, and a press is
the only thing that ever asks the browser for the microphone.

Settings (process environment; invalid values refuse startup without echoing them):

- `ELEVENLABS_API_KEY`: the same key as spoken narration; `run-gameplay.sh` exports it from
  `.env`. Without it speech input is off and the button is not shown.
- `DF_GAMEPLAY_STT=off`: switch speech input off. The startup line reads `STT on model=<id>
  budget=<seconds>s` or `STT off (<reason>)`.
- `DF_GAMEPLAY_STT_SECONDS_BUDGET`: audio seconds the server may transcribe in one run
  (default 600). Each clip is charged, rounded up to whole seconds, before the provider call
  and never refunded; when it is spent the phone says so and typing continues.
- `DF_GAMEPLAY_STT_MODEL`: default and only accepted value today is `scribe_v2`, the model the
  registered ElevenLabs speech-to-text route sends.

Browsers only grant the microphone on `https` or `localhost`. A phone that joins over plain
`http://<this-machine-ip>:63309` cannot use it; the button then explains that instead of
failing silently. To use speech input from a phone on the network, put an HTTPS front or a
tunnel in front of the server and set `DF_GAMEPLAY_ALLOWED_ORIGINS` to that `https://` origin.

Privacy: the clip is decoded to 16 kHz mono in the page and sent in one request to
`POST /gameplay/speech`, authenticated with the player's room binding. The server holds the
audio and the transcript in memory for the length of that request only, and returns the
words to the player who sent them. Neither is written to disk, cached, logged or put in a
telemetry span (which records sizes, durations and an outcome label). Only the words are
used: nothing is derived from the voice, and nothing is kept. Microphone tracks are stopped
as soon as the recording ends.
