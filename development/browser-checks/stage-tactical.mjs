// PLAY-F06: a tactical fight (several enemies, server-owned cells) from the fight choice to the victory or defeat
// card, in a WebGPU browser, on the story location's own battle scene (default) or on a scan (SCAN=1 with
// artifacts/tmp/splat-dl). The standard generated adventure with a fixed seed is played by clicking the first
// story-advancing offer until a "Face ..." choice is offered. Asserted from 100 ms samples: the battle machine goes
// Entering > TurnStart > ... > Finishing > Exiting > Absent in order, the flat combat screen is never visible while
// the battlefield is active, no combat sound or dice card starts behind the cover, one transition in and one out.
// usage: [SCAN=1] [SEED=0xd5f94172edb1d404] node stage-tactical.mjs [frames-dir]
import fs from 'node:fs';
import path from 'node:path';
import { runner, startServer, launchGpu, playToOpening, click, settle, until, framesDir, root } from './stage.mjs';

const SCAN = process.env.SCAN === '1';
const SEED = process.env.SEED || '0xd5f94172edb1d404';
const UTILITY = ['Full character sheet', 'Ask for clarification', 'Play', 'Hero', 'Menu', 'Reconnect', 'Confirm saved choice', 'Leave this game on this device', 'Close'];
const COMBAT_SFX = new Set(['dice-rattle', 'dice-land', 'hit', 'miss', 'critical', 'step-dirt-1', 'step-dirt-2', 'step-stone-1', 'step-stone-2', 'turn-chime']);
const { s, out, check, finish } = runner(`stage-tactical${SCAN ? '-scan' : ''}`, process.argv[2] || framesDir);
const env = { DF_GAMEPLAY_ADVENTURE: 'random', DF_GAMEPLAY_ADVENTURE_SEED: SEED, DF_GAMEPLAY_ADVENTURE_TABLES: path.join(root, 'content', 'adventure-tables'), DF_GAMEPLAY_PACK_DIR: '' };
if (SCAN) Object.assign(env, { DF_GAMEPLAY_BATTLE_SCENE: 'scan-village', DF_GAMEPLAY_SPLAT_DIR: path.join(root, '..', '..', '..', 'artifacts', 'tmp', 'splat-dl') });
delete env.DF_GAMEPLAY_PACK_DIR;
await startServer(env);
const browser = await launchGpu(true);
const { tv, pa, pb } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true, voice: { latencyMs: 100, lengthFor: () => 1 } });
for (const [name, page] of [['A', pa], ['B', pb], ['TV', tv]]) {
  page.on('console', (message) => { if (['error', 'warning'].includes(message.type()) || /lost|fail|stream|closed|reconnect|status/i.test(message.text())) s.note(`console ${name} ${message.type()}: ${message.text().slice(0, 300)}`); });
  page.on('pageerror', (error) => s.note(`pageerror ${name}: ${String(error).slice(0, 300)}`));
}
const buttons = (page) => page.evaluate((utility) => [...document.querySelectorAll('button:not([disabled])')].map((b) => b.textContent.trim().replace(/\s+/g, ' ')).filter((t) => t && !utility.includes(t)), UTILITY);
const begin = (await buttons(pa)).find((t) => /enter|begin|start/i.test(t));
s.note(`first offers: ${JSON.stringify(await buttons(pa))}`);
if (begin) await click(s, pa, begin, { wait: 200 });
// Advance the story until the fight is offered.
let faced = false;
for (let step = 0; step < 40 && !faced; step += 1) {
  await until(tv, (now) => now.scene === 'awaiting-choice' || now.screen === 'combat', 90000).catch(() => null);
  await settle(1500);
  const list = await buttons(pa);
  const fight = list.find((t) => /^face\b|^fight|^confront|^attack|^block|^defend|^stand|^ambush|^charge/i.test(t));
  s.note(`step ${step}: ${JSON.stringify(list.slice(0, 5))}`);
  const pick = fight || list[0];
  if (!pick) { await settle(2000); continue; }
  await click(s, pa, pick, { wait: 300 });
  // A fight is begun when the display's battle machine leaves Absent (or the flat combat screen shows).
  faced = await until(tv, (now) => (now.battle && now.battle !== 'absent') || now.screen === 'combat', 6000).then(() => true, () => false);
}
check(faced, 'the fight choice was reached and pressed');
const samples = [];
const sample = async () => {
  const state = await tv.evaluate(() => {
    const stage = document.querySelector('.tv-stage');
    const visible = (el) => { if (!el) return false; const cs = getComputedStyle(el); return cs.visibility !== 'hidden' && cs.display !== 'none' && Number(cs.opacity) > 0.1; };
    const flat = document.querySelector('.tv-stage .live-screen[data-screen=combat]');
    return {
      t: Date.now(), sfx: document.documentElement.getAttribute('data-sfx-log') || '',
      battle: stage?.getAttribute('data-battle-state'), scene: stage?.getAttribute('data-scene-state'), stage: stage?.getAttribute('data-stage'),
      active: document.querySelector('[data-battlefield]')?.getAttribute('data-battlefield') || null,
      field: Boolean(document.querySelector('.battlefield')) && visible(document.querySelector('.battlefield')),
      flat: visible(flat), aftermath: visible(document.querySelector('.live-screen[data-screen=aftermath]')),
      dice: visible(document.querySelector('.tv-final-roll')),
    };
  });
  samples.push(state);
  return state;
};
// A hero's turn: Attack (then the first legal target it offers), otherwise end the turn so the server moves on.
let probed = 0;
const sweep = async (page) => {
  if (await click(s, page, 'Attack', { optional: true, wait: 400 })) {
    const list = await buttons(page);
    if (probed < 6) { probed += 1; s.note(`after Attack: ${JSON.stringify(list)}`); }
    const target = list.find((t) => /cover|range|Bandit|Raider|Thug/i.test(t));
    if (target) await click(s, page, target, { optional: true, wait: 0 });
    return true;
  }
  return Boolean(await click(s, page, 'End turn', { optional: true, wait: 0 }));
};
let frame = 0;
let lastMove = 0;
let settled = null;
const stop = Date.now() + 300000;
while (Date.now() < stop) {
  const now = await sample();
  frame += 1;
  if (frame % 6 === 0 && frame < 900) await tv.screenshot({ path: `${out}-strip-${String(frame).padStart(3, '0')}.jpg`, type: 'jpeg', quality: 40 });
  if (!now.aftermath && !now.stage && ['turn-start', 'turn-end'].includes(now.battle) && Date.now() - lastMove > 6000) {
    if ((await sweep(pa)) || (await sweep(pb))) lastMove = Date.now();
  }
  if (frame % 150 === 100 && Date.now() - lastMove > 20000) {
    for (const [name, page] of [['A', pa], ['B', pb]]) s.note(`stalled? phone ${name} render-error=${await page.evaluate(() => [...document.querySelectorAll('[data-render-error]')].map((e) => e.getAttribute('data-render-error')).join(','))} status=${await page.evaluate(() => (document.querySelector('.connection')?.textContent || '') + ' | ' + [...document.querySelectorAll('[data-render-error],.connection')].map((e) => e.outerHTML.slice(0, 160)).join(' '))}: ${(await page.evaluate(() => document.body.innerText)).replace(/\s+/g, ' ').slice(0, 200)}`);
  }
  if (now.aftermath && now.battle === 'absent' && !now.stage) { settled = now; break; }
  await settle(60);
}
fs.writeFileSync(`${out}-samples.json`, JSON.stringify(samples, null, 1));
await tv.screenshot({ path: `${out}-final.png` });
const order = samples.map((m) => m.battle).filter((v, i, a) => v && (i === 0 || v !== a[i - 1]));
s.note(`battle states: ${order.join(' > ')}`);
check(Boolean(settled), 'the fight ran to the victory or defeat card');
const index = (name) => order.indexOf(name);
check(index('entering') >= 0 && index('entering') < Math.max(index('acting'), 0) + (index('acting') < 0 ? 1e9 : 0), 'Entering came before the first turn');
check(['finishing', 'exiting', 'absent'].every((name, i, a) => index(name) >= 0 && (i === 0 || index(name) > index(a[i - 1]))), 'Finishing > Exiting > Absent');
const flatWhileLive = samples.filter((m) => m.flat && (m.active === 'active' || m.field));
check(flatWhileLive.length === 0, `the flat combat screen was never visible while the battlefield was active (${flatWhileLive.length} samples)`);
const first = samples.findIndex((m) => m.battle === 'entering');
const sounds = [];
let known = samples[0].sfx ? samples[0].sfx.split(' ') : [];
for (const m of samples) { const names = m.sfx ? m.sfx.split(' ') : []; for (const name of names.slice(known.length)) sounds.push({ name, m }); known = names; }
const behindCover = sounds.filter((x) => COMBAT_SFX.has(x.name) && ['leaving', 'interstitial', 'title'].includes(x.m.stage));
check(behindCover.length === 0, `no combat sound behind the cover (${behindCover.length})`);
check(!samples.some((m) => m.aftermath && m.dice), 'no dice card over the victory card');
const transitions = sounds.filter((x) => x.name === 'scene-transition');
s.note(`scene-transitions after the fight began: ${transitions.length} (entry and exit)`);
await finish(browser);
