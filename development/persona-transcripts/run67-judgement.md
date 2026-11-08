# Run 67: the judgement before the mayor

On the grounded hall (arrest-hall-real.png): each side's move a beat, the one a line is put to answers it (addressee judge). Haldor exposes the painted dagger, the defence breaks the guard's sightline, the fruit seller fixes the time; the mayor acquits and refers the guard for inquiry. Found: the sergeant repeated the bought guard's words, Haldor answered before examining, the verdict was said twice.

- ! ok state town_mayor turn=0 mood valence=13 arousal=51
- !   emotions: trust=45 irritation=20 fear=25 pride=40
- !   knows the names of: nobody
- ! ok state town_mayor turn=0 mood valence=13 arousal=51
- !   emotions: trust=45 irritation=20 fear=25 pride=40
- !   knows the names of: warrior
- ! ok state town_mayor turn=0 mood valence=13 arousal=51
- !   emotions: trust=45 irritation=20 fear=25 pride=40
- !   knows the names of: warrior market_factor
- ! ok state town_mayor turn=0 mood valence=13 arousal=51
- !   emotions: trust=45 irritation=20 fear=25 pride=40
- !   knows the names of: warrior market_factor market_guard_lawful
- ! ok state town_mayor turn=0 mood valence=13 arousal=51
- !   emotions: trust=45 irritation=20 fear=25 pride=40
- !   knows the names of: warrior market_factor market_guard_lawful market_guard_bought
- ! ok state town_mayor turn=0 mood valence=13 arousal=51
- !   emotions: trust=45 irritation=20 fear=25 pride=40
- !   knows the names of: warrior market_factor market_guard_lawful market_guard_bought weapon_seller

**[01] Mayor:** Let the charge be stated before this hall: theft of the Nightglass dagger and wounding the watchman. How do you answer it?

**[02] Mayor:** State your answer to the charge, and distinguish what you admit from what you deny.
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: npc
- feels: trust 68, pride 58, fear 25; mood 23 valence, 48 arousal; toward warrior: indifferent (3); focus warrior on accused’s answer
- time: line 6.4s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6394 ms

**[03] Osric:** Your Honor, this is the near-black Nightglass stolen from the stall and taken from the prisoner; what more proof could any honest town require?
- fired triggers: charge-questioned

**[04] Mayor:** A charge is not a finding; the weapon's presence does not establish who stole it or wounded the watchman. Let the defence question the evidence.
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: npc
- feels: trust 74, pride 67, irritation 25, fear 25; mood 23 valence, 48 arousal; toward warrior: indifferent (3); toward market_factor: indifferent (0); focus market_factor on proof for the charge
- time: line 7.3s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7332 ms

**[05] Kael:** Let the smith who forged that dagger examine it. Mayor, do you fear what he’ll say?
- fired triggers: accused-before-judge

**[06] Mayor:** The smith’s examination is relevant; I have no cause to fear testimony that may establish the facts. Let him proceed.
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: ally
- feels: trust 80, pride 73, fear 25, irritation 24; mood 26 valence, 48 arousal; toward warrior: indifferent (5); toward market_factor: indifferent (0); focus warrior on smith's examination of the dagger
- time: line 6.7s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6723 ms

**[07] Mayor:** Bladesmith, come to the dais, please. Is the dagger held by the accuser the one you forged?

**[08] Haldor:** Yes. That is the Nightglass I forged; its near-black steel, fitted bone handle, and violet sigils are mine.
- behavior: careful-appraisal
- fired triggers: appraisal-question
- fired emotional triggers: questioned-claims (by state: none)
- tone: npc
- feels: pride 86, trust 54, joy 42; mood 51 valence, 48 arousal; toward market_guard_lawful: indifferent (1); toward town_mayor: indifferent (5); focus town_mayor on identifying the Nightglass
- time: line 7.0s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7042 ms

**[09] Haldor:** The handle sits loose on the tang, and those violet marks are paint, not cut and silver-inlaid runes. Accuser, where did you truly get this dagger?
- fired triggers: appraisal-question

