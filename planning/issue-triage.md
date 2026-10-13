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
| SES-01 | 1 | Pressing choices quickly broke a room for good. Two choices under a second apart, the second pressed three more times at 0.9 s. The server then answered everything with `UNAVAILABLE site=authority reason=InvalidCandidate` (73 times in two minutes), TV and phones showed "The connection to the game was lost", and the room never recovered. Not yet known whether the trigger is the repeated press or choosing before the previous step settled. | SR (BOSAKA) | Part `5abd72e`: the refused fight behind it is fixed; why a refusal left the room dead is still open |
| SES-02 | 1 | The Hockley Valley scan (`scan-valley-bridges`) cannot start a fight against bandits: every press logged `REFUSAL action=DefendCourier reason=InvalidCandidate`. The same seed (0xb2563e1a9bd2a1a0) on the village scan started the fight at once. Cause unknown: every error on the fight-start path is logged as the same `InvalidCandidate`. | SC (KENOKA, SUFARO) | Fixed `5abd72e`: the cause was the order heroes were confirmed in, not the scan (HNT-01). Verified on a running server |
| SES-03 | 2 | When a fight cannot start the players are told nothing; the choice stays on screen and can be pressed for ever. | SC | |
| SES-04 | 1 | When both phones went away mid-fight the game stayed on "Brakka's turn" with no time limit seen. A new browser opening the room was offered only a fresh join, with no way to take the abandoned hero back. | SC (ZATOZO) | |
| SES-05 | 2 | Start-up story text missed its 25 s deadline in 3 of 5 starts this evening and in playthrough 2. The session then runs on filler text for every line. Nothing says so on the TV, the phones or the lobby, nothing retries, and the lobby opens regardless. | P2, SC | |
| SES-06 | 3 | When the text does arrive, some of it is refused by validation and replaced by table text: 17 of 54 slots in playthrough 1, 19 of 55 in playthrough 3, 11 of 55 and 3 of 54 in later starts. | P1, P3, SR, SC | |
| SES-07 | 3 | A choice had to be pressed twice: the first press showed "Waiting for the game server…" and the same choice was offered again. | SR (ZODAPO) | |
| SES-08 | 2 | A failed start-up leaves `df_game` and `df_local_demo` behind, so the next start refuses with `retained_access_unavailable`. | EARLIER | Fixed `7d0f493`: a failed start-up releases its session |
| SES-09 | 4 | "SESSION lease renewal failing (Unavailable); retrying" logged once in a session's first minute; it recovered. | SC | |
| SES-10 | 4 | With the same seed the story text differs between starts (two different titles for one seed), so a seed does not reproduce a session's wording. | SC | |
| SES-11 | 3 | The battle scene drives the machine hard enough to throttle the processor ("its downclocking the cpu"). Sessions run the scan at level of detail 1 only because the session launcher sets it; the server default is still 0. | OWNER | Fixed `97f9d1e`: default level of detail is 1 |

## 2. Story flow and pacing

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| FLO-01 | 2 | The whole story before the fight can be skipped in 1.6 s: three presses 0.3 s apart with no line shown or spoken. Nothing holds a choice until its scene has been presented. | SR | |
| FLO-02 | 2 | The whole game is five screens and about ten minutes: opening, one follow-up, the fight, the aftermath, the epilogue. Three of four choices have a single option. No journey, no investigation, no use for the private clue. The lobby promises places that do not exist. | P1, P2 | |
| FLO-03 | 2 | Owner direction, not built: the story should branch off what the players learn. New information opens leads the player can review on the phone; leads read as hints, not quest markers; players may follow them or go their own way. A plan exists (`artifacts/tmp/leads-a1/plan.md`, four milestones). | OWNER | |
| FLO-04 | 3 | Talking changes nothing that is offered: the choices are the same after a conversation, and a rumour that names the antagonist opens nothing to follow. | P1, P2, IV | |
| FLO-05 | 3 | The only choice before the fight names an enemy nobody has seen ("Face the giant weasel") or reacts to no visible threat ("Stand your ground"). | P1, P2 | |
| FLO-06 | 3 | Choices after the fight are chapter titles, not actions ("The Road Takes Its Toll"), and the phone mangles them into sentences: "Brakka thes Road Takes Its Toll", "Brakka calla'ses Thanks". | P1, P3 | Part `67b9586`: the log no longer mangles titles; the choices are still titles, not actions |
| FLO-07 | 3 | Rules steps are offered as story choices: "Advance 6 seconds · death saves", "Wait · a stable hero regains 1 hit point after 1d4 hours", and "Short Rest · 1 hour, no Hit Point Dice" offered first, twice, to heroes at full hit points. | P1, P2 | |
| FLO-08 | 4 | The title card's sentence is repeated word for word in the Dungeon Master box straight after it, and one title card stayed up for at least 18 s while a long line was read. | P1, P2 | |

