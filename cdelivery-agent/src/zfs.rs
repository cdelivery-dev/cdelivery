use std::collections::{BTreeMap, BTreeSet};

use cdelivery_wire::{Backend, Report, Volume, name};
use docker::types::Filter;

use crate::Agent;
use crate::error::{ErrorKind, Result};

const CONTAINER: &str = "cdelivery-zfs";
const LABEL: &str = "dev.cdelivery.zfs-exec";

pub(crate) async fn reclaim(agent: &Agent) -> Result<()> {
    let containers = agent.docker.containers();
    if let Some(container) = containers.inspect(CONTAINER).await? {
        if container.config.labels.get(LABEL).map(String::as_str) != Some("true") {
            return Err(ErrorKind::Zfs(format!(
                "execution container {} has unexpected ownership",
                container.id
            ))
            .into());
        }
        containers.delete(&container.id).await.map_err(|error| {
            if error.is_conflict() {
                ErrorKind::Busy
            } else {
                ErrorKind::Docker(error)
            }
        })?;
    }
    Ok(())
}

async fn execute(agent: &Agent, script: &str, args: &[&str]) -> Result<String> {
    let spec = serde_json::json!({
        "Image": agent.image,
        "User": "0",
        "Entrypoint": ["/usr/bin/nsenter", "--target", "1", "--mount", "--root", "--wd", "--", "/bin/sh", "-ec", script, CONTAINER],
        "Cmd": args,
        "Env": ["LC_ALL=C"],
        "Labels": {LABEL: "true"},
        "HostConfig": {"Privileged": true, "PidMode": "host", "NetworkMode": "none", "AutoRemove": false, "LogConfig": {"Type": "json-file"}}
    });
    let containers = agent.docker.containers();
    let id = containers.create(CONTAINER, &spec).await?;
    containers.start(&id).await?;
    let status = containers
        .wait(&id, std::time::Duration::from_secs(180))
        .await?;
    let (stdout, stderr) = containers.logs(&id).await?;
    if let Err(error) = containers.delete(&id).await {
        tracing::warn!(error = ?error, "ZFS execution container {id} cleanup failed");
    }
    if status != 0 {
        return Err(ErrorKind::Zfs(format!(
            "execution for {} exited {status}: {}",
            args.join(" "),
            String::from_utf8_lossy(&stderr)
        ))
        .into());
    }
    String::from_utf8(stdout).map_err(|error| std::io::Error::other(error).into())
}

pub(crate) async fn datasets(
    agent: &Agent,
    recursive: bool,
) -> Result<BTreeMap<String, (u64, Option<u64>)>> {
    let output = execute(
        agent,
        r#"[ -x /usr/sbin/zfs ] || exit 0
        pools=$(/usr/sbin/zpool list -H -o name)
        printf '%s\n' "$pools" | grep -qx cdelivery || exit 0
        exec /usr/sbin/zfs list -Hp -t filesystem,snapshot -o name,used,available "$@" cdelivery"#,
        if recursive { &["-r"] } else { &[] },
    )
    .await?;
    let mut datasets = BTreeMap::new();
    for row in output.lines() {
        let fields: Vec<_> = row.split('\t').collect();
        let [name, used, available] = fields.as_slice() else {
            return Err(ErrorKind::Zfs(format!("invalid dataset row: {row}")).into());
        };
        let used = used.parse().map_err(std::io::Error::other)?;
        let available = if *available == "-" {
            None
        } else {
            Some(available.parse().map_err(std::io::Error::other)?)
        };
        datasets.insert((*name).to_owned(), (used, available));
    }
    Ok(datasets)
}

fn matches(volume: &docker::types::Volume, dataset: &str) -> bool {
    volume.driver == "local" && volume.options == cdelivery_wire::zfs_options(dataset)
}

pub(crate) fn backend(volume: &docker::types::Volume, name: &str) -> Option<Backend> {
    if volume.driver == "local" && volume.options.is_empty() {
        Some(Backend::Local)
    } else if matches(volume, &format!("cdelivery/volumes/{name}")) {
        Some(Backend::Zfs)
    } else {
        None
    }
}

