# Game refinement

Started: 2026-10-23. This is the ongoing record of game refinement discussions and decisions. Begin with launching and entering the game. Record confirmed decisions separately from current implementation observations, discussion candidates, and open questions. Discussion alone does not approve implementation.

## Confirmed decisions

### REF-001 — Keep a refinement decision record

- Date: 2026-10-23
- Decision: Use this document to store decisions as the game refinement discussion proceeds, starting with how to launch the game.
- Rationale: Keep the agreed direction available alongside the project design documents.
- Open follow-ups: Clarify the launch experience to refine; record subsequent choices as they are agreed.

### REF-002 — Refine the local PC launcher

- Date: 2026-10-23
- Decision: Focus first on the local launcher: starting, stopping, and resuming the game on this PC.
- Rationale: The user selected this scope for the first refinement discussion.
- Open follow-ups: Decide how the launcher is invoked and how its controls and state are presented.

### REF-003 — Open a launcher window from a desktop shortcut

- Date: 2026-10-23
- Decision: A desktop shortcut opens a local launcher window.
- Rationale: The user selected a desktop shortcut and launcher window as the invocation method.
- Open follow-ups: Decide browser-opening behavior after starting or resuming; define the window's controls and state presentation separately.

### REF-004 — Open the game display automatically when ready

- Date: 2026-10-23
- Decision: After starting or resuming the game, automatically open the shared game display in the default browser only once the server is ready.
- Rationale: The user selected automatic opening of the game display.
- Open follow-ups: Decide what closing the launcher does while the game is running.

### REF-005 — Keep the game running until Stop

- Date: 2026-10-23
- Decision: Closing the launcher window leaves the running game active. An explicit Stop action ends the game.
- Rationale: The user selected keeping the game running until Stop.
- Open follow-ups: Decide how reopening the launcher connects to an existing game; tray UI remains undecided.

### REF-006 — Show the existing running game when reopening

- Date: 2026-10-23
- Decision: Reopening the launcher detects and reconnects to the existing local running game, shows that game with Open game display and Stop actions, and avoids starting a duplicate server for it.
- Rationale: The user selected showing the running game when reopening the launcher.
- Open follow-ups: Define Stop behavior and handling of stopping or save failures.

### REF-007 — Stop gracefully and retain progress for Resume

- Date: 2026-10-23
- Decision: Stop shuts the game down gracefully, keeps its progress available for Resume, and leaves the launcher open.
- Rationale: The user confirmed this Stop behavior.
- Open follow-ups: Define saved-game selection for Resume and exact shutdown/save-failure handling.

### REF-008 — Choose from saved games for Resume

- Date: 2026-10-23
- Decision: Resume presents a selectable list of saved games with their names and last-played dates.
- Rationale: The user selected choosing from saved games rather than resuming only the most recent game.
- Open follow-ups: Decide how new games receive names; exact list presentation remains undecided.

### REF-009 — Name new games automatically without asking

- Date: 2026-10-23
- Decision: A new saved game receives an automatic name without asking the user for a name. There is no editable naming prompt or required custom name.
- Rationale: The user explicitly selected “Name it automatically without asking.”
- Open follow-ups: Define the naming algorithm; whether games can be renamed later remains undecided.

### REF-010 — Allow same-network phones automatically

- Date: 2026-10-23
- Decision: Starting or resuming automatically allows phones on the same network to join. Normal launch does not require a separate opt-in toggle for this LAN access.
- Rationale: The user selected allowing same-network phones automatically.
- Open follow-ups: Decide where joining information is shown. This decision covers LAN access only; internet hosting remains outside this decision.

### REF-011 — Keep join details on the game display only

- Date: 2026-10-23
- Decision: Joining information stays on the game display only. The launcher does not show a room code or join QR.
- Rationale: The user selected keeping join details on the game display only.
- Open follow-ups: Define startup status and failure presentation in the launcher.

### REF-012 — Show startup progress and errors in the launcher

- Date: 2026-10-23
- Decision: Show visible preparation status and progress in the launcher. Open the game display automatically after readiness as agreed in REF-004. On startup failure, show an actionable error with Retry in the launcher. The normal launch flow requires no terminal.
- Rationale: The user confirmed startup progress and errors in the launcher.
- Open follow-ups: Define detailed status/error presentation, shutdown/save-failure behavior, and packaging, build, and update behavior.

### REF-013 — Allow up to six human players

- Date: 2026-10-23
- Decision: The maximum is six human players per game. The game's DM role does not consume a player seat.
- Rationale: The user selected opening the player count to six players.
- Open follow-ups: Minimum player count, default party size, companions, and encounter-balancing policy remain undecided. Expanding the current prototype requires implementation and verification.

### REF-014 — Sign in and offer minimal new-game setup

- Date: 2026-10-23
- Decision: Double-clicking the executable or its shortcut opens a local window supporting account sign-in. After sign-in, offer minimal new-game setup covering player count (maximum six human players), ruleset modifiers, and story length using units such as hours, days, weeks, and months.
- Rationale: The user described this launch and setup sequence and requested planning its settings.
- Open follow-ups: Sign-in provider, session retention, recovery/offline behavior, whether player accounts are required, player-count minimum/default, available modifiers/defaults, and story-length ranges remain undecided; REF-015 resolves its meaning. This does not require fresh sign-in on every launch or select automatic duration enforcement.

### REF-015 — Story length means real-world campaign span

- Date: 2026-10-23
- Decision: Story length in hours, days, weeks, or months means the real-world campaign span across play sessions, rather than accumulated active play hours or elapsed time inside the story world.
- Rationale: The user confirmed the real-world campaign-span interpretation.
- Open follow-ups: REF-016 resolves the span as a flexible target. Decide when its clock starts and how session duration/frequency relate to it. No hard expiration, schedule, frequency, session duration, or automatic enforcement has been selected.

### REF-016 — Treat campaign span as a flexible target

- Date: 2026-10-23
- Decision: The real-world campaign span is a flexible pacing target that can extend when needed, rather than a firm expiration date.
- Rationale: The user selected “Flexible target, extend when needed.”
- Open follow-ups: Decide whether optional expected session duration and play frequency should inform pacing estimates, and when the span begins. No automatic ending or deletion is selected.

### REF-017 — Offer Continue adventures or Create new campaign after login

- Date: 2026-10-23
- Decision: After desktop launch and account login, offer Continue adventures and Create new campaign.
- Rationale: The user confirmed these post-login choices while opening discussion of interesting story generation.
- Open follow-ups: Define campaign-generation behavior. This decision does not approve a generation algorithm, authentication/payment provider, or optional setup extras.

### REF-018 — Join by phone browser before collaborative questions

- Date: 2026-10-23
- Decision: Players connect to the game through their phone browsers before answering collaborative campaign questions. No native phone app is required. A pre-campaign join lobby is available before narrative generation.
- Rationale: The user clarified that phone connection precedes the questionnaire.
- Open follow-ups: Guest versus account joining, draft/lobby persistence and recovery, question participation/completion rules, seed-review authority, and the broader choice of seed modes remain undecided. This does not require every player to answer or hold an account.

### REF-019 — Simple and advanced character appearance

