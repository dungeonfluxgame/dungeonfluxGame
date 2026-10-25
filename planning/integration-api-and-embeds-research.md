# Integration API, session embeds and share links

Status: design only. Lane APIEMBED, 2026-10-25.

## 1. Scope and governing decisions

Owner request: provide future integrations with authenticated APIs for the account details shown in the customer account area and admin
console, and support web embeds and shareable session links. This document defines a bounded design; it does not authorize implementation or
provider setup.

Relevant inputs:

- `artifacts/tmp/agents/LAUNCH.md`, decisions 26, 27, 28, 32 and 33.
- `planning/account-service-research.md`, especially Ã‚Â§Ã‚Â§2-7, 10, 12-14 and 17.
- `planning/google-sign-in-research.md`, especially Ã‚Â§Ã‚Â§3-4 and 7 (server-side sessions, no JS-held
  bearer credentials).
- `planning/master-launch-plan.md`, Ã‚Â§Ã‚Â§3.1-3.2 (account/campaign/session and role boundaries),
  14.1-14.4 (authorization and privacy), 15.2 (service operations).
- Existing contracts: `df-portal/src/app_api/{router.rs,guard.rs,ports.rs,session.rs,admin.rs}`,
  `df-account-app/src/wire.rs`, `df-customer/src/{access.rs,tokens.rs}`.

The account API at `/app/api` remains the first-party browser contract. It uses a `__Host-` session cookie, CSRF on writes, customer-safe
wire types and per-request operator capability checks. The integration API is a separate bearer-token surface at `/api/v1`; it has no cookie
fallback, browser session inheritance, or write authority in v1. The app remains served at `/app/` on the one public origin
`https://dungeonfluxdnd.com` (decision 32).

Decision 26 is a schema boundary: customer responses can contain the customer retail price, estimate, maximum, actual charge and balance
only. They must not contain the table fee, multiplier, provider rate, provider identity when that reveals cost, or supplier cost. Operator
integration scopes may expose the already-admin-visible full breakdown, with audited use. This split is represented by distinct customer and
operator DTOs rather than a `role` switch on a shared response.

## 2. Versioned integration API

### 2.1 Authentication choice

Use personal access tokens (PATs) first. They support named, scoped, expiring, revocable integrations without introducing an authorization
server, client registration, consent pages or refresh-token lifecycle before there is a concrete third-party use case. Keep the scope
vocabulary resource/action based so OAuth 2.0 authorization-code + PKCE can grant the same scopes in phase 2. OAuth is a later alternative
for third-party applications that need per-user consent and delegated grants; it does not replace customer PATs for scripts or trusted
personal tools.

Customer token creation is in `/app/api` under the existing authenticated session and CSRF guard. Require recent re-authentication
(five-minute proof window, consistent with account closure) before creation, scope changes, revocation, or revealing the issued secret.
Limit active tokens per account (proposed default 10, configurable). User chooses a label, scopes, and expiry from allowed values (default
90 days; maximum 365 days; no never-expiring token). Secret appears once in the response and is not recoverable later. The UI shows metadata
and last-used time, never a token prefix that could be mistaken for the secret.

Operator API tokens are separate credentials, never created by a customer PAT route. An operator with current account status and required
MFA creates a token for their own operator principal; require recent MFA, a mandatory reason, a short maximum life (proposed 30 days), and
exact admin scopes. No operator can mint a token on behalf of another operator. Record creation, reveal event, revoke, denied use, and every
successful use in the protected operator audit stream. Use is audited with token id, operator id, requested scope set, endpoint template,
status class, timestamp and correlation id; do not log the Authorization header or secret. Denied-use auditing is rate-limited and
aggregated to avoid attacker-controlled audit floods.

### 2.2 Token wire and storage

Authorization header only: `Authorization: Bearer df_pat_v1_<public-id>_<secret>_<check>`. Never accept a token in query parameters, path
segments, cookies or request body. `public-id` is a random 128-bit lookup id; `secret` is 256 random bits, base64url without padding;
`check` is a short checksum derived from the secret for secret-scanner detection (it is not an authenticator). Prefix and layout are stable
enough for repository/log secret scanners. Reject malformed lengths before storage lookup and return the same invalid-token error for
unknown, expired, revoked and checksum-invalid tokens.