fn capacity(datasets: &BTreeMap<String, (u64, Option<u64>)>) -> Result<(u64, u64)> {
    let (used, available) = datasets
        .get(cdelivery_wire::ZFS_POOL)
        .copied()
        .unwrap_or((0, Some(0)));
    let available =
        available.ok_or_else(|| ErrorKind::Zfs("pool capacity is unavailable".to_owned()))?;
    Ok((used.saturating_add(available), available))
}

async fn map_volume(agent: &Agent, volume: &str, dataset: &str) -> Result<()> {
    let created = agent
        .docker
        .volumes()
        .create_with_options(volume, &cdelivery_wire::zfs_options(dataset))
        .await?;
    if !matches(&created, dataset) {
        return Err(ErrorKind::InvalidVolume(volume.to_owned()).into());
    }
    Ok(())
}

pub(crate) async fn create(agent: &Agent, name: &str) -> Result<()> {
    let dataset = format!("cdelivery/volumes/{name}");
    execute(
        agent,
        r#"exec /usr/sbin/zfs create -u -o mountpoint=legacy "$1""#,
        &[&dataset],
    )
    .await?;
    map_volume(
        agent,
        &format!("{}{name}", cdelivery_wire::VOLUME_PREFIX),
        &dataset,
    )
    .await
}

