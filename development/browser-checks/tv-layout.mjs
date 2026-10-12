// PLAY-V14 (evaluator tooling, not application code): drives games through the shared display's screens and
// runs the layout check (layout-check.mjs) at each, at 1280x720, 1920x1080 and 3840x2160. One command:
//   development/browser-checks/tv-layout.sh [frames-dir]
// Each scenario is a game of its own (a fresh room and database). Every state it reaches is a checkpoint;
// a state it cannot reach is printed as NOT REACHED and counted, never skipped silently. Violations print
// with both elements' text, and a frame with the offending boxes outlined is saved beside the log.
// Environment: DF_BASE (server), DF_FRAMES (frames dir), ONLY=<scenario,...>, VIEWPORTS=1920x1080,...
import fs from 'node:fs';
import path from 'node:path';
import { runner, startServer, launchGpu, playToOpening, click, create, settle, until, states, framesDir, artifactRoot, root, packPath } from './stage.mjs';
import { measure, outlined, describeViolation, VIEWPORTS } from './layout-check.mjs';
import { makeHeroes, playUntil } from './chapter-lib.mjs';
import { watch } from './lib.mjs';

const only = process.env.ONLY ? process.env.ONLY.split(',') : null;
const viewports = process.env.VIEWPORTS
  ? process.env.VIEWPORTS.split(',').map((v) => v.split('x').map(Number))
  : VIEWPORTS;
const { s, out, check, finish } = runner('tv-layout', process.argv[2] || framesDir);
fs.mkdirSync(out, { recursive: true });
const results = []; // { state, size, violations }
const unreached = [];

const slug = (text) => text.replace(/[^a-z0-9]+/gi, '-').toLowerCase();

// One state of one screen at every viewport: resize, let the layout and entrance animations settle, measure.
// With `watchMs` the state is a moment of a timeline (a bubble opens, a card comes up): it is sampled every 1.5 s for
// that long at each viewport and every distinct violation seen is kept; the frame is taken when a new one appears.
async function checkpoint(tv, state, { watchMs = 0 } = {}) {
  const [startW, startH] = [tv.viewportSize().width, tv.viewportSize().height];
  for (const [w, h] of viewports) {
    await tv.setViewportSize({ width: w, height: h });
    await settle(900);
    const found = new Map();
    let lines = 0;
    const until = Date.now() + watchMs;
    let frames = 0;
    do {
      const sample = await measure(tv);
      lines = Math.max(lines, sample.lines);
      if (process.env.DF_DEBUG) s.note(`  boxes: ${sample.seen.join(' | ')}`);
      const fresh = sample.violations.filter((v) => !found.has(`${v.kind}|${v.a.name}|${v.a.text}|${v.b ? `${v.b.name}|${v.b.text}` : ''}`));
      for (const v of fresh) found.set(`${v.kind}|${v.a.name}|${v.a.text}|${v.b ? `${v.b.name}|${v.b.text}` : ''}`, v);
      if (fresh.length) { frames += 1; await outlined(tv, fresh, path.join(out, `${slug(state)}-${w}x${h}${frames > 1 ? `-${frames}` : ''}.png`)); }
      else if (!found.size && w === 1920 && !frames) await tv.screenshot({ path: path.join(out, `${slug(state)}-${w}x${h}.png`) });
      if (Date.now() < until) await settle(1100);
    } while (Date.now() < until);
    const violations = [...found.values()];
    results.push({ state, size: `${w}x${h}`, violations });
    s.note(`${violations.length ? 'VIOL' : 'ok  '} ${state} @${w}x${h}: ${lines} text boxes, ${violations.length} violations`);
    for (const v of violations) s.note(`       ${describeViolation(v)}`);
  }
  await tv.setViewportSize({ width: startW, height: startH });
  await settle(500);
}

const reached = (state, ok, why = '') => {
  if (!ok) { unreached.push(state); s.note(`NOT REACHED ${state}${why ? `: ${why}` : ''}`); }
  return ok;
};

