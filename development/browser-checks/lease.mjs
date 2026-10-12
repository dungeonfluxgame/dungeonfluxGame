// usage: node lease.mjs <base> <out> <pack.json>      (PLAY-F05: the idle table keeps its session)
// Starts its own server with DF_GAMEPLAY_LEASE_SECONDS=20 (the real default is 300), plays to the opening,
// leaves the table idle for 75 s (more than three lease lengths), then: a choice, a typed line from each phone
// with the NPC's answer on the phone and the TV, and a reload of a phone and of the display, which must land
// back in the story as the same hero. Then the lease and grants are forced into the past in the database (a
// suspended machine) and the same things must work again. No fence, loss or permission error may occur.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { launch, session, click, create, settle, watch, tvState } from './lib.mjs';
const here = path.dirname(fileURLToPath(import.meta.url));
const logDir = process.env.DF_CHECK_OUT || path.join(here, '..', '..', 'artifacts', 'tmp', 'browser-checks');
const [base, out, packPath] = process.argv.slice(2);
const s = session(base, out, packPath);
let failures = 0;
const check = (ok, m) => { if (!ok) failures += 1; s.note(`${ok ? 'ok  ' : 'FAIL'} ${m}`); };
const pgBin = process.env.DUNGEONFLUX_POSTGRES_BIN || path.join(logDir, '..', '..', 'build', 'postgres-g05', 'install', 'bin');
const port = process.env.DF_CHECK_PG_PORT || '55531';
const db = `df_gameplay_demo_f05_lease${String(Date.now()).slice(-6)}`;
const sql = (text) => execFileSync(path.join(pgBin, 'psql'), ['-h', '127.0.0.1', '-p', port, '-U', 'df_gameplay_demo_admin_20261004', '-d', db, '-Atc', text], { encoding: 'utf8' });
const serverLog = () => fs.readFileSync(path.join(logDir, 'server-63342.log'), 'utf8');
execFileSync(path.join(here, 'serve.sh'), ['stop']);
process.env.DF_GAMEPLAY_LEASE_SECONDS = process.env.DF_GAMEPLAY_LEASE_SECONDS || '20';
execFileSync(path.join(here, 'serve.sh'), ['start', db], { encoding: 'utf8' });
s.note(serverLog().split('\n').filter((l) => /^(TIMING|ROOM|READY)/.test(l)).join(' | '));
const runPack = path.join(here, '..', 'runs', db, 'pack.json');
if (fs.existsSync(runPack)) Object.assign(s.labels, JSON.parse(fs.readFileSync(runPack, 'utf8')).offer_labels);
const browser = await launch();
const tv = await (await browser.newContext({ viewport: { width: 1920, height: 1080 } })).newPage();
const phone = () => browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
const pa = await (await phone()).newPage();
const pb = await (await phone()).newPage();
for (const [n, p] of [['tv', tv], ['a', pa], ['b', pb]]) watch(s, p, n);
await tv.goto(`${base}/gameplay/display`);
await pa.goto(`${base}/gameplay/player`);
await pb.goto(`${base}/gameplay/player`);
await settle(2500);
await create(s, pa, 'Brakka', 'Copper beard');
await click(s, pa, 'Confirm your hero');
await create(s, pb, 'Thrain', 'Missing ear');
await click(s, pb, 'Confirm your hero');
await settle();
await click(s, pa, s.label('begin-story'));
await settle(5000);
const lease = () => sql("SELECT round(extract(epoch from lease_until - clock_timestamp())) FROM df_game.sessions").trim();
const grants = () => sql("SELECT min(round(extract(epoch from expires_at - clock_timestamp()))) FROM df_local_demo.grants").trim();
s.note(`lease remaining ${lease()} s, grants ${grants()} s`);

// ---- the idle table: three lease lengths with nobody acting ----
for (let t = 0; t < 75; t += 15) {
  await settle(15000);
  s.note(`idle ${t + 15} s: lease remaining ${lease()} s, grants ${grants()} s`);
}
check(Number(lease()) > 0 && Number(grants()) > 0, 'the lease and the grants were renewed during the idle time');

