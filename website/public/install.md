# Install and configure CDelivery with an AI assistant

Use this guide to help the user install CDelivery, complete administrator setup, configure HTTPS access and Insight, and connect your current AI tool through MCP. Communicate in the user's language. Read the whole guide before changing the target environment.

Continue through the authorized installation and configuration work. Ask for missing information and consequential choices, reuse answers already provided, and respect the permissions of your execution environment. Do not report completion until the corresponding checks pass.

## 1. Establish where each action runs

There are two environments, which may be different machines:

- **Installation target:** the Linux server or Mac where CDelivery will run. Run installation, Docker inspection, and administrator setup here.
- **AI client environment:** the machine or service running the user's current AI tool. Write MCP configuration here, and test connectivity from here.

Determine the target from the conversation. If it is unclear, ask whether to install locally or on a remote server. For a remote target, obtain its SSH host or existing SSH alias and login user. Reuse existing SSH configuration and credentials; do not ask the user to paste a private key into the conversation. Connect to the target and verify its identity before installing. A remote Docker context is not a substitute: the installer requires a local Unix socket on the target.

Inspect the current AI tool's identity, version, configuration scope, and available execution capabilities. If you cannot execute commands or edit its configuration, identify exactly which steps require the user's help and continue with the steps you can perform. A cloud client may be unable to reach a private server or a tunnel on the user's laptop.

Ask about the server's intended use when it affects address selection: a single server, or multiple servers that need to communicate. For a fresh administrator account, collect:

| Field | Requirement |
| --- | --- |
| `first_name` | Required, 1–64 UTF-8 bytes |
| `last_name` | Required, 1–64 UTF-8 bytes |
| `email` | Required, valid email, at most 254 UTF-8 bytes |
| Password choice | User supplies a password, or you generate one |

Do not invent personal details. If the user chooses a generated password, use a cryptographically secure generator and a unique password of at least 20 characters. Both supplied and generated passwords must contain at least eight characters, an ASCII lowercase letter, an ASCII uppercase letter, and a digit, with no line breaks. Deliver a generated password to the user through an available private credential channel, or clearly in the direct conversation if no such channel exists. It is a persistent password, not an automatically expiring password.

Pass passwords through process stdin or a request body. Do not put real passwords into shell command arguments, shell history, a repository, or diagnostic logs. The examples below contain placeholders only; use your tool's stdin support or another protected input mechanism for real credentials. A `password_sha256` value is also a credential and must receive the same protection.

If CDelivery is already initialized, preserve its accounts. Ask which existing account should own this AI tool's connection when the answer is missing. Do not reset a password or create another administrator as part of reconnecting an existing installation.

Confirm the optional platform configuration together with these installation choices: a domain for secure HTTPS access, and whether to enable Insight for logs and metrics. Recommend Insight with local storage and 90-day retention; the domain can be skipped. Reuse choices already supplied. Existing configuration takes precedence over these defaults unless the user requests a change.

## 2. Inspect and prepare the target

Read-only checks should establish:

- The operating system, architecture, SSH target identity, and available privileges.
- Whether Docker is installed, running, and reachable through the intended local context.
- Whether the `cdelivery` service already exists and is healthy.
- The server's IPv4 addresses and how the user and AI client will reach port `9000`.

Useful commands, when the corresponding tool is installed:

```sh
uname -s
uname -m
hostname
id
docker context show
docker info --format '{{.OSType}} {{.Architecture}}'
docker service ls
```

The installer supports Linux and macOS with an amd64 or arm64 Linux Docker daemon, version 28 or newer. Linux requires a rootful daemon at `/var/run/docker.sock`. On Ubuntu, Debian, Fedora, RHEL, CentOS Stream and Alpine, the installer installs Docker when it is missing; elsewhere, prepare a working Docker installation first. Downloading the installer itself also requires `curl` and working HTTPS trust; use the target's package manager or transfer the downloaded script over SSH if these are initially missing.

On macOS, use a running OrbStack or Docker Desktop installation. Reuse an existing runtime. If neither is installed, ask which one the user wants; the installer does not install or launch a desktop runtime automatically. For a user who chooses OrbStack and already has Homebrew:

```sh
brew install --cask orbstack
orb start
```

