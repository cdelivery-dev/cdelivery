import { defaultLocale, localeDetails, locales, type Locale } from './config';

export function docsPath(locale: Locale, slug = ''): string {
  const prefix = locale === defaultLocale ? '' : `${localeDetails[locale].path}/`;
  const path = slug.replace(/^\/+|\/+$/g, '');
  return `/docs/${prefix}${path ? `${path}/` : ''}`;
}

export function localePaths() {
  return locales.filter((locale) => locale !== defaultLocale)
    .map((locale) => ({ params: { locale: localeDetails[locale].path } }));
}

export function unlocalizedPath(pathname: string): string {
  for (const locale of locales) {
    if (locale === defaultLocale) continue;
    const prefix = `/${localeDetails[locale].path}`;
    if (pathname === prefix) return '/';
    if (pathname.startsWith(`${prefix}/`)) return pathname.slice(prefix.length);
  }
  return pathname;
}

export function isPublished(pathname: string): boolean {
  const path = unlocalizedPath(pathname);
  return path === '/' || path === '/pricing/' || path.startsWith('/docs/');
}
