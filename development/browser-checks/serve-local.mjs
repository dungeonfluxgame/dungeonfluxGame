// CAPTION-001 evaluator tooling only (not application code): a Windows-safe equivalent of
// serve.sh start/stop for the caption checks. Node on Windows cannot spawn serve.sh (EFTYPE),
// so this replicates what it does: create the database, launch this checkout's gameplay demo
// on the check's own port against the check's own PostgreSQL and build, wait for READY, and
// warm the client bundle. It never touches the shared browser-checks server.pid: pid and logs
// live under DUNGEONFLUX_ARTIFACT_ROOT. Usage mirrors startServer: `const { db, runPack } =
// await startLocal({ DF_GAMEPLAY_TEST_FIGHT: '1', ... })`, then `await stopLocal()`.
import fs from 'node:fs';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.join(here, '..', '..');
const R = process.env.DUNGEONFLUX_ARTIFACT_ROOT || path.join(repo, 'artifacts');
const TARGET = process.env.DUNGEONFLUX_BUILD_ROOT || path.join(R, 'build', 'play');
const PG = process.env.DUNGEONFLUX_POSTGRES_BIN || path.join(repo, 'artifacts', 'build', 'postgres-g05', 'install', 'bin');
const PORT = process.env.PORT || '63379';
const PGPORT = process.env.DF_CHECK_PG_PORT || '55587';
const ADMIN = 'df_gameplay_demo_admin_20261004';
const O = path.join(R, 'tmp', 'browser-checks');
const WEB = path.join(TARGET, 'f05-web');
const BIN = path.join(TARGET, 'debug', process.platform === 'win32' ? 'df-transport-fixture.exe' : 'df-transport-fixture');

function run(bin, args) {
  return spawnSync(bin, args, { encoding: 'utf8' });
}

export async function startLocal(extraEnv = {}) {
  fs.mkdirSync(O, { recursive: true });
  const db = `df_gameplay_demo_caption_${String(Date.now()).slice(-7)}`;
  const psql = path.join(PG, process.platform === 'win32' ? 'psql.exe' : 'psql');
  const exists = run(psql, ['-h', '127.0.0.1', '-p', PGPORT, '-U', ADMIN, '-d', 'postgres', '-Atc', `SELECT 1 FROM pg_database WHERE datname='${db}'`]);
  if (exists.error) throw new Error(`postgres check failed: ${exists.error.message} (${psql})`);
  if (!String(exists.stdout || '').includes('1')) {
    const made = run(path.join(PG, process.platform === 'win32' ? 'createdb.exe' : 'createdb'), ['-h', '127.0.0.1', '-p', PGPORT, '-U', ADMIN, '-T', 'template0', db]);
    if (made.status !== 0) throw new Error(`createdb ${db} failed: ${made.stderr || made.error}`);
  }
  const base = `http://127.0.0.1:${PORT}`;
  const log = path.join(O, `server-${PORT}.log`);
  fs.writeFileSync(log, '');
  const child = spawn(BIN, ['--gameplay-demo', WEB, path.join(repo, 'assets', 'ui')], {
    cwd: repo,
    detached: true,
    stdio: ['ignore', fs.openSync(log, 'a'), fs.openSync(log, 'a')],
    env: {
      ...process.env,
      ...extraEnv,
      DF_GAMEPLAY_PACK_DIR: process.env.DF_GAMEPLAY_PACK_DIR || path.join(repo, 'content', 'harbor-journey'),
      DF_GAMEPLAY_ADVENTURE: process.env.DF_GAMEPLAY_ADVENTURE || 'harbor',
      DF_GAMEPLAY_VOICE_CASTING: path.join(repo, 'content', 'voice-casting.json'),
      DF_GAMEPLAY_DEMO_DATABASE: db,
      DF_GAMEPLAY_LISTEN: `127.0.0.1:${PORT}`,
      DF_GAMEPLAY_DATABASE_PORT: PGPORT,
      DF_GAMEPLAY_TEXT_CACHE: path.join(O, 'text-cache'),
      DF_GAMEPLAY_VOICE_CACHE: path.join(O, 'voice-cache'),
    },
  });
  child.unref();
  fs.writeFileSync(path.join(O, 'server.pid'), String(child.pid));
  const stop = Date.now() + 30000;
  while (Date.now() < stop) {
    const text = fs.readFileSync(log, 'utf8');
    if (/^READY/m.test(text)) break;
    await new Promise((r) => setTimeout(r, 500));
  }
  const ready = fs.readFileSync(log, 'utf8').split('\n').filter((l) => /^(READY|STORYTELLER|VOICE|GAMEPLAY PACK|ADVENTURE)/.test(l)).map((l) => l.slice(0, 200)).join('\n');
  console.log(`${ready}\nDB=${db}`);
  for (const route of ['/gameplay/display', '/gameplay/player', '/pkg/df_tools.js', '/pkg/df_tools_bg.wasm']) {
    await fetch(`${base}${route}`).then((response) => response.arrayBuffer());
  }
  return { db, runPack: path.join(repo, 'development', 'runs', db, 'pack.json') };
}

export async function stopLocal() {
  const pidFile = path.join(O, 'server.pid');
  if (!fs.existsSync(pidFile)) return;
  const pid = Number(fs.readFileSync(pidFile, 'utf8'));
  try { process.kill(pid, 'SIGINT'); } catch { try { process.kill(pid); } catch { /* already gone */ } }
  fs.rmSync(pidFile, { force: true });
}

// A crashed check must not orphan its server on the shared machine: best-effort stop on exit.
// The normal path already stopped it (the pid file is gone) and this is a no-op.
process.on('exit', () => {
  try {
    const pidFile = path.join(O, 'server.pid');
    if (!fs.existsSync(pidFile)) return;
    try { process.kill(Number(fs.readFileSync(pidFile, 'utf8')), 'SIGKILL'); } catch { /* already gone */ }
    fs.rmSync(pidFile, { force: true });
  } catch { /* exit-time cleanup never fails the run */ }
});
