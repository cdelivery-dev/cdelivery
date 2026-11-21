use cipher::MasterKey;
use tokio::sync::Notify;

use crate::concept;
use crate::error::Result;
use crate::infra;

pub(crate) async fn get(
    docker: &infra::docker::Docker,
    key: &MasterKey,
) -> Result<concept::setting::Settings> {
    Ok(infra::setting::load(docker, key).await?.into_inner())
}

macro_rules! channel {
    ($name:ident, $extract:ident, $value:ty, $verify:expr) => {
        pub(crate) async fn $name(
            docker: &infra::docker::Docker,
            key: &MasterKey,
            reload: &Notify,
            resolve: impl FnOnce(Option<&$value>) -> Result<$value>,
        ) -> Result<$value> {
            set(
                docker,
                key,
                reload,
                concept::setting::Settings::$extract,
                concept::setting::Settings::$name,
                resolve,
                $verify,
            )
            .await
        }
    };
}

channel!(set_email, email, concept::setting::Email, async |email| {
    infra::channel::email::Channel::new(email.clone())
        .verify()
        .await
});
channel!(
    set_feishu,
    feishu,
    concept::setting::Feishu,
    async |feishu| { infra::channel::feishu::Rest::feishu(feishu)?.verify().await }
);
channel!(set_lark, lark, concept::setting::Feishu, async |lark| {
    infra::channel::feishu::Rest::lark(lark)?.verify().await
});
channel!(set_slack, slack, concept::setting::Slack, async |slack| {
    infra::channel::slack::Rest::new(slack)?.verify().await
});
channel!(set_telegram, telegram, String, async |token| {
    infra::channel::telegram::Rest::new(token)?.verify().await
});
channel!(set_discord, discord, String, async |token| {
    infra::channel::discord::Rest::new(token)?.verify().await
});

async fn set<C: Clone + PartialEq>(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    reload: &Notify,
    extract: fn(&concept::setting::Settings) -> Option<&C>,
    store: fn(&mut concept::setting::Settings, Option<C>),
    resolve: impl FnOnce(Option<&C>) -> Result<C>,
    verify: impl AsyncFnOnce(&C) -> Result<()>,
) -> Result<C> {
    let mut settings = infra::setting::load(docker, key).await?;
    let value = resolve(extract(settings.get()))?;
    if extract(settings.get()) == Some(&value) {
        return Ok(value);
    }
    verify(&value).await?;
    store(settings.get_mut(), Some(value.clone()));
    save(docker, key, reload, settings).await?;
    Ok(value)
}

pub(crate) async fn set_agent(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    reload: &Notify,
    duty_enabled: Option<bool>,
    resolve: impl FnOnce(Option<&concept::setting::Agent>) -> Result<concept::setting::Agent>,
) -> Result<(concept::setting::Agent, bool)> {
    let mut settings = infra::setting::load(docker, key).await?;
    let duty_paused = !duty_enabled.unwrap_or(settings.get().duty_agent().is_some());
    let agent = resolve(settings.get().agent())?;
    if settings.get().agent() == Some(&agent) && settings.get().duty_paused() == duty_paused {
        return Ok((agent, !duty_paused));
    }
    if settings.get().agent().is_none_or(|current| {
        current.provider() != agent.provider()
            || current.model() != agent.model()
            || current.api_key() != agent.api_key()
            || current.endpoint() != agent.endpoint()
    }) {
        infra::llm::probe(&agent).await?;
    }
    settings.get_mut().set_agent(Some(agent.clone()));
    settings.get_mut().set_duty_paused(duty_paused);
    save(docker, key, reload, settings).await?;
    Ok((agent, !duty_paused))
}

pub(crate) async fn clear<C>(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    reload: &Notify,
    store: fn(&mut concept::setting::Settings, Option<C>),
) -> Result<()> {
    let mut settings = infra::setting::load(docker, key).await?;
    store(settings.get_mut(), None);
    save(docker, key, reload, settings).await
}

pub(crate) async fn discover_models(
    provider: concept::setting::Provider,
    api_key: Option<&str>,
    endpoint: Option<&str>,
) -> Result<Vec<String>> {
    infra::llm::models(provider, api_key, endpoint).await
}

async fn save(
    docker: &infra::docker::Docker,
    key: &MasterKey,
    reload: &Notify,
    settings: concept::versioned::Versioned<concept::setting::Settings>,
) -> Result<()> {
    let changed = settings.changed();
    infra::setting::save(docker, key, &settings).await?;
    if changed {
        reload.notify_one();
    }
    Ok(())
}
