// usage: node fence.mjs <base> <out> <pack.json>
// PLAY-F05 defect 4 (coordinator's mechanism): a database commit that runs past its bound leaves its outcome
// unknown and fences the session. The fault is real: the server's PostgreSQL backends are SIGSTOPped while a
// player acts, then resumed. With DF_GAMEPLAY_COMMIT_SECONDS=1 the commit gives up at 1 s. The session must
// fence (SESSION fenced), heal by itself (SESSION recovered), and every page must be serving again, with no
// click from a player; a display reloaded while fenced must show "Reconnecting…", not a bare splash.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { launch, session, click, create, settle, watch } from './lib.mjs';
const here = path.dirname(fileURLToPath(import.meta.url));
const logDir = process.env.DF_CHECK_OUT || path.join(process.env.DUNGEONFLUX_ARTIFACT_ROOT || path.join(here, '..', '..', 'artifacts'), 'tmp', 'browser-checks');
const [base, out, packPath] = process.argv.slice(2);
const s = session(base, out, packPath);
let failures = 0;
const check = (ok, m) => { if (!ok) failures += 1; s.note(`${ok ? 'ok  ' : 'FAIL'} ${m}`); };
const psql = process.env.DF_PSQL || path.join(process.env.DUNGEONFLUX_POSTGRES_BIN || path.join(logDir, '..', '..', 'build', 'postgres-g05', 'install', 'bin'), 'psql');
const db = `df_gameplay_demo_f05_fence${String(Date.now()).slice(-6)}`;
const serverLog = () => fs.readFileSync(path.join(logDir, 'server-63342.log'), 'utf8');
execFileSync(path.join(here, 'serve.sh'), ['stop']);
process.env.DF_GAMEPLAY_COMMIT_SECONDS = process.env.DF_GAMEPLAY_COMMIT_SECONDS || '1';
process.env.DF_GAMEPLAY_REPLY_SECONDS = process.env.DF_GAMEPLAY_REPLY_SECONDS || '8';
execFileSync(path.join(here, 'serve.sh'), ['start', db], { encoding: 'utf8' });
s.note(serverLog().split('\n').filter((l) => /^(TIMING|ROOM|READY)/.test(l)).join(' | '));
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
await settle(6000);
const read = (p) => p.evaluate(() => ({
  phase: document.querySelector('#display, #player')?.getAttribute('data-phase') ?? null,
  notice: document.querySelector('.df-reconnecting')?.textContent ?? null,
  feedback: document.querySelector('.feedback')?.textContent?.trim() ?? '',
  splash: !!document.querySelector('.tv-splash') && !document.querySelector('.tv-stage'),
  text: document.body.innerText.replace(/\s+/g, ' ').slice(0, 80),
}));
check((await read(tv)).phase === '3', 'before the fault the story is on screen');
await tv.screenshot({ path: `${out}/01-before-fault.png` });

