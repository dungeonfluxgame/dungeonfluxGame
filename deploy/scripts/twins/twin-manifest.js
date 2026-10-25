// Reference implementation (node, no dependencies) of the invariants deploy/scripts/secrets-audit.sh manifest checks with jq.
//   node deploy/scripts/twins/twin-manifest.js
const fs = require('fs');
const path = require('path');
const root = path.resolve(__dirname, '..', '..', '..');
const m = JSON.parse(fs.readFileSync(path.join(root, 'deploy/secrets/secrets.manifest.json'), 'utf8'));
let bad = 0; const f = x => { console.log('FAIL', x); bad++; };
const seen = new Set();
for (const s of m.secrets) {
  if (seen.has(s.name)) f('duplicate ' + s.name); seen.add(s.name);
  if (!s.store_of_record || !s.rotation || !s.rotation.procedure || !s.on_compromise) f('incomplete ' + s.name);
  if (!(s.rotation.procedure in m.procedures)) f('undefined procedure ' + s.name);
  if (s.delivery === 'host') {
    for (const c of s.consumers) {
      if (!(c in m.consumers)) { f('unknown consumer ' + c); continue; }
      if (!s.hosts.some(h => m.consumers[c].hosts.includes(h))) f(s.name + ' delivered to ' + c + ' on no common host');
    }
  } else if (s.delivery !== 'runner-issued' && s.consumers.length > 0) f('non-host entry with consumers ' + s.name);
  if (s.name.startsWith('provider_') && JSON.stringify(s.consumers) !== '["dispatcher"]') f('provider key outside dispatcher ' + s.name);
  if (s.consumers.includes('runtime') && !(s.delivery === 'runner-issued' || s.class === 'config' || s.name === 'db_campaign_runtime_password')) f('runtime holds ' + s.name);
  if (s.consumers.includes('dispatcher') && ['storage', 'journal', 'backup', 'payments'].includes(s.class)) f('dispatcher holds ' + s.name);
  if (['backup_age_identity', 'db_admin_password', 'db_support_password'].includes(s.name) && s.delivery !== 'operator-only') f('must be operator-only ' + s.name);
  if (s.class === 'payments' && !s.staging_policy) f('payments without staging policy ' + s.name);
}
// One consumer, one group, everywhere: the manifest, the installer's group map and the cloud-init groupadd list must agree.
const installer = fs.readFileSync(path.join(root, 'infra/cloud-init/df-secrets-install.sh'), 'utf8');
const tpl = fs.readFileSync(path.join(root, 'infra/cloud-init/droplet.yaml.tftpl'), 'utf8');
for (const [name, c] of Object.entries(m.consumers)) {
  if (!installer.includes('[' + name + ']=' + c.gid)) f('consumer ' + name + ' gid ' + c.gid + ' missing from df-secrets-install.sh');
  if (!tpl.includes('groupadd, -g, "' + c.gid + '", dfs-' + name + ']')) f('consumer ' + name + ' gid ' + c.gid + ' missing from the cloud-init groupadd list');
}
const rs = m.secrets.filter(x => x.name === 'runtime_server_issuer_key');
if (rs.length !== 1 || JSON.stringify(rs[0].consumers) !== '["control-plane"]' || rs[0].delivery !== 'host') f('runtime_server_issuer_key must be delivered to the control plane only');
for (const x of m.secrets) if (x.name !== 'runtime_server_issuer_key' && /runtime_server_issuer_key/.test(JSON.stringify(x.consumers))) f('stray reference');
for (const id of ['refresh-keys', 'release-verification']) if (!(m.launch_gates || []).some(g => g.id === id)) f('launch gate ' + id + ' missing');
if (!(m.recurring_operations || []).some(o => o.id === 'replace-bastion' && o.cadence_days <= 30 && o.dependency)) f('recurring replace-bastion missing');
console.log(m.secrets.length + ' entries; ' + (bad ? bad + ' violation(s)' : 'all manifest invariants hold'));
process.exit(bad ? 1 : 0);
