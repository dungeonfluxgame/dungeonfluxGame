# Run 66: the march to the hall

The guards drag Kael up the street; the sergeant answers Kael and his friends; the bought guard gives the false hour; the fruit seller (written from her guidance) says the bell rang late. Found: the runner made the sergeant answer a question put to Kael (line 4).

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

**[01] Kael:** Sergeant, at what hour do they say the watchman was cut and the dagger taken?

**[02] Sergeant_Dorr:** The hour was not given in the report I received; I cannot state it as fact.
- behavior: procedural-explanation
- fired triggers: asked-for-basis
- tone: ally
- feels: trust 71, pride 38, fear 29, irritation 25; mood 21 valence, 50 arousal; toward warrior: indifferent (3); focus warrior on reported timing of the attack
- time: line 7.4s, backfill started
- covered: false (gaps below)
- - backstory: hour the watchman was cut and the dagger taken
- backfill: running (record 1 when done)
- model: codex:gpt-6-luna
- total: 7373 ms

**[03] Tavin:** It was just after the market bell. Prisoner, did you hear it ring as you ran?

**[04] Sergeant_Dorr:** I wasn't running. I was by the stalls, checking my gear; my companions can attest to that.
- behavior: formal-arrest
- tone: npc
- feels: trust 68, pride 38, irritation 28, fear 27; mood 19 valence, 50 arousal; toward warrior: indifferent (3); focus warrior on the bell and his movements
- time: line 6.9s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6947 ms

**[05] Nessa:** Sergeant, the bell rang late today, and I saw the watchman bleeding before they took this man. Why did nobody ask me?

**[06] Sergeant_Dorr:** You should have been asked. Your account will be recorded before the hearing; tell me what you saw.
- behavior: procedural-explanation
- fired emotional triggers: hearing-requested (by state: none)
- tone: ally
- feels: trust 77, pride 38, irritation 26, fear 25, joy 24; mood 27 valence, 49 arousal; toward warrior: indifferent (3); toward market_witness: indifferent (3); focus market_witness on the witness's account
- time: line 7.1s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7177 ms

**[07] Jory:** Will you let the fruit seller speak before the mayor?
- fired triggers: risk-unclear

**[08] Sergeant_Dorr:** Yes. The fruit seller's account will be recorded before the hearing.
- behavior: procedural-explanation
- fired emotional triggers: hearing-requested (by state: none)
- tone: ally
- feels: trust 84, pride 38, joy 35, irritation 24, fear 24; mood 32 valence, 49 arousal; toward warrior: indifferent (3); toward market_witness: indifferent (3); toward jory_adventurer: indifferent (3); focus market_witness on the witness's account
- time: line 6.1s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6123 ms

**[09] Osric:** A market woman’s memory is worth less than a guard’s eyes; Sergeant, will you keep the prisoner moving?

**[10] Sergeant_Dorr:** The prisoner will keep moving, but the witness will be heard before the mayor; her account belongs in the record.
- behavior: procedural-explanation
- behavior override: trigger hearing-requested fired, so the line took procedural-explanation, not formal-arrest
- fired emotional triggers: hearing-requested (by state: none)
- tone: ally
- feels: trust 87, joy 43, pride 42, irritation 30, fear 23; mood 32 valence, 49 arousal; toward warrior: indifferent (3); toward market_witness: indifferent (3); toward jory_adventurer: indifferent (3); toward market_factor: indifferent (-2); focus market_factor on whether to keep the prisoner moving
- time: line 6.2s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6276 ms

**[11] Sella:** Sergeant, who found the dagger and cloth? Did you see either taken from him yourself?
- fired triggers: threat-nearby

**[12] Sergeant_Dorr:** The dagger and cloth were found by the watch; I did not see either taken from him.
- behavior: procedural-explanation
- fired triggers: asked-for-basis
- revealed secrets: did-not-witness-attack
- tone: ally
- feels: trust 87, pride 41, joy 34, irritation 28, fear 22; mood 31 valence, 49 arousal; toward warrior: indifferent (3); toward market_witness: indifferent (3); toward jory_adventurer: indifferent (3); toward market_factor: indifferent (-2); toward sella_adventurer: indifferent (2); focus sella_adventurer on who found the evidence and whether she witnessed the theft
- time: line 6.7s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6672 ms

