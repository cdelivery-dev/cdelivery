import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import type { Plugin } from 'vite';

export function header(): Plugin {
  const file = fileURLToPath(new URL('components/layout/MainHeader.astro', import.meta.resolve('@prosefly/astro-theme-lotus')));
  return {
    name: 'docs-header-layout',
    async load(id) {
      if (id !== file) return;
      const source = await readFile(file, 'utf8');
      const language = '    <LanguageSelect />\n\n';
      const theme = '        {showThemeControl && <ThemeSwitch />}\n';
      const search = '    <SearchDialog />';
      const menu = '    <MobileMenu\n';
      const end = '  </div>\n</header>';
      if ([language, theme, search, menu, end].some((part) => source.split(part).length !== 2)) {
        throw new Error('Lotus MainHeader no longer matches the header layout adapter');
      }
      return source
        .replace(language, '')
        .replace(theme, '')
        .replace(search, `    <div class="ms-auto flex min-w-0 items-center gap-1 sm:gap-2">\n${search}\n    <div class="flex items-center gap-1">`)
        .replace(menu, `${language}    {showThemeControl && <div class="hidden lg:flex"><ThemeSwitch /></div>}\n\n${menu}`)
        .replace(end, `    </div>\n    </div>\n${end}`);
    },
  };
}
