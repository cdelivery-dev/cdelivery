import { defineCollection } from 'astro:content';
import { docsCollection } from '@cloudflare/nimbus-docs/content';

export const collections = {
  manual_en: defineCollection(docsCollection({ base: 'docs/en' })),
  manual_de: defineCollection(docsCollection({ base: 'docs/de' })),
  manual_es: defineCollection(docsCollection({ base: 'docs/es' })),
  manual_fr: defineCollection(docsCollection({ base: 'docs/fr' })),
  manual_ja: defineCollection(docsCollection({ base: 'docs/ja' })),
  manual_pt_br: defineCollection(docsCollection({ base: 'docs/pt-br' })),
  manual_zh_cn: defineCollection(docsCollection({ base: 'docs/zh-cn' })),
  manual_zh_tw: defineCollection(docsCollection({ base: 'docs/zh-tw' })),
};
