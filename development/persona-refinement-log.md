# Persona refinement log: Mara Vell at the fruit stall

What was run, what the vendor did, what was changed and why. Newest run last. The exercise is
`development/market-thread.sh <beats>` on the saved market scene (`playground go market_stall`,
`world place market`); beats live in `development/market-threads/`. Goal: the vendor answers like
a real person who sells fruit for a living, stays true to her persona, and shows feeling that
fits what happens. Plans: `planning/persona-voice-and-world-bible.md`, `planning/persona-emotion.md`.

Tunable without a rebuild (re-read each call): `persona/emotions.json` (emotion list, appraisal
guide, conduct guide), `world/bible.json` (items, stalls, prices, voice packs), the persona files.

## Run 1 (2026-11-07, before the emotion layer)

20 messages, haggle beats. Name consistent; firm price on one apple and a bulk discount on five
fired by persona triggers. Failures: no prices at all (two price backfills blocked by the
whole-word private-detail guard; she stonewalled "I haven't set the total yet"); every reply saw
only the last line, so she repeated "for the road" five times; ~14 s per answer.
Changes: world bible with prices; Jev (model) privacy judge replaced the word guard; recent lines
and the asker passed to every line; voice block, packs and expertise.

## Run 2 (voice, world bible)

Prices quoted from the bible; no blocked backfills; distinct voices. Failures: the elf (customer)
spoke as the keeper 4 of 10 lines because every prompt held the stall's floor and tiers; the bulk
discount rounded away (3 cp * 90% rounds back to 3 cp); a trigger fired while the line took
another behavior; "friend"/"snug" overused; ~23 s per answer (35 model calls, 363 s, ~20k tokens
in per call, nothing cached).
Changes (commit 55d3aa2d): emotion layer (temperament, emotions, mood, attention, attitude,
emotional triggers, appraisal in the same call); one call per line (coverage merged, backfill
after the line); viewer-scoped world facts; quotes on totals and clamped; fired trigger forces
its behavior; repeat self-check; lean Codex call (~24k -> ~10k tokens in).

## Run 3 (haggle, emotion layer)

Answers 5.8 to 13.7 s, median 6.8 s (was ~16 s). Discount works: five apples 14 cp. Irritation
rose 12 -> 42 on the one-apple haggle and faded as the sale closed; joy 35 -> 69, pride 45 -> 73.
Failures: rapport barely moved (2 -> 4); the repeat check flagged her first reply ("Good day" to
"Good day") and retried, costing a call; "snug for the road" three times.
Changes (commit 72c10dcc): appraisal guide as data in emotions.json (sizes of emotion and rapport
changes); repeat defined as the character's OWN earlier wording or a near-verbatim copy; deltas in
the state audit line; beats moved into files.

## Run 4 (curious)

Median 6.6 s, max 9.3 s. Rapport toward the elf climbed 2 -> 17 over the run. Good human answers:
"Give it a gentle press near the stem; a ripe pear yields just a little." "Rain makes for a quieter
market, but the apples keep well; want a crisp one for the road?"
Failures: asked how long she has kept the stall she dodged (no such fact, and the backfill only
runs after the line, so it cannot help the line that needed it); the elf and the warrior answered
their own questions (13, 9, 11) because the beat reached them as the situation; Mara agreed that
two apples feed two people for a week instead of correcting them; "snug" still recurs.
Changes (this round): a line may fill an ordinary public gap in the moment (`filled` on the gap,
backstory or personality only, recorded after the line through the privacy judge, no second model
call); `intent` on `persona say` (what the character sets out to do, so a line meant to ask asks);
behavioural guidance moved out of the code instruction into a `conduct` list in emotions.json
(speak like a person, live by the trade, keep your own judgment, let feeling show in proportion,
attraction stays non-explicit, respect is earned back slowly); three more emotions in the default
list: attraction, disgust, embarrassment. Irritation 70+ is anger, 90+ rage (conduct guide).

## Next

Harder beats through the emotional range: anger and rage, fear, sadness, surprise, amusement,
disgust, attraction and embarrassment; track whether her state and her words move together and
whether she stays a fruit seller through it.
