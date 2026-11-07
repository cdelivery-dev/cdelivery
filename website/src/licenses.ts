import { copyFile, readFile, writeFile } from 'node:fs/promises';

type License = { name: string; version: string; identifier: string; text?: string };

export async function writeLicenses(dir: URL) {
  const root = new URL('../', import.meta.url);
  const bundled: License[] = JSON.parse(await readFile(new URL('licenses/bundled.json', dir), 'utf8'));
  bundled.push({
    name: 'Nimbus templates', version: '0.8.2', identifier: 'MIT',
    text: await readFile(new URL('../licenses/nimbus-templates-v0.8.2.txt', root), 'utf8'),
  }, {
    name: 'Phosphor Icons', version: '2.1.1', identifier: 'MIT',
    text: await readFile(new URL('../licenses/phosphor-icons-2.1.1.txt', root), 'utf8'),
  });
  const rendered = [
    ['astro', ['LICENSE']],
    ['@cloudflare/nimbus-docs', ['LICENSE']],
    ['@fontsource-variable/inter', ['LICENSE']],
    ['@fontsource-variable/jetbrains-mono', ['LICENSE']],
    ['pagefind', ['LICENSE/LICENSE', 'LICENSE/LICENSE-vscode-ripgrep']],
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
    if (!['MIT', 'BSD-3-Clause', 'Apache-2.0', 'ISC', 'CC0-1.0', 'OFL-1.1'].includes(license.identifier)
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
