import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { test, expect } from '@playwright/test';

const state = resolve(import.meta.dirname, '../../target/tests/current');
const credentials = JSON.parse(readFileSync(resolve(state, 'credentials.json'), 'utf8'));
const environment = JSON.parse(readFileSync(resolve(state, 'env.json'), 'utf8'));
const headers = { Authorization: `Bearer ${credentials.api_key}` };

test('Secret modal and typed app bindings preserve drafts and references', async ({ browser, page, request }) => {
  test.setTimeout(360_000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/login');
  await page.locator('#email').fill(credentials.email);
  await page.locator('#password').fill(credentials.password);
  await page.locator('#login_form input[type=submit]').click();
  await expect(page).not.toHaveURL(/\/login/);
  const name = `sui-${randomUUID().slice(0, 6)}`;
  const textName = `${name}-text`;
  const fileName = `${name}-file`;
  const content = '  secret browser fixture\n第二行\n';
  await page.goto('/secrets?create=Text');
  const modal = page.locator('dialog[open]');
  await expect(modal.locator('#secret-text')).toBeVisible();
  await page.locator('#secret-name').fill(textName);
  await page.locator('#secret-description').fill('d'.repeat(64));
  await expect(page.locator('#secret-description')).toHaveAttribute('maxlength', '64');
  await page.locator('#secret-text').fill(content);
  await modal.locator('.tab-list button').nth(1).click();
  await expect(page.locator('#secret-text')).toBeDisabled();
  await expect(page.locator('#secret-file')).toBeEnabled();
  await modal.locator('.tab-list button').first().click();
  await expect(page.locator('#secret-text')).toHaveValue(content);
  await modal.locator('button[type=submit]').click();
  await expect(page).toHaveURL(/\/secrets$/);
  await expect(page.getByRole('row').filter({ hasText: textName })).toBeVisible();
  const textResponse = await request.get('/api/secrets?kind=Text', { headers });
  expect(textResponse.status()).toBe(200);
  const text = (await textResponse.json()).secrets.find(secret => secret.name === textName);
  expect(text.content).toBeUndefined();
  expect(await page.content()).not.toContain(content);
  await page.goto('/secrets?create=File');
  await expect(modal.locator('#secret-file')).toBeVisible();
  await page.locator('#secret-name').fill(fileName);
  await page.locator('#secret-file').setInputFiles({ name: 'fixture.bin', mimeType: 'application/octet-stream', buffer: Buffer.from([0, 1, 255, 10]) });
  await modal.locator('button[type=submit]').click();
  await expect(page).toHaveURL(/\/secrets$/);
  const fileResponse = await request.get('/api/secrets?kind=File', { headers });
  expect(fileResponse.status()).toBe(200);
  const file = (await fileResponse.json()).secrets.find(secret => secret.name === fileName);
  await page.goto('/groups/tests/apps/.create');
  await page.locator('#app-name').fill(name);
  await page.locator('#image').fill(environment.images.fixture.reference);
  await page.locator('#command_type').selectOption('custom');
  await page.locator('#container_cmd').fill('sleep 3600');
  const env = page.locator('[x-data="containerOptionsData"]');
  await env.locator('.end input').click();
  await page.locator('#env_key').fill('TOKEN');
  await modal.locator('.tab-list button').last().click();
  await expect(page.locator('#env-secret')).toBeEnabled();
  await expect(page.locator('#env-secret option')).not.toContainText([fileName]);
  await page.locator('#env-secret').selectOption(text.id);
  await modal.locator('input.button:not(.secondary):not(.danger)').click();
  await expect(modal).toHaveCount(0);
  await env.locator('tbody button').click();
  await expect(page.locator('#env-secret')).toHaveValue(text.id);
  await expect(page.locator('#env_value')).toBeDisabled();
  await modal.locator('input.secondary').click();
  const files = page.locator('[x-data="xconfigsData"]');
  await files.locator('.end input').click();
  await page.locator('#xconfig_target').fill('/fixture.bin');
  await modal.locator('.tab-list button').last().click();
  await expect(page.locator('#xconfig-secret')).toBeEnabled();
  await expect(page.locator('#xconfig-secret option')).not.toContainText([textName]);
  await page.locator('#xconfig-secret').selectOption(file.id);
  await modal.locator('input.button:not(.secondary):not(.danger)').click();
  await expect(modal).toHaveCount(0);
  await page.locator('form[x-data="appForm"] > .actions input[type=submit]').click();
  await expect(page).toHaveURL(new RegExp(`/apps/${name}$`));
  const appResponse = await request.get(`/api/apps/${name}`, { headers });
  expect(appResponse.status()).toBe(200);
  const app = await appResponse.json();
  expect(app.container_options.env).toEqual([{ name: 'TOKEN', value: { name: textName, id: text.id } }]);
  expect(app.xconfigs).toEqual([{ target: '/fixture.bin', secret: { name: fileName, id: file.id } }]);
  expect(app.tini_version).toBeUndefined();
  for (const [locale, label] of [['en', 'Secret'], ['zh-CN', '机密'], ['zh-TW', '機密'], ['ja', 'シークレット'], ['pt-BR', 'Segredo'], ['es', 'Secreto'], ['fr', 'Secret'], ['de', 'Geheimnis']]) {
    const context = await browser.newContext({ baseURL: environment.base_url, locale, storageState: await page.context().storageState() });
    try {
      const translated = await context.newPage();
      await translated.goto(`/apps/${name}`);
      await expect(translated.locator('body')).toContainText(`${label}: ${textName}`);
    } finally {
      await context.close();
    }
  }
  await page.goto(`/apps/${name}/spec`);
  await expect(env).toContainText(textName);
  await expect(files).toContainText(fileName);
  expect(await page.content()).not.toContain(content);
  await env.locator('tbody button').click();
  await modal.locator('.tab-list button').first().click();
  await page.locator('#env_value').fill('ordinary');
  await modal.locator('input.button:not(.secondary):not(.danger)').click();
  await files.locator('tbody button').click();
  await expect(page.locator('#xconfig-secret')).toHaveValue(file.id);
  await modal.locator('input.danger').click();
  await page.locator('form[x-data="appForm"] > .actions input[type=submit]').click();
  await expect(page).toHaveURL(new RegExp(`/apps/${name}$`));
  const cleared = await (await request.get(`/api/apps/${name}`, { headers })).json();
  expect(cleared.container_options.env).toEqual([{ name: 'TOKEN', value: 'ordinary' }]);
  expect(cleared.xconfigs).toEqual([]);
  expect((await request.patch(`/api/apps/${name}`, { headers, data: { container_options: app.container_options } })).status()).toBe(200);
  for (const [locale, label] of [['en', 'Secret'], ['zh-CN', '机密'], ['zh-TW', '機密'], ['ja', 'シークレット'], ['pt-BR', 'Segredo'], ['es', 'Secreto'], ['fr', 'Secret'], ['de', 'Geheimnis']]) {
    const context = await browser.newContext({ baseURL: environment.base_url, locale, storageState: await page.context().storageState() });
    try {
      const translated = await context.newPage();
      await translated.goto(`/apps/${name}`);
      await expect(translated.locator('ul.stack.bulleted')).toContainText(`${label}: ${textName}`);
    } finally {
      await context.close();
    }
  }
  const rollback = await request.post(`/api/apps/${name}/rollback`, { headers });
  expect(rollback.status()).toBe(200);
  expect((await rollback.json()).container_options.env).toEqual([{ name: 'TOKEN', value: 'ordinary' }]);
  await page.reload();
  await expect(page.locator('ul.stack.bulleted')).toContainText(`Secret: ${textName}`);
  expect((await request.delete(`/api/apps/${name}`, { headers })).status()).toBe(204);
  for (const secret of [textName, fileName]) {
    await expect(async () => {
      expect((await request.delete(`/api/secrets/${secret}`, { headers })).status()).toBe(204);
    }).toPass({ timeout: 120_000, intervals: [1000] });
  }
  expect(errors).toEqual([]);
});

test('Reader can inspect secrets but cannot open or submit write actions', async ({ browser, request }) => {
  const email = `secret-reader-${randomUUID()}@cdelivery.test`;
  const password = 'Reader-Test-Aa123456';
  const response = await request.post('/api/users', { headers, data: { first_name: 'Secret', last_name: 'Reader', email, password_sha256: createHash('sha256').update(password).digest('hex'), role: 'Reader' } });
  expect(response.status()).toBe(201);
  const user = await response.json();
  const context = await browser.newContext({ baseURL: environment.base_url, locale: 'en' });
  try {
    const page = await context.newPage();
    await page.goto('/login');
    await page.locator('#email').fill(email);
    await page.locator('#password').fill(password);
    await page.locator('#login_form input[type=submit]').click();
    await expect(page).not.toHaveURL(/\/login/);
    await page.goto('/secrets?create=Text');
    await expect(page.locator('h1')).toHaveText('Secrets');
    await expect(page.locator('dialog')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Create', exact: true })).toHaveCount(0);
    await expect(page.locator('form[action$="/delete"]')).toHaveCount(0);
    expect((await context.request.post('/api/secrets', { headers: { Origin: environment.base_url }, data: { name: 'reader-denied', content: { Text: 'denied' } } })).status()).toBe(403);
    expect((await context.request.delete('/api/secrets/reader-denied', { headers: { Origin: environment.base_url } })).status()).toBe(403);
  } finally {
    await context.close();
    expect((await request.delete(`/api/users/${user.id}`, { headers })).status()).toBe(204);
  }
});
