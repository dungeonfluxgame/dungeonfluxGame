// Evaluator tooling only (not application code): the fight has one timeline.
//
// Plays a whole fight with the TV and two phones in headless Edge (WebGPU, so a
// scan battlefield renders) and samples all three pages ten times a second:
// the server's latest beat sequence, the sequence the TV has finished
// presenting, what the TV's turn label and health cards show, and what each
// phone offers. Writes a per-beat table (table.md) and asserts the one-clock
// rules from the numbers alone:
//
//   - the TV's presented sequence only moves forward, one beat at a time, and
//     every resolved beat is presented exactly once (beat log has no duplicate
//     or missing sequence and none marked skipped in fast/paused modes);
//   - a phone never enables Attack / End turn while its own released sequence
//     is behind the latest resolved beat (its turn opens after the TV showed
//     the beats before it);
//   - the phones' released sequence is never more than one beat ahead of the
//     TV's presented sequence for more than a sample or two;
//   - the presented sequence never stalls: no 25 s stretch with unpresented
//     beats and no progress (also with the TV hidden, and with no TV at all);
//   - the ending sting sounds after the last blow's impact sounds.
//
// usage:
//   node development/browser-checks/fight-timeline.mjs <base url> <out dir> [mode]
//   mode: fast (default) | paused (10 s idle per turn) | hidden (TV tab hidden
//   30 s mid-fight) | away (TV page closed mid-fight; phones must finish alone)
// env: DF_PLAYWRIGHT (playwright module path), DF_MAX_MS (fight budget),
//      DF_ROOM (room code to join; default: the lobby's room)
import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.DF_PLAYWRIGHT || 'C:/Users/mreca/Desktop/CashFlux/node_modules/playwright');

const [base, out, modeArg] = process.argv.slice(2);
const MODE = modeArg || 'fast';
if (!base || !out || !['fast', 'paused', 'hidden', 'away'].includes(MODE)) {
  console.error('usage: node fight-timeline.mjs <base url> <out dir> [fast|paused|hidden|away]');
  process.exit(2);
}
fs.mkdirSync(out, { recursive: true });
const started = Date.now();
const lines = [];
const note = (text) => {
  const line = `${(((Date.now() - started) / 1000).toFixed(1)).padStart(7)}s ${text}`;
  lines.push(line);
  console.log(line);
};
let failures = 0;
const check = (ok, text) => {
  if (!ok) failures += 1;
  note(`${ok ? 'ok  ' : 'FAIL'} ${text}`);
  return ok;
};
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
const num = (v) => (v === null || v === undefined || v === '' ? null : Number(v));

const browser = await chromium.launch({
  channel: 'msedge',
  headless: true,
  args: [
    '--enable-unsafe-webgpu',
    '--enable-features=WebGPU',
    '--ignore-gpu-blocklist',
    '--autoplay-policy=no-user-gesture-required',
    '--mute-audio',
  ],
});
const tvContext = await browser.newContext({ viewport: { width: 1280, height: 720 } });
const phoneContext = () => browser.newContext({ viewport: { width: 390, height: 844 } });
const paContext = await phoneContext();
const pbContext = await phoneContext();
const tv = await tvContext.newPage();
const pa = await paContext.newPage();
const pb = await pbContext.newPage();
for (const [tag, page] of [['tv', tv], ['a', pa], ['b', pb]]) {
  page.on('pageerror', (e) => note(`PAGEERROR ${tag}: ${String(e.message).slice(0, 180)}`));
}

await tv.goto(`${base}/gameplay/display`);
await tv.waitForTimeout(1500);
await tv.getByRole('button', { name: 'Enable sound' }).click().catch(() => {});
const room = process.env.DF_ROOM || (await tv.evaluate(() => {
  const match = document.body.innerText.match(/\b[A-Z]{6}\b/);
  return match ? match[0] : '';
}).catch(() => ''));
note(`mode=${MODE} room=${room || '(lobby default)'}`);

