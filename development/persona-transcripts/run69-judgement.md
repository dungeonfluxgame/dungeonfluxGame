# Run 69: the judgement played from plotted moves

The judgement's moves plotted by the storyteller (persona story moves judgement: speaker, to, a second-person goal naming the listener by look, under person-N handles), with the hall header and a verdict beat. The fruit seller's timing and the absent witness carry an acquittal; no name or handle leaked, nothing refused. Found: Haldor, never shown the fake up close, swore it was his own blade (sigils 'not paint'), losing the plot's key flaw; three double answers where a question to someone was followed by their own plotted move ([16]/[17], [20]/[21], [40]/[41]); the defence asked after 'the fresh coin' in the guard's purse with nothing in plain sight to ask from.

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

**[01] Osric:** The charge is theft of the Nightglass dagger and the cutting of the watchman; I lay the displayed dagger and torn cloak strip before the hall as evidence. The prisoner’s denial does not account for them.
- fired triggers: charge-questioned

**[02] Mayor:** The exhibits are received; they do not settle who took the dagger or cut the watchman. What did you see that links the prisoner to either act?
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: npc
- feels: trust 60, pride 52, fear 25; mood 20 valence, 49 arousal; toward market_factor: indifferent (2); focus market_factor on evidence for the charge
- time: line 7.1s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7131 ms

**[03] Jory:** Before it was shown here, who found the dagger, and how did it come into your keeping?
- fired triggers: risk-unclear

**[04] Osric:** The dagger came to me through the proper account of the exhibits; I cannot speak to who found it before it was shown here.
- behavior: careful-deflection
- fired triggers: evidence-questions, charge-questioned
- fired emotional triggers: scrutiny-lingers (by state: none)
- tone: danger
- feels: irritation 97, fear 89, pride 75, trust 35; mood -16 valence, 62 arousal; toward warrior: indifferent (-8); toward market_guard_lawful: indifferent (-2); toward mara_adventurer: indifferent (-1); toward town_mayor: indifferent (-5); toward jory_adventurer: indifferent (-1); focus jory_adventurer on who found the dagger
- time: line 8.9s, backfill started
- covered: false (gaps below)
- - backstory: who found the dagger and how it came into Osric's keeping
- backfill: running (record 1 when done)
- model: codex:gpt-6-luna
- total: 8912 ms

**[05] Mara:** Sergeant, who received the dagger first, and how was it kept before this hearing?

**[06] Sergeant_Dorr:** I can’t identify who first received it. I know only that it was received into custody as evidence and kept for this hearing.
- behavior: procedural-explanation
- fired triggers: asked-for-basis
- tone: ally
- feels: trust 89, pride 56, irritation 27; mood 29 valence, 46 arousal; toward warrior: indifferent (5); toward market_witness: indifferent (3); toward jory_adventurer: indifferent (3); toward market_factor: indifferent (-2); toward sella_adventurer: indifferent (2); toward mara_adventurer: indifferent (12); toward town_mayor: indifferent (2); focus mara_adventurer on the dagger’s custody
- time: line 7.6s, backfill started
- covered: false (gaps below)
- - backstory: who received the dagger first
- - backstory: how the dagger was kept before the hearing
- backfill: running (record 2 when done)
- model: codex:gpt-6-luna
- total: 7596 ms

**[07] Bram:** What mark on that strip identifies it as the accused’s cloak, rather than any torn scrap?

**[08] Osric:** The strip bears no mark I can offer as proof of ownership; its presence with the dagger is part of the account before this hall, and proper judgment must weigh the exhibits together.
- behavior: presses-the-charge
- fired triggers: charge-questioned
- fired emotional triggers: public-accusation (by state: none)
- tone: danger
- feels: irritation 97, pride 85, fear 80, trust 35; mood -13 valence, 61 arousal; toward warrior: indifferent (-8); toward market_guard_lawful: indifferent (-2); toward mara_adventurer: indifferent (-1); toward town_mayor: indifferent (-5); toward jory_adventurer: indifferent (-1); toward bram_adventurer: indifferent (-1); focus bram_adventurer on identifying the cloak strip
- time: line 7.2s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7195 ms

**[09] Sella:** Mara, can you match that torn edge to the accused’s cloak as it appears here?
- fired triggers: threat-nearby

**[10] Mara:** I can’t match it from here; the strip has no mark proving it came from his cloak.
- behavior: even-keeled
- behavior override: trigger fear-eases fired, so the line took even-keeled, not none
- fired emotional triggers: fear-eases (by state: fear-eases)
- tone: ally
- feels: trust 64, pride 32, affection 24; mood 39 valence, 42 arousal; toward market_guard_lawful: indifferent (10); toward market_guard_bought: indifferent (2); toward sella_adventurer: indifferent (2); focus sella_adventurer on whether the strip matches the cloak
- time: line 6.6s, backfill started
- covered: false (gaps below)
- - backstory: whether the torn edge matches the accused’s cloak
- backfill: running (record 3 when done)
- model: codex:gpt-6-luna
- total: 6646 ms

