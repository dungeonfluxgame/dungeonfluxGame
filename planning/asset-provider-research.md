# Asset providers: cost and latency

Price snapshot: 2026-09-29, USD, public API rates before tax. Status: research,
not a purchased integration or measured production route. No paid calls were made.
Use the [asset engine](asset-engine.md), [provider ports](subsystem-interfaces.md),
[RPC contract](rpc-api.md) and [commercial model](pricing-and-costs.md).
Revalidate availability, account limits and prices at G08 before implementation.

The strongest default is a cache-first asset engine: prepared ordinary SFX and
music layers, canonical portraits/items/locations generated once, streaming voice,
and optional prefetched short video. Cached playback adds no generation bill;
storage, decoding, transport and fan-out still cost resources. No provider is
demonstrably the cheapest and fastest for every quality, region and workload.

## Practical routes

| Work | Economic candidate | More capable fallback / qualification |
| --- | --- | --- |
| Validated text expression | GPT-6 Luna; test Groq GPT-OSS 20B for simple bounded expression | GPT-6.1 Sol for failed quality cases; never let a cheaper model decide rules or reveal secrets |
| Live player transcription | Deepgram Nova-3 streaming; test Eleven Scribe realtime alongside it | Google live transcription or OpenAI live transcription when measured accuracy/locale warrants cost |
| Final recorded utterance | Groq Whisper Turbo | Higher accuracy route after names/numbers/negation checks; file upload cannot replace live partial transcription |
| Live narration/NPC speech | Gemini 3.8 Flash-Lite TTS cost candidate | Deepgram Aura-2 / Eleven Flash for measured live response; expressive Eleven v4 only with a bounded premium allowance |
| Disposable new still | fal FLUX Schnell | Reference-capable route for recurring identity |
| Canonical/reference still | Runway Muse Image, then Gen-4 Image Turbo | Gemini Flash Lite Image or higher quality model if continuity fails |
| Themes and stems | Existing licensed, synchronized stem pack | Lyria for asynchronous theme exploration; generated/separated stems need qualification and verified compatible timing |
| Ordinary SFX | Existing prepared semantic library | Unusual new sound: asynchronous Eleven SFX, including Runway's explicit generation quote |
| Optional cinema | Veo Lite 720p using a prepared starting frame | Veo Fast/Standard when reference support is necessary; Runway Omni 360p only if low resolution is acceptable |

This is a qualification shortlist, not permission to integrate every supplier.
Start with one accepted route per live capability and one media supplier; add a
second only for demonstrated quality, cost, failure or capacity needs. Keep model
IDs, price version and capability flags in adapter configuration.

## Comparable billing units

Rates are marginal list prices unless a monthly plan is stated. All minute totals
mean submitted or newly generated audio, not human session length. Character-based
TTS uses an adjustable **1,000 characters per output minute** solely for examples;
different languages, pauses and voices change this conversion.

| Text provider/model | Input / cached read / cache write / output, USD per million tokens |
| --- | --- |
| Groq GPT-OSS 20B | 0.075 / not priced here / not priced here / 0.30 |
| OpenAI GPT-6 Luna, standard short context | 0.10 / 0.01 / 0.125 / 0.50 |
| OpenAI GPT-6.1 Sol, same tier | 2 / 0.10 / 2.50 / 10 |
| Google Gemini 3.1 Flash-Lite | 0.25 / 0.025 / separate storage charge / 1.50 |

