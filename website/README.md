# Website

Astro static output for Cloudflare Pages. Node.js 22.12 or newer is required.

The build publishes `LICENSE.txt`, `NOTICE.txt`, and `THIRD_PARTY_LICENSES.txt`, linked from the Terms page. `src/licenses.ts` combines Vite's bundle license report with notices for the theme, rendered components, search, icons, and fonts. The versioned Lotus supplement in `../licenses/` supplies the license omitted from its npm archive. Review the supplements and rendered dependency list when upgrading the theme or changing assets; missing or unsupported bundle licenses fail the build.

| Pages setting | Value |
| --- | --- |
| Root directory | `website` |
| Build command | `npm run check && npm run build` |
| Output directory | `dist` |
| Production domain | `cdelivery.dev` |

`npm ci` installs the locked dependencies. The build validates translation keys and generates the static site and sitemap. `src/routes.ts` includes homepages, Pricing, Privacy, Terms and Docs in the sitemap. The Blog placeholder retains `noindex`, remains in the footer and stays hidden in the top navigation. The homepage switches between a copyable installation command and a localized AI prompt, defaulting to manual installation. `src/config.ts` owns both the command and the production guide URL. `public/install.sh` is published as `/install.sh`; the independent English AI guide in `public/install.md` is published as `/install.md` and covers remote installation, administrator setup and the current AI client's MCP connection. Release metadata is served independently at `https://releases.cdelivery.dev/v1/`.

Docs use Lotus. Content lives in `src/content/docs/{locale}/`, including `en` for English, using the locale paths from `src/config.ts`. English URLs start with `/docs/`; translations start with `/docs/{locale}/`. Keep all eight languages in sync; the build rejects mismatched published page sets. Use `.mdx` for pages: Lotus's callout directives generate MDX components. Frontmatter supplies `title`, `description`, and `sidebar.order`. Use absolute site-root documentation paths for internal links.

Routes accept URLs with or without a trailing slash during development. Static directory output, canonical URLs and sitemap entries retain the trailing slash; production normalization follows Cloudflare Pages' directory routing.

Each language has 64 published pages: the documentation homepage, five getting-started pages, 45 guide pages, and 13 platform-management pages. The API/MCP page belongs to platform management in navigation while retaining its existing `guides/automation/api-mcp/` URL. All 17 previously published URLs per language remain available. Author changes directly in the eight formal locale directories; do not introduce a separate review tree or publish incomplete translations. Verify instructions against the current UI and implementation, including prerequisites, ordered steps, expected results, defaults, limitations, failure handling, and related pages. Installation uses the homepage command from `src/config.ts`; the dashboard is available at the server's IP address on port 9000, or `localhost:9000` on macOS. Keep installation instructions aligned with Docker prerequisites and `--advertise-addr` behavior. Link only to existing pages.

`lotus.config.ts` explicitly orders the translated sidebar under Getting started, Guides, and Platform management. Guides contain application management, domains and HTTPS, health checks and releases, observability, scheduled tasks, AI operations, and messaging. Application configuration and data live under application management; XHealth and XJob remain separate modules. Add subgroups where they reflect a task relationship, and include each published page once. Keep overview labels short while retaining descriptive page titles. Root sections stay open; deeper groups use Lotus's native active-ancestor expansion. `src/styles/docs.css` supplies cumulative indentation and wrapped labels for deeper levels. MDX uses `@prosefly/astro-components`.

`npm run build` generates Pagefind search, per-page Markdown, `/llms.txt` and `/llms-full.txt`. The text bundles include only non-draft English Docs and retain the original Markdown/MDX source. Generated files stay in `dist`. Use `npm run preview` to verify search and AI output after a build.

For documentation acceptance, run `npm run check` and `npm run build`, verify 64 published pages in each of the eight languages, check internal links and retained URLs, and inspect deep sidebar paths on desktop and mobile in both themes. Production output should contain 512 documentation pages and their Markdown exports, with reciprocal language links and matching sitemap entries; the English AI export should contain 64 pages. Verify search with the production preview. Static checks do not replace executing deployment, recovery, or destructive procedures in an appropriate environment.

