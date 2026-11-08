# Run 56: inserts keep the cast

Inserts shown with the room's layout (no line refused), the room back after about 21 s. Found: a room frame flashed by when an insert queued behind it, the cut came back to an older room, the last insert was taken down at once, and the item backfill was refused (expertise is not fillable). See persona-refinement-log.md, refinement 21.

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

**[01] Mara:** How did you form the longsword’s fuller and temper, and were the frost runes cut before or after the quench?

**[02] Haldor:** I fuller it to lighten and stiffen the blade, then clay-back the quench and temper it long so the edge holds while the spine flexes. Frost runes go in after the quench, silver-inlaid and woken by a rune-scribe.
- behavior: careful-appraisal
- behavior override: trigger appraisal-question fired, so the line took careful-appraisal, not shop-talk
- fired triggers: appraisal-question, knowing-question
- fired emotional triggers: questioned-claims (by state: none)
- tone: npc
- feels: pride 67, trust 52, joy 42; mood 44 valence, 50 arousal; toward mara_adventurer: indifferent (3); focus mara_adventurer on longsword forging
- time: line 7.6s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7595 ms

**[03] Jory:** So, are those glowing blades hot enough to cook supper on?

**[04] Haldor:** Cook supper on them? You’d spoil a fine blade and a poor stew. The runes glow, but they’re cool to the touch.
- behavior: patient-tease
- fired triggers: foolish-question
- tone: npc
- feels: pride 64, trust 51, joy 42; mood 44 valence, 50 arousal; toward mara_adventurer: indifferent (3); focus jory_adventurer on rune glow and heat
- time: line 6.6s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6685 ms

**[05] Sella:** Those runes are real enchantments, then—not painted glow to fleece travellers?

**[06] Haldor:** Painted? I forged these blades myself. The runes are silver-inlaid and wake to a real enchantment; feel the Frostline’s chill, if you doubt me.
- behavior: stung-pride
- fired triggers: doubted-craft
- fired emotional triggers: questioned-claims (by state: none)
- tone: npc
- feels: pride 75, trust 50, joy 42; mood 45 valence, 50 arousal; toward mara_adventurer: indifferent (3); focus sella_adventurer on rune authenticity
- time: line 7.6s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7602 ms

**[07] Bram:** Fair enough. What do you ask for the Frostline longsword, Emberfang scimitar, Verdant bastard sword, and Nightglass dagger?
- fired triggers: welcome-settles

**[08] Haldor:** Frostline is 450 gold, Emberfang 380 gold, Verdant 520 gold, and Nightglass 260 gold; all four make 1610 gold together.
- quoted: 45000 cp for 1 frostline_longsword (inside floor 40500, ask 45000)
- quoted: 38000 cp for 1 emberfang_scimitar (inside floor 34200, ask 38000)
- quoted: 52000 cp for 1 verdant_bastard_sword (inside floor 46800, ask 52000)
- quoted: 26000 cp for 1 nightglass_dagger (inside floor 23400, ask 26000)
- behavior: closing-the-sale
- fired triggers: buyer-interest
- tone: ally
- feels: pride 70, trust 49, joy 48; mood 44 valence, 50 arousal; toward mara_adventurer: indifferent (3); toward bram_adventurer: indifferent (4); focus bram_adventurer on prices of the four blades
- time: line 8.0s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 8076 ms

**[09] Mara:** Which of these would hold its edge best on a long road?

**[10] Haldor:** The Verdant holds its edge longest on a long road; that heavy, tough steel asks less honing.
- behavior: careful-appraisal
- fired triggers: appraisal-question
- fired emotional triggers: questioned-claims (by state: none)
- tone: npc
- feels: pride 78, trust 52, joy 46; mood 47 valence, 49 arousal; toward mara_adventurer: indifferent (5); toward bram_adventurer: indifferent (4); focus mara_adventurer on edge retention on the road
- time: line 7.7s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7701 ms

