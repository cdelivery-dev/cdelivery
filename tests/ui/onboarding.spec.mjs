import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { test, expect } from '@playwright/test';

const run = promisify(execFile);
const state = resolve(import.meta.dirname, '../../target/tests/current');
const credentials = JSON.parse(readFileSync(resolve(state, 'credentials.json'), 'utf8'));
const environment = JSON.parse(readFileSync(resolve(state, 'env.json'), 'utf8'));
const headers = { Authorization: `Bearer ${credentials.api_key}` };
const origin = new URL(environment.base_url).origin;

async function login(page, destination = '/', email = credentials.email, password = credentials.password) {
  await page.goto('/login');
  await page.locator('#email').fill(email);
  await page.locator('#password').fill(password);
  await page.locator('#login_form input[type=submit]').click();
  await expect(page).toHaveURL(`${origin}/`);
  if (destination !== '/') await page.goto(destination);
}

async function ready(request, docker) {
  await expect.poll(async () => {
    try {
      const service = JSON.parse(await docker(['service', 'inspect', 'cdelivery']))[0];
      if (service.UpdateStatus && service.UpdateStatus.State !== 'completed') return 0;
      return (await request.get('/api/me', { headers })).status();
    } catch {
      return 0;
    }
  }, { timeout: 180_000, intervals: [1000] }).toBe(200);
}

test('setup navigation is explicit, absent from settings and restricted to administrators', async ({ page, request }) => {
  expect((await request.get('/setup', { maxRedirects: 0 })).headers().location).toBe('/setup/platform');
  await page.goto('/setup');
  await expect(page).toHaveURL(`${origin}/login`);
  await page.goto('/setup/platform');
  await expect(page).toHaveURL(`${origin}/login`);
  const invalid = await page.request.post('/login', {
    headers: { Origin: origin },
    form: { payload: JSON.stringify({ email: 'invalid', password_sha256: 'invalid' }) },
  });
  expect(invalid.status()).toBe(422);
  expect(await invalid.text()).toContain('action="/login"');
  await login(page, '/setup/platform');
  await page.goto('/setup');
  await expect(page).toHaveURL(/\/setup\/platform$/);
  const before = await (await request.get('/api/insight', { headers })).json();
  await expect(page.locator('#insight')).toHaveCount(0);
  await expect(page.locator('#platform-setup-form input[type=submit]')).toHaveCount(1);
  await expect(page.locator('#platform-setup-form form')).toHaveCount(0);
  await page.getByRole('link', { name: 'Skip', exact: true }).click();
  await expect(page).toHaveURL(`${origin}/`);
  await page.context().clearCookies();
  await page.goto('/setup/platform');
  await expect(page).toHaveURL(`${origin}/login`);
  await login(page);
  await expect(page).toHaveURL(`${origin}/`);
  await page.goto('/settings/cdelivery');
  await expect(page.locator('nav a[href="/setup/platform"]')).toHaveCount(0);
  expect(await (await request.get('/api/insight', { headers })).json()).toEqual(before);
  const password = 'Onboarding-Aa123456';
  const email = `setup-${randomUUID().slice(0, 8)}@cdelivery.test`;
  const created = await request.post('/api/users', { headers, data: {
    first_name: 'Setup', last_name: 'Reader', email, role: 'Reader',
    password_sha256: createHash('sha256').update(password).digest('hex'),
  } });
  expect(created.status()).toBe(201);
  const user = await created.json();
  try {
    await login(page, '/', email, password);
    expect((await page.goto('/setup/platform')).status()).toBe(403);
    const denied = await page.request.post('/setup/platform', {
      headers: { Origin: origin, 'Content-Type': 'application/x-www-form-urlencoded' },
      data: 'payload=invalid-json',
    });
    expect(denied.status()).toBe(403);
  } finally {
    expect((await request.delete(`/api/users/${user.id}`, { headers })).status()).toBe(204);
  }
});

