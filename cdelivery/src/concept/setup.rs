#[derive(Debug)]
pub(crate) enum State {
    NeedsSetup,
    NeedsResume,
    Provisioning,
}

#[derive(Debug, strum::AsRefStr)]
pub(crate) enum Initialization {
    Initialized,
    Resumed,
}
