import { defineLotusConfig } from '@prosefly/astro-theme-lotus';
import { site, defaultLocale, localeDetails, locales } from './src/config';
import { translate } from './src/i18n';

const docsTranslations = Object.fromEntries(locales.map((locale) => [
  locale === defaultLocale ? 'root' : localeDetails[locale].path,
  translate(locale)('site.nav.docs'),
]));

export default defineLotusConfig({
  name: site.name,
  description: 'CDelivery documentation for getting started, application operations, and platform management.',
  docsBase: '/docs',
  defaultLocale: 'root',
  locales: Object.fromEntries(locales.map((locale) => [
    locale === defaultLocale ? 'root' : localeDetails[locale].path,
    { label: localeDetails[locale].label, lang: locale, directory: localeDetails[locale].path },
  ])),
  appearance: { accent: '#447dfc', gray: 'neutral', radius: 'medium', defaultMode: 'system' },
  siteNav: [],
  socials: [{ icon: 'simple-icons:github', label: 'GitHub', href: site.github }],
  docsNav: [{
    slug: 'docs',
    label: 'Docs',
    translations: docsTranslations,
    items: [
      'index',
      {
        label: 'Getting started',
        translations: {
          de: 'Erste Schritte',
          es: 'Primeros pasos',
          fr: 'Premiers pas',
          ja: 'はじめに',
          'pt-br': 'Primeiros passos',
          'zh-cn': '快速开始',
          'zh-tw': '快速入門',
        },
        items: [{ autogenerate: { directory: 'getting-started' } }],
      },
      {
        label: 'Guides',
        translations: {
          de: 'Anleitungen',
          es: 'Guías de uso',
          fr: 'Guides d’utilisation',
          ja: '使い方ガイド',
          'pt-br': 'Guias de uso',
          'zh-cn': '使用指南',
          'zh-tw': '使用指南',
        },
        items: [{ autogenerate: { directory: 'guides' } }],
      },
      {
        label: 'Platform management',
        translations: {
          de: 'Plattformverwaltung',
          es: 'Administración de la plataforma',
          fr: 'Administration de la plateforme',
          ja: 'プラットフォーム管理',
          'pt-br': 'Administração da plataforma',
          'zh-cn': '平台管理',
          'zh-tw': '平台管理',
        },
        items: ['administration'],
      },
      {
        label: 'AI documentation (English)',
        link: new URL('/llms-full.txt', site.url).href,
        translations: {
          de: 'KI-Dokumentation (Englisch)',
          es: 'Documentación para IA (inglés)',
          fr: 'Documentation pour l’IA (anglais)',
          ja: 'AI 向けドキュメント（英語）',
          'pt-br': 'Documentação para IA (inglês)',
          'zh-cn': 'AI 文档（英文）',
          'zh-tw': 'AI 文件（英文）',
        },
      },
    ],
  }],
  search: { provider: 'pagefind' },
  llms: { full: true },
  pageActions: [
    { type: 'copy-page', icon: 'lucide:copy' },
    { type: 'view-markdown', icon: 'lucide:file-text' },
  ],
  components: {
    SiteBrand: './src/components/docs/SiteBrand.astro',
  },
  favicon: '/icon.svg',
  head: [
    { tag: 'link', attrs: { rel: 'sitemap', href: '/sitemap-index.xml' } },
  ],
  credits: false,
  footer: { copyright: '© CDelivery', sections: [] },
});
