# G11-D03: Bounded memory candidates and native retrieval ownership

Status: Design decision; production types, storage and numeric limits remain gated.

## Outcome

Adopt the existing `df-session`-owned native retrieval port as the sole owner of
memory-candidate I/O. `df-persistence` implements the port with bounded indexed
PostgreSQL reads. Retrieval is an admitted, cancellable staged job under the
session's existing effect executor and ownership/fencing rules. It returns a
typed, bounded batch to the owning session; it does not update authoritative
knowledge or publish client-visible output.

`df-knowledge` receives that batch and the current pure knowledge basis, checks
observer scope and provenance again, and deterministically ranks/selects only
within the supplied bounds. The session revalidates the basis, source digest,
access generation, and revocation state before it commits or projects selected
memory. Directors consume the resulting bounded proposal/context; they do not
call a database, semantic index, provider, or system clock.

This records the original outcome, “native retrieval owner,” and keeps ownership
consistent with `planning/long-horizon-state.md` and
`planning/subsystem-interfaces.md`. It defines a design boundary, not a concrete
Rust API or proof of a running implementation.

## Candidate and batch contract

A memory candidate is a bounded, attributed reference to canonical committed
history or to a clearly marked derived summary. Its provenance identifies the
episode/decision or covered episode IDs, subject and permitted audience, logical
game time, source/content/run versions, source digest, policy/index generation,
and any claim/evidence IDs. It preserves uncertainty and contradiction; it does
not assert that a summary is canonical truth. Payload snippets are optional,
length-bounded presentation material. The canonical record and replay history
remain authoritative and recoverable independently of every summary or index.

The authorized query binds a trusted observer and access generation, purpose,
session/run/content/source basis, permitted topics/entities/time range, and hard
item, byte, token, and deadline limits. Correlation/trace IDs are diagnostic
context and never grant access. Apply access filtering before candidate ranking
or nearest-neighbor selection, then recheck it on returned rows and at selection
and projection. Return only authorized candidate references and the coverage,
lag, source/index generation, and completeness state needed to assess their
limits. Diagnostics must not expose cross-subject keys, private snippets, or
secret-bearing embeddings.

Admission is fail-closed and bounded at each boundary:

- Reject absent/invalid authorization, revoked or stale access generation,
  mismatched session/run/source basis, unsupported policy/index version, or an
  expired deadline before reading.
- Reject zero/over-limit or internally inconsistent item/byte/token budgets;
  never widen caller limits to make a request succeed.
- Bound the database result and owned batch by both count and encoded bytes.
  Stop before adding an item that crosses either cap and report explicit
  truncation/coverage; never replace this with an unbounded scan.
- A missing/stale optional index may trigger a bounded canonical query. If that
  query cannot meet the same limits, return `Incomplete` with coverage or a typed
  unavailable/capacity result. No index means no embedding/provider call on the
  retrieval path.
- Cancellation before job admission creates no job. Once admitted, the native
  owner controls its lifetime; cancellation/deadline/capacity/permission/stale
  outcomes are explicit, and stale completions are discarded against job and
  basis generations. An accepted run-owned job is not implicitly cancelled by a
  disconnected requester.
- Reauthorize before returning selected material, committing knowledge changes,
  and projecting for each audience. A prior store check, cached result, summary,
  or similarity score is never a disclosure grant.

Candidate ordering must be deterministic for a fixed authorized query and batch:
relevance/salience/recency policy with stable tie breaks, explicit uncertainty,
and no claim that model confidence can supersede committed truth. A contradictory
belief remains an attributed belief. A summary inconsistent with its sources is
quarantined/rejected as derived data; it cannot rewrite a fact. Retention and
compaction preserve identity-defining events, obligations, active facts, source-
linked rulings, replay decisions, and explicit gaps. Rumor transmission retains
attribution and causal deduplication and never rewrites the originating fact.

## Authority and ownership

| Boundary | Owns | Must not own |
| --- | --- | --- |
| `df-session` | Authorized query construction, job admission/lifetime, native port invocation, basis/fence checks, commit and audience projection | SQL/index implementation or pure ranking rules |
| `df-persistence` | `MemoryCandidateStore` adapter, bounded PostgreSQL access paths, durable canonical episodes/decisions, replaceable index reads and generation metadata | Disclosure policy decisions, director decisions, a second timer/job loop or authoritative summaries |
| `df-knowledge` | Pure reauthorization/ranking over supplied bounded candidates, provenance-aware knowledge delta and explicit incompleteness | Database/network/provider/clock I/O, unbounded retrieval, truth mutation from a summary |
| Directors (`df-world`, `df-intent`, `df-interaction`, `df-narrative`, `df-encounter`, `df-combat`) | Consume scoped bounded outputs as inputs to pure proposals under their own policies | Retrieval I/O, unrestricted transcript context, recursive calls, parallel persistence or authority over rules outcomes |
| `df-engine` / rules owner | Compose and validate candidate changes and source-grounded consequences through existing engine/session flow | Model-generated truth, unsupported mechanics or bypass of committed decisions |

