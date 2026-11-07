use cipher::MasterKey;
use insight_wire::event::Kind;
use jiff::Timestamp;

use crate::error::{ErrorKind, Result};
use crate::{concept, infra};

pub(crate) async fn run_round(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    insight: &infra::insight::query::Api,
    controller: &mut concept::scaling::Controller,
) -> Result<()> {
    let diagnostics = async {
        let results = infra::app::scaling::observe(docker, key).await?;
        controller.retain(|name| results.contains_key(name));
        let mut diagnostics = Vec::new();
        let observations: Vec<_> = results
            .into_iter()
            .filter_map(|(name, result)| match result {
                Ok(observation) => Some(observation),
                Err(error) => {
                    controller.invalidate(&name);
                    tracing::warn!(error = ?error, "automatic scaling could not observe app {name}");
                    diagnostics.push((name, error.chain(crate::locale::Locale::En)));
                    None
                }
            })
            .collect();
        if observations.is_empty() {
            return Ok(diagnostics);
        }
        let ids: Vec<_> = observations
            .iter()
            .map(|observation| observation.service_id().clone())
            .collect();
        let (samples, nodes, agent) = tokio::try_join!(
            insight.latest_cpu(docker, &ids),
            infra::node::observe_all(docker),
            infra::insight::agent::observe(docker),
        )?;
        let (_, interval) = agent.ok_or(ErrorKind::InsightNotEnabled)?;
        let now = Timestamp::now();
        controller.observe(&observations, samples, interval, now);
        for observation in observations {
            let name = observation.app().name();
            match controller.decide(&observation, &nodes, now) {
                concept::scaling::Decision::Hold(Some(reason)) => {
                    diagnostics.push((name.to_string(), reason.to_owned()));
                }
                concept::scaling::Decision::Hold(None) => {}
                concept::scaling::Decision::Scale {
                    replicas,
                    utilization,
                } => {
                    match infra::app::scaling::scale(docker, name, observation.version(), replicas)
                        .await
                    {
                        Ok(()) => {
                            infra::event::emit(
                                Kind::AppScaled,
                                name.as_str(),
                                &format!(
                                    "replicas {} -> {replicas}, average CPU {:.1}%",
                                    observation.app().replicas(),
                                    utilization * 100.0,
                                ),
                            );
                        }
                        Err(error) if error.is_conflict() => {}
                        Err(error) => {
                            tracing::warn!(error = ?error, "automatic scaling failed for {name}");
                            diagnostics
                                .push((name.to_string(), error.chain(crate::locale::Locale::En)));
                        }
                    }
                }
            }
        }
        Ok::<_, crate::error::Error>(diagnostics)
    }
    .await
    .inspect_err(|error| {
        controller.reset();
        infra::event::HUB.raise(
            Kind::AppScalingBlocked,
            "/scaling",
            &error.chain(crate::locale::Locale::En),
            None,
        );
    })?;
    infra::event::HUB.replace(Kind::AppScalingBlocked, diagnostics);
    Ok(())
}
