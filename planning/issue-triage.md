# Issue triage list

Every bug and issue found by playing the game on 2026-10-12, gathered into one list for review. It merges the five play-session sections at the end of `planning/implementation-roadmap.md`, the game defects from that file's "Open items after the 2026-10-12 play session" list, the dialogue interview, and things the owner said while watching that were never written down. Duplicates across sessions are merged into one row. Build played: commit `c400f95` plus the uncommitted work of the stopped workers.

Not in this list: business and legal decisions, unmerged branches, and built-but-unverified work. Those stay in the roadmap ("Business and service", "Unmerged work on branches", "Not verified").

## How to read it

**Severity**

| Sev | Meaning |
|---|---|
| 1 | Stops or loses a game |
| 2 | Play is wrong, misleading or badly broken, or the owner called it out as major |
| 3 | A visible defect that does not stop play |
| 4 | Polish |

**Seen in**

| Code | Session |
|---|---|
| P1 | Playthrough 1, room BADENE, party defeated |
| P2 | Playthrough 2, room FUREGA, victory |
| P3 | Playthrough 3, room TAMUDA, owner watching |
| SR | Recorded speed runs, rooms BOSAKA and ZODAPO |
| SC | Other-scan sessions, rooms KENOKA, SUFARO, ZATOZO |
| IV | Dialogue interview with the patron (30 lines, `artifacts/tmp/dialogue-a1/transcript-round2.md`) |
| OWNER | Said by the owner during the session |
| P4 | Hand-played session, room KESEZA, 2026-10-13, victory (section 19) |
| EARLIER | From the open-items list at commit `de32f94`; not re-checked this evening |

The last column is empty for the reviewer: fix now, fix later, will not fix, or duplicate.

## Counts

| Area | Sev 1 | Sev 2 | Sev 3 | Sev 4 | Total |
|---|---|---|---|---|---|
| Session and server (SES) | 3 | 3 | 3 | 2 | 11 |
| Story flow and pacing (FLO) | 0 | 3 | 4 | 1 | 8 |
| Story writing (STO) | 0 | 3 | 7 | 0 | 10 |
| Conversation (DLG) | 0 | 2 | 5 | 0 | 7 |
| Fight: turns and rules (FGT) | 0 | 2 | 5 | 1 | 8 |
| Fight timeline (SYN) | 0 | 5 | 7 | 0 | 12 |
| Fight presentation (FPX) | 0 | 3 | 7 | 0 | 10 |
| Battlefield scans (SCN) | 0 | 1 | 2 | 0 | 3 |
| Sound (AUD) | 0 | 2 | 2 | 0 | 4 |
| Pictures (PIC) | 0 | 2 | 5 | 1 | 8 |
| TV layout (TVX) | 0 | 0 | 5 | 2 | 7 |
| Phone (PHN) | 0 | 2 | 5 | 1 | 8 |
| Lobby and character creation (CRE) | 0 | 0 | 6 | 1 | 7 |
| Endings (END) | 0 | 2 | 1 | 1 | 4 |
| Tools and tests (DEV) | 0 | 0 | 4 | 2 | 6 |
| **All** | **3** | **30** | **68** | **12** | **113** |

This table counts sections 1 to 15 only. Sections 16 to 18 add 36 rows found by reading the code and by reviewing the fixes. Section 19, the game played by hand on 2026-10-13, adds 57 more (8 of severity 2, 44 of severity 3, 5 of severity 4); most repeat a row above with new detail, and four of them reopen a row that had been marked fixed.

## Suggested first ten

In the order I would take them. One at a time, as agreed.

1. **SES-01** — fast or repeated presses kill a room for good.
2. **SYN-01 to SYN-07** — the fight's timeline: skipped enemy turns, the TV and phones disagreeing, the dead standing up, the fanfare early. One piece of work (the one-clock design is accepted and partly built).
3. **SES-05** — start-up story text misses its deadline in more than half of starts, which decides whether the whole session has a story.
4. **FPX-01** — no dice roll is ever seen, although the clips exist.
5. **FGT-01** — a fighter sometimes cannot attack after moving.
6. **SES-04** — a lost phone ends the game for the table.
7. **DLG-01** — nothing a player says or does moves a character.
8. **FLO-01 and FLO-02** — the story can be skipped in under two seconds and is only five screens long.
9. **SES-02** — the Hockley Valley scan cannot start a fight against bandits.
10. **PIC-01** — the heroes in the pictures are not the heroes the players made.

## 1. Session and server

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| SES-01 | 1 | Pressing choices quickly broke a room for good. Two choices under a second apart, the second pressed three more times at 0.9 s. The server then answered everything with `UNAVAILABLE site=authority reason=InvalidCandidate` (73 times in two minutes), TV and phones showed "The connection to the game was lost", and the room never recovered. Not yet known whether the trigger is the repeated press or choosing before the previous step settled. | SR (BOSAKA) | Fixed `5abd72e` (the refused fight) and `1817d9e` (a refusal no longer fences the session owner). Unit and integration tests pass; three recorded games ran without a refusal. The server-side refusal path was not provoked from a browser |
| SES-02 | 1 | The Hockley Valley scan (`scan-valley-bridges`) cannot start a fight against bandits: every press logged `REFUSAL action=DefendCourier reason=InvalidCandidate`. The same seed (0xb2563e1a9bd2a1a0) on the village scan started the fight at once. Cause unknown: every error on the fight-start path is logged as the same `InvalidCandidate`. | SC (KENOKA, SUFARO) | Fixed `5abd72e`: the cause was the order heroes were confirmed in, not the scan (HNT-01). Verified on a running server |
| SES-03 | 2 | When a fight cannot start the players are told nothing; the choice stays on screen and can be pressed for ever. | SC | Fixed `b2f7239f`: the phone that pressed shows a short refusal notice and the choice is usable again (browser check pending) |
| SES-04 | 1 | When both phones went away mid-fight the game stayed on "Brakka's turn" with no time limit seen. A new browser opening the room was offered only a fresh join, with no way to take the abandoned hero back. | SC (ZATOZO) | Fixed `b42aa199`. A seat silent for 30 s is away; a browser with the room code is shown "<hero> (away)" and "Take this hero back", gets a new credential for the same hero and lands in its current view; the old credential is revoked. Checked on a running server and phone: both seats logged away after 30 s, `dev reclaim 1` accepted and that hero attacked; a phone opening the room saw "Sigrun (away)", took her back and was in her fight view; the previous holder was refused. The real-database test passes against PostgreSQL. Not checked: two browsers racing for one seat. There is still no time limit on a turn nobody takes |
| SES-05 | 2 | Start-up story text missed its 25 s deadline in 3 of 5 starts this evening and in playthrough 2. The session then runs on filler text for every line. Nothing says so on the TV, the phones or the lobby, nothing retries, and the lobby opens regardless. | P2, SC | Part `ffed4a4`, `14fc1fee`: every group arrived first time in three live starts, and a late group no longer discards the ones that arrived |
| SES-06 | 3 | When the text does arrive, some of it is refused by validation and replaced by table text: 17 of 54 slots in playthrough 1, 19 of 55 in playthrough 3, 11 of 55 and 3 of 54 in later starts. | P1, P3, SR, SC | |
| SES-07 | 3 | A choice had to be pressed twice: the first press showed "Waiting for the game server…" and the same choice was offered again. | SR (ZODAPO) | Fixed `56b69c7` (browser check pending) |
| SES-08 | 2 | A failed start-up leaves `df_game` and `df_local_demo` behind, so the next start refuses with `retained_access_unavailable`. | EARLIER | Fixed `7d0f493`: a failed start-up releases its session |
| SES-09 | 4 | "SESSION lease renewal failing (Unavailable); retrying" logged once in a session's first minute; it recovered. | SC | |
| SES-10 | 4 | With the same seed the story text differs between starts (two different titles for one seed), so a seed does not reproduce a session's wording. | SC | |
| SES-11 | 3 | The battle scene drives the machine hard enough to throttle the processor ("its downclocking the cpu"). Sessions run the scan at level of detail 1 only because the session launcher sets it; the server default is still 0. | OWNER | Fixed `97f9d1e`: default level of detail is 1 |

## 2. Story flow and pacing

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| FLO-01 | 2 | The whole story before the fight can be skipped in 1.6 s: three presses 0.3 s apart with no line shown or spoken. Nothing holds a choice until its scene has been presented. | SR | Part `9beca956`: each new story scene's choices are withheld for four seconds (seen on a running server, logged as hold and release). Releasing as soon as the TV reports the scene told, and refusing an early press with a reason, need four additive wire fields and are not built (`artifacts/tmp/campaign/luna6/H05/report.md`) |
| FLO-02 | 2 | The whole game is five screens and about ten minutes: opening, one follow-up, the fight, the aftermath, the epilogue. Three of four choices have a single option. No journey, no investigation, no use for the private clue. The lobby promises places that do not exist. | P1, P2 | |
| FLO-03 | 2 | Owner direction, not built: the story should branch off what the players learn. New information opens leads the player can review on the phone; leads read as hints, not quest markers; players may follow them or go their own way. A plan exists (`artifacts/tmp/leads-a1/plan.md`, four milestones). | OWNER | |
| FLO-04 | 3 | Talking changes nothing that is offered: the choices are the same after a conversation, and a rumour that names the antagonist opens nothing to follow. | P1, P2, IV | |
| FLO-05 | 3 | The only choice before the fight names an enemy nobody has seen ("Face the giant weasel") or reacts to no visible threat ("Stand your ground"). | P1, P2 | |
| FLO-06 | 3 | Choices after the fight are chapter titles, not actions ("The Road Takes Its Toll"), and the phone mangles them into sentences: "Brakka thes Road Takes Its Toll", "Brakka calla'ses Thanks". | P1, P3 | Fixed `67b9586`, `8a9fe3cb`: the log no longer mangles titles, and choices keep the generator's action wording. Needs a paid session to see |
| FLO-07 | 3 | Rules steps are offered as story choices: "Advance 6 seconds · death saves", "Wait · a stable hero regains 1 hit point after 1d4 hours", and "Short Rest · 1 hour, no Hit Point Dice" offered first, twice, to heroes at full hit points. | P1, P2 | Part `cde1bce1`: the death-save and stable-hero choices are worded as what the heroes do. The short rest's label comes from the content pack and is unchanged |
| FLO-08 | 4 | The title card's sentence is repeated word for word in the Dungeon Master box straight after it, and one title card stayed up for at least 18 s while a long line was read. | P1, P2 | Fixed `7218c949` for the repeated sentence (browser check pending) |

## 3. Story writing and consistency

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| STO-01 | 2 | The plot does not hold together when the filler text is used: the job is to collect a debt from the man who holds the ledger, who then "takes the ledger"; a named villain never appears; a secret is stated and never explained. | P2 | |
| STO-02 | 2 | The opening gives no job, stake or reward. | P2 | |
| STO-03 | 3 | The fight's introduction names one villain with one weapon and the fight fields three generic enemies with another; the loading card says one enemy springs out and there are three; the victory text blames a villain nobody met. | P1, P2, P3 | Part `b4c7cbdf`: see RUN-08 |
| STO-04 | 3 | Narration contradicts the picture and the events: "that was close" after a fight where nobody was touched; a character "steadies herself behind you" over a picture of her calmly at work; the party is "at the apothecary" and is then offered "Continue to the apothecary". | P2 | |
| STO-05 | 3 | The defeat ending contradicts itself: the patron "is dragged away", the epilogue never mentions her, the end text has her starting again. | P1 | |
| STO-06 | 3 | One leftover sentence is glued onto the fight's introduction, the victory text, the journey's-end narration, the epilogue and the end text. | P2 | Fixed `ea9430a9` for sentences repeated from an earlier scene in another place |
| STO-07 | 3 | Epilogue lines are shared across adventures word for word, spoken by different characters in consecutive games. | P3 | |
| STO-08 | 3 | The private clue runs narration straight into first-person speech with no quotation, and it is repeated in the phone's story log on every later screen. | P1 | Part `d4246c4`: the phone shows the clue once; the missing quotation marks are not done |
| STO-09 | 3 | A character brings up a plot element unprompted and then cannot say who or what it is; characters do not know the plot. | EARLIER, P1 | Part `847473be`: replies are given the scene's public state |
| STO-10 | 2 | A private-clue reply can leak onto the TV, because privacy is decided from a term list and not from what the table has been told. | EARLIER | Fixed `009bb09f`: a reply is private whenever the hero's private clue was given to the model for that turn, whatever its wording |

