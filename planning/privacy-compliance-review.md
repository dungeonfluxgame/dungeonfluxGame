# DungeonFlux privacy and online-service compliance review

> **This review is not legal advice.** It is a research summary by a privacy
> analyst for the owner to take to qualified counsel. Every item marked
> "verify with counsel / verify current text" needs a current primary-source
> check before launch. Nothing here clears the service for launch.

Date: 2026-10-25. Checkout: `HEAD 03c376b4` with a dirty working tree; line
numbers are from the files read in this session and will drift. Prior review:
[legal-risk-review.md](legal-risk-review.md) sections 5 and 7 and its 12-item
checklist; this review extends them and does not repeat them. Sources read:
the five `docs/legal/*` drafts, `planning/erasure-retention-policy.md`,
`planning/account-service-research.md`, `planning/google-sign-in-research.md`,
`planning/integration-api-and-embeds-research.md`,
`planning/face-portrait-capture-research.md`,
`planning/story-memory-vector-search-research.md`,
`planning/master-launch-plan.md` sections 13.11 and 14, `artifacts/tmp/agents/LAUNCH.md`,
`crates/df-customer`, `crates/df-workos`, `crates/df-portal`,
`crates/df-account-app`, `crates/df-persistence/migrations/`,
`crates/df-providers/src/`, `crates/df-tools/src/gameplay/` (`speech_input`,
`memory`, `imagery`), `crates/df-observe`, `crates/df-telemetry`, `deploy/`,
and the `docs/` site. Web checks this session: EDPB territorial-scope
guidance summaries (Art. 27), the Munich Google Fonts ruling summaries
(LG München I, 3 O 17493/20, 20 Jan 2022), and BIPA consent/retention
summaries. All other statute text is cited from knowledge and marked
"verify current text".

How to read: **Priority** P0 = blocks offering the service to that region or
taking money; P1 = before wider release; P2 = harden/monitor. "Where" is the
repo location a builder or counsel can open. Suggested changes name the exact
file and text or behavior to add; the coordinator turns them into build lanes.

---

## One-page summary: top 10 gaps ranked

| # | Gap | Severity | Where | First action |
| --- | --- | --- | --- | --- |
| 1 | No EU/UK representative decision; service would be offered from the US to the EU/UK with no Art. 27 / UK equivalent appointment and no geo-restriction | High | No representative named in `docs/legal/privacy/index.html:112` or plan section 14; hosting US-only (`deploy/config/backup.env.example:8` `nyc3`; plan section 12.2:647) | Owner decides: appoint EU + UK representatives before EU/UK admission, or geo-block the EU/UK at sign-up (section 1) |
| 2 | Privacy draft names no AI processor; table speech, text, images and embeddings go to at least six providers unnamed | High | `docs/legal/privacy/index.html:98` ("to be completed"); `crates/df-providers/src/lib.rs:86-168` | Publish the named sub-processor list in section 4 and sign DPAs before EU/UK or California launch |
| 3 | Account export exists in the service layer but no `/app/api` route exposes it, and it omits characters, statements and uploads the policy promises | High | `crates/df-customer/src/lifecycle.rs:137-194`; no export route in `crates/df-portal/src/app_api/router.rs:23-76` | Wire the export route and extend the document; keep FILE-08 filtering as a launch gate (section 5) |
| 4 | Erasure purges account rows only; story-memory vectors, dossiers, generated media, telemetry and backups have no subject purge path | High | `crates/df-persistence/src/account_store.rs:922-924,1051-1096`; `crates/df-persistence/migrations/0003_story_memory.sql:8-45`; `0010_media_catalog.sql:188-195` | Build the cross-store `DeletionPlan` jobs in section 6 before EU/UK launch |
| 5 | Likeness (face) feature would create face-geometry data; the fal adapter still sends no `X-Fal-Store-IO` header, so inputs sit 90 days by default | High | `planning/face-portrait-capture-research.md:12,23`; `crates/df-providers/src/fal_image.rs:148,267,287` (no header found) | Keep likeness off at launch (LAUNCH decision 13); if revived, BIPA/GDPR consent + fal agreement first (section 3) |
| 6 | Voice is transcribed by ElevenLabs with no in-product microphone notice naming the processor, and provider retention/training terms are unverified | Medium | `crates/df-tools/src/gameplay/speech_input.rs:1-15`; `docs/legal/privacy/index.html:99-100` | Add the just-in-time mic notice in section 11; verify ElevenLabs STT retention before voice launches |
| 7 | Story memory embeds every table line to OpenAI and keeps events append-only with no retention limit; "remember everything" conflicts with erasure and storage limitation | Medium | `crates/df-persistence/migrations/0003_story_memory.sql:20-43`; `0004_story_memory_vector.sql:5`; `planning/story-memory-vector-search-research.md:36-41,297-301` | Adopt the retention + tombstone rule in section 6; disclose OpenAI embedding in the policy |
| 8 | Retention figures conflict across four documents (24-72 h, 7-day PITR + 30-day daily, campaign lifetime + 30 days, 90-day receipts, 30-day sessions) | Medium | `docs/legal/privacy/index.html:103-104`; `planning/erasure-retention-policy.md:95-107`; `crates/df-customer/src/retention.rs:23-27` | Replace all with the single schedule in section 6 and make purge jobs match it |
| 9 | Site loads Google Fonts from Google's CDN on every page, sending visitor IPs to Google with no basis recorded | Medium | `docs/index.html:33-35` and every `docs/**/index.html` head (e.g. `docs/legal/privacy/index.html:33-35`) | Self-host the two font families; no consent banner needed if this is the only third-party call (section 9) |
| 10 | No rights-request intake beyond in-app export/close: no privacy email flow, no deadlines, no identity-verification or refusal/appeal text | Medium | `docs/legal/privacy/index.html:105-106,112`; no intake in `crates/df-portal/src/app_api/router.rs` | Publish `privacy@` intake + the deadlines and verification rules in section 5 |

Safe to keep (checked, do not rebuild): server-side sessions in a `__Host-`
HttpOnly cookie with CSRF and no tokens in JS storage (LAUNCH decision 28;
`crates/df-portal/src/security.rs:1-5`); no Google tokens stored
(`planning/google-sign-in-research.md:26,123-129`); raw speech audio held in
memory only, never logged (`crates/df-tools/src/gameplay/speech_input.rs:7-10`);
content-free telemetry by design (`crates/df-observe/src/lib.rs:5`); the
honest "no end-to-end encryption" line (`docs/legal/privacy/index.html:93`);
trial-credit email-hash kept past deletion holding no address in the clear
(`planning/account-service-research.md:105-109`); append-only audit with
reasons for operator actions (account-service-research section 5.3).

---

## 1. Scope and roles

Current state. The service is a US-hosted, server-authoritative game offered
online: one public origin `https://dungeonfluxdnd.com` (LAUNCH decision 32),
DigitalOcean hosting (plan section 12), WorkOS identity, Stripe payments, and
AI providers for text, speech, images and embeddings. The task brief says
hosting region `nyc1`; the repo config says `nyc3`
(`deploy/config/backup.env.example:6-8`; plan section 12.2 line 647 calls
NYC3 "a candidate"). Both are US regions, so the transfer analysis is the
same, but the owner must confirm the real region before the policy names it.
No document limits who may sign up by country; the pilot is "adults only" by
self-attestation (`docs/legal/terms/index.html:95`).

