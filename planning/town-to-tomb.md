# Town to tomb: two phones, one quest (owner goal, 2026-11-16)

The next enablement scenario for the dev host (`df-table serve`, phones on the player listener).
It builds on [Phone controls and character creation](phone-and-creation.md) (P1 to P4), the
painter's day and the dice gauntlet. Rules follow [Rules support](rules-support.md) and the
admitted source, SRD 5.2.1 (`development/rules-sources/srd-5.2.1-en.pdf`, attribution in
`crates/df-rules/SRD-ATTRIBUTION.txt`). Every gameplay decision taken from words goes to Jev: a
model call with a closed schema, its answer checked in code against ids and enums. No word
matching.

## The run, as the owner asked

1. Two phones join the table by typing the room code shown on the TV (or opening its QR link).
2. Each phone makes a hero in the look studio. The owner picked the looks:
   - Phone 1, a **Tiefling Rogue**: lean, charcoal skin, curled horns capped in brass, black
     leather, twin daggers.
   - Phone 2, a **Human Wizard**: a woman in her sixties, cropped white hair, round spectacles, an
     ink-stained travel coat, a staff that carries a lantern.
3. The party walks into a town and goes to the adventurers' guild. The guild master shows the
   quest board, and the party takes the quest worth the most gold.
4. Before leaving town the party buys weapons and supplies at the town's shops (SRD prices and
   weights), haggles by talking, and fills its bags within carrying capacity.
5. The party travels at least five stops, place by place, to the quest site. Each stop has a
   question, and some have a check, a person to talk to or a clue.
6. At the tomb's sealed door the party solves one riddle from the clues it gathered. Each clue
   opens fullscreen on the phone, and a player can show it fullscreen on the TV.
7. Entering the tomb, an old gate takes the party to another continent: a new region, a new map
   and new people.
8. Exercised on the way: NPC talk, purchase and barter, items and baggage space, skills, ability
   checks and stats. Every plate is photographic (the existing `BriefStyle` direction), never
   an illustration.
9. The coordinator watches the TV and both phones at once, and an adversarial reviewer checks
   the code and the running result.

## Parts, owners and contracts

Each part has one owner and its own paths. Shared contracts are fixed here before work starts.

### A. Room code and two seats (seat worker)

- **Room code.** The table makes a code when its lobby opens: six characters from an
  unambiguous alphabet, shown on the TV lobby (the existing `lobby/model.rs::JoinCode`) with a
  real QR of `http://<lan>:7880/play?code=<code>`. The fake `qr_like_cells` goes.
- **Joining.** A new join (Guest, Friend, Account) must carry the code. A wrong code is refused
  with `code_mismatch`, and repeated misses from one address are slowed. A returning seat key for
  this table rejoins without a code. The phone asks for the code before the join choices, unless
  the link carries it.
- **Two seats start one run.** Start waits until every seated player has a locked hero and is
  ready. Then one runner starts for the whole party with the environment below. The single-seat
  variables stay for one-seat runs (the painter's day and the gauntlet).
  - `DF_RUN_PLAYERS="1,2"`: the seats, in seat order.
  - Per seat `n`: `DF_RUN_HERO_<n>_ID`, `_NAME`, `_SPECIES`, `_CLASS`, `_SHEET`, `_LOOK`,
    `_LOOK_KEY`, `_PERSONA`. They mean what the single-seat `DF_RUN_HERO_*` variables mean.
- **Dev view with two phones.** The playground's View gains **Table**: the TV with two phone
  frames beside it. Frame 2 loads `/play` from a second loopback origin (`localhost` beside
  `127.0.0.1`), so each frame keeps its own seat key. Both phones play through their own seat
  routes, never the dev CLI.
- **Clue fullscreen.** Tapping a clue on the Clues page opens it fullscreen on the phone, with
  pinch and double-tap zoom and a close button. **Show on the table** sends the seat action
  `show_clue { clue }`, and the TV shows that clue fullscreen until a phone closes it or the scene
  moves on. Clue ids come from the clue board; the host checks each id.

### B. Rogue and Wizard in play (hero-build worker)

The two picked classes must be playable at level 1 under SRD 5.2.1, not "design only".

- **Rogue 1:** Expertise (two skills at double proficiency, chosen at creation), Sneak Attack
  (shown on the sheet; it applies when combat arrives), Thieves' Cant, Weapon Mastery (two
  weapons), and Thieves' Tools proficiency for tool checks.
- **Wizard 1:** Spellcasting with Intelligence (spell save DC, spell attack, three cantrips, a
  spellbook of six level 1 spells, prepared spells, two level 1 slots), Ritual Adept, and Arcane
  Recovery.
