use std::collections::BTreeMap;

use axum::Json;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use http::header::{CACHE_CONTROL, CONTENT_TYPE};
use macros::located_error;
use rust_i18n::t;
use serde::Serialize;
use strum::IntoStaticStr;

use crate::config;
use crate::locale::Locale;

#[derive(Debug, IntoStaticStr)]
pub(crate) enum ErrorKind {
    NoService,
    AlreadyInitialized,
    NotInitialized,
    Validation(garde::Report),
    InvalidBody(String),
    InvalidName(String),
    TagNotSemver(String),
    NotBase64,
    GroupMissing(String),
    AuthRequired,
    TokenInvalid,
    CredentialsInvalid,
    ApiKeyInvalid,
    Forbidden,
    OperationTimeout {
        operation: &'static str,
        source: tokio::time::error::Elapsed,
    },
    InvalidOperationOutput,
    UnknownTool(String),
    AppNotFound,
    GroupNotFound,
    ConfigNotFound,
    RouteNotFound,
    MethodNotAllowed,
    Conflict,
    EmailTaken,
    CurrentPasswordInvalid,
    GroupExists,
    GroupBusy,
    AppExists,
    AlreadyRolledBack,
    NothingToRollBack,
    StatefulReplicasPinned,
    MountsRequireStateful,
    MountsPlacementConflict,
    XConfigMountsConflict,
    XHealthNotEnabled,
    XHealthExists,
    XHealthRunning,
    XHealthTargetNotFound,
    XHealthTargetAmbiguous,
    XHealthIntervalInvalid,
    GuardianPending,
    GuardianNotFound,
    GuardianResolved,
    GuardianStatefulUnsupported,
    XJobNotFound,
    XJobExists,
    XJobRunning,
    XJobLimit {
        limit: usize,
    },
    PingressExists,
    PingressNotEnabled,
    PingressBusy(String),
    ReservedName(String),
    InsightNotEnabled,
    InsightLocalStorageRequiresNode,
    InsightQueryFailed(String),
    ImApi {
        platform: &'static str,
        detail: String,
        source: Option<serde_json::Error>,
    },
    AgentEmptyResponse,
    Llm(genai::Error),
    LlmTimeout(tokio::time::error::Elapsed),
    AcmeCaMismatch,
    CertNotFound,
    DomainLimit {
        limit: usize,
        current: usize,
    },
    InvalidDomain(String),
    NodeNotFound,
    NoSwarmManager,
    Cert(CertProblem),
    Acme(AcmeProblem),
    AcmeClient(instant_acme::Error),
    Reqwest(reqwest::Error),
    Spec(SpecProblem),
    Instance(InstanceProblem),
    Logging(logging::Error),
    Io(std::io::Error),
    Docker(docker::Error),
    Cipher(cipher::Error),
    Serde(serde_json::Error),
    Base64(base64::DecodeError),
    Rng(getrandom::Error),
    Argon2(argon2::password_hash::Error),
    Panic(String),
    Smtp(mail_send::Error),
    Imap(async_imap::error::Error),
    Ws(tokio_tungstenite::tungstenite::Error),
    WsProto(prost::DecodeError),
    DiscordHttp(twilight_http::Error),
    DiscordHttpBody(twilight_http::response::DeserializeBodyError),
    DiscordGateway(twilight_gateway::error::ReceiveMessageError),
    UnprocessableBody(String),
    PayloadTooLarge,
    UnsupportedMediaType(&'static str),
    InsightStorageMigrationUnsupported,
    InsightBusy(String),
    ReleaseUnavailable,
    UserNotFound,
    SelfImmutable,
    Askama(askama::Error),
    ReleaseUrlInvalid,
    TotpAlreadyEnabled,
    TotpCodeInvalid,
    Totp(totp_rs::TotpError),
    InsightLabelMatchesNoNodes(String),
    ImNotBound,
    VolumeUnavailable,
    VolumeAgentHttp {
        source: reqwest::Error,
        detail: String,
    },
    VolumeExists,
    VolumeNotFound,
    VolumeDescriptionSave(Box<Error>),
    Image(oci_spec::distribution::ParseError),
    FieldRequired(String),
    SecretRequired(String),
    VersionNotLatest {
        field: String,
        latest: String,
    },
    FieldUnsupported(String),
    ReplicaBoundsInvalid,
    VolumeInUse(Box<Error>),
}

#[derive(Debug, IntoStaticStr)]
pub(crate) enum SpecProblem {
    MissingLabel { service: String, label: String },
    NotApp(String),
    NotReplicated(String),
    BadName(String),
    BadGroup(String),
    UnsupportedMount(String),
    NotPinned(String),
    BadLogOption(&'static str),
    NoRestartPolicy(String),
    BadUpdateConfig(String),
    BadPortProtocol(String),
    BadLabel { service: String, label: String },
    BadRunner { service: String, detail: String },
    UnknownLogDriver { service: String, driver: String },
    MissingEnv { service: String, env: String },
    BadEnv { service: String, env: String },
    BadPlacement(String),
    BadAcmeCertLabel(String),
    NotGlobal(String),
}

impl SpecProblem {
    fn message(&self, locale: Locale) -> String {
        let code: &'static str = self.into();
        let key = format!("spec.{code}");
        let l = locale.tag();
        match self {
            Self::MissingLabel { service, label } | Self::BadLabel { service, label } => {
                t!(key, locale = l, service = service, label = label)
            }
            Self::UnknownLogDriver { service, driver } => {
                t!(key, locale = l, service = service, driver = driver)
            }
            Self::BadRunner { service, detail } => {
                t!(key, locale = l, service = service, detail = detail)
            }
            Self::MissingEnv { service, env } | Self::BadEnv { service, env } => {
                t!(key, locale = l, service = service, env = env)
            }
            Self::BadLogOption(option) => t!(key, locale = l, option = option),
            Self::BadAcmeCertLabel(label) => t!(key, locale = l, label = label),
            Self::NotApp(service)
            | Self::NotReplicated(service)
            | Self::BadName(service)
            | Self::BadGroup(service)
            | Self::UnsupportedMount(service)
            | Self::NotPinned(service)
            | Self::NoRestartPolicy(service)
            | Self::BadUpdateConfig(service)
            | Self::BadPortProtocol(service)
            | Self::BadPlacement(service)
            | Self::NotGlobal(service) => t!(key, locale = l, service = service),
        }
        .into_owned()
    }
}

#[derive(Debug, IntoStaticStr)]
pub(crate) enum CertProblem {
    ChainEmpty,
    PemDecode,
    Parse,
    Expired,
    DomainNotCovered(String),
    KeyMismatch,
    KeyUnsupported,
}

impl CertProblem {
    fn message(&self, locale: Locale) -> String {
        let code: &'static str = self.into();
        let key = format!("cert.{code}");
        let l = locale.tag();
        if let Self::DomainNotCovered(domain) = self {
            t!(key, locale = l, domain = domain)
        } else {
            t!(key, locale = l)
        }
        .into_owned()
    }
}

#[derive(Debug, IntoStaticStr)]
pub(crate) enum AcmeProblem {
    NoAuthorization(String),
    ChallengeNotOffered(String),
    OrderNotReady(String),
    RenewalStartOutOfRange(i64),
    Timeout(tokio::time::error::Elapsed),
    AccountUriInvalid(String),
}

impl AcmeProblem {
    fn message(&self, locale: Locale) -> String {
        let code: &'static str = self.into();
        let key = format!("acme.{code}");
        let l = locale.tag();
        match self {
            Self::NoAuthorization(domain) => t!(key, locale = l, domain = domain),
            Self::ChallengeNotOffered(kind) => t!(key, locale = l, kind = kind),
            Self::OrderNotReady(status) => t!(key, locale = l, status = status),
            Self::RenewalStartOutOfRange(second) => {
                t!(key, locale = l, second = second)
            }
            Self::Timeout(_) => t!(key, locale = l),
            Self::AccountUriInvalid(uri) => t!(key, locale = l, uri = uri),
        }
        .into_owned()
    }
}

#[derive(Debug, IntoStaticStr)]
pub(crate) enum InstanceProblem {
    NotReplicated,
    NotSingleton { replicas: u64 },
    NetworkMissing,
    NetworkNotOverlay,
    NotAttached,
}

impl InstanceProblem {
    fn message(&self, locale: Locale) -> String {
        let code: &'static str = self.into();
        let key = format!("instance.{code}");
        let l = locale.tag();
        let service = config::SERVICE_NAME;
        let network = config::NETWORK_NAME;
        match self {
            Self::NotReplicated => t!(key, locale = l, service = service),
            Self::NotSingleton { replicas } => {
                t!(key, locale = l, service = service, replicas = replicas)
            }
            Self::NetworkMissing | Self::NetworkNotOverlay => {
                t!(key, locale = l, network = network)
            }
            Self::NotAttached => t!(key, locale = l, service = service, network = network),
        }
        .into_owned()
    }
}

impl ErrorKind {
    fn field(&self) -> Option<&String> {
        match self {
            Self::FieldRequired(field)
            | Self::SecretRequired(field)
            | Self::FieldUnsupported(field)
            | Self::VersionNotLatest { field, .. } => Some(field),
            _ => None,
        }
    }