const clickButton = (page, label) => page.evaluate((wanted) => {
  const button = [...document.querySelectorAll('main button, button')]
    .find((b) => b.textContent.trim() === wanted && !b.disabled);
  if (button) button.click();
  return Boolean(button);
}, label);

async function makeHero(page, name, picks) {
  await page.goto(room ? `${base}/gameplay/player?room=${room}` : `${base}/gameplay/player`);
  await page.waitForTimeout(1500);
  await clickButton(page, 'Join this room');
  await page.waitForSelector('.identity-panel input', { timeout: 20000 });
  await page.click('.identity-panel input');
  await page.keyboard.type(name, { delay: 20 });
  for (let index = 0; index < picks.length; index += 1) {
    await page.evaluate(([i, pick]) => {
      const group = document.querySelectorAll('section.character-choice-group')[i];
      const button = group && [...group.querySelectorAll('button')]
        .find((b) => b.textContent.trim().startsWith(pick));
      if (button) button.click();
    }, [index, picks[index]]);
    await page.waitForTimeout(120);
  }
  await clickButton(page, 'Confirm your hero');
  note(`${name} confirmed`);
}

await Promise.all([
  makeHero(pa, 'Brakka', ['Dwarf', 'Fighter', 'Soldier', 'Lawful Good', 'Elvish', 'Giant', 'Insight', 'Persuasion',
    'Dice', 'Party agrees', 'Stalwart', 'Greatsword hero']),
  makeHero(pb, 'Sigrun', ['Dwarf', 'Fighter', 'Soldier', 'Neutral Good', 'Elvish', 'Goblin', 'Perception', 'Survival',
    'Playing Cards', 'Party agrees', 'Longbow', 'Woman with a bow']),
]);
await pa.waitForTimeout(2000);

// --- sampling ---------------------------------------------------------------

const tvSample = () => tv.evaluate(() => {
  const stage = document.querySelector('.tv-stage') || document.querySelector('[data-presented-seq]');
  const attr = (el, name) => (el ? el.getAttribute(name) : null);
  const text = (el) => (el ? el.textContent.trim().replace(/\s+/g, ' ') : '');
  const plates = [...document.querySelectorAll('.bf-plate')].map((plate) => ({
    name: text(plate.querySelector('.bf-plate-name')),
    hp: text(plate.querySelector('.bf-plate-hp')),
    down: plate.getAttribute('data-down') === 'true',
    active: plate.getAttribute('data-active') === 'true',
  }));
  const dice = [...document.querySelectorAll('video, .dice-clip, [data-dice]')].map((el) => ({
    tag: el.tagName, cls: String(el.className || '').slice(0, 40),
    src: (el.currentSrc || el.getAttribute('src') || '').split('/').pop(),
  }));
  return {
    t: Date.now(), hidden: document.hidden,
    presented: attr(stage, 'data-presented-seq'),
    latest: attr(stage, 'data-latest-seq'),
    beatlog: attr(stage, 'data-beat-log') || '',
    battle: attr(stage, 'data-battle-state'),
    bflive: attr(stage, 'data-battlefield-live'),
    report: document.querySelector('[data-presented-report]')?.getAttribute('data-presented-report') ?? null,
    instance: attr(stage, 'data-display-instance'),
    turn: text(document.querySelector('.bf-turn')),
    round: text(document.querySelector('.bf-round')),
    title: text(document.querySelector('.bf-title')),
    plates,
    chips: [...document.querySelectorAll('.bf-chip')].map((chip) => ({
      label: text(chip), down: chip.hasAttribute('data-down'), active: chip.hasAttribute('data-active'),
    })),
    roll: text(document.querySelector('.bf-roll')),
    finishOpen: Boolean(document.querySelector('.bf-hud[data-finish]')),
    dice,
    sfx: document.documentElement.getAttribute('data-sfx-log') || '',
    screen: document.querySelector('.live-screen')?.getAttribute('data-screen') ?? null,
    bodyHead: document.body.innerText.slice(0, 200).replace(/\n+/g, ' | '),
  };
}).catch(() => null);