| Finding | Where | Requirement | Gap | Suggested change | Priority | Verify? |
| --- | --- | --- | --- | --- | --- | --- |
| GDPR applies if EU users are targeted or monitored (Art. 3(2)); UK GDPR mirrors it | No targeting/geo rule in repo; sign-up collects only email + age attestation (`crates/df-customer/src/access.rs:180-214`) | GDPR Arts. 3(2), 12-14; UK GDPR equivalent. *Verify current text* | The drafts assume US law only; no EU/UK notices, rights or representative | Owner decision recorded in the policy: either (a) serve the EU/UK with sections 2-8 of this review implemented, or (b) geo-restrict (next row). Do not drift into EU service by default | P0 for EU/UK | Verify with counsel |
| EU representative (Art. 27) and UK representative needed for a non-EU controller offering services to the EU/UK, unless occasional/low-risk exception applies | Absent from `docs/legal/privacy/index.html:112` open items and plan section 14.3 | GDPR Art. 27; EDPB Guidelines 3/2018 on territorial scope (checked via summaries this session); UK GDPR Art. 27 equivalent. *Verify current text* | A paid, always-on game with voice and AI profiling is not "occasional"; the exception will not carry it | If the EU/UK is served: appoint one EU representative and one UK representative before the first EU/UK account; name each, with contact details, in `docs/legal/privacy/index.html` section "Who is responsible" and in the in-app privacy notice. Owner-only action (section 13) | P0 for EU/UK | Verify with counsel |
| Geo-restriction is the lawful alternative to full EU/UK compliance at pilot stage | Sign-up has IP throttles but no country gate (`planning/account-service-research.md` section 6.2) | GDPR Art. 3(2) turns on targeting; blocking EU/UK sign-ups and payments avoids triggering it. *Verify current text* | No block exists, so EU users can sign up today in design terms | If the owner defers the EU/UK: block account creation and checkout for EU/UK IP + billing country in `crates/df-portal/src/app_api/auth.rs` sign-up and the Stripe checkout route, show "not yet available in your region", and state the served countries in the terms. Revisit before any EU marketing | P0 if chosen | Verify with counsel |
| DungeonFlux is the controller for account, play, voice, memory and media; WorkOS, Stripe, DigitalOcean and AI providers are processors (or independent controllers for their own fraud/security logs) | Roles nowhere stated; `docs/legal/privacy/index.html:97-98` calls them "outside services" | GDPR Arts. 4(7), 4(8), 28; CCPA/CPRA "business" vs "service provider/contractor" (*verify current text*) | Without role labels the DPA list and the policy cannot be written | Add the role table in section 4 to the privacy policy; sign Art. 28 DPAs (or CCPA contractor terms) with each processor before launch | P0 | Verify with counsel |
| US state privacy laws (CCPA/CPRA plus the other comprehensive state laws) apply by thresholds and targeted advertising/sharing definitions, not by a single federal law | No state-law analysis in repo; no "sale/share" or opt-out language anywhere | CCPA/CPRA (Cal. Civ. Code 1798.100 et seq.); other state laws vary. *Verify current text and thresholds* | A paid game with no ads may sit under some thresholds, but California rights (know, delete, correct, limit sensitive data) should be honored by design | Adopt the section 5 rights flow for all US users (simpler than state-by-state gating); add "we do not sell or share personal information" only if true once analytics/ads choices are final; add a California notice-at-collection link at sign-up if California is served | P1 | Verify with counsel |
| COPPA applies to services directed to children under 13 or with actual knowledge of under-13 users | Adults-only attestation (`crates/df-account-app/src/view/auth.rs:137-143`); mature themes in gameplay | COPPA, 15 U.S.C. 6501-6506; FTC COPPA Rule. *Verify current text* | The service is not child-directed, which is the main defense; a checkbox alone is weak if minors in fact join | Keep 18+ positioning, neutral age screen (ask age without steering), terminate + delete accounts found to be under 18 (and any under-13 data immediately on discovery); record the procedure in the ops runbook (section 3) | P1 | Verify with counsel |
| Canada (PIPEDA), Australia (Privacy Act 1988) and Brazil (LGPD) each impose consent, purpose and rights duties with local variations | Not addressed in any draft | PIPEDA; Australia Privacy Act; LGPD. *Verify current text* | Serving those countries on US/EU text alone leaves gaps (e.g. LGPD legal bases and DPO expectations, Australian APP notices) | High level for the pilot: either include CA/AU/BR in the served-country list only after counsel maps the deltas, or geo-restrict with the EU/UK. Do not silently serve worldwide | P1 | Verify with counsel |

---

## 2. Lawful bases and record of processing

Current state. Sign-up persists an adult attestation, a terms version and an
acceptance timestamp (`crates/df-persistence/migrations/0005_accounts.sql:11-13`;
`crates/df-customer/src/access.rs:293-294`). The terms checkbox bundles terms
and privacy acceptance into one control
(`crates/df-account-app/src/view/auth.rs:165-173`). Marketing consent is
described as separate (`docs/legal/terms/index.html:95`) but no marketing
preference field or `MailPurpose` exists: every purpose in
`crates/df-customer/src/email.rs:10-21` is transactional. No lawful-basis
mapping or Art. 30 record exists anywhere in the repo.

| Finding | Where | Requirement | Gap | Suggested change | Priority | Verify? |
| --- | --- | --- | --- | --- | --- | --- |
| One bundled checkbox covers terms, privacy notice and age; no separate marketing opt-in is stored | `crates/df-account-app/src/view/auth.rs:137-173`; `0005_accounts.sql:11-13` | GDPR Arts. 6, 7 (consent must be specific, unbundled where it is the basis); PECR/ePrivacy for marketing (section 10). *Verify current text* | Terms acceptance (contract) is fine bundled with age attestation; using the same click as "consent" to all processing is not | Keep two controls: (1) required "I am 18 or older" + "I accept the terms" (contract basis, already persisted); (2) optional, unticked "Email me product news" stored as `marketing_opt_in_at` / `marketing_opt_out_at` on the account row (new migration by B). Privacy notice linked, not "accepted" | P0 | Verify with counsel |
| No lawful basis is recorded per purpose | Absent repo-wide | GDPR Art. 6 (a basis per purpose); Art. 30 record. *Verify current text* | Counsel cannot approve, and users cannot be told, why each use is lawful | Adopt the draft record below as the Art. 30 table; publish the basis column in plain language in `docs/legal/privacy/index.html` | P0 | Verify with counsel |
| Story memory and embeddings reuse play text for a second purpose (long-term recall) without a stated basis or opt-out | `crates/df-tools/src/gameplay/memory/`; `0003_story_memory.sql` | GDPR Arts. 5(1)(b) purpose limitation, 6, 21 (objection). *Verify current text* | Recall is arguably part of the game contract, but indefinite retention plus OpenAI embedding needs naming | Name "story memory" as its own purpose in the policy with basis "contract (the game remembers your campaign)" plus a settings toggle "Let the DM remember across sessions" (off = no new embeddings, existing vectors tombstoned per section 6) | P1 | Verify with counsel |

Draft record of processing (Art. 30 style; controller: the DungeonFlux seller
entity, to be named). Adopt as a table in the privacy policy and keep the
full version as an internal record.