    pub(crate) fn im_api(platform: &'static str, detail: impl Into<String>) -> Self {
        Self::ImApi {
            platform,
            detail: detail.into(),
            source: None,
        }
    }

    fn message(&self, locale: Locale) -> String {
        let code: &'static str = self.into();
        let key = format!("error.{code}");
        let l = locale.tag();
        match self {
            Self::Spec(problem) => return problem.message(locale),
            Self::Instance(problem) => return problem.message(locale),
            Self::Cert(problem) => return problem.message(locale),
            Self::Acme(problem) => return problem.message(locale),
            Self::PingressBusy(apps) | Self::InsightBusy(apps) => {
                t!(key, locale = l, apps = apps)
            }
            Self::DomainLimit { limit, current } => {
                t!(key, locale = l, limit = limit, current = current)
            }
            Self::XJobLimit { limit } => t!(key, locale = l, limit = limit),
            Self::InvalidDomain(domain) => t!(key, locale = l, domain = domain),
            Self::NoService => t!(key, locale = l, service = config::SERVICE_NAME),
            Self::InvalidBody(detail)
            | Self::InsightQueryFailed(detail)
            | Self::VolumeAgentHttp { detail, .. }
            | Self::UnprocessableBody(detail)
            | Self::Panic(detail)
            | Self::UnknownTool(detail) => {
                t!(key, locale = l, detail = detail)
            }
            Self::ImApi {
                platform, detail, ..
            } => {
                t!(key, locale = l, platform = platform, detail = detail)
            }
            Self::InsightLabelMatchesNoNodes(label) => t!(key, locale = l, label = label),
            Self::OperationTimeout { operation, .. } => {
                t!(key, locale = l, operation = operation)
            }
            Self::InvalidName(name) | Self::ReservedName(name) => {
                t!(key, locale = l, name = name)
            }
            Self::TagNotSemver(tag) => t!(key, locale = l, tag = tag),
            Self::FieldRequired(field)
            | Self::SecretRequired(field)
            | Self::FieldUnsupported(field) => t!(key, locale = l, field = field),
            Self::VersionNotLatest { field, latest } => {
                t!(key, locale = l, field = field, latest = latest)
            }
            Self::GroupMissing(group) => t!(key, locale = l, group = group),
            Self::UnsupportedMediaType(media_type) => {
                t!(key, locale = l, media_type = media_type)
            }
            Self::Reqwest(e) => t!(key, locale = l, detail = e),
            _ => t!(key, locale = l),
        }
        .into_owned()
    }
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message(Locale::En))
    }
}

