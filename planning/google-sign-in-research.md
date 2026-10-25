# Google sign-up and sign-in research

Date: 2026-10-25  
Status: design for the next implementation lane; no code or provider configuration changed.

## 1. Governing decisions and current boundary

This proposal follows:

- `artifacts/tmp/agents/LAUNCH.md`, decisions 1, 26, 27, 28 and 32: account screens are the Rust WASM `df-account-app` mounted at `/app/`; WorkOS User Management is server-side; F owns money; account views show only customer prices; production and staging each have one configured origin.
- `planning/account-service-research.md`, especially §§2.1, 4.4, 5, 7.2, 7.5 and 7.6: WorkOS stores passwords, our service owns account rows, email verification, sessions, throttles, mails and closure; session cookie is `__Host-`, verification replaces the password and revokes prior sessions, WorkOS email verification is disabled, and closure needs re-authentication within five minutes.
- `artifacts/tmp/agents/B/round-2.md` and `round-2b.md`: WorkOS adapter, provider links, identity lookup, bounded throttles, closure sequencing and the explicit rulings summarized above.
- `planning/coding-style.md` §§6-9, 11-12: typed contracts and errors, bounded provider calls and queues, shared audit path, fake providers, and sentence-named tests.
- `planning/client-architecture.md` and launch decision 28: WASM app is a thin client; all identity decisions and session issuance remain on the server.

Current code has password signup/signin routes and `IdentityProvider` for WorkOS passwords. It has a WorkOS user id (`ProviderUser.subject`) and account-to-provider link. There is no Google OAuth flow or account-app OAuth callback screen yet. `df-account-app` already has age and terms fields on password signup; their persistence/version contract was previously noted as unfinished. Treat acceptance recording as required shared signup work, not a Google-only visual checkbox.

## 2. Decision summary

1. Add one provider flow: Google OAuth through WorkOS User Management. The browser visits WorkOS only for the provider authorization redirect; API keys, code exchange, identity matching and our session issuance stay server-side.
2. Start with a CSRF-protected `POST /app/api/oauth/google/start`, not a state-changing `GET`. The account-app button submits a typed intent: `sign_up`, `sign_in` or `link`. It cannot submit a return URL.
3. Persist a random single-use OAuth attempt and PKCE verifier server-side for ten minutes. Bind it to a random short-lived `__Host-df_oauth` cookie with `Secure; HttpOnly; SameSite=Lax; Path=/`; no `Domain`. Store hashes for state and cookie binding; the verifier is temporary server-side data and is deleted on use/expiry.
4. Use fixed callback URLs: production `https://dungeonfluxdnd.com/app/api/oauth/google/callback`; staging `https://staging.dungeonfluxdnd.com/app/api/oauth/google/callback`. Local preview uses only the fake adapter and its local callback; it must not contact WorkOS.
5. Never silently attach Google to an existing account. For an email match, require the account holder to sign in with that account's password once, then restart OAuth in `link` intent while authenticated and recently re-authenticated. Google `email_verified` and our account's verified flag are both required to complete a link.
6. New Google signups collect the 18+ attestation and current terms/privacy acceptance on `/app/sign-up` before OAuth begins. On callback, create a pending account and send our own verification email. The account is not usable until that link is completed. Google `email_verified` is a matching/linking condition, not a replacement for our established first-party verification contract.
7. Do not request, store or log Google access/refresh tokens. WorkOS user id is the provider subject stored against the account; we need no Google API scopes beyond identity.
8. Account closure keeps the existing five-minute recent-proof rule and retention order: after full campaign completion, delete the WorkOS user and identity link, then purge our account data. Sign-out revokes our session only; there is no Google token or Google browser session to revoke.

## 3. End-to-end flow

### 3.1 Sign-up entry

- `/app/sign-up` shows email/password signup as today and a visually distinct **Continue with Google** button.
- Before that button can submit, require the existing `I am 18 or older` attestation and acceptance of the current terms and privacy notice. The server persists the accepted terms version and timestamp plus adult-attestation timestamp with the new account. A client-only checkbox is not evidence.
- The button issues a `POST /app/api/oauth/google/start` with `intent=sign_up`, current CSRF proof, and consent fields. It does not send an arbitrary `next`, `return_to`, origin or redirect URI.
- The server validates the configured origin, CSRF and consent; enforces signup/IP throttles; creates the OAuth attempt; and returns a 303 to the WorkOS authorization URL.
- If consent is missing, stay on `/app/sign-up`, identify the required checkbox, and do not create an attempt or call WorkOS.

