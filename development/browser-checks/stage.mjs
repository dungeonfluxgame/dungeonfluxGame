// Evaluator tooling only (not application code): helpers for the PLAY-F06 stage-machine checks.
// A recorder on the display page logs every machine state change and every audio event with a
// timestamp, a fake voice answers /gameplay/voice/* with silent WAV clips of a chosen length, and
// the asserts read the recorded states instead of sleeping.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { chromium, exe, session, click, create, settle, watch } from './lib.mjs';

export const here = path.dirname(fileURLToPath(import.meta.url));
export const root = path.join(here, '..', '..');
export const artifactRoot = process.env.DUNGEONFLUX_ARTIFACT_ROOT || path.join(root, 'artifacts');
export const logDir = process.env.DF_CHECK_OUT || path.join(artifactRoot, 'tmp', 'browser-checks');
// Retained frames and event logs of the PLAY-F06 checks.
export const framesDir = process.env.DF_FRAMES || path.join(root, '..', '..', '..', 'artifacts', 'tmp', 'play-f06', 'frames');
export const packPath = path.join(root, 'content', 'harbor-journey', 'pack.json');

// A silent 8 kHz mono WAV of the given length.
export function silentWav(seconds) {
  const rate = 8000;
  const samples = Math.max(1, Math.round(rate * seconds));
  const buffer = Buffer.alloc(44 + samples, 128);
  buffer.write('RIFF', 0); buffer.writeUInt32LE(36 + samples, 4); buffer.write('WAVE', 8);
  buffer.write('fmt ', 12); buffer.writeUInt32LE(16, 16); buffer.writeUInt16LE(1, 20); buffer.writeUInt16LE(1, 22);
  buffer.writeUInt32LE(rate, 24); buffer.writeUInt32LE(rate, 28); buffer.writeUInt16LE(1, 32); buffer.writeUInt16LE(8, 34);
  buffer.write('data', 36); buffer.writeUInt32LE(samples, 40);
  return buffer;
}

const RECORDER = `
(() => {
  const rec = (window.__rec = []);
  const push = (type, data) => rec.push({ t: Math.round(performance.timeOrigin + performance.now()), type, ...data });
  const names = ['data-scene-state', 'data-line-state', 'data-line-id', 'data-stage-pending', 'data-stage', 'data-speaker', 'data-actor-state', 'data-battle-state', 'data-intent-state', 'data-intent', 'data-thinking', 'data-present'];
  const describe = (target) => ({ cls: String(target.className || '').slice(0, 40), slot: target.getAttribute && target.getAttribute('data-slot') || undefined });
  const bubble = (b) => ({ kind: b.getAttribute('data-kind'), id: b.getAttribute('data-line-id'), state: b.getAttribute('data-line-state'),
    text: (b.querySelector('.tv-bubble-text') || b).textContent.trim().replace(/\\s+/g, ' ').slice(0, 160),
    name: (b.querySelector('.tv-bubble-name') || { textContent: '' }).textContent.trim(),
    slot: b.closest('[data-slot]') && b.closest('[data-slot]').getAttribute('data-slot') });
  const start = () => {
    new MutationObserver((muts) => {
      for (const m of muts) {
        if (m.type === 'attributes') {
          const value = m.target.getAttribute(m.attributeName);
          if (m.target.classList && m.target.classList.contains('tv-bubble') && m.attributeName === 'data-line-state') push('bubble~', bubble(m.target));
          else push('attr', { name: m.attributeName, value, ...describe(m.target) });
        } else {
          for (const n of m.addedNodes) if (n.nodeType === 1) {
            const found = n.matches && n.matches('.tv-bubble') ? [n] : n.querySelectorAll ? [...n.querySelectorAll('.tv-bubble')] : [];
            for (const b of found) push('bubble+', bubble(b));
            if (n.matches && n.matches('.tv-thinking, .tv-intent')) push('mark+', { cls: String(n.className) });
          }
          for (const n of m.removedNodes) if (n.nodeType === 1) {
            if (n.matches && n.matches('.tv-bubble')) push('bubble-', bubble(n));
            if (n.matches && n.matches('.tv-thinking, .tv-intent')) push('mark-', { cls: String(n.className) });
          }
        }
      }
    }).observe(document.documentElement, { subtree: true, childList: true, attributes: true, attributeFilter: names });
  };
  if (document.documentElement) start(); else document.addEventListener('DOMContentLoaded', start);
  const Native = window.Audio;
  window.Audio = function (...args) {
    const el = new Native(...args);
    for (const ev of ['playing', 'ended', 'error', 'pause']) el.addEventListener(ev, () => { if (el.src.startsWith('blob:')) push('audio', { event: ev, duration: el.duration }); });
    return el;
  };
  window.Audio.prototype = Native.prototype;
})();
`;

// Installs the recorder before any page script runs.
export async function record(page) {
  await page.addInitScript(RECORDER);
}

// Answers voice requests with silent clips. `lengthFor(key)` is the clip length in seconds, `latencyMs`
// how long the fake server takes, `fail(key)` to answer 404.
export async function fakeVoice(page, { latencyMs = 150, lengthFor = () => 3, fail = () => false, log = [] } = {}) {
  await page.route('**/gameplay/voice/**', async (route) => {
    const key = route.request().url().split('/').pop();
    const at = Date.now();
    await new Promise((resolve) => setTimeout(resolve, latencyMs));
    if (fail(key)) { log.push({ key, at, status: 404 }); await route.fulfill({ status: 404, body: '' }); return; }
    const seconds = lengthFor(key);
    log.push({ key, at, status: 200, seconds });
    await route.fulfill({ status: 200, contentType: 'audio/wav', body: silentWav(seconds) });
  });
}