## 3. Story writing and consistency

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| STO-01 | 2 | The plot does not hold together when the filler text is used: the job is to collect a debt from the man who holds the ledger, who then "takes the ledger"; a named villain never appears; a secret is stated and never explained. | P2 | |
| STO-02 | 2 | The opening gives no job, stake or reward. | P2 | |
| STO-03 | 3 | The fight's introduction names one villain with one weapon and the fight fields three generic enemies with another; the loading card says one enemy springs out and there are three; the victory text blames a villain nobody met. | P1, P2, P3 | |
| STO-04 | 3 | Narration contradicts the picture and the events: "that was close" after a fight where nobody was touched; a character "steadies herself behind you" over a picture of her calmly at work; the party is "at the apothecary" and is then offered "Continue to the apothecary". | P2 | |
| STO-05 | 3 | The defeat ending contradicts itself: the patron "is dragged away", the epilogue never mentions her, the end text has her starting again. | P1 | |
| STO-06 | 3 | One leftover sentence is glued onto the fight's introduction, the victory text, the journey's-end narration, the epilogue and the end text. | P2 | |
| STO-07 | 3 | Epilogue lines are shared across adventures word for word, spoken by different characters in consecutive games. | P3 | |
| STO-08 | 3 | The private clue runs narration straight into first-person speech with no quotation, and it is repeated in the phone's story log on every later screen. | P1 | Part `d4246c4`: the phone shows the clue once; the missing quotation marks are not done |
| STO-09 | 3 | A character brings up a plot element unprompted and then cannot say who or what it is; characters do not know the plot. | EARLIER, P1 | |
| STO-10 | 2 | A private-clue reply can leak onto the TV, because privacy is decided from a term list and not from what the table has been told. | EARLIER | |

## 4. Conversation with characters

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| DLG-01 | 2 | Nothing a player says or does moves a character. After flirting, propositions, a drawn sword, threats and an assault the patron stayed friendly; no roll was asked for; the scene did not react. Moods, attitude changes and the Influence roll are not wired. | IV, P1, P2 | |
| DLG-02 | 2 | Replies still fall back to stock text. In playthrough 2 half the replies were rejected by validation (`reply_rejected`), and the fallback for an accusation was the patron's opening line word for word. Provider failures (`call_failed`) have no retry. | P2 | |
| DLG-03 | 3 | Characters cannot answer ordinary personal questions (children, home, how they know the others). Replies pad with unrelated stock lines and repeat a term of address in every reply. | IV, P1 | |
| DLG-04 | 3 | The patron cannot say what her own errand's object is or why its deadline matters. | P1 | |
| DLG-05 | 3 | Asked what the job pays, the patron gave three unrelated sentences; the next scripted line then promised repayment. Scripted lines and conversation do not share what the character knows. | P2 | |
| DLG-06 | 3 | A reply answers only one of several questions asked together. | EARLIER | |
| DLG-07 | 3 | The same stock lines come back across answers ("it has to arrive before dawn", "I took it on trust"). | EARLIER, IV | |