**[11] Jory:** Guard, where were you standing when you saw the warrior flee, and what lay between you and the stall?
- fired triggers: risk-unclear

**[12] Tavin:** I stood by the market post. Nothing lay between me and the stall; I saw him flee plainly.
- behavior: questioned
- behavior override: trigger scrutiny-of-account fired, so the line took questioned, not swears-to-it
- fired triggers: scrutiny-of-account, oath-doubted
- tone: danger
- feels: irritation 91, fear 91, pride 45, embarrassment 35; mood -34 valence, 64 arousal; toward warrior: indifferent (-5); toward market_factor: indifferent (0); toward mara_adventurer: indifferent (-2); toward town_mayor: indifferent (-1); toward bram_adventurer: indifferent (-14); toward jory_adventurer: indifferent (0); focus jory_adventurer on his position and sightline
- time: line 6.4s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6451 ms

**[13] Bram:** From the market post, could you make out the dagger’s handle and marks?

**[14] Tavin:** I saw enough to identify the dagger. The handle and marks were clear from my post.
- behavior: questioned
- fired triggers: scrutiny-of-account, oath-doubted
- tone: danger
- feels: irritation 92, fear 85, pride 45, embarrassment 22; mood -34 valence, 63 arousal; toward warrior: indifferent (-5); toward market_factor: indifferent (0); toward mara_adventurer: indifferent (-2); toward town_mayor: indifferent (-1); toward bram_adventurer: indifferent (-14); toward jory_adventurer: indifferent (0); focus bram_adventurer on dagger sightline
- time: line 6.5s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6542 ms

**[15] Mara:** When did you see the watchman clutching his bleeding arm—before the guards seized Kael, or after?

**[16] Nessa:** Before, ma'am. I saw him clutching his arm near my stall before the guards seized Kael.
- behavior: careful-witness
- tone: npc
- feels: trust 69, irritation 54, fear 50, pride 29; mood -10 valence, 53 arousal; toward sella_adventurer: indifferent (10); toward town_mayor: indifferent (2); focus sella_adventurer on when she saw the watchman
- time: line 6.6s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6686 ms

**[17] Nessa:** I saw the watchman clutching his bleeding arm near my stall before the guards took Kael away, sir. I cannot say who hurt him.
- fired triggers: fair-hearing

**[18] Mayor:** That testimony establishes the watchman was wounded before Kael was seized, but does not identify who struck him. Do you know whether anyone else was near your stall?
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: npc
- feels: trust 71, pride 61, fear 25; mood 24 valence, 48 arousal; toward market_factor: indifferent (2); toward market_witness: indifferent (2); focus market_witness on whether testimony links Kael to the attack
- time: line 7.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7822 ms

**[19] Sella:** Master bladesmith, does that handle fit and those violet sigils match the Nightglass you forged?

**[20] Haldor:** Yes. The close-fitted bone handle and violet sigils match the Nightglass I forged; I can’t say who carried it.
- behavior: careful-appraisal
- fired triggers: appraisal-question
- fired emotional triggers: questioned-claims (by state: none)
- tone: npc
- feels: pride 93, trust 55, joy 42; mood 51 valence, 48 arousal; toward market_guard_lawful: indifferent (1); toward town_mayor: indifferent (5); toward market_factor: indifferent (-4); toward sella_adventurer: indifferent (2); focus sella_adventurer on matching the dagger to his Nightglass
- time: line 7.0s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7042 ms

**[21] Haldor:** The bone is fitted close to the tang, and those violet marks are proper sigils, not paint; they match my Nightglass. I can verify the blade, not who bore it.
- fired triggers: appraisal-question

**[22] Sella:** That proves the blade is yours, not who took it or cut the watchman.
- behavior: easy-alert
- tone: ally
- feels: trust 64, joy 42, affection 40, pride 39, fear 26; mood 40 valence, 47 arousal; toward market_guard_lawful: indifferent (2); toward weapon_seller: indifferent (6); toward market_factor: indifferent (2); focus weapon_seller on what the dagger proves
- time: line 6.7s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6724 ms

**[23] Kael:** Then tell me, factor: what need for proper order brought this charge before the smith could identify his blade?
- fired triggers: accused-before-judge

**[24] Osric:** The market cannot wait on every uncertainty; the dagger and strip warranted a charge, and proper judgment can weigh the account.
- behavior: presses-the-charge
- fired triggers: charge-questioned
- tone: danger
- feels: irritation 97, pride 77, fear 72, trust 35; mood -13 valence, 61 arousal; toward warrior: indifferent (-10); toward market_guard_lawful: indifferent (-2); toward mara_adventurer: indifferent (-1); toward town_mayor: indifferent (-5); toward jory_adventurer: indifferent (-1); toward bram_adventurer: indifferent (-1); focus warrior on why the charge was brought
- time: line 7.0s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6980 ms