| Purpose | Data | Basis (GDPR) | Recipients | Retention (section 6) |
| --- | --- | --- | --- | --- |
| Account and sign-in | Email, display name, password hash or WorkOS subject, age/terms record | Contract Art. 6(1)(b) | WorkOS (processor) | Account life + closure window |
| Play the game | Characters, campaign state, typed lines, rulings | Contract | AI text providers (per call) | Campaign life (section 6) |
| Voice transcription | Mic audio (transient), transcript the player confirms | Contract; mic permission in browser | ElevenLabs STT | Audio not retained (section 3) |
| Story memory | Canonical sentences, embeddings, dossiers | Contract; objection honored via toggle | OpenAI embeddings | Campaign life + tombstone on erasure |
| Generated images/video/music | Prompts, reference art, outputs | Contract | fal, BFL, OpenAI image, ElevenLabs, World Labs (per route) | Media schedule (section 6) |
| Face likeness (not at launch) | Photo (transient), face embedding at provider, portrait | Explicit consent Art. 9(2)(a) | fal (PuLID) | Until withdrawal (section 3) |
| Payments and credit | Purchase, allocation, usage records (no card numbers) | Contract; legal obligation for tax records | Stripe | Financial schedule (section 6) |
| Fraud, abuse, security | Sign-in events, throttles, audit, webhook inbox | Legitimate interests Art. 6(1)(f); legal obligation where applicable | Stripe, WorkOS (own security logs) | Security schedule (section 6) |
| Product emails | Email, marketing preference | Consent Art. 6(1)(a) | Email provider (to be selected) | Until opt-out + suppression list |
| Analytics | None on the marketing site today (no script found); in-product events must stay content-free | Legitimate interests or consent if tracking expands | None / to be named | Event schedule (section 6) |
| Telemetry and diagnostics | OTEL logs/traces, IDs and timings, no prompt bodies by design | Legitimate interests | Self-hosted (SQLite telemetry DB) | Telemetry schedule (section 6) |

---

## 3. Special and sensitive data

Current state. Voice: `POST /gameplay/speech` sends one WAV clip to
ElevenLabs Scribe v2; audio and transcript live in memory only and are never
written to disk, cached, logged or spanned
(`crates/df-tools/src/gameplay/speech_input.rs:1-15,471-500`); spans record
sizes and durations only (lines 387-402). Likeness: researched, not built;
launch decision 13 keeps uploaded real-person photos out of generation
references. Age: an "I am 18 or older" checkbox plus a persisted attestation
timestamp. Roleplay text can reveal health, sexuality, beliefs or other
sensitive topics even though the game never asks for them.

| Finding | Where | Requirement | Gap | Suggested change | Priority | Verify? |
| --- | --- | --- | --- | --- | --- | --- |
| Voice recordings are personal data; a voiceprint used to identify a speaker would be biometric data | `speech_input.rs:7-10` states "only the words are used; nothing is derived from the voice itself" | GDPR Art. 9 (biometric data for identification); Illinois BIPA, Texas CUBI, Washington biometric law cover voiceprints. *Verify current text* | The design avoids voiceprints, but no policy text or provider confirmation locks that in | Keep the words-only rule as a launch invariant; state in the privacy policy "we do not create voiceprints or identify you by your voice"; get ElevenLabs confirmation that STT creates no retained voiceprint (section 4). Any future voice cloning or speaker ID restarts this analysis | P1 | Verify with counsel / provider terms |
| Face likeness, if built, processes face geometry to generate a recognizable portrait | `planning/face-portrait-capture-research.md:10,100-102,132-167`; consent screen draft section 6.2 of that doc | GDPR Art. 9(1) + explicit consent Art. 9(2)(a); BIPA section 15 (written informed consent, public retention schedule, no sale/profit, reasonable care) checked via summaries; CUBI/Washington analogues. *Verify current text* | No consent flow, retention policy or fal agreement exists in code; the adapter lacks the no-store header | Do not ship likeness at launch. If revived: implement that research section 6 verbatim (attestation "This is my face. I am 18 or older", `X-Fal-Store-IO: 0`, 1-hour object lifecycle, "Remove my likeness" in product, consent record in PostgreSQL), publish a BIPA retention/destruction policy, sign the fal data agreement, and consider excluding Illinois until counsel clears it | P0 if built | Verify with counsel |
| Even testing likeness similarity scores is biometric processing | `face-portrait-capture-research.md:120-122` | Same as above | The doc already limits this to consenting staff with scores-only retention | Keep that limit; record consent for test subjects; delete test photos on completion | P2 | Verify with counsel |
| Children: the 18+ gate is self-attestation; GDPR Art. 8 sets the information-society consent age (13-16 by member state) and COPPA covers under-13s | `crates/df-account-app/src/view/auth.rs:137-143`; `docs/legal/privacy/index.html:107-108` admits "an age checkbox is not a full compliance program" | GDPR Art. 8; COPPA. *Verify current text* | An adult-only contract service can rely on attestation plus enforcement, but there is no discovery/termination/deletion procedure | Add to the terms and ops runbook: neutral age attestation at sign-up (no pre-ticked box), no marketing to unverified accounts, on learning a user is under 18 suspend and offer deletion of their data, on learning under-13 delete promptly and record the action. Counsel to confirm whether payment-card verification may be used as a supporting signal for hosts/funders only | P1 | Verify with counsel |
| Photos that appear to show a child must be refused without age estimation | `face-portrait-capture-research.md:150-155` | Avoids creating a second biometric use (age estimation) | Correct approach, currently only a research note | If uploads ship: human-review refusal path, no automated age estimation, no retention of the refused image | P1 if uploads ship | No |
| Roleplay text may incidentally reveal sensitive (Art. 9) topics about a player | Storyteller and memory prompts (`crates/df-tools/src/gameplay/memory/capture.rs`); mature themes noted in legal review section 5 | GDPR Art. 9 is triggered by processing that reveals special-category data; Arts. 5, 25 (minimization, by design). *Verify current text* | The game does not solicit sensitive data, but memory stores whatever is said and embeddings send it to OpenAI | Add one policy line ("Do not share sensitive personal details in play; the DM does not need them"), keep private-visibility filtering before ranking/prompting (already the memory design), and rely on erasure (section 5) rather than trying to classify every line | P2 | Verify with counsel |

---

## 4. Processors and transfers

Current state. Provider routes in code: OpenAI (text Responses, image edit,
embeddings `text-embedding-3-small` to `https://api.openai.com/v1/embeddings`,
vision judges), ElevenLabs (Scribe v2 STT `scribe_v2`, Flash v2.5 TTS, music
and SFX), fal (FLUX images, Kling/Wan video, SAM segmentation, BiRefNet),
Black Forest Labs FLUX (direct), World Labs Marble, and TypeSafe "Jev" text
(`crates/df-providers/src/lib.rs:86-168`; `crates/df-providers/src/embeddings.rs:14-18`;
`crates/df-providers/src/speech_adapter.rs:32-41`). Identity is WorkOS
(`crates/df-workos/src/adapter.rs`); payments Stripe and hosting DigitalOcean
are planned (plan sections 5, 12). The fal adapter sends references as inline
data URIs with `sync_mode` and no `X-Fal-Store-IO` header
(`crates/df-providers/src/fal_image.rs:148,267,287,292`). No DPA, SCC or
sub-processor list exists in the repo. Email delivery is still a dev outbox
writing under `artifacts/` (`planning/account-service-research.md` section 2
decision 4); no production email processor is selected. The GitHub repo link
is public marketing, not a data processor, unless issues/support move there.