Use the same local Docker endpoint for all later Docker commands. Examples assume that `docker` already targets it and that the current user has sufficient privileges. Adapt them with the required `sudo` or `doas` invocation without losing the selected endpoint. Do not switch Docker contexts globally just to make an example work.

If the installer reports a conflicting package, occupied port, incompatible existing resource, or unsupported runtime, investigate that specific condition. Do not remove packages, workloads, networks, accounts, or stored data merely to make installation proceed.

## 3. Choose addresses and run the installer

Keep these addresses distinct:

| Address | Purpose |
| --- | --- |
| Server communication IPv4 | Passed to `--advertise-addr`; other servers must be able to reach it |
| Dashboard base URL | The URL the user can open in a browser |
| MCP base URL | The URL reachable from the AI client environment |

For a fresh installation, select a stable IPv4 address belonging to the target. If several interfaces, VPNs, or intended networks are plausible, show the relevant choices and ask the user. `127.0.0.1` is permitted for a single-server installation only. The parameter accepts an IPv4 address, not a hostname or interface name.

Existing installations retain their communication address. Do not supply a different `--advertise-addr` to an existing cluster: the installer rejects attempts to change it. Re-running the installer checks an existing installation; it does not upgrade or replace it.

Download fully before executing. Run this on the installation target, replacing the example address for a fresh installation:

```sh
(
  set -eu
  umask 077
  install_dir=$(mktemp -d "${TMPDIR:-/tmp}/cdelivery-ai-install.XXXXXX")
  trap 'rm -rf "$install_dir"' 0
  trap 'exit 130' INT
  trap 'exit 143' HUP TERM
  curl --proto '=https' --proto-redir '=https' -fsSL \
    --connect-timeout 15 --max-time 120 \
    https://cdelivery.dev/install.sh -o "$install_dir/install.sh"
  sh "$install_dir/install.sh" --advertise-addr 192.168.1.10
)
```

For an existing installation, omit `--advertise-addr 192.168.1.10`. For a new installation, always provide the chosen address when no interactive terminal is available; otherwise use Up/Down and Enter to select through `/dev/tty`. Limited terminals fall back to numbered input. The IPv6 fallback menu uses the same controls and defaults to exiting so the reported problem can be resolved.

Set `NO_COLOR=1` in the installer's environment to disable menu colors and bold styling while keeping arrow-key navigation.

On a Linux host with IPv6, the installer prepares an IPv6 gateway so Pingress receives original IPv6 client addresses. If it cannot do so safely, it explains why and stops before changing the server's cluster state. Without an interactive terminal, report that reason to the user and let them choose: resolve it and rerun, or rerun with `--no-ipv6` to keep IPv4 only, in which case IPv6 clients appear as the gateway address.

The script selects the release image, checks prerequisites, creates the service, and waits up to 180 seconds for the setup or login page to respond. Confirm that the service has a running task and record its exact image reference, including the digest when present:

```sh
docker service ps cdelivery --no-trunc
docker service inspect cdelivery --format '{{.Spec.TaskTemplate.ContainerSpec.Image}}'
```

On failure, inspect the retained service rather than starting a second deployment:

```sh
docker service ps cdelivery --no-trunc
docker service logs --tail 100 cdelivery
```

Review diagnostics before including them in the conversation; never reproduce credentials. A readiness timeout can leave a deployment that is still starting. Check the actual service and HTTP state before deciding to retry.

### Reach the dashboard securely

The initial dashboard uses port `9000`; macOS normally exposes it at `http://localhost:9000`. The printed address may be a server communication address that is unreachable from the user's device. Check reachability separately from the target, the user's device when possible, and the AI client environment.

Keep initial setup and credentials on loopback, an SSH tunnel, or an existing trusted HTTPS endpoint. Restrict access to the management port using the server's network controls. Choosing `127.0.0.1` as the communication address does not restrict the published dashboard port to loopback. Do not resolve a connection failure by opening that port to the whole Internet.

For a remote server, this command runs on the machine that will use the connection:

```sh
ssh -N -o ExitOnForwardFailure=yes \
  -L 127.0.0.1:19000:127.0.0.1:9000 SERVER_SSH_ALIAS
```

