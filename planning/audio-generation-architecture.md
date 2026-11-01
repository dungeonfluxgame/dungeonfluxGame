# Unified audio-generation architecture

Date: 2026-10-28 (America/New_York; evidence captured 2026-10-29 UTC)
Status: Source-backed proposal; no runtime implementation, provider qualification or audible acceptance
Task: AUDIO-SURFACE-PLAN-20261028-A1-SOL
Source: Desktop repository, HEAD `6d0ce07e1d775d8bff9f569b23826a0503e13035`, with concurrent uncommitted work.
Inventory, reachability and limits: [Audio surface map](audio-generation-surface-map.md).

## Decision

Expand `df-media` with one native `AudioService` facade for speech synthesis, batch/streaming transcription, music, ambience, SFX and deterministic audio preparation. Keep `df-audio` as the browser/device playback and capture boundary. Do not create a second crate named audio, move provider execution into the browser, or add an independent generation owner beside the Asset Engine.

This follows [Asset Engine](asset-engine.md), [subsystem architecture](subsystem-architecture.md) and [interfaces](subsystem-interfaces.md): df-media already owns media scheduling and speech contracts. Its actual implementation is currently pure scheduling/prediction, not a wired live service (`crates/df-media/src/lib.rs:1`, `speech.rs:148`). The proposed facade is a specialization of the existing AssetEngine/MediaService plan. For an audio demand, `AssetEngine.admit/status/cancel/complete/reconcile` delegates to the same AudioService job record and scheduler; it does not enqueue or reserve a second job. Retire or make `MediaService.run(MediaJob::Audio)` a compatibility delegate. Exactly one owner controls each queue slot, flight, retry, cancellation, publication and usage settlement.

A new `df-sound` crate would isolate native execution, but duplicates the approved Asset Engine ownership and requires another coordination layer while df-media already has SpeechScheduler/MediaScheduler. Prefer a native feature/target module within df-media, preserving its pure default modules. Freeze this decision in a numbered ADR before implementation; names here are proposals.

## Current architecture and the gap

```text
launcher / run-gameplay.sh / df-tools main
  -> gameplay::serve
     -> Voice::start -> outgoing DISPLAY line registration + prewarm
     |  -> resource Kit/Hub + VoiceLineSpec -> NativeSpeechProvider::speak
     |  -> Eleven /text-to-speech/{voice}/stream -> full bounded MP3 -> file cache
     |  -> voice GET delivery -> live_voice HtmlAudioElement -> stage ClipSignal
     |
     -> SpeechInput::start -> authenticated player WAV upload / cancellation
     |  -> Hub accounting -> NativeSpeechProvider::transcribe -> Eleven batch Scribe
     |  -> private final text -> editable phone draft -> ordinary confirmed input
     |
     -> PlaceMusic + ApproachModel/Jev judgments or authored music block
        -> HubMusic + MusicSetResource -> MusicGenerator
        -> FOUR /v1/music children + ONE /sound-generation child
        -> Rust loop/loudness work + ffmpeg codec -> MP3 files/manifest
        -> music GET delivery -> live_music's Web Audio graph

shipped music + SFX manifests/files -> static routes -> live_music / live_sfx
combat timeline SoundEvent -> live_sfx (separate AudioContext)
kill-shot video -> HTML video audio; home video always muted

df-media pure schedulers    df-audio PCM queue/native capture/browser fixture
       [not the execution/playback owners of the live wrappers above]
fal submit_sound -> adapter/fixture tests; no discovered gameplay caller
```

The /stream TTS URL does not establish incremental delivery. NativeHttp buffers a bounded response; `PreparedSpeechResponse::complete` returns one encoded chunk after EOF plus completion (`speech_adapter.rs:429,623,697`). Live speech downloads whole segmented MP3 clips, then plays object URLs (`gameplay_browser/live_voice.rs:53,584,798,858`). No current realtime STT caller was found. Existing `SttEventKind::Partial` is a contract variant, not such a caller (`df-provider-api/src/provider.rs:67`).

Current live speech pins Flash v2.5, English and requested MP3 22.05 kHz/32 kbps. STT pins Scribe v2, English and raw signed 16-bit little-endian mono PCM at 16 kHz inside the adapter. A declared PCM TTS enum is explicitly refused. Requested MP3 metadata is not verified decoded duration/channel information. Music pins music_v2 and eleven_text_to_sound_v2, requesting MP3 44.1 kHz/128 kbps; mastering re-encodes at 160 kbps (`eleven_music.rs:19-36`, `place_music/master.rs:27`). See the surface map for exact local limits versus vendor capabilities.

