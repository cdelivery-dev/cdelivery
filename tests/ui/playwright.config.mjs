import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { resolve } from 'node:path';
import { defineConfig } from '@playwright/test';

const state = resolve(import.meta.dirname, '../../target/tests/current');
const results = process.env.CDELIVERY_UI_RESULTS ??= resolve(state, '../results', `ui-${randomUUID()}`);
mkdirSync(results, { recursive: true, mode: 0o700 });
let environment;
try {
  environment = JSON.parse(readFileSync(resolve(state, 'env.json'), 'utf8'));
  if (!environment.ready || !['single', 'cluster'].includes(environment.profile)) throw new Error('Run cargo xtask env up single or cluster first');
  writeFileSync(resolve(results, 'env.json'), JSON.stringify(environment, null, 2));
} catch (error) {
  writeFileSync(resolve(results, 'setup-error.txt'), String(error));
  throw error;
}

export default defineConfig({
  testDir: '.',
  testMatch: '*.spec.mjs',
  workers: 1,
  retries: 0,
  timeout: 120_000,
  expect: { timeout: 15_000 },
  outputDir: resolve(results, 'artifacts'),
  reporter: [['list'], ['json', { outputFile: resolve(results, 'result.json') }], ['html', { outputFolder: resolve(results, 'report'), open: 'never' }]],
  use: {
    baseURL: environment.base_url,
    browserName: 'chromium',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
});
