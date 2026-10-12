// Evaluator tooling: prints the buttons phone A offers at each step of the harbor story, clicking the first
// scene-advancing offer each time (to learn which quick actions exist). usage: node stage-offers.mjs
import fs from 'node:fs';
import { runner, startServer, launchGpu, playToOpening, click, settle, framesDir } from './stage.mjs';

const { s, check, finish } = runner('stage-offers', framesDir);
const { runPack } = await startServer();
if (fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
const browser = await launchGpu(false);
const { pa } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: false });
await click(s, pa, s.label('begin-story'), { wait: 1500 });
const buttons = async () => pa.evaluate(() => [...document.querySelectorAll('button:not([disabled])')].map((b) => b.textContent.trim().replace(/\s+/g, ' ')).filter(Boolean));
for (let step = 0; step < 4; step += 1) {
  const list = await buttons();
  s.note(`step ${step}: ${JSON.stringify(list)}`);
  const next = [s.label('ask-courier'), s.label('escort-courier'), s.label('defend-courier')].find((l) => list.some((b) => b.includes(l)));
  if (!next) break;
  await click(s, pa, next, { wait: 1500 });
}
check(true, 'listed');
await finish(browser);