**[11] Mara:** Could you hold the Verdant up to the light, so I can study its vine etching and gold-inlaid guard up close?

**[12] Haldor:** Certainly. The vine etching runs clean along the steel, and the gold-inlaid guard catches the light; mind the edge as you look.
- behavior: show-the-blade
- fired triggers: close-look
- fired emotional triggers: earned-confidence (by state: none)
- tone: ally
- feels: pride 73, trust 66, joy 58; mood 48 valence, 49 arousal; toward mara_adventurer: indifferent (7); toward bram_adventurer: indifferent (4); focus mara_adventurer on examining the Verdant
- time: line 7.2s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7232 ms

**[13] Jory:** Is that little black dagger meant for children, then? It’s so small.

**[14] Haldor:** Small, yes; a child's toy, no. Nightglass is made for quick, quiet work up close.
- behavior: patient-tease
- fired triggers: foolish-question
- tone: npc
- feels: pride 74, trust 63, joy 54; mood 47 valence, 49 arousal; toward mara_adventurer: indifferent (7); toward bram_adventurer: indifferent (4); toward jory_adventurer: indifferent (1); focus jory_adventurer on Nightglass dagger
- time: line 7.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7846 ms

**[15] Bram:** That’s a fine blade, not a dear one. What would you have me take for it?

**[16] Haldor:** For a long road, take the Verdant; its heavy, tough steel holds an edge longest.
- behavior: careful-appraisal
- fired triggers: appraisal-question
- fired emotional triggers: earned-confidence (by state: none)
- tone: ally
- feels: pride 76, trust 71, joy 64; mood 48 valence, 49 arousal; toward mara_adventurer: indifferent (7); toward bram_adventurer: indifferent (7); toward jory_adventurer: indifferent (1); focus bram_adventurer on choosing a blade
- time: line 6.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6773 ms

**[17] Sella:** Could you show me the Frostline runes working here, so I can see they’re genuine?

**[18] Haldor:** Certainly. Hold your hand over the flat—feel that chill; the runes are cool, and they bite a wound with cold.
- behavior: careful-appraisal
- fired triggers: appraisal-question
- fired emotional triggers: questioned-claims, earned-confidence (by state: none)
- tone: ally
- feels: pride 83, trust 79, joy 71; mood 52 valence, 48 arousal; toward mara_adventurer: indifferent (7); toward bram_adventurer: indifferent (7); toward jory_adventurer: indifferent (1); toward sella_adventurer: indifferent (3); focus sella_adventurer on proving the Frostline runes
- time: line 7.1s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7111 ms

**[19] Sella:** Could I hold the Frostline and look close at the frost runes along the fuller, the guard, and the grip?

**[20] Haldor:** Certainly—here it is, hilt first. Look at the silver inlay along the fuller, the guard, and the grip; the runes stay cool to the touch.
- behavior: show-the-blade
- fired triggers: close-look
- fired emotional triggers: earned-confidence (by state: none)
- tone: ally
- feels: trust 84, pride 77, joy 76; mood 51 valence, 48 arousal; toward mara_adventurer: indifferent (7); toward bram_adventurer: indifferent (7); toward jory_adventurer: indifferent (1); toward sella_adventurer: indifferent (5); focus sella_adventurer on examining the Frostline
- time: line 7.9s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7950 ms

**[21] Kael:** Which of the four suits a guard fighting in narrow streets?

**[22] Haldor:** The Frostline suits that work best: a guard’s blade, and its cold bite can slow a wound.
- behavior: careful-appraisal
- fired triggers: appraisal-question
- fired emotional triggers: questioned-claims (by state: none)
- tone: ally
- feels: trust 84, pride 82, joy 78; mood 54 valence, 48 arousal; toward mara_adventurer: indifferent (7); toward bram_adventurer: indifferent (7); toward jory_adventurer: indifferent (1); toward sella_adventurer: indifferent (5); toward warrior: indifferent (2); focus warrior on choosing a guard's blade
- time: line 6.8s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 6775 ms

