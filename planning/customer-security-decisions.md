# Customer security and locale decisions

Date: 2026-10-25

Status: target design contract for coordinator and owner review; no product implementation or runtime approval.

These decisions fill C08 and the account/admin locale gaps in `artifacts/tmp/agents/FLOWMAP/flows.md`. They apply to customer accounts and the mounted account/admin application. Operator enrollment and operator MFA remain governed by ADMINFLOW and `planning/service-operations.md`; customer MFA does not satisfy or change operator MFA requirements.

## Customer multifactor authentication

1. Customer MFA is optional and is not a prerequisite for account creation, sign-in, campaign membership, play, or payment. The target factor contract is described below, but account-service research 7.6 currently marks WorkOS multi-factor unsupported. The security page must therefore show MFA as unavailable until the identity owner freezes and verifies an integrated factor authority and recovery path. It must not imply an enrollment or factor is active before the server confirms it. Once available and activated, sign-in requires the password and a valid TOTP; one unused recovery code may satisfy that second step once.
2. The initial customer factor is an authenticator-app TOTP factor. SMS, email codes, passkeys, and hardware-key enrollment are outside this decision. A customer starts enrollment after fresh password reauthentication, sees the enrollment secret once, and must confirm a valid TOTP before the factor becomes active. Pending enrollment expires and cannot authenticate.
3. Successful enrollment creates ten single-use recovery codes. The server stores only verifier material, displays the codes once, and refuses reuse. Regenerating codes requires fresh authentication and invalidates every prior code atomically. The UI warns the customer to save them privately; it never puts codes in a URL, browser storage, logs, telemetry, or support views.
4. Replacing or disabling an active factor requires fresh password authentication plus a valid current TOTP or unused recovery code. A successful change invalidates the prior factor and recovery codes, rotates the account session, and sends a security notice to the verified account email. No operator may silently bypass this customer flow.
5. The target lost-device path is the existing verified-email account recovery flow in `planning/account-service-research.md` 7.3 (single-use 15-minute challenge, credential-generation change and prior-session revocation), extended by the identity owner to invalidate the lost factor and recovery codes in the same durable outcome. Section 7.6 currently does not implement MFA or that factor reset. Until the identity owner approves and verifies this integrated recovery, customer MFA remains unavailable; no support override or claim that email recovery is phishing-resistant is allowed.
6. MFA challenges and all security-setting mutations are server-authoritative, same-origin and CSRF protected. Enrollment/change/recovery results use durable operation receipts; replay returns the recorded result and never creates another factor or code set. Audit and OpenTelemetry record outcome, account/operation correlation and reason codes only; factor secrets, codes, tokens and private form text are excluded.

The account page uses the following safe outcome meanings in every supported locale:

| Outcome | Customer meaning |
| --- | --- |
| `factor_unavailable` | This security method is temporarily unavailable; no factor was changed. |
| `enrollment_pending` | Setup is not complete; the factor cannot yet be used to sign in. |
| `invalid_or_expired_code` | The code was not accepted; setup or the requested change did not complete. |
| `recovery_code_used` | This recovery code has already been used; no further change occurred. |
| `reauthentication_required` | Sign in again to confirm this security change. |
| `operation_pending` | The result is still being confirmed; look up this same operation before retrying. |
| `security_change_complete` | The server confirmed the change and sent a notice to the verified email. |

Anonymous and foreign-account refusals remain enumeration-safe under `planning/account-service-research.md` 6.3. Copy is localized by stable keys; error text never includes submitted codes, tokens, credentials, or internal provider details.

## Account and admin language selection

The account and admin application supports English (`en`), Spanish (`es`), German (`de`), and French (`fr`) as one product surface. Availability means the route, labels, validation, status, accessibility names, and recovery/error outcomes are translated and reviewed in that locale. A partial catalog is reported as unavailable for that locale; it is not presented as complete.

For an authenticated account, an explicit language choice on the current request wins; otherwise use the account's persisted preference, then English. An explicit choice is validated by exact membership in the configured supported-locale set, saved to the account profile, and kept across routes and later sessions. For an unauthenticated visitor, an explicit language choice wins; otherwise use a server-readable, host-only locale preference cookie from a previous explicit choice, then English. On sign-in, an explicit pre-auth choice is applied to the account and persisted. `Accept-Language` and ambient browser/OS settings may be offered as a suggestion, but never silently change the selected language. Invalid or unsupported choices fall back to configured English and disclose that fallback accessibly.

