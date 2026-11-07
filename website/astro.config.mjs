import { copyFile } from 'node:fs/promises';
import { defineConfig } from 'astro/config';
import nimbus from '@cloudflare/nimbus-docs';
import { tableScroll } from '@cloudflare/nimbus-docs/markdown';
import tailwindcss from '@tailwindcss/vite';
import sitemap from '@astrojs/sitemap';
import { collections } from './src/docs/config.ts';
import { site, defaultLocale, localeDetails, locales } from './src/config.ts';
import { isPublished } from './src/routes.ts';
import { alternateLinks } from './src/docs/metadata.ts';
import { writeLicenses } from './src/licenses.ts';

export default defineConfig({
  site: site.url,
  output: 'static',
  trailingSlash: 'ignore',
  vite: {
    plugins: [tailwindcss()],
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
    nimbus({
      site: site.url,
      title: site.name,
      description: 'CDelivery documentation for getting started, application operations, and platform management.',
      locale: defaultLocale,
      github: site.github,
      agents: { discovery: false },
      sidebar: {
        defaultCollapsed: true,
        items: locales.map((locale) => ({
          label: localeDetails[locale].label,
          segment: locale === defaultLocale ? '/docs' : `/docs/${localeDetails[locale].path}`,
          landing: locale === defaultLocale ? '/docs' : `/docs/${localeDetails[locale].path}`,
          items: [{ autogenerate: { collection: collections[locale] } }],
        })),
      },
    }, {
      sitemap: false,
      markdown: {
        hastPlugins: [tableScroll()],
        features: { headingAttributes: true },
        componentMap: {
          Card: {
            revision: '1',
            render: ({ attrs, children }) => `- **${attrs.href ? `[${attrs.title}](${attrs.href})` : attrs.title}** — ${children.trim()}`,
          },
        },
      },
      rules: { 'nimbus/frontmatter-shape': 'error', 'nimbus/internal-link': 'error' },
    }),
    sitemap({
      filter: (page) => isPublished(new URL(page).pathname),
      serialize: (item) => ({ ...item, links: alternateLinks(new URL(item.url).pathname) }),
    }),
    {
      name: 'website-output',
      hooks: {
        'astro:build:done': async ({ dir }) => {
          await writeLicenses(dir);
          await Promise.all(locales.filter((locale) => locale !== defaultLocale).map((locale) => {
            const path = localeDetails[locale].path;
            return copyFile(new URL(`${path}/404/index.html`, dir), new URL(`${path}/404.html`, dir));
          }));
          await Promise.all(locales.map((locale) => {
            const prefix = locale === defaultLocale ? 'docs' : `docs/${localeDetails[locale].path}`;
            return copyFile(new URL(`${prefix}/404/index.html`, dir), new URL(`${prefix}/404.html`, dir));
          }));
        },
      },
    },
  ],
});