const phoneSample = (page) => page.evaluate(() => {
  const root = document.querySelector('[data-presented-seq]');
  const main = document.querySelector('main');
  const text = main ? main.innerText : '';
  const round = (text.match(/ROUND[^\n]*/) || [''])[0];
  return {
    released: root?.getAttribute('data-presented-seq') ?? null,
    latest: root?.getAttribute('data-latest-seq') ?? null,
    yourTurn: /YOUR TURN/.test(round),
    onTv: /On the TV now/.test(text),
    round,
    buttons: [...document.querySelectorAll('main button')]
      .map((b) => ({ label: b.textContent.trim().replace(/\s+/g, ' ').slice(0, 48), disabled: b.disabled }))
      .filter((b) => b.label),
    offers: [...document.querySelectorAll('main button')]
      .filter((b) => !b.disabled)
      .map((b) => b.textContent.trim())
      .filter((x) => x && x.length > 3 && x.length < 90),
    end: /THE END/.test(text),
    outcome: (text.slice(0, 500).match(/VICTORY|DEFEAT/) || [''])[0],
    fight: /ROUND \d/.test(text),
  };
}).catch(() => null);

// One hero turn, as fast as the phone allows (from play-and-record.mjs).
const heroTurn = (page, prefer) => page.evaluate(async (wanted) => {
  const nap = (ms) => new Promise((done) => setTimeout(done, ms));
  const main = () => document.querySelector('main');
  const button = (label) => [...document.querySelectorAll('main button')]
    .find((b) => b.textContent.trim() === label && !b.disabled);
  const select = (pattern) => [...document.querySelectorAll('main select')]
    .find((s) => [...s.options].some((o) => pattern.test(o.value)));
  const set = (element, value) => {
    element.value = value;
    element.dispatchEvent(new Event('input', { bubbles: true }));
    element.dispatchEvent(new Event('change', { bubbles: true }));
  };
  const log = [];
  const reaction = button('Opportunity attack');
  if (reaction) { reaction.click(); log.push('opportunity attack'); await nap(800); return log.join(', '); }
  const round = () => (main().innerText.match(/ROUND[^\n]*/) || [''])[0];
  if (!/YOUR TURN/.test(round())) return '';
  const myHp = () => { const m = main().innerText.match(/(\d+) \/ (\d+) HP/); return m ? Number(m[1]) : 99; };
  const wind = myHp() <= 6 && button('Second Wind');
  if (wind) { wind.click(); log.push('second wind'); await nap(900); }
  const pattern = new RegExp(wanted);
  const options = () => { const s = select(/-action$/); return s ? [...s.options].filter((o) => /-action$/.test(o.value)) : []; };
  const preferred = () => options().filter((o) => pattern.test(o.textContent));
  if (!preferred().length && wanted === 'Greatsword') {
    const text = main().innerText;
    const foes = [...text.matchAll(/\n[A-Z][A-Za-z ]+\d\n\n[^\n]*\n\n(\d+) \/ \d+ HP\nCell\nColumn (\d+), row (\d+)/g)]
      .filter((m) => Number(m[1]) > 0).map((m) => [Number(m[2]) - 1, Number(m[3]) - 1]);
    const cells = select(/^\d+-\d+$/);
    if (cells && button('Move') && foes.length) {
      const values = [...cells.options].map((o) => o.value).filter((v) => /^\d+-\d+$/.test(v));
      const distance = (v) => { const [x, y] = v.split('-').map(Number); return Math.min(...foes.map((f) => Math.max(Math.abs(f[0] - x), Math.abs(f[1] - y)))); };
      values.sort((a, b) => distance(a) - distance(b));
      if (values[0]) set(cells, values[0]);
      await nap(150);
      const move = button('Move');
      if (move) { move.click(); log.push('move'); }
      for (let i = 0; i < 24 && !preferred().length; i += 1) await nap(500);
      if (!preferred().length) log.push('greatsword never offered after move');
    }
  }
  const pool = preferred().length ? preferred() : options();
  if (pool.length && button('Attack')) {
    const pick = pool[pool.length - 1];
    set(select(/-action$/), pick.value);
    await nap(150);
    button('Attack').click();
    log.push('attack ' + pick.textContent.trim().split(' ·')[0]);
    await nap(1200);
  }
  for (let i = 0; i < 20 && !button('End turn') && /ROUND/.test(round()); i += 1) await nap(300);
  const end = button('End turn');
  if (end) { end.click(); log.push('end'); }
  await nap(300);
  return log.join(', ');
}, prefer).catch(() => '');

