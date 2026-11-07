use genai::adapter::AdapterKind;
use genai::resolver::{AuthData, Endpoint};
use genai::{ModelIden, ServiceTarget};
use jiff::SignedDuration;
use serde::Deserialize;

use crate::concept;
use crate::config;
use crate::error::{ErrorKind, Result};

#[derive(Debug)]
pub(crate) struct Llm {
    client: genai::Client,
    target: ServiceTarget,
    timeout: SignedDuration,
}

impl Llm {
    pub(crate) fn new(cfg: &concept::setting::Agent, timeout: SignedDuration) -> Result<Self> {
        Ok(Self {
            client: genai::Client::builder().build()?,
            target: service_target(cfg)?,
            timeout,
        })
    }

    pub(crate) async fn chat(
        &self,
        req: genai::chat::ChatRequest,
        options: Option<&genai::chat::ChatOptions>,
    ) -> Result<genai::chat::ChatResponse> {
        let call = self.client.exec_chat(self.target.clone(), req, options);
        match tokio::time::timeout(self.timeout.unsigned_abs(), call).await {
            Ok(response) => Ok(response?),
            Err(source) => Err(ErrorKind::LlmTimeout(source).into()),
        }
    }
}

pub(crate) async fn models(
    provider: concept::setting::Provider,
    api_key: Option<&str>,
    endpoint: Option<&str>,
) -> Result<Vec<String>> {
    let future = async {
        let web = reqwest::Client::new();
        match provider {
            concept::setting::Provider::Anthropic => anthropic_models(&web, api_key).await,
            concept::setting::Provider::Gemini => gemini_models(&web, api_key).await,
            provider => {
                let (adapter, endpoint, auth) = base(provider, api_key, endpoint)?;
                let client = genai::Client::builder().with_reqwest(web).build()?;
                let mut models = client.all_model_names(adapter, (endpoint, auth)).await?;
                if provider == concept::setting::Provider::OpenAI {
                    models.retain(|model| openai_chat_model(model));
                }
                Ok(models)
            }
        }
    };
    let models =
        match tokio::time::timeout(config::AGENT_CHANNEL_TIMEOUT.unsigned_abs(), future).await {
            Ok(models) => models?,
            Err(source) => return Err(ErrorKind::LlmTimeout(source).into()),
        };
    Ok(normalize_models(models))
}

fn normalize_models(mut models: Vec<String>) -> Vec<String> {
    for model in &mut models {
        *model = model.trim().to_owned();
    }
    models.retain(|model| !model.is_empty());
    models.sort_unstable();
    models.dedup();
    models
}

pub(crate) async fn probe(cfg: &concept::setting::Agent) -> Result<()> {
    let llm = Llm::new(cfg, config::AGENT_CHANNEL_TIMEOUT)?;
    let request = genai::chat::ChatRequest::new(vec![genai::chat::ChatMessage::user(
        "Call cdelivery_probe exactly once with value ready.",
    )])
    .with_tools(vec![
        genai::chat::Tool::new("cdelivery_probe")
            .with_strict(false)
            .with_schema(serde_json::json!({
                "type": "object",
                "properties": {"value": {"type": "string"}},
                "required": ["value"],
                "additionalProperties": false,
            })),
    ]);
    let calls = llm.chat(request, None).await?.into_tool_calls();
    if let [call] = calls.as_slice()
        && call.fn_name == "cdelivery_probe"
        && call
            .fn_arguments
            .get("value")
            .and_then(serde_json::Value::as_str)
            == Some("ready")
    {
        Ok(())
    } else {
        Err(ErrorKind::InvalidBody("model did not complete the tool probe".to_owned()).into())
    }
}

fn openai_chat_model(model: &str) -> bool {
    !model.starts_with("text-embedding")
        && matches!(
            AdapterKind::from_model(model),
            Ok(AdapterKind::OpenAI | AdapterKind::OpenAIResp)
        )
}

async fn anthropic_models(web: &reqwest::Client, api_key: Option<&str>) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Model {
        id: String,
    }
    #[derive(Deserialize)]
    struct Page {
        data: Vec<Model>,
        has_more: bool,
        last_id: Option<String>,
    }
    let key = required_key(api_key)?;
    let mut after_id = None;
    let mut models = Vec::new();
    loop {
        let mut request = web
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .query(&[("limit", "1000")]);
        if let Some(after_id) = &after_id {
            request = request.query(&[("after_id", after_id)]);
        }
        let page: Page = request.send().await?.error_for_status()?.json().await?;
        models.extend(page.data.into_iter().map(|model| model.id));
        if !page.has_more {
            return Ok(models);
        }
        after_id = page.last_id;
        if after_id.is_none() {
            return Err(
                ErrorKind::InvalidBody("anthropic model page has no cursor".to_owned()).into(),
            );
        }
    }
}

