# Run 68: the judgement, refined

Addressee judge told the speaker's goal; Haldor examines before he speaks; the guards' public descriptions say what anyone can see. The fake is ruled out at once, the guard's sightline falls apart, the mayor acquits once. Found: everyone calls the bought guard 'Sergeant' after the mayor did; Kael answers twice where a question to him meets his own beat.

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

**[01] Mayor:** Let the charge be stated before this hall. How do you answer the accusation of stealing the Nightglass dagger and cutting the watchman who sought to stop you?

**[02] Kael:** I stole neither dagger nor bloodied that watchman. A scrap from my torn cloak proves nothing—ask who handled the blade, and what they truly saw.
- behavior: fights-for-his-life
- fired triggers: accused-before-judge
- tone: danger
- feels: irritation 81, pride 54, fear 47, trust 43, joy 25; mood -2 valence, 57 arousal; toward market_factor: indifferent (-13); toward market_guard_lawful: indifferent (1); toward weapon_seller: indifferent (1); toward market_guard_bought: indifferent (-3); toward mara_adventurer: indifferent (3); toward bram_adventurer: indifferent (4); toward town_mayor: indifferent (0); focus town_mayor on answering the charge
- time: line 6.7s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6709 ms

**[03] Osric:** The Nightglass was taken from the prisoner; held here before this hall, it is plain proof enough, is it not, Your Honor?
- fired triggers: charge-questioned

**[04] Mayor:** A charge is not a finding, Master Pell. Custody establishes where the dagger was held, not who stole it; what testimony connects the prisoner to the taking?
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: npc
- feels: trust 62, pride 48, irritation 26, fear 25; mood 17 valence, 50 arousal; toward market_factor: indifferent (0); focus market_factor on whether custody proves the charge
- time: line 6.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6818 ms

**[05] Kael:** Let the smith who forged the Nightglass look at it. Does Your Honor fear what he’ll say?
- fired triggers: accused-before-judge

**[06] Mayor:** I fear no relevant testimony; let the smith examine the dagger and speak to what he knows.
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- learned the name of: warrior
- tone: npc
- feels: trust 72, pride 58, irritation 25, fear 25; mood 22 valence, 48 arousal; toward market_factor: indifferent (0); toward warrior: indifferent (2); focus warrior on hearing relevant testimony
- time: line 6.2s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6225 ms

**[07] Haldor:** This is not the Nightglass I forged: the handle sits loose on the tang, and these violet marks are painted, not cut and inlaid. Master Pell, where did you truly get this dagger?
- fired triggers: appraisal-question

**[08] Osric:** The dagger was recovered from the prisoner, Master Brask; its origin is not before this hall, only the charge and the evidence supporting it.
- behavior: careful-deflection
- fired triggers: evidence-questions, charge-questioned
- fired emotional triggers: scrutiny-lingers (by state: none)
- tone: danger
- feels: irritation 96, fear 91, pride 85, trust 35; mood -15 valence, 62 arousal; toward warrior: indifferent (-4); toward market_guard_lawful: indifferent (-2); toward mara_adventurer: indifferent (-1); toward town_mayor: indifferent (-2); focus town_mayor on the false dagger’s origin
- time: line 6.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6861 ms

**[09] Tavin:** I swear before this hall: I saw the prisoner run from the stall with that dagger in his hand. Does a guard’s oath count for nothing now, Your Honor?
- fired triggers: oath-doubted

**[10] Mayor:** Your oath is heard, Sergeant; state where you stood and what you saw, apart from what you inferred.
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- learned the name of: market_guard_bought
- tone: npc
- feels: trust 79, pride 67, fear 25, irritation 24; mood 25 valence, 48 arousal; toward market_factor: indifferent (0); toward warrior: indifferent (2); toward market_guard_bought: indifferent (1); focus market_guard_bought on the guard’s claimed eyewitness account
- time: line 6.4s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6402 ms

**[11] Mara:** Sergeant, exactly where were you standing when you say you saw Kael run, and what stood between you and the stall?