Store token id, account/operator principal, label, canonical scopes, created/expiry/revoked timestamps, last-used timestamp, and a keyed
HMAC-SHA-256 digest of the secret under a versioned server pepper. Store pepper key id for rotation; keep key material in server secret
configuration, never PostgreSQL. Compare candidate digest in constant time. Do not persist plaintext secret or include it in `Debug`,
telemetry, traces, audit fields, error messages or database diagnostics. On pepper rotation, accept configured active and retiring key ids;
re-HMAC the presented token digest on successful use, then retire old key after a defined window. Token generation and checksum verification
use `df-customer`'s cryptographic conventions where suitable, but access tokens have a separate private type and do not reuse browser
session tokens.

Every request passes one integration guard: parse bearer token, find by public id, validate expiry/revocation and digest, resolve principal
status, compute endpoint's required scope, check granted scope, apply rate limits, and attach an immutable `IntegrationPrincipal` and
request id to handler context. Do not trust client-provided account or operator ids. Owner account closure revokes all customer tokens
transactionally with account status transition. Operator role removal/MFA revocation invalidates operator tokens on next request through
current grant-generation validation. On unknown principal or permission, return a non-enumerating `404` for foreign-owned resources and
`403` for a known principal's missing scope.

### 2.3 Scope catalog

Scopes are exact, non-wildcard grants; `read` is not transitive between resource classes. V1 scopes:

| Scope | Caller and allowed data |
|---|---|
| `account:read` | Own account metadata: account id, display name, verified/standing state, created time. Email is opt-in at token creation and has a separate `account:email:read` grant to avoid incidental disclosure. |
| `account:email:read` | Own verified email address only; requires `account:read` as well. |
| `auth_sessions:read` | Own sign-in session metadata: opaque session id, device label, created/last-seen/expiry/revoked state. Never IP address, cookie, CSRF secret or browser fingerprint. |
| `play_sessions:read` | Own campaign membership's session summaries and participation dates; only fields the account is authorized to see in the customer UI. |
| `assets:read` | Asset metadata and authorized download links/bytes through existing audience-safe media delivery; no private or quarantined assets. |
| `funding:read` | Own customer-safe balance, trial and funding allocation summary. Never another payer's balance or funding source details. |
| `statements:read` | Own customer-safe top-up, charge, credit, refund and spend statement lines. |
| `stats:read` | Own account and play usage aggregates with no other participant's private statistics. |
| `admin:accounts:read` | Operator `SupportInspect` equivalent, accounts list and masked account detail. Always revalidate current role and MFA per call. |
| `admin:accounts:pii:read` | Unmasked email and account consent/authentication metadata; current grant and every-read audit required. |
| `admin:auth_sessions:read` | Operator read of account sign-in session metadata; sensitive support scope separate from account listing. |
| `admin:play_sessions:read` | Operator session directory metadata, with redaction by default. Never grants narrative/private note access. |
| `admin:assets:read` | Operator asset metadata and review state, not arbitrary private asset bytes. |
| `admin:ledger:read` | Operator ledger, refunds/disputes state, reconciliation summary and full cost breakdown only for the specifically selected accounts/endpoints audited. |
| `admin:stats:read` | Operational/account aggregates allowed to SupportInspect; no raw telemetry or player content. |

Admin scopes are not composed from customer scopes. They require an operator principal with current second factor and matching
`OperatorCapability` on every request. Preserve existing distinctions: refunds, goodwill credits, dispute response, provider control and
content review are write actions, not implied by any read scope. V1 has no mutation scopes, no refund/credit/provider-control endpoints, no
account impersonation, and no account-wide export. Future writes need separately reviewed scope, explicit confirmation/idempotency contract,
narrow action and audit before addition.

The account detail set in the request is broader than the current `wire.rs`: account metadata, sign-in sessions, play sessions, assets,
funding, top-up and spend history, and stats. Reuse `AccountView`'s field vocabulary, `OverviewView`, `FundingView`,
`StatementsView`/`StatementLine`, `AdminAccountRow`, `AdminAccountDetail` and `LedgerEntry` where safe. Do not expose `SessionView` directly
because it includes CSRF; do not expose `LedgerEntry` to customer scopes because it includes provider cost, table fee and multiplier. Some
requested resources (sign-in-session listing, assets catalog, account stats, complete account detail) lack a current integration-ready wire
type or port: add purpose-built projections through owning subsystem ports rather than inventing database reads in the router. Confirm the
complete Ã¢â‚¬Å“customer account area and admin consoleÃ¢â‚¬Â list from `artifacts/tmp/lanes/P2/brief.md` before finalizing endpoint
coverage; it is the source list for the implementation lane.

### 2.4 Endpoint catalog and shapes

