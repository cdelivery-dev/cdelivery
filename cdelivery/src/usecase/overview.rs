use cipher::MasterKey;
use jiff::Timestamp;

use crate::concept;
use crate::config;
use crate::error::Result;
use crate::infra;

struct Observed {
    nodes: Vec<concept::node::Node>,
    deployment: concept::insight::Deployment,
    summary: concept::overview::Summary,
}

async fn observe(docker: &infra::docker::Docker, key: &MasterKey) -> Result<Observed> {
    let (nodes, units, deployment, pingress, insight_server, insight_agents) = tokio::try_join!(
        infra::node::observe_all(docker),
        infra::app::units(docker),
        super::insight::overview(docker, key),
        infra::platform::observe_instances(docker, config::PINGRESS_SERVICE_NAME),
        infra::platform::observe_instances(docker, config::INSIGHT_SERVICE_NAME),
        infra::platform::observe_instances(docker, config::INSIGHT_AGENT_SERVICE_NAME),
    )?;
    let summary = concept::overview::summary(
        &nodes,
        &units,
        &deployment,
        pingress,
        insight_server,
        insight_agents,
    );
    Ok(Observed {
        nodes,
        deployment,
        summary,
    })
}

pub(crate) async fn summary(
    docker: &infra::docker::Docker,
    key: &MasterKey,
) -> Result<concept::overview::Summary> {
    Ok(observe(docker, key).await?.summary)
}

pub(crate) async fn load(
    docker: &infra::docker::Docker,
    api: &infra::insight::query::Api,
    key: &MasterKey,
    range: (Timestamp, Timestamp),
    bucket_step_secs: u32,
) -> Result<(concept::overview::Summary, concept::overview::Metrics)> {
    let observed = observe(docker, key).await?;
    let metrics = match metrics(
        api,
        &observed.deployment,
        &observed.nodes,
        range,
        bucket_step_secs,
    )
    .await
    {
        Ok(metrics) => metrics,
        Err(error) => {
            tracing::warn!(error = ?error, "overview metrics unavailable");
            concept::overview::Metrics::Unavailable(error.chain(crate::locale::Locale::En))
        }
    };
    Ok((observed.summary, metrics))
}

async fn metrics(
    api: &infra::insight::query::Api,
    deployment: &concept::insight::Deployment,
    nodes: &[concept::node::Node],
    range: (Timestamp, Timestamp),
    bucket_step_secs: u32,
) -> Result<concept::overview::Metrics> {
    let (Some(_), Some((_, sample_interval))) = (deployment.server(), deployment.agent()) else {
        return Ok(concept::overview::Metrics::Disabled);
    };
    let sample_interval = u32::from(*sample_interval);
    let step_secs = (bucket_step_secs
        .max(sample_interval)
        .div_ceil(sample_interval)
        * sample_interval)
        .min(insight_wire::metric::STEP_SECS_MAX);
    let filter = pingress_wire::access::Filter::default();
    let (series, access) = tokio::try_join!(
        api.overview(range, step_secs),
        api.access_stats(range, bucket_step_secs, &filter),
    )?;
    Ok(concept::overview::Metrics::Ready {
        step_secs,
        series,
        node_names: nodes
            .iter()
            .map(|node| (node.id().clone(), node.hostname().clone()))
            .collect(),
        access_step_secs: bucket_step_secs,
        access,
    })
}
