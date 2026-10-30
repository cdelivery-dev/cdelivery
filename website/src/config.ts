const repository = 'cdelivery-dev/cdelivery';
const url = 'https://cdelivery.dev';

export const site = {
  name: 'CDelivery',
  url,
  github: `https://github.com/${repository}`,
  x: 'https://x.com/cdeliverdev',
  email: 'hello@cdelivery.dev',
  repository,
  installCommand: `curl -fsSL ${url}/install.sh | sh`,
  installGuide: `${url}/install.md`,
};

export const defaultLocale = 'en';
export const localeDetails = {
  de: { label: 'Deutsch', path: 'de', ogLocale: 'de_DE' },
  en: { label: 'English', path: 'en', ogLocale: 'en_US' },
  es: { label: 'Español', path: 'es', ogLocale: 'es_ES' },
  fr: { label: 'Français', path: 'fr', ogLocale: 'fr_FR' },
  ja: { label: '日本語', path: 'ja', ogLocale: 'ja_JP' },
  'pt-BR': { label: 'Português (Brasil)', path: 'pt-br', ogLocale: 'pt_BR' },
  'zh-CN': { label: '简体中文', path: 'zh-cn', ogLocale: 'zh_CN' },
  'zh-TW': { label: '繁體中文', path: 'zh-tw', ogLocale: 'zh_TW' },
} as const;
export type Locale = keyof typeof localeDetails;
export const locales = Object.keys(localeDetails).sort() as Locale[];