## 5. Fight: turns and rules

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| FGT-01 | 2 | A fighter sometimes cannot attack after moving next to an enemy. Intermittent: it worked on the first turn in two later runs; in room ZODAPO the server's own record shows him moving next to an enemy twice and making no attack. In playthrough 3 he never attacked on his own turn. | P3, SR | |
| FGT-02 | 2 | Only the d20 is rolled and shown. Damage appears as a finished number and the server's combat record holds no damage dice ("he rolls hit but doesnt roll damage"). | P3, OWNER | |
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
| SYN-01 | 2 | The TV skips enemy turns. In playthrough 1 six beats were resolved before the first hero turn; the TV showed the first, waited about 25 s and jumped to the sixth, so the bites that hurt the heroes were never seen. In playthrough 2 a whole enemy turn was marked skipped ("the 3rd creature didnt even get a chance to play"). In playthrough 3 two of 26 beats were skipped. | P1, P2, P3, OWNER | Fixed `8d5425b`: enemy turns seen in order in a three-enemy recording; beat-by-beat proof pending |
| SYN-02 | 2 | A defeated enemy acts again on screen: "it said enemy vanquished then he stood back up", "the dead guy moved". Consistent with the TV applying a death and then playing an older queued beat for the same figure. | OWNER (twice) | Fixed `8d5425b` (browser check pending) |
| SYN-03 | 2 | The phone offers a hero his turn while the TV is still several beats behind (beat 6 of 6 on the phone, 2 of 6 on the TV); the TV's own counter went 6, 0, 2 during the entrance. | P1 | |
| SYN-04 | 2 | The TV's health cards are wrong: hurt before any bite was shown, then full for 26 s while both phones said hurt, then hurt again. A heal on the phone (4 to 10) did not reach the TV for two rounds. | P1, P3 | Fixed `0bf7237`: heals wait for their beat; cards running ahead of damage beats still to confirm in a browser |
| SYN-05 | 2 | The phone keeps showing "YOUR TURN" with Attack, Move and End turn enabled for up to ten seconds after the turn is spent. Presses are refused ("That offer has ended") or silently dropped. | P1, SR, SC | Fixed `0761a5b` (browser check pending) |
| SYN-06 | 3 | The victory fanfare, the scene transition and the ending music start before the last blow is shown (3.4 s early in one fight; before the last arrow was loosed in another). | P1, P2 | Fixed `4570a60` for the finale music (not yet heard); the fanfare and scene transition are still to confirm |
| SYN-07 | 3 | The turn label flickers between names within seconds and can run one to two turns behind the server. | P1, EARLIER | |
| SYN-08 | 3 | Heroes wait 11 to 15 s at a time for the TV to finish enemy turns; in the fastest possible game the fight was 2 min 56 s of 3 min 18 s. | SR | |
| SYN-09 | 3 | The turn strip looks out of order ("the character icon order seems broken"): the highlight jumps because skipped enemy turns are never shown, dead combatants stay as unlabelled dim circles, and the phone lists the party first instead of turn order. | P1, OWNER | |
| SYN-10 | 3 | The TV asked "Brakka: Opportunity Attack?" with Brakka off screen. | P3 | |
| SYN-11 | 3 | Enemies that only moved are shown walking after the attacks instead of in initiative order. | EARLIER | |
| SYN-12 | 3 | In one four-enemy victory the celebration ran 107 s with two fanfares. Not investigated. | EARLIER | |