While this SSH process is running, use `http://127.0.0.1:19000` on that machine. Choose another available local port when necessary. A tunnel on one machine does not make the same URL reachable from a different machine or a cloud client. A temporary tunnel is not a persistent connection; document how it must be kept running or use an existing durable private or HTTPS route.

## 4. Complete administrator setup

Inspect `GET /setup` without automatically following redirects. On the target:

```sh
curl --noproxy '*' --silent --show-error --include \
  --connect-timeout 5 --max-time 15 http://127.0.0.1:9000/setup
```

- `303` with a location ending in `/setup/platform` means the initialized application is serving. Skip account creation. This destination requires an administrator session; without one, it redirects to `/login`.
- `200` means a setup page, not necessarily a new installation. Its `data-setup-state` is `NeedsSetup`, `NeedsResume`, or `Provisioning`: respectively, create the administrator, continue through `/setup/resume`, or wait for startup. The page includes hidden forms for these states; use the state marker to determine the available action.
- On a provisioning page, wait for startup before attempting another write.

### Linux: use the setup CLI in a separate container

Set `IMAGE` to the exact deployed image reference recorded above. Run on the target's Linux host:

```sh
docker run --rm -i --network host "$IMAGE" setup
```

Send one JSON object to its stdin, then close stdin:

```json
{
  "first_name": "Alex",
  "last_name": "Example",
  "email": "admin@example.com",
  "password": "REPLACE_WITH_THE_CHOSEN_PASSWORD"
}
```

All four fields are required and unknown fields are rejected. Use a JSON encoder for actual values. The CLI validates the password and hashes it before submitting to `http://127.0.0.1:9000/setup`. This temporary container needs no Docker socket, key mounts, or shell. Do not run `setup` inside the service container: initialization replaces that container, which can interrupt the command reporting its result. On macOS, use the HTTP method below instead of relying on container host networking.

### macOS or direct HTTP: submit the setup form

Calculate lowercase hexadecimal SHA-256 of the exact UTF-8 password bytes, without a trailing newline, and put the resulting 64-character string in `password_sha256`. Enforce the password rules above before hashing; this HTTP boundary only receives the digest.

```sh
curl --noproxy '*' --silent --show-error --include --max-time 45 \
  --header 'Origin: http://127.0.0.1:9000' \
  --header 'Accept: application/json' \
  --data-urlencode payload@- \
  http://127.0.0.1:9000/setup
```

Provide the following JSON through stdin, using the real digest in place of the placeholder:

```json
{
  "first_name": "Alex",
  "last_name": "Example",
  "email": "admin@example.com",
  "password_sha256": "REPLACE_WITH_64_LOWERCASE_HEX_CHARACTERS"
}
```

The request is `POST /setup` with `Content-Type: application/x-www-form-urlencoded`; its only form field is `payload`, containing the encoded JSON object. A raw JSON request body is not accepted. `Accept` must be exactly `application/json` to select the JSON response. `Origin` must match the request origin, including scheme and any non-default port, without a trailing slash. For the example SSH tunnel, change both the URL and `Origin` to `http://127.0.0.1:19000`. Do not disable TLS verification for an HTTPS endpoint.

### Interpret the result and wait for startup

Successful setup returns HTTP `202`, `Content-Type: application/json`, and one of:

```json
{"status":"Initialized"}
```

```json
{"status":"Resumed"}
```

The CLI prints this JSON to stdout and exits `0`. `Initialized` means the submitted administrator was saved and startup was submitted. `Resumed` means existing initialization was resumed, preserving the previously saved account and password; the submitted account details are ignored.

Errors use `application/problem+json`. For example, HTTP `409`:

```json
{
  "type": "about:blank",
  "title": "Conflict",
  "status": 409,
  "code": "AlreadyInitialized",
  "detail": "already initialized"
}
```

`AlreadyInitialized` changes nothing and may mean startup is still in progress. Validation errors return `422` and include field messages in `errors`; malformed requests return `400`, and an invalid origin returns `403`. The CLI exits nonzero and writes server errors to stderr; local diagnostics are English text. Requests are limited to 64 KiB.

For a page explicitly showing resumable setup, you may use the existing resume form without collecting another password: `POST /setup/resume` with the same matching `Origin`, form field `payload={}`, and no JSON-response expectation. Success redirects with `303` to `/setup`. Do not submit it repeatedly while provisioning.

