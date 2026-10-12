// PLAY-F06 acceptance (2): a choice submitted mid-speech. A frame taken immediately after still shows
// the old scene with its line in progress (data-scene-state=presenting, data-stage-pending=true, old
// title); the old scene finishes its lines (the narrator's clip is never cut) and the new scene enters
// only after the last of them ends. usage: node stage-midspeech.mjs [frames-dir]
import fs from 'node:fs';
import { runner, startServer, launchGpu, playToOpening, click, settle, states, until, events, clearEvents, framesDir } from './stage.mjs';

const { s, out, check, finish } = runner('stage-midspeech', process.argv[2] || framesDir);
const { runPack } = await startServer();
if (fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
const browser = await launchGpu(false);
const { tv, pa } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true, voice: { latencyMs: 200, lengthFor: () => 6 } });
await click(s, pa, s.label('begin-story'), { wait: 100 });
const opening = await until(tv, (now) => now.narrator === 'speaking', 40000);
const oldTitle = opening.title;
s.note(`old scene "${oldTitle}" narrator speaking`);
await clearEvents(tv);
// The table chooses while the narrator is still speaking.
await click(s, pa, s.label('ask-courier'), { wait: 0 });
// The choice reaches the display through the server: wait for its pending successor, not for a fixed delay.
await until(tv, (now) => now.pending === 'true', 8000);
await settle(200);
const just = await states(tv);
await tv.screenshot({ path: `${out}-1-just-after-choice.png` });
s.note(`just after: ${JSON.stringify({ scene: just.scene, line: just.line, pending: just.pending, title: just.title, narrator: just.narrator })}`);
check(just.title === oldTitle, 'a frame right after the choice still shows the old scene title');
check(just.scene === 'presenting' && just.pending === 'true', `the old scene is still presenting with a pending successor (${just.scene}, pending=${just.pending})`);
check(['opening', 'speaking'].includes(just.narrator), `the narrator's line is still in progress (${just.narrator})`);
// Poll the title until the scene changes; remember when.
const sampled = [];
let changed = null;
const stop = Date.now() + 60000;
while (Date.now() < stop) {
  const now = await states(tv);
  sampled.push({ t: Date.now(), title: now.title, scene: now.scene, line: now.line, narrator: now.narrator });
  if (now.title !== oldTitle && changed === null) { changed = Date.now(); await tv.screenshot({ path: `${out}-2-new-scene-entering.png` }); }
  if (changed && now.scene === 'awaiting-choice') break;
  await settle(100);
}
const log = await events(tv);
fs.writeFileSync(`${out}-events.json`, JSON.stringify(log, null, 1));
const plays = log.filter((e) => e.type === 'audio' && e.event === 'playing');
const ended = log.filter((e) => e.type === 'audio' && e.event === 'ended');
const swap = log.find((e) => e.type === 'attr' && e.name === 'data-stage' && e.value === 'interstitial');
s.note(`clips played after the choice: ${plays.length}, ended: ${ended.length}`);
check(Boolean(swap), 'the scene transition ran');
check(ended.length >= 1 && swap && swap.t >= ended[0].t, 'the transition began only after the narrator clip had ended (never cut)');
check(ended.length >= 2 && swap && swap.t >= ended[1].t, 'the old scene said its remaining line (the courier) before the transition');
const premature = sampled.filter((m) => m.title !== oldTitle && swap && m.t < swap.t - 50);
check(premature.length === 0, 'the new title never showed before the transition');
const newOpen = log.find((e) => e.type === 'attr' && e.name === 'data-scene-state' && e.value === 'entering' && swap && e.t >= swap.t);
check(Boolean(newOpen), 'the new scene entered through its machine');
check(changed !== null, 'the new scene is on screen');
await finish(browser);
