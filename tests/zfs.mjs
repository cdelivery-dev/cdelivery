import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { promisify } from 'node:util';

const exec = promisify(execFile);
const base = process.env.CDELIVERY_TEST_CONTROLLER_URL;
const image = process.env.CDELIVERY_TEST_AGENT_IMAGE;
const appImage = process.env.CDELIVERY_TEST_APP_IMAGE || 'docker.io/library/busybox:1.37.0-musl';
const remote = JSON.parse(process.env.CDELIVERY_TEST_REMOTE || '[]');
assert(base && image, 'Set CDELIVERY_TEST_CONTROLLER_URL and CDELIVERY_TEST_AGENT_IMAGE');
assert([undefined, 'preview'].includes(process.env.CDELIVERY_TEST_CASE));
const quote = value => `'${value.replaceAll("'", "'\\''")}'`;

async function docker(...args) {
  const [program, ...prefix] = remote;
  const { stdout } = await exec(program || 'docker', program
    ? [...prefix, ['sudo', 'docker', ...args].map(quote).join(' ')]
    : args, { timeout: 180000, maxBuffer: 4 * 1024 * 1024 });
  return stdout.trim();
}

async function until(label, check) {
  const deadline = Date.now() + 240000;
  let failure;
  while (Date.now() < deadline) {
    try {
      const result = await check();
      console.log(`PASS ${label}`);
      return result;
    } catch (error) { failure = error; }
    await new Promise(resolve => setTimeout(resolve, 3000));
  }
  throw new Error(`${label}: ${failure?.message}`, { cause: failure });
}

const suffix = randomUUID().slice(0, 8);
const app = `za-${suffix}`, volume = `zv-${suffix}`, local = `zl-${suffix}`, group = `zg-${suffix}`;
const keyName = `zfs-${suffix}`;
const controller = await docker('ps', '-q', '--filter', 'label=com.docker.swarm.service.name=cdelivery');
const token = await docker('exec', controller, '/entrypoint', 'create-api-key',
  process.env.CDELIVERY_TEST_ACCOUNT_EMAIL || 'zfs@example.test', keyName, 'Admin');

