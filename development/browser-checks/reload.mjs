// usage: node reload.mjs <base> <out> <pack.json> [reloads=10]
// PLAY-F05 defect 4: reload the display ten times and a player page ten times (also all three at
// once) and end connected. Reports the connection state of every page after every round.
import { launch, session, click, create, settle, watch } from './lib.mjs';
const [base, out, packPath, reloadsArg] = process.argv.slice(2);
const reloads = Number(reloadsArg || 10);
const s = session(base, out, packPath);
const browser = await launch();
const tvCtx = await browser.newContext({ viewport: { width: 1280, height: 720 } });
const phone = () => browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
const tv = await tvCtx.newPage();
const ctxA = await phone();
const ctxB = await phone();
const pa = await ctxA.newPage();
const pb = await ctxB.newPage();
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
await settle(4000);

const state = async (page) => page.evaluate(() => {
  const root = document.querySelector('#display, #player');
  return {
    phase: root?.getAttribute('data-phase') ?? null,
    reconnecting: !!document.querySelector('.df-reconnecting'),
    stage: !!document.querySelector('.tv-stage'),
    feedback: document.querySelector('.feedback')?.textContent?.trim() ?? null,
    text: document.body.innerText.replace(/\s+/g, ' ').slice(0, 70),
  };
});
const connected = (st, kind) => !!st.phase && !st.reconnecting && (kind !== 'tv' || st.stage);
const wait = async (page, kind, ms = 20000) => {
  const until = Date.now() + ms;
  let st;
  while (Date.now() < until) {
    st = await state(page);
    if (connected(st, kind)) return { ok: true, st };
    await settle(500);
  }
  return { ok: false, st };
};
let failures = 0;
const report = (label, kind, r) => {
  if (!r.ok) failures += 1;
  s.note(`${r.ok ? 'ok  ' : 'FAIL'} ${label} ${kind} ${JSON.stringify(r.st)}`);
};
report('baseline', 'tv', await wait(tv, 'tv'));
report('baseline', 'a', await wait(pa, 'a'));
report('baseline', 'b', await wait(pb, 'b'));
for (let i = 1; i <= reloads; i += 1) {
  await tv.reload();
  report(`display reload ${i}`, 'tv', await wait(tv, 'tv'));
}
for (let i = 1; i <= reloads; i += 1) {
  await pa.reload();
  report(`player A reload ${i}`, 'a', await wait(pa, 'a'));
}
for (let i = 1; i <= 4; i += 1) {
  await Promise.all([tv.reload(), pa.reload(), pb.reload()]);
  report(`all-at-once ${i}`, 'tv', await wait(tv, 'tv'));
  report(`all-at-once ${i}`, 'a', await wait(pa, 'a'));
  report(`all-at-once ${i}`, 'b', await wait(pb, 'b'));
}
// rapid fire: reload without waiting for the page to settle
for (let i = 0; i < 12; i += 1) { await Promise.all([tv.reload({ waitUntil: 'commit' }), pa.reload({ waitUntil: 'commit' })]); await settle(150); }
report('after rapid reloads', 'tv', await wait(tv, 'tv'));
report('after rapid reloads', 'a', await wait(pa, 'a'));
s.note(failures === 0 ? 'PASS every page ended connected' : `FAIL ${failures} checks`);
(await import('node:fs')).writeFileSync(`${out}/reload-log.txt`, s.log.join('\n') + '\n');
await browser.close();
process.exit(failures === 0 ? 0 : 1);