**[25] Mara:** Guard, set your purse where this hall can see it, and tell us whose coin it bears.

**[26] Tavin:** My purse bears my own mark. This is an insult to a guard’s oath; the matter is settled.
- behavior: swears-to-it
- behavior override: trigger oath-doubted fired, so the line took swears-to-it, not defensive
- fired triggers: oath-doubted
- tone: danger
- feels: irritation 92, fear 85, pride 45; mood -34 valence, 64 arousal; toward warrior: indifferent (-5); toward market_factor: indifferent (0); toward mara_adventurer: indifferent (-5); toward town_mayor: indifferent (-1); toward bram_adventurer: indifferent (-14); toward jory_adventurer: indifferent (0); focus mara_adventurer on purse and coin
- time: line 6.5s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6474 ms

**[27] Kael:** When did that fresh coin enter your purse, guard—and who gave it to you?
- fired triggers: accused-before-judge

**[28] Tavin:** That coin is mine, and no one gave it me. I’ll answer no more of this insult.
- behavior: swears-to-it
- fired triggers: oath-doubted
- fired emotional triggers: accused-of-lying (by state: none)
- tone: danger
- feels: irritation 96, fear 89, pride 45; mood -35 valence, 64 arousal; toward warrior: indifferent (-7); toward market_factor: indifferent (0); toward mara_adventurer: indifferent (-5); toward town_mayor: indifferent (-1); toward bram_adventurer: indifferent (-14); toward jory_adventurer: indifferent (0); focus mara_adventurer on purse and coin
- time: line 6.9s, backfill started
- covered: false (gaps below)
- - private_backstory: when coin entered purse and who gave it
- backfill: running (record 1 when done)
- model: codex:gpt-6-luna
- total: 6947 ms

**[29] Jory:** Sergeant, what do the charge and custody record say about who brought the dagger in, and when?
- fired triggers: risk-unclear

**[30] Sergeant_Dorr:** The market factor brought the dagger to the guard; I kept it secured with our evidence, separate from Kael and the market goods, before the hearing. The record gives no time beyond that.
- behavior: procedural-explanation
- fired triggers: asked-for-basis
- tone: npc
- feels: trust 89, pride 53, irritation 25; mood 30 valence, 45 arousal; toward warrior: indifferent (5); toward market_witness: indifferent (3); toward jory_adventurer: indifferent (5); toward market_factor: indifferent (-2); toward sella_adventurer: indifferent (2); toward mara_adventurer: indifferent (12); toward town_mayor: indifferent (2); focus jory_adventurer on dagger’s receipt and custody
- time: line 9.3s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 9374 ms

**[31] Bram:** Before you decide whether this charge can stand, will you hear the accused’s account?

**[32] Mayor:** Yes. The accused will be heard before I judge; let him give his account.
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: ally
- feels: trust 78, pride 68, fear 25; mood 26 valence, 47 arousal; toward market_factor: indifferent (2); toward market_witness: indifferent (2); toward bram_adventurer: indifferent (3); focus bram_adventurer on hearing Kael’s account
- time: line 6.4s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6389 ms

**[33] Kael:** I stood by the stalls with my companions, tending my gear and keeping watch. I stole no dagger, and I never wounded the watchman.
- fired triggers: accused-before-judge

**[34] Mayor:** Your denial is heard; tell me what you observed while keeping watch, and whether your companions can confirm it.
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: npc
- feels: trust 84, pride 73, fear 25; mood 28 valence, 47 arousal; toward market_factor: indifferent (2); toward market_witness: indifferent (2); toward bram_adventurer: indifferent (3); toward warrior: indifferent (2); focus warrior on Kael’s account
- time: line 6.6s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6644 ms

**[35] Kael:** Factor, what evidence besides the strip puts me at the watchman’s side when he was hurt?
- fired triggers: accused-before-judge

**[36] Osric:** The strip and the dagger put you at the smith’s stall; the watchman’s wound fits the charge, and proper judgment can weigh the account.
- behavior: presses-the-charge
- fired triggers: charge-questioned
- fired emotional triggers: public-accusation (by state: none)
- tone: danger
- feels: irritation 97, pride 91, fear 65, trust 35; mood -9 valence, 60 arousal; toward warrior: indifferent (-13); toward market_guard_lawful: indifferent (-2); toward mara_adventurer: indifferent (-1); toward town_mayor: indifferent (-5); toward jory_adventurer: indifferent (-1); toward bram_adventurer: indifferent (-1); focus warrior on evidence linking Kael to the wound
- time: line 8.4s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 8423 ms

