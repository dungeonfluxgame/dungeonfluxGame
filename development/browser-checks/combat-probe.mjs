// usage: node combat-probe.mjs <base> <out> <pack.json>   - lists the enabled buttons of both phones through a fight
import { launch, session, click, create, settle, watch } from './lib.mjs';
const [base, out, packPath] = process.argv.slice(2);
const s = session(base, out, packPath);
const browser = await launch();
const tv = await (await browser.newContext({ viewport: { width: 1920, height: 1080 } })).newPage();
const phone = () => browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
const pa = await (await phone()).newPage();
const pb = await (await phone()).newPage();
for (const [n, p] of [['tv', tv], ['a', pa], ['b', pb]]) watch(s, p, n);
await tv.goto(`${base}/gameplay/display`);
await pa.goto(`${base}/gameplay/player`);
await pb.goto(`${base}/gameplay/player`);
await settle(2500);
await create(s, pa, 'Brakka', 'Copper beard');
await click(s, pa, 'Confirm your hero');
await create(s, pb, 'Thrain', 'Missing ear');
await click(s, pb, 'Confirm your hero');
await settle();
await click(s, pa, s.label('begin-story'));
await settle(5000);
await click(s, pa, s.label('ask-courier'));
await click(s, pa, s.label('defend-courier'));
const buttons = (p) => p.evaluate(() => [...document.querySelectorAll('button:not([disabled])')].map((b) => b.textContent.trim().replace(/\s+/g, ' ')).filter((t) => t && t.length < 60));
for (let i = 0; i < 6; i += 1) {
  s.note(`t${i} A: ${JSON.stringify(await buttons(pa))}`);
  s.note(`t${i} B: ${JSON.stringify(await buttons(pb))}`);
  await settle(3000);
}
await pa.screenshot({ path: `${out}/probe-a.png`, fullPage: true });
await browser.close();