Current line keys and hub coordination are useful migration inputs, but speech identity is synthesized from line digests and fixed recovery revision (`voice.rs:1182`); STT constructs local request IDs (`speech_input.rs:417`). They do not prove real campaign authorization, durable Unknown recovery, private routing or complete asset publication. Existing local cache directories are not durable hosted availability.

## Ownership and dependency direction

```text
session/feature owner -- checked demand -------------------------+
  | semantic judgments/prompt drafting -> df-ai::TextService     |
  | accepted text/intent + current scope                         v
  +--------------------------------------> df-media::AudioService
mixed-media AssetEngine -----------------> same audio owner
                                           | one MediaScheduler/flight lifecycle
                                           | private audio backends and transforms
                                           v
                             native transport / authorized egress dispatcher
                                           |
                       df-assets immutable byte + metadata publication
                                           |
                     audience projection / RPC / authorized asset delivery
                                           v
                     df-audio capture/device/mixer/playback ports
                         ^ capture chunks          | observations
                         +---- authenticated API/session --------+
```

df-media's native audio module uses private adapters and injected consumer-owned storage, admission, usage and TextService ports. df-persistence implements durable repositories; root composition supplies local SpendLedger/MeteringPort or hosted finance adapters. Do not import df-tools into a library or create another ledger. Root composition owns credentials and hosted egress authorization; a campaign cannot load supplier keys to bypass the dispatcher.

The current native `df-audio -> df-media` dependency already exists (`df-audio/Cargo.toml:16`). Never introduce `df-media -> df-audio`. Shared audio value types belong in existing pure df-types/df-model or neutral df-provider-api contracts as appropriate; device implementations consume them. No df-ai -> df-media or df-imagery -> audio implementation dependency is needed. Mixed-media orchestration may call TextService, ImageService and AudioService; these services never recursively orchestrate each other. A narrow supplied text port can call TextService for explicit audio semantic work; it has no raw provider handle.

Move native speech/music/Fal sound adapter execution behind AudioService. Reuse suitable native HTTP/process plumbing in df-providers without retaining public executable audio factories after cutover. Shared Fal video transport can remain neutral infrastructure; sound submission is private to the audio owner. Native feature/target boundaries keep credentials, reqwest, process execution and durable repositories out of WASM/pure policy builds.

Music orchestration remains one parent job with four required layer children and optional ambience. The same owner retains each paid child outcome even when the parent fails. SpeechScheduler's existing chunk accounting/acknowledgment and MediaScheduler priority/cancellation are reused or deliberately evolved once; do not wrap them in an additional independently bounded retry/queue owner. Resource Hub remains the coordination authority during migration through an injected port. Transfer its audio registry/flights/failure memory once; compatibility Voice/HubMusic/SpeechInput facades delegate and lose execution ownership.

## Proposed typed consumer API

Freeze concrete Rust newtypes/enums, serialization, ports and protocol allocation in A0. This is a design sketch, not compilable source or new RPC numbering.

```rust
AudioService::submit(context, AudioRequest) -> AudioAdmission;
AudioService::status(context, job) -> AudioStatus;
AudioService::subscribe(context, job, cursor) -> bounded AudioEvent stream;
AudioService::cancel(context, job, reason) -> CancelOutcome;
AudioService::shutdown(deadline) -> DrainOutcome;

// Same service, distinct owned input lifecycle; not a second provider route:
AudioService::open_capture(context, CaptureRequest) -> CaptureAdmission;
AudioService::push_capture(handle, CaptureChunk) -> CaptureAck;
AudioService::finish_capture(handle, end_sequence) -> Finalizing;
AudioService::cancel_capture(handle, reason) -> CancelOutcome;

enum AudioOperation {
    Speak(ApprovedSpeech),
    TranscribeBatch(AuthorizedAudioInput, TranscriptOptions),
    TranscribeStream(CaptureSpec, TranscriptOptions),
    ComposeMusic(MusicRecipe),
    GenerateAmbience(SoundIntent),
    GenerateEffect(SoundIntent),
    Prepare(AuthorizedAudioInput, OrderedTransforms),
}
```

All operations use one AudioRequest/Result/Event envelope. open/push/finish manages a streaming request admitted through this envelope. A batch caller never gets a raw provider constructor; a streaming caller never receives a provider socket/token. Local fixtures, replay, future implementations and deterministic transforms use this API.

