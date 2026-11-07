use jiff::Timestamp;

mod cpu;

use crate::config;
use crate::error::{ErrorKind, Result};

#[derive(Debug)]
pub(crate) struct Api(reqwest::Client);

#[derive(Debug, macros_derive::Ctor)]
pub(crate) struct LogParams {
    range: (Timestamp, Timestamp),
    skip: u32,
    limit: u16,
    words: Vec<String>,
    source: Option<String>,
    node_id: Option<String>,
    container_id: Option<String>,
}

#[derive(serde::Serialize)]
struct AccessPage<'a> {
    skip: u32,
    limit: u16,
    #[serde(flatten)]
    filter: &'a pingress_wire::access::Filter,
}

#[derive(serde::Serialize)]
struct AccessStep<'a> {
    step_secs: u32,
    #[serde(flatten)]
    filter: &'a pingress_wire::access::Filter,
}

impl Api {
    pub(crate) fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(config::INSIGHT_QUERY_TIMEOUT.unsigned_abs())
            .build()?;
        Ok(Self(client))
    }

    pub(crate) async fn host_series(
        &self,
        range: (Timestamp, Timestamp),
        node_id: &str,
        step_secs: u32,
    ) -> Result<Vec<insight_wire::metric::HostBucket>> {
        let params = vec![
            ("step_secs".to_owned(), step_secs.to_string()),
            ("node_id".to_owned(), node_id.to_owned()),
        ];
        self.get(&path_range("/metrics/host", range)?, &params)
            .await
    }

    pub(crate) async fn filesystem_series(
        &self,
        range: (Timestamp, Timestamp),
        node_id: &str,
        step_secs: u32,
    ) -> Result<Vec<insight_wire::metric::FilesystemBucket>> {
        let params = vec![
            ("step_secs".to_owned(), step_secs.to_string()),
            ("node_id".to_owned(), node_id.to_owned()),
        ];
        self.get(&path_range("/metrics/filesystem", range)?, &params)
            .await
    }

    pub(crate) async fn overview(
        &self,
        range: (Timestamp, Timestamp),
        step_secs: u32,
    ) -> Result<Vec<insight_wire::metric::OverviewSeries>> {
        self.get(
            &path_range("/metrics/overview", range)?,
            &[("step_secs", step_secs)],
        )
        .await
    }

    pub(crate) async fn node_latest(
        &self,
        range: (Timestamp, Timestamp),
    ) -> Result<Vec<insight_wire::metric::NodeReport>> {
        self.get(&path_range("/metrics/node-latest", range)?, &())
            .await
    }

    pub(crate) async fn container_series(
        &self,
        range: (Timestamp, Timestamp),
        service_id: String,
        container_id: Option<String>,
        step_secs: u32,
    ) -> Result<Vec<insight_wire::metric::ContainerBucket>> {
        let mut params = vec![
            ("step_secs".to_owned(), step_secs.to_string()),
            ("service_id".to_owned(), service_id),
        ];
        if let Some(container_id) = container_id {
            params.push(("container_id".to_owned(), container_id));
        }
        self.get(&path_range("/metrics/container", range)?, &params)
            .await
    }

    pub(crate) async fn container_instances(
        &self,
        range: (Timestamp, Timestamp),
        service_id: String,
    ) -> Result<Vec<insight_wire::metric::ContainerInstance>> {
        let path = path_range("/metrics/container/instances", range)?;
        let params = vec![("service_id".to_owned(), service_id)];
        self.get(&path, &params).await
    }

    pub(crate) async fn logs(
        &self,
        service_id: &str,
        params: LogParams,
    ) -> Result<Vec<insight_wire::log::Log>> {
        let (path, mut query) = request(
            &format!("/logs/{service_id}"),
            params.range,
            params.skip,
            params.limit,
        )?;
        query.extend(
            [
                ("source", params.source),
                ("node_id", params.node_id),
                ("container_id", params.container_id),
            ]
            .into_iter()
            .filter_map(|(key, value)| Some((key.to_owned(), value?))),
        );
        query.extend(
            params
                .words
                .into_iter()
                .map(|word| ("words".to_owned(), word)),
        );
        self.get(&path, &query).await
    }

    pub(crate) async fn access(
        &self,
        range: (Timestamp, Timestamp),
        skip: u32,
        limit: u16,
        filter: &pingress_wire::access::Filter,
    ) -> Result<Vec<pingress_wire::access::Access>> {
        let path = path_range("/pingress/access", range)?;
        let page = AccessPage {
            skip,
            limit,
            filter,
        };
        self.get(&path, &page).await
    }

    pub(crate) async fn access_stats(
        &self,
        range: (Timestamp, Timestamp),
        step_secs: u32,
        filter: &pingress_wire::access::Filter,
    ) -> Result<Vec<pingress_wire::access::StatsRow>> {
        let path = path_range("/pingress/access/stats", range)?;
        self.get(&path, &AccessStep { step_secs, filter }).await
    }

    pub(crate) async fn access_status_stats(
        &self,
        range: (Timestamp, Timestamp),
        step_secs: u32,
        filter: &pingress_wire::access::Filter,
    ) -> Result<Vec<pingress_wire::access::StatusRow>> {
        let path = path_range("/pingress/access/stats/status", range)?;
        self.get(&path, &AccessStep { step_secs, filter }).await
    }

    pub(crate) async fn events(
        &self,
        range: (Timestamp, Timestamp),
        skip: u32,
        limit: u16,
        kind: Option<insight_wire::event::Kind>,
        subject: Option<&str>,
    ) -> Result<Vec<insight_wire::event::Event>> {
        let (path, mut params) = request("/events", range, skip, limit)?;
        if let Some(kind) = kind {
            params.push(("kind".to_owned(), kind.as_str().to_owned()));
        }
        if let Some(subject) = subject {
            params.push(("subject".to_owned(), subject.to_owned()));
        }
        self.get(&path, &params).await
    }

    pub(crate) async fn messages(
        &self,
        range: (Timestamp, Timestamp),
        limit: u16,
        timeline: String,
    ) -> Result<Vec<insight_wire::message::Message>> {
        let (path, mut params) = request("/messages", range, 0, limit)?;
        params.push(("timeline".to_owned(), timeline));
        self.get(&path, &params).await
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        params: &impl serde::Serialize,
    ) -> Result<Vec<T>> {
        let url = format!(
            "http://{}:{}{path}",
            insight_wire::SERVICE_NAME,
            insight_wire::HTTP_PORT
        );
        let response = self.0.get(url).query(params).send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes);
            let detail = super::super::event::truncate_bytes(&body, 500);
            return Err(ErrorKind::InsightQueryFailed(format!("{status} {detail}")).into());
        }
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}

fn path_range(path: &str, range: (Timestamp, Timestamp)) -> Result<String> {
    let (start, end) = range;
    if start >= end {
        return Err(ErrorKind::InvalidBody(format!(
            "invalid time range: start {start} is not before end {end}"
        ))
        .into());
    }
    Ok(format!("{path}/{}/{}", nanos(start)?, nanos(end)?))
}

fn request(
    path: &str,
    range: (Timestamp, Timestamp),
    skip: u32,
    limit: u16,
) -> Result<(String, Vec<(String, String)>)> {
    Ok((
        path_range(path, range)?,
        vec![
            ("skip".to_owned(), skip.to_string()),
            ("limit".to_owned(), limit.to_string()),
        ],
    ))
}

fn nanos(at: Timestamp) -> Result<i64> {
    i64::try_from(at.as_nanosecond()).map_err(|_| {
        ErrorKind::InvalidBody(format!("timestamp {at} is outside the insight query range")).into()
    })
}
