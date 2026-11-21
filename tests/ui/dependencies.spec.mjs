import { createHash, randomUUID } from 'node:crypto';
import { test, expect } from '@playwright/test';
import { headers, login, readEnvironment } from './support.mjs';

const environment = readEnvironment();

test('App dependency binding participates in a failed release and can be cleaned up', async ({ page, request, browser }, testInfo) => {
  test.setTimeout(600_000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const prefix = `du-${randomUUID().slice(0, 4)}`;
  const source = `${prefix}-source`;
  const consumer = `${prefix}-consumer`;
  let changePath;
  let reader;
  let readerContext;
  const created = await request.post('/api/apps', { headers, data: {
    name: source, group: 'tests', image: environment.images.fixture.reference,
    replicas: { Fixed: 1 }, container_options: { command: ['sleep', '86400'] },
  } });
  expect(created.status()).toBe(201);
  try {
    await login(page);
    await page.goto('/dependencies');
    await expect(page.locator('tbody tr').filter({ hasText: source })).toHaveCount(0);
    await expect(page.locator('.query-bar')).toHaveCount(0);
    await page.locator('a[href="/dependencies/.create"]').click();
    await expect(page).toHaveURL(/\/dependencies\/\.create\/neon$/);
    await expect(page.locator('#dependency-token')).toBeVisible();
    await page.locator('a[href="/dependencies/.create/planetscale"]').click();
    await expect(page.locator('#dependency-project')).toHaveCount(0);
    await expect(page.locator('nav.tab-list a[aria-current="page"]')).toHaveText('PlanetScale');
    await expect(page.locator('dialog')).toHaveCount(0);

    await page.goto('/groups/tests/apps/.create');
    await page.locator('#app-name').fill(consumer);
    await page.locator('#image').fill(environment.images.fixture.reference);
    await page.locator('#command_type').selectOption('custom');
    await page.locator('#container_cmd').fill('sleep 86400');
    const env = page.locator('[x-data="containerOptionsData"]');
    await env.locator('.end input').click();
    const modal = page.locator('dialog[open]');
    await page.locator('#env_key').fill('P1_HOST');
    await modal.locator('.tab-list button').nth(2).click();
    await expect(page.locator('#env-dependency')).toBeEnabled();
    await page.locator('#env-dependency').selectOption(`App:${source}`);
    await expect(page.locator('#env-dependency-field')).toBeHidden();
    await expect(page.locator('#env-dependency-field')).toBeDisabled();
    await expect(page.locator('#env_value')).toBeDisabled();
    await modal.locator('input.button:not(.secondary):not(.danger)').click();
    await page.locator('form[x-data="appForm"] > .actions input[type=submit]').click();
    await expect(page).toHaveURL(new RegExp(`/apps/${consumer}$`));
    const configured = await request.get(`/api/apps/${consumer}`, { headers });
    expect(configured.status()).toBe(200);
    expect((await configured.json()).container_options.env).toEqual([
      { name: 'P1_HOST', value: { dependency: { App: source }, field: 'host' } },
    ]);
    await page.goto(`/apps/${consumer}/spec`);
    await env.locator('tbody button').click();
    await expect(page.locator('#env-dependency')).toHaveValue(`App:${source}`);
    await expect(page.locator('#env-dependency option')).not.toContainText([consumer]);
    await modal.locator('input.secondary').click();

    expect((await request.patch(`/api/apps/${source}`, { headers, data: {
      description: 'Before guarded update',
    } })).status()).toBe(200);
    const previous = (await (await request.get(`/api/apps/${source}`, { headers })).json()).rollback;
    const enabled = await request.put(`/api/apps/${consumer}/xhealth`, { headers, data: {
      method: { Custom: { command: ['false'] } }, interval: { unit: 'Hours', value: 24 },
      target: { InternalPort: { port: 8080 } }, release_check: true,
    } });
    expect(enabled.status()).toBe(201);
    await page.goto(`/apps/${source}/spec`);
    await page.locator('#container_cmd').fill('sleep 86401');
    await page.locator('form[x-data="appForm"] > .actions input[type=submit]').click();
    await expect(page).toHaveURL(new RegExp(`/apps/${source}$`));
    const submitted = await (await request.get('/api/changes', { headers })).json();
    const id = submitted.changes.find(change => change.target.App?.name === source).id;
    changePath = `/api/changes/${id}`;
    await expect.poll(async () => {
      const current = await (await request.get(changePath, { headers })).json();
      return current.state === 'Open' && !current.busy;
    }, { timeout: 180_000, intervals: [1000] }).toBe(true);
    const failed = await (await request.get(changePath, { headers })).json();
    const failure = failed.apps.find(app => app.name === consumer).result.Failed.error;
    expect((await page.request.get(`/changes/${id}`)).status()).toBe(404);
    await page.goto(`/apps/${source}`);
    await expect(page.locator('main header h1')).toHaveText(source);
    await expect(page.locator('main header nav')).toBeVisible();
    const current = page.locator('#current-change');
    const diagnostic = current.locator('.change-consumers .change-diagnostics');
    const logs = diagnostic.locator(`a[href="/apps/${consumer}/logs?source=guardian_check"]`);
    await expect(current.locator('h3').first()).toHaveText('Current change');
    await expect(current.locator('.change-consumers h3')).toHaveText(consumer);
    await expect(current.locator('.change-payload details')).not.toHaveAttribute('open');
    await expect(current.locator('relative-time')).toHaveCount(1);
    expect(await current.locator('.change-payload').evaluate(element =>
      Boolean(element.compareDocumentPosition(element.parentElement.querySelector('.change-consumers')) & Node.DOCUMENT_POSITION_FOLLOWING))).toBe(true);
    await expect(logs).toBeVisible();
    await diagnostic.locator('summary').click();
    await expect(diagnostic.locator('pre')).toBeHidden();
    await expect(logs).toBeHidden();
    await diagnostic.locator('summary').click();
    await expect(logs).toBeVisible();
    await expect(page.locator('main .actions.end button')).toHaveCount(2);
    await expect(page.locator('.change-diagnostics')).toHaveAttribute('open');
    await expect(page.locator('.change-consumers .change-diagnostics')).toContainText(failure);
    await expect(page.locator('.change-record > .change-diagnostics')).toHaveCount(0);
    await expect(page.locator('main')).not.toContainText(id);
    await expect(page.locator('main .actions.end form').first()).toHaveAttribute('action', `/changes/${id}/close`);
    await expect(page.locator(`a[href="/apps/${source}/logs?source=guardian_check"]`)).toHaveCount(0);
    await expect(page.locator(`form[action="/changes/${id}/apply"] button`)).toBeEnabled();
    await expect(page.locator(`form[action="/changes/${id}/complete"]`)).toHaveCount(0);
    await expect(page.locator('form[action$="/retry"]')).toHaveCount(0);
    await expect(page.locator('button[form="rollback_form"]')).toBeDisabled();
    await expect(page.locator('button[form="restart_form"]')).toBeDisabled();
    const during = await (await request.get(`/api/apps/${source}`, { headers })).json();
    expect(during.rollback).toEqual(previous);
    await page.goto(`/apps/${consumer}`);
    await expect(current.locator('.change-consumers h3')).toHaveCount(0);
    await expect(current.locator(`a[href="/apps/${source}#current-change"]`)).toBeVisible();
    await expect(logs).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('current-change.png') });
    const email = `${prefix}-reader@cdelivery.test`;
    const password = 'Change-Reader-123456';
    const account = await request.post('/api/users', { headers, data: {
      first_name: 'Change', last_name: 'Reader', email, role: 'Reader',
      password_sha256: createHash('sha256').update(password).digest('hex'),
    } });
    expect(account.status()).toBe(201);
    reader = await account.json();
    readerContext = await browser.newContext({ baseURL: environment.base_url });
    const readerPage = await readerContext.newPage();
    await login(readerPage, email, password);
    await readerPage.goto(`/apps/${consumer}`);
    await expect(readerPage.locator('#current-change .change-consumers details')).toBeVisible();
    await expect(readerPage.locator('#current-change form')).toHaveCount(0);
    await expect(readerPage.locator('button[form="restart_form"]')).toHaveCount(0);
    await expect(readerPage.locator('button[form="rollback_form"]')).toHaveCount(0);
    page.once('dialog', dialog => dialog.accept());
    await page.locator(`form[action="/changes/${id}/close"] button`).click();
    await expect(page).toHaveURL(new RegExp(`/apps/${source}$`));
    await expect.poll(async () => (await (await request.get(changePath, { headers })).json()).state,
      { timeout: 120_000, intervals: [1000] }).toBe('Closed');
    const unchanged = await (await request.get(`/api/apps/${source}`, { headers })).json();
    expect(unchanged.container_options.command).toEqual(['sleep', '86400']);
    expect(unchanged.rollback).toEqual(previous);
    await expect(current).toHaveCount(0);
    await expect(page.locator('button[form="rollback_form"]')).toBeEnabled();
    await expect(page.locator('button[form="restart_form"]')).toBeEnabled();

    expect((await request.put(`/api/apps/${source}/xhealth`, { headers, data: {
      method: { Custom: { command: ['true'] } }, interval: { unit: 'Hours', value: 24 },
      target: { InternalPort: { port: 8080 } }, release_check: true,
    } })).status()).toBe(201);
    const multiple = await request.patch(`/api/apps/${source}`, { headers, data: {
      container_options: { command: ['sleep', '86402'] },
    } });
    expect(multiple.status()).toBe(202);
    const nextId = (await multiple.json()).change;
    changePath = `/api/changes/${nextId}`;
    await expect.poll(async () => {
      const change = await (await request.get(changePath, { headers })).json();
      return change.state === 'Open' && !change.busy;
    }, { timeout: 180_000, intervals: [1000] }).toBe(true);
    await page.goto(`/apps/${source}`);
    await expect(current.locator('.change-consumers h3')).toHaveText([consumer, source]);
    await expect(current.locator('.change-consumers details[open]')).toHaveCount(1);
    const passed = current.locator('.change-consumers li').filter({ hasText: source });
    await expect(passed.locator('details')).not.toHaveAttribute('open');
    await expect(passed.locator('summary')).toHaveText('Check passed');
    await expect(passed.locator('a[href*="guardian_check"]')).toBeHidden();
    await passed.locator('summary').click();
    await expect(passed.locator('a[href*="guardian_check"]')).toBeVisible();
    page.once('dialog', dialog => dialog.accept());
    await current.locator(`form[action="/changes/${nextId}/apply"] button`).click();
    await expect(page).toHaveURL(new RegExp(`/apps/${source}$`));
    await expect.poll(async () => (await (await request.get(changePath, { headers })).json()).state,
      { timeout: 120_000, intervals: [1000] }).toBe('Completed');
    await expect(current).toHaveCount(0);
    const applied = await (await request.get(`/api/apps/${source}`, { headers })).json();
    expect(applied.container_options.command).toEqual(['sleep', '86402']);
    expect(applied.rollback.Available.changes.container_options.command).toEqual(['sleep', '86402']);
    expect((await request.get(`/api/changes/${id}`, { headers })).status()).toBe(404);
    expect(errors).toEqual([]);
  } finally {
    if (readerContext) await readerContext.close();
    if (reader) expect((await request.delete(`/api/users/${reader.id}`, { headers })).status()).toBe(204);
    if (changePath) {
      const current = await (await request.get(changePath, { headers })).json();
      if (current.state === 'Open' && !current.busy) {
        await request.post(`${changePath}/close`, { headers });
        await expect.poll(async () => (await (await request.get(changePath, { headers })).json()).state,
          { timeout: 120_000, intervals: [1000] }).toBe('Closed');
      }
    }
    await request.delete(`/api/apps/${consumer}`, { headers });
    expect((await request.delete(`/api/apps/${source}`, { headers })).status()).toBe(204);
  }
});