**[12] Tavin:** I stood by the north canvas, with the stall awning and passing folk between us. I had the market in view; I saw him run.
- behavior: questioned
- fired triggers: scrutiny-of-account, oath-doubted
- fired emotional triggers: pressed-on-sightline (by state: none)
- revealed secrets: obstructed-view
- tone: danger
- feels: fear 95, irritation 89, embarrassment 64, pride 49; mood -33 valence, 63 arousal; toward warrior: indifferent (-5); toward market_factor: indifferent (0); toward mara_adventurer: indifferent (0); toward town_mayor: indifferent (-1); toward bram_adventurer: indifferent (-10); focus mara_adventurer on his position and sightline
- time: line 6.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6813 ms

**[13] Osric:** Let the record show this strip was found by the stall. Prisoner, why is your own cloak torn?
- fired triggers: charge-questioned

**[14] Kael:** It tore while I checked my gear by the stalls. My companions saw it. That scrap proves nothing about the dagger—ask them what they saw.
- behavior: fights-for-his-life
- fired triggers: accused-before-judge
- tone: danger
- feels: irritation 73, pride 54, fear 40, trust 37, joy 25; mood 0 valence, 57 arousal; toward market_factor: indifferent (-16); toward market_guard_lawful: indifferent (1); toward weapon_seller: indifferent (1); toward market_guard_bought: indifferent (-3); toward mara_adventurer: indifferent (3); toward bram_adventurer: indifferent (4); toward town_mayor: indifferent (0); focus town_mayor on cloak scrap and accusation
- time: line 8.5s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 8523 ms

**[15] Kael:** Who found that strip, and when? No one searched me, and no one saw me take it.
- fired triggers: accused-before-judge

**[16] Osric:** The strip was found by the stall; who found it, and when, is not established. Let us keep to the evidence before us.
- behavior: careful-deflection
- fired triggers: evidence-questions
- fired emotional triggers: scrutiny-lingers (by state: none)
- tone: ally
- feels: fear 98, irritation 96, pride 71, trust 35; mood -19 valence, 63 arousal; toward warrior: indifferent (-8); toward market_guard_lawful: indifferent (-2); toward mara_adventurer: indifferent (-1); toward town_mayor: indifferent (-2); focus warrior on who found the cloak strip
- time: line 7.2s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7229 ms

**[17] Sergeant_Dorr:** I did not witness the taking; the account of the attack was reported to me. Your Honor, was the dagger ever entered into custody with a witness to its finding?
- fired triggers: asked-for-basis

**[18] Mayor:** The custody account is before this hall; was a witness present when the dagger was found, and is that witness here to testify?
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: npc
- feels: trust 84, pride 73, fear 25, irritation 23; mood 27 valence, 47 arousal; toward market_factor: indifferent (0); toward warrior: indifferent (2); toward market_guard_bought: indifferent (1); toward market_guard_lawful: indifferent (2); focus market_guard_lawful on custody and the dagger’s finding
- time: line 6.6s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6573 ms

**[19] Nessa:** Sir, the market bell rang late today. I saw the watchman clutching his bleeding arm before the guards took this man. Why was I not asked before?

**[20] Mayor:** Your testimony is relevant, and should have been heard; tell this hall what you saw, and when.
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: npc
- feels: trust 88, pride 79, fear 25, irritation 22; mood 28 valence, 47 arousal; toward market_factor: indifferent (0); toward warrior: indifferent (2); toward market_guard_bought: indifferent (1); toward market_guard_lawful: indifferent (2); toward market_witness: indifferent (3); focus market_witness on witness testimony about the injured watchman
- time: line 6.9s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6908 ms

**[21] Mayor:** Sergeant, at what hour exactly did you see him run, and how could you see him through the canvas and passing crowd?
- fired triggers: vague-account

**[22] Tavin:** I can't give the exact hour. The canvas and passing folk cut the sightline; I saw him run, and that's my account.
- behavior: questioned
- fired triggers: scrutiny-of-account
- fired emotional triggers: pressed-on-sightline (by state: none)
- tone: danger
- feels: fear 97, embarrassment 89, irritation 81, pride 48; mood -33 valence, 63 arousal; toward warrior: indifferent (-5); toward market_factor: indifferent (0); toward mara_adventurer: indifferent (-2); toward town_mayor: indifferent (-1); toward bram_adventurer: indifferent (-10); focus mara_adventurer on his position and sightline
- time: line 7.5s, backfill started
- covered: false (gaps below)
- - backstory: the exact hour he saw Kael run
- backfill: running (record 1 when done)
- model: codex:gpt-6-luna
- total: 7553 ms