- Date: 2026-10-23
- Decision: Provide thematic D&D character appearance creation in two modes: simple toggles/main categories such as species (the user's “race”), gender, build, and hair; and advanced detailed textual descriptions of body features.
- Rationale: The user identified character creation as a major gap and requested both approaches.
- Open follow-ups: Exact categories/options, defaults, content support, preview behavior, and ordering with mechanical creation. Appearance does not grant strength, stats, equipment, or other mechanics.

### REF-020 — Optional own-face character likeness

- Date: 2026-10-23
- Decision: A player may optionally capture their own face using a device camera/webcam and integrate that likeness into their character.
- Rationale: The user requested optional own-face integration.
- Open follow-ups: Consent, capture compatibility, likeness generation, retention/deletion, and provider selection. Face input is not account authentication and does not infer gender, ethnicity, abilities, or stats.

### REF-021 — Optional free account for persistent characters

- Date: 2026-10-23
- Decision: Offer an optional free player account to persist a character and use it in other sessions. An account is not required for guest character creation.
- Rationale: The user requested free optional player accounts and character portability.
- Open follow-ups: Identity provider, guest claiming, campaign compatibility, and which mechanics transfer across campaigns. No provider or payment policy beyond the free player-account requirement is selected.

### REF-022 — Welcome and introduce the story at play start

- Date: 2026-10-23
- Decision: After character creation, play starts with the narrator giving a warm welcome and story introduction.
- Rationale: The user requires this welcome and introduction at the start of play.
- Open follow-ups: Intro pacing, accessible presentation/skip/replay, and resumed-session welcome/recap behavior.

### REF-023 — Explore scenes with changing generated views

- Date: 2026-10-23
- Decision: Players move from scene to scene and change what they are looking at through generated imagery.
- Rationale: The user described scene exploration and generated visual changes as the play experience.
- Open follow-ups: View controls, cost/latency bounds, generation/fallback timing, and supported navigation. This does not select instant unlimited generation or physical 3D navigation; changing viewpoint alone cannot rewrite canonical state.

### REF-024 — Tempo-directed music and audio

- Date: 2026-10-23
- Decision: The tempo engine tempers music and audio to the permitted scene context during play.
- Rationale: The user requested tempo-engine control of the auditory experience.
- Open follow-ups: Detailed profiles, audio preferences, and tuning; no hidden facts may leak through cues.

### REF-025 — Server tracks rules/state and saves session progress

- Date: 2026-10-23
- Decision: The server tracks authoritative rules, character stats, and game state, and saves state at session end for continuation.
- Rationale: The user requested server-managed mechanics/state and an end-of-session save.
- Open follow-ups: Save confirmation, shutdown failure/recovery, and checkpoint details. Narration cannot invent mechanics or become the sole stored memory.

### REF-026 — Custom character sheet, portraits and visual reference pack

- Date: 2026-10-23
- Decision: Focus character creation on player-customized characters. Produce the character sheet and portraits, plus a visual reference sheet/pack used to keep the character consistent in gameplay display plates.
- Rationale: The user wants each custom hero's accepted identity to carry into the shared story imagery rather than relying on fixed demo characters.
- Open follow-ups: Exact sheet/portrait/reference layouts, generation and approval order, persistence and failure policy. Proposed front/profile/back/detail reference views below are a design recommendation, not a separately confirmed layout. Mechanical statistics remain derived and validated by the server, not generated as authoritative image text.

### REF-027 — Disable generated video for now

- Date: 2026-10-23
- Decision: Disable video generation for now to reduce cost; retain still portraits, character reference imagery and scene plates.
- Rationale: The user explicitly prioritized cost and asked to turn off generated video.
- Open follow-ups: Implementation must cover home, scene/kill and character-action clips, not only the lobby. Any future explicit development opt-in must be deliberate; merely possessing a provider key must not enable generation. Cached playback and static visual motion are distinct from requesting new provider video.

### REF-028 — Prefer OpenAI for character imagery

- Date: 2026-10-23
- Decision: Prefer OpenAI's image provider for the custom-character and still-image workflow. This supersedes the older FLUX-only preference in historical research/implementation comments; photographic fantasy continuity remains the visual direction.
- Rationale: The user explicitly selected this provider preference.
- Open follow-ups: The configured image model already defaults to OpenAI's image model; the existing automatic provider route can nevertheless select FLUX when its key exists. Align the actual route with this preference, preserve truthful explicit development overrides, and verify output quality. This does not authorize unrestricted live generation, choose a new model revision, or claim provider capabilities have been tested here.

### REF-029 — Include all races/species and classes in settings

- Date: 2026-10-23
- Decision: The user wants all races/species and classes available in campaign/creation settings, starting with the 2024 core Player's Handbook catalog: 10 species and 12 classes. The user explicitly selected “2024 core catalog first.”
- Rationale: Custom creation should not be confined to the prototype's Dwarf Fighter choices.
- Open follow-ups: The initial catalog scope is now selected as the 2024 core Player's Handbook set (10 species and 12 classes listed below). Expansion books and homebrew are not selected for this initial scope. This records the requested breadth, not an implementation claim. Catalog visibility, host enablement, rights/source admission and actual playable mechanical support must be distinct; unsupported mechanics cannot become playable through a toggle.

### REF-030 — Players roll and assign their own character statistics

- Date: 2026-10-23
- Decision: Character creation is interactive and player-owned, rather than a pre-generated roster or fixed build packages as the only path. Generate six scores by rolling four d6 and totaling the highest three, then let the player assign those scores to abilities. Offer an explicit reroll of the whole six-score set before confirmation.
- Rationale: The user explicitly requested rolling and rerolling characters instead of pre-generated characters. SRD 5.2.1 p. 21 supplies the random-generation method.
- Open follow-ups: The server generates and persists the dice, draft revision and assignment; exact operation retries return the original result. Reject fabricated scores, stale assignments and rerolls after finalization. Existing array/point-buy alternatives may remain, but cannot replace the requested roll flow. Full level-1 source-qualified species/class creation precedes the requested review of ten distinct saved characters; this does not claim complete class gameplay effects.

### REF-031 — Fully custom appearance with an independent optional look roll

- Date: 2026-10-23
- Decision: Keep simple appearance controls alongside lossless detailed public text for body and facial features. Players can optionally roll or reroll a look, then edit it. Cosmetic rolls are independent of statistic rolls, species, class and backstory; they preserve the player's custom text. A player's approved public description and identity revision persist and become visual-reference inputs for portraits and character reference packs.
- Rationale: The user explicitly reinforced custom looks beyond statistics. Closed menu IDs and phrase matching alone cannot express an individual character.
- Open follow-ups: Preserve bounded Unicode and multiline text exactly through save/resume, escape it when rendering, and treat it as data in visual prompts. Public appearance never grants mechanics or includes sealed backstory/raw selfies automatically. Stale asynchronous suggestions cannot overwrite newer edits. Rolling a look or changing controls must not call paid image/video providers; generating and approving new portrait/reference assets remains an explicit separate step.

## Active play and session close — proposed flow

The four requirements above are confirmed; implementation details below remain proposals consistent with [Narrative engine](narrative-engine.md), [Tempo engine](tempo-engine.md), [Runtime reliability](runtime-reliability.md), [Storage architecture](storage-architecture.md), and [Campaign cinematics](campaign-cinematics.md).

| Stage | Proposed behavior |
| --- | --- |
| Start | When characters are ready, a proposed host Start action begins the warm welcome and story introduction, followed by an establishing scene. Exact start authority/readiness policy remains open. |
| Welcome | Warm, accessible narration with captions and optional audible voice, matched to campaign tone rather than a rigid script. Proposed skip/replay controls do not alter state. Muted or failed audio does not block play. |
| Input and outcome | Players act; server rules validate and resolve outcomes; narrative and presentation express permitted committed facts. Neither narration nor generated imagery authors a die result, new rule, or secret disclosure. |
| Explore | Scene/viewpoint requests reference canonical locations, cast, time, and audience. Preserve photographic/live-action continuity; keep the current usable view until a validated replacement is ready. Generation failure uses an honest fallback, not fabricated scene changes. |
| Sound | Tempo shapes audience-safe music/audio with captions, volume/mute preferences, and nonblocking failure behavior. Presentation intensity does not change rules or disclose hidden danger. |
| Continue | Maintain durable committed decisions and canonical state throughout play, with a clean checkpoint at session end. This is the recommended persistence approach, not a newly selected save implementation. |
| End | Existing Stop gracefully closes play and leaves the launcher open. Propose reporting saved/ended success only after the durable checkpoint succeeds; expose failure honestly and preserve recovery options. |
| Resume | Proposed warm welcome plus audience-safe recap rather than automatically repeating the complete original introduction. Replay of the original intro can be optional; resumed narration behavior is not yet selected. |

### Persistence and stop details — proposal

Recommend saving committed decisions continuously so a failure near session end does not discard an entire session, then producing a coherent end checkpoint. Server-owned durable snapshots/decision records should include HP/resources, inventory, positions, conditions, turn/reaction state, elapsed game clocks, faction/NPC truth, per-observer knowledge/secrets, narrative threads, asset references, stored seeds and rules/content revisions, accepted jobs, and account/access permissions through their owning records. AI prose summaries alone cannot restore canonical truth.

For Stop, propose rejecting new decisions, finishing or safely fencing bounded in-flight commands, and checkpointing the coherent state before success. Pending reactions and accepted paid jobs need durable recovery identities so resumption does not reroll outcomes, duplicate generation, or repeat unknown paid calls. A save failure must not be presented as success. Exact drain/cancel deadlines, process shutdown policy, and failure controls remain open.

### Remaining play-loop questions

- Intro duration/pacing, skip/replay, and resumed-session recap behavior.
- Viewpoint controls and cost/latency limits, continuity checks, and fallback presentation.
- Save confirmation and recovery when checkpointing or graceful stop fails.

No code, generated assets, provider calls, or commits are authorized by this planning update.

## Character creation and portability — refinement draft

Governing references: [Backstory-driven character creation](character-backstory-creation.md) and [Face portrait capture research](face-portrait-capture-research.md). Their dated implementation observations are historical snapshots, not a current claim of supported builds. The 2026-10-17 capture report's provider names, prices, and no-spend-cap recommendation are research inputs requiring revalidation, not approvals. No provider selection or live spending is authorized here.

### Proposed creation flow — not approved

After a premise is known, offer a new hero or an existing/imported hero. Guide source-valid species/class/background/abilities/equipment choices separately from appearance; exact rules-step ordering remains open. Use “species” for the 2024 label, with “race” as the user's synonymous description. This discussion selects no new species/catalog, licensed content, age, sex, or visual defaults.

Simple controls and advanced text should edit the same canonical appearance descriptor, so changing modes preserves choices. A muscular build is visual description, not an ability-score increase. Both portrait and full-body imagery should preserve approved character continuity; optional face integration should create a coherent likeness rather than literally paste the selfie onto an unrelated body.

Proposed preview behavior: inexpensive local/debounced feedback while editing, an explicit generated preview request with truthful preparation/fallback, and no paid generation for every toggle. These capabilities are unimplemented proposals, not promised instantaneous previews. Review the legal character sheet and public appearance before Ready. Backstory hooks and secrets retain the public/private/sealed consent boundaries in the backstory plan.

### Own-face capture — proposal and constraints

Prefer capture on the player's own phone with front camera, while allowing a laptop webcam. Offer preview, retake, approve, and clear. Ask explicit consent before transmitting the picture to any third-party generation provider. Request camera only, not microphone, and stop camera tracks on success, cancellation, or error. Feature-detect support and provide a photo/file picker alternative; camera permission refusal must not prevent character creation.

Browser `getUserMedia` requires a secure context: a phone visiting an ordinary HTTP private LAN IP is insufficient; localhost's exception does not make the host PC's IP trusted on a phone. A trusted HTTPS approach must be planned and tested, without treating certificate installation as an agreed user step. The photo-picker `capture` hint is browser-dependent, not a guarantee. [MDN getUserMedia](https://developer.mozilla.org/en-US/docs/Web/API/MediaDevices/getUserMedia), [MDN capture attribute](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Attributes/capture).

Propose minimizing raw-photo retention and defining deletion after approved output, but do not promise that a provider never retains input without verified terms/configuration. Store approved likeness identity separately from account identity; it is not face login. Consent revocation and deletion across raw input, approved output, and derived asset lineage require explicit rules.

### Character portability — open policy

Separate account-owned character identity, appearance, and persona from campaign-specific instances: level, XP, gear, conditions, private quest facts, and snapshots. Import must not overwrite another campaign, duplicate items, transfer secrets automatically, or bypass host rules validation/approval. A proposed default is identity/appearance continuity with mechanics adapted to the target campaign, but this has not been selected.

The central question is whether “other sessions” means continuing the same campaign, joining different campaigns with identity only, or carrying a compatible mechanical sheet between campaigns. Guest-to-account claiming must prove control of both the current session character and the account; matching an email address is insufficient. Player-account creation, recovery, and linking providers remain open.

### Public reveal — proposal

Keep the selfie/capture and editing private on the phone. Reveal only an approved public character portrait/persona on the shared display, never raw selfie, private backstory, or hidden sheet facts. A restrained one-time character-ready cue could play through the selected shared sound device, deduplicated rather than replayed on reconnect. Exact reveal timing and character ordering remain open.

## Campaign story generation — discussion proposal

Status: proposed refinement, not a selected algorithm. Governing references: [Adventure generator sections 13–17](adventure-generator.md#13-the-campaign-layer-frame), [Narrative engine](narrative-engine.md), and [Campaign authoring](campaign-authoring.md). The existing generator includes a campaign frame and chapter-at-a-time generation around a narrower five-beat playable chapter shape. Richer variety and supported actions require further implementation/verification; dated document headers alone do not show that all generation is missing.

### Candidate creation flow — not approved

After minimal setup, make the pre-campaign lobby ready and let phones join before collaborative questions or narrative generation (REF-018). The optional premise/theme or Surprise me remains a proposal. Generate several internally distinct premises rather than repeatedly reskinning one plot. Optionally show two or three spoiler-free pitches for the host to choose from; pitch selection has not been chosen as a required step.

Generate a world situation rather than a complete script: central conflict; factions with motivations and resources; secrets with multiple evidence paths; locations with stakes; and character hooks after the party's characters are known. Freeze a validated canonical initial state. Prepare the opening and next chapter in useful detail while leaving later chapters elastic enough to respond to actual play.

Persist canonical facts, who knows each fact, choices, and consequences. Antagonist progress follows approved game-time/world transitions, not the real-world campaign wall clock. Do not retcon established facts, force outcomes, or erase a legal choice to preserve an outline. The flexible campaign-span target informs proposed scope/pacing; session frequency remains unknown, so no play schedule is assumed.

For novelty, compare structural patterns such as conflict, faction relationships, discovery method, stakes, and resolution opportunities rather than names alone. Comparing this table's recent campaigns could help avoid repetition only where that history is available and its use is consented; do not assume cross-account history access.

AI proposes structured world/story candidates. Engine validation must check coherence, feasible source-legal encounters and choices, reachable clue alternatives, and audience/privacy boundaries before accepting canonical state. Narrated prose cannot author dice results or mechanically resolved outcomes. This follows existing server authority and content-validation requirements rather than handing rules authority to story text.

### Illustrative premise — not selected

A coastal town's protective lighthouse is draining memories to maintain a barrier. Its keeper wants to preserve the town, a trading faction wants to exploit the stored memories, and an exiled engineer knows another costly repair. Investigating the machinery, negotiating access to its records, or helping affected families can reveal different evidence. Repairing the barrier, exposing the trade, or accepting a temporary compromise creates different allies, costs, and future pressures; none is a guaranteed scripted outcome. Any magical mechanics, checks, and encounters would need supported rules validation.

### Next discussion

Should creating a campaign begin with Surprise me, or offer an optional campaign-vibe choice? Neither has been selected. Optional pitches, premise input, novelty policy, and generation details above remain proposals.

## Campaign seed modes — user proposals under discussion, 2026-10-23

The user proposed two ways to seed a narrative: a host-only questionnaire setting the mood, or a collaborative mode where each player contributes a few words and AI synthesizes a cohesive narrative seed. These are proposals under discussion, not confirmed modes or a finalized interaction design. REF-018 confirms only that players join through phone browsers before collaborative questions; host-only questions remain a proposed alternate.

A conceptual narrative seed is distinct from the stored random seed used for reproducible table generation. Persist accepted structured concepts and generated bytes, provenance/revisions, and applicable randomness separately. The seed should support initial preparation; the campaign's story emerges through play rather than being fixed by these inputs.

### Host questionnaire — candidate design

Offer four optional short categories: desired emotional tone (wonder, tense, hopeful); adventure draw or motif (lost city, political intrigue); stakes scale (a neighborhood, a kingdom); and pacing/roleplay-action preference. Rules modifiers and campaign length are already separate setup fields; do not ask them again. Avoid a mandatory lore essay. An unanswered field grants no specific preference and can use an explicitly disclosed creative choice rather than invented consent.

This mode is quick and coherent when one host has a vision, but can overrepresent that vision. A proposed review step should let the group flag mismatches without requiring everyone to write a premise. Exact questions, counts, and review authority remain open.

### Confirmed connection order

Host launcher sign-in and new-campaign settings lead to a ready pre-campaign lobby. Open the shared display automatically when the local server/lobby is ready (REF-004), without waiting for campaign generation. Show its join QR and room details on the display only (REF-011). Up to six human players can connect on the same network using phone browsers. Collaborative questions, if that mode is selected, are answered on the connected phones; synthesis/review and generation follow, then gameplay. Review/launch authority and participation rules remain proposals rather than newly approved requirements.

A new-campaign draft/lobby is distinct from a completed generated campaign. Its save lifecycle, recovery, abandonment, and transition to a playable campaign remain open. Joining authentication remains unresolved between guest and account approaches.

### Collaborative words — candidate design

Invite each of up to six players to contribute two or three short inputs, optionally tagged as desired feeling, thing/place/motif, and complication/question. Allow raw words with AI suggesting categories instead of requiring form-like precision. Private initial submissions could reduce anchoring by the first confident contributor. Provide skip/asynchronous participation and a disclosed completion time; do not require everyone present or interpret silence as a vote.

AI should interpret ambiguity explicitly and ask when an interpretation materially changes the premise. Synthesize a primary tone with compatible motifs; contradictory tones may be grounded reinterpretations or returned for revision. Do not promise literal inclusion of every word, force a kitchen-sink setting, or use majority preference to bulldoze a participant. Explain what was incorporated and what needs clarification.

### Shared synthesis and review — proposals

The structured conceptual seed could contain a public premise, tone, central conflict, faction motivations, open questions, contribution-to-theme mapping, and constraints. Keep private GM truth in a separate server-only representation; a public spoiler-free synthesis and contribution mapping must not expose personal secrets or private boundaries. AI drafts are not game canon until validated and accepted for launch.

Separate desired content from content boundaries. Boundaries are exclusions rather than creative prompts, and vetoed themes cannot be restored by majority vote. Preserve the audience scopes of [character hooks and knowledge](narrative-engine.md#authoring-and-state). Existing [content rating](adventure-generator.md#21-content-rating-play-r01) is a distinct setting; this proposal does not replace its recorded defaults or disclose an individual's preferences to the table.

Propose lightweight review: edit one's own inputs and reblend the synthesis before committing; defer expensive art until acceptance. Host final launch permission is a proposal, with personal boundaries preserved. Validation still checks source-legal mechanics, coherence, reachable information, privacy, and canonical compatibility under [Campaign authoring](campaign-authoring.md). AI text cannot create rules or override server balancing/dice resolution. Avoid a compulsory structural recipe, predetermined twist, or mandatory climax that makes every campaign feel the same.

### Worked three-player example — illustration only

- Player A: wonder / lighthouse / disappearing stars.
- Player B: hopeful / old machines / who maintains them?
- Player C: tense / debt / what is the cost of safety?

One cohesive synthesis: an aging lighthouse guides ships by borrowing starlight, and its maintenance guild is calling in an old civic debt as the sky grows dim. Wonder leads, hope comes from recoverable engineering knowledge, and tension comes from competing obligations. The opening can ask whether to investigate the guild's demand, help an affected ship, or inspect the machine. This maps the contributions into one conflict without requiring three unrelated plots. The true cause, factions, and outcomes remain drafts; no twist or solution is guaranteed, and supported mechanics must be validated.

### Other optional ideas — not selected

- Hybrid host frame plus player motifs, as a possible default candidate only.
- A shared bond or unresolved question linking characters, using consented character information.
- An opening dilemma whose direction is chosen during play rather than settled in setup.
- An image or sound as an optional seed artifact only with appropriate rights/access; no sourcing or upload flow is selected.
- A later campaign inheriting selected legacy consequences only with consent and available authorized history.

Open: whether to offer both modes, optional hybrid/default choice, input limits, asynchronous timing, review/launch authority, privacy presentation, and how much early preparation to generate. No algorithm or mode selection is confirmed by this discussion.

## Pregame flow, visual flair, and sound — refinement proposal, 2026-10-23

Status: a detailed discussion draft requested by the user. Confirmed decisions are recorded above; the proposed stage details, visuals, sounds, and authority below are not new approvals. Follow [Client presentation](client-presentation.md) and [Tempo engine](tempo-engine.md) for audience-safe rendering, cue lifetime, playback unlock, accessible preferences, and stale-cue rejection. This is conceptual planning; no assets or implementation are produced.

### Proposed sequence and presentation

| Stage | Action and state | Visual direction | Sound proposal |
| --- | --- | --- | --- |
| Launcher | Sign in; Continue adventures or Create new campaign; new-game settings. | Clean readable local window with understated photographic/live-action atmosphere; avoid making every control fantasy parchment. | No competing launcher background soundtrack. |
| Lobby ready | Local server/lobby readiness opens shared browser display before generation. QR and room details appear here only; phones join on the same network. | Ambient lit tabletop or restrained scene, symbolic seats/camp lights, room details always legible. | Explicit Enable sound gesture on the shared display; quiet ambience after unlock. |
| Players join | Up to six players; phone display-name/seat selection is a proposal and joining auth remains open. Connected, submitted, and ready are distinct states. | Consistent seat identity on phone/display using labels and symbols as well as color. A new join warms one light. | One soft deduplicated join chime for a new join, not every reconnect. |
| Questions | Host selects a proposed seed mode; if collaborative, phones collect proposed two or three private inputs with Send/Skip and editing before a declared lock. | Clear phone prompts; public display shows only permitted aggregate progress, never typed words or private flags. | Optional local tap feedback, muted by default; submitted cues distinct from joins and batched so six completions do not become noisy. |
| Synthesis/review | Public spoiler-free synthesis; players can adjust their own inputs/reblend. Host final launch permission remains a proposal. Preserve boundaries and private GM facts. | Restrained word motes only for consented public motifs, with a skip/static alternative. No secret words or generic loud particle spectacle. | Short restrained transition, no sound that discloses a private boundary or secret. |
| Prepare opening | Lock the accepted concept, validate it, and prepare only the opening/near-term content needed to begin. Retry preserves accepted state. | Honest phase labels, calm sustained ambience, readable error/retry. No invented percentages, countdown rush, or fake progress. | Quiet sustained ambience; no repeated completion sting while waiting. |
| First scene ready | Enter gameplay when required state and playable fallbacks are ready. Character ordering remains open. | Smooth readable scene/camera transitions; reduced-motion/static equivalents. Optional video/art never gates access to a usable first scene. | Narration uses captions and ducks ambience/music, respecting mute and separate volumes. |

The prior two seed modes remain unconfirmed alternatives; this flow does not require every player to answer, create an account, or arrive simultaneously. Skipping is an explicit choice, not inferred consent. Absent-player policy (wait for everyone versus host proceeds), timing rules, and what counts as ready require decisions; no forced timer is selected.

### Lifecycle and recovery — proposed

Possible server-owned stages are lobby, collecting, synthesizing, review, preparing, ready, and active. These are draft names rather than finalized contracts. Reconnecting or late-joining phones receive the exact current permitted step, existing seat identity, and their accepted submissions; they do not restart the group or replay old audio. Presence alone does not mean answered/ready. Question locking, edit revisions, and proceed authority need explicit rules.

Retry should keep submitted inputs and an accepted canonical seed, use idempotent generation/publication operations, and avoid charging/regenerating accepted work unnecessarily. Private submissions, content boundaries, account keys, and server-only GM truth must never leak into public motifs or progress. The new-game draft/lobby save lifecycle remains open.

Character creation ordering is also a proposal: collect broad party interests, establish an accepted premise, then create characters suited to it and ask consent before weaving backstory hooks into canonical facts. Alternatives (characters first or premise first without hooks) remain possible. The full campaign should not be generated in advance; later story emerges from choices.

### Audio and accessibility — proposed defaults

The shared display is the proposed primary sound device. Because browsers can block autoplay, show Enable sound and preserve a fully usable silent path. Phones should not echo public narration; optional local tap/tactile feedback starts muted and respects capability/accessibility. No voice input is required. Persist mute, separate music/effect/narration levels, captions, and reduced-motion choices; presentation controls must not change game mechanics.

Audio failure must not block joining, answering, review, or play. Do not replay old narration/cues on reconnect. Avoid mandatory flashing, camera punches, or shake; provide readable static emphasis. Defer expensive generated art/video until the concept is accepted, and do not require remote asset completion for the lobby. Preserve the established photographic/live-action visual direction while keeping controls legible.

### Questions to settle next

- Visual concept: ambient photographic tabletop with symbolic seats/camp lights, or another restrained scene treatment?
- Sound hosting: shared display only as the default, with muted optional phone feedback?
- Character ordering: broad interests, then premise, then character creation with consented hooks, or characters first?

These are discussion questions, not selected defaults or implementation authorization.

## Minimal setup: planning draft

The setup categories and launch sequence in REF-014 and story-length meaning/target behavior in REF-015–REF-016 are confirmed. Layout, defaults, and the candidate settings below remain unapproved. Proposed layout: three primary fields with optional settings collapsed initially.

| Primary field | Confirmed direction / governing design | Open decisions |
| --- | --- | --- |
| Player count | Maximum six human players; DM role does not use a seat. | Minimum, default, changing the count later, companions, and balancing. |
| Ruleset modifiers | Offer this setup category. Existing [rules support](rules-support.md) requires standard 2024 fifth-edition D&D as the base; do not silently mix editions or relabel house rules as standard. | Which supported modifiers to expose and their defaults. Candidate house rules require explicit opt-in, participant disclosure, and versioned definitions under [Generated content](generated-content.md#optional-custom-definitions-are-a-distinct-ruleset). Preserve authoritative source-valid mechanics and honest dice outcomes. |
| Story length | Real-world campaign span across sessions, in units such as hours, days, weeks, and months (REF-015), as a flexible pacing target that can extend (REF-016). | Allowed values, when the clock starts, and how session duration/frequency relate to the span. No automatic duration enforcement is selected. |

REF-015 resolves duration meaning as real-world campaign span across play sessions; REF-016 makes it a flexible target that can extend. The next open proposal is optional expected session duration and play frequency to help estimate achievable story scope and pacing. These settings and any schedule remain unapproved. The target does not select automatic ending or deletion.

### Additional base settings to consider — proposals only

- Session length, separate from overall campaign span.
- Starting character level.
- Tone and content boundaries.
- Combat, exploration, and social emphasis.
- Challenge within the selected rules.
- Narration/music volume and captions as device preferences, distinct from campaign configuration.

Account sign-in must align with [Customer, commerce and admission contracts](commerce-service.md#account-and-tenant-lifecycle): identity adapter selection remains open, and identity, host, payer, and player permissions are separate. This document does not select a login provider or require every player to hold an account. Sign-in recovery, remembered sessions, and offline behavior require further decisions. The existing service plan states customer accounts are not yet implemented.

Proposed Resume behavior: use the saved game's configuration rather than repeat new-game setup. This has not been approved.

## Account authentication research and proposal — 2026-10-23

Status: research and recommendation for discussion only. No identity provider, authentication strategy, or session policy is selected here; this section adds no confirmed decision ID.

The [existing account lifecycle plan](commerce-service.md#account-and-tenant-lifecycle) leaves managed identity versus a native passkey/email adapter unresolved at G04. Host, payer, and tenant owner are separate capabilities. Linking identities requires proof of both identities; matching email addresses alone must not merge accounts.

### Recommended candidate flow — not approved

Use a managed OIDC identity service. Begin sign-in from the launcher, open the default system browser for authorization-code flow with PKCE, then return through a verified callback to the launcher. Remember a revocable session using the Windows credential store so returning users can avoid repeated prompts while the session remains valid. Credential entry would occur in the system browser even though sign-in begins in the local window; this change in interaction needs explicit design approval. Do not silently embed OAuth credential entry in the launcher. [RFC 8252](https://www.rfc-editor.org/rfc/rfc8252) and [Google native-app OAuth guidance](https://developers.google.com/identity/protocols/oauth2/native-app) describe external-browser/native-app authorization and PKCE considerations.

Candidate choices: Google as the primary social option, Microsoft as an additional option for a Windows audience, email one-time code as a fallback, and optional passkeys for return convenience. This is a recommendation, not a selected button set or enrollment requirement.

| Strategy | Convenience and tradeoffs to assess |
| --- | --- |
| Social sign-in | Existing browser sessions can reduce effort; depends on an external identity account and requires provider/linking/recovery design. |
| Email one-time code | Avoids a new password and works when email is read on another device; requires entering a code and handling delivery delay/expiry. |
| Email magic link | Avoids typing a code, but opening email on another device or browser can complicate returning to the desktop launcher; callback and cross-device behavior require verification. |
| Passkeys | Convenient return sign-in after enrollment; enrollment, available authenticators, device transfer, and recovery need a clear alternative path. |

[Auth0 passwordless email guidance](https://developer.auth0.com/resources/labs/authentication/passwordless-with-email), [Clerk sign-in options](https://clerk.com/docs/guides/configure/auth-strategies/sign-up-sign-in-options), and [Auth0 Universal Login](https://auth0.com/docs/universal-login) are vendor references for evaluating these choices, not evidence that a provider has been selected or integrated.

Provider and callback support for the Rust desktop application must be verified before selection. Auth0 is one candidate only. Clerk's native Electron SDK uses JavaScript and does not directly match the project's Rust-only application requirement; a compatible standards-based integration would need verification. No vendor pricing or cost conclusion is recorded.

### Proposed ownership boundary — not approved

Keep cloud account identity separate from the local gameplay server. LAN room invitations grant only the appropriate joining capability and never carry the owner's account token. Optional guest players could reduce joining friction, but guest access and player-account requirements remain undecided. Authentication alone must not grant payer, owner, host, or private-player capabilities.

Open decisions: browser-based credential interaction, identity provider and Rust integration, remembered-session lifetime/revocation, sign-out behavior, offline access, account recovery, guest players, and provider cost. Review these alongside the existing account/tenant contracts before implementation.

## Payment research and proposals — 2026-10-23

Status: the user requested retaining the authentication research and considering payment processors. Adding those notes does not ratify Google, email code, remembered sessions, or an identity provider. No processor, merchant-of-record service, pricing model, or billing policy is selected. This section adds no confirmed decision ID.

### Candidate shortlist

| Candidate | Evaluation direction |
| --- | --- |
| Stripe Checkout, Billing, and customer portal | Hosted checkout and billing management, with control over the integration and the responsibilities of a standard processor arrangement. [Checkout](https://stripe.com/payments/checkout), [customer portal](https://support.stripe.com/questions/billing-customer-portal). |
| Paddle | Merchant-of-record candidate to reduce transaction-commerce administration, subject to seller/product/market eligibility and contractual scope. [How Paddle works](https://developer.paddle.com/get-started/how-paddle-works/saas/). |
| Lemon Squeezy | Alternative merchant-of-record candidate; evaluate API/checkout fit, customer experience, statement descriptor, eligibility, and operational terms. [Taking payments](https://docs.lemonsqueezy.com/guides/developer-guide/taking-payments), [statement descriptor](https://docs.lemonsqueezy.com/help/getting-started/statement-descriptor). |

Standard Stripe processing and [Stripe Managed Payments](https://stripe.com/gb/managed-payments), its separate merchant-of-record offering, must be evaluated separately. Managed Payments eligibility, geographic availability, and product fit require verification; Checkout alone does not establish a merchant-of-record arrangement.

Compare merchant-of-record convenience against standard processor control and merchant responsibilities. Merchant-of-record transaction-tax handling applies within documented transaction/contract scope; it does not establish exemption from every business tax or company obligation. No pricing or eligibility promise is recorded.

### Candidate payment flow — not approved

Launch browser checkout and Manage billing from the launcher. A cloud backend verifies signed webhooks and publishes versioned entitlement state. A checkout return URL is never proof of payment. Neither the executable nor local gameplay server holds merchant secrets or handles raw cards. Account owner, host, and payer remain separate capabilities.

Use durable idempotent operations, webhook deduplication, and reconciliation for delayed, duplicate, out-of-order, and unknown results. Follow the [commerce service payment contracts](commerce-service.md#payment-and-entitlement-state-machine) and [service operations](service-operations.md); these boundaries require implementation and verification.

The existing proposed Stripe adapter, 72-hour renewal grace period, and historical pricing/allowance assumptions in [Commerce service](commerce-service.md), [Commercial validation](commercial-validation.md), and [Pricing and costs](pricing-and-costs.md) are prior planning inputs. This research does not newly confirm those policies or select Stripe.

### Open evaluation questions

- Seller country and customer geographies; eligibility, currencies, and payment methods.
- Subscription versus per-campaign purchase versus usage top-ups; none selected.
- Included AI usage, explicit spend caps, overage consent, and prepared fallbacks; no unlimited-cost promise.
- Refunds, cancellation, disputes, payouts, support ownership, and migration/export options.
- Transaction-tax scope, verified provider fees/terms, and other business obligations for the chosen arrangement.

Authentication and payments should share verified identity while retaining separate host, tenant-owner, payer, and entitlement capabilities. Sign-in alone does not establish paid entitlement. Authentication recommendations, offline/recovery/session policy, provider costs, and these payment proposals remain open.

## Player count

The agreed maximum is six human players, excluding the game's DM role (REF-013). This decision sets the maximum only. It does not select a one-player minimum, a four-player default, companion behavior, or an encounter-balancing policy.

For tabletop research context, the [2024 D&D Basic Rules, “The Basics”](https://www.dndbeyond.com/sources/dnd/br-2024/the-basics) describes four to six adventurer players plus a Dungeon Master. This guidance is separate from actual DungeonFlux software support: the existing prototype has two seats; six-player support still requires implementation and verification.

Open decisions: minimum supported players, default party size, whether or how companions fill a party, and encounter balancing across party sizes.

## Agreed launch flow

1. Double-clicking the executable or its shortcut opens the local launcher window with account sign-in. After sign-in, offer Continue adventures or Create new campaign (REF-017); new-campaign setup covers player count, ruleset modifiers, and story length. Sign-in session retention remains undecided. If the game is already running, reconnect to it and show Open game display and Stop, without starting a duplicate server.
2. Starting a new game assigns its saved-game name automatically without a prompt. Resume offers saved games with names and last-played dates. Exact control layout remains undecided; reusing saved configuration on Resume is a proposal only.
3. Start or resume automatically allows phones on the same network to join. The launcher shows preparation status and progress, then opens the shared display in the default browser once the local server is ready. For a new campaign, this is pre-campaign lobby readiness; do not wait until narrative generation is finished.
4. Joining details stay on the game display only. Players join in their phone browsers before collaborative questions. If collaborative seeding is selected, questions are answered on phones, then synthesis/review and generation precede gameplay; review authority and full mode details remain undecided. Startup failures show an actionable error with Retry in the launcher; the normal flow needs no terminal.
5. Closing the launcher leaves the game running. Reopening reconnects to it. Stop shuts down gracefully, keeps progress available for Resume, and leaves the launcher open.

This is the agreed direction for refinement, not a claim that the launcher has been implemented.

## Launching the game: current implementation

These are source observations, not newly agreed product requirements. The game has not been launched or verified as part of this document change.

The current local entry point is [`development/run-gameplay.sh`](../development/run-gameplay.sh). The [development guide, “Play the game locally”](../development/README.md#play-the-game-locally) describes building and starting the local game server and PostgreSQL, then opening the `display` URL from the server's `READY` line and the `player` URL in browser tabs for the player seats.

```sh
# Build and start a local game.
development/run-gameplay.sh

# Resume a saved game using the database name printed by the launcher.
development/run-gameplay.sh --no-build --database <name>

# Allow phones or other devices on the same network to join.
development/run-gameplay.sh --lan
```

`--no-build` uses existing build outputs. `--lan` attempts to expose the server through this machine's network address; the script reports when no LAN address is available. A room code appears in the player link and the display's join panel. Ctrl-C stops the server and PostgreSQL when the launcher started PostgreSQL. Refer to the guide and script for prerequisites, configuration, and additional options.

## Launching the game: discussion

### Resolved — invocation

REF-003 selects the desktop shortcut and launcher window; REF-014 also allows double-clicking the executable and adds account sign-in followed by minimal setup. REF-006 selects Open game display and Stop actions for the running game. REF-008 selects saved-game listing for Resume. Other controls and detailed state presentation remain undecided.

### Resolved — opening the game display

REF-004 selects automatic opening in the default browser after server readiness on start or resume.

### Resolved — closing the launcher while running

REF-005 leaves the game running when the launcher window closes, until an explicit Stop action. Tray UI has not been selected.

### Resolved — reopening the launcher

REF-006 reconnects to the existing running game and shows Open game display and Stop without starting a duplicate server for it.

### Resolved — Stop behavior

REF-007 selects graceful shutdown, progress available for Resume, and leaving the launcher open. Exact shutdown and save-failure handling remain undecided.

### Resolved — Resume selection

REF-008 selects a saved-game list with names and last-played dates.

### Resolved — new-game naming

REF-009 selects automatic naming without a prompt. The naming algorithm and later renaming remain undecided.

### Resolved — same-network phone joining

REF-010 enables same-network phone joining automatically on start or resume, without a separate opt-in toggle for normal launch. This does not select an internet-hosting approach.

### Resolved — joining information

REF-011 keeps join details on the game display only; the launcher does not show the room code or join QR.

### Resolved — startup status and errors

REF-012 selects visible preparation status/progress and actionable startup errors with Retry in the launcher. The normal launch flow requires no terminal.

### Remaining open details

- Automatic naming algorithm and whether saved games can be renamed later.
- Exact shutdown and save-failure handling.
- Packaging and installation, whether or when builds occur, and update behavior.
- Detailed layout, saved-game list presentation, and status/error presentation; tray UI remains undecided.

### Candidates for discussion — not approved

The placement and presentation of Start new and Resume controls, along with detailed running and stopped states, remain proposals. REF-012 confirms preparation status and progress. REF-006 confirms Open game display and Stop actions for the running game; detailed presentation is undecided.

The earlier browser entry candidate (host new/resume and player join) is deferred outside the current local-launcher scope. It is not an approved product decision.

## Custom character creation — refinement and implementation boundaries, 2026-10-23

REF-026–029 confirm the requested direction. The detailed workflow here is a proposal; source inspection and next-step acceptance are retained in [Character creation research](character-creation-refinement-research.md). No full-catalog character creator is claimed implemented.

### One accepted hero, several outputs

1. Select the campaign's pinned content catalog and source-valid mechanical options. Simple controls and advanced appearance text edit the same canonical appearance draft; changing modes preserves choices. Appearance is cosmetic and never grants ability scores, class features or equipment.
2. Derive and validate the readable character sheet on the server. Keep identity/backstory and mechanical state separate; an illustrated sheet is a view of these records, not the source of truth.
3. Generate and review a portrait and visual reference pack from the accepted public appearance and actual granted gear. Proposed reference views are front, profile, back and useful face/clothing detail; exact count/layout remains open. These are visual continuity references, not a second mechanical sheet.
4. Approval pins an immutable identity/asset revision. Later edits propose a new revision and preserve previously referenced assets; failures preserve the draft and usable prior imagery. Reuse accepted images and cache hits instead of regenerating every toggle, session or scene.
5. Scene plates consume the permitted public cast's approved portrait/reference assets and current server scene facts. Never submit a private character sheet, sealed backstory or raw selfie as ordinary scene-plate context. Optional own-face capture remains separately consented under REF-020; no face-authentication or provider upload path is newly approved here.

The current advanced endpoint matches known menu phrases locally. A fully descriptive custom-body schema and durable approved portrait/reference pack remain implementation work. Existing canonical character generation and plate composition hooks can be extended rather than replaced.

### Selected initial 2024 core catalog

Verified on 2026-10-23 against the official [2024 species overview](https://www.dndbeyond.com/posts/1783-the-10-species-in-the-2024-players-handbook) and [2024 class index](https://www.dndbeyond.com/sources/dnd/br-2024/character-classes). These name lists establish the initial planning denominator, not full book rights, complete entry data or implemented rules.

- Species: Aasimar, Dragonborn, Dwarf, Elf, Gnome, Goliath, Halfling, Human, Orc, Tiefling.
- Classes: Barbarian, Bard, Cleric, Druid, Fighter, Monk, Paladin, Ranger, Rogue, Sorcerer, Warlock, Wizard.

The user selected this 2024 core catalog first. Expansion books and homebrew remain unselected future scope. Do not silently mix 2014 options, expansion books, third-party content or homebrew into a standard 2024 campaign. Core backgrounds, feats, species/lineage choices, class features, subclasses, equipment and spell choices are dependencies, not implied by adding these 22 labels.

Settings should identify source/version and distinguish **listed**, **host-enabled**, **implemented/playable**, **unimplemented**, and **source/rights unavailable**. An enabled choice without its required handlers must remain unavailable for mechanical finalization with a clear reason. Name-only or cosmetic previews must not masquerade as playable classes.

Current inspected boundaries: the rules parser recognizes nine SRD species but creation validates only Dwarf, Halfling and Orc. The current harbor pack actually offers only Dwarf, with Fighter/Soldier builds. Fixed Fighter/Champion progression through level 5 is not general levels 1–20 support, multiclassing or a complete 2024 class catalog. The only acquired frozen source is SRD 5.2.1; the manifest still marks full core books and non-SRD rights unresolved.

### Cost and provider direction

Still portraits and reference images stay in scope; new generated video is off by default. Home clips require `DF_GAMEPLAY_HOME_VIDEO=on`; scene and kill clips require `DF_GAMEPLAY_SCENE_VIDEO=on`; character-action clips require `DF_GAMEPLAY_CHARACTER_VIDEO=on`. These are explicit development opt-ins, not normal defaults. Existing cached character clips may remain readable, but disabled generation must not request new clip-only image frames or provider video. Static stills/portraits/scene-plate generation remain separate.

The image model was already OpenAI by default. Provider routing should now select OpenAI when `DF_GAMEPLAY_IMAGERY_PROVIDER` is absent/empty or `openai`, even if a fal key is configured. Explicit `flux` remains a development override requiring its key; unknown values refuse. Existing imagery admission still requires a usable OpenAI credential before route selection, so a fal-only environment must not be described as an automatic OpenAI or working FLUX fallback. This source limitation is disclosed for a future admission refinement.

### Proposed implementation order

- First: enforce the cost/provider defaults and record the complete selected initial catalog with truthful support statuses; no fake playable choices.
- Next bounded feature: persistent custom appearance draft, legal server-derived sheet, explicit portrait/reference-pack generation and approval, and those approved assets flowing into an actual shared scene plate. Verify one full creation→approval→resume→plate loop before expanding mechanics.
- Separately: pin/admit the complete selected rules/catalog and implement source-linked class/species/background/subclass/spell/advancement handlers in reviewed increments. Unsupported entries remain visible backlog, not synthetic Fighter substitutions.

## Local development tooling — implementation notes, 2026-10-23

The user prioritized configuration and launch cleanup before the new character-creator design, then requested direct development entry into existing screens/phases and documentation of executable flags/configuration. These tools open the existing implemented game screens. The existing character creator now has a visual redesign in `hero_creator.rs` and `hero_creator.css`: a photographic sample-character stage, responsive dossier layout, and separate appearance, story and equipment sections that retain the same draft. This delivers the visual editing surface only; the full custom-identity workflow, account persistence, generated portraits/reference packs and complete 2024 core mechanics remain pending.

The user explicitly confirmed that character creation must work well on phones, tablets and laptops. Editing takes priority over decorative artwork: phones use a prominent character portrait and touch-sized controls, tablets use the available width intentionally, and laptops give the editor more room than the illustration. Both portrait and landscape layouts must retain readable controls, reachable confirmation, and the same draft when switching sections. Responsive review includes narrow 320px phones, 390px phones, 768px portrait tablets, 1024px landscape tablets, 1366px laptops and 844px short phone landscape; these viewport checks describe layout evidence, not physical-device certification.

- [Launch and configuration reference](../development/launch-reference.md) documents actual executable modes, shell and development-helper flags, defaults, environment precedence, content files, and private credential-file locations without credential values. The native executable reads process environment, not a general launcher JSON/TOML file; `.env` loading belongs to the normal shell launcher and is explicitly disabled in the development helper.
- `development/dev-gameplay.py` offers `--list-screens`, `--screen`, and `--role`, prepares a fresh isolated local game, disables provider generation, and uses authenticated server-valid actions to reach an implemented phase. Player entry uses a bounded single-use local ticket and retains ordinary server authorization. Planned account and questionnaire screens are not fabricated by this tool. Later-phase attempts can explicitly refuse or time out; supported target names are not evidence every late-game path is verified.
- Launch cleanup consolidates the database-port setting and its compatible legacy alias, removes duplicate campaign builds, provides side-effect-free launch-input preflight, and targets the exact server-published stop marker in an owned per-launch directory.
- [Windows desktop entry](../development/windows-desktop.md) documents the custom application icon, native Windows resource embedding, and installation of **DungeonFlux (Development)**. This shortcut opens the current development character-creation flow in a browser through a visible console. It is distinct from the planned account-sign-in launcher; closing this console is not the product's agreed persistent-launcher behavior.

These implementation notes do not change the confirmed product decisions above. Visual character creation, accounts, payments, six-player expansion, and collaborative campaign seeding remain separate implementation work.

## Explicit portrait generation — refinement, 2026-10-23

- **REF-032 — Generate portraits from the player's current appearance in the creator.** Replace the temporary sketch with an explicit **Generate portrait / Regenerate portrait** action connected to OpenAI. The selected visual controls and exact public custom appearance text form the input; private story, names and camera images do not. Editing does not generate paid calls. A changed appearance marks the prior portrait out of date, and a stale result must not replace the current look. Preserve the prior usable image on failure, show real pending/error/unknown states, and never automatically resend an uncertain generation.
- Historical qualification used one actual UI-triggered image request, 1024×1536 medium, at most180 seconds, with no image inputs, judges, rewrites, video or unrelated provider work. The temporary request allowance was rejected and removed under REF-034. It is not a development-session quota. Account billing and final portrait/reference-pack approval remain separate work.
- The first working portrait milestone must qualify the actual mounted creator → authenticated request → existing image resource worker → OpenAI adapter → visible result path. A separately generated or hard-coded image is not evidence that this feature works. Durable approved portrait/reference revision attachment is still a required follow-up before claiming full creation finalization.

- **REF-033 — Portrait placeholders communicate generation status.** Before a portrait exists, show an intentional shadowy silhouette rather than a fixed sample person. While an image is being made, animate a visible glow/orbit over the shadow and show readable generation status; reduced-motion users receive a static status. Keep the previous usable portrait in state during regeneration and restore it after failure. The ambient backdrop must not include a fixed character photograph.

- **REF-034 — No portrait quotas during local development.** The user explicitly rejected session allowances and development spend limits. Remove all portrait request counters/quotas from the local creator and developer helper. Generate and Regenerate remain freely available as explicit user actions. Preserve authentication, exact-operation idempotence, no automatic network retries, current-draft matching and real status reporting. Commercial payment/entitlement policies belong after development. This supersedes the earlier one-request qualification setting above; it does not authorize background generation or repeated automatic calls.

- **REF-035 — Apply Claude Code's frontend-design skill to the creator.** The user requested a more exciting and interesting UI and UX using the installed Claude Code design skill. Refine the current working creator into a distinctive fantasy character workshop with a stronger portrait stage, clearer creation sections and more visual appearance controls. Preserve real input-driven OpenAI generation, detailed custom appearance, the shadow/loading states and quota-free local development. Phone, tablet and laptop usability, readable controls, keyboard access and reduced motion remain required. The precise visual treatment is an implementation choice to be reviewed in the running application.

- **REF-036 — Give the portrait more space on phones.** The user rejected the small horizontal thumbnail in the first workshop pass. Use a substantially larger portrait stage in mobile layouts so the character is easy to see, with editing controls below. Preserve usable narrow-phone and short-landscape layouts, readable generation status and reachable confirmation.

- **REF-037 — Style native dropdown options consistently.** Give select menus readable option text, appropriate dark backgrounds and distinct selected/disabled states matching the workshop. Preserve native keyboard and mobile picker behavior; mobile operating systems may draw their own picker surfaces.

- **REF-038 — Use generated placeholder artwork and a portrait reveal.** Replace the CSS-only silhouette with an AI-generated shadowed adventurer asset. Show a visible animated generation loader over this asset while a request is pending, with readable status and a static reduced-motion alternative. When the current portrait image has loaded, fade it into view. Preserve current-draft matching and the previous usable image on failure; these animations must not dispatch image requests. The initial local motion-toggle implementation was rejected under REF-040.

- **REF-039 — Classify public appearance descriptions with Jev before image routing.** The user requested Jev to categorize Public appearance details before generation, retaining OpenAI image generation for supported requests and selecting a more permissive alternative for mature content. Use the existing Jev System One choice client to judge the complete public appearance in context; no keyword routing. The initial OpenAI text-classifier implementation missed this intent and is superseded by Jev before final qualification. Provider compatibility is separate from a safety label: ordinary appearance uses OpenAI, while a distinct nonsexual adult anatomy-detail category can select BFL before generation. Body proportions are not automatically sexual content. Actual nudity and violence retain separate maturity/context checks. The first live test selected OpenAI for the reported wording and the provider refused; this prompted the separate anatomy-detail routing category rather than an automatic retry or provider fallback. The user clarified NSFW as nudity and gore. Treat nonsexual adult nudity and violence as separate research categories; the selected alternate integration is direct BFL FLUX 3 Image; precise supported intensity still requires provider-specific validation. BFL credentials are configured privately and a read-only authenticated connection check succeeded; no credential value belongs in this document. No automatic fallback is approved by this note. Keep the exact public appearance draft and classify only at an explicit generation action, not every keystroke. Provider refusal must remain distinct from transport failure and must not silently trigger another paid request.

  Live qualification on 2026-10-23: the actual creator UI on port 63519 selected the nonsexual adult anatomy-detail category through Jev in 376 ms and dispatched BFL before any image generation. The visible loading indicator animated. BFL then failed after 32,307 ms; no completed portrait or successful reveal is claimed for this request. The draft remained intact. The user requested actionable logs: preserve stage, provider, closed error category and available HTTP status with operation/request correlation, without keys, raw prompts, response bodies or signed delivery URLs.

  Research checked on 2026-10-23: [OpenAI image generation](https://developers.openai.com/api/docs/guides/image-generation) documents input/output filtering, auto/low moderation levels and stable moderation-blocked errors; [OpenAI moderation](https://developers.openai.com/api/docs/guides/moderation) supports text/image classification. These controls do not establish that a particular mature-art prompt will be accepted. [fal acceptable use](https://fal.ai/legal/acceptable-use-policy) has its own content restrictions across third-party models; it cannot be assumed to be an unrestricted fallback. [Black Forest Labs usage policy](https://bfl.ai/legal/usage-policy) also applies to its models and derivatives. An alternate route needs a specifically selected model and verified supported content, not an NSFW label alone.
- **REF-040 — Remove the portrait motion option.** The user explicitly rejected the Reduce portrait motion control. Remove the setting and make the pending loader and image-load reveal automatic. Keep motion confined to the portrait: under reduced-motion preferences use a small steady loading indicator and a short opacity reveal, without ambient breathing, blur or scale effects. This supersedes the initial opt-in implementation noted in REF-038 and the earlier static-only reduced-motion wording in REF-033/038.

- **REF-041 — Make portrait failure logs actionable.** On an explicit generation request, record Jev category, selected provider/model, operation and matching request hash. Provider failures retain the failed step, closed error class, observed numeric HTTP status (null if no HTTP response), bounded BFL delivery hostname and a closed admission reason. Keep secrets, raw appearance prompts, response bodies and signed URL paths/queries out of logs. Show a useful stage-specific failure in the creator and preserve the draft. The implementation uses the shared audit path, with independently checked fixtures for submission, polling, delivery rejection and download status. Native and WASM integration checks pass. The documented exact `delivery.bfl.ai` hostname is accepted alongside regional BFL delivery domains while HTTPS, public DNS, pinning and credential-free download checks remain enforced; this compatibility correction alone does not prove the cause of a previous live failure.
The first live request with the richer logs identified `delivery.us7.bfl.ai` and `dns_target_refused`. Its actual public DNS answers included Azure IPv6 `2603:1061:14:46::1`; the shared validator mistakenly checked `2000::/7`, excluding that address. Correct the mask to the intended [IANA global-unicast `2000::/3`](https://www.iana.org/assignments/ipv6-address-space) while preserving special/private-address and mixed-DNS denials. This is the concrete defect identified by the new logs; the earlier exact-host compatibility change was not established as the cause.
Final qualification: after the shared IPv6 correction, a real creator UI request on port 63549 selected Jev anatomy-detail in 276 ms and BFL produced the portrait in 56,548 ms (whole creator job 56,825 ms). The browser decoded and displayed the 1024×1536 portrait with opacity 1. The exact requested body description remained in the draft. The loader had been visibly animated; this final observation confirms the loaded reveal outcome, not an independently sampled intermediate fade frame. Final native build SHA256: `7A587AA8CD1FE6ED1562B2BD7CA24C955F2930554229F3393B5C1EE1BD071229`; served WASM `A83CCFBEB2806AC85F64FF9C5E6CB6CE1EE2A3EAA20DD4012531A85AF769951C`. Native and WASM integration checks, seven provider boundary fixtures, two IPv6 regressions and eight existing output-fetch tests passed; previously disclosed baseline Clippy blockers remain. No full catalog, character-sheet approval or scene-reference integration is claimed by this portrait qualification.
- **REF-042 — Select ancestry, class and background in the creator.** The user requested readable option menus, adjustable character backgrounds and selectable ancestry across D&D classes. Use the previously selected 2024 core catalog: ten ancestries, twelve classes and sixteen backgrounds. All ancestry/class/background combinations may be selected for character design, preserved in the current draft and passed as checked public visual context to portrait generation. A catalog change invalidates a portrait for the previous design. Native option popup text must remain readable even when Windows ignores custom popup backgrounds; preserve mobile picker and keyboard behavior. Existing source-backed gameplay validation remains authoritative: a selectable design without its required mechanics must be identified clearly rather than silently finalized as Fighter/Soldier. Full class mechanics are not supplied by adding names to a selector.

  Background names verified on 2026-10-23 from the official [2024 background overview](https://www.dndbeyond.com/posts/1785-the-backgrounds-and-origin-feats-in-the-2024): Acolyte, Artisan, Charlatan, Criminal, Entertainer, Farmer, Guard, Guide, Hermit, Merchant, Noble, Sage, Sailor, Scribe, Soldier, Wayfarer. This is a catalog-name reference, not a claim of full-book source/rights acquisition or complete implemented mechanics.
  Integrated qualification: all 1,920 design combinations passed focused server tests; protocol and native/WASM checks passed. Actual UI selection and portrait generation were exercised on port 63589, including a Dragonborn/Ranger/Guide with explicit nonhuman anatomy and custom bronze scales/green cloak. Jev took 271 ms; the complete OpenAI creator job took 10,069 ms. The loaded portrait was 1024×1536. A class edit marked the previous portrait out of date without another paid call. Responsive checks from 320 to 1366 pixels showed no horizontal overflow. See [catalog implementation and qualification](character-design-catalog.md) for retained limits, build identities and baseline lint/schema blockers. Background means the D&D origin selection in this change; adjustable portrait scenery remains unconfirmed.

- **REF-043 — Adjust the scenery behind the character portrait.** The user clarified that “adjustable backgrounds” means portrait scenery. Add a distinct portrait scenery control with useful setting choices and custom description. Feed the selected public scenery into the actual portrait generation context; changing it marks the prior portrait out of date and requires an explicit Generate/Regenerate action. Keep the D&D origin background selection separate. Scenery is visual direction only and does not change campaign location, story facts or character rules. Code is integrated: studio, forest, tavern, ruins, coast and custom public setting (360 characters / 768 bytes). Checked scenery participates in classification, generation fingerprints and final portrait attachment matching. Live UI/image qualification was deferred when the user requested finishing and shutting down servers.

- **REF-044 — Keep portrait routing out of player messages.** Show simple progress such as “Preparing your portrait…” and “Creating your portrait…”. Do not display Jev, supplier/model names or content-routing categories to players. Preserve those diagnostic details in developer logs. Apply the same distinction to player-facing provider/configuration failure messages without changing generation admission or routing. Pending, ready and supplier/configuration error copy is integrated and covered by focused checks; live UI qualification remains deferred with REF-043.

- **REF-045 — Distinguish body proportions from an adult nude reference.** The user clarified that the requested portrait is a nonsexual nude reference of a clearly adult fictional character. Preserve that explicit public description rather than replacing it with a clothed body-proportion request. Existing mature-content admission and supplier policies still apply; the alternate supplier is not an uncensored guarantee. Qualify the actual UI path and report observed refusal versus delivery failure accurately. Do not infer censorship from the older generic `provider_failed` log or retry through another supplier automatically.

  The proposed new live reference test was not submitted: the user subsequently requested finishing and stopping the servers. No successful nude result or supplier moderation diagnosis is claimed. Existing mature/ restrained admission and supplier safety settings were not changed.

- **REF-046 — A hero library on this PC, owned through a key held by each phone.** Heroes are saved long term in a local hero library on the host PC. Each phone browser holds a private random key; the library stores only its hash and returns to a phone only the heroes saved under that key. Before accounts exist, this is how a player gets their heroes back on a later night. A later account (REF-021) may claim these heroes only with proof of both the phone key and the account. Clearing that browser's storage loses access to its heroes until accounts exist; this limit is accepted and must be stated plainly to players.
  - Date: 2026-10-23. Rationale: the user selected “Phone key + PC library” over a shared shelf or per-hero recovery codes.
- **REF-047 — Only a hero's identity carries into another campaign.** A hero brought into a different campaign keeps name, public look, approved portrait, backstory and the creator's design choices. The new campaign sets level, equipment, and campaign facts. Within one campaign, progression continues on Resume from that campaign's own save. This supersedes the progression part of owner decision O3 (2026-10-12) in [Backstory-driven character creation](character-backstory-creation.md), which had heroes carry level and progression across campaigns.
  - Date: 2026-10-23. Rationale: the user selected “Identity only” over carrying level, XP and gear.
- **REF-048 — Save heroes to the library automatically.** A hero is written to the library without a separate button when the player accepts it in the creator, and again whenever its accepted identity or approved portrait changes. Each write is a new immutable revision; earlier revisions are kept. A campaign records the hero and the revision it was created from, so later library edits never rewrite a campaign's history.
  - Date: 2026-10-23. Rationale: the user selected automatic saving. Because REF-047 carries identity only, a session save or Stop writes to the library only when the identity or portrait changed during play.
- **REF-049 — Unfinished creator drafts live in the phone's browser.** An unfinished draft (selections, free text, scenery and the latest portrait reference) is kept in that phone browser's own storage, so a reload on the same phone restores it. It does not follow the player to another phone or browser. Accepting the hero clears the draft.
  - Date: 2026-10-23. Rationale: the user selected the phone browser over server-side per-seat drafts.

- **REF-058 — Player heroes are always the character the player created.** Everywhere a player hero is shown or described (TV scene plates, canonical hero images, health variants, party and turn portraits, phone sheets, battlefield figures and kill shots), it comes from that player's accepted character sheet: name, look controls, public description, species/class/background and gear, with the player's approved creator portrait as the image reference when one exists and otherwise an image generated from that description. Stock quick-start characters (stock portraits, figure photos, sprites and locator phrases) are never used for a player hero, and the stock "Hero appearance" choice is removed from character creation. Old saves that hold a stock portrait choice still load; their display uses their own look, or the stock entry's written description as words only.
  - Date: 2026-10-24. Rationale: the owner rejected stock characters on the display and asked for the player-created character sheets (live play showed the plates using stock art numbers and dropping heroes whose look differed from the stock photo).

- **REF-059 — Every hero gets a self-portrait when the player locks in the design.** Confirming the character is the explicit action that creates the portrait: if the current design has no approved portrait, the creator generates one from the player's own sheet (the same Jev-classified route as Generate) and then commits the hero with it attached, so the portrait is the hero's sheet, party, HUD and turn image and the reference for their canonical plate figure. A portrait already generated for the unchanged design is reused, never paid for twice. If generation fails or its result is unknown, the hero still joins and every display uses the hero's canonical image from the same sheet, never a stock picture or a bare initial once that image exists. This refines REF-032: editing still never generates.
  - Date: 2026-10-24. Rationale: the owner saw a hero without a self-portrait in play and asked that it happen when the player locks in their character design.

- **REF-060 — Scene plates frame the party wider, with the heroes facing each other.** The display plate's camera sits about 5 feet (1.5 m) further back than the current framing, so the heroes and the surrounding place read as one wider shot, and the party heroes are composed facing each other (profile or three-quarter, as if in conversation) rather than all turned toward the speaker. Every hero still comes from their own character sheet (REF-058/059). The TV's live figures follow the same direction: heroes look at each other.
  - Date: 2026-10-24. Rationale: the owner asked to zoom the display plate out by 5 feet and have the players look at each other.

- **REF-061 — Hero names may be simple or complex.** A hero name is one word or several, in letters from any alphabet, up to 32 characters: single spaces, apostrophes, hyphens and periods may join letters ("Vix", "Vix Ashlantern", "D'Artagnan", "Jean-Luc", "Ælfwyn", "St. Brynn"). One rule (`df_rules::local_journey::valid_character_name`) decides for both the phone and the server, and the phone marks the name field itself, with the rule in words, as soon as a name breaks it. This replaces the earlier limit of 24 ASCII letters, hyphens and underscores; underscores stay accepted so older heroes still load.
  - Date: 2026-10-24. Rationale: the owner saw "Vix Ashlantern" refused for its space and asked for names to be simple or complex.

- **REF-062 — A table seats six heroes.** The lobby, seats, creation, scene plates, party lists on the TV and phones, and fights take up to six player heroes instead of four. Plates frame all six from their own character sheets (REF-058/060).
  - Date: 2026-10-24. Rationale: the owner asked to enable six characters on the home screen, not just four.
- **REF-063 — Every SRD 5.2.1 species is playable as a Fighter.** Dragonborn, Dwarf, Elf, Gnome, Goliath, Halfling, Human, Orc and Tiefling can be created with the Fighter/Soldier build, with the species choices the source requires (ancestry, lineage, legacy, Human skill and Origin feat). Traits the engine can resolve are implemented from the source; traits that need machinery it lacks (spellcasting, conditions and their saves, Heroic Inspiration, flight, Large Form) are shown on the character sheet as "Not in play yet" and recorded as coverage gaps, never approximated (rules-support.md). Aasimar is not in SRD 5.2.1, so it stays a portrait-only choice until a licensed source is pinned. In-game images describe each species' anatomy, not just its name.
  - Date: 2026-10-24. Rationale: the owner asked for six characters of different races; the coordinator chose source-faithful traits with explicit gaps over a Dwarf-only table or approximated traits.

## Character storage: per campaign and long term — design, 2026-10-23

Governing decisions: REF-021, REF-026, REF-031 and REF-046–049. Governing documents: [Storage architecture](storage-architecture.md), [Runtime reliability](runtime-reliability.md), [RPC API](rpc-api.md), [Client architecture](client-architecture.md). This section is the implementation design for those decisions; items marked *reading* are the coordinator's interpretation, open to correction.

| Layer | Where it lives | What it holds | Lifetime |
| --- | --- | --- | --- |
| Draft | Phone browser `localStorage`, one key per phone (`df.creator.draft.v1`) | Unaccepted selections, public look text, scenery, latest portrait reference | Until Accept or the player discards it (REF-049) |
| Campaign hero | The saved game's own PostgreSQL database (existing checkpoint) | Accepted hero, campaign progression, gear, campaign facts, plus the library hero ID and revision it came from | As long as that saved game exists |
| Library hero | A separate `df_gameplay_demo_library` database in the same local PostgreSQL cluster | Phone-key profiles (key hash only), heroes, immutable identity revisions, portrait bytes | Long term, across all saved games (REF-046) |

- **Portrait bytes move into the library database.** Today portraits sit in `artifacts/tmp/play/imagery-cache`, a cache that cleanup may delete. A library revision stores the approved portrait's bytes and SHA-256 in the library database, so a long-term hero never depends on a cache file.
- *Reading:* species, class and background are part of the identity that carries (REF-047), because they are the creator's design choices. Rolled ability scores (REF-030), level and equipment are set per campaign. A carried design that the new campaign's rules cannot play yet stays saved and is refused at finalization with the existing "rules for this combination are not ready" message, never substituted.
- **Phone key.** 32 random bytes generated in the phone browser on first save and kept in `localStorage` (`df.library.key.v1`); the server stores only its SHA-256. The key travels on library calls over the existing gRPC-over-WebSocket transport and never appears in a URL, the TV view or logs. On the LAN this is unencrypted HTTP, which is acceptable for local play and is recorded as a limit for hosted play.
- **Calls.** List my heroes (name, portrait, design summary), Load a hero into the creator draft, and Save the committed hero of my seat. Save reads the hero from the campaign's committed state on the server; the phone sends only its seat credential and library key. Saves are idempotent by profile, campaign, seat and identity revision; a phone that was offline at Accept saves when it reconnects.
- **Failure handling.** A library outage never blocks creating or playing a hero in a campaign; the creator shows "Not yet saved to your heroes" and retries on reconnect. Private and sealed backstory are stored in the library under the owning key only and never sent to the TV or another phone.
- **Dev CLI.** `dev heroes list`, `dev heroes show <id>`, `dev heroes revisions <id>` and `dev heroes delete <id>`, writing their audit lines through the shared path.

Out of scope for this slice: accounts and claiming, hosted play, face capture, renaming saved games, and carrying anything beyond identity.

## Desktop launcher — refinement and implementation, 2026-10-23

Governing decisions: REF-002–012 (local launcher), REF-014 and REF-017 (sign-in and setup, still partly open). Implementation: `crates/df-launcher` (`dungeonflux.exe`). This section records the decisions made while building it and what was verified.

- **REF-050 — The launcher is a desktop app on Tauri's window and webview layers.** `dungeonflux.exe` uses tao and wry directly (the window and WebView2 layers Tauri is built on), because Tauri itself is not in the offline crate cache while wry and tao are. Its pages are server-rendered HTML with plain forms, served to its own window through a private `dungeonflux:` scheme; the window runs no JavaScript.
  - Date: 2026-10-23. Rationale: the user asked for "tauri or whatever" desktop app; the project requires Rust application code and offline builds.
- **REF-051 — The table display opens in the launcher's own window.** This refines REF-004: by default the display opens in a DungeonFlux window (on a second screen when one is connected, full screen there), and a launcher-managed WebView2 window lets the game's sound play without the browser's autoplay block. The default browser and "don't open it" stay available in Settings. The game server stays a separate process, so phones keep their connection and closing the launcher leaves the game running (REF-005). For development and testing the launcher can attach to a server started elsewhere (Settings, Developer, or `--connect`) instead of starting its own.
  - Date: 2026-10-23. Rationale: the user asked for the display in the launcher's own webview if feasible, always configurable for development and testing. Feasibility was verified: the full table display (WASM, WebSocket RPC, scene art) runs in the window.
- **REF-052 — One central configuration file.** `config/dungeonflux.toml` holds every launcher setting and the provider keys and is ignored by Git; `config/dungeonflux.example.toml` is the clean copy in Git, rendered from the defaults with a comment on every setting (a test keeps the two identical). The launcher creates the real copy on first run and the Settings page saves into it, keeping comments. Keys missing there may still come from the game folder's `.env`. Machine state (saved-games list, running-game record, screenshots, webview data) stays in `%APPDATA%\DungeonFlux`.
  - Date: 2026-10-23. Rationale: the user asked for a central config file with a clean committed copy and a gitignored copy holding the keys.
- **REF-053 — Saved games show the table as it was last played.** The launcher saves a picture of the table display window (WebView2's own capture, JPEG) when the game is stopped from the launcher, when the display window is closed, and every two minutes while it is open. The most recent game is shown as a large Continue card with its picture; the others as rows with thumbnails. Games never shown in the launcher's window use the adventure's artwork.
  - Date: 2026-10-23. Rationale: the user asked for a screenshot on exit, shown for resumption.
- **REF-054 — Agents and scripts skip the launcher screen.** `dungeonflux start [harbor|random]`, `resume <name>`, `stop [--force]`, `status` and `saves` use the same saves, settings and running-game record as the window. `--dev-control` prints the server's developer-control address and token file; `--to <phase>` then drives the game to creation, opening, dialogue, combat, complete or ended with `df-transport-fixture dev play --to`. A game started this way appears in an open launcher window, and one started in the window can be stopped this way.
  - Date: 2026-10-23. Rationale: the user asked for the dev CLI to bypass the launcher screen via the agent.
- **REF-055 — Before Start, the launcher says what the game will include.** A short list under the adventure choice says whether the AI storyteller, voices and music, phone joining and the display window are on, and links each to the setting or key that changes it. Moving between Play and Settings plays a short transition: the brass tab slides, the leaving page dims and sinks, and the arriving page is revealed by a widening circle of lantern light from the tab (CSS view transitions). Settings, Table display, Launcher animations chooses Lively (the default, the full effect whatever Windows says), Gentle (short fades) or Follow Windows (gentle when Windows' Animation effects are off, which browsers report as reduced motion).
  - Date: 2026-10-23. Rationale: the user asked for a clearer pre-game screen and a fun Play/Settings transition. Revised the same day: the transition never showed on the user's PC, because Windows' Animation effects are off there and the first version removed every animation under reduced motion, because arriving at Play skipped the transition (the opt-in sat in the external stylesheet, which a short page could paint before), and because the reveal circle was sized to the tall Settings page. All three are fixed and the choice is a setting.
- **REF-056 — Agents read and write every setting and window property.** `dungeonflux config [get|set|reset]` covers every setting in the central file by name, with the Settings page's rules; keys show only as set or unset and are read from standard input. `dungeonflux window [get|set]` reads and changes the open launcher's control and table display windows (visibility, size, position, page, full screen, screen, screenshot) through a loopback line guarded by a random token in the data folder. Every launcher action is logged by name, never by value, to `<data>/logs/launcher.log` (`dungeonflux logs`).
  - Date: 2026-10-23. Rationale: the user asked for the dev CLI to reach all settings and window properties, readable and writable by the agent, and for logs on every part of the launcher.
- **REF-057 — One gate per paid capability, and keys only for what is on.** Each paid capability (storyteller, memory, voices, music, speech input, pictures, plates, creator portraits, home, scene and character video, Jev's reading of players, and World Labs 3D places) has one switch, read once by the server's `Gates` structure and asked by every generation site; nothing reaches a provider around it. The launcher also withholds every key no enabled feature needs, so a feature turned off cannot reach its provider even through an older server. Video, creator portraits and 3D places are off by default. The Play page never mentions keys.
  - Date: 2026-10-23. Rationale: the user asked that a capability turned off generates nothing, with no leaks, through a single gate structure.
- **REF-064 — While a game runs, the Play page is about that game alone.** The saved-games list is hidden while a game is starting, running, stopping or connected. The running game sits in the centre under the lit lantern: its name, "running since", the room code on a brass tablet, then table display, how players join and the day it was first played, then the display actions and Stop. The hall photograph drifts slowly behind a pool of lantern light, with warm motes and harbor fog, and the lantern's flame flickers like a candle. All of it is CSS only, and the Gentle and Follow Windows motion settings keep it still.
  - Date: 2026-10-24. Rationale: the owner asked for the running game's details in the centre with interesting graphics and subtle ambient animation, no saved games while a game runs, and a flickering lantern.
  - Refined the same day: the running page shows two live clocks (this session and all sessions, counting by CSS alone; each saved game now keeps its total play time) and the heroes at the table with portrait, name and health, plus where the story is, read from a token-guarded `/gameplay/launcher-status` route on the game server and shown in a frame that refreshes itself. Every Play page answers the pointer and keyboard (lifts, presses, a light sweep on primary buttons, focus rings that settle in and drift out, cards and saved games that lean in, one staggered entrance), and the idle, starting and stopping pages share the dust and haze. Rationale: the owner asked for session time, total time and current players, animations all over the landing page, and the dust and haze everywhere.

Verified on 2026-10-23 with a provider-free test configuration (no keys, loopback only, separate database port and data folder): start, stop and resume through the launcher; closing the launcher process leaves the game serving and a reopened launcher finds it and stops it; the table display rendered in the launcher's window with no console errors; closing the display window saved a screenshot; `dungeonflux start --dev-control`, `--to creation`, `status`, `saves` and `stop` against real servers, including piped output; Play and Settings pages at 375, 800 and 1100 pixels with no horizontal overflow; `dungeonflux config` and `dungeonflux window` against a live window (size, position, page, visibility, display open and close, display screenshot) and refusals of bad values. 41 unit tests and Clippy with warnings denied pass. The transition was checked frame by frame in a headless Edge with animations slowed down, both directions, with Windows' reduced motion emulated: eight of eight Play and Settings changes animate, and Gentle and Follow Windows run the fades. The generation gates were checked live with fake keys for every provider: started through the launcher with every feature off, the server received no key variables; started directly with every key present and every switch off, its `GATES` line and `dev gates` reported all 13 capabilities off and no provider call appeared. Not verified: the transition inside the WebView2 window itself (the same engine, version 155), and a game with paid providers.

Known limits: resuming a save more than about five minutes after its server stopped is refused by the server (SES-17 in [issue triage](issue-triage.md)); account sign-in (REF-014) is not implemented because no provider is chosen; the launcher does not build the game, it finds an existing build (newest under `artifacts/build` unless set in Settings); ending the launcher's whole process tree also ends the game (LCH-01). The crate is its own Cargo workspace with its own lockfile for now, so adding it did not change the shared `Cargo.lock` while other work is in flight; build it with `cargo build --manifest-path crates/df-launcher/Cargo.toml --target-dir artifacts/build/launcher`.

## Development run closeout

At the user's request, 22 DungeonFlux development game servers and their 22 owned PostgreSQL instances were stopped gracefully through validated per-run stop markers. No forced termination was needed; all 44 identified game/database listening ports were closed and project game/database processes were absent. Source, saved data, generated assets and evidence were preserved. No new preview or supplier image request was started for REF-043 through REF-045 after the stop instruction.

Requested stale compiler-cache cleanup was blocked before execution by automatic approval review, including the retry following explicit user authorization. The only supplied reason was “blocked by policy”. No cache files were deleted; no free-space change is attributed to cleanup.

## Future decision entries

Append confirmed decisions using this format; keep unresolved alternatives in the discussion section.

- ID: REF-065 (increment for each decision)
- Date: YYYY-MM-DD
- Decision: The agreed behavior or requirement.
- Rationale: Why it was selected.
- Open follow-ups: Remaining questions, dependencies, or work to scope.
