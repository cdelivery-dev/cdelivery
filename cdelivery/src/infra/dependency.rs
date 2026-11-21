pub(crate) mod sizes;

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::{concept, error};

const PREFIX: &str = "cdelivery-dependency-";
const LABEL_PROVIDER: &str = "dev.cdelivery.dependency_provider";
const LABEL_DESCRIPTION: &str = "dev.cdelivery.description";
const LABEL_APP: &str = "dev.cdelivery.dependency_app";
const LABEL_CLOUD_PROVIDER: &str = "dev.cdelivery.dependency_cloud_provider";
const LABEL_ARCHITECTURE: &str = "dev.cdelivery.dependency_architecture";

pub(crate) fn resource_name(name: &concept::name::Name) -> String {
    format!("{PREFIX}{name}")
}

fn decode(
    secret: &docker::types::Secret,
) -> error::Result<concept::versioned::Versioned<concept::dependency::Dependency>> {
    let labels = &secret.spec.labels;
    let provider = labels
        .get(LABEL_PROVIDER)
        .and_then(|value| value.parse().ok())
        .ok_or(error::ErrorKind::SecretNotFound)?;
    let name = secret
        .spec
        .name
        .strip_prefix(PREFIX)
        .ok_or(error::ErrorKind::SecretNotFound)?
        .parse()?;
    let group = labels
        .get(super::group::LABEL_GROUP)
        .map(|group| group.parse())
        .transpose()?;
    let local_app = labels.get(LABEL_APP).map(|name| name.parse()).transpose()?;
    let cloud_provider = labels.get(LABEL_CLOUD_PROVIDER).cloned();
    let architecture = labels.get(LABEL_ARCHITECTURE).cloned();
    if (provider == concept::dependency::Provider::Local) != group.is_some()
        || group.is_some() != local_app.is_some()
        || (provider == concept::dependency::Provider::PlanetScale) != cloud_provider.is_some()
        || cloud_provider.is_some() != architecture.is_some()
        || cloud_provider
            .iter()
            .chain(&architecture)
            .any(|value| value.trim().is_empty())
    {
        return Err(error::ErrorKind::SecretBindingInvalid.into());
    }
    Ok(concept::versioned::Versioned::new(
        secret.version.index,
        concept::dependency::Dependency::new(
            concept::secret::Reference::new(name, secret.id.clone()),
            group,
            provider,
            local_app,
            cloud_provider,
            architecture,
            labels.get(LABEL_DESCRIPTION).cloned(),
            secret.created_at,
        ),
    ))
}

pub(crate) async fn create(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
    group: Option<&concept::name::Name>,
    description: Option<String>,
    mut fields: BTreeMap<String, String>,
) -> error::Result<()> {
    let provider = fields
        .get("provider")
        .ok_or(error::ErrorKind::SecretBindingInvalid)?;
    let mut labels = HashMap::from([(LABEL_PROVIDER.to_owned(), provider.clone())]);
    if provider == "PlanetScale" {
        for (field, label) in [
            ("cloud_provider", LABEL_CLOUD_PROVIDER),
            ("architecture", LABEL_ARCHITECTURE),
        ] {
            let value = fields
                .remove(field)
                .filter(|value| !value.trim().is_empty())
                .ok_or(error::ErrorKind::SecretBindingInvalid)?;
            labels.insert(label.to_owned(), value);
        }
    }
    if let Some(group) = group {
        labels.insert(super::group::LABEL_GROUP.to_owned(), group.to_string());
    }
    if let Some(app) = fields.get("app") {
        labels.insert(LABEL_APP.to_owned(), app.clone());
    }
    if let Some(description) = description.filter(|value| !value.is_empty()) {
        labels.insert(LABEL_DESCRIPTION.to_owned(), description);
    }
    let data = cdelivery_wire::postgres::encode_fields(&fields)?;
    match docker
        .secret_create(&resource_name(name), labels, &data)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) if error.is_conflict() => Err(error::ErrorKind::SecretExists.into()),
        Err(error) => Err(error),
    }
}