### 3.2 Sign-in entry

- `/app/sign-in` has the same **Continue with Google** button, with `intent=sign_in` and the standard pre-session CSRF proof.
- The server applies the sign-in throttle before the provider call. WorkOS then redirects back to our fixed callback.
- A successful identity resolves to an existing account link and issues our normal session cookie; it does not keep a WorkOS session.
- If the WorkOS user has no account link, do not create an account from a sign-in intent. Show a generic screen with links to `/app/sign-up` and `/app/sign-in`; no address enumeration.

### 3.3 Start endpoint

- Exact route: `POST /app/api/oauth/google/start` on the account API router in `df-portal::app_api`.
- Accept only a closed enum `OAuthIntent::{SignUp, SignIn, Link}` and the expected CSRF form token. For `Link`, require a live customer session plus a fresh password re-authentication proof (existing 10-minute general window is sufficient; closure remains 5 minutes).
- Redirect URI comes only from `HostConfig` environment configuration and an exact allowlist. Reject host/proxy-header ambiguity under the existing trusted-proxy and origin policy.
- Generate independent cryptographically random `state`, PKCE verifier, and cookie binding values. Use SHA-256 base64url-no-padding for the S256 challenge. Store attempt digest(s), verifier, intent, optional account id and recorded consent, created/expiry time, and an `unused` status in PostgreSQL. No request parameters control the return route.
- Set `__Host-df_oauth=<opaque binding>; Secure; HttpOnly; SameSite=Lax; Path=/; Max-Age=600`. Cookie is host-only; clear it on every callback outcome.
- Redirect to `GET https://api.workos.com/user_management/authorize` with `provider=GoogleOAuth`, `client_id`, exact `redirect_uri`, `state`, `code_challenge`, and `code_challenge_method=S256` where supported. Do not request `oauth_tokens` or unrelated scopes.

### 3.4 Callback and code exchange

- Exact route: `GET /app/api/oauth/google/callback`. Accept only `code` and `state` (plus a provider cancellation/error value); ignore or reject unknown return destinations.
- Require the `__Host-df_oauth` cookie. Find the matching, unexpired attempt by state digest and cookie-binding digest; atomically claim it once before code exchange. A duplicate callback, mismatched state/cookie, expired attempt or already-claimed row fails closed. Delete/expire attempt secrets after completion; retention cleanup deletes expired attempts.
- Exchange server-to-server with `POST https://api.workos.com/user_management/authenticate`, `grant_type=authorization_code`, `client_id`, fixed `redirect_uri`, `code`, and the stored `code_verifier` when WorkOS supports PKCE for this endpoint. Do not expose the WorkOS API key or exchange response to the browser.
- Validate response shape strictly: WorkOS user id, syntactically valid email, and boolean `email_verified`. Treat malformed or missing values as an unavailable identity result. Ignore all OAuth token fields; do not request them.
- WorkOS maps the Google identity to its user. Our stable lookup key is the WorkOS user id, not Google email alone. Email is used only for matching and conflict resolution.

### 3.5 Identity resolution

Resolve under a transaction with uniqueness/race handling on account email and provider subject:

