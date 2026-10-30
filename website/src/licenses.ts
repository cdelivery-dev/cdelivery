import { copyFile, readFile, writeFile } from 'node:fs/promises';

type License = { name: string; version: string; identifier: string; text?: string };

export async function writeLicenses(dir: URL) {
  const root = new URL('../', import.meta.url);
  const bundled: License[] = JSON.parse(await readFile(new URL('licenses/bundled.json', dir), 'utf8'));
  for (const license of bundled) {
    if (license.name === '@prosefly/astro-theme-lotus' && license.version === '0.8.0') {
      license.text = await readFile(new URL('../licenses/astro-theme-lotus-0.8.0.txt', root), 'utf8');
      license.text += '\nLocal modifications: CDelivery header replacement and Pagefind build hook.\n';
    }
  }
  const rendered = [
    ['astro', ['LICENSE']],
    ['@prosefly/astro-components', ['LICENSE']],
    ['pagefind', ['LICENSE/LICENSE', 'LICENSE/LICENSE-vscode-ripgrep']],
    ['@expressive-code/core', ['LICENSE']],
    ['@expressive-code/plugin-frames', ['LICENSE']],
    ['@expressive-code/plugin-line-numbers', ['LICENSE']],
    ['@expressive-code/plugin-text-markers', ['LICENSE']],
    ['@tailwindcss/typography', ['LICENSE']],
    ['tailwindcss', ['LICENSE']],
    ['@shikijs/themes', ['LICENSE']],
  ] as const;
  for (const [name, paths] of rendered) {
    const directory = new URL(`node_modules/${name}/`, root);
    const metadata = JSON.parse(await readFile(new URL('package.json', directory), 'utf8'));
    bundled.push({
      name, version: metadata.version, identifier: metadata.license,
      text: (await Promise.all(paths.map((path) => readFile(new URL(path, directory), 'utf8')))).join('\n\n'),
    });
  }
  const parts = [await readFile(new URL('public/THIRD_PARTY_LICENSES.txt', root), 'utf8')];
  for (const license of bundled) {
    if (!['MIT', 'BSD-3-Clause', 'Apache-2.0', 'ISC', 'CC0-1.0'].includes(license.identifier)
      || !license.text?.trim()) {
      throw new Error(`Review missing or unsupported license: ${license.name}@${license.version} (${license.identifier})`);
    }
    parts.push(`${license.name}@${license.version}\nLicense: ${license.identifier}\n\n${license.text}`);
  }
  await Promise.all([
    copyFile(new URL('../LICENSE', root), new URL('LICENSE.txt', dir)),
    copyFile(new URL('../NOTICE', root), new URL('NOTICE.txt', dir)),
    writeFile(new URL('THIRD_PARTY_LICENSES.txt', dir), parts.join(`\n${'='.repeat(80)}\n\n`)),
  ]);
}
