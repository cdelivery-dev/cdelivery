use tokio_util::sync::CancellationToken;

use crate::config;
use crate::error::{ErrorKind, Result};
use crate::{infra, operation};

pub(super) async fn run(
    llm: &infra::llm::Llm,
    tools: &mut super::tool::Session<'_>,
    system: &str,
    cancel: &CancellationToken,
    context: impl FnOnce(usize) -> Vec<genai::chat::ChatMessage>,
) -> Result<Option<String>> {
    let reference = jiff::Timestamp::now();
    let definitions = tools.definitions();
    let system = format!(
        "{system}\n\n{}\n\nAnalysis reference time (UTC): {reference}.\n\n{}",
        operation::catalog::HISTORY_GUIDANCE,
        super::prompt::BUDGET,
    );
    let suffix = super::prompt::LIMIT_ROUNDS
        .len()
        .max(super::prompt::LIMIT_CONTEXT.len())
        .saturating_add(super::prompt::WIND_DOWN.len())
        .saturating_add(2);
    let reserved = definitions
        .iter()
        .map(genai::chat::Tool::size)
        .fold(system.len().saturating_add(suffix), usize::saturating_add);
    let budget = config::AGENT_PROMPT_CHAR_MAX.saturating_sub(reserved);
    if cancel.is_cancelled() {
        return Ok(None);
    }
    let mut messages = context(budget);
    let last = config::AGENT_MAX_TURNS.saturating_sub(1);
    let note = 'wind: {
        for _ in 0..last {
            let request = genai::chat::ChatRequest::new(messages.clone())
                .with_system(&system)
                .with_tools(definitions.clone());
            let response = tokio::select! {
                biased;
                () = cancel.cancelled() => return Ok(None),
                response = llm.chat(request, None) => response?,
            };
            let text = response.texts().concat();
            let Some(assistant) = response.into_assistant_message_for_tool_use() else {
                return reply(&text).map(Some);
            };
            let calls = assistant
                .content
                .tool_calls()
                .into_iter()
                .cloned()
                .collect::<Vec<_>>();
            let Some(per_call) = fit(budget, &messages, &assistant, &calls) else {
                break 'wind super::prompt::LIMIT_CONTEXT;
            };
            messages.push(assistant);
            for call in calls {
                if cancel.is_cancelled() {
                    return Ok(None);
                }
                let output = tools
                    .call(&call.fn_name, call.fn_arguments.clone(), per_call)
                    .await;
                messages.push(genai::chat::ChatMessage::from(
                    genai::chat::ToolResponse::from_tool_call(&call, output),
                ));
            }
        }
        super::prompt::LIMIT_ROUNDS
    };
    let request = genai::chat::ChatRequest::new(messages)
        .with_system(format!("{system}\n{note}\n{}", super::prompt::WIND_DOWN))
        .with_tools(definitions);
    let options =
        genai::chat::ChatOptions::default().with_tool_choice(genai::chat::ToolChoice::None);
    let response = tokio::select! {
        biased;
        () = cancel.cancelled() => return Ok(None),
        response = llm.chat(request, Some(&options)) => response?,
    };
    reply(&response.texts().concat()).map(Some)
}

fn reply(text: &str) -> Result<String> {
    match text.trim() {
        "" => Err(ErrorKind::AgentEmptyResponse.into()),
        text => Ok(text.to_owned()),
    }
}

fn fit(
    budget: usize,
    messages: &[genai::chat::ChatMessage],
    assistant: &genai::chat::ChatMessage,
    calls: &[genai::chat::ToolCall],
) -> Option<usize> {
    let mut used = size(messages).saturating_add(assistant.size());
    for call in calls {
        used = used.saturating_add(genai::chat::ToolResponse::from_tool_call(call, "").size());
    }
    let per_call = budget
        .checked_sub(used)?
        .checked_div(calls.len())?
        .min(config::AGENT_TOOL_OUTPUT_MAX);
    (per_call >= config::AGENT_TOOL_OUTPUT_MIN).then_some(per_call)
}

fn size(messages: &[genai::chat::ChatMessage]) -> usize {
    messages
        .iter()
        .map(genai::chat::ChatMessage::size)
        .fold(0, usize::saturating_add)
}
