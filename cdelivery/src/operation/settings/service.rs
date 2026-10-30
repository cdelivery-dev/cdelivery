use crate::concept;
use crate::error::{self, ErrorKind, Result};

pub(super) fn resolve_discovery(
    dto: super::typ::DiscoverAgentModelsRequest,
    current: Option<&concept::setting::Agent>,
) -> Result<(concept::setting::Provider, Option<String>, Option<String>)> {
    let provider = dto.provider.into();
    let endpoint = normalize_endpoint(provider, dto.endpoint)?;
    let same_scope =
        current.is_some_and(|agent| agent.provider() == provider && agent.endpoint() == &endpoint);
    let api_key = resolve_api_key(
        provider,
        dto.api_key,
        same_scope,
        current.and_then(|agent| agent.api_key().as_ref()),
    )?;
    Ok((provider, api_key, endpoint))
}

pub(super) fn resolve_agent(
    dto: super::typ::UpdateAgentSettingsRequest,
    current: Option<&concept::setting::Agent>,
) -> Result<concept::setting::Agent> {
    let language = current.map_or(crate::locale::Locale::En, concept::setting::Agent::language);
    let language = dto.language.unwrap_or(language);
    let duty_policy = current.map_or(
        concept::setting::DutyPolicy::NearCritical,
        concept::setting::Agent::duty_policy,
    );
    let duty_policy = dto.duty_policy.map_or(duty_policy, Into::into);
    let patrol_interval = current.map_or(
        crate::config::AGENT_PATROL_INTERVAL_DEFAULT,
        concept::setting::Agent::patrol_interval,
    );
    let patrol_interval = dto.patrol_interval.map_or(patrol_interval, Into::into);
    let role = current.map_or(concept::user::Role::Reader, concept::setting::Agent::role);
    let role = dto.role.map_or(role, Into::into);
    let provider = error::required(
        dto.provider
            .map(Into::into)
            .or(current.map(concept::setting::Agent::provider)),
        "provider",
    )?;
    let model = required_string(
        dto.model,
        current.map(concept::setting::Agent::model),
        "model",
    )?;
    let endpoint = dto
        .endpoint
        .unwrap_or_else(|| current.and_then(|agent| agent.endpoint().clone()));
    let endpoint = normalize_endpoint(provider, endpoint)?;
    let same_scope =
        current.is_some_and(|agent| agent.provider() == provider && agent.endpoint() == &endpoint);
    let api_key = resolve_api_key(
        provider,
        dto.api_key,
        same_scope,
        current.and_then(|agent| agent.api_key().as_ref()),
    )?;
    Ok(concept::setting::Agent::new(
        provider,
        model,
        api_key,
        endpoint,
        language,
        duty_policy,
        patrol_interval,
        role,
    ))
}

fn resolve_api_key(
    provider: concept::setting::Provider,
    api_key: Option<String>,
    same_scope: bool,
    current: Option<&String>,
) -> Result<Option<String>> {
    if provider == concept::setting::Provider::Ollama {
        if api_key.is_some() {
            return Err(ErrorKind::FieldUnsupported("api_key".into()).into());
        }
        return Ok(None);
    }
    required_secret(api_key, same_scope, current, "api_key").map(Some)
}

fn normalize_endpoint(
    provider: concept::setting::Provider,
    endpoint: Option<String>,
) -> Result<Option<String>> {
    if matches!(
        provider,
        concept::setting::Provider::Ollama | concept::setting::Provider::OpenAICompatible
    ) {
        let endpoint = error::required(endpoint, "endpoint")?;
        return Ok(Some(format!("{}/", endpoint.trim_end_matches('/'))));
    }
    if endpoint.is_some() {
        return Err(ErrorKind::FieldUnsupported("endpoint".into()).into());
    }
    Ok(None)
}

pub(super) fn resolve_email(
    dto: super::typ::UpdateEmailSettingsRequest,
    current: Option<&concept::setting::Email>,
) -> Result<concept::setting::Email> {
    let address = required_string(
        dto.address,
        current.map(concept::setting::Email::address),
        "address",
    )?;
    let smtp = resolve_smtp(&address, dto.smtp, current)?;
    let imap = match dto.imap {
        Some(None) => None,
        Some(Some(changes)) => Some(resolve_imap(&address, Some(changes), current)?),
        None => current
            .and_then(|email| email.imap().as_ref())
            .map(|_| resolve_imap(&address, None, current))
            .transpose()?,
    };
    Ok(concept::setting::Email::new(address, smtp, imap))
}

