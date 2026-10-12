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
const mark = async (t) => {
  const body = await text(t.tv);
  for (const word of ['Epilogue', 'The End']) {
    if (new RegExp(`^${word}$`, 'im').test(body) && !seen.has(word)) {
      seen.add(word);
      s.note(`TV shows ${word}`);
      await shot(t, `scene-${word.toLowerCase().replace(' ', '-')}`);
    }
  }
  return ended(t);
};
const reached = await playUntil(table, mark, { steps: 500, label: 'chapter' });
await settle(3000);
await shot(table, 'final');
s.note(`final TV: ${JSON.stringify(await text(tv)).slice(0, 400)}`);
if (!reached) failures.push('the fight chapter did not reach its end card on the TV');
if (!seen.has('Epilogue')) failures.push('the epilogue was never shown');
const phone = await text(pa);
if (!/The End/i.test(phone)) failures.push(`phone A does not show the end: ${JSON.stringify(phone.slice(0, 200))}`);
await finish(table, 'fight-ending');
