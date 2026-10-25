# DungeonFlux legal risk review

> **This report is not legal advice.** It is a research summary for the owner to
> take to qualified counsel. Laws, licences and provider terms change; every
> item marked "verify with counsel / verify current terms" needs a current
> primary-source check before money is taken.

Date: 2026-10-25. Scope: repository at `C:\Users\mreca\Desktop\dungeonfluxGame`
(HEAD `6dd065d8`), the public site in `docs/`, planning docs, `content/`, and
`crates/df-providers/src/`. No code was changed for this review. External
sources consulted: CC BY 4.0 legal code and the D&D Beyond SRD page (fetched
2026-10-25); all other external terms are cited from the repo's own dated
research notes and marked for re-verification.

---

## How to read this report

- **Severity** is a triage signal for the owner, not a legal conclusion.
  - *High*: could block launch, force a rebrand, close the payment
    account, or create regulator/plaintiff exposure if shipped as-is.
  - *Medium*: needs a decision, document or build task before paid
    launch; unlikely to block a free, invite-only pilot on its own.
  - *Low*: keep doing what the repo already does; monitor on change.
- **Where** gives the exact file and line (or document section) an
  owner or counsel can open. Line numbers are from the checkout above
  and will drift as files change; the section heading is the stable
  reference.
- **Why it matters** states the legal or commercial mechanism in one
  sentence: what duty, licence condition or contract term is engaged.
- **Mitigation** is the smallest concrete next step found in the repo's
  own plans where one exists. Several mitigations are already adopted
  as launch decisions and only need enforcement, not invention.
- **Verify?** marks every claim that could not be confirmed from a
  primary source in this session. Treat each as an open question for
  counsel or for a current-terms check, not as a settled fact.
- Findings describe the product *as planned and as built so far*.
  Much of the money, identity and hosted stack is designed but not
  yet live (`planning/master-launch-plan.md:46-51`). A "High" finding
  about an unbuilt feature is a gate to clear before building it for
  real money, not an accusation about running behaviour.
- Nothing in this report clears the product for launch. The checklist
  at the end is the list of decisions and documents that would have
  to exist before counsel could give that clearance.
- Suggested use: (1) owner reads the one-page summary and marks any
  risk they dispute; (2) counsel works the "Verify?" column and the
  12-item checklist; (3) each accepted mitigation becomes a launch
  task with an owner and a gate (before accounts, before money, or
  before wider release); (4) this file is re-issued when the price
  version, provider set or rules denominator changes.
- Retention: keep this review with the launch records. It records
  what was known, what was checked and what was left unverified on
  the date above, which matters if a decision is later questioned.

---

## One-page summary: top 10 risks

| # | Risk | Severity | Where it arises | First action |
| --- | --- | --- | --- | --- |
| 1 | Full 2024 rules ambition exceeds SRD rights; no RightsGrant for PHB/DMG/MM content | High | `planning/rules-support.md:53-55`; `development/rules-sources/manifest.json` (`full_2024_rules_coverage: false`, books `unacquired`) | Freeze launch scope to SRD 5.2.1 subset; treat any non-SRD catalog as a paid-launch blocker until a written licence exists |
| 2 | Domain `dungeonfluxdnd.com` embeds the "D&D" mark; site leads with "AI Dungeon Master" | High | `docs/CNAME:1`; `docs/index.html:6,11,68,72`; `artifacts/tmp/agents/LAUNCH.md:139` (decision 32) | Get trademark counsel opinion before launch spend; prepare a fallback name/domain; keep "not affiliated" notices |
| 3 | Prepaid credit + multi-payer "pot" may be treated as stored value / money transmission and breaches Stripe terms if unapproved | High | `planning/master-launch-plan.md` §5.1 (lines 218-226), §6; `docs/legal/credits/index.html:94-104` | Obtain written Stripe + counsel approval for the exact flow before building checkout; keep the fixed-price fallback ready |
| 4 | Face-photo likeness feature would process biometric data; current fal adapter retains inputs 90 days by default | High | `planning/face-portrait-capture-research.md:12,23,132-167`; `crates/df-providers/src/fal_image.rs` (no `X-Fal-Store-IO` header) | Do not ship likeness until consent, retention and a fal data agreement are signed; set no-store headers for all person references |
| 5 | AI narration/voices/images are generated live; no guard stops a model reproducing non-SRD text, voices or likenesses | High | `crates/df-tools/src/gameplay/storyteller.rs:108-130` (facts-only rule, no SRD-only guard); `crates/df-providers/src/` | Add SRD-only content constraints, output filtering and provider terms review per route before paid launch |
| 6 | No root `LICENSE` file although manifests declare MIT; public GitHub repo linked from every page | Medium | `Cargo.toml` (`license = "MIT"`); `README.md:220-225`; `planning/dependency-license-policy.md:23` | Decide the project licence deliberately; add the file or change declarations; do not assume MIT is granted today |
| 7 | Dependency licence inventory is not release-clean (Unicode-3.0, Unlicense, BSL branches, vendored `h2` provenance) | Medium | `planning/dependency-license-policy.md:21-27`; `crates/df-rpc-bridge/vendor/h2/`; `Cargo.lock` (377 packages, see §3) | Complete the SPDX/text/notice audit per target (native + WASM) and record the `h2` upstream diff before distribution |
| 8 | Privacy draft does not yet name the AI providers that receive table speech/text, and retention figures conflict across docs | Medium | `docs/legal/privacy/index.html:98` ("to be completed"); `planning/erasure-retention-policy.md:92-107`; `docs/pricing/index.html:231-245` | Publish a named sub-processor list with regions/retention; reconcile retention periods into one counsel-approved schedule |
| 9 | Adults-only service rests on a checkbox; voice, mature themes and possible minors in photos raise COPPA/age-assurance exposure | Medium | `docs/legal/privacy/index.html:107-108`; `planning/google-sign-in-research.md:34,147`; `crates/df-tools/src/gameplay/storyteller.rs:134-141` | Counsel to set the age gate, attestation wording and failure handling; keep likeness/voice off for unverified accounts |
| 10 | Marketing pages promise `$10` credit and account creation while sign-up/pricing are unapproved drafts | Medium | `docs/index.html:74,168-174,256-257`; `docs/pricing/index.html:63-64` (`noindex`, "not approved") | Keep `noindex`/draft banners until the offer is real; align homepage CTAs with actual availability |

---

## 1. D&D rules and IP