All endpoints require TLS and `Authorization: Bearer ...`. JSON uses `application/json; charset=utf-8`; identifiers are opaque strings;
money remains integer micro-dollars/cents as the existing wire types define. Timestamps are RFC 3339 UTC strings in the new envelope, with
explicit migration mapping from existing `*_at_ms` wire fields. Unknown request properties are rejected. Collection reads support `limit`
(default 50, maximum 100) and opaque `cursor`; cursors are signed, account-bound, endpoint-bound, stable sort key + unique id, and expire
after 24 hours. No `offset` paging.

| Method and path | Scope | Response body |
|---|---|---|
| `GET /api/v1/account` | `account:read` | `{ "data": { "account_id", "display_name", "email?", "verified", "standing", "created_at" }, "meta": { "request_id" } }`. Email is omitted absent `account:email:read`. |
| `GET /api/v1/auth-sessions` | `auth_sessions:read` | `{ "data": [{ "session_id", "device_label", "created_at", "last_seen_at", "expires_at", "revoked" }], "page": { "next_cursor" } }`. Own sessions only. |
| `GET /api/v1/play-sessions` | `play_sessions:read` | `{ "data": [{ "session_id", "campaign_id", "campaign_name", "started_at", "ended_at", "duration_seconds", "role" }], "page" }`. Membership-scoped, no roleplay text. |
| `GET /api/v1/assets` | `assets:read` | `{ "data": [{ "asset_id", "name", "kind", "campaign_id?", "created_at", "audience", "download_url?" }], "page" }`. Only safe authorized assets; download uses short-lived audience-bound URL or authenticated delivery. |
| `GET /api/v1/funding` | `funding:read` | `{ "data": <OverviewView + customer-safe FundingView>, "meta" }`. No payment instrument or supplier detail. |
| `GET /api/v1/statements` | `statements:read` | `{ "data": [<StatementLine>], "page": { "next_cursor" } }`. Optional bounded `from`, `to`, `category` filters. |
| `GET /api/v1/stats` | `stats:read` | `{ "data": { "sessions_count", "play_seconds", "campaign_count", "assets_count", "spend_micro_dollars", "period_start", "period_end" }, "meta" }`. Own data only; support daily/weekly/monthly period presets, no arbitrary heavy query. |
| `GET /api/v1/admin/accounts` | `admin:accounts:read` | `{ "data": [<AdminAccountRow>], "page" }`; supports exact account id or bounded masked/email search, no arbitrary unbounded query. |
| `GET /api/v1/admin/accounts/{account_id}` | `admin:accounts:read` | `{ "data": { "account": <AdminAccountRow>, "ledger_summary": <customer-safe or explicitly admin projection> }, "meta" }`; do not silently include raw ledger. |
| `GET /api/v1/admin/accounts/{account_id}/auth-sessions` | `admin:auth_sessions:read` | Paged session metadata with account id and redacted device fields. |
| `GET /api/v1/admin/accounts/{account_id}/play-sessions` | `admin:play_sessions:read` | Paged session metadata; no private narrative. |
| `GET /api/v1/admin/accounts/{account_id}/assets` | `admin:assets:read` | Paged metadata and review state; no direct private media retrieval. |
| `GET /api/v1/admin/accounts/{account_id}/ledger` | `admin:ledger:read` | Paged `LedgerEntry` equivalent, including full breakdown allowed only to operator; no refunds or other writes. |
| `GET /api/v1/admin/stats` | `admin:stats:read` | Bounded aggregate counts and service measures, never raw OTEL log/trace export. |

Request/response type names should be added in `df-account-app::wire` only if they are shared with `/app/api`; otherwise keep versioned REST
wire DTOs private to `df-portal::integration_api`. Existing `OverviewView`, `FundingView`, `StatementsView` can be projected into versioned
envelopes without changing the app contract. Stable public JSON fields get additive evolution only within v1; removal/type change requires
`/api/v2`.

### 2.5 Semantics, cache, limits and errors

Read-mostly means GET/HEAD only in v1. `HEAD` mirrors GET headers without body. Every resource lookup applies owner/account and campaign
membership predicates in the owning port; foreign IDs return the same `404` shape as absent IDs. Page cursors never encode clear account
data and cannot be transferred across principal, endpoint or filter set. Use weak ETags from representation version + stable row revision
for resources and pages; `If-None-Match` returns `304`. Customer resource `Cache-Control: private, no-cache`; sensitive auth-session,
funding, statement and all admin responses use `no-store`. ETags do not override authorization: authenticate and authorize before returning
`304`.

