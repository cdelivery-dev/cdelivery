use cipher::MasterKey;
use insight_wire::event::Kind;

use crate::error::Result;
use crate::{concept, config, infra};

pub(crate) async fn configuration(
    docker: &infra::docker::Docker,
) -> Result<(concept::image::Reference, Option<concept::domain::Rule>)> {
    infra::instance::configuration(docker).await
}

#[expect(clippy::option_option)]
pub(crate) async fn update(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    releases: &infra::release::Catalog,
    version: Option<&str>,
    domain: Option<Option<concept::domain::Rule>>,
) -> Result<()> {
    let image = version
        .map(|version| {
            super::release::resolve(
                releases,
                concept::release::Component::Cdelivery,
                version,
                "version",
            )
        })
        .transpose()?;
    if let Some(Some(rule)) = &domain {
        super::pingress::check_routes(
            docker,
            key,
            std::slice::from_ref(rule),
            Some(config::SERVICE_NAME),
        )
        .await?;
    }
    let (updated_image, domain_changed) =
        infra::instance::configure(docker, image.as_ref(), domain.as_ref().map(Option::as_ref))
            .await?;
    if updated_image.is_none() && !domain_changed {
        return Ok(());
    }
    let mut details = Vec::with_capacity(2);
    if let Some(image) = updated_image {
        details.push(format!("image {image}"));
    }
    if domain_changed {
        match domain {
            Some(Some(rule)) => details.push(format!(
                "domain {}://{}",
                if rule.https() { "https" } else { "http" },
                rule.route().domain
            )),
            Some(None) => details.push("domain cleared".to_owned()),
            None => {}
        }
    }
    infra::event::emit(
        Kind::PlatformUpdated,
        config::SERVICE_NAME,
        &details.join(", "),
    );
    Ok(())
}
