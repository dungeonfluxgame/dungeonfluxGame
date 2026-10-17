# Gameplay feel and tempo engine

Date: 2026-09-30
Status: First-class planned Rust subsystem; policies/device budgets uncalibrated

`df-experience` recommends macro pacing. `df-tempo` controls moment intensity.
`df-presentation` converts permitted semantic events/profiles into concrete plans;
`df-media` supplies approved assets/audio. Thin Rust/WASM clients interpolate,
render and play these plans in persistent mounted shells under
[Client presentation](client-presentation.md). They do not infer game danger or
run tempo/game policy independently.

## State and server output

`TempoState` persists normalized tension, energy, danger, mystery, urgency, triumph
and dread; rising/falling/stable momentum; rest/build/anticipation/reveal/conflict/
peak/release phase; target/curve anchors and policy version; recent impulses and
per-effect fatigue/refractory history. All values/units validate bounded ranges.
The supplied example increments/BPMs are tuning ideas, not universal targets.

`TempoEngine::advance` takes explicit presentation time and permitted inputs from
committed narrative/encounter/combat/world/interaction events, known player state
and `PacingRecommendation`. It returns a target `TempoFrame`, bounded curve/phase
change and `TempoCueIntent`. Continuous server-owned policy uses inertia, deadbands,
rate limits and hysteresis to avoid jitter; a named, approved shock may spike and
then decay within caps. Use scheduled boundary decisions and timestamped curves,
not 60 RPCs/DB commits/LLM calls per second. Clients evaluate cosmetic interpolation
against the server anchor and clamp capability/accessibility limits; they cannot
choose a new target, phase or rules expiry.

Frames/profiles bind session/run, presentation epoch/sequence, policy version,
start/timebase, duration/curve, audience scope and fallback. Current profile/active
cues are available in the full Watch snapshot; repeats do not replay impacts.
Visibility/reconnect resyncs current state, rejects old epochs, skips expired
one-shots and clears replaced resources. Audio Mix/Cancel messages preserve ordered
track/epoch/timeline semantics and actual playback reporting.

## Presentation profile and cues

| Channel | Typed parameters and constraints |
| --- | --- |
| Music | approved theme/stem IDs, per-layer gain, bounded crossfade, optional musical boundary, reveal duck/silence/stinger; cache campaign/NPC/faction motifs, no fresh song each tick |
| SFX | semantic library cue, density/priority/reverb/impact/ducking; ordinary effects use prepared/procedural-approved fallback without blocking a turn |
| Camera/screen | permitted framing, movement/zoom, contrast/vignette/particles and bounded optional impact treatment; no secret entity focus |
| Lighting/environment appearance | cosmetic shader/palette/fog treatment within allowed scene representation; real weather/visibility/collision changes require world/rules decisions |
| UI | supported emphasis/transition/layout variants preserving readability, stable controls/focus and all legal options; no fake countdown or shortened rule timer |
| Narration | approved cadence/pause/performance hints for future validated clauses; preserve wording/meaning/captions and accepted outcome |

Screen shake is an event impulse; tempo sets a ceiling. Fatigue budgets,
refractory windows, novelty rotation and rare peaks prevent continuous shaking,
flashing, maximal music or repeated stingers. All channels have count/amplitude/
duration budgets and priority conflict/interruption rules. Speech has protected
intelligibility; ducking/release changes are tied to actual narration timelines,
not guessed provider latency. Music layers must share compatible timebases/stem
metadata, with explicit fallback when they cannot be synchronized.

Cosmetic intensity never blocks accepted outcomes or necessary input, hides a
legal option, causes layout to jump under a finger, or turns decorative urgency
into a mechanical restriction. A real countdown comes only from an approved
server timer policy. Fictional NPC/environment choreography is a world/interactions
proposal when it changes actual state; a cosmetic animation cannot become fact.

### Place music

Each place has its own score, decided by the server and generated on demand.
Implementation: `crates/df-tools/src/gameplay/place_music*` (server),
`music_score.rs` (shared rules) and `gameplay_browser/live_music.rs` (display player).

- **MusicSpec.** Mood, energy, setting family, time of day, interior or exterior,
  instrumentation palette, an optional ambience description and an optional
  fallback bed, all from closed labels plus bounded author text. Only two things
  may decide it: the pack's `music` block for that place (`pack.json`, keyed by
  `place_id`), or Jev choosing among the closed labels from the place's content and
  public scene facts. There is no keyword table, name match or asset-name guess
  anywhere in the decision (a test reads the decision code to keep it so). With
  neither a block nor a Jev answer the place has no spec: the shared score plays
  with no ambience bed and the plan records why.