Return one documented error object: `{ "error": { "code":
"invalid_token|insufficient_scope|not_found|invalid_request|rate_limited|unavailable|internal_error", "message": "...", "request_id": "...",
"required_scope?": "..." } }`. Do not return SQL, provider, account-existence or token diagnostics. `401` includes `WWW-Authenticate:
Bearer`; `403` only for valid principal with missing scope; `404` for missing/foreign-owned entity; `429` includes bounded `Retry-After`;
`503` means safe retry of read. Never reuse browser CSRF errors for bearer calls. `request_id` is correlation-only and safe to report.

Initial limits (configuration with a reviewed default, not buried constants): customer PAT 120 requests/minute and 10 concurrent requests;
operator PAT 60/minute and 5 concurrent; additional per-account aggregate 600/minute customer and 300/minute operator across tokens. Apply
both token and account buckets, then endpoint cost classes for expensive list/stats. Use token id/account id keyed limits, not raw token/IP;
IP limits may supplement anonymous malformed-token defense but are not the main tenant quota. Emit shared OTEL metrics for accepted, denied,
throttle, latency, cursor failure, and export lag; avoid labels containing token id, account id, email or resource id. Audit operator calls
through existing protected audit port with a correlation id; ordinary customer reads use aggregate telemetry, not durable per-read account
history.

CORS is disabled by default: integration calls are for server-to-server and native clients. Because the website and `/api/v1` share one
origin, a browser can technically call same-origin, but PAT creation should display a warning and discourage embedding PATs in browser code.
Do not emit wildcard CORS or reflect `Origin`. If a vetted browser SDK later needs CORS, add an explicit customer-configured origin
allowlist and preflight policy after a threat review; never allow credentials. The app cookie API continues its strict same-origin policy.

### 2.6 Webhooks, deferred

No outbound webhook delivery in v1. Define compatible event names now (`account.updated`, `session.ended`, `asset.ready`,
`statement.created`) and use an outbox in phase 2. Subscription creation requires a separate `webhooks:manage` permission; receiver URL
HTTPS allowlist, DNS/IP rebinding defense, no private/link-local addresses, bounded redirects (prefer none), signed timestamped HMAC
payload, event id, delivery idempotency, retry schedule/dead letter, secret rotation and endpoint disable/replay UI. Signatures cover exact
raw bytes with key id and timestamp; consumers reject stale signatures. Do not dispatch internal operator events, private content or
provider-cost data to customer webhooks.

## 3. Session share pages, embeds and short links

### 3.1 What is shareable and who consents

A shared session is a separate, explicitly curated projection, not a replay or export. Proposed fields: campaign name or user-edited public
title, date, approximate duration, party roster limited to consenting participants, each included character's chosen public display
name/card, selected approved scene images, and host-authored highlights. No raw narration, private roleplay text, chat/transcripts, secret
notes, private images, location/device details, account identifiers, exact member activity, or other payer's data. Session outcomes and
highlights are not automatically generated from private game memory. Use a fixed template and explicit per-field selection; any generated
copy is reviewed by the host before publishing.

The host (current campaign host at creation time) can draft and issue a link. Co-hosts and players cannot publish for the campaign in v1;
they can request changes through normal campaign coordination. The creator must have campaign authority and an ended session. Every player
whose name, character name/card, image, or attributable quote is included must individually opt in for that exact preview. A participant can
opt out of public display without losing their in-game character; omit them or render an anonymous party count. Host consent alone cannot
publish another player's identity or contributed portrait. Selected scene images must be public-approved and have all relevant
creator/rights grants. Use no contribution, funder or spend details in v1. A future optional Ã¢â‚¬Å“total session costÃ¢â‚¬Â must show only
the retail amount and require explicit owner consent plus consent of every displayed payer; no platform fee/multiplier/cost breakdown.

This privacy-first consent flow is a launch design, not a condition to expose account functionality: shared pages are optional and off by
default. Revoking an individual consent invalidates any public projection that still includes that field; simplest safe behavior is to
suspend all active links for that session until the host edits and republishes a compliant version. The host can revoke all links at any
time. Link creation/revocation/consent events are audited as campaign actions, without storing narrative content in audit.

### 3.2 Link, rendering and embed contract

Create links as `https://dungeonfluxdnd.com/s/<opaque-token>`. Token is at least 192 random bits, base64url, has a non-secret recognizable
`dfs_` prefix and checksum, and maps only to a stored digest. It is generated once and shown only to the host. It is unguessable and not an
authentication credential. Persist digest, session id, creator, projection revision, created/expiry/revoked times, indexing choice, consent
snapshot/version, non-identifying aggregate view counter and moderation state. Token lookup uses constant-time digest compare after
fixed-shape parsing, uniform not-found behavior and request throttling. Never include link token in analytics, logs, referrers to third
parties, app API, or the embed's outbound links.

