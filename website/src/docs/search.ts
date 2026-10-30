import type { AstroIntegration } from 'astro';
import { fileURLToPath } from 'node:url';
import { close, createIndex } from 'pagefind';

export const indexDocs: NonNullable<AstroIntegration['hooks']['astro:build:done']> = async ({ dir, logger }) => {
  try {
    const { index, errors } = await createIndex({
      excludeSelectors: ['[data-pagefind-ignore]', 'script', 'style', 'nav', 'footer'],
    });
    if (!index || errors.length) throw new Error(`Pagefind initialization failed: ${errors.join('\n')}`);
    const result = await index.addDirectory({ path: fileURLToPath(dir), glob: 'docs/**/*.html' });
    if (result.errors.length) throw new Error(result.errors.join('\n'));
    const output = await index.writeFiles({ outputPath: fileURLToPath(new URL('pagefind/', dir)) });
    if (output.errors.length) throw new Error(output.errors.join('\n'));
    logger.info(`Pagefind indexed ${result.page_count} documentation pages.`);
  } finally {
    await close();
  }
};
