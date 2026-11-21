use std::time::Duration;

use tokio::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const UNIT: &str = "cdelivery-host-exec-test.service";

#[tokio::test]
async fn native_execution() -> Result<()> {
    let run = async |program, args: &[&str]| {
        host_exec::execute(
            UNIT,
            program,
            args,
            Instant::now() + Duration::from_secs(30),
        )
        .await
    };
    let output = run("/bin/true", &[]).await?;
    ensure(
        output.success()?.is_empty(),
        "fast exit must retain its status",
    )?;
    let output = run(
        "/bin/sh",
        &[
            "-c",
            "printf '%s' \"$1\"; printf failure >&2; exit 7",
            "sh",
            "$HOME %n",
        ],
    )
    .await?;
    ensure(
        output.stdout == b"$HOME %n"
            && output.stderr == b"failure"
            && output.code == 1
            && output.status == 7,
        "arguments, separate streams and nonzero status must be preserved",
    )?;
    ensure(output.success().is_err(), "nonzero exit must fail")?;
    let output = run(
        "/bin/sh",
        &[
            "-c",
            "head -c 1048576 /dev/zero & head -c 1048576 /dev/zero >&2 & wait",
        ],
    )
    .await?;
    ensure(
        output.stdout.len() == 1_048_576 && output.stderr.len() == 1_048_576,
        "both output streams must drain completely without blocking each other",
    )?;
    output.success()?;
    ensure(
        run("/cdelivery-missing-executable", &[])
            .await?
            .success()
            .is_err(),
        "exec failure must be reported",
    )?;
    let output = run("/bin/sh", &["-c", "kill -TERM $$"]).await?;
    ensure(
        output.code == 2 && output.status == 15 && output.success().is_err(),
        "signal exit must be reported",
    )?;
    ensure(
        host_exec::execute(
            UNIT,
            "/bin/sleep",
            &["30"],
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .is_err(),
        "a command must not outlive the caller's wait budget",
    )?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match run("/bin/true", &[]).await {
            Ok(output) => {
                output.success()?;
                break;
            }
            Err(error) if error.is_busy() && Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(std::io::Error::other(message.to_owned()).into())
    }
}