After success, poll `GET /setup` every two seconds for up to three minutes. Wait for the redirect to `/setup/platform`, then confirm `GET /login` responds successfully. If the request times out or disconnects, inspect the page, service tasks, and logs before retrying. Preserve the original credentials and installation.

## 5. Issue a dedicated API key

First inspect the AI client's existing CDelivery configuration without printing stored secrets. Reuse a connection to the intended instance and user with suitable permissions, then verify access as described below.

For a new connection, use an identifiable name such as `AI tool name - workstation`. The default for this installation flow is role `Admin` with no expiry, so the AI can manage the platform. Explain this access level as part of configuration; use a narrower role or an expiry when the user requests it. A key cannot exceed its owner's current role. An existing non-admin account cannot issue an Admin key; do not promote it automatically.

### Locate the running container again

Initialization replaces the service task. After login is ready, locate the current task on a manager:

```sh
docker service ps cdelivery --filter desired-state=running --no-trunc
docker service ps cdelivery --filter desired-state=running --quiet
```

For each candidate task ID, inspect its actual state, node, and container ID:

```sh
docker inspect --type task "$TASK_ID" \
  --format '{{.Status.State}} {{.NodeID}} {{.Status.ContainerStatus.ContainerID}}'
docker node inspect self --format '{{.ID}}'
```

Use the current task in state `running`, not a pending or historical task. If several candidates remain during a transition, wait for the service to settle. If its node ID differs from the current node, identify and connect to that node before `docker exec`; container IDs are local to a node. On macOS, run Docker commands through the same runtime context. Inspect the chosen container before issuing the key:

```sh
docker container inspect "$CONTAINER_ID" --format '{{.State.Running}} {{.Config.Image}}'
```

### Use the management CLI

Run directly through the image's entrypoint; the image does not provide a shell:

```sh
docker exec "$CONTAINER_ID" /entrypoint create-api-key \
  'admin@example.com' 'AI tool name - workstation' Admin
```

Replace the email with the intended existing account and the name with the current AI tool's name. The role token is exactly `Reader`, `Operator`, or `Admin`. Names must contain 1–64 UTF-8 bytes. Omit the expiry option for the default non-expiring key; when requested, append `--expires-at` followed by a future RFC 3339 timestamp, for example `2030-01-01T00:00:00Z`.

Success prints only the new `cdk_...` key followed by a newline to stdout. Capture it directly for credential storage or client configuration, keeping it out of diagnostic output and the final summary. The command requires access to the initialized service container and its existing key material; running it in an empty temporary container is insufficient.

Every successful invocation creates a new key. Names are not unique, and this operation is not idempotent. If execution disconnects or output is lost, do not blindly rerun it. Inspect account key metadata through an already authenticated session or key, identify any newly created record using its ID and available evidence, and revoke an orphan only when you can establish that it belongs to this attempt. Ask the user to resolve any ambiguity before revoking an existing credential. If no authenticated access is available, have the user sign in and inspect **Account → MCP / API keys**.

### REST alternative and key management

An already authenticated account can manage its own keys through REST. Use an existing Bearer key or a signed-in browser session. The CLI above avoids needing to automate a browser login during first installation.

```http
POST /api/apikeys HTTP/1.1
Authorization: Bearer EXISTING_API_KEY
Content-Type: application/json

{"name":"AI tool name - workstation","role":"Admin","expires_at":null}
```

Send this request to the chosen instance over the protected connection. `role` must be supplied as `Admin` for this flow: the REST default when omitted is `Reader`. Omitted or `null` `expires_at` means no expiry; a supplied timestamp must be in the future. Unknown input fields are rejected. Cookie-authenticated writes also require a matching `Origin`.

HTTP `201` returns this shape, with actual values in place of the examples:

```json
{
  "id": "00000000-0000-4000-8000-000000000001",
  "name": "AI tool name - workstation",
  "key": "cdk_REDACTED",
  "created_at": "2026-10-09T00:00:00Z",
  "expires_at": null,
  "role": "Admin"
}
```

