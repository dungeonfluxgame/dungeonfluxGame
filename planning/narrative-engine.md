# Structural narrative engine

Date: 2026-09-30
Status: Outline adopted; authored campaign and policy calibration pending

`df-narrative` owns story relevance and believable opportunity selection.
[Runtime directors](runtime-directors.md) defines composition, authority, types,
bounds and recovery. State lives in the same authoritative `df-model::GameState`,
not a second narrative database; separation is by typed state/policy boundary.

## Authoring and state

A `CampaignStructure` defines arcs, flexible phases, `StoryBeat` requirements,
canonical fact references, threat stages and alternative delivery strategies.
A beat identifies required information/conditions, preconditions, compatible
character hooks, importance/flexibility, consequences and substitute deliveries.
Its dependency means prerequisite understanding, not completion of a fixed quest
scene. The same location fact can become known by testimony, notes, observation
or a legally resolved magical investigation. Knowledge grants record the actual
recipient; one player's private discovery is not automatically party knowledge.

`NarrativeState` persists current arcs/phases, active/dormant/completed/failed beats,
open world/mystery/character threads, player-declared interests and evidenced
inferred interests, recent choices/interventions, threat references, narrative
gravity, world momentum and intervention/coincidence budgets. `CharacterHook`
contains consented backstory/goals/relationships/values/fears/secrets and audience
scope; personal hooks never disclose a player's secret to the group by default.
Content/world own truth and threat progression; narrative refers to those records
and proposes consequences without becoming another clock writer.

`ContentPack::validate` rejects duplicate/missing IDs, cyclic beat prerequisites,
invalid phase predicates, unreachable authored alternatives, absent fact/NPC/source
references, contradictory unique truths and invalid cue/asset references. Validate
substitute-delivery equivalence and at least one feasible path under stated
conditions; failure/refusal can select an alternative rather than fabricate access.
Loops in repeatable world events need explicit iteration/TTL bounds instead of
being disguised as acyclic beat dependencies. Validate/migrate against pinned
schema/content/policy versions; editing a pack cannot silently retcon a live run.

## Director pass

`NarrativeDirector::evaluate` observes committed/rules-resolved events and permitted
candidate opportunities. It updates threads/interests, evaluates beat/phase
conditions, searches bounded convergence candidates, then returns a proposal or
`NoIntervention`. Convergence may join a player goal, open thread, story beat and
existing world event only when location, chronology, NPC knowledge/motivation and
canonical truth permit it. `df-interaction` can reject an implausible NPC delivery.
The engine then chooses another opportunity; it never puppets an ignorant NPC.

Score components include story/player/character relevance, plausibility, novelty
and urgency, with repetition, coercion and coincidence penalties. Persist chosen
candidate/components, causal basis, relevant alternatives and policy revision so
an operator can explain the result. Example decimal scores in the supplied
outline are heuristics; calibration against actual sessions is G12 work.

Gravity increases the salience of unresolved threats through credible clues,
character connections and consequences. It cannot teleport players, secretly
rewrite their intent, guarantee required beats, erase a valid choice or punish
out-of-world disconnects. Intervention budget replenishes during approved free
play and caps strong coincidences/events. Players may ignore, oppose or abandon
an arc; the world can progress to an authored alternate/failure state. Pause,
threat stages and game-time advance follow `df-world`, not chat wall time.

Fail-forward means a failure may open a source-valid costly alternative or change
circumstances. It never grants forbidden information, converts a failed die into
success, repeats a roll without permission or consumes an unapproved resource.
A failed persuasion can leave a plausible trail only if the NPC's valid action
and world circumstances create one. Arc transitions use explicit predicates (for
example a threshold of disclosed prerequisites), not a hard-coded linear demo.

## Realization and acceptance

A selected `NarrativeMoment` identifies committed event/fact references, permitted
participants/location, semantic mood/focus and presentation importance. It goes to
`df-presentation`, tempo and bounded AI/media effects. AI expresses the approved
moment; invented claims remain candidate text and cannot create canonical facts,
completed beats, dice results or hidden knowledge. Validate text before captions
or speech; apply the separately gated stable-clause contract only when proven.

Acceptance includes alternative deliveries, private versus party knowledge,
blocked NPC disclosure, ignored-arc consequences, pause/travel threat behavior,
excessive coincidence rejection, personal-hook privacy, fail-forward rules
integrity, source edits/replay, stale completions and bounded candidate/LLM work.
Frontier playtests check agency and comprehensibility; measured scores cannot
prove a story is fun. F39/G10/G11/G12 own remaining tasks and evidence.

## Places and exits (SCENE-01, 2026-10-17)

A place lists its exits (`SceneWayOut`: id, name, position, what lies beyond, open). A freeform act
Jev reads as a successful move is put to Jev again as a closed choice among the open exits and a
last option for staying (`place_creation::judge_exit`); the choice is an exit position and nothing
else. The owner then repeats every check (`place::plan`): the party stands where the move leaves
from, the exit exists and is open, and the destination is exactly where the exit leads. An exit
leads to a place id, authored (`beyond_place_id`) or bound by the first move through it. The beat's
own place is returned to; a place the run created is returned to; a place the story owns is not
walked to (its beats move the party); anything else is created once.

The storyteller creates the place from the exit, the adventure's setting, the canon the town's
people have spoken, the recent public table lines and the story memory (`place_creation::create`,
purpose `place_creation`, three JSON slots under the schema in the facts, one retry that names the
rule broken). The server parses the slots into a compact `PlaceRecord`, bounds every field, and
expands it deterministically into a scene model, a cast and a way back to where the party came
from. The checkpoint codec keeps every stored text to 4,096 bytes, so the record is stored as the
compact answer, not the expanded model.

A move is a journal entry with its own policy (`place-move-1`) in the session checkpoint, so the
created place is durable world state in PostgreSQL and a replay, a reconnect and a return read the
same record. `place::overlay` is the one place the rest of the game learns where the party stands:
the beat's scene with the created place's title, narration, cast and scene model in its stead,
until the beat changes. The party moves together because the scene is shared. The display view,
the room photograph, the scene plates (the living-scenes pipeline, found through the published
scene) and the place music follow the new `location_id`. The table is told a move only from the
committed result; a refused or failed move changes nothing and claims nothing. Audit lines:
`PLACE judged`, `PLACE change from=<id> to=<id> exit=<id> created=<bool>`, `PLACE refused ...`.
`dev place`, `dev place list` and `dev place go <exit|place>` reach the same path.

Not yet: a person who follows the party out (escort), a dossier for the people a place creates (they
stand in the scene and on the TV but have no dossier to talk from), and carrying a hero's
purchases from an earlier place into the phone inventory of a later one (items are still scoped
to the place whose scene model holds them).
