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
        let description = format!("{command:?}");
        let response = self.request(command).await?;
        let status = response.status();
        ensure(
            status.is_success(),
            &format!("{description}: {status}: {}", response.text().await?),
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

    async fn start(&self, zfs: bool, bus: bool) -> Result<()> {
        let mut mounts = vec![
            serde_json::json!({"Type":"bind","Source":"/var/run/docker.sock","Target":"/var/run/docker.sock"}),
            serde_json::json!({"Type":"volume","Source":format!("{}-key", self.name),"Target":"/run/secrets","ReadOnly":true}),
            serde_json::json!({"Type":"volume","Source":format!("{}-disk", self.name),"Target":"/volume"}),
        ];
        if bus {
            mounts.push(serde_json::json!({"Type":"bind","Source":"/run","Target":"/host/run","ReadOnly":true}));
        }
        let spec = serde_json::json!({
            "Name":self.name,
            "Mode":{"Global":{}},
            "TaskTemplate":{"ContainerSpec":{
                "Image":self.image,"User":"0",
                "Privileges":{"AppArmor":{"Mode":"disabled"}},
                "Env":["RUST_LOG=info","CDELIVERY_NODE_ID=zfs-test",format!("CDELIVERY_ZFS_ENABLED={zfs}")],
                "Mounts":mounts
            }},
            "EndpointSpec":{"Ports":[{"Protocol":"tcp","TargetPort":5011,"PublishedPort":15011,"PublishMode":"host"}]}
        });
        self.host(&[
            "curl",
            "--fail-with-body",
            "--silent",
            "--show-error",
            "--unix-socket",
            "/var/run/docker.sock",
            "--header",
            "Content-Type: application/json",
            "--data-binary",
            &spec.to_string(),
            "http://localhost/v1.48/services/create",
        ])?;
        self.ready().await
    }

    async fn stop(&self) -> Result<()> {
        self.docker(&["service", "rm", &self.name])?;
        for _ in 0..60 {
            if self
                .docker(&[
                    "ps",
                    "-q",
                    "--filter",
                    &format!("label=com.docker.swarm.service.name={}", self.name),
                ])?
                .is_empty()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err(std::io::Error::other("agent service did not stop").into())
    }

    async fn ready(&self) -> Result<()> {
        for _ in 0..60 {
            if self
                .request(RequestCommand::List { sizes: false })
                .await
                .is_ok()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err(
            std::io::Error::other(self.docker(&["service", "logs", "--tail", "40", &self.name])?)
                .into(),
        )
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
        let mut command = if let Some((program, prefix)) = self.remote.split_first() {
            let mut command = Command::new(program);
            command.args(prefix).arg(shlex::try_join(
                ["sudo"].into_iter().chain(args.iter().copied()),
            )?);
            command
        } else {
            let mut command = Command::new("sudo");
            command.args(args);
            command
        };
        Ok(
            process::run(&mut command, b"", Duration::from_secs(120), None)?
                .trim()
                .to_owned(),
        )
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
async fn volume_and_preview_storage() -> Result<()> {
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
            .timeout(Duration::from_secs(185))
            .build()?,
    };
    let volume = format!("zv-{suffix}");
    let app = uuid::Uuid::new_v4().simple().to_string();
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
        fixture.start(false, false).await?;
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
        fixture.stop().await?;
        fixture.start(true, true).await?;
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
            .request(RequestCommand::PreparePreview {
                name: app.clone(),
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
            fixture.report().await?.previews.contains(&app),
            "snapshot residue from a failed batch must remain discoverable",
        )?;
        ensure(
            fixture
                .request(RequestCommand::PreparePreview {
                    name: app.clone(),
                    names: vec![volume.clone(), second.clone()],
                })
                .await?
                .status()
                == 409,
            "partial preparation must require explicit cleanup before retry",
        )?;
        fixture
            .success(RequestCommand::DeletePreview { name: app.clone() })
            .await?;
        fixture.postgres(&database, &business).await?;
        fixture.query(
            &database,
            "CREATE TABLE verification (value text); INSERT INTO verification VALUES ('original')",
        )?;
        let (first, duplicate) = tokio::join!(
            fixture.request(RequestCommand::PreparePreview {
                name: app.clone(),
                names: vec![volume.clone(), second.clone(), volume.clone()],
            }),
            fixture.request(RequestCommand::PreparePreview {
                name: app.clone(),
                names: vec![second.clone(), volume.clone()],
            }),
        );
        let mut statuses = [first?.status().as_u16(), duplicate?.status().as_u16()];
        statuses.sort_unstable();
        ensure(
            matches!(statuses, [200, 423 | 409]),
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
                .request(RequestCommand::DeletePreview { name: app.clone() })
                .await?
                .status()
                .is_success(),
            "stopped container references must block cleanup",
        )?;
        fixture.docker(&["rm", &holder])?;
        ensure(
            fixture
                .request(RequestCommand::PreparePreview {
                    name: app.clone(),
                    names: vec![volume.clone()],
                })
                .await?
                .status()
                == 409,
            "repeated preparation must preserve existing clones",
        )?;
        ensure(
            fixture.data(&clone, "cat /data/value")? == "candidate",
            "retry must preserve modified clones",
        )?;
        fixture.postgres(&candidate, &clone).await?;
        ensure(
            fixture.query(&candidate, "SELECT value FROM verification")? == "candidate",
            "retry must preserve candidate database writes",
        )?;
        fixture.docker(&["stop", &candidate, &database])?;
        fixture.docker(&["rm", &candidate, &database])?;
        fixture.stop().await?;
        fixture.start(false, true).await?;
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
                .request(RequestCommand::PreparePreview {
                    name: app.clone(),
                    names: vec![volume.clone()],
                })
                .await?
                .status()
                .is_success(),
            "ordinary mode must reject new previews",
        )?;
        fixture
            .success(RequestCommand::DeletePreview { name: app.clone() })
            .await?;
        fixture.host(&["/usr/sbin/zpool", "export", "cdelivery"])?;
        ensure(
            !fixture
                .request(RequestCommand::List { sizes: true })
                .await?
                .status()
                .is_success(),
            "mapped ZFS volumes require a complete live inventory",
        )?;
        let offline = format!("zo-{suffix}");
        fixture
            .success(RequestCommand::Create {
                name: offline.clone(),
            })
            .await?;
        fixture
            .success(RequestCommand::Delete { name: offline })
            .await?;
        ensure(
            fixture.data(&format!("cdelivery-{local}"), "cat /data/value")? == "local",
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
        fixture.stop().await?;
        fixture.start(true, true).await?;
        fixture.host(&[
            "systemd-run",
            "--unit=cdelivery-zfs.service",
            "--property=Type=exec",
            "--property=RuntimeMaxSec=120",
            "--collect",
            "/bin/sleep",
            "120",
        ])?;
        ensure(
            fixture
                .request(RequestCommand::DeletePreview { name: app.clone() })
                .await?
                .status()
                == 423,
            "running host command must not be replaced",
        )?;
        fixture.docker(&[
            "service",
            "update",
            "--force",
            "--detach=false",
            &fixture.name,
        ])?;
        let mut busy = false;
        for _ in 0..30 {
            if let Ok(response) = fixture
                .request(RequestCommand::DeletePreview { name: app.clone() })
                .await
            {
                busy = response.status() == 423;
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        ensure(busy, "agent restart must not bypass an uncertain execution")?;
        fixture.host(&["systemctl", "stop", "cdelivery-zfs.service"])?;
        fixture
            .success(RequestCommand::PreparePreview {
                name: app.clone(),
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
            fixture.report().await?.previews.contains(&app),
            "snapshot-only residue must remain discoverable",
        )?;
        fixture
            .success(RequestCommand::DeletePreview { name: app.clone() })
            .await?;
        fixture.host(&["/usr/sbin/zfs", "list", &other_snapshot])?;
        fixture.host(&["/usr/sbin/zfs", "destroy", &other_snapshot])?;
        fixture
            .success(RequestCommand::DeletePreview { name: app.clone() })
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
    let cleanup = fixture.stop().await;
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