## 7. Fight presentation: dice, camera, banners

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| FPX-01 | 2 | No dice roll is ever seen. Rendered tumbling clips exist (`assets/ui/generated/dice/`), but the game shows a still image with the number for 0.4 to 1.8 s, with a gold square outline round it. Death saves were rolled five times in one fight and never shown at all. Before this the owner's complaint was the opposite: "the old dice roll stay on screen way too long". | P1, P2, P3, SR, OWNER | Part `0d8934f`: the tumbling clip now plays (seen in a recording); the die is only about 40 px across and death saves are still not shown as rolls |
| FPX-02 | 2 | The camera can sit behind scenery: for a finishing blow it stayed behind the stockade fence, which filled the screen. Owner: "the camera needs an xray feature … the player characters need to be cut out so they can be seen". | P1, OWNER | Fixed `7a3d505` (browser check pending) |
| FPX-03 | 2 | In the default view a hero is 30 to 40 px tall on an 800 px wide screen and most of the frame is empty ground. The actor-centred camera the owner asked for is designed but not built. | P1, P2, SC, EARLIER | |
| FPX-04 | 3 | Bad frames: one near-black blurred frame mid-fight, heavy smear with colour fringing during strikes, and a whole-battlefield motion blur during a camera move that reduces the figures to streaks. | SR, SC | |
| FPX-05 | 3 | The Dungeon Master says nothing between beats or while the game waits on a player; waits of 20 s and more are silent. Owner: "if the match idles the DM should make some more commentary, he tries to keep intensity up". Battle cries and fight narration were decided earlier and never built. | P3, OWNER, EARLIER | |
| FPX-06 | 3 | A thrown javelin is played as a bow shot, with bow sounds ("that was 2 bow strikes in a row"). | P2, OWNER | |
| FPX-07 | 3 | "FINISHING BLOW / ENEMY VANQUISHED" takes the whole screen for the first of three enemies. | P1 | |
| FPX-08 | 3 | The initiative roll-call draws one enemy's tag rotated with its number upside down, and the banner overlaps the tags. | P2 | |
| FPX-09 | 3 | Unlabelled round icons float above figures and under health cards; a downed hero is drawn as a dark spiky heap. | P1 | |
| FPX-10 | 3 | The fight is titled after the story place whatever battlefield is loaded: "THE SALT QUAY" over an autumn riverbed. | SC, EARLIER | |

## 8. Battlefield scans

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| SCN-01 | 2 | Figures walk through the scan's fence; the walkable grid is not derived from the scan. Owner: use "the voxel strategy from the supersplat or playcanvas repos" to block cells. Needs a research pass first. | P3, OWNER | |
| SCN-02 | 3 | Of the four scans only the village has been played to an ending. The riverbed works as a battlefield but was played only to round 2; the summit has not been tried; the valley cannot start a fight (SES-02). | SC | |
| SCN-03 | 3 | The every-scene fight test stages only kobolds, so a scan that fails for another creature passes it. | SC | |

## 9. Sound

Nobody but the owner has heard the game this evening; the recordings are silent. Sound items come from the TV's own logs unless marked OWNER.

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| AUD-01 | 2 | Music stays ducked for as long as any text is on screen, spoken or not: 31 s during a wait for a player, and for the whole of the defeat screen and the end card. The owner's rule is that music ducks under every voiced line (player, character, narrator) and comes back after. Ducking under a voiced line works. | P1, OWNER | Fixed `e995a87` (not yet heard) |
| AUD-02 | 2 | On the defeat screen the narrator's line is hidden under the text and the closing paragraph is not read aloud ("I dont hear the DM narrating"). | P1, OWNER | |
| AUD-03 | 3 | A bite sounds like a sword: each one logs `sword-swing` then `blade-hit`. | P1 | |
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
| TVX-02 | 3 | The hero's own typed line is a faded bubble over the other character's figure, partly under the reply. | P1, EARLIER | Fixed `a1b3e25` (browser check pending) |
| TVX-03 | 3 | On every scene change the TV shows the adventure's title again instead of the new scene's, and during the crossfade it sits over the new picture and the health cards. | P1, EARLIER | Fixed `3720b36` (browser check pending) |
| TVX-04 | 3 | The adventure title and place stay printed large over the picture for the whole scene. | P1 | Fixed `3720b36` (browser check pending) |
| TVX-05 | 3 | A rules log line is shown as a hero's speech bubble: "BRAKKA: has been stable for 1 hours and regains 1 hit point". | P1 | |
| TVX-06 | 4 | The phone's label for a choice differs from the TV's ("Continue to the apothecary · Brannock's Apothecary" against "Continue to the apothecary"). | P2 | |
| TVX-07 | 4 | The speaker's bubble is drawn over the rules note and title; with choice chips off the narrator box stays empty while the table waits. | EARLIER | |