Lotus 0.8.0 dynamically imports Pagefind after Astro 7 closes its module loader. `src/docs/search.ts` replaces that build hook using Pagefind's public API with a static import and indexes only Docs. Remove this replacement when the upstream hook works with Astro 7. The build needs access to Iconify's API for native theme icons; generated pages contain the icons inline.

The Vite log filter ignores only Astro 7.3.3's generated `use astro:head-inject` directive warning in MDX asset modules; Astro propagates their styles using the module query, not the directive. Other warnings remain visible. After changing content file extensions without changing their contents, run `npm exec -- astro sync --force` once to invalidate Astro's content cache.

Marketing pages use `src/i18n.ts` and the existing eight-language dictionaries independently of Lotus UI translations. Privacy and Terms share `InformationLayout.astro`. English uses `/privacy/` and `/terms/`, with `/{locale}/` prefixes for translations. The footer Email link opens `mailto:` using `site.email` from `src/config.ts`. Privacy covers website visits, browser preferences and correspondence; self-hosted instances remain their operators' responsibility. Review the policy against actual hosting and data-processing settings before deployment. Keep policy dates explicit in the page source. Documentation metadata is generated through the content loader; sitemap language links preserve the corresponding page path. Set `lastUpdated` explicitly when needed; filesystem modification times are not publication dates.

Website acceptance includes 544 sitemap entries: 512 Docs pages and 32 pages across Home, Pricing, Privacy and Terms. Check generated internal links and anchors, same-page language switching, reciprocal language metadata, email links and license downloads. Verify both information pages and the footer Email link on desktop and mobile. The removed contact and component demonstration pages must not appear in navigation, search or AI exports. In an isolated deployment, verify recovery commands using `/entrypoint` in the running main container; static checks cannot prove recovery or installation success.

Configure permanent redirects from the production `<project>.pages.dev`, `www.cdelivery.dev` and HTTP URLs to `https://cdelivery.dev`, preserving path suffixes and query strings. Match the production host only; do not include preview subdomains. See Cloudflare's [custom-domain redirect guide](https://developers.cloudflare.com/pages/how-to/redirect-to-custom-domain/). Fingerprinted assets under `/_astro/` use one-year immutable browser caching through `public/_headers`; `/install.sh` and `/install.md` require cache revalidation. Other paths use Pages' default caching. Keep Pages' default [preview `X-Robots-Tag: noindex` behavior](https://developers.cloudflare.com/pages/configuration/preview-deployments/#preview-indexing-by-search-engines).

After deployment, verify:

- The eight homepages return 200, their own canonical URL, reciprocal language links and no `noindex`.
- Pricing, Privacy, Terms and Docs return 200 with canonical and reciprocal language links; their eight translations appear alongside homepages in the sitemap. Blog placeholders return 200 with `noindex` and stay out of the sitemap.
- Missing paths under every locale return HTTP 404 with the matching language, including nested paths and the removed `/contact/` routes.
- Default-host, `www`, HTTP, `.html` and trailing-slash redirects resolve to the intended canonical URL without loops.
- Preview responses include `X-Robots-Tag: noindex`; production homepages do not.
- The font response includes `Cache-Control: public, max-age=31536000, immutable`; HTML retains Pages' default caching.
- `/install.sh` returns the installer source with `Cache-Control: no-cache`.
- `/install.md` returns the AI installation guide as `text/markdown; charset=utf-8` with `Cache-Control: no-cache`; both homepage modes copy their displayed text.
- The release catalog and referenced images can be downloaded from a clean installation environment; installation and first login complete successfully.
- GitHub Issues and private vulnerability reporting are available for the public repository. The contact mailbox is operational; mailbox provisioning is outside this repository.

Keep Lotus's root `404.html` and the localized copies at `/{locale}/404.html` and `/docs/{locale}/404.html`. [Pages uses the nearest ancestor's 404 file](https://developers.cloudflare.com/pages/configuration/serving-pages/); no SPA rewrite is needed. The repository does not configure Cloudflare account rules or publish deployments.