**[10] Osric:** The dagger was recovered from the prisoner, as I have said; its true provenance is a matter for proper inquiry, not conjecture.
- behavior: careful-deflection
- behavior override: trigger evidence-questions fired, so the line took careful-deflection, not cornered-bargainer
- fired triggers: evidence-questions, proof-corners-him, charge-questioned
- fired emotional triggers: proof-closes-in (by state: none)
- learned the name of: weapon_seller
- tone: danger
- feels: pride 94, irritation 78, fear 72, trust 35, embarrassment 34; mood -9 valence, 60 arousal; toward warrior: indifferent (-2); toward market_guard_lawful: indifferent (-2); toward mara_adventurer: indifferent (-1); toward town_mayor: indifferent (0); focus weapon_seller on challenge to the dagger's authenticity
- time: line 8.1s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 8139 ms

**[11] Tavin:** I saw the prisoner run from the stall with that dagger in his hand. Does a guard’s oath count for nothing now, Mayor?
- fired triggers: oath-doubted

**[12] Mayor:** An oath deserves a hearing, Sergeant, not a finding. State where you stood, and what you saw between the stall and the prisoner.
- behavior: presses-hard
- fired triggers: vague-account
- fired emotional triggers: relevant-question (by state: none)
- learned the name of: market_guard_bought
- tone: npc
- feels: trust 88, pride 80, irritation 27, fear 25; mood 26 valence, 47 arousal; toward warrior: indifferent (7); toward market_factor: indifferent (0); toward market_guard_bought: indifferent (0); focus market_guard_bought on the guard's claimed sighting
- time: line 7.1s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7144 ms

**[13] Mara:** Sergeant, where exactly were you standing when you saw Kael run, and what stood between you and the stall?

**[14] Tavin:** I stood by the east canvas post. The canvas and the passing crowd cut across the stall, but I had the sightline on Kael.
- behavior: questioned
- fired triggers: scrutiny-of-account, oath-doubted
- fired emotional triggers: pressed-on-sightline (by state: none)
- revealed secrets: obstructed-view
- tone: danger
- feels: irritation 84, fear 71, pride 48, embarrassment 23; mood -30 valence, 62 arousal; toward warrior: indifferent (-5); toward market_factor: indifferent (0); toward mara_adventurer: indifferent (0); focus mara_adventurer on his sightline
- time: line 7.0s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7012 ms

**[15] Osric:** I lay this torn strip of dark cloth, found by the stall, before the court. Prisoner, why is your own cloak torn?
- fired triggers: evidence-questions

**[16] Kael:** It tore while I checked my gear by the stalls. My companions saw it happen. That proves a tear, not a theft—ask them where I stood.
- behavior: fights-for-his-life
- fired triggers: accused-before-judge
- tone: danger
- feels: irritation 84, fear 67, trust 61, pride 43, joy 25; mood -5 valence, 58 arousal; toward market_factor: indifferent (-11); toward market_guard_lawful: indifferent (1); toward weapon_seller: indifferent (1); toward market_guard_bought: indifferent (-3); toward mara_adventurer: indifferent (3); toward bram_adventurer: indifferent (4); toward town_mayor: indifferent (0); focus market_factor on the torn cloth and accusation
- time: line 7.5s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7564 ms

**[17] Kael:** Who found that strip, and when? No one searched me, and no one saw me take the dagger.
- fired triggers: accused-before-judge

**[18] Osric:** I cannot account for who found it or when; the strip was presented as evidence, and its proper weight is for this court to judge.
- behavior: careful-deflection
- fired triggers: evidence-questions
- fired emotional triggers: scrutiny-lingers (by state: none)
- tone: danger
- feels: fear 96, irritation 89, pride 77, trust 35, embarrassment 28; mood -17 valence, 62 arousal; toward warrior: indifferent (-4); toward market_guard_lawful: indifferent (-2); toward mara_adventurer: indifferent (-1); toward town_mayor: indifferent (0); focus warrior on the strip’s discovery
- time: line 7.9s, backfill started
- covered: false (gaps below)
- - backstory: who found the strip and when
- backfill: running (record 1 when done)
- model: codex:gpt-6-luna
- total: 7919 ms

