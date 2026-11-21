pub(crate) mod dedup;
pub(crate) mod discord;
pub(crate) mod email;
pub(crate) mod feishu;
pub(crate) mod slack;
pub(crate) mod telegram;

use std::collections::BTreeMap;
use std::sync::Arc;

use strum::VariantArray;

use crate::concept;
use crate::config;
use crate::error::Result;

#[derive(Clone)]
pub(crate) enum Client {
    Feishu(Arc<feishu::Rest>),
    Slack(Arc<slack::Rest>),
    Telegram(Arc<telegram::Rest>),
    Discord(Arc<discord::Rest>),
}

impl Client {
    fn build(
        settings: &concept::setting::Settings,
        im: concept::message::Im,
    ) -> Result<Option<Self>> {
        let client = match im {
            concept::message::Im::Feishu => {
                let Some(cfg) = settings.feishu() else {
                    return Ok(None);
                };
                Self::Feishu(Arc::new(feishu::Rest::feishu(cfg)?))
            }
            concept::message::Im::Lark => {
                let Some(cfg) = settings.lark() else {
                    return Ok(None);
                };
                Self::Feishu(Arc::new(feishu::Rest::lark(cfg)?))
            }
            concept::message::Im::Slack => {
                let Some(cfg) = settings.slack() else {
                    return Ok(None);
                };
                Self::Slack(Arc::new(slack::Rest::new(cfg)?))
            }
            concept::message::Im::Telegram => {
                let Some(token) = settings.telegram() else {
                    return Ok(None);
                };
                Self::Telegram(Arc::new(telegram::Rest::new(token)?))
            }
            concept::message::Im::Discord => {
                let Some(token) = settings.discord() else {
                    return Ok(None);
                };
                Self::Discord(Arc::new(discord::Rest::new(token)?))
            }
        };
        Ok(Some(client))
    }

    pub(crate) async fn send_dm(&self, id: &str, text: &str) -> Result<()> {
        send(async {
            match self {
                Self::Feishu(rest) => rest.send("open_id", id, text).await,
                Self::Slack(rest) => rest.send(id, text).await,
                Self::Telegram(rest) => rest.send(id, text).await,
                Self::Discord(rest) => rest.send_dm(id, text).await,
            }
        })
        .await
    }

    pub(crate) async fn send_group(&self, chat_id: &str, text: &str) -> Result<()> {
        send(async {
            match self {
                Self::Feishu(rest) => rest.send("chat_id", chat_id, text).await,
                Self::Slack(rest) => rest.send(chat_id, text).await,
                Self::Telegram(rest) => rest.send(chat_id, text).await,
                Self::Discord(rest) => rest.send_group(chat_id, text).await,
            }
        })
        .await
    }
}

async fn send(future: impl Future<Output = Result<()>>) -> Result<()> {
    tokio::time::timeout(config::AGENT_SEND_TIMEOUT.unsigned_abs(), future)
        .await
        .map_err(|source| std::io::Error::new(std::io::ErrorKind::TimedOut, source))?
}

fn field(value: &serde_json::Value, pointer: &str) -> Option<String> {
    let value = value.pointer(pointer)?.as_str()?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(crate) const fn has_profile(im: concept::message::Im) -> bool {
    matches!(
        im,
        concept::message::Im::Feishu | concept::message::Im::Lark | concept::message::Im::Slack
    )
}

pub(crate) async fn observe_profile(
    settings: &concept::setting::Settings,
    im: concept::message::Im,
    id: &str,
) -> Result<(Option<String>, Option<String>)> {
    match Client::build(settings, im)? {
        Some(Client::Feishu(rest)) => rest.observe_profile(id).await,
        Some(Client::Slack(rest)) => rest.observe_profile(id).await,
        Some(Client::Telegram(_) | Client::Discord(_)) | None => Ok((None, None)),
    }
}

pub(crate) struct Adapters {
    pub(crate) im: BTreeMap<concept::message::Im, Client>,
    pub(crate) email: Option<Arc<email::Channel>>,
}

impl Adapters {
    pub(crate) fn build(settings: &concept::setting::Settings) -> Result<Self> {
        let mut im = BTreeMap::new();
        for &platform in concept::message::Im::VARIANTS {
            if let Some(client) = Client::build(settings, platform)? {
                im.insert(platform, client);
            }
        }
        Ok(Self {
            im,
            email: settings
                .email()
                .map(|cfg| Arc::new(email::Channel::new(cfg.clone()))),
        })
    }
}

pub(crate) fn jitter(window_secs: u64) -> std::time::Duration {
    let window = window_secs.saturating_mul(1000).max(1);
    let mut bytes = [0u8; 8];
    let ms = match getrandom::fill(&mut bytes) {
        Ok(()) => u64::from_le_bytes(bytes) % window,
        Err(e) => {
            tracing::warn!(error = ?e, "ws reconnect jitter rng failed, using half window");
            window / 2
        }
    };
    std::time::Duration::from_millis(ms)
}

pub(crate) async fn run<F, Fut>(
    client: Client,
    cancel: tokio_util::sync::CancellationToken,
    on_inbound: F,
) where
    F: Fn(concept::message::Inbound) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    match client {
        Client::Feishu(rest) => feishu::ws::run(rest, cancel, on_inbound).await,
        Client::Slack(rest) => slack::ws::run(rest, cancel, on_inbound).await,
        Client::Telegram(rest) => telegram::poll::run(rest, cancel, on_inbound).await,
        Client::Discord(rest) => discord::gateway::run(rest, cancel, on_inbound).await,
    }
}

pub(crate) fn spawn_inbound<F, Fut>(
    on_inbound: F,
) -> (
    tokio::sync::mpsc::Sender<concept::message::Inbound>,
    tokio::task::JoinHandle<()>,
)
where
    F: Fn(concept::message::Inbound) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (inbound, mut received) = tokio::sync::mpsc::channel(config::AGENT_IM_INBOUND_QUEUE);
    let worker = tokio::spawn(async move {
        while let Some(message) = received.recv().await {
            on_inbound(message).await;
        }
    });
    (inbound, worker)
}

pub(crate) async fn drain(platform: &'static str, mut worker: tokio::task::JoinHandle<()>) {
    match tokio::time::timeout(config::AGENT_INBOUND_DRAIN.unsigned_abs(), &mut worker).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::error!(error = ?e, "{platform} inbound worker failed"),
        Err(_) => {
            tracing::warn!("{platform} inbound queue did not drain before shutdown");
            worker.abort();
            let _ = worker.await;
        }
    }
}
