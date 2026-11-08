# Frames refinement log: the picture follows the talk

What was built, run and changed for frames during `persona converse`, newest last. How to run it
is in `development/runbook-dev-playground.md` ("Frames that follow the talk", "The look").

## Design (2026-11-07)

Owner ask: a new picture about every 30 seconds that follows the conversation (expressions,
items, positions), made with its latency in mind. Analysis: a plate takes 60 to 110 s (image 50
to 100 s, scan and naming 10 to 15 s), so a fixed 30 s cadence cannot be met and freshness can
never beat the latency. Owner decision: a judge reads the context and gates the update; above 25%
visible change a frame is made.
Built:
- `POST /dev/persona/frame` and `persona frame`: one model call, closed schema (changed percent,
  changes by character id and kind, the next picture, the shot), every id, kind and number
  validated in code.
- `persona converse frames=NAME`: judged while each line is read; above the gate a frame starts
  in the background, one at a time, with `gate=`, `scene=`, `edit=`, `edit_from=`, `hold=`,
  `max_frames=`, `judge_model=` and `with=` on the command line.
- Image edits: `media generate base=<picture>` and the one-shot door's base picture (the Codex
  backend's Edit with the base as the first image), so a frame keeps faces, clothes and the room.
- `persona/frames.json`: gate, kinds, scale, edit rule and camera shots, re-read every call.

## Runs

- Run 1 (`bar-live1`): 12 lines of talk only (Lena put the dance off). Judged 0 to 15%; no
  frame, correctly. Flaw: the score fell back to 0 on quiet lines instead of accumulating.
- Run 2 (`bar-live2`): dancing from line 1, but the score sat at exactly 25 ("one person moved")
  and fell back, so the only frame came after line 13. Its edit kept every face, costume and the
  room and showed the dance (100.5 s). The transcript said "shown 0.0 s": the wait was measured
  before awaiting the task.
  Changes: the change since a frame is the highest score since it (it only grows); the timing is
  taken after the frame lands; scale steps 30 (one person moved or took something) and 40.
- Style, twice: the first bar plate was an epic fantasy-film tavern; "not a fantasy film" then
  gave a modern bar (drum kit, glass mugs). The look now names the world and period (a glossy TV
  period soap opera in a medieval D&D world, intimate medium shot, candlelight) and bans modern
  things and high-fantasy art separately. New base plate `bar-period.png`.
- Run 4 (`bar-period-live1`): 30% after line 3 (Kael rises to dance, Lena puts the tray down),
  edit started, shown 70.7 s later at line 10 with the same faces, costumes and look.
- Bubbles off screen (owner report): a head at the top of the frame left no room above it, and
  covering a face weighed more than being off the stage. Off-stage area now weighs ten times a
  covered face, so the bubble goes below the head. The band and dancers talked because converse
  uses everyone with a persona: `with=` chooses who talks.
- Shots (owner ask): the judge picks a shot from `frames.json` (two-shot, close-up, extreme
  close-up, over-the-shoulder, telephoto, wide, Dutch angle, low, high) and is told the one on
  screen to vary it; the edit rule lets the camera move. Run 5 (`bar-period-shots1`, Kael and
  Lena only): frame 1 two-shot (little change), frame 2 over-the-shoulder onto Lena seated, chin
  on hand, tray set aside, 64 s.

- Run 6 (`bar-period-drama1`): the topic listed events, but the characters opened after them
  ("that's one way to get us both a seat by the hearth") and circled "the road can wait"; one
  frame (35%, sitting together). A topic is what is on the table for every line, not a script.
- Events: `persona converse events=<file>` (`<turn> | <what happens>`): the speakers of that turn
  and the next are told it is happening now, and the judge gets `(Happening now: ...)` as a seen
  line. Run 7 (`bar-period-drama2`, `drama.events`): frame 1, a Dutch angle the judge chose for
  the spill (Lena soaked, tankards tipped), 118 s; frame 2 two-shot, laughing, foreheads close,
  Lena wringing ale from her braid, the drunk walking off behind, 118 s. Owner: "dutch angle is
  nice". Misses: with one frame at a time and 2 minutes a frame, the sword-hilt standoff, Lena
  stepping between them and the drunk backing off (turns 2 to 6) got no frame of their own; the
  dialogue kept "keep clear, I've got him" because each line ended "react to the last thing said".
  Changes: a line reacts first to what is happening now; `parallel=N` (1 to 4) makes up to N
  frames at once, shown strictly in order (each waits for the one before it to be shown) with one
  spinner for all of them. Not yet run live.
- Whiplash close-up (owner: "I wanted to see Lena's face when she heard that"): the beats runner
  makes no frames, so the moment was judged from the run's real lines (35%, "her smile is gone;
  she looks hurt") and edited by hand as a close-up (`lena-betrayed.png`, 55 s): her eyes glassy,
  lips parted, Kael laughing in a blur at the edge. The judge had picked a two-shot; the close-up
  look now says "the face of whoever was just hit hardest".

## Open

- Latency: Codex edits take 60 to 100 s, so a frame lands 6 to 10 lines after its moment; a
  faster edit model is a provider decision.
- The first frame has no previous shot, so it tends to repeat the base framing.
- The topic's events can be skipped: characters opened as if the shove had already happened.
- The judge still scores some single-line changes low; tune the scale and gate in `frames.json`.
- `parallel=` frames are unit-checked only; a live run is owed.
- The beats runner (`market-thread.sh`) has no frames; a scripted keeper scene cannot show faces
  as it plays.
