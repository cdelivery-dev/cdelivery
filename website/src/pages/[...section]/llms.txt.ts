import type { APIRoute } from 'astro';
import { llmsSectionRoute } from '@cloudflare/nimbus-docs/agent-endpoints';
import { GET as englishIndex } from '../llms.txt';

const route = llmsSectionRoute();
export const getStaticPaths = route.getStaticPaths;
export const GET: APIRoute = (context) => context.params.section === 'docs'
  ? englishIndex()
  : route.GET(context);
