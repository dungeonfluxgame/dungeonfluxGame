// usage: node home-frames.mjs <base> <out> <prefix> <steps> [expected-join-link]
// Evaluator tooling only (not application code). PLAY-V11: frames and layout checks of the shared
// display's home screen. <steps> is a comma list of <heroes>@<width>x<height>, for example
// "0@1920x1080,2@1920x1080,6@3840x2160"; heroes join (and stay) as the numbers rise. Per step it
// writes <out>/<prefix>-h<heroes>-<width>x<height>.png and checks that nothing overlaps or is
// clipped, that the block edges align, that the room code and QR sit whole inside their card, and
// (with an expected join link) that the QR on screen decodes to that link. LONG_TITLE and LONG_LEDE
// replace the title and mood line with the longest text the tables can roll; NO_PICTURE=1 removes
// the painting and the mood. The demo room holds two players, so five-hero and six-hero frames
// set the extra seats in the page (layout evidence only). Prints a JSON summary.
import { launch, settle, click, create, session } from './lib.mjs';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const [base, out, prefix, stepsArg, expectedLink] = process.argv.slice(2);
const steps = stepsArg.split(',').map((step) => {
  const [heroes, size] = step.split('@');
  const [width, height] = size.split('x').map(Number);
  return { heroes: Number(heroes), width, height };
});
fs.mkdirSync(out, { recursive: true });
const here = path.dirname(fileURLToPath(import.meta.url));
const NAMES = [['Brakka', 'Copper beard'], ['Thrain', 'Missing ear'], ['Mira', 'Ink-stained hands'], ['Oswin', 'Scarred chin'], ['Lysa', 'Silver braid'], ['Dorn', 'Heavy boots']];

const browser = await launch();
const tvContext = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
const tv = await tvContext.newPage();
const errors = [];
tv.on('pageerror', (e) => errors.push(`PAGEERROR ${e.message.slice(0, 200)}`));
tv.on('console', (m) => { if (m.type() === 'error') errors.push(`CONSOLE ${m.text().slice(0, 200)}`); });
await tv.goto(`${base}/gameplay/display`);
await settle(3500);
const s = session(base, out);
const phones = [];
async function addHero() {
  const [name, detail] = NAMES[phones.length];
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
  const page = await context.newPage();
  await page.goto(`${base}/gameplay/player`);
  await settle(2500);
  await create(s, page, name, detail);
  await click(s, page, 'Confirm your hero');
  phones.push(page);
  await settle(1200);
}