**[23] Bram:** Sergeant, why’s your purse newly heavy this morning, and who filled it?

**[24] Tavin:** That is a slanderous question, sir. My purse is my own concern; the prisoner’s custody is the matter before this hall.
- behavior: swears-to-it
- behavior override: trigger oath-doubted fired, so the line took swears-to-it, not defensive
- fired triggers: oath-doubted
- fired emotional triggers: accused-of-lying (by state: none)
- tone: danger
- feels: fear 97, irritation 91, embarrassment 56, pride 45; mood -35 valence, 64 arousal; toward warrior: indifferent (-5); toward market_factor: indifferent (0); toward mara_adventurer: indifferent (-2); toward town_mayor: indifferent (-1); toward bram_adventurer: indifferent (-14); focus bram_adventurer on the purse and its source
- time: line 7.3s, backfill started
- covered: false (gaps below)
- - backstory: why his purse is newly heavy and who filled it
- backfill: running (record 1 when done)
- model: codex:gpt-6-luna
- total: 7296 ms

**[25] Kael:** Your Honor, will you judge me by what this hall has shown today, not by who shouted loudest?
- fired triggers: accused-before-judge

**[26] Mayor:** Yes. I will judge only what this hall has established; the guard’s obstructed sight leaves a material doubt.
- behavior: careful-deliberation
- fired triggers: material-doubt
- fired emotional triggers: relevant-question (by state: none)
- tone: ally
- feels: trust 92, pride 85, fear 25; mood 30 valence, 46 arousal; toward market_factor: indifferent (0); toward warrior: indifferent (5); toward market_guard_bought: indifferent (3); toward market_guard_lawful: indifferent (2); toward market_witness: indifferent (3); focus warrior on whether testimony supports the charge
- time: line 6.4s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6470 ms

**[27] Osric:** Your Honor, will you let a sword for hire and his hangers-on shame the town’s own guard, when their doubts cannot undo his oath?
- fired triggers: charge-questioned

**[28] Mayor:** The oath is heard, but it does not settle the charge; I will judge only what the testimony establishes.
- behavior: careful-deliberation
- behavior override: trigger material-doubt fired, so the line took careful-deliberation, not measured-judge
- fired triggers: material-doubt
- tone: npc
- feels: trust 87, pride 86, irritation 26, fear 25; mood 27 valence, 47 arousal; toward market_factor: indifferent (-2); toward warrior: indifferent (5); toward market_guard_bought: indifferent (3); toward market_guard_lawful: indifferent (2); toward market_witness: indifferent (3); focus market_factor on whether oath outweighs doubt
- time: line 6.6s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6587 ms

**[29] Mayor:** The testimony leaves material doubts, and the oath does not resolve them. Prisoner, have you anything further to say before I judge?
- fired triggers: material-doubt

**[30] Kael:** You’ve heard the doubts. No one saw me take that dagger, and no witness ties me to the watchman’s wound. Judge what this hall proved—not what they assume.
- behavior: fights-for-his-life
- fired triggers: accused-before-judge
- tone: ally
- feels: pride 60, irritation 56, trust 38, fear 31, joy 25; mood 7 valence, 55 arousal; toward market_factor: indifferent (-16); toward market_guard_lawful: indifferent (1); toward weapon_seller: indifferent (1); toward market_guard_bought: indifferent (-3); toward mara_adventurer: indifferent (3); toward bram_adventurer: indifferent (4); toward town_mayor: indifferent (4); focus town_mayor on final defense before judgment
- time: line 7.0s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7030 ms

**[31] Mayor:** The charge is not proved: you are acquitted and released; the accuser’s claim is dismissed, and the guard’s account is referred for inquiry into his oath.
- fired triggers: material-doubt