test('administrator setup resumes from its intermediate page without recreating the account', async ({ page, request }) => {
  test.setTimeout(360_000);
  const docker = async args => (await run('orb', ['-m', environment.machines[0], '-u', 'root', '/usr/bin/docker', ...args], { timeout: 30_000 })).stdout;
  const identity = await (await request.get('/api/me', { headers })).json();
  const [before] = JSON.parse(await docker(['service', 'inspect', 'cdelivery']));
  const secret = before.Spec.TaskTemplate.ContainerSpec.Secrets.find(secret => secret.SecretName === 'cdelivery');
  await login(page);
  try {
    await docker(['service', 'update', '--detach', '--secret-rm', 'cdelivery', 'cdelivery']);
    await expect.poll(async () => {
      try { return await (await request.get('/setup', { maxRedirects: 0 })).text(); } catch { return ''; }
    }, { timeout: 120_000, intervals: [1000] }).toContain('data-setup-state="NeedsResume"');
    await page.goto('/setup');
    await expect(page.locator('#setup_resume_button')).toBeEnabled();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    let navigations = 0;
    page.on('request', request => {
      if (request.isNavigationRequest()) navigations += 1;
    });
    let submissions = 0;
    await page.route('**/setup/resume', async route => {
      submissions += 1;
      await route.fulfill({ status: 500, contentType: 'application/problem+json', body: JSON.stringify({ detail: 'attachment failed' }) });
    });
    await page.route('**/setup', route => route.request().isNavigationRequest() ? route.continue() : route.abort('connectionreset'));
    await page.locator('#setup_resume_button').click();
    await expect(page.getByRole('alert')).toBeVisible();
    await expect(page.locator('[data-preparation] .diagnostic')).toHaveText('attachment failed');
    await expect(page.locator('#setup_resume_button')).toBeVisible();
    await expect(page.locator('#setup_resume_button')).toBeEnabled();
    await page.unroute('**/setup');
    await page.unroute('**/setup/resume');
    await page.clock.install();
    let probes = 0;
    await page.route('**/setup/resume', async route => {
      submissions += 1;
      await route.abort('connectionreset');
    });
    await page.route('**/setup', async route => {
      probes += 1;
      await route.abort('connectionreset');
    });
    await page.locator('#setup_resume_button').click();
    await expect.poll(() => probes).toBeGreaterThan(0);
    await page.clock.fastForward(120_000);
    await expect(page.getByRole('status')).toContainText('Completion has not been confirmed yet.');
    await expect(page.getByRole('link')).toHaveCount(0);
    expect(navigations).toBe(0);
    const stopped = probes;
    await page.clock.fastForward(20_000);
    expect(probes).toBe(stopped);
    await page.getByRole('button', { name: 'Check again', exact: true }).click();
    await expect.poll(() => probes).toBeGreaterThan(stopped);
    expect(submissions).toBe(2);
    await page.unroute('**/setup');
    await page.clock.fastForward(2000);
    await expect(page.locator('#setup_resume_button')).toBeVisible();
    await expect(page.locator('#setup_resume_button')).toBeEnabled();
    await expect(page.getByRole('alert')).toBeHidden();
    expect(submissions).toBe(2);
    expect(navigations).toBe(0);
    await page.unroute('**/setup/resume');
    await page.clock.resume();
    let disconnected = 0;
    await page.route('**/setup', async route => {
      disconnected += 1;
      await route.abort('connectionrefused');
    });
    page.on('request', request => {
      if (request.method() === 'POST' && new URL(request.url()).pathname === '/setup/resume') submissions += 1;
    });
    const [resumed] = await Promise.all([
      page.waitForResponse(response => response.request().method() === 'POST' && response.url().endsWith('/setup/resume')),
      page.locator('#setup_resume_button').click(),
    ]);
    expect(resumed.status()).toBe(202);
    expect(resumed.headers()['set-cookie']).toBeUndefined();
    await expect.poll(() => disconnected).toBeGreaterThan(0);
    await expect(page.getByRole('status')).toContainText('Starting up. Setup will continue automatically.');
    await expect(page.locator('#setup_resume_button')).toBeHidden();
    await expect(page).toHaveURL(`${origin}/setup`);
    expect(navigations).toBe(0);
    await page.unroute('**/setup');
    await expect(page).toHaveURL(`${origin}/setup/platform`, { timeout: 120_000 });
    expect(navigations).toBe(1);
    expect(submissions).toBe(3);
    expect(await (await request.get('/api/me', { headers })).json()).toEqual(identity);
    const [after] = JSON.parse(await docker(['service', 'inspect', 'cdelivery']));
    expect(after.Spec.TaskTemplate.ContainerSpec.Secrets.find(secret => secret.SecretName === 'cdelivery').SecretID).toBe(secret.SecretID);
  } finally {
    const [current] = JSON.parse(await docker(['service', 'inspect', 'cdelivery']));
    if (!current.Spec.TaskTemplate.ContainerSpec.Secrets?.some(secret => secret.SecretName === 'cdelivery')) {
      await docker(['service', 'update', '--detach', '--secret-add', 'source=cdelivery,target=cdelivery', 'cdelivery']);
    }
    await ready(request, docker);
  }
});

