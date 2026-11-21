import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect } from '@playwright/test';

const state = resolve(import.meta.dirname, '../../target/tests/current');
const credentials = JSON.parse(readFileSync(resolve(state, 'credentials.json'), 'utf8'));

export const headers = { Authorization: `Bearer ${credentials.api_key}` };

export function readEnvironment() {
  return JSON.parse(readFileSync(resolve(state, 'env.json'), 'utf8'));
}

export async function login(page, email = credentials.email, password = credentials.password) {
  await page.goto('/login');
  const destination = new URL('/', page.url()).href;
  await page.locator('#email').fill(email);
  await page.locator('#password').fill(password);
  await page.locator('#login_form input[type=submit]').click();
  await expect(page).toHaveURL(destination);
}
