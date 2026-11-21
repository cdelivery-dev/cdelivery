use cipher::MasterKey;

use crate::{concept, error, infra};

pub(crate) struct CreateParams {
    pub(crate) name: concept::name::Name,
    pub(crate) description: Option<String>,
    pub(crate) dsn: String,
    pub(crate) provider: concept::dependency::Provider,
    pub(crate) api_token: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) app: Option<concept::name::Name>,
}

pub(crate) async fn create(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    params: CreateParams,
) -> error::Result<()> {
    let dsn = cdelivery_wire::postgres::Dsn::parse(&params.dsn)?;
    let mut fields = dsn.fields();
    fields.insert("provider".to_owned(), params.provider.as_ref().to_owned());
    match params.provider {
        concept::dependency::Provider::Local => {
            let app = params
                .app
                .as_ref()
                .ok_or_else(|| error::ErrorKind::FieldRequired("app".to_owned()))?;
            if params.api_token.is_some() || params.project_id.is_some() {
                return Err(
                    error::ErrorKind::FieldUnsupported("api_token/project_id".to_owned()).into(),
                );
            }
            fields.insert("app".to_owned(), app.to_string());
        }
        concept::dependency::Provider::Neon | concept::dependency::Provider::PlanetScale => {
            if params.app.is_some() {
                return Err(error::ErrorKind::FieldUnsupported("app".to_owned()).into());
            }
            let token = params
                .api_token
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| error::ErrorKind::SecretRequired("api_token".to_owned()))?;
            fields.extend(
                infra::dependency::discover(
                    params.provider,
                    &dsn,
                    token,
                    params.project_id.as_deref(),
                )
                .await?,
            );
        }
    }
    let _admission = super::change::admission().await;
    let group = if let Some(name) = &params.app {
        let app = super::app::get(docker, key, name).await?;
        super::change::ensure_app_available(docker, key, &app).await?;
        if app.mounts().is_none() {
            return Err(error::ErrorKind::GuardianStatefulUnsupported.into());
        }
        super::volume::require_guardian(docker, &app).await?;
        Some(app.group().clone())
    } else {
        None
    };
    infra::dependency::create(
        docker,
        &params.name,
        group.as_ref(),
        params.description,
        fields,
    )
    .await
}

pub(crate) async fn get(
    docker: &infra::docker::Docker,
    name: &concept::name::Name,
) -> error::Result<concept::dependency::Dependency> {
    super::found(
        infra::dependency::load(docker, name),
        error::ErrorKind::SecretNotFound,
    )
    .await
}

pub(crate) async fn list(
    docker: &infra::docker::Docker,
    group: Option<&concept::name::Name>,
) -> error::Result<Vec<concept::dependency::Dependency>> {
    Ok(infra::dependency::list(docker, None)
        .await?
        .into_iter()
        .filter(|dependency| {
            group.is_none() || dependency.group().is_none() || dependency.group().as_ref() == group
        })
        .collect())
}

pub(crate) fn planetscale_sizes(
    dependency: Option<&concept::dependency::Dependency>,
) -> error::Result<Vec<cdelivery_wire::postgres::PlanetScaleSku>> {
    if dependency
        .is_some_and(|source| source.provider() != concept::dependency::Provider::PlanetScale)
    {
        return Err(error::ErrorKind::FieldUnsupported("dependency".to_owned()).into());
    }
    let mut sizes =
        infra::dependency::sizes::list().ok_or(error::ErrorKind::PlanetScaleSizesUnavailable)?;
    sizes.retain(|sku| {
        !sku.metal
            && dependency.is_none_or(|source| {
                source.cloud_provider().as_ref() == Some(&sku.provider)
                    && source.architecture().as_ref() == Some(&sku.architecture)
            })
    });
    Ok(sizes)
}

pub(crate) async fn validate_reference(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    consumer: &concept::app::App,
    reference: &concept::dependency::Reference,
) -> error::Result<()> {
    match reference {
        concept::dependency::Reference::App(name) => {
            if name == consumer.name() {
                return Err(error::ErrorKind::SecretBindingInvalid.into());
            }
            let app = super::app::get(docker, key, name).await?;
            if app.group() != consumer.group() {
                return Err(error::ErrorKind::SecretBindingInvalid.into());
            }
            super::volume::require_guardian(docker, &app).await?;
        }
        concept::dependency::Reference::PostgreSQL(reference) => {
            let dependency = get(docker, reference.name()).await?;
            if dependency.reference() != reference
                || dependency
                    .group()
                    .as_ref()
                    .is_some_and(|group| group != consumer.group())
            {
                return Err(error::ErrorKind::SecretBindingInvalid.into());
            }
        }
    }
    Ok(())
}

pub(crate) async fn delete(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    name: &concept::name::Name,
) -> error::Result<()> {
    let _admission = super::change::admission().await;
    let Some(dependency) = infra::dependency::load(docker, name).await? else {
        return Ok(());
    };
    super::change::ensure_dependency_available(docker, key, dependency.get()).await?;
    if let Some(app) = infra::app::secret_in_use(docker, dependency.get().reference().id()).await? {
        return Err(error::ErrorKind::SecretInUse(app).into());
    }
    infra::change::delete_completed(
        docker,
        key,
        &concept::dependency::Reference::PostgreSQL(dependency.get().reference().clone()),
    )
    .await?;
    infra::dependency::delete(docker, name).await
}
