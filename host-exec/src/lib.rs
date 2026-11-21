use std::os::fd::AsFd;

use jiff::SignedDuration;
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tokio::time::Instant;
use zbus::{Fd, OwnedObjectPath, Variant};

const ADDRESS: &str = "unix:path=/host/run/dbus/system_bus_socket";
const DESTINATION: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const POLL_INTERVAL: SignedDuration = SignedDuration::from_millis(50);
const STOP_TIMEOUT: SignedDuration = SignedDuration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub(crate) enum ErrorKind {
    #[error("host command I/O failed: {0}")]
    Io(#[source] std::io::Error),
    #[error("host D-Bus request failed; execution may be incomplete: {0}")]
    Bus(#[source] zbus::Error),
    #[error("host command exceeded its deadline; execution may be incomplete")]
    Timeout,
    #[error("a previous host command is still present")]
    Busy,
    #[error("host command failed ({result}, code {code}, status {status}): {stderr}")]
    Command {
        result: String,
        code: i32,
        status: i32,
        stderr: String,
    },
}

macros::located_error! {
    Error => ErrorKind,
    from {
        std::io::Error => Io,
        zbus::Error => Bus,
    }
}

impl Error {
    pub fn is_busy(&self) -> bool {
        matches!(self.kind, ErrorKind::Busy)
    }
}

#[derive(Debug)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub code: i32,
    pub status: i32,
    pub result: String,
}

impl Output {
    pub fn success(self) -> Result<Vec<u8>> {
        if self.result == "success" && self.code == 1 && self.status == 0 {
            Ok(self.stdout)
        } else {
            Err(ErrorKind::Command {
                result: self.result,
                code: self.code,
                status: self.status,
                stderr: String::from_utf8_lossy(&self.stderr).into_owned(),
            }
            .into())
        }
    }
}

#[expect(clippy::too_many_lines)]
pub async fn execute(
    unit: &str,
    program: &str,
    args: &[&str],
    deadline: Instant,
) -> Result<Output> {
    tokio::time::timeout_at(deadline, async {
        let connection = zbus::connection::Builder::address(ADDRESS).build().await?;
        let manager = zbus::Proxy::new(
            &connection,
            DESTINATION,
            MANAGER_PATH,
            "org.freedesktop.systemd1.Manager",
        )
        .await?;
        let (stdout_reader, stdout_writer) = std::os::unix::net::UnixStream::pair()?;
        let (stderr_reader, stderr_writer) = std::os::unix::net::UnixStream::pair()?;
        stdout_reader.set_nonblocking(true)?;
        stderr_reader.set_nonblocking(true)?;
        let mut stdout_reader = UnixStream::from_std(stdout_reader)?;
        let mut stderr_reader = UnixStream::from_std(stderr_reader)?;
        let mut command = vec![program];
        command.extend_from_slice(args);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ErrorKind::Timeout.into());
        }
        let runtime = u64::try_from(remaining.as_micros()).unwrap_or(u64::MAX);
        let stop = u64::try_from(STOP_TIMEOUT.as_micros()).unwrap_or(u64::MAX);
        let properties = vec![
            ("Type", Variant::from("exec")),
            ("Restart", Variant::from("no")),
            ("CollectMode", Variant::from("inactive-or-failed")),
            ("AddRef", Variant::from(true)),
            ("KillMode", Variant::from("control-group")),
            ("TimeoutStartUSec", Variant::from(runtime.min(stop))),
            ("RuntimeMaxUSec", Variant::from(runtime)),
            ("TimeoutStopUSec", Variant::from(stop)),
            ("Environment", Variant::from(vec!["LC_ALL=C"])),
            ("StandardInput", Variant::from("null")),
            (
                "StandardOutputFileDescriptor",
                Variant::from(Fd::from(stdout_writer.as_fd())),
            ),
            (
                "StandardErrorFileDescriptor",
                Variant::from(Fd::from(stderr_writer.as_fd())),
            ),
            (
                "ExecStartEx",
                Variant::from(vec![(program, command, vec!["no-env-expand"])]),
            ),
        ];
        let auxiliary: Vec<(&str, Vec<(&str, Variant<'_>)>)> = Vec::new();
        let started: std::result::Result<OwnedObjectPath, zbus::Error> = manager
            .call("StartTransientUnit", &(unit, "fail", properties, auxiliary))
            .await;
        match started {
            Ok(_) => {}
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "org.freedesktop.systemd1.UnitExists" =>
            {
                return Err(ErrorKind::Busy.into());
            }
            Err(error) => return Err(error.into()),
        }
        drop(stdout_writer);
        drop(stderr_writer);
        let path: OwnedObjectPath = manager.call("GetUnit", &(unit,)).await?;
        let properties = zbus::Proxy::new(
            &connection,
            DESTINATION,
            path.as_str(),
            "org.freedesktop.DBus.Properties",
        )
        .await?;
        let wait = async {
            loop {
                let state: zbus::OwnedVariant = properties
                    .call("Get", &("org.freedesktop.systemd1.Unit", "ActiveState"))
                    .await?;
                if matches!(<&str>::try_from(&state), Ok("inactive" | "failed")) {
                    break;
                }
                tokio::time::sleep(POLL_INTERVAL.unsigned_abs()).await;
            }
            let service = zbus::Proxy::new(
                &connection,
                DESTINATION,
                path.as_str(),
                "org.freedesktop.systemd1.Service",
            )
            .await?;
            let code = service.get_property::<i32>("ExecMainCode").await?;
            let status = service.get_property::<i32>("ExecMainStatus").await?;
            let result = service.get_property::<String>("Result").await?;
            manager.call::<_, _, ()>("UnrefUnit", &(unit,)).await?;
            Ok::<_, Error>((code, status, result))
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let (outcome, _, _) = tokio::try_join!(
            wait,
            async {
                stdout_reader
                    .read_to_end(&mut stdout)
                    .await
                    .map_err(Error::from)
            },
            async {
                stderr_reader
                    .read_to_end(&mut stderr)
                    .await
                    .map_err(Error::from)
            },
        )?;
        loop {
            match manager
                .call::<_, _, OwnedObjectPath>("GetUnit", &(unit,))
                .await
            {
                Err(zbus::Error::MethodError(name, _, _))
                    if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit" =>
                {
                    break;
                }
                Err(error) => return Err(error.into()),
                Ok(_) => tokio::time::sleep(POLL_INTERVAL.unsigned_abs()).await,
            }
        }
        Ok(Output {
            stdout,
            stderr,
            code: outcome.0,
            status: outcome.1,
            result: outcome.2,
        })
    })
    .await
    .map_err(|_| Error::from(ErrorKind::Timeout))?
}
