# Campaigns, membership and the table (lobby, invitations, display pairing)

Date: 2026-10-25 (round 0i: four precise changes after the eighth critic review)
Status: plan (agent S1). No code exists for anything described here. Implementation rounds follow
critic review of this note. Plan section numbers refer to `planning/master-launch-plan.md`; its
section 3.2 matrix is the authority (the PDF text extract garbles it).

Scope: campaigns (create wizard, overview, settings), membership and the section 3.2 role matrix,
invitations and join tokens, room codes (discovery only), the lobby (seats, character selection,
readiness, microphone state, display pairing), and the campaign dashboard data. **S1 serves JSON, never
HTML** (decision 29): every screen is a route of P's `df-account-app` (a wasm app mounted by B at
`/app/`), rendered from the `DashboardView` and the campaign JSON API defined here. Not in scope:
identity and accounts (B), the app and its screens (P), money (F), session lifecycle, the edge,
runtime routing, checkpoints (O1), character templates, assets, exports (O2).

## 1. Governing documents and what each fixes here

| Document | Followed here |
| --- | --- |
| `master-launch-plan.md` 3 (vocabulary, 3.2 matrix), 8 (routes), 9.4-9.6, 10 (J01, J02, J07, J08, J14), 14.1, 15.1, 15.4, 17 (AUTH-01, AUTH-02, UX-01) | Every requirement in section 2 |
| `artifacts/tmp/agents/LAUNCH.md`, decisions 1, 5, 6, 7-12, 24, 27-29 | Fakes only, configuration not constants; one database with a `campaigns` schema (7); migrations 0008-0009 (8); hosted players authenticate, six seats configurable (9); host transfer does not transfer ownership (11); creating a campaign needs a verified email (12); membership revocation is S1's check-at-use and O1 reads membership at each batch start (24); money core is F's (27); account screens are P's wasm app and S1 serves JSON (28, 29); lock order (29); event-bumping transactions set `transaction_timeout` (29) |
| `planning/hosted-runtime-research.md` (O1) | 6.1 fence contract, 6.2 edge routing and tickets, 6.4 membership (D24); the agreement in 13.1 below |
| `planning/asset-storage-research.md` (O2, committed 10373d08) | The gateway is an in-memory consumer of the feed (13.3) |
| `planning/finance-ledger-research.md` (F) | `start_readiness`, `open_session_funding`, the `funding` consumer (13.2) |
| `planning/account-service-research.md` (B) and P's round-0 note | Session cookie, CSRF header `X-DF-CSRF`, the `/app/api/` envelope, `CampaignDirectory::list_for_dashboard` |
| `AGENTS.md`, `planning/coding-style.md` | Rust only, thin client, crate-per-subsystem, typed errors, injected clock, audit lines, dev CLI view per system, tunables in files, sentence-named tests |
| `planning/g04-d01-decision.md`, `g04-d02-decision.md`, `remote-play.md` | Payer, member and host separate; idempotent membership; two-party transfers; player and display roles; presence is not membership |
| `crates/df-auth` (`membership.rs`, `permissions.rs`, `readiness.rs`, `throttle.rs`), `crates/df-session/src/invitation.rs` | Reused, extended additively (section 4) |

## 2. Requirement map

Client screens named `/app/...` are P's; endpoints are S1's JSON (section 7).

| # | Requirement (plan) | Crates and types | Tables (schema `campaigns`) | Screens and endpoints | Tests (section 12) |
| --- | --- | --- | --- | --- | --- |
| R1 | Campaign exists with no session (3.1) | `df-campaign::Campaign`, `CampaignStatus` | `campaign` | `/app/campaigns/:id`, `GET campaigns/:id` | `a_campaign_with_no_session_still_opens_its_overview` |
| R2 | Create wizard, five steps, draft saves, free, a failed setup cannot lose the campaign; verified email (9.5, decision 12) | `CampaignDraftStep`, `TableDefaults`, `CampaignService::save_step/finish` | `campaign` (status `draft`), `campaign_operation` | `/app/campaigns/new`, `drafts` endpoints | `finishing_the_wizard_twice_with_one_operation_creates_one_campaign`, `an_unverified_account_cannot_start_a_draft` |
| R3 | Overview with six tabs (9.4) | `OverviewView` (lists only mounted tabs) | `campaign`, `campaign_member` | `GET campaigns/:id`, `/sessions`, `/settings` | `the_overview_view_lists_only_mounted_tabs` |
| R4 | Role matrix 3.2; payment and campaign roles independent | `df_auth::roles::{CampaignRole, Capability, Grant, Delegations, OwnerAction}`, `support_grant` | `campaign_member.role`, `.delegations` | every endpoint | `every_cell_of_plan_3_2_matches_grant` (parses the plan), `every_route_admits_exactly_the_roles_the_matrix_allows` |
| R5 | Invitations; preview reveals only what 9.6 allows; distinct failure reasons (9.6, J02) | `df_session::join_invitation::{JoinInvitation, JoinFailure}`, `InvitePreview` | `campaign_invitation` | `/app/join#token=`, `join/*` endpoints | `each_invitation_failure_has_its_own_reason`, `the_invite_preview_has_exactly_four_fields`, `failure_responses_show_no_campaign_details` |
| R6 | Room code is discovery plus approval, never authority over an existing seat (9.6, AUTH-01) | `df_auth::short_code::ShortCode`, `JoinRequest` | `join_code`, `join_request` | `join/code/*` | `a_room_code_always_ends_in_approval`, `a_room_code_never_resolves_to_an_existing_member` |
| R7 | Lobby: seats, character selection, readiness, microphone state (9.6, J02, UX-01) | `df_session::lobby::{StartBlocker, decide_start}`, `LobbyMember`, `MicrophoneState` | `lobby_member` | `/app/sessions/:id/lobby`, `sessions/:id/lobby` endpoints | `a_sponsored_player_reaches_ready_without_any_payment_step` |
| R8 | Display pairing: expiring one-time challenge, read-only device-bound ticket, several displays per session (9.6, 14.1, AUTH-02) | `df_session::display_pairing::*` | `display_pairing`, `display_ticket` | `/app/display`, `display/*` endpoints | `a_display_ticket_is_refused_by_every_non_public_route` |
| R9 | Dashboard: Resume, Join, Create (9.4) | `DashboardView`, `CampaignDirectory` | reads | `GET campaigns` | `an_account_with_no_campaigns_gets_the_plan_empty_state_view` |
| R10 | Leave or removal revokes seat and private access at once (J07, 15.4) | `campaign_event`, `MemberSnapshot`, `CampaignMembershipAuthority` | `campaign_member`, `campaign_event` | member endpoints | `a_removed_member_fails_the_next_batch_revalidation`, `a_consumer_never_sees_event_two_before_event_one_commits` |
| R11 | Host transfer and ownership transfer are separate two-party acts (J08, decision 11) | `TransferProposal` | `transfer_proposal` | settings endpoints | `a_host_transfer_leaves_the_owner_unchanged` |
| R12 | Archived campaign viewable and exportable (9.4) | `CampaignStatus::Archived` | `campaign` | `GET campaigns/:id` | `an_archived_campaign_refuses_a_lobby_but_opens_read_only` |
| R13 | Rate limits on invitations, room-code guessing, pairing (14.1) | `df_auth::keyed_throttle` | none (process memory) | `join/*`, `display/*`, invitations | `flooding_one_throttle_layer_does_not_lock_out_another` |
| R14 | Account closure cannot orphan a campaign (J14) | `CampaignStore::closure_campaigns`, `leave_all`, `schedule_archived_deletions` | `campaign`, `campaign_member` | called by B's close transaction | `closure_is_blocked_while_the_account_owns_or_hosts_an_active_campaign`, `accept_transfer_and_closure_never_deadlock` |

## 3. Crate layout and edges

Pure policy goes in the existing wasm-safe crates I own; the native facade is a new crate, as B did
with `df-customer`, because `df-portal` must stay free of databases and game types.

| Crate | Change | Direct project dependencies |
| --- | --- | --- |
| `df-types` (B holds the lock) | gains `clock` (`Clock`, `SystemClock` native-only, `FakeClock` behind a `test-fakes` feature) and a native-only `token` module (`RandomUnavailable`, `random_bytes`, `SecretToken`, `TokenDigest`, `tagged_digest`, the time-encoded `new_operation_id` and `operation_issued_at_ms`), moved from `df-customer` (request B-2). Native-only dependencies `getrandom`, `sha2` | none new project edges |
| `df-auth` (pure, wasm-safe) | add `roles.rs`, `short_code.rs`, `keyed_throttle.rs` (a pure bounded map over the existing `throttle` policy) | `df-types` only (unchanged) |
| `df-session` | add `join_invitation.rs`, `lobby.rs`, `display_pairing.rs` (pure, wasm-safe). `invitation.rs` and `InvitationFailure` stay untouched: `tests/invitation_contract.rs` pins them | unchanged |
| `df-campaign-wire` (**new**, pure serde, wasm-safe) | the JSON types P's wasm app deserializes: `DashboardView`, `OverviewView`, `CampaignView`, `MemberCard`, `InvitePreview`, `LobbyView`, `DisplayStatus`, request bodies, and the closed `Reason` enum carried in error responses. One definition shared by the server and the app (request C-1b); customer-visible types follow decision 26 | `serde` only |
| `df-campaign` (**new**, native) | the facade: service, consumer-owned ports, in-memory fakes (feature `test-fakes`), contract tests | `df-types`, `df-auth`, `df-session`, `df-campaign-wire`, `df-observe` |
| `df-persistence` | `campaign_store.rs` implementing `CampaignStore`, Postgres tests, migrations `0008_campaigns.sql` and `0009_lobby.sql` | adds `df-campaign` |
| `df-portal` | `src/campaigns/`: the JSON routes only. No HTML, no CSS, no database, no game types | adds `df-campaign` |
| `df-account-app` (P) | the screens; depends on `df-campaign-wire` for the types | adds `df-campaign-wire` |

No browser crate depends on `df-campaign`. It never runs on a game owner thread and never calls a
provider. It does not depend on `df-customer`.

**Mounting.** B mounts P's `app_api::router` at `/app/api/` and serves the bundle at `/app/`. I export
one function from `campaigns/mod.rs`, mounted by B beside it:

```text
pub struct CampaignApiState<S, M, C> { portal: PortalState<S, M>, campaigns: CampaignService<C>, ... }
pub fn campaign_api_router<S: AccountStore, M: EmailSender, C: CampaignStore>(
    state: CampaignApiState<S, M, C>,
) -> axum::Router        // routes under /app/api/campaigns, /join, /sessions, /display
```

It uses P's request guard (authenticated session via `CustomerService::authenticate`; for every POST the
`X-DF-CSRF` header, `Content-Type: application/json`, no cross-site `Sec-Fetch-Site`, and an `Origin`
equal to the configured one; anonymous routes use the pre-session CSRF token that `GET session` returns)
and P's response envelope (request P-2). A foreign id is `not_found`, the same as a missing one. Every
response is `Cache-Control: no-store`.

**The route table is the only way a route exists.** `campaigns/routes.rs` holds a single
`const ROUTES: &[RouteSpec]`; each `RouteSpec` has the method, the path template, an `AuthClass`
(`Public`, `Session`, `SessionVerified`, `Member`, `Delegated(Delegation)`, `Host`, `Owner`,
`DeviceCookie`), the **`Capability` the route exercises** (or `None` with a stated reason for the
non-matrix classes), the kinds of its path parameters (`CampaignId`, `SessionId`, `MemberId`,
`InvitationId`, `RequestId`, `TicketId`) and a `HandlerId` enum value. `campaign_api_router` builds the
`axum::Router` by iterating the table, and the handler for a `HandlerId` comes from one exhaustive
`match`, so a new handler variant without a table entry does not compile and a route cannot be
registered outside the table. The tests in section 12 iterate this table.

## 4. Pure policy (df-auth, df-session)

### 4.1 `df_auth::roles` (the 3.2 matrix, table-driven)

```text
enum CampaignRole { Host, CoHost, Player, Contributor, PairedDisplay }   // Support is separate (below)
enum Capability {
    InviteOrRemovePlayers, StartOrPauseSession, AllocateOwnCredit, RaiseAnotherPayersExposure,
    ViewSharedPotTotals, ViewOwnCardAndReceipts, EditOwnCharacterTemplate,
    RewriteCampaignInventory, ViewPrivateCharacterNotes, ExportCampaign, DeleteCampaign,
}
enum Grant {                       // one variant per distinct cell wording in the plan
    Yes, No, Never, Delegated, EmergencyPauseRequest, OptionalSummary, OwnOnly,
    ThroughLegalGameActions, AudienceFiltered, OwnPermittedView, FundingRecordsOnly,
    WithConfirmation, NotByDefault, LeaveOnly,
}
fn grant(role, capability) -> Grant                       // the plan cell, verbatim
enum Delegation { InvitePlayers, StartOrPauseSession, ExportCampaign }   // the three Delegated cells
struct Delegations(u8)             // per co-host, set by the host; the only delegation gate
fn authorize(role, delegations, capability) -> Result<Authorized, RoleRefusal>
```

`authorize` turns the cell into a decision: `Yes`; `OwnOnly` (the caller proves the object is theirs);
`ThroughLegalGameActions` (allowed only as a game command the rules accept; the role never edits
state); `AudienceFiltered`, `OwnPermittedView`, `FundingRecordsOnly` (allowed, with the projection
named so the exporter filters); `WithConfirmation` (host with the typed confirmation, see ownership
below); `Delegated` (allowed only when the matching `Delegation` bit is set on that member);
`EmergencyPauseRequest` (produces a request, never a pause); `OptionalSummary` (allowed when the
campaign's `display_summary_enabled` is true); and `No`, `Never`, `NotByDefault`, `LeaveOnly` as typed
refusals (`LeaveOnly` allows the leave action and refuses delete).

**One gate per delegation.** There is no campaign-level `invite_permission`. The bits are the three
Delegated cells of the matrix: `InvitePlayers` gates inviting and removing players, creating and
rotating codes and deciding join requests; `StartOrPauseSession` gates opening the lobby, starting,
pausing, **and approving a display pairing**; `ExportCampaign` gates export. The wizard's invitation
step sets `default_co_host_delegations`, applied when someone is promoted to co-host and editable per
co-host afterwards.

**Seats.** The host and co-hosts hold a seat by default and count toward `seat_count`; the wizard asks
"How many people will play, including you?". **Only the host and co-hosts may be seatless**: they can
step out of and back into their own seat (`set_seat`, a self route) to run the table without a character. For a player,
`set_seat` is a **role change**: a player becomes a contributor (freeing the seat, checked against
`maximum_contributors`) and a contributor becomes a player (taking a seat, checked against
`seat_count`), both under the campaign lock; a seatless player cannot exist, so `maximum_contributors`
bounds every seatless non-host. A contributor cannot be made a co-host (Q10, decided): only a seated
member can be promoted, and a test pins it.

**Ownership is separate from the host role (decision 11).** `Campaign.owner_account_id` holds
`OwnerAction { Archive, Delete, TransferOwnership }` (a pure `authorize_owner_action(is_owner, action)`
that also demands the typed campaign-name confirmation for archive and delete). The matrix cell
"Delete campaign: Host with confirmation" is the creator's case, since the creator is both owner and
host on day one; after a host transfer the new host does not hold the delete or archive right and the
owner keeps it whatever role they then have (Q9, decided). The matrix test pins the Host cell as
`WithConfirmation`; a separate test pins the ownership layer.

Support is not a campaign role. Section 3.2's Support column is operator access, which B and
`permissions.rs` already model. I add one pure function, `support_grant(capability) -> SupportAccess`,
naming what each cell permits (`AuditedAssistanceOnly`, `IncidentControl`, `NeverImpersonateConsent`,
`Denied`, `ScopedAudited`, `RedactedSupportData`, `RecoveryAssistance`, `ExceptionalAuditedCorrection`,
`BreakGlassJustified`, `BackupRoleSeparately`, `PolicyControlled`), so the column is pinned and an
operator surface cannot silently exceed it. It grants nothing; `permissions.rs` still decides.

Payment and campaign roles are independent (3.2 note): `CampaignRole` has no money state and
`AllocateOwnCredit` is a capability of every non-display role, including `Contributor`. A role change
never changes an allocation and allocating never changes a role.

**Rank table (who may remove or readmit whom).** Pure, in `roles`, tested as a table:

| Actor | May remove | May allow a rejoin of someone removed by |
| --- | --- | --- |
| Host | co-hosts, players, contributors | anyone (host rank is the highest) |
| Co-host with `InvitePlayers` | players, contributors | a co-host or lower, never a host |
| Everyone else | nobody | nobody |

Nobody removes the host, and **nobody removes the owner** (an owner who is not the host keeps
archive, delete and transfer rights and must keep access to the screens they act on); the only exits
for the owner are an ownership transfer followed by `leave`, or archive. A member removes themselves
only through `leave`. The member row records `removed_by_role`, and `allow_rejoin` is permitted only
when the caller's rank is the same as or higher than that role's.

### 4.2 `df_auth::short_code`

`ShortCode`: eight symbols from the Crockford base-32 alphabet (digits and letters without I, L, O, U),
40 bits, from caller-supplied entropy, shown as `ABCD-EFGH`. `parse` trims, removes dashes and spaces,
upper-cases and maps the look-alikes (`O` to `0`, `I`/`L` to `1`); it is a syntax normalization of a
code, not an interpretation of player words. The value has no `Debug`/`Display`, only `display_form()`.
It serves two uses: the room code (8.3), stored as a keyed domain-separated digest, and the display
pairing code (8.5), stored readable while pending.

Guessing arithmetic, because AUTH-01 depends on it: 2^40 is about 1.1e12. With a process ceiling of 600
lookups a minute (864,000 a day, tunable) and 1,000 live codes, the chance that a day of sustained
guessing finds any live code is about 8e-4; per-IP and per-account ceilings are far lower; codes live at
most 24 h; and a hit alone yields a join request for host approval, never a seat (8.3). The process
ceiling is in memory: it resets on restart and each replica has its own, so the effective ceiling is the
configured number times the replica count. An attacker can spend the process ceiling to deny code
lookups to everyone for a minute at a time; that is accepted because invitation links do not use it,
saturation raises an alert and a `dev campaigns` counter, and the 24 h life, the approval step and the
per-account limits are what bound the damage.

### 4.3 `df_auth::keyed_throttle`

A pure bounded map `KeyedWindows` over `throttle::{ThrottlePolicy, Window}` with a stated
`maximum_keys`. **It fails closed per instance.** On an insert at capacity it first sweeps windows that
have already elapsed (they carry no information); if the map is still full the new key is refused
(`Throttle::Saturated`, an audit line, a saturation counter in `dev campaigns`). It never evicts a live
window, so a flood of fresh keys cannot reset a real account's counter.

**One map per layer**, each with its own `maximum_keys`: lookup per IP, lookup per account, lookup
process-wide (a single key), invitation creation per account, display challenge per IP, display
approval per account. Flooding the per-IP lookup map with spoofed sources therefore cannot lock out
account-keyed layers or the invitation flow. Account keys are keyed hashes of the account id.

**Two IP keys per request, checked loose first.** Every IP-keyed layer counts each request against a
**loose key** (IPv6 /48, IPv4 /24) whose limits are **ten times** the tight limits (configurable;
coordinator ruling), and against a **tight key** (IPv6 /64, IPv4 /32) with the limits in section 11.
**The loose key is checked first, and a request the loose key refuses never inserts a tight key**, so a
single /48, which holds 65,536 /64 keys, can create at most the loose limit's worth of tight keys: with
loose limits ten times the tight ones, at most 100 tight keys per 15 minutes on lookups and 60 per
minute on display challenges. The tight map holds 262,144 keys by default and the loose map 65,536.
**Neither map is sized above what the loose limits could allow in a window** (65,536 loose keys times
100 tight keys each is 6.5 million): the maps are bounded on purpose and fail closed, and the figure
that matters is how many distinct source prefixes an attacker needs. A prefix (an IPv6 /48 or an IPv4
/24) creates at most its loose limit's worth of tight keys per window, so the tight map is filled by
`ceil(tight map size / loose limit)` prefixes: **2,622** on lookups (262,144 / 100), **874** on
invitation previews, display status and ticket calls (262,144 / 300) and **4,370** on display
challenges (262,144 / 60). The loose map is filled by 65,536 prefixes. **When a map is full, every new
key on that layer is refused until windows elapse**; existing keys keep working, and the process-wide
ceiling is untouched. That residual is real and is listed in the threat model (section 9): the weakest
layer falls to about 900 distinct sources in one window, which is a distributed attack the edge
answers (S2-1). Three guards keep the figures honest. The configuration is **refused at load**
(`TunablesRefusal::ThrottleMapsDoNotMatchLimits`) when a tight map is smaller than the loose map or
when the limits let fewer than 256 prefixes fill a tight map, so raising a limit cannot quietly make
the layer cheap to saturate. The dev view prints the figure per layer. And a refusal because a map was
full is an audit line carrying `alert=yes` on every layer, per-key windows included (the events in
`audit::ALERT_EVENTS`), which S2's alert rule fires on; ordinary throttles are not alerted. Keys are keyed hashes. The **client IP comes from B's
trusted-proxy extraction** (the configured proxy chain in `bootstrap-origin-policy.md`), never the
socket peer; behind the load balancer the peer is the same address for every request. If the extraction
yields no trusted client address, IP-keyed layers refuse the request rather than key it on the proxy.

### 4.4 `df_session::join_invitation`

`JoinInvitation`: expiry (exclusive), `max_uses`, `use_count`, `revoked`, `joins_as`
(`Player` | `Contributor`) and the creation time. `consume(member_state, now, seats_free,
contributors_free) -> Result<JoinUse, JoinFailure>` with `JoinFailure { Expired, Revoked, Exhausted,
Full, ContributorsFull, AlreadyJoined, NoLongerValid, CampaignUnavailable, AccountClosing }`. Fixed, tested order: an **active** member of
the campaign gets `AlreadyJoined` (also after expiry or exhaustion, so an exact retry stays
resolvable); a campaign that is not `active` (`draft`, `archived`, `deleting`) gives `CampaignUnavailable` (checked under the campaign lock for accept, approve and rejoin, so no member is added to an archived or deleting campaign); a joining account that is `closing` or `closed` gives `AccountClosing` (8.6); a `removed` member without `may_rejoin` gets `NoLongerValid` (the response does not reveal
the removal); then `Revoked`, `Expired`, `Exhausted`, then `Full` (player invitations, against the open
seats) or `ContributorsFull` (contributor invitations, against `maximum_contributors`, default 12). A
`left` member, or a `removed` member whose `may_rejoin` was set by someone of sufficient rank (4.1), may
use any currently valid invitation, and **joins in the invitation's `joins_as` role** (never a fixed
`player`). `max_uses = 1` is the one-shot case. `OneShotInvitation` is left as is. The unknown-token
case is a store miss and returns the same response as a malformed token.

