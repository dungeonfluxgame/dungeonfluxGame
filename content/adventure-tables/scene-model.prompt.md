Create the structured, authoritative model of this scene before writing any prose.

<!-- Why: the model is the shared source for the scene's narration, characters, actions, and picture. -->
Use only the supplied adventure facts, cast, place, existing findings, and scene requirements.
Keep named cast IDs and names exact. Background people may be invented for this place, but each
must have a stable ID, a job, a position, and a truthful relationship to the named cast.

<!-- Why: a picture must not introduce a person or prop that the server cannot resolve. -->
List every visible person and foreground object that the plate should depict. Give each a
player-friendly position phrase such as "at the back on the left", "by the hearth", or "on the
bar in front". Include who owns each object and whether it is currently in use. Do not place
unlisted people or objects in picture descriptions. Every portable object is one stable item
instance: give it an optional provenance object with `kind` (`authored_location` or
`generated_offer`), a stable `source_id`, and a seller ID only for generated stock. Ownership,
current holder, and room position are separate fields; never replace an item with a second copy
when the plate is refreshed.

<!-- Why: an NPC must answer about the same room the players see, without inventing facts. -->
For each person, provide three short knowledge entries: one about this place, one about the
errand or enemy that exactly matches a supplied fact, and one about themself. Give that person a
single want and an initial Friendly, Indifferent, or Hostile attitude. Give each exit a known
destination and a concise description of what is visible beyond it, including the supplied time
of day and weather.

<!-- Why: clues must support established truths without changing them or handing out outcomes. -->
Place observable evidence for supplied fact IDs only. A clue describes a physical trace or
first-hand observation, never a new fact, person, place, exit, weakness, or reward. When a finding
is attached, use its exact finding ID and skill DC. Use distinct skills for separate pieces of
evidence and do not state the hidden fact outright.

<!-- Why: server actions need stable targets and explicit mechanical effects. -->
Give each thing a two-to-four-sentence detailed description, a one-line list label, a picture
description, a closer-look result, its check skill and DC, whether it can be picked up, exact
coin value only when supplied, and a one-sentence use effect. Give shops only established goods
at the supplied SRD prices. Do not decide check outcomes, inventory changes, purchases, thefts,
or story outcomes; the server resolves them. Generated offers are finite stock entries with a
concrete `item_id` that remains stable before and after purchase. A theft changes custody only
after an adjudicated rules result while preserving ownership, and should be described as a
witnessed or discoverable event only when the supplied scene facts support it.

<!-- Why: bounded, grounded structured output can safely be stored with the generated adventure. -->
Return only JSON matching the supplied schema. Every required field must be present. Keep all
strings within schema bounds, use unique stable IDs, and ensure owners, sellers, clue facts,
findings, and destinations refer to supplied entities. If a required detail has no support in
the supplied facts, use a plain neutral description and do not invent a consequential claim.
