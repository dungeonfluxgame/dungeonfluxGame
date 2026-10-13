// Whole-game browser run: creates two heroes, follows story offers and fight turns, and records TV/phone video.
// usage: DF_PLAYWRIGHT=<node_modules/playwright> node play-and-record.mjs --base <url> --room <room> --out <dir> [--story-hold-ms <ms>] [--no-video]
// Playwright is loaded from DF_PLAYWRIGHT, matching lib.mjs; the Edge channel is selected by Playwright.
import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';

const require = createRequire(import.meta.url);
if (!process.env.DF_PLAYWRIGHT) {
  console.error('DF_PLAYWRIGHT must point to the Playwright package used by the browser checks.');
  process.exit(2);
}
const { chromium } = require(process.env.DF_PLAYWRIGHT);

const options = { base: process.env.DF_BASE, room: process.env.DF_ROOM, out: process.env.DF_OUT,
  storyHoldMs: Number(process.env.STORY_HOLD_MS || 0), noVideo: process.env.NO_VIDEO === '1' };
for (let index = 2; index < process.argv.length; index += 1) {
  const argument = process.argv[index];
  if (argument === '--no-video') { options.noVideo = true; continue; }
  const key = ({ '--base': 'base', '--room': 'room', '--out': 'out', '--story-hold-ms': 'storyHoldMs' })[argument];
  if (!key || !process.argv[index + 1]) {
    console.error('usage: node play-and-record.mjs --base <url> --room <room> --out <dir> [--story-hold-ms <ms>] [--no-video]');
    process.exit(2);
  }
  options[key] = key === 'storyHoldMs' ? Number(process.argv[++index]) : process.argv[++index];
}
const { base, room, out, storyHoldMs, noVideo } = options;
if (!base || !room || !out || !Number.isFinite(storyHoldMs) || storyHoldMs < 0) {
  console.error('base, room, out and a non-negative story hold time are required.');
  process.exit(2);
}
fs.mkdirSync(out, { recursive: true });
if (!noVideo) {
  fs.mkdirSync(path.join(out, 'video-tv'), { recursive: true });
  fs.mkdirSync(path.join(out, 'video-phone'), { recursive: true });
}
const started = Date.now();
const note = (message) => {
  const line = `${((Date.now() - started) / 1000).toFixed(1).padStart(6)}s ${message}`;
  console.log(line);
  fs.appendFileSync(path.join(out, 'run.log'), `${line}\n`);
};
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: [
  '--enable-unsafe-webgpu', '--enable-features=WebGPU', '--ignore-gpu-blocklist',
  '--autoplay-policy=no-user-gesture-required', '--mute-audio',
] });
const contextOptions = (width, height, videoDir) => ({ viewport: { width, height },
  ...(videoDir ? { recordVideo: { dir: videoDir, size: { width, height } } } : {}) });
