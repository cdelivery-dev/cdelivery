use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

pub const INDEX_ID: &str = "cdelivery/event";
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr, VariantArray,
)]
pub enum Kind {
    AppCreated,
    AppUpdated,
    AppRolledBack,
    AppRestarted,
    AppDeleted,
    GroupCreated,
    GroupUpdated,
    GroupDeleted,
    NodeRelabeled,
    NodeRemoved,
    NodeOffline,
    NodeOnline,
    RuntimeIssue,
    RuntimeRecovered,
    PingressEnabled,
    PingressUpdated,
    PingressDisabled,
    AcmeEnabled,
    AcmeDisabled,
    CertUploaded,
    CertRemoved,
    CertIssued,
    CertRenewed,
    CertIssueFailed,
    CertIssueRecovered,
    XHealthEnabled,
    XHealthUpdated,
    XHealthTriggered,
    XHealthDisabled,
    XHealthFailed,
    XHealthRecovered,
    XJobCreated,
    XJobUpdated,
    XJobDeleted,
    XJobFailed,
    XJobRecovered,
    GuardianSubmitted,
    GuardianPromoted,
    GuardianRetried,
    GuardianSkipped,
    GuardianCancelled,
    GuardianFailed,
    GuardianRecovered,
    Patrol,
    InsightUpdated,
    PlatformUpdated,
    ReconcileError,
    AcmeIssueError,
    AcmeRenewError,
    AppScaled,
    AppScalingBlocked,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub timestamp: jiff::Timestamp,
    pub kind: Kind,
    pub subject: String,
    pub actor: String,
    pub detail: Option<String>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct Condition {
    pub kind: Kind,
    pub subject: String,
    pub since: jiff::Timestamp,
    pub detail: String,
    pub observed_at: jiff::Timestamp,
}