for (const locale of ['en', 'zh-CN']) {
  test.describe(locale, () => {
    test.use({ locale });
    test('domain setup validates certificates and keeps setup available after saving', async ({ page, request }, info) => {
      const domain = `setup-${randomUUID().slice(0, 8)}.cdelivery.test`;
      const baseline = await (await request.get('/api/instance', { headers })).json();
      const gateway = await (await request.get('/api/pingress', { headers })).json();
      const insight = await (await request.get('/api/insight', { headers })).json();
      expect(gateway.acme_enabled).toBe(false);
      expect(baseline.domain).toBeNull();
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      try {
        await login(page, '/setup/platform');
        await page.locator('#setup-domain').fill('invalid');
        await page.locator('#platform-setup-form input[type=submit]').click();
        await expect(page.locator('#setup-domain')).toBeFocused();
        await page.locator('#setup-domain').fill(domain);
        const [failed] = await Promise.all([
          page.waitForResponse(response => response.request().method() === 'POST' && response.url().endsWith('/setup/platform')),
          page.locator('#platform-setup-form input[type=submit]').click(),
        ]);
        expect(failed.status()).toBe(400);
        await expect(page.locator('[role=alert]:visible')).toBeVisible();
        await expect(page.locator('#setup-domain')).toHaveValue(domain);
        const dialog = page.getByRole('dialog', { name: locale === 'en' ? 'Request failed' : '请求失败' });
        await expect(dialog).toBeVisible();
        await dialog.getByRole('button', { name: locale === 'en' ? 'Back' : '返回', exact: true }).click();
        await expect(dialog).toBeHidden();
        expect((await (await request.get('/api/instance', { headers })).json()).domain).toBeNull();
        expect(await (await request.get('/api/pingress', { headers })).json()).toEqual(gateway);
        const key = info.outputPath('domain-key.pem');
        const cert = info.outputPath('domain-cert.pem');
        await run('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-subj', `/CN=${domain}`, '-addext', `subjectAltName=DNS:${domain}`, '-keyout', key, '-out', cert]);
        expect((await request.put(`/api/pingress/certs/${domain}`, { headers, data: {
          cert_chain_pem: readFileSync(cert, 'utf8'), private_key_pem: readFileSync(key, 'utf8'),
        } })).status()).toBe(201);
        await page.locator('#setup-domain').fill(domain.toUpperCase());
        await page.locator('#platform-setup-form input[type=submit]').click();
        await expect(page).toHaveURL(`${origin}/`);
        await page.goto('/setup/platform');
        await expect(page).toHaveURL(/\/setup\/platform$/);
        await expect(page.locator('#platform-setup-form input[type=submit]')).toBeVisible();
        await expect(page.locator('#domain, #insight')).toHaveCount(0);
        expect(await (await request.get('/api/insight', { headers })).json()).toEqual(insight);
        const stale = await page.request.post('/setup/platform', {
          headers: { Origin: origin }, form: { payload: JSON.stringify({ domain: { domain: 'other.cdelivery.test' }, insight: {} }) },
        });
        expect(stale.status()).toBe(200);
        expect((await (await request.get('/api/instance', { headers })).json()).domain).toEqual({ domain, https: true });
        expect((await request.delete(`/api/pingress/certs/${domain}`, { headers })).status()).toBe(204);
        await page.goto('/setup/platform');
        await expect(page).toHaveURL(/\/setup\/platform$/);
        const invalid = await page.request.post('/setup/platform', {
          headers: { Origin: origin }, form: { payload: JSON.stringify({ domain: { domain: 'invalid' }, insight: {} }) },
          maxRedirects: 0,
        });
        expect(invalid.status()).toBe(422);
        const error = await invalid.text();
        expect(error).toContain('id="error"');
        expect(error).toContain('<code>domain.domain</code>');
        expect(errors).toEqual([]);
      } finally {
        expect((await request.patch('/api/instance', { headers, data: { domain: baseline.domain } })).status()).toBe(202);
        expect((await request.delete(`/api/pingress/certs/${domain}`, { headers })).status()).toBe(204);
      }
    });
  });
}

