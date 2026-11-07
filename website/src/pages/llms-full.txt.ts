import { getMarkdownStaticPaths, getMarkdownPayload } from '@cloudflare/nimbus-docs/agent-endpoints';
import { collections } from '../docs/config';

export async function GET() {
  const pages = await getMarkdownStaticPaths({ collection: collections.en, surface: 'markdown' });
  const content = await Promise.all(pages.map(async ({ props: { reference } }) => {
    const page = await getMarkdownPayload({ collection: collections.en, surface: 'markdown', reference });
    if (!page) throw new Error(`Missing English Markdown: ${reference.id}`);
    return page.body;
  }));
  return new Response(content.join('\n\n'), { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
}
