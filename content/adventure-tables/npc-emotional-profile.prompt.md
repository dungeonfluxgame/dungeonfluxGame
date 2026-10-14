Use only the supplied complete dossier when writing this character's emotional profile.
<!-- Why: the emotional profile must be part of this character, not an unrelated add-on. -->
Return only the requested structured fields, with short sentences and no extra narrative.
<!-- Why: bounded output fits the stored adventure and the runtime prompt. -->
Follow `facts.output_schema` exactly, including every required field and its allowed values.
<!-- Why: the server validates this profile against the same schema before storing it. -->
Every attraction, fear, delight, aversion, and grudge must cite one existing dossier field in `because`.
<!-- Why: the server can reject unsupported or uncited emotional claims. -->
Do not contradict, expand beyond, or claim a cause absent from the cited dossier text.
<!-- Why: the profile must remain consistent with this character's generated history. -->
Give this adult character an orientation, choosing a realistic spread across the cast; individual attractions still need dossier evidence.
<!-- Why: orientation is a character trait, while each particular attraction remains grounded. -->
Describe specific attraction only toward adult heroes and only through visible traits supported by a cited dossier entry.
<!-- Why: the profile can express varied orientations without inventing a reason to desire a particular hero. -->
Choose forgiving or unforgiving from the character's bonds, flaws, formative event, and motive.
<!-- Why: permanent resentment needs a source-grounded character reason. -->
Choose low, middling, or high patience from the same dossier, and write only brief observable tells.
<!-- Why: pacing and Insight need a small profile the table can actually use. -->
Preserve the five supplied legacy reactions as the profile's legacy reactions without replacing or contradicting them.
<!-- Why: old temperament behavior remains available while the profile becomes the single rendered description. -->
