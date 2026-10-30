use macros::located_error;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ErrorKind {
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("insight sender task exited unexpectedly")]
    SenderExited,
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
    #[error("MessagePack decode error: {0}")]
    MsgPack(#[source] rmpv::decode::Error),
    #[error("Docker error: {0}")]
    Docker(#[source] docker::Error),
    #[error("schema error: {0}")]
    Schema(#[source] insight_schema::Error),
    #[error("runtime error: {0}")]
    Runtime(#[source] logging::Error),
}

located_error! {
    Error => ErrorKind,
    from {
        std::io::Error => Io,
        rmpv::decode::Error => MsgPack,
        docker::Error => Docker,
        insight_schema::Error => Schema,
        logging::Error => Runtime,
    }
}
