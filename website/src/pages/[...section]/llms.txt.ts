import type { APIRoute } from 'astro';
import { llmsSectionRoute } from '@cloudflare/nimbus-docs/agent-endpoints';
import { defaultLocale } from '../../config';
import { docsSectionLabels, translate } from '../../i18n';
import { docsLocale } from '../../routes';
import { GET as englishIndex } from '../llms.txt';

const route = llmsSectionRoute({
  body(markdown, { url }) {
    const locale = docsLocale(url.pathname);
    if (locale === defaultLocale) return markdown;
    const t = translate(locale);
    const labels = docsSectionLabels(locale);
    const label = (title: string) => labels[title.toLowerCase()] ?? title;
    return markdown
      .replace(/^## Pages$/gm, `## ${t('docs.indexPages')}`)
      .replace(/^## Sections$/gm, `## ${t('docs.indexSections')}`)
      .replace(/^# (.+)$/m, (_, title: string) => `# ${label(title)}`)
      .replace(/^- \[([^\]\n]+)\](?=\(\S+\/llms\.txt\)$)/gm, (_, title: string) => `- [${label(title)}]`);
  },
});
export const getStaticPaths = route.getStaticPaths;
export const GET: APIRoute = (context) => context.params.section === 'docs'
  ? englishIndex()
  : route.GET(context);