1. **Existing link by WorkOS subject:** account must be active, our email verification must be complete, and the returned email must match the account's normalized address. If not, refuse and audit a sanitized reason. Issue our normal session on success.
2. **No subject link; sign-in intent:** do not create or link an account. Show generic recovery/sign-up guidance.
3. **No subject link; link intent:** require the account id from the stored attempt, that account's recent password re-auth proof, `Google email_verified=true`, and our `email_verified_at` set. Require normalized email equality. Link only if the account has no WorkOS identity link and the WorkOS subject is not already attached elsewhere. Otherwise refuse without relinking or merging.
4. **No subject link; sign-up intent; no account at the normalized email:** require `Google email_verified=true`; create an account and WorkOS identity link in one transaction with status pending our verification. Persist consent evidence. Queue our verification message and show the pending-verification screen. Do not issue an authenticated session yet.
5. **Sign-up intent; address already belongs to an account:** never attach automatically. If existing account is unverified, direct the user to our email verification and password replacement flow first. If it is verified, require a password sign-in once, then restart OAuth from Settings to link. A confirmation-email-only link is disallowed for this launch because it would bypass the verify-and-set-password pre-hijack rule.
6. **Google reports `email_verified=false`:** do not create, sign in, or link. Show a fixed error explaining that Google could not confirm the email and offer password sign-in or retry.

The pre-hijack defense remains intact: our emailed verification link for password signups asks the user to set a password, updates the WorkOS password first, then consumes verification, bumps credential generation and revokes prior sessions. OAuth does not reuse that link to replace a password. For an existing password account, successful password sign-in is the ownership proof before adding Google.

### 3.6 Consent and account usability

- Ask for age and terms on `/app/sign-up` before starting Google; after returning, do not ask again unless the consent version changed while OAuth was in flight.
- Persist a versioned terms/privacy acceptance record and adult attestation with the account creation transaction. Terms version and retention shape are an owner decision if the existing account contract does not yet define them.
- Google email verification does not bypass our first-party verification email. This preserves current account-service policy and keeps account usability rules consistent across identity methods.
- Do not issue a session until our verification is complete. `/app/verify-email` uses the existing first-party email link flow. At first verification, ask the person to set a password as required by the existing pre-hijack rule. Set it at WorkOS before consuming our verification link, bump credential generation, and revoke all pre-verification sessions. A refusal leaves the link unspent.
- The user sets a WorkOS password during first-party verification, so existing password reset and re-authentication paths remain available. Never set that password before the email link is proven.

### 3.7 Session and re-authentication

- Successful Google sign-in issues a first-party opaque random session token and CSRF secret exactly as password login does. Store only token hash; set `__Host-df_session` with `Secure; HttpOnly; SameSite=Lax; Path=/`, no `Domain`; rotate/clear the pre-session cookie as normal.
- Every authenticated API call authorizes from our session and account state. A WorkOS/Google identity response is not itself a session or application permission.
- For a sensitive action needing fresh proof, start a new Google authorization with `prompt=login` and, where supported, `max_age=0`; require the returned identity subject to equal the already linked subject and `auth_time`/provider proof to satisfy the configured freshness check. Do not treat query parameters alone as proof. Verify in WorkOS sandbox whether these hints are forwarded and whether an authentication-time claim is available.
- Closure specifically keeps `CLOSURE_REAUTHENTICATION_MS` (five minutes). If WorkOS cannot provide a verifiable fresh-auth signal, require current password for password-linked accounts or defer Google-only closure until a supported re-auth method is defined; do not silently extend the proof window.

### 3.8 Sign-out and closure

- Sign-out revokes the current DungeonFlux web session and clears its cookie. There is no retained provider session or token to revoke. Google browser SSO may still be active at Google; the next OAuth may reuse it unless forced re-auth is requested.
- Sign out everywhere revokes every DungeonFlux session through the existing generation/session mechanism.
- Closure follows current account closure behavior and five-minute proof. At retention completion, only after `complete_closure` returns `Completed`, delete the WorkOS user, remove its identity link, then purge personal account data. If WorkOS deletion fails or is unknown, keep data and retry/audit per current retention policy; never purge first.

## 4. Security requirements

### 4.1 CSRF, state, and PKCE

- Start is a same-origin POST with current pre-session/session CSRF token, origin check, and `Sec-Fetch-Site` policy. Callback state is independently random and single-use; state does not replace start CSRF protection.
- Verify callback state and the host-only cookie binding in constant time where applicable. Bind attempt to exact environment, redirect URI and intent server-side.
- Use PKCE S256. Never accept a caller-provided challenge, verifier, client id, redirect URI or provider URL.
- Ten-minute expiry is checked at claim and code exchange. Rate limit attempts and callbacks by IP and keyed email/account where known. Bound active attempts per IP/session and delete expired rows in bounded batches.
- OAuth attempt row transition is atomic (`pending -> claimed`); one valid callback can win. Provider timeout consumes the attempt; user restarts, preventing replay.

