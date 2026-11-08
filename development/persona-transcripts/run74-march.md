# Run 74: the march to the hall, from the plot (scene 2 of 3)

Scene 2 on march-photo-fresh.png, states carried from the arrest (only the fruit seller is new). The bought guard warns the crowd of the blade's magic ([02]); Nessa calls out that the watchman bled before the arrest ([03]); Kael, bound, asks the sergeant to stop the guard turning the crowd ([14]) and calls him 'Dark-beard', never having learned his name ([26]); the accuser hardens his story to having seen Kael flee ([22]). 28 lines in 8.5 minutes, no refusals, four frames. Found: Nessa told her account three times ([03], [08], [10]) until the no-repeat conduct rule was firmed mid-scene (then [12], [21] of the judgement are short); Kael asked the accuser what the bought guard had claimed ([21]).

- ! ok state market_witness turn=0 mood valence=-8 arousal=50
- !   emotions: trust=35 irritation=18 fear=28 sadness=15 pride=20
- !   knows the names of: nobody
- ! ok state market_witness turn=0 mood valence=-8 arousal=50
- !   emotions: trust=35 irritation=18 fear=28 sadness=15 pride=20
- !   knows the names of: market_factor
- ! ok state market_witness turn=0 mood valence=-8 arousal=50
- !   emotions: trust=35 irritation=18 fear=28 sadness=15 pride=20
- !   knows the names of: market_factor market_guard_bought
- ! ok state market_witness turn=0 mood valence=-8 arousal=50
- !   emotions: trust=35 irritation=18 fear=28 sadness=15 pride=20
- !   knows the names of: market_factor market_guard_bought market_guard_lawful

**[01] Sergeant_Dorr:** Keep the passage clear. The prisoner will be brought before the mayor for a fair hearing; the dagger and sightline may be examined there.

**[02] Tavin:** Keep back from him. That blade’s magic makes him dangerous.

**[03] Nessa:** I saw the watchman clutching his bleeding arm before the guards seized the man; I can’t say who wounded him, sir.
- behavior: guarded
- fired triggers: risk-nearby
- fired emotional triggers: retaliation-fear (by state: none)
- tone: npc
- feels: fear 51, trust 35, irritation 32; mood -22 valence, 56 arousal; focus market_guard_bought on warning about the blade
- time: line 6.9s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6937 ms

**[04] Sergeant_Dorr:** That account does not identify who wounded him; it will be entered for the hearing, and the prisoner remains in custody by order.
- behavior: procedural-explanation
- fired triggers: asked-for-basis
- fired emotional triggers: evidence-questioned (by state: none)
- tone: npc
- feels: trust 77, irritation 68, pride 68; mood 16 valence, 53 arousal; toward market_guard_bought: indifferent (2); toward sella_adventurer: indifferent (2); toward warrior: indifferent (6); toward bram_adventurer: indifferent (8); toward market_witness: indifferent (1); focus market_witness on witness account of the watchman’s injury
- time: line 6.9s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6909 ms

**[05] Jory:** What did you actually see the bound man do with the blade?
- fired triggers: risk-unclear

**[06] Tavin:** I saw him flee the stall with the blade. That is my account.
- behavior: questioned
- behavior override: trigger scrutiny-of-account fired, so the line took questioned, not defensive
- fired triggers: scrutiny-of-account, oath-doubted
- fired emotional triggers: accused-of-lying (by state: none)
- tone: danger
- feels: irritation 93, fear 79, pride 45; mood -33 valence, 63 arousal; toward warrior: indifferent (-3); toward jory_adventurer: indifferent (-1); focus warrior on what the bound man did with the blade
- time: line 7.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7890 ms

**[07] Jory:** You said you saw him flee. Could you see his face and the blade clearly through the crowd?
- behavior: wary
- fired triggers: risk-unclear
- tone: npc
- feels: irritation 49, trust 45, fear 43, joy 42, pride 34, amusement 34, affection 30; mood 23 valence, 54 arousal; focus market_guard_bought on the guard’s account of the flight
- time: line 8.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 8830 ms

**[08] Nessa:** I saw the watchman clutching his bleeding arm before the guards seized the man. I can’t say who wounded him.

