// PLAY-F06: a whole fight's entrance and exit, on a real splat scan (scan-village, a real multi-second load) in a WebGPU
// browser, sampled every ~100 ms with a screenshot strip. FIGHT=enemy: the goblin wins initiative (its dexterity is
// the maximum), so the view that starts the fight already carries its resolved first attack; FIGHT=hero: a hero wins.
// The bandit has 1 hit point and AC 1 so the first hero strike ends the fight (a miss swings again). Asserts:
// (a) no combat sound, dice card or turn beat while the cover is up or the splat is not complete; (b) the initiative
// phase is on screen for its full duration (>= 2.5 s with the roll-call); (c) at least 2.5 s pass between the killing
// strike's dice and the exit transition, the battlefield being the visible screen throughout; (d) the victory card
// never coexists with a combat dice card or bubble; (e) exactly one scene-transition on entry and one on exit.
// usage: FIGHT=enemy|hero node stage-fight.mjs [frames-dir]    (needs artifacts/tmp/splat-dl)
import fs from 'node:fs';
import path from 'node:path';
import { runner, startServer, launchGpu, playToOpening, click, settle, until, clearEvents, framesDir, artifactRoot, root } from './stage.mjs';

const FIGHT = process.env.FIGHT || 'enemy';
const SCAN_DIR = process.env.DF_SPLAT_DIR || path.join(root, '..', '..', '..', 'artifacts', 'tmp', 'splat-dl');
const COMBAT_SFX = new Set(['dice-rattle', 'dice-land', 'hit', 'miss', 'critical', 'step-dirt-1', 'step-dirt-2', 'step-stone-1', 'step-stone-2', 'turn-chime']);
const { s, out, check, finish } = runner(`stage-fight-${FIGHT}`, process.argv[2] || framesDir);
const pack = path.join(artifactRoot, 'tmp', 'play-f06', `fight-pack-${FIGHT}`);
fs.rmSync(pack, { recursive: true, force: true });
fs.cpSync(path.join(root, 'content', 'harbor-journey'), pack, { recursive: true });
const tuning = JSON.parse(fs.readFileSync(path.join(pack, 'tuning.json'), 'utf8'));
tuning.bandit.hit_points = 1;
tuning.bandit.armor_class = 1;
tuning.bandit.abilities.dexterity = FIGHT === 'enemy' ? 20 : 1;
fs.writeFileSync(path.join(pack, 'tuning.json'), JSON.stringify(tuning, null, 2));
await startServer({ DF_GAMEPLAY_PACK_DIR: pack, DF_GAMEPLAY_BATTLE_SCENE: 'scan-village', DF_GAMEPLAY_SPLAT_DIR: SCAN_DIR });
Object.assign(s.labels, JSON.parse(fs.readFileSync(path.join(pack, 'pack.json'), 'utf8')).offer_labels);
const browser = await launchGpu(true);
const { tv, pa, pb } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true, voice: { latencyMs: 100, lengthFor: () => 1 } });
await click(s, pa, s.label('begin-story'), { wait: 100 });
await until(tv, (now) => now.scene === 'awaiting-choice', 60000);
await click(s, pa, s.label('escort-courier'), { wait: 100 });
await until(tv, (now) => now.title && now.title.includes('Oath') && now.scene === 'awaiting-choice', 60000);
await settle(1500);

const samples = [];
const sample = async () => {
  const state = await tv.evaluate(() => {
    const stage = document.querySelector('.tv-stage');
    const visible = (el) => { if (!el) return false; const cs = getComputedStyle(el); return cs.visibility !== 'hidden' && cs.display !== 'none' && Number(cs.opacity) > 0.1; };
    const field = document.querySelector('.battlefield');
    const hud = document.querySelector('.bf-hud');
    return {
      t: Date.now(), sfx: document.documentElement.getAttribute('data-sfx-log') || '',
      battle: stage?.getAttribute('data-battle-state'), scene: stage?.getAttribute('data-scene-state'), stage: stage?.getAttribute('data-stage'),
      splat: document.documentElement.getAttribute('data-battlefield-splat') || document.querySelector('[data-battlefield-splat]')?.getAttribute('data-battlefield-splat') || null,
      field: Boolean(field) && visible(field), initiative: Boolean(hud && hud.hasAttribute('data-initiative')),
      dice: visible(document.querySelector('.tv-final-roll')) || visible(document.querySelector('.combat-roll-panel')),
      aftermath: visible(document.querySelector('.live-screen[data-screen=aftermath]')),
      actionBubbles: [...document.querySelectorAll('.tv-bubble[data-kind=action]')].filter(visible).length,
    };
  });
  samples.push(state);
  return state;
};
const clickedAt = Date.now();
await click(s, pa, s.label('defend-courier'), { wait: 0 });
let frame = 0;
let lastSwing = 0;
let settled = null;
const stop = Date.now() + 300000;
while (Date.now() < stop) {
  const now = await sample();
  if (frame < 600) { await tv.screenshot({ path: `${out}-strip-${String(frame).padStart(3, '0')}.jpg`, type: 'jpeg', quality: 55 }); frame += 1; }
  if (!now.aftermath && !now.stage && now.battle && ['turn-start', 'turn-end'].includes(now.battle) && Date.now() - lastSwing > 7000) {
    const swung = (await click(s, pa, 'Attack', { optional: true, wait: 0 })) || (await click(s, pb, 'Attack', { optional: true, wait: 0 })) || (await click(s, pa, 'End turn', { optional: true, wait: 0 })) || (await click(s, pb, 'End turn', { optional: true, wait: 0 }));
    if (swung) lastSwing = Date.now();
  }
  if (frame % 40 === 20 && Date.now() - lastSwing > 25000) s.note(`stalled? phone A buttons: ${JSON.stringify(await pa.evaluate(() => [...document.querySelectorAll('button:not([disabled])')].map((b) => b.textContent.trim()).slice(0, 8)))} phase ${JSON.stringify(now)}`);
  if (now.aftermath && now.battle === 'absent' && !now.stage) { settled = now; break; }
  await settle(40);
}
fs.writeFileSync(`${out}-samples.json`, JSON.stringify(samples, null, 1));
await tv.screenshot({ path: `${out}-final.png` });
check(Boolean(settled), 'the fight ran to the victory card');

