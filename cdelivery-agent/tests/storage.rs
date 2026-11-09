use std::process::Command;
use std::time::Duration;

use cdelivery_wire::{Backend, Command as RequestCommand, Report, Request};

#[path = "../../tests/process.rs"]
mod process;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct Fixture {
    remote: Vec<String>,
    image: String,
    url: String,
    name: String,
    client: reqwest::Client,
}

impl Fixture {
    fn docker(&self, args: &[&str]) -> Result<String> {
        let mut command = if let Some((program, prefix)) = self.remote.split_first() {
            let mut command = Command::new(program);
            command.args(prefix).arg(shlex::try_join(
                ["sudo", "docker"].into_iter().chain(args.iter().copied()),
            )?);
            command
        } else {
            Command::new("docker")
        };
        if self.remote.is_empty() {
            command.args(args);
        }
        Ok(
            process::run(&mut command, b"", Duration::from_secs(120), None)?
                .trim()
                .to_owned(),
        )
    }

    async fn request(&self, command: RequestCommand) -> Result<reqwest::Response> {
        let body = cipher::Cipher::from_key(&[0; 32]).seal(&Request {
            node_id: "zfs-test".to_owned(),
            command,
        })?;
        Ok(self.client.post(&self.url).body(body).send().await?)
    }

    async fn success(&self, command: RequestCommand) -> Result<()> {
        let response = self.request(command).await?;
        let status = response.status();
        ensure(
            status.is_success(),
            &format!("{status}: {}", response.text().await?),
        )
    }

