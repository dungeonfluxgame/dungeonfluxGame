// PLAY-F06 acceptance (1): the opening scene. The narrator box opens with the narrator's clip and the
// character's bubble opens only when the character's clip starts; the choice chips are visible only
// once every line has been said. Asserts on the exposed machine states (data-scene-state,
// data-line-state, data-speaker) and on recorded audio events, never on sleeps.
// usage: node stage-opening.mjs [frames-dir]      (own server, own PostgreSQL, fake voice)
import fs from 'node:fs';
import path from 'node:path';
import { runner, startServer, launchGpu, playToOpening, click, settle, states, until, events, clearEvents, framesDir } from './stage.mjs';

const frames = process.argv[2] || framesDir;
const { s, out, check, finish } = runner('stage-opening', frames);
const base = process.env.DF_BASE;
const { runPack } = await startServer();
if (fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
const browser = await launchGpu(false);
const { tv, pa } = await playToOpening(s, browser, { base, sound: true, voice: { latencyMs: 200, lengthFor: () => 3.5 } });
if (process.env.DF_DEBUG) tv.on('console', (m) => s.note(`tv console ${m.type()}: ${m.text().slice(0, 300)}`));
await clearEvents(tv);
await click(s, pa, s.label('begin-story'), { wait: 100 });

// Sample the chips against the scene state while the scene plays out.
const samples = [];
const stop = Date.now() + 45000;
let frame = 0;
while (Date.now() < stop) {
  const now = await states(tv);
  samples.push({ t: Date.now(), ...now });
  if (process.env.DF_DEBUG && samples.length % 25 === 0) s.note(`state ${JSON.stringify({ scene: now.scene, line: now.line, stage: now.stage, pending: now.pending, speaker: now.speaker })}`);
  if (now.narrator === 'speaking' && frame === 0) { frame = 1; await tv.screenshot({ path: `${out}-1-narrator-speaking.png` }); }
  if (now.bubbles.some((b) => b.kind === 'speech' && b.state === 'speaking') && frame === 1) { frame = 2; await tv.screenshot({ path: `${out}-2-courier-speaking.png` }); }
  if (now.scene === 'awaiting-choice' && now.line === 'lingering') break;
  await settle(120);
}
await tv.screenshot({ path: `${out}-3-awaiting-choice.png` });
s.note(`final state ${JSON.stringify(await states(tv))}`);
if (process.env.DF_PROBE) {
  const probe = await tv.evaluate(() => new Promise((resolve) => {
    const t0 = performance.now();
    const out = { hidden: document.hidden, visibility: document.visibilityState, focus: document.hasFocus() };
    const guard = setTimeout(() => resolve({ ...out, timerFired: false, guard: performance.now() - t0 }), 4000);
    requestAnimationFrame(() => { out.rafMs = Math.round(performance.now() - t0); });
    setTimeout(() => { out.timerMs = Math.round(performance.now() - t0); }, 50);
    setTimeout(() => { clearTimeout(guard); resolve(out); }, 1500);
  }));
  s.note(`probe ${JSON.stringify(probe)}`);
}
const log = await events(tv);
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(`${out}-events.json`, JSON.stringify(log, null, 1));
const plays = log.filter((e) => e.type === 'audio' && e.event === 'playing');
const ended = log.filter((e) => e.type === 'audio' && e.event === 'ended');
const narratorOpening = log.find((e) => e.type === 'attr' && e.name === 'data-line-state' && e.cls.includes('tv-dm') && e.value === 'opening');
const courierBubble = log.find((e) => e.type === 'bubble+' && e.kind === 'speech');
check(plays.length >= 2, `two clips played (${plays.length})`);
check(Boolean(narratorOpening), 'the narrator box opened through its machine');
check(Boolean(courierBubble), 'the courier bubble opened through its machine');
if (plays.length >= 2 && narratorOpening && courierBubble) {
  const [first, second] = plays;
  const gapN = narratorOpening.t - first.t;
  const gapC = courierBubble.t - second.t;
  s.note(`narrator box opened ${gapN} ms after its clip started; courier bubble ${gapC} ms after its clip started`);
  check(gapN >= -60 && gapN <= 500, `narrator box opens as the narrator clip starts (${gapN} ms)`);
  check(gapC >= -60 && gapC <= 500, `courier bubble opens as the courier clip starts (${gapC} ms)`);
  check(courierBubble.t > (ended[0]?.t ?? Infinity) - 60, 'the courier bubble waits for the narrator clip to end');
  check(courierBubble.t > narratorOpening.t + 3000, 'the courier bubble opens after the narrator had its whole clip');
}
const order = (name, cls) => log.filter((e) => e.type === 'attr' && e.name === name && (!cls || e.cls.includes(cls))).map((e) => e.value);
const lineOrder = order('data-line-state', 'tv-dm');
s.note(`narrator box states: ${lineOrder.join(' > ')}`);
const fromOpening = lineOrder.slice(lineOrder.indexOf('opening'));
check(['opening', 'speaking', 'lingering', 'closing'].every((v, i) => fromOpening.indexOf(v) >= 0 && (i === 0 || fromOpening.indexOf(v) > fromOpening.indexOf(['opening', 'speaking', 'lingering', 'closing'][i - 1]))), 'narrator box goes opening > speaking > lingering > closing');
const sceneOrder = order('data-scene-state').filter((v, i, a) => i === 0 || v !== a[i - 1]);
s.note(`scene states: ${sceneOrder.join(' > ')}`);
check(sceneOrder.indexOf('entering') >= 0 && sceneOrder.indexOf('presenting') > sceneOrder.indexOf('entering') && sceneOrder.indexOf('awaiting-choice') > sceneOrder.indexOf('presenting'), 'scene goes entering > presenting > awaiting-choice');
const chipsEarly = samples.filter((m) => m.scene && m.scene !== 'awaiting-choice' && m.chipsVisible);
check(chipsEarly.length === 0, `choice chips are never visible before awaiting-choice (${chipsEarly.length} samples)`);
const last = await until(tv, (now) => now.scene === 'awaiting-choice' && now.chipsVisible, 3000).catch(() => samples[samples.length - 1]);
check(last.scene === 'awaiting-choice' && last.chips.length > 0 && last.chipsVisible, `chips visible now: ${JSON.stringify(last.chips)}`);
await finish(browser);