- `GET /api/apikeys` returns `200` and `{"keys":[...]}`. Each record has `id`, `name`, `created_at`, `expires_at`, and `role`; it never includes the secret key.
- `DELETE /api/apikeys/KEY_ID` revokes that account's key by UUID and returns `204` with no body. Repeating the deletion is safe. Do not identify a key to revoke by name alone.
- A new secret is returned only once. Password changes do not revoke API keys; users manage them separately, and deleting the owning account invalidates its keys.

### Verify API access

From the AI client environment, send this request over the intended protected connection using the new or reused key:

```http
GET /api/me HTTP/1.1
Authorization: Bearer API_KEY
Accept: application/json
```

Expect HTTP `200` and the intended account identity. A `401` indicates missing or invalid authentication; a `403` indicates insufficient access for the attempted operation. The response includes `id`, `first_name`, `last_name`, `email`, `role`, `totp_enabled`, and `preferences`. The effective role is the lower of the account role and the key's role ceiling; check key metadata for the latter. Pass authentication headers through the client's protected request mechanism or stdin-supported tooling to keep the token out of process arguments.

## 6. Configure the platform

Use the verified Bearer key over the protected management connection. REST works before the new MCP connection is configured; an already connected client can use the equivalent MCP operations below. Read actual platform state before deciding what is missing:

| REST | MCP | Read |
| --- | --- | --- |
| `GET /api/instance` | `instance_get` | Existing platform domain and controller version |
| `GET /api/pingress` | `pingress_get` | Gateway placement, version and `acme_enabled`; `404` with `PingressNotEnabled` means absent |
| `GET /api/insight` | `insight_overview` | Existing `server` and `agent`, their versions and configuration |
| `GET /api/nodes` | `node_list` | Node IDs, names, addresses, state and availability |

For a new placement, select a node with `state: "Ready"` and `availability: "Active"`. Select it automatically if exactly one qualifies; ask the user to choose by name and address if several qualify, or leave the step pending if none do. Gateway placement and Insight storage placement are separate choices. Local Insight data stays on its selected node, and its placement cannot be changed after enabling it.

When a component is missing, read `GET /api/versions` (`version_list`) and use its `components.pingress.version`, `components.insight.version` or `components.insight_agent.version` as applicable. Preserve installed versions; this setup flow does not imply an upgrade. Substitute actual values for the placeholders below.

### Domain

Skip this part if no domain was requested. Keep an existing domain unless the user explicitly requests replacing it; report a mismatch instead of overwriting it. Use a hostname, without a scheme, port or path. Its DNS records must point to the reachable ingress server, with ports 80 and 443 open. Preserve the current management connection until HTTPS is verified. On an existing installation, check that an application's domain will not be taken over by the platform domain.

Reuse an existing gateway and its certificate policy. Only when it is absent, send `PUT /api/pingress` (`pingress_enable`):

```json
{
  "version": "<components.pingress.version>",
  "placement": { "NodeId": "<ingress-node-id>" },
  "acme": "LetsEncrypt"
}
```

HTTP `201` returns the gateway configuration. Creation is not idempotent: after an uncertain response, read `/api/pingress` before trying again. Do not use `acme: null` as a placeholder; it disables automatic certificates and clears the ACME account and its certificates.

For an existing gateway with `acme_enabled: false`, check `GET /api/pingress/certs` (`acme_list_certs`) for a valid certificate for the requested domain. If one is missing, obtain a certificate from the user or their agreement to enable automatic certificates; otherwise leave domain configuration pending. Preserve existing manual certificates and CA choices.

Then send `PATCH /api/instance` (`instance_update`):

```json
{ "domain": { "domain": "ops.example.com", "https": true } }
```

HTTP `202` has no response body. Read back `/api/instance` to confirm the saved domain.

### Insight

Proceed only if the user agreed to enable Insight. Apply the domain change first: enabling Insight can restart the controller and gateway to connect their logs. If domain setup failed, continue the independently authorized Insight setup and report the domain failure separately. Authentication or target-identity failures must be resolved before any further writes.

For a first enable with both components absent, send `PATCH /api/insight` (`insight_enable`):

```json
{
  "storage": "Local",
  "retention_days": 90,
  "placement": { "NodeId": "<insight-node-id>" },
  "server_version": "<components.insight.version>",
  "agent_version": "<components.insight_agent.version>"
}
```