test('fresh ingress setup selects a single node and validates before deployment', async ({ page, request }) => {
  const gateway = await (await request.get('/api/pingress', { headers })).json();
  const nodes = (await (await request.get('/api/nodes', { headers })).json()).nodes;
  const active = nodes.filter(node => node.state === 'Ready' && node.availability === 'Active');
  expect((await request.delete('/api/pingress', { headers })).status()).toBe(204);
  try {
    await login(page, '/setup/platform');
    await expect(page.locator('#setup-node')).toHaveValue(active.length === 1 ? active[0].id : '');
    await expect(page.locator('#setup-node')).toBeEnabled();
    await page.locator('#setup-domain').fill('invalid');
    await page.locator('#platform-setup-form input[type=submit]').click();
    await expect(page.locator('#setup-domain')).toBeFocused();
    const invalid = await page.request.post('/setup/platform', {
      headers: { Origin: origin },
      form: { payload: JSON.stringify({ domain: { domain: 'https://invalid.example.com/path', placement: { NodeId: active[0].id } }, insight: {} }) },
    });
    expect(invalid.status()).toBe(422);
    expect((await request.get('/api/pingress', { headers })).status()).toBe(404);
    await page.reload();
    await expect(page.locator('#insight')).toHaveCount(0);
    await page.locator('#platform-setup-form input[type=submit]').click();
    await expect(page).toHaveURL(`${origin}/`);
    expect((await request.get('/api/pingress', { headers })).status()).toBe(404);
  } finally {
    expect((await request.put('/api/pingress', { headers, data: {
      version: gateway.version, placement: gateway.placement, acme: null,
    } })).status()).toBe(201);
  }
});

