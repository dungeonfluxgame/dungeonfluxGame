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

Correction (found during refinement 13): the host reads `emotions.json` from its scratch copy
(`artifacts/tmp/persona-live/playground`), and refinement 12 was written only to `assets/`, so
runs 20 and 21 ran without it. Their better orchard and provisioning answers were model
variation, not the refinement. The copy is now synced and the trap is in the runbook.

## Refinement 13 and runs 22 to 24 (the bar at night: Kael flirts with Lena)

Setup: `move act warrior plate=bar` moved the warrior alone from the market to a packed bar;
the design invented Lena (server), Tomas (fiddler), Rina and Elsa (dancers). Lena's persona was
generated from the owner's guidance as a hint only: she is working, hears lines all night and
takes none seriously, but may open up to a man who sees her as a person. The model turned that
into data: a default `brisk-service` stance, an `open-hearted` stance, a `sincere-interest`
trigger (model-judged), boundaries against unwanted touching and fake welcome, and a secret
(the flirting wears on her) that needs a sincere question. Beats:
`market-threads/flirt.beats` (20 rounds: cheap lines, a compliment, real questions, a crude
remark, an apology, loneliness, an invitation, jealousy bait, goodnight). The runner now takes
`@keeper` and situation settings from the beats file, so one script drives any scene.

Fixes on the way:
- Lena's first generation was refused: the model listed "patience" (a temperament number) as
  an emotion. The persona and temperament schemas now close every emotion field to the emotion
  ids (an enum), so the model cannot name another; code still validates.
- Names: residents were generated as "Dancer One" and "The Fiddler" because the hint dropped
  the design's name; it is now passed. The warrior is Kael, the elf Sylvi. A layout region
  carries a shown name (set by `identify` from the transition, or at runtime with
  `media locate label`); the SAM tag, the anchored speech bubble, `scene cast` and
  `media locate show` all show it.

