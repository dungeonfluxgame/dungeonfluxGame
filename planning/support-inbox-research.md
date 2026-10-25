# Support inbox, triage and operator action queue

Date: 2026-10-25. Lane: SUPPORTQ. Status: design and plan only; nothing provisioned or sent.
Source: `3f990907273c80f893ec01d113255b13457ab5cb` plus today's shared dirty tree.
Owner request: ingest `support@dungeonfluxdnd.com` (new Google Workspace Gmail mailbox),
read and work the queue in the admin console, use Jev to classify and LLMs to draft actions.
Assumption: "date actions" means draft actions; this design also tracks due dates explicitly.

## 1. Scope, governing contracts and current evidence

Deliver one operator inbox: receive -> classify -> investigate -> propose -> approve -> execute
through the owning service -> reconcile -> reply/resolve. Models never approve or execute.
V1 has one mailbox, threaded tickets, assignments, deadlines, private attachments and replies.
No customer-mailbox integration, autonomous agent, public ticket portal or general workflow builder.

| Governing source | Sections / decisions applied |
|---|---|
| [Coding style](coding-style.md), [Client architecture](client-architecture.md) | 2, 6-8, 10-12; thin Rust wasm, typed decisions, one meter/audit path, bounded owned work |
| [LAUNCH decisions](../artifacts/tmp/agents/LAUNCH.md) | 21: metered Jev; 26: no supplier prices in replies; 27: F/B ownership; 28, 32-34: P app, one origin, operator MFA, host secrets/PKI |
| [P2 brief](../artifacts/tmp/lanes/P2/brief.md) | Existing account detail tabs, scoped admin controls, opaque paging, 375 px, 700 KiB wasm budget |
| [Finance ledger](finance-ledger-research.md) | 14.1-14.2, 18, 19-22: refund preview/outbox, disputes, idempotency, locks, restore |
| [Account service](account-service-research.md) | 7.3, 7.5, 10, 13: verified identity, recovery, export/closure, operators and audit; newer D28/D34 override separate-login prose |
| [Privacy review](privacy-compliance-review.md) | 4-7, 12 PRIV-04/05/08/10: processors, rights intake, retention, deletion, breach routing |
| [Legal review](legal-risk-review.md) | 5-7, 9: sensitive content, money, privacy, notice/takedown/appeal; owner/counsel decide legal merits |
| [Integration API](integration-api-and-embeds-research.md) | 2.1-2.3: separate scoped operator credentials, every-use audit; v1 tokens are read-only |
| [Master launch plan](master-launch-plan.md) | 7.3: case-linked financial actions; 11: notifications, unknown results, accessibility; 16: telemetry and runbooks |
| [Storage](storage-architecture.md), [Observability](observability.md) | PostgreSQL business truth; separate SQLite OTEL; no bodies/secrets in diagnostics; recovery evidence |
| [Service operations](service-operations.md), [Commerce](commerce-service.md), [Commercial validation](commercial-validation.md) | Scoped operators, untrusted input, deletion overlay, durable admission/uncertainty; launch decisions supersede older pricing/cap proposals |
| [Asset storage](asset-storage-research.md) | 7-8, 11-13: quarantine, private delivery, subject purge and recovery journal |

Source inspected: `df-account-app/src/view/admin.rs`, `admin/{accounts,queues,parts,clock}.rs`,
admin tests and `route.rs`; `df-portal/src/app_api/ports.rs`; provider/cost files below.
`AdminRoute` has Summary/Accounts/Account/Disputes/Reconciliation/Providers/Review/Audit, no inbox.
`ReviewQueue` is specifically uploaded-portrait review; retain it and add a distinct SupportPort.
`OperatorDirectory::{grant,append_audit}` and `FinancePort::{refund,dispute_evidence}` are seams;
they do not establish that B's real grant/MFA adapter or the new support adapters are wired.

## 2. Decisions and alternatives