test('Insight setup shares storage fields, uses defaults and resumes without overwriting configuration', async ({ page, request }, info) => {
  test.setTimeout(900_000);
  const instance = await (await request.get('/api/instance', { headers })).json();
  const baseline = await (await request.get('/api/insight', { headers })).json();
  const domain = `setup-${randomUUID().slice(0, 8)}.cdelivery.test`;
  const storage = baseline.server.storage;
  if (storage.S3Compatible) storage.S3Compatible.secret_access_key = '2'.repeat(64);
  const restored = {
    storage, placement: baseline.server.placement, retention_days: baseline.server.retention_days,
    server_version: environment.images.insight.version, agent_version: environment.images['insight-agent'].version,
    sample_interval_secs: baseline.agent.sample_interval_secs, flush_interval: baseline.server.flush_interval,
  };
  const docker = async args => (await run('orb', ['-m', environment.machines[0], '-u', 'root', '/usr/bin/docker', ...args], { timeout: 30_000 })).stdout;
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  try {
    expect((await request.delete('/api/insight', { headers })).status()).toBe(204);
    await ready(request, docker);
    await login(page, '/setup/platform');
    await expect(page.locator('#domain')).toBeVisible();
    await expect(page.locator('#insight')).toBeVisible();
    const invalid = await page.request.post('/setup/platform', {
      headers: { Origin: origin },
      form: { payload: JSON.stringify({ domain: { domain }, insight: { retention_days: 0 } }) },
    });
    expect(invalid.status()).toBe(422);
    expect((await (await request.get('/api/instance', { headers })).json()).domain).toBeNull();
    const stopped = await page.request.post('/setup/platform', {
      headers: { Origin: origin },
      form: { payload: JSON.stringify({ domain: { domain }, insight: { placement: baseline.server.placement } }) },
    });
    expect(stopped.status()).toBe(400);
    expect((await (await request.get('/api/insight', { headers })).json()).status).toBe('Disabled');
    const key = info.outputPath('partial-key.pem');
    const cert = info.outputPath('partial-cert.pem');
    await run('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-subj', `/CN=${domain}`, '-addext', `subjectAltName=DNS:${domain}`, '-keyout', key, '-out', cert]);
    expect((await request.put(`/api/pingress/certs/${domain}`, { headers, data: {
      cert_chain_pem: readFileSync(cert, 'utf8'), private_key_pem: readFileSync(key, 'utf8'),
    } })).status()).toBe(201);
    const partial = await page.request.post('/setup/platform', {
      headers: { Origin: origin },
      form: { payload: JSON.stringify({
        domain: { domain },
        insight: { placement: { NodeId: 'missing' }, retention_days: 45, storage: 'Local' },
      }) },
    });
    expect(partial.status()).toBe(400);
    expect(await partial.text()).toContain('id="error"');
    expect((await (await request.get('/api/instance', { headers })).json()).domain).toEqual({ domain, https: true });
    expect((await (await request.get('/api/insight', { headers })).json()).status).toBe('Disabled');
    await page.reload();
    await expect(page).toHaveURL(/\/setup\/platform$/);
    await expect(page.locator('#domain')).toHaveCount(0);
    await expect(page.locator('#insight-retention')).toHaveValue('90');
    await expect(page.locator('#insight-storage-type')).toHaveValue('Local');
    await expect(page.locator('#insight-server-version, #insight-agent-version, #insight-sample-interval, #insight-flush-interval')).toHaveCount(0);
    if (environment.profile === 'single') {
      await expect(page.locator('#insight-node')).toHaveValue(baseline.server.placement.NodeId);
    } else {
      await expect(page.locator('#insight-node')).toHaveValue('');
    }
    await page.locator('#insight-storage-type').selectOption('S3Compatible');
    await expect(page.locator('#insight-storage-secret-key')).toBeVisible();
    expect(await page.locator('#insight-storage-secret-key').evaluate(input => input.required)).toBe(true);
    if (environment.profile === 'single') {
      await page.locator('#insight-storage-type').selectOption('Local');
      expect(await page.locator('#insight-storage-secret-key').evaluate(input => input.disabled)).toBe(true);
    } else {
      for (const [id, key] of [['endpoint', 'endpoint'], ['region', 'region'], ['s3-bucket', 'bucket_name'], ['access-key', 'access_key_id'], ['secret-key', 'secret_access_key']]) {
        await page.locator(`#insight-storage-${id}`).fill(storage.S3Compatible[key]);
      }
      await page.locator('#insight-node').selectOption(baseline.server.placement.NodeId);
    }
    const [enabled] = await Promise.all([
      page.waitForResponse(response => response.request().method() === 'POST' && response.url().endsWith('/setup/platform')),
      page.locator('#platform-setup-form input[type=submit]').click(),
    ]);
    expect(enabled.status()).toBe(202);
    const payload = JSON.parse(new URLSearchParams(enabled.request().postData()).get('payload'));
    expect(Object.keys(payload)).toEqual(['insight']);
    expect(Object.keys(payload.insight).sort()).toEqual(['placement', 'retention_days', 'storage']);
    await ready(request, docker);
    await expect(page).toHaveURL(`${origin}/`, { timeout: 120_000 });
    await expect(page.locator('main .diagnostic')).toHaveCount(0);
    const configured = await (await request.get('/api/insight', { headers })).json();
    expect(configured.status).toBe('Enabled');
    expect(configured.server.retention_days).toBe(90);
    expect(configured.server.flush_interval).toEqual({ unit: 'Minutes', value: 3 });
    expect(configured.agent.sample_interval_secs).toBe(5);
    await page.goto('/setup/platform');
    await expect(page).toHaveURL(/\/setup\/platform$/);
    await page.goto('/insight');
    await expect(page.locator('#insight-sample-interval')).toHaveValue('5');
    await expect(page.locator('#insight-server-version')).toBeVisible();
    await page.locator('form[action="/insight/configure"]').evaluate(form => form.addEventListener('submit', event => event.preventDefault(), { once: true }));
    await page.locator('form[action="/insight/configure"] input[type=submit]:not([form])').click();
    await expect(page.locator('form[action="/insight/configure"] input[type=submit]:not([form])')).toBeDisabled();
    await expect(page.locator('[data-preparation] .actions')).toBeHidden();
    await page.reload();
    const [unchanged] = await Promise.all([
      page.waitForResponse(response => response.request().method() === 'POST' && response.url().endsWith('/insight/configure')),
      page.locator('form[action="/insight/configure"] input[type=submit]:not([form])').click(),
    ]);
    expect(unchanged.status()).toBe(303);
    expect(JSON.parse(new URLSearchParams(unchanged.request().postData()).get('payload'))).toEqual({});
    expect(await (await request.get('/api/insight', { headers })).json()).toEqual(configured);
    expect((await request.patch('/api/insight', { headers, data: {
      retention_days: 7, sample_interval_secs: 2, flush_interval: { unit: 'Seconds', value: 9 },
    } })).status()).toBe(200);
    const customized = await (await request.get('/api/insight', { headers })).json();
    const stale = await page.request.post('/setup/platform', {
      headers: { Origin: origin }, form: { payload: JSON.stringify({ insight: { ...payload.insight, retention_days: 90 } }) },
    });
    expect(stale.status()).toBe(200);
    expect(await (await request.get('/api/insight', { headers })).json()).toEqual(customized);
    await docker(['service', 'update', '--detach', '--log-driver', 'local', 'cdelivery-pingress']);
    await expect.poll(async () => {
      const [gateway] = JSON.parse(await docker(['service', 'inspect', 'cdelivery-pingress']) || '[]');
      return gateway?.UpdateStatus?.State;
    }, { timeout: 180_000, intervals: [1000] }).toBe('completed');
    await page.goto('/setup/platform');
    await page.reload();
    await expect(page).toHaveURL(/\/setup\/platform$/);
    await expect(page.getByText('Your existing Insight configuration will be used when you save.')).toBeVisible();
    await page.locator('#platform-setup-form input[type=submit]').click();
    await expect(page).toHaveURL(`${origin}/`);
    const services = ['cdelivery', 'cdelivery-pingress', 'cdelivery-insight', 'cdelivery-insight-agent'];
    await expect.poll(async () => {
      const state = JSON.parse(await docker(['service', 'inspect', ...services]));
      return state.every(service => !service.UpdateStatus || service.UpdateStatus.State === 'completed');
    }, { timeout: 180_000, intervals: [1000] }).toBe(true);
    const before = JSON.parse(await docker(['service', 'inspect', ...services]));
    for (const service of before.slice(0, 2)) expect(service.Spec.TaskTemplate.LogDriver.Name).toBe('fluentd');
    expect(await (await request.get('/api/insight', { headers })).json()).toEqual(customized);
    await page.goto('/setup/platform');
    await page.locator('#platform-setup-form input[type=submit]').click();
    await expect(page).toHaveURL(`${origin}/`);
    const after = JSON.parse(await docker(['service', 'inspect', ...services]));
    expect(after.map(service => [service.ID, service.Version.Index, service.Spec]))
      .toEqual(before.map(service => [service.ID, service.Version.Index, service.Spec]));
    await docker(['service', 'rm', 'cdelivery-insight-agent']);
    await page.goto('/setup/platform');
    await expect(page.locator('#platform-setup-form')).toHaveCount(1);
    await expect(page.locator('#insight form')).toHaveCount(0);
    await expect(page.locator('#insight-storage-type')).toHaveCount(0);
    await page.locator('#platform-setup-form input[type=submit]').click();
    await ready(request, docker);
    await expect(page).toHaveURL(`${origin}/`, { timeout: 120_000 });
    const resumed = await (await request.get('/api/insight', { headers })).json();
    expect(resumed.status).toBe('Enabled');
    expect(resumed.server).toEqual(customized.server);
    expect(resumed.agent.sample_interval_secs).toBe(5);
    if (environment.profile === 'single') {
      expect((await request.patch('/api/insight', { headers, data: { sample_interval_secs: 2 } })).status()).toBe(200);
      const agent = (await (await request.get('/api/insight', { headers })).json()).agent;
      await docker(['service', 'rm', 'cdelivery-insight']);
      await page.goto('/setup/platform');
      await expect(page.locator('#insight-storage-type')).toHaveValue('Local');
      await expect(page.locator('#insight-retention')).toHaveValue('90');
      await page.locator('#platform-setup-form input[type=submit]').click();
      await ready(request, docker);
      await expect(page).toHaveURL(`${origin}/`, { timeout: 120_000 });
      const recovered = await (await request.get('/api/insight', { headers })).json();
      expect(recovered.status).toBe('Enabled');
      expect(recovered.agent).toEqual(agent);
    }
    await page.goto('/insight');
    page.once('dialog', dialog => dialog.dismiss());
    await page.locator('input[form="insight-disable-form"]').click();
    expect((await (await request.get('/api/insight', { headers })).json()).status).toBe('Enabled');
    page.once('dialog', dialog => dialog.accept());
    const [disabled] = await Promise.all([
      page.waitForResponse(response => response.request().method() === 'POST' && response.url().endsWith('/insight/disable')),
      page.locator('input[form="insight-disable-form"]').click(),
    ]);
    expect(disabled.status()).toBe(202);
    await expect(page.locator('input[form="insight-disable-form"]')).toHaveCount(0, { timeout: 120_000 });
    await ready(request, docker);
    expect((await (await request.get('/api/insight', { headers })).json()).status).toBe('Disabled');
    if (storage.S3Compatible) {
      await page.locator('#insight-storage-type').selectOption('S3Compatible');
      for (const [id, key] of [['endpoint', 'endpoint'], ['region', 'region'], ['s3-bucket', 'bucket_name'], ['access-key', 'access_key_id'], ['secret-key', 'secret_access_key']]) {
        await page.locator(`#insight-storage-${id}`).fill(storage.S3Compatible[key]);
      }
    }
    await page.locator('#insight-node').selectOption(baseline.server.placement.NodeId);
    const [reenabled] = await Promise.all([
      page.waitForResponse(response => response.request().method() === 'POST' && response.url().endsWith('/insight/configure')),
      page.locator('form[action="/insight/configure"] input[type=submit]:not([form])').click(),
    ]);
    expect(reenabled.status()).toBe(202);
    await expect(page.locator('input[form="insight-disable-form"]')).toBeVisible({ timeout: 120_000 });
    await ready(request, docker);
    await expect(page).toHaveURL(`${origin}/insight`);
    expect((await (await request.get('/api/insight', { headers })).json()).status).toBe('Enabled');
    expect(errors).toEqual([]);
  } finally {
    await ready(request, docker);
    expect((await request.patch('/api/insight', { headers, data: restored })).status()).toBe(200);
    expect((await request.patch('/api/instance', { headers, data: { domain: instance.domain } })).status()).toBe(202);
    expect((await request.delete(`/api/pingress/certs/${domain}`, { headers })).status()).toBe(204);
    await ready(request, docker);
  }
});

