import { markdownRoute } from '@cloudflare/nimbus-docs/agent-endpoints';
import { defaultLocale, site } from '../../config';
import { translate } from '../../i18n';
import { docsLocale, docsPath } from '../../routes';

export const { GET, getStaticPaths } = markdownRoute({
  body(markdown, { url }) {
    const locale = docsLocale(url.pathname);
    if (locale === defaultLocale) return markdown;
    const t = translate(locale);
    const index = new URL(`${docsPath(locale)}llms.txt`, site.url);
    return markdown.replace(/^> Documentation Index\n> [^\n]+\n> [^\n]+$/m,
      () => `> [${t('docs.documentationIndex')}](${index})\n> ${t('docs.agentFormats')}`);
  },
});
