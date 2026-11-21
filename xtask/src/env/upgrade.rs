use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::{COMMAND_TIMEOUT, ErrorKind, Result, docker, execute, machine, save, wait};
use crate::environment::{Credentials, Environment, Image};

#[expect(clippy::too_many_lines)]
pub(super) fn run(root: &Path, action: &str) -> Result<()> {
    let state = root.join("target/tests/current");
    let mut environment: Environment = serde_json::from_slice(&fs::read(state.join("env.json"))?)?;
    if !environment.ready
        || !matches!(environment.profile.as_str(), "single" | "cluster")
        || environment
            .machines
            .first()
            .is_none_or(|name| *name != format!("cdelivery-test-{}-manager", environment.owner))
    {
        return Err(ErrorKind::Invalid(
            "upgrade fixtures require an owned ready platform environment".into(),
        )
        .into());
    }
    let artifacts = root.join("target/tests/results").join(format!(
        "upgrade-{action}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&artifacts)?;
    if action == "prepare" {
        for name in super::COMPONENTS.into_iter().chain(["hurl"]) {
            let base_version = environment.images[name]
                .version
                .split('-')
                .next()
                .ok_or_else(|| ErrorKind::Invalid("missing base version".into()))?;
            let (prefix, patch) = base_version
                .rsplit_once('.')
                .ok_or_else(|| ErrorKind::Invalid("invalid base version".into()))?;
            let patch: u64 = patch
                .parse()
                .map_err(|_| ErrorKind::Invalid("invalid patch version".into()))?;
            for (target, increment) in [("upgrade", 1), ("broken", 2)] {
                if if target == "upgrade" {
                    environment.upgrades.contains_key(name)
                } else {
                    environment.failures.contains_key(name)
                } {
                    continue;
                }
                let version = format!("{prefix}.{}-test.{}", patch + increment, environment.owner);
                let baseline = &environment.images[if name == "hurl" && target == "broken" {
                    "fixture"
                } else {
                    name
                }]
                .reference;
                let reference =
                    format!("{}/cdelivery-tests/{name}:{version}", environment.registry);
                let local = reference.replace(&environment.registry, "localhost:19500");
                crate::process::run(
                    docker(&[
                        "buildx",
                        "build",
                        "--load",
                        "--target",
                        target,
                        "--tag",
                        &local,
                        "--build-arg",
                        &format!(
                            "BASE={}",
                            baseline.replace(&environment.registry, "localhost:19500")
                        ),
                        "--build-arg",
                        &format!("VERSION={version}"),
                        "--file",
                    ])
                    .arg(root.join("tests/fixtures/upgrade.Dockerfile"))
                    .arg(root.join("tests/fixtures")),
                    b"",
                    Duration::from_secs(600),
                    Some(&artifacts.join(format!("build-{name}-{target}.log"))),
                )?;
                let digest =
                    super::push(&local, &artifacts.join(format!("push-{name}-{target}.log")))?;
                let image = Image {
                    reference,
                    version,
                    digest,
                };
                if target == "upgrade" {
                    environment.upgrades.insert(name.to_owned(), image);
                } else {
                    environment.failures.insert(name.to_owned(), image);
                }
                save(root, &environment)?;
            }
        }
    }
    let releases = super::publish(
        root,
        &environment,
        if action == "prepare" {
            &environment.upgrades
        } else {
            &environment.images
        },
    )?;
    let manager = &environment.machines[0];
    for (name, service) in [
        ("insight", "cdelivery-insight"),
        ("insight-agent", "cdelivery-insight-agent"),
        ("cdelivery-agent", "cdelivery-agent"),
        ("pingress", "cdelivery-pingress"),
        ("cdelivery", "cdelivery"),
    ] {
        let stored: Value = serde_json::from_str(&execute(
            &mut machine(manager, &["docker", "service", "inspect", service]),
            b"",
            COMMAND_TIMEOUT,
        )?)?;
        let current = stored[0]["Spec"]["TaskTemplate"]["ContainerSpec"]["Image"]
            .as_str()
            .ok_or_else(|| ErrorKind::Invalid("missing current image".into()))?;
        if current.split('@').next() != Some(environment.images[name].reference.as_str())
            || service == "cdelivery"
        {
            execute(
                &mut machine(
                    manager,
                    &[
                        "docker",
                        "service",
                        "update",
                        "--detach=true",
                        "--force",
                        "--image",
                        &environment.images[name].reference,
                        service,
                    ],
                ),
                b"",
                COMMAND_TIMEOUT,
            )?;
        }
        wait("baseline task images", || {
            let tasks = execute(
                &mut machine(
                    manager,
                    &[
                        "docker",
                        "service",
                        "ps",
                        "--no-trunc",
                        "--filter",
                        "desired-state=running",
                        "--format",
                        "{{.Image}} {{.CurrentState}}",
                        service,
                    ],
                ),
                b"",
                COMMAND_TIMEOUT,
            )?;
            let expected = if matches!(name, "cdelivery" | "insight") {
                1
            } else {
                environment.machines.len()
            };
            if tasks.lines().count() != expected
                || tasks.lines().any(|line| {
                    !line.starts_with(&environment.images[name].reference)
                        || !line.contains(" Running ")
                })
            {
                return Err(ErrorKind::Invalid(format!(
                    "{service} baseline tasks pending: {tasks}"
                ))
                .into());
            }
            Ok(())
        })?;
    }
    let credentials: Credentials =
        serde_json::from_slice(&fs::read(state.join("credentials.json"))?)?;
    wait("release catalog", || {
        let values: Value = serde_json::from_str(&super::api(
            &environment,
            &credentials.api_key,
            "GET",
            "/api/versions",
            None,
        )?)?;
        for (name, release) in &releases {
            if name == "templates" {
                continue;
            }
            if values["components"][name]["image"] != release["image"] {
                return Err(ErrorKind::Invalid(format!("release {name} not loaded")).into());
            }
        }
        Ok(())
    })?;
    fs::copy(state.join("env.json"), artifacts.join("env.json"))?;
    Ok(())
}
