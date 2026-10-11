import type { HeadConfig } from '@prosefly/astro-theme-lotus';
import { defaultLocale, localeDetails, locales, site, type Locale } from '../config';
import { docsPath } from '../routes';

export function alternateLinks(pathname: string) {
  const docs = pathname.startsWith('/docs/');
  const segments = pathname.slice(docs ? '/docs/'.length : 1).split('/').filter(Boolean);
  if (locales.some((locale) => locale !== defaultLocale && localeDetails[locale].path === segments[0])) {
    segments.shift();
  }
  const slug = segments.join('/');
  const links = locales.map((locale) => ({
    lang: locale,
    url: new URL(docs ? docsPath(locale, slug) : locale === defaultLocale ? '/' : `/${localeDetails[locale].path}/`, site.url).href,
  }));
  return [...links, { lang: 'x-default', url: new URL(docs ? docsPath(defaultLocale, slug) : '/', site.url).href }];
}

export function head(locale: Locale, slug: string, title: string, description: string): HeadConfig {
  const path = docsPath(locale, slug);
  const canonical = new URL(path, site.url).href;
  return [
    { tag: 'link', attrs: { rel: 'canonical', href: canonical } },
    ...alternateLinks(path).map(({ lang, url }) => ({ tag: 'link', attrs: { rel: 'alternate', hreflang: lang, href: url } })),
    { tag: 'meta', attrs: { property: 'og:site_name', content: site.name } },
    { tag: 'meta', attrs: { property: 'og:title', content: `${title} - ${site.name}` } },
    { tag: 'meta', attrs: { property: 'og:description', content: description } },
    { tag: 'meta', attrs: { property: 'og:url', content: canonical } },
    { tag: 'meta', attrs: { property: 'og:type', content: 'article' } },
    { tag: 'meta', attrs: { property: 'og:locale', content: localeDetails[locale].ogLocale } },
    { tag: 'meta', attrs: { name: 'twitter:card', content: 'summary' } },
  ];
}
