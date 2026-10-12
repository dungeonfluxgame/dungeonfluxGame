// PLAY-F06: a fight that ends on its very first action (a pack whose bandit has 1 hit point and AC 1, so the first
// swing kills it), in a WebGPU-capable browser. The one view that ends the fight is queued behind the battle machine:
// Acting > Resolving > Finishing (the finishing blow in full) > Exiting (the battlefield fades) > Absent, and only then
// does the scene machine take the victory scene. Asserted from a dense sample strip (every ~100 ms, each with a
// screenshot): the stage root never shows the flat dice card, the flat combat screen never shows while the
// battlefield is live, `data-battle-state=absent` never coexists with a visible battlefield, no combat call-out bubble
// survives into the victory scene, and the whole exit stays inside its stated bound.
// usage: node stage-victory.mjs [frames-dir]
import fs from 'node:fs';
import path from 'node:path';
import { runner, startServer, launchGpu, playToOpening, click, settle, until, events, clearEvents, framesDir, artifactRoot, root } from './stage.mjs';

// blow cues (about 4.3 s) + BATTLE_FINISH_CAP_MS (8 s) + the scene transition (about 2 s) + slack
const EXIT_BOUND_MS = 17000;
const { s, out, check, finish } = runner('stage-victory', process.argv[2] || framesDir);
const pack = path.join(artifactRoot, 'tmp', 'play-f06', 'oneshot-pack');
fs.rmSync(pack, { recursive: true, force: true });
fs.cpSync(path.join(root, 'content', 'harbor-journey'), pack, { recursive: true });
const tuning = JSON.parse(fs.readFileSync(path.join(pack, 'tuning.json'), 'utf8'));
tuning.bandit.hit_points = 1;
tuning.bandit.armor_class = 1;
fs.writeFileSync(path.join(pack, 'tuning.json'), JSON.stringify(tuning, null, 2));
const extra = process.env.DF_BATTLE_SCENE ? { DF_GAMEPLAY_BATTLE_SCENE: process.env.DF_BATTLE_SCENE, DF_GAMEPLAY_SPLAT_DIR: process.env.DF_SPLAT_DIR || '' } : {};
await startServer({ DF_GAMEPLAY_PACK_DIR: pack, ...extra });
Object.assign(s.labels, JSON.parse(fs.readFileSync(path.join(pack, 'pack.json'), 'utf8')).offer_labels);
const browser = await launchGpu(true);
const { tv, pa, pb } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: false });
await click(s, pa, s.label('begin-story'), { wait: 100 });
await until(tv, (now) => now.scene === 'awaiting-choice', 60000);
await click(s, pa, s.label('escort-courier'), { wait: 100 });
await until(tv, (now) => now.title && now.title.includes('Oath') && now.scene === 'awaiting-choice', 60000);
await click(s, pa, s.label('defend-courier'), { wait: 100 });
await until(tv, (now) => ['turn-start', 'turn-end'].includes(now.battle), 90000);
await settle(500);
await clearEvents(tv);

