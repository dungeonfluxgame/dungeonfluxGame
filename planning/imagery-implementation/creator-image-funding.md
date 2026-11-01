# Creator account image funding

Owner decision, 2026-10-28: before joining a campaign, hosted creator portraits
charge the source-authenticated creator account's existing purchased balance,
with explicit consent and a spending limit. This specific decision supersedes
the general no-spend-cap guidance for this account-owned workflow. It does not
change campaign admission policy or authorize provider calls during tests.

The exact source migration and disposable database scope was explicitly approved
after automatic approval review initially rejected the write; see
[creator-funding-approval.md](creator-funding-approval.md).

## Contracts

`df_commerce::draft_image` defines a consent scope containing account, draft and
revision, job, source/configuration digests, provider/model/route, immutable price
version and canonical maximum-unit categories. `Binding` adds stage, asset,
publication attempt, finance operation/attempt/effect and payload digest.
Length-prefixed domain-separated fingerprints cover every field. Consent terms
also fingerprint cap, expiry and revision; a changed cap or deadline cannot
reuse an existing quote. These pure checks never authenticate, reserve funds or
permit a provider send.

`df_finance::draft_image` extends the existing `FinanceService` with
`record_draft_image_consent`, `revoke_draft_image_consent`, `quote_draft_image`,
`admit_draft_image`, `prepare_draft_image`, `mark_draft_image_sent`,
`mark_draft_image_unknown`, `settle_draft_image` and
`release_unsent_draft_image`. Every entry point takes the externally authenticated
`df_types::AccountId`; no seat, campaign or session is used to infer a payer.
Runtime/HTTP consent presentation and source identity authentication are composition
work, outside this foundation.

The PostgreSQL command takes supplier, account, consent, quote and existing
purchased-lot locks before committing the registered finance event. Quote pricing
uses existing immutable tariff rows and exact numeric arithmetic. Maximum retail
and supplier amounts round upward; known retail rounds down and remains bounded
by the pinned maximum. Known supplier cost rounds half up. No tariff is supplied
by the request and a missing category refuses admission.

Money remains in `finance.account_balance`, the existing double-entry journal,
funding lots, reservations, provider attempts, receipts and usage events. Account
code 2150 holds purchased credit by payer and lot. Account reservation shares play
the same role as campaign reservation shares; they are not another money ledger.
Consent remaining capacity is derived from existing charged amounts plus all open
holds. New account event equations and deferred controls independently check
shares, balance keys, exact quote ownership, aggregate cap and attempt state.
Campaign allocation and account admission contend on the same payer and lot locks.

Admission requires an `AdmissionPort` in the same transaction. The real
`df_persistence::imagery_draft_admission::DraftAdmissionPort` checks its injected
canonical `DraftAuthority`, actual account-private catalog fields and quota,
attached frozen imagery job and exact recipe/configuration snapshots. It inserts
the existing `image_publication_allocation` and existing finance binding, then
lets finance commit. A replay verifies historical linkage without reserving quota,
repairing missing rows, enqueuing work or granting another send. The authoritative
receipt admission time controls job expiry and catalog timestamps.

## Settlement and recovery

Persist Sent before egress. Only the first successful `mark_draft_image_sent`
returns true; a replay cannot authorize another deliberate send. A changed
dispatcher conflicts. Unknown outcomes retain the full customer reservation and
supplier exposure, including after expiry or consent revocation. Release refuses
potentially sent work. Known settlement or billed failure closes supplier exposure
and releases only the known unused customer remainder. Unusable output charges
the customer zero and records supplier cost in the existing absorbed-loss account.
An underestimated supplier invoice never increases customer consent: customer
charge remains capped, the excess is an explicit absorbed loss and future supplier
admission stops pending incident handling.

A failed COMMIT acknowledgement returns `CommitUnknown`. Recover by replaying the
same account/operation, consent, quote and binding; do not release or send again.
Duplicate settlement compares stored units and usability; changed evidence
conflicts. Restricted/disputed funding lots fail closed for their existing dispute
owner rather than becoming available credit.

## Installation and integration

Apply migration 0043 after finance 0020-0025 and the imagery composition 0039.
Standalone finance tests can apply its finance portion without media/imagery;
the actual imagery composition must have 0039 present when installing 0043.
The existing `df_imagery_admission` role inherits the ordinary finance entry point
and retains its narrow catalog/journal privileges. Ordinary finance gets no
catalog write privileges. No production migration is performed by this work.

A runtime can wrap `DraftAdmissionPort::bind` to add its durable execution queue
record in the same supplied transaction. On replay it must only verify the
existing queue mapping. The runtime must supply current canonical draft/source
authority and present consent for every pinned route before admission. It must
retain the exact quote id and expiry across retries and recover unknown outcomes
before any retransmission.

The account-specific transaction command constructs registered events through
`finance.commit_event`; it does not bypass posting allowlists, equations, journal
hash chains, balance locks or existing lot conservation. The SQL command is kept
atomic because all admission and settlement evidence must be decided under the
same locked snapshot.


## Paid stages under one job consent

The 2026-10-29 follow-up adds JobScope/JobConsent with a closed allowed StageRoute
set and one cap over all existing charged and held reservations. StageCall pins
canonical execution stage, dispatch, dependency parent, input/configuration basis
and the planned publication pair. Local transforms never enter paid admission.
Exact quote recovery preserves the original finance ids and expiry; the separate
read_draft_stage_quote lookup does not mint or mutate anything.

CanonicalStagePort binds both account and campaign stages to actual execution in
the money transaction. Pre-freeze Classification creates no media asset, quota or
publication allocation. The first output at planned_output_stage creates the real
common allocation; Inspection, Correction and Extraction reuse it while retaining
their distinct execution stage ids. Legacy first-image contracts stay compatible.
StageAuthority verifies source grants before catalog/job locks, then its required
record_prepared_sources callback retains exact source receipts after prepare.
Replay verifies existing linkage and sources without repair.

mark_draft_stage_sent and mark_provider_stage_sent require a source/execution
verification port inside the financial Sent transaction. Only first committed
Sent returns true. CanonicalStageSend locks the current claim, cursor, gate and
source authority; the runtime adapter commits execution Sent in that same supplied
transaction. The old account send entry point refuses multi-stage bindings.
CanonicalStageSettlement verifies the exact selected durable RuntimeUsage,
route, revision, units and usability before using existing settlement planners,
then mirrors execution settlement in that transaction. Unknown creator stages
retain the full account hold and supplier exposure. Existing campaign customer
hold deadlines remain unchanged; released customer holds with open supplier
liability use the existing late reconciliation owner and never become fake usable
completion or permission to retry.

Source is implemented; independent multi-stage and full consumer acceptance are
separate from the already accepted exact-pricing repair. No deployment or paid
provider call is part of these tests.