// --- walk the story to the fight --------------------------------------------

const SKIP = ['Say it', 'Tap to talk', 'Ask for clarification', 'Decline this choice', 'Confirm offered choice',
  'Full character sheet', 'Play', 'Hero', 'Menu', 'Reconnect', 'Confirm saved choice',
  'Leave this game on this device', 'Close', 'Move', 'Attack', 'Dash', 'Dodge', 'Disengage', 'Second Wind',
  'End turn', 'Opportunity attack', 'Decline opportunity attack', 'Join this room', 'Confirm original strike',
  'Confirm your hero'];
const storyPick = (sample) => sample.offers.find((o) => !SKIP.includes(o) && !/^Show choices/.test(o) && !/^Short Rest/.test(o))
  || sample.offers.find((o) => !SKIP.includes(o));

async function toFight() {
  const deadline = Date.now() + 240000;
  while (Date.now() < deadline) {
    const a = await phoneSample(pa);
    if (a && a.fight) return true;
    const tvNow = await tvSample();
    if (tvNow && tvNow.battle && tvNow.battle !== 'absent') return true;
    if (a) {
      const pick = storyPick(a);
      if (pick) {
        note(`choose: ${pick}`);
        await clickButton(pa, pick);
        const before = JSON.stringify(a.offers);
        const pressed = Date.now();
        for (;;) {
          await sleep(300);
          const now = await phoneSample(pa);
          if (!now || now.fight || now.end || JSON.stringify(now.offers.filter((o) => !SKIP.includes(o))) !== before) break;
          if (Date.now() - pressed > 30000) { note(`no change 30 s after "${pick}"`); break; }
        }
        continue;
      }
    }
    await sleep(600);
  }
  return false;
}

check(await toFight(), 'the story reached the fight');
const fightStart = Date.now();
note('fight started; sampling at 10 Hz');

// --- the fight loop -----------------------------------------------------------

const samples = [];
const table = [];
let lastPresented = -1;
let lastProgressAt = Date.now();
let lastProgressKey = '';
let actions = 0;
let lastActionAt = 0;
let frames = 0;
let ended = false;
let tvGone = false;
let blankPage = null;
let hiddenAt = 0;
let shownAt = 0;
const budget = Number(process.env.DF_MAX_MS || 480000);
const stopAt = Date.now() + budget;