impl std::error::Error for ErrorKind {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OperationTimeout { source, .. }
            | Self::LlmTimeout(source)
            | Self::Acme(AcmeProblem::Timeout(source)) => Some(source),
            Self::Llm(source) => Some(source),
            Self::AcmeClient(source) => Some(source),
            Self::Reqwest(source) | Self::VolumeAgentHttp { source, .. } => Some(source),
            Self::VolumeDescriptionSave(source) | Self::VolumeInUse(source) => {
                Some(source.as_ref())
            }
            Self::Logging(source) => Some(source),
            Self::Io(source) => Some(source),
            Self::Docker(source) => Some(source),
            Self::Cipher(source) => Some(source),
            Self::ImApi {
                source: Some(source),
                ..
            }
            | Self::Serde(source) => Some(source),
            Self::Base64(source) => Some(source),
            Self::Rng(source) => Some(source),
            Self::Argon2(source) => Some(source),
            Self::Smtp(source) => Some(source),
            Self::Imap(source) => Some(source),
            Self::Ws(source) => Some(source),
            Self::WsProto(source) => Some(source),
            Self::DiscordHttp(source) => Some(source),
            Self::DiscordHttpBody(source) => Some(source),
            Self::DiscordGateway(source) => Some(source),
            Self::Askama(source) => Some(source),
            Self::Totp(source) => Some(source),
            Self::Image(source) => Some(source),
            _ => None,
        }
    }
}

