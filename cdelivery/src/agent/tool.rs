use crate::{concept, infra, operation};

pub(super) struct Session<'a> {
    app: &'a crate::boot::App,
    access: concept::permission::Access,
    receipts: Vec<String>,
}

impl<'a> Session<'a> {
    pub(super) const fn new(
        app: &'a crate::boot::App,
        access: concept::permission::Access,
    ) -> Self {
        Self {
            app,
            access,
            receipts: Vec::new(),
        }
    }

    pub(super) fn receipts(&self) -> &[String] {
        &self.receipts
    }

    pub(super) fn definitions(&self) -> Vec<genai::chat::Tool> {
        operation::catalog::visible(&self.access)
            .map(|spec| {
                genai::chat::Tool::new(spec.id)
                    .with_description(spec.description)
                    .with_strict(false)
                    .with_schema(serde_json::Value::Object((spec.schema)()))
            })
            .collect()
    }

    pub(super) async fn call(
        &mut self,
        name: &str,
        raw: serde_json::Value,
        limit: usize,
    ) -> String {
        let output = match operation::catalog::find(name) {
            Some(spec) => {
                let tracked = spec.allows(&self.access)
                    && matches!(spec.effect, operation::Effect::Write { .. });
                let result = (spec.run)(self.app, &self.access, raw, jiff::Timestamp::now()).await;
                if tracked {
                    let receipt = match &result {
                        Ok(_) => format!("{}: OK.", spec.id),
                        Err(error) => {
                            let code: &str = error.kind().into();
                            format!("{}: {code}. Check the current state.", spec.id)
                        }
                    };
                    self.receipts.push(receipt);
                }
                match result {
                    Ok(payload) => encode(payload),
                    Err(error) => {
                        if error.status().is_server_error() {
                            tracing::error!(error = ?error, "agent tool {name} failed");
                        }
                        operation::problem_json(&error).to_string()
                    }
                }
            }
            None => operation::problem_json(
                &crate::error::ErrorKind::UnknownTool(name.to_owned()).into(),
            )
            .to_string(),
        };
        deliver(&output, limit)
    }
}

fn encode(payload: operation::Payload) -> String {
    match payload {
        operation::Payload::Json(object) => serde_json::Value::Object(object).to_string(),
        operation::Payload::Artifact(artifact) => match String::from_utf8(artifact.bytes) {
            Ok(text) => text,
            Err(binary) => serde_json::json!({
                "uri": artifact.uri,
                "filename": artifact.filename,
                "mime": artifact.mime,
                "size_bytes": binary.as_bytes().len(),
            })
            .to_string(),
        },
    }
}

fn deliver(output: &str, limit: usize) -> String {
    if output.len() <= limit {
        return output.to_owned();
    }
    let prefix = format!("[Truncated from {} bytes]\n", output.len());
    let content = infra::event::truncate_bytes(output, limit.saturating_sub(prefix.len()));
    format!("{prefix}{content}")
}
