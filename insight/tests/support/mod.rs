use std::future::Future;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ErrorKind {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Process(#[from] crate::process::Error),
    #[error(transparent)]
    Store(#[from] insight::error::Error),
    #[error(transparent)]
    ObjectStore(#[from] object_store::Error),
    #[error(transparent)]
    Schema(#[from] insight_schema::Error),
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Timeout(#[from] tokio::time::error::Elapsed),
    #[error("{0}")]
    Check(String),
}

macros::located_error!(Error => ErrorKind, from {
    std::io::Error => Io, serde_json::Error => Json, crate::process::Error => Process,
    insight::error::Error => Store, object_store::Error => ObjectStore,
    insight_schema::Error => Schema, tokio::task::JoinError => Join,
    tokio::time::error::Elapsed => Timeout,
});

pub(crate) struct Context {
    pub(crate) root: PathBuf,
    project: String,
}

impl Context {
    pub(crate) fn compose(&self, args: &[&str]) -> Result<String> {
        Ok(crate::process::run(
            Command::new("docker")
                .args([
                    "--context",
                    "orbstack",
                    "compose",
                    "--project-name",
                    &self.project,
                    "--file",
                ])
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/compose.yaml"))
                .args(args),
            b"",
            Duration::from_secs(180),
            None,
        )?)
    }
}

#[expect(clippy::print_stderr)]
pub(crate) async fn run(remote: bool) -> Result<()> {
    let state = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/tests");
    let artifacts = state.join("results").join(name(if remote {
        "storage-s3"
    } else {
        "storage-local"
    }));
    std::fs::create_dir_all(&artifacts)?;
    std::fs::set_permissions(
        state.join("results"),
        std::fs::Permissions::from_mode(0o700),
    )?;
    let started = Instant::now();
    let mut context = Context {
        root: artifacts.join("data"),
        project: String::new(),
    };
    let mut result = match tokio::time::timeout(Duration::from_mins(20), async {
        if remote {
            ensure(
                state.join("current/env.json").is_file(),
                "no current environment; run cargo xtask env up storage",
            )?;
            let bytes = std::fs::read(state.join("current/env.json"))?;
            let environment: serde_json::Value = serde_json::from_slice(&bytes)?;
            ensure(
                environment
                    .get("profile")
                    .and_then(serde_json::Value::as_str)
                    == Some("storage")
                    && environment
                        .get("ready")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true),
                "run cargo xtask env up storage first",
            )?;
            let owner = environment
                .get("owner")
                .and_then(serde_json::Value::as_str)
                .filter(|owner| uuid::Uuid::parse_str(owner).is_ok())
                .ok_or_else(|| ErrorKind::Check("invalid environment owner".into()))?;
            context.project = format!("cdelivery-tests-{owner}");
            std::fs::write(artifacts.join("env.json"), bytes)?;
        }
        crate::durability(&context, remote).await
    })
    .await
    {
        Ok(result) => result,
        Err(error) => Err(error.into()),
    };
    if !context.project.is_empty() {
        if result.is_err() {
            let logs = context.compose(&["logs", "--no-color", "--tail", "200", "garage"]);
            if let Err(error) = std::fs::write(artifacts.join("garage.log"), format!("{logs:?}")) {
                eprintln!("recording Garage diagnostics failed: {error:?}");
            }
        }
        if let Err(error) = context.compose(&["start", "garage"]) {
            eprintln!("restoring Garage failed: {error:?}");
            if let Err(write_error) =
                std::fs::write(artifacts.join("restore.txt"), format!("{error:?}"))
            {
                eprintln!("recording restoration failure failed: {write_error:?}");
            }
            if result.is_ok() {
                result = Err(error);
            }
        }
    }
    if result.is_ok() {
        std::fs::remove_dir_all(&context.root)?;
    }
    if let Err(error) = std::fs::write(
        artifacts.join("result.txt"),
        format!("{result:?}\nelapsed {:?}\n", started.elapsed()),
    ) {
        eprintln!("recording storage result failed: {error:?}");
        if result.is_ok() {
            return Err(error.into());
        }
    }
    result
}

pub(crate) fn name(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

#[track_caller]
pub(crate) fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(ErrorKind::Check(message.into()).into())
    }
}

pub(crate) async fn eventually<T, F: Future<Output = Result<T>>>(
    description: &str,
    mut check: impl FnMut() -> F,
) -> Result<T> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let mut pending = String::from("probe did not complete");
    loop {
        match tokio::time::timeout_at(deadline, check()).await {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) if matches!(error.kind, ErrorKind::Check(_)) => {
                pending = format!("{error:?}");
                tokio::time::sleep_until(
                    deadline.min(tokio::time::Instant::now() + Duration::from_secs(2)),
                )
                .await;
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                return Err(ErrorKind::Check(format!(
                    "timed out waiting for {description}: {pending}"
                ))
                .into());
            }
        }
    }
}