export const events = (page) => page.evaluate(() => window.__rec.slice());
export const clearEvents = (page) => page.evaluate(() => { window.__rec.length = 0; });

export async function states(page) {
  return page.evaluate(() => {
    const stage = document.querySelector('.tv-stage');
    const get = (name) => (stage ? stage.getAttribute(name) : null);
    const dm = document.querySelector('.tv-dm');
    return {
      scene: get('data-scene-state'), line: get('data-line-state'), lineId: get('data-line-id'), pending: get('data-stage-pending'),
      stage: get('data-stage'), speaker: get('data-speaker'), battle: get('data-battle-state'),
      narrator: dm ? dm.getAttribute('data-line-state') : null,
      title: document.querySelector('.tv-title')?.textContent ?? null,
      screen: document.querySelector('.live-screen')?.getAttribute('data-screen') ?? null,
      arrange: document.querySelector('.tv-figures')?.getAttribute('data-arrange') ?? null,
      actors: [...document.querySelectorAll('.tv-fig')].map((f) => `${f.getAttribute('data-slot')}:${f.getAttribute('data-actor-state')}`),
      bubbles: [...document.querySelectorAll('.tv-bubble')].map((b) => ({ kind: b.getAttribute('data-kind'), state: b.getAttribute('data-line-state'), text: (b.querySelector('.tv-bubble-text') || b).textContent.trim().slice(0, 80) })),
      intents: [...document.querySelectorAll('.tv-intent')].map((i) => `${i.closest('[data-slot]')?.getAttribute('data-slot')}:${i.getAttribute('data-intent')}:${i.getAttribute('data-line-state')}`),
      chips: [...document.querySelectorAll('.tv-choice-label')].map((c) => c.textContent),
      chipsVisible: (() => { const c = document.querySelector('.tv-choices'); if (!c) return null; const cs = getComputedStyle(c); return cs.visibility !== 'hidden' && cs.display !== 'none' && Number(cs.opacity) > 0.5; })(),
    };
  });
}

// Polls until `ready(states)` holds; resolves with the states or throws after `timeoutMs`.
export async function until(page, ready, timeoutMs = 20000, every = 100) {
  const stop = Date.now() + timeoutMs;
  for (;;) {
    const now = await states(page);
    if (ready(now)) return now;
    if (Date.now() > stop) throw new Error(`timed out; last states ${JSON.stringify(now)}`);
    await settle(every);
  }
}

export function runner(name, outDir) {
  const out = path.join(outDir, name);
  const s = session(process.env.DF_BASE || 'http://127.0.0.1:63342', out, packPath);
  let failures = 0;
  const check = (ok, m) => { if (!ok) failures += 1; s.note(`${ok ? 'ok  ' : 'FAIL'} ${m}`); return ok; };
  const finish = (browser) => {
    fs.writeFileSync(path.join(out, 'log.txt'), s.log.join('\n') + '\n');
    return browser.close().then(() => { s.note(failures ? `${failures} FAILED` : 'ALL PASSED'); process.exit(failures ? 1 : 0); });
  };
  return { s, out, check, finish };
}

// Starts this checkout's server on its own port with its own database; returns the database name.
export async function startServer(extraEnv = {}) {
  const db = `df_gameplay_demo_f06_${String(Date.now()).slice(-7)}`;
  execFileSync(path.join(here, 'serve.sh'), ['stop']);
  Object.assign(process.env, extraEnv);
  execFileSync(path.join(here, 'serve.sh'), ['start', db], { encoding: 'utf8' });
  const runPack = path.join(here, '..', 'runs', db, 'pack.json');
  // Read the client bundle once so the first page does not wait on the server's cold cache.
  for (const route of ['/gameplay/display', '/gameplay/player', '/pkg/df_tools.js', '/pkg/df_tools_bg.wasm']) {
    await fetch(`${process.env.DF_BASE}${route}`).then((response) => response.arrayBuffer());
  }
  return { db, runPack };
}

export async function launchGpu(gpu = false, extra = []) {
  const args = ['--autoplay-policy=no-user-gesture-required', ...extra];
  if (gpu) args.push('--enable-unsafe-webgpu', '--use-angle=metal', '--enable-features=Vulkan,UseSkiaRenderer');
  return chromium.launch({ headless: true, executablePath: exe, args });
}

// Plays the room: TV, two phones, two heroes, up to the opening scene. Returns the pages.
export async function playToOpening(s, browser, { base, sound = true, voice, phoneSetup, tvViewport = { width: 1920, height: 1080 } } = {}) {
  const tv = await (await browser.newContext({ viewport: tvViewport })).newPage();
  const phone = () => browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
  const pa = await (await phone()).newPage();
  const pb = await (await phone()).newPage();
  await record(tv);
  if (phoneSetup) for (const page of [pa, pb]) await phoneSetup(page);
  if (voice) await fakeVoice(tv, voice);
  for (const [n, p] of [['tv', tv], ['a', pa], ['b', pb]]) watch(s, p, n);
  await tv.goto(`${base}/gameplay/display`);
  await pa.goto(`${base}/gameplay/player`);
  await pb.goto(`${base}/gameplay/player`);
  await settle(2500);
  if (sound) await tv.mouse.click(40, 500);
  await create(s, pa, 'Brakka', 'Copper beard');
  await click(s, pa, 'Confirm your hero');
  await create(s, pb, 'Thrain', 'Missing ear');
  await click(s, pb, 'Confirm your hero');
  await settle();
  return { tv, pa, pb };
}

export { session, click, create, settle, watch, chromium, exe };
