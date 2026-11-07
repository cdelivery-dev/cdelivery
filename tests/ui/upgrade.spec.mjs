import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { test, expect } from '@playwright/test';

const run = promisify(execFile);
const root = resolve(import.meta.dirname, '../..');
const state = resolve(root, 'target/tests/current');
const credentials = JSON.parse(readFileSync(resolve(state, 'credentials.json'), 'utf8'));

for (const locale of ['en', 'zh-CN']) {
  test.describe(locale, () => {
    test.use({ locale });
    test('version selection, cancellation and real controller upgrade', async ({ page, request }, info) => {
      test.setTimeout(1_200_000);
      const command = async action => {
        const result = await run(resolve(root, 'target/release/xtask'), ['env', 'upgrade', action], { cwd: root, timeout: 900_000, maxBuffer: 16 * 1024 * 1024 });
        await info.attach(`upgrade-${action}`, { body: result.stdout + result.stderr, contentType: 'text/plain' });
      };
      const headers = { Authorization: `Bearer ${credentials.api_key}` };
      try {
        await command('prepare');
        const environment = JSON.parse(readFileSync(resolve(state, 'env.json'), 'utf8'));
        const target = environment.upgrades.cdelivery;
        const docker = async args => (await run('orb', ['-m', environment.machines[0], '-u', 'root', '/usr/bin/docker', ...args], { timeout: 30_000 })).stdout;
        const tasks = () => docker(['service', 'ps', '--filter', 'desired-state=running', '--format', '{{.ID}}', 'cdelivery']);
        const before = await tasks();
        expect((await request.patch('/api/me/preferences', { headers, data: { language: null } })).status()).toBe(204);
        await page.goto('/login');
        await page.locator('#email').fill(credentials.email);
        await page.locator('#password').fill(credentials.password);
        await page.locator('#login_form input[type=submit]').click();
        await expect(page).not.toHaveURL(/\/login/);
        await page.goto('/settings/cdelivery');
        const form = page.locator('form').filter({ has: page.locator('#version-release') });
        const submit = form.locator('input[type=submit]');
        await expect(submit).toBeDisabled();
        await page.locator('#version-release').selectOption(target.version);
        await expect(submit).toBeEnabled();
        await expect(submit).toHaveValue(locale === 'en' ? 'Save' : '保存');
        page.once('dialog', dialog => dialog.dismiss());
        await submit.click();
        expect(await tasks()).toBe(before);
        await expect(submit).toBeEnabled();
        page.once('dialog', dialog => dialog.accept());
        const [response] = await Promise.all([
          page.waitForResponse(response => response.request().method() === 'POST' && response.url().endsWith('/settings/cdelivery')),
          submit.click(),
        ]);
        expect(response.status()).toBe(303);
        expect(JSON.parse(new URLSearchParams(response.request().postData()).get('payload'))).toEqual({ version: target.version });
        await expect.poll(async () => {
          const service = JSON.parse(await docker(['service', 'inspect', 'cdelivery']))[0];
          return service.UpdateStatus?.State === 'completed' && service.Spec.TaskTemplate.ContainerSpec.Image.split('@')[0] === target.reference;
        }, { timeout: 360_000, intervals: [2000] }).toBe(true);
        expect(await tasks()).not.toBe(before);
        const id = (await docker(['ps', '--filter', 'label=com.docker.swarm.service.name=cdelivery', '--format', '{{.ID}}'])).trim();
        const image = (await docker(['inspect', '--format', '{{.Image}}', id])).trim();
        const digests = JSON.parse(await docker(['image', 'inspect', '--format', '{{json .RepoDigests}}', image]));
        expect(digests.some(digest => digest.split('@')[1] === target.digest.split('@')[1])).toBe(true);
        const current = await request.get('/api/instance', { headers });
        expect(current.status()).toBe(200);
        expect((await current.json()).version).toBe(target.version);
        await page.goto('/settings/cdelivery');
        await expect(page).not.toHaveURL(/\/login/);
        await expect(page.locator('#version-release')).toBeEnabled();
        await expect(page.locator('#version-release option')).toHaveText(target.version);
        await expect(submit).toBeDisabled();
      } finally {
        await command('restore');
      }
    });
  });
}