HTTP `200` returns the configuration. Storage, retention, placement and the missing components' versions are required for first setup; an empty request does not apply the UI's defaults. Omit sampling and flushing settings to use the platform defaults.

If either component already exists, send only the missing component's fields. An existing server keeps its storage, placement, retention and version; an existing agent keeps its version and sampling interval. Do not replace object storage with `Local` or resubmit read-back configuration containing omitted secrets. If both components exist but a previous attempt was interrupted, `PATCH /api/insight` with `{}` reconciles their platform log connections without resetting their settings. An already healthy installation needs no write.

### Recovery and verification

After a disconnect, wait for the management API, then read the domain, gateway and Insight state again before retrying missing work. Keep successful changes; do not roll them back because another step failed. Use the existing two-second polling interval with a three-minute deadline per readiness check, and report unresolved readiness as pending.

- **Runtime:** confirm `ready: true` for the requested gateway, Insight server and agents in `GET /api/overview` (`platform_overview`).
- **HTTPS:** from the AI client environment, reach the configured domain with normal certificate verification and confirm the intended instance with authenticated `GET /api/instance`.
- **Insight:** confirm fresh data with `GET /api/nodes/NODE_ID/metrics` and `GET /api/instance/logs`.

Use the verified HTTPS base URL for the next step. If domain setup was skipped or remains pending, retain the reachable private or tunneled management URL.

## 7. Configure your current AI tool's MCP connection

Use your actual client's current help or official documentation to find its native remote MCP configuration. Inspect the installed version and existing configuration before editing. Do not apply another client's configuration shape or guess a filename based on the operating system.

Use these connection parameters:

| Parameter | Value |
| --- | --- |
| Connection name | `cdelivery`, or an instance-specific name if that is already used |
| Endpoint | The client-reachable base URL followed by `/mcp` |
| Transport | Remote HTTP MCP with stateless protocol support |
| Authentication | `Authorization: Bearer API_KEY` |
| Protocol version | `2026-07-28` |
| Default scope | Current user's client configuration |

The server uses discovery and stateless requests for protocol `2026-07-28`. It does not support the legacy `initialize` handshake or older protocol versions. A client advertising generic MCP or HTTP support may still be incompatible. Verify its version and protocol capabilities from official documentation or an actual connection. Do not add a proxy, downgrade the server, or change the platform protocol to hide an incompatibility; report the exact blocker.

Prefer a native credential store or a supported environment-variable reference. Ensure any referenced variable is available to the actual client process after reload; a variable in an unrelated shell is insufficient. If the client only supports a literal key, place it in its private user configuration with appropriate file permissions. Never place credentials in a project repository. Preserve unrelated configuration and MCP connections, and never silently replace a connection to another instance.

Perform the client's supported reload or reconnect action. Use its loaded connection to retrieve the tool list, then call `me_get` and `instance_get` with `{}`. Confirm successful results for the intended account and instance. These read-only calls complete connection testing. If a user-driven restart is required, report the connection as awaiting reload and verification.

## 8. Hand over

Finish with a concise report in the user's language containing:

- Installation target, deployed image, and dashboard URL or tunnel instructions.
- Administrator email, whether setup was new or resumed, and how the password was delivered.
- Domain and HTTPS status; Insight status, storage node or backend, and retention.
- AI client connection name, configuration location or native scope, key role, and expiry.
- Which checks actually passed, and any remaining user action such as restarting the client or maintaining a tunnel.

Keep API keys out of the report. Describe any pending step and its blocker precisely.

## References

- [Standard installation and setup](https://cdelivery.dev/docs/getting-started/installation/)
- [API and MCP access](https://cdelivery.dev/docs/guides/automation/api-mcp/)
- [Official installer](https://cdelivery.dev/install.sh)
- [Homebrew](https://brew.sh/)
- API reference for the installed version: open the instance's `/scalar` page after signing in, using the same protected base URL.

For version-specific schema inspection without relying on a browser, the installed entrypoint also supports `contract openapi` and `contract mcp`. Run it in the identified service container, for example `docker exec "$CONTAINER_ID" /entrypoint contract mcp`. These commands print contracts and do not create credentials. Consult them when an existing installation differs from the current guide.