test('preparation retains the page after a lost submission response and only retries reads', async ({ page }) => {
  await login(page, '/setup/platform');
  let submissions = 0;
  let probes = 0;
  let ready = false;
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/setup/platform', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    submissions += 1;
    expect((await route.fetch()).status()).toBe(202);
    await route.abort('connectionreset');
  });
  await page.route('**/api/overview', async route => {
    probes += 1;
    if (probes === 1) return route.abort('connectionrefused');
    await route.fulfill({ json: { metrics: ready ? { status: 'Ready', charts: { series: [] } } : { status: 'Unavailable', error: 'warming up' } } });
  });
  await page.locator('#platform-setup-form input[type=submit]').click();
  await expect.poll(() => probes).toBeGreaterThanOrEqual(2);
  await expect(page).toHaveURL(`${origin}/setup/platform`);
  await expect(page.locator('[data-preparation] [role=status]:visible')).toHaveText(['Applying settings']);
  const dialog = page.getByRole('dialog', { name: 'Applying settings', exact: true });
  expect(await dialog.evaluate(element => element.matches(':modal'))).toBe(true);
  await expect(dialog).toBeFocused();
  await page.keyboard.press('Escape');
  await page.mouse.click(0, 0);
  await expect(dialog).toBeVisible();
  await page.locator('#setup-domain').evaluate(input => input.focus());
  await expect(page.locator('#setup-domain')).not.toBeFocused();
  await page.locator('#platform-setup-form').evaluate(form => form.requestSubmit());
  expect(submissions).toBe(1);
  ready = true;
  await expect(page).toHaveURL(`${origin}/`);
  expect(submissions).toBe(1);
  expect(errors).toEqual([]);
});

