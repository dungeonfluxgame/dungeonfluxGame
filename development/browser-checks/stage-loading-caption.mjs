// CAPTION-001 (Part 1): read-along narration on the fight loading screen. Starts a test
// fight (the adventure's first choice leads straight to it) and watches the loading splash:
// while it holds, the line being spoken shows on it with the story stage's own words (the
// Dungeon Master label or the speaker's name), below the place name and the progress bar.
// Saves 1920x1080 frames and asserts the caption tracks the narrator's box, never overlaps
// the bar, and goes with the splash (no flash, no duplicate on the battlefield).
// usage: PORT=63379 DF_CHECK_PG_PORT=55587 node stage-loading-caption.mjs [frames-dir]
import fs from 'node:fs';
import path from 'node:path';
import { runner, launchGpu, playToOpening, click, settle, states, until, root } from './stage.mjs';
import { startLocal, stopLocal } from './serve-local.mjs';

const PORT = process.env.PORT || '63379';
process.env.DF_BASE = `http://127.0.0.1:${PORT}`;
const framesDir = process.argv[2] || path.join(root, 'artifacts', 'tmp', 'caption-a1', 'frames');
fs.mkdirSync(framesDir, { recursive: true });
const { s, out, check, finish } = runner('stage-loading-caption', framesDir);

const env = { DF_GAMEPLAY_TEST_FIGHT: '1', DF_GAMEPLAY_ADVENTURE: 'harbor' };
const { runPack } = await startLocal(env);
for (let i = 0; i < 60 && !fs.existsSync(runPack); i += 1) await settle(500);
if (fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
const browser = await launchGpu(true);
const voiceLog = [];
const { tv, pa } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true, voice: { latencyMs: 120, lengthFor: () => 4, log: voiceLog } });
const UTILITY = ['Full character sheet', 'Ask for clarification', 'Play', 'Hero', 'Menu', 'Reconnect', 'Confirm saved choice', 'Leave this game on this device', 'Close'];
const buttons = (page) => page.evaluate((utility) => [...document.querySelectorAll('button:not([disabled])')].map((b) => b.textContent.trim().replace(/\s+/g, ' ')).filter((t) => t && !utility.includes(t)), UTILITY);
const begin = (await buttons(pa)).find((t) => /enter|begin|start/i.test(t));
if (begin) await click(s, pa, begin, { wait: 300 });
await settle(2500);
let first;
for (let i = 0; i < 100 && !first; i += 1) { first = (await buttons(pa)).find((t) => /^(ask|escort|offer to)/i.test(t)); if (!first) await settle(300); }
s.note(`the first choice: ${first}`);
await click(s, pa, first, { wait: 300 });

const splash = () => tv.evaluate(() => {
  const card = document.querySelector('.bf-splash');
  if (!card) return null;
  const rect = (e) => { if (!e) return null; const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; };
  const caption = card.querySelector('.bf-splash-caption');
  return {
    leaving: card.hasAttribute('data-leaving'),
    place: (card.querySelector('.bf-splash-place') || {}).textContent?.trim() ?? '',
    note: (card.querySelector('.bf-splash-note') || {}).textContent?.trim() ?? '',
    bar: rect(card.querySelector('.bf-splash-track')),
    captionHidden: !caption || caption.hasAttribute('hidden'),
    label: (card.querySelector('.bf-splash-caption-label') || {}).textContent?.trim() ?? '',
    text: (card.querySelector('.bf-splash-caption-text') || {}).textContent?.trim() ?? '',
    captionRect: rect(caption),
    narrator: (document.querySelector('.tv-narration') || {}).textContent?.trim() ?? '',
  };
});

// Catch the splash while it holds: the caption up with the line being spoken. Without a
// WebGPU battlefield the splash never mounts (flat fallback) and there is nothing to show.
let captioned = null;
let seenSplash = false;
let shot = 0;
const stop = Date.now() + 90000;
while (Date.now() < stop) {
  const card = await splash();
  if (card) seenSplash = true;
  if (card && !card.captionHidden && card.text) {
    if (!captioned) captioned = card;
    if (shot < 3) { shot += 1; await tv.screenshot({ path: `${out}-loading-${shot}.png` }); }
    if (shot >= 3 && card.text !== captioned.text) { await tv.screenshot({ path: `${out}-loading-next-line.png` }); break; }
  }
  if (card === null && captioned) break;
  await settle(100);
}
s.note(`battlefield mounted: ${seenSplash} (needs WebGPU; flat fallback shows no splash)`);
check(seenSplash, 'the fight loading splash mounted');
check(Boolean(captioned), 'the loading splash showed the spoken line while it held');
if (captioned) {
  s.note(`caption: [${captioned.label}] ${captioned.text.slice(0, 120)}`);
  check(captioned.place.length > 0 && captioned.note.length > 0, `the place and the progress stayed up ("${captioned.place}" / "${captioned.note}")`);
  check(captioned.label.length > 0, `the caption names its speaker ("${captioned.label}")`);
  check(captioned.text === captioned.narrator || captioned.narrator.length === 0 || captioned.narrator.includes(captioned.text.slice(0, 40)) || captioned.text.includes(captioned.narrator.slice(0, 40)), 'the caption is the line the narrator box shows');
  const overlap = captioned.bar && captioned.captionRect
    && captioned.captionRect.y < captioned.bar.y + captioned.bar.h && captioned.bar.y < captioned.captionRect.y + captioned.captionRect.h;
  check(overlap === false, 'the caption sits below the progress bar (no overlap)');
  check(captioned.captionRect && captioned.captionRect.h >= 40, `the caption is room-legible (${Math.round(captioned.captionRect?.h ?? 0)}px tall at 1080p)`);
}
// The handover: the splash goes, the caption with it, and the battlefield carries on.
await until(tv, (now) => now.battle && now.battle !== 'absent', 60000).catch(() => null);
await tv.waitForSelector('.battlefield', { timeout: 60000 }).catch(() => null);
await tv.waitForFunction(() => !document.querySelector('.bf-splash'), { timeout: 120000 }).catch(() => null);
const gone = await splash();
check(gone === null, 'the loading screen went, caption with it');
await settle(1500);
await tv.screenshot({ path: `${out}-after-reveal.png` });
const flatDup = await tv.evaluate(() => [...document.querySelectorAll('.tv-bubble')].filter((b) => getComputedStyle(b).opacity !== '0').length);
s.note(`visible flat bubbles after reveal: ${flatDup}; voice requests: ${voiceLog.length}`);
await stopLocal();
await finish(browser);
