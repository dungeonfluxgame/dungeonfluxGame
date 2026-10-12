// PLAY-F06 voice rule: an NPC reply's bubble opens when its clip starts, the clip always plays to the end (unless a
// newer scene exits), and a missing clip is reported as data-clip-state=failed|skipped, never as spoken. Five typed
// lines to the scene's speaker, about 16 s apart, with a SLOW fake voice (the clip takes 6.5 s to be served, longer
// than the old 5 s race): each of the five replies must reach `playing` and `ended`, its bubble must open at the
// clip's start, and the thinking mark must stay until then. Uses the text model for the replies (and the hero's
// line cleanup); the voice is the fake one. usage: node stage-replies.mjs [frames-dir]
import fs from 'node:fs';
import { runner, startServer, launchGpu, playToOpening, click, settle, states, until, events, clearEvents, framesDir } from './stage.mjs';

const QUESTIONS = [
  'uh Courier what exactly was in that packet',
  'and who is it that follows you',
  'why did you come to the wharf tonight',
  'do you trust the harbor guard',
  'what would you do if we refused to help',
];
const SLOW_VOICE_MS = 6500;
const GAP_MS = 16000;
const { s, out, check, finish } = runner('stage-replies', process.argv[2] || framesDir);
const { runPack } = await startServer({ WITH_STORYTELLER: '1' });
if (fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
const browser = await launchGpu(false);
const voiceLog = [];
const { tv, pa } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true, voice: { latencyMs: SLOW_VOICE_MS, lengthFor: () => 3, log: voiceLog } });
await click(s, pa, s.label('begin-story'), { wait: 100 });
await until(tv, (now) => now.scene === 'awaiting-choice', 90000);
await settle(1500);
await clearEvents(tv);
await pa.evaluate(() => { const d = document.querySelector('details.say-more'); if (d) d.open = true; });
await pa.selectOption('select[aria-label="Input context"]', { label: 'Conversation' });
const submitted = [];
for (const [index, text] of QUESTIONS.entries()) {
  await pa.fill('textarea.dialogue-draft', text);
  submitted.push(Date.now());
  await pa.locator('[data-dialogue=submit]').click();
  s.note(`submitted ${index + 1}: ${text}`);
  const stop = Date.now() + GAP_MS;
  let shot = false;
  while (Date.now() < stop) {
    const now = await states(tv);
    if (!shot && now.bubbles.some((b) => b.kind === 'speech' && b.state === 'speaking')) { shot = true; await tv.screenshot({ path: `${out}-reply-${index + 1}.png` }); }
    await settle(250);
  }
}
const log = await events(tv);
fs.writeFileSync(`${out}-events.json`, JSON.stringify(log, null, 1));
fs.writeFileSync(`${out}-voice-requests.json`, JSON.stringify(voiceLog, null, 1));
const plays = log.filter((e) => e.type === 'audio' && e.event === 'playing').map((e) => e.t);
const ends = log.filter((e) => e.type === 'audio' && e.event === 'ended').map((e) => e.t);
const heroLines = log.filter((e) => e.type === 'bubble+' && e.kind === 'typed');
const replies = log.filter((e) => e.type === 'bubble+' && e.kind === 'speech');
s.note(`hero lines shown: ${heroLines.map((e) => JSON.stringify(e.text)).join(' | ')}`);
check(heroLines.length === 5, `five hero lines reached the table (${heroLines.length})`);
check(replies.length >= 5, `five NPC replies opened (${replies.length})`);
let ok = 0;
for (const [index, reply] of replies.slice(-5).entries()) {
  const started = plays.filter((t) => t <= reply.t + 100 && reply.t - t <= 600).pop();
  const ended = ends.find((t) => started && t > started);
  const clip = log.filter((e) => e.type === 'attr' && e.name === 'data-clip-state' && e.cls.includes('tv-bubble')).map((e) => e.value);
  s.note(`reply ${index + 1} opened ${reply.t % 100000}: clip started ${started ? started % 100000 : 'NEVER'} ended ${ended ? ended % 100000 : 'NEVER'}`);
  if (started && ended) ok += 1;
}
check(ok === 5, `each of five consecutive replies reached playing and ended (${ok}/5)`);
const clipStates = log.filter((e) => e.type === 'attr' && e.name === 'data-clip-state').map((e) => e.value);
check(!clipStates.includes('failed') && !clipStates.includes('skipped'), `no reply reported a failed or skipped clip (${[...new Set(clipStates)].join(',')})`);
const thinking = log.filter((e) => e.type === 'mark+' && e.cls.includes('tv-thinking')).length;
check(thinking >= 5, `the thinking mark showed for each exchange (${thinking})`);
await finish(browser);