| Finding | Where | Requirement | Gap | Suggested change | Priority | Verify? |
| --- | --- | --- | --- | --- | --- | --- |
| Every processor needs a DPA / service-provider terms and a named entry in the policy | `docs/legal/privacy/index.html:98` TODO; no DPA records in repo | GDPR Art. 28; Chapter V transfers (SCCs or EU-US Data Privacy Framework where the importer is certified); CCPA contractor terms. *Verify current text and each provider's DPF status* | Zero DPAs signed; transfer mechanism unknown per provider | Owner signs/records per provider: DPA, region(s), retention, training use, sub-processors. Publish the table below in the privacy policy and update it before any provider change | P0 | Verify current terms per provider |
| fal retains request payloads 90 days by default; person references already flow as data URIs for plates | `fal_image.rs:148,267,287`; `face-portrait-capture-research.md:23,157-168` | GDPR Arts. 5(1)(e), 28; the policy cannot promise shorter retention than the processor keeps | Any real-person reference sent today inherits the 90-day default | Set `X-Fal-Store-IO: 0` (and object-lifecycle preference) for all person/reference inputs in `crates/df-providers/src/fal_image.rs` and `fal_media.rs`; confirm in writing whether inline data URIs create CDN copies and how to delete them; record the answer in the provider file | P0 for person images | Verify current fal terms |
| Training use and zero-retention modes are unverified for OpenAI, ElevenLabs and Jev | `crates/df-providers/src/registry.rs` gate model; legal review section 4 | GDPR Art. 28 instructions; policy accuracy (Arts. 12-14) | API (not consumer) terms usually govern, but eligibility and defaults differ by product and account | Before paid launch, record per route: does the provider train on inputs, what is retained and how long, is zero/data-retention mode enabled on the account. Enable zero-retention where offered; name the outcome in the policy ("providers process to answer and do not train on your content" only if every route supports it) | P0 | Verify current terms per provider |
| EU/UK to US transfers need a mechanism even with a US host | Hosting US (`nyc3` config); providers US | GDPR Chapter V (Arts. 44-49): adequacy (DPF for certified importers), SCCs, or derogations. *Verify current text* | No mechanism recorded for DigitalOcean, WorkOS, Stripe or any AI provider | In the provider file, record for each importer: DPF certification (check the live list) or executed SCCs (2021 modules) in the DPA. If a provider offers neither, exclude EU/UK users from routes that send it personal data or geo-restrict (section 1) | P0 for EU/UK | Verify with counsel / DPF list |
| Google sign-in shares identity data with Google and WorkOS; no tokens are stored (good) | `planning/google-sign-in-research.md:26,123-129,225` | GDPR Arts. 13/14 transparency; Google/WorkOS terms. *Verify current text* | The sign-in button exists in design; the disclosure does not | Add to sign-up and privacy policy: "Continue with Google shares your name, email and Google verification with Google and our sign-in provider WorkOS. We do not receive or store your Google password or Google tokens." Complete the six WorkOS sandbox gates in that doc section 6 before production | P1 | Verify in WorkOS sandbox |
| Production email processor is unselected; transactional mail will carry verification and payment data | Dev outbox only (`account-service-research.md` decision 4); mail subdomain planned (LAUNCH decision 32) | Art. 28 DPA; CAN-SPAM/PECR handling in section 10 | No provider, DPA or region to disclose | Select the email provider as an owner task; until then the policy lists it as "email delivery provider (to be named before launch)" and launch is blocked on naming it | P1 | Verify with counsel |

Sub-processor list to publish (fill region/retention from the signed terms;
do not publish until each row is verified):

| Processor | Purpose | Data | Region / transfer | Retention to confirm |
| --- | --- | --- | --- | --- |
| WorkOS | Sign-in, identity | Email, name, auth events | US (confirm) | Account life |
| Google (sign-in only) | Google identity | Name, email, verification flag | Google global | No tokens stored by DungeonFlux |
| Stripe | Payments | Payer identity, amounts; card data only with Stripe | US/IE (confirm) | Stripe schedule |
| DigitalOcean | Hosting, PostgreSQL, Spaces | All hosted data | US `nyc` (confirm 1 vs 3) | Contract + backups |
| OpenAI | Narration, image edit, embeddings, vision checks | Table text, canonical sentences, scene images | US (confirm) | API retention / zero-retention status |
| ElevenLabs | Speech-to-text, text-to-speech, music/SFX | Mic audio (transient), narration text, prompts | US/EU (confirm) | STT/TTS retention, voiceprint answer |
| fal | FLUX images, video, segmentation, background removal | Prompts, reference images | US (confirm) | 90-day default until no-store is set |
| Black Forest Labs | FLUX images (direct) | Prompts | DE/US (confirm) | Model licence + retention |
| World Labs | Generated 3D worlds | World prompts/images | US (confirm) | Terms unread (legal review section 4) |
| TypeSafe "Jev" | Text judgments | Story text excerpts | Unknown | Written terms needed |
| Email provider (unselected) | Transactional + consented product mail | Email, message content | TBD | Until delivery + provider schedule |

---

## 5. Data subject rights

Current state. Export is implemented as a service method returning a small
account JSON (identity, sessions, security events; capped at 8 MiB, recent
sign-in within 10 minutes required) and explicitly excludes game content
(`crates/df-customer/src/lifecycle.rs:19-20,37-38,136-194`). No route in
`crates/df-portal/src/app_api/router.rs:23-76` calls it, so users cannot reach
it in the app. The richer media export (encrypted archive, per-export key,
7-day-style expiry, access log) exists as schema
(`crates/df-persistence/migrations/0012_media_exports.sql:5-29`). Closure:
close with 5-minute re-authentication, 30-day reopen window, then retention
purges the provider user and account rows
(`crates/df-customer/src/retention.rs:77-133`;
`crates/df-customer/src/tunables.rs:79`). Rectification is display-name only
(profile save). No restriction, objection, portability-format, email-intake,
deadline or refusal process exists.

| Finding | Where | Requirement | Gap | Suggested change | Priority | Verify? |
| --- | --- | --- | --- | --- | --- | --- |
| Access/export is unreachable in the product and narrower than the policy promise (profile + characters + statements + uploads) | `lifecycle.rs:137-194`; missing route in `router.rs`; promise at `docs/legal/privacy/index.html:106` | GDPR Art. 15 (access) and Art. 20 (portability in a structured, commonly used, machine-readable format); CCPA know/access. *Verify current text* | A built but unmounted export fails the right in practice | Add `POST /app/api/settings/export` + download route in `df-portal` calling `CustomerService::export`; extend the JSON with display name history-free profile, character templates (requester-owned only), usage statements and owned-upload metadata; keep `dungeonflux.account-export/1` versioning; JSON satisfies portability for account data, media delivered via the 0012 archive flow | P0 | No |
| Export must filter other players' private data (FILE-08) | Plan section 13.11 line 863 and test FILE-08 line 1067 ("Test must fail until filtering is correct"); story visibility labels in `0003_story_memory.sql:24-27` | GDPR Art. 15(4) (rights of others); Art. 5 audience enforcement | Campaign excerpts naturally contain other players' lines, notes and hidden state | Export only rows where the requester is owner or a `public`/`party` audience member entitled to see them; exclude `private` events not addressed to the requester, hidden DM state, credentials and operator audit. Keep FILE-08 as a release-blocking acceptance test with a two-account fixture | P0 | No |
| Erasure works for account rows but does not reach story vectors, dossiers, recalls, media bytes, telemetry or derived indexes | `account_store.rs:922-924` purge; `0003_story_memory.sql` (append-only, tombstone only via maintenance flag lines 60-61); `0010_media_catalog.sql:188-195` tombstones exist but no subject wiring shown; telemetry SQLite in `crates/df-telemetry/src/persistence.rs` | GDPR Art. 17; CCPA delete. Exceptions: legal retention (tax), others' rights in shared history. *Verify current text* | The erasure policy (suppression overlay, tombstones, restore-through-overlay) is designed but, in current code, only the account schema is purged | Build the section 6 purge set: on closure completion, tombstone the subject's story events (replace body with redaction marker, null the embedding, drop dossier coverage), purge owned media via existing `media.tombstone` + object deletion, expire exports, and record per-scope outcomes. Shared campaign history keeps minimal nonpersonal provenance per `erasure-retention-policy.md:120-128` | P0 for EU/UK/CA | No |
| Backups resurrect deleted data unless tombstones are replayed on restore | `erasure-retention-policy.md:77-88`; backups 7-day PITR + 30-day daily (plan section 13.10) | Art. 17 with backup ageing disclosed; restore must re-apply suppression | The policy requires it; no restore test evidences it here | Keep the disclosed rule ("backups age out within 30 days; restores re-apply deletions before serving") in the policy and add a restore-drill check to the S2 runbook: restore, replay tombstones, verify the deleted account cannot be read or exported | P1 | No |
| Rectification, restriction and objection have no product path | Only display-name edit (`customer.rs` save_profile); no toggles | GDPR Arts. 16, 18, 21; CCPA correct/limit. *Verify current text* | Users can fix a name but cannot correct statements, pause processing or object to memory/analytics | Add in Settings: correct-display-name (exists), "request correction" free-text to the privacy intake for statement errors, restriction = close/freeze account (sessions revoked, no new dispatch) while a dispute is reviewed, objection toggles for story memory (section 2) and marketing (section 10). Each writes an audit event | P1 | Verify with counsel |
| No intake channel, deadline, identity check or refusal/appeal wording | `docs/legal/privacy/index.html:105-106,112`; no privacy mailbox route | GDPR Art. 12 (one month, extendable by two for complexity, with notice); CCPA 45 days; identity verification proportionate (Art. 12(6)). *Verify current text* | Rights by email are impossible; in-app only helps signed-in users | Publish both channels: in-app Settings > Privacy (export, close, toggles) and `privacy@dungeonfluxdnd.com` (owner creates mailbox). Verify identity by requiring sign-in/re-auth in-app, or by replying to the account email of record (never accept ID documents by default). State deadlines in the policy: "We answer within one month (GDPR) / 45 days (California); if we need longer we will tell you within the first period." Log request, verification, scope outcomes and completion date in the operator audit | P0 | Verify with counsel |

