// PLAY-F06 acceptance (5): a fight started mid-speech, in a WebGPU-capable headless browser
// (chromium-headless-shell with --enable-unsafe-webgpu --use-angle=metal --enable-features=Vulkan,UseSkiaRenderer).
// Chosen behaviour: the line in progress FADES within FIGHT_LINE_FADE_MS (500 ms) and its clip stops; the
// battle machine then runs Entering (the battlefield's banner, initiative roll and roll-call) before any
// attack; the flat combat view never shows before the 3D battlefield. Asserts on data-battle-state,
// data-line-state, data-stage and sampled visibility. usage: node stage-battle.mjs [frames-dir]
import fs from 'node:fs';
import { runner, startServer, launchGpu, playToOpening, click, settle, states, until, events, clearEvents, framesDir } from './stage.mjs';

const FADE_BOUND_MS = 500 + 400;
const { s, out, check, finish } = runner('stage-battle', process.argv[2] || framesDir);
await startServer();
const labels = JSON.parse(fs.readFileSync(new URL('../../content/harbor-journey/pack.json', import.meta.url), 'utf8')).offer_labels;
Object.assign(s.labels, labels);
const browser = await launchGpu(true);
const { tv, pa, pb } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true, voice: { latencyMs: 200, lengthFor: () => 8 } });
const gpu = await tv.evaluate(async () => {
  if (!navigator.gpu) return { present: false };
  try { const adapter = await navigator.gpu.requestAdapter(); return { present: true, adapter: Boolean(adapter) }; } catch (error) { return { present: true, error: String(error) }; }
});
s.note(`WebGPU in the display browser: ${JSON.stringify(gpu)}`);
check(gpu.present && gpu.adapter, 'the browser offers a WebGPU adapter');

await click(s, pa, s.label('begin-story'), { wait: 100 });
await until(tv, (now) => now.narrator === 'speaking', 40000);
await click(s, pa, s.label('escort-courier'), { wait: 100 });
// Wait for the next scene to be speaking: the fight starts in the middle of its line.
await until(tv, (now) => now.title && now.title.includes('Oath') && (now.narrator === 'speaking' || now.bubbles.some((b) => b.state === 'speaking')), 90000);
await clearEvents(tv);
const samples = [];
const sample = async () => {
  const now = await states(tv);
  const extra = await tv.evaluate(() => {
    const visible = (el) => { if (!el) return false; const cs = getComputedStyle(el); return cs.visibility !== 'hidden' && cs.display !== 'none' && Number(cs.opacity) > 0.1; };
    const flat = document.querySelector('.tv-stage .live-screen');
    const field = document.querySelector('.battlefield');
    const hud = document.querySelector('.bf-hud');
    return { flat: visible(flat), field: Boolean(field) && visible(field), initiative: Boolean(hud && hud.hasAttribute('data-initiative')), t: Date.now() };
  });
  samples.push({ ...now, ...extra });
  return samples[samples.length - 1];
};
const clickedAt = Date.now();
await click(s, pa, s.label('defend-courier'), { wait: 0 });
s.note('fight started mid-speech');
let attacked = false;
let shot = 0;
const stop = Date.now() + 90000;
let sawRolling = false;
while (Date.now() < stop) {
  const now = await sample();
  if (now.battle === 'entering' && shot === 0) { shot = 1; await tv.screenshot({ path: `${out}-1-entering.png` }); }
  if (now.initiative && shot === 1) { shot = 2; await tv.screenshot({ path: `${out}-2-initiative.png` }); }
  if (now.battle && !['absent', 'entering'].includes(now.battle) && shot === 2) { shot = 3; await tv.screenshot({ path: `${out}-3-turn.png` }); }
  if (now.stage === 'rolling') { sawRolling = true; if (shot === 3) { shot = 4; await tv.screenshot({ path: `${out}-4-first-attack.png` }); } }
  if (sawRolling && now.stage === null && now.battle === 'turn-end') break;
  if (!sawRolling && now.battle === 'turn-start' && !attacked) {
    attacked = (await click(s, pa, 'Greatsword strike', { optional: true, wait: 0 })) || (await click(s, pb, 'Greatsword strike', { optional: true, wait: 0 }));
  }
  await settle(80);
}
const log = await events(tv);
fs.writeFileSync(`${out}-events.json`, JSON.stringify(log, null, 1));
fs.writeFileSync(`${out}-samples.json`, JSON.stringify(samples.map(({ t, battle, stage, flat, field, initiative, line, scene }) => ({ t, battle, stage, flat, field, initiative, line, scene })), null, 1));
const at = (predicate) => log.find(predicate)?.t;
const battle = (value) => at((e) => e.type === 'attr' && e.name === 'data-battle-state' && e.value === value);
const entering = battle('entering');
// When the enemy won initiative its held turn follows the entrance at once, so TurnStart may last no time at all.
const turnStart = battle('turn-start') ?? battle('acting');
const acting = battle('acting');
const rolling = at((e) => e.type === 'attr' && e.name === 'data-stage' && e.value === 'rolling');
s.note(`battle states: ${log.filter((e) => e.type === 'attr' && e.name === 'data-battle-state').map((e) => e.value).filter((v, i, a) => i === 0 || v !== a[i - 1]).join(' > ')}`);
check(Boolean(entering && turnStart && turnStart >= entering), 'the battle machine ran Entering and then began the first turn');
check(Boolean(acting && acting >= turnStart), 'Acting only after the entrance');
check(Boolean(rolling && turnStart && rolling >= turnStart), `the first attack (rolling) came after the entrance completed (${rolling - turnStart} ms after TurnStart)`);
// Line: faded within the bound, not cut abruptly and not held behind the fight.
const lineClosing = at((e) => (e.type === 'bubble~' || (e.type === 'attr' && e.name === 'data-line-state' && e.cls.includes('tv-dm'))) && e.value === 'closing' || e.state === 'closing');
s.note(`defend clicked at ${clickedAt}; line closing at ${lineClosing}`);
const closingAfter = log.filter((e) => (e.state === 'closing' || e.value === 'closing') && e.t >= clickedAt).map((e) => e.t)[0];
check(Boolean(closingAfter) && closingAfter - clickedAt <= 6000, `the line in progress closed through its machine when the fight began (${closingAfter ? closingAfter - clickedAt : 'never'} ms after the click; includes phone and server delay)`);
const stoppedClip = log.find((e) => e.type === 'audio' && e.event === 'pause' && e.t >= clickedAt);
check(Boolean(stoppedClip), 'the speaking clip was stopped for the fight');
// The flat view never shows before the battlefield while the entrance runs.
const fightSamples = samples.filter((m) => m.battle === 'entering' || m.battle === 'turn-start');
const flatBeforeField = fightSamples.filter((m) => m.flat && !m.field);
check(flatBeforeField.length === 0, `the flat combat view never showed before the 3D battlefield (${flatBeforeField.length} of ${fightSamples.length} samples)`);
const fieldSeen = samples.some((m) => m.field);
check(fieldSeen, '3D battlefield appeared');
const initiativeSeen = samples.filter((m) => m.initiative);
check(initiativeSeen.length > 0, 'the initiative banner and roll-call showed');
if (initiativeSeen.length && rolling) {
  const last = initiativeSeen[initiativeSeen.length - 1].t;
  check(rolling >= last - 100, `the first attack came after the initiative roll-call ended (${rolling - last} ms)`);
}
await finish(browser);
