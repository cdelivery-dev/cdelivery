use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};

use docker::types::TaskState;
use futures_util::StreamExt;
use insight_wire::cpu::{Request, Response};

use crate::config;
use crate::error::Result;

impl super::Api {
    pub(crate) async fn latest_cpu(
        &self,
        docker: &super::super::super::docker::Docker,
        service_ids: &[String],
    ) -> Result<Response> {
        if service_ids.is_empty() {
            return Ok(Response::default());
        }
        let mut addresses = HashSet::new();
        for task in docker.task_list(config::INSIGHT_SERVICE_NAME).await? {
            if task.desired_state != TaskState::Running || task.status.state != TaskState::Running {
                continue;
            }
            if let Some(address) = task
                .networks_attachments
                .iter()
                .flat_map(|network| &network.addresses)
                .next()
            {
                let host = address.split('/').next().unwrap_or(address);
                match host.parse::<IpAddr>() {
                    Ok(ip) => {
                        addresses.insert(SocketAddr::new(ip, insight_wire::HTTP_PORT));
                    }
                    Err(error) => {
                        tracing::warn!(error = ?error, "invalid Insight task address {address}");
                    }
                }
            }
        }
        let selected: HashSet<_> = service_ids.iter().map(String::as_str).collect();
        let requests = service_ids
            .chunks(insight_wire::cpu::MAX_SERVICES)
            .flat_map(|ids| {
                addresses.iter().map(move |address| async move {
                    self.0
                        .post(format!("http://{address}/metrics/cpu/latest"))
                        .json(&Request {
                            service_ids: ids.to_vec(),
                        })
                        .send()
                        .await?
                        .error_for_status()?
                        .json::<Response>()
                        .await
                })
            });
        let mut responses = futures_util::stream::iter(requests)
            .buffer_unordered(config::SCALING_QUERY_CONCURRENCY)
            .take_until(tokio::time::sleep(
                config::SCALING_QUERY_TIMEOUT.unsigned_abs(),
            ))
            .boxed();
        let mut merged = Response::default();
        while let Some(response) = responses.next().await {
            match response {
                Ok(response) => {
                    for (service, samples) in response.services {
                        if !selected.contains(service.as_str()) {
                            continue;
                        }
                        let current = merged.services.entry(service).or_default();
                        for (id, sample) in samples {
                            if current
                                .get(&id)
                                .is_none_or(|value| sample.observed_at > value.observed_at)
                            {
                                current.insert(id, sample);
                            }
                        }
                    }
                    for (id, sample) in response.nodes {
                        if merged
                            .nodes
                            .get(&id)
                            .is_none_or(|value| sample.observed_at > value.observed_at)
                        {
                            merged.nodes.insert(id, sample);
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(error = ?error, "latest CPU query failed for an Insight instance");
                }
            }
        }
        Ok(merged)
    }
}
