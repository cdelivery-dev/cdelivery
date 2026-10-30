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
  <a href="https://cdelivery.dev">Website</a> ·
  <a href="https://cdelivery.dev/docs/">Documentation</a> ·
  <a href="#get-started">Getting started</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square" alt="License: Apache-2.0"></a>
</p>

## Features

- **Deploy and scale.** [Run container images](https://cdelivery.dev/docs/guides/apps/) on your own servers, starting with one machine and adding more as your applications grow.
- **Release with confidence.** [XHealth](https://cdelivery.dev/docs/guides/release-safety/xhealth/) checks business workflows; [Guardian](https://cdelivery.dev/docs/guides/release-safety/) validates candidate releases before they receive user traffic and blocks promotion on failed checks. Guardian requires XHealth and applications without local data volumes.
- **Logs and metrics.** Enable and configure [Insight](https://cdelivery.dev/docs/guides/observability/) to collect and query server and application metrics, application logs, and gateway access records, using local disk or object storage.
- **HTTPS and load balancing.** [Pingress](https://cdelivery.dev/docs/guides/networking/) routes requests, balances traffic across instances, and manages HTTPS certificates automatically, with support for multiple gateways.
- **AI operations.** Configure a model provider for [AI diagnostics and proactive duty checks](https://cdelivery.dev/docs/guides/automation/), with actions limited by the active role. Connect your own tools through [MCP and REST](https://cdelivery.dev/docs/guides/automation/api-mcp/).
- **Configuration and scheduled jobs.** [XConfig](https://cdelivery.dev/docs/guides/apps/xconfig/) distributes configuration files to applications without local data volumes; [XJob](https://cdelivery.dev/docs/guides/automation/xjob/) runs commands on a schedule or on demand in fresh containers.

## Get started

Prepare your server using the [installation guide](https://cdelivery.dev/docs/getting-started/installation/), then run:

```sh
curl -fsSL https://cdelivery.dev/install.sh | sh
```

On first installation, choose a stable communication address that your other servers can reach; `127.0.0.1` is for a single server only.

Open `http://SERVER_IP:9000`, create your first administrator account, and sign in. Make port `9000` reachable from your management network.

The guide also covers address options and macOS setup. Next, [deploy your first application](https://cdelivery.dev/docs/getting-started/first-app/).

## Documentation

Read the [documentation](https://cdelivery.dev/docs/) for installation, application management, networking, observability, and automation. Guides include setup steps, verification, limitations, and troubleshooting.

[Platform management](https://cdelivery.dev/docs/administration/) covers servers, accounts, permissions, upgrades, and recovery. For external tools, see [API and MCP access](https://cdelivery.dev/docs/guides/automation/api-mcp/).

The interface and documentation support English, Simplified Chinese, Traditional Chinese, Japanese, Brazilian Portuguese, Spanish, French, and German.

## Performance

### Starting memory

Memory usage under low load:

| Component | Role | Memory |
| --- | --- | ---: |
| CDelivery | Core service | 30 MiB |
| Pingress | Reverse proxy gateway | 30 MiB |
| Observability | Data collection and storage | 150 MiB |

### Benchmarks

| Component | Throughput | Peak memory | Test environment |
| --- | ---: | ---: | --- |
| Insight | 2,491 MB/s ingestion | 2.21 GiB | AWS c8gb.2xlarge, Graviton4, 8 cores, 16 GiB RAM, gp3 |
| Pingress | 45,982 requests/s | 963 MiB | AWS c8gn.large, Graviton4, 2 cores, 4 GiB RAM |

Insight figures come from the [October 17, 2026 measurement record](website/src/data/insight-benchmark.json). Actual resource usage and throughput vary with workload and configuration.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) to contribute and [SECURITY.md](SECURITY.md) to report vulnerabilities privately.

## License

Copyright 2026 The CDelivery Authors. First-party source code, documentation, scripts, and tests are licensed under [Apache License 2.0](LICENSE); see [NOTICE](NOTICE).

Third-party components retain their own copyrights and licenses. See [license notices](licenses/README.md) for their distribution locations, generation instructions, and supplements.
