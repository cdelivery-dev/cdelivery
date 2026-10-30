import { copyFile } from 'node:fs/promises';
import { defineConfig } from 'astro/config';
import lotus from '@prosefly/astro-theme-lotus';
import sitemap from '@astrojs/sitemap';
import theme from './lotus.config.ts';
import { site, defaultLocale, localeDetails, locales } from './src/config.ts';
import { isPublished } from './src/routes.ts';
import { alternateLinks } from './src/docs/metadata.ts';
import { indexDocs } from './src/docs/search.ts';
import { header } from './src/docs/header.ts';
import { writeLicenses } from './src/licenses.ts';

const docs = lotus(theme);
docs.hooks['astro:build:done'] = indexDocs;

export default defineConfig({
  site: site.url,
  output: 'static',
  trailingSlash: 'ignore',
  vite: {
    plugins: [header()],
    build: {
      license: { fileName: 'licenses/bundled.json' },
      rolldownOptions: {
        onLog(level, log, defaultHandler) {
          if (level === 'warn' && log.code === 'MODULE_LEVEL_DIRECTIVE'
            && log.message.includes('use astro:head-inject')
            && log.id?.endsWith('.mdx?astroPropagatedAssets')) return;
          defaultHandler(level, log);
        },
      },
    },
  },
  i18n: {
    defaultLocale,
    locales: locales.map((locale) => ({ path: localeDetails[locale].path, codes: [locale] })),
    routing: { prefixDefaultLocale: false },
  },
  integrations: [
    docs,
    sitemap({
      filter: (page) => isPublished(new URL(page).pathname),
      serialize: (item) => ({ ...item, links: alternateLinks(new URL(item.url).pathname) }),
    }),
    {
      name: 'website-output',
      hooks: {
        'astro:build:done': async ({ dir }) => {
          await writeLicenses(dir);
          await Promise.all(locales.filter((locale) => locale !== defaultLocale).flatMap((locale) => {
            const path = localeDetails[locale].path;
            const source = new URL(`${path}/404/index.html`, dir);
            return [
              copyFile(source, new URL(`${path}/404.html`, dir)),
              copyFile(source, new URL(`docs/${path}/404.html`, dir)),
            ];
          }));
        },
      },
    },
  ],
});
