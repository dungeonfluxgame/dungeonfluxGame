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
| HNT-14 | 2 | Cancelling during a speech upload does not stop the browser's request or the server's transcription. | W30 F1 | Fixed `c813ce7` (the browser stops its request), `4c919660` (cancelling stops the server's transcription: the provider call gets a cancellation signal) |
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
| OBS-37 | 3 | Enemies never take an opportunity attack. Brakka walked out of the reach of two weasels; the server log shows his move split in three at the points where a reaction could be taken; none was, and nothing was shown or said. | FGT-04 | Fixed `60a76918`: see RUL-02 |
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
| RUL-02 | 2 | Enemies never take an opportunity attack: a hero walked out of the reach of two weasels and the server paused the move at each point, took nothing and showed nothing. Disengage, offered every turn, is worthless. | "Opportunity Attacks": "You can make an Opportunity Attack when a creature that you can see leaves your reach ... take a Reaction to make one melee attack"; avoided by Disengage. | Fixed `60a76918`: enemies take the opportunity attack whenever the rule allows, with a natural weapon, a held melee weapon or an Unarmed Strike; the policy is versioned in the fight's stored record, so an old fight replays unchanged. Not yet: a bite that knocks the mover prone should stop the move (with the enemy-turn work). Not yet seen in a played game |
| RUL-03 | 3 | The phone's character sheet lists "Skills Perception, Survival": the two chosen Fighter skills. The Soldier background's Athletics and Intimidation are missing, and so are the saving throw proficiencies (Strength, Constitution), the Proficiency Bonus, Speed, Initiative, Hit Point Dice, armour and weapon training and the Dwarf's traits (Darkvision 120 ft., Dwarven Resilience, Stonecunning). Not yet checked: whether the server's ability checks use the background's two skills. | Soldier background: "Skill Proficiencies: Athletics and Intimidation". Fighter: "Saving Throw Proficiencies: Strength and Constitution". Dwarf traits. | Open |
| RUL-04 | 3 | Savage Attacker is applied correctly (two rolls of 2d6, 6+4 and 6+5, the higher kept, +3 = 14) and shown wrongly: the phone's roll card reads "14 damage (6 + 4 + 6 + 5)", which adds to 21, and never shows the +3. | Savage Attacker: "roll the weapon's damage dice twice and use either roll". | Open: show both rolls, which was kept, and the modifier |
| RUL-05 | 4 | Second Wind can be used at full hit points: it healed 0 and spent a use, with no warning. | Legal as written (a Bonus Action to regain Hit Points). | Open, low: say "you are at full hit points" on the control, or ask once |
| RUL-06 | 3 | After shooting a shortbow (a two-handed weapon) the sheet still read "Greatsword (in hand)", and the next turn's greatsword attack was offered without a word about changing weapons. Not yet checked in the engine: whether the bow shot stowed the sword, or hands are not enforced for a bow. | "Equipping and Unequipping Weapons": one weapon may be equipped or unequipped when you attack as part of the Attack action. Shortbow and Greatsword: Two-Handed. | Open: check the engine, then show what is in hand truthfully |
| RUL-07 | 3 | Every barrier taken from a scan gives Total Cover, whatever its height: a hero could not target a weasel behind a knee-high bush in the village. | Cover: Half "covers at least half of the target", Three-Quarters "at least three-quarters", Total "covers the whole target". | Open: the collider knows how high the solid voxels reach; grade the cover by that height against the target's |
| RUL-08 | 3 | A Short Rest gives no way to spend a Hit Point Die: the control says "Short Rest · 1 hour, no Hit Point Dice", and a hero at 9 of 13 stayed at 9. | Short Rest: "Spend Hit Point Dice. You can spend one or more of your Hit Point Dice to regain Hit Points ... roll the die and add your Constitution modifier ... (minimum of 1 Hit Point)". A level 1 Fighter has one d10. | Part `e5a7f734`: the healing rule exists in the rules crate with its tests. The rest's flow, the per-hero choice and the stored state are with a second worker |
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
| SCA-11 | 2 | Village, west-east layout: a hero starts in a pocket. The square c2r4 has one way out, through the square where the other hero starts. When that hero fell there, the first was offered no Move at all (but still Dash and Disengage), the wolves could not reach him, and the fight stalled (RUL-14). The check that passed this layout only asks for one free step from each start square. | | Fixed `30cbf359`: start squares re-derived for the village, the valley and the riverbed; the summit is marked not playable and left out. Tests and the 3,168-combination sweep; not yet seen in a played game |
| RUL-19 | 3 | Talking to a character never calls for a roll. A hero can threaten, bribe, flatter or lie; a model labels the approach and a trust counter moves the character's attitude one step. The rules module for the Influence action exists and is tested (`crates/df-rules/src/influence.rs`), and the social layer's own header says it "is NOT the standard Influence action, which is an ability check that does not exist yet". | "Influence [Action]" (p. 184): a Hesitant monster calls for an ability check, Charisma (Deception, Intimidation, Performance or Persuasion) or Wisdom (Animal Handling), "default DC equal to 15 or the monster's Intelligence score, whichever is higher", Advantage if Friendly, Disadvantage if Hostile; on a failure the same urging waits 24 hours. | Open: when a hero urges a character to do something it is hesitant about, roll it through the existing module, show the roll on the phone and the TV like any other, and let the result decide; keep the trust counter only for tone |
| RUL-20 | 3 | An "observe" choice in the story advances it without any check and records nothing about what was seen (HNT-04). | Search action: "Wisdom (Perception)" to notice something; Study action: Intelligence checks; "Typical DCs": 10 easy, 15 medium, 20 hard. | Open: make it a Search or Study check with a stated DC, show the roll, and give different text for success and failure; see HNT-04 for why refusing the offer outright broke the content |
| RUL-21 | 4 | A hero's stabilising or helping options aside (RUL-13), the Help action to give an ally Advantage on an attack ("Help [Action]": assist an attack roll against an enemy within 5 feet) is not offered either, nor Hide, Ready, Search, Study or Utilize for anything but picking up a weapon. The fight's rules note does not say so. | The list of actions in "Combat": Attack, Dash, Disengage, Dodge, Help, Hide, Influence, Magic, Ready, Search, Study, Utilize. | Open, low: say which actions the game does not offer in the rules note players can read; Help (attack) is the one worth building |

Agrees with the book, seen in the log: attack bonus +5 (Strength 17, Proficiency Bonus +2) and +3 with a shortbow (Dexterity 13); greatsword 2d6+3; thrown javelin 1d6+3 at 25 ft.; a natural 20 doubling the dice (2d6+1 with the shortbow); Disadvantage on a ranged attack with an enemy within 5 feet (both dice shown, the lower used); Half Cover (+2 AC) from an ally in the line of fire; Graze dealing the Strength modifier on a miss, and killing with it; Savage Attacker's two rolls; Second Wind 1d10+1, two uses, one back after a Short Rest; Armor Class 17 (chain mail 16, Defense +1); 13 hit points (d10, Constitution +2, Dwarven Toughness +1); the giant weasel's stat block (AC 13, 9 hit points, Speed 40 ft., Bite +5 for 1d4+3); moving through an ally's square but not ending there; a blocked square refused; enemies rolling one Initiative as a group and a tie going to the hero by the table's stated rule.

Also agrees, from ten short test fights (two of each creature, logs kept in `artifacts/logs/rules/`) and an archer played from a phone in room FERUFA: skeleton (AC 14 not tested, 13 hit points, Shortbow +5 for 1d6+3 at 80/320); zombie (15 hit points, Speed 20 ft., Slam +3 for 1d8+1, a natural 20 rolling 2d8+1); goblin warrior (10 hit points, Shortbow +4 for 1d6+2; but see RUL-17); kobold warrior (7 hit points, Dagger +4 for 1d4+2, Pack Tactics and Sunlight Sensitivity cancelling to a single die); scout (16 hit points, Multiattack with two Longbow attacks at +4 for 1d8+2); bandit (11 hit points, Light Crossbow +3 for 1d8+1); guard (Spear +3 for 1d6+1); cultist (Ritual Sickle +3, 1d4+1 Slashing plus 1 Necrotic: the log line shows only the Slashing part, the hit points fall by the full amount); giant rat (7 hit points, Bite +5 for 1d4+3, Pack Tactics); wolf (11 hit points, Bite +4 for 1d6+2, Pack Tactics with an ally within 5 feet); the death saving throw at the start of a dying hero's turn (10 or more succeeds); attacks against a creature that took Dodge having Disadvantage; the archer's sheet (AC 15 from studded leather and Dexterity 17; 61 GP from Fighter B and Soldier B) and her Longbow at +7 (Dexterity +3, Proficiency +2, Archery +2) for 1d8+3 with Savage Attacker's two rolls; the Longbow's Slow mastery (the weasel hit by it moved with 30 ft. of Speed and the phone listed it as "slowed"); the Light property's extra attack (Scimitar then Shortsword, both at +5 with Dexterity, the second spending the Bonus Action because the Nick weapon made the first attack, not the extra one).

## 23. Dialogue, investigation and notes, played 2026-10-13 (room BOBOPE, story model on)

The owner asked for attention on talking to characters, investigating and note taking. Ten typed lines to the patron and one choice, with the story model, the approach model and pictures on; the whole transcript is in `artifacts/tmp/campaign/luna6/V-evidence.txt`.

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| TLK-01 | 2 | The patron cannot answer plain questions about his own errand. "What happens at dawn if Tomas is not warned?" got "Please warn Tomas before dawn." "Who is after Tomas, and why?" got the same line plus where Tomas works. "What exactly does the letter say?" got "it weighs more than it looks". After ten lines the table still did not know what the letter warns of, what happens at dawn, who the villain is or what he wants. | DLG-02, STO-01, STO-02 | Open: every character needs answers written for the questions a table asks (what, who, why, where, when, what is at stake, what is in it for us), from the adventure's own facts; "I do not know" said plainly when it does not |
| TLK-02 | 2 | Two replies of ten were thrown out by the reply validator and replaced with stock lines that ignore what was asked (one was the character's biography: "I fed a ship's crew for six years"). The server log says only `NPC REPLY fallback reason=reply_rejected`: not which rule rejected it, nor the text. | DLG-02 | Open: log the rule and the offending words; on a rejection ask the model once more with the reason; a fallback must be chosen for the question's subject and never be the line just spoken |
| TLK-03 | 3 | The same sentence comes back again and again: "If the letter never reaches him, we may lose our only chance" was said four times in ten replies, twice as the whole reply. | | Open: a reply may not repeat a sentence the character has already said in this scene |
| TLK-04 | 3 | A threat changed the patron's attitude to Hostile and the table log said "Jorun cools toward Brakka", but his words did not change at all (the opening line again), and one choice later he whispered a private clue to that same hero "with old kindness". | DLG-01, RUL-19 | Open: the reply after a threat answers the threat; a Hostile character does not hand that hero a private clue |
| TLK-05 | 3 | A lie ("The harbourmaster sent us; you are to tell us everything") was labelled a plain question, not deception. Asking about pay was labelled a question and got no answer: the errand has no reward at all. | RUL-19, STO-02 | Open: seen again (TLK-14) |
| TLK-06 | 3 | The phone and the TV keep showing the scene's opening line as the character's speech through a whole conversation; replies appear only in the table log. | | Part: sampled on the running TV (room POZOPO), the hero's line and then the character's reply are each shown as they are spoken, and the reply stays. What is left: after a reload of the page, and in the view a phone is sent (`journey.dialogue`), the scene's opening line is back |
| TLK-07 | 2 | Characters are not told what has happened in the game. After a lost fight in which a hero died, the contact said "I only heard the letter was lost on the road, not that bandits took it" and, to a hero grieving his companion, "I can't confirm Sigrun died for the letter." | TLK-01 | Open: what the table has played through (the fight and how it ended, who fell, what was taken, what was promised) is given to every later reply as facts the character has just been told or can see |
| TLK-08 | 3 | The villain has no stable identity in what characters say: the patron fears him ("Whatever Kelbin Underhill wants with it, it won't ask twice") and the contact says "Kelbin Underhill owes me a drink, and I owe them one back". Nobody can say who he is or what he wants. | TLK-01, STO-01 | Open: one set of facts about the antagonist (who, what they want, how each character stands toward them), used by every character |
| INV-01 | 2 | Investigating does nothing. "I look around the ferry office: the desk, the ledger, the door. Is anything out of place?" and "I watch Jorun's face. Is he hiding something?" were accepted as "plan-only" and nothing came back: no narrator, no roll, no clue. No choice on any screen offers to look, search or listen either. | RUL-20, HNT-04, FLO-03 | Open: seen again in thirteen forms on the combined build (INV-03). A worker's helpers exist and are not called |
| INV-02 | 2 | There is nothing to find. Scenes carry no clues beyond the one private line a choice hands to one hero; the story before the fight is two screens (FLO-02), so nothing learned can change anything. | FLO-02, FLO-03 | Open: the adventure generator writes, per scene, two or three findable facts with what finds them, and at least one changes a later choice (a safer way in, a name to use, a weakness of the enemy) |
| NTE-01 | 2 | The game keeps no notes. "Can we write down what we know so far?" was accepted and ignored. What the table can look at afterwards is the table log (every line, in order) and, on one phone, one private clue while that scene lasts. Names, places and tasks heard are not collected anywhere. | | Part `30cbf359`: the notebook exists: kept by the server, stored with the run, a page on the phone, a recap on the TV's end card, notes added from the phone. Seen on a phone. What is wrong with it is NTE-03 to NTE-06 |
| NTE-02 | 3 | A private clue is shown to one phone only while that scene lasts, then it is gone; the hero's player cannot read it again or pass it on except by remembering it. | NTE-01 | Fixed `30cbf359`: a private clue stays in the finder's notebook and can be shared with the party. Seen on a phone |
| DTH-01 | 3 | A lost fight, played to its end (room BOBOPE: one hero died on her third failed death saving throw, the other became Stable and woke with 1 hit point after 3 hours, both by the book). While it happens the phone shows only "Dying" and one control, "Hold on while the fallen fight for breath (death saves)", pressed once a round; the roll, and how many successes and failures each hero has, are not shown until the scene after. | RUL-16 | Open: show each death saving throw as a roll, and the tally beside the hero ("2 successes, 1 failure") on the phones and the TV |
| DTH-02 | 3 | The story text after the defeat carries a raw rules label: "RULES:The party has been defeated. Heroes at 0 HP follow timed death saves; defeat does not restore hit points or held weapons." | RUL-09 | Open: rules notes go where the fight's rules note lives; the story stays story |
| DTH-03 | 2 | A dead hero's player has nothing of their own. The phone of the hero who died still shows her sheet at 0 hit points and offers the party's choices ("Short Rest", "Face what comes next") and the speech composer, as if she were alive; nothing says "your hero has died" or offers a way back into the game. | | Open (decision: the player of a dead hero is told so plainly, may make a new hero who joins at the next story scene, and until then watches; a dead hero cannot speak or choose) |
| DEV-07 | 4 | The server log printed `COMBAT fight-end outcome=victory rounds=6` for that total defeat, in round 7. The story and the phones told the defeat correctly. | | Open: seen again on the combined build (`COMBAT fight-end outcome=victory rounds=5` for a defeat); the enemy-turn worker did not reach it |
| DEV-08 | 4 | The development tool has no action name for the aftermath's choices ("Hold on while the fallen fight for breath", "Wait with the wounded until they stir", "Close the chapter"): a scripted game cannot get past them. | RUL-11 | Open: seen again on the combined build; the aftermath had to be played from a phone page |

## 24. Second round on the combined build, 2026-10-13 evening (room NOTEFO, story model on)

Played on the play server built from the working tree with every worker's uncommitted change in it (dialogue, notebook, enemy-turn timing, Short Rest dice), two heroes against three bandits with light crossbows, lost to the last hero and played through to "The End". The TV recording is in `artifacts/tmp/campaign/luna6/S07-evidence.txt`. Seen working: characters answer the question asked, a bargain got "I'll pay 12 gold", a threat is answered in words, the notebook opens on the phone with people, places, the task and clues, each enemy blow is shown in turn, death saving throws are rolled by the book (a natural 1 counted as two failures; three failures and the hero died), the story after a total defeat says nobody came back, and "Start a new adventure" is offered at The End.

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| SYN-13 | 2 | The TV's health cards revert. A card changes when its blow lands (Bandit 1 11 to 2), goes BACK to the old value about a second later, and takes the right value again 8 to 26 seconds later; a killed enemy was shown at 2 hit points again, and Brakka went 13, 4, 13, 4. This is the owner's first complaint of the day ("health healing then immediately resetting"), fixed and recorded clean in `1db1e80e` and `8645c254`, and back since today's changes to the director and the presented cursor (`45ea9a22`, `ca629d00`, and the uncommitted third pass). | SYN-04 | Open: the card is projected to the presented sequence; find which sequence the view sync uses just after an impact, with a test that replays this recording |
| SYN-14 | 3 | After the enemies' phase the headings of two enemies are shown once more ("Bandit 2", "Bandit 3", about 1.2 s each) before the hero's heading, although the third pass on this was in the build. | SYN-01 | Open, with SYN-13 |
| NTE-03 | 3 | The table's chat list shows the notebook's own log lines to players: "DM table: NOTE added kind=place title="the Lantern-Maker's Shop" scene=inn", "NOTE added kind=clue title="Private clue" scene=defeat". | NTE-01 | Open: a note is announced in words ("Added to the notebook: the Lantern-Maker's Shop") or not at all; the log line stays in the server log |
| NTE-04 | 3 | The notebook's entries are untidy: the same private clue is stored once per scene (four copies by the end, "Scene: inn", "Scene: defeat", "Scene: combat"); every entry ends with a raw scene id ("Scene: dialogue-seal", "Scene: opening"); the task read "Status: done" after the whole party died with the errand failed; a place reads "the Lamplit Market: the market" and another "the market: You leave your meeting place for the market." | NTE-01 | Open: one entry per fact, the scene given by its title, the task's status from how the errand ended (given, done, failed) |
| NTE-05 | 3 | The notebook page's "Close" bar sits on top of the scene title behind it, and the page lists a dead hero's private clue with "Share with the party". The page has had no design pass. | NTE-01 | Part `30cbf359`: the sheet fills the screen under its Close bar on a solid backdrop, the task's status has its own line, raw scene keys are not printed, the note box empties when the note is sent. Seen on a 375 px phone. A real design pass and the TV recap remain |
| TLK-09 | 3 | The patron contradicts herself about pay: asked what the party gets, she said "I can promise you nothing"; pressed, "I'll pay 12 gold"; and the notebook's task says the reward is that she will "send word ahead that you are friends". | TLK-01, STO-01 | Open: one reward fact per adventure, known to the character from the start; bargaining may raise it by a stated rule |
| TLK-10 | 3 | A patron made Hostile to a hero by that hero's threat still drew that same hero close and whispered the private clue ("Only you and Nolan know the truth") when the party took the "ask" choice. | TLK-04 | Open: the clue goes to a hero the character is not Hostile to, or is withheld with a line saying why |
| DTH-04 | 3 | A dead hero keeps acting in the record: "Brakka ends the turn" (on the TV as a banner over the narrator's box, on the phones in the chat list), "Brakka faces what comes next", "Brakka closes the chapter"; the TV and the sheet label a dead hero "DOWN"; the dead hero's phone header still offers "Second Wind · 2 remaining". With both heroes dead the table is still asked "What do you do?". | DTH-03 | Open, with DTH-03: a dead hero is labelled dead and takes no turn; when every hero is dead the story goes to its end without asking |
| DTH-05 | 3 | Death saving throws are now listed in the chat ("makes a Death Saving Throw: 1, a natural 1, two failures (0 successes, 2 failures)"), but only on the scene after: while they are rolled the phone still shows "Dying" and nothing else. | DTH-01 | Open, with DTH-01 |
| PHN-10 | 4 | On taking a hero back, the phone's composer says "The input scope changed. Your text remains editable; choose a current offer." with nothing typed. | | Open: say nothing when the box is empty; plain words when it is not |
| FGT-10 | 4 | Attack wording is the wrong way round: a thrown javelin "shoots the javelin", a crossbow bolt "strikes Sigrun". | | Open: thrown weapons are thrown, shot weapons shoot, melee strikes |

## 25. Dialogue and investigation in depth, and what the owner asked while watching, 2026-10-13 late evening

Forty-four typed lines before any fight (room KUTUKU, "Ulric's Urgent Warning", story model on), in two scenes; every exchange with its fault is in `artifacts/tmp/campaign/luna6/V2-evidence.txt`, the raw output beside it. Also two more full games on the same build ("The Stolen Whistle", won; a second adventure started from The End). Seen working: who and where are answered directly; the reward is one figure and stays the same when asked twice (14 gp); a persuading line got the enemy's name the character had held back; a threat made him Hostile to the hero who threatened and not to the other; the private clue went to the hero he was not Hostile to and was a real clue (a ring with a green stone); an apology and a bribe got lines in character; a won fight's rolls agree with the kobold warrior's stat block (dagger +4, 1d4+2, Pack Tactics, critical hit doubling the die).

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| INV-03 | 2 | Thirteen of thirteen lines in which a hero looked, searched, listened, watched a face, scouted, tracked, studied a thing or tried to recall something did nothing at all: disposition "plan-only", no narrator, no roll, no finding, no word that the game cannot do it. The book has an action for each: Search (Insight, Medicine, Perception, Survival) and Study (Arcana, History, Investigation, Nature, Religion), and Stealth and passive Perception decide surprise before a fight. The helpers a worker wrote for this (`resolve_skill_check`, `passive_perception`, findings in the pack) are in the tree and never called. | INV-01, INV-02, RUL-20 | Part `6299d7f2`: Jev reads the line first (local rules as fallback) and the server rolls the book's Search or Study check; findings reach the notebook; a phone has "Look around". Played: see INV-04 to INV-09 |
| RUL-22 | 2 | No roll is ever made in a conversation. Persuasion, a bargain, a lie, a threat, an apology and a bribe were each decided by a model's label alone. The book's Influence action: no check when the character is willing or unwilling; when hesitant, a Charisma check (Deception, Intimidation, Performance, Persuasion) against DC 15 or the character's Intelligence score, with Advantage if Friendly and Disadvantage if Hostile; a failed urging cannot be repeated for 24 hours. | RUL-19 | Open, with the dialogue work |
| TLK-11 | 3 | A line said by one hero to the other is answered by the character: "Brakka, do you trust him?" got "You don't trust him, then" from Ulric, and "What did he whisper to you, Sigrun?" got "I won't share what was whispered to me" from Ulric, as if he were her. | TLK-01 | Open: a line addressed to another hero by name goes to the table log only (the character may react if it concerns them, by rule, not by default) |
| TLK-12 | 3 | Stock biography lines still come out as answers: "I know you're Sigrun. I'm promised to Gisela, and I have no children."; "I trust a dog's welcome more than a priest's. I was raised under a market stall, learning to haggle before I could read." (to "is that true, a ring with a green stone?"); and after a joke, "I would not ask this of you if there were anyone else I could trust with this." | TLK-01 | Open: a biography line is used only when the hero asks about the character's own life |
| TLK-13 | 3 | Answers drawn from the wrong subject or another adventure's template: "What is in the map bundle?" got "I haven't opened the sealed letter" (there is no letter in this adventure); "Show me the map bundle" got "I'm at the Reedmoor Boardwalk; Osric waits at the Ferryman's Hut"; "What is the Barrow-Cold Sentinel?" got "It's the Barrow-Cold Sentinel. It wants to stop this errand." | TLK-08, STO-01 | Open: the errand's object and the enemy each have facts a character can state (what it is, what it looks like, what is known and rumoured) |
| TLK-14 | 3 | The classifier mislabels plain cases: a lie ("the watch captain told us the plan") is "persuasion"; "I am Sigrun. We will carry your warning" is "deception"; an apology is "other". | TLK-05 | Open |
| TLK-15 | 3 | The game changes the player's words: "We will carry your warning" is shown in the table log as "your Warning", "who took the whistle" as "the Whistle". | | Open: a hero's line is shown as typed |
| TLK-16 | 3 | A bribe is accepted and nothing changes hands: "Here, take five gold" got "I'll take it" and the sheet still reads 18 gp. An apology and a payment together left the character Hostile with no roll and no word of what would mend it. | RUL-22 | Open: gold given is deducted and recorded; mending an attitude is an Influence check |
| TLK-17 | 2 | Characters never tire. A Hostile man answered the same question politely four times running, and forty-four lines in a row cost nothing. Owner, watching: "after a while npcs should tire of questions and their moods should sour after a variable while until they stop responding to you, and that's based on their emotional state". | TLK-04 | Open (decision: each character has a patience that starts from temperament and attitude (Friendly most, Hostile least, with a seeded variation), is spent by each line and faster by a repeated or pointless one, is restored by a line that helps them; as it runs down the replies shorten and the expression sours, the players are told in words ("Ulric is losing patience"), and at zero the character says so once and stops answering that hero until the scene changes) |
| TLK-18 | 3 | The errand cannot be refused ("No. We will not carry it" got "No. That is still the errand."), and asking the game what has been noted ("What have we written down so far?") did nothing. | NTE-01 | Open: a refusal is acknowledged and the scene's choices stay; a notebook question opens the notebook or reads back the party's notes |
| TLK-19 | 3 | A patron can start Hostile to the heroes she is asking for help (1 in 20 by the table in `content/adventure-tables/npc-social.json`), and then speaks against her own errand: "Your claim proves nothing" (it is her claim), "Words are cheap". | TLK-04 | Open (decision: a patron who offers the errand starts Friendly or Indifferent; a Hostile start is for contacts and antagonists) |
| TLK-20 | 4 | A character speaks the narrator's text in the first person: "I have the whistle, and you all stand before me. Salt wind stirs the hanging nets as I count the cost in silence." | STO-01 | Open |
| TLK-21 | 3 | The TV shows a change of attitude as the character's speech: five seconds after her reply, Gwenna's speech box read "softens a little toward Sigrun" under her name, replacing what she had said. (Sampled on the running TV, room POZOPO.) | TLK-04 | Open: an attitude change is a table note with its own look, not a line in the speaker's box; the last thing the character said stays |
| PHN-11 | 2 | From a real phone an attempt to look around is sent to the character as speech. The composer has a list "Just saying it / As a question / In conversation / As a plan / About the game / Joke" (default "Just saying it") and a second list "Choose an offered action (optional)"; a player who types "I look around the bell chamber for anything odd." and presses "Say it" got Gwenna's "Dogs judge folk better than priests do. I trust a dog's welcome." and, under it, "Only spoken words: no action or check was made." Owner: "can we use jev to classify this text?" | INV-03 | Part `6299d7f2`: an attempt is recognised whatever context the phone sends. The composer's lists are still there (to check on a phone) |
| RWD-01 | 2 | A won adventure pays nothing. Owner, at the end of "The Stolen Whistle": "how comes we didn't get a reward?" The patron names a price when asked (14 gp) or a favour, and the ending gives no gold, no item and no experience; nothing on either screen says what the party earned. | STO-01, TLK-09 | Open (decision: the reward named in the opening is paid at the ending, each hero's gold is raised and the ending card lists it; experience points for the defeated creatures by the book's table are shown beside it) |
| PIC-13 | 2 | A named character who is in the scene but is not the speaker is left out of the picture, while invented extras are painted in. Owner: "why wasn't the woman in the display plate?" The ending's text has Petra standing with the party; the picture's prompt lists the two heroes, the speaker (Tomas), "a journeyman sweeping shavings" and "a customer examining a lantern". | PIC-01 | Open: every named character the scene's text places there is in the picture with their portrait as a reference |
| TVX-09 | 3 | The screen shown while a new adventure is prepared is black with one line of text. Owner: "a new adventure is being prepared is a black ugly screen, fill it with art/graphics". | | Fixed `4b4c8dcd`: a drawn night scene (moonlit range, lit tower, embers, a d20 in a turning compass ring, the title and four changing lines) on the TV and the phones, and on the holding page. Seen in a browser on a real change of adventure, TV and 375 px phone |
| RUL-23 | 2 | After a won fight every hero is offered "Short Rest · 1 hour, no Hit Point Dice", a hero at 3 of 13 hit points with a d10 to spend included, and pressing it is refused (`REFUSAL action=ShortRest reason=InvalidCandidate step=step-transition-missing`). Seen on the test fight and on a full game. The rest worked before today's unfinished rest work. | RUL-08, RUL-09 | Open, and worse on `30cbf359`: the rest now starts ("Brakka takes a short rest") and then the game cannot continue. The only offer is "Choose whether to spend your Hit Point Die" with nothing to choose with (`step=short-rest-choice-shape`), the phone's button does nothing, the other hero has no offer, and the story's own choice is gone. Worker U14, after the two conversation workers |
| RUL-24 | 3 | Kobold warriors attacked with Disadvantage from Sunlight Sensitivity in a scene the story had just set at night ("Lanterns sway above black, slow water"). The trait applies only in sunlight. | | Open: the fight takes day or night from the story's scene, and says which |
| SES-16 | 2 | A game stopped dead. In a second adventure started from The End (the first was lost with both heroes dead), the choice that starts the fight was accepted and from then on every request, the TV's and both phones' included, was refused with `UNAVAILABLE site=authority reason=InvalidCandidate step=unknown`; the game could not be continued. Tried three more times (same seed on a fresh server; a second adventure after a won one, twice) and it did not happen again. | SCA-10 | Part `83ea51fd` and uncommitted step names in the view projection: the next occurrence can be read from the log. The cause is not known; three attempts to repeat it failed |
| DEV-09 | 4 | `WATCH failed code=ResourceExhausted reason=gameplay actor queue full` was logged twice while a new adventure was starting. | | Open |
| END-06 | 3 | The TV's "The End" screen is cluttered: a notebook panel ("PEOPLE") sits on the right with its own scroll bar that nobody at a TV can scroll, the narrator's box is drawn under the health cards with its text showing through them, and a one-line text field with a caret and a tiny scroll bar ("The road is quiet behind you.") floats over the lower left. Seen on the keyless server, room VESADO. | END-03, NTE-05 | Open: the coordinator's design work, with the notebook page |
| NTE-06 | 3 | The notebook lists creatures as people, twice: "Shaggy Giant Weasel: antagonist; found at the Windward Pass" and "The Shaggy Giant Weasel: the giant weasel; found at the Windward Pass". | NTE-04 | Open: a beast is not a person; one entry per thing |
| SCA-12 | 3 | Any rebuild that touches a rules source file ends every game in progress: the server refuses the stored game at startup (`CHECKPOINT rules pin differs: handler_digest`), because the stored document is pinned to a digest of twenty-six source files. Lost three test games today. Right for a release, costly in development. | SCA-10 | Open (decision: keep the pin; in a development build the refusal offers the way out in its own message, "start a new game with DF_GAMEPLAY_DEMO_DATABASE=<new name>", and the play scripts do it) |

## 26. Anything and anyone in the scene, 2026-10-13 night (room DOKENU, story model on)

The owner, watching: "focus on the chat and investigation deeply", "the player should be able to interact with anything and anyone in the environment", "and if it doesn't work then develop these features, and refine them until they do". Twenty lines in a taproom whose picture shows the innkeeper at the bar, two carters at dice, a serving woman, tankards, herbs on the beams, a hearth, a clock and a window; the transcript is `artifacts/tmp/campaign/luna6/V3-evidence.txt`. Result: the scene has one thing in it that can be interacted with, the speaking character.

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| ENV-01 | 2 | Nobody but the scene's speaker can be spoken to. Five lines to the carters and the serving woman who are painted in the picture were each answered by the innkeeper, in her own voice, on their behalf ("Carters, do you know a man called Tobin?" got "Tobin, you mean? He is the contact at the Binding Workshop"; "Serving woman, is Ilsa a good mistress?" got "I can't judge a mistress from where I stand"). | TLK-11 | Open (decision: every scene has its people, the named cast and two or three background people with a name, a look, an attitude, a few things they know and one they want; a line aimed at a person is answered by that person under their own name. Worker V06, after the investigation worker) |
| ENV-02 | 2 | Nothing in the scene can be acted on. Eleven lines that picked up, smelled, looked at, went behind, opened, paid for, sat at or stole something painted in the picture did nothing at all. | INV-03 | Open (decision: every scene has its things and its ways out, each with a look, what a closer look shows, whether it can be taken and what using it does; look, take, use, give, buy and join in are server actions with a shown result; a line about something that is not there gets a plain answer. Worker V06) |
| ENV-03 | 3 | An innkeeper cannot sell. "A bowl of stew and a bed for the night. What does that cost?" got "I can pay you 14 GP" (the reward for the errand). There are no prices, no purchases, and coin never leaves a purse. | TLK-16, RWD-01 | Open: the book's "Food, Drink, and Lodging" prices where somebody present sells; coin is deducted. Worker V06 |
| ENV-04 | 3 | Heroes cannot deal with each other: "Sigrun, lend me five gold for the dice" was answered by the innkeeper, and nothing can be given from one sheet to another. | TLK-11 | Open: give is an action between heroes too. Worker V06 |
| ENV-05 | 3 | The people the story is about have no facts. "Where is the sick child now? Can we see them?" got "Tobin waits at the Binding Workshop"; "What sort of man is Tobin?" got "Tobin is the contact at the Binding Workshop, known to me by that connection alone." Characters speak of each other in the game's own words ("the contact", "the patron"). | TLK-13, STO-01 | Open: every person an adventure names has a few stated facts another character can tell (who they are to the speaker, what they are like, where they are and why); the words "the contact" and "the patron" never reach a player. With the dialogue worker |
| ENV-06 | 3 | The picture and the game disagree about what is in the room: the picture's code invents its own background people and foreground objects ("a serving woman carrying a tray of mugs", "a pair of carters arguing softly over dice", "a dented pewter tankard") that the game knows nothing about. | PIC-13 | Open: one scene model, used by the picture's prompt and by the game. Worker V06 |
| ENV-07 | 2 | The scene has no state. Owner: "I want the entire scene to have state, meaning if the user sees an object and tries to interact with it, the interaction should persist, drive the narrative and the display plate generation". Nothing a hero does to a thing is remembered, told to the storyteller or the characters, or shown in the picture. | ENV-02 | Open (decision: every thing and person of the scene model has state kept for the adventure; "the scene as it is now" is given to every narration, reply and choice; a visible change redraws the picture from the current still. Workers V06 (state, narration) and V07 (pictures)) |
| ENV-08 | 2 | Things are not generated in detail or presented, and there are no clues to find. Owner: "I want the item to be generated in detail and presented to the user, and for example if it's story critical I want if feasible hints/clues/evidence left for the users to observe, all driven by clever prompting". | INV-02 | Open (decision: each thing has a detailed description, a hidden detail with its skill and DC, and a close-up picture shown on an item card; every fact the party needs has at least two observable pieces of evidence placed before it is needed, by a structured-output prompt to the storyteller that the server validates, with the tables as the fallback. Workers V06 and V07) |
| ENV-09 | 2 | A player cannot point at what they see. Owner, at the taproom picture: "questions like who are those men in the back or I want a pint of beer or whose mug is that should all be legal". | ENV-01, ENV-03 | Open (decision: every person and thing has a place in the picture in a player's words and an owner; lines are resolved by place and pointing words as well as names; the owner's three lines are acceptance tests. Worker V06) |
| EMO-01 | 2 | Characters and enemies have no emotional life: one attitude per hero on the book's three-step scale, moved one step by five fixed reactions (flattery, flirting, threats, lies, bribes). Owner: "lets work on the npc/baddie emotional graph, model their attraction/disgust to certain words, visual or actions as a part of their character generation, for example ilsa is a lesbian and has an attraction to female player characters, or ilsa is a bit sensitive so threats really scare her ... make sure it can drive the narrative, also some people are forgiving and some aren't so lets also model that in so the player can permanently sour relations with the npcs"; "for the love of god use jev and llms to enable that system as I don't want gigantic rust algorithms"; "also tie it to the npc background dossier generator, tie them in closely". | TLK-04, TLK-17, RUL-22 | Open (decision: an emotional profile written by the story model as part of the dossier, each feeling grounded in a cited dossier entry; a small state in words per character per hero; Jev appraises each moment from a fixed list of reactions; patience, permanent grudges for unforgiving characters, the book's Influence check; states drive replies, offers, expressions and an enemy's flight or surrender; thin Rust. Worker E01, Sol) |
| MOT-01 | 2 | Scenes do not move. Owner: "even though the scenes are updating, the player characters never move and nor does the background, the only thing that moves is the npc facial expression"; "with those hard timings and deps I still want more motion and dynamism in the scenes". Measured tonight: a scene picture takes about 19 s ($0.06), a plate edit 18 s ($0.04), an expression cut-out 13 s; the TV's only camera move was a 3.6 % push over 150 s. | MOT-02 | Part `f43d3fa7`: a living camera on the TV (a continuous slow drift of the picture, a push toward the speaker's head while a line is spoken and a slow ease back, flickering firelight and lantern glows), checked on the running TV. Still to come, in order: MOT-02 (moving pictures), then puppets and state snapshots with the scene model (V06, V07) |
| MOT-02 | 2 | Nobody in a scene picture moves. | MOT-01 | Open (decision: every scene picture also becomes a 5-second looping video of ambient life (people breathe, shift and go on with what they are doing, the fire moves) by the image-to-video provider the lobby already uses, faded in over the still and looped without a jump. Worker M01, after the emotional model) |
| MOT-03 | 2 | The picture cannot show what the players do or feel. Owner: "I want the display plate to be an every 10 second snapshot of the game state (json prompting) so we can see emotions on all the characters faces and have them interact with the scene and move around". | ENV-07, EMO-01 | Open, planned `3002c60b` (planning/asset-engine.md, "Scene rendering engine v2"): a scene graph sent with the display view, drawn by the TV in layers (set or its video loop, cut-out actors at spots scaled by depth with shadows, relighting, idle motion and walking, props, atmosphere, camera rig with director shots) and baked into a photographic frame from the same JSON when settled, at most every 10 s. Workers R01 (graph and stage), then E01, M01, V06, V07 |

## 27. Investigation after the first fix, 2026-10-14 night (room GEGAGA, story model and Jev on)

The owner: "rebuild and test scene investigation, do a series of tests to check how flexible it is"; then, watching: "on the display I don't see the success or failure status of the investigation". Thirty-two lines in the taproom opening, mostly sent as plain speech; the transcript and the server lines are `artifacts/tmp/campaign/luna6/V4-evidence.txt`. Jev now reads almost every line as an attempt and mostly chooses the right skill, and the server rolls the book's check. What is wrong:

| ID | Sev | Issue | Relates to | Decision |
|---|---|---|---|---|
| INV-04 | 2 | Twelve of thirty-two lines were refused with an error to the player ("observation was refused", "name the person you want to watch"): the "already tried" guard keys on the target, the target is empty for most lines, so after one "I look around" every general look is refused; "I study her face" does not resolve "her" to the speaker; "the people at the back table" are not resolved to anyone. | INV-03 | Open (decision: never refuse an attempt with an error; a repeat is answered in words ("You have already searched the room; nothing new"), a different target is a new attempt, pronouns and descriptions resolve to the scene's people and things. Worker V08, Sol) |
| INV-05 | 2 | A result is not tied to what was examined: looking around, at the moon or behind the counter all reveal the same finding (the rats at the forecourt); asking to see a letter that does not exist reveals the errand's hidden detail. | INV-02, ENV-02 | Open (decision: a finding is found ON a place, thing or person; an attempt can reveal only findings on its target, a general look only those on the room; V08) |
| INV-06 | 2 | Nothing is described. Success or failure, the answer is "You find nothing unusual" (the hearth, a lock, tracks on a success, her breathing, the herbs, magic on a success), or the scene's title and narration pasted back. Things not in the scene are not said to be absent ("the notice pinned to the door"). | ENV-08 | Open (decision: every attempt gets a sentence about its target written by the story model from the scene's facts, honest about success (what is noticed) and failure (what is not), and plain about absence; V08) |
| INV-07 | 2 | The display shows no success or failure, and the roll is drawn as the hero's speech: "BRAKKA: Brakka searches behind the counter: Wisdom (Perception) 16 + 3 = 19." Owner: "on the display I don't see the success or failure status of the investigation". | TVX-05, RUL-16 | Open (decision: a check shows as a roll, not speech: skill, d20, modifier, total and the outcome in words ("success: you notice ..." / "nothing found"), on the TV and the roller's phone; the DC stays hidden; V08 with the TV owner) |
| INV-08 | 3 | The extracted target and the roll line are mangled: "Brakka searches look around", "Sigrun scouts brakka s pack while he isn t looking", "studies this place s history" (lower-cased, apostrophes stripped, the player's words pasted in). | INV-03 | Open (V08) |
| INV-09 | 3 | Wrong skills: listening at a door rolled Stealth (it is Perception); rummaging another hero's pack unseen rolled Stealth (it is Sleight of Hand against his passive Perception, and it is an act on another hero). | INV-03 | Open (V08) |
| ENV-10 | 2 | The speaker does not know her own room. Owner: "you didn't ask the lady about the environment, like who is that lady in the background (her coworker)". Asked "who is that lady in the background with the tray?", "who are those two men at the back table?", "does she work here with you?", "whose mug is that on the bar?", "is it safe to talk in here, with the others listening?", Ilsa answered "Tobin waits at the Binding Workshop" (twice), "I couldn't say whose mug that is", "Ale should be dark and the night darker", and "I was raised on a river barge" to "how long have you kept this inn?". Transcript `artifacts/tmp/campaign/luna6/V5-raw-room.txt`. | ENV-01, ENV-06 | Open (decision: the scene model is made first and the picture designed from it with intent; every character is given the scene as what they know of the room. Worker V06) |
| ENV-11 | 3 | A true answer about the room is thrown away: the model's "I don't know who the lady with the tray is" was rejected by a reply rule (`NPC REPLY rejected rule=spoken_rule`) and replaced by the stock "I'm Ilsa the patron at the Orrery Library. Tobin is the contact at the Binding Workshop", which speaks the game's internal words. | TLK-12, ENV-05 | Open (V06 with the reply rules' owner, E01) |
| TLK-22 | 3 | Brakka, a man, walked to the woman with the tray and asked her name; Ilsa answered instead, and called him "miss". | ENV-01 | Open (V06: the addressed person answers; E01: the reply knows who is speaking) |
| INV-10 | 3 | "I pour myself an ale from the jug" rolled Intelligence (Nature); "I pick up the pewter tankard", "I take a candle", "I look closely at the bread" were refused with an error; examining the herbs (a success) did not say whether the white fever-herb is among them. | INV-04, ENV-02 | Open (V08 for attempts, V06 for taking and using) |
| RUL-25 | 1 | The game dead-ends after a won fight. With Sigrun at 9 of 13, she took "Short Rest · 1 hour, no Hit Point Dice" (accepted); then Brakka had no choices at all and Sigrun only "Short Rest · 1 hour · 1 Hit Point Die to spend", which is refused (`REFUSAL action=ShortRest reason=InvalidCandidate step=short-rest-choice-shape`); "Follow Ilsa to the bindery" is refused for both. The ending cannot be reached. Played on `ae3f2ffe`. | RUL-23 | Open: worker Q01, first priority (the Hit Point Die choice group on the pending rest, the rested story; a hero can always decline the rest and go on) |
| TVX-10 | 2 | The TV is too fast to read and flickers. One question made three lines (the hero's, an attitude note, the reply), each up about 3 s, lingering 0.7 s, the next opening 28 ms after; an unvoiced line was held 1.8 s + 35 ms a character (2.5 to 7 s). Owner: "the scenes and chat are just flickering rapidly and the tts can't keep up", "the bubbles don't stay up long enough for a player to read them", "ffs have you not been play testing this as a human and measuring against human comfort?". | TVX-07 | Part (uncommitted, worker B24): lines linger 2.5 s, are held 2.5 s + 65 ms a character (4 to 14 s); measured after: a reply up 7.2 s plus 2.5 s. Queueing two speakers, the catch-up flash and the voice pacing remain with B24 |
| PIC-14 | 2 | The background is replaced every line and the figures vanish: 27 new scene plates were asked for one room in about ten lines, nothing in it changed (`IMAGERY request kind=scene_plate reason=place-inn-taproom state=0`); each has a new composition and the cut-out figures disappear under it. Owner: "changes background too often where all the characters disappear"; "queue up the next image without tearing down the current one and swap it smoothly when it's ready, it's ok to be a little late rather than flickering on and off". | MOT-03, PIC-13 | Part (uncommitted, worker B24): a plate only per scene and party; measured after: 3 requests at scene start, none per line. The off-screen load and crossfade swap remain with B24 |
| EMO-02 | 2 | Characters' feelings change too often and are shown as their speech. Two plain questions: "Ilsa seems fond of Brakka." and "softens a little toward Brakka" (twice) in Ilsa's bubble, and "Ilsa seems guarded toward Sigrun." although the server logged `reaction=unmoved`. A 7 s answer became a 36 s exchange. Owner: "the npcs emotional state to the players changes too frequently". | EMO-01 | Open: worker Q02 (feelings with inertia, a rare note only for a real change, never as speech) |
| INV-11 | 2 | A check is still drawn as the hero's speech ("BRAKKA: Brakka searches the taproom: Wisdom (Perception) 15 + 3 = 18.") with no success or failure, and its finding is unrelated to the room ("three giant rats waiting at the forecourt"). Owner: "perhaps the DM should be reading the pass or fail of a dice roll check". | INV-05, INV-07 | Open (decision: the Dungeon Master's voice reads who tried what, the total and the outcome, then what is found, while the TV shows the dice and SUCCESS or FAILURE. Workers Q01 and B22) |
| FGT-11 | 3 | The roll bar at the bottom centre of the TV in a fight ("Giant Rat 2 strikes Brakka · Bite · MISS · d20 7 + 5 = 12") flickers between two styles (owner). | OBS-42 | Open: worker B21 |
| FGT-12 | 2 | No Dungeon Master narration and no battle cries are heard in a fight (owner: "I don't hear any DM narration or any player character war cries or chatter during combat"). The fight-line tables and staging exist (`fight_lines.rs`, `live_motion.rs`); no voice request was logged during the fight. | FPX-05 | Open: worker B21 |
| FGT-13 | 3 | An opportunity attack stalls the fight: "Brakka: Opportunity Attack?" waited seven minutes with nothing saying whose decision it is on the TV; "Target · Brakka" is drawn over a hero; the attack offers only a javelin or an unarmed strike, not the greatsword in hand. Both heroes are drawn with the same bearded figure (Sigrun is a red-haired dwarf woman). | SYN-10, FPX-10 | Open: worker B21 |
| DEV-10 | 4 | Offers whose action is `unknown` ("Look around", "Opportunity attack", "Decline opportunity attack") cannot be named by the development tool or the logs. | DEV-08 | Open: worker Q01 |
| DEV-11 | 3 | The server log has no display timings: nothing records when a line opened, how long it was held, its voice clip, or when a plate, clip or figure appeared or changed, so a session's pace cannot be measured from the log. `AGENTS.md` now asks every worker to measure scene and interaction timing and to add missing timings to the logs. | TVX-10 | Open: worker B24 |
| PHN-12 | 3 | The phone shows a blank black page for 7.9 s on first load while 21.6 MB of browser code arrives; nothing says it is loading. | | Open: worker B18 (a light loading screen drawn before the code; the coordinator's release build is smaller) |
| CRE-08 | 2 | A player cannot define their hero. The creation page fixes Dwarf, Fighter, Soldier; its one free field ("A detail that makes you memorable") is hidden; there is no backstory, no appearance, no gender or pronouns, so every hero is drawn as the same bearded dwarf (Maren, a woman, has a bearded man's portrait). Owner (/goal 2026-10-14): "allow users to define their characters, their back stories and their looks, via menus or via text". | CRE-01, FGT-13 | Open: worker C01 (Sol) when B18 has finished |
| CRE-09 | 3 | Pressing "Confirm your hero" with choices unmade shows "Waiting for the game server..." then nothing: no word of which choices are missing. The page is 3,699 px tall on a phone. | CRE-01 | Open: C01 |
| PHN-13 | 3 | On the phone the send button "Say it" sits under the bottom tab bar (y 780 to 828 on an 812 px screen) and is a small secondary button under the large "Tap to talk", even when text is typed; the bottom navigation's buttons have no accessible names; an empty gap sits between the choices and "Say something else". | PHN-04 | Open: worker B18 |
| TVX-11 | 4 | The phone offers two choices where the TV shows three ("Look around" is on the TV only). | TVX-06 | Open: worker B22 |
| AUD-01 | 3 | The TV starts with sound off and only a small "Enable sound" button says so; pressing it once turned its label to "Sound is off. Press here to turn it on". With sound off the opening narration and the first speech appeared at the same moment, the bubble covering the narrator's box. | TVX-07, TVX-10 | Open: workers B22 (the control and its words), B24 (unvoiced lines are paced the same as voiced) |
| DLG-08 | 2 | The patron does not know his own errand: asked "who is Osric and why does this letter matter?", Halden (who asked the party to carry it to Osric) answered "I can't say who Osric is or why it matters." | DLG-04, OBS-23 | Open: worker Q02 (the patron's dossier holds the errand's who, what and why) |
| EMO-03 | 2 | A polite question made the patron "wary" (and the note was shown twice); one question took 33 s on the TV, a third of it reaction notes. | EMO-02 | Open: worker Q02 |
| PIC-15 | 3 | The first picture of a scene was an empty shop interior although the scene is an open market, and the speaking patron was not in it; the scene's own plate arrived later. A reaction note's bubble sits at the far left, away from the character. | PIC-14 | Open: worker B24 (the fallback still) and B25 (fallback art) |
| TXT-01 | 4 | The hero's typed line is shown with a word capitalised that the player did not ("this Letter"). | | Open: worker B18 (the phone's line) or Q01 (the server's tidy-up): whoever finds it |
| INV-12 | 2 | Searching a place the scene has under another name fails and still rolls: in the taproom (it has a bar) "I look behind the counter." answered "The counter is not here, and you see no counter in this space." after a Wisdom (Perception) check of 21. A target that is not there needs no roll, and a counter is the bar. Played on HEAD 43e3251c plus Q01's and Q02's uncommitted work. | INV-04, ENV-02 | Open: worker Q03 (target aliases from the scene model; no roll without a target) |
| TVX-12 | 2 | The TV's check card names the wrong hero and is not seen: Sigrun searched (`CHECK actor="Sigrun" ... total=21 outcome=success`) and the card's data reads "Brakka · Wisdom (Perception) · 21"; nothing about the search appeared on screen (no line, no visible card). | INV-07, INV-11 | Open: Q03 (the display check's hero), B26 (the card is shown, with the narrator line) |
| TVX-13 | 3 | A TV that joins or reloads during a scene shows only the picture and the choices: the opening narration and the current speaker's line are missing, so the table has no context. | TVX-07 | Open: worker B26 (settled lines shown at once on join) |
| SCN-04 | 3 | A purchase works (4 cp paid for ale) but reads like a log line: "Brakka pays 4 CP for ale (mug). a mug of ale"; nobody pours it or reacts, and no close-up is asked for. | ENV-07 | Open: worker Q03 (the Dungeon Master tells it and the seller acts; the item card) |
| TVX-14 | 4 | The heroes' health bars on the TV are yellow at full hit points (13/13). | | Open: worker B21 |
| ENV-12 | 4 | Better (good news, recorded): Ilsa now names the woman with the tray ("That is Nella, the serving woman by the bar on the left."); the scene lists its people (Ilsa, Tessa, Bram, Nella) and wares; the picture shows them where the model puts them; the background stayed the same through three turns. | ENV-10, PIC-14 | Seen on Q01 and Q02's uncommitted work: commit pending |
| FGT-14 | 3 | The phone's fight turn is a form, not a game: a paragraph of instructions ("Tap a lit square, then Move. Chosen: Column 2, row 4. Tap an enemy, then Attack. Aim: Javelin (throw) -> Giant Rat 1 · no cover · normal range: 20 ft."), three drop-downs ("Weapon and target", "Use the weapon's mastery when it has one: Yes", "Savage Attacker (once a turn): Yes"), End turn above Move, and Attack 1,340 px down a 3,676 px page. Played on HEAD 27d0e2f1 from a 375 px phone. | RUN-07, OBS-43, FGT-13 | Open: worker B28 (one primary action at a glance, no developer form) |
| STO-09 | 2 | The ending turns the errand round: the job was to fetch the fever-herb from Tobin for a sick child; the epilogue says "Tobin takes the fever-herb" and Tobin thanks the party ("You brought the fever-herb"); Ilsa says "You bled for me" after a fight in which nobody was hurt; "The innkeeper now looks to Brakka for support on the way" repeats from the fight into the epilogue. | STO-01, STO-05 | Open: worker B20 (the plot's facts drive the ending; no claim the fight does not support) |
| END-07 | 3 | The end card shows no reward and no experience after a won adventure, and is headed with the first scene's title ("Ilsa Seeks Your Help") instead of the adventure's ("The Fever-Herb Errand"). Better: the card is now one clean composition over the final picture. | RWD-01, END-06 | Open: workers Q03 (the paid reward and XP in the view), B26 (the card shows them and the adventure title) |
| PIC-16 | 3 | The epilogue picture shows Tobin twice: once painted into the plate and once as a cut-out figure standing in front of it. | PIC-14 | Open: worker T01 (a figure baked into the plate is not also drawn as a cut-out) |
| TVX-15 | 4 | Aftermath choices still differ: the phone reads "Follow Ilsa to the bindery · the Binding Workshop", the TV "Follow Ilsa to the bindery". | TVX-06 | Open: worker B26 |
| TVX-16 | 2 | The TV and the phone disagree on hit points: the TV's health bars showed the heroes at 11/11 while the phone and the victory card showed 13/13 (a level 1 Dwarf Fighter with Constitution 14 has 10 + 2 + 1 = 13 with Dwarven Toughness). Seen by the independent reviewer in the market scene's screenshot. | TVX-14 | Open: worker B21 (the TV's health cards) with Q03 (the view's maximum) |
| FGT-15 | 4 | "Javelin (throw) · normal range: 20 ft" reads as the weapon's range (a javelin is 30/120 ft); it means the target is 20 ft away. | FGT-06 | Open: worker B28 |
| RUL-26 | 3 | After a won fight the TV says "The party decides: rest or move on" but offers no Short Rest when the heroes are unhurt; a party may take a Short Rest whenever it wants (SRD 5.2.1 Short Rest). | RUL-23, RUL-25 | Open: worker Q03 |
| NTE-07 | 3 | The notebook's errand reads "Reward: There is talk of trouble farther along. Come and hear it when you can.": a story hook in the reward's place; the promised payment is nowhere. | RWD-01, END-07 | Open: workers Q03 (the reward paid and recorded), B20 (the generated reward) |
| CRE-10 | 1 | On the pushed build `67d54be1` no hero can be made on the phone: after joining, the page says "The table could not load the hero creator. Your seat is saved. Try reconnecting." and Reconnect does nothing. Cause: `wire.rs` `creation_offer` sends empty `identity_groups` and no `default_identity`, and the admission refused every supplied identity (`hero-identity-storage-unavailable`). | CRE-08 | Part (uncommitted coordinator fix in `wire.rs`, played: the creator loads, Maren was made): the offer now carries `hero_identity::menu_offer()` and `Identity::from_menus(&[])`, and a valid identity is accepted and logged `HERO identity accepted ... stored=no`. Work needed: store the identity with the character (state, checkpoint codec and recovery, sheet), so portraits, plates and replies use it |
| CRE-11 | 3 | Reading a typed description is phrase matching only ("automatic reading is off; only matching menu phrases were filled in"): "a wiry grey-haired dwarf woman in her middle years, with a burn scar and a blue cloak" set feminine, wiry, grey, burn scar and blue cloak, but kept They/them for "woman", Adult for "middle years", and the fear "Protect my home" for "I fear losing another friend to the road". | CRE-10 | Open. Work: connect the model reader (Jev to classify, the story model to extract) behind the provider budget in `preview_hero_identity`, the keyless reader as the fallback |
| CRE-12 | 3 | The creator still requires the old preset figure ("Red beard, greatsword", "Braided hair, greatsword", "Woman with a bow") after the player has described their looks, and the eight choice drop-downs (alignment, two languages, two skills, gaming set, figure) have no visible labels inside "Skills, languages and equipment". Only Dwarf is offered. | CRE-08 | Open. Work: drop or derive the preset figure from the identity; label every drop-down; offer more species once the rules support them |
| PIC-17 | 2 | The pictures are still painterly, not photographic (owner: "all the graphics are still painterly while I asked that it look photographic and realistic"). Cause: every scene plate is generated with reference images that are hand-painted files: the party sheet from `figure-hero-*`, the speaker's `sprite-*` figure and face, and the place's `scene-place-*` concept art (`gameplay/living_scenes/plates.rs` about lines 1195-1260 and 1438); image models copy a reference's style more than the prompt's FILM_LOOK words. | PIC-14, B25 | Open. Work: send only references the live pipeline generated with the film look (or none); say in the plate prompt that references fix identity, layout and costume, never style; regenerate the static painted art (hero figures, cast sprites, place art, lobby and join backgrounds) as photographs |
| TVX-17 | 2 | The narrator repeats the scene's opening narration after a search (owner: "the narrator just repeated the first level entry narration again"). Cause: `wire.rs` (search `investigation::observation`) replaces the view's narration with the observation's text while the check is the latest, then the next view switches back to the scene narration; the TV takes the change for a new narrator line and replays the cached opening clip (only four voice clips were made). | TVX-07 | Open. Work: keep the scene narration for the whole scene; the observation stays its Narrator table line (a fix was drafted and reverted unreviewed) |
| INV-13 | 2 | Looking around succeeds but describes nothing: "I look around the guardhouse for anything useful or out of place" rolled Wisdom (Perception) 21, success, `finding=none`, and the Dungeon Master said only "the guardhouse", although the scene model holds Lorcan, Jessa and Rusk, a worktable, an oil lantern, a small iron key, a lidded chest, a clay cup and three ways out. | INV-06, ENV-02 | Open. Work: a successful room observation describes the visible people, notable things and ways out from the scene model (fewer on a failure), never clues |
| DLG-09 | 2 | A true answer is thrown away again: asked "who is following you, and what is in the letter?", the model answered "Someone is following me, but I don't know who. The letter is sealed with wax." and `NPC REPLY rejected rule=spoken_rule` replaced it with "letter is a wax-sealed letter." (lower case, circular). A stopped helper found the fallback matcher splits facts on " is " and matches any 3-letter token. | ENV-11 | Open. Work: the spoken rule accepts honest answers that mention the errand's own object or admit not knowing; the fallback is a proper sentence |
| PHN-15 | 3 | Exploring on the phone is buried and not tappable: "In this scene" (people, things with "look · take · use", ways out) sits about 3,000 px down a 3,845 px page; "look · take · use" are tiny plain text, not buttons, and run into the labels ("centerlook"); "Look around" is far below the choices, so the choice list shows a gap where the TV shows it. | PHN-14, TVX-11 | Open. Work: put the scene's people and things near the top as tappable targets with real Look, Take and Use buttons; keep the phone's choices identical to the TV's |
| PHN-16 | 3 | The composer can lose a typed line: focusing the text box scrolls the page, the next tap landed outside the composer and the draft was cleared. A "Tap to talk" bar peeks out behind "Say it". When the fight starts the phone stays scrolled to the bottom (the composer), not the fight. | PHN-13 | Open |
| ITM-01 | 3 | Taking the small iron key works and its card on the phone has a good description ("dark iron with a square bow and a long narrow bit...") but no picture, although the server made it (`IMAGERY ready kind=item ... ms=12055`). The phone line reads "Maren picks up small iron key." (no article). | ENV-07 | Open |
| FGT-17 | 3 | The choice was "Face Cindle the Kobold" but no Cindle takes the field: the enemies are "Kobold Warrior 1, 2, 3". The phone's reaction prompt shows no countdown while the TV shows "(0s remaining)". | STO-01, FGT-13 | Open |
| ENV-13 | 4 | Better on the pushed build (recorded): the opening states the job, the stake and the pay ("The job is to escort Lorcan to Rowan at the Farmhouse Kitchen. If it fails, the warning inside will arrive too late." / "I'll pay you eleven gold"); the opening plays one line at a time (narration voiced 13.8 s, 2.5 s linger, then Lorcan 9.6 s); dialogue is subtitle text; the creator is 1,205 px with a "Still needed" list; the sheet lists saves, all skills and Dwarf traits; mood notes are gone; the check card names the right hero; the fight heading matches the story; hit points agree; an opportunity attack uses the greatsword and the TV says whose decision it is. The TV hides the QR code because this server listens only on this computer (by design). | | Recorded |
| RWD-02 | 2 | The promised gold is not paid: Lorcan offered "eleven gold"; after a won adventure Maren still has her starting 18 gp, and the end card shows "Encounter experience: 75 XP" (correct: three Kobold Warriors at 25 XP) but no reward. Cause (Q01's note): the generated adventure's reward (`reward.gold`, written by the generator since `5e52f893`) is not bound to the pack's `ErrandReward`, which `journey/rewards.rs` pays. | RWD-01, END-07, NTE-07 | Open. Work: carry the generated reward amount into the pack's ErrandReward (and the record's fact sheet), pay it at a won end, show it on the end card and in the notebook |
| STO-10 | 3 | Stock sentences repeat and read badly: "The retired soldier now looks to Maren for support on the way." closes the victory card, the epilogue narration and the next scene; "Rowan glances over her shoulder as Rowan welcomes Lorcan"; the rest reads "The original victory and Lorcan's unresolved journey remain." | STO-09, RUL-09 | Open. Work: no fixed sentence repeats within an adventure; the rest is told as story |
| FGT-18 | 3 | The choice said "Face Cindle the Kobold" and the victory card says a rival paid Cindle, but Cindle never fights (the enemies are Kobold Warrior 1 to 3), and Cindle's battle cry ("This tunnel is ours! Leave the letter and go, or I leave you to the clutch!") appears in the phone's chat during the epilogue and at The End. | FGT-17 | Open. Work: the named antagonist takes the field (named on the TV and phone); fight lines stay in the fight |
| TVX-18 | 3 | In the epilogue Rowan's subtitle is placed beside the man on the left (Lorcan), not beside Rowan. | TVX-07 | Open |
| RUL-27 | 3 | After a won fight both unhurt heroes are offered "Short Rest · 1 hour, no Hit Point Dice" although each has an unspent Hit Point Die (the label should say it is there to spend, or that they are at full hit points). The rest itself works: Second Wind's use comes back and the story goes on (RUL-25 is fixed). Second Wind's heal (3 to 13 hit points) leaves no line in the server log. | RUL-23, RUL-25 | Open. Work: an honest rest label; log every Second Wind roll and heal |
| DLG-10 | 3 | The contact does not know her own part: Rowan, who receives the letter and "learns the letter's truth" on the victory card, answered (rejected by the spoken rule again) "I don't know who paid the kobolds; I know only that the letter is sealed with wax, not what will be done with it." | DLG-09, DLG-08 | Open. Work: the contact's dossier holds the errand's outcome and what she will do next |
| PHN-17 | 4 | Sending an empty line shows "Enter between 1 and 4096 UTF-8 bytes." | PHN-16 | Open |
| FGT-19 | 4 | On the 3D battlefield both heroes are the same bearded male figure; Maren (a woman with grey hair and a blue cloak) is not recognisable. | CRE-10, FGT-13 | Open (with CRE-10's identity storage) |
| ENV-14 | 4 | Better (recorded): the fight followed the rules visibly (Sunlight Sensitivity gave a kobold Disadvantage in daylight, Pack Tactics Advantage, a missed greatsword still dealt its Graze 3, Savage Attacker kept the higher damage roll, the opportunity attack used the greatsword); the Short Rest no longer dead-ends; Second Wind came back; the end card shows the adventure's title and its XP. | | Recorded |
| PIC-18 | 2 | The scene picture never changes while a scene plays (owner: "the display plate did not cycle or change during this phase"). Taking the small iron key asked for a redraw (`IMAGERY request kind=scene_plate reason=place-guardhouse state=1`), but every scene-plate edit failed at the provider in about 1.2 s (`IMAGERY failed kind=scene_plate id=none ... status=invalid_request`, three times), so the first picture stayed for the whole scene. Meanwhile 23 of the session's 40-image budget went to hero "health" cut-out variants (12 to 15 s each). | PIC-14, PIC-17, MOT-03 | Open. Work: find why the plate edit request is invalid (the edit call's references, size or mask) with a provider-free test of the request's shape; spend the picture budget on scene plates first and make health variants only on a real tier change, cached; then make the picture follow the scene at a calm pace (a settled change at most every 10 s: what was taken or opened, who is speaking, where people stand) with the crossfade swap |
| LDG-01 | 2 | The game has long waits and almost no sign that it is still working (owner: "some classy and genre specific loaders and spinners would be helpful to show the app is still working"). Measured waits: the phone's first load (8 to 11 s blank before the "Setting the table..." screen was added, which is a generic bar); "Waiting for the game server..." after confirming a hero and then a pale button; a character's reply (the model thinks 5 to 35 s on the phone with only the typed line and nothing moving); a scene picture (22 to 24 s) and its edits; a hero's cut-out (27 to 34 s); an item close-up (12 s); a voice clip (0.2 to 0.9 s but queued behind the reading time); "Read my description" in the creator; a new adventure being prepared; enemy turns in a fight (3 to 10 s of nothing on the phone); the TV between a choice and the next scene. | PHN-12, TVX-09, PHN-16 | Open. Work: one small loader family in a fantasy tabletop style, used by the wait it fits, quiet and filmic (no ornate frames), always with a plain words line and honouring reduced motion |
| LDG-02 | 3 | Which loader for which wait (the design for LDG-01): (1) a character is thinking about a reply: a small quill that writes a line under the speaker's portrait on the phone and a candle flame beside the thinking character on the TV (the TV already keeps a thinking mark until the voice clip starts); (2) a picture is being made: a slow vignette with a drifting torch glow over the previous picture, never replacing it (the previous plate stays up, B24's rule), and a thin "the scene is taking shape" line only after 4 s; (3) the phone is joining or loading: the existing "Setting the table..." with a turning d20 inside a compass ring (as the preparing screen has); (4) hero creation reading a description: ink spreading across a page; (5) the DM is narrating or the enemies are acting: a slow-turning initiative ring with the next name; (6) a new adventure being prepared: the existing night scene, with the rolls the generator makes listed as dice landing (the generator logs its rolls); (7) an item close-up: a lens-focus pull. Each states what is happening in words, appears only after a delay so fast actions do not flash it (about 600 ms), and logs its start and end as DISPLAY/PHONE timing lines. | LDG-01 | Open: design note for the loader lane |
| DLG-11 | 2 | After about ten model replies every character falls back to a stock line: the story model's token budget for the whole run (default 150,000, `DF_GAMEPLAY_TEXT_TOKEN_BUDGET`, after the adventure's own text is written) runs out. Benchmark of 37 lines on the taproom adventure: 10 fallbacks `budget_exhausted`, 2 `rate_limited`, 2 `call_failed`; the pressure group (repeat, threat, apology, thanks) fell back 6 of 6, and room descriptions switched to rules text (`OBSERVATION source=rules reason=storyteller token budget exhausted`). Nothing tells the player or the log how much is left. | DLG-09, EMO-02 | Fixed (owner: "remove that restriction and all other restrictions"): the defaults are now the maximums (story model 100,000,000 tokens and a 60 s deadline, voice 1,000,000 characters, pictures 10,000 at 20 s, speech 86,400 s, video 1,000) and the per-player reply limiter is gone; rerun of the 37-line benchmark: fallbacks 14 to 1, `budget_exhausted` 10 to 0. Every limit can still be set with its DF_GAMEPLAY_* variable |
| DLG-12 | 3 | A slow reply ends in nothing useful: two patron questions waited 13.1 s and 13.0 s (the 12 s text deadline, `NPC REPLY fallback reason=call_failed`) and got "Go to the Binding Workshop." for "Why can't you go yourself?" and, for "Is there any danger on the way?", a raw table dump: "the Restless Giant Rat is known as the giant rat. The Restless Giant Rat scurries into view, filthy and scabbed ... Two riders came through the Orrery Library before dawn, looking over their shoulders". Model replies take p50 6 to 8 s (the patron's mean 8.2 s over six questions) and nothing shows the wait. | LDG-01, DLG-11 | Fixed by the same change for the 12 s deadline (slowest reply 13.1 s to 9.2 s, both slow lines now real answers); the thinking loader (LDG-02) and a retry remain |
| DLG-13 | 3 | Stock lines do not answer: "Ale should be dark and the night darker. That is my whole philosophy." was the answer to an apology ("Ilsa, I am sorry for my friend. Thank you for trusting us") and to giving bread to Nella; "How kind. I want word of my brother. Any word at all, even a rude one." answered a question to Tessa about gambling; "I would not ask this of you if there were anyone else I could trust with it." answered "can we trust the travelers in here?"; "Where exactly is Tobin?" asked three times got three different stock lines with no "I already told you" and no change in patience in six lines. | DLG-07, EMO-01 | Part: the stock-line answers are gone on the benchmark (apology, bread, trust and the repeated question all got real replies); still open: a repeated question is not acknowledged or tired of ("Where exactly is Tobin?" three times was answered kindly each time), and replies after a full stop begin lower case ("Head to the Binding Workshop. that’s where he’s waiting.", "The Binding Workshop, Brakka. that is where Tobin is waiting.") |
| DLG-14 | 3 | Naming a bystander does not always reach them: "Tessa, what are you gambling for?" was sent to Ilsa (`STORY say target="Ilsa"`) after the line reader failed (`APPROACH source=keywords reason=error`, then the keyword fallback); "Nella, how long have you worked here?" got "I don't know how long, truth be told." (a character who does not know her own tenure); "Who are the two men at the back table, and what are they doing?" was read as an action, rolled a check and answered with the scene's raw text "a broad-shouldered traveler in a road-dusted coat rolls dice with the other carter" (one man, no names, though the scene names Tessa and Bram). | DLG-08, ENV-10 | Open. Work: an address by name is resolved from the scene's people before anything else; a question about people is never an action; bystanders get a short dossier line (tenure, what they do); names and the second person are in the answer |
| INV-14 | 3 | Questions about a thing get the thing's card, not an answer: "What time does that brass clock say?" returned the clock's description ("a brass face behind cloudy glass and a narrow keyhole below the dial") and "You do not notice anything further about brass wall clock."; "What is in the bundle of drying herbs? Is any of it the white fever-herb?" returned the herbs' description with the same closing sentence and an item card, never saying whether the fever-herb (the errand's object) is among them; the look at the dice and cup says only "familiar wear offering nothing new". | INV-13, ENV-08 | Open. Work: a question about a thing is answered from its facts (the clock gives the hour from the scene's time of day; the herbs say yes or no from the plot's facts, with a way to find out); one closing sentence at most |
| SCN-05 | 3 | Item and scene answers read as data: "dripping candle stub is being used; ask its holder first." (lower case) refused taking a candle nobody held, a minute after Sigrun looked at it; "Sigrun picks up bundle of drying herbs." and "Sigrun opens front door." (no article; opening the door says nothing of what is outside and the TV showed no change); "front door leads to the taproom in dusk; a low mist outside is visible beyond it." and "The window leads to the taproom in dusk;" (lower case, a semicolon, cut off); a failed Perception listening at the door (5 against 10) still gave the full description. | SCN-04, INV-13 | Open. Work: sentence case and articles in every scene sentence; a look is not a hold; opening a way out tells what is beyond it and the picture follows; a failed check gives less |
| TVX-19 | 2 | The check cards go by too fast under quick play (owner: "the perception check went by too fast to read", "all the checks are going too fast"). Measured: one check at a normal pace keeps its card visible for about 8 s (from 3.0 s to about 11 s) while the narrator speaks 3.5 s to 14.8 s, which reads; but the card's 8 s fade starts when the view arrives, a new check removes the previous card at once (`set_check` removes other panels) and nothing ties the card to the line queue, so checks arriving 3 to 5 s apart (two heroes, or the benchmark) flash past. | INV-07, TVX-10 | Open. Work: checks queue like lines; a card holds for its reading time and until the narrator has read the result; the next check waits its turn; the dice roll first (tumbling d20, FPX-01) and then the card; log the card's start and end |
| ENV-15 | 4 | Benchmark of dialogue and exploration on the taproom adventure (37 lines, build `67d54be1` plus the creator fix, story model and Jev on): 0 empty replies, 0 rejected replies, 37 accepted; reply time p50 1.0 s, p90 6.9 s, max 13.1 s; 14 fallbacks (38%), all but 4 of them `budget_exhausted` or `rate_limited`; 8 checks, 8 scene changes, 6 item cards. Strong: the patron's facts (child, Tobin, 14 GP, "I don't know how long the child has been sick" as honest ignorance); Nella named by Ilsa; "Whose mug?" answered "belongs to Ilsa Whitlock"; "I look behind the counter" resolves to the bar with no roll (INV-12 fixed); purchases told as story and exact in coin (Nella pours the ale, 4 CP; bread 2 CP); hearth and candle descriptions rich and instant; things that are not there refused plainly ("You see no golden chandelier here."). Raw data: `artifacts/tmp/campaign/luna6/V8-bench.jsonl` | | Recorded; rerun after the fixes with `bench6.py` |
| TVX-20 | 2 | While a check card is up (about 8 s) the TV shows no text at all (owner: "nothing is on screen even though the characters and dialog was playing"). Measured over 43 s of a play session: with the card visible, 0 of 21 samples had any subtitle, bubble or narrator box; 22% of the time a line was "speaking" nothing readable was on screen, one run lasting 6 s. The card replaces the hero's action bubble and lower third by design (`set_check` clears the figure's bubble container, `live_stage.rs` around line 877) so the narrator's voiced reading of the result and any reply that arrives are invisible. | TVX-12, TVX-19, TVX-07 | Open. Work: the card sits beside the subtitle and never replaces it; only the hero's own action bubble yields; a test that no moment has a speaking line and no visible text; checks queue (TVX-19) |
| ENV-16 | 4 | Rerun of the dialogue and exploration benchmark with the limits removed (`V9-bench-nolimits.jsonl`, same 37 lines): 0 rejected, 1 fallback (`reply_rejected`); reply time p50 2.7 s, p90 6.8 s, max 9.2 s, mean 2.9 s; the lines that were stock answers are real replies ("I appreciate the apology, Sigrun. And I’m grateful you said that. I only hope the fever-herb reaches the child."; "I’m rolling dice, not gambling for anything in particular." from the bystander). Still weak: a thrice-repeated question is answered the same kind way each time, and "Bread for Nella, then. Dogs judge folk better than priests do." is a non sequitur. | ENV-15, DLG-11 | Recorded |
| NTE-08 | 4 | One place is listed twice in the notebook: "the bindery" and "the Binding Workshop". | NTE-03 | Open: worker Q02 (the notebook) |
| PHN-14 | 3 | The phone's notebook and menu open as sheets over the tab bar: switching from the notebook to the menu needs Close first; tapping another tab does nothing. The menu has no sound, voice or text-size setting. The hero sheet's heading says "Abilities and saves" but lists no saving throws. | PHN-13 | Open: worker B18 (sheets and tabs), C-lane after B18 (the sheet's saves) |

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

## 2026-10-15 dialogue regression session

The agent CLI now reaches NPC dialogue without using the character creator: `dev play --to dialogue` on a fresh two-seat local room joined both seats, made Brakka and Sigrun through the live creation offer, chose the opening `ask` path, and returned `phase=dialogue`, `combat=null` after six admitted steps in 8.4 s (`e7a71311`). Running it again in the same room returned with zero steps in 98 ms. Focused tests passed (7 CLI play, 4 creation selection). The room was stopped cleanly after the visual check.

On that room's display, the state machine reported `data-battle-state=absent` and `data-battlefield-live=false` during dialogue (`37ace03e`). A generated 2560-pixel plate loaded and was visibly shown behind the Courier's dialogue; Brakka and Sigrun's health labels were separated. This confirms the previously empty scene and plate were fixed for this fresh run (`dae5b98b`, `a2327cdd`). The player still judged the plate's finish painterly rather than photographic, so PIC-17 remains open. The visual check did not establish audible TTS or mute behavior.

The earlier room could not be resumed (`retained_access_unavailable Unauthorized`), so its saved state was left untouched and the regression used a fresh database. This is still an untriaged resume failure. Five Courier questions in that earlier room returned repetitive deferrals about the packet, seal, ship, and follower; dialogue quality needs a separate pass against TLK-13/DLG-09 rather than a claim of improvement from this rendering session.


### PIC-17 documentary photographic revision (2026-10-15)

The `-photo` suffix did not establish a photographic medium: visual review of the
previous courier, heroes and harbor still found glossy CGI costumes, polished
mail and theatrical blue/amber light. A new built-in imagegen series starts from
candid documentary photography on a practical historical film set, without old
painted or CGI references. The coordinator independently viewed and accepted the
candid harbor, male swordsman and courier. Seven source generations and the
encoded artifacts are described in `assets/ui/generated/documentary-photo-v2.assets.json`.

The three hero performers, courier, bandit, empty arrival harbor and candid harbor
now use versioned `-documentary-photo` stems. Public cast/portrait routing resolves
the earlier nominal photo aliases to that revision. Runtime photographic-reference
selection accepts the reviewed suffix only, never the rejected `-photo` or `-film`
siblings. A fresh plate without place photography receives the empty documentary
harbor photograph as an explicitly style-only reference; written scene facts still
own its place, people, objects and light. The documentary medium precedes scene
JSON. Quality remains the configured provider setting: there is no evidence that
raising quality alone fixes the medium.

The empty harbor is the lobby/arrival image; the candid cast photograph is the
fixed harbor dialogue preview. The old painted inn fallback is removed, leaving
the stage base color until a valid runtime image is ready. Ending and combat
fallbacks use the reviewed harbor and new performer portraits. Earlier image
files remain historical source artifacts, but their rejected photo siblings are
absent from active serving lists and fallback selection. This is not a claim that
all other generated campaign locations or cast have been photographically reshot.

All generated actor alpha channels survived WebP encoding byte-for-byte. The
image viewer shows dark RGB in transparent regions; sampled halo-region pixels
have alpha zero. Browser compositing and the integrated provider-generated plate
remain required independent acceptance checks. Four isolated native routing
tests passed; authored changed Rust files pass rustfmt and diff whitespace checks.
Full native/WASM checks and live output acceptance belong to the coordinator.


### PIC-17 generated plate regression and photographic edit attempt (2026-10-15)

Independent live acceptance of `97f88c56`, room GADUME, rejected ready plate
`da28150425464bbdec413929084cef2ca6d4228a030cfb365db507b4a288945a`.
The lobby and candid dialogue preview read as photographs, but the 2560-pixel
runtime plate again showed CGI finish. Its 3,761-character ShotSpec prompt still
requested a crisp best-lit speaker facing the camera, deep warm lights, and cold
blue dusk against warm practical light. The reviewed empty harbor was a secondary
style-only reference after separate actor cutouts; no complete photographic frame
was being preserved. The failed screenshot is retained by the coordinator as
`gadume-generated-plate-still-stylized.png`.

This attempt uses the reviewed candid harbor as image 1 for the fixed exterior
salt-quay scene, followed by party/cast identity photographs within the four-image
limit. A separate short edit prompt retains that frame's actual optical capture
and excludes the shot director's aesthetic restaging, extras and lighting grade.
The scene's speaker task/expression, traveler activities and required other cast
remain explicit. A canonical SceneModel, when present, retains its complete JSON,
including time, weather and changed object states. Other locations use their
existing pipeline; the static lobby and cast preview are unchanged. Two native
test-call compile regressions were repaired alongside request-order and canonical
state coverage. This is an implementation candidate: only another full-display
provider/browser observation can establish that the generated output is improved.

### PIC-17 live generated-plate acceptance and cast follow-up (2026-10-15)

The combined native/WASM build at `a45dcca9` ran a fresh two-player dialogue
room SATEVO with all local model, voice, image, and video switches enabled.
The new plate was generated from the reviewed candid photograph as the first
reference (306,720 bytes), four references total, 1,950 prompt characters, and
finished in 25,022 ms. CLI and browser agreed on `PLATE_STATE_READY`,
`server_effective_source=live_plate`, and a decoded 2560×1440 image. An
independent visual evaluation accepted its natural photographic old-world look
and found no overlapping hero labels or dialogue. It also found a content defect:
Sigrun had been replaced by a male porter at the right edge. The two NPC
questions in that room remained in dialogue; video generation reported
`price_unavailable`, and the fixed harbor emitted no `text_model` resource
events. The SATEVO plate therefore did not pass full cast acceptance.

The follow-up `a7d0c636` keeps image 1 as the photograph and explicitly holds
the two party performers at their left and right positions. A fresh room REBEVO
reached dialogue with Brakka and Sigrun via `dev play --to dialogue`. Its
generated plate key `0608a4b7a3c26f3bc2ba2bed64149c82467b144cefef1e648bc1882be76825fa`
used the same four reference sizes, a 1,757-character prompt, and completed in
42,890 ms. The display loaded it at 2560 pixels, `data-plate-state=ready`,
`data-shot=plate`, and visibly retained both armored heroes, including red-haired
Sigrun at the right edge. The photographic medium and medieval quay still held.
The courier's face and hood differ from the reviewed source photograph, so cast
identity continuity remains open even though the style and missing-hero failures
are improved. Retained screenshots are `satevo-live-plate.jpg` and
`rebevo-photographic-plate-both-heroes.png` in the coordinator's outputs.

### Dialogue readability and live layout baseline (2026-10-16 UTC)

Room SUMURE used native/WASM build `ux-baseline-c0bdded0`, database
`df_gameplay_demo_play_20261016030017`, two heroes and enabled paid providers.
The browser stayed in dialogue with no combat. Source changes made during this
pass were not present in that fixed baseline.

- A 332-character hero question was displayed as 236 characters and held at the
  old 14,000 ms reading cap. The Courier thinking indicator covered its text.
  A same-scene subsequent NPC reply was readable, so this was a presentation
  length/placement defect rather than a missing model response.
- TVX-19/20 reproduced after two Insight checks: the card faded to opacity zero
  while `data-check-active=true` remained. A later line reported `speaking`, but
  every bubble was hidden by the global check-active CSS rule. Evidence:
  `ux-baseline-speaking-text-hidden-after-check.png` and
  `ux-baseline-timeline.json` in the coordinator's outputs.
- Same-phase lines 6–9 had open records without terminal/held records. Component
  visibility and lifetimes require actual browser reporting before CLI timings
  can certify readability. Logical `speaking` alone is insufficient.
- Retained DM scene context at `top:11vh` crowded the hero health row. The owner
  requested a lower caption position, then requested CLI translation, scale and
  rotation for every visual element so later adjustments need no compilation.
  The new live layout capability needs one initial installation build; its
  subsequent commands must apply immediately and expose measured readback.
- `dev back 2` restored a lower table-log sequence. New questions were accepted
  but suppressed by the connected browser until reload. This history/reconnect
  regression remains open and separate from the forward session machine.
- Dialogue still repeats deferrals about the crest, missing ship and follower.
  Asking the Courier's name produced only “I'm a courier”; addressing the Bandit
  elicited a Courier reply (“I'm no bandit”), reproducing the DLG-14 routing shape.
  Asking what had not yet been disclosed produced another refusal to explain.
  Neither the visual repairs nor a successful provider call establish dialogue
  quality improvement.

Source candidates for the next integrated browser pass include a 30 s reading
cap, 420-character captions, obstacle-aware loading indicators, queued check
cards with explicit readable phases, and removal of the global speech-hiding
rule. The photographic plate candidate retains the outgoing frame through a
1 s dissolve and supplies a deterministic numbered contact sheet of all selected
heroes and scene NPCs. These remain pending current-build visual acceptance.

### Live placement acceptance and remaining lifecycle repairs (2026-10-16 UTC)

Room RAGURA ran fixed native/WASM build `layout-4e00e958`, database
`df_gameplay_demo_play_20261016040310_1578`. The independent browser pass accepted
the current photographic medieval plate and cast, and the live layout registry,
translation, scale/rotation, partial updates, reset, applied readback, reconnect
inheritance and stale target rejection. The initial inventory had 237 addressable
elements and zero overflow. CLI-only subtitle/context moves applied without a
reload or restart and left gameplay revision unchanged. This is placement
acceptance, not acceptance of all presentation timing or dialogue quality.

- A decoded, generation-matching plate stayed unapplied after the final line
  became idle; a later interaction woke it. The frame/timer scheduler stopped
  because the conductor was idle even though decoded plate work remained.
  The repair keeps that work on the existing clock, including the 2.5 s reading
  linger gate and lost-transition retirement. Installed runtime retest pending.
- The evaluator's first long input began "I study" and returned disposition
  `action`; it performed Investigation rather than a spoken question. The
  faithful 332-character Courier question returned `social` and appeared in
  table-log entries 8/9, but its hero bubble host was hidden. The line had
  `read_ms=24080`, logical `held_ms=27206` and `visible_held_ms=0` for bubble and
  subtitle. The old check was recreated on redraw after its timed removal.
  The source repair deduplicates by authoritative Roll sequence and uses the
  existing named subtitle lane when a long bubble cannot fit. Retest pending.
- The upper-right title overlapped the sound control at the evaluator's large
  viewport. The 1024x768 view kept the remaining labels clear but hid the title.
  This title placement concern remains open.
- Accepted display batches reported aggregate client capture loss of one record
  while the timeline mirror's own dropped count was zero. No server telemetry
  refusal was logged. The current aggregate does not identify the lost event or
  its source; complete lifetime capture remains unaccepted.
- The original Courier deferrals remained repetitive. No dialogue quality,
  audio, or crossfade acceptance follows from the placement pass.

The session and exact native/WASM artifacts were retained before shutdown.
An old checkpoint correctly refused another build ID, so repair-build acceptance
must use a fresh quick-start database rather than bypassing its build pin.
SAM research is saved in `planning/segment-anything-feasibility.md` and indexed
in the visual resource manual; integration is deferred at the owner's request.

### Same-scene plate refresh and caption runtime acceptance (2026-10-16 UTC)

Fixed native/WASM build `layout-b6cb696d`, source `b6cb696d`, ran room PODOBU,
database `df_gameplay_demo_play_20261016050159_1369`. CLI quickstart created
Brakka and Sigrun and stopped in NPC dialogue without combat. Native build passed
in 44.82 seconds and WASM in 11.68 seconds, with existing warnings. The actual
in-app browser was explicitly reloaded after installation; an automatically
reconnected older page alone does not prove that its WASM was updated.

- The complete 332-character social question was visible for 26,950 ms at a
  1460-pixel viewport, above its 24,080 ms reading budget. Independent Chrome
  observation measured 27,236 ms. At 1024x768, the unplaceable bubble used named
  caption line 6 with the exact full text and 26,808 ms visible time. Old terminal
  events did not overwrite it; successor line 7 acquired ownership cleanly.
- The named lane was translated -96 px through the live CLI to clear the existing
  scene-context overlap. Default positioning and the title/sound-control concern
  remain open. Temporary responsive viewport overrides were reset after testing.
- A completed Courier reply automatically edited the shown photo `88dbbe...` into
  `48811fa9...`. Request diagnostics contain the prior photo digest, canonical
  cast-sheet digest `8e3fdbf7...`, and speaker portrait digest `58eff9c7...`, in that
  order. Repeating the identical reply did not queue another edit.
- A different reply produced `429cfcf2...` in 17,792 ms. Immediate browser sampling
  captured six intermediate incoming opacities, while the outgoing photo remained
  at opacity one. Zero-to-one took approximately 1.02 seconds at roughly 100 ms
  observation cadence; no blank frame appeared. The photo activated without a
  later gameplay input. Medieval photographic styling and cast continuity held
  in the visible portions; edge-cropped faces cannot establish precise identity.
- Fixed harbor request events now expose prompt/reference hashes without a
  manifest. Edited projection latency is null rather than an inflated scene age;
  original request latency and scene age are separate fields.

The earlier REGESU build `layout-8fd307b8` again reproduced DLG-14: explicitly
addressing Bandit returned Courier's reply. A descriptive address to the man behind
Courier instead returned an empty reply after a 5,140 ms model failure; that case
does not establish a routing result. Source investigation found the authored pack
path always selected the journey speaker, and plate bubble placement also used
the focus NPC's head for every NPC. Jev selection from validated public cast IDs,
persisted actor identity, voice/bubble routing, and per-NPC image head metadata
were the next implementation task. Installed routing and placement results are
recorded in the subsequent passes below.

Dialogue quality remains open: questions about the crest and delivery destination
still repeatedly defer rather than provide useful detail. Client capture-gap
aggregates, complete lifecycle coverage, queued-check timing and audible playback
are not certified by this pass. Evidence is indexed in the coordinator outputs by
`podobu-build-b6cb696d.json`, `podobu-accepted-caption-refresh-timeline.json`, and
the `podobu-*-samples.json` and screenshots.

### Canonical NPC routing and head traces (2026-10-16 UTC)

Build `routing-9b42661f`, room BAMERE, database
`df_gameplay_demo_play_20261016055351_1828`, passed the installed CLI classification
cases after a real reload of the Codex display:

- Exact Bandit address returned Bandit's persona and canonical actor ID in 857 ms.
- The man behind Courier selected Bandit through Jev in 160 ms; the whole reply
  took 863 ms. The actor ID persisted into the table line and browser bubble host.
- Absent Harbourmaster selected the absent outcome in 161 ms and returned an
  explicit refusal in 489 ms, with no new display line. The previous GOGOTA build
  incorrectly answered as Courier because the Jev prompt contradicted its absent
  choice. This is a measured correction, not a claim about every possible name.
- Ordinary discourse, "Wait, who are you?", used current focus Courier through
  Jev in 128 ms. Addressing Bandit and Courier together refused in 249 ms.
- `dev timeline --type plate` now contains head-location and actor-head records.
  A four-person provider batch completed in 16,662 ms and another in 21,320 ms.
  Both located four of four targets and mapped `npc-0` to `courier`, `npc-1` to
  `bandit`. Durable-note cache hits are separately visible. These head timings are
  distinct from NPC reply latency.

Spatial placement was rejected in this build: matching actor IDs and image-owned head
boxes reached the browser, but both root's actual Codex view and independent
Chrome observation rejected the Bandit box over Courier's torso. The solver's
first-clear side preference and viewport-to-portal coordinate mapping were the
next repair. Hero question text cleared the HUD in this build. Default named
lane/context overlap still needed the live subtitle translation of -96 px.

Evidence: coordinator outputs `bamere-build-9b42661f.json`,
`bamere-routing-acceptance.json`, `bamere-routing-timeline.json`, and the
`bamere-*-visual.png` screenshots. Voice HTTP 200 delivery and DOM `playing` were
observed; actual audible identity was not judged. Client capture gaps remain a
separate limitation even when the server mirror reports `dropped: 0`.

### Speaker proximity runtime acceptance (2026-10-16 UTC)

Build `routing-0187f83e`, source `0187f83e` (worker `bf6d906e`), ran room GOSASU,
database `df_gameplay_demo_play_20261016061307_1304`. Native build passed in 62 s,
WASM in 12.88 s, and the two-player dialogue quickstart in 7,933 ms. The actual
Codex display was reloaded before acceptance. Independent Chrome observation and
root's actual Codex view both accepted the corrected speaker association.

- Bandit line 3 was below his own head at approximately `[981,360,365,203]` in
  Codex, with a top pointer and no face/HUD collision. The waiting indicator was
  also beside Bandit rather than Courier. Independent Chrome observed the same
  association at its own viewport, approximately `[1105,395,420,173]`.
- Courier line 5 stood below Courier at approximately `[566,392,365,266]`.
  The full 332-character preceding question rendered without truncation. The
  two browser lifecycle producers measured 26,726 and 26,983 ms visible time,
  both above its 24,080 ms reading budget. Root's immediate screenshot polling
  captured only the final 19.4 s; the full duration comes from lifecycle records.
- The automatic story edit activated photo `993bf2f9...` with seven intermediate
  opacity samples and no blank frame. The photo remained photographic/medieval.
  Exact matching head records for Courier and Bandit were visible in the CLI.
- A subsequent descriptive address selected Bandit through Jev in 132 ms; the
  complete response took 895 ms. Its successor line 7 used `bubbles-bandit`,
  no missing-head fallback or retained anchor hold, and a box around
  `[972,365,365,139]` beside Bandit after the photo refresh.
- A further question sampled 47 frames over 10 s. Speaking zoom reached 1.16x
  and returned to 1x. Bandit line 9 stayed at approximately
  `[972,381,365,171]`, clear of heads/HUD with its pointer toward Bandit.
- The final state remained `dialogue`, `combat: null`, revision epoch 1/sequence 7.

Source changes compare nearby lateral and above/below candidates, map measured
heads through the actual active image bounds, and convert the chosen viewport
box into the bubble portal's coordinates. Existing readable captions retain
ownership; successor lines use the updated geometry. Twelve focused geometry
tests, native/WASM checks and independent review passed. Required strict Clippy
still reports the same existing 80 native/15 WASM findings; it is not a clean lint
pass. The WASM DOM test sources compiled; this running-browser pass supplies the
physical output evidence.

Evidence: coordinator outputs `gosasu-build-0187f83e.json`,
`gosasu-routing-acceptance.json`, `gosasu-final-timeline.json`,
`gosasu-long-courier-samples.json`, `gosasu-zoom-samples.json`, and the
`gosasu-*-visual.png` screenshots. This is bounded acceptance for routing,
placement, readability and plate continuity. Default caption/context positioning
still uses the -96 px live subtitle override. Audio was not auditioned; client
capture gaps, narrative deferrals/memory quality, title/control layout and queued
check timing remain separate open work.

### Dialogue refinement investigation (2026-10-16 UTC; acceptance pending)

Current source had conflicting history instructions, forced hero-name addressing,
and generic fact paraphrasing. Changes in `npc_dossier.rs` and `npc_facts.rs`
prioritize the latest turn and relevant answers, preserve public follow-up context,
separate voice hints from facts, and make names optional. NPC reasoning moves from
Low to Medium within the existing 450-token cap. Added the canonical 12-turn
`natural-conversation` scenario in `npc_talk_live_tests.rs`; old adversarial and
private-listener scenarios remain.

Five bounded direct Responses experiments produced 70 replies / 118,256
supplier-reported tokens (120,000 total cap): 32 baseline/v1 comparisons, 16 v2 Low
samples, 8 model/reasoning comparisons, 12 Medium canaries and 2 capture replays.
Four calls benchmarked Sol; the game model was not changed. Contexts were synthetic
public states derived from retained Perrin fixtures, not running game sessions.
The first candidates were rejected for unrelated footrace/guild padding and an
invented negative book action. Medium Luna samples replaced these with grounded
limits and resolved the location follow-up. A simple destination became
"Take it to the Old Hayloft." Missing family data received a short boundary rather
than self-ignorance or invented children. This is bounded prompt evidence only.

Remaining: run current native/WASM and live canaries through actual validators,
retries, fallback, persistence and both players. Repeated canonical facts can still
hit the four-content-word repeat rule. Unknown/irrelevant fact selection and exhausted
subject fallbacks need dedicated fixtures; a prompt cannot prove semantic grounding.
Review both content ratings, private/public continuity and actual audio delivery.
The final source policy was shortened after experiments to preserve the existing
guidance headroom gate (Restrained 3,118; Mature 3,692; style 467 characters).
Formatting/whitespace checks passed. Builds, browser play and audio audition remain
blocked by zero free C: bytes and the stopped local server. No quality-complete claim.

**Deferred nice-to-have — SAM 3.1 character localization for speech bubbles.** On
the exact generated plate, optionally cache person boxes or masks in normalized
source-image coordinates and use them as extra geometry to refine bubble placement.
Map detected instances to canonical actor IDs in a separate step; localization alone
does not establish which hero or NPC is depicted. Key results to the plate hash and
revision, reject stale results, and measure bounded inference latency and spend. Keep
the current head-location and bubble-placement path as the fallback. Hosted inference
is still unverified; see [Segment Anything feasibility](segment-anything-feasibility.md).

**2026-10-16 KOKOZA retest:** The disk/build blocker above is historical. A fresh
native and WASM build of `0996445c` ran as `dialogue-0996445c`, with two ready
heroes in dialogue and no combat. Sixteen NPC exchanges and a packet inspection
were exercised with visual checks after each interaction. The inspection resolved
to 8 + 0 = 8 but used a clipped narration phrase as its target; its result card
remained hidden for its entire 11.8-second lifecycle. NPC replies also invented
unknown injury, history, payment, motive, employer and witness details. Follow-up
repairs therefore cover authored disclosure/silence, grounded question answers,
canonical investigation targets, DM result narration, and queued card visibility.
Seven completed 1,000 ms plate crossfades were observed before later scene tests,
but subtle edit direction made the progression hard to perceive. Actor identity
measurement and zero-width retained captions also failed in the running build.
Repair code is awaiting integrated compilation and running-output evaluation;
this retest does not establish audio audition or complete feature acceptance.

**TEPEVA follow-up, `dialogue-dm-6adb3e5a`:** Native and WASM builds completed;
159 focused repair tests passed. Fifty failures from the broader native sweep
were also reproduced on the prior `d99afc5f` snapshot. Strict Clippy still reports
80 native and 15 WASM findings. The old KOKOZA database was preserved and refused
the changed content revision; a fresh two-player CLI quickstart reached dialogue.
Its layout revision is zero, with no caption-hiding override.

The first live Perception check (13 + 1 = 14) produced a visible 600 by 186 px
card, but DM acceptance failed. Generic narration line 6 opened at 192404 ms;
the check card first became visible at 199176 ms, 6772 ms later. A second DM line
read the full 90-character roll metadata. The user heard both defects. The card
reached readable at 204620 ms but never closed: the final lingering line stops
the presentation clock before its terminal transition. Result-only voice,
duplicate narration suppression, visible reveal ordering, final card retirement
and complete CLI timing evidence remain under repair. Browser audio reached its
7.7845-second end; root did not audition it. Capture gaps reported four dropped
records, so incomplete lifecycle fields are not proof that a media event failed.

**BEKEMO, `dialogue-9162f2fd`:** 157 focused native checks passed and native/WASM
builds completed. Build pins prevented loading the old TEPEVA checkpoint; that
database was preserved and a fresh two-player CLI quickstart reached dialogue.
The redesigned card was visibly readable at 600 by 307 px, showed the accepted
intention and full finding, and retired from the DOM. This run was muted during
the check, so it does not verify DM playback. The absent CLI retirement event was
traced to an unsupported `read-complete` capture reason; use the shared `read`
reason and retest the actual Hidden event and held time.

The user reported absent Courier audio. Its caption was present, but no Courier
voice request existed: the scripted caption carried actor ID `courier` while the
registered voice only carried the name `Courier`. The audio element's finished
6.32175-second clip belonged to the preceding DM line. Shared canonical speaker
resolution and case-insensitive name matching are under repair; retain exact actor
IDs and verify the Courier's own request/playback after rebuilding.

**SAMURE, `dialogue-e3caa3f1`:** 23 voice/casing tests passed, WASM library and
test compilation passed, and the user confirmed the Courier was audible. This
repairs the canonical actor/name mismatch. A live lowercase Courier question
also resolved to the canonical `courier` actor. The Bandit motive question returned
`withheld`, with no invented NPC reply.

The roll retest still failed human timing acceptance. Card result became readable
at 189668 ms but hid at 192168 ms; DM voice began at 192189 ms. The result had only
2500 ms to read and disappeared 21 ms before speech. The user confirmed the DM
was late and the card disappeared too soon. The silent roll predecessor's ordinary
linger and Closing cleanup are being replaced with an explicit bounded handoff;
the result must own its full reading and voice window. The canonical `read`
retirement event is now visible in the CLI. Remaining visual observations include
incorrect actor head assignments and role-label replies such as "I'm Courier."

**FUTERA, `dialogue-30a4857a`:** 94 focused native tests passed; WASM library
and tests compile, formatting passes, and strict lint diagnostics match the prior
build (native 80/86, WASM 15/30). The Courier's identity question now returns the
authored deferral "You can call me the courier. My name can wait." Lowercase and
uppercase questions retain canonical actor ID `courier`; a compound question
preserves the authorized story clause. Public identity facts, aliases/refusals,
explicit withholding, trust gates and private audience continuity have focused
boundary tests. Full independent visual coverage remains unavailable to the
evaluator's browser session.

Three actual checks revealed their results 213, 213 and 243 ms before DM playback.
The readable windows were 5703, 8652 and 7703 ms, all through actual voice end.
Root visually observed the cards during playback and after retirement. The user
confirmed the repeated check went well and selected gentler entry/exit fades as
the remaining polish. A settled reconnect card restored silently and retired;
its budget ran behind the sound modal, so human reading behind that modal is not
accepted. Retained scene context also overlapped the named Courier caption and
persisted through a new check. The scoped follow-up retires recovery context
when a current line/check owns presentation and adds conductor-owned opacity
fades; new running acceptance is pending.

The same playtest exposed two scene-context gaps. Both natural-language packet
inspections were refused: fixed harbor has no structured scene model or packet
entity/affordance in its investigation catalog. This is not evidence that simply
stripping the article from classifier output will repair it. A Courier scene
question produced an unsupported "nothing more from here" clause. Latest check
narration can replace the stable public scene paragraph, while the no-dossier
path lacks the shared structured room model. Two-pass preparation/plate
observation and time-aware scene-memory representation are being designed from
these concrete failures; no image observation is authorized to create a clue,
hidden name or rules fact automatically.

**GUFEDU, `dialogue-a99bcf03`:** The selected 250 ms entry / 350 ms exit opacity
polish and recovery-context retirement are integrated. A first fade run exposed
that re-appending a card's bubble host to its existing portal restarted its entry
at result reveal and DM opening. The same-parent guard now prevents those moves.
In the corrected running build the result became readable at 26368 ms, DM started
at 26587, voice ended at 29677, Closing began at 32071 and removal was reported at
32489. No extra transparent transitions occurred at reveal/handoff. The reloaded
Courier caption and next check both had zero competing scene-context nodes.
The card held after its own voice end and retired; final lowercase identity speech
still used the Courier's authored deferral. Root observed actual display output;
independent IAB and separate human fade-smoothness review remain unavailable.

WASM library/tests and formatting pass. Strict WASM lint remains the same 15/30
existing findings. Native timing/name/voice code is unchanged from 94 passing
focused tests. The remaining scene gaps and conservative temporal representation
are documented in [two-pass scene hydration](scene-observation-hydration.md) and
[scene memory](scene-memory-representation.md). Those are planning proposals;
the scene entity bridge, byte-bound observations and merge reducer are not yet
implemented. Actor localization, generic clues, startup-modal reading coverage
and lossless capture remain separate unfinished items.

**SANAPU, `portrait-ec334de7`:** Per the user's preferred portrait/name/text
panel, all managed character speech and typed dialogue now uses `.tv-lower`
instead of floating speech bubbles. Brakka, Sigrun and Courier portrait/name
switches were visually inspected in the real Codex display with actor head
geometry available. Reconnect and photograph replacement retained the caption;
choices stayed 17.51 px below it. Brakka's question had 10200 ms steady readable
time against a 7700 ms budget; Sigrun's had 6500 ms against 4000 ms. Subtitle and
line timeline events, live subtitle registration and speaking camera zoom remain
available. The operations manual now identifies `--component subtitle` for
character dialogue. Native/WASM builds, affected formatting and WASM library/test
compilation pass. Strict WASM lint still fails with the unchanged 15/30 baseline;
32 distinct normalized diagnostics match a99bcf03. Browser tests were compiled,
not executed by a WASM test runner. Independent source findings were repaired;
independent visual approval remains unavailable because its IAB inventory was
empty and Chrome creation lacked the visibility capability. No new human audio
or combat acceptance is claimed. Root's actual IAB observations are separate.

**SANAPU continuation, same `portrait-ec334de7` build:** A slow NPC/clue pass
submitted six NPC questions, two supported observation checks and a packet-seal
inspection, with actual Codex display inspection after each input, including
rejections. No combat, dice override, manual plate direction or runtime source
change was used. Five new public story plates reached the stage, for story
sequences 12, 19, 21, 23 and 25. Their render times were 17.1, 21.2, 20.5, 17.6
and 17.2 seconds, but request-to-browser-readable times were 37.1, 47.8, 39.7,
36.7 and 37.0 seconds. Post-render actor-head measurement blocked projection
for roughly 18–25 seconds. Optional geometry should be considered for separately
owned, revision-fenced hydration after image publication; that change is not
implemented or approved by this playtest.

The Bandit's plain parley answer correctly used actor `bandit`, his portrait,
focus metadata and an over-shoulder camera request, without disclosing an
employer. The resulting new plate `a991d8c9...` nevertheless kept the gesturing
Courier dominant and the Bandit cropped. This is a confirmed framing defect,
not an actor-routing regression or a missing image update. The edit prompt's
composition preservation needs explicit speaker-change staging and actual
visual acceptance. Repeated edits also showed more pronounced skin/cloth
texture, a visual quality concern requiring separate review.

Brakka's look-around failed normally; Sigrun's successful Bandit watch returned
only "Bandit is here." No new clue was proved. Existing private notes predated
the watch and were not used in public prompts. Concrete packet inspection was
refused because the fixed scene still lacks its structured target. The Courier
avoided inventing a ship name, but sensory/proximity details in other replies
remain unsupported by a structured witness model. The apparent closer/unknown
position sequence is a grounding gap rather than proof of a hard temporal
contradiction. Source-linked NPC/world observation remains necessary.

The named dialogue panel stayed clear over all five plate replacements, with
17.51 px between it and choices. Check-card visibility was retained; no human
audio audition or continuous video capture is claimed. Runtime timeline loss
means the retained records are not a complete audiovisual trace. Agent evidence
is in `outputs/npc-scene-playtest-20261016/combined-playtest-report.md` and its
raw case/timeline files in the local testing chat. Operational CLI examples now
document `search --option look-around`, `watch-<cast-id>`, and positional scene
names; synthetic `room` is not a legacy finding selection. Awkward intention
copy ("Brakka searches look around") and generic interaction refusals remain
open UX/CLI issues. No implementation fix is claimed by this continuation.

### Roleplay action experiments — 2026-10-16

Observed on room SANAPU, build `portrait-ec334de7`, with an actual display
inspection after each input. Checkpoint `social-action-start` retains the starting
state. These are failures of the tested runtime; source repairs require a fresh
build and integrated acceptance before closure.

| ID | Priority | Reproduction and observed result | Required resolution |
| --- | --- | --- | --- |
| RPL-01 | 2 | `I take the packet from the Courier.` became a successful Stealth check, **Brakka scouts Courier**, and **Courier is here**. No packet transfer occurred. | Distinct physical intent and canonical object/holder binding. Resolve registered ownership/consent; otherwise explain the unavailable action. Never substitute scouting. |
| RPL-02 | 2 | `I attack the Courier with my greatsword, trying to kill him.` became social speech and a plea. Direct `attack --target Courier` was unavailable in dialogue. | Recognize attack separately from a spoken threat. Admit authored NPC combat through the session/encounter owner, then ordinary initiative, turn, equipment and attack rules. Unsupported targets need an explicit reason. No automatic kill or Bandit substitution. |
| RPL-03 | 3 | A one-GP packet offer received a sensible refusal; a direct threat received a fearful plea. Both left game revision unchanged. The fixed scene has no packet object model; both Courier feelings lookups failed. | Separate bargain/proposal, completed transaction, Influence willingness and persistent relationship consequences. Dialogue alone does not prove a sale, roll or state update. |
| RPL-04 | 2 | The old Jev menu contained speech and observation choices only. Physical requests were forced toward those categories. | Add bounded roleplay action candidates, native resolution and structured CLI traces. Preserve questions, jokes, hypothetical/conditional plans, quoted speech, partial input, ambiguity and stale/retry semantics. |

The test used `plan` context for the two physical declarations, following the
old observation examples. The repaired path must respect that declared context:
it may propose a pending action but cannot silently treat a nonexecuting plan as
permission to commit. Direct execution tests must also cover ordinary unspecified
input and explicitly selected speech contexts. The earlier runtime nevertheless
classified these as unrelated scouting/social speech rather than describing the
actual requested action.

A new merchant-shop mode is being added for realistic interaction coverage:
merchant, named adventurer and soldier, registered goods and prices. Scene/cast
presence in source is not proof that all actors render or can be addressed in
the live display. A purchase must record payment and actual held stock together;
successful prose alone does not satisfy that check. The baseline evidence is
`outputs/npc-hostility-playtest-20261016/baseline-observations.md` in the testing
chat; subsequent acceptance will record its exact candidate source and build.

### Merchant and multiplayer playtest — 2026-10-16 (rooms FURABO, VENOFU)

Observed live with the TV display and two player phones (Brakka, Sigrun) on builds
`merchant-limits-49b6c3eb` and `video-593c7e00-c1a` (source `4375a3ad`), plus server
logs, `dev ledger` and the table log. Server price bounds held in every case (floor,
buyback ceiling, sold out, selling another hero's item, rule-override injection).
Status reflects source commits; every fix still needs a live retest on a fresh build.

| ID | Priority | Reproduction and observed result | Status |
| --- | --- | --- | --- |
| MPX-01 | 1 | Accepting any merchant quote ("Fair enough, Marek. Two gold it is") was refused: `REQUEST invalid action_kind=1 choices=4` because `journey_input` capped every action at 3 choices; phone said "The native owner refused this". | Fixed in source (scene interactions bound 4, fights 3), commit pending |
| MPX-02 | 1 | Phone rejected a valid reply whenever the turn committed state ("does not match this dialogue scope") and kept the draft, gluing it onto the next line. | Fixed `ad5fee70` |
| MPX-03 | 1 | Phones hide the other hero's speech and the NPC replies to them; players cannot converse on their phones (the TV shows all lines). | In progress |
| MPX-04 | 1 | Haggling never produced a counteroffer: Marek ignored stated figures and asked "what are you proposing"; the model had no floor price or hero amount. | Fixed `4375a3ad`, `fe771e05` (article "a" was parsed as a second amount) |
| MPX-05 | 1 | A Hesitant or Unwilling NPC answers with canned lines ("I need a moment...", "I will not help you with that.") and no Influence check is ever offered; haggling, threats and blackmail dead-end. | In progress |
| MPX-06 | 2 | "I hand the traveler's clothes to Sigrun." was judged `sell_to_merchant`, the recipient resolved as an absent NPC and the turn failed silently. | In progress |
| MPX-07 | 2 | A transient authority outage (`UNAVAILABLE site=authority reason=UnresolvedCommit`, seats marked away after websocket idle) dropped a hero's line with `STORY input status=failed` and no message on the phone. | Open |
| MPX-08 | 2 | Every line-cleanup model call failed (`reasoning.effort=minimal` rejected by gpt-6); the local fallback capitalised ordinary nouns ("Fine Cloth", "this Shop") because the place name was split into word-names. | Effort fixed (pending commit); scene-name split in progress |
| MPX-09 | 2 | Display plate drifts painterly: each plate edit starts from the previous edit's output, compounding artefacts; fresh plates stay photographic. | In progress |
| MPX-10 | 2 | TV typing "..." bubbles accumulate per typed line and never retire (dropped intent runs emitted no Done). | Fixed in source, commit pending |
| MPX-11 | 2 | The quote bookkeeping row replaced Marek's spoken bubble on the TV and printed the internal id ("merchant offers..."). | Name fixed `d1cb7ef7`; bubble needs a TableKind for trade rows (open) |
| MPX-12 | 2 | Sold-out stock: Marek quoted "two gold" for clothes he no longer had. | Fixed `4375a3ad` (verified live: "I'm out of traveler's clothes") |
| MPX-13 | 3 | Narrator purchase line used the item description: "Marek hands over A practical set of traveler's clothes in sturdy wool and linen for Brakka". | In progress |
| MPX-14 | 3 | After blackmail (reaction uneasy, wary much) the table noted "Marek seems fond of Brakka." | In progress |
| MPX-15 | 3 | TV caption flashed the opening narration while a reply was pending; pending marker outlived the answer. | Fixed `00d33cb5` |
| MPX-16 | 3 | Other player's phone shows "The offer or scope changed. Your text remains editable." with an empty draft when someone else acts; Marek's greeting appears twice in the feed. | In progress |
| MPX-17 | 3 | Merchant reply classification runs for non-merchant NPC replies (Corin), wasting a Jev call per turn. | Open |
| MPX-18 | 3 | A hero's theft of the counter label failed silently on the pre-`747e841c` build. | Retest after rebuild |
| MPX-19 | 3 | A provider failure logs only `failure_category=provider_failure`, without status detail. | Open (shared provider code) |
| MPX-20 | 3 | The imagery cache path is built from the source root, ignoring `DUNGEONFLUX_ARTIFACT_ROOT`, so a run from another root regenerates every image at cost. | Open |
| MPX-21 | 3 | Cosmetic: "Fighter Â· level 1" mojibake on the hero sheet; the required portrait picker is an unlabelled "Choose..." select; the "Updating scene..." chip stays after the plate is ready. | Open |
| MPX-22 | 3 | `HELP_ATTACK` existed in the protocol and rules but had no df-tools offer, wire mapping or dev action. | Wired in source, commit pending |
| MPX-23 | 1 | Party notebook: Notes tab "Note it" shows "Noted." but the note never appears in Party notes on either phone and the server logs no notebook activity. | In progress |
| MPX-24 | 2 | "I write in our party notes: ..." typed in the dialogue box was routed as speech to Marek (`STORY say ... target="Marek"`), then no table line and no reply: the line vanished. Jev's party-note intent was not reached. | Diagnosis in progress (server side in dialogue.rs) |
| MPX-25 | 3 | `dev notes <hero>` still rejects heroes owned by a phone ("unknown hero"); the checkpoint read covers scene/view only. | Open |
| MPX-26 | 1 | Hero-directed actions fail: "I lean over and whisper to Sigrun so Marek can't hear: ..." -> `NPC target ... validation=absent reason=absent`, `STORY input status=failed`, nothing logged, no message. Vocative speech ("Sigrun, should we ...") works; whispering, handing, showing or gesturing to a hero does not, because the addressee resolver only knows NPCs. Whispers also need a private delivery path (party only, not the TV/NPCs). | Open |
| MPX-27 | 2 | After the lobby, a player's phone has no party roster: Sigrun's phone mentions Brakka nowhere (no name, status, HP or held items), so a player cannot see their companions without the TV. | Open |
| MPX-28 | 3 | Two players sending lines a moment apart works: both logged in send order and each NPC answered its own addressee (verified live; keep as a regression check). | Verified |
| MPX-29 | 2 | Influence checks (2024 Influence, DC 15) now resolve on the server with real dice, but the roll is never shown on the TV or phones, so players cannot tell a check happened. | Open |
| MPX-30 | 1 | Regression in build main-76788964: a plain price question ("What are you asking for the traveler's clothes?") and a direct offer got the stock "I can't give you a clear answer to that." with no model call. Jev's per-part focus maps the question to an authored-answer subject; with no authored answer npc_answer substitutes the stock line instead of letting the model answer with the merchant context. Same path answers "Do you remember me?" with an authored self-introduction. | In progress |
| MPX-31 | 2 | Lasting consequences are too weak: after a sword threat and blackmail Marek only went friendly -> indifferent with wary=much; resentment stayed none and never_forgives false. The state persists per hero (also across restart) and does not leak to Sigrun, but his replies stay cordial ("I appreciate you saying so") and do not remember what was done. Owner requirement: threats, blackmail and other bad acts must permanently change how that NPC answers that hero. | Open |
| MPX-32 | 3 | Reaction note for an attitude drop lacks its subject: "cools toward Brakka" (should name Marek). | Open |
| MPX-33 | 2 | After a server restart the table log (in-memory ring) is empty: phones and TV lose the whole visible conversation; only the scripted greeting remains. | Open |
| MPX-34 | 3 | Line cleanup with effort none still misses its 1.5 s deadline about half the time (`failure_category=deadline` at 1508 ms); the original words are used. | In progress (deadline raise) |
| MPX-35 | 1 | Private asides are not private: a whisper to a fellow hero ("I lean over and whisper to Sigrun so Marek can't hear: ...") is now routed as party speech, never to the NPC, but the only channel is the public table log, which the TV shows. Needs a party-only channel delivered to the speaker and the addressed hero's phones and kept off the TV and NPC context (remote-play rule: a TV never receives private whispers). | Open |
| MPX-36 | 2 | Resending a line after an unknown commit (authority briefly unavailable) generates a new input id per resend, so a line that did commit can be applied twice. Reuse the input id on retry (client) or dedupe raw lines by input id (server). | Open |
| MPX-37 | 3 | Without Jev, a line starting "Well, ..." (PossibleAddress) now answers "Nobody here goes by that name." instead of going to the NPC in focus; the keyless fallback should default to the focus NPC. | Open |
| MPX-38 | 1 | Phone reconnect is slow and leaky: after the server logs "STREAM ended ... websocket tunnel idle ... TimedOut", the phone shows "Reconnecting..." with a stale feed for about 30 s even when visible (the console fills with failed ws://.../gameplay/tunnel attempts while a fresh socket opens in 5 ms). "Say it" stays enabled meanwhile, a reload sits on "Rejoining your adventure" for about 20 s, and the reconnect re-render jumps the page scroll to the bottom. (An earlier note that sends were silently dropped was a test-harness misclick, not confirmed.) | Open |
| MPX-39 | 1 | An in-terms buyback rolls Influence: "would you buy these back for 1 gold?" (100 CP, inside Marek's 1 to min(100, purse) buyback terms) was read as a bargain, rolled Persuasion against DC 15, failed, and Marek hedged ("can't say without judging their condition") with no quote. A request inside the merchant's standing terms should get a firm quote with no check; only asks outside the terms are Hesitant. | Open |
| MPX-40 | 2 | A refused discount withdraws the standing quote: after "would you take 160?" Marek refused, and the 200 CP quote closed as merchant_withdrew. A refusal keeps the listed price open. | Open |
| MPX-41 | 2 | Haggling is inconsistent: a discount request to a friendly merchant (t03) rolled nothing, while the in-terms buyback (MPX-39) rolled Persuasion with advantage. A discount ask should run the Hesitant Persuasion check (advantage when Friendly), and success should let the merchant come down within the price bounds. Players never see the roll (MPX-29). | Open |
| MPX-42 | 2 | The hero's own line reaches the table 4 to 5 s after sending: STORY input waits on line cleanup (about 1.4 to 2.5 s) plus routing. Publish the raw line at once and swap in the cleaned text, or run cleanup alongside routing. | Open |
| MPX-43 | 3 | When the server log lacks it, the phone feed appends the authored scene greeting ("Welcome. Have a look around...") at the end of the feed, so it shows after newer lines (seen under Brakka's buyback line). Place it before the first server line. | Open |
| MPX-44 | 3 | Mojibake on the phone hero card: "Dwarf Fighter Â· Level 1" (a UTF-8 middle dot decoded as Latin-1). | Open |
| MPX-45 | 3 | fal kling-video image-to-video fails on every request (5 of 5, failure_category=provider_failure, about 20 s), with no provider error text in the log. Log the provider's status and body, and stop retrying a model that fails every time. Also /assets/ui/generated/scene-mareks-cloth-shop returns 404 on each phone load. | Open |
| MPX-46 | 1 | The merchant sells stock he no longer has: after Brakka bought Marek's only traveler's clothes, Sigrun asked for a set at two gold; the reply context still carried a buy term (buy_terms=1), Marek said "Yes, I charge two gold pieces for those", and on "I'll take them. Here's two gold" answered "That'll do. Thank you." The ledger correctly did not move, but the story says a sale happened. Sold-out items leave the reply context and an attempt is answered as sold out. | Open |
| MPX-47 | 3 | While a line is pending the phone says "Waiting for the online dialogue owner..." - internal wording; say what the player is waiting for (e.g. "Sending...") instead. | Open |
| MPX-48 | 1 | Party notes never reach the phones: Sigrun's "Note it" saved the note (notebook.json entry 4, author Sigrun, and the server answered "Noted in the party notes."), but 30+ s later both phones' notebooks still show "No party notes yet." People entries do show, so the note is lost between the notebook store and the projected/pushed journey notebook. | Open |
| MPX-49 | 2 | The note confirmation is attributed to the NPC: the composer's last-reply box shows "MAREK - Noted in the party notes.", and the echo reads "You: note: Marek is out of..." with the internal "note:" prefix. The whisper notice ("Whispers can't be kept private yet...") gets the same MAREK label. Show the confirmation as a system/narrator line and echo the note without the prefix. | Open |
| MPX-50 | 3 | A spoken give loses its words: "Sigrun, Marek's out of these. Take my traveler's clothes, they fit you better than me." produced only the narration "Brakka gives traveler's clothes to Sigrun." (the transfer itself is correct in the ledger). Keep the hero's spoken line on the table before the action narration. | Open |
| MPX-51 | 2 | Line cleanup strips or unbalances quotation marks, so action and speech run together: typed `I give Marek my warmest smile. "You have the finest eye for cloth..."` was shown as `I give Marek my warmest smile. You have the finest eye for cloth...`; another line lost only its closing quote. Keep the player's quotation marks. | Open |
| MPX-52 | 2 | The merchant ignores who holds an item: after Brakka gave his traveler's clothes to Sigrun, Brakka's buyback demand was still judged sell_to_merchant and Marek answered about buying "them" back from Brakka (asking to inspect them) instead of noting Brakka no longer has them. The ledger rejected it (item_unclear), but the reply should know the hero holds nothing to sell. | Open |
| MPX-53 | 2 | A question about a not-for-sale item goes unanswered: Sigrun flattered Marek and asked "Would you sell me that blue linen on the counter? I'd make it worth your while." (Jev: ask_price, approach bribe); Marek replied "I'd rather we deal plainly. Thank you, that's kind of you." without saying whether the linen is for sale. The merchant should say plainly that it is not for sale (it is shop stock with no offer). Also the dev CLI ledger lists no merchant stock although its help says it does; stock is only visible through `scene <hero>`. | Open |
| MPX-54 | 1 | Most physical, sensory and expressive acts have no route: sitting, throwing, breaking, hiding an item, lighting, wearing or trying on, pushing furniture, dancing, eating, praying, casting a non-attack spell, hugging, shoving, grabbing and punching all answer "Nothing like that is here to act on." The 13 Jev line intents (speech, examine, watch-person, keep-watch, track, study-recall, party-note, game-question, physical attack/item/transfer/trade) have no place for them. Probe round 1, main-f953b05a. | Open |
| MPX-55 | 1 | Senses and pointed questions collapse into a generic search: "What does the shop smell like?" and "I listen to the street" roll Perception and repeat the room or door description; touching linen rolls Investigation and gives the visual text; "Is Marek being honest?" rolls Insight and returns his appearance; "I wait quietly" rolls Stealth; "I follow Jessa to the door" scouts the door; "I step out into the market street" searches the door. The answer should address the sense or question asked. | Open |
| MPX-56 | 2 | Looking again rolls Perception and says only "You have already examined the room; nothing new turns up." A repeat look should re-describe the place with no roll. | Open |
| MPX-57 | 2 | Template narration is ungrammatical: "street door leads to the market street outside.", "Sigrun opens street door.", "You are not holding folded paper label.", "folded paper label belongs to someone else; ask its owner first." (missing articles and capitals). | Open |
| MPX-58 | 2 | The hero's own words never reach the table for actions or examinations; only the roll and narrator rows appear. | Open |
| MPX-59 | 2 | Handing coin to another hero ("I hand Brakka two silver pieces") and throwing an item at someone (read as give) fail with "interaction was refused" and no narration. | Open |
| MPX-60 | 2 | NPCs lack local knowledge: Marek cannot name the nearest inn and refuses to repeat any market rumour; Corin deflects where he is headed. NPCs should know plausible local facts, invented once and kept as canon. | Open |
| MPX-61 | 2 | A lie gets no Deception check: "I'm a tax inspector from the city. Show me your ledgers." (approach deception) drew "That does not sound right to me." with no Deception against Insight. | Open |
| MPX-62 | 3 | A free item owned by the shop cannot be read or picked up ("I pick up the folded label and read it" was refused as owned); reading an item's text without taking it has no route. | Open |
| MPX-63 | 1 | The hero's own gear is not a target: "I count the coins in my purse" -> "You see no coins in my purse here."; "I check my greatsword's edge" -> "You see no my greatsword's edge here."; drawing or sheathing a weapon and juggling javelins answer "Nothing like that is here to act on." Probe round 2. | Open |
| MPX-64 | 1 | Coin to an NPC fails: tipping Marek a silver, paying a silver for information, and buying gloves for another hero are refused ("interaction was refused" or no route). Gifts of coin to an NPC also do not credit the NPC purse (conservation gap found in the code survey). | Open |
| MPX-65 | 2 | Showing an item to an NPC ("I show Marek my chain mail and ask about the stitching") is read as "share" and refused. | Open |
| MPX-66 | 2 | NPC pockets and packs are not reachable: pickpocketing Marek's coin purse (steal) and planting the label in Corin's pack (give) are refused. | Open |
| MPX-67 | 2 | Services have no mechanics: "Could you take in my sleeves? I'll pay for your time." gets a deferral ("Let me see the sleeves...") with no price, offer or payment path. | Open |
| MPX-68 | 2 | Moving within the scene is read as a search: "I walk up to the counter" and "I jump to touch the beam" search the counter or door; "I climb onto the counter" has no route; "I duck behind the rack" rolls Stealth but describes the rack and records no hidden state. | Open |
| MPX-69 | 3 | "I write a note thanking Marek and leave it on the counter" becomes a party note instead of an in-world note left in the scene. | Open |
| MPX-70 | 1 | Merchant knows only one price: with nine offers and four services stocked (e1a8e3b7, f81490e6), "what have you got for sale?" lists only the traveler's clothes, and Marek "can't name a price" for the cloaks, gloves or a bulk deal, and cannot sell the gloves for another hero. The merchant reply context does not carry every available offer and service with its price. Probe round 3, main-f81490e6. | Open |
| MPX-71 | 1 | Open-ended routing regressions (969290e7): "I open the door to the market street wider" -> "refused because there is no target", "I close the shop door" -> "nothing like that here to move" (both worked before); "I sit on the edge of the counter" -> "moves the wooden counter. Marek takes it badly."; "I laugh at Brakka's joke" -> "listens to themself"; dropping a label the hero does not hold succeeds; "I throw the folded label at Corin" still routes to give and is refused. | Open |
| MPX-72 | 2 | Freeform refusals use machine wording: "refused because there is no available target", "the spell is refused as unknown", "cannot write the note because no target is specified". Refusals must be in-world sentences. Writing a note to leave on the counter should simply happen. | Open |
| MPX-73 | 2 | Thin freeform answers: "I stop and listen to the street outside." -> no sounds; "I look at myself in the fitting mirror" -> no reflection; "I walk up to the counter" -> "Brakka moves forward."; a failed Insight on Marek repeats his appearance instead of "you can't tell". Pronoun "themself" used for Sigrun. | Open |
| MPX-74 | 2 | Reading is misread as taking: "I pick up the folded label and read it" runs a theft check ("fails to steal folded paper label, and Corin notices"). Reading visible text needs no custody change. | Open |

## Work in flight and open findings, 2026-10-17

Everything being worked on or found during the 2026-10-17 session, written down so it survives a crash.

- **WIP rows** are workstreams with uncommitted work in the shared working tree. Each lists the files it owns.
- **NEW rows** are findings that have not been fixed yet.

Commits landed this session up to `32bcad49`:
- `e1a8e3b7`: stock and theft
- `969290e7`: open-ended acts
- `aaa6a17f`: local canon, Deception vs Insight
- `f81490e6`: coin and goods movement
- `263f6d26`: NPC-reply leaks
- `d1249ac0`: merchant quotes every item and service, read without taking, template grammar
- `fb6bfd93`: fights with any shop NPC
- `560b4afd`: doors, freeform misreads, refusals, thin answers
- `32bcad49`: failed Insight, throwing is not giving

Owner decisions recorded this session:
- Art is photographic only. Painted references are deleted. Missing depictions (monsters, villains, patrons) are generated live when first needed, never ahead of time.
- No keyword tables anywhere, not even as fallbacks. Jev or an LLM decides meaning; without Jev the result is neutral and audited.
- Every system has logs in the dev CLI, plus create/read/update/delete where feasible.
- Every generated resource goes through one contract: ResourceSpec, then ResourceKey, then the hub's get_or_request. The hub owns the spend ledger, single-flight, the RESOURCE audit line and the dev-CLI hooks.
- Splat worlds (World Labs Marble): the narrative engine flags from the conversation that a fight is likely, early enough to generate the world in time. It also decides where the fight happens (inside the shop, or moved outside).
- The DM remembers everything, or everything relevant, to keep the story consistent. Story memory uses OpenAI embeddings and pgvector in the game's own Postgres.
- The landing display plate becomes a gentle ambience video loop: the same still is the first and last frame, and the still crossfades to the video in place. Try cheaper fal models.

| ID | Sev | Item | Status |
|---|---|---|---|
| WIP-01 | 2 | Photographic-only art contract and the single resource hub. Painted UI art is deleted from `assets/ui/generated`. The universal resolver is `df-render/src/art_contract.rs`; the hub is `gameplay/resources.rs` plus `resource_adapters.rs`. Also touches imagery/*, living_scenes/plates*, the battle-scene content JSON, the proto `floor`/`interior` fields and staging.rs. | In progress (agent) |
| WIP-02 | 2 | Place music on demand. Replaces the bed_of keyword table, which gave every place the night harbour score. A MusicSpec is chosen by content or Jev, and ElevenLabs generates the music through the hub. Files: `gameplay/place_music*`, `df-providers/src/eleven_music.rs` and its test. | In progress (agent) |
| WIP-03 | 2 | Keyword removal, NPC conversation (K1): npc_reply.rs, npc_room.rs, and local_lore role knowledge (now a Jev role-domain choice). Leftovers handed over: the dead `classify_keywords` branches near npc_reply.rs:1901 and :2022, npc_room.rs:275, and the CLAIM_WORDS/RULE_WORDS/RUMOUR_OPENERS lists in local_lore check_text. | In progress (agent) |
| WIP-04 | 2 | Keyword removal, commerce (K2). Done but not verified. The merchant_bargaining phrase lists are deleted; Jev `judge_trade_item` now picks the item; living_scene interactions use Jev gift/give and a recipient choice. Files: journey/merchant_bargaining.rs and its tests, journey/living_scene.rs and its tests, adventure/shop.rs. Asking about several items at once is being restored with one LLM multi-pick (owner: "switch to an llm in this context"), owned by WIP-05. | Waiting on WIP-05 |
| WIP-05 | 2 | Keyword removal, line routing (K3). Done in approach_model, dialogue, investigation, freeform_action and line_cleanup, with a source-scan test. It now also owns the dialogue.rs call sites for K2's new signatures, and verifies K2 and K3 together on a clean HEAD copy with all files LF. Next: replace the scene_model name resolvers (`resolve_target`, `resolve_item`, `service_for_line`, `person_for_line`) with Jev picks. | In progress (agent) |
| WIP-06 | 3 | Dev CLI reads and writes for the new systems: `dev lore` (with add), `dev social`, `dev fight`, `dev scene-changes`, `dev economy`, `dev audit`, `dev decisions`, `dev music`, and the market and offers in `dev ledger`. Files: dev_cli.rs, dev_control/{mod,parse,reads,timeline,audit_log,economy,fight,lore,music,social_view}.rs, plus hunks in local_lore.rs (visibility and `development_facts`), living_scene.rs (`scene_change_journal`) and gameplay.rs (`lore:`). Verified on a clean HEAD copy: dev_control 66 of 67 pass (the one failure also fails at HEAD), dev_cli 18 of 18, dev_write_request 2 of 2, bin and wasm build. | Committed `0dbae2c9` |
| WIP-07 | 2 | Phase-2 hub: moves video, voice, text and speech-to-text onto the hub, and fixes scene-clip charging and keys and the voice key (which omits model, format and language). Owns home_video.rs, voice.rs, storyteller.rs, speech_input.rs, and df-providers native_http and speech_adapter. It has been asked to log the provider's status and body on failure, and to build the HomeLoop spec with the same still as first and last frame and an ambient prompt. | In progress (agent) |
| WIP-08 | 2 | PLY Gaussian splats and a walkable grid from the World Labs collider GLB, in df-render: battle/scene_file.rs, stage.rs, layout.rs, hud.rs, gpu/renderer.rs. Research and proof files are in `artifacts/tmp/worldlabs`. Next: a SplatWorld hub resource kind, plus narrative-engine Jev/LLM judgements for fight likelihood (when to generate) and arena choice (where). | In progress (agent) |
| WIP-09 | 3 | Meta SAM 3.1 research: segmentation for cut-outs, splats and masks. A CPU torch run is in progress. | In progress (agent) |
| WIP-10 | 2 | Automated end-to-end voice-input test using a generated voice clip. Its `dev speech` command design feeds the dev CLI. | In progress (agent) |
| WIP-11 | 1 | DM total recall (story memory). Every table line, ruling, roll, narration, canon fact, grievance, promise and scene change becomes an append-only `story_memory` event in Postgres, embedded with OpenAI text-embedding-3-small into pgvector (HNSW) alongside full-text search. Before the DM writes, recall returns pinned facts, related events and per-entity dossiers. Jev judges contradictions; superseded events are marked and kept, never deleted. `dev memory` reads it, and spend goes through the hub. Research: `artifacts/tmp/vector-memory-research/report.txt`. Wiring recall into the storyteller, NPC reply and freeform prompts waits for WIP-03, WIP-05 and WIP-07 to land. | In progress (agent) |
| WIP-12 | 2 | pgvector, built from source (MSVC, nmake) for the bundled EDB Postgres 18.6, which ships without it. Pinned as pgvector 0.8.7 in `development/pgvector.sh` (EDB 18.6 x64, runs under emulation on ARM64) and proven on a scratch cluster, never the durable one. | In progress (agent) |
| WIP-13 | 2 | Landing ambience loop. The landing still is `assets/ui/scenes/mara-harbor-v4.png`. Diagnosis and scratch tools are in `artifacts/tmp/video-diag`: `fal.py`, `call1_mirror.py`, and `seam.py` (seam checker and contact sheet). Recommended models: `fal-ai/wan-flf2v` at 720p (about $0.40 per clip) or `fal-ai/pixverse/v5/transition` (about $0.20). Kling o1 is about $0.56 per 5 s. | Blocked on NEW-01 |
| NEW-01 | 1 | The fal key in `.env` (`DF_FAL_KEY`, the only fal key on the machine) is rejected: queue submit, pricing and status calls all return `401 invalid key credentials`. No video call has ever succeeded here, and there is no mp4 anywhere under artifacts. The owner needs to supply a working key. | Open (owner) |
| NEW-02 | 2 | The home-video prompt works against the loop. HOME_MOTION asks for "a slow, almost imperceptible optical push-in", but with the same still as the last frame the camera has to push back out. Make it fully locked-off. FILM_LOOK asks for "real adult performers, skin pores" in an empty boat shed; ambience loops need a landscape-only look clause. | Open (WIP-07) |
| NEW-03 | 2 | `run_job` / `validate_output` in home_video.rs require exactly 1920x1080, so a clip of any other size is paid for and then discarded. Kling, Wan and PixVerse all return smaller sizes. Accept the real output size. | Open (WIP-07) |
| NEW-04 | 3 | Three request problems: the same 712 KB still is sent twice as data URIs (about 1.4 MB body), so upload it once and reuse the URL; a submit error such as 401 is reported as `QueueIdentity` instead of its HTTP status; and `with_video_model` accepts only two endpoints. | Open (WIP-07) |
| NEW-05 | 2 | Two TV loop problems. First, the still runs `df-drift` (scale 1.02 to 1.07) and the lobby filter, so the fade to video jumps in framing and grade. Copy the computed transform and filter onto `.tv-loop` and pause the drift when the fade starts. Second, the swap runs only from requestAnimationFrame when `near_end && !paused`, so in a throttled tab the clip ends and the loop freezes for good; add an `ended` listener. Fade in only after the first frame has painted, over 1.2 s ease-in-out. Files: gameplay_browser/live_home_video.rs, live_tv.rs, CSS. | Open (client, after WIP-01) |
| NEW-06 | 2 | After K2, `coin_amount_cp` reads "a gold ring" as 1 GP, and this feeds the gift path. Parse a coin amount only when Jev says coin is being handed over. | Open (WIP-05) |
| NEW-07 | 3 | living_scene.rs ~3577 classifies an offer by `name.contains("ale")`. Use a structured content field instead. | Open (WIP-05) |
| NEW-08 | 3 | Freeform adjudication writes no audit line on success; only `FREEFORM plan=unavailable` is logged. Add an audit line with the act, target, roll and consequences (freeform_action.rs, dialogue.rs). | Open |
| NEW-09 | 3 | Dev CLI writes not supported yet. Lore edit/delete and social edits need ledger mutators. Fight restore/end and purse, coin and offer edits go through the override path, which stays off until the milestone 3 projection-safety proof. `dev art` and `dev music regenerate/clear` wait for WIP-01 and WIP-02. | Open |
| NEW-10 | 3 | Two failures already present at HEAD: `dev_control::layout::tests::visibility_round_trips_without_resetting_transforms` panics with `Shape` (layout.rs:747), and `adventure/expression_tests.rs` is not rustfmt-clean (lines 1 and 576). | Open |
| NEW-11 | 3 | Line-ending churn: several workers saved CRLF copies of LF files. The repo is LF (`.gitattributes` eol=lf, rustfmt newline_style=Unix), and every commit must be LF. | Open (process) |
| NEW-12 | 3 | Disk is tight: 13 GB free on the afternoon of 2026-10-17, after hitting 100% earlier. Prune each agent's build directory under `artifacts/build` as it finishes. | Open (process) |
