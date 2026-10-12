// usage: node play-t03-create.mjs <base> <room> <out> <packageA> <packageB>
// Creates one hero per phone with the named combat package (greatsword or archer) and reports the
// outcome text each phone shows after "Confirm your hero". Evaluator tooling (PLAY-T03), not app code.
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

const [base, room, out, packageA = 'greatsword', packageB = 'archer'] = process.argv.slice(2);
if (!base || !room || !out) {
  console.error('usage: node play-t03-create.mjs <base> <room> <out> <packageA> <packageB>');
  process.exit(2);
}
fs.mkdirSync(out, { recursive: true });
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
const UNCERTAIN = /Outcome unknown|may not have been saved|lost|Reconnecting/;

const browser = await chromium.launch({ headless: true, executablePath: exe });
const phone = () =>
  browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 1, isMobile: true, hasTouch: true });
const log = [];
const note = (m) => {
  const line = `${new Date().toISOString().slice(11, 23)} ${m}`;
  log.push(line);
  console.log(line);
};

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
  const picks = await page.evaluate(
    ([choices, pkg, appearance]) => {
      const result = [];
      for (const h of [...document.querySelectorAll('h2')]) {
        const container = h.parentElement;
        const buttons = [...container.querySelectorAll('button[aria-pressed]')];
        if (buttons.length === 0) continue;
        const heading = h.textContent.trim();
        let target = buttons[0];
        if (heading === 'Alignment') target = buttons.find((b) => b.textContent.trim() === 'Lawful Good') || buttons[0];
        if (heading in choices) target = buttons.find((b) => b.textContent.trim() === choices[heading]) || buttons[0];
        if (heading.startsWith('Combat training')) target = buttons.find((b) => b.textContent.trim() === pkg) || buttons[0];
        if (heading.startsWith('Hero appearance')) target = buttons.find((b) => b.textContent.trim() === appearance) || buttons[0];
        target.click();
        result.push(`${heading}=${target.textContent.trim()}`);
      }
      return result;
    },
    [CHOICES, spec.package, spec.appearance],
  );
  note(`${tag} picks: ${picks.join(' | ')}`);
  await settle(800);
  await page.locator('button', { hasText: 'Confirm your hero' }).first().click();
  await settle(4000);
  await settle(2000);
  const text = await page.locator('body').innerText();
  fs.writeFileSync(`${out}/${tag}-after-confirm.txt`, text);
  const hit = text.match(UNCERTAIN);
  const rejected = /can't be done right now/.test(text);
  // A saved hero leaves the creation form: the form's confirm control and "Draft - editing" are gone.
  const created = !/Confirm your hero/.test(text) && !/Draft - editing/.test(text);
  note(`${tag} after confirm: uncertain=${hit ? hit[0] : 'none'} rejected=${rejected} created=${created}`);
  await page.screenshot({ path: `${out}/${tag}-after-confirm.png`, fullPage: true });
  return created && !hit;
}

const pa = await (await phone()).newPage();
const pb = await (await phone()).newPage();
for (const [tag, page] of [['a', pa], ['b', pb]]) {
  page.on('pageerror', (e) => note(`PAGEERROR ${tag}: ${e.message.slice(0, 200)}`));
  page.on('console', (m) => { if (m.type() === 'error') note(`CONSOLE ${tag}: ${m.text().slice(0, 200)}`); });
}
await pa.goto(`${base}/gameplay/player?room=${room}`);
await pb.goto(`${base}/gameplay/player?room=${room}`);
await settle(2500);
const okA = await createHero('a', pa, 'Brakka', packageA);
const okB = await createHero('b', pb, 'Sylvie', packageB);
fs.writeFileSync(`${out}/create-timeline.txt`, log.join('\n') + '\n');
await browser.close();
console.log(`RESULT a=${okA ? 'created' : 'NOT-CREATED'} b=${okB ? 'created' : 'NOT-CREATED'}`);