located_error! {
    Error => ErrorKind,
    from {
        logging::Error => Logging,
        std::io::Error => Io,
        docker::Error => Docker,
        cipher::Error => Cipher,
        serde_json::Error => Serde,
        base64::DecodeError => Base64,
        getrandom::Error => Rng,
        argon2::password_hash::Error => Argon2,
        reqwest::Error => Reqwest,
        mail_send::Error => Smtp,
        async_imap::error::Error => Imap,
        tokio_tungstenite::tungstenite::Error => Ws,
        prost::DecodeError => WsProto,
        twilight_http::Error => DiscordHttp,
        twilight_http::response::DeserializeBodyError => DiscordHttpBody,
        twilight_gateway::error::ReceiveMessageError => DiscordGateway,
        genai::Error => Llm,
        instant_acme::Error => AcmeClient,
        askama::Error => Askama,
        totp_rs::TotpError => Totp,
        oci_spec::distribution::ParseError => Image,
        garde::Report => Validation,
        SpecProblem => Spec,
        InstanceProblem => Instance,
        CertProblem => Cert,
        AcmeProblem => Acme,
    }
}

impl Error {
    pub(crate) fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    pub(crate) fn nested(mut self, prefix: &str) -> Self {
        if let ErrorKind::FieldRequired(field)
        | ErrorKind::SecretRequired(field)
        | ErrorKind::FieldUnsupported(field)
        | ErrorKind::VersionNotLatest { field, .. } = &mut self.kind
        {
            *field = format!("{prefix}.{field}");
        }
        self
    }

    pub(crate) fn is_conflict(&self) -> bool {
        matches!(self.kind, ErrorKind::Conflict)
    }

