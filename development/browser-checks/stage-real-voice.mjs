// PLAY-F06: a real-voice listen-through. The real voice provider (ELEVENLABS key read from .env by serve.sh, never
// printed) and the real text model speak the opening scene, the next scene and two typed exchanges; the page's own
// recorder logs every clip start/end against every bubble and narrator-box open/close so the timing log shows each
// bubble opening when its clip starts. The log is written to <frames>/stage-real-voice-timing.txt. Spends real
// voice characters: keep the script short. usage: node stage-real-voice.mjs [frames-dir]
import fs from 'node:fs';
import { runner, startServer, launchGpu, playToOpening, click, settle, states, until, events, clearEvents, framesDir } from './stage.mjs';

const { s, out, check, finish } = runner('stage-real-voice', process.argv[2] || framesDir);
const { runPack } = await startServer({ WITH_STORYTELLER: '1', WITH_ELEVEN: '1' });
if (fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
const browser = await launchGpu(false);
const { tv, pa } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true });
await clearEvents(tv);
await click(s, pa, s.label('begin-story'), { wait: 100 });
await until(tv, (now) => now.scene === 'awaiting-choice', 120000);
await settle(2000);
await click(s, pa, s.label('ask-courier'), { wait: 100 });
await until(tv, (now) => now.title && now.title.includes('Whisper') && now.scene === 'awaiting-choice', 120000);
await settle(2000);
await pa.evaluate(() => { const d = document.querySelector('details.say-more'); if (d) d.open = true; });
await pa.selectOption('select[aria-label="Input context"]', { label: 'Conversation' });
for (const text of ['who is hunting you', 'and what is in the packet']) {
  await pa.fill('textarea.dialogue-draft', text);
  await pa.locator('[data-dialogue=submit]').click();
  const before = (await events(tv)).filter((e) => e.type === 'audio' && e.event === 'ended').length;
  const stop = Date.now() + 60000;
  while (Date.now() < stop && (await events(tv)).filter((e) => e.type === 'audio' && e.event === 'ended').length <= before) await settle(500);
  await settle(2500);
}
const log = await events(tv);
const t0 = log[0]?.t ?? 0;
const rows = [];
for (const e of log) {
  const at = ((e.t - t0) / 1000).toFixed(2).padStart(7);
  if (e.type === 'audio' && ['playing', 'ended', 'error'].includes(e.event)) rows.push(`${at}s  clip ${e.event}${e.event === 'playing' ? ` (${Number(e.duration).toFixed(1)}s long)` : ''}`);
  else if (e.type === 'bubble+') rows.push(`${at}s  bubble OPEN  ${e.kind} ${e.name}: ${e.text.slice(0, 50)}`);
  else if (e.type === 'bubble-') rows.push(`${at}s  bubble gone  ${e.kind} ${e.name}`);
  else if (e.type === 'attr' && e.name === 'data-line-state' && e.cls.includes('tv-dm')) rows.push(`${at}s  narrator box ${e.value}`);
  else if (e.type === 'attr' && e.name === 'data-scene-state') rows.push(`${at}s  scene ${e.value}`);
  else if (e.type === 'attr' && e.name === 'data-clip-state') rows.push(`${at}s  clip-state ${e.value}`);
}
const dedup = rows.filter((row, i) => i === 0 || row.slice(9) !== rows[i - 1].slice(9));
fs.writeFileSync(`${out}-timing.txt`, dedup.join('\n') + '\n');
fs.writeFileSync(`${out}-events.json`, JSON.stringify(log, null, 1));
s.note(`timing log: ${out}-timing.txt (${dedup.length} rows)`);
const plays = log.filter((e) => e.type === 'audio' && e.event === 'playing').map((e) => e.t);
const opens = log.filter((e) => (e.type === 'bubble+' && (e.kind === 'speech')) || (e.type === 'attr' && e.name === 'data-line-state' && e.cls.includes('tv-dm') && e.value === 'opening')).map((e) => e.t);
const matched = opens.filter((t) => plays.some((p) => t >= p - 50 && t - p <= 600));
check(plays.length >= 4, `real clips played (${plays.length})`);
check(matched.length >= plays.length - 1, `each voiced bubble opened as its clip started (${matched.length} of ${plays.length})`);
check(!log.some((e) => e.type === 'attr' && e.name === 'data-clip-state' && ['failed', 'skipped'].includes(e.value)), 'no clip failed or was skipped');
await finish(browser);
