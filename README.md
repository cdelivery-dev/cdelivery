<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset=".github/assets/logo-dark.svg">
    <img src=".github/assets/logo-light.svg" alt="" width="46" height="28">
  </picture>
  CDelivery
</h1>

<p align="center"><strong>Safer deployments. Simpler operations.</strong></p>

<p align="center">A lightweight, self-hosted operations platform for deploying and running applications on your own servers.</p>

<p align="center">
  <a href="https://cdelivery.dev">Website</a> /
  <a href="https://cdelivery.dev/docs/">Documentation</a> /
  <a href="#get-started">Getting started</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square" alt="License: Apache-2.0"></a>
</p>

## Features

- **Release with confidence.** [XHealth](https://cdelivery.dev/docs/guides/release-safety/xhealth/) regularly checks that your app is online and that users can complete tasks like signing in. During an update, [release checks](https://cdelivery.dev/docs/guides/release-safety/guardian/) start a preview instance in the same production environment—only health checks reach it while the current version keeps serving users, and a failed check blocks the release.
  - **Verify app updates with connected apps.** When one app calls another, CDelivery checks that they still work together when either [app dependency](https://cdelivery.dev/docs/guides/apps/dependencies/app/) is updated. If a health check fails for either app, the release is blocked and the current version keeps serving users.
  - **Verify database changes with your apps.** CDelivery creates a temporary branch of the production [database](https://cdelivery.dev/docs/guides/apps/dependencies/postgresql/) and applies the change there. It connects a preview instance of each dependent app to the branch and runs that app’s health checks; live apps keep using the production database, which receives the change only after all checks pass.
- **Deploy and scale.** [Run container images](https://cdelivery.dev/docs/guides/apps/) on your own servers, starting with one machine and adding more as your applications grow. [Automatic replica scaling](https://cdelivery.dev/docs/guides/apps/scaling/) adds or removes replicas based on CPU load.
- **Logs and metrics.** Enable and configure [Insight](https://cdelivery.dev/docs/guides/observability/) to collect and query server and application metrics, application logs, and gateway access records—ready out of the box with no SDK required, using local disk or object storage.
- **HTTPS and load balancing.** [Pingress](https://cdelivery.dev/docs/guides/networking/), built on Cloudflare’s [Pingora](https://github.com/cloudflare/pingora), routes requests, balances traffic across instances, and manages HTTPS certificates automatically across single- and multi-instance deployments.
- **AI operations.** Configure a model provider for [AI diagnostics and proactive on-call](https://cdelivery.dev/docs/guides/automation/duty/), with actions limited by the active role and notifications through [Slack, email, Discord, Telegram, Feishu, and Lark](https://cdelivery.dev/docs/guides/automation/im/). Connect your own tools through [MCP and REST](https://cdelivery.dev/docs/guides/automation/api-mcp/).
- **Configuration and scheduled jobs.** [XConfig](https://cdelivery.dev/docs/guides/apps/xconfig/) distributes uploaded configuration to applications without local data volumes and validates updates with XHealth. Shared [Secrets](https://cdelivery.dev/docs/guides/apps/secrets/) supply environment variables and read-only files, including files for applications with local volumes. [XJob](https://cdelivery.dev/docs/guides/automation/xjob/) runs commands on a schedule or on demand in fresh containers.
- **Lightweight by design.** A compact codebase of **48k lines of Rust**, with the core service using **50 MiB** of memory under low load.
- **No vendor lock-in.** Choose the servers, storage, and infrastructure that fit your needs. Fewer unexpectedly huge bills.

## Get started

Prepare your server using the [installation guide](https://cdelivery.dev/docs/getting-started/installation/), then run:

```sh
curl -fsSL https://cdelivery.dev/install.sh | sh
```

Or use AI-assisted installation by pasting this prompt into your AI coding tool:

```text
Help me install and configure CDelivery: https://cdelivery.dev/install.md
```

On first installation, choose a stable communication address that your other servers can reach; `127.0.0.1` is for a single server only.

Open `http://SERVER_IP:9000`, create your first administrator account, and sign in. Make port `9000` reachable from your management network.

The guide also covers address options and macOS setup. Next, [deploy your first application](https://cdelivery.dev/docs/getting-started/first-app/).

## Where CDelivery is different

Running containers, reverse proxying, single-node HTTPS, and basic restarts are table stakes. The differences below are where your choice changes reliability and ongoing effort:

| | Typical self-hosted tools | Kubernetes + open-source ops stack | CDelivery |
| --- | --- | --- | --- |
| **Avoid “worked in staging, broke in prod”** | Process or port liveness only; traffic switches without verifying workflows | Basic probes; isolated prod and caller checks need custom pipelines | Isolated preview in real production (no live traffic) + caller checks before rollout |
| **Rehearse database changes on real data** | Not included; teams back up and run changes directly on production | Orchestrates containers only; DB branching needs extra tools and scripts | Forks the production DB to a temporary branch (CDelivery-deployed, Neon, and PlanetScale) and validates dependent apps first |
| **Observability cost & upkeep** | Live container logs only; long-term search needs extra tools or SaaS | Self-hosted HA Prometheus/Loki is heavy; SaaS like Datadog brings high bills | Built-in Insight, no SDK, queryable in 3s, free multi-instance HA, ~1% of Datadog cost |
| **Multi-node HA & upgrades** | Built mainly for single nodes; distributed HA often needs paid add-ons | Combining components like Traefik and Argo makes upgrades risky | Scales from 1 to 5,000 servers; free multi-instance HA, rolling upgrades, multi-node HTTPS sync |
| **Dependencies & footprint** | Often bundles a DB or cache; config, jobs, and logs pile up components | Control plane, etcd, and add-ons take gigabytes of RAM and steady ops | Requires only Docker; built-in XConfig, Secrets, and XJob; ~200 MiB starting memory |
| **AI on-call with full context** | Lacks long-term history and active on-call; troubleshooting is manual copy-paste | Siloed tools leave AI with data but no actions (or actions without context) | Unifies metrics, logs, access, and events; proactive on-call, role-scoped actions, MCP/API |
| **Ownership & cost** | Self-hosted, no per-seat fees | Open source; main cost is ops staffing | Apache License 2.0, free for commercial use, 100% of data stays on your servers |

<sub>Reflects the typical out-of-the-box experience of each approach rather than a specific product. Many of these capabilities can be built with extra tools, custom pipelines, or paid plans.</sub>

## Documentation

Read the [documentation](https://cdelivery.dev/docs/) for installation, application management, networking, observability, and automation. Guides include setup steps, verification, limitations, and troubleshooting.

[Platform management](https://cdelivery.dev/docs/administration/) covers servers, accounts, permissions, upgrades, and recovery. For external tools, see [API and MCP access](https://cdelivery.dev/docs/guides/automation/api-mcp/) or the AI-readable [documentation index](https://cdelivery.dev/llms.txt).

The interface and documentation support English, Simplified Chinese, Traditional Chinese, Japanese, Brazilian Portuguese, Spanish, French, and German.

## Performance

### Starting memory

Memory usage under low load:

| Component | Role | Memory |
| --- | --- | ---: |
| CDelivery | Core service | 50 MiB |
| Pingress | Reverse proxy gateway | 30 MiB |
| Observability | Data collection and storage | 120 MiB |

### Benchmarks

| Component | Throughput | Peak memory | Test environment |
| --- | ---: | ---: | --- |
| Insight | 2,491 MB/s ingestion | 2.21 GiB | AWS c8gb.2xlarge, Graviton4, 8 cores, 16 GiB RAM |
| Pingress | 45,982 requests/s | 963 MiB | AWS c8gn.large, Graviton4, 2 cores, 4 GiB RAM |

## AI disclosure

CDelivery is the DevOps system we use in production. We spent 18 months developing it and started over at least ten times before settling on the current version.

This version started with more than 30,000 lines of production Rust code that we wrote by hand. In July 2026, we began using AI to help with design, development, testing, code review, and acceptance testing.

AI is just a tool. Every feature decision, design decision, and code change must have a named developer responsible for it. Before a change is merged, that developer must personally review the code and verify the change. They’re also responsible for any fixes it needs later.

Each commit is limited to 300 lines, counting additions and deletions together. Keeping changes small gives us time to talk them through with the AI and understand every detail. It also helps keep the AI from changing more than the task calls for.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) to contribute and [SECURITY.md](SECURITY.md) to report vulnerabilities privately.

## License

Copyright 2026 The CDelivery Authors. First-party source code, documentation, scripts, and tests are licensed under [Apache License 2.0](LICENSE); see [NOTICE](NOTICE).

Third-party components retain their own copyrights and licenses. See [license notices](licenses/README.md) for their distribution locations, generation instructions, and supplements.