### 4.5 `df_session::lobby`

`decide_start(snapshot) -> Result<StartDecision, Vec<StartBlocker>>`, `StartBlocker { NoSeatedMembers,
MemberNotReady, CharacterNotChosen, DisplayRequiredButUnpaired, FundingNotReady, SessionNotInLobby,
CampaignArchived }`. Only active members with `holds_seat` **whose `member_snapshot` is `authorized`**
are counted: a seated member whose account is `closing` or `closed` is excluded from the roster handed to
O1 and from the readiness counts, is shown in the lobby as "cannot play (account closing)", and Start does
not wait for them. Readiness counts come from
`df_auth::readiness::aggregate_current_readiness` (capacity = the campaign's `seat_count`). Funding
readiness is F's typed `start_readiness` result, never computed here. The lobby view lists every
blocker.

### 4.6 `df_session::display_pairing`

A device-authorization-style state machine (RFC 8628 shape) in pure types:

```text
enum PairingState { Pending, Approved, Redeemed, Cancelled, Expired }
struct DisplayPairing { user_code, device_secret_digest, device_label, created_at_ms, expires_at_ms, state, ... }
fn approve(&mut self, approver_may_pair: bool, session, now_ms)       // gate: host or StartOrPauseSession
fn redeem(&mut self, device_secret_digest, now_ms) -> Redeemed        // issues a ticket exactly once
fn reissue(&mut self, device_secret_digest, ticket_cookie_present, now_ms) -> Reissued   // lost response; once per redemption
struct DisplayTicket { ticket_digest, device_secret_digest, session, approved_by, expires_at_ms, ... }
fn ticket_valid(ticket, presented_device_digest, approver_authority, session_state, now_ms)
fn extend(ticket, presented_device_digest, approver_authority, session_state, now_ms) -> NewExpiry
```

Validity is **exclusive at the expiry instant**: valid while `now_ms < expires_at_ms`, refused at
`now_ms == expires_at_ms` (the convention of `MembershipRecord`). The ticket carries exactly one
capability, `ViewPublicProjection`, with no setter. It is bound (14.1) to the session, the campaign, the
role `PairedDisplay`, the approving member and the **device-secret digest**: presenting a ticket
without the matching device secret is refused. This protects a ticket that leaks **on its own**
(a log line, a screenshot, a header dump). It does not protect against theft of the whole cookie jar,
since both display cookies sit in the same browser; the threat model says so (section 9).

**Renewal extends the same ticket.** During the last `display_renewal_window` of its life (default one
hour), with the device secret, while the approver is still authorized, the session has not ended and
the pairing's `renewable_until` (approval time plus `display_pairing_maximum_hours`, default 24, never
past session end) has not passed, `extend` moves `expires_at_ms` forward (to the new ticket lifetime,
capped at `renewable_until`). The ticket id and digest do not change, so an already-open stream is not
closed by a renewal and the edge's per-batch verification simply sees the later expiry. Renewal at or
after the old expiry instant, outside the window, or past the cap is refused; after that the display
re-pairs. The display client (P's screen, or O1's once the TV is on the runtime page) calls `POST display/renew` (same origin, both cookies; 8.5 step 4), which also
refreshes the ticket cookie's `Max-Age`.

**Reissue after a lost redemption response.** If the response that carried the ticket cookie is lost,
the display holds the device cookie but no ticket. A `POST display/redeem` with the correct device
secret, **no valid ticket cookie**, and a pairing in `Redeemed` state within
`display_reissue_window_seconds` (default 60) of the redemption issues a new ticket and revokes the
previous one with reason `reissued`. **A redemption allows at most one reissue** (`reissue_count`
on the pairing goes 0 to 1 and never beyond), and the revoke and the new issue are **one transaction
under the campaign row lock**, so a reissue replaces one live ticket with one live ticket and cannot
exceed `maximum_displays_per_session`. After the window, after the one reissue, or when a valid
ticket cookie is present, no further ticket is issued.

**Where the cap of four is enforced.** At **redemption**, under the campaign row lock, counting live
(unrevoked, unexpired) tickets of the session; a redemption over the cap is refused with reason
`display_limit` and the pairing stays `Approved` until it expires, while the lobby shows "Display limit
reached; unpair one". **Pending, unapproved pairings do not count** (they carry no authority and are
bounded per IP). **Approval is also checked** against live tickets plus approved-but-unredeemed
pairings of the session, so a host cannot over-approve; redemption is the authoritative check.

Revocation reasons are a closed set: `unpaired`, `host_transferred`, `archived`, `session_ended`,
`approver_authority_lost`, `reissued`, `expired_cleanup`. Triggers:

- *Session end*: O1's runtime calls `revoke_display_tickets(session, session_ended)` on every end, and
  **the control plane calls it when a placement is released** (ruling (c), decision 29). **These writes
  are not gated by the owner fence** (13.1): revocation only removes access, so a stale placement cannot
  do harm by revoking. They are **scoped**, not open: a runtime role may revoke only tickets whose
  session is in its own `leased_table_sessions()` (row security on `display_ticket` follows O1's scope
  convention), so a runtime for campaign X is refused for campaign Y; the control-plane role, which has
  its own scope setting, may revoke for any campaign (it is how a released placement's tickets are
  revoked, since that placement's token no longer resolves). The caller's `OwnerFence`, when it has one,
  is recorded for audit only. Verification also treats a session that O1 reports as ended, or cannot
  read, as revoked, whatever the released placement's token resolves to.
- *Loss of the approver's delegation*: ticket validity re-evaluates the approver (an active member who
  is the host, or a co-host holding `StartOrPauseSession`) at verification time, and every store
  method that removes a member, clears the bit, demotes a co-host or transfers the host role revokes
  that member's approved tickets (`approver_authority_lost`) in the same transaction.
- *Host transfer revokes **every** display ticket of the session* (14.1: rotate on host transfer;
  coordinator decision), not only those the old host approved; displays re-pair under the new host.
