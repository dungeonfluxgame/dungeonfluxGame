// usage: node talk.mjs <base> <out> <run-dir> [room]
// Evaluator tooling (not application code), PLAY-N06. Two phones join a room with the text model on.
// Hero A first draws the patron aside (receiving the private clue), then types a five-exchange
// conversation on a phone-sized viewport. The same model-free rules the Rust transcript checks use
// (npc_talk_rules) are asserted on what phone A and the TV actually show:
//   * at most three sentences and 300 characters per reply, first sentence an answer;
//   * at most one "I do not know" in the conversation, and never as an opening when a fact was due;
//   * no sentence repeated across the conversation, no speech-style tic or opener said;
//   * the TV shows the reply the phone shows, except a reply that spoke of the private clue, which
//     the table sees only as a murmur;
//   * hero B's phone and the TV never show a term only the private clue holds;
//   * no horizontal scrolling on the phone.
// <run-dir> holds the served run's adventure.json and pack.json. Exit status 1 when a rule fails.
import fs from 'node:fs';
import { launch, session, click, create, settle, watch, tvState } from './lib.mjs';

const [base, out, runDir, room] = process.argv.slice(2);
const pack = JSON.parse(fs.readFileSync(`${runDir}/pack.json`, 'utf8'));
const adventure = JSON.parse(fs.readFileSync(`${runDir}/adventure.json`, 'utf8'));
const s = session(base, out, `${runDir}/pack.json`);
const patron = adventure.dossiers[0];
const contact = adventure.dossiers[1];
const destination = (pack.destination_label || '').replace(/^the /i, '');
const lines = [
  `${patron.name}, what is the trouble? Why did you send for us?`,
  `Tell me about ${contact.name}. Can we trust them?`,
  `Who or what might stand in our way on the road to ${destination}?`,
  'How long do we have, and what should we watch for?',
  'Anything else before we set out?',
];