async function api(method, path, body, expected) {
  const response = await fetch(`${base}/api${path}`, {
    method,
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(210000),
  });
  const text = await response.text();
  if (expected !== undefined) assert.equal(response.status, expected, `${method} ${path}: ${text}`);
  else assert(response.ok, `${method} ${path}: ${response.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

const keyId = (await api('GET', '/apikeys')).keys.find(key => key.name === keyName).id;
const settings = await api('GET', '/volume');
const version = (await api('GET', '/versions')).components.cdelivery_agent.version;
const node = (await api('GET', '/nodes')).nodes[0].id;
const volumes = `/nodes/${node}/volumes`, appPath = `/apps/${app}`;
const business = `cdelivery-${volume}`, dataset = `cdelivery/volumes/${volume}`;
const held = `${dataset}-held`;
const data = command => docker('run', '--rm', '--volume', `${business}:/data`,
  'busybox:1.37.0-musl', 'sh', '-c', command);
const host = (...args) => docker('run', '--rm', '--privileged', '--pid=host',
  '--entrypoint', '/usr/bin/nsenter', image, '--target', '1', '--mount', '--root', '--wd', '--', ...args);
const inspect = async name => JSON.parse(await docker('service', 'inspect', name))[0];

function checkMount(spec, source = business, device = dataset) {
  const mount = spec.Spec.TaskTemplate.ContainerSpec.Mounts.find(mount => mount.Target === '/data');
  assert.equal(mount.Source, source);
  assert.deepEqual(mount.VolumeOptions.DriverConfig, { Name: 'local', Options: { type: 'zfs', device } });
  return mount;
}

async function running() {
  const id = await docker('ps', '-q', '--filter', `label=com.docker.swarm.service.name=${app}`);
  assert(id && !id.includes('\n'), 'expected one running app container');
  assert.equal(await docker('exec', id, 'cat', '/data/value'), 'original');
  return id;
}

async function stop() {
  await docker('service', 'scale', '--detach', `${app}=0`);
  await until('app releases volume references', async () => {
    const ids = await docker('ps', '-aq', '--filter', `label=com.docker.swarm.service.name=${app}`);
    if (ids) await docker('rm', ...ids.split('\n'));
  });
}

async function mode(enabled) {
  await api('DELETE', '/volume');
  assert.equal((await api('GET', '/volume')).zfs_enabled, null);
  await until('previous Agent stopped', async () => {
    assert.equal(await docker('ps', '-q', '--filter', 'label=com.docker.swarm.service.name=cdelivery-agent'), '');
  });
  await api('PUT', '/volume', { version, zfs_enabled: enabled });
  await until(`Agent mode ${enabled}`, async () => {
    assert.equal((await api('GET', volumes)).zfs_enabled, enabled);
  });
}

let renamed = false;
console.log(`Resources: ${app} ${volume} ${local} ${group}`);
try {
  await mode(false);
  await api('POST', volumes, { name: local });
  await mode(true);
  assert((await api('GET', volumes)).volumes.some(item => item.name === local && item.ready && item.backend === 'Local'));
  await api('POST', volumes, { name: volume });
  await data('printf original > /data/value');
  await api('POST', '/groups', { name: group });
  const serve = 'mkdir -p /www; printf healthy > /www/index.html; exec httpd -f -p 8080 -h /www';
  await api('POST', '/apps', {
    name: app, group, image: appImage, replicas: { Fixed: 1 },
    mounts: { node_id: node, mounts: [{ volume, target: '/data' }] },
    container_options: { command: ['sh', '-c', serve] },
  });
  await until('app reads original ZFS data', running);
  checkMount(await inspect(app));
  await api('PATCH', appPath, { description: 'ZFS mount round trip' });
  checkMount(await inspect(app));

  if (process.env.CDELIVERY_TEST_CASE !== 'preview') {
    await stop();
    await docker('volume', 'rm', business);
    await docker('service', 'scale', '--detach', `${app}=1`);
    await until('missing Docker mapping recreates ZFS mount with original data', running);
    assert.equal(JSON.parse(await docker('volume', 'inspect', business))[0].Options.device, dataset);

    await stop();
    await docker('volume', 'rm', business);
    await host('/usr/sbin/zfs', 'rename', dataset, held);
    renamed = true;
    await docker('service', 'scale', '--detach', `${app}=1`);
    await until('missing dataset rejects mounting instead of creating ordinary storage', async () => {
      const ids = (await docker('service', 'ps', '-q', app)).split('\n');
      const tasks = JSON.parse(await docker('inspect', ...ids));
      assert(tasks.some(task => ['failed', 'rejected'].includes(task.Status.State) && task.Status.Err.includes('mount')));
      assert.equal(await docker('ps', '-q', '--filter', `label=com.docker.swarm.service.name=${app}`), '');
      assert.equal(JSON.parse(await docker('volume', 'inspect', business))[0].Options.type, 'zfs');
    });
    await stop();
    await host('/usr/sbin/zfs', 'rename', held, dataset);
    renamed = false;
    await docker('service', 'scale', '--detach', `${app}=1`);
    await until('restored dataset serves original data', running);

    await api('PATCH', appPath, { mounts: [{ volume, target: '/data', read_only: true }] });
    await until('mount update keeps ZFS parameters', async () => {
      const spec = await inspect(app);
      assert.equal(checkMount(spec).ReadOnly, true);
      assert.equal(spec.UpdateStatus.State, 'completed');
      await running();
    });
    await api('POST', `${appPath}/rollback`);
    await until('rollback restores original mount with ZFS parameters', async () => {
      const spec = await inspect(app);
      assert(!checkMount(spec).ReadOnly);
      assert.equal(spec.UpdateStatus.State, 'rollback_completed');
      await running();
    });
    const before = await running();
    await api('POST', `${appPath}/restart`);
    await until('restart retains original ZFS data', async () => assert.notEqual(await running(), before));
  }

  await api('PUT', `${appPath}/xhealth`, {
    method: { Custom: { command: ['sh', '-c', 'test "$(wget -qO- "http://$TARGET_HOST:$TARGET_PORT")" = healthy'] } },
    interval: { unit: 'Minutes', value: 1 }, target: { InternalPort: { port: 8080 } }, release_check: true,
  });
  const changed = `printf candidate > /data/candidate; ${serve}`;
  const submitted = await api('PATCH', appPath, { container_options: { command: ['sh', '-c', changed] } }, 202);
  await until('Change promotes onto the original dataset', async () => {
    assert.equal((await api('GET', `/changes/${submitted.change}`)).state, 'Completed');
    assert.equal((await api('GET', appPath)).container_options.command[2], changed);
    checkMount(await inspect(app));
    await running();
    assert.equal(await docker('service', 'ls', '--filter', `label=dev.cdelivery.change_id=${submitted.change}`, '--format', '{{.Name}}'), '');
  });
  let previous;
  for (const value of ['isolated', 'fresh']) {
    const failed = await api('PATCH', appPath, { container_options: { command: ['sh', '-c',
      `test "$(cat /data/value)" = original && printf ${value} > /data/value && mkdir -p /www && printf broken > /www/index.html && exec httpd -f -p 8080 -h /www`] } }, 202);
    assert.notEqual(failed.change, previous, 'new submission creates a different Change');
    previous = failed.change;
    await until('failed Change retains isolated candidate data', async () => {
      const change = await api('GET', `/changes/${failed.change}`);
      assert.equal(change.state, 'Open');
      assert.equal(change.busy, false);
      assert.equal(change.error, null);
      assert.equal(typeof change.apps.find(item => item.name === app).result.Failed.error, 'string');
    });
    const service = await docker('service', 'ls', '--filter', `label=dev.cdelivery.change_id=${failed.change}`,
      '--filter', 'label=dev.cdelivery.app_service_kind=GuardianApp', '--format', '{{.Name}}');
    const spec = await inspect(service);
    const preview = spec.Spec.Labels['dev.cdelivery.preview'];
    assert.match(preview, /^[0-9a-f]{32}$/);
    const clone = `cdelivery-guardian.${preview}.${volume}`;
    checkMount(spec, clone, `cdelivery/guardian/${preview}/${volume}`);
    assert((await api('GET', volumes)).previews.includes(preview));
    assert.equal(await data('cat /data/value'), 'original');
    assert.equal(await docker('run', '--rm', '--volume', `${clone}:/data`, 'busybox:1.37.0-musl', 'cat', '/data/value'), value);
    await api('DELETE', `/nodes/${node}/preview-volumes/${preview}`, undefined, 409);
    await api('POST', `/changes/${failed.change}/close`);
    await until('manual close removes preview data and releases the App', async () => {
      assert.equal((await api('GET', `/changes/${failed.change}`)).state, 'Closed');
      assert(!(await api('GET', volumes)).previews.includes(preview));
    });
    await api('DELETE', `/nodes/${node}/preview-volumes/${preview}`);
  }
  const orphan = randomUUID().replaceAll('-', '');
  await host('/usr/sbin/zfs', 'snapshot', `${dataset}@cdelivery-guardian.${orphan}`);
  assert((await api('GET', volumes)).previews.includes(orphan));
  await api('DELETE', `/nodes/${node}/preview-volumes/${orphan}`);
  assert(!(await api('GET', volumes)).previews.includes(orphan));

  await mode(false);
  const report = await api('GET', volumes);
  assert(report.volumes.some(item => item.name === volume && item.ready && item.backend === 'Zfs'));
  assert(report.volumes.some(item => item.name === local && item.ready && item.backend === 'Local'));
  await api('POST', `${appPath}/restart`);
  await until('existing ZFS app works in ordinary mode', running);
  await stop();
  await api('DELETE', appPath);
  await until('ordinary mode deletes existing ZFS volume', () => api('DELETE', `${volumes}/${volume}`));
  await assert.rejects(host('/usr/sbin/zfs', 'list', dataset));
  await api('DELETE', `${volumes}/${local}`);
  await api('DELETE', `/groups/${group}`);
  console.log('PASS controller ZFS acceptance');
} finally {
  if (renamed) {
    await stop();
    await host('/usr/sbin/zfs', 'rename', held, dataset);
  }
  if (settings.image) await mode(settings.zfs_enabled);
  else await api('DELETE', '/volume');
  await api('DELETE', `/apikeys/${keyId}`);
}
