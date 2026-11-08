# Run 59: varied openers, inserts only for inspections

The judge no longer offered the insert shot; Haldor's openers varied (no Aye or Certainly). Found: two Verdant inserts in a row (a quiet line reset the last item looked at), and Haldor said 'Mara' unprompted: the scene's words were still the fruit-stall fixture's ('Mara Vell at her fruit stall'). See persona-refinement-log.md, refinement 22.

- ! ok state weapon_seller turn=0 mood valence=40 arousal=51
- !   emotions: joy=42 trust=48 irritation=18 fear=12 pride=52
- !   knows the names of: nobody
- ! ok state warrior turn=0 mood valence=15 arousal=54
- !   emotions: joy=25 trust=20 irritation=20 fear=20 sadness=15 surprise=10 pride=35 amusement=15
- !   knows the names of: nobody
- ! ok state mara_adventurer turn=0 mood valence=35 arousal=44
- !   emotions: trust=44 irritation=16 fear=10 pride=31 affection=24
- !   knows the names of: nobody
- !   triggers holding: fear-eases
- ! ok state jory_adventurer turn=0 mood valence=45 arousal=49
- !   emotions: joy=42 trust=45 irritation=20 fear=18 pride=32 amusement=34 affection=30
- !   knows the names of: nobody
- ! ok state sella_adventurer turn=0 mood valence=44 arousal=47
- !   emotions: joy=42 trust=48 irritation=20 fear=16 pride=38 affection=40
- !   knows the names of: nobody
- ! ok state bram_adventurer turn=0 mood valence=31 arousal=42
- !   emotions: trust=42 irritation=16 fear=12 sadness=18 pride=38 amusement=28 affection=32
- !   knows the names of: nobody
- ! ok state mara_adventurer turn=0 mood valence=35 arousal=44
- !   emotions: trust=44 irritation=16 fear=10 pride=31 affection=24
- !   knows the names of: warrior
- !   triggers holding: fear-eases
- ! ok state mara_adventurer turn=0 mood valence=35 arousal=44
- !   emotions: trust=44 irritation=16 fear=10 pride=31 affection=24
- !   knows the names of: warrior jory_adventurer
- !   triggers holding: fear-eases
- ! ok state mara_adventurer turn=0 mood valence=35 arousal=44
- !   emotions: trust=44 irritation=16 fear=10 pride=31 affection=24
- !   knows the names of: warrior jory_adventurer sella_adventurer
- !   triggers holding: fear-eases
- ! ok state mara_adventurer turn=0 mood valence=35 arousal=44
- !   emotions: trust=44 irritation=16 fear=10 pride=31 affection=24
- !   knows the names of: warrior jory_adventurer sella_adventurer bram_adventurer
- !   triggers holding: fear-eases
- ! ok state jory_adventurer turn=0 mood valence=45 arousal=49
- !   emotions: joy=42 trust=45 irritation=20 fear=18 pride=32 amusement=34 affection=30
- !   knows the names of: warrior
- ! ok state jory_adventurer turn=0 mood valence=45 arousal=49
- !   emotions: joy=42 trust=45 irritation=20 fear=18 pride=32 amusement=34 affection=30
- !   knows the names of: warrior mara_adventurer
- ! ok state jory_adventurer turn=0 mood valence=45 arousal=49
- !   emotions: joy=42 trust=45 irritation=20 fear=18 pride=32 amusement=34 affection=30
- !   knows the names of: warrior mara_adventurer sella_adventurer
- ! ok state jory_adventurer turn=0 mood valence=45 arousal=49
- !   emotions: joy=42 trust=45 irritation=20 fear=18 pride=32 amusement=34 affection=30
- !   knows the names of: warrior mara_adventurer sella_adventurer bram_adventurer
- ! ok state sella_adventurer turn=0 mood valence=44 arousal=47
- !   emotions: joy=42 trust=48 irritation=20 fear=16 pride=38 affection=40
- !   knows the names of: warrior
- ! ok state sella_adventurer turn=0 mood valence=44 arousal=47
- !   emotions: joy=42 trust=48 irritation=20 fear=16 pride=38 affection=40
- !   knows the names of: warrior mara_adventurer
- ! ok state sella_adventurer turn=0 mood valence=44 arousal=47
- !   emotions: joy=42 trust=48 irritation=20 fear=16 pride=38 affection=40
- !   knows the names of: warrior mara_adventurer jory_adventurer
- ! ok state sella_adventurer turn=0 mood valence=44 arousal=47
- !   emotions: joy=42 trust=48 irritation=20 fear=16 pride=38 affection=40
- !   knows the names of: warrior mara_adventurer jory_adventurer bram_adventurer
- ! ok state bram_adventurer turn=0 mood valence=31 arousal=42
- !   emotions: trust=42 irritation=16 fear=12 sadness=18 pride=38 amusement=28 affection=32
- !   knows the names of: warrior
- ! ok state bram_adventurer turn=0 mood valence=31 arousal=42
- !   emotions: trust=42 irritation=16 fear=12 sadness=18 pride=38 amusement=28 affection=32
- !   knows the names of: warrior mara_adventurer
- ! ok state bram_adventurer turn=0 mood valence=31 arousal=42
- !   emotions: trust=42 irritation=16 fear=12 sadness=18 pride=38 amusement=28 affection=32
- !   knows the names of: warrior mara_adventurer jory_adventurer
- ! ok state bram_adventurer turn=0 mood valence=31 arousal=42
- !   emotions: trust=42 irritation=16 fear=12 sadness=18 pride=38 amusement=28 affection=32
- !   knows the names of: warrior mara_adventurer jory_adventurer sella_adventurer
- ! ok state warrior turn=0 mood valence=15 arousal=54
- !   emotions: joy=25 trust=20 irritation=20 fear=20 sadness=15 surprise=10 pride=35 amusement=15
- !   knows the names of: mara_adventurer
- ! ok state warrior turn=0 mood valence=15 arousal=54
- !   emotions: joy=25 trust=20 irritation=20 fear=20 sadness=15 surprise=10 pride=35 amusement=15
- !   knows the names of: mara_adventurer jory_adventurer
- ! ok state warrior turn=0 mood valence=15 arousal=54
- !   emotions: joy=25 trust=20 irritation=20 fear=20 sadness=15 surprise=10 pride=35 amusement=15
- !   knows the names of: mara_adventurer jory_adventurer sella_adventurer
- ! ok state warrior turn=0 mood valence=15 arousal=54
- !   emotions: joy=25 trust=20 irritation=20 fear=20 sadness=15 surprise=10 pride=35 amusement=15
- !   knows the names of: mara_adventurer jory_adventurer sella_adventurer bram_adventurer

