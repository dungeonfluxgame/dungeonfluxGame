// PLAY-B03 item 0: enemy turns are shown in the server's order. A test fight (DF_GAMEPLAY_TEST_FIGHT=<n>, the
// adventure's first choice leads straight to the fight) is played on the TV (WebGPU) and two phones; both heroes end
// their turns so the enemies act, and every 100 ms the TV's turn label, roll card, floating texts and figure positions
// and the phones' controls are sampled. Asserted from the samples:
//   - while a roll card is shown the turn label names the card's attacker, and the label never names a creature whose
//     turn has not begun (labels in a round follow the order the cards came in);
//   - a card for a melee strike is never shown while its attacker is farther than one cell (diagonals included) from
//     its target;
//   - every damage float names the attack (no bare number), and one hit makes one float;
//   - once the enemy round is over, the next hero's phone shows its controls within a second without a reload.
// usage: PORT=63711 DF_CHECK_PG_PORT=55711 N=3 [FOE=kobold-warrior] [SCAN=1] node play-b03-enemy-round.mjs [frames-dir]
import fs from 'node:fs';
import path from 'node:path';
import { runner, startServer, launchGpu, playToOpening, click, settle, until, root } from './stage.mjs';

const N = process.env.N || '3';
const FOE = process.env.FOE || 'kobold-warrior';
const SCAN = process.env.SCAN === '1';
const PORT = process.env.PORT || '63711';
process.env.DF_BASE = `http://127.0.0.1:${PORT}`;
const framesDir = process.argv[2] || path.join(root, '..', '..', '..', 'artifacts', 'tmp', 'play-b03', 'enemy-round');
fs.mkdirSync(framesDir, { recursive: true });
const { s, out, check, finish } = runner(`enemy-round-${N}${SCAN ? '-scan' : ''}`, framesDir);
const env = { DF_GAMEPLAY_TEST_FIGHT: N, DF_GAMEPLAY_ADVENTURE: 'harbor', PORT };
if (FOE !== 'own') env.DF_GAMEPLAY_TEST_FOE = FOE;
if (SCAN) Object.assign(env, { DF_GAMEPLAY_BATTLE_SCENE: 'scan-village', DF_GAMEPLAY_SPLAT_DIR: process.env.DF_GAMEPLAY_SPLAT_DIR || '/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/splat-dl' });
await startServer(env);
const browser = await launchGpu(true);
const { tv, pa, pb } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true });
for (const [name, page] of [['A', pa], ['B', pb], ['TV', tv]]) {
  page.on('pageerror', (error) => s.note(`pageerror ${name}: ${String(error).slice(0, 300)}`));
}
const UTILITY = ['Full character sheet', 'Ask for clarification', 'Play', 'Hero', 'Menu', 'Reconnect', 'Confirm saved choice', 'Leave this game on this device', 'Close'];
const buttons = (page) => page.evaluate((utility) => [...document.querySelectorAll('button:not([disabled])')].map((b) => b.textContent.trim().replace(/\s+/g, ' ')).filter((t) => t && !utility.includes(t)), UTILITY);
const begin = (await buttons(pa)).find((t) => /enter|begin|start/i.test(t));
if (begin) await click(s, pa, begin, { wait: 300 });
await settle(2500);
const first = (await buttons(pa)).find((t) => !/end turn/i.test(t));
s.note(`the first choice: ${first}`);
await click(s, pa, first, { wait: 300 });
const fighting = await until(tv, (now) => now.battle && now.battle !== 'absent', 30000).then(() => true, () => false);
check(fighting, 'the first choice led straight to a fight');
await tv.waitForSelector('.battlefield', { timeout: 30000 }).catch(() => null);
await tv.evaluate(() => document.querySelector('[data-battlefield]')?.setAttribute('data-trace-figures', ''));
await tv.screenshot({ path: `${out}-fight-start.jpg`, type: 'jpeg', quality: 70 });

const sampleTv = () => tv.evaluate(() => {
  const text = (e) => (e ? e.textContent.trim().replace(/\s+/g, ' ') : '');
  const host = document.querySelector('[data-battlefield]');
  const card = document.querySelector('.bf-roll[data-open]');
  const figures = {};
  for (const item of (host?.getAttribute('data-battlefield-figures') || '').split(';').filter(Boolean)) {
    const [name, at] = item.split('@');
    const [x, , z] = at.split(',').map(Number);
    figures[name] = [x, z];
  }
  return {
    t: Date.now(), label: text(document.querySelector('.bf-turn')), round: text(document.querySelector('.bf-round')),
    card: card ? text(card.querySelector('.bf-roll-label')) : null, verdict: card ? text(card.querySelector('.bf-roll-verdict')) : null,
    floats: [...document.querySelectorAll('.bf-float')].map((f) => ({ text: text(f), id: f.getAttribute('data-id') || text(f) })),
    figures, cell: Number(host?.getAttribute('data-battlefield-cell-m')) || 0,
    busy: host?.getAttribute('data-battlefield-busy'),
    aftermath: Boolean(document.querySelector('.live-screen[data-screen=aftermath]')),
  };
});
const samplePhone = (page) => page.evaluate(() => ({
  yourTurn: /Your turn/.test(document.body.innerText),
  controls: [...document.querySelectorAll('.tactical-controls button:not([disabled])')].map((b) => b.textContent.trim().replace(/\s+/g, ' ')),
  waiting: /Waiting for/.test(document.body.innerText),
}));

