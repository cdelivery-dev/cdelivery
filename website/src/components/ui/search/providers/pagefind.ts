import type { SearchProvider, SearchResult } from "@cloudflare/nimbus-docs/types";
import { searchIndex } from "virtual:nimbus/config";

interface PagefindSubResult {
  title?: string;
  url?: string;
}

interface PagefindResultData {
  url: string;
  excerpt?: string;
  meta?: { title?: string };
  sub_results?: PagefindSubResult[];
}

interface PagefindSearchResponse {
  results: Array<{ data(): Promise<PagefindResultData> }>;
}

interface PagefindApi {
  init(): Promise<void>;
  options(options: { baseUrl?: string }): Promise<void>;
  search(query: string): Promise<PagefindSearchResponse>;
}

let pagefind: PagefindApi | undefined;

export const provider: SearchProvider = {
  async init() {
    if (pagefind) return;
    const pagefindUrl = new URL(`${searchIndex.bundle}pagefind.js`, window.location.origin);
    const loaded = (await import(pagefindUrl.href)) as PagefindApi;
    const baseUrl = `${(import.meta.env.BASE_URL ?? "/").replace(/\/+$/, "")}/`;
    await loaded.options({ baseUrl });
    await loaded.init();
    pagefind = loaded;
  },

  async search(query) {
    if (!pagefind) await this.init?.();
    if (!pagefind) return [];

    const search = await pagefind.search(query);
    const results = await Promise.all(search.results.slice(0, 10).map((result) => result.data()));
    return results.map((result): SearchResult => ({
      title: result.meta?.title ?? "Untitled",
      url: result.url,
      snippet: result.excerpt,
      subResults: result.sub_results
        ?.filter((sub): sub is Required<PagefindSubResult> => Boolean(sub.title && sub.url))
        .map((sub) => ({ title: sub.title, url: sub.url })),
    }));
  },
};