**[19] Sergeant_Dorr:** I stood by the east canvas post; the canvas and passing crowd crossed the view. The guard reported seeing Kael run, but I did not witness the attack. Mayor, was the dagger entered into evidence with a witness to its finding?
- fired triggers: asked-for-basis

**[20] Mayor:** The record establishes no witness to its finding; Sergeant, who presented the dagger, and when was it recovered?
- behavior: careful-deliberation
- fired triggers: material-doubt
- fired emotional triggers: relevant-question (by state: none)
- tone: ally
- feels: trust 91, pride 84, irritation 25, fear 25; mood 28 valence, 47 arousal; toward warrior: indifferent (7); toward market_factor: indifferent (0); toward market_guard_bought: indifferent (0); toward market_guard_lawful: indifferent (3); focus market_guard_lawful on the dagger's chain of evidence
- time: line 7.0s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7013 ms

**[21] Nessa:** Your Honor, the market bell rang late today. I saw the watchman clutching his bleeding arm before the guards took this man. Why was I never asked before?

**[22] Mayor:** Your account is relevant; the hearing must now weigh it. You were not called because your testimony had not yet been brought before the court.
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: npc
- feels: trust 92, pride 87, fear 25, irritation 24; mood 29 valence, 47 arousal; toward warrior: indifferent (7); toward market_factor: indifferent (0); toward market_guard_bought: indifferent (0); toward market_guard_lawful: indifferent (3); toward market_witness: indifferent (3); focus market_witness on the witness’s account and why they were not asked
- time: line 6.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6869 ms

**[23] Mayor:** Guard, at what exact hour did you see the prisoner run, and how could you see him through the canvas and passing crowd?
- fired triggers: vague-account

**[24] Tavin:** I saw him run; the hour was late market, and the canvas and crowd did not hide him from me.
- behavior: questioned
- behavior override: trigger scrutiny-of-account fired, so the line took questioned, not swears-to-it
- fired triggers: scrutiny-of-account, oath-doubted
- fired emotional triggers: pressed-on-sightline (by state: none)
- tone: danger
- feels: irritation 85, fear 80, pride 47, embarrassment 46; mood -32 valence, 63 arousal; toward warrior: indifferent (-5); toward market_factor: indifferent (0); toward mara_adventurer: indifferent (0); toward town_mayor: indifferent (-1); focus mara_adventurer on the questioned sightline and hour
- time: line 7.1s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7148 ms

**[25] Bram:** Then tell us, guard: why’s your purse heavier this morning, and who filled it?

**[26] Tavin:** That purse is my own. I took no coin from anyone; my oath stands.
- behavior: swears-to-it
- fired triggers: oath-doubted
- fired emotional triggers: accused-of-lying (by state: none)
- tone: danger
- feels: irritation 96, fear 88, pride 46, embarrassment 29; mood -34 valence, 64 arousal; toward warrior: indifferent (-5); toward market_factor: indifferent (0); toward mara_adventurer: indifferent (0); toward town_mayor: indifferent (-1); toward bram_adventurer: indifferent (-10); focus bram_adventurer on the accusation of taking coin
- time: line 6.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6833 ms

**[27] Kael:** Mayor, judge me by what this hall has shown today, not by who shouted loudest—will you?
- fired triggers: accused-before-judge

**[28] Mayor:** You shall be judged by the testimony and evidence established here; no voice alone shall decide the charge.
- behavior: careful-deliberation
- behavior override: trigger material-doubt fired, so the line took careful-deliberation, not measured-judge
- fired triggers: material-doubt
- fired emotional triggers: relevant-question (by state: none)
- tone: ally
- feels: trust 94, pride 90, fear 25, irritation 22; mood 30 valence, 47 arousal; toward warrior: indifferent (9); toward market_factor: indifferent (0); toward market_guard_bought: indifferent (2); toward market_guard_lawful: indifferent (3); toward market_witness: indifferent (3); focus warrior on fair judgment of the evidence
- time: line 6.4s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6473 ms

**[29] Osric:** Now, will you let a sword for hire and his hangers-on shame the town’s own guard, Mayor, when the prisoner’s friends have every cause to bend their account?
- fired triggers: charge-questioned

