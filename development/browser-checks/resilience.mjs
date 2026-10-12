// usage: node resilience.mjs <base> <out>      (PLAY-F05 defects 4 and 5; starts and stops its own servers)
// Needs serve.sh next to it. Checks: per-run room code on the display, the join form and the ?room= link;
// a wrong code is refused; a server restart of the same run keeps its code and every page reconnects with a
// visible "Reconnecting…"; a seat saved in another run (same pinned code) is discarded for a clean join screen.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { launch, settle } from './lib.mjs';
const here = path.dirname(fileURLToPath(import.meta.url));
const logDir = process.env.DF_CHECK_OUT || path.join(here, '..', '..', 'artifacts', 'tmp', 'browser-checks');
const [base, out] = process.argv.slice(2);
fs.mkdirSync(out, { recursive: true });
const log = [];
let failures = 0;
const note = (m) => { log.push(m); console.log(m); };
const check = (ok, m) => { if (!ok) failures += 1; note(`${ok ? 'ok  ' : 'FAIL'} ${m}`); };
const serve = (...args) => execFileSync(path.join(here, 'serve.sh'), args, { env: { ...process.env, ...(globalThis.extraEnv || {}) }, encoding: 'utf8' });
const stamp = () => String(Date.now()).slice(-6);
const readyCode = () => {
  const text = fs.readFileSync(path.join(logDir, 'server-63342.log'), 'utf8');
  return (/^READY .* room=(\S+)$/m.exec(text) || [])[1];
};
const state = (page) => page.evaluate(() => {
  const root = document.querySelector('#display, #player');
  return {
    phase: root?.getAttribute('data-phase') ?? null,
    reconnecting: !!document.querySelector('.df-reconnecting'),
    feedback: document.querySelector('.feedback')?.textContent?.trim() ?? '',
    code: document.querySelector('input[readonly], .join-form input, input')?.value ?? null,
    text: document.body.innerText.replace(/\s+/g, ' ').slice(0, 120),
  };
});
const until = async (page, test, ms = 30000) => {
  const end = Date.now() + ms;
  let st;
  while (Date.now() < end) { st = await state(page); if (test(st)) return { ok: true, st }; await settle(500); }
  return { ok: false, st };
};

const browser = await launch();
const tvCtx = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
const plCtx = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
const tv = await tvCtx.newPage();
const pl = await plCtx.newPage();

// ---- part 1: a generated, per-run room code ----
serve('stop');
const dbA = `df_gameplay_demo_f05_res${stamp()}`;
serve('start', dbA);
const code = readyCode();
note(`READY room code: ${code}`);
check(/^[BDFGKMNPRSTVZ][AEOU]([BDFGKMNPRSTVZ][AEOU]){2}$/.test(code || ''), 'the generated code is three consonant-vowel syllables');
check(code !== 'LANTERN', 'the code is not LANTERN');
await tv.goto(`${base}/gameplay/display`);
await settle(2500);
await tv.screenshot({ path: `${out}/display-code.png` });
const tvText = await tv.evaluate(() => document.body.innerText);
check(tvText.includes(code), `the display's join panel shows ${code}`);
await pl.goto(`${base}/gameplay/player`);
await settle(2500);
let st = await state(pl);
check(st.code === code, `the join form is filled with ${code} (${st.code})`);
// a wrong typed code is refused with a visible message
await pl.evaluate(() => {
  const input = document.querySelector('input');
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, 'WRONG1');
  input.dispatchEvent(new Event('input', { bubbles: true }));
});
await pl.locator('button:not([disabled])', { hasText: 'Join this room' }).first().click();
let wrong = await until(pl, (s) => /not available/i.test(s.feedback), 12000);
check(wrong.ok, `a wrong code is refused: "${wrong.st.feedback}"`);
// the right code, lower case, with the link form
await pl.goto(`${base}/gameplay/player?room=${code.toLowerCase()}`);
await settle(2000);
st = await state(pl);
check(st.code === code, `?room= link fills the form in upper case (${st.code})`);
await pl.locator('button:not([disabled])', { hasText: 'Join this room' }).first().click();
let joined = await until(pl, (s) => s.phase !== null, 15000);
check(joined.ok, `joining with the code works (phase ${joined.st.phase})`);
await pl.screenshot({ path: `${out}/player-joined.png` });

// ---- part 2: the server restarts (same run); pages say Reconnecting… and recover ----
serve('stop');
await settle(7000);
const downTv = await state(tv);
const downPl = await state(pl);
check(downTv.reconnecting, 'the display shows "Reconnecting…" while the server is down');
check(downPl.reconnecting, 'the player page shows "Reconnecting…" while the server is down');
await tv.screenshot({ path: `${out}/display-reconnecting.png` });
serve('start', dbA);
check(readyCode() === code, `a restart of the same run keeps its room code (${readyCode()})`);
const backTv = await until(tv, (s) => s.phase !== null && !s.reconnecting, 60000);
check(backTv.ok, `the display reconnected after the restart: ${JSON.stringify(backTv.st)}`);
const backPl = await until(pl, (s) => !s.reconnecting && (s.phase !== null || /Join this room/.test(s.text)), 60000);
check(backPl.ok, `the player page recovered after the restart: ${JSON.stringify(backPl.st)}`);
await tv.screenshot({ path: `${out}/display-recovered.png` });

// ---- part 3: a seat saved in another run, same pinned code, is discarded ----
serve('stop');
globalThis.extraEnv = { DF_GAMEPLAY_ROOM_CODE: 'LANTERN' };
const dbB = `df_gameplay_demo_f05_res${stamp()}`;
serve('start', dbB);
check(readyCode() === 'LANTERN', 'DF_GAMEPLAY_ROOM_CODE pins the code');
const tab = await (await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true })).newPage();
await tab.goto(`${base}/gameplay/player`);
await settle(2000);
await tab.locator('button:not([disabled])', { hasText: 'Join this room' }).first().click();
const first = await until(tab, (s) => s.phase !== null, 15000);
check(first.ok, 'a seat was joined in run B');
const saved = await tab.evaluate(() => Object.keys(sessionStorage).map((k) => `${k}=${sessionStorage.getItem(k).slice(0, 12)}…`));
note(`saved seat: ${JSON.stringify(saved)}`);
serve('stop');
const dbC = `df_gameplay_demo_f05_res${stamp()}`;
serve('start', dbC);
await tab.reload();
const clean = await until(tab, (s) => /Players join here/.test(s.text) && s.phase === null, 20000);
check(clean.ok, `a seat from another run is discarded and the join screen is clean: ${JSON.stringify(clean.st)}`);
await tab.screenshot({ path: `${out}/stale-seat-clean-join.png` });
// and joining in the new run works
await tab.locator('button:not([disabled])', { hasText: 'Join this room' }).first().click();
const again = await until(tab, (s) => s.phase !== null, 15000);
check(again.ok, 'joining the new run works');

serve('stop');
fs.writeFileSync(`${out}/resilience-log.txt`, log.join('\n') + '\n');
await browser.close();
note(failures === 0 ? 'PASS' : `FAIL ${failures} checks`);
process.exit(failures === 0 ? 0 : 1);
