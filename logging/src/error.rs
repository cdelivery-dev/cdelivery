use macros::located_error;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ErrorKind {
    #[error("failed to initialize logging")]
    Initialization(#[source] tracing_subscriber::util::TryInitError),
    #[error("core task exited unexpectedly")]
    CoreTaskExited,
}

located_error! {
    Error => ErrorKind,
    from {
        tracing_subscriber::util::TryInitError => Initialization,
    }
}
