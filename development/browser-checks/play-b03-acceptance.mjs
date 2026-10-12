// PLAY-B03 acceptance frames: a test fight (N enemies) on the TV (WebGPU) and two phones, played to the end.
// Captures: a hero's typed line as a bubble over that hero's figure (camera eased in), a named damage text with its
// damage type, the victory celebration and the sound log. Asserted: the typed line's bubble was drawn by the battlefield
// (.bf-speech-bubble over the speaker, none of the flat stage's bubbles), a named float with a damage type was shown,
// the celebration lasted about six seconds, and the display's sound log has exactly one victory fanfare.
// usage: PORT=63711 DF_CHECK_PG_PORT=55711 N=1 [FOE=kobold-warrior] [SCAN=1] node play-b03-acceptance.mjs [frames-dir]
import fs from 'node:fs';
import path from 'node:path';
import { runner, startServer, launchGpu, playToOpening, click, settle, until, root } from './stage.mjs';

const N = process.env.N || '1';
const FOE = process.env.FOE || 'kobold-warrior';
const SCAN = process.env.SCAN === '1';
const PORT = process.env.PORT || '63711';
process.env.DF_BASE = `http://127.0.0.1:${PORT}`;
const framesDir = process.argv[2] || path.join(root, '..', '..', '..', 'artifacts', 'tmp', 'play-b03', 'acceptance');
fs.mkdirSync(framesDir, { recursive: true });
const { s, out, check, finish } = runner(`acceptance-${N}${SCAN ? '-scan' : ''}`, framesDir);
const env = { DF_GAMEPLAY_TEST_FIGHT: N, DF_GAMEPLAY_ADVENTURE: 'harbor', PORT };
if (FOE !== 'own') env.DF_GAMEPLAY_TEST_FOE = FOE;
if (SCAN) Object.assign(env, { DF_GAMEPLAY_BATTLE_SCENE: 'scan-village', DF_GAMEPLAY_SPLAT_DIR: process.env.DF_GAMEPLAY_SPLAT_DIR || '/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/splat-dl' });
await startServer(env);
const browser = await launchGpu(true);
const { tv, pa, pb } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true });
const UTILITY = ['Full character sheet', 'Ask for clarification', 'Play', 'Hero', 'Menu', 'Reconnect', 'Confirm saved choice', 'Leave this game on this device', 'Close'];
const buttons = (page) => page.evaluate((utility) => [...document.querySelectorAll('button:not([disabled])')].map((b) => b.textContent.trim().replace(/\s+/g, ' ')).filter((t) => t && !utility.includes(t)), UTILITY);
const begin = (await buttons(pa)).find((t) => /enter|begin|start/i.test(t));
if (begin) await click(s, pa, begin, { wait: 300 });
await settle(2500);
let first;
for (let i = 0; i < 100 && !first; i += 1) { first = (await buttons(pa)).find((t) => /^(ask|escort|offer to)/i.test(t)); if (!first) await settle(300); }
s.note(`the first choice: ${first}`);
await click(s, pa, first, { wait: 300 });
await until(tv, (now) => now.battle && now.battle !== 'absent', 30000);
await tv.waitForSelector('.battlefield', { timeout: 30000 }).catch(() => null);
// Wait out the entrance (banner, initiative, roll-call).
await until(tv, (now) => ['turn-start', 'acting', 'turn-end'].includes(now.battle), 60000).catch(() => null);

const tvRead = () => tv.evaluate(() => {
  const text = (e) => (e ? e.textContent.trim().replace(/\s+/g, ' ') : '');
  const bubble = document.querySelector('.bf-speech-bubble');
  const visible = (e) => { if (!e) return false; const r = e.getBoundingClientRect(); const cs = getComputedStyle(e); return r.width > 0 && cs.display !== 'none' && cs.visibility !== 'hidden' && Number(cs.opacity) > 0.2; };
  return {
    t: Date.now(), bubble: visible(bubble) && text(bubble.querySelector('.bf-speech-text')) ? { name: text(bubble.querySelector('.bf-speech-name')), text: text(bubble.querySelector('.bf-speech-text')) } : null,
    flatBubbles: [...document.querySelectorAll('.tv-bubble')].filter(visible).filter((b) => /Stay behind me/.test(text(b))).length,
    floats: [...document.querySelectorAll('.bf-float')].map((f) => text(f)),
    label: text(document.querySelector('.bf-turn')), finish: Boolean(document.querySelector('.bf-hud[data-finish]')),
    battle: document.querySelector('.tv-stage')?.getAttribute('data-battle-state'),
    sfx: document.documentElement.getAttribute('data-sfx-log') || '',
    aftermath: Boolean(document.querySelector('.live-screen[data-screen=aftermath]')),
  };
});