// --- the rules, mirroring npc_talk_rules.rs -------------------------------------------------------
const normal = (t) => t.replace(/[’‘]/g, "'").toLowerCase();
const sentences = (t) => (t.match(/[^.!?]+[.!?]+(?=\s|$)|[^.!?]+$/g) || []).map((x) => x.trim()).filter(Boolean);
const words = (t) => normal(t).split(/[^a-z0-9']+/).map((w) => w.replace(/^'+|'+$/g, '')).filter(Boolean);
const key = (t) => words(t).join(' ');
const DENIALS = ["i don't know", 'i do not know', "i can't say", 'i cannot say', "i can't tell", 'i cannot tell',
  "i couldn't say", 'i have no idea', 'no idea', "i don't have", 'i do not have', "i'm not sure", 'i am not sure',
  "i can't speak", 'i cannot speak', 'i know nothing'];
const isDenial = (t) => DENIALS.some((d) => normal(t).includes(d));
const phrases = [];
for (const d of adventure.dossiers) {
  for (const field of [d.speech_style.tic, d.speech_style.opener, d.speech_style.closer]) {
    const quoted = field.split("'")[1];
    const phrase = (quoted || field).replace(/\.$/, '').trim();
    if (phrase.split(/\s+/).length >= 2) phrases.push(phrase.toLowerCase());
  }
}
const spoken = new Set((adventure.speakers || []).map((x) => x.name.toLowerCase()));
if (pack.destination_label) spoken.add(pack.destination_label.replace(/^the /i, '').toLowerCase());
const clueOnly = (pack.secret_terms || []).filter((t) => normal(pack.courier_clue).includes(normal(t))
  && !spoken.has(t.toLowerCase()));

const findings = [];
const fail = (check, detail) => { findings.push({ check, detail }); s.note(`RULE FAILED ${check}: ${detail}`); };

const browser = await launch();
const tvCtx = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
const phone = () => browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
const tv = await tvCtx.newPage();
const pa = await (await phone()).newPage();
const pb = await (await phone()).newPage();
for (const [n, p] of [['tv', tv], ['a', pa], ['b', pb]]) watch(s, p, n);
const q = room ? `?room=${room}` : '';
await tv.goto(`${base}/gameplay/display`);
await pa.goto(`${base}/gameplay/player${q}`);
await pb.goto(`${base}/gameplay/player${q}`);
await settle(2500);
await create(s, pa, 'Brakka', 'Copper beard');
await click(s, pa, 'Confirm your hero');
await create(s, pb, 'Thrain', 'Missing ear');
await click(s, pb, 'Confirm your hero');
await settle();
await click(s, pa, s.label('begin-story'));
await settle(9000);
await tv.screenshot({ path: `${out}/01-tv-opening.png` });
// Hero A is whispered the clue; hero B is not.
await click(s, pa, s.label('ask-courier'), { optional: true });
await settle(4000);

const replies = [];
const phoneReplies = () => pa.evaluate(() => [...document.querySelectorAll('.npc-bubble-reply')].map((e) => e.textContent.trim()));
for (let i = 0; i < lines.length; i += 1) {
  await pa.evaluate(() => { const d = document.querySelector('details.say-more'); if (d) d.open = true; });
  await pa.selectOption('select[aria-label="Input context"]', { label: 'Conversation' });
  await pa.fill('textarea.dialogue-draft', lines[i]);
  const before = (await phoneReplies()).join('|');
  await pa.locator('[data-dialogue=submit]').click();
  s.note(`submitted ${i + 1}: ${lines[i]}`);
  let reply = '';
  for (let wait = 0; wait < 90; wait += 1) {
    await settle(500);
    const now = await phoneReplies();
    if (now.join('|') !== before && now.length) { reply = now[now.length - 1]; break; }
  }
  await settle(1500);
  await pa.screenshot({ path: `${out}/phone-a-${i + 1}.png`, fullPage: true });
  await tv.screenshot({ path: `${out}/tv-${i + 1}.png` });
  if (!reply) { fail('no_reply', `exchange ${i + 1} got no reply on the phone`); continue; }
  replies.push({ hero: lines[i], reply, tv: await tvState(tv) });
  s.note(`reply ${i + 1}: ${reply}`);
}

// --- assert -----------------------------------------------------------------------------------------
const seen = new Map();
let admissions = 0;
replies.forEach(({ hero, reply, tv: shown }, index) => {
  const n = index + 1;
  const parts = sentences(reply);
  if (parts.length > 3) fail('too_many_sentences', `exchange ${n}: ${parts.length}`);
  if (reply.length > 300) fail('too_long', `exchange ${n}: ${reply.length} characters`);
  if (parts[0] && isDenial(parts[0])) admissions += 1;
  for (const part of parts) {
    if (words(part).length < 4) continue;
    if (seen.has(key(part))) fail('repeated_sentence', `exchange ${n}: ${part} (first in ${seen.get(key(part))})`);
    else seen.set(key(part), n);
  }
  for (const phrase of phrases) if (normal(reply).includes(phrase)) fail('says_a_speech_style_phrase', `exchange ${n}: ${phrase}`);
  const asked = new Set(words(hero).filter((w) => w.length >= 4));
  const added = words(parts[0] || '').filter((w) => w.length >= 4 && !asked.has(w));
  if (asked.size >= 4 && added.length <= 1) fail('restates_the_question', `exchange ${n}: ${parts[0]}`);
  const onTv = shown.bubbles.some((b) => b.text.includes(reply.slice(0, 40)));
  const private_ = clueOnly.some((t) => normal(reply).includes(normal(t)));
  if (!onTv && !private_) fail('tv_does_not_show_the_reply', `exchange ${n}`);
  if (onTv && private_) fail('tv_shows_the_private_clue', `exchange ${n}`);
});
if (admissions > 1) fail('says_i_dont_know_more_than_once', `${admissions} replies`);
const overflow = await pa.evaluate(() => document.documentElement.scrollWidth > window.innerWidth + 1);
if (overflow) fail('phone_scrolls_sideways', 'horizontal overflow on the phone');
const bText = (await pb.evaluate(() => document.body.innerText)) + (await tv.evaluate(() => document.body.innerText));
for (const term of clueOnly) if (normal(bText).includes(normal(term))) fail('private_clue_reached_another_screen', term);

fs.writeFileSync(`${out}/talk.json`, JSON.stringify({ lines, replies: replies.map((r) => ({ hero: r.hero, reply: r.reply })), findings }, null, 1));
fs.writeFileSync(`${out}/timeline.txt`, `${s.log.join('\n')}\n`);
await browser.close();
s.note(findings.length ? `${findings.length} RULE FAILURE(S)` : 'all rules hold');
process.exit(findings.length ? 1 : 0);
