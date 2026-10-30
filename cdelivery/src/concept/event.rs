use insight_wire::event::Kind;

const CONDITIONS: [(Kind, Kind); 6] = [
    (Kind::NodeOffline, Kind::NodeOnline),
    (Kind::RuntimeIssue, Kind::RuntimeRecovered),
    (Kind::CertIssueFailed, Kind::CertIssueRecovered),
    (Kind::XHealthFailed, Kind::XHealthRecovered),
    (Kind::XJobFailed, Kind::XJobRecovered),
    (Kind::GuardianFailed, Kind::GuardianRecovered),
];

pub(crate) fn recovery(kind: Kind) -> Option<Kind> {
    CONDITIONS
        .into_iter()
        .find(|&(failure, _)| failure == kind)
        .map(|(_, recovery)| recovery)
}

pub(crate) fn attention(kind: Kind) -> bool {
    CONDITIONS
        .into_iter()
        .any(|(failure, recovery)| failure == kind || recovery == kind)
}
