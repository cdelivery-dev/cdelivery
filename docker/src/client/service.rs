use http::StatusCode;

use super::{Client, filter_query, to_json_body};
use crate::error::{Error, ErrorKind, Result};
use crate::types::{
    Filter, RegistryAuth, Service, ServiceCreateResponse, ServiceSpec, ServiceStatus,
    UpdateResponse,
};

#[derive(Debug)]
pub struct Services<'a>(pub(crate) &'a Client);

impl Services<'_> {
    pub async fn list(&self, filter: &Filter) -> Result<Vec<Service>> {
        let mut query = vec![("status", "true".to_owned())];
        query.extend(filter_query(filter));
        let mut services: Vec<Service> = self.0.list("/services", &query).await?;
        services.sort_by_key(|s| s.created_at);
        Ok(services)
    }

    pub async fn inspect(&self, name: &str) -> Result<Option<Service>> {
        self.0.get_opt(&format!("/services/{name}"), &[]).await
    }

    pub async fn status(&self, name: &str) -> Result<Option<ServiceStatus>> {
        let filter = Filter::new().name(name);
        let Some(service) = self
            .list(&filter)
            .await?
            .into_iter()
            .find(|service| service.spec.name == name)
        else {
            return Ok(None);
        };
        let status = service
            .status
            .ok_or_else(|| Error::new(ErrorKind::MissingServiceStatus(name.to_owned())))?;
        Ok(Some(status))
    }

    pub async fn create(
        &self,
        spec: &ServiceSpec,
        auth: Option<&RegistryAuth>,
    ) -> Result<ServiceCreateResponse> {
        self.0
            .post_json("/services/create", &[], Some(to_json_body(spec)), auth)
            .await
    }

    pub async fn update(
        &self,
        name: &str,
        version: u64,
        spec: &ServiceSpec,
        auth: Option<&RegistryAuth>,
    ) -> Result<Vec<String>> {
        self.update_call(name, &[("version", version.to_string())], spec, auth)
            .await
    }

    pub async fn restart(&self, name: &str, auth: Option<&RegistryAuth>) -> Result<Vec<String>> {
        let service = self.inspect(name).await?.ok_or_else(|| not_found(name))?;
        let mut spec = service.spec.clone();
        spec.task_template.force_update = spec.task_template.force_update.wrapping_add(1);
        self.update_call(
            name,
            &[
                ("version", service.version.index.to_string()),
                ("registryAuthFrom", "spec".to_owned()),
            ],
            &spec,
            auth,
        )
        .await
    }

    pub async fn rollback(&self, name: &str, version: u64) -> Result<Vec<String>> {
        let service = self.inspect(name).await?.ok_or_else(|| not_found(name))?;
        self.update_call(
            name,
            &[
                ("version", version.to_string()),
                ("registryAuthFrom", "spec".to_owned()),
                ("rollback", "previous".to_owned()),
            ],
            &service.spec,
            None,
        )
        .await
    }

    pub async fn delete(&self, name: &str) -> Result<bool> {
        self.0.delete_idempotent(&format!("/services/{name}")).await
    }

    async fn update_call(
        &self,
        name: &str,
        query: &[(&str, String)],
        spec: &ServiceSpec,
        auth: Option<&RegistryAuth>,
    ) -> Result<Vec<String>> {
        let resp: Option<UpdateResponse> = self
            .0
            .post_maybe(
                &format!("/services/{name}/update"),
                query,
                Some(to_json_body(spec)),
                auth,
            )
            .await?;
        Ok(resp.and_then(|r| r.warnings).unwrap_or_default())
    }
}

fn not_found(name: &str) -> Error {
    Error::new(ErrorKind::Docker {
        code: StatusCode::NOT_FOUND,
        msg: format!("service not found: {name}"),
    })
}
