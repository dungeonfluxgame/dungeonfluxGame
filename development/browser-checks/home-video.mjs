// usage: node home-video.mjs <base> <out> <mode> [seconds=24]
// Evaluator tooling only (not application code). Opens the shared display and samples the home
// loop every 100 ms: which clip is on top, each clip's currentTime and computed opacity, the loop
// layer's state, and screenshots at each cross-fade. Modes:
//   wrap    the lobby with a cached loop: fade-in time, swaps, screenshots either side of each wrap
//   off     the video service is off: the still only
//   broken  a cached file that will not play: the lobby is the still
//   story   two heroes begin the story: the loop is released and nothing plays behind the scene
// Writes <out>/<mode>.jsonl (one line per sample) and <out>/<mode>-summary.json.
import { launch, settle, click, create, session } from './lib.mjs';
import fs from 'node:fs';
import path from 'node:path';

const [base, out, mode, secondsArg] = process.argv.slice(2);
const seconds = Number(secondsArg || 24);
fs.mkdirSync(out, { recursive: true });
const browser = await launch();
const context = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(`PAGEERROR ${e.message.slice(0, 200)}`));
page.on('console', (m) => { if (m.type() === 'error') errors.push(`CONSOLE ${m.text().slice(0, 160)}`); });

const sample = () => page.evaluate(() => {
  const layer = document.querySelector('.tv-loop');
  const clips = [...document.querySelectorAll('.tv-loop-clip')].map((video) => ({
    on: video.hasAttribute('data-on'),
    first: video.hasAttribute('data-first'),
    current: Number(video.currentTime.toFixed(3)),
    paused: video.paused,
    opacity: Number(getComputedStyle(video).opacity),
    ready: video.readyState,
  }));
  return {
    layerHidden: layer ? layer.hasAttribute('hidden') : null,
    state: layer ? layer.getAttribute('data-state') : null,
    mode: document.querySelector('.tv-stage')?.getAttribute('data-tv-mode') ?? null,
    clips,
  };
});

const log = [];
const started = Date.now();
const jsonl = fs.createWriteStream(path.join(out, `${mode}.jsonl`));
const write = (entry) => { log.push(entry); jsonl.write(`${JSON.stringify(entry)}\n`); };

await page.goto(`${base}/gameplay/display`);
let phones = [];
if (mode === 'story') {
  // Two heroes join, create themselves and begin: the story takes over the stage. The begin
  // label is the offer's text on the phone; pass it as the fifth argument.
  const s = session(base, out);
  const phone = () => browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true }).then((c) => c.newPage());
  const pa = await phone();
  const pb = await phone();
  phones = [pa, pb];
  await pa.goto(`${base}/gameplay/player`);
  await pb.goto(`${base}/gameplay/player`);
  await settle(2500);
  await create(s, pa, 'Brakka', 'Copper beard');
  await click(s, pa, 'Confirm your hero');
  await create(s, pb, 'Thrain', 'Missing ear');
  await click(s, pb, 'Confirm your hero');
  await settle();
}

let previousFront = null;
let swaps = 0;
let firstFadeAt = null;
const screenshots = [];
const deadline = Date.now() + seconds * 1000;
let storyStarted = false;
while (Date.now() < deadline) {
  const at = Date.now() - started;
  const entry = { at, ...(await sample()) };
  const front = entry.clips.findIndex((clip) => clip.on);
  if (front >= 0 && firstFadeAt === null) {
    firstFadeAt = at;
    await page.screenshot({ path: path.join(out, `${mode}-first-fade.png`) });
  }
  if (front >= 0 && previousFront !== null && front !== previousFront) {
    swaps += 1;
    const label = `${mode}-swap-${swaps}`;
    await page.screenshot({ path: path.join(out, `${label}-after.png`) });
    screenshots.push(label);
  }
  if (front >= 0) previousFront = front;
  write(entry);
  if (mode === 'story' && !storyStarted && Date.now() - started > 4000) {
    storyStarted = true;
    const beginLabel = process.argv[6] || 'Begin';
    const clicked = await click(session(base, out), phones[0], beginLabel, { wait: 500 });
    // When the label is not offered, the visible buttons are recorded so the label can be fixed.
    const buttons = await phones[0].evaluate(() => [...document.querySelectorAll('button')].map((b) => b.textContent.trim()).filter(Boolean));
    write({ at: Date.now() - started, event: `begin ${clicked ? 'clicked' : 'missing'}: ${beginLabel}`, buttons });
  }
  await new Promise((resolve) => setTimeout(resolve, 100));
}

const summary = {
  mode,
  seconds,
  firstFadeAtMs: firstFadeAt,
  swaps,
  screenshots,
  finalState: log.at(-1) ?? null,
  errors,
};
fs.writeFileSync(path.join(out, `${mode}-summary.json`), JSON.stringify(summary, null, 2));
console.log(JSON.stringify(summary, null, 2));
jsonl.end();
await browser.close();
