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
      const row = { t: Math.round(performance.now()), sound: m.sound, phase: m.phase, intensity: m.intensity, cue: m.cue, master: m.master, gains: JSON.stringify(m.gains) };
      if (!last || last.sound !== row.sound || last.phase !== row.phase || last.cue !== row.cue || last.gains !== row.gains) window.__music.push(row);
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
s.note('creating hero A');
await create(s, pa, 'Brakka', 'Copper beard');
s.note('hero A created');
await click(s, pa, 'Confirm your hero');
await create(s, pb, 'Thrain', 'Missing ear');
await click(s, pb, 'Confirm your hero');
await settle();
await click(s, pa, s.label('begin-story'));
await settle(5000);
await click(s, pa, s.label('ask-courier'));
await click(s, pa, s.label('defend-courier'));
let won = false;
const fightEnd = Date.now() + 9 * 60 * 1000;
for (let turn = 0; turn < 60 && !won && Date.now() < fightEnd; turn += 1) {
  s.note(`fight turn ${turn}: ${(await tv.evaluate(() => document.body.innerText.replace(/\s+/g, ' ').slice(0, 90)))}`);
  // The fight is over when the display's music enters its Release.
  won = await tv.evaluate(() => (window.__music || []).some((row) => row.phase === 'release'));
  if (won) break;
  let acted = false;
  for (const page of [pa, pb]) {
    for (const pattern of [/Greatsword strike/, /End your turn/]) {
      const button = page.locator('button:not([disabled])', { hasText: pattern }).first();
      if (await button.count()) { await button.click(); await settle(800); acted = true; break; }
    }
  }
  if (!acted) await settle(1200);
}
const wonAt = await tv.evaluate(() => Math.round(performance.now()));
s.note(`victory reached: ${won} at t=${wonAt}`);
await settle(24000);
const log = await tv.evaluate(() => window.__music);
fs.writeFileSync(`${out}/music-log.json`, JSON.stringify(log, null, 1));
for (const row of log) s.note(`music t=${row.t} sound=${row.sound} phase=${row.phase} intensity=${row.intensity} cue=${row.cue} gains=${row.gains}`);
// The cue's phase is the server's; the mix settles to the calm bed when the release hold has passed.
const release = log.find((row) => row.phase === 'release');
// The raw cue stays Release; the mix leaves the resolving motif (the next change of gains) when the hold ends.
const calmGains = release?.gains;
if (release && calmGains) {
  const settled = log.find((row) => row.t > release.t && row.gains !== calmGains);
  const held = settled ? settled.t - release.t : null;
  s.note(`release began t=${release.t}; the mix is back at the calm bed ${held === null ? 'not within the window' : `${held} ms later`}`);
  check(held === null || held >= 12000, `the release is held about 14 s before the calm bed (${held} ms)`);
  check(held !== null, 'the mix did settle to the calm bed after the hold');
} else {
  check(false, 'no release phase or calm bed was logged');
}
await tv.screenshot({ path: `${out}/victory-tv.png` });
fs.writeFileSync(`${out}/victory-log.txt`, s.log.join('\n') + '\n');
await browser.close();
process.exit(failures === 0 ? 0 : 1);