| Choice | Recommendation and reason | Alternative considered |
|---|---|---|
| Mail transport | Gmail API; stable mailbox message/thread IDs, history synchronization, same-mailbox send | IMAP + SMTP works with OAuth, but adds connection/UID recovery and thread mapping; no benefit for one Gmail mailbox ([Google IMAP](https://developers.google.com/workspace/gmail/imap/imap-smtp)) |
| Mail authorization | Internal Workspace OAuth web client; owner consents as support mailbox, offline refresh token | Service account with domain-wide delegation avoids user refresh consent but grants domain impersonation authority; an application subject allowlist is not a Google-enforced one-mailbox boundary ([delegation](https://developers.google.com/identity/protocols/oauth2/service-account#delegatingauthority)) |
| Scopes | `gmail.readonly` + `gmail.send`; queue state lives in PostgreSQL, no Gmail label writes | `gmail.modify` adds unnecessary mutation; `mail.google.com` adds permanent deletion; metadata alone cannot read bodies ([scope catalog](https://developers.google.com/workspace/gmail/api/auth/scopes)) |
| Change delivery | `users.watch` -> authenticated Pub/Sub push -> durable sync work; 5-minute history polling repairs missed hints | Poll-only is a simpler temporary rollout mode using the same cursor; IMAP IDLE still needs recovery; push without reconciliation can lose changes |
| Queue authority | Native support service, PostgreSQL `support` schema, private O2 objects | Gmail labels lack revisioned proposals, audit, capabilities and atomic work claiming; workflow SQLite is development-only |
| Model work | One batched Jev judgement, separate on-demand typed LLM proposals/replies | A general tool agent has broader authority and harder audit; rules/keyword classification violates project policy |
| Integration | New small `df-support` crate; provider adapter in `df-providers`, store in `df-persistence`, P's existing UI/API | Putting business workflow in the portal or game module would couple presentation/game ownership to mail processing |

Owner must confirm this is a sign-in-capable Workspace user, not only an alias/group.
If it is an alias, authorize its owning mailbox only after narrowing what may be ingested;
do not silently read the owner's unrelated mail. Prefer a dedicated licensed support mailbox.
No DWD by default. If selected later, Workspace super-admin approves exact scopes, isolates the
credential and records its broader blast radius. Pub/Sub's push service account needs no DWD.
Use host secret files/secret manager for client secret, refresh token, Jev/LLM keys and signing
material; access tokens memory-only. Nothing secret in repo, PostgreSQL payloads, browser or logs.

## 3. Ingestion, threading, attachments and sending

### 3.1 Reliable Gmail synchronization

1. Owner selects an explicit initial import boundary (recommend all mail in this newly created
   support mailbox). Record boundary/config revision; no incidental historical personal import.
2. Establish watch and record bootstrap history H0, page the admitted mailbox snapshot, then
   replay history from H0. Include Sent and Spam/Trash discovery in reconciliation so manual
   replies and wrongly filtered rights requests are visible; protocol labels are provenance,
   never a semantic classifier. Do not filter solely on INBOX or an email body keyword.
3. Push callback verifies Google JWT signature/issuer, configured audience, expiry and expected
   push-service-account email/verification claim, subscription and mailbox binding; size-bound
   the envelope before decoding. It persists a mailbox dirty flag/work item before acknowledging.
   Notification IDs deduplicate hints; Pub/Sub IDs are not Gmail IDs ([JWT checks](https://docs.cloud.google.com/pubsub/docs/authenticate-push-subscriptions)).
4. One fenced worker per mailbox calls `history.list` from the last committed cursor, pages
   changes, fetches unknown messages, and transactionally upserts messages/tickets/work/audit.
   Advance the cursor only after all work for the covered history is durably represented.
   Treat history IDs as opaque monotonic values, not contiguous counters; reordered hints never
   move a cursor backward or skip work. A watch renewal's history ID is not a sync checkpoint.
5. Duplicate notification, history or page replay is harmless: unique `(mailbox_id,message_id)`
   and `(mailbox_id,thread_id)` constraints. RFC Message-ID is threading evidence, not dedupe.
   A deleted Gmail source becomes `source_unavailable`; it does not silently erase case evidence.
6. Renew daily and alert before returned watch expiration; Google requires renewal within seven
   days. Notifications can be dropped, so poll history every five minutes and after restart.
   A stale cursor returning 404 starts a resumable full resync within the admitted scope, with
   erasure tombstones applied before upsert. Keep existing tickets while rebuilding the cursor.
   ([Push lifecycle](https://developers.google.com/workspace/gmail/api/guides/push),
   [history recovery](https://developers.google.com/workspace/gmail/api/guides/sync).)

A push is a wake-up hint, never an email body or proof of sender identity. Retry transport work
with bounded jitter/backoff and persisted leases; 429 honors Retry-After. Expired OAuth grants
show "Reconnect mailbox" to the owner; the queue stays readable, no password/IMAP fallback.
Proposed bounds: 100 IDs/page, one mailbox sync flight, two attachment validators and two model
flights globally; 15 s total mail request, bounded 60 s work slice with durable continuation.
Full mailbox/history metadata must be admitted even when model capacity is full; overflow leaves
work pending in PostgreSQL, not dropped mail. Storage exhaustion stops cursor progress and alerts.

### 3.2 Message representation and attachment safety

Parse bounded MIME, preserve plain text and restricted provenance, prefer text/plain; derive
escaped text from HTML without loading URLs, images, styles, forms or scripts. No raw HTML UI.
Keep the full accepted text, plus an explicitly derived quote-collapsed view with a parser version
and source spans. MIME/HTML quote structure and exact already-stored message overlap may collapse
duplicates; no keyword/signature table decides what a request means. Ambiguous blocks stay visible.
Models see bounded new text plus bounded relevant thread context; they can never erase the original.
Unsupported/encrypted/oversized parts create an intake exception and visible metadata for manual
review; do not drop an entire message or pretend its inaccessible attachment was classified.

Proposed limits: 35 MiB raw MIME stream, 128 KiB decoded display text, 16 KiB model text,
20 MIME parts, 16 nesting levels, 10 attachments, 10 MiB/file and 20 MiB decoded total.
Check encoded and decoded sizes, filenames/control characters, magic bytes, MIME/extension agreement,
decompression/pixel limits and digest. Over-limit raw mail gets metadata-only intake and owner alert.
Never fetch remote attachments, links, embedded credentials or password-protected archives.
O2 stages bytes privately as quarantine objects with digest, scan version and bounded expiry.
Reuse `df-assets/src/validation.rs::validate_image` for PNG/JPEG/WebP sanitization; it currently
does not implement antivirus or a general PDF/text/polyglot validator. O2 must add isolated
malware scanning and multi-format/polyglot checks; fail closed on scan unavailable/unknown.
Proposed allowlist: these sanitized raster images, UTF-8 plain text, and PDF only after O2's
PDF sanitizer/active-content checks exist. No SVG, HTML, executables, office macros or archives.
Email attachments are neither portrait attestations nor generation references. No OCR/model upload
by default. Rejected/quarantined bytes cannot be previewed, downloaded or attached to replies.
Current media audiences lack a case-scoped operator audience: request O2's `SupportCase(TicketId)`
and authorized delivery contract; never fake an account owner or use PublicStatic/OwnerPrivate.

### 3.3 Threads and approved outbound replies

One Gmail thread maps to one ticket; no subject-only merge. Operator may relate tickets, with
audit, without copying parties or merging authorization. A new inbound reopens a resolved ticket.
New participants are unverified parties even inside a familiar thread; previous identity proofs
never cover them. A reply-all or forward can expose third-party text and requires explicit review.
Send only from `support@dungeonfluxdnd.com`, through Gmail `messages.send`, after individual send
approval. Use its `threadId`, matching Subject, original RFC Message-ID in In-Reply-To and bounded
References chain; reject CR/LF header injection. These are Gmail's documented threading conditions
([thread rules](https://developers.google.com/workspace/gmail/api/guides/threads)).
Default recipient is the reviewed sender; flag differing Reply-To/forwarded addresses, no automatic
CC/BCC or inherited reply-all. Customer-specific details go only to the independently verified
account channel, with a separate approved message if that differs from this correspondent.
Persist immutable approved MIME digest, recipient set, proposal revision, operation ID and outbox
before sending. Stable RFC Message-ID assists reconciliation; Gmail send has no assumed exactly-once
idempotency guarantee. Timeout after send -> Unknown, search/reconcile Sent and sync before any
operator-authorized retry; absence from search alone does not prove unsent. Never blind resend.
Sync manual Gmail replies as outbound history; they invalidate stale drafts and count toward service
response timing once observed. Delivery acceptance is not recipient delivery; show bounce/unknown.

## 4. Data model and ownership

Support owns workflow, not balances, credentials, gameplay truth or rights decisions.
All IDs are typed; timestamps UTC; mutations use operation keys, payload fingerprints and expected
ticket/proposal revisions. PostgreSQL is business truth; telemetry and Gmail are not action ledgers.

| Entity (`support.*`, proposed) | Required fields / invariant |
|---|---|
| mailbox / sync | Mailbox ID, configured address, OAuth secret reference only, watch expiry, committed cursor, bootstrap boundary, lease/fence, sync health |
| ticket | ID, mailbox/thread unique key, created/first-received/last-inbound timestamps, revision, status, assignee, classification head, sensitivity, response target, retention class, hold reference |
| message | Ticket, Gmail ID unique per mailbox, direction, RFC IDs/headers, received vs header-Date, envelope parties, Gmail labels, safe text/object refs, quote spans, authentication result provenance, unavailable/truncated flags |
| party / account_link | Address encrypted at rest, display label untrusted, email-match candidate, identity proof ID/source/time/expiry, confirmed AccountId or null; never match names/fuzzy addresses |
| attachment | Message/part/digest, original safe filename, declared/detected type, bytes, O2 object ID, quarantine/scan verdict, expiry, access policy; not an arbitrary URL |
| classification | Immutable version, source message-set digest/revision, schema/prompt/provider/model versions, closed judgements/confidence, resolved candidate refs, operator override origin |
| proposal | Tagged action payload, source/classification revision, evidence refs, validated target snapshot, fingerprint, expiry, state, required capability class; never executable model text |
| decision / execution | Approve/edit/reject, actor/reason, proof ID, approved payload digest; operation/outbox, attempt/result/external refs, Unknown reconciliation; immutable history |
| deadline / privacy_case | Received origin, jurisdiction/policy version, candidate/confirmed legal dates, acknowledgement/due/extension dates, owner, identity state, completion/refusal/appeal and evidence |
| audit / operation_receipt | Protected operator-audit reference and minimal typed facts, request/correlation ID; dedupe and retired namespace tombstones, no duplicated body audit |

Status enum: New -> Open -> WaitingForCustomer / WaitingForInternal -> Resolved; Spam is an
operator decision, reversible. New inbound reopens except Spam, where it enters a review bucket.
Model/work states (Pending/Running/Failed/NeedsReview) are separate from ticket resolution.
Proposal states: Draft -> Approved -> Queued -> Running -> Succeeded/Failed/Unknown;
Draft -> Rejected/Superseded/Expired. Editing creates a new draft, never changes approved bytes.
Unknown is not Failed; resolution of the ticket does not cancel an uncertain external effect.
Assignee must have current appropriate access; assignment grants no capability. Concurrent claim,
edit or reply returns a conflict with current state; no last-write-wins approval.

The coordinator assigns migration numbers; B reviews/owns support persistence integration and
operator identity/audit grants, with a dedicated support-store lane writing its allocated migration
and `df-persistence/src/support_store/**`. F/O2 retain their own schema/API amendments.
No reuse of already allocated 0005-0024 numbers; no support SQL writes into other owners' tables.

## 5. Jev classification contract

Meaning is judged under closed JSON contracts, never keyword matching, even on provider failure.
The current `SystemOneChoiceProvider::choose` returns one `usize`, requires exactly `q1`, discards
confidence and has a four-second deadline. `creator_classification.rs` demonstrates one contextual
choice and `LedgerMeter`; its old 800 ms comment is stale. Do not claim it supports this record today.
Extend the shared adapter with a typed batched-judgement method, preserving `choose` consumers.
Jev supports choice/score/noul primitives over one state, not arbitrary free-text JSON generation
([TypeSafe primitives](https://docs.typesafe.ai/primitives)). Build the versioned record from
validated primitive answers plus authoritative server lookups; do not ask Jev to invent identifiers.

Proposed JSON Schema `dungeonflux.support-classification/1` (all fields required):

```json
{"type": "object", "additionalProperties": false,
  "required": ["category", "urgency", "language", "sentiment", "confidence",
    "concerns", "account_candidate", "campaign_candidate", "session_candidate"],
  "properties": {
    "category": {"enum": ["billing_refund", "payment_dispute", "bug_report",
      "account_access", "privacy_data_request", "legal_notice_takedown_ip",
      "abuse_safety", "feedback", "sales_press", "spam", "undetermined"]},
    "urgency": {"enum": ["critical", "high", "normal", "low", "undetermined"]},
    "language": {"enum": ["en", "es", "fr", "de", "pt", "ja", "zh", "ar",
      "hi", "other", "undetermined"]},
    "sentiment": {"enum": ["positive", "neutral", "frustrated", "distressed",
      "mixed", "undetermined"]},
    "confidence": {"type": "number", "minimum": 0, "maximum": 1},
    "concerns": {"type": "object", "additionalProperties": false,
      "required": ["privacy", "legal", "safety"], "properties": {
        "privacy": {"enum": ["yes", "no", "uncertain"]},
        "legal": {"enum": ["yes", "no", "uncertain"]},
        "safety": {"enum": ["yes", "no", "uncertain"]}}},
    "account_candidate": {"type": ["string", "null"], "maxLength": 80},
    "campaign_candidate": {"type": ["string", "null"], "maxLength": 80},
    "session_candidate": {"type": ["string", "null"], "maxLength": 80}
  }}
```

Generate each candidate field's schema/choice enum from at most 20 server-authorized opaque
candidate handles plus null; the string shape above alone is insufficient validation. A handle
resolves to a typed ID only after exact verified-email lookup, identity checks and membership/owner
service confirmation. No authorized candidate -> null; too many -> manual selection, no truncated
guess. A quoted account/campaign/session ID is only an unconfirmed claim until those checks pass.
Server-owned criteria include uncertainty; record confidence is category confidence; other primitive
confidence stays in restricted evidence. It measures [answer distribution](https://docs.typesafe.ai/confidence).
Provisional category-confidence below 0.8 -> NeedsReview, never an authorization threshold.
Keep the primitive response, version and source refs in restricted classification storage for the
ticket's retention period; diagnostics carry IDs/outcomes only. Malformed/missing answers, timeout,
redaction uncertainty or unsupported language -> no decision, unclassified manual queue, no guessed
fallback. Human triage is available without a model. Sentiment never limits rights or sets priority.
Independent privacy/legal/safety concerns keep mixed requests visible even if the primary category
is billing or spam. Unknown/unclassified mail has a daily human review target, including Gmail Spam.
Operators may correct category/urgency or request metered rejudgement; keep old results, reason and
new version. New inbound invalidates old pending proposals. No automatic spam deletion or reply;
manual spam review/rescue, and per-message confirmation before suppressing mixed rights/legal cases.

## 6. Typed LLM action proposals and approval

An operator requests a plan or reply draft after seeing classification. Supply redacted text,
confirmed context, narrow policy facts and candidate handles; no tools, arbitrary retrieval or
network access in the model session. Return strict `dungeonflux.support-actions/1`: source revision,
at most five tagged proposals with evidence message IDs and bounded display explanations.
Every variant has a closed schema (`additionalProperties:false`), typed target and either complete
validated fields or `NeedsInformation`; no partial fabricated IDs/amounts/dates. Server adds required
capabilities and deadlines; the model cannot assign itself authority or approve its own output.

| Category | Proposed tagged action and owning boundary |
|---|---|
| Billing/refund | `Refund { account, lot, amount_micro_dollars, preview_revision }`: F's fresh `preview_refund` then `request_refund`; only eligible purchased credit, whole processor cents. P's current `FinancePort::refund(...lot_id,amount_cents)` is an adapter seam, not permission to bypass F |
| Payment dispute | `PrepareDisputeEvidence { dispute, evidence_refs }`: F status plus B processor deadline; operator approves exact redacted package via `DisputeResponse`; an email never proves a processor dispute or unfreezes a lot |
| Bug | `CreateBugIssue { repository_key, title, redacted_repro, expected, observed, build_ref }`: fixed configured private GitHub repository, no arbitrary URL/tool; review issue visibility and exact body before authorized creation |
| Account access | `RequestRecovery { verified_account_ref }`: B's enumeration-safe recovery flow to the registered channel; no temporary password, email reassignment, identity linking or impersonation by support |
| Privacy/data | `OpenRightsCase { subject_ref, right, scope_refs }`: B/PRIV-10 case plus typed export/erasure/restriction subproposal; privacy owner confirms legal policy/deadline and identity before executing |
| Legal/takedown/IP | `RouteLegalNotice { notice_ref, owner_queue }`, `AcknowledgeNotice { reply }`; owner/counsel determines validity, legal hold/takedown/counter-notice/appeal as separately reviewed typed decisions, never model legal advice |
| Abuse/safety | `EscalateSafety { restricted_evidence_refs }`; authorized safety owner evaluates any restriction through B/O2. Critical threat/breach follows ops incident path, not automatic sanctions |
| Feedback | `RecordFeedback { summary }` with minimal approved redaction; optional `ReplyDraft` |
| Sales/press | `RouteContact { fixed_owner_queue }`, optional `ReplyDraft`; no marketing enrollment or provider promises |
| Spam | `SuggestSpam { reason_code }`; operator confirms or rescues; no external action or paid draft by default |

`ReplyDraft { recipient_handles, body_text, allowed_fact_refs }` is shared; bounded plain text only.
Reply may state actual customer charges/balance, never provider rates, markup, keys, other players'
data or signed media URLs (D26). Prefer rendered server facts for financial details; unsupported
free-form claims are flagged for human removal. Separate draft approval from actual Send approval.
The confirmation shows exact target, amount/lot or export/erasure scope, recipients, external visibility,
irreversibility, current preconditions and required reason. Edit supersedes and invalidates approvals.
Approval binds operation ID, immutable payload digest, ticket/proposal/target revision and fresh MFA.
Revalidate grants, proof, identity, target state and second approval if F requires it at dispatch;
expired proof returns NeedsApproval without sending. Persist audit and effect intent atomically;
no remote call while holding a transaction. Audit outage fails mutation closed.
Do not hold a ticket-row lock while calling F/B/O2. Dispatch after commit; same operation key and
canonical request fingerprint flow into that owner's idempotency contract. Refund success means
F/B terminal evidence, not queue acceptance; Unknown keeps finance's payable/hold until reconciled.
GitHub create is also potentially Unknown; reconcile by private operation marker before retry.

## 7. Identity, permissions and hostile email

Trust only Gmail's receiving authentication assessment with trusted header provenance; ignore
sender-inserted Authentication-Results, display names and forwarded copies. Persist SPF/DKIM/DMARC
result/alignment and uncertainty. DMARC passing via aligned DKIM or SPF is domain evidence, not
proof of the individual/local part or account control; missing/failing/forwarded results need review.
Never demand all three pass (forwarding can break SPF), and never accept a look-alike domain.
Exact comparison uses B's verified-email normalization, not ad hoc plus/dot/fuzzy folding.
Only a verified address of record can be an account candidate; authenticated in-app re-authentication
or B's independent challenge to that address must confirm control before binding the ticket and
revealing account facts. Privacy request intake/clock never waits for this proof.

New capabilities proposed for B: SupportQueueRead (masked metadata), SupportContentRead (email
bodies/parties), SupportTriage, SupportReply, SupportAttachmentRead, BugIssueCreate, PrivacyRequest,
LegalNotice and SafetyEscalation. These are not present in today's `OperatorCapability` enum.
Preserve Refund/GoodwillCredit/DisputeResponse; no support grant implies Deletion/AccountRestriction.
Sensitive privacy/legal/safety tickets require the category capability plus content permission;
unclassified restricted content goes to an explicitly assigned intake owner. Every admin route
rechecks current grants and fresh MFA server-side per D34; sensitive actions need an action-bound
unused proof (proposed five-minute maximum, B freezes proof/replay rules). No customer cookie/PAT
or read-only integration token authorizes support mutations. Future admin token reads need separate
reviewed support scopes; v1 inbox is browser operator-session only.

JSON-escape and delimit email fields in prompts, explicitly stating quoted fields are data, never
instructions. Delimiters are not a security boundary: allowlisted variants, candidate validation,
capability checks and human approval enforce authority. Never follow emailed/model URLs or commands.
Mask secrets, tokens, card/bank data and unrelated parties before model egress using typed field
omission and security redaction; content-based meaning/redaction judgement remains schema-based.
If safe redaction cannot be established, withhold the model call and request manual handling.
Render escaped text; remote images/tracking off; suspicious links inert with full destination shown.
No bearer/signed download URLs in replies. Restricted attachment delivery is server-streamed,
no-store, attachment disposition, nosniff, current grant/case ACL per request, audited on each read.
Proposed abuse bounds: two model jobs/operator/minute, ten/sender/day (keyed address hash), one
active job/ticket/revision. Exceeding a limit queues manual work; never rejects a rights request.

## 8. Privacy, retention and rights deadlines

Store only correspondence/provenance needed to work a case, safe attachments, identity evidence
references, classification/proposals and decisions. No passwords, full cards, unsolicited ID copies
or full game transcripts; access is purpose-bound, encrypted at rest/in transit, audited and scoped.
Support is a separate processing purpose in the processor/processing register and privacy notice.
Include Google Workspace/Pub/Sub, Jev/TypeSafe and the selected draft provider; Google Limited Use
and internal-app verification/exemption checks precede model egress of Gmail data
([Workspace data policy](https://developers.google.com/workspace/workspace-api-user-data-developer-policy),
[verification](https://support.google.com/cloud/answer/13464321)). Internal consent is not a waiver
of disclosure, minimization or processor agreements. Never train a generalized model on inbox data.

The privacy review section 6 does not give a support-correspondence period. Proposed additions
below need owner/counsel approval and one policy revision; do not substitute ledger or campaign
lifetimes for email. Configure bounded purge jobs, not indefinite defaults; approval blocks live intake.

| Class | Proposed period / owner and exception |
|---|---|
| Raw MIME and extra headers | 30 days from receipt, then remove raw object; retain minimal trusted provenance. Support/O2; a scoped legal hold may retain exact evidence |
| Safe message text, accepted attachments, summaries, classifications, proposals | Open case while necessary, reviewed every 90 days; 12 months after resolution, then purge/anonymize. Support/O2; counsel-approved hold reviewed with an owner/end condition |
| Confirmed spam/rejected unsafe attachments | 30 days maximum for spam metadata/text; rejected/quarantine bytes at most 72 h, shorter if O2 policy requires. No general-access malware archive |
| Rights-request register | Separate minimal request/date/response/decision log, not full email; counsel freezes period. If CCPA applies, section 7101 requires at least 24 months ([2026 regulations](https://cppa.ca.gov/regulations/pdf/ccpa_statute_eff_20260101.pdf)); keep restricted, not model context |
| Financial/legal evidence and operator audit | Counsel-approved class/hold, finance/legal owner; no invented statutory period, no entire-ticket blanket hold |
| Exports / operation receipts / telemetry | Privacy review: export 7 days; receipts 90 days then retired namespace; telemetry proposed 30 days hot. Content derivatives expire with source even if receipt remains |
| Backups / mailbox copies | Existing 7-day PITR and 30-day daily backups with restore suppression; Workspace/Sent/Trash/Vault retention separately enforced and disclosed, not covered by deleting PostgreSQL |

Ordinary grants cannot read privacy/legal holds. DSAR includes the subject's ticket messages,
attachments, inferred labels and decisions about them with third-party/legal exemptions reviewed;
do not export raw operator audit, internal counsel notes or unrelated complainants wholesale.
Search both verified account linkage and proven historical correspondent addresses; not only current
AccountId. No account is required to lodge a request; verification must be proportionate.
Extend B's export/DeletionPlan with support rows/objects/derived indexes/prompts/exports and Google
copies; remove identity links, invalidate pending jobs, revoke delivery and replay suppression on
restore/full resync. Retain narrow exception metadata and explain each withheld scope to the subject.
With readonly/send scopes the app cannot delete Gmail: owner-run Workspace deletion/retention plus
Vault-hold reconciliation is required before claiming completion. Do not broaden scope silently.
Source-ID suppression tombstones survive while an erased Gmail copy could be reimported; retire only
after source removal/admission-boundary proof. A 90-day receipt expiry must not resurrect erased mail.

Create a provisional deadline on first observed mailbox receipt even if unclassified, then reconcile
earlier trusted receipt on import; never use the sender's manipulable Date or model-proposed date.
Proposed internal first response: critical owner escalation immediately (target 1 h acknowledgement),
high 4 h, normal 1 business day, low 3 business days; owner must staff/approve before public promises.
Service targets pause only in documented WaitingForCustomer cases; legal clocks never auto-pause.
Rights intake from support is valid: PRIV-10 also needs monitored `privacy@` routed into the same
restricted process (owner chooses alias vs separate mailbox). Daily review of unclassified/spam
is mandatory while models fail. Privacy owner confirms jurisdiction, right and due date, preserving
original receipt and calculation policy; uncertainty uses earliest applicable candidate and alerts.
Baseline from privacy review: one calendar month, with lawful two-month extension and timely notice,
for GDPR access; California know/delete/correct 45 calendar days, up to another 45 with notice,
plus 10-business-day receipt acknowledgement. Different rights may have shorter limits; no universal
"45 days" timer. Counsel validates per-right/jurisdiction rules, holidays, identity timing and appeals
([ICO access](https://ico.org.uk/for-organisations/uk-gdpr-guidance-and-resources/individual-rights/right-of-access/what-should-we-consider-when-responding-to-a-request/),
[California rights](https://cppa.ca.gov/faq)). Waiting, spam, merges and reclassification cannot reset dates.
Legal notices, processor disputes and breach reports have separately confirmed deadlines/incident
owners; an LLM does not determine statutory validity, jurisdiction, extensions or breach awareness.

Provider egress is gated on a signed data-use/retention/region record for each route. Prefer approved
zero retention/no training. OpenAI's API does not train on this data by default; ordinary abuse logs
can retain content up to 30 days, subject to exceptions. `store:false` removes application storage
where supported, not abuse monitoring; ZDR requires eligibility/approval and endpoint limitations
([OpenAI data controls](https://developers.openai.com/api/docs/guides/your-data)). Use foreground text
only, no uploaded files, hosted conversations, tools or background state. Jev retention/DPA is not
established by the local adapter or gateway price page; require written confirmation, otherwise manual
triage. Sensitive legal/privacy attachments never go to a model merely to determine their category.

## 9. Admin console experience (P's existing design)

Add `AdminRoute::Support` and `SupportTicket { ticket_id }`, paths `/app/admin/support` and
`/app/admin/support/<id>`, same `/app/sign-in`, server-checked operator session/MFA. New Support
nav item beside Review queue; account detail gets a capability-filtered related-cases link.
Use existing `admin-chrome`, `page_wide`, tables/rows, pills, reason panels, clock and CSS tokens;
sentence case, explicit text statuses, same opaque cursor paging. No separate app/design system.
Queue columns: safe subject (content grant only; otherwise "Restricted ticket"), masked party,
category, status, urgency, assignee, received/age, first-response target and legal due flag.
Filters: category/status/urgency/assignee/unassigned, age range, approaching/overdue SLA, unclassified,
authentication problem, sensitive intake and spam; stable server sorting by due date then ID.
Default view prioritizes overdue obligations, then oldest actionable unassigned; no sentiment rank.
Restricted tickets do not leak subject/snippet/attachment count through broad searches or badges.

Ticket page: chronological plain-text thread (load older page), collapsed quotes with Show original,
attachment safety states, authentication/identity warnings; right panel holds classification,
confidence/version, confirmed linked account and minimal membership/session/finance context.
Candidates say "Unverified match", reveal no account detail. Relevant account tabs open through P's
existing authorized routes; no embedded private game text or broad ledger in a generic prompt.
Actions panel shows evidence and typed proposals with Approve / Edit / Reject / Needs information;
MFA and capability reason are visible. Reply composer has Draft reply, editable text, recipient
review and distinct Send. Never auto-send on Enter; ordinary Enter remains a newline.
Show Queued/Running/Unknown/Failed with reconciliation status; disable repeats in UI but enforce
idempotency server-side. Approved external effects are not represented as reversible Undo actions.
Bulk operations: at most 50 explicitly selected IDs for assignment/ordinary status, each revision
and per-item audit/outcome. No bulk send, refund, privacy/legal action, deletion or restricted spam
suppression; ambiguous/mixed requests open individually. Selection never silently spans all pages.
Keyboard: Tab/Shift-Tab and visible focus; optional j/k navigation outside inputs, Enter opens selected
ticket, Escape dismisses panel; buttons perform actions, no hidden destructive shortcuts.
At 375 px use stacked list/thread/panel views; maintain focus, scroll anchor and unsent text across
same-phase refresh/reconnect. Drafts memory-only until explicitly saved server-side under retention;
no content/tokens in localStorage, URL/query, analytics or clipboard automatically.
Reauthorize on reconnect; stale responses cannot overwrite current draft or approvals; grant loss
clears sensitive panes/drafts. Announce action results and due-state changes accessibly, not every tick.

## 10. Metering and monthly estimates

All calls use C's shared path: required provider `CallMeter`, `LedgerMeter`/`SpendLedger`,
`BilledCall`, typed usage and dated `content/pricing.json` tariff. These types currently live under
`df-tools/src/gameplay/cost/{provider_meter,prices,specs,port}.rs`; do not import private gameplay
internals into support. C must expose a narrow shared host-level meter bridge or extract it once
with existing consumers migrated. No duplicate cost book, side counter or unmetered HTTP client.
Support work is company operational spend, never a ticket sender's wallet/campaign reservation;
C/F freeze its platform attribution/admission route before dispatch. No company spend-cap invention.
Every draft, judgement, reclassification, retry and unusable answer records actual/estimated usage,
price/provider/model/prompt revision, ticket/job/attempt/call IDs, outcome and uncertainty. Sent calls
retain liability under existing rules; queue restart does not silently replay paid work.

Illustrative USD rates from local tariff dated 2026-10-24, checked 2026-10-25: Jev input $0.042/M
(local source is [Zuplo Jev integration](https://zuplo.com/docs/ai-gateway/jev), not a TypeSafe price
commitment); drafting example `gpt-5.4-mini` input $0.75/M, output $4.50/M agrees with
[official model pricing](https://developers.openai.com/api/docs/models/gpt-5.4-mini).
Illustrative rates do not establish a selected provider or account availability.
Assume per ticket: one batched Jev call (4,000 total input tokens incl. questions), one action draft
(4,000 input + 800 output), one reply draft (2,000 input + 400 output); no caching discount/tools.
Jev = 168 micro-dollars; action = 6,600; reply = 3,300; total = 10,068 ($0.010068)/ticket.

| Tickets/month | Jev calls | Draft calls | Model cost | With 25% extra calls/usage (scenario) | Human time at 10 min/ticket, $30/h (assumption) |
|---|---:|---:|---:|---:|---:|
| 10 | 10 | 20 | $0.10068 | $0.12585 | $50 |
| 100 | 100 | 200 | $1.0068 | $1.2585 | $500 |
| 1,000 | 1,000 | 2,000 | $10.068 | $12.585 | $5,000 |

Counts depend on inbound messages/operator requests, not just tickets; a three-exchange ticket can
cost several times this. Manual-only/spam tickets can cost less. Attachment scanners, Workspace
seat, Pub/Sub, object retention/egress, database, observability and legal labor are excluded and
must be added from the owner's actual contracts. Report these separately, not as a $10 all-in inbox.

## 11. Operations, API boundary and observability

Proposed SupportPort: paged list/detail/thread, claim/update with expected revision, rejudge,
request_plan/request_reply, decide_proposal, approve_send, execution_status; explicit typed outcomes
include Conflict/NeedsIdentity/NeedsCapability/NeedsMfa/Unavailable/Unknown/OperationExpired.
P maps it into `/app/api/admin/support/**`, including request-body bounds, CSRF/no-store and
non-enumerating foreign/restricted ID refusals. B supplies server-resolved operator identity and
fresh MFA evidence; never trust `OperatorAct.operator` or requested capabilities from browser JSON.
Read projections contain only granted fields; models/attachments/credentials never run in wasm.

Durable audit captures every content/PII/attachment read, assignment/classification edit, proposal,
approval/rejection, external dispatch/result and send; safe target IDs/closed reasons, not mail body.
Queue commits reference the existing protected operator audit, with transactional audit intent if
stores differ; freeze B's crash-safe implementation before enabling mutations. Operator reason text
is private bounded data with redaction, not an unrestricted telemetry attribute.
Audit phase/read events need distinct stable child keys: current `append_audit` deduplicates by
operation ID. Preserve the external action's root key; do not collapse dispatch/result into approval.
Use `df_observe::begin` and shared `record_audit_line`; propagate correlation across sync/model/
outbox/execution. OTEL records queue age vs execution time, sync lag/watch expiry, cursor recovery,
quarantine/scan age, no-decision rate, draft corrections, approval latency, deadline misses, send
Unknown/bounce, meter write/ingestion loss and supplier cost. Never subject/body/address/token/URL.
Proposed alerts: sync lag >10 min, watch <24 h remaining with renewal failure, oldest unreviewed
intake >1 business day, deadline approaching (7 days/24 h/overdue), Unknown send/action unresolved.
An owned native worker JoinSet/leases handles shutdown/drain; jobs recheck revision, deletion and
grant fences before egress. Stale jobs are superseded; provider calls have caller deadlines and no
adapter-internal retries. Read-only `dev support` reports safe counts/age/health, never ticket text.
Restore runbook: replay O2 deletion/revocation overlay first, suppress erased mail, reconcile pending
external effects without resend, recover history cursor, then enable approved dispatch and reads.

## 12. Ordered build lanes (future dispatch, not this task)

One bounded item/worker; coordinator freezes contracts, edit leases, source/build/config and tests
before dispatch. Shared files below require serial integration, not concurrent ownership by labels.

| Order / lane | Owner and permitted future files | Exit evidence / prerequisite |
|---|---|---|
| SUP-A contracts | Coordinator + B/F/O2/C/P review; proposed `crates/df-support/{Cargo.toml,src/lib.rs,src/contracts.rs,src/ports.rs,src/fakes.rs}`; coordinator owns root Cargo.toml/lock and numbering | Freeze IDs, scopes, status/deadline policy, meter seam, operator proof/audit transaction, O2 case audience, F refund adapter; no production placeholders |
| SUP-B durable store | B-reviewed support-store lane: `crates/df-persistence/migrations/<assigned>_support_inbox.sql`, `src/support_store/{mod,queries,rows}.rs`, tests and lib export | Fakes + scratch PostgreSQL store-contract/concurrency tests; unique IDs, CAS/leases, erasure/receipt semantics; migration role separate from runtime |
| SUP-C intake/reply transport | Support worker: `df-support/src/{ingestion,mime,sync,outbox}.rs`, tests/fakes; provider owner `df-providers/src/gmail/{mod,oauth,history,send}.rs`; server owner `df-server/src/config/support.rs` and composition registration | Fake Gmail bootstrap/history/404/push/reply/Unknown; O2 quarantine integration. Server owns callback `/webhooks/support/gmail`, secrets and owned tasks; S2 owns deployment config |
| SUP-D models/meter | C/shared-provider owner first `df-providers/src/systemone.rs` + tests and shared meter seam; then `df-support/src/{classification,planner,redaction}.rs` | Batch Jev exact-schema fake tests, metering for every call; no-decision/manual fallback and privacy/provider gates, existing Jev consumers unchanged |
| SUP-E approved actions | Support owner `df-support/src/{approval,actions,reconciliation,retention}.rs`; F/B/O2/GitHub adapters in their owner paths, never direct cross-schema SQL | Typed APIs frozen, action proof/audit/Unknown/erasure integration tested; fixture issue target only, no real issue/email/provider execution |
| SUP-F admin API | P: `df-portal/src/app_api/{support.rs,ports.rs,router.rs,wire_support.rs,fakes_support.rs}` and tests; B mounts real SupportPort in composition | Every endpoint guarded, bounded and mounted; two-account/operator fixture, no fake in hosted production; retains portrait ReviewQueue |
| SUP-G admin app | P: `df-account-app/src/{route,state,api,wire}.rs`, support modules under `state/`, `view/admin/support/{queue,ticket,actions,reply}.rs`, admin nav; existing token CSS only if needed | Native + wasm checks/lints, 700 KiB build gate, browser smoke/screenshots 1280/375, reconnect/draft/failure/access acceptance |
| SUP-H integrated review | Coordinator-owned wiring/acceptance, S2 runbooks/config and Site notice updates as separate bounded items | Independent frontier evaluator operates fixed-build fake-mail flow with vision; runtime boundary evidence plus synthetic PRIV-10 and restore drill; owner-only staging Google setup later |

Needed from F: fresh refundable-lot preview revision/expiry, exact cents conversion/refusal, typed
refund operation/case reference, Unknown reconciliation/holds, dispute evidence/deadline, second-
approver threshold and platform support-cost attribution. No balance mutation or SQL workaround.
Needed from B: verified-email/historical identity lookup, independent challenge/recovery, current
grants + action-bound MFA, protected durable audit, rights intake/export/erasure participant, policy
clock ownership and migration slots. Do not expose broad account exports to ordinary support.
Needed from O2: SupportCase audience, server-only intake/quarantine bytes, antivirus/polyglot/PDF
contract, safe streaming delivery, purge/hold/tombstone/restore participant; current image check
alone does not meet this. Needed from P: routes/projections, SupportPort fakes and existing-design
views; S1 confirms membership/session handles through a directory projection, never private canon.

## 13. Acceptance catalog (SUP-01 onward)

Use fake Gmail/Jev/LLM/GitHub/F/B/O2, injected clock/randomness and barriers; no sleeps or live spend.
Keep named fixtures: refund, dispute+privacy mixed, multilingual bug, forged sender, hostile prompt,
legal attachment, forwarded request, new thread participant, Spam rights request and model outage.

| ID | Required observable proof |
|---|---|
| SUP-01 | Duplicate/reordered notifications, history pages and restart produce one message/ticket/work item; cursor never advances past unpersisted work |
| SUP-02 | Watch renewal failure, lost hints, 404 history and bootstrap race recover every admitted message; erased source never reappears; revocation pauses sync visibly |
| SUP-03 | Forged JWT/audience/subscription/mailbox rejected; trusted auth headers distinguished from attacker duplicates; valid callback is acknowledged only after durable scheduling |
| SUP-04 | MIME/HTML/quote parser preserves new and inline answers, never loads external content; encrypted/oversize mail remains visible as incomplete intake |
| SUP-05 | Fake scanner rejects malware/polyglot, MIME mismatch, PDF active content, archives and size/pixel bombs; scanner-down quarantines; private bytes never use public delivery |
| SUP-06 | Jev batch validates exact question keys/types/options/confidence; malformed/timeout -> NeedsReview, no keyword fallback; mixed privacy/legal/safety survives spam primary |
| SUP-07 | Spoofed/local-part/look-alike/forwarded/Reply-To and new-party fixtures cannot confirm identity or expose account data; verified challenge + membership resolves only allowed IDs |
| SUP-08 | Prompt injection cannot add action/tool/repository/recipient; fabricated IDs, amounts, dates and unknown fields refused; redaction failure makes no model call |
| SUP-09 | Missing/revoked/expired grant, missing/replayed/stale MFA, customer PAT and other ticket ID fail without data; content/attachment reads have protected audit evidence |
| SUP-10 | Concurrent edit/new inbound/refund-state change invalidates draft/approval; same operation+payload executes once, changed payload conflicts, retired key refuses |
| SUP-11 | Refund-vs-spend/dispute race obeys F; promotional/held/ineligible amounts refused; processor Unknown retains hold, same-key reconcile, second approval where required |
| SUP-12 | Gmail approved reply has correct mailbox/thread/RFC headers/subject and reviewed recipients; send timeout stays Unknown and restart never blindly sends twice |
| SUP-13 | Bug issue preview/approval uses fixed private target and redacted body; creation timeout reconciles before retry; legal acknowledgement cannot perform a takedown |
| SUP-14 | Rights intake works without account/model, calendar month/end-month/leap/holiday and 45-day/10-business-day fixtures match approved policy; spam/wait/reclassify cannot hide/reset clock |
| SUP-15 | Subject export includes permitted ticket inferences/attachments, excludes other parties/privileged notes; erasure removes derivatives and cancels stale model/reply jobs, narrow holds explained |
| SUP-16 | Controlled-clock retention expires raw/quarantine/resolved/exports; restore/full resync with old Google copies honors tombstones; audit/register exceptions do not retain whole bodies |
| SUP-17 | Meter records each sent call/retry/unusable/Unknown with price and estimated usage, no customer debit; captured logs/cost records contain no email/private text/secrets |
| SUP-18 | Real wasm browser fixture: filter -> claim -> read -> reclassify -> draft/edit/reject/approve -> pending/terminal reply -> resolve/reopen; accessibility and bounded bulk per-item results |
| SUP-19 | 1280/375 screenshots and keyboard pass; refresh/reconnect/reordered responses preserve text/focus; revoked access clears sensitive state; asset/scan/model failure stays usable |
| SUP-20 | Fixed-build boundary drill: audit/storage failure, worker cancellation/lease loss/restart, full-resync and deletion restore; observed trace/cost/operation IDs match durable outcomes |

For future code: pinned Rust toolchain/root rustfmt, affected native Clippy `-D warnings`, focused
tests plus real PostgreSQL contracts, wasm checks for P's changed crates and `df-tools`, P's wasm
bundle budget and lane gate. No code was changed here; no claim those runtime checks passed.
Planning verification: line budget, ASCII/LF/whitespace, relative source links, named SUP coverage
and arithmetic; retain command output in SUPPORTQ. Coordinator still owns independent review,
integration, commits/CHANGELOG, queue state and the single passive screenshot recorder.

## 14. Owner setup and decisions before live use

1. Confirm dedicated support user and permitted initial import; choose internal one-mailbox OAuth
   (recommended) or explicitly accept DWD risk. Create/select owner-controlled Google Cloud project,
   enable Gmail + Pub/Sub APIs, internal consent/client, exact server-side callback and authorized
   mailbox; review restricted-scope exemption/security-assessment applicability before activation.
2. Owner completes consent/offline authorization; seal refresh/client secrets on host, test revoke/
   rotate/reconnect procedure. Do not reuse customer Google sign-in scopes/tokens for the inbox.
3. Create topic in the watch caller's project, grant publish to
   `gmail-api-push@system.gserviceaccount.com`, create authenticated push subscription with fixed
   audience/service account and retry/dead-letter policy. S2 reviews endpoint/PKI/ingress; no public
   operator capability on callback. Register watch, expiration alarm and repair polling.
4. Verify Workspace sends from the actual support address with SPF/DKIM/DMARC. D32's separate
   `mail.dungeonfluxdnd.com` transactional sender remains for account receipts/security mail;
   threaded support replies deliberately use this Workspace mailbox. No agent changes DNS.
5. Name intake/privacy/legal/safety owners and backup coverage; create/route `privacy@`; approve
   service targets, jurisdictions/deadline rules, support retention and Gmail/Vault deletion duties.
   Counsel decides legal holds/notice/appeal procedure and confirms the minimal rights-register period.
6. Approve model route/DPA/data-use/zero-retention evidence (especially Jev), attachment allowed
   types/scanner operations, private GitHub issue target and support capabilities. Freeze F/B/O2/C/P
   contracts first; planning approval alone authorizes no cloud setup, message, charge or build.
