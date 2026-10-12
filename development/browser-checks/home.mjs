// usage: node home.mjs <base> <out> <label> [width=1920] [height=1080] [strip=0]
// Evaluator tooling only (not application code): screenshots the shared display's lobby, reads the
// home theme the server sent (data-home, custom properties), the block boxes, the room-code and
// party text colours behind their glyphs, and writes JSON to <out>/<label>.json. With strip=1 the
// themed attributes are removed first, which is the same lobby drawn by the stylesheet's default.
import { launch, settle } from './lib.mjs';
import fs from 'node:fs';
const [base, out, label, widthArg, heightArg, stripArg] = process.argv.slice(2);
const width = Number(widthArg || 1920);
const height = Number(heightArg || 1080);
const strip = stripArg === '1';
fs.mkdirSync(out, { recursive: true });
const browser = await launch();
const context = await browser.newContext({ viewport: { width, height } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(`PAGEERROR ${e.message.slice(0, 200)}`));
page.on('console', (m) => { if (m.type() === 'error') errors.push(`CONSOLE ${m.text().slice(0, 200)}`); });
await page.goto(`${base}/gameplay/display`);
await settle(4000);
if (strip) {
  await page.evaluate(() => {
    const stage = document.querySelector('.tv-stage');
    stage.removeAttribute('data-home');
    stage.removeAttribute('style');
  });
  await settle(400);
}
const geometry = await page.evaluate(() => {
  const stage = document.querySelector('.tv-stage');
  const rect = (selector) => {
    const element = document.querySelector(selector);
    if (!element) return null;
    const r = element.getBoundingClientRect();
    return { x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height) };
  };
  const style = stage ? getComputedStyle(stage) : null;
  return {
    mode: stage?.getAttribute('data-tv-mode'),
    home: stage?.getAttribute('data-home'),
    accent: style?.getPropertyValue('--home-accent').trim(),
    tint: style?.getPropertyValue('--home-tint').trim(),
    vignette: style?.getPropertyValue('--home-vignette').trim(),
    atmosphere: [...document.querySelectorAll('.tv-fx')].map((e) => e.className),
    art: getComputedStyle(document.querySelector('.tv-art')).backgroundImage.slice(0, 160),
    title: rect('.tv-title'),
    titleBlock: rect('.tv-title-block'),
    join: rect('.tv-join'),
    code: rect('.tv-code'),
    roster: rect('.tv-roster'),
    cards: [...document.querySelectorAll('.tv-roster .tv-card')].filter((c) => !c.hidden && c.offsetParent).map((c) => {
      const r = c.getBoundingClientRect();
      return { x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height) };
    }),
    codeText: document.querySelector('.tv-code')?.textContent?.trim(),
    codeFont: getComputedStyle(document.querySelector('.tv-code') || document.body).fontSize,
    titleFont: getComputedStyle(document.querySelector('.tv-title') || document.body).fontSize,
    // Content taller than its box is a layout failure: scrollHeight above clientHeight.
    overflow: ['.tv-title-block', '.tv-join', '.tv-roster', '.tv-code-box'].map((selector) => {
      const element = document.querySelector(selector);
      return element ? { selector, scroll: element.scrollHeight, client: element.clientHeight, box: rect(selector) } : null;
    }),
  };
});
await page.screenshot({ path: `${out}/${label}.png` });
// The same frame with every glyph made transparent, so the pixels behind each text box are the
// real ground the text sits on (panels and painting, without the text itself).
const text = await page.evaluate(() => {
  const selectors = ['.tv-title', '.tv-code', '.tv-code-label', '.tv-label', '.tv-member-name', '.tv-member-status', '.tv-url', '.tv-lede', '.tv-place'];
  const out = [];
  for (const selector of selectors) {
    const element = document.querySelector(selector);
    if (!element || !element.offsetParent) continue;
    // The glyph extent, not the block: a block-level label spans its container's full width.
    const range = document.createRange();
    range.selectNodeContents(element);
    const r = range.getBoundingClientRect();
    out.push({ selector, color: getComputedStyle(element).color, box: [Math.round(r.left), Math.round(r.top), Math.round(r.right), Math.round(r.bottom)] });
  }
  for (const element of document.querySelectorAll('.tv-stage .tv-safe *')) element.style.setProperty('color', 'transparent', 'important');
  for (const element of document.querySelectorAll('.tv-stage .tv-safe *')) element.style.setProperty('text-shadow', 'none', 'important');
  return out;
});
await settle(300);
await page.screenshot({ path: `${out}/${label}-ground.png` });
geometry.text = text;
const report = { label, width, height, strip, geometry, errors };
fs.writeFileSync(`${out}/${label}.json`, JSON.stringify(report, null, 2));
console.log(JSON.stringify(report, null, 2));
await browser.close();
