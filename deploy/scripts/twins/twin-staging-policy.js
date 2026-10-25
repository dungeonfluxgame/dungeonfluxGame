// Reference implementation (node, no dependencies) of deploy/scripts/check-compose-policy.sh over the RENDERED staging compose file.
// Needs a docker client (no daemon) unless --rendered FILE is given. The check of the generated secrets directories runs only if
// artifacts/tmp/staging/secrets exists (deploy/scripts/staging-secrets.sh).
//   node deploy/scripts/twins/twin-staging-policy.js [--rendered rendered.json]
const fs = require('fs');
const path = require('path');
const root = path.resolve(__dirname, '..', '..', '..');
function load() {
  const i = process.argv.indexOf('--rendered');
  if (i > 0) return JSON.parse(fs.readFileSync(process.argv[i + 1], 'utf8'));
  const out = require('child_process').execFileSync('docker', ['compose', '--project-directory', path.join(root, 'deploy/compose'), '--file', path.join(root, 'deploy/compose/staging.compose.yml'), '--env-file', path.join(root, 'deploy/images/images.lock'), '--env-file', path.join(root, 'deploy/compose/staging.env'), '--profile', 'runtime', 'config', '--format', 'json'], { encoding: 'utf8', maxBuffer: 64 << 20 });
  return JSON.parse(out);
}
const c = load();
const svc = c.services, nets = c.networks; let bad = 0;
const fail = m => { console.log('FAIL', m); bad++; };
const members = n => Object.keys(svc).filter(s => svc[s].networks && Object.keys(svc[s].networks).includes(n)).sort();
for (const n of Object.keys(nets)) if (n !== 'public' && !nets[n].internal) fail('network not internal: ' + n);
const want = {
  public: ['edge'],
  ingress: ['control-plane', 'edge'],
  data: ['bucket-init', 'control-plane', 'db-gate', 'ingestor', 'migrate', 'object-store', 'postgres'],
  dispatch: ['control-plane', 'db-gate', 'egress', 'ingestor'],
  monitoring: ['ingestor'],
  slot1: ['control-plane', 'db-gate', 'egress', 'ingestor', 'runtime-1'],
  slot2: ['control-plane', 'db-gate', 'egress', 'ingestor', 'runtime-2'],
  'egress-out': ['egress']
};
for (const [n, w] of Object.entries(want)) { const m = members(n); if (JSON.stringify(m) !== JSON.stringify(w.slice().sort())) fail(n + ' members ' + m + ' != ' + w); }
const norm = p => p.split('\\').join('/');
for (const r of ['runtime-1', 'runtime-2']) {
  const k = Object.keys(svc[r].networks);
  if (k.length !== 1 || !/^slot[0-9]$/.test(k[0])) fail(r + ' networks ' + k);
  for (const v of svc[r].volumes || []) if (v.target === '/run/secrets' && !norm(v.source).endsWith('/' + r)) fail(r + ' mounts foreign secrets ' + v.source);
}
const caps = { postgres: ['CHOWN', 'DAC_OVERRIDE', 'FOWNER', 'SETGID', 'SETUID'], edge: ['NET_BIND_SERVICE'] };
const uids = {};
for (const [name, s] of Object.entries(svc)) {
  if (name !== 'bucket-init' && !s.read_only) fail(name + ' not read_only');
  if (!(s.cap_drop || []).includes('ALL')) fail(name + ' cap_drop');
  if (s.privileged) fail(name + ' privileged');
  for (const k of ['network_mode', 'pid', 'ipc', 'userns_mode', 'cgroup']) if (s[k] === 'host' || (typeof s[k] === 'string' && s[k].startsWith('container:'))) fail(name + ' ' + k + '=' + s[k]);
  if (s.devices && s.devices.length) fail(name + ' devices');
  if (s.sysctls && Object.keys(s.sysctls).length) fail(name + ' sysctls');
  for (const o of s.security_opt || []) if (/unconfined/.test(o)) fail(name + ' security_opt ' + o);
  if (!(s.security_opt || []).includes('no-new-privileges:true')) fail(name + ' no-new-privileges');
  const allowed = caps[name] || [];
  for (const cap of s.cap_add || []) if (!allowed.includes(cap)) fail(name + ' cap_add ' + cap);
  if (s.ports && s.ports.length) { if (name !== 'edge') fail(name + ' publishes ports'); else for (const p of s.ports) if (p.host_ip !== '127.0.0.1') fail('edge publishes on ' + p.host_ip); }
  for (const v of s.volumes || []) {
    if (/docker\.sock/.test(v.source)) fail(name + ' docker socket');
    if (v.type === 'bind' && !norm(v.source).startsWith(norm(root) + '/')) fail(name + ' bind outside the repository ' + v.source);
  }
  const map = {'control-plane':'control-plane',egress:'dispatcher',ingestor:'ingestor',edge:'edge',migrate:'migrate',postgres:'postgres','object-store':'object-store','bucket-init':'bucket-init','runtime-1':'runtime-1','runtime-2':'runtime-2'};
  const got = (s.volumes || []).filter(v => v.target === '/run/secrets').map(v => norm(v.source).split('/').pop());
  const want = map[name];
  if ((want === undefined && got.length) || (want !== undefined && JSON.stringify(got) !== JSON.stringify([want]))) fail(name + ' mounts secrets ' + JSON.stringify(got) + ' expected ' + want);
  if (name === 'postgres' || name === 'object-store') continue;
  const m = /^([0-9]+):([0-9]+)$/.exec(s.user || '');
  if (!m) { fail(name + ' has no numeric uid:gid'); continue; }
  const uid = Number(m[1]);
  if (uid === 0 || uid === 65532) fail(name + ' uid ' + uid);
  if (uids[uid]) fail(name + ' shares uid ' + uid + ' with ' + uids[uid]); uids[uid] = name;
  if (name.startsWith('runtime-') ? uid < 20000 : uid >= 20000) fail(name + ' uid ' + uid + ' outside its range');
}
const secretsRoot = path.join(root, 'artifacts/tmp/staging/secrets');
const rt = fs.existsSync(path.join(secretsRoot, 'runtime-1')) ? fs.readdirSync(path.join(secretsRoot, 'runtime-1')) : [];
if (!rt.length) console.log('SKIP secrets directory content (run deploy/scripts/staging-secrets.sh first)');
for (const f of rt) if (/(session|storage|provider|payments|workos|email|journal|media_gateway|dispatch_tls|dispatcher_client|db_control|db_egress|db_admin|db_migrator|db_runner|db_billing|db_support)/.test(f)) fail('runtime has ' + f);
console.log('uids:', JSON.stringify(uids));
console.log(bad ? ('FAILURES ' + bad) : 'topology ok');
