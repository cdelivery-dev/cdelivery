import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { test, expect } from '@playwright/test';

const state = resolve(import.meta.dirname, '../../target/tests/current');
const credentials = JSON.parse(readFileSync(resolve(state, 'credentials.json'), 'utf8'));
const environment = JSON.parse(readFileSync(resolve(state, 'env.json'), 'utf8'));

async function resetLanguage({ request }) {
  const response = await request.patch('/api/me/preferences', {
    headers: { Authorization: `Bearer ${credentials.api_key}` },
    data: { language: null },
  });
  expect(response.status()).toBe(204);
}

test.beforeEach(resetLanguage);
test.afterEach(resetLanguage);

async function login(page) {
  await page.goto('/login');
  await page.locator('#email').fill(credentials.email);
  await page.locator('#password').fill(credentials.password);
  await page.locator('#login_form input[type=submit]').click();
  await expect(page).not.toHaveURL(/\/login/);
}

test('native validation prevents incomplete login submission', async ({ page }) => {
  await page.goto('/login');
  let submitted = false;
  page.on('request', request => { if (request.method() === 'POST') submitted = true; });
  await page.locator('#login_form input[type=submit]').click();
  await expect(page.locator('#email')).toBeFocused();
  expect(await page.locator('#email').evaluate(input => input.validity.valueMissing)).toBe(true);
  expect(submitted).toBe(false);
});

test('all eight account languages persist across navigation', async ({ page }) => {
  await login(page);
  for (const [value, tag] of [['En', 'en'], ['ZhCn', 'zh-CN'], ['ZhTw', 'zh-TW'], ['Ja', 'ja'], ['PtBr', 'pt-BR'], ['Es', 'es'], ['Fr', 'fr'], ['De', 'de']]) {
    await page.goto('/account/preferences');
    await page.locator('#account-language').selectOption(value);
    await page.locator('form[action="/account/preferences"] input[type=submit]').click();
    await expect(page.locator('html')).toHaveAttribute('lang', tag);
    await page.goto('/groups');
    await expect(page.locator('html')).toHaveAttribute('lang', tag);
    await expect(page.locator('h1')).not.toContainText('ui.');
  }
  await page.goto('/account/preferences');
  await page.locator('#account-language').selectOption('');
  await page.locator('form[action="/account/preferences"] input[type=submit]').click();
});