**[01] Mara:** How did you form the fuller and temper the blade, and were the frost runes cut before or after the quench?

**[02] Haldor:** The fuller lightens and stiffens the blade; I temper it long after the clay-backed quench. Frost runes are cut afterward, before the rune-scribe wakes them.
- behavior: careful-appraisal
- behavior override: trigger appraisal-question fired, so the line took careful-appraisal, not shop-talk
- fired triggers: knowing-question, appraisal-question
- fired emotional triggers: questioned-claims (by state: none)
- learned the name of: mara_adventurer
- tone: npc
- feels: pride 66, trust 51, joy 42; mood 44 valence, 50 arousal; toward mara_adventurer: indifferent (2); focus mara_adventurer on Frostline forging
- time: line 8.2s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 8213 ms

**[03] Jory:** So, do those glowing blades run hot enough to cook supper on one?

**[04] Haldor:** You’d spoil a fine blade and still have a cold supper—the runes glow, but they’re cool to the touch.
- behavior: patient-tease
- fired triggers: foolish-question
- tone: npc
- feels: pride 63, trust 50, joy 42; mood 46 valence, 50 arousal; toward mara_adventurer: indifferent (2); toward jory_adventurer: indifferent (1); focus jory_adventurer on runes and heat
- time: line 6.9s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6913 ms

