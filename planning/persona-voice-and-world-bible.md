# Persona voice and the world bible

Date: 2026-11-07
Status: Design adopted; implementation in progress. Governs `persona/*` prompt assembly and the
new `world` module. Related: [scene-transitions.md](scene-transitions.md),
[narrative-engine.md](narrative-engine.md), [long-horizon-state.md](long-horizon-state.md),
`development/runbook-dev-playground.md`.

## Why

The 20-message market thread (2026-11-07) showed Mara Vell's replies were flat. Causes, from the
run, and the fix for each:

| Cause seen in the run | Fix |
| --- | --- |
| `speech_style` is one vague sentence, so lines come out generic | A structured **voice block** with sample lines; voice packs inherited from a community |
| No prices or stall facts: she dodged ("I haven't set the total yet"), a backfill invented one and the private-detail guard blocked it | A **world bible** of canonical facts and price rules; code computes quotes, the persona only voices them |
| A fruit seller who "doesn't know what keeps best" | **Expertise** on the persona: what they know from their trade, covered without backfill |
| Each `persona ask` saw only the last line, so she repeated herself ("for the road" five times) | `recent` lines passed to ask and say; the scene text states only what is happening now; an anti-repeat rule |
| A malformed backfill answer was refused and the character stonewalled | The backfill is retried once with the refusal reason; a still-failing backfill never blocks the line |
| The discount trigger fired on a line that only accepted a price | Trigger judgement is told who spoke |
| 9 to 17 s per answer | Measured and reported; not changed here |

## Voice

A persona gains `voice` (all fields optional, old files still load): rhythm (sentence length,
formality), diction (favoured words, words never used), tics (openers, closers, fillers, with a
limit on how often one may appear), address (how they speak to strangers and friends), idioms,
deflection, and `samples`, three to five lines marked as style samples and never as facts.
`voice_packs` names the communities the character belongs to. Layering, weakest first: pack, then
persona, then the stance of the behavior in force (behaviors already carry a `speech_style`).

A voice pack is a world-bible entry: id, description, lexicon, idioms, samples. Packs are keyed
by community (a town, a trade, an in-world culture), never by race, gender or nationality, and
dialect is carried by word choice, rhythm and idiom, never phonetic spelling. Generation assigns
packs from the closed list in the bible by a model call (Jev), never by matching words.

Sample lines are copied too easily: the instruction forbids reusing one verbatim.

## World bible

A content file under the playground folder (`world/bible.json`), loaded by `world`:
`items` (id, name, unit, `base_value_cp`, tags), `places` (id, name, stalls: who sells which item
ids), `voice_packs`. The code computes **price quotes**: base value, a bounded modifier chosen
from circumstances and the persona's stance, a floor; the model words the quoted number and may
choose a position inside the allowed range, which code validates and clamps. A backfill never
invents a price, quantity or place; those come from the bible. The active place scopes which
facts enter the persona prompt (structural, not matched against text).

## Flexibility

Everything a designer varies is data: the bible file, the packs, the modifier ranges, the sample
lines. New kinds of fact are new fields with defaults. `world::context_for` is the single seam
between the persona prompts and the bible.

## Checking it

Blind attribution: a Jev call is shown a line and the cast and names who said it. Higher accuracy
across the market thread means the voices are distinct. It is repeatable and measures voice, not
liking. The re-runnable exercise is `development/market-thread.sh`.
