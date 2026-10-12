// usage: node fight-ending.mjs <base> <out> <room>
// PLAY-E04 fight endings. Plays a fight chapter (DF_GAMEPLAY_ADVENTURE=random, a fight chapter type) on
// two phones and a display from the lobby through the fight, the resolution and the epilogue to the end
// card, and fails if any screen shows an uncertain outcome or the chapter never reaches "The End".
// The server must be fresh (a new database). Evaluator tooling, not application code.
import { openTable, makeHeroes, playUntil, ended, shot, text, options, finish } from './chapter-lib.mjs';
import { click, settle } from './lib.mjs';

const [base, out, room] = process.argv.slice(2);
if (!base || !out || !room) {
  console.error('usage: node fight-ending.mjs <base> <out> <room>');
  process.exit(2);
}
const table = await openTable(base, out, room);
const { s, tv, pa, failures } = table;
await makeHeroes(table);
const begin = (await options(table))[0];
s.note(`begin: ${begin.label}`);
await click(s, begin.page, begin.label, { wait: 9000 });

// Play to the fight's end, then record the step that follows it.
const seen = new Set();
const seenText = [];
const mark = async (t) => {
  const body = await text(t.tv);
  if (seenText.length < 200) seenText.push(body);
  // The epilogue is the step whose only story choice is to close the chapter.
  if (!seen.has('Epilogue') && (await options(t)).some((o) => /^Close the chapter/.test(o.label))) {
    seen.add('Epilogue');
    s.note('the epilogue is up: a phone offers "Close the chapter"');
    await settle(1500);
    await shot(t, 'scene-epilogue');
    if (/^victory$/im.test(await text(t.tv))) failures.push('the epilogue is drawn with the victory banner');
  }
  if (!seen.has('The End') && /^the end$/im.test(body)) {
    seen.add('The End');
    s.note('TV shows The End');
    await shot(t, 'scene-the-end');
  }
  return ended(t);
};
const reached = await playUntil(table, mark, { steps: 500, label: 'chapter' });
await settle(3000);
await shot(table, 'final');
s.note(`final TV: ${JSON.stringify(await text(tv)).slice(0, 400)}`);
if (!reached) failures.push('the fight chapter did not reach its end card on the TV');
if (!seen.has('Epilogue')) failures.push('the epilogue was never shown');
for (const [name, page] of [['A', pa], ['B', table.pb]]) {
  const phone = await text(page);
  if (!/The End/i.test(phone)) failures.push(`phone ${name} does not show the end: ${JSON.stringify(phone.slice(0, 200))}`);
}
const tvFinal = await text(tv);
if (/^victory$/im.test(tvFinal)) failures.push('the end card is drawn with the victory banner');
if (!/^the end$/im.test(tvFinal)) failures.push('the TV does not show the end card');
// Which of the four closing scenes was told, read from the epilogue text (see beat-scripts.json).
const told = ['Every hero stands', 'not every hero arrives whole', 'The fight was lost', 'no one came back', 'The road is quiet behind you', 'price of the road is plain', 'no safer now', 'keeps its dead'].find((phrase) => tvFinal.includes(phrase) || seenText.some((t) => t.includes(phrase)));
s.note(`closing told: ${told ?? 'unrecognised'}`);
await finish(table, 'fight-ending');
