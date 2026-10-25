# DungeonFlux versioning policy

Status: proposed owner policy, 2026-10-25.

## 1. Product version

DungeonFlux has one product version for each hosted-service release. It identifies the
server, browser and phone clients, and any web bundles built and shipped together.
The root workspace version in `Cargo.toml` is the single source for that version.
Cargo crates are internal implementation units and are not independently published
to crates.io. Keep workspace crate package versions inherited from
`[workspace.package]`; do not create a separate release cadence for each crate.

There are a few current exceptions: `df-campaign`, `df-campaign-wire`,
`df-finance`, `df-persistence`, `df-runner`, and `df-server` declare their own
`0.1.0` rather than inheriting the workspace version. `df-launcher` is a separate
non-workspace package with its own lockfile and version. The product release must
still identify the exact launcher and crate revisions in its build manifest; align
these package versions to the workspace when practical, or leave them as explicit
package versions without treating them as product releases. Versioned client/server
contracts and durable data formats have their own compatibility versions (section
5), because they can outlive a deploy.

Use SemVer 2.0.0 for the product version: `MAJOR.MINOR.PATCH[-prerelease][+build]`.
The published product identifier omits build metadata. A development build may add
`+git.<short-sha>`; it does not change precedence or compatibility.

## 2. Before 1.0.0

DungeonFlux is pre-launch and currently declares workspace version `0.1.0`. Keep
major at zero until the paid launch gate in `planning/master-launch-plan.md` section
19 is passed. In 0.y.z, use these rules:

- Bump MINOR for each owner-approved, user-facing capability increment or
  compatibility-breaking change to a client/server contract, save format, or
  operator-facing integration. Reset PATCH to zero. Example: `0.1.4` to `0.2.0`.
- Bump PATCH for compatible fixes, security fixes, content corrections that do not
  change a contract, and compatible additions. Example: `0.2.0` to `0.2.1`.
- A breaking change during 0.x is permitted, but it still bumps MINOR; do not use
  PATCH for a breaking change just because the product is pre-1.0.

1.0.0 means the owner has accepted the paid launch gate in section 19, including the
proved capability level and its required operational and customer evidence. Reaching
1.0 is not a calendar date or a code-completion milestone. Keep the product at 0.x
while that gate is not met.

Pilot builds use `0.y.z-alpha.N` for internal pilots, `0.y.z-beta.N` for broader
customer pilots, and `0.y.z-rc.N` for a release candidate. Start each sequence at 1
and increase N for every new artifact in that sequence. A final release removes the
prerelease suffix. Examples: `0.2.0-alpha.1`, `0.2.0-beta.2`,
`0.2.0-rc.1`, then `0.2.0`. A CI-only build can identify its source as
`0.2.0-alpha.1+git.a1b2c3d`; metadata never substitutes for the prerelease label.

## 3. From 1.0.0 onward

Choose the smallest level whose public compatibility promise describes the change.
Changes that ship together use the highest level required by any included change.

| Bump | DungeonFlux trigger and example |
|---|---|
| MAJOR | Break a supported client, integration, or stored-data reader without a compatible transition. Removing a protobuf field and reusing its number; changing a required `/api/v1` meaning; releasing a save format old supported servers cannot read without a migration; or requiring an unannounced manual database step. Example: `1.8.4` to `2.0.0`. |
| MINOR | Add compatible product capability or an opt-in, versioned contract. Examples: a new optional protobuf field, a new optional response field, a new campaign feature, or a new save format that has a reader/migration path for supported old saves. Example: `1.8.4` to `1.9.0`. |
| PATCH | Correct behavior while preserving supported contracts and data. Examples: a security fix, crash fix, rendering correction, rules-content clarification that does not change a promised existing outcome, or a price-file correction that is not yet published/effective. Example: `1.8.4` to `1.8.5`. |

Additional examples and boundaries:

- A newly added optional protobuf field is MINOR; removing or changing the type or
  meaning of an existing field is MAJOR unless the old field remains supported and
  a new field carries the new meaning.
- A forward-only migration that adds nullable/defaulted structures and needs no
  manual operator action is usually PATCH or MINOR depending on user-visible
  capability. A migration that needs a manual step or breaks supported rollback or
  rolling-deploy compatibility is MAJOR after 1.0.
- A save/checkpoint writer change that old supported servers cannot read is MAJOR
  unless migration or read-only refusal protects the old data and supported clients.
- An effective published price change is a customer-visible commercial change:
  announce it and use MINOR unless a signed customer agreement or launch promise
  requires a more conservative major boundary. Never rewrite a price version already
  used by a charge. A typo fix in an unpublished price file can be PATCH.