- **Set.** Four layers (bed, motif, tension, combat) from `/v1/music` at the shared
  score's 84 BPM, one key chosen from the mood (relaxed and warm places get a major
  key) and 16 bars, plus one ambience bed from `/v1/sound-generation`. Each layer is
  cut to exactly 2,016,000 frames, made seamless with an equal-power tail crossfade
  (bed 2000 ms, motif and combat 714 ms, tension 450 ms) and set to -23.4 LUFS
  (ambience -30); the ambience loops 24 s. The loop and the BS.1770 loudness are
  plain Rust; only MP3 decode and encode use `ffmpeg` (`DF_FFMPEG` or `PATH`). The
  set is a `MusicSet` resource of the generation hub (`gameplay/resources.rs`), keyed
  by the canonical spec and the generator version; the hub owns the registry, the
  single in-flight request per key, the timeout and attempt limit, the failure memory,
  the spend ledger and the `RESOURCE` and `SPEND` audit lines. Place music reaches it
  through `place_music::hub::MusicHub`, implemented over the hub by `HubMusic`
  (`place_music/resource.rs`), which also keeps the MP3 bytes on disk under
  `<cache>/sets/<key>/` (default `artifacts/tmp/play/music-cache`) and serves them at
  `GET /gameplay/music/{key}/{bed|motif|tension|combat|ambience}` for keys the hub
  reports ready. A set found on disk is not generated again in a later run. Nothing
  is generated ahead of time: the first view of a place requests its set. One set
  costs five provider calls, recorded on the ledger (unit `MusicCall`). There is no
  budget, cap or limit on place music: nothing refuses a set for cost. One attempt per
  set means a failed set stays failed until a developer regenerates it.
  Timeouts: 120 s per provider call, 300 s for the whole set (every call and ffmpeg
  run gets only the time that remains), 330 s for the hub's own bound on the job.
- **Fallback.** While the set is pending, failed or unavailable the plan is FALLBACK:
  the shared score plus the existing bed the spec names as fitting (or none). The
  display switches to the place's layers at the next loop boundary of the running
  layers with the usual 4 s crossfade, and back to the shared score if any of the four
  fails to load. Fights keep the shared battle suite.
- **Wire.** `DisplayGameplayView.place_music` (`PlaceMusicPlan`): spec label, setting
  family, state, set key, the four layer paths, the ambience path, a fallback note and
  the bpm, loop length and sample rate. A client that cannot match the shared score's
  timebase, or sees a path outside the served music routes, keeps the fallback. The
  tempo engine still decides the mix; the plan names only which files are mixed.

## Audience, preferences and observations

Construct inputs/profiles from audience-permitted facts or a proven public cue.
A secret boss, unrevealed threat-clock stage, NPC belief or another player's hidden
HP cannot affect public music/lighting/tempo in a revealing way. Test paired states
that differ only in hidden data: unauthorized profile/cues/assets should match.
Player-specific known danger can have a separately routed permitted profile.
Projection filters before serialization/prefetch manifests, not just in a widget.

SetPreferences stores authorized reduced-motion/flashing, audio limits/captions
and readability preferences. Server plans honor these; clients clamp further to
safe supported capability settings without changing mechanics. Reduced-motion
means no screen shake/forced camera punches and an equivalent readable emphasis.
User-selected volume/mute remains respected; avoid coercive loudness. Actual
amplitude/contrast/flash/latency budgets need G12 device acceptance rather than
invented numerical safety claims.

Experience uses committed activity windows, voluntary participation/feedback and
explicit permitted reports. Inactivity is not proof of boredom, nor laughter proof
of low tension. No covert microphone monitoring, emotion inference or automatic
social penalty from local muting/audio cancellation. A capture/performance report
is an observation under current lease, not an NPC action. Any barge-in interaction
requires an authenticated explicit event and server validation under voice policy.

## Acceptance and optimization

F42–F44/G12 validate calm/build/anticipation/reveal/conflict/peak/release, bounded
shock/decay, fatigue/rotation, known versus hidden inputs, speech/music ducking,
stale/repeated cue rejection, reduced motion, stable legal controls and present-time
versus game-time separation. Test slow assets/provider failure/paused campaign,
late join, sleeping phone, reconnect mid-crossfade and obsolete-buffer cancellation.
Benchmark policy time, actor wakeups, snapshot bytes, frame timing/audio drift,
resource retention and actual video/prefetch spend. Frontier vision/computer-use
plus audio observation checks the running two-role result; policy tests alone do
not prove smoothness or pleasant gameplay. Tune from measured session feedback
without pretending heuristic values predict fun with certainty.