// Timestamp each sound by diffing the log between samples.
const sounds = [];
let known = samples[0].sfx ? samples[0].sfx.split(' ') : [];
for (const m of samples) {
  const names = m.sfx ? m.sfx.split(' ') : [];
  for (const name of names.slice(known.length)) sounds.push({ t: m.t, name, at: m });
  known = names;
}
// Transitions already in the log at the first sample belong to the story scenes before the fight.
const before = (samples[0].sfx.match(/scene-transition/g) || []).length;
const transitions = sounds.filter((x) => x.name === 'scene-transition');
const revealed = (m) => m.field && m.splat !== 'loading' && m.splat !== 'showing' && m.splat !== 'idle' && !['leaving', 'interstitial', 'title'].includes(m.stage);
const earlyCombat = sounds.filter((x) => COMBAT_SFX.has(x.name) && !revealed(x.at));
check(earlyCombat.length === 0, `no combat sound while the cover is up or the splat is incomplete (${earlyCombat.map((x) => `${x.name}@${x.at.stage}/${x.at.splat}`).join(', ') || 'none'})`);
const revealAt = samples.find((m) => m.field && revealed(m))?.t;
// The initiative phase is the first contiguous run of samples with the initiative HUD up (one false sample may fall in it).
const firstInitiative = samples.findIndex((m) => m.initiative);
let endInitiative = firstInitiative;
for (let i = firstInitiative; i >= 0 && i < samples.length; i += 1) {
  if (samples[i].initiative) endInitiative = i;
  else if (!samples.slice(i, i + 4).some((m) => m.initiative)) break;
}
const initiativeSamples = firstInitiative < 0 ? [] : samples.slice(firstInitiative, endInitiative + 1);
const initiativeMs = initiativeSamples.length ? initiativeSamples[initiativeSamples.length - 1].t - initiativeSamples[0].t : 0;
s.note(`revealed at +${revealAt - clickedAt} ms; initiative HUD on screen ${initiativeMs} ms`);
check(initiativeMs >= 2500, `the initiative roll and roll-call were each on screen in full (${initiativeMs} ms)`);
check(Boolean(revealAt && initiativeSamples[0] && initiativeSamples[0].t >= revealAt - 100), 'the initiative started only after the reveal');
const firstTurn = samples.find((m) => ['rolling', 'impact'].includes(m.stage));
check(!firstTurn || (revealAt && firstTurn.t >= initiativeSamples[initiativeSamples.length - 1].t - 100), 'no turn beat started before the roll-call ended');
const firstStrike = samples.find((m) => m.stage === 'rolling' && m.t >= (initiativeSamples[initiativeSamples.length - 1]?.t ?? 0));
const exitAt = samples.find((m) => m.stage === 'leaving' && m.t >= (firstStrike?.t ?? Infinity) + 500)?.t;
const swingSamples = samples.filter((m) => firstStrike && exitAt && m.t >= firstStrike.t && m.t < exitAt);
// The killing strike is the last 'rolling' before the exit.
const lastRolling = [...samples].reverse().find((m) => m.stage === 'rolling' && exitAt && m.t < exitAt);
if (lastRolling && exitAt) {
  s.note(`killing strike's dice to the exit transition: ${exitAt - lastRolling.t} ms`);
  check(exitAt - lastRolling.t >= 2500, `at least 2.5 s between the killing strike's dice and the exit transition (${exitAt - lastRolling.t} ms)`);
  const between = samples.filter((m) => m.t >= lastRolling.t && m.t < exitAt);
  check(between.every((m) => m.field), 'the battlefield was the visible screen throughout');
}
check(!samples.some((m) => m.aftermath && (m.dice || m.actionBubbles)), 'the victory card never coexisted with a combat dice card or bubble');
const entryTransitions = transitions.filter((x) => !exitAt || x.t < exitAt - 1500);
const exitTransitions = transitions.filter((x) => exitAt && x.t >= exitAt - 1500);
s.note(`scene-transitions: ${entryTransitions.length} on entry, ${exitTransitions.length} on exit`);
check(entryTransitions.length === 1, `exactly one scene-transition on entry (${entryTransitions.length})`);
check(exitTransitions.length === 1, `exactly one scene-transition on exit (${exitTransitions.length})`);
await finish(browser);