    #[expect(clippy::too_many_lines)]
    pub(crate) fn status(&self) -> StatusCode {
        use ErrorKind::{
            Acme, AcmeCaMismatch, AcmeClient, AgentEmptyResponse, AlreadyInitialized,
            AlreadyRolledBack, ApiKeyInvalid, AppExists, AppNotFound, Argon2, Askama, AuthRequired,
            Base64, Cert, CertNotFound, Cipher, ConfigNotFound, Conflict, CredentialsInvalid,
            CurrentPasswordInvalid, DiscordGateway, DiscordHttp, DiscordHttpBody, Docker,
            DomainLimit, EmailTaken, FieldRequired, FieldUnsupported, Forbidden, GroupBusy,
            GroupExists, GroupMissing, GroupNotFound, GuardianNotFound, GuardianPending,
            GuardianResolved, GuardianStatefulUnsupported, ImApi, ImNotBound, Image, Imap,
            InsightBusy, InsightLabelMatchesNoNodes, InsightLocalStorageRequiresNode,
            InsightNotEnabled, InsightQueryFailed, InsightStorageMigrationUnsupported, Instance,
            InvalidBody, InvalidDomain, InvalidName, InvalidOperationOutput, Io, Llm, LlmTimeout,
            Logging, MethodNotAllowed, MountsPlacementConflict, MountsRequireStateful, NoService,
            NoSwarmManager, NodeNotFound, NotBase64, NotInitialized, NothingToRollBack,
            OperationTimeout, Panic, PayloadTooLarge, PingressBusy, PingressExists,
            PingressNotEnabled, ReleaseUnavailable, ReleaseUrlInvalid, ReplicaBoundsInvalid,
            Reqwest, ReservedName, Rng, RouteNotFound, SecretRequired, SelfImmutable, Serde, Smtp,
            Spec, StatefulReplicasPinned, TagNotSemver, TokenInvalid, Totp, TotpAlreadyEnabled,
            TotpCodeInvalid, UnknownTool, UnprocessableBody, UnsupportedMediaType, UserNotFound,
            Validation, VersionNotLatest, VolumeAgentHttp, VolumeDescriptionSave, VolumeExists,
            VolumeInUse, VolumeNotFound, VolumeUnavailable, Ws, WsProto, XConfigMountsConflict,
            XHealthExists, XHealthIntervalInvalid, XHealthNotEnabled, XHealthRunning,
            XHealthTargetAmbiguous, XHealthTargetNotFound, XJobExists, XJobLimit, XJobNotFound,
            XJobRunning,
        };
        match &self.kind {
            NoService | NoSwarmManager | Instance(_) | VolumeUnavailable => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            AlreadyInitialized | NotInitialized | Conflict | EmailTaken | GroupExists
            | GroupBusy | AppExists | AlreadyRolledBack | NothingToRollBack | XHealthExists
            | XHealthRunning | XJobExists | XJobRunning | GuardianPending | GuardianResolved
            | PingressExists | PingressBusy(_) | InsightBusy(_) | AcmeCaMismatch
            | TotpAlreadyEnabled | VolumeExists | VolumeInUse(_) => StatusCode::CONFLICT,
            Validation(_)
            | UnprocessableBody(_)
            | CurrentPasswordInvalid
            | TotpCodeInvalid
            | StatefulReplicasPinned
            | ReplicaBoundsInvalid
            | MountsRequireStateful
            | MountsPlacementConflict
            | XConfigMountsConflict
            | XHealthTargetNotFound
            | XHealthTargetAmbiguous
            | XHealthIntervalInvalid
            | GuardianStatefulUnsupported
            | DomainLimit { .. }
            | XJobLimit { .. }
            | InsightLocalStorageRequiresNode
            | FieldRequired(_)
            | SecretRequired(_)
            | FieldUnsupported(_)
            | VersionNotLatest { .. }
            | InsightLabelMatchesNoNodes(_)
            | InsightStorageMigrationUnsupported
            | Cert(_) => StatusCode::UNPROCESSABLE_ENTITY,
            InvalidBody(_) | InvalidName(_) | ReservedName(_) | InvalidDomain(_) | Image(_)
            | TagNotSemver(_) | NotBase64 | GroupMissing(_) => StatusCode::BAD_REQUEST,
            AuthRequired | TokenInvalid | CredentialsInvalid | ApiKeyInvalid => {
                StatusCode::UNAUTHORIZED
            }
            Forbidden | SelfImmutable => StatusCode::FORBIDDEN,
            AppNotFound | GroupNotFound | ConfigNotFound | RouteNotFound | XHealthNotEnabled
            | XJobNotFound | GuardianNotFound | PingressNotEnabled | InsightNotEnabled
            | CertNotFound | NodeNotFound | UnknownTool(_) | UserNotFound | ImNotBound
            | VolumeNotFound => StatusCode::NOT_FOUND,
            MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            UnsupportedMediaType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            OperationTimeout { .. } | LlmTimeout(_) => StatusCode::GATEWAY_TIMEOUT,
            ReleaseUnavailable
            | Reqwest(_)
            | DiscordHttp(_)
            | DiscordHttpBody(_)
            | Acme(_)
            | InsightQueryFailed(_)
            | VolumeAgentHttp { .. }
            | ImApi { .. }
            | AcmeClient(_) => StatusCode::BAD_GATEWAY,
            Spec(_)
            | Logging(_)
            | Io(_)
            | Docker(_)
            | Cipher(_)
            | Serde(_)
            | Base64(_)
            | Rng(_)
            | Argon2(_)
            | AgentEmptyResponse
            | Llm(_)
            | Panic(_)
            | Smtp(_)
            | Imap(_)
            | Ws(_)
            | WsProto(_)
            | DiscordGateway(_)
            | InvalidOperationOutput
            | Askama(_)
            | ReleaseUrlInvalid
            | VolumeDescriptionSave(_)
            | Totp(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub(crate) fn detail(&self, locale: Locale) -> (String, Option<BTreeMap<String, String>>) {
        match &self.kind {
            ErrorKind::Validation(report) => {
                (self.kind.message(locale), Some(field_errors(report)))
            }
            _ if self.status() == StatusCode::INTERNAL_SERVER_ERROR => (self.chain(locale), None),
            kind => (kind.message(locale), None),
        }
    }

    pub(crate) fn problem(&self) -> Problem {
        let status = self.status();
        let (detail, errors) = self.detail(Locale::En);
        let errors = errors.or_else(|| {
            self.kind
                .field()
                .map(|field| BTreeMap::from([(field.clone(), detail.clone())]))
        });
        Problem {
            type_: "about:blank",
            title: status.canonical_reason().unwrap_or("Error"),
            status: status.as_u16(),
            code: <&'static str>::from(&self.kind),
            detail,
            errors,
        }
    }

    pub(crate) fn chain(&self, locale: Locale) -> String {
        let mut text = self.kind.message(locale);
        let mut source = std::error::Error::source(&self.kind);
        while let Some(e) = source {
            text.push_str(": ");
            text.push_str(&e.to_string());
            source = e.source();
        }
        let location = self.location;
        format!("{text} (at {}:{})", location.file(), location.line())
    }
}

#[track_caller]
pub(crate) fn required<T>(value: Option<T>, field: &str) -> Result<T> {
    match value {
        Some(value) => Ok(value),
        None => Err(ErrorKind::FieldRequired(field.to_owned()).into()),
    }
}

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct Problem {
    #[serde(rename = "type")]
    type_: &'static str,
    title: &'static str,
    status: u16,
    code: &'static str,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<BTreeMap<String, String>>,
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.status();
        if status.is_server_error() {
            tracing::error!(error = ?self, "request failed");
        }
        let body = self.problem();
        (
            status,
            [
                (CONTENT_TYPE, "application/problem+json"),
                (CACHE_CONTROL, "no-store"),
            ],
            Json(body),
        )
            .into_response()
    }
}

fn field_errors(report: &garde::Report) -> BTreeMap<String, String> {
    report
        .iter()
        .map(|(path, error)| (path.to_string(), error.to_string()))
        .collect()
}