The existing database is PostgreSQL for durable gameplay. Do not add a memory
database or another schema authority. Summaries and semantic indexes are
replaceable, versioned derived aids. They can accelerate finding canonical
records, but cannot replace snapshots, committed facts/decisions, exact replay
inputs, provenance, or required rights/audit evidence. An absent or stale derived
artifact is a visible coverage gap, not a reason to invent history.

## Literal std-only contract example

This intentionally small executable illustration shows bounded admission and
refusals at the contract edge. It is not a production API, authorization
mechanism, ranking algorithm, database adapter, or selected production limit.
The `authorized` and `basis_current` booleans stand in for trusted session checks;
production types and enforcement are frozen by G03/G05/G10/G11.

```rust
#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    Unauthorized,
    StaleBasis,
    InvalidLimit,
    ByteLimit,
}

#[derive(Debug)]
struct Candidate {
    canonical_ref: u64,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct Batch {
    items: Vec<Candidate>,
    bytes: usize,
    truncated: bool,
}

fn admit(
    authorized: bool,
    basis_current: bool,
    max_items: usize,
    max_bytes: usize,
    candidates: Vec<Candidate>,
) -> Result<Batch, Refusal> {
    if !authorized {
        return Err(Refusal::Unauthorized);
    }
    if !basis_current {
        return Err(Refusal::StaleBasis);
    }
    if max_items == 0 || max_bytes == 0 {
        return Err(Refusal::InvalidLimit);
    }

    let mut batch = Batch {
        items: Vec::new(),
        bytes: 0,
        truncated: false,
    };
    for candidate in candidates {
        if batch.items.len() == max_items {
            batch.truncated = true;
            break;
        }
        let Some(next_bytes) = batch.bytes.checked_add(candidate.bytes.len()) else {
            return Err(Refusal::ByteLimit);
        };
        if next_bytes > max_bytes {
            if batch.items.is_empty() {
                return Err(Refusal::ByteLimit);
            }
            batch.truncated = true;
            break;
        }
        batch.bytes = next_bytes;
        batch.items.push(candidate);
    }
    Ok(batch)
}

fn candidate(canonical_ref: u64, bytes: &[u8]) -> Candidate {
    Candidate {
        canonical_ref,
        bytes: bytes.to_vec(),
    }
}

fn main() {
    let accepted = admit(
        true,
        true,
        2,
        5,
        vec![
            candidate(10, b"abc"),
            candidate(11, b"de"),
            candidate(12, b"x"),
        ],
    );
    let batch = accepted.unwrap();
    assert_eq!(batch.items.len(), 2);
    assert_eq!(batch.bytes, 5);
    assert_eq!(batch.items[0].canonical_ref, 10);
    assert!(batch.truncated);

    assert!(matches!(
        admit(false, true, 2, 5, vec![]),
        Err(Refusal::Unauthorized)
    ));
    assert!(matches!(
        admit(true, false, 2, 5, vec![]),
        Err(Refusal::StaleBasis)
    ));
    assert!(matches!(
        admit(true, true, 0, 5, vec![]),
        Err(Refusal::InvalidLimit)
    ));
    let bounded = admit(true, true, 2, 3, vec![candidate(20, b"four")]);
    assert!(matches!(bounded, Err(Refusal::ByteLimit)));

    let byte_bounded = admit(
        true,
        true,
        3,
        3,
        vec![candidate(30, b"ab"), candidate(31, b"cd")],
    )
    .unwrap();
    assert_eq!(byte_bounded.items.len(), 1);
    assert_eq!(byte_bounded.bytes, 2);
    assert!(byte_bounded.truncated);
}
```

The example deliberately asserts typed denial for unauthorized/stale/invalid
requests and an oversized candidate, plus bounded, visibly truncated results at
the item and byte caps. Production must additionally classify and expose
coverage/omission reason without leaking protected metadata.

## Alternatives considered