**[23] Kael:** Could I see the Nightglass up close—the violet sigils, its edge, and the bone handle?

**[24] Haldor:** Take it hilt first. The violet sigils are silver-inlaid; mind the keen edge, and see how the bone handle sits in your palm.
- behavior: show-the-blade
- fired triggers: close-look
- worded again: the line reuses a phrase or shape of recent
- tone: ally
- feels: trust 84, joy 79, pride 76; mood 53 valence, 48 arousal; toward mara_adventurer: indifferent (7); toward bram_adventurer: indifferent (7); toward jory_adventurer: indifferent (1); toward sella_adventurer: indifferent (5); toward warrior: indifferent (4); focus warrior on examining the Nightglass
- time: line 14.1s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 14174 ms

**[25] Bram:** What would you take for the scimitar and the dagger together?

**[26] Haldor:** Together, the Emberfang and Nightglass come to 608 gold; fair trade for two well-made blades.
- quoted: 36100 cp for 1 emberfang_scimitar (inside floor 34200, ask 38000)
- quoted: 24700 cp for 1 nightglass_dagger (inside floor 23400, ask 26000)
- behavior: closing-the-sale
- fired triggers: buyer-interest
- fired emotional triggers: earned-confidence (by state: none)
- tone: ally
- feels: trust 87, joy 84, pride 71; mood 52 valence, 48 arousal; toward mara_adventurer: indifferent (7); toward bram_adventurer: indifferent (12); toward jory_adventurer: indifferent (1); toward sella_adventurer: indifferent (5); toward warrior: indifferent (4); focus bram_adventurer on pricing two blades
- time: line 7.3s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7374 ms

**[27] Kael:** I’ll take the Frostline today, if you’ll give me a fair price. What’s your best offer?

**[28] Haldor:** Fair trade, then: the Frostline is yours for 427 gold and 5 silver.
- quoted: 42750 cp for 1 frostline_longsword (inside floor 40500, ask 45000)
- behavior: closing-the-sale
- fired triggers: buyer-interest
- worded again: the line says a price without its slot: write {price:N} where it says a lot's price and {order_total} where it says what lots come to together, never the number
- tone: ally
- feels: trust 88, joy 86, pride 67; mood 52 valence, 48 arousal; toward mara_adventurer: indifferent (7); toward bram_adventurer: indifferent (12); toward jory_adventurer: indifferent (1); toward sella_adventurer: indifferent (5); toward warrior: indifferent (9); focus warrior on selling the Frostline
- time: line 14.3s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 14375 ms

**[29] Mara:** How should I care for the Frostline's runes on the road?