for (const locale of ['en', 'zh-CN']) {
  test.describe(locale, () => {
    test.use({ locale });
    test('XHealth runs the saved configuration and refreshes manually', async ({ page, request }) => {
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      const name = `ui-hc-${randomUUID().slice(0, 8)}`;
      const headers = { Authorization: `Bearer ${credentials.api_key}` };
      const path = `/api/apps/${name}`;
      const health = `${path}/xhealth`;
      const created = await request.post('/api/apps', {
        headers,
        data: {
          name, group: 'tests', image: environment.images.fixture,
          replicas: { Fixed: 1 }, container_options: { command: ['sleep', '86400'] },
        },
      });
      expect(created.status()).toBe(201);
      let readerId;
      try {
        await login(page);
        await page.goto(`/apps/${name}/xhealth`);
        const button = page.locator('form[action$="/xhealth/run"] input[type=submit]');
        await expect(button).toHaveCount(0);
        const enabled = await request.put(health, {
          headers,
          data: {
            method: { Custom: { command: ['sleep', '15'] } },
            interval: { unit: 'Hours', value: 24 },
            target: { InternalPort: { port: 8080 } },
          },
        });
        expect(enabled.status()).toBe(201);
        await page.reload();
        await expect(button).toBeDisabled();
        const passed = async () => {
          const response = await request.get(health, { headers });
          expect(response.status()).toBe(200);
          return (await response.json()).status.Passed?.finished_at ?? null;
        };
        await expect.poll(passed, { timeout: 60_000 }).not.toBeNull();
        await expect(button).toBeDisabled();
        await page.reload();
        await expect(button).toBeEnabled();
        await expect(button).toHaveValue(locale === 'en' ? 'Run now' : '立即执行');
        await page.locator('#custom-command').fill('');
        const previous = await passed();
        const [submitted] = await Promise.all([
          page.waitForResponse(response => response.request().method() === 'POST' && response.url().endsWith(`/apps/${name}/xhealth/run`)),
          button.click(),
        ]);
        expect(submitted.status()).toBe(303);
        expect(submitted.request().postData()).toBe('payload=%7B%7D');
        await expect(page).toHaveURL(new RegExp(`/apps/${name}/xhealth$`));
        await expect(button).toBeDisabled();
        await expect(page.locator('#custom-command')).toHaveValue('sleep 15');
        await expect.poll(async () => {
          const current = await passed();
          return current !== null && current !== previous;
        }, { timeout: 60_000 }).toBe(true);
        await expect(button).toBeDisabled();
        await page.reload();
        await expect(button).toBeEnabled();
        const email = `${name}@cdelivery.test`;
        const password = 'Test-Password-Aa123';
        const reader = await request.post('/api/users', {
          headers,
          data: {
            first_name: 'Health', last_name: 'Reader', email, role: 'Reader',
            password_sha256: createHash('sha256').update(password).digest('hex'),
          },
        });
        expect(reader.status()).toBe(201);
        readerId = (await reader.json()).id;
        await page.context().clearCookies();
        await page.goto('/login');
        await page.locator('#email').fill(email);
        await page.locator('#password').fill(password);
        await page.locator('#login_form input[type=submit]').click();
        await expect(page).not.toHaveURL(/\/login/);
        await page.goto(`/apps/${name}/xhealth`);
        await expect(button).toHaveCount(0);
        expect(errors).toEqual([]);
      } finally {
        const deleted = await request.delete(path, { headers });
        expect(deleted.status()).toBe(204);
        if (readerId) {
          const deletedReader = await request.delete(`/api/users/${readerId}`, { headers });
          expect(deletedReader.status()).toBe(204);
        }
      }
    });
    test('XJob interval and timeout form', async ({ page, request }) => {
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      const name = `ui-job-${randomUUID().slice(0, 8)}`;
      const headers = { Authorization: `Bearer ${credentials.api_key}` };
      const created = await request.post('/api/apps', {
        headers,
        data: {
          name, group: 'tests', image: environment.images.fixture,
          replicas: { Fixed: 1 },
          container_options: { command: ['sleep', '86400'] },
        },
      });
      expect(created.status()).toBe(201);
      try {
        await login(page);
        await page.goto(`/apps/${name}/xjobs/.create`);
        await page.locator('#xjob-name').fill('backup');
        await expect(page.locator('#xjob-interval')).toHaveValue('30');
        await expect(page.locator('#xjob-timeout')).toHaveValue('');
        await page.locator('#xjob-command').fill('true');
        await page.locator('form[action$="/xjobs"] input[type=submit]').click();
        await expect(page.locator('#xjob-timeout')).toBeFocused();
        expect(await page.locator('#xjob-timeout').evaluate(input => input.validity.valueMissing)).toBe(true);
        await page.locator('#xjob-timeout').fill('60');
        await page.locator('form[action$="/xjobs"] input[type=submit]').click();
        await page.goto(`/apps/${name}/xjobs/backup`);
        await expect(page.locator('#xjob-timeout')).toHaveValue('60');
        await expect(page.locator('#xjob-interval')).toHaveValue('30');
        await page.locator('#xjob-interval').fill('45');
        await page.locator('form[action$="/backup/update"] input[type=submit]').click();
        await page.reload();
        await expect(page.locator('#xjob-interval')).toHaveValue('45');
        await expect(page.locator('#xjob-timeout')).toHaveValue('60');
        expect(errors).toEqual([]);
      } finally {
        const deleted = await request.delete(`/api/apps/${name}`, { headers });
        expect(deleted.status()).toBe(204);
      }
    });
    test('group and application lifecycle', async ({ page }) => {
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await login(page);
      const name = `ui-${randomUUID().slice(0, 8)}`;
      await page.goto('/groups/.create');
      await page.locator('#group-name').fill(name);
      await page.locator('#group-description').fill('Browser acceptance');
      await page.locator('#create_group_form input[type=submit]').click();
      await expect(page).toHaveURL(/\/groups$/);
      await page.goto(`/groups/${name}`);
      await expect(page.locator('#group-description')).toHaveValue('Browser acceptance');
      await page.locator('#group-description').fill('Updated');
      await page.locator('#edit_group_form input[type=submit]').click();
      await page.goto(`/groups/${name}`);
      await expect(page.locator('#group-description')).toHaveValue('Updated');
      await page.goto(`/groups/${name}/apps/.create`);
      await page.locator('#app-name').fill(name);
      await page.locator('#image').fill(environment.images.fixture);
      await page.locator('#command_type').selectOption('custom');
      await page.locator('#container_cmd').fill('sleep 86400');
      await page.locator('form[action="/apps"] input[type=submit]').click();
      await expect(page).toHaveURL(new RegExp(`/apps/${name}`));
      for (const suffix of ['', '/spec', '/logs', '/xhealth', '/xjobs']) {
        const response = await page.goto(`/apps/${name}${suffix}`);
        expect(response.status()).toBe(200);
        await expect(page.locator('main')).not.toContainText('ui.app.');
      }
      await page.goto(`/apps/${name}/spec`);
      page.once('dialog', dialog => dialog.accept());
      await page.locator('button[form="delete_app_form"]').click();
      await expect(page).not.toHaveURL(new RegExp(`/apps/${name}`));
      await page.goto(`/groups/${name}`);
      page.once('dialog', dialog => dialog.accept());
      const [deleted] = await Promise.all([
        page.waitForResponse(response => response.request().method() === 'POST' && response.url().endsWith(`/groups/${name}/delete`)),
        page.locator('button[form="delete_group_form"]').click(),
      ]);
      expect(deleted.status()).toBe(303);
      await expect(page).toHaveURL(/\/groups$/);
      expect(errors).toEqual([]);
    });
  });
}
