import type { NormalizedLocale } from '@prosefly/astro-theme-lotus';

declare global {
  namespace App {
    interface Locals {
      t: {
        (key: string, values?: Record<string, string | number | boolean | undefined>): string;
        locale(): NormalizedLocale;
      };
    }
  }
}