const samples = [];
const sample = async () => {
  const state = await tv.evaluate(() => {
    const stage = document.querySelector('.tv-stage');
    const visible = (el) => { if (!el) return false; const cs = getComputedStyle(el); return cs.visibility !== 'hidden' && cs.display !== 'none' && Number(cs.opacity) > 0.1; };
    const opacity = (el) => (el ? Number(getComputedStyle(el).opacity) : 0);
    const field = document.querySelector('.battlefield');
    const flat = document.querySelector('.tv-stage .live-screen[data-screen=combat]');
    const after = document.querySelector('.tv-stage .live-screen[data-screen=aftermath]');
    return {
      t: Date.now(),
      battle: stage?.getAttribute('data-battle-state'), scene: stage?.getAttribute('data-scene-state'), stage: stage?.getAttribute('data-stage'),
      live: stage?.getAttribute('data-battlefield-live'),
      field: Boolean(field) && visible(field), fieldOpacity: opacity(field), flatCombat: visible(flat), aftermath: visible(after),
      finalRoll: visible(document.querySelector('.tv-final-roll')),
      actionBubbles: [...document.querySelectorAll('.tv-bubble[data-kind=action]')].filter(visible).map((b) => b.textContent.trim().slice(0, 40)),
    };
  });
  samples.push(state);
  return state;
};
const clickedAt = Date.now();
let attacked = false;
let lastSwing = 0;
let frame = 0;
const stop = Date.now() + 90000;
let settled = null;
while (Date.now() < stop) {
  const now = await sample();
  // Swing whenever a strike is offered and nothing is playing (a miss lets the fight go on and swing again).
  if (!now.aftermath && Date.now() - lastSwing > 7000 && (!now.stage) && ['turn-start', 'turn-end'].includes(now.battle)) {
    const swung = (await click(s, pa, 'Greatsword strike', { optional: true, wait: 0 })) || (await click(s, pb, 'Greatsword strike', { optional: true, wait: 0 }));
    if (swung) { attacked = true; lastSwing = Date.now(); }
  }
  if (attacked && frame < 400) { await tv.screenshot({ path: `${out}-strip-${String(frame).padStart(3, '0')}.jpg`, type: 'jpeg', quality: 60 }); frame += 1; }
  if (attacked && now.aftermath && now.battle === 'absent' && !now.stage) { settled = now; break; }
  await settle(60);
}
const log = await events(tv);
fs.writeFileSync(`${out}-events.json`, JSON.stringify(log, null, 1));
fs.writeFileSync(`${out}-samples.json`, JSON.stringify(samples, null, 1));
await tv.screenshot({ path: `${out}-final.png` });
const order = log.filter((e) => e.type === 'attr' && e.name === 'data-battle-state').map((e) => e.value).filter((v, i, a) => i === 0 || v !== a[i - 1]);
s.note(`battle states: ${order.join(' > ')}`);
const want = ['acting', 'resolving', 'finishing', 'exiting', 'absent'];
const positions = want.map((name) => order.indexOf(name));
check(positions.every((p, i) => p >= 0 && (i === 0 || p > positions[i - 1])), 'Acting > Resolving > Finishing > Exiting > Absent');
check(Boolean(settled), 'the victory scene settled');
const at = (name) => log.find((e) => e.type === 'attr' && e.name === 'data-battle-state' && e.value === name)?.t;
const victoryAt = samples.find((m) => m.aftermath)?.t;
check(Boolean(victoryAt && at('exiting') && victoryAt >= at('exiting')), 'the victory scene appears only after the battle machine reached Exiting');
check(!samples.some((m) => m.finalRoll), 'the flat dice card never showed');
check(!samples.some((m) => m.flatCombat && (m.live === 'true' || m.field)), 'the flat combat screen never showed while the battlefield was up');
const flash = samples.filter((m) => m.battle === 'absent' && m.field && m.fieldOpacity > 0.5);
check(flash.length === 0, `never battle=absent with a visible battlefield (${flash.length} samples)`);
const leaked = samples.filter((m) => m.aftermath && m.actionBubbles.length);
check(leaked.length === 0, `no combat call-out bubble on the victory scene (${leaked.length} samples)`);
const total = settled ? settled.t - clickedAt : null;
s.note(`exit took ${total} ms from the first attempt to attack`);
check(total !== null && total <= EXIT_BOUND_MS + 8000, `the whole fight exit is inside its bound (${total} ms; blow plus cap ${EXIT_BOUND_MS} ms plus the click wait)`);
const rolling = log.find((e) => e.type === 'attr' && e.name === 'data-stage' && e.value === 'rolling')?.t;
const leaving = log.find((e) => e.type === 'attr' && e.name === 'data-stage' && e.value === 'leaving')?.t;
if (rolling && leaving) {
  s.note(`blow to leaving ${leaving - rolling} ms`);
  check(leaving - rolling >= 4000, `the finishing blow was shown in full before the scene left (${leaving - rolling} ms)`);
}
await finish(browser);
