use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NanoDuration(pub u64);

impl NanoDuration {
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs.saturating_mul(1_000_000_000))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NanoCpus(pub u64);

impl NanoCpus {
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn from_cores(cores: f64) -> Self {
        Self((cores.max(0.0) * 1e9).round() as u64)
    }

    #[expect(clippy::cast_precision_loss)]
    pub fn cores(&self) -> f64 {
        self.0 as f64 / 1e9
    }
}