### 4.2 Redirect and response safety

- All return paths are enum-mapped: sign-up pending -> `/app/verify-email`; sign-in -> `/app/dashboard`; link -> `/app/settings`. No arbitrary URL, `next`, query parameter or forwarded host may become a redirect.
- Callback always redirects to this origin and a fixed path. Provider `error_description` and raw error bodies are never rendered. No code, state, email, verifier or tokens in URLs after callback, logs, audit lines or referrers.
- Callback and error responses set `Cache-Control: no-store`, a strict referrer policy and existing security headers. Remove authorization query parameters with the redirect.
- OAuth error messages are generic and actionable, without revealing whether an email/account exists.

### 4.3 Throttles, audit, and observability

- Start throttle: existing signup/sign-in buckets plus an OAuth-start per-IP ceiling and a bound on pending attempts. Callback code exchange has a provider deadline and its own per-attempt single-use guard. Do not introduce an unbounded in-memory state map.
- Count failed/unverified provider responses using existing sign-in throttle semantics. The third concurrent sign-in check for one address remains refused as established by round 2b.
- Emit one shared-path correlated audit line per public transition, for example `ACCOUNT oauth_started intent=sign_in`, `ACCOUNT oauth_callback outcome=linked`, `ACCOUNT oauth_callback outcome=state_refused`, `ACCOUNT oauth_link_refused reason=recent_auth_required`, and `ACCOUNT oauth_provider_unavailable`. Use keyed account/IP labels; never log email, code, state, cookie, verifier, API key, response body or OAuth tokens.
- Preserve tracing/correlation from start through callback using an opaque attempt id; async provider calls have deadlines and remain owned by request handling.

### 4.4 Secrets and stored data

Persist only what is needed:

- `accounts.oauth_attempt`: opaque attempt id, state digest, cookie-binding digest, temporary PKCE verifier, intent, optional target account id, consent evidence, exact redirect URI key/environment, creation/expiry, claimed/completed state.
- Existing account email and WorkOS identity subject/link, plus first-party email verification and consent fields/records.
- Never persist Google profile photo, Google access/refresh/id tokens, WorkOS access/refresh tokens, auth code, raw state or raw cookie binding. Do not ask for Google Calendar/Drive or other scopes.

The temporary PKCE verifier is a credential-like secret: exclude from `Debug`, SQL diagnostics and logs; delete on claim/failure/expiry. Sandbox verification must confirm the returned `user.email_verified` is the appropriate Google-backed claim for the WorkOS Google connection.

## 5. Error screens and recovery

Use fixed account-app states, not raw provider errors:

- **Canceled:** “Google sign-in was canceled.” Buttons retry and return to sign-in/sign-up.
- **Expired/replayed attempt or state mismatch:** “This sign-in link expired or was already used. Start again.” Clear cookie; no account change.
- **Google email not verified:** explain that Google could not verify the address; retry after verifying with Google or use email/password.
- **Existing account needs password proof:** generic message directing the user to sign in with the existing email/password first; after sign-in go to Settings → Security → Connect Google.
- **No account for Google sign-in:** direct to sign-up; do not disclose an email match.
- **Terms/adult consent missing or changed:** return to sign-up and request current acceptance before restarting OAuth.
- **Provider unavailable/malformed response:** “Google sign-in is temporarily unavailable.” Keep current session/account state unchanged; retry later.
- **Account closing/closed/restricted:** use the existing account status screen; do not issue session or re-open an account past its reopen window.
- **Link conflict/already linked elsewhere:** refuse link; offer sign-in with the already-linked method and support route. Never detach or overwrite automatically.

Every error links only to fixed `/app/sign-in`, `/app/sign-up` or `/app/settings` routes. Do not include an email in the message or return URL.

## 6. WorkOS calls and sandbox gates

Expected authorization request:

`GET https://api.workos.com/user_management/authorize?provider=GoogleOAuth&client_id=...&redirect_uri=...&state=...&code_challenge=...&code_challenge_method=S256`

