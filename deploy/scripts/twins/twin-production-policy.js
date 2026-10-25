// Reference implementation (node, no dependencies) of deploy/scripts/check-production-policy.sh over the RENDERED production compose file, plus
// mutation tests that prove the policy catches what it claims to. Needs a docker client (no daemon) unless --rendered FILE is given.
//   node deploy/scripts/twins/twin-production-policy.js [--rendered rendered.json]
// Exit 1 on a violation of the real file or on a mutation that is NOT caught.
const fs = require('fs');
const path = require('path');
const root = path.resolve(__dirname, '..', '..', '..');
function load() {
  const i = process.argv.indexOf('--rendered');
  if (i > 0) return JSON.parse(fs.readFileSync(process.argv[i + 1], 'utf8'));
  const out = require('child_process').execFileSync('docker', ['compose', '--project-directory', path.join(root, 'deploy/compose'), '--file', path.join(root, 'deploy/compose/production.compose.yml'), '--env-file', path.join(root, 'deploy/images/images.lock'), '--env-file', path.join(root, 'deploy/compose/production.env'), '--env-file', path.join(root, 'deploy/compose/host.env.example'), 'config', '--format', 'json'], { encoding: 'utf8', maxBuffer: 64 << 20 });
  return JSON.parse(out);
}
const base = load();

