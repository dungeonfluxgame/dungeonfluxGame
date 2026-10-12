// usage: node play-u02-table-settings.mjs <base> <room> <out> [create] [keep-hidden]
//        node play-u02-table-settings.mjs <base> <room> <out> display-only
// PLAY-U02 M1 check: a seated player hides the numbered choice chips on the shared display from a
// phone's menu, the other phone shows the new state, a display reload keeps it, and it is shown
// again. With `create`, both phones first create a hero and begin the story. `keep-hidden` stops
// with the chips hidden (for a restart check); `display-only` only reads the display. Evaluator
// tooling (not application code): it drives the real server and reports what each screen shows.
import fs from 'node:fs';
import { chromium, exe, settle } from './lib.mjs';

const [base, room, out, ...flags] = process.argv.slice(2);
if (!base || !room || !out) {
  console.error('usage: node play-u02-table-settings.mjs <base> <room> <out> [create] [keep-hidden]');
  process.exit(2);
}
const mode = flags.includes('display-only') ? 'display-only' : flags.includes('create') ? 'create' : '';
const keepHidden = flags.includes('keep-hidden');
fs.mkdirSync(out, { recursive: true });
const log = [];
const note = (m) => {
  const line = `${new Date().toISOString().slice(11, 23)} ${m}`;
  log.push(line);
  console.log(line);
};

const browser = await chromium.launch({ headless: true, executablePath: exe });
const phone = () =>
  browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 1, isMobile: true, hasTouch: true });
const desktop = () => browser.newContext({ viewport: { width: 1280, height: 720 }, deviceScaleFactor: 1 });

// What the shared display shows of the choice chips and the narrator box.
async function tvState(display) {
  return display.evaluate(() => {
    const stage = document.querySelector('.tv-stage');
    const row = document.querySelector('.tv-choices');
    const box = document.querySelector('.tv-stagebox');
    const rect = (el) => {
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { top: Math.round(r.top), height: Math.round(r.height), visible: r.height > 0 && getComputedStyle(el).display !== 'none' };
    };
    return {
      chips: stage?.getAttribute('data-chips') ?? null,
      rowDisplay: row ? getComputedStyle(row).display : null,
      chipCount: document.querySelectorAll('.tv-choice').length,
      chipLabels: [...document.querySelectorAll('.tv-choice-label')].map((c) => c.textContent.trim()),
      narratorBox: rect(box),
      stageState: stage?.getAttribute('data-scene-state') ?? null,
    };
  });
}

// The phone's menu item for the table setting: its label and whether it is pressed.
async function menuSetting(page) {
  await page.locator('.live-tab', { hasText: 'Menu' }).first().click();
  await settle(400);
  const toggle = page.locator('button', { hasText: 'Show choices on the TV:' }).first();
  const visible = await toggle.isVisible().catch(() => false);
  const result = visible
    ? { label: (await toggle.textContent()).trim(), pressed: await toggle.getAttribute('aria-pressed') }
    : { label: null, pressed: null };
  await closeMenu(page);
  return result;
}

async function closeMenu(page) {
  const close = page.locator('.live-sheet-close').first();
  if (await close.isVisible().catch(() => false)) {
    await close.click();
    await settle(400);
  }
}

async function pressSetting(page) {
  await page.locator('.live-tab', { hasText: 'Menu' }).first().click();
  await settle(400);
  const toggle = page.locator('button', { hasText: 'Show choices on the TV:' }).first();
  await toggle.click();
  await settle(1500);
  const feedback = await page.locator('.live-toast .feedback').first().textContent().catch(() => '');
  await closeMenu(page);
  return feedback.trim();
}

async function createHero(tag, page, name, kind) {
  const packages = {
    greatsword: { package: 'Stalwart · Greatsword and Defense', appearance: 'Greatsword hero' },
    archer: { package: 'Longbow and Archery · Fighter B + Soldier B', appearance: 'Woman with a bow' },
  };
  const spec = packages[kind];
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
  const choices = {
    'First additional language': 'Elvish',
    'Second additional language': 'Orc',
    'First Fighter skill': 'Acrobatics',
    'Second Fighter skill': 'Survival',
    'Gaming set proficiency': 'Playing Cards',
  };
  await page.evaluate(
    ([picks, pkg, appearance]) => {
      for (const h of [...document.querySelectorAll('h2')]) {
        const buttons = [...h.parentElement.querySelectorAll('button[aria-pressed]')];
        if (buttons.length === 0) continue;
        const heading = h.textContent.trim();
        let target = buttons[0];
        if (heading === 'Alignment') target = buttons.find((b) => b.textContent.trim() === 'Lawful Good') || buttons[0];
        if (heading in picks) target = buttons.find((b) => b.textContent.trim() === picks[heading]) || buttons[0];
        if (heading.startsWith('Combat training')) target = buttons.find((b) => b.textContent.trim() === pkg) || buttons[0];
        if (heading.startsWith('Hero appearance')) target = buttons.find((b) => b.textContent.trim() === appearance) || buttons[0];
        target.click();
      }
    },
    [choices, spec.package, spec.appearance],
  );
  await settle(800);
  await page.locator('button', { hasText: 'Confirm your hero' }).first().click();
  await settle(4000);
  note(`${tag} hero ${name} confirmed`);
}