while (Date.now() < stopAt) {
  const now = tvGone ? null : await tvSample();
  const a = await phoneSample(pa);
  const b = await phoneSample(pb);
  const row = {
    t: Date.now(),
    dt: Date.now() - fightStart,
    tv: now,
    presentedN: now ? num(now.presented) : null,
    latestN: now ? num(now.latest) : null,
    aRel: a ? num(a.released) : null, aLat: a ? num(a.latest) : null,
    bRel: b ? num(b.released) : null, bLat: b ? num(b.latest) : null,
    a, b,
  };
  samples.push(row);

  const progressKey = `${row.presentedN ?? ''}|${Math.max(row.aRel ?? 0, row.bRel ?? 0)}|${Math.max(row.latestN ?? 0, row.aLat ?? 0, row.bLat ?? 0)}`;
  if (progressKey !== lastProgressKey) { lastProgressKey = progressKey; lastProgressAt = Date.now(); }

  if (row.presentedN !== null && row.presentedN !== lastPresented) {
    const phoneOffers = (sample) => (!sample ? '(gone)'
      : sample.buttons.filter((x) => !x.disabled).map((x) => x.label).slice(0, 6).join(', ') || '(none)');
    table.push({
      seq: row.presentedN,
      latest: row.latestN,
      atMs: row.dt,
      turn: now ? `${now.round} ${now.turn}`.trim() : '',
      plates: now ? now.plates.map((p) => `${p.name} ${p.hp}${p.down ? ' DOWN' : ''}`).join('; ') : '',
      chips: now ? now.chips.map((c) => `${c.label}${c.active ? '*' : ''}${c.down ? '(down)' : ''}`).join(' ') : '',
      roll: now ? now.roll : '',
      phoneA: phoneOffers(a),
      phoneB: phoneOffers(b),
    });
    if (frames < 80 && now && !now.hidden) {
      await tv.screenshot({ path: path.join(out, `beat-${String(frames).padStart(2, '0')}-seq${row.presentedN}.jpg`), type: 'jpeg', quality: 55 }).catch(() => {});
      frames += 1;
    }
    lastPresented = row.presentedN;
  }

  if (MODE === 'hidden' && actions >= 2 && !hiddenAt) {
    blankPage = await tv.context().newPage();
    await blankPage.goto('about:blank');
    await blankPage.bringToFront();
    hiddenAt = Date.now();
    note(`TV hidden (presented ${row.presentedN}, latest ${row.latestN})`);
  }
  if (MODE === 'hidden' && hiddenAt && !shownAt && Date.now() - hiddenAt >= 30000) {
    await tv.bringToFront();
    shownAt = Date.now();
    note(`TV shown again (presented ${row.presentedN}, latest ${row.latestN})`);
  }
  if (MODE === 'away' && actions >= 2 && !tvGone) {
    note(`closing the TV page (presented ${row.presentedN}, latest ${row.latestN})`);
    tvGone = true;
    await tv.close().catch(() => {});
  }

  const fightOver = (a && (a.end || a.outcome) && !a.fight) || (b && (b.end || b.outcome) && !b.fight)
    || (now && now.screen === 'aftermath' && row.presentedN !== null && row.presentedN >= (row.latestN ?? 0) && (row.latestN ?? 0) > 0);
  if (fightOver) { ended = true; break; }

  // After the fight the phones show aftermath choices: take the first so the
  // run can also reach THE END instead of idling here.
  const pauseDue = MODE === 'paused' ? 10000 : 300;
  if (Date.now() - lastActionAt >= pauseDue) {
    let did = '';
    if (a && a.fight) did = await heroTurn(pa, 'Greatsword');
    if (did) note(`Brakka: ${did}`);
    let didB = '';
    if (b && b.fight) didB = await heroTurn(pb, 'Longbow');
    if (didB) note(`Sigrun: ${didB}`);
    if (did || didB) { actions += 1; lastActionAt = Date.now(); }
  }
  await sleep(100);
}

await sleep(3000);
const finalTv = tvGone ? null : await tvSample();
const finalA = await phoneSample(pa);
const finalB = await phoneSample(pb);
if (!tvGone) await tv.screenshot({ path: path.join(out, 'final.png') }).catch(() => {});
fs.writeFileSync(path.join(out, 'samples.json'), JSON.stringify(samples, null, 1));

// --- the per-beat table --------------------------------------------------------