// 1. A hero's typed line during the fight.
await pa.evaluate(() => { const d = document.querySelector('details.say-more'); if (d) d.open = true; });
await pa.selectOption('select[aria-label="Input context"]', { label: 'Conversation' }).catch(() => null);
await pa.fill('textarea.dialogue-draft', 'Stay behind me, Thrain. We hold this line.');
await pa.locator('[data-dialogue=submit]').click();
await settle(1500);
s.note(`phone A after the line: ${(await pa.evaluate(() => document.body.innerText)).replace(/\s+/g, ' ').slice(0, 700)}`);
let speech = null;
for (let i = 0; i < 100 && !speech; i += 1) {
  const now = await tvRead();
  if (now.bubble) { speech = now; await tv.screenshot({ path: `${out}-speech.jpg`, type: 'jpeg', quality: 80 }); }
  await settle(100);
}
check(Boolean(speech), 'the typed line showed as a bubble over a figure on the battlefield');
if (speech) s.note(`bubble: ${speech.bubble.name}: ${speech.bubble.text} (flat bubbles visible: ${speech.flatBubbles})`);
check(!speech || speech.flatBubbles === 0, 'no flat-stage bubble was drawn for it');
await settle(1500);
await tv.screenshot({ path: `${out}-speech-later.jpg`, type: 'jpeg', quality: 80 });
if (process.env.STOP_AFTER_SPEECH === '1') await finish(browser);

// 2. Play to the end: hero turns attack, and the TV is watched for named floats and the celebration.
let namedFloat = null;
let finishStart = null;
let finishEnd = null;
let last = null;
const stop = Date.now() + 420000;
let lastMove = 0;
while (Date.now() < stop) {
  const now = await tvRead();
  last = now;
  const named = now.floats.find((f) => /· \d+ [a-z]+/.test(f));
  if (named && !namedFloat) { namedFloat = named; await tv.screenshot({ path: `${out}-float.jpg`, type: 'jpeg', quality: 80 }); }
  if ((now.finish || now.battle === 'finishing') && finishStart === null) { finishStart = now.t; await tv.screenshot({ path: `${out}-finishing.jpg`, type: 'jpeg', quality: 80 }); }
  if (finishStart !== null && now.t - finishStart > 3000 && !s.shot3) { s.shot3 = true; await tv.screenshot({ path: `${out}-celebration-3s.jpg`, type: 'jpeg', quality: 80 }); }
  if (finishStart !== null && now.aftermath && finishEnd === null) { finishEnd = now.t; await tv.screenshot({ path: `${out}-aftermath.jpg`, type: 'jpeg', quality: 80 }); break; }
  if (Date.now() - lastMove > 2500) {
    for (const page of [pa, pb]) {
      if (await click(s, page, 'Attack', { optional: true, wait: 300 })) {
        const list = await buttons(page);
        const target = list.find((t) => /cover|range|Kobold|Bandit|Goblin|Wolf|Scout|Skeleton|Zombie|Rat|Weasel|Mastiff|Guard|Cultist/i.test(t) && !/end turn|attack/i.test(t));
        if (target) await click(s, page, target, { optional: true, wait: 0 });
        lastMove = Date.now();
        break;
      }
      if (await click(s, page, 'End turn', { optional: true, wait: 0 })) { lastMove = Date.now(); break; }
    }
  }
  if (Date.now() % 20000 < 150) s.note(`tv: ${now.label} | battle ${now.battle} | floats ${JSON.stringify(now.floats)}`);
  await settle(100);
}
check(Boolean(namedFloat), `a named damage text with its type was shown (${namedFloat})`);
check(finishStart !== null, 'the fight ended on the battlefield');
if (finishStart !== null && finishEnd !== null) {
  const seconds = (finishEnd - finishStart) / 1000;
  s.note(`celebration: ${seconds.toFixed(1)} s from the finishing banner to the victory card`);
  check(seconds >= 5.5, 'the celebration lasted about six seconds or more');
}
const sounds = (last?.sfx || '').split(' ').filter(Boolean);
const fanfares = sounds.filter((x) => x === 'victory-fanfare').length;
s.note(`victory-fanfare entries in data-sfx-log: ${fanfares}`);
check(fanfares === 1, 'exactly one victory sting in the sound log');
await finish(browser);