**[13] Kael:** Sergeant, do you believe the man holding me?
- fired triggers: danger-nearby

**[14] Sergeant_Dorr:** I have not judged his account; it is reported, not something I witnessed. You will have a fair hearing before the mayor.
- behavior: procedural-explanation
- tone: ally
- feels: trust 87, pride 40, joy 27, irritation 26, fear 21; mood 31 valence, 48 arousal; toward warrior: indifferent (5); toward market_witness: indifferent (3); toward jory_adventurer: indifferent (3); toward market_factor: indifferent (-2); toward sella_adventurer: indifferent (2); focus warrior on whether the accuser is credible
- time: line 6.1s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6163 ms

**[15] Mara:** Sergeant, has the guard holding Kael given the same account twice about where he found the dagger?

**[16] Sergeant_Dorr:** I cannot confirm whether he repeated it; I have not heard his account. The finding was reported to me, not witnessed by me.
- behavior: procedural-explanation
- fired triggers: asked-for-basis
- tone: ally
- feels: trust 88, pride 43, irritation 24, joy 21; mood 31 valence, 47 arousal; toward warrior: indifferent (5); toward market_witness: indifferent (3); toward jory_adventurer: indifferent (3); toward market_factor: indifferent (-2); toward sella_adventurer: indifferent (2); toward mara_adventurer: indifferent (2); focus mara_adventurer on the guard’s account
- time: line 6.9s, backfill started
- covered: false (gaps below)
- - backstory: whether the guard gave the same account twice about where he found the dagger
- backfill: running (record 1 when done)
- model: codex:gpt-6-luna
- total: 6967 ms
- --- frames (artifacts/tmp/arrest/scene2-frames.log) ---
- [01] Kael: Sergeant, at what hour do they say the watchman was cut and the dagger taken?
- frame judge after line 1: 10% changed, 10% since the frame (gate 25); Kael expression: Kael turns a questioning look toward the sergeant as he asks about the hour of the attack.
- [02] Sergeant Alwen Dorr: The hour was not given in the report I received; I cannot state it as fact.
- frame judge after line 2: 10% changed, 10% since the frame (gate 25); Kael expression: Kael looks questioning as he asks the sergeant about the hour.; Sergeant Alwen Dorr expression: The sergeant answers with a measured, matter-of-fact expression.
- [03] Tavin Rusk: It was just after the market bell. Prisoner, did you hear it ring as you ran?
- frame judge after line 3: 20% changed, 20% since the frame (gate 25); Kael expression: Kael watches the sergeant, asking a pointed question.; Sergeant Alwen Dorr expression: The sergeant answers evenly, unable to supply the hour.; Tavin Rusk expression: Tavin turns his attention to the bound prisoner as he questions him.
- [04] Sergeant Alwen Dorr: I wasn't running. I was by the stalls, checking my gear; my companions can attest to that.
- frame judge after line 4: 25% changed, 25% since the frame (gate 25); Kael expression: turns his attention to Sergeant Dorr, questioning; Sergeant Alwen Dorr expression: answers steadily, then looks toward Tavin; Tavin Rusk expression: fixes a challenging look on the bound prisoner; Sergeant Alwen Dorr pose: gestures toward herself as she says she was checking her gear by the stalls
- [05] Nessa Vale: Sergeant, the bell rang late today, and I saw the watchman bleeding before they took this man. Why did nobody ask me?
- frame judge after line 5: 25% changed, 25% since the frame (gate 25); Tavin Rusk expression: Tavin fixes the bound prisoner with a questioning look.; Sergeant Alwen Dorr expression: The sergeant looks guarded as he denies running.; Nessa Vale pose: Nessa calls out from the apple barrow, challenging the guards.; Nessa Vale expression: Nessa looks indignant, demanding to know why nobody asked her.
- [06] Sergeant Alwen Dorr: You should have been asked. Your account will be recorded before the hearing; tell me what you saw.
- frame judge after line 6: 30% changed, 30% since the frame (gate 25); Tavin Rusk expression: Questions the bound prisoner with a challenging, narrowed look.; Sergeant Alwen Dorr expression: Turns toward Nessa with an attentive, apologetic expression.; Sergeant Alwen Dorr pose: Invites Nessa to give her account before the hearing.; Nessa Vale expression: Looks insistent as she addresses the sergeant.
- frame 1 (arrest-march-a-1) started after line 6, shot two-shot, editing arrest-march.png
- [07] Jory: Will you let the fruit seller speak before the mayor?
- frame judge after line 7: 10% changed, 10% since the frame (gate 25); Jory expression: Jory speaks up, intent on the sergeant.
- [08] Sergeant Alwen Dorr: Yes. The fruit seller's account will be recorded before the hearing.
- frame judge after line 8: 10% changed, 10% since the frame (gate 25); Jory expression: Jory looks earnest as he asks for the fruit seller to speak first.; Sergeant Alwen Dorr expression: The sergeant answers with a calm, affirming expression.
- [09] Osric Pell: A market woman’s memory is worth less than a guard’s eyes; Sergeant, will you keep the prisoner moving?
- frame judge after line 9: 10% changed, 10% since the frame (gate 25); Sergeant Alwen Dorr expression: Her apologetic look firms into a clear, resolute expression as she answers.; Osric Pell expression: He wears a dismissive, challenging look as he questions the sergeant.
- [10] Sergeant Alwen Dorr: The prisoner will keep moving, but the witness will be heard before the mayor; her account belongs in the record.
- frame judge after line 10: 20% changed, 20% since the frame (gate 25); Sergeant Alwen Dorr expression: Her attentive, apologetic look firms into a decisive expression as she answers.; Osric Pell expression: He speaks dismissively of the witness, his expression condescending.; Nessa Vale expression: She looks ready to give her account, her expression insistent.
- [11] Sella: Sergeant, who found the dagger and cloth? Did you see either taken from him yourself?
- frame judge after line 11: 20% changed, 20% since the frame (gate 25); Sergeant Alwen Dorr expression: Turns toward Sella with a firm, attentive look.; Sella pose: Addresses the sergeant, asking who found the dagger and cloth and whether he saw them taken.
- [12] Sergeant Alwen Dorr: The dagger and cloth were found by the watch; I did not see either taken from him.
- frame judge after line 12: 25% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: Keeps an attentive, firm look as she answers Sella.; Sella pose: Turns toward the sergeant and questions her about the dagger and cloth.; Osric Pell expression: Looks dismissive as he disparages the witness.
- [13] Kael: Sergeant, do you believe the man holding me?
- frame judge after line 13: 25% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: Keeps an attentive, firm look as he answers.; Osric Pell expression: Looks dismissive, with a challenging, narrowed gaze.; Sella expression: Questions the sergeant with a skeptical, narrowed look.; Kael expression: Looks toward the sergeant, asking whether he believes the guard holding him.
- [14] Sergeant Alwen Dorr: I have not judged his account; it is reported, not something I witnessed. You will have a fair hearing before the mayor.
- frame judge after line 14: 25% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: Keeps a measured, impartial expression as she answers.; Osric Pell expression: Wears a dismissive, challenging look toward the witness and sergeant.; Nessa Vale expression: Looks attentive and ready to give her account.; Kael expression: Looks toward the sergeant with a questioning, wary expression.
- [15] Mara: Sergeant, has the guard holding Kael given the same account twice about where he found the dagger?
- frame judge after line 15: 10% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: Her attentive, apologetic expression turns questioning as she asks whether the guard gave the same account twice.
- [16] Sergeant Alwen Dorr: I cannot confirm whether he repeated it; I have not heard his account. The finding was reported to me, not witnessed by me.
- frame judge after line 16: 25% changed, 25% since the frame (gate 25); Sergeant Alwen Dorr expression: The sergeant looks measured and impartial as she explains what she did not witness.; Kael expression: Kael looks tense and searching as he asks whether the sergeant believes him.; Osric Pell expression: Osric looks dismissive as he questions the fruit seller’s credibility.; Nessa Vale expression: The fruit seller looks resolute, ready to have her account heard.; Tavin Rusk pose: The guard continues gripping Kael’s arm and moving him uphill.; Sergeant Alwen Dorr pose: The sergeant turns toward Kael and the accuser while keeping the hearing in view.
- frame 1 (arrest-march-a-1) shown 112.5 s after it started at line 6
