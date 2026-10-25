# Customer accounts, funds, operator dashboard and business site

Date: 2026-10-24 (round 0b: revised after the critic's review)
Status: plan (agent B). No code exists for anything described here. Rounds 1-4 implement it in the
order at the end; each round is critic-reviewed before the coordinator commits it.

This plan answers the owner's request of 2026-10-24: billing, an admin dashboard, auth and customer
login, setting up an account, adding funds and settings for how fast the funds are spent, account
management, password resets, and the business web pages that present the service.

## 1. Governing documents and how this plan follows them

| Document | Sections followed here |
| --- | --- |
| `AGENTS.md` | Application architecture (Rust only, server authority, one crate per subsystem), Hosted-service authority (durable admission and native egress), Implementation and evaluation (fakes, owned processes, browser checks) |
| `planning/coding-style.md` | All of it; especially 6 (typed errors, integer money, injected clock), 7 (owned async, bounded queues, deadlines), 8.3-8.9 (choke points, no spend caps, audit lines, dev CLI views, owner-tunable files), 9 (tests), 11 (parameterized SQL, no credentials in diagnostics) |
| `planning/commerce-service.md` | Independent authority and boundaries; Account and tenant lifecycle (closure, deletion, export, restricted tenants); Payment state machine (webhook inbox, verify raw body, dedupe, no ordering assumption, return URL proves nothing, chargeback and refund lead to Restricted); Atomic spend (top-ups are deferred liability, consumption is earned, reversals append). Deviations are listed in 15.2. |
| `planning/service-operations.md` | Recovery and operator access (separate operator auth with MFA, time-limited scoped roles with a reason, refunds a separate permission, redacted support views); Tenant isolation (explicit tenant predicates, nonexistence-safe errors, paired cross-tenant fixtures); Additional security (origin allowlist, cookie, CSRF, rate ceilings); Retention classes (challenge tokens 15 min) |
| `planning/commercial-validation.md` | Credit cash is deferred liability until consumed; no claim of demand or profit |
| `planning/pricing-and-costs.md` | Customer price separate from supplier cost; idempotent purchase/consumption/refund/webhook handling; no secrets in billing logs |
| `planning/c-df-commerce-d01..d03-decision.md` | `df-commerce` stays pure and the sole policy authority; candidates commit atomically with an idempotent result; Unknown never releases liability; never charge above consent, overruns are platform loss |
| `planning/c-df-auth-d01..d03-decision.md` | Principal, tenant, payer, campaign and operator are separate capabilities; credentials resolve only at the current generation; rotation fences prior grants; payment never grants game access; a cached grant cannot authorize admission (15.2) |
| `planning/g04-d01-decision.md` | Payer is an account principal independent of any game seat; recovery is single-use, short-lived, bounded, identical public response; success increments credential generation and revokes prior grants |
| `planning/g04-d02-decision.md` | Player and shared-display roles; host is a capability; target browsers |
| `planning/bootstrap-origin-policy.md` | Host-only `Secure; HttpOnly` cookie, no `Domain`, no long-lived credential in a URL, exact configured origin, CSRF on cookie-authenticated mutations, trusted-proxy rule |
| `planning/typed-identity-policy.md` | New role IDs are distinct 16-byte `df-types` identities with the canonical byte validation |
| `planning/storage-architecture.md` | PostgreSQL for durable state, bounded pool, explicit deadlines, no transaction spans an external call |
| `planning/rpc-api.md` | Customer and payment boundary (identical anonymous responses, checkout return is never paid confirmation, native HTTPS webhook isolated from game endpoints); operation identity, fingerprints and the closed rejection codes |
| `planning/client-architecture.md`, `c-df-web-d01/d03` | Thin clients; no JS/TS application code; role and route are presentation only, never permission |
| `planning/remote-play.md` | Players join as guests; accounts never become seats |
| `planning/observability.md` | One shared audit/OTEL path, no credentials or private content in logs, success and failure paths both visible |
| `planning/subsystem-architecture.md` | Crate edges; async ports belong to their consumer's crate and adapters depend on them; composition wires adapters; browser crates never depend on server crates |

## 2. Decisions already made (coordinator, 2026-10-24; recorded, not reopened)

1. Identity is email plus password first. Argon2id with OWASP parameters, a per-user salt,
   constant-time verification, password length 12 to 128 characters, and a breached/common-password
   check with no network. Email verification is required before adding funds. Passkeys and managed
   identity are later work, kept possible by the credential port.
2. Sessions are a host-only `Secure; HttpOnly; SameSite=Lax` cookie holding an opaque random token;
   only its hash is stored; idle and absolute expiry; rotation on login and privilege change; logout
   revokes; "sign out everywhere". CSRF protection on every state-changing form. Login, signup and
   reset are rate limited per account and per IP.
3. Password reset uses a single-use, short-lived opaque token stored hashed, bounded attempts and an
   identical public response whether or not the account exists. Success bumps the credential
   generation and revokes other sessions (G04-D01). Email change needs the current password and
   confirmation on both addresses. The lifetime is 15 minutes, the `service-operations.md` retention
   class for challenge tokens (coordinator, round 0b; replaces the brief's 30 minutes).
4. Email delivery is a port. The development adapter writes each message to a bounded outbox under
   `artifacts/`, with a dev CLI read. A real provider adapter is later work. No live email.
5. Funds are prepaid credit through a hosted checkout redirect (Stripe Checkout; the browser never
   sees card data), behind a `PaymentGateway` port with a faithful fake for every test. The real
   adapter follows Stripe's documented API and is exercised only by the owner later, in test mode.
   Webhooks verify the signature over the raw bounded body, persist to an inbox before acknowledging,
   dedupe by event id, and handle out-of-order and duplicate delivery. No live calls, no real keys.
   Subscriptions are later work.
6. "How fast the funds get burnt" is a customer-chosen spend pace, never a platform-imposed cap:
   quality tier, picture cadence, voice on/off, and an optional per-hour pace. The account page shows
   the balance and an estimated play time left from the cost engine's prices. When funds run out the
   game finishes what was already admitted, then pauses into a safe read-only state with a top-up
   prompt; no hidden lockout, no lost committed outcome.
7. The admin dashboard is a separate platform-operator capability: customers, balances, ledger,
   top-ups, refunds (through the gateway port, re-authentication required), spend by campaign and
   provider (from the cost engine's records), webhook inbox state, and an audit log of every operator
   action. Operators never see players' private game data or credential material.
8. The business site: public pages plus sign up, log in, verify email, reset password, account pages
   and admin pages. Server-rendered HTML from Rust plus static CSS; no JavaScript or TypeScript
   application code. `docs/` stays separate. Photographic live-action fantasy look; unambiguous
   controls; responsive to phone width; accessible.

Coordinator decisions on the round 0 questions (2026-10-24):

9. The customer rate card is a DRAFT placeholder, never shown as a price; the owner sets real
   pricing later.
10. The zero-balance pause applies only to tables paired with an account while the portal is on;
    owner and local play never pause. It stops all new optional paid work, not only player actions
    (9.2).
11. The hourly pace is soft: it steps optional media down and never pauses.
12. Customers may choose a slower picture cadence than 30 s; the default stays REF-068's 30 s;
    visible changes still redraw at once.
13. The `argon2` dependency (RustCrypto) is approved. Downloading a common-password list is not
    approved yet: the check sits behind a small port with a short hand-curated built-in list; the
    full list is an owner-approved follow-up.
14. Calls whose outcome is unknown are absorbed by the platform, not charged.
15. Legal terms, tax and the refund policy are a paid-launch gate, not part of this task.
16. Agent B adds the role IDs to `df-types` in Round 1, small and additive.

Coordinator decisions after the second review (round 0c, 2026-10-24):

17. Paid admission uses a durable prepaid reservation drawn down when a billed call begins
    (`BilledCall::new`) with the call's estimate and trued up when it is settled or dropped. On a
    paired table `BilledCall::new` can refuse, and every call site handles the refusal. The
    after-the-fact cost-log consumer is the reconciliation and conservation check, not the bound.
    Local and owner play hold no reservation and are never refused (8.4, 15.2).
18. Launch accepts card payments only (`payment_method_types = ["card"]`), so there is no
    delayed-payment path; delayed methods (bank debits, vouchers) are later work.

## 3. What exists today, and the layout decisions that follow

### 3.1 Current code

- `df-auth` is pure policy (`df-types` only): guest bootstrap deduplication, membership admission,
  credential fences (`rotation.rs`), account recovery (`recovery.rs`, whose `recover_credentials`
  enforces single use, expiry, bounded attempts and a generation bump), operator permissions
  (`permissions.rs`, `OperatorPermission::{SupportInspect, Refund, Deletion, ...}`). No hashing,
  tokens, storage or I/O.
- `df-commerce` is pure policy: `credit.rs` models top-up principal, deferred and earned credit,
  refunded principal and refund holds with exact `Money`; `ports.rs` defines synchronous,
  subscription-shaped `CommerceRepository` and `PaymentGateway` traits with no implementation.
- `df-server` has no `Cargo.toml`, is not a workspace member and is not the running server. The
  running server is `df-tools` `gameplay::serve` (axum), launched by `df-launcher`; game routes are
  under `/gameplay/*`, `/pkg/*` and asset paths; `/dev/*` is the loopback dev control surface.
- The cost engine (`df-tools/src/gameplay/cost.rs`, agent C) meters each billed call in exact
  micro-dollars and appends one `CostEvent` JSON line per call to
  `development/runs/<database>/cost.jsonl`. `CostEvent` is `pub(in crate::gameplay)`. The writer is
  an owned thread fed by a bounded queue (`WRITE_QUEUE = 1024`); a full queue keeps the in-memory
  totals and counts a write failure (the line is lost from the file); the writer flushes but does
  not fsync.
- `df-persistence` uses `tokio-postgres`; each store has its own database and versioned migrations;
  Postgres tests read an env var naming an owned scratch server.
- Media choices are process-wide startup settings in `df-tools`; a running server hosts one table.

### 3.2 Layout decisions

1. **Server-rendered portal.** Coordinator decision 8 replaces the mounted `df-ui` panel plan of
   `commerce-service.md` for the portal; plain forms on one origin need no browser transport (that
   path was blocked on G02) and work with `HttpOnly` cookies and CSRF tokens.
2. **Same process and origin as the game.** The portal router is merged into the gameplay router
   behind `DF_PORTAL=on` (default off; off changes nothing).
3. **Pure policy stays pure.** Rules without I/O go into `df-auth` and `df-commerce` with supplied
   times and values.
4. **Ports belong to the consumer** (`subsystem-architecture.md`). `df-customer` defines its store
   and email ports; `df-commerce` keeps `PaymentGateway`. `df-persistence` and `df-providers`
   implement them and depend on the port crates, never the reverse. Only composition (today
   `df-tools` `gameplay::serve`; later `df-server`) constructs adapters and hands them to the
   services.
5. **The native facade gets its own crate, `df-customer`,** because `df-server` does not build.
   It can move into `df-server` mechanically when that crate becomes the composition binary.
6. **The web surface gets its own crate, `df-portal`,** with no database, gateway or `df-tools`
   types. Cargo edges enforce this: `df-portal` does not depend on `df-tools`,
   `df-persistence` or `df-providers`.
7. **Billing never meters calls.** It consumes the cost engine's records through a shared record
   type (4.5) and turns them into customer credit consumption (8.4). The cost engine stays the
   single meter (coding style 8.3).
8. **Players do not need accounts.** Seats stay guest identities (G04-D01, remote play).
9. **Separate service database.** Accounts and billing live in their own database `df_service`,
   because an account outlives every per-run game database. This deviates from commerce's shared
   admission transaction; admission is bounded instead by a durable prepaid reservation
   (decision 17, 8.4), and the residual deviation is stated in 15.2.

## 4. Crates, modules and edges

### 4.1 Dependency edges (added to `subsystem-architecture.md` in Round 1)

| Crate | Direct project dependencies |
| --- | --- |
| `df-metering` (new) | none (serde, serde_json only; no I/O) |
| `df-customer` (new) | `df-types`, `df-auth`, `df-commerce`, `df-observe`, `df-metering` |
| `df-portal` (new) | `df-types`, `df-observe`, `df-customer` |
| `df-persistence` (edge added) | existing list plus `df-customer` (to implement its store ports) |
| `df-providers` (unchanged edges) | already depends on `df-commerce`; implements `PaymentGateway` |
| `df-tools` (edges added) | plus `df-metering` (cost writer), `df-customer`, `df-portal` (composition) |

No browser crate depends on any of these.

### 4.2 `df-auth` (existing, pure; additions)

| Module | Public items | Contents |
| --- | --- | --- |
| `password_policy.rs` | `PasswordRefusal`, `check_new_password` | 12..=128 Unicode scalar values (at most 512 bytes); refuses a password equal to the email or its local part (ASCII case-insensitive); refuses members of a supplied common-password lookup (the caller's port). |
| `web_session_policy.rs` | `SessionLifetimes`, `SessionCheck`, `check_session` | From stored created/last-seen/revoked/generation facts, the account's current generation and a supplied `now`: Active, IdleExpired, AbsoluteExpired, Revoked or StaleGeneration; whether `last_seen` needs a touch; whether a recent-authentication window is open. |
| `throttle.rs` | `ThrottlePolicy`, `ThrottleDecision` | Fixed-window decision for one key with supplied time; the bounded map lives in `df-customer`. |
| `recovery.rs` (reused) | unchanged | `recover_credentials` is the reset policy, run over a transaction snapshot that implements `AccountAuthority` (7.3). |
| `permissions.rs` (two additive variants) | `OperatorPermission` | Existing `SupportInspect` (read-only views), `Refund`, `Deletion` (account deletion on request, 7.5); new `AccountRestriction` (restrict and unrestrict, a separate permission per `service-operations.md` "Recovery and operator access") and `OperatorEnrolment` (re-enrol another operator's lost TOTP device, 10.1). |

### 4.3 `df-commerce` (existing, pure; additions)

| Module | Public items | Contents |
| --- | --- | --- |
| `credit.rs` (additions) | `CreditEvent::ExternalReversal`, `CreditEvent::ReserveUsage`, `CreditEvent::ConsumeReserved`, `CreditEvent::ReleaseUsage`, `CreditState::usage_reserved` | Refunds made outside the portal and lost disputes reduce available credit and record refunded principal; any part already consumed is returned as a shortfall for the caller to record as platform loss (8.5). Disputes in progress use the existing `HoldRefund`/`ObserveRefund` hold. The prepaid reservation (8.4) is a second hold beside refund holds: the invariant becomes `refund_hold + usage_reserved <= deferred_credit`; `ReserveUsage` moves available credit into it, `ConsumeReserved` turns reserved credit into earned credit, `ReleaseUsage` returns the unused part. |
| `standing.rs` | `AccountStanding { Active, Restricted { reason } }`, `RestrictionReason`, `standing_after` | Commerce lifecycle: a dispute, an external refund that exceeded available credit, or an operator restriction makes a tenant Restricted (no new top-ups or paid play; reads and export still work). |
| `topup.rs` | `TopupOffer`, `TopupRefusal`, `validate_topup` | Presets and a custom whole-dollar range from the tunables file; whole cents; one currency. |
| `customer_rate.rs` | `CustomerRate`, `customer_amount` | Supplier micro-dollars to customer micro-units with the rate card; rounds toward the customer (down) with checked arithmetic. |
| `funds.rs` | `FundsState { Funded, Low, Exhausted }`, `funds_state` | From credit state, estimated cost per hour and the low-funds threshold. |
| `pace.rs` | `SpendPace`, `QualityTier`, `PictureCadence`, `HourlyPace`, `pace_decision`, `play_time_left` | Customer settings as closed enums; the soft governor decision (9.3); exact integer play-time arithmetic. |
| `ports.rs` (amended) | `GatewayOperationKind::CreateTopupCheckout { amount }`, `create_topup_checkout`, `inspect_checkout`, `inspect_refund` | Additive one-time-payment variants. The traits are synchronous and unimplemented; Round 2 proposes `async fn` with the same outcome enums. The amendment goes to the critic and coordinator before code depends on it. |

### 4.4 `df-customer` (new, native; the account and billing facade)

Third-party dependencies: `argon2` (approved), `getrandom`, `sha2`, `hmac`, `subtle`, `tokio` (sync,
time, rt), `serde_json` (export). No axum, no HTML, no SQL.

| Module | Responsibility |
| --- | --- |
| `lib.rs` | `CustomerService`, `BillingService`, `OperatorService`, their typed errors and config. |
| `ports.rs` | Consumer-owned async ports: `AccountStore`, `BillingStore`, `OperatorStore`, `EmailSender`, `CommonPasswords`, `CostTotalsSource` (the cost book's per-run totals and its write-failure count, implemented by composition from `df-tools`). Each method takes a deadline and returns a typed outcome that keeps Unknown distinct from Failed. |
| `clock.rs` | `Clock` with `SystemClock`; `FakeClock` in the test-fakes feature. |
| `password_hashing.rs` | Argon2id (m = 19 MiB, t = 2, p = 1) in `spawn_blocking` behind `MAXIMUM_CONCURRENT_HASHES = 4` with a 5 s wait deadline (then typed Busy); PHC strings; rehash on login when parameters change. |
| `common_passwords.rs` | `BuiltInCommonPasswords`: a short hand-curated list of common passwords of 12+ characters compiled in, lowercased, binary-searched. A full list behind the same port is an owner-approved follow-up. |
| `tokens.rs` | 32-byte random tokens, base64url text, SHA-256 digests for storage, constant-time comparison. Token types implement neither `Debug` nor `Display`. |
| `email.rs` | `Message` and builders per purpose; `DevOutbox` adapter (7.4); the mail worker: one owned task fed by a bounded queue (`MAIL_QUEUE_CAPACITY = 256`, a full queue rejects the request with a "try again" page and one audit line, never a silent drop). |
| `throttle.rs` | Bounded per-key counters (`MAXIMUM_TRACKED_KEYS = 65_536`, oldest window evicted first). Keys are a keyed hash (HMAC-SHA256 with a per-process random key) of the normalized email or of the client IP, never an account id, so unknown addresses throttle exactly like known ones. |
| `accounts.rs` | Signup, verification, login, logout, sessions, password change, reset, email change, closure, export. |
| `billing.rs` | Checkout, return lookup, webhook processing, reconciliation, wallet and history reads, refunds, disputes, pairing. |
| `prepaid.rs` | The table's prepaid tranche: durable reservation and refill through `BillingStore` (8.4), and the in-process `df_metering::PrepaidDraw` it backs. |
| `usage_debit.rs` | The cost-record consumer: consumes reserved credit for recorded calls, reconciles and releases tranches, keeps the conservation check (8.4). |
| `pace.rs` | Loads and validates `content/spend-pace.json`; keeps `TablePace` and `FundsState` per table in `tokio::sync::watch` values the game reads. |
| `operators.rs` | Operator login with TOTP (RFC 6238, 30 s step, one step of skew, used steps refused), enrolment confirmed by a first code, lost-device re-enrolment, seal-key rotation, scoped time-limited grants, reasons, re-authentication, audit (10.1). |
| `retention.rs` | The owned maintenance task, following the `service-operations.md` retention classes: expired sessions and tokens; operation receipts kept 90 days, then replaced by a nonreplayable namespace tombstone; deletion of closed accounts after the window (7.5) with redaction of free-text fields that concern them; raw webhook bodies after 30 days. |
| `dev_view.rs` | Read models behind the dev CLI views (13.3). |
| `fakes.rs` (feature `test-fakes`) | Faithful in-memory implementations of the store ports with the same uniqueness, revision and append-only rules, used by unit and router tests; the Postgres adapters must pass the same contract tests. |

Every task (mail worker, webhook worker, reconciliation, usage consumer, retention) is held in one
`JoinSet` owned by the services and stopped on the server's shutdown signal; nothing is detached.

### 4.5 `df-metering` (new, tiny shared metering contract crate)

The contract between the cost engine (writer and drawer) and billing (reader and funder). No I/O.

- `CostRecord` (the serialized form of one `cost.jsonl` line: call id, sequence, run, session,
  at_ms, provider, model, cause, detail, usage source, lines with quantity and optional
  micro-dollars, total micro-dollars), `RunTotals` (the closing line a cleanly closed run writes:
  record count, supplier total, write failures), `CostRecordError`, `encode_line`, `decode_line`.
  Provider, category, cause and source are closed enums with their wire names. The cost engine
  writes through `encode_line` and the billing consumer reads through `decode_line`: one codec, no
  drifting duplicate.
- `PrepaidDraw`: the in-process, lock-free view of one table's durable prepaid tranche, in supplier
  micro-dollars. `draw(estimate_micro_usd) -> Result<DrawTicket, DrawRefusal>` subtracts with a
  compare-and-swap loop and refuses when the remaining tranche is smaller than the estimate, and
  refuses a zero estimate outright (`DrawRefusal::ZeroEstimate`; a paid table never draws zero);
  `true_up(ticket, actual_micro_usd)` corrects by the difference (the remaining tranche may go below
  zero by at most the difference, which billing records as overrun); `release(ticket)` returns a
  not-billed call's estimate; `credit_back(micro_usd)` returns an unknown-outcome call's estimate
  once billing has recorded it as not charged (8.4). A `DrawTicket` is not `Clone`; the
  `BilledCall` holds it in an `Option` and exactly one of `settle`, `not_billed` or `Drop` takes it. `remaining()`, `drawn_since_refill()` and a `Notify` for "below the
  refill threshold" serve the billing side. No allocation per call, no await, no I/O.

Agent C owns the `cost.rs` changes that write `CostRecord`/`RunTotals` and call `PrepaidDraw`
(request in 4.9). Until they land, Round 2 tests the consumer against fixture files written with
`encode_line` and the draw against a fake call loop.

### 4.6 `df-portal` (new, native; the web surface)

Depends on `df-customer`, `df-types`, `df-observe`, `axum`, `tower`, `http`.

| Module | Responsibility |
| --- | --- |
| `lib.rs` | `PortalConfig` (validated origin and switches), `router(config, services) -> axum::Router`, `webhook_router(services) -> axum::Router` (separate router, own `DefaultBodyLimit` of 256 KiB, no shared state or layers with game routes), `dev_routes(services)` (merged into the token-guarded `/dev/*`). |
| `html.rs` | Escaping HTML writer; every dynamic value escaped; no template engine. |
| `layout.rs` | Page chrome, skip link, error summary, labelled form fields with `aria-describedby`. |
| `security.rs` | Cookies, CSRF, Origin and `Sec-Fetch-Site` checks, security headers, `no-store`; the request-span and access-log path records the route template, never the query string, so `?token=` never reaches logs or spans. |
| `site/` | Public pages. |
| `auth_routes.rs`, `account_routes.rs`, `admin_routes.rs` | Route handlers (section 6). |
| `webhook_routes.rs` | `POST /webhooks/payments/stripe` (8.3). |
| `fake_checkout_routes.rs` | The simulated hosted checkout, mounted only with the fake gateway. |
| `assets/portal.css`, `assets/` | Token-driven static CSS and business-site photographs from the project's existing public art. |

### 4.7 `df-persistence` (existing; additions)

New files `account_store.rs`, `billing_store.rs`, `operator_store.rs` implementing the
`df-customer` ports, their Postgres tests, and migrations `0005_accounts.sql`, `0006_billing.sql`,
`0007_operators.sql` for the `df_service` database; one `pub mod` line each in `lib.rs`.

### 4.8 `df-providers` and `df-types` (additions)

- `df-providers`: `payment_stripe.rs` (Checkout Session create and retrieve, Refund create and
  retrieve, Dispute retrieve, webhook signature verification; caller deadlines; `Idempotency-Key` =
  operation id; refuses `sk_live_`/`rk_live_` keys outright) and `payment_fake.rs` (8.2).
- `df-types` (Round 1, by agent B, additive): `AccountId`, `TenantId`, `CampaignId`, `WebSessionId`,
  `CheckoutId`, `OperatorId` with the existing `identity!` macro and validation.

### 4.9 Files owned by other agents (requested, not edited)

| File | Owner | Hunk |
| --- | --- | --- |
| `Cargo.toml` (workspace) | coordinator | members `df-metering`, `df-customer`, `df-portal` |
| `df-tools/src/gameplay/cost.rs` | C | (a) Serialize through `df_metering::encode_line`; fsync after each written batch (at most every second); `close_run` writes a `RunTotals` line; expose per-run totals and the write-failure count for `CostTotalsSource`. (b) The prepaid draw (decision 17, 8.4): `SpendLedger` holds `Option<Arc<PrepaidDraw>>` (None on local/owner play and unpaired tables); `BilledCall::new` prices its estimate with the existing `Price::micro_usd` and, when a draw is present, calls `draw`; it returns `Result<BilledCall, FundsRefusal>`, which is always `Ok` when no draw is present; `settle` calls `true_up` with the metered amount; `not_billed` calls `release`; `Drop` (unknown outcome) keeps the estimate drawn until billing credits it back (8.4). On a paid table an estimate on a `Metering::Priced` route that prices to zero is refused (`FundsRefusal::ZeroEstimate`, one `BILLING draw_refused reason=zero_estimate` line), never drawn as zero. (c) Estimates in the route's priced categories: today `imagery.rs` (`charge`, about line 1843) and `character_clips.rs` (about line 389) estimate `(Calls, 1)`, which `content/pricing.json` does not price for gpt-image-2.5 or BFL, so they would draw zero. They must estimate image output tokens from the requested size and quality, image and text input tokens from the reference images and prompt length, and BFL credits from the model's credit schedule. The storyteller estimate (`storyteller.rs`, about line 1107) must add the output-token ceiling (`TextOutput` at the request's maximum output tokens) to its input estimate. (d) Order: the draw happens before any unit counter or side record is charged (`ledger.reserve(IMAGE_UNIT, 1).commit(1)` and `calls.fetch_add` in `imagery.rs`, `kit.charge(Unit::TextToken, ..)` in `storyteller.rs`, the `cost.json` write in `character_clips.rs`), so a refused call is never counted as made. (e) The nine `Metering::Pending` routes (`cost.rs` about lines 1793-1857: embeddings, speech-to-text, segmenter, head locator, plate judge, music, Kling video, World Labs, Jev) build no `BilledCall` today; metering them is C's backlog (C-B1, C-B2, C-B5) and a prerequisite of paid play (8.8). |
| every metered `BilledCall::new` / `begin_call` site (storyteller, approach model, imagery, character clips, voice; the Pending routes once metered) | C with W2 / W4 / W6 / N | Handle `FundsRefusal` exactly like the capability's existing provider-unavailable outcome: prepared or silent fallback, one audit line, no retry, no stall. The storyteller skips its retry loop on a refusal, and a refused admitted action gets the defined narration fallback (9.2). |
| `df-tools/src/gameplay.rs` | W4 | composition: build adapters and services when `DF_PORTAL=on`, merge `router` and `webhook_router`, pass the shutdown signal |
| `dev_control/mod.rs`, `dev_cli.rs` | W4 | merge `dev_routes`; `dev accounts/outbox/billing/webhooks/operator-audit` subcommands |
| `actor.rs` (admission) | W4 | read the funds `watch` at player-action admission; pause state (9.2) |
| `resources.rs` (hub `get_or_request`/`render`), plate cadence, `imagery.rs`, `voice.rs`, music, look-ahead | C / W2 / W4 | read the funds and pace `watch` values where new paid work is requested and where quality, cadence and voice are chosen (9.1, 9.2) |
| TV and phone clients | W5 | funds notices and the pairing code |
| `df-launcher` | W8 | `DF_PORTAL_*` lines when the owner enables the portal |

## 5. PostgreSQL schema and migrations (database `df_service`)

Money columns are `numeric(39,0)` micro-units with `CHECK (x >= 0)`; currency `char(3)`; times
`timestamptz` from the service clock; IDs `bytea` with `CHECK (octet_length(id) = 16)`. Queries are
parameterized; every tenant-owned query carries an explicit `tenant_id` or `account_id` predicate
taken from the authenticated session, never from a form or URL. Unique and foreign-key violations map
to typed, nonexistence-safe errors. Append-only tables reject `UPDATE` and `DELETE` with a trigger.
Each migration is one transaction recorded in `schema_migration`.

### 5.1 `0005_accounts.sql` (Round 1)

| Table | Key columns | Notes |
| --- | --- | --- |
| `account` | `account_id` pk, `email_normalized` unique null, `email_display` null, `email_tombstone` bytea null, `email_verified_at`, `display_name`, `status` (`active`, `closing`, `closed`), `closed_at`, `delete_after`, `credential_generation`, `revision`, `created_at` | Email trimmed and ASCII-lowercased, at most 254 bytes. On deletion the email columns are cleared and only a keyed digest remains (7.5). |
| `password_credential` | `account_id` pk/fk, `phc_hash`, `updated_at` | Deleted on account deletion. |
| `web_session` | `session_id` pk, `token_digest` unique, `account_id`, `kind` (`customer`, `operator`), `credential_generation`, `created_at`, `last_seen_at`, `idle_expires_at`, `absolute_expires_at`, `recent_auth_until`, `revoked_at`, `revoke_reason` (closed set), `csrf_secret`, `device_label` (phone, tablet, desktop, unknown) | No raw token. |
| `account_token` | `token_id` pk, `token_digest` unique, `account_id`, `purpose` (`verify_email`, `password_reset`, `email_change_old`, `email_change_new`), `email_change_id`, `credential_generation_at_issue`, `created_at`, `expires_at`, `consumed_at`, `attempts`, `attempt_limit` | Reset 15 min (decision 3), verification 24 h, email change 24 h. A new token of a purpose consumes older ones. |
| `email_change` | `email_change_id` pk, `account_id`, new address, `old_confirmed_at`, `new_confirmed_at`, `completed_at`, `cancelled_at`, `expires_at` | Completes only when both are confirmed and the address is still free. |
| `operation_receipt` | pk (`namespace`, `principal`, `operation_id`), `fingerprint`, `outcome`, `result_ref`, `created_at`, `expires_at` (created + 90 days) | Form idempotency (6.4). Kept 90 days (`service-operations.md` retention), then replaced by `retired_operation_namespace` so the key can never execute again. |
| `retired_operation_namespace` | pk (`namespace`, `principal`), `retired_through` | Any operation recorded on or before `retired_through` whose receipt is gone is refused as `OperationExpired`, never executed. |
| `account_event` | `event_id`, `account_id`, `at`, `kind` (closed set), `device_label` | Append-only security history shown to the customer; no email, IP or token. |
| `export_request` | `export_id` pk, `account_id`, `requested_at`, `ready_at`, `expires_at`, `byte_length` | The export file itself is generated on demand (7.5). |

### 5.2 `0006_billing.sql` (Round 2)

| Table | Key columns | Notes |
| --- | --- | --- |
| `tenant` | `tenant_id` pk, `owner_account_id` unique, `standing` (`active`, `restricted`), `restriction_reason`, `created_at` | One personal tenant per account, created at email verification. |
| `wallet` | `tenant_id` pk, `currency`, `revision`, the `CreditState` amounts including `usage_reserved`, `updated_at` | `CHECK` enforces the `CreditState` invariants (`refund_hold + usage_reserved <= deferred_credit`); updates use `WHERE revision = $expected`. |
| `prepaid_reservation` | `reservation_id` pk, `tenant_id`, `campaign_id`, `run`, `session` (the cost book's session: one per server start), `reserved_customer`, `reserved_supplier_micro_usd`, `consumed_customer`, `state` (`active`, `releasing`, `released`), `created_at`, `last_refill_at` | One active reservation per (campaign, run, session); each refill adds to it in one transaction with `ReserveUsage`. |
| `run_totals` | pk (`campaign_id`, `run`, `session`), `snapshot_at`, `record_count`, `supplier_micro_usd`, `write_failures`, `closed_cleanly` | The cost book's totals for one server session, copied every minute and at close (from `CostTotalsSource` and the `RunTotals` line), so a crash cannot hide a conservation gap. |
| `ledger_entry` | `entry_id`, `tenant_id`, `operation_id`, `kind` (`topup_deferred`, `consumption_earned`, `consumption_reversed`, `refund_held`, `refund_unknown`, `refund_paid`, `refund_failed`, `external_refund`, `dispute_held`, `dispute_won`, `dispute_lost`), `amount`, `currency`, `wallet_revision_after`, `source_kind` (`checkout`, `usage`, `refund`, `gateway_event`), `source_ref`, `created_at` | Unique (`tenant_id`, `operation_id`) and unique (`tenant_id`, `source_kind`, `source_ref`, `kind`), so one source can never be applied twice under different operation ids. `source_ref` per source kind: `checkout`, the checkout id; `usage`, the text `<campaign_id>/<run>/<call_id>` (the same triple as the `usage_debit` key, so two runs or two tables reusing a call id stay distinct); `refund`, the refund id; `gateway_event`, the gateway event id. Append-only. |
| `checkout` | `checkout_id` pk, `tenant_id`, `operation_id` unique, `gateway`, `gateway_session_ref` unique, `payment_intent_ref` unique null, `charge_ref` unique null, `amount`, `currency`, `status` (`creating`, `open`, `paid`, `expired`, `failed`, `unknown`), `refunded_amount`, `disputed_amount`, `created_at`, `idempotency_expires_at` (created + 24 h), `expires_at`, `paid_at` | `refunded_amount + disputed_amount <= amount`. The payment intent and charge references are stored in the crediting transaction because refund and dispute events name them, not the session. |
| `gateway_inbox` | pk (`gateway`, `event_id`), `event_type`, `object_ref`, `received_at`, `raw_body` (<= 256 KiB), `status` (`received`, `applied`, `ignored`, `failed`), `attempts`, `next_attempt_at`, `last_error`, `processed_at` | Inserted before the 200; raw body deleted 30 days after processing. |
| `campaign` | `campaign_id` pk, `game_key` unique, `display_name`, `created_at` | A table. |
| `payer_grant` | `campaign_id`, `tenant_id`, `revision`, `granted_at`, `revoked_at`; one active grant per campaign | Transfer is later work. |
| `pairing_code` | `code_digest` pk, `campaign_id`, `created_at`, `expires_at` (10 min), `consumed_at`, `attempts` | 8.8. |
| `spend_pace` | `campaign_id` pk, `tier`, `picture_cadence`, `voice_enabled`, `hourly_pace`, `revision`, `updated_at`, `updated_by_account` | |
| `usage_cursor` | pk (`campaign_id`, `run`), `file_identity` (digest of the first complete line), `byte_offset`, `last_sequence_by_session` (session to last sequence), `state` (`following`, `halted`), `halted_reason`, `halted_at` | One cursor per file: the run is the game database name and its `cost.jsonl` spans server restarts (`cost::open_run`), each restart a new session. 8.4. |
| `usage_debit` | pk (`campaign_id`, `run`, `call_id`), `tenant_id`, `at`, `provider`, `model`, `cause`, `usage_source`, `supplier_micro_usd`, `unpriced_line_count`, `customer_amount`, `charged_amount`, `ledger_entry_id` | One row per cost record; admin spend views read it. |
| `platform_loss` | `loss_id`, `campaign_id`, `run`, `call_id`, `amount`, `reason` (`overrun_beyond_reservation`, `unknown_outcome_not_billed`, `unpriced_usage`, `external_reversal_shortfall`, `dropped_cost_lines`, `accrued_while_halted`, `run_not_closed`), `at` | Commerce D03 and decision 14: never charge above consent; absorbed losses are recorded, never silent. |

### 5.3 `0007_operators.sql` (Round 3)

| Table | Key columns | Notes |
| --- | --- | --- |
| `operator_grant` | `grant_id` pk, `account_id`, `permissions` (closed values), `granted_at`, `expires_at` (at most 30 days later), `granted_by` (`cli`), `ticket` (required reason or ticket text), `revoked_at` | Time-limited, scoped; renewal is a new grant. A grant does not change `operator_totp` (10.1). |
| `operator_totp` | `totp_id` pk, `account_id`, `secret_sealed`, `seal_key_id`, `state` (`pending`, `active`, `revoked`), `created_at`, `activated_at`, `revoked_at`, `revoked_by`, `revoke_reason`, `last_step`; at most one `pending` and one `active` per account | Sealed with the key named by `seal_key_id` (10.1). |
| `operator_enrolment` | `enrolment_id` pk, `totp_id`, `token_digest` unique, `expires_at` (15 min), `consumed_at` | The single-use link that shows a pending secret once. |
| `operator_audit` | `entry_id`, `account_id`, `grant_id`, `at`, `action` (closed set), `target_account`, `target_campaign`, `reason` (required for every mutating action and every email reveal, 1..500 characters), `outcome`, `request_id` | Append-only; every operator request adds one row. |
| `refund_request` | `refund_id` pk, `operation_id` unique, `checkout_id`, `tenant_id`, `amount`, `status` (`held`, `sent`, `paid`, `failed`, `unknown`), `account_id` (operator), `reason`, `created_at`, `updated_at` | |

Runtime role: `SELECT, INSERT, UPDATE` on mutable tables, `SELECT, INSERT` on append-only ones;
migrations use a separate owner role. Row-level security is a recorded deployment gate.

## 6. HTTP routes and their rules

### 6.1 Common rules

- **Origin.** `DF_PORTAL_ORIGIN` is the one canonical external origin (https; `http://127.0.0.1` or
  `http://localhost` with a port only for loopback development). Redirects and checkout return URLs
  are built from it, never from `Host` or forwarding headers.
- **Cookies.** `__Host-df_session=<token>; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=<absolute>`;
  operators use `__Host-df_operator`; anonymous forms use `__Host-df_presession` (1 h). No `Domain`.
  Each cookie is accepted only on its own route family. A token arriving before login is never
  promoted; login always issues a new one.
- **CSRF.** Every `POST` carries `csrf = HMAC-SHA256(csrf_secret, form_name)` of the current session
  or pre-session (constant-time compare); `Sec-Fetch-Site: cross-site` is refused; a present `Origin`
  must equal the configured origin. No state-changing `GET`.
- **Headers.** `Content-Security-Policy: default-src 'none'; style-src 'self'; img-src 'self';
  form-action 'self' https://checkout.stripe.com; frame-ancestors 'none'; base-uri 'none'` (no script
  at all), `X-Content-Type-Options: nosniff`, `Referrer-Policy: same-origin` (`no-referrer` on token
  pages), HSTS on https; `Cache-Control: no-store` on account, operator and token pages.
- **Logging.** Spans and access logs carry the route template and request id only; query strings,
  form bodies and cookies are never recorded.
- **Limits.** Portal form bodies at most 16 KiB; the webhook router has its own 256 KiB limit,
  enforced before buffering more; 10 s handler deadline.
- **Errors.** Domain refusals re-render the form with an error summary; storage or gateway trouble
  renders a safe "try again" page with the request id. Another tenant's id is indistinguishable from
  a missing one.

### 6.2 Route table

Auth: P public, A anonymous form, C customer session, V verified email, R recent authentication
(password re-entered within 10 min), O operator session (password plus TOTP, current grant), O+R
operator re-authenticating in the form, S gateway signature, D dev-control token on loopback.

| Method and path | Auth | Rate limit (keys are keyed hashes) | Round |
| --- | --- | --- | --- |
| `GET /`, `/how-it-plays`, `/pricing`, `/faq`, `/terms`, `/privacy` | P | none | 4 (stubs in 1) |
| `GET /portal/portal.<hash>.css`, `/portal/assets/*` | P | none | 1 |
| `GET/POST /signup` | A | 5/h per IP; 3/h per email | 1 |
| `GET/POST /verify-email` | A or C | 10/min per IP | 1 |
| `POST /verify-email/resend` | C | 3/h per email | 1 |
| `GET/POST /login` | A | 5 failures/15 min per email, 20/15 min per IP | 1 |
| `POST /logout` | C | none | 1 |
| `GET/POST /password-reset` | A | 3/h per email, 10/min per IP | 1 |
| `GET/POST /password-reset/confirm` | A | 10/min per IP; token attempt limit 5 | 1 |
| `GET/POST /email-change/confirm` | A or C | 10/min per IP | 1 |
| `GET /account`; `GET/POST /account/profile`; `GET /account/security` | C | profile 30/h per account | 1 |
| `POST /account/security/password`, `/account/security/email` | C plus current password | 5/15 min, 3/h per account | 1 |
| `GET /account/sessions`; `POST /account/sessions/revoke`, `/revoke-others`, `/revoke-all` | C | none | 1 |
| `GET /account/data`; `POST /account/data/export`; `GET /account/data/export/{id}` | R | 3/day per account | 1 (billing parts 2) |
| `POST /account/close`; `POST /account/close/cancel` | R | 3/day per account | 1 (balance rule 2) |
| `GET /account/funds`, `/account/funds/history?cursor=` | C | none | 2 |
| `POST /account/funds/checkout` | V, tenant Active | 10/h per account | 2 |
| `GET /account/funds/return?checkout=`, `/account/funds/cancelled?checkout=` | C | none | 2 |
| `GET/POST /account/pace` | C (payer of the table) | 30/h per account | 2 |
| `GET /account/tables`; `POST /account/tables/pair`, `/unpair` | C (pair needs V) | pair 10/h per account | 2 |
| `POST /webhooks/payments/stripe` (webhook router) | S | 600/min per IP | 2 |
| `GET /dev-checkout/{id}`; `POST /dev-checkout/{id}/pay`, `/cancel` | A, fake gateway only | none | 2 |
| `GET/POST /admin/login`; `POST /admin/logout` | A plus TOTP; O | 5 failures/15 min per email, 10/15 min per IP | 3 |
| `GET /admin`, `/admin/customers?q=&cursor=`, `/admin/customers/{id}`, `/admin/customers/{id}/ledger` | O `SupportInspect` | none | 3 |
| `POST /admin/customers/{id}/reveal-email` | O `SupportInspect`, reason | 30/h per operator | 3 |
| `GET /admin/top-ups`, `/admin/refunds`, `/admin/disputes`, `/admin/spend?by=`, `/admin/webhooks`, `/admin/conservation`, `/admin/audit` | O `SupportInspect` | none | 3 |
| `POST /admin/customers/{id}/refunds` | O+R `Refund`, reason | 10/h per operator | 3 |
| `POST /admin/customers/{id}/restrict`, `/unrestrict` | O+R `AccountRestriction`, reason | 10/h per operator | 3 |
| `GET/POST /admin/enrol?token=` | A plus password and the first TOTP code | 10/min per IP | 3 |
| `POST /admin/operators/{id}/reenrol` | O+R `OperatorEnrolment`, reason | 5/h per operator | 3 |
| `POST /admin/customers/{id}/delete` | O+R `Deletion`, reason | 10/h per operator | 3 |
| `POST /admin/webhooks/{gateway}/{event}/retry` | O `SupportInspect`, reason | 30/h per operator | 3 |
| `GET /dev/accounts`, `/dev/outbox[/{n}]`, `/dev/billing[/tables]`, `/dev/webhooks`, `/dev/operator-audit` | D | none | 1-3 |
| `POST /dev/billing/fake-event` | D, fake gateway only | none | 2 |

`/admin/*` exists only with `DF_PORTAL_ADMIN=on`.

### 6.3 Enumeration-safe responses and equal timing

- Signup always hashes the submitted password first (equal Argon2 cost), then attempts the insert;
  a new address and an existing one both enqueue exactly one mail job (verification link, or "someone
  tried to sign up with your address") and render the same page.
- Reset requests do no lookup in the handler: the handler checks the throttle (keyed on the email
  hash) and enqueues a reset job for every request; the mail worker looks the address up, issues the
  token and writes the message, or discards the job for an unknown address. Response time does not
  depend on existence.
- Login failure for an unknown address, a wrong password or a closed account renders one message;
  unknown addresses run one dummy Argon2 verify. An unverified account can log in to resend
  verification but cannot add funds.
- Tests compare bodies and statuses byte for byte for each pair and assert that throttling an unknown
  address behaves exactly like throttling a known one (13.4 lists the tests).

### 6.4 Idempotent form posts

Every mutating form carries a hidden `operation` (16 random bytes) recorded in `operation_receipt`
with a fingerprint of the normalized fields (for password fields, a keyed hash of a digest, never the
password). A double submit returns the first outcome; the same operation with different fields is
`OperationConflict`. Checkout and refund use the operation id as the gateway `Idempotency-Key`.

## 7. Accounts and authentication (Round 1)

### 7.1 Signup and verification

1. Validate the email syntax (one `@`, non-empty parts, no whitespace or control characters, at
   most 254 bytes) and the password policy (common passwords through the `CommonPasswords` port).
2. Hash, then in one transaction insert `account`, `password_credential`, a `verify_email` token,
   the receipt and an `account_event`; a unique-email conflict rolls back and enqueues the "already
   registered" mail instead (6.3).
3. The verification link opens a page with one "Verify my email" button (link scanners cannot
   verify). Verification marks `email_verified_at`; Round 2 also creates the tenant and wallet in
   that transaction.

### 7.2 Login, sessions and logout

- Login verifies with Argon2id, rehashes if needed, inserts a new `web_session` with fresh token and
  CSRF secret, clears the pre-session cookie and returns `303 /account`.
- Lifetimes (owner-tunable in `content/portal.json`): customer idle 14 days, absolute 30 days;
  operator idle 15 min, absolute 8 h; recent-authentication window 10 min; `last_seen` touched at most
  every 5 min.
- Each request runs `check_session` (the stored generation must equal the account's).
- Re-entering the password for a sensitive action rotates the session (new token, old row revoked).
- Logout revokes the current row; "sign out of other devices" revokes the others; "sign out
  everywhere" revokes all, including the current one.

### 7.3 Password change, reset and email change

- Password change needs the current password, bumps `credential_generation`, revokes other sessions
  and rotates the current one.
- Reset: the mail worker issues a `password_reset` token (15 min, attempt limit 5). The confirm page
  posts the token and the new password twice. The service loads token, account and credential rows
  `FOR UPDATE` into a snapshot implementing `df_auth::recovery::AccountAuthority`, runs
  `recover_credentials`, writes the new hash, consumes the token, revokes all sessions and records
  the event in one transaction.
- Email change needs the current password and a free new address (if taken, the same page is shown
  and nothing goes to the new address). Tokens go to both addresses; when both confirm within 24 h
  the address changes, the generation bumps, other sessions are revoked, and both addresses get a
  notice. The old address can cancel.

### 7.4 Email port and dev outbox

`EmailSender::send(message, deadline)` is called only by the mail worker. The dev outbox writes
`<n>-<purpose>.eml` atomically (temporary name, then rename) under `DF_PORTAL_DEV_OUTBOX` (default
`artifacts/dev-outbox/`), keeps at most 200 messages (`MAXIMUM_OUTBOX_MESSAGES`, oldest removed
first). A failed send is one `EMAIL failed` line; the account operation stays committed and the
person can resend. Email bodies are the only place a token appears.

### 7.5 Closure, deletion and export (commerce account lifecycle)

- **Export.** With recent authentication, `POST /account/data/export` builds a JSON document on
  demand (bounded to 8 MiB, served as an attachment with `no-store`, link valid 24 h) containing the
  profile, security events, sessions (device labels and times), and from Round 2 the ledger,
  checkouts, tables and pace settings. It never contains game content, other people's data, password
  hashes or tokens. A Restricted tenant can still export (commerce lifecycle).
- **Closure.** With recent authentication, `POST /account/close` sets `closing`: all sessions are
  revoked, tables are unpaired, and no new top-ups are possible. From Round 2: with a positive
  balance the closure page says the remaining balance needs an operator refund (refund policy is a
  paid-launch gate, decision 15) and records a support request visible on the operator dashboard;
  with a zero balance closure completes. The account can be reopened by logging in during a 30-day
  window (`delete_after`).
- **Deletion is blocked until money is settled.** Neither the retention task nor an operator
  deletes an account while it has a positive available balance, an active prepaid reservation, a
  refund that is `held`, `sent` or `unknown`, or an open dispute. The closure page and the operator's
  customer page say exactly what is outstanding and what settles it ("Your remaining balance of
  $12.40 must be refunded first"; "A dispute on your 2026-10-20 top-up is still open"); the retention
  task skips such accounts and lists them on the operator dashboard.
- **Deletion.** Once settled and after the window, the retention task deletes the password
  credential, sessions, tokens, email changes, security events and export requests, clears the email
  (keeping a keyed digest so the same address can sign up again without reviving the old record), and
  keeps the tenant, wallet, ledger, checkouts and debits pseudonymously, as billing records
  (`service-operations.md` retention; jurisdiction periods are a legal gate). An operator with
  `Deletion` can run the same deletion at once on a request, with a reason, under the same settlement
  rule. Shared game campaigns are not touched; a paired table simply loses its payer.
- **Redaction of free text.** Operator `reason` and grant `ticket` fields and refund reasons are the
  only free text that can mention a person. Operators are told on every reason field not to enter
  personal data; and when an account is deleted, every such field on rows that target it is replaced
  with `[redacted after deletion]` (the target id, action and time stay).

## 8. Funds (Round 2)

### 8.1 Adding funds

1. Presets (default 10, 25, 50, 100 USD) and a custom whole-dollar amount (default 10 to 500),
   owner-tunable; the page shows what the amount buys at the current pace. Verified email and an
   Active tenant are required.
2. `POST /account/funds/checkout` validates, inserts a `checkout` row (`creating`), then calls
   `create_topup_checkout` outside any transaction (10 s deadline, idempotency key = operation id,
   `client_reference_id` = checkout id, `mode = payment`, `payment_method_types = ["card"]`
   (decision 18), so a completed session is paid at once and there is no delayed-payment path). Known success stores the reference (`open`) and returns
   `303` to the hosted page; Unknown sets `unknown` and shows "We could not confirm the checkout was
   created; try again" (8.6).
3. The return page never credits; it shows the stored status, refreshing every 5 s with
   `<meta http-equiv="refresh">` while `open`.

### 8.2 The fake gateway

`payment_fake.rs` behaves like Stripe Checkout for one-time payments: create returns a session and a
URL on the portal's own `/dev-checkout/{id}` page with "Pay (test)" and "Cancel"; paying emits
`checkout.session.completed` signed in Stripe's `Stripe-Signature` format with the fake secret and
delivers it through the real webhook handler (raw bytes, in process); retrieve returns the session
state; refunds and disputes can be scripted (succeeded, failed, pending then succeeded, no answer;
dispute opened, won, lost). Test hooks deliver twice, out of order, drop the first delivery, or make
create time out after recording the session. The fake is the default (`DF_PORTAL_GATEWAY=fake`) and
**the portal refuses to start with the fake gateway unless both the configured origin and the
server's listen address are loopback** (origin host `127.0.0.1`, `[::1]` or `localhost`; listen
address `127.0.0.1` or `::1`). The origin check alone is not enough: the launcher binds `0.0.0.0`
when phones are on the network (`df-launcher/src/server.rs`), and a LAN client could then post
`/dev-checkout/{id}/pay` without any `Origin`. The listen check uses the same `settings.listen`
value the dev control surface checks (`dev_control::setting_on(settings.listen)` in `gameplay.rs`),
so a fake checkout can never credit funds on a reachable server. The Stripe adapter needs `DF_PORTAL_GATEWAY=stripe` plus key and webhook-secret
files, refuses live keys, and is not run in these rounds.

### 8.3 Webhooks and crediting

1. The webhook router reads at most 256 KiB, verifies `Stripe-Signature` (timestamp within 5 min,
   any `v1=` HMAC-SHA256 over `"{t}.{raw body}"` matching a configured secret in constant time;
   several secrets allowed for rotation), parses only id, type and object reference, inserts the inbox
   row (`ON CONFLICT DO NOTHING`) and returns 200. Bad signatures return 400 and one audit line;
   nothing is stored.
2. The webhook worker claims `received` rows (`FOR UPDATE SKIP LOCKED`) and re-reads the
   authoritative object from the gateway (10 s deadline; `credit.rs` refuses webhook-only evidence).
3. Checkout events: only a session that is complete and paid, with matching amount, currency and our
   `client_reference_id`, produces `SettledTopup` with `VerifiedLatest`. One transaction locks the
   wallet, runs `propose_credit_adjustment`, updates the wallet at the expected revision, inserts the
   ledger entry (unique by operation and by (`checkout`, checkout id, kind)), stores the session's
   payment intent and charge references on the checkout row, marks the checkout `paid` and the inbox
   row `applied`. Repeats find the entry (`AlreadyRecorded`) and change nothing.
   Order does not matter because each event re-reads the object.
4. Failures back off (10 s, 1 min, 10 min, 1 h; `failed` after 8 attempts) and show on the operator
   webhook page.

### 8.4 Spending: the prepaid reservation, and the cost records as reconciliation

**The bound: a durable prepaid reservation drawn at `BilledCall::new` (decision 17).**

- *Reserve.* When a paired table has available credit (at startup, at pairing, after a top-up), the
  billing service reserves a tranche in one `df_service` transaction: `ReserveUsage` moves customer
  credit into `usage_reserved` and the `prepaid_reservation` row records it with its supplier
  equivalent (customer amount converted back through the rate card, rounded down, so the draw can
  never spend more supplier money than the customer reserved). Tranche size is the larger of 15
  minutes of estimated play and $1, limited by the available balance (tunable in `portal.json`). Only
  after the commit is the amount added to the table's `df_metering::PrepaidDraw`.
- *Draw.* `BilledCall::new` prices its estimate (the cost engine's existing `Price::micro_usd`) and
  draws it from the `PrepaidDraw`: a compare-and-swap on an atomic, no I/O, no await. If the
  remaining tranche is smaller than the estimate it returns `FundsRefusal` and the call site takes
  the capability's existing provider-unavailable fallback (4.9). `settle` trues the draw up to the
  metered amount; `not_billed` releases it. `Drop` with an unknown outcome keeps the estimate drawn
  until the usage consumer has recorded that call as `unknown_outcome_not_billed` (decision 14); it
  then calls `credit_back`, so the estimate returns to the table mid-session instead of waiting for
  the session to end. An estimate on a priced route that prices to zero is refused, never drawn as
  zero (4.5). Calls on `Metering::Unpriced` routes (metered, deliberately unpriced) draw nothing and
  are booked as `unpriced_usage` platform loss.
- *Refill.* An owned refill task wakes when the remaining tranche falls below half of it and reserves
  another tranche durably, then adds it to the draw. The tranche is sized so that a refill (one
  `df_service` transaction, target under 100 ms) completes long before the remaining half is spent;
  a refusal while a refill is in flight and unreserved balance remains is counted as
  `draw_refused reason=refill_pending` and alerted on the operator page, because it means the
  tranche is too small. If the wallet cannot fund a refill the table moves to `Low` and then
  `Exhausted` (9.2).
- *Completion margin.* Unit: supplier micro-dollars, like the draw. Per-action cost is the p95 of the
  table's measured supplier cost per player action in this session (cost records grouped by the
  causing hero action); until 20 actions are measured it is the tier's reference per-action cost from
  `content/spend-pace.json`. The margin is that per-action cost times (the number of player actions
  admitted and not yet finished, plus one), recomputed at each admission and completion; the game's
  admission point reports the in-flight count through the same shared funds state it reads. When the
  unreserved balance is zero and the draw's remaining falls to the margin, the table goes
  `Exhausted`: new player actions and optional work stop, and the margin pays for the media of
  actions already admitted. The draw refuses only when the margin is gone too, so a metered call
  site that misses the funds `watch` still cannot spend beyond the reservation.
- *Bound.* The customer is never charged more than was durably reserved. The only overrun is a
  true-up in which a call's actual cost exceeded its estimate after the tranche was empty; billing
  records it as `overrun_beyond_reservation` platform loss. Worst case: the sum over calls in flight
  at that moment of actual minus estimate; Round 2 measures it per exhaustion (audit line and
  dashboard).
- *Local and owner play* hold no reservation: `SpendLedger` has no draw and `BilledCall::new` always
  succeeds (decision 10).
- *Crash and restart.* The reservation is durable; the draw is memory. The run is the game
  database name and survives restarts; each server start is a new cost-book session, and the
  reservation belongs to (campaign, run, session). At start, reservations of earlier sessions move to
  `releasing`; the usage consumer finishes debiting those sessions' records, then `ReleaseUsage`
  returns the unconsumed remainder to the customer. Calls drawn but never recorded (lost with the process) are not charged
  to the customer; the reserved-but-unaccounted amount is the upper bound of a `run_not_closed`
  conservation gap.

**Reconciliation and conservation: the cost records.**

- *Source.* The usage consumer reads `cost.jsonl` of each run through `df_metering::decode_line`. It
  never meters.
- *Cursor.* Per (`campaign_id`, `run`) it stores the file identity (digest of the first complete
  line), the byte offset after the last complete line, and the last sequence of each session (the
  file spans restarts; call ids are `<session>-<n>`). It advances only past
  a newline-terminated line; a partial last line waits for the next pass. A file shorter than the
  offset, a different first line, or an undecodable line halts the cursor.
- *A halted cursor fails closed.* Halting sets the table's funds `watch` to `Exhausted` (the pause in
  9.2), freezes the draw at zero so every further `BilledCall::new` refuses, stops refills, raises one
  `BILLING cursor_halted` alert (operator page and `dev billing`), and records everything the cost
  book accrued after the last debited record (from `run_totals`) as `accrued_while_halted` platform
  loss; nothing of it is charged. An operator resumes, with a reason, only after inspecting the file
  (re-verified identity and offset) or closing the run.
- *Debit.* Each record becomes one `usage_debit` keyed by (`campaign_id`, `run`, `call_id`); its
  ledger `source_ref` is the same triple, `<campaign_id>/<run>/<call_id>` (5.2), so two runs or two
  tables reusing a call id stay distinct. Records before pairing are not billed. Priced lines convert
  with `customer_amount` (rounded toward the customer) and are applied with `ConsumeReserved` against
  the run's reservation; an amount beyond what is left of it is `overrun_beyond_reservation`. Unpriced
  lines are recorded as `unpriced_usage` platform loss, never as a silent zero. `source=unknown`
  records are `unknown_outcome_not_billed`.
- *Totals survive a crash.* Every minute and at close, the consumer copies the cost book's totals
  for the current session and its write-failure count (`CostTotalsSource`, and the `RunTotals` line
  `close_run` writes) into `run_totals`, keyed (campaign, run, session). A session that ended
  without `closed_cleanly` is a `run_not_closed` conservation gap, bounded by that session's
  reservation; it cannot pass silently.
- *Dropped lines.* The cost writer can lose lines (full queue) and does not fsync today (fsync is
  requested from agent C, 4.9). The conservation check catches losses: a gap between `run_totals`
  and the debits is recorded as `dropped_cost_lines` platform loss and alerted.
- *Unmetered routes.* The nine `Metering::Pending` routes (4.9 e) never reach the cost book, so
  neither the draw nor the conservation check can see them. Until C meters them, their spend is an
  explicit platform-loss class, `unmetered_route`, of unknown amount: the dashboard and `dev billing`
  list the Pending routes and say that their spend is not visible. Paid pairing is refused while any
  route is Pending (8.8).
- *Conservation check* (in `dev billing` and on `/admin/conservation`): per session, cost-book
  supplier total = debited supplier amounts + unpriced + unknown + dropped + accrued-while-halted; per
  reservation, consumed + released + unreleased remainder = reserved; per tenant, ledger sums by kind
  equal the wallet columns, `topup_principal = deferred + earned + refunded` and
  `usage_reserved` = the sum of active reservations' unconsumed amounts. A mismatch shows in red with
  the difference.
- The consumer runs every second and is idempotent; replaying a file adds nothing.

### 8.5 Refunds outside the portal and disputes

- Refund and dispute events name a charge or payment intent, not a session; they are matched to
  the checkout through the `charge_ref` and `payment_intent_ref` stored when it was credited (8.3).
  An event whose charge matches no checkout is marked `ignored` and shown on the operator webhook
  page.
- `charge.refunded` (for example a refund from the Stripe dashboard): after re-reading the charge,
  the refunded amount not already recorded by a portal refund becomes `ExternalReversal`: available
  credit is reduced and recorded as refunded principal (`external_refund` entry); any part already
  consumed is a shortfall recorded as `external_reversal_shortfall` platform loss and the tenant
  becomes Restricted (commerce lifecycle) until an operator reviews it.
- `charge.dispute.created`: the disputed amount is held (`dispute_held`; a hold reduces available
  credit, like a refund hold) and the tenant becomes Restricted. `charge.dispute.closed` won releases
  the hold (`dispute_won`); lost applies `ExternalReversal` (`dispute_lost`, shortfall as platform
  loss). Restriction is lifted only by an operator with a reason. Restricted tenants keep reads and
  export; their tables pause like an empty balance.

### 8.6 Reconciliation

Every 15 min and at startup, the reconciliation pass looks at checkouts in `creating`, `open` and
`unknown` older than 2 min:

- Within 24 h of creation (`idempotency_expires_at`), it repeats the create call with the same
  idempotency key and fingerprint; Stripe returns the original session for a repeated key within 24
  h, so a `creating` row (crash before the call finished) or an `unknown` one learns its session
  reference, and an `open` one is retrieved by reference to catch a lost webhook.
- After 24 h the key can no longer be replayed. A row with a session reference is retrieved by
  reference. A row without one cannot be found by our id; it is resolved only by a webhook carrying
  our `client_reference_id`, and is otherwise marked `expired` (Checkout sessions expire within 24
  h, so no payment can still arrive).
- Refunds in `unknown` are retrieved by the same key until known.

### 8.7 Balance, estimate and history

Available balance (deferred credit minus holds), display rounded down to the cent; estimated play
time at the current pace and at each tier; funds state; standing; a paginated history from
`ledger_entry` (25 rows per page, opaque cursor scoped to the session's tenant).

### 8.8 Tables and pairing

A game server with the portal on registers its `campaign` row at startup (keyed by its database
identity). The host menu on the display shows an 8-character pairing code (unambiguous alphabet, 10
min, single use, stored hashed, 5 attempts); the host enters it on `/account/tables`, creating the
`payer_grant`. An unpaired table runs exactly as today and is never charged.

**Paid pairing requires complete metering.** Composition passes the billing service the cost
engine's route table (`cost::BILLED_ROUTES`) as a coverage fact. While any route is
`Metering::Pending`, pairing with the Stripe gateway is refused ("This server cannot bill play yet")
and `dev billing` names the Pending routes. With the fake gateway (loopback only, 8.2) pairing is
allowed so Round 2 can be exercised, and the Pending routes' spend is the `unmetered_route` class
(8.4). A coverage test in `df-tools` (requested from C) fails if a route is added without a
`Metering` value, and a `df-customer` test pins that a Pending route refuses Stripe pairing. Showing the code is a
W5 request; `dev billing tables` prints it so Round 2 can be tested first.

## 9. Spend pace and the game (Round 2)

### 9.1 Settings

| Setting | Values | Game effect (where it is read) |
| --- | --- | --- |
| Quality tier | Economical, Standard, Premium | Picture quality and size (imagery profile and `ImageQuality`); voice model (Economical and Standard `eleven_flash_v2_5`, Premium `eleven_multilingual_v2`) |
| Picture cadence | Scene changes only, Relaxed (about 60 s), Standard (REF-068, about 30 s, the default) | The periodic same-scene plate refresh only; visible changes redraw at once |
| Voice | On, Off | Narration and character voices; captions remain |
| Hourly pace | None, or dollars per hour | The soft governor (9.3) |

Tiers, their media mapping and reference usage per hour live in `content/spend-pace.json`
(owner-tunable, validated at load, per-knob fallback with an audit line). The game reads
`watch::Receiver<TablePace>` where these choices are made today; a watch read is a small copy with
no I/O.

### 9.2 Out of funds (and Restricted)

`Exhausted` or a Restricted tenant pauses the paired table:

- Work already admitted finishes: the player action being resolved commits, and its media is paid
  from the completion margin of the reservation (8.4); anything beyond the reservation is absorbed.
- No new paid work starts. Player actions are refused with the existing `Paused` rejection, and the
  same funds `watch` is read wherever new paid work is requested without a player action: periodic
  plate refreshes, look-ahead and prefetch, timer-driven music and clips, NPC timers that call the
  storyteller, story-memory summaries and embeddings, and splat worlds. Those requests return the
  existing prepared or "unavailable" fallback, never a stall. The hub request point (`resources.rs`)
  and the timer sites are hunks requested from their owners (4.9). Behind the watch, the draw itself
  refuses once the margin is spent, so the bound holds for every metered route even where a site is
  missed; the Pending routes build no `BilledCall`, which is why paid play waits for them (8.8).
- If a call for an action already admitted is refused (the margin was too small), the action's
  rules outcome is still committed and shown, and its narration uses the defined fallback: the
  committed outcome's plain summary line (the deterministic, logged outcome text the view already
  carries) followed by the pause notice. The storyteller does not enter its retry loop for a refusal.
  This is a `BILLING draw_refused reason=margin` line and counts against the margin rule (8.4).
- A halted usage cursor (8.4) pauses the table the same way.
- The display and phones show "This table is out of funds. The host can add funds at
  <origin>/account/funds"; nothing is lost; committed outcomes stay.
- A credited top-up (or an operator lifting a restriction) returns the table to `Funded` and play
  resumes.
- `Low` (under 30 min of estimated play, tunable) shows a notice on the host's phone only.
- Owner and local play (`DF_PORTAL=off`, or an unpaired table) never pause (decision 10).

### 9.3 The hourly pace governor (soft)

Above the customer's hourly pace (trailing 60-minute customer spend, `pace_decision`), the table's
effective settings step down one level at a time (cadence first, then tier); they step back up below
80 % of the pace. It never pauses and never refuses a line. Each step is a `PACE` line and is shown
on the account page.

### 9.4 Estimated play time left

Cost per hour is the table's measured customer spend per hour of play at the current settings once
30 min of such play exists, otherwise the tier's reference usage priced with `content/pricing.json`
and the DRAFT rate card. Play time left = available balance / cost per hour, whole minutes, integer
arithmetic; labelled an estimate.

## 10. Operator dashboard (Round 3)

### 10.1 Capability

- An operator is a verified account with a current `operator_grant`: scoped permissions
  (`SupportInspect`, `Refund`, `Deletion`, `AccountRestriction`, `OperatorEnrolment`), an expiry of
  at most 30 days, and a recorded ticket or reason. Payer, host or tenant owner never imply it.
- **Grants are issued in composition, never on the web.** `df-portal` has no database access, so
  the commands live in the server's composition binary: subcommands of the `df-tools` command line
  (`operator grant --account <id> --permission <p> --days <n> --ticket <text>`, `operator revoke`,
  `operator list`, `operator reenrol`, `operator reseal`), run on the server host by the platform
  owner. They build the `df-persistence` store and call `df_customer::OperatorService`. They are a
  W4 hunk (4.9); a `df-launcher` subcommand is the alternative if the owner prefers it there.
- **Enrolment confirms a first code.** If the account has no active TOTP, a grant creates a
  `pending` secret and mails a single-use, 15-minute enrolment link (the dev outbox in development).
  The operator opens it, enters the account password, sees the secret once (as text and an
  `otpauth://` URI), and enters a first code; only a valid first code makes it `active`. An
  unconfirmed pending secret expires with its link.
- **Grants and TOTP are independent.** A new or renewed grant keeps an existing active TOTP; revoking
  every grant removes all permissions but keeps the TOTP; the retention task revokes the TOTP of an
  account with no grant for 90 days.
- **Lost device.** Another operator with `OperatorEnrolment`, re-authenticating and giving a reason,
  revokes the affected operator's active secret (their operator sessions are revoked too) and issues
  a new pending enrolment link to the affected operator's own email; the owner can do the same with
  `operator reenrol`. No operator can re-enrol themselves through the web.
- **Seal-key rotation.** Config names the current seal key file and up to two previous ones by id.
  A secret sealed with a previous key is re-sealed with the current key at that operator's next
  successful login and by `operator reseal`; a previous key file can be removed once `operator list`
  shows no secret sealed with it. A secret whose key is missing cannot be unsealed: login fails
  closed and the operator must be re-enrolled.
- Operator login is a separate page and cookie: password plus a current, unused TOTP code.
- Every mutating operator action (refund, restrict, unrestrict, delete, webhook retry, re-enrolment,
  email reveal) requires a reason, recorded in `operator_audit`; refunds, restrictions, deletion and
  re-enrolment also require re-entering password and TOTP in the form. Restrict and unrestrict need
  `AccountRestriction`, not `SupportInspect`.

### 10.2 Pages

| Page | Shows | Never shows |
| --- | --- | --- |
| Dashboard | Customers, verified, deferred liability, top-ups and refunds (today, 30 days), disputes, restricted tenants, webhook backlog and failures, platform loss by reason, conservation status, pending closure refunds | Game content |
| Customers | Search by exact email or account id; list with masked email (`a***@e***.test`), status, standing, balance | Full email (unless revealed with a reason), hashes, tokens, TOTP secrets, IPs |
| Customer detail | Masked email (reveal with a reason when the action needs it, for example to answer a support request), display name, created, verified, session count and device labels, wallet, ledger, top-ups with refundable remainder, disputes, tables and pace | Game state, hero data, dialogue, private views |
| Top-ups, Refunds, Disputes | Cross-customer lists with status | Card data (never present) |
| Spend | Customer and supplier totals by campaign and by provider/model/category from `usage_debit` | Prompts or game text |
| Webhooks | Inbox rows by status, attempts, last error; retry one failed event | Raw bodies |
| Conservation | The checks in 8.4 per run, campaign and tenant | |
| Audit | `operator_audit` rows | |

### 10.3 Refunds

A refund targets one paid checkout, for at most its unrefunded and undisputed amount and at most the
available credit (credit held in an active prepaid reservation is not available). One
transaction locks the wallet, applies `HoldRefund`, inserts `refund_request`
(`held`) and the ledger entry; then, outside it, the gateway refund is created with the refund's
operation id as idempotency key (10 s deadline). Known success or failure applies `ObserveRefund`;
Unknown keeps the hold and reconciliation retrieves it by the same key. A double click is one
operation; refunds and usage debits racing each other serialize on the wallet lock and re-check the
available amount.

## 11. Business site (Round 4, with smoke checks earlier)

- Pages: home (an AI game master for a real table; the TV as the shared screen and phones as each
  player's hand; photographic scenes and voiced characters), how a table plays (join with a phone,
  speak or type, the host pays), pricing (prepaid credit, no subscription; the explanation of how
  credit is spent per tier comes from `content/pricing.json` and `content/spend-pace.json`; while the
  rate card is DRAFT the page shows no customer prices, only the explanation, decision 9), FAQ, terms
  (DRAFT banner), privacy (DRAFT banner).
- Visual direction: the project's photographic live-action fantasy look with existing public game
  art as full-bleed photographs under dark scrims; serif display headings and a readable sans body;
  one accent colour; CSS custom properties for every value. Pages scroll normally (the hidden-scroll
  rule in coding style 10 governs the TV stage and phone sheets).
- Every control has a visible text label; primary, secondary and destructive buttons look different
  and say what they do.
- Responsive to 375 px; visible keyboard focus; WCAG AA contrast; error summaries link to fields.
- Every round that ships pages (1, 2, 3, 4) ends with a real-browser smoke check of its pages at
  1440 x 900 and 375 x 812 with screenshots in `artifacts/tmp/agents/B/round-<n>/`, on a private port
  (never 63309), stopping only the process started for it. Round 4 is the full visual pass.

## 12. Threat model

| Threat | Controls | Tests |
| --- | --- | --- |
| Password guessing | Argon2id; length; common-password port; per-email-hash and per-IP throttles; identical failures | throttle after the limit and reset after the window (fake clock), for known and unknown addresses alike |
| Credential stuffing at scale | Per-IP ceiling; bounded hash concurrency returning Busy | Busy after the semaphore deadline |
| Session theft | `HttpOnly`, `Secure`, host-only cookie; token hashed at rest; idle and absolute expiry; sessions page; generation bump revokes all on password change or reset | expired, revoked and stale-generation sessions refused; no raw token in the database |
| Session fixation | Pre-login tokens never promoted; login issues new; privilege change rotates | pre-login token unusable after login |
| Takeover through reset | 32-byte single-use token, hashed, 15 min, 5 attempts; all sessions revoked; reset page posts; `no-referrer`; `?token=` never logged | reuse, expiry and attempt limit refused; other sessions dead; spans and audit lines contain no token |
| Takeover through email change | Current password; both addresses confirm; old address can cancel | one confirmation does nothing |
| Enumeration | Identical bodies, statuses and throttling; mail work on the worker for every request; equal hash cost on signup and login | byte-equal responses; no handler-side lookup on reset |
| CSRF and login CSRF | Synchronizer tokens on every POST; `Sec-Fetch-Site` and Origin checks; `SameSite=Lax`; no mutating GET | missing, wrong and cross-site posts refused |
| Clickjacking and injection | `frame-ancestors 'none'`; no script in CSP; escaping | hostile display names render escaped |
| Cross-tenant access | Tenant predicates from the session only; nonexistence-safe errors | paired fixtures (13.4) |
| Webhook forgery | Raw-body HMAC, constant time, 5 min tolerance; nothing stored on failure; credit only after an authoritative re-read | bad signature, stale timestamp, wrong secret |
| Webhook replay and duplicates | Inbox key on event id; ledger unique by operation and by source | duplicates credit once |
| Out-of-order delivery | Every event re-reads the object | expired-after-paid and paid-after-expired end paid once |
| Lost webhook or create | Reconciliation (8.6) | 24 h replay path and post-24 h webhook path |
| Fake gateway on a reachable server | Fake refused unless both the origin and the listen address are loopback | startup refused for a non-loopback origin, and for a `0.0.0.0` listen address with a loopback origin |
| Double credit under concurrency | Wallet lock plus expected revision; unique keys | two workers on one checkout credit once |
| Refund races | Hold before the gateway call; same key on retry; Unknown keeps the hold | refund versus debit; double click; unknown then reconciled |
| Out-of-portal refunds and disputes | `ExternalReversal`, dispute holds, Restricted standing | 8.5 cases |
| Spending beyond the balance | Durable reservation drawn at `BilledCall::new`; pause of all new paid work; true-up overrun absorbed and recorded; wallet never negative | draw refusal at an empty tranche; overrun recorded |
| Lost or altered cost records | Cursor identity and offset; halt on shrink, replace or bad line, which pauses the table and books accrual as platform loss; durable run totals; conservation check | 13.4 cases |
| Crash hiding spend | Durable reservation and per-minute `run_totals`; unclosed runs are a bounded conservation gap | 13.4 cases |
| Operator abuse | Scoped, expiring grants issued only by the server-side composition command; TOTP confirmed at enrolment; short sessions; reasons and re-authentication; separate restriction permission; masked email; audit row per request | 13.4 cases |
| Lost operator device | Re-enrolment only by another operator with `OperatorEnrolment` or by the owner on the server; old secret revoked; new link to the operator's own email | 13.4 cases |
| Secrets in logs | Token, password and key types without `Debug`/`Display`; ids and closed codes only in audit lines; no query strings in spans | assertions over captured audit lines and spans from every flow |
| Load on the game | Portal work never runs on the game owner thread; hashing on the blocking pool behind a semaphore; the game reads `watch` values only | login lines carry `ms=` so hash cost is visible |

## 13. Observability and tests

### 13.1 Audit lines (`df_observe::record_audit_line`; one per event; no private text)

```text
ACCOUNT <signup|verify|login|login_refused|logout|password_changed|reset_requested|reset_completed|email_change_requested|email_changed|sessions_revoked|export|closing|closed|deleted|throttled|csrf_refused> account=<id|none> reason=<closed code> ms=<n> request=<id>
EMAIL <queued|written|discarded|failed|queue_full> purpose=<closed> message=<n> ms=<n>
BILLING <checkout_created|checkout_unknown|webhook_received|webhook_refused|webhook_applied|webhook_ignored|webhook_failed|credited|debited|overrun|not_billed|cursor_halted|conservation_mismatch|reconciled|refund_held|refund_paid|refund_failed|refund_unknown|external_refund|dispute|restricted|reserved|refilled|released|draw_refused|run_not_closed|funds> tenant=<id> campaign=<id> checkout=<id> event=<id> amount_micros=<n> reason=<closed> state=<funded|low|exhausted> ms=<n>
PACE <set|governor_down|governor_up> campaign=<id> tier=<..> cadence=<..> voice=<on|off> hourly_micros=<n|none>
OPERATOR <login|login_refused|view|reveal_email|refund|restrict|unrestrict|delete|retry_webhook|enrolled|reenrolled|resealed|granted|revoked|refused> operator=<id> grant=<id> target=<id|none> outcome=<allowed|refused|failed>
```

`ip_hash` (keyed, truncated) appears only on `throttled` lines.

### 13.2 OTEL

Each request opens `df_observe::begin(context, "portal.<route template>")` with the request id; the
workers open spans per item (inbox event id, checkout id, call id); database and gateway calls record
latency and outcome class. No email, token, query string or body is an attribute.

### 13.3 Dev CLI reads

| Command | Shows |
| --- | --- |
| `dev accounts` | Counts by status and verification; the 20 most recent accounts with masked email, status, sessions, last event |
| `dev outbox [<n>]` | Outbox index; one message in full (dev only) |
| `dev billing` | Gateway, wallets, recent ledger entries, open/unknown checkouts, reservations and draw state per table, usage cursors (offset, identity, state, lag), run totals, funds state and pace per table, platform loss by reason, the conservation check |
| `dev billing tables` | Campaigns, payer grants, current pairing code |
| `dev webhooks` | Inbox rows by status |
| `dev operator-audit` | Last 50 operator audit rows |

The only dev mutation is `POST /dev/billing/fake-event` (fake gateway only).

### 13.4 Test plan per round

All tests are sentence-named, use `FakeClock`, injected randomness, the fake gateway and the dev
outbox; none sleeps, spends or touches the network. Router tests use `tower::ServiceExt::oneshot`.
The store contract tests run against the in-memory fakes and, as required tests, against the scratch
PostgreSQL the coordinator provides (`DUNGEONFLUX_ACCOUNTS_PG_PORT`, `_USER`, `_DATABASE`), one fresh
schema per test. A round is not reported done with those tests skipped.

**Paired cross-tenant fixtures (every round).** Two verified accounts A and B with their own
sessions, tokens, tables, checkouts, ledger rows and exports. Every route and store method that takes
an id is called by A with B's ids (session revoke, export download, checkout return, history cursor,
pace, unpair, refund target in the operator view's customer scope) and must answer exactly like a
missing id, with B's rows unchanged.

**Round 1.** Pure: password bounds (11 refused; 12 and 128 accepted; 129 refused), built-in common
password refused case-insensitively, password equal to email refused; session outcomes and touch
interval; throttle windows. Native: Argon2 round trip, wrong password, rehash on parameter change,
dummy verify for unknown email (fake hasher counts calls), Busy after the deadline; token digests
stored, raw tokens never; outbox bound and eviction; mail queue full rejects visibly. Router: exact
cookie attributes; CSRF missing/wrong/cross-site refused; pre-login token not promoted; signup, login
and reset responses byte-equal for known and unknown addresses; reset handler enqueues one job for
both; throttling keyed on the email hash behaves the same for unknown addresses; captured spans and
audit lines contain no `token=` value, email or password; CSP and `no-store` present; hostile display
name escaped; webhook-style large bodies are refused by the portal form limit. Store contract (fakes
and Postgres): signup, verify, login, logout; double submit returns the first outcome; changed fields
conflict; password change revokes others and rotates current; reset single use, 15 min expiry,
attempt limit, all sessions revoked; email change needs both confirmations; sign out everywhere;
export contents and exclusions; closure revokes sessions; deletion after the window clears personal
columns and keeps billing rows; free-text reason fields that target a deleted account are
redacted; receipts are retired to a namespace tombstone after 90 days and an old key then returns
`OperationExpired`. Browser smoke check of the Round 1 pages.

**Round 2.** Pure: top-up validation; `customer_amount` rounding toward the customer at a fractional
boundary (pinned); funds thresholds; pace steps and hysteresis; play-time arithmetic and one-cent
rounding; `ExternalReversal` with and without shortfall; standing transitions. Gateway fake: create,
Unknown create then same-key retry, retrieve, refund and dispute scripts; the created session
pins `payment_method_types = ["card"]`; startup refused with the fake for a non-loopback origin and
for a `0.0.0.0` listen address with a loopback origin. Webhooks: valid, bad signature, stale timestamp, rotated secret,
oversized body refused before buffering; duplicates credit once; two events for one checkout credit
once; out-of-order expired/paid; concurrent workers credit once; return page never credits.
Reconciliation: `creating` row resolved by same-key replay within 24 h; `unknown` resolved the same
way; after 24 h, resolved by a webhook carrying our reference, else expired. Refunds outside the
portal and disputes created, won and lost, each matched to its checkout through the stored
charge and payment intent references and producing the right ledger entries, holds, standing and
platform loss; an event for an unknown charge is ignored and listed. Reservation and draw (pure, with
a fake call loop): draw refuses when the tranche is smaller than the estimate; true-up up and down;
release on not-billed; drop keeps the estimate until reconciliation releases it; concurrent draws
never overdraw; a zero estimate is refused on a paid table and allowed with no draw; the ticket is
taken exactly once across settle, not-billed and drop; an unknown-outcome estimate is credited back
after the consumer records it; refill only after the durable commit; a refusal during a pending refill
is counted and alerted; the margin grows with the number of actions in flight and uses the measured
p95 after 20 actions; a refused call charges no unit counter or side record (fake call loop asserts
ordering); Stripe pairing refused while a fixture route table has a Pending route, fake pairing
allowed with the `unmetered_route` notice; `run_totals` keyed by session, and a restart in the same
run releases the earlier session's reservation; completion margin flips the table to Exhausted
before the draw refuses; local play with no draw never refuses; restart releases an earlier session's
unconsumed reservation after its records are debited.
Usage: a partial last line does not move the cursor and is debited once completed;
shrunk, replaced or undecodable files halt the cursor with an alert, pause the table, freeze the
draw and record accrual after the halt as platform loss; replay debits once; the same call id in two
runs or two campaigns debits separately and its ledger `source_ref` is the full triple; a run without
a clean close is reported as a bounded `run_not_closed` gap from `run_totals`; records before pairing not billed; unpriced
lines record platform loss; unknown-source not billed; a true-up beyond the reservation records
overrun; wallet never negative; the conservation check reports a planted dropped line. Pause: Exhausted and
Restricted both stop new optional paid requests through the funds watch (fake hub) while an admitted
action completes. End to end in process: sign up, verify, pair a table, top up through the fake
checkout, consume from a fixture cost log, see balance and history. Browser smoke check of the
funds, pace and tables pages and the fake checkout.

**Round 3.** Customer cookie refused on `/admin`; operator cookie refused on customer routes; payer
and host are not operators; expired or revoked grant refused; missing or reused TOTP refused; a
pending TOTP is not usable until a first code confirms it; an expired enrolment link refused;
re-enrolment by another operator revokes the old secret and that operator's sessions; self re-enrolment
through the web refused; a secret sealed with a previous key is re-sealed at login and one with a
missing key fails closed; restrict without `AccountRestriction` refused (even with `SupportInspect`);
deletion refused while a balance, reservation, refund or dispute is open, with the stated reason; a
mutating action without a reason refused; refunds, restrictions and deletion without
re-authentication refused; refund above refundable or available refused; double-click refund is one
refund; refund racing a usage debit leaves the wallet consistent; Unknown refund keeps the hold and
reconciles. Every operator request writes one audit row and one `OPERATOR` line. Rendered operator
pages contain no hash, token, TOTP secret, full email (before reveal) or game text. Browser smoke
check of the admin pages.

**Round 4.** Pricing page explanation equals values computed from the fixture tables and shows no
customer price while the rate card is DRAFT; drafts carry their banner; full browser pass at desktop
and phone widths: screenshots, no horizontal scroll at 375 px, focus order, visible focus, error
announcement and links, contrast.

## 14. Configuration and tunables

Entry-point settings, read once by composition and passed explicitly: `DF_PORTAL` (default off),
`DF_PORTAL_ORIGIN`, `DF_PORTAL_DATABASE_URL_FILE` (never logged), `DF_PORTAL_GATEWAY` (`fake`
default, loopback only; `stripe`), `DF_PORTAL_STRIPE_KEY_FILE`, `DF_PORTAL_STRIPE_WEBHOOK_SECRET_FILE`,
`DF_PORTAL_ADMIN` (default off), `DF_PORTAL_OPERATOR_SEAL_KEY_FILES` (current key id and file,
plus up to two previous ones), `DF_PORTAL_DEV_OUTBOX`.

Owner-tunable files, validated at load with per-knob defaults and an audit line for any fallback:
`content/portal.json` (session lifetimes, rate limits, top-up presets and range, low-funds threshold,
tranche size and completion margin),
`content/spend-pace.json` (tiers, cadences, reference usage per hour), `content/customer-rates.json`
(the DRAFT rate card).

## 15. Owner decisions and deviations

### 15.1 Recorded decisions

Decisions 9-16 in section 2 settle the round 0 questions; decisions 17-18 the round 0b review.

### 15.2 Deviations from the governing documents (for the owner)

1. **Separate service database; admission through a durable prepaid reservation.**
   `commerce-service.md` shares one PostgreSQL transaction between game and commerce authority at
   admission; auth D03 and commerce say a cached snapshot cannot authorize paid work; `AGENTS.md`
   requires current grants at durable admission and native egress. Accounts and billing stay in
   `df_service`, separate from the per-run game databases. The coordinator decided the admission
   design (decision 17): a durable reservation committed in `df_service` before any of it can be
   spent, drawn in memory at `BilledCall::new` and trued up at settle or drop. The in-memory draw can
   therefore never authorize more than was durably reserved; it is a fenced spending view of a
   durable grant, not a free-standing cached snapshot. What remains of the deviation, stated for the
   owner: the game's decision commit and the reservation are not one transaction, so (a) a true-up
   can exceed the reservation by the sum of in-flight estimate errors, and (b) a crash loses the
   record of calls drawn after the last written cost line; both are absorbed as platform loss, never
   charged, and are measured (overrun per exhaustion, `run_not_closed` gaps). The cost-log consumer is
   reconciliation and conservation, not the bound. The bound covers metered routes only; the nine
   Pending routes are outside it, so paid pairing waits for C to meter them (8.8).
2. **The protected recovery journal (paid-launch gate).** `service-operations.md` requires an
   append-only protected journal before credit is exposed after a restore. It does not exist; these
   rounds use PostgreSQL only. In particular, a restore of `df_service` from an older backup must
   replay deletion and revocation tombstones (deleted accounts, revoked sessions, operator secrets,
   retired operation namespaces) before serving data; without the journal there is nothing to replay
   them from, so restore remains a paid-launch gate.
3. **Platform spend limits are not implemented.** `commerce-service.md` proposes platform supplier
   liability limits of $100/day and $1,000/month and per-scope concurrency ceilings. They are spend
   caps, which the owner's standing rule forbids; this plan omits them. The only stop is the
   customer's own empty balance or a restricted account.
4. **Reservation by tranche, not per call in the database.** Commerce plans a durable reservation
   per paid effect; this plan reserves durably per tranche and draws per call in memory, so no I/O
   enters the call path and the cost engine stays the single meter (decision 17).
5. **Server-rendered portal and `df-customer` facade.** Replace the mounted `df-ui` panels and the
   `df-server` facade of `commerce-service.md` for the reasons in 3.2.
6. **The pause covers all new optional paid work** (9.2), not only player actions (decision 10).
7. **Reset lifetime is 15 min** per `service-operations.md` (decision 3).
8. **Card payments only at launch** (decision 18); delayed payment methods are later work.

## 16. Requests and prerequisites for the coordinator

- The scratch PostgreSQL for the required store tests (13.4), before Round 1 review.
- Workspace members `df-metering`, `df-customer`, `df-portal`.
- Agent C: the `cost.rs` hunks in 4.9: (a) shared record codec, fsync, `RunTotals`, totals and
  write-failure count; (b) the prepaid draw in `SpendLedger` and `BilledCall::new` returning
  `Result<BilledCall, FundsRefusal>`, with every call site handling the refusal as its existing
  provider-unavailable fallback.
- W4: the `operator` subcommands of the `df-tools` command line (10.1), Round 3.
- The other-agent hunks in 4.9: router composition and dev CLI after Round 1; funds, pace, pause and
  pairing in Round 2.
- Permission to run the server on a private port for the smoke checks in Rounds 1-4.
- Review of the `PaymentGateway`/`CommerceRepository` amendment (4.3) before Round 2 depends on it.

## 17. Delivery order

1. **Round 1: accounts and auth.** `df-types` role IDs; `df-auth` additions; `df-customer` (ports,
   clock, hashing, built-in common passwords, tokens, email worker and dev outbox, throttle,
   accounts, closure, export, retention, dev views, test fakes); `df-persistence` `account_store` and
   `0005_accounts.sql`; `df-portal` (security, html, layout, auth and account routes, CSS, stub public
   pages); `subsystem-architecture.md` map entries for the new crates and edges. Done when the Round 1
   tests pass including the Postgres contract tests, the gate passes, the in-process router test signs
   up, verifies through the outbox link, logs in and out, and the browser smoke check is recorded.
2. **Round 2: funds.** `df-metering` (record codec and `PrepaidDraw`); `df-commerce` `topup`,
   `customer_rate`, `funds`, `pace`, `standing`, the reservation and `ExternalReversal` credit events
   and the port amendment; `df-providers` fake and Stripe adapters;
   `df-persistence` `billing_store` and `0006_billing.sql`; `df-customer` billing, prepaid tranche
   and refill, usage reconciliation, pace, checkout reconciliation, disputes; `df-portal` funds, pace, tables, webhook router and fake checkout;
   requests for agent C's draw and record changes, the game's funds gate, pause sites, pace reads
   and pairing display. Must be done in Round 2 (carried from the third critic pass):
   - The `DrawTicket` sits in an `Option` in `BilledCall`, taken once by `settle`, `not_billed` or
     `Drop`.
   - Unknown-outcome draws stay drawn only until the consumer records them, then `credit_back`
     returns them mid-session; refills continue meanwhile.
   - The draw happens before any unit counter or side record is charged (4.9 d).
   - Estimates are in the route's priced categories, the storyteller's includes its output ceiling,
     and a zero estimate on a priced route is refused (4.9 c).
   - A storyteller refusal skips its retry loop; a refused admitted action gets the narration
     fallback in 9.2.
   - The completion margin rule in 8.4 (supplier micro-dollars; measured p95 per action or the tier
     reference; times actions in flight plus one).
   - Refusals during a pending refill are counted and alerted.
   - Run identity is (campaign, run, session) for reservations and totals.
   - Stripe pairing is refused while any route is Pending.
3. **Round 3: operator dashboard.** `0007_operators.sql`, `operator_store`, `df-customer` operators,
   enrolment, re-enrolment and seal rotation, `df-portal` admin routes, the conservation page; the
   `df-tools` operator subcommands requested from W4.
4. **Round 4: business site and visual design.** Public pages, pricing explanation, the visual pass
   over every page, full browser checks.

Each round writes `artifacts/tmp/agents/B/round-<n>.md` with files changed, behaviours and their
tests, the gate result line, test filters and counts, open questions and requests.