- Unpair revokes one named ticket (or all of the session's); archive revokes every ticket of the
  campaign's sessions.

Every revoked ticket appends **its own** `display_ticket_revoked` event row carrying its ticket id.

## 5. The facade: `df-campaign`

Ports (consumer-owned, async, each call under a deadline; `Unknown` kept distinct from `Failed` as in
B's `StoreError`):

| Port | Implemented by | Purpose |
| --- | --- | --- |
| `CampaignStore` | `df-persistence` (`campaign_store.rs`), fake in `fakes.rs` | every table in section 6; each mutating method is one transaction that also appends its `campaign_event` rows |
| `AccountDirectory` | composition, over B | display name, verified-email flag, tenant and account status (read only, batched) |
| `SessionDirectory` | O1 | open the single lobby session (O1's `open_session`), read session state, start with a roster, request pause |
| `CharacterDirectory` | O2 | list an account's templates, instantiate a campaign character idempotently, list a campaign's characters, portrait handle |
| `SaveDirectory` | O1 | last durable save summary for cards |
| `FundingDirectory` | F | `start_readiness(session)` typed blockers, `open_session_funding(session, campaign, tenant)` called before O1's `Start`; and a one-way notification that a membership event exists. It has no cancel, release or reassign method |
| `AdventureCatalog` | content file, validated at load | the offered adventures, their ratings and whether setup generation is required |

`CampaignDirectory` is the trait P's `app_api` declares for the dashboard (`list_for_dashboard`); the
facade implements it, so P's `overview` can embed `DashboardView` without an extra request, and the same
view is served at `GET campaigns`.

Service (`CampaignService<C>`, cheap to clone; one owned retention task in a `JoinSet` stopped on the
shutdown signal):

- Campaigns: `start_draft`, `save_step`, `finish`, `list_for_dashboard`, `overview`, `update_settings`
  (a `seat_count` decrease is refused below the seats in use), `archive`, `unarchive`, `request_delete`
  `schedule_archived_deletions`, `cancel_scheduled_deletions`, and the chunked, resumable `leave_all(account, after_campaign_id)`. `request_delete` and `archive` refuse with `session_in_progress`.
  `schedule_archived_deletions`, `cancel_scheduled_deletions`, `leave_all`.
- Members: `create_invitation`, `revoke_invitation`, `preview_token`, `accept_token`, `preview_code`,
  `join_with_code`, `decide_join_request` (chooses the role), `leave`, `remove`, `allow_rejoin`,
  `set_co_host`, `set_delegations`, `set_seat` (self, host or co-host), `set_player_role`, `propose_transfer(kind)`, `accept_transfer`,
  `cancel_transfer`.
- Lobby: `open_lobby`, `choose_character`, `set_ready`, `set_microphone` (preference and the browser
  permission result, both from the member's own session), `start`; and the fenced write O1 makes,
  `session_started`.
- Display: `begin_display_pairing`, `lookup_display_code`, `approve_display_pairing`,
  `display_status`, `redeem_display_ticket`, `extend_display_ticket`, `verify_display_ticket`,
  `revoke_display_tickets`, `unpair_display`.
- Membership read: `member_snapshot(account, campaign)` (5.1).

### 5.1 Membership read, the event feed and the lag bound

**`MemberSnapshot`** is the one read the edge, O2, F and B use: `{ member_id, account_id, campaign_id,
tenant_id, role, holds_seat, delegations, status, authorized, member_revision, campaign_status, feed_epoch,
seat_count, event_seq }`. It is what O1's edge needs to mint a connection ticket and what the runtime
re-checks. **It is one SQL statement**: the member row, the campaign's `event_seq` and `feed_epoch` and the account's
status are read in a single statement and so one MVCC snapshot. Two reads would let an event commit
between them and be skipped by any consumer that starts from the snapshot's `event_seq`. **An account in
`closing` or `closed` status is not authorized** (`authorized = false`, reason `account_closing`): B's
session revocation does not close open runtime streams, and a closing account can last while an operator
refunds a balance. The account status comes from B's stable view (B-9) in the same statement.

`df_auth::membership::MembershipAuthority::read_current` is synchronous with `&mut self`, but the store
is async. The adapter loads first and checks second: `member_snapshot` is the one indexed read;
`CampaignService::authorize(request, now)` awaits it, wraps the snapshot in a small `SnapshotAuthority`
implementing the synchronous trait over that one record, and runs `authorize_membership` or
`revalidate_membership` on it, so the pure checks of `df-auth` remain the single implementation. A
failed or timed-out load (`membership_read_deadline_ms`, default 1000) is the trait's `Source` error and
**fails closed**. `Principal = AccountId`, `Tenant = TenantId` (the campaign's creation tenant),
`Campaign = CampaignId`, `Role = CampaignRole`, `Revision = u64`.

**Check-at-use is the guarantee** (decision 24). It is called **at connect**; **at each private asset
delivery or grant issue and renewal** (O2); **at the start of each publication batch**, once per batch
for the members the batch addresses (O1, on its stream layer's pooled connection, never on the owner
thread); and by F at payer admission. The staleness of a revoked member's access at the runtime is the
duration of one publication batch (`publication_batch_maximum_ms`, stated by O1), and a batch whose
membership read fails or times out does not start. **The feed does not carry that guarantee for O1**:
it makes stream closing prompt. **The lag rule binds only in-memory gates that serve from cached
authority without a fresh `member_snapshot` per use**, today O2's media gateway: such a gate stops
admitting new streams and stops releasing private bytes once its lag exceeds `feed_maximum_lag_seconds`,
until a poll reaches the head. Lag is **the time since the last successful poll that reached the head**
(a poll that returned no further changes), never "the age of the oldest unacknowledged event", which a
consumer that has stopped polling cannot measure. Any future consumer that caches authority joins the
rule.

**The feed is gapless per campaign.** `campaign.event_seq` is incremented with `UPDATE campaign SET
event_seq = event_seq + 1, last_event_at_ms = <primary clock> ... RETURNING event_seq` **inside the
same transaction as the change**, which takes the campaign row lock; `campaign_event` is keyed
(`campaign_id`, `event_seq`). Because the lock is held until commit, event N+1 cannot be assigned until
event N has committed or rolled back, so a reader of committed rows sees a contiguous prefix and can
never skip a lower number that commits later (a `bigserial` assigned at insert would). Every change that
adds an event takes the campaign lock, including joins and display revocations, so the lock is also the
serialization point for seat counts (6, 8.6). A consumer that finds a missing number waits for it (a
commit not yet visible) rather than skipping, with the deadline O1 states.

**Cursors.** Durable cursors exist **only for consumers with durable side effects**, and they live in
the consumer's own schema, never in `campaigns`: **F keeps its `funding` cursor in F's schema**
(`funding_feed_position`), **advanced in the same transaction as the effect** it acknowledges, with F's
effect rows **also unique on (`campaign_id`, `event_seq`)**, so a crash between an effect and an
acknowledgement cannot apply an event twice and a replay is a no-op. F's effect transaction therefore
locks only F rows (consistent with F's rule that no finance transaction locks an S1 row), and S1 has no
cursor table. **F does not gate admission on feed lag**: it reads a fresh `member_snapshot` at every
payer admission (decision 24's check-at-use), and uses the feed only to pause a leaver's allocation and
release unreserved credit promptly. **In-memory consumers** (the runtime's feed reader, the edge, O2's
gateway replicas) keep no durable cursor, because a shared cursor would let one replica acknowledge
events another has not seen: each starts, for every campaign it holds, from the `event_seq` of a **fresh
`MemberSnapshot` taken right after it subscribes** (the single-statement read above is what makes this
safe), keeps its position in memory, and re-snapshots a campaign on any gap or restart. **The snapshot carries `feed_epoch` from the same statement**: a consumer that stores an epoch with its position and finds a different `feed_epoch` in a fresh snapshot discards its old position and starts at the snapshot's `event_seq` on the new epoch, so a failover or restore cannot leave it comparing sequence numbers across epochs.

**Finding which campaigns changed: three reads, and only the first two are authoritative.**
Per-campaign sequences cannot be scanned as one stream, and a commit-time stamp cannot be trusted alone
(a commit can become visible long after its stamp while it waits for a synchronous standby, which
`transaction_timeout` may not interrupt; a clock can step back; a failover can reuse `event_seq`
numbers). So:

- **`campaign_heads(campaign_ids) -> [(campaign_id, event_seq, feed_epoch)]`** (a primary-key read
  of the campaigns a consumer holds) is how every **in-memory consumer** finds changes. It does not
  depend on any stamp: for each held campaign, if the head is above the consumer's position it reads
  the events above that position. It runs every `feed_poll_seconds` and **is the poll whose success
  defines lag** (below). This is also the read O2's gateway uses for its lag figure; one query covers
  every campaign it holds.
- **`campaign_heads_after(after_campaign_id, limit) -> [(campaign_id, event_seq, feed_epoch)]`**, a
  keyset on the primary key, is the **durable catch-up** read for a consumer with durable effects
  (F). At startup and every `feed_reconcile_seconds` (default 30) it sweeps all campaigns; a campaign it
  has never seen gets a position **only if no position row exists**: the sweep inserts with `ON CONFLICT
  DO NOTHING`, at the head read in the same statement as a fresh `member_snapshot` (F has no allocation
  there, so there is nothing to replay). **F creates a campaign's position row in the same transaction as
  its first payer grant for that campaign**, with the position taken from the snapshot that transaction
  read; the sweep can therefore never overwrite a position that a concurrent first grant just wrote, and
  cannot skip, say, a `left` event that committed after that grant's snapshot. A campaign it knows is
  replayed from its stored position. So events committed while F was down are applied once each.
- **`campaign_changes_since(lower_bound_ms, after_keyset)`** over the indexed (`last_event_at_ms`,
  `campaign_id`) is a **discovery hint** that lets a durable consumer react within seconds instead of
  at the next sweep. A missed hint costs at most `feed_reconcile_seconds`; it never costs an event. Its
  rules follow.
- **Regression, failover and epochs.** `campaign.feed_epoch` is a **128-bit value, not a counter**. The
  restore step of the runbook (S2) sets it to a **fresh random 128-bit value** for every campaign, in
  the same step that raises O1's `recovery_epoch`; "plus one" would give two restores from the same backup
  the same epoch and consumers on that epoch would see nothing change. **Failover rule:** a managed
  failover that loses committed transactions is detected by O1 (its 6.3 halts service when it detects
  the loss); the epoch is re-randomized **before service resumes**, so no consumer is serving on the old
  epoch once traffic returns. While the halt lasts, O2's gateway follows the same gate as every other
  caller of the fenced functions: it stops admitting and releasing private bytes, because its
  `campaign_heads` read fails or reports a different epoch, and it resumes only after it has taken fresh
  snapshots on the new epoch. A consumer that sees `event_seq` below its position, or a changed
  `feed_epoch`, **starts a new feed epoch for that campaign**: it drops cached authority and takes a
  fresh `member_snapshot`, and F re-derives that campaign's payer standing from the snapshot instead of
  trusting its stored position (and alerts). Event rows record the epoch they were written in, so a
  replayed row from a lost epoch cannot be mistaken for a current one.

For the hint read, the rules that make it complete are:

1. **Stamp and poll on the primary.** `last_event_at_ms` is the primary's `clock_timestamp()` taken
   inside the event transaction, and every poll reads the primary, never a replica (a replica's lag
   would break the bound below).
2. **The lower bound is the database start time of the previous poll that reached the head, minus
   `feed_overlap_seconds`**, where the start time is the primary's **`statement_timestamp()`** of the poll's own statement (not `clock_timestamp()` evaluated mid-statement, which is later than the snapshot and would eat the margin).
   Argument: a commit the previous poll could not see was in progress at that poll's snapshot, so it
   committed after the poll's start; its stamp was taken inside a transaction no longer than `T`, so the
   stamp is no earlier than (poll start minus `T`); the next poll reads from (poll start minus overlap),
   which is earlier whenever overlap exceeds `T`. Poll duration, a raised `feed_poll_seconds` and a slow
   poll do not matter, because the bound is anchored on the poll's start, not on the highest stamp seen.
   A consumer's first poll uses its subscription time minus the overlap. **A poll that spans several
   pages anchors on the start of the first page's statement**, and every page of that poll reads from
   that one lower bound; later pages run under later snapshots, so the start of a later page would not
   cover a commit that the first page's snapshot could not see.
3. **`T` is real, and the role default is the guarantee.** PostgreSQL's `transaction_timeout`
   (PostgreSQL 17 or later; the floor is 17, LAUNCH.md) starts its timer at the first statement of a
   transaction, and, as I recall `assign_transaction_timeout`, changing the setting inside a running
   transaction does not reschedule a timer that is already running. If a role or session default above
   `T` were in force, `SET LOCAL ... = T` would change what `current_setting` reports while the real
   deadline stayed the longer one, and a check built on `current_setting` would pass while the bound
   failed. So the design does not rely on `SET LOCAL` shortening anything:
   - **`ALTER ROLE <role> SET transaction_timeout = <T or less>` on every role that can bump
     `event_seq`**: S1's runtime role, O1's runtime and control-plane roles, B's role (it calls the
     closure methods), and any other role that calls an event-bumping method. A migration or the
     provisioning script sets it; the role default is in force from the first statement of every
     transaction on that role.
   - **A startup check refuses to start** unless the role's effective `transaction_timeout` is positive
     and at most `T`: each process reads its own role's entry from `pg_db_role_setting` (and the
     database-level entry, and `pg_settings` on a fresh connection) and compares in normalized
     milliseconds. `0` means disabled and counts as not set; units (`ms`, `s`, `min`) are normalized
     before comparing. The same check runs at load for the configuration (**`feed_overlap_seconds` must
     exceed `feed_transaction_timeout_seconds`**).
   - **`SET LOCAL transaction_timeout = T` at the start of each event-bumping method is the second
     guard**, for a role whose default was changed after startup.
   - A method called **inside a caller's transaction** (B's close transaction, 8.7) reads the effective
     value with `SHOW transaction_timeout` normalized to milliseconds and refuses with `TimeoutNotSet`
     unless it is positive and at most `T` (`0` is unset); B's role default makes that true.
   - **The timeout ends the whole session**, not only the transaction. Every pool that runs these
     methods (S1's, O1's, B's) discards a connection that reports a termination, and a termination
     that arrives **during `COMMIT`** is `Unknown`: the commit may or may not have happened, it is
     never reported as failure or success, and the caller looks the operation up by its idempotency key
     before repeating anything.

`LISTEN` on a `campaign_event` channel carries the campaign id and sequence as a wake-up hint only. The
test for a transaction that stamps at time `s` and commits at `s + 9.9 s` while polls continue is in
section 12.

**Lag bound.** In-memory consumers run `campaign_heads` every `feed_poll_seconds` (default 2). Lag is
**the time since the last successful `campaign_heads` read that covered every campaign the consumer
holds**; the maximum lag for a gating consumer is `feed_maximum_lag_seconds` (default 5). Each consumer's
lag is visible in `dev campaigns`, and exceeding the maximum is an alert as well as the admission stop.
A durable consumer that is down replays its backlog from its stored positions through
`campaign_heads_after` on restart. O2's gateway needs no cursor table for its lag figure: its
`campaign_heads` read covers every campaign it holds. Because the authoritative read is by primary key
and does not use stamps, the missed-commit risks above (a commit waiting for a standby, a clock step, a
failover) cannot hide an event from a held campaign for longer than one poll.

**Fencing by revision, and what `revoking` means.** Each event row carries `member_revision` (the
member's new revision) and a `revoking` flag. **`revoking` means "drop cached authority and re-check
against a fresh snapshot", not "deny"**: after `archived` a member still has read access (R12, 8.7), and
the fresh snapshot decides. Member-level rows: a consumer keeps `highest_revoking_revision[member]`; a
stream or capability opened at an older revision is closed or re-authorized once the feed shows a
higher revoking one, and processing the same event twice or out of order cannot reopen it.

**Row scope, and the rule each consumer applies.** Every event row has a `scope` and its `revoking`
flag is fixed by its kind, so no consumer has to guess what a row without a member means:

| Kind | `scope` | `revoking` | What a consumer does |
| --- | --- | --- | --- |
| `joined`, `rejoined`, `rejoin_allowed` | `member` | no | nothing for authority; B/F may note it |
| `left`, `removed` | `member` | yes | fence that member: `highest_revoking_revision[member]`, close that member's streams, re-authorize their grants |
| `role_changed`, `seat_changed`, `delegations_changed` | `member` | yes when the change lowers standing or clears a delegation, else no | as above when revoking |
| `host_transferred`, `ownership_transferred` | `member` (two rows: old holder revoking, new holder not) | old: yes; new: no | the old holder's grants re-authorize (this is how O2's `Owner { account }` grants lose that principal) |
| `display_ticket_revoked` | `ticket` (carries `ticket_id`, no member) | yes | close streams whose ticket id matches; **do not** re-authorize member or campaign grants |
| `archived`, `delete_requested`, `delete_scheduled` | `campaign` | yes | every grant records the snapshot `event_seq` at issue; a campaign-scope revoking row with a higher `event_seq` forces re-authorization of every grant on that campaign (members still read an archived campaign after the re-check) |
| `unarchived`, `delete_cancelled`, `invitations_revoked` | `campaign` | no | informational; no effect on authority |

**An unknown kind fails closed.** A consumer that reads an event kind it does not know treats the row as a
**campaign-scope revoking** row (every grant on that campaign re-authorizes against a fresh snapshot) and
raises an alert (`FEED unknown_kind`); it never drops the row and never ignores it, so a future revoking
kind added by S1 cannot be lost by an older consumer. A row that fails to parse at all is the same case.
A consumer never drops a row for lacking a member revision (only `member` rows have one). Migration 0008
adds a **CHECK constraint** on `campaign_event` tying `scope` to its columns: `scope = 'member'` requires
`member_id` and `member_revision` and no `ticket_id`; `scope = 'ticket'` requires `ticket_id` and no
`member_id`; `scope = 'campaign'` requires neither (and `kind` is constrained to the closed list), so a
malformed row cannot be written; a table test pins every kind against the CHECK. Processing the same row
twice or out of order cannot reopen access.

### 5.2 Idempotency

Create-campaign and accept-invitation carry an `operation` id (B's pattern, ids from
`df_types::token::new_operation_id`) recorded in `campaign_operation` with a fingerprint: a repeat
returns the first result (checked before expiry, so an accepted invitation stays resolvable), a changed
payload is `OperationConflict`, an id older than 90 days is `OperationExpired` and never executes.
Other mutations are state transitions guarded by the row `revision` (a stale request gets
`stale_revision` and the app reloads). A lost commit acknowledgement is `Unknown`, never shown as
failure.

Every mutating method takes the caller's `AccountId` from the authenticated session, never from a
request body; every query carries the account or campaign predicate from that identity **and, for child
ids, the campaign id as well** (a member, invitation, request or ticket id is looked up as a pair with
the campaign in the path, so a child of another campaign the same account also manages answers as
missing). Another account's id answers exactly as a missing id.

## 6. PostgreSQL schema (schema `campaigns` in the single database, migrations 0008 and 0009)

LAUNCH decision 7: one database, separate schemas; this is schema `campaigns`. **There are no foreign
keys to other owners' tables** (B's accounts and tenants, O1's sessions), as F does: a foreign key
takes `FOR KEY SHARE` on the referenced row when a campaigns row is written, which is a lock on an
accounts row taken after the campaign lock and would break the order below. Instead the ids are
**validated at creation** through the ports (`AccountDirectory` confirms the account exists, is
verified and has a tenant when a campaign is created; a joining or transferring account id comes from
the authenticated session or is read under the explicit `account_status` share lock of 8.6), and
`session_id` is validated by `SessionDirectory` and cleaned up by O1's `session_ended`. A plain
`JOIN` against B's status view inside `member_snapshot` is a read and takes no row lock. Conventions as B's `0005`: 16-byte ids with length checks,
times in milliseconds, parameterized SQL, append-only tables reject update and delete by trigger, each
migration one transaction, tables referenced as `campaigns.<name>`. **Lock order (decision 29):**
accounts and finance rows, then O1 directory and session rows, then `campaigns.campaign`, then campaign
child rows, then media. Every campaigns transaction follows it; no campaigns transaction afterwards
locks an accounts, finance or O1 row. **A fenced write** (one called by O1's runtime that sets state,
today `session_started`) begins with **`df_runtime.enter_service()` as its first statement**, then
`df_runtime.fence_current(tenant, campaign, recovery_epoch, claim_seq, placement_id, 'game')` (six
arguments, O1's committed 6.1), and only then locks `campaigns.campaign`; during a halt in progress
`enter_service()` or the fence raises `DF002` (`ServiceHalting`), which the store maps to the typed,
retryable `service_halting` refusal and writes nothing. **Member-creating paths** take the account row
first through B's definer function (8.6), then the campaign lock.

| Table | Key columns | Notes |
| --- | --- | --- |
| `campaign` | `campaign_id` pk, `tenant_id` (the creator's tenant at creation, immutable; an isolation key, not a payer; no foreign key), `owner_account_id` (delete, archive and ownership rights; no foreign key), `name` (1..80), `adventure_kind`, `rules_edition` (closed, `srd_5_2_1`), `content_rating` (closed), `seat_count` (seated members including the host), `room_codes` (`off`, `with_approval`), `default_co_host_delegations` (bitset), `funding_model` (`named_sponsor`, `shared_pot`), `sponsor_account_id` null, `voice_default`, `picture_default`, `spend_preset`, `custom_hourly_micro_dollars` null, `session_ceiling_micro_dollars` null, `display_summary_enabled`, `planned_session_at_ms` null, `status` (`draft`, `active`, `archived`, `deleting`), `draft_step`, `revision`, **`event_seq`** (gapless per-campaign counter, 5.1), **`feed_epoch`** (bumped by the restore and failover runbook, 5.1), **`last_event_at_ms`** (the primary's clock at the last event; indexed with `campaign_id`), `created_at_ms`, `archived_at_ms`, `scheduled_delete_at_ms` null (set by an owner's account closure, 8.7), `draft_expires_at_ms` | The wizard fills a `draft` row; `finish` flips it to `active`. Creation needs a verified creator, so a tenant always exists. The sponsor columns are an intention only: **F records the sponsor's consent** and Start stays blocked until F's `start_readiness` says so. Every path that grants or changes a seat, and every event-writing change, takes `SELECT ... FOR UPDATE` on this row. |
| `campaign_member` | `member_id` pk (`df_types::MemberId`), `campaign_id`, `account_id`, **unique (`campaign_id`, `account_id`)**, `role` (`host`, `co_host`, `player`, `contributor`), `holds_seat`, `delegations` bitset, `status` (`active`, `left`, `removed`), `may_rejoin`, `removed_by_role` null, `last_invitation_id` null, `joined_at_ms`, `ended_at_ms`, `revision` | One row per person per campaign for life: a returning person **reactivates the same row and `MemberId`** (revision plus one, role from the invitation's `joins_as` or the request's `approved_as`, delegations cleared, **`may_rejoin` cleared**, event `rejoined`). Partial unique index: one active host per campaign; a host swap **demotes the old host first** and promotes the new one in the same transaction, because the index is not deferrable. Check: a contributor never holds a seat. Seats in use = active rows with `holds_seat` (host and co-hosts included) and may never exceed `campaign.seat_count`; contributors in use may never exceed `maximum_contributors`. |
| `campaign_invitation` | `invitation_id` pk, `campaign_id`, `token_digest` unique, `created_by_member`, `joins_as` (`player`, `contributor`), `created_at_ms`, `expires_at_ms`, `max_uses`, `use_count`, `revoked_at_ms` | Token is 32 random bytes, base64url, shown once; only its domain-separated digest is stored. No acceptance table: `AlreadyJoined` is an active member row. Revoked in bulk when its creator loses the right to invite (8.6). |
| `join_code` | `code_digest` pk (keyed, domain-separated), `campaign_id`, `created_at_ms`, `expires_at_ms`, `revoked_at_ms` | One live code per campaign when `room_codes = with_approval`; rotating revokes the old one. |
| `join_request` | `request_id` pk, `campaign_id`, `account_id`, `state` (`pending`, `approved`, `denied`, `withdrawn`), `approved_as` null, `created_at_ms`, `decided_by_member`, `decided_at_ms` | Unique pending per (campaign, account); at most `pending_join_request_cap` pending per campaign (default 20). Every code path ends here. |
| `transfer_proposal` | `proposal_id` pk, `campaign_id`, `kind` (`host`, `ownership`), `from_member`, `to_member`, `to_account` (copied at proposal time, so acceptance can lock the account before the campaign), `old_host_becomes` (`player`, `co_host`) null, `campaign_revision`, `proposed_at_ms`, `expires_at_ms`, `state` | Accepted only by the target at the recorded campaign revision. A `host` proposal swaps roles and never touches `owner_account_id`; an `ownership` proposal changes only `owner_account_id`. Neither touches a payer grant. |
| `lobby_member` | pk (`session_id`, `member_id`), `character_choice` (`undecided`, `template`, `existing`), `character_ref` null, `ready`, `microphone_preference`, `microphone_report` (`unknown`, `granted`, `declined`, `unsupported`), `updated_at_ms`, `revision` | Ready is cleared when the character choice changes. **Removal and leave delete the member's `lobby_member` rows for sessions that have not ended** in the same transaction, and `decide_start` ignores rows of inactive members. |
| `display_pairing` | `pairing_id` pk, **`campaign_id` null until approval, then set**, `user_code` (readable while pending, null in a terminal state; unique among pending), `device_secret_digest`, `device_label` (closed: phone, tablet, desktop, unknown), `ip_key` (keyed /64 hash), `created_at_ms`, `expires_at_ms` (10 min), `state`, `session_id` null, `approved_by_member` null, `approved_at_ms`, `redeemed_at_ms`, `reissue_count` (0 or 1), `renewable_until_ms` null | Created by a POST from the display; approval sets `campaign_id` and `session_id` together. The user code is not a secret by itself (approval needs an authenticated delegated member, redemption needs the device secret), so it is stored readable so the status response can show it again. Pending rows per `ip_key` are capped. |
| `display_ticket` | `ticket_id` pk, `ticket_digest` unique, `pairing_id`, **`campaign_id`**, `session_id`, `device_secret_digest`, `device_label`, `approved_by_member`, `issued_at_ms`, `expires_at_ms`, `revoked_at_ms`, `revoke_reason` (closed, 4.6) | No capability column; a display ticket can only view the public projection. A session may have up to `maximum_displays_per_session` live tickets (default 4), enforced at redemption (4.6). `campaign_id` lets archive find every ticket of the campaign's sessions and lets an event take the campaign lock; the unpair route looks a ticket up by (`ticket_id`, `session_id`, `campaign_id`) and authorizes against that campaign's membership, since session ids carry no foreign key. |
| `campaign_event` | pk (`campaign_id`, `event_seq`), **`scope`** (`member`, `ticket`, `campaign`), `feed_epoch` (the 128-bit epoch the row was written in), `member_id` null, `member_revision` null, `revoking` bool, `at_ms`, `kind` (`joined`, `rejoined`, `left`, `removed`, `rejoin_allowed`, `role_changed`, `delegations_changed`, `seat_changed`, `invitations_revoked`, `host_transferred`, `ownership_transferred`, `archived`, `unarchived`, `delete_requested`, `delete_scheduled`, `delete_cancelled`, `display_ticket_revoked`), `actor_member` null, `ticket_id` null, `writer_fence` null (the caller's `OwnerFence`, recorded for audit on O1 writes, 13.1) | Append-only. One row per revoked ticket and one row per affected member of a transfer (5.1). Gapless per campaign. A CHECK constraint ties `scope` to its columns (5.1). **`scope` and the consumer rule for each kind** are in 5.1 (member rows carry a member and a revision; ticket rows carry a ticket id; campaign rows carry neither). |
| `campaign_operation` | pk (`namespace`, `account_id`, `operation_id`), `fingerprint`, `result_ref`, `created_at_ms` | Namespaces `create_campaign`, `accept_invitation`. 90-day retention, then a namespace tombstone, as B does. |

Retention task (one owned task, ticking every `retention_tick_seconds`): expired drafts (30 days),
expired invitations and codes (30 days after expiry), terminal display pairings (1 day), revoked tickets
(7 days), `campaign_operation` as above. **It also executes scheduled deletions** (8.7): for each campaign
with `status = 'archived'` and `scheduled_delete_at_ms <= now`, it takes the campaign lock, re-checks that
the date has passed, that `scheduled_delete_at_ms` is still set (it was not cancelled) and that the status
is still `archived`, then sets `status = 'deleting'`, clears nothing else, and appends `delete_requested`
(campaign-level, revoking) in that transaction; O2 purges from that event. `campaign_event` and
`campaign_member` are history and are not purged by it.

## 7. Screens (P) and JSON endpoints (S1)

**Screens in P's app that this design needs** (request P-1): `/app/campaigns` (the campaign dashboard:
Resume, Join, Create, with the plan's empty state "Your first adventure starts with a table. Invited
already? Enter a room code."), `/app/campaigns/new` (the five-step wizard), `/app/campaigns/:id`
(Overview, Sessions, Characters, Assets, Funding, Settings tabs), `/app/join` (invitation preview and
room-code entry), `/app/sessions/:id/lobby`, and `/app/display` (the TV: the pairing flow). The play and
display runtime screens `/sessions/:id/play` and `/sessions/:id/display` are O1's. Characters and assets
tabs link to O2's screens and the funding tab to F's, shown only when the target is mounted (`OverviewView`
lists the tabs that exist).

Legend: P public (anonymous, pre-session CSRF); S web session; V verified email; M active member;
D(bit) host or co-host holding that delegation; O campaign owner; DEV the existing loopback dev-token
guard; DC the display device cookie. All paths are under `/app/api/`. Every POST needs the guard of
section 3. Rate limits are keyed hashes (IPv6 by /64 and /48), one throttle map per layer (4.3).

| Method and path | Auth | Limit |
| --- | --- | --- |
| `GET campaigns` (`DashboardView`) | S | none |
| `POST campaigns/drafts`, `GET campaigns/drafts/:draft`, `PUT campaigns/drafts/:draft/steps/:n` (1..5), `POST campaigns/drafts/:draft/finish` | S + V | 20 saves/h per account |
| `GET campaigns/:id`, `/sessions` | M | none |
| `GET campaigns/:id/settings` | M (read-only for non-hosts) | none |
| `POST campaigns/:id/settings` | host | 30/h |
| `POST campaigns/:id/archive`, `/unarchive`, `/delete`, `/ownership` (propose) | O (the owner) AND `authorize_owner_action`, with the typed confirmation; refused with `session_in_progress` only while a session is past the lobby (`preparing` with funding open, or `playing`; a lobby does not block, 8.4) | 10/h |
| `POST campaigns/:id/invitations`, `/invitations/:invitation/revoke` | D(InvitePlayers) | 20/h per account |
| `POST campaigns/:id/code/rotate`, `/requests/:request/approve`, `/deny` | D(InvitePlayers) | 20/h |
| `POST join/preview` (token in the body) | P | tight and loose IP keys, 30/min |
| `POST join/hold` (sets the pending-join cookie), `GET join/pending` | P / S | 30/min per IP |
| `POST join/accept` | S | 10/h per account |
| `POST join/code/lookup`, `POST join/code/request` | S | 10/15 min per IP, 20/h per account, process ceiling |
| `POST campaigns/:id/members/:member/remove`, `/allow-rejoin` | D(InvitePlayers), with the rank rule in 4.1 | 30/h |
| `POST campaigns/:id/members/:member/role`, `/delegations` | host (`role` also changes a player to a contributor and back, 4.1) | 30/h |
| `POST campaigns/:id/seat` (self: a host or co-host steps out of or into their own seat) | M, role host or co-host | 30/h |
| `POST campaigns/:id/leave` | M (refused for the host and the owner, 8.6) | none |
| `POST campaigns/:id/host-transfer` (propose), `/transfer/accept`, `/transfer/cancel` | the host proposes a host transfer, the owner proposes an ownership transfer (`/ownership`, above), the target accepts | 5/h |
| `POST campaigns/:id/lobby` | D(StartOrPauseSession) (opens or resumes the lobby session) | 10/h |
| `GET sessions/:id/lobby` (`LobbyView`) | M | poll every `lobby_poll_seconds` |
| `POST sessions/:id/lobby/character`, `/ready`, `/unready`, `/microphone` | M | 60/h |
| `POST sessions/:id/lobby/start` | D(StartOrPauseSession) | 10/h |
| `POST sessions/:id/lobby/display/lookup`, `/approve` (needs the `pairing_id` returned by lookup) | D(StartOrPauseSession) | 10/15 min per account |
| `POST sessions/:id/lobby/display/:ticket/unpair`, `/display/unpair-all` | D(StartOrPauseSession) | 30/h |
| `POST display/challenge` | P + pre-session CSRF | 6/min and 5 pending per IP key (tight), 10 times that per loose key; no process-wide ceiling |
| `GET display/status` (no side effects) | DC | 30/min per IP key |
| `POST display/redeem` | DC (SameSite=Strict device cookie), no CSRF token: Origin and Sec-Fetch-Site checks, 8.5 step 4 | per address first (30/min per IP key, loose key first, one layer shared with renew), then 12/h per pairing, counted only for a pairing the store found |
| `POST display/renew` | DC + ticket cookie, no CSRF token: Origin and Sec-Fetch-Site checks; called by the display screen and by O1's display client | per address first (the layer shared with redeem), then 12/h per ticket, counted only for a ticket the store found and presented with its own device secret (a wrong device is refused without spending the window) |
| `GET /dev/campaigns`, `/dev/members/:campaign`, `/dev/invitations`, `/dev/lobby/:session`, `/dev/tickets` | DEV | none |

There is no ticket-minting endpoint (the edge mints connection tickets, 13.1) and no separate
microphone-report endpoint: the lobby screen asks for the browser's microphone permission itself and
sends the result with the member's own session to `lobby/microphone`. Closed `reason` values returned in
error responses (invitation expired, revoked, exhausted, full, contributors_full, already_joined,
no_longer_valid, display_limit, stale_revision, account_closing, ...) live in `df-campaign-wire`; the
envelope's `code` stays P's closed set.

**Session-id routes resolve their campaign first.** `sessions/:id/lobby*` carries a session id and no
campaign. The handler first asks `SessionDirectory::campaign_of(session)` (an O1 read, which takes no
campaigns lock and matches decision 29's order: O1 rows before campaigns), then calls `member_snapshot` for
that campaign and authorizes against it. A session that is unknown, or that belongs to a campaign the
caller is not an active member of, answers exactly as a missing id; approving a display or making a
lobby change also requires the session to be non-ended (reason `session_ended`). The confused-deputy test
covers these routes (section 12).

**Pause is not a campaign endpoint.** The player's emergency pause request and the host's in-session
pause go through O1's RPC. O1 authorizes them with the pure `authorize(role, delegations, capability)`
over `member_snapshot` (`EmergencyPauseRequest` for a player, `StartOrPauseSession` for the host or a
delegated co-host); S1 exports a helper `authorize_capability(account, campaign, capability)` for that
call (O1-7) and owns no route for it.

**Route tests state their expectation exactly.** Owner-class routes (archive, unarchive, delete,
ownership) expect "the caller is the owner AND `authorize_owner_action` allows it"; a host who is not the
owner is refused. That narrows the plan's "Host with confirmation" delete cell on purpose (decision 11).

## 8. Flows

### 8.1 Create wizard (9.5, J01)

Requires a verified email (decision 12). An unverified signed-in account gets `forbidden` with reason
`email_unverified`, and P's screen shows B's resend; drafts cannot be started; the host always has a
tenant. Joining is not gated this way (8.2).

Step 1: name, adventure kind (from the validated `AdventureCatalog`; at launch one bounded adventure),
supported rules (read-only text naming the standard 2024 rules subset), content rating (closed enum
from the adventure's allowed ratings; adult service, no explicit sexual content option exists), and
"How many people will play, including you?" (1 to `maximum_seats`, default 4). Step 2: "Choose my hero
in the lobby" (default) or one of my templates (O2). Step 3: voice default, picture default ("Use
existing art only" or automatic), spend preset and, for Custom, hourly and session ceilings. Step 4:
**co-host defaults** (the three delegation bits), how people join (invitation link only, or link plus a
room code that always needs approval), and the funding model: "I will sponsor this table" (default; a
named sponsor, here the creator) or "Shared pot". Choosing a sponsor records an intention; the
sponsor's consent is F's and the lobby shows "Waiting for the sponsor's funding approval" until F
reports it. Step 5: summary and "Creating a campaign is free. Nothing is charged until a session
starts."

Each step is a `PUT` that saves the draft; Back never loses input; returning resumes at the saved step
(`draft_step`). Create is idempotent by operation id. Preset labels and amounts come from F's preset
catalog with the "proposed pilot pricing" label until pricing is approved (and decision 26: only what a
table pays is shown). If an adventure requires AI setup generation, the campaign is created first and
free; setup runs as a separate quoted job shown on the Sessions tab, so a failed setup leaves a campaign
with a "setup not run" state, never a missing campaign or an unexplained charge.

### 8.2 Invitations and join (9.6, J02)

A host or a co-host with `InvitePlayers` creates a link (default lifetime 7 days, `max_uses` default the
open seats) as a **player** invitation or a **contributor** invitation. A contributor joins without a
seat, views pot totals and allocates their own credit, and may later become a player (a role change by
the host, bounded by the open seats, 4.1). Contributors are bounded by `maximum_contributors` (default
12) so a leaked contributor link cannot admit unlimited people who can see pot totals. The invitation
link is `<origin>/app/join#token=<token>`: **the token travels in the URL fragment, which no server,
proxy, access log or Referer ever sees**, and P's app strips it with `replaceState` as it does for
verification tokens. It is shown once with a copy field. (This replaces the earlier path token and the
edge-log request.) Three client-side rules keep the fragment from leaking (request P-3):

- **`/app/join` answers 200 directly.** A fragment is carried across HTTP redirects, so there is no
  sign-in redirect and no trailing-slash redirect on that path; the shell is served as is and the
  screen decides what to show. Otherwise the token would follow a redirect, possibly to another origin.
- **P's app strips the fragment before its router, client telemetry, panic or error reporting, or any
  `location.href` logging starts.** The first thing the entry point does is read the fragment into a
  local and call `replaceState`. A test proves that no client log line and no telemetry payload ever
  contains `token=`.
- **Residual:** the original URL, fragment included, stays in the browser's history database on a
  shared device even after `replaceState`. The token is shown once, lives seven days at most, and is
  revocable (the host can revoke it, and each use is counted); the threat model lists this.

`POST join/preview` takes the token in the body and returns the preview **only when the invitation is
currently usable**. `InvitePreview` is a closed four-field struct: campaign name, host display name,
scheduled time if present, and content rating information; no member list, notes, assets or finance.
**Every failure response omits the preview** and carries only a closed `reason`, because a host
revokes a link when it leaks and an expired, revoked or exhausted response must not tell the holder
what table it was: expired, revoked, exhausted, full (P's copy: "All seats are taken. Ask the host to
free a seat or to send you a contributor invitation, which needs no seat."), contributors_full,
already_joined (returned only to the signed-in member), no_longer_valid, and unknown or malformed (one
response, byte-identical).

An anonymous visitor to a usable invitation is offered sign in or sign up by P's app. `POST join/hold`
sets a host-only `__Host-df_join` cookie (`HttpOnly; Secure; SameSite=Lax`, 1 h) holding the token, so
the interrupted invitation survives sign in, sign up and verification without a `next=` parameter or any
token in JS-reachable storage (decision 28). After sign in, P's app (request P-3) calls `GET join/pending`,
which reads the cookie and returns the preview, and offers "You were invited to <campaign>. Join?".
`POST join/accept` is the join, and it follows the lock order for every path that creates a member: it
reads the joiner's account id from the session (no lock), takes the account share lock through B's
definer function (8.6), then locks the campaign row, then **re-verifies under the locks**: the campaign
is `active` (else `campaign_unavailable`), the account is not `closing` or `closed` (else
`account_closing`), and the invitation is still usable. Then it applies `JoinInvitation::consume`,
inserts the member or reactivates the same row, appends `joined` or `rejoined`, and clears the cookie (as
does every terminal reason). A signed-in user who wants a different account is shown the signed-in name
and a switch-account action; nothing is accepted silently. `require_verified_email_to_join` is
configuration, default false.

### 8.3 Room codes (6)

A code is discovery plus approval, and it **always** ends in approval by the host or a co-host with
`InvitePlayers` (plan 9.6; a seat taken without approval would see pot totals). There is no auto-accept
mode. With `room_codes = off` no code exists. With `with_approval` an authenticated user who enters a
valid code (`join/code/lookup`) gets the same four-field preview and can create a pending `join_request`
(`join/code/request`; at most `pending_join_request_cap` per campaign, beyond it reason `requests_full`);
the approver chooses the role (player within the seat count, or contributor within `maximum_contributors`)
and approval is the only path from a code to a member row. Approval uses the stored `account_id`, so it takes the same locks as accept (the account share lock, then the campaign lock, then re-verification) and refuses `account_closing` for a closed account and `campaign_unavailable` for a campaign that is not `active`. **A request from a removed account that has
no `may_rejoin` is refused at approval** with reason `allow_back_first`, never readmitted by approving a
request. In every case:

- A code never resolves to an existing member: an existing member who types it gets `already_joined`.
- There is no reclaim by code and no seat token. The local `ReclaimRoomRequest` and the derived room
  code are not enabled for hosted campaigns (O1 section 6.2 agrees).
- Seat recovery is the same authenticated account re-opening the campaign: the member row is keyed by
  account, so reconnect rebinds the same `MemberId` and character. A stranger with the code cannot reach
  it.
- Lookups pass `keyed_throttle` (per IP key, per account, process-wide); a refusal looks like an
  unknown code and is audited with a keyed IP hash.

### 8.4 Lobby (9.6, J02, J04)

`POST campaigns/:id/lobby` is available to the host and to a co-host with `StartOrPauseSession`. It
asks O1 (`open_session`) to open the campaign's single non-ended session in the lobby state,
idempotently. Other members' screens show "Waiting for the host to open the lobby" and poll. The
`LobbyView` carries each seated member's display name, chosen character, ready state, microphone state,
and the **list of displays**; the screen polls `GET sessions/:id/lobby` every `lobby_poll_seconds`.
Rules:

- A member picks a template (O2 instantiates the campaign character idempotently) or, on a resumed
  campaign, their existing campaign character; a returning owner is bound to their own character and
  nobody else can pick it.
- Ready is cleared when the character choice changes; Start is disabled while `decide_start` returns
  blockers, and each is shown.
- The funding card is F's. The lobby shows "No charge until the host starts, except explicitly approved
  setup jobs" when F's `start_readiness` says so; a sponsored player sees no card field anywhere.
- **Microphone.** The lobby screen is part of P's wasm app, so it can request the browser permission
  itself. The member's preference (ask, or type) and the permission result (`granted`, `declined`,
  `unsupported`) are sent together to `lobby/microphone` with the member's own web session and CSRF
  token; nothing about it needs a runtime binding or a fenced write (this removes the earlier
  `report_microphone` path through O1). `Declined`, `Unsupported` or a typed preference leaves typed
  play; capture state during play is the runtime's concern.
- Start: the facade checks `decide_start`, calls F's `open_session_funding` (finance first), then calls
  O1's `Start` through `SessionDirectory` with the roster of active, **authorized** members that hold
  seats; O1 re-checks authority from the database (O1 section 5.1). That order follows the lock order of
  decision 29 (finance, then O1 rows, then campaigns). After O1 begins the session it calls
  `session_started(session, fence)` (the fenced write of 6: `enter_service()`, then `fence_current(...,
  'game')`, then the campaign lock). **`session_started` refuses unless the campaign is `active`**
  (reason `campaign_unavailable`), and O1 ends the session on that refusal. Otherwise it freezes the
  lobby rows.
- **Archive and delete with a session.** `archive` and `request_delete` first read the campaign's session
  state through `SessionDirectory` (an O1 read, before the campaign lock, as decision 29 orders) and
  **refuse with `session_in_progress` only when the session is past the lobby**: `preparing` with
  funding open (paid work may be admitted), or `playing` (active, paused, settling). **A session that is
  still in `lobby`, or `preparing` with funding not open, does not block**: the owner, who may not be the
  host and cannot end a lobby, is never locked out of archive and delete by an idle lobby. The archive
  proceeds, and O1 ends any session still in `lobby` (or `preparing` without funding) when it sees the
  campaign-scope `archived` or `delete_requested` event (O1-8). A lobby session opened between the read
  and the lock is covered the same way: `open_lobby` refuses a campaign that is not `active`,
  `session_started` refuses a non-active campaign, and O1 ends the session on the event.
### 8.5 Display pairing (9.6, 14.1, AUTH-02)

Connection tickets are minted by O1's edge, not by this facade (13.1). The facade supplies the display
ticket, the device-bound verification, and the pairing. The TV side is P's `/app/display` screen.

1. The TV or laptop opens `/app/display`: a screen with a button "Show pairing code". It creates
   nothing.
2. The button calls `POST display/challenge` with the pre-session CSRF token, 6 per minute and 5 pending
   per tight IP key (10 times those per loose key). It creates a `display_pairing` with a `device_label`
   (from the same user-agent classification as B's session device labels), sets the **device secret** in
   `__Host-df_display_device` (`Secure; HttpOnly; SameSite=Strict; Path=/`, **`Max-Age` =
   `display_pairing_maximum_hours` in seconds**, because renewal needs it for as long as the pairing can
   be renewed), never in a URL or body, and returns the user code `ABCD-EFGH`. The screen polls
   `GET display/status`, which has no side effects.
3. A host or a co-host with `StartOrPauseSession`, authenticated, types the code in the lobby's "Pair a
   display" form. **Lookup does not approve.** It returns the pending display's creation time, device
   label and how long it has been waiting, and P's screen shows the warning "Only pair a screen you can
   see showing this code" and a Confirm button (a second call carrying the `pairing_id`). This is the
   defence against a host typing a code an attacker handed them (RFC 8628 phishing). Lookup and approval
   share the per-account limit.
4. When status reports `approved`, the display screen calls **`POST display/redeem`** (a POST, so the
   old "a GET performs the redemption" workaround is gone). **Redeem and renew do not use a CSRF token.**
   B's pre-session CSRF cookie lives one hour and a TV that sat idle, or one already streaming, no longer
   has a fresh one. They rely instead on: the `SameSite=Strict` device cookie (no cross-site request
   carries it); an `Origin` header that must be present and equal to the configured origin; a
   `Sec-Fetch-Site` header that, **when present, must be `same-origin`** (present with any other value is refused; **when absent the request is accepted only because `Origin`, `X-DF-Device` and the Strict device cookie are all present**, which keeps older TV browsers that do not send the header able to redeem; coordinator ruling);
   `Content-Type: application/json`; and a custom `X-DF-Device` header, which forces a CORS preflight
   that is never answered. A request failing any of them is refused with `csrf_refused` and changes
   nothing. (`display/challenge` creates an unauthenticated row and does use the pre-session token; the
   screen fetches a fresh one from `GET session` each time the button is pressed.) Redemption runs exactly once, issues the
   ticket, sets `__Host-df_display_ticket` (same attributes, **`Max-Age` = the ticket's remaining life in
   seconds**), and returns the session id; the screen then navigates to `/sessions/:id/display`. **If
   the response was lost**, repeating `display/redeem` follows the reissue rule of 4.6. Expired,
   cancelled, wrong or missing secret all return one neutral reason and **clear the device cookie**
   (`Max-Age=0`); every terminal state (cancelled, expired, unpaired, revoked) clears both display
   cookies.
5. **The ticket needs the device secret.** Verification demands both cookies (4.6).
6. **Reaching the runtime.** Same origin is final: the edge proxies `/sessions/:id/play`,
   `/sessions/:id/display` and the stream upgrade on the portal's public origin (O1 section 8.3), so the
   browser sends both display cookies to those paths automatically. O1's edge calls
   `verify_display_ticket(ticket, device_secret, session)` before minting a `Display` connection ticket.
   There is no cross-origin variant and no body-POST attach.
7. The ticket authorizes only `ViewPublicProjection`. No campaign endpoint accepts it (they need a web
   session), no funding endpoint accepts it (F), and it is not a player credential.
8. **Several displays.** A session may have up to `maximum_displays_per_session` displays (cap
   enforced at redemption under the campaign lock, 4.6). The `LobbyView` lists every pending pairing and
   every paired display with creation time and device label, each with its own Unpair, plus Unpair all.
   Unpairing revokes that ticket and clears nothing else; the ticket is looked up by (ticket, session,
   campaign) and authorized against that campaign's membership.
9. **Renewal and revocation.** `POST display/renew` extends the same ticket (4.6) and refreshes the
   cookie's `Max-Age`, with the checks of step 4 and both cookies. **The caller is O1's display client**,
   because the renewal window is the last hour of a 12-hour ticket, when the TV is on O1's
   `/sessions/:id/display` and not on P's `/app/display` screen; P's screen calls it too when it is the
   one on screen. The same client repeats `display/redeem` if it is ever sent back to the pairing flow. Revocation of an already-open stream goes through the feed: each revoked ticket has its
   own `display_ticket_revoked` row and O1 closes streams whose ticket id is revoked; the edge's
   per-batch verification is the backstop.

### 8.6 Leaving, removal, transfers (J07, J08)

Rank rules are in 4.1. Additionally:

- The **host cannot leave before a host transfer**; the **owner cannot leave before an ownership
  transfer**. The refusal says what is required. A creator who is both completes both. A host who
  wants to end the table with no one to hand to archives it (owner right).
- Removal sets `status = removed, may_rejoin = false, removed_by_role`, frees the seat, clears the
  member's open-session `lobby_member` rows, bumps the member revision and appends `removed` (revoking,
  with that revision) in one transaction. The next batch-start revalidation fails with `Revoked`.
- **Allowing someone back** is the explicit `allow-rejoin` action, permitted only to a caller whose
  rank is the same as or higher than the remover's. After it, any currently valid invitation, or an
  approved code request, **reactivates the same row and `MemberId`**, clears `may_rejoin`, and uses the
  invitation's `joins_as` (or the request's `approved_as`). An old link posted earlier cannot readmit
  someone who was not allowed back.
- **Links outlive neither their creator's right nor their seat.** When a co-host is removed, leaves,
  is demoted, or loses the `InvitePlayers` bit, every unexpired invitation they created is revoked
  and the room code is rotated **in the same transaction**, with one `invitations_revoked` event.
  A host transfer does the same for the displaced host unless they remain a co-host holding the bit.
- The player's character templates are untouched (O2); the campaign keeps the member id and role
  history without the account's name.

**Leaving or removal never cancels or reassigns admitted reservations or jobs.** This is a structural
guarantee: the `FundingDirectory` notification the facade calls has no cancel, release or reassign
method, so the facade cannot do it. What F does on the event (its `funding` consumer pauses the leaver's
allocation use and releases unreserved credit, J07) is F's, and in-flight work keeps its original hold
and payer attribution (POT-04, plan 5.5). The behavioural test lives with F's consumer; mine asserts
that the fake notification receives exactly the event and nothing else.

**Host transfer** (J08): the host proposes at a campaign revision to an active seated player or any active co-host (a co-host may be seatless);
the target accepts; the old host is demoted first, then the target promoted (the partial unique index
is not deferrable), the old host becomes a player or a co-host (proposer's choice, default player),
**two `host_transferred` rows are appended, one per member** (revoking for the old host, not revoking
for the new host, each with that member's bumped revision), **every display ticket of the session is
revoked** (one row each), and invitations the old host created are handled as above. It changes
`owner_account_id` **not at all** and no payer grant or allocation. **Ownership transfer** is a
separate proposal by the owner to an active **seated member or co-host** (a contributor cannot be the target of a proposal; the same rule is checked at proposal and at acceptance), accepted by the target at the recorded revision;
it changes only `owner_account_id` and appends two `ownership_transferred` rows (revoking for the old
owner, so grants whose principal was the owner are re-authorized; not revoking for the new). While a
host is disconnected, a co-host holding the relevant bits manages the lobby and invitations (J08); no
co-host can archive, delete or transfer anything.

**A seatless host is valid.** The host-transfer path explicitly allows the new host to hold no seat: thetarget may be a seatless co-host, and the closure hand-over of 8.7 may give the host role to an owner whois a contributor. The host and co-hosts are the only roles that may be seatless (4.1), so the new hostkeeps `holds_seat = false` until they take a seat through the self `/seat` route (bounded by`seat_count`). `decide_start` counts only seated members, so a seatless host can start a table of seatedplayers.

**The account lock, for every path that creates a member (and for `accept_transfer`).** It follows
decision 29. The paths are: `join/accept` (a new member or a rejoin), approving a join request (which
uses a stored `account_id`, possibly old), and `accept_transfer`. Each reads the relevant account id
without a lock, takes the **account share lock through B's definer function first**, then locks the
campaign row, then **re-verifies under the locks**: for a join, that the invitation or request is still
usable, the campaign is `active`, and the account is not `closing` or `closed` (else `account_closing`);
for `accept_transfer`, that the proposal is still `proposed`, not expired, at the recorded campaign
revision, the target is still an active seated member or co-host (the same rule as at proposal, for both kinds), the account is not `closing` or
`closed`, and, for an ownership proposal, the campaign is not scheduled for deletion (reasons
`stale_revision`, `account_closing`, `expired`, `deletion_scheduled`). Without this, an account that
closes after `leave_all` has run could still be added by an in-flight accept or by a host approving an
old request, and would hold a seat for good, since `member_snapshot` only returns `authorized = false`.
Taking the account first matches B's **completion** transaction (8.7 step 7), which updates the account
row and then locks campaigns through `leave_all`: with the old order (campaign, then account) an
acceptance and a completion that share a campaign would each hold one lock and wait for the other.
(B's earlier close steps 1 to 4 lock only the account and the campaigns of 8.7 step 4, so they are not the
cycle.) Proposals to a closing account are refused at proposal time.

**The definer function.** `FOR SHARE` on a row needs `UPDATE` privilege on its table, and S1's role must
not have write rights on B's rows. So S1 does not lock B's table itself: B provides a `SECURITY DEFINER`
function `accounts.lock_account_status(account_id) -> status` that takes the share lock and returns the
status, with `EXECUTE` granted to S1's role (request B-9). `member_snapshot` reads B's status through a
SELECT-only view, which locks nothing. The lock-order tests run **under S1's real role privileges**
(the test fixture creates the production roles and grants, and a test asserts S1's role has no
`UPDATE` on any accounts table).

**Multi-campaign lockers.** Every method that locks more than one campaign (`leave_all`,
`schedule_archived_deletions`, `cancel_scheduled_deletions`, and the proposal cancellation of 8.7) locks
them in `campaign_id` order, so two concurrent closures that share campaigns cannot deadlock either.

### 8.7 Account closure (J14)

Closing an account is **blocked while the account owns or hosts an `active` campaign** (coordinator
decision). The order inside **B's close transaction**, which is one transaction (B-8):

1. Lock the account row and set it to `closing` (this is the first lock, and the same row
   `accept_transfer` takes FOR SHARE, so a concurrent acceptance either commits first or sees
   `closing`).
2. Call `closure_campaigns(account)` in that transaction. It returns two lists: **blocking** campaigns
   (active, reason `Owns`, `Hosts`, or both), and **owned archived** campaigns.
3. **If there are blockers, roll back** the whole transaction; the account is not `closing`, and B's
   close response carries the blocking list so P's settings screen can show, for each blocker, a link
   to transfer hosting or ownership or to archive it (owner only), and for each owned archived campaign
   a link to transfer ownership or delete it. A user whose account is not `closing` can follow those
   links (a `closing` account's sessions are revoked and any login reopens it, which is why the check
   must not leave the account `closing` while blockers exist).
4. **If there are none, commit `closing` together with `schedule_archived_deletions(account,
   delete_after_ms)`** in the same transaction (B's closing window). That sets `scheduled_delete_at_ms`
   on each owned archived campaign that was not transferred or deleted, appends `delete_scheduled`
   (campaign-level, revoking, so grants re-check and the overview shows the notice), and B's notifier
   tells every member by notice and email that the campaign will be deleted on that date and that an
   export is available until then (O2's audience-filtered export). O2 purges at the date. Drafts owned
   by the account are deleted. **The same transaction cancels the account's outgoing transfer
   proposals** (host and ownership proposals it made and that are still `proposed`). **The lock set is
   the sorted union of two sets**: the owned archived campaigns, **and every campaign that has a
   `proposed` row made by the account** (a host's proposal in a campaign it hosts but does not own is in
   the second set only). All are locked in `campaign_id` order. A proposal made before closing therefore
   cannot be accepted afterwards and hand a new owner a campaign scheduled for deletion; as a second
   guard `accept_transfer` refuses an ownership proposal on a campaign with `scheduled_delete_at_ms` set.
5. **Reopening** (a login within the window) calls `cancel_scheduled_deletions(account)` **in the
   reopening transaction**, appending `delete_cancelled`; it refuses a campaign that is already
   `deleting` (reason `already_deleting`) and B tells the user that campaign has been removed. **The
   scheduled deletion is executed by the retention task** (6): at `scheduled_delete_at_ms` it takes the
   campaign lock, re-checks the date and that the schedule was not cancelled, and flips `archived` to
   `deleting` with a `delete_requested` event. A reopen at the boundary races it only through the
   campaign lock, so exactly one wins: either the schedule is cancelled first and the flip finds it
   cleared, or the flip happens first and the cancel is refused.
6. While the account is `closing`, `member_snapshot` returns `authorized = false` (5.1): its open runtime
   streams survive session revocation, so authorization must fail at the next check-at-use. A closing
   account also cannot be added to a campaign (`account_closing`, 8.6).
7. **Completion is chunked, resumable and abortable.** Each chunk is its own transaction and begins with
   B's statement that takes the account row. **Under that row lock, B checks that the account is still
   `closing`; if not (a reopening login landed between chunks), the chunk returns `Aborted`, changes
   nothing, and B stops the loop.** Chunks already completed stay completed: a reopened account
   simply is no longer a member of those campaigns, and its membership is rebuilt through invitations
   (the same as having left; the closure page said so before it began). The order stays account, then
   campaigns; B writes the account's final state in a last transaction after a chunk returns `Done`.
   `leave_all(account, after_campaign_id)` processes **at most `K` campaigns per transaction**
   (`leave_all_chunk`, default 20) in `campaign_id` order, each chunk under the 10 s
   `transaction_timeout`, and returns `Continue(last_campaign_id)`, `Done` or `Aborted`. B's completion
   loops until `Done` or `Aborted`, and after a crash or a stop it resumes from the stored
   `after_campaign_id`; the account stays `closing` until `Done`, so no new membership can appear
   meanwhile (8.6). Memberships are not capped, which is why one transaction per membership count would
   eventually exceed the timeout.
   **The blocker rule has no mid-way hole.** `closure_campaigns` is checked once, in the close
   transaction (steps 1 to 3); later chunks do not re-run it as a stop condition, because that could
   strand an account in `closing` with some campaigns already left. Instead the cases that could create
   a late blocker are closed at their source:
   - **`unarchive` is refused while the campaign's host account is not `active`** (reason
     `host_account_not_active`). The order is: read the host's account id **without a lock**, take the
     **account share lock through `accounts.lock_account_status`**, then lock the campaign row, then
     **re-verify under the locks** (the campaign is still `archived`, the host member is still the host,
     and the host account id is unchanged and still `active`); **the share lock is held until commit**, so
     B's close transaction (which updates the account row first) and `unarchive` serialize: either the
     unarchive commits first and the closing account is then a blocker in B's step 2, or the close
     transaction marks the account `closing` first and the unarchive is refused. **A host account that is
     missing or deleted counts as not active.** A test runs the race in both orders.
   - `open_lobby` and the host-transfer acceptance already refuse a closing account.

   A chunk that still finds the closing account as host of an **`active`** campaign (a state the rules
   above should make impossible) hands the host role to the owner as below and records a
   `closure_late_host` audit line. In each campaign the chunk:
   - **skips every campaign the closing account owns.** Those are handled by scheduled deletion (step 4):
     the closing account's membership and host role stay in a campaign that is about to be deleted, and
     no hand-over is made to itself;
   - leaves the account if it is only a player, contributor or non-host co-host;
   - and **if the account is the host of a campaign it does not own, passes the vacated host role to the
     owner**, with the same events as a host transfer (two member rows, the old host revoking, the owner
     not; a host swap demotes before it promotes). **The owner may be a contributor with no seat**: the
     host-transfer path allows a seatless host explicitly (the host and co-hosts are the only roles that
     may be seatless, 4.1), so the owner receives the host role without taking a seat and can take one
     later through the self `/seat` route. **If the owner is not active** (their account is `closing`,
     `closed`, missing or deleted, or their member row is no longer active), **the hand-over is skipped**:
     the closing account keeps the host row (it is unauthorized in `member_snapshot` anyway), the chunk
     writes a `closure_handover_skipped` audit line and raises an alert, and an operator resolves it. The
     owner is normally an active member (nobody removes the owner and the owner cannot leave before an
     ownership transfer), so the campaign is otherwise never left without a host and the owner can start a
     lobby after unarchiving.

No campaign is left dormant without an owner: it is transferred, deleted by the owner, or scheduled for
deletion with notice. Every method here runs under the lock order of decision 29 (accounts, then
campaigns, then children) and, when called inside B's transaction, requires B to have set
`transaction_timeout` (5.1).

## 9. Threat model

| Threat | Controls | Tests |
| --- | --- | --- |
| Invitation token guessing or reuse (AUTH-01) | 256-bit token in the URL fragment and request body only, digest at rest, per-IP and per-account limits, expiry, revocation, use count, byte-identical not-found response, failure responses without details | `an_unknown_token_and_a_malformed_token_return_the_same_response`, `a_reused_exhausted_invitation_never_adds_a_second_member`, `failure_responses_show_no_campaign_details` |
| Room-code guessing, code as authority (AUTH-01) | 40-bit code, three throttle layers, fail-closed per-layer maps, 24 h life, always approval, never maps to an existing member, pending request cap | `a_room_code_always_ends_in_approval`, `guessing_codes_hits_the_process_ceiling`, `the_pending_request_cap_refuses_the_twenty_first_request` |
| Seat takeover by a stranger or an old link | Seats keyed by account; removed accounts need `allow-rejoin` by sufficient rank; no seat tokens | `an_old_link_cannot_retake_a_removed_seat`, `a_cohost_cannot_allow_back_someone_the_host_removed` |
| Display ticket abuse (AUTH-02) | Distinct type and digest domain, one capability, device-secret binding, expiry, revocation, no campaign endpoint accepts it | `a_display_ticket_is_refused_by_every_non_public_route` |
| Leaked ticket | Needs the device secret as well, so a ticket leaked on its own (log, screenshot, header dump) is useless. **Not claimed:** protection against theft of the whole browser cookie jar, where both cookies leave together; mitigated by `HttpOnly`, `SameSite=Strict`, a 12 h life and per-display unpair | `a_ticket_without_its_device_secret_is_refused` |
| Pairing phishing (host types an attacker's code) | Lookup shows creation time and device label and needs a Confirm; per-display unpair | `approval_requires_the_lookup_step_and_returns_the_display_details` |
| Pairing flooding | Per-IP rate and pending caps on a tight (/64) and a loose (/48) key, loose key checked first, maps sized above one allocation's reach, trusted-proxy client IP, stale rows expire in 10 min; no process-wide ceiling so one source cannot block everyone. **Residual:** when a map is full (the tight map falls to about 874 to 4,370 distinct /48 or /24 prefixes in one window, the loose map to 65,536; 4.3) every new key on that layer is refused until windows elapse, and the saturated audit lines alert; and a botnet of many allocations can reach the 100000 pending valve. Both alert and are answered at the edge (S2-1) | `one_ip_cannot_exhaust_pairing_for_other_ips`, `one_ipv6_48_allocation_cannot_fill_the_tight_key_map`, `a_request_refused_by_the_loose_key_never_inserts_a_tight_key`, `a_full_loose_map_refuses_every_new_loose_key_and_leaves_existing_keys_working` |
| Lost redemption response | Reissue once within 60 s with the device secret and no ticket cookie, old ticket revoked, under the campaign lock | `a_lost_redemption_response_is_recovered_by_one_reissue` |
| Role escalation via requests | Role and delegations read from the member row only; `authorize` on every handler; revision-guarded updates | `every_route_admits_exactly_the_roles_the_matrix_allows` |
| Cross-account and confused-deputy access | Predicates from the session; child ids always looked up with their campaign; nonexistence-safe errors | `account_b_ids_answer_like_missing_ids_on_every_route`, `a_child_of_another_campaign_the_same_account_manages_answers_like_a_missing_id` |
| Token or code in logs | Invitation token only in a fragment and a request body, never a path or query; route templates only in spans; no logging of bodies; P strips the fragment before the router, telemetry or error reporting start | `spans_and_audit_lines_contain_no_token_or_code`, `no_client_log_or_telemetry_payload_contains_token_equals` |
| Invitation token in browser history | **Residual, accepted:** the original URL with its fragment stays in the history database of a shared browser even after `replaceState`; mitigated by showing the token once, a seven-day life, host revocation and counted uses. The fragment also follows HTTP redirects, so `/app/join` answers 200 directly | `app_join_answers_200_with_no_redirect` |
| CSRF against display redeem and renew | `SameSite=Strict` device cookie, `Origin` required and equal, `Sec-Fetch-Site`, when present, must be `same-origin` (absent is accepted only with Origin, X-DF-Device and the Strict cookie), JSON content type, custom `X-DF-Device` header forcing an unanswered preflight; no dependence on the one-hour pre-session cookie | `redeem_and_renew_do_not_depend_on_the_presession_csrf_cookie` |
| Race on the last seat or contributor slot | Campaign row lock on every seat-granting path; counts checked in the same transaction | `two_simultaneous_accepts_fill_exactly_the_open_seats` (forced interleaving) |
| Revoked member keeps access | Check-at-use (decision 24), event feed with member revisions, gapless per-campaign sequence | `a_removed_member_fails_the_next_batch_revalidation`, `a_consumer_never_sees_event_two_before_event_one_commits` |
| Closing account keeps open streams | `member_snapshot` is unauthorized for a closing account | `a_closing_account_is_not_authorized_by_member_snapshot` |
| Event missed by the changed-campaign poll | Primary clock stamps, primary-only polling, lower bound anchored on the start of the first page of the previous head-reaching poll, `ALTER ROLE ... SET transaction_timeout` on every role that bumps events with a startup check (`SET LOCAL` is the second guard; a termination during COMMIT is Unknown), overlap validated above it | `a_transaction_committing_9_9_seconds_after_its_stamp_is_still_read` |
| Event applied twice or skipped | F's cursor advances in the effect's transaction and the effect is unique on (campaign, event_seq); the snapshot reads member row, `event_seq` and account status in one statement | `a_crash_between_effect_and_cursor_cannot_apply_an_event_twice`, `a_snapshot_cannot_skip_an_event_that_commits_between_its_reads` |
| Link outlives its creator | Invitations revoked and code rotated when the creator loses the right | `removing_a_cohost_revokes_their_invitations_and_rotates_the_code` |
| Owner locked out of own screens | Nobody removes the owner; owner cannot leave before an ownership transfer | `nobody_can_remove_the_owner` |
| Stale runtime writes lobby state | State-setting writes (`session_started`) begin with `enter_service()`, then O1's six-argument `fence_current(..., 'game')`, then the campaign lock; a halt in progress gives a retryable `service_halting` and no write | `a_write_from_a_stale_placement_is_refused` (three cases), `a_fenced_write_during_a_halt_gets_the_retryable_refusal` |
| Runtime revokes another campaign's tickets | Revocation writes are unfenced but scoped: a runtime role only reaches tickets of its own `leased_table_sessions()`; the control-plane role reaches any; verification treats an ended or unreadable session as revoked | `a_runtime_for_campaign_x_is_refused_on_campaign_y`, `the_control_plane_role_can_revoke_for_a_released_placement` |
| Closed account holds a seat | Every member-creating path takes the account share lock (B definer function), then the campaign lock, then re-verifies; `decide_start` excludes unauthorized members | `a_join_cannot_interleave_with_the_completion_transaction_to_leave_a_closed_account_seated`, `approving_an_old_request_for_a_closed_account_is_refused`, `decide_start_excludes_unauthorized_members` |
| S1 role can write B's rows | S1 gets no UPDATE right; B's `SECURITY DEFINER` function takes the share lock | `s1_role_has_no_update_privilege_on_accounts`, `lock_order_tests_run_under_the_production_roles` |
| Join to an archived or deleting campaign | `CampaignUnavailable` under the campaign lock; `session_started` refuses a non-active campaign; archive and delete refuse while a session exists | `no_member_is_added_to_an_archived_or_deleting_campaign`, `session_started_refuses_a_non_active_campaign`, `archive_and_delete_refuse_with_session_in_progress` |
| Cached-authority gate keeps serving a revoked member | In-memory gates (O2's gateway) stop admitting and releasing bytes past the lag limit (lag = time since the last poll that reached the head) | `a_gating_consumer_stops_admitting_and_publishing_past_the_lag_limit` |
| Throttle reset or lockout by flooding | Per-layer fail-closed maps, tight and loose IP keys, trusted-proxy client IP; documented limits of an in-process ceiling (4.2) | `flooding_one_throttle_layer_does_not_lock_out_another` |
| Closure races a transfer | B's close transaction locks the account first; `accept_transfer` takes the account FOR SHARE first, then the campaign, then re-verifies; `leave_all` and the other multi-campaign lockers lock in campaign_id order and each chunk aborts if the account is no longer `closing`; closing cancels the account's outgoing proposals | `accept_transfer_and_the_completion_transaction_never_deadlock`, `two_concurrent_closures_sharing_two_campaigns_never_deadlock`, `closing_cancels_the_accounts_outgoing_transfer_proposals`, `accept_transfer_refuses_a_closing_account`, `a_chunk_aborts_when_the_account_was_reopened_between_chunks`, `unarchive_and_closure_race_in_both_orders` |
| Hostile text | Rendered by P's app as text, never HTML; server returns JSON only | `hostile_campaign_and_display_names_round_trip_as_plain_json_strings` |

## 10. Observability and dev CLI

Audit lines through `df_observe::record_audit_line`, one per event, closed verbs, ids and hashes only:

```text
CAMPAIGN <draft_started|step_saved|created|settings_changed|archived|unarchived|delete_requested|delete_scheduled|delete_cancelled|ownership_proposed|ownership_transferred|closure_blocked|refused> campaign=<id> actor=<account> reason=<closed> ms=<n>
INVITE <created|previewed|accepted|refused|revoked|revoked_with_creator> campaign=<id> invitation=<id> reason=<expired|revoked|exhausted|full|contributors_full|already_joined|no_longer_valid|unknown> ms=<n>
ROOMCODE <rotated|looked_up|refused|throttled|saturated|request_created|request_decided|request_cap> campaign=<id> reason=<closed> ip_hash=<keyed /64 or /48, truncated, throttled and saturated only>
MEMBER <joined|rejoined|left|removed|rejoin_allowed|role_changed|delegations_changed|seat_changed|host_transfer_proposed|host_transferred|transfer_cancelled> campaign=<id> member=<id> actor=<member|none>
LOBBY <opened|character_chosen|ready|unready|microphone|start_blocked|started|fence_refused> session=<id> member=<id> reason=<blocker>
DISPLAY <challenge_created|challenge_refused|looked_up|approved|redeemed|reissued|redeem_refused|extended|unpaired|ticket_revoked> session=<id|none> reason=<closed>
FEED <consumer_lag|consumer_behind|gap_detected|timeout_not_set|unknown_kind|epoch_changed|handover_skipped> consumer=<funding|runtime|assets> campaign=<id> lag_s=<n>
```

OTEL: one span per request named `campaigns.<route template>`; store calls record latency and outcome
class. Dev CLI reads (read-only, in the existing loopback `/dev/*` surface, merged by composition):
`dev campaigns` (counts by status, per-layer throttle saturation, per-consumer feed lag),
`dev members <campaign>` (roster, roles, delegations, seat and contributor counts against capacity),
`dev invitations` (live invitations, uses, expiry; digests only), `dev lobby <session>` (cards,
blockers, displays), `dev tickets` (live display tickets and pairings: ids, approver, label, expiry; no
secrets). A feature without its dev view is not done.

## 11. Configuration (`content/campaigns.json`, validated at load, per-knob fallback with an audit line)

| Knob | Default | Plan basis |
| --- | --- | --- |
| `maximum_seats` | 6 | 1.1 and LAUNCH decision 9; seats including the host. O1's runtime ceiling must be the same value or higher (13.1) |
| `default_seats` | 4 | wizard default |
| `maximum_contributors` | 12 | coordinator decision |
| `pending_join_request_cap` | 20 per campaign | coordinator decision |
| `campaigns_per_account`, `drafts_per_account`, `draft_retention_days` | 20, 5, 30 | abuse bounds |
| `invitation_lifetime_days`, `invitation_default_max_uses` | 7, open seats | 9.6 |
| `join_code_lifetime_hours` | 24 | 6 |
| `lookup_limit_per_ip`, `lookup_limit_per_account`, `lookup_limit_process_per_minute` | 10 per 15 min, 20 per hour, 600 per replica | 14.1 |
| `throttle_maximum_keys_tight`, `throttle_maximum_keys_loose`, `throttle_maximum_keys_account` | 262144, 65536 per IP-keyed layer; 65536 for account-keyed layers | fail-closed bound. Load refuses a tight map smaller than the loose map, and limits under which fewer than 256 prefixes fill a tight map (4.3) |
| `display_ticket_limit_per_ip_per_minute` | 30 (redeem and renew share it) | address layer in front of the per-pairing and per-ticket windows |
| `throttle_loose_multiplier` | 10 (loose key limits = 10 times the tight limits; coordinator ruling) | 14.1 |
| `display_challenge_minutes`, `display_ticket_hours` | 10, 12 | 9.6 |
| `display_renewal_window_minutes`, `display_pairing_maximum_hours`, `display_reissue_window_seconds` | 60, 24, 60 | 14.1 |
| `display_challenge_limit_per_ip_per_minute`, `display_pending_per_ip_maximum`, `display_pending_hard_limit` | 6, 5, 100000 (an operational valve that raises an alert; not a throttle) | flood bounds |
| `maximum_displays_per_session` | 4 | multi-display |
| `membership_read_deadline_ms` | 1000 | fail closed |
| `feed_poll_seconds`, `feed_maximum_lag_seconds`, `feed_reconcile_seconds` | 2, 5, 30 (the durable catch-up sweep) | 5.1 |
| `leave_all_chunk` | 20 campaigns per transaction | 8.7 |
| `retention_tick_seconds` | 30 | 6 |
| `feed_transaction_timeout_seconds`, `feed_overlap_seconds` | 10, 15; **load fails unless the overlap exceeds the timeout, and startup fails unless every event-bumping role has a positive `transaction_timeout` at most the timeout** | 5.1 |
| `closure_archived_delete_notice` | B's closing window (30 days), minimum 14 days | 8.7 |
| `lobby_poll_seconds` | 5 | 11 UX |
| `require_verified_email_to_join` | false | 9.3 |
| `transfer_expiry_hours` | 48 | J08 |

## 12. Test plan per round

All tests are sentence-named, use `FakeClock`, injected entropy, the in-memory store and fake
directories; none sleeps or touches a network. Store contract tests run against the fake and, as
required tests, against the coordinator's scratch PostgreSQL (version 17 or later, the floor in LAUNCH.md; one fresh schema per test). API tests
use `tower::ServiceExt::oneshot` and assert JSON bodies. A round is not done with those skipped.

**Tests that would fail before the fix.**

- *Matrix from the plan.* `every_cell_of_plan_3_2_matches_grant` reads `planning/master-launch-plan.md`
  with `include_str!`, finds `## 3.2 Role matrix`, parses the table, and compares every cell (11 rows
  by 5 role columns, plus Support against `support_grant`) with `grant(role, capability)`. A test-local
  map turns each row label, column header and cell text into the enum; an unknown one **fails the
  test**, so editing the plan without the code is caught.
- *Routes admit exactly the matrix.* `every_route_admits_exactly_the_roles_the_matrix_allows` iterates
  `ROUTES`; for every route with a `Capability` it runs the real handler as each fixture identity (host,
  co-host with and without the relevant bit, player, contributor, non-member, paired display) and
  asserts the response is allowed if and only if `authorize(role, delegations, capability)` allows it,
  with the store unchanged on refusal. Routes with no capability carry an explicit expectation in the
  table, so the test cannot skip one silently.
- *Display ticket on every non-public route, reads included.*
  `a_display_ticket_is_refused_by_every_non_public_route` iterates **every non-`Public` route, GET as
  well as POST** (the "secret view" half of AUTH-02), filling path parameters from a seeded fixture,
  presenting a valid ticket and device cookie, and asserting a refusal, no private content in the body
  (seeded markers absent) and an unchanged store. `unregistered_paths_and_methods_are_not_served` probes
  outside the table.
- *Cross-account and confused deputy.* `account_b_ids_answer_like_missing_ids_on_every_route` iterates
  `ROUTES` with B's ids and random ids and asserts identical responses and unchanged rows.
  `a_child_of_another_campaign_the_same_account_manages_answers_like_a_missing_id`: one account hosts
  campaigns X and Y; each id-taking route is called with X in the path and a child (member, invitation,
  request, ticket) of Y, and must answer as missing.
- *Preview.* `the_invite_preview_has_exactly_four_fields` destructures `InvitePreview` exhaustively (a
  fifth field does not compile) and serializes the usable-invitation response for a campaign seeded
  with member names, notes and finance markers, asserting none appear.
  `failure_responses_show_no_campaign_details` serializes the expired, revoked, exhausted, full and
  contributors-full responses for a seeded campaign and asserts the campaign name and host name are
  absent.
- *Event feed ordering.* `a_consumer_never_sees_event_two_before_event_one_commits`: transaction 1 takes
  the campaign lock and assigns `event_seq` 1 but has not committed; transaction 2 is started and must
  block on the lock (asserted through the lock wait, not a sleep); a reader polling between sees nothing;
  transaction 1 commits, then transaction 2 gets `event_seq` 2 and commits; the reader sees exactly
  [1, 2] in order. It is written for the fake store's lock model and again against Postgres with two
  connections; a variant with a `bigserial` column fails this test, which is what it pins. Rollback of
  transaction 1 leaves no gap.
- *Keyset polling completeness.* `a_transaction_committing_9_9_seconds_after_its_stamp_is_still_read`:
  with a fake clock, a transaction stamps `last_event_at_ms` at `s` and commits at `s + 9.9 s` while polls
  every 2 s continue (each poll anchored on the previous head-reaching poll's start time minus the 15 s
  overlap); the next poll after the commit reads it. Variants: `a_slow_poll_does_not_shrink_the_overlap`
  (a poll that takes 8 s, then the next poll still covers it), `a_raised_poll_interval_still_covers_a_late_commit`,
  `the_poll_never_reads_a_replica` (the connection used is the primary pool, asserted by type), and
  `configuration_is_refused_when_the_overlap_does_not_exceed_the_transaction_timeout`.
  `a_multi_page_poll_anchors_on_the_start_of_its_first_page` (a commit that the first page's snapshot
  could not see, stamped before a later page's start, is read by the next poll).
  *Timeout contract tests* (Postgres, required, because a fake clock cannot see them):
  `a_role_default_of_60_seconds_with_set_local_10_seconds_is_refused_at_startup` (the startup check reads
  `pg_db_role_setting` and `pg_settings`, normalizes units, treats `0` as unset, and refuses to start),
  `set_local_does_not_shorten_a_timer_already_running_on_the_connection` (a measurement of the real
  behaviour on the target server: it holds a 60 s role default, issues `SET LOCAL transaction_timeout =
  10s` and records when the session is terminated; the design does not depend on the outcome, it exists
  so a changed server behaviour is noticed), `a_role_default_of_10_seconds_ends_a_longer_transaction_and_the_pool_discards_the_connection`,
  `a_termination_during_commit_is_reported_as_unknown`,
  `every_event_bumping_method_sets_the_transaction_timeout` (the second guard), and
  `a_method_called_in_a_callers_transaction_with_a_zero_or_larger_timeout_is_refused_with_timeout_not_set`
  (`0` and `60s` and `1min` are all refused; `10s` and `10000ms` are accepted).
- *Snapshot and funding atomicity.* `a_snapshot_cannot_skip_an_event_that_commits_between_its_reads`
  (the single-statement read, with a forced interleaving: an event commits between a two-statement read's
  statements and the single-statement version cannot return a member row older than its `event_seq`).
  `a_crash_between_effect_and_cursor_cannot_apply_an_event_twice` uses the reference durable consumer
  (the shape F implements): the effect and the cursor are one transaction, a crash injected between them
  is rolled back as a unit, and a replay of the same (campaign, event_seq) is a unique-key no-op.
  `an_in_memory_consumer_starts_from_the_snapshot_event_seq` and `two_replicas_never_share_an_acknowledgement`.
- *Forced interleaving for seats.* `two_simultaneous_accepts_fill_exactly_the_open_seats` holds
  transaction 1 open after it has locked the campaign row and counted seats, asserts transaction 2
  blocks on the lock, releases transaction 1, and asserts exactly the open seats are filled; the same
  harness covers approve-request, `set_seat`, rejoin, and a `seat_count` decrease below the seats in use
  (refused).
- *Expiry edges.* `a_display_ticket_is_valid_one_millisecond_before_expiry_and_refused_at_expiry`,
  `renewal_at_exactly_the_expiry_instant_is_refused`,
  `renewal_inside_the_window_extends_the_same_ticket_without_closing_its_stream`,
  `renewal_past_the_pairing_cap_or_session_end_is_refused`, and the same boundary for invitations and
  join codes.
- *Cookies.* `display_cookies_carry_the_documented_max_age_and_are_cleared_at_terminal_states` asserts
  the device cookie's `Max-Age` equals `display_pairing_maximum_hours`, the ticket cookie's equals the
  remaining life, and `Max-Age=0` on cancelled, expired, unpaired and revoked pairings.
- *Reissue.* `a_lost_redemption_response_is_recovered_by_one_reissue` redeems, drops the response,
  repeats `display/redeem` with the device cookie only inside the window (one new ticket, old revoked
  `reissued`), a second repeat with the new cookie present issues nothing, and a repeat after the window
  issues nothing.
- *Revocation of an open stream.* `an_open_display_stream_is_closed_by_the_revocation_event` drives a fake
  runtime stream registry that consumes the feed with the member-revision fence, for unpair, session
  end, placement release, the approver losing the delegation, the approver's removal, host transfer (all
  tickets of the session) and archive; it asserts one `display_ticket_revoked` row per ticket and that
  the batch-start revalidation closes the stream when the feed is withheld.
- *Fenced writes.* `a_write_from_a_stale_placement_is_refused`: `session_started` carrying an old
  `claim_seq` is refused with no change (the fake directory models `enter_service()` and the six-argument
  `fence_current`). Extra cases: `a_write_with_the_current_claim_seq_but_another_placement_is_refused`
  (a claim lost on failover), `a_write_after_a_raised_recovery_epoch_is_refused` (a restore), and
  `a_fenced_write_during_a_halt_gets_the_retryable_refusal` (a halt in progress: the write raises
  `DF002`, the store returns `service_halting`, nothing is written, and the same call succeeds after the
  halt). The Postgres contract version checks the statement order (`enter_service()`, then the fence,
  then the campaign row) with a concurrent claim and a concurrent halt.
- *Unfenced, scoped revocation.* `a_runtime_for_campaign_x_is_refused_on_campaign_y` (row security on
  `display_ticket` with `leased_table_sessions()`), `a_runtime_for_campaign_x_can_revoke_its_own_tickets_without_a_fence`
  (the fence is recorded in `writer_fence` for audit), `the_control_plane_role_can_revoke_for_a_released_placement`,
  and `a_released_placements_token_cannot_revoke_but_verification_treats_the_ended_session_as_revoked`.
- *Gating gates stop at the lag limit.* `a_gating_consumer_stops_admitting_and_publishing_past_the_lag_limit`
  drives the reference gate used for O2's gateway: lag is the time since the last poll that reached the
  head; at `feed_maximum_lag_seconds` it refuses new streams and withholds private bytes, and it resumes
  after a poll reaches the head. A gate that reads `member_snapshot` per use (O1) is not subject to it.
- *Feed completeness beyond stamps (round 0g).* `a_commit_that_becomes_visible_long_after_its_stamp_is_found_by_the_head_read`
  (the stamp is at `s`, visibility at `s + 60 s` as under a synchronous-standby wait: the stamp-based hint
  misses it, `campaign_heads` for a held campaign finds it within one poll, and F's sweep within
  `feed_reconcile_seconds`), `a_clock_step_back_cannot_hide_an_event_from_a_held_campaign`,
  `a_regression_of_event_seq_starts_a_new_epoch_and_resnapshots`,
  `a_feed_epoch_bump_resnapshots_even_when_event_seq_increases` (a failover that reuses numbers),
  `the_hint_poll_anchors_on_statement_timestamp_not_clock_timestamp`, and lag counts only a successful
  `campaign_heads` read that covered every held campaign (`a_failed_head_read_does_not_reset_the_lag_clock`).
- *Durable catch-up.* `a_durable_consumer_down_across_three_events_applies_each_exactly_once`
  (the reference consumer in F's shape: three events commit while it is down, it restarts, runs
  `campaign_heads_after` and replays from its stored position; each effect applied once, the unique key
  absorbs a repeat), `an_unseen_campaign_gets_a_position_at_its_head_in_the_snapshot_statement`, and
  `campaign_heads_after_pages_by_primary_key_without_skipping`.
- *Row scope.* `every_event_kind_has_a_scope_and_a_fixed_revoking_flag` (a table test over all kinds),
  `a_ticket_scope_row_closes_the_matching_stream_and_reauthorizes_no_grant`, and
  `a_campaign_scope_non_revoking_row_has_no_effect_on_authority`.
- *Session-id routes.* `a_session_of_another_campaign_the_same_account_manages_answers_like_a_missing_id`,
  `an_unknown_session_answers_like_a_missing_id`, `an_ended_session_refuses_display_approval_and_lobby_changes`
  (extending the confused-deputy test over `sessions/:id/lobby*`).
- *Seats and roles.* `a_cohost_can_leave_and_retake_their_own_seat_through_the_self_route` and
  `a_player_cannot_use_the_self_seat_route`. Owner-class routes: `archive_delete_and_ownership_expect_owner_and_authorize`
  (a host who is not the owner is refused; the owner who is no longer host is allowed).
  `emergency_pause_and_in_session_pause_are_authorized_by_authorize_over_member_snapshot` (the helper
  `authorize_capability` O1 calls; there is no campaign route for either).
- *Closure follow-ups (round 0g).* `a_host_proposal_in_a_campaign_it_does_not_own_is_cancelled_by_closure`
  (the lock set is the sorted union of owned-archived campaigns and campaigns with the account's
  `proposed` rows), `leave_all_passes_a_vacated_host_role_to_the_owner_with_transfer_events`,
  `leave_all_chunks_resume_after_a_stop`, `a_chunk_aborts_when_the_account_was_reopened_between_chunks` (a reopening login lands between chunk 1 and chunk 2: chunk 2 sees the account no longer `closing` under the row lock, returns `Aborted`, changes nothing, and B stops),
  `a_closing_account_cannot_be_added_to_any_campaign_while_chunks_are_pending`,
  `scheduled_deletion_is_executed_by_the_retention_task_under_the_campaign_lock`,
  `a_cancel_at_exactly_the_scheduled_instant_and_the_flip_are_serialized` (both orders under the lock; never
  both outcomes), `cancelling_a_campaign_already_deleting_is_refused`.
- *Campaign state.* `no_member_is_added_to_an_archived_or_deleting_campaign` (accept, approve, rejoin),
  `session_started_refuses_a_non_active_campaign`, `archive_and_delete_refuse_with_session_in_progress_only_past_the_lobby` (preparing with funding open and playing refuse; lobby and preparing without funding proceed and O1 ends them on the event), `an_owner_who_is_not_the_host_can_archive_over_an_idle_lobby`,
  `open_lobby_refuses_a_campaign_that_is_not_active`.
- *Sec-Fetch-Site absence.* `redeem_and_renew_accept_an_absent_sec_fetch_site_when_origin_x_df_device_and_the_strict_cookie_are_present`
  and `redeem_and_renew_refuse_a_present_sec_fetch_site_that_is_not_same_origin` (with the user agents of
  the supported-browser list in the fixture, including one that omits the header).
- *Epochs and positions (round 0h).* `two_restores_from_one_backup_produce_two_different_feed_epochs`
  (the restore step draws a fresh random 128-bit epoch each time; a counter would give the same value
  twice), `a_failover_that_loses_commits_re_randomizes_the_epoch_before_service_resumes`,
  `the_gateway_stops_releasing_bytes_while_the_epoch_changes_and_resumes_after_fresh_snapshots`,
  `f_creates_its_position_row_in_the_transaction_of_its_first_payer_grant` and
  `a_sweep_never_overwrites_a_position_written_by_a_concurrent_first_grant` (an interleaving: the first
  grant takes its snapshot, a `left` event commits, the sweep runs `ON CONFLICT DO NOTHING`, the grant
  commits; the `left` event is applied once, not skipped).
- *Unknown kinds.* `an_unknown_event_kind_is_a_campaign_scope_revocation_and_raises_an_alert` (the
  reference consumer), `the_scope_check_constraint_rejects_a_malformed_row` (a `member` row with no
  revision, a `ticket` row with a member, a `campaign` row with a member each fail on insert), and
  `every_event_kind_satisfies_the_scope_check_constraint` (a table test over all kinds).
- *Transfer targets.* `ownership_cannot_be_proposed_to_a_contributor` and
  `ownership_acceptance_re_checks_the_same_rule` (proposal and acceptance agree: a seated member or a
  co-host, including a seatless co-host); `a_seatless_cohost_can_accept_the_host_role` and
  `a_contributor_owner_receiving_the_host_role_through_closure_holds_no_seat`.
- *Round 0i.* `unarchive_and_closure_race_in_both_orders` (a forced interleaving: B's close transaction
  marks the host account `closing` first, then `unarchive` is refused with `host_account_not_active`; or
  `unarchive` takes the share lock first, commits, and B's step 2 then lists the campaign as a blocker),
  `unarchive_is_refused_when_the_host_account_is_missing_or_deleted`,
  `unarchive_holds_the_account_share_lock_until_commit`,
  `leave_all_skips_campaigns_the_closing_account_owns` (an archived campaign it owns and hosts keeps its
  membership and is scheduled for deletion; no hand-over to itself),
  `leave_all_skips_the_handover_with_an_audit_line_and_an_alert_when_the_owner_is_not_active` (owner
  `closing`, `closed`, missing, or with an inactive member row: the closing account keeps the host row,
  `closure_handover_skipped` is logged and alerted, nothing else changes),
  `member_snapshot_carries_feed_epoch_from_the_same_statement` and
  `a_snapshot_on_a_new_feed_epoch_resets_the_consumers_position` (the stored position is discarded and
  restarts at the snapshot's `event_seq`).
- *Event semantics.* `a_host_transfer_writes_one_revoking_row_for_the_old_host_and_one_plain_row_for_the_new`,
  `an_ownership_transfer_revokes_the_old_owner_principal`,
  `a_campaign_level_revoking_event_forces_every_grant_to_reauthorize` and
  `an_archived_campaign_still_lets_members_read_after_the_reauthorization` (revoking means re-check, not
  deny).
- *Seats and roles.* `set_seat_on_a_player_changes_the_role_to_contributor_and_is_bounded_by_the_contributor_limit`
  and `only_host_and_cohost_may_be_seatless`.
- *Display cap and reissue.* `the_fifth_redemption_is_refused_under_the_campaign_lock`,
  `pending_pairings_do_not_count_toward_the_display_cap_but_approved_unredeemed_ones_count_at_approval`,
  `a_redemption_allows_at_most_one_reissue`, and `a_reissue_cannot_exceed_the_display_cap` (two concurrent
  reissues with the forced-interleaving harness).
- *Throttle keys.* `one_ipv6_48_allocation_cannot_fill_the_tight_key_map` (65,536 distinct /64 sources inside
  a /48 are stopped by the loose key before the tight map saturates),
  `a_request_refused_by_the_loose_key_never_inserts_a_tight_key`,
  `a_full_loose_map_refuses_every_new_loose_key_and_leaves_existing_keys_working`,
  `the_loose_key_limits_are_ten_times_the_tight_limits`, and
  `the_client_ip_comes_from_the_trusted_proxy_chain_not_the_socket_peer`.
- *Closure.* `the_close_transaction_rolls_back_on_blockers_and_commits_closing_with_scheduled_deletions`
  (the fake models B's close: account `closing` set first, `closure_campaigns`, rollback with blockers,
  otherwise commit with `schedule_archived_deletions`),
  `a_user_with_blockers_is_not_left_closing_and_can_follow_the_transfer_links`,
  `reopening_cancels_scheduled_deletions_in_the_same_transaction`,
  `a_closing_account_is_not_authorized_by_member_snapshot`, `accept_transfer_refuses_a_closing_account`,
  `a_proposal_to_a_closing_account_is_refused`, `accept_transfer_reverifies_the_proposal_after_the_locks`,
  `accept_transfer_and_the_completion_transaction_never_deadlock` (the acceptance paired with B's
  completion transaction of 8.7 step 7, which updates the account and then `leave_all` locks a campaign
  the account belongs to, in both interleavings; the harness has a switch that restores the **old order**
  (campaign, then account) and the test asserts the old order reports `deadlock_detected` while the
  specified order does not, so the test can fail before the fix),
  `two_concurrent_closures_sharing_two_campaigns_never_deadlock` (two accounts that are both members of
  campaigns X and Y close at once; the multi-campaign lockers lock in `campaign_id` order; a switch for
  the reverse order shows the deadlock),
  `closing_cancels_the_accounts_outgoing_transfer_proposals` and
  `accepting_an_ownership_proposal_on_a_campaign_scheduled_for_deletion_is_refused`,
  `leave_all_hands_the_host_role_to_a_contributor_owner_without_a_seat` (a seatless host is valid), `unarchive_is_refused_while_the_hosts_account_is_not_active` (so no blocker appears between chunks), `a_late_blocker_does_not_strand_the_account_in_closing`,
  `closure_lists_owned_archived_campaigns_with_transfer_or_delete`, and
  `closing_with_archived_campaigns_schedules_their_deletion_with_a_notice_and_reopening_cancels_it`.
- *Funding is notified, never directed.* `the_funding_notification_has_no_cancel_method_and_receives_exactly_one_call`
  (a structural guarantee plus the call count). The behavioural POT-04 test belongs to F's consumer.
- *Display redeem and renew without a CSRF token.* `redeem_and_renew_do_not_depend_on_the_presession_csrf_cookie`
  (the pre-session cookie expired or absent: still accepted with the device cookie, a matching `Origin`,
  `Sec-Fetch-Site: same-origin` and `X-DF-Device`), and `redeem_and_renew_are_refused_without_origin_or_with_another_site`
  (a missing `Origin`, a foreign `Origin`, a present `Sec-Fetch-Site` that is not `same-origin`, a missing
  `X-DF-Device` and a non-JSON content type each change nothing). `an_idle_display_can_redeem_after_the_presession_cookie_lapsed`.
- *Fragment hygiene (P's app and the server).* `app_join_answers_200_with_no_redirect` (no sign-in
  redirect, no trailing-slash redirect on that path) and `no_client_log_or_telemetry_payload_contains_token_equals`
  (P's app test: the entry point strips the fragment before the router, telemetry and error reporting
  start; a captured log and telemetry sink never sees `token=`, including after a forced panic and an
  API error).
- *No cross-owner foreign keys.* `campaign_ids_are_validated_through_the_ports_at_creation` (an unknown
  or unverified account, or an account with no tenant, cannot create a campaign) and
  `no_campaigns_table_references_another_owners_table` (a schema-introspection test over
  `pg_constraint`).
- *Microphone.* `declining_the_microphone_leaves_typed_play` posts the permission result with the
  member's own session; a different member's session cannot set it.
- *Scope of AUTH-02.* My route-table tests cover the campaign endpoints and the ticket verification.
  Other owners' routes are outside this table; the merged-router AUTH-02 run belongs to S4's acceptance
  suite (request S4-1).

**Round 1 (policy, campaigns, wizard, dashboard).** Pure: the matrix and support tests, `Delegated`
honored only with its bit, the rank table (including nobody removes the owner and the rejoin rank
rule), owner-versus-host rules, `a_contributor_cannot_be_made_co_host`, money capabilities unaffected by
role changes; short-code alphabet and normalization; per-layer fail-closed `KeyedWindows` with loose-first
order (`flooding_one_throttle_layer_does_not_lock_out_another`, IPv6 /64 and /48 keying); the
`JoinInvitation` check order for every failure including contributor limits and removed versus left;
`decide_start` blockers; pairing and ticket transitions with the exact-expiry cases. `df-campaign-wire`
round trips. Facade (fake store): draft, resume, back, finish idempotency, conflict, expired operation
id, limits, host member created with the campaign, verified-email requirement, archived read-only,
`closure_is_blocked_while_the_account_owns_or_hosts_an_active_campaign`. API: `DashboardView` empty
state, validation reasons, CSRF and cross-site refusal, the route-table tests. Gate and the
paired-account fixture.

**Round 2 (invitations, join, room codes).** The preview and the failure reasons, token versus
malformed token byte equality, the pending-join cookie surviving sign in and cleared after accept,
accept idempotency, the forced-interleaving seat tests (Postgres contract), contributor invitations and
`maximum_contributors`, join requests with role choice and the pending cap, removed-account requests
refused without `may_rejoin`, `allow-rejoin` by rank and reactivation of the same `MemberId` and row
with `joins_as`, links revoked with the co-host, throttles per layer and saturation, code rotation, a
code never reaching an existing member, no token or code in spans or audit lines. The Postgres
adapter, migration 0008, the event-feed ordering and keyset completeness tests.

**Round 3 (lobby, characters, readiness, microphone, display).** Open lobby with the unified gate,
character choice through the fake `CharacterDirectory`, ready cleared on change, blockers shown,
removal clearing lobby rows, a sponsored player reaching Ready with no funding field (UX-01 slice),
microphone through the member's session and typed fallback, the full pairing flow through the API
(challenge, status, lookup, confirm, redeem once, reissue, extension, several displays and per-display
unpair), the cookie, expiry and revocation tests above, fenced `session_started`, migration 0009.

**Round 4 (settings, leave and remove, transfers, archive, closure, events, smoke).** Rank and leave
rules, host versus ownership transfer (owner unchanged by a host transfer; host transfer revokes every
ticket of the session), archive blocks the lobby and opens read-only with export offered, delete
request marks `deleting` and emits `delete_requested`, `unarchive` emits `unarchived`, the closure tests,
retention removes only expired rows, the Postgres adapter passes every contract test, and a real-browser
smoke of P's campaign screens (dashboard, wizard, overview, join, lobby, display) with the S1 API at
1440x900 and 375x812, screenshots in `artifacts/tmp/agents/S1/round-4/`, on a private port (never
63309), stopping only the process I started.

Acceptance catalog items covered: AUTH-01 for the campaign endpoints (round 2) and AUTH-02 for the
campaign endpoints and the ticket verification (round 3); UX-01 the card-free slice up to Ready (the full
supported session belongs to S4's integrated run); the campaign and table steps of J01, J02, J07, J08
and J14. AUTH-03 is O2's.

## 13. What I need from other owners

### 13.1 Agreement with O1 (recorded in both notes)

1. **Session row.** My lobby endpoint calls O1's `open_session(campaign)` through `SessionDirectory`; O1
   owns the row from then on. `session_id` columns in my schema carry no foreign key.
2. **Membership read.** I supply `MemberSnapshot` (5.1) and `member_snapshot(account, campaign)`, which
   includes the co-host delegation bits, whether the member holds a seat, the member revision, the
   campaign's `seat_count`, status and `event_seq`, and `authorized` (false for a closing account). O1's
   edge uses it to mint connection tickets (`TicketRole::{Player, Host, CoHost}`).
3. **Seats.** O1's edge mints every connection ticket; this facade mints none. Seat binding stays O1's
   (`df_runtime.seat_bindings`); membership stays mine. **O1 seats only members whose snapshot has
   `holds_seat = true`, never more than `campaign.seat_count`, and never mints a member**: the session
   member is my `member_id`. `runtime.maximum_seats` is a ceiling on `seat_count`, not a second source
   of truth; the two defaults are both 6 and creation refuses `seat_count` above mine.
4. **Display.** The pairing is mine (8.5). The display ticket and device secret travel as cookies on
   the one public origin; the edge calls `verify_display_ticket(ticket, device_secret, session)` before
   minting a `Display` connection ticket. Renewal is `POST /app/api/display/renew`, which extends the same
   ticket. **O1's display client is the caller of renew** while the TV is on `/sessions/:id/display` (the
   renewal window is the last hour of a 12-hour ticket), and of `display/redeem` if it is ever sent back
   to pairing. Both calls authenticate with the device cookie, a required matching `Origin`,
   `Sec-Fetch-Site: same-origin` when that header is present (absent is accepted only because `Origin`,
   `X-DF-Device` and the Strict cookie are all present), a JSON content type and an `X-DF-Device` header;
   they do **not** use B's one-hour pre-session CSRF cookie, which an idle or streaming TV no longer has
   (8.5 step 4).
5. **Fenced writes into my schema use O1's `OwnerFence` and the committed functions.** The one
   state-setting write O1 makes into `campaigns`, `session_started`, carries
   `OwnerFence { recovery_epoch, claim_seq, placement_id }`. My store's statement order, as O1's committed
   6.1 and its request to S1 (0) require, is: **`df_runtime.enter_service()` as the first statement**,
   then `df_runtime.fence_current(tenant, campaign, recovery_epoch, claim_seq, placement_id, 'game')`
   (six arguments, path `'game'`), and only then `campaigns.campaign ... FOR UPDATE`. A false result
   refuses the write with no change; a halt in progress raises `DF002` (`ServiceHalting`), which the store
   maps to the typed retryable `service_halting`. No portal path takes the fence, so the two cannot
   deadlock. My call into O1 (`Start`) is fenced by O1. `session_started` additionally refuses unless the
   campaign is `active` (O1 ends the session on that refusal). The microphone report is no longer an O1
   write (8.4). **Revocation writes are not gated by the fence** but are **scoped**:
   `revoke_display_tickets` and `session_ended` called by a runtime role reach only tickets and sessions
   in that runtime's own `leased_table_sessions()` (O1's scope convention, enforced by row security on
   `display_ticket`), so a runtime for campaign X is refused for campaign Y; the control-plane role,
   with its own scope setting, may revoke for any campaign. The presented `OwnerFence` is stored on the
   event rows (`writer_fence`) for audit. O1 calls them on every session end, and **the control plane
   calls `revoke_display_tickets` when a placement is released** (decision 29), because a released
   placement's token no longer resolves under the scope function; my backstop (an ended or unreadable
   session is treated as revoked) holds regardless.
6. **Membership and the feed (D24).** O1 reads membership through `member_snapshot` at connect and at
   the start of each publication batch (on the stream layer's pooled connection, never the owner thread)
   and states `publication_batch_maximum_ms`; that read is the revocation guarantee. O1's feed reader is
   not a gating consumer: it uses the feed to close streams promptly (**the committed O1 6.4 still
   treats it as lag-gated; that text should follow this item**). It keeps no durable cursor: for each
   campaign it holds it starts from the `event_seq` of a fresh `member_snapshot`, waits (with its stated
   deadline) when a number is missing, and finds changes with `campaign_heads(held ids)` every poll
   (primary-key read, no stamps, 5.1); a head below its position or a changed `feed_epoch` starts a new
   epoch (drop cached authority, re-snapshot). `event_seq` is allocated from the counter column on the
   `campaign` row under the row lock, in the same transaction as the change, so it is gapless and
   commits in order. The consumer rule for each event `scope` is the table in 5.1.
7. **Local-only paths.** The seat-token `Reclaim` and the derived room code stay local-only.
8. **`transaction_timeout` on O1's roles.** Every O1 role that calls an event-bumping method
   (`session_started`, `session_ended`, `revoke_display_tickets`; the control plane's role for the
   placement-release revocation) has `ALTER ROLE ... SET transaction_timeout` at most
   `feed_transaction_timeout_seconds` (10), and O1's processes run the startup check of 5.1 and refuse to
   start otherwise. O1's pool discards a connection whose session was terminated, and a termination
   during `COMMIT` is `Unknown`. The PostgreSQL floor is 17 (LAUNCH.md).

### 13.2 Requests by owner

**B (identity, accounts, router).**
- B-1. Mount `campaign_api_router` beside `app_api::router` (dependency on `df-campaign`, `mod campaigns;`
  and the `pub use`). Release the locks on `crates/df-auth/src/lib.rs`, `crates/df-persistence/src/lib.rs`
  and `Cargo.toml`, `crates/df-types/src/lib.rs` and `identity.rs`, and the workspace `Cargo.toml`
  (members `df-campaign`, `df-campaign-wire`) when the rounds that need them begin.
- B-2. Move `Clock`, `SystemClock`, `FakeClock`, `RandomUnavailable`, `random_bytes`, `SecretToken`,
  `TokenDigest`, `tagged_digest`, `new_operation_id` and `operation_issued_at_ms` into `df-types`
  (`clock`, `token`), with `df-customer` re-exporting them.
- **B-8. Account closure (J14), coordinator decisions.** Closing runs in one transaction: set `closing`
  first (taking the account row lock), call `closure_campaigns(account)` in that transaction, **roll back
  and return the blocking list if there are blockers**, otherwise commit `closing` together with
  `schedule_archived_deletions(account, delete_after_ms)`, and notify every member of a scheduled
  campaign (deletion date, export available until then). Reopening calls `cancel_scheduled_deletions` in
  the reopening transaction. **The close transaction also cancels the account's outgoing transfer
  proposals** (inside `schedule_archived_deletions`). Completed closure is a separate transaction that
  updates the account row and then calls `leave_all(account)`, which (per chunk, under the account row lock, aborts if the account is no longer `closing`) locks
  campaigns in `campaign_id` order. **B's database role has `ALTER ROLE ... SET transaction_timeout` at
  most `feed_transaction_timeout_seconds` (10)** and B's process runs the startup check of 5.1; a
  per-transaction `SET LOCAL` alone is not enough (5.1). B's pool discards a terminated connection, and
  a termination during `COMMIT` is `Unknown`. Serve `/app/join` (the bundle shell) with 200 and no
  redirect, including no trailing-slash redirect, so the invitation fragment cannot follow a redirect (8.2).
- **B-9.** (a) Expose a stable **SELECT-only view** of account status (`accounts.account_status_view`),
  readable in one statement with my member and campaign rows, so `member_snapshot` is unauthorized for
  `closing` and `closed` accounts. (b) Provide a **`SECURITY DEFINER` function**
  `accounts.lock_account_status(account_id) -> status` that takes the share lock and returns the status,
  with `EXECUTE` granted to S1's role: `FOR SHARE` needs `UPDATE` privilege on the table, and S1's role
  must have no write rights on B's rows. Every member-creating path and `accept_transfer` call it first
  (8.6). (c) Expose the trusted-proxy client-IP extraction of `bootstrap-origin-policy.md` as a function
  the campaign routes call (4.3). (d) Say whether a restricted or closed creator tenant can gate a
  campaign after its ownership has moved: S1's position is that `campaign.tenant_id` is only an isolation
  key and a restricted tenant does not gate the campaign; money gating is F's, per session.

**F (money core, decision 27).**
- F-1 (was B-5). Consume the `campaign_event` feed with a durable cursor **in F's own schema**
  (`funding_feed_position`; S1 has no cursor table), **advanced in the same transaction as the effect,
  with effect rows unique on (`campaign_id`, `event_seq`)** (5.1), so F's effect transaction locks only
  F rows. On `left` and `removed` stop new use of that member's allocation and release unreserved credit
  (J07, POT-04) without touching admitted reservations or running jobs, and own the behavioural POT-04
  test. Host or ownership transfers change no payer grant. **F does not gate payer admission on feed
  lag**: admission does a fresh `member_snapshot` read (check-at-use, decision 24), so F's note should
  drop "S1 5.1 gating rule" for admission and keep the feed only for the leaver's allocation. Correct
  F's line that says its cursor is S1's `campaign_event_cursor`.
- F-2. `start_readiness(session)` and `open_session_funding(session, campaign, tenant)` as F's 18.8
  describes (S1's Start calls the second before O1's `Start`, finance first); the spend preset catalog
  (ids, labels, hourly amounts, the "proposed pilot pricing" flag, decision 26 wording); verify
  membership via `member_snapshot` at payer admission; `payer_grant` references
  `campaigns.campaign.campaign_id` (was B-6).

**P (account app, decision 28).**
- P-1. Screens (section 7): campaign dashboard, wizard, overview with tabs, join and room-code entry,
  lobby, and the TV `/app/display` flow; they render from `df-campaign-wire` types. P's `overview` may
  embed `DashboardView` through the `CampaignDirectory` trait the facade implements.
- P-2. Export the request guard and the response envelope for S1's router to reuse (session or
  pre-session authentication, `X-DF-CSRF`, `Content-Type`, `Sec-Fetch-Site`, Origin), and allow a `reason`
  field from `df-campaign-wire`'s closed enum beside the envelope's `code`.
- P-3 (was B-3/B-4). After sign in or sign up, the app checks `GET join/pending` and offers the pending
  invitation. **`/app/join` answers 200 with no redirect. The fragment is stripped by a small
  synchronous script in the HTML shell that B serves, allowed by a CSP hash, which runs before the wasm
  is fetched and before any JS glue or Rust code starts** (the glue's `onerror` handler runs before the
  Rust entry point, so stripping in Rust alone is too late). The script copies the token into a
  non-global variable handed to the app's entry point, calls `replaceState` and nothing else; the app
  sends the token only in a request body. A test proves no client log, error-handler or telemetry
  payload contains `token=`, including after a forced panic and an API error. No `Navigation` or
  stylesheet work is needed from B.
- P-4. The display screen: poll `GET display/status`, call `POST display/redeem` when approved (retry the
  same call on a lost response), call `display/renew` inside the renewal window while it is the screen
  on display, navigate to `/sessions/:id/display`, and clear its state on a terminal reason. Redeem and
  renew carry no CSRF token (8.5 step 4). Once the TV is on O1's display page, **O1's display client**
  makes the renew call.

**O1.** The agreement is 13.1. Open mismatches in 13.3. Also: O1-1 `SessionDirectory` (`open_session`,
`state`, `start(session, roster)` with `{ member_id, account_id, role, holds_seat, character_ref }`,
`request_pause`); O1-3 `SaveDirectory::latest(campaign)`; O1-4 runtime admission through
`member_snapshot` at connect and batch start, and closing streams on `display_ticket_revoked`; O1-5 the
display client calls `display/renew` (and `display/redeem` if sent back to pairing) with the
authentication of 8.5 step 4; O1-6 the role-level `transaction_timeout` and startup check of 13.1
item 8; O1-7 pauses: the player's emergency pause request and the host's in-session pause go through O1's RPC, authorized by S1's pure `authorize(role, delegations, capability)` over `member_snapshot` (S1 exports `authorize_capability(account, campaign, capability)`; S1 has no pause route); O1-8 `SessionDirectory::campaign_of(session)` and a non-ended-session read for archive and delete, and O1 ends any session still in `lobby` or `preparing` on a campaign-scope `archived` or `delete_requested` event; O1-9 O1's 6.4 text (renew 'with both cookies and CSRF'; the feed reader treated as lag-gated) follows 8.5 step 4 and 13.1 item 6, and O1 confirms the role-level 10 s timeout suits its long transactions or names the exception.

**O2.** O2-1 id newtypes `CharacterTemplateId`, `CampaignCharacterId` in `df-types`; O2-2
`CharacterDirectory`; O2-3 the gateway calls `member_snapshot` at grant issue and renewal and records the
snapshot `event_seq` in each grant; its feed reader polls `campaign_heads(held campaign ids)` (a primary-key read; lag = time since the last successful read covering every held campaign), uses (`campaign_id`, `event_seq`), the `revoking` flag, the `scope` column and the consumer rules of 5.1 (a `ticket` row closes the matching stream and re-authorizes no grant)
and the rules of 5.1; O2-4 hooks for `deleting`, `delete_requested`, `delete_scheduled` and
`delete_cancelled`, and an audience-filtered export entry point (the closure notice window of 8.7
depends on it).

**S2 (infrastructure).** S2-1. Rate limit `/app/api/display/*` and `/app/api/join/*` at the edge, since
the in-process ceilings are per replica and the loose-map residual (4.3) is answered there. Invitation
tokens are no longer path bearers (fragment and body only), so no log redaction is needed for them.

**S4 (quality).** S4-1. Run AUTH-02 and the cross-account checks over the merged router (the `/app/api`
routers of P and S1 and the edge routes), since my table covers only mine.

**Coordinator.** C-1 add `crates/df-campaign` and **`crates/df-campaign-wire`** as workspace members;
C-2 the scratch PostgreSQL (17 or later) and the schema name `campaigns`; C-3 the `df-campaign` edges and modules
in `planning/subsystem-architecture.md` (locked by B; I send exact lines in `request.md`); C-4 allow the
`df-types` additions of B-2.

### 13.3 Open mismatches against the O1 and O2 notes (to resolve in their next rounds)

| Owner | Where | Mismatch | S1 position |
| --- | --- | --- | --- |
| O1 | the display paragraph and 8.2 (renewal) | O1 0f took in renewal through `POST /display/renew` extending the same ticket (confirm in its text) | Settled between S1 and O1 0f. Remaining: **O1's display client is the caller of renew** once the TV is on `/sessions/:id/display`, authenticated by the device cookie plus Origin, Sec-Fetch-Site and `X-DF-Device` and not by B's one-hour pre-session cookie (8.5 step 4) |
| O1 | section 6.2 item 4 (seat bounds) | O1 0f took in `holds_seat = true` and `campaign.seat_count` from the snapshot (confirm in its text) | Settled; `runtime.maximum_seats` stays a ceiling (13.1 item 3) |
| O1 | roles that call event-bumping methods | No role-level `transaction_timeout` or startup check | Required: 13.1 item 8 (the `SET LOCAL` alone does not shorten a running timer) |
| O1 | section 14 / fence | `report_microphone` was an O1 fenced write in earlier S1 notes | Removed: the lobby screen reports through the member's own session (8.4); the only fenced O1 write is `session_started` |
| O1 | who calls `revoke_display_tickets` once a released placement's token no longer resolves | Unspecified | The control plane calls it on placement release (decision 29); the unreadable-session backstop stands (4.6) |
| O2 | section 4 / L40 (`MembershipReader::load`) and the committed text | Names S1's internal adapter | The call is `member_snapshot(account, campaign)` |
| O2 | L725-726 (a cursor table and a cross-campaign read) | Asks for one query over a per-campaign cursor table so the lag figure covers every campaign | The authoritative read for a gateway is `campaign_heads(held campaign ids)` (primary key, no stamps); the gateway keeps in-memory positions and no shared durable cursor; its lag figure is the time since its last successful `campaign_heads` read covering every held campaign (5.1). `campaign_changes_since` is only a discovery hint for durable consumers |
| O2 | grants | Must record the snapshot `event_seq` for the campaign-level rule | S1 requires it (5.1, O2-3) |
| O2 | `Owner { account }` grants | Lose access when ownership moves | `ownership_transferred` is revoking for the old owner and carries that member's bumped revision (5.1) |
| O2 | the cursor | A durable cursor named `assets` on a global sequence | In-memory consumer: start from the snapshot `event_seq` (5.1) |
| F | `finance-ledger-research.md` L915 and 18.x | F's cursor is described as S1's `campaign_event_cursor` row | S1 drops that table: F keeps its cursor in its own schema (`funding_feed_position`), advanced in the effect transaction, effect rows unique on (`campaign_id`, `event_seq`); F's rule that no finance transaction locks an S1 row then holds (5.1, F-1) |
| F | L445 (payer admission gated on feed lag, "S1 5.1 gating rule") | Admission gated on the feed | Not required: F does a fresh `member_snapshot` at admission; only O2's gateway is a gating consumer (5.1, F-1) |

## 14. Decisions and open questions

**Decided:**

- Q1. The control plane (`campaigns.campaign_member`) is authoritative for the roster of eligible members
  and their roles; `df-session` and O1 stay authoritative for who is seated and ready in a live session.
  The member id is minted once by the control plane and used as the session member.
- Q2. Hosted players authenticate (LAUNCH decision 9); guest bootstrap, the local seat-token reclaim
  and the derived room code are local-only.
- Q3. Six seats, configurable, counting the host; a solo campaign is allowed.
- Q4. `Tenant` in `MembershipAuthority` is the creator's tenant at creation, immutable, an isolation
  key; creation requires a verified email.
- Q5. Verified email to join is configuration, default off.
- Q7. The microphone permission is requested by P's wasm lobby screen and reported with the member's
  own session (8.4); the earlier open question is closed by decision 28.
- Q9. Delete and archive belong to the owner with a typed confirmation; the new host after a host
  transfer does not hold them; a demoted owner keeps them.
- Q10. Only seated members can be promoted to co-host; a contributor cannot (test
  `a_contributor_cannot_be_made_co_host`).
- D6. Only the remover's rank or higher may allow a rejoin; a removed account's join request without
  `may_rejoin` is refused at approval; a rejoin clears `may_rejoin` and uses `joins_as` (coordinator).
- D7. `maximum_contributors` defaults to 12; the pending join-request cap defaults to 20 (coordinator).
- D8. Account closure is blocked while the account owns or hosts an active campaign; the closure page
  also lists owned archived campaigns; if the owner closes anyway they are deleted under the normal
  retention window after a notice and an export window for members, so no campaign is left dormant
  without an owner (8.7; coordinator).
- D9. A host transfer revokes every display ticket of the session (4.6; coordinator).
- D10. Same origin is final for the display; the edge mints connection tickets; the facade supplies the
  membership read and `verify_display_ticket` (13.1).
- D15. The close transaction cancels the account's outgoing transfer proposals (8.7; coordinator, round
  0f).
- D16. O1's display client calls renew; redeem and renew rely on the `SameSite=Strict` device cookie plus
  Origin and Sec-Fetch-Site checks, not B's one-hour pre-session CSRF cookie (8.5 step 4; coordinator).
- D17. F keeps its funding cursor in F's own schema, unique on (`campaign_id`, `event_seq`) and advanced in
  the effect transaction; S1 has no cursor table; F does not gate admission on feed lag and reads a
  fresh `member_snapshot` instead (5.1; coordinator).
- D18. No foreign keys from `campaigns` to other owners' tables; ids are validated at creation, as F
  does (6; coordinator).
- D19. The PostgreSQL floor is 17 (LAUNCH.md); `transaction_timeout` is set at role level on every role
  that can bump `event_seq`, with a startup check (5.1).
- D20. Fenced writes: `enter_service()`, then the six-argument `fence_current(..., 'game')`, then the
  campaign lock; a halt gives a retryable `service_halting` (13.1 item 5; O1's committed 6.1).
- D21. Every member-creating path takes the account share lock (through B's definer function) before the
  campaign lock and re-verifies; S1's role has no write right on B's rows (8.6; coordinator, round 0g).
- D22. The poll anchor is `statement_timestamp()`; stamps are only a hint. In-memory consumers read
  `campaign_heads(held ids)`; durable consumers sweep with `campaign_heads_after`; a regression or a
  `feed_epoch` change starts a new epoch (5.1; coordinator).
- D23. `leave_all` is chunked and resumable; each chunk, under the account row lock, aborts if the account
  is no longer `closing`; the blocker check runs once in the close transaction; `unarchive` is refused
  while the host's account is not `active`; a vacated host role passes to the owner, who may be a
  seatless contributor (8.7; coordinator, round 0h).
- D24. `CampaignUnavailable` on join; `session_started` requires an `active` campaign; archive and delete
  refuse with `session_in_progress` only when the session is past the lobby (preparing with funding open, or
  playing); O1 ends a lobby on the event; the retention task executes scheduled deletions under the campaign
  lock and a cancel of a `deleting` campaign is refused (8.4, 8.7).
- D25. Unfenced revocation writes are scoped by O1's `leased_table_sessions()` or the control-plane role
  (4.6, 13.1).
- D26. `campaign_event.scope` (`member`, `ticket`, `campaign`) with a fixed consumer rule per kind (5.1).
- D27. Session-id routes resolve their campaign through `SessionDirectory` before authorizing; co-hosts
  have a self `/seat` route; owner-class routes expect owner AND authorize; pauses go through O1's RPC
  (7).
- D28. If `Sec-Fetch-Site` is absent a redeem or renew is accepted when `Origin`, `X-DF-Device` and the
  Strict device cookie are present; if present and not `same-origin` it is refused (8.5 step 4;
  coordinator ruling).
- D29. `feed_epoch` is a fresh random 128-bit value set by the restore step and re-randomized before
  service resumes after a failover that loses commits; two restores give two epochs (5.1).
- D30. F creates its position row in the transaction of its first payer grant; the sweep inserts only
  with `ON CONFLICT DO NOTHING` (5.1).
- D31. An unknown event kind is a campaign-scope revocation plus an alert; migration 0008 adds a CHECK
  constraint on `scope` and its columns (5.1).
- D32. Ownership can be proposed to and accepted by only a seated member or a co-host (coordinator, round 0h);
  a seatless host is valid in the host-transfer path (8.6).
- D11. The second (loose) throttle key's limits are ten times the tight limits, configurable
  (coordinator).
- D12. `/app` belongs to P's `df-account-app`; S1 serves JSON only (`DashboardView` and the campaign
  API), the types live in `df-campaign-wire`, B mounts the router (decision 29, ruling (a)).
- D13. The cross-owner lock order includes campaigns: accounts and finance, then O1 directory and session
  rows, then `campaigns.campaign`, then campaign child rows, then media (decision 29, ruling (b)).
- D14. The control plane calls `revoke_display_tickets` on placement release; the ended-or-unreadable
  session backstop stands (ruling (c)).

**Still open:**

- Q6. Email-targeted invitations are out of rounds 1-4 (links and codes only); sending through B's
  `EmailSender` is a later round and needs a decision on typing third-party addresses.
- Q8. `request_delete` only marks `deleting`; purge, backups and financial-record retention are O2 and F.

## 15. Delivery order

1. **Round 1: policy, campaigns, wizard, dashboard data.** `df-auth` `roles`, `short_code`,
   `keyed_throttle`; `df-session` `join_invitation`, `lobby`, `display_pairing`; `df-campaign-wire`;
   `df-campaign` (ports, records, service for campaigns and the wizard, fakes, contract tests);
   `df-portal` route table and the JSON handlers for campaigns, the wizard and `GET campaigns`;
   `/dev/campaigns`. Needs B-2 (or a coordinator ruling) before it compiles without `df-customer`.
2. **Round 2: invitations, join, room codes,** the join endpoints, requests, throttles, the pending-join
   cookie, the Postgres adapter, migration 0008, the event feed, `campaign_heads`, `campaign_heads_after` and the hint read `campaign_changes_since`.
3. **Round 3: lobby, characters, readiness, microphone, display pairing,** migration 0009, the fake
   O1/O2/F directories, then the real ports as those owners land them.
4. **Round 4: settings, leave and remove, transfers, archive, closure, delete request, events,** dev CLI
   views, retention task, browser smoke through P's screens at desktop and phone widths.

Each round writes `artifacts/tmp/agents/S1/round-<n>.md` with files changed, behaviours and their tests
(failing-before), the gate line, filters and counts, and requests. Locks are taken per file before
editing; shared files held by B wait for release and are touched with the smallest hunks.
