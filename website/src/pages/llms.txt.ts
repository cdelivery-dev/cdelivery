import { getCollection } from 'astro:content';
import { getIndexedEntries } from '@cloudflare/nimbus-docs/runtime';
import { collections } from '../docs/config';
import { defaultLocale, locales, site } from '../config';

export async function GET() {
  const pages = new Map(await Promise.all(locales.map(async (locale) => [
    locale,
    new Set((await getCollection(collections[locale], ({ data }) => !data.draft)).map(({ id }) => id)),
  ] as const)));
  const expected = pages.get(defaultLocale)!;
  for (const [locale, slugs] of pages) {
    if (slugs.size !== expected.size || [...expected].some((slug) => !slugs.has(slug))) {
      throw new Error(`Documentation pages do not match ${defaultLocale}: ${locale}`);
    }
  }
  const entries = (await getIndexedEntries()).filter((entry) => entry.collection === collections.en);
  const content = `# ${site.name}\n\nEnglish documentation for installing CDelivery and operating applications.\n\n`
    + entries.map((entry) => `- [${entry.title}](${new URL(entry.markdownUrl, site.url)}): ${entry.description ?? ''}`).join('\n');
  return new Response(`${content}\n`, { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
}
