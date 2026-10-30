use std::io::{Read, Write};
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ErrorKind {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Failed(String),
}

macros::located_error!(Error => ErrorKind, from { std::io::Error => Io });

pub(crate) fn run(
    command: &mut Command,
    input: &[u8],
    timeout: Duration,
    log: Option<&Path>,
) -> Result<String> {
    let started = Instant::now();
    let output = output(command, input, timeout, log)?;
    if !output.status.success() {
        return Err(ErrorKind::Failed(format!(
            "{}: {} after {:?}: {}",
            command.get_program().to_string_lossy(),
            output.status,
            started.elapsed(),
            diagnostic(&output, log),
        ))
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) fn output(
    command: &mut Command,
    input: &[u8],
    timeout: Duration,
    log: Option<&Path>,
) -> Result<Output> {
    let program = command.get_program().to_string_lossy().into_owned();
    let mut log_file = log.map(std::fs::File::create).transpose()?;
    if let Some(file) = &log_file {
        command.stdout(file.try_clone()?).stderr(file.try_clone()?);
    } else {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    let mut child = command.process_group(0).stdin(Stdio::piped()).spawn()?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let input = input.to_vec();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        if let Some(mut stdin) = stdin {
            stdin.write_all(&input)?;
        }
        Ok(())
    });
    let read = |stream: Option<_>| {
        std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            if let Some(mut stream) = stream {
                Read::read_to_end(&mut stream, &mut bytes)?;
            }
            Ok(bytes)
        })
    };
    let output = read(stdout);
    let errors = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        if let Some(mut stderr) = stderr {
            stderr.read_to_end(&mut bytes)?;
        }
        Ok(bytes)
    });
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            if let Err(error) = rustix::process::kill_process_group(
                rustix::process::Pid::from_child(&child),
                rustix::process::Signal::KILL,
            ) && error != rustix::io::Errno::SRCH
            {
                return Err(std::io::Error::from(error).into());
            }
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let join_error = || ErrorKind::Failed(format!("{program} output thread failed"));
    let stdout = output.join().map_err(|_| join_error())??;
    let stderr = errors.join().map_err(|_| join_error())??;
    let written = writer.join().map_err(|_| join_error())?;
    if let Some(file) = &mut log_file {
        writeln!(
            file,
            "\n{program}: {status}; elapsed {:?}; timed_out={timed_out}",
            start.elapsed()
        )?;
    }
    let output = Output {
        status,
        stdout,
        stderr,
    };
    if timed_out {
        return Err(ErrorKind::Failed(format!(
            "{program}: {status}; timed_out=true after {:?}: {}",
            start.elapsed(),
            diagnostic(&output, log),
        ))
        .into());
    }
    if status.success() {
        written?;
    }
    Ok(output)
}

fn diagnostic(output: &Output, log: Option<&Path>) -> String {
    log.map_or_else(
        || {
            format!(
                "stdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout)
                    .chars()
                    .take(4096)
                    .collect::<String>(),
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(4096)
                    .collect::<String>(),
            )
        },
        |path| format!("see {}", path.display()),
    )
}
