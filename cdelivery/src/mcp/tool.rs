use std::sync::{Arc, LazyLock};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rmcp::model::{CallToolResult, ContentBlock, ResourceContents, Tool, ToolAnnotations};

use crate::operation;

static TOOLS: LazyLock<Vec<Tool>> =
    LazyLock::new(|| operation::catalog::SPECS.iter().map(definition).collect());

pub(crate) fn definitions(access: &crate::concept::permission::Access) -> Vec<Tool> {
    operation::catalog::SPECS
        .iter()
        .zip(TOOLS.iter())
        .filter(|(spec, _)| spec.allows(access))
        .map(|(_, tool)| tool.clone())
        .collect()
}

pub(crate) fn all() -> &'static [Tool] {
    &TOOLS
}

fn definition(spec: &operation::catalog::OperationSpec) -> Tool {
    let tool = Tool::new(spec.id, spec.description, Arc::new((spec.schema)()))
        .with_annotations(annotations(spec.effect));
    match spec.output_schema {
        Some(output_schema) => tool.with_raw_output_schema(Arc::new(output_schema())),
        None => tool,
    }
}

pub(crate) fn encode(payload: operation::Payload) -> CallToolResult {
    match payload {
        operation::Payload::Json(object) => {
            CallToolResult::structured(serde_json::Value::Object(object))
        }
        operation::Payload::Artifact(artifact) => {
            let contents = match String::from_utf8(artifact.bytes) {
                Ok(text) => {
                    ResourceContents::text(text, artifact.uri).with_mime_type(artifact.mime)
                }
                Err(binary) => {
                    ResourceContents::blob(STANDARD.encode(binary.as_bytes()), artifact.uri)
                        .with_mime_type(artifact.mime)
                }
            };
            CallToolResult::success(vec![ContentBlock::resource(contents)])
        }
    }
}

fn annotations(effect: operation::Effect) -> ToolAnnotations {
    match effect {
        operation::Effect::Read => ToolAnnotations::new().read_only(true),
        operation::Effect::Write {
            destructive,
            idempotent,
        } => ToolAnnotations::new()
            .destructive(destructive)
            .idempotent(idempotent),
    }
}
