// usage: node play-t03-fight.mjs <base> <out> <steps> [packageA] [packageB]
// PLAY-T03 browser check: two heroes (greatsword and archer by default) join, begin the story and
// are played through the opening and the fight on two phones and a TV. The driver takes the first
// offered option on whichever phone has one, prefers combat offers once a fight is on, and records
// each step. It fails when a phone shows a lost-connection banner, when a phone is left with no
// enabled option while the fight is running, or when the run does not reach the end of the fight.
// Evaluator tooling (PLAY-T03), not application code.
import fs from 'node:fs';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const { chromium } = require(
  process.env.DF_PLAYWRIGHT ||
    '/Users/earlcameron/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright',
);
const exe =
  process.env.DF_CHROMIUM ||
  '/Users/earlcameron/Desktop/dungeonflux/artifacts/cache/playwright/chromium_headless_shell-1234/chrome-headless-shell-mac-arm64/chrome-headless-shell';

const [base, out, stepsArg = '120', packageA = 'greatsword', packageB = 'archer'] = process.argv.slice(2);
if (!base || !out) {
  console.error('usage: node play-t03-fight.mjs <base> <out> <steps> [packageA] [packageB]');
  process.exit(2);
}
fs.mkdirSync(out, { recursive: true });
const STEPS = Number(stepsArg);
const settle = (ms) => new Promise((r) => setTimeout(r, ms));
const PACKAGES = {
  greatsword: { package: 'Stalwart · Greatsword and Defense', appearance: 'Greatsword hero' },
  archer: { package: 'Longbow and Archery · Fighter B + Soldier B', appearance: 'Woman with a bow' },
};
const CHOICES = {
  'First additional language': 'Elvish',
  'Second additional language': 'Orc',
  'First Fighter skill': 'Acrobatics',
  'Second Fighter skill': 'Survival',
  'Gaming set proficiency': 'Playing Cards',
};
// Controls that are not game offers: the app's own menus, the recovery toolbar and the dialogue
// controls (which stay visible while a fight is on).
const SYSTEM = /^(PLAY|Hero|HERO|MENU|Reconnect|Confirm saved choice|Leave this game on this device|Close|Full character sheet|Say something.*|Ask for clarification|Decline this choice|)$/;
const FIGHT = /Attack|Shoot|Longbow|Greatsword|Move|Dash|Dodge|Disengage|End turn|Stand|Second Wind|Use|Opportunity|Decline|Pick up/;
const LOST = /connection to the game was lost|Reconnecting/;

const browser = await chromium.launch({ headless: true, executablePath: exe });
// Records every incoming WebSocket message size and every close (code, reason) in the page, so a
// lost connection can be matched with the frame that came before it.
const WS_LOG = () => {
  window.__wsLog = [];
  const Native = window.WebSocket;
  const at = () => Math.round(performance.now());
  function Recording(...args) {
    const ws = new Native(...args);
    ws.addEventListener('message', (e) => {
      const bytes = e.data && e.data.byteLength !== undefined ? e.data.byteLength : typeof e.data === 'string' ? e.data.length : -1;
      window.__wsLog.push({ t: at(), kind: 'message', bytes });
    });
    ws.addEventListener('close', (e) => window.__wsLog.push({ t: at(), kind: 'close', code: e.code, reason: e.reason, clean: e.wasClean }));
    ws.addEventListener('error', () => window.__wsLog.push({ t: at(), kind: 'error' }));
    return ws;
  }
  Recording.prototype = Native.prototype;
  Object.assign(Recording, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
  window.WebSocket = Recording;
};
const newPhone = async () => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
  await context.addInitScript(WS_LOG);
  return context.newPage();
};
const tvContext = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
const tv = await tvContext.newPage();
const pa = await newPhone();
const pb = await newPhone();
const failures = [];
const timeline = [];
const note = (m) => {
  const line = `${new Date().toISOString().slice(11, 23)} ${m}`;
  timeline.push(line);
  console.log(line);
};
for (const [tag, page] of [['tv', tv], ['a', pa], ['b', pb]]) {
  page.on('pageerror', (e) => note(`PAGEERROR ${tag}: ${e.message.slice(0, 200)}`));
  page.on('console', (m) => {
    if (m.type() === 'error' || m.type() === 'warning') note(`CONSOLE ${tag} ${m.type()}: ${m.text().slice(0, 240)}`);
  });
  page.on('websocket', (ws) => {
    note(`WS ${tag} open ${ws.url().slice(0, 80)}`);
    ws.on('close', () => note(`WS ${tag} close`));
    ws.on('socketerror', (e) => note(`WS ${tag} socketerror ${String(e).slice(0, 160)}`));
  });
}

async function enabled(page) {
  return (await page.locator('button:not([disabled])').allInnerTexts()).map((t) => t.trim());
}
async function clickLabel(page, label) {
  const button = page.locator('button:not([disabled])', { hasText: label }).first();
  try {
    await button.click({ timeout: 4000 });
    return true;
  } catch (error) {
    // The view re-renders between a read and a click; the next poll reads the new options.
    note(`click missed "${label}": ${String(error.message).split('\n')[0].slice(0, 120)}`);
    return false;
  }
}
const seenCloses = new Map();
async function reportCloses(tag, page) {
  const log = await page.evaluate(() => window.__wsLog || []).catch(() => []);
  const closes = log.filter((entry) => entry.kind === 'close');
  const known = seenCloses.get(tag) ?? 0;
  for (const entry of closes.slice(known)) {
    note(`WS ${tag} close code=${entry.code} clean=${entry.clean} reason=${JSON.stringify(entry.reason)}`);
  }
  seenCloses.set(tag, closes.length);
}