test('preparation includes submission in its deadline and rechecking does not submit again', async ({ page }) => {
  await page.clock.install();
  await login(page, '/setup/platform');
  let submissions = 0;
  let probes = 0;
  let ready = false;
  await page.route('**/setup/platform', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    submissions += 1;
    const response = await route.fetch();
    expect(response.status()).toBe(202);
    await page.clock.fastForward(60_000);
    await route.fulfill({ response });
  });
  await page.route('**/api/overview', async route => {
    probes += 1;
    if (probes === 2 && !ready) return;
    await route.fulfill({ json: { metrics: ready ? { status: 'Ready', charts: { series: [] } } : { status: 'Unavailable', error: 'startup failed' } } });
  });
  await page.locator('#platform-setup-form input[type=submit]').click();
  await expect.poll(() => probes).toBeGreaterThan(0);
  await expect(page.locator('[data-preparation] .diagnostic')).toHaveText('startup failed');
  await page.clock.fastForward(2000);
  await expect.poll(() => probes).toBe(2);
  await page.clock.fastForward(58_000);
  await expect(page.locator('[data-preparation] [role=status]:visible')).toHaveText(['Completion has not been confirmed yet.']);
  await expect(page.getByRole('dialog', { name: 'Completion has not been confirmed yet.', exact: true })).toBeVisible();
  await expect(page.locator('[data-preparation] .diagnostic')).toHaveText('startup failed');
  const stopped = probes;
  await page.clock.fastForward(20_000);
  expect(probes).toBe(stopped);
  expect(submissions).toBe(1);
  ready = true;
  await page.getByRole('button', { name: 'Check again', exact: true }).click();
  await expect(page).toHaveURL(`${origin}/`);
  expect(submissions).toBe(1);
});