**[37] Mara:** Mayor, will you let this hall consider whether a charge resting on an unverified blade and a blocked sightline can safely stand?

**[38] Mayor:** That is relevant; the hall may consider whether the evidence supports the charge, but no charge stands or falls on assertion alone.
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: ally
- feels: trust 88, pride 78, fear 25; mood 29 valence, 47 arousal; toward market_factor: indifferent (2); toward market_witness: indifferent (2); toward bram_adventurer: indifferent (3); toward warrior: indifferent (2); toward mara_adventurer: indifferent (3); focus mara_adventurer on relevance of evidence
- time: line 6.6s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6648 ms

**[39] Kael:** Mayor, may I speak once more before you rule?
- fired triggers: accused-before-judge

**[40] Mayor:** You may speak once more; keep your words to what bears on the charge.
- behavior: measured-judge
- fired emotional triggers: relevant-question (by state: none)
- tone: ally
- feels: trust 90, pride 81, fear 25; mood 30 valence, 46 arousal; toward market_factor: indifferent (2); toward market_witness: indifferent (2); toward bram_adventurer: indifferent (3); toward warrior: indifferent (4); toward mara_adventurer: indifferent (3); focus warrior on request to speak
- time: line 6.3s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6311 ms

**[41] Mayor:** You may give your last word, sir; keep it to what bears on the charge.

**[42] Kael:** The strip proves my cloak tore here. It proves nothing about the dagger or the watchman. I ask you to judge only what the evidence shows.
- behavior: fights-for-his-life
- fired triggers: accused-before-judge
- tone: ally
- feels: trust 57, pride 49, irritation 43, joy 38, fear 22; mood 19 valence, 52 arousal; toward market_factor: indifferent (-16); toward market_guard_lawful: indifferent (1); toward weapon_seller: indifferent (1); toward market_guard_bought: indifferent (-1); toward mara_adventurer: indifferent (3); toward bram_adventurer: indifferent (4); toward town_mayor: indifferent (14); focus town_mayor on final word on the charge
- time: line 6.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6825 ms

**[43] Mayor:** The evidence does not establish that you stole the dagger or wounded the watchman; the charge is dismissed, you are to be released, the factor’s accusation is entered as unproven, and the guard’s account remains under inquiry.
- fired triggers: material-doubt

