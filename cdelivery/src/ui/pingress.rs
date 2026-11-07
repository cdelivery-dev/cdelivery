use std::sync::Arc;

use askama::Template;
use axum::extract::multipart::{Multipart, MultipartError, MultipartRejection};
use axum::extract::{Path, State};
use axum::response::Response;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, StatusCode};
use jiff::Timestamp;

use crate::error::ErrorKind;
use crate::locale::tt;
use crate::{concept, operation};

const CERT_CHAIN_LIMIT: usize = 65_536;
const PRIVATE_KEY_LIMIT: usize = 16_384;
const MULTIPART_OVERHEAD_LIMIT: usize = 16_384;
pub(super) const BODY_LIMIT: usize =
    CERT_CHAIN_LIMIT + PRIVATE_KEY_LIMIT + MULTIPART_OVERHEAD_LIMIT;

#[derive(Template)]
#[template(path = "pingress/spec.html")]
struct SpecPage {
    dashboard: super::dashboard::Dashboard,
    pingress: Option<operation::pingress::typ::Pingress>,
    admin: Option<AdminData>,
}

struct AdminData {
    nodes: operation::node::typ::Nodes,
    current_version: Option<String>,
    target_version: Option<String>,
    latest_unavailable: bool,
}

#[derive(Template)]
#[template(path = "pingress/certificates.html")]
struct CertificatesPage {
    dashboard: super::dashboard::Dashboard,
    pingress: Option<operation::pingress::typ::Pingress>,
    certs: operation::pingress::typ::Certs,
    can_edit: bool,
}

#[derive(Template)]
#[template(path = "pingress/cert.html")]
struct CertificatePage {
    dashboard: super::dashboard::Dashboard,
    entry: operation::pingress::typ::CertEntry,
    pingress_enabled: bool,
    can_edit: bool,
}

pub(super) fn routes() -> axum::Router<Arc<crate::boot::App>> {
    axum::Router::new()
        .route("/pingress", axum::routing::get(spec))
        .route("/pingress/enable", axum::routing::post(enable))
        .route("/pingress/update", axum::routing::post(update))
        .route("/pingress/disable", axum::routing::post(disable))
        .route("/pingress/certificates", axum::routing::get(certificates))
        .route(
            "/pingress/certificates/{domain}",
            axum::routing::get(certificate).post(upload_certificate),
        )
        .route(
            "/pingress/certificates/{domain}/remove",
            axum::routing::post(remove_certificate),
        )
        .route("/pingress/acme/enable", axum::routing::post(enable_acme))
        .route("/pingress/acme/disable", axum::routing::post(disable_acme))
}

async fn spec(
    authorized: super::auth::Authorized<operation::pingress::Get>,
    State(app): State<Arc<crate::boot::App>>,
) -> super::response::Result<Response> {
    let can_manage = authorized
        .access()
        .allows(concept::permission::Requirement::Admin);
    let (dashboard, pingress, nodes, [latest_version]) = tokio::join!(
        super::dashboard::Dashboard::load(
            &app,
            authorized.access(),
            super::dashboard::Location::Pingress,
        ),
        load_pingress(&app, authorized.access()),
        super::load_nodes(&app, authorized.access(), can_manage),
        super::latest_versions(
            &app,
            authorized.access(),
            can_manage,
            [concept::release::Component::Pingress],
        ),
    );
    let pingress = pingress?;
    let dashboard = dashboard?;
    let admin = if can_manage {
        let current_version = pingress.as_ref().map(|current| current.version.clone());
        let latest_unavailable = latest_version.is_none();
        let target_version = super::target_version(current_version.as_deref(), latest_version)?;
        Some(AdminData {
            nodes: nodes?,
            current_version,
            target_version,
            latest_unavailable,
        })
    } else {
        None
    };
    super::response::render(
        &SpecPage {
            dashboard,
            pingress,
            admin,
        },
        StatusCode::OK,
    )
}

async fn enable(
    authorized: super::auth::Authorized<operation::pingress::Enable>,
    State(app): State<Arc<crate::boot::App>>,
    super::command::Payload(payload): super::command::Payload<
        operation::pingress::typ::CreatePingressRequest,
    >,
) -> super::response::Result<Response> {
    authorized.execute(&app, payload).await?;
    Ok(super::router::redirect("/pingress"))
}

async fn update(
    authorized: super::auth::Authorized<operation::pingress::Update>,
    State(app): State<Arc<crate::boot::App>>,
    super::command::Payload(payload): super::command::Payload<
        operation::pingress::typ::UpdatePingressRequest,
    >,
) -> super::response::Result<Response> {
    authorized.execute(&app, payload).await?;
    Ok(super::router::redirect("/pingress"))
}

async fn disable(
    authorized: super::auth::Authorized<operation::pingress::Disable>,
    State(app): State<Arc<crate::boot::App>>,
    super::command::Payload(_): super::command::Payload<operation::input::Empty>,
) -> super::response::Result<Response> {
    authorized.execute(&app, operation::input::Empty {}).await?;
    Ok(super::router::redirect("/pingress"))
}

async fn certificates(
    authorized: super::auth::Authorized<operation::pingress::ListCerts>,
    State(app): State<Arc<crate::boot::App>>,
) -> super::response::Result<Response> {
    let (dashboard, pingress, certs) = tokio::try_join!(
        super::dashboard::Dashboard::load(
            &app,
            authorized.access(),
            super::dashboard::Location::Pingress,
        ),
        load_pingress(&app, authorized.access()),
        authorized.execute(&app, operation::input::Empty {}),
    )?;
    super::response::render(
        &CertificatesPage {
            dashboard,
            pingress,
            certs,
            can_edit: authorized
                .access()
                .allows(concept::permission::Requirement::Admin),
        },
        StatusCode::OK,
    )
}