Expected server exchange:

`POST /user_management/authenticate` with `grant_type=authorization_code`, `client_id`, `code`, exact `redirect_uri`, and `code_verifier` if PKCE is supported. Use server credentials only in the back-end adapter. Response must include a WorkOS user and email verification flag; do not request OAuth tokens.

Mark **verify in WorkOS sandbox** before production:

1. Whether `code_challenge`/`code_challenge_method=S256` are accepted on the User Management authorize endpoint and `code_verifier` is accepted by authorization-code authenticate.
2. Exact authorization-code exchange fields and response shape for GoogleOAuth, including user id, email casing/normalization and `email_verified` source.
3. Whether `prompt=login` and `max_age=0` are forwarded to Google and whether a trustworthy fresh-authentication timestamp is returned.
4. Whether `provider=GoogleOAuth` is the exact configured provider identifier in the current WorkOS tenant and how Google consent errors are represented.
5. Confirm no WorkOS-created session persists when the OAuth code grant is used, or identify the supported way to terminate it immediately. Do not retain any returned tokens.
6. Confirm redirect URI allowlisting behavior for production and staging, and that WorkOS email verification remains disabled for password users.

Do not make live calls in this design lane. The build lane uses faithful fakes/local fixtures until the owner runs these scoped sandbox checks.

## 7. Fake and acceptance plan

### 7.1 `FakeOAuth`

Add `FakeOAuth` under `crates/df-workos/src/fake.rs` (or a focused private child module) with in-memory authorize, code-exchange, user, callback and one-use code behavior. It should implement the same OAuth port contract as WorkOS, issue deterministic test URLs/codes, accept configured verified/unverified identities, and record safe call counts. It must not make network calls. Wire it into `df-portal` example `account_app_preview` under `test-fakes`, with a clearly local callback and fixed test users; preview must complete signup, first-party verification, sign-in, link and sign-out without Google or WorkOS network access.

The fake must simulate expired/replayed state, wrong cookie binding, unverified email, same-email existing account, account already linked to another subject, provider refusal/unavailability, and a code exchange with an invalid PKCE verifier. It must not log raw secrets.

### 7.2 Unit tests

- Start rejects missing/wrong CSRF, invalid intent, missing consent, untrusted origin and caller-supplied return URI; start creates bounded 10-minute attempt and exact redirect.
- Callback rejects missing/mismatched cookie/state, expired state, replay, wrong PKCE verifier, malformed WorkOS user and provider cancellation; attempt is single-use even under concurrent callbacks.
- New verified Google email creates one pending account, stores versioned consent, sends our verification mail, and issues no session; unverified Google email creates nothing.
- Sign-in linked subject gets the same session outcome/cookie attributes as password sign-in; unknown subject under sign-in creates no account.
- Existing email match never silently links. Verified match requires password login then fresh link intent; unverified match follows our verify-and-set-password flow and generation/session revocation first.
- Refuse normalized email mismatch, duplicate subject, already-linked account and account-closing/closed outcomes without replacing existing links.
- Session/logout/close behavior shares current session policy; closing requires proof <=5 minutes and provider deletion occurs only after closure completion.
- No tokens/code/state/verifier appear in Debug or audit output. All provider calls obey deadline and failure mapping.

### 7.3 PostgreSQL tests

Use an owned scratch database and the service role. Verify `oauth_attempt` uniqueness and atomic single-use claim under two connections; expiry and purge; consent persistence on account creation; unique account-email/WorkOS-subject races; rollback leaves no partial account or link; runtime role can use required statements only; closure deletes provider link only after `Completed`. Run migrations through the next free global migration number and test upgrade from the current schema.

### 7.4 Browser tests

Run a real browser against the local `account_app_preview` fake. Verify both auth screens show usable Google controls; signup consent is required; redirect/callback works; same-phase screen update renders once; error views are accessible; refresh/reconnect handles a consumed attempt; session cookie is HttpOnly/Secure/SameSite=Lax and host-only; no token enters JS storage, URL history or console; sign-out clears local session; fake provider failure and asset failure show recoverable states. Exercise mobile width and keyboard navigation. No live provider call or real account.