- **Slots as vitals:** spending a slot, the long-rest refill and Arcane Recovery are new
  `VitalsChange` arms in the existing vitals path (`creation::apply_vitals`). There is no second
  path.
- **Casting outside combat:** the seat action `cast { spell, level }`. The host checks the spell
  is known, or prepared, or a ritual (with time), and checks the slot. It spends the slot and
  hands the cast to the dungeon master, who narrates what the spell shows. Jev judges what it
  reveals against the scene's facts.
- **Phone:** the Hero page lists spells with Cast, and the Roll page uses Expertise in skill
  totals. The display belongs to the seat worker once these sheet fields land.

### C. Equipment, weight and baggage (inventory worker)

- **Catalog:** the SRD 5.2.1 equipment tables (weapons, armour, adventuring gear, packs,
  containers, tools), each item with its cost, weight and printed page. It extends
  `srd_equipment_prices.rs` and does not replace it.
- **Carrying capacity:** Strength × 15 lb for Small and Medium creatures (SRD rules glossary,
  Carrying Capacity). Over the capacity, a hero can drag but not carry. The bag shows the total
  weight against capacity.
- **Baggage space:** a container's capacity (a backpack holds 30 lb in 1 cubic foot, a pouch 6
  lb, a sack 30 lb, and so on) limits what goes in it. An item with no room is refused with a
  reason, never dropped silently.
- **Shops:** a shop's stock comes from the catalog. The phone's Shop page lists stock with price
  and weight, and Buy sends the seat action `buy { item, qty }`. That goes through the same
  transfer as an agreed deal, so coins and goods move together or not at all. Haggling stays
  speech, judged as it is today.
- **Stability:** every inventory change goes through one owner and one log line (`INVENTORY ...`).
  A refused change leaves the bag and coins exactly as they were.
- **A Start is a fresh journey** (coordinator decision 2026-11-16, after run 9 doubled the
  starting kit): each hero's inventory is set to their starting kit and starting coins, never
  added to what a previous run left; the scenario's quests start unoffered. Grants are
  idempotent: starting the same run twice gives the same bag.
- **Change:** paying a price the payer's coins can't meet exactly takes larger coins and gives
  change from the payee's purse. Every scenario NPC who takes money (shopkeeper, toll keeper,
  ferryman) starts with a purse that can make change. A refusal tells the player why on the
  phone, and the NPC's line never claims a payment the purse did not record.

### D. Quest, riddle, teleport and the two-seat run (runner worker)

- **Quest:** a quest is authored in the scenario: an id, a title, its giver, a reward in gp, a
  site and the ids of its clues. The guild's board is a handout (a photographic close-up of
  three notices). The party picks a quest by tapping a card or by saying so; Jev picks among the
  quest ids. The accepted quest goes into world state, the TV journal and the phone's Notes. An
  advance in gold is paid to each hero on acceptance. The quest completes when the tomb opens
  (the riddle is solved): the reward is paid then, split evenly among the party's heroes, with
  the quest marked complete in world state, the TV journal and the phone's Notes (owner decision
  2026-11-16).
- **Two seats:**
  - Each ask goes to a named seat or to the party. A party ask goes to both phones. The first
  answer moves the party, and the other phone gets a short notice, as proposed in open item 6
  of the phone plan.
  - Personal moves (buy, check, talk, cast) stay with their seat.
  - Every plate is painted from both heroes' reference sheets.
- **Travel:** at least five stops between the town and the tomb, using the existing way and
  arrival machinery. Each stop has a question, and the stops between them include a check, an
  NPC and a clue.
- **Riddle:** the tomb door's riddle has one answer id and the clue ids that lead to it. A
  player's line goes to Jev, which judges it against the answer id and returns `solved`,
  `close` or `wrong`, with the clue ids it used. Hints follow the existing clue judge and never
  give the answer.
- **Teleport:** entering the tomb runs the region change. The party's place becomes the gate's
  far end on a new continent. The map clears to the new region, a title card marks the
  crossing, and the next plates come from the new region's design.
- **Scenario file:** `development/market-threads/town-to-tomb.beats`.

## Done when

- Both phones join with the code. A wrong code is refused. A refresh returns each phone to its
  seat.
- Both heroes are made from the phones and look as described. Both appear in every plate.
- The party takes the richest quest from the guild. Both heroes buy weapons and supplies with
  exact SRD prices and weights. A purchase that would overfill a container or exceed capacity is
  refused with a reason.
- The party makes five or more stops with at least one check, one NPC talk and one barter.
- The party gathers clues, opens one fullscreen on a phone and shows one on the TV, then solves
  the riddle.
- The tomb takes the party to the new continent, and the map shows it.
- The TV and both phones agree at every step (the same scene, the same calls and results, the
  same HP, coins and bags).
- An adversarial reviewer passes the code and the visual run.
- The runbook records the run.
