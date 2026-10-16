# Tempo driven scene music

Task: make the shared display's music follow the authoritative server tempo cue and
select an ambience that matches the current public scene.

Governing documents: `planning/tempo-engine.md` (server tempo owns audiovisual
intensity, sample aligned layers, speech ducking and stale cue handling),
`planning/client-architecture.md` (the browser only renders server provided plans),
`planning/observability.md` (record audio buffering, delivery and playback timing),
`planning/runtime-directors.md` (tempo is cosmetic and audience safe), and
`development/visual-resource-manual.md` (agent inspection and evidence commands).

Owner: `df-tools`, especially `music_score.rs`, `gameplay/dev_control/reads.rs`,
`gameplay/dev_control/timeline.rs`, and the browser music state exposed by
`gameplay_browser/live_music.rs`.

Acceptance: a display view exposes the authoritative tempo phase/intensity and the
derived public music plan; scene/location identifiers resolve to the matching
ambience family; dialogue uses the main score, combat uses the battle suite, and
outcomes use release/finale handling; stale cues do not restart one-shots; the CLI
and manual show the plan and diagnostic limits; native tests and the browser target
compile pass. Live audio playback remains a coordinator-owned check.
