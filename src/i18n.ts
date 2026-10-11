import { defaultLocale, locales, type Locale } from './config';
import de from './content/i18n/de.json';
import en from './content/i18n/en.json';
import es from './content/i18n/es.json';
import fr from './content/i18n/fr.json';
import ja from './content/i18n/ja.json';
import ptBR from './content/i18n/pt-BR.json';
import zhCN from './content/i18n/zh-CN.json';
import zhTW from './content/i18n/zh-TW.json';

type MessageKey = keyof typeof en;
const dictionaries: Record<Locale, Record<MessageKey, string>> = {
  de, en, es, fr, ja, 'pt-BR': ptBR, 'zh-CN': zhCN, 'zh-TW': zhTW,
};
const keys = Object.keys(en);

for (const [locale, messages] of Object.entries(dictionaries)) {
  const entries = Object.entries(messages);
  if (entries.length !== keys.length || entries.some(([key, value]) => !keys.includes(key) || !value.trim())) {
    throw new Error(`Invalid translations for ${locale}: keys must match English and values must be nonempty`);
  }
}

export function translate(locale: string | undefined) {
  const messages = dictionaries[locales.find((value) => value === locale) ?? defaultLocale];
  return (key: MessageKey, values?: Record<string, string | number>): string =>
    messages[key].replace(/\{\{\s*([\w.-]+)\s*\}\}/g, (match, name: string) => String(values?.[name] ?? match));
}
