# CDelivery

**Safer deployments. Simpler operations. More time for what matters to you.**

CDelivery is a self-hosted operations platform for running applications on your own servers. It brings deployment, business workflow checks, logs and metrics, HTTPS, configuration distribution, and AI assistance into one place.

Start with one server and add more as your applications grow. Spend less time integrating and maintaining infrastructure tools, and more time building your product.

[Website](https://cdelivery.dev) · [Documentation](https://cdelivery.dev/docs/) · [Getting started](https://cdelivery.dev/docs/getting-started/introduction/)

## Get started

Run the installation command on your server:

```sh
curl -fsSL https://cdelivery.dev/install.sh | sh
```

Open `http://SERVER_IP:9000`, create your first administrator account, and sign in. Allow access to port `9000` in your server's firewall or security group.

On first installation, always choose the server communication address from a numbered list. Private IPv4 addresses appear first; `127.0.0.1` is always available for a single server. To skip the menu and specify the address:

```sh
curl -fsSL https://cdelivery.dev/install.sh | sh -s -- --advertise-addr 192.168.1.10
```

Use a stable address that your other servers can reach. Existing installations retain their current communication address. On macOS, start OrbStack or Docker Desktop first, run the same installation command, then open `http://localhost:9000`.

Follow the [installation guide](https://cdelivery.dev/docs/getting-started/installation/), then [deploy your first application](https://cdelivery.dev/docs/getting-started/first-app/).

## Operate and release with confidence

**XHealth** runs scheduled checks against critical business workflows, such as user login, so you can verify that your application works beyond simply responding to a request.

Enable **Guardian** to validate a candidate version in an isolated environment before release. The validation instance receives no user traffic, and failed checks block the release. The same validation can cover application configuration changes.

Guardian requires XHealth and is available for applications without local data volumes. Learn more about [health checks](https://cdelivery.dev/docs/guides/release-safety/xhealth/) and [release validation](https://cdelivery.dev/docs/guides/release-safety/).

## Built-in observability

**Insight** collects server and application metrics, application logs, and access records for requests routed through Pingress. Choose local disk or object storage such as S3, enable collection, and investigate your applications without integrating an SDK.

Search logs by source and time range, inspect resource usage, and correlate issues with operation events. Collection, storage, and querying are part of the platform, with support for multiple instances and high availability.

## HTTPS and load balancing

**Pingress** is CDelivery's reverse proxy gateway, built on Cloudflare's Pingora. It routes requests to your applications, balances traffic across instances, and manages HTTPS certificates automatically.

Run a single gateway or multiple instances for high availability, with automatic certificate management in either setup.

## AI diagnostics and proactive monitoring

The built-in **AI assistant** uses runtime metrics, application logs, access records, and operation events to help diagnose issues and carry out operations within the user's permissions. Connect your model provider and messaging channel, or use your preferred AI tools through **MCP** and the **REST API**.

Enable **AI duty** for proactive checks driven by change events and a schedule. Configure its role and notification policy to control how it investigates problems, acts, and notifies you.

## Configuration and routine automation

**XConfig** distributes configuration files to application instances without an SDK or a separate configuration service. Upload a file, choose its path inside the container, and update it through CDelivery. With Guardian enabled, new configurations pass release validation before going live. XConfig cannot be combined with local data volumes in the same application.

**XJob** runs application commands on an interval or calendar schedule, with manual runs when needed. Each run uses a fresh container with the application's image, and its result is available in the dashboard.

## Small footprint

CDelivery keeps deployment management, the gateway, and observability lightweight, leaving more resources for your applications.

The homepage reports the following starting memory usage under low load:

| Component | Role | Memory |
| --- | --- | ---: |
| CDelivery | Core service | 30 MiB |
| Pingress | Reverse proxy gateway | 30 MiB |
| Observability | Data collection and storage | 150 MiB |

Reported benchmarks include Insight throughput of **1,305.96 MB/s** with **5.96 GiB** peak memory on an M4 Mac, and Pingress peak throughput of **48,000 requests/s** on an AWS m7a.large. Actual resource usage and performance vary with workload and configuration.

## Explore the documentation

- [Deploying and running applications](https://cdelivery.dev/docs/guides/apps/)
- [Configuration files](https://cdelivery.dev/docs/guides/apps/xconfig/) and [data persistence](https://cdelivery.dev/docs/guides/apps/persistence/)
- [Domains and HTTPS](https://cdelivery.dev/docs/guides/networking/)
- [Logs and metrics](https://cdelivery.dev/docs/guides/observability/)
- [AI operations assistant](https://cdelivery.dev/docs/guides/automation/) and [API and MCP access](https://cdelivery.dev/docs/guides/automation/api-mcp/)
- [Scheduled tasks](https://cdelivery.dev/docs/guides/automation/xjob/)
- [Accounts, permissions, and platform management](https://cdelivery.dev/docs/administration/)

The interface and documentation support English, Simplified Chinese, Traditional Chinese, Japanese, Brazilian Portuguese, Spanish, French, and German.

If CDelivery is useful to you, [give it a star on GitHub](https://github.com/cdelivery-dev/cdelivery).