Run 22 (stopped at round 10): the warrior introduced himself as "the Warrior"; rerun after
naming him.
Run 23 (`persona-transcripts/run23-flirt.md`): the words carried the arc well: deflection
("That line won't pour a drink, friend"), a boundary at the crude remark ("If your route
depends on my figure, you'd best get back to watching the road"), opening up when asked if the
attention wears on her ("you asked like you mean me, not the apron"), yes to the drink, rapport
to friendly (32). The feelings did not: irritation 66 at the first line, then 100 for the whole
run. Cause: a state trigger (`patience-thins`, irritation >= 75, irritation +12) re-added its
changes on every line while it held, and a pushed emotion does not fade, so it fed itself; it
also forced her brisk stance in the middle of a warm moment. Attraction never registered.
Refinement 13: a state trigger keeps its stance while it holds but adds its changes once, on the
line that crosses its level (test added). Data (live): a cheap or routine unwelcome line is
irritation +2 to 10, a crude one +15 to 30; romantic warmth toward one person (at ease, a yes to
time together) is attraction +5 to 15 and is not trust.
Run 24 (`persona-transcripts/run24-flirt.md`): feelings now follow the story: irritation falls
from 100 to 46 as he earns it, attraction appears when she accepts the drink (29) and grows to
35 at goodnight ("I'd like that; come by when I'm not carrying half the bar"), her stance moves
from brisk to open-hearted at his loneliness line and stays there. Still open: irritation spikes
too fast (93 by "Could I get an ale?"): her `another-advance` event trigger (+15) is judged to
fire on ordinary lines and stacks with the appraisal; joy and trust saturate at 100; rapport
ends lower (15) than run 23; one customer beat ("whether she ever dances") came out as a
question about the dancer.

## Refinement 14 and runs 25 to 29 (romance edge cases; goal hour from 16:41)

Run 24 still spiked: a plain "Could I get an ale?" took Lena from 57 to 93, because her
`another-advance` event trigger and the appraisal both reacted again to the flirt one line
earlier, and a compliment stacked the appraisal, the trigger's +15 and `patience-thins`
crossing (+12). Changes:
- Prompt: an emotional trigger's when_event fires when the line just said does it, never
  again for one already in recent; the engine adds a fired trigger's sets, so the appraisal
  leaves out what they change.
- Data: appraise what the line just said does now (an earlier line was already felt); two new
  emotions, affection (tender love, valence 80, arousal 30, fades 5% a turn) and jealousy
  (valence -50, arousal 70, fades 25%), with guide lines (affection builds slowly from being
  cared for, understood, missed or chosen, never from one compliment; jealousy rises only with
  attraction or affection already there, eases when chosen) and a conduct line (affection
  makes you tender and a little protective; jealousy shows sideways, never a scene or a claim).
- Runner: `@state <emotion> <level>` starts the keeper mid-story; long batches run a copy
  (an edit to the live runner garbled run 25's first attempt).
Results (transcripts `run25-flirt.md` to `run29-farewell.md`):
- Flirt (25): a plain order now eases her (55 -> 47); peak 91 only at the crude remark, back to
  40 by goodnight; friendly (43); affection appears at goodnight (24); "I'd rather have a dance
  with you than watch you get all the attention, though."
- Jealous (28, starts attracted): "Try not to look so pleased with yourself when you come back
  to me." "Just don't make me compete with the whole room for your attention." Jealousy 24 at
  "Mind if I dance with her?"; asked if she was jealous: "A little, maybe. But I liked that you
  chose to stay." Affection 23 -> 38, attraction to 100, embarrassment 23 at "I like you too".
- Farewell (29, starts fond): sadness 52 -> 73 inside affection 42 -> 90; "Months? I'll miss
  you, Kael"; "I'm falling for you too, Kael. So write"; the kiss: "Come here, Kael. You can
  carry that with you." Tender, never explicit.
- Shy (27): "I'd like that. Pick an evening when I'm not carrying half the tavern on a tray."
  But his fumbled "your braid's... very braided" fired her flirt trigger: irritation 32 -> 66.
- Pushy (26): Kael's own persona refused every pushy beat ("No. You said no. I'll keep my hands
  to myself"), so the test did not test her; her irritation stayed 74 to 96 even while he
  backed off, and rose at his parting tip.
Open: "mugs" and "keep it moving" recur within a run; a preset scene starts rapport at 0, so
she reads "indifferent" while saying "I like you too"; attraction can saturate within eight
lines.
Change after these runs (data): nervous, clumsy sincerity reads as endearing (amusement and a
little warmth, not irritation, unless it keeps pressing after a no). For the pushy test Kael
gets a temporary `drunk-pushy` stance in the scratch copy only, dropped after the run.

## Runs 30 and 31 (pushy with a drunk Kael; shy with the awkwardness line)

Pushy (`run30-pushy.md`, Kael on a temporary `drunk-pushy` stance, dropped after): Lena reads
like a real woman at work being pressed. "No, friend. I'm working, and that answer won't change
with a ring." "I'm not yours, and you don't get to decide who looks at me." At the hand grab:
"No. Keep your hands to yourself and step back, or leave." Fear rose with anger (to 54), the
standing went hostile (-45) and stayed wary after his apology ("Keep your word, and we're
square"). "You ignored my no, then blamed me for not warming to you."
Shy (`run31-shy.md`): the fumbled braid compliment now gives irritation 32 -> 49 (66 before),
amusement 69 -> 78, attraction rising to 41, embarrassment 24; "I'd rather hear what you meant
than a practiced line"; yes to the river walk.

## Refinement 15: standings set by hand

Scenes that start mid-story set feelings with `@state`, but rapport always started at 0, so a
Lena who said "I like you too" read as indifferent in her own facts. New: `persona state <id>
rapport <toward> <-100..100>` (`POST /dev/persona/state/rapport`), one `set_rapport` beside the
rapport the appraisal moves (they share the find-or-add of a standing; refused with
`invalid_standing` toward oneself or a non-name), and `@rapport <toward> <value>` in beats files
(jealous starts at 35, farewell at 55, rejected at 40). Two more scenes: `romance-rejected` (he
gently turns her down) and `romance-dancer` (Rina, who flirts with the room as part of her act).

## Runs 32 to 35 (preset standings; rejected; Rina) and refinement 16

- Farewell (`run33`): "Take this ribbon from my apron, Kael. Keep it close, and come back to
  me." Sadness 27 -> 56 inside affection 51 -> 100; friendly 57 -> 81.
- Rejected (`run34`): dignity without coldness or begging: "I won't ask you to stay, Kael. But
  I'd still like that evening, if you do." "I'm sad you're leaving, not angry with you." Sadness
  41 and surprise 39 at the turn-down; no embarrassment recorded (expected a sting).
- Rina (`run35`): flirts unlike Lena, warm and stage-confident: "I'm friendly with the room,
  but I save my attention for the company I choose." "You needn't match me, dear; just let the
  music find your feet." Attraction stays 0 while she flirts back: the flirting is her act. Kael's
  persona again refused the hand-on-waist beat, so her touch boundary went untested.
- Jealous (`run32`): she said "A little, maybe" but no jealousy was recorded (run 28 had 24):
  words and state disagreed.
- Across all four: joy, trust and attraction pin at 100 within a few lines (her
  `seen-as-person` trigger, +18 trust and +10 joy, fires on almost every warm line), so the top
  of the scale has no shades.
- Reliability: one Codex call stalled; the host's 600 s text deadline with retries held the
  run for about 30 minutes before a retry answered in 6.5 s, and the runner's `timeout 200`
  did not stop it on Windows. A persona line needs a far shorter deadline (open, not changed).
Refinement 16: above 50 a rise lands with the room left above it (a push at 80 moves half as
far as at 50; a fall always lands whole; `HEADROOM_FROM_LEVEL`, test added), so strong feelings
keep their shades; data: jealousy registers even when she plays it down in words.

## Runs 36 to 38 (headroom, then the parting band)

- Jealous with headroom (`run36`): nothing pins at 100 any more (trust 73 to 94, attraction 45
  to 72, affection up to 28); jealousy registered at "Mind if I go dance with her?" (24) and
  eased when he chose her ("I'd rather have your company than watch you dance"); "A little,
  maybe. But I'm glad you chose to stay with me." Words and state agree again.
- Farewell with headroom (`run37`): joy rose 62 -> 88 while he said he was leaving and sadness
  stayed 25 to 36: the model read each promise to write or return as joy.
- Data: someone they care for leaving is sadness +15 to 35 and joy -5 to 15 even when the
  parting is warm; a promise eases the sadness a little but does not turn it into joy.
- Farewell with the parting band (`run38`): sadness leads, 24 -> 72, above joy, while affection
  grows 42 -> 68 and fear for him on the road appears (29); "Months is a long time, Kael. I'm
  glad you told me, but I'll miss you." "Just don't make me regret letting you go."
- Bug found: her keepsake was "Take five apples, Kael; they'll keep on the road." The world
  bible has no bar, so the active world place is still the market and the fruit stall's goods
  reach her facts. Open: a bar place in the bible, and the transition setting the world place.

Where the romance work stands (goal hour 16:41 to 17:50): Lena deflects cheap lines, sets a
boundary at crude ones and at a grab, warms to sincerity, says yes to a drink or a walk, shows
jealousy sideways, grieves a farewell inside her affection, keeps her dignity when turned down;
Rina flirts as a performer; the feelings now follow the words with shades left at the top.
Open: her first reaction to a cheap line is still sharp (32 -> 55); the marriage proposal goes
to 100; "mugs" and "keep it moving" recur; no embarrassment at being turned down; Rina's touch
boundary untested (Kael's persona refuses the beat); a 600 s text deadline lets one stalled
Codex call hold a scene for 30 minutes; the world place after a transition.

## Run 39 and refinement 17 (whiplash: flirt to the top, then contempt)

Beats `romance-whiplash.beats`; Lena starts fond (attraction 70, affection 40, rapport 50). The
runner can now switch a stance on mid-scene with `! <dev command>` lines; Kael got a temporary
`cold-contempt` stance for three beats, dropped before his apology.
Words: warm to the top ("I'm glad you said it, Kael. I think I'm falling for you, too"), then hurt
("If that's all it was, Kael, you should've said so before I let myself believe you"), cold
("Then pass along, Kael. I've got better company to tend to") and a boundary without stooping
("Another ale, sure. You can quit calling me a wench while I'm at it").
Feelings undershot: sadness only to 60, irritation to 60, no surprise and no embarrassment at
being shamed in front of the room, and she stayed friendly (35); she forgave at once ("I can
forgive you, Kael").
Refinement 17 (data): sudden contempt or betrayal from someone trusted or loved has its own band
(surprise, sadness and irritation up 20 to 45, embarrassment when others can hear, trust,
attraction and affection down, rapport down 20 to 40); after cruelty from someone close an
apology brings trust back only 0 to 5 a line, and forgiveness is guarded and conditional. Rerun
owed.

## Refinement 18 and runs 40 to 43 (whiplash, the weaknesses of run 8)

Owner review of run 8 (the live whiplash scene): strong words, but every Lena line opened with
"Kael"; modern phrasing ("that's a lot to hear", "one evening at a time"); Kael's cruelty mild;
shock (28) and public shame (21) barely registered.
Changes (data, live): conduct lines to vary how a line opens (a name when it carries weight, not
as a habit) and to speak as someone of this world, with no modern idioms or therapy talk; a shock
band (a flip of manner: surprise 25 to 45 on that line) and a public-shame band (embarrassment 15
to 35 on top of the anger). Scene: a crueler temporary stance for Kael, and an apology turn that
names what he did.
Runs 40 and 41 (`run40-41-whiplash.md`): openers fixed (1 of 6 with the name), no modern idioms,
Kael's cruelty landed ("You were a fool to hope, Lena"). Run a stopped at turn 6: the addressee
judge named the speaker, the answer was refused, and the refusal ended the talk (and would have
left Kael's stance behind; run b's own drop removed it). Run b overshot: surprise 95, sadness 93,
embarrassment 88 at the end, because the model added shock on every cruel line after the flip.
Fixes: a refused or failed addressee reading falls back to taking turns (code); shock is only for
what is new, and each further jab adds less shame (data).
Runs 42 and 43 (`run42-43-whiplash.md`): both complete; openers 0 and 2 of 6 with the name
("Kael. Don't touch me", where it carries weight); no modern idioms; "Don't laugh at me, Kael.
Not after that." "Find another server; I'm done being your joke." "Your shame doesn't mend what
you did in front of them." Surprise spiked and faded (28 and 54 at the end), embarrassment 43 and
89 (b was shamed twice in public), trust down to 28 and 8, affection to 4. Open: Kael's apologies
stay short ("I'm sorry. I won't touch you again"): his own terse persona.

## Refinement 19: names are learned, never given (and a rules pass)

Owner question: "how did she know his name was Kael?" Every line was handed the others' public
persona (name, job, backstory) and `persona converse` labelled lines with names, so a stranger
knew a name nobody said. Owner rule: a name comes only from context, narrative or backstory.
Changes: a character's state keeps `known_names` (fresh: nobody, at most 64); each line's model
call reports `names_learned` (ids present in the scene, checked in code) from what was said aloud,
the situation or events, or their own backstory; someone whose name is not known reaches the
line with no name, no backstory, no voice samples and no expertise (only trade, manner and
speech: what anyone could see or hear); converse labels lines and seats by id; `persona state
<id> knows <other> [true|false]` presets it. Tests: a learned name is kept and an id not present
is dropped; a known name is kept once, can be forgotten, never points at oneself.
Runs 44 to 46 (`strangers.events`, and a topic saying they have known each other for years):
- First strangers run: Lena right ("First time at my counter, friend?"), but Kael said "Lena" at
  once: her public backstory begins "Lena is a young server". Stranger briefs now drop backstory,
  voice and expertise.
- Second: Kael still said "Lena": the events line read "sits down at Lena's counter", which is
  narrative and so a fair source. Reworded to "the server's counter".
- Third: right both ways ("Evening, friend" ... "Kael. And you are?" "Lena. Keep that eye on the
  door, Kael."); the narrative run learned both names from the topic alone.
Found: in the narrative run Kael called Lena "Mara": the line facts carry the shown scene's
description, still the market fixture's ("the elf, the warrior and Mara Vell at her fruit
stall"), because a transition swaps the picture but not the scene's words. Open.
Rules pass (independent review against AGENTS.md): no string matching for gameplay, no
input-driven panics, no leftovers. Fixed: frame tasks are stopped when a conversation ends early;
the frame and addressee judges share one route error (`persona/judge_route.rs`) instead of a
copy; the CLI gets their typed refusals through `reply_value`; the frame gate is decided by the
server from the peak the CLI sends; `learn_names` reuses `set_known_name`; an events file cannot
start `persona converse`; `parallel=0` is refused, not turned into 1; narrower visibility, docs on
the new fields, and the owed frame tests are marked `TODO(DEV-12)`. Left as they are (older
patterns, noted): blocking state-file IO in the line path (as `apply_appraisal` already does),
error names copied from the address judge, and an empty `display_name` as the stranger mark.

## Refinement 20 and runs 47 to 54 (the blade gauntlet at Haldor Brask's stall)

Owner ask: five adventurers each test the bladesmith with their own kind of question (expert,
ignorant, suspicious, haggling, ready to buy); he must be able to sell and want to close; the
responses recorded and graded for later refinement, with the 30 s display plate running.
Scene: `weapons-stall-blades.png`, `world place market`, `market-threads/blade-gauntlet.beats`
(12 beats, 24 lines; the band knows each other's names, Haldor knows none).

Changes:
- World bible: `coins` (gp 100, sp 10, cp 1; `name` is what follows a count), the four blades'
  values and Haldor's stall (5% posted-off bulk for two of a kind, floor 90%).
- Codex text: the prompt goes over standard input (`codex exec -`). Run 47 refused every Haldor
  line as `configuration`: four price tables pushed the prompt past Windows' 32,767-character
  command line.
- Haldor's persona (scratch data): he forged the blades (fuller, clay-backed quench, runes cut
  after the temper, woken by a scribe), edge-holding and care knowledge, a proof he can give
  (chill off the flat, frost a cup of water, never cut anyone), and `closing-the-sale`: matches a
  blade to the buyer, names the price, asks for the coin; gives ground slowly (about 5% first,
  the floor only at the very last).
- Price slots (code): a line never writes an amount. It writes `{price:N}` and `{order_total}`,
  and code puts in the total held, in the largest coins that fit. The model could not add or
  convert reliably: run 48 said 598g 50s for a pair, run 51 "722 gold and 5 silver" for
  361 + 247, run 52 "36 gold and 100 silver" for 361, and its self-reported total
  (`stated_total_cp`, tried in runs 50 to 52 and removed) did not match what it said. A
  line that quotes without slots, or names a lot it did not quote, is worded again once.
- Frames for the runner: `persona watch` reads `<speaker id> | <line>` from standard input and
  drives the same frame watch as `persona converse frames=`; the runner's `@frames` lines start
  it. The frame judge's instruction said "talk alone changes nothing", so a counter scene scored
  0% on all 24 lines of run 50. It now counts what the words make plain and no more: lifting or
  pointing at the thing answered about, a proof being given, grins at a gaffe, folded arms at a
  doubt, a handshake on a bargain.

Scorecard (Haldor's 12 lines per run; Y = right, n = wrong, - = not reached):

| Check | 48 | 49 | 50 | 51 | 54 |
| --- | --- | --- | --- | --- | --- |
| Expert craft answer (fuller, quench, runes after) | Y | Y | Y | Y | Y |
| Ignorant questions corrected kindly (hot blades, child's dagger) | Y | Y | Y | Y | Y |
| Suspicion met with fact, proof shown not claimed | n (creepy) | Y | Y | Y (anvil flare invented) | Y |
| Four prices right, in gold | Y | Y | Y | Y | Y |
| Edge-holding answer (the Verdant) | n (hedged) | Y | Y | Y | Y |
| First concession modest | Y (5%) | n (floor) | Y (5%) | Y (5%) | Y (5%) |
| Pair total said = lots held | n (598.5 vs 640) | n (638 vs 640) | n (606.5 vs 608) | n (722.5 vs 608) | Y (608) |
| Closes and asks for the coin | n | Y (floor) | Y (430) | Y (430) | Y (427.5) |
| Care advice | Y | Y | Y | Y | Y |
| Frames shown (gate 25, hold 30 s) | - | - | 0 | 4 | 2 |

Frames: run 51 made four (lines 6, 12, 19, 24; each shown 110 to 132 s after it started; wide,
over-the-shoulder, wide, over-the-shoulder), faces and blades kept. Run 54 made two (lines 13 and
17), one a two-shot of Sella's palm flat on the Frostline with frost beading on the steel as
Haldor offers the chill.

Weaknesses to refine later:
- Verbal tics: "friend" in all 12 Haldor lines; "Shall I wrap it?" closes two lines in a row.
- The first wording often writes the number instead of the slot (2 of 4 priced lines in run 53),
  costing a retry (about 7 s).
- A concession is not always remembered: run 53a priced the Emberfang at 380 for "all four"
  after giving 361 two lines before.
- Frames come late in a quiet counter scene (none before line 13 in run 54); the judge is honest
  about small changes, and the gate (25) is the owner's.
- Small inventions under the appraisal stance: "the Emberfang flare on my anvil" (run 51).

Transcripts: `persona-transcripts/run48-gauntlet.md` to `run54-gauntlet.md`
(`run52-53-bundle.md` for the bundle checks).

## Refinement 21 and runs 55 to 57 (Haldor's voice; the camera on an item)

Owner ask: refine the weaknesses of refinement 20, and add the camera: when someone inspects an
item, the display plate fills the screen with it so the players at the table can see its quality,
with the backfill engine giving more detail about the item.

Voice (data): the shared conduct rule on openings now covers forms of address and closing
questions (no "friend" or "Shall I wrap it?" as a habit; most lines need no address). Haldor's
address is "now and then 'friend' when he means it"; new samples; "proud of his own craft" and "a
dry, warm humour"; four stances, each with a trigger the model judges: `stung-pride` (his work
doubted), `shop-talk` (a question only a smith would ask), `patient-tease` (an ignorant question)
and `show-the-blade` (a close look asked for).

Camera (code):
- The frame judge names `inspected_item`, an id from the items offered at the active place
  (`world::place_items`, checked in code; null when nobody looks closely). A new inspection cuts
  at once, whatever the gate and inside the hold, to `inspect_shot` (`insert` in `frames.json`:
  the item fills the frame edge to edge). The judge writes the close-up from the item's tags and
  what was said.
- An insert is not scanned and is shown with the room's layout, so the cast stays; the room
  comes back after `insert_hold` (20 s, a CLI option), and the next room frame edits the room.
- Before an insert, the backfill engine is asked, for the keeper of the item's stall, about the
  making of it and how it looks up close; the text goes into his public backstory (expertise
  stays authored, per the persona design) and into the insert's prompt.
- A frame stays up at least 8 s before one made alongside it replaces it.

Runs:
- Run 55 (`run55-gauntlet.md`): the voice landed. "Cook supper on it? You'd spoil a fine blade
  and a poor stew." "Painted? I forged these myself." No habitual "friend". Inserts for the
  Verdant, Frostline and Nightglass. Found: an insert's own (empty) scan emptied the cast, and
  three adventurer lines after it were refused `unknown_character`.
- Run 56 (`run56-gauntlet.md`): no line refused through four inserts; the room back after about
  21 s. Found:
  - A room frame flashed by when an insert queued behind it.
  - The cut came back to an older room.
  - The last insert was taken down at once.
  - The item backfill was refused `not_fillable`: expertise is not a backfill field, by design.
- Run 57 (`run57-gauntlet.md`): every line answered. Three inserts, each with one backfilled
  detail: "the Frostline's slender silver-steel blade was folded and pattern-welded ... the
  guard is plain and firm, the grip wrapped for a sure hold, and the rounded pommel balances the
  blade"; the Nightglass's "near-black blade, its violet sigils dimming in shadow ... the bone
  handle fitted close to the tang". Measured: a frame up 8.1 s before the next replaced it;
  inserts up 20.2 and 24.6 s; frames made in 67 to 134 s.

Open:
- "Certainly, here, hilt first" opens all three close-look answers: a stance's habit the
  conduct rule does not reach yet.
- The Frostline insert shows silver runes where the stall plate glows blue: the backfill wrote
  "catch the light in silver".
- Each run appends another backfilled paragraph per item to Haldor's backstory, because the
  backfill does not know it described the blade before.
- Bram's haggle beat was once worded backwards ("That's a fine blade, not a dear one").
- While an insert is up, bubbles sit where the room's faces were.

## Refinement 22 and runs 58 to 64 (no pile-up, no lost close looks, no name leaks)

Owner asks: fix the repeated close-look opener and the backstory pile-up; then, after Haldor said
"Mara" unprompted (run 59), make sure nothing in a scene leaks a name, nothing is hard-coded and
no gameplay decision uses string matching: decisions go to Jev and into prompts and data.

Changes, in the order the runs found them:
- **Pile-up (run 57):** before an insert, the coverage judge is asked whether the keeper already
  knows the item up close; only then the backfill (run 58: "already knew" on all four inserts,
  the backstory unchanged at 1631 characters). The frame judge reads each keeper's backstory.
- **Openers:** "Certainly, here, hilt first" (run 57), then "Aye" (run 58). Fixed as data: the
  show-the-blade stance and Haldor's rhythm ("never two answers running with the same word").
  Varied from run 59 on.
- **Shots (run 58):** the judge took the insert shot for a gated frame. The insert is no longer
  among the shots it picks from.
- **Name leak (run 59): an audit of every path a name takes into a prompt.** Fixed:
  - **Fixture words:** a scene fixture's title and description are no longer given to a line,
    a backfill or a generation. They are author notes ("Mara Vell at her fruit stall"); the
    world's place says where people are.
  - **Ids that carry names (`mara_adventurer`):** a character never sees the id of someone whose
    name it does not know. Everywhere the id would be, it sees a handle (`person-1`):
    `present`, `asked_by`, `recent` labels, `others`, its own standings and attention, and the
    keeper of a stall in the world facts. The model's answers are read back to ids in code
    (`persona/handles.rs`); a `recent` label that is no one it knows becomes `someone`.
  - **Backfill:** it was given the others' full public views (names included), and would store
    a name for good. It now sees them as the line does, with the same handles in the same order.
  - **`persona converse`:** it put ids and lines into free text, with a hard-coded "table at the
    inn". It now sends `recent` as data and names no place.
  - **Generation:** gets the world's place, and the cast as strangers.
  - **The shared public note** tells every prompt that a handle is no name, never to be said or
    written into a gap or detail.
- **Lost close looks:** run 60 lost two to busy room slots, run 62 two more to a busy insert
  slot. Inserts now have a pool of their own, wait for no room frame, and hold the screen; a
  room frame that finishes meanwhile waits.
- **Cast loss (run 61):** an extreme close-up named only the keeper, and three lines were
  refused. A frame whose scan misses some of the cast is shown with the cast of the last full
  picture.
- **Into the judge and the data:**
  - Whether a close look is new is the judge's `new_look`, not a code comparison.
  - The insert hold, the least time on screen, the question asked of the keeper and the insert's
    lead-in are `frames.json` `show`.
- **Independent rules review (a reviewer agent on the diff):** the fixes above for titles, stall
  keepers, unknown labels and generation; one stranger-view function, not two; one layout
  helper, not three; weapon-specific words out of the generic frame instruction; stale docs.
  One claim was refuted: the insert hold has a single clock, the task's finish.

Run 64 (`run64-gauntlet.md`):
- 30 lines answered, none refused, no name, handle or "someone" said.
- Exactly one insert per close look (Verdant, Frostline, Nightglass), each "already known", so
  nothing was backfilled.
- A tight room frame (5 of 6 named) was shown with the full cast; inserts were up about 20 s.
- Prices right (380 to 361, the pair 608, the Frostline 427 gold 5 silver).

Open:
- Run 63's judge took a care question as a new Nightglass look.
- One insert in run 61 was lost to a transient image backend error (-32603); there is no retry.
- A stranger's job, personality and speech style, the bible's place text and caller text (topic,
  intent, events) are authored, and could name someone if written carelessly.

## Refinement 23 and runs 65 to 69 (the arrest: the storyteller plots the Nightglass Charge)

Goal (owner, 2026-11-08): Kael is framed and arrested at the stall, dragged through town and
judged by the mayor, with a plotted backstory and personas for everyone interactive.

- **The storyteller (`persona story`):**
  - One plot call against a closed schema: the charge, the truth, the culprit, evidence with
    flaws, beats per scene, and a `NarrativeGuidance` per character as it stands before the
    first scene.
  - `apply` hands each character only its own guidance, through generation or a guided
    backfill. The first apply wrote the plot's future into Kael's backstory; nobody remembers
    their own future now.
- **Every interactive character has a persona and shows its name:**
  - Sergeant Alwen Dorr, Tavin Rusk, Osric Pell, Nessa Vale, Mayor Edrin Vale.
  - Their tags and bubbles show display names, never ids.
- **The look:** grounded period drama. Plates remade as `-real` with explicit tone-down prompts.
- **Runs 65 to 67 (`run65-arrest.md`, `run66-march.md`, `run67-judgement.md`):**
  - The scenes held.
  - The runner had the keeper answer questions put to others (fixed with the addressee judge,
    told the speaker's goal, and a self-skip).
  - The judgement was agreeable until a dispute conduct rule and stances went in.
- **Run 68 (`run68-judgement.md`):** the acquittal is earned in play: the fake is ruled out and
  the guard's sightline falls apart. Open:
  - everyone calls the bought guard "Sergeant" after the mayor did;
  - Kael answers twice where a question to him meets his own beat.
- **Plotted moves (`persona story moves <scene>`), refined over six plots:**
  - Each move is `speaker`, `to` (an id or `room`) and a goal: an order to the speaker, the
    listener named by look.
  - Code refuses: a move put to oneself (plot 1: the mayor asked himself); a judgement where the
    accused has fewer than three moves (plot 3: one); anyone not in the shown picture (the arrest
    plot had Mara plead to the mayor at the stall).
  - Plot 2 and plot 4 came back word for word: the question carried the old moves. They are
    cleared now.
  - Plot 5 wrote "as a question to the bought guard", echoing the ids. The storyteller now sees
    `person-N` handles, read back in code.
  - Plot 6 scripted no answers and named no hidden part.
  - Still open: the defence asks after "the fresh coin" in the guard's purse before anyone has
    seen it (the plot's beat text says the purse gives him away; the beat should not say how it
    ends).

## Refinement 24 and runs 69 to 72 (the judgement from plotted moves; the photographic look)

- **Run 69 (`run69-judgement.md`), the first played from plotted moves:**
  - No name or handle leaked, and nothing was refused.
  - Haldor, never shown the fake up close, swore it was his own blade ("not paint").
  - Three double answers: a question to someone, then their own plotted move.
  - The defence fished for "the fresh coin" in the guard's purse.
- **Fixes:**
  - The runner lets a beat by the one just asked stand as their answer.
  - A refused moves answer is asked once more with `previous_refusal`.
  - The moves rule says a hidden flaw (a payment) is left for play to find, and an answer-move
    carries the aim of the answerer's own side.
  - The plot's judgement beat had said how it ends ("the purse reveals a payment"; it was
    edited out of the scratch plot).
- **Exhibits:** evidence on show gets `up_close`, what anyone sees of it as it truly is.
  - The runner adds it to every situation, so Haldor finds the loose handle and painted marks
    himself (run 71, [08]).
  - Open: the storyteller writes names into exhibits ("Haldor's handle", "Rusk claims"); they
    were made name-free by hand for runs 71 and 72.
- **Run 70 was stopped at line 33:** Kael "remembered" run 69's acquittal mid-hearing (his
  attention held the mayor's verdict). The judgement now resets every participant first, then
  presets the names each has learned.
- **Run 71 (`run71-judgement.md`):** clean start, one verdict, no refusals. The mayor acquits
  on all three flaws found in play. Open:
  - a question inside a keeper's answer goes unanswered ([13], [16]);
  - the plot told the bought guard to concede his blocked view;
  - the accuser's invented chain of custody, contradicted by the sergeant, is never seized on.
- **The look (owner: "too fantasy and painted", then "not photographic, it's like an
  illustration"):**
  - The house direction asked for a warm candlelit glow, creamy depth of field and teal and
    orange, so frames came out golden and waxy. It is now an unretouched photograph from the
    set of a TV drama: camera and lens named, natural light, real skin, neutral grade.
  - An edit keeps its base's finish, so the hall is a fresh photographic plate
    (`hall-photo-fresh.png`).
  - `media generate` takes `look=house|none` (house by default).
- **Run 72:** the judgement on the photographic hall (graded below when it ends).

## Timeline (commit times, 2026-11-07)

12:11 persona engine (emotion layer, one call per line, lean Codex) -> 12:18 refinement 1 ->
12:30 refinement 2 -> 12:39 refinement 3 -> 12:43 refinement 4 -> 12:53 refinement 5 -> 12:59
refinement 6 -> 13:03 refinement 7 -> 13:10 refinement 8 -> 13:15 refinement 9 -> 13:20
refinement 10 -> 13:28 refinement 11 -> runs 18 and 19. Nineteen runs of the market thread over
about 90 minutes, across six beat files (haggle, curious, temper, heart, supplies, rage). Then
refinement 12 (runs 20 and 21, data not live, see the correction) and the bar at night:
refinement 13 with runs 22 to 24 (flirt).