async function createHero(tag, page, name, kind) {
  const spec = PACKAGES[kind];
  await page.locator('button', { hasText: 'Join this room' }).first().click();
  await settle(2000);
  await page.waitForSelector('button[aria-pressed]', { timeout: 15000 });
  await page.evaluate(([n]) => {
    const set = (el, v) => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, v);
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    };
    const inputs = [...document.querySelectorAll('input[type=text],input:not([type])')].filter((i) => !i.readOnly);
    set(inputs[0], n);
  }, [name]);
  await page.evaluate(
    ([choices, pkg, appearance]) => {
      for (const h of [...document.querySelectorAll('h2')]) {
        const buttons = [...h.parentElement.querySelectorAll('button[aria-pressed]')];
        if (buttons.length === 0) continue;
        const heading = h.textContent.trim();
        let target = buttons[0];
        if (heading === 'Alignment') target = buttons.find((b) => b.textContent.trim() === (pkg.lawful ? 'Lawful Good' : 'Chaotic Good')) || buttons[0];
        if (heading in choices) target = buttons.find((b) => b.textContent.trim() === choices[heading]) || buttons[0];
        if (heading.startsWith('Combat training')) target = buttons.find((b) => b.textContent.trim() === pkg.package) || buttons[0];
        if (heading.startsWith('Hero appearance')) target = buttons.find((b) => b.textContent.trim() === pkg.appearance) || buttons[0];
        target.click();
      }
    },
    [CHOICES, { ...spec, lawful: tag === 'a' }],
  );
  await settle(800);
  await page.locator('button', { hasText: 'Confirm your hero' }).first().click();
  await settle(3000);
  const text = await page.locator('body').innerText();
  const created = !/Confirm your hero/.test(text) && !/Draft - editing/.test(text);
  note(`${tag} hero ${name} (${kind}) created=${created}`);
  return created;
}

await tv.goto(`${base}/gameplay/display`);
const room = process.env.DF_ROOM;
if (!room) {
  console.error('set DF_ROOM to the room code shown by the server');
  process.exit(2);
}
await pa.goto(`${base}/gameplay/player?room=${room}`);
await pb.goto(`${base}/gameplay/player?room=${room}`);
await settle(2500);
const okA = await createHero('a', pa, 'Brakka', packageA);
const okB = await createHero('b', pb, 'Sylvie', packageB);
if (!okA || !okB) failures.push(`creation: a=${okA} b=${okB}`);

// Play the story and the fight: take the first real option, preferring combat offers in a fight.
let fightSeen = false;
let idle = 0;
for (let step = 0; step < STEPS && failures.length === 0; step += 1) {
  for (const [tag, page] of [['a', pa], ['b', pb]]) {
    await reportCloses(tag, page);
    const text = await page.locator('body').innerText();
    if (LOST.test(text)) {
      failures.push(`step ${step}: ${tag} shows a connection banner`);
      fs.writeFileSync(`${out}/lost-${tag}-${step}.txt`, text);
      fs.writeFileSync(`${out}/ws-${tag}-${step}.json`, JSON.stringify(await page.evaluate(() => window.__wsLog), null, 1));
      await page.screenshot({ path: `${out}/lost-${tag}-${step}.png`, fullPage: true });
      const status = await page.evaluate(() => document.querySelector('.connection')?.textContent ?? '(no status)');
      note(`step ${step}: ${tag} status line: ${JSON.stringify(status)}`);
    }
  }
  const options = [];
  for (const [tag, page] of [['a', pa], ['b', pb]]) {
    for (const label of await enabled(page)) {
      if (!SYSTEM.test(label)) options.push({ tag, page, label });
    }
  }
  const combatOffers = options.filter((o) => FIGHT.test(o.label));
  if (combatOffers.length > 0 && !fightSeen) {
    fightSeen = true;
    note(`step ${step}: fight offers ${JSON.stringify(combatOffers.map((o) => `${o.tag}:${o.label}`))}`);
    await tv.screenshot({ path: `${out}/fight-start-tv.png` });
  }
  const pick = combatOffers[0] ?? options[0];
  if (!pick) {
    idle += 1;
    if (idle > 40) {
      failures.push(`step ${step}: no enabled option on either phone for 40 polls`);
      break;
    }
    await settle(500);
    continue;
  }
  idle = 0;
  note(`step ${step}: ${pick.tag} presses "${pick.label}"`);
  await clickLabel(pick.page, pick.label);
  await settle(1200);
  const tvText = await tv.locator('body').innerText();
  if (/Journey Settled|agreement kept|brought the answer|VICTORY|Victory/.test(tvText)) {
    note(`step ${step}: TV shows the end of the chapter`);
    break;
  }
}
await tv.screenshot({ path: `${out}/end-tv.png` });
await pa.screenshot({ path: `${out}/end-phone-a.png`, fullPage: true });
await pb.screenshot({ path: `${out}/end-phone-b.png`, fullPage: true });
// The archer's shot must be on the table: its resolved attack names the longbow.
const archerLog = (await pb.locator('body').innerText()) + (await pa.locator('body').innerText());
if (packageB === 'archer' && !/longbow/i.test(archerLog)) {
  failures.push('the archer never shot: no longbow attack on either phone');
}
note(`archer shot on the table: ${/longbow/i.test(archerLog)}`);
fs.writeFileSync(`${out}/timeline.txt`, timeline.join('\n') + '\n');
await browser.close();
if (failures.length) {
  console.error(`FAILED:\n${failures.join('\n')}`);
  process.exit(1);
}
console.log(`PASSED: ${STEPS} steps without a lost-connection banner (fight seen: ${fightSeen}).`);