**[44] Kael:** Then release me, and let the guard answer for his account.
- behavior: fights-for-his-life
- fired triggers: accused-before-judge
- tone: ally
- feels: trust 62, joy 52, pride 46, irritation 38, fear 21; mood 25 valence, 52 arousal; toward market_factor: indifferent (-16); toward market_guard_lawful: indifferent (1); toward weapon_seller: indifferent (1); toward market_guard_bought: indifferent (-1); toward mara_adventurer: indifferent (3); toward bram_adventurer: indifferent (4); toward town_mayor: indifferent (19); focus town_mayor on release and dismissed charge
- time: line 7.0s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7025 ms
- --- frames (artifacts/tmp/arrest/judgement3-frames.log) ---
- [01] Osric Pell: The charge is theft of the Nightglass dagger and the cutting of the watchman; I lay the displayed dagger and torn cloak strip before the hall as evidence. The prisoner’s denial does not account for them.
- frame judge after line 1: 30% changed, 30% since the frame (gate 25); Osric Pell pose: holds the displayed dagger and torn cloak strip out before the hall as evidence
- frame 1 (arrest-judgement-c-1) started after line 1, shot wide, editing arrest-hall-real.png
- [02] Mayor Edrin Vale: The exhibits are received; they do not settle who took the dagger or cut the watchman. What did you see that links the prisoner to either act?
- frame judge after line 2: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: Leans forward with an intent, questioning look as he addresses the room.
- [03] Jory: Before it was shown here, who found the dagger, and how did it come into your keeping?
- frame judge after line 3: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: leans forward with a stern, questioning look; Jory expression: looks toward the dagger with a questioning expression
- [04] Osric Pell: The dagger came to me through the proper account of the exhibits; I cannot speak to who found it before it was shown here.
- frame judge after line 4: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: His stern gaze is fixed on the witnesses as he leans forward, questioning them.; Osric Pell pose: He holds the displayed dagger and torn cloak strip steady before the hall, answering the question.; Jory pose: He directs a questioning look toward the merchant, asking who found the dagger.
- [05] Mara: Sergeant, who received the dagger first, and how was it kept before this hearing?
- frame judge after line 5: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: Keeps a stern, questioning look as he leans forward.; Jory pose: Questions the merchant from the rail.; Osric Pell expression: Looks guarded as he answers.; Mara pose: Turns her attention to the sergeant and asks about the dagger's custody.
- [06] Sergeant Alwen Dorr: I can’t identify who first received it. I know only that it was received into custody as evidence and kept for this hearing.
- frame judge after line 6: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale pose: leans forward, questioning the prisoner and witnesses; Osric Pell expression: holds the displayed dagger and cloak strip, guarded and noncommittal; Sergeant Alwen Dorr expression: answers with a restrained, matter-of-fact expression; Jory expression: looks toward the dagger as he asks who found it; Mara expression: questions the sergeant with a focused, probing look
- [07] Bram: What mark on that strip identifies it as the accused’s cloak, rather than any torn scrap?
- frame judge after line 7: 15% changed, 15% since the frame (gate 25); Mayor Edrin Vale expression: Leans forward sternly, questioning the evidence.; Jory expression: Looks toward the merchant, intent on the dagger’s chain of custody.; Osric Pell expression: Looks guarded as he answers about the dagger.; Mara expression: Turns her attention to the sergeant, questioning the custody record.; Sergeant Alwen Dorr expression: Answers steadily, unable to identify who first received the dagger.; Bram expression: Looks toward the torn cloak strip, questioning its identifying mark.
- [08] Osric Pell: The strip bears no mark I can offer as proof of ownership; its presence with the dagger is part of the account before this hall, and proper judgment must weigh the exhibits together.
- frame judge after line 8: 10% changed, 15% since the frame (gate 25); Mayor Edrin Vale expression: leans forward with a stern, questioning look; Jory expression: looks toward the merchant as he asks how the dagger was found; Osric Pell expression: keeps a guarded, restrained expression while answering; Mara expression: looks toward the sergeant as she asks about custody; Sergeant Alwen Dorr expression: answers with a steady, matter-of-fact expression; Bram expression: looks toward the merchant as he questions the strip; Osric Pell expression: answers with a firm, defensive expression
- [09] Sella: Mara, can you match that torn edge to the accused’s cloak as it appears here?
- frame judge after line 9: 20% changed, 20% since the frame (gate 25); Mayor Edrin Vale expression: his stern face tightens as he presses for a link to either act; Jory pose: turns toward the merchant and questions how the dagger entered his keeping; Osric Pell expression: answers guardedly, keeping his account to the dagger's formal transfer; Mara pose: addresses the sergeant about the dagger's custody; Sergeant Alwen Dorr expression: answers evenly that she cannot identify who first received it; Bram pose: questions the merchant about a mark on the torn strip; Osric Pell expression: states that the strip has no mark proving ownership; Sella pose: asks Mara whether the torn edge matches the accused's cloak
- [10] Mara: I can’t match it from here; the strip has no mark proving it came from his cloak.
- frame judge after line 10: 15% changed, 20% since the frame (gate 25); Mayor Edrin Vale expression: sternly questioning the hall; Mara expression: uncertain, unable to match the strip; Jory expression: questioning the chain of custody; Bram expression: skeptical of the strip as proof; Sella expression: watching Mara for an answer; Osric Pell expression: guarded, conceding the strip has no ownership mark
- [11] Jory: Guard, where were you standing when you saw the warrior flee, and what lay between you and the stall?
- plate arrest-judgement-c-1: arrest-judgement-c-1.png is behind the scene (16 regions found, 8 of 11 characters named), with the cast of arrest-hall-real.png
- frame 1 (arrest-judgement-c-1) shown 121.4 s after it started at line 1
- frame judge after line 11: 25% changed, 25% since the frame (gate 25); Mayor Edrin Vale expression: sternly questioning, leaning forward; Osric Pell expression: guarded and serious as he answers; Sergeant Alwen Dorr expression: composed, unable to identify the first recipient; Mara expression: focused, looking toward the torn strip; Sella expression: questioning Mara about the strip; Jory expression: questioning the guard
- [12] Tavin Rusk: I stood by the market post. Nothing lay between me and the stall; I saw him flee plainly.
- frame judge after line 12: 0% changed, 25% since the frame (gate 25)
- [13] Bram: From the market post, could you make out the dagger’s handle and marks?
- frame judge after line 13: 25% changed, 25% since the frame (gate 25); Mayor Edrin Vale expression: stern, questioning gaze fixed on the witnesses; Jory pose: leans toward the evidence and questions the factor and guard; Mara expression: serious, looking toward the torn strip; Bram expression: skeptical, eyes on the strip and guard; Sella pose: turns toward Mara as she asks about the strip; Sergeant Alwen Dorr expression: measured, unable to identify the first recipient; Tavin Rusk expression: plainly attentive as he answers from the market post; Osric Pell expression: guarded as he explains the exhibits do not prove ownership; Kael position: remains bound and kneeling, silent during the questions
- [14] Tavin Rusk: I saw enough to identify the dagger. The handle and marks were clear from my post.
- frame judge after line 14: 30% changed, 30% since the frame (gate 25); Mayor Edrin Vale expression: Leans forward with a questioning, stern look.; Jory pose: Addresses the witnesses with an intent, questioning posture.; Mara expression: Looks toward the accused, unable to match the strip.; Bram expression: Questions the lack of identifying marks with a skeptical look.; Sella expression: Looks toward Mara, seeking confirmation.; Osric Pell expression: Answers guardedly, without claiming knowledge of who found the dagger.; Sergeant Alwen Dorr expression: Looks uncertain as she says she cannot identify who first received the dagger.; Tavin Rusk expression: Answers firmly, looking certain of what he saw from the post.
- frame 2 (arrest-judgement-c-2) started after line 14, shot over-the-shoulder, editing arrest-hall-real.png
- [15] Mara: When did you see the watchman clutching his bleeding arm—before the guards seized Kael, or after?
- frame judge after line 15: 10% changed, 10% since the frame (gate 25); Mara expression: Her face is intent and questioning as she asks about the watchman.; Tavin Rusk expression: His expression tightens as he is questioned about the timing.
- [16] Nessa Vale: Before, ma'am. I saw him clutching his arm near my stall before the guards seized Kael.
- frame judge after line 16: 30% changed, 30% since the frame (gate 25); Mara pose: turns toward the fruit seller, questioning her; Nessa Vale expression: answers firmly, looking toward Mara; Nessa Vale pose: indicates the spot near her stall as she describes what she saw
- [17] Nessa Vale: I saw the watchman clutching his bleeding arm near my stall before the guards took Kael away, sir. I cannot say who hurt him.
- frame judge after line 17: 10% changed, 30% since the frame (gate 25); Mayor Edrin Vale expression: Keeps a questioning look on the witness.; Nessa Vale expression: Answers firmly, looking toward the questioner.
- frame 3 (arrest-judgement-c-3) started after line 17, shot wide, editing arrest-hall-real.png
- [18] Mayor Edrin Vale: That testimony establishes the watchman was wounded before Kael was seized, but does not identify who struck him. Do you know whether anyone else was near your stall?
- frame judge after line 18: 0% changed, 0% since the frame (gate 25)
- [19] Sella: Master bladesmith, does that handle fit and those violet sigils match the Nightglass you forged?
- frame judge after line 19: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: continues questioning the witness, intent and probing; Sella expression: turns her attention from Mara toward the bladesmith as she asks about the dagger
- [20] Haldor Brask: Yes. The close-fitted bone handle and violet sigils match the Nightglass I forged; I can’t say who carried it.
- frame judge after line 20: 15% changed, 15% since the frame (gate 25); Mayor Edrin Vale expression: maintains a questioning look toward the fruit seller; Sella position: turns her attention from Mara toward the bladesmith; Haldor Brask expression: answers steadily, unable to identify who carried the dagger
- [21] Haldor Brask: The bone is fitted close to the tang, and those violet marks are proper sigils, not paint; they match my Nightglass. I can verify the blade, not who bore it.
- frame judge after line 21: 22% changed, 22% since the frame (gate 25); Mayor Edrin Vale expression: holds a questioning look toward the fruit seller; Nessa Vale expression: answers firmly, then listens as the bladesmith confirms the dagger’s details; Sella pose: turns her attention from Mara toward the bladesmith and the dagger; Haldor Brask pose: indicates the dagger’s bone handle and violet sigils as he verifies its features
- [22] Sella: That proves the blade is yours, not who took it or cut the watchman.
- plate arrest-judgement-c-2: arrest-judgement-c-2.png is behind the scene (16 regions found, 10 of 11 characters named), with the cast of arrest-hall-real.png
- frame 2 (arrest-judgement-c-2) shown 94.9 s after it started at line 14
- frame judge after line 22: 25% changed, 25% since the frame (gate 25); Mayor Edrin Vale expression: Keeps a questioning look toward the fruit seller.; Nessa Vale expression: Answers firmly, then looks doubtful at the accusation's uncertainty.; Sella expression: Turns a challenging, narrowed look toward the bladesmith.; Haldor Brask item: Holds the Nightglass dagger up for close inspection, its bone handle and violet sigils visible.; Haldor Brask pose: Indicates the close-fitted handle and violet sigils as he explains.; Sella pose: Faces the bladesmith, challenging what the dagger proves.
- frame 4 (arrest-judgement-c-4) started after line 22, shot insert on Nightglass dagger, editing arrest-hall-real.png
- [23] Kael: Then tell me, factor: what need for proper order brought this charge before the smith could identify his blade?
- frame judge after line 23: 6% changed, 6% since the frame (gate 25); Kael expression: Turns his questioning look toward the factor.
- [24] Osric Pell: The market cannot wait on every uncertainty; the dagger and strip warranted a charge, and proper judgment can weigh the account.
- frame judge after line 24: 25% changed, 25% since the frame (gate 25); Kael expression: looks toward the factor with a questioning expression; Osric Pell expression: answers with a firm, guarded look
- [25] Mara: Guard, set your purse where this hall can see it, and tell us whose coin it bears.
- frame judge after line 25: 30% changed, 30% since the frame (gate 25); Osric Pell expression: answers with a guarded, matter-of-fact look; Mara pose: addresses the guard and gestures for him to show his purse; Tavin Rusk pose: is prompted to set his purse out for the hall to see
- frame 5 (arrest-judgement-c-5) started after line 25, shot wide, editing arrest-hall-real.png
- [26] Tavin Rusk: My purse bears my own mark. This is an insult to a guard’s oath; the matter is settled.
- plate arrest-judgement-c-3: arrest-judgement-c-3.png is behind the scene (16 regions found, 7 of 11 characters named), with the cast of arrest-hall-real.png
- frame 3 (arrest-judgement-c-3) shown 120.8 s after it started at line 17
- frame judge after line 26: 10% changed, 10% since the frame (gate 25); Tavin Rusk expression: Looks offended and resolute as he speaks.
- [27] Kael: When did that fresh coin enter your purse, guard—and who gave it to you?
- insert arrest-judgement-c-4: arrest-judgement-c-4.png is behind the scene, with the cast of arrest-hall-real.png; no details backfilled (refused (private_detail): the model judged that the backfill gives away the character's private side in a public field, so nothing was written (a judgement of meaning, not a guarantee))
- frame 4 (arrest-judgement-c-4) shown 66.5 s after it started at line 22
- frame judge after line 27: 15% changed, 15% since the frame (gate 25); Tavin Rusk expression: defensive, lips pressed as he insists the matter is settled; Kael expression: questioning, gaze fixed on the guard; Tavin Rusk pose: keeps his purse presented for the hall to see
- [28] Tavin Rusk: That coin is mine, and no one gave it me. I’ll answer no more of this insult.
- frame judge after line 28: 25% changed, 25% since the frame (gate 25); Tavin Rusk expression: He looks defensive, jaw set as he insists the coin is his.; Kael expression: He fixes the guard with a questioning, challenging look.
- [29] Jory: Sergeant, what do the charge and custody record say about who brought the dagger in, and when?
- back to arrest-judgement-c-3.png after the insert (29.4 s up)
- frame judge after line 29: 20% changed, 25% since the frame (gate 25); Tavin Rusk expression: His jaw is set and his eyes harden defensively.; Jory pose: He leans toward the witness, asking about the charge and custody record.; Mara expression: She watches the guard with a firm, questioning look.; Sergeant Alwen Dorr expression: Her uncertain look sharpens as she listens to the question.
- [30] Sergeant Alwen Dorr: The market factor brought the dagger to the guard; I kept it secured with our evidence, separate from Kael and the market goods, before the hearing. The record gives no time beyond that.
- frame judge after line 30: 30% changed, 30% since the frame (gate 25); Tavin Rusk expression: Defiant, jaw set as he refuses further answers.; Kael expression: Fixed, questioning look on the guard.; Jory pose: Turns toward the sergeant and asks about the record.; Sergeant Alwen Dorr expression: Looks toward Jory, answering evenly.; the room prop: The dagger remains secured separately with the guard's evidence; no time is established in the record.
- frame 6 (arrest-judgement-c-6) started after line 30, shot over-the-shoulder, editing arrest-hall-real.png
- [31] Bram: Before you decide whether this charge can stand, will you hear the accused’s account?
- frame judge after line 31: 10% changed, 10% since the frame (gate 25); Bram expression: Bram's skeptical look eases as he turns attention to the accused's account.; Kael expression: The bound blond bearded man in the fur mantle looks ready to give his account.
- [32] Mayor Edrin Vale: Yes. The accused will be heard before I judge; let him give his account.
- frame judge after line 32: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: His questioning look settles into a firm, attentive expression.; Kael pose: He remains kneeling, poised to give his account.
- [33] Kael: I stood by the stalls with my companions, tending my gear and keeping watch. I stole no dagger, and I never wounded the watchman.
- frame judge after line 33: 30% changed, 30% since the frame (gate 25); Mayor Edrin Vale expression: his questioning look settles into a firm, attentive expression; Kael expression: he looks steadily toward the mayor as he gives his account; Kael pose: still kneeling and bound, he speaks without further movement
- [34] Mayor Edrin Vale: Your denial is heard; tell me what you observed while keeping watch, and whether your companions can confirm it.
- frame judge after line 34: 30% changed, 30% since the frame (gate 25); Mayor Edrin Vale expression: listens with a firm, questioning look; Kael expression: speaks with a steady, earnest expression; Kael pose: addresses the mayor from his bound kneeling position; Mara expression: watches the accused attentively; Sella expression: listens toward the accused; Jory expression: listens toward the accused; Bram expression: listens toward the accused
- [35] Kael: Factor, what evidence besides the strip puts me at the watchman’s side when he was hurt?
- frame judge after line 35: 30% changed, 30% since the frame (gate 25); Mayor Edrin Vale expression: listens with a firm, attentive look; Kael expression: looks toward the market factor with a challenging, questioning expression; Osric Pell expression: faces the accused, attentive to the question
- [36] Osric Pell: The strip and the dagger put you at the smith’s stall; the watchman’s wound fits the charge, and proper judgment can weigh the account.
- plate arrest-judgement-c-5: arrest-judgement-c-5.png is behind the scene (16 regions found, 7 of 11 characters named), with the cast of arrest-hall-real.png
- frame 5 (arrest-judgement-c-5) shown 135.8 s after it started at line 25
- frame judge after line 36: 40% changed, 40% since the frame (gate 25); Mayor Edrin Vale expression: sternly listens, then questions the accused; Kael pose: gives his account while kneeling bound; Kael expression: steadfastly denies stealing the dagger or wounding the watchman; Osric Pell expression: guarded and matter-of-fact as he cites the strip, dagger, and wound; Osric Pell pose: addresses the mayor from the left, presenting his evidence; Bram expression: listens skeptically as he asks the mayor to hear the accused
- frame 7 (arrest-judgement-c-7) started after line 36, shot wide, editing arrest-hall-real.png
- [37] Mara: Mayor, will you let this hall consider whether a charge resting on an unverified blade and a blocked sightline can safely stand?
- frame judge after line 37: 8% changed, 8% since the frame (gate 25); Mayor Edrin Vale expression: his questioning look tightens into a guarded, considering frown; Mara pose: she addresses the mayor with one hand raised in appeal
- [38] Mayor Edrin Vale: That is relevant; the hall may consider whether the evidence supports the charge, but no charge stands or falls on assertion alone.
- frame judge after line 38: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: His questioning look settles into a measured, attentive expression.; Mara pose: She addresses the mayor with a formal, questioning gesture.
- [39] Kael: Mayor, may I speak once more before you rule?
- frame judge after line 39: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: The stern questioning look shifts to a measured, attentive expression as he acknowledges the point.; Kael pose: The bound blond bearded man asks to speak again before the ruling.
- [40] Mayor Edrin Vale: You may speak once more; keep your words to what bears on the charge.
- frame judge after line 40: 0% changed, 10% since the frame (gate 25)
- [41] Mayor Edrin Vale: You may give your last word, sir; keep it to what bears on the charge.
- frame judge after line 41: 10% changed, 10% since the frame (gate 25); Mayor Edrin Vale expression: Keeps a questioning, attentive look on the accused.; Kael expression: Maintains a steady, resolute expression as he prepares to speak.
- [42] Kael: The strip proves my cloak tore here. It proves nothing about the dagger or the watchman. I ask you to judge only what the evidence shows.
- frame 6 (arrest-judgement-c-6) shown 137.8 s after it started at line 30
- frame judge after line 42: 20% changed, 20% since the frame (gate 25); Mayor Edrin Vale expression: listens with a measured, attentive look; Kael pose: speaks his final words steadily from his bound kneeling position; Kael expression: resolute, stressing the limits of what the strip proves; Mara pose: waits at the rail as the accused makes his final statement
- [43] Mayor Edrin Vale: The evidence does not establish that you stole the dagger or wounded the watchman; the charge is dismissed, you are to be released, the factor’s accusation is entered as unproven, and the guard’s account remains under inquiry.
- frame judge after line 43: 40% changed, 40% since the frame (gate 25); Mayor Edrin Vale expression: his questioning look settles into a firm, decisive expression; Mayor Edrin Vale pose: sits upright after issuing the ruling; Kael pose: is no longer kneeling, released from his bonds and standing free; Tavin Rusk pose: no longer presses the accused's shoulder; Osric Pell expression: his guarded expression tightens at the accusation being entered as unproven; Sergeant Alwen Dorr expression: watches the guard's account remain under inquiry with a serious look
- frame 8 (arrest-judgement-c-8) started after line 43, shot wide, editing arrest-hall-real.png
- [44] Kael: Then release me, and let the guard answer for his account.
- frame judge after line 44: 0% changed, 0% since the frame (gate 25)
- plate arrest-judgement-c-7: arrest-judgement-c-7.png is behind the scene (16 regions found, 9 of 11 characters named), with the cast of arrest-hall-real.png
- frame 7 (arrest-judgement-c-7) shown 124.8 s after it started at line 36
- plate arrest-judgement-c-8: arrest-judgement-c-8.png is behind the scene (16 regions found, 8 of 11 characters named), with the cast of arrest-judgement-c-6.png
- frame 8 (arrest-judgement-c-8) shown 135.7 s after it started at line 43