---

## 6. Retention schedule

Conflicts found (must be reconciled into the one table below): rejected
output 24-72 h (`docs/legal/privacy/index.html:104`, repeated on the pricing
page); backups 7-day PITR + 30-day daily (same page; plan section 13.10);
campaign lifetime plus a 30-day deletion window and backup candidate at most
30 days (`planning/erasure-retention-policy.md:95-107`); operation receipts 90
days, sessions 30 days after end, tokens 24 h
(`crates/df-customer/src/retention.rs:23-27`); challenge tokens 15 minutes,
raw capture at most 24 h (erasure policy lines 98-101). The code values win
where code exists; counsel approves the rest. Delete "proposed / may change"
from the published policy once approved: publish one schedule and enforce it.

| Category | Period (adopt) | Enforced where | Notes |
| --- | --- | --- | --- |
| Account profile, credentials, identity link | Account life; on closure, 30-day reopen window then purge | `df-customer` retention pass (`retention.rs:81-133`) — exists for account rows | Keep the trial-grant email hash past deletion (no address in clear) and say so in the policy |
| Web sessions | Idle 14 days, absolute 30 days; ended sessions deleted 30 days after end | `retention.rs:23`; `account_store.rs:1054-1057` — exists | Publish "sign-in history kept 30 days after a session ends" |
| Verify/reset/email-change tokens | Expire 15 min (reset) / 24 h (verify, change); delete 24 h after expiry | `retention.rs:25`; `account_store.rs:1066-1069` — exists | Matches erasure policy challenge class |
| Operation receipts | 90 days, then nonreplayable tombstone | `retention.rs:27`; `account_store.rs:1081-1085` — exists | Deleting the row alone is not enough: keep the retired-namespace refusal from the account plan |
| Typed play text, rulings, campaign state | Campaign life; on account erasure, subject's private payload tombstoned at once, physical purge within 30 days | Needs new purge job in `df-persistence` story store + `df-tools` memory (section 5) | Shared committed outcomes keep minimal nonpersonal provenance; never regenerate deleted text |
| Story embeddings, dossiers, recall packs | Same as the source event; embedding nulled on tombstone | `0004_story_memory_vector.sql:5`; dossier table in `0003` — job to build | A vector can reveal its text; treat it as the text for retention |
| Raw microphone audio | Not retained (memory only); diagnostic sample only with separate consent, at most 24 h | `speech_input.rs:7-10` — exists by design | Provider-side retention governed by section 4 confirmation |
| Generated media (images, audio, video) | Campaign/account life; intermediate and rejected output 24-72 h; quarantine per media states | `0010_media_catalog.sql:40-45,64-65` states exist; sweep job (`0014_media_sweep.sql`) to verify | The 24-72 h figure survives only for rejected/intermediate output |
| Uploaded portraits | Until removed, account deletion, or review rejection | Media catalog + upload table (`0010:136-151`) | Refused child-appearing images: no retention (section 3) |
| Exports | Download window 7 days, then archive + wrapped key deleted | `0012_media_exports.sql:19` — expiry modeled; job to verify | State 7 days in the policy (replaces "a short time, for example") |
| Stripe webhook raw bodies | 30 days after processing, then delete body, keep event metadata | Account plan section 5.2 gateway inbox | Add purge to the B retention pass |
| Billing, tax, ledger records | Period set by counsel per launch jurisdiction (do not invent a number); restricted store, no gameplay access | Finance schema (`0020`-`0024` migrations) | This is the Art. 17 exception; name the basis, not a guessed period, until counsel sets it |
| Security/audit logs (account events, operator audit) | Account events with the account; operator audit per counsel-approved period | `0005_accounts.sql:98-107` append-only | Never include prompt bodies, tokens or card data |
| OTEL telemetry (SQLite) | Set a fixed period (propose 30 days hot, then delete); spool bounded by size today | `crates/df-telemetry/src/persistence.rs` — add age purge next to the size quota | Telemetry must be inside every `DeletionPlan` scope list |
| Backups | PITR 7 days; independent daily 30 days; tombstones replayed on every restore | S2 runbooks (`deploy/scripts/restore-drill.sh`) | Disclose backup ageing in the policy; never promise instant backup erasure |
| Provider copies | Shortest the provider allows; fal no-store once section 4 lands | Provider file (section 4) | Policy states provider retention per provider, not one blanket promise |

Purge-job rule for every row above: each period names an owning job, a bounded
batch, an audit line with counts only, and a test with a controlled clock.
Rows marked "exists" still need their tests run at integration; rows marked
"to build/verify" are build tasks in section 12.

---

## 7. Security and breach response

Current state, strengths to keep: Argon2id password hashing with bounded
concurrency (account-service-research section 4.4), hashed tokens and session
digests (`0005_accounts.sql:44,74`), `__Host-` HttpOnly SameSite=Lax cookies
with CSRF, origin and `Sec-Fetch-Site` checks, rate limits and keyed-hash
throttles, append-only audit with mandatory reasons for operator actions,
separate operator MFA (TOTP) and scoped time-limited grants, parameterized
SQL with tenant predicates, per-process isolation for campaign runtimes and
an egress dispatcher that re-checks fences (LAUNCH decisions 18, 23), Spaces
private by default with short-lived authorized delivery (plan section 13).
Gaps are about proof and process, not missing primitives.