The server-rendered public summary at `/s/<token>` provides Open Graph and Twitter Card tags derived only from the public projection: title,
short host-authored description, approved image URL, canonical share URL. Do not include private text in HTML source or metadata. Default
`X-Robots-Tag: noindex, nofollow, noarchive`; creator may opt in to indexing only after a preview and consent checks, then emit appropriate
robots metadata. `noindex` is a crawler instruction, not access control: anyone with the link can view until expiry/revocation. Add a clear
Ã¢â‚¬Å“Report this pageÃ¢â‚¬Â action with a low-friction abuse category and request id; reporting can suspend a link for review under
configured thresholds, with operator review/audit.

Iframe URL: `https://dungeonfluxdnd.com/embed/s/<opaque-token>` renders a responsive card with title, campaign, date, duration, consenting
party/characters, selected art, and a link to the full `/s/` page. Use no cookies, session endpoints, player actions, scripts that send data
to DungeonFlux, third-party trackers or auto-playing media. A small inline/static stylesheet and no JavaScript is preferred. Light and dark
themes use `?theme=light|dark|auto`, default `auto`; invalid theme falls back to auto. Height is content-driven; document dimensions and
`postMessage` are not required initially. `frame-ancestors` must permit approved embedding by default? Choose a conservative default
`frame-ancestors 'self'`; creator may opt into embedding by enabling Ã¢â‚¬Å“Allow embedsÃ¢â‚¬Â and then use `frame-ancestors https:` (all
HTTPS origins), with no wildcard `*` or arbitrary registered origin required in phase 1. If product later requires domain allowlisting,
store exact normalized origins and construct CSP from that list. Public page itself is not frameable unless embed is enabled. Set
`X-Content-Type-Options: nosniff`, strict `Referrer-Policy: no-referrer`, restrictive `Permissions-Policy`, and CSP default-src none;
style-src self; img-src self plus the first-party media gateway; base-uri none; form-action none; frame-ancestors per setting. Embed page
must use `sandbox="allow-same-origin"` only if required by parent measurement; otherwise host should iframe with `sandbox="allow-scripts"`
omitted (scripts absent) and `loading="lazy"`.

Support oEmbed discovery from the canonical share page via `<link rel="alternate" type="application/json+oembed" href="...">`. oEmbed
endpoint is `GET /oembed?url=<canonical-share-url>&format=json`, returns version, type `rich`, title, provider name/url, thumbnail URL,
width/height and sanitized HTML containing only the iframe. Accept only same-origin canonical `/s/<token>` URLs; do not dereference
arbitrary URLs. Restrict height/width to configured bounds and escape URL/attributes. No oEmbed discovery for revoked, expired, non-public
or embed-disabled links.

### 3.3 Lifecycle and privacy controls

Default link expiry: 90 days (configuration), creator may select 7, 30 or 90 days; no indefinite link at launch. Host may renew by issuing a
new link after rechecking current host role and participant consents. Revoke is immediate; purge digest after the configured
tombstone/abuse-retention period, while preserving only minimal security audit data. Use `Cache-Control: public, max-age=60,
stale-while-revalidate=300` for valid public projections; revocation/consent changes must purge/shorten cache through controlled CDN
invalidation or use short TTL. Set `no-store` on invalid-token/error pages and any link-management preview containing nonpublic draft
fields. Include projection revision in cache key/ETag; never shared-cache a management response. Metrics count page/embed hits by link id in
an aggregate counter with retention; no cookies, local storage, fingerprinting, IP-based identity, or viewer-level history. Scrub IP from
request logs or retain only under the shared security retention rules; public view counts are approximate and may include crawler requests,
clearly labeled.

Rate-limit token misses by IP prefix and global budget, but do not expose exact lookup distinctions. Public link tokens can leak through
screenshots or browser history; expiry and revocation are the containment. Configure referrer policy so visiting a share page never sends
the token URL as referrer. Keep third-party fonts/analytics out. If external scene image delivery is unavoidable, proxy via the first-party
media gateway and use a public projection asset grant; never put private object-store URLs in HTML.

Campaign page and character card links: defer campaign pages because membership/role/active campaign details create broader privacy and
membership changes. A character card is a cheap follow-up only after character ownership and portrait rights contracts support independent
public projection and revocation. Neither is in v1 acceptance; do not reuse the session-link token endpoint with arbitrary content types.

## 4. Crates, modules, persistence and screens

### 4.1 Crate ownership

Keep HTTP composition in `crates/df-portal/src/` beside `app_api/`:

- `integration_api/mod.rs`, `router.rs`, `auth.rs`, `guard.rs`, `handlers.rs`, `wire.rs`,
  `errors.rs`, `limits.rs`.
- `share/mod.rs`, `router.rs`, `tokens.rs`, `projection.rs`, `handlers.rs`, `headers.rs`,
  `oembed.rs`.
- Compose `/api/v1`, `/s`, `/embed/s`, and `/oembed` in the existing portal router. Do not add JS/TS
  application logic. Render public summary as server HTML (or Rust-generated static template) to
  support social link scrapers; account app remains Rust/WASM.

Token lifecycle policy and typed scope definitions belong in private `df-customer` types/service methods because that crate owns account
identity/session status, but avoid expanding the browser auth type into a generic API principal. Account-scoped session list data comes from
`df-customer`; campaign session/member projection from `df-session`/S1; asset metadata and safe delivery from `df-assets`/O2; funding and
statements from F through existing finance port; admin grant/audit decisions through B's operator directory; OTEL remains shared through
existing instrumentation. Add narrow typed ports to `df-portal` that compose these owners. Never query another crate's tables directly from
route handlers.

### 4.2 Migrations

Workspace migrations currently end at `0024_finance_boundary.sql`. Proposed next assignments, subject to allocator confirmation immediately
before build:

1. `0025_integration_tokens.sql` in account schema: token id, principal kind and account/operator
   id, label, scopes, pepper key id, digest, created/expiry/revoked/last-used times, grant
   generation, and indexes by principal, expiry, token id. Constraints enforce valid life and
   owner-kind fields; no plaintext column. Audit records use existing protected audit storage unless
   its owner approves a dedicated link; do not create a competing audit log.
2. `0026_share_links.sql` in campaign/session owner schema: link id, token digest, session id,
   creator account id, projection revision, expiry/revocation, indexing/embed flags, consent-version
   snapshot, moderation state, aggregate view count and tombstone time. Index digest uniquely and
   session/expiry for revocation/sweep. No rendered private content or plaintext token persisted;
   current public projection is rebuilt from explicitly consented source fields or a separate
   bounded public projection owned by S1/O1.

Coordinate migration numbers and schema ownership with B, F, S1 and O1; numbers in this design are provisional and must not be assumed free.
Cross-owner IDs are validated at creation; no cross-owner foreign keys (decision 31). Cleanup is bounded, cancellable and observable; do not
mutate telemetry storage or put account data in the development queue SQLite.

### 4.3 Account-app surfaces and preview fakes

Add Settings Ã¢â€ â€™ Developer tokens in `df-account-app` with create flow: label, exact scopes, expiry, permission explanation, recent
re-auth, one-time reveal with copy action and explicit Ã¢â‚¬Å“shown onceÃ¢â‚¬Â text, then metadata-only list with last used/revoke. Require
CSRF and session guard. Explain that tokens are secrets for trusted tools and must not be embedded in public/browser code. Do not add API
token storage to browser local/session storage.

Add a Share button on ended session summary, host-only, opening a preview that shows the exact public page/card, fields and participant
consent status. Link cannot be created until all included players have opted in; offer omit-person path. Show expiry, indexing (off by
default), embed permission, and a one-time link reveal. List/revoke active shares in session settings. All app mutations remain `/app/api`
cookie+CSRF routes; no bearer tokens are presented to game participants.

Extend the existing `account_app_preview` fakes with deterministic customer/operator token stores, fake current operator grant/MFA state,
fake session/member and asset projections, fake finance reads, and fake share consent/revocation. Preview fake links must use a local
hostname and never be crawled; public preview must not run real mail, storage, payment or provider calls. Seed examples that demonstrate
denied unconsented character, revoked link, private asset exclusion, customer-safe statement and admin-only ledger detail. Mark all fake
data synthetic.

## 5. Ordered implementation lane and acceptance checks

Prerequisite: confirm P2 account detail inventory from `artifacts/tmp/lanes/P2/brief.md`; freeze the typed owner ports and exact role/scope
mappings with B, F, S1, O1 and O2; recheck global migrations. No endpoint can be considered complete until its owning port is connected to
durable data and account/campaign predicates, not just a fake helper. Implement in order:

1. **Contracts/security foundation:** define scope enums, customer/admin DTO separation, integration
   principal, error envelope, cursor and token digest types; add `0025` and token store/service.
   Customer Settings lifecycle routes first. Add fakes. Tests: API-01 scope allow/deny matrix;
   API-02 one-time token reveal and digest-only persistence; API-03 expiry/revocation/account
   closure invalidation; API-04 malformed/prefix/checksum/constant-time verifier behavior; API-05
   operator MFA/reason and audit creation/use/revocation; API-06 token/account rate limits and
   secret redaction.
