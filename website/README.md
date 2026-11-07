# Website

CDelivery's static website uses Astro with Nimbus for documentation and is hosted on Cloudflare Pages. Requires Node.js 22.12 or newer.

## Development

Run commands from `website/`:

```sh
npm ci
npm run dev
```

Build, validate, and preview:

```sh
npm run build && npm run check && npm run lint:docs
npm run preview
```

Build first so link checks use the current route map.

## Content

- Documentation lives in `src/content/docs/{locale}/` as MDX, including `en` for English. Keep all eight languages in sync.
- Frontmatter defines `title`, `description`, `sidebar.order`, and localized `sidebar.group.label` where needed.
- Use absolute site-root paths for internal links, such as `/docs/getting-started/installation/`.

The build generates license files; review `src/licenses.ts` and `../licenses/` when updating dependencies or assets.

## Deployment

| Pages setting | Value |
| --- | --- |
| Root directory | `website` |
| Build command | `npm run build && npm run check && npm run lint:docs` |
| Output directory | `dist` |
| Production domain | `cdelivery.dev` |

Configure permanent redirects from the production `<project>.pages.dev`, `www.cdelivery.dev`, and HTTP URLs to `https://cdelivery.dev`, preserving paths and query strings. Exclude preview subdomains and retain their `X-Robots-Tag: noindex` header. These settings are managed in Cloudflare.