| Finding | Where | Requirement | Gap | Suggested change | Priority | Verify? |
| --- | --- | --- | --- | --- | --- | --- |
| Encryption in transit/at rest is assumed, not evidenced per store | Plan section 13 (Spaces/DB encryption at rest noted line 794); `deploy/` compose and Caddy edge | GDPR Art. 32 (encryption, confidentiality, resilience, regular testing). *Verify current text* | No single record states TLS versions, DB/Spaces encryption, key custody or the export-key model (per-export key under an S2 KEK, LAUNCH decision 14) | Add a one-page security record (internal, summarized in the policy as "encryption in transit and at rest") listing: edge TLS, Postgres/Spaces at-rest encryption, secret store, backup encryption + offline age key, export keys, admin MFA/VPN path (plan section 12.7) | P1 | No |
| Access control is strong in design; least-privilege proof (DB roles, RLS gate) is still a deployment checklist item | Account plan section 5 (runtime role cannot migrate; RLS "a recorded deployment gate") | Art. 32; Art. 25 | A gate that is only recorded can be skipped silently | Make the S2 qualification checklist fail closed: staging checks (`deploy/qualification/staging-checks.csv`) must evidence role separation, trusted sources, firewall and non-public DB before prod data | P1 | No |
| No breach-notification runbook: the incident runbooks cover money, database, secrets and storage, not personal-data breach duties | Plan section 16.4 runbooks; section 16.3 P0 includes "leaked secret" and "cross-tenant disclosure" | GDPR Art. 33 (authority within 72 hours of awareness) and Art. 34 (subjects when high risk); UK equivalent; US state breach laws (varying triggers/deadlines). *Verify current text* | A P0 today stops admission but nobody is tasked with assessing, notifying or documenting a breach | Add "Personal data breach" to plan section 16.4 (owner: S2 drafts, B/O2 supply scope data): detect and timestamp awareness, contain, assess data/subjects/processors involved, notify the lead authority within 72 h where required (for non-EU controllers without one-stop-shop, notify each concerned authority — confirm mechanics with counsel and the Art. 27 representative), notify affected users without undue delay when high risk, notify processors without undue delay in reverse, keep a breach register (facts, effects, measures) even when not notified | P0 for EU/UK | Verify with counsel |
| Processor breach duties are not in any DPA checklist | Section 4 (no DPAs yet) | Art. 28(3)(f) assistance; Art. 33(2) processor-to-controller notice | Providers may learn of a breach first (e.g. a media or identity provider) | In each DPA (section 4), require breach notice to DungeonFlux without undue delay and name the security contact mailbox; test the contact path in staging | P1 | Verify current terms |
| Security testing cadence is not stated | Plan section 16 targets; acceptance catalog section 17 | Art. 32(1)(d) regular testing. *Verify current text* | Restore drills exist; access-control and export-filter tests need the same cadence | Add to the release checklist: FILE-08, closure-purge, restore-through-overlay and cross-tenant fixtures run per release; record build/config/revision tested (AGENTS.md handoff rule) | P2 | No |

---

## 8. Privacy by design and DPIA

Current state. By-design strengths already present: thin clients with
server-side authority (AGENTS.md), audience labels enforced before
retrieval, ranking, prompting and delivery (plan section 14.4), content-free
telemetry (`crates/df-observe/src/lib.rs:5`), data-minimal identity (no
Google tokens), memory-only audio, and defaults that keep likeness and
uploaded real-person references off at launch. No DPIA or Art. 25 assessment
document exists.

| Finding | Where | Requirement | Gap | Suggested change | Priority | Verify? |
| --- | --- | --- | --- | --- | --- | --- |
| No Data Protection Impact Assessment for voice + AI profiling + likeness + systematic story memory | Absent; triggers present in `speech_input.rs`, `memory/`, `face-portrait-capture-research.md` | GDPR Art. 35 (DPIA for new technologies, biometric identification, systematic monitoring/evaluation; many supervisory lists name these). *Verify current text and the lead authority list* | The highest-risk features have research docs but no risk assessment, necessity/proportionality analysis or residual-risk sign-off | Complete the DPIA outline below before voice + memory launch to the EU/UK; likeness needs its own prior assessment and, if high risk remains, prior consultation (Art. 36). Owner signs residual risk | P0 for EU/UK voice/memory; P0 before likeness anywhere | Verify with counsel |
| Privacy by default: memory, voice and marketing must start off or minimal, not on | Memory design embeds by default; mic is opt-in (good) | Art. 25(2) (only necessary data, by default). *Verify current text* | Mic default is right; story-memory embedding and any future analytics have no user-visible default control | Defaults to ship: microphone off until used; story memory on for the campaign (contract necessity) with the objection toggle in section 2; marketing off (unticked); no analytics cookies (section 9); likeness off/absent | P1 | Verify with counsel |

Draft DPIA outline (one document, owner-approved, reviewed on change):
1. Describe the processing: voice transcription, AI narration/judgment,
   story-memory embeddings and recall, generated media, and (separately)
   likeness; data flows per provider (section 4 table); who is affected
   (players, other table members whose speech is transcribed or remembered).
2. Necessity and proportionality: why typed input alone is insufficient for
   voice; why embeddings are needed for recall; less-intrusive alternatives
   considered (full-text only, local transcription, no likeness) and why
   they were accepted or rejected; data-minimization choices already made.
3. Rights and freedoms risks: re-identification from vectors/dossiers,
   other players' speech captured in memory, sensitive-topic capture,
   provider retention/training, child access, likeness misuse (photo of
   someone else), function creep of transcripts.
4. Measures: audience filtering before ranking, memory-only audio, no-store
   provider settings, tombstone erasure, closure purge, age gate,
   consent screens, audit, encryption record (section 7), user toggles.
5. Residual risk and sign-off: what remains (e.g. provider copies during
   their retention window), who accepted it, review date and triggers
   (new provider, new purpose, likeness decision).

---

## 9. Cookies and tracking

Current state. The marketing/legal site is static HTML/CSS with no authored
JavaScript (`docs/README.md`, confirmed by search: no `document.cookie`,
`localStorage`, analytics or tag code in `docs/`). It sets no cookies itself.
Every page loads Spectral + Barlow Semi Condensed from Google Fonts
(`docs/index.html:33-35`, `docs/styles.css:19-20`), which sends each
visitor's IP address (and user agent/referrer within browser limits) to
Google before any consent interaction. The app uses strictly-necessary
cookies only: `__Host-` session and pre-session cookies, host-only, Secure,
HttpOnly, SameSite=Lax, with CSRF (`crates/df-portal/src/security.rs:1-5,20-22`;
`crates/df-customer/src/tunables.rs:74-80` lifetimes). No analytics, ads,
embeds or third-party frames were found on the site; share/embeds are design
only (`planning/integration-api-and-embeds-research.md` section 3: embed page
to use no cookies, no scripts, no third-party trackers — keep that rule).

| Finding | Where | Requirement | Gap | Suggested change | Priority | Verify? |
| --- | --- | --- | --- | --- | --- | --- |
| Google Fonts via CDN discloses visitor IPs to Google without a basis; a Munich court found dynamic Google Fonts embedding unlawful without consent because self-hosting was available (summaries checked this session; much-debated, single regional court) | Font links in every `docs/**/index.html` head | GDPR Art. 6 (basis for the disclosure/transfer); ePrivacy device-access rules do not cover fonts, the GDPR transfer analysis does. *Verify current text* | No basis is recorded and none is needed for the fonts to work; consent-before-load would break typography | Self-host the two families as WOFF2 under `docs/assets/fonts/` with `@font-face` in `docs/styles.css`; delete the `fonts.googleapis.com`/`fonts.gstatic.com` links and preconnects site-wide; record the OFL licences in `THIRD_PARTY_NOTICES.md`. Result: the static site makes zero third-party requests and needs no banner for fonts | P1 (P0 if serving Germany/EU without change) | Verify font licences |
| No cookie banner is required for the current footprint — and adding one anyway would be wrong | Site sets no cookies; app cookies are strictly necessary for sign-in/security | ePrivacy Directive / UK PECR: strictly-necessary cookies are exempt from consent; non-essential storage/access needs prior consent. *Verify current text* | The privacy draft lists "cookie and local storage disclosures" as open (`docs/legal/privacy/index.html:112`), inviting an unnecessary banner | Publish a short cookie notice (in the privacy policy, no banner): name each `__Host-` cookie, purpose (sign-in session, pre-session CSRF), lifetime (idle 14 days / absolute 30 days), and state "we use no analytics or advertising cookies". If analytics, embeds or A/B tools are ever added, gate them behind prior opt-in consent that blocks the third-party call until accepted | P1 | Verify with counsel |
| Future share embeds must not become a tracking surface | `integration-api-and-embeds-research.md:222-233,248-256` (no cookies/scripts/trackers; aggregate view counts) | ePrivacy/GDPR if viewer data is collected on third-party sites | Design is sound; risk is implementation drift (fonts, analytics, viewer history) | Build the embed exactly as designed: first-party media only, self-hosted/no fonts, aggregate counters with short retention, `Referrer-Policy: no-referrer`; add a release check that the embed page issues no `Set-Cookie` and makes no third-party request | P2 | No |