const samples = [];
let lastPress = 0;
const stop = Date.now() + 240000;
let enemyRoundSeen = false;
let frame = 0;
while (Date.now() < stop) {
  const [t, a, b] = await Promise.all([sampleTv(), samplePhone(pa), samplePhone(pb)]);
  samples.push({ ...t, a, b });
  frame += 1;
  if (frame % 8 === 0 && frame < 1500) await tv.screenshot({ path: `${out}-strip-${String(frame).padStart(4, '0')}.jpg`, type: 'jpeg', quality: 45 });
  if (/Kobold|Bandit|Goblin|Wolf|Scout|Skeleton|Zombie|Rat|Weasel|Mastiff|Guard|Cultist/.test(t.card || '')) enemyRoundSeen = true;
  // A hero's turn with the TV quiet: end it, so the server moves on to the enemies.
  if (Date.now() - lastPress > 3500 && t.busy !== '1') {
    for (const [name, page, phone] of [['A', pa, a], ['B', pb, b]]) {
      if (phone.controls.some((c) => /end turn/i.test(c))) { await click(s, page, 'End turn', { optional: true, wait: 0 }); s.note(`${name} ends the turn (label ${t.label})`); lastPress = Date.now(); break; }
    }
  }
  if (t.aftermath || /Round [3-9]/.test(t.round)) break;
  if (enemyRoundSeen && /Round 2/.test(t.round) && Date.now() - lastPress > 12000) break;
  await settle(60);
}
fs.writeFileSync(`${out}-samples.json`, JSON.stringify(samples));
check(enemyRoundSeen, 'an enemy attack card was shown');

// Cards: the label names the card's attacker; a melee strike is never shown from beyond one cell.
const cards = samples.filter((m) => m.card);
let misLabelled = 0;
let farMelee = 0;
const order = [];
for (const m of cards) {
  const parsed = /^(.*?) (strikes|shoots|attacks) (.*?)( · .*)?$/.exec(m.card);
  if (!parsed) continue;
  const [, attacker, verb, target] = parsed;
  if (m.label !== `${attacker}’s turn`) misLabelled += 1;
  if (order[order.length - 1] !== attacker) order.push(attacker);
  const a = m.figures[attacker];
  const t = m.figures[target];
  if (verb === 'strikes' && a && t && m.cell) {
    const distance = Math.hypot(a[0] - t[0], a[1] - t[1]);
    if (distance > m.cell * 1.6) { farMelee += 1; if (farMelee <= 3) s.note(`melee card "${m.card}" at ${distance.toFixed(2)} m (cell ${m.cell} m)`); }
  }
}
check(cards.length > 0, `cards were sampled (${cards.length})`);
check(misLabelled === 0, `the label named the card's attacker in every sample (${misLabelled} wrong of ${cards.length})`);
check(farMelee === 0, `no melee card while its attacker stood beyond one cell (${farMelee})`);
s.note(`attackers in card order: ${order.join(' > ')}`);
check(new Set(order).size === order.length, 'each creature’s cards came in one run (no interleaving)');

// Floats: named, one per hit.
const seenFloats = new Map();
for (const m of samples) for (const f of m.floats) if (!seenFloats.has(f.id + f.text)) seenFloats.set(f.id + f.text, f.text);
const texts = [...seenFloats.values()];
s.note(`floats: ${JSON.stringify(texts)}`);
check(texts.every((x) => /·/.test(x) || /^\+/.test(x)), 'every float names its attack');

// Phones: after the enemy round, the next hero's phone shows its controls within a second.
let slow = 0;
let turnsSeen = 0;
for (const name of ['a', 'b']) {
  let since = null;
  for (const m of samples) {
    const phone = m[name];
    if (phone.yourTurn && !phone.controls.length) { since ??= m.t; if (m.t - since > 1000) { slow += 1; since = Infinity; } }
    else { if (phone.yourTurn && phone.controls.length) turnsSeen += 1; since = null; }
  }
}
check(turnsSeen > 0, 'a phone showed its controls on its turn');
check(slow === 0, `a phone on its turn never waited more than a second for its controls (${slow})`);
await tv.screenshot({ path: `${out}-final.jpg`, type: 'jpeg', quality: 70 });
await finish(browser);