**[09] Sergeant_Dorr:** That account will be entered for the hearing; it does not identify who wounded him.
- behavior: procedural-explanation
- tone: ally
- feels: trust 78, pride 63, irritation 60; mood 17 valence, 51 arousal; toward market_guard_bought: indifferent (2); toward sella_adventurer: indifferent (2); toward warrior: indifferent (6); toward bram_adventurer: indifferent (8); toward market_witness: indifferent (3); focus market_witness on the watchman’s injury
- time: line 6.7s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6754 ms

**[10] Nessa:** I saw the watchman clutching his arm before the guards seized him; I can’t say what the bound man did with the blade, sir.
- behavior: careful-witness
- worded again: Repeats her earlier account of seeing the watchman clutching his arm before the guards seized the man.
- tone: npc
- feels: fear 48, trust 37, irritation 27; mood -19 valence, 55 arousal; toward market_guard_lawful: indifferent (1); focus market_guard_bought on the guard’s warning
- time: line 14.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 14808 ms

**[11] Mara:** When did you see him clutching his arm—before the guards seized the man, or after?

**[12] Nessa:** It was before the guards seized him, sir. I saw the watchman clutching his arm then.
- behavior: careful-witness
- worded again: Repeats her own account of seeing the watchman clutching his arm before the guards seized the man.
- tone: npc
- feels: fear 46, trust 39, irritation 25; mood -16 valence, 54 arousal; toward market_guard_lawful: indifferent (1); toward market_guard_bought: indifferent (1); focus market_guard_bought on the guard’s warning
- time: line 13.6s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 13659 ms

**[13] Mara:** That places his injury before the seizure, but it still doesn’t show who caused it.
- behavior: even-keeled
- fired emotional triggers: fear-eases (by state: fear-eases)
- tone: ally
- feels: trust 58, pride 31, affection 24; mood 38 valence, 42 arousal; toward weapon_seller: indifferent (4); toward market_witness: indifferent (4); focus market_witness on when the watchman was injured
- time: line 6.6s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6609 ms

**[14] Kael:** My hands are bound. Keep that dark-bearded guard from turning this crowd against me.
- fired triggers: accused-before-judge

**[15] Sergeant_Dorr:** Keep the passage clear. The crowd will not decide this; you will have your hearing before the mayor.
- behavior: market-control
- fired triggers: crowd-disrupts
- fired emotional triggers: rising-disorder (by state: none)
- tone: ally
- feels: trust 79, irritation 71, pride 59, fear 35; mood 6 valence, 54 arousal; toward market_guard_bought: indifferent (2); toward sella_adventurer: indifferent (2); toward warrior: indifferent (7); toward bram_adventurer: indifferent (8); toward market_witness: indifferent (3); focus warrior on keeping the crowd orderly
- time: line 6.5s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6544 ms

**[16] Kael:** Then keep him from steering the crowd till we reach it. I’ll answer for myself before the mayor.
- behavior: fights-for-his-life
- fired triggers: accused-before-judge
- tone: ally
- feels: irritation 70, pride 37, trust 27, joy 25, fear 24; mood -1 valence, 57 arousal; toward market_factor: indifferent (-7); toward weapon_seller: indifferent (2); toward market_guard_bought: indifferent (-5); toward market_guard_lawful: indifferent (2); focus market_guard_lawful on the promised hearing
- time: line 6.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6831 ms

**[17] Bram:** Sergeant, can we make the march without telling the crowd this charge is already settled?
- fired triggers: danger-calls

**[18] Sergeant_Dorr:** Yes. The charge is not settled; the prisoner will answer before the mayor, and the crowd will keep order on the march.
- behavior: procedural-explanation
- fired emotional triggers: hearing-requested (by state: none)
- tone: ally
- feels: trust 85, irritation 63, pride 62, fear 33; mood 13 valence, 52 arousal; toward market_guard_bought: indifferent (2); toward sella_adventurer: indifferent (2); toward warrior: indifferent (7); toward bram_adventurer: indifferent (11); toward market_witness: indifferent (3); focus bram_adventurer on fair hearing and orderly march
- time: line 7.4s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7409 ms

**[19] Bram:** Good. Keep the charge unsettled before the crowd, and let the mayor hear the witness first.
- behavior: practical
- fired triggers: danger-calls
- tone: ally
- feels: trust 55, pride 38, affection 32, amusement 28; mood 33 valence, 41 arousal; toward market_guard_lawful: indifferent (9); focus market_guard_lawful on fairness of the march
- time: line 6.4s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6383 ms

**[20] Osric:** What would you have the market hear of your part in this, before the mayor hears your account?
- fired triggers: charge-questioned