// Runs inside the page: every geometric rule of the composition, as a list of failures.
function measure([longTitle, longLede, synthetic, noPicture]) {
  const stage = document.querySelector('.tv-stage');
  // NO_PICTURE: no painting at all (the layers' backgrounds removed) and the neutral mood.
  if (noPicture) {
    for (const art of document.querySelectorAll('.tv-art')) art.setAttribute('style', 'background-image:none');
    stage.removeAttribute('style');
  }
  // The demo room holds two players, so a larger party is drawn by setting the seats the way the
  // stage's own update does (the same attributes and text). Layout evidence only, not a server party.
  if (synthetic > 2) {
    const roster = document.querySelector('.tv-roster');
    const seats = Math.min(Math.max(synthetic, 4), 6);
    roster.setAttribute('data-count', String(seats));
    [...roster.querySelectorAll('.tv-card')].forEach((card, i) => {
      card.hidden = i >= seats;
      if (i < 2 || i >= synthetic) return;
      card.setAttribute('data-state', 'ready'); card.setAttribute('data-ready', 'true'); card.removeAttribute('data-empty');
      card.querySelector('.tv-member-name').textContent = ['Brakka', 'Thrain', 'Mira', 'Oswin Longbeard', 'Lysa Moonwhisper', 'Dorn'][i];
      card.querySelector('.tv-member-status').textContent = 'Ready';
      card.querySelector('.tv-portrait-glyph').textContent = '';
      card.querySelector('.tv-portrait-img').setAttribute('style', `background-image:url(/assets/ui/generated/portrait-hero-${(i % 3) + 1})`);
    });
  }
  if (longTitle) document.querySelector('.tv-title').textContent = longTitle;
  if (longLede) document.querySelector('.tv-lede').textContent = longLede;
  const box = (el) => { const r = el.getBoundingClientRect(); return { l: r.left, t: r.top, r: r.right, b: r.bottom, w: r.width, h: r.height }; };
  const visible = (el) => el && !el.hidden && el.offsetParent !== null && getComputedStyle(el).visibility !== 'hidden';
  const one = (selector) => document.querySelector(selector);
  const W = innerWidth; const H = innerHeight;
  const fails = [];
  const need = (ok, what) => { if (!ok) fails.push(what); };
  const named = { mark: '.tv-title-block .tv-brand', title: '.tv-title', place: '.tv-place', lede: '.tv-lede', join: '.tv-join', qr: '.tv-qr', codeBox: '.tv-code-box', code: '.tv-code', url: '.tv-url', hint: '.tv-hint', roster: '.tv-roster', block: '.tv-title-block' };
  const rects = {};
  for (const [key, selector] of Object.entries(named)) { const el = one(selector); rects[key] = el && visible(el) ? box(el) : null; }
  for (const key of ['mark', 'title', 'lede', 'join', 'qr', 'codeBox', 'code', 'roster']) need(rects[key], `${key} is not visible`);
  const safe = { l: W * 0.05, r: W * 0.95, t: H * 0.05, b: H * 0.95 };
  const tol = 1.5;
  for (const key of ['block', 'join', 'roster']) {
    const r = rects[key]; if (!r) continue;
    need(r.l >= safe.l - tol && r.r <= safe.r + tol && r.t >= safe.t - tol && r.b <= safe.b + tol, `${key} leaves the 5% safe area`);
  }
  const overlap = (a, b) => a && b && a.l < b.r - tol && b.l < a.r - tol && a.t < b.b - tol && b.t < a.b - tol;
  for (const [a, b] of [['block', 'join'], ['block', 'roster'], ['join', 'roster']]) need(!overlap(rects[a], rects[b]), `${a} overlaps ${b}`);
  need(rects.block && rects.roster && Math.abs(rects.block.l - rects.roster.l) <= tol, 'title column and party row do not share the left edge');
  need(rects.join && rects.roster && Math.abs(rects.join.r - rects.roster.r) <= tol, 'join card and party row do not share the right edge');
  // Everything inside the join card keeps even padding from the card's edge.
  const inside = (child, parent, name, min) => {
    if (!child || !parent) return;
    const gaps = [child.l - parent.l, parent.r - child.r, child.t - parent.t, parent.b - child.b];
    need(gaps.every((g) => g >= min), `${name} is closer than ${Math.round(min)}px to its card edge (${gaps.map(Math.round).join(',')})`);
  };
  const pad = W * 0.006;
  inside(rects.qr, rects.join, 'qr', pad); inside(rects.codeBox, rects.join, 'room code box', pad);
  inside(rects.url, rects.join, 'address', pad);
  need(!rects.qr || !rects.codeBox || !overlap(rects.qr, rects.codeBox), 'qr overlaps the room code box');
  if (rects.code) {
    const range = document.createRange(); range.selectNodeContents(one('.tv-code'));
    const text = range.getBoundingClientRect();
    inside({ l: text.left, r: text.right, t: rects.code.t, b: rects.code.b }, rects.codeBox, 'room code', W * 0.004);
  }
  // Nothing is cut: text boxes hold their text, and no ellipsis or line clamp is in play.
  const truncated = [];
  for (const selector of ['.tv-title', '.tv-place', '.tv-lede', '.tv-code', '.tv-url', '.tv-hint', '.tv-label', '.tv-member-name', '.tv-member-role', '.tv-member-status']) {
    for (const el of document.querySelectorAll(selector)) {
      if (!visible(el)) continue;
      const style = getComputedStyle(el);
      const range = document.createRange(); range.selectNodeContents(el);
      const text = range.getBoundingClientRect(); const own = el.getBoundingClientRect();
      const slack = 0.35 * parseFloat(style.fontSize);
      const clipped = (style.overflow !== 'visible' && (el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1)) || el.scrollHeight > el.clientHeight + slack || text.right > own.right + 1 || text.left < own.left - 1;
      if (clipped || style.webkitLineClamp !== 'none' || (style.textOverflow === 'ellipsis' && style.overflow !== 'visible')) truncated.push(`${selector}: ${el.textContent.trim().slice(0, 40)}`);
    }
  }
  need(truncated.length === 0, `text cut: ${truncated.join(' | ')}`);
  const cards = [...document.querySelectorAll('.tv-roster .tv-card')].filter(visible);
  const cardBoxes = cards.map(box);
  need(cards.length >= 4 && cards.length <= 6, `unexpected seat count ${cards.length}`);
  need(cardBoxes.every((c) => Math.abs(c.w - cardBoxes[0].w) <= tol && Math.abs(c.t - cardBoxes[0].t) <= tol && Math.abs(c.h - cardBoxes[0].h) <= tol), 'party cards are not one size on one row');
  for (let i = 1; i < cardBoxes.length; i += 1) need(cardBoxes[i].l >= cardBoxes[i - 1].r - tol, 'party cards overlap or wrap');
  if (rects.roster) for (const c of cardBoxes) need(c.l >= rects.roster.l - tol && c.r <= rects.roster.r + tol && c.t >= rects.roster.t - tol && c.b <= rects.roster.b + tol, 'a party card leaves the party row');
  // Each card's own content stays inside it.
  for (const card of cards) {
    const c = card.getBoundingClientRect();
    for (const child of card.querySelectorAll('.tv-portrait,.tv-member-name,.tv-member-role,.tv-member-status')) {
      if (!visible(child)) continue;
      const r = child.getBoundingClientRect();
      need(r.left >= c.left - tol && r.right <= c.right + tol && r.top >= c.top - tol && r.bottom <= c.bottom + tol, `${child.className} leaves its card`);
    }
  }
  const qrImage = one('.tv-qr-image');
  const lines = (selector) => { const el = one(selector); return Math.round(el.getBoundingClientRect().height / parseFloat(getComputedStyle(el).lineHeight)); };
  return {
    fails, rects, seats: cards.length, filled: cards.filter((c) => !c.hasAttribute('data-empty')).length,
    codeFont: getComputedStyle(one('.tv-code')).fontSize,
    titleFont: getComputedStyle(one('.tv-title')).fontSize,
    titleLines: lines('.tv-title'),
    ledeLines: lines('.tv-lede'),
    titleText: one('.tv-title').textContent, ledeText: one('.tv-lede').textContent, placeText: one('.tv-place').textContent,
    qrLoaded: !!(qrImage && qrImage.complete && qrImage.naturalWidth > 0),
    atmosphere: [...document.querySelectorAll('.tv-fx')].map((e) => e.className.replace('tv-fx tv-fx-', '')),
    mode: stage.getAttribute('data-tv-mode'),
  };
}

