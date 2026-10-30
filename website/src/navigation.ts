import { site } from './config';

const docs = { path: '/docs/', key: 'site.nav.docs' } as const;
const blog = {
  path: '/blog/', key: 'site.nav.blog', hidden: true,
  items: [
    { path: '/blog/deploy-container-apps/', key: 'site.nav.blog.deploy-container-apps', icon: 'box' },
    { path: '/blog/manage-cluster/', key: 'site.nav.blog.manage-cluster', icon: 'server' },
    { path: '/blog/validate-releases/', key: 'site.nav.blog.validate-releases', icon: 'shield' },
    { path: '/blog/diagnose-incidents/', key: 'site.nav.blog.diagnose-incidents', icon: 'activity' },
    { path: '/blog/automate-routine-jobs/', key: 'site.nav.blog.automate-routine-jobs', icon: 'clock' },
    { path: '/blog/operate-through-chat/', key: 'site.nav.blog.operate-through-chat', icon: 'chat' },
  ],
} as const;
const pricing = { path: '/pricing/', key: 'site.nav.pricing' } as const;

export const navigation = [docs, blog, pricing] as const;

export const footerNavigation = [
  {
    key: 'site.footer.explore',
    items: [docs, blog, pricing],
  },
  {
    key: 'site.footer.about',
    items: [
      { path: `mailto:${site.email}`, key: 'site.footer.email' },
      { path: '/privacy/', key: 'site.footer.privacy' },
      { path: '/terms/', key: 'site.footer.terms' },
    ],
  },
] as const;

export type Menu = Extract<typeof navigation[number], { items: unknown }>;
