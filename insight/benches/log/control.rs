use std::net::{Ipv4Addr, SocketAddr};
use std::process::{Child, Command};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use insight::error::{ErrorKind, Result};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

pub(crate) const SERVER_ARGUMENT: &str = "--insight-bench-server";
pub(crate) const RUN_TIMEOUT: Duration = Duration::from_mins(15);
type Transport = Framed<TcpStream, LengthDelimitedCodec>;
pub(crate) type Writer = SplitSink<Transport, Bytes>;
pub(crate) type Reader = SplitStream<Transport>;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RunSpec {
    pub(crate) rows_per_index: u64,
    pub(crate) query_words: [String; 3],
    pub(crate) query_rows: u64,
    pub(crate) trace_id: String,
    pub(crate) trace_rows: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ServerReport {
    pub(crate) parquet_bytes: u64,
    pub(crate) merge_wait: Duration,
    pub(crate) query: Option<Duration>,
    pub(crate) files: Option<std::path::PathBuf>,
    pub(crate) peak_rss_bytes: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum Message {
    Ready { address: SocketAddr, pid: u32 },
    RunSpec(RunSpec),
    Drain,
    Stored,
    Report(ServerReport),
}

pub(crate) fn channel(stream: TcpStream) -> Result<(Writer, Reader)> {
    stream.set_nodelay(true)?;
    let codec = LengthDelimitedCodec::builder()
        .length_field_length(4)
        .max_frame_length(64 * 1024)
        .new_codec();
    Ok(Framed::new(stream, codec).split())
}

pub(crate) async fn send(writer: &mut Writer, message: &Message) -> Result<()> {
    let bytes = serde_json::to_vec(message)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    writer.send(bytes.into()).await?;
    Ok(())
}

pub(crate) async fn receive(reader: &mut Reader) -> Result<Message> {
    let frame: BytesMut = reader
        .next()
        .await
        .ok_or_else(|| ErrorKind::Invariant("benchmark control connection closed".to_owned()))??;
    Ok(serde_json::from_slice(&frame)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?)
}

pub(crate) fn unexpected(message: &Message) -> insight::error::Error {
    ErrorKind::Invariant(format!("unexpected benchmark control message: {message:?}")).into()
}

#[derive(Debug)]
struct Process(Option<Child>);

impl Process {
    async fn wait(&mut self) -> Result<()> {
        let Some(child) = self.0.as_mut() else {
            return Ok(());
        };
        loop {
            if let Some(status) = child.try_wait()? {
                if status.success() {
                    self.0 = None;
                    return Ok(());
                }
                return Err(
                    ErrorKind::Invariant(format!("benchmark server exited with {status}")).into(),
                );
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let Some(child) = self.0.as_mut() else {
            return;
        };
        #[cfg(unix)]
        if let Err(error) = rustix::process::kill_process_group(
            rustix::process::Pid::from_child(child),
            rustix::process::Signal::KILL,
        ) && error != rustix::io::Errno::SRCH
        {
            tracing::error!(error = ?error, "benchmark server process group cleanup failed");
        }
        #[cfg(not(unix))]
        if let Err(error) = child.kill() {
            tracing::error!(error = ?error, "benchmark server cleanup failed");
        }
        if let Err(error) = child.wait() {
            tracing::error!(error = ?error, "benchmark server wait failed");
        }
    }
}

pub(crate) struct Server {
    pub(crate) writer: Writer,
    pub(crate) reader: Reader,
    pub(crate) address: SocketAddr,
    pub(crate) pid: u32,
    process: Process,
}

impl Server {
    pub(crate) async fn start() -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let executable = std::env::current_exe()?;
        let mut command = match std::env::var_os("INSIGHT_BENCH_SERVER_TIME_FILE") {
            Some(path) => {
                if !cfg!(target_os = "linux") {
                    return Err(ErrorKind::Config(
                        "INSIGHT_BENCH_SERVER_TIME_FILE requires Linux GNU time".to_owned(),
                    )
                    .into());
                }
                let mut command = Command::new("/usr/bin/time");
                command.args(["-v", "-o"]).arg(path).arg(&executable);
                command
            }
            None => Command::new(&executable),
        };
        command
            .arg(SERVER_ARGUMENT)
            .arg(listener.local_addr()?.to_string());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let process = Process(Some(command.spawn()?));
        let (stream, _) = tokio::time::timeout(Duration::from_secs(30), listener.accept())
            .await
            .map_err(|_| {
                ErrorKind::Invariant("benchmark server connection timed out".to_owned())
            })??;
        let (writer, mut reader) = channel(stream)?;
        let ready = tokio::time::timeout(Duration::from_secs(30), receive(&mut reader))
            .await
            .map_err(|_| ErrorKind::Invariant("benchmark server startup timed out".to_owned()))??;
        let Message::Ready { address, pid } = ready else {
            return Err(unexpected(&ready));
        };
        Ok(Self {
            writer,
            reader,
            address,
            pid,
            process,
        })
    }

    pub(crate) async fn wait(&mut self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(30), self.process.wait())
            .await
            .map_err(|_| ErrorKind::Invariant("benchmark server exit timed out".to_owned()))?
    }
}