// ---- the fault: make the database's COMMIT slow, as it is on a saturated machine ----
const sql = (text) => execFileSync(psql, ['-h', '127.0.0.1', '-p', process.env.DF_CHECK_PG_PORT || '55531', '-U', 'df_gameplay_demo_admin_20261004', '-d', db, '-Atc', text], { encoding: 'utf8' });
const slow = process.env.SLOW_SECONDS || '3';
sql(`CREATE TABLE public.f05_flag(sleep_seconds int); INSERT INTO public.f05_flag VALUES (0)`);
sql(`CREATE FUNCTION public.f05_slow() RETURNS trigger LANGUAGE plpgsql AS $body$ BEGIN PERFORM pg_sleep((SELECT sleep_seconds FROM public.f05_flag LIMIT 1)); RETURN NULL; END $body$`);
sql(`CREATE CONSTRAINT TRIGGER f05_slow AFTER INSERT ON df_game.operations DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.f05_slow()`);
sql(`UPDATE public.f05_flag SET sleep_seconds = ${slow}`);
s.note(`commits now take ${slow} s to finish (bound ${process.env.DF_GAMEPLAY_COMMIT_SECONDS} s)`);
await pa.locator('button:not([disabled])', { hasText: s.label('ask-courier') }).first().click();
await settle(2500);
sql('UPDATE public.f05_flag SET sleep_seconds = 0');
s.note('commits are fast again');
const t0 = Date.now();
let fencedAt = null; let recoveredAt = null; let reloaded = false;
while (Date.now() - t0 < 45000) {
  const log = serverLog();
  if (!fencedAt && /SESSION fenced/.test(log)) { fencedAt = Date.now() - t0; s.note(`server: ${log.split('\n').find((l) => /SESSION fenced/.test(l))}`); }
  if (fencedAt && !reloaded) {
    reloaded = true;
    await tv.reload();
    await settle(1500);
    const st = await read(tv);
    s.note(`display reloaded while fenced: ${JSON.stringify(st)}`);
    await tv.screenshot({ path: `${out}/02-display-reloaded-while-fenced.png` });
    check(!st.splash || !!st.notice, 'a display reloaded while fenced says Reconnecting… instead of a bare splash');
  }
  if (!recoveredAt && /SESSION recovered/.test(log)) { recoveredAt = Date.now() - t0; s.note(`server: ${log.split('\n').find((l) => /SESSION recovered/.test(l))}`); break; }
  await settle(500);
}
check(fencedAt !== null, `the session fenced after the stalled commit (${fencedAt} ms after the resume)`);
check(recoveredAt !== null, `the session recovered by itself (${recoveredAt} ms after the resume), no player click`);
await settle(6000);
const after = { tv: await read(tv), a: await read(pa), b: await read(pb) };
s.note(`after: ${JSON.stringify(after)}`);
check(after.tv.phase !== null && !after.tv.notice && !after.tv.splash, 'the display serves views again');
check(after.a.phase !== null && !after.a.notice, 'player A serves views again');
check(after.b.phase !== null && !after.b.notice, 'player B serves views again');
await tv.screenshot({ path: `${out}/03-after-recovery-tv.png` });
await pa.screenshot({ path: `${out}/04-after-recovery-phone.png` });
// the game still works: take the next action
const next = s.label('escort-courier');
const clicked = await click(s, pa, s.label('defend-courier'), { optional: true, wait: 3000 }) || await click(s, pa, next, { optional: true, wait: 3000 });
s.note(`next action clicked: ${clicked}`);
const final = await read(tv);
s.note(`final display: ${JSON.stringify(final)}`);
// ---- ownership lost for good: another process took the session's owner fence ----
if (process.env.LOST === '1') {
  sql(`UPDATE df_game.sessions SET owner_fence = '\\x00112233445566778899aabbccddeeff'`);
  s.note('the session owner fence was taken over in the database');
  for (const page of [pa, pb]) await click(s, page, s.label('greatsword-attack'), { optional: true, wait: 1500 });
  let lostLine = null;
  for (let i = 0; i < 40 && !lostLine; i += 1) {
    lostLine = serverLog().split('\n').find((l) => /SESSION lost/.test(l)) || null;
    await settle(500);
  }
  s.note(`server: ${lostLine}`);
  check(!!lostLine, 'the server states that the session is lost');
  await settle(6000);
  const lost = { tv: await read(tv), a: await read(pa) };
  s.note(`lost state: ${JSON.stringify(lost)}`);
  await tv.reload();
  await settle(4000);
  const tvLost = await read(tv);
  s.note(`display reloaded after the loss: ${JSON.stringify(tvLost)}`);
  check(/restarted/.test(tvLost.notice || ''), 'the display says the game server must be restarted');
  check(/restarted/.test(lost.a.notice || '') || /restarted/.test(lost.a.feedback || ''), 'the phone says the game server must be restarted');
  await tv.screenshot({ path: `${out}/05-lost-tv.png` });
  await pa.screenshot({ path: `${out}/06-lost-phone.png` });
}
fs.writeFileSync(`${out}/fence-log.txt`, s.log.join('\n') + '\n' + serverLog().split('\n').filter((l) => /SESSION|TIMING/.test(l)).join('\n') + '\n');
await browser.close();
execFileSync(path.join(here, 'serve.sh'), ['stop']);
process.exit(failures === 0 ? 0 : 1);