const tableHeader = '| seq | latest | presented at (ms) | TV turn label | TV health cards | TV strip | phone A offers | phone B offers |\n|---|---|---|---|---|---|---|---|';
const tableRows = table.map((r) => `| ${r.seq} | ${r.latest ?? ''} | ${r.atMs} | ${r.turn} | ${r.plates} | ${r.chips} | ${r.phoneA} | ${r.phoneB} |`);
fs.writeFileSync(path.join(out, 'table.md'), `# Fight timeline (${MODE})\n\n${tableHeader}\n${tableRows.join('\n')}\n`);
note(`per-beat table: ${table.length} presented steps (table.md)`);
for (const line of [tableHeader, ...tableRows]) console.log(line);

// --- assertions from the numbers ----------------------------------------------

check(ended || Boolean((finalA && finalA.outcome) || (finalB && finalB.outcome) || (finalTv && finalTv.screen === 'aftermath')),
  `the fight ran to its ending (screen=${finalTv?.screen}, outcomeA=${finalA?.outcome}, outcomeB=${finalB?.outcome})`);
if (MODE !== 'away') {
  check(samples.some((r) => r.tv && r.tv.report === 'sent'),
    'the display reported presented sequences (data-presented-report=sent seen)');
}
const inFight = samples.filter((r) => r.latestN !== null && r.latestN > 0 && r.presentedN !== null);
let backwards = 0;
let jumps = 0;
let prev = null;
for (const r of inFight) {
  if (prev !== null) {
    if (r.presentedN < prev && r.presentedN !== 0) backwards += 1;
    if (r.presentedN > prev + 1) jumps += 1;
  }
  if (r.presentedN !== 0 || prev === null || prev === 0) prev = r.presentedN;
  if (r.presentedN === 0 && prev > 0) prev = 0;
}
check(backwards === 0, `the TV presented sequence never went backward (${backwards} samples)`);
note(`presented jumps of more than one beat between samples: ${jumps} (coherent catch-up may jump; the beat log decides)`);

let phoneAheadSamples = 0;
let phoneAheadSince = null;
let phoneAheadMaxMs = 0;
let offerViolations = 0;
let yourTurnWhileBehind = 0;
for (const r of samples) {
  for (const [sample, rel, lat] of [[r.a, r.aRel, r.aLat], [r.b, r.bRel, r.bLat]]) {
    if (!sample || rel === null || lat === null || lat === 0) continue;
    const actionEnabled = sample.buttons.some((x) => !x.disabled && /^(Attack|End turn|Move|Dash|Dodge|Disengage)$/.test(x.label));
    if (actionEnabled && rel < lat) offerViolations += 1;
    if (sample.yourTurn && rel < lat) yourTurnWhileBehind += 1;
  }
  if (r.presentedN === null) continue;
  const rel = Math.max(r.aRel ?? 0, r.bRel ?? 0);
  if (rel > r.presentedN + 1) {
    phoneAheadSamples += 1;
    if (phoneAheadSince === null) phoneAheadSince = r.t;
    phoneAheadMaxMs = Math.max(phoneAheadMaxMs, r.t - phoneAheadSince);
  } else {
    phoneAheadSince = null;
  }
}
check(offerViolations === 0, `no phone enabled a turn action while its released sequence lagged the latest beat (${offerViolations} samples)`);
check(yourTurnWhileBehind === 0, `no phone said YOUR TURN while beats were unreleased (${yourTurnWhileBehind} samples)`);
check(phoneAheadMaxMs <= 2000, `phones never more than one beat ahead of the TV for over 2 s (max ${phoneAheadMaxMs} ms in ${phoneAheadSamples} samples)`);