- A rules-content addition is MINOR when it adds an available option; a correction
  that changes existing character or encounter outcomes needs owner review and is at
  least MINOR. A cited text clarification with identical outcomes can be PATCH.
- A client-visible UI addition is MINOR; a cosmetic correction or accessibility fix
  is PATCH. Removing a promised workflow or changing user-visible meaning is MAJOR.
- Security fixes are PATCH when they preserve interfaces; disclose and schedule a
  MAJOR only when compatibility has to break to contain the issue. Do not delay a
  security repair to align with a feature release.

## 4. Changes and approvals

Keep the repository's existing descriptive subject style (`planning:`, `deploy:`,
`gameplay:`, `docs:`). Conventional Commit syntax is not required. Each change brief
and release note should state whether the product version impact is none, patch,
minor, or major, with the affected compatibility surface. The owner resolves
disagreement and approves every release.

Before cutting a release, integrate and review the source changes, choose the bump
from sections 2 or 3, update the workspace version, add a player-facing entry in
`docs/changelog/` for visible changes, and update the developer changelog
(`CHANGELOG.md`) with the release commit hash and subject under the active work
block. The coordinator owns that developer changelog. Build and qualify the exact
tagged commit; record commit, configuration, content revision, migration level and
image digests in release notes. The owner approves the release/tag and any
production deployment.

Release steps:

1. Ensure the intended source and release notes are integrated and reviewed. No
unrelated dirty files may be included in a release.
2. Run `development/scripts/version.sh bump <major|minor|patch|pre>` as appropriate,
   review its diff, and complete the changelog entries.
3. Run required CI, release qualification, browser checks for shipped clients, and
   the capability evidence required by master launch plan sections 12.6 and 19.
4. Commit the release version and changelogs. The owner creates annotated tag
   `vX.Y.Z` (or the prerelease version) on that exact commit and pushes the tag.
5. CI builds artifacts from the tag. It may publish readable version and
   `sha-<short>` image tags, but deployment and verification use immutable
   `@sha256:<digest>` references. The existing signed release workflow is a
   separate, owner-gated production operation; a Git tag alone is not approval to
   deploy.

For a hotfix, branch from the last supported release, apply the smallest repair,
run the same affected compatibility and release checks, and increment PATCH. If a
contract break is unavoidable, use the applicable MAJOR version and transition
plan. Merge the fix back to active development and record both release tags. Never
rewrite or move a published tag.

## 5. Separately versioned contracts and data

Product versioning does not replace these compatibility identifiers. Any value
called “planned” below is a required contract to implement before the corresponding
surface is released; it is not a claim that the current code already exposes it.

### 5.1 Protobuf and gRPC protocol

`crates/df-protocol` defines the game wire contract; `crates/df-campaign-wire`
currently contains Rust wire definitions without a separate `.proto` schema.
Never renumber a protobuf field. When removing a field, reserve both its number and
name in the message. Never reuse a reserved or previously published number.
Compatible optional/additive fields do not require a protocol major bump; changed
field type, meaning, or removed required capability needs a new compatible
contract, usually protocol MAJOR.

Add a `protocol_version` to client connection metadata and make every client send
it. The server must advertise and enforce a supported minimum/maximum range, return
a typed incompatibility outcome, and log the client and server product versions.
During rolling deployments, the server must support the protocol versions used by
all still-supported clients. Maintain a protocol compatibility note/table with
each release; do not infer compatibility from the product version. Current
protobuf fields already use `reserved` for retired fields in places, but no general
client `protocol_version` handshake was found during this policy review.

### 5.2 Account application JSON and integration API

`crates/df-account-app/src/wire.rs` defines first-party JSON. It has no URL version
today. Evolve responses additively: new response fields are optional or have a
documented default, existing names/types/meaning remain stable, and old clients
ignore unknown response fields. Keep request DTOs strict where they reject unknown
fields; introducing request options must therefore remain backward compatible with
older servers. If a breaking first-party wire change is needed, negotiate a wire
version or maintain the prior route during a client upgrade rather than silently
changing the JSON shape.

The planned third-party integration API is explicitly `/api/v1` in
`planning/integration-api-and-embeds-research.md`. V1 is additive only: do not
remove, rename, change type/meaning, or tighten accepted values of a published
field. A breaking change gets `/api/v2`. Mark deprecated fields/endpoints in docs,
announce removal, and retain them for at least 12 months and two product MINOR
releases after the replacement is available, whichever is longer, unless a
security/legal issue requires earlier removal. Give users a dated end-of-support
notice. The document is currently design-only; publish no v1 compatibility promise
until its implementation and owner approval.

### 5.3 Saves and checkpoints