External basis: SRD 5.2.1 is offered under CC BY 4.0 (see
`development/rules-sources/manifest.json`, SHA-256
`8974902d…5e3d87`, and the publisher page https://www.dndbeyond.com/srd).
CC BY 4.0 §3(a) requires attribution, a licence link, and an indication of
changes. The repo's attribution line follows the SRD's own suggested wording.
Whether any given monster/spell/class text in the repo is actually inside
SRD 5.2.1 must be checked entry-by-entry against the pinned PDF; this review
did not re-read all 364 pages. *Verify with counsel / verify current terms*
for the SRD's current version and wording.

| Finding | Where | Why it matters | Severity | Mitigation | Verify? |
| --- | --- | --- | --- | --- | --- |
| Attribution is present and consistent on the site | Footer of every `docs/**/index.html` (e.g. `docs/index.html:285`, `docs/rules/index.html:74`); `content/hero-tables/sources.json:19` | CC BY 4.0 §3(a) attribution is the core SRD condition; the line names WotC, the SRD, the source URL and the CC BY 4.0 legal-code URL | Low | Keep verbatim on every surface that ships SRD material, including the WASM app and exports, not only the marketing site | Verify current SRD attribution wording at launch |
| Attribution may not travel with the game client, PDFs or exports | No attribution found in `crates/` UI strings in this review; exports described in `planning/master-launch-plan.md` §13.11 (lines 861-867) | CC BY 4.0 follows the material, not the website; a client or export without attribution breaches the licence even if the site footer is correct | Medium | Put the attribution in the app (About/credits), character sheets and export packages; add a release checklist item | No |
| Launch scope is honestly labelled a subset today | `docs/rules/index.html:92-168` (Built / Partly built / Not built); `docs/index.html:133-134` | Accurate subset disclosure defeats most consumer and licence complaints about missing spells/classes | Low | Keep the page synced to the tested build; never advertise "full 2024 rules" (plan §19, lines 1121-1125) | No |
| The required end-state is full 2024 coverage, which the SRD does not grant | `planning/rules-support.md:8-17,53-55`; `planning/rules-coverage.md:25-34`; `development/rules-sources/manifest.json` scope block | PHB/DMG/Monster Manual text, most subclasses, spells and monsters are outside the SRD; personal book ownership or model memory is not a commercial right (plan's own words) | High | Hold the line in `planning/commercial-validation.md:17-19`: no non-SRD entry ships without a recorded RightsGrant; keep the SRD-only subset as the launch denominator | Verify with counsel what (if any) licensing channel exists |
| Grep for classic product-identity names found almost nothing | Searched `content/`, `crates/`, `docs/`, `planning/` for beholder, mind flayer/illithid, Forgotten Realms/Faerûn, Waterdeep, Baldur's Gate, Strahd, Vecna, Mordenkainen, Bigby, Drizzt, Elminster, displacer beast, Tiamat, Bahamut: **no hits**. Only hit: `owlbear` in `crates/df-rules/src/srd_creatures.rs:354` and tactical code | Product-identity creatures/settings are the highest-risk SRD exclusions; their absence is good. Owlbear needs a page-level check because it is widely treated as open in the SRD tradition, but that was not confirmed against the pinned PDF in this review | Low (owlbear: Medium until checked) | Confirm owlbear (and each of the 17 creatures in `srd_creatures.rs:56`) against SRD 5.2.1 pages cited in the file (e.g. pp.261-296) and record the locator per entry | Verify owlbear + full 17-creature list against the pinned PDF |
| SRD catalog transcription is documented but not independently proven complete | `content/hero-tables/README.md:26-33` (page locators pp.9-99); `content/hero-tables/catalog-srd.json` (12 classes, 1 subclass each) | Transcription errors or silent additions (e.g. a non-SRD subclass name) would turn "SRD" data into unlicensed data; the README itself notes one check "needs the licensed reference or a reviewer" (line 151) | Medium | Keep per-entry source locators; sample-audit the catalog against the PDF before paid launch; keep `custom-v1` content tagged as original (`sources.json:31-57`) | Verify sample entries with counsel/reviewer |
| Custom backgrounds could accidentally mirror non-SRD published backgrounds | `content/hero-tables/backgrounds-custom-v1.json`; guard described in `content/hero-tables/README.md:150-151` | A re-skinned PHB background is still derivative even if numbers use the SRD template | Medium | Run the combination check (name/skill/feat/tool) against a licensed reference under counsel; keep the "not copied or paraphrased" record (`sources.json:42`) | Verify with counsel |
| No OGL 1.0a material found | Repo-wide search for "Open Game License / OGL 1.0" returned no hits | OGL 1.0a carries its own Section 15 attribution and product-identity terms; absence avoids that stack entirely | Low | Keep it that way; if any OGL content is ever imported, record it as a separate source tag | No |
| Model-generated narration has no SRD-only guard | `crates/df-tools/src/gameplay/storyteller.rs:108-130` constrains output to supplied FACTS and bans inventing named people/places, but nothing limits the model to SRD vocabulary or blocks memorised book text; prompts invoke "an experienced Dungeon Master" (line 108, 149 in `expression.rs`) | A model can emit non-SRD spell text, named settings or trademarked characters from training data; the facts-only rule reduces but does not eliminate regurgitation | High | Add an SRD allow-list/denylist post-filter for high-risk names and verbatim text, log hits, and brief counsel on the residual risk; keep rules resolution in server code (already the design, `planning/rules-support.md:39-41`) | Verify provider output-rights terms (§4) |
| Adventure tables claim originality; not audited line-by-line | `planning/adventure-generator.md:5` ("All table prose is original; nothing is copied from the Dungeon Master's Guide"); `content/adventure-tables/*.json` | The claim is the right control, but a single copied table (e.g. trinkets, names) would undermine it | Low | Spot-check tables against published books; keep author records for new tables | No |

## 2. Trademarks

External basis: "Dungeons & Dragons", "D&D" and "Dungeon Master" are Wizards
of the Coast marks; the SRD's CC licence does not license trademarks (stated
in `docs/legal/content/index.html:104` and plan §14.3, line 891). WotC's
Fan Content Policy permits limited non-commercial fan use and does not cover
a paid product. *Verify with counsel / verify current terms* — this review
did not fetch the current Fan Content Policy text.

| Finding | Where | Why it matters | Severity | Mitigation | Verify? |
| --- | --- | --- | --- | --- | --- |
| Domain embeds "dnd": `dungeonfluxdnd.com` | `docs/CNAME:1`; `artifacts/tmp/agents/LAUNCH.md:139-144` (decision 32, owner-owned, GoDaddy) | A domain containing "D&D"/"DnD" for a paid D&D-adjacent service is the classic implied-endorsement fact pattern; nominative fair use is weakest in domains and branding | High | Trademark counsel opinion before launch marketing; budget for a rename/rebrand; do not print the domain on merchandise/ads until cleared | Verify with counsel |
| Homepage leads with "AI Dungeon Master" | `<title>` and OG/Twitter titles `docs/index.html:6,11,20`; hero `docs/index.html:67-72`; repeated in app (`crates/df-account-app/src/view/auth.rs:39`, `crates/df-tools/src/gameplay_browser/live_motion.rs:52`) | "Dungeon Master" is both a WotC mark and arguably generic for the GM role; using it as the product headline ("an AI Dungeon Master for your living room") heightens endorsement risk beyond body-copy mentions | High | Counsel to rule on headline vs body use; safer phrasing is "AI game master" / "compatible with fifth-edition rules"; the repo already flags this (`docs/legal/content/index.html:106`) | Verify with counsel |
| Body copy uses "Dungeon Master" pervasively as the narrator label | Dozens of hits across `crates/df-tools/src/`, `crates/df-render/src/`, planning docs (§1 grep) | Descriptive use ("the Dungeon Master narrates") is more defensible than branding use, but volume + headline use blur the line | Medium | Keep descriptive uses if counsel agrees; change product-facing titles, store listings and ads first | Verify with counsel |
| "Is this official Dungeons & Dragons? No." plus non-affiliation footers | `docs/index.html:244-245`; footers e.g. `docs/index.html:285` | Express disclaimer is the single best mitigation and matches SRD attribution practice | Low | Keep on home, pricing, rules and app store copy; add to the WASM app itself | No |
| "Compatible with fifth-edition rules" style phrasing already in use | `docs/index.html:7` ("fifth-edition fantasy adventure"); `docs/rules/index.html:61` | This is the safer industry formulation and avoids the marks in most copy | Low | Prefer this phrasing in titles/metadata; reserve mark mentions for the attribution/disclaimer blocks | No |
| SEO/metadata still carry the mark in titles | `docs/index.html:6,11,20` (title/OG/Twitter) | Search/social titles are consumer-facing branding, not nomination | Medium | If counsel advises, retitle to "DungeonFlux: an AI game master for your living room" and keep marks out of `og:title` | Verify with counsel |
| "DungeonFlux" itself not cleared | No clearance search exists in the repo; `docs/legal/content/index.html:106` lists "a separate review of the product name and domain" as open | An identical or confusing game/software mark would force a late rename; this review cannot search registers | Medium | Commission a clearance search (USPTO/EUIPO + common-law) before paid launch; treat this as a note only here | Verify with counsel (clearance search) |
| GitHub repo name and README use the game name publicly | Footer link `https://github.com/monstercameron/dungeonfluxGame` on every page (e.g. `docs/index.html:275`) | Public source distribution amplifies any trademark/licence defect and preserves history | Low | Keep the non-affiliation line in `README.md` (present at lines 168-169); resolve the licence file (§3) | No |

## 3. Third-party libraries and bundled material

Method: `cargo metadata --format-version 1 --offline` was run with the repo's
pinned `CARGO_HOME`/`RUSTUP_HOME` on 2026-10-25 and exited 0; it resolved
**377 packages** (50 workspace members, all declaring MIT). Full JSON is in
`artifacts/tmp/lanes/LEGAL/metadata.json`. Declarations below are
package-supplied metadata, not licence-text proof — the repo's own policy
(`planning/dependency-license-policy.md:13`) rightly refuses to treat
metadata as approval.

| Finding | Where | Why it matters | Severity | Mitigation | Verify? |
| --- | --- | --- | --- | --- | --- |
| No standalone GPL/AGPL/MPL package in the locked graph | `Cargo.lock`; metadata summary: 170× `MIT OR Apache-2.0`, 108× MIT, remainder permissive options (see working file) | No strong-copyleft obligation currently attaches to the hosted service or the shipped WASM bundles | Low | Re-run this inventory at every lockfile change; block new copyleft at admission (policy lines 11-15) | No |
| Two `MIT OR Apache-2.0 OR LGPL-2.1-or-later` expressions (`r-efi` 5.3.0 and 6.0.0) | `Cargo.lock` / metadata | LGPL would oblige source/notice/relinking duties if that branch were ever selected or construed as chosen; selecting MIT/Apache avoids it only if recorded | Medium | Record the selected permissive branch per package in the release inventory, exactly as `planning/dependency-license-policy.md:25` requires; never ship LGPL-branch code | No |
| 18 packages declare only `Unicode-3.0` (ICU/zerovec family, `unicode-ident` cumulative term) | Metadata: `icu_*`, `zerovec*`, `tinystr`, `yoke`, etc. | Unicode-3.0 is permissive but has its own notice terms and is on the repo's "unresolved until individually added" list (policy line 15) | Medium | Add Unicode-3.0 by documented decision with its notice text, or replace the deps; do not silently normalise | No |
| `Unlicense OR MIT` (5), `Unlicense/MIT` legacy slash forms, BSL-1.0 branches (`ryu`, `wasite`, `whoami`), `CDLA-Permissive-2.0` (1) | Metadata summary in `artifacts/tmp/lanes/LEGAL/` | Slash forms are not valid SPDX and BSL/Unlicense need the individual review the policy demands (policy line 25) | Medium | Normalise only by reading the actual licence files; record branch choices; keep HOLD status until done | No |
| Vendored `h2 0.4.19` provenance is explicitly HOLD | `[patch.crates-io]` in `Cargo.toml:28-29`; `crates/df-rpc-bridge/vendor/h2/LICENSE` (MIT, 2017 h2 authors); `planning/dependency-license-policy.md:21` | The code is MIT-licensed, but the upstream revision, vendoring date and local patch diff are unrecorded; an unreviewed patch could contain non-upstream material | Medium | Diff the vendored tree against upstream `hyperium/h2` v0.4.19, record the result, keep its LICENSE in distributed artifacts | No |
| No root `LICENSE` file; public repo implication | `README.md:220-225` admits the gap; every `docs/` footer links the public GitHub repo | The world can read the source but has no licence to use it; conversely the owner has not accidentally granted MIT — but the manifest declaration (`Cargo.toml`: `license = "MIT"`) creates ambiguity a court or contributor could argue over | Medium | Owner decision (already listed open in `artifacts/tmp/agents/LAUNCH.md:69`): add a deliberate LICENSE (MIT, proprietary, or source-available) and align `Cargo.toml` | Verify with counsel |
| Ported PlayCanvas code is noticed correctly | `THIRD_PARTY_NOTICES.md:1-58` (MIT text reproduced, engine v2.23.2, file-level mapping) | MIT requires the copyright/permission notice in copies; the port is substantial, so this file must ship with binaries/source releases | Low | Ship `THIRD_PARTY_NOTICES.md` with every distribution, including WASM bundles' about page | No |
| Two landscape scans have **no licence** | `THIRD_PARTY_NOTICES.md:103-106` (`scan-riverbed`, `scan-valley-bridges`, `scan-summit` — "used for private local testing only") | Any leak into a build, screenshot, trailer or the public site would be unlicensed use of third-party 3D captures | Medium | Keep them out of release scene rotation (already stated), CI-check release manifests for those IDs, never use in marketing | No |
| One scan is CC BY 4.0 and attributed | `THIRD_PARTY_NOTICES.md:94-101` ("Osada Slavotovo", Andrii Shramko) | Attribution must travel with the scene where it is shown | Low | Surface the credit in-app where `scan-village` is playable | No |
| Site fonts are Google Fonts (Spectral, Barlow Semi Condensed), loaded from Google's CDN | `<link>` in every `docs/**/index.html` head (e.g. `docs/index.html:33-35`); families defined in `docs/styles.css:19-20` | Both families are open-licensed (OFL) as far as general knowledge goes, but the repo records no font licence/notice, and CDN loading sends visitor IPs to Google (privacy, §7) | Low | Record the font licences in third-party notices; consider self-hosting the WOFF2 files to remove the third-party request | Verify current font licences |
| No npm/JS application code found | No `package.json` in the app tree (only `development/dependency-audit/packages.json`); `docs/README.md:7` ("this site contains no authored JavaScript") | Keeps the JS licence surface at zero, consistent with the Rust-only rule in `AGENTS.md` | Low | Keep generated WASM glue notices with the bundle | No |
| WASM bundles are "distribution" for licence purposes | Browser clients ship compiled deps to every visitor (architecture in `planning/client-architecture.md`) | MIT/Apache/BSD notices must accompany distributed copies even when the copy is a `.wasm` file; a hosted-service (SaaS) reading does not cover the client bundle | Medium | Add a Notices/About view in the app listing all shipped crates with licence texts; generate it from the frozen WASM inventory | No |

## 4. AI and media providers

Routes found in code: OpenAI (text, image edit, embeddings, vision judges),
ElevenLabs (TTS/STT/music/SFX), fal (FLUX images, Kling/Wan video, SAM
segmentation, BiRefNet), Black Forest Labs FLUX 3 (direct), World Labs
Marble, TypeSafe "Jev" (text), plus WorkOS (identity), Stripe (payments) and
DigitalOcean (hosting) in the plan. Evidence: `crates/df-providers/src/lib.rs:86-168`,
`content/pricing.json:5-49`, `crates/df-providers/src/registry.rs:5-12`.
Provider marketing claims ("Commercial use" tags) are not terms reviews —
the repo says so itself (`planning/sam-3-1-research.md:45-46`,
`crates/df-providers/src/lib.rs:1-4`).

| Finding | Where | Why it matters | Severity | Mitigation | Verify? |
| --- | --- | --- | --- | --- | --- |
| No provider route has a completed rights/terms qualification | `crates/df-providers/src/registry.rs` Gate model (AccountEntitlement, RightsApproval…); `planning/asset-provider-research.md:29-32` ("not permission to integrate") | Sending paid user content to a provider whose terms bar the use (or claim rights in outputs) creates breach and IP exposure at scale | High | Complete a per-route terms record (commercial output rights, training use, retention, sub-processors, attribution) before paid launch; block dispatch on `RightsApproval` | Verify current terms for every provider |
| fal retains request payloads 90 days by default; adapter sends no no-store header | `planning/face-portrait-capture-research.md:23,157-168`; `crates/df-providers/src/fal_image.rs` (data-URI references, `sync_mode`) | Player photos, reference images and prompts sit on a third party for 90 days; the privacy draft cannot promise otherwise | High | Send `X-Fal-Store-IO: 0` (+ object lifecycle headers) for all person/reference inputs; get fal's enterprise data terms in writing | Verify current fal retention/terms |
| ElevenLabs music terms may exclude this exact use | `planning/asset-provider-research.md:123-134` (model-specific music terms "exclude self-serve Studio Games", unresolved for multi-device browser distribution) | Cached generated music in a monetised game could breach the music licence even though the API allows the call | Medium | Written clarification or Enterprise Music agreement before music ships; otherwise use licensed stock stems (already the default row, line 25) | Verify current ElevenLabs music terms |
| SAM 3/3.1 licence is a custom Meta licence, not OSI; hosted use governed by fal/Meta terms | `planning/sam-3-1-research.md:31-46` | The SAM License restricts certain end uses and can terminate on suit; "does not address commercial use in words". A tabletop game is not a listed prohibited use, per the note — but that is not counsel's opinion | Medium | Counsel to confirm commercial hosted segmentation; prefer the hosted route (no weights redistribution) until then | Verify with counsel |
| FLUX image models (fal + BFL direct) output/likeness terms unreviewed | `crates/df-providers/src/fal_image.rs:1` (owner's FLUX-only rule); `crates/df-providers/src/bfl_image.rs:18`; `content/pricing.json:39-46` | FLUX model licences differ by variant (dev/schnell vs pro); commercial and likeness rights cannot be assumed from API access | Medium | Record the exact model licence per endpoint (fal-hosted vs BFL API) before selling generated portraits | Verify current BFL/fal model terms |
| Kling video via fal: output rights and retention unreviewed | `crates/df-providers/src/fal_media.rs:138` (`KLING_O1_IMAGE_TO_VIDEO`); `planning/character-clip-research.md:114-117` | Cinematic clips are a paid upsell (plan §4.4); unclear output ownership would undermine selling them | Medium | Same per-route terms record; confirm commercial use of Kling outputs through fal | Verify current terms |
| World Labs Marble terms "saved but not read" | `planning/worldlabs-splat-worlds-research.md:368,488` | Generated 3D worlds are core scenery; unknown redistribution/derivative rights block shipping them | Medium | Read the saved terms + current ToS; confirm the right to ship generated worlds in a paid game | Verify current World Labs terms |
| Player speech/text is sent to OpenAI for narration and embeddings | `planning/story-memory-vector-search-research.md:28-29,302-306` (`text-embedding-3-small`, canonical sentences embedded); `content/pricing.json:31` | Table conversation is personal data (and story IP); OpenAI's API data-use/retention terms decide whether it trains models or is retained | Medium | Confirm API (not consumer) terms apply, zero-retention eligibility, and name OpenAI in the privacy policy (§7) | Verify current OpenAI API terms |
| Voice: ElevenLabs TTS/STT processes player microphone audio | `crates/df-providers/src/speech_adapter.rs`; plan §14.4 (line 897) | Voice is biometric-adjacent; voice-cloning misuse and retention are provider-policy issues, and the content draft bans unauthorised voice copying (`docs/legal/content/index.html:96`) | Medium | Keep cloning off at launch; disclose STT/TTS providers; confirm no voiceprint retention; enforce the ban in product, not only policy | Verify current ElevenLabs terms |
| "Jev" (TypeSafe) text route has thin public terms/pricing evidence | `content/pricing.json:48` ("TypeSafe publishes no pricing page, rate from the launch coverage and gateway docs") | A core narration/judgment dependency without verified commercial terms is a supply and rights risk | Medium | Get written terms from the vendor before it carries paid traffic | Verify with vendor/counsel |
| Pricing config contains unverified "OWNER TO CONFIRM" rates | `content/pricing.json:26,41,46` | Billing customers from unconfirmed tariffs risks under/over-charging disputes, though customer prices are pace-capped (plan §4) | Low | Confirm or replace those rows before the price version is published | No |
| Provider failure must never become in-world speech — already a rule | `artifacts/tmp/agents/LAUNCH.md:98-100` (decision 21); plan §4.4 (line 214) | Prevents deceptive "the DM says you can't afford that" patterns and keeps failure handling honest for consumer law (§8) | Low | Keep; test the notice path | No |

## 5. Player content and likeness

| Finding | Where | Why it matters | Severity | Mitigation | Verify? |
| --- | --- | --- | --- | --- | --- |
| Face/portrait likeness is researched, not built — the right time to gate it | `planning/face-portrait-capture-research.md:4` ("light prototype. No provider call was made"); launch decision 13 (`artifacts/tmp/agents/LAUNCH.md:81-82`: uploaded real-person photos not used as generation references at launch) | Shipping later without the §6 controls would create biometric liability; the current launch posture avoids it | Low | Keep likeness out of launch scope; if revived, implement the research §6 consent/retention design first | No |
| If built as designed, the flow still creates face-geometry data at fal (PuLID) | `planning/face-portrait-capture-research.md:10,100-102,132-136` | Illinois BIPA (written consent, published retention, no sale), Texas/Washington analogues, and GDPR Art. 9 explicit consent are all triggered by face-geometry processing — the research correctly identifies this | High | BIPA/GDPR-compliant written consent, retention schedule and destruction plan before any live call; signed fal data agreement; consider excluding Illinois or geofencing until compliant | Verify with counsel (BIPA/GDPR) |
| The similarity test itself is biometric processing | `planning/face-portrait-capture-research.md:120-122` (ArcFace-style scoring on team photos) | Even internal testing needs consent and deletion discipline | Low | Consenting staff only, scores-only retention (already proposed); document it | No |
| Voice recordings: raw audio "not kept by default" is a promise that must match STT provider behaviour | `docs/legal/privacy/index.html:99-100`; `docs/pricing/index.html:246`; plan §13.8 (line 826) | If the STT provider retains audio (training, abuse review), the promise is false even if DungeonFlux deletes its copy | Medium | Verify each STT route's retention; align the promise to the strictest provider in the chain | Verify current provider terms |
| Generated images resembling real people beyond the likeness feature | Plate prompts replace `NO_REAL_PEOPLE` only for consented likenesses (`planning/face-portrait-capture-research.md:21,231`); storyteller rule 8 bans sexual content about real people (`crates/df-tools/src/gameplay/storyteller.rs:126-130`) | A model can still render a celebrity-like face from a name prompt; right-of-publicity and provider policies both bite | Medium | Keep name-based likeness prompts blocked; keep the `based_on_real_person` safeguards; add a report/takedown path (§8) | No |
| Uploaded portraits pass a content screen; flagged items go to operator review | `artifacts/tmp/agents/LAUNCH.md:81-82` (decision 13); plan §13.4 (lines 776-784) | Uploads are the main vector for infringing or unlawful images entering the service | Low | Keep quarantine → screen → promote; log the permission basis per upload (plan §13.3, line 766) | No |
| Adults-only rests on self-attestation | `planning/google-sign-in-research.md:34,147` (18+ attestation + terms version persisted); `docs/legal/terms/index.html:95`; mature themes at `crates/df-tools/src/gameplay/storyteller.rs:134-141` | A checkbox is weak age assurance for a mature, voice-enabled, paid service; if minors slip in, COPPA (US) and GDPR Art. 8 (EU) duties arise that the service is not built for | Medium | Counsel to set proportionate age assurance for the pilot cohort (invite-only helps); keep payment-verified-email gating for funding/hosting (plan §9.3, line 466) | Verify with counsel |
| Minor-photo refusal relies on appearance, not analysis — correct but fragile | `planning/face-portrait-capture-research.md:150-155` | Estimating age from a face would add biometric processing; refusing child-appearing photos without analysis is the lesser risk, but needs a human-review fallback | Low | Keep no-age-estimation rule; route edge cases to refusal, not analysis | No |

## 6. Money

Model in one line: users buy USD service credit ($10/$20/$50), allocate it to
a session "pot" with others, and the table spends it on time + metered AI
(plan §5-6; `docs/legal/credits/index.html:93-104`). The plan is unusually
candid that labels do not change legal substance (line 24) and that Stripe's
terms restrict stored-value use of its credits feature (line 222).

| Finding | Where | Why it matters | Severity | Mitigation | Verify? |
| --- | --- | --- | --- | --- | --- |
| Reusable prepaid credit may be "stored value" under Stripe's restricted-business rules and state money-transmission law | Plan §5.1 (lines 218-226) citing Stripe SSA service terms + restricted businesses [S6-S7]; `docs/pricing/index.html:261` repeats the gate | If Stripe or a regulator treats the balance as stored value, the account can be closed and the model reclassified; a custom ledger does not exempt it (plan's own warning) | High | Written Stripe approval for the exact diagrams (plan §5.1) + counsel memo on money transmission before any real-money build; ship the fixed-price session fallback if refused | Verify current Stripe terms; counsel |
| Group pot pools several people's money for one service | Plan §5.3-5.5 (lines 246-275); `docs/legal/credits/index.html:97-100` | Pooling is safe only while every dollar stays a purchase from the single seller with no transfer/withdrawal (plan §5.1, line 224); any future "pay the host" or cash-out feature changes the analysis entirely | High | Keep: no Connect, no host payouts, no transfers, unused allocations return to the payer's own credit (line 265); counsel to confirm this stays outside money transmission in launch states | Verify with counsel |
| Expiry of purchased credit is undecided | `docs/legal/credits/index.html:106`; `docs/legal/refunds/index.html:109`; plan §6.7 (lines 358-366) | Many US states restrict expiry/fees on stored-value-like products and treat long-dormant balances as unclaimed property (escheat); the pilot proposes "refundable without an arbitrary expiration fee" but no dormancy process | Medium | Decide expiry (prefer none for purchased credit), dormancy notices and escheat handling per launch state before selling | Verify with counsel (state gift-card/escheat law) |
| Promotional credit ($10 trial) is correctly separated | Plan §6.5 (line 346); `docs/legal/credits/index.html:101-102`; LAUNCH decision 3 | Promo credit with no cash value, non-transferable and non-convertible avoids most stored-value/gift-card duties for that class | Low | Keep the classes ledger-separate forever; never let promo become refundable | No |
| Refunds go to the original payment method; disputes freeze only the disputed lot | Plan §6.7 (lines 360-366), §5.5 (line 275); `docs/legal/refunds/index.html:94-107` | Matches Stripe's refund model [S13] and protects other contributors; chargeback losses stay with the company | Low | Keep; publish realistic refund timing once measured | Verify current Stripe refund terms |
| Tax/VAT treatment is an open decision, and contributors may differ | Plan §6.7 (line 366), §4.2 (line 184: caps include "applicable consumption-time tax"); `docs/pricing/index.html:222` ("Prices exclude applicable taxes") | Digital services are taxable in many US states and in the EU/UK at the customer's location; charging at funding vs consumption changes the answer | High | Tax counsel decision (sourcing, product classification, registration) before launch; limit launch jurisdictions to reviewed ones (plan line 366) | Verify with counsel/accountant |
| Auto-reload is deferred and consent-gated | Plan §5.6 (lines 277-281); `docs/legal/credits/index.html:104` | Saved-card recurring charges without explicit consent are a top source of complaints, disputes and state auto-renewal claims | Low | Keep deferred; if shipped, implement the separate-consent design exactly | Verify current Stripe saved-payment terms |
| Customer-facing prices hide the markup — by design | LAUNCH decision 26 (`artifacts/tmp/agents/LAUNCH.md:117-119`): customers see pace/estimate/max/actual, "Never the table fee, the multiplier or provider prices" | Lawful if the total price and caps are clear (they are, §8), but counsel should confirm no drip-pricing or disclosure rule requires the split | Low | Keep totals prominent; record counsel's view | Verify with counsel |
| "Do not fund company expenses with unearned customer obligations" is stated as treasury policy | Plan §6.1 (line 295) | Spending prepaid balances before earning them creates refund-ability and, in some regimes, trust/segregation issues | Medium | Accountant to set recognition + liquidity policy; keep the ledger's liability classes distinct (§6.5) | Verify with accountant/counsel |

## 7. Privacy and accounts

| Finding | Where | Why it matters | Severity | Mitigation | Verify? |
| --- | --- | --- | --- | --- | --- |
| Privacy draft does not name the AI/infrastructure providers | `docs/legal/privacy/index.html:97-98` ("will be listed here by name", TODO line 98) | Table speech, text, images and embeddings go to OpenAI/ElevenLabs/fal/etc. (§4); an unnamed sub-processor list is the draft's biggest gap for GDPR Art. 13/14 and state privacy laws | Medium | Complete the list from production config before launch (already the stated TODO); include regions, purposes and retention per provider | Verify provider regions/terms at launch |
| Draft retention figures differ from the erasure policy | `docs/legal/privacy/index.html:103-104` (backups 7-day PITR + 30-day daily) vs `planning/erasure-retention-policy.md:95-107` (classes incl. campaign-lifetime + 30-day deletion window, backup candidate ≤30 days) and `docs/pricing/index.html:244` (rejected output 24-72 h) | Inconsistent promises are unenforceable and mislead users; erasure must also reach derived indexes/summaries/caches (erasure policy lines 71-76) | Medium | One counsel-approved retention schedule; make the site, the policy doc and the deletion jobs match | No |
| Erasure design is strong on paper, unbuilt in code | `planning/erasure-retention-policy.md:13-39,275-286` (suppression overlay, tombstones; "no `df-persistence`/`df-assets` crate or `DeletionPlan` type" in that checkout) | GDPR/CCPA deletion rights need a working workflow, not a policy model; backups resurrecting deleted data is the classic failure (plan §13.10, line 855) | Medium | Build the DeletionPlan workflow before EU/UK or California launch; test restore-through-overlay | Verify applicable law with counsel |
| WorkOS/Google sign-in data is minimised by design | `planning/google-sign-in-research.md:26,123-129` (no Google tokens stored; WorkOS subject is the link key); LAUNCH decision 28 (server-side sessions, `__Host-` HttpOnly cookie) | Avoids token leakage through the WASM client; Google sign-in still shares identity data with Google/WorkOS and must be disclosed | Low | Disclose Google/WorkOS sharing in the privacy policy (research §9.1 already requires it); complete the six sandbox gates (research §6) before production | Verify WorkOS/Google current terms |
| Account export must exclude other players' private data | Plan §13.11 (line 863); acceptance test FILE-08/AUTH-03 (lines 1054,1067: "Test must fail until filtering is correct") | A campaign export naturally contains other people's characters, notes and speech; over-export breaches their privacy and the draft promise (`docs/legal/privacy/index.html:106`) | Medium | Keep audience-filtered export; treat FILE-08 as a launch-blocking test | No |
| The marketing site makes third-party requests (Google Fonts) with no cookie/consent discussion | Font `<link>`s in every `docs/` page head; privacy open item "Cookie and local storage disclosures" (`docs/legal/privacy/index.html:112`) | Fonts via Google CDN disclose visitor IPs to Google; several EU decisions treat this as a transfer needing a basis. The static site sets no cookies itself (no JS found, §3) | Low | Self-host fonts, or document the transfer and its basis; add cookie disclosure for the app (session/CSRF cookies are strictly necessary) | Verify with counsel if EU users are admitted |
| "We do not claim end-to-end encryption" — honest and necessary | `docs/legal/privacy/index.html:93`; plan §14.4 (line 897) | The server and AI providers read content to run the game; claiming otherwise would be deceptive | Low | Keep verbatim | No |
| Telemetry must not become a shadow copy of private play | Plan §16.1 (line 981: log IDs/prices "without logging private prompt contents by default"); erasure policy lines 101,120-123 | OTEL/SQLite telemetry (AGENTS.md observability rules) can silently retain the very text deletion is meant to remove | Medium | Keep prompt bodies out of telemetry; include telemetry stores in every DeletionPlan scope list | No |

## 8. Site and marketing

| Finding | Where | Why it matters | Severity | Mitigation | Verify? |
| --- | --- | --- | --- | --- | --- |
| Pricing is labelled "proposed", `noindex`, and "you cannot buy credit yet" | `docs/pricing/index.html:8,63-64,82`; plan §19 (line 1125) | Clear "not yet available" framing defeats most drip-pricing/bait claims; the worked examples are ceilings, not promises (lines 148,196: "Invented figures for illustration") | Low | Keep until the owner approves a numbered price version with an effective date (page already has the slot, lines 266-279) | No |
| Homepage sells the trial and account creation ahead of availability — mostly hedged | `docs/index.html:74,168-174,256-257` ("once sign-up opens", "when sign-up opens") vs CTA buttons "Create a free account" linking `/app/sign-up` (lines 53,73,257) | If the app route is live before the offer is real, the hedge fails; consumer law judges the overall impression, not the footnote | Medium | Confirm what `/app/sign-up` does today; if signup is not open, the CTA should say so at the destination, not only in body copy | No |
| Rules claims are carefully subsetted | `docs/rules/index.html:92-168`; homepage "Real SRD rules, still growing" (`docs/index.html:133`) | Avoids the "full D&D" overclaim; the internal goal (full 2024 coverage) is not marketed | Low | Keep marketing and the rules page locked to the tested build | No |
| "AI Dungeon Master" headline is the main marketing/trademark exposure | `docs/index.html:6,68,72,118` (see §2) | Same fact as §2, viewed as advertising: titles and ads get less nominative-use protection than body text | High | See §2 mitigation | Verify with counsel |
| Images are labelled concept/early-access design where it matters | `docs/index.html:111,130` ("Screens from our early-access design. The live game may differ"); footer line 285 ("Scenes shown are concept art") | AI-generated marketing art presented as gameplay would mislead; the labels prevent that | Low | Keep labels on any image not captured from the shipping build; check OG images (`docs/assets/og-*.jpg`) carry no implied gameplay claim when shared | No |
| OG/social images are generated art of unknown per-image provenance | `docs/assets/og-default.jpg` etc.; generation routes in §4 | If a generated OG image accidentally resembles a real person or a trademarked character, it becomes the most-shared asset | Low | Human-review OG/hero art for likeness/trademark resemblance before launch; record the generating route per asset | No |
| News/changelog make factual build claims | `docs/news/2026-10-24-*/index.html`; `docs/changelog/index.html:119` ("More SRD foes…") | Build claims ("17 SRD creatures" etc.) must stay true as the catalog changes, or they become stale advertising | Low | Date-stamp claims (already done) and re-check at launch | No |
| Legal pages are marked drafts and `noindex` — and say they do not bind anyone | e.g. `docs/legal/terms/index.html:66-72`, identical banners on privacy/refunds/credits/content | Correct interim posture; the risk is only if accounts/money launch while the drafts are still "not a contract" | Medium | Hard launch gate: counsel-approved terms/privacy/refund/credit/content texts replace the drafts before the first real account or payment | No |
| Business identity is missing everywhere it is legally needed | Footer TODO (`docs/index.html:134` pattern; plan §9.1 item 9, §14.3 line 889: seller entity, address, support) | Terms, receipts, privacy notices and Stripe onboarding all need the legal seller; `support@dungeonfluxdnd.com` exists in structured data (`docs/index.html:24`) but no entity/address | Medium | Owner to form/name the seller entity and publish identity + support address before Stripe application | No |

## 9. Anything else legally dubious

| Finding | Where | Why it matters | Severity | Mitigation | Verify? |
| --- | --- | --- | --- | --- | --- |
| Local `.env` holds eight live-looking provider keys; correctly **not** committed | `.env` in the working tree (key names only inspected: OpenAI, ElevenLabs, fal, Jev, World Labs, BFL); `.gitignore` excludes `.env*`; `git ls-tree HEAD` shows no `.env`; no `.env` in git history | Committed keys would be a breach and a billing emergency; the current state is safe but fragile (one `git add -A` mistake) | Low | Keep the ignore rules; add a pre-commit secret scan; rotate any key ever pasted into chat/logs (plan §16.4 secret-compromise runbook, line 1021) | No |
| The old project is a reference archive, and the site was migrated from it | `AGENTS.md` (dungeonflux.old is read-only reference, not spec); `docs/README.md:3-7` (21 WebP assets preserved from `dungeonflux.old/website/`) | Migrated assets carry the old project's provenance; if any old asset was itself unlicensed, the new site inherits the problem | Low | Spot-check the 21 migrated assets' origin; the dice demo was already replaced (line 6) | No |
| Battle-scene scan data with no licence could ship by accident | `THIRD_PARTY_NOTICES.md:103-106`; scene JSONs in `content/battle-scenes/scan-*.json` | See §3: configuration, not bytes, lives in the repo, but a release could bundle the local scan directory | Medium | Release-manifest check for the three unlicensed scan IDs | No |
| `development/rules-sources/srd-5.2.1-en.pdf` (6 MB) is stored in the repo tree | `development/rules-sources/manifest.json` (`file`, `size_bytes: 6031375`) | Redistributing the SRD PDF itself is permitted under CC BY 4.0 with attribution, but the manifest should travel with it and the repo licence gap (§3) muddies the repo's own terms | Low | Keep the manifest beside the PDF (already done); confirm the PDF is excluded from public release artifacts if the owner prefers link-only distribution | Verify with counsel if the repo is public |
| Mature content + user-generated stories create moderation duties the drafts leave open | Plan §14.2 (line 885), §14.4 (line 901); `docs/legal/content/index.html:101-102` (report/takedown/appeal "still to be defined"); storyteller mature themes `crates/df-tools/src/gameplay/storyteller.rs:134-141` | A paid, voice-enabled UGC service needs report/takedown/appeal and law-enforcement processes before launch, not after the first incident; payment/AI providers also impose content limits (plan line 894) | Medium | Write and staff the procedures pre-launch; align the content profile to the strictest provider in the chain | Verify provider content policies at launch |
| No DMCA/agent or copyright-complaint route exists yet | Absent from `docs/legal/*` and plan §14 (only "content takedown" generally, line 901) | User uploads + AI outputs will draw copyright complaints; a designated-agent/notice process (US) is cheap insurance and supports safe-harbor arguments | Medium | Add a copyright notice-and-takedown page + agent registration before uploads open | Verify with counsel |
| Accessibility and consumer UX duties are planned, not evidenced | Plan §11 (lines 612: keyboard, captions, reduced motion, screen-reader finance text) | Disability-discrimination exposure (ADA/Equality Act/EAA depending on market) grows once the service is paid and public | Low | Keep §11 as launch acceptance criteria with a tested build, per ADR 0005 evaluation rules | Verify applicable duties with counsel |

---

## Owner and counsel checklist (before taking real money)

Decisions and documents needed, in dependency order:

1. **Seller identity.** Legal entity, address, statement descriptor, support
   contact; publish in terms/privacy/footers (plan §14.3).
2. **Trademark opinion.** `dungeonfluxdnd.com` + "DungeonFlux" clearance
   search; ruling on "Dungeon Master" in titles vs body copy; fallback brand
   ready (§2).
3. **Rules scope freeze.** Launch denominator = SRD 5.2.1 subset; per-entry
   source locators audited (incl. owlbear and the 17 creatures); non-SRD
   content stays behind a recorded RightsGrant (§1).
4. **Payment structure approval.** Written Stripe approval for prepaid credit
   + group pots, or adoption of the fixed-price fallback (plan §5.1).
5. **Money-law memo.** Money transmission / stored-value analysis for launch
   states; expiry, dormancy and escheat handling for purchased credit (§6).
6. **Tax decision.** Product classification, sourcing (funding vs
   consumption), registration footprint; launch jurisdictions limited
   accordingly (plan §6.7).
7. **Final consumer documents.** Counsel-approved terms, privacy, refunds,
   credits and content policies replacing the `docs/legal/*` drafts,
   including the named sub-processor list and one retention schedule (§7-8).
8. **Age and content boundary.** Age-assurance method for the adult pilot,
   mature-content profile aligned to payment/AI provider limits, and
   report/takedown/appeal + copyright-complaint procedures (§5, §9).
9. **Provider terms file.** Per-route record for OpenAI, ElevenLabs, fal,
   BFL, World Labs, TypeSafe/Jev, WorkOS, Stripe, DigitalOcean: commercial
   output rights, training use, retention, regions, attribution (§4).
10. **Biometric gate.** Likeness stays out of launch, or BIPA/GDPR consent,
    retention and fal data agreements are completed first (§5).
11. **Licence hygiene.** Root LICENSE decision; dependency inventory signed
    off per target (incl. Unicode-3.0, Unlicense, BSL branches, `r-efi`
    branch selection, vendored `h2` diff); notices view shipped in the WASM
    app (§3).
12. **Privacy operations.** Working export/deletion workflows passing
    FILE-08/AUTH-03; telemetry excluded from private-content retention;
    restore-through-overlay tested (§7).

## Safe to keep (checked and found fine)

- **SRD attribution wording and placement on the site.** It follows the
  SRD's suggested form and appears on every page footer plus the rules and
  content pages (§1).
- **No product-identity names in the shipped corpus.** The grep for
  beholder/mind flayer/named settings and similar returned no hits in
  `content/`, `crates/`, `docs/` or `planning/` (§1).
- **No OGL 1.0a material.** No Open Game License text or Section 15
  attribution stack exists in the repo (§1).
- **Honest subset marketing.** The rules page enumerates Built / Partly
  built / Not built and the homepage says "still growing" (§8).
- **Non-affiliation disclaimers.** "Not affiliated with or endorsed by
  Wizards of the Coast" appears in footers and the homepage FAQ (§2).
- **No GPL/AGPL/MPL in the locked dependency graph.** 377 packages resolve
  to permissive licences, subject to the branch/notice work in §3.
- **PlayCanvas port notices.** `THIRD_PARTY_NOTICES.md` reproduces the MIT
  text with a file-level provenance map (§3).
- **Secrets hygiene today.** `.env` is ignored and absent from HEAD and
  history; provider keys were not printed during this review (§9).
- **Money architecture choices.** Original-payment-method refunds,
  dispute isolation per funding lot, promo/purchased separation, deferred
  auto-reload, and "labels do not change legal substance" candour (plan §5-6).
- **Privacy honesty.** No end-to-end-encryption claim; microphone opt-in;
  export excludes other players' private data by design (§7).
- **Draft labelling.** Legal pages and pricing are marked drafts, dated
  24 October 2026, and `noindex` until approved (§8).

---

## Appendix A: searches actually run (2026-10-25)

All searches used the repo's bounded search over the named subtrees.
Hits below are the complete result sets at the stated caps, not samples.

### A.1 Product-identity and setting names

Pattern (case-insensitive): beholder, mind flayer, illithid,
Forgotten Realms, Faerûn, Waterdeep, Baldur's Gate, Strahd, Vecna,
Mordenkainen, Bigby, Drizzt, Elminster, displacer beast, Tiamat,
Bahamut, gelatinous cube.

- `content/`: no hits.
- `crates/`: no hits for the pattern as a whole.
- `docs/`: no hits.
- `planning/`: no hits in the capped result set.
- Sole related hit in the wider run: `owlbear` —
  `crates/df-rules/src/srd_creatures.rs:354` (creature id),
  `:792,827,834,841` (tests/fixtures), plus tactical mirrors in
  `crates/df-rules/src/tactical/state.rs:93,465,1561`,
  `crates/df-tools/src/gameplay/fight_witness.rs:346,861` and
  `crates/df-tools/src/gameplay/journey/tactical.rs:991,3946`.
  Disposition: verify against the pinned SRD PDF (§1).

### A.2 Trademark phrases

Pattern: "Dungeon Master".

- `docs/index.html:6,11,20` — page/OG/Twitter titles.
- `docs/index.html:67,68,72,107,118,174,216` — hero and body branding.
- `docs/how-it-works/index.html:128,152` — body copy and image alt text.
- `docs/news/2026-10-24-building-in-the-open/index.html:74` — body copy.
- `docs/legal/content/index.html:106` — the repo's own open item on the term.
- Code mirrors include `crates/df-account-app/src/view/auth.rs:39`,
  `crates/df-tools/src/gameplay_browser/live_motion.rs:52` (`NARRATOR_NAME`),
  `crates/df-protocol/proto/gameplay.proto:136` (comment) and the
  storyteller prompts in §1. Full hit list is in the lane working notes.

Pattern: "Open Game License" / "OGL 1.0" — no hits repo-wide.

### A.3 SRD attribution occurrences

The full attribution sentence (SRD 5.2.1 + WotC + dndbeyond.com/srd +
CC BY 4.0 legal-code link + non-affiliation) appears in the footer of
every `docs/**/index.html`, in `docs/rules/index.html:74`,
`docs/legal/content/index.html:104`, and in structured form in
`content/hero-tables/sources.json:19` and
`development/rules-sources/manifest.json`. No equivalent string was
found in the WASM app crates in this review (§1, row 2).

## Appendix B: cargo licence inventory (method and full counts)

Command (offline, repo-pinned homes, exit 0, 2026-10-25):

```text
CARGO_HOME=.../artifacts/cache/cargo
RUSTUP_HOME=.../artifacts/cache/rustup
CARGO_TARGET_DIR=.../artifacts/build/agents
cargo metadata --format-version 1 --offline
```

Output retained at `artifacts/tmp/lanes/LEGAL/metadata.json`
(377 packages; 50 workspace members, each declaring MIT).
Declared-licence counts (package metadata, not text proof):

| Count | Declared licence expression |
| ---: | --- |
| 170 | MIT OR Apache-2.0 |
| 108 | MIT |
| 18 | Unicode-3.0 |
| 17 | Apache-2.0 OR MIT |
| 10 | MIT/Apache-2.0 (legacy slash form) |
| 9 | Apache-2.0 |
| 5 | Unlicense OR MIT |
| 5 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| 3 | Apache-2.0 OR ISC OR MIT |
| 3 | MIT OR Apache-2.0 OR Zlib |
| 2 | Zlib OR Apache-2.0 OR MIT |
| 2 | Zlib |
| 2 | MIT OR Zlib OR Apache-2.0 |
| 2 | BSD-3-Clause OR Apache-2.0 |
| 2 | MIT OR Apache-2.0 OR LGPL-2.1-or-later (`r-efi`) |
| 2 | ISC |
| 2 | Unlicense/MIT (legacy slash form) |
| 2 | Apache-2.0 OR BSL-1.0 OR MIT |
| 2 | BSD-2-Clause OR Apache-2.0 OR MIT |
| 1 each | 0BSD/MIT/Apache-2.0; ISC AND (Apache-2.0 OR ISC); one large cumulative ISC/Apache/MIT/BSD expression; CC0-1.0 OR MIT-0 OR Apache-2.0; Apache-2.0 / MIT; MIT AND BSD-3-Clause; Apache-2.0 AND ISC; Apache-2.0 OR BSL-1.0; BSD-3-Clause; (MIT OR Apache-2.0) AND Unicode-3.0; CDLA-Permissive-2.0 |

No package declared GPL, AGPL or MPL in any branch position except the
two unselected LGPL alternatives above. No package used a `license-file`
field. Platform-filtered (native vs WASM) inventories were **not**
re-run in this lane; the repo's earlier snapshot
(`planning/dependency-license-policy.md:25`) found 153 native and 143
WASM records at its pinned revision, and both closures must be frozen
per release profile before the notices view in §3 can be generated.

Vendored code: exactly one vendored crate,
`crates/df-rpc-bridge/vendor/h2` (h2 0.4.19, MIT LICENSE present,
`Cargo.toml:28-29` patch). No vendored JS. No `package.json` application
tree. Fonts: Spectral + Barlow Semi Condensed via Google Fonts CDN
(`docs/styles.css:19-20`); licences unrecorded in-repo (verify).

## Appendix C: provider data-flow inventory (what leaves the server)

Derived from `crates/df-providers/src/lib.rs`, the adapter modules and
`content/pricing.json`. "Verify" means the current provider terms must
be checked for that row before paid launch.

| Provider / route | Data sent | Repo evidence | Terms status |
| --- | --- | --- | --- |
| OpenAI text (Responses) | Table text, story facts, prompts | `registry.rs:10`; `content/pricing.json:15-31` | Verify API data-use/retention |
| OpenAI image edit | Prompts + reference images | `lib.rs:122-128`; `pricing.json:5-14` | Verify output rights |
| OpenAI embeddings | Canonical story sentences | `lib.rs:114-120`; §4 | Verify retention/training |
| OpenAI vision (head locate, plate judge) | Scene plate images | `lib.rs:108-112,162-168` | Verify |
| ElevenLabs TTS/STT | Narration text; player microphone audio | `speech_adapter.rs`; `pricing.json:32-34` | Verify voice/retention terms |
| ElevenLabs music/SFX | Text prompts | `lib.rs:140-146`; `pricing.json:35-36` | Music terms unresolved (§4) |
| fal FLUX (Ultra, Kontext) | Prompts + reference images as data URIs | `fal_image.rs:28-32` | 90-day default retention; verify |
| fal Kling/Wan video | First/last frame images + prompt | `fal_media.rs:37-39,138` | Verify output rights |
| fal SAM 3/3.1 | Plate image + noun phrase | `fal_segment.rs:42-53` | Custom Meta licence (§4) |
| fal BiRefNet | Images for background removal | `fal_image.rs:32` | Verify |
| BFL FLUX 3 (direct) | Prompts | `bfl_image.rs:18` | Verify model licence |
| World Labs Marble | Image/text world prompts | `lib.rs:155-160` | Terms saved, unread (§4) |
| TypeSafe Jev | Text for judgments | `pricing.json:48` | No public pricing page; verify |
| WorkOS (+ Google) | Email, credentials, identity events | §7; `google-sign-in-research.md` | Verify tenant terms in sandbox |
| Stripe | Payer identity, amounts (no card data on our servers) | Plan §5.2 | Approval gate (§6) |
| DigitalOcean | All hosted data at rest/in transit | Plan §12 | Verify DPAs/regions at signup |

## Appendix D: documents and drafts relied on

- Business/money: `planning/master-launch-plan.md` §1-7, §12-14, §17-19;
  `artifacts/tmp/agents/LAUNCH.md` decisions 1, 3, 13, 21, 26, 32.
- Site drafts (all "Draft for review by counsel. Not yet in effect.",
  dated 24 October 2026, `noindex`): `docs/legal/terms/index.html`,
  `docs/legal/privacy/index.html`, `docs/legal/refunds/index.html`,
  `docs/legal/credits/index.html`, `docs/legal/content/index.html`;
  plus `docs/pricing/index.html` and `docs/rules/index.html`.
- Rules: `planning/rules-support.md`, `planning/rules-coverage.md`,
  `development/rules-sources/manifest.json`,
  `content/hero-tables/sources.json`, `content/hero-tables/README.md`,
  `crates/df-rules/src/srd_creatures.rs`.
- Providers: `planning/asset-provider-research.md`,
  `planning/face-portrait-capture-research.md`,
  `planning/character-clip-research.md`,
  `planning/video-loop-provider-research.md`,
  `planning/worldlabs-splat-worlds-research.md`,
  `planning/sam-3-1-research.md`,
  `planning/story-memory-vector-search-research.md`,
  `planning/google-sign-in-research.md`, `content/pricing.json`.
- Licences/privacy: `planning/dependency-license-policy.md`,
  `planning/erasure-retention-policy.md`, `THIRD_PARTY_NOTICES.md`.
- Fetched externally for this review (2026-10-25): CC BY 4.0 legal code
  (creativecommons.org/licenses/by/4.0/legalcode) and the D&D Beyond SRD
  landing page (dndbeyond.com/srd). No accounts were created, no APIs
  were called, and no data was sent anywhere.
