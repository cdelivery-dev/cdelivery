# Tests

Use macOS with OrbStack, Rust, Node.js and npm. Start OrbStack, then run the following from the repository root. All `cargo xtask` commands also work from repository subdirectories and use the same repository root. Missing dependencies are downloaded; warm runs reuse prepared machines, upstream images, BuildKit caches and registry layers.

```sh
cargo xtask env up single
cargo test --locked -p tests --test system -- --test-threads=1
npm --prefix tests/ui ci --prefer-offline
npm --prefix tests/ui exec -- playwright install chromium
npm --prefix tests/ui test
cargo xtask env down

cargo xtask env up cluster
cargo test --locked -p tests --test system -- --include-ignored --test-threads=1
npm --prefix tests/ui test
cargo xtask env down

cargo xtask env up storage
cargo test --locked -p insight --test storage -- --test-threads=1
cargo xtask env down
```

`single` clones one prepared Ubuntu 24.04 machine with Local Insight storage; `cluster` clones three with Garage S3Compatible storage. Both build five deployment binaries using production release profiles and record unique image tags and digests. `storage` starts only Garage. Compose waits for its bucket healthcheck before marking the environment ready. Install npm dependencies again only when the lockfile changes; Playwright reuses its browser download.

The `system` target reads the current topology. `insight/tests/storage.rs` exercises the production Store directly. Local durability needs no running environment:

```sh
cargo test --locked -p insight --test storage local_durability -- --test-threads=1
```

API, MCP and browser tests use localhost:19000, forwarded to the manager's CDelivery port 9000. Node HTTP/HTTPS probes use `orb` and the nodes' own ports 80/443; published-port checks run from the manager to each node. ACME reaches port 80 directly. Garage, Pebble, registry and release services use their declared dependency ports.

Prepared machines supply curl 7.88 or newer with OpenSSL and HTTP2. Node probes disable user configuration and proxies, enforce a 10-second timeout, and check the body, negotiated HTTP version and peer certificate. Each node's latest probe records the command, exit status and raw stdout/stderr in `curl-{machine}.txt`; missing or invalid JSON and conflicting exit codes fail immediately. Test CAs remain in each attempt's results and reach the nodes through OrbStack file sharing. Only the missing/unknown SNI rejection probes disable client certificate validation; they must receive a TLS server alert, followed by a successful verified request.

`xtask env` manages infrastructure; Cargo and Playwright run assertions. Store tests stay with Insight. The Rust targets have `test = false`, so ordinary `cargo test` does not launch them. Use Cargo's name filter to rerun a case:

```sh
cargo test --locked -p tests --test system https_manual -- --test-threads=1 --nocapture
cargo test --locked -p tests --test system https_acme -- --test-threads=1 --nocapture
```

Node recovery has `ignore = "requires cluster"`. Cluster runs need `--include-ignored`; forcing it on a single node fails its precondition. Select it alone with `cargo test --locked -p tests --test system node_failure_and_recovery -- --ignored --test-threads=1`.

Run one environment or test command at a time, with one test thread. `up` refuses an existing environment. Changes to deployed code, dependencies or provisioning require `down` then `up`; assertion changes can run against the current environment. HTTPS uses reserved `*.cdelivery.test` names without changing Mac trust or hosts settings.

| Cases | Checks |
| --- | --- |
| Setup and authentication | Repeated setup, host administrator recovery, Cookie/API key roles, live role changes, password invalidation, revocation and Own isolation |
| REST and MCP | Catalog consistency, authorization before decoding, unknown fields, validation errors, patch omission/null/value and unchanged stored versions |
| Apps | Deploy, replicas, rollback, replacement tasks, XConfig binary downloads, volume persistence, creation/deletion conflicts and idempotent deletion, XHealth/Hurl manual runs, concurrent starts, interval reset and restart recovery, Guardian failure and promotion, XJob intervals, timeouts, concurrent starts and restart recovery |
| HTTPS | TLS 1.2/1.3 with HTTP/1.1 and HTTP/2, manual certificates and hot replacement, expired/mismatched certificates, missing/unknown SNI rejection, gateway survival, HTTP-01, DNS-PERSIST-01, ARI renewal and certificate distribution to all gateways |
| Insight | Real application logs, access records, audit events and node/container metrics; immutable storage backend |
| Storage | Production Store against disk/Garage, row counts and checksums, ordinary PUT, files over 96 MiB and multipart ETags, outage/backfill, degraded startup, remote readers and recovery into an empty local directory |
| Cluster | Membership, reusable/revoked invitations, cross-node published ports, worker failure, rescheduling and rejoin |
| Browser | Native form validation, group/app lifecycle, XHealth manual runs and XJob forms in English and Simplified Chinese, eight account languages |

