// usage: node home-pair-measure.mjs <base> <out> <label> [seconds=60]
// Evaluator tooling only (not application code). Opens the shared display on a lobby whose loop
// is already cached, then samples every display frame: each loop clip is drawn to a 32x18 canvas
// at the display rate, and a frame is kept whenever either clip has a new decoded frame. The
// picture is approximated as the clips mixed by their computed opacity (the still is taken as
// covered by full-opacity clips). For each kept frame the mean luminance and the mean absolute
// per-block luminance difference against the previous kept frame are measured. A join is a
// cross-fade window (both clips on, until the outgoing one reaches zero opacity). The summary
// compares the largest change inside each join window with the mid-clip changes outside them.
// Writes <out>/<label>-frames.jsonl (per-frame measures) and <out>/<label>-joins.json.
import { launch, settle } from './lib.mjs';
import fs from 'node:fs';
import path from 'node:path';

const [base, out, label, secondsArg] = process.argv.slice(2);
const seconds = Number(secondsArg || 60);
fs.mkdirSync(out, { recursive: true });

const browser = await launch();
const context = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(`PAGEERROR ${e.message.slice(0, 200)}`));

await page.goto(`${base}/gameplay/display`);
await page.waitForFunction(
  () => document.querySelector('.tv-loop')?.getAttribute('data-state') === 'playing',
  null,
  { timeout: 120000 },
);
await settle(1500);

await page.evaluate(() => {
  const cvs = document.createElement('canvas');
  cvs.width = 32;
  cvs.height = 18;
  const ctx = cvs.getContext('2d', { willReadFrequently: true });
  const st = (window.__pair = { samples: [], lastTimes: [null, null], stop: false });
  const grab = (v) => {
    ctx.drawImage(v, 0, 0, 32, 18);
    const d = ctx.getImageData(0, 0, 32, 18).data;
    const y = new Array(576);
    for (let i = 0; i < 576; i += 1) {
      const p = i * 4;
      y[i] = 0.299 * d[p] + 0.587 * d[p + 1] + 0.114 * d[p + 2];
    }
    return y;
  };
  const tick = () => {
    if (st.stop) return;
    const clips = [...document.querySelectorAll('.tv-loop-clip')];
    if (clips.length === 2 && clips.every((v) => v.readyState >= 2)) {
      const times = clips.map((v) => v.currentTime);
      if (times.some((t, i) => t !== st.lastTimes[i])) {
        st.lastTimes = times;
        st.samples.push({
          t: performance.now(),
          times,
          ops: clips.map((v) => Number(getComputedStyle(v).opacity)),
          on: clips.map((v) => v.hasAttribute('data-on')),
          ys: clips.map((v) => grab(v)),
        });
      }
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
});

await new Promise((resolve) => setTimeout(resolve, seconds * 1000));
const samples = await page.evaluate(() => {
  window.__pair.stop = true;
  return window.__pair.samples;
});

// Mix the two clips by their opacity; a frame is the mean of the 576 block luminances.
const frames = samples
  .map((s) => {
    const total = s.ops[0] + s.ops[1];
    if (total < 0.01) return null;
    const mixed = s.ys[0].map((y0, i) => (s.ops[0] * y0 + s.ops[1] * s.ys[1][i]) / total);
    const mean = mixed.reduce((a, b) => a + b, 0) / mixed.length;
    return { t: s.t, times: s.times, ops: s.ops, on: s.on, mixed, mean };
  })
  .filter(Boolean);

const measured = [];
for (let k = 1; k < frames.length; k += 1) {
  const a = frames[k - 1];
  const b = frames[k];
  const blockDiff =
    b.mixed.reduce((sum, y, i) => sum + Math.abs(y - a.mixed[i]), 0) / b.mixed.length;
  measured.push({
    t: Math.round(b.t),
    dt: Math.round(b.t - a.t),
    ops: b.ops.map((o) => Number(o.toFixed(3))),
    meanLuma: Number(b.mean.toFixed(3)),
    dLuma: Number(Math.abs(b.mean - a.mean).toFixed(3)),
    blockDiff: Number(blockDiff.toFixed(3)),
  });
}

// Join windows: both clips on, until one of them has faded out.
const windows = [];
let open = null;
for (const m of measured) {
  const both = m.ops[0] > 0.001 && m.ops[1] > 0.001;
  if (both && open === null) open = { start: m.t, end: m.t };
  else if (open !== null) {
    open.end = m.t;
    if (!both) {
      windows.push(open);
      open = null;
    }
  }
}
const inWindow = (t) => windows.some((w) => t >= w.start - 100 && t <= w.end + 100);
const midClip = measured.filter((m) => !inWindow(m.t));
const percentile = (values, p) => {
  if (values.length === 0) return null;
  const sorted = [...values].sort((x, y) => x - y);
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))];
};
const joins = windows.map((w) => {
  const inside = measured.filter((m) => m.t >= w.start && m.t <= w.end);
  return {
    startMs: Math.round(w.start),
    durationMs: Math.round(w.end - w.start),
    frames: inside.length,
    maxDLuma: Math.max(0, ...inside.map((m) => m.dLuma)),
    maxBlockDiff: Math.max(0, ...inside.map((m) => m.blockDiff)),
  };
});
const summary = {
  label,
  seconds,
  frames: frames.length,
  joins: joins.length,
  midClip: {
    frames: midClip.length,
    dLumaP95: percentile(midClip.map((m) => m.dLuma), 95),
    dLumaMax: percentile(midClip.map((m) => m.dLuma), 100),
    blockDiffP95: percentile(midClip.map((m) => m.blockDiff), 95),
    blockDiffMax: percentile(midClip.map((m) => m.blockDiff), 100),
  },
  joinsDetail: joins,
  joinsStandOut: joins.filter(
    (j) =>
      j.maxBlockDiff > (percentile(midClip.map((m) => m.blockDiff), 100) ?? Infinity) ||
      j.maxDLuma > (percentile(midClip.map((m) => m.dLuma), 100) ?? Infinity),
  ).length,
  errors,
};
fs.writeFileSync(path.join(out, `${label}-frames.jsonl`), measured.map((m) => JSON.stringify(m)).join('\n') + '\n');
fs.writeFileSync(path.join(out, `${label}-joins.json`), JSON.stringify(summary, null, 2));
console.log(JSON.stringify({ ...summary, joinsDetail: summary.joinsDetail.slice(0, 12) }, null, 2));
await browser.close();