Changing language changes presentation only. It preserves the route, current form values that are safe to retain, authentication state, operation key/receipt, and campaign/game state; it does not repeat a mutation. Locale tags use `df-types::LocaleTag` and the exact supported-set selection contract in `planning/c-df-locale-d02-decision.md`. Account locale storage is an account preference, not game/campaign locale state. Admin screens use the same preference and catalog policy. Rule/action identity, authorization, typed outcomes, and money values are unchanged by translation.

## Browser and assistive-technology target

The account application follows the target browser families in `planning/g04-d02-decision.md`; that decision selects families, not proven versions. Customer and operator account flows target:

| Surface | Target browser | Assistive technology interaction target |
| --- | --- | --- |
| Phone/tablet | Safari on iOS/iPadOS; Chrome on Android | VoiceOver with Safari; TalkBack with Chrome; touch and keyboard alternatives where available. |
| Laptop/desktop | Chrome, Edge, Firefox, Safari | NVDA with Firefox and Chrome on Windows; VoiceOver with Safari on macOS; keyboard-only completion in each desktop browser family. |

Specific browser/OS version floors remain an implementation and release gate; no compatibility claim follows from this target table. Before release, the owner publishes version floors and runs real-browser checks for account sign-in, language persistence, security settings, visible focus, labels, announced validation/status, zoom/reflow, and keyboard-only completion at the supported matrix. Manual screen-reader observation is required; automation or source review alone is not evidence of assistive-technology support. A failed or unavailable check remains explicit and cannot be recorded as a pass.

## Contract mapping and conflicts resolved

| Flow contract | Decision in this document | Required evidence before product approval |
| --- | --- | --- |
| C08 customer MFA | Optional target TOTP, confirmation before activation, ten one-use recovery codes, guarded replacement/disable, integrated email recovery after loss of both factor and codes; currently unavailable until the identity owner completes the factor/recovery contract. | Identity-owner contract; enrollment, replay, invalid/expired code, consumed recovery code, lost-device recovery, session rotation and audit tests with controlled time and durable receipts. |
| Shared languages | Four complete account/admin locales; explicit selection persists; exact-supported negotiation; English default; no silent device-language override. | Same route and meaning in all four locales; persisted choice across sign-in, refresh and later session; unsupported locale disclosed. |
| Shared accessibility | Browser/AT target matrix above; keyboard, focus, names and status announcements are required. | Real-browser and manual assistive-technology observations; publish tested browser/OS version floors. |
| Shared time/reauthentication | Fresh password proof for enrollment and recovery-code/factor mutations; factor proof or a recovery code for replacement/disable; pending challenges expire and stale/replayed receipts never mutate state. | Injected-clock boundaries, expiry, replay, reconnect and concurrent-tab tests. No wall-clock sleep. |
| Shared security | Server-side authorization, same-origin/CSRF, enumeration-safe refusals, safe headers, no credentials in URL/storage/logs, durable receipts, correlated shared audit/OTEL. | Unauthorized own/foreign/anonymous cases; origin/CSRF; headers and private-data scans; protected audit and receipt evidence. |

`planning/account-service-research.md` 7.6 currently treats multi-factor as an unsupported WorkOS step; the target customer factor cannot ship until the identity owner resolves that contract. `LAUNCH.md` decisions 28 and 34 control hosted identity and routes. Decision 28 replaces the older AuthKit hosted-page / authorization-code / direct-OIDC wording in decision 1 and older server-rendered account-screen prose: hosted identity uses WorkOS's server-side User Management API; our server issues the `__Host-` HttpOnly session and enforces CSRF and authorization. The client stores no identity token. Decision 34 assigns the listed `/app/*` routes and requires fresh server-checked MFA on every operator route. Decision 34's operator requirement is not a customer MFA mandate. `planning/campaigns-and-table-research.md` D28 concerns the distinct `Sec-Fetch-Site` compatibility rule for display-ticket redemption/renewal; it does not weaken account CSRF or origin checks. D34's game gRPC-over-WebSocket tunnel is also separate from operator SSH/mTLS access and from customer identity.

## Remaining owner gates

These decisions do not approve consumer terms, privacy/retention periods, data-subject handling, processor or identity-provider settings, transfer of authenticator data, legal interpretations, security certification, or paid launch. The owners of privacy, legal, security, identity-provider, and commercial policy must approve those matters before applicable release. No test result, planning decision, or translation creates a new compliance promise. No product implementation, prototype, runtime approval, or customer availability claim is authorized by this document.