    async fn report(&self) -> Result<Report> {
        Ok(self
            .request(RequestCommand::List { sizes: true })
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn start(&self, zfs: bool) -> Result<()> {
        self.docker(&[
            "run",
            "--detach",
            "--name",
            &self.name,
            "--publish",
            "15011:5011",
            "--env",
            "CDELIVERY_NODE_ID=zfs-test",
            "--env",
            &format!("CDELIVERY_ZFS_ENABLED={zfs}"),
            "--volume",
            "/var/run/docker.sock:/var/run/docker.sock",
            "--volume",
            &format!("{}-key:/run/secrets:ro", self.name),
            "--volume",
            &format!("{}-disk:/volume", self.name),
            &self.image,
        ])?;
        self.ready().await
    }

    async fn ready(&self) -> Result<()> {
        for _ in 0..60 {
            if let Ok(response) = self.request(RequestCommand::List { sizes: false }).await
                && response.status().is_success()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err(std::io::Error::other(self.docker(&["logs", &self.name])?).into())
    }

    fn data(&self, volume: &str, command: &str) -> Result<String> {
        self.docker(&[
            "run",
            "--rm",
            "--volume",
            &format!("{volume}:/data"),
            "busybox:1.37.0-musl",
            "sh",
            "-c",
            command,
        ])
    }

    fn host(&self, args: &[&str]) -> Result<String> {
        let mut command = vec![
            "run",
            "--rm",
            "--privileged",
            "--pid=host",
            "--entrypoint",
            "/usr/bin/nsenter",
            &self.image,
            "--target",
            "1",
            "--mount",
            "--root",
            "--wd",
            "--",
        ];
        command.extend_from_slice(args);
        self.docker(&command)
    }

    fn query(&self, container: &str, sql: &str) -> Result<String> {
        self.docker(&[
            "exec", "--user", "postgres", container, "psql", "-At", "-c", sql,
        ])
    }

    async fn postgres(&self, container: &str, volume: &str) -> Result<()> {
        self.docker(&[
            "run",
            "--detach",
            "--name",
            container,
            "--network",
            "none",
            "--env",
            "POSTGRES_HOST_AUTH_METHOD=trust",
            "--volume",
            &format!("{volume}:/var/lib/postgresql"),
            "postgres:18-alpine",
        ])?;
        for _ in 0..120 {
            if self
                .query(container, "SELECT 1")
                .is_ok_and(|value| value == "1")
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err(std::io::Error::other(self.docker(&["logs", container])?).into())
    }
}

fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(std::io::Error::other(message.to_owned()).into())
    }
}

#[tokio::test]
#[expect(clippy::too_many_lines)]
async fn volume_and_guardian_storage() -> Result<()> {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let suffix = suffix
        .get(..8)
        .ok_or_else(|| std::io::Error::other("invalid UUID"))?;
    let fixture = Fixture {
        remote: serde_json::from_str(
            &std::env::var("CDELIVERY_TEST_REMOTE").unwrap_or_else(|_| "[]".to_owned()),
        )?,
        image: std::env::var("CDELIVERY_TEST_AGENT_IMAGE")?,
        url: std::env::var("CDELIVERY_TEST_AGENT_URL")?,
        name: format!("cd-zfs-test-{suffix}"),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()?,
    };
    let volume = format!("zv-{suffix}");
    let app = format!("za-{suffix}");
    let business = format!("cdelivery-{volume}");
    let local = format!("zl-{suffix}");
    let database = format!("zp-{suffix}");
    let candidate = format!("zc-{suffix}");
    let second = format!("zz-{suffix}");
    let clone = cdelivery_wire::guardian_volume(&app, &volume);
    let second_clone = cdelivery_wire::guardian_volume(&app, &second);
    fixture.docker(&[
        "run",
        "--rm",
        "--volume",
        &format!("{}-key:/key", fixture.name),
        "busybox:1.37.0-musl",
        "dd",
        "if=/dev/zero",
        "of=/key/cdelivery-agent",
        "bs=32",
        "count=1",
    ])?;
    let outcome: Result<()> = async {
        fixture.start(false).await?;
        fixture
            .success(RequestCommand::Create {
                name: local.clone(),
            })
            .await?;
        fixture.data(&format!("cdelivery-{local}"), "printf local > /data/value")?;
        fixture
            .success(RequestCommand::Create {
                name: volume.clone(),
            })
            .await?;
        fixture.data(&business, "printf original > /data/value")?;
        fixture.docker(&["rm", "--force", &fixture.name])?;
        fixture.start(true).await?;
        ensure(
            fixture
                .report()
                .await?
                .volumes
                .get(&volume)
                .is_some_and(|volume| volume.ready && volume.backend == Some(Backend::Local)),
            "ordinary volumes must remain available in ZFS mode",
        )?;
        ensure(
            fixture
                .request(RequestCommand::Create {
                    name: volume.clone(),
                })
                .await?
                .status()
                == 409,
            "ordinary same-name mapping must conflict",
        )?;
        fixture
            .success(RequestCommand::Delete {
                name: volume.clone(),
            })
            .await?;
        fixture
            .success(RequestCommand::Create {
                name: volume.clone(),
            })
            .await?;
        fixture.data(&business, "printf original > /data/value")?;
        ensure(
            fixture
                .report()
                .await?
                .volumes
                .get(&volume)
                .is_some_and(|v| v.ready),
            "created ZFS volume must be usable",
        )?;
        fixture
            .success(RequestCommand::Create {
                name: second.clone(),
            })
            .await?;
        fixture.data(
            &format!("cdelivery-{second}"),
            "printf second > /data/value",
        )?;
        let held = format!("cdelivery/guardian-held-{suffix}");
        fixture.host(&["/usr/sbin/zfs", "rename", "cdelivery/guardian", &held])?;
        if let Err(error) =
            fixture.host(&["/usr/sbin/zfs", "create", "-V", "1M", "cdelivery/guardian"])
        {
            fixture.host(&["/usr/sbin/zfs", "rename", &held, "cdelivery/guardian"])?;
            return Err(error);
        }
        let failed = fixture
            .request(RequestCommand::PrepareGuardian {
                app: app.clone(),
                names: vec![volume.clone(), second.clone()],
            })
            .await;
        let mapped = fixture.docker(&["volume", "inspect", &clone]);
        fixture.host(&["/usr/sbin/zfs", "destroy", "cdelivery/guardian"])?;
        fixture.host(&["/usr/sbin/zfs", "rename", &held, "cdelivery/guardian"])?;
        ensure(
            failed?.status() == 500 && mapped.is_err(),
            "clone failure must stop the batch before Docker mappings are created",
        )?;
        ensure(
            fixture.report().await?.guardians.contains(&app),
            "snapshot residue from a failed batch must remain discoverable",
        )?;
        fixture.postgres(&database, &business).await?;
        fixture.query(
            &database,
            "CREATE TABLE verification (value text); INSERT INTO verification VALUES ('original')",
        )?;
        let (first, duplicate) = tokio::join!(
            fixture.request(RequestCommand::PrepareGuardian {
                app: app.clone(),
                names: vec![volume.clone(), second.clone(), volume.clone()],
            }),
            fixture.request(RequestCommand::PrepareGuardian {
                app: app.clone(),
                names: vec![second.clone(), volume.clone()],
            }),
        );
        let mut statuses = [first?.status().as_u16(), duplicate?.status().as_u16()];
        statuses.sort_unstable();
        ensure(
            statuses == [200, 423],
            "overlapping writes must fail fast without disrupting the active operation",
        )?;
        let snapshots = fixture.host(&[
            "/usr/sbin/zfs",
            "get",
            "-Hp",
            "-o",
            "value",
            "createtxg",
            &format!("cdelivery/volumes/{volume}@cdelivery-guardian.{app}"),
            &format!("cdelivery/volumes/{second}@cdelivery-guardian.{app}"),
        ])?;
        let transactions: Vec<_> = snapshots.lines().collect();
        ensure(
            matches!(transactions.as_slice(), [first, second] if first == second),
            "all source snapshots must share one ZFS transaction",
        )?;
        ensure(
            fixture.data(&second_clone, "cat /data/value")? == "second",
            "batch must clone every source volume",
        )?;
        ensure(
            fixture.data(&clone, "cat /data/value")? == "original",
            "clone must start with source data",
        )?;
        fixture.data(&clone, "printf candidate > /data/value")?;
        fixture.postgres(&candidate, &clone).await?;
        ensure(
            fixture.query(&candidate, "SELECT value FROM verification")? == "original",
            "database clone must recover committed source data",
        )?;
        fixture.query(&candidate, "UPDATE verification SET value = 'candidate'")?;
        ensure(
            fixture.query(&database, "SELECT value FROM verification")? == "original",
            "candidate database writes must not affect production",
        )?;
        fixture.docker(&["stop", &candidate])?;
        fixture.docker(&["rm", &candidate])?;
        ensure(
            fixture.data(&business, "cat /data/value")? == "original",
            "candidate writes must not reach the source",
        )?;
        let holder = fixture.docker(&[
            "create",
            "--volume",
            &format!("{clone}:/data"),
            "busybox:1.37.0-musl",
            "true",
        ])?;
        ensure(
            !fixture
                .request(RequestCommand::DeleteGuardian { app: app.clone() })
                .await?
                .status()
                .is_success(),
            "stopped container references must block cleanup",
        )?;
        fixture.docker(&["rm", &holder])?;
        fixture
            .success(RequestCommand::PrepareGuardian {
                app: app.clone(),
                names: vec![volume.clone()],
            })
            .await?;
        ensure(
            fixture.data(&clone, "cat /data/value")? == "original",
            "retry must rebuild rather than reuse modified clones",
        )?;
        fixture.postgres(&candidate, &clone).await?;
        ensure(
            fixture.query(&candidate, "SELECT value FROM verification")? == "original",
            "retry must discard candidate database writes",
        )?;
        fixture.docker(&["stop", &candidate, &database])?;
        fixture.docker(&["rm", &candidate, &database])?;
        fixture.docker(&["rm", "--force", &fixture.name])?;
        fixture.start(false).await?;
        let report = fixture.report().await?;
        ensure(
            report
                .volumes
                .get(&volume)
                .is_some_and(|volume| volume.ready && volume.backend == Some(Backend::Zfs)),
            "ZFS volumes must remain available in ordinary mode",
        )?;
        ensure(
            fixture.data(&format!("cdelivery-{local}"), "cat /data/value")? == "local",
            "mode changes must preserve ordinary data",
        )?;
        ensure(
            !fixture
                .request(RequestCommand::PrepareGuardian {
                    app: app.clone(),
                    names: vec![volume.clone()],
                })
                .await?
                .status()
                .is_success(),
            "ordinary mode must reject new Guardian clones",
        )?;
        fixture
            .success(RequestCommand::DeleteGuardian { app: app.clone() })
            .await?;
        fixture.host(&["/usr/sbin/zpool", "export", "cdelivery"])?;
        ensure(
            fixture
                .report()
                .await?
                .volumes
                .get(&local)
                .is_some_and(|volume| volume.ready),
            "ordinary volumes must work when the pool is not imported",
        )?;
        ensure(
            !fixture
                .request(RequestCommand::Delete {
                    name: volume.clone(),
                })
                .await?
                .status()
                .is_success(),
            "ZFS deletion must fail when the pool is unavailable",
        )?;
        fixture.docker(&["volume", "inspect", &business])?;
        fixture.host(&["/usr/sbin/zpool", "import", "-N", "cdelivery"])?;
        ensure(
            fixture.data(&business, "cat /data/value")? == "original",
            "pool reimport must preserve data and mapping",
        )?;
        fixture.docker(&["rm", "--force", &fixture.name])?;
        fixture.start(true).await?;

        let helper_name = "cdelivery-zfs";
        let helper = fixture.docker(&[
            "create",
            "--name",
            helper_name,
            "--label",
            "dev.cdelivery.zfs-exec=true",
            "--entrypoint",
            "/usr/bin/nsenter",
            &fixture.image,
            "--version",
        ])?;
        fixture
            .success(RequestCommand::DeleteGuardian { app: app.clone() })
            .await?;
        ensure(
            fixture.docker(&["inspect", &helper]).is_err(),
            "unstarted owned helper must be reclaimed",
        )?;
        let foreign = fixture.docker(&[
            "create",
            "--name",
            helper_name,
            "--entrypoint",
            "/usr/bin/nsenter",
            &fixture.image,
            "--version",
        ])?;
        ensure(
            fixture
                .request(RequestCommand::DeleteGuardian { app: app.clone() })
                .await?
                .status()
                == 500,
            "unknown helper ownership must block cleanup",
        )?;
        fixture.docker(&["inspect", &foreign])?;
        fixture.docker(&["rm", &foreign])?;
        let helper = fixture.docker(&[
            "run",
            "--detach",
            "--name",
            helper_name,
            "--label",
            "dev.cdelivery.zfs-exec=true",
            "--privileged",
            "--pid=host",
            "--entrypoint",
            "/usr/bin/nsenter",
            &fixture.image,
            "--target",
            "1",
            "--mount",
            "--root",
            "--wd",
            "--",
            "/bin/sleep",
            "120",
        ])?;
        ensure(
            fixture
                .request(RequestCommand::DeleteGuardian { app: app.clone() })
                .await?
                .status()
                == 423,
            "running helper must not be removed",
        )?;
        fixture.docker(&["restart", &fixture.name])?;
        let mut busy = false;
        for _ in 0..30 {
            if let Ok(response) = fixture
                .request(RequestCommand::DeleteGuardian { app: app.clone() })
                .await
            {
                busy = response.status() == 423;
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        ensure(busy, "agent restart must not bypass an uncertain execution")?;
        fixture.docker(&["stop", &helper])?;
        fixture
            .success(RequestCommand::PrepareGuardian {
                app: app.clone(),
                names: vec![volume.clone()],
            })
            .await?;
        let other_snapshot =
            format!("cdelivery/volumes/{volume}@cdelivery-guardian.other-{suffix}");
        fixture.host(&["/usr/sbin/zfs", "snapshot", &other_snapshot])?;
        ensure(
            !fixture
                .request(RequestCommand::Delete {
                    name: volume.clone(),
                })
                .await?
                .status()
                .is_success(),
            "clone dependency must block dataset destruction",
        )?;
        ensure(
            fixture
                .report()
                .await?
                .volumes
                .get(&volume)
                .is_some_and(|v| !v.ready),
            "partial deletion must remain visible as a residual",
        )?;
        fixture.docker(&["volume", "rm", &clone])?;
        fixture.host(&[
            "/usr/sbin/zfs",
            "destroy",
            &format!("cdelivery/guardian/{app}/{volume}"),
        ])?;
        ensure(
            fixture.report().await?.guardians.contains(&app),
            "snapshot-only residue must remain discoverable",
        )?;
        fixture
            .success(RequestCommand::DeleteGuardian { app: app.clone() })
            .await?;
        fixture.host(&["/usr/sbin/zfs", "list", &other_snapshot])?;
        fixture.host(&["/usr/sbin/zfs", "destroy", &other_snapshot])?;
        fixture
            .success(RequestCommand::DeleteGuardian { app: app.clone() })
            .await?;
        fixture
            .success(RequestCommand::Delete {
                name: volume.clone(),
            })
            .await?;
        fixture
            .success(RequestCommand::Delete {
                name: volume.clone(),
            })
            .await?;
        ensure(
            !fixture.report().await?.volumes.contains_key(&volume),
            "repeat cleanup must remove all residuals",
        )?;
        fixture
            .success(RequestCommand::Delete {
                name: local.clone(),
            })
            .await?;
        fixture
            .success(RequestCommand::Delete {
                name: second.clone(),
            })
            .await?;
        Ok(())
    }
    .await;
    for container in [&candidate, &database] {
        if fixture.docker(&["inspect", container]).is_ok() {
            fixture.docker(&["rm", "--force", container])?;
        }
    }
    let cleanup = fixture.docker(&["rm", "--force", &fixture.name]);
    fixture.docker(&[
        "volume",
        "rm",
        &format!("{}-key", fixture.name),
        &format!("{}-disk", fixture.name),
    ])?;
    outcome?;
    cleanup?;
    Ok(())
}