Each durable save/checkpoint encoding has its own explicit format/schema version,
separate from product, database and content versions. When a new format is written,
keep a reader or deterministic migration for every supported prior format. If an
older format cannot be migrated safely, refuse writes and offer read-only access
with an explicit reason; never silently discard or reinterpret data. This follows
SAVE-05: preserve old-format refusal/migration behavior and verify real pre-change
checkpoint bytes. Keep compatibility fixtures for every released format and test
round trips and unknown versions.

### 5.4 Database migrations

`crates/df-persistence/migrations/` uses numbered, forward-only SQL. A migration
already present in a released commit is immutable: add the next unused number and
never edit, delete or renumber an applied migration. Gaps in the existing sequence
are permitted and must not be backfilled. A release records both the ordered
migration set and the highest successfully applied migration number (for example,
`db_schema=0024`); the database migration ledger remains authoritative for actual
state. Expand/contract and rollback compatibility follow section 12.6 of the master
launch plan. Destructive or irreversible data operations require backup/restore
rehearsal and an explicit downtime/rollback plan.

### 5.5 Content, rules, and prices

Content packs have content-owned schema/pack versions, such as the current rules
source manifest's `schema_version`. Identify each release's content by immutable
revision or digest and record that revision with the running deployment. Rules
content additions and corrections follow product impact rules in section 3;
imported or summarized content never becomes executable rules by implication.

Pricing has a `price_version` in admission and ledger contracts. Every charge must
record the exact `price_version` used to calculate it, along with the immutable
price snapshot needed to explain the charge. Once a price version is published or
used by a charge, never edit it. Create a new version with effective time and
customer notice. A price version is independent from product version: code can
ship without changing prices, and prices can change without a server binary
release where the approved price loader permits it.

## 6. Version identity in artifacts and runtime

At build time, expose the workspace version from Cargo metadata (`env!("CARGO_PKG_VERSION")`)
and the source Git SHA from the build environment. Do not duplicate a manually
maintained version constant. Provide an authenticated or otherwise appropriately
scoped health/version response with product version, full commit SHA, protocol
range, content digest, and database schema level. Show product version and commit
in each client About/diagnostics view. Attach product version, commit SHA, and
deployment environment as OpenTelemetry resource attributes; keep high-cardinality
SHA out of metric labels and never put private player data in version diagnostics.

Build server, client and web bundles from the same source revision. Web bundles
(including `df-account-app/build-web.sh` and `df-tools` outputs) must expose the
product version/commit in their generated build metadata or About view. Publish
container images with `:<version>` and `:sha-<short>` human-readable tags, and pin
all deployment references by `@sha256:<digest>`. The version tag is immutable once
published; do not deploy by a floating tag. Record exact versions, image digests,
content digest, protocol range and schema level in release notes and the pinned
deployment manifest. `deploy/images/images.lock` already documents exact tag plus
digest references; its placeholders are not release evidence.

## 7. Version helper and compatibility CI

`development/scripts/version.sh show` reads the root workspace version.
`development/scripts/version.sh bump major|minor|patch|pre` edits only that
`[workspace.package] version` in root `Cargo.toml`, refuses to run if that file is
dirty, and prints the follow-up release steps. For 0.x, `major` is refused; use
`minor` for a breaking pre-1.0 boundary. `pre` advances `alpha.N` on the current
base version, starts `alpha.1` on the next minor version when run from a stable
version, and advances the number within the current beta/rc stage. Moving between
alpha, beta, rc, and stable is an owner-reviewed edit to the workspace version.

The CI compatibility workflow checks that every migration file already present in
the target/base commit is unchanged in the proposed commit; new numbered files are
allowed. This is cheap and reliable with Git tree comparison. The existing
protobuf schema has no pinned descriptor baseline or parser-based breaking-change
tool, so text matching field declarations would be unsafe and can miss reuse,
renames, nested messages or syntax changes. Add a descriptor-aware check (for
example, a pinned Buf breaking check with a committed baseline) as a separate task
before external clients depend on the protocol. Until then, reviewers apply the
field-number rules in section 5.1 to every protobuf change.

## 8. Initial version proposal

Propose `0.1.0` as the first owner-approved product tag. The current workspace and
internal packages already use `0.1.0`, there are no Git tags, and the paid launch
gate is still ahead; starting at `0.1.0` identifies the baseline without implying
that the service is generally available. Tag the reviewed release commit after
this policy is integrated and its associated release notes are ready. The owner
can run:

```sh
development/scripts/version.sh show
git status --short
git tag -a v0.1.0 -m "DungeonFlux 0.1.0 baseline"
git push origin v0.1.0
```

If the owner wants the version/tag to identify the current pre-policy HEAD instead,
use `git tag -a v0.1.0 03c376b4 -m "DungeonFlux 0.1.0 baseline"`; normally tag the
integrated, reviewed release commit so the policy and release notes are included.