Minimal consent design if tracking is ever added (do not build now): one
banner with equal "Accept" / "Reject" buttons, no pre-ticked categories, no
third-party script loaded before choice, choice stored server-side for
signed-in users and in a strictly-necessary consent cookie for visitors,
re-asked on purpose change. Anything less (dark patterns, consent-or-leave
for non-essential tracking) is the fact pattern regulators fine.

---

## 10. Marketing and email

Current state. Only transactional mail exists (verification, reset, email
change, password/email changed notices, campaign-deletion warning) in
`crates/df-customer/src/email.rs:10-35`. No marketing preference, unsubscribe
mechanism, suppression list or production email provider exists. The terms
draft correctly separates marketing consent
(`docs/legal/terms/index.html:95`) and the privacy draft states "marketing
messages need separate consent" (`docs/legal/privacy/index.html:110`).

| Finding | Where | Requirement | Gap | Suggested change | Priority | Verify? |
| --- | --- | --- | --- | --- | --- | --- |
| No marketing consent record, so the first product email would have no lawful basis | Sign-up form (`view/auth.rs`); no field in `0005_accounts.sql` | GDPR Art. 6(1)(a)/7 + ePrivacy/PECR soft opt-in limits; CAN-SPAM (US: opt-out model, honest headers, postal address). *Verify current text* | Code and schema cannot express the consent the drafts promise | Add the unticked opt-in (section 2), store grant/withdrawal timestamps, and add `MailPurpose::ProductNews` gated on a current opt-in at send time in `df-customer`. Never infer consent from account creation or purchase | P0 before any marketing send | Verify with counsel |
| No unsubscribe or suppression handling | `email.rs` (no preference link concept) | PECR/CAN-SPAM: every marketing message needs a working opt-out, honored promptly; transactional mail exempt from consent but not from identification duties. *Verify current text* | Nothing to honor an opt-out with; a manual "email support" answer will not scale or evidence compliance | One-click unsubscribe (signed link + in-app toggle), honored before the next send (build for immediate; counsel to confirm the outer deadline per regime), suppression stored as a keyed email hash so a deleted account stays suppressed, physical postal address of the seller in every marketing footer once the entity exists | P0 before any marketing send | Verify with counsel |
| Sender identity and subdomain setup are planned but not a launch record | LAUNCH decision 32 (`mail.dungeonfluxdnd.com`, SPF/DKIM/DMARC as config) | CAN-SPAM accurate From/subject; deliverability and spoofing control | Config intent, no verified records or provider | Owner task: select provider (section 4), publish SPF/DKIM/DMARC, use the mail subdomain for all sends, keep `support@` and the new `privacy@` mailboxes monitored | P1 | No |

---

## 11. Consumer notices at collection points

Consumer and transparency law meet at the moment data is collected: the
notice must be there, specific and truthful before the data flows. The
privacy policy alone does not cure a silent collection point.

| Finding | Where | Requirement | Gap | Suggested change | Priority | Verify? |
| --- | --- | --- | --- | --- | --- | --- |
| Sign-up collects email, age and terms acceptance with no short privacy notice at the form | `crates/df-account-app/src/view/auth.rs:137-173` (checkboxes link the legal pages; no summary) | GDPR Arts. 13/14 (information at collection); US state notice-at-collection. *Verify current text* | A link to a draft policy that "does not bind anyone" is not a launch notice | Under the sign-up button add: "We use your email to run your account. What your table says and types is sent to AI services to play the game (named in the privacy policy). You can export or delete your data in Settings." Replace the draft policy with the counsel-approved text before the first real account; remove `noindex` and the unbounded draft banner on all five legal pages at the same gate | P0 | Verify with counsel |
| Microphone has no just-in-time notice naming the processor or the no-retention rule | `speech_input.rs` flow; privacy draft `docs/legal/privacy/index.html:99-100` is generic | Arts. 13, 5 (transparency, minimization); the browser permission prompt does not explain DungeonFlux's use | Players grant the browser permission without learning audio goes to ElevenLabs and that typing always works | First-use mic dialog (and a line beside the mic control thereafter): "Your voice is sent to ElevenLabs to turn into text. We do not keep the recording. Typing always works instead." Buttons: "Use microphone" / "Keep typing". No recording indicator gaps: show live state while the clip uploads | P0 before voice launch | No |
| Photo upload (portraits) has rights attestation fields but no privacy notice at the point of upload | `0010_media_catalog.sql:146-147` (`attests_rights`, `depicts_real_person`); LAUNCH decision 13 screening | Arts. 13, 9 if a face is analyzed; consumer fairness (no surprise screening/retention) | Attestation covers rights, not what happens to the image (screening, review queue, retention, provider) | Upload dialog adds: "Uploaded portraits are screened automatically; flagged images go to a person for review. We keep the image while it is in your library. Do not upload photos of children or of other people without their permission." Likeness, if ever built, uses the separate explicit-consent screen in section 3, never this dialog | P1 | Verify with counsel |
| Share links can publish other players' names, characters and images on host consent alone (design stage) | `integration-api-and-embeds-research.md:184-204` (already requires each included player's opt-in) | GDPR Arts. 6, 7, 17/21 for the depicted players; consumer fairness | Design is correct; the risk is a builder simplifying it to host-only consent | Build exactly as designed: per-player opt-in bound to the exact preview, omission path, revocation suspends links, expiry 7/30/90 days, `noindex` by default, report action. Add a launch test (SHARE-02/03 in that doc) to the acceptance catalog | P1 | No |
| Pricing and trial claims interact with privacy (what is free, what is metered) without a data-use line | `docs/legal/credits/index.html:101-104`; `docs/pricing/index.html` | Consumer protection: clear total price and material terms before purchase. (Money-law analysis stays in the legal review section 6.) | The $10 trial and pace pricing are hedged as drafts; no line explains that play content goes to paid AI providers per call | At checkout and first paid session, show: "Play uses AI services paid from your credit. Your table's words are sent to those services to run the game." Keep draft/`noindex` framing until the offer is real (legal review risk 10) | P2 | Verify with counsel |

---

## 12. Concrete change list (build tasks)

Disjoint file areas; the coordinator assigns lanes. Owners: B accounts
(`df-customer`, `df-persistence` account store), P account app
(`df-account-app`, `df-portal/src/app_api`), O2 media (`df-assets`, media
catalog/sweep), F money (finance, processor records), S2 ops (deploy,
runbooks), Site (`docs/`). No task here edits another task's files.