Reasoning/output tokens, long context, priority modes and cache storage can change
the total. Groq advertises 1,000 tokens/s for GPT-OSS 20B; this is throughput,
not first validated response latency. Sources: [Groq models](https://console.groq.com/docs/models),
[OpenAI prices](https://developers.openai.com/api/docs/pricing),
[Google prices](https://ai.google.dev/gemini-api/docs/pricing).

| STT route | USD per submitted audio minute | Important billing/behavior difference |
| --- | --- | --- |
| Groq Whisper Large V3 Turbo | 0.000667 | 0.04/hour; minimum 10 billed seconds per request; file-based |
| Deepgram Nova-3 mono / multilingual streaming | 0.0048 / 0.0058 currently | Limited-time offer, end date not published here; regular 0.0077 / 0.0092 |
| Eleven Scribe v2 realtime | 0.0065 | 0.39/hour; streaming |
| Google Gemini 3.5 Transcribe Live | approximately 0.009 blended | Input and transcript output tokens both billed; not just the 0.005 input estimate |
| OpenAI GPT-Live-Transcribe | estimated 0.017 | GPT-Transcribe file route approximately 0.0045 |

Groq's 10-second floor means 1,000 three-second utterances bill 166.7 minutes,
not their 50-minute actual duration: approximately $0.111 rather than $0.033.
Overlapping repeated chunks also bill again. Sources: [Groq STT](https://console.groq.com/docs/speech-to-text),
[Deepgram prices](https://deepgram.com/pricing), [Eleven prices](https://elevenlabs.io/pricing/api),
[Google prices](https://ai.google.dev/gemini-api/docs/pricing),
[OpenAI prices](https://developers.openai.com/api/docs/pricing).

| TTS candidate | Published USD billing | Normalized output-minute estimate / caveat |
| --- | --- | --- |
| Gemini 3.8 Flash-Lite TTS | 6/million audio tokens now, 12 from 2027-01-01 | 0.009 now, 0.018 later, plus text input; 25 audio tokens/s |
| Gemini 3.8 Flash TTS | 9/million audio tokens now, 18 from 2027-01-01 | 0.0135 now, 0.027 later, plus text input |
| Deepgram Aura-1 / Aura-2 | 0.015 / 0.030 per 1,000 characters | 0.015 / 0.030 at the assumed speaking density; quality/voice controls differ |
| Eleven Flash/Turbo / v4 / v4 Turbo | regular 0.040 / 0.080 / 0.040 per 1,000 characters | v4 offers 0.022, Turbo 0.011 until October 12; use regular rates for durable budgets |
| Cartesia Sonic 3.6 | Pro $5/month includes approximately 133 minutes | approximately 0.0376/min only at full allowance use; Startup $49/~1,667, Scale $299/~10,667; verify overage and character conversion before reservation |

Google text input is 0.50/million now, 1 from January; batch halves its standard
rates but is unsuitable for live turns. Cartesia's Pro/Startup/Scale TTS concurrency
is 3/5/15, so a cheap nominal plan may fail a many-campaign workload. Sources:
[Google prices](https://ai.google.dev/gemini-api/docs/pricing),
[Google streaming TTS](https://ai.google.dev/gemini-api/docs/speech-generation),
[Deepgram prices](https://deepgram.com/pricing), [Eleven prices](https://elevenlabs.io/pricing/api),
[Cartesia prices](https://www.cartesia.ai/pricing).

| Image / video candidate | USD quote and output specification |
| --- | --- |
| fal FLUX.1 Schnell | 0.003 per megapixel, rounded up per published description; assuming decimal MP, 1,024×1,024 estimates 0.006 conservatively. The precise MP basis is unverified and might classify that size as one MP; confirm billing before dispatch. Text-only endpoint, no canonical-reference guarantee |
| Runway Muse Image / Gen-4 Image Turbo | 0.01 / 0.02 per image; price table says any resolution; both advertise reference input, actual supported dimensions need adapter validation |
| Gemini 3.1 Flash Lite Image | 0.0336 per 1K output, plus inputs/thinking; batch 0.0168 |
| Google Veo Lite / Fast / Standard | 720p with audio 0.05 / 0.10 / 0.40 per generated second; 8-second clip 0.40 / 0.80 / 3.20 |
| Runway Gemini Omni Flash 1.1 | 360p 0.034/generated second +0.01/reference image +0.01/input-video second; 720p 0.10/output second with same extras |

Runway credits cost $0.01 each. Do not confuse a draft still, reference edit, 360p
clip and TV-quality clip. Google Veo 3.1 routes are preview; 4/6/8-second durations
apply, higher resolutions require 8 seconds. Lite lacks multi-reference/extension
support in the feature table; a prepared first frame is different from multiple
identity references. Sources: [fal endpoint](https://fal.ai/models/fal-ai/flux/schnell),
[Runway prices](https://docs.dev.runwayml.com/guides/pricing/),
[Runway capabilities](https://docs.dev.runwayml.com/guides/models/),
[Google prices](https://ai.google.dev/gemini-api/docs/pricing),
[Veo specifications](https://ai.google.dev/gemini-api/docs/veo).

## Music, stems and SFX

Lyria 3.5 lists $0.08/song; legacy Lyria Clip is $0.04 for 30 seconds. Its API
returns stereo mixes, not a documented synchronized stem set. Generate themes
ahead of play, verify loop boundaries and approved rights, then reuse. Independent
prompts for bass, percussion and melody do not guarantee a compatible arrangement.
[Lyria guide](https://ai.google.dev/gemini-api/docs/music-generation),
[Google terms](https://ai.google.dev/gemini-api/terms).

Eleven lists music $0.15/minute and SFX $0.12/minute, but its FAQ says generation
billing and its SFX documentation differs on credits. Treat exact duration/minimum,
stem-separation cost and plan conversion as unresolved; get a written quote or
authorized bounded billing test before dispatch. Runway provides a simpler SFX
quote: automatic-duration `eleven_text_to_sound_v2` $0.02/generation, specified
duration $0.01/second. Pick automatic duration only when its accepted length fits.
[Eleven prices](https://elevenlabs.io/pricing/api),
[SFX API billing help](https://help.elevenlabs.io/hc/en-us/articles/25735337678481-How-much-does-it-cost-to-generate-sound-effects),
[SFX capabilities](https://elevenlabs.io/docs/overview/capabilities/sound-effects),
[Runway prices](https://docs.dev.runwayml.com/guides/pricing/).

Eleven exposes a stem-separation API, including two/six-stem modes, but explicitly
warns of potentially high latency; it is an offline preparation candidate.
Its model-specific music terms exclude self-serve Studio Games, prohibit music
libraries/repositories and limit entity eligibility. The definition involves a
monetized game used on more than one platform. Whether DungeonFlux's multi-device
browser distribution falls within that definition is unresolved; device types
alone do not establish multiple contractual platforms. Broad commercial-use
marketing is insufficient to approve this cached-theme use. Require written rights
clarification or an appropriate Enterprise Music agreement and quote before this
route qualifies. [Stem API](https://elevenlabs.io/docs/api-reference/music/separate-stems),
[music-specific terms](https://elevenlabs.io/eleven-music-model-specific-terms),
[music terms](https://elevenlabs.io/music-terms).

## Latency claims: what they actually establish

| Candidate | Vendor evidence | What DungeonFlux must measure |
| --- | --- | --- |
| Groq text | published token throughput | First validated clause, complete accepted result, failure rate |
| Groq Whisper | published faster-than-real-time file processing | Capture end + upload + transcript + validated intent; no live partial claim |
| Eleven Flash | approximately 75ms internal inference, explicitly excludes network/application | Server dispatch to browser's first audible approved sample |
| Cartesia Sonic | under 90ms first-audio claim | Actual region/concurrency, buffering, browser playback and quality |
| Deepgram Aura-2 | approximately 90ms steady-state TTFB claim | Full live path and cold/tail conditions |
| fal Schnell | sub-second generation marketing/example inference timing | Queue + complete validated bytes + download + visible image |
| Veo | documented request range 11 seconds–6 minutes | Deadline hit rate through byte publication and presentation |

These numbers are incompatible measurements, not comparable p50/p95 results.
No DungeonFlux latency benchmark exists. Streaming Google TTS returns raw 24kHz
mono PCM chunks; adapters must identify framing rather than assume each chunk is
a WAV file. Sources: [Eleven latency definition](https://elevenlabs.io/docs/eleven-api/concepts/latency),
[Cartesia claim](https://www.cartesia.ai/vs/cartesia-vs-gemini-tts),
[Deepgram runtime report](https://deepgram.com/learn/engineering-real-time-low-latency-voice-ai-at-scale),
[Google TTS](https://ai.google.dev/gemini-api/docs/speech-generation),
[Veo limitations](https://ai.google.dev/gemini-api/docs/veo).

## Budget, reuse and predictability

An illustrative three-hour campaign session with 400k input/60k output Luna tokens,
90 aggregate streaming input minutes, 30 new TTS minutes +10k input tokens,
12 Muse stills and already prepared music/SFX costs **$0.897** in current raw rates.
Adding a 20% billed regeneration contingency gives **$1.0764**. At regular Nova-3
and January TTS rates this becomes **$1.7196**. These are adjustable research
assumptions, not equivalent-quality measurements, launch COGS or a player cap.
They exclude hosting, transport, storage, payment, rights and support costs.

18 new eight-second Lite clips add $7.20 raw / $8.64 with the same contingency;
Standard adds $57.60 / $69.12. Generated seconds are paid once for a shared scene;
playback viewers affect delivery costs, while separate private scenes add generation.
A 50% used-asset hit rate doubles generation cost per used asset. Ten newly
generated automatic-duration SFX add $0.20; replaying prepared SFX adds zero
generation spend. JSON assumptions are independently editable in the output model.

Reserve conservative upper bounds using exact money and a price-version snapshot.
Track submitted seconds, characters, image megapixel rounding, references, output
seconds, minimums, retries, rejected/unused/stale results, unknown outcomes and
actual invoices. Google says blocked/unsuccessful Veo generations are not charged;
do not extend that promise to other providers or cancel/timeout cases. An unknown
paid-call outcome must reconcile before a repeat or refund.

Prefetch only within an approved campaign budget, bounded horizon/top-K, queue and
bytes limits. Scores are heuristics until calibrated, not free probability facts.
Record branch hit rate, lead time, avoided waits, stale/wasted dollars and quality
rejections. Retire unstarted stale work; ready assets never canonize a predicted
event. Keep future manifests server-private and hash identity/style/reference/
provider/format versions with campaign and audience scope. Cross-campaign cache
reuse needs an explicitly shareable content basis; identical bytes do not grant
permission. Deliver authorized assets through the approved Asset RPC boundary.

R2 standard storage lists $0.015/GB-month with direct egress free, but server-relayed
asset delivery still incurs the server host's egress and CPU costs. R2 requests
also bill. [R2 rates](https://developers.cloudflare.com/r2/pricing/).

## Qualification benchmark and implementation handoff

G08 should approve a future Rust development harness over the existing provider
ports, not an additional game service. No harness or paid run is created here.

1. Freeze versioned permitted fixtures: D&D names/numbers/negation, overlapping
   speech/noise/locales, short and long NPC clauses, emotional narration, recurring
   character/item reference edits, ordinary/unique SFX, loop/stem transitions and
   milestone clips. Supply expected semantic/identity constraints and review rubrics.
2. Run candidates from proposed server regions with reused TLS/WebSocket sessions,
   cold and warm caches, controlled concurrency and actual supported group capacity.
   Separate queue/network/inference/validation/publication/relay/device times.
3. Record first partial transcript, final transcript, first validated intent;
   first model token and approved stable clause; TTS first byte and audible sample;
   complete still/video publication and first visible frame. Report sample counts,
   p50/p95/p99, timeouts, refusals, retries, quality failures and reconciled dollars
   per accepted output; never present vendor claims as measured samples.
4. Exercise barge-in/current capture lease, stale cue fencing, unavailable/late media,
   slow devices, codec negotiation, privacy differences and reconnect. Partial STT
   remains tentative. Publish TTS/captions only from validated stable clauses after
   its safety gate; otherwise validate full text. Optional cinema never blocks play.
5. Frontier computer-use/vision plus audible review checks both TV and player clients,
   identity continuity, readable fallbacks, smooth transitions and intelligibility.
   Freeze the least expensive accepted route meeting explicit deadlines and quality;
   configure spending caps, provider concurrency and circuit breakers from evidence.

Exclude Sora: OpenAI's official shutdown was 2026-09-24. Legacy OpenAI transcription
models have a 2027-02-26 removal date; use current replacements for a new integration.
Gemini 2.5 Flash Image retires 2026-10-02; avoid starting on it. Keep preview routes
optional with a prepared fallback. [OpenAI deprecations](https://developers.openai.com/api/docs/deprecations),
[Google price/lifecycle notice](https://ai.google.dev/gemini-api/docs/pricing).

## Observed OpenAI spend, 2026-10-18 to 2026-10-24 (owner's billing page, mostly 2026-10-24 play-testing)

| Line item | Week total | Of which 2026-10-24 |
|---|---|---|
| gpt-image-2.5-flare image input (reference images) | $14.852 | $14.852 |
| gpt-image-2.5-flare image output | $16.165 | $15.898 |
| gpt-image-2.5-flare text input (prompts) | $10.38 | - |
| gpt-image-2.5-flare text / image cached input | $0 | $0 |
| gpt-image-2.5-sunburst image input | $1.829 | $1.776 |
| gpt-image-2.5-sunburst image output | $2.201 | $1.954 |
| gpt-image-2.5-sunburst text input | $0.157 | $0.15 |
| text-embedding-3-small, gpt-5.4-mini, gpt-5.6-luna (all lines) | $0 | $0 |

Total about $45.6, almost all GPT Image 2.5 on 2026-10-24. Inputs (reference images $16.7, prompt text $10.5)
cost more than outputs ($18.4): plate edits send 2-4 high-fidelity reference images and 17-20k-character prompts.
Smaller output sizes and low quality (the dev picture profile, commit 422f2d11) only cut the output share; the
next levers are fewer and smaller reference images per edit, shorter plate prompts, and fewer plate renders
(the REF-068 cadence starts a render about every 30 s while people talk).