const results = [];
for (const step of steps) {
  while (phones.length < Math.min(step.heroes, 2)) await addHero();
  await tv.setViewportSize({ width: step.width, height: step.height });
  await settle(1800);
  const report = await tv.evaluate(measure, [process.env.LONG_TITLE || '', process.env.LONG_LEDE || '', step.heroes, process.env.NO_PICTURE === '1']).catch((e) => ({ error: String(e), fails: [String(e)] }));
  const label = `${prefix}-h${step.heroes}-${step.width}x${step.height}`;
  await settle(400);
  await tv.screenshot({ path: path.join(out, `${label}.png`) });
  let decoded = null;
  if (expectedLink && report.rects?.qr) {
    const qrPath = path.join(out, `${label}-qr.png`);
    await tv.locator('.tv-qr-image').screenshot({ path: qrPath });
    try { decoded = execFileSync('swift', [path.join(here, 'decode-qr.swift'), qrPath], { encoding: 'utf8' }).trim(); } catch (e) { decoded = `DECODE FAILED ${String(e.message).slice(0, 100)}`; }
    if (decoded !== expectedLink) report.fails.push(`qr decodes to ${decoded}, expected ${expectedLink}`);
  }
  results.push({ label, decoded, ...report });
}
const summary = { prefix, base, errors, results: results.map((r) => ({ label: r.label, fails: r.fails, seats: r.seats, filled: r.filled, codeFont: r.codeFont, titleFont: r.titleFont, titleLines: r.titleLines, ledeLines: r.ledeLines, qrLoaded: r.qrLoaded, decoded: r.decoded })) };
fs.writeFileSync(path.join(out, `${prefix}.json`), JSON.stringify({ ...summary, detail: results }, null, 2));
console.log(JSON.stringify(summary, null, 2));
await browser.close();