| Contract | Required fields and meaning |
| --- | --- |
| Context | Actual tenant/account/campaign where applicable; authenticated principal and audience permission supplied separately; session/run/recovery epoch, operation/job/generation, source/knowledge/identity/rights revisions, locale/template/validator/config revision, deadline, execution mode and correlation. OperationContext is correlation, never permission. |
| Admission | Ready, Joined, Accepted, Deferred or typed denial. Same operation/different fingerprint is Conflict. Lost receipt looks up the operation before any resend. |
| ApprovedSpeech | Immutable exact approved text/hash, speech envelope/claim qualification, speaker identity revision, voice reference/version, locale, pronunciation profile revision, style/prosody options and caption binding. Unchecked model tokens or arbitrary raw player text are not Speak inputs. |
| AuthorizedAudioInput | Owned bounded PCM, complete encoded bytes, or authorized immutable asset handle; declared format plus digest, duration basis, provenance, consent/rights and expiry. No untrusted arbitrary URL or filesystem path dereference. |
| MusicRecipe | Checked prompt or authored composition specification, output/duration/loop requirements, parent/child recipe, required/optional roles, pacing/harmony/motif identity and mastering revision. A reference may be symbolic or authorized audio only if the backend actually supports audio conditioning. |
| SoundIntent | Approved text/semantic purpose, duration/range, loop request, role (ambience/effect), intensity and output format. Reuse prepared effects for ordinary combat/UI cues. |
| AudioFormat | Container, codec, sample rate Hz, channel count/layout, PCM signedness/endian/bit depth or float format, encoded bitrate, declared versus inspected facts. Codec/container mismatch and unsupported conversion fail typed preflight. |
| AudioResult | Typed ReusableAsset(s), ScopedLiveStream or TransientTranscript(TranscriptResult). A stream handle is not a reusable Ready asset. Complete assets carry inspected content/hash/bytes, actual duration/frame count/timebase, format, loop points, loudness/peak and lineage. Live duration/usage may remain Unknown until terminal observation; final STT may remain request-lived memory. Requested facts stay distinguishable from inspected facts. |
| TranscriptResult | Verbatim recognized text distinct from optional edited/normalized text; requested/detected language and probability, final segment identity/revision, speaker/channel labels only when supported, word/character ranges and offsets, confidence/log-probability evidence and terminal disposition. A speaker label never authenticates a player. |
| AudioEvent | Admitted, Started, Progress, ordered EncodedAudio/DecodedPcm or transcript Partial/Final, InputClosed, Completed, Failed, Cancelled, Stale and UsageCorrection; every event carries immutable basis/sequence/timebase. Capability determines allowed payload variants. |
| Errors | Disabled, Unavailable, UnsupportedCapability/Format/Option, PolicyDenied, LimitExceeded, InvalidInput, PermissionDenied, Capacity, Refused, Deadline, Cancelled, Stale, Conflict, Redacted and Unknown; field/bound/config provenance safe to inspect. No silent substitution. |

Encoded provider chunks, capture chunks and decoded playback frames are different granularities. HTTP chunk boundaries are not MP3 frames or timeline offsets. Native adapters parse supplier events and validate EOF; an explicit decoder owns codec framing and sample-time conversion. Complete encoded assets can be decoded on the browser device for playback. Incremental encoded streams require a qualified stateful decoder; never repeatedly call decodeAudioData on arbitrary incomplete fragments. PCM chunks carry frame offset, frame count, sample rate and layout under one stream anchor.

Server transforms own reusable derivatives: decode/encode, channel mapping/downmix, resample, trim, loop cut/crossfade, loudness/peak normalization and bounded fades, in explicit ordered versioned recipes. Output validation checks finite samples, frame arithmetic, actual duration and loop endpoints. Transform output is an immutable derivative; preserve source lineage and never re-charge source generation. Device resampling/volume/ducking/crossfade is presentation under df-audio, not a provider request. Preserve current mastering behavior first; acoustic quality changes need separate acceptance.

TTS alignment is optional evidence, not guessed timestamps. Record exact spoken/normalized text and alignment-to-text map, source character unit, frame/timebase, offset and confidence. Validate monotonic ranges against actual audio duration and approved content. Captions match the actual recording, including prepared fallback. Without qualified alignment, retain whole-line reading/actual clip timing and label fine alignment Unavailable. Translation or semantic pronunciation rewriting uses TextService, receives current audience/locale qualification and changes the speech/caption/cache identity; it is never hidden inside an audio adapter.

## Safety, identity and privacy

Features project permitted context before text or audio dispatch. Prompt construction may be deterministic; language-model judgments (music labels, ambience selection, NPC wording, translation or expressive rewrite) go through planned TextService under registered purposes, not direct Jev/Responses calls. AudioService realizes approved sound/text and owns structural media validation, not rules, story truth, relationships or canonical commits. Preserve df-ai expression and df-interaction grounded speech qualification. Revalidate basis before egress, publication, stored reads and audience delivery.