function check(c) {
  const out = [];
  const fail = m => out.push(m);
  const svc = c.services, nets = c.networks;
  for (const [n, v] of Object.entries(nets)) {
    if (v.internal !== true && !['public', 'control-out', 'db-out', 'egress-out', 'monitor-out'].includes(n)) fail('non-internal network ' + n);
    const m = /^slot([0-9]+)$/.exec(n);
    if (m && (v.internal !== true || v.name !== 'df-slot-' + m[1])) fail('slot network ' + n);
  }
  const members = n => Object.keys(svc).filter(s => svc[s].networks && Object.keys(svc[s].networks).includes(n)).sort().join(',');
  const want = {
    public: 'edge', 'control-out': 'control-plane', 'db-out': 'db-gate', 'egress-out': 'egress',
    ingress: 'control-plane,edge', data: 'control-plane,db-gate,ingestor', dispatch: 'control-plane,db-gate,egress,ingestor',
    monitoring: 'ingestor,prometheus', observe: 'alertmanager,host-textfile,prometheus', 'monitor-out': 'alertmanager', 'runner-net': 'db-gate,runner', slot1: 'control-plane,db-gate,egress,ingestor', slot2: 'control-plane,db-gate,egress,ingestor'
  };
  for (const [n, w] of Object.entries(want)) if (members(n) !== w) fail('network ' + n + ' has {' + members(n) + '} expected {' + w + '}');
  const bridge = {"public":"dfpub0","ingress":"dfin0","data":"dfdata0","dispatch":"dfdsp0","monitoring":"dfmn0","observe":"dfobs0","runner-net":"dfrun0","control-out":"dfctl0","db-out":"dfdb0","egress-out":"dfegr0","monitor-out":"dfmon0"};
  for (const [n, v] of Object.entries(nets)) {
    const want = bridge[n] || (/^slot[0-9]+$/.test(n) ? 'df' + n : null);
    if (!want || (v.driver_opts || {})['com.docker.network.bridge.name'] !== want) fail('network ' + n + ' bridge name');
  }
  for (const [name, s] of Object.entries(svc)) for (const v of s.volumes || []) if (/allowed-images/.test(v.target) && (v.target !== '/etc/dungeonflux/allowed-images.d' || v.source !== '/etc/dungeonflux/allowed-images.d' || v.read_only !== true)) fail(name + ' allow-list mount ' + v.target);
  const dp = svc['docker-proxy'];
  if (dp.network_mode !== 'none' || (dp.networks && Object.keys(dp.networks).length)) fail('docker-proxy has a network');

  const map = { 'control-plane': 'control-plane', egress: 'dispatcher', ingestor: 'ingestor', edge: 'edge', runner: 'runner', alertmanager: 'monitoring' };
  const caps = { edge: ['NET_BIND_SERVICE'] };
  const rw = { edge: ['/data'], ingestor: ['/var/lib/dungeonflux/telemetry'], 'docker-proxy': ['/run/proxy', '/var/run/docker.sock'], runner: ['/run/dungeonflux/runtime'], prometheus: ['/prometheus'], alertmanager: ['/alertmanager'] };
  const uids = {};
  const sockets = [];
  for (const [name, s] of Object.entries(svc)) {
    const sec = (s.volumes || []).filter(v => v.target === '/run/secrets');
    const got = sec.map(v => v.source.split('/').pop());
    const w = map[name];
    if (w === undefined ? sec.length > 0 : (JSON.stringify(got) !== JSON.stringify([w]) || !sec.every(v => v.read_only === true))) fail(name + ' secrets mounts ' + JSON.stringify(got));
    for (const v of sec) if (!v.source.startsWith('/run/dungeonflux/secrets/')) fail(name + ' secrets source ' + v.source);
    if (s.read_only !== true || !(s.cap_drop || []).includes('ALL')) fail(name + ' read_only/cap_drop');
    if (!(s.security_opt || []).includes('no-new-privileges:true')) fail(name + ' no-new-privileges');
    if (!s.mem_limit || !s.pids_limit) fail(name + ' limits');
    if (!s.image || /:latest($|@)/.test(s.image)) fail(name + ' image');
    if (s.privileged === true) fail(name + ' privileged');
    for (const k of ['network_mode', 'pid', 'ipc', 'userns_mode', 'cgroup']) if (s[k] === 'host' || (typeof s[k] === 'string' && s[k].startsWith('container:'))) fail(name + ' ' + k);
    if (s.network_mode != null && (name !== 'docker-proxy' || s.network_mode !== 'none')) fail(name + ' network_mode ' + s.network_mode);
    if ((s.devices || []).length || Object.keys(s.sysctls || {}).length || (s.security_opt || []).some(o => /unconfined/.test(o))) fail(name + ' devices/sysctls/unconfined');
    for (const c of s.cap_add || []) if (!(caps[name] || []).includes(c)) fail(name + ' cap_add ' + c);
    for (const p of s.ports || []) if (name !== 'edge' || !['80', '443'].includes(String(p.target)) || p.published !== String(p.target)) fail(name + ' port ' + p.published);
    for (const v of s.volumes || []) {
      if (/docker\.sock/.test(v.source)) sockets.push(name);
      if (v.type === 'bind' && !/^(\/etc\/dungeonflux\/|\/opt\/dungeonflux\/|\/run\/dungeonflux\/|\/var\/lib\/dungeonflux\/|\/var\/run\/docker\.sock$)/.test(v.source)) fail(name + ' bind ' + v.source);
      if (v.read_only !== true && !(rw[name] || []).includes(v.target)) fail(name + ' writable ' + v.target);
    }
    const m = /^([0-9]+):([0-9]+)$/.exec(s.user || '');
    if (!m) { fail(name + ' no uid:gid'); continue; }
    const uid = Number(m[1]);
    if (uid === 65532) fail(name + ' uid 65532');
    if (uid === 0) fail(name + ' uid 0');
    if (uid >= 20000) fail(name + ' uid in slot range');
    { if (uids[uid]) fail(name + ' shares uid ' + uid + ' with ' + uids[uid]); uids[uid] = name; }
    const ga = s.group_add || [];
    if (ga.length) {
      const ok = name === 'docker-proxy' && ga.length === 1 && /^[0-9]+$/.test(ga[0]);
      if (!ok) fail(name + ' group_add ' + ga);
    }
  }
  if (JSON.stringify(sockets) !== '["docker-proxy"]') fail('docker socket mounted by ' + JSON.stringify(sockets));
  const t = (dp.volumes || []).map(v => v.target).sort();
  if (JSON.stringify(t) !== '["/etc/dungeonflux/allowed-images.d","/run/dungeonflux/runtime","/run/proxy","/var/run/docker.sock"]') fail('docker-proxy mounts ' + t);
  const rt = (dp.volumes || []).find(v => v.target === '/run/dungeonflux/runtime');
  if (!rt || rt.read_only !== true || rt.source !== '/run/dungeonflux/runtime') fail('docker-proxy runtime tree not read-only at the bind path');
  return out;
}