2. **Customer API reads:** implement account, auth sessions, play sessions, assets, funding,
   statements and stats through typed owner ports, one route at a time. Tests: API-07 account
   self-only and optional email scope; API-08 session metadata excludes cookie/CSRF/IP; API-09
   campaign membership isolation and cursor binding; API-10 asset audience/quarantine filtering;
   API-11 finance projection contains no fee/multiplier/provider cost; API-12 top-up/spend paging
   stability and statement status; API-13 stats privacy/aggregation; API-14 ETag conditional read
   after authorization; API-15 errors, CORS absent, limits, `Retry-After`, and request-id behavior.
3. **Operator read API:** add admin token creation and scoped account, auth-session, play-session,
   asset, ledger and stats reads; map scopes to current capabilities/MFA and audit. Tests: API-16
   each route checks current grant at call time; API-17 no cross-account lookup leakage; API-18
   ledger cost fields inaccessible to customer token and visible only under `admin:ledger:read`;
   API-19 denied/successful calls audited without secret; API-20 account suspension, MFA loss and
   grant generation immediately invalidate; API-21 admin cursor/filter limits.
4. **Share persistence and management:** add `0026`, consent projection port and host management
   routes in `/app/api`; fake and real projection from ended session. Tests: SHARE-01 host/current
   membership and ended-state required; SHARE-02 each included participant's consent binds to exact
   preview/revision; SHARE-03 unconsented name/portrait/quote absent; SHARE-04 no roleplay text,
   private notes, payer or money fields in all public outputs; SHARE-05 revoke and consent
   withdrawal remove access; SHARE-06 expiry and bounded cleanup; SHARE-07 high-entropy token digest
   and enumeration-uniform response; SHARE-08 host edits invalidate old projection safely.
5. **Public pages/embed/oEmbed:** server HTML, metadata, CSP and cache semantics, abuse report and
   non-identifying counters. Tests: SHARE-09 OG/Twitter output contains only public projection;
   SHARE-10 noindex default and indexing opt-in; SHARE-11 embed CSP/frame policy and no
   cookies/scripts/third-party requests; SHARE-12 light/dark rendering and hostile title/image
   escaping; SHARE-13 oEmbed rejects arbitrary URL and disabled/revoked links; SHARE-14 public
   report and view-count throttling; SHARE-15 cache invalidation and referrer policy after
   revocation.
6. **Integrated evaluation:** same-origin app browser test and external iframe preview, issue/revoke
   tokens and links, inspect real responses/headers, verify session cookie behavior is unaffected,
   and inspect audio/media-independent session card at human-readable scale. No production token,
   public share, live processor, or third-party webhook is created for testing. Capture
   build/config/source revision and preview evidence under the lane's artifact path.

Owner items: **B/P** owns app token management UI, first-party routes, account token lifecycle and operator authentication/audit
integration; **F** owns safe finance read projections; **S1/O1** own campaign/session membership and share projection authorization/source;
**O2** owns asset visibility, rights and first-party public media delivery; **P** mounts portal routers and shares account-app Rust UI;
coordinator confirms endpoint inventory, migration assignment, scope names, limits/retention defaults and separate independent evaluator.
Identity provider or OAuth client setup is not required for phase 1 PATs. Phase 2 OAuth authorization-code + PKCE requires client
registration, consent/grant revocation, redirect validation, refresh-token policy and a new explicit design/build lane.

## 6. Open decisions / implementation gates

1. Verify the full customer/admin account detail inventory in P2 brief; this design includes the
   stated categories but cannot infer omitted fields.
2. Assign migration numbers after checking the current global allocator and owners' concurrent work;
   `0025`/`0026` are proposals only.
3. Confirm concrete owner port contracts and fields for sign-in session listing, play-session
   history, asset library and account stats. Existing wire types do not cover all requested
   resources.
4. Choose PAT limits/expiry defaults, pepper rotation operational contract, operator token maximum
   life, and last-used update granularity. Proposed values above need configuration/owner review
   before implementation.
5. Confirm existing operator MFA is sufficiently recent and cryptographically bound to token
   creation; current `OperatorGrant.has_second_factor` indicates presence, not freshness. Add typed
   proof if needed.
6. Resolve embed `frame-ancestors` policy: this design uses same-origin by default and opt-in
   all-HTTPS embedding. If customers need exact origin allowlists, add verified origin management
   before enabling arbitrary third-party embeds.
7. Confirm public projection storage/rebuild contract and CDN invalidation support; if prompt
   revocation cannot invalidate cached content, lower public cache TTL and keep a revocation gate at
   the edge.