Voice casting is feature policy: preserve the deterministic roster, stable actor identity, narrator, explicit override/fallback, age/gender/tone policy and run binding. Backend voice identity is an opaque typed reference, not a player's identity or permission. Bind roster/voice/locale/model/settings/pronunciation dictionary revisions and authorized reference hashes to output identity. Imported voices/reference audio require scoped rights and consent, deletion/revocation handling and provider retention disclosure. Do not infer biometric identity, emotion or permission from captured sound. No cloning/voice-conversion product is implied by this consolidation.

Preserve the current STT privacy promise (`speech_input.rs:1-15`): raw waveform and transcript are request-lived memory, uncached and absent from ordinary logs. Minimal durable attempt records retain identity, fingerprints, sizes/time, route, send state and usage/liability, not a new recording archive. Retaining transcription content for replay is a separately consented retention choice; default replay miss is typed Unavailable. On crash or lost ephemeral output, preserve Unknown/settlement evidence and require a new deliberate capture when needed, without auto-resending old audio. Scrub/release buffers promptly; expired/deleted/private input cannot be reconstructed from diagnostics.

Browser capture starts only from deliberate interaction in an allowed secure context and current capture lease. Distinguish permission pending/denied, missing device, unsupported codec, device loss and browser operation failure. Own and stop every acquired track, including late permission results after cancel. Typed visible recording state, duration/bytes, stop/cancel and transcript review remain available. Page hidden, typing, disconnect, changed offer/run, disposal or revoked lease terminate unaccepted capture and reject late transcript application. Audio recording never proves user authorization for a gameplay action.

Final STT segments remain editable drafts or input proposals. Tentative partials cannot send an action, canonize speech, decide a roll or irreversibly trigger speculative spending. Existing confirmation sends through the ordinary server-authorized input path. Streaming VAD/commit merely finalizes a transcription segment; it is not a D&D turn or action. Barge-in requires a session-validated interruption against the active conversation/cue; local mute/stop is a device preference, not an NPC interaction or relationship penalty.

Remote/mixed-room routing follows [Remote play](remote-play.md): one output lease per capability group for public tracks, private listener tracks only on authorized unlocked devices, no private cue/manifest/prefetch/caption leakage to the TV. Fail private audio to authorized text, never to the group speaker. Reconnect reauthorizes and receives current timeline/active tracks; no replay of expired one-shots. Multi-tab and public speaker ownership are explicit. Paired hidden-state tests cover prompts, keys, events, manifests and audible cue differences.

## Streaming, cancellation and playback ownership

Use bounded queues measured by events, encoded bytes, decoded frames, age and deadline, with per-account/campaign concurrency and reserved input/speech capacity. Queue limits are configurable finite resource policy, not a budget cap. Backpressure across capture -> upload -> provider and provider -> decoder -> playback is explicit. An overflow terminates/cancels a stream with a resumable/typed outcome; never drop a required terminal chunk, concatenate discontinuous PCM silently or freeze the session owner. A slow subscriber cannot retain unlimited bytes; resync by permitted cursor/current timeline.

Every job/stream/capture/cue carries operation, run/recovery epoch, generation and sequence. Each decoded chunk schedules at one immutable audio-clock anchor plus frame offset, not arrival time. Normal EOF drains queued tail; cancel stops scheduled nodes/HTML media, clears pending buffers/object URLs and rejects late results before acknowledging stop. Cancellation of one interested waiter does not cancel a shared admitted job for other consumers. RPC receipt loss stops the wait; accepted run-owned work survives with its own deadline. Unknown provider liability survives every cancellation.

df-audio should expose per-device `unlock/consume/cancel/set_mix/dispose` and capture ports with actual state observations. Consolidate live_voice/live_music/live_sfx behind one owned mixer graph/clock per output device: speech, score, ambience, effect and cinematic buses; shared mute/unlock, gain, limiter, speech/SFX ducking and crossfade arbitration. Preserve conductor ownership of line order and reading time. An HTMLAudioElement/video bridge may initially feed that graph; inability to route a media element or select a device is typed/visible, not a claim of synchronized playback. Do not create an AudioContext per effect or interpret current separate contexts as already synchronized.

The source has no discovered PannerNode/spatial audio execution. Positional metadata may be a future capability only when an actual consumer requires it; do not infer spatialization from battlefield token IDs. Current token-based effect choice and presentation timing remain deterministic.

Embedded kill-shot video audio must join the cinematic bus or be explicitly muted with an approved replacement audio plan. A video-only service keeps video generation; bundled audio remains one supplier video charge. Any extracted/remixed derivative uses Prepare and links the video parent, without charging its embedded track again. Home video stays muted. Missing sound never changes combat outcomes or holds rewards.

