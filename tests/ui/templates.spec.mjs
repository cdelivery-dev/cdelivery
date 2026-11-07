import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { test, expect } from '@playwright/test';

const run = promisify(execFile);
const state = resolve(import.meta.dirname, '../../target/tests/current');
const credentials = JSON.parse(readFileSync(resolve(state, 'credentials.json'), 'utf8'));
const headers = { Authorization: `Bearer ${credentials.api_key}` };

for (const [id, title, image, target, passwordName, database] of [
  ['postgresql_18', 'PostgreSQL', 'postgres:18-alpine', '/var/lib/postgresql', 'POSTGRES_PASSWORD', 'postgres'],
  ['mariadb_12', 'MariaDB', 'mariadb:12.3-noble', '/var/lib/mysql', 'MARIADB_ROOT_PASSWORD', 'app'],
]) {
  test(`${title} template creates a database with persistent data`, async ({ page, request }) => {
    test.setTimeout(900_000);
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto('/login');
    await page.locator('#email').fill(credentials.email);
    await page.locator('#password').fill(credentials.password);
    await page.locator('#login_form input[type=submit]').click();
    await expect(page).not.toHaveURL(/\/login/);
    const imagesResponse = await request.get('/api/versions', { headers });
    expect(imagesResponse.status()).toBe(200);
    const images = (await imagesResponse.json()).templates;
    expect(images).toEqual({ postgresql_18: 'postgres:18', mariadb_12: 'mariadb:12.3' });
    const nodesResponse = await request.get('/api/nodes', { headers });
    expect(nodesResponse.status()).toBe(200);
    const node = (await nodesResponse.json()).nodes[0];
    const name = `db-${randomUUID().slice(0, 8)}`;
    const volumePath = `/api/nodes/${node.id}/volumes/${name}`;
    const appPath = `/api/apps/${name}`;
    expect((await request.post(`/api/nodes/${node.id}/volumes`, { headers, data: { name } })).status()).toBe(201);
    const docker = async args => (await run('orb', ['-m', node.hostname, '-u', 'root', '/usr/bin/docker', ...args], { timeout: 180_000 })).stdout.trim();
    const container = () => docker(['ps', '--filter', `label=com.docker.swarm.service.name=${name}`, '--format', '{{.ID}}']);
    const response = await page.goto(`/groups/tests/apps/.template/${id}`);
    expect(response.status()).toBe(200);
    await expect(page).toHaveTitle(new RegExp(title));
    await expect(page.locator('a[href$="/.template/mysql"]')).toHaveCount(0);
    await expect(page.locator('main header nav a[translate=no]')).toHaveText(['PostgreSQL', 'MariaDB']);
    await expect(page.locator('#image')).toHaveValue(images[id]);
    const initial = JSON.parse(await page.locator('#app-form-init').textContent()).request;
    const password = initial.container_options.env.find(variable => variable.name === passwordName).value;
    expect(password).toMatch(/^[A-Za-z0-9_-]{20}$/);
    expect(initial.container_options.env.some(variable => variable.name === 'PGDATA')).toBe(false);
    await page.locator('#app-name').fill(name);
    await page.locator('#image').fill(image);
    await page.locator('#mounts-node').selectOption(node.id);
    const mounts = page.locator('[x-data^="mountsData("]');
    await mounts.locator('.end button').click();
    await expect(page.locator('#mount-target')).toHaveValue(target);
    await page.locator('#mount-volume').selectOption(name);
    await mounts.locator('dialog input.button:not(.secondary):not(.danger)').click();
    await page.locator('form[x-data="appForm"] > .actions input[type=submit]').click();
    await expect(page).toHaveURL(new RegExp(`/apps/${name}$`));
    const savedResponse = await request.get(appPath, { headers });
    expect(savedResponse.status()).toBe(200);
    const saved = await savedResponse.json();
    expect(saved.image).toBe(`docker.io/library/${image}`);
    await expect(page.getByText(saved.image, { exact: true })).toBeVisible();
    await page.goto(`/apps/${name}/spec`);
    await expect(page.locator('#image')).toHaveValue(saved.image);
    expect(saved.mounts.mounts[0].target).toBe(target);
    const query = async (containerId, sql) => id === 'postgresql_18'
      ? docker(['exec', '--env', `PGPASSWORD=${password}`, containerId, 'psql', '-h', '127.0.0.1', '-U', 'postgres', '-d', database, '-At', '-v', 'ON_ERROR_STOP=1', '-c', sql])
      : docker(['exec', '--env', `MYSQL_PWD=${password}`, containerId, 'mariadb', '-h', '127.0.0.1', '-u', 'root', '-N', '-B', database, '-e', sql]);
    let before;
    await expect(async () => {
      before = await container();
      expect(before).toMatch(/^[0-9a-f]+$/);
      expect(await query(before, 'SELECT 1')).toBe('1');
    }).toPass({ timeout: 360_000, intervals: [2000] });
    const sentinel = randomUUID();
    await query(before, `CREATE TABLE template_check (value varchar(36)); INSERT INTO template_check VALUES ('${sentinel}');`);
    expect((await request.post(`${appPath}/restart`, { headers })).status()).toBe(204);
    await expect(async () => {
      const after = await container();
      expect(after).toMatch(/^[0-9a-f]+$/);
      expect(after).not.toBe(before);
      expect(await query(after, 'SELECT value FROM template_check')).toBe(sentinel);
    }).toPass({ timeout: 360_000, intervals: [2000] });
    expect((await request.delete(appPath, { headers })).status()).toBe(204);
    await expect(async () => {
      expect((await request.delete(volumePath, { headers })).status()).toBe(200);
    }).toPass({ timeout: 180_000, intervals: [2000] });
    expect(errors).toEqual([]);
  });
}