**[30] Haldor:** Keep the runes dry and clean; oil the steel, but never grind across the silver—if a rune dulls, a scribe resets it, not a whetstone.
- behavior: careful-appraisal
- fired triggers: appraisal-question
- fired emotional triggers: questioned-claims (by state: none)
- tone: npc
- feels: trust 88, joy 75, pride 75; mood 53 valence, 48 arousal; toward mara_adventurer: indifferent (9); toward bram_adventurer: indifferent (12); toward jory_adventurer: indifferent (1); toward sella_adventurer: indifferent (5); toward warrior: indifferent (9); focus mara_adventurer on Frostline rune care
- time: line 7.1s
- covered: true
- backfill: not needed
- model: codex:gpt-6-luna
- total: 7122 ms
- --- frames (artifacts/tmp/market/run56-frames.log) ---
- [01] Mara: How did you form the longsword’s fuller and temper, and were the frost runes cut before or after the quench?
- frame judge after line 1: 0% changed, 0% since the frame (gate 25)
- [02] Haldor Brask: I fuller it to lighten and stiffen the blade, then clay-back the quench and temper it long so the edge holds while the spine flexes. Frost runes go in after the quench, silver-inlaid and woken by a rune-scribe.
- frame judge after line 2: 10% changed, 10% since the frame (gate 25); Mara expression: Looks curious and intent as she asks about the longsword.; Haldor Brask expression: Answers with a focused, explanatory look.
- [03] Jory: So, are those glowing blades hot enough to cook supper on?
- frame judge after line 3: 25% changed, 25% since the frame (gate 25); Mara pose: leans toward the longsword, asking about its fuller, temper, and runes; Haldor Brask pose: explains the longsword's construction, gesturing toward it from behind the counter; Jory expression: grins after joking about cooking supper on the glowing blades
- [04] Haldor Brask: Cook supper on them? You’d spoil a fine blade and a poor stew. The runes glow, but they’re cool to the touch.
- frame judge after line 4: 25% changed, 25% since the frame (gate 25); Mara pose: leans in toward the longsword as she asks about its fuller and frost runes; Haldor Brask pose: gestures toward the longsword while explaining its forging and runes; Jory expression: grins after joking about cooking supper on the glowing blades; Haldor Brask expression: smiles at Jory's joke as he answers
- [05] Sella: Those runes are real enchantments, then—not painted glow to fleece travellers?
- frame judge after line 5: 25% changed, 25% since the frame (gate 25); Mara pose: Leans toward the counter, asking about the longsword’s fuller, temper, and runes.; Haldor Brask pose: Explains the longsword’s crafting with a hand indicating the blades.; Jory expression: Grins as he jokes about cooking supper on the glowing blades.; Haldor Brask expression: Smiles at Jory’s joke while explaining the runes are cool.; Sella expression: Looks skeptical, questioning whether the runes are genuine enchantments.
- [06] Haldor Brask: Painted? I forged these blades myself. The runes are silver-inlaid and wake to a real enchantment; feel the Frostline’s chill, if you doubt me.
- frame judge after line 6: 30% changed, 30% since the frame (gate 25); Haldor Brask pose: Haldor gestures toward the Frostline longsword, inviting Sella to feel its chill.; Sella pose: Sella reaches toward the Frostline longsword to test its chill.; Sella expression: Sella watches the runes with a narrowed, doubtful look.; Jory expression: Jory grins at his supper-on-the-blades joke.
- frame 1 (gauntlet-e-1) started after line 6, shot insert on Frostline longsword, editing weapons-stall-blades.png
- [07] Bram: Fair enough. What do you ask for the Frostline longsword, Emberfang scimitar, Verdant bastard sword, and Nightglass dagger?
- frame judge after line 7: 0% changed, 0% since the frame (gate 25)
- [08] Haldor Brask: Frostline is 450 gold, Emberfang 380 gold, Verdant 520 gold, and Nightglass 260 gold; all four make 1610 gold together.
- frame judge after line 8: 0% changed, 0% since the frame (gate 25)
- [09] Mara: Which of these would hold its edge best on a long road?
- frame judge after line 9: 10% changed, 10% since the frame (gate 25); Haldor Brask expression: Haldor looks attentive as he answers the price question.; Mara expression: Mara studies the blades with a practical, appraising look as she asks about edge durability.
- [10] Haldor Brask: The Verdant holds its edge longest on a long road; that heavy, tough steel asks less honing.
- frame judge after line 10: 10% changed, 10% since the frame (gate 25); Mara expression: She looks thoughtful as she asks about the Verdant's edge.; Haldor Brask expression: He answers matter-of-factly, indicating the Verdant bastard sword.
- [11] Mara: Could you hold the Verdant up to the light, so I can study its vine etching and gold-inlaid guard up close?
- insert gauntlet-e-1: gauntlet-e-1.png is behind the scene, with the cast of weapons-stall-blades.png; no details backfilled (refused (not_fillable): the persona's expertise is not a field a backfill can fill)
- frame 1 (gauntlet-e-1) shown 48.9 s after it started at line 6
- frame judge after line 11: 30% changed, 30% since the frame (gate 25); Haldor Brask prop: The four blades remain laid out on the red velvet, with their prices stated: Frostline 450 gold, Emberfang 380, Verdant 520, and Nightglass 260.; Mara expression: Mara studies the Verdant bastard sword with focused interest.; Haldor Brask item: Haldor lifts the Verdant bastard sword toward the light, showing its vine etching and gold-inlaid guard.
- frame 2 (gauntlet-e-2) started after line 11, shot insert on Verdant bastard sword, editing weapons-stall-blades.png
- [12] Haldor Brask: Certainly. The vine etching runs clean along the steel, and the gold-inlaid guard catches the light; mind the edge as you look.
- frame judge after line 12: 10% changed, 10% since the frame (gate 25); Haldor Brask pose: Leans in over the counter, indicating the green-runed bastard sword as he describes its etching and guard.; the room light: Light catches the gold-inlaid guard of the green-runed bastard sword.
- [13] Jory: Is that little black dagger meant for children, then? It’s so small.
- back to weapons-stall-blades.png after the insert (22.4 s up)
- frame judge after line 13: 15% changed, 15% since the frame (gate 25); Haldor Brask pose: Leans over the counter, indicating the green-runed bastard sword's clean vine etching and gold-inlaid guard.; Jory expression: Looks at the small black dagger with a dismissive, teasing expression.
- [14] Haldor Brask: Small, yes; a child's toy, no. Nightglass is made for quick, quiet work up close.
- frame judge after line 14: 15% changed, 15% since the frame (gate 25); Haldor Brask item: Indicates the green-runed bastard sword's clean vine etching and gold-inlaid guard as he cautions about its edge.; Jory expression: Looks dismissively amused at the dagger's small size.; Haldor Brask expression: Firmly corrects Jory, matter-of-fact.
- [15] Bram: That’s a fine blade, not a dear one. What would you have me take for it?
- frame judge after line 15: 15% changed, 15% since the frame (gate 25); Haldor Brask item: Haldor indicates the green-runed bastard sword, its gold-inlaid guard catching the light.; Jory expression: Jory looks dismissively at the small black dagger.; Bram pose: Bram lowers his hand from his chin and gestures toward the blades as he asks about a trade.
- [16] Haldor Brask: For a long road, take the Verdant; its heavy, tough steel holds an edge longest.
- insert gauntlet-e-2: gauntlet-e-2.png is behind the scene, with the cast of weapons-stall-blades.png; no details backfilled (refused (not_fillable): the persona's expertise is not a field a backfill can fill)
- frame 2 (gauntlet-e-2) shown 47.0 s after it started at line 11
- frame judge after line 16: 25% changed, 25% since the frame (gate 25); Haldor Brask item: Haldor indicates the Verdant bastard sword’s clean vine etching and gold-inlaid guard as he warns about its edge.; Jory expression: Jory looks skeptical, questioning whether the small Nightglass dagger is a child’s toy.; Haldor Brask expression: Haldor responds firmly that the Nightglass dagger is a real blade for quick, quiet work up close.; Bram expression: Bram appraises the blade as fine but not expensive and asks its price.; Haldor Brask item: Haldor recommends the Verdant bastard sword for a long road, pointing out its tough steel and long-lasting edge.
- [17] Sella: Could you show me the Frostline runes working here, so I can see they’re genuine?
- frame judge after line 17: 30% changed, 30% since the frame (gate 25); Haldor Brask pose: Haldor points along the Verdant bastard sword's vine etching, keeping a hand near its edge.; Jory expression: Jory looks skeptical as he remarks on the dagger's small size.; Haldor Brask item: Haldor indicates the Verdant bastard sword as the durable choice for a long road.; Sella pose: Sella turns her attention to the Frostline longsword and asks to see its runes working.
- frame 3 (gauntlet-e-3) started after line 17, shot over-the-shoulder, editing weapons-stall-blades.png
- [18] Haldor Brask: Certainly. Hold your hand over the flat—feel that chill; the runes are cool, and they bite a wound with cold.
- back to weapons-stall-blades.png after the insert (21.3 s up)
- frame judge after line 18: 30% changed, 30% since the frame (gate 25); Haldor Brask pose: Haldor leans over the counter and gestures to the Verdant sword's flat, inviting someone to hold a hand above it.; Sella pose: Sella holds a hand over the Verdant sword's flat to feel the chill.; Sella expression: Sella studies the runes closely as she feels their cold bite.
- [19] Sella: Could I hold the Frostline and look close at the frost runes along the fuller, the guard, and the grip?
- frame judge after line 19: 30% changed, 30% since the frame (gate 25); Haldor Brask pose: Haldor invites Sella to hold her hand over the Frostline's flat and indicates its blade.; Sella pose: Sella holds the Frostline longsword up close to study the frost runes along its fuller, guard, and grip.; the room item: The Frostline longsword is lifted from the counter for close inspection.
- frame 4 (gauntlet-e-4) started after line 19, shot insert on Frostline longsword, editing weapons-stall-blades.png
- [20] Haldor Brask: Certainly—here it is, hilt first. Look at the silver inlay along the fuller, the guard, and the grip; the runes stay cool to the touch.
- [21] Kael: Which of the four suits a guard fighting in narrow streets?
- [22] Haldor Brask: The Frostline suits that work best: a guard’s blade, and its cold bite can slow a wound.
- [23] Kael: Could I see the Nightglass up close—the violet sigils, its edge, and the bone handle?
- [24] Haldor Brask: Take it hilt first. The violet sigils are silver-inlaid; mind the keen edge, and see how the bone handle sits in your palm.
- [25] Bram: What would you take for the scimitar and the dagger together?
- [26] Haldor Brask: Together, the Emberfang and Nightglass come to 608 gold; fair trade for two well-made blades.
- [27] Kael: I’ll take the Frostline today, if you’ll give me a fair price. What’s your best offer?
- [28] Haldor Brask: Fair trade, then: the Frostline is yours for 427 gold and 5 silver.
- frame 3 (gauntlet-e-3) shown 126.9 s after it started at line 17
- insert gauntlet-e-4: gauntlet-e-4.png is behind the scene, with the cast of weapons-stall-blades.png; no details backfilled (refused (not_fillable): the persona's expertise is not a field a backfill can fill)
- frame 4 (gauntlet-e-4) shown 107.6 s after it started at line 19
- frame judge after line 28: 50% changed, 50% since the frame (gate 25); Haldor Brask item: Offers the Frostline longsword hilt first, indicating its silver inlay, guard, grip and cool runes.; Kael pose: Takes and closely examines the Nightglass dagger hilt first, looking over its violet sigils, keen edge and bone handle.; Bram expression: Turns attention to the scimitar and dagger while asking their combined price.; Haldor Brask expression: Explains the Frostline is suited to narrow-street guard work and quotes 427 gold and 5 silver.; Kael expression: Shows interest in buying the Frostline and asks for a fair price.
- frame 5 (gauntlet-e-5) started after line 28, shot insert on Nightglass dagger, editing weapons-stall-blades.png
- [29] Mara: How should I care for the Frostline's runes on the road?
- frame judge after line 29: 0% changed, 0% since the frame (gate 25)
- [30] Haldor Brask: Keep the runes dry and clean; oil the steel, but never grind across the silver—if a rune dulls, a scribe resets it, not a whetstone.
- back to weapons-stall-blades.png after the insert (21.9 s up)
- frame judge after line 30: 30% changed, 30% since the frame (gate 25); Mara pose: Turns her attention from the blades toward Haldor as she asks about the Frostline's runes.; Haldor Brask pose: Leans toward Mara and gestures toward the Frostline while explaining its care.
- insert gauntlet-e-5: gauntlet-e-5.png is behind the scene, with the cast of gauntlet-e-3.png; no details backfilled (refused (not_fillable): the persona's expertise is not a field a backfill can fill)
- frame 5 (gauntlet-e-5) shown 46.6 s after it started at line 28
- back to gauntlet-e-3.png after the insert (0.0 s up)