## 8. Next build lane: bounded implementation plan

### `df-workos`

- `crates/df-workos/src/adapter.rs`: add WorkOS OAuth authorize URL construction and authorization-code exchange through the existing bounded HTTPS transport; exact allowlisted callback; strict response parser; token fields discarded.
- `crates/df-workos/src/fake.rs`: add `FakeOAuth` contract and one-use fake codes/users; keep existing `FakeWorkOsServer` password endpoint behavior.
- `crates/df-workos/src/lib.rs` and focused tests: expose only the adapter seam needed by composition; test request fields, escaping, no tokens, malformed answers and failure mapping.

### `df-customer`

- `crates/df-customer/src/provider.rs`: add typed OAuth authorize/exchange results and provider methods or a narrow OAuth sub-port. Keep WorkOS user subject as stable link key; redact temporary credentials.
- `crates/df-customer/src/provider_flows.rs`: implement start/callback resolution, consent, email match/link rules, Google sign-in session issuance and audit events. Add OAuth attempt and account-consent store contracts in `ports.rs`/`records.rs`; enforce bounds and deadlines.
- `crates/df-customer/src/service.rs` and `throttle.rs`: wire methods into current service, reuse shared session and throttle paths; add per-IP active-attempt bounds if current throttle contract lacks them.
- `crates/df-customer/src/retention.rs`: bounded expired-attempt cleanup; retain current close order and delete WorkOS user only after completion outcome.
- `crates/df-persistence/src/account_store/` (coordinate with account-store owner): implement PG attempt claim/store, consent write, unique link checks and cleanup. `crates/df-persistence/migrations/0025_google_oauth.sql` is the next free migration number in the current workspace. It should add bounded OAuth-attempt persistence and the minimal versioned consent record/columns only; no Google token table.

### `df-portal` API

- `crates/df-portal/src/app_api/`: add POST start and GET callback handlers, typed input/output, fixed redirect mapping, cookie set/clear, response headers, CSRF/origin checks and error mapping. Update router composition and focused route tests. Do not create a new browser API to WorkOS.
- `crates/df-portal/examples/account_app_preview.rs` and test fixtures: inject `FakeOAuth`, test callback host/path and deterministic local identities. No network dependency.

### `df-account-app`

- `crates/df-account-app/src/route.rs`: add a fixed OAuth result route/state enum if needed; never parse or preserve provider tokens or callback query values.
- `crates/df-account-app/src/api.rs` and `wire.rs`: represent fixed start intent and result screens only; use same-origin POST and ordinary server redirect.
- `crates/df-account-app/src/view/auth.rs` and `state.rs`: add “Continue with Google” to `/app/sign-up` and `/app/sign-in`; require and carry age/terms only for sign-up; show accessible fixed error/retry screens.
- Keep all OAuth redirects, decisions and session data in `df-portal`/`df-customer`; the WASM app remains a thin presentation client.

## 9. Owner setup before launch

1. Create/configure a Google Cloud OAuth client and consent screen for DungeonFlux. Use minimum identity scopes only; add public privacy-policy disclosure that Google sign-in shares authentication/identity data with Google and WorkOS. Verify any required verification/branding steps with the owner.
2. Enable Google as a connection in the WorkOS User Management dashboard; confirm `GoogleOAuth` provider id, environment/client credentials and that WorkOS email verification remains off for password-managed users.
3. Register exact callback URI in Google Cloud and WorkOS: production `https://dungeonfluxdnd.com/app/api/oauth/google/callback`; staging `https://staging.dungeonfluxdnd.com/app/api/oauth/google/callback`. Keep staging and production credentials and cookies isolated.
4. Configure per-environment WorkOS client id/API secret in server secret storage. Never put secrets in WASM bundle, static assets, repo, logs or test fixtures.
5. Run the six sandbox gates in §6; record observed response shape and provider fresh-auth capabilities before enabling in production.

## 10. Explicit open items

