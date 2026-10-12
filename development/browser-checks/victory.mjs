// usage: node victory.mjs <base> <out> <pack.json>
// PLAY-F05 defects 2 and 6. (2) the sound control: one click enables sound, a mute is kept only inside the page
// session, a reload comes back locked and the first click enables sound again. (6) the display's music state log
// across a won fight: the Release phase holds ~14 s before the calm bed.
import fs from 'node:fs';
import { launch, session, click, create, settle, watch } from './lib.mjs';
const [base, out, packPath] = process.argv.slice(2);
const s = session(base, out, packPath);
let failures = 0;
const check = (ok, m) => { if (!ok) failures += 1; s.note(`${ok ? 'ok  ' : 'FAIL'} ${m}`); };
const browser = await launch();
const tvCtx = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
const phone = () => browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
const tv = await tvCtx.newPage();
const pa = await (await phone()).newPage();
const pb = await (await phone()).newPage();
for (const [n, p] of [['tv', tv], ['a', pa], ['b', pb]]) watch(s, p, n);
await tv.addInitScript(() => {
  window.__music = [];
  const record = () => {
    const root = document.querySelector('#display');
    const raw = root?.getAttribute('data-music-state');
    if (!raw) return;
    try {
      const m = JSON.parse(raw);
      const last = window.__music[window.__music.length - 1];
      const row = { t: Math.round(performance.now()), sound: m.sound, phase: m.phase, intensity: m.intensity, cue: m.cue, master: m.master };
      if (!last || last.sound !== row.sound || last.phase !== row.phase || last.cue !== row.cue) window.__music.push(row);
    } catch { /* not ready */ }
  };
  setInterval(record, 100);
});
const voiceState = () => tv.evaluate(() => document.querySelector('.tv-voice-control')?.getAttribute('data-voice-state'));
await tv.goto(`${base}/gameplay/display`);
await pa.goto(`${base}/gameplay/player`);
await pb.goto(`${base}/gameplay/player`);
await settle(2500);

// ---- defect 2: the sound control ----
check((await voiceState()) === 'locked', `display starts locked (${await voiceState()})`);
await tv.locator('.tv-voice-control').click();
await settle(500);
check((await voiceState()) === 'on', `one click enables sound (${await voiceState()})`);
const label = await tv.locator('.tv-voice-control').getAttribute('aria-label');
check(label === 'Mute narration', `the control now offers to mute (${label})`);
await tv.locator('.tv-voice-control').click();
await settle(300);
check((await voiceState()) === 'muted', `a second click mutes inside the page session (${await voiceState()})`);
await tv.reload();
await settle(2500);
check((await voiceState()) === 'locked', `a reload comes back locked, not muted (${await voiceState()})`);
await tv.locator('.tv-voice-control').click();
await settle(500);
check((await voiceState()) === 'on', `the first click after the reload enables sound (${await voiceState()})`);
const stored = await tv.evaluate(() => JSON.stringify({ ...sessionStorage, ...localStorage }));
check(!stored.includes('voice.muted'), `no mute choice is stored across reloads (${stored})`);

// ---- play to a win ----
await create(s, pa, 'Brakka', 'Copper beard');
await click(s, pa, 'Confirm your hero');
await create(s, pb, 'Thrain', 'Missing ear');
await click(s, pb, 'Confirm your hero');
await settle();
await click(s, pa, s.label('begin-story'));
await settle(5000);
await click(s, pa, s.label('ask-courier'));
await click(s, pa, s.label('defend-courier'));
let won = false;
for (let turn = 0; turn < 60 && !won; turn += 1) {
  const done = async (page) => (await page.locator('button:not([disabled])', { hasText: s.label('short-rest') }).count()) + (await page.locator('button:not([disabled])', { hasText: s.label('choose-harbor-scene') }).count());
  if ((await done(pa)) + (await done(pb)) > 0) { won = true; break; }
  let acted = false;
  for (const page of [pa, pb]) {
    if (await page.locator('button:not([disabled])', { hasText: s.label('greatsword-attack') }).count()) {
      await click(s, page, s.label('greatsword-attack'), { wait: 800 });
      await click(s, page, s.label('end-turn'), { optional: true, wait: 800 });
      acted = true;
    } else if (await page.locator('button:not([disabled])', { hasText: s.label('end-turn') }).count()) {
      await click(s, page, s.label('end-turn'), { wait: 800 }); acted = true;
    }
  }
  if (!acted) await settle(1200);
}
const wonAt = await tv.evaluate(() => Math.round(performance.now()));
s.note(`victory reached: ${won} at t=${wonAt}`);
await settle(24000);
const log = await tv.evaluate(() => window.__music);
fs.writeFileSync(`${out}/music-log.json`, JSON.stringify(log, null, 1));
for (const row of log) s.note(`music t=${row.t} sound=${row.sound} phase=${row.phase} intensity=${row.intensity} cue=${row.cue}`);
const release = log.filter((row) => row.phase === 'release');
if (release.length) {
  const start = release[0].t;
  const after = log.find((row) => row.t > start && row.phase !== 'release');
  const held = after ? after.t - start : null;
  s.note(`release began t=${start}; next phase ${after?.phase} at ${held} ms later`);
  check(held === null || held >= 12000, `the release is held about 14 s before the calm bed (${held} ms)`);
} else {
  check(false, 'no release phase was logged');
}
await tv.screenshot({ path: `${out}/victory-tv.png` });
fs.writeFileSync(`${out}/victory-log.txt`, s.log.join('\n') + '\n');
await browser.close();
process.exit(failures === 0 ? 0 : 1);
