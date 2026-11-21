use cipher::{Domain, MasterKey};
use docker::types::{
    ContainerSpec, NetworkAttachment, ServiceFileTarget, ServiceMode, ServiceSecret, ServiceSpec,
    TaskTemplate,
};
use jiff::Timestamp;

use crate::error::{ErrorKind, Result};
use crate::{concept, config};

pub(crate) fn name(change: &concept::change::Change) -> String {
    format!("cdelivery-postgres-{}", change.id().simple())
}

#[expect(clippy::too_many_lines)]
pub(crate) async fn create(
    docker: &super::super::docker::Docker,
    change: &concept::change::Change,
    dependencies: &[concept::dependency::Dependency],
    image: Option<&concept::image::Reference>,
) -> Result<()> {
    if docker.service_exists(&name(change)).await? {
        return Ok(());
    }
    let image = image.ok_or(ErrorKind::ReleaseUnavailable)?;
    let main = super::super::instance::require_service(docker).await?;
    let key_name = super::super::change::resource_name(change.id());
    let secret = docker
        .secret_inspect(&key_name)
        .await?
        .ok_or(ErrorKind::ChangeNotFound)?;
    let mut secrets = vec![ServiceSecret {
        secret_id: secret.id,
        secret_name: key_name,
        file: ServiceFileTarget {
            name: cdelivery_wire::postgres::KEY_PATH.to_owned(),
            ..ServiceFileTarget::default()
        },
    }];
    for dependency in dependencies {
        secrets.push(ServiceSecret {
            secret_id: dependency.reference().id().clone(),
            secret_name: super::super::dependency::resource_name(dependency.reference().name()),
            file: ServiceFileTarget {
                name: format!(
                    "{}/{}",
                    cdelivery_wire::postgres::SECRET_DIRECTORY,
                    dependency.reference().name()
                ),
                ..ServiceFileTarget::default()
            },
        });
    }
    let mut labels = std::collections::HashMap::from([(
        super::preview::LABEL_CHANGE.to_owned(),
        change.id().to_string(),
    )]);
    let networks: std::collections::BTreeSet<_> = std::iter::once(config::NETWORK_NAME.to_owned())
        .chain(
            dependencies
                .iter()
                .filter_map(|dependency| dependency.group().as_ref())
                .map(super::super::group::resource_name),
        )
        .collect();
    super::super::label::managed(&mut labels);
    let mut spec = ServiceSpec {
        name: name(change),
        labels: labels.clone(),
        task_template: TaskTemplate {
            container_spec: ContainerSpec {
                image: image.to_string(),
                secrets,
                labels,
                ..ContainerSpec::default()
            },
            networks: Some(
                networks
                    .into_iter()
                    .map(|target| NetworkAttachment {
                        target,
                        aliases: None,
                        driver_opts: None,
                    })
                    .collect(),
            ),
            log_driver: main.spec.task_template.log_driver,
            ..TaskTemplate::default()
        },
        mode: ServiceMode::Replicated { replicas: 1 },
        endpoint_spec: None,
        update_config: None,
        rollback_config: None,
        extra: serde_json::Map::new(),
    };
    super::runner::one_shot(&mut spec);
    docker.service_create(&spec, None).await?;
    let client = reqwest::Client::builder()
        .timeout(config::CHANGE_POLL_INTERVAL.unsigned_abs())
        .build()?;
    let deadline = config::expiry(Timestamp::now(), config::GUARDIAN_DEPLOY_TIMEOUT);
    let url = format!(
        "http://{}:{}/health",
        name(change),
        cdelivery_wire::postgres::PORT
    );
    loop {
        if client
            .get(&url)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        if Timestamp::now() >= deadline {
            return Err(ErrorKind::ChangeExecution(
                "PostgreSQL executor did not become ready".to_owned(),
            )
            .into());
        }
        tokio::time::sleep(config::CHANGE_POLL_INTERVAL.unsigned_abs()).await;
    }
}

pub(crate) async fn request(
    key: &MasterKey,
    change: &concept::change::Change,
    command: cdelivery_wire::postgres::Command,
) -> Result<cdelivery_wire::postgres::Response> {
    let change_id = change.id().to_string();
    let cipher = cipher::Cipher::from_key(&key.scoped_key(Domain::Postgres, change_id.as_bytes()));
    let body = cipher.seal(&command)?;
    let response = reqwest::Client::builder()
        .connect_timeout(config::OPERATION_STANDARD_TIMEOUT.unsigned_abs())
        .build()?
        .post(format!(
            "http://{}:{}/",
            name(change),
            cdelivery_wire::postgres::PORT
        ))
        .body(body)
        .send()
        .await?
        .error_for_status()?;
    match cipher.open(&response.text().await?)? {
        cdelivery_wire::postgres::Response::Failed { message } => {
            Err(ErrorKind::ChangeExecution(message).into())
        }
        response => Ok(response),
    }
}

pub(crate) async fn delete(
    docker: &super::super::docker::Docker,
    change: &concept::change::Change,
) -> Result<()> {
    super::preview::stop_and_delete(docker, &name(change)).await
}

pub(crate) async fn create_connection(
    docker: &super::super::docker::Docker,
    change: &concept::change::Change,
    dependency: &concept::secret::Reference,
    dsn: &str,
) -> Result<String> {
    let name = change.resource_name("connection", dependency.name().as_str());
    let fields = cdelivery_wire::postgres::Dsn::parse(dsn)?.fields();
    let data = cdelivery_wire::postgres::encode_fields(&fields)?;
    docker
        .secret_create(
            &name,
            std::collections::HashMap::from([(
                super::preview::LABEL_CHANGE.to_owned(),
                change.id().to_string(),
            )]),
            &data,
        )
        .await
}

pub(crate) async fn delete_connection(
    docker: &super::super::docker::Docker,
    change: &concept::change::Change,
    dependency: &concept::secret::Reference,
) -> Result<()> {
    docker
        .secret_delete(&change.resource_name("connection", dependency.name().as_str()))
        .await
}