## 12. Phone

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| PHN-01 | 2 | The fight screen is developer drop-downs with dozens of cell and weapon options. The tap-to-move map the owner decided on has no brief yet. | EARLIER, all | |
| PHN-02 | 2 | A downed player's phone says nothing about dying or death saves and offers nothing to do. | P1 | Fixed `0761a5b`: death-save progress shown (browser check pending) |
| PHN-03 | 3 | Developer notes are shown to players: "Disclosed custom rule tactical.cover-centre-line…", "Disclosed gap giant-weasel: Climb Speed 30 ft is not modelled", "Greatsword Dropped; ownership retained", "defeat does not restore hit points or held weapons". | P1 | |
| PHN-04 | 3 | Saying your own line is awkward and mislabelled: the send button reads "Ask for clarification" under two drop-downs reading "Uncertain intention" and "No action selected", and the character's reply is shown with a different portrait from the scene's. | P1, IV | Part `a252a22`: labels fixed; reply portrait still differs |
| PHN-05 | 3 | "Confirm original strike" is offered from the aftermath to the end of the game and never explained. | P1 | Fixed `8fa68c8`, `0761a5b` (browser check pending) |
| PHN-06 | 3 | The fight's log line is carried into the story log of the next scene. | P2 | |
| PHN-07 | 3 | The hero portrait is cropped to the top of a head, and an unstyled status line sits over the background picture. | P1 | Fixed `0ad7d67` (browser check pending) |
| PHN-08 | 4 | The "Ask" button re-renders while a voice line plays, so a tap can miss. | EARLIER | Fixed `a252a22` (browser check pending) |

## 13. Lobby and character creation

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| CRE-01 | 3 | Only Dwarf, Fighter and Soldier exist, so every party is the same. The form is about 7,100 px tall with twelve groups, four of which offer a single option under "Choose one offered option". | P1 | Part `e8a800f`, `1d664bd`: single options read as facts; only one species, class and background exist |
| CRE-02 | 3 | The field "A detail that makes you memorable" is in the page but collapsed to zero size; it cannot be seen or used. | P1 | Fixed `edece58` (browser check pending) |
| CRE-03 | 3 | Developer wording is shown to players: "every submitted build is validated on the server", "Fighter B + Soldier B", "Other lawful SRD combinations remain unsupported", "Initiative tie agreement". | P1 | |
| CRE-04 | 3 | The phone's join page is titled "Gather at the Lantern Wharf" with a harbour picture whatever the adventure's place is. | P1 | Fixed `301391e`: neutral title; the real place needs a server field (browser check pending) |
| CRE-05 | 3 | The TV lobby shows neither the adventure's title nor what happens next or who starts; "THE PARTY IS GATHERED" is small grey text in a corner. | P2 | Fixed `6b5c691`. Seen in a browser |
| CRE-06 | 3 | The TV lobby prints "Or enter the code at 127.0.0.1:63319", an address no phone can reach, with no hint that the session is local only. | P1 | Fixed `6b5c691`. Seen in a browser |
| CRE-07 | 4 | The sticky "Confirm your hero" bar covers the form text beneath it. | P1 | Fixed `edece58` (browser check pending) |

## 14. Endings

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| END-01 | 2 | The defeat screen is not about this fight: two generic sentences with nothing about the enemies, who fell or how ("the defeat screen isnt contextual"). | P1, OWNER | |
| END-02 | 2 | After "The End" nothing on the TV or the phones offers a new game or a way back to the lobby; the phone still shows the composer. | P1 | Part `301391e`: phone shows the end state; no new-game path yet |
| END-03 | 3 | The end card leaves the health cards and the last speaker's line visible under it, the scene picture changes behind it, and it can appear before the last line has finished. | P1, EARLIER | Part `3720b36`: cards, bubbles and choices hidden; picture changing behind the card not done |
| END-04 | 4 | Older victory-card faults (cut-out figures over a dark plate, text cut off, a bubble in mid-air) were not seen again: the victory screen was clean in playthroughs 2 and 3. Confirm and close. | EARLIER | |

