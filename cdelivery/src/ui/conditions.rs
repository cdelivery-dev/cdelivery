use insight_wire::event::{Condition, Kind};

use crate::{concept, operation};

pub(super) type Result = std::result::Result<Vec<Condition>, String>;

pub(super) async fn load(app: &crate::boot::App, access: &concept::permission::Access) -> Result {
    super::read::<operation::event::Conditions>(app, access)
        .await
        .map(|output| output.conditions)
        .map_err(|error| super::read_error(&error, "recorded condition query failed"))
}

pub(super) fn for_app(
    mut conditions: Vec<Condition>,
    spec: &operation::app::typ::view::AppSpec,
) -> Vec<Condition> {
    conditions.retain(|condition| match condition.kind {
        Kind::RuntimeIssue | Kind::XHealthFailed | Kind::GuardianFailed => {
            condition.subject == spec.name
        }
        Kind::AppScalingBlocked => {
            matches!(
                spec.replicas,
                operation::app::typ::replicas::Replicas::Dynamic { .. }
            ) && (condition.subject == spec.name || condition.subject == "/scaling")
        }
        Kind::XJobFailed | Kind::ReconcileError => condition
            .subject
            .split_once('/')
            .is_some_and(|(app, _)| app == spec.name),
        Kind::CertIssueFailed => spec
            .domains
            .iter()
            .any(|rule| rule.https && rule.domain == condition.subject),
        _ => false,
    });
    conditions
}