async fn gemini_models(web: &reqwest::Client, api_key: Option<&str>) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Model {
        name: String,
        #[serde(default)]
        supported_generation_methods: Vec<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Page {
        #[serde(default)]
        models: Vec<Model>,
        next_page_token: Option<String>,
    }
    let key = required_key(api_key)?;
    let mut page_token = None;
    let mut models = Vec::new();
    loop {
        let mut request = web
            .get("https://generativelanguage.googleapis.com/v1beta/models")
            .header("x-goog-api-key", key)
            .query(&[("pageSize", "1000")]);
        if let Some(page_token) = &page_token {
            request = request.query(&[("pageToken", page_token)]);
        }
        let page: Page = request.send().await?.error_for_status()?.json().await?;
        models.extend(
            page.models
                .into_iter()
                .filter(|model| {
                    model
                        .supported_generation_methods
                        .iter()
                        .any(|method| method == "generateContent")
                })
                .map(|model| {
                    model
                        .name
                        .strip_prefix("models/")
                        .unwrap_or(&model.name)
                        .to_owned()
                }),
        );
        let Some(next) = page.next_page_token.filter(|token| !token.is_empty()) else {
            return Ok(models);
        };
        page_token = Some(next);
    }
}

fn base(
    provider: concept::setting::Provider,
    api_key: Option<&str>,
    endpoint: Option<&str>,
) -> Result<(AdapterKind, Endpoint, AuthData)> {
    Ok(match provider {
        concept::setting::Provider::OpenAI => (
            AdapterKind::OpenAI,
            Endpoint::from_static("https://api.openai.com/v1/"),
            auth(api_key)?,
        ),
        concept::setting::Provider::Anthropic => (
            AdapterKind::Anthropic,
            Endpoint::from_static("https://api.anthropic.com/v1/"),
            auth(api_key)?,
        ),
        concept::setting::Provider::Gemini => (
            AdapterKind::Gemini,
            Endpoint::from_static("https://generativelanguage.googleapis.com/v1beta/"),
            auth(api_key)?,
        ),
        concept::setting::Provider::DeepSeek => (
            AdapterKind::DeepSeek,
            Endpoint::from_static("https://api.deepseek.com/v1/"),
            auth(api_key)?,
        ),
        concept::setting::Provider::OpenRouter => (
            AdapterKind::OpenRouter,
            Endpoint::from_static("https://openrouter.ai/api/v1/"),
            auth(api_key)?,
        ),
        concept::setting::Provider::Ollama => (
            AdapterKind::Ollama,
            custom_endpoint(endpoint)?,
            AuthData::None,
        ),
        concept::setting::Provider::OpenAICompatible => (
            AdapterKind::OpenAI,
            custom_endpoint(endpoint)?,
            auth(api_key)?,
        ),
    })
}

fn service_target(cfg: &concept::setting::Agent) -> Result<ServiceTarget> {
    let mut refined = None;
    if cfg.provider() == concept::setting::Provider::OpenAI {
        let adapter = AdapterKind::from_model(cfg.model())?;
        if !matches!(adapter, AdapterKind::OpenAI | AdapterKind::OpenAIResp) {
            return Err(ErrorKind::InvalidBody("model is not an OpenAI model".to_owned()).into());
        }
        refined = Some(adapter);
    }
    let (adapter, endpoint, auth) = base(
        cfg.provider(),
        cfg.api_key().as_deref(),
        cfg.endpoint().as_deref(),
    )?;
    Ok(ServiceTarget {
        endpoint,
        auth,
        model: ModelIden::new(refined.unwrap_or(adapter), cfg.model().clone()),
    })
}

fn auth(api_key: Option<&str>) -> Result<AuthData> {
    Ok(AuthData::from_single(required_key(api_key)?))
}

fn required_key(api_key: Option<&str>) -> Result<&str> {
    api_key
        .filter(|key| !key.is_empty())
        .ok_or_else(|| ErrorKind::InvalidBody("api key is required".to_owned()).into())
}

fn custom_endpoint(endpoint: Option<&str>) -> Result<Endpoint> {
    let raw = endpoint.ok_or_else(|| ErrorKind::InvalidBody("endpoint is required".to_owned()))?;
    Ok(Endpoint::from_owned(raw))
}