**[05] Sella:** Those runes are true enchantments, or painted glow meant to fleece travellers?

**[06] Haldor:** Painted? I forged the steel myself; the runes are cut after the temper, then woken by a scribe. Feel the Frostline’s chill—glow, not heat.
- behavior: careful-appraisal
- behavior override: trigger appraisal-question fired, so the line took careful-appraisal, not stung-pride
- fired triggers: doubted-craft, appraisal-question
- fired emotional triggers: questioned-claims (by state: none)
- tone: npc
- feels: pride 77, trust 49, joy 42; mood 51 valence, 49 arousal; toward mara_adventurer: indifferent (2); toward jory_adventurer: indifferent (1); focus sella_adventurer on rune authenticity
- time: line 7.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7870 ms

**[07] Bram:** What are you asking for the Frostline longsword, Emberfang scimitar, Verdant bastard sword, and Nightglass dagger?
- fired triggers: welcome-settles

**[08] Haldor:** Frostline, 450 gold; Emberfang, 380 gold; Verdant, 520 gold; Nightglass, 260 gold. Fine steel, each one.
- quoted: 45000 cp for 1 frostline_longsword (inside floor 40500, ask 45000)
- quoted: 38000 cp for 1 emberfang_scimitar (inside floor 34200, ask 38000)
- quoted: 52000 cp for 1 verdant_bastard_sword (inside floor 46800, ask 52000)
- quoted: 26000 cp for 1 nightglass_dagger (inside floor 23400, ask 26000)
- behavior: closing-the-sale
- fired triggers: buyer-interest
- fired emotional triggers: earned-confidence (by state: none)
- tone: npc
- feels: pride 78, trust 61, joy 57; mood 51 valence, 49 arousal; toward mara_adventurer: indifferent (2); toward jory_adventurer: indifferent (1); toward bram_adventurer: indifferent (5); focus bram_adventurer on blade prices
- time: line 8.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 8824 ms

**[09] Mara:** Which of these blades will hold its edge best on a long road?

**[10] Haldor:** The Verdant holds its edge longest on a long road; it’s the toughest of the four, though the Frostline takes a keener edge and wants honing.
- behavior: careful-appraisal
- fired triggers: appraisal-question
- tone: ally
- feels: pride 80, trust 63, joy 53; mood 49 valence, 49 arousal; toward mara_adventurer: indifferent (4); toward jory_adventurer: indifferent (1); toward bram_adventurer: indifferent (5); focus mara_adventurer on edge retention on the road
- time: line 7.2s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7201 ms

**[11] Mara:** Could you hold the Verdant up to the light, so I can study the vine etching and gold-inlaid guard up close?

**[12] Haldor:** The Verdant’s up close, Mara—see how the vine etching runs in silver-green beneath the clean finish, and how the gold guard branches firm around the grip.
- behavior: show-the-blade
- fired triggers: close-look
- fired emotional triggers: earned-confidence (by state: none)
- learned the name of: mara_adventurer
- tone: ally
- feels: pride 81, trust 73, joy 63; mood 50 valence, 48 arousal; toward mara_adventurer: indifferent (7); toward jory_adventurer: indifferent (1); toward bram_adventurer: indifferent (5); focus mara_adventurer on Verdant's etching and guard
- time: line 7.7s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7744 ms

**[13] Jory:** Is that little black dagger meant for children, then?

**[14] Haldor:** Not a child’s toy, no; it’s balanced for close, quiet work, and best left to a steady hand.
- behavior: patient-tease
- fired triggers: foolish-question
- tone: npc
- feels: pride 81, trust 70, joy 57; mood 48 valence, 49 arousal; toward mara_adventurer: indifferent (7); toward jory_adventurer: indifferent (3); toward bram_adventurer: indifferent (5); focus jory_adventurer on Nightglass dagger’s intended use
- time: line 7.5s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7551 ms

**[15] Bram:** That scimitar’s far too dear for what it is. Would you take a good deal less?

