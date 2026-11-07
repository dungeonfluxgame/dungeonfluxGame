# Persona emotion, mood, attention and a leaner line call

Date: 2026-11-07
Status: Design adopted; implementation in progress. Extends
[persona-voice-and-world-bible.md](persona-voice-and-world-bible.md). Types are frozen in
`crates/df-content/src/persona/emotion.rs`; the live state seam is
`crates/df-table-host/src/persona/state.rs`; the viewer-aware world seam is
`world::context_for_viewer` and `world::check_quote`.

## Why (from the market thread, run 2)

Flat affect (all 20 lines warm-neutral, no irritation at haggling, no pleasure at a sale), a
reply that ignored what was said (an idiom for a complaint), the customer speaking as the
keeper, a trigger firing while the line took another behavior, a discount rounded away, 9 to 17 s
per answer (35 model calls, 363 s, ~20k tokens in for ~300 out, nothing cached).

## Layers

1. **Temperament** (stable, on the persona): baseline emotion levels, reactivity, patience,
   warmth, pride, decay, attention capacity, distractibility. Generated and editable like voice.
2. **Emotion state** (live): intensity 0 to 100 for each emotion in the closed list
   (`persona/emotions.json`, default `DEFAULT_EMOTIONS`), fading toward the baseline each turn;
   the **mood** (valence and arousal) is derived from it by the content's mapping.
3. **Attention** (live): the character and topic in focus and the recent lines it registered, at
   most its capacity. A line answers what is in focus first; a distractible character is pulled to
   the latest line, a steady one holds its focus.
4. **Attitude and rapport** (live, per other character): Friendly, Indifferent or Hostile on the
   2024 scale plus a finer rapport. The server owns it; the rules own what it changes (for a
   shopkeeper, how far a price may fall inside the bible's bounds).

Emotion and temperament are wording and presentation aids and never a mechanical benefit by
themselves. No text is matched to find a feeling: a model appraises, code validates and clamps.

## Emotional triggers

An `EmotionalTrigger` has either `when_state` (a numeric condition on the live levels, checked by
code) or `when_event` (a statement a model judges, like every trigger), may `set` emotion deltas
and may switch the character to a behavior. A trigger that fires makes its behavior the one the
line takes.

## One call per line

The line call receives the state (emotions, mood, attention, attitude), the voice layers, the
viewer-scoped world facts and the recent lines, and returns in one answer: the line, the behavior
and fired triggers, any secrets and leak flag, the quote (item, quantity, total), the **gaps** (the
old coverage question), and the **appraisal** of what was just said. Code validates every id,
clamps the appraisal by the temperament, clamps the quote with `world::check_quote`, updates the
state and drives the tone. The backfill of a gap runs after the line, off the critical path, and
benefits the next line. This replaces the separate coverage call.

## What is sent (tokens and latency)

Measured: ~20k tokens in per call, almost all the Codex CLI's own agent prompt, none cached; 9.7 s
median for a line, 6.6 s for coverage. Levers: trim what the CLI sends (reasoning effort, tools,
instructions; configurable through `DF_TEXT_CODEX_ARGS`), compact JSON facts with empty fields
dropped, a stable prefix first (instruction, schema, persona) and the changing part last so a
cache can hit, only the layers a call needs, short schema descriptions, and the direct API path
(Responses) with a cached prefix once the key has credit. Target: one call of about 10 s on the
Codex path, 1 to 3 s on the API.

## Correctness

Viewer-scoped world facts (a customer sees the posted price, never the floor or tiers); quotes
computed on the total, rounded up, clamped by code; a fired trigger forces its behavior; a repeat
self-check with one retry; per-call timings in the reply.

## Checking it

The market thread (`development/market-thread.sh`) plus three scored checks by a model: blind
attribution of each line to its speaker (voice), emotion fit (does the line's feeling match the
state it was given), and role integrity (does a customer ever speak as the keeper).
