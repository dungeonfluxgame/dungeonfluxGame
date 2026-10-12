// usage: node converse.mjs <base> <out> <pack.json> [questions=1] [room]
// Plays to the first scene with a speaker, types lines to the NPC and logs what the TV shows over time.
import { launch, session, click, create, settle, watch, tvState } from './lib.mjs';
const [base, out, packPath, questionsArg, room] = process.argv.slice(2);
const questions = Number(questionsArg || 1);
const s = session(base, out, packPath);
const browser = await launch();
const tvCtx = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
const phone = () => browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
const tv = await tvCtx.newPage();
const pa = await (await phone()).newPage();
const pb = await (await phone()).newPage();
for (const [n, p] of [['tv', tv], ['a', pa], ['b', pb]]) watch(s, p, n);
const q = room ? `?room=${room}` : '';
await tv.goto(`${base}/gameplay/display`);
await pa.goto(`${base}/gameplay/player${q}`);
await pb.goto(`${base}/gameplay/player${q}`);
await settle(2500);
await tv.screenshot({ path: `${out}/00-tv-lobby.png` });
await create(s, pa, 'Brakka', 'Copper beard');
await click(s, pa, 'Confirm your hero');
await create(s, pb, 'Thrain', 'Missing ear');
await click(s, pb, 'Confirm your hero');
await settle();
await click(s, pa, s.label('begin-story'));
await settle(9000);
await tv.screenshot({ path: `${out}/01-tv-opening.png` });
const start = Date.now();
const timeline = [];
let last = '';
const poll = async (label, ms) => {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    const st = await tvState(tv);
    const key = JSON.stringify(st);
    if (key !== last) {
      last = key;
      const line = `${((Date.now() - start) / 1000).toFixed(1)}s ${label} ${key}`;
      timeline.push(line);
      await tv.screenshot({ path: `${out}/t${String(timeline.length).padStart(2, '0')}-${label}.png` });
      s.note(line);
    }
    await settle(250);
  }
};
const hasPanel = await pa.locator('textarea.dialogue-draft').count();
s.note(`dialogue panel on A: ${hasPanel}`);
if (!hasPanel) {
  await pa.screenshot({ path: `${out}/02-phone-nopanel.png`, fullPage: true });
  process.exit(2);
}
for (let i = 0; i < questions; i += 1) {
  const text = i === 0 ? (process.env.QTEXT || 'Innkeeper, what news of the road tonight?') : 'And who has been asking after the courier?';
  await pa.evaluate(() => { const d = document.querySelector('details.say-more'); if (d) d.open = true; });
  await pa.selectOption('select[aria-label="Input context"]', { label: 'Conversation' });
  await pa.fill('textarea.dialogue-draft', text);
  await pa.locator('[data-dialogue=submit]').click();
  s.note(`submitted: ${text}`);
  const submitted = Date.now();
  await poll('after-submit', 3000);
  await tv.screenshot({ path: `${out}/10-tv-q${i}-hero.png` });
  await poll('wait-reply', 22000);
  await tv.screenshot({ path: `${out}/11-tv-q${i}-reply.png` });
  await pa.screenshot({ path: `${out}/12-phone-q${i}.png`, fullPage: true });
}
// advance to the next beat
const labels = Object.entries(s.labels);
s.note(`labels: ${labels.map(([k]) => k).join(',')}`);
await poll('idle', 4000);
await click(s, pa, s.label('ask-courier'), { optional: true });
await poll('next-beat', 9000);
await tv.screenshot({ path: `${out}/20-tv-nextbeat.png` });
await pa.screenshot({ path: `${out}/21-phone-nextbeat.png`, fullPage: true });
(await import('node:fs')).writeFileSync(`${out}/timeline.txt`, s.log.join('\n') + '\n');
await browser.close();