const wanted = (name) => !only || only.includes(name);
const buttons = (page) => page.evaluate(() => [...document.querySelectorAll('button:not([disabled])')].map((b) => b.textContent.trim().replace(/\s+/g, ' ')).filter(Boolean));

// ---------------------------------------------------------------------------------------------
// lobby: the home screen with no heroes, two and six
async function lobby() {
  await startServer({});
  const browser = await launchGpu(false);
  const tv = await (await browser.newContext({ viewport: { width: 1920, height: 1080 } })).newPage();
  await tv.goto(`${process.env.DF_BASE}/gameplay/display`);
  await settle(4000);
  await checkpoint(tv, 'home with 0 heroes');
  const names = ['Brakka', 'Thrain'];
  for (const [index, name] of names.entries()) {
    const phone = await (await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true })).newPage();
    await phone.goto(`${process.env.DF_BASE}/gameplay/player`);
    await settle(1500);
    await create(s, phone, name, `Hero number ${index + 1}`);
    await click(s, phone, 'Confirm your hero');
  }
  await settle(2500);
  await checkpoint(tv, 'home with 2 heroes');
  // The server's room takes two players, so six cards cannot be reached by playing. The display draws up to six
  // (PARTY_MAX); this stands six ready cards in the page itself, with the text the display would give them.
  await tv.evaluate(() => {
    document.querySelector('.tv-roster').setAttribute('data-count', '6');
    const extra = ['Sylvie', 'Osric', 'Mira', 'Dunn'];
    [...document.querySelectorAll('.tv-roster .tv-card')].forEach((card, index) => {
      card.removeAttribute('hidden');
      if (index < 2) return;
      card.removeAttribute('data-empty');
      card.setAttribute('data-state', 'ready');
      card.setAttribute('data-ready', 'true');
      card.querySelector('.tv-member-name').textContent = extra[index - 2];
      card.querySelector('.tv-member-status').textContent = 'Ready';
      card.querySelector('.tv-portrait-glyph').textContent = extra[index - 2][0];
    });
  });
  await settle(800);
  await checkpoint(tv, 'home with 6 heroes (stand-in cards: the room takes two players)');
  await browser.close();
}

// ---------------------------------------------------------------------------------------------
// The scene has stopped changing: no card or transition is up and the table is asked to choose.
const calm = (now) => !now.stage && now.scene === 'awaiting-choice' && now.chipsVisible;
const choose = async (tv, phone, text) => {
  const list = await buttons(phone);
  s.note(`phone offers: ${JSON.stringify(list.slice(0, 8))}`);
  const ok = await click(s, phone, text, { wait: 100 });
  if (ok) {
    await until(tv, (now) => now.stage || now.scene !== 'awaiting-choice' || now.screen === 'combat', 6000).catch(() => null);
    await until(tv, calm, 90000).catch(() => null);
    await settle(1200);
  }
  return ok;
};

// A hero's turn is an Attack (then its first target) or ending the turn; repeated until the aftermath screen is up.
async function fightOut(tv, pa, pb, limitMs = 240000) {
  const stop = Date.now() + limitMs;
  let round = 0;
  while (Date.now() < stop) {
    round += 1;
    const done = await tv.evaluate(() => Boolean(document.querySelector('.tv-stage .live-screen[data-screen=aftermath]')));
    if (done) return true;
    const tap = (page, text, wait) => click(s, page, text, { optional: true, wait }).catch(() => false);
    for (const page of [pa, pb]) {
      if (await tap(page, 'Attack', 300)) {
        const list = await buttons(page).catch(() => []);
        const target = list.find((t) => /cover|range|Bandit|Raider|Thug|Zindra|Kobold/i.test(t));
        if (target) await tap(page, target, 0);
      } else await tap(page, 'End turn', 0);
    }
    await settle(1500);
  }
  return false;
}