| Lifetime | Location | Cleanup |
| --- | --- | --- |
| Prepared bases and installed versions | `target/tests/cache/` | `env clean` removes recorded base machines |
| Uploaded image layers | External registry volume | `env clean` removes this project-owned volume |
| Runtime manifest, credentials and release catalog | `target/tests/current/` | `env down` removes the current runtime and its disposable data |
| Attempt results, timings, manifests, logs and browser traces | `target/tests/results/` | Retained across `up`, `down`, `clean` and reruns; remove explicitly when no longer needed |

The base contains Docker without deployment or test data. Distro, architecture and installation script determine its cache identity; a changed identity or missing machine requires preparation. A remote systemd unit retains interrupted preparation and downloads: after `down`, the next `up` reconnects or retries. Sealed bases are stopped and identity-free; clones must have distinct machine and Docker identities. Port configuration runs after cloning.

`current/env.json` records ownership and expected configuration; OrbStack, Docker and live responses determine availability. Rust and Playwright read the same manifest and credentials. Release files are derived from recorded images. `down` preserves registry layers and results, and removes disposable Garage/Pebble data.

Every attempt has separate results; failures retain the environment. Provisioning failures capture dependency logs and container health details. Commands stream long output and terminate their process group on timeout; base preparation remains owned by systemd. Fault cases collect evidence before restoring nodes or Garage, including after timeout, and record restoration failures separately. Reruns reset fixed TLS fixtures and browser language.

`down` removes only recorded runtime resources and uniquely tagged images. `cargo xtask env clean` requires no active environment and removes recorded bases and the owned registry cache. Neither prunes shared BuildKit caches, upstream images or results.

Durability assertions start after a successful Store flush or verified remote upload. They do not promise preservation of an in-memory ingest buffer after a forced kill. Multi-instance queries require the current S3 inventory and fail during an outage; local flushes continue, and recovery checks the complete row count and checksum. OrbStack machines share a kernel: worker interruption checks process/node recovery, not independent physical-kernel failures. Public CA/cloud services, public installation, manager quorum failures, external IM/LLM services and performance benchmarks are outside these targets.

`Cargo.lock` temporarily selects `boring` and `boring-sys` 5.2.0 for Pingora 0.9.0. Version 5.3.0 crashes when Pingora clears its certificate callback; [the upstream fix](https://github.com/google/boringssl/commit/2752c13d111a7849d7396d964ecae9c4cda7f05b) is not included in that release. Upgrade both packages only to a release containing the fix and rerun HTTPS on single and cluster. The workspace Tokio `~1.53` constraint separately avoids Pingora's zero-capacity buffer panic; lift it after adopting an official Pingora fix and rerunning HTTP/HTTPS acceptance. Builds and acceptance use `--locked`; an explicit dependency update still requires these checks. Production polling intervals remain unchanged.

Before submitting changes:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked -p tests --test system -p insight --test storage -- -D warnings
cargo xtask comments
cargo xtask imports
cargo xtask locale
cargo xtask ui
```

`locale` checks translation keys, all eight languages and matching placeholders; key order is unrestricted. `ui` checks localized JavaScript, required assets and deferred script loading; it does not enforce inline-style or theme-color rules.

Run `cargo xtask loc` when you need production/test line counts by crate. It reports counts without enforcing a size limit. Use `cargo xtask --help` to list commands.

Image building outside these tests lives in `scripts/build-image.sh`; the production Dockerfile remains at the repository root.