**[30] Mayor:** No, sir. The guard’s standing does not make his account proof, nor do the prisoner’s companions’ ties erase relevant testimony; I shall judge what this hearing has established.
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: npc
- feels: trust 95, pride 91, irritation 27, fear 25; mood 28 valence, 47 arousal; toward warrior: indifferent (9); toward market_factor: indifferent (-2); toward market_guard_bought: indifferent (2); toward market_guard_lawful: indifferent (3); toward market_witness: indifferent (3); focus market_factor on weighing testimony impartially
- time: line 6.9s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6942 ms

**[31] Mayor:** The testimony leaves material doubts unresolved. Prisoner, have you anything further to say before I render judgment?
- fired triggers: material-doubt

**[32] Kael:** The strip proves only that my cloak tore. No one saw me take that dagger, and no one can say who found it or when. Judge the gaps, not the accusation.
- behavior: fights-for-his-life
- fired triggers: accused-before-judge
- tone: danger
- feels: irritation 79, fear 51, pride 51, trust 47, joy 25; mood -2 valence, 57 arousal; toward market_factor: indifferent (-13); toward market_guard_lawful: indifferent (1); toward weapon_seller: indifferent (1); toward market_guard_bought: indifferent (-3); toward mara_adventurer: indifferent (3); toward bram_adventurer: indifferent (4); toward town_mayor: indifferent (0); focus town_mayor on unresolved doubts in the evidence
- time: line 6.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6874 ms

**[33] Mayor:** The evidence leaves the finding of theft and the charge of assault unproved. Kael is acquitted and shall be released; the accuser’s claim is dismissed, and the guard’s account is referred for inquiry into its contradictions and his purse.
- fired triggers: material-doubt