// story: the harbor adventure from the opening to the fight, on the flat combat screen (no WebGPU)
async function story() {
  await startServer({});
  Object.assign(s.labels, JSON.parse(fs.readFileSync(packPath, 'utf8')).offer_labels);
  const browser = await launchGpu(false);
  const { tv, pa, pb } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: false, voice: { latencyMs: 100, lengthFor: () => 1 } });
  await choose(tv, pa, s.label('begin-story'));
  await checkpoint(tv, 'opening scene with narrator and choices');
  await choose(tv, pa, s.label('ask-courier'));
  await checkpoint(tv, 'talk scene after asking the courier');
  // A hero speaks to the courier: the thinking mark while the reply is composed, then the bubbles.
  await pa.evaluate(() => { const d = document.querySelector('details.say-more'); if (d) d.open = true; });
  await pa.selectOption('select[aria-label="Input context"]', { label: 'Conversation' }).catch(() => null);
  await pa.fill('textarea.dialogue-draft', 'Courier, what exactly was in that packet?').catch(() => null);
  await pa.locator('[data-dialogue=submit]').click().catch(() => null);
  const thinking = await until(tv, (now) => now.bubbles.length > 0 || now.intents.length > 0, 8000).then(() => true, () => false);
  reached('talk scene with a hero line, bubble and thinking mark', thinking, 'no bubble or thinking mark appeared without the text model');
  if (thinking) { await settle(600); await checkpoint(tv, 'talk scene with a hero line, bubble and thinking mark'); }
  await choose(tv, pa, s.label('escort-courier'));
  await checkpoint(tv, 'scene after offering protection');
  await click(s, pa, s.label('defend-courier'), { wait: 100 });
  await until(tv, (now) => now.screen === 'combat', 60000).catch(() => null);
  await settle(2500);
  await checkpoint(tv, 'flat combat fallback');
  await fightOut(tv, pa, pb);
  await until(tv, (now) => now.screen === 'aftermath' && !now.stage, 30000).catch(() => null);
  await settle(2500);
  const screen = await states(tv);
  if (reached('victory or defeat screen', screen.screen === 'aftermath', JSON.stringify(screen).slice(0, 200))) {
    await checkpoint(tv, 'victory or defeat screen (flat)', { watchMs: 12000 });
    // After the fight: a short rest, then on to the inn.
    if (await click(s, pa, 'Short Rest', { optional: true, wait: 3000 }) || await click(s, pb, 'Short Rest', { optional: true, wait: 3000 })) {
      await settle(4000);
      await checkpoint(tv, 'rest', { watchMs: 6000 });
    } else reached('rest', false, 'no Short Rest offered');
    if (await click(s, pa, s.label('choose-harbor-scene'), { optional: true, wait: 3000 }) || await click(s, pb, s.label('choose-harbor-scene'), { optional: true, wait: 3000 })) {
      await until(tv, calm, 60000).catch(() => null);
      await settle(2000);
      await checkpoint(tv, 'scene after the fight (the inn)', { watchMs: 6000 });
    } else reached('scene after the fight (the inn)', false, 'no continue offered');
  }
  await browser.close();
}