pub(crate) async fn load(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
) -> error::Result<Option<concept::versioned::Versioned<concept::dependency::Dependency>>> {
    docker
        .secret_inspect(&resource_name(name))
        .await?
        .as_ref()
        .map(decode)
        .transpose()
}

pub(crate) async fn list(
    docker: &super::docker::Docker,
    group: Option<&concept::name::Name>,
) -> error::Result<Vec<concept::dependency::Dependency>> {
    let mut filter = docker::types::Filter::new().name(PREFIX);
    if let Some(group) = group {
        filter = filter.label(format!("{}={group}", super::group::LABEL_GROUP));
    }
    let mut dependencies = Vec::new();
    for secret in docker.secret_list_filtered(&filter).await? {
        if !secret.spec.name.starts_with(PREFIX) {
            continue;
        }
        match decode(&secret) {
            Ok(dependency) => {
                dependencies.push(dependency.into_inner());
            }
            Err(error) => {
                tracing::warn!(error = ?error, "skipping unreadable dependency metadata");
            }
        }
    }
    dependencies.sort_by(|a, b| {
        a.reference()
            .name()
            .as_str()
            .cmp(b.reference().name().as_str())
    });
    Ok(dependencies)
}

pub(crate) async fn delete(
    docker: &super::docker::Docker,
    name: &concept::name::Name,
) -> error::Result<()> {
    match docker.secret_delete(&resource_name(name)).await {
        Err(error) if matches!(error.kind(), error::ErrorKind::Docker(source) if source.is_in_use()) => {
            Err(error::ErrorKind::SecretInUseRuntime(Box::new(error)).into())
        }
        result => result,
    }
}

fn endpoint(base: &str, segments: &[&str]) -> error::Result<url::Url> {
    let mut url = url::Url::parse(base).map_err(cdelivery_wire::postgres::Error::from)?;
    url.path_segments_mut()
        .map_err(|()| error::ErrorKind::InvalidBody("invalid provider API endpoint".to_owned()))?
        .extend(segments);
    Ok(url)
}

