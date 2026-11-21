use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{COMMAND_TIMEOUT, ErrorKind, Result, execute, machine};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Base {
    name: String,
    versions: String,
}

pub(super) fn machines() -> Result<BTreeMap<String, String>> {
    let output = execute(
        Command::new("orbctl").args(["list", "--format", "json"]),
        b"",
        COMMAND_TIMEOUT,
    )?;
    let machines: Vec<serde_json::Value> = serde_json::from_str(&output)?;
    machines
        .iter()
        .map(
            |value| match (value["name"].as_str(), value["state"].as_str()) {
                (Some(name), Some(state)) => Ok((name.to_owned(), state.to_owned())),
                _ => Err(ErrorKind::Invalid(
                    "OrbStack returned a machine without a name or state".into(),
                )
                .into()),
            },
        )
        .collect()
}

#[expect(clippy::too_many_lines)]
pub(super) fn prepare(root: &Path, artifacts: &Path) -> Result<String> {
    let script = include_str!("../../../tests/fixtures/base.sh");
    let signature = Sha256::digest(format!("ubuntu:24.04:{}:{script}", std::env::consts::ARCH));
    let signature: String = signature
        .iter()
        .take(16)
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(nibble.into(), 16))
        .collect();
    let name = format!("cdelivery-test-base-{signature}");
    let cache = root.join("target/tests/cache");
    fs::create_dir_all(&cache)?;
    let path = cache.join(format!("{name}.json"));
    let machines = machines()?;
    let existing = machines.get(&name);
    if existing.is_some() && !path.exists() {
        return Err(ErrorKind::Invalid(format!(
            "base {name} has no ownership record; refusing to modify it"
        ))
        .into());
    }
    if existing.is_some() && path.exists() {
        let base: Base = serde_json::from_slice(&fs::read(&path)?)?;
        if base.name == name
            && !base.versions.is_empty()
            && existing.is_some_and(|state| state == "stopped")
        {
            fs::copy(&path, artifacts.join("base.json"))?;
            println!("Reusing prepared base {name}");
            return Ok(name);
        }
    }
    let mut base = Base {
        name: name.clone(),
        versions: String::new(),
    };
    fs::write(&path, serde_json::to_vec_pretty(&base)?)?;
    if existing.is_none() {
        println!("Creating prepared base {name}");
        crate::process::run(
            Command::new("orbctl").args(["create", "ubuntu:24.04", &name]),
            b"",
            Duration::from_mins(15),
            Some(&artifacts.join("base-create.log")),
        )?;
    }
    execute(
        &mut machine(
            &name,
            &[
                "sh",
                "-c",
                "if [ \"$(systemctl show --property=ActiveState --value cdelivery-test-prepare.service)\" = activating ]; then cat > /dev/null; else cat > /root/cdelivery-test-base.sh; fi",
            ],
        ),
        script.as_bytes(),
        COMMAND_TIMEOUT,
    )?;
    execute(&mut machine(&name, &["sh", "-s"]), br#"set -eu
if test -f /root/cdelivery-test-base.ready; then exit 0; fi
if [ "$(systemctl show --property=ActiveState --value cdelivery-test-prepare.service)" = activating ]; then exit 0; fi
cat > /etc/systemd/system/cdelivery-test-prepare.service <<'UNIT'
[Unit]
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/sh /root/cdelivery-test-base.sh
TimeoutStartSec=30min
StandardOutput=append:/root/cdelivery-test-base.log
StandardError=append:/root/cdelivery-test-base.log
UNIT
systemctl daemon-reload
systemctl start --no-block cdelivery-test-prepare.service
"#, COMMAND_TIMEOUT)?;
    fs::write(
        artifacts.join("base-task.txt"),
        execute(
            &mut machine(
                &name,
                &[
                    "systemctl",
                    "show",
                    "--property=InvocationID,ActiveState,MainPID,Result",
                    "cdelivery-test-prepare.service",
                ],
            ),
            b"",
            COMMAND_TIMEOUT,
        )?,
    )?;
    let start = Instant::now();
    loop {
        let output = execute(
            &mut machine(
                &name,
                &[
                    "sh",
                    "-c",
                    "if test -f /root/cdelivery-test-base.log; then cat /root/cdelivery-test-base.log; fi",
                ],
            ),
            b"",
            COMMAND_TIMEOUT,
        )?;
        fs::write(artifacts.join("base-install.log"), output)?;
        let status = execute(
            &mut machine(
                &name,
                &[
                    "sh",
                    "-c",
                    "if test -f /root/cdelivery-test-base.ready; then echo ready; else systemctl show --property=ActiveState --value cdelivery-test-prepare.service; fi",
                ],
            ),
            b"",
            COMMAND_TIMEOUT,
        )?;
        match status.trim() {
            "ready" => break,
            "activating" if start.elapsed() < Duration::from_mins(31) => std::thread::sleep(Duration::from_secs(2)),
            status => return Err(ErrorKind::Invalid(format!("base preparation state={status}; see {}; the base and downloads are retained; down then up retries or reattaches to the same systemd task", artifacts.join("base-install.log").display())).into()),
        }
    }
    base.versions = execute(
        &mut machine(&name, &["cat", "/root/cdelivery-test-base.versions"]),
        b"",
        COMMAND_TIMEOUT,
    )?;
    execute(
        &mut machine(
            &name,
            &[
                "sh",
                "-c",
                "rm -f /var/lib/dbus/machine-id /var/lib/systemd/random-seed; truncate -s 0 /etc/machine-id",
            ],
        ),
        b"",
        COMMAND_TIMEOUT,
    )?;
    execute(
        Command::new("orbctl").args(["stop", &name]),
        b"",
        COMMAND_TIMEOUT,
    )?;
    fs::write(&path, serde_json::to_vec_pretty(&base)?)?;
    fs::copy(&path, artifacts.join("base.json"))?;
    println!("Prepared base in {:?}", start.elapsed());
    Ok(name)
}

pub(super) fn clean(root: &Path) -> Result<()> {
    let cache = root.join("target/tests/cache");
    if !cache.exists() {
        return Ok(());
    }
    let existing = machines()?;
    for entry in fs::read_dir(&cache)? {
        let path = entry?.path();
        let Some(filename) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(signature) = filename.strip_prefix("cdelivery-test-base-") else {
            continue;
        };
        if signature.len() != 32
            || !signature.bytes().all(|byte| byte.is_ascii_hexdigit())
            || path.extension().is_none_or(|extension| extension != "json")
        {
            return Err(ErrorKind::Invalid(
                "invalid base cache ownership; refusing cleanup".into(),
            )
            .into());
        }
        let base: Base = serde_json::from_slice(&fs::read(&path)?)?;
        if base.name != filename {
            return Err(
                ErrorKind::Invalid("base cache name differs from its record".into()).into(),
            );
        }
        if existing.contains_key(&base.name) {
            execute(
                Command::new("orbctl").args(["delete", "--force", &base.name]),
                b"",
                COMMAND_TIMEOUT,
            )?;
        }
        fs::remove_file(path)?;
    }
    Ok(())
}
