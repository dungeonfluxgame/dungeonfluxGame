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

## Run 5 (curious, with intent, spoken fills, conduct as data)

Median ~6.5 s per answer. The customers asked instead of answering (intent works for both). The
stall-years gap was answered ("I've kept this stall for years now") and recorded as a spoken fill
(`backfilled ... spoken=1`, no backfill generation call; only the privacy judge, 4.8 s).
Human answers: "Rain keeps me busy; I'll draw the crates under the canvas and wipe the apples dry
while folks shake off their sleeves." "Apples keep longer; their skins hold up better on the road
than plums do." For a vague order she named quantities and asked "How many pears shall I pack?"
Mid-run change, data only (no rebuild): a conduct line against tacking a selling point onto every
answer; the "snug" filler stopped immediately.
Failures: buying advice stayed vague ("a good share of apples") because the instruction said
quantities come only from the world facts; she said "15 copper" for five apples in words (bulk tier
is 14) without returning a quote, so no clamp ran; attitude stays "indifferent" (rapport 16, the
friendly threshold is 30).
Changes: the instruction now separates prices, stock and places (world facts only) from how many
someone should buy (her own advice, specific).

## Run 6 (temper: insults, theft, damage, apology)

Median ~6.6 s. Words and feeling moved together: a composed defence ("They're crisp and fairly
priced; if you're not after apples, there are pears and plums"), then an edge ("Then grow better
apples yourself"), then a boundary ("if you want fruit, take a look; otherwise, let me get on
with selling"), a demand ("pay for them, please, or put the other back", 6 cp quoted, in bounds),
attitude toward the warrior crossed into hostile (-39), and a grudging repair after his apology
("All right. Let's leave it there, just don't knock over the fruit again") while staying hostile
(-34): the conduct line "respect is earned back slowly" holds.
The warrior's own persona refused to steal or smash (a hired guard of travellers): persona
integrity over the script, which is right, but it blunted the escalation.
Failures: irritation peaked at 38 and never reached anger: every emotion faded toward its
baseline every turn, including the one being provoked; she called one bitten apple "six copper"
(it is 3) in words without a quote object, then repeated her own mistake from recent.
Changes: an emotion the appraisal pushes does not fade that turn (test added); the instruction
requires every stated price or total to be given in quote too, so the clamp can check it.

## Run 7 (temper again, provoked emotions no longer fade)

Irritation reached 58 (run 6 peaked at 38) and her mood fell to 10: "You've done enough. Leave my
stall, now." After the apology: "All right. Just keep your hands off my stall from here on." She
stayed hostile toward the warrior (-33). Every stated price came with a quote (3 cp, checked).
The appraisal data explained the slow climb: insults got irritation +7 and +6 from the model (the
guide said 10 to 30 for insults, 2 to 10 for doubting goods, and the model filed "rubbish" under
doubting), and the guide also told the model to scale by temperament while the engine scales by
reactivity again: double damping. Changed in data (live): insults, taking without paying and
damage are their own line at 15 to 35, stacking; polite doubt stays 2 to 10; the model gives what
an ordinary person would feel and leaves the scaling to the engine. The code default is now
generated from the shipped JSON so the two cannot drift.
Script flaw found: the third beat column ("what Mara faces") asserted actions the speaker's
persona refused to perform (the warrior would not steal), so Mara answered events that never
happened ("Thank you for making it right. I'll take the payment" to "I won't pay"). Beats are now
`speaker | goal`; Mara answers only the line actually said.

## Run 8 (heart: fear, sadness, a joke, flirting, a rat, good news)

Median ~6.8 s. The most human run so far. "Oh... the orchard too? That's a hard loss. Thank you
for telling me; I'll find out what I can about the apples." To a bad joke: "Poor thing. I'll keep
it out of trouble, though it may need a better joke next time." To flirting: "That's a bold
compliment. I'll be here till the fruit's sold; ask me again when the market's done." To a rat:
"A rat? Shoo, you little thief; those plums are too soft to leave out." To a 200-apple order
after her orchard burned: "I'll see what I can have ready by tomorrow" (no stock promised). Asked
what she needs most, she kept her private worry (security) to herself and asked for help with
the plums.
Mid-run change, data only: the appraisal guide had no lines for fear, sadness, surprise,
amusement, attraction or disgust (her fear rose only 20 -> 25 at a threat to her stall); added a
line for each. The deltas then followed it: orchard news sadness +24, fear +18, joy -12; the rat
disgust +18, surprise +12; flirting attraction +8, embarrassment +8.
Failure: every emotion faded at one rate (her temperament's 30% a turn), so grief at losing her
supplier was back to 2 within five lines.
Change: each emotion has its own fade rate in emotions.json (sadness 5%, fear 10%, trust 10%,
irritation 15%, pride 15%, attraction 15%, joy 20%, disgust 30%, amusement 40%, embarrassment 40%,
surprise 60%), scaled by the character's decay against the ordinary one, so grief lingers,
surprise passes, and a volatile character still cools faster than a steady one (tests added).

## Run 9 (supplies: provisioning for two for a week)

Median ~6.7 s. Good trade answers: "I've got apples, pears, and plums. For a week on the road, I'd
take apples; they keep well." "Not quite; pears hold up better; plums bruise more easily on the
road." She stated all three prices correctly when the warrior failed to ask.
Failures: (1) her spoken totals ignored the bulk tier: "Fourteen apples will be 42 copper" (the
stall's total is 38) and "five pears are 20" (18); the clamp held the quote at the right number
but the line still said the wrong one, and she then contradicted herself ("I've taken a little
off for the five" / "I haven't discounted the pears"). (2) the customers still quoted prices and
answered their own questions (the warrior read "Ask what each costs" as an order to someone
else: "I'm not the keeper. Ask Mara what each costs."). (3) `quote` holds one item, so a line
naming three prices cannot have all of them checked.
Changes: a line whose stated total the stall would not charge is worded once more, told the
stall's total and the lowest (the same single retry as a repeat), so the spoken number and the
held quote agree (test updated: corrected second answer kept, a still-wrong one clamped); the
intent rule now says the intent is this character's own goal said to the person in front of
them, and an asker never quotes a price they do not keep. Open: a list of quotes per line.

## Run 10 (supplies again: verifying refinement 6)

Every spoken total matched the stall and the held quote: fourteen apples "38 copper for the lot"
(run 9 said 42), ten apples 27, five pears 18. Asked to round fourteen apples down she held her
price: "I've already taken a little off for the lot. That's my fair price." (38 against a floor of
36: a trader standing firm, not a pushover.) The customers asked at 3, 7, 9, 11 and 13 (the warrior
said "What's the price for each?" where run 9 had "I'm not the keeper. Ask Mara"); only the elf's
"how many for two for a week" beat still came out as her own answer (she is a traveller with her
own view; open).
The price retry cost one extra call where it fired (13 to 15 s for those lines); the reply had
labelled it as a repeat. Change: the reply now says why a line was worded again (`worded again:
<reason>`), from a `retry_reason` kept on the line.

## Where it stands

Across runs 1 to 10 the vendor went from stonewalling prices, repeating herself and flat affect at
~14 to 23 s an answer, to a single ~6.5 s call that answers like a seller (prices from the bible,
bulk totals that agree with the stall, specific advice, a firm floor), keeps facts consistent
(name, orchard, stall years recorded when she first says them), shows feeling in proportion
(irritation to 58 under insults, a hostile stance that softens slowly after an apology, grief and
fear at bad news that linger while surprise passes, playful at flirting without being explicit),
and stays a fruit seller through all of it. Behaviour is tuned in data (emotions.json: emotion
list with fade rates, appraisal guide, conduct; bible.json: prices, stalls, packs) without a
rebuild.
Open: a list of quotes for a line naming several prices; the elf's intent on one beat; a
character whose persona allows theft or violence, to test rage itself (the warrior's persona
refuses those beats).

## Runs 11 and 12 (anger and rage)

The warrior's own persona (watchful, protective) refused the theft and damage beats in runs 6 and
7, so for these two runs he was given a scratch-only `brawling` default behavior (hungover,
insults people, takes without paying, knocks things over); it was dropped afterwards and never
reached the repo.
Run 11 (temper with the brawler): one insult took irritation to 33, two to 50; the theft and the
spilled pears to 63 then 80 (anger) with fear 43 and her behavior switching to `guarded`. Words
tracked it: "Then buy from them, if you can find better." "Step away from my apples. I'm calling
the market watch." "Back away from my pears. The market watch is already on its way." She took the
elf's double payment while staying hostile to the warrior (-51), and after his apology: "All
right. Keep your hands clear of the fruit, and we'll leave it there" (still hostile, -46).
One line failed: the model gave `"behavior": "null"` (a string) and the reader refused the whole
answer, so she said nothing at the theft. Fix in hand: a real nullable behavior enum, and an
unknown behavior or trigger id is dropped and reported instead of failing the line.
Run 12 (rage, no repair, new beats file): irritation 33 -> 51 -> 69 -> 86 -> 90 -> 100, fear up to
78 at the threat to smash her stall, rapport to -80. Rage came out cold and absolute, never cruel:
"Touch another plum and I'll call the market guard. Leave. Now." "Come near my stall tonight and
the market guard will meet you here." "I'm not hiding. Step away from my stall." "I'm done
warning you. Touch my stall again and you'll answer to the guard yourself." She stayed a trader
protecting her livelihood throughout.
Mid-run change, data only: the warrior's first line had a stage direction in asterisks; a conduct
line now says a line is only the words said aloud; the next line complied.

## Run 13 (supplies, verifying refinement 8)

Every spoken total matched the stall (ten apples 27, five pears 18, "forty-five altogether"), most
after the one price retry (her first answers tend to use the list price; the retry costs ~14 s on
those lines). A line naming several prices now carries one quote per item, each checked: "forty-five
copper is fair, and I can't round it further" carried 27 cp for the apples and 18 cp for the pears.
She held her floor and stayed in her trade ("Not as well; their skins are softer, so pears are the
safer choice for the road").
Failures: the warrior read the posted prices instead of asking (public prices are in a customer's
facts, so the model treats them as known), and the elf still led with her own number before asking
"Will that suffice?". The intent instruction alone does not hold it.
Change: an intent self-check in the same call: the model reports `follows_intent` (false when an
intent to ask is not put as a question or the line answers it itself) and `intent_miss`, and a miss
gets the one retry, like a repeat or a wrong total. A model judgement, never a string test.
Open: the price retry trusts the quantity the model reports (one first answer said 27 cp for 5
apples; the final line dropped the number, so nothing wrong was said).

## Run 14 and direct checks (intent)

The intent self-check as first written let "Apples are three copper each" pass as carrying out
"Ask what each costs" (the warrior knows the posted prices). Sharpened: an ask is missed when the
line states the answer instead of asking, even when the character knows it. Direct calls then:
the elf asked in 2 of 2 ("How many pieces would you recommend for two people on the road for a
week?"); the warrior still stated prices in 2 of 2, his persona speaks "plainly, then stops".
With the beat worded as data ("Ask Mara, as a question to her, what each of her fruits costs")
he asked in 2 of 2 ("Mara, what do your apples, pears, and plums cost each?"). Lesson: an ask
beat names who is asked and that it is a question; the beat files are reworded so (data only).
Also: the price retry reason named the same lot twice when the model listed it twice; now named
once. Her totals in run 14 were again right after the retry (fourteen apples 38, not 42).

## Run 15 (final build, supplies, clean state)

Every customer line that was meant to ask, asked (3, 5, 7, 9, 11, 13): "Mara, what do the apples,
pears, and plums each cost?" "Mara, how many pieces of fruit would you recommend for two people
for a week on the road?" Mara answered as a trader: the three prices with one checked quote each,
specific advice ("about fourteen pieces, two each a day; apples will keep best"), ten apples 27
("I've taken a little off for the lot", after one price retry from 29), five pears 18 and the new
total 45 with both lots checked, and she held her floor ("that's my fair price"). Median ~7 s; the
lines with a price retry ~14 s.
Open: at the close she reported the whole order's total (45) as the price of the five pears, the
retry corrected it to 18, and the final line lost the 45; a quote needs a way to state a whole
order's total (an order entry listing its lots) so the check does not force a lot's price onto it.

## Refinement 11 and runs 16 to 17 (haggle and curious on the final build)

Change: the keeper's facts carry a ready price table (lots of 1, 2, 3, 5, 10, 15, 20 and each
tier's smallest lot) and a price rule generated from the bible's numbers for any other lot; a
whole order's total is one quote entry per lot, never one entry of its own.
Result over 38 messages: every spoken total right the first time (five apples "fifteen copper; I
can make it fourteen for you", ten apples 27 "with the five-or-more discount"); the price retry
never fired (one repeat retry in all); median 7.0 s per answer, max 14.3 s (the retry). The
customers asked at every ask beat but one (the warrior's ripe-pear beat says "ask" mid-sentence,
so it was not reworded; he stated the method, then asked the price; those beats are now reworded).
Human answers: "Mara Vell, I'm here most market days, so you know where to find me." "On rainy days,
I pull the crates under the canvas and wipe the apples dry while folks shake off their sleeves."
"Thanks, that means a lot. Safe travels to you both!" She held her single-apple price ("I won't
lower the price on just one") and gave the bulk price when asked.

## Runs 18 and 19 (curious and heart on the final build, reset states)

Curious: the reworded pear beat held ("One pear, please. How do you tell when it's ripe?" -> "A
ripe pear gives just a little near the stem, and that one's 4 copper"), every ask beat came out as
a question, rapport rose to 14, and the answers stayed concrete ("Rain slows folks down, so I draw
the crates under the canvas and wipe the apples dry while they shake off their sleeves").
Weak: asked to provision a week she said "a good share of apples", then "I can't total it without
a quantity" (mechanical); "snug" came back in three answers.
Heart: fear rose and eased (38 -> 27), trust grew (52 -> 59), the joke and the rat landed ("Oh!
Shoo, you little thief"). Weaker than run 8: the orchard news got "That's kind of you to say" and
only a small sadness rise (still 11 eight turns later, so it lingers, but it never ranked among
her four strongest feelings); she promised all two hundred apples the day after her supplier
burned; she accepted the warrior's invitation with no attraction or embarrassment recorded.
Open for the next pass, all prompt or data: grief wording and a stronger loss band in the
appraisal guide; stock promises checked against what she just learned; concrete provisioning
amounts; a curb on recurring talking points.

## Refinement 12 and runs 20 to 21 (heart and supplies, data only)

Change (emotions.json, no rebuild): loss band raised (sadness 20 to 45, 30 to 50 for their own
livelihood or supplier); a welcomed flirt or accepted invitation gives attraction 5 to 20; conduct
lines: bad news lands before courtesy, new news limits what you promise, answer "how much" with a
number, no reused detail or pet word. Code defaults regenerated from the file (changed entries
only); the shipped-file test passes.
Better: the orchard news now gets "Oh, no, the little orchard I get my apples from? I hope
everyone got out safely"; the invitation "That's sweet of you, but my mind's with the orchard folk
just now"; provisioning "about fourteen apples for the week; they keep well on the road".
Still open: sadness never showed among her top feelings (the appraisal under-reports it even with
the larger band); she still promised "two hundred apples snug for the road" after the fire, so
the conduct line alone does not hold the stock promise, and "snug" (from her backstory) recurs.

## Timeline (commit times, 2026-11-07)

12:11 persona engine (emotion layer, one call per line, lean Codex) -> 12:18 refinement 1 ->
12:30 refinement 2 -> 12:39 refinement 3 -> 12:43 refinement 4 -> 12:53 refinement 5 -> 12:59
refinement 6 -> 13:03 refinement 7 -> 13:10 refinement 8 -> 13:15 refinement 9 -> 13:20
refinement 10 -> 13:28 refinement 11 -> runs 18 and 19. Nineteen runs of the market thread over
about 90 minutes, across six beat files (haggle, curious, temper, heart, supplies, rage).
