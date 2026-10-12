// CAPTION-001 (Part 2): the voice queue must not stall. Plays a test fight end to end with
// the provider off (fake silent clips): its opening, fight and victory lines speak in a row,
// later ones requested while earlier clips are still playing or lingering — the pattern the
// live session stalled in. Two faults are injected through the media element: the 3rd play
// rejects with NotAllowedError (a raced cancel, a backgrounded tab or a busy device refusing
// mid-session; before the fix this re-locked sound for the rest of the session), and every
// play from the 6th on hangs forever (clips that never report started/ended/failed). Asserts
// the queue keeps speaking past both: fetches continue (request count), sound stays on, the
// overruns are released and counted (stall counter), and later clips play — including after
// the last hang, which nothing supersedes. (Forty voiced lines need the paid text model —
// replies never come with the storyteller off — so volume beyond one adventure's lines is
// covered by the native conductor/voice tests and the counters here.)
// usage: PORT=63379 DF_CHECK_PG_PORT=55587 node stage-voice-stall.mjs [frames-dir]
import fs from 'node:fs';
import path from 'node:path';
import { runner, launchGpu, playToOpening, click, settle, states, until, events, clearEvents, root } from './stage.mjs';
import { options, choose } from './chapter-lib.mjs';
import { startLocal, stopLocal } from './serve-local.mjs';

const PORT = process.env.PORT || '63379';
process.env.DF_BASE = `http://127.0.0.1:${PORT}`;
const framesDir = process.argv[2] || path.join(root, 'artifacts', 'tmp', 'caption-a1', 'frames');
fs.mkdirSync(framesDir, { recursive: true });
const { s, out, check, finish } = runner('stage-voice-stall', framesDir);

const { runPack } = await startLocal({ DF_GAMEPLAY_TEST_FIGHT: '1', DF_GAMEPLAY_ADVENTURE: 'harbor' });
for (let i = 0; i < 60 && !fs.existsSync(runPack); i += 1) await settle(500);
if (fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
const browser = await launchGpu(true);
const voiceLog = [];
const { tv, pa, pb } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true, voice: { latencyMs: 50, lengthFor: () => 1, log: voiceLog } });
await click(s, pa, s.label('begin-story'), { wait: 100 });
await until(tv, (now) => now.scene === 'awaiting-choice', 120000);
// Let the opening lines speak before the walk starts: the first fetches must go out here.
await until(tv, (now) => voiceLog.length >= 1 && (now.line === 'lingering' || now.line === 'idle'), 60000).catch(() => null);

// Faults through the media element: refuse the 3rd voiced play with NotAllowedError, hang
// every play from the 6th on forever. Both strike after clips have already played; the last
// hang is never superseded, so the watchdog must release and count it.
await tv.evaluate(() => {
  window.__blobPlays = 0;
  const proto = window.HTMLMediaElement.prototype;
  if (proto.__captionWrapped) return;
  proto.__captionWrapped = true;
  const orig = proto.play;
  proto.play = function () {
    if (String(this.src || '').startsWith('blob:')) {
      window.__blobPlays += 1;
      if (window.__blobPlays === 3) return Promise.reject(new DOMException('interrupted', 'NotAllowedError'));
      if (window.__blobPlays >= 6) return new Promise(() => {});
    }
    return orig.call(this);
  };
});
await clearEvents(tv);

// Walk test-fight cycles (chaining into the next adventure when one ends) until forty voice
// fetches have gone out: dozens of voiced lines in a row, each scene's lines requested while
// the last scene's clips are still playing or lingering. Paced: every choice waits for the
// current lines to rest, never outrunning the voice the way a blind fast walk does.
const seenIds = new Set();
const table = { pa, pb };
const SKIP = /^Short Rest|^Say something|^Show choices|^Ready\. |^Starting/;
async function atRest(timeoutMs) {
  const stop = Date.now() + timeoutMs;
  for (;;) {
    const now = await states(tv);
    if (now.lineId) seenIds.add(now.lineId);
    if ((now.line === 'lingering' || now.line === 'idle') && now.pending === 'false') return now;
    if (Date.now() > stop) return null;
    await settle(300);
  }
}
let ended = false;
const stop = Date.now() + 15 * 60000;
for (;;) {
  if (/^the end$/im.test(await tv.locator('body').innerText())) { ended = true; break; }
  if (Date.now() > stop) break;
  // Recovery offers are not story: never press them into the walk (they loop).
  const pick = choose(
    (await options(table)).filter((o) => !/Confirm original strike|Confirm saved choice/.test(o.label)),
    SKIP,
  );
  if (!pick) { await settle(500); continue; }
  s.note(`presses (${pick.who}) "${pick.label}" (requests=${voiceLog.length})`);
  try { await pick.page.locator('button:not([disabled])', { hasText: pick.label }).first().click({ timeout: 4000 }); } catch { s.note(`click missed "${pick.label}"`); }
  await atRest(45000);
}
await atRest(120000);
// The last hang is never superseded: give its 15 s watchdog room to release and count it.
await tv.waitForFunction(() => (document.querySelector('.tv-voice-control')?.getAttribute('data-voice-stalls') ?? '0') !== '0', { timeout: 60000 }).catch(() => null);
const log = await events(tv);
fs.writeFileSync(`${out}-events.json`, JSON.stringify(log, null, 1));
const plays = log.filter((e) => e.type === 'audio' && e.event === 'playing').length;
const control = tv.locator('.tv-voice-control');
const voiceState = await control.getAttribute('data-voice-state');
const stalls = await control.getAttribute('data-voice-stalls');
s.note(`voice requests: ${voiceLog.length}, clips played: ${plays}, distinct line ids: ${seenIds.size}, state=${voiceState}, stalls=${stalls}${ended ? ' (adventure ended)' : ''}`);
check(ended, 'the adventure played to its end card');
check(voiceLog.length >= 6, `every line fetched past both faults (${voiceLog.length} requests)`);
check(voiceState === 'on', `sound stayed on through refusal and overrun (${voiceState})`);
check(plays >= 2 && plays <= voiceLog.length - 2, `clips played around the faults (${plays} heard of ${voiceLog.length} requests)`);
check(seenIds.size >= 6, `lines opened in a row (${seenIds.size} distinct line ids)`);
check(Number(stalls) >= 1, `the overruns were released and counted (${stalls})`);
// Clips kept playing after the faults: play calls went on past the refusal and the hangs.
const playsAfterStall = await tv.evaluate(() => window.__blobPlays || 0);
check(playsAfterStall >= 6, `play calls continued past every fault (${playsAfterStall} calls)`);
await stopLocal();
await finish(browser);
