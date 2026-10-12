// PLAY-N04 acceptance (7): a messy typed line ("uhh  courier whats  so special abt the packet") appears in hero 1's
// bubble as a clean sentence (and in every phone's conversation log), after the loader bubble and before the
// thinking mark and the NPC's reply, in that order; a line sent as "About the game" shows nothing on the display.
// The text model cleans the line (real model; the voice is the fake one). The raw line is what the speaker answers.
// usage: node stage-clean.mjs [frames-dir]
import fs from 'node:fs';
import { runner, startServer, launchGpu, playToOpening, click, settle, states, until, events, clearEvents, framesDir } from './stage.mjs';

const RAW = 'uhh  courier whats  so special abt the packet';
const { s, out, check, finish } = runner('stage-clean', process.argv[2] || framesDir);
const { runPack } = await startServer({ WITH_STORYTELLER: '1' });
if (fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
const browser = await launchGpu(false);
const { tv, pa, pb } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true, voice: { latencyMs: 300, lengthFor: () => 2 } });
await click(s, pa, s.label('begin-story'), { wait: 100 });
await until(tv, (now) => now.scene === 'awaiting-choice', 90000);
await settle(1500);
await clearEvents(tv);
await pa.evaluate(() => { const d = document.querySelector('details.say-more'); if (d) d.open = true; });
await pa.selectOption('select[aria-label="Input context"]', { label: 'Conversation' });
await pa.locator('textarea.dialogue-draft').click();
await pa.keyboard.type(RAW, { delay: 25 });
await pa.locator('[data-dialogue=submit]').click();
const started = Date.now();
let shot = 0;
const stop = Date.now() + 45000;
while (Date.now() < stop) {
  const now = await states(tv);
  if (shot === 0 && now.intents.length) { shot = 1; await tv.screenshot({ path: `${out}-1-loader.png` }); }
  if (shot === 1 && now.bubbles.some((b) => b.kind === 'typed')) { shot = 2; await tv.screenshot({ path: `${out}-2-clean-line.png` }); }
  if (shot === 2 && now.bubbles.some((b) => b.kind === 'speech' && b.state === 'speaking')) { shot = 3; await tv.screenshot({ path: `${out}-3-reply.png` }); }
  if (shot === 3 && now.scene === 'awaiting-choice') break;
  await settle(150);
}
const log = await events(tv);
fs.writeFileSync(`${out}-events.json`, JSON.stringify(log, null, 1));
const first = (match) => log.find(match)?.t;
const loader = first((e) => e.type === 'bubble+' && e.kind === 'intent');
const hero = log.find((e) => e.type === 'bubble+' && e.kind === 'typed');
const thinking = first((e) => e.type === 'mark+' && String(e.cls).includes('tv-thinking'));
const reply = log.find((e) => e.type === 'bubble+' && e.kind === 'speech');
s.note(`hero bubble: ${JSON.stringify(hero?.text)}`);
check(Boolean(loader), 'the loader bubble showed on the hero');
check(Boolean(hero), 'the hero line appeared');
check(Boolean(hero) && !/uh+/i.test(hero.text) && !/\s{2}/.test(hero.text) && /^[A-Z]/.test(hero.text) && /[.?!]$/.test(hero.text), 'it is a clean sentence (no filler, capitalised, punctuated)');
check(Boolean(hero) && /whats|what's/i.test(hero.text) && /packet/i.test(hero.text) && /courier/i.test(hero.text), 'it keeps the meaning and who is addressed');
check(Boolean(hero) && !/abt/i.test(hero.text), 'the slip "abt" is fixed');
check(Boolean(loader && hero && loader < hero.t), 'order: loader before the clean line');
check(Boolean(thinking && hero && thinking >= hero.t), 'order: the clean line before the thinking mark');
check(Boolean(reply && thinking && reply.t >= thinking), 'order: the thinking mark before the reply');
const phone = await pa.evaluate(() => [...document.querySelectorAll('.table-line')].map((row) => row.textContent.trim().replace(/\s+/g, ' ')));
s.note(`phone log: ${JSON.stringify(phone)}`);
check(phone.some((row) => hero && row.includes(hero.text.slice(0, 20))), 'the phone conversation log shows the same cleaned line');
check(!phone.some((row) => /uhh/.test(row)), 'the raw filler is not in the phone log');

// A meta line shows nothing.
await clearEvents(tv);
await pa.selectOption('select[aria-label="Input context"]', { label: 'About the game' });
await pa.fill('textarea.dialogue-draft', 'um how do the rules for initiative work');
await pa.locator('[data-dialogue=submit]').click();
await settle(6000);
const metaLog = await events(tv);
check(!metaLog.some((e) => (e.type === 'bubble+' && (e.kind === 'typed' || e.kind === 'intent'))), 'a meta line shows nothing on the display');
await finish(browser);
