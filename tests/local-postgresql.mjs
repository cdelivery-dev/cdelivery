import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import { promisify } from 'node:util';

const exec = promisify(execFile);
const base = process.env.CDELIVERY_TEST_CONTROLLER_URL;
const databaseImage = process.env.CDELIVERY_TEST_POSTGRES_IMAGE;
const consumerImage = process.env.CDELIVERY_TEST_CONSUMER_IMAGE;
const remote = JSON.parse(process.env.CDELIVERY_TEST_REMOTE || '[]');
const output = process.env.CDELIVERY_TEST_RESULTS_DIR;
const keep = process.env.CDELIVERY_TEST_KEEP === '1';
assert(base && databaseImage && consumerImage && output, 'Set controller URL, both images and results directory');
const prefix = process.env.CDELIVERY_TEST_PREFIX || `pg-${randomUUID().slice(0, 4)}`;
const group = prefix, database = `${prefix}-postgres`, consumer = `${prefix}-t1`;
const dependency = `${prefix}-database`, volume = `${prefix}-data`;
const password = randomUUID();
const quote = value => `'${value.replaceAll("'", "'\\''")}'`;
const report = { base, group, database, consumer, dependency, volume, checks: [], changes: [] };

async function docker(...args) {
  const [program, ...options] = remote;
  const result = await exec(program || 'docker', program
    ? [...options, ['sudo', 'docker', ...args].map(quote).join(' ')] : args,
  { timeout: 240000, maxBuffer: 4 * 1024 * 1024 });
  return result.stdout.trim();
}

async function until(label, check, timeout = 600000) {
  const deadline = Date.now() + timeout;
  let failure;
  while (Date.now() < deadline) {
    try { return await check(); } catch (error) { failure = error; }
    await new Promise(resolve => setTimeout(resolve, 2000));
  }
  throw new Error(`${label}: ${failure?.message}`, { cause: failure });
}

async function pass(label) {
  report.checks.push(label);
  await writeFile(`${output}/local-postgresql.json`, JSON.stringify(report, null, 2));
  console.log(`PASS ${label}`);
}

const main = await docker('ps', '-q', '--filter', 'label=com.docker.swarm.service.name=cdelivery');
const token = await docker('exec', main, '/entrypoint', 'create-api-key',
  process.env.CDELIVERY_TEST_ACCOUNT_EMAIL || 'zfs@example.test', `${prefix}-acceptance`, 'Admin');

