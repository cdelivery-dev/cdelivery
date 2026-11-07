import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { test, expect } from '@playwright/test';

const state = resolve(import.meta.dirname, '../../target/tests/current');
const credentials = JSON.parse(readFileSync(resolve(state, 'credentials.json'), 'utf8'));
const headers = { Authorization: `Bearer ${credentials.api_key}` };

for (const locale of ['en', 'zh-CN']) {
  test.describe(locale, () => {
    test.use({ locale });
    test('Pingress defaults to ACME and supports explicit opt-out', async ({ page, request }) => {
      const response = await request.get('/api/pingress', { headers });
      expect(response.status()).toBe(200);
      const baseline = await response.json();
      expect(baseline.acme_enabled).toBe(false);
      const certificates = await request.get('/api/pingress/certs', { headers });
      expect((await certificates.json()).certs).toEqual([]);
      const duplicate = await request.put('/api/pingress', { headers, data: { version: baseline.version, placement: baseline.placement, acme: null } });
      expect(duplicate.status(), 'Deploy the current source before running this test').toBe(409);
      expect((await request.delete('/api/pingress', { headers })).status()).toBe(204);
      try {
        expect((await request.patch('/api/me/preferences', { headers, data: { language: null } })).status()).toBe(204);
        await page.goto('/login');
        await page.locator('#email').fill(credentials.email);
        await page.locator('#password').fill(credentials.password);
        await page.locator('#login_form input[type=submit]').click();
        await expect(page).not.toHaveURL(/\/login/);
        await page.goto('/pingress');
        await expect(page.locator('#pingress-acme')).toBeChecked();
        const nodes = await request.get('/api/nodes', { headers });
        const node = (await nodes.json()).nodes.find(node => node.role === 'Manager');
        await page.locator('#pingress-node').selectOption(node.id);
        let selected;
        await page.route('**/pingress/enable', async route => {
          selected = JSON.parse(new URLSearchParams(route.request().postData()).get('payload'));
          if (selected.acme === null) await route.continue();
          else await route.fulfill({ status: 303, headers: { location: '/pingress' } });
        });
        await page.locator('form[action="/pingress/enable"] input[type=submit]').click();
        await expect.poll(() => selected?.acme).toBe('LetsEncrypt');
        await expect(page.locator('#pingress-node')).toHaveValue('');
        await expect(page.locator('#pingress-acme')).toBeChecked();
        await page.locator('#pingress-node').selectOption(node.id);
        await page.locator('#pingress-acme').uncheck();
        await page.locator('form[action="/pingress/enable"] input[type=submit]').click();
        await expect(page.locator('form[action="/pingress/update"]')).toBeVisible();
        await expect(page.locator('#pingress-acme')).toHaveCount(0);
        const deployed = await request.get('/api/pingress', { headers });
        expect((await deployed.json()).acme_enabled).toBe(false);
      } finally {
        const current = await request.get('/api/pingress', { headers });
        expect([200, 404]).toContain(current.status());
        const data = { version: baseline.version, placement: baseline.placement };
        const restored = current.status() === 404
          ? await request.put('/api/pingress', { headers, data: { ...data, acme: null } })
          : await request.patch('/api/pingress', { headers, data });
        expect(restored.ok()).toBe(true);
      }
    });
  });
}