**[16] Haldor:** The Emberfang’s sound steel, but I’ll come down a little: 361 gold.
- quoted: 36100 cp for 1 emberfang_scimitar (inside floor 34200, ask 38000)
- behavior: closing-the-sale
- fired triggers: buyer-interest
- tone: npc
- feels: pride 82, trust 67, joy 53; mood 45 valence, 49 arousal; toward mara_adventurer: indifferent (7); toward jory_adventurer: indifferent (3); toward bram_adventurer: indifferent (3); focus jory_adventurer on Bram’s offer to haggle over the scimitar
- time: line 8.2s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 8197 ms

**[17] Sella:** Could you show us here that the Frostline’s runes truly work?

**[18] Haldor:** The runes are cut after tempering and stay cool to the touch; feel the chill from the flat, or I can frost a cup here.
- behavior: careful-appraisal
- fired triggers: appraisal-question, close-look
- fired emotional triggers: questioned-claims (by state: none)
- tone: ally
- feels: pride 86, trust 69, joy 55; mood 47 valence, 49 arousal; toward mara_adventurer: indifferent (7); toward jory_adventurer: indifferent (3); toward bram_adventurer: indifferent (3); toward sella_adventurer: indifferent (3); focus sella_adventurer on Demonstrating the Frostline runes
- time: line 7.0s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7010 ms

**[19] Sella:** May I hold the Frostline and look closely at the runes along the fuller, the guard, and the grip?

**[20] Haldor:** Take it by the grip; the fuller lightens the blade and stiffens it, while the guard is plain and firm. The frost runes are cut after tempering, and stay cool to the touch.
- behavior: show-the-blade
- fired triggers: close-look
- fired emotional triggers: earned-confidence (by state: none)
- tone: ally
- feels: pride 79, trust 77, joy 63; mood 48 valence, 48 arousal; toward mara_adventurer: indifferent (7); toward jory_adventurer: indifferent (3); toward bram_adventurer: indifferent (3); toward sella_adventurer: indifferent (6); focus sella_adventurer on Examining the Frostline
- time: line 6.6s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6607 ms

**[21] Kael:** Which of the four would you put in a guard’s hand for fighting in narrow streets?

**[22] Haldor:** For a guard in narrow streets, I’d put the Nightglass in hand: it’s balanced for close work, not a child’s toy.
- behavior: careful-appraisal
- fired triggers: appraisal-question
- fired emotional triggers: questioned-claims (by state: none)
- tone: npc
- feels: pride 84, trust 78, joy 57; mood 50 valence, 48 arousal; toward mara_adventurer: indifferent (7); toward jory_adventurer: indifferent (3); toward bram_adventurer: indifferent (3); toward sella_adventurer: indifferent (6); toward warrior: indifferent (2); focus warrior on Choosing a blade for narrow streets
- time: line 6.9s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6898 ms

**[23] Kael:** Could you show me the Nightglass up close—the violet sigils, the edge, and the bone handle?

**[24] Haldor:** The Nightglass’s violet sigils dim in shadow; look along the clean edge and close-fitted bone handle, balanced for close work.
- behavior: show-the-blade
- fired triggers: close-look
- tone: ally
- feels: pride 85, trust 79, joy 60; mood 49 valence, 48 arousal; toward mara_adventurer: indifferent (7); toward jory_adventurer: indifferent (3); toward bram_adventurer: indifferent (3); toward sella_adventurer: indifferent (6); toward warrior: indifferent (4); focus warrior on Nightglass details
- time: line 8.3s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 8289 ms

**[25] Bram:** What would you take for the Emberfang and the Nightglass together?

**[26] Haldor:** For the Emberfang and Nightglass together, I’d take 608 gold; that’s 361 gold for the scimitar and 247 gold for the dagger.
- quoted: 36100 cp for 1 emberfang_scimitar (inside floor 34200, ask 38000)
- quoted: 24700 cp for 1 nightglass_dagger (inside floor 23400, ask 26000)
- behavior: closing-the-sale
- fired triggers: buyer-interest
- tone: ally
- feels: pride 79, trust 75, joy 65; mood 49 valence, 49 arousal; toward mara_adventurer: indifferent (7); toward jory_adventurer: indifferent (3); toward bram_adventurer: indifferent (8); toward sella_adventurer: indifferent (6); toward warrior: indifferent (4); focus bram_adventurer on combined blade price
- time: line 8.0s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7990 ms

