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

The `image_access_preflight` case serves a temporary Bearer-authenticated proxy on ports 19502 and 19503, backed by the environment's real registry. It checks pull scope, missing images, credential inheritance and clearing, registry failures, and rejection before config uploads or deployment.

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
cargo test --locked -p tests --test system upgrade_ -- --test-threads=1 --nocapture
```

Upgrade cases prepare two additional artifacts per component in the owned registry: a newer version built from the current binary with a distinct image digest, and an artifact that cannot start. They also exercise a nonexistent image tag. These cases verify the update mechanism, persisted state, automatic rollback for Insight and its agent, and paused failures with host rollback for CDelivery, Pingress and the volume agent; they do not establish compatibility with a historical release. Hurl uses the same versioned artifact procedure and verifies its saved file and actual runner digest.

`cargo xtask env upgrade prepare` records the artifacts in `env.json`, advertises their releases and restarts the controller to load the catalog through the normal startup path. `cargo xtask env upgrade restore` restores baseline service images and releases. Rust and browser upgrade cases call these commands automatically and restore after failures; run `restore` manually after a killed test process. The first preparation builds and pushes the extra images; subsequent cases reuse their recorded tags and digests. Upgrade polling allows the existing 60-second monitor per node to complete. Controller failures can temporarily interrupt the API; rollout status, task IDs and actual container image digests determine the outcome.

Service inspection in upgrade polling retries successful commands with empty or whitespace-only output within the existing 360-second deadline. The polling loop checks the deadline before starting another probe, including when failed probes return synchronously; an in-progress external command retains its own timeout. Each abnormal response appends its timestamp, command, exit status and raw stdout/stderr to `inspect-{service}.log`. Nonzero exit status and nonempty invalid JSON fail immediately. The original isolated empty-output failure remains unexplained; retrying does not establish its cause. Original evidence is under `target/tests/results/upgrade-volume-33cdbe7c81ff43edab80c9b972b0b6c5/`.

Node recovery and idle log connection recovery have `ignore = "requires cluster"`. Cluster runs need `--include-ignored`; forcing these cases on a single node fails their preconditions. Select them individually with `cargo test --locked -p tests --test system node_failure_and_recovery -- --ignored --test-threads=1` or `cargo test --locked -p tests --test system insight_idle_connection_recovery -- --ignored --test-threads=1`.

Idle connection recovery waits 315 seconds with one worker's log connection quiet and another worker producing logs every ten seconds. It requires the idle collector to observe the remote close, then checks three exact records from new applications on both workers and resumed output from the original quiet application. Component tasks must remain unchanged. Application output, collector logs and query results are retained before cleanup.

Run one environment or test command at a time, with one test thread. `up` refuses an existing environment. Changes to deployed code, dependencies or provisioning require `down` then `up`; assertion changes can run against the current environment. HTTPS uses reserved `*.cdelivery.test` names without changing Mac trust or hosts settings.

| Cases | Checks |
| --- | --- |
| Database templates | Versioned IDs, remote images and missing/malformed entries in the shared release catalog case; Reader REST/MCP agreement in protocol cases; browser creation of PostgreSQL and MariaDB, editable images, authentication and data persistence across container replacement |
| Setup and authentication | Repeated setup, host administrator recovery, Cookie/API key roles, live role changes, password invalidation, revocation, short credential expiry, Origin checks, corrupted user records, Own isolation, TOTP enrollment, replay, five-minute cooldown and disable |
| REST and MCP | Full catalog permission matrix with independent policy anchors, authorization before decoding, unknown fields, validation errors, patch omission/null/value and unchanged stored versions |
| Apps | Deploy, replicas, rollback, replacement tasks, XConfig binary downloads, volume persistence, creation/deletion conflicts and idempotent deletion, XHealth/Hurl manual runs, concurrent starts, interval reset and restart recovery, Guardian failure and promotion, XJob intervals, timeouts, concurrent starts and restart recovery |
| HTTPS | Pingress ACME opt-out, registration timeout and persistence failure, retained-account retry and CA replacement before deployment, uploaded-certificate preservation, TLS 1.2/1.3 with HTTP/1.1 and HTTP/2, manual certificates and hot replacement, expired/mismatched certificates, missing/unknown SNI rejection, gateway survival, HTTP-01, DNS-PERSIST-01, ARI renewal and certificate distribution to all gateways |
| Boundaries and lifecycle | Large XConfig byte preservation and size rejection, artifact ownership, job count limits, dependent runner cleanup, settings rejection without external traffic, IM binding round trips, invitation revocation, node label replacement, component busy checks, disable/re-enable and bounded automatic scaling |
| Routing | Prefix boundaries, combined header/cookie conditions, route precedence and deletion, platform domain precedence and clearing, certificate cache invalidation |
| Insight | Real application logs, access records, audit events and node/container metrics; immutable storage backend, query validation, combined filters, fixed-window pagination and app isolation; idle connection recovery with an active control and resumed application output |
| Duty alerts | Only alerts, recoveries and patrols invoke the model; immediate startup failure and recovery each notify once; AI success, disabled duty, model/tool failures, empty/NOOP replies, timeout, cancellation, role changes, write receipts and rejected delivery; each patrol case waits the real 15-minute minimum interval |
| Upgrades | CDelivery, volume agent, Pingress, Insight server and agent, combined Insight update and Hurl; REST/MCP/UI submission, distinct running digests, missing/broken images, pause and rollback, latest-version enforcement, no-op submissions, malformed catalogs, account/session/config/volume/certificate/log retention, post-upgrade job runs and collection |
| Storage | Production Store against disk/Garage, row counts and checksums, ordinary PUT, files over 96 MiB and multipart ETags, outage/backfill, degraded startup, remote readers and recovery into an empty local directory; out-of-order batches, restart, half-open time windows and pagination; buffered TCP ingest shutdown, schema adaptation through append/compaction/reopen, and compaction/upload/cleanup across a Garage outage |
| Cluster | Membership, reusable/revoked invitations, cross-node published ports, worker failure, rescheduling and rejoin |
| Browser | Native form validation, account/key/password flows, role restrictions, group/app lifecycle, XHealth manual runs, XJob forms and real controller upgrade in English and Simplified Chinese, navigation across eight account languages; optional first-setup navigation, platform-domain certificates, Insight defaults and partial-deployment recovery |

The release catalog case verifies that invalid templates are omitted with warnings while valid templates and component versions remain available; invalid top-level structure or component records reject the whole update. It restarts the controller to load each fixture through its normal startup path, without waiting for the hourly refresh or verifying retention of an in-memory snapshot across a failed scheduled refresh. Browser database cases pull official image variants and verify persisted rows after container replacement.

| Lifetime | Location | Cleanup |
| --- | --- | --- |
| Prepared bases and installed versions | `target/tests/cache/` | `env clean` removes recorded base machines |
| Uploaded image layers | External registry volume | `env clean` removes this project-owned volume |
| Runtime manifest, credentials and release catalog | `target/tests/current/` | `env down` removes the current runtime and its disposable data |
| Attempt results, timings, manifests, logs and browser traces | `target/tests/results/` | Retained across `up`, `down`, `clean` and reruns; remove explicitly when no longer needed |

The base contains Docker without deployment or test data. Distro, architecture and installation script determine its cache identity; a changed identity or missing machine requires preparation. A remote systemd unit retains interrupted preparation and downloads: after `down`, the next `up` reconnects or retries. Sealed bases are stopped and identity-free; clones must have distinct machine and Docker identities. Port configuration runs after cloning.

`current/env.json` records ownership and expected configuration; OrbStack, Docker and live responses determine availability. Each entry in `images`, `upgrades` and `failures` contains its reference, version and digest. Rust and Playwright read the same manifest and credentials. Release files are derived from these image records. This disposable format has no migration: release an occupied environment, remove it with the previous tool, then rebuild with the new tool. `down` preserves registry layers and results, and removes disposable Garage/Pebble data.

Every attempt has separate results; failures retain the environment. Provisioning failures capture dependency logs and container health details. Commands stream long output and terminate their process group on timeout; base preparation remains owned by systemd. System cases explicitly register their node, upgrade or platform restoration callback, which runs after failure diagnostics and outside the case timeout. Restoration failures are recorded in `restore-error.txt`, preserve an existing case failure and fail an otherwise successful case. Node restoration attempts every worker and retains each failure separately. Garage restoration remains local to storage acceptance. Both Rust polling loops check the deadline before starting another probe. Reruns reset fixed TLS fixtures and browser language.

TOTP uses the real cooldown and can take about six minutes. Acceptance does not wait for 30-minute invitation expiry, seven-day session expiry or long certificate renewal periods. External IM and model providers are not contacted. Concurrent same-name group creation and NUL-containing application command arguments are excluded from this acceptance gate by scope decision. The former can return 500 instead of 409; the latter can be accepted and fail during deployment before rollback.

Idle first-batch loss is covered by `insight_idle_connection_recovery`. The collector now detects a remote close while waiting for data and reconnects when the next batch arrives. The real 315-second idle test passed: both new applications returned all three records, the original quiet application resumed without missing records, and component tasks remained unchanged. Evidence is under `target/tests/results/idle-connection-8c1b876e4d4e4c0ba2c00ab6ef41019a/`. This fixes reuse of an already closed idle connection; it does not add delivery acknowledgements or crash recovery. Upgrade tests retain their pre-upgrade persistence check. Original failure evidence remains under `target/tests/results/upgrade-cdelivery-6b5510b5c0e843b281abd0af45d33d4f/failure/initial-log-gap.txt` and `target/tests/results/idle-log-confirm-6955c07861a2481cada1a3cc71cf76ea/`.

`down` removes only recorded runtime resources and uniquely tagged images. `cargo xtask env clean` requires no active environment and removes recorded bases and the owned registry cache. Neither prunes shared BuildKit caches, upstream images or results.

Restart durability assertions start after a successful Store flush or verified remote upload. The graceful-shutdown case first confirms that both TCP writers expose all records in queries with no Parquet files on disk, then cancels ingestion while the client remains connected and checks the reopened data. This covers accepted buffered records, not every client socket write or a forced kill. Schema fixtures cover missing, extra and reordered columns plus integer widening through append, compaction and reopen. The compaction case stops Garage, confirms unuploaded replacements remain locally, stops storage workers before restoring Garage, checks the original remote data, then restarts storage and verifies backfill, cleanup and recovery into an empty directory. These scenarios use public interfaces and normal maintenance intervals; they do not control private queue depth or reproduce a fixed upload/deletion interleaving while the network remains available.

Multi-instance queries require the current S3 inventory and fail during an outage; local flushes continue, and recovery checks the complete row count and checksum. OrbStack machines share a kernel: worker interruption checks process/node recovery, not independent physical-kernel failures. Public CA/cloud services, public installation, manager quorum failures, external IM/LLM services and performance benchmarks are outside these targets.

`Cargo.lock` selects `boring` and `boring-sys` 5.4.0 for Pingora. Their bundled BoringSSL includes [the certificate callback fix](https://github.com/google/boringssl/commit/2752c13d111a7849d7396d964ecae9c4cda7f05b); HTTP/HTTPS acceptance passed on single and cluster with this version. Pingora 0.9.1 includes [the zero-capacity buffer fix](https://github.com/cloudflare/pingora/commit/572e04440b05e4437c3f0dc828c8db2e7c45be6a), so Tokio uses the normal 1.x version requirement. Builds and acceptance use `--locked`; dependency updates require HTTP/HTTPS acceptance on single and cluster. Production polling intervals remain unchanged.

Duty acceptance runs model and SMTP endpoints inside the Rust test process on system-assigned ports, reached through the manager's `host.orb.internal` address. It uses shared memory for scenario controls and observations, with real HTTP, SMTP and STARTTLS traffic from the controller. Its temporary CA is mounted only in the test controller; restoration removes model/email settings and the trust mount. Fixture tasks stop with the test, and successful cases verify that both listening ports are released. No Python runtime, external model or notification account is needed.

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