pub(crate) async fn destroy(agent: &Agent, dataset: &str) -> Result<()> {
    execute(agent, r#"exec /usr/sbin/zfs destroy "$1""#, &[dataset]).await?;
    Ok(())
}

pub(crate) async fn list(agent: &Agent, sizes: bool) -> Result<Report> {
    let datasets = datasets(agent, true).await?;
    let inventory = agent.docker.volumes().list(sizes).await?;
    let mut volumes = BTreeMap::new();
    let mut guardians = BTreeSet::new();
    for (dataset, (used, _)) in &datasets {
        if let Some(name) = dataset
            .strip_prefix("cdelivery/volumes/")
            .filter(|name| name::volume(name))
        {
            volumes.insert(
                name.to_owned(),
                Volume {
                    size_bytes: sizes.then_some(*used),
                    backend: Some(Backend::Zfs),
                    ready: false,
                },
            );
        }
        if let Some(app) = dataset
            .strip_prefix("cdelivery/guardian/")
            .and_then(|path| path.split('/').next())
            .filter(|app| name::shape_ok(app) && !name::platform(app))
        {
            guardians.insert(app.to_owned());
        }
        if let Some((source, app)) = dataset.split_once("@cdelivery-guardian.")
            && source
                .strip_prefix("cdelivery/volumes/")
                .is_some_and(name::volume)
            && name::shape_ok(app)
            && !name::platform(app)
        {
            guardians.insert(app.to_owned());
        }
    }
    for volume in inventory {
        if let Some(app) = volume
            .name
            .strip_prefix(cdelivery_wire::GUARDIAN_VOLUME_PREFIX)
            .and_then(|name| name.split_once('.'))
            .filter(|(app, volume)| {
                name::shape_ok(app) && !name::platform(app) && name::volume(volume)
            })
            .map(|(app, _)| app)
        {
            guardians.insert(app.to_owned());
        }
        let Some(name) = volume
            .name
            .strip_prefix(cdelivery_wire::VOLUME_PREFIX)
            .filter(|name| name::volume(name))
        else {
            continue;
        };
        let backend = backend(&volume, name);
        let has_dataset = datasets.contains_key(&format!("cdelivery/volumes/{name}"));
        let entry = volumes.entry(name.to_owned()).or_insert(Volume {
            size_bytes: None,
            backend,
            ready: false,
        });
        entry.backend = backend;
        entry.ready = match backend {
            Some(Backend::Local) => !has_dataset,
            Some(Backend::Zfs) => has_dataset,
            None => false,
        };
        if backend == Some(Backend::Local) {
            entry.size_bytes = volume
                .usage_data
                .and_then(|usage| u64::try_from(usage.size).ok());
        }
    }
    let (total_bytes, available_bytes) = if agent.zfs_enabled {
        capacity(&datasets)?
    } else {
        let fs = rustix::fs::statvfs(cdelivery_wire::DISK_PATH).map_err(std::io::Error::from)?;
        (
            fs.f_blocks.saturating_mul(fs.f_frsize),
            fs.f_bavail.saturating_mul(fs.f_frsize),
        )
    };
    Ok(Report {
        volumes,
        guardians,
        zfs_enabled: agent.zfs_enabled,
        total_bytes,
        available_bytes,
    })
}

pub(crate) async fn cleanup(
    agent: &Agent,
    app: &str,
    datasets: &BTreeMap<String, (u64, Option<u64>)>,
    volumes: &[docker::types::Volume],
) -> Result<()> {
    if !datasets.contains_key(cdelivery_wire::ZFS_POOL) {
        return Err(ErrorKind::Zfs("pool is not imported".to_owned()).into());
    }
    let filter = Filter::new().label(format!("dev.cdelivery.app_name={app}"));
    for container in agent.docker.containers().list(true, true, &filter).await? {
        if container
            .labels
            .get("dev.cdelivery.app_service_kind")
            .is_some_and(|kind| kind == "GuardianApp" || kind == "GuardianCheck")
        {
            agent.docker.containers().delete(&container.id).await?;
        }
    }
    let prefix = format!("{}{app}.", cdelivery_wire::GUARDIAN_VOLUME_PREFIX);
    for volume in volumes {
        if volume.name.strip_prefix(&prefix).is_some_and(name::volume) {
            agent.docker.volumes().delete(&volume.name).await?;
        }
    }
    let parent = format!("cdelivery/guardian/{app}");
    if datasets.contains_key(&parent) {
        execute(agent, r#"exec /usr/sbin/zfs destroy -r "$1""#, &[&parent]).await?;
    }
    let suffix = format!("@cdelivery-guardian.{app}");
    if datasets.keys().any(|dataset| {
        dataset
            .strip_suffix(&suffix)
            .and_then(|source| source.strip_prefix("cdelivery/volumes/"))
            .is_some_and(name::volume)
    }) {
        execute(
            agent,
            r#"exec /usr/sbin/zfs destroy -r "$1""#,
            &[&format!("cdelivery/volumes{suffix}")],
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn prepare(agent: &Agent, app: &str, names: Vec<String>) -> Result<()> {
    if !agent.zfs_enabled {
        return Err(ErrorKind::ZfsDisabled.into());
    }
    let inventory = datasets(agent, true).await?;
    let volumes = agent.docker.volumes().list(false).await?;
    let names: BTreeSet<_> = names.into_iter().collect();
    for name in &names {
        let dataset = format!("cdelivery/volumes/{name}");
        if !inventory.contains_key(&dataset)
            || !volumes.iter().any(|volume| {
                volume.name == format!("{}{name}", cdelivery_wire::VOLUME_PREFIX)
                    && matches(volume, &dataset)
            })
        {
            return Err(ErrorKind::InvalidVolume(name.clone()).into());
        }
    }
    cleanup(agent, app, &inventory, &volumes).await?;
    let (total, available) = capacity(&datasets(agent, false).await?)?;
    if available < total / 10 {
        return Err(ErrorKind::Zfs("less than 10% pool space is available".to_owned()).into());
    }
    let snapshots: Vec<_> = names
        .iter()
        .map(|name| format!("cdelivery/volumes/{name}@cdelivery-guardian.{app}"))
        .collect();
    let parent = format!("cdelivery/guardian/{app}");
    let mut args = vec![parent.as_str()];
    args.extend(snapshots.iter().map(String::as_str));
    execute(
        agent,
        r#"parent=$1
        shift
        /usr/sbin/zfs snapshot "$@"
        for snapshot do
            source=${snapshot%@*}
            /usr/sbin/zfs clone -p -o mountpoint=legacy "$snapshot" "$parent/${source##*/}"
        done"#,
        &args,
    )
    .await?;
    for name in &names {
        map_volume(
            agent,
            &cdelivery_wire::guardian_volume(app, name),
            &format!("{parent}/{name}"),
        )
        .await?;
    }
    Ok(())
}