- Let each director read the database: rejected because it couples pure policy
  code to I/O, duplicates access checks and lifecycle ownership, and invites
  unbounded per-NPC query loops.
- Let `df-knowledge` own native storage: rejected because its assigned
  authorization/ranking operation is pure; native retrieval belongs to the
  session-owned consumer port and persistence adapter.
- Let summaries or embeddings be the memory authority: rejected because derived
  data may be stale, lossy, contradictory, revoked, or absent and cannot satisfy
  canonical replay or rights guarantees.
- Always use an external/vector index: deferred. No embedding/index choice is
  justified until representative workload evidence shows benefit; canonical
  bounded queries remain the fallback.
- Return all matching episodes and truncate in the director: rejected because
  it violates memory/latency bounds before the cap and can leak unauthorized
  candidates during ranking.

## Preserved acceptance and unresolved production gates

The original criteria remain: **native retrieval owner** and actual
source/build-bound evidence with unsupported, pending, failed, and unperformed
checks explicit. The original verification intent remains to freeze the cited
decision, show a bounded contract example, compare ownership, and retain
alternatives and unresolved facts. This document satisfies the design decision
and example portions only; it does not claim runtime behavior or the independent
review gate.

Still unresolved: G03 concrete Rust IDs/errors/codecs and access-generation types;
G05 physical PostgreSQL schema, indexes, access paths, deadlines, connection and
result limits; G08 if embedding work incurs provider cost; G10 episode/summary,
versioning, replay, rights and migration schemas; G11 actual workload-derived
item/byte/token/time/CPU/query bounds, retention/decay policy, relevance
calibration and false-disclosure evaluation. The selected index technology is
not frozen. The exact formatter/compiler commands are owned by G01.

The literal std-only example was formatted, compiled with warnings denied, and
executed against its finite assertions. Native/WASM workspace checks and
integrated storage/retrieval checks are unperformed because this is a design-only
change and concrete production types and physical schema are pending. A later
implementation must verify accepted
and adversarial boundaries, stale index and stale consolidation, source hashes,
missing episodes, cancellation/revocation, deterministic ties, privacy
noninterference, bounded query work and replay with zero provider calls. Final
multi-session continuity behavior remains S05/MEMORY-LONGHORIZON-ACCEPT evidence,
not a result of this document.

## Source basis

- `planning/long-horizon-state.md`: retrieval/contradiction semantics, bounded
  request and attributed result, native staged-job ownership, pure ranking,
  reauthorization, derived-index fallback, replay and rights lifecycle.
- `planning/subsystem-interfaces.md`: common bounded/error/access rules and the
  `df-session` port, `df-persistence` adapter, `df-knowledge` pure consumer split.
- `planning/storage-architecture.md`: PostgreSQL canonical gameplay state and
  derived authorized indexes; no extra memory database/schema authority.
- `planning/interaction-engine.md`: truth/knowledge/belief/witness/memory
  distinction, provenance, decay, rumor attribution and bounded retrieval.
- `planning/runtime-directors.md`: pure directors, session commit ownership,
  deterministic bounds, replay and unresolved G10/G11 gates.
- `planning/implementation-roadmap.md`: G03/G05/G08/G10/G11 gates and S05
  continuity acceptance; no empty parallel implementation.
- `planning/generated-content.md`: candidate validation, versioning, privacy,
  stale results and explicit failures inform derived consolidation handling.
- `planning/coding-style.md`, `rustfmt.toml`, and `rust-toolchain.toml`: public
  contract, ownership, deterministic behavior, typed failures and verification
  expectations.
- `development/backlog-catalog.json`, task `B-G11-D03`: frozen objective,
  owners, expected native retrieval owner and original acceptance criteria.
- `AGENTS.md` and ADR 0001–0005: scoped worktree/document ownership, meaningful
  devlog, source/build-bound evidence, independent review, and honest unperformed
  checks.

## Amendment 2026-10-17: owner decision, vector index now

The owner decided to build the DM's story memory on text embeddings with a pgvector index in the
game's PostgreSQL now. This overrides only the "Alternatives considered" deferral of an always-on
vector index ("deferred until representative workload evidence"). Everything else in this policy
stands and is followed by the implementation: PostgreSQL only, audience filtering before ranking
and again on the returned rows, hard item and byte bounds, deterministic ordering, derived data
that never rewrites a fact, and replay with zero provider calls. Where the implementation departs
from a rule (the query embedding on the retrieval path; rows that no canonical source can
rewrite) the departure and its reason are recorded in `planning/story-memory.md`. The policy
text above is otherwise unchanged.