fn resolve_smtp(
    address: &str,
    patch: Option<super::typ::UpdateSmtpSettingsRequest>,
    current: Option<&concept::setting::Email>,
) -> Result<concept::setting::Smtp> {
    let patch = patch.unwrap_or_default();
    let saved = current.map(concept::setting::Email::smtp);
    let host = required_string(
        patch.host,
        saved.map(concept::setting::Smtp::host),
        "smtp.host",
    )?;
    let port = error::required(
        patch.port.or(saved.map(concept::setting::Smtp::port)),
        "smtp.port",
    )?;
    let smtp_scope = current.is_some_and(|email| {
        email.address() == address && email.smtp().host() == &host && email.smtp().port() == port
    });
    let smtp_password = required_secret(
        patch.password,
        smtp_scope,
        saved.map(concept::setting::Smtp::password),
        "smtp.password",
    )?;
    Ok(concept::setting::Smtp::new(host, port, smtp_password))
}

fn resolve_imap(
    address: &str,
    patch: Option<super::typ::UpdateImapSettingsRequest>,
    current: Option<&concept::setting::Email>,
) -> Result<concept::setting::Imap> {
    let saved = current.and_then(|email| email.imap().as_ref());
    let patch = patch.unwrap_or_default();
    let host = required_string(
        patch.host,
        saved.map(concept::setting::Imap::host),
        "imap.host",
    )?;
    let port = error::required(
        patch.port.or(saved.map(concept::setting::Imap::port)),
        "imap.port",
    )?;
    let authserv_id = required_string(
        patch.authserv_id,
        saved.map(concept::setting::Imap::authserv_id),
        "imap.authserv_id",
    )?;
    let same_scope = current.is_some_and(|email| {
        email.address() == address
            && email
                .imap()
                .as_ref()
                .is_some_and(|saved| saved.host() == &host && saved.port() == port)
    });
    let password = required_secret(
        patch.password,
        same_scope,
        saved.map(concept::setting::Imap::password),
        "imap.password",
    )?;
    Ok(concept::setting::Imap::new(
        host,
        port,
        password,
        authserv_id,
    ))
}

pub(super) fn resolve_feishu(
    dto: super::typ::UpdateFeishuSettingsRequest,
    current: Option<&concept::setting::Feishu>,
) -> Result<concept::setting::Feishu> {
    let app_id = required_string(
        dto.app_id,
        current.map(concept::setting::Feishu::app_id),
        "app_id",
    )?;
    let same_scope = current.is_some_and(|feishu| feishu.app_id() == &app_id);
    let secret = required_secret(
        dto.app_secret,
        same_scope,
        current.map(concept::setting::Feishu::app_secret),
        "app_secret",
    )?;
    Ok(concept::setting::Feishu::new(app_id, secret))
}

pub(super) fn resolve_slack(
    dto: super::typ::UpdateSlackSettingsRequest,
    current: Option<&concept::setting::Slack>,
) -> Result<concept::setting::Slack> {
    let same_scope = current.is_some();
    let bot_token = required_secret(
        dto.bot_token,
        same_scope,
        current.map(concept::setting::Slack::bot_token),
        "bot_token",
    )?;
    let app_token = required_secret(
        dto.app_token,
        same_scope,
        current.map(concept::setting::Slack::app_token),
        "app_token",
    )?;
    Ok(concept::setting::Slack::new(bot_token, app_token))
}

pub(super) fn resolve_bot(
    dto: super::typ::UpdateBotSettingsRequest,
    current: Option<&String>,
) -> Result<String> {
    required_secret(dto.bot_token, current.is_some(), current, "bot_token")
}

fn required_string(patch: Option<String>, current: Option<&String>, field: &str) -> Result<String> {
    error::required(patch.or_else(|| current.cloned()), field)
}

fn required_secret(
    provided: Option<String>,
    same_scope: bool,
    current: Option<&String>,
    field: &str,
) -> Result<String> {
    provided
        .or_else(|| current.filter(|_| same_scope).cloned())
        .ok_or_else(|| ErrorKind::SecretRequired(field.to_owned()).into())
}
