# Scene combat binding contract

Status: implementation brief for the additive scene-NPC attack path.

## Binding

An authored binding is required for every scene NPC that may enter combat. The
fixed harbor pack binds only the canonical Courier entity (`0x67` repeated 16
times, the existing `courier` identity) to the `commoner` SRD profile. The
binding is selected by stable scene-person ID and the current persisted entity;
it is never selected by display name, nearest actor, or a provider response.

The binding stores the creature profile ID, identity revision, and a separate
`scene-npc-assault` objective. Shop and generated NPCs remain unavailable until
their scene content supplies an explicit binding.

## Resolution

An explicit final physical attack validates the current scene binding before
any mutation. Admission uses the existing participant and source checks, draws
normal initiative for both heroes and the bound NPC, and leaves the encounter's
active turn at the first initiative result. It does not resolve an attack as
part of admission, so an NPC that wins initiative takes its ordinary turn only
through the normal turn transition.

The assault objective is separate from `defend-courier`. Victory and defeat
therefore do not satisfy the courier defense source, award its story rewards,
or run the courier defense reaction. The bound entity remains the same
persisted Courier identity through encounter and aftermath; it is never renamed
to Bandit or replaced with a proxy.

## Source

The Commoner profile is from the official [SRD v5.2.1](https://www.dndbeyond.com/srd),
published under CC-BY-4.0 by Wizards of the Coast: AC 10, 4 HP (1d8), all
ability scores 10, and Club +2 to hit for 2 (1d4) bludgeoning damage. The
catalog records the attribution and source revision with the profile.
