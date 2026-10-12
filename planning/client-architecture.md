# Browser client architecture

Date: 2026-09-29
Status: Client roles and authority agreed; concrete interface/API plan added, implementation pending

## Agreed direction

DungeonFlux is a browser game with two primary client roles connected to the same
game session:

| Role | Device targets | Purpose |
| --- | --- | --- |
| Shared display | TV, laptop screen, or laptop connected to a TV | Present the game to the group |
| Player client | Mobile phone, laptop, or tablet | Display the player's view and send player input |

A client role is independent of device type. A laptop can provide the shared
display or a player client. Player interfaces must accommodate different screen
sizes and touch, mouse, and keyboard input as appropriate.

## Implementation language

The application is full-stack Rust: native Rust on the backend and Rust compiled
to WebAssembly for browser clients. Do not introduce Go, JavaScript, or TypeScript
application code. Frameworks and rendering libraries remain to be selected.

Generated JavaScript glue for WebAssembly loading and browser API access is
allowed. Application logic and UI source remain Rust; this exception does not
permit handwritten JavaScript or TypeScript application code. See the
[official wasm-bindgen browser example](https://wasm-bindgen.github.io/wasm-bindgen/examples/without-a-bundler.html).

Subsystems are separate Rust crates as described in
[Subsystem architecture](subsystem-architecture.md).
[Subsystem interfaces](subsystem-interfaces.md) and [RPC API](rpc-api.md) now define
the planned boundaries. Resolve applicable gates and freeze types before consumers.

DungeonFlux must support the standard 2024 fifth-edition D&D rules. Precise
source/catalog pinning remains a gate; see [Rules support](rules-support.md) and
[Rules coverage](rules-coverage.md) for the planned families and completion criteria.

Observability is a core design requirement across both client roles and backend
subsystems. See [Observability](observability.md) for the shared ports, RPC/query
shapes and remaining implementation gates.

PostgreSQL stores durable gameplay data. OpenTelemetry runtime logs and correlated
spans go to a separate SQLite database for agent review; the SQLite development
queue/devlog remains separate. See [Storage architecture](storage-architecture.md).

Structured client/server game communication uses RPC through the planned Rust
port of the gRPC-over-WebSocket bridge. See [RPC transport](rpc-transport.md) for
the full streaming requirement, latency goals, and browser feasibility gate.

## Agreed division of responsibilities

The server owns the game: authoritative session state, D&D rules resolution,
dice outcomes, action validation, progression, and AI/media orchestration. It
determines the appropriate view and audio for each connected client role.

Clients are thin Rust/WebAssembly presentation applications. They render
server-provided views and play server-provided audio. Player clients collect input
and send action requests to the server; submitting an action does not resolve it
locally. The shared display presents the group's view, while player clients show
their permitted personal views. The server must filter private information before
sending it, rather than relying on the client to hide it.

Client state is limited to presentation and connection needs, such as focus,
layout, view revisions, audio buffers, and loading indicators. Clients do not own
game state or duplicate the rules engine. Local rendering and audio decoding are
presentation work; game decisions remain server responsibilities.

Both roles must stay mounted through ordinary updates, navigation and reconnect,
without full-page reloads. Program presentation through reusable Rust components
and typed, bounded server-provided presentation variants/timelines; preserve smooth
interruptible motion, valid local focus/drafts and resource lifetimes. Read
[Client presentation](client-presentation.md) for the programming model, update
lifecycle, animation/recovery contract and measured browser acceptance gates.

Reconnecting clients recover the current permitted view from the server. The new
interface/API plan covers view shapes, ordering, audio timelines and async jobs;
concrete protobuf types, browser transport/codec implementation and device budgets
are prerequisites in [Implementation roadmap](implementation-roadmap.md).

That planning must cover the [runtime reliability requirements](runtime-reliability.md):
confirmed action results, initial/updated/reconnected views, timing, audio
cancellation and playback, and private player projections. The first integrated
slice must run through both client roles in a real browser, including a rejected
action and reconnect. Choose a supported-device matrix before claiming compatibility;
desktop automation alone does not prove phone sleep/network or audio behavior.

## Decisions still open

- Rust frontend/backend frameworks, rendering, and RPC bridge implementation.
- Concrete credential bootstrap/persistence, configured player limits, and
  deployment account policy; session/join/resume/binding semantics are now planned.
- Detailed host-panel layout; host is a capability attached to an authorized
  player/display client, not a mandatory third device role.
- Supported browsers, including whether direct smart-TV browsing is required.
- Shared-display layout, responsive player layouts, and server-controlled audio
  routing and synchronization.
- Exact rulebook/content catalog, launch campaign, PostgreSQL physical schemas,
  and durable media deployment; interfaces/recovery semantics are now planned.

Agents must reference this document for client roles and device targets. Proposals
and open decisions above must not be treated as approved implementation contracts.

## Display stage machines

Status: implemented for scene, line, voice clip, actor, battle and the hero speech-intent loader
(PLAY-F06 M1-M2, PLAY-N04 M3-M4).

The shared display presents through closed (state, event) tables in `stage_machines.rs`,
owned and driven by the conductor in `stage_conductor.rs`; nothing appears, changes or
disappears except through one of them. An event that is illegal in a state is ignored, never a
panic. All bounds are named constants at the top of `stage_machines.rs`. Display presentation
only: the server still decides every outcome and phones are never delayed.

| Machine | States (DOM attribute) | Notes |
| --- | --- | --- |
| Scene (`data-scene-state`) | Hidden, Entering, Presenting, AwaitingChoice, Exiting | A new view only sets the pending successor (`data-stage-pending`); the swap is in Exiting, after the lines on screen. Choice chips are visible only in AwaitingChoice. |
| Line (`data-line-state` on the stage, the narrator box and each bubble) | Queued, Opening, Speaking, Lingering, Closing, Done | At most one line in Opening/Speaking/Lingering. A voiced line opens when its clip starts; an unvoiced one reads for `reading_time_ms`. |
| Clip (not on the DOM) | Silent, Requested, Ready, Playing, Ended, Failed, Skipped | Driven by the voice player's Loaded/Started/Finished/Error; watchdogs end a lost signal. |
| Battle (`data-battle-state`) | Absent, Entering, TurnStart, Acting, Resolving, TurnEnd, Finishing, Exiting | In front of the battlefield's own animation. Entering covers the scene change, the 3D battlefield mounting and its banner, initiative roll and roll-call; dice that arrive meanwhile are held and play after it, and the flat combat view stays hidden (when WebGPU is present) so it never flashes before the 3D view. A battlefield that will not appear, or `BATTLE_ENTER_CAP_MS`, releases it. |
| Actor (`data-actor-state` per figure) | Offstage, Entering, Idle, Speaking, Exiting | Speaking mark and stage lighting follow the line machine. Actors absent from the next scene exit before the backdrop changes. |

Transitions (scene): Hidden -Restore-> Presenting; Entering -EntranceDone-> Presenting;
Presenting -LinesDone-> AwaitingChoice; AwaitingChoice -LinesResumed-> Presenting;
Presenting|AwaitingChoice -ExitBegin-> Exiting; Exiting -ExitDone-> Entering.
Line: Queued -Begin-> Opening -OpenDone-> Speaking -SpeechDone-> Lingering -LingerDone-> Closing
-CloseDone-> Done; Opening|Speaking -SpeechDone-> Lingering; Opening|Speaking|Lingering -Close->
Closing; Queued -Close-> Done (never shown). Clip: Silent -Request-> Requested -Loaded-> Ready
-Started-> Playing -Finished-> Ended; Error/WaitExpired end any waiting state as Failed (a
playing clip as Ended); Cancel gives Skipped. Actor: Offstage -Cast-> Entering -EnterDone-> Idle
-LineOpened-> Speaking -LineClosed-> Idle -Leave-> Exiting -ExitDone-> Offstage.

Catch-up: a newer view never cuts speech. One waiting scene change lets the remaining lines
play; two or more shorten the remaining lines to `CATCH_UP_TEXT_MS` of text and converge on the
newest scene; no wait exceeds `SCENE_WAIT_CAP_MS`. Outcomes (dice, defeat, a fight's start or
end) are not held behind speech: the line in progress fades within `FIGHT_LINE_FADE_MS` and its
clip is cancelled. The cue sequencer (`stage_director.rs`) keeps the transition and outcome
timings behind the conductor. A page whose animation frames are throttled still advances on a
100 ms fallback timer.

Battle transitions: Absent -Begin-> Entering -EntranceDone-> TurnStart -ActBegin-> Acting
-ResolveBegin-> Resolving -TurnClose-> TurnEnd -TurnOpen-> TurnStart; a defeat on the last blow
-Finish-> Finishing -FinishDone-> Exiting -ExitDone-> Absent. A fight that starts while a line is
spoken fades that line within `FIGHT_LINE_FADE_MS` (chosen over letting it finish under the
banner) so outcomes are never held behind story speech.

Later additions. Line states Composing -> Submitted -> (replaced by the hero's own line) and
Composing|Submitted -> Cancelled -> Closing (a loader on the hero's figure, or on the HUD card in a
fight): Composing -Submit-> Submitted, -Cancel-> Cancelled, -Replace-> Done, -Close-> Closing; a
loader never takes a turn, expires after `INTENT_EXPIRY_MS` without the table affirming it, and
leaves with its scene. The signal is ephemeral (`DialogueService.Compose`, `JourneyView.composing`
17, display audience only), kept in the server's table log, never committed.

Voice rule: a reply's bubble opens when its clip starts, a clip that is coming is waited for up to
`CLIP_READY_WAIT_MS` (the thinking mark stays meanwhile), and a clip that failed or was skipped is
reported as `data-clip-state` rather than looking spoken.

Fight entrance and exit. The entrance (scene change, the battlefield mounting, a scan's splat
streaming in, banner, initiative roll and roll-call) always plays in full in front of the audience,
whoever wins initiative: an enemy's opening turn carried by the starting view is held and staged
as ordinary beats after the roll-call, nothing of the fight (including sounds) starts while the
cover is up or the splat is incomplete, and there is one transition into the fight. The exit is
queued behind the finishing blow: dice, impact and fall play, the battlefield finishes its own
finishing shot (at most `BATTLE_FINISH_CAP_MS`), the battlefield fades, and only then does the scene
machine take the victory or defeat scene (one transition). While the 3D battlefield is live
(`data-battlefield-live`) the flat combat screen, its dice card and its call-out bubbles never show.

Tactical fights. A tactical fight (server-owned cells, several enemies, enemy turns resolved by the
server between two hero actions) enters and exits through the same battle machine as a one-enemy
fight. Every outcome a view adds is staged, oldest first (`presentation::new_outcomes`), each as a
blow-only cue set (`ViewDiff.blow_only`: dice, impact, fall, and no redraw of the view, which
belongs to the newest turn); blows are outcome-bearing cues, so a display that is behind shortens
them and never drops them. A turn that ends on the server's next combatant reopens `TurnStart` once
the beats are over. A fight that ends on an enemy's turn (a defeat) exits through
Finishing > Exiting > Absent exactly as a victory does. A hero's opportunity-reaction prompt is part
of the view the display receives, so staging simply waits on the server's resolution. A scene's
own narration is told once: narration that changes inside a scene already on stage (a look's
result) is told as a new narrator line (the appended part when the told text grew), a reply to table
talk never replays it, and while a line presents the choice chips are hidden. `data-chips="off"`
on the stage hides the chip row and gives its space to the narrator box. A session view may be as
large as one RPC message (`RPC_MESSAGE_BYTES`); a smaller bound silently broke the acting hero's
phone, whose tactical view is the largest.
