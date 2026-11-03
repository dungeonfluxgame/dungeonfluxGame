# Persistent, programmable browser presentation

Date: 2026-09-29
Status: Required client behavior; framework and measured device budgets remain gates

## Programming model and authority

The TV/shared-display and player applications are persistent Rust/WASM shells.
Normal game updates, phase changes, navigation, overlays, late assets and reconnect
must update the mounted application without a full-page reload. Initial loading
and an explicit deployment/recovery reload are separate operations, never a normal
response to a game update. Deployment activation is explicit and must not silently
replace a build in an active session or playtest.

Program the presentation through reusable Rust components, typed view models,
layout/theme settings and scene/animation primitives in the existing browser
crates. Components can be refined behind their public contracts independently of
game rules. Keep application and UI source Rust; generated WASM loading/browser
binding JavaScript is allowed. Framework selection stays open until G01/G04/G08.

Server views can select supported presentation variants and provide bounded typed
scene, camera, caption, transition and timeline instructions. Use an explicit
allowlist of implemented variants with schema versions and validation limits;
do not introduce arbitrary executable server code, scripts, expressions, CSS/HTML
injection, or an unrestricted UI interpreter. New component behavior requires a
Rust client build; existing variants and content can change through server views.
No new crate, independent presentation RPC service, or gameplay scripting runtime
is needed for this requirement.

The server supplies permitted data, legal offers, committed outcomes and audio.
Clients may interpolate a supplied movement path or animate an already resolved
roll, but never calculate legal movement, collision, dice results or turn timing.
Local tabs, expanded panels, focus, draft input, connection status and animation
progress are presentation state, with no authority over the game.

## Update and resource lifecycle

