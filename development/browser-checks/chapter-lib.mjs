// Evaluator tooling only (not application code): a table of one display and two phones, and a driver
// that plays a chapter from the lobby to its end card by reading what the phones offer.
import fs from 'node:fs';
import { launch, session, click, create, settle, watch } from './lib.mjs';

// Controls that are not game offers: the app's own menus, the recovery toolbar and the dialogue controls.
export const SYSTEM =
  /^(PLAY|Hero|HERO|MENU|Reconnect|Confirm saved choice|Leave this game on this device|Close|Full character sheet|Say something.*|Ask for clarification|Decline this choice|)$/;
// Fight offers, most useful first.
const FIGHT_ORDER = [/^Attack/, /Greatsword|Longbow|Shoot/, /Second Wind/, /^Move/, /Dash|Dodge|Disengage|Stand|Pick up|Use|Opportunity|Decline/, /End turn|Advance/];
const UNCERTAIN = /Outcome unknown|may not have been saved/;

export async function openTable(base, out, room) {
  fs.mkdirSync(out, { recursive: true });
  const s = session(base, out, null);
  const browser = await launch();
  const phoneContext = () => browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
  const tv = await (await browser.newContext({ viewport: { width: 1920, height: 1080 } })).newPage();
  const pa = await (await phoneContext()).newPage();
  const pb = await (await phoneContext()).newPage();
  for (const [n, p] of [['tv', tv], ['a', pa], ['b', pb]]) watch(s, p, n);
  const table = { s, browser, tv, pa, pb, out, base, room, failures: [], shots: 0 };
  await tv.goto(`${base}/gameplay/display`);
  await pa.goto(`${base}/gameplay/player${room ? `?room=${room}` : ''}`);
  await pb.goto(`${base}/gameplay/player${room ? `?room=${room}` : ''}`);
  await settle(2500);
  return table;
}

export async function makeHeroes(table, names = [['Brakka', 'Copper beard'], ['Thrain', 'Missing ear']]) {
  const { s, pa, pb } = table;
  await create(s, pa, names[0][0], names[0][1]);
  await click(s, pa, 'Confirm your hero');
  await create(s, pb, names[1][0], names[1][1]);
  await click(s, pb, 'Confirm your hero');
  await settle();
}

export const enabled = (page) => page.locator('button:not([disabled])').allInnerTexts().then((all) => all.map((t) => t.trim()));
export const text = (page) => page.locator('body').innerText();
// The end card is the TV's scene titled "The End" (its text may be upper-cased by CSS).
export const ended = async (table) => /^the end$/im.test(await text(table.tv));

export async function shot(table, name) {
  const { tv, pa, pb, out } = table;
  await tv.screenshot({ path: `${out}/${name}-tv.png` });
  await pa.screenshot({ path: `${out}/${name}-a.png`, fullPage: true });
  await pb.screenshot({ path: `${out}/${name}-b.png`, fullPage: true });
}

export async function options(table) {
  const all = [];
  for (const [who, page] of [['a', table.pa], ['b', table.pb]]) {
    for (const label of await enabled(page)) if (!SYSTEM.test(label)) all.push({ who, page, label });
  }
  return all;
}

// One story or fight step: the best fight offer if a fight is on, otherwise the first offer that is not
// a Short Rest (a rest is always offered and never moves the story on).
let consecutiveMoves = 0;
export function choose(all, skip = /^Short Rest|^Say something|^Begin a new adventure|^Go on to chapter|^Ready\. |^Starting/) {
  // A hero that has moved twice ends the turn: Move stays offered and would otherwise be pressed for ever.
  const order = consecutiveMoves >= 2 ? [FIGHT_ORDER[0], FIGHT_ORDER[1], FIGHT_ORDER[2], FIGHT_ORDER[5], FIGHT_ORDER[3], FIGHT_ORDER[4]] : FIGHT_ORDER;
  for (const pattern of order) {
    const hit = all.find((o) => pattern.test(o.label));
    if (hit) {
      consecutiveMoves = /^Move/.test(hit.label) ? consecutiveMoves + 1 : 0;
      return hit;
    }
  }
  return all.find((o) => !skip.test(o.label)) ?? null;
}

// Plays until `done(table)` is true, pressing one offered choice per step. Records every choice.
export async function playUntil(table, done, { steps = 400, label = 'play', skip } = {}) {
  const { s, failures } = table;
  let idle = 0;
  for (let step = 0; step < steps; step += 1) {
    if (await done(table)) return true;
    for (const [name, page] of [['tv', table.tv], ['a', table.pa], ['b', table.pb]]) {
      if (UNCERTAIN.test(await text(page))) failures.push(`${label} step ${step}: ${name} shows an uncertain outcome`);
    }
    const pick = choose(await options(table), skip);
    if (!pick) {
      idle += 1;
      if (idle > 80) { failures.push(`${label} step ${step}: nothing offered for 80 polls`); return false; }
      await settle(500);
      continue;
    }
    idle = 0;
    s.note(`${label} step ${step}: ${pick.who} presses "${pick.label}"`);
    try { await pick.page.locator('button:not([disabled])', { hasText: pick.label }).first().click({ timeout: 4000 }); } catch (e) { s.note(`click missed "${pick.label}"`); }
    await settle(1100);
  }
  return done(table);
}

export async function finish(table, name, failuresExtra = []) {
  const { s, out, browser, failures } = table;
  fs.writeFileSync(`${out}/${name}.txt`, s.log.join('\n') + '\n');
  await browser.close();
  const all = [...failures, ...failuresExtra];
  if (all.length) { console.error(`FAILED:\n${all.join('\n')}`); process.exit(1); }
  console.log(`PASSED: ${name}`);
}
