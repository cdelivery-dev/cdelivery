use super::{Client, filter_query};
use crate::error::{ErrorKind, Result};
use crate::types::{Container, ContainerDetail, Filter};

#[derive(Debug)]
pub struct Containers<'a>(pub(crate) &'a Client);

impl Containers<'_> {
    pub async fn list(
        &self,
        all: bool,
        tasks_only: bool,
        filter: &Filter,
    ) -> Result<Vec<Container>> {
        let mut filter = filter.clone();
        if tasks_only {
            filter = filter.add("is-task", "true");
        }
        let mut query = vec![("all", all.to_string())];
        query.extend(filter_query(&filter));
        self.0.list("/containers/json", &query).await
    }

    pub async fn inspect(&self, id: &str) -> Result<Option<ContainerDetail>> {
        self.0.get_opt(&format!("/containers/{id}/json"), &[]).await
    }

    pub async fn create(&self, name: &str, spec: &serde_json::Value) -> Result<String> {
        #[derive(serde::Deserialize)]
        struct Created {
            #[serde(rename = "Id")]
            id: String,
        }
        let created: Created = self
            .0
            .post_json(
                "/containers/create",
                &[("name", name.to_owned())],
                Some(spec.to_string()),
                None,
            )
            .await?;
        Ok(created.id)
    }

    pub async fn start(&self, id: &str) -> Result<()> {
        self.0
            .post_discard(&format!("/containers/{id}/start"), &[], None, None)
            .await
    }

    pub async fn wait(&self, id: &str, timeout: std::time::Duration) -> Result<i64> {
        #[derive(serde::Deserialize)]
        struct Exit {
            #[serde(rename = "StatusCode")]
            status: i64,
        }
        let bytes = self
            .0
            .request_timeout(
                http::Method::POST,
                &format!("/containers/{id}/wait"),
                &[("condition", "not-running".to_owned())],
                None,
                None,
                timeout,
            )
            .await?;
        let exit: Exit = serde_json::from_slice(&bytes)?;
        Ok(exit.status)
    }

    pub async fn logs(&self, id: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        let bytes = self
            .0
            .request(
                http::Method::GET,
                &format!("/containers/{id}/logs"),
                &[("stdout", "true".to_owned()), ("stderr", "true".to_owned())],
                None,
                None,
            )
            .await?;
        let mut remaining = bytes.as_ref();
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        while !remaining.is_empty() {
            let (header, rest) = remaining.split_at_checked(8).ok_or(ErrorKind::LogFrame)?;
            let [stream, 0, 0, 0, a, b, c, d] = *header else {
                return Err(ErrorKind::LogFrame.into());
            };
            let length = u32::from_be_bytes([a, b, c, d]) as usize;
            let (payload, rest) = rest.split_at_checked(length).ok_or(ErrorKind::LogFrame)?;
            match stream {
                1 => stdout.extend_from_slice(payload),
                2 => stderr.extend_from_slice(payload),
                _ => return Err(ErrorKind::LogFrame.into()),
            }
            remaining = rest;
        }
        Ok((stdout, stderr))
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        self.0
            .delete_not_found_ok(&format!("/containers/{id}"), &[])
            .await
    }
}
