#![expect(clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::environment::{Credentials, Environment};

mod base;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(180);
const API_PORT: u16 = 19000;
const REGISTRY_VOLUME: &str = "cdelivery-tests-registry";
const COMPONENTS: [&str; 5] = [
    "cdelivery",
    "cdelivery-agent",
    "pingress",
    "insight",
    "insight-agent",
];

#[derive(Debug, thiserror::Error)]
pub(crate) enum ErrorKind {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Process(#[from] crate::process::Error),
    #[error("{0}")]
    Invalid(String),
}

macros::located_error!(Error => ErrorKind, from {
    std::io::Error => Io,
    serde_json::Error => Json,
    crate::process::Error => Process,
});

pub(crate) fn run(root: &Path, args: &[&str]) -> Result<()> {
    match args {
        ["up", profile @ ("single" | "cluster" | "storage")] => up(root, profile),
        ["down"] => down(root),
        ["clean"] => clean(root),
        _ => Err(ErrorKind::Invalid(
            "usage: cargo xtask env up <single|cluster|storage> | down | clean".into(),
        )
        .into()),
    }
}

fn execute(command: &mut Command, input: &[u8], timeout: Duration) -> Result<String> {
    Ok(crate::process::run(command, input, timeout, None)?)
}

fn docker(args: &[&str]) -> Command {
    let mut command = Command::new("docker");
    command.args(["--context", "orbstack"]).args(args);
    command
}

fn machine(name: &str, args: &[&str]) -> Command {
    let mut command = Command::new("orb");
    command.args(["-m", name, "-u", "root"]).args(args);
    command
}

fn compose(root: &Path, environment: &Environment, args: &[&str]) -> Command {
    let mut command = docker(&[
        "compose",
        "--project-name",
        &format!("cdelivery-tests-{}", environment.owner),
    ]);
    command
        .arg("--file")
        .arg(root.join("tests/compose.yaml"))
        .args(args);
    command
}

fn save(root: &Path, environment: &Environment) -> Result<()> {
    let path = root.join("target/tests/current/env.json");
    fs::write(
        path.with_extension("tmp"),
        serde_json::to_vec_pretty(environment)?,
    )?;
    fs::rename(path.with_extension("tmp"), &path)?;
    fs::copy(
        &path,
        root.join("target/tests/results")
            .join(&environment.owner)
            .join("env.json"),
    )?;
    Ok(())
}

#[expect(clippy::too_many_lines)]
fn up(root: &Path, profile: &str) -> Result<()> {
    if std::env::consts::OS != "macos" {
        return Err(
            ErrorKind::Invalid("test environments require macOS and OrbStack".into()).into(),
        );
    }
    let state = root.join("target/tests/current");
    if state.exists() {
        return Err(ErrorKind::Invalid(
            "an environment already exists; use cargo xtask env down first".into(),
        )
        .into());
    }
    execute(
        &mut docker(&["info", "--format", "{{.ServerVersion}}"]),
        b"",
        COMMAND_TIMEOUT,
    )?;
    let ports: &[u16] = match profile {
        "storage" => &[19390],
        "single" => &[19500, 19080, 19400, 19501, 19805],
        _ => &[19500, 19080, 19390, 19400, 19501, 19805],
    };
    for &port in ports {
        std::net::TcpListener::bind(("0.0.0.0", port))?;
    }
    let roles: &[&str] = match profile {
        "single" => &["manager"],
        "cluster" => &["manager", "worker1", "worker2"],
        _ => &[],
    };
    if profile != "storage" {
        std::net::TcpListener::bind(("0.0.0.0", API_PORT))?;
        std::net::UdpSocket::bind(("0.0.0.0", 19853))?;
    }
    fs::create_dir_all(state.join("releases"))?;
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
    let owner = uuid::Uuid::new_v4().simple().to_string();
    let artifacts = root.join("target/tests/results").join(&owner).join("up");
    fs::create_dir_all(&artifacts)?;
    fs::set_permissions(
        root.join("target/tests/results"),
        fs::Permissions::from_mode(0o700),
    )?;
    let mut environment = Environment {
        profile: profile.to_owned(),
        machines: roles
            .iter()
            .map(|role| format!("cdelivery-test-{owner}-{role}"))
            .collect(),
        owner,
        addresses: BTreeMap::new(),
        images: BTreeMap::new(),
        versions: BTreeMap::new(),
        digests: BTreeMap::new(),
        base_url: String::new(),
        support_ip: String::new(),
        registry: "docker.orb.internal:19500".into(),
        ready: false,
    };
    save(root, &environment)?;
    let started = Instant::now();
    let result = provision(root, &mut environment, &artifacts);
    if result.is_err() {
        let mut commands = vec![(
            "dependencies-failure",
            compose(root, &environment, &["logs", "--no-color", "--tail", "200"]),
        )];
        match execute(
            &mut compose(root, &environment, &["ps", "--all", "--quiet"]),
            b"",
            Duration::from_secs(30),
        ) {
            Ok(ids) if !ids.trim().is_empty() => {
                let mut inspect = docker(&["inspect", "--format", "{{.Name}} {{json .State}}"]);
                inspect.args(ids.split_whitespace());
                commands.push(("dependency-state", inspect));
            }
            Ok(_) => {}
            Err(error) => eprintln!("listing dependencies for diagnostics failed: {error:?}"),
        }
        if let Some(manager) = environment.machines.first()
            && environment.addresses.contains_key(manager)
        {
            commands.extend([
                (
                    "services",
                    machine(manager, &["/usr/bin/docker", "service", "ls"]),
                ),
                (
                    "cdelivery",
                    machine(
                        manager,
                        &[
                            "/usr/bin/docker",
                            "service",
                            "logs",
                            "--tail",
                            "200",
                            "cdelivery",
                        ],
                    ),
                ),
            ]);
        }
        for (name, mut command) in commands {
            if let Err(error) = crate::process::run(
                &mut command,
                b"",
                Duration::from_secs(30),
                Some(&artifacts.join(format!("{name}.log"))),
            ) {
                eprintln!("collecting environment diagnostics failed: {error:?}");
            }
        }
    }
    let report = format!("{result:?}\nelapsed {:?}\n", started.elapsed());
    if let Err(error) = fs::write(artifacts.join("result.txt"), report) {
        eprintln!("writing environment result failed: {error:?}");
        if result.is_ok() {
            return Err(error.into());
        }
    }
    result
}

#[expect(clippy::too_many_lines)]
fn provision(root: &Path, environment: &mut Environment, artifacts: &Path) -> Result<()> {
    let state = root.join("target/tests/current");
    let started = Instant::now();
    let base = if environment.profile == "storage" {
        None
    } else {
        Some(base::prepare(root, artifacts)?)
    };
    if base.is_some() {
        registry_volume(false)?;
    }
    let mut args = vec![
        "up",
        "--detach",
        "--pull",
        "missing",
        "--wait",
        "--wait-timeout",
        "180",
    ];
    if base.is_some() {
        args.extend(["registry", "releases", "pebble", "challtestsrv"]);
    }
    if environment.profile != "single" {
        args.push("garage");
    }
    crate::process::run(
        &mut compose(root, environment, &args),
        b"",
        Duration::from_mins(15),
        Some(&artifacts.join("dependencies.log")),
    )?;
    let Some(base) = base else {
        environment.ready = true;
        save(root, environment)?;
        println!(
            "Ready: Garage\ncargo test --locked -p insight --test storage -- --test-threads=1"
        );
        return Ok(());
    };
    build(root, environment, artifacts)?;
    let mut machine_ids = BTreeSet::new();
    let mut docker_ids = BTreeSet::new();
    for (index, name) in environment.machines.iter().enumerate() {
        println!("Cloning {name}");
        execute(
            Command::new("orbctl").args(["clone", &base, name]),
            b"",
            Duration::from_mins(15),
        )?;
        let mut configure = machine(name, &["sh", "-s", "--", &environment.registry]);
        if index == 0 {
            configure.arg(API_PORT.to_string());
        }
        crate::process::run(
            &mut configure,
            include_bytes!("../../tests/fixtures/machine.sh"),
            COMMAND_TIMEOUT,
            Some(&artifacts.join(format!("{name}-configure.log"))),
        )?;
        let curl = execute(
            &mut machine(name, &["/usr/bin/curl", "--version"]),
            b"",
            COMMAND_TIMEOUT,
        )?;
        fs::write(artifacts.join(format!("{name}-curl.txt")), &curl)?;
        let version = curl.split_whitespace().nth(1).and_then(|version| {
            let mut parts = version.split('.');
            Some((
                parts.next()?.parse::<u64>().ok()?,
                parts.next()?.parse::<u64>().ok()?,
            ))
        });
        if version.is_none_or(|version| version < (7, 88))
            || !curl.contains("OpenSSL/")
            || !curl.split_whitespace().any(|feature| feature == "HTTP2")
        {
            return Err(ErrorKind::Invalid(format!(
                "{name}: curl requires version 7.88 or newer with OpenSSL and HTTP2: {curl}"
            ))
            .into());
        }
        let identity = execute(
            &mut machine(name, &["cat", "/etc/machine-id"]),
            b"",
            COMMAND_TIMEOUT,
        )?;
        let docker_id = execute(
            &mut machine(name, &["/usr/bin/docker", "info", "--format", "{{.ID}}"]),
            b"",
            COMMAND_TIMEOUT,
        )?;
        if identity.trim().is_empty()
            || docker_id.trim().is_empty()
            || !machine_ids.insert(identity.trim().to_owned())
            || !docker_ids.insert(docker_id.trim().to_owned())
        {
            return Err(ErrorKind::Invalid(
                "cloned nodes must have distinct machine and Docker identities".into(),
            )
            .into());
        }
        fs::write(
            artifacts.join(format!("{name}-identity.txt")),
            format!("machine={}\ndocker={}\n", identity.trim(), docker_id.trim()),
        )?;
        let address = execute(
            &mut machine(name, &["hostname", "-I"]),
            b"",
            COMMAND_TIMEOUT,
        )?;
        let address = address
            .split_whitespace()
            .find(|address| {
                address
                    .parse::<std::net::Ipv4Addr>()
                    .is_ok_and(|ip| !ip.is_loopback())
            })
            .ok_or_else(|| ErrorKind::Invalid(format!("no IPv4 address for {name}")))?;
        environment
            .addresses
            .insert(name.clone(), address.to_owned());
        save(root, environment)?;
    }
    let manager = &environment.machines[0];
    let address = &environment.addresses[manager];
    environment.base_url = format!("http://127.0.0.1:{API_PORT}");
    let hosts = execute(
        &mut machine(manager, &["getent", "ahostsv4", "docker.orb.internal"]),
        b"",
        COMMAND_TIMEOUT,
    )?;
    hosts
        .split_whitespace()
        .next()
        .ok_or_else(|| ErrorKind::Invalid("docker.orb.internal did not resolve".into()))?
        .clone_into(&mut environment.support_ip);
    save(root, environment)?;
    execute(
        &mut machine(
            manager,
            &[
                "/usr/bin/docker",
                "swarm",
                "init",
                "--advertise-addr",
                address,
            ],
        ),
        b"",
        COMMAND_TIMEOUT,
    )?;
    execute(
        &mut machine(
            manager,
            &[
                "/usr/bin/docker",
                "network",
                "create",
                "--driver",
                "overlay",
                "--attachable",
                "--label",
                "dev.cdelivery.managed=true",
                "cdelivery",
            ],
        ),
        b"",
        COMMAND_TIMEOUT,
    )?;
    println!("Deploying cdelivery and creating the test administrator");
    execute(
        &mut machine(
            manager,
            &[
                "/usr/bin/docker",
                "service",
                "create",
                "--name",
                "cdelivery",
                "--label",
                "dev.cdelivery.managed=true",
                "--replicas",
                "1",
                "--constraint",
                "node.role==manager",
                "--network",
                "cdelivery",
                "--stop-grace-period",
                "50s",
                "--update-monitor",
                "60s",
                "--rollback-monitor",
                "60s",
                "--mount",
                "type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock",
                "--publish",
                "published=9000,target=9000",
                "--env",
                &format!(
                    "CDELIVERY_RELEASES_URL=http://{}:19080/",
                    environment.support_ip
                ),
                "--env",
                &format!("CDELIVERY_ACME_DNS={}:19853", environment.support_ip),
                "--host",
                &format!("pebble:{}", environment.support_ip),
                "--host",
                &format!("http.cdelivery.test:{address}"),
                &environment.images["cdelivery"],
            ],
        ),
        b"",
        Duration::from_secs(300),
    )?;
    wait("setup endpoint", || {
        api(environment, "", "GET", "/setup", None).map(|_| ())
    })?;
    let mut credentials = Credentials {
        email: "admin@cdelivery.test".into(),
        password: format!("Test-{}-Aa1", uuid::Uuid::new_v4()),
        api_key: String::new(),
    };
    let setup = json!({"first_name":"Test", "last_name":"Admin", "email":credentials.email, "password":credentials.password});
    let output = execute(
        &mut machine(
            manager,
            &[
                "/usr/bin/docker",
                "run",
                "--rm",
                "-i",
                "--network",
                "host",
                &environment.images["cdelivery"],
                "setup",
            ],
        ),
        setup.to_string().as_bytes(),
        COMMAND_TIMEOUT,
    )?;
    fs::write(artifacts.join("setup.json"), output)?;
    let container = wait("cdelivery startup", || {
        api(environment, "", "GET", "/login", None)?;
        let value = execute(
            &mut machine(
                manager,
                &[
                    "/usr/bin/docker",
                    "ps",
                    "--filter",
                    "label=com.docker.swarm.service.name=cdelivery",
                    "--format",
                    "{{.ID}}",
                ],
            ),
            b"",
            COMMAND_TIMEOUT,
        )?;
        if value.trim().is_empty() {
            return Err(ErrorKind::Invalid("cdelivery container is not running".into()).into());
        }
        Ok(value.trim().to_owned())
    })?;
    let output = execute(
        &mut machine(
            manager,
            &[
                "/usr/bin/docker",
                "exec",
                &container,
                "/entrypoint",
                "create-api-key",
                &credentials.email,
                "tests",
                "Admin",
            ],
        ),
        b"",
        COMMAND_TIMEOUT,
    )?;
    output.trim().clone_into(&mut credentials.api_key);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(state.join("credentials.json"))?;
    serde_json::to_writer_pretty(&mut file, &credentials)?;
    wait("authenticated API", || {
        api(environment, &credentials.api_key, "GET", "/api/nodes", None)
    })?;
    for name in environment.machines.iter().skip(1) {
        let invitation: Value = serde_json::from_str(&api(
            environment,
            &credentials.api_key,
            "POST",
            "/api/nodes/join-invitations",
            Some(&json!({"role":"Worker"})),
        )?)?;
        let token = invitation["invitation"]
            .as_str()
            .ok_or_else(|| ErrorKind::Invalid("missing join invitation".into()))?;
        let body = api(
            environment,
            token,
            "POST",
            "/join/redeem",
            Some(&json!({"advertise_addr":environment.addresses[name]})),
        )?;
        execute(
            &mut machine(
                name,
                &[
                    "curl",
                    "--fail-with-body",
                    "--silent",
                    "--show-error",
                    "--unix-socket",
                    "/var/run/docker.sock",
                    "--header",
                    "Content-Type: application/json",
                    "--data-binary",
                    "@-",
                    "http://localhost/v1.48/swarm/join",
                ],
            ),
            body.as_bytes(),
            COMMAND_TIMEOUT,
        )?;
    }
    initialize(environment, &credentials)?;
    wait("deployment tasks", || {
        let output = execute(
            &mut machine(
                manager,
                &[
                    "/usr/bin/docker",
                    "service",
                    "ls",
                    "--format",
                    "{{.Name}} {{.Replicas}}",
                ],
            ),
            b"",
            COMMAND_TIMEOUT,
        )?;
        for (service, replicas) in [
            ("cdelivery", 1),
            ("cdelivery-agent", environment.machines.len()),
            ("cdelivery-pingress", environment.machines.len()),
            ("cdelivery-insight", 1),
            ("cdelivery-insight-agent", environment.machines.len()),
        ] {
            if !output
                .lines()
                .any(|line| line == format!("{service} {replicas}/{replicas}"))
            {
                return Err(ErrorKind::Invalid(format!("{service} is not ready: {output}")).into());
            }
        }
        Ok(())
    })?;
    environment.ready = true;
    save(root, environment)?;
    let selection = if environment.profile == "cluster" {
        " --include-ignored"
    } else {
        ""
    };
    println!(
        "Ready in {:?}: {}\ncargo test --locked -p tests --test system -- --test-threads=1{selection}\nnpm --prefix tests/ui test\ncargo xtask env down",
        started.elapsed(),
        environment.base_url
    );
    Ok(())
}

#[expect(clippy::too_many_lines)]
fn build(root: &Path, environment: &mut Environment, artifacts: &Path) -> Result<()> {
    let metadata: Value = serde_json::from_str(&execute(
        Command::new("cargo").current_dir(root).args([
            "metadata",
            "--locked",
            "--no-deps",
            "--format-version",
            "1",
        ]),
        b"",
        COMMAND_TIMEOUT,
    )?)?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or_else(|| ErrorKind::Invalid("cargo metadata has no packages".into()))?;
    for (name, fixture) in COMPONENTS.into_iter().map(|name| (name, None)).chain([
        ("fixture", Some(("busybox:1.37.0-musl", "1.37.0"))),
        (
            "hurl",
            Some(("ghcr.io/orange-opensource/hurl:8.0.1", "8.0.1")),
        ),
    ]) {
        let version = fixture
            .map(|(_, version)| version)
            .or_else(|| {
                packages
                    .iter()
                    .find(|package| package["name"] == name)
                    .and_then(|package| package["version"].as_str())
            })
            .ok_or_else(|| ErrorKind::Invalid(format!("missing package {name}")))?;
        let version = format!("{version}-test.{}", environment.owner);
        let image = format!("{}/cdelivery-tests/{name}:{version}", environment.registry);
        let local = image.replace(&environment.registry, "localhost:19500");
        if let Some((source, _)) = fixture {
            let images = execute(
                &mut docker(&["image", "ls", "--quiet", source]),
                b"",
                COMMAND_TIMEOUT,
            )?;
            if images.trim().is_empty() {
                crate::process::run(
                    &mut docker(&["pull", source]),
                    b"",
                    Duration::from_secs(600),
                    Some(&artifacts.join(format!("pull-{name}.log"))),
                )?;
            }
            execute(&mut docker(&["tag", source, &local]), b"", COMMAND_TIMEOUT)?;
        } else {
            println!("Building {name}");
            let profile = if name == "cdelivery" {
                "release-size"
            } else {
                "release-speed"
            };
            crate::process::run(
                docker(&[
                    "buildx",
                    "build",
                    "--load",
                    "--tag",
                    &local,
                    "--build-arg",
                    &format!("BIN_NAME={name}"),
                    "--build-arg",
                    &format!("PROFILE={profile}"),
                ])
                .arg(root),
                b"",
                Duration::from_secs(7200),
                Some(&artifacts.join(format!("build-{name}.log"))),
            )?;
        }
        crate::process::run(
            &mut docker(&["push", &local]),
            b"",
            Duration::from_secs(600),
            Some(&artifacts.join(format!("push-{name}.log"))),
        )?;
        let digests = execute(
            &mut docker(&[
                "image",
                "inspect",
                "--format",
                "{{json .RepoDigests}}",
                &local,
            ]),
            b"",
            COMMAND_TIMEOUT,
        )?;
        let digests: Vec<String> = serde_json::from_str(&digests)?;
        let repository = local
            .rsplit_once(':')
            .map_or(local.as_str(), |(repository, _)| repository);
        let digest = digests
            .into_iter()
            .find(|digest| digest.starts_with(&format!("{repository}@")))
            .ok_or_else(|| ErrorKind::Invalid(format!("no pushed digest for {local}")))?;
        environment.images.insert(name.into(), image);
        environment.versions.insert(name.into(), version);
        environment.digests.insert(name.into(), digest);
        save(root, environment)?;
    }
    let releases: BTreeMap<_, _> = environment
        .images
        .iter()
        .filter(|(name, _)| name.as_str() != "fixture")
        .map(|(name, image)| {
            (
                name.replace('-', "_"),
                json!({"image":image,"changelog":""}),
            )
        })
        .collect();
    let path = root.join(format!(
        "target/tests/current/releases/v1/{}/cdelivery-tests",
        environment.registry
    ));
    fs::create_dir_all(&path)?;
    fs::write(
        path.join("latest.json"),
        serde_json::to_vec_pretty(&releases)?,
    )?;
    save(root, environment)
}

fn initialize(environment: &Environment, credentials: &Credentials) -> Result<()> {
    let key = &credentials.api_key;
    let nodes: Value = serde_json::from_str(&wait("all nodes ready", || {
        let body = api(environment, key, "GET", "/api/nodes", None)?;
        let nodes: Value = serde_json::from_str(&body)?;
        let ready = nodes["nodes"].as_array().is_some_and(|nodes| {
            nodes.len() == environment.machines.len()
                && nodes.iter().all(|node| node["state"] == "Ready")
        });
        if !ready {
            return Err(ErrorKind::Invalid("nodes are not ready".into()).into());
        }
        Ok(body)
    })?)?;
    let nodes = nodes["nodes"]
        .as_array()
        .ok_or_else(|| ErrorKind::Invalid("missing nodes".into()))?;
    for node in nodes {
        let id = node["id"]
            .as_str()
            .ok_or_else(|| ErrorKind::Invalid("missing node ID".into()))?;
        api(
            environment,
            key,
            "PATCH",
            &format!("/api/nodes/{id}"),
            Some(&json!({"labels":["tests"]})),
        )?;
    }
    let manager = nodes
        .iter()
        .find(|node| node["role"] == "Manager")
        .ok_or_else(|| ErrorKind::Invalid("no manager".into()))?;
    let placement = json!({"NodeId":manager["id"]});
    api(
        environment,
        key,
        "PUT",
        "/api/pingress",
        Some(&json!({"version":environment.versions["pingress"],"placement":{"Label":"tests"}})),
    )?;
    api(
        environment,
        key,
        "PUT",
        "/api/volume",
        Some(&json!({"version":environment.versions["cdelivery-agent"]})),
    )?;
    let storage = if environment.profile == "single" {
        json!("Local")
    } else {
        json!({"S3Compatible":{
            "endpoint":format!("http://{}:19390", environment.support_ip), "region":"garage", "bucket_name":"insight",
            "access_key_id":"GK00000000000000000000000000000000", "secret_access_key":"2222222222222222222222222222222222222222222222222222222222222222"
        }})
    };
    api(
        environment,
        key,
        "PATCH",
        "/api/insight",
        Some(
            &json!({"storage":storage,"placement":placement,"server_version":environment.versions["insight"],"agent_version":environment.versions["insight-agent"],"sample_interval_secs":1,"flush_interval":{"unit":"Seconds","value":5},"retention_days":1}),
        ),
    )?;
    api(
        environment,
        key,
        "POST",
        "/api/groups",
        Some(&json!({"name":"tests"})),
    )?;
    Ok(())
}

fn api(
    environment: &Environment,
    key: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<String> {
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--noproxy",
        "*",
        "--fail-with-body",
        "--silent",
        "--show-error",
        "--max-time",
        "60",
        "--request",
        method,
        "--header",
        "Content-Type: application/json",
    ]);
    if !key.is_empty() {
        command.args(["--header", &format!("Authorization: Bearer {key}")]);
    }
    if body.is_some() {
        command.args(["--data-binary", "@-"]);
    }
    command.arg(format!("{}{path}", environment.base_url));
    execute(
        &mut command,
        body.map(Value::to_string).unwrap_or_default().as_bytes(),
        COMMAND_TIMEOUT,
    )
}

