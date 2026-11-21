import { createHash, randomUUID } from 'node:crypto';
import { test, expect } from '@playwright/test';
import { headers, login, readEnvironment } from './support.mjs';

const environment = readEnvironment();

async function resetLanguage({ request }) {
  const response = await request.patch('/api/me/preferences', {
    headers,
    data: { language: null },
  });
  expect(response.status()).toBe(204);
}

test.beforeEach(resetLanguage);
test.afterEach(resetLanguage);

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
    for (const path of ['/groups', '/nodes', '/pingress', '/insight', '/volume', '/dependencies', '/account', '/account/apikeys', '/settings/cdelivery', '/setup/platform']) {
      const response = await page.goto(path);
      expect(response.status()).toBe(200);
      await expect(page.locator('html')).toHaveAttribute('lang', tag);
      await expect(page.locator('main')).not.toContainText(/ui\.[a-z]+\./);
    }
  }
  await page.goto('/account/preferences');
  await page.locator('#account-language').selectOption('');
  await page.locator('form[action="/account/preferences"] input[type=submit]').click();
});

test('server volume settings, counts and navigation', async ({ page, request, browser }) => {
  test.setTimeout(300_000);
  const settings = await (await request.get('/api/volume', { headers })).json();
  const version = environment.images['cdelivery-agent'].version;
  const { nodes } = await (await request.get('/api/nodes', { headers })).json();
  const node = nodes[0];
  const path = `/api/nodes/${node.id}/volumes`;
  const name = `ui-${randomUUID().slice(0, 8)}`;
  const errors = [];
  let created = false;
  let reader;
  page.on('pageerror', error => errors.push(error.message));
  const count = page.locator(`a[data-node="${node.id}"]`);
  const management = page.locator('#volume-management');
  const join = page.locator('[x-data="nodeJoin"]');
  const ready = () => expect.poll(async () => (await request.get(path, { headers })).status(), { timeout: 60_000 }).toBe(200);
  async function checkJoin(zfs) {
    await join.locator('summary').click();
    for (const role of ['Worker', 'Manager']) {
      await join.locator('select').selectOption(role);
      if (zfs) await expect(join.locator('code')).toContainText(' --zfs');
      else await expect(join.locator('code')).not.toContainText(' --zfs');
    }
    await join.locator('summary').click();
  }
  try {
    await login(page);
    await page.goto('/volume');
    await expect(page).toHaveURL(/\/nodes#volume-management$/);
    await expect(management).not.toHaveAttribute('open', '');
    await expect(page.locator('a[href="/volume"]')).toHaveCount(0);
    expect(await management.evaluate(element => Boolean(element.compareDocumentPosition(document.querySelector('[x-data="nodeJoin"]')) & Node.DOCUMENT_POSITION_FOLLOWING))).toBe(true);
    if (settings.image) {
      await ready();
      await checkJoin(settings.zfs_enabled);
      await management.locator('summary').click();
      page.once('dialog', dialog => dialog.accept());
      await page.locator('input[form="volume-disable-form"]').click();
    }
    await expect(management.locator('summary')).toContainText('(Disabled)');
    const queries = [];
    const observe = req => { if (req.url().includes('/volumes?sizes=false')) queries.push(req.url()); };
    page.on('request', observe);
    await page.reload();
    await expect(page.locator('[x-data="nodeVolumes"]')).toHaveCount(0);
    await checkJoin(false);
    expect(queries).toEqual([]);
    page.off('request', observe);
    expect((await page.goto(`/nodes/${node.id}/volumes`)).status()).toBe(200);
    await expect(page.locator('main')).toContainText('Volume management is disabled.');
    await expect(page.locator('.tab-list')).toBeVisible();
    await page.getByRole('link', { name: 'Go to settings' }).click();
    await management.locator('summary').click();
    await management.locator('input[type=checkbox]').uncheck();
    await expect(management.locator('pre')).toBeHidden();
    await page.locator('#volume-enable-form input[type=submit]').click();
    await expect(page).toHaveURL(/\/nodes#volume-management$/);
    await expect(management.locator('summary')).toContainText('(Enabled / Local volumes)');
    await ready();
    await page.reload();
    await checkJoin(false);
    const before = await (await request.get(path, { headers })).json();
    await expect(count).toHaveText(String(before.volumes.length));
    expect((await request.post(path, { headers, data: { name } })).status()).toBe(201);
    created = true;
    await page.reload();
    await expect(count).toHaveText(String(before.volumes.length + 1));
    await count.click();
    await expect(page).toHaveURL(new RegExp(`/nodes/${node.id}/volumes$`));
    await expect(page.locator('tbody')).toContainText(name);
    await page.route(`**/api/nodes/${node.id}/volumes?sizes=false`, route => route.abort());
    await page.goto('/nodes');
    await expect(count).toHaveText('Unavailable');
    await expect(page.locator('table.nodes')).toBeVisible();
    await page.unroute(`**/api/nodes/${node.id}/volumes?sizes=false`);
    await page.reload();
    await expect(count).toHaveText(String(before.volumes.length + 1));

    const email = `volume-reader-${randomUUID().slice(0, 8)}@cdelivery.test`;
    const password = 'Volume-Reader-123456';
    const response = await request.post('/api/users', { headers, data: {
      first_name: 'Volume', last_name: 'Reader', email, role: 'Reader',
      password_sha256: createHash('sha256').update(password).digest('hex'),
    } });
    expect(response.status()).toBe(201);
    reader = await response.json();
    const context = await browser.newContext({ baseURL: environment.base_url });
    try {
      const readonly = await context.newPage();
      await login(readonly, email, password);
      await readonly.goto('/nodes');
      await expect(readonly.locator(`a[data-node="${node.id}"]`)).toHaveText(String(before.volumes.length + 1));
      await expect(readonly.locator('#volume-management input[type=submit], [x-data="nodeJoin"]')).toHaveCount(0);
      await readonly.goto(`/nodes/${node.id}/volumes`);
      await expect(readonly.locator('[x-ref="volumeModal"]')).toHaveCount(0);
    } finally {
      await context.close();
    }
    await page.goto('/nodes');
    await management.locator('summary').click();
    page.once('dialog', dialog => dialog.accept());
    await page.locator('input[form="volume-disable-form"]').click();
    await management.locator('summary').click();
    await management.locator('input[type=checkbox]').setChecked(settings.zfs_enabled === true);
    await page.locator('#volume-enable-form input[type=submit]').click();
    await ready();
    await page.reload();
    await checkJoin(settings.zfs_enabled === true);
    await count.click();
    await expect(page.locator('tbody')).toContainText(name);
    expect(errors).toEqual([]);
  } finally {
    if (!(await (await request.get('/api/volume', { headers })).json()).image) {
      expect((await request.put('/api/volume', { headers, data: { version, zfs_enabled: false } })).status()).toBe(200);
    }
    await ready();
    if (created) expect((await request.delete(`${path}/${name}`, { headers })).status()).toBe(200);
    if (reader) expect((await request.delete(`/api/users/${reader.id}`, { headers })).status()).toBe(204);
    expect((await request.delete('/api/volume', { headers })).status()).toBe(200);
    if (settings.image) {
      expect((await request.put('/api/volume', { headers, data: { version, zfs_enabled: settings.zfs_enabled } })).status()).toBe(200);
      await ready();
    }
  }
});

for (const locale of ['en', 'zh-CN']) {
  test.describe(locale, () => {
    test.use({ locale });
    test('account forms, key lifecycle and password invalidation', async ({ page, request }) => {
      const email = `ui-account-${randomUUID().slice(0, 8)}@cdelivery.test`;
      const password = 'Browser-Aa123456';
      const response = await request.post('/api/users', { headers, data: {
        first_name: 'Browser', last_name: 'Account', email, role: 'Operator',
        password_sha256: createHash('sha256').update(password).digest('hex'),
      } });
      expect(response.status()).toBe(201);
      const user = await response.json();
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      try {
        await login(page, email, password);
        await page.goto('/account');
        await page.locator('#account-first-name').fill('');
        await page.locator('form[action="/account"] input[type=submit]').click();
        await expect(page.locator('#account-first-name')).toBeFocused();
        await page.locator('#account-first-name').fill('Updated');
        await page.locator('form[action="/account"] input[type=submit]').click();
        await page.reload();
        await expect(page.locator('#account-first-name')).toHaveValue('Updated');
        await page.goto('/account/preferences');
        await page.locator('#account-theme').selectOption('Dark');
        await page.locator('form[action="/account/preferences"] input[type=submit]').click();
        await page.reload();
        await expect(page.locator('#account-theme')).toHaveValue('Dark');
        await page.goto('/account/apikeys');
        expect(await page.locator('#account-apikey-role option').evaluateAll(options => options.map(option => option.value))).toEqual(['Reader', 'Operator']);
        await page.locator('#account-apikey-name').fill('browser-key');
        await page.locator('form[action="/account/apikeys"] input[type=submit]').click();
        const key = (await page.locator('pre[x-ref=key]').textContent()).trim();
        expect((await request.get('/api/me', { headers: { Authorization: `Bearer ${key}` } })).status()).toBe(200);
        await page.goto('/account/apikeys');
        await expect(page.locator('main')).not.toContainText(key);
        const row = page.locator('tr').filter({ hasText: 'browser-key' });
        page.once('dialog', dialog => dialog.dismiss());
        await row.locator('input[type=submit]').click();
        await expect(row).toHaveCount(1);
        expect((await request.get('/api/me', { headers: { Authorization: `Bearer ${key}` } })).status()).toBe(200);
        page.once('dialog', dialog => dialog.accept());
        await row.locator('input[type=submit]').click();
        await expect(row).toHaveCount(0);
        expect((await request.get('/api/me', { headers: { Authorization: `Bearer ${key}` } })).status()).toBe(401);
        const forbidden = await page.goto('/settings/cdelivery');
        expect(forbidden.status()).toBe(403);
        await page.goto('/pingress');
        await expect(page.locator('#pingress-version')).toHaveCount(0);
        await page.goto('/account/password');
        await page.locator('#account-current-password').fill(password);
        await page.locator('#account-new-password').fill('Changed-Aa123456');
        await page.locator('#account-confirm-password').fill('Mismatch-Aa123456');
        await page.locator('form[action="/account/password"] input[type=submit]').click();
        expect(await page.locator('#account-confirm-password').evaluate(input => input.validity.customError)).toBe(true);
        await page.locator('#account-confirm-password').fill('Changed-Aa123456');
        await page.locator('form[action="/account/password"] input[type=submit]').click();
        await expect(page).toHaveURL(/\/login/);
        await login(page, email, 'Changed-Aa123456');
        expect(errors).toEqual([]);
      } finally {
        expect((await request.delete(`/api/users/${user.id}`, { headers })).status()).toBe(204);
      }
    });
    test('XHealth runs the saved configuration and refreshes manually', async ({ page, request }) => {
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      const name = `ui-hc-${randomUUID().slice(0, 8)}`;
      const path = `/api/apps/${name}`;
      const health = `${path}/xhealth`;
      const created = await request.post('/api/apps', {
        headers,
        data: {
          name, group: 'tests', image: environment.images.fixture.reference,
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
      const created = await request.post('/api/apps', {
        headers,
        data: {
          name, group: 'tests', image: environment.images.fixture.reference,
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
        await expect(page.locator('#xjob-interval-unit')).toHaveValue('Minutes');
        await expect(page.locator('#xjob-timeout')).toHaveValue('30');
        await expect(page.locator('#xjob-timeout-unit')).toHaveValue('Minutes');
        await page.locator('#xjob-command').fill('true');
        await page.locator('#xjob-timeout').fill('');
        await page.locator('form[action$="/xjobs"] input[type=submit]').click();
        await expect(page.locator('#xjob-timeout')).toBeFocused();
        expect(await page.locator('#xjob-timeout').evaluate(input => input.validity.valueMissing)).toBe(true);
        await page.locator('#xjob-timeout').fill('60');
        await page.locator('form[action$="/xjobs"] input[type=submit]').click();
        await page.goto(`/apps/${name}/xjobs/backup`);
        await expect(page.locator('#xjob-timeout')).toHaveValue('1');
        await expect(page.locator('#xjob-timeout-unit')).toHaveValue('Hours');
        await expect(page.locator('#xjob-interval')).toHaveValue('30');
        await page.locator('#xjob-interval').fill('45');
        await page.locator('form[action$="/backup/update"] input[type=submit]').click();
        await page.reload();
        await expect(page.locator('#xjob-interval')).toHaveValue('45');
        await expect(page.locator('#xjob-timeout')).toHaveValue('1');
        await expect(page.locator('#xjob-timeout-unit')).toHaveValue('Hours');
        expect(errors).toEqual([]);
      } finally {
        const deleted = await request.delete(`/api/apps/${name}`, { headers });
        expect(deleted.status()).toBe(204);
      }
    });
    test('platform domain defaults and explicit HTTP persist', async ({ page, request }) => {
      const current = await request.get('/api/instance', { headers });
      expect(current.status()).toBe(200);
      const saved = (await current.json()).domain;
      const domain = `ui-${randomUUID().slice(0, 8)}.cdelivery.test`;
      try {
        expect((await request.patch('/api/instance', { headers, data: { domain: null } })).status()).toBe(202);
        await login(page);
        await page.goto('/settings/cdelivery');
        const form = page.locator('form[action="/settings/cdelivery"]');
        const https = form.locator('input[type=checkbox]');
        await expect(https).toBeChecked();
        await expect(form.locator('input[type=submit]')).toBeDisabled();
        const created = await request.patch('/api/instance', { headers, data: { domain: { domain } } });
        expect(created.status()).toBe(202);
        const configured = await request.get('/api/instance', { headers });
        expect(configured.status()).toBe(200);
        expect((await configured.json()).domain.https).toBe(true);
        await page.reload();
        await expect(https).toBeChecked();
        await https.uncheck();
        await form.locator('input[type=submit]').click();
        await page.reload();
        await expect(https).not.toBeChecked();
        const updated = await request.get('/api/instance', { headers });
        expect(updated.status()).toBe(200);
        expect((await updated.json()).domain).toEqual({ domain, https: false });
      } finally {
        expect((await request.patch('/api/instance', { headers, data: { domain: saved } })).status()).toBe(202);
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
      await page.goto(`/groups/${name}/apps/.template/postgresql_18`);
      await page.locator('form[x-data="appForm"] > details > summary').click();
      await expect(page.locator('#log-driver-type-select')).toHaveValue('Insight');
      await page.goto(`/groups/${name}/apps/.create`);
      await expect(page.locator('#image')).toHaveValue('');
      await page.locator('form[x-data="appForm"] > details > summary').click();
      await expect(page.locator('#log-driver-type-select')).toHaveValue('Insight');
      await page.locator('#log-driver-type-select').selectOption('FollowHost');
      const published = page.locator('[x-data="publishedData"]');
      await published.locator('.end input[type=button]').click();
      const domainDialog = published.locator('dialog');
      await expect(domainDialog.locator('input[type=checkbox]')).toBeChecked();
      await domainDialog.locator('input[type=checkbox]').uncheck();
      await page.locator('#domain_name').fill(`${name}.cdelivery.test`);
      await page.locator('#domain_target').fill('8080');
      await domainDialog.locator('input.button:not(.secondary):not(.danger)').click();
      await page.locator('#app-name').fill(name);
      await page.locator('#image').fill(environment.images.fixture.reference);
      await page.locator('#command_type').selectOption('custom');
      await page.locator('#container_cmd').fill('sleep 86400');
      await page.locator('form[action="/apps"] input[type=submit]').click();
      await expect(page).toHaveURL(new RegExp(`/apps/${name}`));
      await expect(page.getByText(environment.images.fixture.reference, { exact: true })).toBeVisible();
      for (const suffix of ['', '/spec', '/logs', '/xhealth', '/xjobs']) {
        const response = await page.goto(`/apps/${name}${suffix}`);
        expect(response.status()).toBe(200);
        await expect(page.locator('main')).not.toContainText('ui.app.');
      }
      await page.goto(`/apps/${name}/spec`);
      await expect(page.locator('#image')).toHaveValue(environment.images.fixture.reference);
      await page.locator('form[x-data="appForm"] > details > summary').click();
      await expect(page.locator('#log-driver-type-select')).toHaveValue('FollowHost');
      await published.locator('tbody button').click();
      await expect(domainDialog.locator('input[type=checkbox]')).not.toBeChecked();
      await domainDialog.locator('input.secondary').click();
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