## 4. Conversation with characters

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| DLG-01 | 2 | Nothing a player says or does moves a character. After flirting, propositions, a drawn sword, threats and an assault the patron stayed friendly; no roll was asked for; the scene did not react. Moods, attitude changes and the Influence roll are not wired. | IV, P1, P2 | Part `397e6e3d`, `44792e86`, `7edf7afd`: every spoken line now gets an approach from a model (flirting, bribe, insult, kindness and the rest were each labelled correctly on a running server) and the server's fixed rules move the character's attitude from it. What a flirt, a bribe or an insult then changes beyond attitude (an offer, a roll, a refusal to deal) is not built |
| DLG-02 | 2 | Replies still fall back to stock text. In playthrough 2 half the replies were rejected by validation (`reply_rejected`), and the fallback for an accusation was the patron's opening line word for word. Provider failures (`call_failed`) have no retry. | P2 | Open. A second attempt (retry once on a provider failure, never repeat the scripted opening line, hostile fallbacks not repeated) is written but two of its own tests fail; held at `artifacts/tmp/campaign/luna6/F68/held.patch` |
| DLG-03 | 3 | Characters cannot answer ordinary personal questions (children, home, how they know the others). Replies pad with unrelated stock lines and repeat a term of address in every reply. | IV, P1 | |
| DLG-04 | 3 | The patron cannot say what her own errand's object is or why its deadline matters. | P1 | |
| DLG-05 | 3 | Asked what the job pays, the patron gave three unrelated sentences; the next scripted line then promised repayment. Scripted lines and conversation do not share what the character knows. | P2 | |
| DLG-06 | 3 | A reply answers only one of several questions asked together. | EARLIER | |
| DLG-07 | 3 | The same stock lines come back across answers ("it has to arrive before dawn", "I took it on trust"). | EARLIER, IV | |

## 5. Fight: turns and rules

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| FGT-01 | 2 | A fighter sometimes cannot attack after moving next to an enemy. Intermittent: it worked on the first turn in two later runs; in room ZODAPO the server's own record shows him moving next to an enemy twice and making no attack. In playthrough 3 he never attacked on his own turn. | P3, SR | Open. A reviewer (K13) could not find a path in the current code that removes the attack after a completed move; it needs a recorded reproduction |
| FGT-02 | 2 | Only the d20 is rolled and shown. Damage appears as a finished number and the server's combat record holds no damage dice ("he rolls hit but doesnt roll damage"). | P3, OWNER | Part `a30f6295`: the damage dice are kept by the fight engine, sent in the outcome and logged (`damage_faces=[6, 5]`; with Savage Attacker `[1, 1, 5, 6]`). The TV attack card already prints the faces when it is given them (not seen: the TV page cannot be photographed here). No tumbling damage dice, and the damage modifier has no wire field |
| FGT-03 | 3 | An enemy can die to a miss: the greatsword's graze deals damage on a failed roll, the TV says "ENEMY VANQUISHED", and nothing explains why. The table log calls it a "miss". | P3, SR, EARLIER | |
| FGT-04 | 3 | Nothing shows that a hero's reaction is spent, so an enemy walking away without an opportunity attack looks like a mistake by the game ("no opportunity attack when they walked away"). The rule itself was applied correctly. | P3, OWNER | |
| FGT-05 | 3 | Attack lists offer weapons the heroes' builds do not state: Spear and Shortbow for the greatsword fighter, Scimitar and Shortsword for the archer. | P1 | Not a rules fault: the Soldier background really starts with a spear and a shortbow. What is left is presentation (main weapon first) |
| FGT-06 | 3 | The greatsword fighter is offered "Javelin (throw)" at 5 ft, which is rolled with disadvantage, with nothing marking it as the worse choice. | P2 | |
| FGT-07 | 3 | Balance swings widely for a party of two: dead in three rounds in one game, victory in three actions in the next, seven enemy misses in a row in a third. Enemy count, hit points and turn order need tuning. | P1, P2, SR | |
| FGT-08 | 4 | Two rulings are applied without being disclosed: initiative ties between a hero and a monster go to the heroes, and a dead creature's square does not block movement. | EARLIER | |

## 6. Fight timeline: TV, phones and server out of step

The owner's words: "the battle scene was out of sync", "make sure the state machine makes sense". A one-clock design was accepted (`artifacts/tmp/fight-a4/M1-READY.md`) and partly built; the worker was stopped before verifying it. Two faults are known in the unfinished work: the pacing timer never starts, and the TV stops reporting what it has shown once it is idle.

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| SYN-01 | 2 | The TV skips enemy turns. In playthrough 1 six beats were resolved before the first hero turn; the TV showed the first, waited about 25 s and jumped to the sixth, so the bites that hurt the heroes were never seen. In playthrough 2 a whole enemy turn was marked skipped ("the 3rd creature didnt even get a chance to play"). In playthrough 3 two of 26 beats were skipped. | P1, P2, P3, OWNER | Part `8d5425b`, `283a76d6`, `37f678d5`, `45ea9a22`: recorded again on a running TV after `45ea9a22`: all three blows of an enemy phase are shown in order, none skipped. Still wrong: the phase takes 49 seconds for three bites (an impact lasts 7 to 18 seconds), and after the last blow the enemies' headings are cycled twice more for twenty seconds (`artifacts/tmp/campaign/luna6/S05-evidence.txt`) |
| SYN-02 | 2 | A defeated enemy acts again on screen: "it said enemy vanquished then he stood back up", "the dead guy moved". Consistent with the TV applying a death and then playing an older queued beat for the same figure. | OWNER (twice) | Fixed `8d5425b` (browser check pending) |
| SYN-03 | 2 | The phone offers a hero his turn while the TV is still several beats behind (beat 6 of 6 on the phone, 2 of 6 on the TV); the TV's own counter went 6, 0, 2 during the entrance. | P1 | Part `b4d7f432`, `c05f53c7`, `37f678d5`, `8645c254`: the TV's health cards now follow the presented beats exactly (RUN-02). The phone still offers a hero his turn on the server's clock |
| SYN-04 | 2 | The TV's health cards are wrong: hurt before any bite was shown, then full for 26 s while both phones said hurt, then hurt again. A heal on the phone (4 to 10) did not reach the TV for two rounds. | P1, P3 | Fixed `0bf7237`, `deb37c2e`: a heal reaches the TV at once (sampled on a running game). REV-03 still stands |
| SYN-05 | 2 | The phone keeps showing "YOUR TURN" with Attack, Move and End turn enabled for up to ten seconds after the turn is spent. Presses are refused ("That offer has ended") or silently dropped. | P1, SR, SC | Fixed `0761a5b`: the waiting state was seen in a recording; see REV-01 for a fault in the fix |
| SYN-06 | 3 | The victory fanfare, the scene transition and the ending music start before the last blow is shown (3.4 s early in one fight; before the last arrow was loosed in another). | P1, P2 | Part `4570a60` (finale music), `283a76d6` (the victory screen no longer mounts before the finishing blow; director tests pass). Browser check pending |
| SYN-07 | 3 | The turn label flickers between names within seconds and can run one to two turns behind the server. | P1, EARLIER | Part: see RUN-03 |
| SYN-08 | 3 | Heroes wait 11 to 15 s at a time for the TV to finish enemy turns; in the fastest possible game the fight was 2 min 56 s of 3 min 18 s. | SR | |
| SYN-09 | 3 | The turn strip looks out of order ("the character icon order seems broken"): the highlight jumps because skipped enemy turns are never shown, dead combatants stay as unlabelled dim circles, and the phone lists the party first instead of turn order. | P1, OWNER | Part `9f4f1ff`: downed chips are named and marked; the highlight jumping is the timeline work |
| SYN-10 | 3 | The TV asked "Brakka: Opportunity Attack?" with Brakka off screen. | P3 | |
| SYN-11 | 3 | Enemies that only moved are shown walking after the attacks instead of in initiative order. | EARLIER | |
| SYN-12 | 3 | In one four-enemy victory the celebration ran 107 s with two fanfares. Not investigated. | EARLIER | |

## 7. Fight presentation: dice, camera, banners

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| FPX-01 | 2 | No dice roll is ever seen. Rendered tumbling clips exist (`assets/ui/generated/dice/`), but the game shows a still image with the number for 0.4 to 1.8 s, with a gold square outline round it. Death saves were rolled five times in one fight and never shown at all. Before this the owner's complaint was the opposite: "the old dice roll stay on screen way too long". | P1, P2, P3, SR, OWNER | Part `0d8934f`: the tumbling clip now plays (seen in a recording); the die is only about 40 px across and death saves are still not shown as rolls |
| FPX-02 | 2 | The camera can sit behind scenery: for a finishing blow it stayed behind the stockade fence, which filled the screen. Owner: "the camera needs an xray feature … the player characters need to be cut out so they can be seen". | P1, OWNER | Fixed `7a3d505` (browser check pending) |
| FPX-03 | 2 | In the default view a hero is 30 to 40 px tall on an 800 px wide screen and most of the frame is empty ground. The actor-centred camera the owner asked for is designed but not built. | P1, P2, SC, EARLIER | Part `493477ca`, `fc1fc3f9`, `3b0e4114`: the wide shot comes about a quarter closer (reachable ground may run under the side cards, figures may not), the tilt-shift keeps the figures' band sharp, and every standing figure has a side-coloured ring. Seen on a running TV page. Figures are still small when a dozen are on the field |
| FPX-04 | 3 | Bad frames: one near-black blurred frame mid-fight, heavy smear with colour fringing during strikes, and a whole-battlefield motion blur during a camera move that reduces the figures to streaks. | SR, SC | |
| FPX-05 | 3 | The Dungeon Master says nothing between beats or while the game waits on a player; waits of 20 s and more are silent. Owner: "if the match idles the DM should make some more commentary, he tries to keep intensity up". Battle cries and fight narration were decided earlier and never built. | P3, OWNER, EARLIER | |
| FPX-06 | 3 | A thrown javelin is played as a bow shot, with bow sounds ("that was 2 bow strikes in a row"). | P2, OWNER | Fixed `1db1e80e`: a thrown melee weapon is no longer presented as a bow shot (not seen) |
| FPX-07 | 3 | "FINISHING BLOW / ENEMY VANQUISHED" takes the whole screen for the first of three enemies. | P1 | Fixed `4fc941ef` (browser check pending) |
| FPX-08 | 3 | The initiative roll-call draws one enemy's tag rotated with its number upside down, and the banner overlaps the tags. | P2 | |
| FPX-09 | 3 | Unlabelled round icons float above figures and under health cards; a downed hero is drawn as a dark spiky heap. | P1 | Part `af39b8bb`: a body keeps one mark and it no longer covers other figures or the cards; the marks are still unlabelled |
| FPX-10 | 3 | The fight is titled after the story place whatever battlefield is loaded: "THE SALT QUAY" over an autumn riverbed. | SC, EARLIER | |

## 8. Battlefield scans

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| SCN-01 | 2 | Figures walk through the scan's fence; the walkable grid is not derived from the scan. Owner: use "the voxel strategy from the supersplat or playcanvas repos" to block cells. Needs a research pass first. | P3, OWNER | Fixed `d29a3a4f` (owner's request): a Rust port of PlayCanvas's voxel collider derives each scan's blocked squares, blocked steps and ground heights; the rules refuse movement, reach and line of fire across a fence; the TV keeps figures on checked ground. Full server suite 1,313 passed; seen on a running TV page (section 21) |
| SCN-02 | 3 | Of the four scans only the village has been played to an ending. The riverbed works as a battlefield but was played only to round 2; the summit has not been tried; the valley cannot start a fight (SES-02). | SC | Part: the village, the valley and the riverbed were looked at on the TV with the owner on 2026-10-13 (section 21) and each starts a fight with everyone on free ground. None but the village has been played to an ending; the summit is not fit for a fight (SCA-03) |
| SCN-03 | 3 | The every-scene fight test stages only kobolds, so a scan that fails for another creature passes it. | SC | |

## 9. Sound

Nobody but the owner has heard the game this evening; the recordings are silent. Sound items come from the TV's own logs unless marked OWNER.

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| AUD-01 | 2 | Music stays ducked for as long as any text is on screen, spoken or not: 31 s during a wait for a player, and for the whole of the defeat screen and the end card. The owner's rule is that music ducks under every voiced line (player, character, narrator) and comes back after. Ducking under a voiced line works. | P1, OWNER | Fixed `e995a87` (not yet heard) |
| AUD-02 | 2 | On the defeat screen the narrator's line is hidden under the text and the closing paragraph is not read aloud ("I dont hear the DM narrating"). | P1, OWNER | Fixed `3b05752`: long narration is spoken in parts (not yet heard) |
| AUD-03 | 3 | A bite sounds like a sword: each one logs `sword-swing` then `blade-hit`. | P1 | Fixed `1db1e80e`: a natural weapon no longer plays the sword swing and blade hit (not heard) |
| AUD-04 | 3 | The TV's "Enable sound" control is tiny and in a corner; the game is silent until someone finds it. | P1 | Fixed `dc17a41`, `307c2b4`. Seen in a browser |