test('a gateway timeout after saving is recovered by reads without another submission', async ({ page }) => {
  await login(page, '/setup/platform');
  let submissions = 0;
  await page.route('**/setup/platform', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    submissions += 1;
    expect((await route.fetch()).status()).toBe(202);
    await route.fulfill({
      status: 504, contentType: 'application/problem+json',
      body: JSON.stringify({ status: 504, code: 'OperationTimeout', detail: 'response timed out' }),
    });
  });
  await page.locator('#platform-setup-form input[type=submit]').click();
  await expect(page).toHaveURL(`${origin}/`);
  expect(submissions).toBe(1);
});

test('a lost platform submission cannot complete with an unsaved domain', async ({ page }) => {
  await page.clock.install();
  await login(page, '/setup/platform');
  let submissions = 0;
  let checked = 0;
  await page.locator('#setup-domain').fill('unconfirmed.cdelivery.test');
  await page.route('**/setup/platform', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    submissions += 1;
    await route.abort('connectionreset');
  });
  await page.route('**/api/instance', async route => {
    checked += 1;
    await route.continue();
  });
  await page.locator('#platform-setup-form input[type=submit]').click();
  await expect.poll(() => checked).toBeGreaterThan(0);
  await page.clock.fastForward(120_000);
  await expect(page).toHaveURL(`${origin}/setup/platform`);
  await expect(page.locator('[data-preparation] [role=status]:visible')).toHaveText(['Completion has not been confirmed yet.']);
  expect(submissions).toBe(1);
});