Device lifecycle records receipt, decode, scheduled, actual started/ended/stopped/error separately from server generation/delivery. No software event proves a speaker was audible. Suspend/resume, background tabs, autoplay rejection, output device changes and reconnect require explicit epoch/reanchor/resume policy; stale buffers cannot burst on wake. Recreate/dispose graphs, listeners and tasks under one mounted owner.

## One lifecycle, publication and recovery

1. Resolve current mode/profile/capabilities and authorize scope/rights. Off/prepared/replay miss performs no live I/O. Deduplicate operation identity separately from content reuse.
2. Use the one audio/resource scheduler. Persist admitted job/attempt intent and frozen config; obtain the existing local/hosted admission once. Required and optional music child stages have stable attempt IDs and independent evidence.
3. Record the dispatch permit/possible-send boundary before supplier I/O, including protected hosted recovery-journal requirements. Execute one attempt through current egress authority. No per-feature resend loop.
4. Retain minimal attempt/send/usage observations even if the consumer/run changed; payload retention follows the operation's policy. Reusable synthesis/transforms validate full EOF/codec/container/format/timebase/content/rights before publication. STT returns a transient TranscriptResult to its authorized request owner and journals metadata only by default. Approved live audio may broker qualified sequenced chunks before terminal EOF under scoped stream ownership; validate framing, approved clause and current rights before each delivery, with terminal duration/usage explicit when unknown. No unqualified text or arbitrary encoded fragment becomes playable output. Failed required music layer rejects the set; retain successful and Unknown child costs. Optional ambience fallback is named.
5. Only reusable synthesized/transformed results publish complete immutable bytes and authorized metadata atomically through df-assets. A live stream becomes reusable only if recording is permitted and the complete verified recording is successfully published. Incomplete/cancelled/failed streams never become cache hits or replace a good recording. STT needs no asset publication and does not retain waveform/transcript merely to finish a journal row. Current file-cache manifest-last is compatibility evidence, not full hosted publication.
6. Deliver only while the current owner/audience/timeline basis still holds. Stale success may settle its original charge but cannot publish to a newer scene, replay a one-shot, clear newer busy state or commit gameplay.
7. Restart reconciles stable supplier identity where supported; PossiblySent/Unknown never becomes unsent on timeout, lease expiry, process death or lost waiter. If supplier idempotency/status lookup cannot establish safety, retain Unknown and prepared fallback; no blind billable repeat, model/provider failover or rewording after refusal.

Request identity includes scope/audience, source/rights/voice/reference digests, exact approved text or PCM/encoded digest, locale/pronunciation, recipe/validator/version, model/backend/account/effective options, output/timebase/loop/transforms and mode. Capture operation IDs are not reusable transcription-cache keys. Bound caches/flight registries and preserve durable Unknown history independently of eviction. Import legacy generated music/voice only with established authorized lineage; a hash filename does not grant private access or automatically justify regeneration.

## Configuration and development flexibility