const beatlogText = finalTv ? finalTv.beatlog : (samples.filter((r) => r.tv).at(-1)?.tv.beatlog || '');
const entries = beatlogText.split(',').filter(Boolean).map((e) => {
  const [seq, kind, startMs, endMs] = e.split(':');
  return { seq: Number(seq), kind, start: Number(startMs), end: Number(endMs) };
});
const latestFinal = Math.max(0, ...samples.map((r) => Math.max(r.latestN ?? 0, r.aLat ?? 0, r.bLat ?? 0)));
const terminal = entries.filter((e) => ['impact', 'defeat', 'healing', 'skipped'].includes(e.kind) && e.seq > 0);
const terminalSeqs = terminal.map((e) => e.seq);
const dup = terminalSeqs.filter((s, i) => terminalSeqs.indexOf(s) !== i);
check(dup.length === 0, `no beat presented twice (${terminal.length} terminal phases, duplicates: ${dup.join(',') || 'none'})`);
const inOrder = terminalSeqs.every((s, i) => i === 0 || s >= terminalSeqs[i - 1]);
check(inOrder, 'beats presented in sequence order in the beat log');
const presentedSeqs = new Set(entries.filter((e) => e.seq > 0).map((e) => e.seq));
const missing = [];
for (let s = 1; s <= latestFinal; s += 1) if (!presentedSeqs.has(s)) missing.push(s);
const skipped = entries.filter((e) => e.kind === 'skipped').map((e) => e.seq);
note(`beat log: latest=${latestFinal}, entries=${entries.length}, missing=[${missing.join(',')}], skipped=[${skipped.join(',')}]`);
note(`beat log detail: ${entries.map((e) => `${e.seq}:${e.kind}:${e.end - e.start}ms`).join(' ')}`);
if (MODE === 'fast' || MODE === 'paused') {
  check(skipped.length === 0, `no beat skipped on the TV (${skipped.length}: ${skipped.join(',')})`);
}
check(missing.length === 0 || MODE === 'away', `every resolved beat appears in the TV beat log (missing: ${missing.join(',') || 'none'})`);

const stalled = samples.length > 0 && !ended && (Date.now() - lastProgressAt > 25000);
check(!stalled, `no 25 s stall with unpresented beats (last progress ${Math.round((Date.now() - lastProgressAt) / 1000)} s ago)`);
if (MODE === 'hidden') {
  const caught = samples.find((r) => shownAt && r.t >= shownAt && r.presentedN !== null && r.latestN !== null && r.presentedN >= r.latestN && r.latestN > 0);
  check(Boolean(caught), 'after 30 s hidden the TV caught up to the newest beat');
}
if (MODE === 'away') {
  check(actions > 2, `the phones kept playing with no TV connected (${actions} hero turns)`);
}

const sfx = (finalTv ? finalTv.sfx : (samples.filter((r) => r.tv).at(-1)?.tv.sfx || '')).split(' ').filter(Boolean);
const stingAt = Math.max(sfx.lastIndexOf('victory-fanfare'), sfx.lastIndexOf('defeat-toll'));
const transitionAt = sfx.lastIndexOf('scene-transition');
const lastImpactSfx = Math.max(...['arrow-impact', 'hit', 'miss', 'critical', 'body-fall', 'blade-hit'].map((n) => sfx.lastIndexOf(n)));
note(`sfx tail: ${sfx.slice(-18).join(' ')}`);
if (stingAt >= 0 && lastImpactSfx >= 0) {
  check(stingAt > lastImpactSfx, `the ending sting follows the last blow (sting@${stingAt}, last impact@${lastImpactSfx})`);
}
if (transitionAt >= 0 && lastImpactSfx >= 0) {
  check(transitionAt > lastImpactSfx, `the scene transition follows the last blow (transition@${transitionAt}, last impact@${lastImpactSfx})`);
}

const fightMs = (samples.at(-1)?.dt ?? 0);
note(`fight wall time ${Math.round(fightMs / 1000)} s, hero turns driven ${actions}, samples ${samples.length}`);
fs.writeFileSync(path.join(out, 'log.txt'), lines.join('\n') + '\n');
await tvContext.close().catch(() => {});
await paContext.close().catch(() => {});
await pbContext.close().catch(() => {});
await browser.close();
note(failures ? `${failures} FAILED` : 'ALL PASSED');
process.exit(failures ? 1 : 0);