fn wait<T>(description: &str, mut check: impl FnMut() -> Result<T>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        match check() {
            Ok(value) => return Ok(value),
            Err(error) if Instant::now() >= deadline => {
                return Err(ErrorKind::Invalid(format!(
                    "timed out waiting for {description}: {error:?}"
                ))
                .into());
            }
            Err(_) => std::thread::sleep(Duration::from_secs(2)),
        }
    }
}

fn down(root: &Path) -> Result<()> {
    let state = root.join("target/tests/current");
    if !state.join("env.json").exists() {
        println!("No test environment.");
        return Ok(());
    }
    let environment: Environment = serde_json::from_slice(&fs::read(state.join("env.json"))?)?;
    if uuid::Uuid::parse_str(&environment.owner).is_err()
        || environment.machines.iter().any(|name| {
            !["manager", "worker1", "worker2"]
                .iter()
                .any(|role| *name == format!("cdelivery-test-{}-{role}", environment.owner))
        })
    {
        return Err(ErrorKind::Invalid(
            "invalid resource ownership in env.json; refusing cleanup".into(),
        )
        .into());
    }
    let artifacts = root
        .join("target/tests/results")
        .join(&environment.owner)
        .join(format!("down-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&artifacts)?;
    if let Err(error) = crate::process::run(
        &mut compose(root, &environment, &["logs", "--no-color", "--tail", "200"]),
        b"",
        Duration::from_secs(30),
        Some(&artifacts.join("dependencies.log")),
    ) {
        eprintln!("collecting dependency diagnostics failed: {error:?}");
    }
    let machines = if environment.machines.is_empty() {
        BTreeMap::new()
    } else {
        base::machines()?
    };
    for name in &environment.machines {
        if machines.contains_key(name) {
            println!("Deleting {name}");
            execute(
                Command::new("orbctl").args(["delete", "--force", name]),
                b"",
                COMMAND_TIMEOUT,
            )?;
        }
    }
    execute(
        &mut compose(
            root,
            &environment,
            &["down", "--volumes", "--remove-orphans"],
        ),
        b"",
        COMMAND_TIMEOUT,
    )?;
    let images = execute(
        &mut docker(&["image", "ls", "--format", "{{.Repository}}:{{.Tag}}"]),
        b"",
        COMMAND_TIMEOUT,
    )?;
    for image in images.lines().filter(|image| {
        image.starts_with("localhost:19500/cdelivery-tests/")
            && image.ends_with(&format!("-test.{}", environment.owner))
    }) {
        execute(&mut docker(&["image", "rm", image]), b"", COMMAND_TIMEOUT)?;
    }
    fs::remove_dir_all(state)?;
    println!("Test environment removed; caches and target/tests/results are retained.");
    Ok(())
}

fn registry_volume(remove: bool) -> Result<()> {
    let volumes = execute(
        &mut docker(&["volume", "ls", "--format", "{{.Name}}"]),
        b"",
        COMMAND_TIMEOUT,
    )?;
    if volumes.lines().any(|name| name == REGISTRY_VOLUME) {
        let label = execute(
            &mut docker(&[
                "volume",
                "inspect",
                "--format",
                "{{index .Labels \"dev.cdelivery.tests.cache\"}}",
                REGISTRY_VOLUME,
            ]),
            b"",
            COMMAND_TIMEOUT,
        )?;
        if label.trim() != "true" {
            return Err(ErrorKind::Invalid("registry volume is not owned by tests".into()).into());
        }
        if remove {
            execute(
                &mut docker(&["volume", "rm", REGISTRY_VOLUME]),
                b"",
                COMMAND_TIMEOUT,
            )?;
        }
    } else if !remove {
        execute(
            &mut docker(&[
                "volume",
                "create",
                "--label",
                "dev.cdelivery.tests.cache=true",
                REGISTRY_VOLUME,
            ]),
            b"",
            COMMAND_TIMEOUT,
        )?;
    }
    Ok(())
}

fn clean(root: &Path) -> Result<()> {
    if root.join("target/tests/current").exists() {
        return Err(
            ErrorKind::Invalid("use cargo xtask env down before cleaning caches".into()).into(),
        );
    }
    base::clean(root)?;
    registry_volume(true)?;
    println!(
        "Prepared bases and registry cache removed; results, shared BuildKit caches and upstream images are retained."
    );
    Ok(())
}