Carry [TextService R3](text-generation-architecture.md#development-flexibility-contract-r3) forward intact. Profiles are operator data. Register backends/capabilities; do not bake arbitrary model-name, endpoint, path, voice-name, fixture-size or test-count allowlists into the facade. Valid Unicode/spaces in profile names and native paths remain valid. Voice IDs/options must satisfy the selected provider's actual wire contract, rather than inheriting incidental ASCII-only limits as universal policy.

| Adjustable surface | Contract |
| --- | --- |
| Resolution | Document defaults -> file/profile -> environment aliases -> launcher/explicit launch override -> authorized runtime patch; exact precedence frozen in A0, one resolver and redacted per-field provenance/diff. Existing launcher settings apply on next game start today, not hot reload. |
| Profiles | Backend/model/base endpoint/account credential reference; speech voice/locale/style/pronunciation; STT locale/commit/diarization/format; music/effect duration/loop/recipe/output; typed backend extension schemas. Arbitrary provider-valid models supported by an adapter may be configured without code edits. |
| Paths/endpoints | Operator-selected executable/scratch/cache/fixture locations, valid Unicode/spaces and local/private/loopback endpoints; native path/argument arrays, never shell interpolation. Hosted egress/rights policy remains separate and enforced. No untrusted player endpoint. |
| Limits | Finite count/bytes/frames/duration/channel/sample-rate/bitrate/memory/queue/age/concurrency/deadline/process-output/capture/fixture envelopes; documented defaults and configurable larger finite development envelopes within actual protocol/host representability. |
| Gates | Global generation, backend, operation/purpose and existing voice/music/STT feature disables. Narrow enable cannot override a higher deny. Distinguish live-generation deny from browser mute/capture permission and prepared-asset access. Status/cancel/reconcile remain usable while disabled. Generation Off or prepared-only may still serve authorized cached/shipped audio. Capture permission/admission, delivery authorization and device playback/mute are separate controls; mute does not cancel an accepted supplier job or erase liability. |
| Activation | NextAdmission for models/voice/format/options; immediate acknowledged gate denial before another child dispatch; RestartRequired for device/backend resources that cannot safely reload. Validate entire patch atomically, keep last good snapshot, show pending restart. Admitted attempts retain original config/liability. |
| Test modes | Local/mock/loopback/replay/prepared/fault injection through exactly the same API; no real keys/wallet needed for offline profiles; explicit authorized fault controls and deterministic clocks. Offline miss cannot fall through to network or another provider. |

Separate diagnostics for actual backend/protocol capability, product/security policy and configured resource envelope: UnsupportedCapability/Option, PolicyDenied or LimitExceeded with effective bound/source. Capability evidence may be adapter-qualified, introspected or operator-declared fixture/config data; unknown support is visible. Qualification confidence is not an invented development ban. No silently dropped option, truncated request, substituted locale/model/codec or guessed voice fallback. Preserve existing explicit prepared fallback selections as reported policy outcomes.

Current 15/16-second capture, 4096-character adapter speech input, 300-second music set deadline, 2 child permits, 8 MiB music response and Fal 180-second/250-step restrictions are source migration inputs. Check each against current vendor capabilities; neither copy all constants as universal ceilings nor remove genuine protocol validation. New purpose/profile/fixture changes require no production source edits or bypass.

## Usage, costs and observability

Use the existing local Hub SpendLedger/MeteringPort cost book and hosted df-finance/admission projection. Coding style prohibits new local spend caps; commerce/service-operations govern hosted consent, grants/liability/concurrency. Preserve both. This plan adds no ledger, wallet, price policy or local cost refusal.

Each actual supplier attempt has one booking owner, exact usage units and provenance: TTS admitted characters versus supplier-reported character cost; STT input samples/duration and existing rounded billed-second estimate; music/effects requested and actual generated seconds; future audio/text tokens or credits where actually reported. Record rate version, exact integer/rational conversion and rounding, account/tier/currency and evidence class (reported, measured, estimate, invoice-confirmed or unknown). Do not price newer models using an old tariff silently. Missing tariff is Unpriced, not zero or a new local refusal. Subscription activity is separate from marginal USD.

Current cost specs `cost/specs.rs:14,37,49` and speech `voice.rs:1021` use existing categories; keep precise raw duration alongside current rounded pricing, not falsely claim vendor-billed measurements. Supplier metadata/character-cost headers are observations, not verified settlement. Source generation, TextService judgments and audio child attempts have linked identities; a parent rollup counts each once. Cache hits report zero new supplier calls while retaining original lineage/cost. Local transformations report local CPU/storage separately, not duplicate supplier charges.

Cancelled-after-send, rejected, unused speculative, failed-but-billed and partially successful music attempts remain attributable. Stream termination is not free: record sent audio/duration, provider usage if known, cancellation request/ack and known/Unknown liability independently of client playback. No automatic refund or paid resend. Durable settlement/correction is idempotent by attempt/revision; reporting failure is visible and rebuilt from retained observations.

Extend existing dev cost/status/control views with purpose/backend/voice/locale/stage and audio-specific breakdowns. No public prompt/transcript/raw waveform or secret account data in diagnostics. Supplier tariff/account details stay in authorized operator views. Audio access does not grant cost-account access.

Use shared df-observe OTEL and existing audit path: operation -> text judgment if any -> audio parent/stage/attempt -> asset -> delivery -> device cue. Capture queued/admitted/provider/EOF/decode/publish times, capture duration, speech first-approved-audio, lag/drift/underrun, bounded queue bytes/frames, cancel and stale discard, fallback/disabled/unsupported, actual started/end observations, cache/coalescing, usage/Unknown and config/build revisions. Only approved opaque identifiers, scope-controlled digests and bounded counts enter ordinary telemetry; private-content hashes can leak correlation and are not presumed safe. Prompts, private speech, transcripts, credentials and raw reference bytes are redacted. Client observations remain best effort; report telemetry gaps and never use OTEL as canonical gameplay or money truth.

## Implementation phases and gates

Logging refinement (2026-10-28): [Shared generation logging](generation-logging.md) defines the concrete event envelope, vocabulary, one audit/OTEL bridge, dispatch-captured context, typed privacy/usage fields and finite loss reporting for this service. Its source-gap table distinguishes current live wrappers, hosted/native producer primitives and browser reports from the proposed typed audio events. Neutral df-observe contracts remain downstream dependencies of audio; no reverse dependency.

Capture/transcript events contain lifecycle/counts only and retain the ephemeral STT default. TTS links approved text candidates; music logs every required/optional child; transforms and ffmpeg report normalized outcomes; device decode/schedule/start/end/mute/route/underrun are separate observations with audibility Unverified. A0-A6 below include logging gates; no audio/logging runtime proof was obtained.

All tests below are proposed; none ran for this planning task.

| Phase | Concrete work and ownership | Exit evidence |
| --- | --- | --- |
| A0 Freeze | Root + media owner freeze AudioService/native feature, pure value types, AssetEngine delegation, hub transfer, text/finance/asset/egress ports, output/capture wire changes and config activation classes; record ADR. | Surface-map entries classified; current source hashes and behavior recorded; no df-media/df-audio cycle; no duplicate scheduler/ledger/publication owner.  Logging gate: Freeze shared logging/clock/event/owner/privacy schemas and compatibility bridge. |
| A1 Offline spine | df-media native facade/config/one scheduler, faithful fake backend, typed batch/stream lifecycle and deterministic transforms; use existing stores via ports. | Offline all operations, gates/provenance/reload, Unicode paths/models, configurable finite limits, cancellation/backpressure and no-live replay misses. No product claim from helper-only tests.  Logging gate: One offline producer/reader path; no sampled trace; transient STT privacy. |
| A2 Execution | Move Eleven speech/music plus Fal sound behind private backends; durable attempts/recovery/usage and transform subprocess supervision; integrate current local book and hosted dispatcher/finance/asset storage before live consumer cutover. | Loopback contract and crash cuts before/after possible send/EOF/publication/settlement; no duplicate billable resend; four-plus-one child outcomes preserved.  Logging gate: Per-child/transform/process/send/Unknown and ledger linkage events verified. |
| A3 Consumers | Requires A2 plus TextService T0 frozen purpose/port contracts, T1 reachable lifecycle and T3 actual SystemOne adapter; coordinate the corresponding T5 consumer cut. Migrate Voice registration/prewarm/delivery, SpeechInput capture review, PlaceMusic/Jev, standalone ambience, dev controls, launcher/run script/fake profile; prepared packs remain readable. | Every matrix consumer uses same facade; current voice identity/clip boundaries/privacy/explicit fallbacks/guarded dev mutation preserved; no GET starts a new provider job.  Logging gate: Captured text-to-audio causal links and one job lifecycle across waiters. |
| A4 Device integration | df-audio mixer/decoder/capture adapters, current conductor + speech/music/SFX + embedded cinematic bus, epoch/cancel/route/lease reports. | Native/WASM boundary checks and real browser device tests including actual playback observation; no stale/duplicate cues or private group leakage.  Logging gate: Device clock/route/cancel/overflow/missing-report cases; no audibility inference. |
| A5 Delete bypasses | Retire public audio execution factories, old feature flights/cache/retries and script provider calls; architecture guard scans executable paths including tests/scripts. | Forbidden-fixture guard fails intentionally then clean source passes; no permanent migration exemptions; ordinary profile/fixture changes unrestricted.  Logging gate: No duplicate string/typed event counts or alternate generation log pipeline. |
| A6 Acceptance | Independent review on fixed build/config/source/asset revisions; scoped paid samples only under a separate authorization/brief, after offline/native/browser gates. | Audible voice, intelligible captions, seamless music/ambience/ducking, action confirmation, remote/private routing and recovery evidence; update map/docs.  Logging gate: Shared logging fault/redaction/cardinality/read-only gates and actual audio evidence. |

The audio-only offline spine may proceed while TextService implementation is pending. A3 cannot complete by naming a nonexistent TextService or retaining direct Jev dispatch. Any temporary ApproachModel facade must already delegate to that one actual text owner and has the text T5/T6 removal gate; no new direct-provider interim bypass.

No live shadow dispatch: compare normalized requests/route decisions with fakes only. Exactly one old/new owner per operation. Rollback must understand retained attempt/Unknown fences or stop new dispatch. Do not weaken durability or rights temporarily while library consumers migrate.

## Proposed acceptance matrix

| Boundary | Required observations |
| --- | --- |
| Pure/native/WASM | Focused affected crate tests, formatting/Clippy/native checks and established df-tools WASM gate when implementation exists; libraries remain target-safe. No arbitrary test-count quotas. |
| Providers | Faithful local HTTP/event/process fixtures validate exact outgoing fields, split UTF-8/JSON/codec framing, bounds/EOF, HTTP refusal, malformed duration/loop/audio, identity/metadata/usage, cancellation and supported options. Fal poll/cancel/download are one attempt lifecycle. |
| Speak | Narrator/NPC, stable cast/recast, long segmented text, current approved scope only, withheld secrets, pronunciation/locale/normalization, missing alignment and matching fallback captions; current clip cannot be cut by next line. |
| Capture/STT | Secure-context/gesture/permission denial, late granted mic after cancel, 1+ channels/downmix/resample/quantization, record/stop/Escape/typing/hidden/disconnect, stale lease, malformed/oversize input, no speech, draft edit/send; tentative/final STT alone performs zero actions. |
| Streaming | Slow producer/consumer, discontinuity, duplicate/out-of-order/missing final, EOF drain, suspend/reanchor, overflow and cancel-after-buffer; distinct input/encoded/decoded timebases and finalization; no raw unqualified TTS text. |
| Music/effects | Authored and TextService-selected specs, pacing variants and symbolic motif; four required layers plus optional ambience with independent costs; duration/loop/mastering checks, prepared misses, no repeated ending sting, restrained rating, duck/crossfade/levels. |
| Config/development | All layers/provenance/gates and atomic reload rollback; model/voice/endpoint/options/Unicode path changes without rebuild; larger finite fixture envelope; typed unsupported/policy/limit errors, never silent fallback. |
| Recovery/cost | Crash before permit, after possible send, after child success, before asset metadata, after publication/before settlement; ephemeral STT content absent; unknown remains; cache/coalescing zero duplicate calls; parent/child/transform costs counted once. |
| Real browser/audible | Fixed build on selected supported desktop/phone browsers and actual output device: unlock/mute, clip start/end/intelligibility, read timing, music loop clicks/seams, ducking/release, clipping, SFX density, cinematic audio, tabs/sleep/reconnect/cancel; record human or appropriate audio observation separately from software events. |
| Remote/privacy | Independently networked public display + local/remote private listeners, duplicate-tab/output leases, headphones unavailable, revocation, slow network and reconnect mid-cue; no private stream/caption/asset/cue leak and no old one-shot replay. |

Current contract tests, fake-profile coverage and historical manifests are source evidence only. They were not executed here; builds remain paused after the disk incident. No provider request, playback, microphone capture, audio quality, latency or remote readiness was tested. Supported devices, codec/stream capability qualification, exact wire changes, retention consent and vendor idempotency/reconciliation support remain A0/A6 gates.

## Official documentation checked

Accessed 2026-10-28 America/New_York. These establish vendor/browser possibilities, not implemented engine capabilities or account entitlements.

- [ElevenLabs TTS stream](https://elevenlabs.io/docs/api-reference/text-to-speech/stream): model/voice/output-format, optional pronunciation and language controls. Current adapter supports a narrower fixed profile.
- [TTS with timing](https://elevenlabs.io/docs/api-reference/text-to-speech/stream-with-timestamps): a distinct JSON audio/timing endpoint; not the current buffered MP3 adapter.
- [Batch transcription](https://elevenlabs.io/docs/api-reference/speech-to-text/convert): formats/language/timing/diarization options; the engine intentionally uses a narrow no-diarization profile.
- [Realtime transcription](https://elevenlabs.io/docs/api-reference/speech-to-text/v-1-speech-to-text-realtime): separate WebSocket partial/committed segment protocol and commit strategies; needs a new private adapter and lifecycle tests.
- [Music compose](https://elevenlabs.io/docs/api-reference/music/compose): prompt or composition plan, 3-600 seconds and newer model choices. Source pins music_v2 and symbolic text motif, not audio conditioning.
- [Sound effects](https://elevenlabs.io/docs/api-reference/text-to-sound-effects/convert): 0.5-30-second duration and loop/output options. Source allows integer 1-30 seconds; that is narrower local adapter behavior.
- [Fal Stable Audio schema](https://fal.ai/models/fal-ai/stable-audio-25/text-to-audio/api): source caps 180 seconds/250 steps; current docs give seconds_total default 190. Requalify before treating source constants as supplier maxima.
- [W3C Web Audio](https://www.w3.org/TR/webaudio/) and [Media Capture and Streams](https://www.w3.org/TR/mediacapture-streams/): decoding, graph clocks, suspend/resume, track ownership and permission lifecycle inform device acceptance. Draft specifications and API availability do not establish actual browser behavior.

No OpenAI/Codex audio backend was found or selected. Text/image backend availability does not imply an audio implementation. Further vendors are outside this consolidation decision.
