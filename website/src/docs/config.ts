import type { Locale } from '../config';

export const collections = {
  en: 'manual_en',
  de: 'manual_de',
  es: 'manual_es',
  fr: 'manual_fr',
  ja: 'manual_ja',
  'pt-BR': 'manual_pt_br',
  'zh-CN': 'manual_zh_cn',
  'zh-TW': 'manual_zh_tw',
} as const satisfies Record<Locale, string>;