**[21] Kael:** My part? I did not take that dagger or wound the watchman. You say you saw me flee—could you see my face and the blade clearly through that crowd?
- behavior: fights-for-his-life
- fired triggers: accused-before-judge
- tone: danger
- feels: irritation 72, pride 39, trust 26, joy 25, fear 23; mood -1 valence, 57 arousal; toward market_factor: indifferent (-9); toward weapon_seller: indifferent (2); toward market_guard_bought: indifferent (-5); toward market_guard_lawful: indifferent (2); focus market_factor on his part in the accusation
- time: line 7.1s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7080 ms

**[22] Osric:** The crowd did not obscure the man or the blade; I saw you flee with it, and the charge stands for the mayor to judge.
- behavior: presses-the-charge
- fired triggers: charge-questioned
- fired emotional triggers: public-accusation (by state: none)
- tone: ally
- feels: pride 91, irritation 69, trust 35; mood 10 valence, 55 arousal; toward warrior: indifferent (-2); focus warrior on his sightline
- time: line 6.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6836 ms

**[23] Sella:** Market factor, what did you see yourself before the guards took Kael?
- fired triggers: threat-nearby

**[24] Osric:** I saw the man and the blade clearly; the charge stands for proper judgment, does it not?
- behavior: presses-the-charge
- fired triggers: charge-questioned
- fired emotional triggers: public-accusation (by state: none)
- tone: danger
- feels: pride 94, irritation 78, trust 35; mood 8 valence, 55 arousal; toward warrior: indifferent (-2); toward sella_adventurer: indifferent (0); focus sella_adventurer on his own sighting
- time: line 7.3s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7306 ms

**[25] Sella:** A charge isn’t proof. The sergeant said it rests on your account, not what she saw herself.
- behavior: watchful
- fired triggers: threat-nearby
- fired emotional triggers: uncertainty-sharpens-her (by state: none)
- tone: ally
- feels: trust 56, pride 44, joy 42, affection 40, fear 33, irritation 26; mood 33 valence, 51 arousal; toward market_guard_lawful: indifferent (6); focus market_factor on the factor’s claimed eyewitness account
- time: line 9.6s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 9670 ms

**[26] Kael:** Dark-beard, the fruit seller saw him clutching his arm before you seized me. How does that timing prove I wounded him?
- fired triggers: accused-before-judge

**[27] Tavin:** It proves nothing about who wounded him. I saw you flee with the dagger; that is my account.
- behavior: questioned
- behavior override: trigger scrutiny-of-account fired, so the line took questioned, not defensive
- fired triggers: scrutiny-of-account, oath-doubted
- fired emotional triggers: accused-of-lying (by state: none)
- tone: danger
- feels: irritation 97, fear 84, pride 48; mood -34 valence, 64 arousal; toward warrior: indifferent (-5); toward jory_adventurer: indifferent (-1); focus warrior on injury and seizure timing
- time: line 7.0s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6989 ms