// A random adventure played from the lobby by pressing what the phones offer (chapter-lib), with a checkpoint
// at every new kind of screen the display reaches: a fight, its victory or defeat, the resolution, the epilogue,
// a look at something, the end card. `gpu` gives the 3D battlefield instead of the flat fight screen.
async function chapter(seed, gpu, label) {
  startedAdventure(seed);
  await startServer({});
  const browser = await launchGpu(gpu);
  const phone = () => browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
  const tv = await (await browser.newContext({ viewport: { width: 1920, height: 1080 } })).newPage();
  const table = { s, browser, tv, pa: await (await phone()).newPage(), pb: await (await phone()).newPage(), out, base: process.env.DF_BASE, failures: [], shots: 0 };
  for (const page of [tv, table.pa, table.pb]) watch(s, page, 'chapter');
  await tv.goto(`${table.base}/gameplay/display`);
  await table.pa.goto(`${table.base}/gameplay/player`);
  await table.pb.goto(`${table.base}/gameplay/player`);
  await settle(2500);
  await makeHeroes(table);
  const seen = new Set();
  const names = [];
  const sample = async () => tv.evaluate(() => {
    const stage = document.querySelector('.tv-stage');
    if (!stage || stage.getAttribute('data-tv-mode') !== 'scene') return null;
    const screen = document.querySelector('.tv-stage .live-screen')?.getAttribute('data-screen') || 'scene';
    const field = Boolean(document.querySelector('.battlefield')) && getComputedStyle(document.querySelector('.battlefield')).visibility !== 'hidden';
    const foes = field ? document.querySelectorAll('.bf-hud [data-side="2"], .bf-hud .bf-foe').length : 0;
    return { screen, field, calm: !stage.getAttribute('data-stage'), ended: stage.hasAttribute('data-ended'), title: document.querySelector('.tv-title')?.textContent?.trim() || '', battle: stage.getAttribute('data-battle-state'), foes };
  });
  const done = async () => {
    const now = await sample();
    if (!now || !now.calm) return false;
    const kind = now.ended ? 'the end card' : now.screen === 'aftermath' ? 'victory or defeat screen' : now.screen === 'combat' ? (now.field ? 'battlefield' : 'flat combat screen') : `scene "${now.title}"`;
    const key = `${kind}`;
    if (!seen.has(key) && !(kind === 'battlefield' && !['turn-start', 'turn-end'].includes(now.battle))) {
      seen.add(key);
      names.push(key);
      await checkpoint(tv, `${label}: ${key}`, { watchMs: now.ended || now.screen === 'aftermath' ? 12000 : 0 });
    }
    return now.ended && seen.has('the end card');
  };
  const reachedEnd = await playUntil(table, done, { steps: 600, label });
  reached(`${label}: the end card`, reachedEnd, `states seen: ${names.join('; ')}`);
  await browser.close();
}

// The random adventure of one seed (see stage-tactical.mjs), set into the environment the server is started with.
function startedAdventure(seed) {
  Object.assign(process.env, {
    DF_GAMEPLAY_ADVENTURE: 'random',
    DF_GAMEPLAY_ADVENTURE_SEED: seed,
    DF_GAMEPLAY_ADVENTURE_TABLES: path.join(root, 'content', 'adventure-tables'),
    DF_GAMEPLAY_PACK_DIR: '',
  });
}

const SCENARIOS = { lobby, story, fight: () => chapter('0xd5f94172edb1d404', false, 'fight chapter (flat)') };
for (const [name, run] of Object.entries(SCENARIOS)) {
  if (!wanted(name)) continue;
  s.note(`=== scenario ${name}`);
  try { await run(); } catch (error) { check(false, `scenario ${name} failed: ${String(error).slice(0, 300)}`); }
}

// ---------------------------------------------------------------------------------------------
const total = results.reduce((n, r) => n + r.violations.length, 0);
const perState = new Map();
for (const r of results) perState.set(r.state, (perState.get(r.state) || 0) + r.violations.length);
s.note('--- violations per screen state (all viewports)');
for (const [state, n] of perState) s.note(`${String(n).padStart(4)}  ${state}`);
const byKind = {};
for (const r of results) for (const v of r.violations) byKind[v.kind] = (byKind[v.kind] || 0) + 1;
s.note(`total ${total} violations ${JSON.stringify(byKind)}; ${results.length} checkpoints; not reached: ${unreached.length ? unreached.join(', ') : 'none'}`);
fs.writeFileSync(path.join(out, 'violations.json'), JSON.stringify({ results, unreached }, null, 1));
check(total === 0, `no layout violations (${total})`);
check(unreached.length === 0, `every screen state was reached (${unreached.length} not)`);
const dummy = { artifactRoot, root, packPath, until, states, playToOpening };
void dummy;
await finish({ close: async () => {} });
