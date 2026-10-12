// PLAY-N04 acceptance (6): the speech-intent bubble. Typing on phone A shows the typing loader on hero 1 of the
// display within about a second; hold-to-talk on phone B (fake microphone) shows the speaking variant on hero 2;
// both at once each have their own loader; closing a tab clears its loader within the expiry; "About the game"
// and "Plan only" show nothing; and none of it commits anything or moves the game revision (checked in
// PostgreSQL). usage: node stage-intent.mjs [frames-dir]
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { runner, startServer, launchGpu, playToOpening, click, settle, states, until, framesDir, artifactRoot } from './stage.mjs';

const DISPLAY_BOUND_MS = 1500;
const EXPIRY_BOUND_MS = 8000;
const { s, out, check, finish } = runner('stage-intent', process.argv[2] || framesDir);
const { db, runPack } = await startServer();
if (fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
const pgBin = process.env.DUNGEONFLUX_POSTGRES_BIN || path.join(artifactRoot, 'build', 'postgres-g05', 'install', 'bin');
const sql = (text) => execFileSync(path.join(pgBin, 'psql'), ['-h', '127.0.0.1', '-p', process.env.DF_CHECK_PG_PORT || '55531', '-U', 'df_gameplay_demo_admin_20261004', '-d', db, '-Atc', text], { encoding: 'utf8' }).trim();
const revision = () => sql('SELECT recovery_epoch, in_epoch_sequence, owner_fence, (SELECT count(*) FROM df_game.operations), (SELECT count(*) FROM df_game.checkpoints), (SELECT count(*) FROM df_game.facts) FROM df_game.sessions');

const browser = await launchGpu(false, ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream']);
// A fake speech service and a fake microphone (see the launch flags): the control appears and a hold records.
const phoneSetup = async (page) => {
  await page.route('**/gameplay/speech/status', (route) => route.fulfill({ status: 200, headers: { 'x-df-speech': 'available' }, body: '' }));
  await page.route('**/gameplay/speech', (route) => route.fulfill({ status: 200, headers: { 'x-df-speech': 'nothing_heard' }, body: '' }));
};
const { tv, pa, pb } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: false, phoneSetup });
await click(s, pa, s.label('begin-story'), { wait: 100 });
await until(tv, (now) => now.scene === 'awaiting-choice', 60000);
await settle(1500);

const prepare = async (page, label) => {
  await page.evaluate(() => { const d = document.querySelector('details.say-more'); if (d) d.open = true; });
  await page.selectOption('select[aria-label="Input context"]', { label });
};
await prepare(pa, 'Conversation');
await prepare(pb, 'Conversation');
const before = revision();
s.note(`game state before: ${before}`);

const waitIntent = async (wanted, timeout = DISPLAY_BOUND_MS) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const now = await states(tv);
    if (wanted(now.intents)) return Date.now() - start;
    await settle(50);
  }
  return null;
};

// ---- typing on phone A
await pa.locator('textarea.dialogue-draft').click();
const typedAt = Date.now();
const typing = pa.keyboard.type('Garick, what is so special about the whistle', { delay: 40 });
const shownIn = await waitIntent((list) => list.some((i) => i.startsWith('hero-1:typing')), 3000);
s.note(`typing bubble on the display ${shownIn} ms after the first key`);
check(shownIn !== null && shownIn <= DISPLAY_BOUND_MS, `typing on phone A shows the typing loader on hero 1 within about a second (${shownIn} ms)`);
await typing;
await tv.screenshot({ path: `${out}-1-typing.png` });

// ---- hold to talk on phone B while A is still typing
const hold = pb.locator('[data-talk=hold]');
await hold.waitFor({ state: 'visible', timeout: 15000 });
// A real key held on the focused control (Space), the control's keyboard form of hold-to-talk.
await hold.focus();
await pb.keyboard.down(' ');
await settle(600);
s.note(`debug env: ${JSON.stringify(await pb.evaluate(() => ({ secure: window.isSecureContext, recorder: typeof MediaRecorder, devices: Boolean(navigator.mediaDevices), status: document.querySelector('.talk-status')?.textContent })))}`);
s.note(`debug: talk state ${await pb.locator('.talk-row').getAttribute('data-state')}; presence A ${await pa.locator('#player').getAttribute('data-presence')} B ${await pb.locator('#player').getAttribute('data-presence')}; tv ${JSON.stringify((await states(tv)).intents)}`);
const bothIn = await waitIntent((list) => list.some((i) => i.startsWith('hero-1:typing')) && list.some((i) => i.startsWith('hero-2:speaking')), 3000);
check(bothIn !== null, `two players composing at once each show their own loader (${bothIn} ms)`);
await tv.screenshot({ path: `${out}-2-both.png` });
const mid = await states(tv);
s.note(`both: ${JSON.stringify(mid.intents)}`);
await pb.keyboard.up(' ');
const clearedB = await waitIntent((list) => !list.some((i) => i.startsWith('hero-2')), 3000);
check(clearedB !== null, `releasing hold-to-talk clears the speaking loader (${clearedB} ms)`);

// ---- game state untouched
const after = revision();
s.note(`game state after: ${after}`);
check(after === before, 'typing and talking committed nothing and moved no revision');

// ---- the tab closes while typing: the loader expires
const closedAt = Date.now();
await pa.context().close();
const gone = await waitIntent((list) => !list.some((i) => i.startsWith('hero-1')), EXPIRY_BOUND_MS + 2000);
s.note(`loader gone ${Date.now() - closedAt} ms after the tab closed`);
check(gone !== null && Date.now() - closedAt <= EXPIRY_BOUND_MS, `a closed tab never leaves a stuck bubble (gone ${Date.now() - closedAt} ms after close, bound ${EXPIRY_BOUND_MS})`);
await tv.screenshot({ path: `${out}-3-after-close.png` });

// ---- table talk shows nothing
for (const label of ['About the game', 'Plan only']) {
  await prepare(pb, label);
  await pb.locator('textarea.dialogue-draft').fill('');
  await pb.locator('textarea.dialogue-draft').click();
  await pb.keyboard.type('what are the rules for this', { delay: 30 });
  const shown = await waitIntent((list) => list.length > 0, 2500);
  check(shown === null, `"${label}" shows no bubble`);
}
// ...and Conversation shows it again for the same phone.
await prepare(pb, 'Conversation');
await pb.locator('textarea.dialogue-draft').fill('');
await pb.locator('textarea.dialogue-draft').click();
await pb.keyboard.type('Who is hunting you', { delay: 30 });
check((await waitIntent((list) => list.some((i) => i.startsWith('hero-2:typing')), 3000)) !== null, 'Conversation shows hero 2 typing again');
check(revision() === before, 'still no game state moved');
await finish(browser);