const tvContext = await browser.newContext(contextOptions(1280, 720, noVideo ? null : path.join(out, 'video-tv')));
const phoneContextA = await browser.newContext(contextOptions(390, 844, noVideo ? null : path.join(out, 'video-phone')));
const phoneContextB = await browser.newContext(contextOptions(390, 844, null));
const tv = await tvContext.newPage();
const pa = await phoneContextA.newPage();
const pb = await phoneContextB.newPage();
let lost = false;
for (const page of [tv, pa, pb]) page.on('console', (message) => {
  if (message.type() === 'error' && message.text().includes('The connection to the game was lost')) lost = true;
});
const checkLost = async () => {
  for (const page of [tv, pa, pb]) {
    if ((await page.locator('body').innerText()).includes('The connection to the game was lost')) lost = true;
  }
  if (lost) throw new Error('A page showed "The connection to the game was lost".');
};
const state = (page) => page.evaluate(() => {
  const text = document.querySelector('main')?.innerText || '';
  const offers = [...document.querySelectorAll('main [data-exploration-offer]:not(:disabled)')]
    .map((button) => button.textContent.trim()).filter((label) => label.length > 3 && label.length < 90);
  const round = (text.match(/ROUND[^\n]*/i) || [''])[0];
  return { offers, round, fight: /ROUND \d/i.test(text), end: /THE END/i.test(text),
    outcome: (text.slice(0, 500).match(/VICTORY|DEFEAT/i) || [''])[0], head: text.slice(0, 160) };
});
// A button is looked up once per click, so a changed offer is never accidentally clicked twice.
const clickButton = (page, label) => page.evaluate((wanted) => {
  const button = [...document.querySelectorAll('main button')]
    .find((candidate) => candidate.textContent.trim() === wanted && !candidate.disabled);
  if (button) button.click();
  return Boolean(button);
}, label);
const clickStoryChoice = (page, label) => page.evaluate((wanted) => {
  const button = [...document.querySelectorAll('main [data-exploration-offer]')]
    .find((candidate) => candidate.textContent.trim() === wanted && !candidate.disabled);
  if (button) button.click();
  return Boolean(button);
}, label);
async function makeHero(page, name, picks) {
  await page.goto(`${base}/gameplay/player?room=${encodeURIComponent(room)}`);
  await page.waitForTimeout(1500);
  await clickButton(page, 'Join this room');
  await page.waitForSelector('.identity-panel input', { timeout: 15000 });
  await page.click('.identity-panel input');
  await page.keyboard.type(name, { delay: 20 });
  for (let index = 0; index < picks.length; index += 1) {
    await page.evaluate(([choiceIndex, pick]) => {
      const group = document.querySelectorAll('section.character-choice-group')[choiceIndex];
      const button = group && [...group.querySelectorAll('button')].find((item) => item.textContent.trim().startsWith(pick));
      if (button) button.click();
    }, [index, picks[index]]);
    await page.waitForTimeout(120);
  }
  await clickButton(page, 'Confirm your hero');
  note(`${name} confirmed`);
}
async function heroTurn(page, prefer) {
  return page.evaluate(async (wanted) => {
    const sleepInPage = (ms) => new Promise((done) => setTimeout(done, ms));
    const main = () => document.querySelector('main');
    const button = (label) => [...document.querySelectorAll('.combat-action-rail button')].find((item) => item.textContent.trim() === label && !item.disabled);
    const round = () => (main().innerText.match(/ROUND[^\n]*/i) || [''])[0];
    const reaction = [...document.querySelectorAll('.combat-action-rail button')]
      .find((item) => item.textContent.trim().startsWith('Opportunity attack') && !item.disabled);
    if (reaction) { reaction.click(); await sleepInPage(800); return reaction.textContent.trim(); }
    if (!/YOUR TURN/i.test(round())) return '';
    const attack = button('Attack');
    if (attack) { attack.click(); await sleepInPage(1200); }
    for (let index = 0; index < 20 && !button('End turn') && /ROUND/i.test(round()); index += 1) await sleepInPage(300);
    const end = button('End turn');
    if (end) { end.click(); await sleepInPage(400); return `${attack ? 'attack' : ''} ${wanted}, end`.trim(); }
    return attack ? `attack ${wanted}` : '';
  }, prefer);
}
let reachedEnd = false;
try {
  await tv.goto(`${base}/gameplay/display`);
  await Promise.all([makeHero(pa, 'Brakka', ['Dwarf', 'Fighter', 'Soldier', 'Lawful Good', 'Elvish', 'Giant', 'Insight', 'Persuasion', 'Dice', 'Party agrees', 'Greatsword and Defense', 'Greatsword hero']),
    makeHero(pb, 'Sigrun', ['Dwarf', 'Fighter', 'Soldier', 'Neutral Good', 'Elvish', 'Goblin', 'Perception', 'Survival', 'Playing Cards', 'Party agrees', 'Longbow and Archery', 'Woman with a bow'])]);
  await pa.waitForTimeout(2500);
  const first = (await state(pa)).offers[0];
  if (!first) throw new Error('No opening story choice was offered.');
  await clickStoryChoice(pa, first);
  let lastHead = '';
  let idle = 0;
  const deadline = Date.now() + 9 * 60 * 1000;
  while (Date.now() < deadline) {
    await checkLost();
    const current = await state(pa);
    if (current.end) { reachedEnd = true; break; }
    if (current.fight) {
      for (const [page, name, weapon] of [[pa, 'Brakka', 'Greatsword'], [pb, 'Sigrun', 'Longbow']]) {
        const result = await heroTurn(page, weapon);
        if (result) note(`${name}: ${result}`);
      }
      continue;
    }
    const pick = current.offers.find((offer) => !/^Short Rest/.test(offer)) || current.offers[0];
    if (pick) {
      if (storyHoldMs > 0 && !current.outcome) await sleep(storyHoldMs);
      note(`choose: ${pick}`);
      await clickStoryChoice(pa, pick);
      // Press each choice once, then wait until the offers change before considering another choice.
      const before = JSON.stringify(current.offers);
      const pressed = Date.now();
      for (;;) {
        await sleep(300);
        await checkLost();
        const updated = await state(pa);
        if (updated.fight || updated.end || JSON.stringify(updated.offers) !== before) break;
        if (Date.now() - pressed > 25000) throw new Error(`No offer change 25 seconds after "${pick}".`);
      }
      idle = 0;
    } else {
      if (current.head !== lastHead) { note(`waiting: ${current.head.slice(0, 90)}`); lastHead = current.head; }
      await sleep(500);
      idle += 1;
      if (idle > 240) throw new Error('No story offers appeared for two minutes.');
    }
  }
  await checkLost();
  if (!reachedEnd && (await state(pa)).end) reachedEnd = true;
  if (!reachedEnd) throw new Error('The game did not reach the end card before the deadline.');
  note('THE END reached on the phone');
  await sleep(12000);
  await checkLost();
  note('end card check passed');
} catch (error) {
  note(`FAIL ${error.message}`);
  process.exitCode = 1;
} finally {
  await tvContext.close();
  await phoneContextA.close();
  await phoneContextB.close();
  await browser.close();
}
