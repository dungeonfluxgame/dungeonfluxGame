// PLAY-F06 acceptance (4): sound muted. The same order as with sound, on reading time: no clip is
// requested or played, the narrator's box opens first, the courier's bubble after the narrator's reading
// time, and the chips only at the end. usage: node stage-muted.mjs [frames-dir]
import fs from 'node:fs';
import { runner, startServer, launchGpu, playToOpening, click, settle, states, until, events, clearEvents, framesDir } from './stage.mjs';

const { s, out, check, finish } = runner('stage-muted', process.argv[2] || framesDir);
const { runPack } = await startServer();
if (fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
const browser = await launchGpu(false);
const voiceLog = [];
const { tv, pa } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: false, voice: { latencyMs: 100, lengthFor: () => 3, log: voiceLog } });
// Unlock, then mute: the state the owner described.
const control = tv.locator('.tv-voice-control');
await control.click();
await control.click();
await settle(300);
check((await control.getAttribute('data-voice-state')) === 'muted', 'the display is muted');
await clearEvents(tv);
await click(s, pa, s.label('begin-story'), { wait: 100 });
await settle(1500);
s.note(`phone A: ${(await pa.evaluate(() => document.body.innerText)).replace(/\s+/g, ' ').slice(0, 300)}`);
await pa.screenshot({ path: `${out}-0-phone.png` });
const samples = [];
const stop = Date.now() + 45000;
let shot = 0;
while (Date.now() < stop) {
  const now = await states(tv);
  samples.push({ t: Date.now(), ...now });
  if (now.narrator === 'speaking' && shot === 0) { shot = 1; await tv.screenshot({ path: `${out}-1-narrator-reading.png` }); }
  if (now.bubbles.some((b) => b.kind === 'speech' && b.state === 'speaking') && shot === 1) { shot = 2; await tv.screenshot({ path: `${out}-2-courier-reading.png` }); }
  if (now.scene === 'awaiting-choice' && now.line === 'lingering') break;
  await settle(120);
}
s.note(`final state ${JSON.stringify(await states(tv))}`);
const log = await events(tv);
fs.writeFileSync(`${out}-events.json`, JSON.stringify(log, null, 1));
check(voiceLog.length === 0, `no clip was requested while muted (${voiceLog.length})`);
check(log.filter((e) => e.type === 'audio').length === 0, 'no audio played');
const narrator = log.find((e) => e.type === 'attr' && e.name === 'data-line-state' && e.cls.includes('tv-dm') && e.value === 'opening');
const courier = log.find((e) => e.type === 'bubble+' && e.kind === 'speech');
check(Boolean(narrator && courier), 'both lines opened');
if (narrator && courier) {
  const text = samples.find((m) => m.narrator === 'speaking');
  const gap = courier.t - narrator.t;
  const narration = await tv.evaluate(() => document.querySelector('.tv-narration').textContent.trim());
  const reading = Math.min(7000, Math.max(2500, 1800 + 35 * [...narration].length));
  s.note(`narration ${[...narration].length} characters: reading time ${reading} ms; narrator>courier gap ${gap} ms`);
  check(gap >= reading && gap <= reading + 250 + 700 + 400, `the courier opens one reading time (${reading} ms) plus open and linger after the narrator (${gap} ms)`);
  check(Boolean(text), 'the narrator box showed its text while reading');
}
const chipsEarly = samples.filter((m) => m.scene && m.scene !== 'awaiting-choice' && m.chipsVisible);
check(chipsEarly.length === 0, 'chips never visible before awaiting-choice');
const final = await until(tv, (now) => now.scene === 'awaiting-choice' && now.chipsVisible, 3000).catch(() => null);
check(Boolean(final) && final.chips.length > 0, 'chips visible at the end');
await finish(browser);
