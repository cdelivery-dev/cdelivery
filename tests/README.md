# Tests

## Run

Use macOS with OrbStack, Rust, Node.js and npm. Run one environment or test command at a time. Rust acceptance targets have `test = false`; select them explicitly and use one test thread.

From the repository root:

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

`single` uses one Ubuntu 24.04 machine with Local Insight storage; `cluster` uses three with Garage S3Compatible storage; `storage` starts only Garage. Repeat npm installation when its lockfile changes.

`up` refuses an existing environment. Rebuild with `down` then `up` after deployed code, dependency or provisioning changes; assertion-only changes can reuse the current deployment. `cargo xtask` also works from repository subdirectories.

`--include-ignored` runs **all** ignored cases, including cluster recovery and both duty patrol cases that each wait the real 15-minute interval. TOTP takes about six minutes; idle log recovery waits 315 seconds. To rerun a subset:

```sh
cargo test --locked -p tests --test system change_ -- --test-threads=1
cargo test --locked -p tests --test system postgres_release_selection -- --test-threads=1
cargo test --locked -p tests --test system node_failure_and_recovery -- --ignored --test-threads=1
npm --prefix tests/ui test -- dependencies.spec.mjs secrets.spec.mjs
```

## Storage prerequisites

Local Insight durability needs no environment:

```sh
cargo test --locked -p insight --test storage local_durability -- --test-threads=1
```

PostgreSQL executor acceptance needs Docker and an executor image:

```sh
sh scripts/build-image.sh cdelivery-postgres
cargo test --locked -p tests --test postgres -- --nocapture
```

Set `CDELIVERY_POSTGRES_IMAGE` to select an existing image; otherwise the test uses the executor package's version from `cargo pkgid`. The test removes its containers, volumes, network and temporary secret files.

`postgres_release_selection` uses local PostgreSQL and a dependency Secret fixture without consumers or cloud API calls. It checks independent image tags and repositories, manual recreation, and operations with a missing executor catalog entry. The test changes the isolated environment's catalog and restarts its controller.

ZFS acceptance needs a disposable Linux host with a real partition-backed `cdelivery` pool prepared by `website/public/install.sh --zfs`. OrbStack machines share a kernel; use an independent Linux kernel, such as a QEMU guest, for ZFS. Load the current Agent image, `busybox:1.37.0-musl` and `postgres:18-alpine` into that host's Docker daemon:

```sh
CDELIVERY_TEST_AGENT_IMAGE=cdelivery-zfs-agent:verify \
CDELIVERY_TEST_AGENT_URL=http://127.0.0.1:15011/ \
cargo test --locked -p cdelivery-agent --test storage -- --test-threads=1
```

For remote access, set `CDELIVERY_TEST_REMOTE` to a JSON command prefix such as `["ssh","test-host"]`; it must accept a final shell command and allow passwordless `sudo` for Docker, curl, systemd and ZFS commands. Forward the Agent port to the URL above. The host needs Docker 28 or later with cluster mode initialized and systemd 245 or later. The test creates a global agent service, first without D-Bus for ordinary-volume checks, then with `/run:/host/run:ro` and AppArmor disabled. It checks native execution across an agent restart, explicit cleanup after partial preparation, repeated-preview conflicts and PostgreSQL clone write isolation. It exports/imports the pool and temporarily renames its preview parent to provoke clone failures. It does not create pools or modify partitions; interrupted runs may need manual restoration.

`cargo test --locked -p host-exec --test systemd` requires the same Linux system bus at `/host/run/dbus/system_bus_socket`, host UID 0 and permission to start transient units. In a container, mount `/run:/host/run:ro` and disable its AppArmor profile. It checks literal arguments, separate large stdout/stderr streams, fast exits, exec/signal failures and timeout recovery. `insight`'s `local_durability` test also exercises ZFS Parquet persistence and HTTP aggregation, including unlimited quota, zero reservation and missing-sample semantics.

For native metrics acceptance, load current Insight and Insight Agent images into the same disposable host, set `CDELIVERY_TEST_INSIGHT_IMAGE` and `CDELIVERY_TEST_INSIGHT_AGENT_IMAGE`, and run `cargo test --locked -p insight --test native`. It uses the same remote prefix, creates an isolated network and server on host port 19010, and checks real filesystem/zvol properties through the HTTP API. Both agent variants run as global services; the variant without access to D-Bus must continue reporting host and filesystem metrics. The test removes its services, server, volumes, network and named ZFS datasets.

Controller volume acceptance needs the same host, a current controller/Agent release catalog and an existing test administrator:

```sh
CDELIVERY_TEST_CONTROLLER_URL=http://127.0.0.1:19031 \
CDELIVERY_TEST_AGENT_IMAGE=cdelivery-zfs-agent:verify \
CDELIVERY_TEST_ACCOUNT_EMAIL=zfs@example.test \
node tests/zfs.mjs
```

`CDELIVERY_TEST_CASE=preview` selects preview/mode-change checks. `CDELIVERY_TEST_APP_IMAGE` selects a registry-hosted BusyBox image. Active Changes must be closed before deleting their preview volumes. The script restores the original mode and revokes its temporary API key; failures retain named resources.

For local PostgreSQL dependencies, build `tests/fixtures/postgres-consumer.Dockerfile` with `tests/fixtures` as context and publish it to a reachable registry:

```sh
CDELIVERY_TEST_CONTROLLER_URL=http://127.0.0.1:19000 \
CDELIVERY_TEST_ACCOUNT_EMAIL=zfs@example.test \
CDELIVERY_TEST_POSTGRES_IMAGE=registry.example/test/postgres:18-alpine \
CDELIVERY_TEST_CONSUMER_IMAGE=registry.example/test/postgres-consumer:verify \
CDELIVERY_TEST_RESULTS_DIR=target/tests/results/local-postgresql \
node tests/local-postgresql.mjs
```

Use the same remote prefix. Each run creates a separate group, ZFS-backed database, dependency and consumer.

Fixtures remain for inspection; `CDELIVERY_TEST_KEEP=1` additionally leaves a failed rehearsal open. `CDELIVERY_TEST_PREFIX` selects resource names. The JSON report records names and Change IDs; exit revokes the temporary API key.

## Results and cleanup

| Location | Lifetime |
| --- | --- |
| `target/tests/cache/` | Prepared base records; `env clean` removes recorded bases |
| External registry volume | Cached image layers; removed by `env clean` |
| `target/tests/current/` | Environment manifest, credentials and releases; removed by `env down` |
| `target/tests/results/` | Per-attempt reports, timings, logs and browser traces; retained until explicitly removed |

Failures retain the environment and diagnostics. After an interrupted upgrade test, run `cargo xtask env upgrade restore`.

`env down` removes recorded runtime resources and uniquely tagged images. `env clean` requires no active environment and removes prepared bases and the registry cache. Neither command removes shared build caches, upstream images or results. Remove old environments with the previous tool before changing their manifest format.

## Static checks

Workspace builds require libpq headers and libraries, with `pg_config` on `PATH` (Homebrew: `$(brew --prefix libpq)/bin`).

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked -p tests --test system --test postgres -p insight --test storage --test native -p cdelivery-agent --test storage -p host-exec --test systemd -- -D warnings
cargo xtask comments
cargo xtask imports
cargo xtask locale
cargo xtask ui
```
