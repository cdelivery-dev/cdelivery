import { defineLotusConfig } from '@prosefly/astro-theme-lotus';
import { site, defaultLocale, localeDetails, locales } from './src/config';
import { translate } from './src/i18n';

const docsTranslations = Object.fromEntries(locales.map((locale) => [
  locale === defaultLocale ? 'root' : localeDetails[locale].path,
  translate(locale)('site.nav.docs'),
]));

export default defineLotusConfig({
  name: site.name,
  description: 'CDelivery documentation for getting started, application operations, and platform management.',
  docsBase: '/docs',
  defaultLocale: 'root',
  locales: Object.fromEntries(locales.map((locale) => [
    locale === defaultLocale ? 'root' : localeDetails[locale].path,
    { label: localeDetails[locale].label, lang: locale, directory: localeDetails[locale].path },
  ])),
  appearance: { accent: 'var(--lotus-text-strong)', gray: 'neutral', radius: 'medium', defaultMode: 'system' },
  siteNav: [],
  socials: [{ icon: 'simple-icons:github', label: 'GitHub', href: site.github }],
  docsNav: [{
    slug: 'docs',
    label: 'Docs',
    translations: docsTranslations,
    items: [
      "index",
      {
        "label": "Getting started",
        "translations": {
          "zh-cn": "快速开始",
          "zh-tw": "快速入門",
          "ja": "はじめに",
          "pt-br": "Primeiros passos",
          "es": "Primeros pasos",
          "fr": "Premiers pas",
          "de": "Erste Schritte"
        },
        "collapsed": false,
        "items": [
          "getting-started/introduction",
          "getting-started/requirements",
          "getting-started/installation",
          "getting-started/first-app",
          "getting-started/console"
        ]
      },
      {
        "label": "Guides",
        "translations": {
          "zh-cn": "使用指南",
          "zh-tw": "使用指南",
          "ja": "使い方ガイド",
          "pt-br": "Guias de uso",
          "es": "Guías de uso",
          "fr": "Guides d’utilisation",
          "de": "Anleitungen"
        },
        "collapsed": false,
        "items": [
          {
            "label": "Application management",
            "translations": {
              "zh-cn": "应用管理",
              "zh-tw": "應用程式管理",
              "ja": "アプリ管理",
              "pt-br": "Gerenciamento de aplicações",
              "es": "Gestión de aplicaciones",
              "fr": "Gestion des applications",
              "de": "Anwendungsverwaltung"
            },
            "collapsed": true,
            "items": [
              {
                "label": "Overview",
                "link": "/docs/guides/apps/",
                "slug": "guides/apps",
                "translations": {
                  "zh-cn": "概览",
                  "zh-tw": "概覽",
                  "ja": "概要",
                  "pt-br": "Visão geral",
                  "es": "Descripción general",
                  "fr": "Présentation",
                  "de": "Überblick"
                }
              },
              "guides/apps/create",
              {
                "label": "Runtime configuration",
                "translations": {
                  "zh-cn": "运行配置",
                  "zh-tw": "執行設定",
                  "ja": "実行設定",
                  "pt-br": "Configuração de execução",
                  "es": "Configuración de ejecución",
                  "fr": "Configuration d’exécution",
                  "de": "Laufzeitkonfiguration"
                },
                "collapsed": true,
                "items": [
                  "guides/apps/images",
                  "guides/apps/container",
                  "guides/apps/resources"
                ]
              },
              {
                "label": "Configuration and data",
                "translations": {
                  "zh-cn": "配置与数据",
                  "zh-tw": "設定與資料",
                  "ja": "設定とデータ",
                  "pt-br": "Configuração e dados",
                  "es": "Configuración y datos",
                  "fr": "Configuration et données",
                  "de": "Konfiguration und Daten"
                },
                "collapsed": true,
                "items": [
                  "guides/apps/xconfig",
                  "guides/apps/xconfig/mounting",
                  "guides/apps/persistence",
                  "guides/apps/persistence/stateful"
                ]
              },
              {
                "label": "Scaling and placement",
                "translations": {
                  "zh-cn": "扩缩容与调度",
                  "zh-tw": "擴縮與排程",
                  "ja": "スケーリングと配置",
                  "pt-br": "Escala e posicionamento",
                  "es": "Escalado y ubicación",
                  "fr": "Mise à l’échelle et placement",
                  "de": "Skalierung und Platzierung"
                },
                "collapsed": true,
                "items": [
                  "guides/apps/scaling",
                  "guides/apps/placement"
                ]
              },
              {
                "label": "Lifecycle",
                "translations": {
                  "zh-cn": "生命周期",
                  "zh-tw": "生命週期",
                  "ja": "ライフサイクル",
                  "pt-br": "Ciclo de vida",
                  "es": "Ciclo de vida",
                  "fr": "Cycle de vie",
                  "de": "Lebenszyklus"
                },
                "collapsed": true,
                "items": [
                  "guides/apps/lifecycle",
                  "guides/apps/delete"
                ]
              }
            ]
          },
          {
            "label": "Domains and HTTPS",
            "translations": {
              "zh-cn": "域名与 HTTPS",
              "zh-tw": "網域與 HTTPS",
              "ja": "ドメインと HTTPS",
              "pt-br": "Domínios e HTTPS",
              "es": "Dominios y HTTPS",
              "fr": "Domaines et HTTPS",
              "de": "Domains und HTTPS"
            },
            "collapsed": true,
            "items": [
              {
                "label": "Overview",
                "link": "/docs/guides/networking/",
                "slug": "guides/networking",
                "translations": {
                  "zh-cn": "概览",
                  "zh-tw": "概覽",
                  "ja": "概要",
                  "pt-br": "Visão geral",
                  "es": "Descripción general",
                  "fr": "Présentation",
                  "de": "Überblick"
                }
              },
              "guides/networking/connectivity",
              "guides/networking/routing",
              "guides/networking/automatic-certificates",
              "guides/networking/custom-certificates",
              "guides/networking/faq"
            ]
          },
          {
            "label": "Health checks and releases",
            "translations": {
              "zh-cn": "健康检查与发布",
              "zh-tw": "健康檢查與發布",
              "ja": "ヘルスチェックとリリース",
              "pt-br": "Verificações e lançamentos",
              "es": "Comprobaciones y despliegues",
              "fr": "Contrôles et déploiements",
              "de": "Prüfungen und Releases"
            },
            "collapsed": true,
            "items": [
              {
                "label": "XHealth",
                "translations": {
                  "zh-cn": "XHealth",
                  "zh-tw": "XHealth",
                  "ja": "XHealth",
                  "pt-br": "XHealth",
                  "es": "XHealth",
                  "fr": "XHealth",
                  "de": "XHealth"
                },
                "collapsed": true,
                "items": [
                  "guides/release-safety/xhealth",
                  "guides/release-safety/xhealth/status"
                ]
              },
              {
                "label": "Releases and rollback",
                "translations": {
                  "zh-cn": "发布与回滚",
                  "zh-tw": "發布與回復",
                  "ja": "リリースとロールバック",
                  "pt-br": "Lançamentos e reversão",
                  "es": "Despliegues y reversión",
                  "fr": "Déploiements et restauration",
                  "de": "Releases und Rollback"
                },
                "collapsed": true,
                "items": [
                  "guides/release-safety/update-strategy",
                  "guides/release-safety",
                  "guides/release-safety/rollback"
                ]
              }
            ]
          },
          {
            "label": "Logs, metrics, and events",
            "translations": {
              "zh-cn": "日志、指标与事件",
              "zh-tw": "日誌、指標與事件",
              "ja": "ログ・メトリクス・イベント",
              "pt-br": "Logs, métricas e eventos",
              "es": "Registros, métricas y eventos",
              "fr": "Journaux, métriques et événements",
              "de": "Logs, Metriken und Ereignisse"
            },
            "collapsed": true,
            "items": [
              {
                "label": "Overview",
                "link": "/docs/guides/observability/",
                "slug": "guides/observability",
                "translations": {
                  "zh-cn": "概览",
                  "zh-tw": "概覽",
                  "ja": "概要",
                  "pt-br": "Visão geral",
                  "es": "Descripción general",
                  "fr": "Présentation",
                  "de": "Überblick"
                }
              },
              {
                "label": "Logs",
                "translations": {
                  "zh-cn": "日志",
                  "zh-tw": "日誌",
                  "ja": "ログ",
                  "pt-br": "Logs",
                  "es": "Registros",
                  "fr": "Journaux",
                  "de": "Logs"
                },
                "collapsed": true,
                "items": [
                  "guides/observability/logs/application",
                  "guides/observability/logs/access"
                ]
              },
              {
                "label": "Metrics",
                "translations": {
                  "zh-cn": "指标",
                  "zh-tw": "指標",
                  "ja": "メトリクス",
                  "pt-br": "Métricas",
                  "es": "Métricas",
                  "fr": "Métriques",
                  "de": "Metriken"
                },
                "collapsed": true,
                "items": [
                  "guides/observability/metrics/application",
                  "guides/observability/metrics/server"
                ]
              },
              "guides/observability/events",
              "guides/observability/settings"
            ]
          },
          {
            "label": "Scheduled tasks",
            "translations": {
              "zh-cn": "定时任务",
              "zh-tw": "排程任務",
              "ja": "定期タスク",
              "pt-br": "Tarefas agendadas",
              "es": "Tareas programadas",
              "fr": "Tâches planifiées",
              "de": "Geplante Aufgaben"
            },
            "collapsed": true,
            "items": [
              "guides/automation/xjob",
              "guides/automation/xjob/schedule",
              "guides/automation/xjob/results"
            ]
          },
          {
            "label": "AI operations",
            "translations": {
              "zh-cn": "AI 运维",
              "zh-tw": "AI 維運",
              "ja": "AI 運用",
              "pt-br": "Operações com IA",
              "es": "Operaciones con IA",
              "fr": "Exploitation avec l’IA",
              "de": "KI-Betrieb"
            },
            "collapsed": true,
            "items": [
              {
                "label": "Overview",
                "link": "/docs/guides/automation/",
                "slug": "guides/automation",
                "translations": {
                  "zh-cn": "概览",
                  "zh-tw": "概覽",
                  "ja": "概要",
                  "pt-br": "Visão geral",
                  "es": "Descripción general",
                  "fr": "Présentation",
                  "de": "Überblick"
                }
              },
              "guides/automation/models",
              "guides/automation/duty"
            ]
          },
          {
            "label": "Messages and notifications",
            "translations": {
              "zh-cn": "消息与通知",
              "zh-tw": "訊息與通知",
              "ja": "メッセージと通知",
              "pt-br": "Mensagens e notificações",
              "es": "Mensajes y notificaciones",
              "fr": "Messages et notifications",
              "de": "Nachrichten und Benachrichtigungen"
            },
            "collapsed": true,
            "items": [
              {
                "label": "Overview",
                "link": "/docs/guides/automation/im/",
                "slug": "guides/automation/im",
                "translations": {
                  "zh-cn": "概览",
                  "zh-tw": "概覽",
                  "ja": "概要",
                  "pt-br": "Visão geral",
                  "es": "Descripción general",
                  "fr": "Présentation",
                  "de": "Überblick"
                }
              },
              {
                "label": "Channels",
                "translations": {
                  "zh-cn": "渠道接入",
                  "zh-tw": "管道接入",
                  "ja": "チャネル設定",
                  "pt-br": "Canais",
                  "es": "Canales",
                  "fr": "Canaux",
                  "de": "Kanäle"
                },
                "collapsed": true,
                "items": [
                  "guides/automation/im/email",
                  "guides/automation/im/slack",
                  "guides/automation/im/telegram",
                  "guides/automation/im/discord",
                  "guides/automation/im/lark",
                  "guides/automation/im/feishu"
                ]
              },
              "guides/automation/im/notifications"
            ]
          }
        ]
      },
      {
        "label": "Platform management",
        "translations": {
          "zh-cn": "平台管理",
          "zh-tw": "平台管理",
          "ja": "プラットフォーム管理",
          "pt-br": "Administração da plataforma",
          "es": "Administración de la plataforma",
          "fr": "Administration de la plateforme",
          "de": "Plattformverwaltung"
        },
        "collapsed": false,
        "items": [
          {
            "label": "Overview",
            "link": "/docs/administration/",
            "slug": "administration",
            "translations": {
              "zh-cn": "概览",
              "zh-tw": "概覽",
              "ja": "概要",
              "pt-br": "Visão geral",
              "es": "Descripción general",
              "fr": "Présentation",
              "de": "Überblick"
            }
          },
          {
            "label": "Servers and groups",
            "translations": {
              "zh-cn": "服务器与分组",
              "zh-tw": "伺服器與分組",
              "ja": "サーバーとグループ",
              "pt-br": "Servidores e grupos",
              "es": "Servidores y grupos",
              "fr": "Serveurs et groupes",
              "de": "Server und Gruppen"
            },
            "collapsed": true,
            "items": [
              "administration/nodes",
              "administration/nodes/labels",
              "administration/nodes/volumes",
              "administration/groups"
            ]
          },
          {
            "label": "Accounts and permissions",
            "translations": {
              "zh-cn": "账户与权限",
              "zh-tw": "帳戶與權限",
              "ja": "アカウントと権限",
              "pt-br": "Contas e permissões",
              "es": "Cuentas y permisos",
              "fr": "Comptes et autorisations",
              "de": "Konten und Berechtigungen"
            },
            "collapsed": true,
            "items": [
              "administration/accounts/roles",
              "administration/accounts/users",
              "administration/accounts/profile",
              "guides/automation/api-mcp"
            ]
          },
          {
            "label": "Platform maintenance",
            "translations": {
              "zh-cn": "平台维护",
              "zh-tw": "平台維護",
              "ja": "プラットフォーム保守",
              "pt-br": "Manutenção da plataforma",
              "es": "Mantenimiento de la plataforma",
              "fr": "Maintenance de la plateforme",
              "de": "Plattformwartung"
            },
            "collapsed": true,
            "items": [
              "administration/platform/access",
              "administration/platform/upgrades",
              "administration/platform/recovery",
              "administration/platform/uninstall"
            ]
          }
        ]
      },
      {
        label: 'AI documentation (English)',
        link: new URL('/llms-full.txt', site.url).href,
        translations: {
          de: 'KI-Dokumentation (Englisch)',
          es: 'Documentación para IA (inglés)',
          fr: 'Documentation pour l’IA (anglais)',
          ja: 'AI 向けドキュメント（英語）',
          'pt-br': 'Documentação para IA (inglês)',
          'zh-cn': 'AI 文档（英文）',
          'zh-tw': 'AI 文件（英文）',
        },
      },
    ],
  }],
  search: { provider: 'pagefind' },
  llms: { full: true },
  pageActions: [
    { type: 'copy-page', icon: 'lucide:copy' },
    { type: 'view-markdown', icon: 'lucide:file-text' },
  ],
  components: {
    SiteBrand: './src/components/docs/SiteBrand.astro',
  },
  favicon: '/icon.svg',
  head: [
    { tag: 'link', attrs: { rel: 'sitemap', href: '/sitemap-index.xml' } },
  ],
  credits: false,
  footer: { copyright: '© CDelivery', sections: [] },
});
