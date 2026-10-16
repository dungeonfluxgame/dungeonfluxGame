// The display mute must silence the music output immediately, then restore its normal fade on
// unmute. The analyser taps the music master bus, so this checks audio samples as well as labels.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import {
  runner,
  startServer,
  record,
  click,
  settle,
  watch,
  chromium,
  exe,
} from './stage.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const base = process.env.DF_BASE || 'http://127.0.0.1:63342';
const { s, out, check, finish } = runner('sound-mute', process.env.DF_CHECK_OUT || path.join(here, '..', '..', 'artifacts', 'tmp', 'browser-checks'));
const manageServer = process.env.DF_MUTE_MANAGE_SERVER !== '0';
const { runPack } = manageServer ? await startServer() : { runPack: null };
let room = process.env.DF_ROOM;
if (!manageServer) {
  for (const route of ['/gameplay/display', '/gameplay/player', '/pkg/df_tools.js', '/pkg/df_tools_bg.wasm']) {
    const response = await fetch(`${base}${route}`);
    if (!response.ok) throw new Error(`mute-check server returned ${response.status} for ${route}`);
  }
}
if (runPack && fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
if (manageServer && !room) {
  const artifactRoot = process.env.DUNGEONFLUX_ARTIFACT_ROOT || path.join(here, '..', '..', 'artifacts');
  const serverLog = path.join(artifactRoot, 'tmp', 'browser-checks', `server-${process.env.PORT || '63342'}.log`);
  const match = fs.readFileSync(serverLog, 'utf8').match(/^ROOM\s+([A-Z0-9]+)/m);
  room = match?.[1];
}
if (!room) throw new Error('set DF_ROOM to the room code shown on the display');

const browser = await chromium.launch({
  headless: true,
  executablePath: exe,
  args: ['--autoplay-policy=no-user-gesture-required'],
});

try {
  const tv = await (await browser.newContext({ viewport: { width: 1920, height: 1080 } })).newPage();
  const phone = () => browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
  const pa = await (await phone()).newPage();
  const pb = await (await phone()).newPage();
  await record(tv);
  await tv.addInitScript(`
    (() => {
      const record = { context: null, output: null, analyser: null, gainCount: 0 };
      window.__dfMusicAudio = record;
      const originalCompressor = AudioContext.prototype.createDynamicsCompressor;
      AudioContext.prototype.createDynamicsCompressor = function (...args) {
        record.context = this;
        return Reflect.apply(originalCompressor, this, args);
      };
      const originalGain = AudioContext.prototype.createGain;
      AudioContext.prototype.createGain = function (...args) {
        const gain = Reflect.apply(originalGain, this, args);
        if (this === record.context && ++record.gainCount === 1) {
          record.output = gain;
          record.analyser = this.createAnalyser();
          record.analyser.fftSize = 2048;
          gain.connect(record.analyser);
        }
        return gain;
      };
    })();
  `);

  for (const [name, page] of [['tv', tv], ['a', pa], ['b', pb]]) watch(s, page, name);
  await tv.goto(`${base}/gameplay/display`);
  await pa.goto(`${base}/gameplay/player?room=${room}`);
  await pb.goto(`${base}/gameplay/player?room=${room}`);
  await settle(2_500);
  await tv.mouse.click(40, 500);
  await createHero(pa, 'Brakka');
  await createHero(pb, 'Thrain');
  await settle();
  await click(s, pa, s.label('begin-story'), { wait: 100 });

  const audibleState = await tv.waitForFunction(() => {
    const root = document.getElementById('display');
    const music = root?.getAttribute('data-music-state');
    const probe = window.__dfMusicAudio;
    if (!music || !probe?.analyser || !probe.output) return false;
    const state = JSON.parse(music);
    const samples = new Float32Array(probe.analyser.fftSize);
    probe.analyser.getFloatTimeDomainData(samples);
    const rms = Math.sqrt(samples.reduce((sum, sample) => sum + sample * sample, 0) / samples.length);
    return state.sound === 'on' && state.layers_started && rms > 0.0001;
  }, { timeout: 30_000 }).then(() => tv.evaluate(() => {
    const root = document.getElementById('display');
    const probe = window.__dfMusicAudio;
    const samples = new Float32Array(probe.analyser.fftSize);
    probe.analyser.getFloatTimeDomainData(samples);
    return {
      state: JSON.parse(root.getAttribute('data-music-state')),
      rms: Math.sqrt(samples.reduce((sum, sample) => sum + sample * sample, 0) / samples.length),
      outputGain: probe.output.gain.value,
    };
  }));
  check(audibleState.rms > 0.0001, `background music reaches the browser audio graph (RMS ${audibleState.rms})`);

  const control = tv.locator('.tv-voice-control');
  await control.click();
  const immediateMute = await tv.evaluate(() => ({
    voice: document.querySelector('.tv-voice-control')?.getAttribute('data-voice-state'),
    music: JSON.parse(document.getElementById('display').getAttribute('data-music-state')),
    outputGain: window.__dfMusicAudio.output.gain.value,
  }));
  s.note(`immediate mute ${JSON.stringify({ voice: immediateMute.voice, sound: immediateMute.music.sound, outputGain: immediateMute.outputGain })}`);
  check(immediateMute.voice === 'muted', 'the display switch records mute when the click handler returns');
  check(immediateMute.music.sound === 'off', 'the music director records sound off when the click handler returns');
  check(immediateMute.outputGain === 0, 'the music output gate is zero when the click handler returns');
  await settle(100);
  const mutedState = await tv.evaluate(() => {
    const root = document.getElementById('display');
    const probe = window.__dfMusicAudio;
    const samples = new Float32Array(probe.analyser.fftSize);
    probe.analyser.getFloatTimeDomainData(samples);
    return {
      voice: document.querySelector('.tv-voice-control')?.getAttribute('data-voice-state'),
      music: JSON.parse(root.getAttribute('data-music-state')),
      rms: Math.sqrt(samples.reduce((sum, sample) => sum + sample * sample, 0) / samples.length),
      outputGain: probe.output.gain.value,
    };
  });
  s.note(`muted audio probe ${JSON.stringify({ voice: mutedState.voice, sound: mutedState.music.sound, rms: mutedState.rms, outputGain: mutedState.outputGain })}`);
  check(mutedState.rms < audibleState.rms * 0.02, `the music waveform is silent after mute (RMS ${mutedState.rms}, before ${audibleState.rms})`);

  await control.click();
  await tv.waitForFunction(() => {
    const root = document.getElementById('display');
    const probe = window.__dfMusicAudio;
    if (!root || !probe?.analyser) return false;
    const music = JSON.parse(root.getAttribute('data-music-state') || '{}');
    const samples = new Float32Array(probe.analyser.fftSize);
    probe.analyser.getFloatTimeDomainData(samples);
    const rms = Math.sqrt(samples.reduce((sum, sample) => sum + sample * sample, 0) / samples.length);
    return music.sound === 'on' && rms > 0.0001;
  }, { timeout: 3_000 }).catch(() => null);
  const restoredState = await tv.evaluate(() => {
    const probe = window.__dfMusicAudio;
    const samples = new Float32Array(probe.analyser.fftSize);
    probe.analyser.getFloatTimeDomainData(samples);
    return {
      voice: document.querySelector('.tv-voice-control')?.getAttribute('data-voice-state'),
      music: JSON.parse(document.getElementById('display').getAttribute('data-music-state')),
      rms: Math.sqrt(samples.reduce((sum, sample) => sum + sample * sample, 0) / samples.length),
      outputGain: probe.output.gain.value,
    };
  });
  s.note(`unmuted audio probe ${JSON.stringify({ voice: restoredState.voice, sound: restoredState.music.sound, rms: restoredState.rms, outputGain: restoredState.outputGain })}`);
  check(restoredState.voice === 'on', 'the display switch records sound on after unmute');
  check(restoredState.music.sound === 'on', 'the music director records sound on after unmute');
  check(restoredState.rms > audibleState.rms * 0.02, `music returns through its normal fade (RMS ${restoredState.rms}, before ${audibleState.rms})`);
} catch (error) {
  s.note(`ERROR ${error instanceof Error ? error.stack : String(error)}`);
  check(false, 'the browser mute scenario completed');
} finally {
  if (manageServer) execFileSync(path.join(here, 'serve.sh'), ['stop']);
}

await finish(browser);

async function createHero(page, name) {
  await page.locator('button', { hasText: 'Join this room' }).first().click();
  await settle(2_000);
  await page.waitForSelector('button[aria-pressed]', { timeout: 15_000 });
  const selections = await page.evaluate((heroName) => {
    const nameInput = [...document.querySelectorAll('input[type=text],input:not([type])')].find((input) => !input.readOnly && input.getAttribute('aria-label') !== 'Room code');
    if (!nameInput) throw new Error('the hero creation form has no editable name field');
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    setValue.call(nameInput, heroName);
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));
    nameInput.dispatchEvent(new Event('change', { bubbles: true }));
    const picks = [];
    for (const heading of document.querySelectorAll('h2')) {
      const buttons = [...heading.parentElement.querySelectorAll('button[aria-pressed]')];
      const target = buttons.find((button) => button.getAttribute('aria-pressed') === 'true') || buttons[0];
      if (target && target.getAttribute('aria-pressed') !== 'true') {
        target.click();
        picks.push(`${heading.textContent.trim()}=${target.textContent.trim()}`);
      }
    }
    return picks;
  }, name);
  s.note(`${name} hero choices: ${selections.join(' | ')}`);
  await settle(800);
  await page.locator('button', { hasText: 'Confirm your hero' }).first().click();
  await settle(4_000);
  await page.waitForFunction(() => !document.body.innerText.includes('Confirm your hero'), { timeout: 15_000 });
}