async function api(method, path, body, expected) {
  const response = await fetch(`${base}/api${path}`, {
    method, headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(240000),
  });
  const text = await response.text();
  if (expected !== undefined) assert.equal(response.status, expected, `${method} ${path}: ${text}`);
  else assert(response.ok, `${method} ${path}: ${response.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

async function container(name) {
  const id = await docker('ps', '-q', '--filter', `label=com.docker.swarm.service.name=${name}`);
  assert(id && !id.includes('\n'), `Expected one running container for ${name}`);
  return id;
}

async function sql(name, statement) {
  return docker('exec', await container(name), 'psql', '-U', 'acceptance', '-d', 'acceptance', '-XAt', '-v', 'ON_ERROR_STOP=1', '-c', statement);
}

async function change(sql) {
  const created = await api('POST', '/changes', { dependency, sql }, 201);
  report.changes.push(created.id);
  return created.id;
}

async function idle(id, state) {
  return until(`change ${id} becomes ${state}`, async () => {
    const current = await api('GET', `/changes/${id}`);
    assert.equal(current.busy, false);
    assert.equal(current.state, state, current.error || current.stage);
    return current;
  });
}

async function resources(id) {
  const names = await docker('service', 'ls', '--filter', `label=dev.cdelivery.change_id=${id}`, '--format', '{{.Name}}');
  return names ? JSON.parse(await docker('service', 'inspect', ...names.split('\n'))) : [];
}

async function cleaned(id, clones) {
  assert.deepEqual(await resources(id), []);
  assert.equal(await docker('secret', 'ls', '--filter', `label=dev.cdelivery.change_id=${id}`, '--format', '{{.Name}}'), '');
  const previews = (await api('GET', `/nodes/${node}/volumes`)).previews;
  assert(!previews.some(name => clones.includes(name)), 'Preview volumes must be removed');
}

async function close(id) {
  const clones = (await resources(id)).map(item => item.Spec.Labels['dev.cdelivery.preview']).filter(Boolean);
  await api('POST', `/changes/${id}/close`, undefined, 200);
  await idle(id, 'Closed');
  await cleaned(id, clones);
}

await mkdir(output, { recursive: true });
const keyId = (await api('GET', '/apikeys')).keys.find(key => key.name === `${prefix}-acceptance`).id;
const node = (await api('GET', '/nodes')).nodes[0].id;
report.node = node;
try {
  const version = (await api('GET', '/versions')).components.cdelivery_agent.version;
  const settings = await api('GET', '/volume');
  if (!settings.image) await api('PUT', '/volume', { version, zfs_enabled: true });
  await until('ZFS agent ready', async () => assert.equal((await api('GET', `/nodes/${node}/volumes`)).zfs_enabled, true));
  await api('POST', '/groups', { name: group });
  await api('POST', `/nodes/${node}/volumes`, { name: volume });
  await api('POST', '/apps', {
    name: database, group, image: databaseImage, replicas: { Fixed: 1 },
    description: 'Local PostgreSQL on a real ZFS volume',
    mounts: { node_id: node, mounts: [{ volume, target: '/var/lib/postgresql' }] },
    container_options: { env: [
      { name: 'POSTGRES_USER', value: 'acceptance' },
      { name: 'POSTGRES_DB', value: 'acceptance' },
      { name: 'POSTGRES_PASSWORD', value: password },
    ] },
  });
  await until('PostgreSQL ready', async () => assert.equal(await sql(database, 'SELECT 1'), '1'));
  await sql(database, "CREATE TABLE fixture (id integer PRIMARY KEY, value text NOT NULL); INSERT INTO fixture VALUES (1, 'healthy'); CREATE TABLE markers (value text PRIMARY KEY)");
  await api('POST', '/dependencies', {
    name: dependency, provider: 'Local', app: database,
    description: 'ZFS rehearsal and consumer XHealth acceptance',
    dsn: `postgresql://acceptance:${password}@${database}:5432/acceptance?sslmode=disable&application_name=zfs-acceptance`,
  });
  const source = await api('GET', `/dependencies/${dependency}`);
  assert.equal(source.group, group);
  const otherGroup = `${prefix}-other`;
  await api('POST', '/groups', { name: otherGroup });
  await api('PATCH', `/apps/${database}`, { group: otherGroup }, 409);
  assert(!(await api('GET', `/dependencies?group=${otherGroup}`)).dependencies.some(item => item.id === source.id));
  const reference = { PostgreSQL: { name: source.name, id: source.id } };
  const env = Object.entries({ DATABASE_URL: 'dsn', DB_HOST: 'host', DB_PORT: 'port', DB_DATABASE: 'database', DB_USERNAME: 'username', DB_PASSWORD: 'password' })
    .map(([name, field]) => ({ name, value: { dependency: reference, field } }));
  await api('POST', '/apps', {
    name: consumer, group, image: consumerImage, replicas: { Fixed: 1 },
    description: 'Checks real SQL reads using the full DSN and split connection fields',
    container_options: { env },
  });
  await api('PATCH', `/apps/${consumer}`, { group: otherGroup }, 422);
  await api('DELETE', `/groups/${otherGroup}`);
  const healthConfig = {
    method: { Custom: { command: ['sh', '-c', 'test "$(/opt/busybox wget -qO- "http://$TARGET_HOST:$TARGET_PORT/cgi-bin/health")" = healthy'] } },
    interval: { unit: 'Minutes', value: 1 }, target: { InternalPort: { port: 8080 } }, release_check: true,
  };
  for (const binding of [reference, { App: database }]) {
    for (const name of ['TARGET_HOST', 'TARGET_PORT']) {
      await api('PATCH', `/apps/${consumer}`, { container_options: { env: [...env, { name, value: { dependency: binding, field: 'host' } }] } });
      await api('PUT', `/apps/${consumer}/xhealth`, healthConfig, 422);
    }
  }
  await api('PATCH', `/apps/${consumer}`, { container_options: { env } });
  await until('consumer queries original database', async () => {
    assert.equal(await docker('exec', await container(consumer), '/opt/busybox', 'wget', '-qO-', 'http://127.0.0.1:8080/cgi-bin/health'), 'healthy');
  });
  await api('PUT', `/apps/${consumer}/xhealth`, healthConfig);
  for (const binding of [reference, { App: database }]) {
    await api('PATCH', `/apps/${consumer}`, { container_options: { env: [...env, { name: 'TARGET_HOST', value: { dependency: binding, field: 'host' } }] } }, 422);
  }
  await pass('XHealth rejects reserved target variables from both dependency types before and after configuration');
  await pass('Local dependency and all six connection fields work');

  const success = await change("ALTER TABLE fixture ADD COLUMN note text; INSERT INTO markers VALUES ('success'); SELECT pg_sleep(15)");
  const clones = [];
  await until('successful SQL release', async () => {
    const current = await api('GET', `/changes/${success}`);
    const temporary = await resources(success);
    for (const item of temporary) {
      const clone = item.Spec.Labels['dev.cdelivery.preview'];
      if (clone && !clones.includes(clone)) clones.push(clone);
    }
    if (['Apply', 'Finish'].includes(current.stage)) {
      assert(!temporary.some(item => item.Spec.Name.startsWith('cdelivery-preview-') || item.Spec.Name.startsWith('cdelivery-check-')));
    }
    assert.equal(current.state, 'Completed', current.error || current.stage);
  });
  assert(clones.length > 0, 'Successful release must create a real preview');
  assert.equal(await sql(database, "SELECT value FROM markers WHERE value = 'success'"), 'success');
  await cleaned(success, clones);
  await pass('Compatible migration rehearses, cleans temporary resources, then applies');

  const pin = `${prefix}-pin`;
  await docker('service', 'create', '--detach', '--name', pin, '--secret', `cdelivery-change-${success.replaceAll('-', '')}`, databaseImage, 'sleep', '86400');
  await api('POST', '/changes', { dependency, sql: 'SELECT 1' }, 500);
  assert.equal((await api('GET', `/changes/${success}`)).state, 'Completed');
  assert.equal((await api('GET', '/changes')).changes.filter(item => item.target.PostgreSQL?.dependency.id === source.id).length, 1);
  await api('PATCH', `/apps/${consumer}`, { description: 'No reservation after rejected submission' });
  await docker('service', 'rm', pin);
  await until('record pin removed', async () => assert.equal(await docker('ps', '-q', '--filter', `label=com.docker.swarm.service.name=${pin}`), ''));
  await pass('Failed record replacement rejects submission without starting work or reserving apps');

  const failed = await change('ALTER TABLE fixture DROP COLUMN value');
  const rejected = await idle(failed, 'Open');
  await api('GET', `/changes/${success}`, undefined, 404);
  assert.equal((await docker('secret', 'ls', '--filter', `label=dev.cdelivery.change.target=${dependency}`, '--format', '{{.Name}}')).split('\n').length, 1);
  await api('POST', '/changes', { dependency, sql: 'SELECT 1' }, 409);
  assert.equal((await api('GET', `/changes/${failed}`)).state, 'Open');
  assert.equal(rejected.stage, 'Check');
  assert.equal(rejected.error, null);
  assert(rejected.apps.find(app => app.name === consumer).result.Failed.error);
  assert.equal(await sql(database, 'SELECT value FROM fixture WHERE id = 1'), 'healthy');
  const preview = (await resources(failed)).find(item => item.Spec.Name.startsWith('cdelivery-preview-') && item.Spec.TaskTemplate.ContainerSpec.Mounts?.some(mount => mount.Target === '/var/lib/postgresql'));
  assert(preview);
  assert.equal(await sql(preview.Spec.Name, "SELECT count(*) FROM information_schema.columns WHERE table_name = 'fixture' AND column_name = 'value'"), '0');
  await sql(preview.Spec.Name, "INSERT INTO markers VALUES ('preview-only')");
  assert.equal(await sql(database, "SELECT count(*) FROM markers WHERE value = 'preview-only'"), '0');
  await api('PATCH', `/apps/${database}`, { description: 'must reject while occupied' }, 409);
  await api('PATCH', `/apps/${consumer}`, { description: 'must reject while occupied' }, 409);
  await pass('Breaking migration fails consumer XHealth, clone writes stay isolated, participants remain reserved');
  await close(failed);
  await pass('Manual cancellation removes preview instances, secrets and volumes');

  const invalid = await change('SELECT missing_acceptance_function()');
  const invalidResult = await idle(invalid, 'Open');
  assert(invalidResult.error);
  assert.equal(await sql(database, 'SELECT value FROM fixture WHERE id = 1'), 'healthy');
  await close(invalid);
  await pass('SQL failure leaves the original database intact and can be cancelled');

  const replacement = await change("INSERT INTO markers VALUES ('after-cancel')");
  await idle(replacement, 'Completed');
  assert.equal(await sql(database, "SELECT value FROM markers WHERE value = 'after-cancel'"), 'after-cancel');
  await pass('A new change after cancellation succeeds');

  await api('PATCH', `/apps/${consumer}/xhealth`, { release_check: false });
  const partial = await change("BEGIN; INSERT INTO markers VALUES ('complete-once'); COMMIT; SELECT 1/0");
  const uncertain = await idle(partial, 'Open');
  assert.equal(uncertain.stage, 'Apply');
  assert.equal(await sql(database, "SELECT count(*) FROM markers WHERE value = 'complete-once'"), '1');
  await docker('service', 'rm', `cdelivery-postgres-${partial.replaceAll('-', '')}`);
  await until('executor stopped', async () => assert.deepEqual(await resources(partial), []));
  const since = await docker('info', '--format', '{{.SystemTime}}');
  await api('POST', `/changes/${partial}/complete`, undefined, 200);
  await idle(partial, 'Completed');
  await cleaned(partial, []);
  assert.equal(await sql(database, "SELECT count(*) FROM markers WHERE value = 'complete-once'"), '1');
  const untilTime = await docker('info', '--format', '{{.SystemTime}}');
  const events = await docker('events', '--since', since, '--until', untilTime, '--filter', 'type=service', '--filter', 'event=create', '--format', '{{json .}}');
  assert(!events.includes(`cdelivery-postgres-${partial.replaceAll('-', '')}`), 'Complete must not recreate an executor');
  await api('PATCH', `/apps/${consumer}/xhealth`, { release_check: true });
  await pass('Complete confirms a partial production result without replaying SQL or recreating an executor');

  const interrupted = await change('SELECT pg_sleep(60)');
  await until('rehearsal database running before interruption', async () => {
    const current = await api('GET', `/changes/${interrupted}`);
    assert.equal(current.stage, 'Prepare');
    assert((await resources(interrupted)).some(item => item.Spec.TaskTemplate.ContainerSpec.Mounts?.some(mount => mount.Target === '/var/lib/postgresql')));
  });
  await docker('service', 'update', '--detach', '--force', 'cdelivery');
  await until('controller restarted', async () => assert.notEqual(await container('cdelivery'), main));
  const recovered = await idle(interrupted, 'Open');
  assert.equal(recovered.stage, 'Prepare');
  await close(interrupted);
  await pass('Controller restart preserves an interrupted rehearsal for manual cancellation');

  await until('periodic XHealth succeeds', async () => {
    const health = await api('GET', `/apps/${consumer}/xhealth`);
    assert(health.status.Passed, JSON.stringify(health.status));
  });
  await pass('Periodic XHealth verifies the live database after release');
  if (keep) {
    report.pending = await change('ALTER TABLE fixture DROP COLUMN value');
    assert.equal((await idle(report.pending, 'Open')).stage, 'Check');
    await pass('Retained the latest isolated failed rehearsal for manual review');
  }
} finally {
  await writeFile(`${output}/local-postgresql.json`, JSON.stringify(report, null, 2));
  await api('DELETE', `/apikeys/${keyId}`);
}
