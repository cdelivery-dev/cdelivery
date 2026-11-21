use std::collections::{BTreeMap, BTreeSet};

use cdelivery_wire::{Backend, Report, Volume, name};
use docker::types::Filter;
use tokio::time::Instant;

use crate::Agent;
use crate::error::{ErrorKind, Result};

async fn execute(program: &str, args: &[&str], deadline: Instant) -> Result<String> {
    let stdout = host_exec::execute("cdelivery-zfs.service", program, args, deadline)
        .await
        .map_err(|error| {
            if error.is_busy() {
                ErrorKind::Busy
            } else {
                ErrorKind::Host(error)
            }
        })?
        .success()?;
    String::from_utf8(stdout).map_err(|error| std::io::Error::other(error).into())
}

pub(crate) async fn datasets(deadline: Instant) -> Result<BTreeMap<String, (u64, Option<u64>)>> {
    let output = execute(
        "/usr/sbin/zfs",
        &[
            "list",
            "-Hpr",
            "-t",
            "filesystem,snapshot",
            "-o",
            "name,used,available",
            "cdelivery",
        ],
        deadline,
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
    let Some(&(used, Some(available))) = datasets.get(cdelivery_wire::ZFS_POOL) else {
        return Err(ErrorKind::Zfs("pool capacity is unavailable".to_owned()).into());
    };
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

pub(crate) async fn create(agent: &Agent, name: &str, deadline: Instant) -> Result<()> {
    let dataset = format!("cdelivery/volumes/{name}");
    execute(
        "/usr/sbin/zfs",
        &["create", "-u", "-o", "mountpoint=legacy", &dataset],
        deadline,
    )
    .await?;
    map_volume(
        agent,
        &format!("{}{name}", cdelivery_wire::VOLUME_PREFIX),
        &dataset,
    )
    .await
}

pub(crate) async fn destroy(dataset: &str, deadline: Instant) -> Result<()> {
    execute("/usr/sbin/zfs", &["destroy", dataset], deadline).await?;
    Ok(())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn list(agent: &Agent, sizes: bool, deadline: Instant) -> Result<Report> {
    let inventory = agent.docker.volumes().list(sizes).await?;
    let needs_zfs = agent.zfs_enabled
        || inventory.iter().any(|volume| {
            if let Some(name) = volume.name.strip_prefix(cdelivery_wire::VOLUME_PREFIX)
                && name::volume(name)
            {
                return backend(volume, name) == Some(Backend::Zfs);
            }
            volume
                .name
                .strip_prefix(cdelivery_wire::GUARDIAN_VOLUME_PREFIX)
                .and_then(|suffix| suffix.split_once('.'))
                .is_some_and(|(preview, name)| {
                    name::preview(preview)
                        && name::volume(name)
                        && matches(volume, &format!("cdelivery/guardian/{preview}/{name}"))
                })
        });
    let datasets = if needs_zfs {
        datasets(deadline).await?
    } else {
        BTreeMap::new()
    };
    let mut volumes = BTreeMap::new();
    let mut previews = BTreeSet::new();
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
            .filter(|app| name::preview(app))
        {
            previews.insert(app.to_owned());
        }
        if let Some((source, app)) = dataset.split_once("@cdelivery-guardian.")
            && source
                .strip_prefix("cdelivery/volumes/")
                .is_some_and(name::volume)
            && name::preview(app)
        {
            previews.insert(app.to_owned());
        }
    }
    for volume in inventory {
        if let Some(app) = volume
            .name
            .strip_prefix(cdelivery_wire::GUARDIAN_VOLUME_PREFIX)
            .and_then(|name| name.split_once('.'))
            .filter(|(app, volume)| name::preview(app) && name::volume(volume))
            .map(|(app, _)| app)
        {
            previews.insert(app.to_owned());
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
        previews,
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
    deadline: Instant,
) -> Result<()> {
    if !datasets.contains_key(cdelivery_wire::ZFS_POOL) {
        return Err(ErrorKind::Zfs("pool is not imported".to_owned()).into());
    }
    let filter = Filter::new().label(format!("dev.cdelivery.preview={app}"));
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
        execute("/usr/sbin/zfs", &["destroy", "-r", &parent], deadline).await?;
    }
    let suffix = format!("@cdelivery-guardian.{app}");
    if datasets.keys().any(|dataset| {
        dataset
            .strip_suffix(&suffix)
            .and_then(|source| source.strip_prefix("cdelivery/volumes/"))
            .is_some_and(name::volume)
    }) {
        execute(
            "/usr/sbin/zfs",
            &["destroy", "-r", &format!("cdelivery/volumes{suffix}")],
            deadline,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn prepare(
    agent: &Agent,
    app: &str,
    names: Vec<String>,
    deadline: Instant,
) -> Result<()> {
    if !agent.zfs_enabled {
        return Err(ErrorKind::ZfsDisabled.into());
    }
    let inventory = datasets(deadline).await?;
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
    let parent = format!("cdelivery/guardian/{app}");
    let suffix = format!("@cdelivery-guardian.{app}");
    let prefix = format!("{}{app}.", cdelivery_wire::GUARDIAN_VOLUME_PREFIX);
    if inventory.keys().any(|dataset| {
        dataset == &parent
            || dataset.starts_with(&format!("{parent}/"))
            || dataset.ends_with(&suffix)
    }) || volumes
        .iter()
        .any(|volume| volume.name.starts_with(&prefix))
    {
        return Err(ErrorKind::PreviewExists(app.to_owned()).into());
    }
    let (total, available) = capacity(&inventory)?;
    if available < total / 10 {
        return Err(ErrorKind::Zfs("less than 10% pool space is available".to_owned()).into());
    }
    let snapshots: Vec<_> = names
        .iter()
        .map(|name| format!("cdelivery/volumes/{name}@cdelivery-guardian.{app}"))
        .collect();
    let mut args = vec![
        "-ec",
        r#"parent=$1
        shift
        /usr/sbin/zfs snapshot "$@"
        for snapshot do
            source=${snapshot%@*}
            /usr/sbin/zfs clone -p -o mountpoint=legacy "$snapshot" "$parent/${source##*/}"
        done"#,
        "cdelivery-zfs",
        parent.as_str(),
    ];
    args.extend(snapshots.iter().map(String::as_str));
    execute("/bin/sh", &args, deadline).await?;
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