**[28] Kael:** Then say where you saw me, and who saw me take it. The seller saw the watchman bleeding before you seized me.
- behavior: fights-for-his-life
- fired triggers: accused-before-judge
- tone: danger
- feels: irritation 65, pride 45, joy 25, trust 24, fear 21; mood 1 valence, 57 arousal; toward market_factor: indifferent (-9); toward weapon_seller: indifferent (2); toward market_guard_bought: indifferent (-7); toward market_guard_lawful: indifferent (2); focus market_guard_bought on the guard’s accusation
- time: line 7.4s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7441 ms
- --- frames (artifacts/tmp/arrest/seq/run74-march-frames.log) ---
- [01] Sergeant Alwen Dorr: Keep the passage clear. The prisoner will be brought before the mayor for a fair hearing; the dagger and sightline may be examined there.
- frame judge after line 1: 0% changed, 0% since the frame (gate 25)
- [02] Tavin Rusk: Keep back from him. That blade’s magic makes him dangerous.
- frame judge after line 2: 25% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: sternly directs the crowd to keep the passage clear; Tavin Rusk expression: warns the crowd back with a guarded, wary look
- [03] Nessa Vale: I saw the watchman clutching his bleeding arm before the guards seized the man; I can’t say who wounded him, sir.
- frame judge after line 3: 20% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr pose: keeps the passage clear, holding the warrant and spear as she leads the prisoner toward the hall; Tavin Rusk expression: looks wary, warning the others to keep back from the bound man; Nessa Vale expression: speaks cautiously, her face troubled as she recounts what she saw; the room position: the group continues uphill toward the town hall, with townsfolk watching from doorways and windows
- [04] Sergeant Alwen Dorr: That account does not identify who wounded him; it will be entered for the hearing, and the prisoner remains in custody by order.
- frame judge after line 4: 20% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: Her expression is firm and composed as she gives the order.; Sergeant Alwen Dorr pose: She keeps the passage clear and holds the sealed warrant while directing the prisoner toward a fair hearing.; Tavin Rusk expression: He looks wary, warning others to keep back from the prisoner.; Nessa Vale expression: She looks troubled and guarded as she gives her uncertain account.; Nessa Vale pose: She indicates the injured watchman’s arm as she explains what she saw.; Osric Pell expression: He looks dissatisfied that the account does not identify the culprit.
- [05] Jory: What did you actually see the bound man do with the blade?
- frame judge after line 5: 20% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: sternly directing the passage to be kept clear; Tavin Rusk pose: warning the crowd back from the prisoner; Nessa Vale expression: serious and uncertain as she recounts what she saw; Sergeant Alwen Dorr pose: holding the sealed warrant as she orders the prisoner kept in custody; Jory pose: questioning the bound man about what he did with the dagger
- [06] Tavin Rusk: I saw him flee the stall with the blade. That is my account.
- frame judge after line 6: 20% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: The sergeant looks firm and authoritative as she orders the passage kept clear.; Tavin Rusk expression: The broad guard looks wary and protective as he warns the crowd away from the prisoner.; Nessa Vale expression: The fruit seller looks uncertain as she recounts what she witnessed.; Jory expression: The dark-haired companion watches the bound man intently while asking what he did with the blade.; Osric Pell expression: The heavy bearded merchant looks doubtful and guarded as he gives his account.
- [07] Jory: You said you saw him flee. Could you see his face and the blade clearly through the crowd?
- frame judge after line 7: 25% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: Her expression is firm as she keeps the passage clear.; Tavin Rusk pose: He keeps the bound man close and guarded, holding back anyone approaching.; Nessa Vale expression: She looks uneasy as she gives her account.; Osric Pell expression: He looks intent and watchful beside the prisoner.; Jory pose: He turns toward the guard and questions him directly.; Tavin Rusk expression: He looks guarded under Jory’s questioning.
- [08] Nessa Vale: I saw the watchman clutching his bleeding arm before the guards seized the man. I can’t say who wounded him.
- frame judge after line 8: 25% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: speaks firmly, keeping the passage clear; Tavin Rusk pose: keeps the bound man close and guarded; Osric Pell expression: looks wary as he insists the blade makes the prisoner dangerous; Nessa Vale expression: speaks cautiously, unable to identify who wounded the watchman; Jory pose: questions the witness and accuser about what they saw; Osric Pell expression: faces Jory's challenge with a guarded look
- [09] Sergeant Alwen Dorr: That account will be entered for the hearing; it does not identify who wounded him.
- frame judge after line 9: 25% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: Keeps a firm, measured expression as she addresses the crowd.; Tavin Rusk pose: Turns toward the bound man and holds a guarded, warning posture.; Nessa Vale expression: Looks sober and uncertain as she recounts what she saw.; Jory pose: Questions the witnesses, focused on the bound man and blade.; Osric Pell expression: Watches the testimony with a stern, intent look.
- [10] Nessa Vale: I saw the watchman clutching his arm before the guards seized him; I can’t say what the bound man did with the blade, sir.
- frame judge after line 10: 10% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: The sergeant looks firm and attentive as she states the prisoner will remain in custody.; Tavin Rusk expression: The broad guard looks wary as he warns that the blade makes the prisoner dangerous.; Nessa Vale expression: The fruit seller looks troubled and uncertain as she recounts seeing the injured watchman, but cannot identify who wounded him.; Jory pose: Jory leans into the questioning, pressing the guard on what he saw through the crowd.; Tavin Rusk expression: The broad guard looks guarded as he gives his account that the bound man fled with the blade.
- [11] Mara: When did you see him clutching his arm—before the guards seized the man, or after?
- frame judge after line 11: 15% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: Turns a questioning, attentive look toward Nessa.; Nessa Vale expression: Looks uncertain and guarded as she repeats what she saw.; Mara pose: Leans toward Nessa as she asks when the witness saw the watchman clutching his arm.; Jory pose: Faces Tavin, pressing him with a skeptical question.; Tavin Rusk expression: Looks defensive under Jory’s questioning.
- [12] Nessa Vale: It was before the guards seized him, sir. I saw the watchman clutching his arm then.
- frame judge after line 12: 35% changed, 35% since the frame (gate 25); Sergeant Alwen Dorr expression: The sergeant watches the witness intently, her expression firm.; Nessa Vale expression: The fruit seller looks uneasy as she repeats what she saw.; Mara pose: The seasoned scout questions the witness, one hand raised slightly.; Osric Pell expression: The merchant's face is guarded as the testimony is clarified.
- frame 1 (march-photo-a-1) started after line 12, shot wide, editing march-photo-fresh.png
- [13] Mara: That places his injury before the seizure, but it still doesn’t show who caused it.
- frame judge after line 13: 0% changed, 0% since the frame (gate 25)
- [14] Kael: My hands are bound. Keep that dark-bearded guard from turning this crowd against me.
- frame judge after line 14: 10% changed, 10% since the frame (gate 25); Mara expression: speaking with a serious, questioning look; Kael expression: speaking with a tense, pleading expression; Kael pose: bound hands held visible as he appeals to the crowd; Tavin Rusk expression: the dark-bearded guard is the focus of the warrior’s appeal
- [15] Sergeant Alwen Dorr: Keep the passage clear. The crowd will not decide this; you will have your hearing before the mayor.
- frame judge after line 15: 30% changed, 30% since the frame (gate 25); Mara expression: Her face is serious and questioning.; Kael expression: He looks tense and appeals to the crowd, his bound hands visible.; Tavin Rusk pose: He continues dragging the bound blond man uphill by the arm.; Sergeant Alwen Dorr pose: She strides ahead, keeping the passage clear and directing the hearing toward the mayor.
- frame 2 (march-photo-a-2) started after line 15, shot wide, editing march-photo-fresh.png
- [16] Kael: Then keep him from steering the crowd till we reach it. I’ll answer for myself before the mayor.
- frame judge after line 16: 0% changed, 0% since the frame (gate 25)
- [17] Bram: Sergeant, can we make the march without telling the crowd this charge is already settled?
- frame judge after line 17: 8% changed, 8% since the frame (gate 25); Kael expression: speaks with a firm, resolute expression; Bram expression: looks doubtful and questioning as he addresses the sergeant
- [18] Sergeant Alwen Dorr: Yes. The charge is not settled; the prisoner will answer before the mayor, and the crowd will keep order on the march.
- frame judge after line 18: 30% changed, 30% since the frame (gate 25); Tavin Rusk pose: continues dragging the bound blond man uphill by the arm; Kael expression: looks resolute as he says he will answer before the mayor; Bram expression: looks concerned as he questions whether the crowd should hear the charge is settled; Sergeant Alwen Dorr expression: answers calmly and firmly that the charge is not settled; Sergeant Alwen Dorr pose: directs the orderly march toward the mayor
- [19] Bram: Good. Keep the charge unsettled before the crowd, and let the mayor hear the witness first.
- frame judge after line 19: 25% changed, 30% since the frame (gate 25); Tavin Rusk pose: keeps the prisoner moving without turning him toward the crowd; Kael expression: looks resolved, ready to answer for himself before the mayor; Bram expression: looks relieved, then intent on keeping the hearing fair; Bram pose: addresses the sergeant, asking that the witness be heard first; Sergeant Alwen Dorr expression: calmly assures them the charge is unsettled and the march orderly; Sergeant Alwen Dorr pose: continues leading the way toward the mayor while keeping the passage clear
- [20] Osric Pell: What would you have the market hear of your part in this, before the mayor hears your account?
- frame judge after line 20: 10% changed, 30% since the frame (gate 25); Kael expression: the blond bearded man looks resolute as he answers for himself; Bram expression: the red-bearded man looks earnest, then approving; Sergeant Alwen Dorr expression: the lean dark-haired woman sergeant looks firm as she explains the charge remains unsettled; Osric Pell expression: the heavy bearded merchant turns a guarded, questioning look toward the dark-haired woman
- [21] Kael: My part? I did not take that dagger or wound the watchman. You say you saw me flee—could you see my face and the blade clearly through that crowd?
- frame judge after line 21: 25% changed, 30% since the frame (gate 25); Kael expression: looks defensive and intent on the witness; Osric Pell expression: questions the blond man with a guarded, challenging look; Sergeant Alwen Dorr expression: calmly assures them the charge is unsettled; Tavin Rusk pose: continues guiding the bound man uphill; Bram expression: looks approving after the sergeant's answer
- [22] Osric Pell: The crowd did not obscure the man or the blade; I saw you flee with it, and the charge stands for the mayor to judge.
- frame judge after line 22: 40% changed, 40% since the frame (gate 25); Sergeant Alwen Dorr expression: Keeps a composed, authoritative expression as she confirms the charge is unsettled.; Kael expression: Looks defensive and indignant, denying the theft and asking whether the witness could see clearly.; Osric Pell expression: Looks firm and certain as he insists he saw the warrior flee with the blade.; Bram pose: Addresses the sergeant, then urges that the witness be heard before the mayor.; Nessa Vale expression: Looks resolute as she stands by her account.; the room prop: The bound prisoner is being marched toward the mayor with the charge explicitly still unsettled.
- [23] Sella: Market factor, what did you see yourself before the guards took Kael?
- plate march-photo-a-1: march-photo-a-1.png is behind the scene (16 regions found, 8 of 9 characters named), with the cast of march-photo-fresh.png
- frame 1 (march-photo-a-1) shown 133.9 s after it started at line 12
- frame judge after line 23: 25% changed, 40% since the frame (gate 25); Kael expression: His face is defensive and questioning as he challenges the witness's view.; Osric Pell expression: He looks firm and guarded as he insists the charge stands.; Bram expression: He looks relieved and approving.; Sergeant Alwen Dorr pose: She keeps the crowd orderly and listens to the exchange.; Sella pose: She turns her attention from the fruit seller toward the market factor and asks what he saw.
- frame 3 (march-photo-a-3) started after line 23, shot over-the-shoulder, editing march-photo-fresh.png
- [24] Osric Pell: I saw the man and the blade clearly; the charge stands for proper judgment, does it not?
- frame judge after line 24: 0% changed, 0% since the frame (gate 25)
- [25] Sella: A charge isn’t proof. The sergeant said it rests on your account, not what she saw herself.
- frame judge after line 25: 10% changed, 10% since the frame (gate 25); Osric Pell expression: his guarded face turns questioning as he seeks agreement; Sella expression: her expression is firm and skeptical as she rebuts the charge
- [26] Kael: Dark-beard, the fruit seller saw him clutching his arm before you seized me. How does that timing prove I wounded him?
- frame 2 (march-photo-a-2) shown 139.0 s after it started at line 15
- frame judge after line 26: 25% changed, 25% since the frame (gate 25); Osric Pell expression: His guarded face tightens as Osric asks whether the charge stands for judgment.; Tavin Rusk expression: He looks more closely at the factor as the account is challenged.; Sergeant Alwen Dorr expression: She listens with a narrowed, questioning look.; Kael expression: He looks intent and challenging as he questions whether the timing proves the wound.; Sella expression: She looks firm and skeptical as she says the charge is not proof.
- [27] Tavin Rusk: It proves nothing about who wounded him. I saw you flee with the dagger; that is my account.
- frame judge after line 27: 40% changed, 40% since the frame (gate 25); Osric Pell expression: His guarded face tightens into a questioning look as he asks whether the charge stands for judgment.; Sella expression: She looks skeptical, insisting that the charge is not proof.; Kael expression: He looks sharply questioning as he challenges how the timing proves he wounded the man.; Tavin Rusk expression: He looks firm and certain as he says he saw the blond man flee with the dagger.; Nessa Vale expression: She watches the exchange from beside her apple cart.
- frame 4 (march-photo-a-4) started after line 27, shot wide, editing march-photo-fresh.png
- [28] Kael: Then say where you saw me, and who saw me take it. The seller saw the watchman bleeding before you seized me.
- frame judge after line 28: 10% changed, 10% since the frame (gate 25); Kael expression: His questioning look sharpens as he asks for the place and witnesses.; Tavin Rusk expression: His face tightens at the accusation that he seized Kael after seeing a wounded watchman.
- frame 3 (march-photo-a-3) shown 169.2 s after it started at line 23
- plate march-photo-a-4: march-photo-a-4.png is behind the scene (16 regions found, 8 of 9 characters named), with the cast of march-photo-a-2.png
- frame 4 (march-photo-a-4) shown 122.7 s after it started at line 27