8. Confirm abuse handling and public-link retention/erasure policy with service operations and
   counsel. Share tokens are bearer-style public access links, not access control for confidential
   sessions.

## 7. References to current source contracts

- `app_api/router.rs` defines `/app/api` endpoints; mount a sibling integration router without
  weakening its guards.
- `app_api/guard.rs` enforces session, origin, CSRF and body shape. Bearer auth requires a separate
  guard, not changes that permit a session cookie to satisfy `/api/v1`.
- `app_api/ports.rs` already separates finance, campaign, operator and review owners. Extend narrow
  read ports; do not bypass them with portal SQL.
- `app_api/admin.rs` checks current operator grant, second factor, capability and reason, and
  appends audit entries. Admin API should preserve these properties per read.
- `app_api/session.rs` returns a CSRF token in `SessionView`; do not reuse that DTO for integration
  API.
- `df-account-app/src/wire.rs` documents customer-safe versus admin ledger fields. `StatementsView`
  is cursor-paged; `LedgerEntry` includes `table_fee_micro_dollars`, `multiplier_permille`, provider
  and provider cost.
- `df-customer/src/access.rs` owns auth/session validation and current customer service; `tokens.rs`
  uses cryptographic random secrets, SHA-256 digests and constant-time comparisons. Create a
  separate domain type and keyed secret-digest scheme for PATs and public share tokens.


### 2.7 Reconciled P2 account/admin inventory

The P2 brief was reviewed. Its precise checklist is wider than the summary categories, so v1 resource coverage includes these fields and
distinctions:

| P2 area | Integration projection |
|---|---|
| Customer account details | `GET /account` includes email when separately scoped, display name, creation time, verified state, consent versions/times, sign-in method labels, closure/standing state. No password or provider subject. |
| Customer sign-in sessions | `GET /auth-sessions` lists device sessions. Revocation stays in `/app/api` for v1; external read credentials cannot revoke browser access. |
| Play history | `GET /play-sessions` includes campaign, start/end, duration, visible players, and this caller's retail session spend; newest first and cursor-paged. Player visibility follows the same campaign audience policy as the app. |
| Assets | `GET /assets` supports `kind` and `campaign_id`, and returns kind, campaign, created time, size, thumbnail through approved media URL scheme, and retention/hold state. Excludes quarantined or audience-inaccessible media. |
| Funding and top-ups | `GET /funding` includes purchased/promotional available, allocated and reserved amounts, campaign pots and pace. Statements include top-up and promo events, refunds and spending. No write/top-up mutation endpoint is added; existing hosted checkout stays in app API. |
| Customer statement history | Date-range and category filters, totals (`topped_up`, `promotional_granted`, `spent`, `refunded`), per-month and per-campaign spend aggregates, cursor-paged lines, and a CSV export endpoint or documented client-side bounded export. Only customer-visible paid amounts. |
| Customer dashboard statistics | `GET /stats` covers total and current-month hours, sessions, active/archived campaigns, characters, players hosted, assets generated, monthly/lifetime spend, and average spend per table-hour. Server aggregates through owning ports; averages omit zero-hour denominator and return `null`, never divide by zero. |
| Admin account record | Admin account detail covers unmasked email (separate sensitive capability and audited per read), created/verified/consent/standing/closure/sign-in methods, auth sessions, play sessions, assets, five balances, pots per campaign, pace, filtered ledger totals and monthly totals, refunds/disputes and account-targeted audit. |
| Admin account search | Search email/id/campaign; filters for standing, verified and has-balance; sorts by balance, last activity, lifetime top-ups or lifetime spend. Bounded query/page only. Every unmasked email view is durably audited. |

Add distinct admin scope `admin:accounts:pii:read` for unmasked email and consent/authentication metadata. `admin:accounts:read` returns
masked email and account status only. Require current `SupportInspect` plus this narrower grant, recent MFA per existing policy, and one
audit event on each unmasked read. This narrows current console behavior rather than treating list access as blanket PII permission. If
current `OperatorCapability` cannot represent the narrower grant, B must add it before integration routes ship.

For the account record and all customer settings, read projections omit mutable secrets and expose only status. Email/password changes,
sign-out, closure, email preferences, export requests and top-up remain first-party browser actions with re-auth/CSRF/idempotency rules.
`account:read` does not authorize mutations or create export jobs. CSV output contains only already-authorized filtered statement lines;
bound export to a configured maximum date range/row count, stream or page through the same port, and never put data in an unauthenticated
URL. Admin bulk exports are out of scope.

