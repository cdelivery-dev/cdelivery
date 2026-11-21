use std::process::Command;
use std::time::Duration;

use insight_wire::metric::{ZfsDatasetBucket, ZfsPoolBucket};

#[path = "../../tests/process.rs"]
mod process;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
#[expect(clippy::too_many_lines)]
async fn native_storage_collection() -> Result<()> {
    let remote: Vec<String> = serde_json::from_str(
        &std::env::var("CDELIVERY_TEST_REMOTE").unwrap_or_else(|_| "[]".into()),
    )?;
    let run = |args: &[&str]| -> Result<String> {
        let mut command = if let Some((program, prefix)) = remote.split_first() {
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
    };
    let docker = |args: &[&str]| run(&[&["docker"], args].concat());
    let name = format!("cd-metrics-{}", uuid::Uuid::new_v4().simple());
    let dataset = format!("cdelivery/{name}");
    let volume = format!("{dataset}/volume");
    let key = format!("{name}-key");
    let data = format!("{name}-data");
    let empty = format!("/tmp/{name}");
    let agent_image = std::env::var("CDELIVERY_TEST_INSIGHT_AGENT_IMAGE")?;
    let image = std::env::var("CDELIVERY_TEST_INSIGHT_IMAGE")?;
    let encrypted =
        cipher::Cipher::from_key(&[0; 32]).seal(&insight_wire::ObjectStoreConfig::Local)?;
    run(&["mkdir", &empty])?;
    run(&[
        "/usr/sbin/zfs",
        "create",
        "-o",
        "mountpoint=none",
        "-o",
        "quota=64M",
        "-o",
        "refquota=32M",
        "-o",
        "reservation=1M",
        "-o",
        "refreservation=2M",
        &dataset,
    ])?;
    run(&["/usr/sbin/zfs", "create", "-s", "-V", "8M", &volume])?;
    let network = docker(&[
        "network",
        "create",
        "--driver",
        "overlay",
        "--attachable",
        &name,
    ])?;
    docker(&[
        "run",
        "--rm",
        "--volume",
        &format!("{key}:/key"),
        "busybox:1.37.0-musl",
        "dd",
        "if=/dev/zero",
        "of=/key/cdelivery-insight",
        "bs=32",
        "count=1",
    ])?;
    docker(&[
        "run",
        "--detach",
        "--name",
        &name,
        "--network",
        &network,
        "--network-alias",
        "cdelivery-insight",
        "--publish",
        "127.0.0.1:19010:9010",
        "--volume",
        &format!("{key}:/run/secrets:ro"),
        "--volume",
        &format!("{data}:/opt/insight"),
        "--env",
        "RUST_LOG=info",
        "--env",
        "NODE_ID=storage-test",
        "--env",
        &format!("STORAGE={encrypted}"),
        "--env",
        "DATA_RETENTION_DAYS=1",
        "--env",
        "WRITER_FLUSH_INTERVAL_SECS=1",
        "--env",
        "SINGLE_INSTANCE=true",
        &image,
    ])?;
    let start = (jiff::Timestamp::now().as_second() - 600) * 1_000_000_000;
    let outcome: Result<()> = async {
        for (node, bus) in [("native", true), ("missing-bus", false)] {
            let service = format!("{name}-{node}");
            let mut mounts = vec![
                serde_json::json!({"Type":"bind","Source":"/var/run/docker.sock","Target":"/var/run/docker.sock","ReadOnly":true}),
                serde_json::json!({"Type":"bind","Source":"/","Target":"/host","ReadOnly":true,"BindOptions":{"Propagation":"rslave","ReadOnlyForceRecursive":true}}),
                serde_json::json!({"Type":"bind","Source":"/run","Target":"/run"}),
            ];
            if !bus {
                mounts.push(serde_json::json!({"Type":"bind","Source":empty,"Target":"/host/run","ReadOnly":true}));
            }
            let spec = serde_json::json!({
                "Name":service,"Mode":{"Global":{}},
                "TaskTemplate":{
                    "ContainerSpec":{"Image":agent_image,"User":"0","Privileges":{"AppArmor":{"Mode":"disabled"}},
                        "Env":["RUST_LOG=info",format!("NODE_ID={node}"),"SAMPLE_INTERVAL_SECS=5"],"Mounts":mounts},
                    "Networks":[{"Target":network}]
                }
            });
            run(&["curl", "--fail-with-body", "--silent", "--show-error", "--unix-socket", "/var/run/docker.sock", "--header", "Content-Type: application/json", "--data-binary", &spec.to_string(), "http://localhost/v1.48/services/create"])?;
            let query = |kind| -> Result<serde_json::Value> {
                let end = jiff::Timestamp::now().as_nanosecond();
                let output = run(&["curl", "--fail-with-body", "--silent", "--show-error", &format!("http://127.0.0.1:19010/metrics/{kind}/{start}/{end}?node_id={node}&step_secs=300")])?;
                Ok(serde_json::from_str(&output)?)
            };
            let observed: Result<()> = async {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
                loop {
                    let host = query("host").ok();
                    let filesystem = query("filesystem").ok();
                    let pools = query("zfs-pool").ok();
                    let datasets = query("zfs-dataset").ok();
                    let present = |value: &Option<serde_json::Value>| value.as_ref().and_then(serde_json::Value::as_array).is_some_and(|rows| !rows.is_empty());
                    if present(&host) && present(&filesystem) && (!bus || (present(&pools) && present(&datasets))) {
                        break;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        return Err(std::io::Error::other(docker(&["service", "logs", "--tail", "40", &service])?).into());
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                let pools: Vec<ZfsPoolBucket> = serde_json::from_value(query("zfs-pool")?)?;
                let datasets: Vec<ZfsDatasetBucket> = serde_json::from_value(query("zfs-dataset")?)?;
                if bus {
                    ensure(pools.iter().any(|pool| pool.pool == "cdelivery" && pool.size_bytes.avg.is_some_and(|bytes| bytes > 0.0)), "native pool capacity must reach the HTTP API")?;
                    let filesystem = datasets.iter().find(|sample| sample.dataset == dataset).ok_or_else(|| std::io::Error::other("native filesystem dataset missing"))?;
                    ensure(filesystem.quota_bytes.avg == Some(67_108_864.0) && filesystem.refquota_bytes.avg == Some(33_554_432.0) && filesystem.reservation_bytes.avg == Some(1_048_576.0) && filesystem.refreservation_bytes.avg == Some(2_097_152.0), "native quota and reservation bytes must reach the HTTP API")?;
                    ensure(datasets.iter().any(|sample| sample.dataset == volume && sample.volsize_bytes.avg == Some(8_388_608.0) && sample.quota_bytes.avg.is_none()), "native zvol size and inapplicable quota must reach the HTTP API")?;
                } else {
                    ensure(pools.is_empty() && datasets.is_empty(), "missing D-Bus must omit ZFS samples while other metrics continue")?;
                    ensure(docker(&["service", "logs", "--tail", "40", &service])?.contains("ZFS pool metrics collection failed"), "missing D-Bus must be attempted and diagnosed")?;
                }
                Ok(())
            }.await;
            docker(&["service", "rm", &service])?;
            for _ in 0..60 {
                if docker(&["ps", "-q", "--filter", &format!("label=com.docker.swarm.service.name={service}")])?.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            observed?;
        }
        Ok(())
    }.await;
    docker(&["rm", "--force", &name])?;
    docker(&["network", "rm", &network])?;
    docker(&["volume", "rm", &key, &data])?;
    run(&["/usr/sbin/zfs", "destroy", "-r", &dataset])?;
    run(&["rmdir", &empty])?;
    outcome
}

fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(std::io::Error::other(message.to_owned()).into())
    }
}
