import { defineCollection } from 'astro:content';
import { docsSchema } from '@prosefly/astro-theme-lotus/content';
import { loader } from './docs/loader';

export const collections = {
  docs: defineCollection({ loader: loader(), schema: docsSchema() }),
};