const read = (p) => p.evaluate(() => ({
  phase: document.querySelector('#display, #player')?.getAttribute('data-phase') ?? null,
  notice: document.querySelector('.df-reconnecting')?.textContent ?? null,
  text: document.body.innerText.replace(/\s+/g, ' ').slice(0, 400),
}));
const converse = async (page, name, line) => {
  await page.evaluate(() => { const d = document.querySelector('details.say-more'); if (d) d.open = true; });
  await page.selectOption('select[aria-label="Input context"]', { label: 'Conversation' });
  await page.fill('textarea.dialogue-draft', line);
  await page.locator('[data-dialogue=submit]').click();
  const until = Date.now() + 15000;
  let seen = false;
  while (Date.now() < until && !seen) {
    seen = await page.evaluate(() => !!document.querySelector('.npc-bubble-reply') && !document.querySelector('.npc-thinking'));
    await settle(400);
  }
  const answer = await page.evaluate(() => document.querySelector('.npc-bubble-reply')?.textContent ?? null);
  const message = await page.evaluate(() => document.querySelector('.dialogue-feedback')?.textContent ?? '');
  check(!!answer, `${name}'s typed line is answered on the phone: "${(answer || message).slice(0, 80)}"`);
  check(!/authorization is unavailable/i.test(message), `${name} gets no permission error (${message})`);
  return answer;
};

async function acts(label, startAt) {
  const choice = await click(s, pa, s.label('ask-courier'), { optional: true, wait: 2500 });
  check(choice, `${label}: a choice button works`);
  const st = await read(pa);
  check(!/not been saved|unknown/i.test(st.text) || st.phase !== null, `${label}: no "Outcome unknown" after the choice`);
  await converse(pb, `${label}: Thrain`, 'Tell me what you saw tonight.');
  await converse(pa, `${label}: Brakka`, 'And who sent you here?');
  const tvNow = await tvState(tv);
  check(tvNow.bubbles.some((b) => b.kind === 'speech'), `${label}: the TV shows the speaker's bubble`);
  await tv.screenshot({ path: `${out}/${startAt}-tv.png` });
  await pa.screenshot({ path: `${out}/${startAt}-phone.png`, fullPage: true });
  // reload the phone and the display: both land back in the story, as the same hero
  await pa.reload();
  await settle(4000);
  const reloaded = await read(pa);
  check(reloaded.phase !== null && !/Join your companions|Players join here/.test(reloaded.text), `${label}: a reloaded phone lands back in the story (${reloaded.text.slice(0, 60)})`);
  check(/Brakka/.test(await pa.evaluate(() => document.body.innerText)), `${label}: ...as the same hero (Brakka)`);
  await tv.reload();
  await settle(4000);
  check((await read(tv)).phase !== null, `${label}: a reloaded display shows the story`);
}
await acts('after idle', 'idle');

// ---- a suspended machine: lease and grants lapse in the database, then something happens ----
sql("UPDATE df_game.sessions SET lease_until = clock_timestamp() - interval '10 seconds'");
sql("UPDATE df_local_demo.grants SET expires_at = clock_timestamp() - interval '10 seconds'");
s.note(`forced lapse: lease remaining ${lease()} s, grants ${grants()} s`);
await acts('after a lapse', 'lapse');
check(Number(lease()) > 0 && Number(grants()) > 0, 'the lapsed lease and grants were renewed by the owner');

const log = serverLog();
const sessionLines = log.split('\n').filter((l) => /^SESSION/.test(l));
s.note(`server SESSION lines: ${JSON.stringify(sessionLines)}`);
check(!sessionLines.some((l) => /fenced|lost/.test(l)), 'the session was never fenced or lost');
fs.writeFileSync(`${out}/lease-log.txt`, s.log.join('\n') + '\n');
await browser.close();
execFileSync(path.join(here, 'serve.sh'), ['stop']);
process.exit(failures === 0 ? 0 : 1);