## 10. Pictures

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| PIC-01 | 2 | The heroes in the scene pictures are not the heroes the players made: tall red-haired humans seen from behind where the party is two dwarves with their own portraits. Other characters change appearance between pictures, and one described as a mapmaker is drawn as an apothecary. | P1, P2 | |
| PIC-02 | 2 | Owner request, partly written and never verified: direct any generated output from the command line, for example "rerender the plate but brakka unsheathes his sword", for every generated output and not only pictures. The worker was stopped; brief at `artifacts/tmp/devctl-a3/brief.md`. | OWNER | |
| PIC-03 | 3 | A picture refresh keeps everyone in the same pose ("on the display plate refreshes the characters dont move"): the refresh prompt asks for the same pose, framing and lighting with only the expression changed. | P3, OWNER | Fixed `7ecd7f9` (needs a paid picture to confirm) |
| PIC-04 | 3 | Heroes look unhurt in the pictures at 1 of 13 hit points. Owner: "the player characters should look beat up in the display plates when they are low hp". | P1, OWNER | |
| PIC-05 | 3 | The picture does not follow what happens: after a hero swept the herbs off the table and called the patron a liar, the new picture showed her smiling with the herbs still there. A character's face can change between answers and not match the line. | P2, EARLIER | |
| PIC-06 | 3 | Pictures of one place drift between shots (interior night, then outdoor day), and the phone's lobby picture does not match the TV's. | EARLIER | |
| PIC-07 | 3 | The contact in the epilogue is not in the scene picture until after the end card is up. | P1 | |
| PIC-08 | 4 | Cost: 20 pictures (half the session's budget of 40) and 18 voice clips for a ten-minute game; the opening picture is remade for each party size. | P1, EARLIER | Fixed `dce149c` |

## 11. TV layout

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| TVX-01 | 3 | A speech bubble flashes before a character talks ("before wystan talks a speech bubble flashes"): a small bubble appears beside the hero for half a second, an empty strip sits at the speaker's place for about 3 s, then the speaker's bubble grows in while the hero's fades out on top of it. A stray "..." bubble was also seen over a background figure. | P3, OWNER | Part `4c7e69b`: typed line leaves first; empty strip while generating not done |
| TVX-02 | 3 | The hero's own typed line is a faded bubble over the other character's figure, partly under the reply. | P1, EARLIER | Fixed `a1b3e25`. Seen by hand 2026-10-13: both heroes' typed lines sat by their own figures |
| TVX-03 | 3 | On every scene change the TV shows the adventure's title again instead of the new scene's, and during the crossfade it sits over the new picture and the health cards. | P1, EARLIER | Fixed `3720b36`. Seen by hand 2026-10-13: each scene card carried its own title |
| TVX-04 | 3 | The adventure title and place stay printed large over the picture for the whole scene. | P1 | Fixed `98ba2bf2`: the title greets the scene for about seven seconds, then is hidden. Sampled in a browser |
| TVX-05 | 3 | A rules log line is shown as a hero's speech bubble: "BRAKKA: has been stable for 1 hours and regains 1 hit point". | P1 | Part `6a013ebd`: the heal caption is brief and worded as an action. The stable-recovery line outside fights was not looked at |
| TVX-06 | 4 | The phone's label for a choice differs from the TV's ("Continue to the apothecary · Brannock's Apothecary" against "Continue to the apothecary"). | P2 | |
| TVX-07 | 4 | The speaker's bubble is drawn over the rules note and title; with choice chips off the narrator box stays empty while the table waits. | EARLIER | |

## 12. Phone

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| PHN-01 | 2 | The fight screen is developer drop-downs with dozens of cell and weapon options. The tap-to-move map the owner decided on has no brief yet. | EARLIER, all | Fixed `8ef03800`, `99f57003`, `f911eb05`: the phone draws a map of the fight above the controls; a lit square is tapped to choose where to move and an enemy is tapped to aim, then Move or Attack is pressed. Seen on a 375 px page in a running game: tapping G3 changed the aim and the server logged the attack on Giant Weasel 3. The weapon is still chosen from a list (one entry per weapon and target), and the map shows no walls or cover |
| PHN-02 | 2 | A downed player's phone says nothing about dying or death saves and offers nothing to do. | P1 | Fixed `0761a5b`: death-save progress shown (browser check pending) |
| PHN-03 | 3 | Developer notes are shown to players: "Disclosed custom rule tactical.cover-centre-line…", "Disclosed gap giant-weasel: Climb Speed 30 ft is not modelled", "Greatsword Dropped; ownership retained", "defeat does not restore hit points or held weapons". | P1 | Fixed `5b9700c`, `1cb72de3`, `a0ae78d6`, `70a9d608` for the fight note and the creation form (browser check pending); the full sheet overlay still shows raw values |
| PHN-04 | 3 | Saying your own line is awkward and mislabelled: the send button reads "Ask for clarification" under two drop-downs reading "Uncertain intention" and "No action selected", and the character's reply is shown with a different portrait from the scene's. | P1, IV | Part `a252a22`: labels fixed; reply portrait still differs |
| PHN-05 | 3 | "Confirm original strike" is offered from the aftermath to the end of the game and never explained. | P1 | Fixed `8fa68c8`, `0761a5b` (browser check pending) |
| PHN-06 | 3 | The fight's log line is carried into the story log of the next scene. | P2 | Fixed `57f0bfde`: a story scene's log starts at the action that opened the scene, with one line for how the fight ended |
| PHN-07 | 3 | The hero portrait is cropped to the top of a head, and an unstyled status line sits over the background picture. | P1 | Fixed `0ad7d67`, `5b9700c`. Resource chips seen in a recording; portrait crop not yet looked at |
| PHN-08 | 4 | The "Ask" button re-renders while a voice line plays, so a tap can miss. | EARLIER | Fixed `a252a22`, `134d711b`: the fight controls are updated in place (checked on a running page) |

## 13. Lobby and character creation

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| CRE-01 | 3 | Only Dwarf, Fighter and Soldier exist, so every party is the same. The form is about 7,100 px tall with twelve groups, four of which offer a single option under "Choose one offered option". | P1 | Part `e8a800f`, `1d664bd`: single options read as facts; only one species, class and background exist |
| CRE-02 | 3 | The field "A detail that makes you memorable" is in the page but collapsed to zero size; it cannot be seen or used. | P1 | Not a fault as built: the field is hidden on purpose, the server's character contract has no such field (see OBS-04). `edece58` changed nothing a player could see |
| CRE-03 | 3 | Developer wording is shown to players: "every submitted build is validated on the server", "Fighter B + Soldier B", "Other lawful SRD combinations remain unsupported", "Initiative tie agreement". | P1 | |
| CRE-04 | 3 | The phone's join page is titled "Gather at the Lantern Wharf" with a harbour picture whatever the adventure's place is. | P1 | Fixed `301391e`, `2cde9ae0`: neutral title and the place's own picture (browser check pending) |
| CRE-05 | 3 | The TV lobby shows neither the adventure's title nor what happens next or who starts; "THE PARTY IS GATHERED" is small grey text in a corner. | P2 | Fixed `6b5c691`. Seen in a browser |
| CRE-06 | 3 | The TV lobby prints "Or enter the code at 127.0.0.1:63319", an address no phone can reach, with no hint that the session is local only. | P1 | Fixed `6b5c691`, `a44e30b`, `49704baf`: no QR tile and no address for a local-only game. TV lobby seen in a browser; phone waiting page not yet seen |
| CRE-07 | 4 | The sticky "Confirm your hero" bar covers the form text beneath it. | P1 | Fixed `16819357` (the real cause: a gap under the bar and a translucent tab bar). Seen on a 375 px page |

## 14. Endings

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| END-01 | 2 | The defeat screen is not about this fight: two generic sentences with nothing about the enemies, who fell or how ("the defeat screen isnt contextual"). | P1, OWNER | Fixed `7e524f80`, `1b5a67d3`: the defeat text opens with what the party fought and who fell (browser check pending) |
| END-02 | 2 | After "The End" nothing on the TV or the phones offers a new game or a way back to the lobby; the phone still shows the composer. | P1 | Fixed `147a995e`: each seated phone shows "Start a new adventure" at The End; the run is restarted inside the process on a fresh database. Seen on a running TV and phone: both showed "A new adventure is being prepared...", and about twenty seconds later the TV had a new lobby with a different adventure and the phone had moved to the new room by itself |
| END-03 | 3 | The end card leaves the health cards and the last speaker's line visible under it, the scene picture changes behind it, and it can appear before the last line has finished. | P1, EARLIER | Part `3720b36`, `233ac51`, `209c7cd6`: overlays and the health strip are hidden and the card waits for the last line; the picture changing behind the card is not done |
| END-04 | 4 | Older victory-card faults (cut-out figures over a dark plate, text cut off, a bubble in mid-air) were not seen again: the victory screen was clean in playthroughs 2 and 3. Confirm and close. | EARLIER | |

## 15. Tools and tests

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| DEV-01 | 3 | The development command line cannot inspect or take over a session started from browsers: `dev health` and `dev state` answer "no seat has joined yet" while two players are in a fight. Health must not need a seat. | SC | Fixed `7526b4d` |
| DEV-02 | 3 | Refusals on the fight path all log `InvalidCandidate`, so a log cannot say which step refused (see SES-01, SES-02). | SR, SC | Part `65ff23c`, `d495f61`, `0f97c59d`: refusing steps are named, and the server log now covers turns, heals, fight ends and story steps |
| DEV-03 | 3 | `cargo test -p df-tools --lib` keeps nine long-standing failures, and the home-video tests time out under load. | EARLIER | Fixed `ff1dfb07`, `a9ea26a1`, `c4eeeb81`, `7ecfbb17`, `dbdcad0c`: all twelve long-failing tests pass. Confirmed by a full run on 2026-10-13: 1,313 passed, 0 failed. `8b0967d4` also fixed two generated-fight audits in df-combat that had failed since thrown weapons began to land |
| DEV-04 | 3 | The command line's back, forward and restore are written and unit-tested but have no verified transcript; direct edits (`override`) are disabled. | this session | |
| DEV-05 | 4 | The recording method used this evening (hidden browser with WebGPU, TV and one phone recorded) lives in the coordinator's scratch folder. Worth making a repository check so every fight change comes with a video. | SR | Fixed `0a9436f` |
| DEV-06 | 4 | Work in the tree since commit `c400f95` is uncommitted: the stopped workers' changes and this evening's roadmap sections. | this session | |

## 16. Found by reading the code

Thirty read-only reviewers each read one module on the evening of 2026-10-12 and reported only defects with a concrete trigger. Each finding below is unverified until a worker reproduces it; the full write-up with file, line, trigger and proposed fix is in `artifacts/tmp/campaign/hunt/<reviewer>/findings.md`.

| ID | Sev | Issue | Source | Decision |
|---|---|---|---|---|
| HNT-01 | 1 | A fight is refused at the start when the heroes were confirmed in a different order from the order they joined: the fight builds its hero list in confirmation order and the admission check expects join order. Very likely the real cause of SES-02 and of the refused fight behind SES-01, since the recording script confirms both heroes at once. | H12 F1 | Fixed `5abd72e`. Verified on a running server: refused before, starts now |
| HNT-02 | 3 | Restoring a pending move accepts a diagonal path through blocked corners and can leave the fight permanently stuck. Needs a hand-altered saved game to trigger, so lowered from severity 1. | H14 F1 | Fixed `0e8e4a1` |
| HNT-03 | 2 | A delayed attack command from an earlier turn can be accepted on the same hero's later turn. | H08 F1 | Fixed `cafd68c`, `e47c1a3` |
| HNT-04 | 2 | An "observe" investigation offer advances the story without resolving or recording what was observed (medium confidence). | H05 F1 | Open. Refusing the unfinished "observe" offer at validation made the adventure generator reject the shipped content (170 tests); set aside (`artifacts/tmp/campaign/luna6/F70/rejected.patch`). It needs a real handler for the offer, or the content must stop using it |
| HNT-05 | 2 | A wolf's or mastiff's bite knocks its target prone on every hit, without the Strength save the rules require. | H13 F1 | WRONG, being reverted (RUL-01): `cd92d25e` added a Strength save from the 2014 rules. SRD 5.2.1 says the bite knocks a Medium or smaller target prone on a hit, with no save |
| HNT-06 | 2 | A healed figure stays down on the TV during playback and its next action can be dropped. | H22 F1 | Fixed `4abbe0f` |
| HNT-07 | 2 | Start-up story text that came back incomplete is cached as complete, so refused slots are never asked for again. | H27 F1 | Fixed `a470d5e` |
| HNT-08 | 2 | After stepping back with the development tool, a late report from the TV can release a beat in the restored fight (medium confidence). | H15 F1 | Open, design needed: a restored fight needs a presentation generation in the wire contract so a late report from before the restore is ignored (`artifacts/tmp/campaign/luna6/F71/report.md`) |
| HNT-09 | 3 | Thrown weapons are used up but never recorded where they land, so they cannot be picked up. | H13 F2 | Fixed `0e8e4a1` |
| HNT-10 | 3 | When the server loses its ownership lease for good, an expired wake-up stays armed and the session thread spins. | H02 F1 | Fixed `33b0ba9` |

Causes the reviewers found for issues already listed above:

| Issue | Cause found | Source |
|---|---|---|
| SES-04 | A seat cannot be reclaimed after the room phase, and a fight has no inactivity deadline. | H03 F1, F2 |
| SES-05 | An outer deadline throws away prose groups that had already arrived when a later request runs long. | H24 F1, H27 F2 |
| SES-08 | A start-up failure after admission strands the local database and lets its retained credentials expire. | H30 F1 |
| FLO-06 | Authored ending labels are run through the code that turns action labels into sentences. | H07 F1 |
| FGT-05 | The attack list ignores the fighter build's greatsword-only restriction. | H10 F1 |
| SYN-01, SYN-02 | A later combat outcome identical to an earlier one is taken for an old beat and never queued; and a six-outcome cut before the queue discards deaths and earlier turns. | H17 F1, F2 |
| SYN-04 | The TV's health cards apply a heal before its beat is shown. | H18 F1 |
| SYN-05 | The phone re-enables turn-spending buttons from a stale view after the action's response. | H19 F1 |
| PHN-02 | The phone drops the server's death-save progress for a downed hero. | H19 F2 |
| FPX-01 | Rolls that arrive while the TV is catching up are deliberately drawn as stills, and almost every roll arrives that way. | H21 F1 |
| FPX-02 | Close-up camera poses for finishing blows and other actions are chosen without checking the scene's occupancy. | H23 F1 |
| STO-03 | The generated villain text has no matching named combatant and describes one arrival while the roster expands to several. | H25 F1 |
| DLG-02 | A hostile character's fallback repeats one curt answer once its few answer keys are used up. | H26 F1 |
| DLG-01 | The shipped pack has no social ledger, so spoken and typed lines never reach the code that records attitude. | H29 F1 |
| PIC-08 | The opening picture's key includes the party, and the lobby requests it at each confirmation. | H28 F1 |

## 17. Found by reading the code, second pass

Forty more read-only reviewers covered the modules the first thirty did not. Same rules: each finding has a concrete trigger and is unverified until a worker reproduces it. Write-ups are in `artifacts/tmp/campaign/hunt/W01` to `W40`.

| ID | Sev | Issue | Source | Decision |
|---|---|---|---|---|
| HNT-11 | 2 | The server will serve a file outside its configured folder through a symlink, for scan files and for the lobby-video cache. | W06 F1, F2 | Fixed `0f5b695` |
| HNT-12 | 2 | A stale End turn from an earlier round can use up the same hero's later turn (the same missing revision check as HNT-03). | W39 F1 | Fixed `e47c1a3` |
| HNT-13 | 2 | Push-to-talk audio recorded under one view can be sent under the next view's offer. | W20 F1 | Fixed `d8e4a5f` (browser check pending) |
| HNT-14 | 2 | Cancelling during a speech upload does not stop the browser's request or the server's transcription. | W30 F1 | Part `c813ce7`. The server half needs a disconnect signal the HTTP handler does not have once the upload body has arrived (`artifacts/tmp/campaign/luna6/F72/report.md`) |
| HNT-15 | 2 | Cleanup of model text can remove every meaningful word and still pass validation; Markdown formatting survives cleanup and reaches the spoken line. | W15 F1, F2 | Fixed `1222ca1` |
| HNT-16 | 2 | Duplicate speaker profiles in the content pack make a character's safety facts depend on array order. | W01 F1 | Fixed `7e6c6e1` |
| HNT-17 | 2 | A catch-up on the TV can erase a submitted scene transition and leave the TV on the previous scene (medium confidence). | W27 F1 | Fixed `129547c` |
| HNT-18 | 2 | A defeat scene that has authored dialogue is labelled as a victory (medium confidence). | W29 F1 | Fixed `224b74f` |
| HNT-19 | 2 | Scan level-of-detail range counts are trusted past the chunk size and can submit an invalid GPU dispatch (medium confidence). | W33 F1 | Fixed `80f4bb8` |
| HNT-20 | 2 | The browser's connection set-up can wait for ever, and join retries never stop when the tunnel cannot be made (medium confidence). | W40 F1, W26 F1 | Fixed `ed7d78b`, `56b69c7` (browser check pending) |
| HNT-21 | 3 | A morning scene whose text contains "knight" gets night lighting; a valid library scene is permanently marked as having no picture. | W02 F2, F1 | Fixed `5e97f97` |
| HNT-22 | 3 | An oversized picture cache file is reported as a successful image and then served as unavailable. | W04 F2 | Fixed `dc9c81f` |
| HNT-23 | 3 | A speech-intent loader can be left on the fight display when its line closes after the TV has gone back to a story scene. | W17 F1 | Fixed `6dcfb0e` (browser check pending) |
| HNT-24 | 3 | A dice folder setting that is not valid Unicode is silently ignored. | W07 F1 | Fixed `b0f0eb4` |
| HNT-26 | 1 | A fight with three or more enemies is refused at its start in 290 of 3,360 combinations of creature, scene, layout and count (every-creature test, 23:27). Cause found at 23:41, not fixed: during the enemies' first turns a pending move pauses the mover on an ally's cell and the snapshot check that every living combatant has its own cell rejects it. Test and probes: `artifacts/tmp/campaign/rules-a2/unfinished.patch`, `scn03-run1.log`. | rules worker | Fixed `876d2fde`, `00c60785`: the every-creature sweep went from 290 refusals in 3,360 to 0 |
| HNT-25 | 3 | Animals' sensory traits are missing from the creature catalogue and the gap is not disclosed (medium confidence). | W36 F1 | |

Causes the second pass found for issues already listed:

| Issue | Cause found | Source |
|---|---|---|
| PIC-01 | The scene picture can be built from a different hero appearance from the one chosen in character creation. | W04 F1 |
| PIC-04 | A hero at 1 hit point keeps healthy art when the hurt variant is not cached at the start of the fight. | W16 F1 |
| PIC-05 | Actions inside one scene never reach the picture update. | W16 F2 |
| AUD-02 | A defeat narration longer than 600 characters cannot be voiced, and nothing says so. | W05 F1 |
| AUD-03, FPX-06 | Sound cues are chosen from melee or ranged style only, so bites sound like swords and thrown javelins like bows. | W23 F1 |
| SYN-06 | The finale stinger is triggered by the completed server view, before the last beat is presented. | W22 F1 |
| SYN-09 | Downed combatants with portrait art stay as nameless dim circles in the turn strip. | W32 F1 |
| STO-08 | The phone repeats a saved private clue as narration on every later story view. | W19 F1 |
| DLG-03 | A "how did you meet" question filters out the generated acquaintance fact. | W13 F1 |
| DLG-07 | The fallback rotation is longer than the remembered reply history, so fallbacks repeat. | W14 F1 |
| DEV-01 | The development tool's read commands need its own seat cache to supply the session scope. | W11 F1 |
| END-03 | The end card appears when the final line starts lingering, before its hold and close finish. | W18 F1 |
| TVX-07 | The narrator caption is hidden after narration ends while choice hints are off. | W18 F2 |
| SES-07 | Offered story choices stay enabled while a press is rejected as busy. | W26 F2 |

## 18. Review of the fixes themselves

Sixteen independent read-only reviewers each checked four of the 63 fix commits made on the evening of 2026-10-12. Forty-nine commits were judged sound; one flagged fault was already fixed (`307c2b4`) and one was fixed at once (`b3f740ee`). The rest are below, unverified and unfixed. Write-ups: `artifacts/tmp/campaign/hunt/V01` to `V16`.

| ID | Sev | Issue in the fix | Commit | Decision |
|---|---|---|---|---|
| REV-01 | 2 | After a fight action is rejected while the same offer is still on screen, the phone may stay in its waiting state: the new "turn spent" gate treats any receipt as confirmation. This matters more now that refusals are answered with a recorded rejection (`1817d9e`). | `0761a5b` (SYN-05) | Fixed `2d76c3e6`: only an accepted action spends the turn's buttons (browser check pending) |
| REV-02 | 2 | Start-up story text: an outer deadline can still cancel the whole write when a later wave of group requests runs long, discarding groups that had arrived. | `ffed4a4` (SES-05, work in progress) | Fixed `14fc1fee`: groups that arrived before the deadline are kept. Tests pass |
| REV-03 | 2 | A member who takes damage and is then healed between two feed snapshots can end with the wrong hit points on the TV, because the heal hold skips members who were also attack targets. | `0bf7237` (SYN-04) | Fixed `f0a14bf4`. df-render tests pass |
| REV-04 | 2 | A typed hero line shown in the lower strip (a hero with no figure in the scene) is not closed before the reply's bubble opens; the hand-over only looks at figure bubbles. | `4c7e69b` (TVX-01) | Fixed `ab0fc916` (browser check pending) |
| REV-05 | 2 | Start-up cleanup after admission covers the campaign worker and the social ledger only; a later failing step (for example an invalid picture setting) still returns without releasing the session. | `7d0f493` (SES-08) | Fixed `1a66b6be` for the picture setting, the failing step named; other late start-up steps were not audited |
| REV-06 | 3 | The dice clip is forced visible with an inline style, which overrides the reduced-motion rule that should keep it a still. | `0d8934f` (FPX-01) | Fixed `d44deca0`: reduced motion shows the still; ordinary rolls still tumble |
| REV-07 | 3 | The close-up camera reserves room for a 0.85 push-in, but speech close-ups push in to 0.78, so a speaker can still leave the frame or be covered. | `7a3d505` (FPX-02) | Fixed `4852c21d`. df-render tests pass |
| REV-08 | 3 | The rules note is folded away on the TV as well as the phone, inside narration that is clipped to eight lines; the TV has no way to open it. | `5b9700c` (PHN-03) | Fixed `fc5f7d6c` (as REV-16): a row on the TV could never be seen under the battlefield, so the note is shown on the phones only |
| REV-09 | 3 | Cleanup of model text still accepts a Markdown list hyphen at the start of a line. | `1222ca1` (HNT-15) | Fixed `a6efb137` |
| REV-10 | 3 | The play-and-record check can pick a persistent control ("Play", "Full character sheet") as if it were a story choice. It still completes a game, but its log is misleading. | `0a9436f` (DEV-05) | Fixed `585d88ea` (selectors checked against the sources; the script was not run) |
| REV-11 | 3 | Second review round, 2026-10-13 (sixteen reviewers over the 64 fixes of the night; 57 judged sound). A speech bubble is not re-anchored when the same picture arrives with a new layout: the function returns early when the URL is unchanged. | `eca342df` (OBS-13) | Fixed `05e96ab2`: a loaded picture takes the new layout at once (not seen on a TV: screenshots of the TV page time out on this machine) |
| REV-12 | 2 | The phone's fight controls are still replaced after an action: offer ids change with every checkpoint, and the in-place update is keyed by offer id. Seen by hand as well (the Move button was a new node after an attack). | `134d711b` (OBS-20) | Fixed `8309c3b1`. What was really wrong: every redraw retires all click listeners and a control kept in place was never bound again, so its button was dead until the panel was rebuilt. Controls are now matched by what they do and bound to the offer they carry. Read on a running phone page: after a redraw that kept the nodes, Move and Attack were each accepted once. The controls are still rebuilt after the player's own action (REV-20) |
| REV-13 | 3 | The page a phone sees before it has joined still shows the neutral picture; only the waiting page takes the adventure's place. | `2cde9ae0` (OBS-02) | Fixed `af3b71bb`. Seen on a running phone page: the guardhouse picture loads behind the room-code form |
| REV-14 | 2 | A character's newest line is removed from the phone's log in a fight, where there is no story card to show it; and when a new scene reuses a speaker, the card can show that speaker's line from the previous scene. | `06b4e213` (OBS-14) | Fixed `8b538a82` (unit tests; the fight log on a running phone page shows the lines) |
| REV-15 | 4 | Cleaned model text may still start with a hyphen followed by a tab. | `a6efb137` (REV-09) | Not a fault: a tab is a control character and the line is already refused by the existing check; a worker confirmed it against the current code |
| REV-16 | 3 | The TV's rules note row sits under the full-screen battlefield layer, so it is never seen in a 3D fight. | `1cb72de3` (REV-08) | Fixed `fc5f7d6c`: the TV no longer mounts the note; the phones carry it |
| REV-17 | 3 | Only the giant weasel's "not used here" note is reworded; fights with giant rats or goblin warriors still show "Disclosed gap <id>". | `a0ae78d6` (OBS-46) | Fixed `fc5f7d6c`. Read on a running phone page: "House rule: cover is adjudicated along the segment between cell centres. Not used here: Climb Speed 30 ft is not modelled ...". The sentences themselves are still written for developers |
| REV-18 | 3 | Found while checking REV-12 on a running phone page: the row layout written for the phone's fight buttons (Dash, Dodge and Disengage across; Second Wind beside End turn) had stopped applying when the controls gained a wrapper, so every button took a full row. | `1cb72de3` (REV-08), `134d711b` (OBS-20) | Fixed `77265628`. Measured on a 375 px page: Dash, Dodge and Disengage share a row at 111 px each, Second Wind and End turn at 171 px each |
| REV-19 | 4 | The developer command line forgets its seats when the server is restarted on the same game: `dev act Sigrun ...` answers "unknown hero" and `dev join 2` is refused as a stale offer, so a restarted game cannot be driven on by script. Same family as SES-04. | Dev control | Fixed `953f1462`. Checked on a running server: after a restart on the same game, `dev act Brakka attack` was accepted with no new join |
| REV-20 | 4 | After every accepted fight action the server sends one view with no offers and the next, about 140 ms later, with the new ones; the phone removes its controls for the first and builds new ones for the second. Not visible to a player. | View delivery | Not a fault: by design the server withholds a player's offers while the beat is being shown (`table_log.rs`), so the phone has nothing to draw in between |
| REV-21 | 4 | The seat take-back records activity by writing the database on every authenticated view and request, and reads the away seats once a second, all on the session's own thread; a database stall would now stall views. Not measured. | `b42aa199` (SES-04) | Fixed `43270920`: activity is kept in memory and written at most once every five seconds per seat (at once when an away seat returns); a failed write is logged and no longer fails the view |
| REV-22 | 4 | The build disk filled during the night (0 GB free; "There is not enough space on the disk"): nine cargo target folders under `artifacts/build`, five of them left by workers that had finished. | Tooling | Fixed by hand: the five stale folders were removed (65 GB freed). The coordinator and the workers now use two target folders only |

## 19. Hand-played session, 2026-10-13 (room KESEZA)

One full game played by hand in a browser, at the owner's request ("scour the entire process for bugs", "just use computer use to observe the game"): one TV page and two phone pages at 375 by 812, from the lobby through hero creation, the opening scene, three typed lines to the patron, a three-round fight against three giant weasels, the victory screen, two aftermath scenes and "The End". Tree at `6d4ab2f7`. Text, voice and pictures on; the story text came from the model (50 of 54 slots); the village scan was forced as the battlefield by the session launcher; seed `0x982e5d9d6433d25a`, adventure "Perrin at the Guardhouse". Sound was not heard (the browser pane plays none to me); sound items come from the TV's network record. The server log (`artifacts/tmp/play-session-1/server.log`) holds no refusal for this game.

"Relates to" names the earlier row an item repeats or contradicts. Where that row was marked fixed and the fault was still on screen, the earlier row's decision has been changed to say so.

### The order I would take these in

1. **OBS-31, OBS-32, OBS-33, OBS-34**: the fight's timeline on the TV. Between enemy beats the TV falls back to an older state of the fight, a heal never reaches it, and the last blow is never shown. This is the SYN group seen plainly by hand, and the display has no log to diagnose it from (OBS-57).
2. **OBS-25, OBS-24, OBS-22**: characters do not know the adventure's state, and nothing a player says moves them.
3. **OBS-43, OBS-44**: the phone's fight panel.
4. **OBS-04, OBS-06, OBS-12, OBS-09**: four fixes from the evening of 2026-10-12 that are not effective on screen.
5. **OBS-48**: the short rest.

### Lobby, join and hero creation

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| OBS-01 | 3 | The TV lobby still draws a QR code that leads to a loopback address, right beside the line saying phones cannot join yet. | CRE-06 (text fixed, code left) | Fixed `49704baf`: no QR tile when the join address is this computer. Seen in a browser |
| OBS-02 | 3 | Phone join page: a harbour picture whatever the adventure's place; the heading "Join the table" is repeated by the card under it; the page holds two "Join this room" buttons, one of them off screen. | CRE-04 | Fixed `2cde9ae0`, `9f58bedf`: the place's own picture, and the card now says "Enter the room code from the TV." (read back from the running page) |
| OBS-03 | 3 | Developer wording is still all through the creation form: the opening paragraph, "…retained.", "three masteries", "Other lawful SRD combinations remain unsupported" (three times), "Fighter B + Soldier B", "Hero appearance (presentation only)", "Art only; this changes no character rules", "Draft · editing", "Supported rules SRD5.2.1-2024-local-journey-2+custom-…". | CRE-03 | Fixed `70a9d608`. Read back from the running page: the developer sentences are gone |
| OBS-04 | 3 | "A detail that makes you memorable" is still a text box of zero size. The fix did not take effect. | CRE-02 (marked fixed `edece58`) | Not a fault as built: the field is hidden on purpose because the server's character contract has no such field (`gameplay_browser.rs` "Flavor is not an admitted field"). Offering it is a feature, not a repair |
| OBS-05 | 3 | The heading "Give your story a name" sits over the field for the hero's name; the page opens scrolled past its own heading; the say-something composer is on the creation page, where there is nobody to talk to. | new | Fixed `70a9d608`, `dd795408`, `b2f7239f`: "Name your hero", no composer on the creation page, and the page heading is no longer printed twice. Read back from the running page |
| OBS-06 | 4 | The sticky "Confirm your hero" bar still covers the text of the Class card. The fix did not take effect. | CRE-07 (marked fixed `edece58`) | Fixed `16819357`: the cause was a 7 px gap under the bar and a translucent tab bar, not page length. Seen on a 375 px page |
| OBS-07 | 3 | Two starting packages have the same name and the same description ("Greatsword and Defense", for Stalwart and for Vanguard), and the appearance options change when the package changes, with nothing saying why. | new | Fixed `9e53905c`: the two packages differ in ability scores, which the page now shows (sent by the server for all three packages), and each keeps its name |
| OBS-08 | 3 | The tie rule printed on the form names the wrong enemy: "the GM chooses allies before the Bandit when tied" in an adventure whose enemy is a giant weasel. | CRE-03, FGT-08 | Fixed `9f26d62a`, `70a9d608`. Read back from the running page: "When turns are tied ... allies before the enemy" |
| OBS-09 | 3 | Phone waiting page: the loopback address is still printed under the room code (`127.0.0.1:63319/gameplay/player?room=…`); the place name "The Windward Pass" sits over the harbour picture; the party is shown as initials, not the portraits just chosen; the button that starts the game is below the fold. | CRE-06 (TV fixed, phone not), CRE-04 | Fixed `49704baf`, `270548ad`: the waiting page shows the room code and no address. Read back from the running page. (The first fix left the page blank for a local game until `270548ad`.) Picture, initials and button position remain |

### Story scenes on the TV

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| OBS-10 | 3 | The title card's sentence is repeated in the Dungeon Master box straight after it, and that same opening narration then stays in the box for the whole scene and comes back after every exchange, however much has been said since. | FLO-08 | Fixed `7218c949`: a refreshed picture is no longer a new scene, and the title card drops a sub-line the narrator is about to read (browser check pending) |
| OBS-11 | 3 | During the hand-over from narration to the patron's first line, the Dungeon Master box and the patron's line box are drawn on top of each other and neither can be read. | TVX-01 | Fixed `b787a808`: the narrator box leaves in 0.12 s (browser check pending) |
| OBS-12 | 3 | The scene title and place stay printed large in the top right for the whole of every story scene. The fix did not take effect in story scenes. | TVX-04 (marked fixed `3720b36`) | Fixed `98ba2bf2`: the title shows for about seven seconds after the cut, then is hidden; restarts each scene. Sampled in a browser |
| OBS-13 | 3 | The first speech bubble is drawn over the speaker's face, then jumps to another spot when the picture refreshes. | new | Fixed `eca342df`: figure positions are applied when the picture is on screen (needs a paid picture to confirm) |
| OBS-49 | 3 | The narration does not describe the picture, and one place has three looks: "the bothy is stone and turf, its stove glowing … Calla stands with her lantern held high" is read over a barn with a sheep pen and no lantern; the empty backdrop for the same place four seconds earlier is a stone room with a stove; the next scene, same place, is a different barn with hens and a cauldron. In the second scene "snow hisses over the stones" is read over an indoor guardroom. | PIC-06, STO-04 | |
| OBS-50 | 4 | The DungeonFlux logo shows through the scene title card during a change of scene. The epilogue has no title card: the screen is an empty backdrop for about two seconds. | new | |

### Story screen and composer on the phone

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| OBS-14 | 3 | The scene title is printed twice at the top; the speaker's newest line appears twice (cut short in the log, in full on the card); the main card keeps the scene's opening line after newer replies; log lines are cut mid-word with "…" and cannot be opened. | new | Fixed `06b4e213`, `ce75ec38`. Read back from the running page: one title, the newest reply on the card and not repeated in the log. Log lines are still cut short |
| OBS-15 | 4 | The hero sheet reads like stored data: "Skills insight, persuasion", "Alignment lawful good", "Languages Common, elvish, giant", "Starting equipment Fighter A + Soldier A · 18 GP", "Greatsword Held", "Weapon Longbow · Slow · +2 Archery". | CRE-03 | Fixed `da21f17b`. Read back from the running page: "Skills Insight, Persuasion", "Languages Common, Dwarvish, Giant", "Starting equipment 18 gp", "Greatsword (in hand)", "Alignment Lawful Good" |
| OBS-19 | 3 | Saying one line takes five controls (text box, Tap to talk, a "Just saying it" list, "Choose an offered action (optional)", then Say it, Decline this choice or Confirm offered choice). "Nothing happens until you confirm a choice" is untrue: Say it sends at once. "Decline this choice" and "Confirm offered choice" are shown with nothing offered. "Only spoken words: no action or check was made." stays on screen from then on. The composer is still there at The End. The page also holds a second, hidden "Say it" and a second unnamed text box. | PHN-04 | Fixed `dd795408`. Read back from the running page: new helper text, no decline or confirm with nothing offered |
| OBS-20 | 3 | Controls are rebuilt whenever anything on the page changes: "Say it" while typing, "Move" and "Attack" after picking from a list. A press aimed at the old control can land on nothing. Seen five times in this game. | PHN-08 (marked fixed `a252a22`) | Fixed `134d711b`. Checked on a running phone page: the same controls survive a list change and the attack that followed was accepted |
| OBS-21 | 3 | The patron's reply in the composer is shown with a hooded stock portrait while the story card above it shows his own. | PHN-04 | Fixed `dd795408` (browser check pending) |
| OBS-51 | 3 | The table log is out of order and incomplete: Calla's greeting is listed before "Brakka leads on to the Shepherds' Bothy"; two short rests are logged as one; the log is empty after leaving the page and coming back. | PHN-06 | Fixed `ef7747de`, `c4d94432`: an action reserves its log number before the commit, the phone keeps the server's order, and the log survives leaving the page (browser check pending) |
| OBS-56 | 4 | Menu: "Confirm saved choice" is not explained. Credits is a separate page that takes the phone out of the game, and reads "Local browser journey · player-created Dwarf Fighter / Soldier · selected SRD 5.2.1 rules. Other character options and mechanics remain in development." Coming back does resume the hero. | new | Part `9f58bedf`: the menu entry is shown only when it applies and says what it does. Credits wording unchanged |

### Choices, conversation and story

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| OBS-16 | 3 | "Ask Perrin what troubles him" is offered straight after he has said what troubles him, and the two choices are the same after three typed exchanges. | FLO-04 | |
| OBS-17 | 3 | The only choice before the fight names an enemy nobody has seen or heard of: "Shield Perrin from the giant weasel". | FLO-05 | |
| OBS-18 | 3 | The choices after the fight are chapter titles ("Calla's Quiet Welcome", "The Ledger's Last Road"), and the log quotes them back: Brakka chose "Calla's Quiet Welcome". | FLO-06 | Fixed `8a9fe3cb`: cause found, the story-text model was rewriting the generator's action labels into chapter titles; labels are no longer sent for rewriting. Needs a paid session to see |
| OBS-22 | 2 | A threat with drawn steel gets no reaction. The patron restates his errand, no roll is asked for, and the refreshed picture shows him smiling. | DLG-01, PIC-05 | Fixed `397e6e3d`, `44792e86`, `7edf7afd`: a threat cools the character before the reply, however it is worded (Jev reads the approach; 142 to 497 ms per line on a running server). With the story model off the reply is a stock line chosen for the new attitude |
| OBS-23 | 3 | Asked what the job is and what it pays, the patron answers "The exact job, I can't add to what I've told you. The pay, I don't know." Two scenes later he promises "I'll see you're thanked"; nothing is ever paid or mentioned again. | DLG-05, STO-02 | Open. Giving every patron one errand fact and a reward fact made generated adventures fail their own validation (a forbidden term in the errand line, and others) across seven suites; held at `artifacts/tmp/campaign/luna6/G08/held.patch` |
| OBS-24 | 3 | Plot facts shift between lines: the errand is to "reach Calla", then to carry the ledger "to the Shepherds' Bothy"; the animal enemy is spoken of by its table label as a schemer ("Whatever the Shaggy Giant Weasel wants with it, it won't ask twice"); the victory text says "Someone set the Shaggy Giant Weasel in your path". | STO-03, STO-09 | Open, with OBS-23 |
| OBS-25 | 2 | The contact does not know the adventure. Asked where Perrin is and what the ledger holds, Calla says the heroes have "come for the ledger", that she has "carried it myself", and that she has "no word of where they are", one screen after Perrin thanked the party at her door and one screen before the narration has her take the ledger from them. Her reply also starts a sentence in lower case ("it's heavier than it looks"). | STO-09, DLG-04 | Fixed `847473be`: the reply is given the scene's public narration and cast, ranked above the character's background. Tests pass; needs a paid session to hear |
| OBS-26 | 3 | Calla says nearly the same line in two scenes running ("I know a sealed cellar nobody has opened in years. Ask me when you are rested"), and the cellar is never offered or mentioned again. Perrin is in the narration of both aftermath scenes and in neither picture. | STO-07, FLO-04 | Open. A fix that removed the repeated hook from the other line broke lead-delivery continuity (the game tracks which line delivered the lead) and was taken out; needs the generator to choose one slot for the hook |
| OBS-52 | 3 | The epilogue and the end card close on a stray scenery sentence: "Ice glitters on the old waystation walls." It was first read in the guardhouse scene; at the end the party is in the bothy, and it is the only line on the end card. | STO-06 | Fixed `ea9430a9`: an ending sentence that repeats an earlier scene set elsewhere is dropped. Adventure tests pass |

### Fight: start, rules and what the players are told

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| OBS-27 | 3 | The TV goes from the choice straight to the loading card. The fight's introduction is on the phones only, and it describes one creature ("the Shaggy Giant Weasel pours from darkness … It rushes at you") where three take the field. | STO-03 | Part `87950360`: the fight's introduction is told before the first blow (browser check pending). The text still describes one creature where three take the field |
| OBS-28 | 3 | The fight is headed "WINDWARD PASS" and the story speaks of snow and ice; the battlefield is a green village with market tents. The session launcher forces the village scan, so this is the test set-up showing through, but nothing in the game ties the scan to the place. | FPX-10 | |
| OBS-29 | 3 | The initiative roll-call and the turn strip disagree. The tags show the weasels rolling 6, 16 and 9 from left to right; the strip lists them in name order after the first hero, and Brakka, also on 6, is last. Either the tags are on the wrong figures or the strip is not in initiative order. The enemy tags are still drawn tilted. The server log has no initiative line to settle it. | FPX-08, FGT-08, SYN-09 | Cause confirmed by the new server log line `COMBAT initiative order=[Giant Weasel 1:18, Giant Weasel 2:18, Giant Weasel 3:18, Sigrun:16, Brakka:11]`: the enemies share one roll and the strip is in order. What is left: the tags showing numbers before the dice have landed |
| OBS-36 | 3 | The table log contradicts the roll: "Brakka swings the greatsword — miss" for a graze that did 3 damage and is shown as GRAZE everywhere else; "Sigrun strikes with the longbow"; "Brakka moves" with no distance where an enemy gets "moves 4 squares (20 ft)"; the first enemy's move is missing. | FGT-03 | Fixed `c342181f` (wording), `c4d94432` (the phone no longer drops actions from its first view). Browser check pending |
| OBS-37 | 3 | Enemies never take an opportunity attack. Brakka walked out of the reach of two weasels; the server log shows his move split in three at the points where a reaction could be taken; none was, and nothing was shown or said. | FGT-04 | In progress (RUL-02): decision made on 2026-10-13, enemies take the opportunity attack whenever the rule allows, and a stored fight replays under the policy it began with. A worker is building it |
| OBS-38 | 3 | An animal talks: a speech bubble "GIANT WEASEL 2: Weakling!" at the start of its turn. | new | Fixed `79388033`: creatures with no language get no spoken lines. Tests pass |
| OBS-48 | 3 | The short rest. It is a story choice labelled "Short Rest · 1 hour, no Hit Point Dice"; taking it replaces the victory text on the TV with rules prose ("An uninterrupted hour has passed without Hit Point Dice or healing. Eligible heroes recover one Second Wind use; dead or stable 0-HP heroes are not automatically revived. The original victory and Perrin's unresolved journey remain."); it heals nobody (a hero on 7 of 13 has no way to recover for the rest of the game); it can be taken again and again (Second Wind went 0, 1, 2 in two presses); and it is offered again in the next scene. | FLO-07 | Fixed `f3e9f99e`: the rest is an event of the Aftermath state in the session machine, accepted once per fight and only while a living hero has something to regain; a stored game that had already rested still loads and plays on. Hit Point Dice are not implemented. The rules prose on the TV is unchanged |
| OBS-47 | 3 | The victory card says the party is "At the Shepherds' Bothy" under the title "The Ledger Reaches Shelter", then offers "Go on to the bothy". Each hero's state is the word "Awake". The hit point numbers inside the bars are too small to read from a sofa. Perrin's thanks are on the phones and not on the TV. | STO-04 | Part `0ac1d671`: "Awake" is gone and the patron's line is on the card (browser check pending). The card still says the party is at the place it then offers to go to (story text) |

### Fight timeline on the TV

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| OBS-31 | 2 | Between enemy beats the TV falls back to an older state of the fight. Round 2, in order, about two seconds apart: weasel 2 walks to Brakka; its bite card; "Bite · 7 piercing" with Brakka's card dropping to 4 of 13, BLOODIED; then a wide shot with weasel 2 back on its old square and Brakka's card back at 8 of 13, HURT; the heading changes to "Sigrun's turn" with her square lit; then "Giant Weasel 3's turn" and its bite. The same happened in round 1 (enemies back on the far row between their two attacks, the heading reading "Sigrun's turn" twice). The state is right once the next hero's turn begins. | SYN-01, SYN-02, SYN-07 | Fixed `283a76d6`, `deb37c2e`, `1db1e80e`, `8645c254`: see RUN-02, which has the recordings and the causes |
| OBS-32 | 2 | Second Wind never reaches the TV. No beat, no card, no number. Brakka healed from 8 to 11 on his phone and the TV kept "8/13 HURT" until he was next bitten, a full round later; he healed from 4 to 12 and the TV kept "4/13 BLOODIED" until the fight ended. The server log has no line for Second Wind either. | SYN-04 (part `0bf7237`), REV-03 | Fixed `deb37c2e`, `1db1e80e`: a heal is now tracked as a change of its own and reaches the battlefield card (recorded on a running TV page: 1 to 8 after Second Wind) |
| OBS-33 | 2 | The last blow of the fight is never shown. Four seconds after the press the victory card was up; no attack card, no hit, no fall. The fight's heading, its turn strip and the unshown attack's card ("Sigrun strikes Giant Weasel 3 · Shortsword") were visible through the victory card as it came in. | SYN-06 | Fixed `283a76d6`. On a running game (a defeat) the screen stayed on the fight through the finishing beat; a victory not yet seen |
| OBS-34 | 2 | The phone is ahead of the TV during enemy turns: Brakka's phone showed the second weasel's hit and "Brakka has 8 HP left" while the TV was still showing the first weasel's miss. | SYN-03 | Fixed `c05f53c7`, `a70bec66`: roll cards wait for the TV, including at a fight's start. The phone's hit point numbers are still the latest state |
| OBS-35 | 3 | On a hero's own turn his phone says "Your actions — Waiting for Brakka…" in a card under the working action panel. Both phones, every turn. | SYN-05 | Fixed `c90814b6`. Read back from the running page: "Your turn." |

### Fight presentation

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| OBS-30 | 4 | In the roll-call close-up the place card ("5th · Brakka — 6") covers the hero's shoulder and sword, and the figure hangs above its own shadow. | FPX-08 | |
| OBS-39 | 3 | A dead enemy leaves two round icons with no label and a pool of blood that are drawn over whatever is at that spot on screen: over Brakka's head and chest when he stood near the body (he looked like the one bleeding), over the health cards on the right in close-ups, and over the "Target · Brakka" label. | FPX-09 | Fixed `af39b8bb`: one mark per body, hidden while another figure overlaps it, under the health cards and target label (browser check pending) |
| OBS-40 | 3 | "FINISHING BLOW / ENEMY VANQUISHED" takes the whole screen for the first and the second of three enemies, and the camera stays in the close-up afterwards for the rest of the turn. | FPX-07 | Fixed `4fc941ef`: only the blow that ends the fight gets the full-screen banner (browser check pending) |
| OBS-41 | 3 | Close-ups frame the wrong thing: the other hero fills the foreground with her bow drawn at the attacker's back while the target is hidden behind him; a scimitar swing is shown with the bow drawn; heroes face the camera, not the enemy; floating text runs off the screen edge ("hortsword · 3 piercing"). | FPX-02, FPX-03 | Part `a27da647`: floating text is clamped inside the screen. The framing of close-ups (a bystander in front, heroes facing the camera) is not addressed |
| OBS-42 | 3 | The tumbling die appears in the middle of the field, away from the attacker (once directly above a corpse); two dice tumble for a single d20 roll; damage is still not rolled on screen. | FPX-01, FGT-02 | Part `5067cda1`: one die per d20, shown beside the attack card (browser check pending). Damage is still not rolled on screen (FGT-02) |

### Fight panel on the phone

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| OBS-43 | 2 | Still the developer lists. Movement is a list of 33 entries of the form "Column 1, row 3" with no grid shown on the TV. The attack list has 12 to 37 entries, one for every weapon, target, "mastery" and "Savage Attacker" combination, worded "Longbow → Giant Weasel 2 · half (+2 AC) cover · normal range: 10 ft · disadvantage" and "none cover". The entry chosen for the player can be the worst one (the longbow at disadvantage through half cover, with a sword that has neither lower in the list), and nothing says why a shot is at disadvantage. | PHN-01, FGT-06 | Fixed `1026cc4a`, `c90814b6`, `77265628`, `8ef03800`, `99f57003`, `f911eb05`: movement and aim by tapping a map, one attack entry per weapon and target (24 where there were 69), plain labels, a row layout. Read back from the running page |
| OBS-44 | 3 | Layout of the panel: labels sit against the left edge on top of the background picture; the lists are cut off ("Column '", "Greatswo"); End turn is under the bottom bar until the page is scrolled; the hit point number cannot be read where the bar's fill ends; a "Bolts 0" chip is shown to heroes who have no crossbow. | PHN-07 | Part `ad9572a3`, `1cb72de3`: panel, full-width lists (350 px on a 375 px page), small actions on one row. End turn is still below the first screen and "Bolts 0" remains |
| OBS-45 | 3 | The list of combatants gives a dead enemy "Conditions prone, unconscious, dead" and gives everyone's place as "Cell Column 4, row 1". | PHN-03 | Fixed `c90814b6`, `9f58bedf`: a dead enemy shows only "dead", and the position row is gone |
| OBS-46 | 3 | Opening "Rules for this fight" shows developer text: "RULESCombat uses the server-selected grid layout, source movement, range, cover and optional opportunity reactions … Disclosed custom rule tactical.cover-centre-line … Disclosed gap giant-weasel: Climb Speed 30 ft is not modelled". | PHN-03 (folded away, wording unchanged) | Fixed `a0ae78d6`. Read back from the running page: "Fights use a grid ... House rule: cover is judged along the line between the centres of the two squares. Not used here: the giant weasel's climbing speed, because the map is flat." |

### Ending, sound and tools

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| OBS-53 | 2 | After "The End" the phones say "A new game is started from the TV", and the TV has nothing to start one with: its only control is "Mute narration". | END-02 | Fixed `147a995e`: see END-02 |
| OBS-54 | 3 | The health cards are still visible, dimmed, behind the end card. | END-03 | Fixed `209c7cd6` (browser check pending) |
| OBS-55 | 3 | The TV asked for four music files and got 404 for each (`/assets/music/c-score-bed`, `-motif`, `-tension`, `-combat`), so this session had no score. The TV's console also logged one "play() request was interrupted because the media was removed from the document". | AUD-01 | Fixed `631ca1eb`: every suite, layer and variant now names a file that exists, with a test. Not yet heard. Recording a real finale set is an owner decision |
| OBS-57 | 3 | The logs cannot explain the TV's faults. For a whole game the server log holds attacks and moves only: no initiative order, no turn start or end, no Second Wind, no rest, no story choice or typed line, no state-machine transition, and nothing about what the TV has presented. The display keeps no log the command line can read. OBS-31 to OBS-34 were established from screenshots alone. | DEV-02 | Part `0f97c59d`, `d62a3020`, `523a00b8`, `99f57003`: the server logs turns, heals, endings, story steps, every state-machine transition and refusal, and requests refused for their shape. The TV's own beat record is in its `data-beat-log` attribute; it is empty while the page is not painting |
| OBS-58 | 2 | After the server process is killed (not shut down), starting it again on the same database is refused with `owned gameplay admission refused: live_owner RevisionConflict` for at least two minutes, so a crashed game cannot be resumed. A clean `dev shutdown` followed by a restart resumes the same room at once. Found while restarting test servers on 2026-10-13. | SES-08 | Fixed `761b5356`, `a7673dd5`: start-up waits out a dead owner's lease (up to 300 s) and takes the game back; a second server against a live game still exits. (The first version of this fix broke every action until `a7673dd5`.) Not yet tried against a real crash |
| OBS-59 | 4 | The two looks offered on the creation form are named "Greatsword hero" and "Second hero". | CRE-03 | Fixed `da21f17b`. Read back from the running page: "Red beard, greatsword" |
| OBS-60 | 3 | The server's new log lines are out of order at a fight's start (the opening attacks are printed before the initiative order and the turn lines), and beginning the story is not logged. | DEV-02 | Fixed `aa982f70`. Seen in a running game's log: choice, initiative, then each turn with its moves and attacks; beginning the story is logged |
| OBS-61 | 3 | A server started right after its database was created sometimes exited with "adventure database connection unavailable or deadline expired"; the same command ten seconds later worked. | SES-08 | Fixed `5272a175`: the first connection is retried for up to 20 s |
| OBS-62 | 3 | What a hero says during a fight reaches nobody. Room PANUVA: the second hero said "Brakka, take the left one" as a plan; the server answered "said" and logged `STORY say`, and the line appeared neither in the other phone's table log nor on the TV. The revision did not change, so no view was sent. | New | Fixed `c67a8419` for the phones. Seen in a running fight: the other phone's log showed "Sigrun: Brakka, take the left one" at once; a question to the game stayed private. The TV shows no speech during a fight |
| OBS-63 | 3 | A request the server refuses for its shape leaves no trace: the phone shows "Outcome unknown. Confirm the same operation or reconnect." and the server log has no line. Found when the new three-choice attack was refused by a two-choice bound; the phone then re-sent the same operation after a reconnect, by itself, and it was accepted. | New | Fixed `99f57003`, `21d2f25c`: the bound is three, the server logs a refused request, and the phone says "The game could not accept that. Choose again." and stops re-sending it (compiled, not seen) |

### Confirmed working in this game

- The scene title cards now carry each scene's own title (TVX-03).
- A hero's typed line is anchored to that hero's figure, for both heroes (TVX-02).
- The fight started with three enemies and the heroes confirmed out of join order (HNT-01; HNT-26 was not hit this time).
- After an action the phone's panel drops the spent buttons at once (SYN-05).
- The tumbling dice clip plays (FPX-01).
- A picture refresh changes the pose (PIC-03).
- Start-up story text arrived for every group on the first attempt (SES-05).
- A personal question got an answer from the character's own record ("No children. Harlan left years back, and good riddance. I manage alone.").
- Leaving the phone page and coming back resumes the same hero.

## 20. Hand-played session, 2026-10-13 (room RUREGO)

A second full game played by hand, on the build of 08:20 with the story model, voices and pictures on: one TV page and two phone pages, a greatsword fighter (Brakka) and an archer (Sigrun). Adventure "Seed-Grain Before Dawn": the bell chamber at the Rookhill Gate, a fight with two skeletons, the Vestry, the ending. About twelve minutes from the first press to "The End". The server log is `artifacts/tmp/play-session-1/server.log` (first part `server-run-RUREGO-part1.log`).

What worked that did not on the last hand-played game: the opening gave a job and a deadline; the patron answered a typed question to the point ("Wystan's a watch sergeant at the Vestry ... I can't name any pay"); each new scene held its choices for four seconds; the phone's fight map moved the hero and aimed the attacks; the attack list was six entries; the letterboxed finishing shot played only for the blow that ended the fight; the short rest was taken once and not offered again; the pictures showed two dwarves; the end card and the phones' end sentence were right. All ten story-text groups arrived on the first attempt.

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| RUN-01 | 1 | After the fight was won both phones went blank ("Reconnecting...", console "Aftermath controls could not be displayed", step `rest-create`). Nobody could choose anything; the game was stuck on the victory card. Cause: a healthy hero's condition text had been changed from "Awake" to an empty string, which the phone's rest screen refuses. In `main` since `0ac1d671`; the scripted check never reaches the aftermath on a phone. | OBS-47 | Fixed `103fe7a0`: the condition line is optional in the view. Seen afterwards on both phones in the same game |
| RUN-02 | 2 | Second Wind does not reach the TV's battlefield health card: Brakka went from 8 to 11 of 13 and the card stayed at "HP 8/13 · HURT" through the rest of the fight (the story-scene card and the phones said 11). The beat record has no entry for the heal. | OBS-32 (marked fixed `deb37c2e`) | Fixed `1db1e80e`, `8645c254`. Two causes, both found from recordings of the TV's cards: each new view rewound the battlefield's presented cursor, and a display running behind the server compared its held-back number with the newest result and staged a heal back to the old value (the owner's "health healing then immediately resetting"). Recorded on a running TV page over three rounds, sampled every 100 ms: every card changed only when its blow or heal was shown (8 to 11, 6 to 8, 11 to 13, 9 to 0, 8 to 13, 13 to 8, 9 to 4) and none returned to an earlier number |
| RUN-03 | 3 | The round counter runs ahead of what is shown: the heading read "ROUND 2 · Skeleton 2's turn" while the skeleton's round 1 shot was playing, and the roll card under it said "ROUND 1". | SYN-07 | Part `1db1e80e`, `8645c254`, `23c8fa48`, `45ea9a22`: the heading never names a hero during the enemy phase now (seen on a running TV). After the last blow the three enemies' headings are shown again in turn, 3.5 s each, with nothing happening (SYN-01) |
| RUN-04 | 3 | An enemy's ranged attack is framed on its target: "Shortbow · 5 piercing" was shown over Brakka standing beside the first skeleton's corpse, with the shooter nowhere in frame, so it read as the corpse being hit. | OBS-41 | |
| RUN-05 | 2 | A plain threat moved nobody: "Hand over the coin you owe us or I will break your arm." to the contact drew "Keep your voice down. Dangerous people are listening." and no attitude change was logged. The same wording to a patron was recognised in a keyless game. | OBS-22 | Fixed `44792e86`, `7edf7afd`: threats are read before bribes, and each spoken line is now classified by the decision model Jev before the reply (the owner's decision), with the keyword reader only as the fallback. Checked on a running server with the real service: an indirect threat with none of the listed phrases came back "intimidation" and cooled the patron |
| RUN-06 | 3 | The phone's table log writes story choices as 'Sigrun chose "Settle things with Wystan"' and 'Brakka chose "Close the chapter"'. Earlier choices read as actions ("offers Rowan their help"). | New | Fixed `8c0f9759`: errand choices read as actions ("settles things with Wystan", "closes the chapter"), tested against every label the tables produce |
| RUN-07 | 3 | Phone fight page: the controls sit at the top of the page above the fight's title, narration and log, and a separate "Your actions" card at the bottom says only "Your turn."; the map's chosen square starts on an arbitrary far cell ("Column 1, row 3") that Move would act on; a greatsword fighter is shown "Arrows 20 · Bolts 0". | OBS-44 | Part `6ee86b2f`, `baa51c53`: the redundant "Your actions: Your turn." card is hidden on the hero's own turn, and the map can no longer be drawn as a few enormous squares (it was 1,700 px tall with no move left). The controls still sit above the fight's title and narration, and the map's chosen square still starts on an arbitrary cell |
| RUN-08 | 3 | The fight's introduction names one "Rusted Sentinel" with a shortsword; the fight is two bow-armed creatures called "Skeleton 1" and "Skeleton 2". | STO-03 | Part `b4c7cbdf`: the storyteller is told the creature count, kinds and real weapons and to describe exactly those. With the story model off the pinned pattern text still names one villain |
| RUN-09 | 3 | The aftermath reads "Bloodied, you reach the vestry" and then offers "Follow Rowan to the Vestry · the Vestry": the party is told it has arrived before it is asked to go, and the destination is printed twice. | OBS-47 | Part `baa51c53`: the destination is printed once. The aftermath text still says the party has reached the place before asking it to go |
| RUN-10 | 3 | On one generated picture the speech bubble lay over the speaker's head: the bubble was on the left of the head and its right edge passed the head's centre (bubble 423 to 679 px, head centre 645 px, on a 1466 px wide TV page). On the next picture it sat beside the head correctly. | OBS-13 | Fixed `9c6886d2`: bubbles are placed through the picture's real crop, with native tests at 16:9 and 4:3 (not seen on a TV) |
| RUN-11 | 4 | Hero creation: with the archer package the only look is "Woman with a bow" and it is not preselected, so the form cannot be confirmed until its one option is tapped. | CRE-01 | Fixed `eeeb2093` (df-ui test; not seen on a phone) |
| RUN-12 | 4 | The end card came about 22 seconds after the last choice, with the TV showing only a picture in between; the phones at "The End" still offer "Say something else...". | END-02 | Part `eeeb2093`: no composer at The End (compiled, not seen). The 22 seconds before the end card is unchanged |
| RUN-13 | 4 | The phone's page before joining showed the bell chamber and the TV lobby showed the Rookhill Gate: two pictures for one waiting room. Once joined, the phone's waiting page matched the TV. | REV-13 | Fixed `a97b3648`: the page before joining asks the session owner for the lobby's public picture name, with a 250 ms fallback to the opening picture |

Seen again, already listed: the battlefield is the green village scan whatever the story's place (OBS-28; this session is started with that scan forced); the initiative tags are drawn tilted (FPX-08); one smeared frame during an attack zoom (FPX-04); the short rest replaces the victory text with rules prose (OBS-48); "Ask Rowan what troubles her" is offered straight after she has said it (OBS-16); two starting packages share a label (OBS-07).

## 21. Scanned battlefields looked at with the owner, 2026-10-13

The owner watched the TV page while the four scans were tried in turn on a test server (two greatsword heroes against three giant weasels, grid overlay on) and asked for changes as he saw them. Each row is something seen on that page.

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| SCA-01 | 2 | A hero stood on the line of the village palisade, drawn half through the stakes (the owner's picture: "no more collisions like this"). | SCN-01 | Fixed `d29a3a4f`: see SCN-01. Seen on a running TV page: the squares under the palisade are refused by the server and a hero walks to the free square in front of it |
| SCA-02 | 3 | With the first version of the fence rule an enemy standing in the open could not be shot: a line that only touched the end of the palisade, or passed a corner a figure cannot squeeze round, counted as blocked. | SCN-01 | Fixed `d29a3a4f`: a wall's end and a blocked diagonal step stop no line; a line through a joint or a corner with wall on both sides is stopped. On a running server all three enemies in the open can be targeted |
| SCA-03 | 3 | The summit scan is coarse leaf-like shards wherever the camera looks (the owner: "the camera spawn point is dogshit"). The finest level of detail looks the same, and so does the scan seen from directly overhead: it is the capture, not the placement. | SCN-02 | Open: the scan is not fit for a fight as captured. It stays a private test scene outside the rotation |
| SCA-04 | 3 | Valley: the wide shot looked through a trunk and a band of blurred leaves in front of the lens. | | Fixed `fc1fc3f9`, `d29a3a4f`: nothing of a scan is drawn in the nearest part of the way from the camera to what it looks at; the valley's wide shot looks down at 52 degrees and its trees are cut at 3.2 m. Seen on a running TV page |
| SCA-05 | 3 | Valley: the grid sat away from the bridge (the owner: "move the grid closer to the bridge as it's more central"). | | Part `d29a3a4f`: moved one row toward the bridge, which is now centred behind the enemies. Two rows leaves a corridor two squares wide between the trees (9 connected squares), and the scan's own centre is a ravine with two bridges and no open ground (tried with the collider: three free squares) |
| SCA-06 | 3 | Valley: the grey ragged edge of the scan showed at the bottom of the picture (the owner: "it needs to move more central to avoid hitting the grey edge of the gaussian"). | SCA-05 | Part `fc1fc3f9`, `d29a3a4f`: the scan now fades out 10.5 m from the centre and the tilt-shift softens the near ground; the ground just south of the grid is still soft, because the capture is thin there and the grid cannot move further in (SCA-05) |
| SCA-07 | 3 | The owner asked for a tilt-shift so the eye rests on the heroes and their enemies. | | Fixed `fc1fc3f9`: the band the figures stand in is sharp and the picture blurs above and below it; the band follows the figures. Seen on the village, the riverbed and the valley |
| SCA-08 | 3 | From the steeper shot three giant weasels on a brown forest floor could hardly be found. | | Fixed `3b0e4114`: a thin ring on the ground under every standing figure, blue for a hero and red for an enemy. Seen on a running TV page |
| SCA-09 | 4 | The test server reads the browser files once at startup: a rebuilt page is not served until the server restarts, which cost two wrong looks. | DEV | Open, low: note for whoever tests next (restart on the same database keeps the game) |
| SCA-10 | 3 | A fight stored by one build of the rules is refused at startup by a build whose rules differ ("checkpoint_decode InvalidCandidate"): seen when the line-of-fire rule changed between two builds on the same test database. No released build is affected, but a rules change will strand games in progress. | | Open: decide whether a stored fight is replayed under the rules it began with (a rules version in the record) or is ended cleanly |

## 22. Play checked against the rule book (SRD 5.2.1), from 2026-10-13

Each row is something seen in a played game and compared with the text of `development/rules-sources/srd-5.2.1-en.pdf` (the 2024 rules; page given where it was read). Rooms KUNAPE (village, two greatsword fighters against three giant weasels, six rounds, played partly from a phone page) and after. "Agrees" rows are kept so the next check does not repeat them.

| ID | Sev | Issue | Rule book | Decision |
|---|---|---|---|---|
| RUL-01 | 2 | A wolf's or a mastiff's bite makes the target roll a Strength save (DC 12, DC 11) and knocks it prone only on a failure. This was added on purpose in `cd92d25e` (HNT-05) from the 2014 rule, and three tests were then changed to match it (`050729eb`). | Wolf and Mastiff stat blocks: "Hit: ... Piercing damage. If the target is a Medium or smaller creature, it has the Prone condition." No save. | Fixed `0086ff8e` for the rules engine and its tests (df-rules 133 passed); the matching simulator tests in df-combat follow with RUL-02 |
| RUL-02 | 2 | Enemies never take an opportunity attack: a hero walked out of the reach of two weasels and the server paused the move at each point, took nothing and showed nothing. Disengage, offered every turn, is worthless. | "Opportunity Attacks": "You can make an Opportunity Attack when a creature that you can see leaves your reach ... take a Reaction to make one melee attack"; avoided by Disengage. | In progress (decision made: enemies take it whenever the rule allows; a stored fight replays under the policy it began with). See OBS-37 |
| RUL-03 | 3 | The phone's character sheet lists "Skills Perception, Survival": the two chosen Fighter skills. The Soldier background's Athletics and Intimidation are missing, and so are the saving throw proficiencies (Strength, Constitution), the Proficiency Bonus, Speed, Initiative, Hit Point Dice, armour and weapon training and the Dwarf's traits (Darkvision 120 ft., Dwarven Resilience, Stonecunning). Not yet checked: whether the server's ability checks use the background's two skills. | Soldier background: "Skill Proficiencies: Athletics and Intimidation". Fighter: "Saving Throw Proficiencies: Strength and Constitution". Dwarf traits. | Open |
| RUL-04 | 3 | Savage Attacker is applied correctly (two rolls of 2d6, 6+4 and 6+5, the higher kept, +3 = 14) and shown wrongly: the phone's roll card reads "14 damage (6 + 4 + 6 + 5)", which adds to 21, and never shows the +3. | Savage Attacker: "roll the weapon's damage dice twice and use either roll". | Open: show both rolls, which was kept, and the modifier |
| RUL-05 | 4 | Second Wind can be used at full hit points: it healed 0 and spent a use, with no warning. | Legal as written (a Bonus Action to regain Hit Points). | Open, low: say "you are at full hit points" on the control, or ask once |
| RUL-06 | 3 | After shooting a shortbow (a two-handed weapon) the sheet still read "Greatsword (in hand)", and the next turn's greatsword attack was offered without a word about changing weapons. Not yet checked in the engine: whether the bow shot stowed the sword, or hands are not enforced for a bow. | "Equipping and Unequipping Weapons": one weapon may be equipped or unequipped when you attack as part of the Attack action. Shortbow and Greatsword: Two-Handed. | Open: check the engine, then show what is in hand truthfully |
| RUL-07 | 3 | Every barrier taken from a scan gives Total Cover, whatever its height: a hero could not target a weasel behind a knee-high bush in the village. | Cover: Half "covers at least half of the target", Three-Quarters "at least three-quarters", Total "covers the whole target". | Open: the collider knows how high the solid voxels reach; grade the cover by that height against the target's |
| RUL-08 | 3 | A Short Rest gives no way to spend a Hit Point Die: the control says "Short Rest · 1 hour, no Hit Point Dice", and a hero at 9 of 13 stayed at 9. | Short Rest: "Spend Hit Point Dice. You can spend one or more of your Hit Point Dice to regain Hit Points ... roll the die and add your Constitution modifier ... (minimum of 1 Hit Point)". A level 1 Fighter has one d10. | Open |
| RUL-09 | 3 | The text after a short rest reads like a disclaimer: "An uninterrupted hour has passed without Hit Point Dice or healing. Eligible heroes recover one Second Wind use; dead or stable 0-HP heroes are not automatically revived. The original victory and Perrin's unresolved journey remain." | | Open: write it as story, and move the rules note under "Rules for this fight" |
| RUL-10 | 4 | On the TV the floating result of a blow ("Greatsword · 3 slashing, graze") runs under the enemy cards at the right and is cut off. | | Open |
| RUL-11 | 4 | After a phone takes a hero back, the development tool's `state` command answers "local gameplay binding denied" for the rest of the game (it reads through that hero's old seat). `view --as tv` still works. | | Open, tool only |
| RUL-12 | 3 | Not yet seen in play, to check next: the Slow mastery of a thrown javelin (the weasel hit by one still had 40 ft. of movement, but the throw came from the development tool, which may not ask for the mastery); the dying rules (death saving throws, damage at 0 hit points, a hit from within 5 feet on an unconscious hero being a Critical Hit); cover from creatures; Dodge and Disengage; long range; each of the other eleven creatures' traits. | | Open: play these |
| RUL-13 | 2 | A dying hero cannot be helped. With Sigrun at 0 hit points making death saving throws beside him, Brakka was offered Attack, Dash, Dodge, Disengage, Second Wind and End turn: nothing to stabilise her, though every hero's starting equipment lists a Healer's Kit. | "Stabilizing a Character": the Help action with a DC 10 Wisdom (Medicine) check. Healer's Kit: "ten uses. As a Utilize action ... stabilize an Unconscious creature that has 0 Hit Points without needing to make a Wisdom (Medicine) check." | In progress: a first worker stopped at the rule against wire changes and wrote no code; a second has leave to append to the wire contract. Correction from the first: only the greatsword packages carry a Healer's Kit (the archer took 50 GP instead), so the archer gets the Wisdom (Medicine) check only |
| RUL-14 | 2 | Two wolves with one hero dying at their feet and the other hero out of reach took the Dodge action every round, for as long as the fight was left to run. Enemies never attack a dying hero (the game's own mercy rule), so with no one else to reach they stand still: the fight cannot end unless the last hero shoots them down. Their turns are not in the server log at all (no `COMBAT turn` line, nothing about the Dodge), and the TV shows nothing happening. | Nothing forbids a monster attacking a creature at 0 hit points ("Damage at 0 Hit Points": one Death Saving Throw failure, two on a Critical Hit). The mercy rule is the game's choice. | Open: log and show every turn, including "takes the Dodge action" and "has no one to reach"; decide what an enemy does when only dying heroes are in reach (proposal: beasts and mindless undead attack the dying, others turn to the nearest standing hero and Dash) |
| RUL-15 | 4 | A hero lying unconscious and prone between a thrower and his target gave the target Half Cover (+2 AC). | Half Cover: "another creature or an object that covers at least half of the target". A Medium creature lying on the ground does not cover half of a wolf. | Open, low: a Prone creature gives no cover to a target of its size or larger |
| RUL-16 | 3 | The phone's roll card hides the second d20: an attack made with Disadvantage (11 and 15 rolled, 11 used) reads "d20 11 +5 = 16" with no word that there was Disadvantage or why; the target's Armor Class (12, +2 for cover) is not shown either, so "HIT" cannot be checked by the player. The server log has all of it. | "Advantage/Disadvantage": roll two d20s and use the higher or lower. | Open: show both dice, the reason, and the Armor Class the total was compared with |
| RUL-17 | 3 | A goblin warrior spends its whole first turn taking off its shield so that it can use its shortbow (the engine's own rule), then fights at Armor Class 13. That turn is not logged and nothing shows on the TV: two goblins stand still for a round. | The stat block gives AC 15 and both Scimitar and Shortbow as actions, with no cost to change between them; "Gear" is what the creature carries. The rule for donning and doffing a Shield (a Utilize action) is written for characters' equipment. | Open: decide (proposal: a stat block's actions are used as written, at the stat block's AC) and in any case log and show the turn |
| RUL-18 | 3 | The server log's attack line does not say when a weapon mastery was used or what it did (Slow, Sap, Vex, Graze is visible only as damage on a miss), nor a saving throw made because of an attack. Whether the javelin's Slow took 10 ft. off a wolf's Speed could not be read from the log. | Slow: "reduce its Speed by 10 feet until the start of your next turn". | Open: add `mastery=` and `effects=[...]` to the line |
| SCA-11 | 2 | Village, west-east layout: a hero starts in a pocket. The square c2r4 has one way out, through the square where the other hero starts. When that hero fell there, the first was offered no Move at all (but still Dash and Disengage), the wolves could not reach him, and the fight stalled (RUL-14). The check that passed this layout only asks for one free step from each start square. | | Open: a start square must have two ways out and must not be a dead end of the walkable area; re-derive the four scans' start squares and add the check to the scene tests. Dash and Disengage should not be offered to a hero who cannot move |
| RUL-19 | 3 | Talking to a character never calls for a roll. A hero can threaten, bribe, flatter or lie; a model labels the approach and a trust counter moves the character's attitude one step. The rules module for the Influence action exists and is tested (`crates/df-rules/src/influence.rs`), and the social layer's own header says it "is NOT the standard Influence action, which is an ability check that does not exist yet". | "Influence [Action]" (p. 184): a Hesitant monster calls for an ability check, Charisma (Deception, Intimidation, Performance or Persuasion) or Wisdom (Animal Handling), "default DC equal to 15 or the monster's Intelligence score, whichever is higher", Advantage if Friendly, Disadvantage if Hostile; on a failure the same urging waits 24 hours. | Open: when a hero urges a character to do something it is hesitant about, roll it through the existing module, show the roll on the phone and the TV like any other, and let the result decide; keep the trust counter only for tone |
| RUL-20 | 3 | An "observe" choice in the story advances it without any check and records nothing about what was seen (HNT-04). | Search action: "Wisdom (Perception)" to notice something; Study action: Intelligence checks; "Typical DCs": 10 easy, 15 medium, 20 hard. | Open: make it a Search or Study check with a stated DC, show the roll, and give different text for success and failure; see HNT-04 for why refusing the offer outright broke the content |
| RUL-21 | 4 | A hero's stabilising or helping options aside (RUL-13), the Help action to give an ally Advantage on an attack ("Help [Action]": assist an attack roll against an enemy within 5 feet) is not offered either, nor Hide, Ready, Search, Study or Utilize for anything but picking up a weapon. The fight's rules note does not say so. | The list of actions in "Combat": Attack, Dash, Disengage, Dodge, Help, Hide, Influence, Magic, Ready, Search, Study, Utilize. | Open, low: say which actions the game does not offer in the rules note players can read; Help (attack) is the one worth building |

Agrees with the book, seen in the log: attack bonus +5 (Strength 17, Proficiency Bonus +2) and +3 with a shortbow (Dexterity 13); greatsword 2d6+3; thrown javelin 1d6+3 at 25 ft.; a natural 20 doubling the dice (2d6+1 with the shortbow); Disadvantage on a ranged attack with an enemy within 5 feet (both dice shown, the lower used); Half Cover (+2 AC) from an ally in the line of fire; Graze dealing the Strength modifier on a miss, and killing with it; Savage Attacker's two rolls; Second Wind 1d10+1, two uses, one back after a Short Rest; Armor Class 17 (chain mail 16, Defense +1); 13 hit points (d10, Constitution +2, Dwarven Toughness +1); the giant weasel's stat block (AC 13, 9 hit points, Speed 40 ft., Bite +5 for 1d4+3); moving through an ally's square but not ending there; a blocked square refused; enemies rolling one Initiative as a group and a tie going to the hero by the table's stated rule.

Also agrees, from ten short test fights (two of each creature, logs kept in `artifacts/logs/rules/`) and an archer played from a phone in room FERUFA: skeleton (AC 14 not tested, 13 hit points, Shortbow +5 for 1d6+3 at 80/320); zombie (15 hit points, Speed 20 ft., Slam +3 for 1d8+1, a natural 20 rolling 2d8+1); goblin warrior (10 hit points, Shortbow +4 for 1d6+2; but see RUL-17); kobold warrior (7 hit points, Dagger +4 for 1d4+2, Pack Tactics and Sunlight Sensitivity cancelling to a single die); scout (16 hit points, Multiattack with two Longbow attacks at +4 for 1d8+2); bandit (11 hit points, Light Crossbow +3 for 1d8+1); guard (Spear +3 for 1d6+1); cultist (Ritual Sickle +3, 1d4+1 Slashing plus 1 Necrotic: the log line shows only the Slashing part, the hit points fall by the full amount); giant rat (7 hit points, Bite +5 for 1d4+3, Pack Tactics); wolf (11 hit points, Bite +4 for 1d6+2, Pack Tactics with an ally within 5 feet); the death saving throw at the start of a dying hero's turn (10 or more succeeds); attacks against a creature that took Dodge having Disadvantage; the archer's sheet (AC 15 from studded leather and Dexterity 17; 61 GP from Fighter B and Soldier B) and her Longbow at +7 (Dexterity +3, Proficiency +2, Archery +2) for 1d8+3 with Savage Attacker's two rolls; the Longbow's Slow mastery (the weasel hit by it moved with 30 ft. of Speed and the phone listed it as "slowed"); the Light property's extra attack (Scimitar then Shortsword, both at +5 with Dexterity, the second spending the Bonus Action because the Nick weapon made the first attack, not the extra one).

## State of the forward state machine

Commit `da6dc50a` adds `gameplay/journey/session_machine.rs`: one typed machine with six states derived from the checkpoint (Lobby, Story, Conversation, Fight, Aftermath, Ended), nine events taken from the selected offer's effect, and one table `next(state, event)` that returns a transition or a typed refusal and only ever moves forward. Every staged command is checked against it after the offer is selected and before any handler runs; a refusal is logged with `step=session-machine`. Its table test and whole-adventure property test pass, and the story and beat-script suites pass with the gate in place. Design note: `artifacts/tmp/campaign/flow-a2/STATE-MACHINE.md`.

Since then (2026-10-13, worker flow-a3):

- `00c60785`: fights with three or more enemies no longer refused at their start (HNT-26; the every-creature sweep of 3,360 combinations has 0 refusals).
- `182fad8a`: offers come from the machine: for each state the events it accepts decide which kinds of offer are built, and a property test checks that every offered action is one `next` accepts.
- `f3e9f99e`: Aftermath has its own rule for the short rest (OBS-48): once per fight, and only while a living hero has something to regain; a stored game that had already rested still loads.
- `e62c349d`: the fight and the conversation are entered through one function each (`enter_fight`, `enter_conversation`), called only after the machine accepted the event, and after staging the machine checks that the candidate landed where the transition named (`outcome_legal`): a fight is entered only by StartFight and left only into Aftermath, a resolution ends only in Aftermath or Ended, and nothing moves backwards. Refused with `step=session-machine-outcome`.

Not done: the machine's transitions are not yet written to the server log (`STATE ...` lines); the Ended state accepts nothing, so there is no new game (OBS-53). The worker ran out of its usage allowance during the third step; the coordinator finished the checks and committed it.

## Fixed and confirmed this evening

Kept here so they are not triaged again.

- Every character reply was stock text because the mature-rating reply rules were 4,171 characters against a 4,000 limit. Trimmed, with a test guarding the limit; five of six replies then came from the model.
- The server refused to start on a dice manifest it could not read. The manifest was rewritten to the layout the parser expects.
- Victory and defeat now have clean, separate screens.
- The loading card shows the narrator's caption.
- Music ducks under a voiced line (it does not yet come back promptly: AUD-01).
- The server starts on Windows without the manual stack patch.
