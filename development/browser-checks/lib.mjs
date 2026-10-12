// Evaluator tooling only (not application code): shared helpers for PLAY-F05 browser checks.
import { createRequire } from 'node:module';
import fs from 'node:fs';
const require = createRequire(import.meta.url);
export const { chromium } = require(process.env.DF_PLAYWRIGHT || '/Users/earlcameron/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright');
export const exe = process.env.DF_CHROMIUM || '/Users/earlcameron/Desktop/dungeonflux/artifacts/cache/playwright/chromium_headless_shell-1234/chrome-headless-shell-mac-arm64/chrome-headless-shell';
export const settle = (ms = 1500) => new Promise((r) => setTimeout(r, ms));

export function session(base, out, packPath) {
  fs.mkdirSync(out, { recursive: true });
  const labels = packPath ? JSON.parse(fs.readFileSync(packPath, 'utf8')).offer_labels : {};
  const log = [];
  const note = (m) => { log.push(`${new Date().toISOString().slice(11, 23)} ${m}`); console.log(m); };
  return { base, out, labels, log, note, label: (k) => labels[k] };
}

export async function click(s, page, text, { optional = false, wait = 1500 } = {}) {
  const button = page.locator('button:not([disabled])', { hasText: text }).first();
  try { await button.waitFor({ state: 'visible', timeout: optional ? 1500 : 8000 }); } catch { if (!optional) s.note(`MISSING button "${text}"`); return false; }
  await button.scrollIntoViewIfNeeded();
  await button.click();
  await settle(wait);
  return true;
}

export async function create(s, page, name, detail) {
  await click(s, page, 'Join this room');
  await page.waitForSelector('button[aria-pressed]', { timeout: 10000 });
  await page.evaluate(([n, d]) => {
    const set = (el, v) => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, v); el.dispatchEvent(new Event('input', { bubbles: true })); el.dispatchEvent(new Event('change', { bubbles: true })); };
    const inputs = [...document.querySelectorAll('input[type=text],input:not([type])')].filter((i) => !i.readOnly);
    set(inputs[0], n); set(inputs[1], d);
  }, [name, detail]);
  await page.evaluate(() => {
    const used = new Set();
    for (const h of [...document.querySelectorAll('h2')].filter((x) => x.parentElement.querySelector('button[aria-pressed]'))) {
      const buttons = [...h.parentElement.querySelectorAll('button[aria-pressed]')];
      const key = h.textContent.includes('language') ? 'lang' : h.textContent.includes('skill') ? 'skill' : h.textContent;
      const b = buttons.find((x) => !used.has(key + x.textContent)) || buttons[0];
      used.add(key + b.textContent);
      b.click();
    }
  });
  await settle(600);
}

export async function launch() {
  return chromium.launch({ headless: true, executablePath: exe });
}

export function watch(s, page, tag) {
  page.on('pageerror', (e) => s.note(`PAGEERROR ${tag}: ${e.message.slice(0, 200)}`));
  page.on('console', (m) => { if (m.type() === 'error') s.note(`CONSOLE ${tag}: ${m.text().slice(0, 200)}`); });
}

// Reads what the TV shows in its bubbles, for ordering checks.
export async function tvState(tv) {
  return tv.evaluate(() => {
    const text = (e) => (e ? e.textContent.trim().replace(/\s+/g, ' ') : '');
    const bubbles = [...document.querySelectorAll('.tv-bubble')].map((b) => ({
      kind: b.getAttribute('data-kind'),
      name: text(b.querySelector('.tv-bubble-name')),
      text: text(b.querySelector('.tv-bubble-text')),
      voiced: b.getAttribute('data-voice-role'),
      figure: b.closest('[data-slot]')?.getAttribute('data-slot') ?? null,
    }));
    const thinking = [...document.querySelectorAll('[data-thinking]')].map((e) => e.getAttribute('data-slot') ?? e.className);
    return { bubbles, thinking };
  });
}
