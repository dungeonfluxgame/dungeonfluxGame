// PLAY-B03: a creation form never holds an offer older than the view it is drawn in. Two phones create heroes at the
// same time: phone 1 confirms with one group unselected and is refused; phone 2 confirms (the room moves on); phone 1
// then completes the missing group and confirms again. It must succeed without a reload (before the fix every press
// returned "That offer has ended"). Also asserts the appearance list follows the package: with the archer package only
// the bow appearance is listed, with a greatsword package only the sword appearances.
// usage: PORT=63711 DF_CHECK_PG_PORT=55711 node play-b03-create-stale.mjs [frames-dir]
import fs from 'node:fs';
import path from 'node:path';
import { runner, startServer, launchGpu, click, settle, root } from './stage.mjs';

const PORT = process.env.PORT || '63711';
process.env.DF_BASE = `http://127.0.0.1:${PORT}`;
const framesDir = process.argv[2] || path.join(root, '..', '..', '..', 'artifacts', 'tmp', 'play-b03', 'create-stale');
fs.mkdirSync(framesDir, { recursive: true });
const { s, out, check, finish } = runner('create-stale', framesDir);
await startServer({ DF_GAMEPLAY_ADVENTURE: 'harbor', PORT });
const browser = await launchGpu(false);
const phone = async () => (await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true })).newPage();
const [p1, p2] = [await phone(), await phone()];
for (const [name, page] of [['p1', p1], ['p2', p2]]) page.on('pageerror', (error) => s.note(`pageerror ${name}: ${String(error).slice(0, 300)}`));
await p1.goto(`${process.env.DF_BASE}/gameplay/player`);
await p2.goto(`${process.env.DF_BASE}/gameplay/player`);
await settle(2500);
for (const page of [p1, p2]) {
  await click(s, page, 'Join this room');
  await page.waitForSelector('button[aria-pressed]', { timeout: 15000 });
}
const setName = (page, name) => page.evaluate((n) => {
  const set = (el, v) => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, v); el.dispatchEvent(new Event('input', { bubbles: true })); el.dispatchEvent(new Event('change', { bubbles: true })); };
  set([...document.querySelectorAll('input[type=text],input:not([type])')].filter((i) => !i.readOnly)[0], n);
}, name);
// Picks the first option of every group (the archer package where asked), leaving out groups whose heading matches `skip`.
const pick = (page, { archer = false, skip = null } = {}) => page.evaluate(([archer, skip]) => {
  const used = new Set();
  for (const h of [...document.querySelectorAll('h2')]) {
    const buttons = [...h.parentElement.querySelectorAll('button[aria-pressed]')];
    if (!buttons.length) continue;
    const heading = h.textContent.trim();
    if (skip && heading.includes(skip)) continue;
    const key = heading.includes('language') ? 'lang' : heading.includes('skill') ? 'skill' : heading;
    let b = buttons.find((x) => !used.has(key + x.textContent)) || buttons[0];
    if (heading.startsWith('Combat training') && archer) b = buttons.find((x) => /Longbow/.test(x.textContent)) || b;
    used.add(key + b.textContent);
    b.click();
  }
}, [archer, skip]);
const appearances = (page) => page.evaluate(() => {
  const h = [...document.querySelectorAll('h2')].find((x) => x.textContent.trim().startsWith('Hero appearance'));
  return h ? [...h.parentElement.querySelectorAll('.character-option')].filter((c) => !c.hidden).map((c) => c.querySelector('button').textContent.trim()) : [];
});
await setName(p1, 'Brakka');
await setName(p2, 'Sylvie');
await pick(p1, { skip: 'Gaming set' });
await settle(500);
const swordList = await appearances(p1);
s.note(`appearances with a greatsword package: ${JSON.stringify(swordList)}`);
check(swordList.length > 0 && !swordList.includes('Woman with a bow'), 'only sword appearances are listed with a greatsword package');
await pick(p2, { archer: true });
await settle(500);
const bowList = await appearances(p2);
s.note(`appearances with the archer package: ${JSON.stringify(bowList)}`);
check(bowList.length === 1 && bowList[0] === 'Woman with a bow', 'only the bow appearance is listed with the archer package');
await p1.screenshot({ path: `${out}-p1-before.jpg`, type: 'jpeg', quality: 60 });
// Phone 1 confirms with a group missing: refused.
await click(s, p1, 'Confirm your hero', { wait: 2500 });
const refusal = await p1.evaluate(() => document.body.innerText);
s.note(`p1 after the first confirm: ${(refusal.match(/rejected[^.]*\./) || refusal.match(/Choose[^.]*\./) || ['(no message)'])[0]}`);
// Phone 2 confirms: the room moves on.
await click(s, p2, 'Confirm your hero', { wait: 3000 });
const p2Done = await p2.evaluate(() => !/Confirm your hero/.test(document.body.innerText));
check(p2Done, 'phone 2 created its hero');
// Phone 1 completes the group and confirms again, without a reload.
await pick(p1, {});
await settle(400);
await click(s, p1, 'Confirm your hero', { wait: 3500 });
const after = await p1.evaluate(() => document.body.innerText);
check(!/That offer has ended/.test(after), 'phone 1 was not told its offer had ended');
check(!/Confirm your hero/.test(after), 'phone 1 created its hero without a reload');
await p1.screenshot({ path: `${out}-p1-after.jpg`, type: 'jpeg', quality: 60 });
await finish(browser);