| Task | Change (exact) | Files | Owner | Proof / evidence |
| --- | --- | --- | --- | --- |
| PRIV-01 Policy rewrite | Replace draft sections in `docs/legal/privacy/index.html`: named controller + representatives (or served-country limit), role table, sub-processor table (section 4), basis summary (section 2), single retention schedule (section 6), rights + deadlines + `privacy@` intake (section 5), cookie notice (section 9), voice/likeness lines (sections 3, 11). Remove draft banner/`noindex` only at the counsel gate, together with terms/refunds/credits/content | `docs/legal/privacy/index.html` (+ matching lines in the other four legal pages) | Site | Counsel-approved text; page names every provider in the production config (diff config vs page) |
| PRIV-02 Self-host fonts | Add WOFF2 files, `@font-face`, delete Google links/preconnects site-wide, record OFL in notices | `docs/assets/fonts/*`, `docs/styles.css`, `docs/**/index.html` heads, `THIRD_PARTY_NOTICES.md` | Site | Browser run: zero third-party requests on home + legal pages; visual parity screenshot |
| PRIV-03 Marketing consent | Add `marketing_opt_in_at`/`marketing_opt_out_at` + suppression hash table (new migration, next free number from coordinator); unticked sign-up checkbox; Settings toggle; `MailPurpose::ProductNews` gated at send; one-click unsubscribe route + footer address slot | `crates/df-persistence/migrations/<next>_marketing_consent.sql`, `crates/df-customer/src/{access,email,records,ports}.rs`, `crates/df-persistence/src/account_store*`, `crates/df-account-app/src/view/auth.rs` + settings view, `crates/df-portal/src/app_api/*` | B + P | Tests: no opt-in = no send; opt-out honored before next send; deleted account stays suppressed; fake-mail assertions |
| PRIV-04 Export route + scope | Mount `POST /app/api/settings/export` and an expiring download; extend the export document (templates, statements, upload metadata); FILE-08 two-account filter test | `crates/df-portal/src/app_api/{router,customer}.rs`, `crates/df-customer/src/lifecycle.rs`, acceptance test FILE-08 | P (service change B if needed) | Browser: signed-in export downloads; fixture with another player's private note proves it is absent (test fails before, passes after) |
| PRIV-05 Subject erasure across stores | On closure completion, run the cross-store purge: story tombstones + embedding nulling + dossier invalidation, owned-media purge via `media.tombstone`, export expiry, telemetry scope entry; per-scope outcome audit | `crates/df-persistence/src/{story,media}_store*` (owner-confirmed paths), `crates/df-customer/src/retention.rs` (orchestration call only), `crates/df-tools/src/gameplay/memory/*` (redaction API) | B with O2 + memory owner via coordinator | Controlled-clock test: close account, run passes, assert story rows redacted, vectors null, media purged, restore-through-overlay drill shows no resurrection |
| PRIV-06 Mic notice + memory toggle | First-use microphone dialog with the section 11 text; Settings toggle "story memory across sessions" (off stops new embeddings and tombstones existing vectors for that account's private events) | `crates/df-account-app/src/view/*` (game/account surface per owner), `crates/df-tools/src/gameplay/{speech_input,memory}/*` (enforcement flags), `crates/df-portal/src/app_api/customer.rs` (settings field) | P + gameplay owner | Browser playthrough: dialog appears once, typing path unaffected, toggle off produces no new embedding calls against the fake provider |
| PRIV-07 fal no-store + provider file | Send `X-Fal-Store-IO: 0` + lifecycle preference for person/reference inputs; complete the per-route provider terms file (DPA, region, retention, training, zero-retention) as an internal record the policy mirrors | `crates/df-providers/src/{fal_image,fal_media}.rs`; provider record in `planning/` only if the coordinator names it, else lane artifact | O2 (adapters) + owner (terms) | Adapter tests assert the headers; provider file signed off row by row before paid launch; no live calls in tests |
| PRIV-08 Retention jobs completion | Verify/build the sweep and expiry jobs for media intermediates (24-72 h), export archives (7 days), webhook raw bodies (30 days), telemetry age purge (30 days); each bounded, audited by counts, controlled-clock tested | `crates/df-persistence/src/*sweep*`, media migrations `0014` follow-ups, `crates/df-telemetry/src/persistence.rs`, B retention pass for inbox bodies | O2 + B + S2 (telemetry owner) | Per-job test: aged fixture rows/objects removed, live rows untouched, audit counts match |
| PRIV-09 Breach + security record | Add the personal-data breach runbook and the one-page security record; add breach-register template; fail-closed staging qualification items | `planning/service-operations.md` runbook sections (S2-owned), `deploy/qualification/staging-checks.csv` | S2 | Tabletop drill record: timed awareness-to-decision exercise with named roles; checklist evidence attached to the launch record |
| PRIV-10 Age + intake ops | Neutral age attestation copy, under-age discovery procedure (suspend, delete, record), `privacy@` mailbox runbook (log, verify, deadline tracking, refusal/appeal templates) | Terms text via PRIV-01; procedure in `planning/service-operations.md`; mailbox is owner-only (section 13) | S2 + Site | Runbook exists; drill request answered inside the stated deadline in staging with synthetic data |

Not build tasks (do not dispatch): any cookie banner (section 9 says none is
needed for the current footprint), likeness/biometric features (section 3
gate), new analytics, or OAuth integration tokens (the
`integration-api-and-embeds-research.md` PAT design needs its own privacy
pass — token scopes expose email/session metadata and must inherit sections
5 and 6 before build).

---

## 13. Owner and counsel checklist

Owner-only actions (no agent can do these):
1. Name the seller entity, address, support and privacy mailboxes
   (`support@` exists in intent; create `privacy@dungeonfluxdnd.com`);
   publish identity in terms/privacy/footers (blocks PRIV-01, CAN-SPAM
   footer, Stripe onboarding).
2. Decide the served map: EU/UK in (with representatives) or geo-restricted
   at launch; same decision, explicitly, for CA/AU/BR (section 1). Confirm
   the real DigitalOcean region (`nyc1` in the request vs `nyc3` in config).
3. If the EU/UK is served: appoint the EU representative and the UK
   representative in writing before the first EU/UK account; appoint a DPO
   only if counsel advises (not automatically required at this scale —
   confirm).
4. Sign/record DPAs and transfer mechanisms (DPF or SCCs) for WorkOS,
   Stripe, DigitalOcean, OpenAI, ElevenLabs, fal, BFL, World Labs, Jev and
   the email provider once selected; enable zero-retention/no-training
   settings on each AI account and file the confirmations (section 4),
   including fal no-store behavior and ElevenLabs voiceprint/retention
   answers in writing.
5. Select the production email provider; publish SPF/DKIM/DMARC for
   `mail.dungeonfluxdnd.com`.
6. Keep likeness off at launch, or fund the section 3 biometric package
   (consent, BIPA retention policy, fal agreement, counsel clearance,
   possible Illinois exclusion) as a separate go/no-go decision.
7. Approve the final retention schedule, the financial-records period per
   jurisdiction (with the accountant), and the breach runbook roles with
   named people and a 72-hour contact path.
8. Run the WorkOS sandbox gates and Google sign-in setup (client, consent
   screen, exact callbacks) from `google-sign-in-research.md` sections 6
   and 9.

Counsel checklist (in dependency order):
1. Confirm the applicable-law map and thresholds (GDPR/UK GDPR, ePrivacy/
   PECR, CCPA/CPRA and other state laws, COPPA, PIPEDA, Australia, LGPD)
   and the controller/processor roles (sections 1, 4).
2. Rule on Art. 27 representatives vs geo-restriction, and on the transfer
   mechanism per processor (section 4).
3. Approve lawful bases, the Art. 30 record, sign-up/mic/upload/share
   notice texts and the unbundled marketing consent (sections 2, 10, 11).
4. Set the age-assurance posture for an 18+ voice service and the
   under-age discovery procedure (section 3); clear or exclude the
   likeness feature under GDPR Art. 9 and BIPA/CUBI/Washington law.
5. Approve the retention schedule, financial retention periods, erasure
   exceptions for shared campaign history, and backup-ageing disclosure
   (sections 5, 6).
6. Approve breach-notification mechanics for a non-EU controller (which
   authorities, via the representative, 72-hour clock) and US state breach
   triggers (section 7); confirm whether the DPIA residual risk needs
   prior consultation for any feature (section 8).
7. Confirm the no-banner cookie position and the Google Fonts fix, and
   the marketing regime per country (consent vs soft opt-in vs CAN-SPAM)
   (sections 9, 10).
8. Replace all five `docs/legal/*` drafts with approved texts as one launch
   gate, with the named sub-processor list and seller identity (PRIV-01).