**[32] Kael:** Then unbind me, Your Honor. I’ll remember you judged what was proved.
- behavior: fights-for-his-life
- fired triggers: accused-before-judge
- tone: ally
- feels: joy 55, pride 55, irritation 49, trust 48, fear 29, surprise 28; mood 18 valence, 56 arousal; toward market_factor: indifferent (-16); toward market_guard_lawful: indifferent (1); toward weapon_seller: indifferent (1); toward market_guard_bought: indifferent (-3); toward mara_adventurer: indifferent (3); toward bram_adventurer: indifferent (4); toward town_mayor: indifferent (10); focus town_mayor on acquittal and release
- time: line 6.7s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6724 ms
- --- frames (artifacts/tmp/arrest/judgement2-frames.log) ---
- [01] Mayor Edrin Vale: Let the charge be stated before this hall. How do you answer the accusation of stealing the Nightglass dagger and cutting the watchman who sought to stop you?
- frame judge after line 1: 0% changed, 0% since the frame (gate 25)
- [02] Kael: I stole neither dagger nor bloodied that watchman. A scrap from my torn cloak proves nothing—ask who handled the blade, and what they truly saw.
- frame judge after line 2: 25% changed, 25% since the frame (gate 25); Mayor Edrin Vale expression: sternly addresses the hall, leaning forward; Kael expression: firm, defiant expression as he denies the accusation; Kael pose: kneeling bound, speaking toward the mayor; Kael item: gestures toward the dagger held at the left as he questions who handled it
- [03] Osric Pell: The Nightglass was taken from the prisoner; held here before this hall, it is plain proof enough, is it not, Your Honor?
- frame judge after line 3: 25% changed, 25% since the frame (gate 25); Mayor Edrin Vale expression: sternly questioning, leaning forward; Kael expression: firmly denying the accusation; Osric Pell pose: holding the dagger up toward the mayor as evidence
- [04] Mayor Edrin Vale: A charge is not a finding, Master Pell. Custody establishes where the dagger was held, not who stole it; what testimony connects the prisoner to the taking?
- frame judge after line 4: 25% changed, 25% since the frame (gate 25); Kael expression: Firmly denies the accusation.; Osric Pell pose: Holds the dagger up as evidence.; Mayor Edrin Vale expression: Sternly distinguishes custody from proof.; Mayor Edrin Vale pose: Leans forward to demand testimony.
- [05] Kael: Let the smith who forged the Nightglass look at it. Does Your Honor fear what he’ll say?
- frame judge after line 5: 20% changed, 25% since the frame (gate 25); Osric Pell expression: His raised, accusing expression has tightened into a doubtful look.; Mayor Edrin Vale pose: He sits upright on the raised chair, facing the prisoner and the accuser.; Kael expression: He looks defiant as he denies the accusation.; Kael pose: Still kneeling and bound, he turns his attention toward the mayor.; Haldor Brask position: At the wooden rail, the grey-bearded bladesmith is singled out as the person asked to examine the dagger.
- [06] Mayor Edrin Vale: I fear no relevant testimony; let the smith examine the dagger and speak to what he knows.
- frame judge after line 6: 40% changed, 40% since the frame (gate 25); Kael expression: The bound blond man speaks with a firm, defiant look.; Osric Pell expression: The heavy merchant looks guarded as he insists the dagger is proof.; Mayor Edrin Vale expression: The stern mayor looks skeptical, questioning the merchant's claim.; Mayor Edrin Vale pose: The mayor leans forward to invite the bladesmith to examine the dagger.
- frame 1 (arrest-judgement-b-1) started after line 6, shot over-the-shoulder, editing arrest-hall-real.png
- [07] Haldor Brask: This is not the Nightglass I forged: the handle sits loose on the tang, and these violet marks are painted, not cut and inlaid. Master Pell, where did you truly get this dagger?
- frame judge after line 7: 25% changed, 25% since the frame (gate 25); Haldor Brask expression: His expression is stern and accusatory.; Haldor Brask pose: He indicates the dagger’s loose handle and painted violet marks as he challenges the merchant.; Osric Pell expression: He looks challenged and guarded.; Osric Pell item: He still holds up the dagger, its handle sitting loose and violet marks visible.
- [08] Osric Pell: The dagger was recovered from the prisoner, Master Brask; its origin is not before this hall, only the charge and the evidence supporting it.
- frame judge after line 8: 30% changed, 30% since the frame (gate 25); Haldor Brask expression: The bladesmith looks stern and accusatory.; Haldor Brask item: He points out the dagger’s loose handle and painted violet marks.; Osric Pell expression: The merchant looks guarded as he answers.; Osric Pell pose: He faces the bladesmith while holding the dagger.
- [09] Tavin Rusk: I swear before this hall: I saw the prisoner run from the stall with that dagger in his hand. Does a guard’s oath count for nothing now, Your Honor?
- frame judge after line 9: 30% changed, 30% since the frame (gate 25); Haldor Brask expression: His guarded look hardens as he challenges the dagger’s authenticity.; Haldor Brask item: He examines the dagger’s loose handle and violet marks, indicating the handle sits loose on the tang and the marks are painted rather than cut and inlaid.; Osric Pell expression: He looks guarded as he answers the bladesmith.; Tavin Rusk expression: He looks insistent as he swears to the prisoner’s flight.
- frame 2 (arrest-judgement-b-2) started after line 9, shot over-the-shoulder, editing arrest-hall-real.png
- [10] Mayor Edrin Vale: Your oath is heard, Sergeant; state where you stood and what you saw, apart from what you inferred.
- frame judge after line 10: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: skeptical expression as he asks for the sergeant’s exact position and separates what she saw from what she inferred
- [11] Mara: Sergeant, exactly where were you standing when you say you saw Kael run, and what stood between you and the stall?
- frame judge after line 11: 15% changed, 15% since the frame (gate 25); Mayor Edrin Vale expression: Keeps a skeptical, attentive look on the sergeant.; Sergeant Alwen Dorr expression: Turns her attention toward Mara, listening to the question.; Mara pose: Addresses the sergeant from the rail, asking where she stood and what obstructed her view.
- [12] Tavin Rusk: I stood by the north canvas, with the stall awning and passing folk between us. I had the market in view; I saw him run.
- frame judge after line 12: 15% changed, 15% since the frame (gate 25); Mayor Edrin Vale pose: leans forward, asking the guard to separate where he stood and what he saw from what he inferred; Mara pose: questions the guard directly about his position and what blocked his view; Tavin Rusk expression: answers with an intent, insistent look; Tavin Rusk pose: indicates he stood by the north canvas, with the awning and passing people between him and the stall
- [13] Osric Pell: Let the record show this strip was found by the stall. Prisoner, why is your own cloak torn?
- frame 1 (arrest-judgement-b-1) shown 84.9 s after it started at line 6
- frame judge after line 13: 30% changed, 30% since the frame (gate 25); Mayor Edrin Vale expression: leans forward skeptically as he questions the guard; Tavin Rusk pose: answers from his place by the north canvas, indicating the awning and passing crowd as obstructions; Mara pose: questions the guard intently about his position and view; Osric Pell pose: addresses the bound man from the rail, asking about his torn cloak
- frame 3 (arrest-judgement-b-3) started after line 13, shot wide, editing arrest-hall-real.png
- [14] Kael: It tore while I checked my gear by the stalls. My companions saw it. That scrap proves nothing about the dagger—ask them what they saw.
- frame judge after line 14: 10% changed, 10% since the frame (gate 25); Kael expression: Looks earnest as he insists the scrap proves nothing.; Kael pose: Indicates the torn scrap while directing attention toward his companions.
- [15] Kael: Who found that strip, and when? No one searched me, and no one saw me take it.
- frame judge after line 15: 10% changed, 10% since the frame (gate 25); Kael expression: His expression is defensive and questioning.; Kael pose: Still kneeling, he gestures toward his companions as witnesses and challenges the claim that anyone saw him take the strip.
- [16] Osric Pell: The strip was found by the stall; who found it, and when, is not established. Let us keep to the evidence before us.
- frame judge after line 16: 25% changed, 25% since the frame (gate 25); Kael expression: Speaks defensively, insisting the torn scrap proves nothing.; Kael pose: Still kneeling bound, he addresses the room and gestures toward his companions.; Osric Pell expression: Keeps a guarded, hard expression while addressing the bound man.; Sergeant Alwen Dorr expression: Watches the exchange intently.; Tavin Rusk expression: Looks uncertain as the account of the strip is questioned.; Mayor Edrin Vale expression: Leans forward skeptically, keeping the hearing focused on evidence.
- [17] Sergeant Alwen Dorr: I did not witness the taking; the account of the attack was reported to me. Your Honor, was the dagger ever entered into custody with a witness to its finding?
- frame judge after line 17: 10% changed, 25% since the frame (gate 25); Kael expression: Defensive, questioning expression.; Osric Pell pose: Addresses the room, pressing that the torn strip proves nothing.; Sergeant Alwen Dorr pose: Turns her attention from the guard to the mayor, asking about custody of the dagger.; Tavin Rusk expression: Uncertainty as he is asked whether the dagger was witnessed entering custody.
- [18] Mayor Edrin Vale: The custody account is before this hall; was a witness present when the dagger was found, and is that witness here to testify?
- frame 2 (arrest-judgement-b-2) shown 113.2 s after it started at line 9
- frame judge after line 18: 10% changed, 25% since the frame (gate 25); Kael expression: Questions the evidence with a guarded, challenging look.; Mayor Edrin Vale expression: Turns his skeptical attention from the guard to the bound man.; Sergeant Alwen Dorr pose: Addresses the mayor, asking about witnessed custody of the dagger.; Osric Pell expression: Keeps a guarded, hard expression as the bound man denies the strip proves anything.
- [19] Nessa Vale: Sir, the market bell rang late today. I saw the watchman clutching his bleeding arm before the guards took this man. Why was I not asked before?
- frame judge after line 19: 26% changed, 26% since the frame (gate 25); Kael expression: He speaks defensively, his face wary.; Osric Pell expression: He listens with a guarded, hardening expression.; Sergeant Alwen Dorr expression: She looks intent and questioning.; Mayor Edrin Vale expression: He watches skeptically as he questions the guard.; Nessa Vale position: She rises from the bench to address the hall.; Nessa Vale expression: She looks urgent and indignant.
- frame 4 (arrest-judgement-b-4) started after line 19, shot wide, editing arrest-hall-real.png
- [20] Mayor Edrin Vale: Your testimony is relevant, and should have been heard; tell this hall what you saw, and when.
- frame judge after line 20: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: His skeptical expression softens into attentive seriousness.; Mayor Edrin Vale pose: Leans forward and invites the witness to speak.
- [21] Mayor Edrin Vale: Sergeant, at what hour exactly did you see him run, and how could you see him through the canvas and passing crowd?
- frame judge after line 21: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: leans forward with a skeptical, questioning look; Tavin Rusk expression: faces the mayor, visibly questioned about the hour and his view through the crowd
- [22] Tavin Rusk: I can't give the exact hour. The canvas and passing folk cut the sightline; I saw him run, and that's my account.
- frame judge after line 22: 25% changed, 25% since the frame (gate 25); Mayor Edrin Vale expression: Leans forward with a skeptical, probing look toward the guard.; Tavin Rusk expression: Looks uncertain as he admits he cannot give the exact hour.; Tavin Rusk pose: Explains the obstructed sightline from his place by the north canvas.
- [23] Bram: Sergeant, why’s your purse newly heavy this morning, and who filled it?
- plate arrest-judgement-b-3: arrest-judgement-b-3.png is behind the scene (16 regions found, 7 of 11 characters named), with the cast of arrest-judgement-b-1.png
- frame 3 (arrest-judgement-b-3) shown 122.2 s after it started at line 13
- frame judge after line 23: 25% changed, 25% since the frame (gate 25); Mayor Edrin Vale pose: leans forward, questioning the guard; Tavin Rusk expression: looks uncertain as he admits he cannot give an exact hour; Tavin Rusk pose: explains that canvas and passing people obstructed his view; Bram expression: looks accusingly at the guard; Bram pose: challenges the guard about his newly heavy purse
- [24] Tavin Rusk: That is a slanderous question, sir. My purse is my own concern; the prisoner’s custody is the matter before this hall.
- frame judge after line 24: 20% changed, 25% since the frame (gate 25); Mayor Edrin Vale pose: leans forward and questions the guard; Tavin Rusk expression: looks defensive and indignant; Bram pose: challenges the guard about his purse; Tavin Rusk pose: holds his ground as he answers; Sergeant Alwen Dorr expression: watches the exchange intently
- [25] Kael: Your Honor, will you judge me by what this hall has shown today, not by who shouted loudest?
- frame judge after line 25: 30% changed, 30% since the frame (gate 25); Mayor Edrin Vale expression: skeptical, pressing the sergeant for an exact hour and a clear sightline; Tavin Rusk expression: defensive, unable to give an exact hour and insisting his purse is his own concern; Tavin Rusk pose: answers the mayor, then turns defensively toward Bram's accusation; Bram expression: challenging the guard with a narrowed, questioning look; Kael expression: wary, appealing to the mayor to judge the evidence rather than the loudest voice
- frame 5 (arrest-judgement-b-5) started after line 25, shot over-the-shoulder, editing arrest-hall-real.png
- [26] Mayor Edrin Vale: Yes. I will judge only what this hall has established; the guard’s obstructed sight leaves a material doubt.
- frame judge after line 26: 5% changed, 5% since the frame (gate 25); Mayor Edrin Vale expression: His skeptical look firms into a measured, doubtful expression.
- [27] Osric Pell: Your Honor, will you let a sword for hire and his hangers-on shame the town’s own guard, when their doubts cannot undo his oath?
- frame judge after line 27: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: His skeptical expression firms as he stresses the guard’s obstructed view.; Osric Pell expression: He turns his appeal toward the mayor, pressing the town’s honor and the companions’ doubts.
- [28] Mayor Edrin Vale: The oath is heard, but it does not settle the charge; I will judge only what the testimony establishes.
- frame judge after line 28: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: His skeptical expression firms into a measured, impartial one.; Osric Pell expression: He presses his accusation with a challenging, insistent look.
- [29] Mayor Edrin Vale: The testimony leaves material doubts, and the oath does not resolve them. Prisoner, have you anything further to say before I judge?
- frame judge after line 29: 25% changed, 25% since the frame (gate 25); Mayor Edrin Vale expression: sternly skeptical, emphasizing the testimony's limits; Mayor Edrin Vale pose: leans forward as he addresses the bound prisoner; Osric Pell expression: pressing and accusatory as he appeals to the mayor; Kael expression: wary, facing the mayor as he is asked to speak
- [30] Kael: You’ve heard the doubts. No one saw me take that dagger, and no witness ties me to the watchman’s wound. Judge what this hall proved—not what they assume.
- frame judge after line 30: 25% changed, 25% since the frame (gate 25); Mayor Edrin Vale expression: He looks grave and doubtful as he weighs the testimony.; Mayor Edrin Vale pose: He remains leaning forward on the raised chair, addressing the prisoner.; Kael expression: He looks wary but resolute as he appeals to the mayor.
- [31] Mayor Edrin Vale: The charge is not proved: you are acquitted and released; the accuser’s claim is dismissed, and the guard’s account is referred for inquiry into his oath.
- plate arrest-judgement-b-4: arrest-judgement-b-4.png is behind the scene (16 regions found, 10 of 11 characters named), with the cast of arrest-judgement-b-2.png
- frame 4 (arrest-judgement-b-4) shown 143.6 s after it started at line 19
- frame judge after line 31: 35% changed, 35% since the frame (gate 25); Mayor Edrin Vale expression: his skeptical look settles into a firm, decisive expression; Mayor Edrin Vale pose: sits upright on the raised chair as he delivers the ruling; Kael expression: his wary expression eases with relief; Kael pose: rises from kneeling as he is released; Osric Pell expression: his guarded expression tightens as his claim is dismissed; Tavin Rusk expression: his defensive look turns uneasy under referral for inquiry; Sergeant Alwen Dorr expression: watches the ruling with a serious, intent expression
- frame 6 (arrest-judgement-b-6) started after line 31, shot wide, editing arrest-hall-real.png
- [32] Kael: Then unbind me, Your Honor. I’ll remember you judged what was proved.
- frame judge after line 32: 10% changed, 10% since the frame (gate 25); Kael expression: His wary expression tightens into a resolute, guarded look as he speaks.
- frame 5 (arrest-judgement-b-5) shown 121.7 s after it started at line 25
- plate arrest-judgement-b-6: arrest-judgement-b-6.png is behind the scene (16 regions found, 10 of 11 characters named), with the cast of arrest-judgement-b-2.png
- frame 6 (arrest-judgement-b-6) shown 161.9 s after it started at line 31