**[34] Mayor:** The finding is acquittal; the charge is unproved, and the guard’s account shall be examined for its contradictions.
- behavior: careful-deliberation
- fired triggers: material-doubt
- fired emotional triggers: uncertain-testimony (by state: none)
- tone: npc
- feels: trust 96, pride 92, fear 49, surprise 30, irritation 23; mood 20 valence, 53 arousal; toward warrior: indifferent (17); toward market_factor: indifferent (-2); toward market_guard_bought: indifferent (2); toward market_guard_lawful: indifferent (3); toward market_witness: indifferent (3); focus warrior on judgment on the charge
- time: line 7.2s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7212 ms
- --- frames (artifacts/tmp/arrest/judgement-frames.log) ---
- [01] Mayor Edrin Vale: Let the charge be stated before this hall: theft of the Nightglass dagger and wounding the watchman. How do you answer it?
- frame judge after line 1: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: His stern expression turns formally questioning as he asks for the answer.
- [02] Mayor Edrin Vale: State your answer to the charge, and distinguish what you admit from what you deny.
- frame judge after line 2: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: sternly addressing the accused; Kael expression: facing the mayor, listening to the charge
- [03] Osric Pell: Your Honor, this is the near-black Nightglass stolen from the stall and taken from the prisoner; what more proof could any honest town require?
- frame judge after line 3: 25% changed, 25% since the frame (gate 25); Mayor Edrin Vale expression: sternly presses for a clear answer; Osric Pell pose: holds the near-black dagger aloft as evidence
- [04] Mayor Edrin Vale: A charge is not a finding; the weapon's presence does not establish who stole it or wounded the watchman. Let the defence question the evidence.
- frame judge after line 4: 18% changed, 25% since the frame (gate 25); Osric Pell pose: holds the dagger up as proof; Sergeant Alwen Dorr expression: watches the evidence with a guarded, serious look; Mayor Edrin Vale expression: sternly cautions that the charge is not a finding; Mayor Edrin Vale pose: leans forward to invite the defence to question the evidence
- [05] Kael: Let the smith who forged that dagger examine it. Mayor, do you fear what he’ll say?
- frame judge after line 5: 35% changed, 35% since the frame (gate 25); Mayor Edrin Vale expression: his stern expression sharpens into a guarded, challenging look; Mayor Edrin Vale pose: leans forward to hear the challenge; Osric Pell expression: his confident look tightens under the mayor’s correction; Kael pose: addresses the mayor with a direct challenge to let the bladesmith examine the dagger
- frame 1 (arrest-judgement-a-1) started after line 5, shot over-the-shoulder, editing arrest-hall-real.png
- [06] Mayor Edrin Vale: The smith’s examination is relevant; I have no cause to fear testimony that may establish the facts. Let him proceed.
- frame judge after line 6: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: his guarded look eases slightly into permission
- [07] Mayor Edrin Vale: Bladesmith, come to the dais, please. Is the dagger held by the accuser the one you forged?
- frame judge after line 7: 30% changed, 30% since the frame (gate 25); Haldor Brask position: steps toward the dais; Haldor Brask pose: turns toward the accuser and the dagger as he is asked to identify it; Mayor Edrin Vale pose: leans forward, inviting the bladesmith to proceed
- [08] Haldor Brask: Yes. That is the Nightglass I forged; its near-black steel, fitted bone handle, and violet sigils are mine.
- frame judge after line 8: 15% changed, 30% since the frame (gate 25); Mayor Edrin Vale expression: his guarded look eases slightly; Haldor Brask position: steps toward the dais; Haldor Brask item: indicates the dagger held by the accuser as his own Nightglass
- frame 2 (arrest-judgement-a-2) started after line 8, shot two-shot, editing arrest-hall-real.png
- [09] Haldor Brask: The handle sits loose on the tang, and those violet marks are paint, not cut and silver-inlaid runes. Accuser, where did you truly get this dagger?
- frame judge after line 9: 15% changed, 15% since the frame (gate 25); Haldor Brask expression: his look hardens into accusation; Haldor Brask pose: he points out the loose handle and violet marks on the dagger
- [10] Osric Pell: The dagger was recovered from the prisoner, as I have said; its true provenance is a matter for proper inquiry, not conjecture.
- frame judge after line 10: 25% changed, 25% since the frame (gate 25); Osric Pell expression: his confident look tightens into a defensive frown; Haldor Brask expression: his look turns accusatory and intent; Osric Pell pose: holds the dagger steady while answering; Haldor Brask pose: leans toward the merchant and challenges him
- [11] Tavin Rusk: I saw the prisoner run from the stall with that dagger in his hand. Does a guard’s oath count for nothing now, Mayor?
- frame judge after line 11: 25% changed, 25% since the frame (gate 25); Haldor Brask expression: His confident look tightens into a skeptical frown.; Haldor Brask pose: He indicates the dagger's loose handle and violet marks as he challenges its provenance.; Osric Pell expression: His confident look tightens as he answers with a guarded expression.; Osric Pell pose: He keeps the dagger raised while answering the challenge.; Tavin Rusk pose: He addresses the mayor, insisting his oath should count.; Mayor Edrin Vale expression: His guarded look sharpens as the dispute turns to the guard's oath.
- [12] Mayor Edrin Vale: An oath deserves a hearing, Sergeant, not a finding. State where you stood, and what you saw between the stall and the prisoner.
- frame judge after line 12: 35% changed, 35% since the frame (gate 25); Haldor Brask expression: His confident look is now tightened into a skeptical frown.; Haldor Brask pose: He keeps the dagger held up while indicating its loose handle and painted violet marks.; Osric Pell expression: His confident look is strained as he answers the accusation.; Tavin Rusk pose: He turns toward the mayor and speaks up in defense of his oath.; Tavin Rusk expression: He looks earnest and challenging.; Mayor Edrin Vale pose: He leans forward on the raised chair, directing the sergeant to state what she saw.; Mayor Edrin Vale expression: His guarded look settles into a measured, attentive expression.; Sergeant Alwen Dorr expression: She looks attentive, ready to give her account.
- [13] Mara: Sergeant, where exactly were you standing when you saw Kael run, and what stood between you and the stall?
- frame 1 (arrest-judgement-a-1) shown 93.7 s after it started at line 5
- frame judge after line 13: 25% changed, 35% since the frame (gate 25); Haldor Brask expression: His confident look tightens as he challenges the dagger’s workmanship and provenance.; Haldor Brask pose: He questions the accuser while holding up the dagger.; Osric Pell expression: His look tightens under the bladesmith’s challenge.; Tavin Rusk expression: He looks toward the mayor as he answers, guarded and insistent.; Mayor Edrin Vale pose: He addresses the sergeant, asking her to account for where she stood and what she saw.; Mara pose: She questions the sergeant about her vantage point and what blocked her view.
- frame 3 (arrest-judgement-a-3) started after line 13, shot wide, editing arrest-hall-real.png
- [14] Tavin Rusk: I stood by the east canvas post. The canvas and the passing crowd cut across the stall, but I had the sightline on Kael.
- frame judge after line 14: 0% changed, 0% since the frame (gate 25)
- [15] Osric Pell: I lay this torn strip of dark cloth, found by the stall, before the court. Prisoner, why is your own cloak torn?
- frame judge after line 15: 30% changed, 30% since the frame (gate 25); Tavin Rusk position: stands by the east canvas post with a clear sightline toward Kael; Tavin Rusk expression: focused on Kael; Osric Pell item: lays a torn strip of dark cloth before the court; Osric Pell pose: presents the cloth strip to the court and questions Kael about his torn cloak; Kael expression: looks at the torn cloth as the accusation is put to him
- [16] Kael: It tore while I checked my gear by the stalls. My companions saw it happen. That proves a tear, not a theft—ask them where I stood.
- frame judge after line 16: 30% changed, 30% since the frame (gate 25); Tavin Rusk position: stands by the east canvas post with a clear sightline toward Kael; Osric Pell item: lays a torn strip of dark cloth before the court; Kael expression: answers with a defensive, intent look; Kael pose: gestures toward his gear as he says the tear happened while checking it
- [17] Kael: Who found that strip, and when? No one searched me, and no one saw me take the dagger.
- frame judge after line 17: 20% changed, 30% since the frame (gate 25); Tavin Rusk position: Stands by the east canvas post, with a clear sightline toward Kael.; Tavin Rusk pose: Indicates that he had a view of Kael despite the canvas and passing crowd.; Osric Pell item: Lays a torn strip of dark cloth before the court.; Kael expression: Defensively challenges the accusation, looking toward the court.; Kael pose: Bound and kneeling, explains his torn cloak and asks his companions to confirm where he stood.; Mara expression: Questions the account with a skeptical, narrowed look.
- [18] Osric Pell: I cannot account for who found it or when; the strip was presented as evidence, and its proper weight is for this court to judge.
- frame judge after line 18: 12% changed, 30% since the frame (gate 25); Tavin Rusk position: Stands by the east canvas post, sightline toward Kael; Osric Pell item: Sets a torn strip of dark cloth before the court; Kael expression: Defensive, challenging look as he answers; Osric Pell expression: Measured, guarded look as he admits he cannot account for the strip's origin
- [19] Sergeant Alwen Dorr: I stood by the east canvas post; the canvas and passing crowd crossed the view. The guard reported seeing Kael run, but I did not witness the attack. Mayor, was the dagger entered into evidence with a witness to its finding?
- frame judge after line 19: 25% changed, 30% since the frame (gate 25); Tavin Rusk position: stands by the east canvas post; Sergeant Alwen Dorr position: stands by the east canvas post, with the canvas and crowd crossing his view; Osric Pell item: lays the torn dark cloth strip before the court; Kael expression: questions who found the strip and when, with a guarded, challenging look; Mayor Edrin Vale expression: listens to the sergeant's question with a stern, guarded look; Sergeant Alwen Dorr pose: asks the mayor whether the dagger's entry into evidence was witnessed
- [20] Mayor Edrin Vale: The record establishes no witness to its finding; Sergeant, who presented the dagger, and when was it recovered?
- frame 2 (arrest-judgement-a-2) shown 156.6 s after it started at line 8
- frame judge after line 20: 25% changed, 30% since the frame (gate 25); Osric Pell item: lays a torn strip of dark cloth before the court; Tavin Rusk position: stands by the east canvas post; Sergeant Alwen Dorr expression: looks guarded as she explains the crowd obscured her view; Mayor Edrin Vale expression: narrows his look, questioning the sergeant; Kael expression: looks defensive as he denies taking the dagger
- frame 4 (arrest-judgement-a-4) started after line 20, shot over-the-shoulder, editing arrest-hall-real.png
- [21] Nessa Vale: Your Honor, the market bell rang late today. I saw the watchman clutching his bleeding arm before the guards took this man. Why was I never asked before?
- frame judge after line 21: 10% changed, 10% since the frame (gate 25); Nessa Vale expression: speaks with an urgent, questioning look; Sergeant Alwen Dorr expression: turns a guarded look toward the witness
- [22] Mayor Edrin Vale: Your account is relevant; the hearing must now weigh it. You were not called because your testimony had not yet been brought before the court.
- frame judge after line 22: 30% changed, 30% since the frame (gate 25); Nessa Vale expression: speaks with an urgent, questioning look; Nessa Vale pose: addresses the mayor from the bench; Nessa Vale item: indicates the mayor and court as she gives her account; Mayor Edrin Vale expression: listens with a measured, attentive expression; Mayor Edrin Vale pose: leans forward to answer the witness
- [23] Mayor Edrin Vale: Guard, at what exact hour did you see the prisoner run, and how could you see him through the canvas and passing crowd?
- frame judge after line 23: 30% changed, 30% since the frame (gate 25); Nessa Vale expression: She speaks with an earnest, questioning look.; Nessa Vale pose: She addresses the mayor from her bench.; Mayor Edrin Vale expression: His questioning look softens into attentive consideration.; Mayor Edrin Vale pose: He turns from the guard to address the witness, then faces the guard again with a direct question.; Sergeant Alwen Dorr expression: She looks guarded and scrutinized.
- [24] Tavin Rusk: I saw him run; the hour was late market, and the canvas and crowd did not hide him from me.
- frame judge after line 24: 25% changed, 30% since the frame (gate 25); Nessa Vale expression: looks intent and aggrieved as she speaks; Nessa Vale pose: addresses the mayor from the bench; Mayor Edrin Vale expression: turns a measured, attentive look toward the witness; Mayor Edrin Vale pose: leans forward to address the guard; Tavin Rusk expression: looks guarded under the mayor's questioning; Tavin Rusk pose: answers the mayor from beside the east canvas post
- [25] Bram: Then tell us, guard: why’s your purse heavier this morning, and who filled it?
- frame judge after line 25: 36% changed, 36% since the frame (gate 25); Nessa Vale pose: Rises from the bench and addresses the mayor.; Nessa Vale expression: Looks earnest and indignant.; Mayor Edrin Vale expression: Turns his attention to the witness, listening with a measured, questioning look.; Mayor Edrin Vale pose: Faces the witness and answers from the raised chair.; Tavin Rusk expression: Looks tense under the question about his purse.; Tavin Rusk pose: Faces the court as Bram challenges him about the heavier purse.; Bram pose: Leans toward the guard as he challenges him.; Bram expression: Wears a skeptical, challenging look.
- [26] Tavin Rusk: That purse is my own. I took no coin from anyone; my oath stands.
- frame 3 (arrest-judgement-a-3) shown 166.2 s after it started at line 13
- frame judge after line 26: 30% changed, 36% since the frame (gate 25); Nessa Vale position: Nessa stands to give her testimony.; Nessa Vale expression: She looks earnest and aggrieved.; Mayor Edrin Vale expression: His questioning look turns attentive as he weighs her account, then skeptical toward the guard.; Mayor Edrin Vale pose: He turns his attention from the witness to question the guard.; Tavin Rusk expression: He looks defensive under the challenge.; Tavin Rusk pose: He holds his purse close while denying taking coin.; Bram expression: He fixes the guard with a challenging look.
- frame 5 (arrest-judgement-a-5) started after line 26, shot wide, editing arrest-hall-real.png
- [27] Kael: Mayor, judge me by what this hall has shown today, not by who shouted loudest—will you?
- frame judge after line 27: 10% changed, 10% since the frame (gate 25); Kael expression: addresses the mayor with a steady, pleading look; Mayor Edrin Vale expression: listens with a stern, appraising look
- [28] Mayor Edrin Vale: You shall be judged by the testimony and evidence established here; no voice alone shall decide the charge.
- frame judge after line 28: 10% changed, 10% since the frame (gate 25); Kael expression: Looks directly toward the mayor with a measured, earnest appeal.; Mayor Edrin Vale expression: His questioning skepticism settles into a formal, attentive expression.
- [29] Osric Pell: Now, will you let a sword for hire and his hangers-on shame the town’s own guard, Mayor, when the prisoner’s friends have every cause to bend their account?
- frame judge after line 29: 15% changed, 15% since the frame (gate 25); Mayor Edrin Vale expression: His skeptical look settles into a firm, impartial expression as he answers Kael.; Osric Pell expression: His confident look turns doubtful and accusatory as he challenges the mayor.; Kael expression: He looks toward the mayor as he asks to be judged by the evidence.
- [30] Mayor Edrin Vale: No, sir. The guard’s standing does not make his account proof, nor do the prisoner’s companions’ ties erase relevant testimony; I shall judge what this hearing has established.
- frame judge after line 30: 10% changed, 15% since the frame (gate 25); Mayor Edrin Vale expression: His skeptical, questioning look settles into a firm, impartial expression.; Osric Pell expression: He looks doubtful and accusatory, his gaze fixed on the mayor.; Kael expression: He looks toward the mayor, appealing for judgment by the evidence.
- [31] Mayor Edrin Vale: The testimony leaves material doubts unresolved. Prisoner, have you anything further to say before I render judgment?
- frame 4 (arrest-judgement-a-4) shown 131.0 s after it started at line 20
- frame judge after line 31: 10% changed, 15% since the frame (gate 25); Mayor Edrin Vale expression: stern and attentive, then skeptical and questioning; Kael pose: kneeling bound, facing the mayor as he speaks; Osric Pell expression: confident look tightened as he challenges the prisoner’s account; Mayor Edrin Vale pose: leans forward to address the guard, then turns his questioning attention to the prisoner
- [32] Kael: The strip proves only that my cloak tore. No one saw me take that dagger, and no one can say who found it or when. Judge the gaps, not the accusation.
- frame judge after line 32: 25% changed, 25% since the frame (gate 25); Mayor Edrin Vale expression: stern and attentive, then skeptical as he weighs the accounts; Mayor Edrin Vale pose: leans forward to hear Kael's final statement; Kael expression: serious and intent as he addresses the mayor; Kael pose: kneels bound, speaking toward the mayor
- [33] Mayor Edrin Vale: The evidence leaves the finding of theft and the charge of assault unproved. Kael is acquitted and shall be released; the accuser’s claim is dismissed, and the guard’s account is referred for inquiry into its contradictions and his purse.
- frame judge after line 33: 50% changed, 50% since the frame (gate 25); Mayor Edrin Vale expression: stern and decisive as he delivers the acquittal; Mayor Edrin Vale pose: sits upright on the raised chair, concluding the hearing; Kael pose: no longer kneels bound; stands released; Kael prop: no longer bound; Tavin Rusk expression: defensive look unsettled by the referral for inquiry; Tavin Rusk pose: holds his purse close, under scrutiny; Osric Pell expression: confident look tightened as his claim is dismissed; Sergeant Alwen Dorr expression: guarded, with the guard’s account referred for inquiry; Mara expression: attentive as the ruling releases her companion
- frame 6 (arrest-judgement-a-6) started after line 33, shot wide, editing arrest-hall-real.png
- [34] Mayor Edrin Vale: The finding is acquittal; the charge is unproved, and the guard’s account shall be examined for its contradictions.
- frame judge after line 34: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: his decisive look turns stern as he declares the acquittal; Tavin Rusk pose: stands defensive, holding his purse close as his account is referred for inquiry
- plate arrest-judgement-a-5: arrest-judgement-a-5.png is behind the scene (16 regions found, 7 of 11 characters named), with the cast of arrest-judgement-a-3.png
- frame 5 (arrest-judgement-a-5) shown 121.8 s after it started at line 26
- plate arrest-judgement-a-6: arrest-judgement-a-6.png is behind the scene (16 regions found, 9 of 11 characters named), with the cast of arrest-judgement-a-4.png
- frame 6 (arrest-judgement-a-6) shown 135.0 s after it started at line 33