async fn planet_list<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    token: &str,
    segments: &[&str],
) -> error::Result<Vec<T>> {
    #[derive(Deserialize)]
    struct Page<T> {
        data: Vec<T>,
        next_page: Option<u64>,
    }
    let url = endpoint("https://api.planetscale.com/v1", segments)?;
    let mut page = 1;
    let mut values = Vec::new();
    loop {
        let response: Page<T> = client
            .get(url.clone())
            .query(&[("page", page), ("per_page", 100)])
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        values.extend(response.data);
        let Some(next) = response.next_page else {
            return Ok(values);
        };
        if next <= page {
            return Err(error::ErrorKind::InvalidBody(
                "PlanetScale pagination did not advance".to_owned(),
            )
            .into());
        }
        page = next;
    }
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn discover(
    provider: concept::dependency::Provider,
    dsn: &cdelivery_wire::postgres::Dsn,
    token: &str,
    project_id: Option<&str>,
) -> error::Result<BTreeMap<String, String>> {
    let client = reqwest::Client::builder()
        .timeout(crate::config::OPERATION_STANDARD_TIMEOUT.unsigned_abs())
        .build()?;
    match provider {
        concept::dependency::Provider::Neon => {
            #[derive(Deserialize)]
            struct Endpoint {
                id: String,
                branch_id: String,
                host: String,
            }
            #[derive(Deserialize)]
            struct Endpoints {
                endpoints: Vec<Endpoint>,
            }
            let project = project_id
                .filter(|project| !project.is_empty())
                .ok_or_else(|| error::ErrorKind::FieldRequired("project_id".to_owned()))?;
            let (name, domain) = dsn.host().split_once('.').ok_or_else(|| {
                error::ErrorKind::InvalidBody("Neon DSN requires a compute hostname".to_owned())
            })?;
            let pooled = name.ends_with("-pooler");
            let host = format!("{}.{domain}", name.strip_suffix("-pooler").unwrap_or(name));
            let response: Endpoints = client
                .get(endpoint(
                    "https://console.neon.tech/api/v2",
                    &["projects", project, "endpoints"],
                )?)
                .bearer_auth(token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let source = response
                .endpoints
                .into_iter()
                .find(|source| source.host.eq_ignore_ascii_case(&host))
                .ok_or_else(|| {
                    error::ErrorKind::InvalidBody(
                        "Neon DSN does not match a compute in this project".to_owned(),
                    )
                })?;
            Ok(BTreeMap::from([
                ("api_key".to_owned(), token.to_owned()),
                ("project_id".to_owned(), project.to_owned()),
                ("branch_id".to_owned(), source.branch_id),
                ("endpoint_id".to_owned(), source.id),
                ("pooled".to_owned(), pooled.to_string()),
            ]))
        }
        concept::dependency::Provider::PlanetScale => {
            #[derive(Deserialize)]
            struct Organization {
                name: String,
            }
            #[derive(Deserialize)]
            struct Database {
                name: String,
                kind: String,
            }
            #[derive(Deserialize)]
            struct Branch {
                id: String,
                name: String,
                kind: String,
                cluster_name: String,
            }
            #[derive(Deserialize)]
            struct Role {
                username: String,
                base_username: String,
            }
            if project_id.is_some() {
                return Err(error::ErrorKind::FieldUnsupported("project_id".to_owned()).into());
            }
            let (_, branch_id) = dsn
                .username()
                .rsplit_once('.')
                .filter(|(username, branch)| !username.is_empty() && !branch.is_empty())
                .ok_or_else(|| {
                    error::ErrorKind::InvalidBody(
                        "PlanetScale username must include its branch routing suffix".to_owned(),
                    )
                })?;
            for organization in
                planet_list::<Organization>(&client, token, &["organizations"]).await?
            {
                for database in planet_list::<Database>(
                    &client,
                    token,
                    &["organizations", &organization.name, "databases"],
                )
                .await?
                {
                    if database.kind != "postgresql" {
                        continue;
                    }
                    let base = [
                        "organizations",
                        &organization.name,
                        "databases",
                        &database.name,
                        "branches",
                    ];
                    let branches = planet_list::<Branch>(&client, token, &base).await?;
                    let Some(branch) = branches.into_iter().find(|branch| branch.id == branch_id)
                    else {
                        continue;
                    };
                    if branch.kind != "postgresql" {
                        return Err(error::ErrorKind::InvalidBody(
                            "PlanetScale dependency requires a PostgreSQL branch".to_owned(),
                        )
                        .into());
                    }
                    let roles = planet_list::<Role>(
                        &client,
                        token,
                        &[
                            "organizations",
                            &organization.name,
                            "databases",
                            &database.name,
                            "branches",
                            &branch.name,
                            "roles",
                        ],
                    )
                    .await?;
                    let role = roles
                        .into_iter()
                        .find(|role| {
                            role.username == dsn.username() && !role.base_username.is_empty()
                        })
                        .ok_or_else(|| {
                            error::ErrorKind::InvalidBody(
                                "PlanetScale DSN does not match a managed role".to_owned(),
                            )
                        })?;
                    let source = sizes::list()
                        .ok_or(error::ErrorKind::PlanetScaleSizesUnavailable)?
                        .into_iter()
                        .find(|sku| sku.name == branch.cluster_name)
                        .ok_or_else(|| {
                            error::ErrorKind::InvalidBody(format!(
                                "source SKU {} is missing from the PlanetScale size catalog",
                                branch.cluster_name
                            ))
                        })?;
                    return Ok(BTreeMap::from([
                        ("api_token".to_owned(), token.to_owned()),
                        ("organization".to_owned(), organization.name),
                        ("database_name".to_owned(), database.name),
                        ("branch_name".to_owned(), branch.name),
                        ("base_username".to_owned(), role.base_username),
                        ("cloud_provider".to_owned(), source.provider),
                        ("architecture".to_owned(), source.architecture),
                    ]));
                }
            }
            Err(error::ErrorKind::InvalidBody(
                "PlanetScale branch was not found using these API credentials".to_owned(),
            )
            .into())
        }
        concept::dependency::Provider::Local => {
            Err(error::ErrorKind::FieldUnsupported("api_token".to_owned()).into())
        }
    }
}
