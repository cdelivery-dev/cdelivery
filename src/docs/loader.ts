import type { Loader } from 'astro/loaders';
import { docsLoader } from '@prosefly/astro-theme-lotus/content';
import type { HeadConfig } from '@prosefly/astro-theme-lotus';
import { defaultLocale, localeDetails, locales, site } from '../config';
import { head } from './metadata';

export function loader(): Loader {
  const source = docsLoader();
  return {
    name: 'docs-with-metadata',
    async load(context) {
      await source.load({
        ...context,
        generateDigest: (content) => context.generateDigest({ content, site, localeDetails }),
        async parseData(entry) {
          const data = await context.parseData({ ...entry, data: { lastUpdated: false, ...entry.data } });
          const [directory, ...segments] = entry.id.split('/');
          const locale = locales.find((value) => localeDetails[value].path === directory);
          if (!locale) throw new Error(`Unknown documentation language: ${entry.id}`);
          return {
            ...data,
            head: [
              ...head(locale, segments.join('/'), String(data.title), String(data.description ?? '')),
              ...(data.head as HeadConfig | undefined ?? []),
            ],
          };
        },
      });
      const pages = new Map(locales.map((locale) => [locale, new Set<string>()]));
      for (const [id, entry] of context.store.entries()) {
        if (entry.data.draft) continue;
        const [directory, ...segments] = id.split('/');
        const locale = locales.find((value) => localeDetails[value].path === directory);
        if (locale) pages.get(locale)?.add(segments.join('/'));
      }
      const expected = pages.get(defaultLocale)!;
      for (const [locale, slugs] of pages) {
        if (slugs.size !== expected.size || [...expected].some((slug) => !slugs.has(slug))) {
          throw new Error(`Documentation pages do not match ${defaultLocale}: ${locale}`);
        }
      }
    },
  };
}