`df-web` owns one mounted shell and its connection/audio/resource lifetimes.
`df-client` accepts only ordered views from the active binding generation;
`df-player` and `df-display` convert those views to their role-specific composition.
`df-ui` owns reusable components/layout tokens, `df-render` scene presentation,
and `df-audio` playback/capture. See [Browser interfaces](subsystem-interfaces.md#browser-interfaces).

Start with complete server snapshots and update local presentation incrementally
using stable entity/component keys. A complete wire snapshot does not require
remounting the page or every component. Reconcile changed values and retain valid
focus, scroll, drafts, subscriptions and decoded resources. Clear state whose
member/run/permission ownership is obsolete; never preserve another member's
private data or submit an invalidated offer. Rejected drafts remain editable.
Delta encoding is optional later under the [RPC view contract](rpc-api.md#views-and-reliable-control).

Every mounted scope owns its listeners, subscriptions, pending asset/decode work,
animation handles and render resources. Update replaces or cancels only affected
work; unmount/dispose releases it exactly once. Asset completion is checked against
the active generation and resource key before publication. Keep caches and GPU,
decoded audio and pending-update memory bounded; repeated same-phase updates must
not accumulate layers, handlers or buffers.

Routes, panels and overlays update in place with touch, keyboard and mouse input,
responsive layouts and explicit focus restoration. Host controls remain permission
gated. Route selection cannot create permissions or initiate a game mutation by
itself. A failed optional renderer/asset uses the permitted flat/still fallback
while the shell, input and error surface stay usable.

## Motion, time and interruption

Animate on the browser frame clock with elapsed time rather than frame counts.
Server timeline anchors determine the current point in game-linked cues; the
monotonic local clock estimates presentation progress and never advances game
state. Synchronize captions/visual cues with observed audio playback where needed,
report drift/stalls, and expose blocked audio with a user-gesture unlock flow.

Each cue has a stable ID, presentation epoch, start/offset/duration and explicit
completion/interruption policy. Superseding updates retarget an ongoing visual
from its current rendered state when appropriate, or cancel it explicitly. Never
queue an unbounded transition backlog or delay authoritative input/results until
a cosmetic animation finishes. Cosmetic effects can be reduced before input or
essential information is degraded.

Coalesce superseded view snapshots before rendering while preserving reliable
control/cancel and audio semantics. A frame renders the newest accepted state;
bounded heavy decode/render preparation must not monopolize input processing.
Reduced-motion preferences remove nonessential motion while preserving outcomes,
captions and usable navigation. Avoid mandatory flashing effects.

On hidden-tab suspension, stop unnecessary rendering and bound buffered work.
On visibility return or reconnect, use the current permitted snapshot/timeline
and seek, skip or cancel cues according to their policy. Do not fast-forward an
unbounded backlog or replay completed one-shots. A new presentation epoch clears
obsolete cues/audio; the shell survives reconnect, with a visible connection state
and only still-valid local drafts retained.

## Acceptance and measurement

G04 records real supported browsers/devices, including whether direct smart-TV
browsing is supported. G08 sets measured input-to-render, frame-time tail, memory,
decode/upload and audio-drift budgets for representative devices. Choose refresh
targets and permitted degradation after measurement; no universal frame-rate or
phone/TV compatibility claim follows from a desktop viewport.

S01 must demonstrate both roles updating and navigating in place, retained input
focus/drafts across valid updates, and reconnect with no page reload or duplicated
listeners. S03 adds actual timed audio/caption observation, blocked audio unlock,
late asset fallback and interruption. S04/S07 exercise overlapping movement,
scene/overlay transitions, reduced motion and supported renderer fallbacks.

Use controlled-clock boundary checks for ordering, cue dedupe/cancellation,
visibility recovery and disposal, plus a frontier evaluator operating and visually
reviewing both real browser roles. Measure representative repeated updates and
simultaneous assets/audio/input; retain build/device identities, frame-time and
memory results with interaction evidence. Observe actual audio when relevant.
Client OTEL reports bounded update/render/decode costs, long frames, active cue
counts, fallback reasons, timeline drift and input-to-render correlation without
logging private view payloads. Unperformed device or playback checks stay pending.

After initial integration, optimize every affected crate through
[Optimization](optimization.md), preserving these contracts and the server's
authority. This document specifies planned behavior; no client runtime exists yet.

## Concept render analysis (2026-11-02)

Source: the 38 renders and the contact sheet in `assets/concept-art/` (`source-manifest.json`). They are
conceptual mockups: they guide the look and add no gameplay requirement. Pixel values below were sampled
with Pillow (median of a small box, or the mean of the brightest few percent for text and glows).
The building blocks are in `crates/df-playground` (`overlay*.rs` model, `view_*.rs` builders) and the
tokens in `assets/playground/theme.css`; fixtures `ui_*` and `scene_ember_roads` exercise them.

### Component inventory

| Component | Renders | Data it needs | States |
| --- | --- | --- | --- |
| Narration box (DM speaker, text, continue, voice mark) | 02, 13, 16-20 boards | speaker, text, continue label, scene progress | with or without continue; dwell note |
| Seat or party card (portrait, name, class chip, HP bar, conditions) | 02, 03, 13 | name, class, level, portrait, HP, conditions, accent | hero, open seat |
| Lobby seat card | 08, 11, 21, 24, 25, 37 | name, class, level, portrait, state, progress | locked in (ready), creating, open, closed |
| Invite panel (QR block, join address, code, N of 6) | 08, 11, 21, 24, 25 | address, code, ready counts | n of 6 |
| Begin button and state legend | 08, 11, 21, 24 | label, enabled, note | enabled, waiting |
| Merchant panel (tabs, item grid, detail card, phone prompt) | 06, 10, 15, 16 | tabs, items (name, glyph, price, rarity), selected, detail, prompt | selected tile, rarity |
| Chest panel and party inventory quick view | 06, 10, 20 | contents, selected, per-hero load | selected, load bar |
| Item inspection | 01, 07, 17, 20 | title, description, rarity | one |
| Roleplay card (title bar, speech bubbles, caption) | 03, 04, 12, 18, 22, 26-35 | title glyph, speaker, text, tone, tail target | hero, ally, mystic, npc, narrator, danger |
| Status and balance pills | 02, 03, 04, 05, 13, 19 | kind, glyph, text, tone | neutral, good, warn, accent |
| Not built (outside the brief) | 05, 09, 19, 23, 36, 37, 38 | session settings, home, journal, loading, devices list | |

### Visual system

- Palette (hex sampled): page ink `#09131d` (plan's existing), navy glass `#14203d` to `#1e2b4f` (lobby, settings),
  warm glass `#1d1612` to `#2b2019` (merchant, narrator, bubbles `#1a1613`), brass rim `#c6a36b` with highlight
  `#f1d9a0` (button rim reads `#fef7d3`), deep brass `#8a5f2a` (selected tile border `#b47c4a`, active tab fill `#684112`).
  Locked in: fill `#0f4834`, text and check `#6fe3b8` (`#7cf6cf` sampled). Creating: progress `#a965fb`, fill `#3a1f6e`
  (card fill `#622ea7`). Open seat: ring `#c8d9ed`, fill `#1f314d` to `#394d6e`. HP green `#6cdb88`; class bars red
  `#f05a5c`, yellow `#f8d74c`, blue `#4faef7`, purple `#c78bfa`. Balance teal `#6cccd3`. Bubble rims by tone:
  hero gold, ally blue-white, mystic violet, npc rose `#e39aa8`, narrator amber, danger red (`#e1b6b4` core, glow).
- Rarity: common ivory grey, uncommon green `#7fd99a`, rare violet `#c78bfa`, legendary gold. Text on glass is ivory
  `#f2e8d5` to `#fdfbf8`; muted text `#b5c2d6` (open seat sub-line) or warm `#dcd4cb`.
- Typography: a high-contrast serif everywhere (the wordmark in small caps; titles in gold italic; bubble and narration
  text roman; captions italic). Sizes in the 1672 px render, as a share of its width: lobby title 3.4 percent,
  narration body 1.7, seat name 1.5, bubble text 1.3, pills 1.0, small captions 1.0. The ladder in `theme.css` is
  1.05 / 1.2 / 1.5 / 1.85 / 2.5 / 3.7 cqw (about a 1.25 ratio).
- Panels: dark glass with a hairline brass or tinted rim, 45 degree chamfered corners (about 0.9 percent of the
  width), a soft coloured glow outside (teal locked in, violet creating, brass selected), a 1 to 2 px inner highlight on
  top. Pills are fully rounded. Seat cards are portrait 3:4. Buttons are chamfered brass with a lit rim.
- Seat card states: locked in = teal rim, glow, portrait, crown for the host, green "locked in" pill; creating =
  violet rim, silhouette or card art, progress bar and a stage line ("Selecting character", percent); open = glass,
  circled plus, "Seat N / Open"; closed = padlock, "Locked" (renamed "closed" here, because "locked in" means ready).
- HP bars are a rounded 0.7 percent track with a green fill (amber then red as it falls, 50 and 25 percent).

### Layout at 16:9 (percent of width by percent of height)

- Lobby: title block 30 to 70 by 7 to 24; six seat cards 7 to 94 wide (each 13.5 wide, gap 1.2) by 29 to 61 high;
  invite panel 10 to 92 by 64 to 80; begin button 35 to 66 by 85 to 93; legend bottom right.
- Narrator: episode pill top right (78 to 99 by 3 to 10); narration box 16 to 83 by 52 to 72; seat strip 3 to 97 by
  76 to 87 (six cards, each about 15 wide); progress and continue 91 to 98.
- Merchant: speech bubble top left 16 to 40; item panel 40 to 78 by 10 to 49 (4 columns by 2 rows); detail card 79 to 99
  by 10 to 40; phone prompt 79 to 99 by 42 to 47; caption line 40 to 98 by 42 to 48 (render 10).
- Chest: contents panel 32 to 75 by 55 to 76; detail 75 to 98; party inventory strip 34 to 75 by 77 to 97.
- Roleplay: 2 to 4 bubbles of about 20 to 25 wide, tail pointing at the speaker; title bar top left; caption lower third.
- Item inspection: title block bottom centre (20 to 80 by 75 to 90), the item fills the plate.

### Conflicts with the plan and the owner's rules

1. Real-money balance and "$3.90/hr" pills (05, 08, 11, 19, 23, 37) and "Est. cost" lines: the public pricing shows only what
   tables pay, and no cost belongs on the shared screen. The pill component accepts a `balance` kind, but no shipped
   scene uses it; a table shows one only when the server sends it.
2. Gold prices on shop tiles are in-world currency and stay, as an optional text field the rules owner fills.
3. Seat strip, bubbles and menus drawn over the plate with the party strip absent (01, 04, 12, 14, 18): K.13 and O.14
   rule 2 keep the strip always visible. Inspection and roleplay fixtures keep the strip.
4. Bubbles positioned at faces must not cover the caption, status or party strip (O.13 rule 12): the model holds a
   bubble's tail target inside a safe area and the box grows upward from it.
5. Dev copy in player text: "Est. duration", "Spend rate", "Session code" rows, "DM engine online" session log (37) and the
   device and connection list (37) are operator information, not player copy; they are left out.
6. Minimap or world map on the shared screen (20, 22 "World Travel") is not on the TV by default; left out.
7. "Locked" and "Locked in" mean different things (closed seat versus ready); the component says "Closed" and "Locked in".
8. "Choose, buy, or sell on your phone" and "Distribute loot... on your phone" agree with the thin-client rule (the TV
   never takes input); the roleplay choice pills (04, 32) are phone choices, with at most four (K.14), and are not drawn on the TV.
9. The narrator render's "Continue" affordance: the TV does not advance by itself and the dwell rules (K.12) decide how
   long a line stays up; the continue mark is drawn only as a hint and is never a control.
10. Numbered scene titles ("3. Negotiate") belong to the storyboard, not to play; the title bar shows the scene's name.

### Built so far (narration box, seat strip, logo)

Plates carry scene art only (`ember-roads-plate.webp` is render 13 with its baked interface painted out). The narration
box, the seat cards and the logo are drawn by `view_narration.rs`, `view_seats.rs` and `view_brand.rs` from the optional
`TvScene` fields `narration`, `seats` and `brand`, and every field is live from the dev CLI: `scene narration
set|edit|clear`, `scene seat set|remove`, `scene seats open|clear`, `scene logo on|off` (`none` takes a field away;
`ember-roads-tour.txt` assembles the scene). The projection must carry `narration` (speaker, text, continue label, badge,
voice, placement, progress), `seats` (looks: class, level, portrait, glyph, accent; open seat count) and `brand`.

### Design tokens (in `theme.css`, all `--ui-*`)

Colours: `--ui-ink`, `--ui-glass-navy(-hi)`, `--ui-glass-warm(-hi)`, `--ui-panel-navy|warm|ink` (gradient fills),
`--ui-brass`, `--ui-brass-hi`, `--ui-brass-deep`, `--ui-rim-brass`, `--ui-rim-dim`, seat states `--ui-ready(-fill|-rim)`,
`--ui-creating(-fill|-rim)`, `--ui-open(-fill|-rim)`, `--ui-closed`, status `--ui-danger`, `--ui-warn`, `--ui-hp-high`,
`--ui-info`, `--ui-money`, text `--ui-text`, `--ui-text-strong`, `--ui-muted-cool`, `--ui-muted-warm`, `--ui-rarity-*`,
`--ui-tone-*`. Shape: `--ui-cut` (chamfer), `--ui-cut-lg`, `--ui-rim-w`, `--ui-rim-w-lg`, `--ui-r-chip`, `--ui-r-tile`.
Glow: `--ui-glow-ready|creating|brass|open`. Spacing: `--ui-space-1` to `--ui-space-6` (0.4 to 4 cqw). Type:
`--ui-t-micro` to `--ui-t-display`. The classes `.ui-glass` (chamfered panel from `--ui-rim` and `--ui-fill`) and
`.ui-glow` (drop-shadow in `--ui-glow`) are shared. All sizes are in cqw so the screen scales as a television; the text
pairs are checked to WCAG AA by a native test (`contrast_tests.rs`).
