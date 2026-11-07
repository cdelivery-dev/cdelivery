use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;
use garde::Validate;
use http::StatusCode;
use jiff::Timestamp;

use crate::boot::App;
use crate::error::ErrorKind;
use crate::locale::tt;
use crate::{concept, operation};

#[derive(serde::Deserialize, Validate)]
#[serde(deny_unknown_fields)]
struct DomainParams {
    #[garde(custom(operation::validate::valid_domain))]
    domain: String,
    #[garde(dive)]
    placement: Option<operation::node::typ::NodeSelector>,
}

#[derive(serde::Deserialize, Validate)]
#[serde(deny_unknown_fields)]
struct SetupParams {
    #[garde(dive)]
    domain: Option<DomainParams>,
    #[garde(dive)]
    insight: operation::insight::typ::EnableInsightRequest,
}

#[derive(askama::Template)]
#[template(path = "onboarding.html")]
struct Page {
    account: super::account::Account,
    current: operation::instance::typ::Instance,
    pingress: Option<operation::pingress::typ::Pingress>,
    insight: operation::insight::typ::Insight,
    nodes: operation::node::typ::Nodes,
}

pub(super) fn routes() -> axum::Router<Arc<App>> {
    axum::Router::new().route("/setup/platform", axum::routing::get(get).post(save))
}

async fn get(
    authorized: super::auth::Authorized<operation::instance::Update>,
    State(app): State<Arc<App>>,
) -> super::response::Result<Response> {
    let access = authorized.access();
    let (current, insight) = tokio::join!(
        super::read::<operation::instance::Get>(&app, access),
        super::read::<operation::insight::Overview>(&app, access),
    );
    let current = current?;
    let insight = insight?;
    let (pingress, nodes) = tokio::join!(
        super::pingress::load_pingress(&app, access),
        super::read::<operation::node::List>(&app, access),
    );
    super::response::render(
        &Page {
            account: super::account::Account::new(access),
            current,
            pingress: pingress?,
            insight,
            nodes: nodes?,
        },
        StatusCode::OK,
    )
}

async fn save(
    authorized: super::auth::Authorized<operation::instance::Update>,
    State(app): State<Arc<App>>,
    super::command::Payload(mut input): super::command::Payload<SetupParams>,
) -> super::response::Result<Response> {
    let access = authorized.access();
    if let Some(domain) = &mut input.domain {
        domain.domain = domain.domain.trim().to_owned();
        if domain.domain.is_empty() {
            input.domain = None;
        }
    }
    input.insight.server_version = None;
    input.insight.agent_version = None;
    input.insight.sample_interval_secs = None;
    input.insight.flush_interval = None;
    let report = operation::validate::report(&input);
    if !report.is_empty() {
        return Err(ErrorKind::Validation(report).into());
    }
    if let Some(domain) = input.domain {
        save_domain(&app, access, domain)
            .await
            .map_err(|error| error.nested("domain"))?;
    }
    enable_insight(&app, access, input.insight)
        .await
        .map_err(|error| error.nested("insight"))?;
    Ok(super::command::complete("/"))
}

async fn save_domain(
    app: &App,
    access: &concept::permission::Access,
    input: DomainParams,
) -> crate::error::Result<()> {
    if super::read::<operation::instance::Get>(app, access)
        .await?
        .domain
        .is_some()
    {
        return Ok(());
    }
    let domain = input
        .domain
        .parse::<concept::domain::DomainName>()?
        .to_string();
    if let Some(pingress) = super::pingress::load_pingress(app, access).await? {
        if !pingress.acme_enabled {
            let certs = super::read::<operation::pingress::ListCerts>(app, access).await?;
            if !certs.certs.iter().any(|entry| {
                entry.domain == domain
                    && entry.certificate.as_ref().is_some_and(|cert| !cert.expired)
            }) {
                return Err(ErrorKind::InvalidBody(
                    tt!("ui.onboarding.certificateRequired").into_owned(),
                )
                .into());
            }
        }
    } else {
        let placement = input
            .placement
            .ok_or_else(|| ErrorKind::FieldRequired("placement".into()))?;
        require_placement(app, access, &placement).await?;
        let [release] =
            super::latest_versions(app, access, true, [concept::release::Component::Pingress])
                .await;
        let version = release.ok_or(ErrorKind::ReleaseUnavailable)?.version;
        operation::execute::<operation::pingress::Enable>(
            app,
            access,
            operation::pingress::typ::CreatePingressRequest {
                version,
                placement,
                acme: Some(operation::pingress::typ::CaChoice::LetsEncrypt),
            },
            Timestamp::now(),
        )
        .await?;
    }
    operation::execute::<operation::instance::Update>(
        app,
        access,
        operation::instance::typ::UpdateInstanceRequest {
            version: None,
            domain: Some(Some(operation::instance::typ::Domain {
                domain,
                https: true,
            })),
        },
        Timestamp::now(),
    )
    .await?;
    Ok(())
}

async fn enable_insight(
    app: &App,
    access: &concept::permission::Access,
    mut input: operation::insight::typ::EnableInsightRequest,
) -> crate::error::Result<()> {
    let current = super::read::<operation::insight::Overview>(app, access).await?;
    if current.server.is_some() {
        input.storage = None;
        input.retention_days = None;
        input.placement = None;
    } else if let Some(placement) = &input.placement {
        require_placement(app, access, placement).await?;
    }
    let [server, agent] = super::latest_versions(
        app,
        access,
        current.server.is_none() || current.agent.is_none(),
        [
            concept::release::Component::Insight,
            concept::release::Component::InsightAgent,
        ],
    )
    .await;
    if current.server.is_none() {
        input.server_version = Some(server.ok_or(ErrorKind::ReleaseUnavailable)?.version);
    }
    if current.agent.is_none() {
        input.agent_version = Some(agent.ok_or(ErrorKind::ReleaseUnavailable)?.version);
    }
    operation::execute::<operation::insight::Enable>(app, access, input, Timestamp::now()).await?;
    Ok(())
}

async fn require_placement(
    app: &App,
    access: &concept::permission::Access,
    placement: &operation::node::typ::NodeSelector,
) -> crate::error::Result<()> {
    let nodes = super::read::<operation::node::List>(app, access).await?;
    if nodes.nodes.iter().any(|node| {
        matches!(node.state, operation::node::typ::NodeState::Ready)
            && matches!(
                node.availability,
                operation::node::typ::NodeAvailability::Active
            )
            && match placement {
                operation::node::typ::NodeSelector::NodeId(id) => &node.id == id,
                operation::node::typ::NodeSelector::Label(label) => node.labels.contains(label),
            }
    }) {
        Ok(())
    } else {
        Err(ErrorKind::InvalidBody(tt!("ui.onboarding.placementUnavailable").into_owned()).into())
    }
}
