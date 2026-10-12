// PLAY-F06 acceptance (3): three quick actions. While the opening scene is still speaking, the table
// chooses "ask" (a scene change), then "defend" (the fight begins), then attacks: the display converges
// on the newest scene within a stated bound, the skipped scene is never shown, and nothing from an older
// scene is still playing when it does. usage: node stage-quick.mjs [frames-dir]
import fs from 'node:fs';
import { runner, startServer, launchGpu, playToOpening, click, settle, states, until, events, clearEvents, framesDir } from './stage.mjs';

const CONVERGE_BOUND_MS = 15000;
const { s, out, check, finish } = runner('stage-quick', process.argv[2] || framesDir);
const { runPack } = await startServer();
if (fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
const browser = await launchGpu(false);
const { tv, pa, pb } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true, voice: { latencyMs: 200, lengthFor: () => 6 } });
await click(s, pa, s.label('begin-story'), { wait: 100 });
await until(tv, (now) => now.narrator === 'speaking', 40000);
await clearEvents(tv);
const start = Date.now();
await click(s, pa, s.label('ask-courier'), { wait: 100 });
await click(s, pa, s.label('defend-courier'), { wait: 100 });
const attacked = (await click(s, pa, 'Greatsword strike', { optional: true, wait: 0 })) || (await click(s, pb, 'Greatsword strike', { optional: true, wait: 0 }));
const lastAction = Date.now();
s.note(`three actions in ${lastAction - start} ms (attack clicked: ${attacked})`);
await tv.screenshot({ path: `${out}-1-just-after.png` });
const converged = await until(tv, (now) => now.screen === 'combat' && now.pending === 'false' && now.scene !== 'exiting', CONVERGE_BOUND_MS + 30000).catch(() => null);
const convergedAt = Date.now();
await tv.screenshot({ path: `${out}-2-converged.png` });
const log = await events(tv);
fs.writeFileSync(`${out}-events.json`, JSON.stringify(log, null, 1));
check(Boolean(converged), 'the display shows the newest scene (the fight)');
const after = convergedAt - lastAction;
s.note(`converged ${after} ms after the last action`);
check(after <= CONVERGE_BOUND_MS, `converged within ${CONVERGE_BOUND_MS} ms of the last action (${after} ms)`);
const shown = (text) => log.some((e) => (e.type === 'bubble+' || e.type === 'bubble~') && e.text && e.text.includes(text));
check(!shown('The crest belongs'), 'the skipped scene\'s courier line never showed');
const titles = await tv.evaluate(() => document.querySelector('.tv-title')?.textContent ?? '');
check(!titles.includes('Whisper'), 'the skipped scene\'s title is not on screen');
const lingering = await states(tv);
check(lingering.bubbles.every((b) => b.kind === 'action' || b.state === 'closing'), `no story bubble from an older scene is on screen (${JSON.stringify(lingering.bubbles)})`);
// Between the first action and the newest scene presenting, the old scene's clip is cut for the fight
// (a pause without an end) and no other clip starts.
const exitingAt = log.find((e) => e.type === 'attr' && e.name === 'data-scene-state' && e.value === 'exiting');
const presentingAt = log.find((e) => e.type === 'attr' && e.name === 'data-scene-state' && e.value === 'presenting' && exitingAt && e.t > exitingAt.t);
const strayStarts = log.filter((e) => e.type === 'audio' && e.event === 'playing' && exitingAt && presentingAt && e.t > exitingAt.t - 100 && e.t < presentingAt.t);
check(Boolean(exitingAt && presentingAt), 'the old scene exited and the newest scene is presenting');
check(strayStarts.length === 0, `no clip of an older scene started while the display converged (${strayStarts.length})`);
const cut = log.find((e) => e.type === 'audio' && e.event === 'pause');
check(Boolean(cut), 'the old clip was stopped for the fight');
const sceneOrder = log.filter((e) => e.type === 'attr' && e.name === 'data-scene-state').map((e) => e.value).filter((v, i, a) => i === 0 || v !== a[i - 1]);
s.note(`scene states: ${sceneOrder.join(' > ')}`);
check(sceneOrder.includes('exiting') && sceneOrder.includes('entering'), 'the scene left and the successor entered');
await finish(browser);