async fn certificate(
    authorized: super::auth::Authorized<operation::pingress::ListCerts>,
    State(app): State<Arc<crate::boot::App>>,
    Path(domain): Path<String>,
) -> super::response::Result<Response> {
    let domain = domain.parse::<concept::domain::DomainName>()?.to_string();
    let (dashboard, pingress, certs) = tokio::try_join!(
        super::dashboard::Dashboard::load(
            &app,
            authorized.access(),
            super::dashboard::Location::Pingress,
        ),
        load_pingress(&app, authorized.access()),
        authorized.execute(&app, operation::input::Empty {}),
    )?;
    let entry = certs
        .certs
        .into_iter()
        .find(|entry| entry.domain == domain)
        .ok_or(ErrorKind::CertNotFound)?;
    super::response::render(
        &CertificatePage {
            dashboard,
            entry,
            pingress_enabled: pingress.is_some(),
            can_edit: authorized
                .access()
                .allows(concept::permission::Requirement::Admin),
        },
        StatusCode::OK,
    )
}

async fn upload_certificate(
    authorized: super::auth::Authorized<operation::pingress::UploadCert>,
    State(app): State<Arc<crate::boot::App>>,
    Path(domain): Path<String>,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> super::response::Result<Response> {
    let domain = domain.parse::<concept::domain::DomainName>()?.to_string();
    let is_multipart = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("multipart/form-data"));
    if !is_multipart {
        return Err(ErrorKind::UnsupportedMediaType("multipart/form-data").into());
    }
    let mut multipart = multipart.map_err(|_| invalid_multipart())?;
    let mut cert_chain_pem = None;
    let mut private_key_pem = None;
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        let name = field.name().ok_or_else(invalid_multipart)?;
        if field.file_name().is_none() {
            return Err(invalid_multipart());
        }
        let (target, limit) = match name {
            "cert_chain_pem" if cert_chain_pem.is_none() => (&mut cert_chain_pem, CERT_CHAIN_LIMIT),
            "private_key_pem" if private_key_pem.is_none() => {
                (&mut private_key_pem, PRIVATE_KEY_LIMIT)
            }
            _ => return Err(invalid_multipart()),
        };
        let bytes = field.bytes().await.map_err(multipart_error)?;
        if bytes.len() > limit {
            return Err(ErrorKind::PayloadTooLarge.into());
        }
        *target = Some(String::from_utf8(bytes.to_vec()).map_err(|_| invalid_multipart())?);
    }
    let cert_chain_pem = cert_chain_pem.ok_or_else(invalid_multipart)?;
    let private_key_pem = private_key_pem.ok_or_else(invalid_multipart)?;
    authorized
        .execute(
            &app,
            operation::pingress::UploadCertInput {
                domain: domain.clone(),
                cert: operation::pingress::typ::UploadCertRequest {
                    cert_chain_pem,
                    private_key_pem: cipher::Secret::new(private_key_pem),
                },
            },
        )
        .await?;
    Ok(super::router::redirect(&format!(
        "/pingress/certificates/{domain}"
    )))
}

async fn remove_certificate(
    authorized: super::auth::Authorized<operation::pingress::RemoveCert>,
    State(app): State<Arc<crate::boot::App>>,
    Path(domain): Path<String>,
    super::command::Payload(_): super::command::Payload<operation::input::Empty>,
) -> super::response::Result<Response> {
    authorized
        .execute(&app, operation::input::Domain { domain })
        .await?;
    Ok(super::router::redirect("/pingress/certificates"))
}

async fn enable_acme(
    authorized: super::auth::Authorized<operation::pingress::EnableAccount>,
    State(app): State<Arc<crate::boot::App>>,
    super::command::Payload(payload): super::command::Payload<
        operation::pingress::typ::EnableAcmeRequest,
    >,
) -> super::response::Result<Response> {
    authorized.execute(&app, payload).await?;
    Ok(super::router::redirect("/pingress/certificates"))
}

async fn disable_acme(
    authorized: super::auth::Authorized<operation::pingress::DisableAccount>,
    State(app): State<Arc<crate::boot::App>>,
    super::command::Payload(_): super::command::Payload<operation::input::Empty>,
) -> super::response::Result<Response> {
    authorized.execute(&app, operation::input::Empty {}).await?;
    Ok(super::router::redirect("/pingress/certificates"))
}

pub(in crate::ui) async fn load_pingress(
    app: &crate::boot::App,
    access: &concept::permission::Access,
) -> crate::error::Result<Option<operation::pingress::typ::Pingress>> {
    match operation::execute::<operation::pingress::Get>(
        app,
        access,
        operation::input::Empty {},
        Timestamp::now(),
    )
    .await
    {
        Ok(pingress) => Ok(Some(pingress)),
        Err(error) if matches!(error.kind(), ErrorKind::PingressNotEnabled) => Ok(None),
        Err(error) => Err(error),
    }
}

fn invalid_multipart() -> super::response::Error {
    ErrorKind::InvalidBody(tt!("ui.pingress.certs.invalidUpload").into_owned()).into()
}

fn multipart_error(error: MultipartError) -> super::response::Error {
    match error.status() {
        StatusCode::PAYLOAD_TOO_LARGE => ErrorKind::PayloadTooLarge.into(),
        StatusCode::BAD_REQUEST => invalid_multipart(),
        _ => ErrorKind::Io(std::io::Error::other(error)).into(),
    }
}