const displayContext = await desktop();
const display = await displayContext.newPage();
display.on('pageerror', (e) => note(`PAGEERROR display: ${e.message.slice(0, 200)}`));
display.on('console', (m) => { if (m.type() === 'error') note(`CONSOLE display: ${m.text().slice(0, 200)}`); });
await display.goto(`${base}/gameplay/display`);
await settle(2500);
note(`display start: ${JSON.stringify(await tvState(display))}`);
await display.screenshot({ path: `${out}/01-display-start.png` });
if (mode === 'display-only') {
  fs.writeFileSync(`${out}/timeline.txt`, log.join('\n') + '\n');
  await browser.close();
  process.exit(0);
}

const contextA = await phone();
const contextB = await phone();
const pa = await contextA.newPage();
const pb = await contextB.newPage();
for (const [tag, page] of [['a', pa], ['b', pb]]) {
  page.on('pageerror', (e) => note(`PAGEERROR ${tag}: ${e.message.slice(0, 200)}`));
  page.on('console', (m) => { if (m.type() === 'error') note(`CONSOLE ${tag}: ${m.text().slice(0, 200)}`); });
}
await pa.goto(`${base}/gameplay/player?room=${room}`);
await pb.goto(`${base}/gameplay/player?room=${room}`);
await settle(2500);

if (mode === 'create') {
  await createHero('a', pa, 'Brakka', 'greatsword');
  await createHero('b', pb, 'Sylvie', 'archer');
  await settle(2000);
  const begin = pa.locator('button', { hasText: 'Enter the Lantern Wharf' }).first();
  if (await begin.isVisible().catch(() => false)) {
    await begin.click();
    note('phone a began the story (Enter the Lantern Wharf)');
  }
  await settle(8000);
  note(`display after begin: ${JSON.stringify(await tvState(display))}`);
}

note(`display before toggle: ${JSON.stringify(await tvState(display))}`);
const beforeA = await menuSetting(pa);
const beforeB = await menuSetting(pb);
note(`menu before: a=${JSON.stringify(beforeA)} b=${JSON.stringify(beforeB)}`);
await pa.screenshot({ path: `${out}/02-phone-a-menu-on.png` });

const savedHidden = await pressSetting(pa);
note(`phone a pressed (hide): feedback="${savedHidden}"`);
await settle(1500);
const hiddenDisplay = await tvState(display);
note(`display after hide: ${JSON.stringify(hiddenDisplay)}`);
await display.screenshot({ path: `${out}/03-display-hidden.png` });
const hiddenB = await menuSetting(pb);
note(`phone b menu after hide: ${JSON.stringify(hiddenB)}`);
await pb.screenshot({ path: `${out}/04-phone-b-menu-off.png` });

await display.reload();
await settle(3000);
const reloadedDisplay = await tvState(display);
note(`display after reload (still hidden?): ${JSON.stringify(reloadedDisplay)}`);
await display.screenshot({ path: `${out}/05-display-reloaded-hidden.png` });

if (keepHidden) {
  fs.writeFileSync(`${out}/timeline.txt`, log.join('\n') + '\n');
  await browser.close();
  process.exit(0);
}
const shownAgain = await pressSetting(pb);
note(`phone b pressed (show): feedback="${shownAgain}"`);
await settle(1500);
const shownDisplay = await tvState(display);
note(`display after show: ${JSON.stringify(shownDisplay)}`);
await display.screenshot({ path: `${out}/06-display-shown-again.png` });
const shownA = await menuSetting(pa);
note(`phone a menu after show: ${JSON.stringify(shownA)}`);
await pa.screenshot({ path: `${out}/07-phone-a-menu-on-again.png` });

fs.writeFileSync(`${out}/timeline.txt`, log.join('\n') + '\n');
await browser.close();
