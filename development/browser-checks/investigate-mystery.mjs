// usage: node investigate-mystery.mjs <base> <out> <room> [mystery|parley]
// Plays a mystery chapter (DF_GAMEPLAY_ADVENTURE=random DF_GAMEPLAY_CHAPTER_TYPE=mystery) from the
// first choice to its conclusion on two phones, and fails if any screen shows an uncertain outcome.
// The server must be fresh (a new database): the room starts at its first beat.
// Authored labels vary per chapter, so the question and onward options are found by their place in
// the script's order rather than by fixed wording. Evaluator tooling, not application code.
import fs from 'node:fs';
import { launch, session, click, create, settle, watch } from './lib.mjs';

const [base, out, room, kind = 'mystery'] = process.argv.slice(2);
if (!base || !out || !room || !['mystery', 'parley'].includes(kind)) {
  console.error('usage: node investigate-mystery.mjs <base> <out> <room> [mystery|parley]');
  process.exit(2);
}
fs.mkdirSync(out, { recursive: true });
const s = session(base, out, null);
const browser = await launch();
const phone = () => browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
const tv = await (await browser.newContext({ viewport: { width: 1920, height: 1080 } })).newPage();
const pa = await (await phone()).newPage();
const pb = await (await phone()).newPage();
for (const [n, p] of [['tv', tv], ['a', pa], ['b', pb]]) watch(s, p, n);
const failures = [];
const UNCERTAIN = /Outcome unknown|may not have been saved/;
const SYSTEM = /^(PLAY|Hero|HERO|MENU|Reconnect|Confirm saved choice|Leave this game on this device|Close|Full character sheet|Say something.*|)$/;
const LOOK_OR_READ = /^Look at |^Read the /;
const LEAVE = /^Follow the trail$/;
const check = async (step) => {
  for (const [name, page] of [['tv', tv], ['a', pa], ['b', pb]]) {
    if (UNCERTAIN.test(await page.locator('body').innerText())) {
      failures.push(`${step}: ${name} shows an uncertain outcome`);
    }
  }
};
const enabled = (page) =>
  page.locator('button:not([disabled])').allInnerTexts().then((all) => all.map((t) => t.trim()));
const tvText = () => tv.locator('body').innerText();
// The end card is the TV's scene titled "The End" (its text may be upper-cased by CSS).
const concluded = async () => /^the end$/im.test(await tvText());

// Polls both phones for the first enabled option that the predicate accepts (chips appear only
// after the TV finishes its lines, so a single read is not enough).
const findOption = async (accept, ms = 30000) => {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    for (const [who, page] of [['a', pa], ['b', pb]]) {
      const label = (await enabled(page)).find((t) => !SYSTEM.test(t) && accept(t));
      if (label) return { who, page, label };
    }
    await settle(500);
  }
  return null;
};
// Presses the chosen option and waits for the TV's scene to change; the TV renders in turns.
const press = async (step, accept) => {
  const found = await findOption(accept);
  if (!found) {
    s.note(`${step}: a options ${JSON.stringify(await enabled(pa))}`);
    s.note(`${step}: b options ${JSON.stringify(await enabled(pb))}`);
    failures.push(`${step}: no matching option on either phone`);
    return false;
  }
  const before = await tvText();
  s.note(`${step}: press ${found.who}: ${found.label}`);
  await click(s, found.page, found.label, { wait: 1000 });
  const until = Date.now() + 30000;
  while (Date.now() < until && (await tvText()) === before) await settle(500);
  await settle(1500);
  s.note(`${step}: phone a after: ${JSON.stringify((await pa.locator('body').innerText()).slice(0, 260))}`);
  await check(`${step}: ${found.label}`);
  await tv.screenshot({ path: `${out}/${step}-tv.png` });
  return true;
};

await tv.goto(`${base}/gameplay/display`);
await pa.goto(`${base}/gameplay/player?room=${room}`);
await pb.goto(`${base}/gameplay/player?room=${room}`);
await settle(2500);
await create(s, pa, 'Brakka', 'Copper beard');
await click(s, pa, 'Confirm your hero');
await create(s, pb, 'Thrain', 'Missing ear');
await click(s, pb, 'Confirm your hero');
await settle();
const begin = (await enabled(pa)).find((t) => !SYSTEM.test(t) && !/^Look/.test(t));
s.note(`begin: ${begin}`);
await click(s, pa, begin, { wait: 9000 });

// The opening offers look, read, question and leave. Looks are pressed twice: a repeated look
// stays in its observed beat, and the question is still offered after it.
if (kind === 'mystery') {
  await press('look-1', (t) => t === 'Look at the ground');
  await press('read-1', (t) => /^Read the /.test(t));
  await press('look-2', (t) => t === 'Look at the ground');
}
// A parley's opening offers only the two courier answers, so its first press is the question.
await press('question', (t) => (kind === 'mystery' ? !LOOK_OR_READ.test(t) && !LEAVE.test(t) : true) && !/^Say something/.test(t));
await page2Screens('ask');
// After the private answer the story moves on; it ends only at the destination.
for (let turn = 0; turn < 12; turn += 1) {
  if (await concluded()) break;
  // Leave the scene when it is offered; otherwise take whatever the story offers next.
  const leave = await findOption((t) => LEAVE.test(t), 3000);
  const found = leave ?? (await findOption((t) => !LOOK_OR_READ.test(t)));
  if (!found) {
    // No option left: the chapter may simply be ending while the TV finishes its last scene.
    const until = Date.now() + 30000;
    while (Date.now() < until && !(await concluded())) await settle(500);
    if (!(await concluded())) failures.push(`turn ${turn}: nothing onward offered`);
    break;
  }
  const pick = found.label;
  await press(`turn-${turn}`, (t) => t === pick);
}
await settle(3000);
await tv.screenshot({ path: `${out}/99-tv-final.png` });
await pa.screenshot({ path: `${out}/99-phone-a-final.png`, fullPage: true });
await pb.screenshot({ path: `${out}/99-phone-b-final.png`, fullPage: true });
s.note(`final TV: ${JSON.stringify(await tvText()).slice(0, 400)}`);
if (!(await concluded())) failures.push('the chapter did not reach its conclusion on the TV');
fs.writeFileSync(`${out}/timeline.txt`, s.log.join('\n') + '\n');
await browser.close();
if (failures.length) {
  console.error(`FAILED:\n${failures.join('\n')}`);
  process.exit(1);
}
console.log(`PASSED: ${kind} chapter played to its conclusion without an uncertain outcome.`);

async function page2Screens(name) {
  await tv.screenshot({ path: `${out}/${name}-tv-after.png` });
  for (const [who, page] of [['a', pa], ['b', pb]]) {
    await page.screenshot({ path: `${out}/${name}-phone-${who}.png`, fullPage: true });
  }
}