## 15. Tools and tests

| ID | Sev | Issue | Seen in | Decision |
|---|---|---|---|---|
| DEV-01 | 3 | The development command line cannot inspect or take over a session started from browsers: `dev health` and `dev state` answer "no seat has joined yet" while two players are in a fight. Health must not need a seat. | SC | Fixed `7526b4d` |
| DEV-02 | 3 | Refusals on the fight path all log `InvalidCandidate`, so a log cannot say which step refused (see SES-01, SES-02). | SR, SC | Part `65ff23c`, `d495f61`: the refusing step is named on the story, fight-start and enemy-turn paths |
| DEV-03 | 3 | `cargo test -p df-tools --lib` keeps nine long-standing failures, and the home-video tests time out under load. | EARLIER | |
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
| HNT-04 | 2 | An "observe" investigation offer advances the story without resolving or recording what was observed (medium confidence). | H05 F1 | |
| HNT-05 | 2 | A wolf's or mastiff's bite knocks its target prone on every hit, without the Strength save the rules require. | H13 F1 | Fixed `cd92d25` |
| HNT-06 | 2 | A healed figure stays down on the TV during playback and its next action can be dropped. | H22 F1 | Fixed `4abbe0f` |
| HNT-07 | 2 | Start-up story text that came back incomplete is cached as complete, so refused slots are never asked for again. | H27 F1 | Fixed `a470d5e` |
| HNT-08 | 2 | After stepping back with the development tool, a late report from the TV can release a beat in the restored fight (medium confidence). | H15 F1 | |
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
| HNT-14 | 2 | Cancelling during a speech upload does not stop the browser's request or the server's transcription. | W30 F1 | Part `c813ce7`: the browser request is aborted; the server still charges and transcribes the clip |
| HNT-15 | 2 | Cleanup of model text can remove every meaningful word and still pass validation; Markdown formatting survives cleanup and reaches the spoken line. | W15 F1, F2 | Fixed `1222ca1` |
| HNT-16 | 2 | Duplicate speaker profiles in the content pack make a character's safety facts depend on array order. | W01 F1 | Fixed `7e6c6e1` |
| HNT-17 | 2 | A catch-up on the TV can erase a submitted scene transition and leave the TV on the previous scene (medium confidence). | W27 F1 | Fixed `129547c` |
| HNT-18 | 2 | A defeat scene that has authored dialogue is labelled as a victory (medium confidence). | W29 F1 | Fixed `224b74f` |
| HNT-19 | 2 | Scan level-of-detail range counts are trusted past the chunk size and can submit an invalid GPU dispatch (medium confidence). | W33 F1 | |
| HNT-20 | 2 | The browser's connection set-up can wait for ever, and join retries never stop when the tunnel cannot be made (medium confidence). | W40 F1, W26 F1 | |
| HNT-21 | 3 | A morning scene whose text contains "knight" gets night lighting; a valid library scene is permanently marked as having no picture. | W02 F2, F1 | Fixed `5e97f97` |
| HNT-22 | 3 | An oversized picture cache file is reported as a successful image and then served as unavailable. | W04 F2 | Fixed `dc9c81f` |
| HNT-23 | 3 | A speech-intent loader can be left on the fight display when its line closes after the TV has gone back to a story scene. | W17 F1 | Fixed `6dcfb0e` (browser check pending) |
| HNT-24 | 3 | A dice folder setting that is not valid Unicode is silently ignored. | W07 F1 | |
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

## Fixed and confirmed this evening

Kept here so they are not triaged again.

- Every character reply was stock text because the mature-rating reply rules were 4,171 characters against a 4,000 limit. Trimmed, with a test guarding the limit; five of six replies then came from the model.
- The server refused to start on a dice manifest it could not read. The manifest was rewritten to the layout the parser expects.
- Victory and defeat now have clean, separate screens.
- The loading card shows the narrator's caption.
- Music ducks under a voiced line (it does not yet come back promptly: AUD-01).
- The server starts on Windows without the manual stack patch.