const clone = () => JSON.parse(JSON.stringify(base));
const real = check(base);
console.log(real.length ? 'REAL FILE VIOLATIONS:\n  ' + real.join('\n  ') : 'real file: policy ok');

const mutations = {
  'second docker socket mount': c => { c.services.runner.volumes.push({ type: 'bind', source: '/var/run/docker.sock', target: '/var/run/docker.sock', bind: {} }); },
  'privileged runner': c => { c.services.runner.privileged = true; },
  'writable secrets mount': c => { delete c.services['control-plane'].volumes.find(v => v.target === '/run/secrets').read_only; },
  'published database port': c => { c.services['db-gate'].ports = [{ target: 5432, published: '5432' }]; },
  'non-internal slot network': c => { delete c.networks.slot1.internal; },
  'duplicated uid': c => { c.services.egress.user = '10001:10003'; },
  'extra capability': c => { c.services.runner.cap_add = ['SYS_ADMIN']; },
  'proxy on a network': c => { c.services['docker-proxy'].networks = { data: {} }; delete c.services['docker-proxy'].network_mode; },
  'host network': c => { c.services.ingestor.network_mode = 'host'; },
  'foreign secrets directory': c => { c.services.egress.volumes.find(v => v.target === '/run/secrets').source = '/run/dungeonflux/secrets/control-plane'; },
  'edge on the data network': c => { c.services.edge.networks.data = {}; },
  'runner as 65532': c => { c.services.runner.user = '65532:10004'; },
  'bind outside the host paths': c => { c.services.edge.volumes.push({ type: 'bind', source: '/home/ci', target: '/x', read_only: true, bind: {} }); },
  'writable config mount': c => { delete c.services.egress.volumes.find(v => v.target === '/etc/dungeonflux/egress.toml').read_only; },
  'runtime-range uid': c => { c.services.ingestor.user = '20001:10008'; },
  'missing pid limit': c => { delete c.services.edge.pids_limit; },
  'extra group': c => { c.services.egress.group_add = ['999']; },
  'runner as root': c => { c.services.runner.user = '0:10004'; },
  'runner with CHOWN': c => { c.services.runner.cap_add = ['CHOWN']; },
  'proxy sees the runtime tree writable': c => { delete c.services['docker-proxy'].volumes.find(v => v.target === '/run/dungeonflux/runtime').read_only; },
  'proxy without the runtime tree': c => { c.services['docker-proxy'].volumes = c.services['docker-proxy'].volumes.filter(v => v.target !== '/run/dungeonflux/runtime'); },
  'runner extra group': c => { c.services.runner.group_add = ['10013']; },
  'unnamed bridge on a slot network': c => { delete c.networks.slot1.driver_opts; },
  'alertmanager on the ingestor network': c => { c.services.alertmanager.networks.monitoring = {}; },
  'unnamed bridge on db-out': c => { delete c.networks['db-out'].driver_opts; },
  'allow-list as a single file': c => { const v = c.services.runner.volumes.find(x => /allowed-images/.test(x.target)); v.source = '/etc/dungeonflux/allowed-images'; v.target = '/etc/dungeonflux/allowed-images'; },
  'public egress network member': c => { c.services['control-plane'].networks['egress-out'] = {}; }
};
let missed = 0;
for (const [name, mutate] of Object.entries(mutations)) {
  const c = clone(); mutate(c);
  const r = check(c);
  if (r.length === 0) { console.log('MISSED mutation: ' + name); missed++; } else console.log('caught: ' + name + ' -> ' + r[0]);
}
process.exit(real.length || missed ? 1 : 0);