- WorkOS PKCE support, authorization-code payload/response, provider identifier, auth freshness hints, and provider-session termination must be verified in WorkOS sandbox.
- Terms version identifier and adult-attestation retention need a shared owner contract; B's round 2 report marked persistent terms/adult fields as unfinished. Do not claim signup compliance from client state alone.
- The verification-and-set-password UI must support newly created OAuth accounts without weakening the existing verify-first, revoke-prior-sessions rule.
- A fresh Google proof for closure is acceptable only if WorkOS returns verifiable freshness. If unavailable, choose a supported re-auth path before launch; keep the five-minute closure rule.
- Migration number 0025 is free in the currently present global migration files. Recheck immediately before implementation because another lane may allocate it first.


## Direct Google OIDC (owner decision 2026-10-25)

This addendum supersedes every WorkOS-Google step and the Google-only pending-verification
steps above. Password identities remain at WorkOS with our verify-first password replacement.
Google uses a separate server-side df-google adapter and accounts.google_identity link keyed
only by Google's sub. A verified new Google mailbox creates an immediately verified account,
records the same adult attestation/current terms acceptance as password signup, and lands signed
in on the dashboard with the $10 trial notice. An email collision never links: sign in with the
existing password first, then explicitly link Google in Settings.

Authorization is GET https://accounts.google.com/o/oauth2/v2/auth, with response_type=code,
client_id, configured public origin plus /app/api/oauth/google/callback, scope=openid email
profile, independent random state and nonce, PKCE S256 and prompt=select_account. Starts remain
same-origin CSRF-protected POSTs. Fresh linking/closure re-authentication also sends max_age=0;
returned auth_time must be no older than five minutes. An ordinary Google session alone does not
provide fresh proof for closure. Attempts expire after ten minutes and are atomically consumed
before exchange, including cancellation/failure; cookie binding is Secure, HttpOnly, SameSite=Lax,
host-only, Path=/ and Max-Age=600. No arbitrary return destination is accepted.

Exchange is server-only POST https://oauth2.googleapis.com/token with grant_type=
authorization_code, code, exact redirect_uri, client_id, client_secret and code_verifier. Calls have
connect/read/total deadlines, no redirects or retries and a bounded response. Per OpenID Connect
Core 3.1.3.7, a token received directly from the token endpoint over authenticated TLS can use TLS
validation in place of the token signature check. This exception is restricted to that direct
response, never browser-supplied tokens. We still require Google issuer (https://accounts.google.com
or accounts.google.com), our exact audience and azp when present, future exp, sane iat, matching
nonce and email_verified=true. Fresh re-authentication additionally checks auth_time. Google access,
refresh and ID tokens are discarded immediately after use. We retain only sub and the email shown
on the account, never profile photos or Google API tokens. The requested profile scope can deliver
a name; it is not persisted.

Configuration names are DF_GOOGLE_CLIENT_ID and DF_GOOGLE_CLIENT_SECRET. Google is enabled only
when both are configured; the account app hides its button otherwise. Production/staging callback
origins remain distinct and come from configured HostConfig, never request or proxy headers.

Sections 3 (adapted flow), 4 (security), 5 (fixed recovery screens) and 7 (local fakes, PostgreSQL and
browser verification) still govern except for the overrides above. New verified Google signup
sends no first-party verification mail and requires no password. Unknown Google sign-in does not
create an account. Unlink refuses the final sign-in method. Google sign-out revokes our session;
closure removes our Google link only after campaign completion and never deletes a Google account.
WorkOS deletion remains exclusively for password-provider links.

The session store retains a first-party reauthentication decision expiry, bounded by both the
validated authentication time plus five minutes and our completion time plus five minutes.
It does not retain Google's auth_time claim. An ordinary Google session has explicit Google
provenance with no fresh proof; disabling the provider configuration must preserve that provenance.

Migration 0029_google_oauth.sql was allocated after checking all crate migration directories;
0025-0028 are taken/reserved (0100 is separately owned by Stripe). FakeGoogle covers preview;
a loopback token endpoint fixture covers transport. No Google call is authorized in this lane.
The coordinator real-account checklist is artifacts/tmp/agents/GOOGLEAUTH/live-check.md.
The same disclosure sentence needs adding to docs/es/legal/privacy/index.html,
docs/de/legal/privacy/index.html and docs/fr/legal/privacy/index.html in a later translation lane.