**[27] Kael:** I’ll take the Frostline if you give me a fair price today. What’s your best price?

**[28] Haldor:** For you, the Frostline’s 427 gold and 5 silver today; its keen edge suits a guard, if you keep it honed on the road.
- quoted: 42750 cp for 1 frostline_longsword (inside floor 40500, ask 45000)
- behavior: closing-the-sale
- fired triggers: buyer-interest
- tone: ally
- feels: pride 74, trust 71, joy 70; mood 49 valence, 49 arousal; toward mara_adventurer: indifferent (7); toward jory_adventurer: indifferent (3); toward bram_adventurer: indifferent (8); toward sella_adventurer: indifferent (6); toward warrior: indifferent (9); focus warrior on Frostline price
- time: line 7.3s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7289 ms

**[29] Mara:** How do I keep the Frostline’s runes sound on the road?

**[30] Haldor:** Keep the runes clean and dry, and don’t grind across them; if one dulls, a scribe must reset it, not a whetstone.
- behavior: careful-appraisal
- fired triggers: appraisal-question
- tone: npc
- feels: pride 75, trust 73, joy 63; mood 48 valence, 49 arousal; toward mara_adventurer: indifferent (9); toward jory_adventurer: indifferent (3); toward bram_adventurer: indifferent (8); toward sella_adventurer: indifferent (6); toward warrior: indifferent (9); focus mara_adventurer on Frostline rune care
- time: line 7.5s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7523 ms
- --- frames (artifacts/tmp/market/run59-frames.log) ---
- [01] Mara: How did you form the fuller and temper the blade, and were the frost runes cut before or after the quench?
- frame judge after line 1: 10% changed, 10% since the frame (gate 25); Mara expression: Mara studies the blades with a focused, inquisitive look.; Mara pose: Mara leans toward Haldor and gestures toward the blades as she asks about the fuller, tempering, and runes.
- [02] Haldor Brask: The fuller lightens and stiffens the blade; I temper it long after the clay-backed quench. Frost runes are cut afterward, before the rune-scribe wakes them.
- frame judge after line 2: 15% changed, 15% since the frame (gate 25); Mara pose: leans toward the blue-runed longsword, asking about its fuller and temper; Haldor Brask pose: gestures toward the blade as he explains its forging; the room item: the blue-runed longsword is the focus of the exchange
- [03] Jory: So, do those glowing blades run hot enough to cook supper on one?
- frame judge after line 3: 25% changed, 25% since the frame (gate 25); Haldor Brask pose: Explains the blade’s fuller and tempering with a hand indicating the swords.; Mara expression: Studies Haldor as she asks about the blade and runes.; Jory expression: Grins as he jokes about cooking supper on the glowing blades.; Mara position: Leans toward the counter and Haldor while asking her question.
- [04] Haldor Brask: You’d spoil a fine blade and still have a cold supper—the runes glow, but they’re cool to the touch.
- frame judge after line 4: 10% changed, 25% since the frame (gate 25); Mara expression: Mara looks inquisitive as she asks about the blade’s fuller, tempering, and runes.; Haldor Brask expression: Haldor answers with an assured, explanatory expression.; Jory expression: Jory grins as he jokes about cooking supper on a glowing blade.; Haldor Brask expression: Haldor responds with a dry, amused look.
- [05] Sella: Those runes are true enchantments, or painted glow meant to fleece travellers?
- frame judge after line 5: 15% changed, 25% since the frame (gate 25); Haldor Brask pose: He gestures toward the blades as he explains the fuller, quench, tempering, and runes.; Jory expression: He grins after joking about cooking supper on a glowing blade.; Haldor Brask expression: He gives Jory a dry, amused look while answering.; Sella expression: She narrows her eyes skeptically at the seller and the runes.
- [06] Haldor Brask: Painted? I forged the steel myself; the runes are cut after the temper, then woken by a scribe. Feel the Frostline’s chill—glow, not heat.
- frame judge after line 6: 30% changed, 30% since the frame (gate 25); Haldor Brask pose: Explains the blade’s fuller and tempering, then gestures toward the Frostline.; Jory expression: Grins at his supper joke.; Haldor Brask expression: Smiles at the joke, then looks firm as he defends his work.; Sella expression: Looks doubtful, questioning whether the runes are genuine.; Haldor Brask pose: Holds the Frostline out for Sella to feel its chill.; Sella pose: Reaches to feel the Frostline’s blade.
- frame 1 (gauntlet-h-1) started after line 6, shot insert on Frostline longsword, editing weapons-stall-blades.png
- [07] Bram: What are you asking for the Frostline longsword, Emberfang scimitar, Verdant bastard sword, and Nightglass dagger?
- frame judge after line 7: 0% changed, 0% since the frame (gate 25)
- [08] Haldor Brask: Frostline, 450 gold; Emberfang, 380 gold; Verdant, 520 gold; Nightglass, 260 gold. Fine steel, each one.
- frame judge after line 8: 0% changed, 0% since the frame (gate 25)
- [09] Mara: Which of these blades will hold its edge best on a long road?
- frame judge after line 9: 0% changed, 0% since the frame (gate 25)
- [10] Haldor Brask: The Verdant holds its edge longest on a long road; it’s the toughest of the four, though the Frostline takes a keener edge and wants honing.
- frame judge after line 10: 20% changed, 20% since the frame (gate 25); Haldor Brask pose: He gestures toward the Verdant bastard sword as he explains its edge lasts longest.; Mara expression: She studies the blades with a focused, considering look.
- [11] Mara: Could you hold the Verdant up to the light, so I can study the vine etching and gold-inlaid guard up close?
- insert gauntlet-h-1: gauntlet-h-1.png is behind the scene, with the cast of weapons-stall-blades.png; weapon_seller already knew the Frostline longsword up close, nothing backfilled
- frame 1 (gauntlet-h-1) shown 56.7 s after it started at line 6
- frame judge after line 11: 30% changed, 30% since the frame (gate 25); Mara pose: leans toward the Verdant bastard sword, studying it up close; Haldor Brask pose: lifts the Verdant bastard sword toward the light to show its etching and guard; the room light: light catches the sword's vine etching and gold-inlaid guard
- frame 2 (gauntlet-h-2) started after line 11, shot insert on Verdant bastard sword, editing weapons-stall-blades.png
- [12] Haldor Brask: The Verdant’s up close, Mara—see how the vine etching runs in silver-green beneath the clean finish, and how the gold guard branches firm around the grip.
- frame judge after line 12: 30% changed, 30% since the frame (gate 25); Haldor Brask pose: Leans in, indicating the Verdant’s blade and guard.; Mara pose: Looks closely at the Verdant bastard sword.; the room item: The green-runed bastard sword is presented up close, its silver-green vine etching and gold-inlaid branching guard visible.
- [13] Jory: Is that little black dagger meant for children, then?
- back to weapons-stall-blades.png after the insert (27.3 s up)
- frame judge after line 13: 30% changed, 30% since the frame (gate 25); Haldor Brask pose: Leans in and indicates the Verdant’s blade and branching gold guard.; Jory expression: Looks skeptically at the small black dagger.
- [14] Haldor Brask: Not a child’s toy, no; it’s balanced for close, quiet work, and best left to a steady hand.
- frame judge after line 14: 30% changed, 30% since the frame (gate 25); Haldor Brask item: Haldor lifts the Verdant bastard sword for a close look, showing its silver-green vine etching and branching gold-inlaid guard.; Jory expression: Jory looks sheepish after his joke about the dagger; the others grin.; Haldor Brask expression: Haldor answers Jory calmly, holding the little black dagger as he explains its balance and purpose.
- frame 3 (gauntlet-h-3) started after line 14, shot insert on Verdant bastard sword, editing weapons-stall-blades.png
- [15] Bram: That scimitar’s far too dear for what it is. Would you take a good deal less?
- [16] Haldor Brask: The Emberfang’s sound steel, but I’ll come down a little: 361 gold.
- [17] Sella: Could you show us here that the Frostline’s runes truly work?
- [18] Haldor Brask: The runes are cut after tempering and stay cool to the touch; feel the chill from the flat, or I can frost a cup here.
- insert gauntlet-h-2: gauntlet-h-2.png is behind the scene, with the cast of weapons-stall-blades.png; 1 detail(s) of the Verdant bastard sword backfilled into weapon_seller
- frame 2 (gauntlet-h-2) shown 75.6 s after it started at line 11
- frame judge after line 18: 30% changed, 30% since the frame (gate 25); Bram pose: Bram leans toward the Emberfang scimitar, challenging its price.; Haldor Brask expression: Haldor answers the haggle with a composed look.; Haldor Brask item: Haldor indicates the Frostline longsword and offers its cool flat for Sella to feel.
- frame 4 (gauntlet-h-4) started after line 18, shot wide, editing weapons-stall-blades.png
- [19] Sella: May I hold the Frostline and look closely at the runes along the fuller, the guard, and the grip?
- [20] Haldor Brask: Take it by the grip; the fuller lightens the blade and stiffens it, while the guard is plain and firm. The frost runes are cut after tempering, and stay cool to the touch.
- insert gauntlet-h-3: gauntlet-h-3.png is behind the scene, with the cast of weapons-stall-blades.png; weapon_seller already knew the Verdant bastard sword up close, nothing backfilled
- frame 3 (gauntlet-h-3) shown 59.1 s after it started at line 14
- frame judge after line 20: 40% changed, 40% since the frame (gate 25); Sella pose: Takes the Frostline by its grip and holds it close to study the runes, guard, and grip.; Haldor Brask pose: Offers the Frostline to Sella by the grip, explaining its fuller and cool frost runes.; the room item: The Frostline longsword is lifted from the red velvet for Sella to inspect.
- frame 5 (gauntlet-h-5) started after line 20, shot insert on Frostline longsword, editing weapons-stall-blades.png
- [21] Kael: Which of the four would you put in a guard’s hand for fighting in narrow streets?
- back to weapons-stall-blades.png after the insert (20.0 s up)
- [22] Haldor Brask: For a guard in narrow streets, I’d put the Nightglass in hand: it’s balanced for close work, not a child’s toy.
- [23] Kael: Could you show me the Nightglass up close—the violet sigils, the edge, and the bone handle?
- [24] Haldor Brask: The Nightglass’s violet sigils dim in shadow; look along the clean edge and close-fitted bone handle, balanced for close work.
- [25] Bram: What would you take for the Emberfang and the Nightglass together?
- [26] Haldor Brask: For the Emberfang and Nightglass together, I’d take 608 gold; that’s 361 gold for the scimitar and 247 gold for the dagger.
- [27] Kael: I’ll take the Frostline if you give me a fair price today. What’s your best price?
- [28] Haldor Brask: For you, the Frostline’s 427 gold and 5 silver today; its keen edge suits a guard, if you keep it honed on the road.
- [29] Mara: How do I keep the Frostline’s runes sound on the road?
- [30] Haldor Brask: Keep the runes clean and dry, and don’t grind across them; if one dulls, a scribe must reset it, not a whetstone.
- frame 4 (gauntlet-h-4) shown 126.5 s after it started at line 18
- insert gauntlet-h-5: gauntlet-h-5.png is behind the scene, with the cast of weapons-stall-blades.png; weapon_seller already knew the Frostline longsword up close, nothing backfilled
- frame 5 (gauntlet-h-5) shown 114.7 s after it started at line 20
- back to gauntlet-h-4.png after the insert (20.0 s up)
